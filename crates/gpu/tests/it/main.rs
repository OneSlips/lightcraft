//! The integration tests, one module per file, linked into one test binary.
//! Binaries of their own: `tests/shutdown.rs` and `tests/shutdown_first.rs` close the GPU for the whole
//! process, and `tests/memory.rs` checks the process's device-memory counters, which renders on other test
//! threads would change; `tests/display.rs` adopts a window device (once per process) and `tests/recovery.rs`
//! replaces the device the others render on.

mod equivalence;
mod fallback;
#[cfg(not(target_arch = "wasm32"))]
mod gpu_probe;
mod hdr_gpu;
mod mask_geometry;
mod skin_gpu;
mod thumbs_bench;
