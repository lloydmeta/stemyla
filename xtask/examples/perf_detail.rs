use engine::stretch::StretcherKind;
use xtask::bench::{callback_cost, perf_song};
fn main() {
    let (song, _) = perf_song(6);
    for kind in StretcherKind::all() {
        for run in 0..2 {
            let r = callback_cost(song.clone(), kind, 0.75, 512, 10_000);
            println!(
                "{:12} run {run}: mean {:5.1}%  p99 {:5.1}%  max {:5.1}%",
                kind.label(),
                r.mean_pct,
                r.p99_pct,
                r.max_pct
            );
        }
    }
}
