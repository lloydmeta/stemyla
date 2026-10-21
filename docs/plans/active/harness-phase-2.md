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
* Before each commit: fmt, clippy (every clippy command in `CLAUDE.md`, from M4 on), `cargo test --workspace --exclude roformer-gpu`, `cargo test -p roformer-gpu --lib` and `--doc`, and `docs-check` pass (GIT-2).  Each commit is signed.
* After implementing: three review rounds, each by a new subagent with fresh context.  Verify each finding, then fix real ones with tests.
* The PR description says what was verified, how, and what was not.  It is written to a file and passed with `--body-file`.
* A new crate is vetted and pinned if young (DEPS-1).  This plan adds only these crates:
  * naga, already in the lock file (M2);
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

### M4: the web app takes only song generation from synth (TD-4, part; LAYER-1, DEPS-1, DOC-1; maintainer decision, made 2026-10-19)

* Change: `synth`'s file writers and their four crates move behind a feature `files`, which is not a default.  `app` takes `synth` without it.  Only tools and tests take it.  A new check fails when `synth` brings another crate into the web app.
* Term: "the web app" is the two WebAssembly modules, `app` and `stem-worker`, as in `THIRD-PARTY.md`.  "The page" means `app` in `invariants.md` and in the layers section of `ARCHITECTURE.md`, so M4 does not use it.  The text that M4 adds says "the web app (`app` and `stem-worker`)" at the first use in the body text of each doc.  These docs: `invariants.md`, `ARCHITECTURE.md`, `harness.md`, `docs/README.md`, decision 0004 and `harness-engineering.md`.
* The gain is a correct licence list and a smaller dependency graph for the web app.  The gain is not a smaller download: the linker already removes the unused writers (see the sizes below).

#### M4 state at the start

On 2026-10-19, at commit `69f8cae` (M3 merged as PR #16):

* `synth/Cargo.toml` has four dependencies and no features: `anyhow` 1, `hound` 3.5.1, `serde` 1 with `derive`, and `serde_json` 1.
* In `synth/src/lib.rs`, only `write_wav` and `write_song` use `hound`, `serde_json` and `anyhow`.  `Onsets` and `Truth` derive `Serialize` only for the `truth.json` that `write_song` writes.  The rest (`generate`, `sum_stems`, the constants and the types) uses only `std`.  `synth` has no tests.
* The callers:
  * `app/src/lib.rs` calls only `synth::generate()` and `synth::SAMPLE_RATE`, for the demo song.
  * `app/tests/ui.rs`: `dropping_several_files_loads_them_as_stems` calls `write_song`.  No wasm test in `app` uses `synth`.
  * `xtask`: `gen-song` in `src/main.rs`, `src/e2e.rs`, and the tests `m1_song.rs` and `m3_engine.rs` call `write_song`.  `src/bench.rs` uses the type `Onsets`.
* The web app's tree: `cargo tree -p app -p stem-worker -e normal --target wasm32-unknown-unknown --prefix depth -f "{p}"` gives `synth` four children: `anyhow`, `hound`, `serde` and `serde_json`.
* Only `hound` leaves the web app.  `serde`, `serde_json` and the crates under them also come through `roformer` and `safetensors` in `stem-worker`.  `app` and `engine` use `anyhow` directly.
* Trunk builds `app` and `stem-worker` in two separate Cargo runs (two `data-trunk rel="rust"` links in `app/index.html`).  So their features are not merged in what ships.
* `.github/workflows/ci.yml` sets `CARGO_TERM_COLOR: always` for every job.  Its `test` and `clippy` jobs build `synth` and `app` with `--workspace` only.
* Facts from two prototypes of the change, in scratch worktrees.  Their manifests and `cfg` attributes are those in "M4 design":
  * `Cargo.lock` does not change.
  * `THIRD-PARTY.md`'s first command, run before and after, differs in one row: `hound`.  This holds after the output is made into a set of rows: `sed -E 's/ \(\*\)$//; s/ \(\/[^)]*\)//' | sort -u`.  The raw output differs in more lines, because the full `serde` node moves from `app`'s tree to `stem-worker`'s.
  * `trunk build --release`: `app_bg.wasm` is 5,194,829 bytes before and 5,194,587 after, 242 bytes less.  `stem-worker_bg.wasm` does not change (939,660 bytes).
  * `cargo tree -e normal` does not apply a dev-dependency's features.  With `app`'s new dev-dependency on `synth` with `files`, `-e normal` shows `synth` with no features and no children.  `-e all --target all` shows `files` and `hound`; with `--target wasm32-unknown-unknown` it does not, because the dev-dependency is native-only.  The Cargo book says that `cargo tree -e normal,build` is "pretty close" to `cargo build` ([cargo tree](https://doc.rust-lang.org/cargo/commands/cargo-tree.html)).
  * After the change, `cargo tree -p app -e normal --target wasm32-unknown-unknown -i hound` prints "warning: nothing to print" and exits 0.  An empty result with exit 0 passes in silence, so the check does not use `-i`.
  * Before the change, the web app's `synth` also has no features, because `synth` has none.  So a check of the feature list alone passes on today's tree, and cannot be written test first.  A check of `synth`'s children fails today.
  * With `--target all`, `-e normal` also shows `serde` on the native side, through `accesskit_atspi_common` (eframe on Linux).  So a check that bans crates by name gives false alarms.  The check reads `synth`'s children instead.
  * With `CARGO_TERM_COLOR=always`, `cargo tree` writes ANSI colour codes around each `(*)` marker, also into a pipe.  The depths and names stay plain.  With `--color never`, there are no codes.
  * A `(*)` line has no children: the Cargo book says that its dependencies "have already been shown elsewhere in the graph, and so are not repeated".
  * With two `-p` roots in one run, Cargo merges their features and prints each package in full only once, under the first root.  A prototype gave `stem-worker` a `synth` with `files`.  The full `synth` node, with `hound`, was then under `app`, and `stem-worker` showed `1synth ... (*)`.  So the check runs `cargo tree` once for each root.
  * The deepest line in today's tree is at depth 9.  One more level gives two-digit depths.
  * `cargo tree` runs from inside a `cargo test` binary without lock contention.  It works for `wasm32-unknown-unknown` when that target is not installed.  It takes about 1 s on the maintainer's Mac.
  * `cargo tree` downloads the packages that it shows.  The `test` job builds for the host only, so a first run without a cache may download the wasm-only crates.  rust-cache then keeps them.  This is not measured.  A machine that is offline and has never fetched them gets an `Err`, not a pass.
  * Without `app`'s new dev-dependency, `cargo test -p app --test ui` fails with `E0425: cannot find function write_song`.  `cargo test --workspace --exclude roformer-gpu --no-run` still builds, because `xtask` turns `files` on for the whole build.
  * After a workspace clippy, the first `-p app` clippy run checks about 90 dependencies again with other features, among them wgpu, naga, egui and eframe.  M1 saw the same effect (68 s).  Measured: `cargo clippy -p app` took 92 s, and then `-p app --all-targets` took 8 s.  Alone, `-p app --all-targets` took 55 s.  `-p synth` checks only `synth` again.
  * A `use serde::Serialize;` without its `cfg` gives `E0432: unresolved import serde` without `files`.
* Cargo facts:
  * An optional dependency named with `dep:` in `[features]` gets no implicit feature of its own ([features](https://doc.rust-lang.org/cargo/reference/features.html)).
  * Resolver 2: "Dev-dependencies do not activate features unless building a Cargo target that needs them (like tests or examples)" (same page).  Resolver 3, which this workspace uses, changes only the default of `incompatible-rust-versions` ([resolver](https://doc.rust-lang.org/cargo/reference/resolver.html)).
  * "When a dependency is used by multiple packages, Cargo will use the union of all features enabled on that dependency" ([features](https://doc.rust-lang.org/cargo/reference/features.html)).  So `--workspace` builds compile `synth` with `files`, and hide a missing `files` in one crate.  Only `-p` builds of a crate that does not take `files`, and the `wasm build (trunk)` job, compile `synth` without `files`.  Trunk does not fail on warnings, and it builds no native code.
  * `cargo clippy -p app --all-targets` builds `app`'s tests, so the dev-dependency's `files` also reaches `app`'s lib.  A prototype put a native-only call to `synth::write_song` in `app/src/lib.rs`.  `-p app --all-targets` passed, and `cargo clippy -p app -- -D warnings` failed with E0425.  So `app`'s native lib needs its own step.
  * `xtask` runs Cargo as `env!("CARGO")`, as `xtask/src/web_test.rs` does.

#### M4 design

* The rule is LAYER-1.  Its text gains one sentence at the end of the paragraph: "In the normal dependency tree of the web app (`app` and `stem-worker`), `synth` brings no other crate."  This is what the check reads: `-f "{p}"` prints no features.  Dev-dependencies are not in that tree, so `app`'s dev-dependency with `files` keeps the rule.  Its "Why" gains: "It also keeps the crates that `synth` needs only for tools and tests out of the web app's build, so `THIRD-PARTY.md`'s first table does not credit them."
* `CLAUDE.md`'s LAYER-1 line stays as it is: it is a summary, and stays true.
* A decision record, `docs/decisions/0004-synth-file-writers-behind-a-feature.md`.  The `files` boundary and LAYER-1's new text outlive this plan.  The maintainer made the choice on 2026-10-19, and then accepts the record after review.  M11's record takes the next free number.
  * Title: `# 0004: synth's file writers are behind a feature, so the web app takes only song generation`.
  * Status line: `Status: proposed`.
  * The line in `docs/README.md`: `` * `decisions/0004-synth-file-writers-behind-a-feature.md`: `synth`'s file writers are behind its `files` feature, so the web app (`app` and `stem-worker`) takes only song generation. ``
  * It has the four sections of 0001 to 0003:
    * Context: "`app`, one of the two WebAssembly modules of the web app (`app` and `stem-worker`), takes `synth` for the demo song.  So it compiles `hound` for writers that it never calls, and `THIRD-PARTY.md` credits `hound`.  `app_bg.wasm` is only 242 bytes smaller without them, because the linker already drops the unused writers."
    * Options:
      * (a) A `files` feature in `synth`, not a default.  It adds no crate.  The cost: `--workspace` builds merge the feature, so CI needs `-p` steps.
      * (b) A separate crate for the writers.  It is an eleventh crate, with a row in the layers table and the crate lists.
      * (c) The writers move into `xtask`.  `app`'s test `dropping_several_files_loads_them_as_stems` then writes its WAV files with `engine::wav::encode_pcm16`, as 16-bit files, not `synth`'s 32-bit float.  So `synth` is no longer the one source of the song's files.
    * Decision: (a), with `app`'s dev-dependency on `synth` with `files` for its native tests only.
    * Consequences, one sentence each:
      * "LAYER-1 gains one sentence: in the web app's normal dependency tree, `synth` brings no other crate."
      * "`synth_brings_no_crate_into_the_web_app` checks the web app's trees."
      * "Three CI `clippy` steps check `synth`, `app`'s lib and `app`'s tests without the workspace's merged features."
  * `harness.md`'s sentence on decisions does not name 0004.  0004 is about the product, not the harness.
* `synth/Cargo.toml`:

  ```toml
  [dependencies]
  anyhow = { version = "1", optional = true }
  hound = { version = "3.5.1", optional = true }
  serde = { version = "1", features = ["derive"], optional = true }
  serde_json = { version = "1", optional = true }

  [features]
  # The WAV and truth.json writers, for tools and tests.  The web app does not take it (LAYER-1).
  files = ["dep:anyhow", "dep:hound", "dep:serde", "dep:serde_json"]
  ```

  The versions do not change, so no crate is new (DEPS-1).
* `synth/src/lib.rs`: `write_wav` and `write_song` get `#[cfg(feature = "files")]`.  `Onsets` and `Truth` get `#[cfg_attr(feature = "files", derive(serde::Serialize))]` above their other derives.  The `use serde::Serialize;` line goes.  The module comment names the feature.
* `xtask/Cargo.toml`: `synth` takes `features = ["files"]`.
* `app/Cargo.toml`: `[dependencies]` keeps `synth` without features.  Add `synth = { path = "../synth", features = ["files"] }` to the existing `[target.'cfg(not(target_arch = "wasm32"))'.dev-dependencies]` section.  Its comment names the test `dropping_several_files_loads_them_as_stems`.  It is native-only, because only the native UI test writes files.  So `hound` also stays out of `THIRD-PARTY.md`'s `## Browser tests only` table.
* CI: the `clippy` job gets three steps after the workspace step.  Each has a comment that says why:
  * `cargo clippy --locked -p synth --all-targets -- -D warnings` checks `synth` without `files`.  It costs a few seconds.
  * `cargo clippy --locked -p app -- -D warnings` checks `app`'s lib and binary without any `files`.  So a native-only call to a writer in `app` fails CI.
  * `cargo clippy --locked -p app --all-targets -- -D warnings` checks that `app`'s tests build without the workspace's merged features.  So a lost dev-dependency fails CI.
  * The two `-p app` steps together check about 90 dependencies again: about 100 s measured, almost all in the first step.  The PR states the time in CI.  The `clippy` job runs beside the longer `test` job.
  * The other direction needs no step: if `xtask` loses `files`, `cargo run -p xtask` in the `browser-tests` job does not compile.
* Commands: three lists get the three clippy commands, so GIT-2's local clippy run covers them.  Like the lines beside them, they have no `--locked`.  Each list keeps its own style:
  * `README.md`, the list of the checks that CI gates on, which has no comments.  Add these three lines after the workspace clippy line:

    ```bash
    cargo clippy -p synth --all-targets -- -D warnings
    cargo clippy -p app -- -D warnings
    cargo clippy -p app --all-targets -- -D warnings
    ```

  * `README.md`'s `## Checks`, whose clippy line has no `-D warnings`.  Add these three lines after it, with the comment column of that block:

    ```bash
    cargo clippy -p synth --all-targets          # synth without its files feature
    cargo clippy -p app                          # app's lib and binary without any files feature
    cargo clippy -p app --all-targets            # app's tests without the workspace's merged features
    ```

  * `CLAUDE.md`'s `## Commands`.  Add these three lines after the workspace clippy line.  The second line's comment uses the comment column.  The other two commands reach that column, so their comments start two spaces after them, as on the `STEMYLA_REQUIRE` line:

    ```bash
    cargo clippy -p synth --all-targets -- -D warnings  # synth without its files feature
    cargo clippy -p app -- -D warnings              # app's lib and binary without any files feature
    cargo clippy -p app --all-targets -- -D warnings  # app's tests without the workspace's merged features
    ```
* The check, in `xtask/src/rules.rs`:
  * `web_app_tree(root: &Path, package: &str) -> Result<String>` runs `env!("CARGO") tree --locked --color never -p <package> -e normal --target wasm32-unknown-unknown --prefix depth -f "{p}"` in `root`.  A failed command gives an `Err` with Cargo's stderr.
  * One run for each package, `app` and `stem-worker`, because one run with both merges their features (see the facts above).  Trunk builds them apart too.
  * `--color never`, because CI's `CARGO_TERM_COLOR: always` reaches the test binary.  So the check reads the same text in CI and on the Mac.
  * The target is `wasm32-unknown-unknown`, because the web app is what ships, and `THIRD-PARTY.md` reads the same target and edges (DEPS-1).  So a native-only breach passes; the native app does not ship.
  * `synth_brings_no_crate(trees: &[(&str, &str)]) -> Vec<Problem>` is a pure function over `(package, tree)` pairs.  It reads each line as a depth (all the leading digits) and a package name (the first word after them).  It skips empty lines.
  * For each `synth` line, at any depth, its children are the following lines at its depth plus one.  The children end at the first line at `synth`'s depth or less.  A `(*)` line gets no special case: Cargo prints no children under it.
  * The name must equal `synth`, so `synthx` does not match.
  * The problems, each with `rule: "LAYER-1"` and `line: None`.  `file` is the manifest of the package whose tree has the problem: `app/Cargo.toml` or `worker/Cargo.toml`.  The "no `synth`" problem takes `app/Cargo.toml`:
    * `synth` has children: `LAYER-1: synth brings anyhow, hound, serde, serde_json into the web app, through app.  Fix: make each dependency of synth optional, behind its files feature.  Take synth without its files feature in the web app.  Tools and tests take features = ["files"].`  The names are the children, in the tree's order.  `app` is the package whose tree has the line.
    * No `synth` line in any tree: `LAYER-1: the web app's trees have no synth, so this check read nothing.  Fix: if the web app no longer takes synth, remove synth_brings_no_crate_into_the_web_app.  Update the docs that name it (DOC-1).`
    * A line that is not empty and has no depth: `LAYER-1: the tree of app has a line without a depth: "<line>".  Fix: run cargo tree with --prefix depth.`
    * A tree whose first line is not the package at depth 0, an empty tree included: `LAYER-1: the tree of app does not start with app, so this check cannot read it.  Fix: run cargo tree -p app with --prefix depth.`  Without it, an empty output with exit 0 would pass in silence.
    * A tree that starts with its package but has no `wasm-bindgen` at depth 1 (added in review round 3): `LAYER-1: the tree of stem-worker has no wasm-bindgen at depth 1, so it is not the wasm tree.  Fix: run cargo tree -p stem-worker with --target wasm32-unknown-unknown.`  Each module of the web app takes `wasm-bindgen` directly, and only on wasm32.  Natively, `stem-worker`'s tree is its root line alone, which passes every other check.
  * The test `synth_brings_no_crate_into_the_web_app` in `xtask/tests/rules.rs` runs `web_app_tree` for both packages and then the check.  It asserts that the list is empty, and prints the messages, as `every_skip_can_be_required` does.
  * `rules.rs`'s module comment says that this check reads Cargo's output, not the files.
* The phase plan put this check in M5.  It moves to M4, so M4's result has an automated check (VERIFY-1).  M5 keeps the crate edges over `cargo metadata`.
* LAYER-1's `* Enforced by:` line becomes: "`synth_brings_no_crate_into_the_web_app` (CI `test` job).  No test checks the crate edges (TD-4)."  `docs-check` confirms that the test exists.  The three clippy steps are not on this line, because they check the build, not the rule.

#### M4 steps

Each step is one signed commit, and fmt, clippy, the tests and `docs-check` pass before it (GIT-2).  Fixes from the review rounds are later commits.

1. The checker, test first.
   * Write `web_app_tree`, and `synth_brings_no_crate` as a stub that returns no problems.  Write the unit tests in `rules.rs`'s test module.  Each asserts the whole `Vec<Problem>`, not that it contains a problem:
     * `synth_with_children_is_a_problem`: lines from today's `app` tree.  They keep `serde`'s own children (`3serde_core`, `3serde_derive`) between the children of `synth`, and a grandchild (`3itoa`) under `serde_json`.  It asserts the exact message, with only the four children and `app`.  So a parser that stops at the first deeper line fails it.
     * `synth_without_children_is_no_problem`.
     * `a_line_at_synths_depth_is_not_its_child`: exactly `0app`, `1synth`, `1engine`, `2hound`.  It asserts no problem.  So a parser that stops only at a shallower line fails it.
     * `a_shallower_line_ends_synths_children`: `synth` at depth 2, then a depth-1 line, then a depth-3 line.  It asserts no problem.  So a parser that stops only at the same depth fails it.
     * `a_deeper_synth_is_read`: a `9engine` line, then `10synth`, then `11hound`.  It gives the problem.  So a parser that reads only `1synth`, or only the first digit of a depth, fails it.
     * `a_repeated_synth_gives_one_problem`: `synth` with a child, then a `synth ... (*)` line.
     * `a_tree_without_synth_is_a_problem`: two trees, neither with `synth`, one with a `synthx` line that has a child.  So a parser that matches `synth` by prefix, when it finds `synth` or its children, fails it.
     * `synth_under_stem_worker_is_named`: an `app` tree with a `1synth` line without children, as the real tree has.  Then a `stem-worker` tree with the lines that the planted `stem-worker` breach gives: `1synth`, `2anyhow`, `2hound`, `2serde v1.0.229 (*)` and `2serde_json v1.0.152 (*)`.  It asserts exactly one problem, which names `stem-worker` and `anyhow, hound, serde, serde_json`.  So a parser that reads only the first `synth`, or that skips `(*)` lines, fails it.
     * `a_line_without_a_depth_is_a_problem`: a `stem-worker` tree that starts with `0stem-worker`, and holds a `synth` without children, an empty line and a line without a depth.  It asserts exactly one problem, which names `stem-worker`.
     * `a_tree_that_does_not_start_with_its_package_is_a_problem`: an empty `app` tree, and a `stem-worker` tree that starts with `0app` and holds a `1synth` line without children.  It asserts exactly two problems, one for each tree.
     * `a_first_line_below_depth_0_is_a_problem` (added in review round 1): a tree that starts with `1app`, with `0app` on a later line.  So a check of the name alone, or of any line, fails it.
     * `a_tree_without_wasm_bindgen_is_a_problem` (added in review round 3): a `stem-worker` tree with `wasm-bindgen` only at depth 2.  So a tree read without `--target wasm32-unknown-unknown` fails the check.
   * Each fixture tree starts with `0<package>` and has `1wasm-bindgen` right after it, unless the test is about one of those lines.  `synth_without_children_is_no_problem` has both trees, as the real tree has: `synth` in `app` only.
   * Fixture lines copied from real output replace each local path with `(/x/<crate>)`, so no home folder is committed.
   * See every test that expects a problem fail against the stub.  The tests that expect no problem pass against it; that is expected.  Then write the function.
   * Update `rules.rs`'s module comment.
2. The change, its sensor and its docs.  The real-tree test fails until the change, and the change makes docs wrong (DOC-1).  So these are one commit.
   * Write `synth_brings_no_crate_into_the_web_app`.  Run it on today's tree.  It fails and names `anyhow, hound, serde, serde_json`.  Record the output in the PR.
   * Make the change, as "M4 design" gives it: `synth/Cargo.toml`, `synth/src/lib.rs`, `xtask/Cargo.toml`, `app/Cargo.toml` and the three CI steps.  The real-tree test now passes.
   * Add the three clippy commands to the three lists in "M4 design".
   * `THIRD-PARTY.md`:
     * Run the first command at its top.  Compare its output with the first table.
     * Remove the `hound` row.  No other row changes.
     * Run the command of `## Browser tests only`.  Confirm that `hound` is not in it, because `app`'s new dev-dependency is native-only.
     * No check compares the tables until M9 (TD-7).
   * `docs/decisions/0004-synth-file-writers-behind-a-feature.md`, with the title, status and content in "M4 design".  Add its line to `docs/README.md`, or `docs-check` fails.
   * `docs/ARCHITECTURE.md`:
     * Under the layers table, replace "`app` depending on `synth` ships the demo song generator in the page (TD-4 in `docs/tech-debt.md`)."  The new text: "`app` takes `synth` as a normal dependency without its `files` feature, so `synth` brings no other crate into the web app (`app` and `stem-worker`; LAYER-1)."
     * The `synth` bullet in `## Crates` becomes: "`synth`: the built-in demo song, synthesised so its stems are known exactly.  Its `files` feature adds writers for the song's WAV files and `truth.json`, for tools and tests."
   * `invariants.md`: LAYER-1's text, "Why" and `* Enforced by:` line, as in "M4 design".  `* Gaps: TD-4.` stays.
   * `tech-debt.md`, TD-4:
     * The title becomes "LAYER-1's crate edges are not checked".
     * Remove "`app` depends on `synth`, so the demo song generator and its dependencies (hound, serde_json) compile into the page build."
     * The fix line becomes: "* Fix: a test over `cargo metadata` with an allowlist of edges per crate (`plans/active/harness-phase-2.md`, M5)."  So the sentence "Decided 2026-10-19: ..." goes.
   * `harness.md`, two components rows:
     * `synth_brings_no_crate_into_the_web_app` (LAYER-1), kind "sensor, computational", added "phase 2".  The failure it addresses: `app`, in the web app (`app` and `stem-worker`), compiled `hound` for `synth`'s file writers, and the licence list credited it.
     * Component cell: "the three `-p` clippy steps in CI".  Kind "sensor, computational", added "phase 2".  Failure cell: "`--workspace` builds merge features, so they hide warnings in `synth` without `files`, a native-only call to a writer in `app`, and a lost dev-dependency".
   * `harness-engineering.md`: in phase 2's bullet on the structural test, "crate layering over `cargo metadata` (TD-4)" becomes "crate layering over `cargo metadata` (TD-4), `synth` in the web app (`app` and `stem-worker`) over `cargo tree` (LAYER-1, M4)".
   * No change to `working-with-agents.md`, because the work of a person does not change.  The map's `synth/` and LAYER-1 lines stay true.

#### M4 acceptance

* `cargo test -p xtask --test rules`: `synth_brings_no_crate_into_the_web_app` passes.  Before the change it failed and named the four crates.  The 12 unit tests pass in `cargo test -p xtask --lib`, and each that expects a problem failed against the stub.
* `THIRD-PARTY.md`: make the first command's output into a set of rows, as "M4 state at the start" gives it.  Apart from the workspace crates, that set equals the first table's rows.  The set before the change differs from the set after only in `hound`.  Record that diff in the PR.
* Planted breaches, run locally and not kept.  Record each result in the PR:
  * `features = ["files"]` on `synth` in `app`'s `[dependencies]`: the real-tree test fails and names `hound`.
  * `default = ["files"]` in `synth/Cargo.toml`: the real-tree test fails.
  * Without `app`'s new dev-dependency, `cargo clippy -p app --all-targets -- -D warnings` fails with E0425 on `write_song`.
  * `use serde::Serialize;` left in `synth/src/lib.rs` without its `cfg`: `cargo clippy -p synth --all-targets -- -D warnings` fails with E0432.
  * Put the plant above the test module, because below it clippy fails first on `items_after_test_module`.  A native-only call to `synth::write_song` in `app/src/lib.rs`: `cargo clippy -p app -- -D warnings` fails with E0425, and `-p app --all-targets` passes.
  * A `synth` dependency with `features = ["files"]`, added to `[target.'cfg(target_arch = "wasm32")'.dependencies]` in `worker/Cargo.toml`: the real-tree test fails and names `stem-worker`.  This edge changes `Cargo.lock`, so the inner `cargo tree --locked` fails until the lock is updated.  Run the outer `cargo test -p xtask --test rules` without `--locked`, so it updates the lock first.  Afterwards, restore `Cargo.lock` with the manifest.
* Each crate builds without the workspace's merged features:
  * `cargo clippy -p synth --all-targets -- -D warnings` passes, and so does the same with `--features files`.
  * `cargo clippy -p app -- -D warnings` passes, and so does the same with `--all-targets`.
  * `cargo test -p app --test ui` passes, and `dropping_several_files_loads_them_as_stems` runs.
  * `cargo test -p xtask --tests` passes.
* `(cd app && trunk build --release)` passes.  The PR states the size of `app_bg.wasm` before and after.
* `grep -c "Enforced by: prose only" docs/invariants.md` gives 7 (8 before).
* fmt, clippy, `cargo test --workspace --exclude roformer-gpu`, `cargo test -p roformer-gpu --lib` and `--doc`, and `docs-check` pass.
* CI: the `test` job log lists `synth_brings_no_crate_into_the_web_app ... ok`, and the `clippy` job runs the three new steps.  The PR states the time that each adds.

### M5: crate layering is a test (closes TD-4; LAYER-1, DOC-1; maintainer decision)

* Change: a check compares the edges between workspace crates, from `cargo metadata`, with the layers table in `docs/ARCHITECTURE.md`.  The table gets one row per crate, so a person and the check read it the same way.  A second check fails when the page side and the worker side share a workspace crate other than `stem-protocol`.
* Terms:
  * An edge is a dependency of one workspace crate on another, of any kind (normal, dev or build) and for any target.
  * A stray is a crate that the build takes from outside the members, but that can hold or reach workspace code: a path crate that is not a member, a member's name from a registry or git, or an outside crate that takes a member.

#### M5 state at the start

On 2026-10-20, at commit `054c003` (M4 merged as PR #17):

* The layers table (`### Layers` in `docs/ARCHITECTURE.md`) has one row per layer, five rows in all.  Its columns are Layer, Crates and "Depend on".  A row with two crates has two lists in its last cell, split by `;`, in the order of the crates.  For example, the row `engine`, `roformer-gpu` has `dsp`; `roformer`.
* Above the table: "Crates depend only on crates in a lower layer (LAYER-1 in `docs/invariants.md`).  The page side and the worker side meet only through `stem-protocol`."
* Under the table: "Any crate but `dsp` (DSP-1) may take `test-support` as a dev-dependency.  No crate may take it as a normal dependency, so it never ships."
* The edges, from `cargo metadata --format-version 1 --all-features`:

  | From | To | Kind | Target |
  | - | - | - | - |
  | `app` | `dsp`, `engine`, `synth` | normal | any |
  | `app` | `stem-protocol` | normal | `cfg(target_arch = "wasm32")` |
  | `app` | `synth` | dev | `cfg(not(target_arch = "wasm32"))` |
  | `engine` | `dsp` | normal | any |
  | `roformer` | `test-support` | dev | any |
  | `roformer-gpu` | `roformer` | normal | any |
  | `roformer-gpu` | `test-support` | dev | any |
  | `stem-worker` | `roformer`, `roformer-gpu`, `stem-protocol` | normal | `cfg(target_arch = "wasm32")` |
  | `xtask` | `dsp`, `engine`, `roformer`, `synth` | normal | any |

  `dsp`, `synth`, `stem-protocol` and `test-support` have no edges.  No crate has a build edge.  Each edge is in the table today, and each crate that the table lists for a row is an edge.  So the check passes on today's manifests once the table has its new form.
* No stray exists today.  Each of the 16 path dependencies is on a member, and no other dependency has a member's name.  Of the 937 packages, every one whose `source` is `null` is a member, and every other one comes from a registry.  No package outside the members has a member's name.  In `resolve.nodes`, no node outside the members takes a member.
* `cargo metadata`'s JSON, from the Cargo book ([cargo metadata](https://doc.rust-lang.org/cargo/commands/cargo-metadata.html)) and from the output on the Mac:
  * `packages` holds every package of the build, and "all feature-enabled dependencies unless --no-deps is used".  With `--no-deps`, it holds the members only.
  * Each dependency has `name`, `source`, `rename` ("null if it is not renamed"), `kind` ("dev", "build", or null for a normal dependency) and `target` (null if not a target dependency).  `path` is "the file system path for a local path dependency", and is "not present if not a path dependency".
  * `name` is the package name, also when `rename` is set: `ts = { package = "test-support", ... }` gives `name` `test-support`.  A dependency's `source` is `null` for a path dependency.  Otherwise it starts with `registry+`, `sparse+` or `git+`.
  * `path` is absolute and normalised.  `"../dsp"`, `"../dsp/"`, `"./../dsp"` and `"../app/../dsp"` all give `<root>/dsp`.  A workspace-inherited `dsp.workspace = true` gives the same `path`.
  * Each package has `id`, `source` (`null` for a path crate) and `manifest_path`, absolute.  `workspace_root` is the absolute root, and `workspace_members` holds the IDs of the members.
* "All `path` dependencies residing in the workspace directory automatically become members" ([workspaces](https://doc.rust-lang.org/cargo/reference/workspaces.html)).  The next sentence names the exception: "The `exclude` key can be used to prevent paths from being included in a workspace."  This workspace has no `exclude`.  A path outside the repo is never a member either.
* The reviews found three ways round a check of member edges alone.  Each hides what the outside crate depends on:
  * A path dependency on a crate that is not a member.  `shim/` takes `roformer`, the root has `exclude = ["shim"]`, and `app` takes `shim` by path.  `cargo metadata --no-deps` lists 10 members, without `shim`.  `cargo tree -p app` shows `shim -> roformer`.
  * A `[patch]` to such a crate.  The root has `[patch.crates-io] shim = { path = "shim" }`, and `app` takes `shim = "0.1"`.  In `--no-deps` output, the dependency has a `registry+` source and no `path`.  `cargo tree -p app -e normal -i roformer` shows `roformer <- shim <- app`.  A patch in `.cargo/config.toml` to a folder outside the repo works the same way.  Without `--no-deps`, `shim` is in `packages` with `source` `null`, and is not a member.
  * A git dependency with a member's name: `roformer = { git = "..." }` in `app` has no `path`.
  * An outside crate that takes a member, with a `[patch]` back to the member.  A local git repo holds `foo`, which takes `roformer`.  `app` takes `foo` from git, and the root has `[patch."file://.../gitfoo"] roformer = { path = "roformer" }`.  No member's dependency is a stray, and `cargo tree -p app -e normal -i roformer` shows `roformer <- foo <- app` (fourth review).  Only `resolve.nodes` shows it: each node has `id` and `deps`, and each dep has `pkg`, the ID of the package.
  * The `[patch]` above, behind a feature that is off: `app` takes `shim = { version = "0.1", optional = true }` with `extra = ["dep:shim"]`.  Without `--all-features`, `packages` has no `shim`, and `cargo build -p app --features extra` links `roformer`.  With `--all-features`, `shim` is in `packages` with `source` `null` (third review).
* A path dependency through a symbolic link to a member fails in Cargo: "two packages named `dsp` in this workspace".
* Prototypes in scratch worktrees, on cargo 1.99.0:
  * `cargo metadata --format-version 1 --all-features --locked` took 0.8 s on the Mac, and lists the same 937 packages as without `--all-features`.  With `--no-deps` it took 0.04 s.
  * With `roformer = { path = "../roformer" }` added to `app`'s `[dependencies]`, `cargo metadata --no-deps --locked` exits 0 and shows the edge.  The full command with `--locked` exits 101: "cannot update the lock file ... because --locked was passed".  Without `--locked`, it exits 0 and rewrites `Cargo.lock`.  So a planted edge runs as in M4: the outer `cargo test` without `--locked` updates `Cargo.lock`, and the inner command then passes.
  * With `app = { path = "../app" }` in `engine`'s `[dev-dependencies]`, a cycle through a dev edge, the full `cargo metadata` exits 0.  So Cargo accepts that planted breach.
  * Two dependencies on one crate, for example a normal and a dev one, give two entries in `dependencies`.
* `cargo metadata` without `--no-deps` downloads the packages that it lists, for every target and, with `--all-features`, for every feature.  M4's `cargo tree --target wasm32-unknown-unknown` downloads only what that target reaches.  So a cold CI cache may download more crates, such as Windows-only ones.  rust-cache then keeps them.  This is not measured.
* `xtask` already takes `serde` with `derive` and `serde_json`, so the parser adds no crate (DEPS-1).
* `docs::section(markdown, "### Layers")` gives the text from the heading to the next `###` heading, at about line 126.  `### Layers` is the last `###` under `## Crates`, so that text runs on into the later `##` sections.  It holds the parameters table, `| Part | Parameters | ...`, at about line 102.  `docs::inline_code` gives the code spans of a text.
* `docs-check`'s `crates_documented` reads every code span under `## Crates`, the layers table included.  It checks only that each crate is named.  So nothing compares the table's edges with the workspace today.
* LAYER-1's `* Enforced by:` line says "No test checks the crate edges (TD-4)", and its `* Gaps:` line names TD-4 only.  In the docs, `TD-4` is in `docs/invariants.md` and `docs/tech-debt.md` only, outside the plans.  Test strings in `xtask/src/docs.rs` also hold it, and `docs-check` reads only Markdown.  The highest tech-debt ID is TD-22.  `docs-check` fails an ID that no doc defines, so this plan calls the next one "the new crates entry" until step 2 writes it.
* `rules::manifest_of` (M4) gives `worker/Cargo.toml` for `stem-worker` by a fixed case.  M5 reads each manifest from `cargo metadata`, and leaves M4's helper as it is.

#### M5 design

* Decision points for the maintainer.  Ask both questions before step 1.  Record each choice in Progress and the Decision log, and add "Decided on <date>: ..." to its question, as M3 did.  If a choice is not the recommended one, make the changes below, then review the plan again before implementing.
* Question 1: what "the page side and the worker side meet only through `stem-protocol`" means for the second check.  Decided on 2026-10-20: the maintainer chose (a), and to revisit it if the app ever separates natively (see decision 0005).
  * (a), recommended: the workspace crates that `app` and `stem-worker` reach through normal edges have only `stem-protocol` in common.  It is LAYER-1's text as written, and it keeps `stem-protocol` the one contract between the two WebAssembly modules.  The gain: it costs nothing today, because the two sides share no other workspace crate.  A later share can loosen the rule, with a record, and to tighten it after a share exists costs more.  The cost: a crate that both sides need later, such as `dsp` in the worker, needs a change to LAYER-1 and a record.  Also, `stem-protocol` can take no workspace crate as a normal dependency, even if it moves to a higher layer.  The walk reads every target, so a native-only edge from `app` to `roformer`, for separation in the native app, also fails.
  * (b) only LAYER-1's "Why": `app` reaches none of `roformer`, `roformer-gpu` and `stem-worker`.  The gain: the two sides may share pure crates such as `dsp`.  The cost: the check names crates, so a new model crate needs a change to the check.  Also, LAYER-1's sentence "The page side and the worker side meet only through `stem-protocol`" is then false, so (b) removes it from LAYER-1 and `ARCHITECTURE.md`.
  * (c) no second check, only the table.  The gain: no second check, 9 fewer unit tests and one fewer real-tree test.  The cost: a change to `app`'s row, with the dependency, lets `app` take `roformer`.  LAYER-1's sentence on the two sides stays prose only, with a new tech-debt entry.
* Question 2: is the table exact, or a ceiling?  Decided on 2026-10-20: the maintainer chose (x).
  * (x), recommended: exact.  A row lists each workspace crate that its crate depends on, apart from dev-dependencies on `test-support`, and no other.  The gain: the doc stays true (DOC-1), and the check cannot pass when the parser finds no edges.  The cost: each added or removed dependency between workspace crates changes the table.
  * (y) a ceiling, as this plan's first M5 text gave ("an allowlist built from the layers table").  A row may list a crate that its crate does not depend on.  The gain: a removed dependency needs no change to the table, and a row can allow a dependency before the code takes it.  The cost: the table can go stale, and the check needs another guard against a parser that finds no edges.
* The plan is written for (a) and (x).  For (b), change these:
  * the shell check, its tests, its messages and the shell-test count in "M5 acceptance";
  * the second test's name, and its words on LAYER-1's `* Enforced by:` line;
  * LAYER-1's text and "Why".  In `ARCHITECTURE.md`, the two sentences on the two sides above the table: "meet only through" and "have only `stem-protocol` in common";
  * the sentence on `the_page_and_the_worker_meet_only_through_stem_protocol` in the guide's changelog line;
  * 0005's text and its line in `docs/README.md`;
  * the `harness.md` row, the planted breach that only the shell check catches, and the CI line in "M5 acceptance";
  * the bullet in `harness-engineering.md`, and the bullet and opening line of `protocol.md`;
  * keep the M4-undo sentence on `rules::WEB_APP`, because (b)'s check uses `app`.
* For (c), change these:
  * remove the shell check and its 9 tests, the second real-tree test, and its name in the CI line in "M5 acceptance";
  * 0005 keeps both questions and records (c): change its Decision, its Consequences and its line in `docs/README.md`.  Its file name and title stay;
  * remove the shell's part of the first planted breach.  Keep the second planted breach, with the result "`crates_depend_only_downwards` passes", as the evidence for the new tech-debt entry;
  * remove the new sentences on the two sides in LAYER-1 and in `ARCHITECTURE.md`, and LAYER-1's new "Why" sentence;
  * remove the second test's name and its sentence on the `* Enforced by:` line, because `docs-check` fails a name with no test;
  * remove the shell test's part of the `harness.md` row, ", with the page side and the worker side apart" in the `harness-engineering.md` bullet, and the bullet and opening-line change of `protocol.md`;
  * remove the M4-undo sentence on `rules::WEB_APP`, and the M3-undo item that names the shell test;
  * remove the sentence on `the_page_and_the_worker_meet_only_through_stem_protocol` in the guide's changelog line;
  * keep the earlier Decision log lines, and add a line with the maintainer's choice;
  * add a tech-debt entry, with the next free ID after the new crates entry, and name it on LAYER-1's `* Gaps:` line.  Heading: `### TD-<n+1>: only the layers table keeps the page side and the worker side apart`.  Body: "A change to `app`'s row, with the dependency, lets `app` take `roformer` (LAYER-1)."  Fix: "the check of option (a) in `plans/active/harness-phase-2.md`, M5."
* For (y), change these:
  * the bullet "The table is exact", and the stale-entry check, its message and its tests, and the stale part of `a_row_that_lists_test_support_is_a_problem`;
  * the clauses "no stale-entry problem" in "What each case gives", and the table-test count;
  * add a guard that fails when the parser finds no edge, with a unit test, and rewrite "Why the check cannot pass while it reads nothing";
  * the paragraph "Each row lists every workspace crate ..." under the table in `ARCHITECTURE.md`, and 0005's text on the table;
  * the planted breach "`synth` added to `engine`'s row", which then passes;
  * the Decision log line on the exact table.
* For (b), (c) and (y), also change every other line of M5 that is true only for the recommended choices: the "Change" bullet, the API block, and the design bullets on the two sides and on decision 0005.  In "Idempotence and recovery", change the M3-undo item's shell test.
* Both answers give LAYER-1 a reading that outlives this plan.  So step 2 writes decision 0005 as `proposed` for any choices, and the maintainer accepts it after review.  M11's record then takes the next free number.
* The new table, lowest layer first.  The last column is "Depends on", because each row now has one crate:

  | Layer | Crate | Depends on |
  | - | - | - |
  | Test support | `test-support` | no workspace crate |
  | Pure | `dsp` | no workspace crate |
  | Pure | `synth` | no workspace crate |
  | Pure | `roformer` | no workspace crate |
  | Pure | `stem-protocol` | no workspace crate |
  | Engine and GPU | `engine` | `dsp` |
  | Engine and GPU | `roformer-gpu` | `roformer` |
  | Shells | `app` | `dsp`, `engine`, `stem-protocol`, `synth` |
  | Shells | `stem-worker` | `roformer`, `roformer-gpu`, `stem-protocol` |
  | Tools | `xtask` | `dsp`, `engine`, `roformer`, `synth` |

* One row per crate, because the `;` form maps each list to a crate by its position.  A person can misread that, and a parser needs a rule for it.
* The table is exact.  A row lists each workspace crate that its crate has an edge to, and no other crate.  It leaves out `test-support`.  So a row that lists a crate without an edge fails the check, as a stale doc must (DOC-1).
* The table allows only downward dependencies when its order holds.  Each layer's rank is the position of its first row, and a layer's rows are together.  A row may list only crates of a lower rank.  So the check also fails a table that lets a crate depend on its own layer or a higher one.
* The layers are fixed: `rules::LAYERS` holds "Test support", "Pure", "Engine and GPU", "Shells" and "Tools", in that order.  The table's layers, in rank order, must equal it.  So an agent cannot pass the rank check with a new layer, which makes LAYER-1's list of layers false (DOC-1).
* `test-support` (M3): it takes no workspace crate, with an edge of any kind.  So a dev edge to it reaches nothing, and the table can leave it out.  A dev edge to it from any crate but `dsp` needs no entry in the table.  A normal or build edge to it is a problem: a normal edge would ship it, and a build edge breaks "a dev-dependency only".  An edge of any kind from `dsp` is a problem (DSP-1).  A row that lists `test-support` is a problem, so the table keeps one way to say it.
* Strays are problems, because a stray hides what it depends on from the check.  The check reads `cargo metadata` without `--no-deps`, so `packages` also holds the crates that the build takes, patched crates included.  It still finds edges by `path`, not by name, so a renamed dependency counts under its package name.
* Gaps that stay, in a new tech-debt entry with the next free ID ("the new crates entry"), titled "LAYER-1 sees crates, not code":
  * A crate from a registry or git, under another name, can hold a copy of a workspace crate, such as `roformer`.
  * Source can reach across crates without an edge: `#[path = "../../roformer/src/x.rs"] mod x;` or `include!`.
  * A workspace crate's target can take its source from another crate's folder: `[lib] path = "../roformer/src/lib.rs"`.
  * A symbolic link in a crate's folder can point to another crate's source.
  * Fix: phase 3's reviewers look for all four.  A later check could ban `#[path]` and `include!` with a path outside the crate.  It could check that each workspace crate's `targets[].src_path` is in its own folder.  It could also fail a tracked symbolic link, which `git ls-files -s` shows with mode `120000`.
* The page side and the worker side, under option (a): the workspace crates that `app` reaches through normal edges, and the workspace crates that `stem-worker` reaches, share only `stem-protocol`.  Registry crates such as `wasm-bindgen` are on both sides, and the check does not read them.
  * Why a second check: a row of the table may list any crate of a lower layer.  So a change to `app`'s row, with the dependency, lets `app` take `roformer`, and the table check passes.  LAYER-1's "Why" is that the page does not link the model or the GPU code.
  * Each shell's set holds the shell itself, so `stem-worker` taking `app` is a shared crate.
  * The walk follows normal edges only, at every step, from every crate it reaches, `stem-protocol` included.  It keeps a set of the crates it has seen, so a cycle cannot make it loop.  A dev or build edge does not link into what ships.  M4's check reads normal edges too, but for wasm32 only.  This walk reads every target, so it is stricter for native code.
  * The two shells are `rules::WEB_APP` (M4).
* Decision 0005, for (a) and (x):
  * File: `docs/decisions/0005-how-strictly-the-crate-checks-read-layer-1.md`.
  * Title: `# 0005: how strictly the crate checks read LAYER-1`.
  * Status line: `Status: proposed`.
  * The line in `docs/README.md`: `` * `decisions/0005-how-strictly-the-crate-checks-read-layer-1.md`: the layers table lists each dependency between workspace crates, and the page side and the worker side share only `stem-protocol` (LAYER-1). ``
  * The four sections of 0001 to 0004:
    * Context: "LAYER-1 says that crates depend only on crates in a lower layer of the layers table.  It also says that the page side and the worker side meet only through `stem-protocol`.  Its 'Why' says that the page does not link the model or the GPU code.  `crates_depend_only_downwards` compares the dependencies between workspace crates with the table.  Two choices set how strictly the checks read the rule."
    * Options:
      * "The table: (x) exact.  A row lists each workspace crate that its crate depends on, apart from dev-dependencies on `test-support`, and no other.  The cost: each added or removed dependency between workspace crates changes the table."
      * "The table: (y) a ceiling.  A row may list a crate that its crate does not depend on.  The cost: the table can go stale, and the check needs another guard against reading nothing."
      * "The two sides: (a) the workspace crates that `app` and `stem-worker` reach through normal dependencies, for any target, have only `stem-protocol` in common.  The cost: a crate that both sides need later, such as `dsp` in the worker, needs a change to LAYER-1 and a record.  A native-only dependency of `app` on `roformer` also fails."
      * "The two sides: (b) `app` reaches none of `roformer`, `roformer-gpu` and `stem-worker`, as LAYER-1's 'Why' says.  The two sides may share pure crates such as `dsp`, so LAYER-1 no longer says that they meet only through `stem-protocol`.  The cost: the check names crates, so a new model crate needs a change to the check."
      * "The two sides: (c) no second check.  The cost: a change to `app`'s row, with the dependency, lets `app` take `roformer`.  LAYER-1's sentence on the two sides stays prose only."
    * Decision: "(x) and (a).", or the choices that the maintainer makes.
    * Consequences, one sentence each:
      * "The layers table lists each dependency between workspace crates, apart from dev-dependencies on `test-support`, and no other."
      * "LAYER-1's sentence on the two sides gains: the workspace crates that `app` and `stem-worker` reach through normal dependencies, for any target, have only `stem-protocol` in common."
      * "`the_page_and_the_worker_meet_only_through_stem_protocol` follows the normal dependencies from `app` and `stem-worker`, and fails on any shared workspace crate but `stem-protocol`."
      * "`stem-protocol` takes no workspace crate as a normal dependency, because both sides reach what it takes."
      * "A crate that both sides need is a change to LAYER-1, with its own record."
      * "`LAYERS` in `xtask/src/rules.rs` fixes the five layers.  A change of layers changes the table, `LAYERS` and LAYER-1 in one commit."
    * A last section, `## Revisit if`, as the maintainer asked on 2026-10-20: "The app separates songs natively, in its own process.  Today only the web app separates, in a Web Worker, and the native app does not.  A native app would run the model on a thread, so `app` would take `roformer` and `roformer-gpu` for its native target, and (a) fails that.  The split exists for web reasons: each WebAssembly module has its own memory, which never shrinks, and iOS stops a tab at 2 GB (MEM-4).  A native thread shares the process's memory, so a crate boundary gives it nothing.  Then read the shared crates in the wasm32 trees only, as `synth_brings_no_crate_into_the_web_app` reads them.  The check can use M4's `web_app_tree`."
  * `harness.md`'s sentence on decisions names 0005 too, because 0005 is about how the harness's checks read a rule.
* The API, in `xtask/src/rules.rs`:

  ```rust
  /// The kind of an edge, as `cargo metadata` gives it.
  #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
  pub enum Kind { Normal, Dev, Build }

  /// A dependency of one workspace crate on another.
  #[derive(Clone, Debug, PartialEq, Eq)]
  pub struct Edge { pub to: String, pub kind: Kind }

  /// Where a stray comes from: a folder, or a `source` such as `git+...`.
  #[derive(Clone, Debug, PartialEq, Eq)]
  pub enum Origin { Path(String), Source(String) }

  /// A crate from outside the members that can hold or reach workspace
  /// code: its name, and where it comes from.
  #[derive(Clone, Debug, PartialEq, Eq)]
  pub struct Stray { pub name: String, pub origin: Origin }

  /// A crate outside the members that takes a member: its name, its
  /// `source`, and the member.
  #[derive(Clone, Debug, PartialEq, Eq)]
  pub struct OutsideEdge { pub from: String, pub source: String, pub to: String }

  /// A workspace crate: its package name, its manifest relative to the
  /// root, its edges and the strays that it takes.
  #[derive(Clone, Debug, PartialEq, Eq)]
  pub struct WorkspaceCrate { pub name: String, pub manifest: String, pub edges: Vec<Edge>, pub strays: Vec<Stray> }

  /// The workspace crates, the path crates and member names in the build
  /// that are not members, and the crates outside the members that take a member.
  #[derive(Clone, Debug, PartialEq, Eq)]
  pub struct Workspace { pub crates: Vec<WorkspaceCrate>, pub outside: Vec<Stray>, pub outside_takes: Vec<OutsideEdge> }

  /// The layers of LAYER-1, lowest first.  The layers table must use these.
  pub const LAYERS: [&str; 5] = ["Test support", "Pure", "Engine and GPU", "Shells", "Tools"];

  /// Runs `cargo metadata --format-version 1 --all-features --locked` in `root`.
  pub fn workspace_metadata(root: &Path) -> Result<String>;

  /// The workspace, from `cargo metadata`'s JSON.
  pub fn workspace(metadata: &str) -> Result<Workspace>;

  /// LAYER-1: the edges match the layers table, the table goes only downwards, and no stray hides a dependency.
  pub fn crates_follow_the_layers(workspace: &Workspace, architecture: &str) -> Vec<Problem>;

  /// LAYER-1: the page side and the worker side meet only through `stem-protocol`.
  pub fn shells_meet_only_through_the_protocol(crates: &[WorkspaceCrate]) -> Vec<Problem>;
  ```

* `workspace_metadata` runs `env!("CARGO") metadata --format-version 1 --all-features --locked` in `root`, as `web_app_tree` runs `cargo tree`.  A failed command gives an `Err` with Cargo's stderr.
  * Without `--no-deps`, so that `packages` holds the patched and excluded path crates.
  * `--all-features`, so that `packages` also holds a crate behind a feature that is off.
  * `--locked`, so that a test never writes `Cargo.lock`.  Checks read the tree and write nothing.
  * No `--color never`: JSON has no colour codes, and the parser reads stdout only.
* `workspace` reads the JSON with `serde` structs that name only the fields it uses.
  * `crates` holds the packages whose `id` is in `workspace_members`, in the order of `packages`.
  * A member's folder is the parent of its `manifest_path`.  The parser compares folders as `Path` values, never as string prefixes, so `/x/roformer` does not match `/x/roformer-gpu`.
  * A dependency with a `path` that equals a member's folder is an edge, with that member's package name.
  * A dependency with a `path` that is not a member's folder is a stray of its crate, with `Origin::Path`.
  * A dependency without a `path` whose `name` is a member's package name is a stray of its crate, with `Origin::Source`.
  * `outside` holds each package that is not a member and whose `source` is `null`, with `Origin::Path` and its folder.  This catches a `[patch]` and an excluded path crate.  Such a package goes in once, also when it has a member's name.
  * `outside` also holds each other package that is not a member but has a member's name, with `Origin::Source`.  This catches a copy of a member that an outside crate takes.  Today there is none.
  * `outside_takes` holds one entry for each pair of a `resolve.nodes` entry and a member in its `deps[].pkg`, where the node is not a member and its package has a `source` that is not `null`.  A path crate that takes a member is already in `outside`, so it is not here as well.  This catches an outside crate that takes a member's name, with a `[patch]` back to the member.  Today there is none.
  * `resolve` is `null` with `--no-deps`, and that is an `Err` (code review round 2).  Each member's node must list every feature that its package declares, or it is an `Err`: without `--all-features`, `xtask`'s `cpal` is off.  So a run with the wrong flags cannot pass in silence.  `outside` is in the order of `packages`, and `outside_takes` in the order of `resolve.nodes`.
  * An edge from a crate to itself, as `engine = { path = "." }` in `[dev-dependencies]` gives, is left out.  Cargo accepts it, and it adds no crate to the build.
  * `manifest` is `manifest_path` relative to `workspace_root`, for example `worker/Cargo.toml`.  A manifest outside the root is an `Err`.
  * A `kind` other than `null`, `"dev"` or `"build"` is an `Err`, so a new kind cannot pass unread.
  * Two dependencies on one crate give two edges, in the order of the JSON, for example `app`'s normal and dev edges to `synth`.
* `crates_follow_the_layers` reads the table:
  * The table is the run of lines that start with `|`, right after `### Layers` and its text.  It must come before the next heading of any level.  So a later table, such as the parameters table, is never read as the layers table.
  * The header must be `| Layer | Crate | Depends on |`, after each cell is trimmed.  The second line is the separator: three cells of `-` only.  A wrong header or separator gives one problem, and the check does not read the rows.
  * Each row starts and ends with `|`, and has three cells.  The Layer cell is not empty.  The crate cell is one code span and nothing else.  The last cell is `no workspace crate`, or code spans joined by `, ` and nothing else, each crate once.  The list need not be sorted.
* What each case gives, so the expected problems are exact:
  * No workspace crate: that one problem.  The table is not read.
  * A table that cannot be read (no table, or a wrong header or separator): that one problem.  The edge, row and rank checks do not run.  The checks of strays and of `test-support` edges still run, because they do not need the table.
  * A row that cannot be read: one problem that quotes the row.  The row is left out of every other check.
  * Two rows for one crate: one problem.  The first row counts.  The second row is left out of every other check.
  * A row for a crate that is not a member: one problem.  The row is left out of every other check.
  * A member with no row: one problem.  The edges from it and to it, and the rows that list it, give no more table problems.  Its strays and `test-support` edges still give their problems.  The rank check skips it.  If its row was the only row of its layer, the layers problem is given too.
  * Ranks come only from the rows that count.
  * A row that is left out of every other check is also left out of the check that a layer's rows are together.
  * A layer whose rows are apart: one problem for the layer, however many rows are apart.
  * Layers that are not `LAYERS`: one problem for the table.  The rank check still runs on the table's own ranks.
  * A listed crate that is not a member: one problem.  It gives no stale-entry problem, and the rank check skips it.
  * A listed `test-support`: one problem.  It is left out of the stale-entry and rank checks.
  * An edge of any kind, or several, from `dsp` to `test-support`: the `dsp` problem, once.
  * An edge of any kind from `test-support` to a workspace crate: one problem for each crate that it takes, and no table problem.
  * A normal or build edge to `test-support` from a crate other than `dsp` and `test-support`: one problem for each of the two kinds that occurs, and no table problem.
  * Several edges from one crate to another, in any order: one problem for the pair.  `<kinds>` names each kind once, in the order normal, dev, build, joined by `, `.
* `shells_meet_only_through_the_protocol` walks normal edges from each of `app` and `stem-worker`, to every depth.  Each set holds its shell.  The shared crates, apart from `stem-protocol`, are a problem.  They are named in sorted order.
* Every problem has `rule: "LAYER-1"` and `line: None`.  `file` is set as follows:
  * `docs/ARCHITECTURE.md` for a problem whose fix is in the table: a table or a row that cannot be read, a stale entry, a rank, layer rows that are apart, a member without a row, a row for a crate that is not a member, two rows, a listed crate that is not a member, `test-support` in a row, and layers that are not `LAYERS`.
  * The crate's `manifest` for a problem whose fix is in a manifest: an edge that the table does not list, a stray of a crate, an edge to `test-support`, and an edge from it.
  * `Cargo.toml` for a problem with the workspace: a package in `outside`, an entry in `outside_takes`, no workspace crate, a missing shell, and shared crates.
* The messages:
  * An edge that the table does not list: `LAYER-1: engine depends on app (dev), but the layers table in docs/ARCHITECTURE.md does not list it.  Fix: remove the dependency from engine/Cargo.toml.  If the design changes, add app to engine's row in the same commit.`
  * A listed crate without an edge: `LAYER-1: the layers table in docs/ARCHITECTURE.md lists dsp for app, but app/Cargo.toml has no dependency on it.  Fix: remove dsp from app's row, so the table stays true.`
  * A row that lists a crate that is not lower: `LAYER-1: the layers table in docs/ARCHITECTURE.md lets engine (Engine and GPU) depend on app (Shells), which is not in a lower layer.  Fix: remove the dependency, or move a crate to another layer of the table, so that each crate depends only on lower layers.`
  * Layers that are not `LAYERS`: `LAYER-1: the layers table in docs/ARCHITECTURE.md has the layers "Test support, Base, Pure, Engine and GPU, Shells, Tools", not those in LAYERS in xtask/src/rules.rs.  Fix: use the five layers in LAYERS: Test support, Pure, Engine and GPU, Shells, Tools.  To change the layers, change the table, LAYERS and LAYER-1 in one commit (DOC-1).`  The layers are in rank order.
  * A layer whose rows are apart: `LAYER-1: the layers table in docs/ARCHITECTURE.md has rows for Pure that are not together.  Fix: put each layer's rows together, lowest layer first.`
  * A member without a row: `LAYER-1: the workspace crate engine has no row in the layers table in docs/ARCHITECTURE.md.  Fix: add a row with its layer and the workspace crates that it depends on.`
  * A row for a crate that is not a member: `LAYER-1: the layers table in docs/ARCHITECTURE.md has a row for foo, which is not a workspace crate.  Fix: remove the row, or use the package name from its Cargo.toml.`
  * Two rows for one crate: `LAYER-1: the layers table in docs/ARCHITECTURE.md has 2 rows for engine.  Fix: keep one row for each crate.`
  * A listed crate that is not a member: `LAYER-1: the layers table in docs/ARCHITECTURE.md lists foo for app, but foo is not a workspace crate.  Fix: correct the name, or remove it from app's row.`
  * A row that lists `test-support`: `LAYER-1: the layers table in docs/ARCHITECTURE.md lists test-support for engine.  Fix: remove it.  The table leaves out test-support, which any crate but dsp may take as a dev-dependency.`
  * A normal edge to `test-support`: `LAYER-1: engine takes test-support as a normal dependency, so it would ship.  Fix: move it to [dev-dependencies] in engine/Cargo.toml.`
  * A build edge to `test-support`: `LAYER-1: engine takes test-support as a build dependency, but it is a dev-dependency only.  Fix: move it to [dev-dependencies] in engine/Cargo.toml.`
  * An edge from `dsp` to `test-support`: `LAYER-1: dsp takes test-support, but dsp must have no dependencies (DSP-1).  Fix: remove it from dsp/Cargo.toml.`
  * An edge from `test-support`: `LAYER-1: test-support takes dsp, but test-support must take no workspace crate.  The crates that take it must reach nothing through it.  Fix: remove dsp from test-support/Cargo.toml.`
  * A path stray of a crate: `LAYER-1: app takes shim from /x/shim, which is not a workspace crate, so this check cannot see what shim depends on.  Fix: make shim a workspace crate with a row in the layers table, or remove it.`
  * A stray with a member's name: `LAYER-1: app takes roformer from git+https://example.com/x, not from the workspace.  Fix: take roformer by path, or remove it.`
  * A package in `outside` with a member's name: `LAYER-1: the build takes roformer from git+https://example.com/x, not from the workspace.  Fix: remove the dependency that brings it in.`
  * A path crate in `outside`: `LAYER-1: the build takes shim from /x/shim, a path crate that is not a workspace crate, so this check cannot see what shim depends on.  Fix: remove the [patch] or exclude that brings it in.  Or make it a workspace crate with a row in the layers table.`
  * An outside crate that takes a member: `LAYER-1: foo, from git+file:///x/gitfoo, takes the workspace crate roformer, so this check cannot see the dependency.  Fix: remove the [patch] that points roformer at the workspace, or remove foo.`
  * No table, or no table before the next heading: `LAYER-1: docs/ARCHITECTURE.md has no table under ### Layers, so this check read nothing.  Fix: restore the table, with the columns Layer, Crate and Depends on.`
  * A header with no line after it (code review round 1): `LAYER-1: the layers table in docs/ARCHITECTURE.md has no separator line after its header.  Fix: add "| - | - | - |" after "| Layer | Crate | Depends on |", then one row for each crate.`
  * Another header or separator: `LAYER-1: the layers table in docs/ARCHITECTURE.md starts with "<line>", not "| Layer | Crate | Depends on |" and "| - | - | - |".  Fix: use those three columns, one row for each crate.`  `<line>` is the first line that is wrong.
  * A row that the check cannot read: `LAYER-1: the layers table in docs/ARCHITECTURE.md has a row that this check cannot read: "<line>".  Fix: give the layer and one crate in a code span.  Then give its dependencies once each, as code spans joined by ", ", or "no workspace crate".`
  * No workspace crate: `LAYER-1: cargo metadata gave no workspace crate, so this check read nothing.  Fix: run cargo metadata --format-version 1 --all-features --locked at the workspace root.`
  * Shared crates: `LAYER-1: app and stem-worker both depend on roformer, roformer-gpu, directly or through other workspace crates.  The page side and the worker side meet only through stem-protocol.  Fix: keep each crate on one side, and send what the other side needs as a message in stem-protocol.`
  * A shell that is not a member: `LAYER-1: the workspace has no crate stem-worker, so this check cannot compare the page side and the worker side.  Fix: if the web app changed, change WEB_APP in xtask/src/rules.rs and LAYER-1's text (DOC-1).`
* Why the check cannot pass while it reads nothing: the table and the edges must match both ways.  If the parser finds no edges, each crate that a row lists is a problem.  If it finds no crates, the "no workspace crate" problem fails it.  If the table is empty or absent, the "no table" problem or each member without a row fails it.
* The real-tree tests, in `xtask/tests/rules.rs`.  Each asserts that the list is empty, and prints the count and the messages, as `synth_brings_no_crate_into_the_web_app` does:
  * `crates_depend_only_downwards` runs `workspace_metadata`, `workspace` and `crates_follow_the_layers` with the text of `docs/ARCHITECTURE.md`.
  * `the_page_and_the_worker_meet_only_through_stem_protocol` runs `shells_meet_only_through_the_protocol` on the same crates.
* LAYER-1, in `invariants.md`, under option (a):
  * The text becomes: "Each workspace crate depends only on the workspace crates that its row in the layers table of `docs/ARCHITECTURE.md` lists, and on `test-support` as a dev-dependency.  This covers dev- and build-dependencies, for every target and feature.  Every path crate in the build is a workspace crate.  No crate from a registry or git has a workspace crate's name, or takes a workspace crate.  Each listed crate is in a lower layer of the table (Test support, Pure, Engine and GPU, Shells, Tools).  `test-support` is a dev-dependency only and takes no workspace crate, and `dsp` does not take it.  The page side and the worker side meet only through `stem-protocol`.  The workspace crates that `app` and `stem-worker` reach through normal dependencies, for any target, have only `stem-protocol` in common.  In the normal dependency tree of the web app (`app` and `stem-worker`), `synth` brings no other crate."
  * "Why" gains, after its first sentence: "It keeps `stem-protocol` the one contract between the two WebAssembly modules."
  * `* Enforced by:` becomes: "`crates_depend_only_downwards`, `the_page_and_the_worker_meet_only_through_stem_protocol` and `synth_brings_no_crate_into_the_web_app` (CI `test` job).  The first compares the dependencies between workspace crates in `cargo metadata` with the layers table.  It also fails on a crate that hides a dependency from that comparison.  The second follows the normal dependencies from `app` and `stem-worker`, and fails on a shared workspace crate other than `stem-protocol`."
  * `* Gaps:` names the new crates entry by its ID.
* `CLAUDE.md`'s LAYER-1 line stays as it is: it is still true.

#### M5 steps

Each step is one signed commit, and fmt, clippy, the tests and `docs-check` pass before it (GIT-2).  Fixes from the review rounds are later commits.

1. The parser and the two checks, test first.
   * Write the types, `workspace_metadata`, and stubs: `workspace` returns an empty `Workspace`, and each check returns no problems.
   * Write the unit tests in `rules.rs`'s test module.  Each test asserts the whole result.  A check test asserts the whole list of `Problem`s, sorted by file and message, so the order is free.  So a wrong `file`, or an extra problem, fails it.
   * The parser tests feed JSON with `workspace_root` `/x`, and paths such as `/x/dsp`.  So no home folder is committed.  The members are listed with `roformer` before `roformer-gpu`, and `stem-protocol`'s manifest is `/x/protocol/Cargo.toml`.  Each asserts the whole `Workspace`, so none passes against the stub:
     * `path_dependencies_on_members_are_edges`: `app` takes `dsp` (normal), `engine` renamed as `audio`, `synth` both as a normal edge and as a dev edge for `cfg(not(target_arch = "wasm32"))`, and `stem-protocol` as a build edge.  It also takes `anyhow` from the registry.  `stem-worker`'s manifest is `/x/worker/Cargo.toml`, and it takes `roformer-gpu`.  `engine` also takes itself as a dev edge, which the result leaves out.  So a parser that uses `rename`, keeps one edge for each crate, or matches folders by prefix fails.
     * `a_path_that_is_not_a_member_is_a_stray`: `app` takes `shim` from `/x/shim`, `nested` from `/x/app/nested`, a folder under a member, and `roformer` from `/x/vendor/roformer`.  So a parser that matches with `Path::starts_with`, or matches a path dependency by its name, fails.
     * `a_members_name_from_elsewhere_is_a_stray`: `dsp` takes `engine` from `registry+https://github.com/rust-lang/crates.io-index`, and `roformer` from `git+https://example.com/x`, each with no `path`.  So a parser that matches by name, drops them, or knows one source only fails.
     * `a_path_package_that_is_not_a_member_is_outside`: a package `shim` with `source` `null`, not in `workspace_members`, as a `[patch]` gives it.  `app`'s dependency on it has a `registry+` source.  Also a git package `roformer` that is not a member, and a path package `roformer` that is not a member, which goes in once.  A `resolve` node for `shim -> roformer` gives no `outside_takes` entry.  So a parser that reads only dependencies, or chains the two filters, fails.
     * `a_package_that_is_not_a_member_is_left_out`: a member, and a registry package that is not in `workspace_members`.
     * `an_outside_package_that_takes_a_member_is_found`: a `resolve` node for a git `foo` with a dep whose `pkg` is `roformer-gpu`'s ID and whose `name` is `roformer_gpu`.  A second dep, on a git `roformer` that is not a member, gives no entry.  So a parser that matches `deps[].name`, or a name in the ID, fails.
     * `an_unknown_kind_is_an_error`: `"kind": "other"`.
     * `a_manifest_outside_the_root_is_an_error`: a member at `/y/app/Cargo.toml`, and in a second case at `/xy/app/Cargo.toml`.  So a parser that strips the root as a string prefix fails.
   * The table tests use a fixture: the new table under `### Layers`, and the crates of today's edges.  A test that adds a row puts it where the layer's rows stay together, unless it tests that rule.
     * `the_layers_of_today_give_no_problem`.
     * `an_edge_that_the_table_does_not_list_is_a_problem`: a normal `engine -> app` edge, and a normal `stem-worker -> dsp` edge.  It expects `worker/Cargo.toml` in the second problem, so a check that builds the file from the crate's name fails.
     * `a_dev_edge_counts`: a dev `engine -> app` edge.  It asserts `(dev)` in the message.
     * `an_edge_from_a_crate_with_no_workspace_crate_is_a_problem`: a normal `synth -> dsp` edge, with `synth`'s row unchanged, gives one problem in `synth/Cargo.toml`.  So a check that compares only rows with a list fails.
     * `a_dev_edge_keeps_a_listed_crate`: a dev `roformer-gpu -> dsp` edge, with `dsp` added to `roformer-gpu`'s row, gives no problem.  So does a build `roformer-gpu -> dsp` edge in place of the dev one.  So a stale-entry check that reads only normal edges fails.
     * `kinds_are_named_once_in_their_order`: a build, a dev, a normal and a second normal `engine -> app` edge, in that order, give one problem with `(normal, dev, build)`.  So a check that drops build edges, or names kinds in the order of the JSON, fails.
     * `a_listed_crate_without_an_edge_is_a_problem`: `app` without its `dsp` edge, and `stem-worker` without its `roformer` edge.  So a check that reads only one way, or does not use `manifest`, fails.
     * `a_row_that_lists_a_higher_layer_is_a_problem`: `app` in `engine`'s row, and the edge.  It asserts only the rank problem.
     * `a_row_that_lists_its_own_layer_is_a_problem`: `dsp` in `synth`'s row, and the edge.  So a check with `<=`, or one that ranks rows and not layers, fails.
     * `the_layers_are_fixed`: a row `` | Base | `dsp` | no workspace crate | `` between the `Test support` row and the `Pure` rows, with `dsp`'s own row removed, and `dsp` in `synth`'s row with the edge.  It asserts the layers problem only.  Then two more cases, each from the base fixture, each with the layers problem only.  First, the `Test support` row after `stem-protocol`'s row, which gives "Pure, Test support, Engine and GPU, Shells, Tools".  Second, `` | Pure | `test-support` | no workspace crate | `` in place of the `Test support` row, a missing layer.  So a check that takes any layer names, compares the layers as a set, or checks only that each layer is in `LAYERS`, fails.
     * `a_layer_whose_rows_are_apart_is_a_problem`: `roformer`'s and `stem-protocol`'s rows moved to after `xtask`'s row.  It asserts one problem for `Pure`.
     * `a_crate_without_a_row_is_a_problem`: `engine`'s row removed, with `app`'s and `xtask`'s rows unchanged.  It asserts one problem, for `engine` alone.
     * `a_row_for_a_crate_that_is_not_a_member_is_a_problem`: a row `` | Pure | `foo` | `dsp` | `` after `stem-protocol`'s row.  It asserts one problem.
     * `a_repeated_row_is_a_problem`: a second `engine` row, with the Layer `Engine and GPU`, right after the first, that lists `app`.  It asserts one problem, because the first row counts.
     * `a_listed_crate_that_is_not_a_member_is_a_problem`: `foo` in `app`'s row.  It asserts one problem.
     * `test_support_as_a_dev_dependency_needs_no_row`: dev edges to `test-support` from `app`, `stem-protocol` and `xtask`, beside the fixture's own, give no problem.
     * `test_support_as_a_normal_or_build_dependency_is_a_problem`: a normal `engine` edge and a build `synth` edge give two problems, with their two messages.
     * `test_support_between_dev_edges_is_still_a_problem`: `engine` takes `test-support` as a dev edge, a normal edge, a dev edge and a second normal edge, in that order.  It asserts one normal-edge problem.  So a parser or a check that keeps the first or the last kind fails.
     * `dsp_may_not_take_test_support`: a dev and a normal `dsp` edge give the `dsp` problem, once.
     * `test_support_takes_no_workspace_crate`: a dev `test-support -> dsp` edge, with `test-support`'s row unchanged, gives the `test-support` problem only.  So a relayered `test-support` cannot carry a dev edge upwards.
     * `a_row_that_lists_test_support_is_a_problem`: `test-support` in `engine`'s row, and `engine` has no edge to it.  It asserts one problem, not a stale-entry problem as well.
     * `strays_are_problems`: a path stray and a git stray in `app`, a path crate in `outside`, and a git package with a member's name in `outside`.  It asserts four problems, each with its own message.
     * `an_outside_package_that_takes_a_member_is_a_problem`: one entry in `outside_takes`.
     * `a_missing_table_is_a_problem`: no `### Layers` heading.  Then a `### Layers` heading with no table, and a later `## X` section that holds a table.  Each gives the "no table" problem only.  Then the first case with a path stray in `app`, which gives the stray problem too.  So a check that returns early on "no table" fails.
     * `the_old_table_is_a_problem`: today's table, with its `Crates` and `Depend on` header.  It asserts the header problem only.
     * `a_bad_separator_is_a_problem`: `| Layer | Crate | Depends on |` and then a row in place of the separator.
     * `a_row_that_cannot_be_read_is_a_problem`: each bad row is added after `xtask`'s row, one case at a time, with the Layer `Pure` except in the empty-Layer case.  Each gives one problem that quotes it, and no layer-order problem.  The crate cell is `` `dsp` `` unless a case names another, so a lenient parser gives a two-rows problem and fails.  The cases are in the last cell unless they name another: `` `dsp` (dev only) ``, `dsp` without a code span in the crate cell (one case) and in the last cell (a second case), `` `dsp` (pure) `` in the crate cell, two crates in the crate cell, a row with two cells, an empty Layer cell, a row without its last `|`, an empty last cell, `` `dsp`,`engine` `` and `` `dsp`, `dsp` ``.
     * `an_empty_workspace_is_a_problem`: no crates, and a table with its header and separator and no rows.  It asserts the "no workspace crate" problem only.  So a check that only compares the two lists fails.
   * The shell tests use the fixture crates:
     * `the_shells_of_today_meet_only_through_stem_protocol`.
     * `a_crate_on_both_sides_is_a_problem`: an `app -> roformer-gpu` edge.  It asserts the message with `roformer, roformer-gpu`, so a check that does not follow `roformer-gpu -> roformer` fails.
     * `a_crate_reached_through_another_is_found`: an `engine -> roformer` edge.  `app` reaches `roformer` only through `engine`.  So a check that reads only `app`'s own edges fails.
     * `the_worker_side_is_followed_to_every_depth`: `stem-worker` without its direct `roformer` edge, and a `roformer -> dsp` edge.  `stem-worker` then reaches `dsp` at depth 3, through `roformer-gpu` and `roformer`.  It asserts the message with `dsp` only, because `app` does not reach `roformer`.  So a walk of depth 2 or less fails.
     * `a_crate_reached_through_stem_protocol_is_found`: a normal `stem-protocol -> test-support` edge gives `test-support`, which neither side reaches otherwise.  So a check that stops at `stem-protocol` fails.
     * `a_shell_is_in_its_own_set`: a `stem-worker -> app` edge gives the message with `app, dsp, engine, synth`.  So a check that leaves out the shell itself gives `dsp, engine, synth`, and fails.  Then, from the base fixture, an `app -> stem-worker` edge gives `roformer, roformer-gpu, stem-worker`.
     * `a_dev_or_build_edge_does_not_join_the_sides`: a dev `app -> roformer` edge, a dev `engine -> roformer` edge, a build `app -> roformer` edge and a build `engine -> roformer` edge give no problem.  So a check that follows dev or build edges, at the first step or later, fails.
     * `a_cycle_ends`: a normal `dsp -> engine` edge beside `engine -> dsp`.  It gives no problem and returns.  A walk with no set of seen crates loops or overflows the stack, so a person sees that failure as a hang or an abort, not as a failed test.
     * `a_missing_shell_is_a_problem`: crates without `stem-worker`, then crates without `app`.  Each gives its problem.
   * See every test that expects a problem fail against the stubs.  The tests that expect no problem pass against them; that is expected.  Then write the functions.
   * Update `rules.rs`'s module comment if it no longer says what the file reads.
2. The real-tree tests, the new table and the docs, in one commit: the new table makes the docs that describe it wrong until they change (DOC-1).
   * Write the two tests in `xtask/tests/rules.rs`.  Run `crates_depend_only_downwards` on today's table.  It fails with the header problem.  Record the output in the PR.
   * Replace the table in `docs/ARCHITECTURE.md` with the table in "M5 design".  Both tests now pass.
   * `docs/ARCHITECTURE.md`, above the table:
     * "Crates depend only on crates in a lower layer (LAYER-1 in `docs/invariants.md`)." becomes "Each workspace crate depends only on the workspace crates that its row lists, and on `test-support` as a dev-dependency (LAYER-1 in `docs/invariants.md`).  Each listed crate is in a lower layer."
     * After "The page side and the worker side meet only through `stem-protocol`.", add: "The workspace crates that `app` and `stem-worker` reach through normal dependencies, for any target, have only `stem-protocol` in common."
   * `docs/ARCHITECTURE.md`, under the table:
     * Before the two sentences on `test-support`, add as a new paragraph: "The rows run from the lowest layer to the highest.  Each row lists every workspace crate that its crate depends on, dev- and build-dependencies included, apart from `test-support`.  `crates_depend_only_downwards` compares the table with `cargo metadata`.  Change the table in the commit that adds or removes a crate, or a dependency between crates.  The layers are those of LAYER-1, and `LAYERS` in `xtask/src/rules.rs` holds them."
     * After the sentence "Change the table ...", add: "Every path crate in the build is a workspace crate with a row.  No crate from a registry or git has a workspace crate's name, or takes a workspace crate."
     * "No crate may take it as a normal dependency, so it never ships." becomes "No crate may take it as a normal or build dependency: it is a dev-dependency only, so it never ships.  It takes no workspace crate."
   * `docs/ARCHITECTURE.md`, `## Tests`: the bullet that names the docs check also names "the structural checks (`xtask/tests/rules.rs`)".  M3 and M4 left this out.
   * `docs/decisions/0005-how-strictly-the-crate-checks-read-layer-1.md`, as in "M5 design".  Add its line to `docs/README.md`, or `docs-check` fails.
   * `invariants.md`: LAYER-1's text, "Why", `* Enforced by:` line and `* Gaps:` line, as in "M5 design".
   * `tech-debt.md`:
     * Delete TD-4.  Under `## Closed`, add: "* TD-4: closed by `crates_depend_only_downwards` (harness phase 2, M5).  It compares the dependencies between crates in `cargo metadata` with the layers table in `ARCHITECTURE.md` (LAYER-1)."
     * Add the new crates entry, with the next free ID `<n>`, under `## Enforcement gaps`, after the last entry of that section.  The text:
       * Heading: `### TD-<n>: LAYER-1 sees crates, not code`.
       * Body: "`crates_depend_only_downwards` reads the dependencies between crates, not the source (LAYER-1)."  Then the four gaps and the fix in "M5 design" as bullets.  Keep `#[path = "../../roformer/src/x.rs"] mod x;` in one code span, so `docs-check` does not read it as a path: `#` and `=` make a code span prose.
   * `harness.md`, one components row.  Component: "`crates_depend_only_downwards`, `the_page_and_the_worker_meet_only_through_stem_protocol` (LAYER-1)".  Kind "sensor, computational", added "phase 2".  Failure: "nothing compared the layers table with the manifests, or kept the page side and the worker side apart; a crate outside the workspace could hide a dependency".  Also add decision 0005 to the sentence on decisions at the top of `harness.md`.  The row goes after M4's two rows.
   * `working-with-agents.md`:
     * Under "Before you merge", add the bullet: "* A change to the layers table in `docs/ARCHITECTURE.md` changes what LAYER-1 allows.  Review it as a design change."
     * In the changelog, add: "* <date>, phase 2 (M5): `crates_depend_only_downwards` compares the layers table with the dependencies between crates.  `the_page_and_the_worker_meet_only_through_stem_protocol` keeps the page side and the worker side apart.  Review a change to the table as a design change."  `<date>` is the date of the commit.
   * `harness-engineering.md`: in phase 2's bullet on the structural test, "crate layering over `cargo metadata` (TD-4)" becomes "crate layering over `cargo metadata` against the layers table, with the page side and the worker side apart (TD-4, M5)".
   * `.claude/rules/protocol.md`: add the bullet "* `stem-protocol` takes no workspace crate as a normal dependency, because both sides take it (LAYER-1, decision 0005)."  Change its opening line from "PROTO-1 and PROTO-2 in `docs/invariants.md`." to "LAYER-1, PROTO-1 and PROTO-2 in `docs/invariants.md`.".  So the rule that loads for `protocol/` matches the new check.
   * Measure the time that the two tests add to `cargo test -p xtask --test rules`.  State it in the PR.

#### M5 acceptance

* `cargo test -p xtask --lib`: the 11 parser tests, the 30 table tests and the 9 shell tests pass.  These counts include the tests that the code review rounds added: `a_row_left_out_is_left_out_of_the_layer_checks` and `a_header_alone_is_a_problem` (round 1), `a_metadata_without_resolve_is_an_error` and `a_metadata_without_all_features_is_an_error` (round 2), and `a_member_without_a_resolve_node_is_an_error` (round 3).  Each that expects a problem, or a parse result that is not empty, failed against the stubs.
* `cargo test -p xtask --test rules`: `crates_depend_only_downwards` and `the_page_and_the_worker_meet_only_through_stem_protocol` pass.  On the old table, `crates_depend_only_downwards` failed with the header problem.
* Planted breaches on the real tree, run locally and not kept.  Run the outer `cargo test -p xtask --test rules` without `--locked`, because a planted edge changes `Cargo.lock`.  Afterwards restore the manifests, `Cargo.lock` and the docs, and delete `shim/` and the local git repo.  Record each result in the PR:
  * `roformer = { path = "../roformer" }` in `app`'s `[dependencies]`: `crates_depend_only_downwards` fails and names `app` and `roformer`.  The shell test fails and names `roformer`.
  * The same edge, with `roformer` also added to `app`'s row: `crates_depend_only_downwards` passes, and only the shell test fails.
  * `app = { path = "../app" }` in `engine`'s `[dev-dependencies]`: `crates_depend_only_downwards` fails with `(dev)`.
  * `test-support = { path = "../test-support" }` in `dsp`'s `[dev-dependencies]`: `crates_depend_only_downwards` fails with the DSP-1 message.
  * A crate `shim/` that takes `roformer`, `exclude = ["shim"]` in the root `Cargo.toml`, and `shim = { path = "../shim" }` in `app`: `crates_depend_only_downwards` fails with the path stray and the path crate in `outside`.
  * The same `shim/`, with `[patch.crates-io] shim = { path = "shim" }` in the root and `shim = "0.1"` in `app`: `crates_depend_only_downwards` fails with the path crate in `outside`.
  * The same `[patch]`, with `shim = { version = "0.1", optional = true }` and `extra = ["dep:shim"]` in `app`: `crates_depend_only_downwards` fails with the path crate in `outside`.
  * A local git repo with a crate `foo` that takes `roformer`, `foo` from that repo in `app`, and a `[patch]` of that repo's `roformer` to `roformer/`: `crates_depend_only_downwards` fails with the outside crate that takes a member.
  * `dsp` removed from `app`'s row: `crates_depend_only_downwards` fails, and says that the table does not list the edge.
  * `stem-protocol`'s row removed: `crates_depend_only_downwards` fails, and names `stem-protocol` as a crate without a row.
  * `synth` added to `engine`'s row, with no edge: `crates_depend_only_downwards` fails with the stale-entry message only, which names `engine/Cargo.toml`.
* `grep -c "Enforced by: prose only" docs/invariants.md` still gives 7.
* fmt, clippy (every clippy command in `CLAUDE.md`), `cargo test --workspace --exclude roformer-gpu`, `cargo test -p roformer-gpu --lib` and `--doc`, and `docs-check` pass.
* CI: the `test` job log lists `crates_depend_only_downwards ... ok` and `the_page_and_the_worker_meet_only_through_stem_protocol ... ok`.  The PR states the time that the two tests add in CI.

### M6: dsp relaxes only the lints DSP-1 needs (closes TD-2; DSP-1, DOC-1; maintainer decision)

* Change: replace `#![allow(clippy::all)]` in `dsp/src/lib.rs` with one `#![expect(clippy::<lint>, reason = "...")]` for each lint that a DSP-1 construct triggers.  Every other clippy lint then has its workspace level in `dsp`.
* The gain: the correctness, suspicious, complexity, `perf` and style groups run on `dsp` again.  A `disallowed-methods` list that reaches `dsp` (a `dsp/clippy.toml` or a root `clippy.toml`) now applies there.  M12 and M13 configure other crates only.  With `-D warnings`, an exception that no longer fires fails clippy.
* The phase plan said "the style lints".  Two of the four lints are in the complexity group, so the title says "the lints".
* Terms: "the beginner forms" are `x = x + ...`, `x = x - ...`, `len() > 0` and an `if` for each bound of a clamp.  "DSP-1's constructs" are the types and syntax that DSP-1 lists.  M6 uses "a beginner" for the reader of `dsp`, as `dsp/src/lib.rs` does.

#### M6 state at the start

On 2026-10-20, at commit `df8a954` (M5 merged as PR #18):

* `dsp/src/lib.rs` (25 lines) has `#![allow(clippy::all)]` on line 9.  Its comment says that the crate uses plain `for`, `while`, `if` and indexing, "so all Clippy lints are switched off".  `dsp/Cargo.toml` has `[lints] workspace = true` and no dependencies.
* The workspace `Cargo.toml` allows two clippy lints for every crate: `needless_range_loop` and `chunks_exact_to_as_chunks`.  No `clippy.toml` and no `.cargo/` folder exist.  The only other clippy attribute in the tree is `#[allow(clippy::too_many_arguments)]` in `roformer-gpu/src/net.rs`.
* CI pins the toolchain: `RUST_VERSION: "1.99.0"` in `.github/workflows/ci.yml`, with the comment "Bump deliberately".  The `clippy` job runs `cargo clippy --locked --workspace --all-targets -- -D warnings`, which checks `dsp`'s lib and its unit tests.  No CI step runs clippy on `dsp` alone.
* DSP-1 in `invariants.md` says: "Clippy's style lints are relaxed there on purpose."  `* Enforced by: prose only.  A scan on 2026-10-19 found no breach.`  `* Gaps: TD-2, TD-3.`
* TD-2 says that the allow "turns off the correctness and performance groups as well as style, and silences `disallowed_methods` (DSP-1)".  Its fix: "allow only the style lints DSP-1 relaxes, by name."
* `.claude/rules/dsp.md` gives DSP-1's constructs and says nothing about clippy.
* The code uses types outside DSP-1's list (`&[f32]`, `Vec<f32>`, `usize`).  `biggest` and `median` return `f32`, `peaks` returns `Vec<(f32, f32)>`, `mix` takes `&[&[f32]]`, and `median` has a `bool`.  So the scan's "found no breach" is false for the types.  It holds for iterators, closures and `swap`.
* Facts from a prototype in a scratch worktree, with clippy 1.99.0:
  * Without the allow, `cargo clippy -p dsp --all-targets -- -D warnings` gives 33 errors from four lints: 16 in the lib and 17 in the lib test.  Most sites count twice, once in each target:
    * `clippy::assign_op_pattern` (style), 25 errors at 13 sites, one of them in a test: every `x = x + ...`, and `p = p - length` in `wrap_loop`.  The fix is `x += ...`.
    * `clippy::len_zero` (style), 4 errors at 2 sites: `samples.len() > 0` in `biggest` and `stems.len() > 0` in `mix`.  The fix is `!samples.is_empty()`.
    * `clippy::manual_clamp` (complexity), 2 errors at 1 site: the two `if`s that clamp `total` in `mix`.  The fix is `total.clamp(-1.0, 1.0)`.
    * `clippy::manual_swap` (complexity), 2 errors at 1 site: the swap through `temp` in `median`'s bubble sort.  The fix is `copy.swap(j, j + 1)`, which DSP-1 forbids.
  * The groups were found by allowing `clippy::style`, then `clippy::complexity`, and seeing which lints were left.  Clippy's lint list gives the default levels: correctness is `deny`; suspicious, style, complexity and `perf` are `warn` ([Clippy lints](https://clippy.rust-lang.org/stable/index.html)).
  * The four crate-level `#![expect]` attributes in "M6 design" pass these:
    * `cargo fmt -p dsp --check`, `cargo clippy -p dsp --all-targets -- -D warnings` and the workspace clippy command;
    * `RUSTFLAGS="-D warnings" cargo test -p dsp` (30 tests, no doctests);
    * `RUSTDOCFLAGS="-D warnings" cargo doc -p dsp`;
    * `RUSTFLAGS="-D warnings" cargo build -p dsp --target wasm32-unknown-unknown --release`, as Trunk builds it.
  * So `rustc` and `rustdoc` without clippy do not report the clippy expectations as unfulfilled.  The Reference says only that tool lints "only get checked when the associated tool is active" ([diagnostics attributes](https://doc.rust-lang.org/reference/attributes/diagnostics.html)).  So this fact rests on the runs.
  * Planted on the prototype: `let n = n;` in `fade_in` fails with `clippy::redundant_locals`, which is in the suspicious group, not correctness.  `n == n` fails with `clippy::eq_op`, which is in the correctness group.  On today's tree, with `#![allow(clippy::all)]`, both pass.
  * A planted `dsp/clippy.toml` with `disallowed-methods = [{ path = "std::vec::Vec::new", reason = "planted" }]` fails clippy with 4 errors in the lib and 9 in the lib test.  `vec![]` expands to `Vec::new`, so the test count holds three `vec![]`s.  `disallowed_methods` is in the style group: `-A clippy::style` silences it.
  * When `median`'s swap is replaced by one assignment, clippy fails with "this lint expectation is unfulfilled" at the `manual_swap` line.  A rewrite with another temporary name still fires `manual_swap`.
  * `#![allow(clippy::all)]` after the four `expect`s fails with four unfulfilled expectations.  The same allow before them passes: the later `expect`s set the level of their four lints again, and the allow silences every other lint.  A blanket allow that replaces the `expect`s also passes.  The Reference: an expectation "is only fulfilled by lint emissions which have been suppressed by the `expect` attribute".
  * Other attributes that pass clippy with `n == n` planted:
    * `#![expect(clippy::all, reason = "...")]` in place of the four.  A style lint fulfils it, and it silences `eq_op`.
    * `#[expect(clippy::eq_op, reason = "...")]` on `fade_in`.
  * `[workspace.lints.clippy]` with `correctness` and `suspicious` at `forbid`, `priority = -1`, does not stop a later `#![allow(clippy::all)]`.  It only warns (`forbidden_lint_groups`), and `n == n` still passes under `-D warnings`.  So no cheap guard exists in Cargo.
  * Cargo cannot hold the list.  `[lints.clippy]` beside `workspace = true` fails: "cannot override `workspace.lints` in `lints`".  Its levels are `forbid`, `deny`, `warn` and `allow`, and `"expect"` fails with "unknown variant" ([manifest, lints](https://doc.rust-lang.org/cargo/reference/manifest.html)).

#### M6 design

* Review round 1 widened the ban to every compound assignment (see the Decision log).  The texts in the tree replace the quoted texts below.
* Decision point for the maintainer.  Ask before step 1.  Record the choice in Progress and the Decision log.  Add "Decided on <date>: ..." to the question.  If the choice is not the recommended one, make the changes listed under it.  Then review the plan again before implementing.
* Question: which lints does `dsp` keep, and which does it fix?  Decided on 2026-10-20: the maintainer chose (x), after weighing a mixed option (z) that fixed only `assign_op_pattern`.
  * (x), recommended: keep all four, each as an `expect` with a reason.  DSP-1 names the beginner forms and forbids their shortcuts, so each reason cites DSP-1's text.  No function body changes.  The gain: the code that a beginner reads stays as it is.  M7's allowlist needs no compound assignment, `is_empty` or `clamp`.  The cost: four exceptions, and DSP-1 gains a sentence that forbids `+=`, `-=`, `is_empty` and `clamp`.
  * (y) fix three, and keep only `manual_swap`.  The gain: one exception, which DSP-1's text already forces, and no new sentence in DSP-1.  The cost: the beginner text changes in 7 files, and M7's allowlist must permit `+=`, `-=`, `is_empty` and `clamp`.  For (y), make these changes:
    * Change the 13 sites of `x = x + ...` and `p = p - length` to `+=` and `-=`.
    * Change the two `len() > 0` to `!...is_empty()`.
    * Change the two `if`s in `mix` to `total = total.clamp(-1.0, 1.0);`.  `mix`'s doc says that the total "is clamped", so the doc stays.
    * `dsp/src/lib.rs` keeps only the `manual_swap` `expect`, and the comment says "the lint below".
    * DSP-1 gets these two sentences in place of the sentence on clippy: "`dsp/src/lib.rs` expects each clippy lint that a DSP-1 construct triggers, by name and with a reason.  Every other lint keeps its workspace level."  DSP-1's "Why" stays.
    * DSP-1's `* Enforced by:` and `* Gaps:` lines change as under (x).
    * `.claude/rules/dsp.md` gets only the second new bullet.
    * M7's first bullet permits `clamp`; remove "leave out `clamp` (from M6)".
    * TD-3 does not name `+=`, `-=`, `is_empty` and `clamp`.
    * TD-2's closed bullet says "one `#![expect]`".  The `harness.md` row says "the `#![expect]` in `dsp/src/lib.rs`".
    * "Why crate-level `expect`" says 1 site, not 17.
    * In "M6 acceptance": remove the planted breaches of the three fixed lints.  "Each of the four is needed" becomes "the one `expect` is needed".  "Four unfulfilled expectations" becomes "one".  "In place of the four" becomes "in place of the one".
    * M7's bullet "Forbidden shortcuts" goes, and M7's allowlist permits `+=`, `-=`, `is_empty` and `clamp`.
    * Decision 0006 records (y).  Its Consequences drop the ban and M7's shortcut line, and name the one `expect`.
* Decision record: `docs/decisions/0006-clippy-exceptions-in-dsp.md`.  Under (x), DSP-1 gains a ban, which is a rule change.  So the choice gets a record, as M11's rule change does.
  * Title: `# 0006: how dsp meets clippy`.
  * Status line: `Status: accepted (<date>, by the maintainer)`.  The maintainer's answer at the decision point accepts the record.
  * The line in `docs/README.md`: `` * `decisions/0006-clippy-exceptions-in-dsp.md`: `dsp` keeps its beginner forms (DSP-1), and `dsp/src/lib.rs` expects each clippy lint that they trigger, by name and with a reason. ``  Under (y): `` * `decisions/0006-clippy-exceptions-in-dsp.md`: `dsp/src/lib.rs` expects each clippy lint that a DSP-1 construct triggers, by name and with a reason. ``
  * Context: "`dsp/src/lib.rs` had `#![allow(clippy::all)]`, so no clippy lint ran on `dsp`.  Four lints fire on its beginner code: `assign_op_pattern`, `len_zero`, `manual_clamp` and `manual_swap`.  DSP-1 forbids only the fix of `manual_swap`."
  * Options: (x) and (y), each with its gain and cost, as the question above gives them.
  * Decision: the maintainer's choice and its date.
  * Consequences, one sentence each.  Under (x):
    * "DSP-1 forbids `+=`, `-=`, `is_empty` and `clamp` in `dsp`."
    * "`dsp/src/lib.rs` expects the four lints; every other lint keeps its workspace level."
    * "A new lint in `dsp` is fixed with DSP-1's constructs, or gets an `expect` whose reason cites DSP-1."
    * "M7's allowlist fails the forbidden shortcuts."
  * `harness.md`'s sentence on decisions names 0006 after 0005, because 0006 sets how a sensor's exceptions work, as 0005 sets how strictly a sensor reads a rule.
* `dsp/src/lib.rs`: lines 6 to 9 (the comment and the allow) become:

  ```rust
  // Clippy suggests shortcuts that a beginner does not know yet.
  // Each lint below fires on a form that DSP-1 keeps on purpose.
  // All other lints keep their workspace level.  If a lint below
  // stops firing, clippy warns, and `-D warnings` (as in CI) fails.
  // Then remove that lint from the list.
  #![expect(
      clippy::assign_op_pattern,
      reason = "DSP-1: `x = x + 1` and `x = x - 1` show the sum in full"
  )]
  #![expect(
      clippy::len_zero,
      reason = "DSP-1: `len() > 0` needs only `len`, not `is_empty`"
  )]
  #![expect(
      clippy::manual_clamp,
      reason = "DSP-1: an `if` for each bound shows what a clamp does"
  )]
  #![expect(clippy::manual_swap, reason = "DSP-1 forbids `swap`")]
  ```

  rustfmt sets the line breaks.  The module doc comment (lines 1 to 4) stays.
* Why crate-level `expect`, not `allow` and not one attribute at each site:
  * `expect` fails when its lint stops firing, so a stale exception cannot stay (phase plan, "which fails when the exception goes stale").
  * One list at the crate root keeps 17 attributes, one at each site, out of the code that a beginner reads.  DSP-1 covers the whole crate, so the scope is the same.
  * Each `expect` must fire in the lib target, not only in tests.  Otherwise `--all-targets` fails the lib build with an unfulfilled expectation.  All four fire in lib code today.
  * Cargo's `[lints]` cannot hold the list (see the facts above).
* The rule for a new lint in `dsp`: fix it with DSP-1's constructs if it can be fixed so.  If every fix needs a construct outside DSP-1, add an `expect` with a reason that cites DSP-1.  If DSP-1's text does not name the form, adding it changes the rule.  So write a `proposed` decision record, and ask the maintainer.  A toolchain bump in CI can bring a new lint, because clippy then checks `dsp` like every other crate.  The bump is deliberate, so the PR that bumps it handles the new lint.
* M6 adds no unit test of its own.  This departs from "Each check has a unit test with a planted breach" in "What every milestone does".  The sensor is clippy, so the planted breaches are manual, and the PR records them.
* The gap that stays: these settings switch lints off in `dsp` and pass clippy.
  * `#![allow(clippy::all)]` before the `expect`s, or in their place.
  * `#![expect(clippy::all, reason = "...")]`, or an `expect` of another group.  A lint attribute with `warnings` passes too, but only for warn-level lints: `eq_op` is deny-level, and still fails.
  * Any other lint attribute: on an item, at the top of a module file, on a statement, or at crate level inside `cfg_attr`.
  * A `[lints]` table in `dsp/Cargo.toml` in place of `workspace = true`, for example `[lints.clippy] all = "allow"`.
  * M7 closes these, and is the next PR.  M7 parses the same files with `syn`, so it would replace a check in M6 for the attributes alone.  Cargo gives no cheap guard (see the facts above).  Until M7 merges, TD-3 names the gap.
  * M7 does not close one more: `-A clippy::all` in `RUSTFLAGS` or in a `.cargo/config.toml`.  It switches clippy off for every crate, so a reviewer sees it in the diff.  TD-3 names it.
* `invariants.md`, DSP-1, under (x):
  * The sentence "Clippy's style lints are relaxed there on purpose." becomes three sentences.  The first: "Write `x = x + 1`, `x = x - 1`, `len() > 0` and an `if` for each bound of a clamp, not `+=`, `-=`, `is_empty` or `clamp`."  Then: "`dsp/src/lib.rs` expects each clippy lint that these forms, or a swap written out, trigger.  Each has a reason, and every other lint keeps its workspace level."  These name no lint, so `invariants.md` does not repeat the list.
  * The "Why" becomes: "`dsp/` is a first reading exercise in Rust; it must stay readable without knowing the iterator API, or shortcuts such as `+=`, `is_empty` and `clamp`."
  * `* Enforced by:` becomes "prose only.  A scan on 2026-10-19 found no iterator, closure or `swap`."  It still starts with "prose only", so the count of "prose only" lines stays 7.  Clippy checks the code's lints, not DSP-1's constructs.
  * `* Gaps:` becomes `* Gaps: TD-3, TD-<n>.`, where TD-<n> is the dsp types entry below.
* `tech-debt.md`:
  * Remove `### TD-2` and its text.  Add under `## Closed`, after TD-1: "* TD-2: closed by four `#![expect]`s in `dsp/src/lib.rs`, one for each lint, with a reason (harness phase 2, M6).  Every other clippy lint keeps its workspace level in `dsp`.  With `-D warnings`, an `expect` whose lint stops firing fails clippy (DSP-1)."  `docs-check` then fails any `TD-2` left outside plans and decisions.
  * TD-3's text becomes three sentences.  "Nothing fails when `dsp/` gains an iterator, a closure, a dependency, `+=`, `-=`, `is_empty` or `clamp`, or a lint setting that switches clippy off."  "These can pass clippy: a lint attribute that switches a lint off, other than the `expect`s in `dsp/src/lib.rs`, and a `[lints]` table in `dsp/Cargo.toml`."  "A `-A clippy::all` in `RUSTFLAGS`, or in a `config.toml` in `.cargo/`, passes too, for every crate, and M7 does not check it."  `docs-check` reads code spans in `tech-debt.md` as paths, so the text must not hold a path to a `.cargo/` file that does not exist.
  * TD-3's fix becomes: "a structural test in `xtask/tests/` that parses `dsp/src` and allows only the listed constructs (an allowlist, not a denylist).  It accepts lint attributes only as crate-level `#![expect]`s in `dsp/src/lib.rs`, of the lints on a list in the check, each with a reason.  It asserts that `dsp/Cargo.toml` has no dependencies, and a `[lints]` table with only `workspace = true` (`plans/active/harness-phase-2.md`, M7)."
  * The dsp types entry: a new entry at the end of `## Known breaches`, with the next free ID.  Today the highest ID is TD-23, so it is the next after TD-23.  Check the file first.  Title: "dsp uses types outside DSP-1's list".  Text: "DSP-1 lists only `&[f32]`, `Vec<f32>` and `usize`.  `biggest` and `median` return `f32`, `peaks` returns `Vec<(f32, f32)>`, `mix` takes `&[&[f32]]`, and `median` has a `bool`."  Fix: "M7 decides each type: widen DSP-1's list, or change the code (`plans/active/harness-phase-2.md`, M7)."
* `.claude/rules/dsp.md`, two bullets after the swap bullet:
  * "Write `x = x + 1`, `x = x - 1`, `len() > 0` and an `if` for each bound of a clamp, not `+=`, `-=`, `is_empty` or `clamp`."
  * "Fix a clippy lint with DSP-1's constructs.  If every fix needs another construct, add the lint to the `#![expect]` list in `dsp/src/lib.rs`.  Give a reason that cites DSP-1.  The lint must fire in lib code, not only in tests; otherwise fix it.  If DSP-1 does not name the form, write a `proposed` decision record, and ask the maintainer."
  * Its "Use only ..." bullet stays: it states the rule, and the dsp types entry records the breach.
* `harness.md`, one components row after M5's row: component "the `#![expect]`s in `dsp/src/lib.rs`, one for each lint", kind "sensor, computational", added "phase 2".  Failure cell: "`#![allow(clippy::all)]` in `dsp` switched off every clippy lint, so a correctness lint, or a disallowed method, would have passed there".
* `harness-engineering.md`, phase 2's clippy bullet: "Narrow `dsp`'s `allow(clippy::all)` (TD-2)." becomes "Narrow `dsp`'s `allow(clippy::all)` (TD-2, M6).", as M4 and M5 tagged their bullets.
* No change to `CLAUDE.md` (the DSP-1 line stays true), `ARCHITECTURE.md`, the guide (the work of a person does not change), or CI.
* M7's section already has the bullets "Lint attributes (from M6)", "Lints table (from M6)", "Forbidden shortcuts (from M6)" and "Types (from the M6 survey)", added with this plan.  Step 2 confirms that they still hold.

#### M6 steps

Step 2 is one signed commit, and fmt, clippy (every clippy command in `CLAUDE.md`), `cargo test --workspace --exclude roformer-gpu`, `cargo test -p roformer-gpu --lib` and `--doc`, and `docs-check` pass before it (GIT-2).  Fixes from the review rounds are later commits.

1. Before the change, record what today's tree lets through.  This step has no commit.
   * Plant `let n = n;` and `let _same = n == n;` after `let n = samples.len();` in `fade_in`.  `cargo clippy -p dsp --all-targets -- -D warnings` passes.
   * Remove the two plants.  Then plant the `dsp/clippy.toml` of "M6 state at the start".  Clippy passes again.
   * Record both results in the PR, and restore the files.
2. The change and its docs, in one commit, because the change makes DSP-1 and TD-2 wrong (DOC-1):
   * `dsp/src/lib.rs`, as "M6 design" gives it, for the maintainer's choice.
   * Decision 0006, and its line in `docs/README.md`.
   * `invariants.md`, `tech-debt.md`, `.claude/rules/dsp.md`, `harness.md` and `harness-engineering.md`, as "M6 design" gives them.
   * In this plan, replace "the dsp types entry" with its ID in M7's heading, in M7's "Types" bullet and in "Idempotence and recovery".  `docs-check` fails an ID that no doc defines, so this waits until the entry exists.
   * Confirm that M7's four bullets from M6 still hold.
   * Run the planted breaches in "M6 acceptance", and record each result in the PR.
   * Update this plan's Progress, and its other living sections where they changed (`plans/PLANS.md`).

#### M6 acceptance

* `cargo clippy -p dsp --all-targets -- -D warnings` passes, and so does the workspace clippy command.
* `RUSTFLAGS="-D warnings" cargo test -p dsp` passes its 30 tests, so `rustc` alone does not fail on the clippy expectations.  `RUSTFLAGS="-D warnings" cargo build -p dsp --target wasm32-unknown-unknown --release` and `RUSTDOCFLAGS="-D warnings" cargo doc -p dsp` pass.
* Planted breaches, run locally and not kept.  Each fails `cargo clippy -p dsp --all-targets -- -D warnings` as stated.  The first three passed before the change (step 1):
  * `let n = n;` in `fade_in`: `clippy::redundant_locals`.
  * `let _same = n == n;` in `fade_in`: `clippy::eq_op`.
  * The planted `dsp/clippy.toml`: `clippy::disallowed_methods`, 4 errors in the lib and 9 in the lib test.
  * Each `expect` removed in turn: its lint fails at its sites.  So each of the four is needed.
  * `median`'s three swap lines replaced by `copy[j] = copy[j + 1];`: "this lint expectation is unfulfilled" at the `manual_swap` line.
  * `#![allow(clippy::all)]` after the `expect`s: four unfulfilled expectations.
* The known gap, with `n == n` planted.  Each of these passes, and stays in TD-3 until M7.  Record each in the PR:
  * `#![allow(clippy::all)]` before the `expect`s;
  * `#![expect(clippy::all, reason = "x")]` in place of the four;
  * `#[expect(clippy::eq_op, reason = "x")]` on `fade_in`;
  * `#[allow(clippy::eq_op)]` on the planted `let`;
  * `[lints.clippy] all = "allow"` in place of `[lints] workspace = true` in `dsp/Cargo.toml`.
* `grep -c "Enforced by: prose only" docs/invariants.md` still gives 7.
* fmt, clippy (every clippy command in `CLAUDE.md`), `cargo test --workspace --exclude roformer-gpu`, `cargo test -p roformer-gpu --lib` and `--doc`, and `docs-check` pass.  `docs-check` finds no `TD-2` outside plans and decisions.
* CI: the `clippy` job passes with no new step.

### M7: dsp's beginner style is a test (narrows TD-3, closes TD-24; DSP-1, DOC-1)

* Scope, set by the maintainer on 2026-10-20: a small denylist of the likely breaches, not a full allowlist.  `dsp` has 453 lines and 3 commits, and nothing has broken DSP-1.  The likely breach is an agent that tidies `dsp` into idiomatic Rust.  So the check reuses M3's `proc-macro2` tokenizer, and adds no crate and no decision record.
* What M6 does not catch: clippy has no lint against iterators, closures or `swap`, so `samples.iter().map(...)` passes it.  One added `+=` passes, because an `expect` fails only when every site of its form is gone.  A lint attribute that switches clippy off, or a `[lints]` table in `dsp/Cargo.toml`, passes too (TD-3).

#### M7 design

* The check: `pub fn dsp_keeps_beginner_style(files: &[(String, String)]) -> Vec<Problem>` in `xtask/src/rules.rs`.  The input is every file under `dsp/`, as a path relative to the root and its text.  A reader `dsp_files(root)` walks `dsp/` with `std::fs`.  The test `dsp_is_beginner_style` in `xtask/tests/rules.rs` runs both on the real tree.  It asserts that the files include `dsp/Cargo.toml`, `dsp/src/lib.rs` and `dsp/src/mix.rs`, so an empty walk cannot pass.
* Each `.rs` file in `dsp/src/` is tokenised with `proc_macro2::TokenStream::from_str`, as `skips_in` does.  The scan enters every group, so macro arguments are read too.  A problem names DSP-1, the file, the line and the fix.  A problem fails on these:
  * a method other than `len`, `push` or `capacity`: an identifier after a `.` that is not part of `..`.  So `iter`, `to_vec`, `sort_by`, `min`, `max`, `abs`, `swap`, `is_empty` and `clamp` fail.  This one rule is an allowlist, because the methods a tidy-up reaches for are many.  A raw identifier (`r#iter`) counts as its name.  Fix: write the loop with `while` and indexing, `len() > 0`, an `if` for each bound, or a swap through `temp`.
  * a path segment `iter`, `into_iter`, `swap`, `min`, `max` or `clamp` after `::`, for example `std::mem::swap` or `f32::max`.  A name first in an import's braces, or after a `,` there, counts too, as in `use std::mem::{replace, swap}`.  A name straight after a module of the same name, as in `pub use max::max`, is `dsp`'s own function and passes.  Fix: as above.
  * a closure: a `|` that is not part of `||` and not followed by a joint `=`, or the keyword `move`.  Fix: write the loop out.
  * a compound assignment: `=` straight after a joint `+`, `-`, `*`, `/`, `%`, `^`, `&` or `|`, or after `<<` or `>>`.  Fix: write `x = x + y`.
  * `for` whose expression has no `..` at its top level, outside any group, before its `{` block.  So `for s in samples`, `for s in &samples[1..]` and `for i in (0..n).rev()` fail, and `for i in 0..n` passes.  Fix: loop over a range of indices.
  * an attribute, outer or inner, whose name is `allow`, `warn`, `deny`, `forbid`, `expect`, `cfg_attr` or `path`, and the macro `include!`.  The exception: the `#![expect(clippy::<lint>, reason = "DSP-1...")]` attributes in `dsp/src/lib.rs`, one lint each, from `DSP1_LINTS` (`assign_op_pattern`, `len_zero`, `manual_clamp`, `manual_swap`).  Fix: fix the lint with DSP-1's constructs, or add the lint to the `#![expect]` list and to `DSP1_LINTS`, with a reason.
  * a file under `dsp/` other than `Cargo.toml` and `.rs` files in `src/`, such as `build.rs` or `clippy.toml`.  The walk skips `.DS_Store`, which Finder makes.  Fix: remove it.
  * `dsp/Cargo.toml` that differs from `DSP_MANIFEST`, a constant with today's text.  So any dependency table and any `[lints]` override fail.  Fix: restore the text.
  * a file that does not tokenise.
* The check reports at most one problem for each rule on a line, so `|x| x` gives one problem, not two.
* Facts from a prototype with `proc-macro2` 1.0.107: `+=` is `+` (joint) then `=`.  `<=` is `<` (joint) then `=`, and `<<=` is two joint `<` then `=`.  `||` is a joint `|` then `|`; `|x|` has two lone `|`.  `#![allow(...)]` is a joint `#`, `!`, and a bracket group.  A doc comment is `#` and a bracket group that starts with `doc`.
* TD-24: DSP-1's text lists the types that the code writes, plus `bool`: `f32`, `usize`, `bool`, `&[f32]`, `&[&[f32]]`, `Vec<f32>` and `Vec<(f32, f32)>`.  This corrects the text to match the code, so no decision record.  The check does not read types.
* What the check does not see stays in TD-3, narrowed: types; a closure with no arguments (`|| x`, which tokenises as `||`); and constructs not on the denylist, such as `match` or `loop`.  Also `-A clippy::all` in `RUSTFLAGS`, in a `config.toml` in `.cargo/`, or in `[workspace.lints]`, for every crate.  Fix: review; no check planned.
* Docs (DOC-1), in the commit that adds the real-tree test:
  * `invariants.md`, DSP-1: the first sentence names the seven types and "`while`, `for` over a range, `if` and indexing".  `* Enforced by:` becomes "`dsp_is_beginner_style` (CI `test` job), a scan of the tokens in `dsp/` for the likely breaches.  Clippy fails a stale `#![expect]`."  `* Gaps: TD-3.`  "Prose only" lines fall from 7 to 6.
  * `tech-debt.md`: TD-3 narrowed as above.  TD-24 removed, and added under `## Closed`: "TD-24: closed by DSP-1's new list of types (harness phase 2, M7)."
  * `.claude/rules/dsp.md`: the types in the first bullet, and one bullet that names the test.
  * `CLAUDE.md`, the DSP-1 line: "`dsp/` is beginner style: plain types, `while`, `for` over a range, `if` and indexing; no iterators, closures or `swap`."
  * `harness.md`: a components row for `dsp_is_beginner_style`.

#### M7 steps

Each step is one signed commit; fmt, every clippy command in `CLAUDE.md`, the tests and `docs-check` pass before it (GIT-2).

1. Test first: `dsp_files`, a stub of the check that returns no problem, and the unit tests.  Every test that expects a problem fails against the stub.  Then write the check, and all pass.  Unit tests:
   * `beginner_code_passes`: a file with `x = x + 1`, `x = -1`, `a <= b`, `a >= b`, `a == b`, `a != b`, `a && b`, `a || b`, `for i in 0..n`, `vec![0.0; n]`, `.len()`, `.push(x)`, `.capacity()`, `Vec::new()`, `stems[s][i]`, doc comments and the four `#![expect]`s in `lib.rs`.  No problem.
   * `each_breach_is_named`: a table of snippets, each giving exactly one problem with DSP-1 and its line: `.iter()`, `.to_vec()`, `.sort_by(f)`, `.min(x)`, `r#iter`, `.swap(a, b)`, `std::mem::swap(a, b)`, `f32::max(a, b)`, `.is_empty()`, `.clamp(a, b)`, `|x| x`, `move`, each of the ten compound assignments, `for s in samples {}`, `for s in &samples[1..] {}`, `for i in (0..n).rev() {}`, `.iter()` inside `assert!`, `#![allow(clippy::all)]`, `#![expect(clippy::all, reason = "x")]`, `#![expect(clippy::eq_op, reason = "DSP-1")]`, `#[allow(clippy::eq_op)]` on a `let`, `#![allow(clippy::eq_op)]` as the first line of a body, `#![cfg_attr(clippy, allow(clippy::all))]`, a listed `#![expect]` in a module file, `#[path = "x.rs"]` and `include!("x.rs")`.
   * `the_manifest_and_files_are_fixed`: `[dev-dependencies]` with an entry, `[lints.clippy] all = "allow"`, `dsp/build.rs` and `dsp/clippy.toml` each give one problem.
2. `dsp_is_beginner_style` on the real tree, and the docs.  Plant `samples.iter().count();` in `biggest`, `i += 1` in `double_speed` and `#![allow(clippy::all)]` in `lib.rs`, one at a time; each fails the test with DSP-1, the file and the line.  Restore each, and record the results in the PR.

#### M7 acceptance

* The unit tests pass, and failed against the stub in step 1.
* `dsp_is_beginner_style` passes on the real tree, and the CI `test` job lists it as `ok`.  The three planted breaches fail it.
* `grep -c "Enforced by: prose only" docs/invariants.md` gives 6.  `Cargo.lock` does not change.
* fmt, clippy, the tests and `docs-check` pass; `docs-check` finds no `TD-24` outside plans, decisions and the `## Closed` bullet.

### M8: Python and assets are tests over the tracked files (closes TD-5, TD-6; BUILD-1, ASSET-1, DOC-1)

* Scope: small checks, by the rule for proportionate sensors (the parent plan's Decision log, 2026-10-20).  Nothing has broken BUILD-1 or ASSET-1, and no commit in the history adds Python or audio.  The likely breaches: an agent adds a helper script in Python, or a CI step that installs Python.  Or a person or agent runs `git add -f` on a song or a model file.  So M8 adds no crate, no decision record and no allowlist.

#### M8 state at the start

* BUILD-1 is "prose only" (TD-5).  The one tracked `.py` file is `tools/roformer_export.py`.  No tracked build file names Python: CI, Dependabot and the `setup-trunk` action in `.github/`, `app/Trunk.toml`, and no `build.rs` exists.
* ASSET-1 rests on `.gitignore` (TD-6).  It lists `*.mp3`, `*.wav`, `*.safetensors` and `*.ckpt`, and the folders `dist/`, `pages/`, `audio/`, `stems/` and `third_party/`, none anchored.
* 151 files are tracked.  The largest is `app/tests/snapshots/spectrograms_classic_drums.png`, 486,664 bytes; next are `Cargo.lock` (246,510) and this plan (223,066).  The selftest fixtures in the ignored `third_party/roformer/selftest/` are 18 and 24 MB.
* On the maintainer's Mac, `git status --ignored` lists only `app/dist/`, `target/` and `third_party/`.  The folders that people or the code use are `app/dist/` (Trunk), `target/` (Cargo, and `target/pages/` for `xtask publish`), `audio/synth/` (`xtask gen-song`) and `stems/` (the input example for `app/tests/ui.rs`'s `STEMS_DIR`), each at the root.  Nothing writes `pages/` at the root.  So the first M8 text's anchored `/dist/` is wrong: it would stop ignoring `app/dist/`.  The anchored form is `/app/dist/`.
* `git ls-files --format` with `%(objectsize)` reads each file's size from the index, so it needs no read of the working tree.  It came in git 2.42 ([2.42.0 docs](https://git-scm.com/docs/git-ls-files/2.42.0); the 2.41 page redirects to 2.40, which lacks the field).  The Mac has git 2.50.1.  The `ubuntu-latest` image (Ubuntu 24.04) has git 2.55.0 ([runner image](https://github.com/actions/runner-images/blob/main/images/ubuntu/Ubuntu2404-Readme.md)).  With `-z`, each entry ends in NUL and no path is quoted (checked on the Mac).

#### M8 design

* The reader: `pub fn tracked_files(root: &Path) -> Result<Vec<TrackedFile>>` in `xtask/src/rules.rs`.  It runs `git -C <root> ls-files -z --format=%(objectsize) %(path)`, and a pure `parse_ls_files(&str) -> Result<Vec<TrackedFile>>` reads the output.  `TrackedFile` has `path: String` and `size: u64`.  A failed `git` run is an error that holds git's stderr and says the check needs git 2.42 or later.  A size that is not a number (`-` for a submodule) is an error that names the path.  The index is what the next commit holds, so a file staged with `git add -f` is seen before the commit.
* BUILD-1: `pub fn python_in_the_build(tracked: &[TrackedFile], texts: &[(String, String)]) -> Vec<Problem>`.  `texts` holds each tracked file that reads as UTF-8, from the working tree.  A problem names BUILD-1, the file, the line if any, and the fix.
  * A tracked path, compared without case, that ends in `.py`, `.pyc` or `.ipynb`, or whose file name is `requirements.txt`, `pyproject.toml`, `Pipfile` or `uv.lock`.  The one exception is the exact path `tools/roformer_export.py`, compared with case.  Fix: "write it in Rust, as an `xtask` command; Python appears only in `tools/roformer_export.py`".
  * In a build file, the word `python`, `python3`, `pip`, `pip3`, `pipx`, `uv` or `uvx`, without case.  A word is a run of letters, digits and `_`, so `setup-python` and `python3.12` hold one, and `pipefail` and `pipeline` hold none.  One problem for each line.  Fix: "use Cargo, Trunk or an `xtask` command".
  * `pub fn is_build_file(path: &str) -> bool` chooses the build files: a path under `.github/`, a file named `Trunk.toml` or `build.rs`, and a file named `config.toml` or `config` in any `.cargo/` folder.  Cargo reads the one in `app/` too, when Trunk builds there.  `pub fn tracked_texts(root: &Path, tracked: &[TrackedFile]) -> Result<Vec<(String, String)>>` reads each tracked file from the working tree.  It skips a file that the working tree has deleted, since a deletion adds no Python.  It skips a file that is not UTF-8, such as a PNG, and a tracked link to a folder.
  * In any tracked text file, the module name of Python's built-in web server (`http` and `.server` joined) or its Python 2 name (`SimpleHTTP` and `Server` joined), matched with case, as Python matches module names.  That server is what an agent suggests to serve a wasm build.  Files under `docs/plans/` and `docs/decisions/` are history and are left out.  `xtask/src/rules.rs` builds each name with `concat!`, so its own text never holds the joined name, and it needs no exception.  One problem for each line.  Fix: "serve with `cargo run -p xtask -- serve`".  The plan and the docs write these names in two parts, so that no doc outside `docs/plans/` holds them.
  * Out of scope: Rust code outside `build.rs` that starts Python, such as a `Command::new` in `xtask`; a shell script that runs Python; a script with a Python shebang and no extension; and suggestions in chat.  BUILD-1's `* Enforced by:` line says so, as GPU-1's line names what its test cannot see.
* ASSET-1: `pub fn assets_in_the_index(tracked: &[TrackedFile]) -> Vec<Problem>`.  The two lib names differ from the real-tree tests' names, as `dsp_keeps_beginner_style` differs from `dsp_is_beginner_style`.
  * A tracked path whose extension (the text after its last `.`), compared without case, is an audio extension (`mp3`, `wav`, `flac`, `ogg`, `opus`, `m4a`, `aac`, `aif`, `aiff`) or a weight extension (`safetensors`, `ckpt`, `pt`, `pth`, `bin`, `onnx`, `npy`, `npz`, `gguf`).  Fix: "remove it from the index with `git rm --cached '<path>'`.  Keep it in an ignored folder (`audio/`, `stems/` or `third_party/`).  Link to its source."
  * A tracked file over `MAX_TRACKED_BYTES`, 1,048,576 bytes (1 MiB).  That is about twice the largest file today.  A 3-minute MP3 at 128 kbit/s is about 2.9 MB, and each selftest fixture is over 18 MB.  So no allowlist is needed.  Fix: "if it is audio or weights, remove it as above; else raise `MAX_TRACKED_BYTES` in `xtask/src/rules.rs`, with a reason in the PR".
  * Out of scope: an audio file under another extension and under 1 MiB, and files in the history.  ASSET-1's `* Enforced by:` line says so.
* `.gitignore`: anchor the folders at the root (`/app/dist/`, `/pages/`, `/audio/`, `/stems/`, `/third_party/`).  Add the extensions that the check has and `.gitignore` lacks, so a normal `git add` leaves them out too.  `target/` stays unanchored, since Cargo makes it in any folder where it runs.
* Docs (DOC-1), in the commit that adds the real-tree tests:
  * `invariants.md`, BUILD-1: `* Enforced by:` becomes "`no_python_in_the_build` (CI `test` job), over the git index.  It fails a tracked Python file other than `tools/roformer_export.py`.  It fails the words `python`, `pip` or `uv` in CI, Trunk, `build.rs` or Cargo's config.  It fails the name of Python's web server in a tracked text file outside plans and decisions.  It does not see Rust code outside `build.rs` that starts Python, or a shell script that runs Python.  It also does not see a script with a shebang and no extension, or a server in another language."  The `* Gaps:` line goes.  "Prose only" lines fall from 6 to 5.
  * `invariants.md`, ASSET-1: `* Enforced by:` becomes "`no_audio_or_weights_are_tracked` (CI `test` job), over the git index, and `.gitignore`.  It fails an audio or weight extension, and a file over 1 MiB.  A short clip under another extension passes.  It does not read the history."  The `* Gaps:` line goes.
  * `tech-debt.md`: TD-5 and TD-6 removed, and under `## Closed`: "TD-5: closed by `no_python_in_the_build` (harness phase 2, M8)." and "TD-6: closed by `no_audio_or_weights_are_tracked` and the anchored `.gitignore` (harness phase 2, M8)."
  * `harness.md`: one components row for the two tests.
  * `README.md`: the "Build the native app, `cargo test`, `cargo clippy`" row adds "git 2.42 or newer".  The `.gitignore` sentence under "Setting up a machine" names the test too.

#### M8 steps

Each step is one signed commit; fmt, every clippy command in `CLAUDE.md`, the tests and `docs-check` pass before it (GIT-2).

1. Test first: `TrackedFile`, `parse_ls_files`, `tracked_files`, `tracked_texts`, `is_build_file`, stubs of `python_in_the_build` and `assets_in_the_index` that return no problem, and the unit tests.  Every test that expects a problem fails against the stubs.  Then write the functions, and all pass.  Unit tests:
   * `ls_files_output_is_parsed`: two NUL-ended entries, one path with a space; a `-` size and a missing space are errors.
   * `build_files_are_chosen`: `.github/workflows/ci.yml`, `.github/dependabot.yml`, `app/Trunk.toml`, `engine/build.rs`, `config.toml` and `config` in the root `.cargo/` folder, and `config.toml` in `app/.cargo/` are build files.  `README.md`, `xtask/src/serve.rs` and `tools/roformer_export.py` are not.
   * `python_files_are_named`: `tools/x.py`, `Tools/X.PY`, `a.pyc`, `a.ipynb`, `requirements.txt`, `pyproject.toml`, `Pipfile`, `uv.lock`, `engine/tools/roformer_export.py` and `tools/ROFORMER_EXPORT.PY` each give one BUILD-1 problem with the path.  `tools/roformer_export.py` gives none.
   * `python_in_a_build_file_is_named`: each gives one problem with its line: `run: pip install numpy`, `uses: actions/setup-python@v5`, `package-ecosystem: "pip"` in `.github/dependabot.yml`, `command = "python3"` in `app/Trunk.toml`, `Command::new("python")` in `engine/build.rs`, `runner = "pip3"` and `uvx ruff` in `config` in `.cargo/`, `- name: Set up Python` (capital P), `run: uv sync`, `pipx run x`, and `python -m pip` on one line.
   * `clean_build_files_pass`: `set -o pipefail`, `pipeline`, `snippet` and `Pythonic` in `ci.yml` give no problem.  A `pip` in `README.md` gives none, since it is not a build file.
   * `a_python_server_is_named`: `python3 -m http` + `.server 8080` in `README.md`, the Python 2 name in `xtask/src/serve.rs`, and the first name in `.github/workflows/ci.yml` each give one problem with its line.  The same text in `docs/plans/x.md` and `docs/decisions/x.md` gives none.  (The test builds the names with `concat!`.)
   * `assets_are_named`: `song.flac`, `a/Model.SafeTensors`, `bs_roformer_sw.f32.safetensors`, `weights.bin` and `x.npy` each give one ASSET-1 problem.  `engine/src/wav.rs`, `xtask/src/bin/x.rs`, `docs/prompt` and `app/tests/snapshots/six_lanes.png` give none.
   * `a_file_over_the_cap_is_named`: 1,048,577 bytes gives one problem with the size; 1,048,576 gives none.
2. The real-tree tests `no_python_in_the_build` and `no_audio_or_weights_are_tracked` in `xtask/tests/rules.rs`, `.gitignore` and the docs.  The tests share one `tracked_files` run.  They assert that the list holds `Cargo.toml`, `tools/roformer_export.py` and `app/tests/snapshots/six_lanes.png`, and that the texts hold `.github/workflows/ci.yml` and `README.md`, so an empty list cannot pass.  Plant each of these on the real tree, one at a time; each fails its test with the rule ID and the path.  Restore each, and record the results in the PR:
   * `git add -f song.flac` (10 bytes);
   * `git add tools/x.py`;
   * `- run: pip install numpy` in `ci.yml`, not staged;
   * `python3 -m http` + `.server` as one word in `README.md`, not staged;
   * `git add -f` a 2 MB file of zeros as `docs/big.txt`.

#### M8 acceptance

* The unit tests pass, and failed against the stubs in step 1.
* Both real-tree tests pass, and the CI `test` job lists them as `ok`.  The five planted breaches fail them.
* `git check-ignore -v engine/src/audio/x.rs` prints nothing and exits 1.  `git check-ignore -v app/dist/index.html stems/a.txt audio/synth/a.txt` names `/app/dist/`, `/stems/` and `/audio/`.  `git status --ignored` on the Mac lists the same three folders as before.
* `grep -c "Enforced by: prose only" docs/invariants.md` gives 5.  `Cargo.lock` does not change.
* fmt, clippy, the tests and `docs-check` pass; `docs-check` finds no `TD-5` or `TD-6` outside plans, decisions and the `## Closed` bullets.

### M9: young crates are pinned and the licence list is a test (TD-7, part; DEPS-1, DOC-1; maintainer decision)

* Scope, by the rule for proportionate sensors: two small checks over Cargo's output, with no new crate.  The breach has happened: `wsola` and `timestretch` came in as caret ranges (TD-7).  The likely next breach is an agent that adds a young crate with a caret, or that adds a crate and does not update `THIRD-PARTY.md`.

#### M9 state at the start

* DEPS-1 is "prose only" (TD-7).  Its text: "Pin a young crate (below 1.0, or recently published) exactly, with `=`."
* `cargo metadata --no-deps` on `2af33b1` gives 61 registry requirements in the members' manifests.  Three are pinned: `rusty_mp3 = "=1.0.0"` twice (1.0, first published 2026-07-29) and `naga = "=30.0.1"`.  36 requirements below 1.0 have a caret, on 18 crates.  Each locked version equals its requirement's version.
* The age of each of the 18 crates on crates.io, by the date of its first version (crates.io API, 2026-10-20):
  * under two years: `wsola` (2026-07-13, 2 versions), `timestretch` (2026-02-26), `cute-dsp` (2026-01-19), `burn-store` (2025-10-28) and `egui_kittest` (2024-12-16);
  * two years or more: `log` (2014), `cpal` (2014), `wasm-bindgen`, `futures-channel` (2018), `js-sys`, `web-sys`, `wasm-bindgen-test`, `wasm-bindgen-futures`, `console_error_panic_hook` (2018), `symphonia` (2020), `eframe` (2021), `burn` (2022) and `safetensors` (2022).
* `js-sys`, `web-sys` and `wasm-bindgen-futures` already take `wasm-bindgen` and `js-sys` with `=` (their published manifests, for example `js-sys` 0.3.106's `Cargo.toml`), so that family moves together in any case.
* Cargo's docs say: "When in doubt, use the default version requirement operator", and that a tight bound "is acceptable ... when no one else depends on your package" ([specifying dependencies](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html)).  No crate here is published, so an `=` pin costs only a manifest edit at each update.  Dependabot watches GitHub Actions only, so no bot edits these pins.
* `THIRD-PARTY.md` has two tables, each under its `cargo tree` command.  On `2af33b1` both are current.  The first command's output, without workspace crates, blank lines and `(*)` repeats, gives the 128 rows of the first table.  The second command's rows that are not in the first give the 8 rows of the second table.  Each command takes 0.4 s with a warm cache.
* `cargo tree -f "{p}"` prints a workspace crate as `app v0.1.0 (/abs/path)`, and a proc macro as `serde_derive v1.0.229 (proc-macro)`.  `{l}` is empty for `app`, `dsp` and `engine`, which set no licence.  Two roots give a blank line between their trees.  `cargo metadata`'s `req` is the normalised requirement, `"*"` when the manifest gives none ([cargo metadata](https://doc.rust-lang.org/cargo/commands/cargo-metadata.html)).

#### M9 decision point for the maintainer: which crates below 1.0 are young

* (a) Every registry crate below 1.0 is young, as DEPS-1's text reads today.  The check has no list.  Cost: 18 pins, among them `log`, `wasm-bindgen`, `web-sys` and `eframe`.  An update of `eframe` that needs a newer `log` then fails to resolve until both pins move.
* (b) Recommended: a crate below 1.0 is young unless `rules::SETTLED_CRATES` names it.  Each entry holds the crate and the date of its first version on crates.io, two years or more before it was added.  The check fails a stale entry.  So an agent that adds a crate below 1.0 must pin it, or list it with a date that a reviewer can check.  Cost: 5 pins and a list of 13.  DEPS-1's text gets "first published under two years ago", and names the list.
* (c) A named list of the young crates, each pinned.  Cost: 5 pins, and a new young crate added with a caret passes, which is the breach that happened.
* Each option applies to every workspace crate and every kind of dependency (normal, dev, build).  A build script or a proc macro of a dev-dependency runs on the maintainer's Mac too.  A recently published crate at 1.0 or later (as `rusty_mp3` was) stays a judgement for the person or agent who adds it; no check sees its age without a network call.
* The answer goes into DEPS-1's text and this plan's Decision log, with no decision record: each option is a reading of DEPS-1's existing words, as M7's correction of the types was.

#### M9 design

* Pins, under (b): `wsola`, `timestretch` and `cute-dsp` in `engine/Cargo.toml`, `egui_kittest` in `app/Cargo.toml` and `burn-store` in `xtask/Cargo.toml`, each to its locked version with `=`.  `Cargo.lock` does not change.
* The pin check: `pub fn young_crates_are_pinned_in(metadata: &str, settled: &[(&str, &str)]) -> Result<Vec<Problem>>` in `xtask/src/rules.rs`.  The real-tree test passes `SETTLED_CRATES`, a list of (crate, date) pairs; unit tests pass a small list.  It reads `cargo metadata`'s JSON (with `req` added to `MetadataDependency`).  It reads only the packages in `workspace_members`, since `workspace_metadata` runs without `--no-deps` and `packages` holds every crate.  It reads every dependency whose `source` is not `null`, that is, every dependency that is not a path.  A registry source starts with `registry+` or `sparse+`.  It names a crate by `name`, the package name, also when the dependency is renamed.  A problem names DEPS-1, the member's manifest, the crate and its requirement:
  * a source that is not a registry, such as a git dependency.  None exists today, and one that tracks a branch is the least pinned kind.  Fix: "use a registry release, or ask the maintainer".
  * a package in `packages`, member or not, whose `source` is neither `null` nor a registry.  A `[patch.crates-io]` that points a registry crate at git leaves the member's dependency with a `registry+` source, so only `packages` shows it.  On `2af33b1`, 927 packages have a registry source and 10 have `null`.  Fix: as above.
  * a registry requirement that is not a pin and is not settled above 1.0, for a crate that `settled` does not name.  A pin is one comparator `=M.m.p`, with or without a pre-release (`=0.1.0-alpha.1`).  A requirement is settled above 1.0 when no comparator has a pre-release, and one comparator sets a lower bound of 1.0.0 or more: `^`, `~`, `=` or `>=` with M of 1 or more; a wildcard `M.*` or `M.m.*` with M of 1 or more; `>` with M of 1 or more; or `>0`.  `<`, `<=` and `*` set no lower bound.  `cargo metadata` writes a bare `0.1` as `^0.1`.  So `*`, `<1`, `<2.0`, `0.*`, `^0`, `>0.1`, `>0.0.1`, `>=0.1`, `>=0.1, <0.3`, `^0.1.0`, `~0.1`, `=0.1` and `^1.0.0-rc.1` each give a problem.  `=0.1.0`, `^1`, `1.*`, `1.2.*`, `>=1.0, <2` and `>0` give none.  Fix: "pin it with `=` and its version in `Cargo.lock`, or, if its first version on crates.io is two years old or more, add it to `SETTLED_CRATES` with that date".  The message does not look up the locked version: `resolve` names a dependency by its import name (`cute_dsp`), and a crate can have two locked versions (`safetensors` 0.3.3 and 0.7.0), so the lookup costs more than it gives.
  * an entry of `settled` that matches no member's registry requirement that is neither a pin nor settled above 1.0.  The problem names `xtask/src/rules.rs`.  Fix: "remove the entry".
  * an entry of `settled` whose date is not in the form `YYYY-MM-DD`.  The problem names `xtask/src/rules.rs`.  The check reads only the form.  A reviewer checks the date against the crates.io API.
  * no registry dependency in any member, or no member, so empty input cannot pass.  A member with no dependency, such as `dsp` or `test-support`, passes.
* The licence check: `pub fn third_party_matches(doc: &str, normal: &str, normal_and_dev: &str, workspace_root: &str) -> Vec<Problem>`.  `third_party_tree(root, edges)` runs `cargo tree` as `web_app_tree` does, with `env!("CARGO")`, `--locked` and `--color never`, and the args from `third_party_args`.  CI's `CARGO_TERM_COLOR: always` otherwise puts colour codes into the output (M4).
  * A row is (crate, version, licence, source).  The parser drops a blank line, then splits each line on `|` into three cells, before it looks for parentheses.  Some licences start with `(`, such as `epaint_default_fonts`'s, and ` (*)` comes after `{r}`.  It drops a trailing ` (*)` from the last cell.  In the first cell, `name vVERSION`, it drops ` (proc-macro)`.  It drops a workspace crate: a path in parentheses that is `workspace_root` or under it, compared as paths (`Path::starts_with`), not as text.  Any other text in parentheses, such as a git URL, `(registry ...)` or a path outside the workspace, is a problem: "cannot read this line; add the crate to the tables by hand and extend the parser".  So no shipped crate leaves the list in silence.  A non-blank line that does not split into three cells, or whose first cell is not `name vVERSION`, is a "cannot read" problem too.  The doc's rows are the lines of a table under the header `| Crate | Version | Licence | Source |`, after its `| - | - | - | - |` separator.  The second table starts at `## Browser tests only`.
  * Problems, each with DEPS-1, `THIRD-PARTY.md` and the fix:
    * a command missing from its section: the first command before `## Browser tests only`, the second after it.  `fn third_party_args(edges: &str) -> [&str; 13]` gives the args in the doc's order: `tree -p app -p stem-worker --target wasm32-unknown-unknown -e <edges> --prefix none -f {p}|{l}|{r}`.  The check builds each doc line from them, `cargo` and the args joined by spaces, with the format in double quotes: `-f "{p}|{l}|{r}"`.  The runner adds `--locked` and `--color never` after `tree`.  So the doc and the check run the same command.  Fix: restore the command.
    * a crate and version in the first command's rows and not in the first table, or in the table and not in the rows.  The same for the second table, against the rows of the second command less the first.  Fix: "add the row" or "remove the row", with the row's text.
    * a crate and version in both, with another licence or source.  Fix: "read the new licence; if it is not permissive, ask the maintainer; then update the row".
    * a row in a table twice.
    * a row of either command with an empty licence.  Fix: "ask the maintainer; the check has no exception for a crate without a licence".  This fails closed.  No crate outside the workspace has one today.
    * a first command's output without `wgpu` or without `eframe`, an empty output included, so the check cannot pass on an empty run.  This is one problem, whichever crate is missing.
* Out of scope, named in DEPS-1's `* Enforced by:` line: a licence policy and an allowlist of registries (`cargo deny`, TD-7), the age of a crate at 1.0 or later, and whether the crate is maintained.

#### M9 steps

Each step is one signed commit; fmt, every clippy command in `CLAUDE.md`, the tests and `docs-check` pass before it (GIT-2).

0. Record the maintainer's answer at the decision point in this plan's Decision log.  The design and steps assume (b).  Under (a), `SETTLED_CRATES` is empty, and step 2 pins all 18 crates.  Under (c), rewrite the pin check's design before step 1.
1. Test first: the two parsers, stubs of the two checks that return no problem, and the unit tests.  Every test that expects a problem fails against the stubs.  Then write the checks, and all pass.  Unit tests, each on a small JSON or text fixture:
   * `pins_and_settled_crates_pass`: `=0.1.0`, `=0.1.0-alpha.1`, `^1`, `^1.0.107`, `1.*`, `>=1.0, <2`, `>0`, a path dependency with `^0.1.0`, a settled crate with `^0.4.34`, and a member with no dependency give no problem.  A package with `^0.1.0` that is not in `workspace_members` gives none.
   * `a_young_crate_with_a_range_is_named`: `^0.1.0`, `^0.0.41`, `*`, `<1`, `<2.0`, `0.*`, `^0`, `>0.1`, `>0.0.1`, `>=0.1`, `>=0.1, <0.3`, `~0.1.2`, `=0.1` and `^1.0.0-rc.1` give one problem each, with the manifest, the crate and the requirement.  The fixtures take the real shape of `cargo metadata`'s JSON.  The cases are in the second of two members, so a check of the first member only fails.  They cover a normal, a dev and a build dependency, a `cfg` target, a `sparse+` source, and a renamed dependency, which the check names by its package name.
   * `a_source_that_is_not_a_registry_is_named`: a member's dependency with a `git+` source and `*`, a member's dependency with a source that starts with `path+`, and a package outside the members with a `git+` source (as a `[patch]` gives it) each give one problem.
   * `a_stale_settled_entry_is_named`: an entry that no member uses, an entry for a crate pinned with `=`, and a date `2024-1-5` give one problem each.
   * `nothing_to_read_is_a_problem`: no member, and members with no registry dependency, each give one problem.
   * `third_party_rows_are_read`: the parser drops a workspace crate, a blank line, ` (*)`, ` (proc-macro)`, and `x v0.1.0 (/root/x) (proc-macro)`.  It keeps two versions of one crate as two rows, a licence that starts with `(`, and an empty source.  A git URL, `(registry \`r\`)`, a path outside the root, a path whose text starts with the root's text (`/root-other/x` for `/root`), `foo v1.0.0|MIT` and `garbage` each give a "cannot read" problem.  The doc parser skips the header and the separator of each table.
   * `the_third_party_tables_match`: a fixture doc and its tree give no problem.
   * `a_third_party_drift_is_named`: a missing row, a stale row, a changed licence, a changed source, a row twice, a dev-only row copied into the first table, each command removed in turn, the first command moved into the second section, and an empty licence each give one problem with DEPS-1.  An empty tree with empty tables, a tree without `wgpu` whose table has no `wgpu` row, and the same for `eframe`, each give one problem.  A changed version (as M11's `wsola` 0.1.0 to 0.2.0) gives two: "add the row" and "remove the row".
2. The pins, the real-tree tests `young_crates_are_pinned` and `third_party_lists_what_ships` in `xtask/tests/rules.rs`, and the docs.  `young_crates_are_pinned` shares the `cargo metadata` run of the layer tests; the `OnceLock` then keeps the JSON too.  Before the pins, `young_crates_are_pinned` fails and names each of the 5 crates.  Plant each of these on the real tree, one at a time; each fails its test with DEPS-1.  Restore each, and record the results in the PR:
   * `wsola = { version = "0.1.0", ... }` without `=` in `engine/Cargo.toml`;
   * `log` removed from `SETTLED_CRATES`;
   * a `SETTLED_CRATES` entry for `zzz`;
   * the `log` row removed from the first table;
   * `MIT` for `Apache-2.0 OR MIT` in the `indexmap` row.
* Docs (DOC-1), in step 2's commit:
  * `invariants.md`, DEPS-1: the text names `SETTLED_CRATES` and "first published under two years ago" for a crate below 1.0.  For a crate at 1.0 or later, "recently published" stays a judgement, as today.  "Generated by the command at its top" becomes "generated by the two commands in it".  `* Enforced by:` becomes "`young_crates_are_pinned` and `third_party_lists_what_ships` (CI `test` job).  The first fails a registry requirement below 1.0 without `=`, unless `SETTLED_CRATES` names the crate, and fails a source that is not a registry, such as git.  It does not check an entry's date; a reviewer does.  The second compares both tables of `THIRD-PARTY.md` with their commands.  Neither sees a young crate at 1.0 or later, or whether a crate is maintained."  `* Gaps: TD-7.`  "Prose only" lines fall from 5 to 4.
  * `tech-debt.md`: TD-7 narrowed to "no licence policy and no allowlist of registries", with the fix `cargo deny`.  Its title becomes "TD-7: no licence policy and no allowlist of registries".
  * `THIRD-PARTY.md`: one sentence under each command says that `third_party_lists_what_ships` compares the table with it.
  * `CLAUDE.md`: the DEPS-1 line stays; it is still true.
  * `harness.md`: a components row for the two tests.  `working-with-agents.md`: a changelog line that says how to fix each failure (run the command and copy the rows; pin, or list with a date).
  * The parent plan's phase 2 bullet tags `THIRD-PARTY.md` against `cargo tree` with M9, and adds "young crates pinned, over `cargo metadata` (DEPS-1, M9)".  Phase 5a's gardening job "`THIRD-PARTY.md` against the lock file" becomes "new releases of the pinned young crates", since `third_party_lists_what_ships` now does the comparison on every PR.

#### M9 acceptance

* The unit tests pass, and failed against the stubs in step 1.
* Both real-tree tests pass, and the CI `test` job lists them as `ok`.  The five planted breaches fail them.
* `Cargo.lock` does not change.  `grep -c "Enforced by: prose only" docs/invariants.md` gives 4.
* fmt, clippy, the tests and `docs-check` pass.

### M10: the hand-written e2e messages match the protocol (TD-16, part; PROTO-1, DOC-1)

* Scope, by the rule for proportionate sensors: one token scan, with no new crate, no new crate edge and no decision record.  No type has drifted.  `stem_protocol::kind` has not changed since the crate came in (`fd4656c`, 2026-10-19).  The likely breach: an agent renames or adds a type in `stem-protocol`, or writes a new e2e case with a misspelt type.  CI runs the `smoke`, `audio-resume` and `storage` scenarios only.  So a drift in `worker-check` fails nowhere, and a drift in `storage` fails late, as a script timeout or a failed check that names no rule.

#### M10 state at the start

On 2026-10-20, at commit `7a99a88` (M9 merged as PR #22):

* `pub mod kind` in `protocol/src/lib.rs` holds 15 items, each `pub const NAME: &str = "...";`, and nothing else.  The private `mod field` holds the field names.  Three of them have the same text as a type: `"done"`, `"selftest"` and `"bench"`.
* `xtask/e2e/worker.js` uses types in three shapes: `{ type: "worker-error", ... }`, `d.type === "status"` (5 times, one as `e.data.type`), and `["model", "error", "selftest", "done"].includes(d.type)`.  `worker-error` is the reply that `e2e.ask` makes when the worker throws; `kind` does not have it.  The file also has `new Worker(..., { type: "module" })`, an option of the `Worker` constructor, not a message.
* `xtask/src/e2e.rs` holds its scripts as Rust string literals, passed to `in_page` and `session.execute`.  They use `{ type: '...' }` 20 times: `download` 11, `model` 4, `init` 3, `forget_model` 1 and `separate` 1.  The Rust code compares one reply type: `r["end"]["type"] == json!("done")`.  `drop_files` has `{ type: f.type }` in a raw literal and `json!({ ..., "type": kind })`.  Both are the MIME types of dropped files, not messages.  No script literal holds `\"`; two prose literals do.
* The other e2e helpers hold no message: `console.js` (`typeof`), `audio_contexts.js` and `audio_like_safari.js` (`prototype`), and `blank.html`, which only loads `worker.js`.
* `xtask/src/rules.rs` will hold misspelt types in its unit tests, so the check must not read it.

#### M10 design

* The types: `fn protocol_kinds(protocol: &str) -> Result<BTreeSet<String>, String>` reads `protocol/src/lib.rs` with `proc-macro2`, as M3 and M7 do.  It takes every string literal in the brace group after the idents `mod kind`, at the top level of the file.  The error says why it found no type.  `string_literals` (M3) already leaves out doc comments.  So `xtask` takes no dependency on `stem-protocol`, the layers table does not change, and a new constant needs no second list.  A dependency would need that list, since Rust code cannot list a module's items.
* The check: `pub fn e2e_types_match_the_protocol(protocol: &str, scripts: &[(String, String)], e2e_rust: &str, script_only: &[(&str, &str)]) -> Vec<Problem>` in `xtask/src/rules.rs`.  `scripts` holds each `xtask/e2e/*.js` file with its text.  `e2e_rust` is the text of `xtask/src/e2e.rs`.  The real-tree test passes `SCRIPT_ONLY_TYPES`; unit tests pass a small list, as M9's do.
* `pub const SCRIPT_ONLY_TYPES: [(&str, &str); 2]`, each a type and its reason: `"worker-error"`, "the reply that `e2e.ask` makes when the worker throws"; and `"module"`, "the `Worker` constructor's option in `worker.js`, not a message".
* A small JS lexer: `fn js_tokens(text: &str) -> Vec<(usize, JsToken)>` gives each token with its line, from 1.  `JsToken` is `Ident(String)`, `Str(String)` or `Punct(char)`.  An ident is a run of letters, digits, `_` and `$`, so `typeof` is not `type`.  The lexer skips white space, `//` and `/* */` comments, and template literals (backtick to backtick, past a `\` escape, with no look inside `${}`).  A string reads to its closing quote, past a `\` escape.  A string that has no closing quote before the end of its line gives no token, and the lex goes on from the next line.  So the lexer never fails, and a prose literal with an apostrophe ("the page's") gives no type.  It reads no regex literal; no script has one.
* The shapes of a type in JS, each giving the string and its line:
  * The ident `type` or the string `"type"`, then `:`, then a string: `{ type: 'download' }` and `{ "type": "download" }`.
  * The ident `type`, alone or after `.`, then an operator, then a string: `d.type === "alive"` and `type === "alive"`.  The operator is `===`, `==`, `!==` or `!=`, read as a run of `=` and `!` puncts.  A chain after `typeof` does not count, so `typeof d.type === "string"` gives no type.
  * A string, an operator, then a chain of idents joined by `.` whose last ident is `type`: `"alive" === e.data.type`.
  * `[`, one or more strings joined by `,`, with an optional trailing `,`, then `]`, `.`, `includes`, `(`, a chain whose last ident is `type`, and `)`.  Each string is a type.
* In `xtask/src/e2e.rs`, from `proc-macro2`'s tokens:
  * Each string literal, plain or raw, is lexed as JS on its own, so an unmatched quote in one hides nothing in the next.  `string_value` gives the source text between the quotes.  A literal is raw when its text (`literal.to_string()`) starts with `r`.  For a plain literal, the check first maps `\\` to `\` and `\"` to `"`; this keeps the newlines.  A token's line is the literal's first line plus the newlines before the token in the literal's text.
  * Two Rust shapes, each with two forms of its value: a string literal, or `json` `!` and a paren group that holds only a string literal.
    * A bracket group that holds only the literal `"type"`, then `==` or `!=`, then the value: `r["end"]["type"] == json!("done")`.
    * In a brace group, the literal `"type"`, then `:`, then a string literal: `json!([{ "type": "download" }])`, as script args.  `json!({ ..., "type": kind })` has an ident after the `:`, so it gives no type.
* Problems, each with PROTO-1, the file, the line if any, and the fix.  When `protocol_kinds` fails, the check gives that one problem and stops, so no type is reported as unknown.  An `e2e.rs` that does not tokenise gives one problem, and no "no type" problem.  An entry of `script_only` that is unused and also in `kind` gives one problem.
  * A type that is in neither `kind` nor `script_only`: "PROTO-1: xtask/e2e/worker.js:26 uses the message type \"alve\", which `stem_protocol::kind` does not have.  Fix: use a type from `kind` (\"init\", ...); or change `kind`, its encoder and its decoder together; or, for a type that only the scripts make or an option that is not a message, add it to `SCRIPT_ONLY_TYPES` with a reason."
  * An entry of `script_only` that no script uses, or that `kind` has.  It names `xtask/src/rules.rs`.  Fix: "remove the entry".
  * `protocol/src/lib.rs` that does not tokenise, has no `mod kind`, or has no string in `kind`.  Fix: "keep the message types as string constants in `stem_protocol::kind`, or update the check".
  * `xtask/src/e2e.rs` that does not tokenise.  Fix: "make the file valid Rust".
  * No type in the `*.js` files, and no type in `xtask/src/e2e.rs`: one problem for each.  Its file is `xtask/e2e` or `xtask/src/e2e.rs`.  Fix: "write each type in a shape that the check reads, or update the check".  So a script that moves or changes shape cannot pass in silence.
* Out of scope, named in PROTO-1's `* Enforced by:` line and in TD-16: field names; a type in a shape the scan does not read (a `switch` and its `case`s, an assignment `m.type = "x"`, `"x" === d?.type`, `d["type"]` in JS, `json!("x") == r["type"]` in Rust, a variable, a template literal, `.as_str() == Some("x")`, and a `\n` escape in a plain Rust literal after a `//` comment, which hides the rest of that literal); a false type that fails closed, such as `{ type: '{kind}' }` in a `format!` string or `c ? type : 'x'`; scripts in other files; a reply with a missing field, which decodes as a default; and a version handshake.

#### M10 steps

Each step is one signed commit; fmt, every clippy command in `CLAUDE.md`, the tests and `docs-check` pass before it (GIT-2).

1. Test first: `js_tokens`, `protocol_kinds`, a stub of `e2e_types_match_the_protocol` that returns no problem, and the unit tests, in a module `proto_tests`.  Every test that expects a problem fails against the stub.  Tests that count problems pass an empty `script_only`, except `the_script_only_list_is_checked`.  Then write the check, and all pass.  Unit tests:
   * `js_is_lexed`: the lines of tokens on three lines, both quotes, an escaped quote in a string, both comment forms, a template literal with `${ "x" }` and with an escaped backtick, a quote not closed before the end of its line, and `typeof`.  A token after a `/* */` comment over two lines, and a token after a template literal over two lines, each has the right line.
   * `the_kinds_are_read`: a fixture with `mod field` and then `pub mod kind` gives the strings of `kind` only.  A doc comment in `kind` is left out, and `field`'s `"text"` is not a type.
   * `the_e2e_types_match`: a fixture `worker.js` with each JS shape, in both quotes and both orders of the comparison, gives no problem.  So does a fixture `e2e.rs` with a script in a plain and a raw literal, and each Rust shape in both forms.
   * `a_misspelt_type_is_named`: `"segmnt"` gives one problem with PROTO-1, the file and the line, in each of these: the four JS shapes, with the ident `type`, the string `"type"`, `d.type` and a bare `type` where they apply; the second of three strings in an `includes` array, with a trailing `,`; each of `===`, `==`, `!==` and `!=` once; `\"segmnt\"` in a plain Rust literal; a script in a raw literal; and each Rust shape in both forms, with `==` and `!=`.  In `e2e.rs` it is on the third line of a script literal, and the line is right.  `"text"`, a field name, gives a problem too.
   * `other_type_words_are_not_types`: `{ type: f.type }`, `typeof a === "string"`, `typeof d.type === "string"`, `"file" === h.kind`, `["a"].includes(d.kind)`, `const { type } = d`, a comment, a template literal, `json!({ "type": kind })` and a prose literal with `\"` each give no problem.  A prose literal with an unmatched `'` before a script literal does not hide the script's misspelt type.
   * `the_script_only_list_is_checked`: an entry that no script uses, and an entry that `kind` has, give one problem each.  `{ type: "module" }` without its entry gives one.
   * `e2e_nothing_to_read_is_a_problem`: a protocol without `mod kind`, a `kind` with no string, an `e2e.rs` that does not tokenise, `*.js` files with no type, and an `e2e.rs` with no type each give one problem.
2. The real-tree test `e2e_messages_use_protocol_types` in `xtask/tests/rules.rs`, and the docs.  The test reads `protocol/src/lib.rs`, each `xtask/e2e/*.js` file in name order, and `xtask/src/e2e.rs`.  It asserts that the scripts include `xtask/e2e/worker.js`.  Plant each of these on the real tree, one at a time; each fails the test with PROTO-1 and the right line.  Restore each, and record the results in the PR:
   * `"alive"` as `"alve"` in `worker.js`;
   * `type: 'forget_model'` as `type: 'forget'` in `e2e.rs`;
   * `json!("done")` as `json!("dne")` in `e2e.rs`;
   * `LOG`'s value as `"logs"` in `protocol/src/lib.rs`, which names `worker.js:43`;
   * `"worker-error"` removed from `SCRIPT_ONLY_TYPES`, which then has the type `[(&str, &str); 1]`.
* Docs (DOC-1), in step 2's commit:
  * `invariants.md`, PROTO-1: `* Enforced by:` adds "`e2e_messages_use_protocol_types` (CI `test` job).  It compares the message types in `xtask/e2e/*.js` and in the scripts of `xtask/src/e2e.rs` with `stem_protocol::kind`.  It does not read field names."  `* Gaps: TD-16.` stays.
  * `tech-debt.md`: TD-16 is narrowed to the field names in the e2e scripts, the type shapes the scan does not read, the silent defaults and the handshake.  Its title becomes "TD-16: the wire format has unchecked field names and silent defaults".  Its fix drops the type test.
  * `.claude/rules/protocol.md`: the e2e bullet says that `e2e_messages_use_protocol_types` checks the type strings, and that a person checks the field names.
  * `working-with-agents.md`: the protocol recipe's "Check:" line adds `cargo test -p xtask --test rules`.
  * `harness.md`: a components row.  `working-with-agents.md`: a changelog line that says how to fix a failure.
  * The parent plan's phase 2 bullet: "(TD-16)" after the e2e types becomes "(PROTO-1, TD-16, M10)".
  * `ARCHITECTURE.md`, `CLAUDE.md` and the manifests do not change.  "Prose only" lines stay at 4.

#### M10 acceptance

* The unit tests pass.  In step 1, each test that expects a problem failed against the stub.
* `e2e_messages_use_protocol_types` passes, and the CI `test` job lists it as `ok`.  The five planted breaches fail it.
* `Cargo.lock` and `xtask/Cargo.toml` do not change.  The CI `browser-tests` job passes; M10 changes no script.
* fmt, clippy, the tests and `docs-check` pass.

### M11: the audio callback does not allocate after warm-up (closes TD-15; RT-1, DEPS-1, DOC-1; maintainer decision, made 2026-10-19)

* Scope: RT-1 is a realtime rule, and its breach is known (TD-15).  So this check is stronger than the token scans of M7 to M10: a test runs the real callback under a counting allocator.  No new crate: wsola 0.2.0 takes no crate that the lock file lacks.

#### M11 state at the start

On 2026-10-21, at commit `7cd8518` (M10 merged as PR #23):

* The callback's path: cpal's data callback in `engine/src/live.rs` (`open`) calls `Schedule::next`, then `PlayerSlot::render_interleaved` (`engine/src/slot.rs`).  That calls `Player::process` (`engine/src/player.rs`) for at most `SCRATCH_FRAMES` (8,192) frames at a time.  `process` renders blocks of at most `PlayerConfig::block` (512) frames, through `render_stretched` or `render_chipmunk`.
* The app makes each player with `Player::with_config(.., self.stretcher.build(song.sample_rate), PlayerConfig::default())` in `app/src/lib.rs`.  It builds `engine` with `default-features = false, features = ["stretch-wsola"]`, so it ships `Varispeed` and `WsolaStretcher`.  It plays through `StretcherKind::default_pitch_preserving()`, which is wsola; `Varispeed` is compiled in but not chosen.  RT-1 covers both.  `xtask` builds `engine` with every stretcher, for its benches.
* What allocates on the path today, from a read of the code:
  * `WsolaStretcher::can_pull` calls wsola 0.1.0's `pull`, which returns a new `Vec` (`self.output.drain(0..n).collect()`).
  * wsola's `TimeStretch` starts with an empty `input` and `output`.  `push` extends `input`, and `compact` drops old input only when more than 65,536 frames (`1 << 16`) lie behind the read position.  So `input` grows, with a reallocation at each doubling, over the first 1.5 s or so of play.  This is the same in 0.1.0 and 0.2.0.
  * `render_chipmunk` calls `dsp::double_speed` twice per block, and each call returns a new `Vec`.
  * `read_mixed` adds an `Anchor` to `anchors` (capacity 1,024) at each loop wrap, and `playhead` drops old anchors only at the end of `process`.  At tempo 1.5, a loop shorter than about 9 frames with 4,096-frame callbacks, or 15 frames with 8,192-frame callbacks, adds more than 1,024 anchors, so the queue grows.  The loop bar cannot make a loop that short: one pixel is about 1,000 frames of the 30 s demo song.  `SharedParams::set_loop` accepts any loop of 1 frame or more.
  * Nothing else allocates.  The rest reads atomics, writes into buffers that `with_config` sizes, uses `VecDeque`s within their capacity, and clones the `Arc`s of `params` and `song` without freeing them.  `Varispeed` holds about 1,100 frames at most, in queues of capacity 32,768.
* wsola 0.2.0 (MIT, published 2026-10-12, by the author of 0.1.0; [crates.io](https://crates.io/crates/wsola)), from a diff of the two published sources:
  * `pull` and `flush` return `impl Iterator<Item = f32>`, which is `self.output.drain(0..n)` and does not allocate.  `output` is a `Vec`, and `step` calls `output.reserve(hop * channels)` first.
  * `new` and `with_config` take `NonZeroU32` and `NonZeroU16`; the errors for zero are gone.
  * The rest of `step`, `compact` and the window is unchanged, so the output is the same, sample for sample.
  * Its one dependency is `thiserror` 2.0, which the lock file already has at 2.0.21.
* The tests of the callback are in `engine/src/slot.rs`.  None counts allocations.
* Facts on the method:
  * A global allocator may use `std::thread_local`: "a few features from `std` are guaranteed to not use `#[global_allocator]` to allocate", and it is one.  The optimiser may remove an allocation, so "you cannot rely on global allocations actually happening" ([`GlobalAlloc`](https://doc.rust-lang.org/std/alloc/trait.GlobalAlloc.html)).  A zero count is evidence, not proof.  The `dev` profile builds the workspace crates at opt-level 0, and the dependencies at 2 (`Cargo.toml`).
  * Why a callback must not allocate: an allocator can take a lock or fault a page, so its time has no bound ([Bencina, "Real-time audio programming 101: time waits for nothing"](http://www.rossbencina.com/code/real-time-audio-programming-101-time-waits-for-nothing), summarised in [these rules of thumb](https://w2.mat.ucsb.edu/240/A/2017/05/17/realtime.html)).
  * The off-the-shelf tool is `assert_no_alloc` 1.1.2 (BSD-1-Clause, last release 2021-08-03).  It is a global allocator over `System` with a flag per thread, and it aborts or warns on an allocation.  By default it does nothing in release builds ([docs.rs](https://docs.rs/assert_no_alloc/latest/assert_no_alloc/)).  M11 does not use it.  A counter in the test is about 40 lines, gives counts for the failure message, and adds no crate (DEPS-1).

#### M11 design

* The rule.  RT-1 becomes "the audio callback never waits, never frees a player, and never allocates or frees memory after warm-up".  Warm-up is the work that `Player::with_config` does, on the thread that makes the player.  RT-1 covers the stretchers that the app ships.
* The record: `docs/decisions/0007-no-allocation-in-the-audio-callback.md`, with `Status: accepted (2026-10-19, by the maintainer)`, since the maintainer chose the rule (decision log, 2026-10-19).  Its options:
  * (a) no allocation after warm-up, chosen;
  * (b) no allocation at all, which cannot hold, since a new player must make its buffers;
  * (c) allocation allowed and measured only, which leaves TD-15 open.
  * Its consequences: `Stretcher::warm_up`, the test, wsola 0.2.0, the gaps in TD-<n>, and the bench-only stretchers that RT-1 does not cover.
* Warm-up: a new required method of `Stretcher`, `fn warm_up(&mut self, max_tempo: f64, max_frames: usize)`.  Its doc: after it, `push` of at most 4,096 frames (what wsola's `interleaved` holds) and `pull` of at most `max_frames`, at a tempo of at most `max_tempo`, do not allocate (RT-1).  It leaves the stretcher as `reset` does, with the tempo it had.  `Player::with_config` calls `stretcher.warm_up(f64::from(params::MAX_TEMPO), cfg.block)` before it returns.
  * `Varispeed`: nothing to do.  Its doc says why: its queues have capacity 32,768 frames from `new`, and hold at most `max_frames * max_tempo` plus one push and 2 frames.
  * `WsolaStretcher`:
    1. Push `(1 << 16) + sample_rate / 4 + MAX_PUSH_FRAMES` frames of silence in one call, `self.ts.push(&vec![0.0; 2 * n])`, not through `Stretcher::push`.  `MAX_PUSH_FRAMES` (4,096, in `engine/src/stretch/mod.rs`) is the largest push that `warm_up` covers; review round 1 added it.  Through `push`, `interleaved` would grow and keep about 1 MB for good (MEM-4).  So `input` gets a capacity of at least that many frames.  In play, `input` holds at most `65,536 + 4 * search + frame + 2 * tempo * hop + push` frames.  Compaction runs after each step, so one step's advance and the search's jitter can pass 65,536.  That is about 75,000 frames at 44.1 kHz and tempo 1.5 with pushes of 4,096; the most a prototype measured, with the player's pushes of 256, was 69,116.  `4 * search + frame + 2 * tempo * hop` is below `sample_rate / 4` at any tempo up to wsola's 4.  A comment in the code gives this sum.  The capacity is then about 650 KB, where doubling gave about 1 MB (MEM-4).
    2. Set the tempo to `max_tempo` and call `can_pull(max_frames + sample_rate / 50)`.  So wsola's `output` reaches its largest size: a request of `max_frames`, plus one wsola hop of 15 ms, which is less than 20 ms.
    3. `reset`, and set the tempo back.
    * The cost: a copy of about 650 KB and a few wsola steps, on the thread that opens a song.  wsola's `reset` clears its `Vec`s and keeps their capacity.
  * `TimestretchStretcher` and `CuteStretcher` are bench-only.  Their `warm_up` does nothing, and its doc says that RT-1 does not cover them.
* Chipmunk mode in place: `render_chipmunk` writes `l[i] = self.chip_l[2 * i]` and `r[i] = self.chip_r[2 * i]`, for `i` in `0..l.len()`.  `dsp::double_speed` stays as the beginner version (DSP-1), off the callback's path.
* Anchors: in `read_mixed`, before each `push_back`, drop the oldest anchor when `anchors.len() == anchors.capacity()`.  Only a loop shorter than 16 frames fills the queue, with callbacks of up to `SCRATCH_FRAMES`.  The dropped anchor can be the one being heard, such as the run before the loop.  The playhead then shows the loop's start early, for as long as the stretcher's buffered audio plays: about 1,900 frames (44 ms) in a prototype.  The audio does not change.
* wsola 0.2.0: `engine/Cargo.toml` takes `wsola = { version = "=0.2.0", optional = true }`.  The adapter passes `NonZeroU32::new(sample_rate)` with an `expect` that says a song has a sample rate, as `with_config`'s `expect` does today.  `can_pull` copies `pull`'s iterator, two samples a frame, into `out_l` and `out_r`.  It stops, as today, when `pull` gives nothing.
* The test file, `engine/tests/realtime.rs`, is its own test binary, so its allocator is in no other test:
  * `CountingAllocator` wraps `System`.  It forwards each call to `System`.  It counts `alloc`, `alloc_zeroed` and `realloc` as allocations, and `dealloc` as frees, in `thread_local!` `Cell<u64>` counters with `const` initialisers.  So other test threads do not mix in.
  * `fn counted<T>(f: impl FnOnce() -> T) -> (T, Counts)` gives the counts on this thread while `f` runs.
  * `SHIPPED: [StretcherKind; 2]` is `Varispeed` and `Wsola`.
  * Each configuration runs on its own thread, which the counters per thread allow.  A prototype took 16.5 s on one thread in the `dev` profile, about 1.4 s per configuration.
* `the_counter_sees_an_allocation`: in `counted`, through `std::hint::black_box`, a `Vec::<u8>::with_capacity(64)` (`alloc`), then `reserve_exact(4096)` on it (`realloc`), and a `vec![0u8; 64]` (`alloc_zeroed`), both dropped.  It counts 3 allocations and 2 frees.  So a counter that misses one of the four calls fails, and the planted breach stays as a test.  Vec growth in play comes through `realloc`.
* `the_callback_does_not_allocate_after_warm_up`, for each of `SHIPPED`, each sample rate of 44,100 and 48,000, and each callback of 128, 512, 4,096 and 10,000 frames (10,000 since review round 2):
  * Set-up, not counted: a song of 6 noise stems, 10 s long, made with `Song`, `Stem` and `Samples::from`.  Then `SharedParams`, `Player::new(song, params, kind)` and a `PlayerSlot` that holds it.  The callback's `data` and `Scratch` are made once.
  * Each callback, `slot.render_interleaved(&mut data, 2, &mut scratch)`, is counted, from the first.  Changes to `params` happen between callbacks.  Durations are of output, unless a phase says "of the song".  A phase of `d` seconds of output is `ceil(d * rate / callback)` callbacks, where `callback` is the callback's frames.  The phases, in order:
    1. "start": play at tempo 1.0 for 0.5 s;
    2. "tempo sweep": tempo from 0.5 to 1.5 in steps of 0.1, 0.1 s each;
    3. "fast": tempo 1.5 for 3.5 s of the song (7/3 s of output), more than twice 65,536 input frames at either rate.  So wsola's `input` reaches its largest size and is compacted twice.  The tempo stays 1.5 from here on;
    4. "chipmunk": chipmunk on for 0.2 s, then off for 0.2 s;
    5. "seek": a seek to 2 s while playing (a fade out and a fade in), 0.2 s of play, a pause of one callback, a seek to 3 s while paused, one more paused callback, and 0.2 s of play again.  So `jump_to` runs while paused;
    6. "loop": a 1 s loop (of the song) from the playhead, for 2 s of output, which is 3 wraps at tempo 1.5, with the crossfade;
    7. "short loop": a 1-frame loop at the playhead, for 0.2 s;
    8. "mix": the loop cleared, then mute, solo, unmute and unsolo of one stem, each followed by 0.1 s of play;
    9. "end": a seek to 0.1 s before the end, and play until `playing` is false, for at most 2 s.  Then two callbacks of silence.
  * So that a player that renders silence, or does not play, cannot pass:
    * each phase gives a sample that is not zero;
    * after "fast", the playhead is at least 3 s of song past its value before the phase;
    * "loop" starts its loop at the playhead, and after it `s <= playhead <= e`.  Varispeed's playhead can equal `e`;
    * after "end", `playing` is false.
  * A failed check of these has its own message: "RT-1 test: wsola, 48000 Hz, 4096-frame callbacks, phase "seek" gave only silence, so it checked nothing.  Fix: make the phase play, or correct the test."
  * A prototype showed that stricter playhead checks fail on correct code.  A playhead runs past a short loop while the stretcher's buffered audio plays, and a seek can land inside the phase's first callback.
  * One failure lists every phase that allocated or freed: "RT-1: the audio callback allocated or freed memory after warm-up: wsola, 48000 Hz, 4096-frame callbacks, phase "short loop": 3 allocations, 0 frees.  Fix: size the buffer in `Player::with_config` or `Stretcher::warm_up`, or reuse a buffer.  Do not allocate in `Player::process` or below."
* The manifest check: a pure `fn app_stretcher_problems(manifest: &str) -> Vec<String>` in `engine/tests/realtime.rs`.
  * It splits the text on each character outside `[A-Za-z0-9_-]`, and collects each word that starts with `stretch-`.  So `"engine/stretch-cute"` in `[features]` gives `stretch-cute`.  The set must be exactly `{"stretch-wsola"}`.
  * Each line that starts with `engine =`, in any table, must hold `default-features = false`, since `engine`'s default features turn on every stretcher.  There must be one such line at least.
  * `engine` in another form is a problem, with the fix "write `engine` as one inline table": a table header that ends in `.engine]`, or a line that starts with `engine.`.
  * `default` after `engine/` or `engine?/`, and `"default"` in the `features` of an `engine =` line, are problems too.
  * Unit tests, `the_manifest_check_finds_a_stretcher`: each of these gives one problem: `stretch-cute` in a `[features]` list; an `engine =` line without `default-features = false`; `engine/default`; `engine?/default`; a `[target.'cfg(unix)'.dependencies.engine]` table; and `engine.path = "../engine"`.  The real manifest is left to `the_app_ships_only_these_stretchers`.
* `the_app_ships_only_these_stretchers`: it runs the check on `app/Cargo.toml`, read with `include_str!`.  Failure: "RT-1: `app/Cargo.toml` turns on {..}, but `engine/tests/realtime.rs` covers only `SHIPPED`.  Fix: add the stretcher to `SHIPPED`, make the test pass, and update this set."  An empty set fails too.
* Unit tests beside the code:
  * `a_warmed_stretcher_sounds_the_same` (`engine/src/stretch/mod.rs`): for each of `StretcherKind::all()`, two stretchers get tempo 0.75, then the same input, a sine, not silence; only one is warmed up, after its tempo is set.  Their output is the same, bit for bit.  So a warm-up that does not reset, or does not restore the tempo, fails.
  * `chipmunk_matches_dsp_double_speed` (`engine/src/player.rs`): one stem, with a rising ramp on the left and a falling ramp on the right, in chipmunk mode.  Each channel gives `dsp::double_speed` of that channel's ramp, sample for sample.  So a mix-up of the channels fails.
  * `a_very_short_loop_keeps_its_anchors` (`engine/src/player.rs`): from frame 0, `set_loop(1_000, 1_001)` at tempo 1.5, then 8,192 frames.  The capacity of `anchors` is as it was, and `1_000 <= playhead <= 1_001`.  A prototype without the bound grew it from 1,024 to 16,384.
* The gaps, as a new entry TD-<n> under "Enforcement gaps", where `<n>` is the next free ID (25 on 2026-10-21), "RT-1's allocation test runs natively only".  It does not count the WebAssembly build, cpal's hosts, or the closure in `live.rs`.  That closure calls `Schedule::next`, whose fields are all `Copy`.  On the web, the first `web_time::Instant::now` in `render_interleaved` fills a `thread_local`.  A zero count is evidence, not proof, since the optimiser may remove an allocation.  The bench-only stretchers are not covered.  The manifest check reads the text of `app/Cargo.toml` only, so it does not see a stretcher that another crate turns on through feature unification.  Fix: none planned; phase 4's evaluator may count in the browser.
* Out of scope: an allocation check in the app at run time, such as `assert_no_alloc` in debug builds.

#### M11 steps

Each step is one signed commit; fmt, every clippy command in `CLAUDE.md`, the tests and `docs-check` pass before it (GIT-2).

1. wsola 0.2.0, alone.
   * Before the bump, a throwaway test in `xtask/tests/`, not committed, renders `synth::generate()` through `Player::new` with `StretcherKind::Wsola`.  It renders 10 s at tempo 0.75 and 10 s at 1.25 in 512-frame blocks, and prints a hash of the samples' bits.  Run it before and after the bump, and again after step 2's warm-up.  The hashes must be equal; record them in the PR, then delete the test.  The latency table in `wsola_adapter.rs` then stays.
   * The pin, the adapter, `Cargo.lock` (only wsola's entry changes) and the wsola row in `THIRD-PARTY.md`.  `young_crates_are_pinned`, `third_party_lists_what_ships` and `m4_stretch` pass.
   * TD-15's text loses wsola's `pull` and keeps the other allocations: wsola's growing `input`, `dsp::double_speed` and the anchors (DOC-1).
2. The test, the fixes and the docs.
   * Test first: write `engine/tests/realtime.rs` and run it on step 1's code.  Record each phase that fails, with its counts, for each stretcher.  Expected, from a prototype: "start" (10 allocations) and "tempo sweep" (2) for wsola; "chipmunk" for both (hundreds); "short loop" for wsola at every callback size, and for varispeed at 4,096 frames only.  "fast" does not fail.  The test is not committed on its own, since it fails (GIT-2).
   * Then `Stretcher::warm_up`, chipmunk in place, the anchor bound and the three unit tests.  All tests pass.
   * Plant each of these on the real tree, one at a time; each fails `the_callback_does_not_allocate_after_warm_up` with RT-1 and the named phase.  Restore each, and record the results in the PR:
     * the `warm_up` call removed from `Player::with_config` ("start" and "tempo sweep", wsola);
     * a warm-up push of `(1 << 16) + sample_rate / 50` frames ("tempo sweep", wsola);
     * `render_chipmunk` back on `dsp::double_speed` ("chipmunk");
     * the anchor bound removed ("short loop");
     * `let _ = format!("{frames}");` in `render_interleaved` (every phase).
   * Plant `"stretch-cute"` in `app/Cargo.toml`'s engine features, then, on its own, remove `default-features = false`.  Each fails `the_app_ships_only_these_stretchers`.  Restore each.
   * Record how long the `realtime` test binary takes, locally and in CI.
* Docs (DOC-1), in step 2's commit:
  * `invariants.md`, RT-1: the title becomes "the audio callback never waits, frees a player or allocates".  The text adds that after warm-up (`Player::with_config`) the callback never allocates or frees memory, for the stretchers the app ships.  "Why" adds that an allocator can take a lock or fault a page.  `* Enforced by:` adds `the_callback_does_not_allocate_after_warm_up` and `the_app_ships_only_these_stretchers` (CI `test` job).  `* Gaps:` becomes TD-<n>.
  * `CLAUDE.md`: "RT-1: the audio callback never waits for the player slot, never frees a player, and never allocates after warm-up."  The map keeps its line count.
  * `tech-debt.md`: TD-15 is deleted, and `* TD-15: closed by the_callback_does_not_allocate_after_warm_up (harness phase 2, M11). ...` goes under `## Closed`.  TD-<n> is added.
  * `.claude/rules/memory.md`: the callback bullet adds "or allocates after warm-up", and names `engine/tests/realtime.rs`.  In the breaches bullet, "TD-15 and TD-17 are risks" becomes "TD-17 is a risk".
  * `engine/src/player.rs`: the comment on the scratch buffers drops "except for `dsp::double_speed` in chipmunk mode", and the doc of `render_chipmunk` no longer says "with `dsp::double_speed`".
  * `.claude/rules/dsp.md`: the allocation bullet keeps its first sentence.  The second becomes "`wrap_loop` is the one `dsp` function on the callback's path, and it must not allocate.  The player decimates chipmunk audio itself (RT-1)."
  * `docs/decisions/0007-no-allocation-in-the-audio-callback.md`, and its line in `docs/README.md`.
  * `harness.md`: a components row.  `working-with-agents.md`: a changelog line that says how to fix a failure.
  * The parent plan's phase 2 bullet: "(TD-15)" after the allocation-counting test becomes "(RT-1, TD-15, M11)".
  * `ARCHITECTURE.md` does not change.  "Prose only" lines stay at 4.

#### M11 acceptance

* In step 2, before the fixes, `the_callback_does_not_allocate_after_warm_up` failed, and the PR names the phases and counts.  After them, it passes for both stretchers, both rates and the four callback sizes.
* The five planted breaches fail it with the named phases, and the two planted manifest changes fail `the_app_ships_only_these_stretchers`.
* `the_counter_sees_an_allocation`, `the_manifest_check_finds_a_stretcher`, `the_app_ships_only_these_stretchers` and the three unit tests pass.
* The demo song's hashes through wsola are equal before and after the bump, and after step 2.  `Cargo.lock` changes only wsola's entry.
* fmt, clippy, the tests and `docs-check` pass, and the CI `test` job lists the `realtime` tests as `ok`.

### M12: song-sized copies are a lint (MEM-1, MEM-2, DOC-1; maintainer decision)

* Scope: a clippy `disallowed-methods` list for `to_vec`, as the phase plan says, with no new crate.  MEM-1 is a memory rule, so a canary proves that each path in the list is on.  Other forms of copy stay a gap (TD-26).

#### M12 state at the start

On 2026-10-21, at commit `cb1d0be` (M11 merged as PR #25):

* No `clippy.toml` exists.  `app`, `stem-worker`, `stem-protocol` and `engine` take `[lints] workspace = true`, and `[workspace.lints.clippy]` allows two lints.
* CI's `clippy` job lints the native target only.  `worker/src/web.rs` and `app/src/store.rs`, `separator.rs` and `picker.rs` compile only for `wasm32`.  So no clippy lint has ever run in CI on the code of TD-13 and TD-14.
* `cargo clippy --locked --target wasm32-unknown-unknown -p app -p stem-worker -p stem-protocol -p engine --all-targets -- -D warnings` passes today.  It took 26 s and 43 s in two runs on this Mac, with the `wasm32` dependencies already checked in the dev profile.  Trunk's release build does not count, since clippy uses its own check output.
* The `to_vec` calls outside tests:

| Site | What it copies | Size | Reason or entry |
| - | - | - | - |
| `app/src/store.rs:162`, `read` | a stored file, from OPFS into wasm memory, for the decoder | a song | MEM-1: the one copy into wasm memory |
| `app/src/picker.rs:48`, `read` | a chosen file into wasm memory, for the decoder | a song | MEM-1: the one copy into wasm memory |
| `app/src/separator.rs:289` | the MP3 reply, which is copied out again to save it | a stem | TD-13 |
| `worker/src/web.rs:684`, `ModelFile::read` | one range of the model file, such as one tensor | a tensor | MEM-1: the one copy into wasm memory |
| `worker/src/web.rs:1047`, `run` | the transferred mix, for the demixer; the JS arrays drop after | a song | MEM-1: the one copy into wasm memory |
| `worker/src/web.rs:1178`, `tensor_f32` | a tensor's shape | a few numbers | small |
| `worker/src/web.rs:1206` and `1278`, selftest | a reference file, to parse it | a few MB | selftest only |
| `worker/src/web.rs:1285`, selftest | the two channels of the reference chunk | one chunk | selftest only |
| `worker/src/web.rs:1593`, `mp3` | both channels of a stem | a stem | TD-14 |
| `worker/src/web.rs:1642` | the stored kernel choice | a few bytes | small |
| `engine/src/song.rs:208`, `resample_linear` | its input, when the two rates are equal; `Song::from_audio` never calls it so | a stem | MEM-1: the contract returns a new buffer |
| `engine/src/player.rs:118`, `Player::with_config` | the crossfade ramp, once for each player | `crossfade` samples | small |

* In tests: `worker/src/web/tests.rs` has 9 calls and `protocol/tests/web.rs` has 7, each of a few bytes.  The tests of `app` and `engine` have none.  `engine/src/decode.rs:191` calls symphonia's `copy_to_vecs_planar`, which the list does not name.
* `roformer` has 8 calls of `to_vec` in `src/`, of which 5 are on slices; `separate.rs:83`, `model.rs:516` and `540` are Burn's `TensorData::to_vec`.  Only 1 of the slice calls is outside `mod tests`: `weights.rs:169`.  `roformer-gpu` has 4, such as the read-back in `lib.rs:1023` and `bench.rs:829`.  `roformer/tests/` and `roformer-gpu/tests/` have 12 more.  The worker runs both.

Facts checked for M12, with clippy 0.1.99 (rustc 1.99.0), in a scratch worktree:

* The path `slice::to_vec` names `<[T]>::to_vec`.  It catches a method call, a call in a closure, `<[f32]>::to_vec` passed as a function, the call `<[f32]>::to_vec(x)`, and calls on an array or a `&Vec` through auto-deref.  It does not catch `to_owned` on a slice.
* `js_sys::Uint8Array::to_vec` and `js_sys::Float32Array::to_vec` resolve in a `wasm32` build.  Together with `slice::to_vec`, they found each `wasm32` site in the table, and the calls in `worker/src/web/tests.rs`.
* A path that does not resolve does not fail `-D warnings`.  A wrong method on a primitive type (`slice::to_vecc`) and a crate that the build lacks (`js_sy::Uint8Array::to_vec`, or `js_sys` in a native build of `app`) give no message.  `stem-protocol` takes `js_sys` on every target, so a wrong `js_sys` path warns in its native build too.  A wrong type or method in a present crate (`js_sys::Nonexistent::to_vec`, `js_sys::Float32Array::to_vecc`) gives a warning that `-D warnings` does not deny.  Clippy exits 0 in each case.  Clippy's docs say that such a path is an error ([lint configuration](https://doc.rust-lang.org/clippy/lint_configuration.html)).  So only a canary proves that a path is on.
* `#[expect(clippy::disallowed_methods, reason = "...")]` works on a `let` statement, a `return` statement, a match arm and a field of a struct expression.  A stale one fails with "this lint expectation is unfulfilled".
* With `-D warnings`, a lint error in `engine` stops the build before `app` is linted, since `app` takes `engine`.  So a count of every site runs without `-D warnings`.
* `docs-check` takes each code span on an `* Enforced by:` line that has only lowercase letters, digits and `_`, and holds a `_`, as a test name (`enforcing_tests_exist` in `xtask/src/docs.rs`).  So `` `to_vec` `` there fails it, and `` `slice::to_vec` `` does not.
* `xtask web-test` runs `-p app --lib --test web --test web_close --test web_output`, `-p stem-worker --lib` (which holds `web/tests.rs`) and `-p stem-protocol --test web`.
* A change to a crate's `clippy.toml` makes cargo check the crate again.
* Clippy has no option that turns `disallowed-methods` off in tests, as `allow-unwrap-in-tests` does for `unwrap_used` ([lint configuration](https://doc.rust-lang.org/clippy/lint_configuration.html)).
* `docs-check` takes a `#[wasm_bindgen_test]` function as a test name on an `* Enforced by:` line (`TEST_ATTRIBUTES` in `xtask/src/docs.rs`).

#### M12 decision point for the maintainer: which crates

* (a) `app`, `worker`, `protocol` and `engine`.  `protocol` has no site today.  But a decoder in `protocol` that copies a stem is the most likely new breach of MEM-1 and MEM-2.  `roformer` and `roformer-gpu` go into TD-26.
* (b) (a), and `roformer` and `roformer-gpu`.  5 more sites outside tests to read and mark, 17 slice calls in tests, and 5 more module `#![expect]`s.  Each crate gets a canary, so the pairs become 12.  TD-26 drops its `roformer` bullet.  M13 then adds MEM-3's entries to `roformer-gpu/clippy.toml`.
* (c) the phase plan's three crates only: `app`, `worker` and `engine`.  `protocol/clippy.toml`, its canary, its module `#![expect]` and the `protocol.md` bullet go, and `stem-protocol` leaves MEM-1's and MEM-2's lines, `memory.md`'s first bullet, the guide's changelog line and the parent bullet.  The pairs become 7.
* (d) one `clippy.toml` at the root with the three paths, and a file in `roformer/`, `roformer-gpu/` and `xtask/` that holds only a comment, to opt out.  `dsp`, `synth` and `test-support` have no `to_vec`.  So `synth` and a new crate are covered by default, at the same number of files.  Each covered crate needs a canary, so `synth` adds one pair, and `dsp` and `test-support` add one each if they get canaries.  The cost: a root file reaches `dsp`'s lint settings, which DSP-1's file check guards against a `dsp/clippy.toml`; every crate's lint configuration then lives in two places, and M13 writes MEM-3 into an opt-out file.
* Recommendation, first written: (a).  It covers each crate where a breach is known, and the crate that carries arrays between them.  No breach is known in the model crates, and a new crate is rare.
* Chosen: (b), by the maintainer on 2026-10-21.  MEM-1 covers model-sized buffers, and the model crates hold the largest buffers in the tab.  The lint's value is the next copy, not the breaches known today.  The sections below follow (a), and "Under (b)" in the design gives what (b) adds.

#### M12 design

* The files: `app/clippy.toml`, `worker/clippy.toml` and `protocol/clippy.toml` list the three paths.  `engine/clippy.toml` lists `slice::to_vec` only, since `engine` handles no typed array.  Its `js_sys`, under `cpal` on `wasm32`, serves only `Reflect` in `live.rs`.  Each file starts with a comment: "MEM-1 (`docs/invariants.md`).  Clippy reads only this file for this crate.  Each path has a canary: `<test name>` in `<file>`."
* Each entry has a `reason` that names the rule and the fix, which clippy prints as a note.  Each reason is a TOML literal string (`'...'`), since it holds double quotes:
  * `slice::to_vec`: "MEM-1: a copy of a song- or model-sized buffer stays in wasm memory for good.  Fix: borrow the slice or share an `engine::Samples`.  For a needed copy, add `#[expect(clippy::disallowed_methods, reason = "MEM-1: why")]` to its statement, field or match arm."
  * `js_sys::Uint8Array::to_vec` and `js_sys::Float32Array::to_vec`: "MEM-1, MEM-2: this copies a JS array into wasm memory.  Fix: keep the JS array, or copy in blocks with `subarray` and `copy_to`.  For the one copy that Rust must read, add `#[expect(clippy::disallowed_methods, reason = "MEM-1: why")]` to its statement, field or match arm."
* Each site in the table gets an `#[expect(clippy::disallowed_methods, reason = "...")]` on its statement, field or match arm: a `let` (`web.rs:1047`, `1206`, `1278`, `1285`, `1593`, `1642`, `player.rs:118`), a field (`separator.rs:289`), a match arm (`web.rs:684`) or a `return` (`song.rs:208`).  A site in a tail expression gets a `let` first (`picker.rs:48`, `store.rs:162`, `web.rs:1178`).  The reason starts with `MEM-1:`, or with `TD-13:` or `TD-14:` for a breach, and gives the "Reason" column in words.  An `#[expect]` never goes on a function or a module, except in tests.
* Tests: `worker/src/web/tests.rs` and `protocol/tests/web.rs` get `#![expect(clippy::disallowed_methods, reason = "MEM-1: tests copy a few bytes to compare them")]`.  In `protocol/tests/web.rs` it goes after `#![cfg(target_arch = "wasm32")]`; before it, the native step fails with an unfulfilled expectation.  In `worker/src/web/tests.rs` it goes after the module doc and before the first `use`.
* Canaries.  Each is a test with one `let` for each path in its crate's file, each under its own `#[expect]`, with the reason "MEM-1 canary: clippy fails here if this path is off".  Each `let` copies one or two values, and the test asserts them.  An inner `#[expect]` takes its own lint, so the canaries in the test files above still prove their paths.  The canaries:
  * `mem1_lint_is_on` (`#[test]`, in `engine/src/song.rs`'s tests): the slice path.  It is compiled by both the native and the `wasm32` clippy steps.
  * `mem1_lint_is_on_for_js_arrays` (`#[wasm_bindgen_test]`), in `app/tests/web.rs`, `worker/src/web/tests.rs` and `protocol/tests/web.rs`: the three paths.  The `wasm32` clippy step compiles them, and `xtask web-test` runs them.
  * Native builds of `app` and `protocol` get no canary.  The slice path resolves the same way on both targets.
  * A canary proves its path only under a clippy step with `--all-targets`, since it is test code.  Its run in `cargo test` or `xtask web-test` proves nothing about the lint.
* CI: the `clippy` job installs `wasm32-unknown-unknown` (`targets:` on `dtolnay/rust-toolchain`).  Directly after the `--workspace --all-targets` step, a new step runs the `wasm32` command from the state above.  Its comment: "the web code compiles only for wasm32, so the native steps never lint it (MEM-1)".  `engine` is in the step for its `wasm32` code in `live.rs`.
* CI cache: the job's `Swatinem/rust-cache` step gets `with: key: wasm32`.  rust-cache saves only when the restored key is not an exact match (`src/restore.ts` and `src/save.ts` at v2.9.2), and its key does not hash the installed targets ([README](https://github.com/Swatinem/rust-cache/blob/v2.9.2/README.md)).  M12 changes no manifest, so without a new key the job restores its old cache exactly, never saves, and checks the `wasm32` dependencies from cold on each run.  The new key adds one cache entry, against GitHub's 10 GB limit for each repository.
* The gaps, as a new entry TD-26 under "Enforcement gaps", where `<n>` is the next free ID (26 on 2026-10-21), "MEM-1's lint sees only `to_vec`".  Its lead sentence: "Clippy fails `to_vec` on a slice, a `Uint8Array` or a `Float32Array` in four crates (six under (b)), and nothing else."  Its bullets:
  * other forms of copy: `to_owned`, `Vec::from`, `Box::from`, `into`, `concat`, `collect`, `extend_from_slice`, `copy_to` into a new buffer, and `clone` of a `Vec` (TD-12);
  * `to_vec` on the other typed arrays, such as `Int16Array`, and a wasm-bindgen import that returns a `Vec<u8>`;
  * an `#[allow]` in place of an `#[expect]`, as clippy's own help line suggests, and a reason that does not say why;
  * crates whose `clippy.toml` lacks the MEM-1 paths: `synth`, which makes the demo song in the web app, and any new crate.  `dsp` is covered by DSP-1's method list; `xtask` and `test-support` do not ship;
  * the canaries prove their paths only while the clippy steps keep `--all-targets`; nothing fails if the `wasm32` step goes;
  * one `#[expect]` covers each call in its statement, so a new `to_vec` in the same `let` passes; the module `#![expect]`s cover each new test in their files;
  * a `#[wasm_bindgen]` export that takes a `Vec<f32>` or a `&[f32]` copies a JS array in, and passes;
  * Fix: none planned.  Phase 3's memory reviewer looks for a new copy of any form.
* Under (b), the maintainer's choice:
  * `roformer/clippy.toml` and `roformer-gpu/clippy.toml` list `slice::to_vec` only, since neither crate handles a typed array.
  * Neither crate has code behind a target `cfg`, so the native steps lint all of it.  Their dev-dependencies do not build for `wasm32` (getrandom 0.3 needs `wasm_js`), so the `wasm32` step leaves them out.
  * The sites, each with an `#[expect]`: `roformer/src/weights.rs:169` (a shape, for an error), `roformer-gpu/src/lib.rs:1023` (the one copy of a chunk out of mapped GPU memory, which drops next), and `roformer-gpu/src/bench.rs:807` (which tiles are usable), `829` and `830` (the matmul shapes to time).
  * Module `#![expect]`s: the tests of `roformer/src/demix.rs` and `weights.rs`, `roformer/tests/reference.rs`, `roformer-gpu/tests/memory.rs` and `roformer-gpu/tests/reference.rs`.
  * Canaries: `mem1_lint_is_on` in the tests of `roformer/src/weights.rs` and of `roformer-gpu/src/lib.rs`.  So the pairs become 12.
  * The docs name the two crates wherever they name the other four, and `.claude/rules/gpu.md` gets the `memory.md` sentences.  TD-26 drops them from its crate bullet.
* Out of scope: `clippy::allow_attributes`, which would fail an `#[allow]`, since the workspace has other `#[allow]`s.  TD-12, TD-13 and TD-14 are fixed in M18.

#### M12 steps

Each step is one signed commit; fmt, every clippy command in `CLAUDE.md`, the tests and `docs-check` pass before it (GIT-2).

1. The `wasm32` clippy step, alone.  It passes on today's tree.
   * `.github/workflows/ci.yml`: the target, the step (`cargo clippy --locked --target wasm32-unknown-unknown -p app -p stem-worker -p stem-protocol -p engine --all-targets -- -D warnings`) and the cache key.
   * `CLAUDE.md`'s commands, after the last clippy line (the map stays under 100 lines): `cargo clippy --target wasm32-unknown-unknown -p app -p stem-worker -p stem-protocol -p engine --all-targets -- -D warnings  # the web code, which compiles only for wasm32`.
   * `README.md`: the same line, without its `#` comment, in the list after "Then the checks CI gates on:", and in the `## Checks` list without `-- -D warnings`, as that list writes its clippy lines.  The setup table's row "Build the native app, `cargo test`, `cargo clippy`" replaces "Nothing more on macOS." with "The `wasm32` clippy command also needs `rustup target add wasm32-unknown-unknown`."
   * `harness.md`: a components row, "the `wasm32` clippy step in CI | sensor, computational | the web code compiles only for `wasm32`, so no lint ever ran on it in CI, where TD-13 and TD-14 are | phase 2".
   * Record in the PR the step's time in CI, on the first run (cold) and on a re-run (warm).
2. The lint, its sites, the canaries and the docs.
   * Lint first: add the four files (six under (b)), with no `#[expect]`.  Run the native workspace command and the `wasm32` command without `-D warnings`, so that `app` is linted too.  Record each warning; expected: 15 calls on the 13 lines of the table's 12 rows, and the calls in the two test files.  Each message names MEM-1 and the fix.  If a site differs from the table, update the table.
   * Then the `#[expect]`s and the canaries.  Every clippy command in `CLAUDE.md` passes.
   * Plant each of these on the real tree, one at a time, and restore it.  Record each result in the PR:
     * `let _copy = left.to_vec();` in `Song::from_audio`, after the `let Some(left) = ... else { ... };` statement and before `decoded.push`, fails the native step, with the MEM-1 note;
     * `let _copy = mix[0].to_vec();` in `run`, before the `#[expect]` on the copy, fails the `wasm32` step;
     * `bytes: Vec::with_capacity(bytes.length() as usize)` at TD-13's site fails with only "this lint expectation is unfulfilled" there.
   * Turn off each of the 12 (crate, path) pairs (10 under (a)), one at a time: append `c` to the path in that crate's `clippy.toml`.  Run the `wasm32` step for `app`, `stem-worker` and `stem-protocol`, and both steps for `engine`.  The native step passes for the other three, since their native code has no site and no canary.  Each named run fails with "this lint expectation is unfulfilled".  Record the lines for each pair in the PR.  With `-D warnings`, a failure in a crate's lib stops its test targets from building, so record what each run shows.  Expected, from a prototype:
     * engine, slice: `song.rs`, `player.rs` and the canary, on both steps;
     * worker, each path: its sites in `web.rs` and the canary in `web/tests.rs`;
     * app, slice and Float32Array: only the canary in `app/tests/web.rs`, since no site in `app`'s lib uses them.  So the canary proves paths that no real site proves;
     * app, Uint8Array: the lib sites in `picker.rs`, `store.rs` and `separator.rs`;
     * protocol, each path: only its canary in `protocol/tests/web.rs`.
* Docs (DOC-1), in step 2's commit:
  * `invariants.md`, MEM-1: `* Enforced by:` adds "Clippy's `disallowed-methods` (CI `clippy` job) for `slice::to_vec`, `js_sys::Uint8Array::to_vec` and `js_sys::Float32Array::to_vec`, in `app`, `stem-worker`, `stem-protocol` and `engine`, on both targets.  The `#[expect]`s in `mem1_lint_is_on` and `mem1_lint_is_on_for_js_arrays`, under the clippy steps with `--all-targets`, prove each path".  Each method keeps its path in the code span, since `docs-check` takes `` `to_vec` `` alone as a test name.  `* Gaps:` adds TD-26.
  * `invariants.md`, MEM-2: `* Enforced by:` adds "MEM-1's lint on `js_sys::Uint8Array::to_vec` and `js_sys::Float32Array::to_vec`, where the receiver copies".  `* Gaps:` adds TD-26.
  * `tech-debt.md`: TD-26.  TD-13 and TD-14 each add "Its site carries an `#[expect]` for the MEM-1 lint", and give their sites by function, not line number (the `Reply::Mp3` arm in `Separator::new`, and `mp3` in `worker/src/web.rs`), since the `#[expect]`s move the lines.
  * `.claude/rules/memory.md`: the first bullet adds "Clippy fails `to_vec` on a slice, a `Uint8Array` or a `Float32Array`, in `app`, `stem-worker`, `stem-protocol` and `engine`.  A needed copy gets `#[expect(clippy::disallowed_methods, reason = "MEM-1: why")]` on its statement, field or match arm."  `.claude/rules/protocol.md`: the same sentences in a new bullet, since `protocol/` is not in `memory.md`'s paths.  `memory.md`'s paths add `app/clippy.toml`.
  * `harness.md`: a second components row, "clippy's `disallowed-methods` for `to_vec`, with a canary for each path (MEM-1) | sensor, computational | three song-sized copies (TD-12 to TD-14) went unnoticed, and a wrong path in `clippy.toml` passes clippy.  It sees `to_vec` only (TD-26) | phase 2".  `working-with-agents.md`: a changelog line, with the same words for what clippy fails, that says how to fix a failure.  The "Change memory" recipe's check adds "read the reason of each new `#[expect]`".
  * The parent plan's phase 2 bullet: "slice and JS-array `to_vec` in `app`, `worker`, `engine` (MEM-1)," becomes "slice and JS-array `to_vec` in `app`, `worker`, `protocol` and `engine`, on both targets (MEM-1, M12);".
  * `ARCHITECTURE.md` does not change.  "Prose only" lines stay at 4.

#### M12 acceptance

* In step 2, before the `#[expect]`s, the two clippy commands, run without `-D warnings`, warn at the 15 calls of the table and in the two test files, each with the MEM-1 note.  After them, every clippy command in `CLAUDE.md` passes, locally and in CI's `clippy` job.
* The three plants and the 12 (crate, path) pairs of (b) fail as stated.
* `mem1_lint_is_on` passes in the CI `test` job, and `mem1_lint_is_on_for_js_arrays` passes in `browser tests (Chrome)` for each of the three crates.
* fmt, the tests and `docs-check` pass.

### M13: GPU buffers are made only by GpuBuffer (MEM-3)

* Change: add `roformer-gpu/clippy.toml` with `disallowed-methods` for `wgpu::Device::create_buffer` and `wgpu::util::DeviceExt::create_buffer_init`, with a MEM-3 reason.  `GpuBuffer`'s constructor carries the one `#[expect]`.
* Acceptance: clippy passes.  A planted `device.create_buffer(...)` in `net.rs` fails clippy with the MEM-3 reason.

### M14: Claude Code deny rules (GIT-1)

* Scope, set by the maintainer on 2026-10-20: deny rules only.  The first plan added a PreToolUse hook in `xtask`, run on every tool call.  Hooks match command text and are not a security boundary, and the controls stay M8's CI test (ASSET-1) and the `main` ruleset (GIT-1).
* Change: add `.claude/settings.json` with deny rules for `Bash(git push *)` and the `publish` forms of `cargo run -p xtask`.  If a deny pattern can tell `--body` from `--body-file`, add one for `gh pr create --body` (failure 7 in the phase plan); else leave it.
* Acceptance: in a session, `git push --dry-run` and `cargo run -p xtask -- publish --help` are refused.  Record both in the PR.  GIT-1's `* Enforced by:` names the deny rules and says that they match command text only.
* Docs: a "What the agent cannot do" note in the guide says what is refused and why.

### M15: rustfmt after each edit

* Scope, set by the maintainer on 2026-10-20: the `rustfmt` hook only.  The Stop hook (fmt and `docs-check` at each stop) and the SessionStart WebDriver report are dropped.
* Change: in `.claude/settings.json`, a PostToolUse hook on `Edit|Write` runs `rustfmt --edition 2024` on the edited file when it ends in `.rs`.  It is a shell command that reads `tool_input.file_path` from stdin, not an `xtask` command.  The control stays CI's `fmt` job; the hook saves a round trip.
* Acceptance: in a session, an edit that leaves a `.rs` file unformatted is formatted, and an edit of a `.md` file is not touched.  Record both in the PR.

### M16: moved to phase 5

* The test-count guard moves to phase 5b, where bots open PRs (maintainer, 2026-10-20).  No deleted test has been seen in this repo; vacuous tests have, and phase 3 targets them.  The guard also doubles the `test` job's build.

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

* `grep -c "Enforced by: prose only" docs/invariants.md` falls from 8 to 3.  BUILD-1, DEPS-1, LAYER-1 and DSP-1 gain tests, and GIT-1 gains deny rules; ASSET-1 and RT-1 gain tests beside what they have.  MEM-4, GPU-3 and GIT-3 stay prose only: phase 4 covers MEM-4 and GPU-3, and nothing in view checks GIT-3.
* Each check's planted breach fails it: the unit tests in `xtask/src/rules.rs`, `test-support`, `roformer-gpu` and `engine/tests/realtime.rs`, and the clippy cases stated in each PR.
* `cargo test --workspace --exclude roformer-gpu`, `cargo test -p roformer-gpu --lib` and `--doc`, clippy and `docs-check` pass; CI is green on `main` with browser tests required (the phase gate).

## Idempotence and recovery

* Each milestone is one PR and reverts on its own.  A reverted check removes its enforcement and nothing else; revert its `* Enforced by:` line and restore its tech-debt entry in the same commit.
* M3 adds a crate.  To undo M3, revert its commits, then put back decision 0003 with `Status: rejected`, and its line in `docs/README.md`.  So the decision numbers have no gap.  If M4 or a later milestone is in the tree (merged and not reverted), keep `xtask/src/rules.rs` and `xtask/tests/rules.rs`.  Remove `skips_use_the_helper`, `allowlisted_skips_exist` and their unit tests.  Remove the private items that only they use: `SKIP_WORDS`, `holds_skip_words_on_purpose`, `SKIP_ALLOWLIST`, `skips_in`, `string_literals`, `string_value`, and the `FromStr` and `proc_macro2` imports.  Remove `rules::rust_files`, unless a later check uses it, the `rust_files` helper in `xtask/tests/rules.rs`, and the tests `every_skip_can_be_required` and `the_skip_allowlist_is_current`.  Otherwise the build or clippy fails on dead code.  Remove `proc-macro2` from `xtask`'s `[dependencies]` unless a later check uses it.  Then commit `Cargo.lock`.  If M5 is in the tree, also remove these:
  * in `ARCHITECTURE.md`: the `Test support` row, the sentences on `test-support` under the layers table, from "Any crate but `dsp`" to "It takes no workspace crate.", ", apart from `test-support`" in the sentence "Each row lists ...", and ", and on `test-support` as a dev-dependency" above the table;
  * in LAYER-1: "Test support, " in the list of layers, ", and on `test-support` as a dev-dependency", and its `test-support` sentence;
  * in `rules.rs`: "Test support" in `LAYERS`, which becomes `[&str; 4]`, "five layers" in the layers message, the `Test support` cases of `the_layers_are_fixed`, the `test-support` rules in `crates_follow_the_layers`, their messages and unit tests, the shell test `a_crate_reached_through_stem_protocol_is_found` (or give it another crate), and the fixture's `Test support` row, `test-support` crate and `test-support` edges;
  * M5 rewrites LAYER-1 and `ARCHITECTURE.md`, so a plain revert of M3's text conflicts there.  Edit them by hand.
* To undo M4, revert its commits.  That reverts the code, the manifests, the CI steps and the command lists in every case.  The revert also puts back the `hound` row, TD-4's text and LAYER-1's text.  If a later check shares a Cargo runner with `web_app_tree`, keep that runner.  Then put back decision 0004 with `Status: rejected`, and its line in `docs/README.md`, so the decision numbers have no gap.
* If M5 is in the tree when M4 is undone, TD-4 is closed, and `docs-check` fails a closed ID outside plans and decisions.  So keep TD-4 closed, and do not put back its text.  The sentence that comes back under the layers table in `ARCHITECTURE.md` must not name TD-4.  Keep `rules::WEB_APP`, because M5's shell check uses it.  M5 rewrites LAYER-1 and edits the same lines in `harness-engineering.md` and `harness.md`, so a plain revert of M4 conflicts there.  Resolve the conflicts by hand.  In LAYER-1, remove only M4's last sentence of the text, M4's sentence in "Why" and M4's test name, and fix the list's "and".  Keep M5's text.
* To undo M5, revert its commits.  That puts back the old table, TD-4 and LAYER-1's lines, and removes TD-4's bullet under `## Closed` and the new crates entry.  Keep the new crates entry's ID, so it is never reused and the plan still names a defined ID: add `* TD-<n>: withdrawn by the revert of M5; TD-4 covers the crate edges again.` under `## Closed`.  Do the same for option (c)'s entry, if it exists.  Put decision 0005 back with `Status: rejected`, and its line in `docs/README.md`, so the decision numbers have no gap.  Keep this plan's living sections, and add a changelog line to the guide that says the check was reverted.  Point TD-4's fix line at the plan's current folder, `active/` or `completed/`.  If a later milestone added an edge between workspace crates, add it to the old table too (DOC-1).
* To undo M6, revert its commits.  That puts back the allow, TD-2, TD-3's text and DSP-1's text.  It removes TD-2's bullet under `## Closed`, decision 0006 and TD-24.  The breach stays, so restore TD-24 by hand, and keep its ID in DSP-1's `* Gaps:` line.  Keep the narrowed `* Enforced by:` text, because "found no breach" is false while the breach stays.  Keep this plan's living sections.  Put decision 0006 back with `Status: rejected`, and its line in `docs/README.md`, so the decision numbers have no gap.  If M7 is in the tree, its allowlist fails the blanket allow, so revert M7 first, or keep M6.
* To undo M7, revert its commits.  That removes the check, and puts back TD-24, TD-3's text and DSP-1's text.  Keep this plan's living sections.
* To undo M10, revert its commits.  That removes the check, the `harness.md` row and the guide's changelog line.  It puts back TD-16's text, PROTO-1's `* Enforced by:` line and the line in `.claude/rules/protocol.md`.  Keep this plan's living sections, and add a changelog line to the guide that says the check was reverted.
* To undo M11, revert its commits.  That puts back wsola 0.1.0, its `THIRD-PARTY.md` row, TD-15, RT-1's text and the rule files.  It removes decision 0007, TD-<n> and the components row.  Put decision 0007 and its line in `docs/README.md` back as they were, still accepted: a revert for a bug does not reverse the maintainer's choice, and TD-15's fix line says so.  Keep TD-<n>'s ID: add `* TD-<n>: withdrawn by the revert of M11; TD-15 covers the gap again.` under `## Closed`.  Keep this plan's living sections, and add a changelog line to the guide that says the check was reverted.
* To undo M12, revert its commits.  That removes the six `clippy.toml` files, the `#[expect]`s, the canaries and the `wasm32` step, and puts back MEM-1's and MEM-2's lines.  Put decision 0008 and its line in `docs/README.md` back as they were, still accepted: a revert for a bug does not reverse the maintainer's choice.  Keep TD-26's ID: add `* TD-26: withdrawn by the revert of M12.` under `## Closed`.  Keep this plan's living sections, and add a changelog line to the guide that says the check was reverted.
* Checks read the tree and write nothing, so they can run any number of times.
* M11 changes a rule that the maintainer chose on 2026-10-19.  If they reverse it, revert M11's rule text and decision, and keep the test as a measurement, marked in `tech-debt.md`.
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
* [x] 2026-10-19: M3, decision 0003 accepted by the maintainer.
* [x] 2026-10-19: M3, PR #16 merged by the maintainer (`69f8cae`).
* [x] 2026-10-19: M4 implementation plan written (the M4 sections under "Plan of work").  A prototype of the change in a scratch worktree measured the licence list, the size of `app_bg.wasm` and `cargo tree`'s output.
* [x] 2026-10-19: M4 plan review round 1, by two new reviewers (code, and docs and process).  No blockers; 8 should-fix findings and 14 nits, all checked and real, all fixed in the plan.  The main fixes: CI's `CARGO_TERM_COLOR: always` colours `cargo tree`'s `(*)` markers, so the check passes `--color never`.  `--workspace` builds hide a lost dev-dependency, so CI's `clippy` job runs clippy on `synth` and on `app` alone.  "The web app" (`app` and `stem-worker`) is now a defined term.  M4 writes decision 0004.
* [x] 2026-10-19: M4 plan review round 2, by two new reviewers.  No blockers; 9 should-fix findings and 11 nits, all checked and real, all fixed in the plan.  The main fixes: one `cargo tree` run per package, since one run with both put a `stem-worker` breach under `app` (shown in a prototype).  Decision 0004's options are written out.  LAYER-1's line names only the test.  `README.md` and `CLAUDE.md` get the two clippy commands.  The undo text covers M5.  Three more parser tests: a two-digit depth, a deeper `synth`, and a `stem-worker` breach.
* [x] 2026-10-19: M4 plan review round 3, by two new reviewers.  No blockers; 5 should-fix findings and 13 nits, all checked.  One should-fix (a two-digit depth that a one-digit parser passed) was already fixed by a staged change, which puts `synth` itself at depth 10.  The rest were real and are fixed in the plan.  The main fixes: LAYER-1's new text says that no crate of the web app takes `files`, since `stem-worker` does not take `synth`.  The undo text keeps a closed TD-4 closed.  The planted `stem-worker` breach runs without `--locked`, since it changes `Cargo.lock`.  A tree that does not start with its package is a problem, so empty output cannot pass.  M5 needs no allowlist entry for `app`'s dev-dependency.
* [x] 2026-10-19: M4 plan review round 4, by two new reviewers.  No blockers; 3 should-fix findings and 7 nits, all checked and real, all fixed in the plan.  The should-fix items were all unit-test gaps.  A parser that ends `synth`'s children only at a shallower line, or that reads only the first `synth`, passed every planned test.  A test's expected count also left out the "no `synth`" problem.  The tests now catch each.  The nits gave exact texts for the command lines and the parent plan's bullet, and a note in M9 on `--color never`.
* [x] 2026-10-19: M4 plan review round 5, by two new reviewers.  No blockers; 3 should-fix findings and 7 nits, all checked and real, all fixed in the plan.  A code fence had text after its closing backticks, so the rest of the plan rendered as code; every fence line is now clean.  A parser that skips `(*)` lines passed every test; `synth_under_stem_worker_is_named` now uses the real lines of the planted breach.  "The web app" is defined at its first use in each doc that M4 changes.  The command lines are given for each of the three lists.
* [x] 2026-10-19: M4 plan review round 6, by two new reviewers.  No blockers; 2 should-fix findings and 3 nits, all checked and real, all fixed in the plan.  The code reviewer found no should-fix item; 17 wrong parsers in its model each fail a planned test.  LAYER-1's new text now says "as a normal dependency", because M4's own dev-dependency takes `files`.  The parent plan's new bullet defines "the web app".  `clippy -p app --all-targets` took 55 s after a workspace clippy.
* [x] 2026-10-19: M4 plan review round 7, by two new reviewers.  No blockers; 2 should-fix findings and 7 nits, all checked and real, all fixed in the plan.  The docs reviewer found no should-fix item.  `-p app --all-targets` turns `files` on for `app`'s lib too, so a native-only call to a writer passed CI; a third step, `cargo clippy -p app`, catches it.  Decision 0004's option (c) had a false cost: `engine::wav::encode_pcm16` already writes WAV files.  LAYER-1's text now covers the whole normal dependency tree.
* [x] 2026-10-19: M4 plan review round 8, by two new reviewers.  No blockers; 1 should-fix finding and 11 nits, all checked and real, all fixed in the plan.  The docs reviewer found no should-fix item.  The should-fix was a regression from round 7: a nit's fix dropped "as a normal dependency" from LAYER-1's text, so M4's own dev-dependency broke the rule.  LAYER-1's sentence was then limited to the web app's normal dependency tree.  The code reviewer found no CI job or command where a breach passes unseen, and no wrong parser that passes the ten tests.
* [x] 2026-10-19: M4 plan review round 9, by two new reviewers.  No blockers; 2 should-fix findings and 7 nits, all checked and real, all fixed in the plan.  LAYER-1's sentence now says only what the check reads: in the web app's normal dependency tree, `synth` brings no other crate.  The check's `-f "{p}"` prints no features, so the sentence no longer names the `files` feature.  The new "Why" no longer says that `THIRD-PARTY.md`'s first table lists only what ships, since it also lists build-only crates such as `syn`.  Measured in CI order: `clippy -p app` took 92 s after a workspace clippy, then `-p app --all-targets` took 8 s.
* [x] 2026-10-19: M4 plan review round 10, by two new reviewers.  No blockers; 1 minor should-fix finding and 10 nits, all checked.  The should-fix: the licence-list diff holds only when `cargo tree`'s output is made into a set of rows, and the plan now gives that command.  8 nits were fixed; 2 optional ones were left out of M4's scope (the parent plan's CI bullet, and DEPS-1's wording of the web app).
* [x] 2026-10-19: the maintainer judged the plan good enough after round 10 (no blockers and no significant should-fix), and asked to commit it and start M4.
* [x] 2026-10-19: M4 step 1: `web_app_tree` and `synth_brings_no_crate`, test first.  Against a stub, the 7 tests that expect a problem failed and the 3 that expect none passed.  With the function, all 10 pass.
* [x] 2026-10-19: M4 step 2: `synth`'s `files` feature, `synth_brings_no_crate_into_the_web_app`, the three CI steps and the docs.  Before the change, the real-tree test failed: "synth brings anyhow, hound, serde, serde_json into the web app, through app".  After it, the test passes, and `Cargo.lock` does not change.  `app_bg.wasm` went from 5,194,829 to 5,194,587 bytes.  The licence command's set of rows equals the first table (128 rows), and `hound` is in neither table.  "Prose only" lines: 7.
  * Planted breaches, each run locally and restored.  (1) `files` on `app`'s `synth`: the test fails and names `hound`.  (2) `default = ["files"]`: the test fails.  (3) No dev-dependency: `clippy -p app --all-targets` fails with E0425, and `--workspace` passes.  (4) An ungated `use serde::Serialize;`: `clippy -p synth` fails with E0432.  (5) A native-only call to `write_song` in `app`: `clippy -p app` fails with E0425, and `--all-targets` passes.  (6) `synth` with `files` in `stem-worker`'s wasm dependencies, run without `--locked`: the test fails and names `stem-worker`.
* [x] 2026-10-19: M4 review round 1: no blockers and no should-fix items; 3 nits.  (1) No test covered the depth-0 rule or a line of spaces: removing `first.depth == 0` or the `trim` passed every test.  `a_first_line_below_depth_0_is_a_problem` and a line of spaces in `a_line_without_a_depth_is_a_problem` now fail each mutant.  (2) `Problem.file` is not printed; the messages name the package, as the plan gives them, so this stays.  (3) Decision 0004 said "the download"; it now says `app_bg.wasm`.
* [x] 2026-10-19: M4 review round 2: no blockers and no should-fix items; 2 nits, both real and fixed.  (1) Matching `synth` by prefix in the children check passed every test; the `synthx` line in `a_tree_without_synth_is_a_problem` now has a child, and that mutant fails.  (2) The plan still said 10 unit tests; it lists the 11th and says 11.
* [x] 2026-10-20: M4 review round 3: no blockers; 1 should-fix and 4 nits, all checked.  The should-fix was real: without `--target wasm32-unknown-unknown`, `stem-worker`'s tree is its root line alone and passes every check, so a breach in the worker passed.  A tree with no `wasm-bindgen` at depth 1 is now a problem.  With the target removed, the real-tree test fails for both trees.  `a_tree_without_wasm_bindgen_is_a_problem` fails a mutant without the check and a mutant that accepts any depth.  Two nits were test gaps (`synth_seen` read from the last tree only, a root on any line); both mutants now fail.  One nit (equivalent mutants) needed no change.  The plan now quotes decision 0004's sentence on `app_bg.wasm`.
* [x] 2026-10-20: M4, three review rounds done and their findings fixed.
* [x] 2026-10-20: M4: PR #17 opened.  CI (run 39331145364): every job passes.  The `test` job lists `synth_brings_no_crate_into_the_web_app ... ok`; 43 test results, none failed.  The `clippy` job runs the three new steps.
  * Step times: `-p synth --all-targets` under 1 s, `-p app` 1m30s, `-p app --all-targets` 22 s.  So the steps add 1m52s, against the plan's estimate of about 100 s locally.
  * The `clippy` job took 5m22s, against 3m35s on `main` (run 39248866763).  In this run it is the longest job, 31 s more than `test` (4m51s).
* [x] 2026-10-20: M4, decision 0004 accepted by the maintainer.
* [x] 2026-10-20: M4, PR #17 merged by the maintainer (`054c003`).
* [x] 2026-10-20: M5 implementation plan written (the M5 sections under "Plan of work").  Prototypes in a scratch worktree ran `cargo metadata --no-deps` with planted edges.
* [x] 2026-10-20: M5 plan review round 1, by two new reviewers (code, and docs and process).  No blockers; 9 should-fix findings and 14 nits, all checked and real, all fixed in the plan.  The main fixes: a path crate excluded from the workspace, or a git crate with a member's name, passed every check, so such strays are now problems.  The second check is a new reading of LAYER-1, so it is now a decision point for the maintainer, with decision 0005.  Tests now assert each problem's file, two edges to one crate, and walks through `stem-protocol` and at every depth.
* [x] 2026-10-20: M5 plan review round 2, by two new reviewers.  No blockers; 12 should-fix findings and 15 nits, all checked and real, all fixed in the plan.  The main fixes: a `[patch]` to a path crate outside the members passed every check, so the check now reads `cargo metadata` without `--no-deps`, and a path package that is not a member is a problem.  The plan now says exactly what each case gives, so each test's expected problems are exact; several tests had wrong counts.  Mutants that keep the last kind, drop build edges, or walk to a fixed depth now fail.  Decision 0005's options are written out, and the undo text for M3 and M4 works after M5.  A new tech-debt entry records what a check of crate edges cannot see.
* [x] 2026-10-20: M5 plan review round 3, by two new reviewers.  No blockers; 11 should-fix findings and 15 nits, all checked and real, all fixed in the plan.  The main fixes: a `[patch]` crate behind a feature that is off passed every check, so the check adds `--all-features`.  A relayered `test-support` could carry a dev edge upwards, so `test-support` now takes no workspace crate.  LAYER-1's new first sentence was false for the dev edges to `test-support`.  Three wrong implementations passed every planned test, and new tests now fail them.  The undo text now keeps every ID defined, and removes the code that M3's revert would leave dead.
* [x] 2026-10-20: M5 plan review round 4, by two new reviewers.  No blockers; 6 should-fix findings and 18 nits, all checked and real, all fixed in the plan.  The main fixes: a git crate that takes a member's name, with a `[patch]` back to the member, passed every check, so the check also reads `resolve.nodes`.  The two kinds of stray now have their own type, and each message has a test.  The rule text said "crates" where it means workspace crates, since the two sides share registry crates such as `wasm-bindgen`.  LAYER-1 now states the rule on strays.
* [x] 2026-10-20: M5 plan review round 5, by two new reviewers.  No blockers; 6 should-fix findings and 21 nits, all checked and real, all fixed in the plan.  The should-fix items were all details of the text: `outside` and `outside_takes` overlapped for a path crate, a parser with `Path::starts_with` passed every test, and the M3 undo text left dead code and a broken sentence.  Also, option (b)'s cost now says that it removes a sentence of LAYER-1, and the first sentence of LAYER-1 now says "workspace crates".
* [x] 2026-10-20: M5 plan review round 6, by two new reviewers.  No blockers.  The code reviewer found no should-fix item, and no realistic breach outside the known gap.  The docs reviewer found 4 should-fix items, and the two reviewers found 21 nits.  All were checked and real, and all are fixed in the plan.  Decision 0005 now states that option (a) also fails a native-only edge.  The plan no longer says that the maintainer accepted an exact table: the accepted text gave a ceiling, so the decision point now asks about it.
* [x] 2026-10-20: M5 plan review round 7, by two new reviewers.  No blockers; 7 should-fix findings and 19 nits, all checked and real, all fixed in the plan.  The main fixes: an agent that follows the rank message could add a new layer, pass every check and make LAYER-1's list of layers false, so `rules::LAYERS` now fixes the layers.  Round 6's question on an exact table had no change list, and its record text was false; the decision points now hold two questions, each with its changes, and decision 0005 records both for any choice.  The M3 undo text again removes `proc-macro2` whether or not M5 is in the tree.
* [x] 2026-10-20: M5 plan review round 8, by two new reviewers.  No blockers.  7 should-fix findings and 15 nits, all small fixes to round 7's new text, all checked and real, all fixed in the plan.  The layers problem got its file, and `the_layers_are_fixed` now fails a set comparison.  The stale-entry direction got a planted breach.  Question 2's ceiling now states its gain.  The layers message and LAYER-1 use the names in `LAYERS`.  The code reviewer found no realistic breach outside the known gap.
* [x] 2026-10-20: plan review stopped after round 8, by the maintainer's rule: no blockers, and no should-fix item beyond small local fixes.  Not yet implemented.  Next: the maintainer answers M5's two decision questions.
* [x] 2026-10-20: M5 decision points: the maintainer chose (a) and (x), and asked to note (a)'s native cost as a choice to revisit.  Decision 0005 gets a "Revisit if" section for it.
* [x] 2026-10-20: M5 step 1: the parser and the two checks, test first.  Against stubs, 39 of the 45 new unit tests failed; the 6 that expect no problem passed.  With the functions, all 70 tests in `rules.rs` pass.
* [x] 2026-10-20: M5 step 2: the real-tree tests, the new table and the docs.  On the old table, `crates_depend_only_downwards` failed with the header problem only.  On the new table, both tests pass.  Each takes about 0.75 s, most of it `cargo metadata`.  The new crates entry is TD-23.  "Prose only" lines: 7.
  * Planted breaches, each run on the real tree and restored.  (1) `app -> roformer`: both tests fail, and name `roformer`.  (2) The same, with `roformer` in `app`'s row: only the shell test fails.  (3) A dev `engine -> app` edge: "(dev)".  (4) A dev `dsp -> test-support` edge: the DSP-1 message.  (5) An excluded path crate `shim`: the path stray and the path crate outside.  (6) A `[patch]` to `shim`, and (7) the same behind a feature that is off: the path crate outside.  (8) A local git crate `foo` that takes `roformer`, with a `[patch]` back to the member: the outside crate that takes a member.  (9) `dsp` removed from `app`'s row: the unlisted edge.  (10) `stem-protocol`'s row removed: the crate without a row.  (11) `synth` added to `engine`'s row: the stale entry only.
* [x] 2026-10-20: M5 code review round 1: no blockers; 1 should-fix and 8 nits, all checked.  The should-fix was real: building the layer list, or the check that a layer's rows are together, from every readable row passed every test.  `a_row_left_out_is_left_out_of_the_layer_checks` fails both mutants.  Six more mutants now fail a test: no de-duplication of `test-support`'s edges, a `dsp` rule that skips build edges, a separator of two cells or with an empty cell, a heading with trailing spaces, and an empty code span.  A header with no separator now has its own message.  The two real-tree tests share one `cargo metadata` run.  Not changed: `table_problems` stays one function of about 190 lines; it reads in the order of "What each case gives".
* [x] 2026-10-20: M5 code review round 2: no blockers; 1 should-fix and 2 nits, all checked and real.  The should-fix: with `--no-deps` added, or `--all-features` removed, every test passed, and the `[patch]` breaches then passed unseen.  `resolve` is now required, and each member's node must list all its declared features.  Both mutants now fail the real-tree tests with a message that names the flag.  Nits: `a_crate_without_a_row_is_a_problem` now also fails a check that counts edges to a crate without a row.  Decision 0005 names the cost of the member-name rule: crates.io has crates named `app`, `dsp`, `engine`, `synth` and `xtask`.
* [x] 2026-10-20: M5 code review round 3: no blockers and no should-fix items; 5 nits, all checked and fixed.  A member without a resolve node, and a later `###` heading, now have tests that fail their mutants.  The real-tree helper's message and comment no longer claim more than they check.  A sentence of decision 0005 was over 25 words.  The reviewer also found the feature check correct for optional dependencies, with and without the `dep:` form.
* [x] 2026-10-20: M5, three code review rounds done and their findings fixed.
* [x] 2026-10-20: M5: PR #18 opened.  CI (run 39360333192): every job passes.  The `test` job lists both new tests as `ok`, and the `rules` binary took 2.17 s.  rust-cache found no cache, because its environment hash changed on `main` before this PR (`296abb86` to `30197891`), so `test` took 10m1s and `clippy` 7m12s.  `cargo metadata` passed on that cold cache.
* [x] 2026-10-20: M5, decision 0005 accepted by the maintainer, with the choice of (a) and (x).
* [x] 2026-10-20: M5, PR #18 merged by the maintainer (`df8a954`).
* [x] 2026-10-20: M6 implementation plan written (the M6 sections under "Plan of work").  A prototype in a scratch worktree ran clippy on `dsp` without the allow, with the four `expect`s, and with each planted breach.
* [x] 2026-10-20: M6 plan review round 1, by two new reviewers (code, and docs and process).  No blockers; 10 should-fix findings and 9 nits, all checked and real, all fixed in the plan.  The main fixes: `#![expect(clippy::all)]` and an `expect` on one function also pass clippy, so M7's rule now names each lint and fails groups.  DSP-1 now names the beginner forms, so each reason cites its text.  The types outside DSP-1's list get a breach entry.  M12's and M13's lists never reach `dsp`; the gain no longer claims it.
* [x] 2026-10-20: M6 plan review round 2, by two new reviewers.  No blockers; 9 should-fix findings and 12 nits, all checked and real, all fixed in the plan.  The main fixes: a `[lints]` table in `dsp/Cargo.toml`, a lint attribute on a statement or at the top of a module file, and `cfg_attr` at crate level also switch lints off, so M7 checks them too.  Under (x), DSP-1 gains a ban, so decision 0006 records the choice.  The types breach now names `f32`, and the undo text keeps it.
* [x] 2026-10-20: M6 plan review round 3, by two new reviewers.  No blockers in M6.  1 should-fix in M6: TD-3's new text named `.cargo/config.toml`, which `docs-check` fails as a missing path.  2 should-fix items in M7's bullets: `syn` does not enter macros, and nothing planted `is_empty` or `clamp`.  11 nits.  All checked and real, all fixed in the plan.  The docs reviewer applied every M6 doc edit to a scratch copy, and `docs-check` then passed.
* [x] 2026-10-20: M6 plan review round 4, by two new reviewers.  No blockers; 3 should-fix findings and 11 nits, all small fixes to the text, all checked and real, all fixed in the plan.  (y) now changes DSP-1's `* Enforced by:` and `* Gaps:` lines too.  A form added to DSP-1 needs a `proposed` record.  M7 compares the whole `dsp/Cargo.toml`, so no `toml` crate is needed.  The docs reviewer applied every doc edit for (x) to a scratch tree, and `docs-check` passed.
* [x] 2026-10-20: plan review stopped after round 4, by the maintainer's rule: no blockers, and no should-fix item beyond small local fixes.  Not yet implemented.
* [x] 2026-10-20: M6 decision point: the maintainer chose (x), all four lints kept as `expect`s.  The agent had offered (z), which fixed only `assign_op_pattern`, because the Rust Book teaches `+=`.  The maintainer kept (x).  Decision 0006 records it as accepted when step 2 writes it.
* [x] 2026-10-20: M6 step 1: on today's tree, `let n = n;` and `n == n` planted in `fade_in` pass clippy, and so does the planted `dsp/clippy.toml`.
* [x] 2026-10-20: M6 step 2: the four `#![expect]`s, decision 0006 and the docs.  The breach entry is TD-24.  clippy, `RUSTFLAGS="-D warnings" cargo test -p dsp` (30 tests), the wasm release build and `cargo doc` pass.  "Prose only" lines: 7.
  * Planted breaches, each run and restored.  `let n = n;` fails with `redundant_locals`, and `n == n` with `eq_op`.  The `dsp/clippy.toml` gives 4 errors in the lib and 9 in the lib test.  Each `expect` removed fails its lint: 25, 4, 2 and 2 errors.  The one-line swap gives one unfulfilled expectation, and `allow(clippy::all)` after the `expect`s gives four.
  * The known gap, each with `n == n` planted, passes as the plan says: `allow(clippy::all)` before the `expect`s, `expect(clippy::all)`, `expect(eq_op)` on `fade_in`, `allow(eq_op)` on the `let`, and `[lints.clippy] all = "allow"`.  TD-3 names it until M7.
* [x] 2026-10-20: M6 review round 1: no blockers and no should-fix items; 3 nits, all real and fixed.  (1) The `assign_op_pattern` `expect` also silences `x = x * y`, `/` and `%`; a planted `amount = amount / n as f32;` passed clippy, while DSP-1 named only `+` and `-`.  DSP-1, the rule file, the reason, TD-3 and decision 0006 now cover every compound assignment.  (2) TD-3 says "the four `expect`s".  (3) The `harness.md` cell uses the past tense.
* [x] 2026-10-20: M6 review round 2: no blockers; 1 should-fix and 6 nits, all checked.  The should-fix was real: round 1 widened decision 0006's ban after the maintainer accepted it, with nothing that marked the change.  0006 now has a dated "Amendment" section, and the PR asks the maintainer to confirm it.  A planted `q = q << 1`, `q = q | 1` and `m = m / 2.0` passed clippy, which confirms the widened ban.  Five nits were fixed: STE lengths in DSP-1 and TD-3, a false past tense in `harness.md`, a note on the quoted texts in "M6 design", and M7's "arithmetic".  One was left: `CLAUDE.md`'s DSP-1 line stays a summary, and stays true.
* [x] 2026-10-20: M6 review round 3: no blockers; 1 should-fix and 3 nits, all checked and real.  The should-fix: Surprises lacked round 1's finding on `assign_op_pattern`; it now has it.  Nits: the Decision log's plan-review entries say "plan review"; the rule file says what to do with a lint that fires only in a test; the PR description says "keeps its workspace level", not the first commit's "every other clippy lint now runs".
* [x] 2026-10-20: M6, three review rounds done and their findings fixed.
* [x] 2026-10-20: M6, the maintainer accepted the amendment to decision 0006: the ban covers every compound assignment.
* [x] 2026-10-20: M6: PR #19 opened.  CI (run 39373923147): every job passes, with no new step.  `clippy` took 5m56s and `test` 4m12s.
* [x] 2026-10-20: M6, PR #19 merged by the maintainer (`6f9c337`).
* [x] 2026-10-20: M7 plan first written as a `syn` allowlist (about 200 lines).  The maintainer judged it out of proportion, and chose a token scan of the likely breaches.  M7 rewritten; a prototype with `proc-macro2` 1.0.107 confirmed the token facts.
* [x] 2026-10-20: M7 plan review round 1, by one reviewer with a prototype of the scan.  No blockers.  The real tree passes the prototype, and the three planted breaches fail it.  2 should-fix findings and 4 nits, all checked and real, all fixed in the plan.  `to_vec`, `sort_by`, `min` and `max` passed a denylist, so methods after `.` are now an allowlist.  `for s in &samples[1..]` passed when `..` was searched inside groups.  Not changed: `needless_range_loop` does not fire, because the workspace allows it.
* [x] 2026-10-20: M7 step 1: `dsp_files`, the check and its three unit tests.  Against the stub, `each_breach_is_named` and `the_manifest_and_files_are_fixed` failed, and `beginner_code_passes` passed.  With the check, all three pass.  Two small additions to the design: a `for` with no `in`, as in `impl A for B`, is not a loop; a listed `#![expect]` passes only at the top level of `dsp/src/lib.rs`.
* [x] 2026-10-20: M7 step 2: `dsp_is_beginner_style` passes on the real tree, and the docs follow it.  Planted one at a time, each breach gave one problem: `samples.iter().count();` in `biggest` (`dsp/src/biggest.rs:8`, the method `iter`), `i += 1;` in `double_speed` (`dsp/src/double_speed.rs:9`, `+=`), and `#![allow(clippy::all)]` (`dsp/src/lib.rs:24`, `allow`).  "Prose only" lines: 6.  `Cargo.lock` did not change.
* [x] 2026-10-20: M7 review round 1, by one new reviewer, who ran about 30 inputs through the check.  No blockers and no should-fix items; 3 nits, all checked and real, all fixed with test cases.  `use std::mem::{replace, swap};` passed, because `swap` is not straight after `::`; a name first in an import's braces, or after a `,` there, now counts.  `#![r#allow(...)]` and `r#include!` passed; the check now strips `r#` there too.  `dsp/Cargo.toml` with CRLF line ends failed; the check now compares it with LF line ends.
* [x] 2026-10-20: M7 review round 2, by one new reviewer, who ran 33 inputs through the check.  No blockers and no should-fix items; 4 nits, all checked and real.  `pub use max::max;` failed, so a name after its own module now passes, with a case in `beginner_code_passes`.  A glob import such as `use std::mem::*` hides `swap`; TD-3 now names it.  DSP-1 in `invariants.md` now names the three methods.  The rule file says a problem names the line "if any".
* [x] 2026-10-20: M7 review round 3, by one new reviewer, who ran 24 inputs and the three planted breaches.  No blockers and no should-fix items; 3 nits, all checked.  The own-module exception holds only for `module::name` with nothing before it, so `self::max::max` still fails; the doc comment now says so.  The design and Decision log now record the scan refinements.  Not changed: `Vec<Vec<f32>>=` reads as `>>=`, but rustfmt never writes it.
* [x] 2026-10-20: M7, three review rounds done and their findings fixed.
* [x] 2026-10-20: M7: PR #20 opened.  CI (run 39392378348): every job passes, and the `test` job lists `dsp_is_beginner_style` and the three unit tests as `ok`.  `test` took 4m20s.
* [x] 2026-10-20: M7, PR #20 merged by the maintainer (`a57272e`).
* [x] 2026-10-20: M8 implementation plan written (the M8 sections under "Plan of work").  Survey of the tracked files, their sizes, the ignored folders on the Mac, and `git ls-files --format` on git 2.42 and later.
* [x] 2026-10-20: M8 plan review round 1, by one new reviewer with a prototype in a scratch worktree.  No blockers; 2 should-fix findings and 5 nits, all checked and real, all fixed in the plan.  The anchored `.gitignore`, the word scan over today's build files and the `ls-files` parse held in the prototype.  The should-fix items: a suggested Python web server in the docs passed unseen, and a literal match finds it, so it is now a rule.  Six wrong implementations passed the unit tests (a suffix or case-free exception, a case-sensitive word match, an extension without its dot, and lists that drop a named entry); new cases fail each.
* [x] 2026-10-20: M8 plan review round 2, by one new reviewer with a prototype in a scratch worktree.  No blockers; 1 should-fix and 8 nits, all checked.  The server rule gave no false result over the 145 tracked text files.  The should-fix: the planned `* Enforced by:` lines left out three gaps that the design names (a shebang script, a server in another language, the history); they now name them.  The nits were all fixed.  New cases fail a scan of Markdown only, a list without `uv` or `pipx`, and an extension read from the first `.`.  `rules.rs` builds the server names with `concat!` and needs no exception.  `.cargo/` means any such folder.  The `check-ignore` acceptance uses `.txt` paths, which only the folder rules match.  Not changed: Rust code that starts Python stays out of scope, as the reviewer accepted.
* [x] 2026-10-20: plan review stopped after round 2, by the maintainer's rule: no blockers, and no should-fix item beyond a small text fix.  Not yet implemented.
* [x] 2026-10-20: M8 step 1: `TrackedFile`, `parse_ls_files`, `tracked_files`, `tracked_texts`, `is_build_file` and the two checks, test first.  Against the stubs, the 7 tests that expect a problem or a parse failed, and `clean_build_files_pass` passed.  With the functions, all 8 pass.
* [x] 2026-10-20: M8 step 2: `no_python_in_the_build` and `no_audio_or_weights_are_tracked` pass on the real tree (151 tracked files), the anchored `.gitignore`, and the docs.  "Prose only" lines: 5.  `Cargo.lock` did not change.
  * Planted breaches, each run and restored, each failed its test with one problem: `git add -f song.flac` (the extension `.flac`), `git add tools/x.py` (a Python file), `pip install numpy` in `ci.yml` (line 212, the word `pip`), the web server in `README.md` (line 153), and `git add -f` of a 2,000,000-byte `docs/big.txt` (over 1,048,576).
  * `git check-ignore -v engine/src/audio/x.rs` prints nothing and exits 1.  `app/dist/index.html`, `stems/a.txt` and `audio/synth/a.txt` match `/app/dist/`, `/stems/` and `/audio/`.  `git status --ignored` lists `app/dist/`, `target/` and `third_party/`, as before.
* [x] 2026-10-20: M8 review round 1, by one new reviewer, who ran inputs and mutants through the checks.  No blockers; 1 should-fix and 6 nits, all checked and real, all fixed.  The should-fix: 14 of the 18 asset extensions had no case, so dropping `mp3` or `wav` passed; `assets_are_named` now lists each one.  Nits: the server names match with case, so a Rust `SimpleHttpServer` passes.  A tracked link to a folder is skipped, not an error, and `tracked_texts_skips_what_has_no_text` tests the skips.  The fix messages quote the path.  New cases fail a mutant for `build.rs` in one crate only, `config` in the root `.cargo/` only, and history matched anywhere in a path.  BUILD-1 names a shell script that runs Python as a gap, and says that `build.rs` is read.
* [x] 2026-10-20: M8 review round 2, by one new reviewer, who ran about 20 inputs and 3 mutants.  No blockers and no should-fix items; 6 nits, all checked.  Two fixed: `_` is now part of a word, so `uv_scale` in a `build.rs` passes, and the asset fix message is three sentences.  Four need no change: mutants that only add false results on odd paths, a file added with `git add -N` (size 0 until staged), the temp-folder test on Windows (CI runs Linux), and `gen-song` run from a crate folder (its audio is ignored by extension).
* [x] 2026-10-20: M8 review round 3, by one new reviewer.  No blockers and no should-fix items; 5 nits, all checked and real, all fixed.  The M8 design now says what the code does after rounds 1 and 2.  The parent plan tags M8.  The cap is "about twice" the largest file.  A raised cap needs "a reason in the PR" in both the guide and the message.  README names the test.  The reviewer found no way for either real-tree test to pass without checking anything.
* [x] 2026-10-20: M8, three review rounds done and their findings fixed.
* [x] 2026-10-20: M8: PR #21 opened.  CI (run 39406037091): every job passes.  The `test` job lists `no_python_in_the_build`, `no_audio_or_weights_are_tracked` and the 9 unit tests as `ok`, so `git ls-files --format` works on the runner.  `test` took 3m14s.
* [x] 2026-10-20: M8, PR #21 merged by the maintainer (`2af33b1`).
* [x] 2026-10-20: M9 implementation plan written (the M9 sections under "Plan of work").  Survey of every registry requirement in `cargo metadata`, the two `THIRD-PARTY.md` commands against the tables, and the crates.io age of the young crates.
* [x] 2026-10-20: M9 plan review round 1, by one new reviewer with a prototype in a scratch worktree.  Every fact in "M9 state at the start" held.  With the 5 pins, `Cargo.lock` and the first tree's rows do not change, and the 13 crates left are the list.  1 blocker, 3 should-fix findings and 4 nits, all checked and real, all fixed in the plan.  The blocker: `dsp` and `test-support` have no dependency, so "a member with no registry dependency" failed the real tree forever.  The should-fix items: the rule for a requirement below 1.0 was not defined, and `<1` passed a plausible mutant; any text in parentheses dropped a git crate from the list in silence; nothing said what the check does with a `SETTLED_CRATES` date.
* [x] 2026-10-20: M9 plan review round 2, by one new reviewer with a prototype of the rule and both parsers.  Every fact held, and the command built from the args matched `THIRD-PARTY.md` byte for byte.  No blockers; 4 should-fix findings and 5 nits, all checked and real, all fixed in the plan.  The should-fix items: the check read the real `SETTLED_CRATES`, so no unit test could pass a small list; `>0.1` and `^1.0.0-rc.1` passed the rule or a plausible mutant; rows keyed by name only, a text prefix for the root, and a git dependency each passed every test.  A git dependency is now a problem.
* [x] 2026-10-20: M9 plan review round 3, by one new reviewer with a prototype of the rule.  All 22 examples of the rule gave the stated answer.  No blockers; 2 should-fix findings and 5 nits, all checked and real, all fixed in the plan.  The should-fix items: `resolve` names a dependency by its import name (`cute_dsp`), so a lookup of the locked version by name failed; the fix message now points at `Cargo.lock` and does no lookup.  Some "one problem each" counts could not hold for the empty-tree cases; the "read nothing" guard is now one problem.  The nits: 13 args, not 12; `M.m.*`; a source that is neither registry nor git now fails; TD-7's new title.
* [x] 2026-10-20: M9 plan review round 4, by one new reviewer who read the section as its implementer.  No blockers; 2 should-fix findings and 7 nits, all checked and real, all fixed in the plan.  Both should-fix items were one-bullet fixes.  A `[patch]` that points a registry crate at git passed both checks, so every package's source is now read.  A `cargo tree` line that the parser cannot read had no stated result; it is now a "cannot read" problem.  The nits: a step 0 records the maintainer's answer; the doc parser skips the header and separator; an empty licence fails in either table; M11 adds rows for new crates; phase 5a's gardening job is retagged.
* [x] 2026-10-20: plan review stopped after round 4, by the maintainer's rule: no blockers, and no should-fix item beyond small local fixes.  Not yet implemented.  Next: the maintainer answers M9's decision point.
* [x] 2026-10-20: M9 step 0: the maintainer chose option (b) at the decision point.
* [x] 2026-10-20: M9 step 1: the two checks and their parsers, test first.  Against stubs of the checks, the 5 tests that expect a problem failed, and the 4 others passed.  With the checks, all 9 pass.  The tests sit in their own module, `deps_tests`, which takes the metadata fixtures of M5's tests.
* [x] 2026-10-20: M9 step 2: `young_crates_are_pinned` and `third_party_lists_what_ships` on the real tree, the 5 pins and the docs.  Before the pins, `young_crates_are_pinned` failed and named exactly `egui_kittest`, `cute-dsp`, `timestretch`, `wsola` and `burn-store`; `third_party_lists_what_ships` passed.  After the pins, both pass, and `Cargo.lock` did not change.  "Prose only" lines: 4.
  * Planted breaches, each run and restored, each failed its test with one DEPS-1 problem: `wsola` without `=` (`engine/Cargo.toml takes wsola ^0.1.0`), `log` removed from `SETTLED_CRATES` (`app/Cargo.toml takes log ^0.4.34`), a `zzz` entry (`lists zzz in SETTLED_CRATES`), the `log` row removed (`lacks log 0.4.34`), and `MIT` in the `indexmap` row (`THIRD-PARTY.md:65 gives indexmap 2.14.2 the licence "MIT"`).
* [x] 2026-10-20: M9 review round 1, by one new reviewer who ran 9 planted breaches and 32 mutants.  No blockers; 1 should-fix and 7 nits, all checked and real, all fixed.  The should-fix: no test covered the second table, so dropping its comparison, or a wrong line offset, passed.  New cases kill those and 9 more mutants: `~1` and `=1`, a pre-release in a second comparator, a dev-only crate without a licence, the second command above the heading, a date of 11 characters, a git-only member, and the guard on `wgpu` and `eframe`.  DEPS-1 says "not an `=` pin", and TD-7 says which check sees another registry.
* [x] 2026-10-20: M9 review round 2, by one new reviewer, who ran the checks under CI's settings, through a symlink, and on plausible agent breaches in a scratch worktree.  No blockers; 1 should-fix and 3 nits, all checked and real, all fixed.  The should-fix: a crate vendored under the root, with a `[patch]`, counted as a workspace crate, so the check said "remove the row" for a crate that still ships.  The check now drops a path crate only when its folder is a member's (`member_folders`, from `cargo metadata`); any other path cannot be read.  Planted on the real tree (`log` in `vendor/log`), it then failed with one "cannot read" problem, deduplicated across repeats and both commands.  This entry first said that was all; the output also held "remove the row" for `log`, which round 3 found and fixed.  Nits: the fix for a young crate names `[workspace.dependencies]`; the "cannot read" fix asks the maintainer for git, another registry or a vendored folder; TD-7 says "in a tree of `THIRD-PARTY.md`" and names the vendored gap.
* [x] 2026-10-20: M9 review round 3, by one new reviewer, who read the whole diff and tried mutants of the newest code.  No blockers; 2 should-fix and 2 nits, all checked.  (1) Real: the vendored `log` still gave "remove the row" beside "cannot read", because its row was compared as usual; a unit test passed only because its doc had no `log` row.  A row whose crate and version are on a line that cannot be read is now left out of the comparison, and the replant gives exactly one problem.  (2) Real: no test had an unread line in the second command only, so a mutant that reads only the first command's unread lines passed.  New cases kill it, its mirror, and dropping the new filter.  (3) Real: a vendored folder inside a member's folder passed as the member; `/root/app/vendor/log` is now a "cannot read" case.  (4) Not changed: a member's git dependency gives two problems in the real shape (the dependency, and its package), both true.
* [x] 2026-10-20: M9, three review rounds done and their findings fixed.
* [x] 2026-10-20: M9: PR #22 opened.  CI (run 39421297846): every job passes.  The `test` job lists `young_crates_are_pinned`, `third_party_lists_what_ships` and the 10 unit tests in `deps_tests` as `ok`, so `cargo tree --locked` works on the runner.  `test` took 3m13s.
* [x] 2026-10-20: M9, PR #22 merged by the maintainer (`7a99a88`).
* [x] 2026-10-20: M10 implementation plan written (the M10 sections under "Plan of work").  Survey of `stem_protocol::kind`, of each message type in `xtask/e2e/*.js` and `xtask/src/e2e.rs`, and of the e2e scenarios that CI runs.
* [x] 2026-10-20: M10 plan review round 1, by one new reviewer with a prototype of the check in a scratch worktree.  Every fact held.  The real tree gave no problem, from 32 types, and each of the 5 plants gave one problem on the right line.  No blockers; 3 should-fix findings and 7 nits, all checked and real, all fixed in the plan.  The should-fix items were shapes that hid a misspelt type: a quoted key `"type": "x"` in JS and in `json!`, a bare `type === "x"`, and `\"` in a plain Rust literal.  The nits: a test order that let a mutant pass, a vacuity rule for each source, an escape in template literals, the fix text for a non-message option, a reference to text the implementer cannot see, the type of the list after plant 5, and the undo line.
* [x] 2026-10-20: M10 plan review round 2, by one new reviewer with a prototype of the check.  The real tree and the 5 plants gave the stated results again, and `docs-check` passed with the planned doc edits.  No blockers; 1 should-fix and 7 nits, all checked and real, all fixed in the plan.  The should-fix: the real tree uses only `===` in JS, only `==` in Rust and no raw script with a type, so a check that dropped one of these passed every test; `a_misspelt_type_is_named` now uses each.  The nits: two over-reporting mutants, the fixture order of `the_kinds_are_read`, the stop after a protocol problem, a signature for `protocol_kinds`, lines after multi-line comments and templates, the gaps of the scan, and the parent plan's tag.
* [x] 2026-10-20: M10 plan review round 3, by one new reviewer with a prototype of the check and the planned doc edits.  The real tree, the 5 plants and `docs-check` gave the stated results, and the planned tests caught each mutant tried.  No blockers and no should-fix; 5 nits, all checked and real, all fixed in the plan: the acceptance text on the stub, an empty `script_only` in the counting tests, how to tell a raw literal, two fix texts, and two more gaps.
* [x] 2026-10-20: plan review stopped after round 3, by the maintainer's rule: no blockers and no should-fix.  Not yet implemented.
* [x] 2026-10-20: M10 step 1: the check and its 7 unit tests, test first.  Against a stub of the check, the 4 tests that expect a problem failed, and the 3 others passed.  With the check, all 7 pass.
* [x] 2026-10-20: M10 step 2: `e2e_messages_use_protocol_types` on the real tree, and the docs.  It passes, from 32 types.  Each of the 5 plants failed it with one PROTO-1 problem: `worker.js:26` ("alve"), `e2e.rs:704` ("forget"), `e2e.rs:1194` ("dne"), `worker.js:43` ("log", after `LOG` became "logs") and `worker.js:52` ("worker-error", after its entry was removed).  "Prose only" lines: 4.
* [x] 2026-10-20: M10 review round 1, by one new reviewer who probed the check and tried mutants.  The real tree gave the same 32 types on the right lines.  No blockers; 1 should-fix and 4 nits, all checked and real, all fixed.  The should-fix: TD-16 did not name every gap that the plan lists, such as a `\n` escape after a `//` comment in a plain Rust literal.  The nits: the plan's `d?.type` gap (only the reversed form is missed), a JS line continuation in a string, and four mutants that the tests let pass.  The fix for the line continuation also gave a string the line it ends on; a test now pins its start line.  Each of the 5 mutants now fails a unit test.
* [x] 2026-10-20: M10 review round 2, by one new reviewer who ran 42 mutants.  No blockers; 1 should-fix and 4 nits, all checked and real, all fixed.  The should-fix: a `js_after_typeof` that skipped every type after any earlier `typeof` passed the tests, so `typeof d === 'object' && d.type === 'segmnt'` is now a case.  The nits: a long doc comment line; Rust fixtures for the bracket and brace rules (`c["state"] == "running"`, `f("type") == "x"`, `m!["type": "x"]`); and TD-16 now names a regex literal and the false type in `typeof d?.type === "string"`.  The three new cases each kill their mutant.
* [x] 2026-10-20: M10 review round 3, by one new reviewer who read the whole diff and ran 25 probes.  No blockers and no should-fix; 3 nits, all checked.  Two were real and are fixed.  TD-16's lists of shapes not read now say "for example", since probes found more, such as `(d.type) === 'x'`.  An `e2e.rs` that does not tokenise also gave "no e2e script uses it" for an entry used only there; that reason now needs a read `e2e.rs`, and a test pins it.  The third, a raw literal with a lone `"`, is correct lexing of invalid JS.
* [x] 2026-10-20: M10, three review rounds done and their findings fixed.
* [x] 2026-10-20: M10: PR #23 opened.  CI (run 39441862201): every job passes.  The `test` job lists `e2e_messages_use_protocol_types` and the 7 unit tests in `proto_tests` as `ok`.  `test` took 4m12s, and `browser tests (Chrome)` passed with no script changed.
* [x] 2026-10-21: M10, PR #23 merged by the maintainer (`7cd8518`).
* [x] 2026-10-21: M11 implementation plan written (the M11 sections under "Plan of work").  Survey of the callback's path in `engine`, of wsola 0.1.0 and 0.2.0 (a diff of both published sources), and of where the app makes players and loops.
* [x] 2026-10-21: M11 plan review round 1, by one new reviewer with a prototype of the design in a scratch worktree.  The demo song's hash was equal on wsola 0.1.0, on 0.2.0 and after warm-up.  After the fixes, all 12 configurations gave 0 allocations and 0 frees; each planted breach failed.  1 blocker, 3 should-fix findings and 6 nits, all checked and real, all fixed in the plan.  The blocker: the playhead checks failed on correct code, so they are now three checks that hold.  The should-fix items: the expected failures were wrong ("tempo sweep", not "fast"); the manifest test passed without `default-features = false`; and the claim on a dropped anchor was false.  The nits: the wsola bound's sum, a thread per configuration, a tempo before `warm_up`, a stale comment, TD-<n>'s text and why `Varispeed` is covered.
* [x] 2026-10-21: M11 plan review round 2, by one new reviewer with a prototype of the whole design and mutants.  The fixed code gave 0 allocations in all 12 configurations, in 3.5 s with a thread per configuration, and the anti-vacuity checks held on correct code.  No blockers; 5 should-fix findings and 8 nits, all checked and real, all fixed in the plan.  The should-fix items: a counter that missed `realloc` passed every test; a chipmunk mutant that mixed up the channels passed; the manifest check missed a feature in `[features]` and had no unit test; the anchor unit test could fail on correct code or test nothing; and a revert marked decision 0007 rejected.  The nits: the warm-up push bypasses `interleaved`, the phases' parameters, frees alone fail, a message for a silent phase, a sine in the warm-up test, a second stale comment, the push size in `warm_up`'s doc, and 2,020 Hz.
* [x] 2026-10-21: M11 plan review round 3, by one new reviewer with a full prototype.  `the_counter_sees_an_allocation` counted 3 allocations and 2 frees in dev and release; the fixed code passed and each planted breach failed.  No blockers; 3 should-fix findings and 3 nits, all checked and real, all fixed in the plan.  The should-fix items, each a local fix: how seconds become callbacks (the named phase of a plant depended on it) and a pause with no callback; the new `dsp.md` sentence was false, since `read_mixed` calls `dsp::wrap_loop`; and the manifest check missed a table-form `engine`, a dotted key and `engine?/default`.  The nits: the exact `memory.md` text, the acceptance list, and a duplicate test case.
* [x] 2026-10-21: plan review stopped after round 3, by the maintainer's rule: no blockers, and no should-fix item beyond small local fixes.  Not yet implemented.
* [x] 2026-10-21: M11 step 1: wsola 0.2.0.  The demo song at tempos 0.75 and 1.25 gave the hash `cc06c36c82de4b04` on 0.1.0 and on 0.2.0.  `Cargo.lock` changed only wsola's version and checksum.
* [x] 2026-10-21: M11 step 2: the test, the fixes and the docs.  Before the fixes, `the_callback_does_not_allocate_after_warm_up` failed in 32 rows, as the plan expected: wsola "start" (10 allocations) and "tempo sweep" (2) at every rate and size; "chipmunk" for both stretchers (288 to 900 allocations, 36 to 150 frees); "short loop" for wsola at every size (2 to 4) and for varispeed at 4,096 frames (3).  After the fixes, all 12 configurations give 0 allocations and 0 frees, in 3.4 s.  The hash after warm-up is still `cc06c36c82de4b04`.
  * Planted breaches, each run and restored: no `warm_up` call ("start" and "tempo sweep", wsola, 12 rows); a warm-up push of `(1 << 16) + rate / 50` frames ("tempo sweep", wsola, 1 allocation, 6 rows); `dsp::double_speed` back in chipmunk mode ("chipmunk", 12 rows); no anchor bound ("short loop", 8 rows); `format!` in `render_interleaved` (every phase).  `stretch-cute` in `app/Cargo.toml`, and the loss of `default-features = false`, each fail `the_app_ships_only_these_stretchers` with one RT-1 problem.
  * Unit-test mutants, each caught: a mix-up of the chipmunk channels (`chipmunk_matches_dsp_double_speed`), no anchor bound (`a_very_short_loop_keeps_its_anchors`, capacity 16,384), and a `warm_up` without `reset` or without the tempo restore (`a_warmed_stretcher_sounds_the_same`).  "Prose only" lines: 4.
* [x] 2026-10-21: M11 review round 1, by one new reviewer who read wsola's sources and ran mutants and other sample rates.  No blockers; 1 should-fix and 4 nits, all checked.  The should-fix, real: `Stretcher::warm_up` promised that a push of 4,096 frames does not allocate, but wsola's warm-up covered only the player's 256.  The new test `a_stretcher_keeps_its_warm_up_promise` failed at 8 and 16 kHz with 1 allocation; the warm-up now pushes 4,096 frames more (about 650 KB in all, not 620 KB), and it passes.  Nits: RT-1's title now says "after warm-up"; the allocator's comment on `try_with` was wrong; wsola's doc says that the push covers any tempo up to 4, so `max_tempo` only sets the warm-up pull's tempo.  Not changed: 22,050 Hz in `RATES`, since the new test covers low rates.
* [x] 2026-10-21: M11 review round 2, by one new reviewer who ran 94 more configurations (8 to 192 kHz, other `PlayerConfig`s, 1 to 6 stems, seeks in fades, loops in chipmunk mode): 0 allocations in all.  No blockers; 1 should-fix and 4 nits, all checked and real, all fixed.  The should-fix: two callback paths had no phase, a pause while a seek's fade runs and a callback over `SCRATCH_FRAMES`, so an allocation planted in either passed.  "seek" now pauses one callback after the seek, and `CALLBACK_FRAMES` adds 10,000; each plant now fails ("seek" at 128 frames; every phase at 10,000).  Nits: the promise test adds 96 and 192 kHz; `Player::with_config` and `Varispeed::warm_up` panic on a hop over `MAX_PUSH_FRAMES` or a block the queues cannot hold, each with a test; `render_chipmunk`'s doc; and "chipmunk" checks that the playhead moves 1.8 song frames or more per frame played, which a player that ignores chipmunk fails.
* [x] 2026-10-21: M11 review round 3, by one new reviewer who read the whole diff and ran 11 mutants.  No blockers; 1 should-fix and 4 nits, all checked.  The should-fix, real: `interleaved`'s capacity and the promise test's push were 4,096 by hand, so a raised `MAX_PUSH_FRAMES` broke RT-1 and failed only a test with a hard-coded number.  Both now use the constant; with the constant at 8,192, the promise test still passes.  Nits fixed: the asserts' edges are pinned (`<` in place of `<=` now fails a test in each); the promise test also pulls 4,096 frames at 44.1 kHz; two doc comments rewrapped; this plan's design text now says `+ MAX_PUSH_FRAMES`, about 650 KB and four callback sizes.  Not changed, with reasons: a `pop_back` mutant of the anchor bound passes, but it keeps the newest anchor, so the playhead after the loop stays right; a smaller warm-up margin (`rate / 8`) passes, since noise does not reach the bound's worst case, which its arithmetic covers; a `warm_up(.., 256)` in `Player::with_config` passes, since the promise test checks the stretchers directly; `required-features` on the test would turn a compile error into a silent skip (VERIFY-1).
* [x] 2026-10-21: M11, three review rounds done and their findings fixed.
* [x] 2026-10-21: M11: PR #25 opened.  CI (run 39521993920): every job passes.  The `test` job lists the 5 `realtime` tests as `ok`; the binary took about 11 s on the runner.  `test` took 15 min, since Rust 1.99.1 changed rust-cache's key and the build started cold (`Finished` in 9m 58s); M11 did not cause that.
* [x] 2026-10-21: M11, PR #25 merged by the maintainer (`cb1d0be`).
* [x] 2026-10-21: M12 implementation plan written (the M12 sections under "Plan of work").  Survey of every `to_vec` in `app`, `worker`, `protocol` and `engine`, of CI's clippy targets, and of clippy's `disallowed-methods` paths in a scratch worktree.
* [x] 2026-10-21: M12 plan review round 1, by one new reviewer with a prototype of the whole design in a scratch worktree.  Every clippy command passed on both targets; inner `#[expect]`s under a module `#![expect]` proved their paths.  1 blocker, 5 should-fix findings and 9 nits, all checked.  The blocker, real: `docs-check` takes `` `to_vec` `` on an `* Enforced by:` line as a test name; MEM-1's line now gives each path.  The should-fix items, real and fixed: with `-D warnings`, engine's errors hid `app`'s sites, so the count runs without it; deleting `app/clippy.toml` never reached the app's canary, so a wrong `Float32Array` path is planted instead; the TD-13 plant also gave an unused variable; three sites are a match arm, a tail expression and a `return`; TD-13 and TD-14 named line numbers that move.  Nits fixed: the counts, the warning for a wrong method in `js_sys`, `engine`'s `js_sys`, TOML literal strings, MEM-2's line, the step 1 `harness.md` row.  Not changed: `memory.md`'s paths for `app/tests/` (a canary is not a site), rustfmt's wrapping, and the engine and worker canaries, which stay for when M18 removes TD-13 and TD-14.
* [x] 2026-10-21: M12 plan review round 2, by one new reviewer with a prototype of the whole design and every doc edit.  The site count, the plants, `docs-check` and `cargo test -p xtask` held.  No blockers; 6 should-fix findings and 12 nits, all checked and real, all fixed in the plan.  The should-fix items: rust-cache saves only on a key that does not match exactly, so the `wasm32` step needs a new cache key (read in rust-cache's `restore.ts` at v2.9.2); the plants tried 3 of the 10 (crate, path) pairs, so step 2 now tries all 10; a plant's stated result was incomplete; a canary proves its path only under a clippy step with `--all-targets`, now in TD-<n> and MEM-1's line; `README.md`'s setup table must name the `wasm32` target; "a JS typed array" claimed more than two types.  The nits: TD-<n>'s list of forms, MEM-2's gaps, the JS reason's lint name, the timing, `web-test`'s targets, `roformer`'s count, "in CI", the plant's place in `from_audio`, the parent bullet's punctuation and "the one copy into wasm memory".  Not changed: `memory.md`'s paths.
* [x] 2026-10-21: M12 plan review round 3, by one new reviewer who applied the design and each doc edit as worded.  Every clippy command, `docs-check` and `cargo test -p xtask` passed, and the 3 plants and 10 pairs failed as stated.  No blockers; 3 should-fix findings and 7 nits, all checked and real, all fixed in the plan.  The should-fix items, each a local fix: a module `#![expect]` before `#![cfg(target_arch = "wasm32")]` in `protocol/tests/web.rs` failed the native step; "each run fails" was false for the native step of three crates; the decision point gave no changes for (b) and (c), and (b)'s size was wrong.  The reviewer's root `clippy.toml` is now option (d).  The nits: `demix.rs` is in `roformer`, plant 1's place, the exact command lines, two more gaps in TD-<n>, both `harness.md` rows, `app/clippy.toml` in `memory.md`'s paths and the cache size.
* [x] 2026-10-21: M12 plan review round 4, by one new reviewer who did both steps as worded in a scratch worktree.  fmt, the 5 clippy commands, the tests and `docs-check` passed; the 3 plants and the 10 pairs failed as stated.  No blockers; 2 should-fix findings and 9 nits, all checked and real, all fixed in the plan.  The should-fix items, each one line: TD-<n> did not name `synth`, which makes the demo song in the web app; the `README.md` setup row said "Nothing more on macOS".  The nits: which `README.md` list, plant 2's place, MEM-1's capital and CI job, TD-<n>'s lead sentence, "in silence", `js_sys` in `stem-protocol`'s native build, (b)'s counts, (c)'s list, the CI step's place.  Plan review stops here: no blockers or significant should-fix.  Not verified: the canaries in a browser, since no WebDriver ran here.
* [x] 2026-10-21: M12, option (b) chosen by the maintainer.
* [x] 2026-10-21: M12 step 1: the `wasm32` clippy step, with the cache key.  It passed on the tree as it was.
* [x] 2026-10-21: M12 step 2: the lint, its sites, the canaries and the docs.  Before the `#[expect]`s, without `-D warnings`: the `wasm32` run warned at the 15 calls on the 13 table lines, the 5 sites of `roformer` and `roformer-gpu`, 9 calls in `worker/src/web/tests.rs` and 7 in `protocol/tests/web.rs`.  The native run warned at the 2 `engine` sites, the 5 model-crate sites and the 17 model-crate test calls.  Each warning had the MEM-1 note.  After them, every clippy command passed.
  * Plants, each run and restored: a copy in `Song::from_audio` failed the native step (`song.rs:114`, `slice::to_vec`); a copy in `run` failed the `wasm32` step (`web.rs:1051`, `js_sys::Float32Array::to_vec`); TD-13's site with no copy failed with only "this lint expectation is unfulfilled" (`separator.rs:290`).
  * The 12 (crate, path) pairs, each turned off and restored, each failed with "this lint expectation is unfulfilled": app slice and Float32Array at the canary only; app Uint8Array at `picker.rs`, `store.rs` and `separator.rs`; worker each path at its sites and the canary; protocol each path at its canary only; engine, roformer and roformer-gpu at their sites and canary on the native step, and at their lib sites on the `wasm32` step.
  * `xtask web-test --browser chrome` (Chrome 155): `mem1_lint_is_on_for_js_arrays` passed in `app`, `stem-worker` and `stem-protocol`, and every other browser test passed.  Safari not run: safaridriver needs "Allow Remote Automation", a person step.
* [x] 2026-10-21: M12 review round 1, by one new reviewer who planted a copy in each of the six crates and turned off paths.  Each failed clippy as claimed.  No blockers; 2 should-fix findings and 4 nits, all checked and real, all fixed.  The should-fix items: the slice message told `stem-worker`, `stem-protocol`, `roformer` and `roformer-gpu` to share an `engine::Samples`, which LAYER-1 forbids them; they now say "move the buffer instead of copying it".  Two bench reasons said "kernel choices" for matmul shapes.  The nits: the parent plan's bullet, `roformer/clippy.toml` in `gpu.md`'s paths, a 27-word sentence in MEM-1, and `store.rs`'s reason, since `read` also loads `meta`.
* [x] 2026-10-21: M12 review round 2, by one new reviewer who read both pinned actions' sources, built the `wasm32` step from cold (1m 34s) and ran the canaries in Chrome.  No blockers; 1 should-fix and 3 nits, all checked.  The should-fix, real: the guide's changelog line still told every crate to share an `engine::Samples`.  Nits fixed: TD-26 names an outer `#[expect]` on `mod tests` too, and the M12 design names TD-26 and the 12 pairs.  Not changed: the file-wide `#![expect]`s in two `roformer-gpu` test files, which the design allows for tests.
* [x] 2026-10-21: M12 review round 3, by one new reviewer who read the whole diff as a maintainer and ran every CI command.  No blockers; 1 should-fix and 2 nits, all checked.  The should-fix, real: the choice at M12's decision point outlives the plan, as M6's and M11's did, so it is now decision 0008, accepted.  Nit fixed: the design gives (b)'s counts beside (a)'s.  Not changed: `app/tests/web.rs` stays outside `memory.md`'s paths, as plan review decided.
* [x] 2026-10-21: M12, three review rounds done and their findings fixed.
* [ ] M12: MEM-1 lint.
* [ ] M13: MEM-3 lint.
* [ ] M14: Claude Code deny rules.
* [ ] M15: rustfmt hook.
* [x] 2026-10-20: M16 moved to phase 5b by the maintainer.
* [ ] M17: UI snapshots compared.
* [ ] M18: TD-13, TD-14, TD-12, TD-17 fixed.
* [x] 2026-10-19: browser tests are a required check (the maintainer, before PR #14; found by `gh api`).
* [ ] M19: phase 2 closed.

## Surprises and discoveries

* 2026-10-19: wsola 0.2.0 already removes the allocation in `pull`, so TD-15 needs no fork.  Evidence: `src/lib.rs` of the published 0.2.0 crate.
* 2026-10-19: MEM-3's `compile_fail` doctest of `Rec` has never run in CI, since `--exclude roformer-gpu` drops doctests as well.
* 2026-10-19: the shipped licence list (`THIRD-PARTY.md`) includes `hound`, `serde`, `serde_derive` and `serde_json`, pulled in only by `synth`'s file writers.  Corrected in the M4 survey: only `hound` comes only through `synth`.  The serde crates also come through `roformer` and `safetensors` in `stem-worker`.
* 2026-10-19: the chipmunk path also allocates in the callback (`dsp::double_speed`), not only the stretcher.
* 2026-10-19, M1 review: `-p roformer-gpu` alone builds wgpu with fewer features than the workspace build, so the new CI steps rebuild wgpu, wgpu-core, wgpu-hal and naga.  On PR #14 it added 68 s to the `test` job.
* 2026-10-19: the docs said `browser tests (Chrome)` was not a required check, and the agent repeated it to the maintainer.  The ruleset had required it since 03:36 UTC that day.  Ruleset facts in docs go stale; check them with `gh api` before stating them.
* 2026-10-19, M1 review: `sources_are_complete` listed 12 placeholder names by hand and missed 8 that the templates use (`{BM}`, `{BN}`, `{BK}`, `{TY}`, `{SB_LEN}`, `{S_ZERO}`, `{SOFTMAX_1}`, `{SOFTMAX_2}`).  A test that lists what to look for goes stale; it now scans for the pattern.
* 2026-10-19, M2: deleting TD-1 made `docs-check` fail on each `TD-1` in the plans.  It checks every ID token in every Markdown file, plans included.  Every later milestone closes an ID, so it needed a rule for closed IDs.
* 2026-10-19, M2: WebGPU counts each `var<workgroup>` rounded up to 16 bytes, and only the ones the entry point statically uses ([compute pipeline creation](https://gpuweb.github.io/gpuweb/#dom-gpudevice-createcomputepipeline)).  The text count in `workgroup_memory_within_defaults` does neither; the tiles here are multiples of 16, so the counts agree.
* 2026-10-19, M3 CI: a change to `Cargo.lock` makes rust-cache restore an older cache by its restore key.  The `test` job then took 10m58s, not about 5 minutes; a re-run took 4m41s.  So a first CI time after a lock change is not the cost of the change.
* 2026-10-19, M3 survey: two `xtask` tests have never run anywhere.  `folder_loading_matches_the_generator` and `mp3_decodes_to_roughly_the_right_length` need `audio/synth/`, which neither CI nor the maintainer's Mac has.  ffmpeg is on neither.
* 2026-10-19, M4 survey: moving `synth`'s writers out of the web app saves 242 bytes of `app_bg.wasm`, because the linker already removed them.  Only `hound` leaves `THIRD-PARTY.md`; `serde_json` stays through `stem-worker`.
* 2026-10-19, M4 review: CI's `CARGO_TERM_COLOR: always` reaches a test's child process, so `cargo tree` writes colour codes into a pipe (107 lines at `69f8cae`).  A check that reads Cargo's output passes `--color never`.
* 2026-10-19, M4 review: `cargo tree -i <crate>` prints "nothing to print" and exits 0 when the crate is not in the graph.  A check built on it passes in silence.
* 2026-10-19, M4 review: `--workspace` builds merge features across every crate, so they hide a lost dev-dependency.  Without `app`'s dev-dependency on `synth` with `files`, `cargo test --workspace --exclude roformer-gpu --no-run` still builds, and `cargo test -p app` fails with E0425.
* 2026-10-19, M4 review: `cargo tree` with two `-p` roots merges their features, and prints a shared crate in full only under the first root.
* 2026-10-19, M1 review: stable rustdoc does not check a `compile_fail` doctest's error code, so MEM-3's doctest passes on any compile error (TD-22).  Evidence: a throwaway crate on rustc 1.99.0.
* 2026-10-20, M6 survey: two of the four lints that `dsp` relaxes are in clippy's complexity group, not style (`manual_clamp`, `manual_swap`).  DSP-1 and TD-2 said "style".  Also, `let x = x;` fires `redundant_locals`, which is in the suspicious group, not correctness.  Evidence: clippy 1.99.0 with each group allowed in turn.
* 2026-10-20, M6 survey: an `expect` does not guard against an `allow` before it.  `#![allow(clippy::all)]` before the four `expect`s passes clippy, and after them it fails with four unfulfilled expectations.
* 2026-10-20, M6 survey: `dsp` uses types outside DSP-1's list: `f32` in `biggest` and `median`, `Vec<(f32, f32)>` in `peaks`, `&[&[f32]]` in `mix` and `bool` in `median`.  So DSP-1's "A scan on 2026-10-19 found no breach" was false for the types.  M6 records them as a breach, and M7 decides them.
* 2026-10-20, M6 review round 1: the `assign_op_pattern` `expect` also silences `x = x * y`, `/`, `%`, `<<`, `|` and `^`, not only `+` and `-`.  Evidence: with the `expect` removed, each fires the lint.
* 2026-10-20, M7 survey: `dsp` never writes `bool`.  `median`'s `swapped` gets it from `true`, so a check of written types cannot see it, and TD-24's "has a `bool`" is an inferred type.
* 2026-10-20, M6 plan review: an `expect` of a lint group silences the whole group.  `#![expect(clippy::all, reason = "x")]` is fulfilled by a style lint, and `n == n` then passes.  So M7 must name each lint, not only require `expect`.
* 2026-10-20, M8 survey: the first M8 text said to anchor `dist/` as `/dist/`.  The only `dist` folder is `app/dist/`, Trunk's output, so `/dist/` would stop ignoring it.  The anchored form is `/app/dist/`.
* 2026-10-21, M12 survey: CI's clippy has never linted the `wasm32`-only code: `worker/src/web.rs` and most of the page side in `app/src/`.  TD-13 and TD-14 are in that code.  A `wasm32` clippy run passes today.
* 2026-10-21, M12 survey: a `disallowed-methods` path that does not resolve passes `-D warnings`, in silence or with a warning, though clippy's docs say it is an error.  Evidence: `slice::to_vecc` and `js_sy::Uint8Array::to_vec` give no message on clippy 0.1.99.
* 2026-10-21, M12: the `wasm32` step also lints the lib code of `roformer` and `roformer-gpu`, which it builds as workspace members.  So their lib `#[expect]`s fail on both steps when the path is off.
* 2026-10-21, M12: the planned reason for the test modules, "tests copy a few bytes", was false for `roformer/tests/reference.rs`, which copies fixture audio.  The reason is now "test code, which does not ship".

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
* 2026-10-19, agent (M4): the `cargo tree` check of the web app moves from M5 to M4, so M4's result has an automated check (VERIFY-1).
* 2026-10-19, agent (M4): the check runs `cargo tree` once for each of `app` and `stem-worker`.  One run with both merges their features, and puts the blame on `app` for a breach in `stem-worker`.
* 2026-10-19, agent (M4): `app`'s dev-dependency on `synth` with `files` is native-only.  Only the native UI test writes files, and `hound` stays out of the `## Browser tests only` table.
* 2026-10-19, agent (M4): the check reads the children of the web app's `synth`, not a list of banned crates.  A feature check passes on today's tree, and a ban on `serde` gives false alarms.
* 2026-10-19, agent (M4): the check reads the wasm tree only, as `THIRD-PARTY.md` does, because only the web app ships.
* 2026-10-19, agent (M4): CI's `clippy` job also runs clippy on `synth`, on `app`'s lib, and on `app` with `--all-targets`.  `--workspace` builds turn `files` on everywhere.  So they hide warnings in `synth` without `files`, a native-only call to a writer in `app`, and a lost dev-dependency.  `-p app --all-targets` alone also turns `files` on for `app`'s lib, through the dev-dependency (review round 7).
* 2026-10-19, agent (M4): M4 writes decision 0004 as `proposed`.  The `files` boundary and LAYER-1's new text outlive this plan.  The maintainer accepted it on 2026-10-20.
* 2026-10-20, agent (M5): the layers table gets one row per crate.  The `;` form maps each list to a crate by position, which a person can misread.
* 2026-10-20, agent (M5): the plan recommends an exact table, both ways.  A row that lists a crate without an edge is a stale doc (DOC-1), and the two-way match keeps the check from passing on empty input.  The first M5 text gave a ceiling, so the maintainer decides it (question 2 of M5's decision points, review round 6).
* 2026-10-20, agent (M5): the check finds edges by `path`, not by name.  A registry crate can have a member's name, and a renamed dependency keeps its package name in `name`.
* 2026-10-20, agent (M5): the plan recommends a second check that keeps the page side and the worker side apart, over normal edges.  A row of the table may list any crate of a lower layer, so a change to `app`'s row, with the dependency, lets `app` take `roformer`.  The maintainer decides how strict it is (question 1 of M5's decision points), and step 2 records both answers as decision 0005.
* 2026-10-20, maintainer (M5): option (a) for the two sides, and (x), an exact table.  (a) fails a native-only dependency of `app` on the model crates.  The app has no plan to separate natively, so this is a conscious choice, to revisit if it does (decision 0005, "Revisit if").
* 2026-10-20, agent (M5 review round 1): a dependency that goes round the members is a problem.  A path crate excluded from the workspace, or a git crate with a member's name, hid its dependencies from the check.
* 2026-10-20, agent (M5 review round 2): the check reads `cargo metadata` without `--no-deps`, with `--locked`.  A `[patch]` to a path crate outside the members has no `path` in `--no-deps` output, so only the full `packages` shows it.  `--locked` keeps the test from writing `Cargo.lock`.
* 2026-10-20, agent (M5 review round 2): what LAYER-1's crate checks cannot see goes into a new tech-debt entry: a copy of workspace code in an outside crate, and `#[path]` or `include!` across crates.
* 2026-10-20, agent (M5 review round 3): `--all-features` too, because a `[patch]` crate behind a feature that is off is not in `packages` without it.  On today's tree it lists the same 937 packages.
* 2026-10-20, agent (M5 review round 3): `test-support` takes no workspace crate, of any kind.  Dev edges to it are not in the table, so a relayered `test-support` could otherwise carry a dependency upwards.
* 2026-10-20, agent (M5 review round 4): the check also reads `resolve.nodes`, and fails on a crate outside the workspace crates that takes one.  LAYER-1 states the rule on strays.  The cost: a patched or vendored path crate must become a workspace crate with a row.
* 2026-10-20, agent (M5 review round 7): `rules::LAYERS` fixes the five layers, and the table's layers must equal it.  Otherwise an agent that follows the rank message can add a layer, and make LAYER-1's list false.  A change of layers changes the table, `LAYERS` and LAYER-1 in one commit.
* 2026-10-20, agent (M5 review round 6): a package outside the members with a member's name fails, even when it is an unrelated crate.  crates.io has crates named `app`, `dsp`, `engine`, `synth` and `xtask`, with almost no reverse dependencies.  If one is ever pulled in, the check gets an allowlist with a reason.
* 2026-10-20, agent (M6): the lints that `dsp` relaxes are crate-level `#![expect]` attributes in `dsp/src/lib.rs`, one for each lint, each with a reason.  `expect` fails a stale exception.  Cargo's `[lints]` cannot hold them: it rejects overrides beside `workspace = true`, and has no `expect` level.
* 2026-10-20, agent (M6): the plan recommends keeping all four lints, option (x), so no function body changes.  The maintainer decides (decision point in M6's design).
* 2026-10-20, agent (M6): M6 adds no check of the lint attributes.  M7 parses the same files with `syn` and closes the gap; TD-3 names it until then.
* 2026-10-20, agent (M6 plan review round 1): under (x), DSP-1 names the beginner forms, so each `expect`'s reason cites DSP-1's text.  Without it, DSP-1 forced only `manual_swap`, and M6's own rule for a new lint failed the other three.
* 2026-10-20, agent (M6 plan review round 1): the types outside DSP-1's list are a breach, with their own entry under `## Known breaches`, not a sentence in TD-3.  TD-3 is a gap in enforcement; M7 closes both.
* 2026-10-20, agent (M6 plan review round 2): decision 0006 records the choice at M6's decision point.  Under (x), DSP-1 gains a ban on `+=`, `-=`, `is_empty` and `clamp`, which is a rule change.
* 2026-10-20, maintainer (M6): option (x).  `dsp` keeps `x = x + 1`, `len() > 0`, the two `if`s of the clamp and the swap through `temp`, each lint as an `expect` with a reason.  DSP-1 forbids `+=`, `-=`, `is_empty` and `clamp`.
* 2026-10-20, agent (M6 review round 1): DSP-1's ban covers every compound assignment, not only `+=` and `-=`.  The `assign_op_pattern` `expect` silences all of them, so a ban on two left the others allowed and unchecked.  This keeps (x)'s intent, so decision 0006 is amended, not replaced.
* 2026-10-20, maintainer (M7): a token scan of the likely breaches, not a `syn` allowlist.  `dsp` is small, rarely changes and was never breached, so a full allowlist costs more than it saves.  TD-3 narrows instead of closing.
* 2026-10-20, agent (M7): DSP-1's list of types is corrected to the types the code writes, with no decision record.  The text was wrong, and no choice is made.
* 2026-10-20, agent (M7): small refinements to the scan, found while implementing and in review.  A `for` with no `in` is not a loop.  A listed `#![expect]` passes only at the top level of `dsp/src/lib.rs`.  Names in an import's braces count as after `::`, and a name after its own module passes.  `r#` is stripped from attribute names and `include!`.  `dsp/Cargo.toml` is compared with LF line ends.  Each closes a false result that a test now pins; none changes what DSP-1 permits.
* 2026-10-20, maintainer: M14 keeps only the deny rules, M15 only the `rustfmt` hook, and M16 moves to phase 5b.  The hooks in `xtask` cost a build per tool call and are not controls.  No deleted test has been seen here.  This replaces the 2026-10-19 decision that hooks are an `xtask` command.

* 2026-10-20, agent (M8): TD-5 and TD-6 close with no new entry.  What the checks cannot see goes in the `* Enforced by:` lines, as GPU-1's line does.  No fix is planned for it, so an entry would add nothing.
* 2026-10-20, agent (M8): the checks read the git index with `git ls-files`, not a walk of the folders.  The index is what the next commit holds, and the walk skips the ignored folders where audio lives.
* 2026-10-20, agent (M8 plan review round 1): a literal match for Python's web server joins the BUILD-1 check, over every tracked text file outside plans and decisions.  It is the breach that an agent most likely writes, and it gives no false result today.
* 2026-10-20, agent (M9): the plan recommends option (b), a crate below 1.0 is young unless `SETTLED_CRATES` names it with a first-version date two years old or more.  It fails on the breach that happened (a young crate added with a caret), and needs 5 pins, not 18.  The maintainer decides (M9's decision point).
* 2026-10-20, maintainer (M9): option (b).  A crate below 1.0 is young unless `SETTLED_CRATES` names it, with a first-version date two years old or more.  5 pins and a list of 13.
* 2026-10-20, agent (M9): the pin check covers every member and every kind of dependency, not only what ships.  Build scripts and proc macros of dev-dependencies and of `xtask` run on the maintainer's Mac.
* 2026-10-20, agent (M9 plan review round 1): a line of `cargo tree` with text in parentheses that is not a workspace path or `proc-macro` is a problem, not a dropped line.  The check fails closed, so a git or path crate cannot leave the licence list in silence.
* 2026-10-20, agent (M9 review round 2): the licence check drops a path crate only when its folder is a member's, not any folder under the root.  A vendored copy under the root still ships, so it must fail closed.
* 2026-10-20, agent (M9 plan review round 2): a git dependency in a member is a DEPS-1 problem.  None exists today, and one that tracks a branch is the least pinned kind.  For a crate that `SETTLED_CRATES` does not name, a requirement with a pre-release passes only as an `=` pin.
* 2026-10-20, agent (M10): the check reads `stem_protocol::kind` from `protocol/src/lib.rs` with `proc-macro2`, not through a dependency.  A dependency needs a second list of the constants and a new edge in the layers table; the scan needs neither.
* 2026-10-20, agent (M10): the scan reads the shapes of a type that the scripts use today, and the reversed comparison.  Other shapes, and field names, stay in TD-16, by the rule for proportionate sensors.
* 2026-10-21, agent (M12): each path in a `clippy.toml` gets a canary, an `#[expect]` on a deliberate copy in a test.  A wrong or stale path passes clippy in silence, so without the canary the lint can be off with no sign.
* 2026-10-21, agent (M12): CI's `clippy` job gains a `wasm32` step for `app`, `stem-worker`, `stem-protocol` and `engine`.  Without it the lint never sees TD-13 or TD-14.
* 2026-10-21, agent (M12): the plan recommends option (a) at M12's decision point: `app`, `worker`, `protocol` and `engine`.  The maintainer decides.
* 2026-10-21, maintainer (M12): option (b), `app`, `worker`, `protocol`, `engine`, `roformer` and `roformer-gpu`.  MEM-1 covers model-sized buffers, and the lint's value is the next copy.  The agent's recommendation of (a) weighed known breaches too much.  Recorded as decision 0008 in review round 3.

## Outcomes and retrospective

To be written when the plan completes.
