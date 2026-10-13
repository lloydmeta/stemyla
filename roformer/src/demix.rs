//! Chunked overlap-add over a whole song, ported from
//! `bs_roformer.utils.demix_track` (bs-roformer-infer 0.1.5).
//!
//! The song is reflect-padded by `border = chunk - step` on both sides,
//! cut into chunks every `step = chunk / overlap` samples, and each chunk's
//! output is weighted by a window with linear fades of `chunk / 10` samples
//! (no fade-in on the first chunk, no fade-out on the last), summed, and
//! divided by the summed window.
//!
//! [`Demixer`] is a state machine rather than a loop around a closure, so
//! the caller can run the network asynchronously (a browser cannot block
//! on GPU readback).

use crate::CHANNELS;

pub type Stereo = [Vec<f32>; CHANNELS];

pub struct Demixer {
    chunk: usize,
    step: usize,
    fade: usize,
    border: usize,
    padded: bool,
    mix: Stereo,
    pos: usize,
    window: Vec<f32>,
    result: Vec<Stereo>,
    counter: Vec<f32>,
}

/// `torch.linspace(start, end, n)` in f32.
fn linspace(start: f32, end: f32, n: usize) -> Vec<f32> {
    if n == 1 {
        return vec![start];
    }
    let step = (end - start) / (n - 1) as f32;
    // torch fills the first half from the start and the second from the end
    (0..n)
        .map(|k| if k < n / 2 { start + step * k as f32 } else { end - step * (n - 1 - k) as f32 })
        .collect()
}

/// Reflect padding on the right, numpy/torch `reflect` mode.
fn reflect_right(x: &[f32], to: usize) -> Vec<f32> {
    let n = x.len();
    assert!(to - n < n, "reflect padding must be shorter than the signal");
    (0..to).map(|j| if j < n { x[j] } else { x[2 * (n - 1) - j] }).collect()
}

/// Reflect padding by `p` on both sides.
fn reflect_both(x: &[f32], p: usize) -> Vec<f32> {
    let n = x.len() as isize;
    assert!((p as isize) < n, "reflect padding must be shorter than the signal");
    (-(p as isize)..n + p as isize)
        .map(|i| {
            let j = if i < 0 { -i } else if i >= n { 2 * (n - 1) - i } else { i };
            x[j as usize]
        })
        .collect()
}

impl Demixer {
    /// `mix`: both channels, same length.  `stems`: outputs per chunk.
    pub fn new(mix: Stereo, chunk: usize, overlap: usize, stems: usize) -> Self {
        assert_eq!(mix[0].len(), mix[1].len());
        let step = chunk / overlap;
        let fade = chunk / 10;
        let border = chunk - step;
        let padded = mix[0].len() > 2 * border && border > 0;
        let mix = if padded { [reflect_both(&mix[0], border), reflect_both(&mix[1], border)] } else { mix };
        let mut window = vec![1.0f32; chunk];
        let fade_in = linspace(0.0, 1.0, fade);
        let fade_out = linspace(1.0, 0.0, fade);
        for k in 0..fade {
            window[chunk - fade + k] *= fade_out[k];
        }
        for k in 0..fade {
            window[k] *= fade_in[k];
        }
        let total = mix[0].len();
        Self {
            chunk,
            step,
            fade,
            border,
            padded,
            pos: 0,
            window,
            result: (0..stems).map(|_| [vec![0.0; total], vec![0.0; total]]).collect(),
            counter: vec![0.0; total],
            mix,
        }
    }

    fn total(&self) -> usize {
        self.mix[0].len()
    }

    pub fn num_chunks(&self) -> usize {
        self.total().div_ceil(self.step)
    }

    /// Index of the next chunk, counting from 0.
    pub fn chunk_index(&self) -> usize {
        self.pos / self.step
    }

    /// The next chunk, padded to `chunk` samples, or `None` when done.
    pub fn next_input(&self) -> Option<Stereo> {
        let total = self.total();
        if self.pos >= total {
            return None;
        }
        let end = (self.pos + self.chunk).min(total);
        let part = |c: usize| {
            let x = &self.mix[c][self.pos..end];
            if x.len() == self.chunk {
                x.to_vec()
            } else if x.len() > self.chunk / 2 + 1 {
                reflect_right(x, self.chunk)
            } else {
                let mut v = x.to_vec();
                v.resize(self.chunk, 0.0);
                v
            }
        };
        Some([part(0), part(1)])
    }

    /// Adds the network output for the chunk last returned by
    /// [`Self::next_input`]; each stem must have at least as many samples
    /// as the unpadded part of the chunk.
    pub fn accept(&mut self, out: &[Stereo]) {
        assert_eq!(out.len(), self.result.len());
        let total = self.total();
        let i = self.pos;
        let length = (total - i).min(self.chunk);
        let mut window = self.window.clone();
        if i == 0 {
            window[..self.fade].fill(1.0);
        } else if i + self.chunk >= total {
            window[self.chunk - self.fade..].fill(1.0);
        }
        for (acc, x) in self.result.iter_mut().zip(out) {
            for c in 0..CHANNELS {
                assert!(x[c].len() >= length, "stem shorter than the chunk");
                for k in 0..length {
                    acc[c][i + k] += x[c][k] * window[k];
                }
            }
        }
        for k in 0..length {
            self.counter[i + k] += window[k];
        }
        self.pos += self.step;
    }

    /// Divides by the summed window and strips the padding.
    pub fn finish(self) -> Vec<Stereo> {
        let total = self.total();
        let (from, to) = if self.padded { (self.border, total - self.border) } else { (0, total) };
        let counter = self.counter;
        self.result
            .into_iter()
            .map(|stem| {
                stem.map(|ch| {
                    (from..to).map(|k| if counter[k] == 0.0 { 0.0 } else { ch[k] / counter[k] }).collect()
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linspace_endpoints_are_exact() {
        let v = linspace(1.0, 0.0, 5);
        assert_eq!(v, vec![1.0, 0.75, 0.5, 0.25, 0.0]);
    }

    #[test]
    fn identity_model_returns_the_mix() {
        let n: usize = 5000;
        let mix: Stereo = [(0..n).map(|i| (i as f32 * 0.01).sin()).collect(), (0..n).map(|i| i as f32 / n as f32).collect()];
        let mut d = Demixer::new(mix.clone(), 1000, 2, 1);
        let mut chunks = 0;
        while let Some(x) = d.next_input() {
            d.accept(&[x]);
            chunks += 1;
        }
        assert_eq!(chunks, (n + 1000).div_ceil(500));
        let out = d.finish();
        for c in 0..2 {
            assert_eq!(out[0][c].len(), n);
            for k in 0..n {
                assert!((out[0][c][k] - mix[c][k]).abs() < 1e-5, "channel {c} sample {k}");
            }
        }
    }
}
