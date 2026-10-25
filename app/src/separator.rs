//! Six-stem separation (BS-RoFormer SW) in a Web Worker on WebGPU.  Web only.
//!
//! The worker is the `stem-worker` crate, built by Trunk next to the app.
//! It owns the model, so the 350 MB of weights never enter the UI thread's
//! memory when the model is chosen through the file picker.
//!
//! A separation takes one of two paths (M3 of
//! `plans/active/stream-from-storage.md`).  On the storage path, the
//! request names the song's session, and the worker writes the stems to
//! its files.  Its `stored` replies carry no samples, and `done` gives
//! [`Event::StemsStored`].  On the memory path, the stems come back after
//! each chunk as transferred `Float32Array`s holding every stem's finished
//! samples, put together here (see [`crate::assemble`]).
//!
//! Page query parameters, for tests and slow machines: `chunk` (samples per
//! model call), `overlap` (1 halves the work), `max_elems` (largest
//! intermediate per GPU step, in f32s), `gpu=integrated` (ask for the
//! low-power GPU), `step_limit` (seconds one GPU step may take before the
//! run stops; default 5), `selftest` (once the model is loaded, compare
//! the GPU with the CPU reference files served under /selftest/ and log the
//! result; `selftest=stages` skips the full 13 s chunk, for slow software
//! GPUs), `bench` (time every matmul kernel on the model's shapes, or only
//! those listed, as in `bench=i64`; see `roformer_gpu::bench`),
//! `bench_rounds` (interleaved rounds per kernel and shape; default 3),
//! `profile=1` (GPU time per kernel from timestamp queries, logged after
//! the self-test's full chunk and after a separation) and `mm` (`auto` or
//! a kernel name from `roformer_gpu::TILES`, to compare kernels end to end)
//! and `attn` (`auto`, `split`, `fused`, or `time` / `freq` for fused
//! attention on that axis only; by default each GPU's measured choice).

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use eframe::egui;
use js_sys::{Float32Array, Uint8Array};
use stem_protocol::{ModelSource, Reply, Request, SegmentStem, Settings, kind};
use wasm_bindgen::prelude::*;
use web_sys::{MessageEvent, Worker, WorkerOptions, WorkerType};

use crate::assemble::Assembly;

/// Written by Trunk's loader shim (`data-loader-shim`, file hashing off).
/// The worker runs the network with the WGSL kernels of `roformer-gpu`.
const WORKER_URL: &str = crate::reset::WORKER_LOADER;

pub enum Event {
    ModelLoading,
    /// The reply to a model request.  `ok`: whether it succeeded; `loaded`
    /// and `stored`: what the worker holds afterwards.
    Model {
        ok: bool,
        loaded: bool,
        stored: bool,
        text: String,
    },
    Progress {
        done: u32,
        total: u32,
        secs: f64,
    },
    Stem {
        name: String,
        left: Vec<f32>,
        right: Vec<f32>,
    },
    /// A storage-path separation ended: each stem's file holds the song's
    /// `len` frames.  `names` are the stems in the worker's order.
    StemsStored {
        names: Vec<String>,
        frames: usize,
        len: usize,
        secs: f64,
    },
    /// Browser storage could not hold the stems; the separation stopped.
    Unstored(String),
    Done {
        secs: f64,
    },
    /// A request failed; `of` is its message type ("separate", "mp3", ...).
    Error {
        of: String,
        text: String,
    },
    /// Something for the status line, such as a wait for another tab.
    Status(String),
    /// The worker died or its script did not load; it never answers again.
    Stopped(String),
    Log(String),
    Selftest {
        ok: bool,
        text: String,
    },
    Bench(String),
    Download {
        done: f64,
        total: f64,
    },
    /// The encoded file, as the worker transferred it.
    Mp3 {
        name: String,
        bytes: Uint8Array,
    },
    /// The encoded file, as the worker sent it.
    Flac {
        name: String,
        file: web_sys::Blob,
    },
}

pub struct Separator {
    worker: Worker,
    events: Rc<RefCell<VecDeque<Event>>>,
    _onmessage: Closure<dyn FnMut(MessageEvent)>,
    _onerror: Closure<dyn FnMut(web_sys::Event)>,
    /// The separation's stems as their segments arrive (memory path).
    assembly: Rc<RefCell<Assembly>>,
    /// `Some` on the storage path, with the last `stored` reply.
    storage: Rc<RefCell<Option<StoredSoFar>>>,
    /// Sent with every request but MP3 and FLAC encoding.
    settings: Settings,
    pub model_ready: bool,
    /// Whether browser storage holds a model file, usable or not.
    pub model_stored: bool,
    pub model_loading: bool,
    pub running: bool,
    /// Why the worker stopped, once it has.
    pub stopped: Option<String>,
    /// (chunks done, chunks total, seconds so far)
    pub progress: (u32, u32, f64),
    pub stems: Vec<(String, Vec<f32>, Vec<f32>)>,
    /// `?selftest`: run the self-test once the model is ready.
    pub selftest_pending: bool,
    /// `?bench`: time the matmul kernels once the worker has a GPU.
    pub bench_pending: bool,
    /// Bytes downloaded and expected while the model downloads.
    pub download: Option<(f64, f64)>,
}

fn has_param(name: &str) -> bool {
    web_sys::window()
        .and_then(|w| w.location().search().ok())
        .is_some_and(|s| {
            s.trim_start_matches('?')
                .split('&')
                .any(|p| p == name || p.starts_with(&format!("{name}=")))
        })
}

/// Worker settings from the page URL (see the module docs).
fn url_settings() -> Settings {
    web_sys::window()
        .and_then(|w| w.location().href().ok())
        .map(|href| Settings::from_page(&href))
        .unwrap_or_default()
}

/// The last `stored` reply of a storage-path separation: the frames in
/// each stem's file, the song's frames and the stem names.
type StoredSoFar = Option<(usize, usize, Vec<String>)>;

/// What `done` means on the storage path: the stems, if the last `stored`
/// reply covers the whole song.
fn stored_at_done(last: StoredSoFar, secs: f64) -> Event {
    let error = |text: String| Event::Error {
        of: kind::SEPARATE.into(),
        text,
    };
    match last {
        Some((frames, len, names)) if frames == len => Event::StemsStored {
            names,
            frames,
            len,
            secs,
        },
        Some((frames, len, _)) => error(format!(
            "the separation ended with {frames} of {len} frames stored"
        )),
        None => error("the separation ended before any stems were stored".into()),
    }
}

/// Posts `request`, transferring what it carries; false if it could not
/// be sent.
fn post(worker: &Worker, request: &Request, settings: &Settings) -> bool {
    let (msg, transfer) = request.to_js(settings);
    worker.post_message_with_transfer(&msg, &transfer).is_ok()
}

/// Megabytes of the page's WebAssembly memory, which only ever grows: on iOS
/// the whole tab is stopped at a fixed limit.
pub fn page_wasm_mb() -> f64 {
    let mem: js_sys::WebAssembly::Memory = wasm_bindgen::memory().unchecked_into();
    mem.buffer()
        .unchecked_into::<js_sys::ArrayBuffer>()
        .byte_length() as f64
        / 1e6
}

/// Copies one segment (every stem's next samples, as transferred
/// `Float32Array`s) into the song-length stems (see [`Assembly::segment`]).
fn place_stems(
    asm: &mut Assembly,
    start: usize,
    len: usize,
    stems: &[SegmentStem],
) -> Result<(), String> {
    let mut end = None;
    for (
        i,
        SegmentStem {
            name,
            left: l,
            right: r,
        },
    ) in stems.iter().enumerate()
    {
        let n = l.length() as usize;
        if r.length() as usize != n {
            return Err(format!("segment of {name}: channels differ in length"));
        }
        if *end.get_or_insert(start + n) != start + n {
            return Err(format!("segment of {name}: stems differ in length"));
        }
        asm.place(
            i,
            name,
            start,
            len,
            |ld, rd| {
                l.copy_to(ld);
                r.copy_to(rd);
            },
            n,
        )?;
    }
    asm.advance(stems.len(), end.unwrap_or(start))
}

impl Separator {
    pub fn new(ctx: egui::Context) -> Result<Self, String> {
        // Ask the browser not to evict the cached model under storage
        // pressure (granted or refused silently, by its own heuristics).
        if let Some(w) = web_sys::window() {
            let _ = w.navigator().storage().persist();
        }
        let opts = WorkerOptions::new();
        opts.set_type(WorkerType::Module);
        let worker = Worker::new_with_options(WORKER_URL, &opts).map_err(|e| format!("{e:?}"))?;
        let events: Rc<RefCell<VecDeque<Event>>> = Default::default();
        let settings = url_settings();
        let assembly: Rc<RefCell<Assembly>> = Default::default();
        let storage: Rc<RefCell<Option<StoredSoFar>>> = Default::default();
        let (asm, path) = (assembly.clone(), storage.clone());
        let (ev, w, init_settings, repaint) = (
            events.clone(),
            worker.clone(),
            settings.clone(),
            ctx.clone(),
        );
        let onmessage = Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
            let reply = match Reply::from_js(&e.data()) {
                Ok(reply) => reply,
                // A segment that cannot be read fails its separation, as one
                // that does not fit does.
                Err(bad) if bad.kind == kind::SEGMENT => {
                    let failed = asm.borrow_mut().segment(true, |_| Err(bad.text));
                    if let Err(text) = failed {
                        ev.borrow_mut().push_back(Event::Error {
                            of: kind::SEPARATE.into(),
                            text,
                        });
                        ctx.request_repaint();
                    }
                    return;
                }
                Err(bad) => {
                    log::warn!("separator: {}", bad.text);
                    return;
                }
            };
            let event = match reply {
                Reply::Alive => {
                    // The worker only listens once its wasm has started.
                    post(&w, &Request::Init, &init_settings);
                    None
                }
                Reply::Model {
                    ok,
                    loaded,
                    stored,
                    text,
                } => Some(Event::Model {
                    ok,
                    loaded,
                    stored,
                    text,
                }),
                Reply::Progress { done, total, secs } => {
                    Some(Event::Progress { done, total, secs })
                }
                Reply::Segment { start, len, stems } => {
                    let placed = asm
                        .borrow_mut()
                        .segment(start == 0, |asm| place_stems(asm, start, len, &stems));
                    placed.err().map(|text| Event::Error {
                        of: kind::SEPARATE.into(),
                        text,
                    })
                }
                Reply::Stored { frames, len, stems } => {
                    if let Some(last) = path.borrow_mut().as_mut() {
                        *last = Some((frames, len, stems));
                    }
                    None
                }
                Reply::Done { secs } if path.borrow().is_some() => {
                    let last = path.borrow_mut().take().flatten();
                    Some(stored_at_done(last, secs))
                }
                Reply::Done { secs } => {
                    let done = asm.borrow_mut().done();
                    match done {
                        None => None,
                        Some(Ok(stems)) => {
                            let mut q = ev.borrow_mut();
                            for (name, left, right) in stems {
                                q.push_back(Event::Stem { name, left, right });
                            }
                            Some(Event::Done { secs })
                        }
                        Some(Err(text)) => Some(Event::Error {
                            of: kind::SEPARATE.into(),
                            text,
                        }),
                    }
                }
                Reply::Unstored { text } => {
                    path.borrow_mut().take();
                    Some(Event::Unstored(text))
                }
                Reply::Error { of, text } => {
                    // An MP3, FLAC or self-test error leaves a running
                    // separation be.
                    if of == kind::SEPARATE {
                        path.borrow_mut().take();
                    }
                    let first = of != kind::SEPARATE || asm.borrow_mut().abandon();
                    first.then_some(Event::Error { of, text })
                }
                Reply::Status(text) => Some(Event::Status(text)),
                Reply::Log(text) => Some(Event::Log(text)),
                Reply::Selftest { ok, text } => Some(Event::Selftest { ok, text }),
                Reply::Bench(text) => Some(Event::Bench(text)),
                Reply::Mp3 { name, bytes } => Some(Event::Mp3 { name, bytes }),
                Reply::Flac { name, file } => Some(Event::Flac { name, file }),
                Reply::Download { done, total } => Some(Event::Download { done, total }),
            };
            if let Some(event) = event {
                ev.borrow_mut().push_back(event);
                ctx.request_repaint();
            }
        });
        worker.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
        // A panic inside the worker (for example after the GPU driver
        // aborted a command buffer) ends the worker; tell the user instead
        // of leaving the progress bar running.  A worker whose script did
        // not load gets a plain `Event` with no message, not an `ErrorEvent`.
        // An uncaught error does not end the worker by itself, and one that
        // trapped while holding the storage lock would keep every other tab
        // waiting; terminating it releases the lock.
        let (ev, dead, asm) = (events.clone(), worker.clone(), assembly.clone());
        let onerror = Closure::<dyn FnMut(web_sys::Event)>::new(move |e: web_sys::Event| {
            dead.terminate();
            asm.borrow_mut().clear();
            let reason = e
                .dyn_ref::<web_sys::ErrorEvent>()
                .map(|e| e.message())
                .filter(|m| !m.is_empty())
                .unwrap_or_else(|| "its script did not load".into());
            ev.borrow_mut().push_back(Event::Stopped(reason));
            repaint.request_repaint();
        });
        worker.set_onerror(Some(onerror.as_ref().unchecked_ref()));
        Ok(Self {
            worker,
            events,
            _onmessage: onmessage,
            _onerror: onerror,
            assembly,
            storage,
            settings,
            model_ready: false,
            model_stored: false,
            // The worker is asked for its stored model as soon as it starts.
            model_loading: true,
            running: false,
            stopped: None,
            progress: (0, 0, 0.0),
            stems: Vec::new(),
            selftest_pending: has_param("selftest"),
            bench_pending: has_param("bench"),
            download: None,
        })
    }

    pub fn take_events(&mut self) -> Vec<Event> {
        self.events.borrow_mut().drain(..).collect()
    }

    /// Hands a dropped model file to the worker, which reads it; false if
    /// the worker is busy with the model or a separation.
    pub fn send_model_file(&mut self, file: &web_sys::File) -> bool {
        if self.model_loading || self.running {
            return false;
        }
        let request = Request::Model(Some(ModelSource::File(file.clone().into())));
        let sent = post(&self.worker, &request, &self.settings);
        self.model_loading |= sent;
        sent
    }

    /// Opens the browser's file picker; the chosen file goes straight to the
    /// worker as a `File`, unread on this thread.
    pub fn pick_model_file(&mut self) {
        if self.model_loading || self.running {
            return;
        }
        let (worker, settings, events) = (
            self.worker.clone(),
            self.settings.clone(),
            self.events.clone(),
        );
        crate::picker::pick_files(".safetensors", false, move |files| {
            if let Some(file) = files.into_iter().next() {
                let request = Request::Model(Some(ModelSource::File(file.into())));
                if post(&worker, &request, &settings) {
                    events.borrow_mut().push_back(Event::ModelLoading);
                }
            }
        });
    }

    /// Ends the worker at once, for example before its stored files are
    /// deleted.
    pub fn stop(&mut self) {
        self.worker.terminate();
        self.assembly.borrow_mut().clear();
        self.storage.borrow_mut().take();
        self.running = false;
        self.model_loading = false;
        self.stopped
            .get_or_insert_with(|| "stopped for a reset".into());
    }

    /// Asks the worker to download the model (from Hugging Face unless
    /// `?model_url=` names another file) and keep it in browser storage.
    pub fn download_model(&mut self) {
        if post(&self.worker, &Request::Download, &self.settings) {
            self.model_loading = true;
            self.download = Some((0.0, 0.0));
        }
    }

    /// Asks the worker to encode a stem or a mix as MP3; the file comes
    /// back as [`Event::Mp3`].  Both channels are transferred (MEM-2).
    pub fn encode_mp3(&self, name: &str, left: Float32Array, right: Float32Array, rate: u32) {
        let request = Request::Mp3 {
            name: Some(name.into()),
            left,
            right,
            rate: Some(rate),
        };
        post(&self.worker, &request, &Settings::default());
    }

    /// Asks the worker to encode a stem or a mix as a 16-bit FLAC; the file
    /// comes back as [`Event::Flac`].  Both channels are transferred (MEM-2).
    pub fn encode_flac(&self, name: &str, left: Float32Array, right: Float32Array, rate: u32) {
        let request = Request::Flac {
            name: Some(name.into()),
            left,
            right,
            rate: Some(rate),
        };
        post(&self.worker, &request, &Settings::default());
    }

    /// Asks the worker to delete the model from browser storage and drop it.
    pub fn forget_model(&mut self) {
        if post(&self.worker, &Request::ForgetModel, &self.settings) {
            self.model_ready = false;
            self.model_loading = true;
        }
    }

    /// Asks the worker to time every matmul kernel on the model's shapes.
    pub fn bench(&mut self) {
        post(&self.worker, &Request::Bench, &self.settings);
        self.bench_pending = false;
    }

    /// Asks the worker to compare its GPU results with the reference files.
    pub fn selftest(&mut self) {
        post(
            &self.worker,
            &Request::Selftest { base: None },
            &self.settings,
        );
        self.selftest_pending = false;
    }

    /// Starts separating a 44.1 kHz stereo mix: into the files of
    /// `session` (`"sessions/<id>"`, the storage path), or back to the page
    /// (`None`, the memory path).  False if it could not be sent.
    pub fn separate(&mut self, left: &[f32], right: &[f32], session: Option<&str>) -> bool {
        let request = Request::Separate {
            left: Float32Array::from(left),
            right: Float32Array::from(right),
            session: session.map(String::from),
        };
        // Before the post: a reply can only come after it.
        *self.storage.borrow_mut() = session.map(|_| None);
        self.assembly.borrow_mut().clear();
        let sent = post(&self.worker, &request, &self.settings);
        if !sent {
            self.storage.borrow_mut().take();
        }
        if sent {
            self.running = true;
            self.progress = (0, 0, 0.0);
            self.stems.clear();
        }
        sent
    }
}

#[cfg(test)]
impl Separator {
    /// The worker, so a test can spy on what the page posts, deliver a
    /// reply, or clear its `onerror` (the test page has no worker script).
    pub(crate) fn worker(&self) -> &Worker {
        &self.worker
    }
}

/// Browser tests of the requests to the worker and its replies (`cargo run
/// -p xtask -- web-test`).  `store.rs` sets the whole binary to run in a
/// page.
#[cfg(test)]
mod tests {
    use super::*;
    use js_sys::{Object, Reflect};
    use wasm_bindgen_test::*;

    #[wasm_bindgen(inline_js = r#"
export function deliver(worker, data) {
  worker.dispatchEvent(new MessageEvent("message", { data }));
}
"#)]
    extern "C" {
        /// Runs `worker`'s `onmessage` on `data` itself, with no
        /// structured clone, so the test can compare the array that the
        /// page keeps with the array sent.
        fn deliver(worker: &Worker, data: &JsValue);
    }

    #[wasm_bindgen_test]
    fn an_mp3_reply_reaches_the_page_as_sent() {
        let mut sep = Separator::new(egui::Context::default()).expect("a worker");
        let sent = Uint8Array::from(&[0xff, 0xfb, 0x90, 0x00][..]);
        let (message, _) = Reply::Mp3 {
            name: "bass.mp3".into(),
            bytes: sent.clone(),
        }
        .to_js();
        deliver(&sep.worker, &message);
        // The worker's script is not in the test page, but its error comes
        // after this synchronous test.
        let mp3 = sep.take_events().into_iter().find_map(|e| match e {
            Event::Mp3 { name, bytes } => Some((name, bytes)),
            _ => None,
        });
        // Before any assert: a failed assert aborts the test.
        sep.stop();
        let (name, bytes) = mp3.expect("an Event::Mp3");
        assert_eq!(name, "bass.mp3");
        assert!(
            Object::is(&bytes, &sent),
            "MEM-2: the page copied the MP3 that the worker transferred.  \
             Fix: keep the Uint8Array in Event::Mp3."
        );
    }

    #[wasm_bindgen(inline_js = r#"
export function spy_on_post(worker) {
  const posts = [];
  worker.postMessage = (data, transfer) => posts.push({ data, transfer });
  return posts;
}
"#)]
    extern "C" {
        /// Keeps each message that `worker` would post, with its transfer
        /// list, in place of posting it.
        fn spy_on_post(worker: &Worker) -> js_sys::Array;
    }

    /// Posts one encode request through `encode`, and returns the request
    /// and its transfer list.
    fn posted_encode(
        encode: impl FnOnce(&Separator, Float32Array, Float32Array),
    ) -> (Request, js_sys::Array, Float32Array, Float32Array) {
        let mut sep = Separator::new(egui::Context::default()).expect("a worker");
        let posts = spy_on_post(&sep.worker);
        let left = Float32Array::from(&[0.25f32, -0.5][..]);
        let right = Float32Array::from(&[0.125f32, 0.75][..]);
        encode(&sep, left.clone(), right.clone());
        sep.stop();
        assert_eq!(posts.length(), 1, "one message");
        let post = posts.get(0);
        let data = Reflect::get(&post, &"data".into()).unwrap();
        let request = stem_protocol::ToWorker::from_js(&data).expect("a request");
        let transfer = Reflect::get(&post, &"transfer".into())
            .unwrap()
            .unchecked_into();
        (request.request, transfer, left, right)
    }

    /// The request sends the page's arrays themselves, transferred.
    fn assert_sent_as_they_are(
        transfer: &js_sys::Array,
        (left, right): (&Float32Array, &Float32Array),
        (sent_left, sent_right): (&Float32Array, &Float32Array),
    ) {
        assert!(
            Object::is(sent_left, left) && Object::is(sent_right, right),
            "MEM-1: the request holds a copy of the channels"
        );
        assert_eq!(transfer.length(), 2, "MEM-2: both channels are transferred");
        assert!(Object::is(&transfer.get(0), &left.buffer()));
        assert!(Object::is(&transfer.get(1), &right.buffer()));
    }

    #[wasm_bindgen_test]
    fn encode_flac_asks_the_worker_for_a_flac_of_the_arrays() {
        let (request, transfer, left, right) = posted_encode(|sep, l, r| {
            sep.encode_flac("lloyd - bass.flac", l, r, 48_000);
        });
        match request {
            Request::Flac {
                name,
                left: l,
                right: r,
                rate,
            } => {
                assert_eq!(name.as_deref(), Some("lloyd - bass.flac"));
                assert_eq!(rate, Some(48_000));
                assert_sent_as_they_are(&transfer, (&left, &right), (&l, &r));
            }
            other => panic!("{other:?}"),
        }
    }

    #[wasm_bindgen_test]
    fn encode_mp3_asks_the_worker_for_an_mp3_of_the_arrays() {
        let (request, transfer, left, right) = posted_encode(|sep, l, r| {
            sep.encode_mp3("lloyd - mix without vocals.mp3", l, r, 44_100);
        });
        match request {
            Request::Mp3 {
                name,
                left: l,
                right: r,
                rate,
            } => {
                assert_eq!(name.as_deref(), Some("lloyd - mix without vocals.mp3"));
                assert_eq!(rate, Some(44_100));
                assert_sent_as_they_are(&transfer, (&left, &right), (&l, &r));
            }
            other => panic!("{other:?}"),
        }
    }

    /// The `data` of each message posted to `posts`, as a request.
    fn posted(posts: &js_sys::Array) -> Vec<(Request, js_sys::Array)> {
        posts
            .iter()
            .map(|post| {
                let data = Reflect::get(&post, &"data".into()).unwrap();
                let request = stem_protocol::ToWorker::from_js(&data).expect("a request");
                let transfer = Reflect::get(&post, &"transfer".into())
                    .unwrap()
                    .unchecked_into();
                (request.request, transfer)
            })
            .collect()
    }

    #[wasm_bindgen_test]
    fn separate_names_the_session_on_the_storage_path() {
        let mut sep = Separator::new(egui::Context::default()).expect("a worker");
        let posts = spy_on_post(&sep.worker);
        assert!(sep.separate(&[0.5, -0.5], &[0.25, 0.75], Some("sessions/1792640000000")));
        assert!(sep.separate(&[0.5], &[0.25], None));
        sep.stop();
        let sent = posted(&posts);
        assert_eq!(sent.len(), 2, "two messages");
        for ((request, transfer), want) in sent.iter().zip([Some("sessions/1792640000000"), None]) {
            match request {
                Request::Separate { session, .. } => assert_eq!(session.as_deref(), want),
                other => panic!("{other:?}"),
            }
            assert_eq!(transfer.length(), 2, "MEM-2: both channels are transferred");
        }
    }

    /// A `stored` reply of `frames` of a song of `len`.
    fn stored(frames: usize, len: usize) -> JsValue {
        let names = ["bass", "drums", "other", "vocals", "guitar", "piano"];
        Reply::Stored {
            frames,
            len,
            stems: names.iter().map(|n| n.to_string()).collect(),
        }
        .to_js()
        .0
        .into()
    }

    fn done() -> JsValue {
        Reply::Done { secs: 12.5 }.to_js().0.into()
    }

    /// MEM-1, MEM-4: on the storage path the page keeps no stem samples
    /// while the separation runs: `Assembly` would add about 508 MB for a
    /// song of 4 minutes.
    #[wasm_bindgen_test]
    fn stored_replies_make_no_song_length_buffers() {
        let len = 10_584_000;
        let mut sep = Separator::new(egui::Context::default()).expect("a worker");
        let _posts = spy_on_post(&sep.worker);
        assert!(sep.separate(&[0.5], &[0.25], Some("sessions/1792640000000")));
        let before = page_wasm_mb();
        for frames in [0, len / 3, 2 * len / 3, len] {
            deliver(&sep.worker, &stored(frames, len));
        }
        deliver(&sep.worker, &done());
        let grown = page_wasm_mb() - before;
        let events = sep.take_events();
        let assembled = sep.assembly.borrow().stems.len();
        let whole = events.iter().find_map(|e| match e {
            Event::StemsStored {
                names,
                frames,
                len,
                secs,
            } => Some((names.clone(), *frames, *len, *secs)),
            _ => None,
        });
        // A second separation whose stems end short of the song.
        assert!(sep.separate(&[0.5], &[0.25], Some("sessions/1792640000000")));
        deliver(&sep.worker, &stored(len - 1, len));
        deliver(&sep.worker, &done());
        let short = sep.take_events().into_iter().find_map(|e| match e {
            Event::Error { of, text } => Some((of, text)),
            _ => None,
        });
        sep.stop();
        let (names, frames, got_len, secs) = whole.expect("an Event::StemsStored");
        assert_eq!(names.len(), 6);
        assert_eq!(names[0], "bass");
        assert_eq!((frames, got_len, secs), (len, len, 12.5));
        assert_eq!(assembled, 0, "Assembly stays empty");
        assert!(
            grown < 1.0,
            "MEM-4: the page grew by {grown} MB during a stored separation"
        );
        let (of, text) = short.expect("an error for stems short of the song");
        assert_eq!(of, kind::SEPARATE);
        assert!(text.contains(&format!("{} of {len}", len - 1)), "{text}");
    }

    #[wasm_bindgen_test]
    fn unstored_ends_the_separation_with_its_reason() {
        let mut sep = Separator::new(egui::Context::default()).expect("a worker");
        let _posts = spy_on_post(&sep.worker);
        assert!(sep.separate(&[0.5], &[0.25], Some("sessions/1792640000000")));
        let (reply, _) = Reply::Unstored {
            text: "QuotaExceededError: the disk is full".into(),
        }
        .to_js();
        deliver(&sep.worker, &reply);
        let unstored = sep.take_events().into_iter().find_map(|e| match e {
            Event::Unstored(text) => Some(text),
            _ => None,
        });
        sep.stop();
        assert_eq!(
            unstored.as_deref(),
            Some("QuotaExceededError: the disk is full")
        );
    }

    #[wasm_bindgen_test]
    fn a_flac_reply_reaches_the_page_as_sent() {
        let mut sep = Separator::new(egui::Context::default()).expect("a worker");
        let parts = js_sys::Array::of1(&Uint8Array::from(&b"fLaC"[..]));
        let sent = web_sys::Blob::new_with_u8_array_sequence(&parts).expect("a blob");
        let (message, _) = Reply::Flac {
            name: "bass.flac".into(),
            file: sent.clone(),
        }
        .to_js();
        deliver(&sep.worker, &message);
        let flac = sep.take_events().into_iter().find_map(|e| match e {
            Event::Flac { name, file } => Some((name, file)),
            _ => None,
        });
        // Before any assert: a failed assert aborts the test.
        sep.stop();
        let (name, file) = flac.expect("an Event::Flac");
        assert_eq!(name, "bass.flac");
        assert!(
            Object::is(&file, &sent),
            "MEM-1: the page made a new file from the FLAC that the worker sent.  \
             Fix: keep the Blob in Event::Flac."
        );
    }
}
