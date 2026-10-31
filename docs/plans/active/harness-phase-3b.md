# Harness phase 3b: runs that drive themselves, at less cost

## Purpose

The maintainer wants a big task to run hands-off as one PR, with an orchestrator and a team for each milestone, as PR #39 did.  Today that mode exists only as prose that a person copies into each plan.  Review rounds after round 3 mostly fix nits.  Every agent runs on the strongest model.  Each commit runs every local check.  Nothing removes the build output, worktrees and branches that a run leaves.

This phase makes the hands-off mode the default for a big task, with one question to the maintainer at the start.  It makes the fix rule stricter after round 3, measures cheaper models for each agent role, runs local checks by changed paths, and cleans up after each run.  It also gives each PR a short summary for people, a `<details>` section for agents, and inline comments where a person must look.

Readers: the orchestrator session that runs this plan, its teams, and the maintainer.

## Context and orientation

### Where this phase sits

`plans/active/harness-engineering.md` has phases 1 to 6.  Phases 1 to 3 are done; the gate of phase 3 is open.  This phase comes before phase 4, because phases 4 and 5 add more agents, and each agent costs more than it must today.  Milestone M8 re-checks phases 4 to 6 and the phase 3 gate against what this phase measures.

### What the retrospective found (2026-10-19 to 2026-10-30)

Sources: the Claude Code sessions of that period, PRs #13 to #39, the 14 logs in `reviews/`, the completed plans, decisions 0001 to 0022, and CI runs.  The research notes stay outside the repo, because they quote session transcripts.

Big work without a hands-off mode:

* Harness phase 2 had 21 milestones.  It ran as about 17 sessions and 18 PRs (#14 to #33) over 77 hours.  Each session needed the same kick-off text, a decision, "start, push and open a pr and watch ci" and a merge by hand: at least 54 maintainer messages.
* Phase 3 (PR #34) and stream-from-storage (PR #39) ran as one PR with an orchestrator.  Before PR #39's run, the planning session did M0 to M2 inline and opened extra branches and PR #38.  The maintainer stepped in three times in six minutes.  The agent then said that the mode exists only as a "How the work runs" section in a plan, and this plan had none.
* Signals that a task is big, seen at the start: a phase or milestone plan, more than 3 milestones, a decision point in more than one milestone, work longer than one session, or the maintainer away for the run.

Review rounds (all 14 logs: 66 rounds, 586 agent runs, 1,402 findings):

* Rounds 4 and later fixed 124 findings.  13 were real bugs (10%); 71 were nits.  One bug was a regression that a fix made (catch-up 6-1 to 7-1).  At least 3 of the 13 were in files that the fix range of the round before did not touch: catch-up 4-1, M7 6-1 and M7 6-2 (plan review, 2026-10-31).
* Confirmed bugs by round: round 1 26, round 2 12, round 3 7, rounds 4 and later 13.
* Missed mutants are 691 of the 1,402 findings (49%) and 424 of the 430 refuted verdicts.  95 of the 424 were in test files (`app/src/save_tests.rs` 68, `app/src/decode_tests.rs` 26).  Most of the rest were wasm32-only code.  In M8 round 1, 68 of the 81 refuted mutants were in `app/src/save_tests.rs`.
* Later in PR #39, `.cargo/mutants.toml` left out those test files and each wasm32-only file.  From `6d0a39d`, a wasm32-only item in a shared file carries `mutants::skip`.  Most of the 141 refuted mutants of `app/src/lib.rs` and the 46 of `protocol/src/lib.rs` are in logs before that commit.  A wasm32 block in a shared function stays in (TD-33).
* Confirmed mutants that went to TD-39 or TD-66 come back each round, because `repeat-of` matches only refuted findings.
* The validator refuted about 2% of reviewer findings: 6 of 288 to 310, by which verdicts count.  4 of the 6 are in the PR #39 logs, one is in PR #35 (`6a4dcf2`) and one is a baseline finding of PR #37 (`0732e53`).  The docs-reviewer's findings are 62% nits, and it uses about 20% of a round's tokens.  The reviewer and the memory-reviewer found 25 and 5 bugs that no other agent found.
* The rule that rounds go past 3 until the stop rule holds lives only in a completed plan (`plans/completed/stream-from-storage.md`, Decision log, 2026-10-27 and 2026-10-28).  `CLAUDE.md`, decision 0011 and `working-with-agents.md` still say three rounds.

Cost and time:

* PR #39: 4.6 days, 142 files, +51.7k lines, 194 commits, about 80% of a week's usage limit.  Every subagent ran on Opus 5.5 at high effort, because every file in `.claude/agents/` says `model: inherit`, and `agents_folder_problems` in `xtask/src/rules.rs` requires it.
* Prices per million tokens, input and output: Opus 5.5 $4 and $20, Sonnet 5.5 $2 and $10, Haiku 5.5 $0.10 and $0.50 for prompts up to 100k tokens ([pricing](https://platform.claude.com/docs/en/about-claude/pricing)).
* A subagent can take `model` and `effort` in its frontmatter, and an Agent call can override both ([sub-agents](https://code.claude.com/docs/en/sub-agents), [model config](https://code.claude.com/docs/en/model-config)).
* The token figures in the logs are final contexts, not tokens used (TD-44).  The phase 3 replay found that a whole case uses 16 to 20 times more.

Local checks and CI:

* Every CI failure in the period was in the browser job (UI snapshots, e2e).  No run failed in rustfmt, clippy or the wasm build, so no local check would have caught one.
* A close-out commit of PR #39, with comments and paths only, ran the full local set for 16 minutes; CI took 7.5 minutes.  The maintainer asked "are we stuck?" during that silence.
* The workspace tests take about 2.5 minutes locally (386 tests) and 5.4 minutes in CI.  fmt, clippy (warm) and docs-check take seconds.

PR descriptions and comments:

* Bodies grew from about 1,500 characters to 15.5k (#39: 2,707 words, no headings, a log of the run).  The "Not verified" section of #13 to #32 is gone from #34 on.
* No PR has an inline review comment or a human comment.  Copilot code review runs on each push by the `main` ruleset, but each of its 28 reviews on #39 says that its quota is used up.

Cleanup:

* Seen leftovers: 53 GB of replay clones in `/private/tmp/stemyla-harness`, 4.3 GB of reviewer worktrees in `target/review`, 15 GB in one `target/review/run/<branch>`, `target` at about 91 GB (the disk filled once), 40 to 45 squash-merged local branches, `worktree-agent-*` branches, a `.claude/worktrees` worktree, an untracked `tests/` folder, `xtask serve` left on port 8080, and `caffeinate`.
* Nothing in the repo removes them.  The agent listed them and waited.  Auto mode refused its `rm -rf` and `git branch -D`, so the maintainer ran `cargo clean` by hand.
* Claude Code's worktree sweep removes only worktrees that it made, and only after `cleanupPeriodDays` ([worktrees](https://code.claude.com/docs/en/worktrees)).  The reviewers make theirs with `git worktree add`, so the sweep never sees them.
* `git branch --merged` does not see a squash-merged branch.

### Files this phase changes

* `.claude/skills/review-rounds/SKILL.md`: the fix rule, the stop rule, the narrow range of late rounds, a cleanup step.  `the_review_skill_starts_with_xtask` pins about 55 of its lines.
* `.claude/skills/orchestrate/SKILL.md` (new): the hands-off mode.
* `.claude/agents/*.md`: `model` and `effort` for each role.  `agents_folder_problems` (`xtask/src/rules.rs`, near line 3159) requires `model: inherit` today; `baseline.md` is frozen by a hash.
* `xtask/src/review.rs`: the mutants filter, the known-missed list.
* `xtask/src/` (new subcommands): `clean`, `review clean`, `check`, `pr-body`, `attention`.  `xtask/src/docs.rs`: the check of M6.
* `docs/working-with-agents.md` in each milestone that adds a subcommand or a skill, because `guide_covers` (`xtask/src/docs.rs`) fails without it.
* `.github/pull_request_template.md` (new).
* `CLAUDE.md` (88 lines; the map limit is 100), `docs/invariants.md` (GIT-2), `docs/plans/PLANS.md`, `docs/working-with-agents.md` (it must name each `xtask` subcommand, skill and agent: `guide_covers`), `docs/harness.md`, `docs/tech-debt.md`, decision 0011.
* Decisions 0023 to 0027, written as `proposed` with this plan.

### Terms

* Orchestrated mode: one main session (the orchestrator) runs a plan's milestones in order.  For each milestone it starts a team leader, which starts the implementers and reviewers.  The orchestrator does not read code.
* Big task: a task that the sizing rule of decision 0023 marks.
* Strict rule: a rule whose findings get a fix in any round (decision 0024).
* Fix range: the commits made since the previous round started.

## Plan of work

Each milestone is one or more signed commits on one branch and one PR.  Each milestone gets review rounds by the fix rule of decision 0024 from M3 on; M1 and M2 use the rule of today.  Each milestone ends with the checks of GIT-2 as they stand at that time.

### M1: cleanup (`xtask clean`)

An `xtask clean` subcommand finds and removes what runs leave.  It lists by default, with sizes, and removes only with `--yes`.

* It covers:
  * `target/review/run/<branch>/` for a branch with no open PR, but only when its `log.md` equals the copy in `docs/reviews/`;
  * worktrees under `target/review/` with no changes and no live process, and under `.claude/worktrees/` only when their work is merged.  `git worktree remove` also handles a worktree whose folder is gone, so `git worktree prune` is not needed;
  * `target/e2e/` and `app/dist/`;
  * a local branch whose tree equals a commit on `origin/main` that the branch does not hold, or whose merged PR has a `headRefOid` equal to the branch's tip;
  * a `xtask serve` whose pid file is in `target/`, if that pid still runs `xtask serve`.  M1 makes `xtask serve` write the pid file.
* It never removes a worktree with changes, or a worktree with ignored files outside its `target/` (ASSET-1).  It never removes a branch that it cannot prove merged, or the current branch.  It never removes anything outside the repo, but a scratch folder under the temp folder that `--also <path>` names.
* `cargo clean` stays a separate, manual step; `xtask clean` prints the size of `target/` and the command.
* The `review-rounds` skill runs `target/debug/xtask review clean <round>` after the round's log is written.  It removes that round's worktrees only (`target/review/<round>-<name>-<n>`).  The form fits the skill's `allowed-tools`, but only in the turn in which the skill loads.  In a later turn, auto mode decides.  If it refuses, the skill reports the command and goes on.
* The orchestrator runs `cargo run -p xtask -- clean --yes` between milestones and at close-out.  The maintainer's `autoMode.allow` entry covers that form (D7).
* `working-with-agents.md` names `clean` and `review clean`.
* Acceptance: unit tests on a temporary repo with planted leftovers:
  * a branch merged by squash, an unmerged branch, and a branch whose name matches a merged PR but whose tip differs;
  * a dirty worktree and a clean worktree;
  * a stale pid file, and a pid file whose pid runs another program;
  * a run folder whose log has no copy in `docs/reviews/`.
* The list mode names each one.  `--yes` removes only the safe ones, and a second run lists nothing.  Each test fails before the change.

### M2: less mutants noise

PR #39 already left out the test files and the wasm32-only files (`.cargo/mutants.toml`).  So this milestone starts with a new baseline.

* First, measure the noise that is left.  Use the M8, M9 and catch-up logs of PR #39, which come after `6d0a39d`.  Count the missed lines by class: a file that today's `.cargo/mutants.toml` leaves out (such as `app/src/save_tests.rs`, out from `cfeefda`), a wasm32-only item with no `mutants::skip`, a wasm32 block in a shared function, a known-missed mutation, and the rest.  Record the counts in Surprises.  If the noise that is left is small, the team says so and cuts the milestone to fit.
* `xtask review mutants` drops each missed line whose enclosing item has `#[cfg(target_arch = "wasm32")]` or a `cfg` that is false on the native build.  This is a backstop for an item that lacks `mutants::skip`, such as `can_encode` at `f28d22d:app/src/lib.rs`.  It parses the file with `proc-macro2`, which xtask already uses with `span-locations`.  `syn` is not an xtask dependency (DEPS-1).  It drops the lines before the validator sees them, and counts them in the log.
* A wasm32 block inside a shared function stays in (TD-33).
* A known-missed list (`xtask/src/review/known_missed.txt` or a table in `review.rs`) holds the mutants of TD-39 and TD-66, each as a function and a mutation.  A missed mutant on the list is logged as known, not sent to a validator.  Another mutant in the same function still goes to a validator.
* The list is the mutants' side of M3's `repeat-of` match to tech debt.  M3 matches reviewer findings; the list matches mutants.  An entry names its TD.
* Acceptance: unit tests with fixture lines from PR #39's logs (`refs/remotes/origin/pr/39`, see preflight).
  * The filter drops a mutant in a `#[cfg(target_arch = "wasm32")]` item of a shared file, such as `can_encode` at `f28d22d:app/src/lib.rs`.
  * It keeps each confirmed mutant that is not on the list.  Findings 1-41 and 1-54 of round 1 in `iphone-crashes-2026-10-25-m8.md` (`xtask/src/e2e/save.rs:427:39`, `*` to `+` and `*` to `/`) still go to a validator; `2fdecda` fixed them with tests.
  * It logs as known the mutant of finding 2-17 of round 2 (`save.rs:503`, the body replaced), which went to TD-39.
* Each test fails before the change.  The skill's pinned lines and the log format test change in the same commit.

### M3: the fix rule and the stop rule (decision 0024)

* The skill, `CLAUDE.md`, `working-with-agents.md` and decision 0011 say the same rule:
  * Rounds 1 to 3 fix every confirmed finding, and each nit that the team leader agrees with.  A nit stays `unvalidated`, as the skill says today.
  * From round 4, a round fixes only blockers, significant should-fix and any confirmed finding on a strict rule.  It logs the rest, nits included, as "reported, not fixed".
  * The rounds stop at the first round from round 3 on with none of those three.  There is no cap.
  * From round 4, the reviewers read the fix range and the files it touches, not the whole branch (D4).
* The skill copies the log to `docs/reviews/` after the last round, not after round 3.  So `xtask clean` keeps a run folder until the copy exists.
* The findings log gets a `significant` column, filled by the team leader or orchestrator, not by the reviewer.  The log's "reported, not fixed" list feeds the PR's `<details>` section.
* `repeat-of` also matches a confirmed finding that went to tech debt, so a deferred finding is not fixed or validated again.
* Expected effect, from the logs: from round 4, about 71 nit fixes stop.  29 of the 53 confirmed late findings are on VERIFY-1, so they are still fixed.
* Acceptance:
  * xtask has no parser for the findings table today; only the Rounds header is pinned (`xtask/tests/rules.rs`).  M3 adds a pure check of a log's findings table.
  * The check applies only to logs that carry the `significant` column, so old logs in `docs/reviews/` still pass.
  * Its tests accept the new column, and reject a fix of a nit from round 4, or of a non-significant should-fix not on a strict rule.
  * `docs-check` runs the check on each file in `docs/reviews/` that has the column, so a real log can fail it.
  * The pinned skill lines change.  A test pins one sentence of the rule in each of the four places, so a drift fails.

### M4: local checks by tier (decision 0026)

* An `xtask check --tier commit|push` subcommand reads the changed paths, picks the checks, prints what it runs and what it skips and why, and runs them.
* Tier `commit` reads the commit's own changes: the staged and unstaged changes against `HEAD`, and the untracked files (`git ls-files --others --exclude-standard`).  On a clean tree, it reads the last commit (`HEAD^..HEAD`).  A test covers each case.  The branch's diff against `main` would select almost every check on a long branch.
* Tier `commit`: `cargo fmt --all --check`; workspace clippy; the tests of each changed crate and of the crates that depend on it; `docs-check` when a doc, `CLAUDE.md`, `.claude/` or `xtask/` changed; the wasm32 clippy when `app`, `worker`, `protocol` or `engine` changed; the `synth` or `app` clippy variant when that crate or a feature changed.
* Tier `push`, and before each review round: the fmt, clippy, test, `docs-check` and Trunk build lines of `CLAUDE.md`'s Commands.  It reads the paths against the merge base with `origin/main`.  `serve`, `web-test`, `e2e` and `publish` are not in it.
* A commit that touches a strict rule's paths runs tier `push` (D2).  No rule has such paths today, so M4 writes a table of paths for each strict rule.
  * Name the files that hold the rule's code, narrower than the `paths:` of `.claude/rules/`.  For example, `.claude/rules/memory.md` covers all of `engine/**`, `app/src/**`, `worker/**` and `decoder/**`.
  * VERIFY-1 and GIT-1 get no paths, since VERIFY-1 is on 870 of 1,402 findings and GIT-1 has no code.
  * Record in Surprises the share of PR #39's commits that the table sends to tier `push`.
* CI stays the authority.  The browser tests stay in CI only, except a scenario that CI failed.
* GIT-2 in `invariants.md` and `CLAUDE.md` names the tiers.  `working-with-agents.md` names `check`.
* Acceptance: unit tests of the path-to-check table and of the strict-rule path table, one for each row.  A timed run of each tier on the same change is recorded in this plan's Surprises.

### M5: a model and effort for each agent role (decision 0025)

* First, `agents_folder_problems` accepts `model` from a fixed list (`inherit`, `opus`, `sonnet`, `haiku`) and an optional `effort` key.  `AGENT_KEYS` (`xtask/src/rules.rs`) requires every key today, so `effort` must be optional and must come before `hooks`.  `baseline.md` stays as it is.
* The measurement.  The team writes the pass rule and the ground-truth list into this plan's Decision log before any run, through the orchestrator:
  * Commits: the PR #39 logs (`reviews/iphone-crashes-2026-10-25-*.md`) at `refs/remotes/origin/pr/39`; PR #35 and #37 at `refs/remotes/origin/pr/35` and `/37` (preflight).
  * Match: a run's finding matches a ground-truth item by the matching rule of `plans/completed/harness-phase-3-m5.md` (step 6, item 2), which `reviews/replay.md` uses.
  * Cost of one run, in used tokens (`reviews/replay.md`): docs-reviewer about 1.5M, a validator 0.06M to 0.37M, supply-chain-reviewer about 1.1M.  The design must fit the budget below.
  * validator, first: 40 logged findings, stratified.  The set holds all 6 refuted reviewer findings, refuted mutants, and confirmed findings.  Arms: `sonnet` and the control, one run each (about 16M).
  * docs-reviewer, second: the rounds with the most docs findings, until the ground truth has at least 40 items (about 8 rounds).  Ground truth: the docs-reviewer's own confirmed blockers and should-fix in those rounds.  Arms: `sonnet` and the control, one run each (about 24M).  With fewer than 40 items, the 95% interval is wider than 0.3 at most recalls, so the arm cannot pass.
  * Last, only if budget is left: `haiku` for the validator, then the effort arms.  A role not measured keeps `inherit`, and Surprises says so.
  * gpu-reviewer, wire-reviewer, supply-chain-reviewer: effort `medium` against the control at `high`, on their logged rounds.
  * Runs: one run for each arm, to fit the budget.  So the measure does not separate run noise from the model's effect; the results table says so.
  * Before the runs, the team works out each arm's expected interval width from its n.  It drops an arm that cannot meet the 0.3 limit, and records why.
  * Recall: the share of ground-truth items that an arm's runs find, with a 95% interval.
  * Bug: a ground-truth item of Kind `bug`.  Missed: the control found it in at least one run, and the candidate found it in no run.
  * Pass rule for a reviewer: recall within 0.15 of the control, and no bug missed.
  * Pass rule for a validator: no confirmed bug refuted, and at least as many refutations of the refuted items as the control.
  * A role whose interval is wider than 0.3 keeps `inherit`, for want of evidence.  gpu-reviewer has only about 6 confirmed findings, so it will likely keep `inherit`.
  * Tokens: read from each run's `modelUsage` (used tokens, not final context; TD-44).
  * Budget: about 45M `modelUsage` tokens.  One milestone's rounds 1 to 3 in the PR #39 logs used about 2.7M final-context tokens (1.6M to 2.9M).  A whole case uses about 16 times its final context.
  * The team checks the budget after each run, in the order above.  It stops at the budget with what it has.
* Not changed, whatever the result: reviewer, memory-reviewer, the intent reviewer, the orchestrator and baseline stay on the strongest model.
* Then set `model` and `effort` for each role that passes.
* Acceptance: the results table with 95% intervals is in this plan's Surprises.  The agent files match it.  `agents_folder_problems` has a test for the new keys that fails before the change.

### M6: orchestrated mode as a skill (decision 0023)

* `.claude/skills/orchestrate/SKILL.md` holds the generic parts of the two "How the work runs" sections (`plans/completed/harness-phase-3.md` and `plans/completed/stream-from-storage.md`): roles, agent rules, preflight, the loop for each milestone, the stop rule (by reference to decision 0024), who edits what, stop-and-wait conditions, close-out with `xtask clean`, and the report format.  M7 adds the PR body steps, because M7 defines the body.
* `PLANS.md` gets an optional section "How the work runs" with fixed sub-headings: mode, branch and PR, maintainer steps, preflight items of the plan, decision points asked at the start, review rule, kick-off prompt, extra stop conditions.  A big task's plan must have it.
* `docs-check` fails a plan in `plans/active/` with more than 3 `M<n>` milestone headings and no "How the work runs" heading.  It accepts that heading at level 2 or 3.  It does not check `plans/completed/`, because four completed plans have more than 3 milestones and no such section, and PLANS.md forbids their edit.
* `working-with-agents.md` names the `orchestrate` skill.
* The sizing rule: `CLAUDE.md`'s "Plans" line says when a task is big.  The skill's description names the same signals, so the model loads it.  The agent proposes the mode, asks one question, then runs hands-off.  No hook starts it, because a hook cannot start a skill ([hooks](https://code.claude.com/docs/en/hooks)).
* The team agents stay `general-purpose` with briefs.  An agent file with write tools would fail `agents_folder_problems`, which stays as it is.
* Acceptance: a pinning test like `the_review_skill_starts_with_xtask` for the skill's key lines (no fork, the subagent cap, signed commits, the stop rule reference, the clean step).  A `docs-check` test fails an active plan of 4 milestones with no "How the work runs" section, and passes a completed one.  A dry run: a new session gets a plan of 4 milestones and the prompt "do this plan"; it proposes orchestrated mode in its first reply (transcript excerpt in Surprises).

### M7: the PR body and attention comments (decision 0027)

* `.github/pull_request_template.md`:
  * A summary for people, at most 150 words: what changed and why.
  * "Needs your attention": each item links to its inline comment.
  * "Not verified": what no automated check covers, such as Safari, a real GPU or an iPhone.
  * Verification in 2 or 3 lines.
  * Then `<details><summary>For agents</summary>` with milestones, commands and results, review rounds, the "reported, not fixed" findings, rule and debt IDs, and the plan and decision paths.
* `xtask pr-body <file>` checks a body: the headings, the summary's word limit, and a `<details>` block.  The orchestrator runs it before each `gh pr edit --body-file`.
* The orchestrate skill gets the PR body steps, and its pinning test gets their key lines.  `working-with-agents.md` names `pr-body` and `attention`.
* Attention comments: at close-out, after the last fix, the orchestrator posts one review with event `COMMENT` and inline comments (decision 0027).  It uses the GitHub MCP server's pending review or `gh api`.  Only these get a comment:
  * a proposed decision, or a change to a rule, `.claude/`, `CLAUDE.md`, `invariants.md`, CI or an allowlisted skip;
  * a trust boundary or correctness hot spot: PROTO-2 and the `model_url` path, the page-worker messages, `unsafe`, a new dependency, MEM and RT-1 paths;
  * an agent's judgement call, from the Decision logs entries marked "agent".
* At most 10 comments.  Each says what to decide and the default the agent took.  `xtask attention` lists the candidate lines from the diff and the Decision logs (a pure function with unit tests); the orchestrator picks from that list.
* First step of M7: confirm on this PR that the author's account can post a `COMMENT` review with line comments on its own PR.  GitHub's docs say only that an author cannot approve its own PR ([approving](https://docs.github.com/en/pull-requests/how-tos/review-pull-requests/approving-a-pull-request-with-required-reviews)).  If it cannot, use plain PR comments that link to the lines, and record that in Surprises.
* Acceptance: `xtask pr-body` tests (a good body passes; a long summary, a missing heading or no `<details>` fails).  `xtask attention` tests (a diff of each class gives its line; a docs-only diff gives none).  This PR's own body and comments use the format.

### M8: close-out

* `harness.md`: a row for each new component, with the failure it addresses.  `working-with-agents.md`: recipes and a changelog line.  `tech-debt.md`: each gap left.
* `harness-engineering.md`: re-check phases 4 to 6 and the phase 3 gate against M3 and M5.  Propose (do not decide) whether the `--baseline` gate is replaced by M5's measurement (D5).
* Re-check D4 against the late bugs outside the fix range of the round before (catch-up 4-1, M7 6-1, M7 6-2).  Propose a change if the narrow range would miss such bugs.
* Run `xtask clean --yes`.  Move this plan to `completed/` with its retrospective.

## Validation and acceptance

* Each milestone's acceptance above, run by the orchestrator through its team.
* `cargo run -p xtask -- docs-check` and the full GIT-2 set pass on the last commit, and CI is green.
* The next big task after the merge measures this phase: no maintainer message is needed to start orchestrated mode, the round count after round 3 is lower, the cost per milestone is lower than PR #39's, and nothing is left for the maintainer to clean.  Phase 6 compares those numbers.

## Idempotence and recovery

Each milestone is a set of commits that `git revert` undoes on its own.  `xtask clean` is safe to run again: its list mode changes nothing.  The measurement of M5 changes nothing in the repo until its last commit.  A new team leader takes over from the milestone plan and its Progress.

## How the work runs

This plan uses the prose recipe until M6 makes the skill.  It follows `plans/completed/stream-from-storage.md`, "How the work runs (M3 to M10)", with these changes.

* Mode: orchestrated, one branch `harness-phase-3b` from `main`, one draft PR.
* Milestone plans: `plans/active/harness-phase-3b-m<n>.md`, one for each milestone, written by its team leader as `PLANS.md` says.  Only the orchestrator edits this plan.
* Maintainer steps before the start, on the Mac:
  * A GPG or SSH signing cache that lasts the run; `caffeinate -dims`; `autoContinueAtUsageLimit` on.
  * A scoped `autoMode.allow` entry in `~/.claude/settings.json` for writes to `.claude/skills/`, `.claude/agents/` and `.claude/settings.json`, and for `gh pr ready`, as phase 3 needed.  Project settings cannot hold it.
  * Start `claude --permission-mode auto` at the repo root and paste the kick-off prompt.
* Preflight (stop at once on a failure): clean tree; `gh auth status`; a signed test object (`git commit-tree -S HEAD^{tree} -p HEAD -m preflight`); `cargo mutants --version`; at least 60 GiB free.
* Also fetch PRs #35, #37 and #39 into named refs, so a later fetch does not lose them: `git fetch origin 'refs/pull/<n>/head:refs/remotes/origin/pr/<n>'`.
* Decision points, asked once at the start: D1 to D8 below.
* Plan review: the cloud session that wrote this plan ran no review round on it.  Before M1, new subagents review this plan and decisions 0023 to 0027 against the retrospective's sources, to the stop rule, at most 3 rounds.
* Review rule: M1 and M2 by decision 0011 as amended in practice (to the stop rule).  From M3, decision 0024 as accepted.
* Only the orchestrator edits `CLAUDE.md` and `.claude/**`, and writes review files that the Write tool refuses to a subagent.
* Push only between review rounds.  Wait for CI before the next team.
* Stop and wait: an invariant would break; auto mode refuses; a step needs a person; the M5 budget is spent; the weekly usage limit is near; the D8 timebox ends.

Decision points (the maintainer answers these at the start):

* D1: accept decisions 0023 to 0027 as written, or with changes.
* D2: a commit that touches a strict rule's paths runs tier `push` (recommended), or tier `commit` like any other.
* D3: the list of strict rules.  Proposed: PROTO-1, PROTO-2, MEM-1 to MEM-5, RT-1, GPU-1 to GPU-3, VERIFY-1, GIT-1, DEPS-1, ASSET-1.
* D4: from round 4, reviewers read the fix range only (recommended; 7 of 10 late bugs were in fix churn), or the whole branch.
* D5: the phase 3 `--baseline` gate stays, or M8 proposes its replacement by M5's measurement.
* D6: the maintainer turns off Copilot review on each push in the `main` ruleset, or requests it once when a PR is ready.  A person changes the ruleset; agents do not.
* D7: an `autoMode.allow` entry for `xtask clean --yes`, or the maintainer runs it by hand.
* D8: a timebox for the whole run.

Kick-off prompt:

> Run docs/plans/active/harness-phase-3b.md as its section "How the work runs" says, on a new branch harness-phase-3b from main, with one draft PR.  Do the preflight first.  Then ask me D1 to D8 once, and run M1 to M8 in order as orchestrator.  Stop only for the reasons that section names.

## Progress

* [x] 2026-10-30: retrospective research (six agents: the PR #39 sessions, the earlier sessions, the review logs, PRs and CI, Claude Code and GitHub features, a harness inventory).
* [x] 2026-10-30: this plan and decisions 0023 to 0027 written as proposed.
* [x] 2026-10-31: preflight passed; D1 to D8 answered (Decision log); decisions 0023 to 0027 accepted.
* [x] 2026-10-31, plan review: at most 3 rounds on this plan and decisions 0023 to 0027.  Round 1 (two reviewers: the code, the evidence) found 4 blockers and 16 should-fix; all fixed in the plan.  Round 2 (one reviewer) found no blocker and 6 should-fix; all fixed.  Round 3 found no blocker and 4 should-fix; all fixed.  The review stops at its cap of 3 rounds.
* [ ] M1: cleanup.
* [ ] M2: less mutants noise.
* [ ] M3: the fix rule and the stop rule.
* [ ] M4: local checks by tier.
* [ ] M5: a model and effort for each agent role.
* [ ] M6: orchestrated mode as a skill.
* [ ] M7: the PR body and attention comments.
* [ ] M8: close-out.

## Surprises and discoveries

* 2026-10-30: GitHub keeps `refs/pull/<n>/head` for PRs #30 to #39 after their branches are deleted, so the commits of PR #39's review rounds can still be fetched.  Evidence: `git ls-remote origin refs/pull/39/head` gives `4068c49`.
* 2026-10-30: the cloud session that wrote this plan could not push (HTTP 403 from GitHub), so the plan reached the maintainer by another route.
* 2026-10-31: `cargo mutants` was not installed on the Mac, so the preflight failed on it.  The orchestrator installed cargo-mutants 27.1.0 with `cargo install --locked`.

## Decision log

* 2026-10-30, maintainer: big tasks get orchestrated mode, found by a sizing rule and confirmed by one question.
* 2026-10-30, maintainer: rounds 1 to 3 fix every confirmed finding, nits included; later rounds fix only blockers, significant should-fix and strict-rule findings, and report the rest; no cap on rounds.
* 2026-10-30, maintainer: a cheaper model for a role only where a measurement on past findings shows no loss; the measurement costs about one milestone's review rounds.
* 2026-10-30, maintainer: local checks run by changed paths; the full set before a push and before a review round.
* 2026-10-30, maintainer: inline comments only for proposed decisions and rule changes, security and correctness hot spots, and agents' judgement calls.
* 2026-10-30, maintainer: this phase comes before phase 4.  The work runs on the maintainer's Mac, not in a cloud session, because the checks need the local toolchain and browsers.
* 2026-10-31, maintainer, D1: decisions 0023 to 0027 are accepted as written.
* 2026-10-31, maintainer, D2: a commit that touches a strict rule's paths runs tier `push`.
* 2026-10-31, maintainer, D3: the strict rules are PROTO-1, PROTO-2, MEM-1 to MEM-5, RT-1, GPU-1 to GPU-3, VERIFY-1, GIT-1, DEPS-1 and ASSET-1.
* 2026-10-31, maintainer, D4: from round 4, reviewers read the fix range and the files it touches only.
* 2026-10-31, maintainer, D5: M8 proposes whether M5's measurement replaces the `--baseline` gate.  M8 does not decide it.
* 2026-10-31, maintainer, D6: the ruleset is the maintainer's change.  Agents do not touch it.
* 2026-10-31, maintainer, D7: `cargo run -p xtask -- clean --yes` has an `autoMode.allow` entry.
* 2026-10-31, maintainer, D8: the timebox is 48 hours from 2026-10-31.  The run stops at a clean point when it ends.
* 2026-10-31, maintainer: the orchestrator may push `harness-phase-3b`, open and edit one draft PR, and post the COMMENT review of M7 without a further question.  It never merges.
* 2026-10-31, maintainer: the plan commit is re-authored to the maintainer and signed, without the AI trailers of the patch.
* Reason for D1 to D8: the maintainer's answers at the start of the run.  D2 and D4 took the plan's recommendation.
* 2026-10-31, agent: plan review round 1 changed the plan.  M2 has a new baseline, because `.cargo/mutants.toml` already leaves out the test and wasm32-only files.  Other fixes: M5's budget unit and pass rule; M6's check scope; M4's strict-rule path table.
* 2026-10-31, agent: in decisions 0023 to 0026, the plan review corrected numbers and stale "if the maintainer accepts" lines.  These clarifications are open for the maintainer:
  * 0024: a nit is never `confirmed`, so "nits included" now means "each nit that the team leader agrees with".
  * 0023: the `docs-check` rule checks only `plans/active/`.
  * 0026, from round 2: tier `commit` reads the commit's own changes, not the branch's diff against `main`.  On a long branch, the branch's diff selects almost every check, so the decision would save no time.
  * 0026, from round 3: tier `push` runs the fmt, clippy, test, `docs-check` and Trunk build lines of the Commands, not `serve`, `web-test`, `e2e` or `publish`.  `publish` is for a person only (GIT-1).
  * None of these changes what the maintainer accepted.  The PR lists them under "Needs your attention".
* 2026-10-31, agent: the M1 team proposed four changes to the M1 section, and the orchestrator applied them (`cfaf8fd`).  Reason: the team found that `git worktree prune` is not needed, and that three cases need a stricter rule.
* 2026-10-31, agent: commits `b7e1088` and `849ec8e` describe the skill's `review clean` step and the guard before `d0164ea` adds them (DOC-1, review finding 1-11 of M1).  History is not rewritten; the PR body notes it.
* 2026-10-31, agent: plan review round 2 cut M5's design to fit the 45M budget.  The validator goes first, then the docs-reviewer on 4 rounds; `haiku` and the effort arms run only if budget is left.  A design of all the arms costs over 100M.

## Outcomes and retrospective

To be written when the plan completes.
