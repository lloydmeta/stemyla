//! Stored lanes in a browser (`cargo run -p xtask -- web-test`): the reader
//! fills the read-ahead buffer from session files planted in browser
//! storage, while the test renders the player (D7 to D9 of
//! `plans/active/stream-from-storage-m6.md`).

use super::*;
use engine::source::{BLOCK_FRAMES, next_run};
use store_meta::LaneFile;
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

#[wasm_bindgen(inline_js = r#"
export function pb_test_sleep(ms) { return new Promise((r) => setTimeout(r, ms)); }
export function pb_test_now() { return performance.now(); }
// Each `arrayBuffer` of a Blob waits `ms` first, until `pb_undelay`.
let real_array_buffer = null;
export function pb_delay(ms) {
  real_array_buffer = Blob.prototype.arrayBuffer;
  Blob.prototype.arrayBuffer = function () {
    return new Promise((r) => setTimeout(r, ms)).then(() => real_array_buffer.call(this));
  };
}
export function pb_undelay() { Blob.prototype.arrayBuffer = real_array_buffer; }
// Records the size of each slice of a lane file (a `File` named *.wav or
// *.f32), with the file's name, until `pb_slices`.
let real_slice = null;
let slices = null;
export function pb_spy_slices() {
  real_slice = Blob.prototype.slice;
  slices = [];
  Blob.prototype.slice = function (start, end) {
    if (/\.(wav|f32)$/.test(this.name || "")) slices.push([this.name, (end ?? this.size) - (start ?? 0)]);
    return real_slice.apply(this, arguments);
  };
}
export function pb_slices() { Blob.prototype.slice = real_slice; return slices; }
"#)]
extern "C" {
    async fn pb_test_sleep(ms: u32);
    fn pb_test_now() -> f64;
    fn pb_delay(ms: u32);
    fn pb_undelay();
    fn pb_spy_slices();
    fn pb_slices() -> js_sys::Array;
}

const SECOND: usize = 44_100;
const NAMES: [&str; 6] = ["bass", "drums", "other", "vocals", "guitar", "piano"];
/// The callback of the page's Web Audio output: 2,048 frames each 46 ms.
const CALLBACK: usize = 2_048;

/// A tone for stem `k`, as the 16-bit samples that its WAV holds.
fn stem_wave(frames: usize, k: usize) -> Vec<f32> {
    let f = 0.0015 * (k + 1) as f32;
    (0..frames)
        .map(|i| {
            let v = 0.08 * (k + 1) as f32 * (f * i as f32).sin();
            (v * 32_768.0).round() / 32_768.0
        })
        .collect()
}

fn interleaved(left: &[f32], right: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 * left.len());
    for (l, r) in left.iter().zip(right) {
        out.extend(l.to_le_bytes());
        out.extend(r.to_le_bytes());
    }
    out
}

/// A planted session `sessions/<id>` of `stems` 16-bit stem WAVs of
/// `frames` and an "original" in `mix.f32`: the song in memory, and the
/// lane file of each lane.
async fn planted(id: &str, frames: usize, stems: usize) -> (Song, LaneFiles) {
    store::test_remove(&format!("sessions/{id}")).await;
    let folder = format!("sessions/{id}");
    let mut song_stems = Vec::new();
    let mut files = Vec::new();
    for (k, name) in NAMES.iter().take(stems).enumerate() {
        let left = stem_wave(frames, k);
        let right: Vec<f32> = left.iter().map(|v| -v).collect();
        let path = format!("{folder}/stems/{name}.wav");
        let wav = engine::wav::encode_pcm16(&left, &right, 44_100);
        store::test_write(&path, &wav).await.expect("planted");
        files.push(store_meta::stem_lane_file(
            &path,
            &wav[..44],
            wav.len() as u64,
            frames,
        ));
        song_stems.push(Stem {
            name: name.to_string(),
            left: left.into(),
            right: right.into(),
        });
    }
    let original: Vec<f32> = (0..frames)
        .map(|i| 0.3 * (0.0007 * i as f32).sin() + 1e-7 * (i % 7) as f32)
        .collect();
    let path = format!("{folder}/mix.f32");
    store::test_write(&path, &interleaved(&original, &original))
        .await
        .expect("planted");
    files.push(Some(LaneFile::f32(path, frames)));
    song_stems.push(Stem {
        name: "original".into(),
        left: original.clone().into(),
        right: original.into(),
    });
    let song = Song {
        sample_rate: 44_100,
        frames,
        stems: song_stems,
    };
    (song, files)
}

/// A page with no audio device, showing `song` from its lane files as
/// session `id`, and a player of the same song in memory with the page's
/// stretcher and the same mutes.
fn showing(id: &str, song: Song, files: LaneFiles) -> (StemApp, Player) {
    let mut app = StemApp {
        want_device: false,
        ..StemApp::default()
    };
    app.session = Some(id.to_string());
    let memory_song = Arc::new(song.clone());
    app.open_song(lanes::Shown::with_files(song, files));
    let params = Arc::new(SharedParams::new(memory_song.stems.len()));
    let stored = app.params.as_ref().expect("params");
    for i in 0..memory_song.stems.len() {
        params.set_muted(i, stored.is_muted(i));
    }
    let memory = Player::with_config(
        memory_song,
        params,
        app.stretcher.build(44_100),
        PlayerConfig::default(),
    );
    (app, memory)
}

fn stored_player(app: &mut StemApp) -> &mut Player {
    match &mut app.output {
        Some(Output::Silent(player)) => player,
        _ => panic!("a silent output"),
    }
}

/// Sets the same control on the page's player and the memory one.
fn both(app: &StemApp, memory: &Player, f: impl Fn(&SharedParams)) {
    f(app.params.as_ref().expect("params"));
    f(memory.params());
}

/// One callback on both players, after the 46 ms that the page's output
/// takes: whether they gave the same samples.
async fn callback(app: &mut StemApp, memory: &mut Player) -> bool {
    pb_test_sleep(46).await;
    render(app, memory)
}

/// One callback on both players now.
fn render(app: &mut StemApp, memory: &mut Player) -> bool {
    let (mut l, mut r) = (vec![0.0; CALLBACK], vec![0.0; CALLBACK]);
    stored_player(app).process(&mut l, &mut r);
    let [ml, mr] = memory.render(CALLBACK);
    l == ml && r == mr
}

fn ahead(app: &StemApp) -> &engine::readahead::ReadAhead {
    &app.playback.as_ref().expect("stored lanes").ahead
}

/// Waits until the read-ahead buffer holds every block of `frames`.
async fn until_held(app: &StemApp, frames: &[u64]) {
    for _ in 0..500 {
        if frames.iter().all(|f| ahead(app).holds(f / BLOCK_FRAMES)) {
            return;
        }
        pb_test_sleep(10).await;
    }
    panic!("the reader did not read the blocks of {frames:?}");
}

#[wasm_bindgen_test]
async fn a_stored_song_plays_as_the_memory_song_in_the_browser() {
    let id = "m6-plays";
    let frames = 30 * SECOND;
    let (song, files) = planted(id, frames, 6).await;
    let (mut app, mut memory) = showing(id, song, files);
    let playback = app.playback.as_ref().expect("stored lanes");
    assert_eq!((playback.stored, playback.lanes), (7, 7));
    // The first run is read before Play, so nothing holds.
    until_held(&app, &[0]).await;
    both(&app, &memory, |p| p.playing.store(true, Relaxed));
    let mut same = Vec::new();
    for step in 0..174 {
        match step {
            40 => both(&app, &memory, |p| p.set_tempo(1.5)),
            70 => both(&app, &memory, |p| p.set_tempo(1.0)),
            80 => both(&app, &memory, |p| p.chipmunk.store(true, Relaxed)),
            100 => both(&app, &memory, |p| p.chipmunk.store(false, Relaxed)),
            // "original" is the last lane, muted as a derived lane.
            105 => both(&app, &memory, |p| p.set_muted(6, false)),
            135 => {
                both(&app, &memory, |p| p.set_muted(6, true));
                // A loop whose start and end lie in blocks that are read.
                let head = app.params.as_ref().expect("params").playhead();
                let (s, e) = (head.saturating_sub(10_000), head + 20_000);
                both(&app, &memory, |p| p.set_loop(s, e));
                until_held(&app, &[s, e, e + 256]).await;
            }
            _ => {}
        }
        same.push(callback(&mut app, &mut memory).await);
    }
    let underruns = ahead(&app).underruns();
    let held = app
        .params
        .as_ref()
        .expect("params")
        .held_frames
        .load(Relaxed);
    let reads = app
        .playback
        .as_ref()
        .expect("stored lanes")
        .stats
        .borrow()
        .reads;
    log::info!("playback test: 174 callbacks, reads {reads}, underruns {underruns}");
    store::test_remove(&format!("sessions/{id}")).await;
    let first_different = same.iter().position(|s| !s);
    assert_eq!(
        first_different, None,
        "each callback equals the memory song"
    );
    assert_eq!(underruns, 0);
    assert_eq!(held, 0);
    assert!(reads > 0);
}

#[wasm_bindgen_test]
async fn a_seek_in_the_browser_plays_the_old_position_until_its_block_arrives() {
    let id = "m6-seek";
    let frames = 30 * SECOND;
    let (song, files) = planted(id, frames, 6).await;
    let (mut app, mut memory) = showing(id, song, files);
    until_held(&app, &[0]).await;
    both(&app, &memory, |p| p.playing.store(true, Relaxed));
    for _ in 0..20 {
        assert!(callback(&mut app, &mut memory).await);
    }
    pb_delay(200);
    let target = 25 * SECOND as u64 + 77;
    app.params.as_ref().expect("params").request_seek(target);
    let asked = pb_test_now();
    let reads_before = app.playback.as_ref().expect("lanes").stats.borrow().reads;
    let run = next_run(target, frames as u64, None, 256);
    let mut seeked = None;
    let mut same = true;
    for _ in 0..60 {
        pb_test_sleep(46).await;
        let ready = run.parts().all(|(s, e)| {
            (s / BLOCK_FRAMES..e.div_ceil(BLOCK_FRAMES)).all(|b| ahead(&app).holds(b))
        });
        // The memory player seeks in the callback where the stored run is
        // ready: until then, the old position plays.
        if ready && seeked.is_none() {
            memory.params().request_seek(target);
            seeked = Some(pb_test_now() - asked);
        }
        same &= render(&mut app, &mut memory);
    }
    pb_undelay();
    let head = app.params.as_ref().expect("params").playhead();
    let underruns = ahead(&app).underruns();
    let reads = app.playback.as_ref().expect("lanes").stats.borrow().reads - reads_before;
    let seek_ms = seeked.unwrap_or(f64::NAN);
    log::info!(
        "playback test: seek to sound {seek_ms:.0} ms with reads delayed 200 ms, {reads} reads"
    );
    store::test_remove(&format!("sessions/{id}")).await;
    assert!(seeked.is_some(), "the target's run was read");
    assert!(same, "the old position, then the target, as in memory");
    assert!(head > target, "the playhead passed the target: {head}");
    assert_eq!(underruns, 0);
}

#[wasm_bindgen_test]
async fn a_lane_file_is_read_one_block_at_a_time() {
    let id = "m6-blocks";
    let frames = 60 * SECOND;
    let (song, files) = planted(id, frames, 1).await;
    let mut app = StemApp {
        want_device: false,
        ..StemApp::default()
    };
    app.session = Some(id.to_string());
    pb_spy_slices();
    app.open_song(lanes::Shown::with_files(song, files));
    app.params
        .as_ref()
        .expect("params")
        .playing
        .store(true, Relaxed);
    for _ in 0..20 {
        pb_test_sleep(46).await;
        let (mut l, mut r) = (vec![0.0; CALLBACK], vec![0.0; CALLBACK]);
        stored_player(&mut app).process(&mut l, &mut r);
    }
    let slices = pb_slices();
    store::test_remove(&format!("sessions/{id}")).await;
    let sizes: Vec<(String, f64)> = slices
        .iter()
        .map(|s| {
            let s: js_sys::Array = s.into();
            (
                s.get(0).as_string().unwrap_or_default(),
                s.get(1).as_f64().unwrap_or(0.0),
            )
        })
        .collect();
    assert!(sizes.len() >= 10, "{} reads", sizes.len());
    for (name, size) in &sizes {
        let block = if name.ends_with(".wav") { 128 } else { 256 } * 1024;
        assert!(*size <= block as f64, "{name}: {size} bytes");
    }
}

#[wasm_bindgen_test]
async fn a_removed_lane_file_goes_silent_and_says_so() {
    let id = "m6-removed";
    let frames = 30 * SECOND;
    let (song, files) = planted(id, frames, 2).await;
    let (mut app, mut memory) = showing(id, song, files);
    until_held(&app, &[0]).await;
    both(&app, &memory, |p| p.playing.store(true, Relaxed));
    for _ in 0..5 {
        callback(&mut app, &mut memory).await;
    }
    assert!(store::test_remove(&format!("sessions/{id}/stems/drums.wav")).await);
    // Play on until the reader reaches a block it has not read yet.
    for _ in 0..100 {
        if ahead(&app).failed_lanes() > 0 {
            break;
        }
        callback(&mut app, &mut memory).await;
    }
    let mut heard = false;
    for _ in 0..10 {
        pb_test_sleep(46).await;
        let (mut l, mut r) = (vec![0.0; CALLBACK], vec![0.0; CALLBACK]);
        stored_player(&mut app).process(&mut l, &mut r);
        heard |= l.iter().any(|v| *v != 0.0);
    }
    let failed = ahead(&app).failed_lanes();
    let drums_failed = ahead(&app).is_failed(1);
    let head = app.params.as_ref().expect("params").playhead();
    let (mut l, mut r) = ([9.0; 64], [9.0; 64]);
    let ahead_arc = Arc::clone(&app.playback.as_ref().expect("lanes").ahead);
    ahead_arc.sources()[1].read(head, &mut l, &mut r);
    let underruns = ahead(&app).underruns();
    let status = app.pending.borrow().status.clone();
    store::test_remove(&format!("sessions/{id}")).await;
    assert_eq!(failed, 1);
    assert!(drums_failed, "the drums lane failed");
    assert_eq!(l, [0.0; 64], "it gives silence");
    assert!(heard, "the other lanes play");
    assert_eq!(underruns, 0);
    assert_eq!(
        status.as_deref(),
        Some("Could not read the drums lane from browser storage; it plays silent.")
    );
}

#[wasm_bindgen_test]
async fn a_new_song_frees_the_old_read_ahead() {
    let id = "m6-freed";
    let (song, files) = planted(id, 4 * SECOND, 1).await;
    let (mut app, _memory) = showing(id, song.clone(), files.clone());
    let first = app.playback.as_ref().expect("stored lanes").watch();
    pb_test_sleep(50).await;
    assert!(first.reading(), "the reader runs");
    // Other tests can leave a buffer alive: count from those alive now.
    let alive = engine::readahead::ReadAhead::live();
    engine::readahead::ReadAhead::take_high_water();
    app.load_song(lanes::Shown::with_files(song, files));
    // Review round 1 of M6, finding 1-6: the old buffer goes before the
    // new one is made, so two never live at once.
    let most = engine::readahead::ReadAhead::take_high_water();
    pb_test_sleep(50).await;
    let second = app.playback.as_ref().expect("stored lanes").watch();
    let (gone, stopped) = (first.ahead.upgrade().is_none(), !first.reading());
    app.open_memory_song(Song {
        sample_rate: 44_100,
        frames: 4,
        stems: vec![Stem {
            name: "lloyd".into(),
            left: vec![0.1; 4].into(),
            right: vec![0.1; 4].into(),
        }],
    });
    pb_test_sleep(50).await;
    let none_left = second.ahead.upgrade().is_none() && !second.reading();
    let protected = store::protected(id);
    store::test_remove(&format!("sessions/{id}")).await;
    assert_eq!(most, alive, "one read-ahead buffer at a time");
    assert!(gone, "no strong reference to the first buffer");
    assert!(stopped, "its reader stopped");
    assert!(none_left, "a song from memory frees the second");
    assert!(!protected, "the session is no longer protected");
}

#[wasm_bindgen_test]
async fn a_restored_session_plays_its_lanes_from_storage() {
    store::test_wipe().await;
    let frames = 2 * SECOND;
    // A complete session of two sources with stems, as M5 and the worker
    // leave it: the sources' files, `mix.f32` and the stems.
    let wav = engine::wav::encode_pcm16(&stem_wave(frames, 0), &stem_wave(frames, 1), 44_100);
    let files = [
        ("bass.wav".to_string(), dt_blob(&wav)),
        ("drums.wav".to_string(), dt_blob(&wav)),
    ];
    let id = store::save_sources(&files, 44_100).await.expect("saved");
    let folder = store::session_folder(&id);
    let source = |k: usize| (stem_wave(frames, k), stem_wave(frames, k + 3));
    let sources = [source(0), source(1)];
    // The meta's order is that of `Song::from_audio`: drums, then bass.
    let mut meta = store::list()
        .await
        .into_iter()
        .find(|(got, _)| *got == id)
        .map(|(_, meta)| meta)
        .expect("meta");
    let planned = store_meta::planned_sources(&meta).expect("planned");
    for (s, (left, right)) in planned.iter().zip(&sources) {
        let path = format!("{folder}/{}", store_meta::source_file(s, false));
        store::test_write(&path, &interleaved(left, right))
            .await
            .expect("planted");
    }
    let mix_left: Vec<f32> = (0..frames)
        .map(|i| sources[0].0[i] + sources[1].0[i])
        .collect();
    let mix_right: Vec<f32> = (0..frames)
        .map(|i| sources[0].1[i] + sources[1].1[i])
        .collect();
    store::test_write(&store::mix_path(&id), &interleaved(&mix_left, &mix_right))
        .await
        .expect("planted");
    let columns = engine::overview::column_count(frames);
    for name in ["0", "1", "mix"] {
        let path = format!("{folder}/overviews/{name}.f32");
        store::test_write(&path, &vec![0; 8 * columns])
            .await
            .expect("planted");
    }
    for (k, name) in NAMES.iter().enumerate() {
        let wave = stem_wave(frames, k);
        let path = format!("{folder}/stems/{}", stem_protocol::stem_file(name));
        let wav = engine::wav::encode_pcm16(&wave, &wave, 44_100);
        store::test_write(&path, &wav).await.expect("planted");
    }
    // A stem kept as a `.part` file plays from memory.
    let part = format!("{folder}/stems/bass.wav.part");
    store::test_write(&part, &wav).await.expect("planted");
    meta.stems = NAMES.iter().map(|n| n.to_string()).collect();
    meta.mix = Some((frames, store_meta::MIX_FORMAT.into()));
    meta.decoded = Some(store_meta::Decoded {
        frames,
        format: store_meta::MIX_FORMAT.into(),
        channels: vec![(0, 2), (1, 2)],
    });
    store::test_write(&format!("{folder}/meta"), meta.to_text().as_bytes())
        .await
        .expect("planted");
    let decoder = decoder::DecodeClient::new("./no-such-decode-worker.js");
    let (opened, _, note) = restore_stored(&id, &decoder)
        .await
        .expect("a session")
        .expect("restored");
    let names: Vec<String> = opened.song.names().into_iter().map(String::from).collect();
    let all_stored = opened.song.files().iter().all(Option::is_some);
    let original_file = opened.song.files().last().cloned().flatten();
    let original = opened.song.as_song().stems.pop().expect("an original");
    let (_, part_file) = store::read_stem(&part, "bass", frames).await.expect("read");
    let mut app = StemApp {
        want_device: false,
        ..StemApp::default()
    };
    app.session = Some(id.clone());
    app.open_song(opened.song);
    let playback = app.playback.as_ref().map(|p| (p.stored, p.lanes));
    // Classic drums, made on the page, play from memory.
    app.add_stem("classic drums", vec![0.0; frames], vec![0.0; frames]);
    let with_drums = app.lane_files();
    // Stems held for an open play from memory; "original" keeps its file
    // (review round 2 of M6, finding 2-11).
    let (again, _, _) = restore_stored(&id, &decoder)
        .await
        .expect("a session")
        .expect("restored");
    let held = vec![
        Stem {
            name: "vocals".into(),
            left: vec![0.1; frames].into(),
            right: vec![0.1; frames].into(),
        },
        Stem {
            name: "bass".into(),
            left: vec![0.2; frames].into(),
            right: vec![0.2; frames].into(),
        },
    ];
    app.stems_for_opening = Some((id.clone(), held, "kept".into()));
    app.pending.borrow_mut().song = Some(Ok(again));
    app.apply_inbox(&egui::Context::default());
    let for_opening = app.lane_files();
    // A song decoded on the page has no lane files.
    let on_the_page = Opened::in_memory(
        Song {
            sample_rate: 44_100,
            frames: 4,
            stems: vec![Stem {
                name: "lloyd".into(),
                left: vec![0.1; 4].into(),
                right: vec![0.1; 4].into(),
            }],
        },
        Some(id.clone()),
        None,
    );
    let on_the_page_files = on_the_page.song.files();
    // The page decodes a song itself when storage fails, and shows it
    // from memory (review round 3 of M6, finding 3-9).
    let chosen = vec![("lloyd.wav".to_string(), pb_file(&wav, "lloyd.wav"))];
    let decoded = open_on_the_page(
        &chosen,
        Some(id.clone()),
        None,
        0.0,
        Some("the test".into()),
    )
    .await
    .expect("decoded on the page");
    app.pending.borrow_mut().song = Some(Ok(decoded));
    app.apply_inbox(&egui::Context::default());
    let page_decode = (
        app.lane_files().iter().all(Option::is_none),
        app.playback.is_none(),
    );
    // The demo plays from memory.
    app.load_demo();
    let demo = (
        app.lane_files().iter().all(Option::is_none),
        app.playback.is_none(),
    );
    store::test_wipe().await;
    assert_eq!(note, None, "read back, not decoded on the page");
    assert_eq!(names.len(), 7, "{names:?}");
    assert_eq!(names.last().map(String::as_str), Some("original"));
    assert!(all_stored, "every lane plays from storage");
    assert_eq!(
        original_file,
        Some(LaneFile::f32(store::mix_path(&id), frames))
    );
    // The memory "original" of two sources, `mix_of_sources`, equals what
    // `mix.f32` holds, which the stored lane plays.
    assert_eq!(*original.left, mix_left[..]);
    assert_eq!(*original.right, mix_right[..]);
    assert_eq!(playback, Some((7, 7)));
    assert_eq!(with_drums.len(), 8);
    assert_eq!(with_drums.last(), Some(&None), "classic drums from memory");
    assert!(
        with_drums[..7].iter().all(Option::is_some),
        "the others keep their files"
    );
    assert_eq!(part_file, None, "a .part stem plays from memory");
    assert_eq!(
        for_opening,
        [
            None,
            None,
            Some(LaneFile::f32(store::mix_path(&id), frames))
        ],
        "stems held for an open from memory, \"original\" from mix.f32"
    );
    assert_eq!(on_the_page_files, [None], "a song decoded on the page");
    assert_eq!(
        page_decode,
        (true, true),
        "shown from memory after a page decode"
    );
    assert_eq!(demo, (true, true));
}

/// D1 of the M7 plan: the shown song's session stays protected while a
/// lane plays from a session file, also when its lanes do not fit the
/// read-ahead buffer and no buffer is made.  A song with no lane file
/// protects nothing.
#[wasm_bindgen_test]
fn a_song_with_stored_lanes_protects_its_session_with_no_read_ahead_buffer() {
    let id = "m7-protected";
    // 22 f32 lanes need more than 32 MiB at 4 and 2 slots.
    let lanes = 22;
    let frames = 4;
    let song = || Song {
        sample_rate: 44_100,
        frames,
        stems: (0..lanes)
            .map(|k| Stem {
                name: format!("acme {k}"),
                left: vec![0.1; frames].into(),
                right: vec![0.1; frames].into(),
            })
            .collect(),
    };
    let files: LaneFiles = (0..lanes)
        .map(|k| Some(LaneFile::f32(format!("{id}/decoded/{k}.f32"), frames)))
        .collect();
    let mut app = StemApp {
        want_device: false,
        ..StemApp::default()
    };
    app.session = Some(id.to_string());
    app.open_song(lanes::Shown::with_files(song(), files));
    let no_buffer = app.playback.is_none();
    let with_files = store::protected(id);
    app.open_song(lanes::Shown::in_memory(song()));
    let in_memory = store::protected(id);
    app.load_demo();
    assert!(no_buffer, "the lanes do not fit the read-ahead buffer");
    assert!(with_files, "the session of stored lanes is protected");
    assert!(!in_memory, "a song in memory protects nothing");
}

#[wasm_bindgen(inline_js = r#"
export function dt_blob(bytes) { return new Blob([bytes]); }
export function pb_file(bytes, name) { return new File([bytes], name); }
"#)]
extern "C" {
    fn dt_blob(bytes: &[u8]) -> web_sys::Blob;
    fn pb_file(bytes: &[u8], name: &str) -> web_sys::File;
}
