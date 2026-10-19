# Execution plans

How to write and keep a plan for work of more than a few steps.  Adapted from OpenAI's ExecPlans ([Using PLANS.md for multi-hour problem solving](https://developers.openai.com/cookbook/articles/codex_exec_plans)) and the long-running harness posts from Anthropic ([effective harnesses for long-running agents](https://www.anthropic.com/engineering/effective-harnesses-for-long-running-agents)).

A plan is written for a reader with no other context: a new session, a background agent, or the maintainer months later.  It lives in `active/` while the work runs and moves to `completed/` when it ends, after which it is not edited.

## Rules

* Self-contained: state what is needed to do the work, define terms, and name files, functions and commands.  Link to `invariants.md` by rule ID rather than restating rules.
* Acceptance is observable: each milestone says which command shows it works, and what output proves it (a test that fails before the change and passes after it).  "It compiles" is not acceptance (VERIFY-1).
* Milestones are small and each one is verifiable and committable on its own (GIT-2).
* The living sections are updated at every stopping point, with dates: before a session ends, before compaction, and when a milestone completes.  A session picking up a plan reads them first.
* A decision that outlives the plan goes in `decisions/` as well.  A gap left behind goes in `tech-debt.md`.
* Ask the maintainer only at the decision points the plan names, or when a rule would have to be broken.

## Sections

Every plan has these, in order.  `docs-check` fails a plan that lacks a living section.

* Purpose: what changes for whom, and why, in a few sentences.
* Context and orientation: the current state, with the files involved.
* Plan of work: the milestones, each with its acceptance check.
* Validation and acceptance: the commands that show the whole plan worked.
* Idempotence and recovery: how to resume or undo safely.
* `## Progress`: dated checkboxes, one per milestone or step, ticked as they land.  The only checklist in the plan.
* `## Surprises and discoveries`: what was unexpected, with evidence.
* `## Decision log`: each decision made while working, with its reason, date and who made it.
* `## Outcomes and retrospective`: filled in at the end: what was achieved, what was not, and what to do differently.
