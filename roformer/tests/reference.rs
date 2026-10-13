//! Checks against PyTorch fixtures written by `tools/roformer_export.py`
//! into `third_party/roformer/` (gitignored; the weights are not ours to
//! publish).  Tests skip with a message when the fixtures are absent.

use std::path::PathBuf;

use burn::backend::NdArray;
use burn::tensor::{Tensor, TensorData};
use roformer::demix::{Demixer, Stereo};
use roformer::model::{BsRoformer, Options};
use roformer::separate::{analyse, apply_masks, to_tensor};
use roformer::stft::{istft, stft};
use roformer::{BINS, FEATURES, STEMS};
use safetensors::{Dtype, SafeTensors};

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../third_party/roformer")
}

fn read(name: &str) -> Option<Vec<u8>> {
    let p = fixture_dir().join(name);
    match std::fs::read(&p) {
        Ok(b) => Some(b),
        Err(_) => {
            eprintln!(
                "skipping: {} not found (run tools/roformer_export.py)",
                p.display()
            );
            None
        }
    }
}

struct Fixture<'a>(SafeTensors<'a>);

impl Fixture<'_> {
    fn f32(&self, name: &str) -> (Vec<usize>, Vec<f32>) {
        let v = self
            .0
            .tensor(name)
            .unwrap_or_else(|_| panic!("fixture lacks {name}"));
        assert_eq!(v.dtype(), Dtype::F32);
        let data = v
            .data()
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        (v.shape().to_vec(), data)
    }
    fn i64(&self, name: &str) -> Vec<i64> {
        let v = self.0.tensor(name).unwrap();
        assert_eq!(v.dtype(), Dtype::I64);
        v.data()
            .chunks_exact(8)
            .map(|c| i64::from_le_bytes(c.try_into().unwrap()))
            .collect()
    }
}

/// Signal-to-error ratio of `got` against `want`, in dB.
fn snr(got: &[f32], want: &[f32]) -> f64 {
    assert_eq!(got.len(), want.len());
    let mut s = 0.0f64;
    let mut e = 0.0f64;
    for (a, b) in got.iter().zip(want) {
        s += (*b as f64).powi(2);
        e += (*a as f64 - *b as f64).powi(2);
    }
    if e == 0.0 {
        f64::INFINITY
    } else {
        10.0 * (s / e).log10()
    }
}

fn tensor3(shape: &[usize], data: Vec<f32>) -> Tensor<NdArray, 3> {
    Tensor::from_data(
        TensorData::new(data, [shape[0], shape[1], shape[2]]),
        &Default::default(),
    )
}

fn vec3(t: Tensor<NdArray, 3>) -> Vec<f32> {
    t.into_data().to_vec().unwrap()
}

/// `[T, F, D]` fixture layout to the model's `[F, T, D]`.
fn tfd_to_ftd(shape: &[usize], data: Vec<f32>) -> Tensor<NdArray, 3> {
    tensor3(shape, data).swap_dims(0, 1)
}

/// Network input features built from torch.stft's output (`[S, F, T, 2]`).
fn torch_features(spec_shape: &[usize], spec: &[f32]) -> Vec<f32> {
    let frames = spec_shape[2];
    let mut feats = vec![0.0f32; frames * FEATURES];
    for t in 0..frames {
        for f in 0..BINS {
            for s in 0..2 {
                let i = ((s * BINS + f) * frames + t) * 2;
                feats[t * FEATURES + (f * 2 + s) * 2] = spec[i];
                feats[t * FEATURES + (f * 2 + s) * 2 + 1] = spec[i + 1];
            }
        }
    }
    feats
}

#[test]
fn stft_matches_torch() {
    let Some(bytes) = read("stft.safetensors") else {
        return;
    };
    let fx = Fixture(SafeTensors::deserialize(&bytes).unwrap());
    let (ashape, audio) = fx.f32("audio");
    let n = ashape[1];
    let (sshape, spec) = fx.f32("spec"); // [S, F, T, 2]
    let (_, back) = fx.f32("istft");
    let frames = sshape[2];
    for c in 0..2 {
        let x = &audio[c * n..(c + 1) * n];
        let s = stft(x);
        assert_eq!((s.frames, s.bins), (frames, BINS));
        let mut got = Vec::new();
        let mut want = Vec::new();
        for f in 0..BINS {
            for t in 0..frames {
                let v = s.data[t * BINS + f];
                got.extend([v.re, v.im]);
                let i = ((c * BINS + f) * frames + t) * 2;
                want.extend([spec[i], spec[i + 1]]);
            }
        }
        let db = snr(&got, &want);
        println!("stft channel {c}: {db:.1} dB");
        assert!(db > 100.0);
        let y = istft(&s);
        let m = y.len();
        let db = snr(&y, &back[c * m..(c + 1) * m]);
        println!("istft channel {c}: {db:.1} dB");
        assert!(db > 100.0);
    }
}

fn stand_in(part: &Stereo) -> Vec<Stereo> {
    (0..6)
        .map(|k| {
            part.clone().map(|ch| {
                ch.iter()
                    .map(|x| (x * (k + 1) as f32 * 0.5).tanh() + 0.01 * k as f32)
                    .collect()
            })
        })
        .collect()
}

#[test]
fn demix_matches_reference() {
    let Some(bytes) = read("demix.safetensors") else {
        return;
    };
    let fx = Fixture(SafeTensors::deserialize(&bytes).unwrap());
    for case in ["a", "b", "c", "d"] {
        let p = fx.i64(&format!("{case}.params"));
        let (chunk, overlap, len) = (p[0] as usize, p[1] as usize, p[2] as usize);
        let (_, mix) = fx.f32(&format!("{case}.mix"));
        let (_, want) = fx.f32(&format!("{case}.out"));
        let mut d = Demixer::new(
            [mix[..len].to_vec(), mix[len..].to_vec()],
            chunk,
            overlap,
            6,
        );
        let chunks = d.num_chunks();
        let mut seen = 0;
        while let Some(x) = d.next_input() {
            let y = stand_in(&x);
            d.accept(&y);
            seen += 1;
        }
        assert_eq!(seen, chunks);
        let got: Vec<f32> = d
            .finish()
            .into_iter()
            .flat_map(|s| s.into_iter().flatten())
            .collect();
        let max = got
            .iter()
            .zip(&want)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f32::max);
        println!(
            "demix case {case}: chunk {chunk} overlap {overlap} length {len}, {chunks} chunks, max error {max:e}"
        );
        assert_eq!(got.len(), want.len());
        assert!(max < 1e-5);
    }
}

#[test]
fn forward_matches_pytorch() {
    let Some(weights) = read("bs_roformer_sw.f32.safetensors") else {
        return;
    };
    let Some(bytes) = read("forward.safetensors") else {
        return;
    };
    let Some(sbytes) = read("stft.safetensors") else {
        return;
    };
    let fx = Fixture(SafeTensors::deserialize(&bytes).unwrap());
    let sx = Fixture(SafeTensors::deserialize(&sbytes).unwrap());
    let dev = Default::default();
    let t0 = std::time::Instant::now();
    let model = BsRoformer::<NdArray>::load(&weights, &dev, Options::default()).unwrap();
    println!("loaded in {:.1} s", t0.elapsed().as_secs_f64());

    let (ishape, input) = fx.f32("input");
    let n = ishape[1];
    let (spec, features) = analyse(&[input[..n].to_vec(), input[n..].to_vec()]);
    let frames = spec[0].frames;

    // Band split, from torch.stft's features and from ours.  RMSNorm
    // rescales every band to unit length, so in bands that are nearly
    // silent (above the MP3's low-pass) FFT rounding differences of
    // -135 dB grow to visible differences; the network's own output there
    // multiplies a near-zero spectrum, so the audio is unaffected (checked
    // end to end below).
    let (sshape, sdata) = sx.f32("spec");
    let x = model.band_split(to_tensor(torch_features(&sshape, &sdata), &dev));
    let (shape, want) = fx.f32("band_split");
    let db = snr(&vec3(x.swap_dims(0, 1)), &want);
    println!("band split (torch STFT): {db:.1} dB");
    assert!(db > 120.0);
    let x = model.band_split(to_tensor(features, &dev));
    println!(
        "band split (our STFT): {:.1} dB",
        snr(&vec3(x.swap_dims(0, 1)), &want)
    );

    // Each layer from the PyTorch input to that layer.
    let mut prev = tfd_to_ftd(&shape, want);
    for i in 0..roformer::DEPTH {
        let (shape, want) = fx.f32(&format!("layer{i}.freq"));
        let got = model.layer(i, prev);
        let db = snr(&vec3(got.swap_dims(0, 1)), &want);
        println!("layer {i}: {db:.1} dB");
        assert!(db > 80.0, "layer {i}");
        prev = tfd_to_ftd(&shape, want);
    }
    let (_, want) = fx.f32("final_norm");
    let normed = model.final_norm(prev);
    let db = snr(&vec3(normed.clone().swap_dims(0, 1)), &want);
    println!("final norm: {db:.1} dB");
    assert!(db > 90.0);

    for s in 0..STEMS.len() {
        let (_, want) = fx.f32(&format!("mask{s}"));
        let got: Vec<f32> = model
            .mask_one(s, normed.clone())
            .into_data()
            .to_vec()
            .unwrap();
        assert_eq!(got.len(), frames * FEATURES);
        let db = snr(&got, &want);
        println!("mask {} ({}): {db:.1} dB", s, STEMS[s]);
        assert!(db > 80.0);
    }

    // End to end: STFT, the whole network, masks, ISTFT.  Errors are given
    // relative to the stem and to the mix: the guitar is almost silent in
    // this clip, so a tiny absolute error is a large fraction of it.
    let (oshape, want) = fx.f32("output"); // [6, 2, L]
    let len = oshape[2];
    let mix_energy: f64 =
        input.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() * len as f64 / n as f64;
    let mut secs = 0.0;
    for (label, feats) in [
        ("torch STFT", torch_features(&sshape, &sdata)),
        (
            "our STFT",
            analyse(&[input[..n].to_vec(), input[n..].to_vec()]).1,
        ),
    ] {
        let t0 = std::time::Instant::now();
        let masks = model.forward(to_tensor(feats, &dev));
        let mask_data: Vec<f32> = masks.into_data().to_vec().unwrap();
        secs = t0.elapsed().as_secs_f64();
        for s in 0..STEMS.len() {
            let m = &mask_data[s * frames * FEATURES..(s + 1) * frames * FEATURES];
            let audio = apply_masks(&spec, m);
            let mut err = 0.0f64;
            let mut dbs = Vec::new();
            for c in 0..2 {
                let w = &want[(s * 2 + c) * len..(s * 2 + c + 1) * len];
                dbs.push(snr(&audio[c], w));
                err += audio[c]
                    .iter()
                    .zip(w)
                    .map(|(a, b)| (*a as f64 - *b as f64).powi(2))
                    .sum::<f64>();
            }
            let vs_mix = 10.0 * (mix_energy / err).log10();
            println!(
                "output {} ({label}): {:.1} / {:.1} dB vs stem, {vs_mix:.1} dB vs mix",
                STEMS[s], dbs[0], dbs[1]
            );
            assert!(vs_mix > 70.0, "{} {label}", STEMS[s]);
        }
    }
    println!("forward pass on {frames} frames: {secs:.1} s");
}

/// Half-precision weights (half the download) against the f32 reference.
#[test]
fn f16_weights_stay_close() {
    let Some(weights) = read("bs_roformer_sw.f16.safetensors") else {
        return;
    };
    let Some(bytes) = read("forward.safetensors") else {
        return;
    };
    let Some(sbytes) = read("stft.safetensors") else {
        return;
    };
    let fx = Fixture(SafeTensors::deserialize(&bytes).unwrap());
    let sx = Fixture(SafeTensors::deserialize(&sbytes).unwrap());
    let dev = Default::default();
    let model = BsRoformer::<NdArray>::load(&weights, &dev, Options::default()).unwrap();
    let (sshape, sdata) = sx.f32("spec");
    let (ishape, input) = fx.f32("input");
    let n = ishape[1];
    let (spec, _) = analyse(&[input[..n].to_vec(), input[n..].to_vec()]);
    let frames = spec[0].frames;
    let masks: Vec<f32> = model
        .forward(to_tensor(torch_features(&sshape, &sdata), &dev))
        .into_data()
        .to_vec()
        .unwrap();
    let (oshape, want) = fx.f32("output");
    let len = oshape[2];
    let mix_energy: f64 =
        input.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() * len as f64 / n as f64;
    for s in 0..STEMS.len() {
        let audio = apply_masks(
            &spec,
            &masks[s * frames * FEATURES..(s + 1) * frames * FEATURES],
        );
        let mut err = 0.0f64;
        let mut energy = 0.0f64;
        for c in 0..2 {
            let w = &want[(s * 2 + c) * len..(s * 2 + c + 1) * len];
            err += audio[c]
                .iter()
                .zip(w)
                .map(|(a, b)| (*a as f64 - *b as f64).powi(2))
                .sum::<f64>();
            energy += w.iter().map(|b| (*b as f64).powi(2)).sum::<f64>();
        }
        let vs_stem = 10.0 * (energy / err).log10();
        let vs_mix = 10.0 * (mix_energy / err).log10();
        println!(
            "f16 weights, {}: {vs_stem:.1} dB vs stem, {vs_mix:.1} dB vs mix",
            STEMS[s]
        );
        assert!(vs_mix > 40.0);
    }
}

/// Running a layer sub-batch by sub-batch (as the browser worker does, to
/// pace the GPU) gives the same result as `layer`.
#[test]
fn layer_in_parts_matches_layer() {
    use roformer::model::Axis;
    let Some(weights) = read("bs_roformer_sw.f16.safetensors") else {
        return;
    };
    let Some(bytes) = read("forward.safetensors") else {
        return;
    };
    let fx = Fixture(SafeTensors::deserialize(&bytes).unwrap());
    let dev = Default::default();
    // A small element cap forces several sub-batches on both axes.
    let opts = Options {
        max_elems: 1 << 20,
        ..Options::default()
    };
    let model = BsRoformer::<NdArray>::load(&weights, &dev, opts).unwrap();
    let (shape, x) = fx.f32("band_split");
    let x = tfd_to_ftd(&shape, x);
    let whole = vec3(model.layer(3, x.clone()));
    let [bands, frames, _] = x.dims();
    let step = model.sub_batch(bands, frames);
    assert!(step < bands);
    let mut parts = Vec::new();
    for k in (0..bands).step_by(step) {
        parts.push(model.transformer_part(
            3,
            Axis::Time,
            x.clone().narrow(0, k, step.min(bands - k)),
        ));
    }
    let x = Tensor::cat(parts, 0).swap_dims(0, 1);
    let step = model.sub_batch(frames, bands);
    assert!(step < frames);
    let mut parts = Vec::new();
    for k in (0..frames).step_by(step) {
        parts.push(model.transformer_part(
            3,
            Axis::Freq,
            x.clone().narrow(0, k, step.min(frames - k)),
        ));
    }
    let pieces = vec3(Tensor::cat(parts, 0).swap_dims(0, 1));
    let db = snr(&pieces, &whole);
    println!("layer in parts vs whole: {db:.1} dB");
    assert!(db > 140.0);
}
