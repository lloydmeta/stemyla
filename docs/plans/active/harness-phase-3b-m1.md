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

* `xtask/src/main.rs` sends each command word to a module.  `docs::xtask_commands` reads its match arms, and `guide_covers` (`xtask/src/docs.rs`) fails `docs-check` when `docs/working-with-agents.md` has no code span with each command word.  So the commit that adds the `clean` arm must also add `clean` to the guide.
* `xtask/src/serve.rs`: `run(args)` parses `[dir] [--port N] [--mount /p=dir] [--inject /url.js]`, binds `127.0.0.1:<port>`, prints `serving <dir> at http://<addr>/`, and serves forever.  It writes no pid file.  `xtask e2e` calls `serve_with` in its own process, not `run`.
* `xtask/src/review.rs`: `run_with` dispatches `start <base> <round>`, `check <round>` and `mutants <round> [--config <file>]`.  Each failure exits 3 (`REFUSED`).  `USAGE` names the three forms.  `run_folder` gives `target/review/run/<branch>/`, with `/` in the branch as `-`, or `detached-<short sha>`.  `take_state` records the local branches, and the worktrees that are not under `target/review/`.  So `review check` fails a round if a branch goes, but not if a reviewer's worktree goes.  `review::worktrees` canonicalises paths, because temporary folders on macOS have two names (`/var/folders` and `/private/var/folders`).
* The agents in `.claude/agents/` make worktrees with `git worktree add --detach target/review/<round>-<name>-<n> HEAD`.  A validator uses `target/review/<round>-validator-<id>-<n>`, where `<id>` is a finding ID such as `1-3`, or `mutants`.  An agent builds a worktree with `--manifest-path` and `--target-dir <worktree>/target`, from the main checkout, so no process need have its working folder in the worktree.  Each agent must remove its worktree, but an agent that stops early leaves it.
* `.gitignore` ignores `target/`, `/app/dist/`, and also audio, stems, weights and `third_party/` (ASSET-1).  A worktree can hold such ignored files, which `git status --porcelain` does not show and which `git worktree remove` deletes without `--force`.
* `xtask/tests/rules.rs`: `REVIEW_SKILL_TEXT` pins lines of `.claude/skills/review-rounds/SKILL.md`, and `the_review_skill_starts_with_xtask` fails a missing one.  The guard `.claude/hooks/read-only-bash.sh` refuses `xtask publish` and `xtask review` to the review agents (`check_xtask_words`), but not `xtask clean`.  Its cases are in `xtask/tests/rules.rs`, near line 560.
* Existing test helpers make temporary repos with `commit.gpgsign=false` (`xtask/src/review.rs` tests, near line 1415) or with `GIT_CONFIG_GLOBAL=/dev/null` (`xtask/tests/publish.rs`).
* What runs left, as the parent plan's retrospective found: replay clones in `/private/tmp/stemyla-harness`, reviewer worktrees and run folders in `target/review/`, squash-merged local branches, `worktree-agent-*` branches and a worktree in `.claude/worktrees/`, `xtask serve` on port 8080.  `git branch --merged` does not see a squash-merged branch.
* No new crate is needed (DEPS-1).  The code uses the standard library, `serde_json` (already a dependency) and the programs `git`, `gh`, `ps`, `kill` and `lsof`.

Terms:

* Item: one thing that `xtask clean` names: a folder, a worktree, a branch, a pid file or a scratch root.
* Removable: an item that `--yes` removes, with the proof that makes it safe.
* Kept: an item that `xtask clean` names but never removes, with the reason.
* Facts: what the code reads from outside the repo's files: the PRs from `gh`, the command line of a pid from `ps`, the open files of processes from `lsof`.  A test gives fake facts where it needs them, so no test needs the network.
* Repo root: the path of the first record of `git worktree list --porcelain`, run in the current folder, canonicalised.  It is the main worktree, also from a linked worktree, and also with `--separate-git-dir`.  `clean.rs` has one function for it, `repo_root(dir)`, which `serve.rs` and the tests use too.

## Plan of work

### How the team works

* The team leader writes this plan and starts one implementer (`general-purpose`) with its path.  The implementer does steps 1 to 5.  Each step brings its own tests, written first and seen to fail.  Each step is one or more signed commits, after the checks of GIT-2, so each commit passes `docs-check`.
* The implementer does not edit `.claude/**`, `CLAUDE.md` or `docs/plans/active/harness-phase-3b.md`.  Step 5 gives those changes to the team leader as text, and the team leader gives them to the orchestrator at the checkpoint.
* Tests of `xtask clean` run on temporary repos only, never on this repo (VERIFY-1).  No test writes into this repo's `target/` other than through cargo.

### Step 1: `xtask clean`, list mode

Add `xtask/src/clean.rs`, `pub mod clean` in `xtask/src/lib.rs`, and the arm `"clean" => std::process::exit(clean::main(&args[1..]))` in `xtask/src/main.rs`.  Add `clean` to the usage line of `main.rs`.  In the same commit, add a `clean` row to the Tools table of `docs/working-with-agents.md`, so `docs-check` passes (DOC-1).

Command line: `xtask clean [--yes] [--also <path>]...`.  Another word prints the usage and exits 2.

Split the code in two parts, so the tests can check the rules without the network:

* `gather(root, facts) -> Survey`: reads the repo at `root` with `git` and the file system, and takes the facts from its argument.
* `plan(&Survey) -> Vec<Item>`: a pure function.  Each `Item` has a kind, a path or name, a size in bytes (for a folder), and `Removable(proof)` or `Kept(reason)`.
* `Facts` holds `open_prs: Option<Vec<String>>` (head branch names), `merged_prs: Option<Vec<(String, String)>>` (head branch name, `headRefOid`), the command line of a pid, the pid that listens on a TCP port, and `open_paths: Option<Vec<PathBuf>>`: each open file and working folder of each process.  `None` means that the fact is unknown, and an item that needs it is kept with the reason "cannot read <fact>".
* `main` gathers the real facts: `gh pr list --state open --json headRefName --limit 500` and `gh pr list --state merged --json headRefName,headRefOid --limit 1000`, both run in the repo root; `ps -p <pid> -o command=`; `ps -p <pid> -o stat=`; `lsof -w -n -P -a -p <pid> -iTCP:<port> -sTCP:LISTEN -t`; `lsof -w -n -P -F n` (each line that starts with `n` and then `/`, which holds the `cwd`, `txt` and open file entries).  Canonicalise each path from `lsof` and each path that it is compared with.
* `ps` and `lsof` exit 1 also when nothing matches: a pid that has ended, or a pid that does not listen.  So exit 1 with empty stdout and empty stderr is the answer "no", not an unknown fact.  A fact is `None` only when the program cannot start, or exits with text on stderr.  `lsof -w -n -P -F n` counts as read when its output has `p` lines.  CI's `test` job runs on Ubuntu, so the tests must pass with the `ps` and `lsof` of both systems.
* A pid whose `stat` starts with `Z` (a zombie) has ended.
* Step 1 also adds `serve::pid_file(root, port)`, the path `<root>/target/xtask-serve/<port>.pid`, which step 2 writes and `clean.rs` reads.

The rules for each kind (parent plan, M1):

1. Run folders: each folder `target/review/run/<name>/`.
   * Removable when all of these hold: `<name>` is not the current branch, with `/` as `-`, of any worktree that stays (see "The second run" below); no open PR has a head branch whose name, with `/` as `-`, is `<name>`; and `<name>/log.md` exists and its blob (`git hash-object`) equals the blob of a file under `docs/reviews/` at `HEAD` or at `origin/main` (`git ls-tree -r <rev> docs/reviews/`).  The copy can have another name, such as `harness-phase-3b-m1.md`.
   * Kept also while it has an open round: a round folder with `state.json` and no `findings.md`.
   * Kept, with the reason: "a branch with an open PR", "the current branch of a worktree", "round <r> is open", "no log.md", "log.md has no copy in docs/reviews/", or "cannot read open PRs".
2. Worktrees: each worktree in `git worktree list --porcelain` whose path is under `<root>/target/review/` or `<root>/.claude/worktrees/`.
   * Removable when all of these hold:
     * `git -C <path> status --porcelain --untracked-files=normal` is empty;
     * `git -C <path> status --porcelain --ignored --untracked-files=normal` shows no ignored path outside a `target/` folder at the top of the worktree, so no ignored song, stem or weight file is lost (ASSET-1);
     * it is not locked;
     * no path in `open_paths` is the worktree or under it;
     * no review round is open for it: a worktree `target/review/<r>-...` is kept while the run folder of a worktree that stays has a round `<r>` with `state.json` and no `findings.md`.  The run folder of a worktree is its branch with `/` as `-`, or `detached-<short sha>` for a detached one.  Only the worktrees outside the two roots count here, since the reviewers' worktrees are detached scratch copies.  So an old run folder from another branch does not keep the worktrees of its round number forever;
     * its `HEAD` is reachable from a ref that is not a branch that this run deletes: `git for-each-ref --contains <sha>` gives such a ref.  Removing a worktree keeps its branch, so a worktree on a branch passes.  A detached `HEAD` that no ref contains is kept, because its commits would be lost;
     * for a worktree under `.claude/worktrees/` only: its `HEAD` is an ancestor of `origin/main`, or its branch is removable by a proof of rule 4.  A Claude Code agent with `isolation: worktree` keeps no file open between its tool calls, so the process test cannot see it.  Its work is safe to lose only when it is merged.
     * no worktree that stays is under its path, since `git worktree remove` deletes it.  A worktree that stays is one outside the two roots, or one that this rule keeps;
   * Kept, with the reason: "has changes", "holds ignored files", "locked", "a process uses it", "cannot read processes", "round <r> of <run> has no findings.md", "its HEAD <sha> is on no ref", "not merged" (`.claude/worktrees/` only), or "holds a worktree that stays".
   * A worktree that `git worktree list` marks `prunable` and whose folder is gone: removable as "prunable" only if it is under one of the two roots and its `HEAD` passes the ref test.  Else kept, with the advice to run `git worktree repair` or prune by hand.
   * git also marks a worktree `prunable` when only its `.git` file is gone.  Its folder can hold changes, so it is kept: "its .git file is gone: run `git worktree repair`".
3. Build output: `target/e2e/` and `app/dist/`, when they exist.  Always removable, with their size.
4. Local branches: each `refs/heads/<b>`.
   * Kept: `main`, `gh-pages`, and the current branch of each worktree that stays.
   * Removable when one proof holds, and the item names the proof:
     * its tip is an ancestor of `origin/main` (`git merge-base --is-ancestor`);
     * its tip's tree equals the tree of a commit on `origin/main` that the tip does not hold (`git log --format=%T <tip>..origin/main`): a squash merge;
     * a merged PR has head branch `<b>` and `headRefOid` equal to the tip.
   * Else kept: "no proof that it is merged".  A branch whose name matches a merged PR but whose tip differs is kept, because it has commits after the merge.
   * With no `origin/main`, the first two proofs are unknown; the third still holds.
5. Pid files: each file `target/xtask-serve/<port>.pid` (step 2 writes them).
   * The pid runs `xtask serve` if its command line (`ps`) has a word whose last path part is `xtask`, then the word `serve`, and the pid listens on `<port>` (`lsof`).  Then the item is "a running xtask serve on port <port>".  `--yes` sends `kill <pid>` (SIGTERM), and waits up to 5 s for the pid to end (no pid, or a zombie).  Then it removes the file.  If the pid still runs after 5 s, it prints `failed`, keeps the file, and the command exits 1.
   * If the pid runs another program, or no program, or does not listen on the port, the file is stale.  `--yes` removes the file only, and never signals the pid.  A file that does not hold a pid is stale too.
6. Scratch roots: each `--also <path>`.
   * The path must be absolute and must exist as a folder.  Its canonical form must be under the canonical temp folder (`std::env::temp_dir()`, `/tmp` or `/private/tmp`), and must not be one of those folders.  It must not be the repo root or a folder that holds it.  It must not be in a worktree of this repo (`git worktree list`).  The same folder twice is one scratch root.  A folder in another `--also` folder is refused.  Else the command refuses before it lists anything, and exits 2.
   * A valid scratch root is removable as a whole, with its size, if no path in `open_paths` is in it and it holds no worktree of this repo (`git worktree list`).  Else it is kept with the reason.  Its contents are scratch, since a plan names it, but a live session may still use it.

Never removed, in any case: a worktree with changes or ignored files, a branch that has no proof, a current branch of a worktree that stays, a file or folder outside the repo but a valid `--also` root, and `target/` as a whole.

The second run: `plan` decides the worktrees first.  The branches and run folders then use the set of worktrees that stay.  So a branch or a run folder that only a removed worktree held is decided in the same run.  The aim: a second run after `--yes` finds nothing new to remove.

Output, in this order:

```text
can remove (<n> items, <size>):
  <kind> <path or name> (<size>): <proof>
keeps (<m> items):
  <kind> <path or name>: <reason>
target/ holds <size>.  `cargo clean` removes it: run it by hand.
run `cargo run -p xtask -- clean --yes` to remove the items above.
```

* With no removable item, the first line is `nothing to remove`, and the last line is left out.
* With `--also`, the last line also gives each `--also <path>`.
* A branch item gives its tip (`branch <b> (<sha>)`), so `git branch <b> <sha>` brings it back.
* Sizes are in binary units (`KiB`, `MiB`, `GiB`), from a walk of the folder that does not follow symbolic links.

Tests of step 1, in `clean.rs` (pure) and `xtask/tests/clean.rs` (the binary, `env!("CARGO_BIN_EXE_xtask")`, in a temporary repo):

* Each temporary repo has `GIT_CONFIG_GLOBAL=/dev/null`, `GIT_CONFIG_NOSYSTEM=1` and `commit.gpgsign=false`, and an `origin/main` ref made with `git update-ref`.  Tests that run the binary give it a fake `gh` first on `PATH` (a script that prints fixed JSON, or exits 1), or call a seam with the facts.  The implementer chooses and records it in the Decision log.
* One rule test of `plan` for each proof and each reason above.
* The list mode, on a temporary repo with the fixtures of step 3 but without the real `xtask serve`, names each item with its kind and its proof or reason.  It changes nothing: the branches, worktrees, folders, pid files and processes are the same after the run.
* `--also` refuses `/`, the home folder, the repo root, a relative path and a missing path, each with exit 2 and nothing listed.

### Step 2: the pid file of `xtask serve`

* `serve::run` writes `<repo root>/target/xtask-serve/<port>.pid` after the bind, with the real port (also for `--port 0`), before the `serving` line.  The repo root is `clean::repo_root` of the current folder.  The file holds `std::process::id()` and a newline.
* Outside a git repo, `serve` writes no pid file, and says so on stderr.  The `serving` line on stdout stays as it is, since scripts read it.
* The path comes from `serve::pid_file` (step 1).
* `serve::run` cannot remove the file when a signal stops it, since it uses the standard library only.  `xtask clean` finds such a stale file.  Add the tech-debt entry for this gap in the same commit (CLAUDE.md, "Debt").
* Test: start the binary with `serve <folder> --port 0`, with its working folder in a temporary repo.  Read the port from the `serving` line.  Check that `target/xtask-serve/<port>.pid` in that repo holds the child's pid, then stop the child.  The test fails before the change.

### Step 3: `--yes`

`--yes` first prints the list.  Then it removes the removable items in this order:

1. Stop each running `xtask serve`, and remove each pid file.
2. Gather the survey again, with new facts, and plan again.  A worktree that a stopped server used is then free.  Only items that are removable in this survey go.
3. Remove each worktree with `git worktree remove <path>` (no `--force`, so git refuses one that changed since the survey).  A prunable worktree goes the same way; `--yes` never runs `git worktree prune`.  If a removal fails, the worktree's branch and run folder stay.  Read the list with `git worktree list --porcelain -z`.
4. Delete each branch with `git branch -D <b>`.
5. Remove each run folder, build folder and scratch root with `std::fs::remove_dir_all`.

It prints `removed <item>` for each one, in the form of the list.  A failure prints `failed <item>: <error>`, goes on, and the command exits 1 at the end.  Else it exits 0.

Tests of step 3, in `xtask/tests/clean.rs`.  Fixtures, planted in one temporary repo (parent plan, M1 acceptance):

* Branches: `squashed` (two commits; `origin/main` gets a commit with the tree of its tip, made with `git commit-tree`); `unmerged` (a commit not on `origin/main`); `moved-on` (the fake merged PR names it with an older `headRefOid`, and it has one more commit); `merged-pr` (a merged PR with the same tip); `ancestor` (an ancestor of `origin/main`).
* Worktrees:
  * `target/review/1-reviewer-1` with a changed file (dirty, kept), and `target/review/1-reviewer-2` clean (removable);
  * `target/review/1-reviewer-3`, clean but with an ignored `x.wav` (kept), and `target/review/1-reviewer-4` with an ignored `target/` folder only (removable);
  * `.claude/worktrees/agent-x` clean on branch `agent-x`, an ancestor of `origin/main`, and `target/review/run/agent-x/` with a copied log: the worktree, the branch and the run folder all go in one run;
  * `.claude/worktrees/agent-y` clean on branch `agent-y` with a commit not on `origin/main` (kept: "not merged");
  * a clean worktree with a `sleep` child that holds a file in it open (kept: "a process uses it");
  * a detached worktree on a commit that no ref holds (kept);
  * a linked worktree outside the two roots, on branch `ancestor-wt`, an ancestor of `origin/main` (the branch is kept: a current branch);
  * a worktree whose folder the test deletes, so it is prunable (removable).
* Pid files: one whose pid has ended (stale); one whose pid is a zombie (stale); one whose pid runs `sleep` (another program; the test checks that `sleep` still runs after `--yes`, then stops it); one of a real `xtask serve` that step 2 writes, started with its working folder in a clean worktree under `target/review/` (`--yes` stops it, and the worktree goes in the same run).  The test reaps that child in a thread, so it does not stay a zombie.
* Run folders: `target/review/run/old` whose `log.md` equals a committed `docs/reviews/old-m1.md` (removable); `target/review/run/nolog-copy` whose `log.md` has no copy (kept); `target/review/run/open-pr` with a copy but an open PR (kept); the run folder of the main checkout's branch with round `2` that has `state.json` and no `findings.md`, and a clean worktree `target/review/2-reviewer-1` (both kept); a run folder `other` of no worktree with an open round `3`, and a clean worktree `target/review/3-reviewer-1` (the worktree is removable).
* Build output: `target/e2e/x` and `app/dist/index.html`.
* Scratch roots: a folder under the temp folder given with `--also` (removable); a second one in which a `sleep` child holds a file open (kept).

Tests:

* `--yes` removes exactly the removable items, and keeps each kept item.  The test checks the files, `git branch`, `git worktree list` and the processes.
* A second run after `--yes` prints `nothing to remove`.
* With `gh` failing, the run folders that need the open PRs are kept with "cannot read open PRs", and `merged-pr` is kept; the tree proof still removes `squashed`.

### Step 4: `xtask review clean <round>`

Add the arm `["clean", round]` to `review::run_with`, and add `clean <round>` to `USAGE`.  In the same commit, add `review clean` to the Tools table of `docs/working-with-agents.md`.

* It lists the worktrees of the repo of the current folder (`toplevel`), as `take_state` does.
* It removes each worktree whose folder name, directly under `<toplevel>/target/review/`, starts with `<round>-<agent>-`, where `<agent>` is a name in `REVIEWERS`, `validator` or `baseline`.  It uses `git worktree remove --force`, because a reviewer's worktree holds its trial changes, and these are scratch by design.  The name test is exact: round `1` does not match `10-reviewer-1`, and `reviewer` does not match `1-docs-reviewer-1`.
* It also removes a folder with such a name that is not a registered worktree (`remove_dir_all`), since it is scratch by design too.
* It runs no `git worktree prune`: `git worktree remove --force` also removes a worktree whose folder is gone.
* It prints `removed <path>` for each one, or `failed <path>: <error>`.  With no match, it prints `no worktrees of round <round>`.  It exits 0, or 3 on a failure.
* It never removes `target/review/target/`, `target/review/run/`, or a worktree of another round.
* The guard already refuses `xtask review` to the review agents, so it refuses `review clean` too.
* Test: in a temporary repo, worktrees `1-reviewer-1` (dirty), `1-docs-reviewer-1`, `1-validator-1-3-1`, `10-reviewer-1`, `2-reviewer-1` and `1-stranger-1`, and an unregistered folder `1-baseline-1`.  `review clean 1` removes the first three and the folder, and keeps the other three.  `git worktree list` no longer shows the removed ones.  A bad round (`0`, `x`) exits 3 with the usage.

### Step 5: the docs, and the text for the orchestrator (DOC-1)

* `docs/working-with-agents.md`: a line in "Review a change" (the skill removes a round's worktrees), a recipe line for cleanup, and a changelog line for M1.  Say that `cargo clean` stays a manual step, and that the D7 entry covers only `cargo run -p xtask -- clean --yes`, so a run with `--also` asks for approval.
* `README.md`: add a short line for `xtask clean` beside the other xtask commands, if its section lists them.
* `docs/tech-debt.md`: an entry for each gap that M1 leaves, in the commit that leaves it.  Candidates: `lsof` or `gh` missing keeps items; an agent that builds with no file open in its worktree is not seen as a user of it, and only the open-round test protects it.
* The implementer gives the team leader, as text, not as a commit:
  * the new lines of `.claude/skills/review-rounds/SKILL.md`: at the end of step 11, after the two tables, run `target/debug/xtask review clean $1`.  If auto mode or a person refuses it, give the command to the person and go on.  It runs after the findings file, where the approvals window of step 11 ends, so the round's counts do not include its approval.  If the Write tool refused the log, the caller runs the command when it writes the log.
  * the line to add to `REVIEW_SKILL_TEXT` in `xtask/tests/rules.rs` (`target/debug/xtask review clean $1`), with the comment change;
  * the guard change in `.claude/hooks/read-only-bash.sh`, so `check_xtask_words` refuses `clean` too, its header comment names `clean`, and its cases in `xtask/tests/rules.rs` (`target/debug/xtask clean --yes` and `cargo run -p xtask -- clean` refused).
* The orchestrator lands the guard change before round 1, so no review agent of M1 can run `xtask clean --yes` on this repo.  A removed branch would also fail `review check`.

### Step 6: review rounds

`cargo build -p xtask`, then `/review-rounds 13dc6e3 <round> "<criteria>"` for rounds 1, 2 and 3, and more until a round has no blocker and no significant should-fix.  The criteria are the parent plan's M1 section and this plan.  Round 1 passes `--baseline`.  Each confirmed finding gets a fix with a test; a nit gets a fix only if the team leader agrees.  After the last round, the team leader copies `target/review/run/harness-phase-3b/log.md` to `docs/reviews/harness-phase-3b-m1.md` and removes the run folder (TD-37).

## Validation and acceptance

* `cargo test -p xtask` passes, with the tests of steps 1 to 4.  Each fixture test fails on `13dc6e3`, where `clean` and `review clean` do not exist.
* `cargo run -p xtask -- clean` in this repo lists and changes nothing; the team leader runs it once and records the output in Surprises.
* `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo run -p xtask -- docs-check` pass.
* The review rounds end by the stop rule, and the log is in `docs/reviews/harness-phase-3b-m1.md`.

## Idempotence and recovery

* The list mode changes nothing, so it is safe to run at any time.  `--yes` gathers again before it removes, and a second run removes nothing more.
* A removed branch has its tip in the output, so `git branch <b> <sha>` brings it back while the object exists.
* Each step is its own commit, which `git revert` undoes.  A new team leader takes over from Progress.

## Progress

* [x] 2026-10-31: plan written (`88cc3cd`).
* [x] 2026-10-31: plan review round 1 (one `Plan` agent): 2 blockers, 8 should-fix and 7 nits.  All fixed in this plan (`3f77c8b`).
* [x] 2026-10-31: plan review round 2 (a new `Plan` agent): 1 blocker, 5 should-fix and 5 nits.  All fixed in this plan.  The review stops at its cap of 2 rounds.
* [x] 2026-10-31: step 1, the list mode of `xtask clean`, `serve::pid_file`, the `clean` row of the guide and TD-67 (implementer).
* [x] 2026-10-31: step 2, the pid file of `xtask serve`, and TD-68 (implementer).
* [x] 2026-10-31: step 3, `xtask clean --yes` (implementer).
* [x] 2026-10-31: step 4, `xtask review clean <round>`, and its row in the guide (implementer).
* [x] 2026-10-31: step 5, the guide and `README.md`.  The texts for the skill, `xtask/tests/rules.rs` and the guard went to the team leader, checked in a scratch worktree: `cargo test -p xtask --test rules` passed with all three, and failed without the two files under `.claude/` (implementer).
* [x] 2026-10-31: checkpoint: the xtask code is done (`9bc3de7` to `b7e1088`).  The team leader checked that the three texts apply, and gave them to the orchestrator.  Next: the orchestrator lands them, then round 1.
* [x] 2026-10-31: the orchestrator landed the texts of step 5 for the skill, the guard and `xtask/tests/rules.rs` (`d0164ea`).  It changed M1 in the parent plan to the cleanup rules that the M1 team found, and recorded why in its Decision log (`cfaf8fd`, `9cf2fd0`).
* [x] 2026-10-31: review round 1 (`--baseline`, at `cfaf8fd`) ran to step 9.  The Write tool refused the findings file to the team leader, so the round stopped there, and the orchestrator writes the file.  Found: 1 blocker (1-b1, a nested worktree), 10 should-fix and 20 missed mutants confirmed, 3 mutants refuted.  The baseline comparison: of 4 baseline findings, the round found 2.
* [x] 2026-10-31: round 1 fixes (`57b2218` to `da0c63b`, implementer): every confirmed finding has a fix and a test that failed first, also the nits 1-4, 1-5, 1-9, 1-13, 1-14 and 1-b5.  1-11 has no fix (orchestrator, parent plan Decision log).  The team leader recorded the list mode on this repo in Surprises (1-b3).
* [x] 2026-10-31: review round 1 fix of 1-b1: a worktree that holds a worktree that stays stays too, to a fixed point (implementer).
* [x] 2026-10-31: review round 1 fix of 1-1: the tree proof counts only the commits of `<tip>..origin/main` (implementer).
* [x] 2026-10-31: review round 1 fix of 1-2 and 1-b2: `--also` refuses a path in any worktree of the repo (implementer).
* [x] 2026-10-31: review round 1 fix of 1-3 and 1-b4: a test of a failed `git worktree remove`; a failed detached worktree holds its `detached-<short sha>` run folder too; the words of the removal are a constant with a test (implementer).
* [x] 2026-10-31: review round 1 fixes of 1-6, 1-7 and 1-9: `review clean` tests gain a round-named worktree outside `target/review/` and a locked one; `target/review` is canonical as the listed paths are (implementer).
* [x] 2026-10-31: review round 1 fixes of the missed mutants 1-16 to 1-36 (but the refuted 1-22, 1-24 and 1-27), 1-b5, 1-4 and `lsof -w` (implementer).
* [x] 2026-10-31: review round 1 fixes of 1-5, 1-12, 1-13, 1-14 and 1-15: the guide says that `--yes` stops each running `xtask serve` of this repo, with a warning; the plan body has no prune; the Progress names `d0164ea`, `cfaf8fd` and `9cf2fd0` (implementer).
* [x] 2026-10-31: review round 2 fix of 2-1: `--yes` removes a worktree through `clean::remove_worktree`, with a test on a worktree with changes (implementer).
* [x] 2026-10-31: review round 2 fix of 2-2: a test of a pid file whose `xtask serve` does not listen on its port, and of `-a` and `-sTCP:LISTEN` in the `lsof` words (implementer).
* [x] 2026-10-31: review round 2 fix of 2-3: `plan` and `gather` make the two roots canonical, so a linked `target/` keeps its reviewer worktrees in the list (implementer).
* [x] 2026-10-31: review round 2 fixes of 2-4, 2-5, 2-6, 2-8 and 2-10: the `lsof -w` texts, one pass for the nested rule, the guide on `--review-only` and a refused log, the nested rule in the plan and the guide, and a total that counts a nested worktree once (implementer).
* [x] 2026-10-31: review round 2 (at `32df7e0`, no `--baseline`): 4 should-fix confirmed (2-1, 2-2, 2-3, 2-8) and 6 nits, no blocker; no missed mutant.  Each confirmed finding and the nits 2-4 to 2-10 are fixed in `64aeb11` to `4645e64`; 2-7 and 2-9 went to the orchestrator as a diff.  Round 3 follows.
* [x] 2026-10-31: the orchestrator landed the round 2 texts for the skill and the pins in `xtask/tests/rules.rs`, and the parent plan's M1 bullet on `review clean` (`bedfd00`).
* [x] 2026-10-31: review round 3 fix of 3-1: a prunable worktree whose folder still exists is kept, since only its `.git` file is gone (implementer).
* [x] 2026-10-31: review round 3 fixes of 3-6, 3-7 and 3-9: `gather` stores the canonical roots and links in the survey, so `plan` is pure again; the total test has sizes that differ; a list test with a linked `.claude/` (implementer).
* [x] 2026-10-31: review round 3 fix of 3-2: the same `--also` folder twice is one scratch root, and a folder in another `--also` folder is refused (implementer).
* [x] 2026-10-31: review round 3 fixes of 3-3, 3-4 and 3-8: the guide names a refused findings file, the plan gives `lsof -w`, and Progress names `bedfd00` (implementer).
* [x] 2026-10-31: review round 3 (at `bedfd00`): 3 should-fix confirmed (3-1, 3-6, 3-7) and 6 nits, no blocker; mutants full, none missed.  3-1 fails the acceptance "a second run lists nothing" in a case a user can reach, so the rounds go on to round 4.  The fixes are `65ce7f8` to `1763373`; 3-5 has no fix (Decision log).

## Surprises and discoveries

* 2026-10-31, implementer: step 1's tests failed first.  The tests of `plan` in `clean.rs` failed 51 of 57 with a `plan` that gave no items; the other 6 test helpers.  The tests in `xtask/tests/clean.rs` failed with "unknown command clean" before the arm in `main.rs`.
* 2026-10-31, implementer: step 2's two tests in `xtask/tests/serve.rs` failed first.  The pid file was missing, and stderr did not say "no pid file" outside a repo.
* 2026-10-31, implementer: step 3's tests failed first.  The two `--yes` tests in `xtask/tests/clean.rs` failed with the usage, exit 2.  The new tests in `clean.rs` did not build: no `stop`, no `STOP_WAIT`, and `report` took no `--also`.
* 2026-10-31, implementer: step 4's two tests in `xtask/tests/review.rs` failed first, with the usage and exit 3.
* 2026-10-31, implementer: `git worktree remove` removes a worktree whose folder is gone (git 2.18 or later).
* 2026-10-31, implementer: `git rev-parse --short` takes one revision only.  `git log --no-walk=unsorted --format="%H %h"` gives the short form of many commits in one call.
* 2026-10-31, implementer: the tests of 1-b1 failed first.  The four new tests of `plan` failed, and `yes_keeps_a_merged_worktree_that_holds_a_changed_worktree` found agent-x removable.
* 2026-10-31, implementer: the test of 1-1 failed first: a branch from `git revert` of main's tip was removable by the tree proof.
* 2026-10-31, implementer: the test of 1-2 failed first: `--also` took a linked worktree in the temporary folder, with exit 0.
* 2026-10-31, implementer: the test of 1-3 failed first, on the run folder `detached-<short sha>` of a failed detached worktree.  With the two mutants of 1-3 (no `branch` in the worktree item, and `!=` for the run folder kind), the test fails too.  When `git worktree remove` cannot delete a file, it still deletes the worktree's record.  Then `git branch -D` would delete the branch, so only the `stuck` set keeps it.
* 2026-10-31, implementer: the tests of 1-6 and 1-7 cover code that was right, so they failed first with the mutants of the findings: no filter on the parent folder, and no `!registered.contains(p)`.  The test of 1-9 failed first: through a linked `target/`, `review clean` removed the folder and left the worktree registered.  For a locked worktree, `git worktree remove --force` exits 128, and the worktree stays.
* 2026-10-31, implementer: the new tests for the missed mutants cover code that was right.  Each failed first with its mutant: `&&` for `||` in `worktree_status` (1-16), `starts_with("target")` (1-b5), no `size` on a worktree item (1-21), an empty map from `short_shas` (1-25 to 1-35), and `||` in the test of an open round (1-31).  The tests of `lsof`, `ask`, `stop`, `human_size` and `dir_size` did not build or failed before.  The test of 1-4 failed first: the list mode wrote the index of each linked worktree, until `git --no-optional-locks status`.
* 2026-10-31, team leader: `cargo run -p xtask -- clean` on this repo at `da0c63b` (list mode, exit 0) changed nothing: `git status`, the branches and the worktrees stayed the same.  Its output: "can remove (1 items, 0 B)": branch `harness-tweaks-2026-10-31` (13d9cca), "its tip is an ancestor of origin/main".  "keeps (3 items)": branch `harness-phase-3b` and the run folder `target/review/run/harness-phase-3b`, "the current branch of a worktree"; branch `main`, "main and gh-pages stay".  "target/ holds 14.6 GiB."  This closes round 1's finding 1-b3.
* 2026-10-31, implementer: the test of 2-1 did not build first (no `remove_worktree`), and it failed with `--force` added at the call site.
* 2026-10-31, implementer: the code of 2-2 was right, so its tests failed first with the mutants of the finding: `Process::Serve` for `Answer::No` in `probe`, and no `-a` in `lsof_listen_args`.  Both stopped the server.
* 2026-10-31, implementer: the test of 2-3 failed first: with a linked `target/`, the list had no worktree items.  A path through a linked `target/` or `.claude/` now shows as under it.
* 2026-10-31, implementer: the test of 2-10 failed first: the total of a worktree and the worktree in it was 30 B, not 20 B.  2-5 changes no behaviour, so the nested tests of 1-b1 cover it.
* 2026-10-31, implementer: the test of 3-1 failed first: a worktree with no `.git` file was removable as "prunable: its folder is gone", and `--yes` failed on it with exit 1.
* 2026-10-31, implementer: the test of 3-6 did not build first (no stored roots).  The tests of 3-7 and 3-9 cover code that was right, and failed first with their mutants: `w.starts_with(p)` in the total, and no `canonical` on the `.claude/worktrees` root.  A first try of the 3-9 test passed with the mutant: `git add -A` had committed the `.claude` link on a branch, and `git switch main` then deleted it.
* 2026-10-31, implementer: the test of 3-2 failed first: the same folder twice gave two scratch items.

## Decision log

* 2026-10-31, team leader: `xtask clean` also removes a branch whose tip is an ancestor of `origin/main`.  It is the plain merge, and it loses no commit.  The parent plan names only the squash and PR proofs.
* 2026-10-31, team leader: a run folder's log may match any file in `docs/reviews/` at `HEAD` or `origin/main`, by blob.  The copy of a milestone run is named `<branch>-m<n>.md`, not `<branch>.md`.
* 2026-10-31, team leader: `--also` accepts only folders under the temp folder.  A wrong path such as the home folder could otherwise lose a person's files.
* 2026-10-31, team leader: `review clean` removes a round's worktrees with `--force`, because a reviewer's trial changes are scratch by design.  `xtask clean` never removes a worktree with changes or ignored files outside `target/`.
* 2026-10-31, team leader: the guard change for `xtask clean` is the orchestrator's, since it is under `.claude/`.  It must land before round 1.
* 2026-10-31, team leader: the parent plan's "a second run lists nothing" means that a second run lists nothing to remove.  The kept items stay in the list, with their reasons, as the parent plan's "the list mode names each one" needs.
* 2026-10-31, team leader: a process "uses" a worktree if it has its working folder or an open file there (`lsof -F n`), not only its working folder.  A reviewer builds from the main checkout, so the open-round test also protects a worktree while its round runs.
* 2026-10-31, team leader: `--yes` runs `git worktree prune` only if each prunable worktree is removable, though the parent plan says "then `git worktree prune`".  A prune acts on the whole repo, and can drop a moved worktree outside the two roots.
* 2026-10-31, team leader: a worktree under `.claude/worktrees/` goes only when its work is merged.  A running Claude Code agent keeps no file open between tool calls.
* 2026-10-31, team leader: the plan review stopped at its cap of 2 rounds.  Round 2's findings are fixed in this plan, with no third review.
* 2026-10-31, team leader: a pid runs `xtask serve` only if its command line says so and it listens on the port of the file.  So a reused pid is not stopped.
* 2026-10-31, implementer: the tests that run the binary put a fake `gh` first on `PATH`, and use the real `ps` and `lsof`.  The tests of `plan` give it a survey made by hand.  `Facts::process` is a function, so a test of `gather` could give fake answers for each pid.
* 2026-10-31, implementer: `--also` also refuses a folder in the repo root.  The plan refuses only the root and a folder that holds it.  In the repo, `remove_dir_all` would lose tracked files.
* 2026-10-31, implementer: the ref test of a worktree uses the set of branches that have a proof, before the rule of the current branch.  A ref passes if it is not such a branch, or if the worktree's `HEAD` is that branch's tip, which its proof covers.  So the worktrees do not depend on the branches that this run deletes, and a worktree on any branch passes.
* 2026-10-31, implementer: step 1 has no `--yes`.  The command refuses it with the usage until step 3, so the tests of step 3 fail first.  The last line of the list, which gives the `--yes` command, comes in step 3 too.
* 2026-10-31, implementer: the kinds print as `pid-file`, `worktree`, `branch`, `run-folder`, `build` and `scratch`, in that order.  A kept item shows no size.  A run folder `detached-<short sha>` of a detached worktree that stays is kept as "the current branch of a worktree".  A pid file with a pid below 2 holds no pid, since `kill 0` signals a process group.
* 2026-10-31, implementer: `--yes` never runs `git worktree prune`.  It removes each removable prunable worktree with `git worktree remove`, as it does the others.  So it never acts on a worktree that it did not name, and a prunable worktree that stays does not stop the others.
* 2026-10-31, implementer: if `git worktree remove` fails, `--yes` does not delete that worktree's branch or remove its run folder, and prints `failed` for each.  The survey decided them as if the worktree went.
* 2026-10-31, implementer: `--yes` acts on the pid files of the first survey only.  The second survey gives the other items.
* 2026-10-31, implementer: the guide says that the skill runs `review clean`, and that the guard refuses `xtask clean`.  Both are true only when the orchestrator lands the texts of step 5, which the plan puts before round 1.
* 2026-10-31, implementer: `review clean` runs no `git worktree prune`, and prints no `pruned` line.  `git worktree remove --force` also removes a registered worktree whose folder is gone, so a prune is not needed.  A prune acts on each worktree of the repo.  The test isolates git from the person's config, so the helpers in `xtask/tests/review.rs` now set `GIT_CONFIG_GLOBAL=/dev/null` and `GIT_CONFIG_NOSYSTEM=1`.
* 2026-10-31, implementer: the choice of `git worktree remove` without `--force` is in `clean::remove_worktree`, which `--yes` calls.  Review round 2 (2-1) found that the first test pinned only the constant.  A test now gives the function a worktree that already has changes: no timing is needed.
* 2026-10-31, team leader: 3-5 has no fix and no test.  The serve test does not pin that the pid file comes before the `serving` line.  The order is a race with no effect for a user, but a clean run in the same instant.

## Outcomes and retrospective

To be written when the milestone completes.
