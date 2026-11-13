//! The BS-RoFormer network on [`Gpu`] kernels.
//!
//! Same layouts as `roformer::model` (the CPU reference): input features
//! `[T, FEATURES]`, hidden state `[BANDS, T, DIM]` (transposed to
//! `[T, BANDS, DIM]` for the frequency transformers), one mask
//! `[T, FEATURES]` per stem.  Weights stay in torch's `[out, in]` layout and
//! are read transposed by the matmul kernel.
//!
//! A [`Job`] owns one chunk's buffers and splits the forward pass into
//! [`Step`]s, each a single short command buffer: one sub-batch of one
//! transformer, one transpose, or one group of mask-estimator bands.  The
//! caller submits a step, waits for the GPU ([`Gpu::done`]), and goes on.

use roformer::weights::{Dtype, Store, WeightError};
use roformer::{
    BandGroup, DEPTH, DIM, DIM_HEAD, FEATURES, FREQS_PER_BANDS, HEADS, HIDDEN, STEMS, band_groups,
    widest_band,
};

use crate::{
    Act, AttentionPolicy, FusedKernel, Gpu, GpuBuffer, MatMul, Rec, View, Weight, WeightFormat,
};

const INNER: usize = HEADS * DIM_HEAD;
const BANDS: usize = FREQS_PER_BANDS.len();

/// One `Transformer(depth=1)`: gated attention then feed-forward.
struct Block {
    attn_norm: GpuBuffer,
    qkv: Weight,        // [1536, 256]
    gates_w: Weight,    // [8, 256]
    gates_b: GpuBuffer, // [8]
    out: Weight,        // [256, 512]
    ff_norm: GpuBuffer,
    w1: Weight, // [1024, 256]
    b1: GpuBuffer,
    w2: Weight, // [256, 1024]
    b2: GpuBuffer,
}

struct SplitGroup {
    g: BandGroup,
    /// Per band RMSNorm gain times sqrt(width), applied on the CPU.
    gamma: Vec<f32>, // [count, width]
    w: Weight,    // [count, 256, width]
    b: GpuBuffer, // [count, 256]
}

struct MaskGroup {
    g: BandGroup,
    w1: Weight,    // [count, 1024, 256]
    b1: GpuBuffer, // [count, 1024]
    w2: Weight,    // [count, 2w, 1024]
    b2: GpuBuffer, // [count, 2w]
}

pub struct Net {
    split: Vec<SplitGroup>,
    layers: Vec<(Block, Block)>,
    final_norm: GpuBuffer,
    masks: Vec<Vec<MaskGroup>>,
    /// Upper bound on the elements of any per-step intermediate (attention
    /// scores, q/k/v, hidden layers); at least one sequence always fits.
    pub budget: usize,
    /// Bytes of GPU buffers holding the parameters.
    pub bytes: u64,
    /// How the weight matrices went up: `F16` only when every one of them
    /// did (an F32 or BF16 file gives `F32` whatever was asked for).
    pub format: WeightFormat,
}

fn norm(gpu: &Gpu, s: &mut Store, name: &str, dim: usize) -> Result<GpuBuffer, WeightError> {
    let scale = (dim as f32).sqrt();
    let g: Vec<f32> = s
        .get(name, &[dim])?
        .into_iter()
        .map(|v| v * scale)
        .collect();
    Ok(gpu.upload(name, &g))
}

fn tensor(gpu: &Gpu, s: &mut Store, name: &str, shape: &[usize]) -> Result<GpuBuffer, WeightError> {
    Ok(gpu.upload(name, &s.get(name, shape)?))
}

fn stacked(
    gpu: &Gpu,
    s: &mut Store,
    names: &[String],
    shape: &[usize],
    scale: f32,
) -> Result<GpuBuffer, WeightError> {
    Ok(gpu.upload(&names[0], &stacked_cpu(s, names, shape, scale)?))
}

/// Uploads weight matrices in the format a load asked for, and notes
/// whether every one of them went up in it.
struct Weights<'g> {
    gpu: &'g Gpu,
    format: WeightFormat,
    all_f16: bool,
}

impl Weights<'_> {
    /// One weight matrix.
    fn one(&mut self, s: &mut Store, name: &str, shape: &[usize]) -> Result<Weight, WeightError> {
        self.stack(s, &[name.to_string()], shape)
    }

    /// The matrices `names` as one buffer, one after another: as f16 when
    /// the load asked for f16 and every one of them is F16 in the file
    /// (decision 0018), else as f32.  Each tensor is read once.
    fn stack(
        &mut self,
        s: &mut Store,
        names: &[String],
        shape: &[usize],
    ) -> Result<Weight, WeightError> {
        let f16 = self.format == WeightFormat::F16
            && names.iter().all(|n| s.dtype(n) == Some(Dtype::F16));
        if !f16 {
            self.all_f16 = false;
            return Ok(self
                .gpu
                .upload_weight(&names[0], &stacked_cpu(s, names, shape, 1.0)?));
        }
        let mut bits = s.get_f16_bits(&names[0], shape)?;
        bits.reserve(bits.len() * (names.len() - 1));
        for n in &names[1..] {
            bits.extend(s.get_f16_bits(n, shape)?);
        }
        Ok(self.gpu.upload_f16(&names[0], &bits))
    }
}

fn stacked_cpu(
    s: &mut Store,
    names: &[String],
    shape: &[usize],
    scale: f32,
) -> Result<Vec<f32>, WeightError> {
    let mut all = Vec::with_capacity(names.len() * shape.iter().product::<usize>());
    for n in names {
        all.extend(s.get(n, shape)?.into_iter().map(|v| v * scale));
    }
    Ok(all)
}

impl Block {
    fn load(w: &mut Weights, s: &mut Store, p: &str) -> Result<Self, WeightError> {
        let gpu = w.gpu;
        let a = format!("{p}.layers.0.0");
        let f = format!("{p}.layers.0.1");
        Ok(Self {
            attn_norm: norm(gpu, s, &format!("{a}.norm.gamma"), DIM)?,
            qkv: w.one(s, &format!("{a}.to_qkv.weight"), &[3 * INNER, DIM])?,
            gates_w: w.one(s, &format!("{a}.to_gates.weight"), &[HEADS, DIM])?,
            gates_b: tensor(gpu, s, &format!("{a}.to_gates.bias"), &[HEADS])?,
            out: w.one(s, &format!("{a}.to_out.0.weight"), &[DIM, INNER])?,
            ff_norm: norm(gpu, s, &format!("{f}.net.0.gamma"), DIM)?,
            w1: w.one(s, &format!("{f}.net.1.weight"), &[HIDDEN, DIM])?,
            b1: tensor(gpu, s, &format!("{f}.net.1.bias"), &[HIDDEN])?,
            w2: w.one(s, &format!("{f}.net.4.weight"), &[DIM, HIDDEN])?,
            b2: tensor(gpu, s, &format!("{f}.net.4.bias"), &[DIM])?,
        })
    }
}

/// Elements one sequence of `seq` tokens needs in the largest per-step
/// buffer of a transformer step.  Fused attention never writes the
/// `[HEADS, seq, seq]` scores, so a step can then take more sequences
/// (4 bands of 1,151 frames instead of 1 at the default budget): more
/// workgroups per dispatch and fewer steps.
fn per_seq(seq: usize, fused: bool) -> usize {
    let scores = if fused { 0 } else { HEADS * seq * seq };
    (seq * 3 * INNER).max(scores).max(seq * HIDDEN)
}

/// The default upper bound on any per-step intermediate, in f32 elements
/// (32 MiB): one band for each split time-attention step at full chunk
/// length (about 4 GFLOP), so each GPU step stays short.  The worker's
/// `?max_elems=` overrides it.  Calibration times the time transformer at
/// the budget of the run.
pub const BUDGET: usize = 8 << 20;

/// The bands of each time-transformer step of one layer, in order, for a
/// chunk of `frames` frames at `budget`, with fused attention or without
/// ([`Job::layer_steps`]).  At 1,151 frames and [`BUDGET`]: 62 steps of 1
/// band without, and 15 steps of 4 bands and 1 of 2 with.
pub fn time_steps(frames: usize, budget: usize, fused: bool) -> Vec<usize> {
    let n = (budget / per_seq(frames, fused)).clamp(1, BANDS);
    let mut out = Vec::new();
    let mut first = 0;
    while first < BANDS {
        out.push(n.min(BANDS - first));
        first += n;
    }
    out
}

impl Net {
    /// Uploads the weights of an exported safetensors file (f32, f16 or
    /// bf16).  Every tensor must be used and have the expected shape.
    /// `format` is how the weight matrices of an F16 file go up; see
    /// [`Self::load_store`].
    pub fn load(
        gpu: &Gpu,
        bytes: &[u8],
        budget: usize,
        format: WeightFormat,
    ) -> Result<Self, WeightError> {
        Self::load_store(gpu, Store::new(bytes)?, budget, format)
    }

    /// [`Self::load`] from a store, which may read the file from outside
    /// WebAssembly memory one tensor at a time.  With `F16`, each weight
    /// matrix (or stack of them) that is F16 in the file goes up as f16
    /// values; the rest, and every norm, bias and gain, go up as f32.  On an
    /// adapter without [`Gpu::has_f16_unpack`], or before or after a failed
    /// f16 start-up check ([`Gpu::f16_ok`] false), everything goes up as
    /// f32.
    /// [`Net::format`] records what went up.
    pub fn load_store(
        gpu: &Gpu,
        mut store: Store,
        budget: usize,
        format: WeightFormat,
    ) -> Result<Self, WeightError> {
        let before = gpu.created_bytes();
        let format = if gpu.has_f16_unpack() && gpu.f16_ok.get() {
            format
        } else {
            WeightFormat::F32
        };
        let mut w = Weights {
            gpu,
            format,
            all_f16: true,
        };
        let s = &mut store;
        let groups = band_groups();
        let mut split = Vec::new();
        for g in &groups {
            let names = |suffix: &str| -> Vec<String> {
                (g.first..g.first + g.count)
                    .map(|k| format!("band_split.to_features.{k}.{suffix}"))
                    .collect()
            };
            split.push(SplitGroup {
                g: *g,
                gamma: stacked_cpu(s, &names("0.gamma"), &[g.width], (g.width as f32).sqrt())?,
                w: w.stack(s, &names("1.weight"), &[DIM, g.width])?,
                b: stacked(gpu, s, &names("1.bias"), &[DIM], 1.0)?,
            });
        }
        let mut layers = Vec::with_capacity(DEPTH);
        for i in 0..DEPTH {
            layers.push((
                Block::load(&mut w, s, &format!("layers.{i}.0"))?,
                Block::load(&mut w, s, &format!("layers.{i}.1"))?,
            ));
        }
        let final_norm = norm(gpu, s, "final_norm.gamma", DIM)?;
        let mut masks = Vec::with_capacity(STEMS.len());
        for stem in 0..STEMS.len() {
            let mut per_group = Vec::new();
            for g in &groups {
                let names = |suffix: &str| -> Vec<String> {
                    (g.first..g.first + g.count)
                        .map(|k| format!("mask_estimators.{stem}.to_freqs.{k}.0.{suffix}"))
                        .collect()
                };
                per_group.push(MaskGroup {
                    g: *g,
                    w1: w.stack(s, &names("0.weight"), &[HIDDEN, DIM])?,
                    b1: stacked(gpu, s, &names("0.bias"), &[HIDDEN], 1.0)?,
                    w2: w.stack(s, &names("2.weight"), &[2 * g.width, HIDDEN])?,
                    b2: stacked(gpu, s, &names("2.bias"), &[2 * g.width], 1.0)?,
                });
            }
            masks.push(per_group);
        }
        store.finish()?;
        Ok(Self {
            split,
            layers,
            final_norm,
            masks,
            budget,
            bytes: gpu.created_bytes() - before,
            format: if format == WeightFormat::F16 && w.all_f16 {
                WeightFormat::F16
            } else {
                WeightFormat::F32
            },
        })
    }

    /// Each band group of the band split with its gains (`[count, width]`,
    /// times the square root of the width), as [`normalise_bands`] takes
    /// them.
    pub fn split_gains(&self) -> impl Iterator<Item = (&BandGroup, &[f32])> {
        self.split.iter().map(|sg| (&sg.g, sg.gamma.as_slice()))
    }

    /// Sequences per transformer step for sequences of `seq` tokens, with
    /// fused attention or without.
    pub fn sub_batch(&self, batch: usize, seq: usize, fused: bool) -> usize {
        (self.budget / per_seq(seq, fused)).clamp(1, batch)
    }

    /// Bands per mask step for a band group of width `w`.
    fn mask_batch(&self, g: &BandGroup, frames: usize) -> usize {
        (self.budget / (frames * HIDDEN.max(2 * g.width))).clamp(1, g.count)
    }
}

/// One command buffer's worth of the forward pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Band-split projections into the hidden state.
    Split,
    /// Time transformer of `layer` on bands `first..first + n`.
    Time {
        layer: usize,
        first: usize,
        n: usize,
    },
    /// Hidden state to `[T, BANDS, DIM]`.
    ToFreq,
    /// Frequency transformer of `layer` on frames `first..first + n`.
    Freq {
        layer: usize,
        first: usize,
        n: usize,
    },
    /// Back to `[BANDS, T, DIM]`.
    ToTime,
    FinalNorm,
    /// Mask estimator of `stem` for bands `k..k + n` of band group `group`;
    /// `last` marks the stem's mask as complete.
    Mask {
        stem: usize,
        group: usize,
        k: usize,
        n: usize,
        last: bool,
    },
}

impl Step {
    /// Which kind of work the step is (the first step of each kind also
    /// pays for compiling its kernels).
    pub fn kind(&self) -> &'static str {
        match self {
            Step::Split => "band split",
            Step::Time { .. } => "time transformer",
            Step::ToFreq | Step::ToTime => "transpose",
            Step::Freq { .. } => "frequency transformer",
            Step::FinalNorm => "final norm",
            Step::Mask { .. } => "mask estimator",
        }
    }
}

impl Step {
    /// Floating-point operations of the step's matmuls (multiply and add
    /// counted separately) for a chunk of `frames` frames; the other
    /// kernels add little.  For throughput reports.
    pub fn flops(&self, frames: usize) -> f64 {
        let transformer = |n: usize, seq: usize| {
            let tokens = (n * seq) as f64;
            let per_token =
                2.0 * (DIM * 3 * INNER + DIM * HEADS + INNER * DIM + 2 * DIM * HIDDEN) as f64;
            let attention = 2.0 * 2.0 * (HEADS * seq * seq * DIM_HEAD) as f64 * n as f64;
            tokens * per_token + attention
        };
        match *self {
            Step::Split => 2.0 * (frames * FEATURES * DIM) as f64,
            Step::Time { n, .. } => transformer(n, frames),
            Step::Freq { n, .. } => transformer(n, BANDS),
            Step::Mask { group, n, .. } => {
                let w = band_groups()[group].width;
                2.0 * (frames * n * (DIM * HIDDEN + HIDDEN * 2 * w)) as f64
            }
            Step::ToFreq | Step::ToTime | Step::FinalNorm => 0.0,
        }
    }
}

impl std::fmt::Display for Step {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Step::Time { layer, first, n } => write!(
                f,
                "layer {layer} time transformer, bands {first}..{}",
                first + n
            ),
            Step::Freq { layer, first, n } => write!(
                f,
                "layer {layer} frequency transformer, frames {first}..{}",
                first + n
            ),
            Step::Mask {
                stem, group, k, n, ..
            } => {
                write!(
                    f,
                    "mask estimator {}, band group {group}, bands {k}..{}",
                    STEMS[stem],
                    k + n
                )
            }
            _ => f.write_str(self.kind()),
        }
    }
}

/// `[seq, 32]` cosines then `[seq, 32]` sines for interleaved rotary
/// embedding, computed as `rotary_embedding_torch` does (angle in f32).
fn rope_table(seq: usize) -> Vec<f32> {
    let mut cos = Vec::with_capacity(seq * 32);
    let mut sin = Vec::with_capacity(seq * 32);
    for p in 0..seq {
        for i in 0..DIM_HEAD / 2 {
            let freq = (1.0 / 10000f64.powf((2 * i) as f64 / DIM_HEAD as f64)) as f32;
            let angle = (p as f32 * freq) as f64;
            cos.push(angle.cos() as f32);
            sin.push(angle.sin() as f32);
        }
    }
    cos.extend(sin);
    cos
}

/// The band-split RMSNorm of band `j` of group `g`, for each frame of
/// `features` (`[T, FEATURES]`): the band's features of the frame,
/// divided by their norm and multiplied by `gamma` (the band's gains times
/// the square root of its width).  It writes the band's rows `[T, width]`
/// to `out`, which holds exactly `T x g.width` floats.
pub fn normalise_band(features: &[f32], g: &BandGroup, j: usize, gamma: &[f32], out: &mut [f32]) {
    let w = g.width;
    let t = features.len() / FEATURES;
    assert_eq!(features.len(), t * FEATURES);
    assert_eq!(gamma.len(), w);
    assert_eq!(out.len(), t * w);
    for f in 0..t {
        let src = &features[f * FEATURES + g.offset + j * w..][..w];
        let n = src.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-12);
        let dst = &mut out[f * w..][..w];
        for c in 0..w {
            dst[c] = src[c] / n * gamma[c];
        }
    }
}

/// The band-split RMSNorm of `features` (`[T, FEATURES]`), one band at a
/// time, in a buffer of the widest band.  `groups` gives each band group
/// with its gains (`[count, width]`).  `write(at, rows)` takes each band's
/// rows (`[T, width]`) and their offset in floats in the normalised input,
/// which is `[count, T, width]` for each group.  [`Job::set_input`]
/// writes them to the GPU.
pub fn normalise_bands<'g>(
    features: &[f32],
    groups: impl IntoIterator<Item = (&'g BandGroup, &'g [f32])>,
    mut write: impl FnMut(usize, &[f32]),
) {
    let t = features.len() / FEATURES;
    let mut band = vec![0.0f32; t * widest_band()];
    for (g, gamma) in groups {
        let w = g.width;
        for j in 0..g.count {
            let rows = &mut band[..t * w];
            normalise_band(features, g, j, &gamma[j * w..(j + 1) * w], rows);
            write(t * g.offset + j * t * w, rows);
        }
    }
}

/// Buffers and steps for one chunk of `frames` STFT frames.
pub struct Job<'a> {
    gpu: &'a Gpu,
    net: &'a Net,
    /// Where this job uses fused attention ([`Gpu::attention`] when the
    /// job was made).  Fixed per job: the step sizes and the scores buffer
    /// depend on it.
    attention: AttentionPolicy,
    pub frames: usize,
    /// Band-normalised input, per group `[count, T, width]`.
    split_in: GpuBuffer,
    /// Hidden state `[BANDS, T, DIM]`.
    x: GpuBuffer,
    /// `[T, BANDS, DIM]` during frequency transformers; after
    /// [`Step::FinalNorm`], the normalised `[BANDS, T, DIM]`.
    xt: GpuBuffer,
    xn: GpuBuffer,
    qkv: GpuBuffer,
    scores: GpuBuffer,
    o: GpuBuffer,
    gates: GpuBuffer,
    h: GpuBuffer,
    y: GpuBuffer,
    mask: GpuBuffer,
    rope_time: GpuBuffer,
    rope_freq: GpuBuffer,
}

impl<'a> Job<'a> {
    /// Allocates the buffers for `frames` frames.
    pub fn new(gpu: &'a Gpu, net: &'a Net, frames: usize) -> Self {
        let (t, f) = (frames, BANDS);
        let attention = gpu.attention.get();
        let nt = net.sub_batch(f, t, attention.time().is_some());
        let nf = net.sub_batch(t, f, attention.freq());
        let tokens = (nt * t).max(nf * f);
        // Scores only for an axis that runs split attention.
        let scores_t = if attention.time().is_some() {
            0
        } else {
            nt * HEADS * t * t
        };
        let scores_f = if attention.freq() {
            0
        } else {
            nf * HEADS * f * f
        };
        let scores = scores_t.max(scores_f);
        let (mut mask_h, mut mask_y) = (0, 0);
        for g in band_groups() {
            let n = net.mask_batch(&g, t);
            mask_h = mask_h.max(n * t * HIDDEN);
            mask_y = mask_y.max(n * t * 2 * g.width);
        }
        Self {
            gpu,
            net,
            attention,
            frames,
            split_in: gpu.buffer("split_in", t * FEATURES),
            x: gpu.buffer("x", f * t * DIM),
            xt: gpu.buffer("xt", f * t * DIM),
            xn: gpu.buffer("xn", tokens * DIM),
            qkv: gpu.buffer("qkv", tokens * 3 * INNER),
            scores: gpu.buffer("scores", scores),
            o: gpu.buffer("o", tokens * INNER),
            gates: gpu.buffer("gates", tokens * HEADS),
            h: gpu.buffer("h", (tokens * HIDDEN).max(mask_h)),
            y: gpu.buffer("y", mask_y),
            mask: gpu.buffer("mask", t * FEATURES),
            rope_time: gpu.upload("rope_time", &rope_table(t)),
            rope_freq: gpu.upload("rope_freq", &rope_table(f)),
        }
    }

    /// Bytes of GPU buffers this job holds.
    pub fn bytes(&self) -> u64 {
        [
            &self.split_in,
            &self.x,
            &self.xt,
            &self.xn,
            &self.qkv,
            &self.scores,
            &self.o,
            &self.gates,
            &self.h,
            &self.y,
            &self.mask,
            &self.rope_time,
            &self.rope_freq,
        ]
        .iter()
        .map(|b| b.size())
        .sum()
    }

    /// Uploads the network input `[T, FEATURES]`, applying the band-split
    /// RMSNorm on the CPU (each band normalised over its own width).  It
    /// normalises one band at a time into a buffer of the widest band, and
    /// writes it to the GPU at the band's offset, so it holds one band,
    /// not the whole input.
    pub fn set_input(&self, features: &[f32]) {
        assert_eq!(features.len(), self.frames * FEATURES);
        normalise_bands(features, self.net.split_gains(), |at, rows| {
            self.gpu.write(&self.split_in, at, rows);
        });
    }

    /// Replaces the hidden state `[BANDS, T, DIM]` (to run single layers).
    pub fn set_hidden(&self, x: &[f32]) {
        assert_eq!(x.len(), BANDS * self.frames * DIM);
        self.gpu.write(&self.x, 0, x);
    }

    /// Replaces the final-normalised hidden state (to run the mask
    /// estimators on their own).
    pub fn set_normed(&self, x: &[f32]) {
        assert_eq!(x.len(), BANDS * self.frames * DIM);
        self.gpu.write(&self.xt, 0, x);
    }

    /// The band-normalised input that [`Self::set_input`] wrote, per group
    /// `[count, T, width]`.
    pub async fn split_input(&self) -> Result<Vec<f32>, String> {
        self.gpu
            .read(&self.split_in, 0, self.frames * FEATURES)
            .await
    }

    pub async fn hidden(&self) -> Result<Vec<f32>, String> {
        self.gpu.read(&self.x, 0, BANDS * self.frames * DIM).await
    }

    /// The final-normalised hidden state, valid after [`Step::FinalNorm`].
    pub async fn normed(&self) -> Result<Vec<f32>, String> {
        self.gpu.read(&self.xt, 0, BANDS * self.frames * DIM).await
    }

    /// The mask `[T, FEATURES]` of the stem whose last mask step ran.
    pub async fn mask(&self) -> Result<Vec<f32>, String> {
        self.gpu.read(&self.mask, 0, self.frames * FEATURES).await
    }

    pub fn layer_steps(&self, layer: usize) -> Vec<Step> {
        let (t, f) = (self.frames, BANDS);
        let mut out = Vec::new();
        let mut first = 0;
        for n in time_steps(t, self.net.budget, self.attention.time().is_some()) {
            out.push(Step::Time { layer, first, n });
            first += n;
        }
        out.push(Step::ToFreq);
        let n = self.net.sub_batch(t, f, self.attention.freq());
        let mut first = 0;
        while first < t {
            out.push(Step::Freq {
                layer,
                first,
                n: n.min(t - first),
            });
            first += n;
        }
        out.push(Step::ToTime);
        out
    }

    pub fn mask_steps(&self, stem: usize) -> Vec<Step> {
        let mut out = Vec::new();
        for (group, g) in band_groups().iter().enumerate() {
            let n = self.net.mask_batch(g, self.frames);
            let mut k = 0;
            while k < g.count {
                out.push(Step::Mask {
                    stem,
                    group,
                    k,
                    n: n.min(g.count - k),
                    last: false,
                });
                k += n;
            }
        }
        if let Some(Step::Mask { last, .. }) = out.last_mut() {
            *last = true;
        }
        out
    }

    /// The steps that every stem's mask needs, in order: the band split,
    /// the layers and the final norm.
    pub fn shared_steps(&self) -> Vec<Step> {
        let mut out = vec![Step::Split];
        for i in 0..DEPTH {
            out.extend(self.layer_steps(i));
        }
        out.push(Step::FinalNorm);
        out
    }

    /// Every step of the forward pass, in order: [`Job::shared_steps`],
    /// then [`Job::mask_steps`] for each stem.
    pub fn steps(&self) -> Vec<Step> {
        let mut out = self.shared_steps();
        for s in 0..STEMS.len() {
            out.extend(self.mask_steps(s));
        }
        out
    }

    /// Records and submits one step.
    pub fn run(&self, step: &Step) {
        let mut r = self.gpu.rec();
        let (t, f) = (self.frames, BANDS);
        match *step {
            Step::Split => self.split(&mut r),
            Step::Time { layer, first, n } => transformer(
                &mut r,
                &self.step_bufs(),
                &self.net.layers[layer].0,
                &self.x,
                t,
                first,
                n,
                &self.rope_time,
                self.attention.time(),
            ),
            Step::ToFreq => r.transpose(&self.x, &self.xt, f, t),
            Step::Freq { layer, first, n } => transformer(
                &mut r,
                &self.step_bufs(),
                &self.net.layers[layer].1,
                &self.xt,
                f,
                first,
                n,
                &self.rope_freq,
                self.attention.freq().then_some(FusedKernel::Tiled),
            ),
            Step::ToTime => r.transpose(&self.xt, &self.x, t, f),
            Step::FinalNorm => r.rmsnorm(&self.x, 0, &self.net.final_norm, &self.xt, 0, f * t),
            Step::Mask {
                stem, group, k, n, ..
            } => self.mask_part(&mut r, &self.net.masks[stem][group], k, n),
        }
        r.submit();
    }

    fn split<'s>(&'s self, r: &mut Rec<'s>) {
        let t = self.frames;
        for sg in &self.net.split {
            let (g, w) = (sg.g, sg.g.width);
            let mut mm = MatMul::new(
                (&self.split_in, View::rows(t * g.offset, w).batch(t * w, 0)),
                (&sg.w, View::t(0, w).batch(DIM * w, 0)),
                (
                    &self.x,
                    View::rows(g.first * t * DIM, DIM).batch(t * DIM, 0),
                ),
                t,
                DIM,
                w,
            );
            mm.outer = g.count;
            mm.bias = Some((&sg.b, 0, DIM));
            r.matmul(&mm);
        }
    }

    /// The buffers that a transformer step of this job writes.
    fn step_bufs(&self) -> StepBufs<'_> {
        StepBufs {
            xn: &self.xn,
            qkv: &self.qkv,
            scores: &self.scores,
            o: &self.o,
            gates: &self.gates,
            h: &self.h,
        }
    }

    fn mask_part<'s>(&'s self, r: &mut Rec<'s>, mg: &'s MaskGroup, k: usize, n: usize) {
        let t = self.frames;
        let (g, w) = (mg.g, mg.g.width);
        let mut h = MatMul::new(
            (
                &self.xt,
                View::rows((g.first + k) * t * DIM, DIM).batch(t * DIM, 0),
            ),
            (
                &mg.w1,
                View::t(k * HIDDEN * DIM, DIM).batch(HIDDEN * DIM, 0),
            ),
            (&self.h, View::rows(0, HIDDEN).batch(t * HIDDEN, 0)),
            t,
            HIDDEN,
            DIM,
        );
        h.outer = n;
        h.bias = Some((&mg.b1, k * HIDDEN, HIDDEN));
        h.act = Act::Tanh;
        r.matmul(&h);
        let mut y = MatMul::new(
            (&self.h, View::rows(0, HIDDEN).batch(t * HIDDEN, 0)),
            (
                &mg.w2,
                View::t(k * 2 * w * HIDDEN, HIDDEN).batch(2 * w * HIDDEN, 0),
            ),
            (&self.y, View::rows(0, 2 * w).batch(t * 2 * w, 0)),
            t,
            2 * w,
            HIDDEN,
        );
        y.outer = n;
        y.bias = Some((&mg.b2, k * 2 * w, 2 * w));
        r.matmul(&y);
        r.glu(&self.y, &self.mask, n, t, w, g.offset + k * w);
    }

    /// Runs every step, waiting for the GPU after each, and returns the
    /// six masks.  For tests and native use; the browser worker drives
    /// [`Self::steps`] itself to pace and time them.
    pub async fn masks(&self, features: &[f32]) -> Result<Vec<Vec<f32>>, String> {
        self.set_input(features);
        let mut out = Vec::new();
        for step in self.steps() {
            self.run(&step);
            self.gpu.done().await;
            if let Some(e) = self.gpu.take_errors().into_iter().next() {
                return Err(e);
            }
            if let Step::Mask { last: true, .. } = step {
                out.push(self.mask().await?);
            }
        }
        Ok(out)
    }
}

/// The buffers that one transformer step writes, besides the hidden
/// state: the normalised input, q/k/v, the scores of split attention, the
/// attention output, the gates and the feed-forward's hidden layer.
struct StepBufs<'s> {
    xn: &'s wgpu::Buffer,
    qkv: &'s wgpu::Buffer,
    scores: &'s wgpu::Buffer,
    o: &'s wgpu::Buffer,
    gates: &'s wgpu::Buffer,
    h: &'s wgpu::Buffer,
}

/// One transformer on sequences `first..first + nb` of `buf`
/// (`[*, seq, DIM]`), updated in place, with the step buffers `s`.
/// [`Job::run`] and calibration ([`TimeStep`]) both record a step with it,
/// so calibration times the dispatches that the separation runs.
#[allow(clippy::too_many_arguments)]
fn transformer<'s>(
    r: &mut Rec<'s>,
    s: &StepBufs<'s>,
    b: &'s Block,
    buf: &'s wgpu::Buffer,
    seq: usize,
    first: usize,
    nb: usize,
    rope: &'s wgpu::Buffer,
    fused: Option<FusedKernel>,
) {
    let tok0 = first * seq;
    let ntok = nb * seq;
    let x_off = tok0 * DIM;
    r.rmsnorm(buf, x_off, &b.attn_norm, s.xn, 0, ntok);
    r.matmul(&MatMul::new(
        (s.xn, View::rows(0, DIM)),
        (&b.qkv, View::t(0, DIM)),
        (s.qkv, View::rows(0, 3 * INNER)),
        ntok,
        3 * INNER,
        DIM,
    ));
    r.rope(s.qkv, rope, ntok, seq);
    let mut g = MatMul::new(
        (s.xn, View::rows(0, DIM)),
        (&b.gates_w, View::t(0, DIM)),
        (s.gates, View::rows(0, HEADS)),
        ntok,
        HEADS,
        DIM,
    );
    g.bias = Some((&b.gates_b, 0, 0));
    r.matmul(&g);

    let alpha = (DIM_HEAD as f32).powf(-0.5);
    match fused {
        // Scores, softmax, p.v and gate in one dispatch; the scores
        // never reach memory.
        Some(kernel) => r.attention(kernel, s.qkv, Some(s.gates), s.o, seq, nb, alpha),
        None => split_attention(r, s, seq, nb, alpha),
    }
    let mut out = MatMul::new(
        (s.o, View::rows(0, INNER)),
        (&b.out, View::t(0, INNER)),
        (buf, View::rows(x_off, DIM)),
        ntok,
        DIM,
        INNER,
    );
    out.accumulate = true;
    r.matmul(&out);

    r.rmsnorm(buf, x_off, &b.ff_norm, s.xn, 0, ntok);
    let mut h = MatMul::new(
        (s.xn, View::rows(0, DIM)),
        (&b.w1, View::t(0, DIM)),
        (s.h, View::rows(0, HIDDEN)),
        ntok,
        HIDDEN,
        DIM,
    );
    h.bias = Some((&b.b1, 0, 0));
    h.act = Act::Gelu;
    r.matmul(&h);
    let mut y = MatMul::new(
        (s.h, View::rows(0, HIDDEN)),
        (&b.w2, View::t(0, HIDDEN)),
        (buf, View::rows(x_off, DIM)),
        ntok,
        DIM,
        HIDDEN,
    );
    y.bias = Some((&b.b2, 0, 0));
    y.accumulate = true;
    r.matmul(&y);
}

/// Attention as separate dispatches: scores matmul into `s.scores`,
/// softmax, p.v matmul, gate.  The fused kernel computes the same.
fn split_attention<'s>(r: &mut Rec<'s>, s: &StepBufs<'s>, seq: usize, nb: usize, alpha: f32) {
    // scores[s, h, i, j] = q[s, i, h] . k[s, j, h] / 8
    let row = 3 * INNER;
    let mut sc = MatMul::new(
        (
            s.qkv,
            View {
                off: 0,
                rs: row,
                cs: 1,
                bo: seq * row,
                bi: DIM_HEAD,
            },
        ),
        (
            s.qkv,
            View {
                off: INNER,
                rs: 1,
                cs: row,
                bo: seq * row,
                bi: DIM_HEAD,
            },
        ),
        (
            s.scores,
            View {
                off: 0,
                rs: seq,
                cs: 1,
                bo: HEADS * seq * seq,
                bi: seq * seq,
            },
        ),
        seq,
        seq,
        DIM_HEAD,
    );
    sc.outer = nb;
    sc.inner = HEADS;
    sc.alpha = alpha;
    r.matmul(&sc);
    r.softmax(s.scores, nb * HEADS * seq, seq);
    // o[s, i, h*64 + d] = sum_j p[s, h, i, j] v[s, j, h, d]
    let mut pv = MatMul::new(
        (
            s.scores,
            View {
                off: 0,
                rs: seq,
                cs: 1,
                bo: HEADS * seq * seq,
                bi: seq * seq,
            },
        ),
        (
            s.qkv,
            View {
                off: 2 * INNER,
                rs: row,
                cs: 1,
                bo: seq * row,
                bi: DIM_HEAD,
            },
        ),
        (
            s.o,
            View {
                off: 0,
                rs: INNER,
                cs: 1,
                bo: seq * INNER,
                bi: DIM_HEAD,
            },
        ),
        seq,
        DIM_HEAD,
        seq,
    );
    pv.outer = nb;
    pv.inner = HEADS;
    r.matmul(&pv);
    r.gate(s.o, s.gates, nb * seq);
}

/// One time-transformer step with zero weights, for calibration
/// (`bench::calibrate`).  It records the dispatches of [`Job::run`] for
/// [`Step::Time`], through the same function, on buffers of its own.  It
/// holds no [`Net`] and no [`Job`]: `?bench` has no model, and a job holds
/// the hidden states of a whole chunk.
pub(crate) struct TimeStep {
    block: Block,
    x: GpuBuffer,
    xn: GpuBuffer,
    qkv: GpuBuffer,
    scores: GpuBuffer,
    o: GpuBuffer,
    gates: GpuBuffer,
    h: GpuBuffer,
    rope: GpuBuffer,
    frames: usize,
}

impl TimeStep {
    /// The buffers for the largest step of [`time_steps`] at `frames` and
    /// `budget`, with weights in `format`, and the scores of a split step
    /// when `split`.  At 1,151 frames and [`BUDGET`], with `split`, about
    /// 113 MB: q/k/v of 4 bands (28 MB), the scores of 1 band (42 MB), the
    /// hidden layer (19 MB), `o` (9.4 MB), `x` and `xn` (4.7 MB each), and
    /// one layer's weights.
    pub(crate) fn new(
        gpu: &Gpu,
        frames: usize,
        budget: usize,
        format: WeightFormat,
        split: bool,
    ) -> Self {
        let most = |fused: bool| time_steps(frames, budget, fused).into_iter().max();
        let bands = most(true).max(most(false)).unwrap_or(1);
        let tokens = bands * frames;
        let scores = if split {
            most(false).unwrap_or(1) * HEADS * frames * frames
        } else {
            0
        };
        let weight = |label, len| gpu.zero_weight(label, len, format);
        let block = Block {
            attn_norm: gpu.buffer("calibration attn_norm", DIM),
            qkv: weight("calibration qkv weight", 3 * INNER * DIM),
            gates_w: weight("calibration gates weight", HEADS * DIM),
            gates_b: gpu.buffer("calibration gates bias", HEADS),
            out: weight("calibration out weight", DIM * INNER),
            ff_norm: gpu.buffer("calibration ff_norm", DIM),
            w1: weight("calibration w1", HIDDEN * DIM),
            b1: gpu.buffer("calibration b1", HIDDEN),
            w2: weight("calibration w2", DIM * HIDDEN),
            b2: gpu.buffer("calibration b2", DIM),
        };
        Self {
            block,
            x: gpu.buffer("calibration x", tokens * DIM),
            xn: gpu.buffer("calibration xn", tokens * DIM),
            qkv: gpu.buffer("calibration qkv", tokens * 3 * INNER),
            scores: gpu.buffer("calibration scores", scores),
            o: gpu.buffer("calibration o", tokens * INNER),
            gates: gpu.buffer("calibration gates", tokens * HEADS),
            h: gpu.buffer("calibration h", tokens * HIDDEN),
            rope: gpu.upload("calibration rope", &rope_table(frames)),
            frames,
        }
    }

    /// Records one time-transformer step of `bands` bands, with the fused
    /// kernel `fused` or split attention, as [`Job::run`] records
    /// [`Step::Time`].
    pub(crate) fn record<'s>(&'s self, r: &mut Rec<'s>, bands: usize, fused: Option<FusedKernel>) {
        let bufs = StepBufs {
            xn: &self.xn,
            qkv: &self.qkv,
            scores: &self.scores,
            o: &self.o,
            gates: &self.gates,
            h: &self.h,
        };
        transformer(
            r,
            &bufs,
            &self.block,
            &self.x,
            self.frames,
            0,
            bands,
            &self.rope,
            fused,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The normalised input `[count, T, width]` for each group, as
    /// `Job::set_input` made it whole before it wrote one band at a
    /// time: the reference for [`normalise_band`].
    fn normalised_whole(features: &[f32], gammas: &[Vec<f32>]) -> Vec<f32> {
        let t = features.len() / FEATURES;
        let mut out = vec![0.0f32; t * FEATURES];
        for (g, gamma) in band_groups().iter().zip(gammas) {
            let w = g.width;
            for j in 0..g.count {
                let gamma = &gamma[j * w..(j + 1) * w];
                for f in 0..t {
                    let src = &features[f * FEATURES + g.offset + j * w..][..w];
                    let n = src.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-12);
                    let dst = &mut out[t * g.offset + (j * t + f) * w..][..w];
                    for c in 0..w {
                        dst[c] = src[c] / n * gamma[c];
                    }
                }
            }
        }
        out
    }

    /// A value in -1..1 from `k`, with no pattern.
    fn noise(k: usize) -> f32 {
        let h = (k as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 40;
        (h % 2001) as f32 / 1000.0 - 1.0
    }

    /// Cut 3 of M1 of `plans/active/gpu-speed.md`.  The input has one
    /// frame of one band that is all zeros.  It asserts:
    /// * the bands that `normalise_bands` writes, put at the offsets that
    ///   it gives, equal the normalised input made whole, bit for bit;
    /// * the widest band written is `widest_band`.
    #[test]
    fn bands_normalised_one_at_a_time_equal_the_whole_input() {
        let t = 37;
        let mut features: Vec<f32> = (0..t * FEATURES).map(noise).collect();
        let quiet = band_groups()[2];
        features[5 * FEATURES + quiet.offset..][..quiet.width].fill(0.0);
        let gammas: Vec<Vec<f32>> = band_groups()
            .iter()
            .map(|g| {
                (0..g.count * g.width)
                    .map(|k| 1.0 + noise(k + 11))
                    .collect()
            })
            .collect();
        let want = normalised_whole(&features, &gammas);
        let mut got = vec![f32::NAN; t * FEATURES];
        let groups = band_groups();
        let mut widest = 0;
        normalise_bands(
            &features,
            groups.iter().zip(gammas.iter().map(Vec::as_slice)),
            |at, rows| {
                widest = widest.max(rows.len() / t);
                got[at..][..rows.len()].copy_from_slice(rows);
            },
        );
        for (k, (g, w)) in got.iter().zip(&want).enumerate() {
            assert_eq!(g.to_bits(), w.to_bits(), "float {k}");
        }
        assert_eq!(
            widest,
            widest_band(),
            "the widest band written is `widest_band`"
        );
    }

    /// Without the scores buffer, a time-axis step takes 4 bands of a full
    /// chunk instead of 1 at the default budget.
    #[test]
    fn fused_attention_fits_more_bands_per_step() {
        let budget = BUDGET;
        let frames = roformer::CHUNK / roformer::HOP + 1;
        assert_eq!((budget / per_seq(frames, false)).clamp(1, BANDS), 1);
        assert_eq!((budget / per_seq(frames, true)).clamp(1, BANDS), 4);
        // The frequency axis is bounded by q/k/v either way.
        assert_eq!(per_seq(BANDS, false), per_seq(BANDS, true));
    }

    /// Asserts that `time_steps`, at a full chunk (1,151 frames) and
    /// `BUDGET`, gives 62 steps of 1 band without fused attention, and 15
    /// steps of 4 bands then 1 of 2 with it.  At a budget of one element,
    /// each step takes 1 band.  At a budget for all the bands, one step
    /// takes the 62.  Each list covers the 62 bands once.
    #[test]
    fn time_steps_of_a_full_chunk() {
        let frames = roformer::CHUNK / roformer::HOP + 1;
        assert_eq!(frames, 1151);
        assert_eq!(time_steps(frames, BUDGET, false), vec![1; 62]);
        let mut fused = vec![4; 15];
        fused.push(2);
        assert_eq!(time_steps(frames, BUDGET, true), fused);
        assert_eq!(time_steps(frames, 1, true), vec![1; BANDS]);
        let all = BANDS * per_seq(frames, false);
        assert_eq!(time_steps(frames, all, false), vec![BANDS]);
        for budget in [1, BUDGET, 3 * BUDGET, all] {
            for fused in [false, true] {
                let steps = time_steps(frames, budget, fused);
                assert_eq!(steps.iter().sum::<usize>(), BANDS, "{budget} {fused}");
            }
        }
    }

    /// One full chunk (1,151 frames) is about 6.1 TFLOP of matmuls, the
    /// figure the timing estimates work from; the step split does not change it.
    #[test]
    fn chunk_flops() {
        let frames = roformer::CHUNK / roformer::HOP + 1;
        let mut steps = vec![Step::Split];
        for layer in 0..DEPTH {
            steps.extend((0..BANDS).map(|first| Step::Time { layer, first, n: 1 }));
            steps.push(Step::Freq {
                layer,
                first: 0,
                n: frames,
            });
        }
        for stem in 0..STEMS.len() {
            for (group, g) in band_groups().iter().enumerate() {
                steps.push(Step::Mask {
                    stem,
                    group,
                    k: 0,
                    n: g.count,
                    last: false,
                });
            }
        }
        let total: f64 = steps.iter().map(|s| s.flops(frames)).sum();
        assert_eq!(frames, 1151);
        assert!((6.0e12..6.3e12).contains(&total), "{total:e}");
    }
}
