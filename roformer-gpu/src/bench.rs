//! Matmul throughput on this GPU for every kernel in [`TILES`], on the
//! shapes one full chunk (1,151 frames) actually uses.  Run in the browser
//! with `?bench`.  [`calibrate`] is the short version every GPU runs once,
//! before its first separation, to choose its kernels; nothing is tuned
//! while a song is being separated.

use crate::{Act, AttentionPolicy, Gpu, GpuBuffer, MatMul, MatMulPolicy, Rec, TILES, View};

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
    kind: Kind,
}

#[derive(Clone, Copy, Debug)]
enum Kind {
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
    weight: GpuBuffer,
    qkv: GpuBuffer,
    out: GpuBuffer,
}

fn matmul<'a>(s: &Shape, b: &'a Buffers, tile: usize) -> MatMul<'a> {
    let row = 1536;
    let mut mm = match s.kind {
        Kind::Linear(act) => {
            let mut mm = MatMul::new(
                (&b.act, View::rows(0, s.k).batch(s.m * s.k, 0)),
                (&b.weight, View::t(0, s.k).batch(s.n * s.k, 0)),
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

fn buffers(gpu: &Gpu) -> Buffers {
    Buffers {
        act: gpu.buffer("bench act", FREQ_TOKENS * 1024),
        weight: gpu.buffer("bench weight", 7 * 1024 * 256),
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
async fn time_one(
    gpu: &Gpu,
    b: &Buffers,
    s: &Shape,
    tile: usize,
    warm: f64,
    round: f64,
    now: &impl Fn() -> f64,
) -> f64 {
    let mm = matmul(s, b, tile);
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
    let queued = ((WAIT_SECS / one.max(1e-6)).ceil() as usize).clamp(1, 64);
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
#[derive(Clone, Debug)]
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

/// Seconds per dispatch of each shape with each kernel (`[shape][tile]`,
/// the median of `rounds` rounds; infinite for kernels that failed
/// [`Gpu::check_kernels`] or are not in `only`).  The kernels take turns
/// within each round (A, B, A, B, ...), so a GPU that heats up and slows
/// down over the run slows every kernel alike instead of whichever ran
/// last.  `now` is a clock in seconds.  With `warm` 0.3, `round` 0.2 and
/// 3 rounds it takes a few minutes on a real GPU.
pub async fn measure(
    gpu: &Gpu,
    warm: f64,
    round: f64,
    rounds: usize,
    only: Option<&[usize]>,
    now: impl Fn() -> f64,
    mut log: impl FnMut(String),
) -> Vec<Vec<Timing>> {
    let recording = gpu.set_recording(false);
    let b = buffers(gpu);
    let run: Vec<bool> = (0..TILES.len())
        .map(|t| gpu.usable.get()[t] && only.is_none_or(|o| o.contains(&t)))
        .collect();
    let mut table = Vec::new();
    for s in &shapes() {
        // What runs when each kernel is asked for: a vec4 kernel hands
        // products it cannot read as vec4s to the fallback.  The fallback is
        // timed once for both (also when it was not asked for itself) and
        // its timings stand for the vec4 kernel.
        let ran: Vec<usize> = (0..TILES.len())
            .map(|t| matmul(s, &b, t).kernel(t))
            .collect();
        let timed: Vec<bool> = (0..TILES.len())
            .map(|u| (0..TILES.len()).any(|t| run[t] && ran[t] == u))
            .collect();
        let mut row = vec![Timing { rounds: Vec::new() }; TILES.len()];
        for r in 0..rounds.max(1) {
            for (t, timing) in row.iter_mut().enumerate() {
                if !timed[t] {
                    continue;
                }
                let w = if r == 0 { warm } else { 0.0 };
                timing
                    .rounds
                    .push(time_one(gpu, &b, s, t, w, round, &now).await);
            }
        }
        for (t, tile) in TILES.iter().enumerate() {
            if !gpu.usable.get()[t] {
                log(format!(
                    "bench: {} {}: skipped (failed the start-up check on this GPU)",
                    s.name, tile.name
                ));
            } else if run[t] && ran[t] != t {
                row[t] = row[ran[t]].clone();
                log(format!(
                    "bench: {} {}: runs {}, {:.1} GFLOP/s (not vec4-aligned: an operand not K-contiguous, or K, a stride or an offset not a multiple of 4)",
                    s.name,
                    tile.name,
                    TILES[ran[t]].name,
                    s.flops() / row[t].median() / 1e9
                ));
            } else if run[t] {
                let tm = &row[t];
                log(format!(
                    "bench: {} ({}x{}x{}, x{}) {}: {:.1} GFLOP/s (median of {}, spread {:.1}%)",
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
        table.push(row);
    }
    if let Some(e) = gpu.take_errors().into_iter().next() {
        log(format!("bench: WebGPU error: {e}"));
    }
    gpu.set_recording(recording);
    table
}

/// Seconds per call of `record` (which adds dispatches to one submission),
/// timed like [`time_one`]: `warm` seconds first, then one round of at
/// least `round` seconds with enough submissions per wait to cover
/// [`WAIT_SECS`].
async fn time_rec<'g>(
    gpu: &'g Gpu,
    warm: f64,
    round: f64,
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
    let queued = ((WAIT_SECS / one.max(1e-6)).ceil() as usize).clamp(1, 64);
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

fn fused_attention<'b>(r: &mut Rec<'b>, b: &'b AttnBufs, seq: usize, nseq: usize) {
    r.attention(&b.qkv, Some(&b.gates), &b.o, seq, nseq, 0.125);
}

/// Seconds per chunk of split and fused attention on each axis, the way
/// a chunk runs them at the default budget: on the time axis split takes
/// one band of 1,151 frames per step (744 steps) and fused four (186
/// steps; it needs no scores buffer); on the frequency axis both take 88
/// frames of 62 bands per step (168 steps).  The two take turns over
/// `rounds` rounds; medians.  Returns ((split, fused) time axis, (split,
/// fused) frequency axis); fused is infinite when it failed its check.
/// Without `freq` the frequency axis is not timed (its timings are empty):
/// calibration uses fused attention there whenever it works.
pub async fn attention_per_chunk(
    gpu: &Gpu,
    warm: f64,
    round: f64,
    rounds: usize,
    freq: bool,
    now: &impl Fn() -> f64,
) -> ((Timing, Timing), (Timing, Timing)) {
    let recording = gpu.set_recording(false);
    let b = attn_bufs(gpu);
    let empty = || Timing { rounds: Vec::new() };
    let (mut ts, mut tf, mut fs, mut ff) = (empty(), empty(), empty(), empty());
    for rd in 0..rounds.max(1) {
        let w = if rd == 0 { warm } else { 0.0 };
        ts.rounds
            .push(time_rec(gpu, w, round, now, &|r| split_attention(r, &b, T, 1)).await * 744.0);
        if freq {
            fs.rounds.push(
                time_rec(gpu, w, round, now, &|r| split_attention(r, &b, 62, 88)).await * 168.0,
            );
        }
        if gpu.fused_ok.get() {
            tf.rounds.push(
                time_rec(gpu, w, round, now, &|r| fused_attention(r, &b, T, 4)).await * 186.0,
            );
            if freq {
                ff.rounds.push(
                    time_rec(gpu, w, round, now, &|r| fused_attention(r, &b, 62, 88)).await * 168.0,
                );
            }
        }
    }
    gpu.set_recording(recording);
    ((ts, tf), (fs, ff))
}

/// Logs [`attention_per_chunk`] and returns its medians.
pub async fn measure_attention(
    gpu: &Gpu,
    warm: f64,
    round: f64,
    rounds: usize,
    now: impl Fn() -> f64,
    mut log: impl FnMut(String),
) -> ((f64, f64), (f64, f64)) {
    let ((ts, tf), (fs, ff)) = attention_per_chunk(gpu, warm, round, rounds, true, &now).await;
    for (axis, split, fused) in [("time", &ts, &tf), ("freq", &fs, &ff)] {
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
        ts.median() + fs.median(),
        tf.median() + ff.median()
    ));
    ((ts.median(), tf.median()), (fs.median(), ff.median()))
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
pub const CALIBRATION: u32 = 2;

/// Warm-up before the first timed round of each attention candidate in
/// [`calibrate`], and the length of every timed round: a time-axis step
/// takes tens of milliseconds, so 0.1 s held only a few of them.
pub const ATTENTION_WARM: f64 = 0.5;
pub const ATTENTION_ROUND: f64 = 0.3;

/// Fused attention stays on the time axis unless split attention measured
/// more than this factor faster.  Fused needs no scores buffer (about
/// 42 MB per band) and so runs four bands per step, which matters more
/// on memory-limited devices than a close timing does.
pub const FUSED_MARGIN: f64 = 1.1;

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

/// The choice from measured seconds: for each kind of product, the kernel
/// with the least total over its shapes (`large[t]`, `small[t]`; infinite
/// for kernels that failed their check); fused attention on the frequency
/// axis whenever it works (it was 2.2 to 3.6 times faster on every GPU
/// measured), and on the time axis unless split measured faster by more
/// than [`FUSED_MARGIN`].
pub fn decide(
    large: &[f64],
    small: &[f64],
    time_split: f64,
    time_fused: f64,
    fused_ok: bool,
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
        attention: AttentionPolicy {
            time: fused_ok && time_fused <= FUSED_MARGIN * time_split,
            freq: fused_ok,
        },
    }
}

/// Chooses this GPU's kernels, once per GPU (the caller keeps the result):
/// the matmul kernel for large products (q/k/v and feed-forward shapes)
/// and for small ones (attention gates, the p.v product of split
/// attention), then whether fused attention beats split attention on the
/// time axis with that matmul choice.  Everything measured takes turns
/// over three rounds, medians compared; a matmul kernel more than 1.5
/// times slower than the best after the first round is not timed again
/// ([`still_in`]).  What is measured is only what varies between GPUs and
/// cannot be read from WebGPU: whether the 8 x 4 or the 4 x 4 register
/// block wins (register file and scheduling), whether the compiler keeps
/// `s64`'s loop-indexed accumulators in registers, whether vec4 loads
/// (`v64`) issue faster than scalar ones, and whether fused attention wins
/// on long sequences.  A product `v64` cannot read as vec4s (the p.v
/// product) runs and is timed as `i64` under `v64`'s name, as it would
/// run in a separation.  Attention gets longer windows
/// ([`ATTENTION_WARM`], [`ATTENTION_ROUND`]) than the matmul products.
/// Takes several seconds.
pub async fn calibrate(gpu: &Gpu, now: impl Fn() -> f64, mut log: impl FnMut(String)) -> Choice {
    let all = shapes();
    let by_name = |n: &str| all.iter().find(|s| s.name == n).copied().expect("shape");
    let large = [
        by_name("time qkv"),
        by_name("time ff down"),
        by_name("freq ff up"),
    ];
    let small = [by_name("time gates"), by_name("time p·v")];
    let b = buffers(gpu);
    let (warm, round, rounds) = (0.1, 0.1, 3);
    let secs = |kind: Vec<Shape>| {
        let b = &b;
        let now = &now;
        async move {
            let mut t = vec![Timing { rounds: Vec::new() }; TILES.len()];
            #[expect(
                clippy::disallowed_methods,
                reason = "MEM-1: a short list of kernel choices"
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
                        total += time_one(gpu, b, s, k, w, round, now).await * s.per_chunk as f64;
                    }
                    timing.rounds.push(total);
                }
            }
            (masked_medians(&t, &alive), t)
        }
    };
    let recording = gpu.set_recording(false);
    #[expect(
        clippy::disallowed_methods,
        reason = "MEM-1: a short list of kernel choices"
    )]
    let (l, lt) = secs(large.to_vec()).await;
    #[expect(
        clippy::disallowed_methods,
        reason = "MEM-1: a short list of kernel choices"
    )]
    let (s, st) = secs(small.to_vec()).await;
    let matmul = decide(&l, &s, 0.0, 0.0, false).matmul;
    gpu.policy.set(matmul);
    let ((ts, tf), _) =
        attention_per_chunk(gpu, ATTENTION_WARM, ATTENTION_ROUND, rounds, false, &now).await;
    let choice = decide(&l, &s, ts.median(), tf.median(), gpu.fused_ok.get());
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
        "kernels for this GPU: matmul {} (large products: {}; small: {}), attention {} (time axis: split {:.2} s (spread {:.1}%), fused {:.2} s (spread {:.1}%) per chunk)",
        choice.matmul.name(),
        fmt(&l, &lt),
        fmt(&s, &st),
        choice.attention.name(),
        ts.median(),
        100.0 * ts.spread(),
        tf.median(),
        100.0 * tf.spread()
    ));
    choice
}

/// Summary lines: estimated matmul seconds per chunk for each kernel alone
/// and for the best kernel per shape, from the medians.
pub fn summary(table: &[Vec<Timing>]) -> Vec<String> {
    let shapes = shapes();
    let flops: f64 = shapes.iter().map(|s| s.flops() * s.per_chunk as f64).sum();
    let med: Vec<Vec<f64>> = table
        .iter()
        .map(|row| row.iter().map(Timing::median).collect())
        .collect();
    let total = |pick: &dyn Fn(usize) -> usize| -> f64 {
        shapes
            .iter()
            .enumerate()
            .map(|(i, s)| med[i][pick(i)] * s.per_chunk as f64)
            .sum()
    };
    let mut out = Vec::new();
    let line = |label: &str, secs: f64| {
        format!(
            "bench: {label}: {secs:.2} s of matmul per chunk ({:.2} TFLOP/s)",
            flops / secs / 1e12
        )
    };
    for (t, tile) in TILES.iter().enumerate() {
        let secs = total(&|_| t);
        if secs.is_finite() {
            out.push(line(&format!("all {}", tile.name), secs));
        }
    }
    let best = |i: usize| {
        (0..TILES.len())
            .min_by(|&a, &b| med[i][a].total_cmp(&med[i][b]))
            .unwrap_or(0)
    };
    out.push(line("best per shape", total(&best)));
    for (i, s) in shapes.iter().enumerate() {
        out.push(format!(
            "bench: best for {}: {}",
            s.name,
            TILES[best(i)].name
        ));
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

    /// Least total per kind; fused attention on the frequency axis when it
    /// works, on the time axis unless clearly slower; nothing fused when
    /// the fused kernel failed its check.
    #[test]
    fn decisions() {
        let inf = f64::INFINITY;
        let c = decide(&[2.0, 1.0], &[1.0, 3.0], 5.0, 4.0, true);
        assert_eq!(c.matmul, MatMulPolicy::Split { large: 1, small: 0 });
        assert_eq!(c.attention, AttentionPolicy::FUSED);
        let c = decide(&[inf, 1.0], &[inf, 3.0], 4.0, 5.0, true);
        assert_eq!(c.matmul, MatMulPolicy::Split { large: 1, small: 1 });
        assert_eq!(
            c.attention,
            AttentionPolicy {
                time: false,
                freq: true
            }
        );
        let c = decide(&[1.0, 2.0], &[1.0, 2.0], 5.0, 1.0, false);
        assert_eq!(c.attention, AttentionPolicy::SPLIT);
        assert_eq!(Choice::parse(&c.name()), Some(c));
        assert_eq!(
            Choice::parse("i128x64,i64;freq").map(|c| c.name()),
            Some("i128x64,i64;freq".to_string())
        );
        assert_eq!(Choice::parse("x64;split"), None);
    }

    /// Fused attention keeps the time axis when split is faster by
    /// [`FUSED_MARGIN`] or less, and loses it beyond that.
    #[test]
    fn fused_attention_margin() {
        let time =
            |split: f64, fused: f64| decide(&[1.0], &[1.0], split, fused, true).attention.time;
        assert!(time(1.0, 1.0));
        assert!(time(1.0, 1.09));
        assert!(time(1.0, FUSED_MARGIN));
        assert!(!time(1.0, 1.11));
        assert!(!time(4.0, 5.0));
        // The margin alone does not rescue the Radeon Pro 5500M's
        // version-1 measurement; the longer windows have to.
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
        let c = decide(&m, &m, 1.0, 2.0, true);
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
