//! How stable the time-axis attention choice is on a freshly opened GPU
//! (ignored by default:
//! `cargo test -p roformer-gpu --release --test calibration -- --ignored --nocapture`).
//! It times the time axis as calibration does since version 5
//! (`time_axis_per_chunk`): whole time-transformer steps at the default
//! budget, with a wait after each.  `attention_per_chunk` times the
//! attention dispatches alone, as `?bench` does.

use roformer_gpu::bench::{
    ATTENTION_ROUND, ATTENTION_ROUNDS, ATTENTION_WARM, TimeSecs, decide, time_axis_per_chunk,
};
use roformer_gpu::{Gpu, WeightFormat};
use std::time::Instant;

#[test]
#[ignore]
fn time_axis_attention_choice_on_a_cold_gpu() {
    let start = Instant::now();
    let now = || start.elapsed().as_secs_f64();
    let settings = [
        ("calib v1", 0.1, 0.1, 3),
        ("calib v2", ATTENTION_WARM, ATTENTION_ROUND, 3),
        (
            "calib v5",
            ATTENTION_WARM,
            ATTENTION_ROUND,
            ATTENTION_ROUNDS,
        ),
        ("bench", 0.3, 0.2, 3),
        ("long", 1.0, 0.5, 5),
    ];
    for open in 0..4 {
        let g = pollster::block_on(Gpu::new(wgpu::PowerPreference::HighPerformance)).unwrap();
        pollster::block_on(g.check_kernels()).unwrap();
        pollster::block_on(g.check_attention());
        eprintln!("open {open}: {} ({:?})", g.info.name, g.info.backend);
        for (name, warm, round, rounds) in settings {
            let t = pollster::block_on(time_axis_per_chunk(
                &g,
                WeightFormat::F32,
                roformer_gpu::net::BUDGET,
                warm,
                round,
                rounds,
                &now,
            ));
            let choice = decide(&[1.0], &[1.0], TimeSecs::of(&t), true, g.vfused_ok.get());
            eprintln!(
                "  {name:8}: split {:6.2} s (spread {:5.1}%), fused {:6.2} s (spread {:5.1}%), vfused {:6.2} s (spread {:5.1}%) -> {}",
                t.split.median(),
                100.0 * t.split.spread(),
                t.fused.median(),
                100.0 * t.fused.spread(),
                t.vfused.median(),
                100.0 * t.vfused.spread(),
                choice.attention.name()
            );
        }
    }
}
