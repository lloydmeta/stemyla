use engine::stretch::StretcherKind;
fn main() {
    for kind in StretcherKind::all() {
        for t in [0.5, 0.75, 1.0, 1.25, 1.5] {
            let (m, s) = xtask::bench::impulse_latency(kind, t);
            let declared = {
                let mut st = kind.build(44_100);
                st.set_tempo(t);
                st.latency()
            };
            println!(
                "{:12} {t:.2}: mean offset {m:8.1} frames, spread {s:7.1}, declared {declared:.0}",
                kind.label()
            );
        }
    }
}
