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
use crate::stft::{Spectrum, istft, stft};
use crate::{BINS, CHANNELS, FEATURES};

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

/// Upload the features to the model's device.
#[cfg(feature = "burn")]
pub fn to_tensor<B: Backend>(features: Vec<f32>, device: &B::Device) -> Tensor<B, 2> {
    let frames = features.len() / FEATURES;
    Tensor::from_data(TensorData::new(features, [frames, FEATURES]), device)
}

#[cfg(feature = "burn")]
/// Separates one chunk (both channels the same length).  Returns, per stem,
/// both channels with `HOP * (frames - 1)` samples.
pub fn separate_chunk<B: Backend>(model: &BsRoformer<B>, channels: &[Vec<f32>; CHANNELS]) -> Vec<[Vec<f32>; CHANNELS]> {
    let (spec, x) = analyse(channels);
    // One stem's mask at a time keeps every tensor under `Options::max_elems`.
    let hidden = model.hidden(to_tensor::<B>(x, model.device()));
    (0..crate::STEMS.len())
        .map(|s| {
            let m: Vec<f32> = model.mask_one(s, hidden.clone()).into_data().to_vec().expect("f32 tensor");
            apply_masks(&spec, &m)
        })
        .collect()
}
