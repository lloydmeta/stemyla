//! M3 acceptance: engine core against the synthetic song, offline.

use engine::stretch::Varispeed;
use engine::{Player, PlayerConfig, SharedParams, Song, Stem};
use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;
use xtask::metrics::{best_lag, max_jump, median_jump, sdr};
use xtask::synth;

const SR: usize = 44_100;
const CALLBACK: usize = 512;

pub fn synth_song() -> (Arc<Song>, synth::Song) {
    let s = synth::generate();
    let song = Song {
        sample_rate: synth::SAMPLE_RATE,
        frames: s.truth.frames,
        stems: s
            .stems
            .iter()
            .map(|(n, st)| Stem {
                name: n.clone(),
                left: st.left.clone().into(),
                right: st.right.clone().into(),
            })
            .collect(),
    };
    (Arc::new(song), s)
}

fn varispeed_player(song: &Arc<Song>, crossfade: usize) -> Player {
    let params = Arc::new(SharedParams::new(song.stems.len()));
    Player::with_config(
        Arc::clone(song),
        params,
        Box::new(Varispeed::new()),
        PlayerConfig {
            crossfade,
            ..Default::default()
        },
    )
}

/// Renders in device-sized callbacks, like a real audio thread.
fn render(p: &mut Player, frames: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(frames);
    let mut l = vec![0.0; CALLBACK];
    let mut r = vec![0.0; CALLBACK];
    while out.len() < frames {
        let n = CALLBACK.min(frames - out.len());
        p.process(&mut l[..n], &mut r[..n]);
        out.extend_from_slice(&l[..n]);
    }
    out
}

#[test]
fn sync_unmuting_drums_mid_render_has_zero_lag() {
    let (song, truth) = synth_song();
    let drums = song.stem_index("drums").unwrap();
    let guitar = song.stem_index("guitar").unwrap();
    let mut p = varispeed_player(&song, 256);
    let params = Arc::clone(p.params());
    params.set_soloed(guitar, true);
    params.playing.store(true, Relaxed);

    let first = 4 * SR;
    let second = 6 * SR;
    let a = render(&mut p, first);
    params.set_soloed(drums, true); // drums join the guitar
    let b = render(&mut p, second);

    // First half: guitar only.
    let g = &truth.stems[guitar].1.left;
    // Gain changes reach input that has not been read yet, so allow for the
    // read-ahead (one hop plus what sits inside the stretcher).
    let ramp = 1024;
    assert!(
        sdr(&g[ramp..first], &a[ramp..]) > 60.0,
        "first half is not the guitar alone"
    );

    // Second half: subtract the guitar, what is left must be the drums,
    // aligned to the sample.  Skip the gain ramp.
    let skip = ramp;
    let residual: Vec<f32> = (skip..second).map(|i| b[i] - g[first + i]).collect();
    let d = &truth.stems[drums].1.left[first + skip..first + second];
    let lag = best_lag(&residual[..SR], &d[..SR], 200);
    let quality = sdr(d, &residual);
    println!("sync: lag {lag} samples, drums SDR {quality:.1} dB");
    assert_eq!(lag, 0);
    assert!(quality > 60.0, "drums SDR {quality}");
}

/// Loops a region of the sustained pad whose length is not a whole number
/// of periods, so a hard jump lands mid-cycle.  Returns (max jump at the
/// wraps, median jump everywhere).
fn loop_jumps(crossfade: usize) -> (f32, f32) {
    let (song, _) = synth_song();
    let keys = song.stem_index("keys").unwrap();
    let mut p = varispeed_player(&song, crossfade);
    let params = Arc::clone(p.params());
    params.set_soloed(keys, true);
    let start = 5 * SR;
    let len = (0.3737 * SR as f64) as usize;
    params.set_loop(start as u64, (start + len) as u64);
    params.request_seek(start as u64); // paused: immediate
    p.render(1);
    params.playing.store(true, Relaxed);
    let out = render(&mut p, 3 * len + 1000);
    // Output frame j is song frame start + j until the first wrap at j = len.
    let mut worst = 0.0f32;
    for w in 1..=3 {
        let j = w * len;
        worst = worst.max(max_jump(&out, j - 8, j + 8));
    }
    (worst, median_jump(&out))
}

#[test]
fn loop_crossfade_removes_the_click() {
    let (with_xf, median) = loop_jumps(256);
    let (without_xf, median0) = loop_jumps(0);
    println!(
        "loop: boundary jump with crossfade {with_xf:.5} (median {median:.5}, ratio {:.2}); \
         without {without_xf:.5} (median {median0:.5}, ratio {:.2})",
        with_xf / median,
        without_xf / median0
    );
    assert!(
        with_xf < 3.0 * median,
        "crossfaded jump {with_xf} vs median {median}"
    );
    assert!(without_xf > 3.0 * median0, "hard jump should click");
    assert!(without_xf > 3.0 * with_xf);
}

#[test]
fn mute_and_solo_change_what_is_heard() {
    let (song, truth) = synth_song();
    let drums = song.stem_index("drums").unwrap();
    let bass = song.stem_index("bass").unwrap();
    let mut p = varispeed_player(&song, 256);
    let params = Arc::clone(p.params());
    params.playing.store(true, Relaxed);
    params.set_muted(drums, true);
    let n = 3 * SR;
    let out = render(&mut p, n);
    // Expected: every stem except drums (the ramp from the initial gain is
    // instantaneous because the player starts with settled gains).
    let others: Vec<f32> = (0..n)
        .map(|i| {
            truth
                .stems
                .iter()
                .enumerate()
                .filter(|(k, _)| *k != drums)
                .map(|(_, (_, s))| s.left[i])
                .sum()
        })
        .collect();
    assert!(sdr(&others[1024..], &out[1024..]) > 60.0);

    // Solo bass: only bass, even though drums are also muted.
    params.set_soloed(bass, true);
    let out2 = render(&mut p, SR);
    let b = &truth.stems[bass].1.left[n + 1024..n + SR];
    assert!(sdr(b, &out2[1024..]) > 60.0);
    assert_eq!(params.effective_gain(drums), 0.0);
    assert_eq!(
        params.effective_gain(song.stem_index("guitar").unwrap()),
        0.0
    ); // guitar silenced by the solo
}

#[test]
fn seek_is_accurate_to_one_block_and_audio_follows() {
    let (song, truth) = synth_song();
    let mut p = varispeed_player(&song, 256);
    let params = Arc::clone(p.params());
    params.playing.store(true, Relaxed);
    render(&mut p, SR);
    let target = (12.345 * SR as f64) as usize;
    params.request_seek(target as u64);
    // Fade-out (256) then fade-in; one callback later the head is past
    // the target by less than one callback.
    let out = render(&mut p, CALLBACK);
    let head = params.playhead() as usize;
    println!("seek: target {target}, playhead {head}");
    assert!(
        head >= target && head <= target + CALLBACK,
        "playhead {head}"
    );
    // Audio after the fade-in matches the mix at the target.
    let more = render(&mut p, SR);
    let mix = &truth.mix.left;
    let start = target + (CALLBACK - 256);
    assert!(sdr(&mix[start..start + SR], &more) > 60.0);
    let _ = out;
}

#[test]
fn folder_loading_matches_the_generator() {
    let (song, generated) = synth_song();
    let dir = std::env::temp_dir().join(format!("stems-m3-folder-{}", std::process::id()));
    let loaded = synth::write_song(&generated, &dir, false).and_then(|()| Song::load_folder(&dir));
    std::fs::remove_dir_all(&dir).ok();
    let loaded = loaded.unwrap();
    let names: Vec<_> = loaded.stems.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["drums", "bass", "guitar", "keys", "lead"]);
    assert_eq!(loaded.frames, song.frames);
    assert_eq!(loaded.stems[0].left, song.stems[0].left);
    let (ml, _) = loaded.mix_lane();
    assert_eq!(ml.len(), song.frames);
}

/// Decision 0012: every song opens at the model's rate, so a separation
/// never needs another conversion.
#[test]
fn the_song_rate_is_the_model_rate() {
    assert_eq!(engine::resample::SONG_RATE, roformer::SAMPLE_RATE);
}
