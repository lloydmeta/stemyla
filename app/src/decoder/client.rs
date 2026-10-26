//! The page's client of the decode worker (D6, D9 of
//! `plans/active/stream-from-storage-m5.md`).
//!
//! It starts the worker at the first decode, and again after the worker
//! stopped.  It holds each post until the worker says `decode-alive`, as
//! `separator.rs` holds `Init`, so a decode posted at startup is not lost.
//! The worker ends each decode with one reply, in the order posted, so each
//! reply belongs to the oldest decode not yet ended.  An `error` event
//! terminates the worker and ends every decode posted.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use stem_protocol::decode::{DecodeReply, DecodeRequest, kind};
use wasm_bindgen::prelude::*;
use web_sys::{MessageEvent, Worker, WorkerOptions, WorkerType};

use crate::store::DecodeGuard;

/// How a decode ended.
pub enum Ended {
    /// The worker's reply: `decoded`, `undecodable` or `decode-unstored`,
    /// with the decode's guard, which the page holds until the end of D7.
    Reply(DecodeReply, DecodeGuard),
    /// The worker could not start, or stopped, or a reset ended it.
    Stopped(String),
}

/// A decode posted and not yet ended.
struct Waiting {
    ended: Rc<RefCell<Option<Ended>>>,
    resolve: js_sys::Function,
    guard: DecodeGuard,
}

struct Inner {
    url: String,
    worker: Option<Worker>,
    alive: bool,
    /// Posts made before `decode-alive`.
    held: Vec<JsValue>,
    waiting: VecDeque<Waiting>,
    /// Set by a reset: no decode is posted again.
    closed: bool,
    _closures: Vec<Box<dyn std::any::Any>>,
}

/// The decode worker, shared by the page's opens and restores.
#[derive(Clone)]
pub struct DecodeClient(Rc<RefCell<Inner>>);

impl DecodeClient {
    /// A client of the worker at `url` (the loader shim), which starts at
    /// the first decode.
    pub fn new(url: &str) -> Self {
        Self(Rc::new(RefCell::new(Inner {
            url: url.into(),
            worker: None,
            alive: false,
            held: Vec::new(),
            waiting: VecDeque::new(),
            closed: false,
            _closures: Vec::new(),
        })))
    }

    /// Posts `request`, with the guard of its session, and gives how it
    /// ended.
    pub fn decode(
        &self,
        request: &DecodeRequest,
        guard: DecodeGuard,
    ) -> Pin<Box<dyn Future<Output = Ended>>> {
        if self.0.borrow().closed || crate::store::is_closed() {
            return Box::pin(std::future::ready(Ended::Stopped(
                "browser storage is being reset".into(),
            )));
        }
        let worker = match self.worker() {
            Ok(worker) => worker,
            Err(reason) => return Box::pin(std::future::ready(Ended::Stopped(reason))),
        };
        let mut resolve = None;
        let promise = js_sys::Promise::new(&mut |done, _| resolve = Some(done));
        let ended = Rc::new(RefCell::new(None));
        let (msg, _) = request.to_js();
        {
            let mut inner = self.0.borrow_mut();
            inner.waiting.push_back(Waiting {
                ended: ended.clone(),
                resolve: resolve.expect("a promise calls its executor at once"),
                guard,
            });
            if inner.alive {
                let _ = worker.post_message(&msg);
            } else {
                inner.held.push(msg.into());
            }
        }
        Box::pin(async move {
            let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
            ended
                .borrow_mut()
                .take()
                .unwrap_or_else(|| Ended::Stopped("the decode ended with no reply".into()))
        })
    }

    /// Ends the worker at once and every decode posted, for a reset: no
    /// decode is posted again.
    pub fn stop(&self) {
        self.0.borrow_mut().closed = true;
        self.stopped("stopped for a reset");
    }

    /// The worker, started if it is not running.
    pub fn worker(&self) -> Result<Worker, String> {
        if let Some(worker) = &self.0.borrow().worker {
            return Ok(worker.clone());
        }
        let opts = WorkerOptions::new();
        opts.set_type(WorkerType::Module);
        let url = self.0.borrow().url.clone();
        let worker = Worker::new_with_options(&url, &opts)
            .map_err(|e| format!("the decode worker could not start: {e:?}"))?;
        let me = self.clone();
        let onmessage = Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
            me.reply(&e.data());
        });
        let me = self.clone();
        let onerror = Closure::<dyn FnMut(web_sys::Event)>::new(move |e: web_sys::Event| {
            let reason = e
                .dyn_ref::<web_sys::ErrorEvent>()
                .map(|e| e.message())
                .filter(|m| !m.is_empty())
                .unwrap_or_else(|| "its script did not load".into());
            log::warn!("decoder: the decode worker stopped: {reason}");
            me.stopped(&reason);
        });
        worker.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
        worker.set_onerror(Some(onerror.as_ref().unchecked_ref()));
        let mut inner = self.0.borrow_mut();
        inner._closures = vec![Box::new(onmessage), Box::new(onerror)];
        inner.worker = Some(worker.clone());
        inner.alive = false;
        Ok(worker)
    }

    fn reply(&self, data: &JsValue) {
        let reply = match DecodeReply::from_js(data) {
            Ok(reply) => reply,
            // A bad `decode-alive` or log line ends no decode.
            Err(bad) if [kind::ALIVE, kind::LOG].contains(&bad.kind.as_str()) => {
                log::warn!("decoder: {}", bad.text);
                return;
            }
            // Any other reply ends a decode, but the client cannot tell
            // which, so every decode ends, and the worker starts again.
            Err(bad) => {
                log::warn!("decoder: {}", bad.text);
                self.stopped(&format!("a reply could not be read: {}", bad.text));
                return;
            }
        };
        match reply {
            DecodeReply::Alive => {
                let mut inner = self.0.borrow_mut();
                inner.alive = true;
                let held = std::mem::take(&mut inner.held);
                if let Some(worker) = &inner.worker {
                    for msg in held {
                        let _ = worker.post_message(&msg);
                    }
                }
            }
            DecodeReply::Log(text) => log::info!("{text}"),
            end => {
                let waiting = self.0.borrow_mut().waiting.pop_front();
                match waiting {
                    Some(w) => {
                        *w.ended.borrow_mut() = Some(Ended::Reply(end, w.guard));
                        let _ = w.resolve.call0(&JsValue::NULL);
                    }
                    None => log::warn!("decoder: a reply came with no decode posted"),
                }
            }
        }
    }

    /// Terminates the worker, ends every decode posted, and starts a new
    /// worker at the next decode.
    fn stopped(&self, reason: &str) {
        let waiting = {
            let mut inner = self.0.borrow_mut();
            if let Some(worker) = inner.worker.take() {
                worker.terminate();
            }
            inner.alive = false;
            inner.held.clear();
            std::mem::take(&mut inner.waiting)
        };
        for w in waiting {
            drop(w.guard);
            *w.ended.borrow_mut() = Some(Ended::Stopped(reason.into()));
            let _ = w.resolve.call0(&JsValue::NULL);
        }
    }
}

/// Browser tests of the client, with a worker that has no script: a spy
/// keeps what the page posts, and the test delivers the replies.
#[cfg(test)]
mod tests {
    use super::*;
    use stem_protocol::decode::Source;
    use wasm_bindgen_test::*;

    #[wasm_bindgen(inline_js = r#"
export function client_spy_on_post(worker) {
  const posts = [];
  worker.postMessage = (data, transfer) => posts.push(data);
  return posts;
}
export function client_deliver(worker, data) {
  worker.dispatchEvent(new MessageEvent("message", { data }));
}
"#)]
    extern "C" {
        fn client_spy_on_post(worker: &Worker) -> js_sys::Array;
        fn client_deliver(worker: &Worker, data: &JsValue);
    }

    fn request(id: &str) -> DecodeRequest {
        DecodeRequest {
            session: format!("sessions/{id}"),
            sources: vec![Source {
                file: "src/0.wav".into(),
                index: 0,
                ext: "wav".into(),
                in_mix: true,
            }],
            mix_frames: None,
        }
    }

    /// A client whose worker has no script, with a spy on its posts.  The
    /// worker's own `error` (no script) is cleared; a test sends its own.
    fn client() -> (DecodeClient, Worker, js_sys::Array) {
        let client = DecodeClient::new("./no-such-decode-worker.js");
        let worker = client.worker().expect("a worker");
        worker.set_onerror(None);
        let posts = client_spy_on_post(&worker);
        (client, worker, posts)
    }

    fn sent(reply: DecodeReply) -> JsValue {
        reply.to_js().0.into()
    }

    /// D6: a decode posted before `decode-alive` goes out after it, so a
    /// startup restore is not lost.
    #[wasm_bindgen_test]
    async fn a_decode_posted_before_alive_goes_out_after_it() {
        let (client, worker, posts) = client();
        let ended = client.decode(&request("1"), DecodeGuard::new("1"));
        let before = posts.length();
        client_deliver(&worker, &sent(DecodeReply::Alive));
        let after = posts.length();
        let first = posts.get(0);
        client_deliver(
            &worker,
            &sent(DecodeReply::Decoded {
                frames: 4,
                new_mix: true,
                channels: vec![2],
            }),
        );
        let ended = ended.await;
        client.stop();
        assert_eq!((before, after), (0, 1), "held until decode-alive");
        assert_eq!(
            DecodeRequest::from_js(&first).map(|r| r.session),
            Ok("sessions/1".into())
        );
        assert!(
            matches!(
                ended,
                Ended::Reply(DecodeReply::Decoded { frames: 4, .. }, _)
            ),
            "the reply ends the decode"
        );
    }

    /// D9: each reply ends the oldest decode, and the guard of each decode
    /// holds its session until it ends.
    #[wasm_bindgen_test]
    async fn each_reply_ends_the_oldest_decode_and_its_guard_goes_with_it() {
        let (client, worker, posts) = client();
        client_deliver(&worker, &sent(DecodeReply::Alive));
        let first = client.decode(&request("21"), DecodeGuard::new("21"));
        let second = client.decode(&request("22"), DecodeGuard::new("22"));
        assert_eq!(posts.length(), 2, "posted at once after decode-alive");
        assert!(crate::store::decoding("21") && crate::store::decoding("22"));
        client_deliver(
            &worker,
            &sent(DecodeReply::Undecodable { text: "a".into() }),
        );
        client_deliver(&worker, &sent(DecodeReply::Unstored { text: "b".into() }));
        let first = first.await;
        let second = second.await;
        assert!(
            matches!(&first, Ended::Reply(DecodeReply::Undecodable { text }, _) if text == "a")
        );
        assert!(matches!(&second, Ended::Reply(DecodeReply::Unstored { text }, _) if text == "b"));
        assert!(
            crate::store::decoding("21"),
            "the page holds the guard to D7"
        );
        drop((first, second));
        assert!(!crate::store::decoding("21") && !crate::store::decoding("22"));
        client.stop();
    }

    /// PROTO-1, D9: a reply that does not decode cannot be matched to a
    /// decode, so the client stops: every waiting decode ends, and no later
    /// reply ends the wrong one.
    #[wasm_bindgen_test]
    async fn a_reply_that_does_not_decode_ends_every_decode() {
        let (client, worker, _posts) = client();
        client_deliver(&worker, &sent(DecodeReply::Alive));
        let first = client.decode(&request("41"), DecodeGuard::new("41"));
        let second = client.decode(&request("42"), DecodeGuard::new("42"));
        let bad = js_sys::Object::new();
        js_sys::Reflect::set(&bad, &"type".into(), &"decoded".into()).unwrap();
        client_deliver(&worker, &bad);
        client_deliver(
            &worker,
            &sent(DecodeReply::Undecodable {
                text: "a later reply".into(),
            }),
        );
        let (first, second) = (first.await, second.await);
        client.stop();
        for (n, ended) in [first, second].into_iter().enumerate() {
            assert!(
                matches!(&ended, Ended::Stopped(reason) if reason.contains("could not be read")),
                "decode {n} ends Stopped"
            );
        }
        assert!(!crate::store::decoding("41") && !crate::store::decoding("42"));
    }

    /// Finding 2-2 of review round 2: a `decode-log` with no `text` ends no
    /// decode, and the worker stays; the next reply ends the decode.
    #[wasm_bindgen_test]
    async fn a_bad_log_line_ends_no_decode() {
        let (client, worker, _posts) = client();
        client_deliver(&worker, &sent(DecodeReply::Alive));
        let ended = client.decode(&request("43"), DecodeGuard::new("43"));
        let bad = js_sys::Object::new();
        js_sys::Reflect::set(&bad, &"type".into(), &kind::LOG.into()).unwrap();
        client_deliver(&worker, &bad);
        client_deliver(
            &worker,
            &sent(DecodeReply::Undecodable {
                text: "acme.wav is no audio".into(),
            }),
        );
        let ended = ended.await;
        let same = client
            .worker()
            .is_ok_and(|now| js_sys::Object::is(&now, &worker));
        client.stop();
        assert!(
            matches!(&ended, Ended::Reply(DecodeReply::Undecodable { text }, _) if text == "acme.wav is no audio"),
            "the decode ends with the reply, not Stopped"
        );
        assert!(same, "the worker is not terminated");
    }

    /// D9: the worker's `error` event (here: its script does not load)
    /// ends every decode posted and drops its guard, and the next decode
    /// starts a new worker.  After a reset, no decode is posted.
    #[wasm_bindgen_test]
    async fn an_error_ends_the_decodes_and_the_next_one_starts_a_new_worker() {
        let client = DecodeClient::new("./no-such-decode-worker.js");
        let ended = client.decode(&request("31"), DecodeGuard::new("31"));
        let first = client.worker().expect("a worker");
        let ended = ended.await;
        let reason = match ended {
            Ended::Stopped(reason) => reason,
            Ended::Reply(reply, _) => panic!("a reply with no worker: {reply:?}"),
        };
        assert!(!reason.is_empty(), "the worker's error ends the decode");
        assert!(!crate::store::decoding("31"), "the guard is dropped");
        let again = client.worker().expect("a new worker");
        again.set_onerror(None);
        assert!(!js_sys::Object::is(&again, &first), "a new worker");
        client.stop();
        let refused = client.decode(&request("32"), DecodeGuard::new("32")).await;
        assert!(matches!(refused, Ended::Stopped(reason) if reason.contains("reset")));
        assert!(!crate::store::decoding("32"));
    }
}
