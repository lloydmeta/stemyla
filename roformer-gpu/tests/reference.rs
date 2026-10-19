//! The whole network on the GPU kernels against the PyTorch fixtures that
//! `tools/roformer_export.py` writes into `third_party/roformer/`
//! (gitignored).  Same thresholds as the CPU port's own test
//! (`roformer/tests/reference.rs`).  Skips without fixtures or adapter, and
//! fails instead under `STEMYLA_REQUIRE=gpu,fixtures`; see `kernels.rs` for
//! running on SwiftShader.

use std::path::PathBuf;

use roformer::separate::{analyse, apply_masks};
use roformer::{BINS, DEPTH, FEATURES, STEMS};
use roformer_gpu::net::{Job, Net, Step};
use roformer_gpu::{Gpu, MatMulPolicy, TILES};
use safetensors::{Dtype, SafeTensors};
use test_support::{Need, skip};

#[track_caller]
fn read(name: &str) -> Option<Vec<u8>> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../third_party/roformer")
        .join(name);
    match std::fs::read(&p) {
        Ok(b) => Some(b),
        Err(_) => {
            skip(
                Need::Fixtures,
                format!("{} not found (run tools/roformer_export.py)", p.display()),
            );
            None
        }
    }
}

fn f32s(st: &SafeTensors, name: &str) -> (Vec<usize>, Vec<f32>) {
    let v = st
        .tensor(name)
        .unwrap_or_else(|_| panic!("fixture lacks {name}"));
    assert_eq!(v.dtype(), Dtype::F32);
    (
        v.shape().to_vec(),
        v.data()
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
    )
}

fn snr(got: &[f32], want: &[f32]) -> f64 {
    assert_eq!(got.len(), want.len());
    let (mut s, mut e) = (0.0f64, 0.0f64);
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

/// `[A, B, D]` to `[B, A, D]`.
fn swap01(shape: &[usize], x: &[f32]) -> Vec<f32> {
    let (a, b, d) = (shape[0], shape[1], shape[2]);
    let mut out = vec![0.0; x.len()];
    for i in 0..a {
        for j in 0..b {
            out[(j * a + i) * d..][..d].copy_from_slice(&x[(i * b + j) * d..][..d]);
        }
    }
    out
}

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

fn run(job: &Job, gpu: &Gpu, steps: &[Step]) {
    for s in steps {
        job.run(s);
        pollster::block_on(gpu.done());
    }
    let errors = gpu.take_errors();
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn forward_matches_pytorch() {
    let Some(weights) = read("bs_roformer_sw.f32.safetensors") else {
        return;
    };
    let Some(fbytes) = read("forward.safetensors") else {
        return;
    };
    let Some(sbytes) = read("stft.safetensors") else {
        return;
    };
    // ROFORMER_GPU_MM=v64 (or any `MatMulPolicy` name, "v64,i64" too) runs
    // the network with those matmul kernels instead of the fallback, on a
    // profiling device so the test can see they ran.  A named kernel that
    // fails the start-up check, or that no product runs (the driver hands
    // products a vec4 kernel cannot read to the fallback), fails the test.
    let mm = std::env::var("ROFORMER_GPU_MM").ok();
    let opened = if mm.is_some() {
        pollster::block_on(Gpu::new_profiling(wgpu::PowerPreference::HighPerformance))
    } else {
        pollster::block_on(Gpu::new(wgpu::PowerPreference::HighPerformance))
    };
    let gpu = match opened {
        Ok(g) => g,
        Err(e) => return skip(Need::Gpu, &e),
    };
    eprintln!("adapter: {} ({:?})", gpu.info.name, gpu.info.backend);
    if mm.is_some() && !gpu.profiling() {
        skip(
            Need::Timestamps,
            "adapter without timestamp-query: the kernels that ran are not checked",
        );
    }
    if let Some(name) = &mm {
        let policy = MatMulPolicy::parse(name).unwrap_or_else(|| panic!("unknown kernel {name}"));
        let problems = pollster::block_on(gpu.check_kernels()).unwrap();
        let usable = gpu.usable.get();
        assert!(
            usable[policy.pick(1000, 1000)] && usable[policy.pick(10, 10)],
            "{name} failed the start-up check: {problems:?}"
        );
        pollster::block_on(gpu.reset_profile()).unwrap();
        gpu.policy.set(policy);
    }
    eprintln!("matmul {}", gpu.policy.get().name());
    let fx = SafeTensors::deserialize(&fbytes).unwrap();
    let sx = SafeTensors::deserialize(&sbytes).unwrap();
    // A small budget, so every transformer and mask group runs in several
    // steps and the sub-batch offsets are exercised.
    let net = Net::load(&gpu, &weights, 400_000).unwrap();
    let (sshape, sdata) = f32s(&sx, "spec");
    let frames = sshape[2];
    let job = Job::new(&gpu, &net, frames);
    let steps = job.steps();
    let count = |f: fn(&Step) -> bool| steps.iter().filter(|s| f(s)).count();
    eprintln!(
        "{frames} frames, {} steps: {} time, {} freq, {} mask",
        steps.len(),
        count(|s| matches!(s, Step::Time { .. })),
        count(|s| matches!(s, Step::Freq { .. })),
        count(|s| matches!(s, Step::Mask { .. })),
    );
    assert!(count(|s| matches!(s, Step::Time { .. })) > DEPTH);

    let feats = torch_features(&sshape, &sdata);
    job.set_input(&feats);
    run(&job, &gpu, &[Step::Split]);
    let (shape, want) = f32s(&fx, "band_split"); // [T, F, D]
    let got = swap01(
        &[shape[1], shape[0], shape[2]],
        &pollster::block_on(job.hidden()).unwrap(),
    );
    let db = snr(&got, &want);
    eprintln!("band split: {db:.1} dB");
    assert!(db > 120.0);

    // SwiftShader runs about 0.15 GFLOP/s here, 2.5 minutes per layer, so by
    // default only the first two and last two layers are checked and the
    // end-to-end pass is skipped; ROFORMER_GPU_FULL=1 runs everything.
    let full = std::env::var_os("ROFORMER_GPU_FULL").is_some();
    let mut prev = (shape.clone(), want);
    for i in 0..DEPTH {
        if !full && (2..DEPTH - 2).contains(&i) {
            let (shape, want) = f32s(&fx, &format!("layer{i}.freq"));
            prev = (shape, want);
            continue;
        }
        job.set_hidden(&swap01(&prev.0, &prev.1));
        run(&job, &gpu, &job.layer_steps(i));
        let (shape, want) = f32s(&fx, &format!("layer{i}.freq"));
        let got = swap01(
            &[shape[1], shape[0], shape[2]],
            &pollster::block_on(job.hidden()).unwrap(),
        );
        let db = snr(&got, &want);
        eprintln!("layer {i}: {db:.1} dB");
        assert!(db > 80.0, "layer {i}");
        prev = (shape, want);
    }
    job.set_hidden(&swap01(&prev.0, &prev.1));
    run(&job, &gpu, &[Step::FinalNorm]);
    let (shape, want) = f32s(&fx, "final_norm");
    let got = swap01(
        &[shape[1], shape[0], shape[2]],
        &pollster::block_on(job.normed()).unwrap(),
    );
    let db = snr(&got, &want);
    eprintln!("final norm: {db:.1} dB");
    assert!(db > 90.0);
    for s in 0..STEMS.len() {
        run(&job, &gpu, &job.mask_steps(s));
        let got = pollster::block_on(job.mask()).unwrap();
        let (_, want) = f32s(&fx, &format!("mask{s}"));
        let db = snr(&got, &want);
        eprintln!("mask {s} ({}): {db:.1} dB", STEMS[s]);
        assert!(db > 80.0);
    }

    if mm.is_some() && gpu.profiling() {
        pollster::block_on(gpu.collect_profile()).unwrap();
        let labels: Vec<String> = gpu.take_profile().into_keys().collect();
        let policy = gpu.policy.get();
        for t in [policy.pick(1000, 1000), policy.pick(10, 10)] {
            let name = TILES[t].name;
            assert!(
                labels.iter().any(|l| l.ends_with(&format!(" {name}"))),
                "no product ran {name}: {labels:#?}"
            );
        }
    }

    if !full {
        return eprintln!("end to end skipped (set ROFORMER_GPU_FULL=1)");
    }

    // End to end with our STFT: errors relative to the mix, as in the CPU
    // port's test (the guitar is nearly silent in this clip).
    let (ishape, input) = f32s(&fx, "input");
    let n = ishape[1];
    let (spec, feats) = analyse(&[input[..n].to_vec(), input[n..].to_vec()]);
    let t0 = std::time::Instant::now();
    let masks = pollster::block_on(job.masks(&feats)).unwrap();
    eprintln!("whole network: {:.1} s", t0.elapsed().as_secs_f64());
    let (oshape, want) = f32s(&fx, "output");
    let len = oshape[2];
    let mix_energy: f64 =
        input.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() * len as f64 / n as f64;
    for (s, m) in masks.iter().enumerate() {
        let audio = apply_masks(&spec, m);
        let mut err = 0.0f64;
        for c in 0..2 {
            let w = &want[(s * 2 + c) * len..(s * 2 + c + 1) * len];
            err += audio[c]
                .iter()
                .zip(w)
                .map(|(a, b)| (*a as f64 - *b as f64).powi(2))
                .sum::<f64>();
        }
        let vs_mix = 10.0 * (mix_energy / err).log10();
        eprintln!("output {}: {vs_mix:.1} dB vs mix", STEMS[s]);
        assert!(vs_mix > 70.0, "{}", STEMS[s]);
    }
}
