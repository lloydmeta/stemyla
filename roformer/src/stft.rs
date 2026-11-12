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

/// Inverse STFT of `frames` frames; returns `HOP * (frames - 1)` samples,
/// like `torch.istft` without a `length` argument.  `frame(t, bins)`
/// writes frame `t`'s `N_FFT / 2 + 1` bins into `bins`.  It is asked for
/// each frame once, in order, so a caller can make each frame as it is
/// needed and hold no whole spectrum.
pub fn istft_frames(frames: usize, mut frame: impl FnMut(usize, &mut [Complex<f32>])) -> Vec<f32> {
    let bins = N_FFT / 2 + 1;
    let window = hann(N_FFT);
    let mut planner = RealFftPlanner::<f32>::new();
    let ifft = planner.plan_fft_inverse(N_FFT);
    let mut spec = ifft.make_input_vec();
    let mut buf = ifft.make_output_vec();
    let mut scratch = ifft.make_scratch_vec();
    let full = N_FFT + HOP * (frames - 1);
    let mut out = vec![0.0f32; full];
    let mut env = vec![0.0f32; full];
    let scale = 1.0 / N_FFT as f32;
    for t in 0..frames {
        frame(t, &mut spec);
        // irfft ignores the imaginary parts of the DC and Nyquist bins.
        spec[0].im = 0.0;
        spec[bins - 1].im = 0.0;
        ifft.process_with_scratch(&mut spec, &mut buf, &mut scratch)
            .expect("imaginary parts of DC and Nyquist were cleared");
        let off = t * HOP;
        for i in 0..N_FFT {
            out[off + i] += buf[i] * scale * window[i];
            env[off + i] += window[i] * window[i];
        }
    }
    let start = N_FFT / 2;
    let len = HOP * (frames - 1);
    (start..start + len)
        .map(|i| {
            assert!(env[i] > 1e-11, "window envelope vanishes at sample {i}");
            out[i] / env[i]
        })
        .collect()
}

/// Inverse STFT; returns `HOP * (frames - 1)` samples, like `torch.istft`
/// without a `length` argument.
pub fn istft(s: &Spectrum) -> Vec<f32> {
    assert_eq!(s.bins, N_FFT / 2 + 1);
    istft_frames(s.frames, |t, bins| {
        bins.copy_from_slice(&s.data[t * s.bins..(t + 1) * s.bins]);
    })
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

    /// The inverse STFT of a whole spectrum, as `istft` was before it
    /// took each frame from a function: the reference for
    /// `istft_frames`.
    fn istft_of_a_whole_spectrum(s: &Spectrum) -> Vec<f32> {
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
            spec[0].im = 0.0;
            spec[s.bins - 1].im = 0.0;
            ifft.process_with_scratch(&mut spec, &mut buf, &mut scratch)
                .unwrap();
            let off = t * HOP;
            for i in 0..N_FFT {
                out[off + i] += buf[i] * scale * window[i];
                env[off + i] += window[i] * window[i];
            }
        }
        let start = N_FFT / 2;
        (start..start + HOP * (s.frames - 1))
            .map(|i| out[i] / env[i])
            .collect()
    }

    /// Cut 2 of M1 of `plans/active/gpu-speed.md`: an inverse STFT that
    /// takes each frame from a function gives the samples of the whole
    /// spectrum's inverse, bit for bit.  The imaginary parts of the DC
    /// and Nyquist bins are not 0, so the test sees that they are cleared.
    #[test]
    fn frames_from_a_function_give_the_inverse_of_the_whole_spectrum() {
        let x: Vec<f32> = (0..HOP * 40)
            .map(|i| ((i * 7919) % 1000) as f32 / 1000.0 - 0.5)
            .collect();
        let mut s = stft(&x);
        for (k, v) in s.data.iter_mut().enumerate() {
            v.im += 0.001 * (k % 13) as f32;
        }
        let want = istft_of_a_whole_spectrum(&s);
        let mut asked = Vec::new();
        let got = istft_frames(s.frames, |t, bins| {
            asked.push(t);
            bins.copy_from_slice(&s.data[t * s.bins..(t + 1) * s.bins]);
        });
        assert_eq!(
            asked,
            (0..s.frames).collect::<Vec<_>>(),
            "each frame once, in order"
        );
        assert_eq!(got.len(), want.len());
        for (k, (g, w)) in got.iter().zip(&want).enumerate() {
            assert_eq!(g.to_bits(), w.to_bits(), "sample {k}");
        }
        let whole = istft(&s);
        assert!(
            whole
                .iter()
                .zip(&want)
                .all(|(a, b)| a.to_bits() == b.to_bits()),
            "istft gives the inverse of the whole spectrum"
        );
    }

    /// One frame gives `HOP x (1 - 1)` samples: none, and no panic.
    #[test]
    fn one_frame_gives_no_samples() {
        let mut asked = 0;
        let out = istft_frames(1, |t, bins| {
            asked += 1;
            bins.fill(Complex::new(t as f32 + 1.0, 0.0));
        });
        assert_eq!(asked, 1, "the frame is asked for once");
        assert!(out.is_empty(), "{} samples", out.len());
    }

    #[test]
    fn reflect_matches_numpy_reflect_mode() {
        // numpy.pad([0,1,2,3], 2, 'reflect') == [2,1,0,1,2,3,2,1]
        let idx: Vec<usize> = (-2..6).map(|i| reflect(i, 4)).collect();
        assert_eq!(idx, vec![2, 1, 0, 1, 2, 3, 2, 1]);
    }
}
