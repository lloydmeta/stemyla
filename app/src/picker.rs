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

/// The most bytes that one read of a file asks JavaScript for: 1 MiB.
/// So JavaScript holds one block of a file at a time (MEM-4).
pub const READ_BLOCK: usize = 1 << 20;

/// The bytes of a chosen file, read in blocks ([`read_blob`]).
pub async fn read(file: &web_sys::File) -> Result<Vec<u8>, String> {
    read_blob(file)
        .await
        .map_err(|e| format!("could not read {}: {e}", file.name()))
}

/// The bytes of `blob`, in one `Vec` sized once from its size.  Each block
/// of at most [`READ_BLOCK`] bytes is read with `Blob.slice` and copied
/// into its part of the `Vec`.  So JavaScript holds one block at a time,
/// not a second copy of the file (finding 2-3 of review round 2 of the
/// catch-up plan, MEM-4).  The decoder reads wasm memory, so the file is
/// copied in once.
pub async fn read_blob(blob: &web_sys::Blob) -> Result<Vec<u8>, String> {
    let size = blob.size() as usize;
    let mut bytes = vec![0; size];
    let mut start = 0;
    while start < size {
        let end = size.min(start + READ_BLOCK);
        let block = blob
            .slice_with_f64_and_f64(start as f64, end as f64)
            .map_err(|e| format!("{e:?}"))?;
        let buffer = wasm_bindgen_futures::JsFuture::from(block.array_buffer())
            .await
            .map_err(|e| format!("{e:?}"))?;
        let view = js_sys::Uint8Array::new(&buffer);
        if view.length() as usize != end - start {
            return Err(format!(
                "a read of bytes {start} to {end} gave {} bytes",
                view.length()
            ));
        }
        view.copy_to(&mut bytes[start..end]);
        start = end;
    }
    Ok(bytes)
}

/// The first `most` bytes of a chosen file, or all of a shorter one: one
/// read of a slice, never the whole file (D3 of the catch-up plan).
pub async fn read_head(file: &web_sys::File, most: usize) -> Result<Vec<u8>, String> {
    let end = file.size().min(most as f64);
    let head = file
        .slice_with_f64_and_f64(0.0, end)
        .map_err(|e| format!("could not read {}: {e:?}", file.name()))?;
    let buffer = wasm_bindgen_futures::JsFuture::from(head.array_buffer())
        .await
        .map_err(|e| format!("could not read {}: {e:?}", file.name()))?;
    #[expect(
        clippy::disallowed_methods,
        reason = "MEM-1: the head of a file, at most `engine::decode::HEAD_BYTES`, is copied in to read its length"
    )]
    let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
    Ok(bytes)
}
