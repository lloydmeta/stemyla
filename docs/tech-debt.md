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
* `a_read_back_is_copied_into_wasm_memory_once` (`worker/src/web/tests.rs`, MEM-1) needs a GPU adapter in the browser.  A wasm test cannot read `STEMYLA_REQUIRE`, so with no adapter it logs a skip and passes.  CI's headless Chrome can have no adapter, so the evidence is a run on the maintainer's Mac.  Fix: a browser with WebGPU in CI's `browser-tests` job.
* The three `timestamps` tests, and the kernel check in `forward_matches_pytorch`, run nowhere.  CI excludes them, and the Mac's adapter has no `timestamp-query`.  Fix: run them on an adapter with `timestamp-query`, and name it in the PR.

### TD-10: GPU-3 is not checked

Nothing records which adapter a kernel passed on, and any new `TILES` entry becomes a calibration candidate on every user's GPU.

* Fix: log `isFallbackAdapter` and the adapter name in selftest and e2e results.  Until then, the `gpu-reviewer` agent flags a diff that makes a kernel the default, on each diff that routes it.  It does not confirm the pass on a real GPU.  That evidence is in the PR description, which the agent does not read.

### TD-11: MEM-4 is never asserted

The worker logs its wasm memory and live GPU bytes and the page its wasm memory, but no test asserts a total, and the page's WebGL textures and the JS heap are not counted.

* Fix: a tab total (page wasm, worker wasm, GPU live bytes) in an e2e scenario that separates a song and fails over a budget.
* `e2e memory-budget` and `separate` run with `chunk=16384`, so their figures are smaller than at the default chunk of 588,800 samples.  At `chunk=16384`, the stem worker's WebAssembly memory stays at 57 MB, and the GPU holds about 732 MB.  At the default chunk, the stem worker holds one chunk's working set: 57 MB at the start, 90 MB after chunk 1 and 123 MB from chunk 2 (catch-up E).  The GPU then holds about 979 MB: 699 MB of parameters and 280 MB of one chunk's activations.  The figure depends on the chunk, not on the browser: Chrome and Safari give the same.  At the default chunk, `memory-budget` fails its growth check of 4 MB, since the worker grows by 66 MB during the first two chunks of a separation.  That growth is one chunk's working set, and does not grow with the song.  Fix: M10's budget takes the default chunk's figures.
* No test sees that `GpuNetwork::stems` (`worker/src/web.rs`) drops a chunk's features (18.9 MB at the default chunk) after it uploads them.  Only a run of the model sees it, and CI has no GPU and no model.  The worker figures above, from `e2e memory-budget --query mm=s64` with the model, include it.

### TD-21: what docs-check cannot see

`xtask docs-check` (DOC-1) checks the shape of the docs, not whether they are still true:

* A doc that names real files and tests but describes old behaviour passes.
* An ID with a prefix not yet in use (`SEC-1`) is not checked, because it cannot be told from text such as `SHA-256`.
* A path counts as present if the file is on disk, so a file not yet added to git passes locally and fails only in CI.
* `README.md` and crate docs (`//!`) are checked for the crate list only.
* A bare file name (`store.rs`) is not checked, because the check cannot tell which crate it means.

* Fix: the drift job in phase 5a of the plan, which compares each doc with the code it describes.

### TD-22: a compile-fail doctest passes on any compile error

MEM-3's compile-fail doctests on `Rec` (`compile_fail,E0505`) and on `Gpu` (`E0616`, `E0624`) pass on any compile error, since stable rustdoc does not check the error code.  A `compile_fail,E0505` doctest that fails with E0308 passes on rustc 1.99.0 (checked 2026-10-19).  `Rec`'s `no_run` doctest compiles the same calls, so an API change still fails a test; a different error in the `compile_fail` copy alone does not.  The `Gpu` doctests have no such twin, since a `no_run` doctest cannot call a private item.  So a rename that also makes an item public (`pub dev`) fails them with another error, and they pass.

* Fix: run `cargo +nightly test -p roformer-gpu --doc` in CI, since nightly rustdoc checks the error code; or check it with a UI-test harness such as `trybuild` (a new crate, DEPS-1).

### TD-23: LAYER-1 sees crates, not code

`crates_depend_only_downwards` reads the dependencies between crates, not the source (LAYER-1).

* A crate from a registry or git, under another name, can hold a copy of a workspace crate, such as `roformer`.
* Source can reach across crates without a dependency: `#[path = "../../roformer/src/x.rs"] mod x;` or `include!`.
* A workspace crate's target can take its source from another crate's folder: `[lib] path = "../roformer/src/lib.rs"`.
* A symbolic link in a crate's folder can point to another crate's source.
* Fix: the `supply-chain-reviewer` agent looks for all four, on each diff that routes it (TD-31).  A later check could ban `#[path]` and `include!` with a path outside the crate.  It could check that each workspace crate's `targets[].src_path` is in its own folder.  It could also fail a tracked symbolic link, which `git ls-files -s` shows with mode `120000`.

### TD-25: RT-1's allocation test runs natively only

`the_callback_does_not_allocate_after_warm_up` does not count the WebAssembly build, cpal's hosts, or the closure in `engine/src/live.rs`.  That closure calls `Schedule::next`, whose fields are all `Copy`.  On the web, the first `web_time::Instant::now` in `render_interleaved` fills a `thread_local`.  A zero count is evidence, not proof, since the optimiser may remove an allocation.  The bench-only stretchers (`timestretch`, `cute-dsp`) are not covered.  `the_app_ships_only_these_stretchers` reads the text of `app/Cargo.toml` only, so it does not see a stretcher that another crate turns on through feature unification (RT-1).

* Fix: none planned; phase 4's evaluator may count allocations in the browser.

### TD-26: MEM-1's lint sees only `to_vec`

Clippy fails `slice::to_vec` in `app`, `stem-worker`, `decode-worker`, `stem-protocol`, `engine`, `roformer` and `roformer-gpu`, and `to_vec` on a `Uint8Array` or a `Float32Array` in the first four (MEM-1, MEM-2).  It does not see:

* other forms of copy: `to_owned`, `Vec::from`, `Box::from`, `into`, `concat`, `collect`, `extend_from_slice`, `copy_to` into a new buffer, and `clone` of a `Vec`;
* `to_vec` on the other typed arrays, such as `Int16Array`, and a wasm-bindgen import that returns a `Vec<u8>`;
* a `#[wasm_bindgen]` export that takes a `Vec<f32>` or a `&[f32]`, which copies a JS array in;
* an `#[allow]` in place of an `#[expect]`, as clippy's own help line suggests, and a reason that does not say why;
* a new `to_vec` in a statement that already has an `#[expect]`, and a new test in a test module with a module-level `#[expect]` or `#![expect]`;
* crates whose `clippy.toml` lacks the MEM-1 paths: `synth`, which makes the demo song in the web app, and any new crate.  `dsp` is covered by DSP-1's method list; `xtask` and `test-support` do not ship;
* a path that no longer resolves, except through the canaries, which prove their paths only while the clippy steps keep `--all-targets`.  Nothing fails if the `wasm32` step goes.

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

### TD-31: the route of `xtask review` is a short denylist, from the branch's own code

This entry concerns which reviewers read a diff.  So it touches each rule that a reviewer checks, not one rule only.

`xtask/src/review.rs` chooses the reviewers of a diff from its paths and from content signs.  The content signs are a short denylist, not a full list.  A sign matches as a whole token only.  So `create_buffer` does not match `create_buffer_init`, which `roformer-gpu/src/lib.rs` calls.  A sign counts only on an added line of a Rust file outside a `tests` folder and outside the reviewer's own paths.  A copy or a GPU buffer that a macro, an alias or a helper makes routes nothing.

Since M6, review round 1, the paths of `memory-reviewer` include `roformer/src/`, `roformer-gpu/src/` and `synth/src/`.  So a change to the GPU memory budget (MEM-4) or to the model read (MEM-1) routes it.  Since M6, review round 2, they also include `dsp/src/`.  Its code runs in the audio callback (RT-1) and builds the song-length mix (MEM-4).  `e2e_messages_use_protocol_types` enforces PROTO-1.  A change to its list `SCRIPT_ONLY_TYPES` in `xtask/src/rules.rs` routes only `supply-chain-reviewer` and `reviewer`.  Neither agent lists PROTO-1, so no reviewer checks PROTO-1 on a change to that list.

The route comes from the branch's own `xtask`.  So a branch can change the route, or remove the rule that a change to `xtask/src/review.rs` routes every reviewer.  `xtask review start` warns when the branch changes `xtask/src/review.rs`, and when the binary is older than that file.

* Fix: review; `reviewer` reads every diff.  Phase 5 routes with the base branch's `xtask`.

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

* cargo-mutants builds and tests natively.  A `wasm32`-only block in a shared function shows as a missed mutant.  A `wasm32`-only function, impl or module in a shared file has `#[cfg_attr(mutants, mutants::skip)]`, so cargo-mutants makes no mutant of it, struct-field deletions included.  No build sets `cfg(mutants)`, so no crate is needed, and `[workspace.lints.rust]` in `Cargo.toml` declares the cfg for `unexpected_cfgs`.  A new `wasm32`-only function without the attribute shows as missed, until a change adds the attribute.  A function with a native variant too, such as `base_latency` in `engine/src/live.rs` and `main` in `app/src/main.rs`, has no attribute.  The validator decides its `wasm32` mutants.
* `exclude_globs` in `.cargo/mutants.toml` leaves out `app/src/decode_tests.rs`, `app/src/decoder/client.rs`, `app/src/picker.rs`, `app/src/playback.rs`, `app/src/playback_tests.rs`, `app/src/separator.rs`, `app/src/store.rs`, `app/src/streamtest.rs`, `decoder/src/web.rs`, `decoder/src/web/` and `worker/src/web.rs`, which compile for `wasm32` only.  So the run never tests the wire files of either worker: `app/src/separator.rs` and `worker/src/web.rs` for the stem worker, and `app/src/decoder/client.rs` and `decoder/src/web.rs` for the decode worker.
* `mutants_command` sets no `STEMYLA_REQUIRE`, and passes on the caller's own.  So without it, where no GPU opens, the GPU tests skip, and a mutant that only they cover shows as missed.
* A diff of tests only, WGSL or JavaScript gives no mutants.
* A run that stops at its limit tests only some mutants, so its result is partial evidence.
* cargo-mutants copies the tree with the ignore rules, the person's global ignore file included.  A tracked file that a rule matches is missing from the copy.  `no_tracked_file_is_ignored` finds such a file with the rules of the machine that runs it, and with a global rule for `Cargo.lock`.  CI has no global rules, so it sees only that one.
* The baseline can fail on a slow test.  In M5, 5 of the 8 case runs that were not dry runs had `classic_drums_beat_the_raw_mix` (`xtask/tests/m8_hpss.rs`) in the baseline.  It timed out in the unmutated tree in 3 of them (`m9`, `m2` and `m10` run 1), and passed in 2 (`docs/reviews/replay.md`).  Where it timed out, no mutant ran, and the line was `none: ...; exit 4: the baseline failed`.  It also failed in rounds 1 and 2 of the 48k audio branch.  Since 2026-10-24 the run builds with the optimised `mutants` profile in `Cargo.toml`.  Then the baseline of that branch's whole diff passed, and 98 mutants ran in 16 minutes.

* Fix: the validator decides each missed mutant (phase 3, M3).  Phase 6 reads the log.  If a slow test makes the baseline time out again, time the tests under the `mutants` profile first.  A replay of a review on a commit before the profile came in has no `[profile.mutants]`.  So remove the `profile` line from the copied config, for example with `sed '/^profile = /d'`.  Else cargo-mutants stops with "profile `mutants` is not defined", and the line says `exit 4: the baseline failed`.

### TD-34: the review agents' guard matches text

This entry concerns GIT-1, and the read-only rule of the review agents.

`.claude/hooks/read-only-bash.sh` is a PreToolUse hook on Bash for the review agents (harness phase 3, M2).  It refuses a command that would change the repo, a remote or a published package, by `git`, `cargo`, `gh` and a short list of other commands.  It matches the command text, so it stops mistakes, not an agent that tries to escape, as TD-28 says of the deny rules.  It fails closed for `git` subcommands only.  `the_bash_guard_refuses_writes` gives each rule of the guard one case or more, and `the_bash_guard_explains_each_refusal` checks the whole line of a refusal.

These pass the guard.  Each item gives examples only, and the list is not complete:

* `git --exec-path=`, and `git grep -O`, which starts a pager program;
* `find -exec` and `find -delete`;
* `$(...)` inside double quotes, `$'...'` quoting, `$"..."` quoting such as `$"git" commit`, and an unquoted `$(...)` as an argument, such as `git -C $(pwd) stash`;
* a redirection before the command, such as `2>/dev/null git commit`;
* a command in a variable, a function or an alias, such as `function f { git commit -m x; }; f`;
* a prefix option that takes a value, such as `env -u X git commit` or `exec -a x git commit`, and `env -S "git commit -m x"`;
* an array element before the command, such as `a[0]=1 git commit -m x`, which bash runs as an assignment and a command;
* `rtk` subcommands such as `rtk run` and `rtk err`;
* `cargo new` and `cargo init`, which write a new package;
* an xtask command in a form that `names_xtask` does not match, such as `cargo run -p xtask@0.1.0`, `-p path+file:///.../xtask`, `-mxtask/Cargo.toml`, and `cd xtask` before `cargo run`.  The deny rules in `.claude/settings.json` still refuse `cargo r* publish*` (TD-28);
* other variables, such as `CARGO_*` and `RUSTC_WRAPPER`;
* interpreters, such as `perl -e` and `awk`, and wrappers, such as `sudo` and `nice`;
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

The `review-rounds` skill (harness phase 3, M3) runs `xtask review` with no approval only in the turn in which it loads.  The notices of the background reviewers and of the mutants run start new turns.  So in the default permission mode, `review check` of step 8 asks a person for approval once in each round.  A late `review mutants` can ask too.  The baseline review runs no `xtask review` command.

Since M4, the log counts these late review commands in their own field and cell.  Since M6, an approval is not an intervention.  Step 11 asks the person for the approvals of each round, and the log records them as cost (TD-38).  An orchestrator in a `claude -p` run, in auto mode or in `dontAsk` mode does not ask (TD-38).  It writes 0, except in a `-p` run where a host or a prompt tool answers prompts.  There it writes the approvals that the run records, or `not counted`.  So the approvals that the skill forces do not fail D4 of `plans/completed/harness-phase-3.md`.  The count of late review commands equals their approvals only in the default permission mode.

* Fix: a project allow rule for `Bash(target/debug/xtask review *)` removes the approval.  That rule widens a permission of the project, so the maintainer decides (the Decision log of `plans/completed/harness-phase-3-m3.md`).

### TD-37: a branch has one set of review rounds

This entry concerns no rule.  It concerns the log of the review rounds.

`CLAUDE.md` tells the agent to run review rounds 1, 2 and 3 after each change that it implements.  But the `review-rounds` skill keeps one log for each branch, `<run>/log.md`.  So a branch has one set of rounds (`plans/completed/harness-phase-3.md`, "How the work runs").  Round 1 of a second set refuses the run: "the run folder holds a log: copy it out, or remove it".  If the agent removes the log and starts again, step "After round 3" writes over the `<branch>.md` copy in the reviews folder.  No doc tells the agent what to do for a branch with more than one cycle of implementation and review.

* Fix: the orchestrator or the maintainer decides how such a branch keeps the log of each set.

### TD-38: the review log cannot count a person's approvals

This entry concerns no rule.  It concerns the measure of D4 of `plans/completed/harness-phase-3.md`.  In the default permission mode, a command of a reviewer, a validator or the baseline can ask a person for approval.  The main agent's writes of the findings file and the log can ask too.  The main agent does not see those approvals.  So step 11 of the `review-rounds` skill asks the person for the approvals of the round and of the baseline.  In an interactive session, auto mode can also fall back to a prompt that waits for a person.  An orchestrator that runs the skill then logs 0 with the mode, and does not see such an approval.  It marks that 0 as a lower bound.  The log records the approvals as cost, not as interventions.  The counts rest on the person.  A person can miscount, or cannot tell the baseline's approvals from the round's, and then gives one total.  The maintainer reads the counts at the gate.

* Fix: count the approvals from Claude Code's own record of approvals, if one becomes available.  Until then, the person gives the counts.

### TD-39: much of xtask's WebDriver client, e2e scenarios and browser runners has no `cargo test` coverage

Much of `xtask/src/webdriver.rs`, `xtask/src/e2e.rs`, `xtask/src/web_test.rs`, `xtask/src/serve.rs` and `xtask/src/roformer_cli.rs` runs only against a live browser, a driver or the model weights (VERIFY-1).  The checker found no `cargo test` that kills these mutants.  The checker of M5 read the code on `main` and ran no mutant, so a bullet marked "(not run)" rests on that reading only.

Part B of `plans/active/stream-from-storage-catchup.md` (2026-10-28) handled the deferred rows of M3 to M6.  `xtask/tests/e2e_cli.rs` runs `e2e smoke`, `separate`, `memory-budget`, `stored-stems` and `old-session` with a `CHROMEDRIVER` that is not a file.  Each run must fail on the driver, before a browser starts.  So a scenario body or an e2e `run` that returns `Ok(())` fails a test.  The 4-hour wait for the model is the constant `MODEL_RUN_SECS`, with a test.  The waits of `e2e decode` and the model wait of `memory-budget` use `holds_or_error`, with a test.  `roformer_refuses_bad_input_before_the_model` runs `xtask roformer`.  `StemApp::audio_stats_lines` and `StemApp::open_folder` have native tests.  What is left, each with its reason:

* the scenario bodies after the driver starts, such as `storage`, `smoke`, `bench`, `separate`, `decode` (with `drop_and_open`), `playback`, `old-session`, `memory-budget` and `stored-stems`: their steps and waits need a browser (not run).  CI's `browser-tests` job runs `smoke`, `decode`, `playback`, `old-session`, `audio-resume`, `storage` and `flac`.  `separate`, `memory-budget`, `stored-stems`, `bench`, `selftest` and `worker-check` also need a GPU and the model, which CI does not have, so a person runs them.  Their checks are pure functions with unit tests (`all_from_storage`, `stored_stems_checks`, `flac_checks`, the growth checks, `old_session_checks` and others).  For example, deleting the `!` of the last `if !ok` in `memory_budget` still survives `cargo test`;
* the `Page` methods, such as `wait_for`, `errors` and `click`, which need a live browser session (not run);
* `Driver::start` after the driver is found, `free_port`, `Session::execute` and the `set_viewport` wrapper, which need a driver (not run);
* `query`, which `parses_scenario_and_options` never calls (not run);
* `run` in `xtask/src/web_test.rs` and in `xtask/src/serve.rs`, which no test calls (not run);
* a request with no headers in `drip`, since the test always sends a Host header (not run);
* `run` in `xtask/src/roformer_cli.rs` after the model loads: the separation needs the model weights, which CI does not have.  Its refusals of bad input have a test;
* `main` in `app/src/main.rs` (native): `eframe::run_native` needs a window, and no test can open one.  Its folder argument is `StemApp::open_folder`, which has a test.  So a `main` that returns `Ok(())`, or that deletes the `viewport` field, survives `cargo test`.

`sends_json_and_reads_a_sized_reply` passes when the client reads to the end of the stream.  The fake closes the socket after 30 s, within the test's 120 s (not run).

Source: `docs/reviews/replay.md`, case `pr9`, finding 1-1 and each `pr9` mutant still on `main`.  The review logs of M3 to M6 of the branch `iphone-crashes-2026-10-25` (`docs/reviews/iphone-crashes-2026-10-25-m3.md` to `-m6.md`), with a `catch-up` row for each deferred finding.

* Fix: a fake driver for `Driver` and `Session`, and a time limit under 30 s on the sized-reply test.  Add unit tests of the pure parts of the scenarios.  For the scenarios that need the model, a GPU runner in CI.

### TD-40: gaps in the RT-1 tests

The tests of RT-1 count allocations, and do not check all that the stretchers and the player do.  The checker of M5 read the code on `main` and ran no mutant, so a bullet marked "(not run)" rests on that reading only:

* No test reaches two branches of `app_stretcher_problems`: `"default"` in the features of an `engine =` line, and a manifest with no `engine =` line.
* `a_stretcher_keeps_its_warm_up_promise` counts allocations only.  No test checks the output of `WsolaStretcher::can_pull` for a push over 256 frames (not run).
* No test bounds the size of the warm-up push in `WsolaStretcher::warm_up` (not run).
* The loop tests check only that the playhead stays inside the loop.  A wrong comparison in `Player::drop_oldest_anchor_when_full` shows the loop start before each wrap, and passes (not run).

Source: `docs/reviews/replay.md`, case `m11`, findings 1-1, 1-8, 1-10 and 1-14.

* Fix: add a breach case for each branch of `app_stretcher_problems`.  Check the stretcher's output and the warm-up size.  Test the playhead before a loop wrap.

### TD-41: gaps in the GPU-1 limit checker's tests, and docs that claim more

`every_kernel_fits_the_default_limits` and its tests do not cover all of the checker (GPU-1, DOC-1):

* `every_layout_matches_its_shaders` pairs shaders, layouts and entry names from its own hand copy, not from `Kernels::new`.  The last sentence of its doc comment claims more than it checks: "The entry points that `Kernels::new` names exist."  The test checks a hand copy of the entry names.
* No test covers the count of read-only storage buffers, the y-axis limit, or a breach of the invocation limit that only z causes.

Source: `docs/reviews/replay.md`, case `m2`, findings 1-2 and 1-4.

* Fix: take the pairs from the pipelines that the code makes, and add the three missing cases.

### TD-42: weak tests of the e2e message-type scan

`e2e_messages_use_protocol_types` (PROTO-1) has tests that do not fail on some wrong scans.  The checker of M5 read the code on `main` and ran no mutant, so a bullet marked "(not run)" rests on that reading only:

* No test has a `SCRIPT_ONLY_TYPES` entry that only `xtask/src/e2e.rs` uses.  So a stale check that reads only the JavaScript passes.
* No test has `["type"] <=` in Rust, or a macro other than `json!` after `"type":` (not run).
* Both `includes` cases put the misspelt type second.  So a scan that skips the first string passes (not run).
* `js_is_lexed` passes when `js_tokens` counts the lines of a `/*` comment wrongly.  The checker ran a scratch copy of `js_tokens` with this mutant.

Source: `docs/reviews/replay.md`, case `m10`, outside findings 1-3, 1-11, 1-12 and 1-13, and finding 1-10, which matches `m10-2`.

* Fix: add each missing case to the tests in `xtask/src/rules.rs`.

### TD-43: the file picker path has no automated check

`pick_song` keeps each chosen `File` and saves it as a Blob, but no automated check runs it (VERIFY-1).  WebDriver cannot drive the browser's file picker.

Source: `docs/reviews/replay.md`, case `m18a`, finding 1-2.

* Fix: do a hand check in Chrome for each PR that changes the picker path.  Record it in the PR.  Stop when a browser test can set the files.

### TD-44: the review log's tokens are final contexts, not the tokens used

This entry concerns no rule.  It concerns the measure of D4 of `plans/completed/harness-phase-3.md`.  The `review-rounds` skill takes each agent's tokens from its `<usage>`, which is the size of the agent's final context.  The Tokens cells of the `## Agents` table and of the rounds table add these figures: "Tokens: the sum of the tokens of those agents".  D4's ratio compares round 1 with the baseline on these sums.

The final context of an agent is lower than the tokens that it used, by a ratio that differs by agent.  In M5, that ratio ran from 2.5 to 26.2 in case `m2`, and from 3.7 to 18.4 in case `m18a`.  Reviewers have high ratios, and validators low ones.  So D4's ratio of summed final contexts can misstate the ratio of tokens used by several times, by the mix of agents.

In `-p` mode, the result's `modelUsage` gives the session's tokens used.  For example, the session of `m18a` used 6,266,253 tokens, and its main agent used 758,710 of them.  The final contexts of its seven subagents sum to 391,169.  The source of these figures is `## Cost` of `docs/reviews/replay.md`.  The session's 6,266,253 comes from `modelUsage`.  The main agent's 758,710 counts its `message.usage` in the M5 transcript, once for each message ID.  The 391,169 is a sum of final contexts.  For `m2` and `m18a`, the sums of each agent's `message.usage` are about 1% below `modelUsage`.

D4's token bar has no reliable measure in the log.  The maintainer decides how to measure it at the gate.

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

* Fix: phase 5 runs the reviewers in a sandbox, in a mode without the classifier.  Until then, a person runs such a mutant by hand when a test of the harness is in doubt.

### TD-51: engine's tests do not run on wasm32

This entry concerns VERIFY-1 for `engine::resample`, which runs on wasm32 in the page.

On wasm32 a `usize` has 32 bits.  So `i * M` in `usize` overflows after about 10 minutes of 48 kHz audio.  `output_len` and `Position` work in `u64`, and `the_length_and_the_position_do_not_overflow_32_bits` checks them natively.  But `xtask web-test` runs no engine test on wasm32.  So a later `usize` product inside `Filter::apply` passes every test (48k audio review round 2, finding 2-1).  The block converter `Resampler` steps the same `Position` and counts its input in `u64`, and has the same gap.

* Fix: run engine's tests on wasm32, for example under `wasm-bindgen-test-runner` in `xtask web-test`.  Until then, no check covers this.

## Known breaches

### TD-52: the FLAC encoder writes a wrong minimum block size, and no 24 bits

This entry concerns DEPS-1.  It is a workaround of a defect in a dependency.  CLAUDE.md asks to record each workaround.  It breaks no rule.

`flacenc` 0.5.1 gives the short last block as the minimum block size in STREAMINFO (`update_frame_info`).  RFC 9639 section 8.2 excludes the last block.  Symphonia then reads the stream as variable-blocksize, and refuses every frame.  So four places set the block sizes again after `flacenc`: `flac` in `worker/src/web.rs`, `compressed` in `engine/src/test_flac.rs`, `flac_16` in `xtask/src/e2e.rs` and `flac` in `app/tests/web.rs`.  `flacenc` also has no 5-bit Rice parameters and no escape codes, so it cannot write 24 bits; a saved FLAC has 16 (`plans/completed/flac.md`, Surprises).

* Fix: remove the overrides when `flacenc` writes the minimum as RFC 9639 says.  For 24 bits, use an encoder that can write them, such as one in this repo.  Until then, `a_flac_decodes_to_the_stem` fails if the worker's override goes.

### TD-53: parts of the page's save and storage paths have no test

This entry concerns VERIFY-1.

The worker's MP3 and FLAC reach the page as `Event::Mp3` and `Event::Flac`.  The arms of `App` that handle them call `store::download` and `store::download_blob`, which make an object URL and click a link.  No test reaches these arms or functions.  The separator tests check that each event keeps the worker's buffer or `Blob` (`an_mp3_reply_reaches_the_page_as_sent`, `a_flac_reply_reaches_the_page_as_sent`), and `e2e flac` checks the worker's file.  The "FLAC encoding" and "MP3 encoding" error labels have no test either.  `encode_flac_asks_the_worker_for_a_flac_of_the_arrays` and `encode_mp3_asks_the_worker_for_an_mp3_of_the_arrays` check the requests.  `start_save` and `step_save` in `app/src/lib.rs` have two browser tests.  `a_lane_added_during_a_save_keeps_the_save` runs a FLAC save with no worker to its end.  `a_save_whose_read_fails_says_so_and_makes_no_file` runs FLAC and WAV saves whose read fails.  `save_menu` has no test.  The WAV arm that downloads a file has no test.  The FLAC and MP3 arms that post to a running worker have no test.  They are web only, so the native UI tests cannot reach them.  The sinks and the job have tests (`app/src/export.rs`).  `load_song` drops the mix lane's follower and the spectrogram job before it makes the new lanes, so an old job does not keep the old song's samples (MEM-4).  That order matters only on the web, where a job can run over several frames, and no test checks it.  `store::usage` has no test.  Source: `plans/completed/flac.md`, review rounds 1 to 3 (findings 1-9, 2-2, 3-5 and 3-6), and `plans/completed/mix-download.md` (M3 and review round 3).

* Fix: browser tests that spy on the link that `store::download` and `store::download_blob` click, for each event and for a WAV save.  A browser test that spies on the worker's request for a FLAC or MP3 save.  A test of the peak memory of `load_song` while a mix job runs.  A test of `usage`.

### TD-54: memory grows with the song, so a long song can stop the tab on an iPhone

This entry concerns MEM-1 and MEM-4.

This paragraph gives the state before M3.  On 2026-10-25, a song of about 5.1 minutes stopped the tab on an iPhone 17 Pro at chunk 2 of 48.  iOS logged `exceeded mem limit: ActiveHard 2048 MB` for the WebContent process.  The page makes all six song-length stems when the first segment arrives (`Assembly::place`), about 650 MB for that song.  The page's song and mix, the worker's mix and the stems grow by about 190 MB per minute of song.  The fixed part is at most about 1720 MB, so at most about 1.7 minutes fit.  The 30-second demo song separates.  Source: `plans/active/stream-from-storage.md` (Context).

M3 of that plan (2026-10-25) removed the page's song-length stems during a storage-path separation.  The worker writes the stems to the session's files, and `Assembly` stays empty.  What is left:

* M4 (2026-10-26) removed the worker's copy of the whole mix on the storage path.  The page writes the mix to a session file in blocks.  The worker reads it one chunk at a time.  Since M5 (2026-10-26), the decode worker decodes a stored song in blocks into session files, so the decode holds no song.  Since M6 (2026-10-27), the page plays from browser storage each lane that has a complete session file, through a read-ahead buffer of at most 32 MiB (`engine::readahead`, `app::playback`).
* Since M7 (2026-10-27), the page reads no session file whole to show or play a stored song (`plans/active/stream-from-storage-m7.md`).  Each lane is its session file with its overview.  The mix lane's waveform, the spectrograms, the saves, the mix write and the memory path's input read the lanes in blocks, and classic drums of a stored song runs in the decode worker.  `e2e old-session` checks that the page's WebAssembly memory after a restore stays under 64 MB and does not grow with the song.  What is left:
  * A save keeps its whole file in the JS heap until M8.  A WAV keeps its bytes.  A FLAC or MP3 keeps two song-length `Float32Array`s (`JsChannels::new(frames)` in `app/src/export.rs`), about 635 MB for 30 minutes.
  * Nothing asserts the whole tab's budget, and there is no iPhone evidence, until M10.
  * An open that decodes on the page holds at most the memory path's limit for its lanes (decision 0020).  A song of many sources still decodes on the page (TD-58).
* The memory path still makes six song-length stems.  It takes a song without a session, or a song when storage has too little free space.  It also takes a song when storage refused the mix or the stems, or the worker cannot read the mix file.  For a separation, and for an open that decodes on the page, the page refuses a song longer than the memory path's limit for its lanes: 100 seconds (`app::MEMORY_PATH_MAX_FRAMES`) for one source, and less for more lanes (`app::memory_path_frames`, decision 0020).
* Between M3 and M7, no build went to the published site, because the page read a stored song back whole.  Since M7 that read-back is gone, so publishing no longer has that risk.  What is left are the memory costs of a save and of an open that decodes on the page.  Publishing is a person's step (GIT-1).

* Fix: `plans/active/stream-from-storage.md` moves the song, the mix and the stems to browser storage (decision 0015).  Its M10 asserts that the peak does not grow with the song.

### TD-55: Safari can miss a mix file written again during a separation

The page never writes `mix.f32` while a separation runs (D6 of M4).  The worker checks this too.  After the last chunk, it compares the file's size and `lastModified` with their values at the open.  Chrome also refuses any read after a write.  Safari reads the bytes on disk now, and gives `lastModified` in whole seconds (`references/browser-quirks.md`).  So in Safari, a write of the same size in the same second as the open is not seen.  The stems then come from mixed samples.

Source: review round 3 of M4 on the branch `iphone-crashes-2026-10-25`, finding 3-3.  `the_end_check_sees_a_mix_file_written_again_at_its_size` tests the check alone.  `a_mix_file_written_again_after_the_last_read_ends_with_unstored` reaches the check through a separation, in Chrome too (finding 4-1 of review round 4 of M4).  The write comes over a second after the open, so Safari can also see it.  A write that makes the file shorter fails a read first (`a_mix_file_changed_during_the_separation_ends_with_unstored`).

* Fix: a hash of the mix, kept in `meta` and checked by the worker as it reads, or a lock on the file during the separation.  Only a fault in the page can write the file at that time.

### TD-16: the wire format has unchecked field names and silent defaults

`xtask/e2e/worker.js`, and the scripts in `xtask/src/e2e.rs` and in the Rust files under `xtask/src/e2e/`, repeat field names by hand.  Nothing checks them.  `e2e_messages_use_protocol_types` checks the message types, but only in the shapes it reads.  It does not read, for example, a `switch` and its `case`s, an assignment `m.type = "x"`, `d["type"]`, `"x" === d?.type`, a variable, a template literal or a regex literal, whose quote hides the rest of its line.  In Rust it does not read, for example, `.as_str() == Some("x")` or `json!("x") == r["type"]`.  In a plain Rust literal, a `\n` escape after a `//` comment hides the rest of the literal.  In a plain Rust literal, an escape other than `\\` and `\"` stays as a `\` and its text.  So an escape such as `\n` or `\t` joins its letter to the next word, and `\ntype` reads as `ntype` (`docs/reviews/replay.md`, case `m10`, finding 1-1).  Outside a JS string, a `\'` opens a string that hides the rest of its line.  So the check does not read a type such as `\'x\'`.  It reads no script outside `xtask/e2e/*.js`, `xtask/src/e2e.rs` and the Rust files under `xtask/src/e2e/`.  Some text in a type's place is not a message, such as `{ type: '{kind}' }` in a `format!` string, a `Blob`'s MIME type, or `"string"` in `typeof d?.type === "string"`: the check fails it, and `SCRIPT_ONLY_TYPES` takes it with a reason.  A reply missing a field decodes as `""`, `0` or `false`.  There is no version handshake between page and worker (PROTO-1).

The decode worker's messages (`stem_protocol::decode`, M5) refuse a missing or wrong field, so this entry concerns the stem worker's messages only.  `e2e_messages_use_protocol_types` checks each hand-written type against the union of both workers' kinds, so a decode kind in a script for the stem worker passes (review round 1 of M5, finding 1-16).

* Fix: fail decoding on a missing required field; a version in `Alive`.  Until then, the `wire-reviewer` agent looks for a field name or a default that one side changes and the other side does not.  It looks on each diff that routes it (TD-31).

### TD-45: the status of decision 0007 may not be a person's choice

Decision 0007 (`decisions/0007-no-allocation-in-the-audio-callback.md`) has the status "accepted (by the maintainer)", and an agent wrote it.  Finding m11 1-2 claims that the limits of its scope are the agent's choices.  The M11 heading of the phase 2 plan says "maintainer decision, made 2026-10-19".  The maintainer judges.  `CLAUDE.md`, "Working here", Decisions: agents write a record as `proposed`, and only a person accepts it.

Source: `docs/reviews/replay.md`, case `m11`, finding 1-2.

* Fix: the maintainer confirms the status, or sets it to `proposed`.

### TD-57: a loop moved behind the head can underrun at the old loop's start

This entry concerns the underrun count of M6 (D4 and D6 of `plans/active/stream-from-storage-m6.md`).

A loop set behind the head is a held seek: the player keeps its old loop until the run of the new loop's wrap is read.  The reader reads ahead along the loop of `SharedParams`, which is the new loop.  So the reader can give the slot of the old loop's start to another block.  If the old loop wraps before the jump, the stored lanes play silence there and count an underrun.  The jump comes after about one block read, so the gap is short.

* Fix: the player gives its own loop with `Hint::Reading`, and `wanted_blocks` follows that loop while a seek holds.

### TD-58: a song of many sources decodes on the page

This entry concerns MEM-1 and MEM-4.

Since C of `plans/active/stream-from-storage-catchup.md` (decision 0020), every open that decodes a song on the page keeps the memory path's limit for its lanes (`app::memory_path_frames`).  It reads each file's head first, and refuses a long song before it reads a whole file.  The decode stops at the limit.  What is left:

* D2 of `plans/active/stream-from-storage-m7.md` still sends a song whose lanes do not fit the read-ahead buffer at its minimum slots to the page (more than about 20 sources and no `stem` lines; `store_meta::open_formats` and `store_meta::fits_read_ahead`).  Such a song holds at least 28 lanes, so it opens only up to about 25 s, and a longer one is refused.
* A file whose head states no length is read whole up to its cap (`engine::decode::largest_file`): about 324 MB for a WAV or FLAC, and about 21 MB for an MP3, for a song of 100 s.  An MP3 whose ID3 tag is larger than the head counts as an MP3; a FLAC behind such a tag gets the cap of an MP3.  The page holds a file's bytes while it decodes that file.

* Fix: a read-ahead buffer that holds fewer lanes than the song shows, or a mix of the sources into fewer lanes, removes the D2 trigger.  A decode on the page from the file in blocks, as the decode worker does, removes the whole read.

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
* TD-24: closed by DSP-1's new list of types (harness phase 2, M7).  The list now names every type that `dsp` uses.
* TD-30: closed by `set_viewport` in `xtask/src/webdriver.rs` (harness phase 2, M17).  A read at once after a resize could still give the old size, so `xtask e2e` sometimes could not size Chrome's viewport.  Each try now decides on a read taken once the page's outer size equals the new window.  `settle` starts no new read after 2 s.  A read that gives the target is read again, and five tries correct a late read.  In the CI runs with the fix, every read at once was already right.  So only the unit tests show a late read handled: `the_viewport_fits_despite_a_stale_read_at_once` and `a_late_settled_read_is_corrected_by_the_next_try`.  The e2e log line `viewport: ...` shows the race if it comes back.
* TD-56: closed by M7 of `plans/active/stream-from-storage.md` (D12 of `plans/active/stream-from-storage-m7.md`).  One counter gives a token to every open (a drop, a pick, a Storage open, the startup restore and Demo).  Only the newest open shows its song, and only `apply_inbox` makes a session current.  `a_late_drop_does_not_show_over_a_newer_song`, `a_late_drop_does_not_show_over_demo`, `two_opens_that_end_in_one_frame_show_the_newer` and `a_stale_restore_on_the_page_does_not_make_its_session_current` check it.
