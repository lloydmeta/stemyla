//! Converts a channel from one sample rate to another.  Every song in the
//! app is at [`SONG_RATE`], the separation model's rate, so a song at
//! another rate is converted when it is opened (decision 0012).
//!
//! The converter is a rational polyphase filter: a Blackman-windowed sinc,
//! tabulated once for each of the `L` positions between input samples that
//! the output can fall on.  From 48 kHz to 44.1 kHz, `L` is 147 and the
//! input moves 160 samples for every 147 output samples.

use anyhow::{Context, Result, ensure};
use std::f64::consts::PI;

/// The sample rate of every song in the app: the rate the separation
/// model was trained at (`roformer::SAMPLE_RATE`).
pub const SONG_RATE: u32 = 44_100;

/// The rates `to_rate` takes, in Hz.  A file's header can state any rate,
/// and the filter table and the output grow with it, so the rest are
/// refused (MEM-4).
const RATES: std::ops::RangeInclusive<u32> = 8_000..=384_000;

/// The most filter phases a conversion may need.  Every standard rate
/// needs 441 or fewer: 8 kHz to 44.1 kHz needs the most.
const MAX_PHASES: u64 = 441;

/// Input samples on each side of an output sample at 48 kHz, where the
/// filter's transition is 5.5 / 64 of 48 kHz, about 4.1 kHz wide.  A
/// higher input rate takes more taps, so the transition stays as wide.
const HALF_TAPS: usize = 32;

/// When the rate rises, the input's own band ends at half its rate.  More
/// taps make the transition below it narrow (4.3 % of the rate).
const RISING_HALF_TAPS: usize = 64;

/// `x`, sampled at `from` Hz, as samples at `to` Hz.  At the same rate it
/// returns `x` itself, not a copy (MEM-1).  Otherwise the output has
/// `ceil(x.len() * to / from)` samples, and `x` is dropped once they are
/// made.  A rate outside 8 kHz to 384 kHz, or one that needs more than
/// [`MAX_PHASES`] phases, is refused.
pub fn to_rate(x: Vec<f32>, from: u32, to: u32) -> Result<Vec<f32>> {
    if from == to {
        return Ok(x);
    }
    Filter::new(from, to)?.apply(&x)
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 { a } else { gcd(b, a % b) }
}

/// `ceil(frames * phases / step)`.  The arithmetic is in u64, not usize: on
/// wasm32 a usize product overflows after about 10 minutes at 48 kHz.  The
/// types hold this on every host, so a native test checks it too.
fn output_len(frames: u64, phases: u64, step: u64) -> u64 {
    (frames * phases).div_ceil(step)
}

/// Where output `i` falls: `phase / phases` of the way past input `at`,
/// that is `at = i * step / phases` and `phase = i * step % phases`.  Both
/// are stepped in u64, not computed from `i`, for the same reason as
/// [`output_len`].
#[derive(Clone, Copy, Debug, PartialEq)]
struct Position {
    at: u64,
    phase: u64,
}

impl Position {
    fn next(self, phases: u64, step: u64) -> Position {
        let (at, phase) = (self.at + step / phases, self.phase + step % phases);
        if phase >= phases {
            Position {
                at: at + 1,
                phase: phase - phases,
            }
        } else {
            Position { at, phase }
        }
    }
}

struct Filter {
    /// `L`: the output steps through this many positions between two
    /// input samples.
    phases: u64,
    /// `M`: input samples for every `L` output samples.
    step: u64,
    half: usize,
    /// `phases` rows of `2 * half` taps.  Row `p` is the filter for an
    /// output that falls `p / phases` of the way past an input sample.
    table: Vec<f32>,
}

impl Filter {
    fn new(from: u32, to: u32) -> Result<Filter> {
        for rate in [from, to] {
            ensure!(
                RATES.contains(&rate),
                "songs at {rate} Hz are not supported; only 8 kHz to 384 kHz are"
            );
        }
        let g = gcd(from, to);
        let (phases, step) = (u64::from(to / g), u64::from(from / g));
        ensure!(
            phases <= MAX_PHASES,
            "songs at {from} Hz are not supported; convert them to {to} Hz first"
        );
        let (half, cutoff) = if to > from {
            // The stop band starts at half the input rate, so no image of
            // the input's band falls below it.  The cutoff is the middle of
            // the transition, as a fraction of half the input rate.
            let half = RISING_HALF_TAPS;
            (half, 1.0 - 5.5 / (2 * half) as f64)
        } else {
            // The cutoff is half the output rate.  The transition is
            // centred on it, so about 2 kHz of the input's band above it
            // folds back: from 48 kHz, to between 20 kHz and 22.05 kHz.
            let scaled = (HALF_TAPS as u64 * u64::from(from)).div_ceil(48_000) as usize;
            // Even, so that the taps are a multiple of 4 for `dot`.
            let half = scaled.next_multiple_of(2).max(HALF_TAPS);
            (half, f64::from(to) / f64::from(from))
        };
        let taps = 2 * half;
        // `dot` sums four taps at a time and would drop a remainder.
        assert_eq!(taps % 4, 0, "{taps} taps");
        let mut table = Vec::with_capacity(phases as usize * taps);
        for p in 0..phases {
            let past = p as f64 / phases as f64;
            let row: Vec<f64> = (0..taps)
                .map(|k| {
                    // Input sample k of the row is this far from the output.
                    let t = (k as f64 - (half as f64 - 1.0)) - past;
                    cutoff * sinc(cutoff * t) * blackman(t / half as f64)
                })
                .collect();
            // Each row sums to 1, so a constant passes exactly.
            let sum: f64 = row.iter().sum();
            table.extend(row.iter().map(|h| (h / sum) as f32));
        }
        Ok(Filter {
            phases,
            step,
            half,
            table,
        })
    }

    fn apply(&self, x: &[f32]) -> Result<Vec<f32>> {
        let taps = 2 * self.half;
        // A refusal here is an error, not a panic in the page.
        let too_long = "the song is too long to convert at this rate";
        let len = usize::try_from(output_len(x.len() as u64, self.phases, self.step))
            .context(too_long)?;
        let mut out = Vec::new();
        out.try_reserve_exact(len).context(too_long)?;
        // Input outside the song counts as silence.
        let mut edge = vec![0.0; taps];
        let mut pos = Position { at: 0, phase: 0 };
        for _ in 0..len {
            let row = &self.table[pos.phase as usize * taps..][..taps];
            // The first input sample of the window, which may be before 0.
            // `at` is at most `x.len()`, so it fits an isize.
            let start = pos.at as isize - (self.half as isize - 1);
            let window = match usize::try_from(start) {
                Ok(s) if s + taps <= x.len() => &x[s..s + taps],
                _ => {
                    for (k, e) in edge.iter_mut().enumerate() {
                        let i = start + k as isize;
                        *e = usize::try_from(i)
                            .ok()
                            .and_then(|i| x.get(i))
                            .copied()
                            .unwrap_or(0.0);
                    }
                    &edge
                }
            };
            out.push(dot(window, row));
            pos = pos.next(self.phases, self.step);
        }
        Ok(out)
    }
}

/// Four sums run side by side, so the compiler can keep them in a vector
/// register.  On the maintainer's Mac this was 2.5 times faster than one
/// sum, natively.
fn dot(x: &[f32], h: &[f32]) -> f32 {
    let mut sums = [0.0f32; 4];
    for (xs, hs) in x.chunks_exact(4).zip(h.chunks_exact(4)) {
        for k in 0..4 {
            sums[k] += xs[k] * hs[k];
        }
    }
    (sums[0] + sums[1]) + (sums[2] + sums[3])
}

fn sinc(t: f64) -> f64 {
    if t.abs() < 1e-12 {
        1.0
    } else {
        (PI * t).sin() / (PI * t)
    }
}

/// The Blackman window over -1 to 1, and 0 outside.
fn blackman(w: f64) -> f64 {
    if w.abs() >= 1.0 {
        0.0
    } else {
        0.42 + 0.5 * (PI * w).cos() + 0.08 * (2.0 * PI * w).cos()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AMP: f64 = 0.5;

    fn tone(hz: f64, rate: u32, frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|i| (AMP * (2.0 * PI * hz * i as f64 / f64::from(rate)).sin()) as f32)
            .collect()
    }

    /// Fits `y` to a sine of `hz` at `rate` by least squares, away from the
    /// edges.  Returns the fit's gain against [`AMP`] and the ratio of the
    /// fit to what is left, both in dB.
    fn gain_and_snr(y: &[f32], hz: f64, rate: u32) -> (f64, f64) {
        let (a, b) = (2_000, y.len() - 2_000);
        let w = |i: usize| 2.0 * PI * hz * i as f64 / f64::from(rate);
        let (mut ss, mut sc, mut cc, mut ys, mut yc) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for (i, &v) in y.iter().enumerate().take(b).skip(a) {
            let (s, c) = w(i).sin_cos();
            (ss, sc, cc) = (ss + s * s, sc + s * c, cc + c * c);
            (ys, yc) = (ys + f64::from(v) * s, yc + f64::from(v) * c);
        }
        let det = ss * cc - sc * sc;
        let (p, q) = ((ys * cc - yc * sc) / det, (yc * ss - ys * sc) / det);
        let (mut signal, mut noise) = (0.0, 0.0);
        for (i, &v) in y.iter().enumerate().take(b).skip(a) {
            let (s, c) = w(i).sin_cos();
            let fit = p * s + q * c;
            signal += fit * fit;
            noise += (f64::from(v) - fit).powi(2);
        }
        let gain = 20.0 * ((p * p + q * q).sqrt() / AMP).log10();
        (gain, 10.0 * (signal / noise).log10())
    }

    fn converted(hz: f64, from: u32) -> Vec<f32> {
        to_rate(tone(hz, from, 2 * from as usize), from, SONG_RATE).unwrap()
    }

    #[test]
    fn a_second_at_48k_is_a_second_at_44k() {
        assert_eq!(
            to_rate(vec![0.0; 48_000], 48_000, 44_100).unwrap().len(),
            44_100
        );
        assert_eq!(to_rate(vec![0.0; 1], 48_000, 44_100).unwrap().len(), 1);
        assert_eq!(to_rate(vec![0.0; 161], 48_000, 44_100).unwrap().len(), 148);
        assert!(to_rate(Vec::new(), 48_000, 44_100).unwrap().is_empty());
    }

    #[test]
    fn the_pass_band_is_flat_to_20_khz() {
        // Every 500 Hz, since the SNR swings by up to 13.4 dB between tones.
        for hz in (0..40)
            .map(|k| 100.0 + 500.0 * f64::from(k))
            .chain([20_000.0])
        {
            let (gain, snr) = gain_and_snr(&converted(hz, 48_000), hz, SONG_RATE);
            let least_snr = if hz <= 18_000.0 { 95.0 } else { 85.0 };
            assert!(gain.abs() < 0.01, "{hz} Hz: gain {gain:.4} dB");
            assert!(snr >= least_snr, "{hz} Hz: SNR {snr:.1} dB");
        }
    }

    #[test]
    fn an_alias_stays_above_20_khz() {
        // 23 kHz is above 44.1 kHz's 22.05 kHz, so it folds to 21.1 kHz.
        let y = converted(23_000.0, 48_000);
        let (alias, _) = gain_and_snr(&y, 21_100.0, SONG_RATE);
        assert!(alias <= -20.0, "alias at {alias:.1} dB");
    }

    #[test]
    fn upsampling_from_32k_keeps_a_tone() {
        assert_eq!(
            to_rate(vec![0.0; 32_000], 32_000, 44_100).unwrap().len(),
            44_100
        );
        for hz in [1_000.0, 10_000.0, 14_000.0] {
            let (gain, snr) = gain_and_snr(&converted(hz, 32_000), hz, SONG_RATE);
            assert!(gain.abs() < 0.01, "{hz} Hz: gain {gain:.4} dB");
            assert!(snr >= 95.0, "{hz} Hz: SNR {snr:.1} dB");
        }
    }

    #[test]
    fn a_rising_rate_makes_no_audible_image() {
        // A tone f below half the input rate has an image at `from - f`,
        // just above half the input rate, in the audible band.
        for (from, hz) in [
            (16_000, 7_800.0),
            (22_050, 10_800.0),
            (24_000, 11_800.0),
            (32_000, 15_500.0),
        ] {
            let image = f64::from(from) - hz;
            let (gain, _) = gain_and_snr(&converted(hz, from), image, SONG_RATE);
            assert!(
                gain <= -60.0,
                "{from} Hz: image at {image} Hz, {gain:.1} dB"
            );
        }
    }

    #[test]
    fn a_large_fall_keeps_the_pass_band() {
        // The taps grow with the input rate, also for a ratio just under 2.
        for from in [64_000, 88_000, 192_000, 384_000] {
            for hz in [18_000.0, 20_000.0] {
                let (gain, snr) = gain_and_snr(&converted(hz, from), hz, SONG_RATE);
                assert!(gain.abs() < 0.01, "{from} Hz, {hz} Hz: gain {gain:.4} dB");
                assert!(snr >= 85.0, "{from} Hz, {hz} Hz: SNR {snr:.1} dB");
            }
        }
    }

    #[test]
    fn every_standard_rate_converts() {
        for from in [
            8_000, 11_025, 12_000, 16_000, 22_050, 24_000, 32_000, 48_000, 88_200, 96_000, 176_400,
            192_000,
        ] {
            let y = to_rate(vec![0.25; from as usize / 5], from, SONG_RATE).unwrap();
            assert_eq!(y.len(), 8_820, "{from} Hz: 0.2 s");
            // Away from the edges, a constant passes exactly.
            let mid = y[4_410];
            assert!((mid - 0.25).abs() < 1e-6, "{from} Hz: {mid}");
        }
    }

    #[test]
    fn input_outside_the_song_counts_as_silence() {
        // 640 frames at 48 kHz are 588 at 44.1 kHz, a whole number, so the
        // padded song's outputs fall on the same phases as the song's.  The
        // offset keeps the first and last samples away from 0.
        let x: Vec<f32> = tone(440.0, 48_000, 1_000).iter().map(|v| v + 0.1).collect();
        let padded: Vec<f32> = [vec![0.0; 640], x.clone(), vec![0.0; 640]].concat();
        let a = to_rate(x, 48_000, SONG_RATE).unwrap();
        let b = to_rate(padded, 48_000, SONG_RATE).unwrap();
        let bits = |v: &[f32]| v.iter().map(|s| s.to_bits()).collect::<Vec<_>>();
        assert_eq!(bits(&a), bits(&b[588..588 + a.len()]));
    }

    #[test]
    fn the_output_is_not_delayed() {
        // A click 0.02 s in stays 0.02 s in: frame 882 at 44.1 kHz.  The
        // tone tests cannot see a delay, since a sine fit takes any phase.
        for from in [32_000, 48_000, 96_000] {
            let mut x = vec![0.0; from as usize / 10];
            x[from as usize / 50] = 1.0;
            let y = to_rate(x, from, SONG_RATE).unwrap();
            let peak = (0..y.len()).max_by(|&a, &b| y[a].total_cmp(&y[b])).unwrap();
            assert_eq!(peak, 882, "{from} Hz");
        }
    }

    #[test]
    fn the_length_and_the_position_do_not_overflow_32_bits() {
        // 30,000,000 frames at 48 kHz, about 10.4 minutes: frames * 147 and
        // i * 160 are both above 2^32.
        let (phases, step) = (147, 160);
        assert_eq!(output_len(30_000_000, phases, step), 27_562_500);
        let mut pos = Position { at: 0, phase: 0 };
        for i in 1..=27_562_500_u64 {
            pos = pos.next(phases, step);
            if i % 1_000_003 == 0 || i == 27_562_500 {
                let want = Position {
                    at: i * 160 / 147,
                    phase: i * 160 % 147,
                };
                assert_eq!(pos, want, "output {i}");
            }
        }
    }

    #[test]
    fn the_same_rate_is_not_copied() {
        let x = tone(440.0, SONG_RATE, 1_000);
        let (at, bits) = (
            x.as_ptr(),
            x.iter().map(|s| s.to_bits()).collect::<Vec<_>>(),
        );
        let y = to_rate(x, SONG_RATE, SONG_RATE).unwrap();
        assert_eq!(y.as_ptr(), at);
        assert_eq!(y.iter().map(|s| s.to_bits()).collect::<Vec<_>>(), bits);
    }

    #[test]
    fn an_odd_rate_is_refused() {
        // 44,056 / 44,100 is 11,014 / 11,025 in lowest terms.
        let e = to_rate(vec![0.0; 10], 44_056, SONG_RATE).unwrap_err();
        assert!(e.to_string().contains("44056 Hz are not supported"), "{e}");
        assert!(to_rate(vec![0.0; 10], 0, SONG_RATE).is_err());
    }

    #[test]
    fn a_rate_outside_8_to_384_khz_is_refused() {
        // Each of these needs 441 phases or fewer, so only the range
        // refuses them: a tiny rate makes a huge song, a huge rate a huge
        // filter table.
        for from in [100, 7_000, 7_900, 384_100, 400_000, 4_000_000] {
            let e = to_rate(vec![0.0; 10], from, SONG_RATE).unwrap_err();
            assert!(
                e.to_string().contains("only 8 kHz to 384 kHz"),
                "{from} Hz: {e}"
            );
        }
        // 44.1 kHz to 441 kHz needs 10 phases, so only the range refuses it.
        let e = to_rate(vec![0.0; 10], SONG_RATE, 441_000).unwrap_err();
        assert!(e.to_string().contains("only 8 kHz to 384 kHz"), "{e}");
    }
}
