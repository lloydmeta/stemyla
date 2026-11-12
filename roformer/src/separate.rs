//! One chunk through the whole pipeline: STFT, network, masks, inverse STFT.
//!
//! The STFT work stays on the CPU in plain Rust; only the network runs on
//! the Burn backend.  [`separate_chunk`] blocks on the result and suits
//! native backends; in a browser, call [`analyse`], run
//! [`BsRoformer::forward`], read each stem's mask back with
//! `into_data_async`, then call [`apply_masks`].

#[cfg(feature = "burn")]
use burn::tensor::backend::Backend;
#[cfg(feature = "burn")]
use burn::tensor::{Tensor, TensorData};

#[cfg(feature = "burn")]
use crate::model::BsRoformer;
use crate::stft::{Spectrum, frames_for, istft, stft};
use crate::{BINS, CHANNELS, FEATURES, HOP, N_FFT, STEMS};

/// Spectra of both channels and the network input `[T, FEATURES]`
/// (row-major), per frame ordered (bin, channel, re/im).
pub fn analyse(channels: &[Vec<f32>; CHANNELS]) -> ([Spectrum; CHANNELS], Vec<f32>) {
    let spec = [stft(&channels[0]), stft(&channels[1])];
    let frames = spec[0].frames;
    let mut x = vec![0.0f32; frames * FEATURES];
    for t in 0..frames {
        let row = &mut x[t * FEATURES..(t + 1) * FEATURES];
        for f in 0..BINS {
            for (s, sp) in spec.iter().enumerate() {
                let v = sp.data[t * BINS + f];
                row[(f * CHANNELS + s) * 2] = v.re;
                row[(f * CHANNELS + s) * 2 + 1] = v.im;
            }
        }
    }
    (spec, x)
}

/// Multiplies both spectra by one stem's complex mask `[T, FEATURES]` and
/// inverts them.
pub fn apply_masks(spec: &[Spectrum; CHANNELS], mask: &[f32]) -> [Vec<f32>; CHANNELS] {
    let frames = spec[0].frames;
    assert_eq!(mask.len(), frames * FEATURES);
    let mut out: [Vec<f32>; CHANNELS] = Default::default();
    for (s, sp) in spec.iter().enumerate() {
        let mut masked = sp.clone();
        for t in 0..frames {
            for f in 0..BINS {
                let i = t * FEATURES + (f * CHANNELS + s) * 2;
                let (mr, mi) = (mask[i], mask[i + 1]);
                let v = &mut masked.data[t * BINS + f];
                let (a, b) = (v.re, v.im);
                v.re = a * mr - b * mi;
                v.im = a * mi + b * mr;
            }
        }
        out[s] = istft(&masked);
    }
    out
}

/// One buffer of the worker's working set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Part {
    pub name: &'static str,
    pub bytes: usize,
}

/// The buffers that live together at one moment of a phase.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Moment {
    pub at: &'static str,
    pub parts: Vec<Part>,
}

impl Moment {
    pub fn bytes(&self) -> usize {
        self.parts.iter().map(|p| p.bytes).sum()
    }
}

/// One phase of a chunk's separation in the worker, with its moments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Phase {
    pub name: &'static str,
    pub moments: Vec<Moment>,
}

impl Phase {
    /// The largest moment of the phase.
    pub fn largest(&self) -> &Moment {
        self.moments
            .iter()
            .max_by_key(|m| m.bytes())
            .expect("a phase has a moment")
    }

    /// The bytes of the largest moment: the phase's peak.
    pub fn bytes(&self) -> usize {
        self.largest().bytes()
    }
}

/// The worker's working set for one chunk, in three phases.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkingSet {
    /// From the chunk's start to the end of its upload to the GPU: the
    /// STFT and the upload.
    pub upload: Phase,
    /// From the end of the upload to the last stem: each mask and its
    /// inverse STFT.
    pub masks: Phase,
    /// From the last stem to the next chunk's start: the demixer's end of
    /// the chunk, the emit and the next read.
    pub between: Phase,
}

impl WorkingSet {
    pub fn phases(&self) -> [&Phase; 3] {
        [&self.upload, &self.masks, &self.between]
    }

    /// The largest phase.
    pub fn peak(&self) -> &Phase {
        self.phases()
            .into_iter()
            .max_by_key(|p| p.bytes())
            .expect("three phases")
    }
}

/// The live heap bytes that the stem worker (`stem-worker`) holds for one
/// chunk of `chunk` samples at `overlap`, after the warm-up (chunk 2 on),
/// above its live heap before the separation.  It names each buffer that
/// is a chunk's size or more, for each moment of each phase.
///
/// It describes the worker's design at this commit.  A change of that
/// design changes this function in the same commit.  The wasm test
/// `a_chunk_of_the_default_length_holds_its_working_set` measures each
/// phase and fails when this function and the worker do not agree.
pub fn working_set(chunk: usize, overlap: usize) -> WorkingSet {
    const F32: usize = 4;
    const COMPLEX: usize = 8;
    let stems = STEMS.len();
    let frames = frames_for(chunk);
    let step = chunk / overlap;
    // The samples of each channel of a stem: `HOP x (frames - 1)`.
    let stem_channel = HOP * (frames - 1) * F32;
    let part = |name, bytes| Part { name, bytes };
    let demixer = [
        part("demixer sums", stems * CHANNELS * chunk * F32),
        part("demixer counter", chunk * F32),
        part("demixer window", chunk * F32),
    ];
    let chunk_input = part("chunk input", CHANNELS * chunk * F32);
    let spectra = part("spectra", CHANNELS * frames * BINS * COMPLEX);
    let features = part("features", frames * FEATURES * F32);
    let moment = |at, parts: &[Part]| Moment {
        at,
        parts: demixer.iter().chain(parts).copied().collect(),
    };
    let upload = Phase {
        name: "upload",
        moments: vec![
            moment(
                "the end of the STFT (`analyse`)",
                &[chunk_input, spectra, features],
            ),
            moment(
                "the normalisation in `Job::set_input`",
                &[
                    chunk_input,
                    spectra,
                    features,
                    part("normalised input", frames * FEATURES * F32),
                ],
            ),
        ],
    };
    let masks = Phase {
        name: "masks",
        moments: vec![
            moment(
                "a stem's inverse STFT of channel 1",
                &[
                    chunk_input,
                    spectra,
                    part("mask", frames * FEATURES * F32),
                    part("masked spectrum", frames * BINS * COMPLEX),
                    part("inverse STFT sums", 2 * (N_FFT + HOP * (frames - 1)) * F32),
                    part("stem output, channel 0", stem_channel),
                    part("stem output, channel 1", stem_channel),
                ],
            ),
            moment(
                "the demixer's `accept_stem`",
                &[
                    chunk_input,
                    spectra,
                    part("one stem", CHANNELS * stem_channel),
                ],
            ),
        ],
    };
    let between = Phase {
        name: "between",
        moments: vec![
            moment(
                "the emit of the segment (`StemWriter::put`)",
                &[
                    chunk_input,
                    part("segment", stems * CHANNELS * step * F32),
                    part("PCM bytes", CHANNELS * 2 * step),
                ],
            ),
            moment(
                "the next read of the mix file (`StoredMix::read`)",
                &[
                    part("bytes of the span", CHANNELS * chunk * F32),
                    part("span", CHANNELS * chunk * F32),
                ],
            ),
        ],
    };
    WorkingSet {
        upload,
        masks,
        between,
    }
}

/// Upload the features to the model's device.
#[cfg(feature = "burn")]
pub fn to_tensor<B: Backend>(features: Vec<f32>, device: &B::Device) -> Tensor<B, 2> {
    let frames = features.len() / FEATURES;
    Tensor::from_data(TensorData::new(features, [frames, FEATURES]), device)
}

#[cfg(feature = "burn")]
/// Separates one chunk (both channels the same length).  Returns, per stem,
/// both channels with `HOP * (frames - 1)` samples.
pub fn separate_chunk<B: Backend>(
    model: &BsRoformer<B>,
    channels: &[Vec<f32>; CHANNELS],
) -> Vec<[Vec<f32>; CHANNELS]> {
    let (spec, x) = analyse(channels);
    // One stem's mask at a time keeps every tensor under `Options::max_elems`.
    let hidden = model.hidden(to_tensor::<B>(x, model.device()));
    (0..crate::STEMS.len())
        .map(|s| {
            let m: Vec<f32> = model
                .mask_one(s, hidden.clone())
                .into_data()
                .to_vec()
                .expect("f32 tensor");
            apply_masks(&spec, &m)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CHUNK, OVERLAP};

    /// The bytes of the part `name` in `moment`, or 0 if it has none.
    fn part(moment: &Moment, name: &str) -> usize {
        moment
            .parts
            .iter()
            .find(|p| p.name == name)
            .map_or(0, |p| p.bytes)
    }

    /// The table "The buffers of one chunk at the default chunk" of
    /// `docs/plans/active/gpu-speed-m1.md`, at `CHUNK` and overlap 2.
    #[test]
    fn the_working_set_at_the_default_chunk_has_the_sizes_of_its_buffers() {
        let w = working_set(CHUNK, OVERLAP);
        let demixer = [
            ("demixer sums", 28_262_400),
            ("demixer counter", 2_355_200),
            ("demixer window", 2_355_200),
        ];
        for phase in w.phases() {
            for moment in &phase.moments {
                for (name, bytes) in demixer {
                    assert_eq!(part(moment, name), bytes, "{} at {}", name, moment.at);
                }
            }
        }
        let upload = w.upload.largest();
        assert_eq!(part(upload, "chunk input"), 4_710_400);
        assert_eq!(part(upload, "spectra"), 18_876_400);
        assert_eq!(part(upload, "features"), 18_876_400);
        assert_eq!(part(upload, "normalised input"), 18_876_400);
        assert_eq!(w.upload.bytes(), 94_312_400);

        let masks = w.masks.largest();
        assert_eq!(part(masks, "chunk input"), 4_710_400);
        assert_eq!(part(masks, "spectra"), 18_876_400);
        assert_eq!(part(masks, "five stems"), 0, "cut 1");
        assert_eq!(part(masks, "mask"), 18_876_400);
        assert_eq!(part(masks, "masked spectrum"), 9_438_200);
        assert_eq!(part(masks, "inverse STFT sums"), 4_726_784);
        assert_eq!(part(masks, "stem output, channel 0"), 2_355_200);
        assert_eq!(part(masks, "stem output, channel 1"), 2_355_200);
        assert_eq!(w.masks.bytes(), 94_311_384);
        assert_eq!(part(&w.masks.moments[1], "one stem"), 4_710_400);

        let between = w.between.largest();
        assert_eq!(part(between, "chunk input"), 4_710_400);
        assert_eq!(part(between, "six stems"), 0, "cut 1");
        assert_eq!(part(between, "window clone"), 0, "cut 1");
        assert_eq!(part(between, "segment"), 14_131_200);
        assert_eq!(part(between, "PCM bytes"), 1_177_600);
        assert_eq!(w.between.bytes(), 52_992_000);
        let read = &w.between.moments[1];
        assert_eq!(part(read, "bytes of the span"), 4_710_400);
        assert_eq!(part(read, "span"), 4_710_400);

        assert_eq!(w.peak().name, "upload");
    }

    /// The sizes follow the chunk: a quarter of the default chunk at
    /// overlap 4 gives the sizes of its own frames.
    #[test]
    fn the_working_set_follows_the_chunk_and_the_overlap() {
        let chunk = CHUNK / 4;
        let w = working_set(chunk, 4);
        let frames = frames_for(chunk);
        let upload = w.upload.largest();
        assert_eq!(part(upload, "chunk input"), 2 * chunk * 4);
        assert_eq!(part(upload, "spectra"), 2 * frames * BINS * 8);
        let emit = &w.between.moments[0];
        assert_eq!(part(emit, "segment"), STEMS.len() * 2 * (chunk / 4) * 4);
    }
}
