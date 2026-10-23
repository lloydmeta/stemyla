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
* `xtask/src/rules.rs` and `xtask/tests/rules.rs` hold the structural tests.  `xtask/tests/publish.rs` makes scratch repos in `std::env::temp_dir()` with the process ID in the name.  `xtask` does not depend on `tempfile`, and M1 adds no crate (DEPS-1).
* `.cargo/` does not exist.  `.gitignore` ignores `target/` and `/.claude/worktrees/`.
* The repo sets `extensions.worktreeconfig = true`.  `git rev-parse --git-common-dir` gives `.git` in the main checkout.  `core.hooksPath` is not set.  Git is 2.50.1 (Apple Git-155).
* cargo-mutants 27.1.0 is installed (`cargo mutants --version`).
* The maintainer's global git config signs every commit.  One test helper runs every `git` call in a scratch repo with `-c commit.gpgsign=false -c user.name=lloyd -c user.email=lloyd@beachape.example`.  So `git stash`, which also commits, has an identity in CI too.  A commit also passes `--no-verify`.
* Each scratch repo of a test commits a `.gitignore` with `target/`, as the real repo has.  Else the run folder shows in `git status`, and the next `start` refuses the dirty tree.  The maintainer's `core.excludesfile` ignores `target/` and `*.log`, so a missing `.gitignore` passes on the Mac and fails in CI.  A test's untracked file has a plain name, such as `x.txt`.
* No unit test needs cargo-mutants, since CI does not install it.  The tests of `review mutants` call `classify` and the clear step with fake values.
* `engine/src/live.rs` is behind the `cpal` feature (`engine/src/lib.rs`), and has tests in `mod tests`.  `is_fatal` has three mutants, and its tests check both results.  `nth_start` has one mutant, `Default::default()` of `cpal::StreamInstant`, which has no `Default`, so the mutant is unviable.
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
* Scratch worktrees go under `target/review/m1-<name>/` in the main checkout.  `target/` is ignored, and the state record leaves out worktrees under `target/review/`.  Scratch branches are named `m1-scratch-<name>`.  Progress lists each one.

### The code layout

* `xtask/src/review.rs`, with `pub mod review;` in `xtask/src/lib.rs`.  Its unit tests are in `#[cfg(test)] mod tests` in the same file, so a change to them routes every reviewer too.
* `pub fn main(args: &[String]) -> i32` dispatches `start`, `check` and `mutants`, and returns the exit code.  `xtask/src/main.rs` gets the arm `"review" => std::process::exit(review::main(&args[1..])),` and `review` in its usage line.
* Exit codes: 0 for success; 3 for each refusal, each usage error and each error of its own, such as a failed `git`.  `main` prints the lines to stdout for exit 0, and to stderr for exit 3.  So no failure of `xtask review` exits 1, which a skill could read as a search with no match.
* Pure functions take text and values, and return values.  Thin wrappers run `git` and `cargo` with `std::process::Command`.
* The seam for tests: `pub fn run(args: &[String], dir: &Path, home: &Path, exe_time: SystemTime) -> (i32, Vec<String>)`.  It returns the exit code and the lines to print.  `main` passes the current folder, `$HOME` and the binary's time, and prints the lines.  Tests call `run` with a scratch repo, and never change the current folder, since tests run in parallel threads.
* More seams, `pub` from the step that writes them: `diff_route` (step 1); `take_state(toplevel, home) -> Result<State>` and `hashed_paths(toplevel, home) -> Result<Vec<(String, PathBuf)>>` (step 2).  Step 2's tests write a record with `take_state`, since `start` comes in step 3.
* Until a step uses them, `run`'s unused parameters are named `_home` and `_exe_time`.  `Reviewer`'s fields are `pub`, for `xtask/tests/rules.rs`.
* `run` finds the toplevel with `git rev-parse --show-toplevel` in `dir`.  A relative path argument, such as `--config`, resolves against `dir`.  Each later `git` call runs in the toplevel, so a relative path that `git` gives is relative to the toplevel.  A function that needs the home folder or the binary's time takes it as an argument, so tests pass their own.
* Every `git diff` call passes `--no-ext-diff --no-color --no-textconv --src-prefix=a/ --dst-prefix=b/`, and `git -c core.quotePath=false`.  So a person's `diff.noprefix`, `diff.mnemonicPrefix`, `diff.external` or `core.quotePath` does not change the output.

### Step 1: the route

* Write `pub fn main` and `pub fn run` first, with the dispatch and the usage.  Each step adds its subcommand to `run`.  A function that no subcommand calls yet is `pub`, so clippy finds no dead code at each commit.  The `xtask/src/main.rs` arm waits for step 3, with the guide's row.
* `pub struct Reviewer { name, include: &[&str], exclude: &[&str] }` and `pub const REVIEWERS: &[Reviewer]`, the six rows of the parent plan's table, in its order.  `docs-reviewer` excludes `docs/plans/**` and `docs/reviews/**`.
* `pub const ROUTE_ALL: &[&str]`: `xtask/src/review.rs`, `.claude/agents/**`, `.claude/skills/**`, `.claude/hooks/**`.  A path that matches one routes all six.
* `pub const MAY_MATCH_NOTHING: &[(&str, &str)]`: `**/build.rs` and `.mcp.json`, each with a reason ("no crate has a build script yet"; "the repo has no MCP config yet").
* `pub const SIGNS: &[(&str, &[&str])]`: the content signs of the parent plan, by reviewer.  Each sign keeps the TD or rule ID of the parent plan in a comment.
* Globs use `docs::glob_matches`.
* Input: `git diff -z --raw -M <range>` and `git diff -U0 -M <range>`.
  * Parse `--raw -z` into entries of old mode, new mode, status and one or two paths.  A rename (`R<score>`) and a copy (`C<score>`) have two paths.  Each path routes.
  * A file with mode `120000` on either side routes `supply-chain-reviewer`.
  * Parse the patch for added lines by file.  A file header runs from `diff --git` to the first `@@`.  Take the path from `+++ b/<path>`; `+++ /dev/null` is a deletion.  Git adds a tab after a path with a space, and C-quotes the whole `b/<path>` when the path has `"`, `\` or a control character.  Strip the tab, unquote the whole text, then strip `b/`.  After an `@@` line, a line that starts with `+` is an added line, until the next `diff --git`.  So an added line `++x`, shown as `+++x`, is not a header.
* Signs: a sign counts on an added line of a `.rs` file with no path segment `tests`.  The file must not match that reviewer's include globs.  A sign matches as a whole token:
  * If the sign starts with a letter, a digit or `_`, the character before it must not be one of those.
  * If the sign ends with one, the character after it must not be one.
  * So `into_owned` is not `to_owned`, and `Vec::from_iter` is not `Vec::from`.
* Output: `Route`, a map from reviewer name to its reasons, in table order.  A reason is given once for each path, and once for each sign and file; repeats of a sign in one file give one reason.  When a path matches two globs of one row, the first glob in row order is the reason.  A `ROUTE_ALL` path gives each reviewer only `<path> changes the review harness`; `reviewer` also keeps `every diff`.  A `ROUTE_ALL` file adds no sign reason; a test adds `to_vec` to `xtask/src/review.rs` and expects none.  A reason is one of: `every diff`; `<path> (<glob>)`; `<path> changes the review harness`; `<path> is a symlink`; `` `<sign>` in <path> ``.
* An empty diff gives an empty route.  A non-empty diff always routes `reviewer`, with the reason `every diff`.
* `pub fn route(raw: &str, patch: &str) -> Route` is pure.  `pub fn diff_route(toplevel: &Path, from: &str, to: &str) -> Result<Route>` runs the two diffs.
* Tests, written first.  Each fails with its row, glob, sign or rule removed.  So write each expected case by hand, as a path, a reviewer and a reason.  Do not make the cases from `REVIEWERS`, `ROUTE_ALL` or `SIGNS`.
  * For each include glob of each row, a path that matches it routes that reviewer with that glob as the reason.
  * An excluded path (`docs/plans/active/x.md`, `docs/reviews/x.md`) does not route `docs-reviewer`.
  * A path in no row (`dsp/src/lib.rs`) routes `reviewer` only.
  * Each path of `ROUTE_ALL` routes all six.  One test per path.
  * Each sign, on an added line of `xtask/src/x.rs`, routes its reviewer.  One case per sign.
  * A sign inside a longer name routes nothing: `into_owned`, `my_to_vec`, `unsafely`, `my_include!(x)`, `Vec::from_iter`, `copy_to_slice`, `xwgpu::`.  Each case holds its sign as text, so it fails with the whole-token rule removed.
  * A sign adds no reason: in that reviewer's paths (`to_vec` in `engine/src/x.rs`); on a removed line; `wgpu::` in `engine/tests/x.rs`, and `to_vec` in `xtask/tests/x.rs`; `to_vec` in a `.md` file.
  * A rename from `docs/x.md` to `engine/src/x.rs` routes `docs-reviewer` and `memory-reviewer`.  The test makes it in a scratch repo with real `git`, to test the parse of real `-M` output.
  * A sign in a file whose name has a space, made with real `git`, routes its reviewer.
  * Mode `120000`: a symlink added in a scratch repo routes `supply-chain-reviewer`.
  * The patch parse: an added line `++x` in the hunk is an added line, not a header.
  * An empty diff gives an empty route.
* Tech debt, in the same commit: a new entry for the route's gaps.  The content signs are a short denylist, not a full list.  For example, `create_buffer` does not match `create_buffer_init`, which `roformer-gpu/src/lib.rs` calls.  The route comes from the branch's own `xtask`, so a branch can change it; `review start` warns (step 3), and phase 5 must route with the base branch's `xtask`.  Fix: review, then phase 5.

### Step 2: the state record and `review check`

* `take_state` leaves `base`, `since` and `started` empty; `start` fills them.
* `#[derive(Serialize, Deserialize)] pub struct State` with:
  * `base`, `since`, `head` (full SHAs), `branch` (the name, or `detached`), `started` (Unix seconds, for M3's wall time).  `check` does not compare these, but `head`.
  * `status`: `git status --porcelain=v1 -z --untracked-files=normal`, split.  The dirty check of `start` passes `--untracked-files=normal` too, so a person's `status.showUntrackedFiles=no` hides nothing.
  * `stash`: `git stash list --format=%H`.
  * `branches`, `tags`: `git for-each-ref --format=%(refname) %(objectname) refs/heads` and `refs/tags`.
  * `other_refs` (added by review round 1): every other ref but `refs/stash`, with the same format, such as `refs/remotes/`, notes and replacements.  So a push or a fetch is a change.
  * `remotes`: `git remote -v`.
  * `worktrees`: `git worktree list --porcelain`, one record for each worktree.  Leave out each worktree under `target/review/` of the toplevel, or of the main worktree (the first record).  Compare canonical paths, since macOS's temp folder is a symlink.
  * `hashes`: a map from a label to `git hash-object --no-filters <file>`, or `absent`.  The labels:
    * `config`: `<git-common-dir>/config`.
    * `config.worktree`: `git rev-parse --git-path config.worktree`.
    * `hooks/<name>`, for each file in the hooks folder: `git rev-parse --git-path hooks`, which follows `core.hooksPath`.
    * `.claude/settings.local.json`, at the toplevel.
    * `~/.claude/settings.json`, from the home folder argument.
  * Resolve a relative path from `git rev-parse` against the toplevel, where `git` ran.
* `pub fn changes(old: &State, new: &State) -> Vec<String>` is pure.  It names each change: the field, and each line or key that appears, goes or differs.
* `review check <round>`: reads the record, takes the state now, prints each change and exits 3, or prints "no change" and exits 0.  A missing record exits 3, with "no record for this branch and round; HEAD may have moved, see git reflog".
* Step 2 also writes `fn run_folder(toplevel) -> PathBuf` (the branch name, or `detached-<short sha>`), since `check` needs it.  Step 3 tests it.
* Tests, written first:
  * For each compared field, two records that differ only in it give one change that names it.  One test per field, so each fails with its field removed.
  * The capture of each field, in a scratch repo.  Each test takes a record with `take_state`, makes one real change, and expects `check` to exit 3 and name that field:
    * `head`: `git commit --allow-empty`.
    * `status`: an untracked `x.txt`.
    * `stash`: an edit, then `git stash`.
    * `tags`: `git tag t`.
    * `remotes`: `git remote add o /x`.
    * `config`: `git config x.y z`.
    * `hooks/<name>`: a new file in the hooks folder.
    * `.claude/settings.local.json`: a new file at the toplevel.
    * `~/.claude/settings.json`: a write to `<scratch home>/.claude/settings.json`.  The test passes its own home folder, never the real one.
  * A hooks file that appears, and one that goes, each give a change.  So does a hashed file that goes from `absent` to present.
  * In a scratch repo: a worktree added with `git worktree add --detach` under `target/review/` makes no change.  One added elsewhere does.  (Without `--detach`, git makes a branch, and `branches` changes.)
  * In a linked worktree of a scratch repo: `config` resolves to the common folder, and `hooks/` to the common hooks folder, not the worktree's folder.  `config.worktree` resolves to the worktree's own folder.
  * `check` on an unchanged scratch repo exits 0; after `git branch x`, it exits 3 and names `branches`.
* A tech-debt entry names the settings files as prose, not in a code span, since `docs-check` checks a span that looks like a path.
* Tech debt, in the same commit: a new entry.  M3 adds to this entry, and does not add a second one.  `review check` sees only what the record holds.  A change to `~/.claude/settings.json` during a round, by a person or by Claude Code, fails it.  Fix: the person checks the change and starts the round again.  Also: with a detached `HEAD`, the run folder name changes after a fix commit, so round 2 finds no record.

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
  9. The cap of 5 applies to path reasons only.  Every sign, symlink and review-harness reason prints, since reviewers may not read the run folder.
* Warnings, which do not change the exit code:
  * The binary is older than `<toplevel>/xtask/src/review.rs`: "warning: target/debug/xtask is older than xtask/src/review.rs; run cargo build -p xtask".  No `review.rs` at the toplevel gives no warning.
  * `xtask/src/review.rs` exists at both the merge base and `HEAD`, and its blobs differ.  The warning: "warning: this branch changes xtask/src/review.rs, so the route comes from the branch".  A `review.rs` missing on either side gives no warning.
  * `pub fn warnings(exe_time, source_time: Option<SystemTime>, base_blob: Option<&str>, head_blob: Option<&str>) -> Vec<String>` is pure.
* Docs, in the same commit as the arm:
  * The guide's Tools table gets a `review` row: `review start`, `review check`, `review mutants`.  It needs cargo-mutants for `mutants`.
  * The guide gets a changelog line for phase 3 (M1).
  * `docs-check` fails without the row.
* Tests, written first:
  * `<base>` resolves to the merge base.  In a scratch repo, `main` moves after the branch starts.  The branch diff holds none of `main`'s new change.
  * `<since>` is the merge base in round 1, and round 1's `head` in round 2, after a new commit.
  * A missing round 1 record makes round 2 exit 3.
  * Round 1 exits 3 when `log.md` exists, and exits 0 when the round folder exists without it.
  * A dirty tree (an untracked file) exits 3 with "commit first".
  * A detached `HEAD` names the folder `detached-<short sha>`; a branch `a/b` names it `a-b`.
  * `warnings`: an older binary warns; a newer one does not; a missing source does not.  Differing blobs warn; equal blobs, and a blob missing on either side, do not.
  * A scratch repo with no `xtask/src/review.rs` exits 0 with no warning.
  * The output, through `run`: six paths under `engine/src/` and `to_vec` in `xtask/src/x.rs` give `memory-reviewer` 5 path reasons, `(+1 more)` and the sign reason.  `route.txt` holds all 7 of its reasons.  Round 2 with no new commit prints `  no changes` for the `since` range.
  * Round 0 and a word exit 3.

### Step 4: `review mutants` and `.cargo/mutants.toml`

* `review mutants <round> [--config <file>]`:
  1. Read the round's `state.json`; a missing record exits 3.
  2. Remove `<round folder>/mutants.out` and `<round folder>/mutants.out.old`.
  3. Write `<round folder>/mutants.diff`: `git diff --no-renames <since> <head> -- *.rs`, with the fixed options above.  `*.rs` is one argument to `Command`, with no quotes.  `--no-renames` shows a renamed file in full, so cargo-mutants sees plain hunks.
  4. Check `cargo mutants --version`.  A failure gives `none`, "cargo-mutants is not installed: cargo install --locked cargo-mutants@27.1.0", and skips the run.
  5. Run, at the toplevel, `cargo mutants --in-diff <mutants.diff> --output <round folder> --copy-vcs=true --gitignore=true --shuffle -j2 --timeout 300 --config <file>`, with `CARGO_TARGET_DIR` removed from the environment.  Its stdout and stderr go to `<round folder>/mutants.log`, so the command prints only its own line.  `<file>` defaults to `<toplevel>/.cargo/mutants.toml`; a relative `--config` is resolved against the current folder.
  6. The limit: 60 minutes, or `STEMYLA_MUTANTS_MINUTES` (a decimal number of minutes, for tests).  Poll `try_wait` each second.  At the limit, run `kill -TERM <pid>`, and mark the run as stopped by its own limit.  If the child has not ended 60 s later, send `kill -KILL <pid>`, and say so in the line.  `Child::kill` sends SIGKILL, which leaves children and copies behind.
  7. Read `<round folder>/mutants.out/outcomes.json` (`caught`, `missed`, `timeout`, `unviable`) and the length of `mutants.json`, if they exist.
  8. Print one line and write it to `<round folder>/mutants.txt`.  Exit 0, whatever the class.
* The line: `mutants: <class>: tested <t> (caught <c>, missed <m>, unviable <u>), timeouts <o>, not tested <n>`, then `; <reason>` for `none` and for a stop.  Tested is caught + missed + unviable.  Not tested is the length of `mutants.json` minus tested and timeouts, or 0 without it.
* `pub fn classify(end: RunEnd, counts: Option<Counts>) -> Line` is pure.  `RunEnd` holds the exit code (or none, for a signal), whether the run stopped at its own limit, and whether cargo-mutants was missing:
  * Own stop, tested 1 or more: `partial`, "stopped at the limit".
  * Own stop, tested 0: `none`, "stopped before a mutant finished".
  * Exit 0 or 2: `full`.  Exit 0 with no `mutants.out`: `none`, "no mutants in the diff".
  * Exit 3: `partial`, "timeouts have no result".  A run with both missed and timed-out mutants exits 3.
  * Exit 1, 4, 5, 6 or 70: `none`, with the meaning of the code.  1: a usage error, or a stop by a signal that was not its own.  4: the baseline failed.  5: the diff does not fit the tree.  6: the diff cannot be read.  70: an internal error of cargo-mutants.  Missing cargo-mutants, a signal, or another code: `none`, with the reason.
* `fn run_with_limit(command: Command, limit: Duration) -> RunEnd` holds the poll and the stop, so a test can pass its own command.
* `.cargo/mutants.toml`:
  * `all_features = true`, so code behind `engine`'s `cpal` and `synth`'s `files` is tested.
  * `exclude_globs`: `app/src/picker.rs`, `app/src/separator.rs`, `app/src/store.rs`, `worker/src/web.rs`.
  * `exclude_re`: the `wasm32`-only functions in shared files (`#[cfg(target_arch = "wasm32")]` items in `app/src/lib.rs`, `app/src/reset.rs`, `engine/src/live.rs` and others that `cargo mutants --list -p app -p stem-worker -p engine` shows).  A name that also has a native variant, such as `base_latency` in `engine/src/live.rs`, stays in.  A comment says why.
  * Check: `cargo mutants --list -p app -p stem-worker -p engine` lists no mutant in the four files, and none in a function that is only `wasm32`.  Progress records the counts before and after.
* Tests, written first:
  * `classify`: one case for each exit code above: 0, 1, 2, 3, 4, 5, 6, 70, another code, a signal, and missing.  Also own stop, with 0 tested and with some tested.  Each fails with its class removed.
  * A round folder holds an old `mutants.out` and `mutants.out.old`.  The clear step removes both.  Then `classify` with a fake exit 0 and no `mutants.out` gives `none`, "no mutants in the diff".
  * `run_with_limit` with `sh -c 'trap "exit 7" TERM; sleep 30 & wait'` and a limit of 2 s: it ends in under 10 s, as stopped by its own limit, with exit 7.  Exit 7 shows SIGTERM, not SIGKILL.
  * The line's counts: tested is caught + missed + unviable, timeouts are apart, and not tested comes from `mutants.json`.
  * A missing record exits 3.
  * `mutants.diff`, in a scratch repo.  A commit changes `a.rs` and `b.md`, and renames `c.rs` to `d.rs` with a change.  The diff holds `a.rs`, and `c.rs` and `d.rs` as a deletion and an addition, and not `b.md`.  Write the diff with a `pub fn` that a test calls without cargo-mutants.
  * The `--config` path: none gives `<toplevel>/.cargo/mutants.toml`; a relative one resolves against `dir`; an absolute one stays.
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
* The docs may go in the commits of steps 1 to 4.  The test goes in after step 4, since `.cargo/**` matches nothing before `.cargo/mutants.toml` exists.

### Step 6: acceptance runs

The binary is `target/debug/xtask` of the main checkout, built from M1's last commit.  Each run records its command, exit code and the lines that matter in Progress.

1. The worktree check:
   * `git worktree add target/review/m1-ms11 ai-agent-harness-phase-2-ms11`.
   * In it: `<main checkout>/target/debug/xtask review start 7cd8518 1`.  Pass: exit 0, and the branch route names exactly `reviewer`, `memory-reviewer`, `supply-chain-reviewer` and `docs-reviewer`.
   * `touch` an untracked file, and run it again.  Pass: exit 3, "commit first".  Remove the file.
   * `git worktree remove target/review/m1-ms11`.
2. The mutants checks, in a worktree `target/review/m1-mutants` on a scratch branch `m1-scratch-mutants` from M1's last commit.  Commits there are signed, with neutral messages.  Each `review start` there warns that the binary is older than `review.rs`; that is expected.
   * Full: change a code line inside one small `dsp` function, and commit.  A comment before the first statement gives no mutant.  Run `review start <M1 commit> 1`, then `review mutants 1`.  Pass: the line says `full`.
   * Caught behind `cpal`: change one line inside `is_fatal` in `engine/src/live.rs`, and commit.  Do not use `nth_start`: its one mutant is unviable.  Run `review start <M1 commit> 2`, then `review mutants 2`.  Pass: `mutants.out/caught.txt` names a mutant of that function.
   * Partial: change a line in several functions of `engine/src/`, enough for more than 10 mutants, and commit.  Run `review start <M1 commit> 3`, then `STEMYLA_MUTANTS_MINUTES=<m> review mutants 3`.  Pick `<m>` above the cold baseline build (about 140 s) and below the whole run, from the first two runs' times.  Pass: the line says `partial` with tested above 0.  Then `pgrep -fl 'cargo-mutants|m1-mutants'` shows no process of the run.  In the same shell, `ls "$TMPDIR" | grep cargo-mutants-m1-mutants-` shows no copy.  (A copy of an earlier probe, `cargo-mutants-mt-*`, is in the folder and is not this run.)
   * Before each run, check that the diff has mutants.  Write `git diff <since> HEAD -- "*.rs"` to a file in `$TMPDIR`.  Then `cargo mutants --in-diff <that file> --list` must not be empty.
   * Remove the worktree.  Keep the branch until the end of the run; the orchestrator deletes it.
3. The route measurement, a fact and not a pass or fail:
   * A detached worktree `target/review/m1-measure`.  For each first-parent commit of `main` from `a9ace3a` (#13) to `fa41c9f` (#33): `git checkout --detach <sha>`, then `review start <sha>^ 1`.  Each run gets its own run folder, `detached-<short sha>`.
   * Progress records, for each PR, its number and the reviewers, and the mean number of reviewers for each PR.
   * Remove the worktree.

### Step 7: three review rounds (team leader)

* Each round starts new reviewer subagents, with the manual recipe of `docs/working-with-agents.md` at `fa41c9f` ("Review a change").  The range is M1's commits: `422ebe3..HEAD`.  The criteria are the parent plan's M1 section.
* Each reviewer gets the range, the criteria, this plan's path, and the read-only rules.  It makes no edits, no commits and no branch changes.  It tries changes only in a worktree under `target/review/`.
* The team leader checks each finding before a fix, and sends each real one to the implementer, with a test that fails first.
* A fix that changes `xtask/src/review.rs` or `.cargo/mutants.toml` runs again the step 6 checks that it affects.  Progress records the new result.
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

* [x] 2026-10-23: milestone plan written by the team leader (`c2d7d84`).
* [x] 2026-10-23: plan review round 1, by two new reviewers (feasibility, with git and cargo-mutants probes; completeness).  1 significant should-fix, 5 other should-fix, 13 nits; all checked and fixed.  The significant one: `nth_start`'s one mutant is unviable, so step 6 uses `is_fatal`.  Also: scratch repos ignore `target/`; no unit test needs cargo-mutants; cases are written by hand; `git` runs in the toplevel; sign reasons are not capped.
* [x] 2026-10-23: plan review round 2, by one new reviewer.  2 significant should-fix, 2 other should-fix, 6 nits; all checked and fixed.  The significant ones: tests reach a scratch repo through `run(args, dir, home, exe_time)`, not the current folder; the whole-token cases no longer use `include_str!`, which is a sign itself.  Also: `pub` for functions not yet called; one reason per sign and file; cargo-mutants' output goes to `mutants.log`.
* [x] 2026-10-23: plan review round 3, by one new reviewer.  1 significant should-fix, 3 other should-fix, 4 nits; all checked and fixed.  The significant one: each field of the state record now has a test of its capture with real git, not only of the comparison.  Also: seams for `take_state` and `hashed_paths`; tests of `start`'s output and of `mutants.diff`; `create_buffer_init` named as a gap of the denylist.
* [x] 2026-10-23: plan review round 4, by one new reviewer.  3 significant should-fix, 1 other should-fix, 4 nits; all checked and fixed.  The significant ones: the output test uses `memory-reviewer`, which has signs; the full case changes a code line, since a comment before the first statement gives no mutant; the copy check names this run's copies only.  Also: one git helper gives every call in a scratch repo an identity, `git stash` included.
* [x] 2026-10-23: plan review round 5, by one new reviewer.  No blocker and no significant should-fix: the stop rule is met.  1 other should-fix and 5 nits; all fixed.  A review fix to `review.rs` or `.cargo/mutants.toml` runs step 6 again; a `ROUTE_ALL` file adds no sign reason; `git status` shows untracked files whatever the config; a SIGKILL follows 60 s after an ignored SIGTERM.
* [x] Step 1: the route.  2026-10-23, `4dbef84`.  `route`, `diff_route`, the four tables and `run`'s usage, with 23 unit tests.  Tests first: on a stub that gave an empty route, 18 of the first 20 failed by assertion; the empty-route and usage tests passed, as expected.  Plants, each undone, each failed its test: an include glob (`xtask/e2e/**`), an exclude (`docs/reviews/**`), a sign (`model_url`), the whole-token rule, a `ROUTE_ALL` glob, the `tests` rule, the `.rs` rule, the own-path rule, the harness rule for signs, the symlink rule, the second path of a rename, the header check in a hunk, the first-glob order and the dedupe.  The dedupe plant first passed every test, since a sign gives one reason for each file already.  So a test with a path given twice was added, and it fails with the dedupe removed.  It, the first-glob test and the repeated-sign test came after the code, and were checked by plants.  TD-31 added.
* [x] Step 2: the state record and `review check`.  2026-10-23, `969c5d5`.  `State`, `take_state`, `hashed_paths`, `changes`, `run_folder` and `review check`, with 27 new unit tests.  Tests first: on stubs (an empty state, no paths, no changes), each failed by assertion but the worktree under `target/review/`, which expects no change; the linked-worktree test failed on a missing label.  A replaced line gives a "goes" and an "appears" change, so each field test checks that every change names the field.  Plants, each undone, each failed its test: the `head` comparison, the `stash` comparison, a hash comparison, a hash key that appears, and the capture of `status`, `stash`, `tags`, `remotes`, `worktrees`, `config`, the local settings and the home settings.  Also the exclusion of `target/review/` worktrees, the hooks folder as `.git/hooks`, and `config` from `--git-dir`.  TD-32 added.
* [x] Step 3: `review start`.  2026-10-23, `dc9d441`.  `review start`, `warnings`, `route_lines`, the `review` arm in `xtask/src/main.rs`, the guide's row and changelog line, and the warning in TD-31, with 13 new unit tests.  `docs-check` failed with the arm and without the row ("does not mention `review`").  Tests first: on stubs (exit 0 with no lines, no warnings), 11 failed by assertion; the test with no `review.rs` and the round test passed, since `parse_round` came in step 2.  Plants, each undone, each failed its test: `rev-parse` for the merge base, round 1's `since` in round 2, the log check, the dirty check, `/` kept in the folder name, a cap of 6, the cap on sign reasons, the time comparison, the blob comparison, and no `state.json`.  The plant of `since.diff` from the merge base first passed, since the test's two diffs held the same lines.  So round 1 now adds a second file, which round 2's diff must not hold, and the plant fails.
* [x] Step 4: `review mutants` and `.cargo/mutants.toml`.  2026-10-23, `e261830`.  `review mutants`, `classify`, `Line`, `clear_old_output`, `write_mutants_diff`, `config_path`, `run_with_limit` and `read_counts`, with 23 new unit tests; the README row, the guide's row and changelog line, and TD-33.  Tests first: on stubs, 21 failed by assertion; the test of `read_counts` and the missing record passed, since that code was not a stub.  `cargo mutants --list -p app -p stem-worker -p engine`: 1575 mutants before the config, 364 of them in the four `wasm32` files; 1161 after, none in the four files and none in the 12 `wasm32`-only functions (`exclude_re`: 50 mutants).  `base_latency` and both `main` functions stay in.  cargo-mutants' schema (`--emit-schema config`) confirmed the keys, and it refuses an unknown key.  Plants, each undone, each failed its test: the meaning of exit 4, exit 2 as full, exit 3 as partial, the own-stop branch, own stop with 0 tested, the missing branch, timeouts counted as tested, not tested without timeouts, the SIGKILL note, exit 0 with no `mutants.out`, the clear of `mutants.out.old`, `-M` for `--no-renames`, no `*.rs` pathspec, SIGKILL first, no SIGKILL after the grace, and `--config` against the toplevel.  The plant with no SIGKILL first hung the test, so the test now waits 15 s at most, and its shell ends by itself after 30 s.
* [x] Step 5: the agent-file test and the docs.  2026-10-23, `a8566d2`.  `every_reviewer_has_an_agent_file` in `xtask/tests/rules.rs`, with `#[ignore = "M2 adds the agent files"]`, and the `docs/harness.md` row.  The README row and the guide went in with steps 3 and 4.  `cargo test -p xtask --test rules -- --ignored every_reviewer_has_an_agent_file` fails with 6 problems, each a missing agent file; the glob part passes.  Plants, each undone, each added a 7th problem that names it: a glob that matches nothing (`nothing-here/**`), the `.mcp.json` entry removed from `MAY_MATCH_NOTHING`, and a staged `.claude/agents/acme-reviewer.md` with no row.
* [x] Step 6: acceptance runs and the route measurement.  2026-10-23, binary `target/debug/xtask` built from `a8566d2`.
  * The worktree check: `git worktree add target/review/m1-ms11 ai-agent-harness-phase-2-ms11`, then in it `<main checkout>/target/debug/xtask review start 7cd8518 1`: exit 0.  The branch route names exactly `reviewer`, `memory-reviewer`, `supply-chain-reviewer` and `docs-reviewer`.  With an untracked `x.txt`: exit 3, "commit first: the tree has changes", then `?? x.txt`.  The worktree is removed.
  * The mutants checks, in `target/review/m1-mutants` on the scratch branch `m1-scratch-mutants` from `a8566d2`, with signed commits.  Each `review start` warned that the binary is older than `review.rs`, as expected.  Before each run, `cargo mutants --in-diff <diff> --list` gave 9, 2 and 16 mutants.
  * Full: `b2a5161` writes the comparison in `dsp::biggest` the other way.  `review start a8566d2 1`, then `review mutants 1`: exit 0, "mutants: full: tested 9 (caught 8, missed 1, unviable 0), timeouts 0, not tested 0", in 17 s.
  * Caught behind `cpal`: `22c7537` lists the stream errors in `is_fatal` in another order.  `review start a8566d2 2` (since `b2a5161`), then `review mutants 2`: exit 0, "mutants: full: tested 2 (caught 2, missed 0, unviable 0), timeouts 0, not tested 0", in 159 s.  `mutants.out/caught.txt` names `replace is_fatal -> bool with true` and `with false`.
  * Partial: `270a323` swaps the operands of 5 sums and products in 3 functions of `engine/src/player.rs` (16 mutants).  `review start a8566d2 3`, then `STEMYLA_MUTANTS_MINUTES=4 review mutants 3`: exit 0, "mutants: full: tested 16 (caught 14, missed 2, unviable 0), timeouts 0, not tested 0", in 207 s, since the baseline built in 70 s.  Again with `STEMYLA_MUTANTS_MINUTES=2.5`: exit 0, "mutants: partial: tested 5 (caught 4, missed 1, unviable 0), timeouts 0, not tested 11; stopped at the limit", in 152 s.  Then `pgrep -fl 'cargo-mutants|m1-mutants'` showed no process of the run, and `ls "$TMPDIR" | grep cargo-mutants-m1-mutants-` showed no copy.  `mutants.log` ends with "Error: interrupted".  The worktree is removed; the branch `m1-scratch-mutants` stays for the orchestrator.
  * The route measurement, in a detached worktree `target/review/m1-measure` (removed after), with `review start <sha>^ 1` at each squash commit (all exit 0).  Reviewers by PR: #13 6 (all); #14 4 (reviewer, gpu, supply-chain, docs); #15 4 (the same); #16 4 (the same); #17 3 (reviewer, supply-chain, docs); #18 4 (reviewer, wire, supply-chain, docs); #19 3; #20 4 (reviewer, memory, supply-chain, docs); #21 3; #22 4 (as #20); #23 4 (as #18); #24 2 (reviewer, supply-chain); #25 4 (as #20); #26 6; #27 6; #28 3; #29 3; #30 4 (as #18); #31 4 (as #20); #32 5 (reviewer, memory, wire, supply-chain, docs); #33 3.  The 3s are reviewer, supply-chain and docs.  Mean: 83 / 21 = 3.95 reviewers for each PR; 81 / 20 = 4.05 without the Dependabot bump #24.  `supply-chain-reviewer` reads all 21, `docs-reviewer` 20.
* [ ] Step 7: review rounds 1, 2 and 3.
  * Round 1, 2026-10-23, by one new reviewer on `422ebe3..fcf3df5`, with the recipe of `docs/working-with-agents.md` at `fa41c9f`.  0 blockers, 3 should-fix, 3 nits; the team leader checked each against the code, and all 6 were real.  The should-fix: a push or a fetch passed `review check`, since the record held only `refs/heads` and `refs/tags`; four plants in the mutants command passed every test; no test set `core.hooksPath`.
  * Round 1 fixes, 2026-10-23, `04452fa`.  The state record holds `other_refs`; `parse_limit` refuses `1e300` with exit 3; the old-binary warning names `current_exe`; scratch repos are removed by a guard, and set their own `core.hooksPath`.  11 new tests (95 in all).  Plants, each undone, each failed its test: `other_refs` not compared, `refs/stash` kept, the hooks folder as `common/hooks`, a 60 s default limit, no `--gitignore=true`, the diff from `base`, `CARGO_TARGET_DIR` kept, `from_secs_f64` (it panics), the warning blobs both from `HEAD`, no main-worktree root, and no octal escapes.  The full check ran again with the new binary, detached at `b2a5161`: "mutants: full: tested 9 (caught 8, missed 1, unviable 0), timeouts 0, not tested 0", in 17 s; no copy left.  528 old `xtask-review-*` folders in `$TMPDIR` were removed.
  * Round 2 fixes, 2026-10-23, `a75a730`.  `run_with` takes the program for `cargo mutants`, and a fake `cargo` in POSIX sh runs the whole `mutants` subcommand; scratch repos make their hooks folder.  6 new tests (100 in all), and 2 stronger ones (the since test, the start test).  Plants, each undone, each failed its test: no `mutants.txt`, the counts read from the round folder, `None` for `--config`, `--config` against the toplevel, `installed` forced, the log check in every round, the branch route and `branch.diff` from `since`, no clear in `prepare_mutants`, `started: 0`, and a failed start that is not `missing`.  With an empty `GIT_TEMPLATE_DIR`, 100 tests pass; without the `create_dir_all`, 42 fail.  TD-32 and the guide's row are corrected.  The full check ran again with the new binary, detached at `b2a5161`: "mutants: full: tested 9 (caught 8, missed 1, unviable 0), timeouts 0, not tested 0", in 19 s; no copy left.

## Surprises and discoveries

* 2026-10-23, implementer: the cold baseline of an `engine` run built in 70 s and tested in 7 s, not the 140 s of the `xtask` tests.  16 `engine` mutants finished in 207 s.  So `STEMYLA_MUTANTS_MINUTES=4` gave `full`, and the partial check needed 2.5 (150 s).  A partial check needs a limit between the baseline and the whole run, for the crate that the diff changes.
* 2026-10-23, implementer: with `--in-diff`, cargo-mutants also tests the mutants that replace a whole function whose body holds a changed line.  So a one-line change in `dsp::biggest` gave 9 mutants, 3 of them for the whole function.
* 2026-10-23, implementer: a plant that sent no SIGKILL made the SIGKILL test hang, not fail.  The test now waits 15 s at most, and its shell ends by itself after 30 s.
* 2026-10-23, implementer: two plants first passed every test: the dedupe of reasons (step 1), and `since.diff` from the merge base (step 3).  Each got a new or stronger test that fails with the plant.

## Decision log

* 2026-10-23, team leader: every failure of `xtask review` exits 3, its own errors included, so a skill stops on each.  The parent plan names exit 3 for a refusal only.
* 2026-10-23, team leader: the unit tests live in `xtask/src/review.rs`, so a change to them routes every reviewer, as a change to the code does.
* 2026-10-23, team leader: the state record also holds `started`, so M3 can count wall minutes from `review start`.  `check` does not compare it.
* 2026-10-23, team leader: `review start` prints at most 5 path reasons for each reviewer, and writes all reasons to `route.txt`.  So a long diff gives a short prompt.
* 2026-10-23, implementer: the line of `review mutants` prints the reason of each class that has one.  So exit 3 prints "; timeouts have no result".  Step 4 gives exit 3 that reason, but printed reasons only for `none` and a stop.
* 2026-10-23, implementer: if `cargo` does not start, `run_with_limit` reports cargo-mutants as missing, since `RunEnd` has no other way to say that nothing ran.
* 2026-10-23, implementer (review round 1): `warnings` also takes the path of the binary, so the old-binary warning names the binary that runs (`current_exe`), not always `target/debug/xtask`.  `review mutants` builds its command with the pure `mutants_args` and `mutants_command`, reads its limit with `parse_limit`, and prepares the round with `prepare_mutants`, so tests reach each part without cargo-mutants.
* 2026-10-23, implementer (review round 2): `run` calls `run_with`, which also takes the program for `cargo mutants`.  So a test runs the whole `mutants` subcommand with a fake `cargo` in POSIX sh.  `mutants_command` takes that program too.
* 2026-10-23, implementer: the partial check ran round 3 twice.  The first limit, 4 minutes, was above the whole run, so the line said `full`.  The second, 2.5 minutes, is above both the 140 s of the plan and the 77 s baseline measured, and gave `partial`.

## Outcomes and retrospective

To be written when M1 completes.
