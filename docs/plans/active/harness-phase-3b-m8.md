# Harness phase 3b, M8: close-out

## Purpose

This is the milestone plan for M8 of `plans/active/harness-phase-3b.md`.  The parent's M8 section stays the criteria.  M8 records what the phase built and measured, and lists the gaps that it leaves.  It proposes changes to the later phases for the maintainer, and decides none of them.

Readers: the review agents of M8's rounds, the orchestrator, and the maintainer.

## Context and orientation

The state at the start, on 2026-11-02, at commit `48667e8` on branch `harness-phase-3b`, with M7 done.  The review base of M8 is `48667e8`.  The range from it also holds the orchestrator's commits of M7's intent review: the code fixes `bc52f6c`, `3a48544` and `c719c58`, the pick order of the `orchestrate` skill in `cd96fc5`, and plan text in `8a16005` and `951bc2f`.  Round 1 reviews them too.

* `docs/harness.md` has no row for a part of phase 3b.
* `docs/working-with-agents.md` has no changelog line for M5, and its `docs-check` row does not name "Decisions for the maintainer" (M6 finding 4-5).
* `docs/tech-debt.md` has TD-67 to TD-72 from this phase.  M7 added no TD.
* `plans/active/harness-engineering.md` still says that a big task asks one question.
* The parent plan stays in `plans/active/` until the last commit, since the orchestrator still writes in it.  So the commits of this milestone name `plans/active/harness-phase-3b.md`.  The orchestrator's last commit moves the plan and fixes those paths.

Terms:

* Late round: a review round from round 4 on.
* Fix range: the commits made since the previous round started.

## Plan of work

### Step 1: the docs (one commit)

* `docs/harness.md`: one row for each part of phase 3b, with the failure that it addresses.  The rows include `xtask clean`, `review clean`, the known-missed list with its yield, the fix rule with `review log-check`, the late range, `xtask check`, the pin `the_agent_models_are_the_measured_ones`, the `orchestrate` skill, the plan rules of `docs-check`, `pr-body` and `attention`.  A section "Phase 3b, measured" gives the numbers.
* `docs/working-with-agents.md`: the Status line of phase 3b, a pause and resume line in the recipe "Run a big task hands-off", the model line of "Review agents", the `docs-check` row, and the changelog lines of M5 and M8.
* `docs/tech-debt.md`: TD-73 (the late range), TD-74 (a team leader cannot write the review files or stop its subagents) and TD-75 (the phase 3 gate compares one run of each review).  Round 1 adds TD-76 to TD-78.  TD-76: the hand-run class of `xtask attention` goes by name.  TD-77: `xtask pr-body` checks the form only.  TD-78: the skill has no pause or resume step.  TD-44 also names the orchestrator's tokens.
* `docs/README.md`: this plan's line, and the `harness.md` line names the numbers of phase 3b.

### Step 2: the proposals (one commit)

`plans/active/harness-engineering.md`, section "Proposals of harness phase 3b", P1 to P7: the phase 3 gate and D5, the re-run of decision 0025 as an amendment for the maintainer, D4 against the late bugs outside the fix range, a measure in place of "fewer review rounds per PR", and phases 4 to 6.  The phase 3b paragraph, the Progress lines and two Surprises lines change too.

### Step 3: the text for the orchestrator

The retrospective of the parent plan, its `git mv` to `plans/completed/` and the path fixes, as one diff in the scratchpad folder `m8/`.  The PR text of M8 (`m8/pr-body-m8.md`): the proposals for "Needs your attention", and the maintainer's steps after the merge.

## Validation and acceptance

* `cargo run -p xtask -- check --tier commit` passes before each commit.  `docs-check` passes on each commit, with each new path, TD ID and test name.
* `git apply --check` passes on `HEAD` for the diff of step 3.
* The review rounds of M8 follow decision 0024, from base `48667e8`.

## Idempotence and recovery

Each step is one commit that `git revert` undoes.  No step changes code.

## Progress

* [x] 2026-11-02: this plan.  No `Plan` agent reviewed it, for time (Decision log).
* [x] 2026-11-02: Step 1: the docs (`harness.md`, the guide, TD-73 to TD-75 and TD-44, the index).
* [x] 2026-11-02: Step 2: the proposals P1 to P7 in `plans/active/harness-engineering.md`.
* [x] 2026-11-02: Step 3: the text for the orchestrator, in the scratchpad folder `m8/`: `final.diff` (the retrospective, the `git mv` of the parent plan and its path fixes; `docs-check` passes on it in a clone) and `pr-body-m8.md`.
* [x] 2026-11-02: review round 1 of `48667e8..ab10c34` (6 reviewers and 2 validators; 474,123 tokens; 11.4 wall minutes; 0 interventions).  2 confirmed should-fix (1-1 significant: the guide gave a pause and resume that the skill does not have; 1-2: the gaps of M7's sensors had no TD) and 5 nits.  Mutants: 14 tested, 14 caught, 0 missed.  The fixes are in "review round 1 fixes".  The skill sentence of 1-4 and its pin are a diff for the orchestrator (`m8/r1-claude.diff`).
* [x] 2026-11-02: review round 2 of `48667e8..e88f11f` (6 reviewers; 374,998 tokens; 8.6 wall minutes; 0 interventions).  No blocker or should-fix, and 7 nits, each fixed in "review round 2 fixes".  Mutants: none in the since diff.
* [x] 2026-11-02: review round 3 of `48667e8..8acc3f1` (2 reviewers and 2 validators; 189,504 tokens; 7.4 wall minutes; 0 interventions).  3-1 confirmed (a refused worktree with no TD, on VERIFY-1): TD-50 now records it.  3-2 pre-existing and significant (step 6 of the skill edits the plan after the comments): a diff for the orchestrator (`m8/r3-claude.diff`).  4 nits, fixed.  Mutants: none in the since diff.  3-1 is on a strict rule, so round 4 follows.
* [ ] Review rounds 4 and later.

## Surprises and discoveries

* 2026-11-02: M7's late rounds found 3 bugs (4-1, 4-7, 5-2), each in `xtask/src/attention.rs`, which the fix range touched.  The drafts of phase 1 had said that the late rounds of this phase found no bug.  The figures now cover M3 to M7.
* 2026-11-02: in review round 2, auto mode refused the supply-chain-reviewer's `git worktree add` ("Security Weaken").  The reviewer did not try another form, made no worktree, and reasoned its mutant without a run.  TD-50 records the gap.  The other reviewers of rounds 1 and 2 made their worktrees with no refusal.

## Decision log

* 2026-11-02, agent (team leader): no `Plan` agent reviews this plan.  Reason: the orchestrator asked to skip the review for the 08:00 end, and the plan is short and changes no code.
* 2026-11-02, agent (team leader): the commits of M8 name `plans/active/harness-phase-3b.md`.  The orchestrator's last commit moves the plan, and fixes the paths in `docs/harness.md`, `docs/working-with-agents.md`, `docs/tech-debt.md`, `docs/README.md` and `plans/active/harness-engineering.md`.  Reason: `docs-check` fails a path that does not exist in those docs.
* 2026-11-02, agent (team leader): the figures on the rounds after round 3 of PR #39 come from an approximate count of the Action cells in 7 logs.  The orchestrator accepted them as approximate.

## Outcomes and retrospective

To be written when the milestone completes.
