# Harness phase 3, M1: `xtask review` routes a diff and guards a round

## Purpose

This is the milestone plan for M1 of `plans/active/harness-phase-3.md`.  It gives more detail than the parent plan's M1 section, which stays the criteria.  After M1, `xtask review` has three subcommands:

* `start` chooses the reviewers from a diff, records the state of the repo, and refuses a dirty tree.
* `check` fails when the state changed during a round.
* `mutants` runs cargo-mutants on the last round's diff, under a time limit, and prints one line.

M2 adds the agents that the route names.  M3's skill calls these commands with `` !`...` `` and in the background, so each refusal exits 3 and stops the skill.  No code that ships changes.

## Context and orientation

The state at the start of M1, on 2026-10-23, at commit `422ebe3` on branch `ai-agent-harness-phase-3`:

* `xtask/src/main.rs` dispatches on `"name" =>` arms and returns `anyhow::Result<()>`, so an error exits 1.  `xtask/src/lib.rs` lists the modules.
* `xtask/src/docs.rs` has `glob_matches` (`*`, `**`, `{a,b}`), `xtask_commands` and `guide_covers`.  `docs-check` fails a command that `docs/working-with-agents.md` does not name in a code span.
* `xtask/src/rules.rs` and `xtask/tests/rules.rs` hold the structural tests.  `xtask/tests/publish.rs` makes scratch repos in `std::env::temp_dir()` with the process ID in the name.  The workspace has no `tempfile` crate; M1 adds no crate (DEPS-1).
* `.cargo/` does not exist.  `.gitignore` ignores `target/` and `/.claude/worktrees/`.
* The repo sets `extensions.worktreeconfig = true`.  `git rev-parse --git-common-dir` gives `.git` in the main checkout.  `core.hooksPath` is not set.  Git is 2.50.1 (Apple Git-155).
* cargo-mutants 27.1.0 is installed (`cargo mutants --version`).
* The maintainer's global git config signs every commit.  A test that commits in a scratch repo passes `-c commit.gpgsign=false`, its own identity, and `--no-verify`.
* `engine/src/live.rs` is behind the `cpal` feature (`engine/src/lib.rs`), and has tests in `mod tests`, such as tests of `nth_start` and `is_fatal`.
* `ai-agent-harness-phase-2-ms11` is a local branch.  Its diff from `7cd8518` changes `.claude/rules/`, `CLAUDE.md`, `Cargo.lock`, `THIRD-PARTY.md`, `docs/`, `engine/Cargo.toml`, `engine/src/` and `engine/tests/realtime.rs`.
* PRs #13 to #33 are the 21 first-parent commits of `main` from `a9ace3a` (#13) to `fa41c9f` (#33).  #24 (`373e0ce`) is a Dependabot bump.  `git log --oneline --first-parent main` lists them.
* The highest tech-debt ID is 30.  `docs-check` fails a plan that names an ID that is not yet defined.  So this plan says "a new tech-debt entry", and each commit that adds an entry gives it the next free number.

Terms are as in the parent plan.  Also:

* Run folder: `target/review/run/<branch>/`, under the repo of the current folder (`git rev-parse --show-toplevel`).
* Round folder: `<run folder>/<round>/`.
* State record: `<round folder>/state.json`.
* Merge base: `git merge-base <base> HEAD`, recorded as `base`.
* `<since>`: the merge base in round 1; the `head` of round n-1's state record in round n.

## Plan of work

### How the team works

* The team leader writes this plan, runs the plan reviews and the three review rounds, and commits each Progress line.
* An implementer does steps 1 to 6.  It writes each test first, runs it, and records in Progress that it failed, with the failure in a few words.
* Before each commit: `cargo fmt --all --check`, each clippy command in `CLAUDE.md`, `cargo test --workspace --exclude roformer-gpu`, `cargo test -p roformer-gpu --lib` and `cargo run -p xtask -- docs-check` (GIT-2).  Sign each commit with `git commit -S`.  Doc changes go in the same commit (DOC-1).
* Each step ends with one signed commit or more, and a Progress line.
* Scratch worktrees go under `target/review/m1-<name>/` in the main checkout.  `target/` is ignored, and `review start` leaves out worktrees under `target/review/`.  Scratch branches are named `m1-scratch-<name>`.  Progress lists each one.

### The code layout

* `xtask/src/review.rs`, with `pub mod review;` in `xtask/src/lib.rs`.  Its unit tests are in `#[cfg(test)] mod tests` in the same file, so a change to them routes every reviewer too.
* `pub fn main(args: &[String]) -> i32` dispatches `start`, `check` and `mutants`, and returns the exit code.  `xtask/src/main.rs` gets the arm `"review" => std::process::exit(review::main(&args[1..])),` and `review` in its usage line.
* Exit codes: 0 for success; 3 for each refusal, each usage error and each error of its own, such as a failed `git`.  A message goes to stderr.  So no failure of `xtask review` exits 1, which a skill could read as a search with no match.
* Pure functions take text and values, and return values.  Thin wrappers run `git` and `cargo` with `std::process::Command`, in a folder that they take as an argument.  `main` passes the current folder.  A function that needs the home folder or the binary's time takes it as an argument, so tests pass their own.
* Every `git diff` call passes `--no-ext-diff --no-color --no-textconv --src-prefix=a/ --dst-prefix=b/`, and `git -c core.quotePath=false`.  So a person's `diff.noprefix`, `diff.mnemonicPrefix`, `diff.external` or `core.quotePath` does not change the output.

### Step 1: the route

* `pub struct Reviewer { name, include: &[&str], exclude: &[&str] }` and `pub const REVIEWERS: &[Reviewer]`, the six rows of the parent plan's table, in its order.  `docs-reviewer` excludes `docs/plans/**` and `docs/reviews/**`.
* `pub const ROUTE_ALL: &[&str]`: `xtask/src/review.rs`, `.claude/agents/**`, `.claude/skills/**`, `.claude/hooks/**`.  A path that matches one routes all six.
* `pub const MAY_MATCH_NOTHING: &[(&str, &str)]`: `**/build.rs` and `.mcp.json`, each with a reason ("no crate has a build script yet"; "the repo has no MCP config yet").
* `pub const SIGNS: &[(&str, &[&str])]`: the content signs of the parent plan, by reviewer.  Each sign keeps the TD or rule ID of the parent plan in a comment.
* Globs use `docs::glob_matches`.
* Input: `git diff -z --raw -M <range>` and `git diff -U0 -M <range>`.
  * Parse `--raw -z` into entries of old mode, new mode, status and one or two paths.  A rename (`R<score>`) and a copy (`C<score>`) have two paths.  Each path routes.
  * A file with mode `120000` on either side routes `supply-chain-reviewer`.
  * Parse the patch for added lines by file.  A file header runs from `diff --git` to the first `@@`.  Take the path from `+++ b/<path>`; `+++ /dev/null` is a deletion.  After an `@@` line, a line that starts with `+` is an added line, until the next `diff --git`.  So an added line `++x`, shown as `+++x`, is not a header.
* Signs: on an added line of a `.rs` file, when the file has no path segment `tests`, and the file is not in that reviewer's include globs.  A sign matches as a whole token.  If the sign starts with a letter, a digit or `_`, the character before it must not be one of those.  If the sign ends with one, the character after it must not be one.  So `into_owned` is not `to_owned`, and `include_str!` is not `include!`.
* Output: `Route`, a map from reviewer name to its reasons, in table order.  A reason is one of: `every diff`; `<path> (<glob>)`; `<path> changes the review harness`; `<path> is a symlink`; `` `<sign>` in <path> ``.
* An empty diff gives an empty route.  A non-empty diff always routes `reviewer`, with the reason `every diff`.
* `pub fn route(raw: &str, patch: &str) -> Route` is pure.  `fn diff_route(repo: &Path, from: &str, to: &str) -> Result<Route>` runs the two diffs.
* Tests, written first.  Each fails with its row, glob, sign or rule removed:
  * For each include glob of each row, a path that matches it routes that reviewer with that glob as the reason.
  * An excluded path (`docs/plans/active/x.md`, `docs/reviews/x.md`) does not route `docs-reviewer`.
  * A path in no row (`dsp/src/lib.rs`) routes `reviewer` only.
  * Each path of `ROUTE_ALL` routes all six.  One test per path.
  * Each sign, on an added line of `xtask/src/x.rs`, routes its reviewer.  One case per sign.
  * A sign inside a longer name routes nothing: `into_owned`, `my_to_vec`, `unsafely`, `include_str!` for `include!`, `copy_to_slice`, `xwgpu::`.
  * A sign adds no reason: in that reviewer's paths (`to_vec` in `engine/src/x.rs`); on a removed line; in `engine/tests/x.rs` and `xtask/tests/x.rs`; in a `.md` file.
  * A rename from `docs/x.md` to `engine/src/x.rs` routes `docs-reviewer` and `memory-reviewer`, made in a scratch repo with real `git`, so the parse of real `-M` output is tested.
  * Mode `120000`: a symlink added in a scratch repo routes `supply-chain-reviewer`.
  * The patch parse: an added line `++x` in the hunk is an added line, not a header.
  * An empty diff gives an empty route.
* Tech debt, in the same commit: a new entry for the route's gaps.  The content signs are a short denylist, not a full list.  The route comes from the branch's own `xtask`, so a branch can change it; `review start` warns (step 3), and phase 5 must route with the base branch's `xtask`.  Fix: review, then phase 5.

### Step 2: the state record and `review check`

* `#[derive(Serialize, Deserialize)] pub struct State` with:
  * `base`, `since`, `head` (full SHAs), `branch` (the name, or `detached`), `started` (Unix seconds, for M3's wall time).  `check` does not compare these, but `head`.
  * `status`: `git status --porcelain=v1 -z`, split.
  * `stash`: `git stash list --format=%H`.
  * `branches`, `tags`: `git for-each-ref --format=%(refname) %(objectname) refs/heads` and `refs/tags`.
  * `remotes`: `git remote -v`.
  * `worktrees`: `git worktree list --porcelain`, one record for each worktree, without those whose path is under `<toplevel>/target/review/`.  Compare canonical paths, since macOS's temp folder is a symlink.
  * `hashes`: a map from a label to `git hash-object --no-filters <file>`, or `absent`.  The labels: `config` (`<git-common-dir>/config`), `config.worktree` (`git rev-parse --git-path config.worktree`), `hooks/<name>` for each file in the hooks folder (`git rev-parse --git-path hooks`, which follows `core.hooksPath`), `.claude/settings.local.json`, and `~/.claude/settings.json` (from the home folder argument).  A relative path from `git rev-parse` is resolved against the repo folder.
* `pub fn changes(old: &State, new: &State) -> Vec<String>` is pure.  It names each change: the field, and each line or key that appears, goes or differs.
* `review check <round>`: reads the record, takes the state now, prints each change and exits 3, or prints "no change" and exits 0.  A missing record exits 3.
* Tests, written first:
  * For each compared field, two records that differ only in it give one change that names it.  One test per field, so each fails with its field removed.
  * A hooks file that appears, and one that goes, each give a change.  So does a hashed file that goes from `absent` to present.
  * In a scratch repo: a worktree added under `target/review/` makes no change.  One added elsewhere does.
  * In a linked worktree of a scratch repo: `config` resolves to the common folder, and `hooks/` to the common hooks folder, not the worktree's folder.  `config.worktree` resolves to the worktree's own folder.
  * `check` on an unchanged scratch repo exits 0; after `git branch x`, it exits 3 and names `branches`.
* Tech debt, in the same commit: a new entry.  `review check` sees only what the record holds.  A change to `~/.claude/settings.json` during a round, by a person or by Claude Code, fails it.  Fix: the person checks the change and starts the round again.

### Step 3: `review start`

* `review start <base> <round>`, in this order:
  1. `<round>` is a whole number of 1 or more, else exit 3 with the usage.
  2. `git status --porcelain` is not empty: exit 3, "commit first".
  3. Merge base: `git merge-base <base> HEAD`.  An unknown `<base>` exits 3.
  4. The run folder: `target/review/run/<branch>/`, with `/` in the branch as `-`; `detached-<git rev-parse --short HEAD>` with no branch.
  5. Round 1: if `<run folder>/log.md` exists, exit 3, "the run folder holds a log: copy it out, or remove it".  M3 writes this log.
  6. `<since>`: the merge base in round 1.  In round n, the `head` of `<run folder>/<n-1>/state.json`; a missing record exits 3 and names it.
  7. Write to the round folder: `branch.diff` (merge base to `HEAD`), `since.diff` (`<since>` to `HEAD`), `route.txt` (both routes, in full), and `state.json`.  Overwrite old files.
  8. Print, in this order: `base: <sha> (merge base of <base> and HEAD)`; `since: <sha>`; `run folder: <absolute path>`; `target folder: <toplevel>/target/review/target`; then `branch <base>..<head>:` and `since <since>..<head>:`, each with one line for each reviewer, `  <name>: <reasons>`.  Print at most 5 reasons for each reviewer, then `(+n more)`; `route.txt` has all.  An empty range prints `  no changes`.  Then each warning.
* Warnings, which do not change the exit code:
  * The binary is older than `<toplevel>/xtask/src/review.rs`: "warning: target/debug/xtask is older than xtask/src/review.rs; run cargo build -p xtask".  No `review.rs` at the toplevel gives no warning.
  * `xtask/src/review.rs` exists at both the merge base and `HEAD`, and its blobs differ: "warning: this branch changes xtask/src/review.rs, so the route comes from the branch".  A `review.rs` missing on either side gives no warning.
  * `pub fn warnings(exe_time, source_time: Option<SystemTime>, base_blob: Option<&str>, head_blob: Option<&str>) -> Vec<String>` is pure.
* Docs, in the same commit as the arm: the guide's Tools table gets a `review` row (`review start`, `review check`, `review mutants`; needs: cargo-mutants for `mutants`), and a changelog line for phase 3 (M1).  `docs-check` fails without the row.
* Tests, written first:
  * `<base>` resolves to the merge base: in a scratch repo, `main` moves after the branch starts, and the branch diff holds none of `main`'s new change.
  * `<since>` is the merge base in round 1, and round 1's `head` in round 2, after a new commit.
  * A missing round 1 record makes round 2 exit 3.
  * Round 1 exits 3 when `log.md` exists, and exits 0 when the round folder exists without it.
  * A dirty tree (an untracked file) exits 3 with "commit first".
  * A detached `HEAD` names the folder `detached-<short sha>`; a branch `a/b` names it `a-b`.
  * `warnings`: an older binary warns; a newer one does not; a missing source does not.  Differing blobs warn; equal blobs, and a blob missing on either side, do not.
  * A scratch repo with no `xtask/src/review.rs` exits 0 with no warning.
  * Round 0 and a word exit 3.

### Step 4: `review mutants` and `.cargo/mutants.toml`

* `review mutants <round> [--config <file>]`:
  1. Read the round's `state.json`; a missing record exits 3.
  2. Remove `<round folder>/mutants.out` and `<round folder>/mutants.out.old`.
  3. Write `<round folder>/mutants.diff`: `git diff --no-renames <since> <head> -- '*.rs'` with the fixed options above.  `--no-renames` shows a renamed file in full, so cargo-mutants sees plain hunks.
  4. Check `cargo mutants --version`.  A failure gives `none`, "cargo-mutants is not installed: cargo install --locked cargo-mutants@27.1.0", and skips the run.
  5. Run, at the toplevel, `cargo mutants --in-diff <mutants.diff> --output <round folder> --copy-vcs=true --gitignore=true --shuffle -j2 --timeout 300 --config <file>`, with `CARGO_TARGET_DIR` removed from the environment.  `<file>` defaults to `<toplevel>/.cargo/mutants.toml`; a relative `--config` is resolved against the current folder.
  6. The limit: 60 minutes, or `STEMYLA_MUTANTS_MINUTES` (a decimal number of minutes, for tests).  Poll `try_wait` each second.  At the limit, run `kill -TERM <pid>`, wait for the child, and mark the run as stopped by its own limit.  `Child::kill` sends SIGKILL, which leaves children and copies behind.
  7. Read `<round folder>/mutants.out/outcomes.json` (`caught`, `missed`, `timeout`, `unviable`) and the length of `mutants.json`, if they exist.
  8. Print one line and write it to `<round folder>/mutants.txt`.  Exit 0, whatever the class.
* The line: `mutants: <class>: tested <t> (caught <c>, missed <m>, unviable <u>), timeouts <o>, not tested <n>`, then `; <reason>` for `none` and for a stop.  Tested is caught + missed + unviable.  Not tested is the length of `mutants.json` minus tested and timeouts, or 0 without it.
* `pub fn classify(end: RunEnd, counts: Option<Counts>) -> Line` is pure.  `RunEnd` holds the exit code (or none, for a signal), whether the run stopped at its own limit, and whether cargo-mutants was missing:
  * Own stop, tested 1 or more: `partial`, "stopped at the limit".
  * Own stop, tested 0: `none`, "stopped before a mutant finished".
  * Exit 0 or 2: `full`.  Exit 0 with no `mutants.out`: `none`, "no mutants in the diff".
  * Exit 3: `partial`, "timeouts have no result".  A run with both missed and timed-out mutants exits 3.
  * Exit 1, 4, 5, 6 or 70: `none`, with the meaning of the code from the parent plan.  Missing cargo-mutants, a signal, or another code: `none`, with the reason.
* `fn run_with_limit(command: Command, limit: Duration) -> RunEnd` holds the poll and the stop, so a test can pass its own command.
* `.cargo/mutants.toml`:
  * `all_features = true`, so code behind `engine`'s `cpal` and `synth`'s `files` is tested.
  * `exclude_globs`: `app/src/picker.rs`, `app/src/separator.rs`, `app/src/store.rs`, `worker/src/web.rs`.
  * `exclude_re`: the `wasm32`-only functions in shared files (`#[cfg(target_arch = "wasm32")]` items in `app/src/lib.rs`, `app/src/reset.rs`, `engine/src/live.rs` and others that `cargo mutants --list -p app -p stem-worker -p engine` shows).  A name that also has a native variant, such as `base_latency` in `engine/src/live.rs`, stays in.  A comment says why.
  * Check: `cargo mutants --list -p app -p stem-worker -p engine` lists no mutant in the four files, and none in a function that is only `wasm32`.  Progress records the counts before and after.
* Tests, written first:
  * `classify`: one case for each exit code above (0, 1, 2, 3, 4, 5, 6, 70, another code, a signal, missing), and own stop with 0 and with some tested.  Each fails with its class removed.
  * A round folder with an old `mutants.out` and `mutants.out.old`: the clear step removes both, and exit 0 then gives `none`, "no mutants in the diff".
  * `run_with_limit` with `sh -c 'trap "exit 7" TERM; sleep 30 & wait'` and a limit of 2 s: it ends in under 10 s, as stopped by its own limit, with exit 7.  Exit 7 shows SIGTERM, not SIGKILL.
  * The line's counts: tested is caught + missed + unviable, timeouts are apart, and not tested comes from `mutants.json`.
  * A missing record exits 3.
* Tech debt, in the same commit: a new entry for cargo-mutants' gaps.  A `wasm32`-only block in a shared function, or a new `wasm32`-only function, shows as a missed mutant until `exclude_re` names it.  A diff of tests only, WGSL or JavaScript gives no mutants.  A stopped run is partial evidence.  Fix: the validator decides each missed mutant (M3); phase 6 reads the log.

### Step 5: the agent-file test and the docs

* `xtask/tests/rules.rs`, `every_reviewer_has_an_agent_file`, with `#[ignore = "M2 adds the agent files"]`:
  * Each name in `review::REVIEWERS` has `.claude/agents/<name>.md`.
  * Each `.claude/agents/*reviewer.md` has a row.
  * Each include glob in `REVIEWERS` matches a tracked file (`git ls-files`, as the other rules tests read the index), but those in `MAY_MATCH_NOTHING`.  `ROUTE_ALL` and the exclude globs are not checked.
  * Each problem names the glob or the file, and the fix.
  * Run it with `--ignored`: the glob part passes, and the agent part fails with six missing files.  Progress records both.  Plant a glob that matches nothing; it fails.
* `README.md`, "Setting up a machine": `cargo install --locked cargo-mutants@27.1.0`, for `xtask review mutants`.
* `docs/harness.md`: a row for `xtask review`, kind "sensor, computational", phase 3.  The failure: one general reviewer read every diff, a reviewer could change the repo unseen, and reviewers planted mutants by hand.
* No doc names `docs/reviews/` before M5.  A doc names a file of the run folder by its full `target/review/...` path.
* This step may go in the commits of steps 1 to 4.

### Step 6: acceptance runs

The binary is `target/debug/xtask` of the main checkout, built from M1's last commit.  Each run records its command, exit code and the lines that matter in Progress.

1. The worktree check:
   * `git worktree add target/review/m1-ms11 ai-agent-harness-phase-2-ms11`.
   * In it: `<main checkout>/target/debug/xtask review start 7cd8518 1`.  Pass: exit 0, and the branch route names exactly `reviewer`, `memory-reviewer`, `supply-chain-reviewer` and `docs-reviewer`.
   * `touch` an untracked file, and run it again.  Pass: exit 3, "commit first".  Remove the file.
   * `git worktree remove target/review/m1-ms11`.
2. The mutants checks, in a worktree `target/review/m1-mutants` on a scratch branch `m1-scratch-mutants` from M1's last commit.  Commits there are signed, with neutral messages:
   * Full: change one line inside one small `dsp` function, such as a comment line, and commit.  Run `review start <M1 commit> 1`, then `review mutants 1`.  Pass: the line says `full`.
   * Caught behind `cpal`: change one line inside a tested function of `engine/src/live.rs`, such as `nth_start` or `is_fatal`, and commit.  Run `review start <M1 commit> 2`, then `review mutants 2`.  Pass: `mutants.out/caught.txt` names a mutant of that function.
   * Partial: change a line in several functions of `engine/src/`, enough for more than 10 mutants, and commit.  Run `review start <M1 commit> 3`, then `STEMYLA_MUTANTS_MINUTES=<m> review mutants 3`.  Pick `<m>` above the cold baseline build (about 140 s) and below the whole run, from the first two runs' times.  Pass: the line says `partial` with tested above 0.  Then `ps -axo pid,ppid,command | grep -E 'cargo|mutants'` shows no process of the run.
   * Remove the worktree.  Keep the branch until the end of the run; the orchestrator deletes it.
3. The route measurement, a fact and not a pass or fail:
   * A detached worktree `target/review/m1-measure`.  For each first-parent commit of `main` from `a9ace3a` (#13) to `fa41c9f` (#33): `git checkout --detach <sha>`, then `review start <sha>^ 1`.  Each run gets its own run folder, `detached-<short sha>`.
   * Progress records, for each PR, its number and the reviewers, and the mean number of reviewers for each PR.
   * Remove the worktree.

### Step 7: three review rounds (team leader)

* Each round starts new reviewer subagents, with the manual recipe of `docs/working-with-agents.md` at `fa41c9f` ("Review a change").  The range is M1's commits: `422ebe3..HEAD`.  The criteria are the parent plan's M1 section.
* Each reviewer gets the range, the criteria, this plan's path, and the read-only rules: no edits, no commits, no branch changes; try changes only in a worktree under `target/review/`.
* The team leader checks each finding before a fix, and sends each real one to the implementer, with a test that fails first.
* A Progress line for each round, committed before the next round starts.

## Validation and acceptance

* `cargo test -p xtask` passes, with the `review.rs` unit tests.  `cargo test -p xtask --test rules -- --ignored every_reviewer_has_an_agent_file` fails only on the missing agent files.
* Each unit test failed before its code, and fails with its row, sign, field or class removed.  Progress names the plants that were run.
* The worktree check and the three mutants checks of step 6 pass.
* The route measurement of PRs #13 to #33 is in Progress.
* fmt, every clippy command of `CLAUDE.md`, the workspace tests, `cargo test -p roformer-gpu --lib` and `docs-check` pass on the last commit.

## Idempotence and recovery

* `review start` and `review mutants` write only to the run folder; `review check` writes nothing.  Running `start` again overwrites its round's files.
* To remove all runs: `rm -rf target/review`, then `git worktree prune`.
* Scratch worktrees are under `target/review/m1-*`; remove each with `git worktree remove --force <path>`.  The scratch branch `m1-scratch-mutants` stays until the orchestrator deletes it.
* To resume: read Progress, then `git log --oneline 422ebe3..HEAD`.
* To undo M1: revert its commits.  Keep the IDs of tech-debt entries that M1 adds: move each under `## Closed` as withdrawn.

## Progress

* [x] 2026-10-23: milestone plan written by the team leader.

## Surprises and discoveries

None yet.

## Decision log

* 2026-10-23, team leader: every failure of `xtask review` exits 3, its own errors included, so a skill stops on each.  The parent plan names exit 3 for a refusal only.
* 2026-10-23, team leader: the unit tests live in `xtask/src/review.rs`, so a change to them routes every reviewer, as a change to the code does.
* 2026-10-23, team leader: the state record also holds `started`, so M3 can count wall minutes from `review start`.  `check` does not compare it.
* 2026-10-23, team leader: `review start` prints at most 5 reasons for each reviewer, and writes all of them to `route.txt`, so a long diff gives a short prompt.

## Outcomes and retrospective

To be written when M1 completes.
