//! How calibration times the time axis.  A test with a fake clock pins
//! what `time_axis_per_chunk` adds up.  The test of how stable the choice
//! is on a freshly opened GPU is ignored by default:
//! `cargo test -p roformer-gpu --release --test calibration -- --ignored --nocapture`.
//! Both time the time axis as calibration does since version 5
//! (`time_axis_per_chunk`): whole time-transformer steps at the default
//! budget, with a wait after each.  `attention_per_chunk` times the
//! attention dispatches alone, as `?bench` does.

use roformer_gpu::bench::{
    ATTENTION_ROUND, ATTENTION_ROUNDS, ATTENTION_WARM, TimeSecs, decide, time_axis_per_chunk,
};
use roformer_gpu::{Gpu, WeightFormat};
use std::cell::Cell;
use std::time::Instant;
use test_support::{Need, skip};

/// Asserts, with a clock that moves 0.01 s at each read, no warm-up, no
/// round length and 2 rounds, that `time_axis_per_chunk` gives each policy
/// that this GPU can run 2 rounds of the figures below, and a kernel that
/// failed its check none:
/// * split: 0.02 s for each step x 744 steps = 14.88 s;
/// * each fused kernel: 0.02 s x (180 + 12) steps = 3.84 s.
///
/// 0.02 s is one submission for each wait: a read before it and a read
/// after it.  `Covering`, which queues more submissions for each wait,
/// gives less for each step.
#[test]
fn the_time_axis_adds_up_one_wait_for_each_step() {
    let g = match pollster::block_on(Gpu::new(wgpu::PowerPreference::HighPerformance)) {
        Ok(g) => g,
        Err(e) => return skip(Need::Gpu, &e),
    };
    pollster::block_on(g.check_kernels()).unwrap();
    pollster::block_on(g.check_attention());
    let (fused_ok, vfused_ok) = (g.fused_ok.get(), g.fused_ok.get() && g.vfused_ok.get());
    let clock = Cell::new(0.0);
    let now = || {
        clock.set(clock.get() + 0.01);
        clock.get()
    };
    let t = pollster::block_on(time_axis_per_chunk(
        &g,
        WeightFormat::F32,
        roformer_gpu::net::BUDGET,
        0.0,
        0.0,
        2,
        &now,
    ));
    for (name, timing, want, ok) in [
        ("split", &t.split, 14.88, true),
        ("fused", &t.fused, 3.84, fused_ok),
        ("vfused", &t.vfused, 3.84, vfused_ok),
    ] {
        let rounds = if ok { 2 } else { 0 };
        assert_eq!(timing.rounds.len(), rounds, "{name}");
        for r in &timing.rounds {
            assert!((r - want).abs() < 1e-9, "{name}: {r}, want {want}");
        }
    }
    let errors = g.take_errors();
    assert!(errors.is_empty(), "{errors:?}");
}

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
