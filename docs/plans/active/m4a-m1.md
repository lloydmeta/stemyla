# M4A, M1: the M4A writer and the ALAC encoder

## Purpose

This is the milestone plan for M1 of `plans/active/m4a.md`.  The parent's M1 section stays the criteria.  M1 adds a new crate, `m4a`, in the Pure layer.  The crate makes the bytes of an M4A file that holds 16-bit stereo Apple Lossless (ALAC).  It does no input or output, and no other crate uses it yet.  M2 uses it for the decode fixtures, and M3 for the save.

Readers: the implementer, the review agents of M1's rounds, the orchestrator, and the team leaders of M2 to M4.

## Context and orientation

The state at the start, on 2026-11-08, at commit `d2a81bb` on branch `feature/m4a`.  The review base of M1 is `b1f588b`.  The range from it also holds the orchestrator's plan commits `b18c494`, `f4ab3bc` and `d2a81bb`.  It holds this plan's commit `1323939`.  It holds the orchestrator's commit `b17d023` of `CLAUDE.md` and `.claude/rules`.

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
* A compressed packet that is not smaller than an escape packet is written again as an escape packet (`set_position(start_position)`).  The port writes the compressed try to a buffer of the encoder, and writes the escape packet to the output instead (review round 1, 1-2).
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

Apple's ALAC reference is Apache-2.0 only.  `alac-encoder` offers MIT OR Apache-2.0, and the port takes it under Apache-2.0.  So `m4a/Cargo.toml` has `license = "Apache-2.0"`, not `license.workspace`.  The licence section of `README.md` says that `m4a/` is Apache-2.0 only.  Decision 0029 (`proposed`) records this, since D1, D2 and D6 choose no licence (review round 1, 1-4).

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

* `m4a/src/bits.rs`: a bit writer with a 64-bit accumulator, as D2 says.  It has `write(value, bits)` for 1 to 32 bits, `byte_align`, `bit_len` and `truncate(bit_len)`.  Review round 1 removed `truncate` (1-2).  Each write keeps only the low `bits` of the value, as `write_lte25` does.
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
  * The reference crate writes the whole compressed try into its output before its size check and rewind (`encode_stereo`).  A try can pass `MAX_PACKET_BYTES`, and the bit writer also writes up to 4 bytes past the bit position (`bit_buffer.rs`).  So the test gives it the source's own work-buffer size plus 8: 4096 x 2 x 5 + 1 + 8 bytes.  The port's `out` grows by at most `MAX_PACKET_BYTES` (review round 1, 1-2).  The escape path of the reference crate casts the input to `&[i16]` (`from_raw_parts`), so the test asserts that the input's address is even before each call.
  * A rewind signal: in each packet, the first eighth is quiet and the rest is random.  The trial encode then predicts a small packet, and the full encode is rewound to an escape packet.  The differential test includes it.
  * A random signal makes escape packets, and a sine makes compressed packets.  The test asserts that both kinds occur.  The escape flag is bit 22 from the packet start: after the 3-bit tag, the 4-bit instance tag, 12 zero bits, the partial bit and 2 shift bits.
  * Each packet is at most `MAX_PACKET_BYTES`.
* A unit test of the bit writer: `truncate` to a bit length in the middle of a byte, then a write, gives the same bytes as a writer that never had the truncated bits.  Review round 1 removed it with `truncate`.
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
* `docs/decisions/0028-m4a-with-alac-as-the-save-format.md`, `Status: accepted (2026-11-08, by the maintainer, as D1, D2 and D6 of plans/active/m4a.md)`.  The maintainer's answers choose its content (the parent's Decision log).  Context: Apple's tools read ALAC M4A but not FLAC (the parent's Purpose).  Decision: the save format M4A with ALAC; the writer in the `m4a` crate of the Pure layer (D6); the encoder as a safe port of `alac-encoder` 0.3.0 (D2), checked against it.  Consequences: the D6 rules, the 4 GiB limit, M4's `esds` entry, and `m4a` under Apache-2.0 only.  Review round 1 moved the licence to decision 0029, `proposed` (1-4).
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
* [x] 2026-11-08, Step 1: the crate and its place in the layers.  First failures: `each_shipped_crate_lints_mem1_copies` failed with "m4a/clippy.toml lacks `path = "slice::to_vec"`" before `m4a/clippy.toml` existed.  `cargo clippy -p m4a --all-targets` failed with "this lint expectation is unfulfilled" on the canary.  `each_include_glob_routes_its_reviewer` failed with "m4a/src/lib.rs should route memory-reviewer ..., got []" before the change to `REVIEWERS`.  `the_fixture_graph_is_the_real_graph` failed on the new member before the change to `fixture()`.  `check_runs_each_check_on_a_repo` failed on the missing `m4a` lines of `README.md` and `docs/ARCHITECTURE.md`.  `every_reviewer_has_an_agent_file` failed until `git add` of `m4a/src/lib.rs`.
* [x] 2026-11-08, Step 2: the bit writer and the ALAC encoder.  First failures, with a stub writer that writes nothing and a stub `encode` that gives 0 bytes: each `bits` unit test failed on its `assert_eq!` (`a_truncate_to_a_byte_boundary_drops_whole_bytes` on the bound assert of `truncate`).  The differential test failed with "silence, 1 frames: packet 0 differs".  `noise_gives_escape_packets_and_a_sine_compressed_packets` failed on "all(|p| p.len() == MAX_PACKET_BYTES)", `each_packet_is_appended_after_the_bytes_of_out` on "size > 0", and the two unit tests of `alac` on `assert_eq!` and "small < big".  The three `#[should_panic]` tests failed with "did not panic as expected".  The real port then passed each test at the first run, in debug and in release.
* [x] 2026-11-08, Step 3: the box writer.  First failures, with a stub `Layout` of 0 packets that checks nothing and gives empty heads: the structure test failed at `assert_eq!(layout.packets(), 3)`; the `MoovLast` and `elst` tests at the box reader's assert "a box header needs 8 bytes"; the length test, the refusals of a rate, an edit and a packet count or size each on its `assert_eq!`; the 4 GiB test at `assert_eq!(one.packets(), 1)`.  With a stub `Display` that writes nothing, `each_refusal_says_what_is_wrong` failed on its `assert!`.
* [x] 2026-11-08, Step 4: the round trip through Symphonia.  First failures, with a stub `alac_file` that encodes silence of the same length and checks neither the length nor the rate: the round trip failed with "square, 1 frames: the samples differ", the `MoovLast` test with "square, 4097 frames: the samples differ", the edit test on its `assert!`, the refusal test on its `assert_eq!`, and `an_odd_length_panics` with "did not panic as expected".
* [x] 2026-11-08, Step 5: the size check, the notices and decision 0028.  First failure, with the full packets forced to escape packets: `a_music_like_signal_gives_fewer_bytes_than_its_wav` failed with "1765889 >= 1764044".  The differential test now also runs the music-like signal.
* [ ] Review rounds.
  * [x] 2026-11-08, review round 1 fixes, before the missed mutants.  First failures: 1-1 `an_edit_past_the_end_is_refused` panicked "attempt to add with overflow" in debug, and failed its `assert_eq!` in release (`Ok` for `skip` 1, `frames` u64::MAX).  1-2 `a_packet_never_grows_out_past_max_packet_bytes` failed with "rewind: packet 0", capacity 32,776, not 16,388.  1-9 failed on `worst_case_bytes` 4294981961, not 4294981977, with the `stts` term of the worst case removed.  1-5: with a writer and an encoder that keep 44,100 Hz, `a_file_at_48_khz_holds_its_rate_in_each_place` failed at "mvhd times", and the 48 kHz round trip failed too.  1-13: with the cookie of a new encoder, `alac_file_writes_the_final_cookie_and_its_rate` failed on the cookie (maxFrameBytes 0, not 408).  1-10: with left and right swapped in the stereo mix, only the two music-like round trips failed ("music-like, 4097 frames: the samples differ").  1-8: 20 tests of `xtask/src/check.rs` failed on their asserts before `lines()` had the `engine` line, `the_ci_runs_the_engine_tests_alone` on "ci.yml has no `- run: cargo test --locked -p engine`".
  * [x] 2026-11-08, review round 1, the missed mutants of `m4a/src/alac.rs`.  `the_port_gives_the_packets_of_the_reference_on_the_probe_signals` runs the probe signals of the validator at 4096 and 4000 frames: impulses on one channel, noise of amplitude 2, sparse noise, and eighth splits of noise and silence.  A scan with each mutant applied found the amplitudes and impulses that the escape decision needs (`if bits_v < min_bits_v`, `min_bits`, `escape_bits` and the size check of the try).  `cargo mutants -p m4a --file m4a/src/alac.rs` then: 314 mutants, 292 caught, 12 missed, 9 unviable, 1 timeout.  Each of the 26 confirmed and unproven mutants is caught.  The 6 missed in `write_coefs`, `write_escape_packet`, `pc_block` and `dyn_code` are the ones that the validator refuted as equivalent.  The other 6 were on `TRY_BYTES`, new in fix 1-2; `the_try_buffer_is_sized_once_to_the_work_buffer_of_the_source` now catches them (first failure: 40959, not 40961).
  * [x] 2026-11-08, review round 2 fixes.  2-1: TD-79 records that `alac-encoder` breaks DEPS-1's "maintained".  2-2: the reason of the `cargo test -p engine` rule of tier `commit` now names the dependents of `engine` that turn on `engine/cpal`.  2-3, 2-5 and 2-6: plan and decision text.  2-4: `the_ci_runs_the_engine_tests_alone` fails if ci.yml loses its workspace test line.  First failure, with that line renamed in the test: "ci.yml runs the workspace tests".  2-7: the doc of `PacketKind::Rewound`.

## Surprises and discoveries

* 2026-11-08, Step 2: no kept caller of `BitBuffer::write` can pass a value with bits above `num_bits`.  Its callers are `write(num_samples, 32)` and `dyn_code_32bit`'s `write(n, 17)`.  `n` is at most 131,071, below 2^17: each prediction error is 17-bit signed after `(del << 15) >> 15`, and `pc1[0]` is a mix value of at most 17 bits.  Also, with 17 bits the shift is `32 - (position & 7) - 17`, at least 8, so the masked branch runs.  The unmasked branch (shift below 0) runs only for 32 bits.  So no unit test shows the case.
* 2026-11-08, Step 2: the one loop of `pc_block` for 4 and 8 coefficients, with the `Wrapping` sum, gives the same bytes as the source for every signal and length.  A mutation of its stop condition (`del * direction <= 0` to `< 0`) failed the differential test at "square, 4095 frames: packet 0 differs".  So the test sees a change in the adaptation.
* 2026-11-08, Step 5: the music-like signal of 10 s gives 975,348 bytes of M4A against 1,764,044 bytes of 16-bit WAV, a ratio of 0.553.
* 2026-11-08, Step 4: Symphonia 0.6.1 opens and decodes each file of the round trip, the file of 0 frames too.  For 0 frames it states 0 frames and gives no packet.  No case needs the orchestrator.
* 2026-11-08, Step 4, for M2: feature unification.  `cargo tree --workspace -e features -i symphonia-format-isomp4` shows the feature `alac` from the dev-dependency of `m4a`, beside `engine`'s `flac`.  So `cargo test --workspace` builds one Symphonia with `isomp4` and `alac`, and `engine`'s tests then decode M4A before M2 turns the features on.  `cargo tree -p engine -e features -i symphonia-format-isomp4` finds no such package.  Review round 1 (1-8) closes the gap: CI and `check --tier push` run `cargo test -p engine` alone, and tier `commit` runs it when its crate tests build `engine` with a member outside the closure and dependents of `engine`.
* 2026-11-08, Step 4: Symphonia 0.6.1 reads no `elst`.  A file with an `Edit` of 2,112 frames decodes to every frame of its track (`a_file_with_an_edit_decodes_its_whole_track`).  This agrees with the parent's Context; M2 trims in `engine`.
* 2026-11-08, Step 2: by a reading of the source, the values that `pc_block` leaves in the buffers reach the bytes only within one packet.  The trial over `frames / 32` keeps the entries from `max(frames / 32, 9)` to `frames / 8` that the trial of `frames / 8` of the same packet wrote, and the next `dyn_comp` reads them.  An entry from an earlier packet is read (`input[1..=numactive]`) but no read of it reaches `dyn_comp`, and no coefficient adapts on it.  The port keeps the buffers at 4096 entries in any case.  The differential test runs each length from a new encoder, so it does not separate the two cases.

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
* 2026-11-08, agent (implementer): the trial encodes write to a `BitCount`, a `BitSink` that counts bits, not to a work buffer.  Reason: they need only the bit count, so the port makes no work buffer.  `dyn_comp` takes any `BitSink`.
* 2026-11-08, agent (implementer): the coefficients are `[[i16; 8]; 2]` for each channel, one set of 4 and one of 8, not `[[[i16; 16]; 16]; 8]`.  Reason: 16-bit stereo uses only those two sets, of channel 0.
* 2026-11-08, agent (implementer): a crate-private `encode_packet` gives the size and a `PacketKind` (compressed, escape, rewound); the public `encode` gives the size only.  Reason: a unit test proves that the rewind path runs, and the public API stays that of the sketch.
* 2026-11-08, agent (implementer): `Layout` has `tail_len()` beside `head_len()`.  Reason: the 4 GiB test computes the worst case of `MoovLast`, whose `moov` is in the tail, with no call of `tail` on 262,000 sizes.
* 2026-11-08, agent (implementer): `Error` has five variants: `SampleRate`, `EditPastEnd`, `TooLarge`, `PacketCount` and `PacketSize`, with a `Display` text each.  `tail` checks the sizes as `head` does.
* 2026-11-08, agent (implementer): the `hdlr` name is "SoundHandler", and `stsc` has its one entry also for a file of 0 packets.
* 2026-11-08, agent (implementer): `alac_file` panics on an odd length, as `AlacEncoder::encode` does, and gives the `Error` of `Layout` for a refusal.  Reason: an odd length is a bug of the caller, and a refusal depends on the input.
* 2026-11-08, agent (implementer): the adaptive Golomb parameters are constants.  Reason: each call of the source gives `mb` 10, `pb` 40 (`(4 * 40) / 4`) and `kb` 14, with `fw` equal to `sw`, so the row logic of `dyn_comp` has no effect.

* 2026-11-08, agent (implementer, review round 1, 1-7): `alac-encoder` 0.3.0 has had no release since January 2022, so DEPS-1's "maintained" does not hold.  It is accepted as a frozen reference: pinned exactly, test-only, never shipped and never updated.  `m4a/Cargo.toml` gives the same reason.  Review round 2 recorded the breach as TD-79 (2-1).
* 2026-11-08, agent (implementer, review round 1, 1-12): `alac_file` holds a whole file in memory, so M3's worker must not call it.  M3's brief names this.  No code change.
* 2026-11-08, agent (implementer, review round 1, 1-2): the compressed try of a packet goes into a buffer of the encoder, sized once in `new` to the source's work buffer (4096 x 2 x 5 + 1 bytes).  `encode` appends it to `out` only if it is smaller than an escape packet.  Else it writes the escape packet to `out`.  So `out` grows by at most `MAX_PACKET_BYTES`.  `BitWriter::truncate` then had no caller, and its two tests went with it.
* 2026-11-08, agent (implementer, review round 1, 1-4): `m4a` is Apache-2.0 only by decision 0029, `proposed`, not by decision 0028.  Reason: 0028 is accepted as D1, D2 and D6, which choose no licence.
* 2026-11-08, agent (implementer, review round 1, 1-8): tier `commit` runs `cargo test -p engine` only when its crate tests build `engine` with a member outside the closure and dependents of `engine`.  Reason: the line guards the Symphonia features of the tests of `m4a`.  Review round 2 found that a dependent of `engine` also turns on features of `engine`: `app` and `xtask` turn on `engine/cpal` (2-2).  Tier `push` and CI run `cargo test -p engine` alone, so they cover that case.
* 2026-11-08, orchestrator: the `cargo test -p engine` line of CLAUDE.md lands in the commit of the round 1 fixes, since a commit of that line alone fails `check --tier commit`.

## Outcomes and retrospective

Not started.
