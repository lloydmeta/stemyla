//! Spectrogram textures for the lanes, and the classic (HPSS) drums job.

use crate::Lane;
use eframe::egui::{self, Color32, ColorImage, TextureOptions};
use engine::Song;
use engine::hpss::{HpssConfig, hpss, spectrogram_db};
use engine::mixdown::MixRecipe;
use std::sync::Arc;

const ROWS: usize = 128;
const MAX_COLS: usize = 1024;
const MAX_HZ: f32 = 8_000.0;

/// Dark blue → purple → orange → yellow, by dB between -90 and -10.
fn colour(db: f32) -> Color32 {
    let t = ((db + 90.0) / 80.0).clamp(0.0, 1.0);
    let stops = [
        (0.0, [8.0, 8.0, 24.0]),
        (0.35, [80.0, 20.0, 110.0]),
        (0.7, [230.0, 90.0, 50.0]),
        (1.0, [250.0, 240.0, 150.0]),
    ];
    for w in stops.windows(2) {
        let ((t0, c0), (t1, c1)) = (w[0], w[1]);
        if t <= t1 {
            let f = (t - t0) / (t1 - t0);
            let c = |i: usize| (c0[i] + (c1[i] - c0[i]) * f) as u8;
            return Color32::from_rgb(c(0), c(1), c(2));
        }
    }
    Color32::from_rgb(250, 240, 150)
}

fn mid(left: &[f32], right: &[f32]) -> Vec<f32> {
    left.iter().zip(right).map(|(a, b)| 0.5 * (a + b)).collect()
}

/// Creates a spectrogram texture for every lane that lacks one.
/// The mix lane shows `mix`, the mix as heard, or else the sources' mix.
pub fn ensure_textures(
    ctx: &egui::Context,
    lanes: &mut [Lane],
    song: Option<&Arc<Song>>,
    mix: Option<&MixRecipe>,
) {
    let Some(song) = song else { return };
    for lane in lanes.iter_mut().filter(|l| l.spectrogram.is_none()) {
        let x = lane_signal(song, lane.stem, mix);
        lane.spectrogram = Some(texture(ctx, &lane.name, &x, song.sample_rate));
    }
}

/// The signal of a lane's spectrogram: the mean of its two channels.  For
/// the mix lane it is made block by block (`export::mix_mid`), so that only
/// that one song-sized buffer is made.
pub fn lane_signal(song: &Arc<Song>, stem: Option<usize>, mix: Option<&MixRecipe>) -> Vec<f32> {
    match (stem, mix) {
        (Some(i), _) => mid(&song.stems[i].left, &song.stems[i].right),
        (None, Some(recipe)) => crate::export::mix_mid(song, recipe, engine::overview::COLUMNS),
        (None, None) => {
            let (l, r) = crate::mix_of_sources(song);
            mid(&l, &r)
        }
    }
}

/// The spectrogram texture of the signal `x` of the lane `name`.
pub fn texture(ctx: &egui::Context, name: &str, x: &[f32], rate: u32) -> egui::TextureHandle {
    let (db, cols, rows) = spectrogram_db(x, rate, MAX_HZ, ROWS, MAX_COLS);
    let pixels: Vec<Color32> = db.iter().map(|v| colour(*v)).collect();
    let img = ColorImage::new([cols, rows], pixels);
    ctx.load_texture(format!("spec-{name}"), img, TextureOptions::LINEAR)
}

/// HPSS on the song's mix (both channels): returns the percussive part.
pub fn classic_drums(song: &Song) -> (Vec<f32>, Vec<f32>) {
    let (l, r) = crate::mix_of_sources(song);
    let cfg = HpssConfig::default();
    (hpss(&l, &cfg).percussive, hpss(&r, &cfg).percussive)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::Stem;

    #[test]
    fn the_mix_lane_signal_is_the_mid_of_the_mix_as_heard() {
        let frames = 5_000;
        let wave = |k: usize| -> Vec<f32> {
            (0..frames)
                .map(|i| ((i * k) % 23) as f32 / 23.0 - 0.5)
                .collect()
        };
        let song = Arc::new(Song {
            sample_rate: 44_100,
            frames,
            stems: ["bass", "lead"]
                .iter()
                .enumerate()
                .map(|(k, name)| Stem {
                    name: (*name).into(),
                    left: wave(k + 2).into(),
                    right: wave(k + 5).into(),
                })
                .collect(),
        });
        let bass = &song.stems[0];
        let want = mid(&bass.left, &bass.right);
        let no_lead = MixRecipe::new(vec![1.0, 0.0]);
        assert_eq!(lane_signal(&song, None, Some(&no_lead)), want);
        assert_eq!(lane_signal(&song, Some(0), Some(&no_lead)), want);
        let (l, r) = crate::mix_of_sources(&song);
        assert_eq!(lane_signal(&song, None, None), mid(&l, &r));
    }
}
