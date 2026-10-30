//! M6 acceptance: interaction tests through AccessKit and pointer events,
//! plus snapshot PNGs in `app/tests/snapshots`.
//! Native only: egui_kittest renders through wgpu.
#![cfg(not(target_arch = "wasm32"))]

use app::StemApp;
use app::lane_store::{LaneStore, MemoryStore};
use app::lanes::{LaneAudio, Overview, Shown, ShownLane};
use app::store_meta::LaneFile;
use eframe::egui::accesskit::Role;
use eframe::egui::{self, Pos2};
use egui_kittest::kittest::Queryable;
use egui_kittest::{Harness, SnapshotResults};
use engine::Song;
use std::rc::Rc;
use std::sync::atomic::Ordering::Relaxed;

const SIZE: [f32; 2] = [1100.0, 600.0];

fn harness(load_demo: bool) -> Harness<'static, StemApp> {
    harness_with(move |app| {
        if load_demo {
            app.load_demo();
        }
    })
}

fn harness_with(open: impl FnOnce(&mut StemApp) + 'static) -> Harness<'static, StemApp> {
    let mut h = Harness::builder()
        .with_size(SIZE)
        .with_pixels_per_point(1.0)
        .build_eframe(move |cc| {
            let mut app = StemApp::new(cc);
            app.want_device = false; // tests never touch audio hardware
            open(&mut app);
            app
        });
    h.run();
    h
}

/// Where the demo song is: in memory, as `load_demo` makes it, or as
/// stored lanes (M7, step 3c).
#[derive(Clone, Copy, Debug)]
enum Demo {
    Memory,
    Stored,
}

const DEMOS: [Demo; 2] = [Demo::Memory, Demo::Stored];

fn demo_harness(demo: Demo) -> Harness<'static, StemApp> {
    match demo {
        Demo::Memory => harness(true),
        Demo::Stored => harness_with(|app| app.open_song(stored_demo().0)),
    }
}

/// The demo song in memory, as `load_demo` makes it.
fn demo_song() -> Song {
    let s = synth::generate();
    Song {
        sample_rate: synth::SAMPLE_RATE,
        frames: s.truth.frames,
        stems: s
            .stems
            .into_iter()
            .map(|(name, st)| engine::Stem {
                name,
                left: st.left.into(),
                right: st.right.into(),
            })
            .collect(),
    }
}

/// An f32 session file of two sides, as the decode worker writes one.
fn f32_file(left: &[f32], right: &[f32]) -> Vec<u8> {
    left.iter()
        .zip(right)
        .flat_map(|(l, r)| [l.to_le_bytes(), r.to_le_bytes()])
        .flatten()
        .collect()
}

/// The overview of two sides, read back from an overview file as the
/// decode worker writes it (D1 of M5).
fn overview_of(left: &[f32], right: &[f32]) -> Rc<Overview> {
    let (columns, _) = engine::overview::lane_columns(left, right);
    let file: Vec<u8> = columns
        .iter()
        .flat_map(|(lo, hi)| [lo.to_le_bytes(), hi.to_le_bytes()])
        .flatten()
        .collect();
    Rc::new(Overview::from_file(&file, left.len()).expect("an overview"))
}

/// The demo song as f32 session files with their overviews and the mix
/// overview, in a store in memory, as stored lanes.
fn stored_demo() -> (Shown, Rc<MemoryStore>) {
    let song = demo_song();
    let store = MemoryStore::new();
    let lanes = song
        .stems
        .iter()
        .enumerate()
        .map(|(k, stem)| {
            let path = format!("demo/decoded/{k}.f32");
            store.put(&path, f32_file(&stem.left, &stem.right));
            ShownLane {
                name: stem.name.clone(),
                audio: LaneAudio::Stored {
                    file: LaneFile::f32(path, song.frames),
                    overview: overview_of(&stem.left, &stem.right),
                    store: Rc::clone(&store) as Rc<dyn LaneStore>,
                },
            }
        })
        .collect();
    let (ml, mr) = app::mix_of_sources(&song);
    let shown = Shown {
        sample_rate: song.sample_rate,
        frames: song.frames,
        lanes,
        mix_overview: Some(overview_of(&ml, &mr)),
    };
    (shown, store)
}

/// Classic drums of `song` as a stored lane in `store`: D8's block HPSS
/// of the mix of its sources, as a 16-bit WAV with its overview.
fn stored_classic_drums(song: &Song, store: &Rc<MemoryStore>) -> LaneAudio {
    use engine::hpss::{BlockHpss, HpssConfig};
    let (ml, mr) = app::mix_of_sources(song);
    let mut hpss = BlockHpss::new(ml.len(), &HpssConfig::default());
    let (mut left, mut right) = (Vec::new(), Vec::new());
    let mut sink = |l: &[f32], r: &[f32]| -> Result<(), ()> {
        left.extend_from_slice(l);
        right.extend_from_slice(r);
        Ok(())
    };
    for (l, r) in ml.chunks(32_768).zip(mr.chunks(32_768)) {
        hpss.put(l, r, &mut sink).unwrap();
    }
    hpss.finish(&mut sink).unwrap();
    let wav = engine::wav::encode_pcm16(&left, &right, synth::SAMPLE_RATE);
    let path = "demo/classic.wav";
    let head = &wav[..4_096.min(wav.len())];
    let file = app::store_meta::stem_lane_file(path, head, wav.len() as u64, song.frames)
        .expect("a complete WAV");
    let decoded = engine::decode::decode_bytes(wav.clone(), Some("wav")).expect("decoded");
    let overview = overview_of(&decoded.channels[0], &decoded.channels[1]);
    store.put(path, wav);
    LaneAudio::Stored {
        file,
        overview,
        store: Rc::clone(store) as Rc<dyn LaneStore>,
    }
}

fn click_at(h: &mut Harness<'_, StemApp>, p: Pos2) {
    h.hover_at(p);
    h.event(egui::Event::PointerButton {
        pos: p,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Default::default(),
    });
    h.step();
    h.event(egui::Event::PointerButton {
        pos: p,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Default::default(),
    });
    h.run();
}

fn stem(h: &Harness<'_, StemApp>, name: &str) -> usize {
    h.state().song.as_ref().unwrap().stem_index(name).unwrap()
}

#[test]
fn six_lanes_are_laid_out_and_aligned() {
    let h = harness(true);
    let app = h.state();
    let names: Vec<_> = app.lanes.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["mix", "drums", "bass", "guitar", "keys", "lead"]);
    assert_eq!(app.lane_rects.len(), 6);
    let first = app.lane_rects[0];
    for r in &app.lane_rects {
        // Same horizontal extent for every lane, so one playhead lines up.
        assert_eq!(r.left(), first.left());
        assert_eq!(r.width(), first.width());
    }
    for w in app.lane_rects.windows(2) {
        assert!(w[1].top() >= w[0].bottom(), "lanes overlap");
    }
    // Each stem name appears as a label.
    for n in ["drums", "bass", "guitar", "keys", "lead"] {
        assert!(h.query_by_label(n).is_some(), "no label {n}");
    }
}

#[test]
fn mute_sets_engine_gain_to_zero() {
    for demo in DEMOS {
        let h = demo_harness(demo);
        let drums = stem(&h, "drums");
        let params = h.state().params.clone().unwrap();
        assert_eq!(params.effective_gain(drums), 1.0);
        // Mute buttons appear in lane order: drums is the first stem lane.
        h.get_all_by_label("Mute").nth(drums).unwrap().click();
        let mut h = h;
        h.run();
        assert!(params.is_muted(drums), "{demo:?}");
        assert_eq!(params.effective_gain(drums), 0.0);
        assert_eq!(params.effective_gain(stem(&h, "bass")), 1.0);
    }
}

#[test]
fn the_mix_lane_follows_mute() {
    for demo in DEMOS {
        the_mix_lane_follows_mute_of(demo);
    }
}

fn the_mix_lane_follows_mute_of(demo: Demo) {
    let mut h = demo_harness(demo);
    assert!(h.query_by_label("as heard: all stems").is_some());
    let song = demo_song();
    let lead = stem(&h, "lead");
    h.get_all_by_label("Mute").nth(lead).unwrap().click();
    h.run();
    let side = |f: fn(&engine::Stem) -> &[f32]| {
        let others: Vec<&[f32]> = song
            .stems
            .iter()
            .filter(|s| s.name != "lead")
            .map(f)
            .collect();
        dsp::mix(&others, &vec![1.0; others.len()])
    };
    let want = engine::overview::lane_columns(&side(|s| &s.left), &side(|s| &s.right));
    let mix = &h.state().lanes[0];
    assert_eq!((mix.columns.clone(), mix.scale), want, "{demo:?}");
    assert!(
        h.query_by_label("as heard: all stems except lead")
            .is_some()
    );

    // Unmuted, the lane shows the mix of every stem again.
    h.get_all_by_label("Mute").nth(lead).unwrap().click();
    h.run();
    let fresh = demo_harness(demo);
    assert_eq!(h.state().lanes[0].columns, fresh.state().lanes[0].columns);
}

#[test]
fn the_mix_lane_spectrogram_follows_mute() {
    for demo in DEMOS {
        the_mix_lane_spectrogram_follows_mute_of(demo);
    }
}

fn the_mix_lane_spectrogram_follows_mute_of(demo: Demo) {
    let mut h = demo_harness(demo);
    h.get_by_label("Spectrograms").click();
    h.run();
    let before = h.state().lanes[0].spectrogram.as_ref().map(|t| t.id());
    assert!(before.is_some(), "the mix lane has a spectrogram");
    let lead = stem(&h, "lead");
    let stem_before = h.state().lanes[lead + 1]
        .spectrogram
        .as_ref()
        .map(|t| t.id());
    h.get_all_by_label("Mute").nth(lead).unwrap().click();
    h.run();
    let app = h.state();
    let after = app.lanes[0].spectrogram.as_ref().map(|t| t.id());
    assert!(after.is_some(), "the mix lane has a spectrogram again");
    assert_ne!(after, before, "the mix lane's spectrogram is made again");
    assert_eq!(
        app.lanes[lead + 1].spectrogram.as_ref().map(|t| t.id()),
        stem_before,
        "a stem's spectrogram stays"
    );
}

/// The pixels of the mix lane, as drawn.
fn mix_lane_pixels(h: &mut Harness<'_, StemApp>) -> Vec<[u8; 4]> {
    let r = h.state().lane_rects[0];
    let image = h.render().expect("a render");
    let mut pixels = Vec::new();
    for y in r.top() as u32..r.bottom() as u32 {
        for x in r.left() as u32..r.right() as u32 {
            pixels.push(image.get_pixel(x, y).0);
        }
    }
    pixels
}

#[test]
fn the_mix_lane_spectrogram_is_the_mix_as_heard_on_either_path() {
    for demo in DEMOS {
        the_mix_lane_spectrogram_is_the_mix_as_heard_of(demo);
    }
}

fn the_mix_lane_spectrogram_is_the_mix_as_heard_of(demo: Demo) {
    let mute_lead = |h: &mut Harness<'_, StemApp>| {
        let lead = stem(h, "lead");
        h.get_all_by_label("Mute").nth(lead).unwrap().click();
        h.run();
    };
    let spectrograms = |h: &mut Harness<'_, StemApp>| {
        h.get_by_label("Spectrograms").click();
        h.run();
    };
    // Spectrograms on, then a mute: the waveform job makes the input.
    let mut follow = demo_harness(demo);
    spectrograms(&mut follow);
    mute_lead(&mut follow);
    // A mute, then spectrograms on: ensure_textures makes the input.
    let mut later = demo_harness(demo);
    mute_lead(&mut later);
    spectrograms(&mut later);
    // No mute: the mix of every stem.
    let mut all = demo_harness(demo);
    spectrograms(&mut all);

    let follow = mix_lane_pixels(&mut follow);
    assert!(
        follow == mix_lane_pixels(&mut later),
        "{demo:?}: the two paths draw the same mix"
    );
    assert!(
        follow != mix_lane_pixels(&mut all),
        "{demo:?}: the mix without lead differs from the mix of every stem"
    );
}

#[test]
fn solo_mutes_the_others() {
    for demo in DEMOS {
        let mut h = demo_harness(demo);
        let bass = stem(&h, "bass");
        let params = h.state().params.clone().unwrap();
        h.get_all_by_label("Solo").nth(bass).unwrap().click();
        h.run();
        let names: Vec<String> = h
            .state()
            .lanes
            .iter()
            .skip(1)
            .map(|l| l.name.clone())
            .collect();
        for (i, name) in names.iter().enumerate() {
            let g = params.effective_gain(i);
            if i == bass {
                assert_eq!(g, 1.0, "{demo:?}: {name}");
            } else {
                assert_eq!(g, 0.0, "{demo:?}: {name}");
            }
        }
    }
}

#[test]
fn tempo_slider_changes_transport_tempo() {
    let mut h = harness(true);
    let params = h.state().params.clone().unwrap();
    assert_eq!(params.tempo(), 1.0);
    let slider = h.get_by_role_and_label(Role::Slider, "Tempo");
    slider.focus();
    h.run();
    h.key_press(egui::Key::ArrowLeft);
    h.run();
    h.key_press(egui::Key::ArrowLeft);
    h.run();
    println!("tempo after two left presses: {}", params.tempo());
    assert!(
        (params.tempo() - 0.9).abs() < 1e-6,
        "tempo {}",
        params.tempo()
    );
}

#[test]
fn clicking_a_lane_seeks_there() {
    let mut h = harness(true);
    let params = h.state().params.clone().unwrap();
    let frames = h.state().song.as_ref().unwrap().frames as f64;
    let r = h.state().lane_rects[2];
    let p = Pos2::new(r.left() + 0.25 * r.width(), r.center().y);
    click_at(&mut h, p);
    let head = params.playhead() as f64;
    println!("seek to {:.3} of the song", head / frames);
    assert!(
        (head / frames - 0.25).abs() < 0.002,
        "playhead at {}",
        head / frames
    );
}

#[test]
fn play_moves_the_playhead_and_chipmunk_toggles() {
    let mut h = harness(true);
    let params = h.state().params.clone().unwrap();
    h.get_by_label("Play").click();
    h.run_steps(30);
    assert!(params.playing.load(Relaxed));
    assert!(params.playhead() > 0, "playhead did not move");
    h.get_by_label("Chipmunk").click();
    h.run_steps(2);
    assert!(params.chipmunk.load(Relaxed));
    h.get_by_label("Pause").click();
    h.run();
    assert!(!params.playing.load(Relaxed));
}

fn drag_loop(h: &mut Harness<'_, StemApp>, from: f32, to: f32) {
    let bar = h.state().loop_bar_rect.unwrap();
    let a = Pos2::new(bar.left() + from * bar.width(), bar.center().y);
    let b = Pos2::new(bar.left() + to * bar.width(), bar.center().y);
    h.hover_at(a);
    h.step();
    h.drag_at(a);
    h.step();
    for k in 1..=6 {
        let t = k as f32 / 6.0;
        h.hover_at(Pos2::new(a.x + (b.x - a.x) * t, a.y));
        h.step();
    }
    h.drop_at(b);
    h.run();
}

#[test]
fn dragging_the_loop_bar_sets_a_loop() {
    let mut h = harness(true);
    let params = h.state().params.clone().unwrap();
    let frames = h.state().song.as_ref().unwrap().frames as f64;
    drag_loop(&mut h, 0.25, 0.5);
    let (s, e) = params.loop_region().expect("loop set");
    println!("loop {:.3}..{:.3}", s as f64 / frames, e as f64 / frames);
    assert!((s as f64 / frames - 0.25).abs() < 0.01);
    assert!((e as f64 / frames - 0.5).abs() < 0.01);
}

#[test]
fn snapshots() {
    let mut results = SnapshotResults::new();

    let mut empty = harness(false);
    results.add(empty.try_snapshot("empty_state"));

    let mut h = harness(true);
    results.add(h.try_snapshot("six_lanes"));

    let drums = stem(&h, "drums");
    h.get_all_by_label("Mute").nth(drums).unwrap().click();
    h.run();
    results.add(h.try_snapshot("muted_drums"));

    let mut l = harness(true);
    drag_loop(&mut l, 0.25, 0.5);
    let r = l.state().lane_rects[1];
    click_at(&mut l, Pos2::new(r.left() + 0.3 * r.width(), r.center().y));
    results.add(l.try_snapshot("loop_region"));
}

/// The snapshots of `snapshots` and of `classic_drums_lane_and_spectrograms`
/// with the demo song as stored lanes: f32 session files with their
/// overviews, in a store in memory (M7, step 3c).  Classic drums is a
/// stored 16-bit WAV of D8's block HPSS.
#[test]
fn snapshots_of_a_stored_song() {
    let mut results = SnapshotResults::new();

    let mut h = demo_harness(Demo::Stored);
    results.add(h.try_snapshot("six_lanes"));

    let drums = stem(&h, "drums");
    h.get_all_by_label("Mute").nth(drums).unwrap().click();
    h.run();
    results.add(h.try_snapshot("muted_drums"));

    let mut l = demo_harness(Demo::Stored);
    drag_loop(&mut l, 0.25, 0.5);
    let r = l.state().lane_rects[1];
    click_at(&mut l, Pos2::new(r.left() + 0.3 * r.width(), r.center().y));
    results.add(l.try_snapshot("loop_region"));

    let mut c = harness_with(|app| {
        let (shown, store) = stored_demo();
        app.open_song(shown);
        app.add_lane("classic drums", stored_classic_drums(&demo_song(), &store));
    });
    c.get_by_label("Spectrograms").click();
    c.run();
    assert!(c.state().lanes.iter().all(|l| l.spectrogram.is_some()));
    assert_eq!(c.state().lanes.last().unwrap().name, "classic drums");
    c.set_size(egui::Vec2::new(1100.0, 680.0));
    c.run();
    results.add(c.try_snapshot("spectrograms_classic_drums"));
}

#[derive(Debug)]
struct TestDrop {
    path: std::path::PathBuf,
}

impl egui::DroppedFile for TestDrop {
    fn path(&self) -> &std::path::Path {
        &self.path
    }
    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.path).map_err(|e| e.to_string())
    }
}

#[test]
fn dropping_several_files_loads_them_as_stems() {
    let dir = std::env::temp_dir().join(format!("stems-drop-{}", std::process::id()));
    let song = synth::generate();
    synth::write_song(&song, &dir, false).unwrap();
    let mut h = harness(false);
    for name in ["drums", "bass", "mix"] {
        let f: std::sync::Arc<dyn egui::DroppedFile + Send + Sync> =
            std::sync::Arc::new(TestDrop {
                path: dir.join(format!("{name}.wav")),
            });
        h.input_mut().dropped_files.push(f);
    }
    h.run();
    let app = h.state();
    let names: Vec<_> = app.lanes.iter().map(|l| l.name.as_str()).collect();
    // mix.wav is skipped: the mix lane is always recomputed from the stems.
    assert_eq!(names, ["mix", "drums", "bass"], "status: {}", app.status);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn classic_drums_lane_and_spectrograms() {
    let mut h = harness(true);
    h.get_by_label("Classic drums").click();
    h.step();
    // The job runs on a thread (and repaints while it waits); step until
    // its lane appears.
    let start = std::time::Instant::now();
    while h.state().lanes.len() < 7 {
        assert!(
            start.elapsed().as_secs() < 120,
            "classic drums never arrived"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
        h.step();
    }
    h.run();
    let app = h.state();
    assert_eq!(app.lanes.last().unwrap().name, "classic drums");
    let params = app.params.clone().unwrap();
    let i = app
        .song
        .as_ref()
        .unwrap()
        .stem_index("classic drums")
        .unwrap();
    assert!(params.is_muted(i), "derived lane must start muted");
    // The mix lane still excludes it: same peaks as before.
    let mut fresh = harness(true);
    fresh.run();
    assert_eq!(app.lanes[0].columns, fresh.state().lanes[0].columns);

    h.get_by_label("Spectrograms").click();
    h.run();
    assert!(h.state().lanes.iter().all(|l| l.spectrogram.is_some()));
    let mut results = SnapshotResults::new();
    h.set_size(egui::Vec2::new(1100.0, 680.0));
    h.run();
    results.add(h.try_snapshot("spectrograms_classic_drums"));
}

fn lane_names(h: &Harness<'_, StemApp>) -> Vec<String> {
    h.state().lanes.iter().map(|l| l.name.clone()).collect()
}

/// Two stems the length of the shown song, as a separation returns them.
fn separated(h: &Harness<'_, StemApp>) -> Vec<engine::Stem> {
    let frames = h.state().song.as_ref().unwrap().frames;
    ["bass", "vocals"]
        .map(|name| engine::Stem {
            name: name.into(),
            left: vec![0.1; frames].into(),
            right: vec![0.1; frames].into(),
        })
        .into()
}

#[test]
fn separated_stems_join_the_song_they_came_from() {
    let mut h = harness(true);
    let job = h.state().separation_job().unwrap();
    let stems = separated(&h);
    h.state_mut().apply_separated(job, stems, 1.0);
    h.run();
    assert_eq!(lane_names(&h), ["mix", "bass", "vocals", "original"]);
}

#[test]
fn separated_stems_join_their_song_after_a_lane_is_added() {
    let mut h = harness(true);
    let job = h.state().separation_job().unwrap();
    let stems = separated(&h);
    let frames = stems[0].left.len();
    h.state_mut()
        .add_stem("classic drums", vec![0.0; frames], vec![0.0; frames]);
    h.state_mut().apply_separated(job, stems, 1.0);
    h.run();
    // Separated stems replace every stem lane, an added one included, as
    // they always have; what matters is that they arrived.
    assert_eq!(lane_names(&h), ["mix", "bass", "vocals", "original"]);
}

#[test]
fn separated_stems_never_join_another_song() {
    let mut h = harness(true);
    let job = h.state().separation_job().unwrap();
    let stems = separated(&h);
    h.state_mut().load_demo();
    h.run();
    let before = lane_names(&h);
    h.state_mut().apply_separated(job, stems, 1.0);
    h.run();
    assert_eq!(lane_names(&h), before);
    assert!(
        h.state().status.contains("discarded"),
        "status: {}",
        h.state().status
    );
}

#[test]
fn classic_drums_never_join_another_song() {
    let mut h = harness(true);
    h.get_by_label("Classic drums").click();
    h.step();
    assert!(h.state().classic_running());
    h.state_mut().load_demo();
    let start = std::time::Instant::now();
    while h.state().classic_running() {
        assert!(
            start.elapsed().as_secs() < 120,
            "classic drums never finished"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
        h.step();
    }
    h.run();
    assert!(!lane_names(&h).contains(&"classic drums".to_string()));
    assert!(
        h.state().status.contains("no longer open"),
        "status: {}",
        h.state().status
    );
}

/// Not part of the suite: renders the app with a stems folder to a PNG, for
/// the report.  `STEMS_DIR=stems/queen-classic OUT_PNG=docs/x.png cargo test
/// -p app --release --test ui render_folder -- --ignored`
#[test]
#[ignore]
fn render_folder() {
    let dir = std::env::var("STEMS_DIR").expect("STEMS_DIR");
    let out = std::env::var("OUT_PNG").expect("OUT_PNG");
    let spectro = std::env::var("SPECTROGRAMS").is_ok();
    let height: f32 = std::env::var("HEIGHT")
        .ok()
        .and_then(|h| h.parse().ok())
        .unwrap_or(420.0);
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let song = engine::Song::load_folder(&root.join(&dir)).unwrap();
    let mut h = Harness::builder()
        .with_size([1100.0, height])
        .with_pixels_per_point(1.0)
        .build_eframe(move |cc| {
            let mut app = StemApp::new(cc);
            app.want_device = false;
            app.show_spectrograms = spectro;
            app.open_memory_song(song);
            app
        });
    h.run();
    let img = h.render().unwrap();
    img.save(root.join(out)).unwrap();
}
