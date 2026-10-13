//! M1 acceptance: stems sum to the mix within 1e-6, and the WAV files decode
//! back with Symphonia to the expected length.

use xtask::synth;

#[test]
fn stems_sum_to_mix_and_decode_back() {
    let song = synth::generate();
    let frames = song.truth.frames;
    assert_eq!(frames, 30 * 44_100);

    let dir = std::env::temp_dir().join(format!("stems-m1-{}", std::process::id()));
    synth::write_song(&song, &dir, false).unwrap();

    let mix = engine::decode_path(&dir.join("mix.wav")).unwrap();
    assert_eq!(mix.sample_rate, 44_100);
    assert_eq!(mix.channels.len(), 2);
    assert_eq!(mix.frames(), frames);

    let mut sum = vec![vec![0.0f32; frames]; 2];
    for name in synth::STEM_NAMES {
        let a = engine::decode_path(&dir.join(format!("{name}.wav"))).unwrap();
        assert_eq!(a.frames(), frames, "{name} length");
        for c in 0..2 {
            for i in 0..frames {
                sum[c][i] += a.channels[c][i];
            }
        }
    }
    let mut worst = 0.0f32;
    for c in 0..2 {
        for i in 0..frames {
            worst = worst.max((sum[c][i] - mix.channels[c][i]).abs());
        }
    }
    assert!(worst < 1e-6, "stems differ from mix by {worst}");

    // Peak below 1, every stem audible.
    let peak = mix.channels[0].iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(peak <= 0.9001 && peak > 0.5, "mix peak {peak}");
    for (name, s) in &song.stems {
        let rms = (s.left.iter().map(|v| (v * v) as f64).sum::<f64>() / frames as f64).sqrt();
        assert!(rms > 0.005, "{name} rms {rms}");
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn mp3_decodes_to_roughly_the_right_length() {
    // Only meaningful when gen-song has been run with ffmpeg available.
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../audio/synth/mix.mp3");
    if !p.exists() {
        eprintln!("skipping: {} not generated", p.display());
        return;
    }
    let a = engine::decode_path(&p).unwrap();
    let expected = 30 * 44_100;
    // MP3 adds encoder delay and padding (a few thousand frames at most).
    assert!(
        a.frames() >= expected && a.frames() < expected + 4_000,
        "mp3 decoded to {} frames",
        a.frames()
    );
}
