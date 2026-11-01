# Harness phase 3b, M4: local checks by tier (decision 0026)

## Purpose

This is the milestone plan for M4 of `plans/active/harness-phase-3b.md`.  It gives more detail than the parent plan's M4 section, which stays the criteria.  M4 puts decision 0026 into the harness.

After M4:

* `cargo run -p xtask -- check --tier commit` reads the changed paths of a commit.  It picks the checks that those paths need, prints what it runs and skips and why, and runs them.
* `cargo run -p xtask -- check --tier push` runs the fmt, clippy, test, `docs-check` and Trunk build lines of `CLAUDE.md`'s Commands.  The agent runs it before each push and each review round.
* A commit that touches the paths of a strict rule runs tier `push` (D2 of the parent plan).
* GIT-2 in `CLAUDE.md` and `docs/invariants.md`, and the `review-rounds` skill, name the tiers.
* No check of `xtask check` is a `serve`, `web-test`, `e2e` or `publish` command, and no check starts a browser.  CI stays the authority.

Readers: the implementer of M4, the review agents of M4's rounds, and the orchestrator.

## Context and orientation

The state at the start of M4, on 2026-11-01, at commit `e474fb2` on branch `harness-phase-3b`:

* GIT-2 (`docs/invariants.md`, near line 159) says that fmt, clippy, the tests and `docs-check` pass before every commit.  `CLAUDE.md` (88 lines; the map limit is 100) says the same on its GIT-2 line (line 78).
* `CLAUDE.md`'s Commands block has these lines that M4 runs: `cargo fmt --all --check`; five `cargo clippy` lines (workspace, `synth`, `app` twice, wasm32); the workspace test line `cargo test --workspace --exclude roformer-gpu`; `cargo test -p roformer-gpu --lib`; `cargo run -p xtask -- docs-check`; `(cd app && trunk build --release)`.  The comment of the `--lib` line says that CI also runs `--doc`.  CI's `test` job runs `cargo test --locked -p roformer-gpu --doc`.
* The GPU test line `STEMYLA_REQUIRE=gpu,f16 cargo test -p roformer-gpu --release` needs a real GPU.  CI does not run it.  This Mac has a real GPU.
* Lines that M4 never runs: `serve`, `web-test`, `e2e` and `publish`.
* `.claude/agents/baseline.md` (line 17) also says "fmt, clippy, the tests and `docs-check`".  `BASELINE_HASH` (`xtask/tests/rules.rs`, near line 1003) freezes that file, and the parent's M5 keeps it as it is.
* Other places that state the GIT-2 checks: the recipe "Fix a bug" (`docs/working-with-agents.md`, near line 43), and "the checks CI gates on" in `README.md` (near line 63).
* The `review-rounds` skill (`.claude/skills/review-rounds/SKILL.md`) says "Run this skill only after fmt, clippy, the tests and `cargo build -p xtask` pass." (Before the steps).  Step 10 says "Before the commit, run fmt, clippy, the tests and `docs-check` (GIT-2)."  `REVIEW_SKILL_TEXT` in `xtask/tests/rules.rs` pins skill lines.
* `xtask/src/review_log.rs` has `strict_rules()`, the 15 IDs of decision 0024 (D3).  M4 builds the strict-rule path table on it, and does not copy the list.
* `xtask/src/docs.rs` has `glob_matches(glob, path)`, which `xtask/src/review.rs` uses for its routes.
* `xtask/src/rules.rs` has:
  * `AUDIO_EXTENSIONS` and `WEIGHT_EXTENSIONS` (near line 1656), private, for ASSET-1;
  * `workspace(metadata)` (near line 488), which reads `cargo metadata` JSON into each member and its edges to other members, with their kinds.  It needs `--all-features` and no `--no-deps`.
  * `workspace_metadata(root)` (near line 387), which runs `cargo metadata --format-version 1 --all-features --locked`.  With `--locked`, cargo never writes `Cargo.lock`.
* `xtask/src/main.rs` dispatches each subcommand.  `docs-check` fails if `docs/working-with-agents.md` does not name a subcommand in a code span (`guide_covers`).
* `xtask/src/clean.rs` is a model for M4's shape: `gather` reads the repo, a pure function decides, and `main` gives an exit code (1 failed, 2 refused).
* The workspace has 11 members (`Cargo.toml`).  Three packages have names that differ from their folders: `worker` is `stem-worker`, `decoder` is `decode-worker`, `protocol` is `stem-protocol`.
* Cargo turns on features across the packages of one run only.  `engine/src/lib.rs` builds `live.rs` and its tests only with the `cpal` feature, and only `app`'s default feature `audio` turns it on.  So `cargo test -p engine` alone skips those tests.
* One test reads a file in another member's folder: `engine/tests/realtime.rs` (near line 674) reads `app/Cargo.toml`.  The tests of `xtask` read the whole repo: `xtask/tests/rules.rs`, `xtask/tests/docs.rs`, `xtask/tests/review.rs` and unit tests in `xtask/src/`.
* Hand-written worker messages are in `xtask/e2e/worker.js`, `xtask/e2e/save_spy.js`, `xtask/src/e2e.rs`, `xtask/src/e2e/decode.rs` and `xtask/src/e2e/weights.rs`.  `e2e_messages_use_protocol_types` reads `xtask/e2e/*.js`, `xtask/src/e2e.rs` and `xtask/src/e2e/**`.
* MEM-4's model budget: `CHUNK` in `roformer/src/lib.rs` (line 44), `roformer/src/weights.rs`, and `WeightFormat` in `roformer-gpu/src/lib.rs` (line 245).
* The default kernels: `calibrate` and `decide` in `roformer-gpu/src/bench.rs` choose them; `TILES` and `DEFAULT_WEIGHTS` are in `roformer-gpu/src/lib.rs` (`.claude/rules/gpu.md`).
* The audio callback also calls `StemSource::read` in `engine/src/source.rs` (RT-1).
* `app/src/streamtest.rs` also writes to OPFS (MEM-5).
* Times on this Mac, warm: `cargo fmt --all --check` 0.4 s; workspace clippy 0.4 s; `docs-check` 1.1 s; `cargo test -p xtask` 64 s (of which `m4_stretch` 24 s and `m8_hpss` 16 s).  The `--lib`, `rules`, `docs` and `review` targets of `xtask` run in about 8 s.
* PR #39 has 194 commits, no merges, from `4a5b356` (its merge base with `origin/main`) to `refs/remotes/origin/pr/39` (`4068c49`).
* The last TD is TD-69.  No new crate is needed (DEPS-1).

Terms:

* Changed path: a file path, relative to the repo root, that the tier reads from git.
* Member: a workspace crate.  A path is in a member when it starts with the member's folder and `/`.  So `roformer-gpu/src/lib.rs` is in `roformer-gpu`, not in `roformer`.
* Workspace file: a path outside every member that changes every build.  These are `Cargo.toml`, `Cargo.lock`, `.cargo/config.toml`, `.cargo/config`, `rust-toolchain`, `rust-toolchain.toml`, `clippy.toml`, `rustfmt.toml` and `.rustfmt.toml`, at the root only.
* Dependency closure of a set of members: the set, and each member that it depends on, again and again.  The edges are normal, dev and build dependencies, for any target.
* Dependents of a set of members: the set, and each member that depends on one of them, again and again, through the same edges.
* Strict path: a glob of the strict-rule path table.
* Line: a command as `CLAUDE.md`'s Commands gives it, without the comment.

## Plan of work

### How the team works

* The team leader writes this plan.  A new `Plan` agent reviews it, at most 2 rounds, and the team leader fixes what it finds.  The team leader commits the plan and its index line, signed, before step 1.
* The team leader starts one implementer (`general-purpose`) with this plan's path.  The implementer does steps 1 to 4.
* Each step is one or more signed commits (`git commit -S`).  Before each commit, run these (GIT-2, as it is before the orchestrator lands step 4):
  * `cargo fmt --all --check`;
  * `cargo clippy --workspace --all-targets -- -D warnings`;
  * `cargo test -p xtask`;
  * `cargo run -p xtask -- docs-check`.
* M4 changes only `xtask` and docs, so the other clippy lines of `CLAUDE.md` do not apply.  From step 2, the implementer also runs `cargo run -p xtask -- check --tier commit --dry-run`.  It records in Surprises each check that the dry run adds to the list above.
* A new test fails first on an assert, not on a compile error.  So the implementer first writes each new pub function as a stub that gives a wrong answer:
  * `lines` gives no check, and `Check::line` gives an empty text.
  * `STRICT_PATHS` is empty, and `strict_hits` gives no hit.
  * `Graph::closure` and `Graph::dependents` give the set that they get.
  * `plan` gives a plan with no check.
  * `changed_paths` gives no path.
  * `Graph::from_workspace` gives a graph with no member.
  * `run_checks` runs no check, prints nothing and gives 0.
  * `main` prints nothing and gives 0.
* Each test checks the whole output, so each test fails on its stub.  The implementer runs the new tests on the stubs, and records the failures in Surprises.  Then it writes the real code, and commits the tests and the code together.  No commit holds a failing test (GIT-2).
* No test runs `xtask check` without `--dry-run` in this repo.  Reason: `check` runs `cargo test`, which runs the tests again.
* The implementer does not edit `.claude/**`, `CLAUDE.md` or `docs/plans/active/harness-phase-3b.md`.  Step 4 gives the team leader one diff file for the orchestrator.
* Docs and commit messages are in ASD-STE100, as `CLAUDE.md` says.

### Step 1: the tables and the plan, as pure functions

Add `xtask/src/check.rs`, and `pub mod check` in `xtask/src/lib.rs`.  Its doc comment names decision 0026 and this plan.

The lines.  `pub fn lines() -> Vec<Check>` gives the 12 lines of tier `push`, in this order:

* fmt (1);
* `docs-check` (2);
* the five clippy lines (3 to 7);
* the workspace test line (8);
* `cargo test -p roformer-gpu --lib` (9);
* `cargo test -p roformer-gpu --doc` (10);
* the GPU test line (11);
* the Trunk build (12).

* A `Check` has an argv, an environment (only `STEMYLA_REQUIRE=gpu,f16` for the GPU line), a folder (the repo root, or `app` for Trunk) and a `line()`.
* `line()` is the environment and the argv, joined by spaces.  For the folder `app`, it is `(cd app && <argv>)`.  So the line that `xtask check` prints is the command that it runs.
* `cargo test -p roformer-gpu --doc` is not a line of `CLAUDE.md` today.  Step 4 adds it, so `CLAUDE.md` names each line that tier `push` runs.

The strict-rule path table.  `pub const STRICT_PATHS: &[(&str, &[&str])]` has one row for each ID of `review_log::strict_rules()`, in the same order.  VERIFY-1 and GIT-1 have explicit empty rows.  `pub fn strict_paths(rule)` reads from it.  The rows name the files that hold the code of each rule, narrower than the `paths:` of `.claude/rules/`.  The implementer checks each row with `grep` before it commits:

| Rule | Strict paths | The code there |
| - | - | - |
| PROTO-1 | `protocol/src/**`, `xtask/e2e/*.js`, `xtask/src/e2e.rs`, `xtask/src/e2e/**` | the encoders and decoders; the hand-written worker messages, as `e2e_messages_use_protocol_types` reads them |
| PROTO-2 | `protocol/src/lib.rs`, `app/src/separator.rs`, `worker/src/web.rs` | the site check of `?model_url=`, and its two callers |
| MEM-1 | `engine/src/song.rs`, `**/clippy.toml` | `Samples`; the `disallowed-methods` lints |
| MEM-2 | `protocol/src/**`, `app/src/separator.rs`, `app/src/decoder/client.rs`, `worker/src/web.rs`, `decoder/src/web.rs` | the transfer lists |
| MEM-3 | `roformer-gpu/src/lib.rs`, `roformer-gpu/clippy.toml` | `GpuBuffer`, `Gpu` and its lints |
| MEM-4 | `roformer/src/lib.rs`, `roformer/src/weights.rs`, `roformer-gpu/src/lib.rs` | `CHUNK`; the weight format (`WeightFormat`) |
| MEM-5 | `app/src/store.rs`, `app/src/export.rs`, `app/src/streamtest.rs`, `worker/src/web.rs`, `worker/src/web/encode.rs`, `worker/src/web/stems.rs`, `decoder/src/web.rs` | the OPFS writes |
| RT-1 | `engine/src/slot.rs`, `engine/src/source.rs`, `engine/src/player.rs`, `engine/src/live.rs`, `engine/src/readahead.rs`, `engine/src/readahead/**`, `engine/src/stretch/**`, `engine/src/test_alloc.rs` | the audio callback, its slot and sources, the read-ahead buffer, the warm-up of the stretchers, the counting allocator |
| GPU-1 | `roformer-gpu/src/shaders/**`, `roformer-gpu/src/lib.rs` | the kernels; `Limits::defaults()` and the layouts |
| GPU-2 | `roformer-gpu/src/shaders/**`, `roformer-gpu/src/bench.rs`, `roformer-gpu/src/lib.rs` | the kernels; `calibrate` and `decide`, which choose the default kernels; `TILES` and `DEFAULT_WEIGHTS` |
| GPU-3 | as GPU-2 | as GPU-2 |
| DEPS-1 | `**/Cargo.toml`, `Cargo.lock`, `THIRD-PARTY.md` | the dependencies and their licences |
| ASSET-1 | `.gitignore`, and each path with an extension of `AUDIO_EXTENSIONS` or `WEIGHT_EXTENSIONS` | what keeps audio and weights out |
| VERIFY-1 | none | VERIFY-1 is on 870 of 1,402 findings (parent plan) |
| GIT-1 | none | GIT-1 has no code |

* `app/src/lib.rs` (340 KB) is in no row, but it holds `MEMORY_PATH_MAX_FRAMES` and some transfer code.  Most commits of the app touch it, so a row with it would send most commits to tier `push`.  The comment of the table says so.
* The implementer makes the two extension lists of `rules.rs` `pub(crate)`, with a function `is_audio_or_weight(path)` that both files use.  It reads the extension in lower case, as `assets_in_the_index` does.
* The table holds globs only.  `strict_hits` adds the ASSET-1 extension case.

`pub fn strict_hits(paths: &[String]) -> Vec<(String, String)>` gives each (rule, path) pair where a path matches a strict path of a rule.  The order is that of `strict_rules()`, then of the paths.

The workspace graph.  `pub struct Graph` holds each member's folder and package name, and its edges to other members (all kinds, all targets).  `Graph::closure(set)` and `Graph::dependents(set)` give the sets of the Terms.  `Graph::from_workspace(&rules::Workspace)` makes a `Graph`, and a test can also make one from a list of members and edges.

The plan.  `pub fn plan(tier: Tier, paths: &[String], graph: Option<&Graph>) -> Plan`.  A `Plan` holds the tier that runs, the strict hits, and each check with `run` or `skip` and a reason.  Tier `commit` with no graph runs tier `push`, since it cannot choose the crates.  The path-to-check table of tier `commit`:

| Row | When | Check |
| - | - | - |
| fmt | always | `cargo fmt --all --check` |
| `docs-check` | always | `cargo run -p xtask -- docs-check` |
| workspace clippy | always | `cargo clippy --workspace --all-targets -- -D warnings` |
| `xtask` repo tests | `xtask` is not one of the dependents of the changed members | `cargo test -p xtask --lib --test rules --test docs --test review` |
| `synth` clippy | a changed member is in the closure of `synth` | `cargo clippy -p synth --all-targets -- -D warnings` |
| `app` clippy | a changed member is in the closure of `app` | both `app` lines |
| wasm32 clippy | a changed member is in the closure of the line's five packages | the wasm32 line |
| crate tests | a changed member | one command, `cargo test -p <a> -p <b> ...`, for the dependents of the changed members, sorted, but `roformer-gpu` |
| `roformer-gpu` tests | `roformer-gpu` is one of those dependents | `cargo test -p roformer-gpu --lib` and `--doc` |
| workspace file | a workspace file changed | every line of tier `push` but the GPU test line and the Trunk build |
| strict path | a strict hit | tier `push` (D2) |

* The crate tests are one command, so cargo turns on features across those packages, as in the workspace test line.  A test checks that they are one `Check`.  A change to `engine` has `app` in its dependents, so `engine`'s `cpal` tests run.
* When the crate tests cover each member but `roformer-gpu`, the check is the workspace test line.
* With no changed path, the plan runs the first four rows only.

Tier `push` runs every line of `lines()` but one case of the GPU test line:

* The GPU test line runs when the branch's paths change `roformer/`, `roformer-gpu/`, `test-support/` or a workspace file.  Its tests test only those folders.
* Else the plan skips it.  The reason says that the branch changes no GPU code.
* The paths also give the strict hits of the branch, for the report.

The skips that the plan always prints, each with its reason:

* In tier `commit`, the GPU test line: it needs a real GPU, and tier `push` runs it.  GPU-2 asks for it on a kernel change.
* `serve`, `web-test`, `e2e`: CI runs the browser tests.  Run a scenario by hand only when CI failed it (decision 0026).
* `publish`: a person only (GIT-1).
* In tier `commit`, the Trunk build: tier `push` runs it.

Tests of step 1, in `xtask/src/check.rs`, on a fixture graph of the 11 members and their real edges:

* One test for each row of the path-to-check table.  Each one gives paths, and checks the whole list of run and skip checks.  Among them:
  * `docs/x.md` gives fmt, `docs-check`, workspace clippy and the `xtask` repo tests, and no test of a changed crate.
  * `dsp/src/lib.rs` gives one test command for the dependents of `dsp`, `xtask` included, so the `xtask` repo tests do not run.  It gives the `app` and wasm32 clippy lines, and no `synth` clippy line.
  * `synth/src/lib.rs` gives the `synth`, `app` and wasm32 clippy lines.
  * `roformer/src/x.rs` is in `roformer`.  `roformer-gpu/src/x.rs` is in `roformer-gpu` only.  It gives its `--lib` and `--doc` lines, `cargo test -p stem-worker`, and the wasm32 clippy line, since `worker` depends on `roformer-gpu`.
  * `engine/src/decode.rs` gives one command that holds `engine` and `app`.
  * Each member whose dependents are all 11 members gives the workspace test line.  The implementer finds these members from the real graph, such as `dsp` and `test-support`.
  * `.cargo/config.toml` gives every line but the GPU test line and Trunk.
  * `Cargo.lock` gives tier `push`.  No graph gives tier `push` too.
  * No path gives the first four rows only.
  * Each of these tests checks the whole expected list, written out.
* `the_fixture_graph_is_the_real_graph`: the fixture graph equals `Graph::from_workspace` of this repo, through `rules::workspace_metadata`.  So a new edge in a `Cargo.toml` fails it.
* One test for each row of the strict-rule path table: a path that hits the rule, and a near path that does not.  Each path that the plan review added (`engine/src/source.rs`, `roformer-gpu/src/bench.rs`, `app/src/streamtest.rs`, `xtask/src/e2e/weights.rs`) has a hit test.  `wav/a.WAV` hits ASSET-1.
* `the_strict_path_table_follows_strict_rules`: the ID column of `STRICT_PATHS` equals `strict_rules()`, in order.  So a change to decision 0024's list fails it.  The rows of VERIFY-1 and GIT-1 are empty, and each other row is not empty.
* `lines_are_the_twelve_of_the_push_tier`: `lines()` gives the 12 lines of the list above, by `line()`.
* Tier `push` with `roformer-gpu/src/x.rs` runs the GPU test line.  Tier `push` with `docs/x.md` skips it with its reason.
* `check_never_runs_the_browser_or_publish`: each tier's plan runs at least one check.  For each tier, and for all the paths of the tests, no argv element of a `run` check equals `serve`, `web-test`, `e2e` or `publish`.  The test compares whole elements, so `--test e2e_cli` does not match.
* A path with a space, and a deleted path, each give an explicit expected list of checks.

### Step 2: the paths, the graph, the run and the command

In `xtask/src/check.rs`:

* `pub fn changed_paths(root: &Path, tier: Tier) -> Result<(Vec<String>, String)>` gives the paths, and a line that says where they came from.  Each git command runs in `root`, so the paths are relative to the root.  It uses `-z` and `--no-renames`, so a rename gives both paths, and a name with a space stays whole.
  * Tier `commit`: `git diff --name-only --no-renames -z HEAD` gives the staged and unstaged changes.  `git ls-files --others --exclude-standard -z` gives the untracked files.
  * Tier `commit` on a clean tree, when `HEAD^` exists: `git diff --name-only --no-renames -z HEAD^ HEAD`.  This also covers a merge commit, against its first parent.
  * Tier `commit` on a clean tree with a root commit: `git diff-tree -r --root --no-commit-id --name-only --no-renames -z HEAD`, each of its files.
  * Tier `push`: `git diff --name-only --no-renames -z <base>`, and the untracked files.  `<base>` is `git merge-base HEAD origin/main`.
  * Tier `push` with no `origin/main`, or when `git merge-base` fails (a shallow clone, no common commit): it prints a `warning:` line.  Then it goes on with no paths, since tier `push` runs every line.
  * With no `HEAD` (a repo with no commit), it gives an error.
* `main` finds the strict hits first.  It reads the graph only for tier `commit` with no strict hit, since tier `push` needs no graph.
* `pub fn graph(root: &Path) -> Result<Graph>` runs `rules::workspace_metadata(root)` and `rules::workspace`, then `Graph::from_workspace`.  It does not copy the runner or the parser.  `--locked` stays, so `check` never writes `Cargo.lock`.
* If `graph` fails, such as on a stale `Cargo.lock`, `main` prints a `warning:` line with the error.  Then `plan` gets no graph, and runs tier `push`.
* `pub fn run_checks(plan: &Plan, out: &mut impl Write, run: impl FnMut(&Check) -> Result<Outcome>) -> i32` runs each `run` check in order.  An `Outcome` is an exit code, or "killed by a signal".
  * Before each check it prints `check: start: <line>`.  After it, it prints `check: ok in <s> s`, or `check: failed (<exit n or a signal>) in <s> s`.  So a long run is never silent.
  * It stops at the first failure, names each check that did not run, and gives 1.  A runner error, such as a program that cannot start, is a failure with its error.
  * Else it prints `check: <n> passed in <s> s`, and gives 0.
* `pub fn main(args: &[String]) -> i32`: `check --tier commit|push [--dry-run]`.
  * It reads the repo root with `git rev-parse --show-toplevel` in the current folder.
  * It prints the tier, where the paths came from and the number of paths.  Then it prints each strict hit as `check: strict <rule>: <path>`.  Then it prints each check as `check: run: <line> (<reason>)` or `check: skip: <line> (<reason>)`.  Each line of `check` starts with `check:` or `warning:`.
  * With `--dry-run`, it stops there, and gives 0.
  * A usage error, a git error or a `cargo metadata` error prints `USAGE` or the error, and gives 2.
  * The real runner starts each argv with `std::process::Command` in its folder.  The output goes to the terminal.
* `xtask/src/main.rs`: the dispatch `"check" => std::process::exit(check::main(&args[1..]))`, and `check` in the usage line.

Tests:

* In `xtask/src/check.rs`:
  * `Graph::from_workspace`: `rules::workspace` on a fixture JSON with three members, a dev edge, a target edge and a registry crate, then the `Graph` of it.
  * `plan` with no graph in tier `commit` and a docs path gives tier `push`.
  * `run_checks` with a fake runner, on the whole text of `out`.  All pass.  The second check fails, so the third does not run and is named.  A runner error is a failure.  A signal is a failure.
* In `xtask/tests/check.rs`, on temporary repos, as `xtask/tests/clean.rs` does (`GIT_ENV`):
  * an unstaged change, a staged change and an untracked file each give their path;
  * a clean tree gives the paths of `HEAD^..HEAD`, and a root commit gives each of its files, a nested one included;
  * a clean tree at a merge commit gives the paths against its first parent;
  * a run from a subfolder gives the same paths, relative to the root, untracked files of other folders included;
  * a rename gives both paths, and a name with a space stays whole;
  * tier `push` with a fake `origin/main` ref (`git update-ref`) gives the paths since the merge base, the uncommitted ones included;
  * tier `push` with no `origin/main` gives the `warning:` line;
  * a repo with no commit gives exit 2.
* An end-to-end test of the binary (`CARGO_BIN_EXE_xtask`, with `GIT_ENV`):
  * It makes a temporary folder under `std::env::temp_dir()`, with its own `[workspace]` and one small crate that has no dependencies, as `xtask/tests/e2e_cli.rs` does.  So cargo needs no network, and does not walk up to this repo.
  * It commits the crate and its `Cargo.lock`, then changes one `.rs` file so that `cargo fmt --all --check` fails.
  * It sets `CARGO_TARGET_DIR` to a folder in the temporary folder, so the nested cargo does not wait on the outer build lock.
  * `check --tier commit --dry-run` gives 0, prints the whole plan and changes no file (`git status --porcelain` is empty but the one change).
  * `check --tier commit` runs fmt, which fails.  It gives 1, and names each check that did not run.
* `check --tier nothing` gives 2 and the usage.

Docs, in the same commit (DOC-1):

* `docs/working-with-agents.md`: a Tools row for `check`, and a changelog line for M4.
* `docs/tech-debt.md`: a new TD, the next free ID after TD-69, for the gaps of tier `commit`:
  * The crate tests unify features over the chosen packages only, so they can pass where the workspace test line fails.
  * A test that reads a file of another folder is not seen.  The one case today, `app/Cargo.toml` for `engine`, matches DEPS-1, so it goes to tier `push`.
  * The strict paths are a narrow list that a person keeps.  `app/src/lib.rs` is in no row.
  * Tier `push` or CI finds each of these before review (decision 0026).

### Step 3: the measurements

* The share of PR #39's commits that the strict-rule path table sends to tier `push`:
  * Make one scratch worktree under `target/` with `git worktree add --detach`.
  * For each of the 194 commits of `4a5b356..refs/remotes/origin/pr/39`, run `git checkout --detach <commit>` in it.  Then run `<repo>/target/debug/xtask check --tier commit --dry-run` there.  `cargo run` in the old tree would build a binary with no `check`.
  * A clean tree reads the commit's own change.
  * Count the commits with a `check: strict` line, and the hits of each rule.
  * Remove the worktree with `git worktree remove`.
* If the share is more than half, list the rows that cause most hits.  Propose narrower paths in the Decision log.  The team leader decides; the review checks it.
* A timed run of each tier on the same change:
  * Use the last commit of step 2, with a warm `target/`.
  * Run `time cargo run -p xtask -- check --tier commit`, then `time cargo run -p xtask -- check --tier push`.
  * Record both times, and the time of each check, in Surprises.
* Commit the plan's Surprises, signed.

### Step 4: the diff for the orchestrator (DOC-1)

The implementer writes one diff file, not a commit, and gives its path to the team leader.  It holds each text that states GIT-2, so GIT-2 changes in one commit (decision 0026, Consequences).  Before it writes the diff, the implementer greps the repo for "fmt, clippy, the tests" and lists each place in Surprises.

`CLAUDE.md`:

* The GIT-2 line: before each commit, run `cargo run -p xtask -- check --tier commit`.  Before each push and each review round, run `--tier push`.  Sign every commit.
* Commands: a line `cargo run -p xtask -- check --tier commit` and a line `cargo run -p xtask -- check --tier push`, each with a short comment.  The `--lib` line of roformer-gpu becomes two lines, `--lib` and `--doc`.
* The "Review:" line: run `check --tier push` and `cargo build -p xtask` before `/review-rounds`.
* The file stays at 100 lines or fewer.

`.claude/skills/review-rounds/SKILL.md`:

* Before the steps: "Run this skill only after `cargo run -p xtask -- check --tier push` and `cargo build -p xtask` pass."
* Step 10: "Before the commit, run `cargo run -p xtask -- check --tier commit` (GIT-2)."
* After the last round, "Run the checks" names `check --tier push`.

`docs/invariants.md`, GIT-2:

* The rule names the two tiers and the strict-path rule (D2).  It says that no check is a browser test or `publish`.
* Enforced by: CI, as today.  Also the tests of steps 1 and 2 by name, among them `the_strict_path_table_follows_strict_rules` and `check_never_runs_the_browser_or_publish`, and the two new pins below.
* Gaps: TD-20 and the new TD of step 2.

`docs/working-with-agents.md`: the recipe "Fix a bug" names `check --tier commit`.  `README.md`: "the checks CI gates on" names `check --tier push`.  Each other place of the grep, but these:

* `.claude/agents/baseline.md` stays as it is.  `BASELINE_HASH` freezes it, and the parent's M5 keeps it.  The implementer records this in Surprises.
* Plans in `docs/plans/completed/` and logs in `docs/reviews/` stay as they are (PLANS.md).

`xtask/tests/rules.rs`:

* `REVIEW_SKILL_TEXT` gets the new skill lines, and its doc comment names them.
* `the_push_tier_runs_the_lines_of_claude_md`:
  * Each `line()` of `check::lines()` is a line of `CLAUDE.md`'s Commands block, without its comment.
  * Each line of the block that starts with `cargo fmt`, `cargo clippy`, `cargo test`, `STEMYLA_REQUIRE=`, `cargo run -p xtask -- docs-check` or `(cd app && trunk build` is in `lines()`.
  * A line that starts with `cargo run -p xtask -- check` is not in that set.
  * Each missing line gives its own line in the failure.
* `git_2_names_the_tiers`: the GIT-2 line of `CLAUDE.md` and the GIT-2 section of `docs/invariants.md` each hold `check --tier commit` and `--tier push`.

The implementer checks the diff in a scratch worktree at the last commit of step 3:

* `git apply --check` passes.
* With only the `xtask/tests/rules.rs` part, `cargo test -p xtask --test rules` fails on an assert of each new pin.
* With the whole diff, `cargo run -p xtask -- check --tier push` passes, and `CLAUDE.md` has at most 100 lines.
* The implementer records the failures in Surprises.

The team leader puts the diff in the orchestrator's scratchpad folder `m4/`, checks `git apply --check` again on `HEAD`, and gives it at the checkpoint before round 1.  So round 1 runs with the new GIT-2.

### Step 5: review rounds

* Run `cargo run -p xtask -- check --tier push` and `cargo build -p xtask`.  Then run `/review-rounds e474fb2 <round> "<criteria>"` from round 1, with no `--baseline`.
* The criteria are the parent plan's M4 section at the commit before round 1, and this plan.
* The rounds follow decision 0024: the fix rule of step 10 of the skill, the late range of step 3 and the stop rule of step 11.  There is no cap.
* The team leader fills the `Significant` field and column.  It writes the log text to `m4/log.md` in the scratchpad, and runs `target/debug/xtask review log-check` on that file.
* The Write tool refuses `findings.md` and `log.md` to the team leader (M1 to M3).  So the team leader runs each round to step 11: the fixes, the commit, `review log-check` and `review clean <round>`.  Then it gives the orchestrator the files, and waits.
* Each fix of behaviour gets a test that fails first.  A fix of `.claude/**` or `CLAUDE.md` goes to the orchestrator as a diff.
* A mutant that the team sends to tech debt gets a `KNOWN_MISSED` entry in M4's last commit.

### Changes to the parent plan

The team leader gives these to the orchestrator as text:

* The M4 Progress line, with the stats of the rounds.
* Surprises: the share of PR #39's commits sent to tier `push`, and the timed runs of the two tiers.
* Decision log lines, agent: one line for each entry of this plan's Decision log that changes what the parent's M4 section says.
* For the PR's "Needs your attention": tier `push` skips the GPU test line when the branch changes no GPU code.  Decision 0026 names each test line, so the maintainer confirms this reading.  CI does not run that line either.

## Validation and acceptance

* `cargo test -p xtask` passes with the tests of steps 1 and 2.  Each test failed on its stub, on an assert (Surprises).
* The parent's acceptance:
  * Unit tests of the path-to-check table and of the strict-rule path table, one for each row (step 1).
  * A timed run of each tier on the same change, in Surprises here and in the parent's Surprises (step 3).
  * The share of PR #39's commits that go to tier `push` (step 3).
* `cargo run -p xtask -- check --tier commit --dry-run` on a docs-only change prints no test of a changed crate.  On a change of `engine/src/slot.rs`, it prints `check: strict RT-1: engine/src/slot.rs` and tier `push`.
* With the orchestrator's commit of the diff, `cargo test -p xtask --test rules` passes, and `cargo run -p xtask -- check --tier push` passes on `HEAD`.
* `review log-check` passes on the log after each round.

## Idempotence and recovery

* `check --dry-run` changes nothing.  `check` runs only the build, lint and test lines of `CLAUDE.md`, which write to `target/` and `app/dist/`.  A rerun gives the same plan for the same paths.
* To undo the tiers, revert the orchestrator's commit of the diff.  Then GIT-2 names fmt, clippy, the tests and `docs-check` again, and `xtask check` stays as a tool.
* Each step is its own commit, which `git revert` undoes.
* A new team leader takes over from Progress.

## Progress

* [x] 2026-11-01: plan written (team leader).
* [x] 2026-11-01: plan review round 1 (one `Plan` agent): no blocker, 12 should-fix and 7 nits.  All are fixed in this plan.
* [x] 2026-11-01: plan review round 2 (a new `Plan` agent): no blocker, 7 should-fix and 8 nits.  All are fixed in this plan.  The review stops at its cap of 2 rounds.
* [x] 2026-11-01: step 1: the tables and the plan, as pure functions (implementer, `d327d8d`).
* [x] 2026-11-01: step 2: the paths, the graph, the run and the command (implementer, `32f7a35`).
* [x] 2026-11-01: step 3: the measurements (implementer, `4b7925c`).
* [x] 2026-11-01: step 4, the implementer's part: the diff `m4/m4-git2.diff` in the team leader's scratchpad, checked on `4b7925c` (implementer).  The team leader checks it again on `HEAD`.
* [ ] Step 4: the orchestrator lands the diff.
* [ ] Checkpoint: the orchestrator lands the diff before round 1.
* [ ] Step 5: review rounds to the stop rule.

## Surprises and discoveries

* 2026-11-01, team leader: `cargo test -p xtask` takes 64 s warm, but its `--lib`, `rules`, `docs` and `review` targets take about 8 s.  So tier `commit` runs those targets on each change.  It runs the whole `xtask` tests only when `xtask` or a member that it depends on changes.
* 2026-11-01, plan review round 1: `cargo test -p engine` alone does not build `engine`'s `cpal` tests.  Only `app`'s default feature turns `cpal` on.  So the crate tests of tier `commit` are one command.
* 2026-11-01, plan review round 1: a rough grep of the draft strict table hits 78 of PR #39's 194 commits (40%).  Step 3 measures it with the real table.
* 2026-11-01, implementer, step 1: each new test failed on its stub, on an assert.  The 36 tests are in `xtask/src/check.rs`:
  * `lines_are_the_twelve_of_the_push_tier`: `left: []`, not the 12 lines.
  * The 18 tests of the plan, such as `a_dsp_path_tests_its_dependents_in_one_command` and `tier_push_runs_the_gpu_line_for_a_gpu_path`: `left: []`, not the whole expected list.  `tier_commit_always_skips_the_gpu_line_and_trunk_with_their_reasons`: an empty reason.
  * `check_never_runs_the_browser_or_publish`: "Commit [] runs a check".
  * The 12 tests of the strict-rule rows: "<path> hits <rule>", such as "engine/src/slot.rs hits RT-1".  `verify_1_and_git_1_have_no_path`: the assert that DEPS-1 has paths.  `strict_hits_follow_the_rules_then_the_paths`: `left: []`.
  * `the_strict_path_table_follows_strict_rules`: `left: []`, not the 15 IDs.
  * `the_fixture_graph_is_the_real_graph`: `Graph { members: {} }`.
  * `the_crate_tests_of_every_member_but_roformer_gpu_are_the_workspace_test_line`: the stub of `dependents` gives the set that it gets, so the assert of the 11 members failed.
  * `a_deleted_path_counts_as_a_change_of_its_member` first failed on an index out of bounds, not on an assert.  The implementer changed it to compare the whole list, and it then failed on the assert.
* 2026-11-01, implementer, step 1: no one member of this repo has all 11 members as its dependents.  `dsp` has 5 (`app`, `decode-worker`, `dsp`, `engine`, `xtask`), and `test-support` has 5 (`roformer`, `roformer-gpu`, `stem-worker`, `test-support`, `xtask`).  So the test of the workspace test line uses the paths of four members: `dsp`, `test-support`, `stem-protocol` and `synth`.  The test also checks that no one member gives all 11.
* 2026-11-01, implementer, step 2: each new test failed on its stub, on an assert:
  * `run_checks_runs_each_run_check_and_gives_0`: `left: ""`, not the text.  `run_checks_stops_at_a_failed_check_and_names_the_rest` and `run_checks_fails_a_signal_and_a_runner_error`: `left: 0`, not 1.
  * `run_command_gives_the_exit_code_the_signal_and_a_start_error`: `left: Exit(0)` for the check that must give `Exit(1)`.
  * `the_graph_of_a_workspace_has_its_members_and_edges_of_all_kinds`: `Graph { members: {} }`.  `Graph::from_workspace` was real from step 1, so the implementer made it a stub again for this run.
  * The 7 tests of `changed_paths` in `xtask/tests/check.rs`: `left: ([], "")`.  `a_run_from_a_subfolder_reads_the_whole_repo`: `left: ".../repo/docs"`, the stub of `repo_root`.
  * `a_bad_tier_gives_exit_2_and_the_usage`: `left: Some(0)`.  `the_dry_run_prints_the_whole_plan_and_changes_no_file`: `left: ""`.  `the_dry_run_prints_each_strict_hit_and_tier_push`: `left: []`.
  * `a_repo_with_no_commit_gives_exit_2`, `the_dry_run_of_tier_push_with_no_origin_main_warns` and `a_graph_that_fails_gives_a_warning_and_tier_push` first failed on an `unwrap_err` or an index, not on an assert.  The implementer added an assert before each, and they then failed on it.
* 2026-11-01, implementer, step 2: the error of `cargo metadata` has more than one line.  So the first real run of `a_graph_that_fails_gives_a_warning_and_tier_push` failed: the `warning:` line broke in two.  `check` now joins the lines of an error in a warning with `; `.  The test checks that each line of the output starts with `check:` or `warning:`.
* 2026-11-01, implementer, step 2: `guide_covers` passed before the new Tools row, since `review check` already puts the word `check` in a code span of the guide.  The Tools row names `check --tier commit` and `check --tier push`.
* 2026-11-01, implementer, step 2: `check --tier commit --dry-run` on the change of step 2 runs fmt, `docs-check`, the workspace clippy and `cargo test -p xtask`.  So it adds no check to the list of "How the team works".
* 2026-11-01, implementer, step 3: the strict-rule path table sends 96 of PR #39's 194 commits to tier `push` (49.5%).  The measure ran `target/debug/xtask check --tier commit --dry-run` of `32f7a35` on each commit, in the scratch worktree `target/m4-pr39`.  Each commit read its own change (`HEAD^..HEAD`), and no run gave a `warning:` line.  The 98 other commits ran tier `commit`.
  * Commits with a hit of each rule: MEM-5 66, PROTO-1 51, MEM-2 32, PROTO-2 21, RT-1 11, MEM-1 9, GPU-2 9, GPU-3 9, DEPS-1 8, GPU-1 7, MEM-3 7, MEM-4 7.  No commit hit ASSET-1, and VERIFY-1 and GIT-1 have no paths.
  * Hits, as (rule, path) pairs: PROTO-1 97, MEM-5 90, MEM-2 51, PROTO-2 33, RT-1 24, DEPS-1 16, GPU-2 12, GPU-3 12, MEM-1 9, MEM-4 8, GPU-1 7, MEM-3 7.
  * The paths in most commits: `app/src/store.rs` 37, `xtask/src/e2e.rs` 30, `app/src/export.rs` 20, `xtask/src/e2e/save.rs` 15, `protocol/src/lib.rs` 12, `worker/src/web.rs` 11, `xtask/e2e/save_spy.js` 10, `app/src/separator.rs` 10.
  * 14 of the 96 commits hit only paths under `xtask/`, and 8 hit only `app/src/store.rs`.  25 hit only those, and 40 hit only those or `app/src/export.rs`.  The share is not more than half, so this plan proposes no narrower paths.
* 2026-11-01, implementer, step 3: the timed runs, at `32f7a35` with a warm `target/`, on this Mac:
  * `time cargo run -p xtask -- check --tier commit`: exit 0 in 67.7 s.  It read the last commit, and ran 4 checks in 66.4 s: fmt 0.4 s, `docs-check` 1.0 s, the workspace clippy 0.4 s, `cargo test -p xtask` 64.6 s.
  * `time cargo run -p xtask -- check --tier push`: exit 1 in 26.5 s.  fmt 0.4 s, `docs-check` 1.0 s, the workspace clippy 0.4 s, the `synth` clippy 0.3 s, `cargo clippy -p app` 16.6 s, `cargo clippy -p app --all-targets` 4.3 s.  Then the wasm32 clippy line failed in 2.4 s: "can't find crate for `core`".  This Mac has no `wasm32-unknown-unknown` target (`rustup target list --installed` gives only `aarch64-apple-darwin`), and no `trunk` on `PATH`.  The run named the 4 checks that did not run.  The GPU test line was skipped, since the branch changes no GPU code.
  * The implementer did not install the target or Trunk, since that changes the maintainer's toolchain.  `README.md` gives both installs.  It ran the 3 test lines after the wasm32 line by hand, in order: `cargo test --workspace --exclude roformer-gpu` 153 s, `cargo test -p roformer-gpu --lib` 17 s, `cargo test -p roformer-gpu --doc` 2 s.  Each passed.
  * So tier `push` with no wasm32 clippy and no Trunk build takes about 200 s here.  Tier `commit` on an `xtask` change takes 68 s, and on a docs change it runs fmt, `docs-check`, the workspace clippy and the `xtask` repo tests (about 8 s for those tests).
* 2026-11-01, implementer, step 4: the grep for "fmt, clippy, the tests" gives these places that state GIT-2 today:
  * `CLAUDE.md` (the GIT-2 line), `docs/invariants.md` (GIT-2), the recipe "Fix a bug" of `docs/working-with-agents.md`, and lines 22 and 302 of `.claude/skills/review-rounds/SKILL.md`.  The diff changes each of them, and also "Run the checks" after the last round of the skill, the "Review:" line of `CLAUDE.md` and "the checks CI gates on" in `README.md`.
  * `.claude/agents/baseline.md` (line 17) stays as it is.  `BASELINE_HASH` freezes it, and the parent's M5 keeps it.
  * Decision 0026 (its Context) and `plans/active/harness-engineering.md` (line 19, the CI of 2026-10-19) tell history, so they stay.  So do this plan, the plans in `docs/plans/completed/` and the logs in `docs/reviews/` (PLANS.md).
* 2026-11-01, implementer, step 4: the diff changes `CLAUDE.md`, the `review-rounds` skill, `docs/invariants.md`, `docs/working-with-agents.md`, `README.md` and `xtask/tests/rules.rs`.  The checks of the diff, at `4b7925c`, in the scratch worktree `target/m4-diff` with the target folder `target/m4-diff-target`:
  * `git apply --check` passes at the repo root.
  * With only the `xtask/tests/rules.rs` part, `cargo test -p xtask --test rules` fails 3 of 30 tests, each on its assert.  `git_2_names_the_tiers`: 4 problems, since the GIT-2 line of `CLAUDE.md` and the GIT-2 section of `docs/invariants.md` each lack `check --tier commit` and `--tier push`.  `the_push_tier_runs_the_lines_of_claude_md`: "CLAUDE.md's Commands has no line `cargo test -p roformer-gpu --doc`".  `the_review_skill_starts_with_xtask`: the 3 new skill lines of `REVIEW_SKILL_TEXT` are missing.
  * With the whole diff, `CLAUDE.md` has 91 lines.  `docs-check` passes, and `cargo test -p xtask --test rules` passes 30 of 30.
  * With the whole diff, `cargo run -p xtask -- check --tier push` gives exit 1 in 40.3 s, cold.  fmt, `docs-check` and the 4 clippy lines before wasm32 pass.  The wasm32 clippy line fails with "can't find crate for `core`", since this Mac has no `wasm32-unknown-unknown` target (step 3).  So the 3 test lines and the Trunk build do not run, and Trunk is not on `PATH` either.  The GPU test line is skipped, since the branch changes no GPU code.
  * The implementer ran the 3 test lines by hand in that worktree: `cargo test --workspace --exclude roformer-gpu` passes in 166 s, `cargo test -p roformer-gpu --lib` in 16 s and `cargo test -p roformer-gpu --doc` in 1 s.
  * The implementer removed the worktree and its target folder (5.9 GB).
* 2026-11-01, implementer, step 1: the `grep` of each strict row found the code in each file of the table.  `decoder/src/pass.rs` and `app/src/reset.rs` name OPFS, but `pass.rs` writes through a trait that `decoder/src/web.rs` gives, and `reset.rs` only removes files.  So they stay out of MEM-5.  `app/src/export.rs` posts with a transfer list, and MEM-5 already holds it.

## Decision log

* 2026-11-01, team leader: tier `commit` runs fmt, `docs-check`, the workspace clippy and the `xtask` repo tests on each change.  Reason: each takes seconds, and each reads files of the whole repo.  The parent runs `docs-check` only when a doc, `CLAUDE.md`, `.claude/` or `xtask/` changed.
* 2026-11-01, team leader: the wasm32 clippy runs when a changed member is in the closure of its five packages.  Reason: a change in `decoder`, `dsp`, `synth`, `roformer`, `roformer-gpu` or `test-support` can fail that line too.  The parent names `app`, `worker`, `protocol` or `engine`.
* 2026-11-01, team leader: the crate tests are one `cargo test` command (plan review round 1).  Reason: cargo turns on features only across the packages of one run.
* 2026-11-01, team leader: tier `push` runs every line, but the GPU test line on a branch with no GPU change.  Its paths give the report and that one choice.  Reason: decision 0026 and the maintainer ask for the full set before a push and a review round.
* 2026-11-01, team leader: tier `push` runs the GPU test line only when the paths change `roformer/`, `roformer-gpu/`, `test-support/` or a workspace file (plan review round 2).  Reason: the line's tests test only those folders.  The line needs a real GPU, and a release build takes minutes.  Decision 0026 names each test line, so the PR asks the maintainer to confirm.
* 2026-11-01, team leader: tier `commit` skips the GPU test line.  Reason: it needs a real GPU, and a commit that changes a kernel or a GPU buffer matches GPU-1 to GPU-3 or MEM-3, so it runs tier `push`.
* 2026-11-01, team leader: `CLAUDE.md` gets the line `cargo test -p roformer-gpu --doc`.  Reason: CI runs it, MEM-3 has doctests, and a test checks that `CLAUDE.md` names each line of tier `push`.
* 2026-11-01, team leader: `check` reads the graph with `rules::workspace_metadata` and `rules::workspace` (plan review rounds 1 and 2).  Reason: the repo then has one runner and one parser of `cargo metadata`.  `--locked` stays, so `check` never writes `Cargo.lock`.  A manifest edit matches DEPS-1 and runs tier `push`, which needs no graph.  If the graph fails, tier `commit` runs tier `push`.
* 2026-11-01, team leader: no table of cross reads (plan review round 1).  Reason: the one case today, `app/Cargo.toml`, matches DEPS-1, so a row for it could not fail a test.  The new TD records the gap.
* 2026-11-01, team leader: the GIT-2 text of `docs/invariants.md`, the recipe "Fix a bug" and `README.md` go in the orchestrator's diff with `CLAUDE.md`.  Reason: decision 0026 says that GIT-2 changes in the same commit everywhere.
* 2026-11-01, implementer: a plan lists each line of `lines()` as `run` or `skip`, then `serve`, `web-test`, `e2e` and `publish` as `skip`.  Tier `commit` also lists the `xtask` repo tests after the wasm32 clippy line, and the crate tests just before the workspace test line.  Reason: the output then names each line of `CLAUDE.md`'s Commands, with its reason.
* 2026-11-01, implementer: tier `push` with no path runs the GPU test line.  Reason: no path can mean that the merge base is not known, and then the plan cannot rule out a GPU change.  A branch with no change since the merge base also gets it, which costs one run.
* 2026-11-01, implementer: the end-to-end test of the binary runs only `check --dry-run`.  It does not run `check --tier commit` for real in the temporary workspace, as step 2 says.  Reason: the team leader's rule is that no test runs `xtask check` without `--dry-run`.  `run_command_gives_the_exit_code_the_signal_and_a_start_error` tests the real runner on `sh` in the temporary folder, and the `run_checks` tests use a fake runner.  No test runs the call from `main` to `run_checks`.
* 2026-11-01, implementer: when `graph` fails, `main` prints a `warning:` line and runs tier `push`, as step 2 says for `graph`.  It does not give exit 2, as the bullet of `main` could read for "a `cargo metadata` error".  Reason: the step gives the rule for this case.
* 2026-11-01, implementer: the output of `check` has these lines: `check: tier <tier>`, then `check: paths: <n>, from <where>`, then the strict hits, then `check: tier <tier> runs: <why>`, then the checks.  On a failure, the last line is `check: <line> failed, after <n> passed, in <s> s`.  `--dry-run` can come before or after `--tier <tier>`.  `repo_root` and `run_command` are `pub`, so the tests call them.
* 2026-11-01, implementer: the timed run of tier `push` in step 3 ran on this Mac as it is, with no `wasm32-unknown-unknown` target and no Trunk.  It stopped at the wasm32 clippy line, and the implementer timed the 3 test lines after it by hand.  Reason: an install changes the maintainer's toolchain, and nobody asked for it.  Tier `push` cannot pass on this Mac until a person runs the two installs of `README.md`.
* 2026-11-01, implementer: a commit that tier `commit` sends to tier `push` (a strict hit, or no graph) gets the GPU test line by the rule of tier `push`, from the commit's own paths.  Reason: D2 says that such a commit runs tier `push`.  So a change to `roformer-gpu/src/lib.rs` runs the GPU test line, and a change to `engine/src/slot.rs` does not.

## Outcomes and retrospective

To be written when the plan completes.
