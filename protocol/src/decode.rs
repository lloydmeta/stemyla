//! The messages between the page and its decode worker (decision 0019,
//! `docs/plans/completed/stream-from-storage-m5.md`, D6).  Each is a plain JS
//! object whose `type` names it.  The types differ from those of the stem
//! worker's messages, so a message for one worker is never read as a
//! message for the other.  No samples cross, so nothing is transferred.
//!
//! The request ([`DecodeRequest`], page to decode worker):
//! * `{type: "decode", session, sources: [{file, index, ext, in_mix}, ...],
//!   mix_frames?}`: decode the stored sources of the session folder
//!   `session` (`"sessions/<id>"`) into its session files.  `file` is the
//!   path of a source from the session folder (`"src/<i>.<ext>"`), `index`
//!   its `<i>` and `ext` its extension.  `in_mix` is false for a derived
//!   source, which stays out of the mix.  The sources come in the order of
//!   `Song::from_audio`.  `mix_frames`, if there, is the frames of a usable
//!   `mix.f32` of the session, which the worker does not write again.
//!
//! The other requests ([`WorkerRequest`], D7 and D8 of
//! `docs/plans/completed/stream-from-storage-m7.md`).  A lane is `{path,
//! format, data_offset, data_frames}`: its path from the session folder,
//! `"f32"` or `"pcm16"`, the bytes before its first frame and the frames
//! it holds (later frames are silence).  Every other path is a part path
//! from the session folder, which the worker writes.
//! * `{type: "overviews", session, frames, lanes: [{...lane, overview},
//!   ...], mix?}`: write the overview of each lane, of `frames` frames, to
//!   its `overview`, and, with `mix`, the overview of the lanes' clamped
//!   sum at volume 1 to `mix`.
//! * `{type: "classic", session, frames, lanes: [lane, ...], wav,
//!   overview}`: write the percussive part of the lanes' clamped sum at
//!   volume 1 (a block HPSS) as a 16-bit stereo WAV at 44.1 kHz to `wav`,
//!   and its overview to `overview`.
//!
//! The replies ([`DecodeReply`], decode worker to page):
//! * `{type: "decode-alive"}` once the worker listens.
//! * `{type: "decoded", frames, new_mix, channels: [1 or 2, ...]}`: the
//!   session's `.part` files are complete.  `frames` is the song's frames,
//!   `new_mix` whether the worker wrote a new `mix.f32.decode.part`, and
//!   `channels` the channels of each source, in the request's order.
//! * `{type: "undecodable", text}`: a source cannot be decoded.
//! * `{type: "decode-unstored", text}`: browser storage failed.
//! * `{type: "decode-log", text}`: a line for the page's log.
//! * `{type: "overviews-made"}`: the overviews are in their part files.
//! * `{type: "classic-made"}`: the WAV and its overview are in their part
//!   files.
//!
//! A `decode` request ends with exactly one of `decoded`, `undecodable`
//! and `decode-unstored`.  An `overviews` request ends with exactly one
//! of `overviews-made` and `decode-unstored`, and a `classic` request
//! with exactly one of `classic-made` and `decode-unstored`.  A missing
//! or wrong field is an error, never a default (TD-16).

use js_sys::{Array, Object};
use wasm_bindgen::JsValue;

use crate::{Malformed, get, kind_of, set, typed};

/// The `type` of each message.
pub mod kind {
    pub const DECODE: &str = "decode";
    pub const ALIVE: &str = "decode-alive";
    pub const DECODED: &str = "decoded";
    pub const UNDECODABLE: &str = "undecodable";
    pub const UNSTORED: &str = "decode-unstored";
    pub const LOG: &str = "decode-log";
    pub const OVERVIEWS: &str = "overviews";
    pub const OVERVIEWS_MADE: &str = "overviews-made";
    pub const CLASSIC: &str = "classic";
    pub const CLASSIC_MADE: &str = "classic-made";
}

/// Field names, besides `type`.
mod field {
    pub const SESSION: &str = "session";
    pub const SOURCES: &str = "sources";
    pub const FILE: &str = "file";
    pub const INDEX: &str = "index";
    pub const EXT: &str = "ext";
    pub const IN_MIX: &str = "in_mix";
    pub const MIX_FRAMES: &str = "mix_frames";
    pub const FRAMES: &str = "frames";
    pub const NEW_MIX: &str = "new_mix";
    pub const CHANNELS: &str = "channels";
    pub const TEXT: &str = "text";
    pub const LANES: &str = "lanes";
    pub const PATH: &str = "path";
    pub const FORMAT: &str = "format";
    pub const DATA_OFFSET: &str = "data_offset";
    pub const DATA_FRAMES: &str = "data_frames";
    pub const OVERVIEW: &str = "overview";
    pub const MIX: &str = "mix";
    pub const WAV: &str = "wav";
}

/// One stored source of a song.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    /// The path from the session folder, such as `"src/0.wav"`.
    pub file: String,
    /// The `<i>` of `src/<i>.<ext>`, which names its session files.
    pub index: u32,
    /// The extension, such as `"wav"`, for the format probe.
    pub ext: String,
    /// False for a derived source, which stays out of the mix.
    pub in_mix: bool,
}

/// A request to decode a session's stored sources.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodeRequest {
    /// The session folder from the root of the private file system.
    pub session: String,
    /// The sources, in the order of `Song::from_audio`.
    pub sources: Vec<Source>,
    /// The frames of a usable `mix.f32`, which the worker keeps.
    pub mix_frames: Option<u64>,
}

/// How a lane file holds its frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaneEncoding {
    /// Two 32-bit floats a frame (`"f32"`): a decoded source or `mix.f32`.
    F32,
    /// Two 16-bit integers a frame (`"pcm16"`): a stem WAV.
    Pcm16,
}

/// A lane file that the worker reads (D7 of the M7 plan).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaneRef {
    /// The path from the session folder, such as `"stems/bass.wav"`.
    pub path: String,
    pub encoding: LaneEncoding,
    /// The bytes before the first frame.
    pub data_offset: u64,
    /// The frames that the file holds; later frames are silence.
    pub data_frames: u64,
}

/// A lane whose overview the worker makes, and the part path of that
/// overview from the session folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OverviewLane {
    pub lane: LaneRef,
    pub overview: String,
}

/// A request for the overviews of lanes (D7 of the M7 plan).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OverviewsRequest {
    /// The session folder from the root of the private file system.
    pub session: String,
    /// The song's frames: the frames of each overview.
    pub frames: u64,
    pub lanes: Vec<OverviewLane>,
    /// The part path of the overview of the lanes' clamped sum at volume
    /// 1, if one is asked for.
    pub mix: Option<String>,
}

/// A request for classic drums of lanes (D8 of the M7 plan).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassicRequest {
    pub session: String,
    pub frames: u64,
    /// The lanes whose clamped sum at volume 1 is the input.
    pub lanes: Vec<LaneRef>,
    /// The part paths of the 16-bit WAV and of its overview.
    pub wav: String,
    pub overview: String,
}

/// A request to the decode worker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkerRequest {
    Decode(DecodeRequest),
    Overviews(OverviewsRequest),
    Classic(ClassicRequest),
}

impl LaneEncoding {
    fn name(self) -> &'static str {
        match self {
            Self::F32 => "f32",
            Self::Pcm16 => "pcm16",
        }
    }
}

impl LaneRef {
    /// Sets the fields of the lane on `o`.
    fn put(&self, o: &Object) {
        set(o, field::PATH, self.path.as_str());
        set(o, field::FORMAT, self.encoding.name());
        set(o, field::DATA_OFFSET, self.data_offset as f64);
        set(o, field::DATA_FRAMES, self.data_frames as f64);
    }

    fn from_entry(entry: &JsValue) -> Result<Self, String> {
        if !entry.is_object() {
            return Err(format!("a lane must be an object, not {entry:?}"));
        }
        let encoding = match get(entry, field::FORMAT).as_string().as_deref() {
            Some("f32") => LaneEncoding::F32,
            Some("pcm16") => LaneEncoding::Pcm16,
            _ => return Err(format!("{} must be \"f32\" or \"pcm16\"", field::FORMAT)),
        };
        Ok(Self {
            path: text_field(entry, field::PATH)?,
            encoding,
            data_offset: whole(entry, field::DATA_OFFSET, MAX_SAFE)? as u64,
            data_frames: whole(entry, field::DATA_FRAMES, MAX_SAFE)? as u64,
        })
    }
}

/// The entries of the array field `k`.
fn list_field(o: &JsValue, k: &str) -> Result<Vec<JsValue>, String> {
    let list = get(o, k);
    if !Array::is_array(&list) {
        return Err(format!("{k} must be an array"));
    }
    Ok(Array::from(&list).iter().collect())
}

impl WorkerRequest {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Decode(_) => kind::DECODE,
            Self::Overviews(_) => kind::OVERVIEWS,
            Self::Classic(_) => kind::CLASSIC,
        }
    }

    /// The message to post, and its transfer list, which is empty.
    pub fn to_js(&self) -> (Object, Array) {
        let req = match self {
            Self::Decode(req) => return req.to_js(),
            Self::Overviews(req) => {
                let o = typed(kind::OVERVIEWS);
                set(&o, field::SESSION, req.session.as_str());
                set(&o, field::FRAMES, req.frames as f64);
                let lanes: Array = req
                    .lanes
                    .iter()
                    .map(|l| {
                        let entry = Object::new();
                        l.lane.put(&entry);
                        set(&entry, field::OVERVIEW, l.overview.as_str());
                        JsValue::from(entry)
                    })
                    .collect();
                set(&o, field::LANES, lanes);
                if let Some(mix) = &req.mix {
                    set(&o, field::MIX, mix.as_str());
                }
                o
            }
            Self::Classic(req) => {
                let o = typed(kind::CLASSIC);
                set(&o, field::SESSION, req.session.as_str());
                set(&o, field::FRAMES, req.frames as f64);
                let lanes: Array = req
                    .lanes
                    .iter()
                    .map(|lane| {
                        let entry = Object::new();
                        lane.put(&entry);
                        JsValue::from(entry)
                    })
                    .collect();
                set(&o, field::LANES, lanes);
                set(&o, field::WAV, req.wav.as_str());
                set(&o, field::OVERVIEW, req.overview.as_str());
                o
            }
        };
        (req, Array::new())
    }

    pub fn from_js(msg: &JsValue) -> Result<Self, Malformed> {
        let kind = kind_of(msg);
        let bad = |text: String| Malformed {
            kind: kind.clone(),
            text: format!("{kind} message: {text}"),
        };
        match kind.as_str() {
            kind::DECODE => DecodeRequest::from_js(msg).map(Self::Decode),
            kind::OVERVIEWS => {
                let lanes = list_field(msg, field::LANES)
                    .and_then(|entries| {
                        entries
                            .iter()
                            .map(|entry| {
                                Ok(OverviewLane {
                                    lane: LaneRef::from_entry(entry)?,
                                    overview: text_field(entry, field::OVERVIEW)?,
                                })
                            })
                            .collect::<Result<_, String>>()
                    })
                    .map_err(bad)?;
                let mix = if get(msg, field::MIX).is_undefined() {
                    None
                } else {
                    Some(text_field(msg, field::MIX).map_err(bad)?)
                };
                Ok(Self::Overviews(OverviewsRequest {
                    session: text_field(msg, field::SESSION).map_err(bad)?,
                    frames: whole(msg, field::FRAMES, MAX_SAFE).map_err(bad)? as u64,
                    lanes,
                    mix,
                }))
            }
            kind::CLASSIC => {
                let lanes = list_field(msg, field::LANES)
                    .and_then(|entries| entries.iter().map(LaneRef::from_entry).collect())
                    .map_err(bad)?;
                Ok(Self::Classic(ClassicRequest {
                    session: text_field(msg, field::SESSION).map_err(bad)?,
                    frames: whole(msg, field::FRAMES, MAX_SAFE).map_err(bad)? as u64,
                    lanes,
                    wav: text_field(msg, field::WAV).map_err(bad)?,
                    overview: text_field(msg, field::OVERVIEW).map_err(bad)?,
                }))
            }
            other => Err(bad(format!(
                "not a request of the decode worker: {other:?}"
            ))),
        }
    }
}

/// A reply of the decode worker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecodeReply {
    Alive,
    Decoded {
        frames: u64,
        new_mix: bool,
        channels: Vec<u8>,
    },
    Undecodable {
        text: String,
    },
    Unstored {
        text: String,
    },
    Log(String),
    /// The overviews of an `overviews` request are in their part files.
    OverviewsMade,
    /// The WAV and the overview of a `classic` request are in their part
    /// files.
    ClassicMade,
}

impl DecodeRequest {
    /// The message to post, and its transfer list, which is empty.
    pub fn to_js(&self) -> (Object, Array) {
        let o = typed(kind::DECODE);
        set(&o, field::SESSION, self.session.as_str());
        let sources: Array = self
            .sources
            .iter()
            .map(|s| {
                let entry = Object::new();
                set(&entry, field::FILE, s.file.as_str());
                set(&entry, field::INDEX, s.index);
                set(&entry, field::EXT, s.ext.as_str());
                set(&entry, field::IN_MIX, s.in_mix);
                JsValue::from(entry)
            })
            .collect();
        set(&o, field::SOURCES, sources);
        if let Some(frames) = self.mix_frames {
            set(&o, field::MIX_FRAMES, frames as f64);
        }
        (o, Array::new())
    }

    pub fn from_js(msg: &JsValue) -> Result<Self, Malformed> {
        let kind = kind_of(msg);
        let bad = |text: String| Malformed {
            kind: kind.clone(),
            text: format!("{kind} message: {text}"),
        };
        if kind != kind::DECODE {
            return Err(bad(format!("not a decode request: {kind:?}")));
        }
        let session = text_field(msg, field::SESSION).map_err(bad)?;
        let list = get(msg, field::SOURCES);
        if !Array::is_array(&list) {
            return Err(bad(format!("{} must be an array", field::SOURCES)));
        }
        let sources = Array::from(&list)
            .iter()
            .map(|entry| {
                if !entry.is_object() {
                    return Err(format!("a source must be an object, not {entry:?}"));
                }
                Ok(Source {
                    file: text_field(&entry, field::FILE)?,
                    index: whole(&entry, field::INDEX, f64::from(u32::MAX))? as u32,
                    ext: text_field(&entry, field::EXT)?,
                    in_mix: flag_field(&entry, field::IN_MIX)?,
                })
            })
            .collect::<Result<_, _>>()
            .map_err(bad)?;
        let mix_frames = if get(msg, field::MIX_FRAMES).is_undefined() {
            None
        } else {
            Some(whole(msg, field::MIX_FRAMES, MAX_SAFE).map_err(bad)? as u64)
        };
        Ok(Self {
            session,
            sources,
            mix_frames,
        })
    }
}

impl DecodeReply {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Alive => kind::ALIVE,
            Self::Decoded { .. } => kind::DECODED,
            Self::Undecodable { .. } => kind::UNDECODABLE,
            Self::Unstored { .. } => kind::UNSTORED,
            Self::Log(_) => kind::LOG,
            Self::OverviewsMade => kind::OVERVIEWS_MADE,
            Self::ClassicMade => kind::CLASSIC_MADE,
        }
    }

    /// The message to post, and its transfer list, which is empty.
    pub fn to_js(&self) -> (Object, Array) {
        let o = typed(self.kind());
        match self {
            Self::Alive | Self::OverviewsMade | Self::ClassicMade => {}
            Self::Decoded {
                frames,
                new_mix,
                channels,
            } => {
                set(&o, field::FRAMES, *frames as f64);
                set(&o, field::NEW_MIX, *new_mix);
                let list: Array = channels.iter().map(|c| JsValue::from(*c)).collect();
                set(&o, field::CHANNELS, list);
            }
            Self::Undecodable { text } | Self::Unstored { text } | Self::Log(text) => {
                set(&o, field::TEXT, text.as_str());
            }
        }
        (o, Array::new())
    }

    pub fn from_js(msg: &JsValue) -> Result<Self, Malformed> {
        let kind = kind_of(msg);
        let bad = |text: String| Malformed {
            kind: kind.clone(),
            text: format!("{kind} message: {text}"),
        };
        let text = || text_field(msg, field::TEXT).map_err(bad);
        Ok(match kind.as_str() {
            kind::ALIVE => Self::Alive,
            kind::DECODED => {
                let list = get(msg, field::CHANNELS);
                if !Array::is_array(&list) {
                    return Err(bad(format!("{} must be an array", field::CHANNELS)));
                }
                let channels = Array::from(&list)
                    .iter()
                    .map(|c| match c.as_f64() {
                        Some(n) if n == 1.0 || n == 2.0 => Ok(n as u8),
                        _ => Err(format!(
                            "{} must each be 1 or 2, not {c:?}",
                            field::CHANNELS
                        )),
                    })
                    .collect::<Result<_, _>>()
                    .map_err(bad)?;
                Self::Decoded {
                    frames: whole(msg, field::FRAMES, MAX_SAFE).map_err(bad)? as u64,
                    new_mix: flag_field(msg, field::NEW_MIX).map_err(bad)?,
                    channels,
                }
            }
            kind::UNDECODABLE => Self::Undecodable { text: text()? },
            kind::UNSTORED => Self::Unstored { text: text()? },
            kind::LOG => Self::Log(text()?),
            kind::OVERVIEWS_MADE => Self::OverviewsMade,
            kind::CLASSIC_MADE => Self::ClassicMade,
            other => return Err(bad(format!("not a decode reply: {other:?}"))),
        })
    }
}

/// The largest whole number that a JS number holds exactly: 2^53 - 1.
const MAX_SAFE: f64 = 9_007_199_254_740_991.0;

/// The text of field `k`, which must be a string.
fn text_field(o: &JsValue, k: &str) -> Result<String, String> {
    get(o, k)
        .as_string()
        .ok_or_else(|| format!("{k} must be text"))
}

/// The value of field `k`, which must be `true` or `false`.
fn flag_field(o: &JsValue, k: &str) -> Result<bool, String> {
    get(o, k)
        .as_bool()
        .ok_or_else(|| format!("{k} must be true or false"))
}

/// The value of field `k`, which must be a whole number from 0 to `most`.
fn whole(o: &JsValue, k: &str, most: f64) -> Result<f64, String> {
    get(o, k)
        .as_f64()
        .filter(|n| n.is_finite() && *n >= 0.0 && n.fract() == 0.0 && *n <= most)
        .ok_or_else(|| format!("{k} must be a whole number from 0 to {most}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_reply_names_its_type() {
        let text = || "t".to_string();
        let kinds: Vec<&str> = [
            DecodeReply::Alive,
            DecodeReply::Decoded {
                frames: 1,
                new_mix: true,
                channels: vec![2],
            },
            DecodeReply::Undecodable { text: text() },
            DecodeReply::Unstored { text: text() },
            DecodeReply::Log(text()),
            DecodeReply::OverviewsMade,
            DecodeReply::ClassicMade,
        ]
        .iter()
        .map(DecodeReply::kind)
        .collect();
        assert_eq!(
            kinds,
            [
                "decode-alive",
                "decoded",
                "undecodable",
                "decode-unstored",
                "decode-log",
                "overviews-made",
                "classic-made"
            ]
        );
    }

    #[test]
    fn no_type_is_a_type_of_the_stem_worker() {
        let stem_worker = [
            crate::kind::INIT,
            crate::kind::MODEL,
            crate::kind::DOWNLOAD,
            crate::kind::FORGET_MODEL,
            crate::kind::SEPARATE,
            crate::kind::SELFTEST,
            crate::kind::BENCH,
            crate::kind::MP3,
            crate::kind::FLAC,
            crate::kind::ALIVE,
            crate::kind::PROGRESS,
            crate::kind::SEGMENT,
            crate::kind::STORED,
            crate::kind::UNSTORED,
            crate::kind::DONE,
            crate::kind::ERROR,
            crate::kind::STATUS,
            crate::kind::LOG,
        ];
        for decode in [
            kind::DECODE,
            kind::ALIVE,
            kind::DECODED,
            kind::UNDECODABLE,
            kind::UNSTORED,
            kind::LOG,
            kind::OVERVIEWS,
            kind::OVERVIEWS_MADE,
            kind::CLASSIC,
            kind::CLASSIC_MADE,
        ] {
            assert!(!stem_worker.contains(&decode), "{decode}");
        }
    }
}
