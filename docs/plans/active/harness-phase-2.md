# Harness phase 2: deterministic sensors

## Purpose

Phase 2 of `plans/active/harness-engineering.md` turns each rule that a machine can check into a test, a lint or a hook.  Today 8 of 21 rules in `invariants.md` are "prose only", and four known breaches went unnoticed.  After this plan, a breach fails a check that names the rule ID and the fix, before a person or a reviewer agent reads the diff.

Each milestone is one small PR, revertible on its own.  The maintainer reviews and merges each PR.

## Context and orientation

The state on 2026-10-19, at commit `a9ace3a` (phase 1 merged as PR #13):

* Rules: `docs/invariants.md` states each rule, why it holds and what enforces it.  `docs/tech-debt.md` lists the gaps (TD-1 to TD-11, TD-21) and the breaches (TD-12 to TD-17).
* The one structural check: `xtask docs-check` (`xtask/src/docs.rs`), run as the test `the_map_and_docs_match_the_code` in `xtask/tests/docs.rs`.  Its pattern is the model for new checks: a pure function returns problems, unit tests feed it a planted breach, and one test runs it on the real tree.
* CI: `.github/workflows/ci.yml` has the jobs `fmt`, `clippy`, `test`, `wasm-build`, `browser-tests` and `publish`.  The `test` job excludes `roformer-gpu` and sets `UPDATE_SNAPSHOTS: "1"`.
* Clippy: the workspace `Cargo.toml` has `[workspace.lints.clippy]`; no `clippy.toml` exists.  `dsp/src/lib.rs` has `#![allow(clippy::all)]`.
* Claude Code: `.claude/` holds `rules/` only.  No `settings.json`, hooks or permission rules exist.
* The audio callback: `engine::slot::PlayerSlot::render_interleaved` (`engine/src/slot.rs`) calls `Player::process` (`engine/src/player.rs`).  The app builds `engine` with only the `stretch-wsola` feature, so it ships two stretchers: `Varispeed` and `WsolaStretcher` (`engine/src/stretch/wsola_adapter.rs`).
* The demo song: `synth` (`synth/src/lib.rs`) has `generate()` and the file writers `write_wav` and `write_song`.  The writers need `hound`, `serde`, `serde_json` and `anyhow`.  `app` calls `synth::generate()` and `synth::SAMPLE_RATE` in `app/src/lib.rs`, and `synth::write_song` in `app/tests/ui.rs` only.

Facts checked for this plan:

* wsola 0.1.0's `TimeStretch::pull` returns a new `Vec` (`self.output.drain(0..n).collect()`).  wsola 0.2.0 (published 2026-10-12, MIT) returns `impl Iterator<Item = f32>`, which is `self.output.drain(0..n)` and does not allocate.  Its `step` algorithm is unchanged except for an `output.reserve`, and its constructors now take `NonZeroU32` and `NonZeroU16` ([crates.io](https://crates.io/crates/wsola), [docs.rs](https://docs.rs/wsola/0.2.0/wsola/struct.TimeStretch.html); source read from the published crate).
* `dsp::double_speed` returns a new `Vec` on each call (`dsp/src/double_speed.rs`).  `Player::render_chipmunk` calls it twice per block.
* A global allocator may use `std::thread_local`, which the standard library guarantees does not call the global allocator.  The optimiser may remove an allocation, so a count is evidence, not proof ([`GlobalAlloc`](https://doc.rust-lang.org/std/alloc/trait.GlobalAlloc.html)).
* Clippy reads `clippy.toml` from `CLIPPY_CONF_DIR`, else `CARGO_MANIFEST_DIR`, else the current directory, and walks up to the root from there ([clippy configuration](https://doc.rust-lang.org/clippy/configuration.html)).  So a `clippy.toml` in a crate's folder applies to that crate only.  `disallowed-methods` entries take `path`, `reason`, `replacement` and `allow-invalid` ([lint configuration](https://doc.rust-lang.org/clippy/lint_configuration.html)).
* naga 30.0.1 is in `Cargo.lock` through wgpu.  Its WGSL front end needs the `wgsl-in` feature, off by default ([features](https://docs.rs/crate/naga/30.0.1/features)).  `EntryPoint::workgroup_size` is `[u32; 3]`, and `naga::proc::Layouter` gives type sizes ([docs.rs](https://docs.rs/naga/30.0.1/naga/proc/struct.Layouter.html)).  naga's validator does not check device limits (`docs/references/webgpu-limits.md`).
* egui_kittest 0.36.2's `SnapshotOptions` has `threshold` (default 0.6) and `max_failed_pixels` (default 0), each settable per operating system ([docs.rs](https://docs.rs/egui_kittest/0.36.2/egui_kittest/struct.SnapshotOptions.html)).
* Claude Code: a PreToolUse hook that exits with code 2 blocks the tool call.  `tool_input.file_path` is always absolute for the file tools.  A Stop hook gets `stop_hook_active`, true when Claude Code already continues because of a Stop hook.  SessionStart stdout becomes context ([hooks](https://code.claude.com/docs/en/hooks)).  Deny rules win over allow rules and apply to each part of a compound command.  They match the command text only, so they "cover the invocation Claude usually produces and [are not] a security boundary around the program" ([permissions](https://code.claude.com/docs/en/permissions)).

Terms:

* Sensor: a check that runs on every change and fails on a breach.  Computational sensors are deterministic (tests, lints).
* Planted breach: a deliberate breach that shows a check can fail.  Where possible it stays as a permanent unit test of the checker.
* Warm-up: the work a `Player` does before it enters the slot, on the thread that makes it.

## Plan of work

### What every milestone does

* Each check's message names the rule ID and the fix, as `docs-check` does: `LAYER-1: app depends on roformer-gpu.  Fix: ...`.
* Each check has a unit test with a planted breach that fails, and runs on the real tree.
* The PR updates the rule's `* Enforced by:` line in `invariants.md` to name the new test.  `docs-check` confirms the test exists.
* The PR deletes a closed `tech-debt.md` entry and adds its ID under `## Closed`, or narrows a partly closed one.  A new breach that the check finds and the PR does not fix gets an `#[expect(..., reason = "TD-n: ...")]` and a tech-debt entry.
* The PR adds the sensor to the components table in `docs/harness.md`, and updates `docs/working-with-agents.md` when a person's work changes.
* Before each commit: fmt, clippy, `cargo test --workspace --exclude roformer-gpu`, `cargo test -p roformer-gpu --lib` and `--doc`, and `docs-check` pass (GIT-2).  Each commit is signed.
* After implementing: three review rounds, each by a new subagent with fresh context.  Verify each finding, then fix real ones with tests.
* The PR description says what was verified, how, and what was not.  It is written to a file and passed with `--body-file`.
* A new crate is vetted and pinned if young (DEPS-1).  This plan adds only naga (already in the lock file), syn (already in the lock file) and wsola 0.2.0.

### M1: CI runs roformer-gpu's GPU-free tests (TD-1, part)

* Change: add `cargo test --locked -p roformer-gpu --lib` and `cargo test --locked -p roformer-gpu --doc` to the `test` job.  The lib tests (`workgroup_memory_within_defaults`, `sources_are_complete`, the `net.rs` and `bench.rs` tests) need no adapter.  The doctests include MEM-3's `compile_fail` check of `Rec`, which has also never run in CI.
* Acceptance: the CI log of the `test` job lists `workgroup_memory_within_defaults ... ok` and the `Rec` doctest.
* Planted breach: locally, raise one `TILES` entry's `pad` until its workgroup memory passes 16,384 bytes.  `cargo test -p roformer-gpu --lib` fails.
* Docs: GPU-1 and MEM-3 enforcement lines say CI runs these.  TD-1 keeps only the naga test.

### M2: every shader is checked against the default limits (closes TD-1; GPU-1)

* Change: add `naga = { version = "=30.0.1", default-features = false, features = ["wgsl-in"] }` as a dev-dependency of `roformer-gpu`, the version already in the lock file.  Add the unit test `every_kernel_fits_the_default_limits` in `roformer-gpu/src/lib.rs`.  It parses each generated source that `sources_are_complete` covers, which includes `matmul.wgsl` as the `s64` tile.  It also parses the other shader files that are not templates: `softmax.wgsl`, `rmsnorm.wgsl` and `elementwise.wgsl`.  The three templates hold `{NAME}` placeholders and are not valid WGSL.  The test asserts the limits in `references/webgpu-limits.md`:
  * workgroup x and y at most 256, and z at most 64;
  * x * y * z at most 256;
  * workgroup storage at most 16,384 bytes;
  * at most 8 storage buffers and 12 uniform buffers per entry point.
* Acceptance: the test passes on the real shaders.  A unit test feeds the checker `@workgroup_size(1, 1, 128)` and a 20,000-byte `var<workgroup>` array; it reports both, with the limit's name and GPU-1.
* Note: confirm naga 30.0.1's API (parse, `Layouter`, global address spaces) before writing.  If `wgsl-in` adds a crate that the lock file lacks, record it in `THIRD-PARTY.md`'s dev section.

### M3: a required test fails instead of skipping (closes TD-8; VERIFY-1, GPU-2)

* Change: the environment variable `STEMYLA_REQUIRE` holds a comma-separated list of `gpu` and `fixtures`.  Every test that skips for want of an adapter or a fixture calls one helper.  The helper panics when that kind is required, and names VERIFY-1.  An unknown word in the list also fails, so a typo cannot turn the check off.
* Where: the skips in `roformer-gpu/tests/kernels.rs`, `memory.rs` and `reference.rs`, and `roformer/tests/reference.rs`.  Each crate gets a small test-support module with the helper and its unit tests.
* Acceptance: unit tests of the helper cover required and missing, required and present, not required, and an unknown word.  On the maintainer's Mac, `STEMYLA_REQUIRE=gpu cargo test -p roformer-gpu --release` passes, and `STEMYLA_REQUIRE=fixtures` fails while the fixtures are absent.
* Docs: GPU-2 and the GPU recipe in `working-with-agents.md` give `STEMYLA_REQUIRE=gpu` as the command for GPU evidence.  If the fixtures stay absent on the Mac, record that in `tech-debt.md`.

### M4: the app links only song generation (TD-4, part; maintainer decision)

* Change: in `synth/Cargo.toml`, make `hound`, `serde`, `serde_json` and `anyhow` optional, behind a feature `files` that is not a default.  Put `write_wav` and `write_song` behind `#[cfg(feature = "files")]`.  Derive `Serialize` on `Onsets` and `Truth` with `cfg_attr(feature = "files", ...)`.  `xtask` takes `synth` with `features = ["files"]`.  `app` keeps `synth` without features, and adds it with `files` as a native dev-dependency for `app/tests/ui.rs`.
* Acceptance: `cargo tree -p app --target wasm32-unknown-unknown -e normal -i hound` finds nothing, and the same holds for `serde_json`.  `cargo test -p synth` and `cargo test -p synth --features files` pass.  `trunk build --release` passes.
* Docs: regenerate `THIRD-PARTY.md` with the command at its top; `hound`, and `serde_json` if nothing else needs it, leave the list.  Remove the TD-4 note under the layers table in `ARCHITECTURE.md`.  Narrow TD-4 to the missing test.

### M5: crate layering is a test (closes TD-4; LAYER-1)

* Change: add `xtask/src/rules.rs` with pure checker functions, and `xtask/tests/rules.rs` that runs them on the real tree, as `docs.rs` does.  The first check reads `cargo metadata --format-version 1 --no-deps` and compares each workspace crate's workspace dependencies with an allowlist built from the layers table in `ARCHITECTURE.md`.  A second check runs `cargo tree -p app --target wasm32-unknown-unknown -e normal` and fails if it holds `hound` or `serde_json` (M4's result).
* Acceptance: the test `crates_depend_only_downwards` passes.  Unit tests plant an `engine -> app` edge and an `app -> roformer-gpu` edge; both fail with LAYER-1.  Adding `features = ["files"]` to `app`'s `synth` dependency fails the second check.
* Note: dev-dependencies count as edges unless the allowlist says otherwise; `app`'s dev-dependency on `synth` with `files` is listed.

### M6: dsp relaxes only the style lints DSP-1 needs (closes TD-2)

* Change: replace `#![allow(clippy::all)]` in `dsp/src/lib.rs` with the style lints that DSP-1's constructs trigger, named one by one with a reason each.  Find them by removing the allow and running clippy.
* Acceptance: `cargo clippy -p dsp --all-targets -- -D warnings` passes.  A planted correctness lint (for example `let x = x;` in a `dsp` function, or a self-comparison) now fails clippy.
* Rule: fixes in `dsp/` keep DSP-1's beginner style; a lint that only an iterator would fix goes on the allow list.

### M7: dsp's beginner style is a test (closes TD-3; DSP-1)

* Change: add a check to `xtask/src/rules.rs` that parses each file in `dsp/src/` with `syn` (full syntax, visitor) and allows only the listed constructs.  It is an allowlist: types `f32`, `usize`, `bool`, `&[f32]` and `Vec<f32>`; `fn`, `let`, `for` over a range, `while`, `if`, indexing, arithmetic, comparisons, and calls to `dsp` functions and the methods DSP-1 permits (`len`, `push`, `Vec::new`, `vec![...]` and the `f32` maths methods).  `#[cfg(test)]` modules are checked too.  The check also asserts that `dsp/Cargo.toml` has an empty `[dependencies]`.
* Acceptance: `dsp_is_beginner_style` passes on the real tree.  Unit tests plant `.iter()`, a closure, `swap` and a dependency; each fails with DSP-1 and the construct's file and line.
* Note: use the `syn` major version already in `Cargo.lock` for `xtask`'s dependency tree, so no new crate is downloaded.  Write the exact method list in the PR from a scan of `dsp/src/`.

### M8: Python and assets are tests over the tracked files (closes TD-5, TD-6; BUILD-1, ASSET-1)

* Change: two checks in `xtask/src/rules.rs` over `git ls-files`.  BUILD-1: no `*.py` except `tools/roformer_export.py`, and no `python` or `pip` in `.github/`, `app/Trunk.toml` or a `build.rs`.  ASSET-1: no tracked file with an audio or weight extension (`mp3`, `wav`, `flac`, `ogg`, `m4a`, `aac`, `safetensors`, `ckpt`, `pt`, `pth`, `onnx`, `npy`), and no tracked file over a size cap except an allowlist (the UI snapshot PNGs).  Add the missing extensions to `.gitignore`, and anchor its folder patterns at the root (`/audio/`, `/stems/`, `/dist/`, `/pages/`).
* Acceptance: `no_python_in_the_build` and `no_audio_or_weights_are_tracked` pass.  Unit tests plant `tools/x.py`, a `pip install` step and `song.flac`; each fails with its rule ID.  `git check-ignore -v engine/src/audio/x.rs` shows that a source folder named `audio` is no longer ignored.

### M9: young crates are pinned and the licence list is a test (TD-7, part; DEPS-1)

* Change: pin `wsola`, `timestretch` and `cute-dsp` in `engine/Cargo.toml` with `=`.  Add a check that reruns the `cargo tree` command at the top of `THIRD-PARTY.md` and compares each crate, version and licence with the tables.  Add a check that every dependency below 1.0 of a shipped crate is pinned with `=`.
* Acceptance: `third_party_lists_what_ships` and `young_crates_are_pinned` pass.  Unit tests plant a crate missing from the tables, a changed licence and a caret range on a 0.x crate; each fails with DEPS-1.
* Scope: `cargo deny` for licence policy and sources stays in TD-7.  It is a new tool and needs its own vetting.

### M10: the hand-written e2e messages match the protocol (TD-16, part; PROTO-1)

* Change: a check extracts the message type strings from `xtask/e2e/worker.js` and the scripts in `xtask/src/e2e.rs`, and compares them with the constants in `stem_protocol::kind`.  Types that exist only in the scripts (for example `worker-error`) are on a named allowlist.
* Acceptance: `e2e_messages_use_protocol_types` passes.  A unit test plants a misspelt type (`"segmnt"`) and fails with PROTO-1.
* Scope: failing on a missing field and a version handshake stay in TD-16.

### M11: the audio callback does not allocate after warm-up (closes TD-15; RT-1; maintainer decision)

* Rule: RT-1 becomes "the audio callback never waits, never frees a player, and never allocates or frees memory after warm-up".  Update `invariants.md`, the RT-1 line in `CLAUDE.md` and `.claude/rules/memory.md`.  Write `decisions/0003-rt-1-forbids-allocation.md` as `proposed`; the maintainer accepts it.
* Test first: `engine/tests/realtime.rs` is its own test binary, so its counting global allocator affects no other test.  The allocator wraps `System` and counts `alloc`, `alloc_zeroed`, `realloc` and `dealloc` in `thread_local` counters, so parallel tests do not mix.  The test `the_callback_does_not_allocate_after_warm_up` makes a song of six noise stems in the test, a `Player` for each stretcher the app ships (`StretcherKind::Varispeed` and `StretcherKind::Wsola`), and puts it in a `PlayerSlot`.  It then calls `render_interleaved` with blocks of 128, 512 and 4,096 frames.  The count must be zero from the first callback, through each of these changes:
  * tempo changes over 0.5 to 1.5;
  * chipmunk on and off;
  * a seek;
  * a loop wrap with crossfade;
  * mute and solo changes.
* The test fails before the fix, on wsola 0.1.0's `pull` and on `dsp::double_speed`.  Record both failures in the PR.
* Fix: bump to `wsola = "=0.2.0"`, and copy `pull`'s iterator straight into the output queues.  In `render_chipmunk`, decimate `chip_l` and `chip_r` into the output slices in place, in `engine`.  `dsp::double_speed` stays as the beginner version, and a unit test asserts that both give the same samples.  Warm-up in `Player::new` runs the stretcher at the highest tempo for long enough to reach its steady buffer sizes, then resets it; `reset` keeps capacity.
* Acceptance: the test passes.  `m4_stretch` and `the_callback_does_not_allocate_after_warm_up` pass on wsola 0.2.0.  Output on the demo song is identical to wsola 0.1.0's, sample for sample, or the latency table in `wsola_adapter.rs` is measured again with its example and the change is stated in the PR.
* Scope: `timestretch` and `cute-dsp` are bench-only and are not in the app; the RT-1 text says it covers the stretchers the app ships.  The closure in `engine/src/live.rs` also calls `Schedule::next`.  If the test cannot reach it, record that gap in `tech-debt.md`.

### M12: song-sized copies are a lint (MEM-1)

* Change: add `clippy.toml` to `app/`, `worker/` and `engine/` with `disallowed-methods` for slice `to_vec` and `js_sys` typed-array `to_vec` (`Uint8Array`, `Float32Array`), each with a reason that names MEM-1 and the fix.  The known breaches get `#[expect(clippy::disallowed_methods, reason = "TD-13: ...")]` and `"TD-14: ..."`.  Other sites that copy small buffers get an `#[expect]` with their reason.
* Acceptance: clippy passes.  A planted `samples.to_vec()` in `engine` fails clippy with the MEM-1 reason.  Removing an `#[expect]` whose site no longer copies fails clippy (`unfulfilled_lint_expectations`).
* Note: first confirm the path syntax that clippy accepts for the inherent slice method, with the planted case.  Each `clippy.toml` holds the crate's full configuration, since clippy reads only the first file it finds.  TD-12's `files.clone()` is a `Vec` clone, not `to_vec`; it stays in `tech-debt.md` for M18.

### M13: GPU buffers are made only by GpuBuffer (MEM-3)

* Change: add `roformer-gpu/clippy.toml` with `disallowed-methods` for `wgpu::Device::create_buffer` and `wgpu::util::DeviceExt::create_buffer_init`, with a MEM-3 reason.  `GpuBuffer`'s constructor carries the one `#[expect]`.
* Acceptance: clippy passes.  A planted `device.create_buffer(...)` in `net.rs` fails clippy with the MEM-3 reason.

### M14: Claude Code guards (GIT-1, ASSET-1, PR bodies)

* Change: add `.claude/settings.json` with deny rules for `Bash(git push *)` and the `publish` forms of `cargo run -p xtask`.  Add a PreToolUse hook, `cargo run -q -p xtask -- hook pre-tool-use`, that reads the hook's JSON on stdin and exits 2 with a reason:
  * a Write or Edit whose `file_path` has an audio or weight extension (ASSET-1);
  * a Bash command that runs `git add` with such a path, or `git add -f` (ASSET-1);
  * a Bash command with `git push` or `xtask` `publish` in any form, as a second layer behind the deny rules (GIT-1);
  * `gh pr create` or `gh pr edit` with `--body` instead of `--body-file` (failure 7 in the phase plan).
* Acceptance: unit tests in `xtask` feed the hook each planted input and assert exit 2 and the rule ID, and assert exit 0 for ordinary edits and commands.  In a session, `git push --dry-run` is refused.
* Known limits, stated in `harness.md`: deny rules and hooks match command text and are not a security boundary.  A hook that cannot build or run does not block.  The controls stay M8's CI test (ASSET-1) and the `main` ruleset (GIT-1).
* Docs: the guide's tools table names `hook`.  A "What the agent cannot do" note says what is refused and why.

### M15: Claude Code feedback hooks

* Change: in `.claude/settings.json`, add hooks that call `xtask hook <event>`:
  * PostToolUse on `Edit|Write`: run `rustfmt --edition 2024` on the edited `.rs` file.
  * Stop: run `cargo fmt --all --check` and `docs-check`; on failure, exit 2 with the output so the agent continues.  When `stop_hook_active` is true, exit 0, so the hook cannot loop.
  * SessionStart: report whether `safaridriver` and a chromedriver are found and answer, from the facts in `references/local-browser-testing.md` (failure 3).
* Acceptance: unit tests cover the Stop hook's loop guard, and a non-Rust file is not formatted.  The Stop hook finishes in under 30 seconds on the maintainer's Mac, measured and stated in the PR.

### M16: a PR cannot silently remove tests

* Change: add `xtask test-list`, which prints the sorted names from `cargo test --workspace --exclude roformer-gpu -- --list`.  The `test` job runs it on the PR head and on the merge base, in a worktree with the same target folder.  It fails if a name on the base is missing from the head, unless the PR has the label `tests-removed`.  The PR description must then say why.
* Acceptance: unit tests of the comparison cover a removed test, a renamed test and an added test.  A draft PR that deletes a test fails the job, and passes when the label is added.
* Note: a label is set by a person with triage access; the PR body is author text.  Measure the extra job time and state it in the PR.

### M17: UI snapshots are compared in CI (closes TD-9; VERIFY-1)

* First, measure: run `app/tests/ui.rs` on CI with comparison on, against the Metal snapshots, and record the per-pixel distance and the failed pixel count for each snapshot.
* Decision point for the maintainer: (a) a Linux threshold through `SnapshotOptions`, if a planted change still fails under it; or (b) a second set of snapshots made on the CI renderer.
* Change: remove `UPDATE_SNAPSHOTS: "1"` from the `test` job and apply the choice.
* Acceptance: CI compares the snapshots.  A planted change (the muted lane's colour) fails the `test` job on CI and on the Mac.

### M18: the breaches the checks found (TD-12, TD-13, TD-14, TD-17)

One PR each, after M12.  Each removes its `#[expect]`, adds a test that pins the fix, and closes the tech-debt entry (`## Closed`).

* TD-13: keep the `Uint8Array` in `Event::Mp3`; a browser test asserts that no copy is made.
* TD-14: interleave from the JS arrays in blocks; a test bounds the worker's memory during an encode.
* TD-12: save the dropped files first, or decode from a shared buffer; a test pins the sharing.
* TD-17: a wasm-bindgen test writes a `subarray` through `app/src/store.rs` and reads back the exact bytes; copy views if it fails.

### M19: close out phase 2

* Person step, done: the maintainer made `browser tests (Chrome)` a required check in the `main` ruleset on 2026-10-19.
* Update the parent plan's progress, the guide's status and changelog ("phase 2"), and `harness.md`.  Count the "prose only" lines in `invariants.md` and record the change from 8.
* Move this plan to `plans/completed/` and fill its retrospective.

## Validation and acceptance

* `grep -c "Enforced by: prose only" docs/invariants.md` falls from 8 to 3.  BUILD-1, DEPS-1, LAYER-1, DSP-1 and GIT-1 gain tests; ASSET-1 and RT-1 gain tests beside what they have.  MEM-4, GPU-3 and GIT-3 stay prose only: phase 4 covers MEM-4 and GPU-3, and nothing in view checks GIT-3.
* Each check's planted breach fails it: the unit tests in `xtask/src/rules.rs`, `roformer-gpu` and `engine/tests/realtime.rs`, and the clippy cases stated in each PR.
* `cargo test --workspace --exclude roformer-gpu`, `cargo test -p roformer-gpu --lib` and `--doc`, clippy and `docs-check` pass; CI is green on `main` with browser tests required (the phase gate).

## Idempotence and recovery

* Each milestone is one PR and reverts on its own.  A reverted check removes its enforcement and nothing else; revert its `* Enforced by:` line and restore its tech-debt entry in the same commit.
* Checks read the tree and write nothing, so they can run any number of times.  `xtask test-list` writes only to stdout.
* M11 changes a rule.  If the maintainer rejects decision 0003, revert M11's rule text and keep the test as a measurement, marked in `tech-debt.md`.
* To resume: read `## Progress`, then `git log --oneline main` to find the last merged milestone.

## Progress

* [x] 2026-10-19: plan written.  Survey of the code, the CI workflow, wsola 0.1.0 and 0.2.0, and the docs for clippy, naga, egui_kittest and Claude Code hooks.
* [x] 2026-10-19: plan reviewed by the maintainer, who asked for M1.
* [x] 2026-10-19: M1 implemented locally: CI runs roformer-gpu's GPU-free tests and doctests.  14 lib tests and 2 doctests pass (15 lib tests after the review added one).  `pad: 66` on `i64` fails `workgroup_memory_within_defaults` with "i64: 16640 bytes".
* [x] 2026-10-19: M1 review: `sources_are_complete` missed 8 placeholders; it now finds any `{NAME}`.  Deleting the `{BM}` replace fails it with `i64: ["{BM}", "{BM}"]`.  Round 3: the scan's test now covers a name with a digit (`{SOFTMAX_1}`); a scan without digits fails it.
* [x] 2026-10-19: M1, three review rounds done and their findings fixed.
* [x] 2026-10-19: M1, CI (PR #14, run 39199280124): the `test` job lists `workgroup_memory_within_defaults ... ok`, 15 lib tests and both `Rec` doctests.  The new steps took 67 s (`--lib`, mostly the wgpu rebuild) and 1 s (`--doc`).
* [x] 2026-10-19: M1, PR #14 merged by the maintainer (with the plan).
* [x] 2026-10-19: M2 implemented locally.  naga 30.0.1 is a dev-dependency with `wgsl-in`.  `cargo tree` and `Cargo.lock` show no new crate (936 packages before and after).  wgpu already turns on `wgsl-in`.
* [x] 2026-10-19: M2, `every_kernel_fits_the_default_limits` passes on 7 sources: 4 tiles, attention, `softmax.wgsl`, `rmsnorm.wgsl` and `elementwise.wgsl`.  The planted shader (`@workgroup_size(1, 1, 128)`, 20,000 bytes) gives exactly two breaches.  Each names the limit, GPU-1 and the fix.
* [x] 2026-10-19: M2, planted breaches in the real tree fail the test: z = 128 in `softmax.wgsl`, 5,000 floats in `rmsnorm.wgsl`, and `pad: 66` on `i64`.  naga counts 16,640 bytes for `i64`, as M1's hand count does.
* [x] 2026-10-19: M2, TD-1 is the first closed ID.  `docs-check` now reads `## Closed` in `tech-debt.md` (maintainer's choice).  It found a stale `TD-1` in `docs/README.md`.
* [x] 2026-10-19: M2 review round 1: 3 findings, all real and fixed with tests.  (1) A `## Closed` heading in any doc hid its IDs from the check; now only the ID files' sections are left out.  (2) "Closed but defined again" named the wrong file.  (3) The docs claimed every limit.  The test now checks `maxBindGroups`, and `every_layout_matches_its_shaders` ties the layouts to the shaders.  Planted: an extra `MATMUL_BINDS` entry and `@group(4)` in `softmax.wgsl` fail.
* [x] 2026-10-19: M2 review round 2: 7 findings, all real and fixed.  The `## Closed` bullets' text is now checked; only the opening ID is left out.  New tests cover values at each limit, every entry point and every file in `shaders/`.  The reviewer's 4 surviving mutants now fail: `>=` on buffers, `.take(1)` on entry points, `get_entry_point(0)`, and a new `.wgsl` file.  A message no longer computes `max - 1`, and the layout test's docs no longer claim that it reads `Kernels::new`.
* [ ] M2: review round 3.
* [ ] M2: PR opened (the maintainer asks for the push).
* [ ] M3: `STEMYLA_REQUIRE`.
* [ ] M4: `synth`'s `files` feature.
* [ ] M5: layering test.
* [ ] M6: dsp's clippy allow narrowed.
* [ ] M7: DSP-1 allowlist test.
* [ ] M8: BUILD-1 and ASSET-1 tests.
* [ ] M9: pins and `THIRD-PARTY.md` test.
* [ ] M10: e2e message types test.
* [ ] M11: RT-1 forbids allocation; allocation test; wsola 0.2.0.
* [ ] M12: MEM-1 lint.
* [ ] M13: MEM-3 lint.
* [ ] M14: Claude Code guards.
* [ ] M15: Claude Code feedback hooks.
* [ ] M16: test-list guard.
* [ ] M17: UI snapshots compared.
* [ ] M18: TD-13, TD-14, TD-12, TD-17 fixed.
* [x] 2026-10-19: browser tests are a required check (the maintainer, before PR #14; found by `gh api`).
* [ ] M19: phase 2 closed.

## Surprises and discoveries

* 2026-10-19: wsola 0.2.0 already removes the allocation in `pull`, so TD-15 needs no fork.  Evidence: `src/lib.rs` of the published 0.2.0 crate.
* 2026-10-19: MEM-3's `compile_fail` doctest of `Rec` has never run in CI, since `--exclude roformer-gpu` drops doctests as well.
* 2026-10-19: the shipped licence list (`THIRD-PARTY.md`) includes `hound`, `serde`, `serde_derive` and `serde_json`, pulled in only by `synth`'s file writers.
* 2026-10-19: the chipmunk path also allocates in the callback (`dsp::double_speed`), not only the stretcher.
* 2026-10-19, M1 review: `-p roformer-gpu` alone builds wgpu with fewer features than the workspace build, so the new CI steps rebuild wgpu, wgpu-core, wgpu-hal and naga.  On PR #14 it added 68 s to the `test` job.
* 2026-10-19: the docs said `browser tests (Chrome)` was not a required check, and the agent repeated it to the maintainer.  The ruleset had required it since 03:36 UTC that day.  Ruleset facts in docs go stale; check them with `gh api` before stating them.
* 2026-10-19, M1 review: `sources_are_complete` listed 12 placeholder names by hand and missed 8 that the templates use (`{BM}`, `{BN}`, `{BK}`, `{TY}`, `{SB_LEN}`, `{S_ZERO}`, `{SOFTMAX_1}`, `{SOFTMAX_2}`).  A test that lists what to look for goes stale; it now scans for the pattern.
* 2026-10-19, M2: deleting TD-1 made `docs-check` fail on each `TD-1` in the plans.  It checks every ID token in every Markdown file, plans included.  Every later milestone closes an ID, so it needed a rule for closed IDs.
* 2026-10-19, M2: WebGPU counts each `var<workgroup>` rounded up to 16 bytes, and only the ones the entry point statically uses ([compute pipeline creation](https://gpuweb.github.io/gpuweb/#dom-gpudevice-createcomputepipeline)).  The text count in `workgroup_memory_within_defaults` does neither; the tiles here are multiples of 16, so the counts agree.
* 2026-10-19, M1 review: stable rustdoc does not check a `compile_fail` doctest's error code, so MEM-3's doctest passes on any compile error (TD-22).  Evidence: a throwaway crate on rustc 1.99.0.

## Decision log

* 2026-10-19, maintainer: RT-1 forbids allocation in the audio callback after warm-up.
* 2026-10-19, maintainer: `synth`'s file writing (`hound`, `serde_json`) moves behind a feature, so the app links only song generation.
* 2026-10-19, agent: warm-up happens in `Player::new`, off the audio thread, so the test can require zero allocations from the first callback.  This is stricter than counting from after the first seconds of playback; a new song swapped in then never allocates in the callback.
* 2026-10-19, agent: the chipmunk decimation moves into `engine`, and `dsp::double_speed` stays as the beginner version, checked equal by a test.  The other way, a `&mut [f32]` function in `dsp`, would widen DSP-1's list of types.
* 2026-10-19, agent: hooks are an `xtask hook` command in Rust, not shell scripts, so their decisions have unit tests (VERIFY-1).  The cost is that a hook does not block when `xtask` fails to build; CI tests stay the controls.
* 2026-10-19, agent: the structural checks are tests in `xtask/tests/rules.rs`, with no new `xtask` command, so CI runs them without a workflow change.
* 2026-10-19, agent: the test-removal guard uses a PR label, set by a person, not text in the PR body, which the author controls.
* 2026-10-19, maintainer: a closed tech-debt ID moves to a `## Closed` bullet with what closed it.  `docs-check` allows it only in plans and decisions, and fails it if it is defined again.  The rejected options: exempt plans and decisions from the ID check (loses typo checks), or reword each reference (loses the history).
* 2026-10-19, agent (M2): the limits test counts buffers over the whole module, not per entry point.  `Kernels::new` gives each module one layout with every binding it declares.  The per-entry-point count is lower, so it can pass a layout that fails.
* 2026-10-19, agent (M2): a shader that does not parse or validate, or that declares an `override`, fails the limits test.  Otherwise it would pass without a check.
* 2026-10-19, agent: `cargo deny`, TD-16's field and version checks, and TD-10 (GPU-3) stay out of phase 2; the phase plan does not list them.

## Outcomes and retrospective

To be written when the plan completes.
