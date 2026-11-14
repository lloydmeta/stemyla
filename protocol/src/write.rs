//! The messages between the page and its write worker (M5.1 of
//! `docs/plans/active/gpu-speed.md`).  The write worker writes the page's
//! song-sized files through sync access handles, since Safari keeps a swap
//! file of each write stream that closes (TD-103).  Each message is a
//! plain JS object whose `type` starts with `write-`, so a message for one
//! worker is never read as a message for another.  Each typed array goes
//! in the transfer list (MEM-2).
//!
//! The requests ([`WriteRequest`], page to write worker).  `within` is a
//! folder from the root of the private file system, which must exist when
//! it is not empty, so a forgotten session does not come back.  `rel` is a
//! path from `within`, whose folders the worker makes.
//! * `{type: "write-file", within, rel, body}`: write the whole file from
//!   `body`, a `Blob` or a `Uint8Array` (transferred).
//! * `{type: "write-block", within, rel, at, bytes}`: write `bytes`, a
//!   `Uint8Array` (transferred), at byte `at` of the file.  With `at` 0,
//!   the worker first cuts the file to 0 bytes.
//! * `{type: "write-end", within, rel, size}`: the blocks of the file are
//!   all sent.  `size` is the bytes that the file must hold: the worker
//!   replies `failed` if it holds other bytes, as when a block is lost.
//!   `size: null` ends an abandoned write: the worker removes the file.
//!
//! The replies ([`WriteReply`], write worker to page):
//! * `{type: "write-ready"}` once the worker listens.
//! * `{type: "write-done", result, error?, bytes?}`: the request that is
//!   the oldest one not yet ended is done.  `result` is `"written"`,
//!   `"gone"` (the folder `within` is gone) or `"failed"`, with the name of
//!   the error in `error`.  A `write-block` reply gives its `bytes` back
//!   (transferred), so the page can use the array again.  A `write-end`
//!   has this reply too: each request has one kind of reply.
//!
//! The worker ends each request with exactly one `write-done`, in the
//! order sent.  A missing or wrong field is an error, never a default
//! (TD-16).

use js_sys::{Array, Object, Uint8Array};
use wasm_bindgen::{JsCast, JsValue};
use web_sys::Blob;

use crate::decode::{MAX_SAFE, text_field, whole};
use crate::{Malformed, get, kind_of, set, typed};

/// The `type` of each message.
pub mod kind {
    pub const FILE: &str = "write-file";
    pub const BLOCK: &str = "write-block";
    pub const END: &str = "write-end";
    pub const READY: &str = "write-ready";
    pub const DONE: &str = "write-done";
}

/// Field names, besides `type`.
mod field {
    pub const WITHIN: &str = "within";
    pub const REL: &str = "rel";
    pub const BODY: &str = "body";
    pub const AT: &str = "at";
    pub const SIZE: &str = "size";
    pub const BYTES: &str = "bytes";
    pub const RESULT: &str = "result";
    pub const ERROR: &str = "error";
}

/// The values of `result`.
mod result {
    pub const WRITTEN: &str = "written";
    pub const GONE: &str = "gone";
    pub const FAILED: &str = "failed";
}

/// The bytes of a whole file.
#[derive(Clone, Debug)]
pub enum Body {
    /// A `Blob`, such as a dropped file: the worker reads it in slices.
    Blob(Blob),
    /// A typed array, transferred.
    Bytes(Uint8Array),
}

/// A request to the write worker.
#[derive(Clone, Debug)]
pub enum WriteRequest {
    /// The whole file `within/rel`.
    File {
        within: String,
        rel: String,
        body: Body,
    },
    /// `bytes` at byte `at` of `within/rel`; at 0 the file is cut first.
    Block {
        within: String,
        rel: String,
        at: u64,
        bytes: Uint8Array,
    },
    /// The end of a block-written file `within/rel`: `Some` of the bytes
    /// that it must hold, or `None` for an abandoned write, whose file the
    /// worker removes.
    End {
        within: String,
        rel: String,
        size: Option<u64>,
    },
}

/// How a request ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Written,
    /// The folder `within` is gone; nothing was made.
    Gone,
    /// The name of the error, such as `QuotaExceededError`.
    Failed(String),
}

/// A reply of the write worker.
#[derive(Clone, Debug)]
pub enum WriteReply {
    Ready,
    Done {
        outcome: Outcome,
        /// The array of a `write-block` request, given back.
        bytes: Option<Uint8Array>,
    },
}

impl WriteRequest {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::File { .. } => kind::FILE,
            Self::Block { .. } => kind::BLOCK,
            Self::End { .. } => kind::END,
        }
    }

    /// The message to post, and its transfer list: the buffer of each
    /// typed array.
    pub fn to_js(&self) -> (Object, Array) {
        let o = typed(self.kind());
        let transfer = Array::new();
        match self {
            Self::File { within, rel, body } => {
                set(&o, field::WITHIN, within.as_str());
                set(&o, field::REL, rel.as_str());
                match body {
                    Body::Blob(blob) => set(&o, field::BODY, blob),
                    Body::Bytes(bytes) => {
                        set(&o, field::BODY, bytes);
                        transfer.push(&bytes.buffer());
                    }
                }
            }
            Self::Block {
                within,
                rel,
                at,
                bytes,
            } => {
                set(&o, field::WITHIN, within.as_str());
                set(&o, field::REL, rel.as_str());
                set(&o, field::AT, *at as f64);
                set(&o, field::BYTES, bytes);
                transfer.push(&bytes.buffer());
            }
            Self::End { within, rel, size } => {
                set(&o, field::WITHIN, within.as_str());
                set(&o, field::REL, rel.as_str());
                let size = size.map_or(JsValue::NULL, |n| JsValue::from(n as f64));
                set(&o, field::SIZE, size);
            }
        }
        (o, transfer)
    }

    pub fn from_js(msg: &JsValue) -> Result<Self, Malformed> {
        let kind = kind_of(msg);
        let bad = |text: String| Malformed {
            kind: kind.clone(),
            text: format!("{kind} message: {text}"),
        };
        let within = || text_field(msg, field::WITHIN).map_err(bad);
        let rel = || text_field(msg, field::REL).map_err(bad);
        match kind.as_str() {
            kind::FILE => {
                let body = get(msg, field::BODY);
                let body = if body.is_instance_of::<Blob>() {
                    Body::Blob(body.unchecked_into())
                } else if body.is_instance_of::<Uint8Array>() {
                    Body::Bytes(body.unchecked_into())
                } else {
                    return Err(bad(format!(
                        "{} must be a Blob or a Uint8Array",
                        field::BODY
                    )));
                };
                Ok(Self::File {
                    within: within()?,
                    rel: rel()?,
                    body,
                })
            }
            kind::BLOCK => Ok(Self::Block {
                within: within()?,
                rel: rel()?,
                at: whole(msg, field::AT, MAX_SAFE).map_err(bad)? as u64,
                bytes: bytes_field(msg, field::BYTES).map_err(bad)?,
            }),
            kind::END => {
                // `null` ends an abandoned write; a missing size is an
                // error, never a default (TD-16).
                let size = if get(msg, field::SIZE).is_null() {
                    None
                } else {
                    Some(whole(msg, field::SIZE, MAX_SAFE).map_err(bad)? as u64)
                };
                Ok(Self::End {
                    within: within()?,
                    rel: rel()?,
                    size,
                })
            }
            other => Err(bad(format!("not a request of the write worker: {other:?}"))),
        }
    }
}

impl WriteReply {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Ready => kind::READY,
            Self::Done { .. } => kind::DONE,
        }
    }

    /// The message to post, and its transfer list: the buffer of the
    /// array given back, if any.
    pub fn to_js(&self) -> (Object, Array) {
        let o = typed(self.kind());
        let transfer = Array::new();
        if let Self::Done { outcome, bytes } = self {
            let value = match outcome {
                Outcome::Written => result::WRITTEN,
                Outcome::Gone => result::GONE,
                Outcome::Failed(error) => {
                    set(&o, field::ERROR, error.as_str());
                    result::FAILED
                }
            };
            set(&o, field::RESULT, value);
            if let Some(bytes) = bytes {
                set(&o, field::BYTES, bytes);
                transfer.push(&bytes.buffer());
            }
        }
        (o, transfer)
    }

    pub fn from_js(msg: &JsValue) -> Result<Self, Malformed> {
        let kind = kind_of(msg);
        let bad = |text: String| Malformed {
            kind: kind.clone(),
            text: format!("{kind} message: {text}"),
        };
        match kind.as_str() {
            kind::READY => Ok(Self::Ready),
            kind::DONE => {
                let outcome = match text_field(msg, field::RESULT).map_err(bad)?.as_str() {
                    result::WRITTEN => Outcome::Written,
                    result::GONE => Outcome::Gone,
                    result::FAILED => Outcome::Failed(text_field(msg, field::ERROR).map_err(bad)?),
                    other => {
                        return Err(bad(format!(
                            "{} must be written, gone or failed, not {other:?}",
                            field::RESULT
                        )));
                    }
                };
                let bytes = if get(msg, field::BYTES).is_undefined() {
                    None
                } else {
                    Some(bytes_field(msg, field::BYTES).map_err(bad)?)
                };
                Ok(Self::Done { outcome, bytes })
            }
            other => Err(bad(format!("not a reply of the write worker: {other:?}"))),
        }
    }
}

/// The `Uint8Array` in field `k`.
fn bytes_field(o: &JsValue, k: &str) -> Result<Uint8Array, String> {
    get(o, k)
        .dyn_into::<Uint8Array>()
        .map_err(|_| format!("{k} must be a Uint8Array"))
}
