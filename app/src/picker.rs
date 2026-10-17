//! The browser's file picker, for touch devices where dropping files on the
//! page is awkward or impossible.  Web only.
//!
//! Safari opens the picker only from inside the user's click (WebKit bug
//! 234566), so call this from a button's `clicked()`: eframe runs that frame
//! inside the pointer event handler.

use wasm_bindgen::prelude::*;
use web_sys::HtmlInputElement;

/// Opens the picker; `on_pick` gets the chosen files (none if the picker
/// returned nothing).  `accept` lists extensions such as `".mp3,.wav"`.
pub fn pick_files(
    accept: &str,
    multiple: bool,
    on_pick: impl FnOnce(Vec<web_sys::File>) + 'static,
) {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let Ok(input) = doc
        .create_element("input")
        .map(|e| e.unchecked_into::<HtmlInputElement>())
    else {
        return;
    };
    input.set_type("file");
    input.set_accept(accept);
    input.set_multiple(multiple);
    let picked = input.clone();
    // Freed once it runs; a cancelled picker leaves this one small closure.
    let onchange = Closure::once_into_js(move || {
        let files = picked
            .files()
            .map(|list| (0..list.length()).filter_map(|i| list.get(i)).collect())
            .unwrap_or_default();
        on_pick(files);
    });
    input.set_onchange(Some(onchange.unchecked_ref()));
    input.click();
}

/// The bytes of a chosen file.
pub async fn read(file: &web_sys::File) -> Result<Vec<u8>, String> {
    let buffer = wasm_bindgen_futures::JsFuture::from(file.array_buffer())
        .await
        .map_err(|e| format!("could not read {}: {e:?}", file.name()))?;
    Ok(js_sys::Uint8Array::new(&buffer).to_vec())
}
