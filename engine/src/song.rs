//! A song is a set of equally long stereo stems at one sample rate.

use crate::decode::{Audio, decode_bytes, decode_path};
use anyhow::{Result, anyhow};
use std::ops::Deref;
use std::path::Path;
use std::sync::Arc;

/// One channel of a stem: immutable once made, and shared rather than
/// copied when a stem or song is cloned.  A song's stems run to hundreds of
/// MB, and the web page's WebAssembly memory never shrinks, so a copy made
/// for a moment (to save stems while they are shown, say) stays for good.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Samples(Arc<Vec<f32>>);

impl Samples {
    pub fn as_slice(&self) -> &[f32] {
        &self.0
    }

    /// Whether `self` and `other` are the same buffer, not just equal.
    pub fn shares(&self, other: &Samples) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Deref for Samples {
    type Target = [f32];

    fn deref(&self) -> &[f32] {
        &self.0
    }
}

/// Takes the vector's buffer: no copy when it has no spare capacity.  Spare
/// capacity is released first, since samples never grow and would hold it
/// for good, and releasing it may reallocate the buffer once.
impl From<Vec<f32>> for Samples {
    fn from(mut v: Vec<f32>) -> Self {
        v.shrink_to_fit();
        Self(Arc::new(v))
    }
}

impl PartialEq<Vec<f32>> for Samples {
    fn eq(&self, other: &Vec<f32>) -> bool {
        *self.0 == *other
    }
}

#[derive(Clone, Debug)]
pub struct Stem {
    pub name: String,
    pub left: Samples,
    pub right: Samples,
}

#[derive(Clone, Debug)]
pub struct Song {
    pub sample_rate: u32,
    pub frames: usize,
    pub stems: Vec<Stem>,
}

/// Lane order used when stems are loaded from a folder.  Unknown names go
/// last, alphabetically.
const PREFERRED_ORDER: [&str; 10] = [
    "drums",
    "bass",
    "guitar",
    "piano",
    "keys",
    "vocals",
    "lead",
    "other",
    "harmonic",
    "percussive",
];

/// File names that are never stems: the original mix is recomputed from the
/// stems so every lane stays aligned.
const NOT_STEMS: [&str; 3] = ["mix", "original", "truth"];

fn rank(name: &str) -> (usize, String) {
    let pos = PREFERRED_ORDER
        .iter()
        .position(|p| *p == name)
        .unwrap_or(PREFERRED_ORDER.len());
    (pos, name.to_owned())
}

impl Song {
    /// Builds a song from decoded files.  Mono becomes stereo, extra channels
    /// are dropped, every stem is resampled to the first stem's rate, and
    /// shorter stems are padded with silence to the longest.
    pub fn from_audio(mut items: Vec<(String, Audio)>) -> Result<Song> {
        if items.is_empty() {
            return Err(anyhow!("no stems to load"));
        }
        items.sort_by_key(|(n, _)| rank(n));
        let sample_rate = items[0].1.sample_rate;
        let mut decoded = Vec::with_capacity(items.len());
        for (name, audio) in items {
            let mut channels = audio.channels.into_iter().take(2).map(|c| {
                if audio.sample_rate == sample_rate {
                    c
                } else {
                    resample_linear(&c, audio.sample_rate, sample_rate)
                }
            });
            let Some(left) = channels.next() else {
                return Err(anyhow!("stem {name} has no channels"));
            };
            decoded.push((name, left, channels.next()));
        }
        let frames = decoded.iter().map(|(_, l, _)| l.len()).max().unwrap_or(0);
        let padded = |c: Vec<f32>| -> Samples { padded_to(c, frames).into() };
        let stems = decoded
            .into_iter()
            .map(|(name, left, right)| {
                let left = padded(left);
                // A mono stem plays the same samples on both sides.
                let right = right.map_or_else(|| left.clone(), padded);
                Stem { name, left, right }
            })
            .collect();
        Ok(Song {
            sample_rate,
            frames,
            stems,
        })
    }

    /// Loads every WAV/MP3 in a folder as a stem (skipping `mix.*`).
    pub fn load_folder(dir: &Path) -> Result<Song> {
        let mut items = Vec::new();
        let mut entries: Vec<_> = std::fs::read_dir(dir)?.filter_map(|e| e.ok()).collect();
        entries.sort_by_key(|e| e.path());
        for e in entries {
            let path = e.path();
            let ext = path
                .extension()
                .and_then(|x| x.to_str())
                .map(str::to_ascii_lowercase);
            if !matches!(ext.as_deref(), Some("wav" | "mp3")) {
                continue;
            }
            let name = stem_name(&path.to_string_lossy());
            if NOT_STEMS.contains(&name.as_str()) {
                continue;
            }
            items.push((name, decode_path(&path)?));
        }
        Song::from_audio(items)
    }

    /// Loads in-memory files, as from a multi-file drop: `(file name, bytes)`.
    pub fn load_files(files: Vec<(String, Vec<u8>)>) -> Result<Song> {
        let mut items = Vec::new();
        for (file_name, bytes) in files {
            let name = stem_name(&file_name);
            if NOT_STEMS.contains(&name.as_str()) {
                continue;
            }
            let ext = file_name.rsplit('.').next().map(str::to_ascii_lowercase);
            items.push((name, decode_bytes(bytes, ext.as_deref())?));
        }
        Song::from_audio(items)
    }

    pub fn duration_s(&self) -> f64 {
        self.frames as f64 / self.sample_rate as f64
    }

    pub fn stem_index(&self, name: &str) -> Option<usize> {
        self.stems.iter().position(|s| s.name == name)
    }

    /// The mix lane: every stem at full volume, summed with `dsp::mix`.
    pub fn mix_lane(&self) -> (Vec<f32>, Vec<f32>) {
        let volumes = vec![1.0; self.stems.len()];
        let lefts: Vec<&[f32]> = self.stems.iter().map(|s| s.left.as_slice()).collect();
        let rights: Vec<&[f32]> = self.stems.iter().map(|s| s.right.as_slice()).collect();
        (dsp::mix(&lefts, &volumes), dsp::mix(&rights, &volumes))
    }
}

/// `c` padded with silence (or cut) to `frames`, growing by exactly what it
/// needs: `resize` alone would double a full buffer, briefly holding three
/// times the channel.
fn padded_to(mut c: Vec<f32>, frames: usize) -> Vec<f32> {
    c.reserve_exact(frames.saturating_sub(c.len()));
    c.resize(frames, 0.0);
    c
}

/// `"path/to/Drums.wav"` becomes `"drums"`.
pub fn stem_name(file_name: &str) -> String {
    let base = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name);
    let base = base.rsplit_once('.').map_or(base, |(b, _)| b);
    base.to_ascii_lowercase()
}

/// Linear-interpolation resampler.  Good enough for aligning a stray stem
/// to the song rate; not used on the playback path.
pub fn resample_linear(x: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || x.is_empty() {
        #[expect(
            clippy::disallowed_methods,
            reason = "MEM-1: the contract returns a new buffer; Song::from_audio does not call this at the same rate"
        )]
        return x.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let n = ((x.len() as f64) / ratio).floor() as usize;
    (0..n)
        .map(|i| {
            let p = i as f64 * ratio;
            let j = p.floor() as usize;
            let f = (p - j as f64) as f32;
            let a = x[j];
            let b = if j + 1 < x.len() { x[j + 1] } else { a };
            a + (b - a) * f
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mem1_lint_is_on() {
        #[expect(
            clippy::disallowed_methods,
            reason = "MEM-1 canary: clippy fails here if this path is off"
        )]
        let copy = [1.0_f32].to_vec();
        assert_eq!(copy, [1.0]);
    }

    fn audio(rate: u32, chans: Vec<Vec<f32>>) -> Audio {
        Audio {
            sample_rate: rate,
            channels: chans,
        }
    }

    #[test]
    fn names_and_order() {
        assert_eq!(stem_name("a/b/Drums.WAV"), "drums");
        let s = Song::from_audio(vec![
            ("zeta".into(), audio(10, vec![vec![0.0; 3]])),
            ("bass".into(), audio(10, vec![vec![0.0; 3]])),
            ("drums".into(), audio(10, vec![vec![0.0; 3]])),
        ])
        .unwrap();
        let names: Vec<_> = s.stems.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["drums", "bass", "zeta"]);
    }

    #[test]
    fn mono_padding_and_resampling() {
        let s = Song::from_audio(vec![
            (
                "drums".into(),
                audio(100, vec![vec![1.0; 10], vec![0.5; 10]]),
            ),
            ("bass".into(), audio(50, vec![vec![0.25; 4]])),
        ])
        .unwrap();
        assert_eq!(s.frames, 10);
        assert_eq!(s.stems[1].left.len(), 10);
        assert_eq!(s.stems[1].left[..8], [0.25; 8]);
        assert_eq!(s.stems[1].right[9], 0.0);
        assert_eq!(s.stems[0].right[0], 0.5);
    }

    #[test]
    fn a_mono_stem_shares_one_buffer_and_clones_share_every_buffer() {
        let s = Song::from_audio(vec![
            ("bass".into(), audio(10, vec![vec![0.25; 4]])),
            ("drums".into(), audio(10, vec![vec![1.0; 4], vec![0.5; 4]])),
        ])
        .unwrap();
        let (drums, bass) = (&s.stems[0], &s.stems[1]);
        assert!(bass.left.shares(&bass.right), "mono: one buffer");
        assert!(!drums.left.shares(&drums.right));
        let copy = s.clone();
        for (a, b) in s.stems.iter().zip(&copy.stems) {
            assert!(a.left.shares(&b.left) && a.right.shares(&b.right));
        }
    }

    #[test]
    fn samples_from_a_vector_keep_its_buffer() {
        let v = vec![1.0, 2.0];
        let at = v.as_ptr();
        let s = Samples::from(v);
        assert_eq!(s.as_slice().as_ptr(), at);
        assert_eq!(s, vec![1.0, 2.0]);
    }

    #[test]
    fn samples_from_a_vector_with_spare_capacity_keep_none_of_it() {
        let mut v = Vec::with_capacity(1024);
        v.extend_from_slice(&[0.5; 1003]);
        let s = Samples::from(v);
        assert_eq!(s.0.capacity(), 1003);
        assert_eq!(s, vec![0.5; 1003]);
    }

    #[test]
    fn padding_grows_a_full_buffer_by_exactly_the_padding() {
        let padded = padded_to(vec![0.25; 600], 1003);
        assert_eq!(padded.capacity(), 1003);
        assert_eq!(padded[..600], [0.25; 600]);
        assert_eq!(padded[600..], [0.0; 403]);
        assert_eq!(padded_to(vec![0.25; 600], 4), vec![0.25; 4]);
    }

    #[test]
    fn padded_stems_keep_no_spare_capacity() {
        let s = Song::from_audio(vec![
            // Padded from 600 samples to 1,003.
            (
                "bass".into(),
                audio(10, vec![vec![0.25; 600], vec![0.5; 600]]),
            ),
            ("drums".into(), audio(10, vec![vec![1.0; 1003]])),
        ])
        .unwrap();
        for stem in &s.stems {
            assert_eq!(stem.left.0.capacity(), 1003, "{}", stem.name);
            assert_eq!(stem.right.0.capacity(), 1003, "{}", stem.name);
        }
    }
}
