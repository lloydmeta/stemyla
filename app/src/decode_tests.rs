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
// Records the largest read of the bytes of any Blob or File, and the
// reads of more than `most` bytes, until stopped.
let dt_sizes = null;
let dt_real_ab = null;
export function dt_start_size_spy(most) {
  dt_real_ab = Blob.prototype.arrayBuffer;
  dt_sizes = { largest: 0, over: 0 };
  Blob.prototype.arrayBuffer = function () {
    dt_sizes.largest = Math.max(dt_sizes.largest, this.size);
    if (this.size > most) dt_sizes.over++;
    return dt_real_ab.call(this);
  };
}
export function dt_stop_size_spy() {
  Blob.prototype.arrayBuffer = dt_real_ab;
  return [dt_sizes.largest, dt_sizes.over];
}
// A file named `name` of `size` bytes: `head`, then zeros.
export function dt_big_file(head, size, name) {
  return new File([head, new Uint8Array(size - head.length)], name);
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
    fn dt_start_size_spy(most: u32);
    fn dt_stop_size_spy() -> Vec<f64>;
    fn dt_big_file(head: &[u8], size: f64, name: &str) -> web_sys::File;
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

/// The samples of lane `k` of `song`, read through the lane reader, as the
/// page reads a stored lane (D3 of the M7 plan).
async fn samples_of(song: &lanes::Shown, k: usize) -> (Vec<f32>, Vec<f32>) {
    let pcm = lanes::LanePcm::new(song.lanes[k].audio.clone(), song.frames, song.sample_rate);
    let mut job = export::ExportJob::new(Box::new(pcm), export::VecSink::new(song.frames));
    while !job.run_while(|| true) {
        dt_sleep(1).await;
    }
    let sink = job.into_sink();
    (sink.left, sink.right)
}

/// Shows `opened` on a page as its newest open, as `apply_inbox` does, and
/// waits until the page has made its session current (D12 of the M7
/// plan).
async fn show_as_newest(opened: Opened) {
    let mut app = StemApp {
        want_device: false,
        ..StemApp::default()
    };
    let token = app.next_open();
    app.pending.borrow_mut().put_song(token, Ok(opened));
    app.apply_inbox(&egui::Context::default());
    dt_sleep(200).await;
    app.load_demo();
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
    let opened =
        background(async move { open_chosen_now(&chosen, &client, MEMORY_PATH_MAX_FRAMES).await });
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
    let opened =
        background(async move { open_chosen_now(&chosen, &client, MEMORY_PATH_MAX_FRAMES).await });
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
    let opened = open_chosen_now(&chosen, &fake.client, MEMORY_PATH_MAX_FRAMES).await;
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
        let song = &opened.song;
        let mut read = Vec::new();
        for k in 0..song.lanes.len() {
            read.push(samples_of(song, k).await);
        }
        store::test_wipe().await;
        assert_eq!(song.names(), names);
        assert_eq!(
            (song.frames, opened.session.as_deref()),
            (frames, Some(id.as_str()))
        );
        // D11 of the M7 plan: each lane is its session file, with no
        // samples in memory, and reads the decoded samples.
        assert!(
            song.lanes.iter().all(|l| l.audio.samples().is_none()),
            "no samples"
        );
        assert!(
            song.lanes.iter().all(|l| l.audio.file().is_some()),
            "each a file"
        );
        assert!(song.mix_overview.is_some(), "the mix overview");
        for ((name, (left, right)), got) in names.iter().zip(&samples).zip(&read) {
            assert!(got.0 == *left, "{name}: left");
            assert!(got.1 == *right, "{name}: right");
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
    let song = opened.expect("opened").song;
    assert_eq!(song.names(), ["drums", "classic drums"]);
    assert_eq!(
        mix,
        Some(interleaved(&samples[0].0, &samples[0].1)),
        "the mix is drums"
    );
}

/// A session of the song on screen, made current, as a reload restores it.
async fn a_current_session() -> String {
    let before = saved(&vec![wav_file("acme.wav", 100, 3)]).await;
    assert!(store::make_current(&before).await, "made current");
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
    let opened =
        background(async move { open_chosen_now(&chosen, &client, MEMORY_PATH_MAX_FRAMES).await });
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
    let opened =
        background(async move { open_chosen_now(&chosen, &client, MEMORY_PATH_MAX_FRAMES).await });
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
    let opened =
        background(async move { open_chosen_now(&chosen, &client, MEMORY_PATH_MAX_FRAMES).await });
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
    let opened = output(&opened).await.expect("opened");
    let opened_not_shown = store::current().await;
    show_as_newest(opened).await;
    let after = store::current().await;
    store::test_wipe().await;
    assert_eq!(
        during,
        Some(before.clone()),
        "not current during the decode"
    );
    assert_eq!(opened_not_shown, Some(before), "not current until it shows");
    assert_eq!(after, Some(id_of(&req.session)), "current once it shows");
}

#[wasm_bindgen_test]
async fn unstored_decodes_on_the_page_and_keeps_the_session() {
    store::test_wipe().await;
    let fake = Fake::new();
    let chosen = vec![wav_file("lloyd.wav", 1_234, 0)];
    let client = fake.client.clone();
    let opened =
        background(async move { open_chosen_now(&chosen, &client, MEMORY_PATH_MAX_FRAMES).await });
    let req = fake.request(0).await;
    fake.reply(DecodeReply::Unstored {
        text: "QuotaExceededError: the disk is full".into(),
    });
    let opened = output(&opened).await.expect("opened on the page");
    let kept = meta_of(&id_of(&req.session)).await;
    let (frames, session, note) = (
        opened.song.frames,
        opened.session.clone(),
        opened.note.clone(),
    );
    show_as_newest(opened).await;
    let current = store::current().await;
    store::test_wipe().await;
    let opened = (session, note);
    assert_eq!(frames, 1_234);
    assert_eq!(current, Some(id_of(&req.session)), "made current");
    assert_eq!(opened.0, Some(id_of(&req.session)), "the session stays");
    assert!(kept.is_some(), "its files stay");
    let note = opened.1.unwrap_or_default();
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
    let opened = open_chosen_now(&chosen, &fake.client, MEMORY_PATH_MAX_FRAMES).await;
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
    let restored =
        background(
            async move { restore_stored(&restore_id, &client, MEMORY_PATH_MAX_FRAMES).await },
        );
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
    let restored = restore_stored(&id, &fake.client, MEMORY_PATH_MAX_FRAMES).await;
    let (opened, meta, note) = restored.expect("a session").expect("restored");
    let read = samples_of(&opened.song, 0).await;
    let stored_lane = opened.song.lanes[0].audio.samples().is_none();
    show_as_newest(opened).await;
    let current = store::current().await;
    store::test_wipe().await;
    assert_eq!(fake.posts.length(), posts, "no decode is posted");
    assert_eq!((note, meta.name.as_str()), (None, "acme"));
    assert!(stored_lane, "a stored lane");
    assert!(read.0 == samples[0].0 && read.1 == samples[0].1);
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
    let restored =
        background(
            async move { restore_stored(&restore_id, &client, MEMORY_PATH_MAX_FRAMES).await },
        );
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
    let read = samples_of(&restored.0.song, 0).await;
    store::test_wipe().await;
    assert_eq!(
        req.mix_frames,
        Some(frames as u64),
        "the usable mix is named"
    );
    assert!(read.0 == left && read.1 == right, "read from the mix");
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
        let opened =
            background(
                async move { open_chosen_now(&chosen, &client, MEMORY_PATH_MAX_FRAMES).await },
            );
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
        let opened = output(&opened).await.expect("opened");
        let before_shown = logged("song kept in browser storage");
        show_as_newest(opened).await;
        let lines = logged("song kept in browser storage");
        store::test_wipe().await;
        let id = id_of(&req.session);
        assert!(
            before_shown.is_empty(),
            "unstored {unstored}: not before it shows"
        );
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

/// Finding 4-2 of review round 4 of M5: when `sessions/current` cannot be
/// written, the page does not log that the song is kept (D12).
#[wasm_bindgen_test]
async fn no_kept_line_when_the_session_cannot_be_made_current() {
    store::test_wipe().await;
    let fake = Fake::new();
    start_log();
    let client = fake.client.clone();
    let chosen = vec![wav_file("lloyd.wav", 1_000, 0)];
    let opened =
        background(async move { open_chosen_now(&chosen, &client, MEMORY_PATH_MAX_FRAMES).await });
    let req = fake.request(0).await;
    let samples = planted(&req, 1_000, &[false]);
    plant_parts(&req, 1_000, &samples, true).await;
    fake.reply(DecodeReply::Decoded {
        frames: 1_000,
        new_mix: true,
        channels: vec![2],
    });
    let opened = output(&opened).await.expect("opened");
    dt_refuse_writes();
    show_as_newest(opened).await;
    dt_allow_writes();
    let lines = logged("song kept in browser storage");
    let warnings = logged("could not mark session");
    store::test_wipe().await;
    assert_eq!(warnings.len(), 1, "the write failed");
    assert_eq!(lines, [], "no kept line");
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
    let opened =
        background(async move { open_chosen_now(&chosen, &client, MEMORY_PATH_MAX_FRAMES).await });
    fake.request(0).await;
    dt_start_read_spy();
    app.prepare_reset();
    let opened = output(&opened).await;
    let open_reads = dt_stop_read_spy();

    let id = saved(&vec![wav_file("acme.wav", 100, 0)]).await;
    let fake = Fake::new();
    app.decoder = fake.client.clone();
    let client = fake.client.clone();
    let restored =
        background(async move { restore_stored(&id, &client, MEMORY_PATH_MAX_FRAMES).await });
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

/// The error of `opened`, the files read and the "decoding on the page"
/// lines since `start`, then storage opens again for the other tests.
async fn reset_outcome<T>(opened: anyhow::Result<T>) -> (String, u32, usize) {
    let reads = dt_stop_read_spy();
    store::test_reopen();
    let page_lines = logged("decoder: decoding on the page").len();
    store::test_wipe().await;
    let error = opened.err().map(|e| format!("{e:#}")).unwrap_or_default();
    (error, reads, page_lines)
}

/// Finding 4-1 of review round 4 of M5: a reset that closes storage
/// before the save ends the open with an error, with no page decode.
#[wasm_bindgen_test]
async fn a_reset_during_the_save_ends_the_open_with_no_page_decode() {
    store::test_wipe().await;
    start_log();
    let fake = Fake::new();
    let chosen = vec![wav_file("lloyd.wav", 100, 0)];
    store::close().await;
    dt_start_read_spy();
    let opened = open_chosen_now(&chosen, &fake.client, MEMORY_PATH_MAX_FRAMES).await;
    let (error, reads, page_lines) = reset_outcome(opened).await;
    assert!(error.starts_with("a reset stopped the open"), "{error}");
    assert_eq!((reads, page_lines), (0, 0), "no file read, no page decode");
}

/// Finding 4-1 of review round 4 of M5: during a reset, a session that the
/// open cannot read ends the open with an error, with no page decode.
#[wasm_bindgen_test]
async fn a_session_not_read_during_a_reset_ends_the_open_with_no_page_decode() {
    store::test_wipe().await;
    start_log();
    let fake = Fake::new();
    fake.client.stop();
    let chosen = vec![wav_file("lloyd.wav", 100, 0)];
    dt_start_read_spy();
    let opened = open_saved(
        &chosen,
        "1792640000001",
        None,
        0.0,
        &fake.client,
        MEMORY_PATH_MAX_FRAMES,
    )
    .await;
    let (error, reads, page_lines) = reset_outcome(opened).await;
    assert!(error.starts_with("a reset stopped the open"), "{error}");
    assert_eq!((reads, page_lines), (0, 0), "no file read, no page decode");
}

/// Finding 4-1 of review round 4 of M5: a decode that a reset stops is a
/// reset, not a cause to decode on the page.
#[wasm_bindgen_test]
async fn a_decode_that_a_reset_stops_is_a_reset() {
    store::test_wipe().await;
    let id = saved(&vec![wav_file("acme.wav", 100, 0)]).await;
    let sources = match store::open_stored(&id).await {
        Some(Ok(store::StoredSong::NeedsDecode { sources, .. })) => sources,
        _ => panic!("a session that needs a decode"),
    };
    let fake = Fake::new();
    let client = fake.client.clone();
    let ended = background(async move { decode_session(&id, &sources, None, &client).await });
    fake.request(0).await;
    fake.client.stop();
    let ended = output(&ended).await;
    store::test_wipe().await;
    match ended {
        Decode::Reset(text) => assert!(text.starts_with("a reset stopped the decode"), "{text}"),
        Decode::OnThePage(failure) => panic!("on the page: {}", failure.why()),
        _ => panic!("not a reset"),
    }
}

/// Session `id`, saved from "acme.wav", with a `decoded` line whose files
/// are not there: its decode must first remove that line (D2 of M5).
async fn session_with_a_stale_decoded_line() -> (String, Chosen) {
    let chosen = vec![wav_file("acme.wav", 100, 0)];
    let id = saved(&chosen).await;
    let mut meta = meta_of(&id).await.expect("a meta");
    meta.decoded = Some(store_meta::Decoded {
        frames: 100,
        format: store_meta::MIX_FORMAT.into(),
        channels: vec![(0, 2)],
    });
    store::test_write(&format!("sessions/{id}/meta"), meta.to_text().as_bytes())
        .await
        .expect("planted");
    (id, chosen)
}

/// Finding 4-1 of review round 4 of M5: during a reset, a cause other
/// than a stopped decode (here `clear_decoded` fails, as storage is
/// closed while the decode client is open) ends an open and a restore
/// with an error, with no page decode.
#[wasm_bindgen_test]
async fn another_page_decode_cause_during_a_reset_ends_with_an_error() {
    store::test_wipe().await;
    start_log();
    let fake = Fake::new();
    let (id, chosen) = session_with_a_stale_decoded_line().await;
    store::close().await;
    dt_start_read_spy();
    let opened = open_saved(
        &chosen,
        &id,
        None,
        0.0,
        &fake.client,
        MEMORY_PATH_MAX_FRAMES,
    )
    .await;
    let (open_error, open_reads, open_lines) = reset_outcome(opened).await;

    start_log();
    let (id, _) = session_with_a_stale_decoded_line().await;
    store::close().await;
    dt_start_read_spy();
    let restored = restore_stored(&id, &fake.client, MEMORY_PATH_MAX_FRAMES)
        .await
        .expect("a session");
    let (restore_error, restore_reads, restore_lines) = reset_outcome(restored).await;
    let posted = fake.posts.length();
    assert!(
        open_error.starts_with("a reset stopped the decode"),
        "{open_error}"
    );
    assert!(
        restore_error.starts_with("a reset stopped the decode"),
        "{restore_error}"
    );
    assert_eq!(
        (open_reads, open_lines, restore_reads, restore_lines),
        (0, 0, 0, 0),
        "no file read, no page decode"
    );
    assert_eq!(posted, 0, "no decode is posted");
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
/// a mix removal, and one on the memory path does not.
#[wasm_bindgen_test]
fn the_page_names_the_session_of_its_separation_for_d7() {
    let mut app = StemApp::default();
    let job = |stage| SeparationJob {
        serial: 0,
        session: Some("1792640000060".into()),
        name: None,
        stage,
        keeps_stems: true,
        lanes: LANE_BUDGET,
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
    let restored =
        background(
            async move { restore_stored(&restore_id, &client, MEMORY_PATH_MAX_FRAMES).await },
        );
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

/// Finding 6-1 of review round 6 of M7: while `save_stems` writes new
/// stems, `meta` names no stems.  So an `overviews` request in that window
/// gives none, and cannot keep overviews of old and new stem WAVs as one
/// set.  The stems are long, so that the window lasts.
#[wasm_bindgen_test]
async fn meta_names_no_stems_while_new_stems_are_written() {
    let frames = 2_000;
    let (id, names, _) = session_with_stems(frames).await;
    let before = meta_of(&id).await.expect("a meta").stems_generation;
    // One shared buffer for each channel of the six stems (MEM-1).
    let channel: engine::Samples = vec![0.1; 60 * 44_100].into();
    let stems: Vec<Stem> = names
        .iter()
        .map(|name| Stem {
            name: name.clone(),
            left: channel.clone(),
            right: channel.clone(),
        })
        .collect();
    let saving = {
        let id = id.clone();
        background(async move { store::save_stems(&id, 44_100, &stems).await })
    };
    let mut window = None;
    for _ in 0..5_000 {
        if saving.borrow().is_some() {
            break;
        }
        let text = store::test_read(&format!("sessions/{id}/meta"))
            .await
            .unwrap_or_default();
        let meta = store_meta::Meta::parse(&String::from_utf8_lossy(&text));
        if meta.stems_generation > before {
            window = Some((meta, saving.borrow().is_none()));
            break;
        }
        dt_sleep(1).await;
    }
    output(&saving).await.expect("saved");
    store::test_wipe().await;
    let (meta, in_the_window) = window.expect("the first meta write was seen");
    assert!(in_the_window, "seen before the stems were all written");
    assert_eq!(meta.stems, Vec::<String>::new(), "no stems named");
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

/// D11 of the M7 plan: when the decode worker fails over to the page,
/// `rebuild` decodes only the sources on the page; complete stem WAVs show
/// as stored lanes with no samples.
#[wasm_bindgen_test]
async fn rebuild_shows_complete_stems_as_stored_lanes() {
    store::test_wipe().await;
    let frames = 3_000;
    let id = saved(&vec![wav_file("acme.wav", frames, 0)]).await;
    let stems: Vec<Stem> = ["bass", "drums"]
        .iter()
        .enumerate()
        .map(|(k, name)| Stem {
            name: name.to_string(),
            left: stem_side(frames, k).into(),
            right: stem_side(frames, k + 4).into(),
        })
        .collect();
    store::save_stems(&id, 44_100, &stems).await.expect("saved");
    let failing = decoder::DecodeClient::new("./no-such-decode-worker.js");
    let (opened, _, note) = restore_stored(&id, &failing, MEMORY_PATH_MAX_FRAMES)
        .await
        .expect("a session")
        .expect("opened");
    store::test_wipe().await;
    let song = &opened.song;
    assert!(
        note.as_deref()
            .is_some_and(|n| n.contains("decoded on this page")),
        "{note:?}"
    );
    assert_eq!(song.names(), ["bass", "drums", "original"]);
    for lane in &song.lanes[..2] {
        assert!(lane.audio.samples().is_none(), "{}: no samples", lane.name);
        let file = lane.audio.file().expect("a stored stem");
        assert!(
            file.path.ends_with(&format!("stems/{}.wav", lane.name)),
            "{}",
            file.path
        );
    }
    assert!(
        song.lanes[2].audio.samples().is_some(),
        "the source, decoded on the page"
    );
}

/// D2 of the M7 plan: a song whose lanes do not fit the read-ahead buffer
/// at its minimum slots (21 sources, no stems) opens on the memory path,
/// decoded on the page, and the note says why.
#[wasm_bindgen_test]
async fn a_song_over_the_read_ahead_cap_opens_in_memory() {
    store::test_wipe().await;
    let chosen: Chosen = (0..21)
        .map(|k| wav_file(&format!("lloyd {k}.wav"), 100, k))
        .collect();
    let id = saved(&chosen).await;
    let fake = Fake::new();
    let (opened, _, note) = restore_stored(&id, &fake.client, MEMORY_PATH_MAX_FRAMES)
        .await
        .expect("a session")
        .expect("opened");
    let posts = fake.posts.length();
    store::test_wipe().await;
    let song = &opened.song;
    assert_eq!(posts, 0, "no decode is posted");
    assert_eq!(song.lanes.len(), 21);
    assert!(
        song.lanes.iter().all(|l| l.audio.samples().is_some()),
        "in memory"
    );
    assert!(
        note.as_deref()
            .is_some_and(|n| n.contains("do not fit the read-ahead buffer")),
        "{note:?}"
    );
}

/// D2 of the M7 plan on the drop and pick path (finding 2-6 of review
/// round 2): 21 sources do not fit the read-ahead buffer, so the open
/// decodes them on the page.  No decode is posted, every lane has samples,
/// and the note says why.
#[wasm_bindgen_test]
async fn a_drop_over_the_read_ahead_cap_opens_in_memory() {
    store::test_wipe().await;
    let chosen: Chosen = (0..21)
        .map(|k| wav_file(&format!("lloyd {k}.wav"), 100, k))
        .collect();
    let fake = Fake::new();
    let opened = {
        let client = fake.client.clone();
        background(async move { open_chosen_now(&chosen, &client, MEMORY_PATH_MAX_FRAMES).await })
    };
    // A decode that is posted waits for a reply: stop at the first post.
    for _ in 0..250 {
        if opened.borrow().is_some() || fake.posts.length() > 0 {
            break;
        }
        dt_sleep(20).await;
    }
    let posts = fake.posts.length();
    assert_eq!(posts, 0, "no decode is posted");
    let opened = output(&opened).await;
    store::test_wipe().await;
    let opened = opened.expect("opened");
    assert_eq!(opened.song.lanes.len(), 21);
    assert!(
        opened
            .song
            .lanes
            .iter()
            .all(|l| l.audio.samples().is_some()),
        "in memory"
    );
    assert!(
        opened
            .note
            .as_deref()
            .is_some_and(|n| n.contains("do not fit the read-ahead buffer")
                && n.starts_with("Decoded on this page: ")
                && !n.contains("browser storage failed")),
        "{:?}",
        opened.note
    );
}

/// D11 of the M7 plan and MEM-1's exception: a stem whose file is not
/// complete (a `.part`) is read whole into memory only for a song of at
/// most the memory path's frames; for a longer song, the open fails.
#[wasm_bindgen_test]
async fn an_incomplete_stem_is_read_whole_only_on_the_memory_path() {
    store::test_wipe().await;
    let frames = 2_000;
    let id = saved(&vec![wav_file("acme.wav", frames, 0)]).await;
    let (left, right) = (stem_side(frames, 0), stem_side(frames, 4));
    let path = format!("sessions/{id}/stems/bass.wav.part");
    store::test_write(&path, &engine::wav::encode_pcm16(&left, &right, 44_100))
        .await
        .expect("planted");
    let failing = decoder::DecodeClient::new("./no-such-decode-worker.js");
    let (names, paths) = (vec!["bass".to_string()], vec![path]);
    let fits = store::stem_lanes(&id, &names, &paths, frames, &failing, frames).await;
    let long = store::stem_lanes(&id, &names, &paths, frames, &failing, frames - 1).await;
    store::test_wipe().await;
    let (lanes, mix) = fits.expect("read into memory");
    let (got_left, got_right) = lanes[0].audio.samples().expect("in memory");
    assert!(
        **got_left == left && **got_right == right,
        "the stem's samples"
    );
    assert!(
        mix.is_none(),
        "no overview of a mix with a stem not complete"
    );
    let error = long.err().map(|e| format!("{e:#}"));
    assert!(
        error.as_deref().is_some_and(|e| e.contains("not complete")),
        "{error:?}"
    );
}

/// Review round 1 of M7, finding 1-2: when the overviews of complete stem
/// WAVs cannot be made, the stems stay stored lanes, with empty overviews.
/// They are not read whole.  Here the decode worker replies `unstored`, and
/// the page's own read fails, because the first stem's file goes.
#[wasm_bindgen_test]
async fn a_stem_with_no_overview_stays_a_stored_lane() {
    let frames = 2_000;
    let (id, names, _) = session_with_stems(frames).await;
    let fake = Fake::new();
    let paths = store::stem_paths(&id, &names);
    let opened = {
        let (id, names, paths) = (id.clone(), names.clone(), paths.clone());
        let client = fake.client.clone();
        background(
            async move { store::stem_lanes(&id, &names, &paths, frames, &client, frames).await },
        )
    };
    fake.overviews(0).await;
    let removed = store::test_remove(&paths[0]).await;
    fake.reply(DecodeReply::Unstored {
        text: "no overviews".into(),
    });
    let got = output(&opened).await;
    store::test_wipe().await;
    assert!(removed, "the first stem's file goes");
    let (lanes, mix) = got.expect("the stems open");
    assert_eq!(lanes.len(), 3);
    for lane in &lanes {
        assert!(
            matches!(
                &lane.audio,
                crate::lanes::LaneAudio::Stored { overview, .. } if overview.columns.is_empty()
            ),
            "{} is a stored lane with an empty overview",
            lane.name
        );
    }
    assert!(mix.is_none(), "no overview of the mix");
}

/// Finding 7-1 of review round 7 of M7: the page's overview fallback stops
/// at the first failed read.  The song is 20 blocks long.  The first
/// stem's file goes after its header is read, so each of its reads fails.
#[wasm_bindgen_test]
async fn the_page_overviews_stop_at_the_first_failed_read() {
    let frames = 20 * crate::export::BLOCK;
    let (id, names, _) = session_with_stems(frames).await;
    let fake = Fake::new();
    let paths = store::stem_paths(&id, &names);
    let opened = {
        let (id, names, paths) = (id.clone(), names.clone(), paths.clone());
        let client = fake.client.clone();
        background(
            async move { store::stem_lanes(&id, &names, &paths, frames, &client, frames).await },
        )
    };
    fake.overviews(0).await;
    let removed = store::test_remove(&paths[0]).await;
    let before = crate::lane_store::reads_for_test();
    fake.reply(DecodeReply::Unstored {
        text: "no overviews".into(),
    });
    let got = output(&opened).await;
    let reads = crate::lane_store::reads_for_test() - before;
    store::test_wipe().await;
    assert!(removed, "the first stem's file goes");
    assert!(got.is_ok(), "the stems open");
    // One read: the page tries once, and stops at the failure (findings
    // 7-1, 8-1 and 8-2 of review rounds 7 and 8 of M7).
    assert_eq!(reads, 1, "one attempt, stopped at the failure");
}

/// Waits for an open's result to reach the inbox, applies it, and waits
/// for the page to make its session current if it shows.
async fn apply_the_open(app: &mut StemApp) {
    for _ in 0..250 {
        if app.pending.borrow().song.is_some() {
            break;
        }
        dt_sleep(20).await;
    }
    assert!(app.pending.borrow().song.is_some(), "an open ended");
    app.apply_inbox(&egui::Context::default());
    dt_sleep(200).await;
}

/// The decode of a late open ends: the worker's files, then `decoded`.
async fn decode_ends(fake: &Fake, n: u32) {
    let req = fake.request(n).await;
    let frames = 1_000;
    plant_parts(&req, frames, &planted(&req, frames, &[false]), true).await;
    fake.reply(DecodeReply::Decoded {
        frames: frames as u64,
        new_mix: true,
        channels: vec![2],
    });
}

/// D12 of the M7 plan: a drop's or a pick's decode is pending, a
/// Storage open of a complete session shows its song, then the decode
/// ends.  The shown song and `sessions/current` stay the Storage song.
#[wasm_bindgen_test]
async fn a_late_drop_does_not_show_over_a_newer_song() {
    for pick in [false, true] {
        store::test_wipe().await;
        let fake = Fake::new();
        let (stored, req, _) =
            open_decoded(&fake, vec![wav_file("acme.wav", 100, 0)], 2_000, &[false]).await;
        drop(stored.expect("opened"));
        let stored_id = id_of(&req.session);
        let mut app = StemApp {
            want_device: false,
            ..StemApp::default()
        };
        app.decoder = fake.client.clone();
        let ctx = egui::Context::default();
        let chosen = vec![wav_file("lloyd.wav", 100, 1)];
        if pick {
            let opens = app.opens.clone();
            open_picked(
                chosen,
                &app.decoder,
                &app.pending,
                &ctx,
                &opens,
                MEMORY_PATH_MAX_FRAMES,
            );
        } else {
            app.open_files(chosen, &ctx);
        }
        let late = fake.request(1).await;
        app.open_from_storage(stored_id.clone());
        apply_the_open(&mut app).await;
        let shown_first = app.session.clone();
        decode_ends(&fake, 1).await;
        apply_the_open(&mut app).await;
        let (shown, current) = (app.session.clone(), store::current().await);
        let late_kept = meta_of(&id_of(&late.session)).await.is_some();
        app.load_demo();
        store::test_wipe().await;
        assert_eq!(
            shown_first,
            Some(stored_id.clone()),
            "pick {pick}: the Storage song shows"
        );
        assert_eq!(
            shown,
            Some(stored_id.clone()),
            "pick {pick}: it stays shown"
        );
        assert_eq!(current, Some(stored_id), "pick {pick}: it stays current");
        assert!(late_kept, "pick {pick}: the late open's session is kept");
    }
}

/// Finding 2-17 of review round 2 of M7: an older pick that ends does not
/// clear the "Opening…" text that a newer pick queued.
#[wasm_bindgen_test]
async fn an_older_pick_does_not_clear_the_text_of_a_newer_one() {
    store::test_wipe().await;
    let fake = Fake::new();
    let app = StemApp {
        want_device: false,
        ..StemApp::default()
    };
    let (ctx, opens) = (egui::Context::default(), app.opens.clone());
    let client = fake.client.clone();
    open_picked(
        vec![wav_file("lloyd.wav", 100, 1)],
        &client,
        &app.pending,
        &ctx,
        &opens,
        MEMORY_PATH_MAX_FRAMES,
    );
    fake.request(0).await;
    let newer = vec![wav_file("acme.wav", 100, 2)];
    let opening = opening_text(&newer);
    open_picked(
        newer,
        &client,
        &app.pending,
        &ctx,
        &opens,
        MEMORY_PATH_MAX_FRAMES,
    );
    decode_ends(&fake, 0).await;
    for _ in 0..250 {
        if app.pending.borrow().song.is_some() {
            break;
        }
        dt_sleep(20).await;
    }
    let status = app.pending.borrow().status.clone();
    // The newer pick's decode ends too, so that no write stays held for
    // the next tests.
    fake.request(1).await;
    decode_ends(&fake, 1).await;
    for _ in 0..250 {
        if store::writes_in_progress() == 0 {
            break;
        }
        dt_sleep(20).await;
    }
    let writes = store::writes_in_progress();
    drop(app);
    store::test_wipe().await;
    assert_eq!(status, Some(opening), "the newer pick's text stays");
    assert_eq!(writes, 0, "no write stays held");
}

/// D12: Demo takes a token, so a drop that ends after it does not
/// show over it, nor become current.
#[wasm_bindgen_test]
async fn a_late_drop_does_not_show_over_demo() {
    store::test_wipe().await;
    let fake = Fake::new();
    let before = a_current_session().await;
    let mut app = StemApp {
        want_device: false,
        ..StemApp::default()
    };
    app.decoder = fake.client.clone();
    app.open_files(
        vec![wav_file("lloyd.wav", 100, 1)],
        &egui::Context::default(),
    );
    fake.request(0).await;
    app.load_demo();
    decode_ends(&fake, 0).await;
    apply_the_open(&mut app).await;
    let (shown, names) = (app.session.clone(), app.lanes.len());
    let current = store::current().await;
    store::test_wipe().await;
    assert_eq!(shown, None, "the demo has no session");
    assert_eq!(names, 6, "the demo's lanes");
    assert_eq!(current, Some(before), "current does not change");
}

/// Findings 2-17 and 3-8 of review rounds 2 and 3 of M7: only the newest
/// open sets or clears the inbox status.
#[wasm_bindgen_test]
fn only_the_newest_open_sets_the_status() {
    let mut inbox = Inbox {
        status: Some("Opening acme…".into()),
        ..Inbox::default()
    };
    inbox.status_of_open(1, 2, None);
    assert_eq!(
        inbox.status.as_deref(),
        Some("Opening acme…"),
        "an older clear"
    );
    inbox.status_of_open(2, 2, Some("Restored \"lloyd\" from this browser.".into()));
    assert_eq!(
        inbox.status.as_deref(),
        Some("Restored \"lloyd\" from this browser."),
        "the newest sets"
    );
    inbox.status_of_open(1, 2, Some("Could not restore the stored song".into()));
    assert_eq!(
        inbox.status.as_deref(),
        Some("Restored \"lloyd\" from this browser."),
        "an older error"
    );
    inbox.status_of_open(2, 2, None);
    assert_eq!(inbox.status, None, "the newest clears");
}

/// D12: two results that reach the inbox before one frame: the newer
/// shows, in either order.
#[wasm_bindgen_test]
async fn two_opens_that_end_in_one_frame_show_the_newer() {
    let song = |name: &str| {
        Opened::in_memory(
            Song {
                sample_rate: 44_100,
                frames: 4,
                stems: vec![Stem {
                    name: name.into(),
                    left: vec![0.1; 4].into(),
                    right: vec![0.1; 4].into(),
                }],
            },
            None,
            None,
        )
    };
    for newer_first in [true, false] {
        let mut app = StemApp {
            want_device: false,
            ..StemApp::default()
        };
        let older = app.next_open();
        let newer = app.next_open();
        {
            let mut inbox = app.pending.borrow_mut();
            if newer_first {
                inbox.put_song(newer, Ok(song("lloyd")));
                inbox.put_song(older, Ok(song("acme")));
            } else {
                inbox.put_song(older, Ok(song("acme")));
                inbox.put_song(newer, Ok(song("lloyd")));
            }
        }
        app.apply_inbox(&egui::Context::default());
        let names = app.song.as_ref().map(|s| s.names().join(","));
        assert_eq!(
            names.as_deref(),
            Some("lloyd"),
            "newer first: {newer_first}"
        );
    }
}

/// D12: a Storage open that the decode worker fails over to the page
/// (`store::restore_id`) ends after a newer open (Demo): it does not make
/// its session current.
#[wasm_bindgen_test]
async fn a_stale_restore_on_the_page_does_not_make_its_session_current() {
    store::test_wipe().await;
    let before = a_current_session().await;
    let incomplete = saved(&vec![wav_file("lloyd.wav", 100, 2)]).await;
    let mut app = StemApp {
        want_device: false,
        ..StemApp::default()
    };
    app.decoder = decoder::DecodeClient::new("./no-such-decode-worker.js");
    app.open_from_storage(incomplete.clone());
    app.load_demo();
    apply_the_open(&mut app).await;
    let shown = app.session.clone();
    let current = store::current().await;
    store::test_wipe().await;
    assert_eq!(shown, None, "the demo stays");
    assert_eq!(current, Some(before), "the stale restore is not current");
}

/// The largest read of a file's bytes during `work`, and the reads of more
/// than a file's head (`engine::decode::HEAD_BYTES`).
async fn reads_of<T>(work: impl Future<Output = T>) -> (T, u64, u32) {
    dt_start_size_spy(engine::decode::HEAD_BYTES as u32);
    let out = work.await;
    let sizes = dt_stop_size_spy();
    (out, sizes[0] as u64, sizes[1] as u32)
}

/// The status line of a failed open, as the page shows it.
fn shown_error(opened: &anyhow::Result<Opened>) -> String {
    match opened {
        Ok(_) => "opened".into(),
        Err(e) => open_error_text("Could not load", e),
    }
}

/// One second at 44.1 kHz: the memory path's limit in these tests.
const SECOND: usize = 44_100;

/// D3 of the catch-up plan: with no browser storage, a drop of a WAV whose
/// header states 2 s is refused, with a limit of 1 s, from its head alone.
/// The status line says why and what to do, and the page shows it as it is.
#[wasm_bindgen_test]
async fn a_drop_with_no_storage_refuses_a_long_wav_from_its_head() {
    store::test_wipe().await;
    let fake = Fake::new();
    let chosen = vec![wav_file("lloyd.wav", 2 * SECOND, 0)];
    let size = chosen[0].1.size() as u64;
    dt_refuse_writes();
    let (opened, largest, over) = reads_of(open_chosen_now(&chosen, &fake.client, SECOND)).await;
    dt_allow_writes();
    store::test_wipe().await;
    let text = shown_error(&opened);
    assert!(size > engine::decode::HEAD_BYTES as u64, "{size} bytes");
    assert_eq!(
        over, 0,
        "no read past the head: the largest was {largest} bytes"
    );
    assert!(
        text.starts_with("Not opened: this song is 0:02 long, too long to open in this browser now.  On this page, a song of one file opens only up to 0:01, and the page decodes it here because browser storage could not keep it ("),
        "{text}"
    );
    assert!(text.contains("QuotaExceededError"), "{text}");
    assert!(
        text.ends_with("Free browser storage in the Storage menu, or open a shorter song."),
        "{text}"
    );
    // The page shows the refusal as its status line.
    let mut app = StemApp {
        want_device: false,
        ..StemApp::default()
    };
    let token = app.next_open();
    app.pending.borrow_mut().put_song(token, opened);
    app.apply_inbox(&egui::Context::default());
    assert_eq!(app.status, text);
}

/// D3 of the catch-up plan: a streaming WAV states no length.  One larger
/// than the largest file of a song of 1 s is refused from its head; a
/// smaller one is read, and its decode stops at the limit.
#[wasm_bindgen_test]
async fn a_wav_of_no_stated_length_is_capped_then_stopped_at_the_limit() {
    store::test_wipe().await;
    let fake = Fake::new();
    let mut header = Vec::from(engine::wav::header(0, 44_100));
    header[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    header[40..44].copy_from_slice(&u32::MAX.to_le_bytes());
    let cap = engine::decode::largest_file(engine::decode::HeadLength::UnstatedPcm, SECOND);
    let large = vec![(
        "lloyd.wav".to_string(),
        dt_big_file(&header, (cap + 1) as f64, "lloyd.wav"),
    )];
    dt_refuse_writes();
    let (refused, _, over) = reads_of(open_chosen_now(&large, &fake.client, SECOND)).await;
    let mut streaming = vec![wav_file("acme.wav", 2 * SECOND, 1)];
    let bytes = {
        let wav = engine::wav::encode_pcm16(&tone(2 * SECOND, 1), &tone(2 * SECOND, 8), 44_100);
        let mut wav = wav;
        wav[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        wav[40..44].copy_from_slice(&u32::MAX.to_le_bytes());
        wav
    };
    streaming[0].1 = dt_file(&bytes, "acme.wav");
    let stopped = open_chosen_now(&streaming, &fake.client, SECOND).await;
    let fits = open_chosen_now(&streaming, &fake.client, 2 * SECOND).await;
    dt_allow_writes();
    store::test_wipe().await;
    let text = shown_error(&refused);
    assert_eq!(over, 0, "no read past the head");
    assert!(
        text.starts_with(&format!(
            "Not opened: lloyd.wav states no length, and at {:.0} MB it can be too long to open in this browser now",
            (cap + 1) as f64 / 1e6
        )),
        "{text}"
    );
    let text = shown_error(&stopped);
    assert!(
        text.starts_with(
            "Not opened: this song is longer than 0:01, too long to open in this browser now."
        ),
        "{text}"
    );
    assert_eq!(fits.expect("within 2 s").song.frames, 2 * SECOND);
}

/// D3 of the catch-up plan and D2 of the M7 plan: 21 sources do not fit
/// the read-ahead buffer, so the page would decode them.  They hold 28
/// lanes, so the limit is a quarter: the drop is refused, and storage
/// does not help.  No decode is posted.
#[wasm_bindgen_test]
async fn a_drop_over_the_read_ahead_cap_refuses_a_long_song() {
    store::test_wipe().await;
    let chosen: Chosen = (0..21)
        .map(|k| wav_file(&format!("lloyd {k}.wav"), 100, k))
        .collect();
    let fake = Fake::new();
    let opened = open_chosen_now(&chosen, &fake.client, 399).await;
    let sessions = store::list().await.len();
    let fits = open_chosen_now(&chosen, &fake.client, 400).await;
    store::test_wipe().await;
    assert_eq!(fake.posts.length(), 0, "no decode is posted");
    let text = shown_error(&opened);
    assert!(
        text.starts_with("Not opened: this song is 0:00 long, too long to open in this browser now.  On this page, a song of 21 files opens only up to 0:00, and the page decodes it here because its 21 lanes do not fit the read-ahead buffer"),
        "{text}"
    );
    assert!(
        text.ends_with("buffer, so it plays from memory.  Open a shorter song."),
        "{text}"
    );
    assert_eq!(sessions, 0, "the refused song's session goes");
    assert_eq!(fits.expect("100 frames of 400 / 4").song.frames, 100);
}

/// D3 of the catch-up plan: a decode worker that cannot keep the song
/// (`decode-unstored`) sends it to the page, which refuses it past the
/// limit, from its head.
#[wasm_bindgen_test]
async fn unstored_refuses_a_long_song_from_its_head() {
    store::test_wipe().await;
    let fake = Fake::new();
    let chosen = vec![wav_file("lloyd.wav", 2 * SECOND, 0)];
    let client = fake.client.clone();
    dt_start_size_spy(engine::decode::HEAD_BYTES as u32);
    let opened = background(async move { open_chosen_now(&chosen, &client, SECOND).await });
    let req = fake.request(0).await;
    fake.reply(DecodeReply::Unstored {
        text: "QuotaExceededError: the disk is full".into(),
    });
    let opened = output(&opened).await;
    let sizes = dt_stop_size_spy();
    let kept = meta_of(&id_of(&req.session)).await;
    store::test_wipe().await;
    let text = shown_error(&opened);
    assert_eq!(
        sizes[1], 0.0,
        "no read past the head: the largest was {}",
        sizes[0]
    );
    assert!(
        text.starts_with("Not opened: this song is 0:02 long, too long to open in this browser now.  On this page, a song of one file opens only up to 0:01, and the page decodes it here because browser storage failed (QuotaExceededError: the disk is full).  Free browser storage"),
        "{text}"
    );
    assert!(kept.is_none(), "the refused song's new session goes");
}

/// D3 of the catch-up plan: a stored session of 21 sources opens on the
/// page (D2 of the M7 plan).  With no `mix` or `decoded` line, the page
/// reads only the head of each source to refuse it.  With a `mix` line, it
/// reads no source at all.  The session stays.
#[wasm_bindgen_test]
async fn a_restore_on_the_page_refuses_a_long_song_from_heads_or_meta() {
    store::test_wipe().await;
    let frames = 20_000;
    let chosen: Chosen = (0..21)
        .map(|k| wav_file(&format!("lloyd {k}.wav"), frames, k))
        .collect();
    let id = saved(&chosen).await;
    let fake = Fake::new();
    let limit = 4 * (frames - 1);
    let (from_heads, _, over) = reads_of(restore_stored(&id, &fake.client, limit)).await;
    let mut meta = meta_of(&id).await.expect("meta");
    meta.mix = Some((frames, store_meta::MIX_FORMAT.into()));
    store::test_write(&format!("sessions/{id}/meta"), meta.to_text().as_bytes())
        .await
        .expect("planted");
    dt_start_read_spy();
    let from_meta = restore_stored(&id, &fake.client, limit).await;
    let source_reads = dt_stop_read_spy();
    let kept = meta_of(&id).await;
    store::test_wipe().await;
    assert_eq!(fake.posts.length(), 0, "no decode is posted");
    assert_eq!(over, 0, "no read past a head");
    for (case, restored) in [("heads", from_heads), ("meta", from_meta)] {
        let error = restored.expect("a session").err().expect("refused");
        let text = open_error_text("Could not load", &error);
        assert!(
            text.starts_with("Not opened: this song is 0:00 long, too long to open in this browser now.  On this page, a song of 21 files opens only up to 0:00")
                && text.ends_with("Open a shorter song."),
            "{case}: {text}"
        );
    }
    assert_eq!(source_reads, 0, "meta gives the length: no source is read");
    assert!(kept.is_some(), "the session stays");
}

/// D3 of the catch-up plan: a restore whose decode worker replies
/// `decode-unstored` decodes on the page, and refuses a long song from the
/// head of its source; the status line offers to free storage.
#[wasm_bindgen_test]
async fn an_unstored_restore_refuses_a_long_song() {
    store::test_wipe().await;
    let fake = Fake::new();
    let id = saved(&vec![wav_file("lloyd.wav", 2 * SECOND, 0)]).await;
    let client = fake.client.clone();
    let restore_id = id.clone();
    let restored = background(async move { restore_stored(&restore_id, &client, SECOND).await });
    let _ = fake.request(0).await;
    fake.reply(DecodeReply::Unstored {
        text: "QuotaExceededError: the disk is full".into(),
    });
    let restored = output(&restored).await.expect("a session");
    store::test_wipe().await;
    let error = restored.err().expect("refused");
    let text = open_error_text("Could not restore the stored song", &error);
    assert!(
        text.starts_with(
            "Not opened: this song is 0:02 long, too long to open in this browser now."
        ) && text.contains("browser storage failed (QuotaExceededError: the disk is full)")
            && text.ends_with("Free browser storage in the Storage menu, or open a shorter song."),
        "{text}"
    );
}

/// Six WAVs of 1 s, each padded with zeros after its data to `bytes`
/// bytes: a song whose files are much larger than its lanes.
fn six_padded_wavs(bytes: usize) -> Chosen {
    ["bass", "drums", "other", "vocals", "guitar", "piano"]
        .iter()
        .enumerate()
        .map(|(k, stem)| {
            let wav = engine::wav::encode_pcm16(&tone(SECOND, k), &tone(SECOND, k + 8), 44_100);
            let name = format!("{stem}.wav");
            let file = dt_big_file(&wav, bytes as f64, &name);
            (name, file)
        })
        .collect()
}

/// The bytes of each file of [`six_padded_wavs`] in the tests below.
const PADDED_BYTES: usize = 6_000_000;

/// Finding 1-3 of review round 1 of the catch-up plan (MEM-4): a decode
/// on the page reads, decodes and drops one file at a time.  Six files of
/// 6 MB open with no browser storage.  The heap peaks at less than two
/// files above its start, not at the six files' 36 MB.
#[wasm_bindgen_test]
async fn a_page_open_holds_one_file_at_a_time() {
    store::test_wipe().await;
    let fake = Fake::new();
    let chosen = six_padded_wavs(PADDED_BYTES);
    dt_refuse_writes();
    let start = store::tests::heap_start();
    dt_start_size_spy(picker::READ_BLOCK as u32);
    let opened = open_chosen_now(&chosen, &fake.client, 2 * SECOND).await;
    let sizes = dt_stop_size_spy();
    let peak = store::tests::heap_peak_above(start);
    dt_allow_writes();
    store::test_wipe().await;
    let song = opened.expect("opened").song;
    assert_eq!((song.frames, song.lanes.len()), (SECOND, 6));
    assert!(
        peak < 2 * PADDED_BYTES,
        "the heap peaked {peak} bytes above its start, for files of {PADDED_BYTES} bytes"
    );
    assert_one_block_at_a_time(&sizes);
}

/// Round 2 finding 2-3 (MEM-4): JavaScript holds at most one block of a
/// file (`picker::READ_BLOCK`), never a whole file, from `sizes` of
/// `dt_stop_size_spy`.
fn assert_one_block_at_a_time(sizes: &[f64]) {
    let (largest, over) = (sizes[0] as usize, sizes[1] as u32);
    assert_eq!(
        over, 0,
        "reads past one block; the largest is {largest} bytes"
    );
    assert!(largest <= picker::READ_BLOCK, "{largest}");
}

/// Finding 1-3 of review round 1 of the catch-up plan (MEM-4): a restore
/// that decodes on the page (the decode worker replies `decode-unstored`)
/// reads, decodes and drops one stored source at a time.
#[wasm_bindgen_test]
async fn a_page_restore_holds_one_file_at_a_time() {
    store::test_wipe().await;
    let fake = Fake::new();
    let id = saved(&six_padded_wavs(PADDED_BYTES)).await;
    let client = fake.client.clone();
    let restore_id = id.clone();
    let start = store::tests::heap_start();
    dt_start_size_spy(picker::READ_BLOCK as u32);
    let restored =
        background(async move { restore_stored(&restore_id, &client, 2 * SECOND).await });
    let _ = fake.request(0).await;
    fake.reply(DecodeReply::Unstored {
        text: "QuotaExceededError: the disk is full".into(),
    });
    let restored = output(&restored).await;
    let sizes = dt_stop_size_spy();
    let peak = store::tests::heap_peak_above(start);
    store::test_wipe().await;
    let (opened, _, _) = restored.expect("a session").expect("opened");
    assert_eq!((opened.song.frames, opened.song.lanes.len()), (SECOND, 6));
    assert!(
        peak < 2 * PADDED_BYTES,
        "the heap peaked {peak} bytes above its start, for files of {PADDED_BYTES} bytes"
    );
    assert_one_block_at_a_time(&sizes);
}

/// Finding 1-3 of review round 1 of the catch-up plan: a WAV whose head
/// states 1 s, and that is larger than any file of 1 s, is refused from
/// its head, with no read past it.
#[wasm_bindgen_test]
async fn a_file_larger_than_its_stated_length_allows_is_refused_from_its_head() {
    store::test_wipe().await;
    let fake = Fake::new();
    let wav = engine::wav::encode_pcm16(&tone(SECOND, 0), &tone(SECOND, 8), 44_100);
    let cap =
        engine::decode::largest_file(engine::decode::HeadLength::Frames(SECOND as u64), SECOND);
    let chosen = vec![(
        "lloyd.wav".to_string(),
        dt_big_file(&wav, (cap + 1) as f64, "lloyd.wav"),
    )];
    dt_refuse_writes();
    let (refused, largest, over) = reads_of(open_chosen_now(&chosen, &fake.client, SECOND)).await;
    dt_allow_writes();
    store::test_wipe().await;
    let text = shown_error(&refused);
    assert_eq!(over, 0, "no read past the head: the largest was {largest}");
    assert!(
        text.starts_with(&format!(
            "Not opened: lloyd.wav is {:.0} MB, more than a file of its length holds (at most {:.0} MB)",
            (cap + 1) as f64 / 1e6,
            cap as f64 / 1e6
        )),
        "{text}"
    );
}

/// Finding 1-3 of review round 1 of the catch-up plan: a restore on the
/// page whose `meta` gives the song's length refuses a source larger than
/// any file of that length, from its size alone: no source is read.
#[wasm_bindgen_test]
async fn a_restore_from_meta_refuses_a_file_larger_than_its_length_allows() {
    store::test_wipe().await;
    let frames = 2_000;
    let cap = engine::decode::largest_file(engine::decode::HeadLength::Frames(frames), 4 * SECOND);
    let mut chosen: Chosen = (0..21)
        .map(|k| wav_file(&format!("lloyd {k}.wav"), frames as usize, k))
        .collect();
    let wav = engine::wav::encode_pcm16(&tone(2_000, 3), &tone(2_000, 9), 44_100);
    chosen[3].1 = dt_big_file(&wav, (cap + 1) as f64, "lloyd 3.wav");
    let id = saved(&chosen).await;
    let mut meta = meta_of(&id).await.expect("meta");
    meta.mix = Some((frames as usize, store_meta::MIX_FORMAT.into()));
    store::test_write(&format!("sessions/{id}/meta"), meta.to_text().as_bytes())
        .await
        .expect("planted");
    let fake = Fake::new();
    dt_start_read_spy();
    let restored = restore_stored(&id, &fake.client, 4 * SECOND).await;
    let source_reads = dt_stop_read_spy();
    store::test_wipe().await;
    let error = restored.expect("a session").err().expect("refused");
    let text = open_error_text("Could not load", &error);
    assert!(
        text.starts_with(&format!(
            "Not opened: lloyd 3.wav is {:.0} MB, more than a file of its length holds",
            (cap + 1) as f64 / 1e6
        )),
        "{text}"
    );
    assert_eq!(source_reads, 0, "no source is read");
}
