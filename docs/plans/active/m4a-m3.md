# M4A, M3: save as ALAC M4A

## Purpose

This is the milestone plan for M3 of `plans/active/m4a.md`.  The parent's M3 section stays the criteria.  After M3, the Save menu of a stem lane and of the mix lane has "M4A (16-bit Apple Lossless)".  The stem worker encodes the save as ALAC with the `m4a` crate, on the storage path and on the memory path.  Logic Pro and GarageBand then take a saved stem directly (the parent's Purpose).

Readers: the implementer, the review agents of M3's rounds, the orchestrator, and the team leader of M4.

## Context and orientation

The state at the start, on 2026-11-10, at commit `80a3e37` on branch `feature/m4a`.  The review base of M3 is `b8b2c95`, which the maintainer named.  Commit `2193806` holds only plan text, and `80a3e37` only a fix of `references/local-browser-testing.md`.  M1 and M2 are done (`plans/completed/m4a-m1.md`, `plans/completed/m4a-m2.md`).  M3 runs on an Apple M2 Pro (arm64), not on the Intel Mac of `references/local-browser-testing.md`.

### The decisions that M3 follows

All are in the parent's "Decisions for the maintainer" and Decision log.

* D1 (a): ALAC in M3.  AAC through WebCodecs is M4.
* D2 (c): the encoder is the safe port in `m4a` (`AlacEncoder`).
* D6 (a): `m4a` only makes bytes.  `stem-worker` does the writes, as for FLAC.  The worker makes the interleaved 16-bit samples with `pcm16`.  The worker writes the head twice: zeros to reserve its size, then the final head at position 0.  M3 makes `FlacOut` a sink for both codecs, with `put` and `replace_head`.  It does not copy it as an `AlacOut`.  Every test that needs an M4A uses `m4a` (TD-52).
* Decision 0028 (accepted): M4A with ALAC as the save format.
* Decision 0029 (`proposed`): `m4a` is Apache-2.0 only.  From M3 the web app ships its code.  M3 does the Consequences of 0029.  The maintainer accepts 0029 at the review of the PR; the run does not wait.
* The maintainer's rules for M3, from the parent's Decision log of 2026-11-10.  Each review round has them in its criteria, word for word.
  * Measure a gap against what the plan promises for M3, not against main.  Main has no M4A save, so "same as main" is never a reason for a gap.
  * From round 4, a round adds no new behaviour.  It fixes only what it finds, and the fixes stay small.
  * A gap that no user of the app can reach goes to a TD with a true reason and a clear text, unless it puts memory at risk or makes a panic.
  * Never leave user-facing text or behaviour wrong to avoid a snapshot or fixture update.  Update the artefact by the README process, and list it in the PR.
  * In late rounds, do not push the team to close gaps on paths the app cannot reach.

### The inputs that the app gives an encode

M2's retrospective asks each milestone to name the inputs that the app reaches, at the start.  A gap on any other input goes to a TD from round 1, unless it puts memory at risk or makes a panic.

* Paths: the storage path (the default: a 16-bit stereo WAV in `saves/`, `Encode::Stored`), and the memory path (two transferred `Float32Array`s, `Encode::Arrays`).  The memory path runs only when browser storage fails, for a song of at most `app::MEMORY_PATH_MAX_FRAMES` frames.
* The rate is always 44,100 Hz (decision 0012).  The channels are always 2.
* A save always has 1 frame or more.  A song is at most the 1 GiB decode limit (decision 0013), about 134 million frames.  So the worst-case ALAC file is about 537 MB, far below the 4 GiB limit of `Layout::new`.
* So a rate other than 44,100 Hz and 0 frames are not reachable from the app.  M3 gives each a clear error text and one test, and adds no other behaviour for them.
* The worker cannot reach the 4 GiB refusal either: `Input::wav` checks the WAV's size first, and the memory path would need more than 10^9 frames.  `a_file_that_can_reach_4_gib_is_refused` in `m4a/tests/mp4.rs` tests it.  M3 maps the error to a text, with no worker test.

### The code today

* `worker/src/web/encode.rs`:
  * `Codec` (`Mp3`, `Flac`) with `kind()` and `reply()`.
  * `Input` (`Arrays` or `Wav`) reads one block at a time into two `f32` slices.  On the WAV path, each read makes a `Vec<u8>` of the block (`bytes.to_vec()`, 16 KiB for 4096 frames).  `pcm16` (in `worker/src/web.rs`) gives an `i32`, rounded as `engine::wav::encode_pcm16` does.
  * `OutFile` writes through one `FileSystemSyncAccessHandle`; `write_at` makes a new `Uint8Array` for each write (MEM-5).
  * `FlacOut` (`Parts` of a JS array, or `File`) has `put` and `replace_header`.  `replace_header` sets part 0, or writes at byte 0.
  * `encode` picks the path, and removes the WAV after a stored encode.  Its default name is `stem.<kind>`, so a kind `alac` gives a wrong name (see Step 2).  `stored` opens the WAV and the output, and removes the output on a failure.
  * `flac_into` reads one block of `FLAC_BLOCK` frames, converts it with `pcm16`, encodes and puts it.  `flac_blob` makes a `Blob` of type `audio/flac` from the parts.
  * `log_memory` logs `memory: encode <kind> <what>: wasm <before> to <after> bytes, file <bytes> bytes`.
* `worker/src/web.rs`: `serve` dispatches `Request::Mp3` and `Request::Flac` to `encode`.  The encode kinds are not in the `queued` list of `handle`, so an encode runs beside a separation.
* `worker/Cargo.toml`: no `m4a`.  The dev-dependency `symphonia` has the feature `flac` only.
* `worker/src/web/tests.rs`:
  * `decode_flac` (Symphonia, with `expect` on the probe), and the FLAC and MP3 tests of both paths.
  * The counting allocator (`LIVE`, `PEAK`) and `stored_peak`.
  * The helpers:
    * `last_file` matches `Mp3 | Flac` only;
    * `arrays_file` sends a `Blob` to `flac_reply`, which asserts `audio/flac`;
    * `stored_peak` writes `SAVE_MP3` for any codec but FLAC;
    * `encode_stored` names the file by `codec.kind()`.
  * The file imports `std::alloc::Layout`, which has the same name as `m4a::Layout`.
* `protocol/src/lib.rs`: `kind::MP3`, `kind::FLAC`, `Request::Mp3`, `Request::Flac`, `Reply::Mp3`, `Reply::Flac`, `Encode`, `Encoded`, and the wire text at the top of the file.  `protocol/tests/web.rs` round-trips each one through a structured clone.
* `app/src/separator.rs`: `encode_mp3`, `encode_flac`, `Event::Mp3`, `Event::Flac` and their tests.  The `match` of a reply into an `Event` has no wildcard arm, so a new `Reply` variant fails to compile without its arm.  The doc of `settings` says "Sent with every request but MP3 and FLAC encoding".
* `app/src/export.rs`: `Format` (`Wav`, `Flac`, `Mp3`), `Format::ALL`, `extension`, `label`, `needs_worker`, `JsSink`.  The test `only_wav_is_made_without_the_worker` checks the extensions `["wav", "flac", "mp3"]`.
* `app/src/lib.rs`:
  * `save_type` gives the download type of a stored save.
  * `save_menu` makes one button for each item of `Format::ALL`.
  * The `Event::Mp3 | Event::Flac` arm offers a file.  The `Event` match has no wildcard arm.
  * The `Event::Error` arm names "MP3 encoding" for the kind `mp3`, else "FLAC encoding", inside the arm of `MP3 | FLAC`.
  * The stored path picks the request with `if order.format == export::Format::Flac { encode_flac } else { encode_mp3 }`.  A third worker format would go to the MP3 encode, so this becomes a `match`.
  * `end_memory_save` matches each `JsSink`.
* `app/src/save_tests.rs`: the browser tests of a save, with loops over `[Format::Flac, Format::Mp3]`.
* `xtask/src/e2e/save.rs`:
  * `FORMATS` (`wav`, `flac`, `mp3`, with their rows in the Save menu), `FLAC_BOUND_BYTES`, `Files`, `reads_back`, `file_checks`, `saves_folder_check` (7 files), and their unit tests.
  * It finds the worker's line by `memory: encode {format} `, so the format name must equal the worker's kind.
  * `WAITS_S` sums every wait of a run.  It grows with `FORMATS.len()`.  `const _: () = assert!(WAITS_S < STEP_LIMIT_S)` checks it at compile time.  `the_waits_are_the_sum_of_each_wait` asserts `WAITS_S == 2_330` and `STEP_LIMIT_S - WAITS_S < 300`.  `STEP_LIMIT_S` is 40 minutes, and `the_ci_step_limit_is_the_waits_limit` keeps it equal to the `timeout-minutes` of the `e2e save` step in `.github/workflows/ci.yml`.  The `browser-tests` job has 60 minutes.
* `xtask/src/e2e.rs`: the module doc names the formats of `save`.
* `xtask/src/check.rs`: `fixture()` holds the crate graph of `check --tier`, and `the_fixture_graph_is_the_real_graph` compares it with `cargo metadata`.  `merges_into_engine` and `crate_tests_of_engines_family_do_not_run_engine_alone` use the dependents of `m4a`.
* `m4a/src/mp4.rs`: the module doc names `FlacOut::replace_header`.
* `docs/ARCHITECTURE.md`:
  * the layers table row of `stem-worker` (`roformer`, `roformer-gpu`, `stem-protocol`), and its line in the crate list;
  * the pipeline diagram node "MP3, FLAC" (line 18), and the save text of "memory on phones" (line 153);
  * the save text of "browser storage" (lines 240 and 242), the save diagram (lines 258 to 260), and the `saves/` line (line 287).
* `THIRD-PARTY.md`: the note on `m4a` says "no shipped crate takes `m4a` yet".  `third_party_lists_what_ships` leaves workspace members out of the tables.
* `docs/decisions/0029-m4a-is-apache-2-only.md`, Consequences: "When a shipped crate takes `m4a` (M3), the app ships Apache-2.0 code.  The licence list of the app must then give `m4a` as Apache-2.0."
* `README.md`: line 3 and step 4 of "How to use" name "WAV, FLAC or MP3"; the `e2e` list describes `save`; the licence section names `m4a/` as Apache-2.0 only.
* `docs/tech-debt.md`: TD-53 names the arms of `Event::Mp3` and `Event::Flac` and the "FLAC encoding" and "MP3 encoding" labels as untested.  TD-54 names the FLAC and MP3 encodes of a save.  The next free TD number is 91.

### The `m4a` API that M3 uses

* `Layout::new(m4a::Codec::Alac, frames as u64, sample_rate, None, Order::FastStart)`.  It refuses a rate of 0 or above 65,535, and a file that can reach 4 GiB (`Error`, with a `Display` text).  The worker's own `Codec::Alac` has the same name, so write the path of each.
* `layout.packets()`, `layout.head_len()`, `layout.head(&SampleEntry::Alac { cookie }, &sizes)`.  `head().len()` equals `head_len()`.  `head` refuses a count of sizes that is not `packets()`.
* `AlacEncoder::new(sample_rate)`, `encode(interleaved: &[i16], &mut out)` appends one packet of 1 to 4096 frames and gives its size.  It panics on 0 frames, an odd length or more than 4096 frames.  `out` grows by at most `MAX_PACKET_BYTES` (16,388).
* `cookie()` is final only after the last packet.  So the head is final only at the end.
* `alac_file` holds the whole file in memory.  The worker never calls it (M1's Decision log, 1-12).  Tests use it as the reference file.

### Memory of the encode

The parent's M3 acceptance fixes the allowance.  The worker keeps one `u32` size for each packet: 4 bytes for each 4096 frames.  `head()` makes `moov` with about 8 bytes for each packet (`stsz` and `stco`).  The two live together at the end, about 12 bytes for each packet.  For 10 minutes at 44.1 kHz that is 6,460 packets, about 77.5 KB.  The rest of the encode is fixed: the encoder's buffers, the two `f32` blocks, one interleaved block, one packet buffer and one read of the WAV.

* `Vec::with_capacity(layout.packets())` for the sizes, so the vector never grows by doubling.
* The placeholder of `head_len()` zeros goes out in small fixed blocks, not as one buffer.
* The packet buffer is made once with `MAX_PACKET_BYTES` of capacity, and cleared after each packet.
* Do not stream `moov`.
* Where the peak is (plan review round 2).  In the loop, about 188 KB is live.  It holds the encoder's buffers (4 x 16 KiB and `TRY_BYTES` of 40,961).  It also holds the two `f32` blocks, the interleaved block, the packet buffer and one WAV read.  At the end, the block buffers are gone, and about 106 KB plus 12 bytes for each packet is live.  So the end is the peak only above about 10,000 packets, about 15.7 minutes.
* So at 10 s and 60 s the peak is in the loop, and it grows by the sizes only: 4 bytes for each packet.  A vector that doubles holds up to 12 bytes for each packet while it grows (the old and the new buffer).  So a bound of 4 bytes for each packet tells `with_capacity` from doubling.  Step 3 asserts both bounds.
* Step 1's own test pins the blocks of the reserve, and that its zeros are not in WebAssembly memory.

### Terms

* Head: `ftyp`, `moov` and the 8-byte `mdat` header, before the first packet (fast start).
* Reserve: `head_len()` zero bytes, written before the first packet, which the final head replaces.
* Packet table: the sizes of the packets, which the worker keeps until the end.
* Sink: where an encode puts its bytes: the parts of a `Blob` (memory path), or the output file (storage path).

## Plan of work

Each step is one signed commit, after `cargo run -p xtask -- check --tier commit` (GIT-2).  Each new test fails first on an assert, then passes.  Record each first failure in the Progress line of its step.  Stage each new file by name (`git add <path>`).  Never run `git add -A`.  Each step fixes, in its own commit, each doc that it makes wrong (DOC-1).  Read each new doc line, Progress line and TD for sentences of more than 25 words before the commit.

How a new test fails first on an assert:

* A stub that gives a wrong value, such as an empty file, makes the first run fail on an assert, not on `todo!()` or a panic in `unwrap`.
* A test that compares a file asserts its bytes against `alac_file` before it decodes the file.  A decode helper gives a `Result`, and the test writes `assert!(result.is_ok(), "{result:?}")` before it unwraps.
* A test of a refusal asserts on the result (`assert!(result.is_err(), ...)`, then on the text), not with `expect_err`.
* Each test gets the stub that makes it fail.  For example, a stub that skips `Layout::new` and writes no reserve makes the 65,536 Hz test fail.
* A new enum variant cannot fail on an assert before it exists.  So add `kind::ALAC`, `Request::Alac` and `Reply::Alac` first with a stub that writes the kind `flac`.  The round trip first asserts `kind_of(&msg) == "alac"`, a literal, so it fails on that assert, not on a later destructuring.
* A test that takes one event or reply from a list asserts `is_some()` first, then unwraps.  For example, the copy of `a_flac_reply_reaches_the_page_as_sent` for ALAC.

No step changes `CLAUDE.md` or `.claude/**`, by the team leader's reading on 2026-11-10.  The `m4a` line of the Layout still holds, and the crate lists of `.claude/rules/` already have `m4a`.  If a step needs such a change, the implementer stops and tells the team leader, who gives the orchestrator a step checkpoint.

The browser runs on this Mac: `CHROMEDRIVER=/Users/lloyd/.local/chromedriver-157/chromedriver-mac-arm64/chromedriver`, and `--browser chrome` for `web-test` and `e2e`.  `web-test` runs Safari by default on macOS, so always pass `--browser chrome`.  End each stale WebDriver session before a retry.  Never quit the maintainer's Safari.

Before Step 1, measure the wasm bundle at `2193806` with `(cd app && trunk build --release)`.  Record the three sizes of `app/dist/*_bg.wasm` in Surprises.

### Step 1: one sink for FLAC and ALAC

* Rename `FlacOut` to a name of both codecs, such as `EncodeOut`.  Rename `replace_header` to `replace_head`.  Keep `put`.
* Add `reserve(len)`: it puts `len` zero bytes as one unit that `replace_head` replaces.
  * `Parts`: push one `Uint8Array::new_with_length(len)`.  The zeros are in JS, not in WebAssembly memory.
  * `File`: write the zeros in blocks of at most 64 KiB.  Each block is a whole new `Uint8Array::new_with_length(n)`, made in JS, so WebAssembly memory holds no zeros (MEM-5).  `OutFile` gets a write of a JS array for this.
  * `replace_head` keeps its rule: part 0, or byte 0.  So the reserve must be the first put of an encode.  Its doc says so.
* `flac_into` calls `reserve(header_bytes)` in place of its first `put` of the stream header.  It makes the first header only for its length.  So `reserve` has a caller in Step 1, and the wasm32 clippy line sees no dead code.  The final bytes do not change.  The write record of a stored FLAC stays the same (`w42@0` first and last).
* The FLAC tests are the guard of this change: `a_stored_flac_is_the_flac_of_its_16_bit_samples`, `stored_encodes_write_whole_arrays_as_they_read` and `a_flac_decodes_to_the_stem` pass with no change of their asserts.
* A browser test of `reserve` on each sink: a reserve, then a `put` of a marker, then `replace_head`.
  * On the `File` sink, assert the exact write record of the spy of `stored_encodes_write_whole_arrays_as_they_read` (`start_order_spy`).  For a reserve of 64 KiB + 1 bytes: `w65536@0 w1@65536`, then the marker's write, then `w65537@0`.  Assert the file's bytes.  Run it also for a reserve of 64 KiB, which is one write.
  * The bound of 64 KiB holds for the writes of the reserve, not for the head.
  * On the `Parts` sink, assert the parts: one part of the head's length, then the marker.
  * A reserve of 1 MiB raises `PEAK` of the counting allocator by less than 1 KiB, on each sink.  So its zeros are not in WebAssembly memory.
  * The stub of the first failure: a `reserve` that writes nothing.
  * The test needs `EncodeOut`, `OutFile::open`, `reserve` and `replace_head` at `pub(super)`.
* `m4a/src/mp4.rs`: the module doc names the new sink and `replace_head`.  Decision 0028 keeps its text, since a decision record is history.
* `docs/invariants.md`, MEM-5 "Enforced by": the new test of the reserve.
* Acceptance: `cargo run -p xtask -- web-test --browser chrome` passes the FLAC tests and the new test, which fails first with a stub `reserve` that writes nothing.

### Step 2: the wire kind `alac`, the worker's encode and the page's reply

The wire, the worker and the page's side of the reply change in one commit, so that each message has its encoder and decoder on both sides (PROTO-1).  A new `Reply` variant also needs its arm in the page's exhaustive matches, or the wasm32 clippy line of `check --tier commit` fails.

* `protocol/src/lib.rs`: `kind::ALAC = "alac"`, `Request::Alac { name, source }` and `Reply::Alac { name, file }`, as `Flac` has.  The same `Encode` and `Encoded`.  The wire text at the top names `alac` with `flac`.  The docs of `Encode`, `Encoded` and the two source and file helpers (lines 345, 367, 876 and 928) name ALAC.
* `protocol/tests/web.rs`: each round trip of `flac` also runs for `alac`: the request on both paths, and the reply with a `Blob`, `Stored` and `Failed`.  The refusal cases (no file, a `path` that is not text) run for `alac` too.
* `protocol/src/decode.rs`: `no_type_is_a_type_of_the_stem_worker` lists `kind::ALAC`.
* TD-16: no hand-written script of `xtask e2e` writes or reads the kind `alac` in this step.  `e2e_messages_use_protocol_types` passes.  Step 5 reads only the worker's `memory: encode alac` log line.  The implementer checks this with `git grep -n "'flac'\|\"flac\"" xtask/`, and records it in the Progress line.
* `worker/Cargo.toml`: `m4a = { path = "../m4a" }` in the wasm32 dependencies.  The dev-dependency `symphonia` gets the features `isomp4` and `alac`, so the tests decode the worker's M4A with a second implementation.
* `worker/src/web.rs`: `serve` sends `Request::Alac` to `encode(Codec::Alac, ...)`.  `handle` keeps `alac` out of the `queued` list, as `flac`.
* `worker/src/web/encode.rs`:
  * `Codec::Alac`, with `kind()` `alac`, `reply()` `Reply::Alac`, and a new `extension()`: `mp3`, `flac` or `m4a`.  The default name of `encode` uses `extension()`, so it is `stem.m4a`, not `stem.alac`.
  * `alac_into(input, rate, out: &mut EncodeOut)`:
    1. Refuse 0 frames with "ALAC encoder: the stem has no samples", as FLAC does.
    2. `Layout::new(m4a::Codec::Alac, frames as u64, rate, None, Order::FastStart)`.  Map its error to "ALAC encoder: <Display text>".
    3. `out.reserve(layout.head_len())`.
    4. `sizes = Vec::with_capacity(layout.packets())`, a packet buffer with `MAX_PACKET_BYTES` of capacity, the two `f32` blocks of `FRAMES_PER_PACKET` for `Input::read`, and one interleaved `i16` buffer of `2 x FRAMES_PER_PACKET`.
    5. For each block of `FRAMES_PER_PACKET` frames: `input.read`, `pcm16(x) as i16` for each sample into the interleaved buffer, `encoder.encode`, `out.put` the packet, push its size, clear the packet buffer.
    6. Drop the block buffers.  `layout.head(&SampleEntry::Alac { cookie: encoder.cookie() }, &sizes)`.  Check that its length is `head_len()`, else an error text as FLAC's "the stream header grew".  Then `out.replace_head`.
  * `alac_blob`: a `Blob` of type `audio/mp4` from the parts.  `stored` and `encode` take `Codec::Alac` as they take `Codec::Flac`.
  * The module doc names ALAC, its head order and its packet table.
* `app/src/separator.rs`: `Reply::Alac` becomes `Event::Alac { name, file }`.  A test as `a_flac_reply_reaches_the_page_as_sent` checks that the event keeps the worker's `Blob`.
* `app/src/lib.rs`: the `Event::Alac` arm offers the file as `Event::Flac` does.  The menu does not offer M4A yet (Step 4).
* `worker/src/web/tests.rs`.  The helpers:
  * `decode_m4a(bytes)` gives a `Result` of the rate and the `i16` channels, through Symphonia.
  * `SAVE_M4A`, and an `m4a_reply` that asserts the `Blob` type `audio/mp4`.
  * `last_file` matches `Reply::Alac`.  `arrays_file` sends a `Blob` to the reply helper of its codec.  `stored_peak` takes the output path of its codec.  `encode_stored` names the file by `codec.extension()`.
  * Import `m4a::Layout` by path, since `std::alloc::Layout` has that name.
* `worker/src/web/tests.rs`.  The tests; each compares the file's bytes with `alac_file` of the same 16-bit samples before it decodes:
  * `an_alac_decodes_to_the_stem` (memory path): the file equals `alac_file`, byte for byte.  Symphonia gives the 16 bits of `pcm16` of the input, at 48,000 Hz and 44,100 Hz.  The reply's `Blob` has type `audio/mp4`, and the name is as sent.
  * Lengths 1, 4095, 4096, 4097 and `3 x 4096 + 7` frames, on both paths.  Each equals `alac_file`, and each decodes bit for bit.
  * `a_stored_alac_is_the_alac_of_its_16_bit_samples`: as the FLAC test.  The reply names the output, the worker removes the WAV, the stored file equals the arrays file, and the memory line names the file's size.
  * `an_alac_clips_at_full_scale_and_rounds`: as the FLAC test.
  * `an_alac_of_unequal_channels_stops_at_the_shorter`: as the FLAC test.
  * `an_alac_of_no_samples_is_refused`.  An encode at 65,536 Hz gives an error with the text of `Layout`.  The app reaches neither input ("The inputs that the app gives an encode").
  * The order of writes of a stored ALAC, in `stored_encodes_write_whole_arrays_as_they_read`:
    * first the reserve at 0, in blocks of at most 64 KiB;
    * then a read and a packet write for each block;
    * last the head at 0, of `head_len()` bytes.  No write is a view (MEM-5).
  * The failure cases of `a_failed_stored_encode_says_why_and_leaves_no_file` and `a_failed_stored_encode_leaves_another_encode_whole` run for `Codec::Alac` too.
  * `a_stored_encode_outside_saves_or_malformed_is_refused` and `errors_name_the_request_they_answer` cover `alac`.
* `xtask/src/check.rs`: the edge `stem-worker -> m4a` in `fixture()`.  `stem-worker` becomes a dependent of `m4a` outside the family of `engine`.  So the crate tests of `m4a` then run `engine` alone and `-p stem-worker`.  `crate_tests_of_engines_family_do_not_run_engine_alone` would then assert what its name denies.  Keep that test on a member of the family other than `m4a`, such as `dsp`, and add a test of the `m4a` case.  The docs of `merges_into_engine` change with it.
* Docs of this step (DOC-1):
  * `docs/ARCHITECTURE.md`: the row ``| Shells | `stem-worker` | `m4a`, `roformer`, `roformer-gpu`, `stem-protocol` |``, and the line of `stem-worker` ("MP3, FLAC and ALAC encoding").  The worker's texts (lines 18 and 153) name ALAC, with its packet table.  The page's save flow (lines 240, 242 and 258 to 260) changes in Step 4.
  * `docs/invariants.md`: MEM-1 "Enforced by" names the ALAC encode tests beside those of MP3 and FLAC.  MEM-5 "Enforced by" names the ALAC write-order test.  DEPS-1 "Gaps" gets the new licence TD.
  * Licence, by decision 0029.  `THIRD-PARTY.md`: the note on `m4a` says that `stem-worker` takes `m4a`, so the web app ships it under Apache-2.0 only.  The first paragraph names the workspace crate `m4a` as Apache-2.0 only.  `README.md`, licence section: the web app ships `m4a/`.  Decision 0029, Consequences: the M3 line says what M3 did, in the present tense.  Its status stays `proposed`.
  * A new TD entry, "the published page ships no licence text" (see the Decision log of 2026-11-10).  Apache-2.0 section 4(a) asks that each recipient of the Work in Object form gets a copy of the licence ([Apache License 2.0](https://www.apache.org/licenses/LICENSE-2.0)).  `rusty_mp3`, `flacenc` and now `m4a` ship as WebAssembly, and `gh-pages` holds no `LICENSE*` or `THIRD-PARTY.md`.  The fix: copy the licence texts and `THIRD-PARTY.md` into the build, in their own PR.
  * TD-53: the `Event::Alac` arm joins the untested arms.  TD-54: the stem worker encodes ALAC from the save's WAV in blocks too.
* Acceptance: `cargo run -p xtask -- web-test --browser chrome` passes the new tests of `worker`, `protocol` and `app`, each failing first.  `cargo test -p xtask --test rules` passes, with `third_party_lists_what_ships`, `crates_depend_only_downwards` and `the_page_and_the_worker_meet_only_through_stem_protocol`.  `cargo test -p xtask --lib` passes `the_fixture_graph_is_the_real_graph` and the engine family tests.

### Step 3: the memory of the encode

* A browser test `a_stored_alac_holds_only_its_packet_table`, beside `a_stored_flac_keeps_the_worker_memory_flat`.  It runs `stored_peak(Codec::Alac, 10)` and `stored_peak(Codec::Alac, 60)`.  `Δ` is `packets(60) - packets(10)`, from `Layout`.  It asserts two bounds on the growth of the peak, each with a fixed slack of 1 KiB:
  * the parent's allowance, `12 x Δ`;
  * the bound of the loop, `4 x Δ`, since at these lengths the peak is in the loop ("Memory of the encode").
* The same test on the memory path, with `arrays` of 10 s and 60 s, as `a_flac_is_encoded_in_blocks` does.  JS holds the input and the parts, so the WebAssembly peak must follow the same bounds.
* A stored ALAC of 30 s holds less than a tenth of one channel, as the FLAC test asserts.
* Record the measured growth of each path in the Progress line.
* The first failures, on both paths:
  * a stub that also keeps each packet in a `Vec<u8>` of WebAssembly memory, as `alac_file` does, fails both asserts;
  * a stub that makes the sizes with `Vec::new()` fails the bound of `4 x Δ`.
* If a stub does not fail on a path, stop and tell the team leader with the numbers.  Do not loosen a bound to pass.
* Acceptance: `cargo run -p xtask -- web-test --browser chrome` passes the tests, which failed first.

### Step 4: the page offers M4A

* `app/src/export.rs`: `Format::M4a`, extension `m4a`, label "M4A (16-bit Apple Lossless)", `needs_worker` true.  `Format::ALL` is `[Wav, Flac, M4a, Mp3]`: the lossless formats first (Decision log).  `JsSink::M4a(JsChannels)`.  `only_wav_is_made_without_the_worker` lists `["wav", "flac", "m4a", "mp3"]`, asserts `Format::M4a.needs_worker()`, and asserts the labels of `Format::ALL` in order (VERIFY-1: no test checks a label today).  The wasm32 test `a_mix_export_does_not_grow_wasm_memory` matches `JsSink` with no wildcard arm, so it gets the `M4a` arm.
* `app/src/separator.rs`: `encode_alac(name, source)` sends `Request::Alac`.  A test as `encode_flac_asks_the_worker_for_a_flac_of_the_arrays`.  The doc of `settings` names M4A encoding.
* `app/src/lib.rs`:
  * `save_type(Format::M4a)` is `audio/mp4`.
  * The stored path picks the request with a `match` on the format, with no `else` arm that sends MP3.
  * `end_memory_save` sends `JsSink::M4a` to `encode_alac`, and its arm of no worker (`None`) takes `JsSink::M4a` too.
  * The `Event::Error` arm names "M4A encoding" for the kind `alac`, and clears the save notes, as for MP3 and FLAC.
  * The doc of `can_encode` names M4A.
* `app/src/save_tests.rs`:
  * Each loop over `[Format::Flac, Format::Mp3]` also runs `Format::M4a`, with the download type `audio/mp4` and the kind `alac`.
  * A test that the stored M4A save posts `Request::Alac`, not `Request::Mp3`.  It fails first with the old `if ... else`.
  * A test that a reply `error` of `alac` gives a status line that starts with "M4A encoding failed".  It fails first with the old arm, which gives "Worker request failed".
  * The doc comments that name "FLAC or MP3" (lines 587 and 1006) name M4A.
* `xtask/src/e2e/save.rs`: the row of `mp3` in `FORMATS` becomes 3, so `e2e save` clicks the right item until Step 5.
* `app/src/store.rs`: the `saves/` doc names `.m4a`.
* `docs/ARCHITECTURE.md`: the page's save flow (lines 240, 242 and 258 to 260) and the `saves/` line (line 287) name M4A.  `README.md`: line 3 and step 4 say "WAV, FLAC, M4A or MP3".  `docs/tech-debt.md` TD-53: "M4A encoding" joins the labels.  `docs/invariants.md`, MEM-2: the test `flac_and_mp3_saves_post_paths_then_offer_the_workers_file` gets an M4A case; rename it if its name then misleads, and name it so there.
* Acceptance: `cargo test -p app` passes `only_wav_is_made_without_the_worker`, which fails first on the old list.  `cargo run -p xtask -- web-test --browser chrome` passes the new `app` browser tests, each failing first.  `cargo run -p xtask -- e2e save --browser chrome` passes, since a fourth item can move the menu's items (`item_y` in `xtask/src/e2e/save.rs`).

### Step 5: `xtask e2e save` saves M4A

* `xtask/src/e2e/save.rs`:
  * `FORMATS` gets the worker's kind beside each format and its row: `("wav", 0, None)`, `("flac", 1, Some("flac"))`, `("m4a", 2, Some("alac"))`, `("mp3", 3, Some("mp3"))`.  The rows follow `Format::ALL`.  The line `memory: encode {kind} ` is found by the kind, not by the extension.
  * The worker check of `m4a`: the growth for 10 minutes is at most `FLAC_BOUND_BYTES` (1 MiB) above that for 1 minute, the limit of FLAC.  Rename the constant if its name then misleads, such as to `LOSSLESS_BOUND_BYTES`.
  * The heap, WebAssembly, slice and read checks already run for each item of `FORMATS`.
  * `reads_back`: the M4A files of 1 and 10 minutes are read whole.  `Files` keeps both.  `file_checks`: each M4A decodes through `engine::decode_bytes(bytes, Some("m4a"))` to `planted_mix(seconds)`, bit for bit (`decodes_to`).  The planted stems are slow sines, so the 10-minute M4A is far smaller than its WAV.  Record its size in Surprises.  If the read of the 10-minute file passes `READ_WAIT_S`, read it in parts with a script that takes an offset, and record that.
  * `saves_folder_check`: 9 files, with 2 of `.m4a`.
  * `Files` keeps the M4A bytes in their own fields.  `keep` gets an arm of `m4a`, since its `_` arm now sends anything that is not `wav` or `flac` to `mp3`.  The doc of `Files` no longer says "1-minute files" only.
  * The waits.  Two more saves add 250 s of waits and 240 s of caps.  The read of each `reads_back` file adds `READ_WAIT_S`, so 5 reads, not `FORMATS.len()`: 60 s more.  `WAITS_S` goes from 2,330 s to 2,880 s.  So `STEP_LIMIT_S` becomes 50 minutes (3,000 s), within 300 s of the waits.  `WAITS_S` counts the reads by `reads_back`.  `the_waits_are_the_sum_of_each_wait` counts 8 saves, 4 formats of caps and 5 reads, and asserts 2,880.
  * The save caps.  `save_cap_s` is 6 times the slowest save, and `SLOWEST_SAVE_S` holds FLAC's times.  Compare the M4A save times of the run with them.  If M4A is slower, update `SLOWEST_SAVE_S`, the caps and their docs, and the waits with them.
  * Docs that count the saves or name the formats:
    * the module doc ("A seventh save" becomes a ninth);
    * the doc of `file_checks`;
    * the doc of `CLEAN_WAIT_S`, with the size of the last clean-up.
  * The unit tests of the pure functions change with them:
    * a run that passes, and a run with each new check failed;
    * `reads_back` for each format and length;
    * `saves_folder_check` at 9 files, and at 8 and 10.
* `.github/workflows/ci.yml`: the `timeout-minutes` of the `e2e save` step becomes 50, as `STEP_LIMIT_S` (`the_ci_step_limit_is_the_waits_limit`).  The `browser-tests` job becomes 75 minutes.  Its comment's rule then holds: the steps before `e2e save`, its own limit, and about 5 minutes to spare.  The steps before took about 15.3 minutes before M3, and the ALAC cases of `web-test` add to them.  Its comment and the step's comment give the new numbers.
* `docs/invariants.md`, MEM-4 "Enforced by": `e2e save` checks the worker during an ALAC encode too.
* `xtask/src/e2e.rs`: the module doc names M4A for `save`.
* `README.md`, the `e2e` list: `save` saves WAV, FLAC, M4A and MP3, and checks that each M4A decodes to the WAV's samples.
* Acceptance: `cargo test -p xtask --lib` passes the unit tests, each failing first.  `cargo run -p xtask -- e2e save --browser chrome` passes on this Mac.  Record each M4A measure and the time of the run in the Progress line.

### Step 6: the numbers and the close

* Measure the wasm bundle again with `(cd app && trunk build --release)`.  Record the three sizes in Surprises, with the growth of `stem-worker_bg.wasm`.
* `cargo run -p xtask -- e2e decode --browser chrome` passes, so M2's decode path still holds.
* Read the parent's M3 section line by line against the code, and record each line's evidence in the Progress line of this step.

### Before review round 1

* `cargo run -p xtask -- check --tier push` and `cargo build -p xtask` pass.
* `cargo run -p xtask -- web-test --browser chrome` passes.
* `cargo run -p xtask -- e2e save --browser chrome` and `e2e decode --browser chrome` pass.

## Validation and acceptance

* `cargo run -p xtask -- check --tier push` passes.
* `cargo test --workspace --exclude roformer-gpu` and `cargo test -p engine` pass.
* `cargo run -p xtask -- web-test --browser chrome` passes the ALAC tests of `worker`, `protocol` and `app`.  They hold on both paths, bit for bit against `pcm16` and `alac_file`, within the packet-table allowance.
* `cargo run -p xtask -- e2e save --browser chrome` passes, with the M4A saves of 1 and 10 minutes.  Each decodes to the planted mix bit for bit, and each meets the limits of FLAC.
* `cargo run -p xtask -- e2e decode --browser chrome` passes.
* The extension test of `app/src/export.rs` lists `m4a`.
* After M3, the orchestrator decodes a saved M4A with `afconvert` and compares it with the WAV save.  The team does not run it.  No audio goes into git (ASSET-1).

## Idempotence and recovery

Each step is one commit, and each commit passes `check --tier commit`.  Step 1 changes no bytes of a FLAC.  A failed step is reverted with `git revert` of its commit.  An old worker answers a request of kind `alac` with `error`, so it writes no wrong file.  The page offers M4A only from Step 4, so Steps 1 to 3 add no item that a user can see.  A new implementer starts from this plan and its Progress.

## Progress

* [x] 2026-11-10: plan written by the team leader.  Plan review round 1 found 2 blockers, 7 should-fix and 4 nits.  Round 2 found 1 blocker, 8 should-fix and 4 nits.  The plan takes each (Decision log).
* [x] Step 1: one sink for FLAC and ALAC.  2026-11-10: `EncodeOut` with `put`, `reserve` and `replace_head`.  `flac_into` reserves its stream header.  First failures, with a `reserve` that writes nothing: `a_reserve_is_one_unit_that_the_head_replaces` gave `w3@0 w65537@0`, not `w65536@0 w1@65536 w3@65537 w65537@0`.  `a_reserve_holds_its_zeros_outside_wasm_memory` gave a file of 0 bytes, not 1,048,576.  With a stub that makes its zeros in a `Vec`, the second test gave 65,584 bytes of WebAssembly memory on the file sink.  Then `web-test --browser chrome` passed: 80 `stem-worker` tests, with the FLAC tests unchanged.
* [x] Step 2: the wire kind `alac`, the worker's encode and the page's reply.  2026-11-10: `kind::ALAC`, `Request::Alac`, `Reply::Alac`, `Codec::Alac` with `alac_into`, and `Event::Alac`.  First failures:
  * `stem-protocol`, with a stub `kind()` that gives `flac` for ALAC: 6 tests failed on "the type on the wire" (`flac`, not `alac`), or on "alac: 1 Hz decodes as 1".
  * `app`, with a stub that gives no event for `Reply::Alac`: `an_alac_reply_reaches_the_page_as_sent` failed on `alac.is_some()`.  The protocol stub failed it at the same assert.
  * `stem-worker`, with an `alac_into` that writes nothing: 9 tests failed on asserts.  The new ALAC tests failed on the bytes of `alac_file` or on `result.is_err()`.  The order test gave no reads, `errors_name_the_request_they_answer` gave no `alac` errors, and the "write refused" case gave no `failed` reply.  `a_failed_stored_encode_leaves_another_encode_whole` passed with this stub, so it now asserts a stored file with bytes, which then failed.
  * `xtask`: with the new edge, `crate_tests_of_engines_family_do_not_run_engine_alone` failed before its change to `dsp`.  `crate_tests_of_m4a_run_engine_alone` failed on the old fixture.
  * TD-16: `git grep -n "'flac'\|\"flac\"" xtask/` shows `flac` in `xtask/src/e2e.rs` and `xtask/src/e2e/save.rs` only.  No hand-written script writes or reads `alac` in this step.
* [x] Step 3: the memory of the encode.  2026-11-10: `an_alac_is_encoded_in_blocks` (memory path) and `a_stored_alac_holds_only_its_packet_table` (storage path).  From 10 s to 60 s, the packets grow by 538, from 108 to 646.  The peak grew by 2,152 bytes on each path, 4 bytes for each packet.  Stored, it went from 188,952 to 191,104 bytes.  In memory, it went from 172,479 to 174,631 bytes.  First failures:
  * a stub that also keeps each packet in a `Vec<u8>`: the peak grew by 9,490,792 bytes on each path, past the bound of 12 bytes for each packet.
  * a stub that makes the sizes with `Vec::new()`: the peak grew by 3,584 bytes stored and 5,376 bytes in memory, past the bound of 4 bytes for each packet (3,176 bytes).
* [x] Step 4: the page offers M4A.  2026-11-10: `Format::M4a` in `Format::ALL` as `[Wav, Flac, M4a, Mp3]`, `encode_alac`, `save_type` `audio/mp4`, and the label "M4A encoding".  First failures:
  * `only_wav_is_made_without_the_worker`, with `Format::M4a` out of `Format::ALL`: the extensions were `["wav", "flac", "mp3"]`, not `["wav", "flac", "m4a", "mp3"]`.
  * `encode_alac_asks_the_worker_for_an_m4a_of_the_arrays`, with an `encode_alac` that posts `Request::Flac`: the kind was `flac`, not `alac`.
  * `worker_saves_post_paths_then_offer_the_workers_file`, with the old `if ... else`: the M4A save posted `mp3`, not `alac`.
  * `an_m4a_encode_error_names_m4a_encoding`, with the old arm: the status line was "Worker request failed: ...", not "M4A encoding failed: ...".
  * Then `web-test --browser chrome` passed: 189 `app` lib tests, 88 `stem-worker` tests and 30 `stem-protocol` tests.
  * `e2e save --browser chrome` passed in 2 minutes 24 s, with the MP3 item at row 3 of the menu.
* [ ] Step 5: `xtask e2e save` saves M4A.
* [ ] Step 6: the numbers and the close.
* [ ] Review rounds.

## Surprises and discoveries

* 2026-11-10, team leader: `encode` names a reply with no name `stem.<kind>`.  The kind `alac` is not the extension `m4a`, so Step 2 adds `Codec::extension`.
* 2026-11-10, team leader: the stored path of `app/src/lib.rs` sends every worker format that is not FLAC to the MP3 encode.  Step 4 makes it a `match`.
* 2026-11-10, team leader: `xtask e2e save` finds the worker's line by the format's extension.  For M4A the worker logs the kind `alac`, so Step 5 adds the kind to `FORMATS`.
* 2026-11-10, team leader: `origin/gh-pages` holds no `LICENSE*` file and no `THIRD-PARTY.md`.  So the published page gives no copy of a licence for any crate.  Step 2 adds a TD entry.
* 2026-11-10, plan review round 1: two more saves push `WAITS_S` of `e2e save` past its limit of 40 minutes, which a compile-time assert checks.  So Step 5 raises the limits of the step and of the CI job.
* 2026-11-10, implementer: the wasm bundle at `2193806` (the same code as `6983ca9`), from `trunk build --release`: `app_bg.wasm` 6,406,239 bytes, `decode-worker_bg.wasm` 1,381,067 bytes, `stem-worker_bg.wasm` 1,250,092 bytes.
* 2026-11-10, implementer: the first `check --tier commit` of Step 4 failed in `an_aiff_decodes_through_a_fifo` of `engine`, on "the write to the FIFO".  No step of M3 changes `engine`.  The test passed 6 times alone, and the next tier run passed.  It looks like a race between the writer thread and the end of the read, under load.

## Decision log

* 2026-11-10, agent (team leader): the sink gets `reserve(len)` beside `put` and `replace_head`.  Reason: `replace_head` replaces one part or byte 0.  So the zeros must be one part on the memory path, and small writes on the storage path (the parent's acceptance).
* 2026-11-10, agent (team leader): an ALAC encode of 0 frames is refused, as a FLAC encode is.  Reason: the app never sends 0 frames, and the same text for each lossless format keeps the page's handling the same.
* 2026-11-10, agent (team leader): `Format::ALL` is `[Wav, Flac, M4a, Mp3]`.  Reason: the lossless formats stay together, and M4 can add its AAC item after MP3.  No snapshot shows the Save menu, so no artefact changes.
* 2026-11-10, agent (team leader): the worker's tests compare with `pcm16` and with `alac_file`, not with `engine`.  Reason: `stem-worker` may not take `engine` (LAYER-1, decision 0005).  `a_flac_clips_at_full_scale_and_rounds` and its ALAC copy pin the rounding of `engine::wav::encode_pcm16`.  `xtask e2e save` compares with `engine` natively.
* 2026-11-10, agent (team leader): `e2e save` reads and decodes both M4A files, of 1 and 10 minutes.  Reason: the parent asks that each saved file decode to the WAV save's samples.  FLAC reads only the 1-minute file, and M3 does not change that.
* 2026-11-10, agent (team leader): the published page ships no licence text, for any crate.  M3 records it as a new TD entry in Step 2, and does not fix it.  Reason: the gap holds for `rusty_mp3` and `flacenc` before M3, and the parent's M3 section asks only for the licence list.  The fix touches the build of every shipped crate, so it is its own PR.  The PR lists it under "Needs your attention".
* 2026-11-10, agent (team leader, plan review round 1): Step 2 holds the page's `Event::Alac` and its arm, since the page's matches of a reply have no wildcard arm.  Step 4 holds the menu, the request and the error label.
* 2026-11-10, agent (team leader, plan review rounds 1 and 2): `STEP_LIMIT_S` and the `e2e save` step of CI become 50 minutes, and the `browser-tests` job 75 minutes.  Reason: the two M4A saves add their waits, and a compile-time assert keeps the waits below the limit.  A run takes about 3 to 4 minutes; the limits stop only a hung run.
* 2026-11-10, agent (team leader, plan review round 2): the memory test of Step 3 asserts two bounds at 10 s and 60 s: the parent's 12 bytes for each packet, and 4 bytes.  Reason: at these lengths the peak is in the loop, so the tighter bound tells `with_capacity` from a doubling vector.  Round 1 had said that no bound could; round 2 showed where the peak is.
* 2026-11-10, agent (team leader, plan review round 2): `flac_into` uses `reserve` for its first stream header.  Reason: `reserve` then has a caller in Step 1, so no dead code fails clippy, and the FLAC's bytes stay the same.
* 2026-11-10, agent (implementer): Step 1 has two tests, not one.  `a_reserve_is_one_unit_that_the_head_replaces` checks the writes, the bytes and the parts.  `a_reserve_holds_its_zeros_outside_wasm_memory` checks the peak.  Reason: the stub that writes nothing cannot fail a bound on the peak.  So the peak test also checks the size of the file, and a second stub with zeros in a `Vec` fails its bound.
* 2026-11-10, agent (implementer): the protocol tests of FLAC run for ALAC in loops, with helpers for a lossless request and reply.  Three tests have new names: `a_lossless_request_keeps_its_name_and_rate`, `a_lossless_reply_keeps_its_blob` and `an_encode_reply_without_its_file_is_refused`.  Reason: one test for each form keeps the two kinds the same.  No doc named the old names.
* 2026-11-10, agent (implementer): `alac_into` calls `alac_packets`, which holds the encoder and the block buffers.  Reason: they are freed when it returns, before `head()` makes `moov` (Step 3, "Where the peak is").
* 2026-11-10, agent (implementer): `an_alac_decodes_to_the_stem` encodes 10 s, not 30 s.  The lengths at each side of a packet are one test, `an_alac_of_each_length_is_the_reference_on_both_paths`.  The 65,536 Hz refusal runs on the memory path only.  Reason: the debug build encodes slowly, and the storage path takes the same `alac_into`.
* 2026-11-10, agent (implementer): `errors_name_the_request_they_answer` sends `alac` at 96,000 Hz.  Reason: M4A refuses a rate above 65,535 Hz, so the request fails, as the MP3 one does.
* 2026-11-10, agent (implementer): `flac_and_mp3_saves_post_paths_then_offer_the_workers_file` runs for M4A too, with the new name `worker_saves_post_paths_then_offer_the_workers_file`.  It asserts the kind of each request, so it is the test that the M4A save posts `Request::Alac`.  Reason: the loop already makes the stored save of each format, and MEM-2 names it.
* 2026-11-10, agent (implementer): `end_stored_save` picks the encode with one `match` on the format, whose `Wav` arm offers the WAV and returns.  Reason: no arm can send a format to the wrong encode, and no arm is unreachable.
* 2026-11-10, agent (implementer): TD-53 keeps the "FLAC encoding" and "MP3 encoding" labels as untested, and names the test of "M4A encoding".  Reason: Step 4 adds that test, so the M4A label is not a gap.

## Outcomes and retrospective

Not started.
