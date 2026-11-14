//! The write worker (M5.1 of `docs/plans/active/gpu-speed.md`): a dedicated
//! Web Worker on the page side.  It writes the page's song-sized files to
//! browser storage (OPFS) through sync access handles: the source files of
//! a song, its mix, the stems of the memory path and the save files.
//! Safari keeps a swap file of each write stream that closes (TD-103), and
//! a sync access handle exists only in a dedicated worker.  Its messages
//! are in `stem_protocol::write`, shared with the page.
//!
//! Only the browser glue, in `web`, is for wasm32; natively the crate is
//! empty.

#[cfg(target_arch = "wasm32")]
mod web;

/// Installs the message handlers (wasm32); nothing natively.
pub fn start() {
    #[cfg(target_arch = "wasm32")]
    web::start();
}

#[cfg(test)]
mod tests {
    /// MEM-1: proves the native path of this crate's `clippy.toml` (see the
    /// `#[expect]`).  `mem1_lint_is_on_for_js_arrays` proves the others.
    #[test]
    fn mem1_lint_is_on() {
        #[expect(
            clippy::disallowed_methods,
            reason = "MEM-1 canary: clippy fails here if this path is off"
        )]
        let copy = [1.0_f32].to_vec();
        assert_eq!(copy, [1.0]);
    }
}
