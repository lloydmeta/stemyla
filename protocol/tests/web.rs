//! Every message survives the trip it makes: encoded, structured-cloned with
//! its transfer list as `postMessage` does, and decoded on the other side
//! (`cargo run -p xtask -- web-test`).

#![cfg(target_arch = "wasm32")]

use js_sys::{Array, Float32Array, Object, Reflect, Uint8Array};
use stem_protocol::{Malformed, ModelSource, Reply, Request, SegmentStem, Settings, ToWorker};
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen(inline_js = r#"
export function post(msg, transfer) { return structuredClone(msg, { transfer }); }
export function blob(bytes) { return new Blob([bytes]); }
export function from_json(text) { return JSON.parse(text); }
"#)]
extern "C" {
    /// What the receiving side of `postMessage` gets.
    fn post(msg: &Object, transfer: &Array) -> JsValue;
    fn blob(bytes: &[u8]) -> web_sys::Blob;
    fn from_json(text: &str) -> JsValue;
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
        left: left.clone(),
        right: right.clone(),
    };
    let got = request_trip(&r, &Settings::default());
    match got.request {
        Request::Separate { left: l, right: r } => {
            assert_eq!(l.to_vec(), [0.5, -0.5]);
            assert_eq!(r.to_vec(), [0.25, 0.75]);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(left.length(), 0, "the sender's buffer was transferred");
    assert_eq!(right.length(), 0);
}

#[wasm_bindgen_test]
fn an_mp3_request_keeps_its_name_and_rate() {
    let r = Request::Mp3 {
        name: Some("bass.mp3".into()),
        left: samples(&[0.1]),
        right: samples(&[0.2]),
        rate: Some(48_000),
    };
    match request_trip(&r, &Settings::default()).request {
        Request::Mp3 {
            name, rate, left, ..
        } => {
            assert_eq!(name.as_deref(), Some("bass.mp3"));
            assert_eq!(rate, Some(48_000));
            assert_eq!(left.to_vec(), [0.1]);
        }
        other => panic!("{other:?}"),
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
        bytes: bytes.clone(),
    }) {
        Reply::Mp3 { name, bytes: b } => {
            assert_eq!((name.as_str(), b.to_vec()), ("bass.mp3", vec![0xff, 0xfb]))
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(bytes.length(), 0);
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
    ] {
        assert_eq!(url(other), None, "{other}");
    }
    let s = Settings::from_page(&format!("{page}https://example.com/m"));
    assert_eq!(s.mm.as_deref(), Some("s64"), "the other settings are kept");
    assert_eq!(Settings::from_page("not a url"), Settings::default());
}
