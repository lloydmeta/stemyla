# Stream from storage, M5: a song decodes from storage into storage

## Purpose

This is the milestone plan for M5 of `plans/active/stream-from-storage.md` (the parent plan).  It gives more detail than the parent's M5 section, which stays the criteria.  After M5:

* The page never holds the encoded source of a song that it keeps in browser storage.  It saves a chosen file to OPFS as a `Blob`, as today, and does not read its bytes.
* A new page-side worker, the decode worker (decision 0019), reads the stored source in blocks with sync reads.  It decodes, converts to 44.1 kHz and writes the session files and the overviews in blocks.  Its memory does not grow with the song.
* A song of one source decodes into `sessions/<id>/mix.f32`, M4's mix file, with M4's `mix` line.  So Separate writes no second mix file.  A song of several sources gets one session file for each source, and `mix.f32` made from them in blocks.
* A restored session decodes its stored sources in the same way, or uses its session files if they are complete.

M5 does not close TD-54.  After the decode, the page still reads the session files back into memory to play the song and draw its lanes, until M6 and M7 (the orchestrator's decision of 2026-10-26).  The page also still reads the stem WAVs of a restored session whole, until M7.

## Context and orientation

The state at the start of M5, on 2026-10-26, at commit `83dfdde` on the branch `iphone-crashes-2026-10-25`.  The parent plan's "The code today" and "Terms", and the M3 and M4 plans (their D1 to D10 and D1 to D8), apply.  These parts concern M5:

* `engine/src/decode.rs`: `decode_bytes` and `decode_path` decode a whole file into `Audio` (one `Vec<f32>` per channel, at the file's rate).  `open` probes the file and computes the reservation.  `decode_into_reservation` decodes packet by packet and grows the channels with `append`.  `MAX_DECODED_BYTES` (decision 0013) stops a file over 1 GiB of `f32` samples, with mono counted as two channels.  The tests use a peak allocator (`PeakAllocator`), FLAC files from `engine/src/test_flac.rs` and hand-made MP3 frames (`mp3_frame`, `quiet_start_mp3`).
* `engine/src/resample.rs`: `to_rate(x, from, to)` converts a whole channel with a polyphase filter (`Filter::apply`).  Output `i` is a dot product of `2 x half` input samples around `Position::at`, with zeros outside the input.  Its length is `ceil(len x phases / step)`.
* `engine/src/song.rs`: `Song::from_audio` sorts the sources by `rank`, takes at most two channels, converts the rate, makes a mono source stereo (both sides share one `Samples`) and pads every source with silence to the longest.  `Song::load_files` skips the names in `NOT_STEMS` (`mix`, `original`, `truth`), after `stem_name`.
* `app/src/lib.rs`: `handle_drops` and `pick_song` read every chosen file whole (`bytes_async`, `picker::read`), then `load_and_keep` decodes them on the main thread (`Song::load_files`) and saves them (`store::save_sources`).  It logs `song kept in browser storage as session <id>`, which `e2e smoke`, `e2e separate` and `e2e memory-budget` wait for.  `StemApp::new` restores `sessions/current` (`store::restore`); the Storage menu opens a session with `store::restore_id`.  `load_song` builds the lanes with `lane_columns` (`COLUMNS` = 4096 columns of `dsp::peaks`, and a scale from `dsp::biggest`), and the mix lane from `mix_of_sources`.  `original_lane` and `with_original` give the original lane.
* `app/src/store.rs`: `save_sources` writes each chosen `Blob` to `sessions/<id>/src/<i>.<ext>` and writes `meta`.  `restore_id` and `rebuild` read the sources whole (`read`, an `#[expect]` of MEM-1) and decode them with `Song::load_files`; `read_stem` reads a stem WAV whole.  `SAVING`, `saving`, `SaveHeld` and `settled` order the saves of a session; `WRITES` and `CLOSED` let a reset wait.  `stored_mix_frames`, `write_mix`, `forget_mix` and `MIX_BLOCK_FRAMES` are M4's.  `app/src/store_meta.rs`: `Meta`, `MIX_FORMAT` (`f32le-1`).
* `app/src/separator.rs`: how the page starts the stem worker (`WORKER_URL`, `Worker::new_with_options`), takes its replies into `Event`s, and logs `page_wasm_mb`.  The app's browser tests make a `Separator` whose script does not exist, spy on its posts (`spy_on_post`) and give it replies (`deliver`).
* `app/index.html` builds `stem-worker` with Trunk (`data-type="worker"`, `data-loader-shim`).  `app/src/reset.rs` lists its three files (`WORKER_FILES`) for "Reset…", and a test checks them against `app/index.html`.  `xtask/src/publish.rs` lists the files of a build.
* LAYER-1 (`docs/invariants.md`), the layers table in `docs/ARCHITECTURE.md`, and `WEB_APP`, `LAYERS` and the LAYER-1 tests in `xtask/src/rules.rs`.  DEPS-1 and `THIRD-PARTY.md` (its two `cargo tree` commands name `app` and `stem-worker`).  MEM-1's clippy lint is one `clippy.toml` in each crate, with a canary test (decision 0008).
* `xtask/src/web_test.rs` runs the browser tests of `app`, `stem-worker` and `stem-protocol`.  `.github/workflows/ci.yml` runs clippy for wasm32 on a list of crates, `web-test`, and `e2e smoke`, `audio-resume`, `storage` and `flac` in Chrome.
* `xtask/src/e2e.rs`: `memory_budget` drops a WAV of 1 and 4 minutes (`long_song`), separates it, and checks the growth of the page and the stem worker during the separation (`page_growth`, `worker_growth`, `growth_check`, `MAX_GROWTH_MB` = 4).  `memory_peak` takes the largest `page wasm`, `worker wasm` and `GPU buffers live` value of the `memory: ...` lines.

Facts from outside the repo, checked on 2026-10-26:

* `FileReaderSync` and `FileSystemSyncAccessHandle` exist in dedicated workers only ([MDN FileReaderSync](https://developer.mozilla.org/en-US/docs/Web/API/FileReaderSync), [MDN FileSystemSyncAccessHandle](https://developer.mozilla.org/en-US/docs/Web/API/FileSystemSyncAccessHandle)).  The stem worker already uses both (`blob_range`, M3's stem writer).
* Symphonia reads a `MediaSource`, which is `Read + Seek` with `is_seekable` and `byte_len` ([docs.rs](https://docs.rs/symphonia-core/latest/symphonia_core/io/trait.MediaSource.html)).  The format reader gives one packet at a time, so a decode in blocks needs no change to symphonia.
* eframe 0.36.2 reads a dropped file's bytes only when `bytes_async` is called; `web_file` gives the browser's `File` (`egui::DroppedFile`).

Terms, besides the parent's, M3's and M4's:

* Decode worker: the new crate's Web Worker (decision 0019).  The stem worker is `stem-worker`.
* Source: a file of the song that `Song::load_files` keeps (not `mix`, `original` or `truth`).  A song of one source is a song file; a song of several sources is a set of stems.
* Source file: `sessions/<id>/decoded/<i>.f32`, one decoded source at 44.1 kHz, where `<i>` is the index of its `src/<i>.<ext>` file.
* Overview file: `sessions/<id>/overviews/<lane>.f32`, the columns of a lane's waveform.
* Complete session: a session whose `meta` has a usable `decoded` line (D2).

## Plan of work

### How the team works

The team leader wrote this plan, has it reviewed, runs the review rounds and commits Progress.  One implementer (a `general-purpose` subagent) does steps 1 to 5 and the review fixes.  It starts no other agent.  Before each commit it runs the checks of GIT-2 (`CLAUDE.md`, Commands), commits signed (`git commit -S`) with the attribution lines that the team leader gives, and does not push.

Each commit leaves the docs true (DOC-1).  A message changes in one commit with its encoder, its decoder and their tests (PROTO-1).  Write each test first and see it fail for the right reason, then write the code.  A workaround or a known gap gets a `docs/tech-debt.md` entry in the same commit.

### Decisions of this milestone

* D1, the session files.  Every session file of a song holds the song's frames: raw 32-bit float, little-endian, interleaved stereo, no header, at 44.1 kHz, as M4's mix file (format `f32le-1`).  A mono source has the same samples on both sides.  A source has at most two channels, as `Song::from_audio` takes.
  * A song of one source: `mix.f32` is its only source file (decision 0016).  There is no `decoded/` folder.
  * A song of several sources: `decoded/<i>.f32` for each source, padded with silence to the longest, and `mix.f32`, which is `dsp::mix_range` of the sources at volume 1 (the samples of `mix_of_sources`).
  * Overviews: `overviews/<i>.f32` for each source lane, and `overviews/mix.f32` for the mix lane (`mix_of_sources`, which `dsp::mix` clamps).  An overview file holds the columns of `lane_columns`: for each column, its minimum and then its maximum, as 32-bit little-endian floats.  The scale is not stored; it comes from the columns.  Each overview has a fixed size (at most 32 KiB), so it does not grow with the song.
  * The samples are those of `decode_bytes` and `Song::from_audio` today, bit for bit (D4).  So the format tag stays `f32le-1`, and an `mix.f32` of M4 is still usable.
* D2, `meta`.  The page is the only writer of `meta`.  When the files of D1 are complete, the page writes the line `decoded<TAB><frames><TAB>f32le-1`, and the `mix` line of M4 if it moved a new `mix.f32` into place.  A session is complete when its `decoded` line has the format `f32le-1`, and every file of D1 has its size: `8 x frames` for `mix.f32` and each `decoded/<i>.f32`, and `8 x columns` for each overview, with `columns` = `ceil(frames / ceil(frames / COLUMNS))`.  `Meta::parse` ignores lines it does not know, so an older build ignores the line.  Before the page asks for a decode, it removes the `decoded` line.
* D3, the decode worker crate.  Directory `decoder/`, package `decode-worker`, in the Shells layer, with the row `decode-worker | dsp, engine, stem-protocol`.  It takes `engine` with `default-features = false` (no stretcher, no `cpal`).  Its other dependencies are crates that `app` or `stem-worker` already take, at the same versions (`wasm-bindgen`, `wasm-bindgen-futures`, `js-sys`, `web-sys`, `console_error_panic_hook`).  So DEPS-1 asks only for the `THIRD-PARTY.md` commands to name it.  Trunk builds it from `app/index.html` as it builds `stem-worker`, with a loader shim: `decode-worker_loader.js`, `decode-worker.js` and `decode-worker_bg.wasm`.  It has its own `clippy.toml` of MEM-1 with a canary test, as `stem-worker` has.
* D4, the engine API.  `engine` gets three block parts, each natively tested against the whole version, bit for bit:
  * `engine::decode::Decoder`: opens a `MediaSource` as `open` does, and gives the next packet's planar `f32` samples on each call.  It keeps the checks of `MAX_DECODED_BYTES` (decision 0013): it stops with the same error when the frames so far pass the limit.  `decode_bytes` and `decode_path` keep their behaviour, their reservation and their tests, built on `Decoder`.
  * `engine::resample::Resampler`: takes input in blocks of any size and gives the output of `to_rate` on the whole input, bit for bit, with the same length.  At the same rate it passes the input through.  It holds at most one filter window of input, not the song.
  * A stereo sink trait (for example `engine::song::FrameSink` with `put(&mut self, left: &[f32], right: &[f32])`) and a function that decodes one source into a sink at 44.1 kHz: at most two channels, mono to both sides, and the rate converted in blocks (for example `engine::song::decode_source`).  `Song::from_audio` keeps its behaviour, and a memory sink gives the samples of `Song::from_audio` for one source.
  * `engine::overview`: `lane_columns` moves from `app` to `engine`, with a block builder that gives the same columns and scale from blocks of any size, when it knows the frames first.  `app`'s `load_song` calls it.  The block builder is for the decode worker and M7.
* D5, the overview is made in a second pass.  The columns of `lane_columns` depend on the song's frames.  An MP3 states only an estimate of its frames, and the frames of a song of several sources are the longest source's.  So the decode worker makes the overviews after the decode, in a pass that reads the session files in blocks.  This is the "same pass" of the parent's M5 section in what it asks: no song-length memory, and no whole read.  For a song of several sources, the same pass pads the source files, writes `mix.f32` and makes every overview.
* D6, the messages (PROTO-1).  New messages in `stem-protocol`, in their own module (for example `stem_protocol::decode`), with their own `kind` constants, one encoder and one decoder each, and round-trip tests in `protocol/tests/web.rs`.  No samples cross, so there is nothing to transfer (MEM-2).
  * Request `decode`: the session folder (`sessions/<id>`), the sources in the order of `Song::from_audio` (each its stored file `src/<i>.<ext>`, its index `<i>` and its extension), and `mix_frames`, the frames of a usable `mix.f32` of D1 of M4, if the page found one.  A missing or wrong field is a decode error, not a default (TD-16).
  * Replies: `decoded` with the song's frames and whether the worker wrote a new `mix.f32.part`; `undecodable` with a text, when a source cannot be decoded (a format error, a file too long); `unstored` with a text, when browser storage fails (a read or a write); and `log` lines.  Each request ends with exactly one of the first three.
* D7, the files the worker writes.  The worker writes each file of D1 as `<name>.part` through one `FileSystemSyncAccessHandle`, in blocks of 131,072 frames (1 MiB), each a new whole `Uint8Array` (MEM-5).  It reads each source in blocks with `FileReaderSync` (as `blob_range` does), through a `MediaSource` with no read larger than 1 MiB.  It removes its `.part` files when it ends with `undecodable` or `unstored`.  On `decoded`, the page, inside `SAVING`, removes the `mix` line if a new mix comes, moves each `.part` file onto its name (as M3's `keep_stored_stems` does), and writes the lines of D2.  If a move or the `meta` write fails, the session stays incomplete, and the page treats it as `unstored` (D9).
* D8, the mix is never written again while it is usable.  When the page sends `mix_frames`, the worker does not write `mix.f32.part`.  For a song of one source it then does not decode at all: the frames are `mix_frames`, and it only makes the overviews from `mix.f32`.  For a song of several sources it decodes the sources, and checks that their frames equal `mix_frames`; if not, it ends with `unstored`.  The page then takes the memory path of D9, and removes the `mix` line and `mix.f32` (`forget_mix`) only if no separation names the session, so the next open decodes again.  So no step of M5 writes a `mix.f32` that a separation may read (TD-55): a separation on the storage path starts only with a usable mix, and the page keeps the mix usable until the separation ends.
* D9, the paths of an open, on the page:
  * The page saves the chosen files with `store::save_sources`, as `Blob`s, before it reads anything.  If that fails (no OPFS, no space), the page reads the files and decodes them on the main thread, as today.  This is the memory path of decision 0015.
  * With a session, the page posts `decode`.  On `decoded` it does D7, then reads the session files back in blocks (D10) and shows the song.  On `undecodable` it shows the error, as today's "Could not load: ...", and forgets the new session.  On `unstored` it reads the stored sources and decodes them on the main thread, as today, keeps the session, and its status line says that browser storage failed.  If the decode worker cannot start, or stops, the page takes that path too.
  * The decode counts as a save of the session (`SAVING`), from the post to the end of D7, so Forget waits for it.  The decode worker's writes count in `WRITES` from the post to the reply, so "Reset…" waits for them, and the reset terminates the decode worker before it deletes storage.
  * The decode worker runs one request at a time, in the order sent; each reply belongs to the oldest decode not yet ended.  If another song opens before a decode ends, the page completes the session (D7) but does not show the older song, as the inbox and `opens` do today.
* D10, the read-back after the decode (until M6 and M7).  The page reads each session file in blocks of 1 MiB (`Blob.slice` and `arrayBuffer`), into two channels reserved at the song's frames.  It never holds a whole file's bytes.  It builds the `Song` as `Song::from_audio` names and orders it.  A song of one source reads `mix.f32`; a song of several sources reads each `decoded/<i>.f32`.  This read stays until M6 and M7 (TD-54).
* D11, a restored session.  `restore_id` reads `meta`.  If the session is complete (D2), the page reads it back (D10), with no decode.  Else it asks the decode worker (D9), with the `mix_frames` of a usable mix (D8).  The stem WAVs and the original lane follow as today (`read_stem`, `with_original`); `read_stem` still reads a WAV whole until M7.  The page no longer reads a source whole, except on the memory path of D9.
* D12, the memory lines.  The page logs `decoder: memory: page wasm N MB as the open starts` before it saves the sources, and `decoder: memory: page wasm N MB as the decode ends` after D7 and before the read-back.  The decode worker logs `decoder: memory: decoder wasm N MB at the decode's start` before it opens a source, and `decoder: memory: decoder wasm N MB at the decode's end` before its last reply.  The words `decoder wasm` differ from `worker wasm`, so `memory_peak` does not take the decode worker's lines as the stem worker's.  The page logs `song kept in browser storage as session <id>` when the song is shown from its session files, so the e2e scenarios that wait for it still wait for a shown song.

### Step 1: the block parts in `engine`

* D4.  Update the module docs of `engine/src/decode.rs`, `engine/src/resample.rs` and `engine/src/song.rs`.
* An MP3 with sound in it for the tests: `engine` may take `rusty_mp3 = "=1.0.0"` as a dev-dependency (already pinned in `worker/Cargo.toml`, DEPS-1).  If its output does not decode, use the hand-made frames of today's tests and record why.
* Tests, native (`cargo test -p engine`):
  * `a_block_decode_gives_the_whole_decode`: a WAV (16-bit and float), a FLAC and an MP3, mono and stereo.  A memory sink gives the samples of `decode_bytes` and `Song::from_audio`, bit for bit; so does a sink that records blocks.
  * `the_block_resampler_gives_to_rate_exactly`: from 48, 22.05, 8 and 96 kHz to 44.1 kHz, with blocks of 1, 7, 1,000 and 131,072 frames and lengths that are not a multiple of the block.  The output equals `to_rate` bit for bit, with the same length.
  * `a_block_decode_keeps_its_memory_flat`: with `PeakAllocator`, the peak above the start for a decode into a sink that keeps nothing differs by less than 1 MiB between a 2-second and a 60-second WAV, FLAC and MP3, at 44.1 and 48 kHz.  A control: `decode_bytes` of the long file peaks more than that bound above the short one.
  * `the_block_overview_gives_lane_columns`: for frames of 1, 4,095, 4,096, 4,097 and 1,000,003, and blocks of any size, the columns and the scale equal `lane_columns`.
  * `the_decode_limit_stops_a_block_decode`: a file over the limit gives the error of `too_long` from `Decoder` too.
* `app` uses `engine::overview` for `lane_columns`; the UI snapshot tests do not change.

### Step 2: the crate, the build and the rules

* The crate of D3 with a `start` entry point that answers no request yet but `log` (or a minimal `decode` that replies `unstored`), so the build and the rules land first.  `decoder/clippy.toml` and its canary test.
* Build: `app/index.html` (the Trunk link), `app/src/reset.rs` (`WORKER_FILES` gets the decode worker's three files, and its test reads both links of `app/index.html`), `xtask/src/publish.rs` (its file list), the comment of `app/Trunk.toml`, and `.github/workflows/ci.yml` (wasm clippy names `-p decode-worker`; the wasm build comment).  `xtask web-test` runs `-p decode-worker --lib`.
* LAYER-1, reworded as decision 0019 asks (the maintainer allows it): "The page side (`app` and `decode-worker`) and the worker side (`stem-worker`) meet only through `stem-protocol`.  The workspace crates that `stem-worker` reaches through normal dependencies, for any target, have only `stem-protocol` in common with those that `app` or `decode-worker` reaches.  In the normal dependency tree of the web app (`app`, `decode-worker` and `stem-worker`), `synth` brings no other crate."  `WEB_APP` gets the crate.  `the_page_and_the_worker_meet_only_through_stem_protocol` checks both page-side crates against `stem-worker`; new unit tests: `decode-worker` and `stem-worker` that share `engine` (or `roformer`) fail, and `decode-worker` and `app` that share `engine` pass.  `synth_brings_no_crate_into_the_web_app` reads three trees.  The layers table, its sentence about the web app, and `LAYERS` if it needs a change.  Keep every existing test of LAYER-1.
* DEPS-1 and `THIRD-PARTY.md`: both commands name `-p decode-worker`, and the tables are generated again; `third_party_lists_what_ships` passes.  DEPS-1's text names the three crates.
* MEM-1: the lint covers `decode-worker` (MEM-1's "Enforced by", decision 0008's count in `docs/ARCHITECTURE.md` or the rules if they give it, and `.claude/rules/*.md` that list the crates).
* `CLAUDE.md` (layout and the wasm clippy command; it stays within 100 lines), `README.md` (layout and commands), `docs/ARCHITECTURE.md` (crates, layers), `docs/working-with-agents.md` if it lists the crates.
* Check: `cargo run -p xtask -- docs-check`, the rules tests, `(cd app && trunk build --release)` makes the three files, and `cargo run -p xtask -- e2e smoke --browser chrome` still passes.

### Step 3: the messages and the decode worker

* `stem-protocol`: D6.  Update the module doc.  Tests in `protocol/tests/web.rs`: each message round-trips through a structured clone with an empty transfer list; each missing or wrong field is a decode error; a reply of the stem worker is not read as a decode reply, and the reverse.
* `decoder/src/`: the request loop, the reads of D7, the decode through `engine` (D4), the second pass (D5), the `.part` files and the removal on failure (D7), D8 and the log lines of D12.  Keep the pass logic over a small trait of session files (for example `read_at`, `write_at`, `len`), so native tests can run it on memory files and the browser tests on OPFS.
* Tests, native (`cargo test -p decode-worker`), on memory files:
  * `one_source_decodes_into_the_mix`: `mix.f32` equals the source's samples interleaved, and the overviews equal `lane_columns` of the source and of `mix_of_sources`' samples.
  * `several_sources_decode_into_padded_files_and_a_mix`: three sources of different lengths and rates, one mono.  Each source file equals that source of `Song::from_audio`, padded; `mix.f32` equals `dsp::mix` of them; each overview equals `lane_columns`.
  * `a_usable_mix_is_not_written_again` (D8): with `mix_frames`, no `mix.f32.part` is written, and for one source no source is read.  `a_mix_of_other_frames_ends_with_unstored`.
  * `a_bad_source_ends_with_undecodable_and_leaves_no_part_file`, `a_failed_write_ends_with_unstored_and_leaves_no_part_file`.
* Tests, browser (`cargo run -p xtask -- web-test`, `-p decode-worker --lib`), with planted sources in OPFS:
  * `decode_writes_pass_whole_arrays`: a spy on `FileSystemSyncAccessHandle.prototype.write` records "whole" or "view" for each call.  Every call is whole, and a song of more than one block makes more than one call (MEM-5).
  * `a_decode_keeps_the_worker_memory_flat`: as `a_stored_mix_keeps_the_worker_memory_flat` of M4, a short and a long WAV give peaks above the start that differ by less than 1 MiB, with a control through `decode_bytes`.  A spy on `FileReaderSync.prototype.readAsArrayBuffer` checks that no read is larger than 1 MiB.
  * `a_wav_a_flac_and_an_mp3_decode_into_session_files`: the files of D1 for each, against `engine`'s whole decode in the same test, bit for bit; the `decoded` reply gives the frames.
  * `a_failed_read_ends_with_unstored`: a source file that changes after the open (as M4's tests do for the mix).

### Step 4: the page

* `app/src/decoder.rs` (new): starts the decode worker, posts `decode`, and turns the replies into events, as `separator.rs` does.  Its tests use a worker with no script, `spy_on_post` and `deliver`, as the separator's tests do.
* `app/src/store_meta.rs`: the `decoded` line (D2).  `app/src/store.rs`: `complete_frames(id)` (the check of D2), the moves and `meta` lines of D7, `read_back` in blocks (D10), and `restore_id` by D11.  Update the module doc's layout.
* `app/src/lib.rs`: the open of D9 for a drop and for the picker (no `bytes_async` and no `picker::read` before `save_sources`), the startup restore and the Storage menu (D11), the memory lines of D12, and the reset (D9).  A status line while the decode runs (for example "Opening "<name>"…").  The memory path keeps today's code.
* Tests, native: `the_decoded_line_reads_back_with_its_format`; a `decoded` line of another format, or a size that differs, gives an incomplete session (the size check as a pure function).
* Tests, browser (`app/src/lib.rs`, `app/src/store.rs`, with a fake decode worker):
  * `an_open_posts_a_decode_and_reads_no_source`: a dropped or picked file is saved, and the page posts `decode` with its session and source, and does not call `arrayBuffer` on the `File` (a spy).
  * `a_decoded_song_is_read_back_and_shown`: with planted `.part` files and a `decoded` reply, the page moves them, writes the `decoded` and `mix` lines, and shows a song equal to the planted samples, for one source and for three.  A mono source reads back on both sides.
  * `the_read_back_makes_no_whole_file_buffer`: as M4's `the_mix_write_makes_no_song_length_buffer`: the page's memory grows by about the two channels, not by the file's bytes as well.
  * `undecodable_shows_the_error_and_forgets_the_session`; `unstored_decodes_on_the_page_and_keeps_the_session`; `no_session_decodes_on_the_page`.
  * `a_complete_session_restores_with_no_decode`; `an_incomplete_session_restores_through_a_decode`, with `mix_frames` for a usable M4 mix.
  * `forget_waits_for_a_decode`.
  * `separate_writes_no_mix_after_a_decode`: after a decoded song of one source, Separate posts `Mix::Stored` with no write to `mix.f32` (the spy of M4's `the_mix_is_written_in_whole_blocks`).

### Step 5: the e2e checks and the docs

* A new scenario `e2e decode`, run in CI's `browser-tests` job (no GPU), with a pure function and a native test for each check:
  * A 44.1 kHz WAV of one source: `mix.f32` and its `mix` and `decoded` lines exist, its SHA-256 equals the native `engine` decode, interleaved, and `overviews/0.f32` and `overviews/mix.f32` equal `engine::overview` of it.  No `decoded/` folder.
  * A 48 kHz WAV of one source: the frames equal the native decode's; the samples equal it within 1e-6 (wasm and native `sin` may differ in the last bit of the filter table).
  * The synthetic stems of `e2e smoke` (several sources): each `decoded/<i>.f32`, `mix.f32` and every overview, against the native decode.
  * A reload restores the last song with no decode (the decode worker logs no start line).
  * No `.part` file remains.
* `e2e memory-budget`, each with a pure function and a native test:
  * The growth of the page from `as the open starts` to `as the decode ends`, and of the decode worker from `at the decode's start` to `at the decode's end`, each at most `MAX_GROWTH_MB`, for each song (both are WAVs of one source).
  * `MemoryPeak` gets the decode worker's peak (`decoder wasm`), in its print and its total.
  * No line `before the mix is written` for these songs of one source: Separate wrote no second mix.
* `e2e separate` fails on a line `before the mix is written` too.
* Run in Chrome on the Mac, with `CHROMEDRIVER` and the weights of the team leader's prompt: `e2e decode`, `e2e smoke`, `e2e storage`, `e2e flac`, `e2e stored-stems --hashes <M3's six>`, `e2e separate`, `e2e memory-budget`, and `web-test`.  Record the numbers in Progress.
* Docs (in the commit that makes each one wrong, at the latest here): `docs/ARCHITECTURE.md` (the pipeline from a song, memory on phones, browser storage: the files of D1 and the `decoded` line), TD-54 (the decode no longer holds the song; the read-back of D10 and `read_stem` stay), MEM-1's and MEM-5's "Enforced by" (the new tests), the README's scenario list, and `docs/references/browser-quirks.md` if a browser surprises.

## Validation and acceptance

The parent's M5 acceptance, check by check:

* "A test decodes a WAV, a FLAC and an MP3 from a block source.  A memory sink and a block sink get the same samples": `a_block_decode_gives_the_whole_decode` (CI `test`), and `a_wav_a_flac_and_an_mp3_decode_into_session_files` (CI `browser-tests`).
* "A browser test opens a song and finds its session files and overviews in OPFS": `e2e decode` (CI `browser-tests`).
* "For a song of one source, it finds `mix.f32` and its `meta` line, and Separate then writes no second mix file": `e2e decode`, `separate_writes_no_mix_after_a_decode` (CI `browser-tests`), and `e2e separate` and `e2e memory-budget` with the real model.
* "The MEM-5 spy covers the decode writes": `decode_writes_pass_whole_arrays` (CI `browser-tests`).
* "`e2e memory-budget` shows no growth of the page or the decode worker for a WAV source, from the start of the open to the decode's end line": the scenario passes with both growths at most 4 MB at 1 and 4 minutes.  WebAssembly memory never shrinks, so the check can miss a small allocation; `a_block_decode_keeps_its_memory_flat` and `a_decode_keeps_the_worker_memory_flat` check it exactly.
* Every check of GIT-2 passes, and `cargo run -p xtask -- web-test --browser chrome`.

## Idempotence and recovery

* Each step is one or more commits that pass every check.  A step can be reverted alone, from the newest back.  Step 2 alone adds a crate that the page does not yet use.
* A decode that stops leaves `.part` files, which nothing reads, and no `decoded` line.  The next open of the session decodes again and replaces them.  Forget removes them with the session.
* A session of M4 or older has no `decoded` line.  Its first open in M5 decodes it, and keeps a usable `mix.f32` (D8).
* If the decode worker fails, the page decodes on the main thread as today (D9), so an open never depends on the new worker alone.

## Progress

* [ ] 2026-10-26: milestone plan written.

## Surprises and discoveries

None yet.

## Decision log

* 2026-10-26, team leader: D5, the overview is made in a pass after the decode, not in the decode's pass.  Its columns depend on the song's frames, which an MP3 does not state.  The pass reads files in blocks, so it keeps the parent's aim: no song-length memory.
* 2026-10-26, team leader: D1 and D4, the block decode gives today's samples bit for bit, so the format tag stays `f32le-1`, and an M4 `mix.f32` stays usable (D1 of M4 allows this).
* 2026-10-26, team leader: D1, every session file of a song holds the song's frames, padded with silence.  So M6 and M7 read every lane in the same way.
* 2026-10-26, team leader: D2 and D7, the page stays the only writer of `meta`, and moves the worker's `.part` files into place, as in M3.  A sync access handle and a writable stream on one file would lock each other.
* 2026-10-26, team leader: D8, the decode never writes a usable mix again.  This keeps TD-55 out of M5: no step rewrites `mix.f32` while a separation names the session.
* 2026-10-26, team leader: D9, when storage is unavailable or fails, the page decodes on the main thread as today (the memory path of decision 0015).  When a source cannot be decoded, the page shows the error and does not try again.
* 2026-10-26, team leader: D11, "`store::read` stops reading whole files" (parent's M5) applies to the sources.  `read_stem` still reads a stem WAV whole until M7, whose section covers a restored session's files.  `write_mix` (M4) still reads the song's samples; after M5 it runs only for a session with no usable mix, and M7 must replace it before it removes the samples from the page.
* 2026-10-26, team leader: D3, the crate is `decode-worker` in `decoder/`.  Its messages live in `stem-protocol`, the one crate of messages (PROTO-1), with their own `kind` constants.

## Outcomes and retrospective

Not started.
