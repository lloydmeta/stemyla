//! The decode worker (decision 0019): a dedicated Web Worker on the page
//! side.  It decodes a song that the page keeps in browser storage, in
//! blocks, with `engine`.  It writes the session files and the overviews
//! of the song (`docs/plans/active/stream-from-storage-m5.md`, D1 to D12).
//! Its messages are in `stem_protocol::decode`, shared with the page.
//!
//! The pass logic ([`pass`], [`source`]) compiles on every target, so
//! `cargo test -p decode-worker` runs its tests natively on files in
//! memory.  Only the browser glue, in `web`, is for wasm32.

pub mod pass;
pub mod source;
#[cfg(target_arch = "wasm32")]
mod web;

/// Installs the message handler (wasm32); nothing natively.
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
