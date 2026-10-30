//! Opening a song frees the one it replaces.  Natively the tests never
//! touch audio hardware, so this is the silent output, which guards
//! `StemApp`'s output handling against regressions; the leak it replaced
//! was in cpal's Web Audio host, and `web_output.rs` checks that in a
//! browser.  `engine::slot`'s tests cover the swap itself.
#![cfg(not(target_arch = "wasm32"))]

use app::lanes::Shown;
use app::{Output, StemApp};
use engine::{Song, Stem};
use std::rc::Rc;
use std::time::Duration;

#[path = "common/web_buffers.rs"]
mod web_buffers;

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

fn silent_app() -> StemApp {
    let mut app = StemApp::default();
    app.want_device = false;
    app
}

#[test]
fn opening_a_song_frees_the_one_it_replaces() {
    let mut app = silent_app();
    app.open_memory_song(song(44_100));
    assert!(matches!(app.output, Some(Output::Silent(_))));
    let first = Rc::downgrade(app.song.as_ref().expect("a song"));

    app.open_memory_song(song(44_100));
    assert!(first.upgrade().is_none(), "the first song is freed");

    let second = Rc::downgrade(app.song.as_ref().expect("a song"));
    app.open_memory_song(song(48_000));
    assert!(second.upgrade().is_none(), "a song at another rate too");
    match &app.output {
        Some(Output::Silent(player)) => assert_eq!(player.sample_rate(), 48_000),
        _ => panic!("a silent output"),
    }
}

#[test]
fn reloading_the_shown_song_frees_the_previous_copy() {
    let mut app = silent_app();
    app.load_song(Shown::in_memory(song(44_100)));
    let before = Rc::downgrade(app.song.as_ref().expect("a song"));
    app.load_song(Shown::in_memory(song(44_100)));
    assert!(before.upgrade().is_none());
    assert!(app.song.is_some());
}

/// Review round 3 finding 3-2: in the 500 ms that
/// `buffers_left_late_by_a_busy_page_play_once_not_doubled` (`web_output.rs`)
/// measures, buffers of `engine::slot::WEB_BUFFER_FRAMES` frames play about
/// 7 times, at 44.1 kHz and at 48 kHz.  One of each late pair rendered is
/// in the band.  Both of each pair (about 14 or 16) is outside it.
#[test]
fn the_band_of_rendered_buffers_excludes_both_of_each_late_pair() {
    for (rate, want) in [(44_100, 7.18), (48_000, 7.81)] {
        let buffers = web_buffers::buffers_in(Duration::from_millis(500), rate);
        assert!(
            (buffers - want).abs() < 0.01,
            "{rate} Hz: {buffers} buffers"
        );
        let band = web_buffers::rendered_once(buffers);
        assert!(band.contains(&buffers), "{rate} Hz: once is in {band:?}");
        assert!(
            !band.contains(&(2.0 * buffers)),
            "{rate} Hz: doubled is outside {band:?}"
        );
    }
}
