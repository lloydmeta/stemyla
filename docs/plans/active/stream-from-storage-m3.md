# Stream from storage, M3: the worker writes the stems to storage

## Purpose

This is the milestone plan for M3 of `plans/active/stream-from-storage.md` (the parent plan).  It gives more detail than the parent's M3 section, which stays the criteria.  After M3:

* For a song kept in browser storage, the worker writes each stem to its session file as it separates.  It writes `sessions/<id>/stems/<stem>.wav.part`, and the page moves it onto `sessions/<id>/stems/<stem>.wav` when the stems are complete.  No stem samples cross to the page, and the page makes no song-length stem buffers during the separation.
* The page reads the finished stem files back into `engine::Samples` to show them.  Playback from storage is M6, so the shown stems stay in memory until then.
* When storage is unavailable or full, the page tells the user and separates on today's memory path, with `Assembly`.  It refuses a song that is too long for the memory path on an iPhone.

M3 does not close TD-54.  On the iPhone, the page still holds six stems after the separation (M6 and M7 remove them), and the worker still copies the whole mix (M4 removes it).

## Context and orientation

The state at the start of M3, on 2026-10-25, at commit `2c7440a` on the branch `iphone-crashes-2026-10-25`.  The parent plan's "The code today" and "Terms" apply.  These parts concern M3:

* `protocol/src/lib.rs`: `Request::Separate { left, right }` carries the mix as two transferred `Float32Array`s.  `Reply::Segment { start, len, stems }` carries every stem's finished samples from `start` on, for a song of `len` frames.  The module doc lists every message.  `protocol/tests/web.rs` round-trips each message through a structured clone.
* `worker/src/web.rs`:
  * `run` copies the mix into WebAssembly memory, feeds `Demixer` a chunk at a time, and calls `post_segment` for each ready `Segment` (`roformer/src/demix.rs`, `Segment { start, stems }`, each stem `[left, right]`).  Then it sends `Reply::Done`.
  * `pcm16` rounds a sample to 16 bits as `engine::wav::write_pcm16` does: `(x * 32768.0).round().clamp(-32768.0, 32767.0)`.
  * The inline JS writes OPFS files through `FileSystemWritableFileStream` (`opfs_write`, `whole`).  `app/src/streamtest.rs` (M1) already writes through a `FileSystemSyncAccessHandle` in an inline worker, with a spy on `FileSystemSyncAccessHandle.prototype.write`.  Use it as the model.
  * `worker/src/web/tests.rs`: browser tests in a dedicated worker, with no GPU.  `SENT` keeps each reply.  `start_spy` and `stop_spy` spy on `FileSystemWritableFileStream.prototype.write` (MEM-5).
* `app/src/separator.rs`: `Separator::separate(left, right)` posts the mix.  The `onmessage` closure puts each segment into `Assembly` (`app/src/assemble.rs`) and, at `done`, pushes one `Event::Stem` for each stem.  The tests use `deliver` (a reply to the page with no clone) and `spy_on_post`.
* `app/src/lib.rs`:
  * The "Separate 6 stems" button takes the original lane (`original_lane`) or `mix_of_sources` and calls `sep.separate`.  `SeparationJob` records the song's serial, session and name.
  * `poll_separator` collects `Event::Stem`s, and at `Event::Done` calls `apply_separated`.  That calls `store::save_stems` (16-bit WAVs and the `stem` lines of `meta`), then shows the stems with `with_original`, or keeps them for a song being opened (`stems_for_opening`).
  * The Storage menu's "Forget" calls `store::forget`.  "Reset…" stops the worker (`Separator::stop`) and then deletes OPFS (`app/src/reset.rs`).
  * `apply_separated` logs `separator: N stems in S s` and `separator: memory: page wasm N MB with the stems shown`.  `e2e separate` waits for the first line, and `e2e memory-budget` for the second.
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

Each commit leaves the docs true (DOC-1): a doc that a step makes wrong changes in that step's commit, not in step 5.  The wire format changes in one commit with `xtask/e2e/worker.js` and the scripts in `xtask/src/e2e.rs` (PROTO-1).  Write each test first and see it fail for the right reason, then write the code.  A test that cannot fail is a finding in review.

### Decisions of this milestone

The parent plan does not settle these points.  The Decision log gives the reasons.

* D1, the wire format (PROTO-1):
  * `Request::Separate` gets `session: Option<String>`, the session folder from the OPFS root (`"sessions/<id>"`).  With a session, the worker writes the stems there (storage path).  Without one, it works as today (memory path).  The field is `session` on the wire.
  * `Reply::Segment` keeps its samples, for the memory path only.  The storage path sends `Reply::Stored { frames, len, stems }` (`{type: "stored", frames, len, stems: [name, ...]}`) after each segment: the frames of each stem in its file so far, of a song of `len`, and the stem names in the worker's order.  So on the storage path no `segment` reply crosses, and no reply carries samples.
  * `Reply::Unstored { text }` (`{type: "unstored", text}`): storage could not hold the stems.  The worker stopped the separation and removed its partial stem files.  It ends the request, as `done` and `error` do.
  * `stem-protocol` gets one function that gives a stem's file name from its name (`file_stem` of `app/src/store.rs`, moved, plus `.wav`).  The worker and `store.rs` both use it, so the two sides agree on the file names.
* D2, the path: when the user presses Separate, the page takes the storage path if the song has a session, and the browser's estimate gives enough free space.  Free space is the quota less the usage (`store::usage`).  Enough is `6 x (44 + 4 x frames)` bytes plus 16 MB.  Otherwise it takes the memory path if the song is at most `MEMORY_PATH_MAX_FRAMES`, and refuses it if longer.  A pure function in `app` makes this choice, so a native test covers it.
* D3, the memory path's limit: `MEMORY_PATH_MAX_FRAMES` is 100 seconds at 44.1 kHz (4,410,000 frames).  It applies on every device, because a page cannot read its memory limit.  The derivation:
  * From M2 (parent plan, Progress), as the parent's M3 section says: the peaks are 966 MB for 1 minute and 1558 MB for 4 minutes.  The peak grows by (1558 - 966) / 3, about 197 MB a minute, from about 769 MB at 0 minutes.  With 950 MB for the parts that M2 does not measure (parent plan, M10), a song fits if 769 + 197 x minutes + 950 is at most 2048: 1.67 minutes or less.
  * M2 ran with `chunk=16384`.  At the default chunk the GPU holds about 250 MB more (979 MB on the iPhone, against 732 MB).  The 950 MB is a margin, not a measure: the parent's evidence table gives about 490 MB for WebGL and the rest.  The two differences about cancel.
  * The parent's memory budget, from the iPhone run at the default chunk, also gives "about 1.7 minutes" for today's design.  The iPhone separated the 30-second demo song.  The constant rounds 1.67 minutes down to 100 seconds.
  * The doc comment of the constant gives this derivation.  M10 can change the constant on its measures.
* D4, a failure during the separation: when a stem file cannot be opened or written, the worker closes its handles, removes the stem files of this separation and sends `unstored`.  Any other failure of the separation (a GPU error, for example) also closes the handles and removes the stem files.  Then the worker sends `error`, as today.  On `unstored`, the page tells the user why:
  * If the separated song is still shown (`StemsGo::Show`) and is at most the limit, the page separates it again on the memory path.
  * If it is longer, the page refuses it, and tells the user to free space in the Storage menu.
  * If another song is shown, the page tells the user to open the song and press Separate again.  It does not separate the shown song.
  * The fallback starts the memory path directly.  It does not choose the path again (D2), so a storage that still looks free cannot send the song back to the storage path.
  * The memory path of the fallback does not save the stems (`store::save_stems`): storage just refused them, and a save that fails part-way would leave the kept stems mixed.  The status line says that the stems are not kept.
* D5, the session's files: the worker writes each stem to `stems/<name>.wav.part`, never to `stems/<name>.wav`.  After `done`, the page moves each `.part` file onto its `.wav` file (as `opfs_commit` of the worker does, with `FileSystemFileHandle.move`), and writes the `stem` lines of `meta`.  It does both inside `SAVING`, so `restore_id` and `forget` wait for them.  It writes the new `stem` lines only when every move succeeded.  If a move fails, it writes `meta` with no `stem` lines, so a restore never mixes the stems of two separations.  The page still shows the stems: it reads each one from its `.wav` file if it moved, else from its `.part` file.  The status line says that the stems are not kept.  So:
  * The kept stems of an earlier separation stay until the new stems are complete.  A failed or stopped separation leaves them as they were.
  * No reader opens a file that the worker writes (M1, Chrome).  A stem lane stays empty until its file is complete.
* D6, the stems that the page shows: after D5, the page reads each stem file into `engine::Samples`, one stem at a time, with the code of `rebuild` (put into one function that both use).  So the shown stems hold the 16-bit samples, as a restored session does (decision 0016).  The Separate button stays disabled until the read ends.
* D7, Forget during a separation: the Storage menu disables "Forget" for the session of a starting or running storage-path separation, with a hover text that says why.  A pure function decides it, so a native test covers it.  "Reset…" stops the worker first, as today.  If the browser keeps a file locked after that, the reset fails and offers "Try again", as today.
* D8, the WAV header: the worker writes `engine::wav::header`'s 44-byte layout itself, because it cannot take `engine` (LAYER-1).  It writes the header with sizes of 0 when it opens a file, and the real RIFF and data sizes after the last segment.  A song whose data would not fit a `u32` WAV size is refused before the first chunk.  The e2e check of step 4 compares the files with `engine::wav::encode_pcm16` byte for byte, so it checks the header too.
* D9, the measure of page growth: the page logs `separator: memory: page wasm N MB as the separation starts` after it posts a separation, and `separator: memory: page wasm N MB as the separation ends` when `done` arrives, before it reads the stems.  `e2e memory-budget` prints the difference for each song and fails if it is more than 4 MB (the tolerance of M10).  The scenario waits for the line `with the stems shown`, not for the first `memory: page wasm` line.  WebAssembly memory never shrinks, so free pages can hide a small allocation.  The check catches song-length buffers such as `Assembly`'s (about 508 MB at 4 minutes).  Before the start, the page has freed the mix that `load_song` makes (one song-length stereo buffer, about 85 MB at 4 minutes), so an allocation up to about that size shows no growth.  `stored_replies_make_no_song_length_buffers` covers the rest.
* D10, the start: the start is asynchronous (`store::usage`).  When the user presses Separate, the page records the `SeparationJob` as starting, and disables Separate and the session's Forget.  It posts the request only if the same song (`song_serial`) is still shown when the estimate arrives.  Otherwise it drops the start and says so.

### Step 1: the stem writer in the worker

Add the stem writer to `worker/src/web.rs` (or a new module under `worker/src/web/`).  It needs D1's file-name function, so add that function to `stem-protocol` in this step.  `run` does not use the writer yet, and `mod web` is private, so clippy fails the unused code (`dead_code`).  Either commit steps 1 and 2 together, or put `#[cfg_attr(not(test), expect(dead_code, reason = "run uses it from step 2"))]` on the writer and remove it in step 2.

* Inline JS opens `<session>/stems/<name>.wav.part` for each stem (D5), with the file name of D1.  It opens the session folder without `create`, so it never brings back a forgotten session.  It opens `stems` and each file with `create`.  It takes a sync access handle, and truncates the file to 0.  Use `await` on `truncate`, `flush` and `close`, which also works where they return a value and not a promise.
* `put(segment)` converts each stem's frames to interleaved 16-bit little-endian samples with `pcm16`.  It writes them at `44 + 4 x start` as a new whole `Uint8Array` (MEM-5), never a view into WebAssembly memory.  A write that returns fewer bytes than given is a failure.
* `open` takes the song's `len`.  It refuses a `len` whose data does not fit a `u32` WAV size before it opens a file, and the worker then sends `error` (D8).
* `finish(len)` writes the header with the sizes of `len` frames, flushes and closes each handle.  It fails if a stem holds fewer than `len` frames.
* `abandon()` closes each handle and removes each stem file.  It removes nothing else.
* Each failure gives the browser's error name and text, for example `QuotaExceededError`.

Tests, in `worker/src/web/tests.rs` (no GPU):

* `stems_are_written_as_16_bit_wavs`: three segments of two stems, then `finish`.  Each file equals the expected bytes, given in the test: the header with its sizes, then the samples.  The samples include 0.5 / 32768 and -0.5 / 32768 (rounding away from zero), 1.0 and 1.5 (clipping to 32767), and -1.0.
* `stem_writes_pass_whole_arrays`: a spy on `FileSystemSyncAccessHandle.prototype.write` records "whole" or "view" for each call.  Every call is "whole" (MEM-5).
* `a_stem_writer_never_brings_back_a_forgotten_session`: with no session folder, opening fails and creates nothing.
* `a_refused_stem_write_removes_the_stem_files`: the spy refuses the third write with `QuotaExceededError`.  The error names it, and after `abandon` no stem file remains.  A new writer can then open the same files.
* `a_part_file_from_an_earlier_separation_is_replaced`: a longer planted `.part` file is truncated, so the file has only the new bytes.  A planted `.wav` file of the same stem is not changed.
* `a_song_too_long_for_a_wav_is_refused`: a `len` whose data does not fit a `u32` WAV size fails before any file opens (D8).

### Step 2: the wire format and the worker's run

* Before any change to `run`, write the memory half of step 4's `stored-stems` scenario.  It posts `separate` with no session to today's worker, and prints the SHA-256 of `engine::wav::encode_pcm16` of each of the six stems.  Run it in Chrome on the Mac and record the six hashes in Progress.  After step 2 and after step 3, run it again: the hashes must not change.  So the memory path stays the code of `main` bit for bit, although step 2 changes its loop.
* `protocol/src/lib.rs`: D1.  Update the module doc.  This step also changes the page as little as it can: `app/src/separator.rs` matches `Reply` exhaustively and builds `Request::Separate`.  Extend `protocol/tests/web.rs`: `separate` with and without `session`, `stored` and `unstored`, each through a structured clone.  A `separate` without a session transfers its two channels, as now.
* `worker/src/web.rs`: `serve` passes the session to `run`.  `run` sends its output to one of two places: `post_segment` (memory path) or the stem writer (storage path).  On the storage path it opens the writer before the first chunk, and sends `stored` after each `put`.  At the end it calls `finish`, then sends `done`.  A storage failure calls `abandon` and sends `unstored`, not `error`, and no `done`.  Every other failure calls `abandon` before the `error` (D4).
* Split `run` so that its loop over the chunks takes the network as a parameter: an async function from one chunk's input to the stems' output.  The real one runs `Job` on the GPU.  The loop function holds the stem writer, `abandon`, and the choice between `unstored` and `error`, so the tests below cover them.  So a test can run the whole loop with a fake network and no GPU, through `Demixer::accept` and `take_ready`.
* Tests, each with a fake network, asserting the whole reply sequence from `SENT`:
  * `a_storage_failure_ends_the_separation_with_unstored`: the spy refuses a stem write.  The replies end with one `unstored` that names the error, with no `error` and no `done`, and no stem file remains.
  * `a_network_failure_removes_the_part_files`: the fake network fails at the second chunk.  The replies end with one `error`, and no stem file remains.
  * `a_stored_separation_sends_no_samples`: a whole run gives `stored` replies whose last `frames` is `len`, then `done`, and no `segment`.  The `.part` files hold the fake network's stems, rounded to 16 bits.
* `xtask/e2e/worker.js`: `ask` ends a request at `unstored` too, and passes each `stored` reply to an `opts.stored` hook.  Add helpers that make a folder at a path and read the bytes of a file at a path.  Check every script in `xtask/src/e2e.rs` that posts `separate` or reads its replies.  Run `cargo run -p xtask -- e2e storage` (`.claude/rules/protocol.md`).

### Step 3: the page

* `app/src/separator.rs`: `Separator::separate(left, right, session: Option<&str>)`.  On the storage path, the page keeps `Assembly` empty.  It keeps the last `stored` reply.  At `done`, it sends `Event::StemsStored { names, frames, len, secs }`, or an error if `frames` is not `len`.  `unstored` becomes `Event::Unstored(text)`.  The memory path stays as it is.
* `app/src/lib.rs`:
  * One function starts a separation, used by the button and by the fallback of D4.  It records the start (D10), makes the mix as the button does today, chooses the path (D2), posts the request, and logs the start line of D9.  The status line says which path, and why when it is the memory path.  A refused song gets a status line that gives its length and the limit.
  * Split `apply_separated` in two parts: the save (`store::save_stems`, memory path only) and the rest (the destination, the show, and the two log lines of "Context and orientation").  Both paths use the second part, so `e2e separate` and `e2e memory-budget` find their lines on both paths, and the storage path writes no WAV twice.
  * `Event::StemsStored`: log the end line of D9.  Then, inside `SAVING`, move the `.part` files onto the `.wav` files and write the `stem` lines of `meta` (D5, a new function in `store.rs`).  Then, for `StemsGo::Show` (and a song being opened, as `stems_for_opening` does), read the stems (D6) and show them with `with_original`.  Then the second part of `apply_separated` shows them.  For `StemsGo::Kept`, stop after `meta`.  If the move or `meta` fails, the status line says that the stems are not kept, as for `save_stems` today.
  * `Event::Unstored`: D4.
  * The Storage menu: D7.
* `app/src/store.rs`: the function of D5, and one function that reads a stored stem to its song's frames, used by `rebuild` and by `StemsStored`.  `file_stem` moves to `stem-protocol` (D1).  Update the module doc: the worker writes the stem files on the storage path.

Tests:

* Native, in `app` (`cargo test`):
  * `the_separation_path_follows_storage_and_length` covers D2.  It covers a session with enough space, a session without enough space, no session, and a song at the limit and one frame over it.
  * `forget_is_disabled_for_the_session_being_separated` covers D7: Forget is disabled for the session of a starting or running storage-path separation, and enabled for other sessions and on the memory path.
  * `an_unstored_separation_goes_to_memory_only_for_the_shown_song` covers D4's three cases.
* Browser, in `app/src/separator.rs` with `spy_on_post` and `deliver`:
  * `separate_names_the_session_on_the_storage_path`: the request carries `session` and transfers both channels.  Without a session it carries none.
  * `stored_replies_make_no_song_length_buffers`: `stored` replies and `done` for a song of 4 minutes (`len` 10,584,000) give `Event::StemsStored`.  `Assembly` stays empty, and `page_wasm_mb()` grows by less than 1 MB (`Assembly` would add about 508 MB).  A `done` whose last `stored` frames are less than `len` gives an error.
* Browser, for the fallback (the parent plan's test that "makes OPFS writes fail"), in a wasm-only test module of `app/src/lib.rs`.  Each test makes `StemApp::default()` with a `Separator` whose posts are spied on, and a short song with a session: plant `sessions/<id>/meta` and set the app's session.  The worker's script is not in the test page, so its `onerror` pushes `Event::Stopped`: clear `onerror` before the test, or ignore that event.  Test the refusal of a long song through D2's pure function and a stubbed song length, not with a song over 100 seconds.
  * `unstored_separates_a_short_song_in_memory`: after `unstored` for a short song, with no stub of `navigator.storage.estimate` (so Chrome reports free space), the status line gives the reason, and the page posts exactly one request, without a session.
  * `unstored_refuses_a_long_song`: after `unstored` for a song over the limit, the status line gives the reason and the limit, and the page posts nothing.
  * `a_full_storage_chooses_the_memory_path_before_the_start`: with `navigator.storage.estimate` stubbed to give no free space, a short song posts without a session and a long one is refused, each with its status line.
  * `a_song_with_a_session_and_space_takes_the_storage_path`: the control.  The same song, with no stub of the estimate, posts with its session when the user presses Separate.  Without it, the tests above would pass for a song with no session.
  * `a_start_for_a_song_no_longer_shown_posts_nothing`: D10.
  * Together with `a_refused_stem_write_removes_the_stem_files` (step 1) and `a_storage_failure_ends_the_separation_with_unstored` (step 2), these cover the parent's fallback check.  The worker tests make real OPFS writes fail.  The page tests start from the `unstored` reply, because a page test has no worker that separates (Decision log).
* Browser, in `app/src/store.rs`:
  * `stored_stems_replace_the_kept_stems`: with planted `.wav` and `.part` files and a `meta` with older `stem` lines, D5's function leaves the `.part` bytes at the `.wav` paths, no `.part` file, and `meta` with the new `stem` lines and the same `file` lines.  The MEM-5 spy sees the `meta` write as whole.
  * `a_stored_stem_reads_back_to_the_song_frames`: a stem WAV planted with `engine::wav::encode_pcm16` reads back to the 16-bit samples, padded to the song's frames.

### Step 4: the e2e checks

* A new scenario, `stored-stems`, from a blank page with the real model (`--weights`), driven by hand as `worker-check` is:
  * The clip is 3 seconds of the synthetic mix (`crate::synth`), with `chunk: 16384, overlap: 2`, so that it has several segments.
  * One worker separates the clip on the memory path, then on the storage path into a planted session folder.
  * The scenario reads the six `.part` files back from OPFS.  It checks in Rust that each file equals `engine::wav::encode_pcm16` of the memory path's stem, byte for byte.  It also checks that the storage path sent no `segment` reply, and that its last `stored` reply has `frames` equal to `len`.
  * It also prints the SHA-256 hashes of step 2, and checks them when `--hashes` gives them.
  * Pass the bytes to Rust as base64 or as numbers.  Do not round in JS: JS's `Math.round` rounds -0.5 to -0, not to -1.
* `e2e memory-budget`: D9.
* Run, in Chrome, with `CHROMEDRIVER` and the weights of the team leader's prompt: `e2e stored-stems`, `e2e separate`, `e2e memory-budget`, `e2e storage`.  `e2e separate` now takes the storage path, so it checks the page's whole storage path with the real model.  Record the numbers in Progress, in this plan.

### Step 5: the docs

* `docs/ARCHITECTURE.md`: memory on phones ("Separated stems leave the worker chunk by chunk") and browser storage: the two paths, the stem writer, the limit.
* `docs/invariants.md`: add the new spy test to MEM-5's "Enforced by" line.  Do not change the text of a rule.  MEM-1 and LAYER-1 need no new words in M3: the page's stems are still `Samples`, and the worker takes no new crate.
* `docs/tech-debt.md`, TD-54: say what M3 removed (the page's song-length stems during a storage-path separation) and what is left.  The storage path has no length limit, but the page then reads all six stems into memory (D6), and so does each restore.  So on an iPhone a long song can now separate, and then stop the tab when the page shows its stems.  Each reload restores `sessions/current` (`StemApp::new`, `store::restore`) and reads the stems whole again, so the tab can stop at each reload.  M6 and M7 remove this.  No build between M3 and M7 goes to the published site: publishing is a person's step (GIT-1), and the orchestrator says so in the PR description.
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
* A separation that stops (a reload, Reset, a closed tab) leaves `.part` files, which no reader opens (D5).  The kept stems stay as they were.  The next separation of the session replaces the `.part` files.  "Forget" removes them with the session.
* The memory path stays as it is, so a fault in the storage path can be bypassed by sending no session.

## Progress

* [x] 2026-10-25: milestone plan written.
* [x] 2026-10-25: plan review 1: 5 should-fix fixed (the stem names, the limit's derivation, a testable loop, `.part` files, an asynchronous start), and 5 nits.
* [x] 2026-10-25: plan review 2: 2 should-fix fixed (the fallback starts the memory path directly; `apply_separated` splits so both paths log the lines of the e2e scenarios), and 8 nits.
* [x] 2026-10-25: plan review 3: 4 should-fix fixed (step 1 and clippy, the fallback tests have a session and a control, the growth test measures memory, the memory path's hashes before and after the split), and 5 nits.  The review loop stops at 3 reviews.
* [x] 2026-10-25: step 1, the stem writer (`worker/src/web/stems.rs`) and `stem_protocol::stem_file`.  The six tests of step 1 pass in Chrome (web-test: worker 48 passed).  `run` does not use the writer yet.  Commit `a705989`.
* [x] 2026-10-25: step 2 begins: `e2e stored-stems` with the memory path only, run twice in Chrome on the Mac (AMD Radeon Pro 5500M) at commit `a705989`, before any change to `run`.  Both runs gave the same six SHA-256 hashes of the stems as 16-bit WAVs (17 segments, 132,300 frames):
  * bass `e41500168c92024ecc32778654fa77e22a630f31200e99965274f6cdd931b028`
  * drums `9e916ca00e639c59f9eb31e2b6b11d4bdb04c43ebe9a38f7e71564ba79f3a024`
  * other `619875b264b286dba4c222bbe8e73d9b63909283905522a7c906f718e1d67f69`
  * vocals `f1b79d872435cdb931306a0c33228905c2d6b3ddfd33e9f1cb55720f52d0b624`
  * guitar `7e5eb334e7dead42ef2f7902c1590b2561347aa7b7f17b817aced81c295127bb`
  * piano `3166e0de310c930f2b40f75c2631e4524b7115c64f90d4ef540102af6da68f90`
* [x] 2026-10-25: step 2, commit `93fab48` (the memory half of `stored-stems`) and the commit after it.  The wire format of D1, with `xtask/e2e/worker.js`.  The worker's loop takes the network as a parameter (`Network`, `GpuNetwork`, `separate_song`).  `e2e stored-stems --hashes` passes after the change: the six hashes did not change.  `e2e storage` passes.  web-test: worker 52 passed, protocol 18 passed.  Commit `c65582e`.
* [x] 2026-10-25: step 3, the page.  `e2e stored-stems --hashes` passes after step 3: the six hashes did not change.  `e2e separate` passes, now on the storage path (page wasm 10 MB as the separation starts and 10 MB as it ends, 14 MB with the stems shown).  web-test: app lib 30 passed (11 new).  Each new page test fails when its rule is broken by hand (six mutations, each failed its test).  Commit `1e52430`.
* [x] 2026-10-25: step 4, the e2e checks, in Chrome on the Mac (AMD Radeon Pro 5500M), at commit `1e52430` with the scenarios of step 4:
  * `e2e stored-stems --hashes`: passed.  The six `.part` files equal `engine::wav::encode_pcm16` of the memory path's stems, byte for byte (529,244 bytes each).  The storage path sent 17 `stored` replies and no `segment` reply; the last `stored` gave 132,300 of 132,300 frames.  The hashes did not change.
  * `e2e separate`: passed, on the storage path.  Page wasm 10 MB as the separation starts, 10 MB as it ends, 13 MB with the stems shown.
  * `e2e memory-budget`: passed, both songs on the storage path.  The page grew by 0 MB during each separation (1 minute: 50 MB at the start and at the end; 4 minutes: 177 MB and 177 MB).  Peaks: 1 minute, page wasm 177 MB, worker wasm 57 MB, GPU live 732 MB, 966 MB in all; 4 minutes, page wasm 686 MB, worker wasm 99 MB, GPU live 732 MB, 1517 MB in all.  The 4-minute peak is 551 MB above the 1-minute peak (M2: 592 MB).  The page's peak comes after the separation, when it reads the six stems back (D6), as TD-54 says.
  * `e2e storage`: passed.
* [x] 2026-10-25: step 5, the docs.  Commit `ffeb4a8` was step 4.  MEM-5's "Enforced by" names `stem_writes_pass_whole_arrays` (the worker's sync writes) and `stored_stems_replace_the_kept_stems` (the page's `meta` write).  Step 3 already updated `docs/ARCHITECTURE.md` and TD-54, and steps 2 and 4 the README's scenarios.  `docs/working-with-agents.md` lists no scenarios, so it does not change.
* [x] 2026-10-26: review round 1 fixes, in the commit "review round 1 fixes" after `3d8364a`.  Fixed, each with a test: 1-1 and 1-b3 (three page tests of the storage path's end, and `separate_enabled`), 1-3 (a queue of posted separations; two separator tests), 1-4 (no save into a full storage), 1-b1 (two tests that an open and a removal wait for `keep_stored_stems`), 1-b2 (a refused header write in `finish`), the confirmed missed mutants (native tests of `clock`, `too_long_text`, `song_name`, the "not kept" note, `Reply::kind`, and the `stored-stems` and `memory-budget` checks), and the nits 1-5, 1-6, 1-8, 1-9, 1-10, 1-11, 1-12, 1-14 and 1-b5.  Not changed: 1-2 (the parent plan) and 1-7 (a commit message).  web-test: app lib 36, app web 14, worker 55, protocol 18 passed.  `e2e stored-stems --hashes` passed (the hashes did not change), and `e2e separate` passed on the storage path (page wasm 10 MB at the start and at the end).

## Surprises and discoveries

* 2026-10-25, step 2: with `mm` set but no `attn`, each new worker calibrates its attention path (`ensure_kernels` in `worker/src/web.rs` calls `choose_kernels` unless both are set).  So two runs of `stored-stems` could do different arithmetic.  With `mm: s64, attn: split`, two runs gave the same six hashes (Progress, step 2).

## Decision log

* 2026-10-25, team leader: D1 keeps `segment` with samples for the memory path, and adds `stored`.  The parent plan says that `segment` no longer carries samples, but the fallback (decision 0015) still needs them.  A separate reply keeps each decoder simple.  `stored` carries the stem names, because the page does not take `roformer`, which holds `STEMS`.
* 2026-10-25, team leader: D2 and D3.  The page cannot read its memory limit, so the limit of the memory path applies everywhere.  On a computer the memory path is rare: it needs a song with no session, or a full storage.  The limit follows the parent's formula on M2's numbers.  M2's smaller chunk and the 950 MB margin differ from the iPhone in opposite directions, and the parent's iPhone estimate gives the same 1.7 minutes.
* 2026-10-25, team leader: D4.  The page, not the worker, chooses to separate again in memory, because only the page knows the song's length and the limit.  It does so only for the song that is still shown, so a late `unstored` never separates another song.  The worker removes its partial files, so they take no space.
* 2026-10-25, team leader, after plan review 1: D5 writes `.part` files that the page moves into place after `done`.  Removing the `stem` lines before the separation would lose the kept stems of a failed re-separation.  The page does the move inside `SAVING`, so a restore never sees half the stems moved.
* 2026-10-25, team leader: D6.  Until M6, the page reads the stems back to play them.
* 2026-10-25, team leader: D7.  The spec does not say what `removeEntry` does to a locked file, so "Forget" is disabled for the session being separated.
* 2026-10-25, team leader: the parent's fallback check is split between the worker's browser tests and the page's browser tests.  A browser test has no GPU, and the e2e scenarios have no way to make one write fail.  The worker's tests run the whole loop with a fake network, so they see the whole reply sequence.
* 2026-10-25, team leader, after plan review 2: the fallback of D4 starts the memory path directly, so it cannot loop back to storage.  `meta` gets the new `stem` lines only when every move succeeded.
* 2026-10-25, team leader, after plan review 3: the memory path's stems are hashed before step 2 changes the loop, and compared after, because `stored-stems` compares two paths through the same new loop.  The fallback's memory path does not save its stems.  The parent's M3 text says that `segment` no longer carries samples: D1 keeps them for the memory path only.  The maintainer asked that the milestone writes only this plan, so the parent plan does not change here; the orchestrator can record D1 there.
* 2026-10-25, implementer, step 1: `file_stem` moves to `stem-protocol` as `stem_file` (with `.wav`) in step 1, not step 3.  So the page and the worker use one copy of the name rule from the first commit.  `store.rs` calls it.
* 2026-10-25, implementer, step 1: `StemWriter::open` keeps the song's `len`, so `finish` takes no argument.  A failure is `StemError::Storage` (the browser refused; the worker sends `unstored`) or `StemError::Refused` (a song too long for a WAV, or segments that do not follow on; the worker sends `error`).
* 2026-10-25, implementer, step 1: when `open` fails part of the way, it closes the handles it took and removes the files it opened.  The caller then has no writer to abandon.
* 2026-10-25, implementer: a Progress line cannot hold the hash of its own commit.  So each step's line gives the step, and the next step's commit adds the hash.
* 2026-10-25, implementer, step 2: `stored-stems` fixes the kernels with `mm: s64, attn: split` (see "Surprises and discoveries").
* 2026-10-25, implementer, step 2: `xtask` takes `sha2` (RustCrypto, first published 2016-05-06, 0.11.1 on 2026-10-12, per crates.io), pinned to the 0.10.9 that `Cargo.lock` already held.  The page sends each stem's floats to the runner as base64 (`e2e.base64`), so JS rounds nothing.
* 2026-10-25, implementer, step 2: the memory half of `stored-stems` is its own commit, before the wire format changes.
* 2026-10-25, implementer, step 2: the worker opens the stem writer before the first chunk.  A storage failure at the open sends `unstored`, and a song too long for a WAV sends `error`.  The loop's failures are `Stop::Storage` (`unstored`) and `Stop::Failed` (`error`); both abandon the writer first.
* 2026-10-25, implementer, step 2: until step 3, the page treats an `unstored` reply as a failed separation, and ignores `stored`.  The page names no session before step 3, so neither arrives.
* 2026-10-25, implementer, step 3: the page's fallback tests make a song "too long" with a smaller limit (`StemApp::memory_limit`), not a stubbed song length.  The pure function of D2 takes the limit as a parameter, and its native test checks `MEMORY_PATH_MAX_FRAMES` at the limit and one frame over it.
* 2026-10-25, implementer, step 3: the fallback of D4 starts a job with no session and `keeps_stems: false`.  So its stems are never saved, and they show only on the song that is still shown (else they are discarded, with a status line that says so).
* 2026-10-25, implementer, step 3: step 3 updates `docs/ARCHITECTURE.md` (memory on phones, browser storage, the diagram) and TD-54, because the page takes the storage path from this commit (DOC-1).  Step 5 adds the test names to MEM-5 and checks the rest.
* 2026-10-25, implementer, step 3: Separate stays disabled while a separation starts and while the page reads stored stems.  When the read ends, the page decides again where the stems go (`show_separated`), because the user can open another song during the read.  A storage-path separation whose song is neither shown nor being opened reads no stems.
* 2026-10-25, implementer, step 3: the page logs the end line of D9 at `done` on both paths, before it reads or shows any stems.
* 2026-10-25, implementer, step 4: the page logs `separator: separating on the storage path` (or `memory`) as it posts a separation.  `e2e separate` checks it, so that scenario fails if the stored clip falls back to the memory path.  `memory-budget` checks the growth of D9 for each song.
* 2026-10-25, implementer, step 4: `stored-stems` tells the two separations apart by the `progress` reply of 0 chunks that starts each one.  `xtask` does not take `stem-protocol`, so the scenario writes the stem file names as `<name>.wav`, which `stem_file` gives for the six stem names.
* 2026-10-25, implementer, review round 1 (finding 1-4): when D2 sends a song with a session to the memory path because storage has too little free space, the job keeps no session and does not save its stems, as D4's fallback does.  A save into a full storage could fail part of the way.  The status line says that the stems will not be kept.
* 2026-10-25, implementer, review round 1 (finding 1-3): the separator keeps a queue of the separations that it posted and the worker has not ended.  The worker runs them in order, and ends each with exactly one `done`, `error` or `unstored`.  So each reply belongs to the oldest one, and a stale reply cannot end a newer separation.  `separate` no longer clears `Assembly`.
* 2026-10-25, implementer, review round 1: the checks of `stored-stems` and `memory-budget` are pure functions with native tests.  The scenario bodies run only with a GPU and the model, so a mutant that empties them survives `cargo test` (TD-39).
* 2026-10-25, team leader: the review rounds record approvals as "0 (auto mode; a fallback prompt is not counted)", as the M1 log did.  The team leader acts for the orchestrator in step 11 of the skill.

## Outcomes and retrospective

Not started.
