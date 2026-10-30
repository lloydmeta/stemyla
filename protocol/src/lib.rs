//! The messages between the page and its separation worker, written once
//! for both sides.  On the wire each is a plain JS object whose `type`
//! names it; the e2e scenarios in `xtask/src/e2e.rs` also post requests by
//! hand, so the format stays as it is.  The messages between the page and
//! its decode worker are in [`decode`].
//!
//! Requests ([`Request`], page to worker), each with the page's
//! [`Settings`]:
//! * `{type: "init"}`: load the model from the browser's private file
//!   system (OPFS) if a previous session kept it.
//! * `{type: "model", file: Blob}` or `{type: "model", bytes: Uint8Array}`:
//!   load a `bs_roformer_sw.*.safetensors` file and keep it.
//! * `{type: "download"}`: fetch the model (from `model_url` if set).
//! * `{type: "forget_model"}`: delete the kept model.
//! * `{type: "separate", left: Float32Array, right: Float32Array,
//!   session?: string}` or `{type: "separate", mix: string, frames,
//!   session?: string}`: separate a 44.1 kHz stereo mix ([`Mix`]).  The
//!   two arrays are the mix (buffers transferred).  Else `mix` is the path
//!   of a mix file from the root of the browser's private file system,
//!   such as `"sessions/<id>/mix.f32"`.  The file holds `frames` frames of
//!   interleaved 32-bit little-endian floats, with no header.  No samples
//!   cross.  A request with both or neither, or with `frames` that is not
//!   a whole number from 0 to `usize::MAX`, is refused.  With `session`
//!   (a folder from the same root, such as `"sessions/<id>"`), the worker
//!   writes each stem to `<session>/stems/<file>.part`, where `<file>` is
//!   [`stem_file`] of its name (the storage path).  Without it, the worker
//!   sends the stems in `segment` replies (the memory path).
//! * `{type: "selftest", base?: string}`: compare GPU results with the CPU
//!   port's reference files (`xtask roformer-selftest`).
//! * `{type: "bench"}`: time the GPU kernels.
//! * `{type: "mp3", name, left, right, rate?}`: encode one stem or mix as a
//!   192 kbps MP3 (buffers transferred, the memory path of a save).  The
//!   worker takes 44,100 Hz with no `rate`.
//! * `{type: "flac", name, left, right, rate?}`: encode one stem or mix as
//!   a 16-bit FLAC (buffers transferred, the memory path of a save).  The
//!   worker takes 44,100 Hz with no `rate`.
//! * `{type: "mp3" or "flac", name, wav, frames, out, rate}`: encode the
//!   16-bit stereo WAV file at `wav`, of `frames` frames, into the file at
//!   `out` ([`Encode::Stored`], the storage path of a save).  Both paths
//!   are from the root of the private file system, in its `saves` folder.
//!   No samples cross.  The worker removes the WAV file at the end, also
//!   when the encode fails.  A request with both arrays and `wav`, or
//!   neither, with `out` or `frames` but no `wav`, with no `out` or no
//!   `rate`, or with `frames` that is not a whole number from 0 to
//!   `usize::MAX`, is refused.  A request of either form with a `rate` that
//!   is not a whole number from 1 to `u32::MAX` is refused.  The worker
//!   also refuses a `wav` or `out`
//!   outside `saves/`, and a `wav` equal to `out`.  Each refusal gives
//!   `error`, and the worker removes nothing.
//!
//! Replies ([`Reply`], worker to page):
//! * `{type: "alive"}` once the worker listens.
//! * `{type: "model", ok, loaded, stored, text}`.
//! * `{type: "download", done, total}` while the model downloads.
//! * `{type: "progress", done, total, secs}` per chunk.
//! * `{type: "segment", start, len, stems: [[name, left, right], ...]}`
//!   after each segment on the memory path: every stem's finished samples
//!   from `start` on, of a song of `len` samples (buffers transferred).
//! * `{type: "stored", frames, len, stems: [name, ...]}` after each
//!   segment on the storage path: each stem's file holds `frames` of the song's
//!   `len` frames.  `stems` gives the stem names in the worker's order.  No
//!   samples cross.
//! * `{type: "unstored", text}`: browser storage failed, for the reason
//!   `text`.  It could not hold the stems, or could not give the stored
//!   mix (missing, of the wrong size, or not readable).  The worker
//!   stopped the separation and removed its partial stem files.  It ends
//!   the request, as `done` and `error` do.
//! * `{type: "done", secs}`, or `{type: "error", of, text}` where `of` is
//!   the request's type; a separation stops with an error as soon as
//!   WebGPU reports one (an invalid kernel would otherwise produce silent
//!   garbage).
//! * `{type: "status", text}`, `{type: "log", text}`,
//!   `{type: "selftest", ok, text}`, `{type: "bench", text}`.
//! * `{type: "mp3", name, bytes}` (buffer transferred).
//! * `{type: "flac", name, file: Blob}`: a `Blob` crosses as a reference,
//!   so the file is not copied.
//! * `{type: "mp3" or "flac", name, path}`: a stored encode wrote its file
//!   at `path`, the request's `out` ([`Encoded::Stored`]).
//! * `{type: "mp3" or "flac", name, path, failed: text}`: a stored encode
//!   failed, for the reason `text` ([`Encoded::Failed`]).  The worker
//!   removed the file at `path` and the WAV file.  A stored encode that the
//!   worker started replies with one of these two, not with `error`, so the
//!   page knows which save failed.

use js_sys::{Array, Float32Array, Object, Reflect, Uint8Array};
use wasm_bindgen::{JsCast, JsValue};
use web_sys::Blob;

pub mod decode;

/// The `type` of each message.
pub mod kind {
    pub const INIT: &str = "init";
    pub const MODEL: &str = "model";
    pub const DOWNLOAD: &str = "download";
    pub const FORGET_MODEL: &str = "forget_model";
    pub const SEPARATE: &str = "separate";
    pub const SELFTEST: &str = "selftest";
    pub const BENCH: &str = "bench";
    pub const MP3: &str = "mp3";
    pub const FLAC: &str = "flac";
    pub const ALIVE: &str = "alive";
    pub const PROGRESS: &str = "progress";
    pub const SEGMENT: &str = "segment";
    pub const STORED: &str = "stored";
    pub const UNSTORED: &str = "unstored";
    pub const DONE: &str = "done";
    pub const ERROR: &str = "error";
    pub const STATUS: &str = "status";
    pub const LOG: &str = "log";
}

/// Field names, besides `type`.
mod field {
    pub const TYPE: &str = "type";
    pub const FILE: &str = "file";
    pub const BYTES: &str = "bytes";
    pub const LEFT: &str = "left";
    pub const RIGHT: &str = "right";
    pub const BASE: &str = "base";
    pub const NAME: &str = "name";
    pub const RATE: &str = "rate";
    pub const OK: &str = "ok";
    pub const LOADED: &str = "loaded";
    pub const STORED: &str = "stored";
    pub const TEXT: &str = "text";
    pub const DONE: &str = "done";
    pub const TOTAL: &str = "total";
    pub const SECS: &str = "secs";
    pub const START: &str = "start";
    pub const LEN: &str = "len";
    pub const STEMS: &str = "stems";
    pub const FRAMES: &str = "frames";
    pub const SESSION: &str = "session";
    pub const MIX: &str = "mix";
    pub const OF: &str = "of";
    pub const WAV: &str = "wav";
    pub const OUT: &str = "out";
    pub const PATH: &str = "path";
    pub const FAILED: &str = "failed";
    pub const CHUNK: &str = "chunk";
    pub const OVERLAP: &str = "overlap";
    pub const MAX_ELEMS: &str = "max_elems";
    pub const STEP_LIMIT: &str = "step_limit";
    pub const BENCH_ROUNDS: &str = "bench_rounds";
    pub const BENCH_QUICK: &str = "bench_quick";
    pub const GPU: &str = "gpu";
    pub const SELFTEST: &str = "selftest";
    pub const MM: &str = "mm";
    pub const MODEL_URL: &str = "model_url";
    pub const BENCH: &str = "bench";
    pub const ATTN: &str = "attn";
    pub const PROFILE: &str = "profile";
    pub const DEBUG: &str = "debug";
    pub const WEIGHTS: &str = "weights";
}

/// Overrides from the page's address (`?chunk=16384&mm=s64`, see the
/// README), sent with every request.  Unset ones leave the worker's
/// defaults.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Settings {
    pub chunk: Option<usize>,
    pub overlap: Option<usize>,
    pub max_elems: Option<usize>,
    /// Seconds a GPU step may take before the run is stopped.
    pub step_limit: Option<f64>,
    pub bench_rounds: Option<f64>,
    pub bench_quick: bool,
    /// `"integrated"` asks for the low-power GPU.
    pub gpu: Option<String>,
    /// `"stages"` skips the full-chunk check.
    pub selftest: Option<String>,
    /// Matrix-multiply kernel override.
    pub mm: Option<String>,
    /// Where Download model fetches from, instead of Hugging Face.
    pub model_url: Option<String>,
    /// Which kernels `bench` times.
    pub bench: Option<String>,
    /// Attention kernel override.
    pub attn: Option<String>,
    /// How the GPU holds the weight matrices: `f32`, `f16` or `auto`.
    pub weights: Option<String>,
    /// Logs GPU time per kernel.
    pub profile: bool,
    /// Progress notes for debugging (set by tests, not the address).
    pub debug: bool,
}

impl Settings {
    /// From a page address's query, with or without its `?`.  Values are
    /// taken as written (not percent-decoded); a number that does not parse
    /// is ignored, and a flag is on unless empty or zero.
    pub fn from_query(query: &str) -> Self {
        let mut s = Self::default();
        for pair in query.trim_start_matches('?').split('&') {
            let Some((k, v)) = pair.split_once('=') else {
                continue;
            };
            let number = v.parse::<f64>().ok();
            let text = Some(v.to_string());
            match k {
                field::CHUNK => s.chunk = number.map(|n| n as usize),
                field::OVERLAP => s.overlap = number.map(|n| n as usize),
                field::MAX_ELEMS => s.max_elems = number.map(|n| n as usize),
                field::STEP_LIMIT => s.step_limit = number,
                field::BENCH_ROUNDS => s.bench_rounds = number,
                field::BENCH_QUICK => s.bench_quick = !v.is_empty() && number != Some(0.0),
                field::GPU => s.gpu = text,
                field::SELFTEST => s.selftest = text,
                field::MM => s.mm = text,
                field::MODEL_URL => s.model_url = text,
                field::BENCH => s.bench = text,
                field::ATTN => s.attn = text,
                field::WEIGHTS => s.weights = text,
                field::PROFILE => s.profile = !v.is_empty() && number != Some(0.0),
                _ => {}
            }
        }
        s
    }

    /// From the page's own address `href`, as [`Settings::from_query`],
    /// except that `model_url` is kept only for a file on the page's own
    /// site: it is not checked against a hash, so a link could otherwise
    /// make the page fetch and store a file from anywhere.
    pub fn from_page(href: &str) -> Self {
        let Ok(page) = web_sys::Url::new(href) else {
            return Self::default();
        };
        let s = Self::from_query(&page.search());
        // Empty resolves to the page itself.
        let own_site = |url: &String| {
            !url.is_empty()
                && web_sys::Url::new_with_base(url, href).is_ok_and(|u| u.origin() == page.origin())
        };
        Self {
            model_url: s.model_url.filter(own_site),
            ..s
        }
    }

    fn write(&self, o: &Object) {
        let count = |v: Option<usize>| v.map(|v| v as f64);
        for (k, v) in [
            (field::CHUNK, count(self.chunk)),
            (field::OVERLAP, count(self.overlap)),
            (field::MAX_ELEMS, count(self.max_elems)),
            (field::STEP_LIMIT, self.step_limit),
            (field::BENCH_ROUNDS, self.bench_rounds),
        ] {
            if let Some(v) = v {
                set(o, k, v);
            }
        }
        for (k, v) in [
            (field::GPU, &self.gpu),
            (field::SELFTEST, &self.selftest),
            (field::MM, &self.mm),
            (field::MODEL_URL, &self.model_url),
            (field::BENCH, &self.bench),
            (field::ATTN, &self.attn),
            (field::WEIGHTS, &self.weights),
        ] {
            if let Some(v) = v {
                set(o, k, v.as_str());
            }
        }
        for (k, v) in [
            (field::BENCH_QUICK, self.bench_quick),
            (field::PROFILE, self.profile),
            (field::DEBUG, self.debug),
        ] {
            if v {
                set(o, k, true);
            }
        }
    }

    /// As the worker reads them: numbers that are not numbers are unset,
    /// and flags follow JS truthiness, as a hand-written request expects.
    fn read(o: &JsValue) -> Self {
        let count = |k| number(o, k).map(|n| n as usize);
        Self {
            chunk: count(field::CHUNK),
            overlap: count(field::OVERLAP),
            max_elems: count(field::MAX_ELEMS),
            step_limit: number(o, field::STEP_LIMIT),
            bench_rounds: number(o, field::BENCH_ROUNDS),
            bench_quick: get(o, field::BENCH_QUICK).is_truthy(),
            gpu: string(o, field::GPU),
            selftest: string(o, field::SELFTEST),
            mm: string(o, field::MM),
            model_url: string(o, field::MODEL_URL),
            bench: string(o, field::BENCH),
            attn: string(o, field::ATTN),
            weights: string(o, field::WEIGHTS),
            profile: get(o, field::PROFILE).is_truthy(),
            debug: get(o, field::DEBUG).is_truthy(),
        }
    }
}

/// The file name of the stem `name` in a session's `stems` folder: the
/// name with letters, digits, spaces and dashes only, then `.wav`.  The
/// page and the worker both use it, so they agree on the files.
pub fn stem_file(name: &str) -> String {
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{safe}.wav")
}

/// A model file for the worker to read.
#[derive(Clone, Debug)]
pub enum ModelSource {
    /// The file the user picked, read by the worker, never by the page.
    File(Blob),
    /// The bytes, for tests and `xtask e2e`: they are copied, not
    /// transferred (they may be a view of WebAssembly memory, which cannot
    /// be), so the page sends a picked file as `File` instead.
    Bytes(Uint8Array),
}

/// The mix that a separation reads.
#[derive(Clone, Debug)]
pub enum Mix {
    /// Both channels, transferred with the request.
    Arrays {
        left: Float32Array,
        right: Float32Array,
    },
    /// A mix file in the browser's private file system: `path` from its
    /// root, `frames` frames of interleaved 32-bit little-endian floats.
    Stored { path: String, frames: usize },
}

/// The audio that an MP3 or FLAC encode reads.
#[derive(Clone, Debug)]
pub enum Encode {
    /// Both channels, transferred with the request (the memory path of a
    /// save).
    /// `rate`: the sample rate in Hz; the worker takes 44.1 kHz with none.
    Arrays {
        left: Float32Array,
        right: Float32Array,
        rate: Option<u32>,
    },
    /// A 16-bit stereo WAV file of `frames` frames at `wav`, encoded into
    /// the file at `out` (the storage path of a save).  Both are paths from
    /// the root of the private file system.  `rate` is required.
    Stored {
        wav: String,
        frames: usize,
        out: String,
        rate: u32,
    },
}

/// The file that an MP3 or FLAC encode gives.
#[derive(Clone, Debug)]
pub enum Encoded {
    /// The MP3 of an [`Encode::Arrays`] (transferred).
    Bytes(Uint8Array),
    /// The FLAC of an [`Encode::Arrays`] (a reference, not a copy).
    Blob(Blob),
    /// A stored encode wrote its file at `path`.
    Stored { path: String },
    /// A stored encode failed, for the reason `text`.  The worker removed
    /// the file at `path` and the WAV file.
    Failed { path: String, text: String },
}

/// A request from the page to the worker.
#[derive(Clone, Debug)]
pub enum Request {
    Init,
    /// `None` when the message carried neither a file nor bytes.
    Model(Option<ModelSource>),
    Download,
    ForgetModel,
    Separate {
        mix: Mix,
        /// The session folder to write the stems to (the storage path), or
        /// `None` to send them to the page (the memory path).
        session: Option<String>,
    },
    Selftest {
        base: Option<String>,
    },
    Bench,
    Mp3 {
        name: Option<String>,
        source: Encode,
    },
    Flac {
        name: Option<String>,
        source: Encode,
    },
}

/// A request and the settings sent with it.
#[derive(Clone, Debug)]
pub struct ToWorker {
    pub request: Request,
    pub settings: Settings,
}

/// A message that could not be read: its `type` (empty if none) and why.
#[derive(Clone, Debug, PartialEq)]
pub struct Malformed {
    pub kind: String,
    pub text: String,
}

/// The `type` of a message, or `""`.
pub fn kind_of(msg: &JsValue) -> String {
    string(msg, field::TYPE).unwrap_or_default()
}

impl Request {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Init => kind::INIT,
            Self::Model(_) => kind::MODEL,
            Self::Download => kind::DOWNLOAD,
            Self::ForgetModel => kind::FORGET_MODEL,
            Self::Separate { .. } => kind::SEPARATE,
            Self::Selftest { .. } => kind::SELFTEST,
            Self::Bench => kind::BENCH,
            Self::Mp3 { .. } => kind::MP3,
            Self::Flac { .. } => kind::FLAC,
        }
    }

    /// The message to post and the buffers to transfer with it.  A
    /// transferred buffer is unusable on this side afterwards.
    pub fn to_js(&self, settings: &Settings) -> (Object, Array) {
        let o = typed(self.kind());
        settings.write(&o);
        let transfer = Array::new();
        match self {
            Self::Init | Self::Download | Self::ForgetModel | Self::Bench => {}
            Self::Model(source) => match source {
                Some(ModelSource::File(f)) => set(&o, field::FILE, f),
                Some(ModelSource::Bytes(b)) => set(&o, field::BYTES, b),
                None => {}
            },
            Self::Separate { mix, session } => {
                match mix {
                    Mix::Arrays { left, right } => put_stereo(&o, &transfer, left, right),
                    Mix::Stored { path, frames } => {
                        set(&o, field::MIX, path.as_str());
                        set(&o, field::FRAMES, *frames as f64);
                    }
                }
                if let Some(s) = session {
                    set(&o, field::SESSION, s.as_str());
                }
            }
            Self::Selftest { base } => {
                if let Some(b) = base {
                    set(&o, field::BASE, b.as_str());
                }
            }
            Self::Mp3 { name, source } | Self::Flac { name, source } => {
                if let Some(n) = name {
                    set(&o, field::NAME, n.as_str());
                }
                match source {
                    Encode::Arrays { left, right, rate } => {
                        put_stereo(&o, &transfer, left, right);
                        if let Some(r) = rate {
                            set(&o, field::RATE, *r);
                        }
                    }
                    Encode::Stored {
                        wav,
                        frames,
                        out,
                        rate,
                    } => {
                        set(&o, field::WAV, wav.as_str());
                        set(&o, field::FRAMES, *frames as f64);
                        set(&o, field::OUT, out.as_str());
                        set(&o, field::RATE, *rate);
                    }
                }
            }
        }
        (o, transfer)
    }
}

impl ToWorker {
    pub fn from_js(msg: &JsValue) -> Result<Self, Malformed> {
        let kind = kind_of(msg);
        let bad = |text: String| Malformed {
            kind: kind.clone(),
            text,
        };
        let request = match kind.as_str() {
            kind::INIT => Request::Init,
            kind::MODEL => {
                let file = get(msg, field::FILE);
                Request::Model(if file.is_undefined() {
                    get(msg, field::BYTES)
                        .dyn_into()
                        .ok()
                        .map(ModelSource::Bytes)
                } else {
                    file.dyn_into().ok().map(ModelSource::File)
                })
            }
            kind::DOWNLOAD => Request::Download,
            kind::FORGET_MODEL => Request::ForgetModel,
            kind::SEPARATE => Request::Separate {
                mix: mix(msg).map_err(bad)?,
                session: string(msg, field::SESSION),
            },
            kind::SELFTEST => Request::Selftest {
                base: string(msg, field::BASE),
            },
            kind::BENCH => Request::Bench,
            kind::MP3 => Request::Mp3 {
                name: string(msg, field::NAME),
                source: encode_source(msg).map_err(bad)?,
            },
            kind::FLAC => Request::Flac {
                name: string(msg, field::NAME),
                source: encode_source(msg).map_err(bad)?,
            },
            other => return Err(bad(format!("unknown message {other}"))),
        };
        Ok(Self {
            request,
            settings: Settings::read(msg),
        })
    }
}

/// One stem's samples in a [`Reply::Segment`].
#[derive(Clone, Debug)]
pub struct SegmentStem {
    pub name: String,
    pub left: Float32Array,
    pub right: Float32Array,
}

/// A reply from the worker to the page.
#[derive(Clone, Debug)]
pub enum Reply {
    Alive,
    Model {
        ok: bool,
        loaded: bool,
        stored: bool,
        text: String,
    },
    Download {
        done: f64,
        total: f64,
    },
    Progress {
        done: u32,
        total: u32,
        secs: f64,
    },
    /// Every stem's samples `start..start + n` of a song of `len`.
    Segment {
        start: usize,
        len: usize,
        stems: Vec<SegmentStem>,
    },
    /// Each stem's file holds `frames` of the song's `len` frames;
    /// `stems` are the stem names in the worker's order.
    Stored {
        frames: usize,
        len: usize,
        stems: Vec<String>,
    },
    /// Browser storage failed: it could not hold the stems, or could not
    /// give the stored mix.  The separation stopped.
    Unstored {
        text: String,
    },
    Done {
        secs: f64,
    },
    /// The request of type `of` failed.
    Error {
        of: String,
        text: String,
    },
    Status(String),
    Log(String),
    Selftest {
        ok: bool,
        text: String,
    },
    Bench(String),
    Mp3 {
        name: String,
        file: Encoded,
    },
    Flac {
        name: String,
        file: Encoded,
    },
}

impl Reply {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Alive => kind::ALIVE,
            Self::Model { .. } => kind::MODEL,
            Self::Download { .. } => kind::DOWNLOAD,
            Self::Progress { .. } => kind::PROGRESS,
            Self::Segment { .. } => kind::SEGMENT,
            Self::Stored { .. } => kind::STORED,
            Self::Unstored { .. } => kind::UNSTORED,
            Self::Done { .. } => kind::DONE,
            Self::Error { .. } => kind::ERROR,
            Self::Status(_) => kind::STATUS,
            Self::Log(_) => kind::LOG,
            Self::Selftest { .. } => kind::SELFTEST,
            Self::Bench(_) => kind::BENCH,
            Self::Mp3 { .. } => kind::MP3,
            Self::Flac { .. } => kind::FLAC,
        }
    }

    /// The message to post and the buffers to transfer with it.
    pub fn to_js(&self) -> (Object, Array) {
        let o = typed(self.kind());
        let transfer = Array::new();
        match self {
            Self::Alive => {}
            Self::Model {
                ok,
                loaded,
                stored,
                text,
            } => {
                set(&o, field::OK, *ok);
                set(&o, field::LOADED, *loaded);
                set(&o, field::STORED, *stored);
                set(&o, field::TEXT, text.as_str());
            }
            Self::Download { done, total } => {
                set(&o, field::DONE, *done);
                set(&o, field::TOTAL, *total);
            }
            Self::Progress { done, total, secs } => {
                set(&o, field::DONE, *done);
                set(&o, field::TOTAL, *total);
                set(&o, field::SECS, *secs);
            }
            Self::Segment { start, len, stems } => {
                set(&o, field::START, *start as f64);
                set(&o, field::LEN, *len as f64);
                let list = Array::new();
                for s in stems {
                    transfer.push(&s.left.buffer());
                    transfer.push(&s.right.buffer());
                    list.push(&Array::of3(&s.name.as_str().into(), &s.left, &s.right));
                }
                set(&o, field::STEMS, list);
            }
            Self::Stored { frames, len, stems } => {
                set(&o, field::FRAMES, *frames as f64);
                set(&o, field::LEN, *len as f64);
                let names: Array = stems.iter().map(|n| JsValue::from_str(n)).collect();
                set(&o, field::STEMS, names);
            }
            Self::Unstored { text } => set(&o, field::TEXT, text.as_str()),
            Self::Done { secs } => set(&o, field::SECS, *secs),
            Self::Error { of, text } => {
                set(&o, field::OF, of.as_str());
                set(&o, field::TEXT, text.as_str());
            }
            Self::Status(text) | Self::Log(text) | Self::Bench(text) => {
                set(&o, field::TEXT, text.as_str())
            }
            Self::Selftest { ok, text } => {
                set(&o, field::OK, *ok);
                set(&o, field::TEXT, text.as_str());
            }
            Self::Mp3 { name, file } | Self::Flac { name, file } => {
                set(&o, field::NAME, name.as_str());
                match file {
                    Encoded::Bytes(bytes) => {
                        set(&o, field::BYTES, bytes);
                        transfer.push(&bytes.buffer());
                    }
                    Encoded::Blob(blob) => set(&o, field::FILE, blob),
                    Encoded::Stored { path } => set(&o, field::PATH, path.as_str()),
                    Encoded::Failed { path, text } => {
                        set(&o, field::PATH, path.as_str());
                        set(&o, field::FAILED, text.as_str());
                    }
                }
            }
        }
        (o, transfer)
    }

    pub fn from_js(msg: &JsValue) -> Result<Self, Malformed> {
        let kind = kind_of(msg);
        let bad = |text: &str| Malformed {
            kind: kind.clone(),
            text: format!("{kind} message: {text}"),
        };
        let text = || string(msg, field::TEXT).unwrap_or_default();
        let flag = |k| get(msg, k).is_truthy();
        let num = |k| number(msg, k).unwrap_or(0.0);
        Ok(match kind.as_str() {
            kind::ALIVE => Self::Alive,
            kind::MODEL => Self::Model {
                ok: flag(field::OK),
                loaded: flag(field::LOADED),
                stored: flag(field::STORED),
                text: text(),
            },
            kind::DOWNLOAD => Self::Download {
                done: num(field::DONE),
                total: num(field::TOTAL),
            },
            kind::PROGRESS => Self::Progress {
                done: num(field::DONE) as u32,
                total: num(field::TOTAL) as u32,
                secs: num(field::SECS),
            },
            kind::SEGMENT => {
                let start = number(msg, field::START).ok_or_else(|| bad("no start"))?;
                let len = number(msg, field::LEN).ok_or_else(|| bad("no len"))?;
                let list: Array = get(msg, field::STEMS)
                    .dyn_into()
                    .map_err(|_| bad("no stems"))?;
                let mut stems = Vec::with_capacity(list.length() as usize);
                for entry in list.iter() {
                    let entry: Array = entry.dyn_into().map_err(|_| bad("malformed stem"))?;
                    let channel = |i| {
                        entry
                            .get(i)
                            .dyn_into::<Float32Array>()
                            .map_err(|_| bad("malformed stem"))
                    };
                    stems.push(SegmentStem {
                        name: entry.get(0).as_string().unwrap_or_default(),
                        left: channel(1)?,
                        right: channel(2)?,
                    });
                }
                Self::Segment {
                    start: start as usize,
                    len: len as usize,
                    stems,
                }
            }
            kind::STORED => {
                let frames = number(msg, field::FRAMES).ok_or_else(|| bad("no frames"))?;
                let len = number(msg, field::LEN).ok_or_else(|| bad("no len"))?;
                let list: Array = get(msg, field::STEMS)
                    .dyn_into()
                    .map_err(|_| bad("no stems"))?;
                let stems = list
                    .iter()
                    .map(|n| n.as_string().ok_or_else(|| bad("a stem name is not text")))
                    .collect::<Result<_, _>>()?;
                Self::Stored {
                    frames: frames as usize,
                    len: len as usize,
                    stems,
                }
            }
            kind::UNSTORED => Self::Unstored { text: text() },
            kind::DONE => Self::Done {
                secs: num(field::SECS),
            },
            kind::ERROR => Self::Error {
                of: string(msg, field::OF).unwrap_or_default(),
                text: text(),
            },
            kind::STATUS => Self::Status(text()),
            kind::LOG => Self::Log(text()),
            kind::SELFTEST => Self::Selftest {
                ok: flag(field::OK),
                text: text(),
            },
            kind::BENCH => Self::Bench(text()),
            kind::MP3 => Self::Mp3 {
                name: string(msg, field::NAME).unwrap_or_default(),
                file: encoded(msg, &kind).map_err(bad)?,
            },
            kind::FLAC => Self::Flac {
                name: string(msg, field::NAME).unwrap_or_default(),
                file: encoded(msg, &kind).map_err(bad)?,
            },
            other => return Err(bad(&format!("unknown type {other:?}"))),
        })
    }
}

fn typed(kind: &str) -> Object {
    let o = Object::new();
    set(&o, field::TYPE, kind);
    o
}

fn set(o: &Object, k: &str, v: impl Into<JsValue>) {
    let _ = Reflect::set(o, &JsValue::from_str(k), &v.into());
}

fn get(o: &JsValue, k: &str) -> JsValue {
    Reflect::get(o, &JsValue::from_str(k)).unwrap_or(JsValue::UNDEFINED)
}

fn string(o: &JsValue, k: &str) -> Option<String> {
    get(o, k).as_string()
}

fn number(o: &JsValue, k: &str) -> Option<f64> {
    get(o, k).as_f64()
}

fn put_stereo(o: &Object, transfer: &Array, left: &Float32Array, right: &Float32Array) {
    set(o, field::LEFT, left);
    set(o, field::RIGHT, right);
    transfer.push(&left.buffer());
    transfer.push(&right.buffer());
}

/// The mix of a `separate` request: a stored mix if it has `mix`, else
/// the two arrays.
fn mix(msg: &JsValue) -> Result<Mix, String> {
    let path = get(msg, field::MIX);
    if path.is_undefined() {
        let (left, right) = stereo(msg)?;
        return Ok(Mix::Arrays { left, right });
    }
    if !get(msg, field::LEFT).is_undefined() || !get(msg, field::RIGHT).is_undefined() {
        return Err("a separation names a stored mix or two arrays, not both".into());
    }
    let path = path.as_string().ok_or("mix must be a path")?;
    Ok(Mix::Stored {
        path,
        frames: whole_frames(msg)?,
    })
}

/// The `frames` of a message, a whole number from 0 to `usize::MAX`.
fn whole_frames(msg: &JsValue) -> Result<usize, String> {
    // Over `usize::MAX`, `as usize` would saturate: on wasm32, 2^32 - 1.
    // `usize::MAX as f64` rounds up to 2^64 on a 64-bit target, so the
    // bound is the first power of two over `usize::MAX`, which is exact.
    let limit = 2f64.powi(usize::BITS as i32);
    let frames = number(msg, field::FRAMES)
        .filter(|f| f.is_finite() && *f >= 0.0 && f.fract() == 0.0 && *f < limit)
        .ok_or_else(|| {
            format!(
                "frames must be a whole number of frames, at most {}",
                usize::MAX
            )
        })?;
    Ok(frames as usize)
}

/// The source of an `mp3` or `flac` request: a stored WAV if it has `wav`,
/// else the two arrays.  A missing field is refused, not read as a
/// default (TD-16).
fn encode_source(msg: &JsValue) -> Result<Encode, String> {
    let has = |k| !get(msg, k).is_undefined();
    if !has(field::WAV) {
        if has(field::OUT) || has(field::FRAMES) {
            return Err("an encode with out or frames names its wav".into());
        }
        let (left, right) = stereo(msg)?;
        let rate = if has(field::RATE) {
            Some(whole_rate(msg)?)
        } else {
            None
        };
        return Ok(Encode::Arrays { left, right, rate });
    }
    if has(field::LEFT) || has(field::RIGHT) {
        return Err("an encode names a stored WAV or two arrays, not both".into());
    }
    let wav = string(msg, field::WAV).ok_or("wav must be a path")?;
    let frames = whole_frames(msg)?;
    if !has(field::OUT) {
        return Err("a stored encode names its out file".into());
    }
    let out = string(msg, field::OUT).ok_or("out must be a path")?;
    if !has(field::RATE) {
        return Err("a stored encode names its rate".into());
    }
    let rate = whole_rate(msg)?;
    Ok(Encode::Stored {
        wav,
        frames,
        out,
        rate,
    })
}

/// The `rate` of a message: a whole number of hertz from 1 to `u32::MAX`
/// (nit 2-8 of review round 2 of M8).
fn whole_rate(msg: &JsValue) -> Result<u32, String> {
    number(msg, field::RATE)
        .filter(|r| r.is_finite() && *r >= 1.0 && r.fract() == 0.0 && *r <= f64::from(u32::MAX))
        .map(|r| r as u32)
        .ok_or_else(|| {
            format!(
                "rate must be a whole number of hertz from 1 to {}",
                u32::MAX
            )
        })
}

/// The file of an `mp3` or `flac` reply: a stored file if it has `path`,
/// else the bytes of an MP3 or the `Blob` of a FLAC.
fn encoded(msg: &JsValue, kind: &str) -> Result<Encoded, &'static str> {
    let path = get(msg, field::PATH);
    if !path.is_undefined() {
        let path = path.as_string().ok_or("path must be a path")?;
        let failed = get(msg, field::FAILED);
        if failed.is_undefined() {
            return Ok(Encoded::Stored { path });
        }
        let text = failed.as_string().ok_or("failed must be text")?;
        return Ok(Encoded::Failed { path, text });
    }
    if kind == kind::MP3 {
        return get(msg, field::BYTES)
            .dyn_into()
            .map(Encoded::Bytes)
            .map_err(|_| "no bytes");
    }
    get(msg, field::FILE)
        .dyn_into()
        .map(Encoded::Blob)
        .map_err(|_| "no file")
}

fn stereo(msg: &JsValue) -> Result<(Float32Array, Float32Array), String> {
    let channel = |k: &str| {
        get(msg, k)
            .dyn_into::<Float32Array>()
            .map_err(|_| format!("{k} must be a Float32Array"))
    };
    Ok((channel(field::LEFT)?, channel(field::RIGHT)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each reply's `type`.  `mp3` and `flac` carry JS values, which a
    /// native test cannot make; the browser tests cover them.
    #[test]
    fn each_reply_names_its_type() {
        let text = || "t".to_string();
        let kinds: Vec<&str> = [
            Reply::Alive,
            Reply::Model {
                ok: true,
                loaded: true,
                stored: true,
                text: text(),
            },
            Reply::Download {
                done: 1.0,
                total: 2.0,
            },
            Reply::Progress {
                done: 1,
                total: 2,
                secs: 0.5,
            },
            Reply::Segment {
                start: 0,
                len: 4,
                stems: Vec::new(),
            },
            Reply::Stored {
                frames: 4,
                len: 4,
                stems: Vec::new(),
            },
            Reply::Unstored { text: text() },
            Reply::Done { secs: 1.0 },
            Reply::Error {
                of: kind::SEPARATE.into(),
                text: text(),
            },
            Reply::Status(text()),
            Reply::Log(text()),
            Reply::Selftest {
                ok: true,
                text: text(),
            },
            Reply::Bench(text()),
        ]
        .iter()
        .map(Reply::kind)
        .collect();
        assert_eq!(
            kinds,
            [
                "alive", "model", "download", "progress", "segment", "stored", "unstored", "done",
                "error", "status", "log", "selftest", "bench"
            ]
        );
    }

    #[test]
    fn a_stem_file_name_keeps_only_safe_characters() {
        assert_eq!(stem_file("bass"), "bass.wav");
        assert_eq!(stem_file("classic drums"), "classic drums.wav");
        assert_eq!(stem_file("lead-vocals"), "lead-vocals.wav");
        assert_eq!(stem_file("a/b:c.d"), "a_b_c_d.wav");
        assert_eq!(stem_file("../acme"), "___acme.wav");
    }

    #[test]
    fn the_query_sets_numbers_strings_and_flags() {
        let s = Settings::from_query(
            "?chunk=16384&overlap=2&mm=s64&model_url=/x.safetensors&profile=1&bench_quick=0&selftest",
        );
        assert_eq!(s.chunk, Some(16384));
        assert_eq!(s.overlap, Some(2));
        assert_eq!(s.mm.as_deref(), Some("s64"));
        assert_eq!(s.model_url.as_deref(), Some("/x.safetensors"));
        assert!(s.profile);
        assert!(!s.bench_quick, "zero is off");
        assert_eq!(s.selftest, None, "a bare name has no value");
    }

    #[test]
    fn the_query_sets_the_weight_format() {
        let s = Settings::from_query("?bench&bench_quick=1&mm=s64&weights=f16");
        assert_eq!(s.weights.as_deref(), Some("f16"));
        assert_eq!(s.mm.as_deref(), Some("s64"));
        assert_eq!(Settings::from_query("chunk=4").weights, None);
    }

    #[test]
    fn a_number_that_does_not_parse_and_unknown_names_are_ignored() {
        let s = Settings::from_query("chunk=lots&colour=blue&step_limit=2.5&profile=");
        assert_eq!(s.chunk, None);
        assert_eq!(s.step_limit, Some(2.5));
        assert!(!s.profile, "empty is off");
        assert!(!Settings::from_query("profile=0").profile, "zero is off");
        assert!(Settings::from_query("profile=yes").profile);
        assert!(
            Settings::from_query("bench_quick=yes").bench_quick,
            "as profile"
        );
        assert_eq!(
            Settings {
                step_limit: Some(2.5),
                ..Settings::default()
            },
            s
        );
    }
}
