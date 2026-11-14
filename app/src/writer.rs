//! The page's side of the write worker (M5.1 of
//! `docs/plans/active/gpu-speed.md`).  The page writes its song-sized
//! files through the write worker, which writes them with sync access
//! handles: Safari keeps a swap file of each write stream that closes
//! (TD-103), and the main thread has no sync access handle.  The small
//! files stay on write streams (`store`).
//!
//! Trunk names a worker's files after its `data-bin` in `index.html`, with
//! no hash (`filehash = false` in `Trunk.toml`), so the names are fixed.
//! "Reset…" fetches these files again (`reset::WORKER_FILES`).

/// The loader shim that the page starts as the write worker.
pub const LOADER: &str = "./write-worker_loader.js";

/// Trunk's names for the write worker's files: the loader shim, the
/// wasm-bindgen glue that it imports and the wasm that the glue fetches.
pub const FILES: [&str; 3] = [LOADER, "./write-worker.js", "./write-worker_bg.wasm"];

#[cfg(target_arch = "wasm32")]
mod client;
#[cfg(target_arch = "wasm32")]
pub use client::*;
#[cfg(all(target_arch = "wasm32", any(test, feature = "fake-writer")))]
mod fake;
#[cfg(all(test, target_arch = "wasm32"))]
mod routing_tests;
