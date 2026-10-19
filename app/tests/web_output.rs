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
    inline_js = "export function sleep(ms) { return new Promise((r) => setTimeout(r, ms)); }
export function busy(ms) { const t = performance.now(); while (performance.now() - t < ms) {} }
export function in_chrome() { return navigator.userAgent.includes('Chrome/'); }"
)]
extern "C" {
    async fn sleep(ms: u32);
    /// Holds the page's thread for `ms`, as decoding a song does.
    fn busy(ms: u32);
    fn in_chrome() -> bool;
}

fn clock(app: &StemApp) -> std::time::Duration {
    match &app.output {
        Some(Output::Live(live)) => live.clock(),
        _ => panic!("no live output: {}", app.status),
    }
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

/// Needs a running audio context: `xtask web-test` lets Chrome play without
/// a click.  Elsewhere the context stays suspended, its clock stands still
/// and nothing can fall behind, so the test says so and stops.
#[wasm_bindgen_test]
async fn an_output_left_behind_by_a_busy_page_is_replaced() {
    let mut app = StemApp::default();
    app.open_song(song(44_100));
    sleep(300).await;
    if clock(&app).is_zero() {
        assert!(!in_chrome(), "web-test runs Chrome with audio allowed");
        console_log!("skipped: the audio context is suspended");
        return;
    }
    let opened = engine::live::opened();
    let first = Arc::downgrade(app.song.as_ref().expect("a song"));
    busy(600);
    app.open_song(song(44_100));
    assert_eq!(
        engine::live::opened(),
        opened + 1,
        "a new output, not one 600 ms behind"
    );
    assert!(first.upgrade().is_none(), "the first song is freed");

    sleep(300).await;
    assert!(
        !clock(&app).is_zero(),
        "the new output runs, so it can be on time"
    );
    app.open_song(song(44_100));
    assert_eq!(
        engine::live::opened(),
        opened + 1,
        "an output on time is kept"
    );
}

/// Needs a running audio context too (see above).  A page blocked for a
/// second leaves cpal's schedule a second behind, which it closes at one
/// second a second by calling back two buffers at once; one of each pair
/// is rendered, so the song goes on at its own speed, not doubled.
#[wasm_bindgen_test]
async fn buffers_left_late_by_a_busy_page_play_once_not_doubled() {
    let mut app = StemApp::default();
    app.open_song(song(44_100));
    sleep(300).await;
    if clock(&app).is_zero() {
        assert!(!in_chrome(), "web-test runs Chrome with audio allowed");
        console_log!("skipped: the audio context is suspended");
        return;
    }
    busy(1_000);
    let (before, from) = (callbacks(&app), clock(&app));
    sleep(500).await;
    let rendered = callbacks(&app) - before;
    let buffers = (clock(&app) - from).as_secs_f64() / (2_048.0 / 44_100.0);
    assert!(buffers > 5.0, "the clock ran for {buffers:.1} buffers");
    assert!(
        (buffers / 2.0..=buffers * 1.5).contains(&(rendered as f64)),
        "{rendered} rendered in {buffers:.1} buffers' time (twice as many doubled)"
    );
}

/// A page that keeps its thread busy for longer than two buffers at a time
/// (as a slow phone, or Chrome drawing without a GPU, can while a song
/// plays) leaves cpal's schedule further behind with every frame, so it
/// never catches up.  The song must still play then, not stay silent.
#[wasm_bindgen_test]
async fn a_page_too_busy_for_the_schedule_to_catch_up_still_plays() {
    let mut app = StemApp::default();
    app.open_song(song(44_100));
    sleep(300).await;
    if clock(&app).is_zero() {
        assert!(!in_chrome(), "web-test runs Chrome with audio allowed");
        console_log!("skipped: the audio context is suspended");
        return;
    }
    let frames = |n| async move {
        for _ in 0..n {
            busy(150);
            sleep(0).await;
        }
    };
    frames(10).await;
    let before = callbacks(&app);
    frames(10).await;
    let rendered = callbacks(&app) - before;
    // One of the two buffers called back each frame.
    assert!(
        (5..=13).contains(&rendered),
        "{rendered} buffers rendered in 1.5 s of 150 ms frames"
    );
}
