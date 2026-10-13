//! BS-RoFormer SW inference on Burn.
//!
//! A port of the band-split RoPE transformer (Lu et al., 2023) as packaged by
//! `bs-roformer-infer` 0.1.5 with the `BS-Rofo-SW-Fixed` configuration: six
//! stems (bass, drums, other, vocals, guitar, piano), stereo, 44.1 kHz.
//!
//! The pipeline per chunk of [`CHUNK`] samples:
//! 1. [`stft`] of both channels on the CPU, packed into network input
//!    features ([`separate::features`]).
//! 2. [`model::BsRoformer::forward`] on any Burn backend: band split,
//!    12 alternating time and frequency transformers, one mask estimator per
//!    stem.  Returns complex masks.
//! 3. Masks applied to the spectrum and inverted on the CPU
//!    ([`separate::apply_masks`]).
//!
//! [`demix::Demixer`] cuts a song into overlapping chunks and overlap-adds
//! the results exactly like `bs_roformer.utils.demix_track`.

pub mod demix;
#[cfg(feature = "burn")]
pub mod model;
pub mod separate;
pub mod stft;
pub mod weights;

/// Sample rate the model was trained on.
pub const SAMPLE_RATE: u32 = 44_100;
/// Audio channels (the model is stereo).
pub const CHANNELS: usize = 2;
/// STFT size, hop and window length.
pub const N_FFT: usize = 2048;
pub const HOP: usize = 512;
/// One-sided STFT bins.
pub const BINS: usize = N_FFT / 2 + 1;
/// Transformer width, heads, head size and depth.
pub const DIM: usize = 256;
pub const HEADS: usize = 8;
pub const DIM_HEAD: usize = 64;
pub const DEPTH: usize = 12;
/// Feed-forward and mask-estimator hidden width.
pub const HIDDEN: usize = DIM * 4;
/// Samples per model call (13.35 s) and overlap factor used by the
/// reference inference config.
pub const CHUNK: usize = 588_800;
pub const OVERLAP: usize = 2;

/// Output stems in model order.
pub const STEMS: [&str; 6] = ["bass", "drums", "other", "vocals", "guitar", "piano"];

/// STFT bins per band (the `DEFAULT_FREQS_PER_BANDS` of BS-RoFormer); sums
/// to [`BINS`].
pub const FREQS_PER_BANDS: [usize; 62] = [
    2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, //
    4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, //
    12, 12, 12, 12, 12, 12, 12, 12, //
    24, 24, 24, 24, 24, 24, 24, 24, //
    48, 48, 48, 48, 48, 48, 48, 48, //
    128, 129,
];

/// Input features per STFT frame: bins x channels x (re, im).
pub const FEATURES: usize = BINS * CHANNELS * 2;

/// A run of consecutive bands with the same width.  Bands in one group
/// share tensor shapes, so the network handles them as one batched matmul.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BandGroup {
    /// Index of the first band.
    pub first: usize,
    /// Number of bands.
    pub count: usize,
    /// Features per band (`4 * bins`).
    pub width: usize,
    /// Offset of the first band's features in a frame.
    pub offset: usize,
}

/// The seven band groups of [`FREQS_PER_BANDS`].
pub fn band_groups() -> Vec<BandGroup> {
    let mut groups: Vec<BandGroup> = Vec::new();
    let mut offset = 0;
    for (i, &f) in FREQS_PER_BANDS.iter().enumerate() {
        let width = f * CHANNELS * 2;
        match groups.last_mut() {
            Some(g) if g.width == width => g.count += 1,
            _ => groups.push(BandGroup { first: i, count: 1, width, offset }),
        }
        offset += width;
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bands_cover_every_bin() {
        assert_eq!(FREQS_PER_BANDS.iter().sum::<usize>(), BINS);
        let g = band_groups();
        assert_eq!(g.len(), 7);
        let last = g.last().unwrap();
        assert_eq!(last.offset + last.count * last.width, FEATURES);
        assert_eq!(g.iter().map(|g| g.count).sum::<usize>(), 62);
    }
}
