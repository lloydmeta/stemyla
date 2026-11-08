# 0028: M4A with Apple Lossless as a save format

Status: accepted (2026-11-08, by the maintainer, as D1, D2 and D6 of `plans/active/m4a.md`)

## Context

The app saves WAV, MP3 and FLAC.  Apple's tools read AAC and Apple Lossless (ALAC) in an M4A file, but not FLAC: Logic Pro's list of compressed formats has no FLAC ([Logic Pro 10.1 help](https://help.apple.com/logicpro/mac/10.1/en.lproj/lgcp4f68cbe3.html)).  So a FLAC stem does not go directly into these tools.  The one Rust ALAC encoder on crates.io, `alac-encoder` 0.3.0, has `unsafe` code: a bit writer that a release build lets write past its buffer, casts of `&[u8]` to `&[i16]` with no check of alignment, and a `transmute` (the parent plan's D2).

## Decision

* The app saves a stem or the mix as a 16-bit stereo ALAC M4A (D1 (a)).  An AAC save through WebCodecs is a later milestone (M4).
* The writer is the crate `m4a`, in the Pure layer, with no dependencies (D6 (a)).  It makes bytes only, and does no input or output.  It takes interleaved 16-bit samples.  It gives the head twice: zeros to reserve its size, then the final head, in the order of `FlacOut::replace_header`.  Every test that needs an M4A uses it (TD-52).
* The encoder is a port of the encoder of `alac-encoder` 0.3.0 into `m4a`, as safe Rust with `#![forbid(unsafe_code)]`, for 16-bit stereo only (D2 (c)).  A differential test checks that the port and `alac-encoder` 0.3.0, a pinned dev-dependency, give the same packets.

## Consequences

* The D6 rules hold for M3 and later: `stem-worker` does the writes, and no test has a copy of the writer.
* `stco` holds 32-bit offsets, so the writer refuses a file that can reach 4 GiB, about 6.7 hours at 44.1 kHz.  The 1 GiB decode limit (decision 0013) keeps a real save far below it.
* The sample entry can take a second kind: M4 adds an `esds` entry for AAC, with an `elst` for the encoder delay.
* Apple's ALAC reference is Apache-2.0 only, so `m4a` is Apache-2.0 only, not MIT or Apache-2.0 as the rest of the workspace is.  `m4a/src/alac.rs` holds the notices of both sources.
