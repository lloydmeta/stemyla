# M4A, M1: the M4A writer and the ALAC encoder

## Purpose

This is the milestone plan for M1 of `plans/active/m4a.md`.  The parent's M1 section stays the criteria.  M1 adds a new crate, `m4a`, in the Pure layer.  The crate makes the bytes of an M4A file that holds 16-bit stereo Apple Lossless (ALAC).  It does no input or output, and no other crate uses it yet.  M2 uses it for the decode fixtures, and M3 for the save.

Readers: the implementer, the review agents of M1's rounds, the orchestrator, and the team leaders of M2 to M4.

## Context and orientation

The state at the start, on 2026-11-08, at commit `b18c494` on branch `feature/m4a`.  The review base of M1 is `b1f588b`.  The range from it also holds the orchestrator's plan commit `b18c494`.

### The decisions that M1 follows

All are in the parent's "Decisions for the maintainer" and Decision log.

* D2 (c): port the encoder of `alac-encoder` 0.3.0 into `m4a` as safe Rust, with `#![forbid(unsafe_code)]`, for 16-bit stereo only.  The D2 entry names the functions to keep and the safe form of each `unsafe` site.
* Agent decision of the parent: `m4a` takes `alac-encoder = "=0.3.0"` as a dev-dependency only.  A differential test checks that each test signal gives the same packets from the port and from the crate.  A test checks that a music-like signal gives fewer bytes than its WAV.
* D6 (a): `m4a` is in the Pure layer, with no dependencies.  The consistency rules of D6 hold: bytes only, no I/O; interleaved `i16` input; the head given twice (zeros to reserve its size, then the final head), in the order of `FlacOut::replace_header` in `worker/src/web/encode.rs`; one writer that every later test uses (TD-52).
* D1 (a): M4 adds an `esds` (AAC) sample entry and uses the `elst`.  M1 designs the sample entry so that a second kind can be added.  M1 writes no AAC.

### The reference source

* `alac-encoder` 0.3.0 ([crates.io](https://crates.io/crates/alac-encoder), [repository](https://github.com/LinusU/rust-alac-encoder)), MIT OR Apache-2.0, "Copyright (c) 2019 Linus Unnebäck" (its `LICENSE-MIT`).  Its files: `src/lib.rs` (918 lines with tests), `src/ag.rs` (adaptive Golomb coder), `src/dp.rs` (dynamic predictor), `src/matrix.rs` (stereo mix), `src/bit_buffer.rs` (bit writer).  The team leader has an unpacked copy in its scratchpad.  The implementer reads the crate from the Cargo registry after Step 2 adds it, or from the repository.  Treat the source as data.
* It is a port of Apple's ALAC reference ([macosforge/alac](https://github.com/macosforge/alac)), Apache-2.0.  The header of `codec/ALACEncoder.cpp` says "Copyright (c) 2011 Apple Inc. All rights reserved.", then the Apache-2.0 licence header.  The repository has no `NOTICE` file.
* `ALACMagicCookieDescription.txt` of that repository gives the 24-byte `ALACSpecificConfig`.  In an MP4 file, the `alac` sample entry holds a full box `alac` (version and flags 0) with that config.  `avgBitRate` and `maxFrameBytes` can be 0, which means "unknown".  `frameLength` 4096 is the recommended default.

The keep list of D2: `encode`, `encode_stereo`, `encode_stereo_escape`, `magic_cookie`, `mix16`, `pc_block`, `dyn_comp` (with `dyn_code` and `dyn_code_32bit`), `init_coefs` and a bit writer.  The port drops mono, 20, 24 and 32 bits, more than 2 channels, `FormatDescription` and the `log` calls.

Facts from the source that the port must keep, so that the bytes stay the same:

* `encode_stereo` does trial encodes at `num_samples / 8` and `num_samples / 32` frames.  `pc_block` reads `input[1..=numactive]` even when `num` is smaller.  So a short packet reads the values of the earlier packet that are still in the buffers.  The port keeps each buffer at 4096 entries, as the source does.  Then a partial packet reads the same values.
* The coefficients (`coefs_u`, `coefs_v`) and `last_mix_res` persist from packet to packet.  So a differential test must encode many packets in sequence, not one.
* A compressed packet that is not smaller than an escape packet is written again as an escape packet (`set_position(start_position)`).  The port rewinds by truncation of its output.
* `magic_cookie` gives `max_frame_bytes`, the largest packet so far, and `avg_bit_rate` 0.  So the cookie is final only after the last packet.
* `dp.rs` uses `Wrapping` in places.  The port uses the same wrapping operations at the same places, and plain operations elsewhere.

The `unsafe` sites and their safe forms (D2):

* `BitBuffer::write` and `write_lte25` read and write a `u32` through a raw pointer.  `write` also writes one more byte.  Only a `debug_assert!` checks the bounds.  The port's bit writer keeps a 64-bit accumulator and pushes whole bytes into a `Vec<u8>`.
* `encode_mono`, `encode_stereo_escape` and others cast `&[u8]` to `&[i16]` with `slice::from_raw_parts`.  The port takes `&[i16]`, so no cast is necessary.  `mix16` reads `i16` values directly.
* `Signed::sign` in `dp.rs` uses `transmute` on `i32::signum`.  The port uses a `match`.

### The files that M1 touches

* `Cargo.toml`: the workspace `members`.
* New: `m4a/Cargo.toml`, `m4a/clippy.toml`, `m4a/src/lib.rs` and the modules of Step 2 and Step 3, and `m4a/tests/`.
* `Cargo.lock`: `alac-encoder` 0.3.0, and Symphonia's `isomp4` and `alac` crates, all for the tests of `m4a` only.  `log` is in the lock file already.
* `docs/ARCHITECTURE.md`: a line in the list of crates, and the row ``| Pure | `m4a` | no workspace crate |`` in the layers table, with the other Pure rows.  The parser of the table needs the crate name as a code span.
* `README.md`: a row in the layout table, and the licence section (see "The licence of the port").
* `CLAUDE.md`: a line in "Layout".  `.claude/rules/gpu.md`, `.claude/rules/memory.md` and `.claude/rules/protocol.md`: `m4a` in the list of crates whose clippy fails `slice::to_vec`.  Only the orchestrator edits these files.  The team leader gives them as diffs in the orchestrator's scratchpad folder `m1/`: `claude-md.diff` and `claude-rules.diff`.
* `docs/invariants.md` (MEM-1, "Enforced by") and `docs/tech-debt.md` (TD-26): `m4a` in the same list of crates (DOC-1).  `m4a` goes at the end of each list, since "the first four" counts by position.
* `xtask/src/check.rs`: `fixture()` gets `("m4a", "m4a", &[])`, and its comments that count members ("The 11 members", "four members") change, since `the_fixture_graph_is_the_real_graph` compares it with `cargo metadata`.  The test `the_crate_tests_of_every_member_but_roformer_gpu_are_the_workspace_test_line` gets `m4a` in its set of members, since no crate depends on `m4a`.
* `xtask/src/review.rs`: `m4a/src/**` in the include list of `memory-reviewer`, and a case in its route test.
* `THIRD-PARTY.md`: a note on the port, its two notices and the test-only crates.
* New: `docs/decisions/0028-m4a-with-alac-as-the-save-format.md`, `accepted`.
* `docs/README.md`: the line of this plan and the line of decision 0028.

`LAYERS` in `xtask/src/rules.rs` holds the five layer names, not the crates.  The Pure layer exists, so `LAYERS` does not change.  `crates_depend_only_downwards` reads the rows from `docs/ARCHITECTURE.md`.

### Checks that a new crate meets

* `crates_depend_only_downwards` (LAYER-1): the row in the layers table.
* `each_shipped_crate_lints_mem1_copies` (MEM-1, decision 0008): `m4a/clippy.toml` fails `slice::to_vec`, and a test `mem1_lint_is_on` in `m4a` proves the path, as `engine/src/song.rs` does.
* `docs-check`: the crate is in the layout of `CLAUDE.md` and `README.md`, and in the crates of `docs/ARCHITECTURE.md`.
* `young_crates_are_pinned` (DEPS-1): `alac-encoder` is below 1.0, so it is pinned with `=`.  Symphonia stays at the version of `engine/Cargo.toml`.
* `the_fixture_graph_is_the_real_graph` (`xtask/src/check.rs`): the crate graph of `check --tier`.

### The licence of the port

Apple's ALAC reference is Apache-2.0 only.  `alac-encoder` offers MIT OR Apache-2.0, and the port takes it under Apache-2.0.  So `m4a/Cargo.toml` has `license = "Apache-2.0"`, not `license.workspace`.  The licence section of `README.md` says that `m4a/` is Apache-2.0 only.  Decision 0028 records this.

### Terms

* Frame: one left sample and one right sample.
* Packet: one ALAC packet of at most 4096 frames.  The last packet of a file can be shorter.
* Head: the bytes before the first packet: `ftyp`, `moov` and the 8-byte `mdat` header.
* Reserve: the head's size in zeros, written before the first packet.
* Cookie: the 24-byte `ALACSpecificConfig`.
* Differential test: a test that gives the same input to the port and to `alac-encoder` 0.3.0, and compares their bytes.

## Plan of work

Each step is one signed commit, after `cargo run -p xtask -- check --tier commit` (GIT-2).  Each new test fails first on an assert, then passes.  A stub that returns a wrong value, such as an empty `Vec`, makes the first run fail on an assert, not on `todo!()`.  Record each first failure in the Progress line of its step.

### Step 1: the crate and its place in the layers

* Add `m4a` to the workspace `members`.  `m4a/Cargo.toml`: `edition.workspace`, `license = "Apache-2.0"` ("The licence of the port"), `[lints] workspace = true`, no `[dependencies]`.
* `m4a/src/lib.rs`: `#![forbid(unsafe_code)]` and the crate doc.
* `m4a/clippy.toml` as `engine/clippy.toml`, and the canary test `mem1_lint_is_on`.
* The row in the layers table, the crate line in `docs/ARCHITECTURE.md`, the row in the layout of `README.md`, and the licence section of `README.md`.
* `m4a/src/**` in the include list of `memory-reviewer` in `xtask/src/review.rs`, with a case such as `("m4a/src/lib.rs", "memory-reviewer", "m4a/src/**")` in its route test.  Reason: M3's save runs this code on song-length input in the worker (MEM-1, MEM-4).  `dsp/src/**` and `synth/src/**` are in that list for the same reason.
* `m4a` in `fixture()` and in the member set of the workspace test line test, in `xtask/src/check.rs`.
* `m4a` in the crate lists of `docs/invariants.md` and `docs/tech-debt.md` (TD-26).
* The orchestrator commits `m1/claude-md.diff` and `m1/claude-rules.diff` before Step 1 starts.  `docs-check` passes with the `CLAUDE.md` line before `m4a/` exists: a one-part path is not checked, and `crates_documented` reads the members only.  The team leader checked this on 2026-11-08.  The implementer does not edit `CLAUDE.md` or `.claude/**`.
* `every_reviewer_has_an_agent_file` reads `git ls-files`, so `m4a/src/**` matches nothing until `m4a/src/lib.rs` is tracked.  So the implementer stages each new file by name with `git add <path>` before it runs `check --tier commit`.  It never runs `git add -A`.
* Acceptance: `cargo test -p xtask --test rules each_shipped_crate_lints_mem1_copies` fails before `m4a/clippy.toml` exists, and passes after.  Clippy on `m4a --all-targets` proves the canary.  The route test fails before the change to `REVIEWERS`.  `cargo test -p xtask --lib` and `cargo test -p xtask --test rules` pass.

### Step 2: the bit writer and the ALAC encoder

* `m4a/src/bits.rs`: a bit writer with a 64-bit accumulator, as D2 says.  It has `write(value, bits)` for 1 to 32 bits, `byte_align`, `bit_len` and `truncate(bit_len)`.  Each write keeps only the low `bits` of the value, as `write_lte25` does.
  * Check `BitBuffer::write` for values with bits above `num_bits`.  With a negative `shift`, it does not mask them.  If a caller of a kept function can pass such a value, the port gives the same bytes as the source, and a unit test shows the case.  If no caller can, record that in Surprises.
* `m4a/src/alac.rs` (or a folder with `ag.rs`, `dp.rs` and `matrix.rs`): the port.  The public API, as a sketch:

  ```rust
  pub const FRAMES_PER_PACKET: usize = 4096;
  /// The largest packet of 16-bit stereo: an escape packet of 4096 frames.
  pub const MAX_PACKET_BYTES: usize = 16_388;

  pub struct AlacEncoder { /* buffers of FRAMES_PER_PACKET, coefficients, last_mix_res, max_packet */ }

  impl AlacEncoder {
      pub fn new(sample_rate: u32) -> Self;
      /// Appends one packet to `out`.  `interleaved` holds 1 to 4096 frames.
      pub fn encode(&mut self, interleaved: &[i16], out: &mut Vec<u8>) -> usize;
      /// The 24-byte ALACSpecificConfig, final after the last packet.
      pub fn cookie(&self) -> [u8; 24];
  }
  ```

  * `encode` panics on an odd length, on 0 frames, or on more than 4096 frames.  A `#[should_panic]` test checks each panic.  Its first failure is "did not panic", not an assert, and that is its fail-first evidence.  The caller (the writer of Step 4, and M3) never gives such a slice.
  * The module doc holds the two notices, with the licence of the port ("The licence of the port"): the MIT notice of `alac-encoder` (copyright line and permission text), and Apple's copyright line with the Apache-2.0 header.  It also says that the file is changed from those sources, as Apache-2.0 section 4(b) asks.
  * The unrolled loops of `pc_block` for 4 and 8 coefficients become one loop, if the differential test gives the same bytes (D2).  The one loop keeps the `Wrapping` sum of those branches (`dp.rs`), not the plain sum of the general branch.  Else keep them, and record why in Surprises.
  * With 16-bit stereo and `mode` 0, `pc_block` gets only 4 or 8 active coefficients.  So the port drops its branches for 0, 31 and the general case, which are dead code here.  The `mode` 1 calls in `encode_stereo` are dead too, since `mode` is always 0.
* `m4a/Cargo.toml`: `[dev-dependencies] alac-encoder = "=0.3.0"`, with a comment that says why.
* Tests, in `m4a/tests/alac.rs` or in the module:
  * The signals live in `m4a/tests/common/mod.rs`, since several test files use them.
  * The differential test.  The signals are silence, a full-scale square wave (32,767 and -32,768), a mono-like signal (left equals right, a sine), a random 16-bit signal from a fixed-seed generator in the test (no `rand` crate), and a music-like signal (Step 5).  The lengths are 1, 4095, 4096, 4097 and 10 x 4096 + 1,000 frames.  Each signal and length goes through one encoder of each kind, packet by packet.  The test compares each packet, and the cookie after the last packet.  The test gives the crate `i16::to_le_bytes`, since `mix16` reads little-endian bytes.
  * The reference crate writes the whole compressed try into its output before its size check and rewind (`encode_stereo`).  A try can pass `MAX_PACKET_BYTES`, and the bit writer also writes up to 4 bytes past the bit position (`bit_buffer.rs`).  So the test gives it the source's own work-buffer size plus 8: 4096 x 2 x 5 + 1 + 8 bytes.  The port's `out` can also grow past `MAX_PACKET_BYTES` before `truncate`.  Its escape path casts the input to `&[i16]` (`from_raw_parts`), so the test asserts that the input's address is even before each call.
  * A rewind signal: in each packet, the first eighth is quiet and the rest is random.  The trial encode then predicts a small packet, and the full encode is rewound to an escape packet.  The differential test includes it.
  * A random signal makes escape packets, and a sine makes compressed packets.  The test asserts that both kinds occur.  The escape flag is bit 22 from the packet start: after the 3-bit tag, the 4-bit instance tag, 12 zero bits, the partial bit and 2 shift bits.
  * Each packet is at most `MAX_PACKET_BYTES`.
* A unit test of the bit writer: `truncate` to a bit length in the middle of a byte, then a write, gives the same bytes as a writer that never had the truncated bits.
* Acceptance: `cargo test -p m4a` passes, with each test failing first.  `grep -rnE 'unsafe (\{|fn|impl)' m4a/src` finds nothing.

### Step 3: the box writer

* `m4a/src/mp4.rs` (or `writer.rs`): the boxes of the parent's M1 section.  The public API, as a sketch:

  ```rust
  pub enum SampleEntry { Alac { cookie: [u8; 24] } }   // M4 adds Esds
  pub struct Edit { pub skip: u32, pub frames: u64 }   // one elst entry
  pub enum Order { FastStart, MoovLast }               // MoovLast for M2's fixture

  pub struct Layout { /* frames, sample_rate, edit, order, packets */ }

  impl Layout {
      /// Refuses a rate of 0 or above 65,535, and a file that can reach 4 GiB.
      pub fn new(frames: u64, sample_rate: u32, edit: Option<Edit>, order: Order) -> Result<Layout, Error>;
      pub fn packets(&self) -> usize;
      /// The size of the head: ftyp, moov and the mdat header (FastStart).
      pub fn head_len(&self) -> usize;
      /// The final head.  `packet_sizes` holds one size for each packet, in order.
      pub fn head(&self, entry: &SampleEntry, packet_sizes: &[u32]) -> Result<Vec<u8>, Error>;
      /// MoovLast only: the moov that follows the last packet.
      pub fn tail(&self, entry: &SampleEntry, packet_sizes: &[u32]) -> Result<Vec<u8>, Error>;
  }
  ```

  * The caller writes `head_len` zeros, then each packet, then the final head at position 0.  This is the order of `FlacOut::replace_header`.  `head().len()` always equals `head_len()`.
  * `Order::MoovLast` gives `ftyp` and the `mdat` header as the head, and `moov` as the tail.  M2 needs such a file (the parent's M2 acceptance), and D6 forbids a copy of the writer.  M3 uses `FastStart` only.
  * The box order of `FastStart`: `ftyp` (major brand `M4A `, compatible `M4A `, `mp42`, `isom`), `moov`, `mdat`.  `moov` holds `mvhd`, `trak`.  `trak` holds `tkhd`, `edts` with `elst` only with an `Edit`, and `mdia`.  `mdia` holds `mdhd`, `hdlr` (`soun`), `minf`.  `minf` holds `smhd`, `dinf` (`dref` with one `url ` of flags 1), `stbl`.  `stbl` holds `stsd`, `stts`, `stsc`, `stsz`, `stco`.
  * The timescale of `mvhd` and `mdhd` is the sample rate.  The `mdhd` duration is `frames`.  The `mvhd` and `tkhd` durations are `edit.frames` with an `Edit`, else `frames`.  Version 0 boxes, so each duration is 32 bits; `Layout::new` makes sure that each fits.
  * The fields that Symphonia does not check but Apple's decoder can (the orchestrator runs `afconvert` after M3): `mvhd` rate 1.0, volume 1.0, the identity matrix and `next_track_ID` 2; `tkhd` flags 7, `track_ID` 1, volume 1.0 and the identity matrix; `mdhd` language `und`; `stsd` entry count 1; the sample entry's `data_reference_index` 1.
  * The `alac` sample entry: an `AudioSampleEntry` with 2 channels, sample size 16 and the rate as 16.16 fixed point.  Then the full box `alac`, 36 bytes: size, type, version and flags 0, and the cookie.
  * Each packet is one chunk.  `stsc` has one entry (1, 1, 1).  `stco` has one 32-bit offset for each packet, so each offset points at its packet.  `stts` has at most 2 entries: the full packets of 4096 frames, and a last, shorter packet.
  * `elst`: version 0, one entry with `segment_duration = edit.frames`, `media_time = edit.skip` and rate 1.0.  `Layout::new` refuses `skip + frames` larger than the media frames.
  * The 4 GiB refusal: the worst case is `head_len + packets x MAX_PACKET_BYTES`, plus the size of the tail with `MoovLast`.  `Layout::new` refuses a worst case of 2^32 bytes or more.  So each `stco` offset and the `mdat` size fit in 32 bits.  At 44.1 kHz the limit is about 6.7 hours.  The 1 GiB decode limit (decision 0013) is at most 134,217,728 frames, about 50 minutes, so a real save never meets it.
  * `head` refuses a count of sizes that is not `packets()`, and a size above `MAX_PACKET_BYTES`.
  * `SampleEntry` has one variant in M1, and `Layout` knows no codec.  M4 adds the `Esds` variant, the codec in `Layout`, and the check that `head` gets an entry of the layout's codec, with its test (D1).
* Tests, with a small box reader in `m4a/tests/` (test code only):
  * The structure of a file of 3 packets: the box order and sizes, `moov` before `mdat`, each `stco` offset points at the bytes of its packet, `stsz` and `stts` match the packets, and the `alac` box holds the cookie.  It also checks each field of the list above.
  * The same for `MoovLast`: `mdat` before `moov`, and each `stco` offset points at its packet.
  * An `elst` with `skip` 2,112 holds its values, and the durations follow the rule above.  A file with no `Edit` has no `edts`.
  * The refusal at 4 GiB, for each `Order`: `Layout::new` passes for the largest `frames` whose worst case is below 2^32, and refuses the next packet count.  No buffer of that size is made.
  * The refusals of a rate of 0 and of 65,536, of an `Edit` past the end, and of `head` with a wrong count or a size above the maximum.
  * `head().len() == head_len()` for each length of the round trip of Step 4, with and without an `Edit`.
* Acceptance: `cargo test -p m4a` passes, with each test failing first.

### Step 4: the round trip through Symphonia

* A whole-file helper for tests and tools, as a sketch: `pub fn alac_file(interleaved: &[i16], sample_rate: u32, edit: Option<Edit>, order: Order) -> Result<Vec<u8>, Error>`.  It uses `Layout`, `AlacEncoder` and `head` as M3 will, and it is the one writer of every later test (TD-52).  It holds the whole file in memory, so the worker (M3) does not use it.
* `m4a/Cargo.toml`: `symphonia` as a dev-dependency, at the version of `engine/Cargo.toml` (0.6.1), with `default-features = false` and the features `isomp4` and `alac`.
* Tests, in `m4a/tests/round_trip.rs`:
  * Each signal of Step 2 but the music-like one, at the lengths 0, 1, 4095, 4096 and 4097 frames, at 44.1 kHz: Symphonia decodes the file bit for bit, as `i16` samples, and gives the same frame count, 2 channels and 44,100 Hz.
  * A `MoovLast` file of 4097 frames decodes bit for bit too.
* If Symphonia 0.6.1 refuses a valid file, such as the file of 0 frames, do not patch Symphonia.  Report it to the team leader with the evidence, and go on with the other steps.  The orchestrator decides the case (the parent's "Extra stop conditions": narrow the case, a clear message, a TD entry).  The structure test of Step 3 then holds for that length.
* Feature unification: `cargo test --workspace` builds one Symphonia with the features of every crate, so `engine`'s tests then decode M4A before M2 turns the features on.  Record this in Surprises for M2.  M2 shows its first failures with `cargo test -p engine`.
* Acceptance: `cargo test -p m4a` passes, with each test failing first.

### Step 5: the size check, the notices and decision 0028

* The music-like signal: a sum of 3 to 5 sines with an envelope, plus low-level noise from the fixed-seed generator, 10 seconds at 44.1 kHz.  A test checks that `alac_file` of it is smaller than its 16-bit WAV (`44 + frames x 4` bytes).  Write the ratio in Surprises.
* `THIRD-PARTY.md`: a note bullet.  It says that `m4a` holds a port of `alac-encoder` 0.3.0 (MIT OR Apache-2.0, "Copyright (c) 2019 Linus Unnebäck"), itself a port of Apple's ALAC reference (Apache-2.0, "Copyright (c) 2011 Apple Inc.").  It says that `m4a` is Apache-2.0, and names the module that holds both notices.  It names `alac-encoder`, `symphonia-format-isomp4` and `symphonia-codec-alac` as test-only crates of `m4a`.  The tables do not change in M1, since no shipped crate takes `m4a` yet.
* `docs/decisions/0028-m4a-with-alac-as-the-save-format.md`, `Status: accepted (2026-11-08, by the maintainer, as D1, D2 and D6 of plans/active/m4a.md)`.  The maintainer's answers choose its content (the parent's Decision log).  Context: Apple's tools read ALAC M4A but not FLAC (the parent's Purpose).  Decision: the save format M4A with ALAC; the writer in the `m4a` crate of the Pure layer (D6); the encoder as a safe port of `alac-encoder` 0.3.0 (D2), checked against it.  Consequences: the D6 rules, the 4 GiB limit, M4's `esds` entry, and `m4a` under Apache-2.0 only.
* `docs/README.md`: the line of decision 0028.
* Acceptance: `cargo test -p m4a` passes.  `cargo run -p xtask -- docs-check` passes.

### Before review round 1

* `cargo run -p xtask -- check --tier push` and `cargo build -p xtask` pass.
* `cargo clippy -p m4a --all-targets -- -D warnings` passes.
* `cargo test -p m4a --release` passes too, since the source checks its bounds only with `debug_assert!`.

## Validation and acceptance

* `cargo test -p m4a` passes: the differential test, the round trip bit for bit at each length, the structure test, the `elst` test, the refusal at 4 GiB, and the size check.
* `cargo test --workspace --exclude roformer-gpu` and `cargo run -p xtask -- docs-check` pass.
* `cargo run -p xtask -- check --tier push` passes.
* `m4a/src` has no `unsafe` block, and `#![forbid(unsafe_code)]` holds.

## Idempotence and recovery

Each step is one commit, and each commit passes `check --tier commit`.  No other crate uses `m4a` in M1.  So `git revert` of the M1 commits removes it with no effect on the app.  The dev-dependencies change only the test build of `m4a`.  A new implementer starts from this plan and its Progress.

## Progress

* [x] 2026-11-08: plan written by the team leader.
* [ ] Step 1: the crate and its place in the layers.
* [ ] Step 2: the bit writer and the ALAC encoder.
* [ ] Step 3: the box writer.
* [ ] Step 4: the round trip through Symphonia.
* [ ] Step 5: the size check, the notices and decision 0028.
* [ ] Review rounds.

## Surprises and discoveries

None yet.

## Decision log

* 2026-11-08, agent (team leader): `LAYERS` in `xtask/src/rules.rs` does not change.  Reason: it holds the five layer names, and `m4a` goes in the Pure layer, which exists.
* 2026-11-08, agent (team leader): `memory-reviewer` routes `m4a/src/**`.  Reason: M3 runs this code on song-length input in the worker, and the Pure crates `dsp` and `synth` are routed so.  Cost: the change to `xtask/src/review.rs` routes all six reviewers in round 1 (`ROUTE_ALL`).
* 2026-11-08, agent (team leader): `Layout::new` refuses on the worst case of `MAX_PACKET_BYTES` for each packet, not on the real sizes.  Reason: the refusal comes before the first packet, so no file is written in vain.  The worst case is about 6.7 hours at 44.1 kHz, far above the 1 GiB decode limit.
* 2026-11-08, agent (team leader): each packet is one chunk, so `stco` has one offset for each packet.  Reason: the parent's acceptance asks that each `stco` offset point at its packet.  The cost is 4 bytes for each packet, about 151 KiB for 1 hour at 44.1 kHz.
* 2026-11-08, agent (team leader): `avgBitRate` in the cookie stays 0, as the source gives it.  Reason: the differential test compares the cookie, and Apple's description allows 0 as "unknown".
* 2026-11-08, agent (team leader): the writer refuses a sample rate above 65,535.  Reason: the sample entry holds the rate as 16.16 fixed point, and every song in the app is at 44.1 kHz (decision 0012).
* 2026-11-08, agent (team leader): the plan review (1 round, a `Plan` agent) found 1 blocker, 10 should-fix and 6 nits.  The plan takes each, but the review base: it stays `b1f588b`, as the orchestrator's brief says.
* 2026-11-08, agent (team leader): plan review round 2 found 4 should-fix and 3 nits.  The plan takes each.  The orchestrator commits the `CLAUDE.md` and `.claude/rules` diffs before Step 1, so each commit passes `check --tier commit`.
* 2026-11-08, agent (team leader): `m4a` is Apache-2.0 only, since Apple's reference is.  Reason: "The licence of the port".
* 2026-11-08, agent (team leader): the writer has `Order::MoovLast` for M2's fixture.  Reason: D6 forbids a copy of the writer in a test.
* 2026-11-08, agent (team leader): an `iTunSMPB` tag is not in M1.  M2 adds it to the writer if a test of D7 needs it.

## Outcomes and retrospective

Not started.
