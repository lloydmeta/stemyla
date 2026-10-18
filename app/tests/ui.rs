//! M6 acceptance: interaction tests through AccessKit and pointer events,
//! plus snapshot PNGs in `app/tests/snapshots`.

use app::StemApp;
use eframe::egui::accesskit::Role;
use eframe::egui::{self, Pos2};
use egui_kittest::kittest::Queryable;
use egui_kittest::{Harness, SnapshotResults};
use std::sync::atomic::Ordering::Relaxed;

const SIZE: [f32; 2] = [1100.0, 600.0];

fn harness(load_demo: bool) -> Harness<'static, StemApp> {
    let mut h = Harness::builder()
        .with_size(SIZE)
        .with_pixels_per_point(1.0)
        .build_eframe(move |cc| {
            let mut app = StemApp::new(cc);
            app.want_device = false; // tests never touch audio hardware
            if load_demo {
                app.load_demo();
            }
            app
        });
    h.run();
    h
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
    let h = harness(true);
    let drums = stem(&h, "drums");
    let params = h.state().params.clone().unwrap();
    assert_eq!(params.effective_gain(drums), 1.0);
    // Mute buttons appear in lane order: drums is the first stem lane.
    h.get_all_by_label("Mute").nth(drums).unwrap().click();
    let mut h = h;
    h.run();
    assert!(params.is_muted(drums));
    assert_eq!(params.effective_gain(drums), 0.0);
    assert_eq!(params.effective_gain(stem(&h, "bass")), 1.0);
}

#[test]
fn solo_mutes_the_others() {
    let mut h = harness(true);
    let bass = stem(&h, "bass");
    let params = h.state().params.clone().unwrap();
    h.get_all_by_label("Solo").nth(bass).unwrap().click();
    h.run();
    for (i, s) in h.state().song.clone().unwrap().stems.iter().enumerate() {
        let g = params.effective_gain(i);
        if i == bass {
            assert_eq!(g, 1.0, "{}", s.name);
        } else {
            assert_eq!(g, 0.0, "{}", s.name);
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
            left: vec![0.1; frames],
            right: vec![0.1; frames],
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
    let names = lane_names(&h);
    assert!(
        ["bass", "vocals"]
            .iter()
            .all(|n| names.iter().any(|l| l == n)),
        "lanes: {names:?}"
    );
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
            app.open_song(song);
            app
        });
    h.run();
    let img = h.render().unwrap();
    img.save(root.join(out)).unwrap();
}
