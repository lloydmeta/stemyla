# Stream from storage, M4: the worker reads the mix from storage

## Purpose

This is the milestone plan for M4 of `plans/active/stream-from-storage.md` (the parent plan).  It gives more detail than the parent's M4 section, which stays the criteria.  After M4:

* On the storage path, the page keeps the mix that the worker separates in a session file, `sessions/<id>/mix.f32`.  The separate request names that file and carries no samples.
* `roformer::demix::Demixer` reads its input from a source of blocks, one chunk with its overlap at a time.  The worker reads the mix file in blocks, so its memory does not grow with the song.
* The stems do not change: the model separates the same `f32` samples as before (decision 0016).

M4 does not close TD-54.  The page still holds the decoded song (M5) and reads the six stems back after a separation (M6, M7).

## Context and orientation

The state at the start of M4, on 2026-10-26, at commit `f10fbb7` on the branch `iphone-crashes-2026-10-25`.  The parent plan's "The code today" and "Terms", and the M3 plan (`plans/active/stream-from-storage-m3.md`, its D1 to D10), apply.  These parts concern M4:

* `roformer/src/demix.rs`: `Demixer::new(mix: Stereo, chunk, overlap, stems)` keeps the whole mix.  `next_input` cuts the next chunk from it through `sample`, which reads the reflect padding by index.  `accept`, `take_ready` and `finish` hold at most one chunk of running sums.  The tests compare the streamed output with `reference`, the old whole-song implementation, bit for bit.  Other callers: `roformer/tests/reference.rs` and `xtask/src/roformer_cli.rs`.
* `worker/src/web.rs`: `serve` passes `Request::Separate { left, right, session }` to `run`.  `run` copies the two transferred `Float32Array`s into WebAssembly memory (an `#[expect]` of MEM-1), makes the `Demixer` and calls `separate_song`.  `separate_song` and `separate_chunks` take the network as a parameter (`Network`), so `worker/src/web/tests.rs` runs the whole loop with a fake network (`fake_mix`, `fake_demixer`, `FAKE_CHUNK`).  `Stop::Storage` ends with `unstored`, `Stop::Failed` with `error`.  `blob_range` reads a range of a `Blob` with `FileReaderSync`, which is synchronous.
* `protocol/src/lib.rs`: `Request::Separate { left, right, session }`; the module doc gives the wire format.  `protocol/tests/web.rs` round-trips each message.
* `app/src/separator.rs`: `Separator::separate(left, right, session)` copies the mix into two new `Float32Array`s (the JS heap) and transfers them.
* `app/src/lib.rs`: `start_separation` records the job as `JobStage::Starting` and asks `store::usage` for the free space.  `separation_ready` chooses the path with `separation_path` and `storage_needed`, and `post_separation` takes `original_lane` or makes `mix_of_sources`, then posts.  `separation_unstored` is the fallback of D4 of M3.
* `app/src/store.rs`: the session layout (module doc), `session_folder`, `SAVING`, `st_write` (one whole write through `createWritable`).
* `dsp::mix_range` mixes one range of the sources, and equals `dsp::mix` on that range (`mix_range_matches_mix_at_any_start`).
* `xtask/src/e2e.rs`: `stored_stems` posts `separate` by hand twice (memory, then storage) and checks the six hashes of M3's Progress with `--hashes`.  `memory_budget` takes the peak of page wasm, worker wasm and GPU live bytes, and checks the page's growth during the separation (`growth_check`, `MAX_PAGE_GROWTH_MB` = 4).  M3 measured worker wasm peaks of 57 MB at 1 minute and 99 MB at 4 minutes.  `xtask/e2e/worker.js` has `e2e.folder`, `e2e.read` and `e2e.plant` (root only).

Facts from outside the repo, checked on 2026-10-26:

* `FileReaderSync` is in workers only, and reads a `Blob` synchronously ([MDN](https://developer.mozilla.org/en-US/docs/Web/API/FileReaderSync)).  The worker already reads the model this way.
* `FileSystemFileHandle.getFile()` gives a `File` of the state on disk.  The spec takes locks only for `createWritable` and `createSyncAccessHandle` ([File System spec](https://fs.spec.whatwg.org/)).  MDN: "If the file on disk changes or is removed after this method is called, the returned File object will likely be no longer readable" ([MDN](https://developer.mozilla.org/en-US/docs/Web/API/FileSystemFileHandle/getFile)).  "Likely" is not a promise, so the page must not change the file during a separation (D6).

Terms, besides the parent's and M3's:

* Mix file: `sessions/<id>/mix.f32`, the mix that the worker separates (D1).
* Span: the frames of the song that one chunk reads, reflect padding included.

## Plan of work

### How the team works

The team leader wrote this plan, has it reviewed, runs the review rounds and commits Progress.  One implementer (a `general-purpose` subagent) does steps 1 to 4 and the review fixes.  It starts no other agent.  Before each commit it runs the checks of GIT-2 (`CLAUDE.md`, Commands), commits signed (`git commit -S`) with the attribution lines that the team leader gives, and does not push.

Each commit leaves the docs true (DOC-1).  The wire format changes in one commit with `xtask/e2e/worker.js` and the scripts in `xtask/src/e2e.rs` (PROTO-1).  Write each test first and see it fail for the right reason, then write the code.

### Decisions of this milestone

* D1, the mix file: raw 32-bit float, little-endian, interleaved stereo, no header, at 44.1 kHz.  Its frames are its size divided by 8.  The path is `sessions/<id>/mix.f32`.  When the file is complete, `meta` gets the line `mix<TAB><frames><TAB>f32le-1`.  The page uses a mix file only when `meta` has this line with the song's frames and the format `f32le-1`, and the file's size is `8 x frames`.  `store_meta::Meta::parse` ignores lines it does not know, so an older build ignores the line.  For a song of one source file, M5 can decode into this file (decision 0016: "that file is the mix").  If M5 changes the decoded samples or the layout, it uses a new format tag, so the mix of an older session is written again.  A dependency update that changes the decoded samples and keeps the frames is not detected; such a mix differs from the song by the decoder's change only.
* D2, when the page writes it: when the user presses Separate, on the storage path, if the session has no mix file of the song's frames.  The parent says "when a song opens".  The start covers new songs and the restored sessions of M3's layout with one path.  A song that is never separated costs no storage.  M5 writes session files when a song opens; the start then finds the mix file and writes nothing.
* D3, how the page writes it: in blocks of 131,072 frames (1 MiB), each a new whole `Uint8Array` (MEM-5).  A song of one source lane writes that lane (`original_lane`).  A song of several sources writes `dsp::mix_range` of each block, so the page never makes a song-length mix for a separation.  Inside `SAVING`, `write_mix` first removes the `mix` line of `meta` and any `mix.f32`.  Then it writes `mix.f32.part` through one `createWritable` stream, moves it onto `mix.f32`, and writes the `mix` line of D1.  So a `mix` line always names a complete file.  A failed write removes the `.part` file.  As `st_write` does, `write_mix` counts in `WRITES`, does nothing after `CLOSED`, and never makes a forgotten session folder again.
* D4, the wire format (PROTO-1): `Request::Separate { mix: Mix, session: Option<String> }`, where `Mix` is `Arrays { left, right }` (as today, transferred) or `Stored { path, frames }`.  On the wire, `{type: "separate", mix: "sessions/<id>/mix.f32", frames, session?}`, or `left` and `right` as today.  A `mix` whose `frames` is not a finite, non-negative whole number (NaN, -1 and 1.5 included; `as usize` would make them 0), a request with neither `mix` nor both arrays, and a request with both, are decode errors, not defaults (TD-16).  The worker takes all four combinations of mix and session.  The page sends `Stored` with a session on the storage path, and `Arrays` with no session on the memory path.
* D5, the block source: `roformer::demix` gets a trait `MixSource` with `frames()` and `read(start, end) -> Result<Stereo, Self::Error>`.  `Demixer` is generic over it and holds no whole mix.  `next_input(&mut self)` reads one span, at most one chunk, and returns `Result<Option<Stereo>, S::Error>`.  A whole `Stereo` in memory implements `MixSource` with `std::convert::Infallible` as its error.  It is the memory path, and the source of the tests and of `xtask roformer`.  The copy of a span is chunk-sized, not song-sized, and gets an `#[expect]` with that reason if clippy's MEM-1 lint needs one.
* D6, the worker's stored mix: it opens the file with `getFile()` once at the start, and checks that its size is `8 x frames` (computed in `f64`, so a long song does not overflow a wasm32 `usize`).  Then it reads each span with `blob_range`.  So it takes no lock.  The page never writes `mix.f32` while a separation runs: it writes the file only before it posts, and Separate stays disabled until the separation ends (`separate_enabled`).  A missing file, a wrong size or a failed read is a storage failure: the worker removes its stem files and sends `unstored` (D4 of M3), so the page falls back to the memory path.  This holds also for a stored mix with no session.  On `unstored`, the page removes the `mix` line of `meta` and the file `mix.f32` inside `SAVING`, so the next start writes the mix again, a bad file does not refuse a long song for good, and a stale file takes no space.  The next start waits for that removal (`store::settled`) before it reads `meta`, so the two never interleave their writes of `meta`.  The status texts of `separation_unstored`, the refusal included, say that browser storage failed, not that it is full.  The `unstored` doc and the page's status text say "browser storage failed", not only "could not keep the stems".
* D7, the free space: on the storage path the page needs `storage_needed(frames)`, plus `8 x frames` when the session has no usable mix file (D1).  `write_mix` removes an unusable one first.  The status line's reason gives the bytes of the stems and of the mix.  If the mix write fails, the page continues as for a song with no room in storage (finding 1-4 of M3): the memory path if the song is at most the limit, with a status line that says why and that the stems will not be kept; else the refusal.

### Step 1: the block source in `roformer`

* `roformer/src/demix.rs`: D5.  Update the module doc.  `roformer/tests/reference.rs` and `xtask/src/roformer_cli.rs` pass a whole `Stereo`.
* Tests, native (`cargo test -p roformer`):
  * The existing tests pass unchanged in what they check.  Add a case with `overlap` 1 (no padding), which `e2e separate` and `memory-budget` use.
  * `a_block_source_gives_the_whole_mix_result_exactly`: for the cases of `streamed_segments_match_the_whole_song_result_exactly`, a test source that keeps only the song and records each `read` gives the same segments, bit for bit, as `reference`.  Each read covers at most `chunk` frames, and lies inside the song.
  * `a_failed_read_stops_the_demixer`: a source that fails its third read makes `next_input` return that error.

### Step 2: the wire format and the worker

* `protocol/src/lib.rs`: D4.  Update the module doc.  Extend `protocol/tests/web.rs`: `separate` with a stored mix, with and without a session, through a structured clone, with an empty transfer list.  Each decode error of D4 is a case: neither `mix` nor arrays, both, and `frames` missing, NaN, -1 and 1.5.  Extend `hand_written_requests_are_read` with a hand-written stored-mix request.
* `worker/src/web.rs`: a new function takes the network (`impl Network`), the request's mix and session, and the chunk settings.  It opens the stored mix of D6 (a new type, for example in `worker/src/web/mix.rs`) or copies the arrays, checks the size, makes the `Demixer`, and calls the loop.  A mix file that is missing or of the wrong size sends `unstored` from it.  `run` makes the `GpuNetwork` and calls it; the tests call it with the fake network.  `separate_chunks` maps a read failure to `Stop::Storage`.
* The worker logs `memory: worker wasm N MB at the separation's start` before it reads or copies the mix, and `memory: worker wasm N MB at the separation's end` before `done`.  The words differ from the page's lines of D9 of M3 (`as the separation starts`, `as the separation ends`), which `page_growth` in `xtask/src/e2e.rs` finds by their text.  Step 4 checks the difference.
* `GpuNetwork.total` comes from the `Demixer`, so the new function makes the `Demixer` first and then asks for the network (for example through a closure, or by setting `total` after).
* `app/src/separator.rs` changes only as much as the new type needs, so the page still compiles and sends `Arrays` until step 3.
* Tests in `worker/src/web/tests.rs`, with the fake network and a mix file planted under a session:
  * `a_stored_mix_separates_as_the_mix_in_memory`: a separation from the planted mix file and one from the same mix as arrays give the same `.part` file bytes.  A third separation from the mix file with no session gives `segment` replies with the same samples.
  * `a_stored_mix_keeps_the_worker_memory_flat`: separations of a planted short mix (a few chunks) and a planted long mix (about 30 chunks), with the fake network and a session.  With `LIVE` and `PEAK` of the test allocator, the peak above the live bytes at the start differs between the two by less than the bytes of one span.  The test also runs the long mix as arrays and asserts that its peak is more than that bound above the short one's, so the control stays in the test.  The allocator sees only the Rust heap, so the test also spies on `FileReaderSync.prototype.readAsArrayBuffer`: each read is at most `8 x chunk` bytes, and there is one read for each chunk.  So a read of the whole file into the JS heap fails it.
  * `a_missing_mix_file_ends_with_unstored` and `a_mix_file_of_the_wrong_size_ends_with_unstored`: the replies end with one `unstored` that names the cause, with no `error` and no `done`, and no stem file remains.
  * `a_failed_mix_read_ends_with_unstored`: the loop is generic over the source, so the test gives it a source that fails its second read.  The same end.
* `xtask/e2e/worker.js`: a helper that plants a file at a path.  `stored-stems`: the storage half interleaves the clip (`stored_stems_clip` writes the left channel, then the right), plants it as `mix.f32` in its session and posts `{type: "separate", mix, frames, session}`.  The memory half stays as it is.  Check every other script in `xtask/src/e2e.rs` that posts `separate`.  Run `e2e storage` and `e2e stored-stems --hashes` (D8 below).

### Step 3: the page

* `app/src/store.rs`: `stored_mix_frames(id)` (the frames of the usable mix of D1, or `None`), and `write_mix(id, song)` (D3).  `store_meta.rs` reads and writes the `mix` line.  Update the module doc's layout.
* `app/src/lib.rs`:
  * The start asks for the usage and for the usable mix of D1 together.  `separation_path` takes the bytes needed, so a missing mix adds `8 x frames` (D7).
  * On the storage path with no mix file of the song's frames, the job waits in a new `JobStage::WritingMix` while `write_mix` runs.  The status line says that the page writes the mix to browser storage.  Separate and the session's Forget stay disabled (`separate_enabled`, `forget_enabled`, whose native test adds `WritingMix`).  Before the write, the page logs `separator: memory: page wasm N MB before the mix is written`.  When the write ends, the page posts, if the same song is still shown (D10 of M3).  If the write failed, D7.
  * `post_separation` sends `Mix::Stored` on the storage path, and makes no mix and no JS arrays there.  The memory path stays as it is.
  * `separation_unstored`: the status text of D6, and the removal of the `mix` line.
  * The page tests that press Separate and expect a post (for example `unstored_separates_a_short_song_in_memory`) wait for the mix write too.  Do not weaken what they check.
* `app/src/separator.rs`: `Separator::separate(mix, session)`.
* Tests:
  * Native: `the_separation_path_counts_a_missing_mix`: a song whose mix file is missing needs `8 x frames` more for the storage path.  `store_meta`: the `mix` line reads back, and an unknown format tag gives no usable mix.
  * Browser, `app/src/store.rs`: `the_mix_is_written_in_whole_blocks` spies on `FileSystemWritableFileStream.prototype.write` (MEM-5): every call is whole, and a song of more than one block makes more than one call.  `a_stored_mix_holds_the_song_interleaved`: for a song of one lane and a song of three sources, the bytes of `mix.f32` equal the lane, or `mix_of_sources`, interleaved, bit for bit, with a length that is not a multiple of the block.  No `.part` file is left.  `the_mix_write_makes_no_song_length_buffer`: as `stored_replies_make_no_song_length_buffers` does, for a song of three sources of about 4 minutes kept alive by the test, `page_wasm_mb()` grows by less than 4 MB across `write_mix` (a whole mix would add about 85 MB).
  * Browser, `app/src/lib.rs` (M3's separation tests): `a_song_with_a_session_and_space_takes_the_storage_path` also checks that the request names `sessions/<id>/mix.f32` with the song's frames, carries no `left` or `right`, and transfers nothing (the spy of `app/src/lib.rs`'s tests records the transfer list, as `spy_on_post` of `separator.rs` does), and that the mix file holds the song.  `a_failed_mix_write_takes_the_memory_path` (with the write refused by a spy) posts arrays with no session, with the status line of D7.  `a_kept_mix_is_not_written_again`: with a planted mix file and `mix` line of the song's frames, the page posts with no write to it.  `a_mix_without_its_meta_line_is_written_again`: a planted file of the right size with no `mix` line is written again.  The same for a `mix` line of other frames, and for a file whose size differs from its `mix` line.  `unstored_removes_the_mix_line`: after `unstored`, `meta` has no `mix` line and `mix.f32` is gone.
  * `separate_names_the_session_on_the_storage_path` in `app/src/separator.rs` follows D4.

### Step 4: the e2e checks and the docs

* `e2e memory-budget`, each with a pure function and a native test:
  * The worker's growth during each separation, from the start line to the end line of step 2, is at most 4 MB (rename `MAX_PAGE_GROWTH_MB` to `MAX_GROWTH_MB`).  A native test gives the page's and the worker's lines interleaved in their real order: the worker's end line comes before the page's.  WebAssembly memory never shrinks, and the model's load leaves free pages, so this check can miss a small allocation (as D9 of M3 says for the page).  M3's numbers (99 MB at 4 minutes, from 57 MB at 1 minute, with an 85 MB mix copy) show that it sees the old copy.  `a_stored_mix_keeps_the_worker_memory_flat` is the exact check.
  * The page's growth (D9 of M3) runs from the line `before the mix is written` when the page logs it, so it covers the mix write.
  * The scenario prints the worker wasm peak of each song.
* `e2e separate` and `e2e memory-budget` fail on the page's line `separator: separating on the memory path`, and on its warning on `unstored` (today `separator: browser storage could not keep the stems`; D6 changes its words, and the check follows them).  So a fallback cannot pass them.
* Run in Chrome on the Mac, with `CHROMEDRIVER` and the weights of the team leader's prompt: `e2e stored-stems --hashes <M3's six>`, `e2e separate`, `e2e memory-budget`, `e2e storage`, and `web-test`.  Record the numbers in Progress.
* Docs (in the commit that makes each one wrong, at the latest here): `docs/ARCHITECTURE.md` (memory on phones; browser storage: `mix.f32`), TD-54 in `docs/tech-debt.md` (the worker no longer copies the mix on the storage path), MEM-5's "Enforced by" (the new spy test), MEM-1's "Enforced by" (`a_stored_mix_keeps_the_worker_memory_flat`, `the_mix_write_makes_no_song_length_buffer`; `a_mix_is_transferred_not_copied` of MEM-2 now covers the arrays only), and the README if its scenario list changes.  MEM-1 and LAYER-1 need no new words: the worker's mix copy stays only on the memory path, with its `#[expect]`, and no crate takes a new dependency.

### D8, the check that the stems do not change

`e2e stored-stems --hashes` with the six hashes of M3's Progress checks both things that decision 0016 asks for.  The memory path's stems keep their hashes, so the arrays path and the new `Demixer` give the stems of M3.  The storage path's `.part` files equal the memory path's stems byte for byte, so the mix file gives the same stems as the arrays.

## Validation and acceptance

The parent's M4 acceptance, check by check:

* "`Demixer` tests give the same output from a block source as from a whole mix": `a_block_source_gives_the_whole_mix_result_exactly` (CI `test`), `a_stored_mix_separates_as_the_mix_in_memory` (CI `browser-tests`), and `e2e stored-stems --hashes` with the real model (D8).
* "The MEM-5 spy covers the mix write": `the_mix_is_written_in_whole_blocks` (CI `browser-tests`).
* "`e2e memory-budget` shows no worker growth with the song's length": the scenario passes, with the worker's growth during each separation at most 4 MB.  From M3's numbers the load leaves about 43 MB of free pages, so this check sees only a buffer larger than that, at 4 minutes.  `a_stored_mix_keeps_the_worker_memory_flat` (CI `browser-tests`) checks it exactly, for the Rust heap and, through its spy on `FileReaderSync`, for the JS heap.
* Every check of GIT-2 passes, and `cargo run -p xtask -- web-test --browser chrome`.

## Idempotence and recovery

* Each step is one or more commits that pass every check.  A step can be reverted alone, from the newest back.
* A start that stops during the mix write leaves `mix.f32.part`, which nothing reads.  The next write replaces it.  "Forget" removes it with the session.
* A `mix.f32` of the wrong size makes the worker send `unstored`, and the page falls back to the memory path (D6).  A file of the right size with no `mix` line, or an older format tag, is written again at the next start (D1).  Neither the page nor the worker reads the samples to check them.
* The memory path sends arrays as today, so a fault in the stored mix can be bypassed.

## Progress

* [x] 2026-10-26: milestone plan written (commit `cba0ef4`).
* [x] 2026-10-26: plan review 1 (fixes in `13a4426`): 6 should-fix fixed (the worker's growth is measured from the separation's start and by an allocator test, the mix checks run through a function that takes the network, the clip is interleaved, a fallback fails the e2e checks, the page's growth covers the mix write, a `mix` line in `meta`), and 7 nits.
* [x] 2026-10-26: plan review 2: 3 should-fix fixed (the worker's memory lines have their own words, the decision log says what M5 and M7 must do for a mix of several sources, a bad mix file is written again after `unstored`, with tests), and 8 nits.
* [x] 2026-10-26: plan review 3: 3 should-fix fixed (a test that the page's mix write makes no song-length buffer, a spy on the worker's reads of the mix file, every decode error of D4 tested), and 6 nits.  The review loop stops at 3 reviews.
* [x] 2026-10-26: step 1, the block source.  `roformer::demix::MixSource` (D5): `Demixer<S>` reads one span for each chunk, and keeps no whole mix.  A `Stereo` is a source with `Infallible` as its error.  New tests `a_block_source_gives_the_whole_mix_result_exactly` (8 cases, overlap 1 included; each read is inside the song and at most one chunk; one read for each chunk) and `a_failed_read_stops_the_demixer`.  `cargo test -p roformer --lib demix`: 6 passed.  The first test failed when the span was the whole song.  Workspace tests: 623 passed.
* [x] 2026-10-26: step 2, the wire format and the worker.  Commit `fb519d6` was step 1.  `stem_protocol::Mix` (D4), `worker/src/web/mix.rs` (`StoredMix`, D6), and `separate_mix`, which takes a function that makes the network for the song's chunks.  The worker logs its memory at the separation's start and end.  `e2e stored-stems` plants the clip, interleaved, as `mix.f32` for the storage half.  `e2e stored-stems --hashes` passed in Chrome on the Mac: the six hashes of M3 did not change, and the six `.part` files from the mix file equal the memory path's stems (529,244 bytes each, 17 segments).  `e2e storage` passed.  `a_stored_mix_keeps_the_worker_memory_flat`: Rust heap peak 237,984 bytes for 5,000 frames and 238,240 bytes for 30,000 frames (32 reads of the mix file, each at most one span), and 477,856 bytes for the 30,000 frames as arrays.  It failed when the worker read the whole mix file at once.  web-test: app lib 41, app web 14, web_close 1, web_output 4, worker 63, protocol 20 passed.
* [x] 2026-10-26: step 3, the page.  Commit `0bed77e` was step 2.  `store::write_mix`, `stored_mix_frames` and `forget_mix`; the `mix` line in `store_meta`; `JobStage::WritingMix`; `storage_path_needs`; the storage path posts `Mix::Stored`.  Native: `the_separation_path_counts_a_missing_mix` and `the_mix_line_reads_back_with_its_format` (workspace tests: 625 passed).  web-test: app lib 48 (7 new), app web 14, web_close 1, web_output 4, worker 63, protocol 20 passed.  Each new page test failed when its rule was broken by hand (five mutations: one song-length block, no size check of the mix file, no removal on `unstored`, a kept mix written again, and the removal counted only when it first runs).  `e2e separate` passed on the storage path: worker wasm 57 MB at the separation's start and at its end, page wasm 10 MB as the separation ends.
* [x] 2026-10-26: step 4, the e2e checks and the docs.  Commit `5597c01` was step 3.  `worker_growth`, `growth_check` for the page and the worker (`MAX_GROWTH_MB`), `page_growth` from the line before the mix write, and `no_fallback_check` in `separate` and `memory-budget`, with native tests (workspace tests: 627 passed).  Runs in Chrome on the Mac (AMD Radeon Pro 5500M), with the page built after step 3:
  * `e2e stored-stems --hashes` with the six hashes of M3: passed.  The six `.part` files from the mix file equal the memory path's stems (529,244 bytes each).
  * `e2e separate`: passed, on the storage path with no fallback line.  Page wasm 10 MB before the mix is written and as the separation ends, 14 MB with the stems shown; worker wasm 57 MB at the separation's start and end.
  * `e2e memory-budget`: passed, both songs on the storage path.  1 minute: page and worker grew by 0 MB (page 50 MB, worker 57 MB); peaks page wasm 177 MB, worker wasm 57 MB, GPU live 732 MB, 966 MB in all.  4 minutes: page and worker grew by 0 MB (page 177 MB, worker 57 MB); peaks page wasm 686 MB, worker wasm 57 MB, GPU live 732 MB, 1475 MB in all.  The 4-minute peak is 509 MB above the 1-minute peak (M3: 551 MB).  The worker's peak at 4 minutes fell from 99 MB (M3) to 57 MB, its peak after the model load.
  * `e2e storage`: passed.
  * web-test: app lib 48, app web 14, web_close 1, web_output 4, worker 63, protocol 20 passed.
* [x] 2026-10-26: review round 1 fixes, in the commit "review round 1 fixes" after `2eecbaa` (step 4).  Fixed, each with a test: 1-57 (`a_block_source_gives_the_whole_mix_result_exactly` checks that each read is the minimal span, with the case n=15, chunk 10, overlap 2; it failed with the mutant `<` to `<=` in `Demixer::span`), 1-9 (`frames` over `usize::MAX` is a decode error; the 2^33 case failed first), 1-8 (a stored mix with `left` only; it failed when the decoder checked only `right`), 1-10 (`the_mix_write_makes_no_song_length_buffer` adds a song of one lane; it failed, at 84.7 MB, when the lane was copied whole).  1-30, 1-35, 1-41 and 1-53 (scenario bodies that need a GPU, a browser or the weights) are in TD-39.  Docs: 1-2 to 1-7, 1-12 to 1-14.  Not changed: 1-11 (chunk-sized buffers in `StoredMix::read`).  web-test: app lib 48, app web 14, web_close 1, web_output 4, worker 63, protocol 20 passed.  Workspace tests: 627 passed.

## Surprises and discoveries

* 2026-10-26, step 3: `saving` counted a save in `SAVING` only when its future first ran.  `separation_unstored` starts `forget_mix` with `spawn_local`, so a start right after it could read `meta` before the removal began.  `unstored_removes_the_mix_line` failed with that order.  `forget_mix` now counts from its call (`SaveHeld`).

## Decision log

* 2026-10-26, team leader: D1, a raw file with no header.  Only this app reads it, its frames come from its size, and it has no 4 GB limit as a WAV has.  The name `mix.f32` does not depend on how many sources the song has.
* 2026-10-26, team leader: D2, the mix is written at the start of a storage-path separation, not when a song opens (the parent's words).  One path covers new songs and the older sessions of M3, and a song that is never separated takes no storage.  This changes the parent's M4 text; the orchestrator records it there.
* 2026-10-26, team leader, after plan review 2: M4's `write_mix` reads the song's samples in memory.  M5 can decode a song of one source file into `mix.f32`, which makes the start's write a no-op for it.  A song of several sources still needs a mix of its source files.  When M7 removes the samples from the page, M5 or M7 must write `mix.f32` and its `mix` line from the session files in blocks.  The orchestrator records this for M5 and M7.
* 2026-10-26, team leader: D5, `Demixer` asks a synchronous source for each span.  The worker reads a `Blob` with `FileReaderSync`, which is synchronous, so the loop needs no new `await`.
* 2026-10-26, team leader, after plan review 1: a `mix` line in `meta` marks a complete mix file of a known format, so a later milestone can tell an M4 file from its own.  The worker's growth is measured from the separation's start, because the model's load sets the worker's peak.  Blocks of 1 MiB keep the page's mix write under the 4 MB tolerance.
* 2026-10-26, team leader: D6, every failure of the mix file is `unstored`, so the page's fallback of D4 of M3 covers it, and needs no new reply.
* 2026-10-26, implementer, step 1: `MixSource::read` takes `&mut self`, so a test source can record its reads without a `RefCell`.  The callers of a `Stereo` source write `while let Ok(Some(x))`, because its error type has no values.
* 2026-10-26, implementer, step 2: `separate_mix` takes a function that makes the network from the song's chunks, so `run` makes `GpuNetwork` with `total` from the `Demixer`.  The worker opens the mix file before the stem writer, so a missing file or a wrong size makes no stem file.  A `mix` that is not a string is also a decode error.
* 2026-10-26, implementer, step 3: `apply_inbox` takes the `egui::Context`, so the mix write can ask for a repaint when it ends.  `write_mix` fails if it cannot remove an old `mix.f32`, so the page takes the memory path then (D7).  The status line of a failed mix write starts "browser storage could not keep the song's mix".  `the_mix_write_makes_no_song_length_buffer` gives the three sources one shared buffer (42 MB, not 254 MB), and reads the file's size, not its bytes, so it leaves no free pages that could hide the growth of a later test.

## Outcomes and retrospective

Not started.
