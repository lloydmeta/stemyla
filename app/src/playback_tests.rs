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

/// A store of browser storage, for lanes that a test plants.
fn opfs() -> std::rc::Rc<dyn lane_store::LaneStore> {
    std::rc::Rc::new(lane_store::OpfsStore::new(None))
}
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
    app.open_song(lanes::Shown::with_files(song, files, &opfs()));
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
    app.open_song(lanes::Shown::with_files(song, files, &opfs()));
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
    app.load_song(lanes::Shown::with_files(song, files, &opfs()));
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
    // D11 of the M7 plan: the original is a stored lane; the lane reader
    // reads its samples.
    let original_lane = opened.song.lanes.last().expect("an original").audio.clone();
    let original = run_export(export::ExportJob::new(
        Box::new(lanes::LanePcm::new(original_lane, frames, 44_100)),
        export::VecSink::new(frames),
    ))
    .await;
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
    let held_mix = std::rc::Rc::new(lanes::Overview {
        columns: vec![(-0.25, 0.5); 3],
        scale: 2.0,
    });
    app.stems_for_opening = Some(crate::HeldStems {
        session: id.clone(),
        stems: memory_lanes(held),
        mix_overview: Some(held_mix.clone()),
        kept: "kept".into(),
        not_kept: None,
    });
    app.pending.borrow_mut().song = Some((app.opens.get(), Ok(again)));
    app.apply_inbox(&egui::Context::default());
    let for_opening = app.lane_files();
    // Finding 1-28 of review round 1 of M7: the held stems' mix overview.
    let shown_mix = app.song.as_ref().and_then(|s| s.mix_overview.clone());
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
    app.pending.borrow_mut().song = Some((app.opens.get(), Ok(decoded)));
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
    assert_eq!(original.left, mix_left);
    assert_eq!(original.right, mix_right);
    assert_eq!(playback, Some((7, 7)));
    assert_eq!(with_drums.len(), 8);
    assert_eq!(with_drums.last(), Some(&None), "classic drums from memory");
    assert!(
        with_drums[..7].iter().all(Option::is_some),
        "the others keep their files"
    );
    assert_eq!(part_file, None, "a .part stem plays from memory");
    assert_eq!(shown_mix, Some(held_mix), "the held stems' mix overview");
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
    app.open_song(lanes::Shown::with_files(song(), files, &opfs()));
    let no_buffer = app.playback.is_none();
    let with_files = store::protected(id);
    app.open_song(lanes::Shown::in_memory(song()));
    let in_memory = store::protected(id);
    app.load_demo();
    assert!(no_buffer, "the lanes do not fit the read-ahead buffer");
    assert!(with_files, "the session of stored lanes is protected");
    assert!(!in_memory, "a song in memory protects nothing");
}

/// A lane store that counts the requests of the jobs and their largest
/// range, around the store of browser storage.
struct Counting {
    inner: lane_store::OpfsStore,
    requests: std::cell::Cell<usize>,
    largest: std::cell::Cell<u64>,
}

impl lane_store::LaneStore for Counting {
    fn read(&self, ranges: Vec<lane_store::ByteRange>, done: lane_store::ReadDone) {
        self.requests.set(self.requests.get() + 1);
        let largest = ranges.iter().map(|r| r.end - r.start).max().unwrap_or(0);
        self.largest.set(self.largest.get().max(largest));
        self.inner.read(ranges, done);
    }
}

fn counting() -> std::rc::Rc<Counting> {
    std::rc::Rc::new(Counting {
        inner: lane_store::OpfsStore::new(None),
        requests: std::cell::Cell::new(0),
        largest: std::cell::Cell::new(0),
    })
}

/// A page with no audio device that shows `song` in memory.
fn memory_page(song: Song) -> StemApp {
    let mut app = StemApp {
        want_device: false,
        lane_store: None,
        ..StemApp::default()
    };
    app.open_memory_song(song);
    app
}

/// Steps the mix follower in frames until it shows the mix as heard.
async fn follow_to_the_end(app: &mut StemApp) {
    let ctx = egui::Context::default();
    for _ in 0..5_000 {
        app.follow_mix_while(&ctx, || true);
        if app
            .mix_follower
            .as_ref()
            .is_some_and(|f| f.progress().is_none())
        {
            return;
        }
        pb_test_sleep(1).await;
    }
    panic!("the mix lane never showed the new mix");
}

/// Runs a save job in frames, waiting for its reads.
async fn run_export<S: export::Sink>(mut job: export::ExportJob<S>) -> S {
    while !job.run_while(|| true) {
        pb_test_sleep(1).await;
    }
    job.into_sink()
}

/// The image of a spectrogram job over `pcm`, run in frames.
async fn run_spectrogram(pcm: Box<dyn engine::mixdown::Pcm>) -> Vec<u32> {
    let mut job = spectrogram::SpectrogramJob::new(pcm);
    while !job.run_while(|| true) {
        pb_test_sleep(1).await;
    }
    job.finish().0.iter().map(|v| v.to_bits()).collect()
}

/// D3 of the M7 plan: with lanes that have files, the mix lane after a
/// mute reads the files, and shows the columns of the same song in
/// memory.
#[wasm_bindgen_test]
async fn a_stored_mix_follows_mute_as_in_memory() {
    let id = "m7-follow";
    let (song, files) = planted(id, 3 * SECOND, 6).await;
    let (mut stored, _) = showing(id, song.clone(), files);
    let store = counting();
    stored.lane_store = Some(std::rc::Rc::clone(&store) as std::rc::Rc<dyn lane_store::LaneStore>);
    // The lanes take the page's store when they are shown.
    let shown = lanes::Shown::clone(stored.song.as_ref().expect("a song"));
    stored.load_song(shown);
    let mut memory = memory_page(song);
    for app in [&mut stored, &mut memory] {
        app.params.as_ref().expect("params").set_muted(1, true);
        follow_to_the_end(app).await;
    }
    let (got, want) = (&stored.lanes[0], &memory.lanes[0]);
    let same = (got.columns.clone(), got.scale) == (want.columns.clone(), want.scale);
    let requests = store.requests.get();
    stored.load_demo();
    store::test_remove(&format!("sessions/{id}")).await;
    assert!(requests > 0, "the mix read the lane files");
    assert!(same, "the columns of the mix in memory");
}

/// D5: a spectrogram of a stem and of the mix as heard, read from the
/// files, equals the one of the same samples in memory, bit for bit.  The
/// planted stems have a mid of 0 (their sides are opposite), so the test
/// adds a 16-bit stem whose sides differ, and the f32 "original" is in the
/// mix.
#[wasm_bindgen_test]
async fn a_stored_spectrogram_equals_the_memory_one() {
    let id = "m7-spectrogram";
    let frames = 3 * SECOND;
    let (song, files) = planted(id, frames, 6).await;
    let store: std::rc::Rc<dyn lane_store::LaneStore> = counting();
    let mut stored = lanes::Shown::with_files(song.clone(), files, &store).audio();
    let mut memory = lanes::Shown::in_memory(song).audio();
    let (left, right) = (stem_wave(frames, 3), stem_wave(frames, 4));
    let path = format!("sessions/{id}/stems/lloyd.wav");
    let wav = engine::wav::encode_pcm16(&left, &right, 44_100);
    store::test_write(&path, &wav).await.expect("planted");
    let file =
        store_meta::stem_lane_file(&path, &wav[..44], wav.len() as u64, frames).expect("a stem");
    stored.push(lanes::LaneAudio::Stored {
        file,
        overview: std::rc::Rc::new(lanes::Overview {
            columns: Vec::new(),
            scale: 1.0,
        }),
        store: std::rc::Rc::clone(&store),
    });
    memory.push(lanes::LaneAudio::Memory {
        left: left.into(),
        right: right.into(),
    });
    assert!(
        stored.iter().all(|lane| lane.samples().is_none()),
        "read from the files"
    );
    let stem = |lanes: &[lanes::LaneAudio]| -> Box<dyn engine::mixdown::Pcm> {
        Box::new(lanes::LanePcm::new(lanes[7].clone(), frames, 44_100))
    };
    let recipe = MixRecipe::new(vec![1.0, 0.0, 0.5, 1.0, 1.0, 1.0, 1.0, 0.75]);
    let mix = |lanes: &[lanes::LaneAudio]| -> Box<dyn engine::mixdown::Pcm> {
        Box::new(lanes::LanesMix::new(lanes, frames, 44_100, &recipe))
    };
    let silent = run_spectrogram(Box::new(lanes::LanePcm::new(
        lanes::LaneAudio::Memory {
            left: vec![0.0; frames].into(),
            right: vec![0.0; frames].into(),
        },
        frames,
        44_100,
    )))
    .await;
    let stem_image = run_spectrogram(stem(&stored)).await;
    let stem_same = stem_image == run_spectrogram(stem(&memory)).await;
    let mix_image = run_spectrogram(mix(&stored)).await;
    let mix_same = mix_image == run_spectrogram(mix(&memory)).await;
    store::test_remove(&format!("sessions/{id}")).await;
    assert!(
        stem_image != silent && mix_image != silent,
        "images with sound"
    );
    assert!(stem_same, "the spectrogram of a stem");
    assert!(mix_same, "the spectrogram of the mix as heard");
}

/// D3: a WAV of a stem, and of the mix as heard, read from the files,
/// equals the one of the same samples in memory, byte for byte.
#[wasm_bindgen_test]
async fn a_save_of_a_stored_lane_equals_the_memory_save() {
    let id = "m7-save";
    let frames = 2 * export::BLOCK + 77;
    let (song, files) = planted(id, frames, 6).await;
    let store: std::rc::Rc<dyn lane_store::LaneStore> = counting();
    let stored = lanes::Shown::with_files(song.clone(), files, &store).audio();
    let memory = lanes::Shown::in_memory(song).audio();
    let recipe = MixRecipe::new(vec![1.0, 0.0, 0.5, 1.0, 1.5, 1.0, 0.0]);
    let wav = |pcm: Box<dyn engine::mixdown::Pcm>| async move {
        let job = export::ExportJob::new(pcm, export::JsWav::new(frames, 44_100));
        let bytes = run_export(job).await.into_bytes();
        let mut out = vec![0; bytes.length() as usize];
        bytes.copy_to(&mut out);
        out
    };
    let stem_same = wav(Box::new(lanes::LanePcm::new(
        stored[3].clone(),
        frames,
        44_100,
    )))
    .await
        == wav(Box::new(lanes::LanePcm::new(
            memory[3].clone(),
            frames,
            44_100,
        )))
        .await;
    let mix_same = wav(Box::new(lanes::LanesMix::new(
        &stored, frames, 44_100, &recipe,
    )))
    .await
        == wav(Box::new(lanes::LanesMix::new(
            &memory, frames, 44_100, &recipe,
        )))
        .await;
    store::test_remove(&format!("sessions/{id}")).await;
    assert!(stem_same, "the WAV of a stem");
    assert!(mix_same, "the WAV of the mix as heard");
}

/// D3 and D5: during a follow, the spectrograms and a save of a 1-minute
/// session, no read of a lane file is more than a block of that lane, and
/// a spectrogram of a lane whose `hop` is over `n_fft` makes one request
/// for each 32 columns.
#[wasm_bindgen_test]
async fn the_page_reads_a_lane_one_block_at_a_time() {
    let id = "m7-blocks";
    let frames = 60 * SECOND;
    let (song, files) = planted(id, frames, 6).await;
    let (mut app, _) = showing(id, song, files);
    let store = counting();
    app.lane_store = Some(std::rc::Rc::clone(&store) as std::rc::Rc<dyn lane_store::LaneStore>);
    // The lanes take the page's store when they are shown.
    let shown = lanes::Shown::clone(app.song.as_ref().expect("a song"));
    app.load_song(shown);
    pb_spy_slices();
    app.params.as_ref().expect("params").set_muted(0, true);
    follow_to_the_end(&mut app).await;
    app.show_spectrograms = true;
    let ctx = egui::Context::default();
    for _ in 0..20_000 {
        app.step_spectrograms(&ctx, || true);
        if app.lanes.iter().all(|l| l.spectrogram.is_some()) {
            break;
        }
        pb_test_sleep(1).await;
    }
    let all_made = app.lanes.iter().all(|l| l.spectrogram.is_some());
    let audio = app.song.as_ref().expect("a song").audio();
    let save = export::ExportJob::new(
        Box::new(lanes::LanePcm::new(audio[1].clone(), frames, 44_100)),
        export::JsWav::new(frames, 44_100),
    );
    run_export(save).await;
    // One lane's spectrogram alone, to count its requests.
    let before = store.requests.get();
    let columns = engine::hpss::SpectrogramColumns::new(frames, 44_100, 8_000.0, 128, 1_024);
    let hop_over_fft = columns.hop() > 1_024;
    run_spectrogram(Box::new(lanes::LanePcm::new(
        audio[1].clone(),
        frames,
        44_100,
    )))
    .await;
    let requests = store.requests.get() - before;
    let slices = pb_slices();
    app.load_demo();
    store::test_remove(&format!("sessions/{id}")).await;
    assert!(all_made, "every spectrogram");
    assert!(hop_over_fft);
    assert_eq!(
        requests,
        columns.columns().div_ceil(32),
        "one request for each 32 columns"
    );
    let block = |name: &str| if name.ends_with(".wav") { 4 } else { 8 } * BLOCK_FRAMES as usize;
    let mut seen = 0;
    for slice in slices.iter() {
        let pair: js_sys::Array = slice.unchecked_into();
        let (name, bytes) = (
            pair.get(0).as_string().unwrap_or_default(),
            pair.get(1).as_f64().unwrap_or(0.0),
        );
        assert!(
            bytes as usize <= block(&name),
            "{name}: a read of {bytes} bytes"
        );
        seen += 1;
    }
    assert!(seen > 100, "{seen} reads");
    assert!(store.largest.get() as usize <= 8 * BLOCK_FRAMES as usize);
}

/// D2 of the M7 plan: with no read-ahead buffer (`Playback::start`
/// refused, as over the cap), a stored song's lanes play silence, never
/// samples, no lane has samples, and the status line says so.  The session
/// stays protected (D1).
#[wasm_bindgen_test]
async fn a_stored_song_with_no_read_ahead_plays_silence_and_says_so() {
    let id = "m7-silent";
    let (song, files) = planted(id, SECOND, 6).await;
    playback::refuse_for_test(true);
    let (mut app, _) = showing(id, song, files);
    playback::refuse_for_test(false);
    let no_buffer = app.playback.is_none();
    let no_samples = app
        .song
        .as_ref()
        .is_some_and(|s| s.lanes.iter().all(|l| l.audio.samples().is_none()));
    let status = app.status.clone();
    app.params
        .as_ref()
        .expect("params")
        .playing
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let (mut l, mut r) = (vec![1.0; CALLBACK], vec![1.0; CALLBACK]);
    stored_player(&mut app).process(&mut l, &mut r);
    let protected = store::protected(id);
    // Finding 1-11 of review round 1 of M7: a Storage open or the startup
    // restore shows the song through `apply_inbox`, with an inbox status
    // and a note.  The status line keeps the note of D2.
    let shown = lanes::Shown::clone(app.song.as_ref().expect("a song"));
    let token = app.next_open();
    {
        let mut inbox = app.pending.borrow_mut();
        inbox.status = Some("Restored \"lloyd\" from this browser.".into());
        inbox.put_song(
            token,
            Ok(Opened {
                song: shown,
                mix: None,
                session: None,
                name: Some("lloyd".into()),
                note: Some("acme".into()),
                guard: None,
                new_session: false,
            }),
        );
    }
    playback::refuse_for_test(true);
    app.apply_inbox(&egui::Context::default());
    playback::refuse_for_test(false);
    let restored = app.status.clone();
    app.load_demo();
    store::test_remove(&format!("sessions/{id}")).await;
    // Finding 2-4 of review round 2 of M7: the inbox status, the open's
    // note, then the note of D2, in that order.
    assert_eq!(
        restored,
        format!("Restored \"lloyd\" from this browser.  acme  {PLAYS_SILENT}")
    );
    assert!(no_buffer, "no read-ahead buffer");
    assert!(no_samples, "no lane has samples");
    assert!(
        l.iter().chain(&r).all(|v| *v == 0.0),
        "the stored lanes play silence"
    );
    assert!(status.contains("play silent"), "{status}");
    assert!(protected, "the session stays protected");
}

/// D1 of the M7 plan (a test of step 5): with no read-ahead buffer, the
/// shown stored song's session is still protected, so `forget_mix`
/// removes nothing while a job may read the mix file.
#[wasm_bindgen_test]
async fn a_protected_session_keeps_its_mix_while_a_job_reads_it() {
    let id = "m7-protected-mix";
    let (song, files) = planted(id, SECOND, 6).await;
    playback::refuse_for_test(true);
    let (mut app, _) = showing(id, song, files);
    playback::refuse_for_test(false);
    let no_buffer = app.playback.is_none();
    let removal = store::forget_mix(id).await;
    let kept = store::test_read(&store::mix_path(id)).await.is_some();
    app.load_demo();
    store::test_remove(&format!("sessions/{id}")).await;
    assert!(no_buffer);
    assert!(removal.is_ok(), "{removal:?}");
    assert!(kept, "the mix file stays while the song shows");
}

#[wasm_bindgen(inline_js = r#"
export function dt_blob(bytes) { return new Blob([bytes]); }
export function pb_file(bytes, name) { return new File([bytes], name); }
"#)]
extern "C" {
    fn dt_blob(bytes: &[u8]) -> web_sys::Blob;
    fn pb_file(bytes: &[u8], name: &str) -> web_sys::File;
}

/// Finding 3-1 of review round 3 of M7: a lane that arrives during a save,
/// such as classic drums from the decode worker, keeps the save.  The job
/// holds clones of the lanes, so it stays valid.  A FLAC save with no
/// worker runs its whole job, then says that the worker is not running.
#[wasm_bindgen_test]
async fn a_lane_added_during_a_save_keeps_the_save() {
    let mut app = StemApp {
        want_device: false,
        ..StemApp::default()
    };
    app.load_demo();
    app.separator = None;
    app.start_save(None, export::Format::Flac);
    let started = app.saving.is_some();
    let frames = app.song.as_ref().expect("a song").frames;
    app.add_stem("classic drums", vec![0.0; frames], vec![0.0; frames]);
    let kept = app.saving.is_some();
    let ctx = egui::Context::default();
    for _ in 0..10_000 {
        if app.saving.is_none() {
            break;
        }
        app.step_save(&ctx);
    }
    let status = app.status.clone();
    app.load_demo();
    assert!(started, "the save started");
    assert!(kept, "the save is kept");
    assert!(
        status.contains(".flac") && status.contains("encoding worker is not running"),
        "the save ran to its end: {status}"
    );
}

/// Findings 4-1, 5-1 and 5-5 of review rounds 4 and 5 of M7.  A save
/// whose read of browser storage fails stops within a few frames.  It
/// makes no file and no encode request, and says why.  A FLAC save has no
/// worker here.  So a FLAC save that went on would say that the worker is
/// not running.  A WAV save that went on would say "Saved".
#[wasm_bindgen_test]
async fn a_save_whose_read_fails_says_so_and_makes_no_file() {
    let id = "m7-save-read-fails";
    let (song, files) = planted(id, 30 * SECOND, 2).await;
    let (mut app, _) = showing(id, song, files);
    app.separator = None;
    assert!(store::test_remove(&format!("sessions/{id}/stems/drums.wav")).await);
    let ctx = egui::Context::default();
    let mut ends = Vec::new();
    for format in [export::Format::Flac, export::Format::Wav] {
        app.start_save(Some(1), format);
        let mut frames = 0;
        while app.saving.is_some() && frames < 2_000 {
            app.step_save(&ctx);
            pb_test_sleep(5).await;
            frames += 1;
        }
        ends.push((format, frames, app.saving.is_none(), app.status.clone()));
    }
    app.load_demo();
    store::test_remove(&format!("sessions/{id}")).await;
    for (format, frames, ended, status) in ends {
        assert!(ended, "{format:?}: the save ended");
        assert!(
            frames <= 10,
            "{format:?}: it stops at the failure: {frames} frames"
        );
        assert!(
            status.starts_with("Could not save ")
                && status.contains("could not be read from browser storage")
                && !status.contains("worker")
                && !status.contains("Saved"),
            "{format:?}: {status}"
        );
    }
}
