//! `xtask stretch-bench`: renders the synthetic song through every compiled
//! stretcher at 0.5x, 0.75x and 1.25x and measures duration, pitch,
//! transients, CPU cost and latency.  `xtask perf`: per-callback engine cost.

use crate::metrics::peak_freq_near;
use crate::synth::{self, Onsets};
use anyhow::Result;
use engine::stretch::StretcherKind;
use engine::{Player, PlayerConfig, SharedParams, Song, Stem};
use std::fmt::Write as _;
use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;
use std::time::Instant;

pub const TEMPOS: [f64; 3] = [0.5, 0.75, 1.25];
const SR: f64 = synth::SAMPLE_RATE as f64;

/// Streams a mono signal (duplicated to stereo) through a stretcher in
/// 512-frame pulls and 256-frame pushes.  Returns the left output, the CPU
/// seconds spent inside the stretcher, and the stretcher's declared latency.
pub fn stretch_signal(kind: StretcherKind, x: &[f32], tempo: f64) -> (Vec<f32>, f64, f64) {
    let mut s = kind.build(synth::SAMPLE_RATE);
    s.set_tempo(tempo);
    let target = (x.len() as f64 / tempo) as usize;
    let mut out = Vec::with_capacity(target + 1024);
    let (mut l, mut r) = (vec![0.0f32; 512], vec![0.0f32; 512]);
    let zeros = vec![0.0f32; 256];
    let mut fed = 0;
    let mut cpu = 0.0;
    while out.len() < target {
        let t0 = Instant::now();
        let mut guard = 0;
        while !s.can_pull(512) && guard < 1000 {
            if fed + 256 <= x.len() {
                s.push(&x[fed..fed + 256], &x[fed..fed + 256]);
            } else {
                s.push(&zeros, &zeros);
            }
            fed += 256;
            guard += 1;
        }
        s.pull(&mut l, &mut r);
        cpu += t0.elapsed().as_secs_f64();
        out.extend_from_slice(&l);
    }
    let latency = s.latency();
    (out, cpu, latency)
}

/// Envelope onset: first sample in `[from, to)` whose magnitude passes
/// `frac` of the window's peak.  Returns (onset index, 10%..90% rise in samples).
fn onset_and_rise(x: &[f32], from: usize, to: usize) -> Option<(usize, usize)> {
    let to = to.min(x.len());
    if from >= to {
        return None;
    }
    // Smoothed magnitude: peak over a 1 ms sliding window.
    let w = (SR / 1000.0) as usize;
    let env: Vec<f32> = (from..to)
        .map(|i| {
            let a = i.saturating_sub(w);
            x[a..=i].iter().fold(0.0f32, |m, v| m.max(v.abs()))
        })
        .collect();
    let peak = env.iter().cloned().fold(0.0f32, f32::max);
    if peak < 1e-4 {
        return None;
    }
    let i10 = env.iter().position(|v| *v >= 0.1 * peak)?;
    let i90 = env.iter().position(|v| *v >= 0.9 * peak)?;
    Some((from + i10, i90.saturating_sub(i10)))
}

pub struct TransientStats {
    /// Least-squares slope of output onset times vs input onset times,
    /// times tempo.  1.0 means the timeline scaled exactly by 1/tempo.
    pub duration_ratio: f64,
    /// Intercept of the same fit, in output frames: measured latency.
    pub latency_frames: f64,
    /// Mean 10%..90% rise time of kick and snare onsets, in ms.
    pub rise_ms: f64,
    /// Onsets found.
    pub n: usize,
}

/// Locates kick and snare onsets in a stretched drums render.
pub fn transient_stats(
    out: &[f32],
    onsets: &Onsets,
    tempo: f64,
    latency_hint: f64,
) -> TransientStats {
    let all: Vec<f64> = onsets
        .kick
        .iter()
        .chain(onsets.snare.iter())
        .cloned()
        .collect();
    onset_stats(out, all, tempo, latency_hint)
}

/// Same as [`transient_stats`] for an explicit list of onset times.
pub fn onset_stats(
    out: &[f32],
    mut all: Vec<f64>,
    tempo: f64,
    latency_hint: f64,
) -> TransientStats {
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    let mut rises = Vec::new();
    all.sort_by(|a, b| a.partial_cmp(b).unwrap());
    for t in all {
        let expected = t * SR / tempo + latency_hint;
        // Search from 20 ms before to 40 ms after the expected position.
        let from = (expected - 0.02 * SR).max(0.0) as usize;
        let to = (expected + 0.04 * SR) as usize;
        if let Some((on, rise)) = onset_and_rise(out, from, to) {
            xs.push(t * SR);
            ys.push(on as f64);
            rises.push(rise as f64 / SR * 1000.0);
        }
    }
    let n = xs.len() as f64;
    let mx = xs.iter().sum::<f64>() / n;
    let my = ys.iter().sum::<f64>() / n;
    let sxy: f64 = xs.iter().zip(&ys).map(|(x, y)| (x - mx) * (y - my)).sum();
    let sxx: f64 = xs.iter().map(|x| (x - mx) * (x - mx)).sum();
    let slope = sxy / sxx;
    TransientStats {
        duration_ratio: slope * tempo,
        latency_frames: my - slope * mx,
        rise_ms: rises.iter().sum::<f64>() / rises.len().max(1) as f64,
        n: xs.len(),
    }
}

/// Pad pitch in a stretched keys render, measured in the middle of the
/// sustain (a 0.5 s window), coarse-to-fine DFT peak search.
pub fn pad_pitch(out: &[f32], tempo: f64) -> f64 {
    let centre_in = 15.0; // seconds into the song: inside the sustain
    let start = (centre_in / tempo * SR) as usize;
    let n = (0.5 * SR) as usize;
    let w = &out[start..start + n];
    let coarse = peak_freq_near(w, SR, synth::PAD_FREQ_HZ, synth::PAD_FREQ_HZ * 0.55);
    peak_freq_near(w, SR, coarse, 3.0)
}

pub struct Row {
    pub kind: &'static str,
    pub tempo: f64,
    pub duration_ratio: f64,
    pub pitch_hz: f64,
    pub pitch_err_pct: f64,
    pub rise_ms: f64,
    pub rise_ratio: f64,
    pub guitar_rise_ratio: f64,
    pub rtf: f64,
    pub latency_xcorr: isize,
    pub latency_measured: f64,
    pub latency_declared: f64,
}

pub fn bench_kind(kind: StretcherKind, song: &synth::Song, tempo: f64, base_rise: f64) -> Row {
    let get = |name: &str| &song.stems.iter().find(|(n, _)| n == name).unwrap().1.left;
    let drums = get("drums");
    let keys = get("keys");
    let (d_out, _, lat) = stretch_signal(kind, drums, tempo);
    let ts = transient_stats(&d_out, &song.truth.onsets, tempo, 0.0);
    let (k_out, _, _) = stretch_signal(kind, keys, tempo);
    let guitar = get("guitar");
    let (g_out, _, _) = stretch_signal(kind, guitar, tempo);
    let g_in = onset_stats(guitar, song.truth.onsets.guitar.clone(), 1.0, 0.0).rise_ms;
    let g_st = onset_stats(&g_out, song.truth.onsets.guitar.clone(), tempo, 0.0).rise_ms;
    // Latency at 1.0x: cross-correlate the stretched drums with the input.
    let one_s = SR as usize;
    let (d1, _, _) = stretch_signal(kind, &drums[..4 * one_s], 1.0);
    let lag = crate::metrics::best_lag(&d1[one_s..3 * one_s], &drums[one_s..3 * one_s], 6000);
    let pitch = pad_pitch(&k_out, tempo);
    // CPU cost on the full mix (stereo path is what the player uses).
    let (_, cpu, _) = stretch_signal(kind, &song.mix.left, tempo);
    let audio_s = song.mix.left.len() as f64 / tempo / SR;
    Row {
        kind: kind.label(),
        tempo,
        duration_ratio: ts.duration_ratio,
        pitch_hz: pitch,
        pitch_err_pct: (pitch / synth::PAD_FREQ_HZ - 1.0) * 100.0,
        rise_ms: ts.rise_ms,
        rise_ratio: ts.rise_ms / base_rise,
        guitar_rise_ratio: g_st / g_in,
        rtf: cpu / audio_s,
        latency_xcorr: lag,
        latency_measured: ts.latency_frames,
        latency_declared: lat,
    }
}

pub fn baseline_rise(song: &synth::Song) -> f64 {
    let drums = &song
        .stems
        .iter()
        .find(|(n, _)| n == "drums")
        .unwrap()
        .1
        .left;
    transient_stats(drums, &song.truth.onsets, 1.0, 0.0).rise_ms
}

pub fn run(_args: &[String]) -> Result<()> {
    let song = synth::generate();
    let base_rise = baseline_rise(&song);
    let mut md = String::new();
    writeln!(
        md,
        "| Stretcher | Tempo | Duration ratio | Pad pitch (Hz) | Pitch error | Drum rise (ms) | Drum rise vs original | Guitar rise vs original | RTF | Onset-fit offset (frames) | Latency at 1.0x, xcorr (frames) | Latency declared |"
    )?;
    writeln!(md, "| - | - | - | - | - | - | - | - | - | - | - | - |")?;
    let mut rows = Vec::new();
    for kind in StretcherKind::all() {
        for &t in &TEMPOS {
            let r = bench_kind(kind, &song, t, base_rise);
            let ok_d = if (r.duration_ratio - 1.0).abs() <= 0.005 {
                ""
            } else {
                " ✗"
            };
            let ok_p = if r.pitch_err_pct.abs() <= 1.0 {
                ""
            } else {
                " ✗"
            };
            writeln!(
                md,
                "| {} | {:.2} | {:.4}{} | {:.2} | {:+.2}%{} | {:.2} | {:.2} | {:.2} | {:.4} | {:.0} | {} | {:.0} |",
                r.kind,
                r.tempo,
                r.duration_ratio,
                ok_d,
                r.pitch_hz,
                r.pitch_err_pct,
                ok_p,
                r.rise_ms,
                r.rise_ratio,
                r.guitar_rise_ratio,
                r.rtf,
                r.latency_measured,
                r.latency_xcorr,
                r.latency_declared
            )?;
            eprintln!("done {} {}", r.kind, t);
            rows.push(r);
        }
    }
    writeln!(
        md,
        "\nOriginal drums rise time: {base_rise:.2} ms.  Pad: {:.2} Hz.  RTF = CPU seconds inside the stretcher per second of output audio (release build, 1 thread).  ✗ marks a miss on the ±0.5% duration or ±1% pitch target.",
        synth::PAD_FREQ_HZ
    )?;
    // Choice rule: pass duration and pitch at 0.75x, then the smallest mean
    // transient smearing (drums and guitar, all tempos), then CPU.
    let mut best: Option<(&str, f64, f64)> = None;
    for kind in StretcherKind::all() {
        let label = kind.label();
        let mine: Vec<&Row> = rows.iter().filter(|r| r.kind == label).collect();
        let at075 = mine.iter().find(|r| (r.tempo - 0.75).abs() < 1e-9).unwrap();
        if (at075.duration_ratio - 1.0).abs() > 0.005 || at075.pitch_err_pct.abs() > 1.0 {
            continue;
        }
        let smear = mine
            .iter()
            .map(|r| r.rise_ratio + r.guitar_rise_ratio)
            .sum::<f64>()
            / (2 * mine.len()) as f64;
        let rtf = mine.iter().map(|r| r.rtf).sum::<f64>() / mine.len() as f64;
        writeln!(
            md,
            "\nCandidate {label}: mean rise ratio {smear:.2}, mean RTF {rtf:.4}"
        )?;
        if best.is_none_or(|(_, s, _)| smear < s) {
            best = Some((label, smear, rtf));
        }
    }
    match best {
        Some((label, _, _)) => writeln!(md, "\nDefault by rule: **{label}**")?,
        None => writeln!(md, "\nNo candidate met the 0.75x targets")?,
    }
    println!("{md}");
    std::fs::create_dir_all("target/bench")?;
    std::fs::write("target/bench/stretch.md", &md)?;
    Ok(())
}

/// Per-callback engine cost: N stems, the given stretcher and tempo,
/// 512-frame callbacks.  Prints mean, p99 and max as a share of the period.
pub fn perf(args: &[String]) -> Result<()> {
    let n_stems: usize = args.first().and_then(|a| a.parse().ok()).unwrap_or(6);
    let tempo: f32 = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(0.75);
    let (song, kind) = perf_song(n_stems);
    let r = callback_cost(song, kind, tempo, 512, 2000);
    println!(
        "{} stems, {} at {tempo}x, 512-frame callbacks ({:.2} ms period): mean {:.1}%, p99 {:.1}%, max {:.1}% of the period",
        n_stems,
        kind.label(),
        r.period_ms,
        r.mean_pct,
        r.p99_pct,
        r.max_pct
    );
    Ok(())
}

pub fn perf_song(n_stems: usize) -> (Arc<Song>, StretcherKind) {
    let s = synth::generate();
    let mut stems: Vec<Stem> = Vec::new();
    for i in 0..n_stems {
        let (name, st) = &s.stems[i % s.stems.len()];
        stems.push(Stem {
            name: format!("{name}{i}"),
            left: st.left.clone().into(),
            right: st.right.clone().into(),
        });
    }
    let song = Song {
        sample_rate: synth::SAMPLE_RATE,
        frames: s.truth.frames,
        stems,
    };
    (Arc::new(song), StretcherKind::default_pitch_preserving())
}

pub struct CallbackCost {
    pub period_ms: f64,
    pub mean_pct: f64,
    pub p99_pct: f64,
    pub max_pct: f64,
}

pub fn callback_cost(
    song: Arc<Song>,
    kind: StretcherKind,
    tempo: f32,
    block: usize,
    callbacks: usize,
) -> CallbackCost {
    let params = Arc::new(SharedParams::new(song.stems.len()));
    params.set_tempo(tempo);
    params.playing.store(true, Relaxed);
    params.set_loop(0, song.frames as u64);
    let stretcher = kind.build(song.sample_rate);
    let mut p = Player::with_config(song.clone(), params, stretcher, PlayerConfig::default());
    let (mut l, mut r) = (vec![0.0; block], vec![0.0; block]);
    // Warm up.
    for _ in 0..20 {
        p.process(&mut l, &mut r);
    }
    let mut times = Vec::with_capacity(callbacks);
    for _ in 0..callbacks {
        let t0 = Instant::now();
        p.process(&mut l, &mut r);
        times.push(t0.elapsed().as_secs_f64());
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let period = block as f64 / song.sample_rate as f64;
    let mean = times.iter().sum::<f64>() / times.len() as f64;
    CallbackCost {
        period_ms: period * 1000.0,
        mean_pct: mean / period * 100.0,
        p99_pct: times[times.len() * 99 / 100] / period * 100.0,
        max_pct: times[times.len() - 1] / period * 100.0,
    }
}

/// Latency in output frames at `tempo`, from a click train: one unit
/// impulse every 0.5 s, located in the output by its largest sample near the
/// ideal position, then averaged.  Returns (mean offset, spread max-min).
pub fn impulse_latency(kind: StretcherKind, tempo: f64) -> (f64, f64) {
    let n = 8 * SR as usize;
    let period = SR as usize / 2;
    let mut x = vec![0.0f32; n];
    let mut t = period;
    while t < n - period {
        x[t] = 0.8;
        t += period;
    }
    let (out, _, _) = stretch_signal(kind, &x, tempo);
    let mut offs = Vec::new();
    let mut t = 2 * period; // skip the first click (start-up)
    while t < n - 2 * period {
        let ideal = t as f64 / tempo;
        let lo = ideal as usize;
        let hi = (ideal + 0.05 * SR) as usize;
        let lo2 = (ideal - 0.02 * SR) as usize;
        let seg = &out[lo2..hi.min(out.len())];
        let (k, _) =
            seg.iter().enumerate().fold(
                (0, 0.0f32),
                |b, (i, v)| if v.abs() > b.1 { (i, v.abs()) } else { b },
            );
        offs.push((lo2 + k) as f64 - ideal);
        let _ = lo;
        t += period;
    }
    let mean = offs.iter().sum::<f64>() / offs.len() as f64;
    let spread = offs.iter().cloned().fold(f64::MIN, f64::max)
        - offs.iter().cloned().fold(f64::MAX, f64::min);
    (mean, spread)
}
