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
    let lin = |name, m, n, k, outer, per_chunk, act| Shape { name, m, n, k, outer, inner: 1, per_chunk, kind: Kind::Linear(act) };
    vec![
        lin("time qkv", T, 1536, 256, 1, 744, Act::None),
        lin("time gates", T, 8, 256, 1, 744, Act::None),
        Shape { name: "time scores", m: T, n: T, k: 64, outer: 1, inner: 8, per_chunk: 744, kind: Kind::Scores },
        Shape { name: "time p·v", m: T, n: 64, k: T, outer: 1, inner: 8, per_chunk: 744, kind: Kind::Values },
        lin("time attn out", T, 256, 512, 1, 744, Act::None),
        lin("time ff up", T, 1024, 256, 1, 744, Act::Gelu),
        lin("time ff down", T, 256, 1024, 1, 744, Act::None),
        lin("freq qkv", FREQ_TOKENS, 1536, 256, 1, 168, Act::None),
        Shape { name: "freq scores", m: 62, n: 62, k: 64, outer: 88, inner: 8, per_chunk: 168, kind: Kind::Scores },
        Shape { name: "freq p·v", m: 62, n: 64, k: 62, outer: 88, inner: 8, per_chunk: 168, kind: Kind::Values },
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
                (&b.qkv, View { off: 0, rs: row, cs: 1, bo: seq * row, bi: 64 }),
                (&b.qkv, View { off: 512, rs: 1, cs: row, bo: seq * row, bi: 64 }),
                (&b.out, View { off: 0, rs: seq, cs: 1, bo: 8 * seq * seq, bi: seq * seq }),
                seq,
                seq,
                64,
            )
        }
        Kind::Values => {
            let seq = s.m;
            MatMul::new(
                (&b.out, View { off: 0, rs: seq, cs: 1, bo: 8 * seq * seq, bi: seq * seq }),
                (&b.qkv, View { off: 1024, rs: row, cs: 1, bo: seq * row, bi: 64 }),
                (&b.act, View { off: 0, rs: 512, cs: 1, bo: seq * 512, bi: 64 }),
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
/// load), then two rounds of at least `round` seconds, keeping the faster.
/// Submissions hold at most 4 GFLOP, so no command buffer runs long even on
/// a slow GPU.
async fn time_one(gpu: &Gpu, b: &Buffers, s: &Shape, tile: usize, warm: f64, round: f64, now: &impl Fn() -> f64) -> f64 {
    let mm = matmul(s, b, tile);
    let per_submit = ((4e9 / s.flops()) as usize).clamp(1, 32);
    let submit = async || {
        let mut r = gpu.rec();
        for _ in 0..per_submit {
            r.matmul(&mm);
        }
        r.submit();
        gpu.done().await;
    };
    let t0 = now();
    loop {
        submit().await;
        if now() - t0 >= warm {
            break;
        }
    }
    let mut best = f64::INFINITY;
    for _ in 0..2 {
        let (t0, mut count) = (now(), 0);
        while count == 0 || now() - t0 < round {
            submit().await;
            count += per_submit;
        }
        best = best.min((now() - t0) / count as f64);
    }
    best
}

/// Seconds per dispatch of each shape with each kernel (`[shape][tile]`;
/// infinite for kernels that failed [`Gpu::check_kernels`]).  `now` is a
/// clock in seconds (the browser has no `std::time`).  With `warm` 0.3 and
/// `round` 0.2 it takes a few minutes on a real GPU.
pub async fn measure(gpu: &Gpu, warm: f64, round: f64, now: impl Fn() -> f64, mut log: impl FnMut(String)) -> Vec<Vec<f64>> {
    let b = buffers(gpu);
    let mut table = Vec::new();
    for s in &shapes() {
        let mut row = Vec::new();
        for (t, tile) in TILES.iter().enumerate() {
            if !gpu.usable.get()[t] {
                row.push(f64::INFINITY);
                log(format!("bench: {} {}: skipped (failed the start-up check on this GPU)", s.name, tile.name));
                continue;
            }
            let secs = time_one(gpu, &b, s, t, warm, round, &now).await;
            row.push(secs);
            log(format!(
                "bench: {} ({}x{}x{}, x{}) {}: {:.1} GFLOP/s",
                s.name,
                s.m,
                s.n,
                s.k,
                s.outer * s.inner,
                tile.name,
                s.flops() / secs / 1e9
            ));
        }
        table.push(row);
    }
    if let Some(e) = gpu.take_errors().into_iter().next() {
        log(format!("bench: WebGPU error: {e}"));
    }
    table
}

/// Chooses this GPU's matmul kernels, once per GPU: times every kernel
/// that passed [`Gpu::check_kernels`] on two large products (a full-chunk
/// q/k/v projection and the time-axis attention scores) and two small ones
/// (frequency-axis attention), and returns the fastest for each kind.  The
/// best kernel differs by GPU: on a Radeon Pro 5500M the original kernel
/// won every shape, on an Intel UHD 630 it was the slowest by 5x.
/// Takes a few seconds; the caller keeps the result per GPU.
pub async fn calibrate(gpu: &Gpu, now: impl Fn() -> f64, mut log: impl FnMut(String)) -> MatMulPolicy {
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
/// and for the best kernel per shape.
pub fn summary(table: &[Vec<f64>]) -> Vec<String> {
    let shapes = shapes();
    let flops: f64 = shapes.iter().map(|s| s.flops() * s.per_chunk as f64).sum();
    let total = |pick: &dyn Fn(usize) -> usize| -> f64 {
        shapes.iter().enumerate().map(|(i, s)| table[i][pick(i)] * s.per_chunk as f64).sum()
    };
    let mut out = Vec::new();
    let line = |label: &str, secs: f64| format!("bench: {label}: {secs:.2} s of matmul per chunk ({:.2} TFLOP/s)", flops / secs / 1e12);
    for (t, tile) in TILES.iter().enumerate() {
        out.push(line(&format!("all {}", tile.name), total(&|_| t)));
    }
    let best = |i: usize| (0..TILES.len()).min_by(|&a, &b| table[i][a].total_cmp(&table[i][b])).unwrap_or(0);
    out.push(line("best per shape", total(&best)));
    for (i, s) in shapes.iter().enumerate() {
        out.push(format!("bench: best for {}: {}", s.name, TILES[best(i)].name));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The listed shapes add up to the chunk's 6.1 TFLOP of matmuls within
    /// about 5% (full-size frequency steps and one mask width overcount it).
    #[test]
    fn shapes_cover_a_chunk() {
        let total: f64 = shapes().iter().map(|s| s.flops() * s.per_chunk as f64).sum();
        assert!((5.9e12..6.5e12).contains(&total), "{total:e}");
    }
}
