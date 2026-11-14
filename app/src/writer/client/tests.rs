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
export function writer_deliver(port, data) {
  port.dispatchEvent(new MessageEvent("message", { data }));
}
"#)]
extern "C" {
    fn writer_spy_on_post(port: &JsValue) -> js_sys::Array;
    fn writer_deliver(port: &JsValue, data: &JsValue);
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
    let first = client.write(&block("a", 0, b"lloyd"));
    let second = client.write(&block("b", 0, b"acme"));
    let port = client.port().expect("a worker");
    let (first, second) = (first.await, second.await);
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
