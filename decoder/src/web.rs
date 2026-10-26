//! The browser glue of the decode worker.

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{DedicatedWorkerGlobalScope, MessageEvent};

/// The text of the log line for a message that gets no answer yet.
const NO_ANSWER: &str = "decoder: no request is answered yet (M5, step 3)";

/// Installs the message handler, which logs each message to the console.
pub fn start() {
    console_error_panic_hook::set_once();
    let handler = Closure::<dyn FnMut(MessageEvent)>::new(|_: MessageEvent| {
        web_sys::console::log_1(&NO_ANSWER.into());
    });
    let scope: DedicatedWorkerGlobalScope = js_sys::global().unchecked_into();
    scope.set_onmessage(Some(handler.as_ref().unchecked_ref()));
    handler.forget();
}

#[cfg(test)]
mod tests {
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_dedicated_worker);

    /// MEM-1: proves each path in this crate's `clippy.toml` (see the
    /// `#[expect]`s).
    #[wasm_bindgen_test]
    fn mem1_lint_is_on_for_js_arrays() {
        #[expect(
            clippy::disallowed_methods,
            reason = "MEM-1 canary: clippy fails here if this path is off"
        )]
        let slice = [1.0_f32].to_vec();
        #[expect(
            clippy::disallowed_methods,
            reason = "MEM-1 canary: clippy fails here if this path is off"
        )]
        let bytes = js_sys::Uint8Array::from(&[2_u8][..]).to_vec();
        #[expect(
            clippy::disallowed_methods,
            reason = "MEM-1 canary: clippy fails here if this path is off"
        )]
        let floats = js_sys::Float32Array::from(&[3.0_f32][..]).to_vec();
        assert_eq!((slice, bytes, floats), (vec![1.0], vec![2], vec![3.0]));
    }
}
