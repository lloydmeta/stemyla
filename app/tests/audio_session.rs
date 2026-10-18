//! The page asks Safari for the "playback" audio session before the app
//! starts, so Silent Mode on an iPhone does not mute it.

const INDEX: &str = include_str!("../index.html");

#[test]
fn the_page_sets_the_playback_audio_session_before_the_app_loads() {
    let session = INDEX
        .find(r#"navigator.audioSession.type = "playback""#)
        .expect("index.html sets navigator.audioSession.type to playback");
    let app = INDEX
        .find(r#"data-bin="app""#)
        .expect("index.html loads the app");
    assert!(
        session < INDEX.find("<body>").expect("body"),
        "set in the head, so before any AudioContext"
    );
    assert!(app < INDEX.find("<body>").expect("body"));
    assert!(
        INDEX.contains(r#"addEventListener("pageshow", playback)"#),
        "set again when the page is shown from the back-forward cache"
    );
}
