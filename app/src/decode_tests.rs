//! The page's open and restore through the decode worker, in a browser
//! (`cargo run -p xtask -- web-test`).  The decode worker is a fake: the
//! test plants the part files that the worker would write, and delivers
//! its replies.

use super::*;
use std::cell::RefCell;
use std::rc::Rc;
use stem_protocol::decode::{DecodeReply, DecodeRequest};
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

#[wasm_bindgen(inline_js = r#"
export function dt_spy(worker) {
  const posts = [];
  worker.postMessage = (data) => posts.push(data);
  return posts;
}
export function dt_deliver(worker, data) {
  worker.dispatchEvent(new MessageEvent("message", { data }));
}
export function dt_sleep(ms) { return new Promise((r) => setTimeout(r, ms)); }
export function dt_file(bytes, name) { return new File([bytes], name); }
// Counts each read of the bytes of an audio file (a `File` named
// *.wav, *.mp3 or *.flac), and each slice of one, which a read could go
// through, until stopped.  Reads of `meta` do not count.
let dt_reads = null;
let dt_real = null;
export function dt_start_read_spy() {
  const proto = Blob.prototype;
  dt_real = ["arrayBuffer", "text", "stream", "bytes", "slice"].map((k) => [k, proto[k]]);
  dt_reads = 0;
  for (const [k, f] of dt_real) {
    if (f) proto[k] = function (...a) {
      if (/\.(wav|mp3|flac)$/i.test(this.name || "")) dt_reads++;
      return f.apply(this, a);
    };
  }
}
export function dt_stop_read_spy() {
  for (const [k, f] of dt_real) if (f) Blob.prototype[k] = f;
  return dt_reads;
}
// Browser storage refuses every write until `dt_allow_writes`.
let dt_real_write = null;
export function dt_refuse_writes() {
  const proto = FileSystemWritableFileStream.prototype;
  dt_real_write = proto.write;
  proto.write = () => Promise.reject(new DOMException("refused by the test", "QuotaExceededError"));
}
export function dt_allow_writes() { FileSystemWritableFileStream.prototype.write = dt_real_write; }
// Counts the OPFS writes until `dt_writes_counted`.
let dt_counted = null;
let dt_real_counted = null;
export function dt_count_writes() {
  const proto = FileSystemWritableFileStream.prototype;
  dt_real_counted = proto.write;
  dt_counted = 0;
  proto.write = function (data) { dt_counted++; return dt_real_counted.call(this, data); };
}
export function dt_writes_counted() {
  FileSystemWritableFileStream.prototype.write = dt_real_counted;
  return dt_counted;
}
"#)]
extern "C" {
    fn dt_spy(worker: &web_sys::Worker) -> js_sys::Array;
    fn dt_deliver(worker: &web_sys::Worker, data: &JsValue);
    async fn dt_sleep(ms: u32);
    fn dt_file(bytes: &[u8], name: &str) -> web_sys::File;
    fn dt_start_read_spy();
    fn dt_stop_read_spy() -> u32;
    fn dt_refuse_writes();
    fn dt_allow_writes();
    fn dt_count_writes();
    fn dt_writes_counted() -> u32;
}

/// A decode worker with no script, which said `decode-alive`, and a spy
/// on what the page posts to it.
struct Fake {
    client: decoder::DecodeClient,
    worker: web_sys::Worker,
    posts: js_sys::Array,
}

impl Fake {
    fn new() -> Self {
        let client = decoder::DecodeClient::new("./no-such-decode-worker.js");
        let worker = client.worker().expect("a worker");
        worker.set_onerror(None);
        let posts = dt_spy(&worker);
        dt_deliver(&worker, &DecodeReply::Alive.to_js().0);
        Self {
            client,
            worker,
            posts,
        }
    }

    /// The `n`th request posted (from 0), once it is posted.
    async fn request(&self, n: u32) -> DecodeRequest {
        for _ in 0..250 {
            if self.posts.length() > n {
                break;
            }
            dt_sleep(20).await;
        }
        assert!(self.posts.length() > n, "no request {n} was posted");
        DecodeRequest::from_js(&self.posts.get(n)).expect("a decode request")
    }

    fn reply(&self, reply: DecodeReply) {
        dt_deliver(&self.worker, &reply.to_js().0);
    }
}

/// Runs `work` in the background; its output arrives in the cell.
fn background<T: 'static>(work: impl Future<Output = T> + 'static) -> Rc<RefCell<Option<T>>> {
    let out = Rc::new(RefCell::new(None));
    let slot = out.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let done = work.await;
        *slot.borrow_mut() = Some(done);
    });
    out
}

/// The output of a background task, once it arrives.
async fn output<T>(out: &Rc<RefCell<Option<T>>>) -> T {
    for _ in 0..250 {
        if out.borrow().is_some() {
            break;
        }
        dt_sleep(20).await;
    }
    out.borrow_mut().take().expect("the task ended")
}

/// A tone of `frames` frames, different for each `seed`.
fn tone(frames: usize, seed: usize) -> Vec<f32> {
    let step = 0.01 * (seed + 1) as f32;
    (0..frames).map(|i| 0.4 * (step * i as f32).sin()).collect()
}

/// A stereo 16-bit WAV file named `name` of `frames` frames.
fn wav_file(name: &str, frames: usize, seed: usize) -> (String, web_sys::File) {
    let wav = engine::wav::encode_pcm16(&tone(frames, seed), &tone(frames, seed + 7), 44_100);
    (name.to_string(), dt_file(&wav, name))
}

fn interleaved(left: &[f32], right: &[f32]) -> Vec<u8> {
    left.iter()
        .zip(right)
        .flat_map(|(l, r)| [l.to_le_bytes(), r.to_le_bytes()])
        .flatten()
        .collect()
}

/// The samples that the worker decodes for each source of `req`, in its
/// order: a tone for each, with the right side alike for `mono`.
fn planted(req: &DecodeRequest, frames: usize, mono: &[bool]) -> Vec<(Vec<f32>, Vec<f32>)> {
    req.sources
        .iter()
        .zip(mono)
        .map(|(s, mono)| {
            let left = tone(frames, s.index as usize);
            let right = if *mono {
                left.clone()
            } else {
                tone(frames, s.index as usize + 3)
            };
            (left, right)
        })
        .collect()
}

/// Plants the part files of a decode of `req`: each source's samples in
/// its session file, and overviews of the right size.  With `new_mix`
/// false, no `mix.f32` part (D8).
async fn plant_parts(
    req: &DecodeRequest,
    frames: usize,
    samples: &[(Vec<f32>, Vec<f32>)],
    new_mix: bool,
) {
    let one = req.sources.iter().filter(|s| s.in_mix).count() == 1;
    for (s, (left, right)) in req.sources.iter().zip(samples) {
        let file = if one && s.in_mix {
            "mix.f32".to_string()
        } else {
            format!("decoded/{}.f32", s.index)
        };
        if file == "mix.f32" && !new_mix {
            continue;
        }
        let path = format!("{}/{file}.decode.part", req.session);
        store::test_write(&path, &interleaved(left, right))
            .await
            .expect("planted");
    }
    if new_mix && !one {
        let path = format!("{}/mix.f32.decode.part", req.session);
        store::test_write(&path, &vec![0; 8 * frames])
            .await
            .expect("planted");
    }
    let columns = engine::overview::column_count(frames);
    let names = req
        .sources
        .iter()
        .map(|s| s.index.to_string())
        .chain(["mix".to_string()]);
    for name in names {
        let path = format!("{}/overviews/{name}.f32.decode.part", req.session);
        store::test_write(&path, &vec![0; 8 * columns])
            .await
            .expect("planted");
    }
}

/// The id of a session folder, `sessions/<id>`.
fn id_of(session: &str) -> String {
    session.trim_start_matches("sessions/").to_string()
}

async fn meta_of(id: &str) -> Option<store_meta::Meta> {
    store::list()
        .await
        .into_iter()
        .find(|(got, _)| got == id)
        .map(|(_, meta)| meta)
}

/// Opens `chosen` through the fake worker, which decodes `frames` frames
/// of each source, `mono` or not, and gives the open's result.
async fn open_decoded(
    fake: &Fake,
    chosen: Chosen,
    frames: usize,
    mono: &[bool],
) -> (
    anyhow::Result<Opened>,
    DecodeRequest,
    Vec<(Vec<f32>, Vec<f32>)>,
) {
    let client = fake.client.clone();
    let opened = background(async move { open_chosen_now(&chosen, &client).await });
    let req = fake.request(fake.posts.length()).await;
    let samples = planted(&req, frames, mono);
    plant_parts(&req, frames, &samples, true).await;
    fake.reply(DecodeReply::Decoded {
        frames: frames as u64,
        new_mix: true,
        channels: mono.iter().map(|m| if *m { 1 } else { 2 }).collect(),
    });
    (output(&opened).await, req, samples)
}

#[wasm_bindgen_test]
async fn an_open_posts_a_decode_and_reads_no_source() {
    store::test_wipe().await;
    let fake = Fake::new();
    let chosen = vec![wav_file("Lloyd - Tune.wav", 2_000, 0)];
    dt_start_read_spy();
    let client = fake.client.clone();
    let opened = background(async move { open_chosen_now(&chosen, &client).await });
    let req = fake.request(0).await;
    let reads = dt_stop_read_spy();
    fake.reply(DecodeReply::Undecodable {
        text: "the test ends it".into(),
    });
    let _ = output(&opened).await;
    store::test_wipe().await;
    assert_eq!(reads, 0, "the page read no source to open it");
    assert!(req.session.starts_with("sessions/"), "{}", req.session);
    assert_eq!(
        req.sources,
        [stem_protocol::decode::Source {
            file: "src/0.wav".into(),
            index: 0,
            ext: "wav".into(),
            in_mix: true,
        }]
    );
    assert_eq!(req.mix_frames, None);
}

#[wasm_bindgen_test]
async fn a_drop_of_no_source_fails_before_any_post() {
    store::test_wipe().await;
    let fake = Fake::new();
    let chosen = vec![wav_file("mix.wav", 100, 0)];
    let opened = open_chosen_now(&chosen, &fake.client).await;
    let sessions = store::list().await.len();
    store::test_wipe().await;
    let error = opened.err().map(|e| format!("{e:#}"));
    assert_eq!(error.as_deref(), Some("no stems to load"));
    assert_eq!(fake.posts.length(), 0, "nothing posted");
    assert_eq!(sessions, 0, "nothing saved");
}

#[wasm_bindgen_test]
async fn a_decoded_song_is_read_back_and_shown() {
    let frames = 3_001;
    // One source, then three: drums, bass and vocals, in the order of
    // `Song::from_audio`, with bass mono.
    let cases: [(Chosen, Vec<bool>, Vec<&str>); 2] = [
        (
            vec![wav_file("acme.wav", 100, 0)],
            vec![false],
            vec!["acme"],
        ),
        (
            vec![
                wav_file("bass.wav", 100, 0),
                wav_file("drums.wav", 100, 1),
                wav_file("vocals.wav", 100, 2),
            ],
            vec![false, true, false],
            vec!["drums", "bass", "vocals"],
        ),
    ];
    for (chosen, mono, names) in cases {
        store::test_wipe().await;
        let fake = Fake::new();
        let (opened, req, samples) = open_decoded(&fake, chosen, frames, &mono).await;
        let opened = opened.expect("opened");
        let id = id_of(&req.session);
        let meta = meta_of(&id).await.expect("meta");
        let parts = store::list().await.len();
        let mut left_over = Vec::new();
        for s in &req.sources {
            let path = format!("{}/decoded/{}.f32.decode.part", req.session, s.index);
            left_over.extend(store::test_read(&path).await.map(|_| path));
        }
        store::test_wipe().await;
        let song = &opened.song.as_song().expect("in memory");
        let got: Vec<&str> = song.stems.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(got, names);
        assert_eq!(
            (song.frames, opened.session.as_deref()),
            (frames, Some(id.as_str()))
        );
        for ((stem, (left, right)), mono) in song.stems.iter().zip(&samples).zip(&mono) {
            assert!(*stem.left == **left, "{}: left", stem.name);
            assert!(*stem.right == **right, "{}: right", stem.name);
            assert_eq!(
                stem.left.shares(&stem.right),
                *mono,
                "{}: shared",
                stem.name
            );
        }
        let decoded = meta.decoded.expect("the decoded line");
        assert_eq!(decoded.frames, frames);
        assert_eq!(meta.mix, Some((frames, store_meta::MIX_FORMAT.into())));
        assert_eq!(parts, 1, "one session");
        assert_eq!(left_over, Vec::<String>::new(), "every part file moved");
    }
}

/// D1, D6: a derived source stays out of the mix, so `drums.wav` and
/// `classic drums.wav` read back as a song of one source.
#[wasm_bindgen_test]
async fn a_derived_source_stays_out_of_the_mix() {
    store::test_wipe().await;
    let fake = Fake::new();
    let chosen = vec![
        wav_file("classic drums.wav", 100, 0),
        wav_file("drums.wav", 100, 1),
    ];
    let (opened, req, samples) = open_decoded(&fake, chosen, 2_000, &[false, false]).await;
    let mix = store::test_read(&format!("{}/mix.f32", req.session)).await;
    store::test_wipe().await;
    let in_mix: Vec<(u32, bool)> = req.sources.iter().map(|s| (s.index, s.in_mix)).collect();
    assert_eq!(
        in_mix,
        [(1, true), (0, false)],
        "drums first, classic drums out of the mix"
    );
    let song = opened.expect("opened").song.as_song().expect("in memory");
    assert_eq!(song.stems[0].name, "drums");
    assert_eq!(song.stems[1].name, "classic drums");
    assert_eq!(
        mix,
        Some(interleaved(&samples[0].0, &samples[0].1)),
        "the mix is drums"
    );
}

/// A session of the song on screen, made current, as a reload restores it.
async fn a_current_session() -> String {
    let before = saved(&vec![wav_file("acme.wav", 100, 3)]).await;
    store::make_current(&before).await;
    before
}

/// D9, finding 2-3 of review round 2: an undecodable new song forgets its
/// session, and the session of the song on screen stays current.
#[wasm_bindgen_test]
async fn undecodable_shows_the_error_and_forgets_the_session() {
    store::test_wipe().await;
    let fake = Fake::new();
    let before = a_current_session().await;
    let chosen = vec![wav_file("lloyd.wav", 100, 0)];
    let client = fake.client.clone();
    let opened = background(async move { open_chosen_now(&chosen, &client).await });
    let req = fake.request(0).await;
    fake.reply(DecodeReply::Undecodable {
        text: "unsupported or unrecognised format: lloyd".into(),
    });
    let opened = output(&opened).await;
    let kept = meta_of(&id_of(&req.session)).await;
    let current = store::current().await;
    store::test_wipe().await;
    let error = opened.err().map(|e| format!("{e:#}"));
    assert_eq!(
        error.as_deref(),
        Some("unsupported or unrecognised format: lloyd")
    );
    assert_eq!(kept, None, "the new session is forgotten");
    assert_eq!(current, Some(before), "sessions/current is unchanged");
}

/// D9, finding 2-3: a new song that the page then cannot decode either
/// forgets its session, and `sessions/current` is unchanged.
#[wasm_bindgen_test]
async fn a_failed_page_decode_forgets_the_new_session() {
    store::test_wipe().await;
    let fake = Fake::new();
    let before = a_current_session().await;
    let chosen = vec![("lloyd.wav".to_string(), dt_file(&[1; 64], "lloyd.wav"))];
    let client = fake.client.clone();
    let opened = background(async move { open_chosen_now(&chosen, &client).await });
    let req = fake.request(0).await;
    fake.reply(DecodeReply::Unstored {
        text: "QuotaExceededError: the disk is full".into(),
    });
    let opened = output(&opened).await;
    let kept = meta_of(&id_of(&req.session)).await;
    let current = store::current().await;
    store::test_wipe().await;
    assert!(opened.is_err(), "the page cannot decode it");
    assert_eq!(kept, None, "the new session is forgotten");
    assert_eq!(current, Some(before), "sessions/current is unchanged");
}

/// D9, finding 2-3: a new session becomes current only once its song
/// opens, so a reload during the decode restores the song on screen.
#[wasm_bindgen_test]
async fn a_new_session_becomes_current_once_its_song_opens() {
    store::test_wipe().await;
    let fake = Fake::new();
    let before = a_current_session().await;
    let client = fake.client.clone();
    let chosen = vec![wav_file("lloyd.wav", 100, 0)];
    let opened = background(async move { open_chosen_now(&chosen, &client).await });
    let req = fake.request(0).await;
    let during = store::current().await;
    let frames = 1_000;
    let samples = planted(&req, frames, &[false]);
    plant_parts(&req, frames, &samples, true).await;
    fake.reply(DecodeReply::Decoded {
        frames: frames as u64,
        new_mix: true,
        channels: vec![2],
    });
    let opened = output(&opened).await;
    let after = store::current().await;
    store::test_wipe().await;
    assert!(opened.is_ok(), "opened");
    assert_eq!(during, Some(before), "not current during the decode");
    assert_eq!(after, Some(id_of(&req.session)), "current once it opens");
}

#[wasm_bindgen_test]
async fn unstored_decodes_on_the_page_and_keeps_the_session() {
    store::test_wipe().await;
    let fake = Fake::new();
    let chosen = vec![wav_file("lloyd.wav", 1_234, 0)];
    let client = fake.client.clone();
    let opened = background(async move { open_chosen_now(&chosen, &client).await });
    let req = fake.request(0).await;
    fake.reply(DecodeReply::Unstored {
        text: "QuotaExceededError: the disk is full".into(),
    });
    let opened = output(&opened).await.expect("opened on the page");
    let kept = meta_of(&id_of(&req.session)).await;
    let current = store::current().await;
    store::test_wipe().await;
    assert_eq!(opened.song.frames, 1_234);
    assert_eq!(current, Some(id_of(&req.session)), "made current");
    assert_eq!(
        opened.session,
        Some(id_of(&req.session)),
        "the session stays"
    );
    assert!(kept.is_some(), "its files stay");
    let note = opened.note.unwrap_or_default();
    assert!(
        note.contains("browser storage failed") && note.contains("disk is full"),
        "{note}"
    );
}

#[wasm_bindgen_test]
async fn no_session_decodes_on_the_page() {
    store::test_wipe().await;
    let fake = Fake::new();
    let chosen = vec![wav_file("lloyd.wav", 1_000, 0)];
    dt_refuse_writes();
    let opened = open_chosen_now(&chosen, &fake.client).await;
    dt_allow_writes();
    store::test_wipe().await;
    let opened = opened.expect("opened on the page");
    assert_eq!((opened.song.frames, opened.session), (1_000, None));
    assert_eq!(fake.posts.length(), 0, "no decode is posted");
}

/// A session saved from `chosen`, and the sources the worker gets for it.
async fn saved(chosen: &Chosen) -> String {
    let files: Vec<(String, web_sys::Blob)> = chosen
        .iter()
        .map(|(n, f)| (n.clone(), web_sys::Blob::from(f.clone())))
        .collect();
    store::save_sources(&files, 44_100).await.expect("saved")
}

#[wasm_bindgen_test]
async fn undecodable_in_a_restore_keeps_the_session() {
    store::test_wipe().await;
    let fake = Fake::new();
    let id = saved(&vec![wav_file("lloyd.wav", 100, 0)]).await;
    let client = fake.client.clone();
    let restore_id = id.clone();
    let restored = background(async move { restore_stored(&restore_id, &client).await });
    let _ = fake.request(0).await;
    fake.reply(DecodeReply::Undecodable {
        text: "file decoded to no audio".into(),
    });
    let restored = output(&restored).await.expect("a session");
    let kept = meta_of(&id).await;
    store::test_wipe().await;
    let error = restored.err().map(|e| format!("{e:#}"));
    assert_eq!(error.as_deref(), Some("file decoded to no audio"));
    assert!(kept.is_some(), "the session stays");
}

#[wasm_bindgen_test]
async fn a_complete_session_restores_with_no_decode() {
    store::test_wipe().await;
    let fake = Fake::new();
    let (opened, req, samples) =
        open_decoded(&fake, vec![wav_file("acme.wav", 100, 0)], 2_500, &[false]).await;
    opened.expect("opened");
    let id = id_of(&req.session);
    let posts = fake.posts.length();
    let restored = restore_stored(&id, &fake.client).await;
    let current = store::current().await;
    store::test_wipe().await;
    let (opened, meta, note) = restored.expect("a session").expect("restored");
    let song = opened.song.as_song().expect("in memory");
    assert_eq!(fake.posts.length(), posts, "no decode is posted");
    assert_eq!((note, meta.name.as_str()), (None, "acme"));
    assert!(*song.stems[0].left == samples[0].0 && *song.stems[0].right == samples[0].1);
    assert_eq!(current, Some(id), "made current");
}

/// D8, D11: a session of M4 (no `decoded` line) with a usable mix restores
/// through a decode that keeps the mix.
#[wasm_bindgen_test]
async fn an_incomplete_session_restores_through_a_decode() {
    store::test_wipe().await;
    let fake = Fake::new();
    let frames = 2_222;
    let id = saved(&vec![wav_file("acme.wav", frames, 0)]).await;
    let (left, right) = (tone(frames, 0), tone(frames, 9));
    let mix = interleaved(&left, &right);
    store::test_write(&store::mix_path(&id), &mix)
        .await
        .expect("planted");
    let mut meta = meta_of(&id).await.expect("meta");
    meta.mix = Some((frames, store_meta::MIX_FORMAT.into()));
    store::test_write(&format!("sessions/{id}/meta"), meta.to_text().as_bytes())
        .await
        .expect("planted");
    let client = fake.client.clone();
    let restore_id = id.clone();
    let restored = background(async move { restore_stored(&restore_id, &client).await });
    let req = fake.request(0).await;
    plant_parts(&req, frames, &planted(&req, frames, &[false]), false).await;
    fake.reply(DecodeReply::Decoded {
        frames: frames as u64,
        new_mix: false,
        channels: vec![2],
    });
    let restored = output(&restored)
        .await
        .expect("a session")
        .expect("restored");
    let meta = meta_of(&id).await.expect("meta");
    let kept_mix = store::test_read(&store::mix_path(&id)).await;
    store::test_wipe().await;
    assert_eq!(
        req.mix_frames,
        Some(frames as u64),
        "the usable mix is named"
    );
    let song = restored.0.song.as_song().expect("in memory");
    assert!(
        *song.stems[0].left == left && *song.stems[0].right == right,
        "read from the mix"
    );
    assert_eq!(kept_mix, Some(mix), "the mix stays");
    assert_eq!(meta.decoded.map(|d| d.frames), Some(frames), "now complete");
    assert_eq!(meta.mix, Some((frames, store_meta::MIX_FORMAT.into())));
}

/// D9, finding 1-3 of review round 1: the reset path before `reset::run`
/// ends the decode worker first, so the decode ends, its guard goes, and
/// `store::close` ends.
#[wasm_bindgen_test]
async fn a_reset_during_a_decode_completes() {
    store::test_wipe().await;
    let fake = Fake::new();
    let mut app = StemApp {
        want_device: false,
        ..StemApp::default()
    };
    app.decoder = fake.client.clone();
    let id = saved(&vec![wav_file("acme.wav", 100, 0)]).await;
    let request = stem_protocol::decode::DecodeRequest {
        session: store::session_folder(&id),
        sources: Vec::new(),
        mix_frames: None,
    };
    let ended = background(app.decoder.decode(&request, store::DecodeGuard::new(&id)));
    let held = (store::decoding(&id), store::writes_in_progress());
    app.prepare_reset();
    let closed = background(store::close());
    let mut waited = 0;
    while closed.borrow().is_none() && waited < 3_000 {
        dt_sleep(20).await;
        waited += 20;
    }
    let close_ended = closed.borrow().is_some();
    let decode_ended = ended.borrow().is_some();
    // Only then storage opens again, so the other tests can write.
    store::test_reopen();
    store::test_wipe().await;
    assert_eq!(held, (true, 1), "the decode holds a write");
    assert!(decode_ended, "the decode ends");
    assert!(close_ended, "store::close ends");
}

/// Keeps each log line of the page, with the session that
/// `store::make_current` had made current when the line was logged.
struct TestLog;

thread_local! {
    static LINES: RefCell<Vec<(String, Option<String>)>> = const { RefCell::new(Vec::new()) };
}

impl log::Log for TestLog {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        let line = record.args().to_string();
        LINES.with_borrow_mut(|lines| lines.push((line, store::test_made_current())));
    }

    fn flush(&self) {}
}

static TEST_LOG: TestLog = TestLog;

/// Starts to keep the log lines, from none.
fn start_log() {
    if log::set_logger(&TEST_LOG).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
    LINES.with_borrow_mut(Vec::clear);
}

/// The lines kept since [`start_log`] that hold `text`.
fn logged(text: &str) -> Vec<(String, Option<String>)> {
    LINES.with_borrow(|lines| {
        lines
            .iter()
            .filter(|(line, _)| line.contains(text))
            .cloned()
            .collect()
    })
}

/// D12, finding 3-1 of review round 3: the page logs "song kept in browser
/// storage" only once `sessions/current` names the session, so an e2e
/// check can reload at that line.  Through the decode worker, and on the
/// page.
#[wasm_bindgen_test]
async fn the_kept_line_comes_once_the_session_is_current() {
    for unstored in [false, true] {
        store::test_wipe().await;
        let fake = Fake::new();
        start_log();
        let client = fake.client.clone();
        let chosen = vec![wav_file("lloyd.wav", 1_000, 0)];
        let opened = background(async move { open_chosen_now(&chosen, &client).await });
        let req = fake.request(0).await;
        if unstored {
            fake.reply(DecodeReply::Unstored {
                text: "QuotaExceededError: the disk is full".into(),
            });
        } else {
            let samples = planted(&req, 1_000, &[false]);
            plant_parts(&req, 1_000, &samples, true).await;
            fake.reply(DecodeReply::Decoded {
                frames: 1_000,
                new_mix: true,
                channels: vec![2],
            });
        }
        let opened = output(&opened).await;
        let lines = logged("song kept in browser storage");
        store::test_wipe().await;
        let id = id_of(&req.session);
        assert!(opened.is_ok(), "unstored {unstored}: opened");
        assert_eq!(
            lines,
            [(
                format!("song kept in browser storage as session {id}"),
                Some(id.clone())
            )],
            "unstored {unstored}"
        );
    }
}

/// D9, finding 3-3 of review round 3: a reset during an open or a restore
/// ends it with an error.  The page reads no file and decodes nothing on
/// its main thread while browser storage is reset.
#[wasm_bindgen_test]
async fn a_reset_ends_an_open_and_a_restore_with_no_page_decode() {
    store::test_wipe().await;
    start_log();
    let mut app = StemApp {
        want_device: false,
        ..StemApp::default()
    };
    let fake = Fake::new();
    app.decoder = fake.client.clone();
    let client = fake.client.clone();
    let chosen = vec![wav_file("lloyd.wav", 100, 0)];
    let opened = background(async move { open_chosen_now(&chosen, &client).await });
    fake.request(0).await;
    dt_start_read_spy();
    app.prepare_reset();
    let opened = output(&opened).await;
    let open_reads = dt_stop_read_spy();

    let id = saved(&vec![wav_file("acme.wav", 100, 0)]).await;
    let fake = Fake::new();
    app.decoder = fake.client.clone();
    let client = fake.client.clone();
    let restored = background(async move { restore_stored(&id, &client).await });
    fake.request(0).await;
    dt_start_read_spy();
    app.prepare_reset();
    let restored = output(&restored).await;
    let restore_reads = dt_stop_read_spy();
    let page_lines = logged("decoder: decoding on the page");
    store::test_wipe().await;
    assert!(opened.is_err(), "the open ends with an error");
    assert!(
        matches!(restored, Some(Err(_))),
        "the restore ends with an error"
    );
    assert_eq!((open_reads, restore_reads), (0, 0), "no file is read");
    assert_eq!(page_lines, [], "no decode on the page");
}

/// D8: after a decoded song of one source, Separate posts the stored mix
/// and writes no mix file.
#[wasm_bindgen_test]
async fn separate_writes_no_mix_after_a_decode() {
    store::test_wipe().await;
    let fake = Fake::new();
    let frames = 4_410;
    let (opened, req, _) =
        open_decoded(&fake, vec![wav_file("acme.wav", 100, 0)], frames, &[false]).await;
    let opened = opened.expect("opened");
    let mut sep = separator::Separator::new(egui::Context::default()).expect("a worker");
    sep.worker().set_onerror(None);
    sep.model_ready = true;
    sep.model_loading = false;
    let posts = dt_spy(sep.worker());
    let mut app = StemApp {
        want_device: false,
        ..StemApp::default()
    };
    app.separator = Some(sep);
    app.session = opened.session.clone();
    app.open_song(opened.song);
    dt_count_writes();
    app.start_separation(&egui::Context::default());
    for _ in 0..250 {
        if app.pending.borrow().separation_start.is_some() {
            break;
        }
        dt_sleep(20).await;
    }
    app.apply_inbox(&egui::Context::default());
    let writes = dt_writes_counted();
    let sent = posts.get(0);
    if let Some(s) = &mut app.separator {
        s.stop();
    }
    store::test_wipe().await;
    assert_eq!(writes, 0, "no mix file is written");
    let request = stem_protocol::ToWorker::from_js(&sent)
        .expect("a request")
        .request;
    match request {
        stem_protocol::Request::Separate {
            mix: stem_protocol::Mix::Stored { path, frames: f },
            ..
        } => {
            assert_eq!(path, format!("{}/mix.f32", req.session));
            assert_eq!(f, frames);
        }
        other => panic!("{other:?}"),
    }
}

/// D8: a separation that may read or write the mix names its session for
/// D7, and one on the memory path does not.
#[wasm_bindgen_test]
fn the_page_names_the_session_of_its_separation_for_d7() {
    let mut app = StemApp::default();
    let job = |stage| SeparationJob {
        serial: 0,
        session: Some("1792640000060".into()),
        name: None,
        stage,
        keeps_stems: true,
    };
    let mut named = Vec::new();
    for stage in [
        JobStage::Starting,
        JobStage::WritingMix,
        JobStage::Storage,
        JobStage::Memory,
    ] {
        app.separating = Some(job(stage));
        app.publish_separating();
        named.push(store::mix_in_use("1792640000060"));
    }
    app.separating = None;
    app.publish_separating();
    named.push(store::mix_in_use("1792640000060"));
    assert_eq!(named, [true, true, true, false, false]);
}

/// D8: Separate waits while a decode of the shown song's session runs.
#[wasm_bindgen_test]
async fn separate_waits_for_a_decode_of_the_shown_session() {
    store::test_wipe().await;
    let id = saved(&vec![wav_file("acme.wav", 100, 0)]).await;
    let mut app = StemApp {
        want_device: false,
        ..StemApp::default()
    };
    app.open_memory_song(Song {
        sample_rate: 44_100,
        frames: 100,
        stems: vec![Stem {
            name: "acme".into(),
            left: tone(100, 0).into(),
            right: tone(100, 1).into(),
        }],
    });
    app.session = Some(id.clone());
    let guard = store::DecodeGuard::new(&id);
    app.start_separation(&egui::Context::default());
    let during = app.separating.is_some();
    drop(guard);
    app.start_separation(&egui::Context::default());
    let after = app.separating.is_some();
    app.separating = None;
    store::test_wipe().await;
    assert!(!during, "no separation starts during the decode");
    assert!(after, "it starts once the decode ends");
}

/// Restores an M4 session of `frames` frames with a usable mix through the
/// fake worker, which ends the decode with `decode-unstored`.  Gives the
/// session's `mix` line and `mix.f32` after the restore.
/// It also gives whether `mix.f32` was there while the open held its guard
/// (D10 of M6), and waits for a removal that the guard held back.
async fn unstored_with_a_kept_mix(
    separating: bool,
) -> (Option<(usize, String)>, Option<Vec<u8>>, bool) {
    store::test_wipe().await;
    let fake = Fake::new();
    let frames = 1_500;
    let id = saved(&vec![wav_file("acme.wav", frames, 0)]).await;
    let mix = interleaved(&tone(frames, 0), &tone(frames, 9));
    store::test_write(&store::mix_path(&id), &mix)
        .await
        .expect("planted");
    let mut meta = meta_of(&id).await.expect("meta");
    meta.mix = Some((frames, store_meta::MIX_FORMAT.into()));
    store::test_write(&format!("sessions/{id}/meta"), meta.to_text().as_bytes())
        .await
        .expect("planted");
    if separating {
        store::set_separating(Some(id.clone()));
    }
    let client = fake.client.clone();
    let restore_id = id.clone();
    let restored = background(async move { restore_stored(&restore_id, &client).await });
    let req = fake.request(0).await;
    assert_eq!(req.mix_frames, Some(frames as u64));
    fake.reply(DecodeReply::Unstored {
        text: "the sources have 1500 frames, not the kept mix's".into(),
    });
    let restored = output(&restored).await.expect("a session");
    let kept_while_open = store::test_read(&store::mix_path(&id)).await.is_some();
    // The song is shown: the open's guard goes.
    let ok = restored.is_ok();
    drop(restored);
    store::test_settled(&id).await;
    store::set_separating(None);
    store::test_settled(&id).await;
    let after = meta_of(&id).await.and_then(|m| m.mix);
    let file = store::test_read(&store::mix_path(&id)).await;
    store::test_wipe().await;
    assert!(ok, "the page decodes the song itself");
    (after, file, kept_while_open)
}

/// D8, finding 1-4 of review round 1: on `decode-unstored` with a kept
/// mix, the page removes the `mix` line and `mix.f32`, so the next open
/// decodes again.
#[wasm_bindgen_test]
async fn unstored_with_a_kept_mix_removes_it() {
    let (line, file, _) = unstored_with_a_kept_mix(false).await;
    assert_eq!(line, None, "the mix line goes");
    assert_eq!(file, None, "mix.f32 goes");
}

/// D10 of M6: an `unstored` while an open of the session holds its guard
/// removes nothing until the open ends.  In `restore_stored`, a decode that
/// ends with `decode-unstored` keeps the mix until the song is shown.
#[wasm_bindgen_test]
async fn an_open_protects_its_session() {
    let (line, file, kept_while_open) = unstored_with_a_kept_mix(false).await;
    assert!(
        kept_while_open,
        "mix.f32 stays while the open holds its guard"
    );
    assert_eq!(
        (line, file),
        (None, None),
        "and goes once the song is shown"
    );

    // An `unstored` of the separator while an open reads the session.
    store::test_wipe().await;
    let frames = 1_500;
    let id = saved(&vec![wav_file("acme.wav", frames, 0)]).await;
    let mix = interleaved(&tone(frames, 0), &tone(frames, 9));
    store::test_write(&store::mix_path(&id), &mix)
        .await
        .expect("planted");
    let mut meta = meta_of(&id).await.expect("meta");
    meta.mix = Some((frames, store_meta::MIX_FORMAT.into()));
    store::test_write(&format!("sessions/{id}/meta"), meta.to_text().as_bytes())
        .await
        .expect("planted");
    let guard = store::OpenGuard::new(&id);
    let protected = store::protected(&id);
    store::forget_mix(&id).await.expect("no error");
    let line_while_open = meta_of(&id).await.and_then(|m| m.mix);
    let file_while_open = store::test_read(&store::mix_path(&id)).await.is_some();
    let waits = store::mix_removal_waits(&id);
    drop(guard);
    store::test_settled(&id).await;
    let line_after = meta_of(&id).await.and_then(|m| m.mix);
    let file_after = store::test_read(&store::mix_path(&id)).await;
    let waits_after = store::mix_removal_waits(&id);
    store::test_wipe().await;
    assert!(protected);
    assert_eq!(
        line_while_open.map(|(f, _)| f),
        Some(frames),
        "the line stays"
    );
    assert!(file_while_open, "mix.f32 stays");
    assert!(waits, "the removal waits");
    assert_eq!((line_after, file_after), (None, None), "then both go");
    assert!(!waits_after);
}

/// D8: a separation that names the session keeps its mix.
#[wasm_bindgen_test]
async fn unstored_keeps_a_mix_that_a_separation_uses() {
    let (line, file, _) = unstored_with_a_kept_mix(true).await;
    assert_eq!(line.map(|(f, _)| f), Some(1_500), "the mix line stays");
    assert_eq!(file.map(|f| f.len()), Some(8 * 1_500), "mix.f32 stays");
}

/// Stem `k`'s samples: a tone already on the 16-bit grid, so a stem WAV
/// holds it exactly.
fn stem_side(frames: usize, k: usize) -> Vec<f32> {
    tone(frames, k)
        .iter()
        .map(|v| (v * 32_768.0).round() / 32_768.0)
        .collect()
}

/// A session with three stems of `frames` frames, saved as WAVs and named
/// in `meta`: its id, the stem names and the sides of each stem.
async fn session_with_stems(frames: usize) -> (String, Vec<String>, Vec<(Vec<f32>, Vec<f32>)>) {
    store::test_wipe().await;
    let (name, file) = wav_file("acme.wav", frames, 0);
    let id = store::save_sources(&[(name, file.into())], 44_100)
        .await
        .expect("a session");
    let names: Vec<String> = ["bass", "drums", "vocals"].map(String::from).into();
    let sides: Vec<(Vec<f32>, Vec<f32>)> = (0..names.len())
        .map(|k| (stem_side(frames, k), stem_side(frames, k + 4)))
        .collect();
    let stems: Vec<Stem> = names
        .iter()
        .zip(&sides)
        .map(|(name, (l, r))| Stem {
            name: name.clone(),
            left: l.clone().into(),
            right: r.clone().into(),
        })
        .collect();
    store::save_stems(&id, 44_100, &stems).await.expect("saved");
    (id, names, sides)
}

/// The overview file bytes of two sides.
fn overview_file_of(left: &[f32], right: &[f32]) -> Vec<u8> {
    let (columns, _) = engine::overview::lane_columns(left, right);
    columns
        .iter()
        .flat_map(|(lo, hi)| [lo.to_le_bytes(), hi.to_le_bytes()])
        .flatten()
        .collect()
}

/// The overviews of `sides` and of their mix at volume 1, as
/// `lane_columns` makes them.
fn want_overviews(sides: &[(Vec<f32>, Vec<f32>)]) -> Vec<Vec<u8>> {
    let ones = vec![1.0; sides.len()];
    let lefts: Vec<&[f32]> = sides.iter().map(|(l, _)| l.as_slice()).collect();
    let rights: Vec<&[f32]> = sides.iter().map(|(_, r)| r.as_slice()).collect();
    let (ml, mr) = (dsp::mix(&lefts, &ones), dsp::mix(&rights, &ones));
    sides
        .iter()
        .map(|(l, r)| overview_file_of(l, r))
        .chain([overview_file_of(&ml, &mr)])
        .collect()
}

/// The bytes of an overview, as its file holds it.
fn bytes_of(overview: &lanes::Overview) -> Vec<u8> {
    overview
        .columns
        .iter()
        .flat_map(|(lo, hi)| [lo.to_le_bytes(), hi.to_le_bytes()])
        .flatten()
        .collect()
}

fn set_bytes(set: &store::StemOverviewSet) -> Vec<Vec<u8>> {
    set.lanes
        .iter()
        .chain([&set.mix])
        .map(|o| bytes_of(o))
        .collect()
}

impl Fake {
    /// The `n`th request posted, an `overviews` request.
    async fn overviews(&self, n: u32) -> stem_protocol::decode::OverviewsRequest {
        for _ in 0..250 {
            if self.posts.length() > n {
                break;
            }
            dt_sleep(20).await;
        }
        assert!(self.posts.length() > n, "no request {n} was posted");
        match stem_protocol::decode::WorkerRequest::from_js(&self.posts.get(n)) {
            Ok(stem_protocol::decode::WorkerRequest::Overviews(req)) => req,
            other => panic!("not an overviews request: {other:?}"),
        }
    }
}

/// Plants the part files of `req`, as the worker writes them.
async fn plant_overview_parts(req: &stem_protocol::decode::OverviewsRequest, files: &[Vec<u8>]) {
    let paths = req
        .lanes
        .iter()
        .map(|l| &l.overview)
        .chain(req.mix.as_ref());
    for (path, bytes) in paths.zip(files) {
        store::test_write(&format!("{}/{path}", req.session), bytes)
            .await
            .expect("planted");
    }
}

/// D7 of the M7 plan: the first call asks the decode worker for the
/// overviews of the stems, moves them into place and writes the
/// `stem-overviews` line (its only write: MEM-5); a second call reads them
/// and posts no request.
#[wasm_bindgen_test]
async fn a_session_with_stems_and_no_overviews_gets_them_once() {
    let frames = 50_000;
    let (id, names, sides) = session_with_stems(frames).await;
    let want = want_overviews(&sides);
    let fake = Fake::new();
    let client = fake.client.clone();
    let first = {
        let id = id.clone();
        background(async move { store::stem_overviews(&id, frames, &client).await })
    };
    let req = fake.overviews(0).await;
    plant_overview_parts(&req, &want).await;
    dt_count_writes();
    fake.reply(DecodeReply::OverviewsMade);
    let first = output(&first).await.expect("the overviews");
    let writes = dt_writes_counted();
    let second = store::stem_overviews(&id, frames, &fake.client)
        .await
        .expect("the overviews");
    let posts = fake.posts.length();
    let meta = meta_of(&id).await.expect("a meta");
    let parts = store::test_read(&format!("sessions/{id}/{}", req.lanes[0].overview)).await;
    store::test_wipe().await;
    assert_eq!(req.lanes.len(), names.len());
    assert_eq!(req.lanes[1].lane.path, "stems/drums.wav");
    assert_eq!(req.lanes[1].overview, "overviews/stems/1.f32.1.decode.part");
    assert_eq!(
        req.mix.as_deref(),
        Some("overviews/stems/mix.f32.1.decode.part")
    );
    assert!(
        set_bytes(&first) == want,
        "the overviews of the stems and their mix"
    );
    assert!(set_bytes(&second) == want);
    assert_eq!(posts, 1, "a second call posts no request");
    assert_eq!(writes, 1, "the page writes only meta");
    assert!(meta.stem_overviews_for(frames), "{meta:?}");
    assert_eq!(parts, None, "the part files moved");
}

/// D7: new stems, from `keep_stored_stems` or `save_stems`, raise the
/// stems generation and remove the `stem-overviews` line.
#[wasm_bindgen_test]
async fn new_stems_remove_the_stem_overviews_line() {
    let frames = 20_000;
    let (id, names, sides) = session_with_stems(frames).await;
    let fake = Fake::new();
    let client = fake.client.clone();
    let made = {
        let id = id.clone();
        background(async move { store::stem_overviews(&id, frames, &client).await })
    };
    let req = fake.overviews(0).await;
    plant_overview_parts(&req, &want_overviews(&sides)).await;
    fake.reply(DecodeReply::OverviewsMade);
    output(&made).await.expect("the overviews");
    let before = meta_of(&id).await.expect("a meta");
    for name in &names {
        let wav = store::test_read(&format!("sessions/{id}/stems/{name}.wav"))
            .await
            .expect("a stem");
        store::test_write(&format!("sessions/{id}/stems/{name}.wav.part"), &wav)
            .await
            .expect("planted");
    }
    store::keep_stored_stems(&id, &names)
        .await
        .kept
        .expect("kept");
    let kept = meta_of(&id).await.expect("a meta");
    let stems: Vec<Stem> = names
        .iter()
        .zip(&sides)
        .map(|(name, (l, r))| Stem {
            name: name.clone(),
            left: l.clone().into(),
            right: r.clone().into(),
        })
        .collect();
    store::save_stems(&id, 44_100, &stems).await.expect("saved");
    let saved = meta_of(&id).await.expect("a meta");
    store::test_wipe().await;
    assert!(before.stem_overviews_for(frames), "{before:?}");
    assert_eq!(
        (kept.stems_generation, kept.stem_overviews),
        (before.stems_generation + 1, None),
        "keep_stored_stems"
    );
    assert_eq!(
        (saved.stems_generation, saved.stem_overviews),
        (kept.stems_generation + 1, None),
        "save_stems"
    );
}

/// D7: an `overviews` request starts, new stems move in, then
/// `overviews-made` arrives.  The page writes no `stem-overviews` line,
/// and removes the part files.
#[wasm_bindgen_test]
async fn overviews_of_older_stems_are_not_kept() {
    let frames = 20_000;
    let (id, names, sides) = session_with_stems(frames).await;
    let fake = Fake::new();
    let client = fake.client.clone();
    let made = {
        let id = id.clone();
        background(async move { store::stem_overviews(&id, frames, &client).await })
    };
    let req = fake.overviews(0).await;
    plant_overview_parts(&req, &want_overviews(&sides)).await;
    for name in &names {
        let wav = store::test_read(&format!("sessions/{id}/stems/{name}.wav"))
            .await
            .expect("a stem");
        store::test_write(&format!("sessions/{id}/stems/{name}.wav.part"), &wav)
            .await
            .expect("planted");
    }
    store::keep_stored_stems(&id, &names)
        .await
        .kept
        .expect("kept");
    fake.reply(DecodeReply::OverviewsMade);
    let got = output(&made).await;
    let meta = meta_of(&id).await.expect("a meta");
    let mut left = Vec::new();
    for lane in req
        .lanes
        .iter()
        .map(|l| &l.overview)
        .chain(req.mix.as_ref())
    {
        left.extend(
            store::test_read(&format!("sessions/{id}/{lane}"))
                .await
                .map(|_| lane.clone()),
        );
    }
    let moved = store::test_read(&format!("sessions/{id}/overviews/stems/0.f32")).await;
    store::test_wipe().await;
    assert!(got.is_none(), "no overviews of older stems");
    assert_eq!(meta.stem_overviews, None, "{meta:?}");
    assert_eq!(left, Vec::<String>::new(), "the part files go");
    assert_eq!(moved, None, "nothing moved into place");
}

/// D7: when the decode worker cannot start, the page makes the overviews
/// through the lane reader, and does not save them.
#[wasm_bindgen_test]
async fn the_page_makes_the_overviews_when_the_worker_cannot() {
    let frames = 50_000;
    let (id, _, sides) = session_with_stems(frames).await;
    let failing = decoder::DecodeClient::new("./no-such-decode-worker.js");
    let got = store::stem_overviews(&id, frames, &failing).await;
    let meta = meta_of(&id).await.expect("a meta");
    store::test_wipe().await;
    let got = got.expect("the page made them");
    assert!(set_bytes(&got) == want_overviews(&sides));
    assert_eq!(meta.stem_overviews, None, "not saved");
}
