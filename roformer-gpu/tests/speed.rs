//! Times one matmul shape and one transformer step (ignored by default:
//! `cargo test -p roformer-gpu --release --test speed -- --ignored --nocapture`).

use roformer_gpu::{Gpu, MatMul, TILES, View};

#[test]
#[ignore]
fn matmul_throughput() {
    let g = pollster::block_on(Gpu::new(wgpu::PowerPreference::HighPerformance)).unwrap();
    eprintln!("adapter: {} ({:?})", g.info.name, g.info.backend);
    for (m, n, k) in [(512, 512, 256)] {
        let a = g.buffer("a", m * k);
        let b = g.buffer("b", k * n);
        let c = g.buffer("c", m * n);
        for (t, tile) in TILES.iter().enumerate() {
            let mut mm = MatMul::new(
                (&a, View::rows(0, k)),
                (&b, View::t(0, k)),
                (&c, View::rows(0, n)),
                m,
                n,
                k,
            );
            mm.tile = Some(t);
            for rep in 0..2 {
                let t0 = std::time::Instant::now();
                let mut r = g.rec();
                r.matmul(&mm);
                r.submit();
                pollster::block_on(g.done());
                let s = t0.elapsed().as_secs_f64();
                let rate = 2.0 * (m * n * k) as f64 / s / 1e9;
                eprintln!(
                    "{m}x{n}x{k} {} run {rep}: {s:.3} s, {rate:.2} GFLOP/s",
                    tile.name
                );
            }
        }
    }
}

#[test]
#[ignore]
fn layer_steps() {
    use roformer_gpu::net::{Job, Net};
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../third_party/roformer/bs_roformer_sw.f32.safetensors"
    );
    let Ok(w) = std::fs::read(path) else { return };
    let g = pollster::block_on(Gpu::new(wgpu::PowerPreference::HighPerformance)).unwrap();
    let budget: usize = std::env::var("BUDGET")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(400_000);
    let net = Net::load(&g, &w, budget).unwrap();
    let frames = 87;
    let job = Job::new(&g, &net, frames);
    job.set_hidden(&vec![0.1; 62 * frames * 256]);
    let mut by_kind = std::collections::BTreeMap::<&str, (usize, f64)>::new();
    for step in job.layer_steps(0).into_iter().chain(job.mask_steps(0)) {
        let t0 = std::time::Instant::now();
        job.run(&step);
        pollster::block_on(g.done());
        let e = by_kind.entry(step.kind()).or_default();
        e.0 += 1;
        e.1 += t0.elapsed().as_secs_f64();
    }
    for (k, (n, s)) in by_kind {
        eprintln!("{k}: {n} steps, {s:.2} s");
    }
}

/// The in-browser benchmark's code path, natively (tiny work per shape).
#[test]
#[ignore]
fn bench_runs() {
    let g = pollster::block_on(Gpu::new(wgpu::PowerPreference::HighPerformance)).unwrap();
    let t0 = std::time::Instant::now();
    let table = pollster::block_on(roformer_gpu::bench::measure(
        &g,
        0.0,
        0.0,
        2,
        Some(&[0, 1]),
        || t0.elapsed().as_secs_f64(),
        |l| eprintln!("{l}"),
    ));
    for l in roformer_gpu::bench::summary(&table) {
        eprintln!("{l}");
    }
}
