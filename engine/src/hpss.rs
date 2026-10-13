//! Classic (non-AI) harmonic/percussive separation by median filtering.
//!
//! In a spectrogram, steady notes are horizontal lines (one frequency held
//! over time) and drum hits are vertical lines (many frequencies at one
//! instant).  A median filter along time keeps the horizontal lines; a median
//! filter along frequency keeps the vertical ones.  Comparing the two gives a
//! mask for each part (Fitzgerald 2010; margins as in Driedger et al. 2014).
//!
//! Every function here works on one channel.

use realfft::RealFftPlanner;
use realfft::num_complex::Complex32;

#[derive(Clone, Copy, Debug)]
pub struct HpssConfig {
    /// FFT size (frames of this many samples).
    pub n_fft: usize,
    /// Step between frames.  `n_fft / 4` gives exact reconstruction.
    pub hop: usize,
    /// Median length along time, in frames (odd).  Longer favours sustain.
    pub harmonic_len: usize,
    /// Median length along frequency, in bins (odd).  Longer favours hits.
    pub percussive_len: usize,
    /// 1.0: soft masks that add up to exactly 1, so harmonic + percussive
    /// reconstructs the input.  Above 1: stricter binary masks, and whatever
    /// neither part claims is returned as the residual.
    pub margin: f32,
    /// Exponent of the soft (Wiener-like) masks used when `margin == 1`.
    pub power: f32,
}

impl Default for HpssConfig {
    fn default() -> Self {
        Self {
            n_fft: 2048,
            hop: 512,
            harmonic_len: 17,
            percussive_len: 17,
            margin: 1.0,
            power: 2.0,
        }
    }
}

/// A complex short-time Fourier transform: `frames[t][k]`.
pub struct Stft {
    pub n_fft: usize,
    pub hop: usize,
    /// Number of input samples, so the inverse can trim the padding.
    pub len: usize,
    pub frames: Vec<Vec<Complex32>>,
}

fn hann(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n as f32).cos())
        .collect()
}

/// STFT with a periodic Hann window.  The signal is padded by `n_fft` on
/// both sides so the first and last samples are fully covered.
pub fn stft(x: &[f32], n_fft: usize, hop: usize) -> Stft {
    let mut planner = RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(n_fft);
    let w = hann(n_fft);
    let padded_len = x.len() + 2 * n_fft;
    let n_frames = (padded_len - n_fft) / hop + 1;
    let mut frames = Vec::with_capacity(n_frames);
    let mut buf = fft.make_input_vec();
    let mut scratch = fft.make_scratch_vec();
    for t in 0..n_frames {
        let start = t * hop;
        for i in 0..n_fft {
            let j = start + i;
            let v = if j >= n_fft && j - n_fft < x.len() {
                x[j - n_fft]
            } else {
                0.0
            };
            buf[i] = v * w[i];
        }
        let mut spec = fft.make_output_vec();
        fft.process_with_scratch(&mut buf, &mut spec, &mut scratch)
            .expect("buffer sizes match the plan");
        frames.push(spec);
    }
    Stft {
        n_fft,
        hop,
        len: x.len(),
        frames,
    }
}

/// Inverse STFT by weighted overlap-add (Hann synthesis window, normalised
/// by the summed squared window), trimmed back to the original length.
pub fn istft(s: &Stft) -> Vec<f32> {
    let n = s.n_fft;
    let mut planner = RealFftPlanner::<f32>::new();
    let ifft = planner.plan_fft_inverse(n);
    let w = hann(n);
    let total = (s.frames.len() - 1) * s.hop + n;
    let mut out = vec![0.0f32; total];
    let mut norm = vec![0.0f32; total];
    let mut buf = ifft.make_output_vec();
    let mut scratch = ifft.make_scratch_vec();
    for (t, frame) in s.frames.iter().enumerate() {
        let mut spec = frame.clone();
        // A real signal's spectrum has real DC and Nyquist bins.
        spec[0].im = 0.0;
        let last = spec.len() - 1;
        spec[last].im = 0.0;
        ifft.process_with_scratch(&mut spec, &mut buf, &mut scratch)
            .expect("buffer sizes match the plan");
        let start = t * s.hop;
        for i in 0..n {
            out[start + i] += buf[i] / n as f32 * w[i];
            norm[start + i] += w[i] * w[i];
        }
    }
    (0..s.len)
        .map(|i| {
            let j = i + n;
            if norm[j] > 1e-8 {
                out[j] / norm[j]
            } else {
                0.0
            }
        })
        .collect()
}

/// Median of a small odd-length window, sorting a scratch copy.  Tested to
/// match the beginner-style `dsp::median`.
pub fn fast_median(values: &[f32], scratch: &mut Vec<f32>) -> f32 {
    scratch.clear();
    scratch.extend_from_slice(values);
    let mid = scratch.len() / 2;
    *scratch
        .select_nth_unstable_by(mid, |a, b| a.partial_cmp(b).unwrap())
        .1
}

/// Median filter along time (for each bin) and along frequency (for each
/// frame) of a magnitude spectrogram `mag[t][k]`.  Edges are padded by
/// repeating the nearest value.  Returns (harmonic-enhanced, percussive-enhanced).
pub fn median_filters(mag: &[Vec<f32>], lh: usize, lp: usize) -> (Vec<Vec<f32>>, Vec<Vec<f32>>) {
    assert!(lh % 2 == 1 && lp % 2 == 1, "median lengths must be odd");
    let t_len = mag.len();
    let k_len = mag.first().map_or(0, Vec::len);
    let mut h = vec![vec![0.0f32; k_len]; t_len];
    let mut p = vec![vec![0.0f32; k_len]; t_len];
    let mut win = Vec::with_capacity(lh.max(lp));
    let mut scratch = Vec::with_capacity(lh.max(lp));
    let (rh, rp) = (lh / 2, lp / 2);
    for t in 0..t_len {
        for k in 0..k_len {
            win.clear();
            for d in 0..lh {
                let tt = (t + d).saturating_sub(rh).min(t_len - 1);
                win.push(mag[tt][k]);
            }
            h[t][k] = fast_median(&win, &mut scratch);
            win.clear();
            for d in 0..lp {
                let kk = (k + d).saturating_sub(rp).min(k_len - 1);
                win.push(mag[t][kk]);
            }
            p[t][k] = fast_median(&win, &mut scratch);
        }
    }
    (h, p)
}

pub struct HpssOutput {
    pub harmonic: Vec<f32>,
    pub percussive: Vec<f32>,
    /// Non-empty only when `margin > 1`.
    pub residual: Vec<f32>,
}

/// Separates one channel into harmonic and percussive parts.
pub fn hpss(x: &[f32], cfg: &HpssConfig) -> HpssOutput {
    let s = stft(x, cfg.n_fft, cfg.hop);
    let mag: Vec<Vec<f32>> = s
        .frames
        .iter()
        .map(|f| f.iter().map(|c| c.norm()).collect())
        .collect();
    let (h, p) = median_filters(&mag, cfg.harmonic_len, cfg.percussive_len);
    let mut sh = Stft {
        n_fft: s.n_fft,
        hop: s.hop,
        len: s.len,
        frames: s.frames.clone(),
    };
    let mut sp = Stft {
        n_fft: s.n_fft,
        hop: s.hop,
        len: s.len,
        frames: s.frames.clone(),
    };
    let mut sr = (cfg.margin > 1.0).then(|| Stft {
        n_fft: s.n_fft,
        hop: s.hop,
        len: s.len,
        frames: s.frames.clone(),
    });
    for t in 0..s.frames.len() {
        for k in 0..s.frames[t].len() {
            let (hv, pv) = (h[t][k], p[t][k]);
            let (mh, mp) = if cfg.margin <= 1.0 {
                let a = hv.powf(cfg.power);
                let b = pv.powf(cfg.power);
                if a + b > 1e-20 {
                    (a / (a + b), b / (a + b))
                } else {
                    (0.5, 0.5)
                }
            } else {
                let mh = if hv > cfg.margin * pv { 1.0 } else { 0.0 };
                let mp = if pv > cfg.margin * hv { 1.0 } else { 0.0 };
                (mh, mp)
            };
            sh.frames[t][k] *= mh;
            sp.frames[t][k] *= mp;
            if let Some(r) = &mut sr {
                r.frames[t][k] *= 1.0 - mh - mp;
            }
        }
    }
    HpssOutput {
        harmonic: istft(&sh),
        percussive: istft(&sp),
        residual: sr.map(|r| istft(&r)).unwrap_or_default(),
    }
}

/// Magnitude spectrogram for display: `rows` frequency rows from 0 Hz up to
/// `max_hz`, at most `max_cols` time columns, values in dB.
pub fn spectrogram_db(
    x: &[f32],
    sample_rate: u32,
    max_hz: f32,
    rows: usize,
    max_cols: usize,
) -> (Vec<f32>, usize, usize) {
    let n_fft = 1024;
    let hop = (x.len() / max_cols.max(1)).max(256);
    let s = stft(x, n_fft, hop);
    let bins = n_fft / 2 + 1;
    let top_bin = ((max_hz / sample_rate as f32) * n_fft as f32).min((bins - 1) as f32) as usize;
    let cols = s.frames.len();
    let mut img = vec![-120.0f32; rows * cols];
    for (c, f) in s.frames.iter().enumerate() {
        for r in 0..rows {
            // Row 0 is the top of the image (highest frequency).
            let k0 = top_bin * (rows - 1 - r) / rows;
            let k1 = (top_bin * (rows - r) / rows).max(k0 + 1);
            let mut m = 0.0f32;
            for k in k0..k1.min(bins) {
                m = m.max(f[k].norm());
            }
            img[r * cols + c] = 20.0 * (m / (n_fft as f32 / 4.0) + 1e-9).log10();
        }
    }
    (img, cols, rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(n: usize, seed: u32) -> Vec<f32> {
        let mut x = seed.max(1);
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as f32 / u32::MAX as f32 - 0.5
            })
            .collect()
    }

    #[test]
    fn stft_round_trip_is_exact() {
        let x = noise(10_000, 7);
        let y = istft(&stft(&x, 2048, 512));
        let err = x
            .iter()
            .zip(&y)
            .fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
        assert!(err < 1e-5, "max error {err}");
    }

    #[test]
    fn fast_median_matches_dsp_median() {
        let mut scratch = Vec::new();
        let mut seed = 1;
        for len in [1usize, 3, 5, 9, 17, 31] {
            for _ in 0..200 {
                seed += 1;
                let v = noise(len, seed);
                assert_eq!(fast_median(&v, &mut scratch), dsp::median(&v), "len {len}");
            }
        }
    }

    #[test]
    fn parts_add_back_up_at_margin_one() {
        let x = noise(20_000, 3);
        let out = hpss(&x, &HpssConfig::default());
        let err = (0..x.len())
            .map(|i| (out.harmonic[i] + out.percussive[i] - x[i]).abs())
            .fold(0.0f32, f32::max);
        assert!(err < 1e-4, "max reconstruction error {err}");
        assert!(out.residual.is_empty());
    }

    #[test]
    fn a_click_is_percussive_and_a_tone_is_harmonic() {
        let sr = 44_100.0;
        let n = 44_100;
        let tone: Vec<f32> = (0..n)
            .map(|i| 0.3 * (std::f32::consts::TAU * 440.0 * i as f32 / sr).sin())
            .collect();
        let mut clicks = vec![0.0f32; n];
        for c in (5_000..n).step_by(11_025) {
            clicks[c] = 0.9;
        }
        let mix: Vec<f32> = tone.iter().zip(&clicks).map(|(a, b)| a + b).collect();
        let out = hpss(&mix, &HpssConfig::default());
        let energy = |v: &[f32]| v.iter().map(|x| (*x as f64).powi(2)).sum::<f64>();
        let err =
            |a: &[f32], b: &[f32]| energy(&a.iter().zip(b).map(|(x, y)| x - y).collect::<Vec<_>>());
        // Each part is closer to its own source than the mix is: the tone
        // loses the clicks and the clicks lose most of the tone.
        let sdr = |r: &[f32], e: &[f32]| 10.0 * (energy(r) / err(r, e)).log10();
        let (h, h0) = (sdr(&tone, &out.harmonic), sdr(&tone, &mix));
        let (p, p0) = (sdr(&clicks, &out.percussive), sdr(&clicks, &mix));
        println!("tone SDR {h:.1} dB (mix {h0:.1}); clicks SDR {p:.1} dB (mix {p0:.1})");
        assert!(p > p0 + 20.0, "clicks: {p:.1} vs {p0:.1}");
        assert!(h > 15.0, "tone: {h:.1} dB");
    }

    #[test]
    fn spectrogram_has_requested_shape() {
        let x = noise(44_100, 9);
        let (img, cols, rows) = spectrogram_db(&x, 44_100, 8_000.0, 64, 200);
        assert_eq!(rows, 64);
        assert!(cols > 100 && cols <= 210);
        assert_eq!(img.len(), rows * cols);
    }
}
