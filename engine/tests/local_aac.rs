//! The local check of M2 of `plans/completed/m4a.md`, with the maintainer's
//! real AAC-LC file.  The file is copyrighted, so it is never in the repo
//! (ASSET-1), and CI does not run this test.
//!
//! Run it with the paths of the M4A and of Apple's decode of it:
//!
//! ```text
//! afconvert -f WAVE -d LEI16 <song.m4a> <scratch>/song.wav
//! STEMYLA_LOCAL_AAC=<song.m4a> STEMYLA_LOCAL_AAC_WAV=<scratch>/song.wav \
//!     cargo test -p engine --release --test local_aac -- --ignored
//! ```

use std::path::PathBuf;

/// The frames of the maintainer's file, as Apple's `afinfo` gives them.
const APPLE_FRAMES: usize = 13_413_120;
const RATE: u32 = 48_000;

fn path_of(name: &str) -> PathBuf {
    match std::env::var_os(name) {
        Some(path) => PathBuf::from(path),
        None => panic!("set {name} to the path of the file; see the doc of this test file"),
    }
}

/// The sum of the two channels of `channels`, for `count` frames from
/// `from`.
fn sum(channels: &[Vec<f32>], from: usize, count: usize) -> Vec<f64> {
    (from..from + count)
        .map(|i| f64::from(channels[0][i]) + f64::from(channels[1][i]))
        .collect()
}

/// The lag in `-reach..=reach` with the largest cross-correlation of
/// `ours` against `apple`, both from frame `from`: a lag of `d` means
/// that `ours[i + d]` matches `apple[i]`.
fn peak_lag(
    apple: &[Vec<f32>],
    ours: &[Vec<f32>],
    from: usize,
    count: usize,
    reach: isize,
) -> isize {
    let apple = sum(apple, from, count);
    let ours = sum(ours, from - reach as usize, count + 2 * reach as usize);
    let mut best = (0, f64::MIN);
    for lag in -reach..=reach {
        let start = (reach + lag) as usize;
        let value: f64 = apple
            .iter()
            .zip(&ours[start..start + count])
            .map(|(a, o)| a * o)
            .sum();
        if value > best.1 {
            best = (lag, value);
        }
    }
    best.0
}

#[test]
#[ignore = "local only: needs STEMYLA_LOCAL_AAC and STEMYLA_LOCAL_AAC_WAV"]
fn the_real_aac_file_decodes_to_apples_length_and_lines_up_with_it() {
    let ours = engine::decode_path(&path_of("STEMYLA_LOCAL_AAC"));
    assert!(ours.is_ok(), "{:?}", ours.err());
    let ours = ours.unwrap();
    let apple = engine::decode_path(&path_of("STEMYLA_LOCAL_AAC_WAV"));
    assert!(apple.is_ok(), "{:?}", apple.err());
    let apple = apple.unwrap();
    assert_eq!(ours.sample_rate, RATE);
    assert_eq!(ours.frames(), APPLE_FRAMES, "the frames of afinfo");
    assert_eq!(apple.frames(), APPLE_FRAMES, "the frames of Apple's decode");
    // 10 s from second 30.
    let from = 30 * RATE as usize;
    let lag = peak_lag(
        &apple.channels,
        &ours.channels,
        from,
        10 * RATE as usize,
        4096,
    );
    assert_eq!(lag, 0, "the lag of the decode against Apple's");
}
