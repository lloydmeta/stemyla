# M4A input and output

## Purpose

Today the app opens and saves WAV, MP3 and FLAC only.  After this plan, the app opens an M4A file as a song or as a stem.  The M4A can hold AAC-LC or Apple Lossless (ALAC).  The app also saves a stem or the mix as a 16-bit ALAC M4A.  A later, optional milestone saves AAC-LC where the browser has an AAC encoder.

M4A is the usual format of the Apple tools.  The iTunes Store sells DRM-free 256 kbps AAC ([Apple, 2007](https://www.apple.com/newsroom/2007/05/30Apple-Launches-iTunes-Plus/)).  Logic Pro imports AAC and Apple Lossless, and its list of compressed formats has no FLAC ([Logic Pro 10.1 help](https://help.apple.com/logicpro/mac/10.1/en.lproj/lgcp4f68cbe3.html)).  A guide tells GarageBand users to convert FLAC to M4A first ([MacMost](https://macmost.com/5-ways-to-convert-audio-files-on-your-mac.html)).  So a FLAC stem from the app does not go directly into these tools, but an ALAC M4A does.

This plan was written in a cloud session on 2026-11-08 as a handoff.  The maintainer starts it on the Mac with the `orchestrate` skill (see "How the work runs").

## Context and orientation

### The code today (commit `15c118d` on `main`)

* `engine/src/decode.rs`: Symphonia 0.6.1 with the features `flac`, `mp3`, `wav` and `pcm` (`engine/Cargo.toml`).  `most_frames` caps the reservation of each channel for MP3 and FLAC.  `head_length` reads the length from the head of a WAV or a FLAC; for other files it gives `HeadLength::Unstated`.  `largest_file` gives the largest file of a stated length.  `MAX_DECODED_BYTES` is 1 GiB (decision 0013).
* `engine/src/song.rs`: `Song::load_folder` takes `wav`, `mp3` and `flac` only.
* `app/src/lib.rs`: `SONG_FILES` (`".mp3,.wav,.flac"`) is the picker's `accept`.  The two `WELCOME` texts name WAV, MP3 and FLAC.  `MEMORY_PATH_MAX_FRAMES` is the limit of the memory path.
* `app/src/export.rs`: `Format` has `Wav`, `Flac` and `Mp3`.  `Format::ALL`, `extension` and `label` make the Save menu.  A test checks the extensions `["wav", "flac", "mp3"]`.
* `app/src/store.rs`: `save_sources` keeps each source as `src/<i>.<ext>`.  `rebuild` decodes it with that extension as the hint.
* `decoder/src/source.rs`: `BlockSource` reads a stored file in blocks, and can seek.  So an M4A with `moov` after `mdat` opens in the decode worker.
* `worker/src/web/encode.rs`: the MP3 and FLAC encoders of a save.  A FLAC writes its header first, then each frame, then the header again at position 0 (`FlacOut::replace_header`).  The memory path gives a `Blob` of parts.  The storage path writes through a `FileSystemSyncAccessHandle` (MEM-5).
* `protocol/src/lib.rs`: `Request::Mp3`, `Request::Flac`, `Reply::Mp3` and `Reply::Flac`, with the kinds `mp3` and `flac` (PROTO-1).
* `xtask/src/rules.rs`: `AUDIO_EXTENSIONS` already has `m4a`, `aac`, `aif` and `aiff`, so ASSET-1 already refuses those files in git.
* `xtask/src/serve.rs`: a MIME type for `wav`, `mp3` and `flac` only.
* `.github/workflows/`: every job runs on `ubuntu-latest`.  The browser jobs run headless Chrome.
* LAYER-1 and decision 0005: `stem-worker` depends on no page-side crate, so it cannot use `engine`.

### What Symphonia 0.6.1 gives

The planning session read the published crates (`symphonia-codec-aac`, `symphonia-format-isomp4`, both 0.6.1, MPL-2.0).

* The features `isomp4`, `aac`, `alac`, `aiff` and `caf` exist in 0.6.1.  There is no `opus` feature in 0.6.1, although the README on GitHub lists one.
* The AAC decoder takes AAC-LC with at most 2 channels only.  It refuses SBR (HE-AAC), other profiles and more than 2 channels with "aac: aac too complex" (`src/aac/mod.rs`, lines 90 to 97).  Some HE-AAC files signal SBR only inside the stream.  The decoder then sets `sbr_present` while it decodes.  Find out in M2 what such a file gives.
* The MP4 reader parses `edts`/`elst` into `TrakAtom`, but no code uses it.  The status table gives ISO/MP4 gapless as "No" ([Symphonia README](https://github.com/pdeljanov/Symphonia)).  So a decoded AAC keeps its encoder delay at the start.
* The encoder delay is often 2112 frames (Apple's encoders), about 48 ms at 44.1 kHz.  Other encoders use 1024 to 2624 ([Vimeo engineering](https://medium.com/vimeo-engineering-blog/a-brief-history-of-gapless-audio-and-what-you-can-do-about-it-ea9e1c343215), [AAC.js wiki](https://github.com/uupaa/AAC.js/wiki/EncoderDelay)).  An `elst` entry or an `iTunSMPB` tag gives the delay and the padding.

### Encoders and browsers

* WebCodecs `AudioEncoder`: Safari has it from version 26 ([MDN compatibility data](https://chromium.googlesource.com/external/github.com/mdn/browser-compat-data/+/HEAD/api/AudioEncoder.json)).  AAC is optional in WebCodecs ([W3C AAC registration](https://www.w3.org/TR/2026/DNOTE-webcodecs-aac-codec-registration-20260924/)).  A user reports that Chrome on Linux does not encode AAC ([mp4-muxer issue 32](https://github.com/Vanilagy/mp4-muxer/issues/32)).  This is not confirmed, because `developer.chrome.com` was not reachable from the planning session.
* `alac-encoder` 0.3.0: a Rust port of Apple's reference encoder, Apache-2.0 OR MIT ([crates.io](https://crates.io/crates/alac-encoder)).  Its last release was in January 2022.
* `oxideav-aac` 0.1.7: a pure-Rust AAC-LC decoder and encoder, MIT, first release in April 2026 ([crates.io](https://crates.io/crates/oxideav-aac)).  Its encode quality is not measured.
* `muxide` 0.5.0: an MP4 muxer whose README names AAC and Opus, not ALAC.  It depends on `serde_json` and `thiserror`.

### Risks for untrusted files

* A small ALAC file can decode to a large one, as a FLAC can (decision 0013).  `most_frames` needs an ALAC case and an AAC case.
* `largest_file` takes the whole file as audio.  An `.mp4` video is mostly video, so the check would refuse it.  So this plan opens `.m4a`, `.aac` and an `.mp4` that holds only sound tracks.  An `.mp4` with a video track gives its own message (D4).
* A file whose `moov` starts after the 64 KiB head has no length in the head.  It is `HeadLength::UnstatedPcm`, not `Unstated`: `Unstated` prices a file at the MP3 rate, so a long ALAC file with `moov` last (FFmpeg's default) would be refused as too large.  The PCM rate bounds ALAC and AAC from above.
* A file with FairPlay DRM (`.m4p`, a `drms` or `enca` sample entry) cannot decode.  It gets its own message.

### Terms

* M4A: an MPEG-4 file (ISO base media file format) that holds one audio track.
* Box: one unit of an MPEG-4 file, a size, a 4-letter type and a body.  `moov` holds the tables; `mdat` holds the coded audio.
* Packet: one coded unit of the track.  An ALAC packet holds 4096 frames here.  An AAC packet holds 1024 frames.
* Encoder delay: the frames at the start of a decoded AAC that are not in the source.  Padding: the same at the end.
* Fast start: `moov` before `mdat`, so a reader needs no seek to the end.

## Plan of work

Each milestone is one or more signed commits on one branch and one PR.  Each milestone runs the review rounds of decision 0024.  Each milestone fixes the docs that it makes wrong (DOC-1).

### How the work runs

#### Mode

Orchestrated mode (decision 0023), with the `orchestrate` skill.  The plan has more than 3 milestones, and D1 changes M3 and M4.  One team leader runs each milestone.

#### Branch and PR

The branch is `feature/m4a`, from `main` at `ff633a3`.  Its first commit is this plan.  The orchestrator opens one draft PR into `main` after M1's first commit.

#### Maintainer steps

* Answer the open entries of "Decisions for the maintainer".
* None that blocks the run.  After M3, the orchestrator decodes a saved ALAC M4A with Apple's decoder (`afconvert -f WAVE -d LEI16`) on the Mac, and compares it with the WAV save bit for bit.  Opening it in Logic Pro is optional evidence for the maintainer.
* For M4 only: if Chrome on the Mac cannot encode AAC, Safari 26 or later with "Allow Remote Automation" (already set on 2026-10-21).

#### Preflight

The skill's preflight, and:

* `cargo test -p engine` passes on the branch before M1.
* `cargo run -p xtask -- e2e decode --browser chrome` and `e2e save --browser chrome` pass before M1.  These are the baselines that M2 and M3 extend.

#### Decision points

See "Decisions for the maintainer".

#### Review rule

Decision 0024, through the `review-rounds` skill.

#### Kick-off prompt

```text
/orchestrate docs/plans/active/m4a.md
```

#### Extra stop conditions

* None beyond the skill's.  The orchestrator decides these cases, records them in the Decision log and the PR, and goes on:
  * A Symphonia 0.6.1 limit blocks an acceptance check, for example a decode panic on a valid fixture.  Do not patch or fork Symphonia.  Narrow the case, give a clear message, and add a TD entry.
  * A new crate fails DEPS-1.  Use another source, such as a fixture from the M1 writer or an encoder of this repo, and record it.

### M1: the M4A writer and the ALAC encoder

Write the M4A box writer and a 16-bit stereo ALAC encoder, natively, with no browser code.  The location follows D2 and D6.

* The writer makes `ftyp`, `moov` (`mvhd`, `trak`, `tkhd`, `mdia`, `mdhd`, `hdlr`, `minf`, `smhd`, `dinf`, `stbl` with `stsd`, `stts`, `stsc`, `stsz`, `stco`) and `mdat`.  The sample entry is `alac` with its 24-byte `ALACSpecificConfig`.
* The writer gives a fast-start file.  The packet count comes from the frame count, so the size of `moov` is known before the first packet.  The writer reserves that size, writes `mdat`, and then writes `moov` at the reserved place.  This is the same order as `FlacOut::replace_header`.
* The writer can also write an `elst` entry, for the tests of M2 and for M4.
* `stco` holds 32-bit offsets.  The writer refuses a file whose worst case is 4 GiB or more (`plans/completed/m4a-m1.md`, Decision log).  A 1 GiB decode limit keeps a 16-bit save far below that.
* Write decision 0028 as `accepted`: M4A with ALAC as the save format, the location of the writer, and the safe port.  The maintainer's answers to D1, D2 and D6 accept it (Decision log).
* `THIRD-PARTY.md` names each new crate (DEPS-1).
* Acceptance: native tests that fail before the change.
  * A writer test round-trips through Symphonia (`isomp4`, `alac`) bit for bit: silence, a full-scale square wave, a mono-like and a random 16-bit signal, and lengths of 0, 1, 4095, 4096 and 4097 frames.
  * A test checks that `moov` comes before `mdat`, and that each `stco` offset points at its packet.
  * A test checks the refusal at 4 GiB, without a 4 GiB buffer.

### M2: open M4A (and AIFF if D5 says so)

* Turn on the Symphonia features `isomp4`, `alac` and `aac` in `engine` (and `aiff` by D5).
* First, extend the `m4a` crate (D6: no copy of the writer in a test).  Each change gets a test that fails first.
  * An `esds` sample entry for AAC, with its `AudioSpecificConfig` and channel count.  `Layout` gets the codec's frames per packet, packet bound and channel count, in place of the ALAC constants.  The AAC-LC, HE-AAC and 6-channel fixtures use it; their packets come from D3 or are dummies where only the message is tested.
  * An optional `iTunSMPB` tag (`udta`, `meta`, `ilst`), counted in the head's length.  M4 can use it too.
  * Fix the comment of `SampleEntry` and the Consequences of decision 0028, which give `esds` to M4.
* Fixtures that the writer does not make are crafted in the test from the writer's output: a `drms` or `enca` entry (patch the 4-byte type), and a video track (a second `trak` whose `hdlr` is `vide`, appended to a `MoovLast` file; and a single track patched from `soun` to `vide`).
* The over-limit ALAC fixture uses `Layout` and `AlacEncoder` directly: one silent packet, its size repeated.  It does not call `alac_file`, which needs the whole input.
* `most_frames`: an ALAC case and an AAC case, with the cap of decision 0013 on the stated length.
* `head_length`: an `mp4_length` case that reads `mdhd`, `hdlr`, `stsd` (and `elst`) from a `moov` that starts in the head.  These boxes sit in the first few hundred bytes of `moov`.  The `moov` can end after the 64 KiB head: the writer adds 8 bytes for each packet, so a save of more than about 12.6 minutes at 44.1 kHz passes the head, and so does the over-limit fixture.  So `mp4_length` reads a `moov` cut off at the end of the head.  Any MP4 whose head does not show the first `soun` track's `mdhd` gives `HeadLength::UnstatedPcm` (see "Risks for untrusted files"): a `moov` after the head, or a `moov` whose `mdhd` is past the head, such as after large cover art.  A raw ADTS `.aac` file has no `moov`, and gets `UnstatedPcm` too.  Fix the comment at `engine/src/decode.rs` that lists the cases.  The stated length counts the AAC encoder delay, so a song just at the limit can be refused; that is accepted.
* `mp4_length` takes `mdhd` from the `soun` track.  When the head shows a track whose `hdlr` is `vide`, the head check refuses the file with the D4 video message, so the page never reads a whole video.  This needs a new `HeadLength` variant and a refusal other than `TooLongToOpen`, in both callers of the head check (`app/src/lib.rs` and `app/src/store.rs`).  A video track that the head does not show gets the message from the decode instead.
* `mp4_length` accepts a version 1 `mdhd` (64-bit times), and an `mdat` of size 0 (to the end of the file) or 1 (a 64-bit size) as it walks the boxes.
* The encoder delay and the padding: `engine::decode` reads `elst`, then `iTunSMPB` (by D7), through the same seekable source, and trims both.  So the decoded length equals the source's length.
* Messages: a DRM file gives "protected, does not open".  An HE-AAC file gives "HE-AAC does not open; use AAC-LC".  A file with more than 2 AAC channels gives its own message.
* `SONG_FILES`, the `WELCOME` texts, the hover text of "Open song…", `Song::load_folder`, `xtask serve` (`audio/mp4`, `audio/aac`, and `audio/aiff` by D5).
* The fixtures come from the `m4a` writer (ALAC, and AAC with the `esds` entry above) and from D3 (the AAC packets).  No audio goes into git (ASSET-1).
* An extra local check, not in CI: the maintainer's real AAC-LC file (see Surprises; its path is in the M2 brief, not in the repo).  It adds to the acceptance checks below and replaces none of them.  It has no `iTunSMPB` and a delay of 1024 frames, so the synthetic fixtures still cover `iTunSMPB`, the delay of 2112 and each other case.  `engine::decode` gives 13,413,120 frames, as Apple's `afinfo` says.  Its cross-correlation with Apple's decode (`afconvert -f WAVE -d LEI16`) peaks at lag 0.  Never copy the file into the repo (ASSET-1).
* Measure the size of the wasm bundle before and after (`trunk build --release`).  Put both numbers in Surprises.
* Acceptance:
  * Native: an ALAC M4A decodes bit for bit.  An ALAC M4A with an `elst` delay of 2112 frames decodes to the source's frames and length, bit for bit.  An AAC-LC M4A decodes to the source's length, and its cross-correlation with the source peaks at lag 0.  A file with `moov` after `mdat` decodes.  The DRM, HE-AAC and 6-channel fixtures give their messages.  A crafted ALAC file that states more than the limit is refused before it decodes.
  * `cargo run -p xtask -- web-test`: the decode worker decodes an M4A in blocks, as native does.
  * `cargo run -p xtask -- e2e decode --browser chrome`: one of its songs is an M4A, and its session files match the native decode.

### M3: save as ALAC M4A

* `stem-protocol`: a request and a reply of kind `alac`, as `flac` has, with one encoder and one decoder (PROTO-1).  The hand-written scripts of `xtask e2e` change in the same commit (TD-16).
* `stem-worker`: an `alac` encode on the memory path (a `Blob` of parts) and on the storage path (an `OutFile`, the `moov` written last at the reserved place).  Each write is a whole new `Uint8Array` (MEM-5).  The encode holds no song-sized buffer (MEM-1, MEM-4).
* `app`: `Format::M4a` with the label "M4A (16-bit Apple Lossless)", in `Format::ALL` and in the Save menu.
* Licence: from M3, the web app ships the Apache-2.0-only code of `m4a` (decision 0029, `proposed`).  Do the Consequences of 0029, such as the licence list, in M3.  The maintainer accepts 0029 at the review of the PR; the run does not wait for it.
* Acceptance:
  * `worker/src/web/tests.rs`: an ALAC encode in blocks decodes to the same 16 bits as `engine::wav::encode_pcm16`, on both paths.  Its peak memory grows with the length by the packet table only: the worker keeps 4 bytes for each packet, and `head()` makes `moov` at about 8 bytes for each packet.  That is about 12 bytes for each 4096 frames, or about 77 KB for 10 minutes.  The test allows that and nothing more.  Do not stream `moov`.  To keep to it, make the vector of packet sizes with `Vec::with_capacity(layout.packets())`, and write the placeholder of `head_len()` zeros in small fixed blocks, not as one buffer.
  * `cargo run -p xtask -- e2e save --browser chrome`: the M4A saves of 1 and 10 minutes meet the same limits as FLAC.  These are the JS heap, the WebAssembly memory, the longest slice and the worker's memory.  Each saved file decodes to the WAV save's samples, bit for bit.
  * The extension test in `app/src/export.rs` lists `m4a`.

### M4: save as AAC-LC (by D1)

Only if D1 takes the AAC save.  The stem worker encodes with WebCodecs `AudioEncoder` (`mp4a.40.2`, 256 kbps), and the `m4a` writer makes the M4A with M2's `esds` sample entry and an `elst` for the encoder delay.

* The packet count of an AAC encode is not known exactly before the encode, because the encoder adds priming and flush packets.  So M4 reserves the head for an upper bound of packets and fills the rest with a `free` box, or writes `moov` last (`Order::MoovLast`).  M4 picks one and records it.  The first needs a change of `Layout`, which today wants exactly `packets()` sizes: M4 changes `m4a` with a test that fails first.
* An AAC save also has the extension `m4a`.  So `e2e save` keys `FORMATS`, `Files`, `reads_back`, `saves_folder_check` and the worker check by the worker's kind, not by the extension.  Today `reads_back("m4a", …)` asks for a decode bit for bit, which an AAC save fails.
* An AAC encode error gets its own label in the `Event::Error` arm of `app/src/lib.rs`.  Today its `_` arm gives "M4A encoding".
* The menu shows "M4A (AAC 256 kbps)" only when `AudioEncoder.isConfigSupported` passes and a short test encode gives a packet.
* The encode respects the encoder's queue (`encodeQueueSize`, `dequeue`), so its memory does not grow with the song.
* Acceptance:
  * A test with the encoder stubbed checks both menu states.  It runs in CI.
  * `cargo run -p xtask -- e2e save --browser safari` on the Mac: the AAC save decodes through M2's path to the source's length, with its cross-correlation peak at lag 0.
  * CI cannot run the real encode if Chrome on `ubuntu-latest` has no AAC encoder.  Then add a TD entry in `docs/tech-debt.md` in the same commit (VERIFY-1).

## Decisions for the maintainer

Each entry is open until the maintainer answers.  The `orchestrate` skill asks the open entries in its one message.  The maintainer answered each entry on 2026-11-08.

* D1, the codec of the save.
  * Options: (a) ALAC in M3, and AAC through WebCodecs in M4; (b) ALAC only, no M4; (c) AAC through `oxideav-aac` in pure Rust; (d) AAC only, through WebCodecs.
  * Recommendation: (a).  ALAC is lossless and deterministic, and Symphonia checks it bit for bit, natively and in CI.  The AAC save then adds small files where the browser can encode.  (c) has an unmeasured encoder, which can sound worse than the 192 kbps MP3 that the app already saves.
  * Answer (maintainer, 2026-11-08): (a).
* D2, the ALAC encoder.
  * Options, first version: (a) `alac-encoder` 0.3.0, pinned exactly; (b) an encoder of our own, adaptive prediction and Rice coding as Apple's reference has.  First recommendation: (a), unless a review of its source finds a defect, `unsafe` code or a thread.
  * Options, revised on 2026-11-08 after a review of the source of `alac-encoder` 0.3.0: (a) `alac-encoder` 0.3.0, pinned exactly; (b) an encoder of our own, from nothing; (c) a port of the encoder of `alac-encoder` into the `m4a` crate, rewritten as safe Rust.
  * The review found `unsafe` code.  `BitBuffer::write` and `write_lte25` read and write a `u32` through a raw pointer, and `write` writes 1 more byte.  Only a `debug_assert!` checks the bit position, so a release build can write past the buffer.  `encode` casts `&[u8]` to `&[i16]` and `&[i32]` with `slice::from_raw_parts`, with no check of alignment or length.  `dp.rs` uses `transmute` on `i32::signum`.
  * Recommendation: (c).  It removes the undefined behaviour, adds no dependency and keeps a proven algorithm.
  * Answer (maintainer, 2026-11-08): (c).
  * The port keeps 16-bit stereo only: `encode`, `encode_stereo`, `encode_stereo_escape`, `magic_cookie`, `mix16`, `pc_block`, `dyn_comp` and a bit writer.  It drops mono, 20, 24 and 32 bits, and more than 2 channels.  Estimate: 650 to 750 lines.
  * Each `unsafe` site gets a safe form.  The bit writer keeps a 64-bit accumulator and pushes whole bytes into a `Vec<u8>`.  The input is `&[i16]`, interleaved, so no cast is necessary.  A `match` on `signum` replaces `transmute`.  The crate has `#![forbid(unsafe_code)]`.
  * The unrolled loops of `pc_block` for 4 and 8 coefficients become one loop, if the differential test shows the same bytes.
  * The port keeps the MIT notice of `alac-encoder` and the Apache-2.0 notice of Apple's ALAC reference, in the crate and in `THIRD-PARTY.md`.
* D3, the AAC fixtures for the decode tests.
  * Options: (a) `oxideav-aac` as a dev-dependency only; (b) WebCodecs in browser tests only.
  * Recommendation: (a).  A fixture needs no good sound, and CI's Chrome may not encode AAC.
  * Answer (maintainer, 2026-11-08): (a).
* D4, `.mp4` files with video.
  * Options: (a) out of scope; (b) in scope, with `largest_file` counting the audio track's `stsz` only.
  * Recommendation: (a).  It keeps the size check simple, and few songs come as video files.
  * Answer (maintainer, 2026-11-08): (a), with a clear message.  A file with a video track (an `hdlr` of type `vide`) gives "This file holds video.  Open an audio file, such as M4A, MP3, WAV or FLAC."  This applies also to an `.mp4` renamed to `.m4a`, and to a dropped file.  M2 tests the message.
  * Agent decision, 2026-11-08: the app also opens an `.mp4` that holds only sound tracks.  Tools such as FFmpeg write audio-only `.mp4` files, and the size check holds for them.  An `.mp4` with a video track gives the message above.
* D5, AIFF in M2.
  * Options: (a) yes; (b) no.
  * Recommendation: (a).  It costs one Symphonia feature and a header reader such as `wav_length`, and Logic and Pro Tools users have AIFF files.
  * Answer (maintainer, 2026-11-08): (a).
* D6, where the writer lives.
  * Options: (a) a new crate `m4a` in the Pure layer, with no dependencies, used by `stem-worker`, by `engine` as a dev-dependency, and by `xtask`; (b) the writer in `stem-worker`, with copies in the tests of `engine` and `xtask`.
  * Recommendation: (a).  The copies of the FLAC fix in five places (TD-52) show the cost of (b).  (a) adds a row to the layers table (LAYER-1, decision 0005).
  * Answer (maintainer, 2026-11-08): (a).  The maintainer asks that the crate agree with the writers that exist.  M1 and M3 keep these rules:
    * The `m4a` crate only makes bytes, as `engine::wav::header` and `engine::wav::write_pcm16` do.  It does no input or output.  `stem-worker` does the writes, as it does for FLAC.
    * The crate takes interleaved 16-bit samples.  The worker makes them with `pcm16`, as for FLAC.  So the rounding is that of `engine::wav::write_pcm16`.
    * The crate gives the head (`ftyp`, `moov` and the `mdat` header) twice: once with zeros to reserve its size, and once at the end.  This is the order of `FlacOut::replace_header`.
    * M3 makes `FlacOut` a sink for both codecs, with `put` and `replace_head`.  It does not copy it as an `AlacOut`.
    * Every test that needs an M4A uses this crate: `engine`, `xtask`, `decode-worker` and `stem-worker`.  No test has a copy of the writer (TD-52).
    * The crate is not DSP-1 beginner style, because DSP-1 applies to `dsp/` only.
* D7, how the decode trims the encoder delay.
  * Options: (a) `elst`, then `iTunSMPB`; (b) `elst` only; (c) no trim, with a TD entry.
  * Recommendation: (a).  Apple's files carry `iTunSMPB`, and some have no usable `elst`.  Without a trim, an AAC stem is about 48 ms late against a WAV lane.
  * Answer (maintainer, 2026-11-08): (a).

## Validation and acceptance

* `cargo run -p xtask -- check --tier push` passes.
* `cargo test --workspace --exclude roformer-gpu` passes, with the new tests of M1 to M3.
* `cargo run -p xtask -- web-test` passes.
* `cargo run -p xtask -- e2e decode --browser chrome` and `e2e save --browser chrome` pass, with the M4A cases.
* With M4: `e2e save --browser safari` passes on the Mac.
* The picker opens an ALAC M4A and an AAC-LC M4A.  Save gives an M4A that Symphonia decodes to the WAV save's samples.

## Idempotence and recovery

Each milestone is its own commits, and each commit passes `check --tier commit`.  A failed milestone is reverted with `git revert` of its commits.  The earlier milestones stay.  The Symphonia features are additive: turning them off restores today's decode.  A save of kind `alac` that the worker does not know gives `error`, so an old worker refuses it and does not write a wrong file.

## Progress

* [x] 2026-11-08: plan written in a cloud session, with the evidence of the Context.  All decisions are open.
* [x] 2026-11-08: the maintainer answered D1 to D7.  The preflight passed.
* [x] 2026-11-08, M1: the M4A writer and the ALAC encoder (`plans/completed/m4a-m1.md`).  The crate `m4a` makes a fast-start M4A of 16-bit stereo ALAC, from a safe port of `alac-encoder` 0.3.0 that gives its packets byte for byte.  Symphonia decodes each test file bit for bit.  3 review rounds (`reviews/feature-m4a-m1.md`), 28 agents, 1,895,639 tokens, 72.6 wall minutes, 0 interventions.  Late rounds: none.  Debt: TD-79.  Decisions: 0028 (accepted), 0029 (proposed).
* [x] 2026-11-10, M2: open M4A, audio-only `.mp4` and AIFF (`plans/completed/m4a-m2.md`).  ALAC and AAC-LC M4A, AIFF and AIFC (PCM and `ima4`), and IMA and Microsoft ADPCM in a WAV open.  Tags and art are hidden from Symphonia, and refused only past a memory bound.  An M4A with video, protection or HE-AAC gets a clear text.  23 review rounds (`reviews/feature-m4a-m2.md`; rounds 20 to 23 after intent review round 1), 211 agents, 13,392,805 tokens, 628.7 wall minutes, 4 interventions.  Intent review: 3 rounds, with each finding fixed.  Late rounds (4 to 23): 136 findings fixed, 15 of them blockers, and 44 merged mutant rows.  Debt: TD-80, TD-86, TD-90 open; TD-81 to TD-85 and TD-87 to TD-89 closed.
* [x] 2026-11-10, M3: save as ALAC M4A (`plans/completed/m4a-m3.md`).  The Save menu offers "M4A (16-bit Apple Lossless)".  The stem worker encodes it with `m4a` on both save paths, through one sink with FLAC.  Each file equals `alac_file` of its 16-bit samples, and Symphonia decodes it bit for bit.  `e2e save` saves M4A of 1 and 10 minutes within the limits of FLAC.  4 review rounds (`reviews/feature-m4a-m3.md`; round 4 after intent review round 2), 23 agents, 2,177,255 tokens, 50.3 wall minutes, 0 interventions.  Late rounds (4): no fixes, and 6 nits reported, not fixed.  Debt: TD-91.  Apple's `afconvert` decodes a saved M4A to the WAV save's samples, bit for bit (Surprises).
* [x] 2026-11-11, M4: save as AAC-LC (`plans/completed/m4a-m4.md`).  The Save menu offers "M4A (AAC 256 kbps)" where the page's probe passes.  The stem worker encodes through WebCodecs, measures the encoder's delay at each encode, and writes `moov` last, with `elst` and `iTunSMPB`.  `e2e save` saves AAC of 1 and 10 minutes in Chrome and Safari on the Mac.  Apple's `afinfo` gives the source's frames and a priming of the skip.  5 review rounds (`reviews/feature-m4a-m4.md`), 35 agents, 3,028,241 tokens, 92.1 wall minutes, 0 interventions.  Late rounds (4 and 5): one fix (4-1, a test), and 2 nits reported, not fixed.  Debt: TD-92 and TD-93 are new, and TD-39 names `check_heap`.

## Surprises and discoveries

* 2026-11-08: `docs.rs` and `developer.chrome.com` were not reachable from the planning session.  So the Chrome platforms with an AAC encoder are not confirmed.  M4's team checks this first.
* 2026-11-08, orchestrator: Chromium on Linux has no AAC encoder for WebCodecs.  Chromium closed [issue 40942807](https://issues.chromium.org/issues/40942807) as Won't Fix: Linux has no system AAC encoder, and licences stop FFmpeg and FDK-AAC.  So CI cannot run M4's real encode.  Chrome on macOS encodes through AudioToolbox, so M4 can try `e2e save --browser chrome` on the Mac.
* 2026-11-08, orchestrator: `AudioEncoder` is in Chrome 94, Firefox 130 and Safari 26 ([MDN browser-compat-data](https://github.com/mdn/browser-compat-data/blob/main/api/AudioEncoder.json)).
* 2026-11-08, orchestrator: `mp4-atom` 0.17.1 is maintained but has no `alac` sample entry.  So the box writer of M1 is written by hand.
* 2026-11-08, orchestrator: the session runs Opus 5.5 (the system prompt) at effort `high` (`modelSettings` in `~/.claude/settings.json`).  The preflight passed at `ff633a3`.  It ran `cargo test -p engine` (182 passed), `e2e decode --browser chrome` and `e2e save --browser chrome` (Chrome 156, chromedriver 155).
* 2026-11-08, orchestrator: the wasm bundle before M2, from `trunk build --release` at `ff633a3`: `app_bg.wasm` 5,836,422 B, `decode-worker_bg.wasm` 809,845 B, `stem-worker_bg.wasm` 1,250,988 B.
* 2026-11-08, orchestrator: the maintainer gave a real song for local checks: an audio-only `.mp4` with brand `M4A `, one `soun` track, AAC-LC, 48 kHz stereo, 279.44 s, 247 kbps, written by FFmpeg (`Lavf62.12.100`).  It is fast start.  Its `elst` gives a delay of 1024 frames, not Apple's 2112, and it has no `iTunSMPB`.  Apple's `afinfo` gives 13,413,120 valid frames, 1024 priming and 256 remainder.  Its `AudioSpecificConfig` (`11 90 56 e5 00`) also has the sync extension `0x2b7` with SBR not present, as FFmpeg writes it.  So it tests the `elst` trim and the parse of that extension.
* 2026-11-08, M1: Symphonia 0.6.1 decodes each M1 file bit for bit, the file of 0 frames included (`m4a/tests/round_trip.rs`).  So no Symphonia limit was found in M1.
* 2026-11-08, M1: Symphonia 0.6.1 ignores `elst`.  A file with an edit decodes to every frame of its track.  So M2 must trim the delay itself (D7).
* 2026-11-08, M1, for M2: `cargo test --workspace` builds one Symphonia with the features of every crate.  The dev-dependency of `m4a` turns on `isomp4` and `alac`, so `engine`'s tests decode M4A in that build before M2 turns the features on.  M2 shows its first failures with `cargo test -p engine`.
* 2026-11-08, M1: a 10 s music-like test signal gives an ALAC M4A of 0.553 times the size of its 16-bit WAV.
* 2026-11-10, M2: the wasm bundle after M2 (a1330ef; rounds 21 to 23 changed only tests), from `trunk build --release`: `app_bg.wasm` 6,405,994 B (+569,572), `decode-worker_bg.wasm` 1,381,138 B (+571,293), `stem-worker_bg.wasm` 1,250,988 B (no change).  On 2026-11-08, at the end of the first implementation, they were 6,313,749 B and 1,294,325 B.  The new Symphonia crates (`isomp4`, `alac`, `aac`, `aiff`) and the box reader are in the two modules that decode.
* 2026-11-08, M2: the local check of the maintainer's AAC-LC file passed.  `engine::decode::decode_path` gives 13,413,120 frames at 48 kHz, as `afinfo` says, and its cross-correlation with Apple's decode (`afconvert -f WAVE -d LEI16`) peaks at lag 0.  The `elst` trim does it: the file's `segment_duration` is in an `mvhd` timescale of 1000.  With the trim off, the decode gives 13,414,400 frames.  The test is `engine/tests/local_aac.rs`, ignored; the files are not in the repo.
* 2026-11-10, M2: the local check of the maintainer's AAC-LC file passed again at d9cdfd2, after the HE-AAC packet check of rounds 20 to 22.  The file does not get the HE-AAC text.  Only 21 of its 13,100 packets end with bits that look like SBR data, and the check needs each packet that it reads to have them.
* 2026-11-08, M2: Symphonia 0.6.1 reads the AAC sync extension `0x2b7` only after an explicit object type 5 or 29.  So an M4A of object type 2 with SBR signalled there, or with implicit SBR and an `mp4a` entry at the output rate, would decode its core at twice the speed.  `engine` reads the AudioSpecificConfig itself and refuses these with the HE-AAC text.  Implicit SBR with no sign (raw ADTS, or an entry at the core rate) decodes as its core (TD-81).  Review round 7 of M2 also finds HE-AAC whose config shows no SBR by its first 8 packets, which end with SBR data, and closes TD-81.
* 2026-11-08, M2: Symphonia 0.6.1 refuses an M4A whose `mvhd` timescale is 0 (TD-82).  Review round 7 of M2 opens it: the view gives `mvhd` the media timescale, and TD-82 is closed.
* 2026-11-08, orchestrator: two CI runs failed when the runners downloaded Trunk, wasm-bindgen and wasm-opt from GitHub Releases (HTTP 504), and a rerun failed the same way.  The test, clippy and rustfmt jobs passed, and `check --tier push` passed here, with its Trunk line.  This is an outage outside the repo.
* 2026-11-08: the README of Symphonia on GitHub lists `opus`, `he-aac` and `he-aac-v2` features.  The published 0.6.1 has none of them.  Read the features of the pinned version, not the README.
* 2026-11-10, orchestrator: Chrome updated itself to 157 during round 17 of M2, and WebDriver refused a session with chromedriver 155.  The orchestrator installed Chrome for Testing chromedriver 157.0.8103.38 (mac-x64) outside the repo, as the README says.  Then `web-test` (30 tests) and `e2e decode` passed on `937d20e`.
* 2026-11-10, orchestrator: M3 runs on another Mac, an Apple M2 Pro (arm64) on macOS 27.2.1.  The session runs Opus 5.5 (the system prompt) at effort `high` (`modelSettings` for `claude-opus-5-5` in `~/.claude/settings.json`, over its `effortLevel` of `xhigh`).  The preflight passed at `b8b2c95`: `cargo test -p engine` (362 passed), `e2e decode --browser chrome` and `e2e save --browser chrome`.  Chrome is 157.0.8103.51.  A Chrome for Testing chromedriver 157.0.8103.39 for mac-arm64 was already on this Mac, outside the repo, so the run installed no tool.
* 2026-11-10, M3: the wasm bundle after M3, from `trunk build --release`: `app_bg.wasm` 6,415,004 B (+8,765), `decode-worker_bg.wasm` 1,381,067 B (no change), `stem-worker_bg.wasm` 1,295,428 B (+45,336).  The `m4a` crate adds 3.6% to the stem worker.  The sizes before M3 are 6,406,239 B, 1,381,067 B and 1,250,092 B, at `2193806`.
* 2026-11-10, M3: the 10-minute M4A of `e2e save` is 23.7 MB, 22% of its WAV.  On the planted slow sines, it is 1.7 times its FLAC.  An M4A save took 4.5 s, a FLAC save 13.4 s.
* 2026-11-10, M3: the published page ships no licence text for any crate (`gh-pages` has no `LICENSE*` or `THIRD-PARTY.md`).  Apache-2.0 section 4(a) asks for one.  M3 records it as TD-91.
* 2026-11-10, orchestrator: the afconvert check of M3 passed on the arm64 Mac at `3a3dd89`.  A scratch worktree outside the repo ran `e2e save --browser chrome` with a patch that wrote the 1-minute M4A save and the WAV save to disk.  Apple's `afinfo` reads the M4A as ALAC, 2 ch, 44.1 kHz, 646 packets, 2,646,000 valid frames.  `afconvert -f WAVE -d LEI16` decodes it to a `data` chunk equal to the WAV save's `data` chunk, bit for bit (SHA-256 `c7f1dcf4…`).  Only the headers differ: Apple's WAV has a `FLLR` chunk.  The run removed the files and the worktree after the check (ASSET-1).
* 2026-11-10, M3, review round 2: a reviewer's `git worktree add` failed in a pipe, and its `&&` chain went on.  Its `sed` then wrote a hold into another reviewer's worktree for a few seconds.  That reviewer's runs there cannot see a hold at the end of the encode, so its results stand.  The main tree did not change.  Detail: `plans/completed/m4a-m3.md`, Surprises.
* 2026-11-10, orchestrator: the maintainer stopped the first M4 team leader by mistake, during plan review round 2.  A stopped agent cannot continue, so a new team leader started from `ea32bbc`.  Only that review round was lost.
* 2026-11-10, M4: Chrome 157 and Safari 27.2 on macOS give `AudioEncoder` for `mp4a.40.2`, on the page and in a worker.  Both delays are 2112 frames.  WebCodecs shows the delay in neither, and `AudioDecoder` trims nothing.  So the worker measures the delay at each encode with an impulse (`plans/completed/m4a-m4.md`).
* 2026-11-10, M4: Safari encodes the first 1024 input frames as silence, and gives a whole `ES_Descriptor` as the description, which its own `AudioDecoder` refuses.  With the default `bitrateMode`, it gave 72 kbit/s for a target of 256 kbit/s.  The worker puts 2048 silent frames before the source, takes the config out of the descriptor, and asks for `constant` mode.
* 2026-11-11, M4: with `constant` mode, Chrome gave 203 kbit/s for the planted sines of `e2e save`, and 255 kbit/s for noise.  Safari gave about 259 kbit/s.  So the label "M4A (AAC 256 kbps)" names the rate that the save asks for.  The MP3 save is constant 192 kbit/s, so its label is its real rate.
* 2026-11-11, M4: Apple's `afinfo` reads the 1-minute AAC save as 2,646,000 valid frames and 4160 priming, the save's skip.  `afconvert` decodes it to 2,646,000 frames, at lag 0.
* 2026-11-11, M4: the stem worker grows by 36,352 bytes (2.8%), to 1,332,274 bytes, at `600b0aa`.
* 2026-11-11, M4: a fake encoder that raises its error with `setTimeout(0)` races the worker's `MessageChannel` yield.  Chrome runs the timer first, and Safari runs the message first.  So a test of an error before a wait passed in Safari with the fix removed (finding 3-1).  The fake now raises that error in a microtask.
* 2026-11-11, M4: in a Safari 27.2 `web-test`, two GPU tests of the worker fail on `GPUAdapterInfo.subgroupMinSize`, also at `69e63db` (TD-93).  With `WASM_BINDGEN_NO_DEBUG=1`, both pass, after the f16 test also takes WebKit's WGSL text.
* 2026-11-11, M4, review round 3: the reviewer and the wire-reviewer both named their worktree `target/review/3-reviewer-1`.  `review check 3` printed "no change", so no result is wrong.  From round 4, each brief names a unique worktree path.
* 2026-11-11, intent review of M4: Chromium's Windows AAC encoder (`media/gpu/windows/mf_audio_encoder.cc` on `main`) takes only 96, 128, 160 and 192 kbit/s.  It refuses any other rate with `kEncoderUnsupportedConfig`.  So Chrome and Edge on Windows probably fail the page's probe at 256 kbit/s, and their Save menu shows no AAC item.  Nobody checked this on Windows.  No user gets a wrong file or a wrong label (TD-94).

## Decision log

* 2026-11-08, agent: M4A is the target format, before Opus, Vorbis and AIFF.  Reason: Apple's tools read AAC and ALAC but not FLAC, and Symphonia 0.6.1 has no Opus decoder.  AIFF is D5.
* 2026-11-08, agent: the answers of the maintainer go in "Decisions for the maintainer".
* 2026-11-08, maintainer: D1 (a), D2 (c), D3 (a), D4 (a), D5 (a), D6 (a), D7 (a).  See "Decisions for the maintainer".
* 2026-11-08, agent: the branch is `feature/m4a`, which the maintainer started, and not `claude/next-file-format-support-u3ib64`.  Reason: one branch and one PR for each plan.
* 2026-11-08, maintainer: decision 0028 is accepted, because the answers to D1, D2 and D6 choose its content.
* 2026-11-08, maintainer: the run does not block on the maintainer.  The plan's stop conditions become decisions of the orchestrator.
* 2026-11-08, orchestrator (intent review of M1, round 1): the `esds` entry and the `iTunSMPB` tag move into M2, in the `m4a` crate, since M2's AAC and `iTunSMPB` fixtures need them and D6 forbids writer copies.  M2's `mp4_length` reads a cut-off `moov`.  M3's memory test allows the packet table.  M3 does the Consequences of 0029.  M4 plans for an unknown packet count.
* 2026-11-08, orchestrator (intent review of M1, round 2): a `moov` after the head gives `UnstatedPcm`; the head check gives the video message; M3 keeps the packet table to its allowance; M4 changes `Layout` test-first.
* 2026-11-08, orchestrator (intent review of M1, round 3, the last by decision 0024): any MP4 without the `soun` `mdhd` in the head, and raw ADTS, give `UnstatedPcm`; the video refusal names its `HeadLength` variant and both callers.
* 2026-11-08, agent (M1): `m4a` is Apache-2.0 only, by decision 0029, `proposed`.  The rest of the workspace stays MIT or Apache-2.0.
* 2026-11-08, agent (M1): CI and tier `push` run `cargo test -p engine` alone, a new CI step and a new line of `xtask check`.
* 2026-11-08, agent: an audio-only `.mp4` opens (D4).  Reason: the maintainer's real file is one, and the D4 answer excludes only video.
* 2026-11-08, agent: M1 checks the port of D2 against `alac-encoder` 0.3.0, pinned exactly, as a dev-dependency of `m4a` only.  Each test signal must give the same packets from both.  Reason: a round trip alone also passes an encoder that writes only escape (uncompressed) packets.  A test also checks that a music-like signal gives fewer bytes than its WAV.
* 2026-11-08, orchestrator: after round 1 of M2 (finding 1-9), the orchestrate skill says that a `CLAUDE.md` change that a step needs lands in that step's commit.  Reason: Steps 1 to 8 of M2 and the commit after them broke DOC-1, 9 commits in all.  It is a harness change, so the PR lists it for the maintainer.
* 2026-11-09, maintainer (during round 2 of M2): do not refuse a file because of its cover art or tags.  Hide them from Symphonia for every format that the app opens, and close TD-84 in the same change.
* 2026-11-09, maintainer: if a format has no safe way to hide its tags, refuse only a file whose tags or art are large enough to put memory at risk (MEM-4).  Open each other file as usual.
* 2026-11-09, orchestrator: the round 2 log counts 4 messages of the maintainer as interventions.  The two lines above record three of them: the second line holds both the refusal and its size limit.  The first message, to hide the M4A art, is in the Decision log of `plans/completed/m4a-m2.md`.
* 2026-11-09, orchestrator: after round 2 of M2 (finding 2-5), the orchestrate skill gives one flow for a step's change of `CLAUDE.md` or `.claude/rules/**`: a step checkpoint, then the team's step commit holds the change.
* 2026-11-09, orchestrator: after round 3 of M2 (finding 3-1), a file is not refused for its count of tag containers alone.  The refusal holds only when the tags that Symphonia keeps pass a limit from the memory budget, counted as bytes plus the overhead of each tag.  Reason: the maintainer's rule of 2026-11-09 refuses only a file whose tags put memory at risk.
* 2026-11-09, orchestrator: after round 5 of M2, a file that opened on `main` behind junk bytes opens again.  The engine scans the same window as Symphonia's probe for each known start, then opens that reader there with the hiding.  Reason: the maintainer's rule refuses a file only when it puts memory at risk, and junk bytes do not.
* 2026-11-09, maintainer: "it did not open on `main`" is no reason for a gap in a format that this plan adds, since `main` did not open M4A.  Measure each gap against the plan's promise.
* 2026-11-09, orchestrator (audit that the maintainer asked for): TD-81, TD-82, TD-85 and the `ftyp` refusal over 4 KiB rested on "a Symphonia limit", "rare" or "no patch of Symphonia".  A view in `engine` is not a patch.  Round 7 of M2 closes each, and the review criteria from round 7 judge each TD, `pre-existing` verdict and refusal in a new format against the plan's promise.
* 2026-11-09, orchestrator (round 8 of M2): if Symphonia can decode a type that the engine refuses, open it.  Reason: a refusal is for memory only, by the maintainer's rule.  This added work past the M2 list.  `ima4` AIFC opens (round 8).  IMA and Microsoft ADPCM in a WAV open (round 9), a change to WAV, which `main` opens.  ADPCM of 3 to 128 channels opens (round 10).  Detail: `plans/completed/m4a-m2.md`.
* 2026-11-09, orchestrator (round 10 of M2): a file of more than 128 channels (`MOST_CHANNELS`) is refused with a clear text.  Reason: MEM-4.  An `ima4` file of about 100 bytes that stated 65,535 channels held 2.17 GB at open.
* 2026-11-09, orchestrator (rounds 12 to 16 of M2): a source that cannot seek gets its own readers.  `tag_stream` and `mp4_stream` hide tags as the bytes pass.  A stream steps over leading ID3v2 tags.  `held` holds a `moov`-last M4A up to 1 GiB.  Only a native FIFO through `decode_path` reaches them.  Reason: the orchestrator judged these gaps against the plan's promise.  The maintainer later limited that rule (see the next line).
* 2026-11-10, orchestrator: from round 17 of M2, a round adds no new behaviour.  It fixes only what it finds.  A gap only on a source that cannot seek goes to a TD with a clear text.  Fix it only if it puts memory at risk or makes a panic.  Reason: the maintainer asked how many late findings were real blockers.  Rounds 12 to 16 closed gaps that only `decode_path` of a FIFO reaches, and each closure brought new findings.  The app's sources all seek.  Round 16's hold of a `moov`-last stream (`547bf74`) stays, since it is done and tested.
* 2026-11-10, maintainer: the hold of a `moov`-last stream (`547bf74`) is fine, since the browser cannot reach it.  It holds up to 1 GiB, only on a native FIFO.
* 2026-11-10, orchestrator: the rules of 2026-11-09 (measure a gap against the plan) and of round 17 of M2 (a late round adds no new behaviour) apply to M3 and M4.  The first applies from round 1, the second from round 4.  A gap that no user of the app can reach goes to a TD with a clear text, unless it puts memory at risk or makes a panic.
* 2026-11-10, maintainer: M3 starts on another computer, from `08b7b5b`.  No M3 work exists before that.  Start it with the kick-off prompt.
* 2026-11-10, agent: the orchestrator sends no start message for M3.  Reason: the maintainer asked for orchestrated mode by the skill's name, D1 to D7 are answered, and no new choice is open.  The review base of M3 is `b8b2c95`, the HEAD after `git pull`.  It holds only plan text after `08b7b5b`.
* 2026-11-10, maintainer (the M3 kick-off): the review criteria of each M3 and M4 round hold the rules of 2026-11-09 and 2026-11-10.  Never leave user-facing text or behaviour wrong to avoid a snapshot or fixture update; update it by the README process, and list it in the PR.  In late rounds, do not push the team to close gaps on paths that the app cannot reach.
* 2026-11-10, agent (M3 team leader): the gap of the licence text is TD-91, for its own PR, and M3 does not fix it.  Reason: it holds for `rusty_mp3` and `flacenc` before M3, and the M3 section asks only for the licence list.  The PR lists it under "Needs your attention".
* 2026-11-10, agent (M3 team leader): the Save menu is WAV, FLAC, M4A, MP3.  The `e2e save` step of CI has 50 minutes, and the `browser-tests` job 75, since the two M4A saves add their waits.  Detail: `plans/completed/m4a-m3.md`, Decision log.
* 2026-11-10, orchestrator (intent review of M3, round 1): the crate `m4a` gives the head once, at the end, and `head_len()` before the encode.  The worker's `EncodeOut::reserve` makes the zeros in JS: in blocks of at most 64 KiB on the storage path, and as one JS part on the memory path.  So no head of zeros is in WebAssembly memory.  This changes the wording of D6 and of decision 0028 ("gives the head twice"), not its intent.  Decision 0028 stays as accepted.
* 2026-11-10, orchestrator (intent review of M3, round 1): the M4 section keys `e2e save` by the worker's kind and gives the AAC error its own label, since an AAC save also has the extension `m4a`.
* 2026-11-10, orchestrator (intent review of M3, round 2): decision 0028 gets an amendment for the sink of M3 (`c738fdd`).  Review round 4 ran on it and found 6 nits, reported and not fixed.  So `plans/completed/m4a-m3.md` keeps the numbers of 3 rounds, and the M3 Progress line holds those of 4 rounds.
* 2026-11-10, orchestrator (intent review of M3, round 3, the last by decision 0024): 3 nits.  The M4 list names `saves_folder_check` too.  The two nits on the completed milestone plan are not fixed, as 4-4.
* 2026-11-10, agent (M4 team leader): the AAC save writes `moov` last (`Order::MoovLast`), with no change of `Layout`.  The worker measures the encoder's delay at each encode, and puts 2048 silent frames before the source for Safari.  The encoder runs in `constant` mode.  `Encoded::Failed` gets the wire field `in_encoder`, so a failed stored AAC encode is not taken as a storage failure (PROTO-1).  Detail: `plans/completed/m4a-m4.md`, Decision log.
* 2026-11-11, orchestrator (intent review of M4, round 1): the Windows gap of 256 kbit/s goes to TD-94 (`76ee090`), and M4 adds no fallback rate.  Reason: a fallback is new behaviour and a new label, and the late rules apply.  The maintainer decides it at the PR review.  TD-92 and `browser-quirks.md` get true texts in review round 6.

## Outcomes and retrospective

Not started.
