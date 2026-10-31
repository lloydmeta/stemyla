# 0024: what a review round fixes, and when the rounds stop

Status: accepted (2026-10-31, by the maintainer, as D1 of `plans/active/harness-phase-3b.md`; proposed by the agent on 2026-10-30; amends D6 of decision 0011)

## Context

Decision 0011 says three rounds.  During PR #39 the maintainer changed that: rounds go on past round 3 until a round has no blocker and no significant should-fix.  That rule lives only in a completed plan.

In the 14 review logs, rounds 4 and later fixed 124 findings.  13 were real bugs (10%) and 71 were nits.  At least 3 of the 13 were in files that the fix range of the round before did not touch.

## Options

* A: three rounds, as decision 0011 says.
* B: rounds to the stop rule; every confirmed finding fixed in each round.
* C: rounds to the stop rule; every confirmed finding fixed in rounds 1 to 3; from round 4, only blockers, significant should-fix and strict-rule findings.

## Decision

C, as the maintainer asked on 2026-10-30.

* Rounds 1 to 3 fix every confirmed finding, and each nit that the team leader agrees with.  A nit stays `unvalidated`.
* From round 4, a round fixes blockers, significant should-fix and every confirmed finding on a strict rule.  It logs the rest as "reported, not fixed", and the PR's `<details>` section lists them.
* The rounds stop at the first round from round 3 on with none of those three.  There is no cap.
* The team leader or the orchestrator judges significance, not the reviewer.  Significant: a rule or decision fails in a case a user can reach, a contradiction, a misleading measure, or later work goes wrong.  Nits: an edge case no real song reaches, wording, a weak test of a minor path.
* Strict rules (decision point D3 of the plan): PROTO-1, PROTO-2, MEM-1 to MEM-5, RT-1, GPU-1 to GPU-3, VERIFY-1, GIT-1, DEPS-1, ASSET-1.
* From round 4, reviewers read the fix range and the files it touches (D4 of the plan).

## Consequences

* Fewer late fixes, so less fix churn for the next round to review.
* A nit found after round 3 stays in the code.  The PR lists it.
* `CLAUDE.md`, the `review-rounds` skill, `working-with-agents.md` and decision 0011 change in the same commit (DOC-1).
