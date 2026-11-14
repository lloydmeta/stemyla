//! A page-side fake of the write worker, for the browser tests (M5.1 of
//! `docs/plans/active/gpu-speed.md`): the test runner serves no worker
//! build.  It sends `write-ready`, takes each request through a
//! structured clone with its transfers, as `postMessage` does, and serves
//! it with write streams, as the page wrote before the write worker: a
//! whole file with `st_write`, and the blocks of a file through one stream,
//! which its block at 0 opens and its `write-end` closes (or aborts, for an
//! abandoned write).  So each file leaves one swap file of its size in
//! Safari (TD-103).  It replies as the worker does, with a block's array
//! given back, and a size check at a `write-end`.  So the app's tests, their spies and their
//! refusals of write streams keep their meaning.  It is compiled only with
//! `cfg(test)` or the `fake-writer` feature.

use js_sys::{Array, Object, Uint8Array};
use stem_protocol::write::{Body, Outcome, WriteReply, WriteRequest};
use wasm_bindgen::prelude::*;

#[wasm_bindgen(inline_js = r#"
async function dir(path, create, from) {
  let d = from || await navigator.storage.getDirectory();
  for (const p of path.split("/").filter((s) => s)) d = await d.getDirectoryHandle(p, { create });
  return d;
}
function split(path) {
  const i = path.lastIndexOf("/");
  return i < 0 ? ["", path] : [path.slice(0, i), path.slice(i + 1)];
}
// A port that serves each request with `serve`, one at a time, in order.
// A request that `serve` rejects ends with the reply `failed(text)`, and
// the queue goes on, so each request gets exactly one reply.
export function fake_new(serve, ready, ready_transfer, failed) {
  const port = { onmessage: null };
  let tail = Promise.resolve();
  const deliver = (msg, transfer) => {
    const data = structuredClone(msg, { transfer });
    if (port.onmessage) port.onmessage({ data });
  };
  port.postMessage = (msg, transfer) => {
    const data = structuredClone(msg, { transfer });
    tail = tail
      .then(() => serve(data))
      .then(
        ([reply, back]) => deliver(reply, back),
        (e) => deliver(failed(`the fake could not serve the request: ${e}`), []),
      )
      .catch((e) => console.error("writer fake: a reply was lost:", e));
  };
  setTimeout(() => deliver(ready, ready_transfer), 0);
  return port;
}
// The open write stream of each block-written file, by its path: one
// stream for each file, from its block at 0 to its `write-end`, so a test
// run leaves one swap file of the file's size in Safari (TD-103).
const streams = new Map();
// The folder of `within/rel` and the file's name; null when the folder
// `within` is gone.
async function place(within, rel) {
  let base;
  try { base = await dir(within, false); }
  catch (e) { if (within && e.name === "NotFoundError") return null; throw e; }
  const [d, n] = split("/" + rel);
  return [await dir(d, true, base), n];
}
async function drop(key) {
  const w = streams.get(key);
  streams.delete(key);
  if (w) try { await w.abort(); } catch (_) {}
}
// Writes `bytes` at `at` of `within/rel` through the file's write stream,
// which a block at 0 opens: "written", "gone", or the name of the error.
export async function fake_block(within, rel, at, bytes) {
  const key = within + "/" + rel;
  try {
    const found = await place(within, rel);
    if (!found) return "gone";
    const [d, n] = found;
    if (at === 0) {
      await drop(key);
      streams.set(key, await (await d.getFileHandle(n, { create: true })).createWritable());
    } else if (!streams.has(key)) {
      // A block after a refused or a lost first block: one stream that
      // keeps the file's bytes.
      streams.set(key, await (await d.getFileHandle(n, { create: true })).createWritable({ keepExistingData: true }));
    }
    const w = streams.get(key);
    await w.seek(at);
    await w.write(bytes);
    return "written";
  } catch (e) {
    await drop(key);
    return (e && e.name) || "Error";
  }
}
// Ends `within/rel`: closes its stream and checks the file's size, or, for
// an abandoned write (`size` null), aborts the stream and removes the file.
export async function fake_end(within, rel, size) {
  const key = within + "/" + rel;
  try {
    const found = await place(within, rel);
    if (!found) { await drop(key); return "gone"; }
    const [d, n] = found;
    if (size === null) {
      await drop(key);
      try { await d.removeEntry(n); } catch (e) { if (e.name !== "NotFoundError") throw e; }
      return "written";
    }
    const w = streams.get(key);
    streams.delete(key);
    if (w) await w.close();
    const held = (await (await d.getFileHandle(n)).getFile()).size;
    return held === size ? "written" : `the file holds ${held} of ${size} bytes`;
  } catch (e) {
    await drop(key);
    return (e && e.name) || "Error";
  }
}
"#)]
extern "C" {
    fn fake_new(
        serve: &js_sys::Function,
        ready: &Object,
        ready_transfer: &Array,
        failed: &js_sys::Function,
    ) -> JsValue;
    async fn fake_block(within: &str, rel: &str, at: f64, bytes: &Uint8Array) -> JsValue;
    async fn fake_end(within: &str, rel: &str, size: &JsValue) -> JsValue;
}

/// A new fake port.
pub fn port() -> JsValue {
    let serve = Closure::<dyn FnMut(JsValue) -> js_sys::Promise>::new(|data: JsValue| {
        wasm_bindgen_futures::future_to_promise(async move { Ok(serve(data).await) })
    });
    let port = port_with(serve.as_ref().unchecked_ref());
    // One closure for each port, which lives as long as the port.
    serve.forget();
    port
}

/// A new fake port that serves each request with the JS function `serve`.
/// A test of the fake's queue gives its own `serve`.
pub(super) fn port_with(serve: &js_sys::Function) -> JsValue {
    let (ready, transfer) = WriteReply::Ready.to_js();
    // One closure for each port, which lives as long as the port.
    let failed = Closure::<dyn Fn(String) -> Object>::new(|text: String| {
        WriteReply::Done {
            outcome: Outcome::Failed(text),
            bytes: None,
        }
        .to_js()
        .0
    })
    .into_js_value();
    fake_new(serve, &ready, &transfer, failed.unchecked_ref())
}

/// The outcome from `st_write`'s or `fake_block`'s answer.
fn outcome(answer: &JsValue) -> Outcome {
    match (answer.as_bool(), answer.as_string().as_deref()) {
        (Some(true), _) | (_, Some("written")) => Outcome::Written,
        (_, Some("gone")) => Outcome::Gone,
        (_, Some(name)) => Outcome::Failed(name.into()),
        _ => Outcome::Failed("Error".into()),
    }
}

/// Serves one request: the reply and its transfer list, as a pair.
async fn serve(data: JsValue) -> JsValue {
    let reply = match WriteRequest::from_js(&data) {
        Err(bad) => WriteReply::Done {
            outcome: Outcome::Failed(bad.text),
            bytes: None,
        },
        Ok(WriteRequest::File { within, rel, body }) => {
            let body: JsValue = match body {
                Body::Blob(blob) => blob.into(),
                Body::Bytes(bytes) => bytes.into(),
            };
            WriteReply::Done {
                outcome: outcome(&crate::store::fake_write_whole(&within, &rel, &body).await),
                bytes: None,
            }
        }
        Ok(WriteRequest::Block {
            within,
            rel,
            at,
            bytes,
        }) => WriteReply::Done {
            outcome: outcome(&fake_block(&within, &rel, at as f64, &bytes).await),
            bytes: Some(bytes),
        },
        Ok(WriteRequest::End { within, rel, size }) => {
            let size = size.map_or(JsValue::NULL, |n| JsValue::from(n as f64));
            WriteReply::Done {
                outcome: outcome(&fake_end(&within, &rel, &size).await),
                bytes: None,
            }
        }
    };
    let (msg, transfer) = reply.to_js();
    Array::of2(&msg, &transfer).into()
}
