//! Matmul throughput on this GPU for every kernel in [`TILES`], on the
//! shapes one full chunk (1,151 frames) actually uses.  Run in the browser
//! with `?bench`.  [`calibrate`] is the short version every GPU runs once,
//! before its first separation, to choose its kernels; nothing is tuned
//! while a song is being separated.

use crate::{Act, Gpu, MatMul, MatMulPolicy, TILES, View};

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
    act: wgpu::Buffer,
    weight: wgpu::Buffer,
    qkv: wgpu::Buffer,
    out: wgpu::Buffer,
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
    let b = buffers(gpu);
    let run: Vec<bool> = (0..TILES.len())
        .map(|t| gpu.usable.get()[t] && only.is_none_or(|o| o.contains(&t)))
        .collect();
    let mut table = Vec::new();
    for s in &shapes() {
        let mut row = vec![Timing { rounds: Vec::new() }; TILES.len()];
        for r in 0..rounds.max(1) {
            for (t, timing) in row.iter_mut().enumerate() {
                if !run[t] {
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
        table.push(row);
    }
    if let Some(e) = gpu.take_errors().into_iter().next() {
        log(format!("bench: WebGPU error: {e}"));
    }
    table
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

/// Chooses this GPU's matmul kernels, once per GPU: times every kernel
/// that passed [`Gpu::check_kernels`] on two large products (a full-chunk
/// q/k/v projection and the time-axis attention scores) and two small ones
/// (frequency-axis attention), and returns the fastest for each kind.  The
/// best kernel differs by GPU: on a Radeon Pro 5500M the original kernel
/// won every shape, on an Intel UHD 630 it was the slowest by 5x.
/// Takes a few seconds; the caller keeps the result per GPU.
pub async fn calibrate(
    gpu: &Gpu,
    now: impl Fn() -> f64,
    mut log: impl FnMut(String),
) -> MatMulPolicy {
    let all = shapes();
    let by_name = |n: &str| all.iter().find(|s| s.name == n).copied().expect("shape");
    let large = [by_name("time qkv"), by_name("time scores")];
    let small = [by_name("freq scores"), by_name("freq p·v")];
    let b = buffers(gpu);
    let best = |kind: &[Shape]| {
        let kind = kind.to_vec();
        let b = &b;
        let now = &now;
        async move {
            let mut out = (0, f64::INFINITY);
            for t in 0..TILES.len() {
                if !gpu.usable.get()[t] {
                    continue;
                }
                let mut secs = 0.0;
                for s in &kind {
                    secs += time_one(gpu, b, s, t, 0.1, 0.1, now).await * s.per_chunk as f64;
                }
                if secs < out.1 {
                    out = (t, secs);
                }
            }
            out
        }
    };
    let (l, ls) = best(&large).await;
    let (s, ss) = best(&small).await;
    let policy = MatMulPolicy::Split { large: l, small: s };
    log(format!(
        "matmul kernels for this GPU: {} for large products ({ls:.1} s per chunk on the timed shapes), {} for small ones ({ss:.1} s)",
        TILES[l].name, TILES[s].name
    ));
    policy
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
        assert_eq!(parse_kernels("s64,v64"), Some(vec![0, 1]));
        assert_eq!(parse_kernels("s64,nope"), None);
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
