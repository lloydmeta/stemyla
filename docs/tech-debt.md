# Tech debt

Known gaps in what enforces the rules, and known breaches of them, by ID.  Each entry names the rule or the measure it concerns, the evidence, and the fix in view.  To close an entry, delete it in the commit that fixes it.  In the same commit, add its ID under `## Closed`, with what closed it.  An ID is never reused.  Plans, decisions and review logs may still name a closed ID; `docs-check` fails it in other docs.  Add new entries above `## Closed`.

Entries found on 2026-10-19 by a survey of the code for the harness plan (`plans/active/harness-engineering.md`); the plan's phase 2 closed most enforcement gaps (`## Closed`).

## Enforcement gaps

### TD-3: DSP-1 is checked only for the likely breaches

`dsp_is_beginner_style` scans the tokens in `dsp/` for the likely breaches.  It does not read types, so a type outside DSP-1's list passes.  A closure with no arguments (`|| x`) passes, because it has the same tokens as `||`.  A construct that the scan does not look for, such as `match` or `loop`, passes.  A glob import, such as `use std::mem::*`, hides the names that it brings in.  A `-A clippy::all` in `RUSTFLAGS`, in a `config.toml` in `.cargo/`, or in `[workspace.lints]`, passes too, for every crate.

* Fix: review; no check is planned.  `dsp` is small and seldom changes, so a full allowlist costs more than it gains (`plans/completed/harness-phase-2.md`, M7).

### TD-7: no licence policy and no allowlist of registries

`young_crates_are_pinned` and `third_party_lists_what_ships` check the pins and the licence list (DEPS-1).  Nothing fails a licence that is not permissive.  A crate from a registry other than crates.io fails only if it is in a tree of `THIRD-PARTY.md`, since `third_party_lists_what_ships` cannot read its line.  A vendored copy of a crate that is made a workspace crate leaves both checks; only a reviewer sees it.

* Fix: `cargo deny` for licences and registries.  It is a new tool, so vet it first (DEPS-1).

### TD-8: some evidence runs nowhere, and required skips rest on a person

`STEMYLA_REQUIRE` turns a skip into a failure (M3 of `plans/completed/harness-phase-2.md`), but these gaps stay (VERIFY-1, GPU-2, MEM-3):

* The PyTorch comparisons run nowhere, because neither CI nor the maintainer's Mac has the fixtures.  Fix: the maintainer makes them on the Mac with `tools/roformer_export.py` and the PyTorch checkpoint.  Then `STEMYLA_REQUIRE=fixtures` is the evidence for the CPU, and `STEMYLA_REQUIRE=gpu,fixtures` for the GPU.
* GPU and fixture evidence holds only when a person sets `STEMYLA_REQUIRE`, and nothing checks that a PR's evidence did.  Fix: phase 4's evaluator sets the variable in its contract.
* `every_skip_can_be_required` sees only messages that start with `skipping` or `skipped`, in any case, so a silent early `return` passes it.  Fix: phase 3's reviewers look for it.
* The three `timestamps` tests, and the kernel check in `forward_matches_pytorch`, run nowhere.  CI excludes them, and the Mac's adapter has no `timestamp-query`.  Fix: run them on an adapter with `timestamp-query`, and name it in the PR.

### TD-10: GPU-3 is not checked

Nothing records which adapter a kernel passed on, and any new `TILES` entry becomes a calibration candidate on every user's GPU.

* Fix: log `isFallbackAdapter` and the adapter name in selftest and e2e results; a GPU reviewer that requires a named real GPU in the PR.

### TD-11: MEM-4 is never asserted

The worker logs its wasm memory and live GPU bytes and the page its wasm memory, but no test asserts a total, and the page's WebGL textures and the JS heap are not counted.

* Fix: a tab total (page wasm, worker wasm, GPU live bytes) in an e2e scenario that separates a song and fails over a budget.

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
* Fix: phase 3's reviewers look for all four.  A later check could ban `#[path]` and `include!` with a path outside the crate.  It could check that each workspace crate's `targets[].src_path` is in its own folder.  It could also fail a tracked symbolic link, which `git ls-files -s` shows with mode `120000`.

### TD-25: RT-1's allocation test runs natively only

`the_callback_does_not_allocate_after_warm_up` does not count the WebAssembly build, cpal's hosts, or the closure in `engine/src/live.rs`.  That closure calls `Schedule::next`, whose fields are all `Copy`.  On the web, the first `web_time::Instant::now` in `render_interleaved` fills a `thread_local`.  A zero count is evidence, not proof, since the optimiser may remove an allocation.  The bench-only stretchers (`timestretch`, `cute-dsp`) are not covered.  `the_app_ships_only_these_stretchers` reads the text of `app/Cargo.toml` only, so it does not see a stretcher that another crate turns on through feature unification (RT-1).

* Fix: none planned; phase 4's evaluator may count allocations in the browser.

### TD-26: MEM-1's lint sees only `to_vec`

Clippy fails `slice::to_vec` in `app`, `stem-worker`, `stem-protocol`, `engine`, `roformer` and `roformer-gpu`, and `to_vec` on a `Uint8Array` or a `Float32Array` in the first three (MEM-1, MEM-2).  It does not see:

* other forms of copy: `to_owned`, `Vec::from`, `Box::from`, `into`, `concat`, `collect`, `extend_from_slice`, `copy_to` into a new buffer, and `clone` of a `Vec`;
* `to_vec` on the other typed arrays, such as `Int16Array`, and a wasm-bindgen import that returns a `Vec<u8>`;
* a `#[wasm_bindgen]` export that takes a `Vec<f32>` or a `&[f32]`, which copies a JS array in;
* an `#[allow]` in place of an `#[expect]`, as clippy's own help line suggests, and a reason that does not say why;
* a new `to_vec` in a statement that already has an `#[expect]`, and a new test in a test module with a module-level `#[expect]` or `#![expect]`;
* crates whose `clippy.toml` lacks the MEM-1 paths: `synth`, which makes the demo song in the web app, and any new crate.  `dsp` is covered by DSP-1's method list; `xtask` and `test-support` do not ship;
* a path that no longer resolves, except through the canaries, which prove their paths only while the clippy steps keep `--all-targets`.  Nothing fails if the `wasm32` step goes.

* Fix: none planned.  Phase 3's memory reviewer looks for a new copy of any form.

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

* Fix: none planned.  Phase 3's GPU reviewer looks for GPU memory that is not destroyed.

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
* whether Claude Code refuses anything: `claude_settings_hold_the_deny_rules` keeps the rules in the file, and M14's step 3 showed the refusals by hand once; a rule taken out of both the file and `REQUIRED_DENY_RULES` passes every test, and only review sees it.

* Fix: re-run M14's step 3 (`plans/completed/harness-phase-2.md`) after a change to the deny rules, or an upgrade of Claude Code or RTK.  After another change to `.claude/settings.json`, run the 9 commands in M15's step 2.  On GitHub, the `main` ruleset refuses a direct push to `main`, and the `gh-pages history` ruleset refuses a rewrite of `gh-pages` (TD-20).  Nothing on GitHub refuses a fast-forward push to `gh-pages`, a PR merge with the admin override, or `xtask publish`.  Phase 5 adds the agents' own identity.

### TD-29: no check stops a change that makes a snapshot pass

A larger `threshold` or `max_failed_pixels`, in a `kittest.toml` or in `SnapshotOptions`, passes a real change of the UI.  egui_kittest reads a `kittest.toml` in any folder above the test, also outside the repo.  A PNG rewritten with `UPDATE_SNAPSHOTS=1` and committed passes.  dify does not count a pixel that it finds anti-aliased, and the default passes a change of 1 colour level (VERIFY-1).

`no_build_file_updates_snapshots` reads only the build files in the repo.  These pass it:

* `std::env::set_var` in a test, or a variable from `xtask`;
* a repository variable on GitHub;
* `[env]` in a Cargo `config.toml` outside the repo;
* `env` in a person's own Claude Code settings.  `env` in `.claude/settings.json` fails `claude_settings_hold_the_deny_rules`, which allows only `$schema`, `permissions` and `hooks`;
* `#[ignore]` on a snapshot test.

* Fix: review; a changed PNG or threshold is a UI change, and the reviewer looks at it.  Phase 3's reviewers check for it.  No check is planned.

### TD-31: the route of `xtask review` is a short denylist, from the branch's own code

`xtask/src/review.rs` chooses the reviewers of a diff from its paths and from content signs.  The content signs are a short denylist, not a full list.  A sign matches as a whole token only.  So `create_buffer` does not match `create_buffer_init`, which `roformer-gpu/src/lib.rs` calls.  A sign counts only on an added line of a Rust file outside a `tests` folder and outside the reviewer's own paths.  A copy or a GPU buffer that a macro, an alias or a helper makes routes nothing.

The route comes from the branch's own `xtask`.  So a branch can change the route, or remove the rule that a change to `xtask/src/review.rs` routes every reviewer.  `xtask review start` warns when the branch changes `xtask/src/review.rs`, and when the binary is older than that file.

* Fix: review; `reviewer` reads every diff.  Phase 5 routes with the base branch's `xtask`.

### TD-32: `xtask review check` sees only what its record holds

`xtask review check` compares the repo with the state record of a round.  The record holds `HEAD`, the status, the stash, the branches, the tags, every other ref (such as remote-tracking refs, notes and replacements), the remotes and the worktrees.  It also holds a hash of the git config, the hooks, the local Claude Code settings in the repo and the person's own Claude Code settings in the home folder.  The check does not see other changes, for example:

* a change to an ignored file other than the local Claude Code settings, or to a file under `target/`;
* a change to the reflog, the global git config, or the index of another worktree;
* a change to a file outside the repo, other than the person's own settings.

A change to the person's own settings during a round, by a person or by Claude Code, fails the check.  With a detached `HEAD`, the name of the run folder holds the short SHA.  So after a fix commit, round 2 finds no record.

The `review-rounds` skill (harness phase 3, M3) stops a round when `review check` fails.  But a change that the record does not hold passes the check, and the round goes on.

* Fix: the person examines the change, and starts the round again.  With a detached `HEAD`, start again at round 1.

### TD-33: `xtask review mutants` tests only part of a diff

`xtask review mutants` runs cargo-mutants on the Rust files of a round's diff.  It does not test some changes, and it reports some mutants that no native test can catch:

* cargo-mutants builds and tests natively.  A `wasm32`-only block in a shared function shows as a missed mutant.  So does a new `wasm32`-only function, until `exclude_re` in `.cargo/mutants.toml` names it.  A function with a native variant too, such as `base_latency` in `engine/src/live.rs`, stays in.
* A diff of tests only, WGSL or JavaScript gives no mutants.
* A run that stops at its limit tests only some mutants, so its result is partial evidence.
* cargo-mutants copies the tree with the ignore rules, the person's global ignore file included.  A tracked file that a rule matches is missing from the copy.  `no_tracked_file_is_ignored` finds such a file with the rules of the machine that runs it, and with a global rule for `Cargo.lock`.  CI has no global rules, so it sees only that one.
* The baseline can fail on a slow test.  In M5, 5 of the 8 case runs that were not dry runs had `classic_drums_beat_the_raw_mix` (`xtask/tests/m8_hpss.rs`) in the baseline.  It timed out in the unmutated tree in 3 of them (`m9`, `m2` and `m10` run 1), and passed in 2 (`docs/reviews/replay.md`).  Where it timed out, no mutant ran, and the line was `none: ...; exit 4: the baseline failed`.

* Fix: the validator decides each missed mutant (phase 3, M3).  Phase 6 reads the log.  For a slow test, give the baseline a longer timeout, or leave the slow test out of the mutants run.

### TD-34: the review agents' guard matches text

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

A review agent runs `cargo test` and `cargo build` on the diff.  `xtask review mutants` runs cargo-mutants on it.  Each of them runs the diff's `build.rs`, proc macros and tests as the person.  That code can read the `gh` token and the cargo token, and use the network.  The guard (TD-34) reads only the text of a Bash command, not what the built code does.

* Fix: phase 5 runs reviews in a sandboxed job with no token.

### TD-36: in the default permission mode, each review round has an approval that the skill forces

The `review-rounds` skill (harness phase 3, M3) runs `xtask review` with no approval only in the turn in which it loads.  The notices of the background reviewers and of the mutants run start new turns.  So in the default permission mode, `review check` of step 8 asks a person for approval once in each round.  A late `review mutants` can ask too.  The baseline review runs no `xtask review` command.

Since M4, the log counts these late review commands in their own field and cell.  Since M6, an approval is not an intervention.  Step 11 asks the person for the approvals of each round, and the log records them as cost (TD-38).  So the approvals that the skill forces do not fail D4 of the parent plan (`plans/active/harness-phase-3.md`, Decisions).  The count of late review commands equals their approvals only in the default permission mode.

* Fix: a project allow rule for `Bash(target/debug/xtask review *)` removes the approval.  That rule widens a permission of the project, so the maintainer decides (the Decision log of `plans/active/harness-phase-3-m3.md`).

### TD-37: a branch has one set of review rounds

`CLAUDE.md` tells the agent to run review rounds 1, 2 and 3 after each change that it implements.  But the `review-rounds` skill keeps one log for each branch, `<run>/log.md`.  So a branch has one set of rounds (`plans/active/harness-phase-3.md`, "How the work runs").  Round 1 of a second set refuses the run: "the run folder holds a log: copy it out, or remove it".  If the agent removes the log and starts again, step "After round 3" writes over the `<branch>.md` copy in the reviews folder.  No doc tells the agent what to do for a branch with more than one cycle of implementation and review.

* Fix: the orchestrator or the maintainer decides how such a branch keeps the log of each set.

### TD-38: the review log cannot count a person's approvals

This entry concerns no rule.  It concerns the measure of D4 of the parent plan (`plans/active/harness-phase-3.md`, Decisions).  In the default permission mode, a command of a reviewer, a validator or the baseline can ask a person for approval.  The main agent's writes of the findings file and the log can ask too.  The main agent does not see those approvals.  So step 11 of the `review-rounds` skill asks the person for the approvals of the round and of the baseline.  The log records them as cost, not as interventions.  The counts rest on the person.  A person can miscount, or cannot tell the baseline's approvals from the round's, and then gives one total.  The maintainer reads the counts at the gate.

* Fix: count the approvals from Claude Code's own record of approvals, if one becomes available.  Until then, the person gives the counts.

### TD-39: much of xtask's WebDriver client, e2e scenarios and browser runners has no `cargo test` coverage

Much of `xtask/src/webdriver.rs`, `xtask/src/e2e.rs`, `xtask/src/web_test.rs` and `xtask/src/serve.rs` runs only against a live browser or a driver (VERIFY-1).  The checker found no `cargo test` that kills these mutants.  The checker of M5 read the code on `main` and ran no mutant, so a bullet marked "(not run)" rests on that reading only:

* the scenarios and their helpers, such as `storage`, `smoke`, `bench` and `separate`, which run only in a browser (not run);
* the `Page` methods, such as `wait_for`, `errors` and `click`, which need a live browser session (not run);
* `Driver::start`, `free_port`, `Session::execute` and the `set_viewport` wrapper, which no test calls (not run);
* `query` and `Opts::weights`, which `parses_scenario_and_options` never calls (not run);
* `run` in `xtask/src/web_test.rs` and in `xtask/src/serve.rs`, which no test calls (not run);
* a request with no headers in `drip`, since the test always sends a Host header (not run).

`sends_json_and_reads_a_sized_reply` passes when the client reads to the end of the stream.  The fake closes the socket after 30 s, within the test's 120 s (not run).

Source: `docs/reviews/replay.md`, case `pr9`, finding 1-1 and each `pr9` mutant still on `main`.

* Fix: a fake driver for `Driver` and `Session`, and a time limit under 30 s on the sized-reply test.  Add unit tests of the pure parts of the scenarios.  For the rest, CI's `browser-tests` job runs `smoke`, `audio-resume` and `storage`.  A person runs the other scenarios.

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

This entry concerns no rule.  It concerns the measure of D4 of the parent plan (`plans/active/harness-phase-3.md`, Decisions).  The `review-rounds` skill takes each agent's tokens from its `<usage>`, which is the size of the agent's final context.  The Tokens cells of the `## Agents` table and of the rounds table add these figures: "Tokens: the sum of the tokens of those agents".  D4's ratio compares round 1 with the baseline on these sums.

The final context of an agent is lower than the tokens that it used, by a ratio that differs by agent.  In `docs/reviews/replay.md`, that ratio ran from 2.5 to 26.2 in case `m2`, and from 3.7 to 18.4 in case `m18a`.  Reviewers have high ratios, and validators low ones.  So D4's ratio of summed final contexts can misstate the ratio of tokens used by several times, by the mix of agents.

In `-p` mode, the result's `modelUsage` gives the session's tokens used.  For example, the session of `m18a` used 6,266,253 tokens, and its main agent used 758,710 of them.  The final contexts of its seven subagents sum to 391,169.

D4's token bar has no reliable measure in the log.  The maintainer decides how to measure it at the gate.

* Fix: the skill and D4 stay as they are.  A later phase can read `modelUsage` where it exists.

### TD-46: in `dontAsk` mode, the agents' probe forms are denied

This entry concerns no rule.  It concerns the headless rounds of phase 5.  In `dontAsk` mode, the harness denied probe commands of the review agents in five forms: a heredoc, a multi-line command, `cd`, an env or variable prefix, and a `>` write.  No allow list admits a heredoc, a multi-line command or a `>` write (the Decision log of `plans/active/harness-phase-3-m5.md`).  For `cd` and a variable prefix, no run tried a wider allow list.  In M5 they were 11, 14, 14, 23 and 9 of the 126 denials of the six counted runs.  The evidence is the denial table in `## Threats` of `docs/reviews/replay.md`, and dry run 2 in the Progress of `plans/active/harness-phase-3-m5.md`.

* Fix: phase 5 runs the agents in a sandbox where Bash needs no allow list.  Or the agents write each probe in a form that an allow rule admits, such as a one-line `sed`.

### TD-47: a `-p` session ends while a background agent runs

This entry concerns no rule.  It concerns the headless rounds of phase 5.  A `-p` session ends when the main agent ends its turn, even while a background validator runs.  In M5, case `m11` run 1 ended with "The mutants validator is running.  After it ends, I will run `review check` and write the findings file."  The validator's notice is `stopped`, and no findings file was written.  The evidence is in `## Threats` of `docs/reviews/replay.md`, and in the Progress of `plans/active/harness-phase-3-m5.md`.

* Fix: the main agent waits for every background agent before it ends its turn.

## Known breaches

### TD-16: the wire format has unchecked field names and silent defaults

`xtask/e2e/worker.js` and the scripts in `xtask/src/e2e.rs` repeat field names by hand, and nothing checks them.  `e2e_messages_use_protocol_types` checks the message types, but only in the shapes it reads.  It does not read, for example, a `switch` and its `case`s, an assignment `m.type = "x"`, `d["type"]`, `"x" === d?.type`, a variable, a template literal or a regex literal, whose quote hides the rest of its line.  In Rust it does not read, for example, `.as_str() == Some("x")` or `json!("x") == r["type"]`.  In a plain Rust literal, a `\n` escape after a `//` comment hides the rest of the literal.  A `\n` or `\t` escape in a plain Rust literal joins its letter to the next word.  So `\ntype` reads as `ntype` (`docs/reviews/replay.md`, case `m10`, finding 1-1).  It reads no script outside `xtask/e2e/*.js` and `xtask/src/e2e.rs`.  Some text in a type's place is not a message, such as `{ type: '{kind}' }` in a `format!` string, a `Blob`'s MIME type, or `"string"` in `typeof d?.type === "string"`: the check fails it, and `SCRIPT_ONLY_TYPES` takes it with a reason.  A reply missing a field decodes as `""`, `0` or `false`.  There is no version handshake between page and worker (PROTO-1).

* Fix: fail decoding on a missing required field; a version in `Alive`.

### TD-45: the status of decision 0007 may not be a person's choice

Decision 0007 (`decisions/0007-no-allocation-in-the-audio-callback.md`) has the status "accepted (by the maintainer)", and an agent wrote it.  Finding m11 1-2 claims that the limits of its scope are the agent's choices.  The M11 heading of the phase 2 plan says "maintainer decision, made 2026-10-19".  The maintainer judges.  `CLAUDE.md`, "Working here", Decisions: agents write a record as `proposed`, and only a person accepts it.

Source: `docs/reviews/replay.md`, case `m11`, finding 1-2.

* Fix: the maintainer confirms the status, or sets it to `proposed`.

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
