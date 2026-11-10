# 0029: the crate m4a is Apache-2.0 only

Status: accepted (2026-11-11, by the maintainer, at the review of PR #43; proposed on 2026-11-08 by the agent)

## Context

The workspace is MIT or Apache-2.0, at the user's option.  Decision 0028 puts an ALAC encoder in the crate `m4a`, as a safe port of `alac-encoder` 0.3.0.  `alac-encoder` is MIT OR Apache-2.0 ([crates.io](https://crates.io/crates/alac-encoder)).  It is itself a port of Apple's ALAC reference ([macosforge/alac](https://github.com/macosforge/alac)).  The header of Apple's `codec/ALACEncoder.cpp` gives Apache-2.0 only.  Apple's repository has no `NOTICE` file.  The Apache-2.0 terms of Apple's code hold in a port of it, so the port cannot be offered under MIT alone.  The D1, D2 and D6 answers of `plans/completed/m4a.md` choose no licence.

## Options

* Apache-2.0 only for `m4a`: `m4a/Cargo.toml` has `license = "Apache-2.0"`.  `alac-encoder` is taken under its Apache-2.0 option.  The other crates stay MIT or Apache-2.0.
* MIT or Apache-2.0 for `m4a`, as the workspace is.  This needs an encoder that is not a port of Apple's code.  An example is a new encoder from the ALAC format only.  D2 chose the port, so this option means a new milestone and a new differential reference.

## Decision

`m4a` is Apache-2.0 only.  `alac-encoder` is taken under its Apache-2.0 option.  The other crates stay MIT or Apache-2.0.

## Consequences

* The licence section of `README.md` says that `m4a/` is Apache-2.0 only.  `THIRD-PARTY.md` names the two sources of the port.
* `m4a/src/alac.rs` holds the notice of each source.  It says that the file was changed from them, as Apache-2.0 section 4(b) asks.
* Since M3, `stem-worker` takes `m4a`, so the web app ships Apache-2.0-only code.  `THIRD-PARTY.md` and the licence section of `README.md` give `m4a` as Apache-2.0 only.
* A person who takes the workspace under MIT cannot take `m4a` under MIT.
