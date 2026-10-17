//! Each kernel against a plain CPU loop.  Runs on whatever adapter wgpu
//! finds; in CI-like runs without a GPU, point Vulkan at SwiftShader:
//! `VK_ICD_FILENAMES=/opt/pw-browsers/chromium-1194/chrome-linux/vk_swiftshader_icd.json WGPU_BACKEND=vulkan`.
//! Tests skip with a message when no adapter is available.

use roformer_gpu::{Act, Gpu, MatMul, TILES, View};

fn gpu() -> Option<Gpu> {
    match pollster::block_on(Gpu::new(wgpu::PowerPreference::HighPerformance)) {
        Ok(g) => {
            eprintln!("adapter: {} ({:?})", g.info.name, g.info.backend);
            Some(g)
        }
        Err(e) => {
            eprintln!("skipping: {e}");
            None
        }
    }
}

fn rand(n: usize, seed: u64) -> Vec<f32> {
    let mut s = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    (0..n)
        .map(|_| {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((s >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0
        })
        .collect()
}

fn read(g: &Gpu, b: &wgpu::Buffer, len: usize) -> Vec<f32> {
    pollster::block_on(g.read(b, 0, len)).unwrap()
}

fn assert_close(got: &[f32], want: &[f32], tol: f32, what: &str) {
    assert_eq!(got.len(), want.len(), "{what}: length");
    let mut worst = 0.0f32;
    for (i, (a, b)) in got.iter().zip(want).enumerate() {
        let e = (a - b).abs() / (1.0 + b.abs());
        assert!(e.is_finite(), "{what}: non-finite at {i}: {a} vs {b}");
        worst = worst.max(e);
        assert!(e <= tol, "{what}: index {i}: got {a}, want {b}");
    }
    eprintln!("{what}: worst relative error {worst:.2e}");
}

fn erf(x: f64) -> f64 {
    // The kernel uses Abramowitz and Stegun 7.1.26; this reference is much
    // tighter (Maclaurin series below 3, continued fraction for erfc above),
    // so the test checks the approximation too.
    let ax = x.abs();
    if ax < 3.0 {
        let (mut sum, mut term, mut n) = (0.0f64, x, 0.0f64);
        while n < 100.0 {
            sum += term / (2.0 * n + 1.0);
            n += 1.0;
            term *= -x * x / n;
        }
        sum * 2.0 / std::f64::consts::PI.sqrt()
    } else {
        let mut k = ax;
        for n in (1..80).rev() {
            k = ax + (n as f64 / 2.0) / k;
        }
        let erfc = (-ax * ax).exp() / std::f64::consts::PI.sqrt() / k;
        x.signum() * (1.0 - erfc)
    }
}

struct Spec {
    m: usize,
    n: usize,
    k: usize,
    outer: usize,
    inner: usize,
    a: View,
    b: View,
    c: View,
    bias: Option<(usize, usize)>,
    alpha: f32,
    act: Act,
    accumulate: bool,
}

fn cpu_matmul(s: &Spec, a: &[f32], b: &[f32], c: &mut [f32], bias: &[f32]) {
    for bo in 0..s.outer {
        for bi in 0..s.inner {
            for i in 0..s.m {
                for j in 0..s.n {
                    let mut acc = 0.0f64;
                    for k in 0..s.k {
                        let av = a[s.a.off + bo * s.a.bo + bi * s.a.bi + i * s.a.rs + k * s.a.cs];
                        let bv = b[s.b.off + bo * s.b.bo + bi * s.b.bi + k * s.b.rs + j * s.b.cs];
                        acc += av as f64 * bv as f64;
                    }
                    let mut v = acc * s.alpha as f64;
                    if let Some((off, bstride)) = s.bias {
                        v += bias[off + bo * bstride + j] as f64;
                    }
                    v = match s.act {
                        Act::None => v,
                        Act::Gelu => 0.5 * v * (1.0 + erf(v / 2f64.sqrt())),
                        Act::Tanh => v.tanh(),
                    };
                    let idx = s.c.off + bo * s.c.bo + bi * s.c.bi + i * s.c.rs + j * s.c.cs;
                    if s.accumulate {
                        v += c[idx] as f64;
                    }
                    c[idx] = v as f32;
                }
            }
        }
    }
}

/// Runs the product with every kernel in `TILES` and compares each with
/// the CPU.
fn check_matmul(g: &Gpu, s: Spec, sizes: [usize; 4], what: &str) {
    let (a, b, bias) = (rand(sizes[0], 1), rand(sizes[1], 2), rand(sizes[3], 4));
    let c0 = rand(sizes[2], 3);
    let mut want = c0.clone();
    cpu_matmul(&s, &a, &b, &mut want, &bias);
    for (t, tile) in TILES.iter().enumerate() {
        let (ab, bb, cb, biasb) = (
            g.upload("a", &a),
            g.upload("b", &b),
            g.upload("c", &c0),
            g.upload("bias", &bias),
        );
        let mut mm = MatMul::new((&ab, s.a), (&bb, s.b), (&cb, s.c), s.m, s.n, s.k);
        mm.outer = s.outer;
        mm.inner = s.inner;
        mm.alpha = s.alpha;
        mm.act = s.act;
        mm.accumulate = s.accumulate;
        mm.bias = s.bias.map(|(off, bo)| (&biasb, off, bo));
        mm.tile = Some(t);
        let mut r = g.rec();
        r.matmul(&mm);
        r.submit();
        let got = read(g, &cb, sizes[2]);
        let errors = g.take_errors();
        assert!(errors.is_empty(), "{errors:?}");
        assert_close(&got, &want, 2e-5, &format!("{what} [{}]", tile.name));
    }
}

#[test]
fn matmul_variants() {
    let Some(g) = gpu() else { return };
    // Plain row-major, sizes off the 64 and 16 tiles, bias and GELU.
    let (m, n, k) = (70, 130, 37);
    check_matmul(
        &g,
        Spec {
            m,
            n,
            k,
            outer: 1,
            inner: 1,
            a: View::rows(0, k),
            b: View::rows(0, n),
            c: View::rows(0, n),
            bias: Some((5, 0)),
            alpha: 1.0,
            act: Act::Gelu,
            accumulate: false,
        },
        [m * k, k * n, m * n, n + 5],
        "row-major, bias, gelu",
    );
    // Torch-layout weight read transposed, offsets, alpha, tanh, accumulate.
    let (m, n, k) = (129, 65, 300);
    check_matmul(
        &g,
        Spec {
            m,
            n,
            k,
            outer: 1,
            inner: 1,
            a: View::rows(3, k),
            b: View::t(7, k),
            c: View::rows(11, n),
            bias: None,
            alpha: 0.3,
            act: Act::Tanh,
            accumulate: true,
        },
        [3 + m * k, 7 + n * k, 11 + m * n, 1],
        "transposed weight, accumulate, tanh",
    );
    // Batched outer x inner with interleaved heads, as in attention.
    let (nb, heads, seq, d) = (3, 4, 50, 16);
    let row = 3 * heads * d;
    check_matmul(
        &g,
        Spec {
            m: seq,
            n: seq,
            k: d,
            outer: nb,
            inner: heads,
            a: View {
                off: 0,
                rs: row,
                cs: 1,
                bo: seq * row,
                bi: d,
            },
            b: View {
                off: heads * d,
                rs: 1,
                cs: row,
                bo: seq * row,
                bi: d,
            },
            c: View {
                off: 0,
                rs: seq,
                cs: 1,
                bo: heads * seq * seq,
                bi: seq * seq,
            },
            bias: None,
            alpha: 0.25,
            act: Act::None,
            accumulate: false,
        },
        [nb * seq * row, nb * seq * row, nb * heads * seq * seq, 1],
        "batched q k^T",
    );
    // Batched with per-outer bias, as in the band split and mask estimators.
    let (cnt, t, w, out) = (5, 33, 24, 96);
    check_matmul(
        &g,
        Spec {
            m: t,
            n: out,
            k: w,
            outer: cnt,
            inner: 1,
            a: View::rows(2, w).batch(t * w, 0),
            b: View::t(0, w).batch(out * w, 0),
            c: View::rows(0, out).batch(t * out, 0),
            bias: Some((0, out)),
            alpha: 1.0,
            act: Act::None,
            accumulate: false,
        },
        [2 + cnt * t * w, cnt * out * w, cnt * t * out, cnt * out],
        "batched per-band weights and bias",
    );
    // Several tiles in both directions, a ragged K tile, and A stored
    // column-major (the loader's other path).
    let (m, n, k) = (200, 300, 70);
    check_matmul(
        &g,
        Spec {
            m,
            n,
            k,
            outer: 1,
            inner: 1,
            a: View::t(0, m),
            b: View::rows(0, n),
            c: View::rows(0, n),
            bias: Some((0, 0)),
            alpha: 1.0,
            act: Act::None,
            accumulate: false,
        },
        [m * k, k * n, m * n, n],
        "several tiles, column-major A",
    );
    // Activations far out in their tails (inputs up to about +-200).
    for act in [Act::Tanh, Act::Gelu] {
        let (m, n, k) = (17, 19, 64);
        check_matmul(
            &g,
            Spec {
                m,
                n,
                k,
                outer: 1,
                inner: 1,
                a: View::rows(0, k),
                b: View::rows(0, n),
                c: View::rows(0, n),
                bias: None,
                alpha: 40.0,
                act,
                accumulate: false,
            },
            [m * k, k * n, m * n, 1],
            &format!("{act:?} tails"),
        );
    }
}

#[test]
fn softmax_rows() {
    let Some(g) = gpu() else { return };
    // More rows than one grid dimension holds, and rows longer than 256.
    for (rows, len) in [(70_000, 5), (40, 1151), (3, 1)] {
        let x: Vec<f32> = rand(rows * len, 9).into_iter().map(|v| v * 30.0).collect();
        let b = g.upload("x", &x);
        let mut r = g.rec();
        r.softmax(&b, rows, len);
        r.submit();
        let got = read(&g, &b, rows * len);
        let mut want = x.clone();
        for row in want.chunks_mut(len) {
            let m = row.iter().cloned().fold(f32::MIN, f32::max) as f64;
            let s: f64 = row.iter().map(|v| (*v as f64 - m).exp()).sum();
            for v in row.iter_mut() {
                *v = ((*v as f64 - m).exp() / s) as f32;
            }
        }
        assert_close(&got, &want, 1e-5, &format!("softmax {rows}x{len}"));
    }
}

#[test]
fn rmsnorm_rows() {
    let Some(g) = gpu() else { return };
    let rows = 300;
    let x = rand(512 + rows * 256, 5);
    let gamma = rand(256, 6);
    let (xb, gb, yb) = (
        g.upload("x", &x),
        g.upload("g", &gamma),
        g.buffer("y", 256 + rows * 256),
    );
    let mut r = g.rec();
    r.rmsnorm(&xb, 512, &gb, &yb, 256, rows);
    r.submit();
    let got = read(&g, &yb, 256 + rows * 256);
    let mut want = vec![0.0f32; 256 + rows * 256];
    for i in 0..rows {
        let src = &x[512 + i * 256..][..256];
        let n = src
            .iter()
            .map(|v| (*v as f64).powi(2))
            .sum::<f64>()
            .sqrt()
            .max(1e-12);
        for c in 0..256 {
            want[256 + i * 256 + c] = (src[c] as f64 / n * gamma[c] as f64) as f32;
        }
    }
    assert_close(&got, &want, 1e-5, "rmsnorm");
}

#[test]
fn rope_gate_transpose_glu() {
    let Some(g) = gpu() else { return };
    // Rotary embedding on q and k of [tokens, 1536]; v untouched.
    let (seq, tokens) = (7, 21);
    let qkv = rand(tokens * 1536, 7);
    let mut table = vec![0.0f32; 2 * seq * 32];
    for p in 0..seq {
        for i in 0..32 {
            let a = p as f64 / 10000f64.powf(2.0 * i as f64 / 64.0);
            table[p * 32 + i] = a.cos() as f32;
            table[seq * 32 + p * 32 + i] = a.sin() as f32;
        }
    }
    let (qb, tb) = (g.upload("qkv", &qkv), g.upload("table", &table));
    let mut r = g.rec();
    r.rope(&qb, &tb, tokens, seq);
    r.submit();
    let got = read(&g, &qb, tokens * 1536);
    let mut want = qkv.clone();
    for t in 0..tokens {
        let p = t % seq;
        for col in (0..1024).step_by(2) {
            let i = (col % 64) / 2;
            let (c, s) = (table[p * 32 + i], table[seq * 32 + p * 32 + i]);
            let (x0, x1) = (qkv[t * 1536 + col], qkv[t * 1536 + col + 1]);
            want[t * 1536 + col] = x0 * c - x1 * s;
            want[t * 1536 + col + 1] = x1 * c + x0 * s;
        }
    }
    assert_close(&got, &want, 1e-6, "rope");

    // Sigmoid gates per head.
    let o = rand(tokens * 512, 8);
    let gates: Vec<f32> = rand(tokens * 8, 9).into_iter().map(|v| v * 100.0).collect();
    let (ob, gb) = (g.upload("o", &o), g.upload("gates", &gates));
    let mut r = g.rec();
    r.gate(&ob, &gb, tokens);
    r.submit();
    let got = read(&g, &ob, tokens * 512);
    let want: Vec<f32> = (0..tokens * 512)
        .map(|i| o[i] / (1.0 + (-gates[(i / 512) * 8 + (i % 512) / 64]).exp()))
        .collect();
    assert_close(&got, &want, 1e-6, "gate");

    // [a, b, 256] -> [b, a, 256]
    let (a, b) = (5, 9);
    let x = rand(a * b * 256, 10);
    let (xb, yb) = (g.upload("x", &x), g.buffer("y", a * b * 256));
    let mut r = g.rec();
    r.transpose(&xb, &yb, a, b);
    r.submit();
    let got = read(&g, &yb, a * b * 256);
    let mut want = vec![0.0; a * b * 256];
    for i in 0..a {
        for j in 0..b {
            want[(j * a + i) * 256..][..256].copy_from_slice(&x[(i * b + j) * 256..][..256]);
        }
    }
    assert_eq!(got, want, "transpose");

    // GLU scattered into a [frames, 4100] mask at an offset.
    let (n, frames, w, off) = (3, 11, 48, 1000);
    let y = rand(n * frames * 2 * w, 11);
    let (yb, mb) = (
        g.upload("y", &y),
        g.upload("mask", &vec![7.0; frames * 4100]),
    );
    let mut r = g.rec();
    r.glu(&yb, &mb, n, frames, w, off);
    r.submit();
    let got = read(&g, &mb, frames * 4100);
    let mut want = vec![7.0f32; frames * 4100];
    for j in 0..n {
        for t in 0..frames {
            for c in 0..w {
                let row = (j * frames + t) * 2 * w;
                want[t * 4100 + off + j * w + c] = y[row + c] / (1.0 + (-y[row + w + c]).exp());
            }
        }
    }
    assert_close(&got, &want, 1e-6, "glu");
    assert!(g.take_errors().is_empty());
}

/// The start-up self-check passes every kernel on a correct GPU.
#[test]
fn check_kernels_accepts_all() {
    let Some(g) = gpu() else { return };
    let problems = pollster::block_on(g.check_kernels()).unwrap();
    assert!(problems.is_empty(), "{problems:?}");
    assert!(g.usable.get().iter().all(|&u| u));
}

/// A device opened for profiling records GPU time per dispatch under
/// labels naming the kernel (and, for matmuls, the shape and tile).  Skips
/// when the adapter has no `timestamp-query`.
#[test]
fn profile_labels_dispatches() {
    let Ok(g) = pollster::block_on(Gpu::new_profiling(wgpu::PowerPreference::HighPerformance))
    else {
        eprintln!("skipping: no adapter");
        return;
    };
    if !g.profiling() {
        eprintln!("skipping: adapter without timestamp-query");
        return;
    }
    let (m, n, k) = (70, 90, 33);
    let a = g.upload("a", &rand(m * k, 1));
    let b = g.upload("b", &rand(k * n, 2));
    let c = g.buffer("c", m * n);
    for _ in 0..2 {
        let mut r = g.rec();
        r.matmul(&MatMul::new(
            (&a, View::rows(0, k)),
            (&b, View::rows(0, n)),
            (&c, View::rows(0, n)),
            m,
            n,
            k,
        ));
        r.softmax(&c, m, n);
        r.submit();
    }
    pollster::block_on(g.done());
    pollster::block_on(g.collect_profile()).unwrap();
    assert!(g.take_errors().is_empty());
    let totals = g.take_profile();
    let mm = format!(
        "matmul 70x90x33 x1 {}",
        TILES[g.policy.get().pick(m, n)].name
    );
    assert_eq!(totals.get(&mm).map(|v| v.0), Some(2), "{totals:?}");
    assert_eq!(totals.get("softmax").map(|v| v.0), Some(2), "{totals:?}");
    assert!(
        totals.values().all(|v| v.1 >= 0.0 && v.1 < 10.0),
        "{totals:?}"
    );
    assert!(g.take_profile().is_empty(), "totals are cleared");
    let lines = roformer_gpu::profile_report(&totals);
    assert!(lines.last().unwrap().starts_with("profile: total GPU time"));
}

/// The usual device never asks for optional features.
#[test]
fn plain_device_does_not_profile() {
    let Some(g) = gpu() else { return };
    assert!(!g.profiling());
    assert!(g.device.features().is_empty());
}

/// Compile time and resident memory after each matmul kernel's first use
/// (SwiftShader diagnostics; `--ignored --nocapture`).
#[test]
#[ignore]
fn compile_each() {
    let Some(g) = gpu() else { return };
    let only: Option<usize> = std::env::var("TILE").ok().and_then(|v| v.parse().ok());
    let (m, n, k) = (70, 90, 33);
    let a = g.upload("a", &rand(m * k, 1));
    let b = g.upload("b", &rand(k * n, 2));
    let c = g.buffer("c", m * n);
    for (t, tile) in TILES.iter().enumerate() {
        if only.is_some_and(|o| o != t) {
            continue;
        }
        let t0 = std::time::Instant::now();
        let mut mm = MatMul::new(
            (&a, View::rows(0, k)),
            (&b, View::rows(0, n)),
            (&c, View::rows(0, n)),
            m,
            n,
            k,
        );
        mm.tile = Some(t);
        let mut r = g.rec();
        r.matmul(&mm);
        r.submit();
        pollster::block_on(g.done());
        let rss = std::fs::read_to_string("/proc/self/status")
            .unwrap_or_default()
            .lines()
            .find(|l| l.starts_with("VmRSS"))
            .unwrap_or("")
            .to_string();
        eprintln!("{}: {:.1} s, {rss}", tile.name, t0.elapsed().as_secs_f64());
    }
}

/// The fused attention kernel against `attention_cpu` on the network's
/// sequence lengths (1,151 frames on the time axis, 62 bands on the
/// frequency axis) and a ragged one, with and without the gate.
#[test]
fn attention_matches_cpu() {
    let Some(g) = gpu() else { return };
    for (seq, nseq, gated) in [
        (133, 2, true),
        (62, 3, true),
        (62, 1, false),
        (1151, 1, true),
    ] {
        let tokens = seq * nseq;
        let qkv = rand(tokens * 1536, 7);
        let gates = rand(tokens * 8, 8);
        let alpha = 0.125;
        let want = roformer_gpu::attention_cpu(&qkv, gated.then_some(&gates[..]), seq, nseq, alpha);
        let (qb, gb) = (g.upload("qkv", &qkv), g.upload("gates", &gates));
        let ob = g.buffer("o", tokens * 512);
        let mut r = g.rec();
        r.attention(&qb, gated.then_some(&gb), &ob, seq, nseq, alpha);
        r.submit();
        let got = read(&g, &ob, tokens * 512);
        let errors = g.take_errors();
        assert!(errors.is_empty(), "{errors:?}");
        assert_close(&got, &want, 2e-5, &format!("attention seq {seq} x{nseq}"));
    }
}

/// The fused kernel gives what the separate path in net.rs gives (scores
/// matmul, softmax, p.v matmul, gate), with net.rs's own strides.
#[test]
fn attention_matches_split_path() {
    let Some(g) = gpu() else { return };
    let (seq, nseq) = (133, 2);
    let tokens = seq * nseq;
    let row = 1536;
    let qkv = g.upload("qkv", &rand(tokens * row, 9));
    let gates = g.upload("gates", &rand(tokens * 8, 10));
    let scores = g.buffer("scores", nseq * 8 * seq * seq);
    let split = g.buffer("o split", tokens * 512);
    let alpha = 0.125f32;
    let mut r = g.rec();
    let mut sc = MatMul::new(
        (
            &qkv,
            View {
                off: 0,
                rs: row,
                cs: 1,
                bo: seq * row,
                bi: 64,
            },
        ),
        (
            &qkv,
            View {
                off: 512,
                rs: 1,
                cs: row,
                bo: seq * row,
                bi: 64,
            },
        ),
        (
            &scores,
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
    sc.alpha = alpha;
    r.matmul(&sc);
    r.softmax(&scores, nseq * 8 * seq, seq);
    let mut pv = MatMul::new(
        (
            &scores,
            View {
                off: 0,
                rs: seq,
                cs: 1,
                bo: 8 * seq * seq,
                bi: seq * seq,
            },
        ),
        (
            &qkv,
            View {
                off: 1024,
                rs: row,
                cs: 1,
                bo: seq * row,
                bi: 64,
            },
        ),
        (
            &split,
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
    r.gate(&split, &gates, tokens);
    r.submit();
    let want = read(&g, &split, tokens * 512);
    let fused = g.buffer("o fused", tokens * 512);
    let mut r = g.rec();
    r.attention(&qkv, Some(&gates), &fused, seq, nseq, alpha);
    r.submit();
    let got = read(&g, &fused, tokens * 512);
    assert!(g.take_errors().is_empty());
    assert_close(&got, &want, 2e-5, "fused attention against the split path");
}

#[test]
fn check_attention_accepts_all() {
    let Some(g) = gpu() else { return };
    let problem = pollster::block_on(g.check_attention());
    assert_eq!(problem, None);
    assert!(g.fused_ok.get());
}
