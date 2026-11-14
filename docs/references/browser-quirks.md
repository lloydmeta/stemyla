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

* The code: the debug glue of `wasm-bindgen-test-runner` asserts the number, so two GPU tests fail in Safari.  So `web-test` sets `WASM_BINDGEN_NO_DEBUG=1` for the crate run of `stem-worker` in Safari (TD-93), and both pass.  The f16 test then needs the text of the next entry.  The release build has no such assert: `e2e selftest --browser safari` passed on 2026-11-11.
* Rule: VERIFY-1.

## WebKit words a WGSL compile error in its own way

Seen in Safari 27.2 on macOS, on an Apple M2 Pro, on 2026-11-11.  A planted WGSL error gives "Expected a Identifier, but got a {".  Chrome's Tint names a parse error.

* The code: `an_f16_variant_the_browser_rejects_keeps_f32_weights` takes the text of each browser, and still checks that the text names the compile failure (TD-93).
* Rule: VERIFY-1.

## Safari rejects an aborted fetch with its own error

Seen in Safari 27.2 on macOS, on an Apple M2 Pro, on 2026-11-11.  `AbortController.abort(reason)` rejects the `fetch` and the reads of its body with "AbortError: Fetch is aborted", not with `reason`.  Chrome gives `reason`.  This is [WebKit bug 246069](https://bugs.webkit.org/show_bug.cgi?id=246069) (NEW).

* The code: `download_to_opfs` in `worker/src/web.rs` gives the signal's reason when the signal is aborted.  So a stalled download says "no data from the server for 60 s" in each browser (TD-96).
* Rule: none.  `e2e storage` checks the text in each browser.

## Safari gives the size of a stored `File` at its first read

Seen in Safari 27.2 on macOS, on an Apple M2 Pro, on 2026-11-11.  A `File` from `getFile()` of an OPFS file gives its `size` at the first read of `size`, not at `getFile()`.  If the file is removed before that first read, `size` can be 0.  In 3 of 4 runs of the stem worker's tests, it was 0, and the test failed.  When the test read `size` before the removal, the test passed in 5 of 5 runs.  The read of the bytes of a removed file fails with NotFoundError.

* The code: the worker reads the size of the stored model just after `getFile()`, before any other message can remove the file (`init` in `worker/src/web.rs`).  `a_stored_file_removed_after_it_was_opened_is_a_read_error` reads it there too.
* Rule: VERIFY-1.

## Safari sets workgroup memory to zero with one thread

Seen in Safari 27.2 on macOS 27.2.1, on an Apple M2 Pro, on 2026-11-13.  The source is Steps 1, 3, 4, 4b and 5 of `plans/completed/gpu-speed-m3.md`.  WebKit's WGSL compiler sets each `var<workgroup>` to zero at the start of a kernel.  Thread 0 alone does it, byte by byte, then all threads wait at a barrier.  Chrome 157 spreads the stores over the threads of the workgroup.

* The code in WebKit (`main` at `5447ffbd5817eb8a1de9c0540bb8a6fe37db555d`, read on 2026-11-13): `RewriteGlobalVariables::initializeVariables` puts `if (local_invocation_index == 0) { ... }` at the kernel's start.  A `workgroupBarrier()` follows it ([GlobalVariableRewriter.cpp in WebKit's Source/WebGPU/WGSL, lines 2552 to 2594](https://github.com/WebKit/WebKit/blob/5447ffbd5817eb8a1de9c0540bb8a6fe37db555d/Source/WebGPU/WGSL/GlobalVariableRewriter.cpp#L2552-L2594)).  Inside it, a loop calls `__wgslZeroWorkgroupVar` on each element ([lines 2666 to 2786](https://github.com/WebKit/WebKit/blob/5447ffbd5817eb8a1de9c0540bb8a6fe37db555d/Source/WebGPU/WGSL/GlobalVariableRewriter.cpp#L2666-L2786)).  That function writes the element one byte at a time ([MetalFunctionWriter.cpp in WebKit's Source/WebGPU/WGSL/Metal, lines 1081 to 1095](https://github.com/WebKit/WebKit/blob/5447ffbd5817eb8a1de9c0540bb8a6fe37db555d/Source/WebGPU/WGSL/Metal/MetalFunctionWriter.cpp#L1081-L1095)).  No option to turn it off was found (2026-11-13).  Safari 27.2 ships from a release branch, which can differ from `main`.
* The cost: a kernel with 10 KiB of workgroup memory and almost no work took 408 us for 2,064 workgroups in Safari.  In Chrome it took 17.9 us.  Chrome's MSL for `v64`, timed through Metal with WebKit's zero-init put in, was 1.29 times slower at K 256.  With WebKit's clamps too, it was 1.49 times slower.
* The part of `v64`'s loss in Safari at K 256 that the fixed cost for each workgroup explains has four estimates:
  * 64 %: Chrome's MSL timed with WebKit's zero-init and clamps (Step 5.2).  At K 1,024 it is 30 %.
  * 55 %: Step 3's fit of the time for each workgroup.
  * about 34 %: `v64-g4` against `v64` in both browsers at K 256 (Step 4b).  `v64-g4` sets its memory to zero once for 4 tiles.  The estimate assumes that `v64-g4`'s own extra cost is the same in both browsers.  At K 1,024 it is not: Safari 1.34 to 1.35 times `v64`, Chrome 1.25 to 1.27.  At K 256, Safari's time with `v64-g4` did not go down (1.014 to 1.033 times `v64`).  The browser ratio fell because Chrome got about 1.20 times slower.
  * about 26 %: the `wg10k` control (Step 4).  It has almost no work, so it can give less than a real kernel.
* By WebKit's source, the cost grows with the bytes of workgroup memory and the number of workgroups.
* The cost is not linear in the bytes (M4 Step 4, 2026-11-13, Safari 27.2, Chrome 157).  Three controls with almost no work ran at `v64`'s grid of 2,064 workgroups.  Safari took 408 to 414 us with 10 KiB, 113 to 115 us with 6 KiB and 10.7 to 11.3 us with none.  Chrome took 17.8 to 18.2 us, 14.0 to 14.3 us and 6.3 us.  So in Safari, 6 KiB costs 0.257 of the cost of 10 KiB over none, in each of 4 readings.  A linear cost gives 0.6.  In Chrome it is 0.66 to 0.69.  The cause of the curve is not known.
* More work for each byte did not help in Safari.  `v64-g4` took 1.014 to 1.033 times `v64` at K 256, and `v128` took 1.30 to 1.59 times.
* The code here: `v64k8`, a calibration candidate in `TILES` since M4 of `plans/active/gpu-speed.md`.  It is `v64` with BK 8: 6 KiB of workgroup memory on the same grid, not 10 KiB.  In `?bench` on the M2 Pro, Safari took 0.94 to 0.95 times `v64`'s time on the large products with it.  Chrome took 1.31 to 1.32 times, so Chrome keeps `v64`.
* `r64`, with no workgroup memory, was slower than `v64` on the large products in both browsers: 1.20 to 1.25 times in Safari, 1.93 to 2.55 in Chrome.  It did not land.
* Rule: GPU-1 (the default limits stay).  The WebKit bug text for the maintainer is [webkit-bug-workgroup-zero-init.md](webkit-bug-workgroup-zero-init.md).

## Safari keeps the swap file of each OPFS write stream that closes

Seen in Safari 27.2 on macOS 27.2.1, on an Apple M2 Pro, on 2026-11-14 (Step 3 of `plans/active/gpu-speed-m51.md`).  Each `createWritable()` makes a `FileSystemWritableStream*` file in `$DARWIN_USER_TEMP_DIR/com.apple.WebKit.Networking+com.apple.Safari/`, where `DARWIN_USER_TEMP_DIR` is the folder of `getconf DARWIN_USER_TEMP_DIR`.  A `close()` keeps it, with the bytes written.  An `abort()` removes it.  The files stayed while Safari ran, and after Safari quit (35,841 files, 176 GB, on 2026-11-14).  The draft bug is `references/webkit-bug-writable-swap-file.md`.

* The code in WebKit (`c526785513`, read on 2026-11-14): `FileSystemStorageHandle::closeWritable` deletes the swap file only for the reason `Aborted`.  Else it copies the swap file onto the stored file, and keeps it ([FileSystemStorageHandle.cpp in WebKit's Source/WebKit/NetworkProcess/storage, lines 297 to 321](https://github.com/WebKit/WebKit/blob/c526785513/Source/WebKit/NetworkProcess/storage/FileSystemStorageHandle.cpp#L297-L321)).
* The cost for each use of the app before M5.1: 349.5 MB for each model save.  Each song open cost the bytes of the dropped files, and each save the bytes of the save file.  After M5.1, only the page's small files leak: 327 bytes for a song open in `e2e smoke` (M5.1, Step 5d).
* The code: the stem worker writes the model and `matmul_kernels` through a sync access handle (`write_file` in `worker/src/web.rs`), which writes in place.  The decode worker and the stem writer use sync access handles too.  The page's main thread has no sync access handle.  So the page sends its song-sized files to the write worker (`writer/`, `app::writer`, decision 0032), which writes them through sync access handles.  Its small files (`meta`, `current`) stay on write streams, and still leak, about 0.3 KB for each song open (TD-103).
* Each WebDriver session also leaves one empty `BlobRegistryFiles-*` folder there.  `NetworkSessionCocoa` makes it for each network session that is not ephemeral ([NetworkSessionCocoa.mm in WebKit's Source/WebKit/NetworkProcess/cocoa, line 1163](https://github.com/WebKit/WebKit/blob/c526785513/Source/WebKit/NetworkProcess/cocoa/NetworkSessionCocoa.mm#L1163)).
* Rule: none.  Each Safari `e2e` and `web-test` run prints what it left (`references/local-browser-testing.md`).  `the_model_writes_open_no_write_stream` checks that the model writes open no write stream.
