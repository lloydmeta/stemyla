# Harness phase 3b, M1: cleanup (`xtask clean`)

## Purpose

This is the milestone plan for M1 of `plans/active/harness-phase-3b.md`.  It gives more detail than the parent plan's M1 section, which stays the criteria.  After M1:

* `xtask clean` lists what runs leave in the repo, with sizes, and changes nothing.  `xtask clean --yes` removes only the items that it can prove safe.
* `xtask review clean <round>` removes the worktrees that the agents of one review round made.
* `xtask serve` writes a pid file in `target/`, so `xtask clean` can find and stop a server that a run left.
* `docs/working-with-agents.md` names `clean` and `review clean`.

Readers: the implementer of M1, the review agents of M1's rounds, and the orchestrator.

## Context and orientation

The state at the start of M1, on 2026-10-31, at commit `13dc6e3` on branch `harness-phase-3b`:

* `xtask/src/main.rs` sends each command word to a module.  `docs::xtask_commands` reads its match arms, and `guide_covers` (`xtask/src/docs.rs`) fails `docs-check` when `docs/working-with-agents.md` has no code span with each command word.
* `xtask/src/serve.rs`: `run(args)` parses `[dir] [--port N] [--mount /p=dir] [--inject /url.js]`, binds `127.0.0.1:<port>`, prints `serving <dir> at http://<addr>/`, and serves forever.  It writes no pid file.  `xtask e2e` calls `serve_with` in its own process, not `run`.
* `xtask/src/review.rs`: `run_with` dispatches `start <base> <round>`, `check <round>` and `mutants <round> [--config <file>]`.  Each failure exits 3 (`REFUSED`).  `USAGE` names the three forms.  `run_folder` gives `target/review/run/<branch>/`, with `/` in the branch as `-`, or `detached-<short sha>`.  `take_state` leaves out each worktree under `target/review/`, so a reviewer's worktree does not fail `review check`.
* The agents in `.claude/agents/` make worktrees with `git worktree add --detach target/review/<round>-<name>-<n> HEAD`.  A validator uses `target/review/<round>-validator-<id>-<n>`, where `<id>` is a finding ID such as `1-3`, or `mutants`.  Each agent must remove its worktree, but an agent that stops early leaves it.
* `xtask/tests/rules.rs`: `REVIEW_SKILL_TEXT` pins lines of `.claude/skills/review-rounds/SKILL.md`, and `the_review_skill_starts_with_xtask` fails a missing one.  The guard `.claude/hooks/read-only-bash.sh` refuses `xtask publish` and `xtask review` to the review agents, but not `xtask clean`.
* Existing test helpers make temporary repos with `commit.gpgsign=false` (`xtask/src/review.rs` tests, near line 1415) or with `GIT_CONFIG_GLOBAL=/dev/null` (`xtask/tests/publish.rs`).
* What runs left, as the parent plan's retrospective found: replay clones in `/private/tmp/stemyla-harness`, reviewer worktrees and run folders in `target/review/`, squash-merged local branches, `worktree-agent-*` branches and a worktree in `.claude/worktrees/`, `xtask serve` on port 8080.  `git branch --merged` does not see a squash-merged branch.
* No new crate is needed (DEPS-1).  The code uses the standard library, `serde_json` (already a dependency) and the programs `git`, `gh`, `ps`, `kill` and `lsof`.

Terms:

* Item: one thing that `xtask clean` names: a folder, a worktree, a branch, a pid file or a scratch root.
* Removable: an item that `--yes` removes, with the proof that makes it safe.
* Kept: an item that `xtask clean` names but never removes, with the reason.
* Facts: what the code reads from outside the repo's files: the PRs from `gh`, the command line of a pid from `ps`, the working folders of processes from `lsof`.  A test gives fake facts, so no test needs the network.

## Plan of work

### How the team works

* The team leader writes this plan and starts one implementer (`general-purpose`) with its path.  The implementer does steps 1 to 4 and commits each step signed, after the checks of GIT-2.
* The implementer does not edit `.claude/**`, `CLAUDE.md` or `docs/plans/active/harness-phase-3b.md`.  Step 5 gives those changes to the orchestrator as text.
* Tests of `xtask clean` run on temporary repos only, never on this repo (VERIFY-1).  Each test that plants a leftover fails before the change.

### Step 1: the module and its pure plan

Add `xtask/src/clean.rs`, `pub mod clean` in `xtask/src/lib.rs`, and the arm `"clean" => std::process::exit(clean::main(&args[1..]))` in `xtask/src/main.rs`.  Add `clean` to the usage line of `main.rs`.

Command line: `xtask clean [--yes] [--also <path>]...`.  Another word prints the usage and exits 2.

Split the code in two parts, so the tests can check the rules without the network:

* `gather(root, facts) -> Survey`: reads the repo at `root` with `git` and the file system, and takes the facts from its argument.
* `plan(&Survey) -> Vec<Item>`: a pure function.  Each `Item` has a kind, a path or name, a size in bytes (for a folder), and `Removable(proof)` or `Kept(reason)`.
* `Facts` holds `open_prs: Option<Vec<String>>` (head branch names), `merged_prs: Option<Vec<(String, String)>>` (head branch name, `headRefOid`), `command_of: fn(pid) -> Option<String>` or a map, and `cwds: Option<Vec<PathBuf>>`.  `None` means that the fact is unknown, and an item that needs it is kept with the reason "cannot read <fact>".
* `main` gathers the real facts: `gh pr list --state open --json headRefName --limit 500` and `gh pr list --state merged --json headRefName,headRefOid --limit 1000`, both run in `root`; `ps -p <pid> -o command=`; `lsof -a -d cwd -F n` (each line that starts with `n`).  A failed program gives `None`.

The root is the main worktree: the parent of `git rev-parse --path-format=absolute --git-common-dir`.  So a run from a linked worktree cleans the main repo.  Refuse a bare repo.

The rules for each kind (parent plan, M1):

1. Run folders: each folder `target/review/run/<name>/`.
   * Removable when all of these hold: `<name>` is not the current branch of any worktree (with `/` as `-`); no open PR has a head branch whose name, with `/` as `-`, is `<name>`; and `<name>/log.md` exists and its blob (`git hash-object`) equals the blob of a file under `docs/reviews/` at `HEAD` or at `origin/main` (`git ls-tree -r <rev> docs/reviews/`).  The copy can have another name, such as `harness-phase-3b-m1.md`.
   * Kept, with the reason: "a branch with an open PR", "the current branch", "no log.md", "log.md has no copy in docs/reviews/", or "cannot read open PRs".
2. Worktrees: each worktree in `git worktree list --porcelain` whose path is under `<root>/target/review/` or `<root>/.claude/worktrees/`.
   * Removable when all of these hold: `git -C <path> status --porcelain --untracked-files=normal` is empty; it is not locked; no working folder in `cwds` is the path or under it; and its `HEAD` is reachable from a ref outside the worktree (`git for-each-ref --contains <sha>` gives a ref that is not this worktree's own branch, or the worktree's branch exists and stays).  Removing a worktree keeps its branch, so a worktree on a branch passes the last test.  A detached `HEAD` that no ref contains is kept, because its commits would be lost.
   * Kept, with the reason: "has changes", "locked", "a process runs in it", "cannot read processes", or "its HEAD <sha> is on no ref".
   * A worktree that `git worktree list` marks `prunable` (its folder is gone) is removable as "prunable".
3. Build output: `target/e2e/` and `app/dist/`, when they exist.  Always removable, with their size.
4. Local branches: each `refs/heads/<b>`.
   * Kept: `main`, `gh-pages`, the current branch of each worktree that stays.  A branch checked out only in a worktree that this plan removes is not kept for that reason, so a second run finds no new item.
   * Removable when one proof holds, and the item names the proof:
     * its tip is an ancestor of `origin/main` (`git merge-base --is-ancestor`);
     * its tip's tree equals the tree of a commit on `origin/main` (`git rev-list --format=%T origin/main`, or `git log --format=%T origin/main`): a squash merge;
     * a merged PR has head branch `<b>` and `headRefOid` equal to the tip.
   * Else kept: "no proof that it is merged".  A branch whose name matches a merged PR but whose tip differs is kept, because it has commits after the merge.
   * With no `origin/main`, the first two proofs are unknown; the third still holds.
5. Pid files: each file `target/xtask-serve/<port>.pid` (step 2 writes them).
   * If the pid's command line (`ps`) runs `xtask serve` (a word whose last path part is `xtask`, then the word `serve`), the item is "a running xtask serve on port <port>".  `--yes` sends `kill <pid>` (SIGTERM), then removes the file.
   * If the pid runs another program, or no program, the file is stale.  `--yes` removes the file only, and never signals the pid.
   * A file that does not hold a pid is stale too.
6. Scratch roots: each `--also <path>`.
   * The path must be absolute and must exist as a folder.  Its canonical form must be under the canonical temp folder (`std::env::temp_dir()`, `/tmp` or `/private/tmp`), and must not be one of those folders.  It must not be the repo root or a folder that holds it.  Else the command refuses before it lists anything, and exits 2.
   * A valid scratch root is removable as a whole, with its size.

Never removed, in any case: a worktree with changes, a branch that has no proof, a current branch, a file or folder outside the repo but a valid `--also` root, and `target/` as a whole.

### Step 2: the output, `--yes` and the pid file

Output of the list mode, in this order:

```text
can remove (<n> items, <size>):
  <kind> <path or name> (<size>): <proof>
keeps (<m> items):
  <kind> <path or name>: <reason>
target/ holds <size>.  `cargo clean` removes it: run it by hand.
run `cargo run -p xtask -- clean --yes` to remove the items above.
```

* With no removable item, the first line is `nothing to remove`, and the last line is left out.  A run that changes nothing prints the same.
* Sizes are in binary units (`KiB`, `MiB`, `GiB`), from a walk of the folder that does not follow symbolic links.

`--yes` removes the removable items in this order: stop each running `xtask serve` and remove each pid file; remove each worktree with `git worktree remove <path>` (no `--force`, so git refuses one that changed since the survey); run `git worktree prune`; delete each branch with `git branch -D <b>`; remove each run folder, build folder and scratch root with `std::fs::remove_dir_all`.  It prints `removed <kind> <path or name>` for each one.  A failure prints `failed <kind> <path or name>: <error>`, goes on, and the command exits 1 at the end.  Else it exits 0.

`--yes` gathers the survey again before it removes, and removes only the items that are removable in that survey.  It prints the list of the survey first, so the person sees what goes.

The pid file of `xtask serve`:

* `serve::run` writes `<workspace root>/target/xtask-serve/<port>.pid` after the bind, with the real port (also for `--port 0`), before the `serving` line.  The file holds `std::process::id()` and a newline.
* Put the path in one function, `serve::pid_file(root, port)`, that `clean.rs` also uses.
* `serve::run` cannot remove the file when a signal stops it, since it uses the standard library only.  `xtask clean` finds such a stale file.

### Step 3: `xtask review clean <round>`

Add the arm `["clean", round]` to `review::run_with`, and add `clean <round>` to `USAGE`.

* It lists the worktrees of the repo of the current folder (`toplevel`), as `take_state` does.
* It removes each worktree whose folder name, directly under `<toplevel>/target/review/`, starts with `<round>-<agent>-`, where `<agent>` is a name in `REVIEWERS`, `validator` or `baseline`.  It uses `git worktree remove --force`, because a reviewer's worktree holds its trial changes by design.  The name test is exact: round `1` does not match `10-reviewer-1`, and `reviewer` does not match `1-docs-reviewer-1`.
* It also removes a folder with such a name that is not a registered worktree (`remove_dir_all`), since only build output can be in it.
* Then it runs `git worktree prune`.
* It prints `removed <path>` for each one, then `pruned`.  With no match, it prints `no worktrees of round <round>`.  It exits 0, or 3 on a failure.
* It never removes `target/review/target/`, `target/review/run/`, or a worktree of another round.
* The guard already refuses `xtask review` to the review agents, so it refuses `review clean` too.

### Step 4: the tests (VERIFY-1)

Write each test first, and see it fail before the code.  Put the pure tests in `clean.rs` and the end-to-end tests in `xtask/tests/clean.rs`, which runs `env!("CARGO_BIN_EXE_xtask")` in a temporary repo.  Each temporary repo has `GIT_CONFIG_GLOBAL=/dev/null`, `GIT_CONFIG_NOSYSTEM=1` and `commit.gpgsign=false`, and an `origin/main` ref made with `git update-ref`.  Tests that run the binary must not need `gh`: put an empty folder first on `PATH` with a fake `gh` script that prints a fixed JSON, or let `main` read the facts through a seam that the tests call.  The implementer chooses one and records it in the Decision log.

Fixtures, planted in one temporary repo (parent plan, M1 acceptance):

* Branches: `squashed` (two commits; `origin/main` gets a commit with the tree of its tip, made with `git commit-tree`); `unmerged` (a commit not on `origin/main`); `moved-on` (the fake merged PR names it with an older `headRefOid`, and it has one more commit).  Also `merged-pr` (a merged PR with the same tip), and `ancestor` (an ancestor of `origin/main`).
* Worktrees: `target/review/1-reviewer-1` with a changed file (dirty); `target/review/1-reviewer-2` clean; `.claude/worktrees/agent-x` clean on a branch; one clean worktree with a `sleep` child process in it (kept: "a process runs in it"); one detached worktree on a commit that no ref holds (kept).
* Pid files: one whose pid has ended (stale); one whose pid runs `sleep` (another program; the test checks that `sleep` still runs after `--yes`, then stops it); one that runs `xtask serve` (the test starts the binary with `serve <folder> --port 0`, copies the pid file that it writes into the temporary repo, and checks that `--yes` stops it).
* Run folders: `target/review/run/old` whose `log.md` equals a committed `docs/reviews/old-m1.md` (removable); `target/review/run/nolog-copy` whose `log.md` has no copy (kept); `target/review/run/open-pr` with a copy but an open PR (kept).
* Build output: `target/e2e/x` and `app/dist/index.html`.
* Scratch root: a folder under the temp folder given with `--also` (removable).  Separate tests refuse `--also /`, `--also` of the home folder, of the repo root, a relative path and a missing path, each with exit 2 and nothing removed.

Tests:

* The list mode names each item above, with its kind and its proof or reason, and changes nothing: the branches, worktrees, folders, pid files and processes are the same after the run.
* `--yes` removes exactly the removable items, and keeps each kept item.  The test checks the files, `git branch`, `git worktree list` and the processes.
* A second run after `--yes` prints `nothing to remove`.
* With `gh` failing (no facts), the run folders that need the open PRs are kept with "cannot read open PRs", and the `merged-pr` branch is kept; the tree proof still removes `squashed`.
* The rule tests of `plan` are pure, one for each proof and each reason.
* `serve` writes its pid file: start the binary with `--port 0`, read the port from the `serving` line, check that `target/xtask-serve/<port>.pid` under the workspace root holds the child's pid, stop the child, and remove the file.
* `review clean <round>`: in a temporary repo, worktrees `1-reviewer-1` (dirty), `1-docs-reviewer-1`, `1-validator-1-3-1`, `10-reviewer-1`, `2-reviewer-1` and `1-stranger-1`, and an unregistered folder `1-baseline-1`.  `review clean 1` removes the first three and the folder, and keeps the other three.  `git worktree list` no longer shows the removed ones.  A bad round (`0`, `x`) exits 3 with the usage.

### Step 5: the docs, and the text for the orchestrator (DOC-1)

* `docs/working-with-agents.md`: add `clean` and `review clean` to the Tools table, a line in "Review a change" (the skill removes a round's worktrees), and a changelog line for M1.  Say that `cargo clean` stays a manual step.
* `README.md`: add a short line for `xtask clean` beside the other xtask commands, if its section lists them.
* `docs/tech-debt.md`: an entry for each gap that M1 leaves.  Candidates: the pid file stays after a signal; `lsof` or `gh` missing keeps items; the guard does not refuse `xtask clean --yes` until the orchestrator changes it.
* The implementer gives the team leader, as text, not as a commit:
  * the new step in `.claude/skills/review-rounds/SKILL.md`: at the end of step 11, after the rounds table, run `target/debug/xtask review clean $1`; if auto mode or a person refuses it, give the command to the person and go on;
  * the line to add to `REVIEW_SKILL_TEXT` in `xtask/tests/rules.rs` (`target/debug/xtask review clean $1`), with the comment change;
  * the guard change in `.claude/hooks/read-only-bash.sh`, so `check_xtask_words` refuses `clean` too, and its cases in `xtask/tests/rules.rs` (`target/debug/xtask clean --yes` and `cargo run -p xtask -- clean` refused).
* The team leader gives that text to the orchestrator at the checkpoint.

### Step 6: review rounds

`cargo build -p xtask`, then `/review-rounds 13dc6e3 <round> "<criteria>"` for rounds 1, 2 and 3, and more until a round has no blocker and no significant should-fix.  The criteria are the parent plan's M1 section and this plan.  Round 1 passes `--baseline`.  Each confirmed finding gets a fix with a test; a nit gets a fix only if the team leader agrees.  After the last round, the team leader copies `target/review/run/harness-phase-3b/log.md` to `docs/reviews/harness-phase-3b-m1.md` and removes the run folder (TD-37).

## Validation and acceptance

* `cargo test -p xtask` passes, with the tests of step 4.  Each fixture test fails on `13dc6e3`, where `clean` and `review clean` do not exist.
* `cargo run -p xtask -- clean` in this repo lists and changes nothing; the team leader runs it once and records the output in Surprises.
* `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo run -p xtask -- docs-check` pass.
* The review rounds end by the stop rule, and the log is in `docs/reviews/harness-phase-3b-m1.md`.

## Idempotence and recovery

* The list mode changes nothing, so it is safe to run at any time.  `--yes` gathers again before it removes, and a second run removes nothing more.
* A removed branch has its tip in the output (`removed branch <b> (<sha>)`), so `git branch <b> <sha>` brings it back while the object exists.
* Each step is its own commit, which `git revert` undoes.  A new team leader takes over from Progress.

## Progress

* [x] 2026-10-31: plan written.

## Surprises and discoveries

None yet.

## Decision log

* 2026-10-31, team leader: `xtask clean` also removes a branch whose tip is an ancestor of `origin/main`.  It is the plain merge, and it loses no commit.  The parent plan names only the squash and PR proofs.
* 2026-10-31, team leader: a run folder's log may match any file in `docs/reviews/` at `HEAD` or `origin/main`, by blob.  The copy of a milestone run is named `<branch>-m<n>.md`, not `<branch>.md`.
* 2026-10-31, team leader: `--also` accepts only folders under the temp folder.  A wrong path such as the home folder could otherwise lose a person's files.
* 2026-10-31, team leader: `review clean` removes a round's worktrees with `--force`, because a reviewer's trial changes are scratch by design.  `xtask clean` never removes a worktree with changes.
* 2026-10-31, team leader: the guard change for `xtask clean` is the orchestrator's, since it is under `.claude/`.

## Outcomes and retrospective

To be written when the milestone completes.
