//! The page's side of the decode worker (decision 0019).  The page starts
//! the decode worker from [`LOADER`] and posts each decode to it; each
//! reply ends the decode that waits for it (`DecodeClient`, web only;
//! `docs/plans/active/stream-from-storage-m5.md`, D6 and D9).
//!
//! Trunk names a worker's files after its `data-bin` in `index.html`, with
//! no hash (`filehash = false` in `Trunk.toml`), so the names are fixed.
//! "Reset…" fetches these files again (`reset::WORKER_FILES`).

/// The loader shim that the page starts as the decode worker.
pub const LOADER: &str = "./decode-worker_loader.js";

/// Trunk's names for the decode worker's files: the loader shim, the
/// wasm-bindgen glue that it imports and the wasm that the glue fetches.
pub const FILES: [&str; 3] = [LOADER, "./decode-worker.js", "./decode-worker_bg.wasm"];

#[cfg(target_arch = "wasm32")]
mod client;
#[cfg(target_arch = "wasm32")]
pub use client::{DecodeClient, Ended};
