//! Stem separation worker.  Runs in a dedicated module Web Worker, so the UI
//! thread stays responsive while the network runs on WebGPU.
//!
//! Messages in (plain JS objects):
//! * `{type: "init"}`: load the model from the browser's private file
//!   system (OPFS) if a previous session cached it.
//! * `{type: "model", file: Blob}` or `{type: "model", bytes: Uint8Array}`:
//!   load a `bs_roformer_sw.*.safetensors` file and cache it.
//! * `{type: "separate", left: Float32Array, right: Float32Array,
//!   chunk?: number, overlap?: number}`: 44.1 kHz stereo mix.
//! * `{type: "selftest", base?: "/selftest/"}`: compare GPU results with the
//!   CPU port's reference files (`xtask roformer-selftest`).
//!
//! Messages out:
//! * `{type: "model", ok, text}`
//! * `{type: "progress", done, total, secs}`
//! * `{type: "stem", name, left, right}` (buffers transferred), one per stem
//! * `{type: "done", secs}` or `{type: "error", text}`; a separation stops
//!   with an error as soon as WebGPU reports one (an invalid kernel would
//!   otherwise produce silent garbage)
//! * `{type: "log", text}` and `{type: "selftest", ok, text}`

#[cfg(target_arch = "wasm32")]
mod web;

/// Installs the message handler (wasm32); nothing natively.
pub fn start() {
    #[cfg(target_arch = "wasm32")]
    web::start();
}
