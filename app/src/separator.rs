//! Six-stem separation (BS-RoFormer SW) in a Web Worker on WebGPU.  Web only.
//!
//! The worker is the `stem-worker` crate, built by Trunk next to the app.
//! It owns the model, so the 350 MB of weights never enter the UI thread's
//! memory when the model is chosen through the file picker.  Results come
//! back as transferred `Float32Array`s, one message per stem.
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
use js_sys::{Array, Float32Array, Object, Reflect};
use wasm_bindgen::prelude::*;
use web_sys::{MessageEvent, Worker, WorkerOptions, WorkerType};

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
    Mp3 {
        name: String,
        bytes: Vec<u8>,
    },
}

pub struct Separator {
    worker: Worker,
    events: Rc<RefCell<VecDeque<Event>>>,
    _onmessage: Closure<dyn FnMut(MessageEvent)>,
    _onerror: Closure<dyn FnMut(web_sys::Event)>,
    /// Extra fields sent with every model and separate message.
    settings: Vec<(String, JsValue)>,
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

fn get(o: &JsValue, k: &str) -> JsValue {
    Reflect::get(o, &JsValue::from_str(k)).unwrap_or(JsValue::UNDEFINED)
}

fn object(fields: &[(&str, JsValue)], extra: &[(String, JsValue)]) -> Object {
    let o = Object::new();
    for (k, v) in fields {
        let _ = Reflect::set(&o, &JsValue::from_str(k), v);
    }
    for (k, v) in extra {
        let _ = Reflect::set(&o, &JsValue::from_str(k), v);
    }
    o
}

/// Worker settings from the page URL (see the module docs).
fn url_settings() -> Vec<(String, JsValue)> {
    let mut out = Vec::new();
    let Some(search) = web_sys::window().and_then(|w| w.location().search().ok()) else {
        return out;
    };
    for pair in search.trim_start_matches('?').split('&') {
        let Some((k, v)) = pair.split_once('=') else {
            continue;
        };
        match k {
            "chunk" | "overlap" | "max_elems" | "step_limit" | "bench_quick" | "bench_rounds" => {
                if let Ok(n) = v.parse::<f64>() {
                    out.push((k.to_string(), JsValue::from_f64(n)));
                }
            }
            "gpu" | "selftest" | "mm" | "model_url" | "bench" | "profile" | "attn" => {
                out.push((k.to_string(), JsValue::from_str(v)))
            }
            _ => {}
        }
    }
    out
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
        let (ev, w, init, repaint) = (
            events.clone(),
            worker.clone(),
            object(&[("type", "init".into())], &settings),
            ctx.clone(),
        );
        let onmessage = Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
            let d = e.data();
            let text = || get(&d, "text").as_string().unwrap_or_default();
            let num = |k: &str| get(&d, k).as_f64().unwrap_or(0.0);
            let event = match get(&d, "type").as_string().as_deref() {
                Some("alive") => {
                    // The worker only listens once its wasm has started.
                    let _ = w.post_message(&init);
                    None
                }
                Some("model") => Some(Event::Model {
                    ok: get(&d, "ok").is_truthy(),
                    loaded: get(&d, "loaded").is_truthy(),
                    stored: get(&d, "stored").is_truthy(),
                    text: text(),
                }),
                Some("progress") => Some(Event::Progress {
                    done: num("done") as u32,
                    total: num("total") as u32,
                    secs: num("secs"),
                }),
                Some("stem") => {
                    let ch = |k: &str| {
                        get(&d, k)
                            .dyn_into::<Float32Array>()
                            .map(|a| a.to_vec())
                            .unwrap_or_default()
                    };
                    Some(Event::Stem {
                        name: get(&d, "name").as_string().unwrap_or_default(),
                        left: ch("left"),
                        right: ch("right"),
                    })
                }
                Some("done") => Some(Event::Done { secs: num("secs") }),
                Some("error") => Some(Event::Error {
                    of: get(&d, "of").as_string().unwrap_or_default(),
                    text: text(),
                }),
                Some("status") => Some(Event::Status(text())),
                Some("log") => Some(Event::Log(text())),
                Some("selftest") => Some(Event::Selftest {
                    ok: get(&d, "ok").is_truthy(),
                    text: text(),
                }),
                Some("bench") => Some(Event::Bench(text())),
                Some("mp3") => Some(Event::Mp3 {
                    name: get(&d, "name").as_string().unwrap_or_default(),
                    bytes: get(&d, "bytes")
                        .dyn_into::<js_sys::Uint8Array>()
                        .map(|a| a.to_vec())
                        .unwrap_or_default(),
                }),
                Some("download") => Some(Event::Download {
                    done: num("done"),
                    total: num("total"),
                }),
                _ => None,
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
        let (ev, dead) = (events.clone(), worker.clone());
        let onerror = Closure::<dyn FnMut(web_sys::Event)>::new(move |e: web_sys::Event| {
            dead.terminate();
            let reason = get(&e, "message")
                .as_string()
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
        let msg = object(
            &[("type", "model".into()), ("file", file.into())],
            &self.settings,
        );
        let sent = self.worker.post_message(&msg).is_ok();
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
                let msg = object(
                    &[("type", "model".into()), ("file", file.into())],
                    &settings,
                );
                if worker.post_message(&msg).is_ok() {
                    events.borrow_mut().push_back(Event::ModelLoading);
                }
            }
        });
    }

    /// Ends the worker at once, for example before its stored files are
    /// deleted.
    pub fn stop(&mut self) {
        self.worker.terminate();
        self.running = false;
        self.model_loading = false;
        self.stopped
            .get_or_insert_with(|| "stopped for a reset".into());
    }

    /// Asks the worker to download the model (from Hugging Face unless
    /// `?model_url=` names another file) and keep it in browser storage.
    pub fn download_model(&mut self) {
        let msg = object(&[("type", "download".into())], &self.settings);
        if self.worker.post_message(&msg).is_ok() {
            self.model_loading = true;
            self.download = Some((0.0, 0.0));
        }
    }

    /// Asks the worker to encode one stem as MP3; the file comes back as
    /// [`Event::Mp3`].
    pub fn encode_mp3(&self, name: &str, left: &[f32], right: &[f32], rate: u32) {
        let (l, r) = (Float32Array::from(left), Float32Array::from(right));
        let msg = object(
            &[
                ("type", "mp3".into()),
                ("name", name.into()),
                ("left", l.clone().into()),
                ("right", r.clone().into()),
                ("rate", rate.into()),
            ],
            &[],
        );
        let transfer: Array = [JsValue::from(l.buffer()), JsValue::from(r.buffer())]
            .into_iter()
            .collect();
        let _ = self.worker.post_message_with_transfer(&msg, &transfer);
    }

    /// Asks the worker to delete the model from browser storage and drop it.
    pub fn forget_model(&mut self) {
        let msg = object(&[("type", "forget_model".into())], &self.settings);
        if self.worker.post_message(&msg).is_ok() {
            self.model_ready = false;
            self.model_loading = true;
        }
    }

    /// Asks the worker to time every matmul kernel on the model's shapes.
    pub fn bench(&mut self) {
        let msg = object(&[("type", "bench".into())], &self.settings);
        let _ = self.worker.post_message(&msg);
        self.bench_pending = false;
    }

    /// Asks the worker to compare its GPU results with the reference files.
    pub fn selftest(&mut self) {
        let msg = object(&[("type", "selftest".into())], &self.settings);
        let _ = self.worker.post_message(&msg);
        self.selftest_pending = false;
    }

    /// Starts separating a 44.1 kHz stereo mix.
    pub fn separate(&mut self, left: &[f32], right: &[f32]) {
        let (l, r) = (Float32Array::from(left), Float32Array::from(right));
        let msg = object(
            &[
                ("type", "separate".into()),
                ("left", l.clone().into()),
                ("right", r.clone().into()),
            ],
            &self.settings,
        );
        let transfer: Array = [JsValue::from(l.buffer()), JsValue::from(r.buffer())]
            .into_iter()
            .collect();
        if self
            .worker
            .post_message_with_transfer(&msg, &transfer)
            .is_ok()
        {
            self.running = true;
            self.progress = (0, 0, 0.0);
            self.stems.clear();
        }
    }
}
