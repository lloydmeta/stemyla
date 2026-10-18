//! The page's Web Audio output, through cpal's Web Audio host, in a page of
//! its own: opening a song must free the one it replaces, although that
//! host never frees a stream's callback (`engine::live`).
#![cfg(all(target_arch = "wasm32", feature = "audio"))]

use app::{Output, StemApp};
use engine::{Song, Stem};
use std::sync::Arc;
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn song(rate: u32) -> Song {
    let frames = rate as usize / 10;
    let wave = vec![0.1; frames];
    Song {
        sample_rate: rate,
        frames,
        stems: vec![Stem {
            name: "lloyd".into(),
            left: wave.clone().into(),
            right: wave.into(),
        }],
    }
}

fn live_rate(app: &StemApp) -> Option<u32> {
    match &app.output {
        Some(Output::Live(live)) => Some(live.sample_rate),
        _ => None,
    }
}

fn callbacks(app: &StemApp) -> u64 {
    app.params
        .as_ref()
        .expect("params")
        .callbacks
        .load(std::sync::atomic::Ordering::Relaxed)
}

#[wasm_bindgen(
    inline_js = "export function sleep(ms) { return new Promise((r) => setTimeout(r, ms)); }"
)]
extern "C" {
    async fn sleep(ms: u32);
}

#[wasm_bindgen_test]
async fn opening_a_song_frees_the_one_it_replaces_on_the_same_output() {
    let mut app = StemApp::default();
    app.open_song(song(44_100));
    assert_eq!(live_rate(&app), Some(44_100), "{}", app.status);
    let opened = engine::live::opened();
    let first = Arc::downgrade(app.song.as_ref().expect("a song"));

    app.open_song(song(44_100));
    assert!(first.upgrade().is_none(), "the first song is freed");
    assert_eq!(engine::live::opened(), opened, "the same output plays it");
    assert_eq!(live_rate(&app), Some(44_100));
    // cpal calls back from timers at the start even while the context is
    // suspended (no click yet), so the kept output reaches the new player.
    sleep(300).await;
    assert!(callbacks(&app) > 0, "the kept output renders the new song");

    let second = Arc::downgrade(app.song.as_ref().expect("a song"));
    app.open_song(song(48_000));
    assert!(second.upgrade().is_none(), "the second song is freed");
    assert_eq!(
        engine::live::opened(),
        opened + 1,
        "a song at another rate gets an output at its rate"
    );
    assert_eq!(live_rate(&app), Some(48_000));

    let third = Arc::downgrade(app.song.as_ref().expect("a song"));
    app.song = None;
    app.output = None;
    assert!(
        third.upgrade().is_none(),
        "dropping the output frees its song"
    );
}
