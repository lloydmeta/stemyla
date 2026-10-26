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
//!
//! The demixer does not keep the song.  It reads the frames of each chunk
//! from a [`MixSource`], one span at a time, so its memory does not grow
//! with the song.  A span is at most one chunk of frames, and the
//! demixer reads the reflect padding from it by index.  A whole [`Stereo`]
//! in memory is a source that cannot fail.  The worker reads a mix file
//! in browser storage through a source of its own.

use std::convert::Infallible;
use std::ops::Range;

use crate::CHANNELS;

pub type Stereo = [Vec<f32>; CHANNELS];

/// Finished samples of every stem, `start` samples into the song.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub start: usize,
    pub stems: Vec<Stereo>,
}

/// The song that a [`Demixer`] separates, read in spans of frames.
pub trait MixSource {
    type Error;

    /// The frames in the song.
    fn frames(&self) -> usize;

    /// Both channels of the frames `start..end`, with `start < end` and
    /// `end <= self.frames()`.
    fn read(&mut self, start: usize, end: usize) -> Result<Stereo, Self::Error>;
}

/// The whole song in memory: both channels, of the same length.
impl MixSource for Stereo {
    type Error = Infallible;

    fn frames(&self) -> usize {
        assert_eq!(self[0].len(), self[1].len());
        self[0].len()
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "MEM-1: a span is at most one chunk, not the song"
    )]
    fn read(&mut self, start: usize, end: usize) -> Result<Stereo, Infallible> {
        Ok([0, 1].map(|c| self[c][start..end].to_vec()))
    }
}

pub struct Demixer<S: MixSource> {
    chunk: usize,
    step: usize,
    fade: usize,
    border: usize,
    padded: bool,
    /// The song, read one span for each chunk.
    source: S,
    /// The frames of the song: `source.frames()` at the start.
    frames: usize,
    pos: usize,
    window: Vec<f32>,
    /// Running sums from `base` (in padded samples) on: at most one chunk
    /// is kept, since a sample before `pos` gets no further contributions.
    base: usize,
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
        .map(|k| {
            if k < n / 2 {
                start + step * k as f32
            } else {
                end - step * (n - 1 - k) as f32
            }
        })
        .collect()
}

/// Reflect padding on the right, numpy/torch `reflect` mode.
fn reflect_right(x: &[f32], to: usize) -> Vec<f32> {
    let n = x.len();
    assert!(
        to - n < n,
        "reflect padding must be shorter than the signal"
    );
    (0..to)
        .map(|j| if j < n { x[j] } else { x[2 * (n - 1) - j] })
        .collect()
}

impl<S: MixSource> Demixer<S> {
    /// `source`: the song.  `stems`: outputs per chunk.
    pub fn new(source: S, chunk: usize, overlap: usize, stems: usize) -> Self {
        let frames = source.frames();
        let step = chunk / overlap;
        let fade = chunk / 10;
        let border = chunk - step;
        let padded = frames > 2 * border && border > 0;
        let mut window = vec![1.0f32; chunk];
        let fade_in = linspace(0.0, 1.0, fade);
        let fade_out = linspace(1.0, 0.0, fade);
        for k in 0..fade {
            window[chunk - fade + k] *= fade_out[k];
        }
        for k in 0..fade {
            window[k] *= fade_in[k];
        }
        Self {
            chunk,
            step,
            fade,
            border,
            padded,
            source,
            frames,
            pos: 0,
            window,
            base: 0,
            result: (0..stems).map(|_| [Vec::new(), Vec::new()]).collect(),
            counter: Vec::new(),
        }
    }

    /// The source of the song.
    pub fn source(&self) -> &S {
        &self.source
    }

    /// Length of the (padded) signal the chunks are cut from.
    fn total(&self) -> usize {
        if self.padded {
            self.frames + 2 * self.border
        } else {
            self.frames
        }
    }

    /// The frame of the song at index `i` of the padded signal: reflect
    /// padding by `border` on both sides, numpy/torch `reflect` mode.
    fn frame(&self, i: usize) -> usize {
        if !self.padded {
            return i;
        }
        let n = self.frames as isize;
        let i = i as isize - self.border as isize;
        let j = if i < 0 {
            -i
        } else if i >= n {
            2 * (n - 1) - i
        } else {
            i
        };
        j as usize
    }

    /// The frames of the song that the padded samples `padded` read.
    fn span(&self, padded: Range<usize>) -> Range<usize> {
        let first = self.frame(padded.start);
        let last = self.frame(padded.end - 1);
        let mut lo = first.min(last);
        let mut hi = first.max(last);
        if self.padded {
            // The reflection turns at the song's first and last frames.
            if padded.start < self.border && padded.end > self.border {
                lo = 0;
            }
            let end = self.border + self.frames;
            if padded.start < end && padded.end > end {
                hi = self.frames - 1;
            }
        }
        lo..hi + 1
    }

    /// Samples in each output stem: the song's length.
    pub fn output_len(&self) -> usize {
        self.frames
    }

    pub fn num_chunks(&self) -> usize {
        self.total().div_ceil(self.step)
    }

    /// Index of the next chunk, counting from 0.
    pub fn chunk_index(&self) -> usize {
        self.pos / self.step
    }

    /// The next chunk, padded to `chunk` samples, or `None` when done.  It
    /// reads one span of the song from the source, and gives the source's
    /// error if that read fails.
    pub fn next_input(&mut self) -> Result<Option<Stereo>, S::Error> {
        let total = self.total();
        if self.pos >= total {
            return Ok(None);
        }
        let end = (self.pos + self.chunk).min(total);
        let span = self.span(self.pos..end);
        let frames = self.source.read(span.start, span.end)?;
        let part = |c: usize| {
            let x: Vec<f32> = (self.pos..end)
                .map(|i| frames[c][self.frame(i) - span.start])
                .collect();
            if x.len() == self.chunk {
                x
            } else if x.len() > self.chunk / 2 + 1 {
                reflect_right(&x, self.chunk)
            } else {
                let mut v = x;
                v.resize(self.chunk, 0.0);
                v
            }
        };
        Ok(Some([part(0), part(1)]))
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
        let off = i - self.base;
        let need = off + length;
        if self.counter.len() < need {
            self.counter.resize(need, 0.0);
            for acc in &mut self.result {
                for ch in acc.iter_mut() {
                    ch.resize(need, 0.0);
                }
            }
        }
        for (acc, x) in self.result.iter_mut().zip(out) {
            for c in 0..CHANNELS {
                assert!(x[c].len() >= length, "stem shorter than the chunk");
                for k in 0..length {
                    acc[c][off + k] += x[c][k] * window[k];
                }
            }
        }
        for k in 0..length {
            self.counter[off + k] += window[k];
        }
        self.pos += self.step;
    }

    /// The samples that no later chunk can change, divided by the summed
    /// window and without the padding, or `None` if there are none yet.
    /// After the last chunk this returns the rest of the song.  Taking
    /// them frees their running sums, so a long song never needs more
    /// than a chunk of them.
    pub fn take_ready(&mut self) -> Option<Segment> {
        let ready = self
            .pos
            .min(self.total())
            .min(self.base + self.counter.len());
        if ready <= self.base {
            return None;
        }
        let n = ready - self.base;
        let (from, to) = if self.padded {
            (self.border, self.total() - self.border)
        } else {
            (0, self.total())
        };
        // The part of [base, ready) inside [from, to), relative to base.
        let lo = from.clamp(self.base, ready) - self.base;
        let hi = to.clamp(self.base, ready) - self.base;
        let counter = &self.counter;
        let stems: Vec<Stereo> = self
            .result
            .iter()
            .map(|stem| {
                [0, 1].map(|c| {
                    (lo..hi)
                        .map(|k| {
                            if counter[k] == 0.0 {
                                0.0
                            } else {
                                stem[c][k] / counter[k]
                            }
                        })
                        .collect()
                })
            })
            .collect();
        let start = (self.base + lo).saturating_sub(from);
        self.counter.drain(..n);
        for acc in &mut self.result {
            for ch in acc.iter_mut() {
                ch.drain(..n);
            }
        }
        self.base = ready;
        (hi > lo).then_some(Segment { start, stems })
    }

    /// Divides by the summed window and strips the padding: every stem
    /// from where [`Self::take_ready`] left off to the end of the song.
    pub fn finish(mut self) -> Vec<Stereo> {
        let mut out: Vec<Stereo> = (0..self.result.len())
            .map(|_| [Vec::new(), Vec::new()])
            .collect();
        self.pos = self.pos.max(self.total());
        while let Some(seg) = self.take_ready() {
            for (o, s) in out.iter_mut().zip(seg.stems) {
                for c in 0..CHANNELS {
                    o[c].extend_from_slice(&s[c]);
                }
            }
        }
        out
    }
}

#[cfg(test)]
#[expect(
    clippy::disallowed_methods,
    reason = "MEM-1: test code, which does not ship"
)]
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
        let mix: Stereo = [
            (0..n).map(|i| (i as f32 * 0.01).sin()).collect(),
            (0..n).map(|i| i as f32 / n as f32).collect(),
        ];
        let mut d = Demixer::new(mix.clone(), 1000, 2, 1);
        let mut chunks = 0;
        while let Some(x) = d.next_input().unwrap() {
            d.accept(&[x]);
            chunks += 1;
        }
        assert_eq!(chunks, (n + 1000).div_ceil(500));
        let out = d.finish();
        for c in 0..2 {
            assert_eq!(out[0][c].len(), n);
            for k in 0..n {
                assert!(
                    (out[0][c][k] - mix[c][k]).abs() < 1e-5,
                    "channel {c} sample {k}"
                );
            }
        }
    }

    /// The previous implementation: song-length running sums over a padded
    /// copy of the mix, divided at the end.  The streaming one must match it
    /// exactly.
    fn reference(
        mix: &Stereo,
        chunk: usize,
        overlap: usize,
        model: impl Fn(&Stereo) -> Vec<Stereo>,
    ) -> Vec<Stereo> {
        let step = chunk / overlap;
        let fade = chunk / 10;
        let border = chunk - step;
        let padded = mix[0].len() > 2 * border && border > 0;
        let reflect_both = |x: &[f32], p: usize| -> Vec<f32> {
            let n = x.len() as isize;
            (-(p as isize)..n + p as isize)
                .map(|i| {
                    let j = if i < 0 {
                        -i
                    } else if i >= n {
                        2 * (n - 1) - i
                    } else {
                        i
                    };
                    x[j as usize]
                })
                .collect()
        };
        let mix = if padded {
            [reflect_both(&mix[0], border), reflect_both(&mix[1], border)]
        } else {
            mix.clone()
        };
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
        let mut result: Vec<Stereo> = Vec::new();
        let mut counter = vec![0.0f32; total];
        let mut pos = 0;
        while pos < total {
            let end = (pos + chunk).min(total);
            let part = |c: usize| {
                let x = &mix[c][pos..end];
                if x.len() == chunk {
                    x.to_vec()
                } else if x.len() > chunk / 2 + 1 {
                    reflect_right(x, chunk)
                } else {
                    let mut v = x.to_vec();
                    v.resize(chunk, 0.0);
                    v
                }
            };
            let out = model(&[part(0), part(1)]);
            if result.is_empty() {
                result = (0..out.len())
                    .map(|_| [vec![0.0; total], vec![0.0; total]])
                    .collect();
            }
            let length = (total - pos).min(chunk);
            let mut w = window.clone();
            if pos == 0 {
                w[..fade].fill(1.0);
            } else if pos + chunk >= total {
                w[chunk - fade..].fill(1.0);
            }
            for (acc, x) in result.iter_mut().zip(&out) {
                for c in 0..CHANNELS {
                    for k in 0..length {
                        acc[c][pos + k] += x[c][k] * w[k];
                    }
                }
            }
            for k in 0..length {
                counter[pos + k] += w[k];
            }
            pos += step;
        }
        let (from, to) = if padded {
            (border, total - border)
        } else {
            (0, total)
        };
        result
            .into_iter()
            .map(|stem| {
                stem.map(|ch| {
                    (from..to)
                        .map(|k| {
                            if counter[k] == 0.0 {
                                0.0
                            } else {
                                ch[k] / counter[k]
                            }
                        })
                        .collect()
                })
            })
            .collect()
    }

    /// A stand-in network: three stems that differ, nonlinear so a wrong
    /// alignment of chunks shows.
    fn fake(x: &Stereo) -> Vec<Stereo> {
        (0..3)
            .map(|s| {
                [0, 1].map(|c| {
                    x[c].iter()
                        .enumerate()
                        .map(|(k, v)| {
                            (v * (s + 1) as f32).sin() + 0.001 * (k % 7) as f32 * c as f32
                        })
                        .collect()
                })
            })
            .collect()
    }

    fn song(n: usize) -> Stereo {
        [
            (0..n).map(|i| (i as f32 * 0.013).sin() * 0.7).collect(),
            (0..n)
                .map(|i| ((i * 7919) % 1000) as f32 / 1000.0 - 0.5)
                .collect(),
        ]
    }

    /// Short (unpadded), just over two chunks, and long songs; chunk sizes
    /// with and without a remainder; overlaps 1 (no padding), 2 and 4.  In
    /// (15, 10, 2) and (5000, 1000, 2), a chunk starts at `border + n`, one
    /// frame after the right reflection turns at `border + n - 1`.
    const STREAM_CASES: &[(usize, usize, usize)] = &[
        (15, 10, 2),
        (700, 1000, 2),
        (1000, 1000, 2),
        (2001, 1000, 2),
        (5000, 1000, 2),
        (5003, 1000, 2),
        (12345, 1000, 4),
        (9999, 640, 2),
        (5003, 1000, 1),
    ];

    #[test]
    fn streamed_segments_match_the_whole_song_result_exactly() {
        for &(n, chunk, overlap) in STREAM_CASES {
            let mix = song(n);
            let want = reference(&mix, chunk, overlap, fake);
            let mut d = Demixer::new(mix.clone(), chunk, overlap, 3);
            assert_eq!(d.output_len(), n);
            let mut got: Vec<Stereo> = (0..3).map(|_| [Vec::new(), Vec::new()]).collect();
            let mut held = 0;
            while let Some(x) = d.next_input().unwrap() {
                d.accept(&fake(&x));
                held = held.max(d.counter.len());
                if let Some(seg) = d.take_ready() {
                    assert_eq!(seg.start, got[0][0].len(), "n={n}: segments are contiguous");
                    for (g, s) in got.iter_mut().zip(seg.stems) {
                        for c in 0..2 {
                            g[c].extend_from_slice(&s[c]);
                        }
                    }
                }
            }
            for (g, s) in got.iter_mut().zip(d.finish()) {
                for c in 0..2 {
                    g[c].extend_from_slice(&s[c]);
                }
            }
            assert!(
                held <= chunk,
                "n={n}: kept {held} running sums, over a chunk"
            );
            for (s, (g, w)) in got.iter().zip(&want).enumerate() {
                for c in 0..2 {
                    assert_eq!(g[c].len(), n, "n={n} stem {s} channel {c}");
                    for k in 0..n {
                        assert_eq!(
                            g[c][k].to_bits(),
                            w[c][k].to_bits(),
                            "n={n} chunk={chunk} overlap={overlap} stem {s} channel {c} sample {k}"
                        );
                    }
                }
            }
        }
    }

    /// A source that keeps only the song, and records the frames of each
    /// read.  It fails its read number `fail_at` (from 1), if given.
    struct Recorder {
        song: Stereo,
        reads: Vec<(usize, usize)>,
        fail_at: Option<usize>,
    }

    impl MixSource for Recorder {
        type Error = String;

        fn frames(&self) -> usize {
            self.song[0].len()
        }

        fn read(&mut self, start: usize, end: usize) -> Result<Stereo, String> {
            self.reads.push((start, end));
            if self.fail_at == Some(self.reads.len()) {
                return Err(format!("read {} failed", self.reads.len()));
            }
            Ok([0, 1].map(|c| self.song[c][start..end].to_vec()))
        }
    }

    #[test]
    fn a_block_source_gives_the_whole_mix_result_exactly() {
        for &(n, chunk, overlap) in STREAM_CASES {
            let mix = song(n);
            let want = reference(&mix, chunk, overlap, fake);
            let source = Recorder {
                song: mix,
                reads: Vec::new(),
                fail_at: None,
            };
            let mut d = Demixer::new(source, chunk, overlap, 3);
            let chunks = d.num_chunks();
            let mut got: Vec<Stereo> = (0..3).map(|_| [Vec::new(), Vec::new()]).collect();
            while let Some(x) = d.next_input().unwrap() {
                d.accept(&fake(&x));
                if let Some(seg) = d.take_ready() {
                    for (g, s) in got.iter_mut().zip(seg.stems) {
                        for c in 0..2 {
                            g[c].extend_from_slice(&s[c]);
                        }
                    }
                }
            }
            let reads = d.source().reads.clone();
            for (g, s) in got.iter_mut().zip(d.finish()) {
                for c in 0..2 {
                    g[c].extend_from_slice(&s[c]);
                }
            }
            assert_eq!(reads.len(), chunks, "n={n}: one read for each chunk");
            let step = chunk / overlap;
            let border = chunk - step;
            let padded = n > 2 * border && border > 0;
            let total = if padded { n + 2 * border } else { n };
            // The frame of the song at padded index `i`, worked out again.
            let frame = |i: usize| -> usize {
                if !padded {
                    return i;
                }
                let j = i as isize - border as isize;
                let last = n as isize - 1;
                (if j < 0 {
                    -j
                } else if j > last {
                    2 * last - j
                } else {
                    j
                }) as usize
            };
            for (k, &read) in reads.iter().enumerate() {
                let pos = k * step;
                let frames: Vec<usize> = (pos..(pos + chunk).min(total)).map(frame).collect();
                let lo = *frames.iter().min().unwrap();
                let hi = *frames.iter().max().unwrap() + 1;
                assert_eq!(
                    read,
                    (lo, hi),
                    "n={n} chunk={chunk} overlap={overlap}: read {k} is the frames that its chunk reads"
                );
            }
            for &(start, end) in &reads {
                assert!(
                    start < end && end <= n,
                    "n={n}: read {start}..{end} is not inside the song"
                );
                assert!(
                    end - start <= chunk,
                    "n={n}: read {start}..{end} is longer than a chunk"
                );
            }
            for (s, (g, w)) in got.iter().zip(&want).enumerate() {
                for c in 0..2 {
                    assert_eq!(g[c].len(), n, "n={n} stem {s} channel {c}");
                    for k in 0..n {
                        assert_eq!(
                            g[c][k].to_bits(),
                            w[c][k].to_bits(),
                            "n={n} chunk={chunk} overlap={overlap} stem {s} channel {c} sample {k}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_failed_read_stops_the_demixer() {
        let source = Recorder {
            song: song(5000),
            reads: Vec::new(),
            fail_at: Some(3),
        };
        let mut d = Demixer::new(source, 1000, 2, 3);
        for _ in 0..2 {
            let x = d.next_input().unwrap().expect("a chunk");
            d.accept(&fake(&x));
        }
        assert_eq!(d.next_input(), Err("read 3 failed".to_string()));
    }

    #[test]
    fn finish_alone_matches_the_whole_song_result_exactly() {
        let mix = song(7777);
        let want = reference(&mix, 1000, 2, fake);
        let mut d = Demixer::new(mix, 1000, 2, 3);
        while let Some(x) = d.next_input().unwrap() {
            d.accept(&fake(&x));
        }
        assert_eq!(d.finish(), want);
    }
}
