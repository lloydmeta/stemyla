//! The playback engine: one read position for all stems, mix first, then
//! stretch once.
//!
//! For every block:
//! 1. read every stem at the shared position,
//! 2. multiply each by its (smoothed) gain and add them up,
//! 3. feed the sum through one stretcher,
//! 4. apply the master volume and clamp to ±1,
//! 5. output.
//!
//! Because there is only one read position, stems cannot drift apart.

use crate::params::{MAX_TEMPO, SharedParams};
use crate::song::Song;
use crate::stretch::{Stretcher, StretcherKind};
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;

#[derive(Clone, Copy, Debug)]
pub struct PlayerConfig {
    /// Largest number of output frames rendered in one inner step.
    pub block: usize,
    /// Input frames read, mixed and pushed into the stretcher at a time.
    pub hop: usize,
    /// Length of the loop crossfade and of the seek fade-out and fade-in.
    /// Zero disables them (used by tests to show the click they prevent).
    pub crossfade: usize,
}

impl Default for PlayerConfig {
    fn default() -> Self {
        Self {
            block: 512,
            hop: 256,
            crossfade: 256,
        }
    }
}

/// Where a run of contiguous song frames starts in the stretcher's input.
#[derive(Clone, Copy, Debug)]
struct Anchor {
    input_index: u64,
    song_pos: u64,
}

enum Fade {
    None,
    /// Fading out before a pending seek; `k` frames done.
    Out {
        k: usize,
        target: u64,
    },
    /// Fading in after a seek; `k` frames done.
    In {
        k: usize,
    },
}

pub struct Player {
    song: Arc<Song>,
    params: Arc<SharedParams>,
    stretcher: Box<dyn Stretcher>,
    cfg: PlayerConfig,

    /// Next song frame to read (input side of the stretcher).
    pos: u64,
    /// Gains actually applied, ramped towards the targets once per hop.
    gains: Vec<f32>,
    /// Crossfade tables: `fin[k] + fout[k] == 1` for every k.
    fin: Vec<f32>,
    fout: Vec<f32>,
    /// Loop crossfade in progress: the old position still fading out.
    xf_old: Option<u64>,
    xf_k: usize,
    fade: Fade,

    // Playhead bookkeeping.
    anchors: VecDeque<Anchor>,
    pushed: u64,
    consumed: f64,
    chipmunk: bool,

    // Scratch buffers, sized once so the audio path does not allocate.
    in_l: Vec<f32>,
    in_r: Vec<f32>,
    pos_buf: Vec<u64>,
    old_buf: Vec<Option<u64>>,
    xf_buf: Vec<usize>,
    chip_l: Vec<f32>,
    chip_r: Vec<f32>,
}

impl Player {
    pub fn new(song: Arc<Song>, params: Arc<SharedParams>, kind: StretcherKind) -> Self {
        let stretcher = kind.build(song.sample_rate);
        Self::with_config(song, params, stretcher, PlayerConfig::default())
    }

    /// Makes a player, and warms its stretcher up on this thread, so the
    /// audio callback does not allocate (RT-1).
    pub fn with_config(
        song: Arc<Song>,
        params: Arc<SharedParams>,
        mut stretcher: Box<dyn Stretcher>,
        cfg: PlayerConfig,
    ) -> Self {
        assert_eq!(
            song.stems.len(),
            params.stems.len(),
            "params must match stems"
        );
        // A fade_in of n+1 ones gives 0, 1/(n+1), .., n/(n+1).  Dropping the
        // leading zero leaves a fade-in whose reverse is the matching
        // fade-out: each pair sums to exactly 1, so the level never dips.
        let ramp = dsp::fade_in(&vec![1.0; cfg.crossfade + 1]);
        let fin: Vec<f32> = ramp[1..].to_vec();
        let mut fout = fin.clone();
        fout.reverse();
        let gains = (0..song.stems.len())
            .map(|i| params.effective_gain(i))
            .collect();
        let max_in = cfg.hop.max(cfg.block * 2);
        stretcher.warm_up(f64::from(MAX_TEMPO), cfg.block);
        Self {
            song,
            params,
            stretcher,
            cfg,
            pos: 0,
            gains,
            fin,
            fout,
            xf_old: None,
            xf_k: 0,
            fade: Fade::None,
            anchors: VecDeque::with_capacity(1024),
            pushed: 0,
            consumed: 0.0,
            chipmunk: false,
            in_l: vec![0.0; max_in],
            in_r: vec![0.0; max_in],
            pos_buf: vec![0; max_in],
            old_buf: vec![None; max_in],
            xf_buf: vec![0; max_in],
            chip_l: vec![0.0; cfg.block * 2],
            chip_r: vec![0.0; cfg.block * 2],
        }
    }

    pub fn params(&self) -> &Arc<SharedParams> {
        &self.params
    }

    pub fn song(&self) -> &Arc<Song> {
        &self.song
    }

    pub fn stretcher_name(&self) -> &'static str {
        self.stretcher.name()
    }

    /// Offline render: plays `frames` frames and returns them per channel.
    pub fn render(&mut self, frames: usize) -> [Vec<f32>; 2] {
        let mut l = vec![0.0; frames];
        let mut r = vec![0.0; frames];
        self.process(&mut l, &mut r);
        [l, r]
    }

    /// Fills both output slices.  This is what the audio callback calls.
    pub fn process(&mut self, out_l: &mut [f32], out_r: &mut [f32]) {
        let p = Arc::clone(&self.params);
        let playing = p.playing.load(Relaxed);

        if let Some(target) = p.take_seek() {
            if playing && self.cfg.crossfade > 0 {
                self.fade = Fade::Out { k: 0, target };
            } else {
                self.jump_to(target);
            }
        }

        if !playing {
            out_l.fill(0.0);
            out_r.fill(0.0);
            if matches!(self.fade, Fade::Out { .. }) {
                if let Fade::Out { target, .. } = self.fade {
                    self.jump_to(target);
                }
                self.fade = Fade::None;
            }
            return;
        }

        let chip = p.chipmunk.load(Relaxed);
        if chip != self.chipmunk {
            self.chipmunk = chip;
            let here = self.playhead();
            self.jump_to(here);
        }

        let mut done = 0;
        while done < out_l.len() {
            let mut n = (out_l.len() - done).min(self.cfg.block);
            if let Fade::Out { k, .. } = self.fade {
                n = n.min(self.cfg.crossfade - k);
            }
            let (l, r) = (&mut out_l[done..done + n], &mut out_r[done..done + n]);
            if self.chipmunk {
                self.render_chipmunk(l, r);
            } else {
                self.render_stretched(l, r);
            }
            self.apply_fade(l, r);
            done += n;
        }

        let master = p.master.get();
        for i in 0..out_l.len() {
            out_l[i] = (out_l[i] * master).clamp(-1.0, 1.0);
            out_r[i] = (out_r[i] * master).clamp(-1.0, 1.0);
        }

        let head = self.playhead();
        p.playhead.store(head, Relaxed);
        if p.loop_region().is_none() && head >= self.song.frames as u64 {
            p.playing.store(false, Relaxed);
        }
    }

    fn apply_fade(&mut self, l: &mut [f32], r: &mut [f32]) {
        match self.fade {
            Fade::None => {}
            Fade::Out { k, target } => {
                for i in 0..l.len() {
                    let g = self.fout[k + i];
                    l[i] *= g;
                    r[i] *= g;
                }
                let k = k + l.len();
                if k >= self.cfg.crossfade {
                    self.jump_to(target);
                    self.fade = Fade::In { k: 0 };
                } else {
                    self.fade = Fade::Out { k, target };
                }
            }
            Fade::In { k } => {
                let m = l.len().min(self.cfg.crossfade - k);
                for i in 0..m {
                    let g = self.fin[k + i];
                    l[i] *= g;
                    r[i] *= g;
                }
                self.fade = if k + m >= self.cfg.crossfade {
                    Fade::None
                } else {
                    Fade::In { k: k + m }
                };
            }
        }
    }

    /// Moves the read position and resets the stretcher.
    fn jump_to(&mut self, target: u64) {
        self.pos = target.min(self.song.frames as u64);
        self.stretcher.reset();
        self.anchors.clear();
        self.pushed = 0;
        self.consumed = 0.0;
        self.xf_old = None;
        self.xf_k = 0;
        self.params.playhead.store(self.pos, Relaxed);
    }

    fn render_stretched(&mut self, l: &mut [f32], r: &mut [f32]) {
        let tempo = self.params.tempo() as f64;
        self.stretcher.set_tempo(tempo);
        let hop = self.cfg.hop;
        let mut guard = 0;
        while !self.stretcher.can_pull(l.len()) && guard < 1024 {
            self.read_mixed(hop);
            self.stretcher.push(&self.in_l[..hop], &self.in_r[..hop]);
            guard += 1;
        }
        self.stretcher.pull(l, r);
        self.consumed += l.len() as f64 * tempo;
    }

    /// Chipmunk mode: read twice as many frames and keep every second one,
    /// in place, as `dsp::double_speed` does without allocating.  Bypasses
    /// the stretcher.
    fn render_chipmunk(&mut self, l: &mut [f32], r: &mut [f32]) {
        let n2 = l.len() * 2;
        let mut got = 0;
        while got < n2 {
            let m = (n2 - got).min(self.cfg.hop);
            self.read_mixed(m);
            self.chip_l[got..got + m].copy_from_slice(&self.in_l[..m]);
            self.chip_r[got..got + m].copy_from_slice(&self.in_r[..m]);
            got += m;
        }
        for i in 0..l.len() {
            l[i] = self.chip_l[2 * i];
            r[i] = self.chip_r[2 * i];
        }
        // No stretcher in the path: what was just read is what is heard.
        self.anchors.clear();
        self.pushed = 0;
        self.consumed = 0.0;
    }

    /// Reads `n` frames of every stem at the shared position, applies the
    /// gains (ramped across the block), sums them into `in_l`/`in_r`, and
    /// handles loop wraps with a crossfade.
    fn read_mixed(&mut self, n: usize) {
        let frames = self.song.frames as u64;
        let looping = self.params.loop_region();

        // 1. Work out which song frame each output frame comes from.
        for i in 0..n {
            if let Some((s, e)) = looping {
                let e = e.min(frames);
                if s < e && self.pos >= e {
                    if self.cfg.crossfade > 0 {
                        self.xf_old = Some(self.pos);
                        self.xf_k = 0;
                    }
                    self.pos = dsp::wrap_loop(self.pos as usize, s as usize, e as usize) as u64;
                    self.drop_oldest_anchor_when_full();
                    self.anchors.push_back(Anchor {
                        input_index: self.pushed + i as u64,
                        song_pos: self.pos,
                    });
                }
            }
            if self.anchors.is_empty() {
                self.anchors.push_back(Anchor {
                    input_index: self.pushed + i as u64,
                    song_pos: self.pos,
                });
            }
            self.pos_buf[i] = self.pos;
            self.pos += 1;
            match self.xf_old {
                Some(old) if self.xf_k < self.cfg.crossfade => {
                    self.old_buf[i] = Some(old);
                    self.xf_buf[i] = self.xf_k;
                    self.xf_old = Some(old + 1);
                    self.xf_k += 1;
                }
                _ => {
                    self.old_buf[i] = None;
                    self.xf_old = None;
                }
            }
        }
        self.pushed += n as u64;

        // 2. Mix with gains ramped from the current to the target values.
        self.in_l[..n].fill(0.0);
        self.in_r[..n].fill(0.0);
        let song = Arc::clone(&self.song);
        for (si, stem) in song.stems.iter().enumerate() {
            let g0 = self.gains[si];
            let g1 = self.params.effective_gain(si);
            self.gains[si] = g1;
            if g0 == 0.0 && g1 == 0.0 {
                continue;
            }
            let step = (g1 - g0) / n as f32;
            for i in 0..n {
                let g = g0 + step * (i + 1) as f32;
                let p = self.pos_buf[i];
                let (mut vl, mut vr) = if p < frames {
                    (stem.left[p as usize], stem.right[p as usize])
                } else {
                    (0.0, 0.0)
                };
                if let Some(old) = self.old_buf[i] {
                    let k = self.xf_buf[i];
                    let (ol, or) = if old < frames {
                        (stem.left[old as usize], stem.right[old as usize])
                    } else {
                        (0.0, 0.0)
                    };
                    vl = vl * self.fin[k] + ol * self.fout[k];
                    vr = vr * self.fin[k] + or * self.fout[k];
                }
                self.in_l[i] += vl * g;
                self.in_r[i] += vr * g;
            }
        }
    }

    /// Keeps `anchors` within its capacity, so the audio callback does not
    /// allocate (RT-1).  Only a loop of under 16 frames fills it.  The
    /// anchor dropped can be the one being heard, and the playhead then
    /// shows the loop's start early, while the stretcher's buffered audio
    /// plays; the audio does not change.
    fn drop_oldest_anchor_when_full(&mut self) {
        if self.anchors.len() == self.anchors.capacity() {
            self.anchors.pop_front();
        }
    }

    /// The song frame being heard right now: the read position, minus what
    /// is still inside the stretcher, mapped back through any loop jumps.
    pub fn playhead(&mut self) -> u64 {
        if self.chipmunk || self.anchors.is_empty() {
            return self.pos;
        }
        let tempo = self.params.tempo() as f64;
        let audible = self.consumed - self.stretcher.latency() * tempo;
        if audible < 0.0 {
            return self.anchors[0].song_pos;
        }
        while self.anchors.len() > 1 && (self.anchors[1].input_index as f64) <= audible {
            self.anchors.pop_front();
        }
        let a = self.anchors[0];
        a.song_pos + (audible - a.input_index as f64).max(0.0) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::song::Stem;
    use crate::stretch::Varispeed;

    fn song(stems: Vec<(&str, Vec<f32>)>) -> Arc<Song> {
        let frames = stems[0].1.len();
        Arc::new(Song {
            sample_rate: 44_100,
            frames,
            stems: stems
                .into_iter()
                .map(|(n, v)| Stem {
                    name: n.into(),
                    left: v.clone().into(),
                    right: v.into(),
                })
                .collect(),
        })
    }

    fn player(song: Arc<Song>, crossfade: usize) -> Player {
        let params = Arc::new(SharedParams::new(song.stems.len()));
        params.playing.store(true, Relaxed);
        Player::with_config(
            song,
            params,
            Box::new(Varispeed::new()),
            PlayerConfig {
                crossfade,
                ..Default::default()
            },
        )
    }

    fn noise(n: usize, seed: u32) -> Vec<f32> {
        let mut x = seed;
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x as f32 / u32::MAX as f32 - 0.5) * 0.4
            })
            .collect()
    }

    #[test]
    fn identity_render_equals_dsp_mix() {
        let a = noise(10_000, 1);
        let b = noise(10_000, 2);
        let s = song(vec![("a", a.clone()), ("b", b.clone())]);
        let mut p = player(s, 256);
        p.params.set_volume(0, 0.5);
        p.params.set_volume(1, 0.75);
        // Start from settled gains so no ramp is involved.
        p.gains = vec![0.5, 0.75];
        let [l, _] = p.render(9_000);
        let reference = dsp::mix(&[&a[..9_000], &b[..9_000]], &[0.5, 0.75]);
        for i in 0..9_000 {
            assert!((l[i] - reference[i]).abs() < 1e-6, "frame {i}");
        }
    }

    #[test]
    fn seek_lands_within_one_block() {
        let s = song(vec![("a", noise(100_000, 3))]);
        let mut p = player(s, 256);
        p.render(1_000);
        p.params.request_seek(50_000);
        p.render(512);
        let h = p.params.playhead();
        // After the fade-out (256 frames) and fade-in, the head is just past
        // the target, by at most one block.
        assert!((50_000..=50_000 + 512).contains(&h), "playhead {h}");
    }

    #[test]
    fn paused_player_is_silent_and_seeks_immediately() {
        let s = song(vec![("a", vec![0.5; 1000])]);
        let mut p = player(s, 256);
        p.params.playing.store(false, Relaxed);
        p.params.request_seek(700);
        let [l, _] = p.render(100);
        assert!(l.iter().all(|v| *v == 0.0));
        assert_eq!(p.pos, 700);
    }

    #[test]
    fn stops_at_the_end_without_a_loop() {
        let s = song(vec![("a", vec![0.1; 2_000])]);
        let mut p = player(s, 256);
        p.render(3_000);
        assert!(!p.params.playing.load(Relaxed));
    }

    #[test]
    fn loop_wraps_and_playhead_stays_inside() {
        let ramp: Vec<f32> = (0..20_000).map(|i| i as f32 / 20_000.0).collect();
        let s = song(vec![("a", ramp)]);
        let mut p = player(s, 0);
        p.params.set_loop(1_000, 3_000);
        for _ in 0..40 {
            p.render(256);
            let h = p.params.playhead();
            assert!(h < 3_000, "playhead {h} escaped the loop");
        }
        // Without a crossfade, output after the wrap is the loop start.
        let [l, _] = p.render(2_000);
        assert!(l.iter().all(|v| (0.0..0.15 + 1e-3).contains(v)));
    }

    #[test]
    fn chipmunk_matches_dsp_double_speed() {
        let rising: Vec<f32> = (0..4_000).map(|i| i as f32 / 4_000.0).collect();
        let falling: Vec<f32> = rising.iter().map(|v| 1.0 - v).collect();
        let s = Arc::new(Song {
            sample_rate: 44_100,
            frames: rising.len(),
            stems: vec![Stem {
                name: "lloyd".into(),
                left: rising.clone().into(),
                right: falling.clone().into(),
            }],
        });
        let mut p = player(s, 256);
        p.gains = vec![1.0];
        p.params.chipmunk.store(true, Relaxed);
        let [l, r] = p.render(1_000);
        assert_eq!(l, dsp::double_speed(&rising[..2_000]));
        assert_eq!(r, dsp::double_speed(&falling[..2_000]));
    }

    #[test]
    fn a_very_short_loop_keeps_its_anchors() {
        let s = song(vec![("a", noise(20_000, 4))]);
        let mut p = player(s, 256);
        let capacity = p.anchors.capacity();
        p.params.set_tempo(1.5);
        p.params.set_loop(1_000, 1_001);
        p.render(8_192);
        assert_eq!(p.anchors.capacity(), capacity, "RT-1: the queue grew");
        let h = p.params.playhead();
        assert!((1_000..=1_001).contains(&h), "playhead {h}");
    }

    #[test]
    fn chipmunk_reads_twice_as_fast() {
        let ramp: Vec<f32> = (0..10_000).map(|i| i as f32 / 10_000.0).collect();
        let s = song(vec![("a", ramp)]);
        let mut p = player(s, 256);
        p.params.chipmunk.store(true, Relaxed);
        let [l, _] = p.render(1_000);
        assert!((l[500] - 1_000.0 / 10_000.0).abs() < 1e-6);
        assert_eq!(p.params.playhead(), 2_000);
    }
}
