//! Spectrogram textures for the lanes, and the classic (HPSS) drums job.

use crate::Lane;
use eframe::egui::{self, Color32, ColorImage, TextureOptions};
use engine::Song;
use engine::hpss::{HpssConfig, hpss, spectrogram_db};
use engine::mixdown::{Mix, MixRecipe, Pcm};
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
        let x = match (lane.stem, mix) {
            (Some(i), _) => mid(&song.stems[i].left, &song.stems[i].right),
            (None, Some(recipe)) => {
                let (mut l, mut r) = (vec![0.0; song.frames], vec![0.0; song.frames]);
                Mix::new(Arc::clone(song), recipe).read(0, &mut l, &mut r);
                mid(&l, &r)
            }
            (None, None) => {
                let (l, r) = crate::mix_of_sources(song);
                mid(&l, &r)
            }
        };
        let (db, cols, rows) = spectrogram_db(&x, song.sample_rate, MAX_HZ, ROWS, MAX_COLS);
        let pixels: Vec<Color32> = db.iter().map(|v| colour(*v)).collect();
        let img = ColorImage::new([cols, rows], pixels);
        lane.spectrogram =
            Some(ctx.load_texture(format!("spec-{}", lane.name), img, TextureOptions::LINEAR));
    }
}

/// HPSS on the song's mix (both channels): returns the percussive part.
pub fn classic_drums(song: &Song) -> (Vec<f32>, Vec<f32>) {
    let (l, r) = crate::mix_of_sources(song);
    let cfg = HpssConfig::default();
    (hpss(&l, &cfg).percussive, hpss(&r, &cfg).percussive)
}
