//! The AI mask worker thread: it owns the models (SAM 3, SAM 2.1, OWLv2, BiRefNet, EoMT, each loaded
//! when first needed) and the last encoded photo, and runs every request (load, render the model
//! input, encode, clicks, descriptions, automatic selections, detail passes) in turn. The session only sends jobs and receives outcomes over channels, so no thread ever
//! waits on a lock the model holds, and the UI thread never runs the model.
//!
//! Every job runs under `catch_unwind`: a panic inside candle becomes an error outcome, drops
//! the (possibly inconsistent) model and leaves the worker ready for the next job. The busy
//! counters are decremented by drop guards, so they can't stay stuck. The model is unloaded
//! after [`IDLE_UNLOAD`] without requests (it holds several GB).
//!
//! Where the model can run on the GPU (Metal on macOS), a job that fails there (a panic or an
//! error from candle, e.g. a kernel the GPU's compiler can't build, issue #1974) frees the
//! model and runs once more on the CPU. When that works, the model stays on the CPU for the rest
//! of the session and the job's outcome says so; when it fails too, the GPU wasn't the cause and
//! the next job uses it again. `LIGHTCRAFT_GPU=0` or `LIGHTCRAFT_GPU_BACKEND=off` keep the model on
//! the CPU from the start.

use std::collections::VecDeque;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use lightcraft_develop::SegMask;
use lightcraft_segment::identity::Signature;
use lightcraft_segment::{
    Encoded, Owl, OwlEncoded, SAM2_MASK_SIDE, Sam2, Sam2Encoded, Sam3,
    birefnet::{BIREFNET_SIDE, BiRefNet},
    people::{Face, FaceModel, PeopleMap, PeopleModel, PeopleRequest, Surfaces, people_mask},
    remote::{self, Operation, Request, text_logits},
    scene::{Scene, SceneModel},
};

use super::people_cache::{self, Analysis};
use super::{AutoKind, AutoModel, Input, Prompt, Tag};
use crate::media::RenderJob;

/// Unload the model after this long without a request.
pub const IDLE_UNLOAD: Duration = Duration::from_secs(10 * 60);

/// What to compute.
#[derive(Clone)]
pub enum Kind {
    /// Load the model and encode the photo (so the first click is fast).
    Prepare,
    Clicks(Vec<lightcraft_segment::Click>),
    Text(String),
    /// A zoomed-in pass over `region` (normalized) of the photo, rendered by the job's `render`.
    Detail {
        prompt: Prompt,
        region: [f64; 4],
    },
    /// An automatic selection (Subject, Sky, …) computed by `model`; `target` says which person
    /// or object (by name, role or examples) when the selection is about one.
    Auto {
        what: AutoKind,
        model: AutoModel,
        target: Target,
    },
    /// The faces on the photo with their appearance signatures (`people.faces`); `people` is
    /// Selfie Multiclass for the clothes, hair and skin colours, when installed.
    Faces {
        face: PathBuf,
        people: Option<PathBuf>,
        /// SAM 2.1, for appearance features beyond colour (when installed).
        sam2: Option<PathBuf>,
    },
    /// Learn an example of a remembered object from a mask on the photo (`concepts.teach`).
    Learn {
        mask: Box<lightcraft_develop::Mask>,
    },
    /// The skin of `area` (of everyone, or of the `person`-th from the left) and every face with
    /// its landmarks, by MediaPipe's Selfie Multiclass (`people`) and Face Landmarker (`face`):
    /// what Smooth Skin and Find Blemishes work with.
    Skin {
        area: lightcraft_develop::SkinArea,
        person: u32,
        people: PathBuf,
        face: PathBuf,
    },
    /// The parts of people the Retouch sections work on (teeth, eyes, lips, hair…), each of
    /// everyone or of one person, and every face, by MediaPipe's Selfie Multiclass (`people`) and
    /// Face Landmarker (`face`).
    Retouch {
        parts: Vec<lightcraft_develop::PartRequest>,
        people: PathBuf,
        face: PathBuf,
    },
    /// Panics (tests of the worker's recovery).
    #[cfg(test)]
    Panic,
    /// Fails like the model on a GPU that can't run it (a panic or a candle error), and
    /// succeeds on the CPU (tests of the CPU fallback).
    #[cfg(test)]
    FailOnGpu {
        panic: bool,
    },
}

/// One request.
pub struct Job {
    /// The model folder.
    pub dir: PathBuf,
    /// Object clicks (and their detail passes) run on SAM 2.1 (`dir` is its folder), not SAM 3.
    pub sam2: bool,
    /// Describe runs on OWLv2 (this folder) and SAM 2.1 (`dir`, with `sam2`), not SAM 3.
    pub detector: Option<PathBuf>,
    /// Which photo and look the request is about (the encode cache's key).
    pub key: u64,
    /// Renders the model input when the cache doesn't hold `key` (always for a detail pass).
    pub render: Option<RenderJob>,
    /// Renders the photo sharper ([`super::FINE_EDGE`]) for a group's per-person crops; run only
    /// when they are needed.
    pub fine: Option<RenderJob>,
    /// The photo's file, when it is missing on disk (for a clearer error if rendering fails).
    pub missing: Option<String>,
    pub kind: Kind,
    pub tag: Tag,
    pub reply: mpsc::Sender<Outcome>,
    /// Where the slow steps (loading a model, analyzing a photo) show as activity rows.
    pub activity: Option<crate::activity::Activity>,
    /// The batch it belongs to and the batches cancelled meanwhile (an undone preset): a job of a
    /// cancelled batch is skipped.
    pub batch: Option<(u64, Arc<std::sync::Mutex<std::collections::HashSet<u64>>>)>,
    /// The batch's activity row: the slow steps' rows show under it (client note 34).
    pub parent: Option<u64>,
    /// Where a photo's people analysis is kept on disk (the preview cache; `None`: memory only).
    pub analyses: Option<Arc<lightcraft_preview::PreviewCache>>,
}

impl Job {
    fn cancelled(&self) -> bool {
        self.batch.as_ref().is_some_and(|(b, set)| set.lock().unwrap_or_else(std::sync::PoisonError::into_inner).contains(b))
    }
}

/// A finished request: the segmentation (`None`: nothing selected / nothing to report).
pub struct Outcome {
    pub tag: Tag,
    pub result: Result<Option<SegMask>, String>,
    /// A newer request for the same selection came in; this one was skipped.
    pub superseded: bool,
    /// The faces a [`Kind::Skin`] request found (left to right, normalized).
    pub faces: Option<Vec<super::FoundFace>>,
    /// Something the user should know about the result (e.g. face features it couldn't find).
    pub note: Option<String>,
    /// What a request that isn't a segmentation returns (faces, a learned example).
    pub payload: Payload,
    /// Something the user should know besides the result ([`GPU_FAILED`]).
    pub notice: Option<String>,
}

/// The notice of the job after which the model runs on the CPU.
pub const GPU_FAILED: &str =
    "The GPU couldn't run the AI mask model, so it runs on the CPU until LightCraft restarts (slower). Details are in the log.";

/// Where the model is loaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Placement {
    /// On the GPU when there is one (Metal on macOS); a job that fails there is retried on the CPU.
    Gpu,
    /// On the CPU: this build has no GPU path, the environment switches the GPU off, or the model
    /// failed on it.
    Cpu,
}

impl Placement {
    fn initial() -> Self {
        if !lightcraft_segment::GPU_BUILD {
            return Placement::Cpu;
        }
        if let Some(why) = lightcraft_gpu::env_off_reason() {
            log::info!("SAM 3 runs on the CPU ({why})");
            return Placement::Cpu;
        }
        Placement::Gpu
    }
}

/// Why a job failed.
#[derive(Debug)]
enum Failure {
    /// The model failed (a candle error, or a panic): on the GPU, the CPU may run it.
    Model(String),
    /// Anything else (no model files, the photo, a remote worker): another device won't help.
    Other(String),
}

impl Failure {
    fn message(self) -> String {
        match self {
            Failure::Model(m) | Failure::Other(m) => m,
        }
    }
}

impl From<String> for Failure {
    fn from(m: String) -> Self {
        Failure::Other(m)
    }
}

impl From<&str> for Failure {
    fn from(m: &str) -> Self {
        Failure::Other(m.into())
    }
}

impl From<lightcraft_segment::Error> for Failure {
    fn from(e: lightcraft_segment::Error) -> Self {
        if matches!(e, lightcraft_segment::Error::Candle(_)) { Failure::Model(e.to_string()) } else { Failure::Other(e.to_string()) }
    }
}

/// Which person or object an automatic selection is about.
#[derive(Clone, Debug, Default)]
pub enum Target {
    #[default]
    None,
    /// A person: their named face box on this photo (normalized), else their known appearance
    /// (`sam2`: for appearance features, when installed).
    /// `ident`: their face by the recognition model the user chose, when it is in use.
    Person { rect: Option<[f64; 4]>, known: Vec<Signature>, sam2: Option<PathBuf>, ident: Option<Arc<Identity>> },
    /// A remembered object's examples.
    Concept(Arc<Vec<lightcraft_segment::concept::Example>>),
}

/// Who a person is by face (test notes 2026-10-25, note 12): the recognition model the user chose
/// in Settings ▸ Faces, and the embeddings of the person's named faces (the People index).
pub struct Identity {
    pub embedder: Arc<lightcraft_faces::runtime::Embedder>,
    /// The model (`<id>@<sha256>`): a photo's face embeddings are kept under it.
    pub tag: String,
    /// The model's cosine for "the same person".
    pub threshold: f32,
    pub known: Vec<Arc<[f32]>>,
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Identity").field("tag", &self.tag).field("threshold", &self.threshold).field("known", &self.known.len()).finish()
    }
}

/// A face found on a photo (normalized box; left to right).
#[derive(Clone, Debug)]
pub struct FoundFace {
    pub rect: [f64; 4],
    pub score: f32,
    pub signature: Signature,
    /// FaceMesh placed its landmarks (not a strong profile).
    pub landmarks: bool,
}

/// Non-segmentation results.
#[derive(Clone, Debug, Default)]
pub enum Payload {
    #[default]
    None,
    Faces(Vec<FoundFace>),
    Example(Box<lightcraft_segment::concept::Example>),
    /// The parts a [`Kind::Retouch`] request asked for, each found or not.
    Retouch(Vec<lightcraft_develop::RetouchMask>),
    /// A selection of a named person found nothing because who they are on this photo couldn't
    /// be told with confidence ([`Pick::Unsure`]).
    Unsure,
}

/// Counters the session reads without locking.
#[derive(Default)]
pub struct Shared {
    /// Requests other than detail passes, queued or running.
    pub pending: AtomicUsize,
    /// Detail passes queued or running.
    pub detail: AtomicUsize,
    /// The model is in memory.
    pub loaded: AtomicBool,
    /// Loading the model or encoding a photo right now (the slow parts).
    pub encoding: AtomicBool,
}

/// Decrements a counter when dropped (also when unwinding).
struct Done<'a>(&'a AtomicUsize);

impl Drop for Done<'_> {
    fn drop(&mut self) {
        let _ = self.0.try_update(Ordering::SeqCst, Ordering::SeqCst, |n| Some(n.saturating_sub(1)));
    }
}

/// Sets a flag while alive.
struct Flag<'a>(&'a AtomicBool);

impl<'a> Flag<'a> {
    fn raise(f: &'a AtomicBool) -> Self {
        f.store(true, Ordering::SeqCst);
        Flag(f)
    }
}

impl Drop for Flag<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// The session's handle on the worker thread (started on the first request, restarted if it
/// ever ends).
#[derive(Default)]
pub struct Worker {
    tx: Option<mpsc::Sender<Job>>,
    thread: Option<std::thread::JoinHandle<()>>,
    pub shared: Arc<Shared>,
}

impl Worker {
    fn counter(&self, job: &Job) -> &AtomicUsize {
        if matches!(job.kind, Kind::Detail { .. }) { &self.shared.detail } else { &self.shared.pending }
    }

    /// Whether the thread has ended (it never should); its counters are then meaningless.
    fn dead(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| t.is_finished())
    }

    pub fn pending(&self) -> usize {
        if self.dead() { 0 } else { self.shared.pending.load(Ordering::SeqCst) }
    }

    pub fn detail(&self) -> usize {
        if self.dead() { 0 } else { self.shared.detail.load(Ordering::SeqCst) }
    }

    pub fn encoding(&self) -> bool {
        !self.dead() && self.shared.encoding.load(Ordering::SeqCst)
    }

    pub fn loaded(&self) -> bool {
        !self.dead() && self.shared.loaded.load(Ordering::SeqCst)
    }

    /// Queue `job`; returns at once.
    pub fn submit(&mut self, job: Job) -> Result<(), String> {
        if self.dead() {
            // a thread that ended took its counters' meaning with it
            self.tx = None;
            self.thread = None;
            self.shared = Arc::new(Shared::default());
        }
        self.counter(&job).fetch_add(1, Ordering::SeqCst);
        let job = match &self.tx {
            Some(tx) => match tx.send(job) {
                Ok(()) => return Ok(()),
                Err(mpsc::SendError(job)) => job,
            },
            None => job,
        };
        // (re)start the thread
        let (tx, rx) = mpsc::channel();
        let shared = self.shared.clone();
        match std::thread::Builder::new().name("sam3".into()).spawn(move || run(&rx, &shared, IDLE_UNLOAD, Placement::initial())) {
            Ok(t) => {
                self.thread = Some(t);
                self.tx = Some(tx.clone());
                tx.send(job).map_err(|_| "the AI mask worker stopped".to_string())
            }
            Err(e) => {
                let _ = self.counter(&job).try_update(Ordering::SeqCst, Ordering::SeqCst, |n| Some(n.saturating_sub(1)));
                Err(format!("could not start the AI mask worker: {e}"))
            }
        }
    }
}

/// The models and what they last computed for a photo.
#[derive(Default)]
struct State {
    model: Option<(PathBuf, Sam3)>,
    cache: Option<(u64, Encoded)>,
    remote: Option<remote::Client>,
    sam2: Option<(PathBuf, Sam2)>,
    sam2_cache: Option<(u64, Sam2Encoded)>,
    owl: Option<(PathBuf, Owl)>,
    /// OWLv2's boxes and class embeddings for a photo (each description reuses them).
    owl_cache: Option<(u64, OwlEncoded)>,
    subject: Option<(PathBuf, BiRefNet)>,
    /// BiRefNet's logits for a photo (Subject and Background share them).
    subject_cache: Option<(u64, Vec<f32>)>,
    scene: Option<(PathBuf, SceneModel)>,
    /// EoMT's scene for a photo (Sky, People, Landscape and Subject share it).
    scene_cache: Option<(u64, Scene)>,
    people: Option<(PathBuf, PeopleModel)>,
    /// Selfie Multiclass's classes for a photo.
    people_cache: Option<(u64, PeopleMap)>,
    face: Option<(PathBuf, FaceModel)>,
    /// The faces (with landmarks) of a photo.
    faces_cache: Option<(u64, Vec<Face>)>,
    /// The class map of a photo with a group's people segmented on their own crops (`None`: no
    /// one small enough to need it).
    crowd_cache: Option<(u64, Option<PeopleMap>)>,
    /// The last rendered input of the openly licensed models (several of them may run on one photo).
    input_cache: Option<(u64, Input)>,
    /// The faces the job that just ran found (a [`Kind::Skin`] request's), for its outcome.
    found: Option<Vec<super::FoundFace>>,
    /// A note about the job just run, for its outcome.
    note: Option<String>,
    /// The job just run couldn't tell with confidence who the person it was asked about is.
    unsure: bool,
    /// The last photos' people, analysed ([`people_cache`]; test notes 2026-10-25, note 10).
    analyses: people_cache::Memory,
    /// A group's per-person crops and their class maps (`crowd_cache` is them pasted).
    crowd_crops: Option<(u64, Vec<[f32; 4]>, Vec<PeopleMap>)>,
    /// Everyone's SAM 2.1 silhouette ([`person_instances`]).
    silhouettes_cache: Option<(u64, Vec<Vec<f32>>)>,
    /// Everyone EoMT sees as a person ([`scene_people`]).
    figures_cache: Option<(u64, Vec<Vec<f32>>)>,
    /// The floors and walls around them ([`scene_surfaces`]).
    surfaces_cache: Option<(u64, Surfaces)>,
    /// Each face's appearance signature, and whether it holds SAM 2.1's features.
    signatures_cache: Option<(u64, bool, Vec<Option<Signature>>)>,
    /// Each face's embedding by a recognition model (its tag).
    idents_cache: Option<(u64, String, Vec<Option<Vec<f32>>>)>,
}

impl State {
    fn any_loaded(&self) -> bool {
        self.model.is_some()
            || self.remote.is_some()
            || self.sam2.is_some()
            || self.owl.is_some()
            || self.subject.is_some()
            || self.scene.is_some()
            || self.people.is_some()
            || self.face.is_some()
    }

    /// Forget what was computed for photos (a failed request may have left it half done).
    fn forget_photos(&mut self) {
        self.cache = None;
        self.sam2_cache = None;
        self.owl_cache = None;
        self.subject_cache = None;
        self.scene_cache = None;
        self.people_cache = None;
        self.faces_cache = None;
        self.crowd_cache = None;
        self.input_cache = None;
        self.crowd_crops = None;
        self.silhouettes_cache = None;
        self.figures_cache = None;
        self.surfaces_cache = None;
        self.signatures_cache = None;
        self.idents_cache = None;
    }
}

/// Whether `later` makes `job` pointless (it asks for the same selection again).
fn supersedes(later: &Job, job: &Job) -> bool {
    match (&later.tag, &job.tag) {
        (Tag::Prepare, Tag::Prepare) => later.key == job.key,
        (Tag::Clicks { photo: a, mask: am, comp: ac, .. }, Tag::Clicks { photo: b, mask: bm, comp: bc, .. }) => (a, am, ac) == (b, bm, bc),
        (Tag::Detail { photo: a, mask: am, comp: ac, .. }, Tag::Detail { photo: b, mask: bm, comp: bc, .. }) => (a, am, ac) == (b, bm, bc),
        _ => false,
    }
}

/// Large matrix products of the models on the GPU (`lightcraft_gpu::gemm`); declines (the CPU
/// does them) whenever the GPU is unavailable, switched off or fails.
struct GpuGemm;

impl lightcraft_segment::accel::Gemm for GpuGemm {
    fn gemm(&self, a: &[f32], rows: usize, k: usize, n: usize, key: u64, weights: &dyn Fn() -> Option<Vec<f32>>) -> Option<Vec<f32>> {
        lightcraft_gpu::gemm::matmul(a, rows, k, n, key, weights)
    }
}

/// Give the models the GPU for their large matrix products when `LIGHTCRAFT_AI_GPU=1`. Off by
/// default: measured on an RTX 5070 (DX12) it saves ~20 % of a warm BiRefNet pass but its first
/// pass costs ~20 s more (kernel compilation, uploading 885 MB of weights), and the rest of the
/// pass (window attention, deformable sampling) stays on the CPU.
fn install_gpu() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let on = std::env::var("LIGHTCRAFT_AI_GPU").is_ok_and(|v| matches!(v.trim(), "1" | "on" | "true" | "yes"));
        if on {
            lightcraft_segment::accel::set_gemm(Some(Arc::new(GpuGemm)));
        }
    });
}

fn run(rx: &mpsc::Receiver<Job>, shared: &Shared, idle: Duration, mut placement: Placement) {
    install_gpu();
    let mut state = State::default();
    let mut queue: VecDeque<Job> = VecDeque::new();
    loop {
        if queue.is_empty() {
            match rx.recv_timeout(idle) {
                Ok(j) => queue.push_back(j),
                Err(RecvTimeoutError::Timeout) => {
                    if state.any_loaded() {
                        state = State::default();
                        shared.loaded.store(false, Ordering::SeqCst);
                        log::info!("AI mask models unloaded after {} minutes without use", idle.as_secs() / 60);
                    }
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
        while let Ok(j) = rx.try_recv() {
            queue.push_back(j);
        }
        let Some(mut job) = queue.pop_front() else { continue };
        let is_detail = matches!(job.kind, Kind::Detail { .. });
        // counts the job as pending until just before its reply goes out: a caller woken by the
        // reply must not still see the worker busy with it
        let done = Done(if is_detail { &shared.detail } else { &shared.pending });
        if queue.iter().any(|later| supersedes(later, &job)) || job.cancelled() {
            drop(done);
            let _ = job.reply.send(Outcome {
                tag: job.tag,
                result: Ok(None),
                superseded: true,
                faces: None,
                payload: Payload::None,
                note: None,
                notice: None,
            });
            continue;
        }
        // what a retry on the CPU needs (running the job takes them)
        let retry = (placement == Placement::Gpu && remote::configured().is_none()).then(|| (job.kind.clone(), job.render.clone()));
        let mut payload = Payload::None;
        let mut result = attempt(&mut state, shared, &mut job, placement, &mut payload);
        let mut notice = None;
        let gpu_failed = match &result {
            Err(Failure::Model(why)) => Some(why.clone()),
            _ => None,
        };
        if let (Some(why), Some((kind, render))) = (gpu_failed, retry) {
            // the GPU may be the cause (e.g. a kernel its compiler can't build, issue #1974): free
            // the model there and try once on the CPU
            log::warn!("SAM 3 failed on the GPU ({why}); trying the CPU");
            state = State::default();
            (job.kind, job.render) = (kind, render);
            payload = Payload::None;
            match attempt(&mut state, shared, &mut job, Placement::Cpu, &mut payload) {
                Ok(r) => {
                    log::warn!("SAM 3 runs on the CPU from now on: it failed on the GPU and works on the CPU");
                    placement = Placement::Cpu;
                    notice = Some(GPU_FAILED.to_string());
                    result = Ok(r);
                }
                Err(e) => {
                    // the CPU fails too, so the GPU isn't the cause: the next job uses it again
                    log::warn!("SAM 3 failed on the CPU too: {}", e.message());
                    state = State::default();
                }
            }
        }
        let result = result.map_err(Failure::message);
        if result.is_err() && !is_detail {
            state.forget_photos();
        }
        let faces = state.found.take();
        shared.loaded.store(state.any_loaded(), Ordering::SeqCst);
        drop(done);
        let note = state.note.take();
        if let Some(n) = &note {
            log::info!("AI masks: {n}");
        }
        let _ = job.reply.send(Outcome { tag: job.tag, result, superseded: false, faces, payload, note, notice });
    }
}

/// Run `job` with the model on `placement`. A panic is a [`Failure::Model`] and drops what the
/// model kept.
fn attempt(state: &mut State, shared: &Shared, job: &mut Job, placement: Placement, payload: &mut Payload) -> Result<Option<SegMask>, Failure> {
    match std::panic::catch_unwind(AssertUnwindSafe(|| run_job(state, shared, job, placement, payload))) {
        Ok(r) => r,
        Err(p) => {
            let why = p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_default();
            log::error!("AI mask model panicked: {why}");
            // don't trust what the model kept: load it again next time
            *state = State::default();
            Err(Failure::Model("the AI model failed unexpectedly (it will be reloaded on the next try)".into()))
        }
    }
}

/// The model for `dir`, loading it on `placement` when needed.
fn model<'a>(state: &'a mut State, shared: &Shared, dir: &std::path::Path, placement: Placement) -> Result<&'a mut Sam3, Failure> {
    if state.model.as_ref().is_none_or(|(d, _)| d != dir) {
        state.model = None;
        state.cache = None;
        let _busy = Flag::raise(&shared.encoding);
        let t = web_time::Instant::now();
        let m = match placement {
            Placement::Gpu => Sam3::load(dir)?,
            Placement::Cpu => Sam3::load_cpu(dir)?,
        };
        log::info!("SAM 3 loaded in {:?} ({})", t.elapsed(), if m.device().is_cpu() { "CPU" } else { "GPU" });
        state.model = Some((dir.to_path_buf(), m));
        shared.loaded.store(true, Ordering::SeqCst);
    }
    state.model.as_mut().map(|(_, m)| m).ok_or_else(|| "the model did not load".into())
}

/// Load the model in `slot` from `dir` when it isn't loaded from there (dropping the photos
/// computed with another copy).
fn load<'a, M>(
    slot: &'a mut Option<(PathBuf, M)>,
    shared: &Shared,
    dir: &std::path::Path,
    name: &str,
    open: impl FnOnce(&std::path::Path) -> lightcraft_segment::Result<M>,
) -> Result<&'a M, String> {
    if slot.as_ref().is_none_or(|(d, _)| d != dir) {
        *slot = None;
        let _busy = Flag::raise(&shared.encoding);
        let t = web_time::Instant::now();
        let m = open(dir).map_err(|e| format!("{name}: {e}"))?;
        log::info!("{name} loaded in {:?}", t.elapsed());
        *slot = Some((dir.to_path_buf(), m));
        shared.loaded.store(true, Ordering::SeqCst);
    }
    slot.as_ref().map(|(_, m)| m).ok_or_else(|| format!("{name} did not load"))
}

/// SAM 2.1 and the photo `job.key` encoded by it (rendering and encoding it when needed).
fn sam2_encoded<'a>(state: &'a mut State, shared: &Shared, job: &mut Job) -> Result<(&'a Sam2, &'a Sam2Encoded), String> {
    if state.sam2.as_ref().is_none_or(|(d, _)| *d != job.dir) {
        state.sam2_cache = None;
    }
    load(&mut state.sam2, shared, &job.dir, "SAM 2.1", Sam2::load)?;
    if state.sam2_cache.as_ref().is_none_or(|(k, _)| *k != job.key) {
        state.sam2_cache = None;
        let input = render(job)?;
        let _busy = Flag::raise(&shared.encoding);
        let _row = row(job, "SAM 2.1");
        let m = state.sam2.as_ref().map(|(_, m)| m).ok_or("SAM 2.1 did not load")?;
        let t = web_time::Instant::now();
        let enc = m.encode(&input.rgb, input.w, input.h).map_err(|e| e.to_string())?;
        log::info!("SAM 2.1 encoded {}×{} in {:?}", input.w, input.h, t.elapsed());
        state.sam2_cache = Some((job.key, enc));
    }
    match (&state.sam2, &state.sam2_cache) {
        (Some((_, m)), Some((_, e))) => Ok((m, e)),
        _ => Err("SAM 2.1 is not ready".into()),
    }
}

/// Clicks inside `region` (normalized) of the photo, in the region's own coordinates; `None` when
/// no including click is inside.
fn clicks_in(clicks: &[super::Click], region: [f64; 4]) -> Option<Vec<lightcraft_segment::Click>> {
    let (rw, rh) = (region[2] - region[0], region[3] - region[1]);
    if !(rw > 0.0 && rh > 0.0) {
        return None;
    }
    let inside: Vec<lightcraft_segment::Click> = clicks
        .iter()
        .map(|c| ((c.at.x - region[0]) / rw, (c.at.y - region[1]) / rh, c.include))
        .filter(|(x, y, _)| (0.0..=1.0).contains(x) && (0.0..=1.0).contains(y))
        .map(|(x, y, positive)| lightcraft_segment::Click { x: x as f32, y: y as f32, positive })
        .collect();
    inside.iter().any(|c| c.positive).then_some(inside)
}

/// Side of the grid an EoMT selection is stored at: its output (160 px on the long edge) refined on
/// the model input ([`lightcraft_segment::matte::refine_coarse`]).
const SCENE_SIDE: usize = 1024;

/// The COCO panoptic classes (by name) each landscape class covers.
fn landscape_classes(class: &str) -> &'static [&'static str] {
    match class {
        "sky" => &["sky-other-merged"],
        "water" => &["sea", "river", "water-other"],
        "vegetation" => &["tree-merged", "grass-merged", "flower", "potted plant"],
        "mountains" => &["mountain-merged", "rock-merged"],
        "architecture" => &[
            "building-other-merged",
            "house",
            "roof",
            "bridge",
            "tent",
            "wall-brick",
            "wall-stone",
            "wall-tile",
            "wall-wood",
            "wall-other-merged",
            "fence-merged",
            "door-stuff",
            "window-other",
            "window-blind",
            "stairs",
        ],
        "naturalGround" => &["sand", "dirt-merged", "gravel", "snow", "playingfield"],
        "artificialGround" => &["road", "pavement-merged", "railroad", "platform", "floor-wood", "floor-other-merged", "rug-merged"],
        _ => &[],
    }
}

/// An activity row for a slow step of `job` (analyzing a photo with `name`).
fn row(job: &Job, name: &str) -> Option<crate::activity::TaskGuard> {
    job.activity.as_ref().map(|a| {
        let t = match job.parent {
            Some(parent) => a.start_child(parent, "ai", "Analyzing photo for AI masks", crate::activity::Cancel::No),
            None => a.start("ai", "Analyzing photo for AI masks", crate::activity::Cancel::No),
        };
        t.detail(name);
        t
    })
}

/// The openly licensed models' input for `job`'s photo, rendered once per photo.
fn photo<'a>(cache: &'a mut Option<(u64, Input)>, job: &mut Job) -> Result<&'a Input, String> {
    if cache.as_ref().is_none_or(|(k, _)| *k != job.key || job.key == 0) {
        *cache = None;
        *cache = Some((job.key, render(job)?));
    }
    cache.as_ref().map(|(_, i)| i).ok_or_else(|| "the photo is not available".to_string())
}

/// Run `f` on the model in `slot` and the photo, unless `cache` holds its result for the photo.
fn cached<'a, M, T>(
    slot: &Option<(PathBuf, M)>,
    cache: &'a mut Option<(u64, T)>,
    input: &mut Option<(u64, Input)>,
    shared: &Shared,
    job: &mut Job,
    name: &str,
    f: impl FnOnce(&M, &Input) -> lightcraft_segment::Result<T>,
) -> Result<&'a T, String> {
    if cache.as_ref().is_none_or(|(k, _)| *k != job.key) {
        *cache = None;
        let m = slot.as_ref().map(|(_, m)| m).ok_or_else(|| format!("{name} did not load"))?;
        let img = photo(input, job)?;
        let _busy = Flag::raise(&shared.encoding);
        let _row = row(job, name);
        let t = web_time::Instant::now();
        let out = f(m, img).map_err(|e| format!("{name}: {e}"))?;
        log::info!("{name}: {}×{} in {:?}", img.w, img.h, t.elapsed());
        *cache = Some((job.key, out));
    }
    cache.as_ref().map(|(_, v)| v).ok_or_else(|| format!("{name} is not ready"))
}

/// Load the model in `slot` from `dir` (forgetting what it computed when it moves).
fn ensure<M, T>(
    slot: &mut Option<(PathBuf, M)>,
    cache: &mut Option<(u64, T)>,
    shared: &Shared,
    dir: &std::path::Path,
    name: &str,
    open: impl FnOnce(&std::path::Path) -> lightcraft_segment::Result<M>,
) -> Result<(), String> {
    if slot.as_ref().is_none_or(|(d, _)| d != dir) {
        *cache = None;
    }
    load(slot, shared, dir, name, open).map(|_| ())
}

/// The MediaPipe face model, loaded again when its small-face detector arrived after it was loaded (fetched in the
/// background for a model an older LightCraft installed), so group photos' small faces are found from then on.
fn ensure_face<T>(
    slot: &mut Option<(PathBuf, FaceModel)>,
    cache: &mut Option<(u64, T)>,
    shared: &Shared,
    dir: &std::path::Path,
) -> Result<(), String> {
    if slot.as_ref().is_some_and(|(d, m)| d == dir && !m.has_far() && dir.join(lightcraft_segment::people::FAR_FILE).is_file()) {
        *slot = None;
        *cache = None;
    }
    ensure(slot, cache, shared, dir, "MediaPipe faces", FaceModel::load)
}

/// An automatic selection; `None` when nothing of the kind was found.
fn auto(state: &mut State, shared: &Shared, job: &mut Job, what: &AutoKind, model: AutoModel, target: &Target) -> Result<Option<SegMask>, String> {
    let invert = |l: Vec<f32>| l.into_iter().map(|v| -v).collect::<Vec<f32>>();
    let background = *what == AutoKind::Background;
    match model {
        AutoModel::Subject(dir) => {
            ensure(&mut state.subject, &mut state.subject_cache, shared, &dir, "BiRefNet", BiRefNet::load)?;
            let logits = cached(&state.subject, &mut state.subject_cache, &mut state.input_cache, shared, job, "BiRefNet", |m, i| {
                m.segment(&i.rgb, i.w, i.h)
            })?
            .clone();
            let logits = if background { invert(logits) } else { logits };
            Ok(Some(SegMask::from_logits(BIREFNET_SIDE, &logits)))
        }
        AutoModel::Scene(dir) => {
            ensure(&mut state.scene, &mut state.scene_cache, shared, &dir, "EoMT", SceneModel::load)?;
            let scene =
                cached(&state.scene, &mut state.scene_cache, &mut state.input_cache, shared, job, "EoMT", |m, i| m.segment(&i.rgb, i.w, i.h))?;
            let Some(probs) = scene_probs(scene, what) else { return Ok(None) };
            // the sky through the leaves: trees are where it may show (not grass: a pale lawn under
            // a hazy sky is not sky)
            let foliage = (*what == AutoKind::Sky).then(|| scene.class_probability(&lightcraft_segment::scene::class_ids(|n| n == "tree-merged")));
            let (sw, sh) = (scene.width, scene.height);
            let input = state.input_cache.as_ref().filter(|(k, _)| *k == job.key).map(|(_, i)| i).ok_or("the photo is not available")?;
            let t = web_time::Instant::now();
            let cov = lightcraft_segment::matte::refine_coarse(&probs, sw, sh, &input.rgb, input.w, input.h, SCENE_SIDE, foliage.as_deref());
            log::info!("EoMT selection refined on the photo at {SCENE_SIDE} cells in {:?}", t.elapsed());
            let logits = lightcraft_segment::mediapipe::coverage_to_logits(&cov);
            let logits = if background { invert(logits) } else { logits };
            Ok(Some(SegMask::from_logits(SCENE_SIDE, &logits)))
        }
        AutoModel::People { people, face, scene, sam2 } => {
            let models = people_cache::model_tag(&[people.as_deref(), face.as_deref(), scene.as_deref(), sam2.as_deref()]);
            let before = recall_people(state, shared, job, people.as_deref(), face.as_deref(), models);
            let out = people_auto(state, shared, job, what, AutoModel::People { people, face, scene, sam2 }, target);
            keep_people(state, job, before.as_deref(), models);
            out
        }
        AutoModel::Concept { sam2, people } => {
            let Target::Concept(examples) = target else { return Err("which remembered object?".into()) };
            job.dir = sam2;
            let found = {
                let (m, enc, input) = sam2_on_input(state, shared, job)?;
                let _row = row(job, "Finding a remembered object");
                lightcraft_segment::concept::find(m, enc, examples, &input.rgb, input.w, input.h).map_err(|e| e.to_string())?
            };
            log::info!("remembered object: best likeness {:.3}", found.as_ref().map_or(f32::NAN, |f| f.score));
            let Some(f) = found else { return Ok(None) };
            // the class map, to take the wearer's skin and hair out of a garment (without the
            // people model the edge is refined all the same)
            if let Some(dir) = &people {
                let mapped = ensure(&mut state.people, &mut state.people_cache, shared, dir, "MediaPipe people", PeopleModel::load).and_then(|()| {
                    cached(&state.people, &mut state.people_cache, &mut state.input_cache, shared, job, "MediaPipe people", |m, i| {
                        m.segment(&i.rgb, i.w, i.h)
                    })
                    .map(|_| ())
                });
                if let Err(e) = mapped {
                    log::warn!("remembered object: skin and hair can't be taken out ({e})");
                }
            }
            let map = state.people_cache.as_ref().filter(|(k, _)| people.is_some() && *k == job.key).map(|(_, m)| m);
            let input = state.input_cache.as_ref().filter(|(k, _)| *k == job.key).map(|(_, i)| i).ok_or("the photo is not available")?;
            Ok(match lightcraft_segment::people::garment_mask(&f.logits, f.mask_side, &input.rgb, input.w, input.h, map) {
                Some(g) => Some(SegMask::from_logits(g.side, &g.logits)),
                None => Some(SegMask::from_logits(f.mask_side, &f.logits)),
            })
        }
    }
}

/// A People selection (with [`auto`]'s arguments; `model` is [`AutoModel::People`]).
fn people_auto(
    state: &mut State,
    shared: &Shared,
    job: &mut Job,
    what: &AutoKind,
    model: AutoModel,
    target: &Target,
) -> Result<Option<SegMask>, String> {
    let invert = |l: Vec<f32>| l.into_iter().map(|v| -v).collect::<Vec<f32>>();
    let background = *what == AutoKind::Background;
    let AutoModel::People { people, face, scene, sam2 } = model else { return Err("not a People model".into()) };
    let map = match &people {
        Some(dir) => {
            ensure(&mut state.people, &mut state.people_cache, shared, dir, "MediaPipe people", PeopleModel::load)?;
            Some(cached(&state.people, &mut state.people_cache, &mut state.input_cache, shared, job, "MediaPipe people", |m, i| {
                m.segment(&i.rgb, i.w, i.h)
            })?)
        }
        None => None,
    };
    if matches!(what, AutoKind::Subject | AutoKind::Background) {
        // a person is the subject when there is one; else EoMT's most confident thing
        let person_there = map.as_ref().is_some_and(|m| m.person_share() > 0.005);
        if !person_there && let Some(scene) = scene {
            return auto(state, shared, job, what, AutoModel::Scene(scene), target);
        }
        let Some(map) = map else { return Ok(None) };
        let req = PeopleRequest::parse(0, &[]).ok_or("people request")?;
        let input = photo(&mut state.input_cache, job)?;
        return Ok(people_mask(&req, &input.rgb, input.w, input.h, Some(map), &[]).map(|g| {
            let logits = if background { invert(g.logits) } else { g.logits };
            SegMask::from_logits(g.side, &logits)
        }));
    }
    let AutoKind::People { person, parts, .. } = what else { return Err("not a People selection".into()) };
    let mut req = PeopleRequest::parse(*person, parts).ok_or("unknown People part")?;
    let faces: Vec<Face> = match &face {
        Some(dir) => {
            ensure_face(&mut state.face, &mut state.faces_cache, shared, dir)?;
            // a face FaceMesh can't fit (3/4 view, small) stays when the class map sees a face there
            cached(&state.face, &mut state.faces_cache, &mut state.input_cache, shared, job, "MediaPipe faces", |m, i| {
                m.faces_confirmed(&i.rgb, i.w, i.h, map)
            })?
            .clone()
        }
        None => Vec::new(),
    };
    if let Target::Person { rect, known, sam2, ident } = target {
        // by name or role: which of the faces it is (appearance features when the
        // examples have them)
        let wants = rect.is_none() && known.iter().any(|k| k.body.is_some() || k.face.is_some());
        // (kept with the photo's analysis: SAM 2.1 needn't encode it again)
        let kept = state
            .signatures_cache
            .as_ref()
            .is_some_and(|(k, features, s)| *k == job.key && s.len() == faces.len() && (*features || !wants || sam2.is_none()));
        if !known.is_empty() && !faces.is_empty() && !kept {
            let emb = sam2_embedding(state, shared, job, sam2.clone(), wants);
            let map = match &people {
                Some(_) => state.people_cache.as_ref().filter(|(k, _)| *k == job.key).map(|(_, m)| m),
                None => None,
            };
            let input = photo(&mut state.input_cache, job)?;
            let sigs = face_signatures(&faces, map, input, emb.as_ref());
            state.signatures_cache = Some((job.key, emb.is_some(), sigs));
        }
        // by face first, when a recognition model is in use and they have named faces
        let ident = ident.as_ref().filter(|i| !i.known.is_empty() && rect.is_none() && !faces.is_empty());
        if let Some(i) = ident
            && state.idents_cache.as_ref().is_none_or(|(k, tag, e)| *k != job.key || *tag != i.tag || e.len() != faces.len())
        {
            let input = photo(&mut state.input_cache, job)?;
            let _row = row(job, "Recognizing faces");
            let t = web_time::Instant::now();
            let e = face_embeddings(&faces, input, &i.embedder);
            log::info!("people: {} face(s) embedded in {:?}", faces.len(), t.elapsed());
            state.idents_cache = Some((job.key, i.tag.clone(), e));
        }
        let ids = ident.and_then(|i| {
            let (_, _, e) = state.idents_cache.as_ref().filter(|(k, tag, _)| *k == job.key && *tag == i.tag)?;
            Some((e.as_slice(), i.known.as_slice(), i.threshold))
        });
        let sigs = state.signatures_cache.as_ref().filter(|(k, _, _)| *k == job.key).map_or(&[][..], |(_, _, s)| s.as_slice());
        let input = photo(&mut state.input_cache, job)?;
        match pick_face(&faces, *rect, known, sigs, ids, input) {
            Pick::Face(i) => {
                log::info!("people: {} face(s), taken: number {}", faces.len(), i + 1);
                req.person = u32::try_from(i).unwrap_or(u32::MAX).saturating_add(1)
            }
            Pick::Unsure => {
                state.unsure = true;
                return Ok(None);
            }
            Pick::Absent => return Ok(None),
        }
    }
    // one person of several: their silhouette (SAM 2.1, clicked on their face and body, the
    // others' left out), so what touches them — an arm round the bride — stays the other's;
    // and everyone's figure (EoMT), so someone whose face wasn't found stays out too
    let instances = if req.person > 0 && !faces.is_empty() {
        let silhouettes = match &sam2 {
            Some(dir) if faces.len() > 1 => person_instances(state, shared, job, dir, &faces),
            _ => None,
        };
        let figures = match &scene {
            Some(dir) => scene_people(state, shared, job, dir),
            None => Vec::new(),
        };
        let (w, h) = state.input_cache.as_ref().filter(|(k, _)| *k == job.key).map_or((0, 0), |(_, i)| (i.w, i.h));
        lightcraft_segment::people::Instances::combine(SAM2_MASK_SIDE, silhouettes, &figures, &faces, w, h)
    } else {
        None
    };
    // Clothes: the floor, wall or table around the people (EoMT) is never the garment
    let surfaces = match &scene {
        Some(dir) if req.classes == [4] => scene_surfaces(state, shared, job, dir),
        _ => None,
    };
    // a group: the class map again on each small person's crop, pasted back
    if people.is_some() {
        crowd_map(state, shared, job, &faces);
    }
    let crowd = state.crowd_cache.as_ref().filter(|(k, _)| *k == job.key).and_then(|(_, m)| m.as_ref());
    let map = match &people {
        Some(_) => crowd.or_else(|| state.people_cache.as_ref().filter(|(k, _)| *k == job.key).map(|(_, m)| m)),
        None => None,
    };
    let input = state.input_cache.as_ref().filter(|(k, _)| *k == job.key).map(|(_, i)| i).ok_or("the photo is not available")?;
    let skin = req.classes.iter().any(|c| matches!(c, 2 | 3));
    let Some(g) =
        lightcraft_segment::people::people_mask_with(&req, &input.rgb, input.w, input.h, map, &faces, instances.as_ref(), surfaces.as_ref())
    else {
        return Ok(None);
    };
    if skin && face.is_none() {
        state.note = Some("Skin includes eyes, brows and mouth: the MediaPipe faces model, which finds them, isn't installed.".into());
    } else if skin && g.faces_without_features > 0 {
        state.note = Some(format!(
            "Face skin: eyes and mouth couldn't be located on {} face(s) (seen in profile), so there only hair is left out.",
            g.faces_without_features
        ));
    }
    Ok(Some(match g.rect {
        Some(r) => SegMask::from_logits_in(g.side, &g.logits, r),
        None => SegMask::from_logits(g.side, &g.logits),
    }))
}

/// What was analysed of `job`'s photo's people before (memory, else the disk), put back into the
/// worker's caches so nothing of it runs again (test notes 2026-10-25, note 10); `None`: nothing.
fn recall_people(
    state: &mut State,
    shared: &Shared,
    job: &mut Job,
    people: Option<&std::path::Path>,
    face: Option<&std::path::Path>,
    models: u64,
) -> Option<Arc<Analysis>> {
    if job.key == 0 {
        return None;
    }
    // (loading a model forgets its photo: load them first)
    if let Some(dir) = people {
        let _ = ensure(&mut state.people, &mut state.people_cache, shared, dir, "MediaPipe people", PeopleModel::load);
    }
    if let Some(dir) = face {
        let _ = ensure_face(&mut state.face, &mut state.faces_cache, shared, dir);
    }
    let k = job.key;
    let a = match state.analyses.get(k) {
        Some(a) => a,
        None => {
            let store = people_cache::Store { cache: job.analyses.clone()?, key: people_cache::key(k, models) };
            let t = web_time::Instant::now();
            let a = Arc::new(people_cache::read(&store)?);
            log::info!("people: the photo's analysis read from the preview cache in {:?}", t.elapsed());
            state.analyses.put(k, a.clone());
            a
        }
    };
    let mine = |c: Option<u64>| c != Some(k);
    if let Some(f) = &a.faces
        && mine(state.faces_cache.as_ref().map(|c| c.0))
    {
        state.faces_cache = Some((k, f.clone()));
    }
    if let Some(m) = &a.map
        && mine(state.people_cache.as_ref().map(|c| c.0))
    {
        state.people_cache = Some((k, people_cache::copy_map(m)));
        state.crowd_cache = None;
    }
    if let Some((boxes, crops)) = &a.crowd
        && mine(state.crowd_crops.as_ref().map(|c| c.0))
    {
        state.crowd_crops = Some((k, boxes.clone(), crops.iter().map(people_cache::copy_map).collect()));
    }
    if let Some(Some(s)) = &a.silhouettes
        && mine(state.silhouettes_cache.as_ref().map(|c| c.0))
    {
        state.silhouettes_cache = Some((k, s.clone()));
    }
    if let Some(f) = &a.figures
        && mine(state.figures_cache.as_ref().map(|c| c.0))
    {
        state.figures_cache = Some((k, f.clone()));
    }
    if let Some(Some(s)) = &a.surfaces
        && mine(state.surfaces_cache.as_ref().map(|c| c.0))
    {
        state.surfaces_cache = Some((k, people_cache::copy_surfaces(s)));
    }
    if let Some((features, sigs)) = &a.signatures
        && mine(state.signatures_cache.as_ref().map(|c| c.0))
    {
        state.signatures_cache = Some((k, *features, sigs.clone()));
    }
    if let Some((tag, e)) = &a.idents
        && mine(state.idents_cache.as_ref().map(|c| c.0))
    {
        state.idents_cache = Some((k, tag.clone(), e.clone()));
    }
    Some(a)
}

/// Keep what is now analysed of `job`'s photo's people (in memory, and on disk when there is a
/// preview cache), when it holds more than `before` did.
fn keep_people(state: &mut State, job: &Job, before: Option<&Analysis>, models: u64) {
    let k = job.key;
    if k == 0 {
        return;
    }
    let mine = |c: Option<u64>| c == Some(k);
    let now = Analysis {
        faces: state.faces_cache.as_ref().filter(|c| c.0 == k).map(|c| c.1.clone()),
        map: state.people_cache.as_ref().filter(|c| c.0 == k).map(|c| people_cache::copy_map(&c.1)),
        crowd: state.crowd_crops.as_ref().filter(|c| mine(Some(c.0))).map(|c| (c.1.clone(), c.2.iter().map(people_cache::copy_map).collect())),
        silhouettes: state.silhouettes_cache.as_ref().filter(|c| c.0 == k).map(|c| Some(c.1.clone())),
        figures: state.figures_cache.as_ref().filter(|c| c.0 == k).map(|c| c.1.clone()),
        surfaces: state.surfaces_cache.as_ref().filter(|c| c.0 == k).map(|c| Some(people_cache::copy_surfaces(&c.1))),
        signatures: state.signatures_cache.as_ref().filter(|c| c.0 == k).map(|c| (c.1, c.2.clone())),
        idents: state.idents_cache.as_ref().filter(|c| c.0 == k).map(|c| (c.1.clone(), c.2.clone())),
    };
    if !before.unwrap_or(&Analysis::default()).adds(&now) {
        return;
    }
    if let Some(cache) = job.analyses.clone() {
        let t = web_time::Instant::now();
        people_cache::write(&people_cache::Store { cache, key: people_cache::key(k, models) }, &now);
        log::info!("people: the photo's analysis kept in the preview cache in {:?}", t.elapsed());
    }
    state.analyses.put(k, Arc::new(now));
}

/// A [`Kind::Skin`] request: the skin of `area` (everyone's, or person `person`'s) as a stored
/// segmentation (`None`: none found), and every face found, normalized, kept in `state.found`.
fn skin(
    state: &mut State,
    shared: &Shared,
    job: &mut Job,
    area: lightcraft_develop::SkinArea,
    person: u32,
    people: &std::path::Path,
    face: &std::path::Path,
) -> Result<Option<SegMask>, String> {
    use lightcraft_develop::SkinArea;
    ensure(&mut state.people, &mut state.people_cache, shared, people, "MediaPipe people", PeopleModel::load)?;
    cached(&state.people, &mut state.people_cache, &mut state.input_cache, shared, job, "MediaPipe people", |m, i| m.segment(&i.rgb, i.w, i.h))?;
    ensure_face(&mut state.face, &mut state.faces_cache, shared, face)?;
    let seen = state.people_cache.as_ref().filter(|(k, _)| *k == job.key).map(|(_, m)| m);
    let faces: Vec<Face> = cached(&state.face, &mut state.faces_cache, &mut state.input_cache, shared, job, "MediaPipe faces", |m, i| {
        m.faces_confirmed(&i.rgb, i.w, i.h, seen)
    })?
    .clone();
    let map = state.people_cache.as_ref().filter(|(k, _)| *k == job.key).map(|(_, m)| m);
    let input = state.input_cache.as_ref().filter(|(k, _)| *k == job.key).map(|(_, i)| i).ok_or("the photo is not available")?;
    let parts: Vec<String> = match area {
        SkinArea::Face => vec!["Face Skin".into()],
        SkinArea::Body => vec!["Body Skin".into()],
        SkinArea::Both => vec!["Face Skin".into(), "Body Skin".into()],
    };
    let req = PeopleRequest::parse(person, &parts).ok_or("unknown People part")?;
    let seg = people_mask(&req, &input.rgb, input.w, input.h, map, &faces).map(|g| match g.rect {
        Some(r) => SegMask::from_logits_in(g.side, &g.logits, r),
        None => SegMask::from_logits(g.side, &g.logits),
    });
    let (w, h) = (input.w.max(1) as f32, input.h.max(1) as f32);
    state.found = Some(faces.iter().map(|f| super::FoundFace::from_pixels(f, w, h)).collect());
    Ok(seg)
}

/// A [`Kind::Retouch`] request: each part asked for (`None` where it isn't there: no teeth
/// showing, no such person), and every face found, normalized, kept in `state.found`.
fn retouch(
    state: &mut State,
    shared: &Shared,
    job: &mut Job,
    parts: &[lightcraft_develop::PartRequest],
    people: &std::path::Path,
    face: &std::path::Path,
) -> Result<Vec<lightcraft_develop::RetouchMask>, String> {
    let _row = row(job, "Finding the parts of people to retouch");
    ensure(&mut state.people, &mut state.people_cache, shared, people, "MediaPipe people", PeopleModel::load)?;
    cached(&state.people, &mut state.people_cache, &mut state.input_cache, shared, job, "MediaPipe people", |m, i| m.segment(&i.rgb, i.w, i.h))?;
    ensure_face(&mut state.face, &mut state.faces_cache, shared, face)?;
    let seen = state.people_cache.as_ref().filter(|(k, _)| *k == job.key).map(|(_, m)| m);
    let faces: Vec<Face> = cached(&state.face, &mut state.faces_cache, &mut state.input_cache, shared, job, "MediaPipe faces", |m, i| {
        m.faces_confirmed(&i.rgb, i.w, i.h, seen)
    })?
    .clone();
    let map = state.people_cache.as_ref().filter(|(k, _)| *k == job.key).map(|(_, m)| m);
    let input = state.input_cache.as_ref().filter(|(k, _)| *k == job.key).map(|(_, i)| i).ok_or("the photo is not available")?;
    let mut out = Vec::with_capacity(parts.len());
    for r in parts {
        let req = PeopleRequest::parse(r.person, &[r.part.people_name().to_string()]).ok_or("unknown People part")?;
        let seg = people_mask(&req, &input.rgb, input.w, input.h, map, &faces).map(super::retouch::seg_of);
        out.push(lightcraft_develop::RetouchMask { part: r.part, person: r.person, seg });
    }
    let (w, h) = (input.w.max(1) as f32, input.h.max(1) as f32);
    state.found = Some(faces.iter().map(|f| super::FoundFace::from_pixels(f, w, h)).collect());
    Ok(out)
}

/// The class map of `job`'s photo with a group's small people segmented again on their own crops
/// ([`lightcraft_segment::people::PeopleModel::segment_people`]), kept in `state.crowd_cache`
/// (`None` there: nobody small enough to need it, or it failed — the whole-image map is used).
/// The crops come from the sharper render (`job.fine`) when there is one.
fn crowd_map(state: &mut State, shared: &Shared, job: &mut Job, faces: &[Face]) {
    if state.crowd_cache.as_ref().is_some_and(|(k, _)| *k == job.key) {
        return;
    }
    state.crowd_cache = None;
    // the crops kept with the photo's analysis: pasted again
    if let (Some((k, boxes, crops)), Some((m, whole))) = (&state.crowd_crops, &state.people_cache)
        && *k == job.key
        && *m == job.key
    {
        let pasted = (!boxes.is_empty()).then(|| lightcraft_segment::people::paste_crops(whole, boxes, crops, CROWD_SIDE));
        state.crowd_cache = Some((job.key, pasted));
        return;
    }
    state.crowd_crops = None;
    let Some((w, h)) = state.input_cache.as_ref().filter(|(k, _)| *k == job.key).map(|(_, i)| (i.w, i.h)) else { return };
    let boxes = lightcraft_segment::people::person_boxes(faces, w, h);
    if boxes.is_empty() {
        state.crowd_cache = Some((job.key, None));
        state.crowd_crops = Some((job.key, Vec::new(), Vec::new()));
        return;
    }
    let _busy = Flag::raise(&shared.encoding);
    let _row = row(job, "Finding each person of a group");
    let t = web_time::Instant::now();
    // the sharper render, else the model input
    let fine = match job.fine.take().map(|r| r.run().rendered) {
        Some(Ok(r)) => {
            let img = r.image;
            Some(Input { rgb: img.data.iter().flat_map(|p| [p[0], p[1], p[2]]).collect(), w: img.width, h: img.height })
        }
        Some(Err(e)) => {
            log::warn!("people: no sharper render for the group's crops ({e}); cropping the model input");
            None
        }
        None => None,
    };
    let rendered = t.elapsed();
    let (Some((_, model)), Some((_, whole)), Some((_, input))) = (&state.people, &state.people_cache, &state.input_cache) else { return };
    let src = fine.as_ref().unwrap_or(input);
    let out = model.segment_crops(&src.rgb, src.w, src.h, &boxes).map(|(kept, crops)| {
        let pasted = lightcraft_segment::people::paste_crops(whole, &kept, &crops, CROWD_SIDE);
        (kept, crops, pasted)
    });
    log::info!(
        "people: {} small people cropped from {}×{} (render {:?}, crops {:?})",
        boxes.len(),
        src.w,
        src.h,
        rendered,
        t.elapsed().saturating_sub(rendered)
    );
    match out {
        Ok((kept, crops, m)) => {
            state.crowd_cache = Some((job.key, Some(m)));
            state.crowd_crops = Some((job.key, kept, crops));
        }
        Err(e) => {
            log::warn!("people: the group's crops failed ({e}); using the whole photo's map");
            state.crowd_cache = Some((job.key, None));
        }
    }
}

/// Side of the class map a group's crops are pasted into.
const CROWD_SIDE: usize = 1024;

/// Everyone EoMT sees as a person on `job`'s photo, as `SAM2_MASK_SIDE²` logits each (empty, with
/// a log line, when EoMT can't run).
fn scene_people(state: &mut State, shared: &Shared, job: &mut Job, dir: &std::path::Path) -> Vec<Vec<f32>> {
    if let Some((k, f)) = &state.figures_cache
        && *k == job.key
    {
        return f.clone();
    }
    let scene = ensure(&mut state.scene, &mut state.scene_cache, shared, dir, "EoMT", SceneModel::load)
        .and_then(|()| cached(&state.scene, &mut state.scene_cache, &mut state.input_cache, shared, job, "EoMT", |m, i| m.segment(&i.rgb, i.w, i.h)));
    match scene {
        Ok(scene) => {
            let person = lightcraft_segment::scene::class_ids(|n| n == "person");
            let figures: Vec<Vec<f32>> = scene
                .instances
                .iter()
                .filter(|i| person.contains(&i.class))
                .map(|i| lightcraft_segment::grid::to_square_logits(&i.mask, scene.width, scene.height, SAM2_MASK_SIDE))
                .collect();
            state.figures_cache = Some((job.key, figures.clone()));
            figures
        }
        Err(e) => {
            log::warn!("people: no figures from EoMT ({e})");
            Vec::new()
        }
    }
}

/// The floors, walls and tables EoMT sees on `job`'s photo, where no one is (`None`, with a log
/// line, when EoMT can't run: the garment is then cut by the people model alone).
fn scene_surfaces(state: &mut State, shared: &Shared, job: &mut Job, dir: &std::path::Path) -> Option<Surfaces> {
    if let Some((k, s)) = &state.surfaces_cache
        && *k == job.key
    {
        return Some(people_cache::copy_surfaces(s));
    }
    let scene = ensure(&mut state.scene, &mut state.scene_cache, shared, dir, "EoMT", SceneModel::load)
        .and_then(|()| cached(&state.scene, &mut state.scene_cache, &mut state.input_cache, shared, job, "EoMT", |m, i| m.segment(&i.rgb, i.w, i.h)));
    match scene {
        Ok(scene) => {
            let s = Surfaces::from_scene(scene);
            state.surfaces_cache = Some((job.key, people_cache::copy_surfaces(&s)));
            Some(s)
        }
        Err(e) => {
            log::warn!("clothes: no floors or walls from EoMT ({e})");
            None
        }
    }
}

/// Everyone's silhouette (SAM 2.1, for each face: clicks on it and down its body, the other faces
/// left out), so where people overlap a pixel goes to whoever's silhouette claims it most; `None`
/// (with a log line) when SAM 2.1 can't give them.
fn person_instances(state: &mut State, shared: &Shared, job: &mut Job, dir: &std::path::Path, faces: &[Face]) -> Option<Vec<Vec<f32>>> {
    if let Some((k, s)) = &state.silhouettes_cache
        && *k == job.key
        && s.len() == faces.len()
    {
        return Some(s.clone());
    }
    let input = state.input_cache.as_ref().filter(|(k, _)| *k == job.key).map(|(_, i)| (i.w, i.h))?;
    let (w, h) = (input.0.max(1) as f32, input.1.max(1) as f32);
    let centre = |f: &Face| {
        let d = &f.detection;
        (((d.x0 + d.x1) / 2.0 / w).clamp(0.0, 1.0), ((d.y0 + d.y1) / 2.0 / h).clamp(0.0, 1.0), ((d.y1 - d.y0) / h).max(0.0))
    };
    let keep = std::mem::replace(&mut job.dir, dir.to_path_buf());
    let out = sam2_on_input(state, shared, job).and_then(|(m, enc, _)| {
        let mut people = Vec::with_capacity(faces.len());
        for (k, f) in faces.iter().enumerate() {
            let (cx, cy, fh) = centre(f);
            // their face, chest and middle (below the frame: left out)
            let mut clicks: Vec<lightcraft_segment::Click> = [0.0, 1.6, 3.2]
                .iter()
                .map(|d| cy + d * fh)
                .filter(|y| *y < 0.98)
                .map(|y| lightcraft_segment::Click { x: cx, y, positive: true })
                .collect();
            for (j, o) in faces.iter().enumerate() {
                if j != k {
                    let (ox, oy, _) = centre(o);
                    clicks.push(lightcraft_segment::Click { x: ox, y: oy, positive: false });
                }
            }
            let p = m.segment_clicks_with(enc, &clicks, false).map_err(|e| e.to_string())?;
            let share = p.logits.iter().filter(|l| **l > -1.0).count() as f32 / p.logits.len().max(1) as f32;
            log::info!("people: person {} of {}: silhouette {:.0}% of the photo", k + 1, faces.len(), share * 100.0);
            people.push(p.logits);
        }
        Ok(people)
    });
    job.dir = keep;
    match out {
        Ok(people) => {
            state.silhouettes_cache = Some((job.key, people.clone()));
            Some(people)
        }
        Err(e) => {
            log::warn!("people: no silhouettes from SAM 2.1 ({e})");
            None
        }
    }
}

/// A face's box in normalized photo coordinates.
fn norm_box(f: &Face, w: usize, h: usize) -> [f64; 4] {
    let (w, h) = (w.max(1) as f64, h.max(1) as f64);
    let d = &f.detection;
    [f64::from(d.x0) / w, f64::from(d.y0) / h, f64::from(d.x1) / w, f64::from(d.y1) / h].map(|v| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 })
}

fn iou(a: [f64; 4], b: [f64; 4]) -> f64 {
    let ix = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
    let iy = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    let inter = ix * iy;
    let union = (a[2] - a[0]).max(0.0) * (a[3] - a[1]).max(0.0) + (b[2] - b[0]).max(0.0) * (b[3] - b[1]).max(0.0) - inter;
    if union > 0.0 { inter / union } else { 0.0 }
}

/// Faces whose appearance scores at least this against a person's examples are taken for them.
pub const PERSON_MATCH: f32 = 0.55;
/// …and only when they look this much more like them than anyone else on the photo does (a
/// mother and her daughter in the same pyjamas: test notes 2026-10-25, note 12).
pub const LOOK_MARGIN: f32 = 0.15;
/// Someone looking at least this much like them makes the photo "not sure" rather than "not
/// there".
pub const LOOK_UNSURE: f32 = 0.35;
/// A face's identity (cosine of face-recognition embeddings) must lead the next face's by this.
pub const FACE_MARGIN: f32 = 0.08;
/// How far below the recognition model's match threshold a face still makes the photo "not
/// sure".
pub const FACE_UNSURE: f32 = 0.12;

/// How sure the pick of a named person among a photo's faces is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    /// This face (index) is them.
    Face(usize),
    /// Someone may be them, but not clearly more than someone else: no guess.
    Unsure,
    /// Nobody here looks like them.
    Absent,
}

/// How much one face is like the person: by the face itself (`face`: cosine similarity of
/// face-recognition embeddings, `None` without a recognition model or an example it embedded),
/// and by appearance (`look`: hair, clothes, build; 0..1).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Likeness {
    pub face: Option<f32>,
    pub look: f32,
}

/// Who of the faces (`c`, one likeness each) the person is. The face decides when it can (a
/// recognition model's cosine of at least `face_match`, ahead of the next face's by
/// [`FACE_MARGIN`]); appearance only breaks a tie between faces alike by face, or decides alone
/// without a recognition model (at least [`PERSON_MATCH`], ahead by [`LOOK_MARGIN`]). Never a
/// guess: someone close but not clearly the one is [`Pick::Unsure`].
pub fn choose(c: &[Likeness], face_match: f32) -> Pick {
    let finite = |v: f32| if v.is_finite() { v } else { 0.0 };
    let by_look = |a: &(usize, &Likeness), b: &(usize, &Likeness)| finite(b.1.look).total_cmp(&finite(a.1.look));
    if c.iter().any(|l| l.face.is_some_and(f32::is_finite)) {
        let face = |l: &Likeness| finite(l.face.unwrap_or(-1.0));
        let mut ranked: Vec<(usize, &Likeness)> = c.iter().enumerate().collect();
        ranked.sort_by(|a, b| face(b.1).total_cmp(&face(a.1)));
        let Some(&(best, top)) = ranked.first() else { return Pick::Absent };
        let next = ranked.get(1).map_or(-1.0, |(_, l)| face(l));
        if face(top) >= face_match {
            if face(top) - next >= FACE_MARGIN {
                return Pick::Face(best);
            }
            // alike by face (twins, a blurred pair): their looks may tell them apart
            let mut close: Vec<(usize, &Likeness)> = ranked.iter().copied().filter(|(_, l)| face(top) - face(l) < FACE_MARGIN).collect();
            close.sort_by(by_look);
            return match (close.first(), close.get(1)) {
                (Some((i, a)), Some((_, b))) if finite(a.look) - finite(b.look) >= LOOK_MARGIN => Pick::Face(*i),
                _ => Pick::Unsure,
            };
        }
        // nobody clearly them by face: a face that came close, or a face the model couldn't read
        // (a profile) that looks like them, keeps the photo open
        let near = face(top) >= face_match - FACE_UNSURE;
        let unread = c.iter().any(|l| l.face.is_none() && finite(l.look) >= LOOK_UNSURE);
        return if near || unread { Pick::Unsure } else { Pick::Absent };
    }
    let mut ranked: Vec<(usize, &Likeness)> = c.iter().enumerate().collect();
    ranked.sort_by(by_look);
    let Some(&(best, top)) = ranked.first() else { return Pick::Absent };
    let next = ranked.get(1).map_or(0.0, |(_, l)| finite(l.look));
    let look = finite(top.look);
    if look >= PERSON_MATCH && look - next >= LOOK_MARGIN {
        Pick::Face(best)
    } else if look >= LOOK_UNSURE {
        Pick::Unsure
    } else {
        Pick::Absent
    }
}

/// Which of `faces` is the person: the one overlapping their named face box, else the one their
/// examples recognise ([`choose`]).
fn pick_face(
    faces: &[Face],
    rect: Option<[f64; 4]>,
    known: &[Signature],
    sigs: &[Option<Signature>],
    ids: Option<(&[Option<Vec<f32>>], &[Arc<[f32]>], f32)>,
    input: &Input,
) -> Pick {
    if let Some(r) = rect {
        let centre = [(r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0];
        let best = faces
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let b = norm_box(f, input.w, input.h);
                let inside = (b[0]..=b[2]).contains(&centre[0]) && (b[1]..=b[3]).contains(&centre[1]);
                (i, iou(b, r).max(if inside { 0.25 } else { 0.0 }))
            })
            .max_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((i, s)) = best
            && s >= 0.2
        {
            return Pick::Face(i);
        }
    }
    let by_face = ids.is_some_and(|(_, k, _)| !k.is_empty());
    if (known.is_empty() && !by_face) || faces.is_empty() {
        return Pick::Absent;
    }
    let likeness: Vec<Likeness> = (0..faces.len())
        .map(|i| Likeness {
            face: ids.and_then(|(e, k, _)| face_score(e.get(i)?.as_deref()?, k)),
            look: match sigs.get(i) {
                Some(Some(sig)) if !known.is_empty() => look_score(sig, known),
                _ => 0.0,
            },
        })
        .collect();
    let pick = choose(&likeness, ids.map_or(1.0, |(_, _, t)| t));
    log::info!("people: likeness {:?} → {pick:?}", likeness.iter().map(|l| (l.face, l.look)).collect::<Vec<_>>());
    pick
}

/// How much a face's embedding `e` is the person's (their named faces `known`): the best cosine
/// (`None`: no comparable embedding).
fn face_score(e: &[f32], known: &[Arc<[f32]>]) -> Option<f32> {
    known.iter().filter_map(|k| lightcraft_faces::matching::cosine(e, k)).filter(|c| c.is_finite()).max_by(f32::total_cmp)
}

/// FaceMesh's points for the five of the recognition template: the eyes' centres (irises), the
/// nose tip and the mouth's corners.
const TEMPLATE_POINTS: [usize; 5] = [468, 473, 1, 61, 291];

/// A face without landmarks is cut with this much room around its box (as the People index cuts
/// a region without them).
const FACE_BOX_PAD: f32 = 1.5;

/// Each face's embedding by `embedder` (`None` where it couldn't be cut out or run): aligned on
/// FaceMesh's landmarks like the People index aligns them, else cut by its box with the room
/// the index gives a box.
fn face_embeddings(faces: &[Face], input: &Input, embedder: &lightcraft_faces::runtime::Embedder) -> Vec<Option<Vec<f32>>> {
    use lightcraft_faces::align::{Rgb, align_to_template, crop_box};
    let img = Rgb { data: &input.rgb, width: input.w, height: input.h };
    let (iw, ih) = embedder.input_size();
    let crops: Vec<Option<Vec<u8>>> = faces
        .iter()
        .map(|f| {
            let points = f.landmarks.as_ref().and_then(|l| {
                let p = TEMPLATE_POINTS.map(|i| l.get(i).map(|q| (q[0], q[1])));
                let [Some(a), Some(b), Some(n), Some(m0), Some(m1)] = p else { return None };
                // the image's left eye and mouth corner first, whichever way the face turns
                let (l, r) = if a.0 <= b.0 { (a, b) } else { (b, a) };
                let (ml, mr) = if m0.0 <= m1.0 { (m0, m1) } else { (m1, m0) };
                Some([l, r, n, ml, mr])
            });
            match points {
                Some(p) => align_to_template(&img, &p, iw, ih),
                None => {
                    let d = &f.detection;
                    crop_box(&img, d.x0, d.y0, d.x1, d.y1, FACE_BOX_PAD, iw, ih)
                }
            }
        })
        .collect();
    let ready: Vec<&[u8]> = crops.iter().filter_map(|c| c.as_deref()).collect();
    let mut out = embedder.embed_batch(&ready).into_iter();
    crops.iter().map(|c| c.as_ref().and_then(|_| out.next().and_then(Result::ok))).collect()
}

/// Each face's appearance signature (`None` where there is none to take), with SAM 2.1's
/// features from `emb` when there is one.
fn face_signatures(faces: &[Face], map: Option<&PeopleMap>, input: &Input, emb: Option<&(Vec<f32>, usize, usize)>) -> Vec<Option<Signature>> {
    (0..faces.len())
        .map(|i| {
            let mut sig = lightcraft_segment::identity::signature(faces, i, map, &input.rgb, input.w, input.h)?;
            if let Some((e, side, dim)) = emb {
                (sig.body, sig.face) = lightcraft_segment::identity::features(e, *side, *dim, faces, i, map, input.w, input.h);
            }
            Some(sig)
        })
        .collect()
}

/// How much `sig` looks like the examples `known` (appearance only): the mean of its two best
/// scores (robust to one odd example).
fn look_score(sig: &Signature, known: &[Signature]) -> f32 {
    let probe = Signature { embedding: None, ..sig.clone() };
    let mut s: Vec<f32> = known
        .iter()
        // (an example's identity embedding is the face's business, not the look's)
        .map(|k| lightcraft_segment::identity::similarity(&probe, &Signature { embedding: None, ..k.clone() }))
        .collect();
    s.sort_by(|a, b| b.total_cmp(a));
    let top: Vec<f32> = s.into_iter().take(2).collect();
    if top.is_empty() { 0.0 } else { top.iter().sum::<f32>() / top.len() as f32 }
}

/// The faces of `job`'s photo with their appearance signatures (`people.faces`).
fn found_faces(
    state: &mut State,
    shared: &Shared,
    job: &mut Job,
    face: &std::path::Path,
    people: Option<&std::path::Path>,
    sam2: Option<PathBuf>,
) -> Result<Vec<FoundFace>, String> {
    // the class map first: it confirms faces FaceMesh can't fit (see `faces_confirmed`)
    if let Some(dir) = people {
        ensure(&mut state.people, &mut state.people_cache, shared, dir, "MediaPipe people", PeopleModel::load)?;
        cached(&state.people, &mut state.people_cache, &mut state.input_cache, shared, job, "MediaPipe people", |m, i| m.segment(&i.rgb, i.w, i.h))?;
    }
    ensure_face(&mut state.face, &mut state.faces_cache, shared, face)?;
    let seen = people.and(state.people_cache.as_ref().filter(|(k, _)| *k == job.key).map(|(_, m)| m));
    let faces = cached(&state.face, &mut state.faces_cache, &mut state.input_cache, shared, job, "MediaPipe faces", |m, i| {
        m.faces_confirmed(&i.rgb, i.w, i.h, seen)
    })?
    .clone();
    let emb = sam2_embedding(state, shared, job, sam2, !faces.is_empty());
    let map = people.and(state.people_cache.as_ref().filter(|(k, _)| *k == job.key).map(|(_, m)| m));
    let input = state.input_cache.as_ref().filter(|(k, _)| *k == job.key).map(|(_, i)| i).ok_or("the photo is not available")?;
    Ok(faces
        .iter()
        .enumerate()
        .filter_map(|(i, f)| {
            let mut signature = lightcraft_segment::identity::signature(&faces, i, map, &input.rgb, input.w, input.h)?;
            if let Some((e, side, dim)) = &emb {
                (signature.body, signature.face) = lightcraft_segment::identity::features(e, *side, *dim, &faces, i, map, input.w, input.h);
            }
            Some(FoundFace { rect: norm_box(f, input.w, input.h), score: f.detection.score, signature, landmarks: f.landmarks.is_some() })
        })
        .collect())
}

/// SAM 2.1's embedding of `job`'s photo (`(values, side, channels)`), when `dir` is given and
/// `wanted`; `None` (and a log line) when it can't be had: appearance then rests on colours.
fn sam2_embedding(state: &mut State, shared: &Shared, job: &mut Job, dir: Option<PathBuf>, wanted: bool) -> Option<(Vec<f32>, usize, usize)> {
    let dir = dir.filter(|_| wanted)?;
    let keep = std::mem::replace(&mut job.dir, dir);
    let out = sam2_on_input(state, shared, job).and_then(|(_, enc, _)| enc.embedding().map_err(|e| e.to_string()));
    job.dir = keep;
    match out {
        Ok(e) => Some(e),
        Err(e) => {
            log::warn!("people: no SAM 2.1 features ({e})");
            None
        }
    }
}

/// SAM 2.1 (from `job.dir`) and the photo encoded by it, from the open models' shared input.
fn sam2_on_input<'a>(state: &'a mut State, shared: &Shared, job: &mut Job) -> Result<(&'a Sam2, &'a Sam2Encoded, &'a Input), String> {
    if state.sam2.as_ref().is_none_or(|(d, _)| *d != job.dir) {
        state.sam2_cache = None;
    }
    load(&mut state.sam2, shared, &job.dir, "SAM 2.1", Sam2::load)?;
    photo(&mut state.input_cache, job)?;
    if state.sam2_cache.as_ref().is_none_or(|(k, _)| *k != job.key) {
        state.sam2_cache = None;
        let input = state.input_cache.as_ref().map(|(_, i)| i).ok_or("the photo is not available")?;
        let _busy = Flag::raise(&shared.encoding);
        let _row = row(job, "SAM 2.1");
        let m = state.sam2.as_ref().map(|(_, m)| m).ok_or("SAM 2.1 did not load")?;
        let t = web_time::Instant::now();
        let enc = m.encode(&input.rgb, input.w, input.h).map_err(|e| e.to_string())?;
        log::info!("SAM 2.1 encoded {}×{} in {:?}", input.w, input.h, t.elapsed());
        state.sam2_cache = Some((job.key, enc));
    }
    match (&state.sam2, &state.sam2_cache, &state.input_cache) {
        (Some((_, m)), Some((_, e)), Some((_, i))) => Ok((m, e, i)),
        _ => Err("SAM 2.1 is not ready".into()),
    }
}

/// The alpha of `mask` over the model input, on a grid of about `edge` cells on the long side
/// (the mask's coordinates are the uncropped photo's, which the input is).
pub(crate) fn mask_alpha(mask: &lightcraft_develop::Mask, input: &Input, edge: usize) -> lightcraft_raster::Plane {
    let (w, h) = (input.w.max(1), input.h.max(1));
    let s = edge.max(8) as f64 / w.max(h) as f64;
    let (gw, gh) = (((w as f64 * s).round() as usize).max(1), ((h as f64 * s).round() as usize).max(1));
    let lin = |v: u8| {
        let c = f32::from(v) / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    let px = |x: usize, y: usize| -> [f32; 3] {
        let (sx, sy) = ((x * w / gw).min(w - 1), (y * h / gh).min(h - 1));
        let i = (sy * w + sx) * 3;
        match input.rgb.get(i..i + 3) {
            Some(p) => [lin(p[0]), lin(p[1]), lin(p[2])],
            None => [0.0; 3],
        }
    };
    let img = lightcraft_raster::Rgb32f::from_fn(gw, gh, px);
    let log_l = lightcraft_raster::Plane::from_fn(gw, gh, |x, y| {
        let p = img.data.get(y * gw + x).copied().unwrap_or_default();
        (0.2627 * p[0] + 0.678 * p[1] + 0.0593 * p[2]).max(1e-6).log2()
    });
    let frame = lightcraft_pipeline::geometry::Frame::new(gw, gh, &lightcraft_develop::DevelopSettings::default(), false);
    let mut m = mask.clone();
    m.visible = true;
    m.adjust.amount = 100.0;
    lightcraft_pipeline::masks::evaluate_one(&m, &frame, gw, gh, &img, &log_l, 0.0, None)
}

/// Learn an example of a remembered object from `mask` on the photo.
fn learn(
    state: &mut State,
    shared: &Shared,
    job: &mut Job,
    mask: &lightcraft_develop::Mask,
) -> Result<Option<lightcraft_segment::concept::Example>, String> {
    let (_, enc, input) = sam2_on_input(state, shared, job)?;
    let alpha = mask_alpha(mask, input, 128);
    let (emb, es, dim) = enc.embedding().map_err(|e| e.to_string())?;
    let (aw, ah) = (alpha.width.max(1), alpha.height.max(1));
    let at = |u: f32, v: f32| {
        let x = ((u * aw as f32) as usize).min(aw - 1);
        let y = ((v * ah as f32) as usize).min(ah - 1);
        alpha.data.get(y * aw + x).copied().unwrap_or(0.0)
    };
    Ok(lightcraft_segment::concept::learn(&emb, es, dim, at, &input.rgb, input.w, input.h))
}

/// The probability of `what` at each cell of `scene`'s grid; `None` when it isn't there.
fn scene_probs(scene: &Scene, what: &AutoKind) -> Option<Vec<f32>> {
    use lightcraft_segment::scene::class_ids;
    let person = class_ids(|n| n == "person");
    let found = |p: &[f32]| p.iter().any(|v| *v > 0.5);
    match what {
        AutoKind::Sky => Some(scene.class_probability(&class_ids(|n| n.starts_with("sky")))),
        AutoKind::Landscape { class } => {
            let names = landscape_classes(class);
            Some(scene.class_probability(&class_ids(|n| names.contains(&n))))
        }
        AutoKind::People { who: Some(_), .. } | AutoKind::Concept { .. } => None,
        AutoKind::People { person: 0, .. } => Some(scene.class_probability(&person)).filter(|p| found(p)),
        AutoKind::People { person: n, .. } => {
            // people counted from the left (by the centre of their mask)
            let mut people: Vec<(f32, &Vec<f32>)> = scene
                .instances
                .iter()
                .filter(|i| person.contains(&i.class))
                .map(|i| {
                    let (mut sx, mut sw) = (0.0f32, 0.0f32);
                    for (k, v) in i.mask.iter().enumerate() {
                        sx += v * (k % scene.width.max(1)) as f32;
                        sw += v;
                    }
                    (if sw > 0.0 { sx / sw } else { 0.0 }, &i.mask)
                })
                .collect();
            people.sort_by(|a, b| a.0.total_cmp(&b.0));
            people.get((*n as usize).saturating_sub(1)).map(|(_, m)| (*m).clone())
        }
        // a subject without BiRefNet: the people, else the most confident thing
        AutoKind::Subject | AutoKind::Background => {
            let p = scene.class_probability(&person);
            if found(&p) {
                return Some(p);
            }
            let things = class_ids(|n| !n.contains('-') && !matches!(n, "sand" | "sea" | "river" | "road" | "snow" | "gravel" | "house" | "roof"));
            scene.instances.iter().filter(|i| things.contains(&i.class)).max_by(|a, b| a.score.total_cmp(&b.score)).map(|i| i.mask.clone())
        }
    }
}

/// Render the model input.
fn render(job: &mut Job) -> Result<Input, String> {
    let r = job.render.take().ok_or("the photo is not available")?.run();
    let img = match r.rendered {
        Ok(r) => r.image,
        Err(e) => {
            return Err(match &job.missing {
                Some(path) => format!(
                    "This photo's file is missing (moved or deleted), so it can't be analyzed for AI masks: {path}. Library ▸ Find Missing Photos can relink it."
                ),
                None => format!("couldn't render the photo for AI masks: {e}"),
            });
        }
    };
    let rgb = img.data.iter().flat_map(|p| [p[0], p[1], p[2]]).collect();
    Ok(Input { rgb, w: img.width, h: img.height })
}

/// The model and the encoded photo `job.key` (rendering and encoding it when needed).
fn encoded<'a>(state: &'a mut State, shared: &Shared, job: &mut Job, placement: Placement) -> Result<(&'a mut Sam3, &'a mut Encoded), Failure> {
    model(state, shared, &job.dir, placement)?;
    if state.cache.as_ref().is_none_or(|(k, _)| *k != job.key) {
        state.cache = None;
        let input = render(job)?;
        let _busy = Flag::raise(&shared.encoding);
        let _row = row(job, "SAM 3");
        let m = state.model.as_mut().map(|(_, m)| m).ok_or("the model did not load")?;
        let t = web_time::Instant::now();
        let mut enc = m.encode(&input.rgb, input.w, input.h)?;
        m.prepare_clicks(&mut enc)?;
        log::info!("SAM 3 encoded {}×{} in {:?}", input.w, input.h, t.elapsed());
        state.cache = Some((job.key, enc));
    }
    let State { model, cache, .. } = state;
    match (model.as_mut(), cache.as_mut()) {
        (Some((_, m)), Some((_, e))) => Ok((m, e)),
        _ => Err("the model is not ready".into()),
    }
}

fn remote_job(state: &mut State, shared: &Shared, job: &mut Job, endpoint: &str) -> Result<Option<SegMask>, String> {
    let _busy = Flag::raise(&shared.encoding);
    if state.remote.is_none() {
        state.remote = Some(remote::Client::connect(endpoint)?);
    }
    let client = state.remote.as_mut().ok_or("remote SAM unavailable")?;
    let mut region = None;
    let operation = match std::mem::replace(&mut job.kind, Kind::Prepare) {
        Kind::Prepare => Operation::Prepare,
        Kind::Text(text) => Operation::Text(text),
        Kind::Clicks(clicks) => Operation::Clicks(clicks.iter().map(|c| [c.x, c.y, if c.positive { 1.0 } else { 0.0 }]).collect()),
        Kind::Detail { prompt, region: r } => {
            region = Some(r);
            let (rw, rh) = (r[2] - r[0], r[3] - r[1]);
            if !(rw > 0.0 && rh > 0.0) {
                return Ok(None);
            }
            match prompt {
                Prompt::Text(text) => Operation::Text(text),
                Prompt::Clicks(clicks) => {
                    let inside: Vec<_> = clicks
                        .iter()
                        .map(|c| [((c.at.x - r[0]) / rw) as f32, ((c.at.y - r[1]) / rh) as f32, if c.include { 1.0 } else { 0.0 }])
                        .filter(|p| (0.0..=1.0).contains(&p[0]) && (0.0..=1.0).contains(&p[1]))
                        .collect();
                    if !inside.iter().any(|p| p[2] == 1.0) {
                        return Ok(None);
                    }
                    Operation::Clicks(inside)
                }
            }
        }
        Kind::Auto { .. } | Kind::Skin { .. } | Kind::Retouch { .. } | Kind::Faces { .. } | Kind::Learn { .. } => {
            return Err("automatic selections run locally".into());
        }
        #[cfg(test)]
        Kind::Panic | Kind::FailOnGpu { .. } => return Err("test panic request".into()),
    };
    let input = if region.is_some() || client.key != Some(job.key) { render(job)? } else { Input { rgb: Vec::new(), w: 0, h: 0 } };
    let request = Request { width: input.w, height: input.h, temporary: region.is_some(), operation };
    let logits = client.request(&request, &input.rgb)?;
    if region.is_none() {
        client.key = Some(job.key);
    }
    Ok(logits.map(|l| match region {
        Some(r) => SegMask::from_logits_in(lightcraft_segment::MASK_SIDE, &l, r),
        None => SegMask::from_logits(lightcraft_segment::MASK_SIDE, &l),
    }))
}

fn run_job(state: &mut State, shared: &Shared, job: &mut Job, placement: Placement, payload: &mut Payload) -> Result<Option<SegMask>, Failure> {
    if let Kind::Auto { what, model, target } = &job.kind {
        let (what, model, target) = (what.clone(), model.clone(), target.clone());
        state.unsure = false;
        let out = auto(state, shared, job, &what, model, &target)?;
        if std::mem::take(&mut state.unsure) {
            *payload = Payload::Unsure;
        }
        return Ok(out);
    }
    if let Kind::Faces { face, people, sam2 } = &job.kind {
        let (face, people, sam2) = (face.clone(), people.clone(), sam2.clone());
        *payload = Payload::Faces(found_faces(state, shared, job, &face, people.as_deref(), sam2)?);
        return Ok(None);
    }
    if let Kind::Learn { mask } = &job.kind {
        let mask = mask.clone();
        if let Some(e) = learn(state, shared, job, &mask)? {
            *payload = Payload::Example(Box::new(e));
        }
        return Ok(None);
    }
    if let Kind::Retouch { parts, people, face } = &job.kind {
        let (parts, people, face) = (parts.clone(), people.clone(), face.clone());
        *payload = Payload::Retouch(retouch(state, shared, job, &parts, &people, &face)?);
        return Ok(None);
    }
    if let Kind::Skin { area, person, people, face } = &job.kind {
        let (area, person, people, face) = (*area, *person, people.clone(), face.clone());
        return Ok(skin(state, shared, job, area, person, &people, &face)?);
    }
    if job.sam2 {
        return Ok(sam2_job(state, shared, job)?);
    }
    if let Some(endpoint) = remote::configured() {
        let result = remote_job(state, shared, job, &endpoint);
        if result.is_err() {
            state.remote = None;
        }
        return result.map_err(|e| Failure::Other(format!("Remote SAM 3 failed: {e}. Check the worker and SSH tunnel, then retry.")));
    }
    let side = lightcraft_segment::MASK_SIDE;
    match std::mem::replace(&mut job.kind, Kind::Prepare) {
        Kind::Prepare => {
            encoded(state, shared, job, placement)?;
            Ok(None)
        }
        Kind::Clicks(clicks) => {
            let (m, enc) = encoded(state, shared, job, placement)?;
            let p = m.segment_clicks(enc, &clicks)?;
            Ok(Some(SegMask::from_logits(side, &p.logits)))
        }
        Kind::Text(text) => {
            let (m, enc) = encoded(state, shared, job, placement)?;
            Ok(text_logits(m, enc, &text)?.map(|l| SegMask::from_logits(side, &l)))
        }
        Kind::Auto { .. } | Kind::Skin { .. } | Kind::Retouch { .. } | Kind::Faces { .. } | Kind::Learn { .. } => {
            Err("an automatic selection reached the SAM 3 path".into())
        }
        #[cfg(test)]
        #[allow(clippy::panic)]
        Kind::Panic => panic!("a test panic"),
        #[cfg(test)]
        #[allow(clippy::panic)]
        Kind::FailOnGpu { panic } => match placement {
            Placement::Gpu if panic => panic!("a test panic on the GPU"),
            Placement::Gpu => Err(Failure::Model("a test error on the GPU".into())),
            Placement::Cpu => Ok(None),
        },
        Kind::Detail { prompt, region } => {
            let input = render(job)?;
            let m = model(state, shared, &job.dir, placement)?;
            let t = web_time::Instant::now();
            let mut enc = m.encode(&input.rgb, input.w, input.h)?;
            let (rw, rh) = (region[2] - region[0], region[3] - region[1]);
            if !(rw > 0.0 && rh > 0.0) {
                return Ok(None);
            }
            let logits = match prompt {
                Prompt::Clicks(clicks) => {
                    let Some(inside) = clicks_in(&clicks, region) else { return Ok(None) };
                    Some(m.segment_clicks(&mut enc, &inside)?.logits)
                }
                Prompt::Text(text) => text_logits(m, &mut enc, &text)?,
            };
            log::info!("SAM 3 detail pass ({}×{}) in {:?}", input.w, input.h, t.elapsed());
            Ok(logits.map(|l| SegMask::from_logits_in(side, &l, region)))
        }
    }
}

/// Describe on the open models: OWLv2's boxes for `text` on `job`'s photo (`detector`: its folder),
/// each segmented by SAM 2.1 (`job.dir`), merged; both analyses of the photo are kept for the
/// next description. `None` when nothing matches.
fn describe(state: &mut State, shared: &Shared, job: &mut Job, detector: &std::path::Path, text: &str) -> Result<Option<SegMask>, String> {
    sam2_on_input(state, shared, job)?;
    ensure(&mut state.owl, &mut state.owl_cache, shared, detector, "OWLv2", Owl::load)?;
    cached(&state.owl, &mut state.owl_cache, &mut state.input_cache, shared, job, "OWLv2", |m, i| m.encode(&i.rgb, i.w, i.h))?;
    let (Some((_, owl)), Some((_, owl_enc)), Some((_, sam)), Some((_, sam_enc))) = (&state.owl, &state.owl_cache, &state.sam2, &state.sam2_cache)
    else {
        return Err("the Describe models are not ready".into());
    };
    let t = web_time::Instant::now();
    let p = lightcraft_segment::describe::describe(owl, owl_enc, sam, sam_enc, text, &Default::default()).map_err(|e| e.to_string())?;
    log::info!("Describe {text:?} in {:?}", t.elapsed());
    Ok(p.map(|p| SegMask::from_logits(SAM2_MASK_SIDE, &p.logits)))
}

/// Object clicks, their preparation and detail passes on SAM 2.1; Describe on OWLv2 and SAM 2.1.
fn sam2_job(state: &mut State, shared: &Shared, job: &mut Job) -> Result<Option<SegMask>, String> {
    let side = SAM2_MASK_SIDE;
    match std::mem::replace(&mut job.kind, Kind::Prepare) {
        Kind::Prepare => {
            sam2_encoded(state, shared, job)?;
            Ok(None)
        }
        Kind::Clicks(clicks) => {
            let (m, enc) = sam2_encoded(state, shared, job)?;
            let p = m.segment_clicks(enc, &clicks).map_err(|e| e.to_string())?;
            Ok(Some(SegMask::from_logits(side, &p.logits)))
        }
        Kind::Detail { prompt: Prompt::Clicks(clicks), region } => {
            let Some(inside) = clicks_in(&clicks, region) else { return Ok(None) };
            let input = render(job)?;
            let m = load(&mut state.sam2, shared, &job.dir, "SAM 2.1", Sam2::load)?;
            let t = web_time::Instant::now();
            let enc = m.encode(&input.rgb, input.w, input.h).map_err(|e| e.to_string())?;
            let p = m.segment_clicks(&enc, &inside).map_err(|e| e.to_string())?;
            log::info!("SAM 2.1 detail pass ({}×{}) in {:?}", input.w, input.h, t.elapsed());
            Ok(Some(SegMask::from_logits_in(side, &p.logits, region)))
        }
        Kind::Text(text) => {
            let detector = job.detector.clone().ok_or("Describe needs the OWLv2 object finder or the SAM 3 model")?;
            describe(state, shared, job, &detector, &text)
        }
        Kind::Detail { prompt: Prompt::Text(text), region } => {
            let detector = job.detector.clone().ok_or("Describe needs the OWLv2 object finder or the SAM 3 model")?;
            let (rw, rh) = (region[2] - region[0], region[3] - region[1]);
            if !(rw > 0.0 && rh > 0.0) {
                return Ok(None);
            }
            let input = render(job)?;
            let t = web_time::Instant::now();
            ensure(&mut state.owl, &mut state.owl_cache, shared, &detector, "OWLv2", Owl::load)?;
            ensure(&mut state.sam2, &mut state.sam2_cache, shared, &job.dir, "SAM 2.1", Sam2::load)?;
            let (Some((_, owl)), Some((_, sam))) = (&state.owl, &state.sam2) else { return Err("the Describe models did not load".into()) };
            let owl_enc = owl.encode(&input.rgb, input.w, input.h).map_err(|e| e.to_string())?;
            let sam_enc = sam.encode(&input.rgb, input.w, input.h).map_err(|e| e.to_string())?;
            let p = lightcraft_segment::describe::describe(owl, &owl_enc, sam, &sam_enc, &text, &Default::default()).map_err(|e| e.to_string())?;
            log::info!("Describe detail pass ({}×{}) in {:?}", input.w, input.h, t.elapsed());
            Ok(p.map(|p| SegMask::from_logits_in(side, &p.logits, region)))
        }
        Kind::Auto { .. } | Kind::Skin { .. } | Kind::Retouch { .. } | Kind::Faces { .. } | Kind::Learn { .. } => {
            Err("an automatic selection reached the SAM 2.1 path".into())
        }
        #[cfg(test)]
        #[allow(clippy::panic)]
        Kind::Panic => panic!("a test panic"),
        #[cfg(test)]
        Kind::FailOnGpu { .. } => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn look(v: f32) -> Likeness {
        Likeness { face: None, look: v }
    }

    fn both(face: f32, look: f32) -> Likeness {
        Likeness { face: Some(face), look }
    }

    /// Regression (test notes 2026-10-25, note 12): a mother and her daughter, alike in looks, both
    /// scored above the match bar and the higher one was taken (the wrong one). A pick must lead
    /// the next face clearly; otherwise the photo is "not sure" and nothing is guessed.
    #[test]
    fn a_look_alike_is_not_guessed() {
        // the old rule took face 1 (0.68 ≥ 0.55): it is barely ahead of face 2
        assert_eq!(choose(&[look(0.2), look(0.68), look(0.62)], 0.4), Pick::Unsure);
        // clearly ahead: taken
        assert_eq!(choose(&[look(0.2), look(0.81), look(0.4)], 0.4), Pick::Face(1));
        // a portrait whose look changed (closer framing): nobody at the bar, someone near it
        assert_eq!(choose(&[look(0.45), look(0.1)], 0.4), Pick::Unsure);
        // nobody like them at all, or nobody there
        assert_eq!(choose(&[look(0.1), look(0.2)], 0.4), Pick::Absent);
        assert_eq!(choose(&[], 0.4), Pick::Absent);
        // one face, like them: taken (nothing to confuse it with)
        assert_eq!(choose(&[look(0.7)], 0.4), Pick::Face(0));
        // hostile numbers are no one
        assert_eq!(choose(&[look(f32::NAN), look(f32::INFINITY)], 0.4), Pick::Absent);
    }

    /// The face decides before the look: the daughter in the same pyjamas looks more like the
    /// examples, but the recognition model knows the mother's face.
    #[test]
    fn the_face_decides_and_the_look_only_breaks_ties() {
        assert_eq!(choose(&[both(0.71, 0.6), both(0.22, 0.75)], 0.4), Pick::Face(0));
        // alike by face: the look breaks the tie when it clearly can, else no guess
        assert_eq!(choose(&[both(0.55, 0.85), both(0.52, 0.4)], 0.4), Pick::Face(0));
        assert_eq!(choose(&[both(0.55, 0.6), both(0.52, 0.58)], 0.4), Pick::Unsure);
        // nobody recognised: near the bar is "not sure", far below it "not there"…
        assert_eq!(choose(&[both(0.33, 0.2), both(0.1, 0.2)], 0.4), Pick::Unsure);
        assert_eq!(choose(&[both(0.1, 0.9), both(0.05, 0.2)], 0.4), Pick::Absent);
        // …unless a face the model couldn't read (a profile) looks like them
        assert_eq!(choose(&[both(0.1, 0.2), look(0.7)], 0.4), Pick::Unsure);
    }

    fn job(tag: Tag, reply: &mpsc::Sender<Outcome>) -> Job {
        // an empty folder: loading fails with an error (no model files)
        let dir = std::env::temp_dir().join(format!("lc-sam3-worker-none-{}", std::process::id()));
        Job {
            dir,
            sam2: false,
            detector: None,
            key: 1,
            render: None,
            fine: None,
            missing: None,
            kind: Kind::Prepare,
            tag,
            reply: reply.clone(),
            activity: None,
            batch: None,
            parent: None,
            analyses: None,
        }
    }

    fn face_at(x: f32, y: f32, size: f32) -> Face {
        let d = lightcraft_segment::mediapipe::Detection {
            x0: x - size / 2.0,
            y0: y - size / 2.0,
            x1: x + size / 2.0,
            y1: y + size / 2.0,
            score: 0.9,
            keypoints: [
                [x - size / 5.0, y - size / 8.0],
                [x + size / 5.0, y - size / 8.0],
                [x, y],
                [x, y + size / 4.0],
                [x - size / 2.0, y],
                [x + size / 2.0, y],
            ],
        };
        Face { detection: d, landmarks: None }
    }

    /// Test notes 2026-10-25, note 10: what a People selection computed of a photo's people is
    /// kept — in memory, and on disk under the photo, its look and the models — and put back for
    /// the next selection on it, after other photos and after a restart; another model set or a
    /// damaged record is analysed again.
    #[test]
    fn a_photos_people_are_analysed_once_and_kept_on_disk() {
        let d = std::env::temp_dir().join(format!("lc-people-analysis-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let cache = Arc::new(lightcraft_preview::PreviewCache::with_disk(1 << 20, &d, 1 << 30));
        let (tx, _rx) = mpsc::channel();
        let mut j = job(Tag::Wait, &tx);
        j.key = 42;
        j.analyses = Some(cache.clone());
        let shared = Shared::default();
        let mut st = State::default();
        assert!(recall_people(&mut st, &shared, &mut j, None, None, 7).is_none(), "nothing yet");
        st.faces_cache = Some((42, vec![face_at(100.0, 80.0, 40.0), face_at(300.0, 90.0, 36.0)]));
        st.people_cache = Some((42, PeopleMap { side: 8, probs: vec![0.5; 8 * 8 * 6] }));
        st.figures_cache = Some((42, vec![vec![3.0; 64]]));
        st.silhouettes_cache = Some((42, vec![vec![-2.0; 64], vec![2.0; 64]]));
        st.surfaces_cache = Some((42, Surfaces::from_grid(2, 2, vec![0.0, 1.0, 0.0, 1.0])));
        st.signatures_cache = Some((42, false, vec![None, Some(Signature { hair: Some([0.1, 0.2, 0.3]), ..Default::default() })]));
        st.crowd_crops = Some((42, vec![[0.1, 0.1, 0.4, 0.9]], vec![PeopleMap { side: 4, probs: vec![0.25; 4 * 4 * 6] }]));
        keep_people(&mut st, &j, None, 7);
        // another photo's selections in between: this one's is still in memory
        assert!(st.analyses.get(42).is_some());
        // a restart: nothing in memory, all of it read back from the disk
        let mut fresh = State::default();
        assert!(recall_people(&mut fresh, &shared, &mut j, None, None, 7).is_some());
        assert_eq!(fresh.faces_cache.as_ref().map(|(k, f)| (*k, f.len())), Some((42, 2)));
        assert_eq!(fresh.people_cache.as_ref().map(|(k, m)| (*k, m.side, m.probs.len())), Some((42, 8, 384)));
        assert_eq!(fresh.figures_cache.as_ref().map(|(k, f)| (*k, f.len(), f[0][0])), Some((42, 1, 3.0)));
        assert_eq!(fresh.silhouettes_cache.as_ref().map(|(_, s)| (s[0][0], s[1][63])), Some((-2.0, 2.0)));
        assert_eq!(fresh.surfaces_cache.as_ref().map(|(_, s)| s.grid().2.to_vec()), Some(vec![0.0, 1.0, 0.0, 1.0]));
        assert_eq!(fresh.signatures_cache.as_ref().map(|(_, f, s)| (*f, s.len(), s[1].is_some())), Some((false, 2, true)));
        assert_eq!(fresh.crowd_crops.as_ref().map(|(_, b, c)| (b.len(), c[0].side)), Some((1, 4)));
        // the group's crops are pasted again without the model
        crowd_map(&mut fresh, &shared, &mut j, &[]);
        assert!(fresh.crowd_cache.as_ref().is_some_and(|(k, m)| *k == 42 && m.as_ref().is_some_and(|m| m.side == CROWD_SIDE)));
        // other models: analysed again
        let mut other = State::default();
        assert!(recall_people(&mut other, &shared, &mut j, None, None, 8).is_none());
        // another look of the photo (another input key): analysed again
        j.key = 43;
        assert!(recall_people(&mut other, &shared, &mut j, None, None, 7).is_none());
        // a damaged record is dropped, never a panic
        cache.put_data(people_cache::key(43, 7), b"LCPA\x01\x00\x00\x00junk");
        assert!(recall_people(&mut other, &shared, &mut j, None, None, 7).is_none());
        assert!(cache.get_data(people_cache::key(43, 7)).is_none(), "dropped");
        // a selection on an uncacheable input (a zoomed detail pass) keeps nothing
        j.key = 0;
        keep_people(&mut st, &j, None, 7);
        assert!(st.analyses.get(0).is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Two faces side by side, one warm, one cool (a recognition model tells them apart).
    fn two_faces() -> (Input, Vec<Face>) {
        let (w, h) = (400, 200);
        let rgb = (0..w * h).flat_map(|i| if i % w < w / 2 { [205, 120, 95] } else { [70, 90, 190] }).collect();
        (Input { rgb, w, h }, vec![face_at(100.0, 100.0, 80.0), face_at(300.0, 100.0, 80.0)])
    }

    /// Test notes 2026-10-25, note 12: the mother and her daughter look alike (same hair, same
    /// colours: the look picks the wrong one, or can't tell); with a recognition model in use the
    /// bride's face decides, and a face that is neither hers nor clearly not is not guessed.
    #[test]
    fn the_face_tells_the_bride_from_her_look_alike() {
        let embedder = crate::tests_people_point::tiny_embedder(16);
        let (input, faces) = two_faces();
        let e = face_embeddings(&faces, &input, &embedder);
        assert!(e.iter().all(Option::is_some), "both faces embedded");
        let (mother, bride) = (e[0].clone().unwrap(), e[1].clone().unwrap());
        let cos = lightcraft_faces::matching::cosine(&mother, &bride).unwrap();
        assert!(cos < 0.9, "the model tells them apart: {cos}");
        // by look the examples are the mother's double
        let look = Signature { hair: Some([0.4, 0.1, 0.05]), clothes: Some([0.9, 0.0, 0.0]), ..Default::default() };
        let sigs = vec![Some(look.clone()), Some(Signature { hair: Some([0.4, 0.1, 0.05]), clothes: Some([0.85, 0.0, 0.02]), ..Default::default() })];
        let known = [look];
        assert_ne!(pick_face(&faces, None, &known, &sigs, None, &input), Pick::Face(1), "by look alone the bride isn't found");
        let hers: Vec<Arc<[f32]>> = vec![bride.clone().into()];
        assert_eq!(pick_face(&faces, None, &known, &sigs, Some((&e, &hers, 0.45)), &input), Pick::Face(1), "her face decides");
        // her face only (no appearance examples yet): enough
        assert_eq!(pick_face(&faces, None, &[], &[], Some((&e, &hers, 0.45)), &input), Pick::Face(1));
        // an example halfway between the two faces (alike by face, alike by look): not guessed
        let unit = |v: &[f32]| {
            let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            v.iter().map(|x| x / n).collect::<Vec<f32>>()
        };
        let between: Vec<f32> = unit(&mother).iter().zip(unit(&bride)).map(|(a, b)| a + b).collect();
        let bar = lightcraft_faces::matching::cosine(&between, &mother).unwrap() - 0.01;
        let neither: Vec<Arc<[f32]>> = vec![between.into()];
        assert_eq!(pick_face(&faces, None, &[], &[], Some((&e, &neither, bar)), &input), Pick::Unsure);
        // …nor when it comes close to someone without reaching the model's bar
        assert_eq!(pick_face(&faces, None, &[], &[], Some((&e, &neither, bar + 0.1)), &input), Pick::Unsure);
        // nobody like her face here
        let other: Vec<Arc<[f32]>> = vec![mother.iter().map(|v| -v).collect::<Vec<f32>>().into()];
        assert_eq!(pick_face(&faces, None, &[], &[], Some((&e, &other, 0.45)), &input), Pick::Absent);
        // a face that can't be cut out (a damaged box) has no embedding, never a panic
        let off = [face_at(f32::NAN, 50.0, 10.0)];
        assert_eq!(face_embeddings(&off, &input, &embedder), vec![None]);
        // landmarks out of range fall back to the box
        let mut odd = faces[1].clone();
        odd.landmarks = Some(vec![[f32::NAN, 3.0]; 10]);
        assert!(face_embeddings(&[odd], &input, &embedder)[0].is_some());
    }

    /// A synthetic group (three figures on a floor, a wall behind): skin-toned heads, dark and
    /// pale clothes. The models' timings don't depend on what they find.
    fn group_photo(w: usize, h: usize) -> (Input, Vec<Face>) {
        let heads = [(0.25, 0.30), (0.50, 0.28), (0.75, 0.31)];
        let mut rgb = vec![0u8; w * h * 3];
        for y in 0..h {
            for x in 0..w {
                let (u, v) = (x as f32 / w as f32, y as f32 / h as f32);
                let mut c = if v > 0.8 { [150, 140, 125] } else { [200, 205, 210] };
                for (k, (hx, hy)) in heads.iter().enumerate() {
                    let (dx, dy) = ((u - hx) * w as f32 / h as f32, v - hy);
                    if dx.hypot(dy) < 0.06 {
                        c = [224, 172, 140];
                    } else if dx.abs() < 0.09 && dy > 0.06 && dy < 0.62 {
                        c = if k == 1 { [245, 245, 240] } else { [30, 32, 40] };
                    }
                }
                rgb[(y * w + x) * 3..(y * w + x) * 3 + 3].copy_from_slice(&c);
            }
        }
        let faces = heads.iter().map(|(hx, hy)| face_at(hx * w as f32, hy * h as f32, 0.12 * h as f32)).collect();
        (Input { rgb, w, h }, faces)
    }

    /// Note 10's measurement, with the real models (`LIGHTCRAFT_AI_MODELS_TEST_DIR`: a models folder
    /// holding Selfie Multiclass, the MediaPipe face model, EoMT-S and SAM 2.1; skipped without):
    /// "everyone's clothes minus one person's" on a batch — Everyone ▸ Clothes on photo A, then
    /// on photo B (the worker moves on), then the subtraction on A, on B, on A again (another
    /// person, or the recipe run again), Everyone on A again — timed the old way (only the last photo's model outputs kept) and
    /// with the photo's analysis kept, and after a restart (read from the disk).
    #[test]
    fn subtracting_a_person_again_reuses_the_photos_analysis() {
        let Some(root) = std::env::var_os("LIGHTCRAFT_AI_MODELS_TEST_DIR").map(PathBuf::from) else {
            lightcraft_testkit::skip!(Model, "LIGHTCRAFT_AI_MODELS_TEST_DIR is not set: skipping the timing");
            return;
        };
        let [people, face, scene, sam2] =
            ["mediapipe-selfie-multiclass", "mediapipe-face-landmarker", "eomt-s-coco", "sam2.1-hiera-tiny"].map(|n| root.join(n));
        if [&people, &face, &scene, &sam2].iter().any(|d| !d.is_dir()) {
            lightcraft_testkit::skip!(Model, "the people, face, EoMT or SAM 2.1 model is not in LIGHTCRAFT_AI_MODELS_TEST_DIR");
            return;
        }
        let model =
            || AutoModel::People { people: Some(people.clone()), face: Some(face.clone()), scene: Some(scene.clone()), sam2: Some(sam2.clone()) };
        let (photo, faces) = group_photo(1024, 683);
        let everyone = AutoKind::People { person: 0, parts: vec!["Clothes".into()], who: None };
        let her = AutoKind::People { person: 0, parts: vec!["Clothes".into()], who: Some(lightcraft_develop::recipe::Who::Role("bride".into())) };
        let rect = norm_box(&faces[1], photo.w, photo.h);
        let target = Target::Person { rect: Some(rect), known: Vec::new(), sam2: Some(sam2.clone()), ident: None };
        let d = std::env::temp_dir().join(format!("lc-people-timing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let cache = Arc::new(lightcraft_preview::PreviewCache::with_disk(1 << 20, &d, 1 << 30));
        let shared = Shared::default();
        let (tx, _rx) = mpsc::channel();
        // one job on photo `key` (its input rendered already: the render costs the same either way)
        let run = |st: &mut State, key: u64, what: &AutoKind, target: &Target, keep: bool| {
            let mut j = job(Tag::Wait, &tx);
            j.key = key;
            j.analyses = keep.then(|| cache.clone());
            if !keep {
                // the old worker: only the last photo's model outputs, and no silhouettes kept
                st.analyses = people_cache::Memory::default();
                (st.crowd_crops, st.silhouettes_cache, st.figures_cache, st.surfaces_cache, st.signatures_cache) = (None, None, None, None, None);
            }
            if st.input_cache.as_ref().is_none_or(|(k, _)| *k != key) {
                st.input_cache = Some((key, Input { rgb: photo.rgb.clone(), w: photo.w, h: photo.h }));
            }
            // the faces as the face model would find them on a real group (it finds none on this drawing)
            ensure_face(&mut st.face, &mut st.faces_cache, &shared, &face).unwrap();
            if st.faces_cache.as_ref().is_none_or(|(k, _)| *k != key) {
                st.faces_cache = Some((key, faces.clone()));
            }
            let t = web_time::Instant::now();
            let out = auto(st, &shared, &mut j, what, model(), target);
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            assert!(out.is_ok(), "{:?}", out.err());
            ms
        };
        let mut report = Vec::new();
        for keep in [false, true] {
            let mut st = State::default();
            // the models loaded first (on another photo): what follows is the photos' own cost
            let _ = run(&mut st, 99, &everyone, &Target::None, false);
            let _ = run(&mut st, 99, &her, &target, false);
            let e_a = run(&mut st, 1, &everyone, &Target::None, keep);
            let e_b = run(&mut st, 2, &everyone, &Target::None, keep);
            let s1 = run(&mut st, 1, &her, &target, keep);
            let _ = run(&mut st, 2, &her, &target, keep);
            let s2 = run(&mut st, 1, &her, &target, keep);
            let e_a2 = run(&mut st, 1, &everyone, &Target::None, keep);
            report.push(format!(
                "{}: Everyone A {e_a:.0} ms, Everyone B {e_b:.0} ms, first subtract on A {s1:.0} ms, (subtract on B), second subtract on A {s2:.0} ms, Everyone A again {e_a2:.0} ms",
                if keep { "kept" } else { "old" }
            ));
            if keep {
                // a restart: the analysis comes from the disk
                let mut st = State::default();
                let _ = run(&mut st, 99, &everyone, &Target::None, false);
                let s3 = run(&mut st, 1, &her, &target, true);
                report.push(format!("kept, after a restart (models loaded): subtract on A {s3:.0} ms"));
            }
        }
        eprintln!("people analysis timings (1024×683, 3 people):\n  {}", report.join("\n  "));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn errors_come_back_and_the_counters_return_to_zero() {
        let mut w = Worker::default();
        let (tx, rx) = mpsc::channel();
        for _ in 0..3 {
            w.submit(job(Tag::Wait, &tx)).unwrap();
        }
        for _ in 0..3 {
            let o = rx.recv_timeout(Duration::from_secs(30)).unwrap();
            assert!(o.result.unwrap_err().contains("not found"));
        }
        let t = web_time::Instant::now();
        while w.pending() > 0 && t.elapsed() < Duration::from_secs(5) {
            std::thread::yield_now();
        }
        assert_eq!(w.pending(), 0);
        assert!(!w.encoding() && !w.loaded());
    }

    /// Describe without SAM 3 runs on OWLv2 and SAM 2.1: models that aren't there are errors
    /// naming them, and a job that lost its detector says what Describe needs.
    #[test]
    fn describe_on_the_open_models_reports_missing_ones() {
        let shared = Shared::default();
        let (tx, rx) = mpsc::channel();
        let (jtx, jrx) = mpsc::channel();
        let mut open = job(Tag::Wait, &tx);
        (open.sam2, open.detector, open.kind) = (true, Some(open.dir.join("owl")), Kind::Text("car, road".into()));
        jtx.send(open).unwrap();
        let mut lost = job(Tag::Wait, &tx);
        (lost.sam2, lost.kind) = (true, Kind::Text("car".into()));
        jtx.send(lost).unwrap();
        let mut detail = job(Tag::Wait, &tx);
        (detail.sam2, detail.detector) = (true, Some(detail.dir.join("owl")));
        detail.kind = Kind::Detail { prompt: Prompt::Text("car".into()), region: [0.2, 0.2, 0.2, 0.6] };
        jtx.send(detail).unwrap();
        shared.pending.store(2, Ordering::SeqCst);
        shared.detail.store(1, Ordering::SeqCst);
        drop(jtx);
        run(&jrx, &shared, Duration::from_secs(60), Placement::Cpu);
        let e = rx.recv().unwrap().result.unwrap_err();
        assert!(e.contains("SAM 2"), "{e}");
        let e = rx.recv().unwrap().result.unwrap_err();
        assert!(e.contains("OWLv2") && e.contains("SAM 3"), "{e}");
        // an empty detail region selects nothing (no model needed)
        assert!(rx.recv().unwrap().result.unwrap().is_none());
    }

    #[test]
    fn a_panic_in_a_job_is_an_error_and_the_worker_carries_on() {
        let shared = Shared::default();
        let (tx, rx) = mpsc::channel();
        let (jtx, jrx) = mpsc::channel();
        let mut j = job(Tag::Wait, &tx);
        j.kind = Kind::Panic;
        jtx.send(j).unwrap();
        jtx.send(job(Tag::Wait, &tx)).unwrap();
        shared.pending.store(2, Ordering::SeqCst);
        drop(jtx);
        run(&jrx, &shared, Duration::from_secs(60), Placement::Cpu);
        let first = rx.recv().unwrap();
        assert!(first.result.unwrap_err().contains("failed unexpectedly"));
        let second = rx.recv().unwrap();
        assert!(second.result.is_err(), "the next job still runs");
        assert_eq!(shared.pending.load(Ordering::SeqCst), 0, "counters reset");
        assert!(!shared.encoding.load(Ordering::SeqCst));
    }

    /// Run `kinds` in turn on a worker that starts on `placement`; their outcomes.
    fn run_kinds(placement: Placement, kinds: Vec<Kind>) -> Vec<Outcome> {
        let (tx, rx) = mpsc::channel();
        let (jtx, jrx) = mpsc::channel();
        let shared = Shared::default();
        shared.pending.store(kinds.len(), Ordering::SeqCst);
        for kind in kinds {
            let mut j = job(Tag::Wait, &tx);
            j.kind = kind;
            jtx.send(j).unwrap();
        }
        drop(jtx);
        run(&jrx, &shared, Duration::from_secs(60), placement);
        assert_eq!(shared.pending.load(Ordering::SeqCst), 0, "counters reset");
        assert!(!shared.encoding.load(Ordering::SeqCst));
        rx.try_iter().collect()
    }

    #[test]
    fn a_job_that_panics_on_the_gpu_runs_on_the_cpu_and_so_do_the_next_ones() {
        // issue #1974: candle panics compiling a Metal kernel the GPU's compiler can't build
        let out = run_kinds(Placement::Gpu, vec![Kind::FailOnGpu { panic: true }, Kind::FailOnGpu { panic: true }]);
        assert_eq!(out.len(), 2);
        assert!(matches!(out[0].result, Ok(None)), "retried on the CPU: {:?}", out[0].result.as_ref().err());
        assert_eq!(out[0].notice.as_deref(), Some(GPU_FAILED));
        assert!(matches!(out[1].result, Ok(None)), "the next job starts on the CPU");
        assert_eq!(out[1].notice, None, "said once");
    }

    #[test]
    fn a_model_error_on_the_gpu_is_retried_on_the_cpu_too() {
        let out = run_kinds(Placement::Gpu, vec![Kind::FailOnGpu { panic: false }]);
        assert!(matches!(out[0].result, Ok(None)));
        assert_eq!(out[0].notice.as_deref(), Some(GPU_FAILED));
    }

    #[test]
    fn a_job_that_fails_on_the_cpu_too_keeps_the_gpu() {
        let out = run_kinds(Placement::Gpu, vec![Kind::Panic, Kind::FailOnGpu { panic: true }]);
        assert!(out[0].result.as_ref().unwrap_err().contains("failed unexpectedly"));
        assert_eq!(out[0].notice, None);
        // not the GPU's fault: the next job tries the GPU first again
        assert!(matches!(out[1].result, Ok(None)));
        assert_eq!(out[1].notice.as_deref(), Some(GPU_FAILED));
    }

    #[test]
    fn a_worker_on_the_cpu_never_tries_the_gpu() {
        // no GPU in this build, or LIGHTCRAFT_GPU=0 / LIGHTCRAFT_GPU_BACKEND=off
        let out = run_kinds(Placement::Cpu, vec![Kind::FailOnGpu { panic: true }]);
        assert!(matches!(out[0].result, Ok(None)));
        assert_eq!(out[0].notice, None);
        if !lightcraft_segment::GPU_BUILD {
            assert_eq!(Placement::initial(), Placement::Cpu);
        }
    }

    #[test]
    fn missing_model_files_are_not_retried_on_the_cpu() {
        let out = run_kinds(Placement::Gpu, vec![Kind::Prepare]);
        assert!(out[0].result.as_ref().unwrap_err().contains("not found"));
        assert_eq!(out[0].notice, None);
        let missing = lightcraft_segment::Error::Missing(PathBuf::from("models"));
        assert!(matches!(Failure::from(missing), Failure::Other(_)));
    }

    #[test]
    fn a_newer_request_for_the_same_selection_supersedes_a_queued_one() {
        let (tx, rx) = mpsc::channel();
        let (jtx, jrx) = mpsc::channel();
        let clicks = |n: usize| Tag::Clicks {
            photo: lightcraft_catalog::PhotoId(1),
            mask: 1,
            comp: 0,
            hint: vec![lightcraft_geom::Point::new(0.5, 0.5); n],
            exclude: vec![],
        };
        for n in 1..=3 {
            jtx.send(job(clicks(n), &tx)).unwrap();
        }
        drop(jtx);
        let shared = Shared::default();
        shared.pending.store(3, Ordering::SeqCst);
        run(&jrx, &shared, Duration::from_secs(60), Placement::Cpu);
        let out: Vec<Outcome> = rx.try_iter().collect();
        assert_eq!(out.len(), 3);
        assert!(out[0].superseded && out[1].superseded && !out[2].superseded);
        assert_eq!(shared.pending.load(Ordering::SeqCst), 0);
    }
}
