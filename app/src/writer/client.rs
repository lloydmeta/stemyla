//! The page's client of the write worker (M5.1 of
//! `docs/plans/active/gpu-speed.md`).
//!
//! It starts its port at the first write, and again after the port
//! stopped.  It holds each request until the port says `write-ready`, as
//! `decoder::DecodeClient` holds a decode, so a request posted before the
//! worker's handler is set is not lost.  The worker ends each request with
//! one `write-done`, in the order posted, so each reply belongs to the
//! oldest request not yet ended.  An `error` or `messageerror` event, or a
//! reply that cannot be read, ends the port (`terminate`, so it holds no
//! file) and fails each request that waits.
//!
//! The port is the write worker in the shipped page.  Under test
//! (`cfg(test)`, or the `fake-writer` feature that `xtask web-test` turns
//! on), it is a page-side fake (`super::fake`), since the test runner
//! serves no worker build.  A port is any object with `postMessage` and
//! `onmessage`.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use js_sys::{Array, Object, Uint8Array};
use stem_protocol::write::{Outcome, WriteReply, WriteRequest};
use wasm_bindgen::prelude::*;

#[wasm_bindgen(inline_js = r#"
export function port_listen(port, on_message, on_error) {
  port.onmessage = (e) => on_message(e.data);
  port.onerror = (e) => on_error((e && e.message) || "its script did not load");
  port.onmessageerror = () => on_error("a message to the page could not be read");
}
export function port_post(port, msg, transfer) { port.postMessage(msg, transfer); }
export function port_end(port) { if (typeof port.terminate === "function") port.terminate(); }
export function worker_port(url) { return new Worker(url, { type: "module" }); }
"#)]
extern "C" {
    fn port_listen(port: &JsValue, on_message: &js_sys::Function, on_error: &js_sys::Function);
    fn port_post(port: &JsValue, msg: &Object, transfer: &Array);
    fn port_end(port: &JsValue);
    #[wasm_bindgen(catch)]
    fn worker_port(url: &str) -> Result<JsValue, JsValue>;
}

/// How a write ended: the worker's outcome and the array that it gave
/// back, if any; or why the port stopped.
pub type Done = Result<(Outcome, Option<Uint8Array>), String>;

/// Makes a port.
type Start = Rc<dyn Fn() -> Result<JsValue, String>>;

/// A request posted and not yet ended.
struct Waiting {
    ended: Rc<RefCell<Option<Done>>>,
    resolve: js_sys::Function,
}

struct Inner {
    start: Start,
    port: Option<JsValue>,
    ready: bool,
    /// Posts made before `write-ready`, with their transfer lists.
    held: Vec<(Object, Array)>,
    waiting: VecDeque<Waiting>,
    _closures: Vec<Box<dyn std::any::Any>>,
}

/// A client of one port, shared by the page's writes.
#[derive(Clone)]
pub struct WriteClient(Rc<RefCell<Inner>>);

impl WriteClient {
    /// A client whose port `start` makes, at the first write.
    pub fn new(start: impl Fn() -> Result<JsValue, String> + 'static) -> Self {
        Self(Rc::new(RefCell::new(Inner {
            start: Rc::new(start),
            port: None,
            ready: false,
            held: Vec::new(),
            waiting: VecDeque::new(),
            _closures: Vec::new(),
        })))
    }

    /// A client of the write worker at `url` (its loader shim).
    pub fn of_worker(url: &str) -> Self {
        let url = url.to_string();
        Self::new(move || {
            worker_port(&url).map_err(|e| format!("the write worker could not start: {e:?}"))
        })
    }

    /// Posts `request`, with its typed array transferred, and gives how it
    /// ended.
    pub fn write(&self, request: &WriteRequest) -> Pin<Box<dyn Future<Output = Done>>> {
        let port = match self.port() {
            Ok(port) => port,
            Err(reason) => return Box::pin(std::future::ready(Err(reason))),
        };
        let (msg, transfer) = request.to_js();
        let mut resolve = None;
        let promise = js_sys::Promise::new(&mut |done, _| resolve = Some(done));
        let ended = Rc::new(RefCell::new(None));
        {
            let mut inner = self.0.borrow_mut();
            inner.waiting.push_back(Waiting {
                ended: ended.clone(),
                resolve: resolve.expect("a promise calls its executor at once"),
            });
            if inner.ready {
                port_post(&port, &msg, &transfer);
            } else {
                inner.held.push((msg, transfer));
            }
        }
        Box::pin(async move {
            let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
            ended
                .borrow_mut()
                .take()
                .unwrap_or_else(|| Err("the write ended with no reply".into()))
        })
    }

    /// The port, started if it is not running.
    pub fn port(&self) -> Result<JsValue, String> {
        if let Some(port) = &self.0.borrow().port {
            return Ok(port.clone());
        }
        let start = self.0.borrow().start.clone();
        let port = start()?;
        let me = self.clone();
        let on_message = Closure::<dyn FnMut(JsValue)>::new(move |data: JsValue| me.reply(&data));
        let me = self.clone();
        let on_error = Closure::<dyn FnMut(String)>::new(move |reason: String| {
            log::warn!("writer: the write worker stopped: {reason}");
            me.stopped(&reason);
        });
        port_listen(
            &port,
            on_message.as_ref().unchecked_ref(),
            on_error.as_ref().unchecked_ref(),
        );
        let mut inner = self.0.borrow_mut();
        inner._closures = vec![Box::new(on_message), Box::new(on_error)];
        inner.port = Some(port.clone());
        inner.ready = false;
        Ok(port)
    }

    fn reply(&self, data: &JsValue) {
        let reply = match WriteReply::from_js(data) {
            Ok(reply) => reply,
            // A reply that cannot be read cannot be matched to a request,
            // so each request ends, and the next write starts a new port.
            Err(bad) => {
                log::warn!("writer: {}", bad.text);
                self.stopped(&format!("a reply could not be read: {}", bad.text));
                return;
            }
        };
        match reply {
            WriteReply::Ready => {
                let mut inner = self.0.borrow_mut();
                inner.ready = true;
                let held = std::mem::take(&mut inner.held);
                if let Some(port) = &inner.port {
                    for (msg, transfer) in held {
                        port_post(port, &msg, &transfer);
                    }
                }
            }
            WriteReply::Done { outcome, bytes } => {
                let waiting = self.0.borrow_mut().waiting.pop_front();
                match waiting {
                    Some(w) => {
                        *w.ended.borrow_mut() = Some(Ok((outcome, bytes)));
                        let _ = w.resolve.call0(&JsValue::NULL);
                    }
                    None => log::warn!("writer: a reply came with no write posted"),
                }
            }
        }
    }

    /// Ends the port, fails each request that waits, and starts a new port
    /// at the next write.
    fn stopped(&self, reason: &str) {
        let waiting = {
            let mut inner = self.0.borrow_mut();
            if let Some(port) = inner.port.take() {
                port_end(&port);
            }
            inner.ready = false;
            inner.held.clear();
            std::mem::take(&mut inner.waiting)
        };
        for w in waiting {
            *w.ended.borrow_mut() = Some(Err(format!("the write worker stopped: {reason}")));
            let _ = w.resolve.call0(&JsValue::NULL);
        }
    }
}

thread_local! {
    /// The client of the page's writes, made at the first.
    static CLIENT: RefCell<Option<WriteClient>> = const { RefCell::new(None) };
}

/// The client of the page's writes: of the write worker, or of the fake
/// under test.
pub fn client() -> WriteClient {
    CLIENT.with_borrow_mut(|client| client.get_or_insert_with(default_client).clone())
}

#[cfg(not(any(test, feature = "fake-writer")))]
fn default_client() -> WriteClient {
    WriteClient::of_worker(super::LOADER)
}

#[cfg(any(test, feature = "fake-writer"))]
fn default_client() -> WriteClient {
    WriteClient::new(|| Ok(super::fake::port()))
}

/// Puts the default client back when dropped.
#[cfg(any(test, feature = "fake-writer"))]
pub struct ClientGuard(());

#[cfg(any(test, feature = "fake-writer"))]
impl Drop for ClientGuard {
    fn drop(&mut self) {
        if let Some(client) = CLIENT.with_borrow_mut(Option::take) {
            client.stopped("the test ended");
        }
    }
}

/// Makes `client` the client of the page's writes, until the guard drops;
/// then the next write makes the default one again.
#[cfg(any(test, feature = "fake-writer"))]
pub fn use_client(client: WriteClient) -> ClientGuard {
    CLIENT.with_borrow_mut(|slot| *slot = Some(client));
    ClientGuard(())
}

#[cfg(test)]
mod tests;
