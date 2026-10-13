//! STFT and inverse STFT with the conventions of `torch.stft` and
//! `torch.istft` as BS-RoFormer calls them: periodic Hann window,
//! `center=True` with reflect padding, one-sided spectrum, no
//! normalisation, inverse divided by the overlap-added squared window.

use realfft::RealFftPlanner;
use realfft::num_complex::Complex;

use crate::{HOP, N_FFT};

/// One-sided spectrum, frame-major: `data[frame * bins + bin]`.
#[derive(Clone, Debug)]
pub struct Spectrum {
    pub frames: usize,
    pub bins: usize,
    pub data: Vec<Complex<f32>>,
}

/// `torch.hann_window(n)` (periodic).
pub fn hann(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| (0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / n as f64).cos()) as f32)
        .collect()
}

/// Number of STFT frames for a signal of `len` samples.
pub fn frames_for(len: usize) -> usize {
    len / HOP + 1
}

/// Reflect-pad index into a signal of length `len` (no edge repeat).
fn reflect(i: isize, len: usize) -> usize {
    let len = len as isize;
    let mut j = i;
    if j < 0 {
        j = -j;
    }
    if j >= len {
        j = 2 * (len - 1) - j;
    }
    j as usize
}

/// Short-time Fourier transform of one channel.
pub fn stft(x: &[f32]) -> Spectrum {
    assert!(x.len() > N_FFT / 2, "signal shorter than half a window");
    let window = hann(N_FFT);
    let mut planner = RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(N_FFT);
    let mut buf = fft.make_input_vec();
    let mut spec = fft.make_output_vec();
    let mut scratch = fft.make_scratch_vec();
    let frames = frames_for(x.len());
    let bins = N_FFT / 2 + 1;
    let mut data = Vec::with_capacity(frames * bins);
    let pad = (N_FFT / 2) as isize;
    for t in 0..frames {
        let start = (t * HOP) as isize - pad;
        for i in 0..N_FFT {
            buf[i] = x[reflect(start + i as isize, x.len())] * window[i];
        }
        fft.process_with_scratch(&mut buf, &mut spec, &mut scratch)
            .expect("buffer sizes come from the planner");
        data.extend_from_slice(&spec);
    }
    Spectrum { frames, bins, data }
}

/// Inverse STFT; returns `HOP * (frames - 1)` samples, like `torch.istft`
/// without a `length` argument.
pub fn istft(s: &Spectrum) -> Vec<f32> {
    assert_eq!(s.bins, N_FFT / 2 + 1);
    let window = hann(N_FFT);
    let mut planner = RealFftPlanner::<f32>::new();
    let ifft = planner.plan_fft_inverse(N_FFT);
    let mut spec = ifft.make_input_vec();
    let mut buf = ifft.make_output_vec();
    let mut scratch = ifft.make_scratch_vec();
    let full = N_FFT + HOP * (s.frames - 1);
    let mut out = vec![0.0f32; full];
    let mut env = vec![0.0f32; full];
    let scale = 1.0 / N_FFT as f32;
    for t in 0..s.frames {
        spec.copy_from_slice(&s.data[t * s.bins..(t + 1) * s.bins]);
        // irfft ignores the imaginary parts of the DC and Nyquist bins.
        spec[0].im = 0.0;
        spec[s.bins - 1].im = 0.0;
        ifft.process_with_scratch(&mut spec, &mut buf, &mut scratch)
            .expect("imaginary parts of DC and Nyquist were cleared");
        let off = t * HOP;
        for i in 0..N_FFT {
            out[off + i] += buf[i] * scale * window[i];
            env[off + i] += window[i] * window[i];
        }
    }
    let start = N_FFT / 2;
    let len = HOP * (s.frames - 1);
    (start..start + len)
        .map(|i| {
            assert!(env[i] > 1e-11, "window envelope vanishes at sample {i}");
            out[i] / env[i]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_recovers_the_signal() {
        let n = HOP * 40;
        let x: Vec<f32> = (0..n)
            .map(|i| ((i * 7919) % 1000) as f32 / 1000.0 - 0.5)
            .collect();
        let s = stft(&x);
        assert_eq!(s.frames, 41);
        let y = istft(&s);
        assert_eq!(y.len(), n);
        let err = x
            .iter()
            .zip(&y)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f32::max);
        assert!(err < 1e-5, "max error {err}");
    }

    #[test]
    fn reflect_matches_numpy_reflect_mode() {
        // numpy.pad([0,1,2,3], 2, 'reflect') == [2,1,0,1,2,3,2,1]
        let idx: Vec<usize> = (-2..6).map(|i| reflect(i, 4)).collect();
        assert_eq!(idx, vec![2, 1, 0, 1, 2, 3, 2, 1]);
    }
}
