# 0002: agents work in the background, up to merging their own PRs, reached in stages

Status: accepted (2026-10-19, by the maintainer)

## Context

The maintainer leads an identity and access management team and uses this repo to learn how AI development should work before applying it to security-sensitive code.  Stemyla is low-stakes: a static page with no accounts, no server and no secrets beyond CI's own.  So it can go further than a security codebase should, to find where autonomy breaks.

Published practice ranges from humans reviewing every agent PR to agents merging their own ([OpenAI, harness engineering](https://openai.com/index/harness-engineering/)).  Every agent incident of 2025-2026 surveyed for the plan (Nx "s1ngularity", PromptPwnd, "Comment and Control", CVE-2026-47751 in claude-code-action) came from an agent reading attacker-controlled text while holding credentials, and vendor fixes patched payloads rather than the design ([Willison, the lethal trifecta](https://simonwillison.net/2025/Jun/16/the-lethal-trifecta/)).

## Options

* Staged and risk-tiered: background PRs only for low-risk change classes, humans for the rest.
* Stop at long-running local sessions with every PR reviewed by a human.
* Full background autonomy here, up to agents merging their own PRs, to learn its limits.

## Decision

Full background autonomy is the target in this repo, reached in stages (`plans/active/harness-engineering.md`, phase 5).  Each stage starts only when the deterministic checks for it exist and its gate is met; no LLM verdict alone ever merges a change.

## Consequences

* The identity that approves or merges an agent's PR must differ from the one that wrote it; the `main` ruleset needs one approval and the maintainer is the only person, so this needs a deliberate design before auto-merge (phase 5's gate).
* Agents that read untrusted text (issues, PR comments, web pages) never hold write credentials in the same job.
* What is learned is recorded for the IAM codebase, where the stage that lets agents merge is expected not to apply to authentication or authorisation paths.
