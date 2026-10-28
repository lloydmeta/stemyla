//! Every message survives the trip it makes: encoded, structured-cloned with
//! its transfer list as `postMessage` does, and decoded on the other side
//! (`cargo run -p xtask -- web-test`).

#![cfg(target_arch = "wasm32")]
#![expect(
    clippy::disallowed_methods,
    reason = "MEM-1: test code, which does not ship"
)]

use js_sys::{Array, Float32Array, Object, Reflect, Uint8Array};
use stem_protocol::decode::{
    ClassicRequest, DecodeReply, DecodeRequest, LaneEncoding, LaneRef, OverviewLane,
    OverviewsRequest, Source, WorkerRequest,
};
use stem_protocol::{
    Encode, Encoded, Malformed, Mix, ModelSource, Reply, Request, SegmentStem, Settings, ToWorker,
};
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen(inline_js = r#"
export function post(msg, transfer) { return structuredClone(msg, { transfer }); }
export function blob(bytes) { return new Blob([bytes]); }
export function from_json(text) { return JSON.parse(text); }
// A storage-path separation, as `xtask e2e stored-stems` writes it: its
// `opts` and `session`, with short channels.
export function hand_written_separate() {
  const opts = { mm: "s64", attn: "split", chunk: 16384, overlap: 2, step_limit: 600 };
  return { type: "separate", left: new Float32Array([0.5]), right: new Float32Array([0.25]),
    session: "sessions/1792640000000", ...opts };
}
// A separation of a stored mix, as `xtask e2e stored-stems` writes it.
export function hand_written_stored_separate() {
  const opts = { mm: "s64", attn: "split", chunk: 16384, overlap: 2, step_limit: 600 };
  return { type: "separate", mix: "sessions/1792640000000/mix.f32", frames: 132300,
    session: "sessions/1792640000000", ...opts };
}
// A stored-mix request whose `frames` is the given value, or missing if
// `undefined`: JSON has no NaN.
export function stored_separate_with_frames(frames) {
  const msg = { type: "separate", mix: "sessions/1/mix.f32" };
  if (frames !== undefined) { msg.frames = frames; }
  return msg;
}
// A stored FLAC encode, as `xtask e2e flac` writes it.
export function hand_written_stored_flac() {
  return { type: "flac", name: "tone.flac", wav: "saves/e2e-1.wav", frames: 13230000,
    out: "saves/e2e-1.flac", rate: 44100 };
}
"#)]
extern "C" {
    /// What the receiving side of `postMessage` gets.
    fn post(msg: &Object, transfer: &Array) -> JsValue;
    fn blob(bytes: &[u8]) -> web_sys::Blob;
    fn from_json(text: &str) -> JsValue;
    fn hand_written_separate() -> JsValue;
    fn hand_written_stored_separate() -> JsValue;
    fn stored_separate_with_frames(frames: JsValue) -> JsValue;
    fn hand_written_stored_flac() -> JsValue;
}

fn samples(v: &[f32]) -> Float32Array {
    Float32Array::from(v)
}

fn settings() -> Settings {
    Settings {
        chunk: Some(16384),
        overlap: Some(2),
        step_limit: Some(600.0),
        mm: Some("s64".into()),
        model_url: Some("/weights/m.safetensors".into()),
        profile: true,
        debug: true,
        ..Settings::default()
    }
}

fn request_trip(r: &Request, s: &Settings) -> ToWorker {
    let (msg, transfer) = r.to_js(s);
    ToWorker::from_js(&post(&msg, &transfer)).expect("decodes")
}

fn reply_trip(r: &Reply) -> Reply {
    let (msg, transfer) = r.to_js();
    Reply::from_js(&post(&msg, &transfer)).expect("decodes")
}

#[wasm_bindgen_test]
fn requests_and_their_settings_arrive_intact() {
    let s = settings();
    for r in [
        Request::Init,
        Request::Download,
        Request::ForgetModel,
        Request::Bench,
        Request::Selftest {
            base: Some("/selftest/".into()),
        },
    ] {
        let got = request_trip(&r, &s);
        assert_eq!(got.request.kind(), r.kind());
        assert_eq!(got.settings, s, "{}", r.kind());
    }
    let got = request_trip(&Request::Selftest { base: None }, &Settings::default());
    assert!(matches!(got.request, Request::Selftest { base: None }));
    assert_eq!(got.settings, Settings::default());
}

#[wasm_bindgen_test]
fn a_model_arrives_as_the_file_or_the_bytes_it_was_sent_as() {
    let s = Settings::default();
    let file = request_trip(&Request::Model(Some(ModelSource::File(blob(&[1, 2])))), &s);
    match file.request {
        Request::Model(Some(ModelSource::File(b))) => assert_eq!(b.size(), 2.0),
        other => panic!("{other:?}"),
    }
    let bytes = Uint8Array::from(&[7u8, 8, 9][..]);
    let got = request_trip(&Request::Model(Some(ModelSource::Bytes(bytes))), &s);
    match got.request {
        Request::Model(Some(ModelSource::Bytes(b))) => assert_eq!(b.to_vec(), [7, 8, 9]),
        other => panic!("{other:?}"),
    }
    let none = request_trip(&Request::Model(None), &s);
    assert!(matches!(none.request, Request::Model(None)));
}

#[wasm_bindgen_test]
fn a_mix_is_transferred_not_copied() {
    let (left, right) = (samples(&[0.5, -0.5]), samples(&[0.25, 0.75]));
    let r = Request::Separate {
        mix: Mix::Arrays {
            left: left.clone(),
            right: right.clone(),
        },
        session: None,
    };
    let got = request_trip(&r, &Settings::default());
    match got.request {
        Request::Separate {
            mix: Mix::Arrays { left: l, right: r },
            session,
        } => {
            assert_eq!(l.to_vec(), [0.5, -0.5]);
            assert_eq!(r.to_vec(), [0.25, 0.75]);
            assert_eq!(session, None, "the memory path");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(left.length(), 0, "the sender's buffer was transferred");
    assert_eq!(right.length(), 0);
}

#[wasm_bindgen_test]
fn a_separation_into_storage_names_its_session() {
    let (left, right) = (samples(&[0.5, -0.5]), samples(&[0.25, 0.75]));
    let r = Request::Separate {
        mix: Mix::Arrays {
            left: left.clone(),
            right: right.clone(),
        },
        session: Some("sessions/1792640000000".into()),
    };
    let (msg, transfer) = r.to_js(&Settings::default());
    assert_eq!(transfer.length(), 2, "MEM-2: both channels are transferred");
    match ToWorker::from_js(&post(&msg, &transfer))
        .expect("decodes")
        .request
    {
        Request::Separate {
            mix: Mix::Arrays { left: l, right: r },
            session,
        } => {
            assert_eq!(session.as_deref(), Some("sessions/1792640000000"));
            assert_eq!(
                (l.to_vec(), r.to_vec()),
                (vec![0.5, -0.5], vec![0.25, 0.75])
            );
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(left.length() + right.length(), 0, "transferred");
}

#[wasm_bindgen_test]
fn a_stored_mix_arrives_with_its_path_and_frames() {
    for session in [Some("sessions/1792640000000".to_string()), None] {
        let r = Request::Separate {
            mix: Mix::Stored {
                path: "sessions/1792640000000/mix.f32".into(),
                frames: 10_584_000,
            },
            session: session.clone(),
        };
        let (msg, transfer) = r.to_js(&settings());
        assert_eq!(transfer.length(), 0, "no samples cross");
        let got = ToWorker::from_js(&post(&msg, &transfer)).expect("decodes");
        match got.request {
            Request::Separate {
                mix: Mix::Stored { path, frames },
                session: s,
            } => {
                assert_eq!(path, "sessions/1792640000000/mix.f32");
                assert_eq!(frames, 10_584_000);
                assert_eq!(s, session);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(got.settings, settings());
    }
}

/// TD-16: a separation that does not name exactly one mix, or a stored
/// mix without whole frames, is refused, not read as a default.
#[wasm_bindgen_test]
fn a_separation_must_name_one_mix_with_whole_frames() {
    let err = |msg: &JsValue| ToWorker::from_js(msg).expect_err("refused");
    let neither = err(&from_json(
        r#"{"type": "separate", "session": "sessions/1"}"#,
    ));
    assert_eq!(neither.kind, "separate");
    assert_eq!(neither.text, "left must be a Float32Array");
    let both = hand_written_separate();
    let _ = Reflect::set(&both, &"mix".into(), &"sessions/1/mix.f32".into());
    let _ = Reflect::set(&both, &"frames".into(), &1.0.into());
    assert_eq!(
        err(&both).text,
        "a separation names a stored mix or two arrays, not both"
    );
    for channel in ["left", "right"] {
        let one_array =
            from_json(r#"{"type": "separate", "mix": "sessions/1/mix.f32", "frames": 4}"#);
        let _ = Reflect::set(&one_array, &channel.into(), &samples(&[0.5]));
        assert_eq!(
            err(&one_array).text,
            "a separation names a stored mix or two arrays, not both",
            "a stored mix with {channel} only"
        );
    }
    let not_text = from_json(r#"{"type": "separate", "mix": 5, "frames": 4}"#);
    assert_eq!(err(&not_text).text, "mix must be a path");
    for frames in [
        JsValue::UNDEFINED,
        JsValue::from_f64(f64::NAN),
        JsValue::from_f64(-1.0),
        JsValue::from_f64(1.5),
        JsValue::from_f64(f64::INFINITY),
        JsValue::from_str("4"),
        // Over a wasm32 usize, which `as usize` would saturate.
        JsValue::from_f64(2f64.powi(33)),
        JsValue::from_f64(usize::MAX as f64 + 1.0),
    ] {
        let e = err(&stored_separate_with_frames(frames.clone()));
        assert_eq!(e.kind, "separate", "{frames:?}");
        assert_eq!(
            e.text,
            format!(
                "frames must be a whole number of frames, at most {}",
                usize::MAX
            ),
            "{frames:?}"
        );
    }
    let most = ToWorker::from_js(&stored_separate_with_frames(JsValue::from_f64(
        usize::MAX as f64,
    )));
    assert!(
        matches!(
            most.map(|w| w.request),
            Ok(Request::Separate {
                mix: Mix::Stored {
                    frames: usize::MAX,
                    ..
                },
                ..
            })
        ),
        "usize::MAX frames is the most"
    );
    let zero = ToWorker::from_js(&stored_separate_with_frames(JsValue::from_f64(0.0)));
    assert!(
        matches!(
            zero.map(|w| w.request),
            Ok(Request::Separate {
                mix: Mix::Stored { frames: 0, .. },
                session: None
            })
        ),
        "0 frames is a whole number"
    );
}

#[wasm_bindgen_test]
fn stored_and_unstored_replies_arrive_intact() {
    let r = Reply::Stored {
        frames: 44_100,
        len: 88_200,
        stems: vec!["bass".into(), "drums".into()],
    };
    let (_, transfer) = r.to_js();
    assert_eq!(transfer.length(), 0, "no samples cross");
    match reply_trip(&r) {
        Reply::Stored { frames, len, stems } => {
            assert_eq!((frames, len), (44_100, 88_200));
            assert_eq!(stems, ["bass", "drums"]);
        }
        other => panic!("{other:?}"),
    }
    match reply_trip(&Reply::Unstored {
        text: "QuotaExceededError: the disk is full".into(),
    }) {
        Reply::Unstored { text } => assert_eq!(text, "QuotaExceededError: the disk is full"),
        other => panic!("{other:?}"),
    }
}

#[wasm_bindgen_test]
fn a_stored_reply_without_its_counts_is_refused() {
    for json in [
        r#"{"type": "stored", "len": 4, "stems": ["bass"]}"#,
        r#"{"type": "stored", "frames": 4, "stems": ["bass"]}"#,
        r#"{"type": "stored", "frames": 4, "len": 4}"#,
        r#"{"type": "stored", "frames": 4, "len": 4, "stems": [1]}"#,
    ] {
        let e = Reply::from_js(&from_json(json)).expect_err(json);
        assert_eq!(e.kind, "stored", "{json}");
    }
}

#[wasm_bindgen_test]
fn an_mp3_request_keeps_its_name_and_rate() {
    let r = Request::Mp3 {
        name: Some("bass.mp3".into()),
        source: Encode::Arrays {
            left: samples(&[0.1]),
            right: samples(&[0.2]),
        },
        rate: Some(48_000),
    };
    match request_trip(&r, &Settings::default()).request {
        Request::Mp3 {
            name,
            rate,
            source: Encode::Arrays { left, .. },
        } => {
            assert_eq!(name.as_deref(), Some("bass.mp3"));
            assert_eq!(rate, Some(48_000));
            assert_eq!(left.to_vec(), [0.1]);
        }
        other => panic!("{other:?}"),
    }
}

#[wasm_bindgen_test]
fn a_flac_request_keeps_its_name_and_rate() {
    let (left, right) = (samples(&[0.1]), samples(&[0.2]));
    let r = Request::Flac {
        name: Some("bass.flac".into()),
        source: Encode::Arrays {
            left: left.clone(),
            right: right.clone(),
        },
        rate: Some(48_000),
    };
    match request_trip(&r, &Settings::default()).request {
        Request::Flac {
            name,
            rate,
            source: Encode::Arrays { left: l, right: r },
        } => {
            assert_eq!(name.as_deref(), Some("bass.flac"));
            assert_eq!(rate, Some(48_000));
            assert_eq!((l.to_vec(), r.to_vec()), (vec![0.1], vec![0.2]));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(left.length() + right.length(), 0, "transferred");
}

fn stored_encode() -> Encode {
    Encode::Stored {
        wav: "saves/1792640000000-3.wav".into(),
        frames: 13_230_000,
        out: "saves/1792640000000-3.flac".into(),
    }
}

/// D5 of M8: a stored encode names its files, and no samples cross.
#[wasm_bindgen_test]
fn a_stored_encode_names_its_files_and_transfers_nothing() {
    for r in [
        Request::Flac {
            name: Some("lloyd - mix.flac".into()),
            source: stored_encode(),
            rate: Some(44_100),
        },
        Request::Mp3 {
            name: Some("lloyd - mix.mp3".into()),
            source: stored_encode(),
            rate: Some(44_100),
        },
    ] {
        let (msg, transfer) = r.to_js(&Settings::default());
        assert_eq!(transfer.length(), 0, "{}: no samples cross", r.kind());
        let got = ToWorker::from_js(&post(&msg, &transfer)).expect("decodes");
        assert_eq!(got.request.kind(), r.kind());
        let (name, source, rate) = match got.request {
            Request::Flac { name, source, rate } | Request::Mp3 { name, source, rate } => {
                (name, source, rate)
            }
            other => panic!("{other:?}"),
        };
        assert_eq!(rate, Some(44_100));
        assert!(name.is_some_and(|n| n.starts_with("lloyd - mix.")));
        match source {
            Encode::Stored { wav, frames, out } => {
                assert_eq!(wav, "saves/1792640000000-3.wav");
                assert_eq!(frames, 13_230_000);
                assert_eq!(out, "saves/1792640000000-3.flac");
            }
            other => panic!("{other:?}"),
        }
    }
}

/// D5 of M8, TD-16: an encode names exactly one source, and a stored one
/// names its whole frames and its output.  A missing field is refused,
/// not read as a default.
#[wasm_bindgen_test]
fn an_encode_must_name_one_source_with_whole_frames_and_an_output() {
    for kind in ["flac", "mp3"] {
        let err = |json: &str| {
            let e = ToWorker::from_js(&from_json(&json.replace("KIND", kind)))
                .expect_err(&format!("{kind}: {json}"));
            assert_eq!(e.kind, kind, "{json}");
            e.text
        };
        assert_eq!(
            err(r#"{"type": "KIND", "name": "x"}"#),
            "left must be a Float32Array"
        );
        assert_eq!(
            err(r#"{"type": "KIND", "wav": "saves/1-1.wav", "frames": 4}"#),
            "a stored encode names its out file"
        );
        // Nit 1-12 of review round 1 of M8: the rate of a stored encode is
        // not read as a default.
        assert_eq!(
            err(
                r#"{"type": "KIND", "wav": "saves/1-1.wav", "frames": 4, "out": "saves/1-1.flac"}"#
            ),
            "a stored encode names its rate"
        );
        assert_eq!(
            err(r#"{"type": "KIND", "wav": "saves/1-1.wav", "frames": 4, "out": 5}"#),
            "out must be a path"
        );
        assert_eq!(
            err(r#"{"type": "KIND", "wav": 5, "frames": 4, "out": "saves/1-1.flac"}"#),
            "wav must be a path"
        );
        assert_eq!(
            err(r#"{"type": "KIND", "frames": 4, "out": "saves/1-1.flac"}"#),
            "an encode with out or frames names its wav"
        );
        for frames in [r#""4""#, "-1", "1.5", "null"] {
            let json = format!(
                r#"{{"type": "KIND", "wav": "saves/1-1.wav", "frames": {frames}, "out": "saves/1-1.flac"}}"#
            );
            assert_eq!(
                err(&json),
                format!(
                    "frames must be a whole number of frames, at most {}",
                    usize::MAX
                ),
                "{frames}"
            );
        }
        assert_eq!(
            err(r#"{"type": "KIND", "wav": "saves/1-1.wav", "out": "saves/1-1.flac"}"#),
            format!(
                "frames must be a whole number of frames, at most {}",
                usize::MAX
            ),
        );
        for channel in ["left", "right"] {
            let both = from_json(
                &r#"{"type": "KIND", "wav": "saves/1-1.wav", "frames": 4, "out": "saves/1-1.flac"}"#
                    .replace("KIND", kind),
            );
            let _ = Reflect::set(&both, &channel.into(), &samples(&[0.5]));
            let e = ToWorker::from_js(&both).expect_err("refused");
            assert_eq!(
                e.text, "an encode names a stored WAV or two arrays, not both",
                "{kind} with {channel}"
            );
        }
    }
}

/// As `xtask e2e` writes them by hand.
#[wasm_bindgen_test]
fn hand_written_requests_are_read() {
    let got = ToWorker::from_js(&from_json(
        r#"{"type": "download", "model_url": "/weights/m", "mm": "s64", "step_limit": 600}"#,
    ))
    .unwrap();
    assert!(matches!(got.request, Request::Download));
    assert_eq!(got.settings.model_url.as_deref(), Some("/weights/m"));
    assert_eq!(got.settings.mm.as_deref(), Some("s64"));
    assert_eq!(got.settings.step_limit, Some(600.0));
    let init = ToWorker::from_js(&from_json(r#"{"type": "init", "chunk": "big"}"#)).unwrap();
    assert_eq!(init.settings.chunk, None, "not a number: the default");
    let separate = ToWorker::from_js(&hand_written_separate()).unwrap();
    match separate.request {
        Request::Separate {
            mix: Mix::Arrays { left, right },
            session,
        } => {
            assert_eq!(session.as_deref(), Some("sessions/1792640000000"));
            assert_eq!((left.to_vec(), right.to_vec()), (vec![0.5], vec![0.25]));
        }
        other => panic!("{other:?}"),
    }
    let stored = ToWorker::from_js(&hand_written_stored_separate()).unwrap();
    match stored.request {
        Request::Separate {
            mix: Mix::Stored { path, frames },
            session,
        } => {
            assert_eq!(path, "sessions/1792640000000/mix.f32");
            assert_eq!(frames, 132_300);
            assert_eq!(session.as_deref(), Some("sessions/1792640000000"));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(stored.settings.chunk, Some(16384));
    match ToWorker::from_js(&hand_written_stored_flac())
        .unwrap()
        .request
    {
        Request::Flac {
            name,
            source: Encode::Stored { wav, frames, out },
            rate,
        } => {
            assert_eq!(name.as_deref(), Some("tone.flac"));
            assert_eq!(wav, "saves/e2e-1.wav");
            assert_eq!(frames, 13_230_000);
            assert_eq!(out, "saves/e2e-1.flac");
            assert_eq!(rate, Some(44_100));
        }
        other => panic!("{other:?}"),
    }
    let s = separate.settings;
    assert_eq!((s.chunk, s.overlap), (Some(16384), Some(2)));
    assert_eq!(
        (s.mm.as_deref(), s.attn.as_deref()),
        (Some("s64"), Some("split"))
    );
    assert_eq!(s.step_limit, Some(600.0));
}

#[wasm_bindgen_test]
fn an_unknown_or_incomplete_request_names_its_type() {
    let err = |json| ToWorker::from_js(&from_json(json)).expect_err("refused");
    assert_eq!(
        err(r#"{"type": "dance"}"#),
        Malformed {
            kind: "dance".into(),
            text: "unknown message dance".into()
        }
    );
    let e = err(r#"{"type": "separate", "left": [1, 2]}"#);
    assert_eq!(e.kind, "separate");
    assert_eq!(e.text, "left must be a Float32Array");
    assert_eq!(err("{}").kind, "");
}

#[wasm_bindgen_test]
fn replies_arrive_intact() {
    match reply_trip(&Reply::Model {
        ok: true,
        loaded: true,
        stored: false,
        text: "model loaded".into(),
    }) {
        Reply::Model {
            ok,
            loaded,
            stored,
            text,
        } => assert!(ok && loaded && !stored && text == "model loaded"),
        other => panic!("{other:?}"),
    }
    match reply_trip(&Reply::Progress {
        done: 3,
        total: 6,
        secs: 1.5,
    }) {
        Reply::Progress { done, total, secs } => assert_eq!((done, total, secs), (3, 6, 1.5)),
        other => panic!("{other:?}"),
    }
    match reply_trip(&Reply::Error {
        of: "separate".into(),
        text: "GPU lost".into(),
    }) {
        Reply::Error { of, text } => {
            assert_eq!((of.as_str(), text.as_str()), ("separate", "GPU lost"))
        }
        other => panic!("{other:?}"),
    }
    for (r, want) in [
        (Reply::Status("s".into()), "status"),
        (Reply::Log("l".into()), "log"),
        (Reply::Bench("b".into()), "bench"),
        (Reply::Alive, "alive"),
        (Reply::Done { secs: 2.0 }, "done"),
        (
            Reply::Download {
                done: 1.0,
                total: 2.0,
            },
            "download",
        ),
        (
            Reply::Selftest {
                ok: false,
                text: "t".into(),
            },
            "selftest",
        ),
    ] {
        assert_eq!(reply_trip(&r).kind(), want);
    }
}

#[wasm_bindgen_test]
fn a_segment_keeps_its_stems_in_order_and_transfers_them() {
    let (bass, drums) = (samples(&[1.0, 2.0]), samples(&[3.0, 4.0]));
    let r = Reply::Segment {
        start: 44_100,
        len: 88_200,
        stems: vec![
            SegmentStem {
                name: "bass".into(),
                left: bass.clone(),
                right: samples(&[-1.0, -2.0]),
            },
            SegmentStem {
                name: "drums".into(),
                left: drums.clone(),
                right: samples(&[-3.0, -4.0]),
            },
        ],
    };
    match reply_trip(&r) {
        Reply::Segment { start, len, stems } => {
            assert_eq!((start, len), (44_100, 88_200));
            let names: Vec<_> = stems.iter().map(|s| s.name.as_str()).collect();
            assert_eq!(names, ["bass", "drums"]);
            assert_eq!(stems[1].left.to_vec(), [3.0, 4.0]);
            assert_eq!(stems[1].right.to_vec(), [-3.0, -4.0]);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(bass.length() + drums.length(), 0, "transferred");
}

#[wasm_bindgen_test]
fn an_mp3_reply_transfers_its_file() {
    let bytes = Uint8Array::from(&[0xffu8, 0xfb][..]);
    match reply_trip(&Reply::Mp3 {
        name: "bass.mp3".into(),
        file: Encoded::Bytes(bytes.clone()),
    }) {
        Reply::Mp3 {
            name,
            file: Encoded::Bytes(b),
        } => {
            assert_eq!((name.as_str(), b.to_vec()), ("bass.mp3", vec![0xff, 0xfb]))
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(bytes.length(), 0);
}

/// A `Blob` is not transferable: it crosses as a reference to the same
/// file, so the reply keeps it whole and the page does not copy it.
#[wasm_bindgen_test]
fn a_flac_reply_keeps_its_blob() {
    let parts = Array::of1(&Uint8Array::from(&b"fLaC"[..]));
    let file = web_sys::Blob::new_with_u8_array_sequence(&parts).expect("a blob");
    match reply_trip(&Reply::Flac {
        name: "bass.flac".into(),
        file: Encoded::Blob(file),
    }) {
        Reply::Flac {
            name,
            file: Encoded::Blob(file),
        } => assert_eq!((name.as_str(), file.size()), ("bass.flac", 4.0)),
        other => panic!("{other:?}"),
    }
}

/// D5 of M8: a stored encode's reply names its output file, or the
/// reason it failed, and transfers nothing.
#[wasm_bindgen_test]
fn a_stored_encode_reply_names_its_file_or_why_it_failed() {
    let made = |name: &str, file: Encoded, flac: bool| {
        let name = name.to_string();
        if flac {
            Reply::Flac { name, file }
        } else {
            Reply::Mp3 { name, file }
        }
    };
    for flac in [true, false] {
        let stored = made(
            "bass",
            Encoded::Stored {
                path: "saves/1-2.flac".into(),
            },
            flac,
        );
        let (_, transfer) = stored.to_js();
        assert_eq!(transfer.length(), 0, "nothing crosses");
        let file = match reply_trip(&stored) {
            Reply::Flac { name, file } | Reply::Mp3 { name, file } => {
                assert_eq!(name, "bass");
                file
            }
            other => panic!("{other:?}"),
        };
        assert!(
            matches!(&file, Encoded::Stored { path } if path == "saves/1-2.flac"),
            "{file:?}"
        );
        let failed = made(
            "bass",
            Encoded::Failed {
                path: "saves/1-2.mp3".into(),
                text: "QuotaExceededError: the disk is full".into(),
            },
            flac,
        );
        let (_, transfer) = failed.to_js();
        assert_eq!(transfer.length(), 0, "nothing crosses");
        match reply_trip(&failed) {
            Reply::Flac {
                file: Encoded::Failed { path, text },
                ..
            }
            | Reply::Mp3 {
                file: Encoded::Failed { path, text },
                ..
            } => {
                assert_eq!(path, "saves/1-2.mp3");
                assert_eq!(text, "QuotaExceededError: the disk is full");
            }
            other => panic!("{other:?}"),
        }
    }
}

#[wasm_bindgen_test]
fn a_flac_reply_without_a_file_is_refused() {
    for (json, why) in [
        (
            r#"{"type": "flac", "name": "bass.flac", "file": 1}"#,
            "no file",
        ),
        (r#"{"type": "flac", "name": "bass.flac"}"#, "no file"),
        (r#"{"type": "mp3", "name": "bass.mp3"}"#, "no bytes"),
        (
            r#"{"type": "mp3", "name": "bass.mp3", "bytes": [1]}"#,
            "no bytes",
        ),
        (
            r#"{"type": "flac", "name": "bass.flac", "path": 3}"#,
            "path must be a path",
        ),
        (
            r#"{"type": "mp3", "name": "bass.mp3", "path": "saves/1-1.mp3", "failed": 3}"#,
            "failed must be text",
        ),
    ] {
        let kind = if json.contains(r#""type": "flac""#) {
            "flac"
        } else {
            "mp3"
        };
        let e = Reply::from_js(&from_json(json)).expect_err(json);
        assert_eq!(e.kind, kind, "{json}");
        assert_eq!(e.text, format!("{kind} message: {why}"), "{json}");
    }
    // Nit 1-13 of review round 1 of M8: an MP3 carries bytes, and a FLAC a
    // `Blob`, never the other way round.
    let flac_with_bytes = from_json(r#"{"type": "flac", "name": "bass.flac"}"#);
    let _ = Reflect::set(
        &flac_with_bytes,
        &"bytes".into(),
        &Uint8Array::from(&[1u8][..]),
    );
    let e = Reply::from_js(&flac_with_bytes).expect_err("refused");
    assert_eq!(e.text, "flac message: no file");
    let mp3_with_a_blob = from_json(r#"{"type": "mp3", "name": "bass.mp3"}"#);
    let _ = Reflect::set(&mp3_with_a_blob, &"file".into(), &blob(&[1]));
    let e = Reply::from_js(&mp3_with_a_blob).expect_err("refused");
    assert_eq!(e.text, "mp3 message: no bytes");
}

#[wasm_bindgen_test]
fn a_malformed_segment_is_refused_with_its_type() {
    let o = from_json(r#"{"type": "segment", "start": 0, "len": 4, "stems": [["bass", 1, 2]]}"#);
    let e = Reply::from_js(&o).expect_err("refused");
    assert_eq!(e.kind, "segment");
    assert!(e.text.contains("malformed stem"), "{}", e.text);
    let missing = Object::new();
    let _ = Reflect::set(&missing, &"type".into(), &"segment".into());
    assert!(Reply::from_js(&missing).is_err());
}

#[wasm_bindgen_test]
fn a_model_url_is_taken_only_from_the_pages_own_site() {
    let page = "http://127.0.0.1:8080/app/?mm=s64&model_url=";
    let url = |model: &str| Settings::from_page(&format!("{page}{model}")).model_url;
    assert_eq!(
        url("/weights/m.safetensors").as_deref(),
        Some("/weights/m.safetensors")
    );
    assert_eq!(url("m.safetensors").as_deref(), Some("m.safetensors"));
    let own = "http://127.0.0.1:8080/weights/m";
    assert_eq!(url(own).as_deref(), Some(own));
    for other in [
        "https://example.com/m.safetensors",
        "//example.com/m.safetensors",
        "http://127.0.0.1:9090/m",
        "javascript:alert(1)",
        "",
    ] {
        assert_eq!(url(other), None, "{other}");
    }
    let s = Settings::from_page(&format!("{page}https://example.com/m"));
    assert_eq!(s.mm.as_deref(), Some("s64"), "the other settings are kept");
    assert_eq!(Settings::from_page("not a url"), Settings::default());
}

/// MEM-1: proves each path in this crate's `clippy.toml` (see the `#[expect]`s).
#[wasm_bindgen_test]
fn mem1_lint_is_on_for_js_arrays() {
    #[expect(
        clippy::disallowed_methods,
        reason = "MEM-1 canary: clippy fails here if this path is off"
    )]
    let slice = [1.0_f32].to_vec();
    #[expect(
        clippy::disallowed_methods,
        reason = "MEM-1 canary: clippy fails here if this path is off"
    )]
    let bytes = js_sys::Uint8Array::from(&[2_u8][..]).to_vec();
    #[expect(
        clippy::disallowed_methods,
        reason = "MEM-1 canary: clippy fails here if this path is off"
    )]
    let floats = js_sys::Float32Array::from(&[3.0_f32][..]).to_vec();
    assert_eq!((slice, bytes, floats), (vec![1.0], vec![2], vec![3.0]));
}

fn decode_request() -> DecodeRequest {
    DecodeRequest {
        session: "sessions/1792640000000".into(),
        sources: vec![
            Source {
                file: "src/0.wav".into(),
                index: 0,
                ext: "wav".into(),
                in_mix: true,
            },
            Source {
                file: "src/1.flac".into(),
                index: 1,
                ext: "flac".into(),
                in_mix: false,
            },
        ],
        mix_frames: Some(132_300),
    }
}

fn decode_replies() -> Vec<DecodeReply> {
    vec![
        DecodeReply::Alive,
        DecodeReply::Decoded {
            frames: 132_300,
            new_mix: true,
            channels: vec![2, 1],
        },
        DecodeReply::Decoded {
            frames: 1,
            new_mix: false,
            channels: vec![],
        },
        DecodeReply::Undecodable {
            text: "unsupported or unrecognised format".into(),
        },
        DecodeReply::Unstored {
            text: "QuotaExceededError".into(),
        },
        DecodeReply::Log(
            "decoder: memory: decoder wasm 2 MB at the start of the decode request".into(),
        ),
    ]
}

#[wasm_bindgen_test]
fn decode_messages_arrive_intact_with_nothing_transferred() {
    for request in [
        decode_request(),
        DecodeRequest {
            mix_frames: None,
            ..decode_request()
        },
    ] {
        let (msg, transfer) = request.to_js();
        assert_eq!(transfer.length(), 0, "no samples cross");
        assert_eq!(
            DecodeRequest::from_js(&post(&msg, &transfer)),
            Ok(request.clone())
        );
    }
    for reply in decode_replies() {
        let (msg, transfer) = reply.to_js();
        assert_eq!(transfer.length(), 0, "no samples cross");
        assert_eq!(
            DecodeReply::from_js(&post(&msg, &transfer)),
            Ok(reply.clone())
        );
    }
}

/// `text`, a decode message in JSON, is refused, and the reason names the
/// field.
fn refused_request(text: &str, field: &str) {
    let got = DecodeRequest::from_js(&from_json(text));
    match got {
        Err(Malformed { kind, text: why }) => {
            assert_eq!(kind, "decode", "{text}");
            assert!(why.contains(field), "{text}: {why}");
        }
        Ok(r) => panic!("{text} was read as {r:?}"),
    }
}

#[wasm_bindgen_test]
fn a_decode_request_with_a_missing_or_wrong_field_is_refused() {
    let source = r#"{"file": "src/0.wav", "index": 0, "ext": "wav", "in_mix": true}"#;
    let request = |fields: &str| format!(r#"{{"type": "decode", {fields}}}"#);
    let with_sources = |sources: &str| {
        request(&format!(
            r#""session": "sessions/1", "sources": [{sources}]"#
        ))
    };
    refused_request(&request(&format!(r#""sources": [{source}]"#)), "session");
    refused_request(
        &request(&format!(r#""session": 7, "sources": [{source}]"#)),
        "session",
    );
    refused_request(&request(r#""session": "sessions/1""#), "sources");
    refused_request(
        &request(r#""session": "sessions/1", "sources": {}"#),
        "sources",
    );
    refused_request(&with_sources("7"), "source");
    refused_request(
        &with_sources(&source.replace(r#""file": "src/0.wav", "#, "")),
        "file",
    );
    refused_request(
        &with_sources(&source.replace(r#""index": 0"#, r#""index": 0.5"#)),
        "index",
    );
    refused_request(
        &with_sources(&source.replace(r#""index": 0"#, r#""index": -1"#)),
        "index",
    );
    refused_request(
        &with_sources(&source.replace(r#""index": 0"#, r#""index": 4294967296"#)),
        "index",
    );
    refused_request(
        &with_sources(&source.replace(r#", "ext": "wav""#, "")),
        "ext",
    );
    refused_request(
        &with_sources(&source.replace(r#""in_mix": true"#, r#""in_mix": 1"#)),
        "in_mix",
    );
    refused_request(
        &with_sources(&source.replace(r#", "in_mix": true"#, "")),
        "in_mix",
    );
    for frames in ["-1", "0.5", "\"12\"", "null", "9007199254740993"] {
        refused_request(
            &request(&format!(
                r#""session": "sessions/1", "sources": [{source}], "mix_frames": {frames}"#
            )),
            "mix_frames",
        );
    }
    // No sources is a request the page never makes, but it is well formed.
    assert_eq!(
        DecodeRequest::from_js(&from_json(&with_sources(""))).map(|r| r.sources.len()),
        Ok(0)
    );
}

/// `text`, a decode reply in JSON, is refused, and the reason names the
/// field.
fn refused_reply(text: &str, field: &str) {
    match DecodeReply::from_js(&from_json(text)) {
        Err(Malformed { text: why, .. }) => assert!(why.contains(field), "{text}: {why}"),
        Ok(r) => panic!("{text} was read as {r:?}"),
    }
}

#[wasm_bindgen_test]
fn a_decode_reply_with_a_missing_or_wrong_field_is_refused() {
    let decoded = |fields: &str| format!(r#"{{"type": "decoded", {fields}}}"#);
    refused_reply(&decoded(r#""new_mix": true, "channels": [2]"#), "frames");
    refused_reply(
        &decoded(r#""frames": 0.5, "new_mix": true, "channels": [2]"#),
        "frames",
    );
    refused_reply(&decoded(r#""frames": 4, "channels": [2]"#), "new_mix");
    refused_reply(
        &decoded(r#""frames": 4, "new_mix": "yes", "channels": [2]"#),
        "new_mix",
    );
    refused_reply(&decoded(r#""frames": 4, "new_mix": true"#), "channels");
    for channel in ["0", "3", "1.5", "\"2\"", "null"] {
        refused_reply(
            &decoded(&format!(
                r#""frames": 4, "new_mix": true, "channels": [2, {channel}]"#
            )),
            "channels",
        );
    }
    for kind in ["undecodable", "decode-unstored", "decode-log"] {
        refused_reply(&format!(r#"{{"type": "{kind}"}}"#), "text");
        refused_reply(&format!(r#"{{"type": "{kind}", "text": 7}}"#), "text");
    }
}

fn lane_ref(path: &str, encoding: LaneEncoding) -> LaneRef {
    LaneRef {
        path: path.into(),
        encoding,
        data_offset: 44,
        data_frames: 132_299,
    }
}

/// D7 and D8 of the M7 plan: one of each request of the decode worker.
fn worker_requests() -> Vec<WorkerRequest> {
    let lanes = vec![
        lane_ref("stems/bass.wav", LaneEncoding::Pcm16),
        LaneRef {
            data_offset: 0,
            data_frames: 132_300,
            ..lane_ref("decoded/0.f32", LaneEncoding::F32)
        },
    ];
    let overviews = OverviewsRequest {
        session: "sessions/1792640000000".into(),
        frames: 132_300,
        lanes: lanes
            .iter()
            .enumerate()
            .map(|(k, lane)| OverviewLane {
                lane: lane.clone(),
                overview: format!("overviews/stems/{k}.f32.3.decode.part"),
            })
            .collect(),
        mix: Some("overviews/stems/mix.f32.3.decode.part".into()),
    };
    vec![
        WorkerRequest::Decode(decode_request()),
        WorkerRequest::Overviews(overviews.clone()),
        WorkerRequest::Overviews(OverviewsRequest {
            mix: None,
            lanes: Vec::new(),
            ..overviews
        }),
        WorkerRequest::Classic(ClassicRequest {
            session: "sessions/1792640000000".into(),
            frames: 132_300,
            lanes,
            wav: "classic.wav.decode.part".into(),
            overview: "overviews/classic.f32.decode.part".into(),
        }),
    ]
}

#[wasm_bindgen_test]
fn worker_requests_and_the_new_replies_arrive_intact_with_nothing_transferred() {
    for request in worker_requests() {
        let (msg, transfer) = request.to_js();
        assert_eq!(transfer.length(), 0, "no samples cross");
        assert_eq!(
            WorkerRequest::from_js(&post(&msg, &transfer)),
            Ok(request.clone())
        );
    }
    // A decode request reads the same either way.
    let (msg, transfer) = decode_request().to_js();
    assert_eq!(
        WorkerRequest::from_js(&post(&msg, &transfer)),
        Ok(WorkerRequest::Decode(decode_request()))
    );
    for reply in [DecodeReply::OverviewsMade, DecodeReply::ClassicMade] {
        let (msg, transfer) = reply.to_js();
        assert_eq!(transfer.length(), 0);
        assert_eq!(
            DecodeReply::from_js(&post(&msg, &transfer)),
            Ok(reply.clone())
        );
    }
}

/// `text`, a request of the decode worker in JSON, is refused, and the
/// reason names `field`.
fn refused_worker_request(text: &str, kind: &str, field: &str) {
    match WorkerRequest::from_js(&from_json(text)) {
        Err(Malformed {
            kind: got,
            text: why,
        }) => {
            assert_eq!(got, kind, "{text}");
            assert!(why.contains(field), "{text}: {why}");
        }
        Ok(r) => panic!("{text} was read as {r:?}"),
    }
}

#[wasm_bindgen_test]
fn an_overviews_or_classic_request_with_a_missing_or_wrong_field_is_refused() {
    let lane =
        r#"{"path": "stems/bass.wav", "format": "pcm16", "data_offset": 44, "data_frames": 10}"#;
    let with_overview = lane.replace(
        '}',
        r#", "overview": "overviews/stems/0.f32.1.decode.part"}"#,
    );
    let overviews = |fields: &str| format!(r#"{{"type": "overviews", {fields}}}"#);
    let classic = |fields: &str| format!(r#"{{"type": "classic", {fields}}}"#);
    let ok_overviews = |lanes: &str| {
        overviews(&format!(
            r#""session": "sessions/1", "frames": 10, "lanes": [{lanes}]"#
        ))
    };
    let ok_classic = |lanes: &str| {
        classic(&format!(
            r#""session": "sessions/1", "frames": 10, "lanes": [{lanes}], "wav": "classic.wav.decode.part", "overview": "overviews/classic.f32.decode.part""#
        ))
    };
    assert!(WorkerRequest::from_js(&from_json(&ok_overviews(&with_overview))).is_ok());
    assert!(WorkerRequest::from_js(&from_json(&ok_classic(lane))).is_ok());
    for (text, field) in [
        (
            overviews(&format!(r#""frames": 10, "lanes": [{with_overview}]"#)),
            "session",
        ),
        (
            overviews(&format!(
                r#""session": 1, "frames": 10, "lanes": [{with_overview}]"#
            )),
            "session",
        ),
        (
            overviews(&format!(r#""session": "s", "lanes": [{with_overview}]"#)),
            "frames",
        ),
        (
            overviews(&format!(
                r#""session": "s", "frames": -1, "lanes": [{with_overview}]"#
            )),
            "frames",
        ),
        (overviews(r#""session": "s", "frames": 10"#), "lanes"),
        (
            overviews(r#""session": "s", "frames": 10, "lanes": {}"#),
            "lanes",
        ),
        (ok_overviews("7"), "lane"),
        // Each reason starts with "overviews message: ", so the check
        // names the field's own phrase (finding 1-15 of review round 1 of
        // M7).
        (ok_overviews(lane), "overview must be text"),
        (
            ok_overviews(&with_overview.replace(r#""path": "stems/bass.wav", "#, "")),
            "path",
        ),
        (
            ok_overviews(&with_overview.replace(r#""pcm16""#, r#""pcm24""#)),
            "format",
        ),
        (
            ok_overviews(&with_overview.replace(r#""format": "pcm16", "#, "")),
            "format",
        ),
        (
            ok_overviews(&with_overview.replace(r#""data_offset": 44"#, r#""data_offset": 4.5"#)),
            "data_offset",
        ),
        (
            ok_overviews(&with_overview.replace(r#""data_frames": 10"#, r#""data_frames": "10""#)),
            "data_frames",
        ),
        (
            overviews(&format!(
                r#""session": "s", "frames": 10, "lanes": [{with_overview}], "mix": 7"#
            )),
            "mix",
        ),
    ] {
        refused_worker_request(&text, "overviews", field);
    }
    for (text, field) in [
        (
            classic(&format!(
                r#""frames": 10, "lanes": [{lane}], "wav": "w", "overview": "o""#
            )),
            "session",
        ),
        (
            classic(&format!(
                r#""session": "s", "lanes": [{lane}], "wav": "w", "overview": "o""#
            )),
            "frames",
        ),
        (
            classic(r#""session": "s", "frames": 10, "wav": "w", "overview": "o""#),
            "lanes",
        ),
        (
            classic(&format!(
                r#""session": "s", "frames": 10, "lanes": [{lane}], "overview": "o""#
            )),
            "wav",
        ),
        (
            classic(&format!(
                r#""session": "s", "frames": 10, "lanes": [{lane}], "wav": "w""#
            )),
            "overview",
        ),
        (
            ok_classic(&lane.replace(r#""data_offset": 44, "#, "")),
            "data_offset",
        ),
    ] {
        refused_worker_request(&text, "classic", field);
    }
    refused_worker_request(r#"{"type": "lloyd"}"#, "lloyd", "not a request");
}

#[wasm_bindgen_test]
fn a_message_for_one_worker_is_not_read_as_one_for_the_other() {
    for reply in decode_replies() {
        let (msg, transfer) = reply.to_js();
        let err = Reply::from_js(&post(&msg, &transfer)).expect_err("not a stem worker reply");
        assert_eq!(err.kind, reply.kind());
    }
    let stem_replies = [
        Reply::Alive,
        Reply::Unstored { text: "t".into() },
        Reply::Log("t".into()),
        Reply::Done { secs: 1.0 },
    ];
    for reply in stem_replies {
        let (msg, transfer) = reply.to_js();
        let err = DecodeReply::from_js(&post(&msg, &transfer)).expect_err("not a decode reply");
        assert_eq!(err.kind, reply.kind());
    }
    let (msg, transfer) = decode_request().to_js();
    assert!(ToWorker::from_js(&post(&msg, &transfer)).is_err());
    let (msg, transfer) = Request::Init.to_js(&Settings::default());
    assert!(DecodeRequest::from_js(&post(&msg, &transfer)).is_err());
    assert!(WorkerRequest::from_js(&post(&msg, &transfer)).is_err());
    for request in worker_requests() {
        let (msg, transfer) = request.to_js();
        assert!(
            ToWorker::from_js(&post(&msg, &transfer)).is_err(),
            "{request:?}"
        );
    }
    for reply in [DecodeReply::OverviewsMade, DecodeReply::ClassicMade] {
        let (msg, transfer) = reply.to_js();
        let err = Reply::from_js(&post(&msg, &transfer)).expect_err("not a stem worker reply");
        assert_eq!(err.kind, reply.kind());
    }
}
