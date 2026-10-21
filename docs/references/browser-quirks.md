# Browser quirks

Behaviour of browsers and libraries that the code works around.  Each entry says where it was seen, when, and what in the code depends on it.

## Safari writes a view's whole buffer to browser storage

Safari 27.0 on macOS 26.7.1 (checked 2026-10-19): `FileSystemWritableFileStream.write(new Uint8Array(16).subarray(4, 8))` writes 16 bytes, not 4; Chrome writes 4.  No matching WebKit bug was found.  A sliced model write once grew a 9 MB write to 18 MB, silently corrupting the file.

* The code: the worker's JavaScript `whole()` helper in `worker/src/web.rs` copies a partial view before writing; a `Uint8Array::from` made in Rust is already whole.
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

## Safari keeps audio suspended while macOS reports the user idle

Safari leaves every `AudioContext` suspended (time 0) while macOS reports the user idle (`pmset -g assertions` shows `UserIsActive 0`), so audio checks fail on any commit, `main` included (checked 2026-10-19).  See `local-browser-testing.md`.
