# 0026: local checks run by tier and by changed paths

Status: accepted (2026-10-31, by the maintainer, as D1 of `plans/active/harness-phase-3b.md`; proposed by the agent on 2026-10-30; amends GIT-2)

## Context

GIT-2 runs fmt, six clippy runs, the workspace tests and `docs-check` before each commit.  In the period from 2026-10-19 to 2026-10-30, every CI failure was in the browser job; no local check would have caught one.  A PR #39 commit of comments and paths ran the full set for 16 minutes, against 7.5 minutes in CI.

## Options

* A: GIT-2 as it is.
* B: by tier: a commit runs the checks of its changed paths; a push and a review round run the full set.
* C: a commit runs fmt and the changed crate's tests; the full set runs only in CI.

## Decision

B, as the maintainer asked on 2026-10-30.

* `xtask check --tier commit` picks checks from the paths changed against the merge base with `origin/main`, and prints what it skips and why.
* `xtask check --tier push` runs the full set of `CLAUDE.md`'s Commands, before each push and each review round.
* If the maintainer accepts D2 of the plan, a commit that touches a strict rule's paths (decision 0024) runs tier `push`.
* CI stays the authority.  The browser tests stay in CI, except a scenario that CI failed.

## Consequences

* A commit can be red on a check that tier `commit` skips.  The push tier or CI finds it before review.
* GIT-2 in `invariants.md` and `CLAUDE.md` changes in the same commit (DOC-1).
