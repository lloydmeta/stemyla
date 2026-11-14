//! Browser tests of the client of the write worker: with a worker that has
//! no script and a spy on its posts, and with the page-side fake.

#![expect(
    clippy::disallowed_methods,
    reason = "MEM-1: test code, which does not ship"
)]

use super::*;
use crate::store;
use wasm_bindgen_test::*;
use web_sys::Worker;

#[wasm_bindgen(inline_js = r#"
export function writer_spy_on_post(port) {
  const posts = [];
  port.postMessage = (data, transfer) => posts.push(data);
  return posts;
}
// Counts the calls of `createWritable` until `writer_writables_stop`.
let writables = null;
let real_writable = null;
export function writer_writables_start() {
  const proto = FileSystemFileHandle.prototype;
  real_writable = proto.createWritable;
  writables = 0;
  proto.createWritable = function (...args) {
    writables++;
    return real_writable.apply(this, args);
  };
}
export function writer_writables_stop() {
  FileSystemFileHandle.prototype.createWritable = real_writable;
  return writables;
}
// A port whose `postMessage` throws, as a post that cannot be cloned does.
export function writer_throwing_port() {
  return {
    onmessage: null,
    postMessage() { throw new DOMException("refused by the test", "DataCloneError"); },
  };
}
// A port that records its posts, and whose `postMessage` throws for a
// request whose `rel` ends in "x", so a test picks the posts that fail.
export function writer_recording_port() {
  const port = { onmessage: null, onerror: null, onmessageerror: null, posts: [] };
  port.postMessage = (data) => {
    if (String(data.rel).endsWith("x")) {
      throw new DOMException("refused by the test", "DataCloneError");
    }
    port.posts.push(data);
  };
  return port;
}
export function writer_posts(port) { return port.posts.length; }
// A `serve` for the fake that rejects a request whose `rel` ends in "x",
// and answers each other one with `reply`.
export function writer_serve_refusing(reply) {
  return async (data) => {
    if (String(data.rel).endsWith("x")) throw new Error("refused by the test");
    return [reply, []];
  };
}
export function writer_sleep(ms) { return new Promise((r) => setTimeout(r, ms)); }
export function writer_fail(port) { port.onerror({ message: "failed by the test" }); }
export function writer_tell(port, data) { port.onmessage({ data }); }
// Counts the calls of `Worker.prototype.terminate` until the stop.
let terminated = null;
let real_terminate = null;
export function writer_terminates_start() {
  real_terminate = Worker.prototype.terminate;
  terminated = 0;
  Worker.prototype.terminate = function () { terminated++; return real_terminate.call(this); };
}
export function writer_terminates_stop() {
  Worker.prototype.terminate = real_terminate;
  return terminated;
}
export function writer_deliver(port, data) {
  port.dispatchEvent(new MessageEvent("message", { data }));
}
"#)]
extern "C" {
    fn writer_spy_on_post(port: &JsValue) -> js_sys::Array;
    fn writer_deliver(port: &JsValue, data: &JsValue);
    fn writer_throwing_port() -> JsValue;
    fn writer_recording_port() -> JsValue;
    fn writer_posts(port: &JsValue) -> u32;
    fn writer_serve_refusing(reply: &JsValue) -> js_sys::Function;
    async fn writer_sleep(ms: u32);
    fn writer_fail(port: &JsValue);
    fn writer_tell(port: &JsValue, data: &JsValue);
    fn writer_terminates_start();
    fn writer_terminates_stop() -> u32;
    fn writer_writables_start();
    fn writer_writables_stop() -> u32;
}

const FOLDER: &str = "writer-tests";

/// A client whose worker has no script, with a spy on its posts.  The
/// worker's own `error` (no script) is cleared; a test sends its own.
fn client() -> (WriteClient, JsValue, js_sys::Array) {
    let client = WriteClient::of_worker("./no-such-write-worker.js");
    let port = client.port().expect("a worker");
    port.unchecked_ref::<Worker>().set_onerror(None);
    let posts = writer_spy_on_post(&port);
    (client, port, posts)
}

fn sent(reply: WriteReply) -> JsValue {
    reply.to_js().0.into()
}

fn block(rel: &str, at: u64, bytes: &[u8]) -> WriteRequest {
    WriteRequest::Block {
        within: String::new(),
        rel: format!("{FOLDER}/{rel}"),
        at,
        bytes: Uint8Array::from(bytes),
    }
}

/// A request posted before `write-ready` goes out after it, so a write
/// posted as the worker starts is not lost; the reply ends it.
#[wasm_bindgen_test]
async fn a_request_posted_before_ready_is_held_then_sent() {
    let (client, port, posts) = client();
    let ended = client.write(&block("a", 0, b"lloyd"));
    let before = posts.length();
    writer_deliver(&port, &sent(WriteReply::Ready));
    let after = posts.length();
    let first = WriteRequest::from_js(&posts.get(0)).map(|r| r.kind());
    writer_deliver(
        &port,
        &sent(WriteReply::Done {
            outcome: Outcome::Written,
            bytes: None,
        }),
    );
    let ended = ended.await.map(|(outcome, _)| outcome);
    client.stopped("the test ended");
    assert_eq!((before, after), (0, 1), "held until write-ready");
    assert_eq!(first.ok(), Some(stem_protocol::write::kind::BLOCK));
    assert_eq!(ended, Ok(Outcome::Written), "the reply ends the write");
}

/// MEM-2: a posted array is transferred (the page's array is empty after
/// the post), and a block's reply gives it back whole, with its bytes.
/// The fake writes it as the worker would.
#[wasm_bindgen_test]
async fn a_posted_array_is_transferred_and_a_block_gives_it_back_whole() {
    store::test_remove(FOLDER).await;
    let client = WriteClient::new(|| Ok(super::super::fake::port()));
    let bytes: Vec<u8> = (0..(2u32 << 20)).map(|i| (i % 251) as u8).collect();
    let array = Uint8Array::from(&bytes[..]);
    let request = WriteRequest::Block {
        within: String::new(),
        rel: format!("{FOLDER}/save.wav"),
        at: 0,
        bytes: array.clone(),
    };
    let ended = client.write(&request).await;
    let after_post = array.byte_length();
    // The fake holds a file's bytes in its write stream until the end.
    let end = client.write(&end("save.wav", Some(2 << 20))).await;
    let back = match &ended {
        Ok((_, Some(back))) => back.byte_length(),
        _ => 0,
    };
    let stored = store::test_read(&format!("{FOLDER}/save.wav")).await;
    store::test_remove(FOLDER).await;
    client.stopped("the test ended");
    assert_eq!(after_post, 0, "the page's array is transferred");
    assert_eq!(back, 2 << 20, "the array comes back whole");
    assert!(
        matches!(&ended, Ok((Outcome::Written, Some(b))) if b.to_vec() == bytes),
        "the same bytes come back"
    );
    assert_eq!(end.map(|(o, _)| o), Ok(Outcome::Written), "the end");
    assert!(stored == Some(bytes), "the block is in the file");
}

/// The fake serves whole files and blocks in order, as the worker does: a
/// later block keeps the bytes before it, and a write into a gone folder
/// is `gone`.
#[wasm_bindgen_test]
async fn the_fake_serves_files_and_blocks_in_order() {
    store::test_remove(FOLDER).await;
    let client = WriteClient::new(|| Ok(super::super::fake::port()));
    let whole = WriteRequest::File {
        within: String::new(),
        rel: format!("{FOLDER}/src/0.wav"),
        body: stem_protocol::write::Body::Bytes(Uint8Array::from(&b"acme"[..])),
    };
    let gone = WriteRequest::File {
        within: format!("{FOLDER}/gone"),
        rel: "src/0.wav".into(),
        body: stem_protocol::write::Body::Bytes(Uint8Array::from(&b"x"[..])),
    };
    let ends = [
        client.write(&whole),
        client.write(&block("mix.f32.part", 0, b"lloyd")),
        client.write(&block("mix.f32.part", 5, b"acme")),
        client.write(&end("mix.f32.part", Some(9))),
        client.write(&gone),
    ];
    let mut outcomes = Vec::new();
    for end in ends {
        outcomes.push(end.await.map(|(outcome, _)| outcome));
    }
    let src = store::test_read(&format!("{FOLDER}/src/0.wav")).await;
    let mix = store::test_read(&format!("{FOLDER}/mix.f32.part")).await;
    store::test_remove(FOLDER).await;
    client.stopped("the test ended");
    assert_eq!(
        outcomes,
        [
            Ok(Outcome::Written),
            Ok(Outcome::Written),
            Ok(Outcome::Written),
            Ok(Outcome::Written),
            Ok(Outcome::Gone)
        ]
    );
    assert_eq!(src.as_deref(), Some(&b"acme"[..]));
    assert_eq!(mix.as_deref(), Some(&b"lloydacme"[..]));
}

/// A worker that fails (an `error` event: here, its script does not load)
/// fails each request that waits, and the next write starts a new worker.
#[wasm_bindgen_test]
async fn a_failed_worker_fails_each_request_and_the_next_write_starts_a_new_one() {
    let client = WriteClient::of_worker("./no-such-write-worker.js");
    writer_terminates_start();
    let first = client.write(&block("a", 0, b"lloyd"));
    let second = client.write(&block("b", 0, b"acme"));
    let port = client.port().expect("a worker");
    let (first, second) = (first.await, second.await);
    let terminated = writer_terminates_stop();
    let again = client.port().expect("a new worker");
    again.unchecked_ref::<Worker>().set_onerror(None);
    let new_port = !js_sys::Object::is(&again, &port);
    client.stopped("the test ended");
    for (n, ended) in [first, second].into_iter().enumerate() {
        assert!(
            matches!(&ended, Err(reason) if reason.starts_with("the write worker stopped: ")),
            "write {n} fails: {:?}",
            ended.map(|(o, _)| o)
        );
    }
    assert!(new_port, "a new worker");
    assert_eq!(
        terminated, 1,
        "the failed worker is ended, so it holds no file"
    );
}

/// PROTO-1: a reply that cannot be read cannot be matched to a request, so
/// each request that waits fails, and no later reply ends the wrong one.
#[wasm_bindgen_test]
async fn a_reply_that_cannot_be_read_fails_each_request() {
    let (client, port, _posts) = client();
    writer_deliver(&port, &sent(WriteReply::Ready));
    let first = client.write(&block("a", 0, b"lloyd"));
    let bad = js_sys::Object::new();
    js_sys::Reflect::set(&bad, &"type".into(), &"write-done".into()).unwrap();
    writer_deliver(&port, &bad);
    let first = first.await;
    client.stopped("the test ended");
    assert!(
        matches!(&first, Err(reason) if reason.contains("could not be read")),
        "{:?}",
        first.map(|(o, _)| o)
    );
}

fn end(rel: &str, size: Option<u64>) -> WriteRequest {
    WriteRequest::End {
        within: String::new(),
        rel: format!("{FOLDER}/{rel}"),
        size,
    }
}

/// The ends of `requests`, each sent through the fake's client in order.
async fn through_the_fake(requests: &[WriteRequest]) -> Vec<Result<Outcome, String>> {
    let client = WriteClient::new(|| Ok(super::super::fake::port()));
    let ends: Vec<_> = requests.iter().map(|r| client.write(r)).collect();
    let mut outcomes = Vec::new();
    for end in ends {
        outcomes.push(end.await.map(|(outcome, _)| outcome));
    }
    client.stopped("the test ended");
    outcomes
}

/// A `write-end` whose file lost a block fails through the client, as the
/// worker does; the whole file ends `written`.
#[wasm_bindgen_test]
async fn a_lost_block_makes_the_end_fail_through_the_client() {
    store::test_remove(FOLDER).await;
    let got = through_the_fake(&[
        block("a.wav", 0, b"lloyd"),
        block("a.wav", 5, b"acme"),
        end("a.wav", Some(9)),
        block("b.wav", 0, b"lloyd"),
        end("b.wav", Some(9)),
    ])
    .await;
    store::test_remove(FOLDER).await;
    assert_eq!(got[2], Ok(Outcome::Written), "the whole file");
    assert_eq!(
        got[4],
        Ok(Outcome::Failed("the file holds 5 of 9 bytes".into())),
        "a lost block"
    );
}

/// An abandoned block write leaves no partial file.
#[wasm_bindgen_test]
async fn an_abandoned_block_write_leaves_no_partial_file() {
    store::test_remove(FOLDER).await;
    let got = through_the_fake(&[block("c.wav", 0, b"lloyd"), end("c.wav", None)]).await;
    let left = store::test_read(&format!("{FOLDER}/c.wav")).await;
    store::test_remove(FOLDER).await;
    assert_eq!(got[1], Ok(Outcome::Written));
    assert_eq!(left, None, "no partial file");
}

/// TD-103: the fake opens one write stream for each block-written file,
/// not one for each block, so a test run leaves one swap file of its size.
#[wasm_bindgen_test]
async fn the_fake_opens_one_write_stream_for_each_block_written_file() {
    store::test_remove(FOLDER).await;
    writer_writables_start();
    let got = through_the_fake(&[
        block("d.wav", 0, b"ll"),
        block("d.wav", 2, b"oy"),
        block("d.wav", 4, b"d"),
        end("d.wav", Some(5)),
    ])
    .await;
    let opened = writer_writables_stop();
    let file = store::test_read(&format!("{FOLDER}/d.wav")).await;
    store::test_remove(FOLDER).await;
    assert_eq!(got.last(), Some(&Ok(Outcome::Written)));
    assert_eq!(opened, 1, "one write stream for the file");
    assert_eq!(file.as_deref(), Some(&b"lloyd"[..]));
}

/// Finding 1-19 of review round 1: a post that throws fails its request,
/// at once or when it was held for `write-ready`, and leaves no request
/// waiting.
#[wasm_bindgen_test]
async fn a_post_that_throws_fails_its_request() {
    let client = WriteClient::new(|| Ok(writer_throwing_port()));
    let port = client.port().expect("the port");
    let held = client.write(&block("e.wav", 0, b"lloyd"));
    writer_tell(&port, &sent(WriteReply::Ready));
    let held = held.await;
    let at_once = client.write(&block("f.wav", 0, b"acme")).await;
    let waiting = client.0.borrow().waiting.len();
    client.stopped("the test ended");
    for (what, ended) in [("held", held), ("at once", at_once)] {
        assert!(
            matches!(&ended, Err(reason) if reason.contains("DataCloneError")),
            "{what}: {:?}",
            ended.map(|(o, _)| o)
        );
    }
    assert_eq!(waiting, 0, "no request waits");
}

fn done(outcome: Outcome) -> JsValue {
    sent(WriteReply::Done {
        outcome,
        bytes: None,
    })
}

/// The outcomes of `ends`, after `client` stops: a request that still
/// waits then ends with "the write worker stopped".
async fn outcomes(
    client: &WriteClient,
    ends: Vec<Pin<Box<dyn Future<Output = Done>>>>,
) -> Vec<Result<Outcome, String>> {
    client.stopped("the test ended");
    let mut outcomes = Vec::new();
    for end in ends {
        outcomes.push(end.await.map(|(outcome, _)| outcome));
    }
    outcomes
}

fn refused(ended: &Result<Outcome, String>) -> bool {
    matches!(ended, Err(reason) if reason.contains("DataCloneError"))
}

/// Finding 2-1 of review round 2: held posts that throw fail their own
/// requests, and each reply ends its own request.  Two posts throw, so
/// the order of the removals counts.
#[wasm_bindgen_test]
async fn held_posts_that_throw_fail_their_own_requests() {
    let client = WriteClient::new(|| Ok(writer_recording_port()));
    let port = client.port().expect("the port");
    let ends = ["a", "bx", "cx", "d"]
        .into_iter()
        .map(|rel| client.write(&block(rel, 0, b"lloyd")))
        .collect();
    writer_tell(&port, &sent(WriteReply::Ready));
    writer_tell(&port, &done(Outcome::Written));
    writer_tell(&port, &done(Outcome::Gone));
    let waiting = client.0.borrow().waiting.len();
    let posted = writer_posts(&port);
    let got = outcomes(&client, ends).await;
    assert_eq!(got[0], Ok(Outcome::Written), "a: its own reply");
    assert!(refused(&got[1]), "bx fails: {:?}", got[1]);
    assert!(refused(&got[2]), "cx fails: {:?}", got[2]);
    assert_eq!(got[3], Ok(Outcome::Gone), "d: its own reply");
    assert_eq!(waiting, 0, "no request waits");
    assert_eq!(posted, 2, "a and d are posted");
}

/// Finding 2-1 of review round 2: a post that throws after `write-ready`
/// fails its own request, not an earlier one that waits for its reply.
#[wasm_bindgen_test]
async fn a_post_that_throws_after_ready_fails_its_own_request() {
    let client = WriteClient::new(|| Ok(writer_recording_port()));
    let port = client.port().expect("the port");
    writer_tell(&port, &sent(WriteReply::Ready));
    let ends = vec![
        client.write(&block("a", 0, b"lloyd")),
        client.write(&block("bx", 0, b"acme")),
    ];
    writer_tell(&port, &done(Outcome::Written));
    let waiting = client.0.borrow().waiting.len();
    let got = outcomes(&client, ends).await;
    assert_eq!(got[0], Ok(Outcome::Written), "a: its own reply");
    assert!(refused(&got[1]), "bx fails: {:?}", got[1]);
    assert_eq!(waiting, 0, "no request waits");
}

/// Finding 2-2 of review round 2: a failed port drops its held posts, so
/// the new port posts only its own requests.
#[wasm_bindgen_test]
async fn a_new_port_posts_none_of_the_failed_ports_held_requests() {
    let client = WriteClient::new(|| Ok(writer_recording_port()));
    let old = client.port().expect("the port");
    let failed = client.write(&block("a", 0, b"lloyd"));
    writer_fail(&old);
    let failed = failed.await.map(|(outcome, _)| outcome);
    let next = client.write(&block("b", 0, b"acme"));
    let port = client.port().expect("a new port");
    let new_port = !js_sys::Object::is(&port, &old);
    writer_tell(&port, &sent(WriteReply::Ready));
    let posted = writer_posts(&port);
    writer_tell(&port, &done(Outcome::Written));
    let got = outcomes(&client, vec![next]).await;
    assert!(
        matches!(&failed, Err(reason) if reason.starts_with("the write worker stopped: ")),
        "{failed:?}"
    );
    assert!(new_port, "a new port");
    assert_eq!(posted, 1, "only the new port's request");
    assert_eq!(got[0], Ok(Outcome::Written), "the reply ends the new write");
}

/// Finding 2-11 of review round 2: a request that the fake cannot serve
/// ends with a failure, and the fake serves the requests after it.
#[wasm_bindgen_test]
async fn a_request_that_the_fake_cannot_serve_does_not_stall_the_next() {
    let serve = writer_serve_refusing(&done(Outcome::Written));
    let client = WriteClient::new(move || Ok(super::super::fake::port_with(&serve)));
    let ends: Vec<_> = ["a", "bx", "c"]
        .into_iter()
        .map(|rel| client.write(&block(rel, 0, b"lloyd")))
        .collect();
    for _ in 0..100 {
        if client.0.borrow().waiting.is_empty() {
            break;
        }
        writer_sleep(10).await;
    }
    let waiting = client.0.borrow().waiting.len();
    let got = outcomes(&client, ends).await;
    assert_eq!(waiting, 0, "no request waits: {got:?}");
    assert_eq!(got[0], Ok(Outcome::Written), "a");
    assert!(
        matches!(&got[1], Ok(Outcome::Failed(text)) if text.contains("refused by the test")),
        "bx fails: {:?}",
        got[1]
    );
    assert_eq!(got[2], Ok(Outcome::Written), "c is served");
}
