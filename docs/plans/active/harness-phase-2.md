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
* A new crate is vetted and pinned if young (DEPS-1).  This plan adds only these crates:
  * naga, already in the lock file (M2);
  * syn, already in the lock file (M7);
  * wsola 0.2.0 (M11);
  * `rusty_mp3`, as a dev-dependency of `xtask`, already in the lock file (M3);
  * `proc-macro2` 1.0.107 or later, as a dependency of `xtask`, already in the lock file and not pinned (M3);
  * the workspace crate `test-support` (M3).

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

### M3: a required test fails instead of skipping (TD-8, part; VERIFY-1, GPU-2, MEM-3, LAYER-1, DEPS-1; maintainer decision)

* Change: the environment variable `STEMYLA_REQUIRE` holds a comma-separated list of `gpu`, `timestamps` and `fixtures`.  Every test that skips because it has no adapter, no adapter feature or no fixture calls one helper.  The helper panics when that need is required, and names VERIFY-1.  An unknown word in the list also fails, so a typo cannot turn the check off.  Tests that can make their own input stop skipping.

#### M3 state at the start

On 2026-10-19, at commit `ad1b159` (M2 merged as PR #15):

* Stable libtest has no run-time skip.  A test that returns early reports `ok`, and `cargo test` hides a passing test's stderr ([pre-RFC: skippable tests](https://internals.rust-lang.org/t/pre-rfc-skippable-tests/14611)).  So each skip below looks like a pass.
* The skip sites in `cargo test` targets.  The first review found no other site in `tests/` folders, `#[cfg(test)]` modules or doctests.

  | File | Test | Skips when | M3 gives it |
  | - | - | - | - |
  | `roformer-gpu/tests/kernels.rs` | every test that calls `gpu()` | `Gpu::new` fails | `gpu` |
  | `roformer-gpu/tests/kernels.rs` | `profile_labels_dispatches`, `profile_recording_and_reset` | `Gpu::new_profiling` fails; the adapter has no `timestamp-query` | `gpu`; `timestamps` |
  | `roformer-gpu/tests/memory.rs` | every test that calls `gpu()` | `Gpu::new` fails | `gpu` |
  | `roformer-gpu/tests/memory.rs` | `collected_profile_readbacks_are_freed` | as the profile tests above | `gpu`; `timestamps` |
  | `roformer-gpu/tests/reference.rs` | `forward_matches_pytorch` | a fixture is absent (`read()`); `Gpu::new` or `Gpu::new_profiling` fails; with `ROFORMER_GPU_MM`, no `timestamp-query`, so the kernels that ran are not checked | `fixtures`; `gpu`; `timestamps` |
  | `roformer/tests/reference.rs` | `stft_matches_torch`, `demix_matches_reference`, `forward_matches_pytorch`, `f16_weights_stay_close`, `layer_in_parts_matches_layer` | a fixture is absent (`read()`) | `fixtures` |
  | `xtask/tests/m3_engine.rs` | `folder_loading_matches_the_generator` | `audio/synth/drums.wav` is absent | no skip: the test writes its own song |
  | `xtask/tests/m1_song.rs` | `mp3_decodes_to_roughly_the_right_length` | `audio/synth/mix.mp3` is absent (`gen-song` with ffmpeg) | no skip: the test encodes its own MP3 |

* The two `xtask` tests have never run in CI or on the maintainer's Mac.  `audio/synth/` is absent on both.  ffmpeg is not on the Mac, and not in the Ubuntu 24.04 runner image ([runner image](https://github.com/actions/runner-images/blob/main/images/ubuntu/Ubuntu2404-Readme.md)).
* A scratch crate in the first review ran both tests' logic as step 3 gives it, and both pass:
  * The folder test loads `drums`, `bass`, `guitar`, `keys`, `lead` with 1,323,000 frames.  `Song::from_audio` sorts by `PREFERRED_ORDER` (`engine/src/song.rs`), which is the generator's order.
  * `rusty_mp3` at 192 kbit/s, decoded by `engine::decode_bytes`, gives 1,323,648 frames.  That is 648 over 30 s, inside the test's bound of 4,000.  Half the bytes gives 661,248 frames, so the planted breach fails the test.  The encode takes about 0.3 s in release.
* The maintainer's Mac (AMD Radeon Pro 5500M, Metal) has an adapter without `timestamp-query`, and has no fixtures.  On macOS, wgpu's `vulkan` feature does not build the Vulkan backend, which needs `vulkan-portability`.  So `WGPU_BACKEND=vulkan` gives no adapter there ("vulkan support not compiled in").
* `Gpu::new` and `Gpu::new_profiling` return `Result<Gpu, String>`.  A failed adapter request and a failed device request both give `Err`.
* `rusty_mp3` 1.0.0 is in `Cargo.lock` through `stem-worker`, pinned with `=` and listed in `THIRD-PARTY.md`.  It is pure Rust with no dependencies, Apache-2.0 ([docs.rs](https://docs.rs/rusty_mp3/1.0.0/rusty_mp3/)).  `worker/src/web.rs` encodes with `Mp3Encoder` at 192 kbit/s.
* In edition 2024, `std::env::set_var` is `unsafe`, and it is unsound while other threads run ([edition guide](https://doc.rust-lang.org/edition-guide/rust-2024/newly-unsafe-functions.html)).  libtest runs tests on many threads, so the unit tests do not set the variable.
* Cargo stops after the first test binary that fails, unless `--no-fail-fast` is given ([cargo test](https://doc.rust-lang.org/cargo/commands/cargo-test.html)).
* Out of scope, with the reason:
  * `#[ignore]` tests (`roformer-gpu/tests/speed.rs`, `calibration.rs`) are benchmarks, and libtest reports them as ignored.
  * `ROFORMER_GPU_FULL` is an opt-in for the slow layers of `forward_matches_pytorch`.  Its message says what did not run.
  * The wasm tests in `app/tests/web_output.rs` skip only when the audio context stays suspended.  They assert that the browser is not Chrome, and CI runs Chrome.  A wasm test cannot read the variable at run time.
  * `xtask e2e storage` without `--weights` notes the cases that it does not run.  The person gives `--weights` on the command line.
  * The UI snapshot tests in `app/tests/ui.rs` pass without a comparison under `UPDATE_SNAPSHOTS=1` in CI.  That is TD-9, and M17 closes it.

#### M3 design

* Decision point for the maintainer: where the helper lives.  The plan first said "a small test-support module" in each crate.  Decided on 2026-10-19: the maintainer chose (a).
  * (a), recommended: a new workspace crate `test-support/` (package `test-support`).  It holds one parser and its tests, which run once.  The cost: a tenth crate, and a new first row in the layers table.  Also: a change to LAYER-1's text, the crate lists in three docs, and a special case in M5's layering check.
  * (b) one file, `test-support/skip.rs`, that each test target includes with `#[path]`.  It adds no crate, and changes no crate list or layer.  But its tests run again in each target that includes it, and a source folder that is not a crate is new to this repo.
  * (c) a copy in each crate, as first planned.  Two copies of the parser can drift, and no test sees it.
  * (d) a module behind a `testing` feature in `roformer`, which `roformer-gpu` already depends on.  It adds no crate and keeps one parser.  The costs: test code in a shipped crate's source, and a later user outside the model crates must depend on `roformer`.  Also, `roformer`'s own tests need its feature, through a dev-dependency on itself or `required-features`.  Under `cargo test --workspace` the feature then turns on in every build of `roformer`.
  * The steps below are written for (a).  For another option, revise the later M3 design bullets, every M3 step, the M3 acceptance and the decision log.  Also revise "Validation and acceptance", "What every milestone does", "Idempotence and recovery" and the `test-support` note in M5.  Then review the plan again before implementing.
  * After the maintainer chooses, step 1 writes the decision record.  The choice and the `STEMYLA_REQUIRE` interface outlive this plan.
* The crate uses only `std` and has no dependencies.  It is only ever a dev-dependency.  `roformer` and `roformer-gpu` take it.
* The API, in `test-support/src/lib.rs`:

  ```rust
  pub const VAR: &str = "STEMYLA_REQUIRE";

  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  pub enum Need { Gpu, Timestamps, Fixtures }

  /// The words of a list, or the message for the first unknown word.
  pub fn parse(list: &str) -> Result<Vec<Need>, String>;

  /// Ok: the line a skip prints.  Err: the message a required skip panics with.
  pub fn verdict(list: Option<&OsStr>, need: Need, why: &str) -> Result<String, String>;

  /// Reads `VAR` at run time, then prints the skip line or panics.
  #[track_caller]
  pub fn skip(need: Need, why: impl Display);
  ```

* `parse` splits the list on `,`, trims spaces and ignores empty items.  The words are lowercase only, and repeats are allowed.  An unset or empty variable requires nothing.  A value that is not Unicode fails.
* The variable is read with `std::env::var_os` when a test skips, never with `env!`.  So a change to it does not rebuild anything.
* `timestamps` implies `gpu`, because a test cannot use `timestamp-query` without an adapter.  Without this, `STEMYLA_REQUIRE=timestamps` and no adapter would pass the profile tests.
* Each `skip` names the one need that is missing.  So the GPU's PyTorch comparison is evidence only under `STEMYLA_REQUIRE=gpu,fixtures`.  `fixtures` alone covers only the CPU comparison in `roformer`.
* Any skip parses the whole list.  So a typo fails at the first skip, even when that skip's need is not in the list.  When nothing skips, a typo cannot hide anything.
* `skip` and the test helpers that call it are `#[track_caller]`: each `gpu()`, and the fixture readers `read()` in `roformer/tests/reference.rs` and `roformer-gpu/tests/reference.rs`.  So the panic names the line in the test.  `skip` panics in its own body, in a `match`, not in a closure: a closure does not get the caller's location (`closure_track_caller` is unstable).
* The messages:
  * Skip: `skipping (<word>): <why>.  Set STEMYLA_REQUIRE=<word> to make this a failure.`  `<word>` is `gpu`, `timestamps` or `fixtures`, and the helper adds no other prefix.  For example, `Gpu::new`'s error gives `skipping (gpu): no GPU adapter: <wgpu error>.  Set STEMYLA_REQUIRE=gpu to make this a failure.`
  * Required, one per need.  `<why>` is the text that the call site gives:
    * `VERIFY-1: STEMYLA_REQUIRE has gpu, but this test cannot run: <why>.  Fix: run it where a GPU adapter opens, or remove gpu from STEMYLA_REQUIRE.`
    * `VERIFY-1: STEMYLA_REQUIRE has timestamps, but this test cannot run: <why>.  Fix: run it on an adapter with timestamp-query, or remove timestamps from STEMYLA_REQUIRE.`
    * `VERIFY-1: STEMYLA_REQUIRE has fixtures, but this test cannot run: <why>.  Fix: write the fixtures with tools/roformer_export.py, or remove fixtures from STEMYLA_REQUIRE.`
    * When `timestamps` implies `gpu`, and the list does not have `gpu` itself: `VERIFY-1: STEMYLA_REQUIRE has timestamps, which needs gpu, but this test cannot run: <why>.  Fix: run it on an adapter with timestamp-query, or remove timestamps from STEMYLA_REQUIRE.`
  * Unknown word: `VERIFY-1: STEMYLA_REQUIRE has the unknown word "gpus".  Fix: use only gpu, timestamps and fixtures, separated by commas.`
  * Each message removes one full stop at the end of `<why>`.  The skip line does this too.  So a wgpu error such as "Not enough memory left." does not give two full stops.
  * Not Unicode: `VERIFY-1: STEMYLA_REQUIRE is not Unicode.  Fix: use only gpu, timestamps and fixtures, separated by commas.`
* `gpu` means that `Gpu::new` succeeds on any adapter.  A software adapter counts here.  GPU-3's real-GPU check stays with TD-10 (phase 4).
* `timestamps` is its own need.  Without it, `STEMYLA_REQUIRE=gpu` would fail on the maintainer's Mac, which gives GPU evidence without `timestamp-query`.

#### M3 steps

Each step is one signed commit, and fmt, clippy, the tests and `docs-check` pass before it (GIT-2).

1. The crate, test first.
   * Write the decision record as `docs/decisions/` number 0003, with `Status: proposed`.  List it in `docs/README.md`, or `docs-check` fails.  Also name it in the sentence on decisions at the top of `harness.md`.
   * Add `test-support` to the workspace `members`.  Commit the changed `Cargo.lock`, because CI builds with `--locked`.
   * In `harness-engineering.md`, change "Each new crate is pinned and listed (DEPS-1)" to "Each new crate is vetted, pinned if young, and listed if shipped (DEPS-1)".  `test-support` and `proc-macro2` make the old sentence false.
   * Its manifest takes `edition.workspace = true`, `license.workspace = true` and `[lints] workspace = true`, as `roformer` does.
   * Write the tests first, against stubs: `parse` returns `Ok(vec![])`, `verdict` returns `Ok(String::new())`, and `skip` does nothing.  See every test fail, except the child test, which returns at once without its marker.  Then write the three functions.
   * Each unit test asserts the exact message, or a `Vec<Need>` that is not empty, so a stub cannot pass it.
   * The unit tests call `parse` and `verdict` with strings, never `set_var`:
     * `a_required_need_that_cannot_run_fails`;
     * `a_need_not_required_skips_with_a_hint`;
     * `an_unknown_word_fails_even_when_not_needed`;
     * `items_are_trimmed_and_empty_items_ignored`, with words among the empty items;
     * `words_are_lowercase_only`;
     * `a_full_stop_at_the_end_of_why_is_not_doubled`;
     * `an_unset_or_empty_list_requires_nothing`, which asserts `verdict`'s exact skip line for `None` and for an empty value;
     * `a_list_that_is_not_unicode_fails` (`#[cfg(unix)]`, through `OsStrExt::from_bytes`);
     * `timestamps_requires_a_gpu`, which asserts that `STEMYLA_REQUIRE=timestamps` fails a `gpu` skip, with the message for an implied `gpu`.  With `gpu,timestamps` it asserts the `gpu` message.
   * The test of `skip` itself is in `test-support/tests/skip.rs`.  It runs its own test binary again as a child process.  So it sets the variable on the child's `Command`, never with `set_var`.
     * The child is the test `child_skips_without_a_gpu`.  It returns at once unless the marker `STEMYLA_TEST_CHILD=1` is set.  Then it calls `skip(Need::Gpu, "no adapter")`.  A marker, not `#[ignore]`, keeps a run with `--include-ignored` correct.
     * `a_required_skip_fails_and_names_the_caller` runs `current_exe()` with `--exact child_skips_without_a_gpu --nocapture`, the marker, and `STEMYLA_REQUIRE=gpu`.  It asserts that the child fails, and that its stderr holds VERIFY-1 and `tests/skip.rs:<line>:`.  The colon before the column keeps line 13 from matching line 130.
     * The parent finds `<line>` in `include_str!("skip.rs")`, not with `line!()` and an offset.  It is the one line whose trimmed text starts with `skip(Need::Gpu, "no adapter");`.  The parent asserts that exactly one line matches, because its own search text also holds the words.
     * `a_skip_that_is_not_required_passes` runs the child with the same arguments and the marker, and with the variable removed (`env_remove`).  `--nocapture` is needed, because libtest holds back a passing test's stderr.  It asserts that the child passes, and that its stderr holds `skipping (gpu)`.
   * Docs in the same commit, or `docs-check` fails: the `## Layout` lists in `CLAUDE.md` and `README.md`, and `## Crates` in `docs/ARCHITECTURE.md`.
   * The layers table gets a first row, "Test support | `test-support` | no workspace crate".  A sentence under it says that any crate but `dsp` (DSP-1) may take it as a dev-dependency.  No crate may take it as a normal dependency.
   * LAYER-1's text in `invariants.md` names the new layer and that exception (DOC-1).
2. The call sites in `roformer` and `roformer-gpu`.
   * Add `test-support` to the `[dev-dependencies]` of `roformer` and `roformer-gpu`.  Commit the changed `Cargo.lock`.
   * At the skip sites in the table, every `Err` from `Gpu::new` or `Gpu::new_profiling` calls `test_support::skip(Need::Gpu, &e)`.  That includes the helpers `gpu()` and the profile tests' own `match`.  Rewrite each `let Ok(g) = ... else` as a `match`, so `skip` gets the error.
   * `Gpu::new` has one `Err` for both failures, so under `gpu` both fail.  That is correct, because both mean that nothing ran.
   * Each `timestamp-query` check calls `skip(Need::Timestamps, ...)`.  In `forward_matches_pytorch`, the check moves to just after the device opens, when `ROFORMER_GPU_MM` is set and `gpu.profiling()` is false.  So a required `timestamps` fails before the network runs, not minutes later.  When `timestamps` is not required, the test continues with the PyTorch comparison.  Only the check of which kernels ran does not occur.  The `why` text: "adapter without timestamp-query: the kernels that ran are not checked".  Remove the note after the network.  Merge the two `if`s into `if mm.is_some() && gpu.profiling()`, or `clippy::collapsible_if` fails.
   * Each fixture reader `read()` calls `skip(Need::Fixtures, format!("{} not found (run tools/roformer_export.py)", p.display()))`.  This keeps today's hint in the skip line.
   * The module comments on skips also name `STEMYLA_REQUIRE`: `roformer-gpu/tests/kernels.rs`, `memory.rs` and `reference.rs`, and `roformer/tests/reference.rs`.
   * TD-8's text and VERIFY-1's text change in step 5.  Until then they are not complete, but they are not false, and the PR merges all five steps together.
3. The two `xtask` tests make their own input.
   * `folder_loading_matches_the_generator`: write `synth::generate()` with `synth::write_song(&song, &dir, false)` into a temporary folder.  The folder's name holds a fixed prefix and the process ID, as `m1_song.rs` does.  Remove the folder at the end.
   * `mp3_decodes_to_roughly_the_right_length`: encode the generated mix in memory with `rusty_mp3` at 192 kbit/s, with the calls that `worker/src/web.rs` makes.  Decode it with `engine::decode_bytes(bytes, Some("mp3"))`.
   * Add `rusty_mp3 = "=1.0.0"` to `xtask`'s `[dev-dependencies]`.  DEPS-1: it is the same pinned version, so no new crate is downloaded.  Commit the changed `Cargo.lock`.
   * TD-7 says that only `rusty_mp3` in `worker/` is pinned exactly.  That is already false (`naga` in `roformer-gpu/`, M2).  Change it to name the exact pins: `rusty_mp3` in `worker/` and `xtask/`, and `naga` in `roformer-gpu/`.
   * Keep each test's expectations, the 4,000-frame bound included.  If a test fails when it first runs, find the cause before an expectation changes.  State the cause in the PR.
   * Measure the time that the two tests add to the `test` job.  State it in the PR.  A prototype in the seventh review took 3.2 s for `m1_song` and 3.8 s for `m3_engine`, in a debug build.
4. The sensor: `xtask/src/rules.rs` and `xtask/tests/rules.rs`, which M5 then extends.
   * Declare `pub mod rules;` in `xtask/src/lib.rs`.
   * `rules::skips_use_the_helper(files: &[(String, String)]) -> Vec<Problem>` takes `(path, text)` pairs.  It is a pure function, as the checks in `docs.rs` are.
   * `rules::Problem` is new: `rule: &'static str`, `file: String`, `line: Option<usize>` and `message: String`.  `line` is `None` for a problem with the whole file.  `message` holds the full text below, and `Display` writes only `message`.  M5 to M10 use the same type.  `docs::Problem` stays as it is.
   * The input is every `.rs` file from `docs::repo_files`.  `skips_use_the_helper` itself leaves out the files in `test-support/` and `xtask/src/rules.rs`.  The checker's own allowlist and planted inputs hold the words that it looks for.
   * A problem is a string literal, plain or raw, whose text starts with `skipping` or `skipped`.
   * The checker tokenises each file with `proc_macro2::TokenStream::from_str`, and reads the literals in groups and macro calls too.  Comments, char literals such as `'"'`, raw strings and escapes then cannot confuse it.  proc-macro2 makes a doc comment into a `#[doc = "..."]` string literal.  So the check leaves out each literal whose `span().source_text()` starts with `//` or `/*`.  A scan of quote characters gets out of step on today's tree, at `'"'` in `serve.rs` and `web_test.rs`.
   * Add `proc-macro2 = { version = "1.0.107", features = ["span-locations"] }` to `xtask`'s `[dependencies]`, for the line numbers.  DEPS-1: 1.0.107 is in `Cargo.lock` through `syn`, and dtolnay maintains it.  It is not pinned with `=`, because it is a mature 1.x crate.  Cargo keeps one version per semver range, so a pin in `xtask` would hold back every proc macro in the workspace.  `xtask` does not ship, so `THIRD-PARTY.md` does not change.  Commit the changed `Cargo.lock`.
   * Cargo builds a second proc-macro2 for `xtask` with `span-locations`, and the proc macros keep theirs (`cargo tree`, in the third review's prototype).  Measure the build time that it adds to the `test` job.  State it in the PR.
   * A file that does not tokenise is a problem, so the check cannot pass without reading it.  The message: `VERIFY-1: <file> does not tokenise: <error>.  Fix: make the file valid Rust, or leave it out of the tree.`
   * An allowlist, of file and literal, has a reason for each entry.  Its one entry is `app/tests/web_output.rs`'s `"skipped: the audio context is suspended"`, because a wasm test cannot read the variable.
   * The message: `VERIFY-1: roformer/tests/x.rs:12 prints "<word>" itself.  Fix: call test_support::skip(Need::..., why), so STEMYLA_REQUIRE can make it a failure.  Outside test code, add the literal to the allowlist with a reason.`  `<word>` is the word that the literal starts with, `skipping` or `skipped`.
   * The test `every_skip_can_be_required` runs it on the real tree.  It asserts that the list is empty, and prints the count and the messages, as `xtask/tests/docs.rs` does.
   * The unit tests plant `eprintln!("skipping: no adapter")` in a `tests/` file and `"skipped: x"` in a `src/` file.  Each gives VERIFY-1 with the file and the line.
   * More unit tests:
     * a skip literal after `'"'` on the same line is found;
     * `r"skipping"` is found;
     * `r#"a "b"#`, `"\\"`, an allowlisted literal, and a skip phrase in a comment give no problem.  The comments include `// skipping`, `///skipping: x`, `//!skipping x` and `/**skipped */`.  Each doc-comment case has no space after the comment marker, so it fails when the exclusion is removed;
     * a file in `test-support/`, and `xtask/src/rules.rs`, give no problem;
     * a file that does not tokenise gives a problem.
5. The docs, and TD-8 narrowed.
   * `invariants.md`: VERIFY-1's text says that `STEMYLA_REQUIRE` turns a skip into a failure where evidence is claimed.  Its `* Enforced by:` line names `every_skip_can_be_required`, `a_required_need_that_cannot_run_fails` and `a_required_skip_fails_and_names_the_caller`.
   * `invariants.md`: GPU-2's rule text and GPU-2's and MEM-3's `* Enforced by:` lines give `STEMYLA_REQUIRE=gpu` for their GPU tests.
   * `tech-debt.md`: TD-8 narrows to what M3 leaves open, with a new title, the rules it concerns (VERIFY-1, GPU-2, MEM-3) and a fix for each part:
     * The PyTorch comparisons run nowhere, because neither CI nor the Mac has the fixtures.  Fix: the maintainer makes them on the Mac with `tools/roformer_export.py` and the PyTorch checkpoint.  Then `STEMYLA_REQUIRE=fixtures` is the evidence for the CPU, and `STEMYLA_REQUIRE=gpu,fixtures` for the GPU.
     * GPU and fixture evidence holds only when a person sets `STEMYLA_REQUIRE`, and nothing checks that a PR's evidence did.  Fix: phase 4's evaluator sets the variable in its contract.
     * The sensor sees only messages that start with `skipping` or `skipped`, so a silent early `return` passes it.  Fix: phase 3's reviewers look for it.
     * The three `timestamps` tests, and the kernel check in `forward_matches_pytorch`, run nowhere: CI excludes them, and the Mac's adapter has no `timestamp-query`.  Fix: run them on an adapter with `timestamp-query`, and name it in the PR.
   * `harness-engineering.md`: phase 3's bullets name the fix of the sensor's limit, for phase 3's reviewers.  Phase 4's bullets name the fix of the evidence, for the evaluator.  Phase 2's bullet on the structural test (`xtask/tests/rules.rs`) names the skip sensor.
   * TD-8 stays in the `Gaps` of VERIFY-1, GPU-2 and MEM-3.
   * Text about skips in `.claude/rules/gpu.md`, and the sentence on the reference tests in `docs/references/local-browser-testing.md`, give `STEMYLA_REQUIRE`.  That file's text on the wasm skip stays, because a wasm test cannot read the variable.
   * The commands for GPU evidence become `STEMYLA_REQUIRE=gpu cargo test -p roformer-gpu --release`.  Change them in `CLAUDE.md`, `.claude/rules/gpu.md`, the GPU recipe in `working-with-agents.md`, and `## Tests` in `docs/ARCHITECTURE.md`.  That last line also says that its PyTorch part needs `STEMYLA_REQUIRE=gpu,fixtures`.  `README.md`'s SwiftShader command stays, because a software adapter is not GPU evidence (GPU-3).
   * Text that says the reference tests "skip without them" adds that `STEMYLA_REQUIRE=fixtures` makes a missing fixture fail.  The text is in `README.md` (the table in `## Setting up a machine`, and `## Checks`) and in `## Tests` in `docs/ARCHITECTURE.md`.  `docs-check` cannot see this text (TD-21), so search these files for "skip".
   * `working-with-agents.md`: a line on how to ask for evidence with `STEMYLA_REQUIRE`, and a changelog entry.
   * `harness.md`: the helper and the sensor in the components table, with the sensor's limit.
   * CI sets no `STEMYLA_REQUIRE`.  Its `test` job has no fixtures (ASSET-1), and it excludes `roformer-gpu`'s adapter tests on purpose.

#### M3 acceptance

* `cargo test -p test-support`: the nine unit tests and the two tests of `skip` pass.  The child test passes at once without its marker.  Each failed against the stubs first.
* `cargo test -p xtask --test rules`: `every_skip_can_be_required` passes.  Its unit tests pass in `cargo test -p xtask --lib`.
* On the maintainer's Mac, record each result in the PR:
  * `STEMYLA_REQUIRE=gpu cargo test -p roformer-gpu --release -- --nocapture` passes, and the log names the adapter.
  * `STEMYLA_REQUIRE=gpu,timestamps cargo test -p roformer-gpu --release --no-fail-fast` fails exactly three tests, each with VERIFY-1: `profile_labels_dispatches`, `profile_recording_and_reset` and `collected_profile_readbacks_are_freed`.
  * `STEMYLA_REQUIRE=fixtures cargo test -p roformer --test reference` fails all five tests, each with VERIFY-1 and the missing path.  Without the variable they pass, and `-- --nocapture` shows `skipping (fixtures)`.
  * `WGPU_BACKEND=vulkan cargo test -p roformer-gpu --release --test memory -- --nocapture` prints `skipping (gpu)` for every test.  With `STEMYLA_REQUIRE=gpu` added, every test in `memory.rs` fails.
  * `STEMYLA_REQUIRE=gpus cargo test -p roformer --test reference` fails, and names `gpus`.
* Planted breaches in the two `xtask` tests, run locally and not kept.  Drop the second half of the MP3 bytes, and the MP3 test fails.  Delete `bass.wav` before loading, and the folder test fails.
* CI: the `test` job log lists `folder_loading_matches_the_generator ... ok` and `mp3_decodes_to_roughly_the_right_length ... ok`.
* fmt, clippy, `cargo test --workspace --exclude roformer-gpu`, `cargo test -p roformer-gpu --lib` and `--doc`, and `docs-check` pass.

### M4: the app links only song generation (TD-4, part; maintainer decision)

* Change: in `synth/Cargo.toml`, make `hound`, `serde`, `serde_json` and `anyhow` optional, behind a feature `files` that is not a default.  Put `write_wav` and `write_song` behind `#[cfg(feature = "files")]`.  Derive `Serialize` on `Onsets` and `Truth` with `cfg_attr(feature = "files", ...)`.  `xtask` takes `synth` with `features = ["files"]`.  `app` keeps `synth` without features, and adds it with `files` as a native dev-dependency for `app/tests/ui.rs`.
* Acceptance: `cargo tree -p app --target wasm32-unknown-unknown -e normal -i hound` finds nothing, and the same holds for `serde_json`.  `cargo test -p synth` and `cargo test -p synth --features files` pass.  `trunk build --release` passes.
* Docs: regenerate `THIRD-PARTY.md` with the command at its top; `hound`, and `serde_json` if nothing else needs it, leave the list.  Remove the TD-4 note under the layers table in `ARCHITECTURE.md`.  Narrow TD-4 to the missing test.

### M5: crate layering is a test (closes TD-4; LAYER-1)

* Change: add pure checker functions to `xtask/src/rules.rs`, and tests that run them on the real tree to `xtask/tests/rules.rs`, as `docs.rs` does.  M3 makes both files.  The first check reads `cargo metadata --format-version 1 --no-deps` and compares each workspace crate's workspace dependencies with an allowlist built from the layers table in `ARCHITECTURE.md`.  A second check runs `cargo tree -p app --target wasm32-unknown-unknown -e normal` and fails if it holds `hound` or `serde_json` (M4's result).
* Acceptance: the test `crates_depend_only_downwards` passes.  Unit tests plant an `engine -> app` edge and an `app -> roformer-gpu` edge; both fail with LAYER-1.  Adding `features = ["files"]` to `app`'s `synth` dependency fails the second check.
* Note: dev-dependencies count as edges unless the allowlist says otherwise; `app`'s dev-dependency on `synth` with `files` is listed.  Any crate but `dsp` (DSP-1) may take `test-support` (M3) as a dev-dependency, and none may take it as a normal dependency.  A unit test plants a `dsp -> test-support` dev-dependency edge, and it fails.

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

* Rule: RT-1 becomes "the audio callback never waits, never frees a player, and never allocates or frees memory after warm-up".  Update `invariants.md`, the RT-1 line in `CLAUDE.md` and `.claude/rules/memory.md`.  Write its record in `docs/decisions/` as `proposed`, with the next free number; the maintainer accepts it.
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
* Each check's planted breach fails it: the unit tests in `xtask/src/rules.rs`, `test-support`, `roformer-gpu` and `engine/tests/realtime.rs`, and the clippy cases stated in each PR.
* `cargo test --workspace --exclude roformer-gpu`, `cargo test -p roformer-gpu --lib` and `--doc`, clippy and `docs-check` pass; CI is green on `main` with browser tests required (the phase gate).

## Idempotence and recovery

* Each milestone is one PR and reverts on its own.  A reverted check removes its enforcement and nothing else; revert its `* Enforced by:` line and restore its tech-debt entry in the same commit.
* M3 adds a crate.  To undo M3, revert its commits, then put back decision 0003 with `Status: rejected`, and its line in `docs/README.md`.  So the decision numbers have no gap.  If M5 or a later milestone has merged, keep `xtask/src/rules.rs` and `xtask/tests/rules.rs`.  Remove only `skips_use_the_helper`, `every_skip_can_be_required` and M5's `test-support` case.  Remove `proc-macro2` from `xtask`'s `[dependencies]` unless a later check uses it.  Then commit `Cargo.lock`.
* Checks read the tree and write nothing, so they can run any number of times.  `xtask test-list` writes only to stdout.
* M11 changes a rule.  If the maintainer rejects M11's decision record, revert M11's rule text and keep the test as a measurement, marked in `tech-debt.md`.
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
* [x] 2026-10-19: M2, `every_kernel_fits_the_default_limits` passes on 8 sources: 4 tiles, attention, `softmax.wgsl`, `rmsnorm.wgsl` and `elementwise.wgsl`.  The planted shader (`@workgroup_size(1, 1, 128)`, 20,000 bytes) gives exactly two breaches.  Each names the limit, GPU-1 and the fix.
* [x] 2026-10-19: M2, planted breaches in the real tree fail the test: z = 128 in `softmax.wgsl`, 5,000 floats in `rmsnorm.wgsl`, and `pad: 66` on `i64`.  naga counts 16,640 bytes for `i64`, as M1's hand count does.
* [x] 2026-10-19: M2, TD-1 is the first closed ID.  `docs-check` now reads `## Closed` in `tech-debt.md` (maintainer's choice).  It found a stale `TD-1` in `docs/README.md`.
* [x] 2026-10-19: M2 review round 1: 3 findings, all real and fixed with tests.  (1) A `## Closed` heading in any doc hid its IDs from the check; now only the ID files' sections are left out.  (2) "Closed but defined again" named the wrong file.  (3) The docs claimed every limit.  The test now checks `maxBindGroups`, and `every_layout_matches_its_shaders` ties the layouts to the shaders.  Planted: an extra `MATMUL_BINDS` entry and `@group(4)` in `softmax.wgsl` fail.
* [x] 2026-10-19: M2 review round 2: 7 findings, all real and fixed.  The `## Closed` bullets' text is now checked; only the opening ID is left out.  New tests cover values at each limit, every entry point and every file in `shaders/`.  The reviewer's 4 surviving mutants now fail: `>=` on buffers, `.take(1)` on entry points, `get_entry_point(0)`, and a new `.wgsl` file.  A message no longer computes `max - 1`, and the layout test's docs no longer claim that it reads `Kernels::new`.
* [x] 2026-10-19: M2 review round 3: 4 findings, all real and fixed.  The limits test now pins its 8 source names, so dropping `.chain(SHADER_FILES ...)` fails it.  A `###` entry under `## Closed` gets its own message.  GPU-1 names `Rec::push`, and this plan counted 7 sources, not 8.
* [x] 2026-10-19: M2, PR #15 opened.  CI (run 39207559143) passes every job.  The `test` job lists the 26 `--lib` tests, among them `every_kernel_fits_the_default_limits ... ok`, and both `Rec` doctests.
* [x] 2026-10-19: M2, `cargo test -p roformer-gpu --release` on the maintainer's Mac (AMD Radeon Pro 5500M, Metal) passes.  12 of 14 `kernels.rs` tests and 4 of 5 `memory.rs` tests ran on the GPU.  Four skipped: three need `timestamp-query`, which this adapter lacks, and `forward_matches_pytorch` needs the absent weights (TD-8).
* [x] 2026-10-19: M2, PR #15 merged by the maintainer (`ad1b159`).
* [x] 2026-10-19: M3 implementation plan written (the M3 sections under "Plan of work").  Survey of every skip in the `cargo test` targets.
* [x] 2026-10-19: M3 plan reviewed in 12 rounds, each by two new reviewers (code, and docs and process).  Every finding was checked, and the real ones were fixed in the plan.
  * Rounds 1 to 4: 60 findings.  The stem-order risk was wrong.  TD-8 narrows instead of closing.  The sensor tokenises with `proc-macro2`, since a quote scan gets out of step at `'"'`.  `timestamps` implies `gpu`, since `timestamps` alone passed a run with no adapter.  `proc-macro2` takes a caret version.
  * Rounds 5 to 9: 43 findings, no blockers.  The messages, the skip line, `rules::Problem` and the revert steps are now defined.
  * Prototypes in rounds 3 to 9 built `test-support` and the sensor from the plan text.  They also applied steps 2 and 3 in a scratch worktree.  Every acceptance result for those steps held on the Mac.
  * Rounds 10 to 12: no blockers and no should-fix items.  The code reviewer of round 11 found nothing.  10 nits, all fixed.
* [x] 2026-10-19: M3 decision point: the maintainer chose option (a), a `test-support` crate.
* [x] 2026-10-19: M3 step 1: `test-support` written test first.  The 9 unit tests and the 2 tests of `skip` failed against the stubs, then passed.  They also pass with `STEMYLA_REQUIRE=gpu` or `gpus` in the outer shell, and with `--include-ignored`.
* [x] 2026-10-19: M3 step 2: every skip in `roformer` and `roformer-gpu` calls `skip`.  On the AMD Radeon Pro 5500M: `STEMYLA_REQUIRE=gpu` passes (16 "adapter:" lines).  `gpu,timestamps --no-fail-fast` fails exactly the three profile tests.  `fixtures` fails all 5 `roformer` reference tests, and `gpus` fails them and names `gpus`.  With `WGPU_BACKEND=vulkan`, all 5 `memory.rs` tests skip, and with `gpu` they all fail.  Each panic names the test's line.
* [x] 2026-10-19: M3 step 3: the folder and MP3 tests run.  Planted: half the MP3 bytes gives 661,248 frames and fails; a deleted `bass.wav` fails the stem names.
* [x] 2026-10-19: M3 step 4: `every_skip_can_be_required` passes, with 11 unit tests.  Planted: without the doc-comment exclusion, the `//!`, `///` and `/**` cases fail.  A planted `eprintln!("skipping: ...")` in `roformer/tests/reference.rs` fails the real-tree test with its file and line.
* [x] 2026-10-19: M3 step 5: docs, and TD-8 narrowed.
* [x] 2026-10-19: M3 review round 1: no blockers; 3 findings, all real and fixed.  (1) The sensor missed `"Skipping"`; it now ignores case, and `a_skip_word_in_any_case_is_a_problem` failed before the fix.  (2) README claimed `STEMYLA_REQUIRE` reaches `worker-check`.  (3) The folder test left its folder when it failed.
* [x] 2026-10-19: M3 review round 2: no blockers; 3 findings, all real and fixed.  (1) The real-tree test passed if the walk found no files, and nothing saw a stale allowlist entry.  It now asserts two known files, and `the_skip_allowlist_is_current` fails a reworded `web_output.rs` literal.  (2) VERIFY-1's text now names the wasm exception.  (3) The folder test removes its folder after a failed write too.
* [x] 2026-10-19: M3 review round 3: no blockers and no should-fix items; 3 nits, all fixed.  The SwiftShader command gets `STEMYLA_REQUIRE=gpu`, `ARCHITECTURE.md` names `ROFORMER_GPU_FULL=1` for the end-to-end check, and one comment is re-wrapped.
* [x] 2026-10-19: M3, PR #16 opened.  CI (run 39241946733) passes every job, and the `test` job lists the two revived tests, both checks of `xtask/tests/rules.rs` and both tests of `skip`.  The first attempt of the `test` job took 10m58s: `Cargo.lock` changed, so rust-cache restored an older cache and 167 crates were compiled again.  A re-run of the job took 4m41s (M2: 5m19s), with a 39 s build.  The revived tests add about 4 s.
* [ ] M3: PR #16 merged by the maintainer.
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
* 2026-10-19, M3 CI: a change to `Cargo.lock` makes rust-cache restore an older cache by its restore key.  The `test` job then took 10m58s, not about 5 minutes; a re-run took 4m41s.  So a first CI time after a lock change is not the cost of the change.
* 2026-10-19, M3 survey: two `xtask` tests have never run anywhere.  `folder_loading_matches_the_generator` and `mp3_decodes_to_roughly_the_right_length` need `audio/synth/`, which neither CI nor the maintainer's Mac has.  ffmpeg is on neither.
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
* 2026-10-19, agent (M3): the plan recommends one `test-support` crate for the helper, not a module in each crate as first planned.  Two copies of the parser could drift, and no test would see it.  The maintainer decides (decision point in M3's design).
* 2026-10-19, maintainer (M3): the helper lives in a new workspace crate `test-support`, option (a).  Step 1 records it as decision 0003.
* 2026-10-19, agent (M3): TD-8 narrows, and gets no new ID.  M3 gives the mechanism, but the fixtures still exist nowhere, and the evidence still rests on a person.
* 2026-10-19, agent (M3): the sensor tokenises with `proc-macro2` 1.0.107, which is already in the lock file.  A scan of quote characters gets out of step on char literals and raw strings.
* 2026-10-19, agent (M3): `timestamps` is a need of its own, beside `gpu` and `fixtures`.  The maintainer's Mac gives GPU evidence, but its adapter has no `timestamp-query`.
* 2026-10-19, agent (M3): `timestamps` implies `gpu`, and the other words imply nothing.  `fixtures` alone must stay usable on a machine with no GPU, for the CPU comparison.
* 2026-10-19, agent (M3): a test that can make its own input stops skipping, and gets no need.  So the song-folder and MP3 tests make their own input and need no ffmpeg.
* 2026-10-19, agent (M3): `proc-macro2` takes a caret version, not an `=` pin.  It is a mature 1.x crate, and a pin in `xtask` would hold back every proc macro in the workspace.  So the phase plan's DEPS-1 sentence becomes "vetted, pinned if young, and listed if shipped".
* 2026-10-19, agent (M3): `dsp` may not take `test-support`, even as a dev-dependency, because DSP-1 gives `dsp` zero dependencies.
* 2026-10-19, agent (M3 review round 3): the README's SwiftShader command gets `STEMYLA_REQUIRE=gpu`, against step 5's text.  `gpu` accepts a software adapter, so the variable makes no GPU-3 claim.  It only makes a wrong driver path fail instead of skipping.
* 2026-10-19, agent (M3): `gpu` accepts a software adapter.  GPU-3's real-GPU check stays with TD-10, in phase 4.
* 2026-10-19, agent: `cargo deny`, TD-16's field and version checks, and TD-10 (GPU-3) stay out of phase 2; the phase plan does not list them.

## Outcomes and retrospective

To be written when the plan completes.
