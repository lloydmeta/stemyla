//! A song is a set of equally long stereo stems at one sample rate.

use crate::decode::{Audio, decode_bytes, decode_path};
use crate::resample::{SONG_RATE, to_rate};
use anyhow::{Context, Result, anyhow};
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
    /// Builds a song at [`SONG_RATE`] from decoded files (decision 0012).
    /// Mono becomes stereo and extra channels are dropped.  A stem at
    /// another rate is converted, and a stem at that rate is kept as it is,
    /// with no copy.  Shorter stems are padded with silence to the longest.
    pub fn from_audio(mut items: Vec<(String, Audio)>) -> Result<Song> {
        if items.is_empty() {
            return Err(anyhow!("no stems to load"));
        }
        items.sort_by_key(|(n, _)| rank(n));
        let mut decoded = Vec::with_capacity(items.len());
        for (name, audio) in items {
            let rate = audio.sample_rate;
            // One channel at a time, so each input is dropped before the
            // next channel is converted.
            let mut channels = audio
                .channels
                .into_iter()
                .take(2)
                .map(|c| to_rate(c, rate, SONG_RATE).with_context(|| format!("stem {name}")));
            let Some(left) = channels.next().transpose()? else {
                return Err(anyhow!("stem {name} has no channels"));
            };
            let right = channels.next().transpose()?;
            decoded.push((name, left, right));
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
            sample_rate: SONG_RATE,
            frames,
            stems,
        })
    }

    /// Loads every WAV, MP3 and FLAC in a folder as a stem (skipping
    /// `mix.*`).
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
            if !matches!(ext.as_deref(), Some("wav" | "mp3" | "flac")) {
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
pub fn padded_to(mut c: Vec<f32>, frames: usize) -> Vec<f32> {
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
            ("zeta".into(), audio(SONG_RATE, vec![vec![0.0; 3]])),
            ("bass".into(), audio(SONG_RATE, vec![vec![0.0; 3]])),
            ("drums".into(), audio(SONG_RATE, vec![vec![0.0; 3]])),
        ])
        .unwrap();
        let names: Vec<_> = s.stems.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["drums", "bass", "zeta"]);
    }

    #[test]
    fn mono_and_padding() {
        let s = Song::from_audio(vec![
            (
                "drums".into(),
                audio(SONG_RATE, vec![vec![1.0; 10], vec![0.5; 10]]),
            ),
            ("bass".into(), audio(SONG_RATE, vec![vec![0.25; 4]])),
        ])
        .unwrap();
        assert_eq!(s.frames, 10);
        assert_eq!(s.stems[1].left.len(), 10);
        assert_eq!(s.stems[1].left[..4], [0.25; 4]);
        assert_eq!(s.stems[1].right[9], 0.0);
        assert_eq!(s.stems[0].right[0], 0.5);
    }

    #[test]
    fn a_96k_flac_opens_at_44k() {
        let tone: Vec<f32> = (0..96_000)
            .map(|i| 0.25 * (0.05 * i as f32).sin())
            .collect();
        let flac = crate::test_flac::compressed(&tone, &tone, 96_000);
        let s = Song::load_files(vec![("lloyd.flac".into(), flac)]).unwrap();
        assert_eq!((s.sample_rate, s.frames), (SONG_RATE, 44_100));
    }

    #[test]
    fn a_folder_with_a_flac_stem_loads_it() {
        let dir = std::env::temp_dir().join(format!("stemyla-flac-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let tone: Vec<f32> = (0..4410).map(|i| 0.25 * (0.05 * i as f32).sin()).collect();
        let files = [
            (
                "bass.flac",
                crate::test_flac::compressed(&tone, &tone, SONG_RATE),
            ),
            (
                "drums.wav",
                crate::wav::encode_pcm16(&tone, &tone, SONG_RATE),
            ),
            ("notes.txt", b"not audio".into()),
        ];
        for (name, bytes) in &files {
            std::fs::write(dir.join(name), bytes).unwrap();
        }
        let loaded = Song::load_folder(&dir);
        std::fs::remove_dir_all(&dir).unwrap();
        let s = loaded.unwrap();
        let names: Vec<_> = s.stems.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["drums", "bass"]);
        assert_eq!(s.frames, 4410);
    }

    #[test]
    fn a_48k_wav_opens_at_44k() {
        let tone: Vec<f32> = (0..48_000)
            .map(|i| 0.25 * (0.05 * i as f32).sin())
            .collect();
        let wav = crate::wav::encode_pcm16(&tone, &tone, 48_000);
        let s = Song::load_files(vec![("lloyd.wav".into(), wav)]).unwrap();
        assert_eq!((s.sample_rate, s.frames), (SONG_RATE, 44_100));
        assert_eq!(s.stems[0].left.len(), 44_100);
        assert_eq!(s.stems[0].right.len(), 44_100);
        // Both sides hold the same tone, so both must convert the same way.
        assert!(s.stems[0].right.as_slice() == s.stems[0].left.as_slice());
    }

    #[test]
    fn each_channel_of_a_48k_stem_converts_on_its_own() {
        // The right side is the left at half the level.  Halving is exact
        // in floating point, so the converted right is exactly half the
        // converted left, and a dropped or unconverted right side differs.
        let left: Vec<f32> = (0..4_800).map(|i| 0.25 * (0.05 * i as f32).sin()).collect();
        let right: Vec<f32> = left.iter().map(|v| v * 0.5).collect();
        let s = Song::from_audio(vec![("bass".into(), audio(48_000, vec![left, right]))]).unwrap();
        let bass = &s.stems[0];
        assert_eq!((bass.left.len(), bass.right.len()), (4_410, 4_410));
        for (l, r) in bass.left.iter().zip(bass.right.iter()) {
            assert_eq!((l * 0.5).to_bits(), r.to_bits());
        }
        assert!(
            bass.left.iter().any(|v| v.abs() > 0.1),
            "the left side is not silent"
        );
    }

    #[test]
    fn stems_at_mixed_rates_all_open_at_44k() {
        let s = Song::from_audio(vec![
            ("drums".into(), audio(48_000, vec![vec![0.5; 4_800]])),
            ("bass".into(), audio(22_050, vec![vec![0.25; 2_205]])),
            ("vocals".into(), audio(SONG_RATE, vec![vec![0.1; 4_410]])),
        ])
        .unwrap();
        assert_eq!((s.sample_rate, s.frames), (SONG_RATE, 4_410));
        // Away from the edges, a constant converts to the same constant.
        for (stem, level) in s.stems.iter().zip([0.5, 0.25, 0.1]) {
            assert_eq!(stem.left.len(), 4_410, "{}", stem.name);
            assert!((stem.left[2_205] - level).abs() < 1e-6, "{}", stem.name);
        }
    }

    #[test]
    fn a_44k_stem_is_not_copied() {
        let (left, right) = (vec![0.25, -0.5, 0.75], vec![0.1, 0.2, 0.3]);
        let at = (left.as_ptr(), right.as_ptr());
        let s =
            Song::from_audio(vec![("bass".into(), audio(SONG_RATE, vec![left, right]))]).unwrap();
        let bass = &s.stems[0];
        assert_eq!(bass.left.as_slice().as_ptr(), at.0);
        assert_eq!(bass.right.as_slice().as_ptr(), at.1);
        assert_eq!(bass.left, vec![0.25, -0.5, 0.75]);
        assert_eq!(bass.right, vec![0.1, 0.2, 0.3]);
    }

    #[test]
    fn a_stem_at_an_odd_rate_is_refused() {
        let e = Song::from_audio(vec![("bass".into(), audio(44_056, vec![vec![0.0; 10]]))])
            .unwrap_err();
        assert!(
            format!("{e:#}").contains("stem bass: songs at 44056 Hz"),
            "{e:#}"
        );
    }

    #[test]
    fn a_mono_stem_shares_one_buffer_and_clones_share_every_buffer() {
        let s = Song::from_audio(vec![
            ("bass".into(), audio(SONG_RATE, vec![vec![0.25; 4]])),
            (
                "drums".into(),
                audio(SONG_RATE, vec![vec![1.0; 4], vec![0.5; 4]]),
            ),
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
                audio(SONG_RATE, vec![vec![0.25; 600], vec![0.5; 600]]),
            ),
            ("drums".into(), audio(SONG_RATE, vec![vec![1.0; 1003]])),
        ])
        .unwrap();
        for stem in &s.stems {
            assert_eq!(stem.left.0.capacity(), 1003, "{}", stem.name);
            assert_eq!(stem.right.0.capacity(), 1003, "{}", stem.name);
        }
    }
}
