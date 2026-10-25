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
export async function plant(path, bytes) {
  let d = await navigator.storage.getDirectory();
  const parts = path.split("/");
  const last = parts.pop();
  for (const p of parts) d = await d.getDirectoryHandle(p, { create: true });
  const w = await (await d.getFileHandle(last, { create: true })).createWritable();
  await w.write(bytes);
  await w.close();
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
    /// Writes `bytes` to the file at `path`, with its folders.
    #[wasm_bindgen(js_name = plant)]
    async fn plant_js(path: &str, bytes: &js_sys::Uint8Array);
    /// Whether a file or folder exists at `path` under the OPFS root.
    async fn exists(path: &str) -> JsValue;
    /// Lets tasks spawned earlier run up to their first wait.
    async fn tick();
    async fn sleep(ms: u32);
}

const RATE: u32 = 44_100;

/// A `Blob` holding a copy of `bytes`, as a chosen file is.
fn blob(bytes: &[u8]) -> web_sys::Blob {
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes));
    web_sys::Blob::new_with_u8_array_sequence(&parts).expect("a blob")
}

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
    store::save_sources(
        &[("Lloyd - Tune.wav".into(), blob(&tone(800, 440.0)))],
        RATE,
    )
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

/// The worker's `.part` file of each stem in `names`, of 800 frames, in
/// session `id`.
async fn plant_parts(id: &str, names: &[&str]) {
    let wave = vec![0.1; 800];
    let wav = engine::wav::encode_pcm16(&wave, &wave, RATE);
    for name in names {
        let path = format!("sessions/{id}/stems/{name}.wav.part");
        plant_js(&path, &js_sys::Uint8Array::from(wav.as_slice())).await;
    }
}

/// Starts `keep_stored_stems` and lets it run up to its first wait, so it
/// is in progress when the caller goes on.  Whether `meta` names the stems
/// arrives in the cell.
async fn keeping(id: &str, names: &[&str]) -> Rc<RefCell<Option<bool>>> {
    let result = Rc::new(RefCell::new(None));
    let (id, out) = (id.to_string(), result.clone());
    let names: Vec<String> = names.iter().map(|n| n.to_string()).collect();
    wasm_bindgen_futures::spawn_local(async move {
        let stored = store::keep_stored_stems(&id, &names).await;
        *out.borrow_mut() = Some(stored.kept.is_ok());
    });
    tick().await;
    assert!(result.borrow().is_none(), "the move is in progress");
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

/// Decision 0012: a 48 kHz source opens at 44.1 kHz, and opens again with
/// the same samples, so the stems separated from it line up with it.
#[wasm_bindgen_test]
async fn a_48k_session_reopens_with_its_stems() {
    wipe().await;
    let wave: Vec<f32> = (0..4_800).map(|i| 0.25 * (0.05 * i as f32).sin()).collect();
    let wav = engine::wav::encode_pcm16(&wave, &wave, 48_000);
    let opened =
        engine::Song::load_files(vec![("lloyd.wav".into(), wav.clone())]).expect("decoded");
    assert_eq!((opened.sample_rate, opened.frames), (44_100, 4_410));
    let id = store::save_sources(&[("lloyd.wav".into(), blob(&wav))], opened.sample_rate)
        .await
        .expect("saved");
    store::save_stems(
        &id,
        opened.sample_rate,
        &[stem("bass", 4_410), stem("drums", 4_410)],
    )
    .await
    .expect("stems saved");
    let (_, song, meta) = store::restore().await.expect("stored").expect("readable");
    assert_eq!(meta.rate, 44_100);
    assert_eq!((song.sample_rate, song.frames), (44_100, 4_410));
    assert_eq!(names(&song), ["bass", "drums", "original"]);
    for s in &song.stems {
        assert_eq!((s.left.len(), s.right.len()), (4_410, 4_410), "{}", s.name);
    }
    let (original, first) = (&song.stems[2], &opened.stems[0]);
    assert!(
        original.left.as_slice() == first.left.as_slice(),
        "the same samples"
    );
    assert!(
        original.right.as_slice() == first.right.as_slice(),
        "the same samples"
    );
}

/// A 16-bit stereo FLAC of `wave` in both channels, from `flacenc`.
fn flac(wave: &[f32], rate: u32) -> Vec<u8> {
    use flacenc::component::BitRepr;
    use flacenc::error::Verify;
    const BLOCK: usize = 4096;
    let samples: Vec<i32> = wave
        .iter()
        .flat_map(|x| {
            let s = (x * 32768.0).round() as i32;
            [s, s]
        })
        .collect();
    let source = flacenc::source::MemSource::from_samples(&samples, 2, 16, rate as usize);
    let config = flacenc::config::Encoder::default()
        .into_verified()
        .expect("the default config");
    let mut stream =
        flacenc::encode_with_fixed_block_size(&config, source, BLOCK).expect("encoded");
    // flacenc gives the short last block as the minimum, which RFC 9639
    // (8.2) excludes, and Symphonia then rejects every frame.
    stream
        .stream_info_mut()
        .set_block_sizes(BLOCK, BLOCK)
        .expect("block sizes");
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream.write(&mut sink).expect("written");
    sink.into_inner()
}

/// A FLAC source is kept as it is (`src/0.flac`) and opens again with its
/// stems.
#[wasm_bindgen_test]
async fn a_flac_session_reopens_with_its_stems() {
    wipe().await;
    let wave: Vec<f32> = (0..4_410).map(|i| 0.25 * (0.05 * i as f32).sin()).collect();
    let bytes = flac(&wave, 44_100);
    let opened =
        engine::Song::load_files(vec![("lloyd.flac".into(), bytes.clone())]).expect("decoded");
    assert_eq!((opened.sample_rate, opened.frames), (44_100, 4_410));
    let id = store::save_sources(&[("lloyd.flac".into(), blob(&bytes))], opened.sample_rate)
        .await
        .expect("saved");
    store::save_stems(
        &id,
        opened.sample_rate,
        &[stem("bass", 4_410), stem("drums", 4_410)],
    )
    .await
    .expect("stems saved");
    let (_, song, meta) = store::restore().await.expect("stored").expect("readable");
    assert_eq!(
        meta.files,
        [("lloyd.flac".to_string(), "0.flac".to_string())]
    );
    assert_eq!((song.sample_rate, song.frames), (44_100, 4_410));
    assert_eq!(names(&song), ["bass", "drums", "original"]);
    let (original, first) = (&song.stems[2], &opened.stems[0]);
    assert!(
        original.left.as_slice() == first.left.as_slice(),
        "the same samples"
    );
}

/// A session stored before decision 0012 from stems at mixed rates can hold
/// stems one frame shorter than its source now opens with.  Each lane is
/// fitted to the source, so the player never reads past a stem's end.
#[wasm_bindgen_test]
async fn stems_that_do_not_fit_the_source_are_fitted_to_it() {
    for frames in [799, 801] {
        let id = stored_song().await;
        store::save_stems(&id, RATE, &[stem("bass", frames)])
            .await
            .expect("stems saved");
        let (_, song, _) = store::restore().await.expect("stored").expect("readable");
        assert_eq!(song.frames, 800);
        for s in &song.stems {
            assert_eq!(
                (s.left.len(), s.right.len()),
                (800, 800),
                "{} from {frames}",
                s.name
            );
        }
    }
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

/// D5 of M3: an open waits for the stored stems to move into place, so it
/// never reads a half-moved session.
#[wasm_bindgen_test]
async fn opening_waits_for_stored_stems_and_shows_them() {
    let id = stored_song().await;
    plant_parts(&id, &["bass", "vocals"]).await;
    let kept = keeping(&id, &["bass", "vocals"]).await;
    let (_, song, meta) = store::restore_id(&id)
        .await
        .expect("stored")
        .expect("readable");
    assert_eq!(*kept.borrow(), Some(true), "the move finished first");
    assert_eq!(meta.stems, ["bass", "vocals"]);
    assert_eq!(names(&song), ["bass", "vocals", "original"]);
}

/// D5 of M3: a removal waits for the stored stems to move into place.
#[wasm_bindgen_test]
async fn forgetting_waits_for_stored_stems_then_removes_the_session() {
    let id = stored_song().await;
    plant_parts(&id, &["bass", "drums"]).await;
    let kept = keeping(&id, &["bass", "drums"]).await;
    assert!(store::forget(&id).await, "removed");
    assert_eq!(*kept.borrow(), Some(true), "the move finished first");
    assert!(!has(&format!("sessions/{id}")).await);
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
    let newer = store::save_sources(&[("acme.wav".into(), blob(&tone(80, 220.0)))], RATE)
        .await
        .expect("saved");
    let ids: Vec<String> = store::list().await.into_iter().map(|(id, _)| id).collect();
    assert_eq!(ids, [newer.clone(), older]);
    assert_eq!(store::current().await, Some(newer));
}

/// MEM-1: proves each path in this crate's `clippy.toml` (see the `#[expect]`s).
#[wasm_bindgen_test]
fn mem1_lint_is_on_for_js_arrays() {
    #[expect(
        clippy::disallowed_methods,
        reason = "MEM-1 canary: clippy fails here if this path is off"
    )]
    let slice = [1.0_f32].to_vec();
    #[expect(
        clippy::disallowed_methods,
        reason = "MEM-1 canary: clippy fails here if this path is off"
    )]
    let bytes = js_sys::Uint8Array::from(&[2_u8][..]).to_vec();
    #[expect(
        clippy::disallowed_methods,
        reason = "MEM-1 canary: clippy fails here if this path is off"
    )]
    let floats = js_sys::Float32Array::from(&[3.0_f32][..]).to_vec();
    assert_eq!((slice, bytes, floats), (vec![1.0], vec![2], vec![3.0]));
}
