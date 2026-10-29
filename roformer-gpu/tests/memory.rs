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
use roformer_gpu::net::{Job, Net, Step};
use roformer_gpu::{Gpu, WeightFormat, weight_format};
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

/// The header of a safetensors file of 16-bit values (`dtype` `BF16` or
/// `F16`), and the size of the whole file.
fn header(tensors: &[(String, Vec<usize>)], dtype: &str) -> (Vec<u8>, usize) {
    let mut entries = Vec::new();
    let mut end = 0;
    for (name, shape) in tensors {
        let start = end;
        end += 2 * shape.iter().product::<usize>();
        let dims: Vec<String> = shape.iter().map(|d| d.to_string()).collect();
        entries.push(format!(
            "\"{name}\":{{\"dtype\":\"{dtype}\",\"shape\":[{}],\"data_offsets\":[{start},{end}]}}",
            dims.join(",")
        ));
    }
    let json = format!("{{{}}}", entries.join(","));
    let mut head = (json.len() as u64).to_le_bytes().to_vec();
    head.extend_from_slice(json.as_bytes());
    let size = head.len() + end;
    (head, size)
}

/// A safetensors file of 16-bit zeros: the header is built, the data is
/// produced on each read, so the ~350 MB file is never in memory.
struct Zeros {
    head: Vec<u8>,
    size: usize,
}

impl Zeros {
    fn new(tensors: &[(String, Vec<usize>)]) -> Self {
        Self::of(tensors, "BF16")
    }

    fn of(tensors: &[(String, Vec<usize>)], dtype: &str) -> Self {
        let (head, size) = header(tensors, dtype);
        Self { head, size }
    }
}

/// A weight matrix is the `weight` of a linear layer; norms' gains and
/// biases are vectors.
fn is_matrix(name: &str) -> bool {
    name.ends_with(".weight")
}

/// Bytes that the f32 load of [`tensors`] puts on the GPU: (matrices,
/// vectors).  The band-split gains stay on the CPU.
fn f32_bytes() -> (u64, u64) {
    let mut out = (0, 0);
    for (name, shape) in tensors() {
        let bytes = 4 * shape.iter().product::<usize>() as u64;
        if is_matrix(&name) {
            out.0 += bytes;
        } else if !(name.starts_with("band_split") && name.ends_with(".0.gamma")) {
            out.1 += bytes;
        }
    }
    out
}

/// A safetensors file of every tensor the network reads, F16, with values
/// made from a hash of their place in the file on each read.  A weight
/// matrix holds values below 1 / sqrt(fan-in) in magnitude, so the mask
/// estimators' `tanh` does not saturate, and about one in 16 of them is an
/// f16 subnormal.  A gain is near 1, a bias near 0.
struct Random {
    head: Vec<u8>,
    size: usize,
    /// Start of each tensor's data in the file: its scale and kind.
    tensors: std::collections::HashMap<usize, (usize, f32, Kind)>,
}

#[derive(Clone, Copy)]
enum Kind {
    Matrix,
    Gain,
    Bias,
}

impl Random {
    fn new(tensors: &[(String, Vec<usize>)]) -> Self {
        let (head, size) = header(tensors, "F16");
        let mut at = head.len();
        let mut map = std::collections::HashMap::new();
        for (name, shape) in tensors {
            let len = shape.iter().product::<usize>();
            let kind = if is_matrix(name) {
                Kind::Matrix
            } else if name.ends_with("gamma") {
                Kind::Gain
            } else {
                Kind::Bias
            };
            let scale = 1.0 / (*shape.last().unwrap() as f32).sqrt();
            map.insert(at, (len, scale, kind));
            at += 2 * len;
        }
        Self {
            head,
            size,
            tensors: map,
        }
    }

    fn value(index: usize, scale: f32, kind: Kind) -> u16 {
        let mut z = (index as u64).wrapping_add(0x9e37_79b9_7f4a_7c15);
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        let u = (z >> 40) as f32 / (1u64 << 23) as f32 - 1.0;
        match kind {
            Kind::Matrix if z.is_multiple_of(16) => {
                ((z >> 16) as u16 & 0x8000) | ((z >> 20) as u16 & 0x3ff).max(1)
            }
            Kind::Matrix => half::f16::from_f32(u * scale).to_bits(),
            Kind::Gain => half::f16::from_f32(1.0 + 0.1 * u).to_bits(),
            Kind::Bias => half::f16::from_f32(0.1 * u).to_bits(),
        }
    }
}

impl Source for Random {
    fn size(&self) -> usize {
        self.size
    }

    fn read(&self, start: usize, end: usize) -> Result<Cow<'_, [u8]>, String> {
        if end <= self.head.len() {
            return Ok(Cow::Borrowed(&self.head[start..end]));
        }
        let &(len, scale, kind) = self
            .tensors
            .get(&start)
            .expect("a read of one whole tensor");
        assert_eq!(end - start, 2 * len);
        let first = (start - self.head.len()) / 2;
        let out = (first..first + len)
            .flat_map(|i| Self::value(i, scale, kind).to_le_bytes())
            .collect();
        Ok(Cow::Owned(out))
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
    // A BF16 file goes up as f32 whatever the format asked for.
    let net = Net::load_store(&g, store, 400_000, WeightFormat::F16).unwrap();
    assert_eq!(net.format, WeightFormat::F32);
    let with_net = g.live_bytes();
    // Every tensor goes to the GPU as f32 except the band-split gains,
    // which are applied on the CPU; every stacked tensor is a whole number
    // of vec4s, so nothing is padded.
    let (matrices, vectors) = f32_bytes();
    assert_eq!(net.bytes, matrices + vectors);
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

/// An F16 file loaded as f16 holds its weight matrices at half the f32
/// size, and its vectors as f32; loaded as f32, it holds all as f32.  A
/// dropped f16 net frees every buffer.
#[test]
fn an_f16_net_holds_its_matrices_at_half_size() {
    let _model = MODEL.lock().unwrap_or_else(|e| e.into_inner());
    let Some(g) = gpu() else { return };
    if !g.has_f16_unpack() {
        return skip(Need::F16, "adapter without SHADER_F16_IN_F32");
    }
    let start = g.live_bytes();
    let (matrices, vectors) = f32_bytes();
    let load = |format| {
        let store = Store::from_source(Zeros::of(&tensors(), "F16")).unwrap();
        Net::load_store(&g, store, 400_000, format).unwrap()
    };
    let net = load(WeightFormat::F32);
    assert_eq!(net.format, WeightFormat::F32);
    assert_eq!(net.bytes, matrices + vectors);
    drop(net);
    assert_eq!(g.live_bytes(), start);
    let net = load(WeightFormat::F16);
    assert_eq!(net.format, WeightFormat::F16);
    assert_eq!(net.bytes, matrices / 2 + vectors);
    assert_eq!(g.live_bytes() - start, net.bytes);
    eprintln!(
        "parameters: f32 {} bytes, f16 {} bytes",
        matrices + vectors,
        net.bytes
    );
    drop(net);
    assert_eq!(g.live_bytes(), start);
    pollster::block_on(g.done());
    let errors = g.take_errors();
    assert!(errors.is_empty(), "{errors:?}");
}

/// A net built from an F16 file with f16 weights gives every mask of a
/// small chunk bit for bit as the same net with f32 weights, f16
/// subnormals among the weights.
#[test]
fn net_f16_weights_match_f32() {
    let _model = MODEL.lock().unwrap_or_else(|e| e.into_inner());
    let Some(g) = gpu() else { return };
    if !g.has_f16_unpack() {
        return skip(Need::F16, "adapter without SHADER_F16_IN_F32");
    }
    let frames = 9;
    let features: Vec<f32> = (0..frames * FEATURES)
        .map(|i| ((i * 7919) % 2001) as f32 / 1000.0 - 1.0)
        .collect();
    let masks = |format| {
        let store = Store::from_source(Random::new(&tensors())).unwrap();
        let net = Net::load_store(&g, store, 400_000, format).unwrap();
        assert_eq!(net.format, format);
        let job = Job::new(&g, &net, frames);
        let masks = pollster::block_on(job.masks(&features)).unwrap();
        let errors = g.take_errors();
        assert!(errors.is_empty(), "{errors:?}");
        masks
    };
    let want = masks(WeightFormat::F32);
    let got = masks(WeightFormat::F16);
    assert_eq!(want.len(), STEMS.len());
    for (stem, (g16, g32)) in got.iter().zip(&want).enumerate() {
        assert!(g32.iter().all(|v| v.is_finite()), "stem {stem}");
        let saturated = g32.iter().filter(|v| v.abs() == 1.0).count();
        assert!(
            saturated * 2 < g32.len(),
            "stem {stem}: {saturated} masks at +-1"
        );
        let (lo, hi) = g32
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
        assert!(hi - lo > 1e-3, "stem {stem}: masks from {lo} to {hi}");
        let d = roformer_gpu::bit_differences(g16, g32);
        assert_eq!(
            d.count, 0,
            "stem {stem}: {} mask values differ, the largest by {:e}",
            d.count, d.largest
        );
    }
}

/// On an adapter without `SHADER_F16_IN_F32` the f16 variants never
/// compile, the f16 check does not pass, and an f16 setting loads an F16
/// file as f32.
#[test]
fn without_the_f16_flag_weights_stay_f32() {
    let _model = MODEL.lock().unwrap_or_else(|e| e.into_inner());
    let g = match pollster::block_on(Gpu::new_without_f16_unpack(
        wgpu::PowerPreference::HighPerformance,
    )) {
        Ok(g) => g,
        Err(e) => return skip(Need::Gpu, &e),
    };
    assert!(!g.has_f16_unpack());
    let problems = pollster::block_on(g.check_kernels()).unwrap();
    assert!(problems.is_empty(), "{problems:?}");
    assert!(!g.f16_ok.get());
    let (format, note) = weight_format(Some("f16"), g.downlevel_flags(), g.f16_ok.get());
    assert_eq!(format, WeightFormat::F32);
    assert!(note.is_some());
    let store = Store::from_source(Zeros::of(&tensors(), "F16")).unwrap();
    // Even asked for f16, the net goes up as f32 on this adapter.
    let net = Net::load_store(&g, store, 400_000, WeightFormat::F16).unwrap();
    assert_eq!(net.format, WeightFormat::F32);
    let (matrices, vectors) = f32_bytes();
    assert_eq!(net.bytes, matrices + vectors);
    let frames = 9;
    let job = Job::new(&g, &net, frames);
    pollster::block_on(job.masks(&vec![0.5; frames * FEATURES])).unwrap();
    drop(job);
    assert_eq!(g.compiled_matmul_kernels(WeightFormat::F16), 0);
    assert!(g.compiled_matmul_kernels(WeightFormat::F32) > 0);
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
    match Net::load_store(&g, store, 400_000, WeightFormat::F32) {
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

/// An f16 weight upload is counted at its packed size, rounded up to 16
/// bytes, and freed when dropped; an f32 weight upload too.
#[test]
fn a_dropped_weight_is_freed() {
    let Some(g) = gpu() else { return };
    let start = g.live_bytes();
    let w = g.upload_weight("f32 weight", &[1.0; 10]);
    assert_eq!(g.live_bytes() - start, 48);
    drop(w);
    assert_eq!(g.live_bytes(), start);
    if !g.has_f16_unpack() {
        return skip(Need::F16, "adapter without SHADER_F16_IN_F32");
    }
    let created = g.created_bytes();
    let odd = g.upload_f16("f16 weight", &[0x3c00; 21]);
    assert_eq!(odd.size(), 48);
    assert_eq!(g.live_bytes() - start, 48);
    assert_eq!(g.created_bytes() - created, 48);
    let whole = g.upload_f16("f16 weight", &[0x3c00; 64]);
    assert_eq!(g.live_bytes() - start, 48 + 128);
    drop(odd);
    drop(whole);
    assert_eq!(g.live_bytes(), start);
    pollster::block_on(g.done());
    let errors = g.take_errors();
    assert!(errors.is_empty(), "{errors:?}");
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
