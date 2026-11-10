# M4A, M4: save as AAC-LC

## Purpose

This is the milestone plan for M4 of `plans/active/m4a.md`.  The parent's M4 section stays the criteria.  After M4, the Save menu of each lane has "M4A (AAC 256 kbps)" where the browser has an AAC encoder.  The stem worker encodes the save with WebCodecs `AudioEncoder`, and the `m4a` crate makes the M4A.  For music, an AAC M4A is about a third of the ALAC M4A of M3.  Apple's tools take it (the parent's Purpose).

Readers: the implementer, the review agents of M4's rounds, the orchestrator, and the maintainer at the review of the PR.

## Context and orientation

The state at the start, on 2026-11-10, at commit `69e63db` on branch `feature/m4a`.  The review base of M4 is `69e63db`.  M1 to M3 are done (`plans/completed/m4a-m1.md`, `plans/completed/m4a-m2.md`, `plans/completed/m4a-m3.md`).  M4 runs on an Apple M2 Pro (arm64), with Chrome 157 and Safari 27.2.

### The decisions that M4 follows

All are in the parent's "Decisions for the maintainer" and Decision log, and in the orchestrator's brief of 2026-11-10.

* D1 (a): ALAC in M3 (done), AAC through WebCodecs `AudioEncoder` in M4.
* D3 (a): `oxideav-aac` is a dev-dependency of `engine` for decode fixtures only.  M4 adds no use of it.
* D6 (a): the crate `m4a` only makes bytes.  `stem-worker` does the writes.  One sink for each codec (`EncodeOut`: `put`, `reserve`, `replace_head`), with no copy.  Every test that needs an M4A uses `m4a` (no writer copy, TD-52).
* D7 (a): the decode trims `elst`, then `iTunSMPB`.  So the save writes both.
* Decision 0028 is accepted, and amended twice.  Decision 0029 is `proposed`.
* The parent's M4 section: key `e2e save` (`FORMATS`, `Files`, `reads_back`, `saves_folder_check` and the worker check) by the worker's kind.  An AAC save also has the extension `m4a`.  Give the AAC encode error its own label.
* The maintainer's rules for M4.  Each review round has them in its criteria, word for word:
  * Measure a gap against what the plan promises for M4, not against main.  Main has no M4A save, so "same as main" is never a reason for a gap.
  * From round 4, a round adds no new behaviour.  It fixes only what it finds, and the fixes stay small.
  * A gap that no user of the app can reach goes to a TD with a true reason and a clear text, unless it puts memory at risk or makes a panic.
  * Never leave user-facing text or behaviour wrong to avoid a snapshot or fixture update.  Update the artefact by the README process, and list it in the PR.
  * In late rounds, do not push the team to close gaps on paths the app cannot reach.

### Facts that the team leader measured on 2026-11-10

A spike in the scratchpad (`m4/spike/`, not in the repo) ran `AudioEncoder` in headless Chrome 157 and in Safari 27.2, on the page and in a worker.  The config was `mp4a.40.2`, 44,100 Hz, 2 channels and 256,000 bit/s.  It decoded the packets with WebCodecs `AudioDecoder`.

* Both browsers give `AudioEncoder`, `AudioDecoder` and `AudioData` on the page and in a dedicated worker.  `isConfigSupported` passes in both.
* Neither browser shows the encoder delay.  The first packet has the timestamp 0, and each packet has a duration of 1024 frames.
* Both delays are 2112 frames: an impulse at input frame 4096 decodes at frame 6208.  This is the delay of Apple's encoder ([Apple, QuickTime file format, appendix G](https://developer.apple.com/documentation/quicktime-file-format/appendix_g_audio_priming_handling_encoder_delay_in_aac)).
* Neither `AudioDecoder` trims the delay: the decode of `n` packets gives `n x 1024` frames.  The W3C AAC registration gives the same rule: an `AudioDecoder` discards no priming samples of a plain AAC stream ([W3C, AAC WebCodecs Registration](https://www.w3.org/TR/2026/DNOTE-webcodecs-aac-codec-registration-20260924/)).
* Safari encodes the first 1024 input frames as silence.  At the delay of 2112 frames, the decode is silent for source frames 0 to 1023, and matches the source after them.  A source of 1024 frames or less encodes to silent packets only.  Mediabunny reports the same Safari bug, and pads the input by 1024 frames ([Mediabunny, "Project sync"](https://mediabunny.dev/blog/project-sync)).  Chrome keeps every frame.
* Mediabunny measures the delay of each encoder at run time with an impulse, since the delay "differs wildly from system to system" (the same post).  The Chrome platforms other than macOS use other encoders, with delays that the team cannot measure here.
* Chrome gives `ceil(n / 1024) + 3` packets for `n` frames (`n` = 1, 1024, 4096, 16,384 and 88,977).  Safari gives `ceil((n + 2112) / 1024)` packets.  So the packet count is not known before the flush.
* The `decoderConfig.description` of the first output differs.  Chrome gives the bare AudioSpecificConfig, `12 10`.  Safari gives a whole `ES_Descriptor` with 4-byte descriptor sizes: `03 80 80 80 22 00 00 00 04 80 80 80 14 40 14 00 18 00 ... 05 80 80 80 02 12 10 06 80 80 80 01 02`.  Its config is the same `12 10`.  Safari's own `AudioDecoder` refuses that description, and takes the bare `12 10`.
* The bit rate.  With the default `bitrateMode` (`variable`), Safari gave 72 kbit/s for a target of 256 kbit/s, on 5 s of sines and noise.  With `bitrateMode: "constant"`, Safari gave 259 kbit/s and Chrome 240 kbit/s.  Chrome gave 240 kbit/s in both modes.
* The largest packet was 1,355 bytes, below the bound of 768 bytes for each channel (1,536 for stereo) in `m4a::Codec::max_packet_bytes`.
* `encodeQueueSize` reached 22 when the spike called `encode` with no wait, and stayed at 1 with a wait for the `dequeue` event after each call.

### The design of the encode

These are agent decisions of the team leader (Decision log).

* Order: the AAC M4A has its `moov` last (`Order::MoovLast`).  The head is `ftyp` and the `mdat` header, 36 bytes, which do not depend on the packet count.  So the encode reserves 36 bytes, puts each packet, puts the tail (`moov`), then puts the final head in place of the reserve.  `Layout` does not change.
* The delay: the worker measures it at the start of each encode, with an impulse through its `AudioEncoder` and `AudioDecoder` ("the probe").  It does not take 2112 as a constant.
* The lead-in: the worker puts `LEAD_IN` (2048) frames of silence before the source.  So Safari's silent first 1024 frames are lead-in, not source.  The trim is `LEAD_IN + delay`.
* The bit rate: `bitrate: 256000` and `bitrateMode: "constant"`, so the label "256 kbps" holds in Safari too.
* The gapless data: one `elst` entry (the skip, then the source's frames) and an `iTunSMPB` tag.  The tag's delay is the skip, `LEAD_IN + delay`, not the encoder's delay alone.  Its other fields are the padding and the source's frames.  Apple's tools and the app's decode (D7) then trim the same frames.
* The config: `m4a` takes the AudioSpecificConfig out of the description, bare or inside an `ES_Descriptor`.  The worker refuses a config that is not AAC-LC, 2 channels and the encode's rate.
* The menu: the page probes at start, on the page, with no worker message.  The probe passes when `isConfigSupported` passes and a short test encode gives a packet.  The parent's M4 section asks for this.
* The wire: a request and a reply of kind `aac`, as `alac` has (PROTO-1).

Why `Order::MoovLast` and not a head reserved for an upper bound of packets:

* The bound needs the delay and the flush packets of each encoder.  An encoder that gives more packets than the bound fails the save, and the team cannot test the encoders of Windows or Android.
* The bound needs a new form of `Layout` and a `free` box.  `Order::MoovLast` needs no change of `Layout`, which M1 and M2 already test (`m4a/tests/round_trip.rs`).
* A saved stem is a local file.  Each reader of a local file can seek: Symphonia, the decode worker's `BlockSource`, and Apple's tools.  Fast start helps only a player that streams over HTTP with no range requests.
* The app opens its own AAC save.  Its head shows no `moov`, so `head_length` gives `HeadLength::UnstatedPcm` (M2).  `largest_file` then allows the PCM size of the decode limit, far above the largest AAC save (about 97 MB).
* Decision 0028 says nothing on the order of an AAC save, so it does not change.

### The inputs that the app gives an encode

As in M3: a gap on any other input goes to a TD from round 1, unless it puts memory at risk or makes a panic.

* Paths: the storage path (a 16-bit stereo WAV in `saves/`, `Encode::Stored`), and the memory path (two transferred `Float32Array`s, `Encode::Arrays`).
* The rate is always 44,100 Hz (decision 0012).  The channels are always 2.
* A save always has 1 frame or more.  A song is at most the 1 GiB decode limit (decision 0013), about 134 million frames or 50.7 minutes.  At 256 kbit/s, the largest AAC save is about 97 MB, and has about 131,000 packets.
* The browsers: Chrome and Safari on macOS encode AAC.  Chrome on Linux has no AAC encoder (issue 40942807, parent's Surprises), so the menu does not offer AAC there.  Other browsers offer AAC only if their probe passes, and the worker measures their delay.
* So a rate other than 44,100 Hz, 0 frames and a config other than AAC-LC stereo are not reachable from the app on this Mac.  Each gets a clear error text and one test, and no other behaviour.

### The code today

* `m4a/src/mp4.rs`: `Codec::Aac { channels, config_len }`, `SampleEntry::Aac { config }`, `Edit { skip, frames }`, `Gapless { delay, padding, frames }`, `Order::MoovLast` (its doc says "Only tests use it"), `Layout::new`, `with_gapless`, `head_len`, `tail_len`, `head`, `tail`, `MAX_CONFIG_LEN` (64).  `Layout::new` takes the frame count, and `head` and `tail` want exactly `packets()` sizes.
* `m4a/src/lib.rs`: the crate doc and `alac_file`.
* `worker/src/web/encode.rs`:
  * `Codec` (`Mp3`, `Flac`, `Alac`) with `kind()`, `extension()` and `reply()`.
  * `Input` (`Arrays` or `Wav`) and `Input::read(at, left, right)`, one block at a time.
  * `OutFile` (`append`, `append_array`, `write_at`, `write_array`), and `EncodeOut` (`Parts` or `File`) with `put`, `reserve` and `replace_head`.
  * `encode`, `stored`, `alac_into`, `alac_end`, `alac_packets`, `alac_blob` (type `audio/mp4`), `log_memory`.
* `worker/src/web.rs`: `serve` dispatches `Request::Alac` to `encode(Codec::Alac, ...)`.  `handle` keeps the encode kinds out of the `queued` list.
* `worker/Cargo.toml`: the dev-dependency `symphonia` has the features `flac`, `isomp4` and `alac`.
* `worker/src/web/tests.rs`: `decode_m4a`, `m4a_reply`, `SAVE_M4A`, `stored_peak`, `last_file`, `arrays_file`, `encode_stored`, the counting allocator, and the ALAC tests of M3.
* `protocol/src/lib.rs`: `kind::ALAC`, `Request::Alac`, `Reply::Alac`, `Request::kind`, and the wire text at the top.  `protocol/tests/web.rs` loops the lossless kinds through their round trips.  `protocol/src/decode.rs`: `no_type_is_a_type_of_the_stem_worker`.
* `app/src/export.rs`: `Format` (`Wav`, `Flac`, `M4a`, `Mp3`), `Format::ALL`, `extension`, `label`, `needs_worker`, `JsSink`, and `only_wav_is_made_without_the_worker`.
* `app/src/separator.rs`: `encode_alac`, `Event::Alac`, and the match of a reply into an `Event`, with no wildcard arm.
* `app/src/lib.rs`: `save_type`, `save_menu` (one button for each item of `Format::ALL`), `can_encode`, the `Event::Error` arm (its inner `_` arm gives "M4A encoding"), the stored path's `match` on the format, and `end_memory_save`.
* `app/src/save_tests.rs`: the loops over the worker formats, `worker_saves_post_paths_then_offer_the_workers_file`, `a_memory_save_posts_the_arrays_of_its_format` and `an_m4a_encode_error_names_m4a_encoding`.
* `xtask/src/e2e/save.rs`: `FORMATS` (`(extension, row, kind)`), `LOSSLESS_BOUND_BYTES`, `Files` (`m4a_short`, `m4a_long`), `reads_back(format, seconds)`, `Files::keep`, `m4a_decodes_to_the_mix`, `file_checks`, `saves_folder_check` (9 files), `READS`, `WAITS_S`, `STEP_LIMIT_S` (50 minutes), the module doc, and their unit tests.  The worker check loops over `["flac", "m4a"]` by extension.
* `.github/workflows/ci.yml`: the `e2e save` step's `timeout-minutes` equals `STEP_LIMIT_S`, and the `browser-tests` job has 75 minutes.
* `test-support/src/lib.rs`: `skip(need, why)`, with the needs `gpu`, `timestamps`, `f16` and `fixtures`.
* `docs/tech-debt.md`: the next free TD number is 92.  TD-53 names untested arms and labels of the page's save.  TD-54 names the encodes of a save.

### Terms

* Probe (worker): the encode and decode of an impulse that gives the encoder's delay.
* Probe (page): the check at start that decides whether the menu offers AAC.
* Delay: the frames before input frame 0 in the encoder's decoded output.
* Lead-in: the `LEAD_IN` frames of silence that the worker puts before the source.
* Skip: `LEAD_IN + delay`, the frames that the `elst` entry and `iTunSMPB` trim at the start.
* Padding: the frames after the source, to the end of the track: `packets x 1024 - skip - frames`.  With Chrome's flush packets, the padding can span more than one packet.
* Description: `decoderConfig.description` of the encoder's first output.
* Fake encoder: a JS class that a test puts in place of `AudioEncoder` (and `AudioDecoder`), with known packets and a known delay.

## Plan of work

Each step is one signed commit, after `cargo run -p xtask -- check --tier commit` (GIT-2).  Each new test fails first on an assert, then passes.  Record each first failure in the Progress line of its step.  Stage each new file by name (`git add <path>`).  Never run `git add -A`.  Each step fixes, in its own commit, each doc that it makes wrong (DOC-1).  Read each new doc line, Progress line and TD for sentences of more than 25 words before the commit.

How a new test fails first on an assert (the rules of M3):

* A stub that gives a wrong value makes the first run fail on an assert, not on `todo!()` or a panic in `unwrap`.
* A test that needs a result writes `assert!(result.is_ok(), "{result:?}")` before it unwraps.  A test of a refusal asserts on the result, then on the text.
* A new enum variant cannot fail on an assert before it exists.  So add `kind::AAC`, `Request::Aac` and `Reply::Aac` first with a stub that writes the kind `alac`.  The round trip asserts `kind_of(&msg) == "aac"`, a literal.
* A test of behaviour that holds today gets a stub that breaks it, and the Progress line names the stub.
* For each path that the plan names, a test fails when the request goes to the wrong encoder (M3's retrospective).
* For a memory bound, measure each stage that can be the peak (M3's retrospective).

Processes: wait for a process by its PID or with `wait`.  Never wait with `pgrep -f`, which matches its own shell and never ends (M3's retrospective).

No step changes `CLAUDE.md` or `.claude/**`, by the team leader's reading on 2026-11-10.  The `m4a` line of the Layout ("or AAC from given packets") still holds.  If a step needs such a change, the implementer stops and tells the team leader, who gives the orchestrator a step checkpoint.

The browsers on this Mac:

* Chrome: `CHROMEDRIVER=/Users/lloyd/.local/chromedriver-157/chromedriver-mac-arm64/chromedriver`, and `--browser chrome` for `web-test` and `e2e`.  `web-test` runs Safari by default on macOS, so pass `--browser` each time.
* Safari: `--browser safari`, with `safaridriver`.  Never quit the maintainer's Safari.  If a Safari session times out, end stale WebDriver sessions and retry once.  If it still fails, stop and tell the team leader.
* Read `docs/references/local-browser-testing.md`.

The team leader measured the wasm bundle at `69e63db` (Surprises).

### Step 1: the AudioSpecificConfig of a description

* `m4a/src/mp4.rs` (or a new module `m4a/src/esds.rs`): `pub fn audio_specific_config(description: &[u8]) -> Result<&[u8], Error>`.
  * A description that starts with tag 3 is an `ES_Descriptor` (ISO/IEC 14496-1, 7.2.6.5).  Walk it: tag 3, its size, `ES_ID` and flags, then tag 4 and its 13 fixed bytes, then tag 5.  The bytes of tag 5 are the config.
  * Each descriptor size is 1 to 4 bytes of 7 bits (ISO/IEC 14496-1, 8.3.3).  Each byte but the last has its top bit set.  Safari uses 4 bytes, and the writer of M2 uses 1.
  * The `ES_Descriptor` flags can add fields: `streamDependenceFlag` adds 2 bytes, `URL_Flag` a length and a URL, `OCRstreamFlag` 2 bytes.  Read them, so a valid descriptor parses.
  * Any other description is the bare config.
  * Give a new `Error` variant, with a `Display` text, for each refusal.  Refuse a config of 0 bytes or more than `MAX_CONFIG_LEN`.  Refuse a descriptor without tag 4 or tag 5, and a size past the end.
  * No panic on any input: each read checks the length.
* `m4a`: `AacConfig::read(config) -> Result<AacConfig, Error>`, the fields of an AudioSpecificConfig that the worker checks (ISO/IEC 14496-3, 1.6.2.1).
  * The fields: the object type (with the escape 31), the rate (the index table of 1.6.3.4, and index 15 with its 24-bit rate), the channel configuration and `frameLengthFlag`.
  * It is a second reader beside `engine::aac_config`.  `engine::aac_config` is `pub(crate)`, and `engine` takes `m4a` only as a dev-dependency.  The doc of `AacConfig` says so.
  * Native tests: `12 10`, HE-AAC `2b 92 08 00`, 1 channel, the 960 flag, the escape 31, index 15, and a cut at each bit.  None panics.
* `Order::MoovLast`: its doc says that the AAC save uses it, since the packet count is known only at the end.
* Tests in `m4a/tests/mp4.rs`:
  * Chrome's description `12 10` gives `12 10`.
  * Safari's description (the bytes in "Facts") gives `12 10`.
  * The `esds` body that the writer makes for a config of 2, 5 and 64 bytes gives that config.  Take it from `Layout::tail` of a `MoovLast` AAC file through the test's box reader, since `moov` is in the tail.
  * A descriptor with each flag set gives the config.
  * The refusals: a tag 5 of 0 bytes and of 65 bytes, and a size of 5 bytes.  Also a cut at each byte of Safari's description, and a tag 3 without tag 4.  None panics.
  * The head of an AAC `MoovLast` layout is 36 bytes for 0, 1, 1024 and 10^8 frames, with and without an `Edit` and a `Gapless` tag.  The worker reserves this length before the count is known.  The stub of the first failure: a `head_len` that counts the packets.
* `m4a/src/lib.rs`: the crate doc names the new function.  `README.md` and `docs/ARCHITECTURE.md`: the `m4a` lines, if they list what the crate does.
* Acceptance: `cargo test -p m4a` passes, with each new test failing first.

### Step 2: the wire kind `aac`, the worker's encode and the page's reply

The wire, the worker and the page's side of the reply change in one commit (PROTO-1).  A new `Reply` variant needs its arm in the page's exhaustive matches.

* `protocol/src/lib.rs`: `kind::AAC = "aac"`, `Request::Aac { name, source }` and `Reply::Aac { name, file }`, as `Alac` has, with the same `Encode` and `Encoded`.  The wire text at the top and the docs of `Encode`, `Encoded` and the source and file helpers name AAC.
* `protocol/tests/web.rs`: the loops over the lossless kinds also run `aac`.  Rename a helper or a test whose name says "lossless" if it then misleads.  `protocol/src/decode.rs`: `no_type_is_a_type_of_the_stem_worker` lists `kind::AAC`.
* TD-16: no hand-written script of `xtask e2e` writes or reads the kind `aac` in this step.  Check with `git grep -n "'alac'\|\"alac\"\|'aac'\|\"aac\"" xtask/`, and record it in the Progress line.  Step 6 reads only the worker's `memory: encode aac` line.
* WebCodecs goes through an inline JS module, as the module of `out_open` in `encode.rs` does.  Do not use the WebCodecs types of `web-sys`: they need `--cfg=web_sys_unstable_apis`, which would change the build of every wasm crate.
* `stem-protocol`: one constant of the encoder config (codec, channels, bit rate, bit rate mode), which the worker and the page's probe both read.  No other copy of these values exists.  LAYER-1 lets `app` and `stem-worker` share only `stem-protocol`.  Put it outside `mod kind`: `protocol_kinds` (`xtask/src/rules.rs`) reads each text in `mod kind` as a message type.
* `worker/src/web.rs`: `serve` sends `Request::Aac` to `encode(Codec::Aac, ...)`.  `handle` keeps `aac` out of the `queued` list.
* `worker/src/web/encode.rs`:
  * `Codec::Aac`, with `kind()` `aac`, `extension()` `m4a`, and `reply()` `Reply::Aac`.
  * Constants: `LEAD_IN` (2048 frames), `AAC_BLOCK` (4096 frames each read), `QUEUE_MOST` (the most `AudioData` in the encoder's queue, for example 4), and `OUT_MOST` (the most frames sent and not yet back as packets, for example 32,768).  The bit rate comes from the constant of `stem-protocol`.
  * The encoder config: `codec: "mp4a.40.2"`, the rate, 2 channels, `bitrate: 256000`, `bitrateMode: "constant"`.
  * `aac_delay(rate)`, the probe, with the save's exact config.  Encode 16,384 frames of 2 channels, with 1.0 at frame 4096 and 0 elsewhere.  Flush, and decode the packets with `AudioDecoder` and the bare config.  The delay is the frame of the largest absolute sample, minus 4096.  Refuse, with "AAC encoder: ..." texts: a peak below 0.25, a delay of 8192 frames or more, or a delay below 0.
  * Close each `AudioData` after its `encode`, and close each encoder and decoder at the end, on every path, errors included.  WebCodecs keeps a copy of each input, and Safari collects garbage late (MEM-2, "Why").  Unclosed blocks of a long song could hold about 1 GB of JS memory (MEM-4).
  * `AudioData` timestamps are in microseconds: `frame x 1,000,000 / rate`, rounded.
  * After its error callback, a codec is already closed, and a second `close` throws.  So close a codec only when its `state` is not `closed`.
  * The chunk queue, the description and the error belong to one encode, not to the module.  Two encodes can run at once in the worker.
  * `aac_into(input, rate, out: &mut EncodeOut)`:
    1. Refuse 0 frames with "AAC encoder: the stem has no samples".
    2. Refuse a rate that is not in the AAC table of sampling frequencies (ISO/IEC 14496-3, 1.6.3.4), with its own text.  The check of item 10 uses the same table, the table of `m4a::AacConfig`.
    3. `isConfigSupported` of the config.  If it fails, "AAC encoder: this browser has no AAC encoder".
    4. `aac_delay(rate)`.
    5. `Layout::new` of an upper bound of frames, `Order::MoovLast`, with an `Edit` and a `Gapless` tag, only to refuse 4 GiB and to give `head_len()` before the encode.  The bound is `(ceil((LEAD_IN + frames + delay) / 1024) + 8) x 1024`.  Its `config_len` is `MAX_CONFIG_LEN`, since the description does not exist yet.
    6. `out.reserve(layout.head_len())`.
    7. Make the encoder.  Its output callback copies each chunk into a new `Uint8Array` in JS, and keeps it in a JS queue.  It keeps the description of the first output.  Its error callback keeps the error.
    8. Encode `LEAD_IN` frames of silence, then the source in blocks of `AAC_BLOCK`, as `f32-planar` `AudioData` with timestamps from the frame count.  An `f32-planar` `AudioData` takes one buffer, with the left plane then the right plane.  Read the source with `Input::read`, as ALAC does.  After each `encode`, wait for `dequeue` while `encodeQueueSize` is above `QUEUE_MOST`.  Add the listener first, then read `encodeQueueSize` again, so the wait cannot miss an event.  Then put each chunk of the JS queue into `out`, and push its size.
       * Also bound the outputs in flight.  By the WebCodecs spec, `encode` can lower `encodeQueueSize` at once, and outputs come only as tasks ([W3C WebCodecs](https://www.w3.org/TR/webcodecs/)).  `Input::read` is sync, so a loop that never yields to a task gets no output until `flush`.
       * So after each block, yield one task (for example a `setTimeout` of 0).  Then, while `frames sent - 1024 x packets received` is above `OUT_MOST` frames (for example 32,768), wait for the next output or error.
       * If no output comes within a time limit (for example 10 s), end the encode with "AAC encoder: the encoder gave no output".  So an encoder that holds its output until `flush` fails with a clear text, and does not hang.
       * If the encoder's queue does not move within the same time limit, end the encode with "AAC encoder: the encoder's queue did not move".  An encoder error during either wait ends the encode at once.
    9. `flush`, then put the last chunks.  An encoder error at any time ends the encode with "AAC encoder: <the error>".
    10. Take the config from the description with `m4a::audio_specific_config`, and read its fields with `m4a::AacConfig::read`.  Refuse a missing description.  Refuse a config that is not object type 2, 2 channels and the encode's rate, with an "AAC encoder: ..." text.  Also refuse `frameLengthFlag` 1 (960 frames), which the app's decode refuses (`engine/src/decode.rs`, `frames_960`).
    11. `packets = sizes.len()`.  Refuse `packets x 1024 < skip + frames`, with "AAC encoder: the encoder gave too few packets".
    12. The final layout: `Layout::new(m4a::Codec::Aac { channels: 2, config_len }, packets x 1024, rate, Some(Edit { skip, frames }), Order::MoovLast)`, then `with_gapless(Gapless { delay: skip, padding, frames })`.
    13. `aac_end(layout, sizes, config, out)`: `out.put(&layout.tail(...))`, then check that `layout.head(...)` has the reserve's length, then `out.replace_head(...)`.  Step 3 calls `aac_end` alone.
  * `EncodeOut::put_array(&Uint8Array)`: on the storage path, it calls `append_array` of the JS array, so WebAssembly memory holds no packet.  On the memory path, it pushes the JS array into the parts.  Each chunk goes through it.  So no write is a view (MEM-5), and no packet is copied into WebAssembly memory.
  * The sizes: `Vec::with_capacity` of the bound's packets.  Drop the block buffers and the encoder before the tail and head are made.
  * `aac_blob`: a `Blob` of type `audio/mp4` from the parts.  `stored` and `encode` pick the encode with a `match` on the codec, with no `else` arm.  Today `stored` sends each codec that is not FLAC to `alac_into`.
  * The module doc names the AAC encode: its probe, its lead-in, its queue, and its `moov` last.
* `app/src/separator.rs`: `Reply::Aac` becomes `Event::Aac { name, file }`.  A test as `an_alac_reply_reaches_the_page_as_sent`.
* `app/src/lib.rs`: the `Event::Aac` arm offers the file as `Event::Alac` does.  The `Event::Error` arm takes `kind::AAC` with the label "AAC encoding", and clears the save notes.  The menu does not offer AAC yet (Step 5).
* A failed stored AAC encode.  Today a stored encode replies `Encoded::Failed`, and the page takes it as a failure of browser storage (`stored_encode_ended` in `app/src/lib.rs`).  It shows the storage text for a song of more than `MEMORY_PATH_MAX_FRAMES`, or runs the encode again on the memory path.  An AAC encoder failure is not a storage failure, and a second encode fails the same way.
  * `stem-protocol`: `Encoded::Failed` gets a field that tells an encoder failure from a storage failure, with its encoder, decoder and round trip (PROTO-1).  The implementer names it, and records the name.  A missing field decodes as a storage failure, the behaviour of today (TD-16).
  * The worker sets it for each AAC refusal of `aac_into` and each encoder error.  It does not set it for a read or a write of browser storage.  The other codecs do not set it: their encoders fail only on inputs that the app does not send.
  * The page: a stored encode that failed in its encoder gives "AAC encoding failed: <text>", with no second encode and no storage text.  Take the label from the format, as the `Event::Error` arm does.
  * Tests: the protocol round trip of the field.  The worker sets it for an encoder error, and not for a refused write.  The page gives the text for a song of 30 s and one of more than 100 s.  The page tests fail first with today's arm.
  * TD-16: check the hand-written scripts of `xtask e2e` that read `Failed`, and record it.
* `app/src/save_tests.rs`: `an_aac_encode_error_names_aac_encoding`, beside `an_m4a_encode_error_names_m4a_encoding`.  A reply `error` of kind `aac` gives a status line that starts with "AAC encoding failed".  It fails first with the old arm, which gives "M4A encoding failed".
* `worker/src/web/tests.rs`.  A fake encoder and a fake decoder in JS, installed on `globalThis` for one test and restored after it:
  * The fake encoder gives one packet for each 1024 frames that it holds, after its own delay of `d` frames, and `extra` flush packets.  Each packet's bytes hold its index and a sum of its frames, so the test can check the input.  It gives a description that the test chooses: bare, or as Safari's `ES_Descriptor`.  It holds its outputs on a timer, so the queue fills.  It records the largest `encodeQueueSize` that it sees.
  * The fake decoder gives, for the probe's packets, an impulse at frame `4096 + d`.
  * The tests, each on both paths where the path matters:
    * `an_aac_is_the_m4a_of_its_packets`: the file equals the bytes of `m4a` for the fake's packets.  These are a 36-byte head, the packets in order, then `moov`.  Its `elst` has the skip `LEAD_IN + d` and the source's frames, and it has `iTunSMPB`.  Build the expected file in the test with `Layout`, `tail` and `head`.  For `d` of 1024, 2112 and 2624, and lengths 1, 1023, 1024, 1025 and `3 x 4096 + 7` frames.
    * `an_aac_encodes_the_lead_in_then_the_source`: the fake sees `LEAD_IN` silent frames, then the source's `f32` samples in order.  The rounding is that of the WAV (`s / 32768`) on the storage path.
    * `an_aac_waits_for_the_encoders_queue`: the largest `encodeQueueSize` that the fake sees is at most `QUEUE_MOST + 1`.  The stub of the first failure: no wait.
    * `an_aac_waits_for_the_encoders_output`: a fake whose `encodeQueueSize` stays 0 and whose outputs come on a timer.  The largest lag of frames sent over frames back is at most `OUT_MOST + AAC_BLOCK`.  The largest JS chunk queue stays below a fixed count.  The stub of the first failure: no output wait.  A second fake that gives no output before `flush` gets "AAC encoder: the encoder gave no output".
    * `an_aac_takes_the_config_out_of_safaris_description`: Safari's description gives `esds` with `12 10`.
    * `an_aac_closes_each_audio_data`: the fake counts each `AudioData` that it gets and each `close`.  After an encode, after an encoder error in the middle of an encode, and after each refusal below, no `AudioData`, encoder or decoder stays open.  The stub of the first failure: no `close`.
    * The refusals, each with its own text:
      * no samples, and a browser with no AAC encoder (a fake `isConfigSupported` that fails);
      * a probe with no peak, with a delay of 8192, and with a delay below 0;
      * an encoder error during the encode;
      * a config of HE-AAC (`2b 92 08 00`), a config of 1 channel, and a missing description;
      * a rate of 22,000 Hz, which has no AAC index, and a config with `frameLengthFlag` 1;
      * too few packets.
    * On the storage path, each refusal is a `Failed` reply, with no output file (`a_failed_stored_encode_says_why_and_leaves_no_file`).
    * `stored_encodes_write_whole_arrays_as_they_read`: an AAC case.  The reserve of 36 bytes at 0, then a write for each packet, then the tail, then the head at 0.  No write is a view.
    * `errors_name_the_request_they_answer` and `a_stored_encode_outside_saves_or_malformed_is_refused` cover `aac`.
    * The reply's `Blob` has type `audio/mp4`, and a reply with no name gets `stem.m4a`.
  * The helpers that match each codec (`last_file`, `arrays_file`, `stored_peak`, `encode_stored`, the reply helpers) take `Codec::Aac`.
* `docs/tech-debt.md`: a new TD entry, "CI cannot run the real AAC encode" (the CI TD of this plan, number 92 at the start).  Chromium on Linux has no AAC encoder for WebCodecs (issue 40942807).  So CI tests the AAC encode only with the fake encoder.  The real encode runs only on a Mac: the Step 4 test and `e2e save`.  The fix: a CI runner with an AAC encoder, such as `macos-latest`.  The TD also records an assumption: the worker's probe measures the delay through `AudioDecoder`, which must trim nothing.  Only macOS shows this (Step 4).  Name it in VERIFY-1's "Gaps" in `docs/invariants.md`.  TD-53 and TD-54 name the AAC arm and encode where they list the others.
* Docs of this step (DOC-1): `docs/ARCHITECTURE.md` (the diagram's worker node, the line of `stem-worker`, the worker's save texts, and the memory-on-phones paragraph: the AAC packet table, the two queues and the probe), `docs/invariants.md` (MEM-1 and MEM-5 "Enforced by"), and the module docs.
* `docs/references/browser-quirks.md`: Safari's AAC encoder, its three quirks (the silent first 1024 frames, the whole `ES_Descriptor` that its own decoder refuses, and 72 kbit/s in variable mode), and the worker's answer to each.
* Acceptance: `cargo run -p xtask -- web-test --browser chrome` passes the new tests of `worker`, `protocol` and `app`, each failing first.

### Step 3: the memory of the AAC encode

* Where the peak is.  In the loop, the two `f32` blocks, the probe's buffers and the sizes are live.  At the end, the blocks are gone.  The sizes (4 bytes for each packet) and the tail (about 8 bytes for each packet) then live together.  So the loop is the peak at 10 s (about 433 packets) and 60 s (about 2,590 packets).  The end is the peak only above a few thousand packets, about 1.5 minutes.  The implementer measures both stages, and records where the end passes the loop.
* A browser test `a_stored_aac_holds_only_its_packet_table`, with the fake encoder, beside `a_stored_alac_holds_only_its_packet_table`.  Run 10 s and 60 s.  `Δ` is the growth of the packet count.
  * In the loop, the peak grows by the sizes only: 4 bytes for each packet, with a slack of 1 KiB.
  * At the end, the sizes and the tail live together: at most 12 bytes for each packet, with a slack of 1 KiB.  Measure the end alone, as `the_end_of_an_alac_holds_only_its_head` does.
* The same on the memory path, as `an_alac_is_encoded_in_blocks` does.
* The JS heap of the worker, where Chrome gives `performance.memory`: the chunks in the JS queue stay below a fixed count.  The implementer decides how to measure, and records it.  TD-11 says that the workers have no `performance.memory`.  If no measure is possible, the two queue tests of Step 2 are the bound, and the Progress line says so.
* The first failures: a stub that keeps each packet in a `Vec<u8>` of WebAssembly memory, and a stub that makes the sizes with `Vec::new()`.  If a stub does not fail on a path, stop and tell the team leader with the numbers.  Do not loosen a bound to pass.
* `docs/invariants.md`, MEM-1 "Enforced by": the AAC memory tests of this step.
* Acceptance: `cargo run -p xtask -- web-test --browser chrome` passes the tests, which failed first.

### Step 4: the real encode

* `worker/Cargo.toml`: the dev-dependency `symphonia` gets the feature `aac`, for this test.
* A browser test `a_real_aac_decodes_to_the_source` in `worker/src/web/tests.rs`.  It runs the browser's own `AudioEncoder`, if `isConfigSupported` passes.  Else it logs "skipped: this browser has no AAC encoder", and passes (the CI TD of Step 2).
  * Encode 10 s of a signal that has no period: sines and noise, as the spike made.  Use both paths.
  * Decode the file with Symphonia (`isomp4`, `aac`).  Symphonia ignores `elst`, so the test reads the skip from the file's `elst`, and drops it.
  * Assert: the skip is `LEAD_IN` plus the delay that the test's own call of `aac_delay` gives.  The trimmed decode has the source's frames.  Its cross-correlation with the source peaks at lag 0 over lags of -2048 to 2048.  Log the delay; on this Mac it is 2112.
  * Assert that the first 4096 frames of the trimmed decode are not silent, and that their error to the source is below a bound.  This catches a loss of the start, such as Safari's.  Measure the bound in both browsers, and set it with a margin.  Record the numbers.
* VERIFY-1: `skips_use_the_helper` (`xtask/src/rules.rs`) fails a literal that starts with "skipped", unless `SKIP_ALLOWLIST` names it.  So add the entry, with its reason.  A wasm test cannot read `STEMYLA_REQUIRE`, and CI's Chrome on Linux has no AAC encoder (the CI TD of Step 2).  Name the test in VERIFY-1's list of the wasm exceptions in `docs/invariants.md`.
* Run it with `web-test --browser chrome` and `web-test --browser safari`.  The stub of the first failure: `LEAD_IN` of 0, which Safari fails on the start check.  Also a skip of 0, which both browsers fail on the lag.
* Acceptance: both runs pass the test, and the Progress line gives the skips, the errors and the bit rate.

### Step 5: the page offers AAC

* `app/src/export.rs`:
  * `Format::Aac`, extension `m4a`, label "M4A (AAC 256 kbps)", `needs_worker` true.
  * `Format::ALL` is `[Wav, Flac, M4a, Mp3, Aac]` (Decision log of M3: AAC after MP3).
  * `pub fn offered(aac: bool) -> impl Iterator<Item = Format>`: `Format::ALL` without `Aac` when `aac` is false.
  * `JsSink::Aac(JsChannels)`.
  * `only_wav_is_made_without_the_worker` lists `["wav", "flac", "m4a", "mp3", "m4a"]`, and asserts the labels.
  * A native test of both menu states: `offered(false)` is the four formats of M3, in order; `offered(true)` is the five.
  * The wasm32 test `a_mix_export_does_not_grow_wasm_memory` gets the `Aac` arm.
* The page's probe: an async function at start, in JS, that gives whether AAC can be saved.
  * It passes only when three things hold.  `AudioEncoder` exists.  `isConfigSupported` of the config in `stem-protocol` passes.  An encode of 4096 frames, then a flush, gives at least one packet within a time limit.
  * The time limit is a parameter: 5 s in the app, and short in the tests.
  * Each failure gives false, with no throw.  The probe closes its `AudioData` and its encoder on each path.
  * The page logs one line: `export: AAC encoder: yes`, or `export: AAC encoder: no (<why>)`.
  * The page keeps the result.  Until the probe ends, the menu does not offer AAC.
* `app/src/lib.rs`: `save_menu` loops over `export::offered(<the probe's result>)`.  `save_type(Format::Aac)` is `audio/mp4`.  The stored path's `match` sends `Format::Aac` to `encode_aac`.  `end_memory_save` sends `JsSink::Aac` to `encode_aac`, and its arm of no worker takes it too.
* `app/src/separator.rs`: `encode_aac(name, source)` sends `Request::Aac`.  A test as `encode_alac_asks_the_worker_for_an_m4a_of_the_arrays`.  The doc of `settings` names AAC.
* `app/src/save_tests.rs` and the browser tests of the probe:
  * The probe with a stubbed `AudioEncoder`: absent, `isConfigSupported` false, an error in `configure`, no packet within the time, and a packet.  Only the last gives true.  This test runs in CI.
  * The menu with a stubbed `AudioEncoder`, in CI: the labels that `save_menu` offers come from the page's kept result.  Before the probe ends, four labels.  After a probe that passes, five, with "M4A (AAC 256 kbps)" last.  After a probe that fails, four.  `save_menu` and the probe's caller take the labels from one function that this test calls.  The stub of the first failure: a `save_menu` that loops over `Format::ALL`.
  * Each loop over the worker formats also runs `Format::Aac`, with the download type `audio/mp4` and the kind `aac`.
  * The stored AAC save posts `Request::Aac`, and the memory-path AAC save posts `Request::Aac` with both arrays.  Each fails first with a stub that sends `Format::Aac` to `encode_alac`.
* Docs: `README.md` (line 3 and step 4 of "How to use": AAC where the browser can encode it), `docs/ARCHITECTURE.md` (the page's save flow and the `saves/` line), `app/src/store.rs` (the `saves/` doc), TD-53 (the "AAC encoding" label has its test).  `docs/invariants.md`, MEM-2 "Enforced by": the two save tests that name FLAC, M4A and MP3 also name AAC.
* Acceptance: `cargo test -p app` passes the native tests, each failing first.  `cargo run -p xtask -- web-test --browser chrome` passes the new browser tests.  `cargo run -p xtask -- e2e save --browser chrome` passes, since the menu has a fifth item on this Mac.

### Step 6: `xtask e2e save` saves AAC

* `xtask/src/e2e/save.rs`:
  * `FORMATS` gets the row `("m4a", 4, Some("aac"))`.  Each check, `Files`, `reads_back`, `Files::keep`, `saves_folder_check` and the worker check key a save by its kind, not by its extension.  The kind of the WAV save is `wav`, from the page, not from the worker.  The implementer chooses the form, such as `(kind, extension, row)`, and records it.
  * Whether the run saves AAC: the page's probe line.  Each restore is a reload, so each save has a new page and a new probe.  Before each save, after the restore, the run waits for the probe line of that page.  `WAITS_S` has one such wait for each save.  The line of the first page decides whether the run saves AAC.  On macOS, the run fails if the page does not offer AAC: Chrome and Safari encode AAC there through the system.  On Linux, it fails if the page offers AAC, since the CI TD of Step 2 then needs an update.  Elsewhere, it follows the page.  A pure function decides it, with unit tests.
  * The worker check of `aac`: the growth for 10 minutes is at most `LOSSLESS_BOUND_BYTES` above that for 1 minute.  Rename the constant if its name then misleads.
  * `reads_back`: each AAC file, of 1 and 10 minutes, is read whole.  `file_checks`: each AAC decodes through `engine::decode_bytes(bytes, Some("m4a"))` to the planted mix's frames exactly.  Its cross-correlation with the planted mix peaks at lag 0 over lags of -1024 to 1024.  Compute it on the first 10 s only.  `xtask` and `engine` build at opt-level 0, and lags over the whole 10-minute file are about 5.4 x 10^10 multiply-adds for each channel.  The mix repeats every 3,142 frames, so no other peak is in this range.  Its error to the mix, over the whole file and over the first 4096 frames, is below a bound.  The implementer measures the bound in both browsers, sets it with a margin, and records it.
  * The planted stems are sines of about 14 Hz and 28 Hz, so the cross-correlation is almost flat near lag 0.  So also assert that the error at lag 0 is less than at lags of -2, -1, 1 and 2, over the whole file.  Record the margins of both browsers.  The exact length and the error bounds are the main checks.  Step 4's test, with a signal that has no period, is the strong check of the lag.  If a run shows a peak at lag 1 or -1, stop and tell the team leader with the numbers.
  * `saves_folder_check`: 11 files with an AAC save (4 of `.m4a`), and 9 without.
  * The waits: two more saves and two more reads.  `WAITS_S`, `READS`, `STEP_LIMIT_S` and their tests change with them.  `the_ci_step_limit_is_the_waits_limit` keeps `STEP_LIMIT_S` equal to the CI step.  Compare the AAC save times with `SLOWEST_SAVE_S`, and update the caps if AAC is slower.
  * The heap, WebAssembly, slice and read checks run for each item of `FORMATS` that the run saves.
  * The docs: the module doc, the doc of `file_checks`, and the doc of `CLEAN_WAIT_S`.
  * The unit tests of the pure functions change with them, each failing first.  They test a run that passes, and a run with each new check failed.  They test `reads_back` for each kind and length, and `saves_folder_check` at 9 and 11 files.  They test the probe rule for each platform.
* Safari.  `e2e save --browser safari` fails at `69e63db`, only on the checks of the JS heap (Surprises).  Safari gives no `performance.memory`, so the calibration and the 12 heap checks read `None`.  Make the run pass in Safari with the checks that Safari can measure.  A check that Safari cannot measure prints "not measured in Safari", and does not pass silently.  Chrome still runs it.  TD-11 already says that only Chrome measures the JS heap; name the Safari run there.  Record each such check.
* `.github/workflows/ci.yml`: the `timeout-minutes` of the `e2e save` step equals the new `STEP_LIMIT_S`.  The `browser-tests` job limit keeps its rule (the steps before, the step's limit, and about 5 minutes to spare).  The step's limit grows by about 9 minutes, so the job limit grows too.  Its comment gives the numbers.
* `docs/invariants.md`, MEM-4 "Enforced by": `e2e save` checks the worker during an AAC encode too, on a Mac.  `xtask/src/e2e.rs`: the module doc names AAC.  `README.md`, the `e2e` list: `save` saves AAC where the browser can encode it, and checks its length and lag.
* Acceptance: `cargo test -p xtask --lib` passes the unit tests, each failing first.  `cargo run -p xtask -- e2e save --browser chrome` and `e2e save --browser safari` pass on this Mac, each with its AAC saves.  Record the AAC measures and the time of each run in the Progress line.

### Step 7: the numbers and the close

* Measure the wasm bundle again.  Record the three sizes in Surprises.
* `cargo run -p xtask -- e2e decode --browser chrome` passes, so M2's decode path still holds.
* A local check with Apple's tools, not in CI.  In a scratch worktree outside the repo, write the 1-minute AAC save of `e2e save --browser chrome` to disk, as the orchestrator did for M3.  `afinfo` must give the source's 2,646,000 valid frames and a priming of the skip.  `afconvert -f WAVE -d LEI16` must decode it to the source's length, with its cross-correlation peak at lag 0.  Remove the files and the worktree after the check (ASSET-1).  Record it in Surprises.  If `afinfo` gives a priming other than the skip, or other valid frames, stop and tell the team leader with the numbers.
* Read the parent's M4 section line by line against the code.  Record each line's evidence in the Progress line of this step.

### Before review round 1

* `cargo run -p xtask -- check --tier push` and `cargo build -p xtask` pass.
* `cargo run -p xtask -- web-test --browser chrome` and `web-test --browser safari` pass.
* `cargo run -p xtask -- e2e save --browser chrome`, `e2e save --browser safari` and `e2e decode --browser chrome` pass.

## Validation and acceptance

* `cargo run -p xtask -- check --tier push` passes.
* `cargo test --workspace --exclude roformer-gpu` and `cargo test -p engine` pass.
* `cargo run -p xtask -- web-test --browser chrome` passes the AAC tests of `worker`, `protocol` and `app`, with the fake encoder.  The menu test with the stubbed encoder runs in CI.
* `web-test --browser chrome` and `--browser safari` on this Mac pass `a_real_aac_decodes_to_the_source` with the real encoder.
* `cargo run -p xtask -- e2e save --browser safari` passes on this Mac.  The AAC save decodes through M2's path to the source's length, with its cross-correlation peak at lag 0.  So does `--browser chrome`.
* CI's `browser-tests` job passes on `ubuntu-latest`, with the menu without AAC.  The CI TD of Step 2 records that CI cannot run the real encode.

## Idempotence and recovery

Each step is one commit, and each commit passes `check --tier commit`.  A failed step is reverted with `git revert` of its commit.  An old worker answers a request of kind `aac` with `error`, so it writes no wrong file.  The page offers AAC only from Step 5, so Steps 1 to 4 add no item that a user can see.  A browser with no AAC encoder sees the menu of M3.  A new implementer starts from this plan and its Progress.

## Progress

* [x] 2026-11-10: plan written by the team leader, after the spike of "Facts".  Plan review round 1 found 1 blocker, 7 should-fix and 9 nits.  The plan takes each (Decision log).  Round 2 found 1 blocker, 3 should-fix and 6 nits, and the plan takes each too.
* [x] 2026-11-10: a new team leader took over after the first one stopped by accident.  The orchestrator asked for one more plan review, the last.  It found 0 blockers, 3 should-fix and 4 nits, and the plan takes each (Decision log).
* [x] Step 1 (2026-11-10): the AudioSpecificConfig of a description.  `m4a::audio_specific_config` and `m4a::AacConfig::read` are in the new module `m4a/src/esds.rs`, with 6 new `Error` variants.  The 16 new tests in `m4a/tests/mp4.rs` failed first on an assert, with three stubs.  The stub `Ok(&description[1..])` and an `AacConfig::read` of fixed fields failed 12 tests.  A `head_len` of `MoovLast` that counts 8 bytes for each packet failed `an_aac_moov_last_head_is_36_bytes_at_each_length`.  A stub of wrong `Display` texts failed `each_description_refusal_says_what_is_wrong`.  An index in place of `get` failed `no_description_or_config_makes_a_panic` on a panic, which is what it checks.  `cargo test -p m4a` passes.
* [x] Step 2 (2026-11-10): the wire kind `aac`, the worker's encode and the page's reply.  The new field of `Encoded::Failed` is `in_encoder` (wire `in_encoder`, a boolean).  A reply without it decodes as a failure of browser storage.  The encode is in the new module `worker/src/web/aac.rs`, and its tests are in `worker/src/web/tests/aac.rs`.  TD-16: `git grep` finds no hand-written script of `xtask` that writes or reads `alac` or `aac`.  The only hits are the Rust row `("m4a", 2, Some("alac"))` of `FORMATS`, a list of audio extensions in `rules.rs`, and a MIME type in `serve.rs`.  The one script that reads `failed`, in `xtask/src/e2e.rs`, reads a FLAC reply, and a new optional field does not change it.  Each new test failed first on an assert, in four stub runs in Chrome.  Run A had six stubs.  They were a skip without the lead-in, no wait for the queue, and no wait for the output.  They were also no `close` of an `AudioData`, the head before the `moov`, and the extension `aac`.  It failed 10 of the 11 tests of `tests/aac.rs`.  Run B: no lead-in, the raw description as the config, and one text for each refusal.  It failed the lead-in, Safari, refusal and no-output tests.  Run C: inverted `in_encoder` flags, and no `saves/` check for AAC.  It failed the two refusal tests of the storage path and `a_stored_encode_outside_saves_or_malformed_is_refused`.  Run D sent `aac` to the ALAC encode, and gave the protocol kinds `alac` and no `in_encoder` on the wire.  It also gave the page its old label and its old failure path.  It failed `errors_name_the_request_they_answer`, the 8 protocol tests of `aac` and `in_encoder`, and the 3 page tests.  The logs are in the m4/ scratch folder of the team leader (`stubA.log` to `stubD-*.log`).  TD-92 is the CI TD.
* [x] Step 3 (2026-11-10): the memory of the AAC encode.  The tests are in `worker/src/web/tests/aac.rs`, with the fake encoder.  From 10 s to 60 s, the peak grew by 8,616 bytes for 2,154 more packets on each path: 4 bytes for each packet.  The peaks were 51,023 and 59,639 bytes on the storage path, and 34,598 and 43,214 bytes on the memory path.  The end of an encode of 51,679 packets held 414,555 bytes on each sink, with a `moov` of 414,265 bytes.  So the sizes and the `moov` hold 12 bytes for each packet.  The end passes the loop at about 6,200 packets (2.4 minutes) on the storage path, and about 4,100 packets (1.6 minutes) on the memory path.  The first failures, in Chrome: a stub that also keeps each packet in WebAssembly memory grew each path by 30,137 bytes, and failed both growth tests.  A stub that makes the sizes with `Vec::new()` grew the paths by 14,336 and 21,504 bytes, and failed both.  A stub that keeps a second `moov` at the end held 828,580 bytes, and failed the end test.  The JS heap: the worker has no `performance.memory` in Chrome (the test logs it, and TD-11 says so).  So the two queue tests of Step 2 are the bound of the JS side.
* [x] Step 4 (2026-11-10): the real encode.  `a_real_aac_decodes_to_the_source` is in `worker/src/web/tests/aac.rs`.  It encodes 10 s of two sines and noise on both paths, and decodes the file with Symphonia.  The lag is computed over 16,384 frames from 1 s.  The delay was 2112 frames in both browsers, so each skip was 4160.  Chrome 157 gave a lag of 0, an error of 0.0275 over the first 4096 frames and 0.0135 over the whole, at 255 kbit/s.  Safari 27.2 gave a lag of 0, and errors of 0.0091 to 0.0092 over the start and over the whole, at 262 kbit/s.  The bounds are 0.1 for the start and 0.05 for the whole, more than 3 times the larger measure.  The first failures: with `LEAD_IN` 0, Safari failed the start check with an error of 0.4997, and Chrome passed, as the plan says.  With a skip of 0, both browsers failed the lag assert with a lag of 1053.  The test logs "skipped: this browser has no AAC encoder" where `isConfigSupported` fails, and `SKIP_ALLOWLIST` names it (VERIFY-1, TD-92).
* [x] Step 5 (2026-11-10): the page offers AAC.  The probe is in the new module `app/src/aac_probe.rs`.  `StemApp::probe_aac` runs it at start and keeps its result.  `StemApp::save_formats` gives the items of the Save menu from that result, and `the_save_menu_offers_aac_after_a_probe_that_passes` calls it.  The first failures: in the native tests, a stub `offered` that keeps AAC, and a wrong label, failed both export tests.  In Chrome, a `save_formats` of `Format::ALL` failed the menu test (5 labels before the probe).  A probe that always passes failed the probe test on "absent".  `encode_aac` that posts `alac` failed its separator test.  `Format::Aac` sent to `encode_alac` on both paths failed `worker_saves_post_paths_then_offer_the_workers_file` and `a_memory_save_posts_the_arrays_of_its_format`.  The test of an encoder failure now posts an AAC save.  `e2e save --browser chrome` passed, and each page logged `export: AAC encoder: yes`.
* [x] Step 6 (2026-11-11): `xtask e2e save` saves AAC.  `FORMATS` has the form `(kind, extension, row)`, and each check keys a save by its kind.  A run in Chrome took 231 s and one in Safari 217 s, each with two AAC saves, and both passed.  The AAC saves took 0.7 s and 5.9 s in Chrome, and 2.1 s and 10.2 s in Safari, below the slowest saves of `SLOWEST_SAVE_S`.  So the caps stay.  The worker grew by 65,536 bytes for 1 minute and 393,216 bytes for 10 minutes in each browser.  The files were 1.5 and 16.2 MB in Chrome, and 1.9 and 19.4 MB in Safari.  Each AAC decoded to the mix's frames exactly, with its peak at lag 0.  The errors over the whole were 0.00028 and 0.00026 in Chrome, and 0.00026 and 0.00025 in Safari.  Over the first 4096 frames they were 0.00072 in Chrome and 0.00019 in Safari.  At lags of -1 and 1 the error was about 0.0032, and at -2 and 2 about 0.0063, in both browsers.  So the margin at lag 1 is about 11 times.  The bounds are 0.002 over the whole and 0.005 over the start.  The run of Safari prints the calibration and the 15 checks of the JS heap as "not measured in Safari".  `WAITS_S` is 3,595 s, so `STEP_LIMIT_S` and the CI step are 60 minutes, and the CI job 85.  The first failures: 8 unit tests failed with stubs of `formats`, the folder check, `aac_rule`, a raw cross-correlation, `file_checks`, `reads_back`, the probe waits and the step limit.  The tests of the error bounds failed with open bounds.
* [x] Step 7 (2026-11-11): the numbers and the close.  The wasm bundle is in Surprises.  `e2e decode --browser chrome` passed.  The check with Apple's tools is in Surprises.  The parent's M4 section, line by line:
  * "The stem worker encodes with WebCodecs `AudioEncoder` (`mp4a.40.2`, 256 kbps)": `worker/src/web/aac.rs` and `stem_protocol::AAC_ENCODER`.  `a_real_aac_decodes_to_the_source` runs it in Chrome and Safari.
  * "the `m4a` writer makes the M4A with M2's `esds` sample entry and an `elst` for the encoder delay": `aac_end` makes the `moov` with `SampleEntry::Aac`, one `elst` entry and an `iTunSMPB` tag.  `an_aac_is_the_m4a_of_its_packets` checks the skip.
  * "M4 picks one and records it": `Order::MoovLast`, in the Decision log.  `an_aac_moov_last_head_is_36_bytes_at_each_length` tests the head.  `Layout` did not change.
  * "`e2e save` keys `FORMATS`, `Files`, `reads_back`, `saves_folder_check` and the worker check by the worker's kind": `FORMATS` is `(kind, extension, row)`.  `the_run_reads_back_the_1_minute_files_and_each_m4a` and `the_saves_folder_holds_11_files_with_aac` test it.
  * "An AAC encode error gets its own label": `encoding_label` gives "AAC encoding".  `an_aac_encode_error_names_aac_encoding` tests the `error` reply, and `a_failed_aac_encoder_says_so_and_does_not_encode_again` the stored encode.
  * "The menu shows "M4A (AAC 256 kbps)" only when `AudioEncoder.isConfigSupported` passes and a short test encode gives a packet": `app/src/aac_probe.rs` and `StemApp::save_formats`.  `the_aac_probe_passes_only_with_a_packet_in_time` tests it.
  * "The encode respects the encoder's queue": `aac_wait_queue`, and the output wait of `OUT_MOST`.  `an_aac_waits_for_the_encoders_queue` and `an_aac_waits_for_the_encoders_output` test them.
  * "A test with the encoder stubbed checks both menu states.  It runs in CI": `the_save_menu_offers_aac_after_a_probe_that_passes`, in the app's lib tests of the `browser-tests` job.
  * "`e2e save --browser safari` on the Mac: the AAC save decodes through M2's path to the source's length, with its cross-correlation peak at lag 0": the Safari run of Step 6.
  * "CI cannot run the real encode ... add a TD entry": TD-92.


* [x] Review round 1 fixes (2026-11-11).  Each test below failed first on an assert, with the fix undone:
  * 1-1: `an_encoder_error_in_the_output_wait_ends_the_encode` failed with "the encode did not end within 3 s" without the error check in the wait loop.
  * 1-10: `a_queue_that_does_not_move_ends_the_encode` failed the same way without the time limit of the queue wait.  The wait now uses the time limit of the output wait.
  * 1-2: the probe test, with the forms "empty flush" and "error flush", failed on "empty flush" without the packet check.  It failed on "error flush" without the error check.
  * 1-3: with a new first statement in `save_once`, `the_real_list_knows_2_17_and_not_1_41_or_1_54` failed, since it reads the first statement from the real file.
  * 1-9: `the_aac_label_names_the_bit_rate_of_the_encoder` failed with a label of 255 kbps.  It is a browser test, since `app` takes `stem-protocol` only for wasm32.  The worker keeps its literal 2 channels, since its input is stereo by construction.  A `const` assert keeps `AAC_ENCODER.channels` at 2.
  * The missed mutants: three new tests of `m4a/tests/mp4.rs` failed with each viable mutant of `esds.rs` lines 18, 80 and 119.  The mutant `1 + 1 - 3` of line 18 does not compile.  The new unit tests of `save.rs` failed with each of 14 mutants applied by hand.  These were in the lag window, `error_at`, `best_lag`, the near lags and the four new pure functions.  The functions are `heap_measured`, `heap_report`, `aac_not_offered` and `worker_kind`.
  * The docs of 1-4 to 1-8, 1-12 to 1-15 changed with no test.  1-11 and the refuted mutants need no change.
* [x] Review round 2 fixes (2026-11-11).  Each test below failed first on an assert, with the fix undone:
  * 2-1: `an_encoder_error_in_the_output_wait_ends_the_encode` now waits 2.5 s for an output, with a race of 1 s.  With the old JS it failed with "the encode did not end within 1 s".  `aac_wait_output` now ends at once when the error came first.
  * 2-3: the refusals have the case of a config of 48 kHz.  With `if false` in place of the rate check, `each_aac_refusal_says_why` and `a_refused_stored_aac_is_an_encoder_failure_with_no_file` failed.
  * 2-2: with `WASM_BINDGEN_NO_DEBUG=1`, `a_read_back_is_copied_into_wasm_memory_once` passed in Safari 27.2.  `an_f16_variant_the_browser_rejects_keeps_f32_weights` failed on its assert of "pars", with WebKit's text "Expected a Identifier, but got a {".  The assert now takes the text of each browser.  Then both tests passed in Safari 27.2 with `WASM_BINDGEN_NO_DEBUG=1`, and both passed in Chrome 157.  TD-93 records it.
  * The docs of 2-4 and 2-6 to 2-10 changed with no test.  2-5 needs no change.
* [x] Review round 3 fixes (2026-11-11).  Each test below failed first on an assert, with the fix undone, in Chrome 157 and in Safari 27.2:
  * 3-1: the fake has the option `error_microtask`, and `an_encoder_error_in_the_output_wait_ends_the_encode` uses it, so the error comes before the wait in each browser.  Without the fix of 2-1, it failed with "the encode did not end within 1 s" in both browsers.
  * 3-2: the fake checks `error_after` in its stuck-queue branch too.  `an_encoder_error_in_the_queue_wait_ends_the_encode` failed the same way in both browsers with `s.wake = on;` deleted.
  * 3-3: each flush has the time limit of the output wait: the probe's encoder and decoder, and the encoder of the save.  `a_flush_that_does_not_end_ends_the_encode` failed in both browsers with plain flushes.  With only the decoder's limit removed, it failed on the decoder case in Chrome.
  * 3-7: the f16 test checks for "Expected a Identifier".  It passed in Safari 27.2 with `WASM_BINDGEN_NO_DEBUG=1`, and in Chrome 157.
  * The docs of 3-4 to 3-6 and 3-8 changed with no test.
* [x] Review round 4 fixes (2026-11-11).  4-1: the fake has the option `probe_flush_never`, and `a_flush_that_does_not_end_ends_the_encode` has a case of the probe's encoder.  With `await s.enc.flush();` in place of `await aac_flush(s, ms);` in `aac_probe`, it failed with "the encode did not end within 3 s" in Chrome 157 and in Safari 27.2.  The nits 4-2 and 4-3 are reported, not fixed.
## Surprises and discoveries

* 2026-11-10, team leader: Safari encodes the first 1024 input frames as silence, and its description is a whole `ES_Descriptor`, not the bare config ("Facts").
* 2026-11-10, team leader: with the default `bitrateMode`, Safari gave 72 kbit/s for a target of 256 kbit/s.  `bitrateMode: "constant"` gives 259 kbit/s.
* 2026-11-10, team leader: the wasm bundle at `69e63db`, from `trunk build --release`: `app_bg.wasm` 6,415,004 bytes, `decode-worker_bg.wasm` 1,381,067 bytes, `stem-worker_bg.wasm` 1,295,922 bytes.  So the step before Step 1 is done.
* 2026-11-10, team leader: `e2e save --browser safari` at `69e63db` fails in 147 s.  The calibration and the 12 checks of the JS heap fail, since Safari gives no `performance.memory`.  Each other check passes, the 10-minute M4A bit for bit included.  Log: `m4/e2e-save-safari-base.log` in the orchestrator's scratchpad.
* 2026-11-10, implementer (Step 3): the stem worker has no `performance.memory` in Chrome, as TD-11 says.  So the JS side of the AAC encode has no heap measure, and the two queue tests of Step 2 bound it.
* 2026-11-11, implementer (Step 6): the raw cross-correlation of the slow planted sines with an exact copy peaked at lag 1, over a 2-second mix.  The energy of a shifted window differs, so a raw sum can favour a lag that is not the true one.  `e2e save` now uses the normalised cross-correlation (Decision log).
* 2026-11-11, implementer (Step 6): `KNOWN_MISSED` in `xtask/src/review.rs` names code lines of `save_once` and `restore`.  Step 6 changed both.  The entry of `restore` follows its new return type, with the two mutants `Ok(0)` and `Ok(1)`.  The entry of `save_once` names its first statement, where cargo-mutants puts the body mutant.  Finding 1-3 of review round 1 found that an alias had pushed that statement down.  The test of the real list now reads the first statement from the real file.
* 2026-11-11, implementer (Step 7): the wasm bundle at `600b0aa`, from `trunk build --release`: `app_bg.wasm` 6,425,776 bytes (+10,772), `decode-worker_bg.wasm` 1,381,067 bytes (no change), `stem-worker_bg.wasm` 1,332,274 bytes (+36,352, 2.8%).
* 2026-11-11, implementer (Step 7): Apple's tools take the 1-minute AAC save of `e2e save --browser chrome`.  `afinfo` gave "2646000 valid frames + 4160 priming + 976 remainder", so the priming is the skip.  `afconvert -f WAVE -d LEI16` decoded it to 2,646,000 frames, with its normalised cross-correlation peak at lag 0 and an error of 0.00071.  The worktree was in the scratch folder, and its files and the worktree are removed (ASSET-1).
* 2026-11-11, implementer (Step 7): with `bitrateMode: "constant"`, Chrome gave 203 kbit/s for the planted sines (`afinfo`: 1,523,787 audio bytes in 60 s), and 255 kbit/s for the noise of Step 4.  Safari gave about 259 kbit/s for both.  So Chrome treats 256 kbit/s as a most for simple music.  The label "256 kbps" names the rate that the save asks for: 256,000 bit/s in constant mode (`stem_protocol::AAC_ENCODER`).  Chrome can give less for simple music.
* 2026-11-11, implementer (before review round 1): `web-test --browser safari` failed `old_save_files_are_removed_and_new_ones_kept`, a test of M8.  Safari gives `lastModified` in whole seconds, and the test wrote its two files 60 ms apart.  So both were older than `between` in Safari.  The test now waits 1.1 s either side of `between`, and passes in Safari.  The stop of `web-test` at the app crate kept the other crates from their Safari run until then.
* 2026-11-11, implementer (before review round 1): in the second `web-test --browser safari`, the app, decode-worker and protocol crates passed.  The worker passed 101 of 104 tests, each AAC test among them.  `an_f16_variant_the_browser_rejects_keeps_f32_weights` and `a_read_back_is_copied_into_wasm_memory_once` fail in wgpu: Safari 27.2 gives no `subgroupMinSize`, and the debug build of wasm-bindgen asserts a number.  Both fail the same way at `69e63db`, so they are older than M4.  `a_stored_file_removed_after_it_was_opened_is_a_read_error` is the Safari flake of `references/local-browser-testing.md`, and passes alone.
* 2026-11-11, team leader (review round 3): two reviewers, the reviewer and the wire-reviewer, both named their worktree `target/review/3-reviewer-1`, and each removed it.  `review check 3` printed "no change", and the main tree stayed clean.  So no result of the round is wrong.  From round 4, each brief names a unique worktree path, and a reviewer stops if `git worktree add` fails.

## Decision log

* 2026-11-10, agent (team leader): the AAC M4A has its `moov` last (`Order::MoovLast`).  It does not reserve a head for an upper bound of packets with a `free` box.  Reason: see "The design of the encode".
* 2026-11-10, agent (team leader): the worker measures the encoder's delay at each encode with an impulse through `AudioEncoder` and `AudioDecoder`.  Reason: WebCodecs does not give the delay, and it differs between encoders.
* 2026-11-10, agent (team leader): the worker puts 2048 frames of silence before the source.  Reason: Safari encodes the first 1024 input frames as silence.  2048 keeps one packet of margin, and the trim removes it.
* 2026-11-10, agent (team leader): the encoder config has `bitrateMode: "constant"`.  Reason: Safari's variable mode gave 72 kbit/s, so the label "256 kbps" would be wrong there.
* 2026-11-10, agent (team leader): the save writes an `iTunSMPB` tag beside the `elst` entry.  Reason: D7 trims either, and Apple's tools read `iTunSMPB`.
* 2026-11-10, agent (team leader): the page probes the encoder itself at start, with no worker message.  Reason: both browsers give `AudioEncoder` on the page and in a worker.  The worker's own probe fails a save with the "AAC encoding" label.
* 2026-11-10, agent (team leader, plan review round 1): the skip of the real-encode test gets a `SKIP_ALLOWLIST` entry (VERIFY-1).  The label test of "AAC encoding" moves to Step 2, with the label.  Each `AudioData`, encoder and decoder is closed, with a test (MEM-4).  The encoder config is one constant in `stem-protocol` (LAYER-1).  WebCodecs goes through inline JS, not `web-sys`.  A CI test links the probe to the menu.  `e2e save` also checks the error at lags near 0.
* 2026-11-10, agent (team leader, plan review round 2): `Encoded::Failed` gets a field for an encoder failure.  So a failed stored AAC encode shows "AAC encoding failed", with no second encode.  Reason: the storage path is the main path, and its text would blame storage.  The config constant is the only copy, outside `mod kind`.  The worker refuses a rate with no AAC index and the 960-frame flag.
* 2026-11-10, agent (team leader, the last plan review): the encode also bounds the outputs in flight (`OUT_MOST`).  `encodeQueueSize` alone does not bound the JS chunks.  `m4a::AacConfig` reads the config's fields, a second reader beside `engine::aac_config` (LAYER-1).  `e2e save` waits for the probe of each page, and takes the lag on the first 10 s only.
* 2026-11-10, agent (implementer, Step 2): the AAC encode is in its own module, `worker/src/web/aac.rs`, with its inline JS.  Its tests are in a child module of the worker tests, `worker/src/web/tests/aac.rs`, so they use the helpers of `tests.rs`.  Reason: `encode.rs` and `tests.rs` are long already (TD-18).
* 2026-11-10, agent (implementer, Step 2): the yield after each block is a message on a `MessageChannel`, not a `setTimeout` of 0.  Reason: browsers clamp a nested `setTimeout` to 4 ms, which adds about 26 s to the 6,460 blocks of 10 minutes.
* 2026-11-10, agent (implementer, Step 2): the probe makes its impulse in JS.  Reason: its 128 KiB stay out of WebAssembly memory, which never shrinks.
* 2026-11-10, agent (implementer, Step 2): the field of `Encoded::Failed` is `in_encoder`, on the wire too.  A value that is not a boolean is refused, and a missing one is a storage failure (TD-16).  The page takes the label of an encoder failure from the kind of the reply, through one function, `encoding_label`, which the `Event::Error` arm also calls.  Reason: Step 2 has no `Format::Aac` yet.
* 2026-11-10, agent (implementer, Step 2): until Step 5, the page test of an encoder failure posts an M4A (ALAC) save and replies with `aac`.  Step 5 posts an AAC save.
* 2026-11-11, agent (implementer, Step 6): `FORMATS` has the form `(kind, extension, row)`.  The checks of the JS heap are a function of their own, `heap_checks`, so that a run in Safari prints each as not measured.  Reason: Safari gives no `performance.memory` (TD-11).
* 2026-11-11, agent (implementer, Step 6): `e2e save` uses the normalised cross-correlation, over 10 s from frame 1024.  Reason: the raw cross-correlation of the slow planted sines peaked at lag 1 for an exact copy of a 2-second mix, since the energy of the shifted window differs.  The window keeps 1024 frames from each end, so each lag reads whole frames.
* 2026-11-11, agent (team leader): the two GPU tests that fail in Safari 27.2 do not block review round 1.  Reason: they fail the same way at `69e63db`, and M4 does not touch the GPU.  TD-93 records them.
* 2026-11-11, agent (implementer, review round 1): the wait for the encoder's queue ends the encode after the time limit of the output wait, with "AAC encoder: the encoder's queue did not move".  Reason: an encoder whose queue never moves would hang the encode, and the page would get no reply (finding 1-10).

## Outcomes and retrospective

Not started.
