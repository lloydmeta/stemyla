//! Stem separation worker.  Runs in a dedicated module Web Worker, so the UI
//! thread stays responsive while the network runs on WebGPU.
//!
//! The messages it takes and sends are in the `stem-protocol` crate, shared
//! with the page.

#[cfg(target_arch = "wasm32")]
mod web;

/// Installs the message handler (wasm32); nothing natively.
pub fn start() {
    #[cfg(target_arch = "wasm32")]
    web::start();
}
