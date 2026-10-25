# Stream from storage, M3: the worker writes the stems to storage

## Purpose

This is the milestone plan for M3 of `plans/active/stream-from-storage.md` (the parent plan).  It gives more detail than the parent's M3 section, which stays the criteria.  After M3:

* For a song kept in browser storage, the worker writes each stem to its session file, `sessions/<id>/stems/<stem>.wav`, as it separates.  No stem samples cross to the page, and the page makes no song-length stem buffers during the separation.
* The page reads the finished stem files back into `engine::Samples` to show them.  Playback from storage is M6, so the shown stems stay in memory until then.
* When storage is unavailable or full, the page tells the user and separates on today's memory path, with `Assembly`.  It refuses a song that is too long for the memory path on an iPhone.

M3 does not close TD-54.  On the iPhone, the page still holds six stems after the separation (M6 and M7 remove them), and the worker still copies the whole mix (M4 removes it).

## Context and orientation

The state at the start of M3, on 2026-10-25, at commit `2c7440a` on the branch `iphone-crashes-2026-10-25`.  The parent plan's "The code today" and "Terms" apply.  These parts concern M3:

* `protocol/src/lib.rs`: `Request::Separate { left, right }` carries the mix as two transferred `Float32Array`s.  `Reply::Segment { start, len, stems }` carries every stem's finished samples from `start` on, for a song of `len` frames.  The module doc lists every message.  `protocol/tests/web.rs` round-trips each message through a structured clone.
* `worker/src/web.rs`:
  * `run` copies the mix into WebAssembly memory, feeds `Demixer` a chunk at a time, and calls `post_segment` for each ready `Segment` (`roformer/src/demix.rs`, `Segment { start, stems }`, each stem `[left, right]`).  Then it sends `Reply::Done`.
  * `pcm16` rounds a sample to 16 bits as `engine::wav::write_pcm16` does: `(x * 32768.0).round().clamp(-32768.0, 32767.0)`.
  * The inline JS writes OPFS files through `FileSystemWritableFileStream` (`opfs_write`, `whole`).  No code uses a `FileSystemSyncAccessHandle` yet.
  * `worker/src/web/tests.rs`: browser tests in a dedicated worker, with no GPU.  `SENT` keeps each reply.  `start_spy` and `stop_spy` spy on `FileSystemWritableFileStream.prototype.write` (MEM-5).
* `app/src/separator.rs`: `Separator::separate(left, right)` posts the mix.  The `onmessage` closure puts each segment into `Assembly` (`app/src/assemble.rs`) and, at `done`, pushes one `Event::Stem` for each stem.  The tests use `deliver` (a reply to the page with no clone) and `spy_on_post`.
* `app/src/lib.rs`:
  * The "Separate 6 stems" button takes the original lane (`original_lane`) or `mix_of_sources` and calls `sep.separate`.  `SeparationJob` records the song's serial, session and name.
  * `poll_separator` collects `Event::Stem`s, and at `Event::Done` calls `apply_separated`.  That calls `store::save_stems` (16-bit WAVs and the `stem` lines of `meta`), then shows the stems with `with_original`, or keeps them for a song being opened (`stems_for_opening`).
  * The Storage menu's "Forget" calls `store::forget`.  "Reset…" stops the worker (`Separator::stop`) and then deletes OPFS (`app/src/reset.rs`).
  * `apply_separated` logs `separator: memory: page wasm N MB with the stems shown`.
* `app/src/store.rs`: `save_sources` makes the session `sessions/<id>` before the song shows, so the page knows the session when the user can press Separate.  The demo song and a song whose save failed have no session.  `save_stems` and `rebuild` hold the stem file names (`file_stem`).  `rebuild` reads a stem WAV whole (`read`) and decodes it (`engine::decode::decode_bytes`), then pads it to the song's frames (`padded_to`).  `SAVING` makes `restore_id` and `forget` wait for a stem save.  `usage` gives the bytes used and the quota (`navigator.storage.estimate()`).
* `xtask/src/e2e.rs`: `memory_budget` separates songs of 1 and 4 minutes in the app and takes the peak of each part from the `memory: ...` lines.  It waits for the first line with `memory: page wasm`.  `worker_check` posts `separate` by hand.  `xtask/e2e/worker.js` (`e2e.ask`) ends a request at a `model`, `error`, `selftest` or `done` reply.
* M1 (parent plan, "Surprises and discoveries"): Chrome failed 2 of 29 reads of a file while a worker wrote it through a sync access handle.  So a stem lane stays empty until its file is complete.

Facts from outside the repo, checked on 2026-10-25:

* `FileSystemSyncAccessHandle` is in dedicated workers only, on OPFS files.  `read`, `write`, `truncate`, `getSize`, `flush` and `close` are synchronous in current browsers.  Earlier versions of the spec made four of them asynchronous, and some old browsers did so ([MDN](https://developer.mozilla.org/en-US/docs/Web/API/FileSystemSyncAccessHandle)).  `write(buffer, { at })` writes at the offset `at` and returns the number of bytes written.
* `createSyncAccessHandle()` takes an exclusive lock on the file.  If it cannot, it rejects with `NoModificationAllowedError` ([File System spec](https://fs.spec.whatwg.org/)).  The spec's `removeEntry` does not say what happens to a locked file, so M3 does not remove a session while the worker writes into it (D7).

Terms, besides the parent plan's:

* Storage path: a separation whose worker writes the stems to their session files.
* Memory path: today's separation, whose worker sends the stems to the page in `segment` replies, put together by `Assembly`.
* Stem writer: the worker's object that holds one sync access handle for each stem file of one separation.

## Plan of work

### How the team works

The team leader wrote this plan, has it reviewed, runs the review rounds and commits Progress.  One implementer (a `general-purpose` subagent) does steps 1 to 5 and the review fixes.  It starts no other agent.  Before each commit it runs the checks of GIT-2 (`CLAUDE.md`, Commands).  It commits signed (`git commit -S`) with the attribution lines that the team leader gives.  It does not push.

Each commit leaves the docs true (DOC-1).  The wire format changes in one commit with `xtask/e2e/worker.js` and the scripts in `xtask/src/e2e.rs` (PROTO-1).  Write each test first and see it fail for the right reason, then write the code.  A test that cannot fail is a finding in review.

### Decisions of this milestone

The parent plan does not settle these points.  The Decision log gives the reasons.

* D1, the wire format (PROTO-1):
  * `Request::Separate` gets `session: Option<String>`, the session folder from the OPFS root (`"sessions/<id>"`).  With a session, the worker writes the stems there (storage path).  Without one, it works as today (memory path).  The field is `session` on the wire.
  * `Reply::Segment` keeps its samples, for the memory path only.  The storage path sends `Reply::Stored { frames, len }` (`{type: "stored", frames, len}`) after each segment: the frames of each stem in their files so far, of a song of `len`.  So on the storage path no `segment` reply crosses, and no reply carries samples.
  * `Reply::Unstored { text }` (`{type: "unstored", text}`): storage could not hold the stems.  The worker stopped the separation and removed its partial stem files.  It ends the request, as `done` and `error` do.
* D2, the path: when the user presses Separate, the page takes the storage path if the song has a session, and the browser's estimate gives enough free space.  Free space is the quota less the usage (`store::usage`).  Enough is `6 x (44 + 4 x frames)` bytes plus 16 MB.  Otherwise it takes the memory path if the song is at most `MEMORY_PATH_MAX_FRAMES`, and refuses it if longer.  A pure function in `app` makes this choice, so a native test covers it.
* D3, the memory path's limit: `MEMORY_PATH_MAX_FRAMES` is 100 seconds at 44.1 kHz (4,410,000 frames).  It applies on every device, because a page cannot read its memory limit.  The derivation, from M2 (parent plan, Progress):
  * M2's peaks: 966 MB for 1 minute, 1558 MB for 4 minutes.  The peak grows by (1558 - 966) / 3, about 197 MB a minute, from about 769 MB at 0 minutes.
  * The parts that M2 does not measure take about 950 MB (parent plan, M10).  So a song fits if 769 + 197 x minutes + 950 is at most 2048, that is 1.67 minutes or less.  The constant rounds this down to 100 seconds.
  * The doc comment of the constant gives this derivation.
* D4, a failure during the separation: when a stem file cannot be opened or written, the worker closes its handles, removes the stem files of this separation and sends `unstored`.  The page tells the user why.  It separates the song again on the memory path if the song is at most the limit.  If not, it refuses it, and tells the user to free space in the Storage menu.  Any other failure of the separation (a GPU error, for example) also closes the handles and removes the stem files.  Then the worker sends `error`, as today.
* D5, the session's `meta`: before the page posts a storage-path separation, it removes the `stem` lines from the session's `meta`.  After `done`, it writes them again with the new stems, inside `SAVING`, as `save_stems` does.  So a restore during the separation finds no stems, and never reads a file that the worker writes (M1).  If the page cannot write `meta`, it takes the memory path, as for a full storage (D2).
* D6, the stems that the page shows: after `done`, the page reads each stem file into `engine::Samples`, one stem at a time, with the code of `rebuild` (put into one function that both use).  So the shown stems hold the 16-bit samples, as a restored session does (decision 0016).  A stem lane stays empty until its file is complete.
* D7, Forget during a separation: the Storage menu disables "Forget" for the session of a running storage-path separation, with a hover text that says why.  "Reset…" stops the worker first, as today.  If the browser keeps a file locked after that, the reset fails and offers "Try again", as today.
* D8, the WAV header: the worker writes `engine::wav::header`'s 44-byte layout itself, because it cannot take `engine` (LAYER-1).  It writes the header with sizes of 0 when it opens a file, and the real RIFF and data sizes after the last segment.  A song whose data would not fit a `u32` WAV size is refused before the first chunk.  The e2e check of step 4 compares the files with `engine::wav::encode_pcm16` byte for byte, so it checks the header too.
* D9, the measure of page growth: the page logs `separator: memory: page wasm N MB as the separation starts` after it posts a separation, and `separator: memory: page wasm N MB as the separation ends` when `done` arrives, before it reads the stems.  `e2e memory-budget` prints the difference for each song and fails if it is more than 4 MB (the tolerance of M10).  The scenario waits for the line `with the stems shown`, not for the first `memory: page wasm` line.

### Step 1: the stem writer in the worker

Add the stem writer to `worker/src/web.rs` (or a new module under `worker/src/web/`).  It is not yet used by `run`.

* Inline JS opens `<session>/stems/<name>.wav` for each stem.  It opens the session folder without `create`, so it never brings back a forgotten session.  It opens `stems` and each file with `create`.  It takes a sync access handle, and truncates the file to 0.  Use `await` on `truncate`, `flush` and `close`, which also works where they return a value and not a promise.
* `put(segment)` converts each stem's frames to interleaved 16-bit little-endian samples with `pcm16`.  It writes them at `44 + 4 x start` as a new whole `Uint8Array` (MEM-5), never a view into WebAssembly memory.  A write that returns fewer bytes than given is a failure.
* `finish(len)` writes the header with the sizes of `len` frames, flushes and closes each handle.  It fails if a stem holds fewer than `len` frames.
* `abandon()` closes each handle and removes each stem file.  It removes nothing else.
* Each failure gives the browser's error name and text, for example `QuotaExceededError`.

Tests, in `worker/src/web/tests.rs` (no GPU):

* `stems_are_written_as_16_bit_wavs`: three segments of two stems, then `finish`.  Each file equals the expected bytes, given in the test: the header with its sizes, then the samples.  The samples include 0.5 / 32768 and -0.5 / 32768 (rounding away from zero), 1.0 and 1.5 (clipping to 32767), and -1.0.
* `stem_writes_pass_whole_arrays`: a spy on `FileSystemSyncAccessHandle.prototype.write` records "whole" or "view" for each call.  Every call is "whole" (MEM-5).
* `a_stem_writer_never_brings_back_a_forgotten_session`: with no session folder, opening fails and creates nothing.
* `a_refused_stem_write_removes_the_stem_files`: the spy refuses the third write with `QuotaExceededError`.  The error names it, and after `abandon` no stem file remains.  A new writer can then open the same files.
* `a_stem_file_from_an_earlier_separation_is_replaced`: a longer planted file is truncated, so the file has only the new bytes.

### Step 2: the wire format and the worker's run

* `protocol/src/lib.rs`: D1.  Update the module doc.  Extend `protocol/tests/web.rs`: `separate` with and without `session`, `stored` and `unstored`, each through a structured clone.  A `separate` without a session transfers its two channels, as now.
* `worker/src/web.rs`: `serve` passes the session to `run`.  `run` sends its output to one of two places: `post_segment` (memory path) or the stem writer (storage path).  On the storage path it opens the writer before the first chunk, and sends `stored` after each `put`.  At the end it calls `finish`, then sends `done`.  A storage failure calls `abandon` and sends `unstored`, not `error`, and no `done`.  Every other failure calls `abandon` before the `error` (D4).
* Test `a_storage_failure_ends_the_separation_with_unstored`: a storage failure in the function that `run` calls for each segment sends one `unstored` with the error name, and no `error` and no `done`.  Structure `run` so that this function runs without a GPU.
* `xtask/e2e/worker.js`: `ask` ends a request at `unstored` too.  Check every script in `xtask/src/e2e.rs` that posts `separate` or reads its replies.  Run `cargo run -p xtask -- e2e storage` (`.claude/rules/protocol.md`).

### Step 3: the page

* `app/src/separator.rs`: `Separator::separate(left, right, session: Option<&str>)`.  On the storage path, the page keeps `Assembly` empty.  It keeps the frames of the last `stored` reply.  At `done`, it sends `Event::StemsStored { frames, len, secs }`, or an error if `frames` is not `len`.  `unstored` becomes `Event::Unstored(text)`.  The memory path stays as it is.
* `app/src/lib.rs`:
  * One function starts a separation, used by the button and by the fallback of D4.  It makes the mix as the button does today, chooses the path (D2), writes `meta` (D5), posts the request, and logs the start line of D9.  The status line says which path, and why when it is the memory path.  A refused song gets a status line that gives its length and the limit.
  * `Event::StemsStored`: log the end line of D9.  Write the `stem` lines of `meta` (a new function in `store.rs`, inside `SAVING`).  Then, for `StemsGo::Show` (and a song being opened, as `stems_for_opening` does), read the stems (D6) and show them with `with_original`.  For `StemsGo::Kept`, write `meta` only.
  * `Event::Unstored`: D4.
  * The Storage menu: D7.
* `app/src/store.rs`: the `meta` functions of D5, and one function that reads a stored stem to its song's frames, used by `rebuild` and by `StemsStored`.  Update the module doc: the worker writes the stem files on the storage path.

Tests:

* Native, in `app` (`cargo test`): `the_separation_path_follows_storage_and_length` covers D2.  It covers a session with enough space, a session without enough space, no session, and a song at the limit and one frame over it.
* Browser, in `app/src/separator.rs` with `spy_on_post` and `deliver`:
  * `separate_names_the_session_on_the_storage_path`: the request carries `session` and transfers both channels.  Without a session it carries none.
  * `stored_replies_make_no_song_length_buffers`: `stored` replies and `done` give `Event::StemsStored`, and `Assembly` stays empty.  A `done` whose last `stored` frames are less than `len` gives an error.
* Browser, for the fallback (the parent plan's test that "makes OPFS writes fail"), with a `Separator` whose posts are spied on:
  * `unstored_separates_a_short_song_in_memory`: after `unstored` for a short song, the status line gives the reason, and the page posts the song again without a session.
  * `unstored_refuses_a_long_song`: after `unstored` for a song over the limit, the status line gives the reason and the limit, and the page posts nothing.
  * `a_full_storage_chooses_the_memory_path_before_the_start`: with `navigator.storage.estimate` stubbed to give no free space, a short song posts without a session and a long one is refused, each with its status line.
  * Together with `a_refused_stem_write_removes_the_stem_files` (step 1) and `a_storage_failure_ends_the_separation_with_unstored` (step 2), these cover the parent's fallback check.  A browser test cannot run the GPU, so no single test does it end to end (Decision log).
* Browser, in `app/src/store.rs`:
  * `the_stem_lines_go_before_and_come_back_after`: D5's two `meta` writes, under the MEM-5 spy, keep the `file` lines.
  * `a_stored_stem_reads_back_to_the_song_frames`: a stem WAV planted with `engine::wav::encode_pcm16` reads back to the 16-bit samples, padded to the song's frames.

### Step 4: the e2e checks

* A new scenario, `stored-stems`, from a blank page with the real model (`--weights`), driven by hand as `worker-check` is:
  * The clip is 3 seconds of the synthetic mix (`crate::synth`), with `chunk: 16384, overlap: 2`, so that it has several segments.
  * One worker separates the clip on the memory path, then on the storage path into a planted session folder.
  * The scenario reads the six stem files back from OPFS.  It checks in Rust that each file equals `engine::wav::encode_pcm16` of the memory path's stem, byte for byte.  It also checks that the storage path sent no `segment` reply, and that its last `stored` reply has `frames` equal to `len`.
  * Pass the bytes to Rust as base64 or as numbers.  Do not round in JS: JS's `Math.round` rounds -0.5 to -0, not to -1.
* `e2e memory-budget`: D9.
* Run, in Chrome, with `CHROMEDRIVER` and the weights of the team leader's prompt: `e2e stored-stems`, `e2e separate`, `e2e memory-budget`, `e2e storage`.  `e2e separate` now takes the storage path, so it checks the page's whole storage path with the real model.  Record the numbers in the parent plan's Progress line for M3.

### Step 5: the docs

* `docs/ARCHITECTURE.md`: memory on phones ("Separated stems leave the worker chunk by chunk") and browser storage: the two paths, the stem writer, the limit.
* `docs/invariants.md`: add the new spy test to MEM-5's "Enforced by" line.  Do not change the text of a rule.  MEM-1 and LAYER-1 need no new words in M3: the page's stems are still `Samples`, and the worker takes no new crate.
* `docs/tech-debt.md`, TD-54: say what M3 removed (the page's song-length stems during a storage-path separation) and what is left.
* `README.md` and `docs/working-with-agents.md`: the new e2e scenario, if `docs-check` or their lists of scenarios need it.

## Validation and acceptance

The parent's M3 acceptance, check by check:

* "A browser test separates a short song and reads the stems back from OPFS.  They equal the stems of `main` bit for bit, after 16-bit rounding": `cargo run -p xtask -- e2e stored-stems --browser chrome --weights <model>` passes.  The memory path is the code of `main`.  Also `stems_are_written_as_16_bit_wavs` (CI `browser-tests`).
* "The MEM-5 spy covers the stem writes": `stem_writes_pass_whole_arrays` (CI `browser-tests`).
* "`e2e memory-budget` shows no page growth during the separation": the scenario prints the growth for 1 and 4 minutes, each at most 4 MB, and passes.
* "A browser test makes OPFS writes fail.  It checks the message, a short song on the memory path, and the refusal of a long song": the tests of step 3's fallback list, with the two worker tests.
* Every check of GIT-2 passes: fmt, the clippy commands of `CLAUDE.md`, `cargo test --workspace --exclude roformer-gpu`, `cargo test -p roformer-gpu --lib`, `docs-check`.  `cargo run -p xtask -- web-test --browser chrome` passes.

## Idempotence and recovery

* Each step is one or more commits that pass every check.  A step can be reverted alone, from the newest back.
* A separation that stops (a reload, Reset, a closed tab) leaves partial stem files, which `meta` does not list (D5).  The next separation of the session replaces them.  "Forget" removes them with the session.
* The memory path stays as it is, so a fault in the storage path can be bypassed by sending no session.

## Progress

* [x] 2026-10-25: milestone plan written.

## Surprises and discoveries

None yet.

## Decision log

* 2026-10-25, team leader: D1 keeps `segment` with samples for the memory path, and adds `stored`.  The parent plan says that `segment` no longer carries samples, but the fallback (decision 0015) still needs them.  A separate reply keeps each decoder simple.
* 2026-10-25, team leader: D2 and D3.  The page cannot read its memory limit, so the limit of the memory path applies everywhere.  On a computer the memory path is rare: it needs a song with no session, or a full storage.
* 2026-10-25, team leader: D4.  The page, not the worker, chooses to separate again in memory, because only the page knows the song's length and the limit.  The worker removes its partial files, so they take no space.
* 2026-10-25, team leader: D5 and D6.  Until M6, the page reads the stems back to play them.  It removes the `stem` lines from `meta` during the separation, so no reader sees a file that the worker writes (M1, Chrome).
* 2026-10-25, team leader: D7.  The spec does not say what `removeEntry` does to a locked file, so "Forget" is disabled for the session being separated.
* 2026-10-25, team leader: the parent's fallback check is split between the worker's browser tests and the page's browser tests.  A browser test has no GPU, and the e2e scenarios have no way to make one write fail.

## Outcomes and retrospective

Not started.
