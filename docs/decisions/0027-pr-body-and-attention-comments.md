# 0027: a PR body for people and agents, and comments where a person must look

Status: proposed (2026-10-30, by the agent, for `plans/active/harness-phase-3b.md`)

## Context

PR bodies grew to 15.5k characters (#39: 2,707 words, no headings).  The "Not verified" section of the early PRs is gone.  No PR has an inline comment, and Copilot's reviews say that its quota is used up.  GitHub renders `<details>` in a PR body ([collapsed sections](https://docs.github.com/en/get-started/writing-on-github/working-with-advanced-formatting/organizing-information-with-collapsed-sections)).

## Decision

* The body starts with a summary for people of at most 150 words, then "Needs your attention", "Not verified" and 2 or 3 lines of verification.  A `<details><summary>For agents</summary>` block holds milestones, commands and results, review rounds, reported findings, rule and debt IDs, and paths.  `.github/pull_request_template.md` holds the headings; `xtask pr-body` checks a body.
* At close-out, the orchestrator posts one `COMMENT` review with at most 10 inline comments, only on: a proposed decision or a change to a rule, `.claude/`, `CLAUDE.md`, `invariants.md`, CI or an allowlisted skip; a trust boundary or correctness hot spot (PROTO-2, page-worker messages, `unsafe`, a new dependency, MEM and RT-1 paths); an agent's judgement call.  Each comment says what to decide and the default the agent took.
* If the author's account cannot post a review on its own PR, plain PR comments link to the lines.

## Consequences

* A person reads the summary and the comments, not the whole log.
* Agents read the `<details>` block when they take over.
