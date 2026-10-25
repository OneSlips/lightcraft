//! The thread that writes the catalog log for [`crate::FsStore`] (natively).
//!
//! An append hands its bytes over and returns; the thread writes them in order and makes each batch
//! durable (`sync_data`) before the next. The caller (the UI thread, for the app) never waits for
//! the drive: on Windows a write to a file waits for a sync of that file in flight, and on a busy
//! disk one sync took 30–90 ms in a test and seconds on a photographer's machine while previews
//! were written, and every flag paid it (note 2 of 2026-10-25: the window froze 1.4 s before a
//! step happened).
//!
//! Cutting the log back and replacing it are rare (after a failed append, after a snapshot); they
//! run on the same thread, after everything handed over before, and the caller waits for them and
//! gets their result. A write that fails is returned by the next append (the journal then cuts the
//! log back and writes a snapshot, which holds everything); appends handed over after it are left
//! out until the log is cut back or replaced, so the file never holds a gap. Dropping the writer
//! waits for everything handed over.

use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

/// How long a caller waits for the thread at most (a cut, a replace, a read).
const WAIT: Duration = Duration::from_secs(60);

enum Msg {
    Append(Vec<u8>),
    Truncate(u64, Sender<io::Result<()>>),
    Replace(Vec<u8>, Sender<io::Result<()>>),
    /// Reply once everything before is written and synced.
    Flush(Sender<()>),
}

#[derive(Default)]
struct State {
    /// Why a write failed (until the log is cut back or replaced).
    failed: Option<String>,
    /// The failure was returned by an append already.
    told: bool,
}

pub(crate) struct LogWriter {
    pub(crate) name: String,
    tx: Option<Sender<Msg>>,
    state: Arc<Mutex<State>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

fn stopped() -> io::Error {
    io::Error::other("the catalog log writer stopped")
}

impl LogWriter {
    /// A writer for `dir/name` (`None` when no thread can start: the store then writes itself).
    pub(crate) fn start(dir: PathBuf, name: &str) -> Option<LogWriter> {
        let (tx, rx) = channel::<Msg>();
        let state = Arc::new(Mutex::new(State::default()));
        let st = state.clone();
        let path = dir.join(name);
        let tmp = dir.join(format!("{name}.tmp"));
        let thread = std::thread::Builder::new().name("catalog-log".into()).spawn(move || run(&rx, &st, &path, &tmp)).ok()?;
        Some(LogWriter { name: name.to_string(), tx: Some(tx), state, thread: Some(thread) })
    }

    fn send(&self, m: Msg) -> io::Result<()> {
        self.tx.as_ref().ok_or_else(stopped)?.send(m).map_err(|_| stopped())
    }

    /// Hand `data` over. Returns a write that failed since, once.
    pub(crate) fn append(&self, data: &[u8]) -> io::Result<()> {
        {
            let mut st = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(why) = st.failed.clone()
                && !st.told
            {
                st.told = true;
                return Err(io::Error::other(format!("the catalog log could not be written: {why}")));
            }
        }
        self.send(Msg::Append(data.to_vec()))
    }

    /// Cut the log to `len` bytes (never longer than what was written whole), after everything
    /// handed over; waits for it.
    pub(crate) fn truncate(&self, len: u64) -> io::Result<()> {
        let (tx, rx) = channel();
        self.send(Msg::Truncate(len, tx))?;
        rx.recv_timeout(WAIT).map_err(|_| stopped())?
    }

    /// Replace the log with `data` (atomically, durably), after everything handed over; waits for it.
    pub(crate) fn replace(&self, data: &[u8]) -> io::Result<()> {
        let (tx, rx) = channel();
        self.send(Msg::Replace(data.to_vec(), tx))?;
        rx.recv_timeout(WAIT).map_err(|_| stopped())?
    }

    /// Wait until everything handed over is written and synced.
    pub(crate) fn flush(&self) {
        let (tx, rx) = channel();
        if self.send(Msg::Flush(tx)).is_ok() {
            let _ = rx.recv_timeout(WAIT);
        }
    }
}

impl Drop for LogWriter {
    fn drop(&mut self) {
        drop(self.tx.take());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// The open log, and how much of it was written whole.
struct Open {
    file: std::fs::File,
    good: u64,
}

fn run(rx: &Receiver<Msg>, st: &Mutex<State>, path: &std::path::Path, tmp: &std::path::Path) {
    let lock = || st.lock().unwrap_or_else(PoisonError::into_inner);
    let fail = |why: String| {
        log::error!("catalog: {}: {why}", path.display());
        lock().failed.get_or_insert(why);
    };
    let mut open: Option<Open> = None;
    // the length written whole when a write failed (what a cut keeps at most)
    let mut good_at_failure: Option<u64> = None;
    while let Ok(first) = rx.recv() {
        let mut batch = vec![first];
        while let Ok(m) = rx.try_recv() {
            batch.push(m);
        }
        let mut dirty = false;
        let mut flushed = Vec::new();
        for m in batch {
            match m {
                Msg::Append(data) => {
                    if lock().failed.is_some() {
                        continue;
                    }
                    if open.is_none() {
                        let opened = std::fs::OpenOptions::new().create(true).append(true).open(path).and_then(|file| {
                            let good = file.metadata()?.len();
                            Ok(Open { file, good })
                        });
                        match opened {
                            Ok(o) => open = Some(o),
                            Err(e) => {
                                fail(e.to_string());
                                continue;
                            }
                        }
                    }
                    let Some(o) = open.as_mut() else { continue };
                    match o.file.write_all(&data) {
                        Ok(()) => {
                            o.good = o.good.saturating_add(data.len() as u64);
                            dirty = true;
                        }
                        Err(e) => {
                            good_at_failure = Some(o.good);
                            open = None;
                            fail(e.to_string());
                        }
                    }
                }
                Msg::Truncate(len, reply) => {
                    let keep = good_at_failure.map_or(len, |g| len.min(g));
                    open = None;
                    dirty = false;
                    let cut = (|| {
                        let f = match std::fs::OpenOptions::new().write(true).open(path) {
                            Ok(f) => f,
                            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
                            Err(e) => return Err(e),
                        };
                        let now = f.metadata()?.len();
                        f.set_len(keep.min(now))?;
                        f.sync_all()
                    })();
                    if cut.is_ok() {
                        *lock() = State::default();
                        good_at_failure = None;
                    }
                    let _ = reply.send(cut);
                }
                Msg::Replace(data, reply) => {
                    open = None;
                    dirty = false;
                    let written = (|| {
                        let mut f = crate::store::create_staging(tmp)?;
                        f.write_all(&data)?;
                        f.sync_all()?;
                        drop(f);
                        std::fs::rename(tmp, path)
                    })();
                    match &written {
                        Ok(()) => {
                            *lock() = State::default();
                            good_at_failure = None;
                        }
                        Err(_) => {
                            let _ = std::fs::remove_file(tmp);
                        }
                    }
                    let _ = reply.send(written);
                }
                Msg::Flush(reply) => {
                    if dirty && let Some(o) = open.as_ref() {
                        if let Err(e) = o.file.sync_data() {
                            fail(e.to_string());
                        }
                        dirty = false;
                    }
                    flushed.push(reply);
                }
            }
        }
        if dirty
            && let Some(o) = open.as_ref()
            && let Err(e) = o.file.sync_data()
        {
            good_at_failure = Some(o.good);
            open = None;
            fail(e.to_string());
        }
        for r in flushed {
            let _ = r.send(());
        }
    }
}
