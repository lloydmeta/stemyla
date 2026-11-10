# 0028: M4A with Apple Lossless as a save format

Status: accepted (2026-11-08, by the maintainer, as D1, D2 and D6 of `plans/completed/m4a.md`)

Amended on 2026-11-08 by the agent, after round 1 of the intent review of M1 of `plans/completed/m4a.md`.  M2 adds the `esds` sample entry for AAC, not M4.  M2's decode tests need AAC files, and D6 forbids a copy of the writer in a test.  So `m4a` writes AAC from given packets, and M4 uses the same entry for its save.  The decision is unchanged.

Amended on 2026-11-10 by the agent, after round 2 of the intent review of M3 of `plans/completed/m4a.md`.  M3 replaced `FlacOut::replace_header` in `worker/src/web/encode.rs` with one sink for FLAC and ALAC, `EncodeOut`.  `m4a` gives `head_len()` before the encode, and `head()` once at the end.  `EncodeOut::reserve` writes the zeros, and `replace_head` puts the final head in their place.  JS makes the zeros: blocks of at most 64 KiB on the storage path, and one JS part on the memory path.  So no head of zeros is in WebAssembly memory.  The decision is unchanged.

## Context

The app saves WAV, MP3 and FLAC.  Logic Pro's list of compressed formats has no FLAC.  The list names AIFF, WAV, AAC, MP3 and Apple Lossless (ALAC) ([Logic Pro 10.1 help, "Import compressed audio files"](https://help.apple.com/logicpro/mac/10.1/en.lproj/lgcp4f68cbe3.html)).  So a FLAC stem does not go directly into Logic Pro.  `alac-encoder` 0.3.0 is the ALAC encoder crate that the plan reviewed.  It has `unsafe` code (D2 of `plans/completed/m4a.md`).  Its bit writer can write past its buffer in a release build.  It casts `&[u8]` to `&[i16]` with no check of alignment.  It uses a `transmute`.

## Decision

* The app saves a stem or the mix as a 16-bit stereo ALAC M4A (D1 (a)).  An AAC save through WebCodecs is a later milestone (M4).
* The writer is the crate `m4a`, in the Pure layer, with no dependencies (D6 (a)).  It makes bytes only, and does no input or output.  It takes interleaved 16-bit samples.  It gives the head twice: zeros to reserve its size, then the final head, in the order of `FlacOut::replace_header`.  Every test that needs an M4A uses it (TD-52).
* The encoder is a port of the encoder of `alac-encoder` 0.3.0 into `m4a`, as safe Rust with `#![forbid(unsafe_code)]`, for 16-bit stereo only (D2 (c)).  A differential test checks that the port and `alac-encoder` 0.3.0, a pinned dev-dependency, give the same packets.

## Consequences

* The D6 rules hold for M3 and later: `stem-worker` does the writes, and no test has a copy of the writer.
* `stco` holds 32-bit offsets, so the writer refuses a file that can reach 4 GiB, about 6.7 hours at 44.1 kHz.  The 1 GiB decode limit (decision 0013) keeps a real save far below it.
* The sample entry can take a second kind: M2 adds an `esds` entry for AAC, for its AAC decode fixtures.  M4 uses it for the AAC save, with an `elst` for the encoder delay.
* The licence of `m4a` is not part of this decision.  Decision 0029 gives it.
