//! How stable the time-axis attention choice is on a freshly opened GPU
//! (ignored by default:
//! `cargo test -p roformer-gpu --release --test calibration -- --ignored --nocapture`).

use roformer_gpu::Gpu;
use roformer_gpu::bench::{ATTENTION_ROUND, ATTENTION_WARM, attention_per_chunk};
use std::time::Instant;

#[test]
#[ignore]
fn time_axis_attention_choice_on_a_cold_gpu() {
    let start = Instant::now();
    let now = || start.elapsed().as_secs_f64();
    let settings = [
        ("calib v1", 0.1, 0.1, 3),
        ("calib v2", ATTENTION_WARM, ATTENTION_ROUND, 3),
        ("bench", 0.3, 0.2, 3),
        ("long", 1.0, 0.5, 5),
    ];
    for open in 0..4 {
        let g = pollster::block_on(Gpu::new(wgpu::PowerPreference::HighPerformance)).unwrap();
        pollster::block_on(g.check_kernels()).unwrap();
        pollster::block_on(g.check_attention());
        eprintln!("open {open}: {} ({:?})", g.info.name, g.info.backend);
        for (name, warm, round, rounds) in settings {
            let ((split, fused), _) =
                pollster::block_on(attention_per_chunk(&g, warm, round, rounds, &now));
            eprintln!(
                "  {name:8}: split {:6.2} s (spread {:5.1}%), fused {:6.2} s (spread {:5.1}%) -> {}",
                split.median(),
                100.0 * split.spread(),
                fused.median(),
                100.0 * fused.spread(),
                if fused.median() < split.median() {
                    "fused"
                } else {
                    "split"
                }
            );
        }
    }
}
