//! Browser tests of the page's routed writes (M5.1 of
//! `docs/plans/active/gpu-speed.md`, Step 5c): the song-sized files go
//! through the write worker, and the small files stay on write streams.
//! A recording port stands in for the worker: it keeps each request and
//! answers `written`, with a block's array given back.

use super::{WriteClient, use_client};
use crate::store;
use engine::{Song, Stem};
use js_sys::{Array, Object, Uint8Array};
use std::cell::Cell;
use std::rc::Rc;
use stem_protocol::write::{Outcome, WriteReply, WriteRequest};
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

#[wasm_bindgen(inline_js = r#"
// A port that keeps each request, structured-cloned with its transfers,
// and answers it with `answer` at once, or when `release` is called if
// `hold` is true.
export function recording_port(ready, answer, hold) {
  const port = { onmessage: null, posts: [], held: [] };
  const deliver = (reply, back) => {
    const data = structuredClone(reply, { transfer: back });
    setTimeout(() => port.onmessage && port.onmessage({ data }), 0);
  };
  port.postMessage = (msg, transfer) => {
    const data = structuredClone(msg, { transfer });
    port.posts.push(data);
    const [reply, back] = answer(data);
    if (hold) port.held.push([reply, back]);
    else deliver(reply, back);
  };
  port.release = () => { for (const [reply, back] of port.held.splice(0)) deliver(reply, back); };
  setTimeout(() => port.onmessage && port.onmessage({ data: ready }), 0);
  return port;
}
export function posts_of(port) { return port.posts; }
export function release(port) { port.release(); }
// Records the name of each file that `createWritable` opens.
let names = null;
let real = null;
export function start_writable_names() {
  const proto = FileSystemFileHandle.prototype;
  real = proto.createWritable;
  names = [];
  proto.createWritable = function (...args) {
    names.push(this.name);
    return real.apply(this, args);
  };
}
export function stop_writable_names() {
  FileSystemFileHandle.prototype.createWritable = real;
  return names.join(" ");
}
export function blob_of(text) { return new Blob([text]); }
export function routing_sleep(ms) { return new Promise((r) => setTimeout(r, ms)); }
"#)]
extern "C" {
    fn recording_port(ready: &Object, answer: &js_sys::Function, hold: bool) -> JsValue;
    fn posts_of(port: &JsValue) -> Array;
    fn release(port: &JsValue);
    fn start_writable_names();
    fn stop_writable_names() -> String;
    fn blob_of(text: &str) -> web_sys::Blob;
    async fn routing_sleep(ms: u32);
}

/// A recording port, which answers `written` (at once, or on `release`
/// when `hold`), and the client of it, as the page's client.
fn recording(hold: bool) -> (JsValue, super::ClientGuard) {
    let answer = Closure::<dyn FnMut(JsValue) -> Array>::new(|data: JsValue| {
        let bytes = match WriteRequest::from_js(&data) {
            Ok(WriteRequest::Block { bytes, .. }) => Some(bytes),
            _ => None,
        };
        let (reply, back) = WriteReply::Done {
            outcome: Outcome::Written,
            bytes,
        }
        .to_js();
        Array::of2(&reply, &back)
    });
    let (ready, _) = WriteReply::Ready.to_js();
    let port = recording_port(&ready, answer.as_ref().unchecked_ref(), hold);
    answer.forget();
    let client_port = port.clone();
    let client = WriteClient::new(move || Ok(client_port.clone()));
    // Listens at once, so the port's `write-ready` is not missed.
    client.port().expect("the recording port");
    (port, use_client(client))
}

/// Each request that the port got: its type and its path.
fn requests(port: &JsValue) -> Vec<String> {
    posts_of(port)
        .iter()
        .map(|post| match WriteRequest::from_js(&post) {
            Ok(WriteRequest::File { within, rel, .. }) => format!("file {within}/{rel}"),
            Ok(WriteRequest::Block {
                within, rel, at, ..
            }) => {
                format!("block {within}/{rel} at {at}")
            }
            Ok(WriteRequest::End { within, rel, size }) => format!("end {within}/{rel} {size:?}"),
            Err(bad) => format!("unread {}", bad.text),
        })
        .collect()
}

fn song(frames: usize) -> Song {
    let left: Vec<f32> = (0..frames).map(|i| (i as f32 * 0.01).sin()).collect();
    Song {
        sample_rate: 44_100,
        frames,
        stems: vec![Stem {
            name: "bass".into(),
            left: left.clone().into(),
            right: left.into(),
        }],
    }
}

/// TD-103: each routed call site posts its file to the write worker, and
/// opens no write stream of it; `meta`, a small write, still does.
#[wasm_bindgen_test]
async fn each_routed_call_site_posts_its_file_and_opens_no_write_stream() {
    store::test_remove("sessions").await;
    store::test_remove(store::SAVES).await;
    let (port, guard) = recording(false);
    start_writable_names();
    let id = store::save_sources(&[("acme.wav".into(), blob_of("lloyd"))], 44_100).await;
    let id = id.unwrap_or_default();
    let stems = song(100).stems;
    let _ = store::save_stems(&id, 44_100, &stems).await;
    let shown = crate::lanes::Shown::in_memory(song(1_000));
    let (input, _) = crate::separation_input(&shown, shown.audio());
    let _ = store::write_mix(&id, input).await;
    let save = format!("{}/routing-1.wav", store::SAVES);
    if let Ok(stream) = store::save_open(&save).await {
        let _ = store::save_put(&stream, &Uint8Array::from(&b"acme"[..])).await;
        let _ = store::save_close(&stream, &save).await;
    }
    let names = stop_writable_names();
    let got = requests(&port);
    drop(guard);
    store::test_remove("sessions").await;
    store::test_remove(store::SAVES).await;
    let base = format!("sessions/{id}");
    for want in [
        format!("file /{base}/src/0.wav"),
        format!("file {base}/stems/bass.wav.page.part"),
        format!("block {base}/mix.f32.part at 0"),
        format!("block /{save} at 0"),
    ] {
        assert!(got.contains(&want), "{want} in {got:?}");
    }
    let names: Vec<&str> = names.split(' ').collect();
    for routed in [
        "0.wav",
        "bass.wav.page.part",
        "mix.f32.part",
        "routing-1.wav",
    ] {
        assert!(
            !names.contains(&routed),
            "no write stream of {routed}: {names:?}"
        );
    }
    assert!(
        names.contains(&"meta"),
        "meta stays on a write stream: {names:?}"
    );
}

/// A reset waits for a routed write in flight, as it waits for a write
/// stream (`store::close`), and a routed write after it starts is
/// refused.
#[wasm_bindgen_test]
async fn a_reset_waits_for_a_routed_write_and_refuses_a_later_one() {
    store::test_remove("sessions").await;
    let (port, guard) = recording(true);
    let saved = Rc::new(Cell::new(None));
    let saving = Rc::clone(&saved);
    wasm_bindgen_futures::spawn_local(async move {
        let id = store::save_sources(&[("acme.wav".into(), blob_of("lloyd"))], 44_100).await;
        saving.set(Some(id.is_ok()));
    });
    for _ in 0..400 {
        if posts_of(&port).length() > 0 {
            break;
        }
        routing_sleep(5).await;
    }
    let posted = posts_of(&port).length();
    let closed = Rc::new(Cell::new(false));
    let closing = Rc::clone(&closed);
    wasm_bindgen_futures::spawn_local(async move {
        store::close().await;
        closing.set(true);
    });
    routing_sleep(200).await;
    let closed_early = closed.get();
    release(&port);
    for _ in 0..400 {
        if closed.get() && saved.get().is_some() {
            break;
        }
        routing_sleep(5).await;
    }
    let later = store::save_sources(&[("beachape.wav".into(), blob_of("acme"))], 44_100).await;
    let after = posts_of(&port).length();
    store::test_reopen();
    drop(guard);
    store::test_remove("sessions").await;
    assert_eq!(posted, 1, "the source file is posted");
    assert!(!closed_early, "the reset waits for the routed write");
    assert!(closed.get(), "the reset ends after the write");
    // The save ends: its source file was written, and the reset refuses
    // its `meta`, a later write.
    assert_eq!(saved.get(), Some(false), "the save in flight ends");
    assert!(
        later
            .as_ref()
            .err()
            .is_some_and(|e| e.to_string().contains("being reset")),
        "{later:?}"
    );
    assert_eq!(after, 1, "nothing is posted after the reset");
}
