//! Synthetic test song with exact ground truth.
//!
//! Used by `xtask gen-song`, the tests, and the app's "Demo song" button.
//!
//! 30 seconds, 44.1 kHz, stereo, 120 BPM, A minor (Am F C G, one chord per
//! bar).  Every stem is generated independently, so the mix is exactly the
//! sum of the stems and each stem is a perfect separation target.

use serde::Serialize;
use std::f64::consts::TAU;

pub const SAMPLE_RATE: u32 = 44_100;
pub const BPM: f64 = 120.0;
pub const DURATION_S: f64 = 30.0;
/// Single sustained pad note (E4).  Used as the pitch reference for stretching.
pub const PAD_FREQ_HZ: f64 = 329.627_6;

pub const STEM_NAMES: [&str; 5] = ["drums", "bass", "guitar", "keys", "lead"];

/// One stereo stem, stored as separate channels.
#[derive(Clone)]
pub struct Stereo {
    pub left: Vec<f32>,
    pub right: Vec<f32>,
}

impl Stereo {
    fn silent(n: usize) -> Self {
        Self {
            left: vec![0.0; n],
            right: vec![0.0; n],
        }
    }
}

#[derive(Serialize, Default, Clone, Debug)]
pub struct Onsets {
    pub kick: Vec<f64>,
    pub snare: Vec<f64>,
    pub hat: Vec<f64>,
    pub bass: Vec<f64>,
    pub guitar: Vec<f64>,
    pub lead: Vec<f64>,
}

#[derive(Serialize, Clone, Debug)]
pub struct Truth {
    pub sample_rate: u32,
    pub bpm: f64,
    pub duration_s: f64,
    pub frames: usize,
    pub stems: Vec<String>,
    pub pad_frequency_hz: f64,
    /// Times when the pad is at full level (after its attack, before its release).
    pub pad_sustain_s: (f64, f64),
    pub beat_times: Vec<f64>,
    pub onsets: Onsets,
    /// Amplitude scale applied to every stem so the mix peak stays below 0.9.
    pub normalisation: f64,
}

pub struct Song {
    pub stems: Vec<(String, Stereo)>,
    pub mix: Stereo,
    pub truth: Truth,
}

/// Tiny deterministic noise source (xorshift64*), so every run is identical.
struct Noise(u64);

impl Noise {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        let v = self.0.wrapping_mul(0x2545_f491_4f6c_dd1d);
        (v >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
    }
}

fn midi_to_hz(m: f64) -> f64 {
    440.0 * 2f64.powf((m - 69.0) / 12.0)
}

/// Adds a mono signal into a stereo stem with constant-power panning
/// (`pan` -1 left .. 1 right).
fn add_panned(dst: &mut Stereo, start: usize, sig: &[f64], pan: f64) {
    let angle = (pan + 1.0) * std::f64::consts::FRAC_PI_4;
    let (gl, gr) = (angle.cos(), angle.sin());
    for (i, s) in sig.iter().enumerate() {
        let j = start + i;
        if j >= dst.left.len() {
            break;
        }
        dst.left[j] += (s * gl) as f32;
        dst.right[j] += (s * gr) as f32;
    }
}

fn kick(sr: f64) -> Vec<f64> {
    let n = (0.45 * sr) as usize;
    let mut phase = 0.0;
    (0..n)
        .map(|i| {
            let t = i as f64 / sr;
            let f = 48.0 + 110.0 * (-t / 0.035).exp();
            phase += TAU * f / sr;
            let click = if i < 40 { 1.0 - i as f64 / 40.0 } else { 0.0 };
            (phase.sin() * (-t / 0.22).exp() + 0.3 * click) * 0.9
        })
        .collect()
}

fn snare(sr: f64, noise: &mut Noise) -> Vec<f64> {
    let n = (0.25 * sr) as usize;
    // Band-limited noise: one-pole low-pass minus a slower one-pole low-pass.
    let (mut lp1, mut lp2) = (0.0, 0.0);
    (0..n)
        .map(|i| {
            let t = i as f64 / sr;
            let w = noise.next();
            lp1 += 0.45 * (w - lp1);
            lp2 += 0.05 * (w - lp2);
            let band = lp1 - lp2;
            let tone = (TAU * 185.0 * t).sin() * (-t / 0.05).exp();
            (1.6 * band * (-t / 0.08).exp() + 0.5 * tone) * 0.8
        })
        .collect()
}

fn hat(sr: f64, noise: &mut Noise) -> Vec<f64> {
    let n = (0.06 * sr) as usize;
    let mut prev = 0.0;
    (0..n)
        .map(|i| {
            let t = i as f64 / sr;
            let w = noise.next();
            let hp = w - prev; // first difference: crude high-pass
            prev = w;
            hp * (-t / 0.012).exp() * 0.35
        })
        .collect()
}

/// Band-limited sawtooth note with a short attack and release.
fn saw_note(sr: f64, freq: f64, dur: f64) -> Vec<f64> {
    let n = (dur * sr) as usize;
    let harmonics = ((sr / 2.0 / freq).floor() as usize).min(30);
    (0..n)
        .map(|i| {
            let t = i as f64 / sr;
            let mut s = 0.0;
            for h in 1..=harmonics {
                s += (TAU * freq * h as f64 * t).sin() / h as f64;
            }
            let env = (t / 0.005).min(1.0) * ((dur - t) / 0.02).clamp(0.0, 1.0);
            // Gentle low-pass feel: scale down.
            s * 0.5 * env
        })
        .collect()
}

/// Karplus-Strong plucked string.
fn pluck(sr: f64, freq: f64, dur: f64, noise: &mut Noise) -> Vec<f64> {
    let n = (dur * sr) as usize;
    let period = (sr / freq).round() as usize;
    let mut line: Vec<f64> = (0..period).map(|_| noise.next()).collect();
    let mut out = Vec::with_capacity(n);
    let mut idx = 0;
    for i in 0..n {
        let next = (idx + 1) % period;
        let v = line[idx];
        line[idx] = 0.996 * 0.5 * (line[idx] + line[next]);
        idx = next;
        let t = i as f64 / sr;
        let release = ((dur - t) / 0.03).clamp(0.0, 1.0);
        out.push(v * release);
    }
    out
}

fn lead_note(sr: f64, freq: f64, dur: f64) -> Vec<f64> {
    let n = (dur * sr) as usize;
    (0..n)
        .map(|i| {
            let t = i as f64 / sr;
            let vib = 1.0 + 0.004 * (TAU * 5.5 * t).sin() * (t / 0.3).min(1.0);
            let ph = TAU * freq * vib * t;
            let s = ph.sin() + 0.3 * (2.0 * ph).sin() + 0.1 * (3.0 * ph).sin();
            let env = (t / 0.02).min(1.0) * ((dur - t) / 0.05).clamp(0.0, 1.0);
            s * 0.5 * env
        })
        .collect()
}

pub fn generate() -> Song {
    let sr = SAMPLE_RATE as f64;
    let frames = (DURATION_S * sr) as usize;
    let beat = 60.0 / BPM;
    let n_beats = (DURATION_S / beat).round() as usize; // 60
    let at = |t: f64| (t * sr).round() as usize;
    let mut noise = Noise(0x5eed_1234_abcd_ef01);
    let mut onsets = Onsets::default();

    // Chord roots per bar (MIDI): A, F, C, G.
    let chords: [[f64; 4]; 4] = [
        [57.0, 60.0, 64.0, 69.0], // Am: A3 C4 E4 A4
        [53.0, 57.0, 60.0, 65.0], // F:  F3 A3 C4 F4
        [55.0, 60.0, 64.0, 67.0], // C:  G3 C4 E4 G4
        [55.0, 59.0, 62.0, 67.0], // G:  G3 B3 D4 G4
    ];
    let bass_roots = [33.0, 29.0, 36.0, 31.0]; // A1 F1 C2 G1
    let lead_melody = [76.0, 74.0, 72.0, 71.0, 72.0, 74.0, 76.0, 79.0]; // E5 D5 C5 B4 ...

    let mut drums = Stereo::silent(frames);
    let mut bass = Stereo::silent(frames);
    let mut guitar = Stereo::silent(frames);
    let mut keys = Stereo::silent(frames);
    let mut lead = Stereo::silent(frames);

    let kick_s = kick(sr);
    let beat_times: Vec<f64> = (0..n_beats).map(|b| b as f64 * beat).collect();

    for (b, &t) in beat_times.iter().enumerate() {
        let bar = b / 4;
        let in_bar = b % 4;
        // Last bar is a quiet outro for the drums: stop one bar early.
        let drums_on = b + 4 < n_beats;
        if drums_on {
            if in_bar == 0 || in_bar == 2 {
                add_panned(&mut drums, at(t), &kick_s, 0.0);
                onsets.kick.push(t);
            } else {
                let s = snare(sr, &mut noise);
                add_panned(&mut drums, at(t), &s, 0.05);
                onsets.snare.push(t);
            }
            for half in 0..2 {
                let th = t + half as f64 * beat / 2.0;
                let h = hat(sr, &mut noise);
                add_panned(&mut drums, at(th), &h, 0.3);
                onsets.hat.push(th);
            }
        }

        // Bass: eighth notes on the bar root, octave jump on the "and" of 4.
        let root = bass_roots[bar % 4];
        for half in 0..2 {
            let th = t + half as f64 * beat / 2.0;
            let m = if in_bar == 3 && half == 1 {
                root + 12.0
            } else {
                root
            };
            let note = saw_note(sr, midi_to_hz(m), beat / 2.0 * 0.9);
            add_panned(&mut bass, at(th), &note, 0.0);
            onsets.bass.push(th);
        }

        // Guitar: strum the bar chord on beats 1 and 3 (strings 12 ms apart).
        if in_bar == 0 || in_bar == 2 {
            let chord = chords[bar % 4];
            for (k, &m) in chord.iter().enumerate() {
                let ts = t + k as f64 * 0.012;
                let p = pluck(sr, midi_to_hz(m), beat * 2.0 - 0.02, &mut noise);
                let scaled: Vec<f64> = p.iter().map(|v| v * 0.35).collect();
                add_panned(&mut guitar, at(ts), &scaled, -0.4);
            }
            onsets.guitar.push(t);
        }

        // Lead: one note per beat for the middle of the song (bars 4..12).
        if (4..12).contains(&bar) {
            let m = lead_melody[(b - 16) % lead_melody.len()];
            let note = lead_note(sr, midi_to_hz(m), beat * 0.85);
            add_panned(&mut lead, at(t), &note, 0.25);
            onsets.lead.push(t);
        }
    }

    // Keys: one sustained E4 pad for the whole song, slow attack and release.
    let attack = 1.0;
    let release = 1.0;
    let pad: Vec<f64> = (0..frames)
        .map(|i| {
            let t = i as f64 / sr;
            let env = (t / attack).min(1.0) * ((DURATION_S - t) / release).clamp(0.0, 1.0);
            let ph = TAU * PAD_FREQ_HZ * t;
            let s = ph.sin() + 0.35 * (2.0 * ph).sin() + 0.12 * (3.0 * ph).sin();
            s * env * 0.4
        })
        .collect();
    add_panned(&mut keys, 0, &pad, 0.35);

    let mut stems = vec![
        ("drums".to_string(), drums),
        ("bass".to_string(), bass),
        ("guitar".to_string(), guitar),
        ("keys".to_string(), keys),
        ("lead".to_string(), lead),
    ];

    // Normalise every stem by the same factor so the mix peak is 0.9.
    let raw_mix = sum_stems(&stems);
    let peak = raw_mix
        .left
        .iter()
        .chain(raw_mix.right.iter())
        .fold(0.0f32, |m, v| m.max(v.abs())) as f64;
    let scale = 0.9 / peak;
    for (_, s) in stems.iter_mut() {
        for v in s.left.iter_mut().chain(s.right.iter_mut()) {
            *v = (*v as f64 * scale) as f32;
        }
    }
    let mix = sum_stems(&stems);

    let truth = Truth {
        sample_rate: SAMPLE_RATE,
        bpm: BPM,
        duration_s: DURATION_S,
        frames,
        stems: stems.iter().map(|(n, _)| n.clone()).collect(),
        pad_frequency_hz: PAD_FREQ_HZ,
        pad_sustain_s: (attack, DURATION_S - release),
        beat_times,
        onsets,
        normalisation: scale,
    };
    Song { stems, mix, truth }
}

/// Plain sum of stems, channel by channel, in f32 (no clamping).
pub fn sum_stems(stems: &[(String, Stereo)]) -> Stereo {
    let n = stems[0].1.left.len();
    let mut out = Stereo::silent(n);
    for (_, s) in stems {
        for i in 0..n {
            out.left[i] += s.left[i];
            out.right[i] += s.right[i];
        }
    }
    out
}

pub fn write_wav(path: &std::path::Path, s: &Stereo) -> anyhow::Result<()> {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut w = hound::WavWriter::create(path, spec)?;
    for i in 0..s.left.len() {
        w.write_sample(s.left[i])?;
        w.write_sample(s.right[i])?;
    }
    w.finalize()?;
    Ok(())
}

/// Writes `mix.wav`, one WAV per stem, `truth.json`, and (if ffmpeg exists)
/// `mix.mp3` into `dir`.
pub fn write_song(song: &Song, dir: &std::path::Path, mp3: bool) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    write_wav(&dir.join("mix.wav"), &song.mix)?;
    for (name, s) in &song.stems {
        write_wav(&dir.join(format!("{name}.wav")), s)?;
    }
    std::fs::write(
        dir.join("truth.json"),
        serde_json::to_string_pretty(&song.truth)?,
    )?;
    if mp3 {
        let status = std::process::Command::new("ffmpeg")
            .args(["-y", "-loglevel", "error", "-i"])
            .arg(dir.join("mix.wav"))
            .args(["-codec:a", "libmp3lame", "-b:a", "192k"])
            .arg(dir.join("mix.mp3"))
            .status();
        match status {
            Ok(s) if s.success() => {}
            other => eprintln!("ffmpeg MP3 encode skipped: {other:?}"),
        }
    }
    Ok(())
}
