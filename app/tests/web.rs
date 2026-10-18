//! Browser tests of the song sessions kept in the browser's private file
//! system (`app::store`), run in a real browser by
//! `cargo run -p xtask -- web-test`.
//!
//! The tests share one page and its origin's storage, so each starts from
//! empty storage, and they run one at a time.  `store::close` is final for
//! the page, so its test lives in `web_close.rs`.
#![cfg(target_arch = "wasm32")]

use app::store;
use engine::Stem;
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen(inline_js = r#"
export async function wipe() {
  const root = await navigator.storage.getDirectory();
  for await (const name of root.keys()) await root.removeEntry(name, { recursive: true });
}
export async function exists(path) {
  let d = await navigator.storage.getDirectory();
  const parts = path.split("/");
  const last = parts.pop();
  try {
    for (const p of parts) d = await d.getDirectoryHandle(p);
    for await (const name of d.keys()) if (name === last) return true;
    return false;
  } catch (e) { return false; }
}
export function tick() { return Promise.resolve(); }
export function sleep(ms) { return new Promise((r) => setTimeout(r, ms)); }
"#)]
extern "C" {
    async fn wipe();
    /// Whether a file or folder exists at `path` under the OPFS root.
    async fn exists(path: &str) -> JsValue;
    /// Lets tasks spawned earlier run up to their first wait.
    async fn tick();
    async fn sleep(ms: u32);
}

const RATE: u32 = 8_000;

/// A short stereo tone as a WAV file, made in memory.
fn tone(frames: usize, hz: f32) -> Vec<u8> {
    let wave: Vec<f32> = (0..frames)
        .map(|i| 0.25 * (std::f32::consts::TAU * hz * i as f32 / RATE as f32).sin())
        .collect();
    engine::wav::encode_pcm16(&wave, &wave, RATE)
}

fn stem(name: &str, frames: usize) -> Stem {
    let wave = vec![0.1; frames];
    Stem {
        name: name.into(),
        left: wave.clone().into(),
        right: wave.into(),
    }
}

async fn has(path: &str) -> bool {
    exists(path).await.as_bool() == Some(true)
}

/// A stored song of one WAV file, made current; returns its id.
async fn stored_song() -> String {
    wipe().await;
    store::save_sources(&[("Lloyd - Tune.wav".into(), tone(800, 440.0))], RATE)
        .await
        .expect("saved")
}

/// Starts `save_stems` and lets it run up to its first wait, so it is in
/// progress when the caller goes on.  Its result arrives in the cell.
async fn saving(id: &str, stems: Vec<Stem>) -> Rc<RefCell<Option<Result<(), String>>>> {
    let result = Rc::new(RefCell::new(None));
    let (id, out) = (id.to_string(), result.clone());
    wasm_bindgen_futures::spawn_local(async move {
        let saved = store::save_stems(&id, RATE, &stems).await;
        *out.borrow_mut() = Some(saved.map_err(|e| e.to_string()));
    });
    tick().await;
    assert!(result.borrow().is_none(), "the save is in progress");
    result
}

fn names(song: &engine::Song) -> Vec<&str> {
    song.stems.iter().map(|s| s.name.as_str()).collect()
}

#[wasm_bindgen_test]
async fn a_saved_song_is_restored_with_its_name_and_rate() {
    let id = stored_song().await;
    assert_eq!(store::current().await.as_deref(), Some(id.as_str()));
    let (got, song, meta) = store::restore().await.expect("stored").expect("readable");
    assert_eq!(got, id);
    assert_eq!(meta.name, "Lloyd - Tune");
    assert_eq!(meta.rate, RATE);
    assert_eq!(meta.files, [("Lloyd - Tune.wav".into(), "0.wav".into())]);
    assert!(meta.stems.is_empty());
    assert_eq!(song.sample_rate, RATE);
    assert_eq!(song.frames, 800);
}

#[wasm_bindgen_test]
async fn stems_saved_later_are_restored_with_the_song() {
    let id = stored_song().await;
    store::save_stems(&id, RATE, &[stem("bass", 800), stem("drums", 800)])
        .await
        .expect("stems saved");
    let (_, song, meta) = store::restore().await.expect("stored").expect("readable");
    assert_eq!(meta.stems, ["bass", "drums"]);
    assert_eq!(names(&song), ["bass", "drums", "original"]);
    assert_eq!(song.frames, 800);
}

#[wasm_bindgen_test]
async fn stems_for_a_missing_session_are_refused_and_create_nothing() {
    wipe().await;
    let saved = store::save_stems("42", RATE, &[stem("bass", 100)]).await;
    assert!(saved.is_err());
    assert!(!has("sessions/42").await);
}

#[wasm_bindgen_test]
async fn forgetting_waits_for_a_save_in_progress_then_removes_the_session() {
    let id = stored_song().await;
    let save = saving(&id, vec![stem("bass", 800), stem("drums", 800)]).await;
    assert!(store::forget(&id).await, "removed");
    assert_eq!(
        *save.borrow(),
        Some(Ok(())),
        "the save finished before the removal"
    );
    assert!(!has(&format!("sessions/{id}")).await);
    assert_eq!(store::current().await, None, "no longer current");
}

#[wasm_bindgen_test]
async fn forgetting_a_missing_session_succeeds() {
    wipe().await;
    assert!(store::forget("42").await);
}

#[wasm_bindgen_test]
async fn opening_waits_for_a_save_in_progress_and_shows_its_stems() {
    let id = stored_song().await;
    let save = saving(&id, vec![stem("vocals", 800)]).await;
    let (_, song, meta) = store::restore_id(&id)
        .await
        .expect("stored")
        .expect("readable");
    assert_eq!(*save.borrow(), Some(Ok(())));
    assert_eq!(meta.stems, ["vocals"]);
    assert_eq!(names(&song), ["vocals", "original"]);
}

#[wasm_bindgen_test]
async fn forgetting_waits_for_every_overlapping_save() {
    let id = stored_song().await;
    let first = saving(&id, vec![stem("bass", 800)]).await;
    // Long enough to outlast the first save by several of `forget`'s polls.
    let second = saving(
        &id,
        (0..6).map(|i| stem(&format!("s{i}"), 200_000)).collect(),
    )
    .await;
    assert!(store::forget(&id).await, "removed");
    assert_eq!(*first.borrow(), Some(Ok(())), "first save");
    assert_eq!(*second.borrow(), Some(Ok(())), "second save");
    assert!(!has(&format!("sessions/{id}")).await);
}

#[wasm_bindgen_test]
async fn sessions_are_listed_newest_first() {
    let older = stored_song().await;
    // Ids are creation times in milliseconds.
    sleep(5).await;
    let newer = store::save_sources(&[("acme.wav".into(), tone(80, 220.0))], RATE)
        .await
        .expect("saved");
    let ids: Vec<String> = store::list().await.into_iter().map(|(id, _)| id).collect();
    assert_eq!(ids, [newer.clone(), older]);
    assert_eq!(store::current().await, Some(newer));
}
