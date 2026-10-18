//! `app::store::close` in a page of its own: it refuses writes until the
//! page reloads, which would break every other storage test.  One test,
//! so its steps run in order.
#![cfg(target_arch = "wasm32")]

use app::store;
use engine::Stem;
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen(inline_js = r#"
async function walk(path) {
  let d = await navigator.storage.getDirectory();
  const parts = path.split("/");
  const last = parts.pop();
  for (const p of parts) d = await d.getDirectoryHandle(p);
  return [d, last];
}
export async function exists(path) {
  try {
    const [d, last] = await walk(path);
    for await (const name of d.keys()) if (name === last) return true;
    return false;
  } catch (e) { return false; }
}
export async function remove(path) {
  try {
    const [d, last] = await walk(path);
    await d.removeEntry(last, { recursive: true });
    return true;
  } catch (e) { return false; }
}
export function sleep(ms) { return new Promise((r) => setTimeout(r, ms)); }
"#)]
extern "C" {
    async fn exists(path: &str) -> JsValue;
    /// Removes `path` and everything in it; false if the browser refused
    /// (a file in it open for writing).
    async fn remove(path: &str) -> JsValue;
    async fn sleep(ms: u32);
}

async fn has(path: &str) -> bool {
    exists(path).await.as_bool() == Some(true)
}

const RATE: u32 = 8_000;

#[wasm_bindgen_test]
async fn close_waits_for_the_write_in_progress_then_refuses_every_write() {
    let id = store::save_sources(
        &[(
            "acme.wav".into(),
            engine::wav::encode_pcm16(&[0.1; 800], &[0.1; 800], RATE),
        )],
        RATE,
    )
    .await
    .expect("saved");
    // Large stems, so a write is in progress when the first file appears.
    let stems: Vec<Stem> = (0..6)
        .map(|i| Stem {
            name: format!("s{i}"),
            left: vec![0.1; 1_000_000].into(),
            right: vec![0.1; 1_000_000].into(),
        })
        .collect();
    let saved = Rc::new(RefCell::new(None));
    let (sid, out) = (id.clone(), saved.clone());
    wasm_bindgen_futures::spawn_local(async move {
        let r = store::save_stems(&sid, RATE, &stems).await;
        *out.borrow_mut() = Some(r.map_err(|e| e.to_string()));
    });
    while !has(&format!("sessions/{id}/stems/s0.wav")).await {
        sleep(1).await;
    }

    store::close().await;
    assert!(
        remove("sessions").await.as_bool() == Some(true),
        "once close returns, no file is open for writing"
    );
    while saved.borrow().is_none() {
        sleep(10).await;
    }
    assert_eq!(
        *saved.borrow(),
        Some(Err("browser storage is being reset".to_string())),
        "the save stops at its next write"
    );
    assert!(!has("sessions").await, "nothing was written after close");

    let refused = store::save_sources(&[("acme.wav".into(), vec![1, 2, 3])], RATE).await;
    assert_eq!(
        refused.expect_err("refused").to_string(),
        "browser storage is being reset"
    );
    assert_eq!(store::current().await, None, "nothing was written");
}
