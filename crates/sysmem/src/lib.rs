//! Give memory the system allocator keeps after frees back to the operating system.
//!
//! macOS's allocator keeps freed large blocks mapped as "reusable": they no longer count in the
//! process's memory footprint but stay in its resident size until the system runs short. After a
//! raw decode (hundreds of MB of temporaries) that made the resident size climb towards the sum of
//! everything ever decoded. `malloc_zone_pressure_relief` (libSystem) returns those pages now.
//! Elsewhere (glibc, Windows) blocks this large are unmapped when freed: nothing to do.
//!
//! It also lowers a worker thread's priority ([`lower_thread_priority`], Windows'
//! `SetThreadPriority`), so renders leave the desktop responsive.
//!
//! And it reads the computer's memory where only a system call tells it ([`total_physical_memory`],
//! Windows' `GlobalMemoryStatusEx`), and the free space of a drive ([`free_disk_space`], Windows'
//! `GetDiskFreeSpaceExW`; elsewhere POSIX `df`, no system call of our own).
//!
//! This is LightCraft's only crate allowed `unsafe` (FFI calls: `malloc_zone_pressure_relief`
//! in `macos`, `SetThreadPriority`, `GlobalMemoryStatusEx` and `GetDiskFreeSpaceExW` in `windows`). Its API is safe, and on failure or on other
//! platforms it simply does nothing.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

/// Return free allocator pages to the system. Returns the number of bytes released (always 0 on
/// platforms where freed large blocks are already unmapped). Safe to call at any time, from any
/// thread.
pub fn release_free_memory() -> usize {
    #[cfg(target_os = "macos")]
    {
        macos::release()
    }
    #[cfg(not(target_os = "macos"))]
    {
        0
    }
}

/// Run the calling thread below normal priority, so the work it does (background renders, the
/// render workers' fan-out) never takes the processor from the desktop and other apps: the mouse
/// and the window stay smooth while every core renders (TEST-NOTES 2026-10-16 item 15). Returns
/// whether the priority changed; on systems where this isn't done (anything but Windows) it is
/// `false` and nothing happens. Safe to call from any thread, any number of times.
pub fn lower_thread_priority() -> bool {
    #[cfg(windows)]
    {
        windows::lower()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// The computer's physical memory (bytes), where the platform needs a system call to tell it
/// (Windows' `GlobalMemoryStatusEx`); `None` elsewhere (the engine reads Linux's and macOS's itself)
/// and on failure. Safe to call from any thread.
pub fn total_physical_memory() -> Option<u64> {
    #[cfg(windows)]
    {
        windows::total_physical()
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Bytes free for this user on the drive that holds `path` (the folder itself, or the nearest
/// folder above it that exists), `None` when it can't be told. Windows asks `GetDiskFreeSpaceExW`;
/// other systems run POSIX `df -Pk`. Blocking (a few ms; `df` a process start): not for a UI frame.
pub fn free_disk_space(path: &std::path::Path) -> Option<u64> {
    let mut at = Some(path);
    while let Some(p) = at {
        if p.is_dir() {
            break;
        }
        at = p.parent();
    }
    let dir = at.filter(|p| !p.as_os_str().is_empty())?;
    #[cfg(windows)]
    {
        windows::free_space(dir)
    }
    #[cfg(not(windows))]
    {
        df_free(dir)
    }
}

/// `df -Pk dir`: the fourth column of its second line is the space available, in KiB.
#[cfg(not(windows))]
fn df_free(dir: &std::path::Path) -> Option<u64> {
    let out = std::process::Command::new("df").arg("-Pk").arg(dir).output().ok()?;
    if !out.status.success() {
        return None;
    }
    parse_df(&String::from_utf8_lossy(&out.stdout))
}

/// The available KiB of `df -P`'s output, in bytes.
#[cfg_attr(windows, allow(dead_code))]
fn parse_df(text: &str) -> Option<u64> {
    let line = text.lines().nth(1)?;
    // (a mount point may hold spaces; the numbers come before it)
    let kib: u64 = line.split_whitespace().nth(3)?.parse().ok()?;
    kib.checked_mul(1024)
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod windows {
    use std::ffi::c_void;

    /// `THREAD_PRIORITY_BELOW_NORMAL` (processthreadsapi.h).
    const BELOW_NORMAL: i32 = -1;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        /// The calling thread's pseudo handle (needs no closing).
        fn GetCurrentThread() -> *mut c_void;
        /// Nonzero on success.
        fn SetThreadPriority(thread: *mut c_void, priority: i32) -> i32;
    }

    pub fn lower() -> bool {
        // SAFETY: both are documented, thread-safe kernel32 functions. `GetCurrentThread` returns
        // a constant pseudo handle for the calling thread (never null, never to be closed), which
        // is exactly what `SetThreadPriority` takes; the priority is a documented constant. Neither
        // reads or writes memory we own, and a failure only leaves the priority as it was.
        unsafe { SetThreadPriority(GetCurrentThread(), BELOW_NORMAL) != 0 }
    }

    /// `MEMORYSTATUSEX` (sysinfoapi.h).
    #[repr(C)]
    struct MemoryStatusEx {
        length: u32,
        memory_load: u32,
        total_phys: u64,
        avail_phys: u64,
        total_page_file: u64,
        avail_page_file: u64,
        total_virtual: u64,
        avail_virtual: u64,
        avail_extended_virtual: u64,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        /// Nonzero on success; `buffer.length` must be the struct's size.
        fn GlobalMemoryStatusEx(buffer: *mut MemoryStatusEx) -> i32;
    }

    pub fn total_physical() -> Option<u64> {
        let mut s = MemoryStatusEx {
            length: std::mem::size_of::<MemoryStatusEx>() as u32,
            memory_load: 0,
            total_phys: 0,
            avail_phys: 0,
            total_page_file: 0,
            avail_page_file: 0,
            total_virtual: 0,
            avail_virtual: 0,
            avail_extended_virtual: 0,
        };
        // SAFETY: `GlobalMemoryStatusEx` is a documented, thread-safe kernel32 function that fills
        // the `MEMORYSTATUSEX` it is given, whose `dwLength` is set to the struct's size as
        // required. `s` is a live, properly aligned `repr(C)` struct with the documented layout
        // (two DWORDs then seven DWORDLONGs), owned by this frame for the whole call; nothing is
        // kept after it returns.
        let ok = unsafe { GlobalMemoryStatusEx(&mut s) } != 0;
        (ok && s.total_phys > 0).then_some(s.total_phys)
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        /// Nonzero on success. `dir` is a NUL-terminated UTF-16 folder name; each out pointer may be null.
        fn GetDiskFreeSpaceExW(dir: *const u16, free_to_caller: *mut u64, total: *mut u64, total_free: *mut u64) -> i32;
    }

    pub fn free_space(dir: &std::path::Path) -> Option<u64> {
        use std::os::windows::ffi::OsStrExt;
        if dir.as_os_str().encode_wide().any(|c| c == 0) {
            return None;
        }
        let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let mut free = 0u64;
        // SAFETY: `GetDiskFreeSpaceExW` is a documented, thread-safe kernel32 function. `wide` is a
        // NUL-terminated UTF-16 string (no NUL inside, checked above) that lives for the whole call;
        // `free` is a live, aligned u64 owned by this frame that it writes; the other two out
        // pointers are null, which the function allows. Nothing is kept after it returns.
        let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, std::ptr::null_mut(), std::ptr::null_mut()) } != 0;
        ok.then_some(free)
    }
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod macos {
    unsafe extern "C" {
        /// libSystem malloc: release free memory of `zone` (all zones when null) back to the
        /// system, up to `goal` bytes (0 = as much as possible). Returns the bytes released.
        fn malloc_zone_pressure_relief(zone: *mut std::ffi::c_void, goal: usize) -> usize;
    }

    pub fn release() -> usize {
        // SAFETY: `malloc_zone_pressure_relief` is a documented, thread-safe libSystem function
        // (malloc/malloc.h). A null zone means "every zone" and a goal of 0 means "as much as
        // possible". It only returns already-free pages to the system: it reads and writes no
        // memory we own, takes no pointers we must keep alive, and has no failure mode beyond
        // releasing 0 bytes.
        unsafe { malloc_zone_pressure_relief(std::ptr::null_mut(), 0) }
    }
}

#[cfg(test)]
mod tests {
    use super::release_free_memory;

    #[test]
    fn releasing_with_nothing_freed_is_harmless() {
        // Twice in a row: the second call usually has nothing left to release.
        let _ = release_free_memory();
        let _ = release_free_memory();
    }

    #[test]
    fn freed_large_blocks_can_be_released_and_memory_stays_usable() {
        // Allocate and free well over the allocator's large-block threshold, then release.
        let blocks: Vec<Vec<u8>> = (0..8).map(|i| vec![i as u8; 16 << 20]).collect();
        let sum: u64 = blocks.iter().map(|b| u64::from(b[b.len() - 1])).sum();
        assert_eq!(sum, (0..8u64).sum());
        drop(blocks);
        let released = release_free_memory();
        if cfg!(not(target_os = "macos")) {
            assert_eq!(released, 0);
        }
        // The allocator must still work normally afterwards.
        let again = vec![7u8; 16 << 20];
        assert_eq!(again[again.len() - 1], 7);
    }

    #[test]
    fn the_physical_memory_is_known_on_windows() {
        let ram = super::total_physical_memory();
        if cfg!(windows) {
            let ram = ram.expect("GlobalMemoryStatusEx");
            assert!((512 << 20..1 << 50).contains(&ram), "{ram} bytes");
        } else {
            assert_eq!(ram, None);
        }
    }

    #[test]
    fn a_thread_can_be_lowered_and_keeps_working() {
        let lowered = std::thread::spawn(|| {
            let changed = super::lower_thread_priority();
            // twice is harmless
            let again = super::lower_thread_priority();
            let sum: u64 = (0..1000u64).sum();
            (changed, again, sum)
        })
        .join()
        .unwrap();
        assert_eq!(lowered.2, 499_500);
        assert_eq!(lowered.0, cfg!(windows));
        assert_eq!(lowered.1, cfg!(windows));
    }

    #[test]
    fn the_free_space_of_the_drive_of_a_folder_is_known() {
        let dir = std::env::temp_dir();
        let free = super::free_disk_space(&dir);
        assert!(free.is_some_and(|b| b > 0), "{free:?}");
        // a folder that isn't there: the drive of the nearest one above it
        assert_eq!(super::free_disk_space(&dir.join("no-such-folder/below")).is_some(), free.is_some());
        assert_eq!(super::free_disk_space(std::path::Path::new("")), None);
    }

    #[test]
    fn df_output_is_read_in_bytes() {
        let text = "Filesystem 1024-blocks Used Available Capacity Mounted on
/dev/disk3s5 971350180 512 2048 50% /Volumes/My Drive
";
        assert_eq!(super::parse_df(text), Some(2048 * 1024));
        assert_eq!(
            super::parse_df(
                "Filesystem
"
            ),
            None
        );
        assert_eq!(
            super::parse_df(
                "h
/dev/x 1 2 lots 3% /
"
            ),
            None
        );
    }

    #[test]
    fn concurrent_calls_are_safe() {
        let handles: Vec<_> = (0..4).map(|_| std::thread::spawn(release_free_memory)).collect();
        for h in handles {
            assert!(h.join().is_ok());
        }
    }
}
