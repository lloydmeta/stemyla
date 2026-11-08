//! The bytes of an M4A file that holds 16-bit stereo Apple Lossless (ALAC),
//! or AAC from given packets.  The crate has no AAC encoder: the caller
//! gives the packets and the AudioSpecificConfig.
//!
//! This crate makes bytes only.  It does no input or output.  The caller
//! writes the bytes, as `stem-worker` does for FLAC.
//!
//! The crate is Apache-2.0 only, because its ALAC encoder is a port of
//! Apple's ALAC reference (decision 0029).

#![forbid(unsafe_code)]

mod alac;
mod bits;
mod mp4;

pub use alac::{AlacEncoder, FRAMES_PER_PACKET, MAX_PACKET_BYTES};
pub use mp4::{Codec, Edit, Error, Layout, MAX_CONFIG_LEN, Order, SampleEntry};

/// The whole M4A file of `interleaved` 16-bit stereo, as ALAC.  It holds
/// the file in memory, so it is for tests and tools: the worker writes in
/// blocks with `Layout` and `AlacEncoder` (M3).  Every test that needs an
/// M4A uses this one writer (TD-52).
///
/// # Panics
///
/// On an odd length of `interleaved`.
pub fn alac_file(
    interleaved: &[i16],
    sample_rate: u32,
    edit: Option<Edit>,
    order: Order,
) -> Result<Vec<u8>, Error> {
    assert!(
        interleaved.len().is_multiple_of(2),
        "ALAC input is interleaved stereo, so its length must be even, not {}",
        interleaved.len()
    );
    let layout = Layout::new(
        Codec::Alac,
        (interleaved.len() / 2) as u64,
        sample_rate,
        edit,
        order,
    )?;
    let mut encoder = AlacEncoder::new(sample_rate);
    let mut file = vec![0; layout.head_len()];
    let sizes: Vec<u32> = interleaved
        .chunks(FRAMES_PER_PACKET * 2)
        .map(|packet| encoder.encode(packet, &mut file) as u32)
        .collect();
    let entry = SampleEntry::Alac {
        cookie: encoder.cookie(),
    };
    let head = layout.head(&entry, &sizes)?;
    file[..head.len()].copy_from_slice(&head);
    file.extend_from_slice(&layout.tail(&entry, &sizes)?);
    Ok(file)
}

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
