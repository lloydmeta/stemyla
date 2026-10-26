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
// *.wav, *.mp3 or *.flac), until stopped.  Reads of `meta` do not count.
let dt_reads = null;
let dt_real = null;
export function dt_start_read_spy() {
  const proto = Blob.prototype;
  dt_real = ["arrayBuffer", "text", "stream", "bytes"].map((k) => [k, proto[k]]);
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
        let song = &opened.song;
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
    let song = opened.expect("opened").song;
    assert_eq!(song.stems[0].name, "drums");
    assert_eq!(song.stems[1].name, "classic drums");
    assert_eq!(
        mix,
        Some(interleaved(&samples[0].0, &samples[0].1)),
        "the mix is drums"
    );
}

#[wasm_bindgen_test]
async fn undecodable_shows_the_error_and_forgets_the_session() {
    store::test_wipe().await;
    let fake = Fake::new();
    let chosen = vec![wav_file("lloyd.wav", 100, 0)];
    let client = fake.client.clone();
    let opened = background(async move { open_chosen_now(&chosen, &client).await });
    let req = fake.request(0).await;
    fake.reply(DecodeReply::Undecodable {
        text: "unsupported or unrecognised format: lloyd".into(),
    });
    let opened = output(&opened).await;
    let kept = meta_of(&id_of(&req.session)).await;
    store::test_wipe().await;
    let error = opened.err().map(|e| format!("{e:#}"));
    assert_eq!(
        error.as_deref(),
        Some("unsupported or unrecognised format: lloyd")
    );
    assert_eq!(kept, None, "the new session is forgotten");
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
    store::test_wipe().await;
    assert_eq!(opened.song.frames, 1_234);
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
    let (song, meta, note) = restored.expect("a session").expect("restored");
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
    let (song, ..) = restored;
    assert!(
        *song.stems[0].left == left && *song.stems[0].right == right,
        "read from the mix"
    );
    assert_eq!(kept_mix, Some(mix), "the mix stays");
    assert_eq!(meta.decoded.map(|d| d.frames), Some(frames), "now complete");
    assert_eq!(meta.mix, Some((frames, store_meta::MIX_FORMAT.into())));
}

/// D9: a reset ends the decode worker first, so its guard goes and
/// `store::close` does not wait for the decode.
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
    let ended = app.decoder.decode(&request, store::DecodeGuard::new(&id));
    let held = (store::decoding(&id), store::writes_in_progress());
    app.stop_workers();
    let after = (store::decoding(&id), store::writes_in_progress());
    let ended = ended.await;
    store::test_wipe().await;
    assert_eq!(held, (true, 1), "the decode holds a write");
    assert_eq!(after, (false, 0), "close would not wait");
    assert!(matches!(ended, decoder::Ended::Stopped(_)));
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
    app.open_song(Song {
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
