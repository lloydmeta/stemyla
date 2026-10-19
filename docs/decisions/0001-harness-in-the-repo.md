# 0001: agent knowledge lives in the repo; the harness is Claude Code only

Status: accepted (2026-10-19, by the maintainer)

## Context

The repo was built by Claude Code with little planning.  Its knowledge lived in a 36-line `CLAUDE.md`, a long `ARCHITECTURE.md`, the maintainer's own Claude Code settings and memory, and plans kept outside the repo in the maintainer's Claude Code folder.  Nothing outside the repo is visible to another machine, a cloud session or a GitHub Actions run, and nothing checked the docs against the code.  `decisions/0002-staged-background-autonomy.md` needs background agents, so they must find everything in the repo.

## Options

* Knowledge in the repo, versioned, with `CLAUDE.md` as a short map into `docs/`.
* Durable knowledge in the repo; plans in progress outside it.
* Everything outside the repo (memory, plans, skills in the maintainer's folder).

And separately: a tool-neutral core (`AGENTS.md`) with Claude Code layered on, or Claude Code only.

## Decision

Knowledge in the repo, versioned: architecture, rules with IDs, decisions, execution plans (active and completed), tech debt and references, under `docs/`, with `CLAUDE.md` as the map and `.claude/` (rules, and later settings, agents and skills) checked in.  This replaces the earlier rule that plans are never committed.  The harness targets Claude Code only.

## Consequences

* A plan or decision is reviewed like code, and visible to every session and agent.
* `.claude/` must be tracked: the maintainer's global gitignore ignores it, so the repo's `.gitignore` re-includes it.
* Docs can drift from code, so `xtask docs-check` runs as a test.
* Process rules that background agents must follow (signed commits, review rounds) are copied into `CLAUDE.md`, with a writing standard (ASD-STE100).  The maintainer's personal style preferences stay in the maintainer's own settings.
* Using another agent tool later means adding an `AGENTS.md`; nothing here prevents it.
