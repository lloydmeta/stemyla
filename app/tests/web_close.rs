//! `app::store::close` in a page of its own: it refuses writes until the
//! page reloads, which would break every other storage test.
#![cfg(target_arch = "wasm32")]

use app::store;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
async fn after_close_every_write_fails_at_once() {
    store::close().await;
    let saved = store::save_sources(&[("acme.wav".into(), vec![1, 2, 3])], 8_000).await;
    assert_eq!(
        saved.expect_err("refused").to_string(),
        "browser storage is being reset"
    );
    assert_eq!(store::current().await, None, "nothing was written");
}
