//! Browser tests of the storage path of a save (M8 of
//! `plans/active/stream-from-storage.md`, `cargo run -p xtask -- web-test`):
//! the WAV save file that the page writes in blocks, and the lifetime of
//! save files.  The test plants a stored song and steps the save's job as
//! the page does.

use super::*;
use export::{ExportJob, Finish, SAVE_WRITE_BYTES, Sink, StoredWav};
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

#[wasm_bindgen(inline_js = r#"
export function sv_sleep(ms) { return new Promise((r) => setTimeout(r, ms)); }
// Keeps each download that the page offers, in place of the download:
// the link's name and the Blob of its object URL.
let links = null;
let real_click = null;
let real_url = null;
export function sv_spy_links() {
  if (real_click) throw new Error("a spy is already on the links");
  links = [];
  const blobs = new Map();
  real_url = URL.createObjectURL;
  URL.createObjectURL = (b) => { const u = real_url(b); blobs.set(u, b); return u; };
  real_click = HTMLAnchorElement.prototype.click;
  HTMLAnchorElement.prototype.click = function () { links.push({ name: this.download, blob: blobs.get(this.href) }); };
}
// Restores both, and gives the downloads offered since `sv_spy_links`.
export function sv_links() {
  URL.createObjectURL = real_url;
  HTMLAnchorElement.prototype.click = real_click;
  real_click = null;
  return links;
}
"#)]
extern "C" {
    async fn sv_sleep(ms: u32);
    pub(crate) fn sv_spy_links();
    pub(crate) fn sv_links() -> js_sys::Array;
}

const SECOND: usize = 44_100;
const NAMES: [&str; 3] = ["bass", "drums", "vocals"];

/// A tone for stem `k`, as the 16-bit samples that its WAV holds.
fn stem_wave(frames: usize, k: usize) -> Vec<f32> {
    let f = 0.0013 * (k + 1) as f32;
    (0..frames)
        .map(|i| {
            let v = 0.3 * (k + 1) as f32 * (f * i as f32).sin();
            (v * 32_768.0).round() / 32_768.0
        })
        .collect()
}

/// A song of `stems` stems of `frames` frames, planted as 16-bit WAVs in
/// session `id`, shown as stored lanes.  Also the WAV of the mix of every
/// stem at full volume, which a save of the mix as heard writes.  The
/// stems are loud, so the mix clips.
async fn stored_song(id: &str, frames: usize, stems: usize) -> (lanes::Shown, Vec<u8>) {
    let folder = format!("sessions/{id}");
    store::test_remove(&folder).await;
    let (mut song_stems, mut files, mut lefts, mut rights) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
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
        lefts.push(left.clone());
        rights.push(right.clone());
        song_stems.push(Stem {
            name: name.to_string(),
            left: left.into(),
            right: right.into(),
        });
    }
    let ones = vec![1.0; stems];
    let side = |c: &[Vec<f32>]| {
        let c: Vec<&[f32]> = c.iter().map(Vec::as_slice).collect();
        dsp::mix(&c, &ones)
    };
    let want = engine::wav::encode_pcm16(&side(&lefts), &side(&rights), 44_100);
    let song = Song {
        sample_rate: 44_100,
        frames,
        stems: song_stems,
    };
    let store: std::rc::Rc<dyn lane_store::LaneStore> =
        std::rc::Rc::new(lane_store::OpfsStore::new(None));
    (lanes::Shown::with_files(song, files, &store), want)
}

/// The job of a save of the mix of every lane of `shown` into a WAV save
/// file at `path`, as the page makes it.
fn mix_save(shown: &lanes::Shown, path: &str) -> ExportJob<StoredWav> {
    let recipe = MixRecipe::new(vec![1.0; shown.lanes.len()]);
    let source = lanes::LanesMix::new(&shown.audio(), shown.frames, 44_100, &recipe);
    let sink = StoredWav::new(path.into(), shown.frames, 44_100, std::rc::Rc::new(|| {}));
    ExportJob::new(Box::new(source), sink).stopping_at_a_failure()
}

/// Steps `job` as `step_save` does, a few ms in each frame, until it ends.
async fn run_job(job: &mut ExportJob<StoredWav>) {
    for _ in 0..20_000 {
        if job.run_while(export::time_left(4.0)) {
            return;
        }
        sv_sleep(1).await;
    }
    panic!("the job did not end");
}

/// Runs the end stage of a done job until it ends.
async fn finish(job: &mut ExportJob<StoredWav>) -> Finish {
    for _ in 0..5_000 {
        match job.sink_mut().finish() {
            Finish::Pending => sv_sleep(1).await,
            end => return end,
        }
    }
    panic!("the end stage did not end");
}

/// Waits until no save write is in progress, or 5 s.
async fn writes_end() {
    for _ in 0..500 {
        if store::test_writes() == 0 {
            return;
        }
        sv_sleep(10).await;
    }
}

const SAVE: &str = "saves/1792640000000-1.wav";

/// D3 of M8, MEM-5: a WAV save of a stored mix of 30 s writes a file equal
/// to `engine::wav::encode_pcm16` of the mix, of exactly 44 + 4 x frames
/// bytes once the end stage is done.  Each write is a whole array of at
/// most 1 MiB.  Every full block goes through one JS array, and the
/// page's WebAssembly heap stays under 4 MB above its start.
#[wasm_bindgen_test]
async fn a_wav_save_of_a_stored_mix_writes_whole_blocks_through_one_array() {
    let id = "m8-wav-save";
    let frames = 30 * SECOND;
    let (shown, want) = stored_song(id, frames, 3).await;
    store::test_remove(store::SAVES).await;
    store::tests::spy();
    let start = store::tests::heap_start();
    let mut job = mix_save(&shown, SAVE);
    run_job(&mut job).await;
    let done = job.is_done();
    let end = finish(&mut job).await;
    let peak = store::tests::heap_peak_above(start) as f64 / 1e6;
    let record = store::tests::stop_spy();
    let repeats = store::tests::spy_repeats();
    drop(job);
    let got = store::test_read(SAVE).await;
    store::test_remove(store::SAVES).await;
    store::test_remove(&format!("sessions/{id}")).await;
    assert!(done, "the job read the whole song");
    assert_eq!(end, Finish::Done);
    let got = got.expect("the save file");
    assert_eq!(got.len(), 44 + 4 * frames, "the file is complete");
    assert!(got == want, "the file is the WAV of the mix");
    let full = want.len() / SAVE_WRITE_BYTES;
    assert_eq!(
        record,
        vec!["whole"; full + 1].join(" "),
        "MEM-5: {full} full blocks, then the last"
    );
    assert_eq!(
        repeats,
        format!("0{}0", "1".repeat(full - 1)),
        "one JS array for every full block, and a new one for the last"
    );
    assert!(
        peak < 4.0,
        "MEM-1: the save held {peak:.1} MB of WebAssembly heap at its peak"
    );
}

/// D3 of M8: a WAV save of less than a block writes one array, of its own
/// length, and the file is complete.
#[wasm_bindgen_test]
async fn a_short_wav_save_writes_one_array_of_its_length() {
    let id = "m8-short-save";
    let (shown, want) = stored_song(id, 1_000, 2).await;
    store::test_remove(store::SAVES).await;
    store::tests::spy();
    let mut job = mix_save(&shown, SAVE);
    run_job(&mut job).await;
    let end = finish(&mut job).await;
    let record = store::tests::stop_spy();
    drop(job);
    let got = store::test_read(SAVE).await;
    store::test_remove(store::SAVES).await;
    store::test_remove(&format!("sessions/{id}")).await;
    assert_eq!(end, Finish::Done);
    assert_eq!(record, "whole");
    assert!(got.as_deref() == Some(&want[..]));
}

/// Steps `job` until a write of a full block is in flight.
async fn until_a_write_is_in_flight(job: &mut ExportJob<StoredWav>) {
    for _ in 0..20_000 {
        job.run_while(export::time_left(4.0));
        if job.fraction() > 0.0 && !job.sink().ready() && job.sink().failed().is_none() {
            return;
        }
        sv_sleep(0).await;
    }
    panic!("no write was in flight");
}

/// D3 of M8: a save that the page drops, while its file opens or while a
/// write is in flight, leaves no file and no open stream.
#[wasm_bindgen_test]
async fn a_dropped_wav_save_leaves_no_file_and_no_stream() {
    let id = "m8-dropped-save";
    let (shown, _) = stored_song(id, 30 * SECOND, 2).await;
    store::test_remove(store::SAVES).await;
    let opening = mix_save(&shown, SAVE);
    drop(opening);
    writes_end().await;
    let after_open = (store::test_writes(), store::test_read(SAVE).await.is_some());
    let mut job = mix_save(&shown, SAVE);
    until_a_write_is_in_flight(&mut job).await;
    drop(job);
    writes_end().await;
    let after_write = (store::test_writes(), store::test_read(SAVE).await.is_some());
    store::test_remove(store::SAVES).await;
    store::test_remove(&format!("sessions/{id}")).await;
    assert_eq!(after_open, (0, false), "dropped while it opens");
    assert_eq!(after_write, (0, false), "dropped during a write");
}

/// D3 of M8: Reset's `store::close` waits for a save's write.  The save's
/// next write fails, so the save removes its file and ends its stream.
#[wasm_bindgen_test]
async fn reset_waits_for_a_wav_save_and_leaves_no_file() {
    let id = "m8-reset-save";
    let (shown, _) = stored_song(id, 30 * SECOND, 2).await;
    store::test_remove(store::SAVES).await;
    let mut job = mix_save(&shown, SAVE);
    until_a_write_is_in_flight(&mut job).await;
    let closed = std::rc::Rc::new(std::cell::Cell::new(false));
    let flag = std::rc::Rc::clone(&closed);
    wasm_bindgen_futures::spawn_local(async move {
        store::close().await;
        flag.set(true);
    });
    sv_sleep(0).await;
    let waited = !closed.get();
    run_job(&mut job).await;
    let failed = job.sink().failed();
    for _ in 0..500 {
        if closed.get() {
            break;
        }
        sv_sleep(10).await;
    }
    let ended = closed.get();
    drop(job);
    store::test_reopen();
    let file = store::test_read(SAVE).await.is_some();
    store::test_remove(store::SAVES).await;
    store::test_remove(&format!("sessions/{id}")).await;
    assert!(waited, "close waits for the write in flight");
    assert!(ended, "close ends once the save stops");
    assert!(
        failed.as_deref() == Some("browser storage is being reset"),
        "{failed:?}"
    );
    assert!(!file, "no file stays");
}

/// D3 and D8 of M8: a refused open, and a write that fails in the middle
/// of the file, each remove the file and say why.
#[wasm_bindgen_test]
async fn a_refused_open_or_write_removes_the_file_and_says_why() {
    let id = "m8-refused-save";
    let (shown, _) = stored_song(id, 30 * SECOND, 2).await;
    store::test_remove(store::SAVES).await;
    store::tests::refuse_next_writable();
    let mut job = mix_save(&shown, SAVE);
    run_job(&mut job).await;
    let refused_open = (job.is_done(), job.sink().failed(), job.sink_mut().finish());
    drop(job);
    writes_end().await;
    let open_file = store::test_read(SAVE).await.is_some();
    store::tests::start_spy(&JsValue::UNDEFINED, 2.0);
    let mut job = mix_save(&shown, SAVE);
    run_job(&mut job).await;
    let record = store::tests::stop_spy();
    let refused_write = (job.is_done(), job.sink().failed());
    drop(job);
    writes_end().await;
    let write_file = store::test_read(SAVE).await.is_some();
    let writes = store::test_writes();
    store::test_remove(store::SAVES).await;
    store::test_remove(&format!("sessions/{id}")).await;
    let (done, failed, end) = refused_open;
    assert!(!done, "a refused open stops the job");
    assert!(
        failed
            .as_deref()
            .is_some_and(|f| f.contains("NoModificationAllowedError")),
        "{failed:?}"
    );
    assert!(matches!(end, Finish::Failed(_)), "{end:?}");
    assert!(!open_file, "a refused open leaves no file");
    let (done, failed) = refused_write;
    assert_eq!(
        record, "whole whole refused",
        "two writes, then the refusal"
    );
    assert!(!done, "a failed write stops the job");
    assert!(
        failed
            .as_deref()
            .is_some_and(|f| f.contains("QuotaExceededError")),
        "{failed:?}"
    );
    assert!(!write_file, "a failed write leaves no file");
    assert_eq!(writes, 0, "no stream stays open");
}

/// D7 of M8: the clean-up at the page's start removes a save file older
/// than its age limit and keeps a newer one.
#[wasm_bindgen_test]
async fn old_save_files_are_removed_and_new_ones_kept() {
    store::test_remove(store::SAVES).await;
    store::test_write("saves/1-1.wav", b"old")
        .await
        .expect("planted");
    sv_sleep(30).await;
    let between = js_sys::Date::now();
    sv_sleep(30).await;
    store::test_write("saves/2-1.flac", b"new")
        .await
        .expect("planted");
    let removed = store::clean_saves(between).await;
    let old = store::test_read("saves/1-1.wav").await;
    let new = store::test_read("saves/2-1.flac").await;
    let none = store::clean_saves(0.0).await;
    store::test_remove(store::SAVES).await;
    let no_folder = store::clean_saves(js_sys::Date::now()).await;
    assert_eq!(removed, ["1-1.wav"]);
    assert_eq!(old, None);
    assert_eq!(new.as_deref(), Some(&b"new"[..]));
    assert!(none.is_empty(), "nothing is older than 1970");
    assert!(no_folder.is_empty(), "no saves folder");
}

/// D6 and D7 of M8: an offered save file is removed when its timer ends,
/// and a missing one is not offered.
#[wasm_bindgen_test]
async fn an_offered_save_file_is_removed_when_its_timer_ends() {
    store::test_remove(store::SAVES).await;
    store::test_write("saves/3-1.wav", b"RIFF")
        .await
        .expect("planted");
    sv_spy_links();
    let offered = store::download_file("lloyd - mix.wav", "saves/3-1.wav", "audio/wav", 50).await;
    let links = sv_links();
    let kept = store::test_read("saves/3-1.wav").await.is_some();
    sv_sleep(200).await;
    let gone = store::test_read("saves/3-1.wav").await.is_none();
    sv_spy_links();
    let missing = store::download_file("lloyd.wav", "saves/4-1.wav", "audio/wav", 50).await;
    let no_links = sv_links().length();
    store::test_remove(store::SAVES).await;
    assert!(offered.is_ok(), "{offered:?}");
    assert_eq!(links.length(), 1, "one download");
    let link = links.get(0);
    let field = |k: &str| js_sys::Reflect::get(&link, &k.into()).unwrap_or(JsValue::UNDEFINED);
    assert_eq!(
        field("name").as_string().as_deref(),
        Some("lloyd - mix.wav")
    );
    let blob: web_sys::Blob = field("blob").dyn_into().expect("a Blob");
    assert_eq!((blob.type_(), blob.size()), ("audio/wav".to_string(), 4.0));
    assert!(
        !blob.is_instance_of::<web_sys::File>(),
        "D6: a Blob of the format's type, made from the File"
    );
    assert_eq!(no_links, 0, "a missing file offers nothing");
    assert!(kept, "the file stays while the download may read it");
    assert!(gone, "the timer removes the file");
    assert!(missing.is_err());
}

/// D1 of M8: each save of the page gets a new file name.
#[wasm_bindgen_test]
fn save_paths_are_new_for_each_save() {
    let (a, b) = (store::new_save_path(), store::new_save_path());
    assert!(
        a.starts_with("saves/") && b.starts_with("saves/"),
        "{a} {b}"
    );
    assert_ne!(a, b);
    let page = |p: &str| p.rsplit_once('-').map(|(page, _)| page.to_string());
    assert_eq!(page(&a), page(&b), "one page");
}
