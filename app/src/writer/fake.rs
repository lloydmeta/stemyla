//! A page-side fake of the write worker, for the browser tests (M5.1 of
//! `docs/plans/active/gpu-speed.md`): the test runner serves no worker
//! build.  It sends `write-ready`, takes each request through a
//! structured clone with its transfers, as `postMessage` does, and serves
//! it with write streams, as the page wrote before the write worker: a
//! whole file with `st_write`, and a block with a stream that keeps the
//! file's bytes after a first block.  It replies the same way, with a
//! block's array given back.  So the app's tests, their spies and their
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
export function fake_new(serve, ready, ready_transfer) {
  const port = { onmessage: null };
  let tail = Promise.resolve();
  const deliver = (msg, transfer) => {
    const data = structuredClone(msg, { transfer });
    if (port.onmessage) port.onmessage({ data });
  };
  port.postMessage = (msg, transfer) => {
    const data = structuredClone(msg, { transfer });
    tail = tail.then(async () => {
      const [reply, back] = await serve(data);
      deliver(reply, back);
    });
  };
  setTimeout(() => deliver(ready, ready_transfer), 0);
  return port;
}
// Writes `bytes` at `at` of `within/rel` through a write stream that
// keeps the file's bytes when `at` is not 0: "written", "gone", or the
// name of the error.
export async function fake_block(within, rel, at, bytes) {
  let w;
  try {
    let base;
    try { base = await dir(within, false); }
    catch (e) { if (within && e.name === "NotFoundError") return "gone"; throw e; }
    const [d, n] = split("/" + rel);
    const fh = await (await dir(d, true, base)).getFileHandle(n, { create: true });
    w = await fh.createWritable({ keepExistingData: at > 0 });
    await w.seek(at);
    await w.write(bytes);
    await w.close();
    return "written";
  } catch (e) {
    if (w) try { await w.abort(); } catch (_) {}
    return (e && e.name) || "Error";
  }
}
"#)]
extern "C" {
    fn fake_new(serve: &js_sys::Function, ready: &Object, ready_transfer: &Array) -> JsValue;
    async fn fake_block(within: &str, rel: &str, at: f64, bytes: &Uint8Array) -> JsValue;
}

/// A new fake port.
pub fn port() -> JsValue {
    let serve = Closure::<dyn FnMut(JsValue) -> js_sys::Promise>::new(|data: JsValue| {
        wasm_bindgen_futures::future_to_promise(async move { Ok(serve(data).await) })
    });
    let (ready, transfer) = WriteReply::Ready.to_js();
    let port = fake_new(serve.as_ref().unchecked_ref(), &ready, &transfer);
    // One closure for each port, which lives as long as the port.
    serve.forget();
    port
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
    };
    let (msg, transfer) = reply.to_js();
    Array::of2(&msg, &transfer).into()
}
