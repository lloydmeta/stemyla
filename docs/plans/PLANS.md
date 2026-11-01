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

## Optional sections

A plan has these only when it needs them.

* "How the work runs": how a big task runs in orchestrated mode (decision 0023).  The `orchestrate` skill holds the generic recipe.  The section holds only what is particular to the plan.
  * A big task's plan must have it.  A task is big by any of the four signals of decision 0023: more than 3 milestones, a decision point in more than one milestone, work longer than one session, or the maintainer says that the run goes on without them.  `docs-check` checks only one signal: it fails a plan in `active/` of more than 3 milestones without the section.
  * The section heading is at level 2 or 3, and can have more text after it, such as `### How the work runs (M3 to M10)`.
  * Its sub-headings, in this order, one level below the section:
    * Mode: orchestrated or not, and the teams that do the milestones.
    * Branch and PR: the branch, its base, and the draft PR.
    * Maintainer steps: what a person does before the start, such as the settings that agents cannot change.
    * Preflight: the checks before the start, which stop the run at once on a failure.
    * Decision points: the questions for the maintainer, asked once at the start.
    * Review rule: the decision that the review rounds of each milestone follow.
    * Kick-off prompt: the text that a person pastes to start the run.
    * Extra stop conditions: the stops of this plan that the skill does not name.
