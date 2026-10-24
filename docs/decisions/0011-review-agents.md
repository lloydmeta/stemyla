# 0011: review agents, routed by xtask review and run by a skill

Status: accepted (2026-10-22, by the maintainer)

## Context

Before phase 3, the agent wrote the prompt of each reviewer by hand (`plans/active/harness-engineering.md`, failure 5).  One general reviewer read every diff.  In 14 of 18 PRs, the reviewers of phase 2 found a test that could pass on wrong code.

Phase 3 (`plans/active/harness-phase-3.md`) is one PR.  The maintainer chose D1 to D6 of that plan on 2026-10-22, before the work started.

## Options

For each part, the plan of phase 3 records an option that was not taken:

* The log (D2): a table in the PR description.
* cargo-mutants (D3): a CI job, blocking or not.
* The rounds (D6): every specialist in every round.
* The replay set (D1): PRs #9 to #12, with the code before the review of #10, #11 and #12 rebuilt from session transcripts.
* The gate's effort (D4): a flat 5 times the baseline, or no ceiling.
* A missed bar (D5): tuning on the measured cases.

## Decision

The acceptance of the maintainer covers D1 to D6 and the plan that the maintainer started.  It does not cover each later choice of an agent.

* Routing and round guards in `xtask review`.  `start` refuses a tree with changes, routes six reviewers by paths and content signs, and records the state.  `check` stops a round on a change to that state.  `mutants` runs cargo-mutants under a limit of 60 minutes.
* A `validator` for each blocker and should-fix, and for the missed mutants.  The skill logs nits without a validator.
* Read-only agents: the tools are `Read` and `Bash`.  The guard `.claude/hooks/read-only-bash.sh` runs as a `PreToolUse` hook with `onFailure: block`.  The agents have no `isolation`, since a worktree branches from the default branch.
* The log (D2): the reviews folder under `docs`, one file for each branch, written after round 3.
* cargo-mutants (D3): in the skill only, not in CI.
* The rounds (D6): three rounds, as `CLAUDE.md` says.
* The replay set (D1) and its miss (D5).  M5 found 8 of 26 (0.31; 95% interval 0.17 to 0.50), below the bar of 0.8.  By D5, `reviews/replay.md` records the result, and the gate decides after the merge.
* The gate's effort (D4): a ceiling on tokens, wall time and interventions.  On 2026-10-24 the orchestrator narrowed D4.  An approval of a tool call is cost, logged apart, not an intervention.  This narrowing is not part of the acceptance of the maintainer of 2026-10-22, and the summary to the maintainer lists it.  The token figure of the log is the final context of each agent, not the tokens that it used.  So D4's token ratio has no reliable measure in the log (TD-44).  The maintainer decides how to measure it at the gate.

## Consequences

* A reviewer is a sensor, never a control.  No `* Enforced by:` line in `invariants.md` names an agent.  The tech-debt entries that review covers name the agent that looks.
* Known gaps:
  * TD-31: the route is a short denylist, from the branch's own code.
  * TD-32: `review check` sees only what its record holds.
  * TD-33: `review mutants` tests only part of a diff.
  * TD-34: the guard matches text.
  * TD-35: a review runs the diff's code with the person's credentials.
  * TD-36: in the default permission mode, the skill forces an approval in each round.
  * TD-37: a branch has one set of rounds.
  * TD-38: the log cannot count a person's approvals.
  * TD-44: the log's tokens are final contexts.
  * TD-46: `dontAsk` mode denies the probe forms of the agents.
  * TD-47: a `-p` session ends while a background agent runs.
* Phase 5 passes the agents with `--agents`, since frontmatter hooks do not run in `-p` mode or in a folder that is not trusted.  Phase 5 routes with the `xtask` of the base branch (TD-31).
* For IAM: a security reviewer is an extra sensor.  Measure its hit rate as for any test (the plan of phases).
