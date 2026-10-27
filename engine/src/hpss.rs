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
    let mut columns = SpectrogramColumns::new(x.len(), sample_rate, max_hz, rows, max_cols);
    for c in 0..columns.columns() {
        let window = columns.window(c);
        columns.put(c, &x[window]);
    }
    columns.finish()
}

/// The FFT size of [`spectrogram_db`].
const SPECTROGRAM_FFT: usize = 1024;

/// The columns of [`spectrogram_db`], made one at a time from the frames
/// of each column's window.  So a caller reads only the frames that the
/// image shows, and holds no song-length signal.
///
/// Column `c` is the STFT frame `c` of [`stft`]: the frames from
/// `c x hop - n_fft` up to `c x hop`, with zeros outside the signal.
pub struct SpectrogramColumns {
    frames: usize,
    hop: usize,
    rows: usize,
    columns: usize,
    top_bin: usize,
    fft: std::sync::Arc<dyn realfft::RealToComplex<f32>>,
    window: Vec<f32>,
    input: Vec<f32>,
    spectrum: Vec<Complex32>,
    scratch: Vec<Complex32>,
    image: Vec<f32>,
}

impl SpectrogramColumns {
    /// For a signal of `frames` frames, with the arguments of
    /// [`spectrogram_db`].
    pub fn new(frames: usize, sample_rate: u32, max_hz: f32, rows: usize, max_cols: usize) -> Self {
        let n_fft = SPECTROGRAM_FFT;
        let hop = (frames / max_cols.max(1)).max(256);
        let bins = n_fft / 2 + 1;
        let top_bin =
            ((max_hz / sample_rate as f32) * n_fft as f32).min((bins - 1) as f32) as usize;
        // The frames of `stft` over the signal padded by `n_fft` on both
        // sides.
        let columns = (frames + n_fft) / hop + 1;
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(n_fft);
        let (input, spectrum, scratch) = (
            fft.make_input_vec(),
            fft.make_output_vec(),
            fft.make_scratch_vec(),
        );
        Self {
            frames,
            hop,
            rows,
            columns,
            top_bin,
            fft,
            window: hann(n_fft),
            input,
            spectrum,
            scratch,
            image: vec![-120.0f32; rows * columns],
        }
    }

    /// The columns of the image.
    pub fn columns(&self) -> usize {
        self.columns
    }

    /// The frames between the starts of two columns.
    pub fn hop(&self) -> usize {
        self.hop
    }

    /// The frames of the signal that column `c` reads.  They can be fewer
    /// than the FFT's size at the ends of the signal, or none.
    pub fn window(&self, c: usize) -> std::ops::Range<usize> {
        let end = c * self.hop;
        end.saturating_sub(SPECTROGRAM_FFT).min(self.frames)..end.min(self.frames)
    }

    /// Makes column `c` from `samples`, the signal at the frames of
    /// `window(c)`.
    pub fn put(&mut self, c: usize, samples: &[f32]) {
        let window = self.window(c);
        assert_eq!(samples.len(), window.len(), "the samples of column {c}");
        let n_fft = SPECTROGRAM_FFT;
        // The index in the FFT's input of the window's first frame.
        let offset = window.start + n_fft - c * self.hop;
        for i in 0..n_fft {
            let v = if i >= offset && i - offset < samples.len() {
                samples[i - offset]
            } else {
                0.0
            };
            self.input[i] = v * self.window[i];
        }
        self.fft
            .process_with_scratch(&mut self.input, &mut self.spectrum, &mut self.scratch)
            .expect("buffer sizes match the plan");
        let (rows, cols, bins) = (self.rows, self.columns, n_fft / 2 + 1);
        for r in 0..rows {
            // Row 0 is the top of the image (highest frequency).
            let k0 = self.top_bin * (rows - 1 - r) / rows;
            let k1 = (self.top_bin * (rows - r) / rows).max(k0 + 1);
            let mut m = 0.0f32;
            for k in k0..k1.min(bins) {
                m = m.max(self.spectrum[k].norm());
            }
            self.image[r * cols + c] = 20.0 * (m / (n_fft as f32 / 4.0) + 1e-9).log10();
        }
    }

    /// The image, its columns and its rows, as [`spectrogram_db`] gives
    /// them.
    pub fn finish(self) -> (Vec<f32>, usize, usize) {
        (self.image, self.columns, self.rows)
    }
}

/// The percussive part of a stereo signal, made in blocks.  It equals
/// `hpss(..).percussive` of each channel, bit for bit.  It holds a fixed
/// number of STFT frames, not the signal: the frames of one time median
/// (`harmonic_len`), and one frame of overlap-add.
///
/// Frame `t` of the STFT reads the signal from `t x hop - n_fft` up to
/// `t x hop`.  Its time median needs the frames up to `t + harmonic_len / 2`.
/// After its overlap-add, the samples before `(t + 1) x hop - n_fft` are
/// complete, because no later frame covers them.
pub struct BlockHpss {
    cfg: HpssConfig,
    /// The frames of the signal.
    frames: usize,
    /// The frames of the STFT.
    stft_frames: usize,
    fft: std::sync::Arc<dyn realfft::RealToComplex<f32>>,
    ifft: std::sync::Arc<dyn realfft::ComplexToReal<f32>>,
    window: Vec<f32>,
    channels: [HpssChannel; 2],
    /// The frames of the signal put so far.
    received: usize,
    /// The next STFT frame to make.
    next_frame: usize,
    /// The next STFT frame to mask and add.
    next_out: usize,
    /// The padded index of the first sample in each channel's `out`, and
    /// of `norm`.
    out_start: usize,
    /// The summed squared window at each sample of `out`.
    norm: Vec<f32>,
    fft_in: Vec<f32>,
    fft_scratch: Vec<Complex32>,
    ifft_out: Vec<f32>,
    ifft_scratch: Vec<Complex32>,
    median: Vec<f32>,
    median_scratch: Vec<f32>,
}

/// The state of one channel of a [`BlockHpss`].
struct HpssChannel {
    /// The signal from `input_start`, up to what was put.
    input: Vec<f32>,
    input_start: usize,
    /// The spectrum and the magnitudes of STFT frame `t` at `t % harmonic_len`.
    spectra: Vec<Vec<Complex32>>,
    mags: Vec<Vec<f32>>,
    /// The overlap-add from `out_start` of the [`BlockHpss`].
    out: Vec<f32>,
    /// The complete samples that go to the sink next.
    done: Vec<f32>,
}

impl BlockHpss {
    /// For a signal of `frames` frames.
    pub fn new(frames: usize, cfg: &HpssConfig) -> Self {
        assert!(
            cfg.harmonic_len % 2 == 1 && cfg.percussive_len % 2 == 1,
            "median lengths must be odd"
        );
        let n_fft = cfg.n_fft;
        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(n_fft);
        let ifft = planner.plan_fft_inverse(n_fft);
        let bins = n_fft / 2 + 1;
        let channel = || HpssChannel {
            input: Vec::with_capacity(n_fft + cfg.hop),
            input_start: 0,
            spectra: vec![vec![Complex32::default(); bins]; cfg.harmonic_len],
            mags: vec![vec![0.0; bins]; cfg.harmonic_len],
            out: Vec::with_capacity(n_fft + cfg.hop),
            done: Vec::with_capacity(n_fft + cfg.hop),
        };
        Self {
            cfg: *cfg,
            frames,
            stft_frames: (frames + n_fft) / cfg.hop + 1,
            fft_in: fft.make_input_vec(),
            fft_scratch: fft.make_scratch_vec(),
            ifft_out: ifft.make_output_vec(),
            ifft_scratch: ifft.make_scratch_vec(),
            fft,
            ifft,
            window: hann(n_fft),
            channels: [channel(), channel()],
            received: 0,
            next_frame: 0,
            next_out: 0,
            out_start: 0,
            norm: Vec::with_capacity(n_fft + cfg.hop),
            median: Vec::with_capacity(cfg.harmonic_len.max(cfg.percussive_len)),
            median_scratch: Vec::with_capacity(cfg.harmonic_len.max(cfg.percussive_len)),
        }
    }

    /// Takes the next block of the signal, and gives `sink` the percussive
    /// frames that are complete.  `left` and `right` have the same length.
    pub fn put<E>(
        &mut self,
        left: &[f32],
        right: &[f32],
        sink: &mut impl FnMut(&[f32], &[f32]) -> Result<(), E>,
    ) -> Result<(), E> {
        assert_eq!(left.len(), right.len(), "the sides of a block differ");
        assert!(
            self.received + left.len() <= self.frames,
            "more frames than the signal has"
        );
        // A hop at a time, so the input holds at most `n_fft + hop` frames.
        let mut at = 0;
        while at < left.len() {
            let n = self.cfg.hop.min(left.len() - at);
            self.channels[0].input.extend_from_slice(&left[at..at + n]);
            self.channels[1].input.extend_from_slice(&right[at..at + n]);
            self.received += n;
            at += n;
            self.advance(sink)?;
        }
        Ok(())
    }

    /// Gives `sink` the last percussive frames.  Panics if the blocks did
    /// not hold all the frames of the signal.
    pub fn finish<E>(
        mut self,
        sink: &mut impl FnMut(&[f32], &[f32]) -> Result<(), E>,
    ) -> Result<(), E> {
        assert_eq!(self.received, self.frames, "the blocks missed frames");
        self.advance(sink)?;
        assert_eq!(self.next_out, self.stft_frames, "every frame is added");
        Ok(())
    }

    /// Makes every STFT frame whose input is here, and adds every frame
    /// whose time median it can make.
    fn advance<E>(
        &mut self,
        sink: &mut impl FnMut(&[f32], &[f32]) -> Result<(), E>,
    ) -> Result<(), E> {
        let (n_fft, hop) = (self.cfg.n_fft, self.cfg.hop);
        while self.next_frame < self.stft_frames
            && (self.next_frame * hop).min(self.frames) <= self.received
        {
            self.make_frame(self.next_frame);
            self.next_frame += 1;
            while self.next_out < self.stft_frames
                && self.next_frame
                    > (self.next_out + self.cfg.harmonic_len / 2).min(self.stft_frames - 1)
            {
                self.add_frame(self.next_out);
                self.next_out += 1;
                self.emit(sink)?;
            }
            // The next frame reads the signal from here.
            let keep_from = (self.next_frame * hop)
                .saturating_sub(n_fft)
                .min(self.received);
            for ch in &mut self.channels {
                let drop = keep_from - ch.input_start;
                ch.input.drain(..drop);
                ch.input_start = keep_from;
            }
        }
        Ok(())
    }

    /// The STFT frame `t` of each channel, and its magnitudes, as `stft`
    /// and `hpss` make them.
    fn make_frame(&mut self, t: usize) {
        let (n_fft, hop, slot) = (self.cfg.n_fft, self.cfg.hop, t % self.cfg.harmonic_len);
        for ch in &mut self.channels {
            for i in 0..n_fft {
                let j = t * hop + i;
                let v = if j >= n_fft && j - n_fft < self.frames {
                    ch.input[j - n_fft - ch.input_start]
                } else {
                    0.0
                };
                self.fft_in[i] = v * self.window[i];
            }
            let spectrum = &mut ch.spectra[slot];
            self.fft
                .process_with_scratch(&mut self.fft_in, spectrum, &mut self.fft_scratch)
                .expect("buffer sizes match the plan");
            for (m, c) in ch.mags[slot].iter_mut().zip(spectrum.iter()) {
                *m = c.norm();
            }
        }
    }

    /// Masks the STFT frame `u` of each channel with its percussive mask,
    /// and adds its inverse to the overlap-add, as `hpss` and `istft` do.
    fn add_frame(&mut self, u: usize) {
        let cfg = self.cfg;
        let (n, last) = (cfg.n_fft, self.stft_frames - 1);
        let (rh, rp) = (cfg.harmonic_len / 2, cfg.percussive_len / 2);
        let start = u * cfg.hop;
        let end = start + n;
        let grow = end - self.out_start;
        if self.norm.len() < grow {
            self.norm.resize(grow, 0.0);
            for ch in &mut self.channels {
                ch.out.resize(grow, 0.0);
            }
        }
        for (side, ch) in self.channels.iter_mut().enumerate() {
            let slot = u % cfg.harmonic_len;
            let k_len = ch.mags[slot].len();
            for k in 0..k_len {
                self.median.clear();
                for d in 0..cfg.harmonic_len {
                    let tt = (u + d).saturating_sub(rh).min(last);
                    self.median.push(ch.mags[tt % cfg.harmonic_len][k]);
                }
                let hv = fast_median(&self.median, &mut self.median_scratch);
                self.median.clear();
                for d in 0..cfg.percussive_len {
                    let kk = (k + d).saturating_sub(rp).min(k_len - 1);
                    self.median.push(ch.mags[slot][kk]);
                }
                let pv = fast_median(&self.median, &mut self.median_scratch);
                ch.spectra[slot][k] *= percussive_mask(hv, pv, &cfg);
            }
            let spectrum = &mut ch.spectra[slot];
            // A real signal's spectrum has real DC and Nyquist bins.
            spectrum[0].im = 0.0;
            spectrum[k_len - 1].im = 0.0;
            self.ifft
                .process_with_scratch(spectrum, &mut self.ifft_out, &mut self.ifft_scratch)
                .expect("buffer sizes match the plan");
            let at = start - self.out_start;
            for i in 0..n {
                ch.out[at + i] += self.ifft_out[i] / n as f32 * self.window[i];
                if side == 0 {
                    self.norm[at + i] += self.window[i] * self.window[i];
                }
            }
        }
    }

    /// Gives `sink` the samples that no later frame adds to, and drops
    /// them from the overlap-add.
    fn emit<E>(&mut self, sink: &mut impl FnMut(&[f32], &[f32]) -> Result<(), E>) -> Result<(), E> {
        let n_fft = self.cfg.n_fft;
        // The padded index up to which the samples are complete.
        let complete = if self.next_out == self.stft_frames {
            n_fft + self.frames
        } else {
            self.next_out * self.cfg.hop
        };
        if complete <= self.out_start {
            return Ok(());
        }
        let count = complete - self.out_start;
        for ch in &mut self.channels {
            ch.done.clear();
            for j in self.out_start..complete {
                if j >= n_fft && j - n_fft < self.frames {
                    let at = j - self.out_start;
                    let norm = self.norm[at];
                    ch.done
                        .push(if norm > 1e-8 { ch.out[at] / norm } else { 0.0 });
                }
            }
            ch.out.drain(..count.min(ch.out.len()));
        }
        self.norm.drain(..count.min(self.norm.len()));
        self.out_start = complete;
        let [left, right] = &self.channels;
        if left.done.is_empty() {
            Ok(())
        } else {
            sink(&left.done, &right.done)
        }
    }
}

/// The percussive mask of one bin, from its harmonic and percussive
/// medians, as `hpss` makes it.
fn percussive_mask(hv: f32, pv: f32, cfg: &HpssConfig) -> f32 {
    if cfg.margin <= 1.0 {
        let a = hv.powf(cfg.power);
        let b = pv.powf(cfg.power);
        if a + b > 1e-20 { b / (a + b) } else { 0.5 }
    } else if pv > cfg.margin * hv {
        1.0
    } else {
        0.0
    }
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

    /// `spectrogram_db` as it was before the column builder: from the
    /// whole STFT of the signal.
    fn spectrogram_db_of_the_stft(
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
        let top_bin =
            ((max_hz / sample_rate as f32) * n_fft as f32).min((bins - 1) as f32) as usize;
        let cols = s.frames.len();
        let mut img = vec![-120.0f32; rows * cols];
        for (c, f) in s.frames.iter().enumerate() {
            for r in 0..rows {
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

    fn bits_of(image: &(Vec<f32>, usize, usize)) -> (Vec<u32>, usize, usize) {
        (
            image.0.iter().map(|v| v.to_bits()).collect(),
            image.1,
            image.2,
        )
    }

    #[test]
    fn the_column_builder_gives_spectrogram_db() {
        for frames in [1_000, 44_100, 300_000, 2_000_000] {
            let x = noise(frames, 11);
            let want = bits_of(&spectrogram_db_of_the_stft(&x, 44_100, 8_000.0, 128, 1_024));
            assert!(
                bits_of(&spectrogram_db(&x, 44_100, 8_000.0, 128, 1_024)) == want,
                "spectrogram_db, {frames} frames"
            );
            let mut builder = SpectrogramColumns::new(frames, 44_100, 8_000.0, 128, 1_024);
            assert_eq!(builder.columns(), want.1, "{frames} frames");
            // The columns in reverse, so the order does not matter.
            for c in (0..builder.columns()).rev() {
                let window = builder.window(c);
                assert!(window.len() <= 1_024, "{frames} frames, column {c}");
                builder.put(c, &x[window]);
            }
            assert!(
                bits_of(&builder.finish()) == want,
                "the windows, {frames} frames"
            );
        }
    }

    /// A tone, with a click every 11,025 frames.
    fn tone_with_clicks(n: usize, hz: f32) -> Vec<f32> {
        (0..n)
            .map(|i| {
                let click = if i % 11_025 == 5_000 { 0.9 } else { 0.0 };
                0.3 * (std::f32::consts::TAU * hz * i as f32 / 44_100.0).sin() + click
            })
            .collect()
    }

    /// The percussive part of `left` and `right` from a `BlockHpss` that
    /// takes blocks of `block` frames.
    fn block_percussive(left: &[f32], right: &[f32], block: usize) -> (Vec<f32>, Vec<f32>) {
        let cfg = HpssConfig::default();
        let mut hpss = BlockHpss::new(left.len(), &cfg);
        let (mut l, mut r) = (Vec::new(), Vec::new());
        let mut sink = |a: &[f32], b: &[f32]| -> Result<(), ()> {
            assert_eq!(a.len(), b.len());
            l.extend_from_slice(a);
            r.extend_from_slice(b);
            Ok(())
        };
        for (a, b) in left.chunks(block).zip(right.chunks(block)) {
            hpss.put(a, b, &mut sink).unwrap();
        }
        hpss.finish(&mut sink).unwrap();
        (l, r)
    }

    /// The tolerance of D8 of the M7 plan: 0, so the block output equals
    /// `hpss` bit for bit.  The block HPSS adds each frame's overlap to a
    /// sample in the order of the frames, as `istft` does, and makes each
    /// FFT, median and mask with the same operations.
    ///
    /// The `engine` crate builds without optimisation in tests, so each
    /// case runs on its own thread.  100,000 frames is more than three
    /// blocks of 32,768 frames, so no longer case is necessary (finding of
    /// the mutants baseline in review round 1 of M7).
    #[test]
    fn block_hpss_gives_hpss() {
        let case = |frames: usize, tone: bool| {
            let cfg = HpssConfig::default();
            let (left, right) = if tone {
                (
                    tone_with_clicks(frames, 440.0),
                    tone_with_clicks(frames, 330.0),
                )
            } else {
                (noise(frames, 5), noise(frames, 6))
            };
            let bits = |x: &[f32]| x.iter().map(|v| v.to_bits()).collect::<Vec<u32>>();
            let want_l = bits(&hpss(&left, &cfg).percussive);
            let want_r = bits(&hpss(&right, &cfg).percussive);
            let blocks: &[usize] = match frames {
                0..=2_048 => &[1, 511, 4_096, 32_768],
                2_049..=100_000 => &[511, 32_768],
                _ => &[32_768],
            };
            for &block in blocks {
                let (l, r) = block_percussive(&left, &right, block);
                assert!(
                    bits(&l) == want_l,
                    "left, {frames} frames in blocks of {block}, tone {tone}"
                );
                assert!(
                    bits(&r) == want_r,
                    "right, {frames} frames in blocks of {block}, tone {tone}"
                );
            }
        };
        std::thread::scope(|scope| {
            for frames in [1, 2_047, 2_048, 100_000] {
                for tone in [false, true] {
                    scope.spawn(move || case(frames, tone));
                }
            }
        });
    }

    /// Each measure runs on its own thread, whose heap count starts at 0,
    /// as the `engine` crate builds without optimisation in tests.
    #[test]
    fn a_block_hpss_keeps_its_memory_flat() {
        const MIB: usize = 1 << 20;
        // 3 s is four blocks of 32,768 frames, and 12 s is sixteen.
        let (short, long) = (3 * 44_100, 12 * 44_100);
        let blocks = |frames: usize| {
            let (left, right) = (noise(frames, 3), noise(frames, 4));
            crate::test_alloc::peak_of(|| {
                let mut hpss = BlockHpss::new(frames, &HpssConfig::default());
                let mut count = 0;
                let mut sink = |a: &[f32], _: &[f32]| -> Result<(), ()> {
                    count += a.len();
                    Ok(())
                };
                for (a, b) in left.chunks(32_768).zip(right.chunks(32_768)) {
                    hpss.put(a, b, &mut sink).unwrap();
                }
                hpss.finish(&mut sink).unwrap();
                count
            })
        };
        // The control: `hpss` of the long signal holds it.
        let whole = |frames: usize| {
            let x = noise(frames, 3);
            crate::test_alloc::peak_of(|| hpss(&x, &HpssConfig::default()).percussive.len()).0
        };
        let (short_peak, short_frames, long_peak, long_frames, whole_short, whole_long) =
            std::thread::scope(|scope| {
                let short_run = scope.spawn(|| blocks(short));
                let long_run = scope.spawn(|| blocks(long));
                let whole_short = scope.spawn(|| whole(short));
                let whole_long = scope.spawn(|| whole(long));
                let (short_peak, short_frames) = short_run.join().unwrap();
                let (long_peak, long_frames) = long_run.join().unwrap();
                (
                    short_peak,
                    short_frames,
                    long_peak,
                    long_frames,
                    whole_short.join().unwrap(),
                    whole_long.join().unwrap(),
                )
            });
        println!(
            "block HPSS peaks {short_peak} and {long_peak} bytes; hpss peaks {whole_short} and {whole_long} bytes"
        );
        assert_eq!((short_frames, long_frames), (short, long));
        assert!(
            long_peak.abs_diff(short_peak) < MIB,
            "10 s peaks {short_peak} bytes, 40 s peaks {long_peak} bytes"
        );
        assert!(
            whole_long > whole_short + MIB,
            "the control peaks {whole_short} and {whole_long} bytes"
        );
    }
}
