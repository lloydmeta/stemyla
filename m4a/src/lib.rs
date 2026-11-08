//! The bytes of an M4A file that holds 16-bit stereo Apple Lossless (ALAC).
//!
//! This crate makes bytes only.  It does no input or output.  The caller
//! writes the bytes, as `stem-worker` does for FLAC.
//!
//! The crate is Apache-2.0 only, because its ALAC encoder is a port of
//! Apple's ALAC reference (decision 0028).

#![forbid(unsafe_code)]

mod alac;
mod bits;
mod mp4;

pub use alac::{AlacEncoder, FRAMES_PER_PACKET, MAX_PACKET_BYTES};
pub use mp4::{Edit, Error, Layout, Order, SampleEntry};

#[cfg(test)]
mod tests {
    #[test]
    fn mem1_lint_is_on() {
        #[expect(
            clippy::disallowed_methods,
            reason = "MEM-1 canary: clippy fails here if this path is off"
        )]
        let copy = [1_i16].to_vec();
        assert_eq!(copy, [1]);
    }
}
