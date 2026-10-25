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

* D1, the mix file: raw 32-bit float, little-endian, interleaved stereo, no header, at 44.1 kHz.  Its frames are its size divided by 8.  The path is `sessions/<id>/mix.f32`.  For a song of one source file, M5 can decode into this file (decision 0016: "that file is the mix").  If M5 changes the decoded samples, M5 rewrites the mix file of an older session.
* D2, when the page writes it: when the user presses Separate, on the storage path, if the session has no mix file of the song's frames.  The parent says "when a song opens".  The start covers new songs and the restored sessions of M3's layout with one path.  A song that is never separated costs no storage.  M5 writes session files when a song opens; the start then finds the mix file and writes nothing.
* D3, how the page writes it: in blocks of 524,288 frames (4 MiB), each a new whole `Uint8Array` (MEM-5).  A song of one source lane writes that lane (`original_lane`).  A song of several sources writes `dsp::mix_range` of each block, so the page never makes a song-length mix for a separation.  It writes `mix.f32.part` through one `createWritable` stream, then moves it onto `mix.f32`, inside `SAVING`.  So a `mix.f32` is always complete.  A failed write removes the `.part` file.
* D4, the wire format (PROTO-1): `Request::Separate { mix: Mix, session: Option<String> }`, where `Mix` is `Arrays { left, right }` (as today, transferred) or `Stored { path, frames }`.  On the wire, `{type: "separate", mix: "sessions/<id>/mix.f32", frames, session?}`, or `left` and `right` as today.  The worker takes all four combinations of mix and session.  The page sends `Stored` with a session on the storage path, and `Arrays` with no session on the memory path.
* D5, the block source: `roformer::demix` gets a trait `MixSource` with `frames()` and `read(start, end) -> Result<Stereo, Self::Error>`.  `Demixer` is generic over it and holds no whole mix.  `next_input` reads one span, at most one chunk, and returns `Result<Option<Stereo>, S::Error>`.  A whole `Stereo` in memory implements `MixSource` with `std::convert::Infallible` as its error.  It is the memory path, and the source of the tests and of `xtask roformer`.  The copy of a span is chunk-sized, not song-sized (MEM-1).
* D6, the worker's stored mix: it opens the file with `getFile()` once at the start, and checks that its size is `8 x frames`.  Then it reads each span with `blob_range`.  So it takes no lock.  The page never writes `mix.f32` while a separation runs: it writes the file only before it posts, and Separate stays disabled until the separation ends (`separate_enabled`).  A missing file, a wrong size or a failed read is a storage failure: the worker removes its stem files and sends `unstored` (D4 of M3), so the page falls back to the memory path.  The `unstored` doc and the page's status text say "browser storage failed", not only "could not keep the stems".
* D7, the free space: on the storage path the page needs `storage_needed(frames)`, plus `8 x frames` when the session has no mix file of the song's frames.  If the mix write fails, the page continues as for a song with no room in storage (finding 1-4 of M3): the memory path if the song is at most the limit, with a status line that says why and that the stems will not be kept; else the refusal.

### Step 1: the block source in `roformer`

* `roformer/src/demix.rs`: D5.  Update the module doc.  `roformer/tests/reference.rs` and `xtask/src/roformer_cli.rs` pass a whole `Stereo`.
* Tests, native (`cargo test -p roformer`):
  * The existing tests pass unchanged in what they check.
  * `a_block_source_gives_the_whole_mix_result_exactly`: for the cases of `streamed_segments_match_the_whole_song_result_exactly`, a test source that keeps only the song and records each `read` gives the same segments, bit for bit, as `reference`.  Each read covers at most `chunk` frames, and lies inside the song.
  * `a_failed_read_stops_the_demixer`: a source that fails its third read makes `next_input` return that error.

### Step 2: the wire format and the worker

* `protocol/src/lib.rs`: D4.  Update the module doc.  Extend `protocol/tests/web.rs`: `separate` with a stored mix, with and without a session, through a structured clone, with an empty transfer list.  A `separate` with neither `mix` nor `left` and `right` is a decode error.
* `worker/src/web.rs`: `run` makes the `Demixer` over a `Stereo` (arrays, as today) or over the stored mix of D6 (a new type, for example in `worker/src/web/mix.rs`).  `separate_chunks` maps a read failure to `Stop::Storage`.
* `app/src/separator.rs` changes only as much as the new type needs, so the page still compiles and sends `Arrays` until step 3.
* Tests in `worker/src/web/tests.rs`, with the fake network and a mix file planted under a session:
  * `a_stored_mix_separates_as_the_mix_in_memory`: a separation from the planted mix file and one from the same mix as arrays give the same `.part` file bytes.
  * `a_missing_mix_file_ends_with_unstored` and `a_mix_file_of_the_wrong_size_ends_with_unstored`: the replies end with one `unstored` that names the cause, with no `error` and no `done`, and no stem file remains.
  * `a_failed_mix_read_ends_with_unstored`: `separate_song` is generic over the source, so the test gives it a source that fails its second read.  The same end.
* `xtask/e2e/worker.js`: a helper that plants a file at a path.  `stored-stems`: the storage half plants the clip as `mix.f32` in its session and posts `{type: "separate", mix, frames, session}`.  The memory half stays as it is.  Check every other script in `xtask/src/e2e.rs` that posts `separate`.  Run `e2e storage` and `e2e stored-stems --hashes` (D8 below).

### Step 3: the page

* `app/src/store.rs`: `stored_mix_frames(id)` (the frames of `mix.f32`, from its size, or `None`), and `write_mix(id, song)` (D3).  Update the module doc's layout.
* `app/src/lib.rs`:
  * The start asks for the usage and for `stored_mix_frames` together.  `separation_path` takes the bytes needed, so a missing mix adds `8 x frames` (D7).
  * On the storage path with no mix file of the song's frames, the job waits in a new `JobStage::WritingMix` while `write_mix` runs.  The status line says that the page writes the mix to browser storage.  Separate and the session's Forget stay disabled (`separate_enabled`, `forget_enabled`).  When the write ends, the page posts, if the same song is still shown (D10 of M3).  If the write failed, D7.
  * `post_separation` sends `Mix::Stored` on the storage path, and makes no mix and no JS arrays there.  The memory path stays as it is.
  * `separation_unstored`: the status text of D6.
* `app/src/separator.rs`: `Separator::separate(mix, session)`.
* Tests:
  * Native: `the_separation_path_counts_a_missing_mix`: a song whose mix file is missing needs `8 x frames` more for the storage path.
  * Browser, `app/src/store.rs`: `the_mix_is_written_in_whole_blocks` spies on `FileSystemWritableFileStream.prototype.write` (MEM-5): every call is whole, and a song of more than one block makes more than one call.  `a_stored_mix_holds_the_song_interleaved`: for a song of one lane and a song of three sources, the bytes of `mix.f32` equal the lane, or `mix_of_sources`, interleaved, bit for bit, with a length that is not a multiple of the block.  No `.part` file is left.
  * Browser, `app/src/lib.rs` (M3's separation tests): `a_song_with_a_session_and_space_takes_the_storage_path` also checks that the request names `sessions/<id>/mix.f32` with the song's frames, carries no `left` or `right`, and transfers nothing, and that the mix file holds the song.  `a_failed_mix_write_takes_the_memory_path` (with the write refused by a spy) posts arrays with no session, with the status line of D7.  `a_kept_mix_is_not_written_again`: with a planted mix file of the song's frames, the page posts with no write to it.
  * `separate_names_the_session_on_the_storage_path` in `app/src/separator.rs` follows D4.

### Step 4: the e2e checks and the docs

* `e2e memory-budget`: a pure function checks that the 4-minute worker wasm peak is at most `MAX_PAGE_GROWTH_MB` (4 MB) above the 1-minute peak, with a native test.  The scenario fails on it, and prints the two peaks.
* Run in Chrome on the Mac, with `CHROMEDRIVER` and the weights of the team leader's prompt: `e2e stored-stems --hashes <M3's six>`, `e2e separate`, `e2e memory-budget`, `e2e storage`, and `web-test`.  Record the numbers in Progress.
* Docs (in the commit that makes each one wrong, at the latest here): `docs/ARCHITECTURE.md` (memory on phones; browser storage: `mix.f32`), TD-54 in `docs/tech-debt.md` (the worker no longer copies the mix on the storage path), MEM-5's "Enforced by" (the new spy test), and the README if its scenario list changes.  MEM-1 and LAYER-1 need no new words: the worker's mix copy stays only on the memory path, with its `#[expect]`, and no crate takes a new dependency.

### D8, the check that the stems do not change

`e2e stored-stems --hashes` with the six hashes of M3's Progress checks both things that decision 0016 asks for.  The memory path's stems keep their hashes, so the arrays path and the new `Demixer` give the stems of M3.  The storage path's `.part` files equal the memory path's stems byte for byte, so the mix file gives the same stems as the arrays.

## Validation and acceptance

The parent's M4 acceptance, check by check:

* "`Demixer` tests give the same output from a block source as from a whole mix": `a_block_source_gives_the_whole_mix_result_exactly` (CI `test`), `a_stored_mix_separates_as_the_mix_in_memory` (CI `browser-tests`), and `e2e stored-stems --hashes` with the real model (D8).
* "The MEM-5 spy covers the mix write": `the_mix_is_written_in_whole_blocks` (CI `browser-tests`).
* "`e2e memory-budget` shows no worker growth with the song's length": the scenario passes, with the 4-minute worker wasm peak at most 4 MB above the 1-minute peak.
* Every check of GIT-2 passes, and `cargo run -p xtask -- web-test --browser chrome`.

## Idempotence and recovery

* Each step is one or more commits that pass every check.  A step can be reverted alone, from the newest back.
* A start that stops during the mix write leaves `mix.f32.part`, which nothing reads.  The next write replaces it.  "Forget" removes it with the session.
* A wrong or stale `mix.f32` makes the worker send `unstored`, and the page falls back to the memory path (D6).  Removing the file makes the next start write it again.
* The memory path sends arrays as today, so a fault in the stored mix can be bypassed.

## Progress

* [x] 2026-10-26: milestone plan written.

## Surprises and discoveries

None yet.

## Decision log

* 2026-10-26, team leader: D1, a raw file with no header.  Only this app reads it, its frames come from its size, and it has no 4 GB limit as a WAV has.  The name `mix.f32` does not depend on how many sources the song has.
* 2026-10-26, team leader: D2, the mix is written at the start of a storage-path separation, not when a song opens (the parent's words).  One path covers new songs and the older sessions of M3, and a song that is never separated takes no storage.  M5 writes session files when a song opens, which makes the start's write a no-op.
* 2026-10-26, team leader: D5, `Demixer` asks a synchronous source for each span.  The worker reads a `Blob` with `FileReaderSync`, which is synchronous, so the loop needs no new `await`.
* 2026-10-26, team leader: D6, every failure of the mix file is `unstored`, so the page's fallback of D4 of M3 covers it, and needs no new reply.

## Outcomes and retrospective

Not started.
