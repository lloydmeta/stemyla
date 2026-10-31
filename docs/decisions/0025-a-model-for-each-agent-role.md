# 0025: each agent role gets a model and an effort, chosen by a measurement

Status: proposed (2026-10-30, by the agent, for `plans/active/harness-phase-3b.md`)

## Context

Every file in `.claude/agents/` says `model: inherit`, and `agents_folder_problems` requires it.  So PR #39 ran every reviewer and validator on Opus 5.5 at high effort, and used about 80% of a week's usage limit.  A subagent can take `model` and `effort` in its frontmatter ([sub-agents](https://code.claude.com/docs/en/sub-agents)).  Sonnet 5.5 costs half of Opus 5.5 for each token, and Haiku 5.5 a fortieth up to 100k-token prompts ([pricing](https://platform.claude.com/docs/en/about-claude/pricing)).

In the logs, the validator refuted 6 of 284 reviewer findings, and the docs-reviewer's findings are 62% nits.  The reviewer and the memory-reviewer found 25 and 5 bugs that no other agent found.

## Options

* A: keep `inherit` everywhere.
* B: pick models by judgement.
* C: measure each candidate role against logged findings, and change only the roles that pass.

## Decision

C, as the maintainer asked on 2026-10-30.

* Candidates: docs-reviewer (`sonnet`), validator (`sonnet`, `haiku`), gpu-reviewer, wire-reviewer and supply-chain-reviewer (effort `medium`).
* Not candidates: reviewer, memory-reviewer, the intent reviewer, the orchestrator and baseline.
* Ground truth: the confirmed blockers and should-fix in the PR #39 logs, at `refs/pull/39/head`.
* Pass: recall within 0.15 of a control run on the current model, no confirmed bug missed, no confirmed bug refuted.  The rule is written before any run.
* Budget: about one milestone's three rounds of tokens.
* `agents_folder_problems` accepts `model` from `inherit`, `opus`, `sonnet` and `haiku`, and an `effort` key.

## Consequences

* A role that fails keeps `inherit`.
* The measurement is re-run when the model line-up changes (phase 6).
