//! Matmul throughput on this GPU for every kernel in [`TILES`], on the
//! shapes one full chunk (1,151 frames) actually uses.  Run in the browser
//! with `?bench`.  [`calibrate`] is the short version every GPU runs once,
//! before its first separation, to choose its kernels; nothing is tuned
//! while a song is being separated.

use crate::net::{TimeStep, time_steps};
use crate::{
    Act, AttentionPolicy, FusedKernel, Gpu, GpuBuffer, MatMul, MatMulPolicy, Rec, TILES, View,
    Weight, WeightFormat,
};
use roformer::DEPTH;

/// One matmul shape of the network and how often a chunk runs it.
#[derive(Clone, Copy, Debug)]
pub struct Shape {
    pub name: &'static str,
    pub m: usize,
    pub n: usize,
    pub k: usize,
    pub outer: usize,
    pub inner: usize,
    /// Dispatches of this shape per chunk.
    pub per_chunk: usize,
    pub kind: Kind,
}

/// What a [`Shape`] multiplies.
#[derive(Clone, Copy, Debug)]
pub enum Kind {
    /// Activations `[m, k]` times a torch-layout weight `[n, k]`.
    Linear(Act),
    /// Attention scores q·kᵀ per head, from a qkv buffer.
    Scores,
    /// Attention probabilities times v per head.
    Values,
}

const T: usize = 1151; // frames in a full chunk
const FREQ_TOKENS: usize = 88 * 62; // one frequency-transformer step

impl Shape {
    pub fn flops(&self) -> f64 {
        2.0 * (self.m * self.n * self.k * self.outer * self.inner) as f64
    }
}

/// The shapes of one full chunk with the default step budget.  Counts:
/// 744 time-transformer steps (62 bands x 12 layers), 168 frequency steps
/// (14 x 12, the last of each layer a little smaller, ignored here) and 84
/// mask-estimator steps (14 x 6 stems; the second mask matmul's width varies
/// by band group, 96 stands for it).
pub fn shapes() -> Vec<Shape> {
    let lin = |name, m, n, k, outer, per_chunk, act| Shape {
        name,
        m,
        n,
        k,
        outer,
        inner: 1,
        per_chunk,
        kind: Kind::Linear(act),
    };
    vec![
        lin("time qkv", T, 1536, 256, 1, 744, Act::None),
        lin("time gates", T, 8, 256, 1, 744, Act::None),
        Shape {
            name: "time scores",
            m: T,
            n: T,
            k: 64,
            outer: 1,
            inner: 8,
            per_chunk: 744,
            kind: Kind::Scores,
        },
        Shape {
            name: "time p·v",
            m: T,
            n: 64,
            k: T,
            outer: 1,
            inner: 8,
            per_chunk: 744,
            kind: Kind::Values,
        },
        lin("time attn out", T, 256, 512, 1, 744, Act::None),
        lin("time ff up", T, 1024, 256, 1, 744, Act::Gelu),
        lin("time ff down", T, 256, 1024, 1, 744, Act::None),
        lin("freq qkv", FREQ_TOKENS, 1536, 256, 1, 168, Act::None),
        Shape {
            name: "freq scores",
            m: 62,
            n: 62,
            k: 64,
            outer: 88,
            inner: 8,
            per_chunk: 168,
            kind: Kind::Scores,
        },
        Shape {
            name: "freq p·v",
            m: 62,
            n: 64,
            k: 62,
            outer: 88,
            inner: 8,
            per_chunk: 168,
            kind: Kind::Values,
        },
        lin("freq attn out", FREQ_TOKENS, 256, 512, 1, 168, Act::None),
        lin("freq ff up", FREQ_TOKENS, 1024, 256, 1, 168, Act::Gelu),
        lin("freq ff down", FREQ_TOKENS, 256, 1024, 1, 168, Act::None),
        lin("mask hidden", T, 1024, 256, 7, 84, Act::Tanh),
        lin("mask out", T, 96, 1024, 7, 84, Act::None),
    ]
}

struct Buffers {
    act: GpuBuffer,
    weight: Weight,
    /// The same weights as f16, when the f16 variants are timed.
    weight16: Option<Weight>,
    qkv: GpuBuffer,
    out: GpuBuffer,
}

impl Buffers {
    fn weight(&self, format: WeightFormat) -> &Weight {
        match format {
            WeightFormat::F32 => &self.weight,
            WeightFormat::F16 => self.weight16.as_ref().expect("f16 bench weights"),
        }
    }
}

/// The product of shape `s` with kernel `tile`; a linear shape reads its
/// weights in `format` (the scores and P·v have no weights, and ignore it).
fn matmul<'a>(s: &Shape, b: &'a Buffers, tile: usize, format: WeightFormat) -> MatMul<'a> {
    let row = 1536;
    let mut mm = match s.kind {
        Kind::Linear(act) => {
            let mut mm = MatMul::new(
                (&b.act, View::rows(0, s.k).batch(s.m * s.k, 0)),
                (b.weight(format), View::t(0, s.k).batch(s.n * s.k, 0)),
                (&b.out, View::rows(0, s.n).batch(s.m * s.n, 0)),
                s.m,
                s.n,
                s.k,
            );
            mm.act = act;
            mm
        }
        Kind::Scores => {
            let seq = s.m;
            MatMul::new(
                (
                    &b.qkv,
                    View {
                        off: 0,
                        rs: row,
                        cs: 1,
                        bo: seq * row,
                        bi: 64,
                    },
                ),
                (
                    &b.qkv,
                    View {
                        off: 512,
                        rs: 1,
                        cs: row,
                        bo: seq * row,
                        bi: 64,
                    },
                ),
                (
                    &b.out,
                    View {
                        off: 0,
                        rs: seq,
                        cs: 1,
                        bo: 8 * seq * seq,
                        bi: seq * seq,
                    },
                ),
                seq,
                seq,
                64,
            )
        }
        Kind::Values => {
            let seq = s.m;
            MatMul::new(
                (
                    &b.out,
                    View {
                        off: 0,
                        rs: seq,
                        cs: 1,
                        bo: 8 * seq * seq,
                        bi: seq * seq,
                    },
                ),
                (
                    &b.qkv,
                    View {
                        off: 1024,
                        rs: row,
                        cs: 1,
                        bo: seq * row,
                        bi: 64,
                    },
                ),
                (
                    &b.act,
                    View {
                        off: 0,
                        rs: 512,
                        cs: 1,
                        bo: seq * 512,
                        bi: 64,
                    },
                ),
                seq,
                64,
                seq,
            )
        }
    };
    mm.outer = s.outer;
    mm.inner = s.inner;
    mm.tile = Some(tile);
    mm
}

/// The bench's buffers, with f16 weights too when `f16`.
fn buffers(gpu: &Gpu, f16: bool) -> Buffers {
    let weights = 7 * 1024 * 256;
    Buffers {
        act: gpu.buffer("bench act", FREQ_TOKENS * 1024),
        weight: gpu.zero_weight("bench weight", weights, WeightFormat::F32),
        weight16: f16.then(|| gpu.zero_weight("bench weight f16", weights, WeightFormat::F16)),
        qkv: gpu.buffer("bench qkv", FREQ_TOKENS * 1536),
        out: gpu.buffer("bench out", (8 * T * T).max(FREQ_TOKENS * 1536)),
    }
}

/// Seconds per dispatch of one shape with one kernel.  Runs for `warm`
/// seconds first (pipeline compilation, and GPU clocks that rise only under
/// load), then one round of at least `round` seconds.  Submissions hold at
/// most 4 GFLOP, so no command buffer runs long even on a slow GPU.
///
/// The round queues enough submissions before each wait for the GPU that
/// one wait covers at least [`WAIT_SECS`].  Waiting after every submission
/// made each one cost a whole number of milliseconds on Chrome, whatever
/// the GPU did (rates fell on levels such as 905, 1,207 and 1,810 GFLOP/s
/// for one shape), so the per-dispatch figure measured the wait as much as
/// the kernel.
#[allow(clippy::too_many_arguments)]
async fn time_one(
    gpu: &Gpu,
    b: &Buffers,
    s: &Shape,
    tile: usize,
    format: WeightFormat,
    warm: f64,
    round: f64,
    now: &impl Fn() -> f64,
) -> f64 {
    let mm = matmul(s, b, tile, format);
    let per_submit = ((4e9 / s.flops()) as usize).clamp(1, 32);
    let submit = |times: usize| {
        for _ in 0..times {
            let mut r = gpu.rec();
            for _ in 0..per_submit {
                r.matmul(&mm);
            }
            r.submit();
        }
    };
    let t0 = now();
    let mut one = f64::INFINITY;
    loop {
        let t = now();
        submit(1);
        gpu.done().await;
        one = one.min(now() - t);
        if now() - t0 >= warm {
            break;
        }
    }
    let queued = queued(Waits::Covering, one);
    let (t0, mut count) = (now(), 0);
    while count == 0 || now() - t0 < round {
        submit(queued);
        gpu.done().await;
        count += queued * per_submit;
    }
    (now() - t0) / count as f64
}

/// Wall time each wait for the GPU should cover in a timed round.
pub const WAIT_SECS: f64 = 0.05;

/// One kernel's timings on one shape over the interleaved rounds.
#[derive(Clone, Debug, Default)]
pub struct Timing {
    /// Seconds per dispatch in each round; empty for a kernel that was not
    /// run (failed [`Gpu::check_kernels`] or not selected).
    pub rounds: Vec<f64>,
}

impl Timing {
    /// Median seconds per dispatch, or infinity when not run.
    pub fn median(&self) -> f64 {
        if self.rounds.is_empty() {
            return f64::INFINITY;
        }
        let mut v = self.rounds.clone();
        v.sort_by(f64::total_cmp);
        let n = v.len();
        if n % 2 == 1 {
            v[n / 2]
        } else {
            0.5 * (v[n / 2 - 1] + v[n / 2])
        }
    }

    /// (max - min) / median over the rounds.
    pub fn spread(&self) -> f64 {
        let lo = self.rounds.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = self.rounds.iter().copied().fold(0.0, f64::max);
        if self.rounds.len() < 2 {
            0.0
        } else {
            (hi - lo) / self.median()
        }
    }
}

/// What [`measure`] timed: seconds per dispatch of each shape with each
/// kernel (`[shape][tile]`) with f32 weights, and with f16 weights on a
/// GPU with [`Gpu::f16_ok`].  The scores and P·v have no weights, so their
/// f16 rows are their f32 timings.
#[derive(Clone, Debug)]
pub struct Measured {
    pub f32: Vec<Vec<Timing>>,
    pub f16: Option<Vec<Vec<Timing>>>,
}

/// Why `?bench` times no f16 weights on this GPU, if it does not.
fn no_f16_reason(gpu: &Gpu) -> Option<&'static str> {
    if !gpu.has_f16_unpack() {
        Some("this adapter cannot unpack f16 (no SHADER_F16_IN_F32)")
    } else if !gpu.f16_ok.get() {
        Some("the f16 start-up check did not pass on this GPU")
    } else {
        None
    }
}

/// Times each shape with each kernel (the median of `rounds` rounds;
/// infinite for kernels that failed [`Gpu::check_kernels`] or are not in
/// `only`), with f32 weights, and on a GPU with [`Gpu::f16_ok`] the linear
/// shapes with f16 weights too.  The kernels and the formats take turns
/// within each round (A f32, A f16, B f32, B f16, ...), so a GPU that heats
/// up and slows down over the run slows every kernel and format alike
/// instead of whichever ran last.  `now` is a clock in seconds.  With
/// `warm` 0.3, `round` 0.2 and 3 rounds it takes a few minutes on a real
/// GPU.
pub async fn measure(
    gpu: &Gpu,
    warm: f64,
    round: f64,
    rounds: usize,
    only: Option<&[usize]>,
    now: impl Fn() -> f64,
    mut log: impl FnMut(String),
) -> Measured {
    let recording = gpu.set_recording(false);
    let why_no_f16 = no_f16_reason(gpu);
    if let Some(why) = why_no_f16 {
        log(format!("bench: f16 weights not timed: {why}"));
    }
    let b = buffers(gpu, why_no_f16.is_none());
    let run: Vec<bool> = (0..TILES.len())
        .map(|t| gpu.usable.get()[t] && only.is_none_or(|o| o.contains(&t)))
        .collect();
    let mut f32_table = Vec::new();
    let mut f16_table = Vec::new();
    for s in &shapes() {
        let linear = matches!(s.kind, Kind::Linear(_));
        let formats: &[WeightFormat] = if linear && why_no_f16.is_none() {
            &WeightFormat::ALL
        } else {
            &[WeightFormat::F32]
        };
        // What runs when each kernel is asked for: a vec4 kernel hands
        // products it cannot read as vec4s to the fallback.  The fallback is
        // timed once for both (also when it was not asked for itself) and
        // its timings stand for the vec4 kernel.  Both formats hand off
        // alike: the f16 buffer is a whole number of vec4s too.
        let ran: Vec<usize> = (0..TILES.len())
            .map(|t| matmul(s, &b, t, WeightFormat::F32).kernel(t))
            .collect();
        let timed: Vec<bool> = (0..TILES.len())
            .map(|u| (0..TILES.len()).any(|t| run[t] && ran[t] == u))
            .collect();
        let empty = vec![Timing { rounds: Vec::new() }; TILES.len()];
        let mut rows = vec![empty; formats.len()];
        for r in 0..rounds.max(1) {
            for t in (0..TILES.len()).filter(|&t| timed[t]) {
                for (f, &format) in formats.iter().enumerate() {
                    let w = if r == 0 { warm } else { 0.0 };
                    let secs = time_one(gpu, &b, s, t, format, w, round, &now).await;
                    rows[f][t].rounds.push(secs);
                }
            }
        }
        for (f, &format) in formats.iter().enumerate() {
            let row = &mut rows[f];
            let weights = if linear {
                format!(", {} weights", format.name())
            } else {
                String::new()
            };
            for (t, tile) in TILES.iter().enumerate() {
                if !gpu.usable.get()[t] {
                    log(format!(
                        "bench: {} {}{weights}: skipped (failed the start-up check on this GPU)",
                        s.name, tile.name
                    ));
                } else if run[t] && ran[t] != t {
                    row[t] = row[ran[t]].clone();
                    log(format!(
                        "bench: {} {}{weights}: runs {}, {:.1} GFLOP/s (not vec4-aligned: an operand not K-contiguous, or K, a stride or an offset not a multiple of 4)",
                        s.name,
                        tile.name,
                        TILES[ran[t]].name,
                        s.flops() / row[t].median() / 1e9
                    ));
                } else if run[t] {
                    let tm = &row[t];
                    log(format!(
                        "bench: {} ({}x{}x{}, x{}) {}{weights}: {:.1} GFLOP/s (median of {}, spread {:.1}%)",
                        s.name,
                        s.m,
                        s.n,
                        s.k,
                        s.outer * s.inner,
                        tile.name,
                        s.flops() / tm.median() / 1e9,
                        tm.rounds.len(),
                        100.0 * tm.spread()
                    ));
                }
            }
            // A fallback timed only on behalf of a vec4 kernel stays out of
            // the table, as a kernel that was not asked for.
            for (t, timing) in row.iter_mut().enumerate() {
                if !run[t] {
                    timing.rounds.clear();
                }
            }
        }
        let f32_row = rows.swap_remove(0);
        if why_no_f16.is_none() {
            f16_table.push(rows.pop().unwrap_or_else(|| f32_row.clone()));
        }
        f32_table.push(f32_row);
    }
    if let Some(e) = gpu.take_errors().into_iter().next() {
        log(format!("bench: WebGPU error: {e}"));
    }
    gpu.set_recording(recording);
    Measured {
        f32: f32_table,
        f16: why_no_f16.is_none().then_some(f16_table),
    }
}

/// How many submissions a timed round queues before each wait for the GPU.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Waits {
    /// Enough to cover [`WAIT_SECS`], as [`time_one`] times a kernel.
    Covering,
    /// One: a wait after each submission, as the separation waits after
    /// each step (`Pace::run` in the worker).
    EachStep,
}

/// The submissions for each wait of a timed round with `waits`, when one
/// submission and its wait took `one` seconds.
pub fn queued(waits: Waits, one: f64) -> usize {
    match waits {
        Waits::Covering => ((WAIT_SECS / one.max(1e-6)).ceil() as usize).clamp(1, 64),
        Waits::EachStep => 1,
    }
}

/// How [`time_axis_per_chunk`], and so calibration, waits for the GPU: after
/// each step, as the separation does.
pub const STEP_WAITS: Waits = Waits::EachStep;

/// Seconds per call of `record` (which adds dispatches to one submission),
/// timed like [`time_one`]: `warm` seconds first, then one round of at
/// least `round` seconds with [`queued`] submissions for each wait.
async fn time_rec<'g>(
    gpu: &'g Gpu,
    warm: f64,
    round: f64,
    waits: Waits,
    now: &impl Fn() -> f64,
    record: &impl Fn(&mut Rec<'g>),
) -> f64 {
    let submit = |times: usize| {
        for _ in 0..times {
            let mut r = gpu.rec();
            record(&mut r);
            r.submit();
        }
    };
    let t0 = now();
    let mut one = f64::INFINITY;
    loop {
        let t = now();
        submit(1);
        gpu.done().await;
        one = one.min(now() - t);
        if now() - t0 >= warm {
            break;
        }
    }
    let queued = queued(waits, one);
    let (t0, mut count) = (now(), 0);
    while count == 0 || now() - t0 < round {
        submit(queued);
        gpu.done().await;
        count += queued;
    }
    (now() - t0) / count as f64
}

/// Buffers for attention timings: q/k/v, gates, scores and output for up
/// to four bands of a full chunk (the scores for one).
struct AttnBufs {
    qkv: GpuBuffer,
    gates: GpuBuffer,
    scores: GpuBuffer,
    o: GpuBuffer,
}

fn attn_bufs(gpu: &Gpu) -> AttnBufs {
    let tokens = (4 * T).max(FREQ_TOKENS);
    AttnBufs {
        qkv: gpu.buffer("bench qkv", tokens * 1536),
        gates: gpu.buffer("bench gates", tokens * 8),
        scores: gpu.buffer("bench scores", (8 * T * T).max(8 * 62 * 62 * 88)),
        o: gpu.buffer("bench o", tokens * 512),
    }
}

/// One attention step done the split way, as net.rs does it: scores
/// matmul (this GPU's matmul policy), softmax, p.v matmul, gate.
fn split_attention<'b>(r: &mut Rec<'b>, b: &'b AttnBufs, seq: usize, nseq: usize) {
    let row = 1536;
    let mut sc = MatMul::new(
        (
            &b.qkv,
            View {
                off: 0,
                rs: row,
                cs: 1,
                bo: seq * row,
                bi: 64,
            },
        ),
        (
            &b.qkv,
            View {
                off: 512,
                rs: 1,
                cs: row,
                bo: seq * row,
                bi: 64,
            },
        ),
        (
            &b.scores,
            View {
                off: 0,
                rs: seq,
                cs: 1,
                bo: 8 * seq * seq,
                bi: seq * seq,
            },
        ),
        seq,
        seq,
        64,
    );
    sc.outer = nseq;
    sc.inner = 8;
    sc.alpha = 0.125;
    r.matmul(&sc);
    r.softmax(&b.scores, nseq * 8 * seq, seq);
    let mut pv = MatMul::new(
        (
            &b.scores,
            View {
                off: 0,
                rs: seq,
                cs: 1,
                bo: 8 * seq * seq,
                bi: seq * seq,
            },
        ),
        (
            &b.qkv,
            View {
                off: 1024,
                rs: row,
                cs: 1,
                bo: seq * row,
                bi: 64,
            },
        ),
        (
            &b.o,
            View {
                off: 0,
                rs: 512,
                cs: 1,
                bo: seq * 512,
                bi: 64,
            },
        ),
        seq,
        64,
        seq,
    );
    pv.outer = nseq;
    pv.inner = 8;
    r.matmul(&pv);
    r.gate(&b.o, &b.gates, seq * nseq);
}

fn fused_attention<'b>(
    r: &mut Rec<'b>,
    b: &'b AttnBufs,
    kernel: FusedKernel,
    seq: usize,
    nseq: usize,
) {
    r.attention(kernel, &b.qkv, Some(&b.gates), &b.o, seq, nseq, 0.125);
}

/// What [`attention_per_chunk`] timed, in seconds per chunk for each
/// round.  A kernel that failed its check, or an axis that was not timed,
/// has no rounds.
#[derive(Clone, Debug)]
pub struct AttentionTimes {
    pub time_split: Timing,
    pub time_fused: Timing,
    pub time_vfused: Timing,
    pub freq_split: Timing,
    pub freq_fused: Timing,
}

/// Seconds per chunk of the attention dispatches alone, split and fused,
/// on each axis, the way a chunk runs them at the default budget: on the
/// time axis split takes one band of 1,151 frames per step (744 steps),
/// and each fused kernel four (186 steps; it needs no scores buffer); on
/// the frequency axis both take 88 frames of 62 bands per step (168
/// steps).  `vfused` runs on the time axis only.  They take turns over
/// `rounds` rounds; medians.  Without `freq` the frequency axis is not
/// timed: calibration uses fused attention there whenever it works.
/// `?bench` logs these figures ([`measure_attention`]); calibration times
/// whole steps instead ([`time_axis_per_chunk`]).
pub async fn attention_per_chunk(
    gpu: &Gpu,
    warm: f64,
    round: f64,
    rounds: usize,
    freq: bool,
    now: &impl Fn() -> f64,
) -> AttentionTimes {
    let recording = gpu.set_recording(false);
    let b = attn_bufs(gpu);
    let empty = || Timing { rounds: Vec::new() };
    let mut t = AttentionTimes {
        time_split: empty(),
        time_fused: empty(),
        time_vfused: empty(),
        freq_split: empty(),
        freq_fused: empty(),
    };
    for rd in 0..rounds.max(1) {
        let w = if rd == 0 { warm } else { 0.0 };
        let wait = Waits::Covering;
        t.time_split.rounds.push(
            time_rec(gpu, w, round, wait, now, &|r| split_attention(r, &b, T, 1)).await * 744.0,
        );
        if freq {
            t.freq_split.rounds.push(
                time_rec(gpu, w, round, wait, now, &|r| {
                    split_attention(r, &b, 62, 88)
                })
                .await
                    * 168.0,
            );
        }
        if gpu.fused_ok.get() {
            let k = FusedKernel::Tiled;
            t.time_fused.rounds.push(
                time_rec(gpu, w, round, wait, now, &|r| {
                    fused_attention(r, &b, k, T, 4)
                })
                .await
                    * 186.0,
            );
            if freq {
                t.freq_fused.rounds.push(
                    time_rec(gpu, w, round, wait, now, &|r| {
                        fused_attention(r, &b, k, 62, 88)
                    })
                    .await
                        * 168.0,
                );
            }
        }
        if gpu.vfused_ok.get() {
            let k = FusedKernel::Vec4;
            t.time_vfused.rounds.push(
                time_rec(gpu, w, round, wait, now, &|r| {
                    fused_attention(r, &b, k, T, 4)
                })
                .await
                    * 186.0,
            );
        }
    }
    gpu.set_recording(recording);
    t
}

/// Logs [`attention_per_chunk`], with the lines of M3 and M4 as they
/// were, and the time axis of each policy as calibration times it
/// ([`time_axis_per_chunk`], weights in `format`, at `budget`).
#[allow(clippy::too_many_arguments)]
pub async fn measure_attention(
    gpu: &Gpu,
    format: WeightFormat,
    budget: usize,
    warm: f64,
    round: f64,
    rounds: usize,
    now: impl Fn() -> f64,
    mut log: impl FnMut(String),
) {
    let t = attention_per_chunk(gpu, warm, round, rounds, true, &now).await;
    for (axis, split, fused) in [
        ("time", &t.time_split, &t.time_fused),
        ("freq", &t.freq_split, &t.freq_fused),
    ] {
        log(format!(
            "bench: attention {axis} axis: split {:.2} s per chunk (spread {:.1}%), fused {:.2} s per chunk (spread {:.1}%)",
            split.median(),
            100.0 * split.spread(),
            fused.median(),
            100.0 * fused.spread()
        ));
    }
    log(format!(
        "bench: attention split {:.2} s per chunk, fused {:.2} s per chunk",
        t.time_split.median() + t.freq_split.median(),
        t.time_fused.median() + t.freq_fused.median()
    ));
    log(format!(
        "bench: attention time axis: vfused {:.2} s per chunk (spread {:.1}%)",
        t.time_vfused.median(),
        100.0 * t.time_vfused.spread()
    ));
    let s = time_axis_per_chunk(gpu, format, budget, warm, round, rounds, &now).await;
    log(format!(
        "bench: time axis, whole steps with a wait after each: split {:.2} s (spread {:.1}%), fused {:.2} s (spread {:.1}%), vfused {:.2} s (spread {:.1}%) per chunk",
        s.split.median(),
        100.0 * s.split.spread(),
        s.fused.median(),
        100.0 * s.fused.spread(),
        s.vfused.median(),
        100.0 * s.vfused.spread()
    ));
}

/// What [`time_axis_per_chunk`] timed, in seconds per chunk for each
/// round: the time axis with each policy.  A kernel that failed its check
/// has no rounds.
#[derive(Clone, Debug)]
pub struct TimeAxis {
    pub split: Timing,
    pub fused: Timing,
    pub vfused: Timing,
}

/// Seconds per chunk of the time transformer with split attention, with
/// the tiled fused kernel and with `vfused`, as the separation runs it:
/// each step of [`time_steps`] at `budget`, for each of the [`DEPTH`]
/// layers, with a wait for the GPU after each step ([`STEP_WAITS`]).  It
/// records each step with the function that `Job::run` uses
/// ([`TimeStep`]), with zero weights in `format`.  So it covers the
/// attention, the linear products, the other dispatches of a step and the
/// waits between steps.  Each step size is timed once and counted for each
/// step of that size: at the default budget, 62 steps of 1 band split, and
/// 15 steps of 4 bands and 1 of 2 fused.  The three take turns over
/// `rounds` rounds; medians.
#[allow(clippy::too_many_arguments)]
pub async fn time_axis_per_chunk(
    gpu: &Gpu,
    format: WeightFormat,
    budget: usize,
    warm: f64,
    round: f64,
    rounds: usize,
    now: &impl Fn() -> f64,
) -> TimeAxis {
    let recording = gpu.set_recording(false);
    let step = TimeStep::new(gpu, T, budget, format, true);
    let sizes = |fused: bool| {
        let mut sizes: Vec<(usize, usize)> = Vec::new();
        for bands in time_steps(T, budget, fused) {
            match sizes.iter_mut().find(|(b, _)| *b == bands) {
                Some((_, count)) => *count += 1,
                None => sizes.push((bands, 1)),
            }
        }
        sizes
    };
    let kernels = [
        (None, true),
        (Some(FusedKernel::Tiled), gpu.fused_ok.get()),
        (Some(FusedKernel::Vec4), gpu.vfused_ok.get()),
    ];
    let mut timings: [Timing; 3] = Default::default();
    for rd in 0..rounds.max(1) {
        let w = if rd == 0 { warm } else { 0.0 };
        for (timing, (kernel, ok)) in timings.iter_mut().zip(kernels) {
            if !ok {
                continue;
            }
            let mut total = 0.0;
            for (bands, count) in sizes(kernel.is_some()) {
                let secs = time_rec(gpu, w, round, STEP_WAITS, now, &|r| {
                    step.record(r, bands, kernel)
                })
                .await;
                total += secs * (count * DEPTH) as f64;
            }
            timing.rounds.push(total);
        }
    }
    gpu.set_recording(recording);
    let [split, fused, vfused] = timings;
    TimeAxis {
        split,
        fused,
        vfused,
    }
}

/// Kernel indices named in a comma-separated list (`"s64,i64"`); `None`
/// for an empty list, a lone "1" or "true" (all kernels), or unknown names.
pub fn parse_kernels(list: &str) -> Option<Vec<usize>> {
    let names: Vec<&str> = list
        .split(',')
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .collect();
    if names.is_empty() || names == ["1"] || names == ["true"] {
        return None;
    }
    names
        .iter()
        .map(|n| TILES.iter().position(|t| t.name == *n))
        .collect()
}

/// Which candidates stay in after the first round of [`calibrate`]: those
/// within 1.5 times the fastest.  A kernel several times slower on this
/// GPU (`s64` without loop unrolling) costs no more measuring time, and
/// [`masked_medians`] keeps it from being chosen.
pub fn still_in(first: &[f64]) -> Vec<bool> {
    let best = first.iter().copied().fold(f64::INFINITY, f64::min);
    first
        .iter()
        .map(|&x| x.is_finite() && x <= 1.5 * best)
        .collect()
}

/// Medians to choose from: infinite for a kernel dropped after the first
/// round (or never timed), so only kernels timed every round can win.  A
/// dropped kernel's single early time must not beat a survivor whose
/// later rounds ran slower, as a GPU warming up does.
pub fn masked_medians(t: &[Timing], alive: &[bool]) -> Vec<f64> {
    t.iter()
        .zip(alive)
        .map(|(t, &a)| if a { t.median() } else { f64::INFINITY })
        .collect()
}

/// What [`calibrate`] chooses for one GPU, as kept in browser storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Choice {
    pub matmul: MatMulPolicy,
    pub attention: AttentionPolicy,
}

impl Choice {
    /// `"<matmul policy>;<attention policy>"`, e.g. `"i128x64,i64;freq"`.
    pub fn name(self) -> String {
        format!("{};{}", self.matmul.name(), self.attention.name())
    }

    /// The inverse of [`Choice::name`].
    pub fn parse(s: &str) -> Option<Self> {
        let (m, a) = s.split_once(';')?;
        Some(Self {
            matmul: MatMulPolicy::parse(m)?,
            attention: AttentionPolicy::parse(a)?,
        })
    }
}

/// Version of the [`calibrate`] procedure.  A per-GPU choice stored by
/// another version is measured again: version 1 timed attention for 0.1 s
/// on a GPU opened moments before, and chose split attention on a Radeon
/// Pro 5500M where a longer bench measured fused 2.5 times faster.
/// Version 3 calibrates once for each weight format, with the f16
/// variants for f16 weights (M9 of `plans/completed/stream-from-storage.md`).
/// Version 4 adds `v64k8`, from M4 of `plans/active/gpu-speed.md`.
/// Version 5 times whole time-transformer steps, with a wait after each,
/// as the separation runs them ([`time_axis_per_chunk`]; M5, TD-101), for
/// [`ATTENTION_ROUNDS`] rounds, with [`FUSED_MARGIN`] 1.0.  Version 6
/// times `vfused` on the time axis too, and takes the faster fused kernel.
pub const CALIBRATION: u32 = 6;

/// The key of this GPU's stored kernel choice for weights in `format`:
/// `"<adapter>|<kernels_id>|<CALIBRATION>|<f32 or f16>"`.  The formats
/// keep separate choices, since the f16 variants can be faster or slower.
pub fn choice_key(adapter: &str, kernels_id: u64, format: WeightFormat) -> String {
    format!(
        "{adapter}|{kernels_id:016x}|{CALIBRATION}|{}",
        format.name()
    )
}

/// The line a worker logs for the kernel choice it uses, and where it came
/// from (`stored`, `calibrated` or `from the URL`), for weights in
/// `format`: `kernels: matmul v64,i64, attention fused (stored; f16
/// weights)`.  `xtask e2e weights` reads it.
pub fn kernels_line(choice: Choice, source: &str, format: WeightFormat) -> String {
    format!(
        "kernels: matmul {}, attention {} ({source}; {} weights)",
        choice.matmul.name(),
        choice.attention.name(),
        format.name()
    )
}

/// Whether a worker whose last kernel choice was for `last` (none yet)
/// chooses again for a load of `format`: once per format, not per worker.
pub fn choose_again(last: Option<WeightFormat>, format: WeightFormat) -> bool {
    last != Some(format)
}

/// Warm-up before the first timed round of each time-axis step in
/// [`calibrate`], and the length of every timed round: a time-axis step
/// takes milliseconds to tens of milliseconds, so 0.1 s held only a few of
/// them.
pub const ATTENTION_WARM: f64 = 0.5;
pub const ATTENTION_ROUND: f64 = 0.3;

/// The timed rounds of the time axis in [`calibrate`], medians compared.
/// With 3 rounds, Chrome on the M2 Pro gave a ratio (fused over split) of
/// 1.020 with a standard deviation of 0.020, so at [`FUSED_MARGIN`] 1.0 it
/// chose the slower policy now and then.  12 rounds, about 4 times the
/// timing, add about 8 s to each calibration (M5).
pub const ATTENTION_ROUNDS: usize = 12;

/// Fused attention stays on the time axis unless split attention measured
/// faster by more than this factor.  It is 1.0: the faster policy wins.
/// Calibration times whole time-transformer steps as the separation runs
/// them ([`time_axis_per_chunk`]), so a margin for fused only makes a GPU
/// choose the slower policy.  Before M5 it was 1.1, to keep fused for its
/// memory: fused needs no scores buffer and runs four bands for each step.
/// The cost of the margin 1.0 is about 42 MB of GPU memory (the scores of
/// 1 band at the default chunk) on a GPU where split wins.  Chrome on the
/// M2 Pro split before `vfused`, with a tab total of 799 MB, under
/// `TAB_BUDGET_MB` (943 MB, MEM-4).
pub const FUSED_MARGIN: f64 = 1.0;

/// The stored kernel choices (`"<key>\t<choice>"` lines, keys
/// `"<adapter>|..."`) with `key`'s choice set to `value`.  Lines for other
/// adapters (the other GPU of a two-GPU laptop) stay.  This adapter keeps
/// its newest `keep_per_gpu - 1` lines for other keys (other kernel sets or
/// calibration versions, from builds published side by side on one
/// origin) before the new line, which goes last; older ones and any
/// earlier line for `key` are dropped.  Lines without a tab are dropped.
pub fn store_choice(
    text: &str,
    adapter: &str,
    key: &str,
    value: &str,
    keep_per_gpu: usize,
) -> String {
    let prefix = format!("{adapter}|");
    let lines: Vec<(&str, &str)> = text
        .lines()
        .filter_map(|l| l.split_once('\t').map(|(k, _)| (k, l)))
        .filter(|(k, _)| *k != key)
        .collect();
    let mine = lines.iter().filter(|(k, _)| k.starts_with(&prefix)).count();
    let mut skip = mine.saturating_sub(keep_per_gpu.saturating_sub(1));
    let mut out = String::new();
    for (k, l) in lines {
        if k.starts_with(&prefix) && skip > 0 {
            skip -= 1;
            continue;
        }
        out += l;
        out += "\n";
    }
    out + &format!("{key}\t{value}\n")
}

/// Seconds per chunk of the time axis with each policy, as calibration
/// times it ([`time_axis_per_chunk`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimeSecs {
    pub split: f64,
    pub fused: f64,
    pub vfused: f64,
}

impl TimeSecs {
    /// The medians of `t`; infinite for a kernel with no rounds.
    pub fn of(t: &TimeAxis) -> Self {
        Self {
            split: t.split.median(),
            fused: t.fused.median(),
            vfused: t.vfused.median(),
        }
    }
}

/// The choice from measured seconds: for each kind of product, the kernel
/// with the least total over its shapes (`large[t]`, `small[t]`; infinite
/// for kernels that failed their check); the tiled fused kernel on the
/// frequency axis whenever it works (it was 2.2 to 3.6 times faster on
/// every GPU measured).  On the time axis, the faster fused kernel: `vfused`
/// only when it passed its check and is faster than the tiled kernel.
/// That one stays unless split measured faster by more than
/// [`FUSED_MARGIN`] (1.0: the faster one).  With no tiled kernel
/// (`fused_ok` false), split on both axes.
pub fn decide(
    large: &[f64],
    small: &[f64],
    time: TimeSecs,
    fused_ok: bool,
    vfused_ok: bool,
) -> Choice {
    let best = |v: &[f64]| {
        (0..v.len())
            .min_by(|&a, &b| v[a].total_cmp(&v[b]))
            .unwrap_or(0)
    };
    Choice {
        matmul: MatMulPolicy::Split {
            large: best(large),
            small: best(small),
        },
        attention: time_policy(time, fused_ok, vfused_ok),
    }
}

/// The attention policy of [`decide`].
fn time_policy(time: TimeSecs, fused_ok: bool, vfused_ok: bool) -> AttentionPolicy {
    if !fused_ok {
        return AttentionPolicy::Split;
    }
    let (secs, policy) = if vfused_ok && time.vfused < time.fused {
        (time.vfused, AttentionPolicy::VFused)
    } else {
        (time.fused, AttentionPolicy::Fused)
    };
    if secs <= FUSED_MARGIN * time.split {
        policy
    } else {
        AttentionPolicy::Freq
    }
}

/// Chooses this GPU's kernels, once per GPU (the caller keeps the result):
/// the matmul kernel for large products (q/k/v and feed-forward shapes)
/// and for small ones (attention gates, the p.v product of split
/// attention), then the fastest time-axis policy with that matmul choice:
/// split, the tiled fused kernel or `vfused`.  Everything measured takes turns
/// over three rounds, medians compared; a matmul kernel more than 1.5
/// times slower than the best after the first round is not timed again
/// ([`still_in`]).  What is measured is only what varies between GPUs and
/// cannot be read from WebGPU: whether the 8 x 4 or the 4 x 4 register
/// block wins (register file and scheduling), whether the compiler keeps
/// `s64`'s loop-indexed accumulators in registers, whether vec4 loads
/// (`v64`) issue faster than scalar ones, and whether fused attention wins
/// on long sequences.  A product `v64` cannot read as vec4s (the p.v
/// product) runs and is timed as `i64` under `v64`'s name, as it would
/// run in a separation.  The linear shapes are timed with weights in
/// `format`, so f16 weights get the kernels whose f16 variants run
/// fastest.
///
/// The time axis is timed as the separation runs it, at `budget`
/// ([`time_axis_per_chunk`]): whole time-transformer steps, with a wait
/// after each.  Before version 5, calibration timed the attention
/// dispatches alone, with many submissions for each wait.  That put split
/// about 0.5 s for each chunk too low in Safari and 0.26 s in Chrome
/// (TD-101).  The time axis gets longer windows ([`ATTENTION_WARM`],
/// [`ATTENTION_ROUND`]) and more rounds ([`ATTENTION_ROUNDS`]) than the
/// matmul products.
///
/// It frees the matmul buffers (about 109 MB with f16 weights) before it
/// times the time axis.  Then it holds about 113 MB ([`TimeStep`] at the
/// default chunk and budget).  Before version 5 the peak was about 196 MB.
/// Both are less than a job's buffers at the default chunk (about 310 MB),
/// which calibration frees before the separation makes them.  Takes several
/// seconds.
pub async fn calibrate(
    gpu: &Gpu,
    format: WeightFormat,
    budget: usize,
    now: impl Fn() -> f64,
    mut log: impl FnMut(String),
) -> Choice {
    let all = shapes();
    let by_name = |n: &str| all.iter().find(|s| s.name == n).copied().expect("shape");
    let large = [
        by_name("time qkv"),
        by_name("time ff down"),
        by_name("freq ff up"),
    ];
    let small = [by_name("time gates"), by_name("time p·v")];
    let (warm, round, rounds) = (0.1, 0.1, 3);
    let recording = gpu.set_recording(false);
    // The matmul buffers live only in this block, so the time axis's
    // buffers do not add to them.
    let ((l, lt), (s, st)) = {
        let b = buffers(gpu, format == WeightFormat::F16);
        let secs = |kind: Vec<Shape>| {
            let b = &b;
            let now = &now;
            async move {
                let mut t = vec![Timing { rounds: Vec::new() }; TILES.len()];
                #[expect(
                    clippy::disallowed_methods,
                    reason = "MEM-1: which tiles are usable, one flag for each"
                )]
                let mut alive = gpu.usable.get().to_vec();
                for rd in 0..rounds {
                    if rd == 1 {
                        let first: Vec<f64> = t.iter().map(Timing::median).collect();
                        alive = still_in(&first);
                    }
                    for (k, timing) in t.iter_mut().enumerate() {
                        if !alive[k] {
                            continue;
                        }
                        let mut total = 0.0;
                        for s in &kind {
                            let w = if rd == 0 { warm } else { 0.0 };
                            total += time_one(gpu, b, s, k, format, w, round, now).await
                                * s.per_chunk as f64;
                        }
                        timing.rounds.push(total);
                    }
                }
                (masked_medians(&t, &alive), t)
            }
        };
        #[expect(
            clippy::disallowed_methods,
            reason = "MEM-1: three matmul shapes to time"
        )]
        let large = secs(large.to_vec()).await;
        #[expect(
            clippy::disallowed_methods,
            reason = "MEM-1: two matmul shapes to time"
        )]
        let small = secs(small.to_vec()).await;
        (large, small)
    };
    let none = TimeSecs {
        split: 0.0,
        fused: 0.0,
        vfused: 0.0,
    };
    let matmul = decide(&l, &s, none, false, false).matmul;
    gpu.policy.set(matmul);
    let t = time_axis_per_chunk(
        gpu,
        format,
        budget,
        ATTENTION_WARM,
        ATTENTION_ROUND,
        ATTENTION_ROUNDS,
        &now,
    )
    .await;
    let choice = decide(
        &l,
        &s,
        TimeSecs::of(&t),
        gpu.fused_ok.get(),
        gpu.vfused_ok.get(),
    );
    gpu.set_recording(recording);
    let fmt = |v: &[f64], t: &[Timing]| {
        TILES
            .iter()
            .zip(v.iter().zip(t))
            .map(|(tile, (x, tm))| {
                if x.is_finite() {
                    format!("{} {x:.2} s", tile.name)
                } else if tm.rounds.is_empty() {
                    format!("{} not used", tile.name)
                } else {
                    format!(
                        "{} {:.2} s, dropped after one round",
                        tile.name, tm.rounds[0]
                    )
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    log(format!(
        "kernels for this GPU, {} weights: matmul {} (large products: {}; small: {}), attention {} (time axis, whole steps with a wait after each: split {:.2} s (spread {:.1}%), fused {:.2} s (spread {:.1}%), vfused {:.2} s (spread {:.1}%) per chunk)",
        format.name(),
        choice.matmul.name(),
        fmt(&l, &lt),
        fmt(&s, &st),
        choice.attention.name(),
        t.split.median(),
        100.0 * t.split.spread(),
        t.fused.median(),
        100.0 * t.fused.spread(),
        t.vfused.median(),
        100.0 * t.vfused.spread()
    ));
    choice
}

/// Summary lines: estimated matmul seconds per chunk for each kernel alone
/// and for the best kernel per shape, from the medians, for f32 weights and
/// for f16 weights when they were timed.  Then the ratio of f16 to f32 for
/// the best kernel per shape, over the whole chunk and over the linear
/// shapes only: the scores and P·v take their f32 timings in both formats,
/// so the whole-chunk ratio is nearer 1.
pub fn summary(m: &Measured) -> Vec<String> {
    let shapes = shapes();
    let flops: f64 = shapes.iter().map(|s| s.flops() * s.per_chunk as f64).sum();
    let medians = |table: &[Vec<Timing>]| -> Vec<Vec<f64>> {
        table
            .iter()
            .map(|row| row.iter().map(Timing::median).collect())
            .collect()
    };
    let best = |med: &[Vec<f64>], i: usize| {
        (0..TILES.len())
            .min_by(|&a, &b| med[i][a].total_cmp(&med[i][b]))
            .unwrap_or(0)
    };
    // Seconds per chunk of the shapes that `keep` passes, with kernel
    // `pick(shape)`.
    let total = |med: &[Vec<f64>], keep: &dyn Fn(&Shape) -> bool, pick: &dyn Fn(usize) -> usize| {
        shapes
            .iter()
            .enumerate()
            .filter(|(_, s)| keep(s))
            .map(|(i, s)| med[i][pick(i)] * s.per_chunk as f64)
            .sum::<f64>()
    };
    let all = |_: &Shape| true;
    let linear = |s: &Shape| matches!(s.kind, Kind::Linear(_));
    let line = |label: &str, secs: f64| {
        format!(
            "bench: {label}: {secs:.2} s of matmul per chunk ({:.2} TFLOP/s)",
            flops / secs / 1e12
        )
    };
    let mut out = Vec::new();
    let tables = std::iter::once((WeightFormat::F32, &m.f32))
        .chain(m.f16.as_ref().map(|t| (WeightFormat::F16, t)));
    let mut best_totals = Vec::new();
    for (format, table) in tables {
        let med = medians(table);
        let prefix = match format {
            WeightFormat::F32 => "",
            WeightFormat::F16 => "f16 weights, ",
        };
        for (t, tile) in TILES.iter().enumerate() {
            let secs = total(&med, &all, &|_| t);
            if secs.is_finite() {
                out.push(line(&format!("{prefix}all {}", tile.name), secs));
            }
        }
        let pick = |i: usize| best(&med, i);
        out.push(line(
            &format!("{prefix}best per shape"),
            total(&med, &all, &pick),
        ));
        for (i, s) in shapes.iter().enumerate() {
            if format == WeightFormat::F32 || linear(s) {
                let with = match format {
                    WeightFormat::F32 => "",
                    WeightFormat::F16 => " with f16 weights",
                };
                out.push(format!(
                    "bench: best for {}{with}: {}",
                    s.name,
                    TILES[pick(i)].name
                ));
            }
        }
        best_totals.push((total(&med, &all, &pick), total(&med, &linear, &pick)));
    }
    if let [(f32_all, f32_linear), (f16_all, f16_linear)] = best_totals[..] {
        for (label, f16, f32) in [
            ("whole chunk", f16_all, f32_all),
            ("linear shapes only", f16_linear, f32_linear),
        ] {
            out.push(format!(
                "bench: f16 / f32 weights, best kernel per shape, {label}: {:.3} (f16 {f16:.2} s, f32 {f32:.2} s per chunk)",
                f16 / f32
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The listed shapes add up to the chunk's 6.1 TFLOP of matmuls within
    /// about 5% (full-size frequency steps and one mask width overcount it).
    #[test]
    fn timing_median_and_spread() {
        let t = Timing {
            rounds: vec![3.0, 1.0, 2.0],
        };
        assert_eq!(t.median(), 2.0);
        assert!((t.spread() - 1.0).abs() < 1e-12);
        let t = Timing {
            rounds: vec![4.0, 2.0],
        };
        assert_eq!(t.median(), 3.0);
        assert!(Timing { rounds: vec![] }.median().is_infinite());
    }

    #[test]
    fn kernel_lists() {
        assert_eq!(parse_kernels("1"), None);
        assert_eq!(parse_kernels(""), None);
        assert_eq!(parse_kernels("i64,i128x64,s64,v64"), Some(vec![0, 1, 2, 3]));
        assert_eq!(parse_kernels("i64,x64"), None);
    }

    /// Seconds of split, the tiled fused kernel and `vfused`.
    fn secs(split: f64, fused: f64, vfused: f64) -> TimeSecs {
        TimeSecs {
            split,
            fused,
            vfused,
        }
    }

    /// Least total per kind; fused attention on the frequency axis when it
    /// works, on the time axis unless slower; nothing fused when the fused
    /// kernel failed its check.
    #[test]
    fn decisions() {
        let inf = f64::INFINITY;
        let c = decide(&[2.0, 1.0], &[1.0, 3.0], secs(5.0, 4.0, inf), true, false);
        assert_eq!(c.matmul, MatMulPolicy::Split { large: 1, small: 0 });
        assert_eq!(c.attention, AttentionPolicy::Fused);
        let c = decide(&[inf, 1.0], &[inf, 3.0], secs(4.0, 5.0, inf), true, false);
        assert_eq!(c.matmul, MatMulPolicy::Split { large: 1, small: 1 });
        assert_eq!(c.attention, AttentionPolicy::Freq);
        let c = decide(&[1.0, 2.0], &[1.0, 2.0], secs(5.0, 1.0, inf), false, false);
        assert_eq!(c.attention, AttentionPolicy::Split);
        assert_eq!(Choice::parse(&c.name()), Some(c));
        assert_eq!(
            Choice::parse("i128x64,i64;freq").map(|c| c.name()),
            Some("i128x64,i64;freq".to_string())
        );
        assert_eq!(Choice::parse("x64;split"), None);
    }

    /// Asserts how `decide` takes `vfused`:
    /// * faster than the tiled kernel, and not slower than split by more
    ///   than `FUSED_MARGIN`: `vfused`;
    /// * slower than the tiled kernel, or as fast: the tiled kernel
    ///   (`fused`);
    /// * both fused kernels slower than split by more than `FUSED_MARGIN`:
    ///   `freq`, whichever is faster;
    /// * `vfused` failed its check: `fused` or `freq`, by the tiled
    ///   kernel's time alone;
    /// * the tiled kernel failed its check: `split`, also with `vfused`
    ///   fast and checked;
    /// * a stored `vfused` choice parses back.
    #[test]
    fn decide_takes_the_faster_fused_kernel() {
        let attention = |t: TimeSecs, fused_ok: bool, vfused_ok: bool| {
            decide(&[1.0], &[1.0], t, fused_ok, vfused_ok).attention
        };
        assert_eq!(
            attention(secs(5.0, 4.0, 3.0), true, true),
            AttentionPolicy::VFused
        );
        assert_eq!(
            attention(secs(3.0, 4.0, 3.0), true, true),
            AttentionPolicy::VFused
        );
        assert_eq!(
            attention(secs(5.0, 3.0, 4.0), true, true),
            AttentionPolicy::Fused
        );
        assert_eq!(
            attention(secs(5.0, 3.0, 3.0), true, true),
            AttentionPolicy::Fused
        );
        assert_eq!(
            attention(secs(2.0, 4.0, 3.0), true, true),
            AttentionPolicy::Freq
        );
        assert_eq!(
            attention(secs(2.0, 3.0, 4.0), true, true),
            AttentionPolicy::Freq
        );
        assert_eq!(
            attention(secs(5.0, 4.0, 3.0), true, false),
            AttentionPolicy::Fused
        );
        assert_eq!(
            attention(secs(3.5, 4.0, 3.0), true, false),
            AttentionPolicy::Freq
        );
        assert_eq!(
            attention(secs(5.0, 4.0, 3.0), false, true),
            AttentionPolicy::Split
        );
        let c = decide(&[1.0], &[1.0], secs(5.0, 4.0, 3.0), true, true);
        assert_eq!(c.name(), "i64,i64;vfused");
        assert_eq!(Choice::parse("i64,i64;vfused"), Some(c));
    }

    /// Asserts that calibration times the time axis with a wait after each
    /// step, as the separation does: `STEP_WAITS` is `EachStep`, which
    /// queues 1 submission for each wait however short a step is.
    /// `Covering` still queues enough to cover `WAIT_SECS` (50 steps of
    /// 1 ms, at most 64).
    #[test]
    fn calibration_waits_after_each_step() {
        assert_eq!(STEP_WAITS, Waits::EachStep);
        for one in [1e-6, 1e-3, 0.02, 1.0] {
            assert_eq!(queued(STEP_WAITS, one), 1, "{one}");
        }
        assert_eq!(queued(Waits::Covering, 1e-3), 50);
        assert_eq!(queued(Waits::Covering, 1e-6), 64);
        assert_eq!(queued(Waits::Covering, 1.0), 1);
    }

    /// Asserts that the two paths of the worker that take an attention
    /// policy, `?attn=` (`apply_attention`) and a stored choice
    /// (`choose_kernels`), each take it only when it is usable on this GPU
    /// (`AttentionPolicy::usable`, with both fused kernels' checks), and
    /// that `?attn=`'s message names `vfused`.
    #[test]
    fn the_worker_takes_a_policy_only_when_its_kernels_passed() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../worker/src/web.rs");
        let worker = std::fs::read_to_string(&path).expect("worker/src/web.rs");
        for want in [
            "Some(p) if p.usable(gpu.fused_ok.get(), gpu.vfused_ok.get()) => gpu.attention.set(p),",
            "&& c.attention.usable(gpu.fused_ok.get(), gpu.vfused_ok.get())",
            "(auto, split, fused, time, freq or vfused)",
        ] {
            assert!(worker.contains(want), "{want} in worker/src/web.rs");
        }
        assert!(
            !worker.contains("AttentionPolicy::SPLIT || gpu.fused_ok"),
            "no path takes a policy on the tiled kernel's check alone"
        );
    }

    /// Asserts that the worker and calibration use one budget: the
    /// worker's `budget` falls back to `net::BUDGET`, the worker has no
    /// budget of its own, and it gives calibration the budget of the run.
    #[test]
    fn the_run_and_calibration_share_one_budget() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../worker/src/web.rs");
        let worker = std::fs::read_to_string(&path).expect("worker/src/web.rs");
        for want in [
            "s.max_elems.unwrap_or(roformer_gpu::net::BUDGET)",
            "choose_kernels(g, format, budget(s))",
            "roformer_gpu::bench::calibrate(gpu, format, budget, ",
        ] {
            assert!(worker.contains(want), "{want} in worker/src/web.rs");
        }
        assert!(
            !worker.contains("const BUDGET"),
            "no budget of the worker's own"
        );
    }

    /// Asserts that the time axis takes the faster policy, with no margin
    /// for fused attention (`FUSED_MARGIN` 1.0, M5): a tie keeps fused, and
    /// split 1 % faster or more takes the time axis.  Chrome on the M2 Pro
    /// (calibration 5: split 3.05 s, fused 3.11 s) splits, and Safari (split
    /// 5.03 s, fused 4.72 s) keeps fused.
    #[test]
    fn fused_attention_margin() {
        let time = |split: f64, fused: f64| {
            let t = secs(split, fused, f64::INFINITY);
            decide(&[1.0], &[1.0], t, true, false)
                .attention
                .time()
                .is_some()
        };
        assert!(time(1.0, 1.0));
        assert!(time(1.0, FUSED_MARGIN));
        assert!(time(1.0, 0.99));
        assert!(!time(1.0, 1.01));
        assert!(!time(1.0, 1.09));
        assert!(!time(3.05, 3.11));
        assert!(time(5.03, 4.72));
        assert!(!time(4.0, 5.0));
        // The margin alone did not rescue the Radeon Pro 5500M's
        // version-1 measurement; the longer windows had to.
        assert!(!time(8.26, 9.51));
        // Its bench a minute later: fused.
        assert!(time(11.46, 4.52));
    }

    /// First rounds 1.0 and 1.6 drop the second kernel; the survivor's
    /// later rounds slow to 1.8, yet the dropped kernel cannot win.
    #[test]
    fn dropped_kernels_cannot_win() {
        let t = [
            Timing {
                rounds: vec![1.0, 1.8, 1.8],
            },
            Timing { rounds: vec![1.6] },
        ];
        let alive = still_in(&[1.0, 1.6]);
        let m = masked_medians(&t, &alive);
        assert_eq!(m, vec![1.8, f64::INFINITY]);
        let c = decide(&m, &m, secs(1.0, 2.0, f64::INFINITY), true, false);
        assert_eq!(c.matmul, MatMulPolicy::Split { large: 0, small: 0 });
    }

    /// Other GPUs' lines stay; this GPU keeps its two newest other lines
    /// and the new one, last; a line for the same key is replaced.
    #[test]
    fn stored_choices() {
        let text = "amd|a|2\ti64;split\nintel|a|2\ti128x64;split\namd|b|2\ts64;fused\namd|c|1\ti64;freq\nno tab\n";
        assert_eq!(
            store_choice(text, "amd", "amd|d|2", "v64;fused", 3),
            "intel|a|2\ti128x64;split\namd|b|2\ts64;fused\namd|c|1\ti64;freq\namd|d|2\tv64;fused\n"
        );
        assert_eq!(
            store_choice(text, "amd", "amd|b|2", "v64;fused", 3),
            "amd|a|2\ti64;split\nintel|a|2\ti128x64;split\namd|c|1\ti64;freq\namd|b|2\tv64;fused\n"
        );
        assert_eq!(
            store_choice(text, "amd", "amd|d|2", "v64;fused", 1),
            "intel|a|2\ti128x64;split\namd|d|2\tv64;fused\n"
        );
        assert_eq!(
            store_choice("", "amd", "amd|d|2", "v64;freq", 3),
            "amd|d|2\tv64;freq\n"
        );
        // An adapter whose name is a prefix of another's keeps its lines
        // apart ("amd" against "amd rdna").
        assert_eq!(
            store_choice("amd rdna|a|2\ti64;split\n", "amd", "amd|d|2", "v64;freq", 1),
            "amd rdna|a|2\ti64;split\namd|d|2\tv64;freq\n"
        );
    }

    /// The two formats' keys differ only in the format, and name the
    /// calibration version: 6, which times `vfused` too, so a choice of
    /// version 5 or older is measured again and never parses as `vfused`.
    #[test]
    fn choice_keys_name_the_format() {
        assert_eq!(CALIBRATION, 6);
        assert_eq!(
            choice_key("AMD Radeon Pro 5500M", 0xab, WeightFormat::F32),
            "AMD Radeon Pro 5500M|00000000000000ab|6|f32"
        );
        assert_eq!(
            choice_key("AMD Radeon Pro 5500M", 0xab, WeightFormat::F16),
            "AMD Radeon Pro 5500M|00000000000000ab|6|f16"
        );
    }

    /// The kernels line names the choice, where it came from and the format.
    #[test]
    fn the_kernels_line_names_the_choice_and_the_format() {
        let choice = Choice::parse("v64,i64;fused").unwrap();
        assert_eq!(
            kernels_line(choice, "stored", WeightFormat::F16),
            "kernels: matmul v64,i64, attention fused (stored; f16 weights)"
        );
        assert_eq!(
            kernels_line(
                Choice::parse("s64;split").unwrap(),
                "from the URL",
                WeightFormat::F32
            ),
            "kernels: matmul s64, attention split (from the URL; f32 weights)"
        );
    }

    /// A worker chooses again when it has not chosen yet, or when its last
    /// choice was for the other format.
    #[test]
    fn a_load_of_the_other_format_chooses_again() {
        use WeightFormat::{F16, F32};
        assert!(choose_again(None, F32));
        assert!(choose_again(None, F16));
        assert!(!choose_again(Some(F32), F32));
        assert!(!choose_again(Some(F16), F16));
        assert!(choose_again(Some(F32), F16));
        assert!(choose_again(Some(F16), F32));
    }

    /// With f32 and f16 keys, a keep of 6 holds three builds' choices of
    /// both formats for one GPU.
    #[test]
    fn stored_choices_of_both_formats() {
        let mut text = String::new();
        for build in ["a", "b", "c", "d"] {
            for format in [WeightFormat::F32, WeightFormat::F16] {
                let key = format!("amd|{build}|3|{}", format.name());
                text = store_choice(&text, "amd", &key, "v64;fused", 6);
            }
        }
        let keys: Vec<&str> = text
            .lines()
            .map(|l| l.split('\t').next().unwrap())
            .collect();
        assert_eq!(
            keys,
            [
                "amd|b|3|f32",
                "amd|b|3|f16",
                "amd|c|3|f32",
                "amd|c|3|f16",
                "amd|d|3|f32",
                "amd|d|3|f16"
            ]
        );
    }

    fn timing(secs: f64) -> Timing {
        Timing { rounds: vec![secs] }
    }

    /// From a planted table: each format's totals, and the ratio of f16 to
    /// f32 over the whole chunk and over the linear shapes only.  The
    /// scores and P·v take their f32 timings in both formats.
    #[test]
    fn summary_gives_the_ratio_of_f16_to_f32() {
        let shapes = shapes();
        let row = |linear: f64, other: f64| -> Vec<Vec<Timing>> {
            shapes
                .iter()
                .map(|s| {
                    let secs = match s.kind {
                        Kind::Linear(_) => linear,
                        _ => other,
                    };
                    // The second kernel is the best everywhere; the others
                    // did not run.
                    let mut row = vec![Timing { rounds: Vec::new() }; TILES.len()];
                    row[0] = timing(2.0 * secs);
                    row[1] = timing(secs);
                    row
                })
                .collect()
        };
        let lines = summary(&Measured {
            f32: row(1e-3, 1e-3),
            f16: Some(row(0.9e-3, 1e-3)),
        });
        let per_chunk = |pick: &dyn Fn(&Kind) -> bool| -> usize {
            shapes
                .iter()
                .filter(|s| pick(&s.kind))
                .map(|s| s.per_chunk)
                .sum()
        };
        let linear = per_chunk(&|k| matches!(k, Kind::Linear(_))) as f64 * 1e-3;
        let other = per_chunk(&|k| !matches!(k, Kind::Linear(_))) as f64 * 1e-3;
        let whole = format!(
            "bench: f16 / f32 weights, best kernel per shape, whole chunk: {:.3} (f16 {:.2} s, f32 {:.2} s per chunk)",
            (0.9 * linear + other) / (linear + other),
            0.9 * linear + other,
            linear + other
        );
        let linear_only = format!(
            "bench: f16 / f32 weights, best kernel per shape, linear shapes only: {:.3} (f16 {:.2} s, f32 {:.2} s per chunk)",
            0.9,
            0.9 * linear,
            linear
        );
        assert!(lines.contains(&whole), "{lines:#?}");
        assert!(lines.contains(&linear_only), "{lines:#?}");
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with("bench: f16 weights, best per shape: ")),
            "{lines:#?}"
        );
        // A best kernel for every shape with f32 weights, and with f16
        // weights for each linear shape only: the others have no weights.
        let linear_names: Vec<&str> = shapes
            .iter()
            .filter(|s| matches!(s.kind, Kind::Linear(_)))
            .map(|s| s.name)
            .collect();
        for s in &shapes {
            let f32_line = format!("bench: best for {}: i128x64", s.name);
            assert!(lines.contains(&f32_line), "{f32_line}: {lines:#?}");
            let f16_line = format!("bench: best for {} with f16 weights: i128x64", s.name);
            assert_eq!(
                lines.contains(&f16_line),
                linear_names.contains(&s.name),
                "{f16_line}: {lines:#?}"
            );
        }
        let f32_only = summary(&Measured {
            f32: row(1e-3, 1e-3),
            f16: None,
        });
        assert!(f32_only.iter().all(|l| !l.contains("f16")), "{f32_only:#?}");
        assert!(
            f32_only
                .iter()
                .any(|l| l.starts_with("bench: best per shape: "))
        );
    }

    #[test]
    fn elimination() {
        let inf = f64::INFINITY;
        assert_eq!(still_in(&[1.0, 1.4, 6.0]), vec![true, true, false]);
        assert_eq!(still_in(&[inf, 2.0, 2.9]), vec![false, true, true]);
    }

    #[test]
    fn shapes_cover_a_chunk() {
        let total: f64 = shapes()
            .iter()
            .map(|s| s.flops() * s.per_chunk as f64)
            .sum();
        assert!((5.9e12..6.5e12).contains(&total), "{total:e}");
    }
}
