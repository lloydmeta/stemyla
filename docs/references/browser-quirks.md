# Browser quirks

Behaviour of browsers and libraries that the code works around.  Each entry says where it was seen, when, and what in the code depends on it.

## Safari writes a view's whole buffer to browser storage

Safari 27.0 on macOS 26.7.1 (checked 2026-10-19): `FileSystemWritableFileStream.write(new Uint8Array(16).subarray(4, 8))` writes 16 bytes, not 4; Chrome writes 4.  No matching WebKit bug was found.  A sliced model write once grew a 9 MB write to 18 MB, silently corrupting the file.

* The code: the JavaScript `whole()` helpers in `worker/src/web.rs`, and the one that `st_write` calls in `app/src/store.rs`, copy a partial view before writing; a `Uint8Array::from` made in Rust is already whole.  The tests spy on `write`, since Chrome writes a view correctly.
* A `&[u8]` that Rust gives to JavaScript is a view into the wasm memory.  On 2026-11-11, in Safari 27.2, the test helper `plant_at` of `decoder/src/web/tests.rs` wrote such a view.  Each planted file then held the whole wasm memory, and 6 tests decoded nothing.  A test helper that plants a file writes a copy too.
* Rule: MEM-5.

## Safari does not count WebGPU memory when collecting garbage

Safari's garbage collector is not told how much memory WebGPU objects hold ([WebKit bug 283321](https://bugs.webkit.org/show_bug.cgi?id=283321)), and wgpu's web backend does nothing when a buffer is dropped, so a dropped buffer can stay allocated long enough to push the tab over its memory limit.

* wgpu 30.0.1's web backend does nothing when a buffer, a query set or a texture is dropped ([source](https://docs.rs/crate/wgpu/30.0.1/source/src/backend/webgpu.rs), checked 2026-10-21).
* The code: `GpuBuffer` in `roformer-gpu/src/lib.rs` calls `destroy()` when dropped.
* Rule: MEM-3.

## cpal's Web Audio host never frees a stream's callback

cpal 0.18's Web Audio host keeps a stream's callback in a reference cycle, so anything the callback holds lives as long as the page.

* The code: the output is opened once and keeps only the player slot (`engine/src/slot.rs`); the slot is emptied before a stream is dropped.
* Rule: RT-1.

## Safari reads a stored file as it is when read

Safari reads an OPFS file's contents at the time of each read, not as it was when the file was opened, so a file replaced while being read gives mixed contents.

* The code: the model is built from the bytes that were hashed (`docs/ARCHITECTURE.md`, memory on phones).
* Seen in Safari 27 on 2026-10-26 (review round 3 of M4): a `File` from `getFile()` keeps its old `size`, but reads the bytes on disk now.  A file written again shorter in the same second reads past its new end as zeros, with no error.  Its `lastModified` gives the value on disk now, in whole seconds.  A read after a write in a later second fails with NotFoundError.  Chrome 155 refuses any read after a write (NotReadableError), and gives `lastModified` in milliseconds.
* The code: the worker keeps the size and `lastModified` of the mix file at the open, and compares them with a new `getFile()` after the last chunk (`worker/src/web/mix.rs`).  A write of the same size in the same second stays unseen in Safari (TD-55).

## Safari keeps audio suspended while macOS reports the user idle

Safari leaves every `AudioContext` suspended (time 0) while macOS reports the user idle (`pmset -g assertions` shows `UserIsActive 0`), so audio checks fail on any commit, `main` included (checked 2026-10-19).  See `local-browser-testing.md`.

## Safari's AAC encoder loses the start and gives a whole `ES_Descriptor`

Seen in Safari 27.2 on macOS, on an Apple M2 Pro, on 2026-11-10, with WebCodecs `AudioEncoder` (`mp4a.40.2`, 44,100 Hz, 2 channels, 256,000 bit/s).  The spike is in `plans/completed/m4a-m4.md` ("Facts").  Chrome 157 on the same Mac shows none of the three.

* Safari encodes the first 1024 input frames as silence.  Mediabunny reports the same bug, and pads the input by 1024 frames ([Mediabunny, "Project sync"](https://mediabunny.dev/blog/project-sync)).  The code: the worker puts 2048 frames of silence before the source (`aac::LEAD_IN` in `worker/src/web/aac.rs`).  The `elst` entry and `iTunSMPB` skip them with the encoder's delay.
* The `decoderConfig.description` of Safari's first output is a whole `ES_Descriptor` with 4-byte descriptor sizes, not the bare AudioSpecificConfig that Chrome gives.  Safari's own `AudioDecoder` refuses that description, and takes the bare config.  The code: `m4a::audio_specific_config` takes the config out of either form.  The worker's probe decodes with the bare config, and the file's `esds` holds it.
* With the default `bitrateMode` (`variable`), Safari gave 72 kbit/s for a target of 256 kbit/s.  With `constant`, it gave 259 kbit/s.  The code: `stem_protocol::AAC_ENCODER` asks for `constant`, so the label "256 kbps" holds.
* Rule: none.  `a_real_aac_decodes_to_the_source` checks the start, the lag and the error in each browser on a Mac.

## Safari gives no `GPUAdapterInfo.subgroupMinSize`

Seen in Safari 27.2 on macOS, on an Apple M2 Pro, on 2026-11-11.  `GPUAdapterInfo.subgroupMinSize` is `undefined`, and wgpu reads it as a number.

* The code: the debug glue of `wasm-bindgen-test-runner` asserts the number, so two GPU tests fail in a Safari `web-test` (TD-93).  With `WASM_BINDGEN_NO_DEBUG=1`, both pass.  The f16 test then needs the text of the next entry.
* Rule: VERIFY-1.

## WebKit words a WGSL compile error in its own way

Seen in Safari 27.2 on macOS, on an Apple M2 Pro, on 2026-11-11.  A planted WGSL error gives "Expected a Identifier, but got a {".  Chrome's Tint names a parse error.

* The code: `an_f16_variant_the_browser_rejects_keeps_f32_weights` takes the text of each browser, and still checks that the text names the compile failure (TD-93).
* Rule: VERIFY-1.
