//! Browser tests of the storage path of a save (M8 of
//! `plans/active/stream-from-storage.md`, `cargo run -p xtask -- web-test`):
//! the WAV save file that the page writes in blocks, and the lifetime of
//! save files.  The test plants a stored song and steps the save's job as
//! the page does.

#![expect(
    clippy::disallowed_methods,
    reason = "MEM-1: test code, which does not ship"
)]

use super::*;
use export::{ExportJob, Finish, SAVE_WRITE_BYTES, Sink, StoredWav};
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

#[wasm_bindgen(inline_js = r#"
export function sv_sleep(ms) { return new Promise((r) => setTimeout(r, ms)); }
// Keeps each download that the page offers, in place of the download:
// the link's name and the Blob of its object URL.  Once on, the spy stays
// on for the rest of the test binary: an offer is async, so a click can
// come after a test ends, and a real click saves a file in the user's
// downloads.
let links = [];
let spying = false;
export function sv_spy_links() {
  links = [];
  if (spying) return;
  spying = true;
  const blobs = new Map();
  const real_url = URL.createObjectURL;
  URL.createObjectURL = (b) => { const u = real_url(b); blobs.set(u, b); return u; };
  HTMLAnchorElement.prototype.click = function () { links.push({ name: this.download, blob: blobs.get(this.href) }); };
}
// The downloads offered since `sv_spy_links`, after the offers in flight.
export async function sv_links() {
  await new Promise((r) => setTimeout(r, 200));
  const out = links;
  links = [];
  return out;
}
"#)]
extern "C" {
    async fn sv_sleep(ms: u32);
    pub(crate) fn sv_spy_links();
    #[wasm_bindgen(js_name = sv_links)]
    async fn sv_links_js() -> JsValue;
}

/// The downloads that the page offered since [`sv_spy_links`], after the
/// offers in flight.
pub(crate) async fn sv_links() -> js_sys::Array {
    sv_links_js().await.unchecked_into()
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
/// file at `path`, as the page makes it: its reader reads ahead, one block
/// of the job at a time (D11 of M8).
fn mix_save(shown: &lanes::Shown, path: &str) -> ExportJob<StoredWav> {
    let recipe = MixRecipe::new(vec![1.0; shown.lanes.len()]);
    let source = lanes::LanesMix::reading_ahead(
        &shown.audio(),
        shown.frames,
        44_100,
        &recipe,
        export::BLOCK,
    );
    let sink = StoredWav::new(path.into(), shown.frames, 44_100, std::rc::Rc::new(|| {}));
    ExportJob::new(Box::new(source), sink).stopping_at_a_failure()
}

/// Steps `job` in slices of 4 ms, with a wait of a task between slices,
/// as the save's task loop does (D11 of M8), until it ends.  The loop
/// waits for a read's or a write's end; this one waits 1 ms.
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
    job.sink_mut().take();
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
    job.sink_mut().take();
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
    let kept = store::test_read("saves/3-1.wav").await.is_some();
    let links = sv_links().await;
    sv_sleep(200).await;
    let gone = store::test_read("saves/3-1.wav").await.is_none();
    sv_spy_links();
    let missing = store::download_file("lloyd.wav", "saves/4-1.wav", "audio/wav", 50).await;
    let no_links = sv_links().await.length();
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

// The page's saves through storage (step 3 of the M8 plan).

#[wasm_bindgen(inline_js = r#"
// Keeps each message posted to `worker`, with its transfer list.
export function sv_spy_posts(worker) {
  const posts = [];
  worker.postMessage = (data, transfer) => posts.push({ data, transfer: transfer || [] });
  return posts;
}
export function sv_deliver(worker, data) {
  worker.dispatchEvent(new MessageEvent("message", { data }));
}
export function sv_error_event() { return new Event("error"); }
"#)]
extern "C" {
    fn sv_spy_posts(worker: &web_sys::Worker) -> js_sys::Array;
    fn sv_deliver(worker: &web_sys::Worker, data: &JsValue);
    fn sv_error_event() -> web_sys::Event;
}

/// A worker that the test drives: what the page posts to it, and its
/// `onerror` handler, which stops it.
struct FakeWorker {
    posts: js_sys::Array,
    stop: js_sys::Function,
}

/// A page with no audio device that shows `shown` as session `id`, named
/// "lloyd", with a fake worker or none.
fn page(id: &str, shown: lanes::Shown, worker: bool) -> (StemApp, Option<FakeWorker>) {
    let mut app = StemApp {
        want_device: false,
        ..StemApp::default()
    };
    app.session = Some(id.into());
    app.open_song(shown);
    app.song_name = Some("lloyd".into());
    app.separator = None;
    let fake = worker.then(|| {
        let sep = separator::Separator::new(egui::Context::default()).expect("a worker");
        // The test page has no worker script, so its error would stop it.
        let stop = sep.worker().onerror().expect("an error handler");
        sep.worker().set_onerror(None);
        let posts = sv_spy_posts(sep.worker());
        app.separator = Some(sep);
        FakeWorker { posts, stop }
    });
    (app, fake)
}

/// Steps the page's save, with an await between frames, until it ends.
async fn run_save(app: &mut StemApp) {
    let ctx = egui::Context::default();
    for _ in 0..20_000 {
        if app.saving.is_none() {
            return;
        }
        app.step_save(&ctx);
        sv_sleep(1).await;
    }
    panic!("the save did not end");
}

/// Each download in `links`: its name, and its `Blob`.
fn downloads(links: &js_sys::Array) -> Vec<(String, web_sys::Blob)> {
    links
        .iter()
        .map(|link| {
            let field =
                |k: &str| js_sys::Reflect::get(&link, &k.into()).unwrap_or(JsValue::UNDEFINED);
            (
                field("name").as_string().unwrap_or_default(),
                field("blob").dyn_into().expect("a Blob"),
            )
        })
        .collect()
}

async fn bytes_of(blob: &web_sys::Blob) -> Vec<u8> {
    let buffer = wasm_bindgen_futures::JsFuture::from(blob.array_buffer())
        .await
        .expect("read");
    js_sys::Uint8Array::new(&buffer).to_vec()
}

/// Each request that the page posted to `worker` since the last call, with
/// its transfer list, as the worker reads it.
fn requests(worker: &FakeWorker) -> Vec<(stem_protocol::Request, js_sys::Array)> {
    let out = worker
        .posts
        .iter()
        .map(|post| {
            let field =
                |k: &str| js_sys::Reflect::get(&post, &k.into()).unwrap_or(JsValue::UNDEFINED);
            let request = stem_protocol::ToWorker::from_js(&field("data")).expect("a request");
            (request.request, field("transfer").unchecked_into())
        })
        .collect();
    worker.posts.set_length(0);
    out
}

/// Delivers `reply` from the worker, then a frame.
fn reply(app: &mut StemApp, reply: stem_protocol::Reply) {
    let sep = app.separator.as_ref().expect("a separator");
    sv_deliver(sep.worker(), &reply.to_js().0);
    app.poll_separator(&egui::Context::default());
}

/// The files in `saves/` after the page's spawned removals ran.
async fn save_files() -> String {
    sv_sleep(100).await;
    store::tests::names_in(store::SAVES)
        .await
        .unwrap_or_default()
}

/// D1 and D6 of M8: a WAV save of a stored song offers a `Blob` of its
/// save file, of type `audio/wav`, with the save's name, and says so.
#[wasm_bindgen_test]
async fn a_wav_save_of_a_stored_song_offers_its_save_file() {
    let id = "m8-offer-wav";
    let frames = 30 * SECOND;
    let (shown, want) = stored_song(id, frames, 3).await;
    store::test_remove(store::SAVES).await;
    let (mut app, _) = page(id, shown, false);
    sv_spy_links();
    app.start_save(None, export::Format::Wav);
    run_save(&mut app).await;
    let got = downloads(&sv_links().await);
    let status = app.status.clone();
    let files = save_files().await;
    let bytes = match got.first() {
        Some((_, blob)) => bytes_of(blob).await,
        None => Vec::new(),
    };
    app.load_demo();
    store::test_remove(store::SAVES).await;
    store::test_remove(&format!("sessions/{id}")).await;
    assert_eq!(got.len(), 1, "one download");
    let (name, blob) = &got[0];
    assert_eq!(name, "lloyd - mix.wav");
    assert_eq!(blob.type_(), "audio/wav", "D6: the format's type");
    assert!(bytes == want, "the download is the WAV of the mix");
    assert!(status.starts_with("Saved lloyd - mix.wav."), "{status}");
    assert!(
        files.ends_with(".wav") && !files.contains(' '),
        "the offered save file stays for its timer: {files}"
    );
}

/// D1, D4 and D5 of M8: a FLAC or MP3 save of a stored song posts a stored
/// request, with nothing transferred, only after its WAV is complete.
/// The worker's reply offers the worker's file, of the format's type.
#[wasm_bindgen_test]
async fn flac_and_mp3_saves_post_paths_then_offer_the_workers_file() {
    let id = "m8-offer-encoded";
    let frames = 10 * SECOND;
    let (shown, _) = stored_song(id, frames, 2).await;
    store::test_remove(store::SAVES).await;
    let (mut app, worker) = page(id, shown, true);
    let worker = worker.expect("a worker");
    let mut outcomes = Vec::new();
    for format in [export::Format::Flac, export::Format::Mp3] {
        app.start_save(Some(1), format);
        run_save(&mut app).await;
        let posted = requests(&worker);
        let encoding = app.status.clone();
        let (request, transfer) = posted.into_iter().next().expect("a request");
        let (name, wav, out, sent_frames) = match request {
            stem_protocol::Request::Flac { name, source, .. }
            | stem_protocol::Request::Mp3 { name, source, .. } => match source {
                stem_protocol::Encode::Stored {
                    wav, frames, out, ..
                } => (name, wav, out, frames),
                other => panic!("{other:?}"),
            },
            other => panic!("{other:?}"),
        };
        let wav_size = store::test_read(&wav).await.map(|b| b.len());
        store::test_write(&out, b"encoded").await.expect("planted");
        sv_spy_links();
        let name = name.expect("a name");
        let file = stem_protocol::Encoded::Stored { path: out.clone() };
        reply(
            &mut app,
            match format {
                export::Format::Flac => stem_protocol::Reply::Flac {
                    name: name.clone(),
                    file,
                },
                _ => stem_protocol::Reply::Mp3 {
                    name: name.clone(),
                    file,
                },
            },
        );
        sv_sleep(50).await;
        let got = downloads(&sv_links().await);
        outcomes.push((
            format,
            name,
            transfer.length(),
            sent_frames,
            wav_size,
            out,
            encoding,
            app.status.clone(),
            got,
        ));
    }
    app.load_demo();
    store::test_remove(store::SAVES).await;
    store::test_remove(&format!("sessions/{id}")).await;
    for (format, name, transfer, sent_frames, wav_size, out, encoding, status, got) in outcomes {
        let ext = format.extension();
        assert_eq!(name, format!("lloyd - drums.{ext}"));
        assert_eq!(transfer, 0, "{ext}: no samples cross");
        assert_eq!(sent_frames, frames);
        assert_eq!(
            wav_size,
            Some(44 + 4 * frames),
            "{ext}: the WAV is complete when the page posts"
        );
        assert!(out.starts_with("saves/") && out.ends_with(&format!(".{ext}")));
        assert!(encoding.starts_with("Encoding "), "{encoding}");
        assert!(status.starts_with(&format!("Saved {name}.")), "{status}");
        assert_eq!(got.len(), 1, "{ext}: one download");
        let (offered, blob) = &got[0];
        assert_eq!(offered, &name);
        let want_type = if format == export::Format::Flac {
            "audio/flac"
        } else {
            "audio/mpeg"
        };
        assert_eq!(
            (blob.type_(), blob.size()),
            (want_type.to_string(), 7.0),
            "D6: the worker's file, of the format's type"
        );
    }
}

/// D8 of M8: when browser storage refuses the save file's open or a write,
/// a song of 30 s saves on the memory path, and a song longer than
/// `MEMORY_PATH_MAX_FRAMES` makes no sink and says why.
#[wasm_bindgen_test]
async fn a_save_that_storage_refuses_is_held_in_memory_only_for_a_short_song() {
    let short = "m8-refused-short";
    let long = "m8-refused-long";
    let (short_song, want) = stored_song(short, 30 * SECOND, 2).await;
    let (long_song, _) = stored_song(long, MEMORY_PATH_MAX_FRAMES + SECOND, 1).await;
    let mut outcomes = Vec::new();
    for (id, shown) in [(short, short_song), (long, long_song)] {
        for how in ["open", "write"] {
            store::test_remove(store::SAVES).await;
            let (mut app, _) = page(id, shown.clone(), false);
            sv_spy_links();
            if how == "open" {
                store::tests::refuse_next_writable();
            } else {
                store::tests::start_spy(&JsValue::UNDEFINED, 1.0);
            }
            app.start_save(None, export::Format::Wav);
            run_save(&mut app).await;
            if how == "write" {
                store::tests::stop_spy();
            }
            let got = downloads(&sv_links().await);
            let bytes = match got.first() {
                Some((_, blob)) => bytes_of(blob).await,
                None => Vec::new(),
            };
            outcomes.push((
                id,
                how,
                got.len(),
                bytes == want,
                app.status.clone(),
                save_files().await,
            ));
            app.load_demo();
        }
    }
    store::test_remove(store::SAVES).await;
    store::test_remove(&format!("sessions/{short}")).await;
    store::test_remove(&format!("sessions/{long}")).await;
    for (id, how, links, same, status, files) in outcomes {
        assert!(files.is_empty(), "{id}, {how}: no save file stays: {files}");
        if id == short {
            assert_eq!(links, 1, "{how}: the short song saves");
            assert!(same, "{how}: the WAV of the mix");
            assert!(
                status.contains("held in memory, because browser storage failed"),
                "{how}: {status}"
            );
        } else {
            assert_eq!(links, 0, "{how}: the long song does not save");
            assert!(
                status.contains("browser storage could not hold the file")
                    && status.contains("Storage menu"),
                "{how}: {status}"
            );
        }
    }
}

/// Steps the page's save until a write of its save file is in flight.
async fn until_writing(app: &mut StemApp) {
    let ctx = egui::Context::default();
    for _ in 0..20_000 {
        app.step_save(&ctx);
        let saving = app.saving.as_ref().expect("a save");
        let writing = {
            let job = saving.job.borrow();
            job.fraction() > 0.0 && !job.sink().ready()
        };
        if writing {
            return;
        }
        sv_sleep(0).await;
    }
    panic!("no write was in flight");
}

/// D3 of M8: a new song during a save drops the save: no file stays, and
/// no download is offered.
#[wasm_bindgen_test]
async fn a_new_song_during_a_save_drops_it() {
    let id = "m8-new-song";
    let (shown, _) = stored_song(id, 30 * SECOND, 2).await;
    store::test_remove(store::SAVES).await;
    let (mut app, _) = page(id, shown, false);
    sv_spy_links();
    app.start_save(None, export::Format::Wav);
    until_writing(&mut app).await;
    app.load_demo();
    let dropped = app.saving.is_none();
    sv_sleep(200).await;
    let links = sv_links().await.length();
    let files = save_files().await;
    let writes = store::test_writes();
    store::test_remove(store::SAVES).await;
    store::test_remove(&format!("sessions/{id}")).await;
    assert!(dropped);
    assert_eq!(links, 0, "no download");
    assert_eq!(files, "", "no save file stays");
    assert_eq!(writes, 0, "no stream stays open");
}

/// D3 of M8: Reset during a save stops it, leaves no save file, and makes
/// no save on the memory path.
#[wasm_bindgen_test]
async fn reset_during_a_save_leaves_no_file_and_no_memory_save() {
    let id = "m8-reset-during-save";
    let (shown, _) = stored_song(id, 30 * SECOND, 2).await;
    store::test_remove(store::SAVES).await;
    let (mut app, _) = page(id, shown, false);
    sv_spy_links();
    app.start_save(None, export::Format::Wav);
    until_writing(&mut app).await;
    app.prepare_reset();
    store::close().await;
    let ctx = egui::Context::default();
    for _ in 0..20 {
        app.step_save(&ctx);
        sv_sleep(5).await;
    }
    let saving = app.saving.is_some();
    let links = sv_links().await.length();
    let files = save_files().await;
    store::test_reopen();
    store::test_remove(store::SAVES).await;
    store::test_remove(&format!("sessions/{id}")).await;
    assert!(!saving, "no save runs, in storage or in memory");
    assert_eq!(links, 0, "no download");
    assert_eq!(files, "", "no save file stays");
}

/// A FLAC save of the mix of `app`'s song, posted to `worker`: the output
/// file that the request names, and its name.
async fn posted_flac(app: &mut StemApp, worker: &FakeWorker) -> (String, String) {
    app.start_save(None, export::Format::Flac);
    run_save(app).await;
    match requests(worker).pop() {
        Some((
            stem_protocol::Request::Flac {
                name,
                source: stem_protocol::Encode::Stored { out, .. },
                ..
            },
            _,
        )) => (out, name.expect("a name")),
        other => panic!("{other:?}"),
    }
}

fn failed(name: &str, out: &str) -> stem_protocol::Reply {
    stem_protocol::Reply::Flac {
        name: name.into(),
        file: stem_protocol::Encoded::Failed {
            path: out.into(),
            text: "QuotaExceededError: the disk is full".into(),
        },
    }
}

/// The left channel that an arrays FLAC request sends, as floats.
fn left_of(request: &stem_protocol::Request) -> Vec<f32> {
    match request {
        stem_protocol::Request::Flac {
            source: stem_protocol::Encode::Arrays { left, .. },
            ..
        } => left.to_vec(),
        other => panic!("{other:?}"),
    }
}

/// D8 of M8: a stored encode that fails saves again on the memory path for
/// a song of 30 s, from the save's own source, also after a change of the
/// mix.  For a longer song, it says that storage could not hold the file.
#[wasm_bindgen_test]
async fn a_failed_stored_encode_saves_again_in_memory_only_for_a_short_song() {
    let id = "m8-failed-encode";
    let frames = 30 * SECOND;
    let (shown, want) = stored_song(id, frames, 2).await;
    store::test_remove(store::SAVES).await;
    let (mut app, worker) = page(id, shown, true);
    let worker = worker.expect("a worker");
    let (out, name) = posted_flac(&mut app, &worker).await;
    // The mix changes before the reply: the save keeps its own source.
    app.params.as_ref().expect("params").set_muted(0, true);
    reply(&mut app, failed(&name, &out));
    let again = app.saving.is_some();
    run_save(&mut app).await;
    let posted = requests(&worker);
    let status = app.status.clone();
    app.load_demo();

    let long = "m8-failed-encode-long";
    let (long_song, _) = stored_song(long, MEMORY_PATH_MAX_FRAMES + SECOND, 1).await;
    let (mut app, worker) = page(long, long_song, true);
    let worker = worker.expect("a worker");
    let (long_out, long_name) = posted_flac(&mut app, &worker).await;
    reply(&mut app, failed(&long_name, &long_out));
    let long_saving = app.saving.is_some();
    let long_status = app.status.clone();
    let long_posts = requests(&worker).len();
    app.load_demo();
    store::test_remove(store::SAVES).await;
    store::test_remove(&format!("sessions/{id}")).await;
    store::test_remove(&format!("sessions/{long}")).await;

    assert!(again, "a save on the memory path starts");
    assert_eq!(posted.len(), 1, "an arrays request");
    let left = left_of(&posted[0].0);
    assert_eq!(left.len(), frames);
    let wav_left = |k: usize| i16::from_le_bytes([want[44 + 4 * k], want[45 + 4 * k]]) as f32;
    assert!(
        left.iter()
            .enumerate()
            .all(|(k, x)| (x * 32768.0 - wav_left(k)).abs() <= 1.0),
        "the save's own mix, not the mix after the mute"
    );
    assert!(
        matches!(
            &posted[0].0,
            stem_protocol::Request::Flac {
                source: stem_protocol::Encode::Arrays {
                    rate: Some(44_100),
                    ..
                },
                ..
            }
        ),
        "nit 3-10: the arrays request's rate"
    );
    assert!(status.contains("held in memory"), "{status}");
    assert!(!long_saving, "no memory save of a long song");
    assert_eq!(long_posts, 0);
    assert!(
        long_status.contains("browser storage could not hold the file"),
        "{long_status}"
    );
}

/// D8 of M8: a failed stored encode that arrives while another save runs
/// waits for it, then saves again.  One that arrives after a new song only
/// says that the save failed.
#[wasm_bindgen_test]
async fn a_failed_encode_waits_for_the_save_in_progress_and_not_for_a_new_song() {
    let id = "m8-failed-waits";
    let (shown, _) = stored_song(id, 30 * SECOND, 2).await;
    store::test_remove(store::SAVES).await;
    let (mut app, worker) = page(id, shown.clone(), true);
    let worker = worker.expect("a worker");
    let (out, name) = posted_flac(&mut app, &worker).await;
    sv_spy_links();
    app.start_save(Some(0), export::Format::Wav);
    reply(&mut app, failed(&name, &out));
    let waiting = app.status.clone();
    let wav_save = app
        .saving
        .as_ref()
        .map(|s| s.order.format == export::Format::Wav);
    let ctx = egui::Context::default();
    for _ in 0..20_000 {
        let wav = app
            .saving
            .as_ref()
            .map(|s| s.order.format == export::Format::Wav);
        if wav != Some(true) {
            break;
        }
        app.step_save(&ctx);
        sv_sleep(1).await;
    }
    let wav_links = sv_links().await.length();
    let next = app.saving.as_ref().map(|s| s.order.name.clone());
    run_save(&mut app).await;
    let posted = requests(&worker);

    // A lane added: the same song, with other lanes.
    let (out, name) = posted_flac(&mut app, &worker).await;
    let frames = app.song.as_ref().expect("a song").frames;
    app.add_stem("classic drums", vec![0.0; frames], vec![0.0; frames]);
    reply(&mut app, failed(&name, &out));
    let after_new_lane = (app.saving.is_some(), app.status.clone());

    let (out, name) = posted_flac(&mut app, &worker).await;
    app.open_song(shown);
    reply(&mut app, failed(&name, &out));
    let after_new_song = (app.saving.is_some(), app.status.clone());
    app.load_demo();
    store::test_remove(store::SAVES).await;
    store::test_remove(&format!("sessions/{id}")).await;

    assert!(waiting.contains("after the save in progress"), "{waiting}");
    assert_eq!(wav_save, Some(true), "the WAV save goes on");
    assert_eq!(wav_links, 1, "the WAV save ends");
    assert_eq!(
        next.as_deref(),
        Some("lloyd - mix.flac"),
        "then the FLAC saves again"
    );
    assert_eq!(posted.len(), 1, "an arrays request");
    assert!(matches!(
        posted[0].0,
        stem_protocol::Request::Flac {
            source: stem_protocol::Encode::Arrays { .. },
            ..
        }
    ));
    let (saving, status) = after_new_lane;
    assert!(!saving, "no save after a lane was added");
    assert!(
        status.starts_with("Could not save lloyd - mix.flac: "),
        "{status}"
    );
    let (saving, status) = after_new_song;
    assert!(!saving, "no save after a new song");
    assert_eq!(
        status,
        "Could not save lloyd - mix.flac: QuotaExceededError: the disk is full."
    );
}

/// D7 and D8 of M8: when the worker stops, the page removes the WAV and
/// the output of each stored encode with no reply.  A stopped worker
/// counts as no worker: the menu offers no FLAC or MP3, and a save in
/// progress removes its WAV and posts nothing.
#[wasm_bindgen_test]
async fn a_stopped_worker_leaves_no_save_files_and_takes_no_encode() {
    let id = "m8-stopped";
    let (shown, _) = stored_song(id, 10 * SECOND, 2).await;
    store::test_remove(store::SAVES).await;
    let (mut app, worker) = page(id, shown, true);
    let worker = worker.expect("a worker");
    let (out, _) = posted_flac(&mut app, &worker).await;
    store::test_write(&out, b"part").await.expect("planted");
    let before = save_files().await;
    let could = app.can_encode();
    app.start_save(None, export::Format::Mp3);
    worker
        .stop
        .call1(&JsValue::NULL, &sv_error_event())
        .expect("stopped");
    app.poll_separator(&egui::Context::default());
    let can = app.can_encode();
    run_save(&mut app).await;
    let status = app.status.clone();
    let posts = requests(&worker).len();
    sv_sleep(500).await;
    let after = save_files().await;
    app.load_demo();
    store::test_remove(store::SAVES).await;
    store::test_remove(&format!("sessions/{id}")).await;
    assert_eq!(
        before.split(' ').count(),
        2,
        "a WAV and an output: {before}"
    );
    assert!(could && !can, "a stopped worker counts as no worker");
    assert!(
        status.contains("the encoding worker is not running"),
        "{status}"
    );
    assert_eq!(posts, 0, "no encode request");
    assert_eq!(after, "", "no save file stays");
}

/// Finding 1-1 of review round 1 of M8: a save dropped after its file
/// closed, but before the page took the file, removes the file.  A
/// `load_song` in the frame before `step_save` does that.
#[wasm_bindgen_test]
async fn a_closed_save_file_that_the_page_did_not_take_is_removed() {
    let id = "m8-closed-dropped";
    let (shown, _) = stored_song(id, 10 * SECOND, 2).await;
    store::test_remove(store::SAVES).await;
    let mut job = mix_save(&shown, SAVE);
    run_job(&mut job).await;
    let end = finish(&mut job).await;
    drop(job);
    let files = save_files().await;
    store::test_remove(store::SAVES).await;
    store::test_remove(&format!("sessions/{id}")).await;
    assert_eq!(end, Finish::Done, "the file closed");
    assert_eq!(files, "", "no save file stays");
}

/// Finding 1-2 of review round 1 of M8: a failed encode that waits for a
/// save in progress is reported, not saved, when the lanes change.  The
/// later save then keeps its own status.
#[wasm_bindgen_test]
async fn a_waiting_failed_encode_is_reported_when_the_lanes_change() {
    let id = "m8-waiting-lanes";
    let (shown, _) = stored_song(id, 10 * SECOND, 2).await;
    store::test_remove(store::SAVES).await;
    let (mut app, worker) = page(id, shown, true);
    let worker = worker.expect("a worker");
    let (out, name) = posted_flac(&mut app, &worker).await;
    sv_spy_links();
    app.start_save(Some(0), export::Format::Wav);
    reply(&mut app, failed(&name, &out));
    let waiting = app.save_queue.len();
    let frames = app.song.as_ref().expect("a song").frames;
    app.add_stem("classic drums", vec![0.0; frames], vec![0.0; frames]);
    let reported = (app.save_queue.len(), app.status.clone());
    run_save(&mut app).await;
    let after = app.status.clone();
    let posted = requests(&worker).len();
    let links = sv_links().await.length();
    app.load_demo();
    store::test_remove(store::SAVES).await;
    store::test_remove(&format!("sessions/{id}")).await;
    assert_eq!(waiting, 1, "the failed encode waits");
    let (queue, status) = reported;
    assert_eq!(queue, 0, "a lane change ends the wait");
    assert!(
        status.starts_with("Could not save lloyd - mix.flac: "),
        "{status}"
    );
    assert!(after.starts_with("Saved lloyd - bass.wav."), "{after}");
    assert_eq!(links, 1, "the WAV save");
    assert_eq!(posted, 0, "no FLAC saves again");
}

/// Finding 1-2 of review round 1 of M8: a failed encode that waits for a
/// save that fails is not left waiting.  The failed save goes on in memory,
/// and then the waiting encode saves again.
#[wasm_bindgen_test]
async fn a_waiting_failed_encode_is_not_left_when_the_save_in_progress_fails() {
    let id = "m8-waiting-failed";
    let (shown, _) = stored_song(id, 30 * SECOND, 2).await;
    store::test_remove(store::SAVES).await;
    let (mut app, worker) = page(id, shown, true);
    let worker = worker.expect("a worker");
    let (out, name) = posted_flac(&mut app, &worker).await;
    sv_spy_links();
    store::tests::start_spy(&JsValue::UNDEFINED, 1.0);
    app.start_save(Some(0), export::Format::Wav);
    reply(&mut app, failed(&name, &out));
    let waiting = app.save_queue.len();
    run_save(&mut app).await;
    store::tests::stop_spy();
    let left = (app.save_queue.len(), app.saving.is_some());
    let posted = requests(&worker);
    let links = sv_links().await.length();
    app.load_demo();
    store::test_remove(store::SAVES).await;
    store::test_remove(&format!("sessions/{id}")).await;
    assert_eq!(waiting, 1, "the failed encode waits");
    assert_eq!(left, (0, false), "nothing waits, and no save runs");
    assert_eq!(links, 1, "the WAV, saved in memory");
    assert!(
        matches!(
            posted.as_slice(),
            [(
                stem_protocol::Request::Flac {
                    source: stem_protocol::Encode::Arrays { .. },
                    ..
                },
                _
            )]
        ),
        "the FLAC saves again in memory: {posted:?}"
    );
}

/// Nit 1-10 of review round 1 of M8: a song of exactly
/// `MEMORY_PATH_MAX_FRAMES` frames saves on the memory path when storage
/// refuses its file.
#[wasm_bindgen_test]
async fn a_song_of_exactly_the_memory_path_limit_saves_in_memory() {
    let id = "m8-at-the-limit";
    let (shown, _) = stored_song(id, MEMORY_PATH_MAX_FRAMES, 1).await;
    store::test_remove(store::SAVES).await;
    let (mut app, _) = page(id, shown, false);
    sv_spy_links();
    store::tests::refuse_next_writable();
    app.start_save(None, export::Format::Wav);
    run_save(&mut app).await;
    let got = downloads(&sv_links().await);
    let status = app.status.clone();
    app.load_demo();
    store::test_remove(store::SAVES).await;
    store::test_remove(&format!("sessions/{id}")).await;
    assert_eq!(got.len(), 1, "it saves: {status}");
    assert_eq!(got[0].1.size(), (44 + 4 * MEMORY_PATH_MAX_FRAMES) as f64);
    assert!(status.contains("held in memory"), "{status}");
}

/// Steps the page's save for `frames` frames, with an await between them.
async fn step_frames(app: &mut StemApp, frames: usize) {
    let ctx = egui::Context::default();
    for _ in 0..frames {
        app.step_save(&ctx);
        sv_sleep(1).await;
    }
}

/// Finding 2-1 of review round 2 of M8: a save failure reported while
/// another save runs stays on the status line beside that save's progress,
/// and then beside its end.
#[wasm_bindgen_test]
async fn a_save_failure_stays_on_the_status_line_while_another_save_runs() {
    let id = "m8-report-stays";
    let (shown, _) = stored_song(id, 30 * SECOND, 2).await;
    store::test_remove(store::SAVES).await;
    let (mut app, worker) = page(id, shown.clone(), true);
    let worker = worker.expect("a worker");
    sv_spy_links();
    // A lane change while a failed encode waits.
    let (out, name) = posted_flac(&mut app, &worker).await;
    app.start_save(Some(0), export::Format::Wav);
    reply(&mut app, failed(&name, &out));
    let frames = app.song.as_ref().expect("a song").frames;
    app.add_stem("classic drums", vec![0.0; frames], vec![0.0; frames]);
    step_frames(&mut app, 3).await;
    let during_lanes = app.status.clone();
    run_save(&mut app).await;
    let end_lanes = app.status.clone();
    // A failed encode of an older song while a save runs.
    let (out, name) = posted_flac(&mut app, &worker).await;
    app.open_song(shown);
    app.song_name = Some("lloyd".into());
    app.start_save(Some(0), export::Format::Wav);
    reply(&mut app, failed(&name, &out));
    step_frames(&mut app, 3).await;
    let during_song = app.status.clone();
    run_save(&mut app).await;
    let end_song = app.status.clone();
    sv_links().await;
    app.load_demo();
    store::test_remove(store::SAVES).await;
    store::test_remove(&format!("sessions/{id}")).await;
    let report = "Could not save lloyd - mix.flac: QuotaExceededError: the disk is full.";
    for (when, status) in [
        ("during the save, after a lane change", &during_lanes),
        ("at the save's end, after a lane change", &end_lanes),
        ("during the save, after a new song", &during_song),
        ("at the save's end, after a new song", &end_song),
    ] {
        assert!(status.contains(report), "{when}: {status}");
    }
    assert!(
        end_lanes.starts_with("Saved lloyd - bass.wav."),
        "{end_lanes}"
    );
    assert!(
        end_song.starts_with("Saved lloyd - bass.wav."),
        "{end_song}"
    );
}

/// Nit 2-4 of review round 2 of M8: a failed encode of a song too long for
/// the memory path is reported at once, and does not wait.
#[wasm_bindgen_test]
async fn a_failed_encode_of_a_long_song_is_reported_at_once() {
    let id = "m8-long-report";
    let (shown, _) = stored_song(id, MEMORY_PATH_MAX_FRAMES + SECOND, 1).await;
    store::test_remove(store::SAVES).await;
    let (mut app, worker) = page(id, shown, true);
    let worker = worker.expect("a worker");
    let (out, name) = posted_flac(&mut app, &worker).await;
    sv_spy_links();
    app.start_save(Some(0), export::Format::Wav);
    reply(&mut app, failed(&name, &out));
    let queue = app.save_queue.len();
    step_frames(&mut app, 3).await;
    let status = app.status.clone();
    app.saving = None;
    sv_links().await;
    app.load_demo();
    store::test_remove(store::SAVES).await;
    store::test_remove(&format!("sessions/{id}")).await;
    assert_eq!(queue, 0, "it does not wait");
    assert!(
        status.contains("Could not save lloyd - mix.flac: browser storage could not hold the file")
            && !status.contains("saves again"),
        "{status}"
    );
}

/// Nit 2-12 of review round 2 of M8: a lane change drops what a posted
/// encode of the old lanes reads.
#[wasm_bindgen_test]
async fn a_lane_change_drops_the_source_of_a_posted_encode() {
    let id = "m8-posted-lanes";
    let (shown, _) = stored_song(id, 10 * SECOND, 2).await;
    store::test_remove(store::SAVES).await;
    let (mut app, worker) = page(id, shown, true);
    let worker = worker.expect("a worker");
    let (out, _) = posted_flac(&mut app, &worker).await;
    let kept = app.posted_saves[&out].order.is_some();
    let frames = app.song.as_ref().expect("a song").frames;
    app.add_stem("classic drums", vec![0.0; frames], vec![0.0; frames]);
    let dropped = app.posted_saves[&out].order.is_none();
    app.load_demo();
    store::test_remove(store::SAVES).await;
    store::test_remove(&format!("sessions/{id}")).await;
    assert!(kept, "a posted encode keeps its source");
    assert!(dropped, "a lane change drops it");
}

/// Finding 3-5 of review round 3 of M8: a save dropped while its last
/// write and its close are in flight leaves no file and no stream.
#[wasm_bindgen_test]
async fn a_save_dropped_during_its_close_leaves_no_file() {
    let id = "m8-drop-in-close";
    let (shown, _) = stored_song(id, 10 * SECOND, 2).await;
    store::test_remove(store::SAVES).await;
    let mut job = mix_save(&shown, SAVE);
    run_job(&mut job).await;
    for _ in 0..1_000 {
        if job.sink().ready() {
            break;
        }
        sv_sleep(1).await;
    }
    let first = job.sink_mut().finish();
    drop(job);
    writes_end().await;
    let writes = store::test_writes();
    let files = save_files().await;
    store::test_remove(store::SAVES).await;
    store::test_remove(&format!("sessions/{id}")).await;
    assert_eq!(first, Finish::Pending, "the close is in flight");
    assert_eq!(writes, 0, "no stream stays open");
    assert_eq!(files, "", "no save file stays");
}

/// A FLAC save of stem `stem`, or of the mix, posted to `worker`: the
/// output file that the request names, and its name.
async fn posted_flac_of(
    app: &mut StemApp,
    worker: &FakeWorker,
    stem: Option<usize>,
) -> (String, String) {
    app.start_save(stem, export::Format::Flac);
    run_save(app).await;
    match requests(worker).pop() {
        Some((
            stem_protocol::Request::Flac {
                name,
                source: stem_protocol::Encode::Stored { out, rate, .. },
            },
            _,
        )) => {
            assert_eq!(rate, 44_100, "nit 3-10: the stored request\x27s rate");
            (out, name.expect("a name"))
        }
        other => panic!("{other:?}"),
    }
}

/// Nit 3-6 of review round 3 of M8: two save failures reported while a save
/// runs each show once, also in the frame where that save moves to the
/// memory path and writes no progress.
#[wasm_bindgen_test]
async fn two_save_failures_each_show_once() {
    let id = "m8-two-reports";
    let (shown, _) = stored_song(id, 30 * SECOND, 2).await;
    store::test_remove(store::SAVES).await;
    let (mut app, worker) = page(id, shown, true);
    let worker = worker.expect("a worker");
    let (mix_out, mix_name) = posted_flac_of(&mut app, &worker, None).await;
    let (bass_out, bass_name) = posted_flac_of(&mut app, &worker, Some(0)).await;
    sv_spy_links();
    store::tests::refuse_next_writable();
    app.start_save(Some(1), export::Format::Wav);
    // The refused open ends before the next frame.
    sv_sleep(100).await;
    reply(&mut app, failed(&mix_name, &mix_out));
    reply(&mut app, failed(&bass_name, &bass_out));
    step_frames(&mut app, 1).await;
    let status = app.status.clone();
    app.saving = None;
    sv_links().await;
    app.load_demo();
    store::test_remove(store::SAVES).await;
    store::test_remove(&format!("sessions/{id}")).await;
    for name in [&mix_name, &bass_name] {
        let text = format!("Could not encode {name} in browser storage");
        assert_eq!(status.matches(&text).count(), 1, "{name}: {status}");
    }
}
