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
* A file whose `moov` is after `mdat` has no length in the 64 KiB head.  It is `HeadLength::Unstated`, as an MP3 without a Xing tag is.
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
* `stco` holds 32-bit offsets.  The writer refuses a file of 4 GiB or more.  A 1 GiB decode limit keeps a 16-bit save far below that.
* Write decision 0028 as `accepted`: M4A with ALAC as the save format, the location of the writer, and the safe port.  The maintainer's answers to D1, D2 and D6 accept it (Decision log).
* `THIRD-PARTY.md` names each new crate (DEPS-1).
* Acceptance: native tests that fail before the change.
  * A writer test round-trips through Symphonia (`isomp4`, `alac`) bit for bit: silence, a full-scale square wave, a mono-like and a random 16-bit signal, and lengths of 0, 1, 4095, 4096 and 4097 frames.
  * A test checks that `moov` comes before `mdat`, and that each `stco` offset points at its packet.
  * A test checks the refusal at 4 GiB, without a 4 GiB buffer.

### M2: open M4A (and AIFF if D5 says so)

* Turn on the Symphonia features `isomp4`, `alac` and `aac` in `engine` (and `aiff` by D5).
* `most_frames`: an ALAC case and an AAC case, with the cap of decision 0013 on the stated length.
* `head_length`: an `mp4_length` case that reads `mdhd` (and `elst`) when `moov` is in the head.  Otherwise it gives `HeadLength::Unstated`.
* The encoder delay and the padding: `engine::decode` reads `elst`, then `iTunSMPB` (by D7), through the same seekable source, and trims both.  So the decoded length equals the source's length.
* Messages: a DRM file gives "protected, cannot open".  An HE-AAC file gives "HE-AAC is not supported; use AAC-LC".  A file with more than 2 AAC channels gives its own message.
* `SONG_FILES`, the `WELCOME` texts, the hover text of "Open song…", `Song::load_folder`, `xtask serve` (`audio/mp4`, `audio/aac`, and `audio/aiff` by D5).
* The fixtures come from the M1 writer (ALAC) and from D3 (AAC).  No audio goes into git (ASSET-1).
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
* Acceptance:
  * `worker/src/web/tests.rs`: an ALAC encode in blocks decodes to the same 16 bits as `engine::wav::encode_pcm16`, on both paths, and its peak memory does not grow with the length.
  * `cargo run -p xtask -- e2e save --browser chrome`: the M4A saves of 1 and 10 minutes meet the same limits as FLAC.  These are the JS heap, the WebAssembly memory, the longest slice and the worker's memory.  Each saved file decodes to the WAV save's samples, bit for bit.
  * The extension test in `app/src/export.rs` lists `m4a`.

### M4: save as AAC-LC (by D1)

Only if D1 takes the AAC save.  The stem worker encodes with WebCodecs `AudioEncoder` (`mp4a.40.2`, 256 kbps), and the M1 writer makes the M4A with an `esds` sample entry and an `elst` for the encoder delay.

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
* [ ] M1: the M4A writer and the ALAC encoder.
* [ ] M2: open M4A (and AIFF by D5).
* [ ] M3: save as ALAC M4A.
* [ ] M4: save as AAC-LC (by D1).

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
* 2026-11-08: the README of Symphonia on GitHub lists `opus`, `he-aac` and `he-aac-v2` features.  The published 0.6.1 has none of them.  Read the features of the pinned version, not the README.

## Decision log

* 2026-11-08, agent: M4A is the target format, before Opus, Vorbis and AIFF.  Reason: Apple's tools read AAC and ALAC but not FLAC, and Symphonia 0.6.1 has no Opus decoder.  AIFF is D5.
* 2026-11-08, agent: the answers of the maintainer go in "Decisions for the maintainer".
* 2026-11-08, maintainer: D1 (a), D2 (c), D3 (a), D4 (a), D5 (a), D6 (a), D7 (a).  See "Decisions for the maintainer".
* 2026-11-08, agent: the branch is `feature/m4a`, which the maintainer started, and not `claude/next-file-format-support-u3ib64`.  Reason: one branch and one PR for each plan.
* 2026-11-08, maintainer: decision 0028 is accepted, because the answers to D1, D2 and D6 choose its content.
* 2026-11-08, maintainer: the run does not block on the maintainer.  The plan's stop conditions become decisions of the orchestrator.
* 2026-11-08, agent: an audio-only `.mp4` opens (D4).  Reason: the maintainer's real file is one, and the D4 answer excludes only video.
* 2026-11-08, agent: M1 checks the port of D2 against `alac-encoder` 0.3.0, pinned exactly, as a dev-dependency of `m4a` only.  Each test signal must give the same packets from both.  Reason: a round trip alone also passes an encoder that writes only escape (uncompressed) packets.  A test also checks that a music-like signal gives fewer bytes than its WAV.

## Outcomes and retrospective

Not started.
