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
