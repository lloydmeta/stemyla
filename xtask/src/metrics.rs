//! Numeric checks used by tests and benches.

/// Signal-to-distortion ratio in dB: `10·log10(‖s‖² / ‖s − ŝ‖²)`.
pub fn sdr(reference: &[f32], estimate: &[f32]) -> f64 {
    let n = reference.len().min(estimate.len());
    let mut num = 0.0f64;
    let mut den = 0.0f64;
    for i in 0..n {
        let s = reference[i] as f64;
        let e = estimate[i] as f64;
        num += s * s;
        den += (s - e) * (s - e);
    }
    10.0 * (num / den.max(1e-20)).log10()
}

/// Lag (in samples, within ±max_lag) that maximises the cross-correlation
/// of `a` against `b`.  Positive means `a` is late relative to `b`.
pub fn best_lag(a: &[f32], b: &[f32], max_lag: isize) -> isize {
    let n = a.len().min(b.len()) as isize;
    let mut best = (0isize, f64::MIN);
    for lag in -max_lag..=max_lag {
        let mut acc = 0.0f64;
        for i in 0..n {
            let j = i - lag;
            if j >= 0 && j < n {
                acc += a[i as usize] as f64 * b[j as usize] as f64;
            }
        }
        if acc > best.1 {
            best = (lag, acc);
        }
    }
    best.0
}

/// Fundamental frequency estimate by autocorrelation with parabolic
/// interpolation, searched between `fmin` and `fmax`.
pub fn pitch_autocorr(x: &[f32], sample_rate: f64, fmin: f64, fmax: f64) -> f64 {
    let min_lag = (sample_rate / fmax).floor() as usize;
    let max_lag = (sample_rate / fmin).ceil() as usize;
    let n = x.len() - max_lag - 1;
    let ac = |lag: usize| -> f64 {
        let mut s = 0.0;
        for i in 0..n {
            s += x[i] as f64 * x[i + lag] as f64;
        }
        s
    };
    let vals: Vec<f64> = (min_lag - 1..=max_lag + 1).map(ac).collect();
    // First strong peak: pick the global max within range.
    let mut best = 1;
    for k in 1..vals.len() - 1 {
        if vals[k] > vals[best] {
            best = k;
        }
    }
    let (y0, y1, y2) = (vals[best - 1], vals[best], vals[best + 1]);
    let denom = y0 - 2.0 * y1 + y2;
    let delta = if denom.abs() > 1e-12 {
        0.5 * (y0 - y2) / denom
    } else {
        0.0
    };
    let lag = (min_lag - 1 + best) as f64 + delta;
    sample_rate / lag
}

/// Peak frequency of a windowed DFT evaluated on a fine grid around `guess`
/// (±`span` Hz).  Slow but exact enough for a single sustained note.
pub fn peak_freq_near(x: &[f32], sample_rate: f64, guess: f64, span: f64) -> f64 {
    let n = x.len();
    let w: Vec<f64> = (0..n)
        .map(|i| 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos())
        .collect();
    let mut best = (guess, 0.0f64);
    let steps = 400;
    for k in 0..=steps {
        let f = guess - span + 2.0 * span * k as f64 / steps as f64;
        let (mut re, mut im) = (0.0, 0.0);
        let dphi = std::f64::consts::TAU * f / sample_rate;
        for i in 0..n {
            let v = x[i] as f64 * w[i];
            let ph = dphi * i as f64;
            re += v * ph.cos();
            im -= v * ph.sin();
        }
        let mag = re * re + im * im;
        if mag > best.1 {
            best = (f, mag);
        }
    }
    best.0
}

/// Largest absolute sample-to-sample difference in `x[from..to]`.
pub fn max_jump(x: &[f32], from: usize, to: usize) -> f32 {
    let mut m = 0.0f32;
    for i in from.max(1)..to.min(x.len()) {
        m = m.max((x[i] - x[i - 1]).abs());
    }
    m
}

/// Median absolute sample-to-sample difference over the whole signal.
pub fn median_jump(x: &[f32]) -> f32 {
    let mut d: Vec<f32> = (1..x.len()).map(|i| (x[i] - x[i - 1]).abs()).collect();
    d.sort_by(|a, b| a.partial_cmp(b).unwrap());
    d[d.len() / 2]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sdr_of_identical_is_huge_and_of_silence_is_zero() {
        let s = [0.5f32, -0.25, 0.1];
        assert!(sdr(&s, &s) > 100.0);
        assert!((sdr(&s, &[0.0; 3]) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn lag_finds_a_shift() {
        let a: Vec<f32> = (0..500).map(|i| ((i * 7919) % 101) as f32 - 50.0).collect();
        let mut b = vec![0.0; 3];
        b.extend_from_slice(&a[..497]);
        assert_eq!(best_lag(&b, &a, 10), 3);
    }

    #[test]
    fn pitch_of_a_sine() {
        let sr = 44_100.0;
        let x: Vec<f32> = (0..8192)
            .map(|i| (std::f64::consts::TAU * 329.63 * i as f64 / sr).sin() as f32)
            .collect();
        assert!((pitch_autocorr(&x, sr, 200.0, 500.0) - 329.63).abs() < 0.5);
        assert!((peak_freq_near(&x, sr, 330.0, 10.0) - 329.63).abs() < 0.1);
    }
}
