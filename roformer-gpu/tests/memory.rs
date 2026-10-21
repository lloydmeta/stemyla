//! GPU buffers are destroyed as soon as they are dropped, on every path:
//! [`Gpu::live_bytes`] comes back to where it was after a model, a chunk's
//! buffers, a failed upload, readbacks, start-up checks and benchmarks.
//! The model is a synthetic file with every tensor the network reads, all
//! zero, served a tensor at a time so the test never holds it whole.
//! Tests skip with a message when no adapter is available, and fail
//! instead under `STEMYLA_REQUIRE=gpu` (or `timestamps`, for the profile
//! readback test).

#![expect(
    clippy::disallowed_methods,
    reason = "MEM-1: test code, which does not ship"
)]

use std::borrow::Cow;
use std::sync::Mutex;

use roformer::weights::{Source, Store, WeightError};
use roformer::{DEPTH, DIM, DIM_HEAD, FEATURES, HEADS, HIDDEN, STEMS, band_groups};
use roformer_gpu::Gpu;
use roformer_gpu::net::{Job, Net, Step};
use test_support::{Need, skip};

/// Held by the tests that upload a whole model, so at most one ~700 MB
/// model is on the GPU at a time.
static MODEL: Mutex<()> = Mutex::new(());

#[track_caller]
fn gpu() -> Option<Gpu> {
    match pollster::block_on(Gpu::new(wgpu::PowerPreference::HighPerformance)) {
        Ok(g) => {
            eprintln!("adapter: {} ({:?})", g.info.name, g.info.backend);
            Some(g)
        }
        Err(e) => {
            skip(Need::Gpu, &e);
            None
        }
    }
}

/// Every tensor `Net::load_store` reads, with its shape, in the order it
/// reads them.  A name or shape that drifts from net.rs fails
/// `a_dropped_net_and_job_free_their_buffers` (a missing or unused tensor).
fn tensors() -> Vec<(String, Vec<usize>)> {
    let inner = HEADS * DIM_HEAD;
    let mut out = Vec::new();
    let groups = band_groups();
    for g in &groups {
        let bands = g.first..g.first + g.count;
        for (suffix, shape) in [
            ("0.gamma", vec![g.width]),
            ("1.weight", vec![DIM, g.width]),
            ("1.bias", vec![DIM]),
        ] {
            for k in bands.clone() {
                out.push((
                    format!("band_split.to_features.{k}.{suffix}"),
                    shape.clone(),
                ));
            }
        }
    }
    for i in 0..DEPTH {
        for axis in 0..2 {
            let a = format!("layers.{i}.{axis}.layers.0.0");
            let f = format!("layers.{i}.{axis}.layers.0.1");
            out.extend([
                (format!("{a}.norm.gamma"), vec![DIM]),
                (format!("{a}.to_qkv.weight"), vec![3 * inner, DIM]),
                (format!("{a}.to_gates.weight"), vec![HEADS, DIM]),
                (format!("{a}.to_gates.bias"), vec![HEADS]),
                (format!("{a}.to_out.0.weight"), vec![DIM, inner]),
                (format!("{f}.net.0.gamma"), vec![DIM]),
                (format!("{f}.net.1.weight"), vec![HIDDEN, DIM]),
                (format!("{f}.net.1.bias"), vec![HIDDEN]),
                (format!("{f}.net.4.weight"), vec![DIM, HIDDEN]),
                (format!("{f}.net.4.bias"), vec![DIM]),
            ]);
        }
    }
    out.push(("final_norm.gamma".into(), vec![DIM]));
    for stem in 0..STEMS.len() {
        for g in &groups {
            for (suffix, shape) in [
                ("0.weight", vec![HIDDEN, DIM]),
                ("0.bias", vec![HIDDEN]),
                ("2.weight", vec![2 * g.width, HIDDEN]),
                ("2.bias", vec![2 * g.width]),
            ] {
                for k in g.first..g.first + g.count {
                    out.push((
                        format!("mask_estimators.{stem}.to_freqs.{k}.0.{suffix}"),
                        shape.clone(),
                    ));
                }
            }
        }
    }
    out
}

/// A safetensors file of bf16 zeros: the header is built, the data is
/// produced on each read, so the ~350 MB file is never in memory.
struct Zeros {
    head: Vec<u8>,
    size: usize,
}

impl Zeros {
    fn new(tensors: &[(String, Vec<usize>)]) -> Self {
        let mut entries = Vec::new();
        let mut end = 0;
        for (name, shape) in tensors {
            let start = end;
            end += 2 * shape.iter().product::<usize>();
            let dims: Vec<String> = shape.iter().map(|d| d.to_string()).collect();
            entries.push(format!(
                "\"{name}\":{{\"dtype\":\"BF16\",\"shape\":[{}],\"data_offsets\":[{start},{end}]}}",
                dims.join(",")
            ));
        }
        let json = format!("{{{}}}", entries.join(","));
        let mut head = (json.len() as u64).to_le_bytes().to_vec();
        head.extend_from_slice(json.as_bytes());
        let size = head.len() + end;
        Self { head, size }
    }
}

impl Source for Zeros {
    fn size(&self) -> usize {
        self.size
    }

    fn read(&self, start: usize, end: usize) -> Result<Cow<'_, [u8]>, String> {
        let mut out = vec![0u8; end - start];
        let (from, to) = (start.min(self.head.len()), end.min(self.head.len()));
        out[..to - from].copy_from_slice(&self.head[from..to]);
        Ok(Cow::Owned(out))
    }
}

/// A loaded model, a chunk's buffers, its forward pass and its six mask
/// readbacks: live bytes grow by exactly what each holds, stay flat across
/// the readbacks, and fall back as each is dropped.
#[test]
fn a_dropped_net_and_job_free_their_buffers() {
    let _model = MODEL.lock().unwrap_or_else(|e| e.into_inner());
    let Some(g) = gpu() else { return };
    let start = g.live_bytes();
    let store = Store::from_source(Zeros::new(&tensors())).unwrap();
    let net = Net::load_store(&g, store, 400_000).unwrap();
    let with_net = g.live_bytes();
    // Every tensor goes to the GPU as f32 except the band-split gains,
    // which are applied on the CPU; every stacked tensor is a whole number
    // of vec4s, so nothing is padded.
    let uploaded: usize = tensors()
        .iter()
        .filter(|(name, _)| !(name.starts_with("band_split") && name.ends_with(".0.gamma")))
        .map(|(_, shape)| 4 * shape.iter().product::<usize>())
        .sum();
    assert_eq!(net.bytes, uploaded as u64);
    assert_eq!(with_net - start, net.bytes);

    let frames = 9;
    let job = Job::new(&g, &net, frames);
    let with_job = g.live_bytes();
    assert_eq!(with_job - with_net, job.bytes());
    job.set_input(&vec![0.5; frames * FEATURES]);
    let mut after_reads = Vec::new();
    for step in job.steps() {
        job.run(&step);
        pollster::block_on(g.done());
        if let Step::Mask { last: true, .. } = step {
            let mask = pollster::block_on(job.mask()).unwrap();
            assert_eq!(mask.len(), frames * FEATURES);
            after_reads.push(g.live_bytes());
        }
    }
    assert_eq!(after_reads, vec![with_job; STEMS.len()]);
    let errors = g.take_errors();
    assert!(errors.is_empty(), "{errors:?}");

    drop(job);
    assert_eq!(g.live_bytes(), with_net);
    drop(net);
    assert_eq!(g.live_bytes(), start);
    pollster::block_on(g.done());
    let errors = g.take_errors();
    assert!(errors.is_empty(), "{errors:?}");
}

/// A file missing the last tensor the network reads fails after nearly
/// all of the model is on the GPU; what was uploaded is destroyed.
#[test]
fn a_failed_load_frees_what_it_uploaded() {
    let _model = MODEL.lock().unwrap_or_else(|e| e.into_inner());
    let Some(g) = gpu() else { return };
    let start = g.live_bytes();
    let mut all = tensors();
    let (last, _) = all.pop().unwrap();
    let store = Store::from_source(Zeros::new(&all)).unwrap();
    let created = g.created_bytes();
    match Net::load_store(&g, store, 400_000) {
        Err(WeightError::Missing(name)) => assert_eq!(name, last),
        Err(e) => panic!("{e}"),
        Ok(_) => panic!("a model missing {last} loaded"),
    }
    assert!(g.created_bytes() - created > 600_000_000);
    assert_eq!(g.live_bytes(), start);
    pollster::block_on(g.done());
    let errors = g.take_errors();
    assert!(errors.is_empty(), "{errors:?}");
}

/// Dropping a buffer destroys it: another handle to the same buffer can
/// no longer be read.
#[test]
fn a_dropped_buffer_is_destroyed() {
    let Some(g) = gpu() else { return };
    let buf = g.upload("data", &[1.0, 2.0, 3.0, 4.0]);
    let handle: wgpu::Buffer = (*buf).clone();
    assert_eq!(
        pollster::block_on(g.read(&handle, 0, 4)).unwrap(),
        [1.0, 2.0, 3.0, 4.0]
    );
    assert!(g.take_errors().is_empty());
    drop(buf);
    let read = pollster::block_on(g.read(&handle, 0, 4));
    let errors = g.take_errors();
    assert!(
        errors
            .iter()
            .any(|e| e.to_lowercase().contains("destroyed")),
        "read gave {read:?}, errors {errors:?}"
    );
}

/// Readbacks, the start-up kernel checks and the attention timings that
/// calibration runs leave no buffer behind.
#[test]
fn readbacks_checks_and_benchmarks_free_their_buffers() {
    let Some(g) = gpu() else { return };
    let start = g.live_bytes();
    let data: Vec<f32> = (0..1000).map(|i| i as f32).collect();
    let buf = g.upload("data", &data);
    let with_buf = g.live_bytes();
    assert_eq!(with_buf - start, 4000);
    for offset in [0, 10, 500] {
        let got = pollster::block_on(g.read(&buf, offset, 100)).unwrap();
        assert_eq!(got, data[offset..offset + 100]);
        assert_eq!(g.live_bytes(), with_buf);
    }
    drop(buf);
    assert_eq!(g.live_bytes(), start);

    pollster::block_on(g.check_kernels()).unwrap();
    pollster::block_on(g.check_attention());
    assert_eq!(g.live_bytes(), start);

    let t0 = std::time::Instant::now();
    let now = || t0.elapsed().as_secs_f64();
    pollster::block_on(roformer_gpu::bench::attention_per_chunk(
        &g, 0.0, 0.0, 1, false, &now,
    ));
    assert_eq!(g.live_bytes(), start);
    let errors = g.take_errors();
    assert!(errors.is_empty(), "{errors:?}");
}

/// On a profiling device each submission's timestamp readback buffer is
/// destroyed once its timestamps are collected.
#[test]
fn collected_profile_readbacks_are_freed() {
    let g = match pollster::block_on(Gpu::new_profiling(wgpu::PowerPreference::HighPerformance)) {
        Ok(g) if g.profiling() => g,
        Ok(_) => return skip(Need::Timestamps, "adapter without timestamp-query"),
        Err(e) => return skip(Need::Gpu, &e),
    };
    let x = g.upload("x", &[1.0; 256]);
    let start = g.live_bytes();
    for _ in 0..3 {
        let mut r = g.rec();
        r.softmax(&x, 4, 64);
        r.submit();
    }
    assert!(g.live_bytes() > start);
    pollster::block_on(g.done());
    pollster::block_on(g.collect_profile()).unwrap();
    assert_eq!(g.live_bytes(), start);
    assert_eq!(g.take_profile().get("softmax").map(|v| v.0), Some(3));
    let errors = g.take_errors();
    assert!(errors.is_empty(), "{errors:?}");
}
