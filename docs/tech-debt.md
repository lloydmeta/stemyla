# Tech debt

Known gaps in what enforces the rules, and known breaches of them, by ID.  Each entry names the rule or the measure it concerns, the evidence, and the fix in view.  To close an entry, delete it in the commit that fixes it.  In the same commit, add its ID under `## Closed`, with what closed it.  An ID is never reused.  Plans, decisions and review logs may still name a closed ID; `docs-check` fails it in other docs.  Add new entries above `## Closed`.

Entries found on 2026-10-19 by a survey of the code for the harness plan (`plans/active/harness-engineering.md`); the plan's phase 2 closed most enforcement gaps (`## Closed`).

## Enforcement gaps

### TD-3: DSP-1 is checked only for the likely breaches

`dsp_is_beginner_style` scans the tokens in `dsp/` for the likely breaches.  It does not read types, so a type outside DSP-1's list passes.  A closure with no arguments (`|| x`) passes, because it has the same tokens as `||`.  A construct that the scan does not look for, such as `match` or `loop`, passes.  A glob import, such as `use std::mem::*`, hides the names that it brings in.  A `-A clippy::all` in `RUSTFLAGS`, in a `config.toml` in `.cargo/`, or in `[workspace.lints]`, passes too, for every crate.

* Fix: review, by the `reviewer` agent, which looks for DSP-1 forms that the scan does not read.  No check is planned.  `dsp` is small and seldom changes, so a full allowlist costs more than it gains (`plans/completed/harness-phase-2.md`, M7).

### TD-7: no licence policy and no allowlist of registries

`young_crates_are_pinned` and `third_party_lists_what_ships` check the pins and the licence list (DEPS-1).  Nothing fails a licence that is not permissive.  A crate from a registry other than crates.io fails only if it is in a tree of `THIRD-PARTY.md`, since `third_party_lists_what_ships` cannot read its line.  A vendored copy of a crate that is made a workspace crate leaves both checks; only a reviewer sees it.

* Fix: `cargo deny` for licences and registries.  It is a new tool, so vet it first (DEPS-1).  Until then, the `supply-chain-reviewer` agent checks the licence and the registry of each new crate with `cargo info`.

### TD-8: some evidence runs nowhere, and required skips rest on a person

`STEMYLA_REQUIRE` turns a skip into a failure (M3 of `plans/completed/harness-phase-2.md`), but these gaps stay (VERIFY-1, GPU-2, MEM-3):

* The PyTorch comparisons run nowhere, because neither CI nor the maintainer's Mac has the fixtures.  Fix: the maintainer makes them on the Mac with `tools/roformer_export.py` and the PyTorch checkpoint.  Then `STEMYLA_REQUIRE=fixtures` is the evidence for the CPU, and `STEMYLA_REQUIRE=gpu,fixtures` for the GPU.
* GPU and fixture evidence holds only when a person sets `STEMYLA_REQUIRE`, and nothing checks that a PR's evidence did.  Fix: phase 4's evaluator sets the variable in its contract.
* `every_skip_can_be_required` sees only messages that start with `skipping` or `skipped`, in any case, so a silent early `return` passes it.  Fix: the `reviewer` agent looks for it.
* `Job::set_input`'s write to the GPU is checked only on a GPU (`set_input_writes_the_normalised_bands_to_the_gpu` in `roformer-gpu/tests/memory.rs`).  CI does not run roformer-gpu's integration tests (`.github/workflows/ci.yml` runs its `--lib` and `--doc` tests only).  A machine with no adapter skips it, and the skip passes without `STEMYLA_REQUIRE=gpu`.  The same holds for `the_masks_follow_the_input_through_the_shared_steps`.  It compares the lists of `Job::shared_steps` and `Job::steps` with `layer_steps` and `mask_steps`, and checks that the masks follow the input.  It does not check the steps inside `layer_steps` and `mask_steps`, or the result of each step that `Job::run` runs.  `xtask e2e selftest` checks those, stage by stage against the CPU port (`selftest=stages`), with the model on a GPU.  An agent or a person runs it by hand; CI does not run it.  `forward_matches_pytorch` checks them too, with fixtures that no machine has (the first bullet).  The pure part, `normalise_bands`, has a test with no GPU.  Fix: a CI runner with a GPU; until then, the bullet on `STEMYLA_REQUIRE` above.
* The network's arithmetic in `roformer-gpu/src/net.rs` (`transformer`, `split_attention`): review round 1 of M5 of `plans/active/gpu-speed.md` left 21 of its 28 mutants there alive, each older than M5.  Each changes an offset, a stride or the scale of the scores, so each dispatch keeps its kernel and its shape.  `calibration_records_the_step_of_the_separation` (`roformer-gpu/tests/memory.rs`) kills the 7 that drop a dispatch or change a shape, with `timestamps` in `STEMYLA_REQUIRE` (the bullet below).  A test against the CPU port kills all 28: the round's validator ran one with the model.  Fix: such a test with random weights of the model's shapes, which needs no model.
* Six tests need `timestamp-query` (`Need::Timestamps`): `profile_labels_dispatches`, `profile_labels_attention_by_its_shape` and `profile_recording_and_reset` in `roformer-gpu/tests/kernels.rs`, `calibration_records_the_step_of_the_separation` and `collected_profile_readbacks_are_freed` in `roformer-gpu/tests/memory.rs`, and `forward_matches_pytorch`.  The last one needs the fixtures too, so it runs nowhere (the first bullet).  The M2 Pro's adapter has `timestamp-query`, so the other five run there; CI excludes them.  This is a known gap: `check --tier push` and the command of GPU-2 set `STEMYLA_REQUIRE=gpu,f16`, so they do not require these tests, and an adapter without `timestamp-query` skips them with a pass.  Fix: add `timestamps` (`STEMYLA_REQUIRE=gpu,f16,timestamps`), so that a skip of these tests fails, and name the adapter in the PR.  Until then, a CI runner with such a GPU is the full fix.

### TD-10: GPU-3 is not checked

Nothing ties a new default kernel to a pass on a named real GPU, and any new `TILES` entry becomes a calibration candidate on every user's GPU.  The log part of the old fix is done: the worker's `model loaded`, `selftest: GPU` and `bench: GPU` lines name the adapter and its `isFallbackAdapter` value (`gpu_text` in `worker/src/web.rs`).  `e2e perf` records them in its result, and refuses as a pass or a baseline each fallback value but `no`, and a software adapter.  An entry deleted by hand from `xtask/perf-baselines.json` skips the raise rule: the next update has no old entry, so `update_refusal` cannot refuse it.  Nothing checks a deleted entry; the PR diff of `xtask/perf-baselines.json` shows it.

* Fix: tie a new default kernel to a pass on a named real GPU in a check.  Until then, the PR names the GPU, and the `gpu-reviewer` agent flags a diff that makes a kernel the default, on each diff that routes it.  It does not confirm the pass on a real GPU.  That evidence is in the PR description, which the agent does not read.

### TD-11: MEM-4 is asserted only in part

The worker logs its wasm memory and live GPU bytes and the page its wasm memory.  `e2e memory-budget` asserts a tab total during a separation at the default chunk with f16 weights (M1 of `plans/active/gpu-speed.md`).  The total is page wasm, worker wasm, decoder wasm, the write worker's wasm and GPU live bytes.  The write worker logs no memory.  So the total counts an estimated 1.25 MB for it (`WRITE_WORKER_WASM_MB`): its module's 17 pages at the start and 2 pages more.  Nothing measures its growth.  It runs with the model, not in CI.  The page's WebGL textures are not counted.  The JS heap is counted only during a save: `e2e save` measures the page's JS heap in Chrome, with `performance.memory`, and checks that it does not grow with the song (M8).  Safari gives no `performance.memory`, so `e2e save --browser safari` prints the calibration and each heap check as "not measured in Safari" (M4 of `plans/completed/m4a.md`).  Nothing measures the JS heap during an open, a separation or playback, and the workers have no `performance.memory`.  So nothing measures the write worker's JS heap.  While it writes a request, it holds that request's array or one slice of it.  A save block is 2 MiB, and a mix block is 1 MiB (`MIX_BLOCK_FRAMES`).  A stem WAV of the memory path is one whole array, up to about 17.6 MB for 100 s.  A source file comes from a `Blob` in slices of 8 MiB.  Its queue also holds the body of each request that waits: a transferred array, or a `Blob`.  Each flow that runs sends one request at a time.  The flows are a save, a mix write, a stem save of the memory path and a source save.  So the queue holds at most one body for each flow that runs.  The stem worker holds the same slices while it writes a picked model.

* Fix: the tab total is done (`e2e memory-budget`, M1 of `plans/active/gpu-speed.md`).  Left: the JS heap during an open, a separation and playback, and the workers' JS heaps.  Also left: a measure of the write worker's WebAssembly memory, and the page's WebGL textures.
* The wasm test `a_chunk_of_the_default_length_holds_its_working_set` (CI `browser-tests` job) runs the worker's CPU half (`chunk_stems`, `worker/src/web.rs`) at the default chunk with a fake GPU half, which calls the real normalisation of one band (`roformer_gpu::net::normalise_bands`).  It checks each phase of a chunk against `roformer::separate::working_set`, so it sees that the CPU half drops a chunk's features (18.9 MB at the default chunk) after the upload.  No test without a GPU sees the real `Job::set_input` and `Gpu::read`: only a run of the model sees them, and CI has no GPU and no model.  `e2e memory-budget`'s check 3 includes them in the worker's growth, at the default chunk with no `mm=s64`, with the model and not in CI.

### TD-21: what docs-check cannot see

`xtask docs-check` (DOC-1) checks the shape of the docs, not whether they are still true:

* A doc that names real files and tests but describes old behaviour passes.
* An ID with a prefix not yet in use (`SEC-1`) is not checked, because it cannot be told from text such as `SHA-256`.
* A path counts as present if the file is on disk, so a file not yet added to git passes locally and fails only in CI.
* `README.md` and crate docs (`//!`) are checked for the crate list only.
* A bare file name (`store.rs`) is not checked, because the check cannot tell which crate it means.

* Fix: the drift job in phase 5a of the plan, which compares each doc with the code it describes.

### TD-23: LAYER-1 does not see a copy of a workspace crate from a registry

`crates_depend_only_downwards` reads the dependencies between crates (LAYER-1).  Since tech-debt rung 1, `code_stays_in_its_crate` also reads the source.  It fails a `#[path]` or an `include!` that takes code from outside its crate.  It also fails a target whose source is outside its package folder, and a tracked symbolic link.  What is left:

* A crate from a registry or git, under another name, can hold a copy of a workspace crate, such as `roformer`.
* The check finds `include!` by its name only, so a renamed import, such as `use std::include as inc;`, passes it.
* `include_str!` and `include_bytes!` pass in every crate, from any folder.  So a crate can take a file of another crate, such as a WGSL shader, with no dependency.  Today only `xtask` and `engine/tests/realtime.rs` read files of other crates, as text.
* Fix: the `supply-chain-reviewer` agent looks for each of these forms, on each diff that routes it (TD-31).

### TD-25: RT-1's allocation test runs natively only

`the_callback_does_not_allocate_after_warm_up` does not count the WebAssembly build, cpal's hosts, or the closure in `engine/src/live.rs`.  That closure calls `Schedule::next`, whose fields are all `Copy`.  On the web, the first `web_time::Instant::now` in `render_interleaved` fills a `thread_local`.  A zero count is evidence, not proof, since the optimiser may remove an allocation.  The bench-only stretchers (`timestretch`, `cute-dsp`) are not covered.  `the_app_ships_only_these_stretchers` reads the text of `app/Cargo.toml` only, so it does not see a stretcher that another crate turns on through feature unification (RT-1).

* Fix: none planned; phase 4's evaluator may count allocations in the browser.

### TD-26: MEM-1's lint sees only `to_vec`

Clippy fails `slice::to_vec` in `app`, `stem-worker`, `decode-worker`, `write-worker`, `stem-protocol`, `engine`, `roformer`, `roformer-gpu`, `synth` and `m4a`, and `to_vec` on a `Uint8Array` or a `Float32Array` in the first five (MEM-1, MEM-2).  It does not see:

* other forms of copy: `to_owned`, `Vec::from`, `Box::from`, `into`, `concat`, `collect`, `extend_from_slice`, `copy_to` into a new buffer, and `clone` of a `Vec`;
* `to_vec` on the other typed arrays, such as `Int16Array`, and a wasm-bindgen import that returns a `Vec<u8>`;
* a `#[wasm_bindgen]` export that takes a `Vec<f32>` or a `&[f32]`, which copies a JS array in;
* an `#[allow]` in place of an `#[expect]`, as clippy's own help line suggests, and a reason that does not say why;
* a new `to_vec` in a statement that already has an `#[expect]`, and a new test in a test module with a module-level `#[expect]` or `#![expect]`;
* a path that no longer resolves, except through the canaries, which prove their paths only while the clippy steps keep `--all-targets`.  Nothing fails if the `wasm32` step goes.

Since tech-debt rung 1, `each_shipped_crate_lints_mem1_copies` fails a shipped crate whose `clippy.toml` lacks `slice::to_vec`.  It fails a crate of `MEM1_JS_CRATES` that lacks either JS-array path.  It also fails a crate without a canary that proves the paths: one statement for each path, under `#[expect(clippy::disallowed_methods)]` and with no `#[cfg]`, that ends in `.to_vec()` on that type.  The check parses each file with `syn`.  It skips a node that a `#[cfg]` turns off.  Such a node is an item, a statement, an expression or a match arm.  It is also a field, a variant, a field value or a generic parameter.  It is also a foreign item, a function, closure or variadic parameter, or a field pattern.  Only `test` and `target_arch = "wasm32"` keep a node.  It also skips a file or `mod` whose inner `#![cfg]` turns it off.  `dsp`, `xtask` and `test-support` are exempt, each with its reason.  What it does not see:

* a new crate that reads JS arrays but is not in `MEM1_JS_CRATES` (`xtask/src/rules.rs`), which needs only the `slice::to_vec` path.  `engine` takes `js-sys` for `cpal`, and is not in the list.  `roformer-gpu` reads a mapped `Float32Array` through `wgpu::web_sys` in `mapped_floats`, and is not in the list either.
* a `#[cfg]` on the `mod` line, in another file, that brings in the canary's file.  The check reads each file alone.
* a canary in a macro call, which `syn` does not parse.  Such a canary counts as missing, so the check fails safe.

* Fix: none planned.  The `memory-reviewer` agent looks for a new copy of any form, on each diff that routes it (TD-31).

### TD-27: MEM-3's lint sees only how `roformer-gpu` makes buffers

Clippy fails five ways to make a GPU buffer in `roformer-gpu` outside `GpuBuffer` (MEM-3), and no other crate reaches a `Gpu`'s device.  It does not see:

* a device that another crate, or a test in another crate, opens with `wgpu` itself; `stem-worker` takes `wgpu` directly, for `wgpu::PowerPreference` only, and a re-export of it from `roformer-gpu` would let it drop `wgpu`;
* textures and query sets, which the web backend does not free either; `Gpu`'s profiling query set is never destroyed, and lives as long as its `Gpu`;
* a buffer made in a statement that has an `#[expect]` for a MEM-1 copy, since both rules share the lint; the file-wide `#![expect]`s in `roformer-gpu/tests/memory.rs` and `tests/reference.rs` pass a buffer made there on a device that the test opens itself;
* an `#[allow]` in place of an `#[expect]`;
* a path that no longer resolves after a wgpu update, except through the canary, and a new helper that makes buffers in a later wgpu;
* a second call that makes a buffer in the `let` of a `GpuBuffer` constructor, which its `#[expect]` passes;
* a new public function or `Deref` on `Gpu` that returns or lends the device, or a clone of it, or a new public constructor that takes a device, since the caller can keep a clone;
* other ways in wgpu to make a buffer that do not run on the web today: `create_buffer_from_hal`, `create_blas` and `create_tlas`;
* a `GpuBuffer` that is kept and never dropped; the tests in `roformer-gpu/tests/memory.rs` measure that on a real GPU only.

* Fix: none planned.  The `gpu-reviewer` agent looks for GPU memory that is not destroyed, on each diff that routes it.  A sign in another crate routes it, except in a `tests` folder (TD-31).

### TD-28: GIT-1's deny rules match command text only

Deny rules in `.claude/settings.json` refuse a push that names `main` or `gh-pages`, `xtask publish` and `gh pr merge`, in the forms that an agent usually writes (GIT-1).  Claude Code matches them against the command text, so they are not a security boundary.  They also refuse by mistake:

* a branch whose name ends in `main`, such as `fix-main`, or holds `gh-pages`, `--all`, `--branches` or `--mirror`;
* a pipeline that starts with a push or `cargo run` and names one of those words or `publish` later, such as `git push origin x | grep main` or `cargo run --help | grep publish`.  In runs on 2026-10-21, Claude Code matched a rule against a whole pipeline, but matched a chain with `&&` part by part.  Run the command without the pipe.

They do not see:

* a push that names no branch, such as `git push` or `git push origin HEAD` while on `main`; a branch named through a variable or a config, such as `push.default`; the `main` ruleset refuses such a push to `main`, but not to `gh-pages`;
* a push that reaches `main` through the matching refspec `:`, a glob refspec such as `refs/heads/*:refs/heads/*`, a quoted branch (`"main"`) or an abbreviated option (`--mir`, `--bra`);
* a push to another branch without being asked, which rests on prose (decision 0010);
* a shell or `eval` that runs a string, and a script that the agent writes and then runs;
* `git -C`, `git -c`, a git alias, `env`, an absolute path to `git` or `xtask`, a path to `xtask` outside `target/`, and a quoted subcommand;
* a `gh` alias, such as `gh alias set m 'pr merge'`;
* a cargo option before `run`, such as `cargo -q run` or `cargo +nightly run`;
* `gh api`, which can merge a PR;
* a session started outside the repo root, or moved with `/cd`, since Claude Code reads the project settings from the session's working directory only;
* a mod that answers `tool.check`, which can approve a denied call on a personal account;
* a cloud session in a project or with several repositories, and a session started with `--setting-sources` without `project`; these read no project permission rules ([settings](https://code.claude.com/docs/en/settings));
* an edit to `.claude/settings.json` during a session, which Claude Code reloads at once;
* whether Claude Code refuses anything: `claude_settings_hold_the_deny_rules` keeps the rules in the file, and M14's step 3 showed the refusals by hand once; a rule taken out of both the file and `REQUIRED_DENY_RULES` passes every test, and only review, by the `supply-chain-reviewer` agent, sees it.

* Fix: re-run M14's step 3 (`plans/completed/harness-phase-2.md`) after a change to the deny rules, or an upgrade of Claude Code or RTK.  After another change to `.claude/settings.json`, run the 9 commands in M15's step 2.  On GitHub, the `main` ruleset refuses a direct push to `main`, and the `gh-pages history` ruleset refuses a rewrite of `gh-pages` (TD-20).  Nothing on GitHub refuses a fast-forward push to `gh-pages`, a PR merge with the admin override, or `xtask publish`.  Phase 5 adds the agents' own identity.

### TD-29: no check stops a change that makes a snapshot pass

A larger `threshold` or `max_failed_pixels`, in a `kittest.toml` or in `SnapshotOptions`, passes a real change of the UI.  egui_kittest reads a `kittest.toml` in any folder above the test, also outside the repo.  A PNG rewritten with `UPDATE_SNAPSHOTS=1` and committed passes.  dify does not count a pixel that it finds anti-aliased, and the default passes a change of 1 colour level (VERIFY-1).

`no_build_file_updates_snapshots` reads only the build files in the repo.  These pass it:

* `std::env::set_var` in a test, or a variable from `xtask`;
* a repository variable on GitHub;
* `[env]` in a Cargo `config.toml` outside the repo;
* `env` in a person's own Claude Code settings.  `env` in `.claude/settings.json` fails `claude_settings_hold_the_deny_rules`, which allows only `$schema`, `permissions` and `hooks`;
* `#[ignore]` on a snapshot test.

* Fix: review; a changed PNG or threshold is a UI change, and the `reviewer` agent looks at it.  No check is planned.

### TD-31: the route of `xtask review` is a short denylist, and the route and the log check come from the branch's own code

This entry concerns which reviewers read a diff, and the check of a review log against the fix rule of decision 0024.  So it touches each rule that a reviewer checks, not one rule only.

`xtask/src/review.rs` chooses the reviewers of a diff from its paths and from content signs.  The content signs are a short denylist, not a full list.  A sign matches as a whole token only.  So `create_buffer` does not match `create_buffer_init`, which `roformer-gpu/src/lib.rs` calls.  A sign counts only on an added line of a Rust file outside a `tests` folder and outside the reviewer's own paths.  A copy or a GPU buffer that a macro, an alias or a helper makes routes nothing.

Since M6, review round 1, the paths of `memory-reviewer` include `roformer/src/`, `roformer-gpu/src/` and `synth/src/`.  So a change to the GPU memory budget (MEM-4) or to the model read (MEM-1) routes it.  Since M6, review round 2, they also include `dsp/src/`.  Its code runs in the audio callback (RT-1) and builds the song-length mix (MEM-4).  `e2e_messages_use_protocol_types` enforces PROTO-1.  A change to its list `SCRIPT_ONLY_TYPES` in `xtask/src/rules.rs` routes only `supply-chain-reviewer` and `reviewer`.  Neither agent lists PROTO-1, so no reviewer checks PROTO-1 on a change to that list.

The route comes from the branch's own `xtask`.  So a branch can change the route, or remove the rule that a change to `xtask/src/review.rs` routes every reviewer.  The log check of `xtask review log-check` and `docs-check` also comes from the branch, in `xtask/src/review_log.rs`.  So a branch can change the fix rule that its own log must follow.  `xtask review start` warns when the branch changes, adds or removes `xtask/src/review.rs` or `xtask/src/review_log.rs`.  It also warns when the binary is older than one of them.

* Fix: review; `reviewer` reads every diff, and a change to `xtask/src/review_log.rs` routes every reviewer.  Phase 5 routes with the base branch's `xtask`, and checks the log with it.

### TD-32: `xtask review check` sees only what its record holds

This entry concerns no rule.  It concerns the state check of a review round.

`xtask review check` compares the repo with the state record of a round.  The record holds `HEAD`, the status, the stash, the branches and the tags.  It holds every other ref, such as remote-tracking refs, notes and replacements, and the remotes and the worktrees.  It also holds a hash of the git config, the hooks and the local Claude Code settings in the repo.  The hash also covers the person's own Claude Code settings in the home folder.  The check does not see other changes, for example:

* a change to an ignored file other than the local Claude Code settings, or to a file under `target/`;
* a change to the reflog, the global git config, or the index of another worktree;
* a change to a file outside the repo, other than the person's own settings.

A change to the person's own settings during a round, by a person or by Claude Code, fails the check.  With a detached `HEAD`, the name of the run folder holds the short SHA.  So after a fix commit, round 2 finds no record.

The `review-rounds` skill (harness phase 3, M3) stops a round when `review check` fails.  But a change that the record does not hold passes the check, and the round goes on.

* Fix: the person examines the change, and starts the round again.  With a detached `HEAD`, start again at round 1.

### TD-33: `xtask review mutants` tests only part of a diff

This entry concerns VERIFY-1, through the mutants run of a review round.

`xtask review mutants` runs cargo-mutants on the Rust files of a round's diff.  It does not test some changes, and it reports some mutants that no native test can catch:

* cargo-mutants builds and tests natively.  A `wasm32`-only block in a shared function shows as a missed mutant.  A `wasm32`-only function, impl or module in a shared file has `#[cfg_attr(mutants, mutants::skip)]`, so cargo-mutants makes no mutant of it, struct-field deletions included.  No build sets `cfg(mutants)`, so no crate is needed, and `[workspace.lints.rust]` in `Cargo.toml` declares the cfg for `unexpected_cfgs`.  A new `wasm32`-only function without the attribute shows as missed, until a change adds the attribute.  The 22 rounds of PR #39 after `6d0a39d` had 11 such lines, in 2 rounds (harness phase 3b, M2).  A function with a native variant too, such as `base_latency` in `engine/src/live.rs`, `main` in `app/src/main.rs` and `mapped_floats` in `roformer-gpu/src/lib.rs`, has no attribute.  The validator decides its `wasm32` mutants.  On 2026-10-28 (review round 1 of `plans/completed/stream-from-storage-catchup.md`), a scan of `cargo mutants --list --all-features` found no other `wasm32`-only function, impl or module that shows mutants.
* `exclude_globs` in `.cargo/mutants.toml` leaves out `app/src/decode_tests.rs`, `app/src/decoder/client.rs`, `app/src/picker.rs`, `app/src/playback.rs`, `app/src/playback_tests.rs`, `app/src/separator.rs`, `app/src/store.rs`, `app/src/streamtest.rs`, `app/src/writer/client.rs`, `app/src/writer/client/tests.rs`, `app/src/writer/fake.rs`, `app/src/writer/routing_tests.rs`, `decoder/src/web.rs`, `decoder/src/web/`, `worker/src/web.rs`, `worker/src/web/`, `writer/src/web.rs` and `writer/src/web/`, which compile for `wasm32` only.  So the run never tests the wire files of any worker: `app/src/separator.rs` and `worker/src/web.rs` for the stem worker, `app/src/decoder/client.rs` and `decoder/src/web.rs` for the decode worker, and `app/src/writer/client.rs` and `writer/src/web.rs` for the write worker.
* `mutants_command` sets no `STEMYLA_REQUIRE`, and passes on the caller's own.  So without it, where no GPU opens, the GPU tests skip, and a mutant that only they cover shows as missed.
* Native code that only the `wasm32` tests reach shows as missed, such as the JS readers of `protocol/src/lib.rs`.  The validator refutes it: 16 lines in the 22 rounds of PR #39 after `6d0a39d`.
* `xtask review mutants` writes the missed lines that `KNOWN_MISSED` (`xtask/src/review.rs`) matches to `known.txt`, each with its TD.  It writes the rest to `missed.txt`.  A test fails when an entry names a closed TD, or code that is gone.  It does not check that the mutant is still at the entry's code line.  An edit can keep the line and move the mutant, such as a new first statement in a function.  Then the entry goes stale, and its mutant goes to `missed.txt`, so nothing is hidden.  No test checks an entry's mutation text or offset against `cargo mutants --list`.  A wrong text or offset can send its own mutant to the validator.  A wrong offset can also hide another mutant with the same text on the same line.  If the tree is not clean at the round's head as the run starts, each missed line goes to `missed.txt`, and the mutants line says why.  The `review-rounds` skill logs each line of `known.txt` as `known`, with its TD, and gives it no validator.  A mutant that a team sends to tech debt gets its `KNOWN_MISSED` entry in the milestone's last commit, not in a round's fixes.  Reason: a change to `xtask/src/review.rs` routes all six reviewers, so an entry between rounds widens the next round.
* A diff of tests only, WGSL or JavaScript gives no mutants.
* A run that stops at its limit tests only some mutants, so its result is partial evidence.
* cargo-mutants copies the tree with the ignore rules, the person's global ignore file included.  A tracked file that a rule matches is missing from the copy.  `no_tracked_file_is_ignored` finds such a file with the rules of the machine that runs it, and with a global rule for `Cargo.lock`.  CI has no global rules, so it sees only that one.
* The baseline can fail on a slow test.  In M5, 5 of the 8 case runs that were not dry runs had `classic_drums_beat_the_raw_mix` (`xtask/tests/m8_hpss.rs`) in the baseline.  It timed out in the unmutated tree in 3 of them (`m9`, `m2` and `m10` run 1), and passed in 2 (`docs/reviews/replay.md`).  Where it timed out, no mutant ran, and the line was `none: ...; exit 4: the baseline failed`.  It also failed in rounds 1 and 2 of the 48k audio branch.  Since 2026-10-24 the run builds with the optimised `mutants` profile in `Cargo.toml`.  Then the baseline of that branch's whole diff passed, and 98 mutants ran in 16 minutes.

* Fix: the validator decides each missed mutant (phase 3, M3).  Phase 6 reads the log.  If a slow test makes the baseline time out again, time the tests under the `mutants` profile first.  A replay of a review on a commit before the profile came in has no `[profile.mutants]`.  So remove the `profile` line from the copied config, for example with `sed '/^profile = /d'`.  Else cargo-mutants stops with "profile `mutants` is not defined", and the line says `exit 4: the baseline failed`.

### TD-34: the review agents' guard matches text

This entry concerns GIT-1, and the read-only rule of the review agents.

`.claude/hooks/read-only-bash.sh` is a PreToolUse hook on Bash for the review agents (harness phase 3, M2).  It refuses a command that would change the repo, a remote or a published package, by `git`, `cargo`, `gh` and a short list of other commands.  It also refuses a wide search: `find` or `du` from the whole disk, a volume or a home folder, and `mdfind` without `-onlyin` a narrower folder.  It matches the command text, so it stops mistakes, not an agent that tries to escape, as TD-28 says of the deny rules.  It fails closed for `git` subcommands only.  `the_bash_guard_refuses_writes` gives each rule of the guard one case or more, and `the_bash_guard_explains_each_refusal` checks the whole line of a refusal.

These pass the guard.  Each item gives examples only, and the list is not complete:

* `git --exec-path=`, and `git grep -O`, which starts a pager program;
* `find -exec` and `find -delete`;
* a wide search in another form, such as `cd / && find .`, `find ~/Pictures`, `find /U*`, `find /Users/lloyd/..`, `grep -r x /`, `ls -R ~` or `rg x /`.  The guard also passes the home folders of a volume, such as `find "/Volumes/Macintosh HD/Users/lloyd"`;
* `$(...)` inside double quotes, `$'...'` quoting, `$"..."` quoting such as `$"git" commit`, and an unquoted `$(...)` as an argument, such as `git -C $(pwd) stash`;
* a redirection before the command, such as `2>/dev/null git commit`;
* a command in a variable, a function or an alias, such as `function f { git commit -m x; }; f`;
* a prefix option that takes a value, such as `env -u X git commit` or `exec -a x git commit`, and `env -S "git commit -m x"`;
* an array element before the command, such as `a[0]=1 git commit -m x`, which bash runs as an assignment and a command;
* `rtk` subcommands such as `rtk run` and `rtk err`;
* `cargo new` and `cargo init`, which write a new package;
* an xtask command in a form that `names_xtask` does not match, such as `cargo run -p xtask@0.1.0`, `-p path+file:///.../xtask`, `-mxtask/Cargo.toml`, and `cd xtask` before `cargo run`.  The deny rules in `.claude/settings.json` still refuse `cargo r* publish*` (TD-28);
* other variables, such as `CARGO_*` and `RUSTC_WRAPPER`;
* interpreters, such as `perl -e` and `awk`, and wrappers other than `nice` and `sudo`, such as `caffeinate -i` and `xcrun`;
* a long option of `sudo` or `nice` that takes a value, such as `sudo --user lloyd find /`;
* brace expansion and globs, such as `{git,commit} -m x` and `/usr/bin/gi? commit`;
* `git worktree prune` and `git worktree move`, since the guard allows each `worktree` subcommand;
* zsh's `=git commit`, `noglob git commit` and `nocorrect git commit`.  The Bash tool runs the person's shell, which is zsh on macOS.  The parse follows bash and zsh only for the forms that it covers.

Bash can still write files, for example by a redirection, `sed -i`, `tee`, `rm`, `mv`, `rustfmt <file>`, or a script that a command writes and then runs.  `git worktree add` without `--detach` makes a branch; `xtask review check` sees it.  `git worktree remove --force` of a worktree outside `target/review/` can lose a person's work.

A heredoc body with an apostrophe is refused as an unclosed quote.  But a second `'` later in the command closes the quote, and the guard does not see the lines between.

The time of the parse grows with the square of the length of the command.  On 2026-10-23, 10,000 characters took 0.6 s and 50,000 characters took 9.3 s on this Mac.  With `onFailure: block` on the hook, a timeout refuses the call.

The agents in `.claude/agents/` run the guard from a hook in their frontmatter.  `the_agents_are_read_only_by_tools` keeps their tools and that hook, and fails a subfolder, a file that is not an agent, and a name that two files share.  These are still open:

* The guard does not stop `Read`, or `cat` through Bash, of the `gh` token or the cargo token.
* Claude Code does not run frontmatter hooks in a folder that a person has not trusted, or in `-p` mode.  So phase 5 passes the agents with `--agents`.
* The text of an agent is a prompt, not a control.

* Fix: `xtask review check` sees a change to the state that it records (TD-32).  Phase 5 runs the reviewers in a sandbox with no token.

### TD-35: a review runs the diff's code with the person's credentials

This entry concerns no rule.  It concerns the safety of a review round.

A review agent runs `cargo test` and `cargo build` on the diff.  `xtask review mutants` runs cargo-mutants on it.  Each of them runs the diff's `build.rs`, proc macros and tests as the person.  That code can read the `gh` token and the cargo token, and use the network.  The guard (TD-34) reads only the text of a Bash command, not what the built code does.

* Fix: phase 5 runs reviews in a sandboxed job with no token.

### TD-36: in the default permission mode, each review round has an approval that the skill forces

The `review-rounds` skill (harness phase 3, M3) runs `xtask review` with no approval only in the turn in which it loads.  The notices of the background reviewers and of the mutants run start new turns.  So in the default permission mode, `review check` of step 8 asks a person for approval once in each round.  A late `review mutants` can ask too.  `review log-check` and `review clean` of step 11 also ask once in each round.  They run after the findings file, so the counts of the round do not include them.  The baseline review runs no `xtask review` command.

Since M4, the log counts the late review commands before the findings file in their own field and cell.  Since M6, an approval is not an intervention.  Step 11 asks the person for the approvals of each round, and the log records them as cost (TD-38).  An orchestrator in a `claude -p` run, in auto mode or in `dontAsk` mode does not ask (TD-38).  It writes 0, except in a `-p` run where a host or a prompt tool answers prompts.  There it writes the approvals that the run records, or `not counted`.  So the approvals that the skill forces do not fail D4 of `plans/completed/harness-phase-3.md`.  The count of late review commands equals their approvals only in the default permission mode.

* Fix: a project allow rule for `Bash(target/debug/xtask review *)` removes the approval.  That rule widens a permission of the project, so the maintainer decides (the Decision log of `plans/completed/harness-phase-3-m3.md`).

### TD-37: a branch has one set of review rounds

This entry concerns no rule.  It concerns the log of the review rounds.

`CLAUDE.md` tells the agent to run review rounds after each change that it implements, until the stop rule of decision 0024 holds.  But the `review-rounds` skill keeps one log for each branch, `<run>/log.md`.  So a branch has one set of rounds (`plans/completed/harness-phase-3.md`, "How the work runs").  Round 1 of a second set refuses the run: "the run folder holds a log: copy it out, or remove it".  If the agent removes the log and starts again, step "After the last round" writes over the `<branch>.md` copy in the reviews folder.  In orchestrated mode, the `orchestrate` skill gives each milestone its own run and its own copy, `<branch>-m<n>.md`.  After the intent review, the orchestrator removes the run folder by hand, since `xtask clean` keeps it while the branch is checked out or has an open PR.  No doc tells the agent what to do for another branch with more than one cycle of implementation and review.

* Fix: the agent or the maintainer decides how such a branch keeps the log of each set, as the `orchestrate` skill does.

### TD-38: the review log cannot count a person's approvals

This entry concerns no rule.  It concerns the measure of D4 of `plans/completed/harness-phase-3.md`.  In the default permission mode, a command of a reviewer, a validator or the baseline can ask a person for approval.  The main agent's writes of the findings file and the log can ask too.  The main agent does not see those approvals.  So step 11 of the `review-rounds` skill asks the person for the approvals of the round and of the baseline.  In an interactive session, auto mode can also fall back to a prompt that waits for a person.  An orchestrator that runs the skill then logs 0 with the mode, and does not see such an approval.  It marks that 0 as a lower bound.  The log records the approvals as cost, not as interventions.  The counts rest on the person.  A person can miscount, or cannot tell the baseline's approvals from the round's, and then gives one total.  The maintainer reads the counts at the gate.

* Fix: count the approvals from Claude Code's own record of approvals, if one becomes available.  Until then, the person gives the counts.

### TD-39: much of xtask's WebDriver client, e2e scenarios and browser runners has no `cargo test` coverage

Much of `xtask/src/webdriver.rs`, `xtask/src/e2e.rs`, `xtask/src/web_test.rs`, `xtask/src/serve.rs` and `xtask/src/roformer_cli.rs` runs only against a live browser, a driver or the model weights (VERIFY-1).  The checker found no `cargo test` that kills these mutants.  The checker of M5 read the code on `main` and ran no mutant, so a bullet marked "(not run)" rests on that reading only.

Part B of `plans/completed/stream-from-storage-catchup.md` (2026-10-28) handled the deferred rows of M3 to M6.  `xtask/tests/e2e_cli.rs` runs `e2e smoke`, `separate`, `memory-budget`, `stored-stems`, `weights` and `old-session` with a `CHROMEDRIVER` that is not a file.  Review round 1 of M9 added `weights`.  Review round 2 of M9 added `selftest`, which stops on the missing reference files.  Review round 1 of that plan added `audio-resume`, `bench` and `playback`.  Review round 1 of M8 added `flac` and `save`.  Review round 4 of M3 of `plans/active/gpu-speed.md` added `storage` and `decode`.  Each run must fail on the driver, before a browser starts.  So a scenario body or an e2e `run` that returns `Ok(())` fails a test.  The 4-hour wait for the model is the constant `MODEL_RUN_SECS`, with a test.  The waits of `e2e decode` and the model wait of `memory-budget` use `holds_or_error`, with a test.  `roformer_refuses_bad_input_before_the_model` runs `xtask roformer`.  `StemApp::audio_stats_lines` and `StemApp::open_folder` have native tests.  What is left, each with its reason:

* the scenario bodies after the driver starts, such as `storage`, `smoke`, `bench`, `separate`, `decode`, `playback`, `old-session`, `memory-budget`, `stored-stems` and `weights`: their steps and waits need a browser (not run).  So do `restore`, `save_once` and `wait_for_offer` of `xtask/src/e2e/save.rs`, and `__e2e_progress` of `xtask/e2e/save_spy.js` (findings 2-15 and 2-17 of review round 2 of M8).  Since step 6 of M8, `click_save`, `save_playing` and the reads of the WebAssembly memory and the long tasks in `save_once` are on this list too (findings 3-12 to 3-15 of review round speed 3 of M8, 4-19 and 4-20 of round speed 4, and 5-18 and 5-19 of round speed 5: a `save_once` or a `save_playing` that returns a default survives `cargo test`).  Their pure parts, `growth`, `long_tasks`, `slice_line`, `slice_of`, `slice_check`, `read_end_line`, `read_end_of`, `read_end_check`, `mean_frame_ms`, `not_frame_bound`, `throttled`, `save_checks` and `playing_checks`, have tests.  Since M4 of `plans/completed/m4a.md`, `check_heap` is on this list too.  It is browser-only glue: it hands the result of `heap_report` to a `Page`, which only a live WebDriver session makes.  So `replace check_heap with ()` survives `cargo test` (finding 2-10 of review round 2 of M4).  `heap_report` has a test.  Also in `save_spy.js`: `__e2e_long_in_save`, `__e2e_long_calibrate`, the throttle and the long frames (`__e2e_long_frame_ms`) of the animation frames, and the heap readings at each write.  CI's run of `e2e save` reaches them.  Their pure parts are functions with tests.  CI's `browser-tests` job runs `smoke`, `decode`, `playback`, `old-session`, `audio-resume`, `storage`, `flac` and `save`.  `separate`, `memory-budget`, `stored-stems`, `bench`, `selftest`, `worker-check` and `weights` also need a GPU and the model, which CI does not have, so a person runs them.  Their checks are pure functions with unit tests (`all_from_storage`, `stored_stems_checks`, `flac_checks`, the growth checks, `old_session_checks` and others).  For example, deleting the `!` of the last `if !ok` in `memory_budget` still survives `cargo test`;
* `drop_and_open` of `xtask/src/e2e/decode.rs`: only a browser reaches it, after the driver starts, so a body that returns `Ok(())` survives `cargo test`.  CI's `browser-tests` job runs `e2e decode`, which fails without it;
* the `Page` methods, such as `errors` and `click`, which need a live browser session (not run).  Since review round 4 of M3 of `plans/active/gpu-speed.md`, `wait_for`, `idle`, `run_script`, `drop_files` and `in_page` have tests on a fake driver;
* `open_song` and `load_and_separate` of `xtask/src/e2e.rs`: only the GPU scenarios reach them, in a browser.  So an `open_song` that returns `Ok(0)` or `Ok(1)` survives `cargo test`.  So does a `load_and_separate` that returns `Ok(0)` or `Ok(1)` (findings 4-23 to 4-26 of review round 4 of M3 of `plans/active/gpu-speed.md`);
* `Driver::start` after the driver is found, `free_port`, `Session::execute` and the `set_viewport` wrapper, which need a driver (not run);
* `query`, which `parses_scenario_and_options` never calls (not run);
* `run` in `xtask/src/web_test.rs` and in `xtask/src/serve.rs`, which no test calls (not run);
* a request with no headers in `drip`, since the test always sends a Host header (not run);
* `run` in `xtask/src/roformer_cli.rs` after the model loads: the separation needs the model weights, which CI does not have.  Its refusals of bad input have a test;
* `main` in `app/src/main.rs` (native): `eframe::run_native` needs a window, and no test can open one.  Its folder argument is `StemApp::open_folder`, which has a test.  So a `main` that returns `Ok(())`, or that deletes the `viewport` field, survives `cargo test`.

PR #39's review rounds sent some mutants inside functions to TD-39.  Each one that `cargo mutants --list` still shows is an entry of `KNOWN_MISSED` (`xtask/src/review.rs`).  The mutants of the constant `WAITS_S` in `xtask/src/e2e/save.rs` are not entries, so they go to the validator.

Source: `docs/reviews/replay.md`, case `pr9`, finding 1-1 and each `pr9` mutant still on `main`.  The review logs of M3 to M6 of the branch `iphone-crashes-2026-10-25` (`docs/reviews/iphone-crashes-2026-10-25-m3.md` to `-m6.md`), with a `catch-up` row for each deferred finding.

* Fix: a fake driver for `Driver` and `Session`.  Add unit tests of the pure parts of the scenarios.  For the scenarios that need the model, a GPU runner in CI.

### TD-41: the GPU-1 layout test pairs shaders from a hand copy

`every_layout_matches_its_shaders` pairs shaders, layouts and entry names from its own hand copy, not from `Kernels::new` (GPU-1).  So a `Kernels::new` that gives a pipeline the wrong layout passes it; only wgpu's validation sees that.  Tech-debt rung 1 corrected the doc comment, which claimed that the test checks the names of `Kernels::new`.  It also added tests of the read-only storage count, the y-axis limit, and a breach of the invocation limit that only z causes.

Source: `docs/reviews/replay.md`, case `m2`, findings 1-2 and 1-4.

* Fix: take the pairs from the pipelines that the code makes, for example from a table that `Kernels::new` reads.  That changes how the kernels are built, so it needs GPU-2's checks.

### TD-43: the file picker path has no automated check

`pick_song` keeps each chosen `File` and saves it as a Blob, but no automated check runs it (VERIFY-1).  WebDriver cannot drive the browser's file picker.

Source: `docs/reviews/replay.md`, case `m18a`, finding 1-2.

* Fix: do a hand check in Chrome for each PR that changes the picker path.  Record it in the PR.  Stop when a browser test can set the files.

### TD-44: the review log's tokens are final contexts, not the tokens used

This entry concerns no rule.  It concerns the measure of D4 of `plans/completed/harness-phase-3.md`.  The `review-rounds` skill takes each agent's tokens from its `<usage>`, which is the size of the agent's final context.  The Tokens cells of the `## Agents` table and of the rounds table add these figures: "Tokens: the sum of the tokens of those agents".  D4's ratio compares round 1 with the baseline on these sums.

The final context of an agent is lower than the tokens that it used, by a ratio that differs by agent.  In M5, that ratio ran from 2.5 to 26.2 in case `m2`, and from 3.7 to 18.4 in case `m18a`.  Reviewers have high ratios, and validators low ones.  So D4's ratio of summed final contexts can misstate the ratio of tokens used by several times, by the mix of agents.

In `-p` mode, the result's `modelUsage` gives the session's tokens used.  For example, the session of `m18a` used 6,266,253 tokens, and its main agent used 758,710 of them.  The final contexts of its seven subagents sum to 391,169.  The source of these figures is `## Cost` of `docs/reviews/replay.md`.  The session's 6,266,253 comes from `modelUsage`.  The main agent's 758,710 counts its `message.usage` in the M5 transcript, once for each message ID.  The 391,169 is a sum of final contexts.  For `m2` and `m18a`, the sums of each agent's `message.usage` are about 1% below `modelUsage`.

D4's token bar has no reliable measure in the log.  The maintainer decides how to measure it at the gate.

Harness phase 3b has the same gap for its measure "cost per milestone against PR #39".  The Progress lines of `plans/completed/harness-phase-3b.md` give the final contexts of the round agents and of some other agents.  Only M5's measurement gives `modelUsage` tokens.  No log gives the tokens that the orchestrator used, or a total for the run.  So each cost per milestone in the plan is a lower bound.

* Fix: the skill and D4 stay as they are.  A later phase can read `modelUsage` where it exists.

### TD-46: in `dontAsk` mode, the agents' probe forms are denied

This entry concerns no rule.  It concerns the headless rounds of phase 5.  In `dontAsk` mode, the harness denied probe commands of the review agents in five forms.  The forms are a heredoc, a multi-line command, `cd`, an env or variable prefix, and a `>` write.  No allow list admits a heredoc, a multi-line command or a `>` write (the Decision log of `plans/completed/harness-phase-3-m5.md`).  For `cd` and a variable prefix, no run tried a wider allow list.  In M5 they were 11, 14, 14, 23 and 9 of the 126 denials of the six counted runs.  The evidence is the denial table in `## Threats` of `docs/reviews/replay.md`, and dry run 2 in the Progress of `plans/completed/harness-phase-3-m5.md`.

* Fix: phase 5 runs the agents in a sandbox where Bash needs no allow list.  Or the agents write each probe in a form that an allow rule admits, such as a one-line `sed`.

### TD-47: a `-p` session ends while a background agent runs

This entry concerns no rule.  It concerns the headless rounds of phase 5.  A `-p` session ends when the main agent ends its turn, even while a background validator runs.  In M5, case `m11` run 1 ended with "The mutants validator is running.  After it ends, I will run `review check` and write the findings file."  The validator's notice is `stopped`, and no findings file was written.  The evidence is in `## Threats` of `docs/reviews/replay.md`, and in the Progress of `plans/completed/harness-phase-3-m5.md`.

* Fix: the main agent waits for every background agent before it ends its turn.

### TD-48: a reviewer of a later round can read the messages of the fix commits

This entry concerns no rule.  It concerns the rule of harness phase 3 that a reviewer does not see the author's reasoning (`plans/active/harness-engineering.md`, phase 3).

In a round above 1, the `review-rounds` skill starts each specialist of the since route on `<since>..HEAD`.  That range holds only the fix commits of the last round.  So `git log` of the range shows the "review round N fixes" messages.  The agent files tell the reviewers not to read these messages.  But the prompt gives only the range, and the guard (TD-34) does not refuse `git log`.  In M6 review round 2, three specialists read these messages and said so in their output.

* Fix: the skill tells each reviewer to read the range with `git diff` and `git show --format=`, which show no message.  Or the guard refuses `git log` without a `--format` option.

### TD-49: a review agent can write files outside the repo

This entry concerns the read-only rule of the review agents.

The guard (TD-34) checks the text of a command, not where the command writes.  `xtask review check` sees only the state of the repo (TD-32).  So a review agent can write a file outside the repo, and no check sees it.  In M6 review round 2, two reviewers wrote copies of the guard to the scratchpad folder of the orchestrator.

* Fix: phase 5 runs the reviewers in a sandbox that can write only to their worktrees under `target/review/`.

### TD-50: a reviewer cannot run a mutant of a harness file in its worktree

This entry concerns VERIFY-1 for the files of the harness, such as the guard and the agent files.

The guard (TD-34) does not refuse an edit in a reviewer's own worktree.  But auto mode's classifier can refuse it.  In M6 review round 3, the classifier refused a reviewer's edit of `.claude/hooks/read-only-bash.sh` in its own worktree, with the reason "Self-Modification".  So in auto mode, a reviewer cannot change a harness file to show that a test fails on wrong code.  It reads the harness file and its tests only.  `xtask review mutants` does not test these files either, since cargo-mutants changes only Rust code (TD-33).

Auto mode can also refuse the worktree itself.  In M8 review round 2 of harness phase 3b, it refused the supply-chain-reviewer's `git worktree add`, with the reason "Security Weaken".  That reviewer then could run no mutant of its own, of Rust code or of a harness file, and it reasoned about its test with no run.  The other reviewers of that round made their worktrees.

* Fix: phase 5 runs the reviewers in a sandbox, in a mode without the classifier.  Until then, a person runs such a mutant by hand when a test of the harness is in doubt.  The `cargo-mutants` run of the round still tests the Rust code of the since diff.

### TD-62: the underrun count of the save during playback sees only a read-ahead that runs dry

This entry concerns RT-1 and D11 of `plans/completed/stream-from-storage-m8.md`.

`e2e save` plays the 10-minute song during a WAV save, and fails on an underrun, as `e2e playback` counts them.  That count sees only a read-ahead buffer that runs dry.  The buffer reads 16 blocks of 32,768 frames ahead, about 12 s of audio.  So a save that holds the main thread for less than about 12 s counts no underrun.  But the audio output has a smaller margin.  cpal queues two buffers of 3,072 frames, so when one ends, only one buffer of audio is queued: about 70 ms at 44.1 kHz, and 64 ms at 48 kHz.  A late audio callback is heard as a click.

Since review round speed 2, the check that fails the run is each save's longest slice.  The save's task measures each slice, and the page logs the longest at the save's end.  `e2e save` fails a save whose longest slice is over 35 ms (`SLICE_LIMIT_MS`): half the margin at 44.1 kHz, and 55 % of it at 48 kHz.  The main thread's long tasks (Chrome's `longtask` entries, over 50 ms) are logged with a calibration, not checked.  A runner that paints in software can have long tasks of its own.  Safari has no long tasks.  `a_slice_ends_within_its_budget_and_one_block`, `a_slice_grows_after_a_long_wait` and `a_slice_after_a_long_wait_for_a_write_takes_the_least` bound each slice natively.  When all the cores of a machine are busy, the system can stop the page's main thread for over 100 ms in one slice.  So only a quiet machine measures the slices (review round speed 4).  The slice check and the read-end check measure wall time, so they fail on a loaded machine.  If CI fails only those checks, read the long tasks and the load of the runner first.  Run `e2e save` again before changing the budget or the depth (finding 5-10 of review round speed 5).

A slice covers the steps of the job.  These are the mix of each block, the copy of each full write into its JS array, and the start of each write and each read.  The work that the save's promises do after a slice is not in a slice.  The end of each read copies the bytes into WebAssembly memory and decodes a span of each lane into its slot.  Since review round speed 4, the save's store measures the decode, and the page logs the longest (`longest read end`).  `e2e save` fails a save whose longest read end is over `SLICE_LIMIT_MS` (`a_waking_store_keeps_the_longest_read_end`).  Up to 2 reads can end in one task.  A slice of the save can follow them in the same task, because wasm-bindgen-futures wakes the save's task from a microtask (finding 5-6 of review round speed 5).  So one task can hold two read ends and a slice.  The copy into WebAssembly memory, the end of each write, and the end stage in the UI frame are not measured.  The end stage is the last write, the close, and the offer or the encode request.  In the saves of `e2e save` with no playback, of 3 lanes, Chrome logged no long task.  What is left: no check bounds a whole task, the copy at a read's end, the ends of the writes or the end stage.  Two read ends and a slice in one task can pass the margin of about 70 ms while each check passes.  No check sees a late audio callback that comes from outside the save.

* Fix: count the audio callbacks that start late, from the times of cpal's callbacks, in the `playback:` line, and make `e2e save` fail on one.

### TD-67: `xtask clean` sees a process only by its open files

This entry concerns no rule.  It concerns `xtask clean` (`plans/completed/harness-phase-3b-m1.md`).

`xtask clean` keeps a worktree or a scratch root that a process uses.  It reads the uses with `lsof -w -n -P -F n`: the working folder and the open files of each process.  An agent that builds a worktree from the main checkout keeps no file open in it between its commands.  So `xtask clean` does not see that agent.  Only the test of the open round keeps the worktrees of a review round that runs.  It does not keep the worktree of a review with no state record.  Without root, `lsof` does not show the processes of other users.  Without `lsof` or `gh`, `xtask clean` keeps each item that needs the missing fact, so it removes less.

* Fix: no check is planned.  Run `xtask clean --yes` between milestones, when no agent runs.

### TD-68: a stopped `xtask serve` leaves its pid file

This entry concerns no rule.  It concerns `xtask serve` and `xtask clean`.

`xtask serve` writes its pid file in `target/xtask-serve/` of the repo (`serve::pid_file`).  It uses the standard library only, so it cannot remove the file when a signal stops it.  So each stopped server leaves a stale file.  `xtask clean` lists such a file as stale, and `--yes` removes it.  A new server on the same port writes over the file.

* Fix: no fix is planned.  A signal handler needs a new crate (DEPS-1) or unsafe code.

### TD-70: tier `commit` of `xtask check` can pass where the full checks fail

This entry concerns GIT-2 and VERIFY-1, through `xtask check` (decision 0026).

Tier `commit` runs the checks that the changed paths of one commit need (`plans/completed/harness-phase-3b-m4.md`).  It has these gaps:

* The crate tests are one `cargo test` command for the chosen packages.  Cargo turns on features only across those packages, so the command can pass where the workspace test line fails.
* A test that reads a file in another folder is not seen.  The one case today is `engine/tests/realtime.rs`, which reads `app/Cargo.toml`.  That path matches DEPS-1, so it runs tier `push`.
* The strict paths (`STRICT_PATHS` in `xtask/src/check.rs`) are a narrow list that a person keeps.  `app/src/lib.rs` is in no row, but it holds `MEMORY_PATH_MAX_FRAMES` and some transfer code.
* Tier `commit` skips the GPU test line and the Trunk build.  A strict path, or a graph that fails, sends the commit to tier `push` instead.  Each tier skips the browser tests.
* Tier `push` skips the GPU test line when its paths change no GPU code: nothing in `roformer/`, `roformer-gpu/` or `test-support/`, and no workspace file.  When no path changed, or no path is known, it runs the line.  CI does not run that line either.
* The GPU test line has `STEMYLA_REQUIRE=gpu,f16`, so it fails on an adapter without `SHADER_F16_IN_F32`.  On such an adapter, run `check` with `STEMYLA_REQUIRE=gpu`.  A `STEMYLA_REQUIRE` in the environment replaces `gpu,f16` in the line when one of its words is `gpu`, `f16` or `timestamps`.  Another value, such as `fixtures`, keeps `gpu,f16`, so the line cannot pass with no adapter.  The printed reason says which of the two occurs.
* Each tier runs its checks on the working tree, not on the index or `HEAD`.  So an unstaged edit or an untracked file that the commit leaves out can make the checks pass where the commit fails.  A staged change that the tree undoes, or a committed change that the tree reverts, is not tested.  The paths of such a change still choose the checks.  Tier `push` has the same gap.  Two checks narrow it, but neither tests each commit.  `xtask review start` refuses a tree with changes, so a review round reviews the commit.  CI runs only on a pull request, on the branch merged into `main`, and it does not run the GPU test line.

Tier `push`, which runs before each push and each review round, or CI finds the other gaps of tier `commit` before review.  No automated check runs the GPU test line when tier `push` skips it.

* Fix: no fix is planned.  Add a strict path when a rule's code moves to a new file.

### TD-71: the measurement of the agents' models has gaps

This entry concerns no rule.  It concerns decision 0025, and the pin `the_agent_models_are_the_measured_ones` in `xtask/tests/rules.rs`.

M5 of harness phase 3b compared cheaper models with a control for the validator and the docs-reviewer, on logged findings.  The results are in the Surprises of `plans/completed/harness-phase-3b-m5.md` ("step 4, the results").  No role passed, so each agent file keeps `model: inherit` with no `effort` key.  The measurement has these gaps:

* Each arm had one run on each case.  So the measure does not separate run noise from the model's effect.
* The team leader rebuilt the finding block of each validator run from its row in the log.  So the block had no evidence and no fix.
* No field of the `claude -p` stream shows the effort of a subagent.  So no check showed that `effort: high` took effect in a candidate arm.
* The `haiku` validator stopped after 3 of its 6 cases, for its cost.  So its result has 25 items, and item 0 of the pass rule cannot hold; its agreement interval is 0.363 wide.
* The measurement did not reach the gpu-reviewer, the wire-reviewer and the supply-chain-reviewer.  They keep `inherit` because there is no evidence, not because they failed.
* The measure is recall, or agreement with the log, on logged findings only.  It does not measure precision.

* Fix: decision 0025 runs the measurement again when the model line-up changes (phase 6).  Until then, the pin keeps each role on `inherit`.

### TD-72: no run shows that a big task asks its decisions up front

This entry concerns no rule.  It concerns decision 0023, the "Plans:" line of `CLAUDE.md`, the section "Decisions for the maintainer" of `docs/plans/PLANS.md` and the recipe "Run a big task hands-off" in `docs/working-with-agents.md`.

The `orchestrate` skill says that the agent sends one message up front, before any work.  The message proposes the mode and lists each choice that hinges on the maintainer, with its options and a recommendation.  M6 of harness phase 3b ran two `claude -p` dry runs on a plan of 4 milestones (`plans/completed/harness-phase-3b-m6.md`, step 5).  Each session proposed the mode in its first reply.  Neither sent the decisions as that message, since the `dontAsk` mode refused the Bash that the preflight needs, and the skill stops at a failed check.  Auto mode refused a run with Bash allowed.  So no run shows the message.  Also, when there is no open choice and the effort is known, the message holds only the mode, the branch and the PR.  If the maintainer also asked for the mode, the agent sends no message.

* Fix: a person starts a session in auto mode with a big plan and the prompt of the recipe, "Do docs/plans/active/<plan>.md.", and records the first reply.  The plan has at least one open entry in "Decisions for the maintainer", and the prompt does not ask for the mode.  It passes when the reply lists the plan's decisions, each with its options and a recommendation.  A reply that only proposes the mode does not pass.

### TD-73: from round 4, a reviewer reads only the fix range

This entry concerns no rule.  It concerns decision 0024, and D4 of `plans/completed/harness-phase-3b.md`.

From round 4, the `review-rounds` skill limits each reviewer with a `Scope:` line.  The scope is the diff of the fix range, and the whole of each file that it touches.  A bug in another file of the branch is out of scope.  In the 14 older review logs, rounds 4 and later found 13 bugs.  At least 3 of them were in files that the fix range of the round before did not touch.  They are catch-up 4-1, M7 6-1 and M7 6-2, all in PR #39.  Earlier rounds read the whole branch, and did not find them.  With the late range, no later round reads those files again.

In harness phase 3b, the 15 late rounds of M3, M4, M6 and M7 found 3 bugs: M7 4-1, 4-7 and 5-2, in `xtask/src/attention.rs`.  M1's late bugs 5-1 and 6-1 were in `xtask/src/clean.rs`.  The fix range of the round before touched each of these files.  In M4, two reviewers of round 4 noted a stale GIT-2 sentence out of scope, and the orchestrator put its fix in round 5's range (`ef602b3`).  No rule tells a reviewer to report such a finding, or a team leader to log it.

* Fix: the maintainer decides on the proposal of the close-out of harness phase 3b, in `plans/active/harness-engineering.md` ("Proposals of harness phase 3b").

### TD-74: a team leader cannot write the review files or stop its subagents

This entry concerns no rule.  It concerns the `orchestrate` skill and the `review-rounds` skill.

In orchestrated mode, a team leader is a subagent.  From M1 of harness phase 3b, the Write tool refused `<run>/<round>/findings.md` and `<run>/log.md` to the team leader.  So in each review round of M1 to M7, the orchestrator wrote them.  So the team leader gives the text of both files at a checkpoint, and the orchestrator writes them and runs `review log-check` and `review clean`.  Each round waits for the orchestrator.  A team leader also cannot stop a stalled subagent.  In M1, a docs-reviewer stalled for about 45 minutes on one Bash call, and only the orchestrator could stop it, with TaskStop.  So the skill calls a subagent stalled after 30 minutes with no result, and the orchestrator stops it.

* Fix: no fix is planned.  If Claude Code lets a subagent write these files and stop its own subagents, remove the checkpoint of each round from the `orchestrate` skill.

### TD-75: the phase 3 gate compares one run of each review

This entry concerns no rule.  It concerns the gate of phase 3 in `plans/active/harness-engineering.md`, and the `--baseline` flag of the `review-rounds` skill.

The gate compares one run of round 1 with one run of the `baseline` agent on the same commit: "of y baseline findings, the round found x".  M5 of harness phase 3b found that one control run of the docs-reviewer found 0.425 [0.285, 0.578] of that agent's own confirmed findings in the logs.  So two runs of one review find different sets, and a miss of a baseline finding in one run can be run noise.  The record of round 1 with `--baseline`: PR #36 found 4 of 4, PR #37 2 of 2, PR #39 (M3) 1 of 3 and PR #41 (M1) 2 of 4.  Together that is 9 of 13, 0.692 [0.424, 0.873].  The gate has no count of the round's findings that the baseline did not find.  D4's token bar has no reliable measure (TD-44).

* Fix: the maintainer decides on the proposal of the close-out of harness phase 3b, in `plans/active/harness-engineering.md` ("Proposals of harness phase 3b").

### TD-76: `xtask attention` finds a hand-run line by the name of its item

This entry concerns no rule.  It concerns the hand-run class of `xtask attention` (decision 0027), which reaches PROTO-1 only through TD-16.

`xtask attention` lists first one line of each hunk that changes a message that `stored-stems`, `worker-check` or `weights` writes by hand (TD-16).  In `xtask/src/e2e.rs`, it finds such a hunk by the name of its item.  The name starts with `stored_stems` or `worker_check`, in upper or lower case, or it is `stem_protocol_file`.  A helper with another name that writes such a message gives no hand-run line.  Its change then comes later in the list, or not at all, and the orchestrator can leave it out of the 10 comments.  M7's rounds found one more helper in each round.  M7 then wrote the rule as two name prefixes and one name (`plans/completed/harness-phase-3b-m7.md`, Decision log).

* Fix: when a new helper writes a hand-written message, add its name to the rule, with a test.  The wire-reviewer reads each change to the e2e scripts.

### TD-77: `xtask pr-body` checks the form of a body, not what "Needs your attention" holds

This entry concerns no rule.  It concerns decision 0027, and `xtask pr-body`.

`xtask pr-body` checks the headings, the length of the summary, the lines of Verification and the `<details>` block.  It does not check what "Needs your attention" holds.  So an item for the maintainer to decide or to do can be missing.  An item there can also have no link to its inline comment.  The body passes.

* Fix: no check is planned.  The orchestrate skill says what the section holds, and the maintainer reads it.

### TD-78: the `orchestrate` skill has no pause or resume step

This entry concerns no rule.  It concerns decision 0023, and the `orchestrate` skill.

The skill's "Stop and wait" lists the reasons to stop, and a request of the maintainer to pause is not one of them.  The skill has no step for a resume.  Its "Start" makes the branch and the draft PR, commits the plan and sends the message up front.  On 2026-11-01, harness phase 3b paused after a review round of M6, with the steps of the resume in the plan's Progress (`d376200`).  The resume followed those steps, not the skill.

* Fix: the maintainer decides on P5 of the close-out of harness phase 3b, in `plans/active/harness-engineering.md` ("Proposals of harness phase 3b").  Until then, the guide's recipe "Run a big task hands-off" says how to pause and resume.

### TD-92: the AAC delay assumes a decoder that trims nothing

This entry concerns VERIFY-1.

Chromium on Linux has no AAC encoder for WebCodecs ([Chromium issue 40942807](https://issues.chromium.org/issues/40942807)).  So CI's `browser-tests` job tests the AAC encode of a save only with the fake encoder of `worker/src/web/tests/aac.rs`.  CI's `macos` job, on `macos-latest`, runs `a_real_aac_decodes_to_the_source` in Chrome with `STEMYLA_REQUIRE=aac`, so a browser with no encoder fails it.  It also runs `e2e save` in Chrome and in Safari, which save AAC there (`aac_rule`).  The job is not a required check at first (Q6 of `plans/active/tech-debt-rungs.md`).

The worker's probe measures the encoder's delay through `AudioDecoder`, so it assumes that `AudioDecoder` trims no frames.  The W3C AAC registration says that it trims none for a plain AAC stream ([W3C, AAC WebCodecs Registration](https://www.w3.org/TR/2026/DNOTE-webcodecs-aac-codec-registration-20260924/)).  Only macOS shows this, in Chrome 157 and Safari 27.2.  A decoder that trims `t` frames would give a delay `t` frames too small.  The file's `elst` and `iTunSMPB` would then start `t` frames early, in the silent lead-in.  So the source would start `t` frames late and lose its last `t` frames.

* Fix: a test that measures the trim of each browser's `AudioDecoder`, in the `macos` job.  The maintainer decides when the `macos` job becomes a required check.

### TD-93: a Safari `web-test` turns off the debug glue of the stem worker

This entry concerns VERIFY-1.

wgpu 30.0.1 reads `GPUAdapterInfo.subgroupMinSize` as a number, and Safari 27.2 gives `undefined` (`references/browser-quirks.md`).  The debug glue of `wasm-bindgen-test-runner` asserts a number.  So `an_f16_variant_the_browser_rejects_keeps_f32_weights` and `a_read_back_is_copied_into_wasm_memory_once` (`worker/src/web/tests.rs`) failed in a Safari `web-test`.  Since M2 of `plans/active/tech-debt-rungs.md`, `web-test` sets `WASM_BINDGEN_NO_DEBUG=1` for the crate run of `stem-worker` in Safari only, and prints why (`run_vars` in `xtask/src/web_test.rs`).  Both tests then pass in Safari 27.2 (2026-11-11).  The variable turns off each debug assert of that crate run, not only this one.

The release build of the stem worker has no debug glue.  On 2026-11-11, `xtask e2e selftest --browser safari` with the f16 weights passed in Safari 27.2 on an Apple M2 Pro.  The worker opened the GPU, and the worst stage was 69.4 dB.  So a user of Safari 27.2 gets a working GPU.

* Fix: a wgpu release that reads `subgroupMinSize` as optional.  Then remove `NO_DEBUG` from `run_vars`.  The PR body of M2 holds a draft wgpu issue for the maintainer, near [gfx-rs/wgpu#10882](https://github.com/gfx-rs/wgpu/issues/10882).

### TD-94: Chrome on Windows has no AAC encoder at 256 kbit/s

This entry concerns no rule.  It concerns M4 of `plans/completed/m4a.md`.

The save asks for 256,000 bit/s (`stem_protocol::AAC_ENCODER`).  Chromium's Windows encoder takes only 96,000, 128,000, 160,000 and 192,000 bit/s (`kSupportedBitrates` in [Chromium's Windows audio encoder](https://chromium.googlesource.com/chromium/src/+/refs/heads/main/media/gpu/windows/mf_audio_encoder.cc)).  Its `ValidateInputOptions` gives `kEncoderUnsupportedConfig` for each other rate.  So Chrome and Edge on Windows probably fail the page's probe, and the Save menu offers no AAC item.  No user gets a wrong file or a wrong label.  Nobody checked this on Windows.

* Fix: the maintainer decides.  One option is a fallback rate of 192 kbit/s with its own label.  Another is no AAC on Windows, as today.

### TD-96: the Safari tests depend on the Mac's session and on WebKit

This entry concerns VERIFY-1.  The causes are outside the repo (step 9 of `plans/completed/tech-debt-rungs-m2.md`, Q8 of `plans/active/tech-debt-rungs.md`).

* A locked screen: `safaridriver` then starts no session, and gives "The session timed out while connecting to a Safari instance".  `web-test` and `e2e` with Safari on macOS read `CGSSessionScreenIsLocked` from `ioreg` first, and fail at once with a clear error (`refuse_a_locked_screen` in `xtask/src/drivers.rs`).  Fix: none in view; a person unlocks the screen.
* An idle user: Safari keeps each `AudioContext` suspended while macOS reports the user idle (`references/browser-quirks.md`).  `web-test` and `e2e` with Safari hold `caffeinate -u` for their run (`keep_user_active`).  Nobody saw the fix with an idle user, since `caffeinate -dimsu` ran for the whole of M2.  Fix: a run with no other `caffeinate`, with the user idle.
* A stale session: a `safaridriver` whose parent is gone keeps its session paired with Safari.  `web-test` and `e2e` end such a driver before each crate run or session start.  Only a session start is tried once more: `e2e` in `start_session`, and `web-test` when `session_did_not_start` holds.  Tests cover the choice and the order of the tries with fakes: `start_plan` and `start_with` in `xtask/src/webdriver.rs`, and `run_crate` in `xtask/src/web_test.rs`.  `end_stale_safari_sessions` has a test of the kill of each stale driver.  No run of M2 left a stale driver, so nobody saw the second try with a real stale session.  Fix: a Safari run after a stale driver, which shows the second try.
* Safari's autoplay rule in `web-test`: Safari 27.2 keeps each audio context of the test page on `http://127.0.0.1` suspended.  `wasm-bindgen-test-runner` 0.2.130 sends empty capabilities to `safaridriver`, and reads `WASM_BINDGEN_TEST_WEBDRIVER_JSON` for the other drivers only (`new_session` in `headless.rs`, the same on `main` on 2026-11-11).  So `web-test` cannot set `webkit:alwaysAllowAutoplay` (`man safaridriver`).  In a probe session with that capability, the context ran after 35 ms.  With no capability, it stayed suspended, also after `resume()` (intent review I1-3 of M2).  So the 3 busy-page tests of `app/tests/web_output.rs` and 2 tests of `app/src/streamtest.rs` skip in Safari on the maintainer's Mac.  The log of CI's `macos` job cannot show whether they skip there.  Before each skip, the test checks that a new audio context of the page stays suspended, and fails if it runs (`held_back_by_the_browser`, `skip_as_the_browser_holds_audio_back`).  By default, the runner prints the console of a test only when the test fails, and `web-test` does not pass `--nocapture`.  So a CI log shows no skip line (CI run 42991255256).  A test that passes on the audio path gives the same log.  `web-test` refuses `-- --nocapture`.  This command shows each skip line: `CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner SAFARIDRIVER=/usr/bin/safaridriver WASM_BINDGEN_TEST_TIMEOUT=600 cargo test --target wasm32-unknown-unknown -p app --lib --test web_output -- --nocapture`.  It sets the runner, the driver and the time limit of each test binary as `web-test` does; the runner's default limit of 20 s stops the app's lib tests.  Fix: a runner that sends those capabilities to `safaridriver`, by a change to wasm-bindgen or a proxy in `xtask`.  Then `web-test` gives Safari `webkit:alwaysAllowAutoplay`, and the tests check the audio clock in Safari too.
* A hidden test page: on 2026-11-11, 2 Safari 27.2 runs of `app --lib` stopped with no failure, while a person used the Mac.  One stopped after 182 tests, in `playback_tests::the_page_reads_a_lane_one_block_at_a_time`.  The other stopped after 82 tests, in `save_tests::a_stopped_worker_leaves_no_save_files_and_takes_no_encode`.  Safari's unified log shows the cause.  Before each stop, the test page became hidden ("view visibility state changed 1 -> 0"), and stayed hidden.  WebKit then gave its process a background assertion only, and later tried to suspend it.  In the second run, a click in the automation window showed the "Automation Interruption" alert.  The runner's limit of 600 s ended the first run.  One run passed with the page hidden at times, so a hidden page does not always stop a run.  The logs are in `m2/i1-3/` (intent review I1-3 of M2).  Fix: none in the repo.  Do not use the Mac during a Safari run.
* WebKit bug 246069 ([Use AbortSignal.reason in Fetch code](https://bugs.webkit.org/show_bug.cgi?id=246069), NEW on 2026-11-11): Safari 27.2 rejects an aborted fetch with its own `AbortError`.  The worker gives the abort reason in its place (`download_to_opfs` in `worker/src/web.rs`).  Fix: remove the workaround when WebKit gives the reason.

### TD-98: CI runs the `web-test` steps of the macOS job again after the runner's hang

This entry concerns VERIFY-1.  It is a workaround: a step that runs again can hide a hang that comes and goes.

* The cause: on the `macos-latest` runner, `wasm-bindgen-test-runner` sometimes waits its 600 s limit for a test page that never reports.  It then prints "Failed to detect test as having been run. It might have timed out." (`headless.rs` of wasm-bindgen-cli 0.2.130).  Why the page does not report is not known.  It can be the hidden test page of [TD-96](#td-96-the-safari-tests-depend-on-the-macs-session-and-on-webkit), which also stopped Safari runs with no failure.
* The failures on 2026-11-12, on branch `gpu-speed/2026-11-12`: 4 run attempts in 3 runs.
  * Run 43152005940, Safari: `app` `web_close` hung.  In the same run, `stem-worker` failed `a_mix_file_changed_after_the_open_gives_no_old_samples` ("shortened: the read gave the old samples"), with no hang ([TD-99](#td-99-a-stem-worker-test-read-old-samples-once-in-safari-on-ci)).
  * Run 43223719719, Safari, `engine --lib`: a WebDriver `execute/sync` request failed, with no hang.
  * Run 43229036261, run attempt 1, Safari: `app` `web_output` hung.
  * Run 43229036261, run attempt 2, Chrome: `stem-worker` `a_real_aac_decodes_to_the_source` hung.  The step took 10.5 minutes; the test takes 1.38 s on the maintainer's Mac.
* The branch did not change the failed tests.  It did change `stem-worker` (`worker/src/web.rs`, `worker/src/web/tests.rs`), the crate of the AAC test and of `a_mix_file_changed_after_the_open_gives_no_old_samples`.  Each failed test passed on the maintainer's Mac.
* The rates: from run 42952206488 (2026-11-11) to run 43229036261 (2026-11-12), 3 of 34 Safari `web-test` steps failed (8.8 %).  1 of 22 Chrome AAC steps failed (4.5 %).  Only 2 of the Safari failures were hangs.  The cancelled steps are not counted.  Source: the jobs of each run of `ci.yml` in that range (`gh api .../attempts/<n>/jobs`, read on 2026-11-13).  The attempts of a step run on one runner, so they may not fail independently.  So no rate is given for 3 attempts.
* The workaround: `.github/workflows/ci.yml` runs the Safari `web-test` and the Chrome AAC step through `nick-fields/retry` v4.0.0, pinned by its SHA.  `web-test` exits with 75 when each failed crate failed only by the hang (`exit_code` in `xtask/src/web_test.rs`).  Each other failure exits with 1.  Each step runs again, up to 3 attempts, only on exit code 75 (`retry_on_exit_code`).  So a failed test, a panic or a failed WebDriver request fails the step at once.  Of the 4 failed run attempts above, the action runs again only after the 2 hangs of run 43229036261.  It does not cover run 43152005940, where `stem-worker` also failed, or the `execute/sync` failure of run 43223719719.  An attempt that reaches its `timeout_minutes` is not run again.  Each time the action runs a step again, it shows a warning in the run.  The job's limit grew from 180 to 285 minutes for the attempts.
* Who looks: the person who reviews a PR.  Each "Attempt <n> failed" warning of a step of the `macos` job is a hang.  The attempt's log names the crate that hung.  If the PR changes that crate, the person runs `cargo run -p xtask -- web-test --browser <browser>` on a Mac before the merge.  It must pass.
* Fix: find why the test page does not report on the runner, then remove the action.  `web-test` could also run only the crate that hung again, in place of the whole step.

### TD-99: a stem-worker test read old samples once in Safari on CI

This entry concerns VERIFY-1.  The cause is not known.

* The failure: CI run 43152005940, job 130963127170 (`browser tests (macOS)`), commit `c42c98ca`, image `macos-26-arm64` with Safari 27.2.  `a_mix_file_changed_after_the_open_gives_no_old_samples` in `worker/src/web/tests.rs` failed with no hang.  The panic: `worker/src/web/tests.rs:3576:24: shortened: the read gave the old samples`.
* What it shows: the test writes 16 zero bytes over the mix after `StoredMix::open`, and awaits `close()`.  The read through the `File` from before the write then gave the old samples of channel 0.  WebKit's source on `main` gives the new bytes or NotFoundError after `close()` resolves.  No path to old bytes was found.
* The rate: 1 of 37 macOS job attempts that ran the test (2.7 %), from run 42952206488 to run 43241235485.  In Chrome, 0 of 59 job attempts from 2026-11-09.  On the maintainer's Mac, 0 of 600 reads gave old bytes.
* TD-98 does not cover it: the action runs a step again only after the runner's hang, and this failure exits with 1.  So it fails CI if it comes back.
* The product risk is low.  The page never writes the mix during a separation (D6 of `plans/completed/stream-from-storage-m4.md`), and the end check of `StoredMix` sees a change of size ([TD-55](#td-55-safari-can-miss-a-mix-file-written-again-during-a-separation)).
* Next step: in the test, after the change, read the mix again through a new `getFile()` before the old `File`.  Assert that the new read sees the change.  If that assert fails on CI, the write had not landed, and the test can wait for it.  If it passes and the old `File` still gives old samples, the cause is in WebKit, and it goes under TD-96.
* Fix: find the cause, then fix the test or the code, or record a WebKit bug.

### TD-100: on CI, the step limit ends `e2e save` before the scenario's limit

This entry concerns no rule.  It concerns the time limit of each browser e2e scenario (the maintainer's request of 2026-11-13).

* `e2e save` has a time limit of 60 minutes (`save::STEP_LIMIT_S`), from when xtask reads its options.  CI has three `e2e save` steps, each with `timeout-minutes: 60` from the start of the step.  The `browser-tests` job has one step.  The `macos` job has one step for Safari and one for Chrome.
* Each step starts before `cargo` builds xtask.  So on CI, the step limit always ends `save` first, and the run writes no limit line.
* The risk is low.  Each wait of `save` has its own end.  CI discards a hosted runner after the job, with its browser and driver.
* Fix: give each of the three steps a limit of 65 minutes, above the scenario's.  Then raise the job limits so that they still hold the steps: `browser-tests` to 90 minutes (now 85), and `macos` to 295 (now 285).  Change `the_ci_step_limit_is_the_waits_limit` and `the_ci_job_limit_holds_its_save_steps` in `xtask/src/e2e/save.rs` with them.

### TD-102: some mutants of calibration's timing have no test

This entry concerns VERIFY-1.  Review round 1 of M5 of `plans/active/gpu-speed.md` left these mutants of `roformer-gpu/src/bench.rs` alive.  Each is older than M5.

* `time_one` replaced with a constant: no test times a matmul shape with a known clock.
* `calibrate`: `==` to `!=` in the format of its matmul buffers, `rd == 1` to `rd != 1`, and `+=` or `*` in its matmul totals.  No test reads calibration's matmul totals.
* `measure_attention` and `fused_attention` replaced with `()`: no test reads `?bench`'s attention lines, or what `fused_attention` records.
* Tests since that round:
  * `the_time_axis_is_timed_as_the_separation_runs_it`, `a_chunk_total_sums_each_call_times_its_count` and `each_timing_goes_to_its_policy` (no GPU): the plan of the time axis, its totals, and which policy each timing feeds.
  * `the_time_axis_adds_up_one_wait_for_each_step` (`gpu` in `STEMYLA_REQUIRE`): `time_rec` with a wait after each step, and the totals of each usable policy, with a fake clock.
  * `calibration_records_the_step_of_the_separation`: what a time-axis step records, and which fused kernel it runs, by its profile labels.  It runs on an adapter with `timestamp-query`.  On an adapter without it, it skips with a pass, unless `timestamps` is in `STEMYLA_REQUIRE`.  By default, `check --tier push` and the command of GPU-2 do not set `timestamps` (TD-8).
* Fix: a GPU test of `calibrate` and `measure_attention` with a clock that moves a fixed step at each read, and a captured log, as the time axis's test does.  A profile-label test of `attention_per_chunk`, as the time-axis step's test does.

## Known breaches

### TD-52: the FLAC encoder writes a wrong minimum block size, and no 24 bits

This entry concerns DEPS-1.  It is a workaround of a defect in a dependency.  CLAUDE.md asks to record each workaround.  It breaks no rule.

`flacenc` 0.5.1 gives the short last block as the minimum block size in STREAMINFO (`update_frame_info`).  RFC 9639 section 8.2 excludes the last block.  Symphonia then reads the stream as variable-blocksize, and refuses every frame.  So five places set the block sizes again after `flacenc`: `flac_into` in `worker/src/web/encode.rs`, `compressed` in `engine/src/test_flac.rs`, `flac_16` in `xtask/src/e2e.rs`, `flac` in `app/tests/web.rs` and `flac` in `decoder/src/web/tests.rs`.  `flacenc` also has no 5-bit Rice parameters and no escape codes, so it cannot write 24 bits; a saved FLAC has 16 (`plans/completed/flac.md`, Surprises).

* Fix: remove the overrides when `flacenc` writes the minimum as RFC 9639 says.  For 24 bits, use an encoder that can write them, such as one in this repo.  Until then, `a_flac_decodes_to_the_stem` fails if the worker's override goes.

### TD-53: parts of the page's save and storage paths have no test

This entry concerns VERIFY-1.

On the memory path of a save, the worker's MP3, FLAC, ALAC M4A and AAC M4A reach the page with their bytes or `Blob`.  They come as `Event::Mp3`, `Event::Flac`, `Event::Alac` and `Event::Aac`.  The arms of `App` that handle them call `store::download` and `store::download_blob`, which make an object URL and click a link.  No test reaches these two arms.  The separator tests check that each event keeps the worker's buffer or `Blob` (`an_mp3_reply_reaches_the_page_as_sent`, `a_flac_reply_reaches_the_page_as_sent`, `an_alac_reply_reaches_the_page_as_sent`, `an_aac_reply_reaches_the_page_as_sent`).  `e2e flac` checks the worker's file.  The "FLAC encoding" and "MP3 encoding" error labels have no test either.  Only the "M4A encoding" and "AAC encoding" labels have one (`an_m4a_encode_error_names_m4a_encoding`, `an_aac_encode_error_names_aac_encoding`).  `save_menu` has no test of its buttons.  Its items come from `save_formats`, which `the_save_menu_offers_aac_after_a_probe_that_passes` tests.  `a_stopped_worker_leaves_no_save_files_and_takes_no_encode` checks `can_encode`, which it asks.

The saves of M8 have browser tests in `app/src/save_tests.rs`, with a spy on the download link and on the worker's requests.  They test the WAV offer of the storage path, and the stored FLAC, M4A, MP3 and AAC requests and their replies.  They test the memory path when storage fails: the WAV offer, and the FLAC, M4A, MP3 and AAC requests (`a_memory_save_posts_the_arrays_of_its_format`).  They also test a dropped save, Reset during a save, a failed encode and a stopped worker.  The offer of a save file waits for `getFile()`, so the status line says "Saved" before the click.  If the offer then fails, the page logs an error and removes the file, but the status line still says "Saved".  No test makes an offer fail.

`load_song` drops the mix lane's follower and the spectrogram job before it makes the new lanes, so an old job does not keep the old song's samples (MEM-4).  That order matters only on the web, where a job can run over several frames, and no test checks it.  `store::usage` has no test.  Source: `plans/completed/flac.md`, review rounds 1 to 3 (findings 1-9, 2-2, 3-5 and 3-6), `plans/completed/mix-download.md` (M3 and review round 3), and step 3 of `plans/completed/stream-from-storage-m8.md`.

* Fix: browser tests that deliver an MP3, a FLAC and an ALAC reply of the memory path to `App`, and spy on the link.  A status line that the failed offer of a save file updates, through the page's inbox.  A test of the peak memory of `load_song` while a mix job runs.  A test of `usage`.

### TD-54: memory grows with the song, so a long song can stop the tab on an iPhone

This entry concerns MEM-1 and MEM-4.

This paragraph gives the state before M3.  On 2026-10-25, a song of about 5.1 minutes stopped the tab on an iPhone 17 Pro at chunk 2 of 48.  iOS logged `exceeded mem limit: ActiveHard 2048 MB` for the WebContent process.  The page makes all six song-length stems when the first segment arrives (`Assembly::place`), about 650 MB for that song.  The page's song and mix, the worker's mix and the stems grow by about 190 MB per minute of song.  The fixed part is at most about 1720 MB, so at most about 1.7 minutes fit.  The 30-second demo song separates.  Source: `plans/completed/stream-from-storage.md` (Context).

M3 of that plan (2026-10-25) removed the page's song-length stems during a storage-path separation.  The worker writes the stems to the session's files, and `Assembly` stays empty.  What is left:

* M4 (2026-10-26) removed the worker's copy of the whole mix on the storage path.  The page writes the mix to a session file in blocks.  The worker reads it one chunk at a time.  Since M5 (2026-10-26), the decode worker decodes a stored song in blocks into session files, so the decode holds no song.  Since M6 (2026-10-27), the page plays from browser storage each lane that has a complete session file, through a read-ahead buffer of at most 32 MiB (`engine::readahead`, `app::playback`).
* Since M7 (2026-10-27), the page reads no session file whole to show or play a stored song (`plans/completed/stream-from-storage-m7.md`).  Each lane is its session file with its overview.  The mix lane's waveform, the spectrograms, the saves, the mix write and the memory path's input read the lanes in blocks, and classic drums of a stored song runs in the decode worker.  `e2e old-session` checks that the page's WebAssembly memory after a restore stays under 64 MB and does not grow with the song.  What is left:
  * Since M8, a save writes its file to browser storage in blocks, and the stem worker encodes FLAC and MP3 from it in blocks (decision 0021).  Since M3 of `plans/completed/m4a.md`, it encodes ALAC from it in blocks too, and since M4 AAC.  `e2e save` measured on 2026-10-29 in Chrome, with a session of a source and two stems: the page's JS heap during a save rose by 1.35 to 1.45 MB for every format, for 1 and for 10 minutes, in one run.  A second run gave up to 2.23 MB.  The build before M8 (`c3952ad`) rose by 11.0 and 106.2 MB for a WAV, and by 21.5 and 212.0 MB for a FLAC or an MP3.  A FLAC encode grew the stem worker's WebAssembly memory by 0.52 MB for 1 minute and 0.46 MB for 10 minutes.  Since D11 of M8 (step 6, 2026-10-29), a reading also starts at each write of the page, and waits for the finalizers as the others do.  In 6 runs of review round speed 2, the JS heap rose by 1.34 to 2.29 MB in a 1-minute save and by 1.73 to 2.52 MB in a 10-minute save.  The page's WebAssembly memory grew by 3.5 to 3.9 MB in each save, and by at most 0.32 MiB more for 10 minutes than for 1.  Since review round speed 4, a save's reader reads 2 spans of 65,536 frames ahead.  The page's WebAssembly memory grew by 7.21 to 7.60 MB in each save of 3 lanes, and at most 0.33 MB more for 10 minutes than for 1.  With readings at 4 writes spread over each save, the JS heap rose by 3.4 to 5.8 MB, and at most 2.36 MB more for 10 minutes.  Since review round speed 5, the readings come at each write of a save and at its end.  Each save's measure must be within a fixed budget of 7.77 MB.  In 4 runs of that round, the JS heap rose by 2.38 to 3.96 MB, and at most 1.57 MB more for 10 minutes than for 1.  In 24 saves of review round speed 6, the JS heap measure was at most 4.52 MB.  `docs/ARCHITECTURE.md` (memory on phones) names each part of a save's memory.  On the memory path of a save, when storage fails, a song of at most `app::MEMORY_PATH_MAX_FRAMES` frames still saves through song-length JS arrays.  An MP3 save holds its frames in the stem worker (TD-59).
  * `e2e memory-budget` asserts a tab total during a separation, with f16 weights (M1 of `plans/active/gpu-speed.md`).  It runs with the model, not in CI.  The total has no JS heap and no WebGL textures.  There is no iPhone evidence.
  * An open that decodes on the page holds at most the memory path's limit for its lanes (decision 0020).  A song of many sources still decodes on the page (TD-58).
* The memory path still makes six song-length stems.  It takes a song without a session, or a song when storage has too little free space.  It also takes a song when storage refused the mix or the stems, or the worker cannot read the mix file.  For a separation, and for an open that decodes on the page, the page refuses a song longer than the memory path's limit for its lanes: 100 seconds (`app::MEMORY_PATH_MAX_FRAMES`) for one source, and less for more lanes (`app::memory_path_frames`, decision 0020).
* Between M3 and M7, no build went to the published site, because the page read a stored song back whole.  Since M7 that read-back is gone, so publishing no longer has that risk.  What is left are the memory costs of a save and of an open that decodes on the page.  Publishing is a person's step (GIT-1).

* 2026-10-30, the maintainer's runs: on an iPhone, a 5-minute song separated and saved an MP3 mix.  In Safari on the Mac, songs of 6.5 and 3.1 minutes separated with GPU buffers live at 630 MB on every chunk and the page at 49 to 53 MB.  The plan closed without its 30-minute iPhone run.
* Fix: a 30-minute song on an iPhone, with Web Inspector's peaks.  TD-11's assertion at the default chunk is done (`e2e memory-budget`, M1 of `plans/active/gpu-speed.md`).  `plans/completed/stream-from-storage.md` moved the song, the mix and the stems to browser storage (decision 0015).

### TD-55: Safari can miss a mix file written again during a separation

The page never writes `mix.f32` while a separation runs (D6 of `plans/completed/stream-from-storage-m4.md`).  The worker checks this too.  After the last chunk, it compares the file's size and `lastModified` with their values at the open.  Chrome also refuses any read after a write.  Safari reads the bytes on disk now, and gives `lastModified` in whole seconds (`references/browser-quirks.md`).  So in Safari, a write of the same size in the same second as the open is not seen.  The stems then come from mixed samples.

Source: review round 3 of M4 on the branch `iphone-crashes-2026-10-25`, finding 3-3.  `the_end_check_sees_a_mix_file_written_again_at_its_size` tests the check alone.  `a_mix_file_written_again_after_the_last_read_ends_with_unstored` reaches the check through a separation, in Chrome too (finding 4-1 of review round 4 of M4).  The write comes over a second after the open, so Safari can also see it.  A write that makes the file shorter fails a read first (`a_mix_file_changed_during_the_separation_ends_with_unstored`).

* Fix: a hash of the mix, kept in `meta` and checked by the worker as it reads, or a lock on the file during the separation.  Only a fault in the page can write the file at that time.

### TD-16: the wire format has unchecked field names and silent defaults

`xtask/e2e/worker.js`, and the scripts in `xtask/src/e2e.rs` and in the Rust files under `xtask/src/e2e/`, repeat field names by hand.  Nothing checks them.  `e2e_messages_use_protocol_types` checks the message types, but only in the shapes it reads.  It does not read, for example, a `switch` and its `case`s, an assignment `m.type = "x"`, `d["type"]`, `"x" === d?.type`, a variable, a template literal or a regex literal, whose quote hides the rest of its line.  In Rust it does not read, for example, `.as_str() == Some("x")` or `json!("x") == r["type"]`.  In a plain Rust literal, a `\n` escape after a `//` comment hides the rest of the literal.  In a plain Rust literal, an escape other than `\\` and `\"` stays as a `\` and its text.  So an escape such as `\n` or `\t` joins its letter to the next word, and `\ntype` reads as `ntype` (`docs/reviews/replay.md`, case `m10`, finding 1-1).  Outside a JS string, a `\'` opens a string that hides the rest of its line.  So the check does not read a type such as `\'x\'`.  It reads no script outside `xtask/e2e/*.js`, `xtask/src/e2e.rs` and the Rust files under `xtask/src/e2e/`.  Some text in a type's place is not a message, such as `{ type: '{kind}' }` in a `format!` string, a `Blob`'s MIME type, or `"string"` in `typeof d?.type === "string"`: the check fails it, and `SCRIPT_ONLY_TYPES` takes it with a reason.  A reply missing a field decodes as `""`, `0` or `false`.  There is no version handshake between page and worker (PROTO-1).

The decode worker's messages (`stem_protocol::decode`, M5) refuse a missing or wrong field, so this entry concerns the stem worker's messages only.  `e2e_messages_use_protocol_types` checks each hand-written type against the union of both workers' kinds, so a decode kind in a script for the stem worker passes (review round 1 of M5, finding 1-16).  CI's `browser-tests` job does not run `e2e stored-stems`, `e2e worker-check` or `e2e weights`, since they need a GPU and the weights.  These three scenarios also write stem worker messages by hand.  When a change touches their messages, the agent runs the changed scenario on its machine's GPU with the local model file, and the PR names the run.  If no GPU or model file is there, the PR lists the scenario as not verified.

Some texts also cross between the worker and `xtask` by hand.  The worker's `gpu_text` (`worker/src/web.rs`) writes ", fallback adapter ".  `perf_result` and the fixture of its tests in `xtask/src/e2e/perf.rs` repeat it.  `never_a_pass` refuses each value of the JS `gpu_fallback` but "no", so a changed text fails closed.  Nothing checks that these texts are the same.  But `perf_result` fails, and names the line, when the text is missing.  Tier `push`, when asked with `STEMYLA_PERF_WEIGHTS` set, runs `e2e perf` through the perf gate.  It does so for a path in `roformer/`, `roformer-gpu/` or `worker/`, a workspace file, or no known path.  A change in `xtask/` alone does not run it.  The query keys of `MEASURE_KEYS` in `perf.rs` are a copy of `stem_protocol::Settings::from_query`; `each_query_key_is_measured_or_not` checks that each key is listed.

* Fix: fail decoding on a missing required field; a version in `Alive`.  Until then, the `wire-reviewer` agent looks for a field name or a default that one side changes and the other side does not.  It looks on each diff that routes it (TD-31).

### TD-45: the status of decision 0007 may not be a person's choice

Decision 0007 (`decisions/0007-no-allocation-in-the-audio-callback.md`) has the status "accepted (by the maintainer)", and an agent wrote it.  Finding m11 1-2 claims that the limits of its scope are the agent's choices.  The M11 heading of the phase 2 plan says "maintainer decision, made 2026-10-19".  The maintainer judges.  `CLAUDE.md`, "Working here", Decisions: agents write a record as `proposed`, and only a person accepts it.

Source: `docs/reviews/replay.md`, case `m11`, finding 1-2.

* Fix: the maintainer confirms the status, or sets it to `proposed`.

### TD-57: a loop moved behind the head can underrun at the old loop's start

This entry concerns the underrun count of M6 (D4 and D6 of `plans/completed/stream-from-storage-m6.md`).

A loop set behind the head is a held seek: the player keeps its old loop until the run of the new loop's wrap is read.  The reader reads ahead along the loop of `SharedParams`, which is the new loop.  So the reader can give the slot of the old loop's start to another block.  If the old loop wraps before the jump, the stored lanes play silence there and count an underrun.  The jump comes after about one block read, so the gap is short.

* Fix: the player gives its own loop with `Hint::Reading`, and `wanted_blocks` follows that loop while a seek holds.

### TD-58: a song of many sources decodes on the page

This entry concerns MEM-1 and MEM-4.

Since C of `plans/completed/stream-from-storage-catchup.md` (decision 0020), every open that decodes a song on the page keeps the memory path's limit for its lanes (`app::memory_path_frames`).  It reads each file's head first, and refuses a long song before it reads a whole file.  It also refuses a file larger than the largest file of its stated length, or of the limit if it states none (`engine::decode::largest_file`).  Then it reads, decodes and drops one file at a time (`engine::song::SongWithin`), and the decode stops at the limit.  What is left:

* D2 of `plans/completed/stream-from-storage-m7.md` still sends a song whose lanes do not fit the read-ahead buffer at its minimum slots to the page (more than about 20 sources and no `stem` lines; `store_meta::open_formats` and `store_meta::fits_read_ahead`).  Such a song holds at least 28 lanes, so it opens only up to about 25 s, and a longer one is refused.
* The page holds the bytes of the file that decodes, as well as the lanes.  It reads the file into wasm memory in blocks (`app::picker::read_blob`).  Each read asks JavaScript for never more than one block of 1 MiB.  Each block is garbage after its copy.  For a song of 100 s, a WAV or FLAC that states no length is read up to about 324 MB.  An MP3 is read up to about 21 MB.  A WAV or FLAC that states a length is read up to 3,072,000 bytes for each of its seconds, and 16 MiB for tags.  An MP3 whose ID3 tag is larger than the head counts as an MP3; a FLAC behind such a tag gets the cap of an MP3.

* Fix: a read-ahead buffer that holds fewer lanes than the song shows, or a mix of the sources into fewer lanes, removes the D2 trigger.  A decode on the page from the file in blocks, as the decode worker does, removes the whole read.

### TD-59: an MP3 save holds about 3.5 times its file in the stem worker

This entry concerns MEM-1 and MEM-4.  It is a known breach of MEM-1: the stem worker holds a buffer that grows with the song.

`rusty_mp3` 1.0.0 turns on its bit reservoir for CBR at 256 kbps or less on MPEG-1 (32, 44.1 and 48 kHz).  Then it keeps every encoded frame until `finish()` (`push_with` and `finish` in its `src/lib.rs`).  Only the `MP3_RESERVOIR` environment variable turns the reservoir off, and wasm has no environment.  So an MP3 encode holds the MP3's frames in the stem worker's WebAssembly memory.  This holds on the memory path and on the storage path of a save (`worker/src/web/encode.rs`, D4 of `plans/completed/stream-from-storage-m8.md`).  While `finish()` runs, the kept frames, the main-data stream and the output frames are live together.

The measures:

* On 2026-10-28 in Chrome, `a_stored_mp3_holds_at_most_four_times_its_file` measured a 60 s MP3 of 1,438,548 bytes, about 1.44 MB a minute at 192 kbps.  Its peak was 5,096,709 bytes, 3.54 times the MP3.
* On 2026-10-29, `e2e save` measured the growth of the stem worker's WebAssembly memory during an encode.  It was 5.1 MB for an MP3 of 1 minute (1,438,548 bytes, 3.55 times).  It was 46.9 MB for an MP3 of 10 minutes (14,379,220 bytes, 3.26 times).
* So a 30-minute MP3 save holds about 43 MB of MP3.  At the 10-minute factor, it takes about 141 MB of the stem worker's WebAssembly memory at its peak.  That memory never shrinks.

A FLAC or WAV save holds no file.  M10 of `plans/completed/stream-from-storage.md` counts this memory in its budget.

* Fix: a switch for the bit reservoir in `rusty_mp3`, upstream, or another MP3 encoder that gives each frame as it encodes it.  A rate of 320 kbps turns the reservoir off, but it changes the files, which is the maintainer's choice.  Until then, `a_stored_mp3_holds_at_most_four_times_its_file` keeps the peak under 4 times the MP3 plus 1 MiB.

### TD-60: the download of a save file is checked in Chrome only

This entry concerns VERIFY-1.

Since M8, the page offers a save file as a `Blob` of its OPFS `File`, through an object URL and a link (decision 0021).  `an_offered_save_file_is_removed_when_its_timer_ends` and `e2e save` check the offer in Chrome.  No check runs it in Safari or on an iPhone, where the maintainer uses the page.  Safari may read a `File` of OPFS as it is when it is read, not as it was when it was opened (`references/browser-quirks.md`).  The page does not change a save file after it offers it.

* 2026-10-30: the maintainer saved an MP3 mix on an iPhone, and it downloaded.  In Safari on the Mac, the maintainer saved a FLAC and a WAV mix.  WAV and FLAC saves on an iPhone are not checked.
* Fix: save a stem as WAV and as FLAC on an iPhone, and play each file from the Files app.

### TD-63: the f16 weight kernels are not checked on an iPhone

This entry concerns VERIFY-1 and GPU-2.

M9 of `plans/completed/stream-from-storage.md` adds an f16 variant of each matmul kernel (decision 0018), and makes f16 weights the default.  On this Mac, Safari compiled and ran every variant on 2026-10-30: `e2e bench --browser safari --query 'bench&bench_quick=1&mm=s64&weights=f16'` passed with no WebGPU error, and logged `f16 weights: the start-up check passed (matmul checks: f32 67 ms, f16 1330 ms)`.  So in Safari the f16 part of the start-up check takes about 1.3 s, against about 43 ms in Chrome; this is a measure only.  No check has run the variants on an iPhone.  Where a Safari rejects a variant or fails the start-up check, the worker keeps f32 weights (decision 0022), so the stems stay right and the tab needs about 350 MB more.  So does a GPU that flushes f16 subnormals to zero, which WGSL allows.  The iPhone's GPU is not yet measured.

* 2026-10-30: the maintainer separated a 5-minute song on an iPhone, but its log was not kept, so whether the iPhone used f16 weights is not known.
* Fix: an iPhone run that keeps the log, with the start-up line and the `memory:` line of f16 weight matrices.

### TD-64: the start-up check compiles the f16 variant of every tile

This entry concerns the time to the first separation (MEM-4 does not apply).

`Gpu::check_kernels` compiles and runs the f16 variant of each tile of `TILES` (five since M4 of `plans/active/gpu-speed.md`), also when a stored kernel choice names only two of them.  The net reuses the pipelines that the check compiled: a `OnceCell` for each tile and format in `Gpu`'s kernels, and the worker keeps the checked `Gpu`.  So only the unchosen tiles cost extra.  In Safari on this Mac, the f16 part of the check took 1330 ms on 2026-10-30, with four tiles, against about 43 ms in Chrome, so the unchosen tiles cost up to about 1 s there.  M10 reads the iPhone's time from the line `f16 weights: the start-up check passed (matmul checks: f32 N ms, f16 M ms)`.

* 2026-10-30: a later run in Safari on the Mac, with a stored choice, logged `matmul checks: f32 47 ms, f16 51 ms`.  So the 1330 ms was a first compile, and Safari keeps the compiled shaders.
* Fix: check only the tiles of the stored choice.  A first run calibrates, which needs each tile anyway.

### TD-65: no test plants an internal error in an f16 variant

This entry concerns VERIFY-1 and GPU-2.

`Gpu::compile_f16` makes each f16 variant inside a validation and an internal error scope (`F16_COMPILE_SCOPES`; `f16_compile_scopes_are_internal_and_validation` pins the list only).  `an_f16_variant_that_does_not_compile_keeps_f32_weights` and its browser twin plant a WGSL error, which raises a validation error.  An internal error, a driver that fails to build a valid pipeline, cannot be planted from WGSL: a layout that does not match the shader is a validation error too.  So the internal scope is not shown to catch one.  A unit test pins the constant only: a `compile_f16` that pushed the validation scope alone would pass every test.

* Fix: a planted internal error, if wgpu or a browser offers a way to make one.

### TD-66: M9's mutants are not all tested

This entry concerns VERIFY-1.

The review rounds of M9 (`plans/completed/stream-from-storage-m9.md`) ran cargo-mutants in part only.  Round 1 tested 95 of 472 mutants (64 caught, 25 missed, 6 unviable) and stopped at its limit of 60 minutes, so 377 were not tested.  Round 2 tested none of its 21: the baseline `cargo test` timed out at 300 s twice while reviewers and builds loaded the Mac.  The missed mutants of round 1 got tests, except the checks that only a live WebDriver page reaches: `note_run` and `run` in `xtask/src/e2e/weights.rs` (lines 231, 233, 254, 256 and 488 at `0789e6b`).  The mutants of `note_run` that round 1 sent to this TD are entries of `KNOWN_MISSED` (`xtask/src/review.rs`).  The other mutants of `note_run` go to the validator.  Line 488 has two `==`, and its row has no column, so it has no entry.

No test with a rejected f16 variant goes through the worker's `gpu` and `load_format` (`worker/src/web.rs`), so their log lines are not tested.  Review round 3 found this; it was there before the round.

* Fix: run `cargo mutants` on M9's files on a quiet machine: `roformer-gpu/src/lib.rs`, `bench.rs` and `net.rs`, `roformer/src/weights.rs`, `xtask/src/e2e/weights.rs`, `xtask/src/e2e.rs`, `protocol/src/lib.rs` and `test-support/src/lib.rs`.  Move the checks of `note_run` and `run` into pure functions with tests.  Call `gpu` and `load_format` from a test in `worker/src/web/tests.rs`, which can reach the private functions.

### TD-79: `alac-encoder`, a test reference, has no maintainer release since 2022

This entry concerns DEPS-1.  It is a known breach of "confirm that it exists and is maintained".

`m4a` takes `alac-encoder` 0.3.0 as a dev-dependency only (`m4a/Cargo.toml`).  The differential test of the ALAC port compares each packet with this crate (decision 0028).  Its last release is 0.3.0, of January 2022.  It is pinned exactly, it never ships, and no update is wanted, since it is the fixed reference of the port.

* Fix: remove the dev-dependency when the port no longer needs the reference.  For example, keep a SHA-256 digest of the reference packets of each test signal as test data.  Do not store the packets: they are encoded audio (ASSET-1).  Until then, do not update the pin.

### TD-80: a raw AAC file (ADTS) keeps its encoder delay

This entry concerns D7 of `plans/completed/m4a.md`: the decode trims the AAC encoder delay and padding.

A raw `.aac` file is a stream of ADTS frames, with no container.  No box states its delay or its padding, so `engine::decode` cannot trim them.  The decode gives the encoder delay as silence at the start, and the padding at the end.  The delay is 1024 frames from `oxideav-aac`, and 2112 frames from Apple's encoder.  So a raw AAC stem starts late against a WAV lane.  The test `a_raw_adts_stream_decodes_with_its_delay` shows it.

* Fix: no standard field holds the delay of ADTS.  An open could take a delay that the caller gives, or the delay of a known encoder.  An M4A of the same audio has no gap.

### TD-86: an ADTS frame of more than one raw data block and no CRC does not open

This entry concerns the ADTS promise of M2 of `plans/completed/m4a.md`, and the limits of Symphonia 0.6.1.

An ADTS frame can hold up to 4 raw data blocks of AAC.  `symphonia-codec-aac` 0.6.1 refuses more than one ("only 1 aac frame per adts packet is supported", `adts.rs`, line 192).  When the frame has a CRC, its header states where each block starts, so the decode gives each block as its own frame (`engine::adts_split`, review rounds 8 and 9).  It does so in each frame of the stream, also after bytes that are not a frame, from a source that seeks or not.  A cut last frame gives its whole blocks, each with its CRC.  When the frame has no CRC (`protection_absent` 1), nothing states where a block ends.  Only a parse of the AAC bitstream of the block finds its end, and that parse is the work of the decoder.  So the decode refuses such a frame with `ADTS_BLOCKS_TEXT`: before the open when it is the first frame of a source that seeks, and in the decode when it comes later.  A first such frame from a source that cannot seek gets "no known format", since the scan does not keep its first bytes.  `an_adts_stream_of_more_than_one_raw_block_gives_its_text` and `an_adts_stream_of_raw_blocks_with_a_crc_decodes` test it.

* Fix: a Symphonia that reads each raw data block of a frame, or a parse in `engine` of the syntax of an AAC raw data block to find its end.

### TD-90: an M4A stream with `moov` last past the limit, or far into the stream, does not open

This entry concerns the M4A promise of M2 of `plans/completed/m4a.md`.

From a source that cannot seek, Symphonia 0.6.1 stops its walk of the top-level boxes at the first `mdat` or `moof` (`symphonia-format-isomp4` `demuxer.rs`, lines 210 to 225), and then has no `moov` (lines 246 to 248).  When the window of the scan (about 1 MiB) shows an `mdat` or `moof` before any `moov`, the decode holds the stream whole, up to the limit of its samples (`MAX_DECODED_BYTES` by default), and opens it as a source that can seek (`engine::held`, review round 16).  Two cases keep `MOOV_LAST_TEXT`:

* A stream past that limit.  A file of more bytes than the decoded samples that the decode takes can hold no song that it opens, but for its tags.
* A stream with more than the window of top-level boxes before its first `mdat` or `moof`.  The order shows only after those bytes pass.  To hold them for each M4A stream would break the flat peak of a stream with `moov` first and a large box before it, which `an_m4a_stream_gets_the_hiding_and_the_texts_of_the_walk` pins.

`an_m4a_stream_of_other_layouts` tests the second case, and `an_m4a_stream_with_moov_last_is_held_and_opens` the first.  The sources of the app seek; `decode_path` of a FIFO is the path that reaches these cases.

* Fix: for the second case, a hold that starts only at a box that the stream does not hide.  No fix is in view for the first.

### TD-91: the published page ships no licence text

This entry concerns DEPS-1.

Apache-2.0 section 4(a) asks that each recipient of the Work in Object form gets a copy of the licence ([Apache License 2.0](https://www.apache.org/licenses/LICENSE-2.0)).  `rusty_mp3` and `flacenc` ship in `stem-worker` as WebAssembly.  Since M3 of `plans/completed/m4a.md`, `m4a` ships there too, and it is Apache-2.0 only (decision 0029).  On 2026-11-10, `origin/gh-pages` held no `LICENSE*` file and no `THIRD-PARTY.md`.  So the published page gives no copy of a licence for any crate.  No check fails this.

* Fix: copy the licence texts and `THIRD-PARTY.md` into the build, in their own PR.

### TD-103: Safari keeps a swap file of each write stream that closes

It breaks no rule.  It is a known leak of disk in Safari, by a defect of WebKit.  `references/browser-quirks.md` gives the quirk, and `references/webkit-bug-writable-swap-file.md` the draft bug.  M5.1 of `plans/active/gpu-speed.md` measured the leak and stopped its largest part.

Safari's folder is `$DARWIN_USER_TEMP_DIR/com.apple.WebKit.Networking+com.apple.Safari`: the folder of `getconf DARWIN_USER_TEMP_DIR`.  On 2026-11-14 it held 35,841 `FileSystemWritableStream*` swap files and 767 `BlobRegistryFiles-*` folders, 176 GB, from test runs.  A `close()` of a `createWritable` stream keeps its swap file, of the bytes written.  An `abort()` removes it (`FileSystemStorageHandle::closeWritable` in WebKit, and the counted runs of M5.1 Step 3).  Safari did not remove the files in 5 minutes while it ran.  After a quit of Safari, with its network process ended, the 35,841 files were still there (the orchestrator, 2026-11-14).

What still leaks after M5.1 Step 5c, in a user's Safari:

* The page's small writes, which stay on `createWritable`: `meta` (0.1 to 0.4 KB a write) and `sessions/current` (13 bytes).  A song open left 3 such files, 327 bytes (M5.1, Step 5d).  The saves of `e2e save` (WAV, FLAC, M4A, MP3 and AAC of 1 and 10 minutes) left 34, about 1 KB.  A `close()` replaces such a file in one step, and the page reads `meta` with no lock, so they stay.
* Not the song-sized files.  Since M5.1 Step 5c, the write worker writes the page's song-sized files through sync access handles (`app::writer`, decision 0032).  These are the source files, the mix, the stems of the memory path and the save files.  Since Step 4, the stem worker writes the model and `matmul_kernels` the same way.  Before, each model save left 349.5 MB.  Each song open left the bytes of the dropped files, and each save the bytes of the save file.

What leaks in test runs only:

* The test helpers that plant files through `createWritable`.  These are `plant` and `plant_at` in `worker/src/web/tests.rs` and `decoder/src/web/tests.rs`, `store_plant_tone` and `store_plant_wav` in `app/src/store.rs`, and the scripts of `xtask/src/e2e.rs`, `xtask/src/e2e/` and `xtask/e2e/worker.js`.  `e2e save` plants a session of 1 minute and one of 10 minutes (`old_session`'s plant): 349.3 MB.
* The page-side fake of the write worker (`fake-writer`), which the app's browser tests use.  It writes each block-written file through one write stream, from its block at 0 to its `write-end`.  So each such file leaves one swap file of its size.  Each whole file that it writes through `st_write` leaves one too.  After Step 5c and before `write-end`, it opened a stream for each block.  A whole Safari `web-test` then left 1,556 swap files, 8,898.9 MB, against 1,316 files, 1,697.2 MB before Step 5c.
* One empty `BlobRegistryFiles-*` folder for each WebDriver session.  It holds no bytes.

* `xtask` does not remove these files: the person's own Safari writes in the same folder, so `xtask` cannot tell its own files.  Each Safari `e2e` and `web-test` run prints one line with the swap files and blob folders that it left (`watch_safari_data` in `xtask/src/drivers.rs`).  A person removes them by hand (`references/local-browser-testing.md`).
* Fix: WebKit removes the swap file at `close()`.  Until then, the page's small files and the test runs' writes leak, and a person deletes the files after a run (`references/local-browser-testing.md`).  Since `write-end` (M5.1), the fake leaves one swap file for each file that it writes in blocks, as the page did before M5.1.

## Legibility for agents

### TD-18: large files and JavaScript inside Rust strings

`roformer-gpu/src/lib.rs`, `app/src/lib.rs`, `worker/src/web.rs` and `xtask/src/e2e.rs` are each over 1,200 lines, and storage logic is JavaScript inside Rust strings (`worker/src/web.rs`, `app/src/store.rs`, `app/src/reset.rs`), out of reach of Rust tooling.

* Fix: split by concern when next changed; move inline JS into `.js` modules.

### TD-19: the running app is legible only through pixels and log text

The app draws to one canvas.  `xtask e2e` clicks fixed coordinates (`VIEWPORT`, `BAR_Y` in `xtask/src/e2e.rs`) and asserts on log lines, so a layout change breaks it, and an evaluator agent cannot read the app's state (VERIFY-1).

* Fix: a small `window.__stemyla` state snapshot (scalars only, per MEM-1), structured console events, `?fixture=demo`, and a command API that queues into the same inbox as a click.

### TD-20: process rules rest on local settings

Commit signing is set in the maintainer's own git config only (GIT-2).  The agent runs with the maintainer's credentials, so GitHub cannot tell the two apart (GIT-1).  The `main` ruleset (checked 2026-10-21 with `gh api repos/lloydmeta/stemyla/rulesets/24980616`):

* does not require signed commits;
* lets the admin role bypass it for pull requests only (since 2026-10-21; before, always).  A direct push to `main` is refused.  An agent can still merge a PR past the approval and the checks (`gh pr merge --admin`); only the deny rules refuse that;
* does not require a branch to be up to date before merging (`strict_required_status_checks_policy: false`), so a PR can merge untested against the current `main` (the failure behind PR #12).

The `gh-pages history` ruleset (id 25544673, since 2026-10-21) blocks force pushes and deletion of `gh-pages` for everyone (GIT-3).  It does not restrict a fast-forward push, so a plain push to `gh-pages` publishes the site (GIT-1).  A rule that only CI may push needs a credential that the agent cannot use, such as a deploy key in an environment limited to `main`.  The maintainer's local `xtask publish` would then stop working.

* Fix: in phase 5 of the plan, the other ruleset changes and the agents' own identity.  Phase 2 denies a push that names `main` or `gh-pages`, publish and merge (decision 0010), and set the two rulesets above.

## Closed

* TD-1: closed by `every_kernel_fits_the_default_limits` (harness phase 2, M2).  It checks every shader against the default limits with naga (GPU-1).
* TD-2: closed by four `#![expect]`s in `dsp/src/lib.rs`, one for each lint, with a reason (harness phase 2, M6).  Every other clippy lint keeps its workspace level in `dsp`.  With `-D warnings`, an `expect` whose lint stops firing fails clippy (DSP-1).
* TD-4: closed by `crates_depend_only_downwards` (harness phase 2, M5).  It compares the dependencies between crates in `cargo metadata` with the layers table in `ARCHITECTURE.md` (LAYER-1).
* TD-5: closed by `no_python_in_the_build` (harness phase 2, M8).  It checks the git index and the build files for Python (BUILD-1).
* TD-6: closed by `no_audio_or_weights_are_tracked` and the anchored `.gitignore` (harness phase 2, M8).  The test checks the git index for audio, weights and large files (ASSET-1).
* TD-9: closed by harness phase 2, M17.  CI's `test` job compares the UI snapshots, and `no_build_file_updates_snapshots` keeps `UPDATE_SNAPSHOTS` out of the build files (VERIFY-1).
* TD-12: closed by `sources_are_saved_from_their_files` (harness phase 2, M18a).  The page saves a chosen song from the browser's `File`s and moves the bytes into the decoder, so it no longer copies them.  The test shows that the source reaches storage as the browser's file (MEM-1).
* TD-13: closed by `an_mp3_reply_reaches_the_page_as_sent` (harness phase 2, M18b).  `Separator` keeps the transferred `Uint8Array` in `Event::Mp3`, and `store::download` takes a `Uint8Array`, so the page has no reason to copy it into wasm memory (MEM-1, MEM-2).  The test does not reach `App`'s arm, where a copy by `copy_to` passes the lint (TD-26).
* TD-14: closed by `an_mp3_is_encoded_in_blocks` (harness phase 2, M18b).  The worker reads the channels from JS in blocks, so an encode holds a few times the size of the MP3, not the PCM (MEM-1).
* TD-15: closed by `the_callback_does_not_allocate_after_warm_up` (harness phase 2, M11).  wsola 0.2.0, a warm-up of each stretcher, chipmunk decimation in place and a bound on the playhead's anchors removed the allocations in the audio callback (RT-1).
* TD-17: closed by `a_view_is_written_whole` (harness phase 2, M18a).  `st_write` copies a view into a whole array, as the worker's `whole()` does (MEM-5).
* TD-22: closed by a step of CI's `test` job, `cargo +nightly-2026-10-30 test --locked -p roformer-gpu --doc` (tech-debt rung 1).  Nightly rustdoc checks the error code of a `compile_fail` doctest.  With `E0308` in place of `E0505` on `Rec`'s doctest, the step failed: "Some expected error codes were not found".  Stable passed it (MEM-3).
* TD-24: closed by DSP-1's new list of types (harness phase 2, M7).  The list now names every type that `dsp` uses.
* TD-30: closed by `set_viewport` in `xtask/src/webdriver.rs` (harness phase 2, M17).  A read at once after a resize could still give the old size, so `xtask e2e` sometimes could not size Chrome's viewport.  Each try now decides on a read taken once the page's outer size equals the new window.  `settle` starts no new read after 2 s.  A read that gives the target is read again, and five tries correct a late read.  In the CI runs with the fix, every read at once was already right.  So only the unit tests show a late read handled: `the_viewport_fits_despite_a_stale_read_at_once` and `a_late_settled_read_is_corrected_by_the_next_try`.  The e2e log line `viewport: ...` shows the race if it comes back.
* TD-40: closed by new RT-1 tests (tech-debt rung 1).  `the_manifest_check_finds_a_stretcher` has a breach case for `"default"` in an `engine =` line and for a manifest with no `engine =` line.  `wsola_gives_the_same_output_for_a_push_of_max_push_frames` checks wsola's output for pushes of 4,096 frames.  `wsola_warms_up_with_a_bounded_push` bounds the bytes of the warm-up, with a new byte count in the test allocator.  `the_playhead_reaches_the_loop_end_before_each_wrap` fails the `!=` and `<=` mutants of `drop_oldest_anchor_when_full`.
* TD-42: closed by new cases in the tests of `e2e_messages_use_protocol_types` (tech-debt rung 1): `a_script_only_type_in_the_rust_alone_is_not_stale`, `a_type_after_a_comment_of_several_lines_has_its_line`, a misspelt type first in `includes`, and `<=` and `format!` after `"type"` in Rust.  Each of the five mutants that the entry named fails a test (PROTO-1).
* TD-51: closed by `a_long_input_converts_with_a_32_bit_usize` and the `engine` crate run of `xtask web-test` (tech debt rungs, M2, step 8).  The tests of `engine::resample` run on wasm32 in a browser.  A planted `usize` product in `Filter::apply` fails there, and passes natively (VERIFY-1).
* TD-95: closed by `a_planted_file_reads_back_as_written` (tech debt rungs, M2, step 5).  The test helper `plant_at` of `decoder/src/web/tests.rs` wrote a view of the wasm memory.  Safari wrote the whole memory (`references/browser-quirks.md`), so each planted file started with other bytes.  The helper now writes a copy, and the 13 tests pass in Safari 27.2 and Chrome 157.  The decode worker itself had no fault (VERIFY-1, MEM-5).
* TD-97: closed by a bound of 3 MiB in D11 of `e2e save`, `the_two_level_pairs_of_safari_pass_and_one_byte_more_fails` and `a_save_that_keeps_a_part_of_the_song_fails` (tech debt rungs, M2, step 11).  In Safari, the growth of the page's WebAssembly memory in a save had two levels, about 1.9 MB apart, whatever the song's length.  So the bound of 1 MiB failed at times: 1 of 6 runs, and CI run 43008561411 (MP3: 5,636,096 and 7,536,640 bytes).  Step A of `plans/completed/tech-debt-rungs-m2-td97.md` saw the same requests of the heap in each measured save, all at the high level.  So the allocator's layout is the likely cause.  The bound does not depend on the cause.  Over 150 Safari pairs, the largest 10-minute minus 1-minute growth was 1,900,544 bytes (CI run 43008561411).  The extremes of single growths seen, 5,505,024 (step 9) and 7,602,176, are 2,097,152 bytes apart.  Over 30 Chrome pairs, the largest difference was 196,608, and 327,680 in absolute value.  At one level, a save that keeps about 1/30 of the song's 16-bit audio fails (349,525 bytes a minute).  In the worst layout of those extremes, a save that keeps about 1/18 fails (582,542 bytes a minute) (VERIFY-1, MEM-4).
* TD-56: closed by M7 of `plans/completed/stream-from-storage.md` (D12 of `plans/completed/stream-from-storage-m7.md`).  One counter gives a token to every open (a drop, a pick, a Storage open, the startup restore and Demo).  Only the newest open shows its song, and only `apply_inbox` makes a session current.  `a_late_drop_does_not_show_over_a_newer_song`, `a_late_drop_does_not_show_over_demo`, `two_opens_that_end_in_one_frame_show_the_newer` and `a_stale_restore_on_the_page_does_not_make_its_session_current` check it.
* TD-61: closed by D11 and review rounds speed 3 and 4 of `plans/completed/stream-from-storage-m8.md`.  Before D11, a save advanced one block in each UI frame, and CI's `browser-tests` job took 51 minutes (run 40799703090).  CI run 40894815433 (2026-10-29), on `bf3334f`, passed in 18.6 minutes.  `web-test` took 9.6 minutes, the other e2e scenarios 4.0, and `e2e save` 3.3, within its goal of 5.  `web-test` took 7.8 minutes in run 40871041420, so it varies by about 2 minutes.
* TD-69: closed by `check_runs_each_check_on_a_repo` in `xtask/src/docs.rs` (tech-debt rung 1).  It runs `docs::check` on a copy of the repo's tracked files, and wants no problem.  Then it plants one problem for each check, and wants each one.  The mutant `replace check -> Result<Vec<Problem>> with Ok(vec![])` fails it, so its `KNOWN_MISSED` entry is gone (VERIFY-1).
* TD-83: closed by review round 1 of `plans/completed/m4a-m2.md`.  The maintainer asked, through the orchestrator on 2026-11-08, that the native `WELCOME` name M4A and AIFF.  It does, with a new `app/tests/snapshots/empty_state.png`, which the PR lists for the maintainer to look at.
* TD-81: closed by review round 7 of `plans/completed/m4a-m2.md`.  An HE-AAC stream whose config shows no SBR puts SBR data in a fill element of each packet, before ID_END.  The decode reads the first 8 packets of the track before it makes the decoder.  It gives the HE-AAC text when each of them holds SBR data in one of its last 3 fill elements, each of a known extension type, that end at ID_END, and one of them starts its SBR data with a header (`decode::first_packets_of_lc`, `aac_config::has_sbr_fill`, `aac_config::has_sbr_header`, review rounds 7, 8 and 20).  With no header in the first 8, it reads on while each packet holds SBR data, to the first header, up to 64 packets.  So a stream from an encoder that sends the header less often than each 64 packets, cut between two headers, still decodes as AAC-LC.  A packet whose SBR data comes before more than 2 other fill elements, or before another element, decodes as AAC-LC.  It decodes the packets that it read first.  `an_aac_whose_packets_carry_sbr_gives_the_he_aac_text` tests an M4A with the core rate in its entry, one with an entry rate of 0, and an ADTS stream, with crafted packets.
* TD-82: closed by review round 7 of `plans/completed/m4a-m2.md`.  Symphonia 0.6.1 refuses an `mvhd` timescale of 0 and uses it only for its `media_info`.  The view of the decode gives `mvhd` the timescale of the first sound track (`decode::mend_mp4`).  The trim reads the file, so it is the delay only.  `only_the_common_form_of_elst_gives_a_trim` decodes such a file.
* TD-85: closed by review round 7 of `plans/completed/m4a-m2.md`.  Symphonia 0.6.1 reads past the end of a `moov` of size 0.  The view of the decode gives that `moov` its size (`decode::mend_mp4`).  `the_art_of_an_m4a_is_hidden_from_symphonia` decodes an ALAC and an AAC file with such a `moov`, bit for bit.
* TD-87: closed by review round 10 of `plans/completed/m4a-m2.md`.  Symphonia 0.6.1 multiplies the step of Microsoft ADPCM with no bound (`codec_ms.rs`, line 96), and the step triples for each nibble of 8 from any start.  Before Symphonia decodes each packet, the decode walks the step of each channel of each block, and refuses the file with `MS_ADPCM_TEXT` when a step passes `i32::MAX / 768` (`engine::adpcm::ms_steps_fit`).  `a_crafted_ms_adpcm_wav_is_refused_with_no_panic` runs in each build, from steps of 16 and 32,767.
* TD-88: closed by review round 13 of `plans/completed/m4a-m2.md`.  From a source that cannot seek, Symphonia 0.6.1 read the art of an M4A whole, and the checks of the walk of `engine::mp4_boxes` did not apply.  The decode now gives an M4A stream to Symphonia through `engine::mp4_stream::Mp4`, the MP4 rule of `TagStream`.  It follows the head of the stream up to the first `mdat` or `moof`, shows each box that the walk hides as `free`, and gives `VIDEO_TEXT`, `PROTECTED_TEXT` and `MOOV_TEXT` by the rules of the walk.  `an_m4a_stream_gets_the_hiding_and_the_texts_of_the_walk` tests an ALAC and an AAC M4A through `Fifo`.
* TD-89: closed by review rounds 13 and 14 of `plans/completed/m4a-m2.md`.  The scan of a source that cannot seek now reads past each leading ID3v2 tag with no buffer (`hidden::id3_tag_bytes`, then `io::copy` to a sink), as a file that can seek starts its view after them.  An MP3, an ADTS or a FLAC stream behind a tag of 64 MiB decodes at a flat peak (`a_stream_behind_a_leading_id3_tag_opens`).  Main (ff633a3) did not open such an MP3 or ADTS stream either.  Main has no M4A, so it is no baseline for an M4A.  An M4A with `moov` first behind tags and junk opens, from a file that can seek and from a stream, with its chunk offsets from the MP4 start, the end of the tags or the file start (`decode::offsets_base`, review round 14).  `an_m4a_behind_tags_and_junk_opens_wherever_its_offsets_count_from` tests it.
* TD-84: closed by review rounds 2 to 6 of `plans/completed/m4a-m2.md`.  The decode hides the tags and pictures of each format from Symphonia, which reads them whole (`engine::hidden`): the M4A `moov/udta`, top-level `meta` and `sidx`, and later `ftyp`, the FLAC PICTURE, VORBIS_COMMENT, APPLICATION and SEEKTABLE blocks, a leading ID3v2 tag, the WAV `LIST` chunk, and the AIFF tag chunks.  It opens one Symphonia reader at the first marker that Symphonia's probe would accept, so the reader starts where the plan starts (`hidden::start_of`, review rounds 5 and 6).  A plan hides at most 4096 small containers (`MOST_SMALL`).  A file is refused before Symphonia opens it only when the small ones past that count, which Symphonia keeps, pass 16 MiB (`MOST_KEPT_BYTES`).  Each counts its bytes and 256 bytes for each entry that it can hold (`kept_charge`, review round 5).  `the_art_of_an_m4a_is_hidden_from_symphonia`, `a_file_opens_at_the_first_marker_with_its_tags_hidden` and the tests beside them check a flat peak with 64 MiB of tags.  `a_file_is_refused_only_when_its_kept_tags_pass_the_limit` checks the peak at the limit for the densest form of each format.
* TD-101: closed by M5 of `plans/active/gpu-speed.md` (Steps 5 and 6).  Calibration timed the attention dispatches alone, which put split about 0.5 s for each chunk too low in Safari and 0.26 s in Chrome.  Calibration version 5 times whole time-transformer steps, with a wait after each, for 12 rounds, and `FUSED_MARGIN` is 1.0.  The bias is now -0.010 s for each chunk in Safari and +0.006 s in Chrome.  The check "Each calibration choice is not slower" (`references/local-browser-testing.md`) passes in both browsers: Safari chooses `fused`, 4 % faster than `freq`, and Chrome chooses `freq`, 1.4 % faster than `fused`.
