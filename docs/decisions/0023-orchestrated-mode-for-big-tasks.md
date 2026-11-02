# 0023: a big task runs in orchestrated mode, after one question

Status: accepted (2026-10-31, by the maintainer, as D1 of `plans/active/harness-phase-3b.md`; proposed by the agent on 2026-10-30)

Amended on 2026-11-01 by the agent, after M5 of `plans/active/harness-phase-3b.md`: decision 0025's measurement found no cheaper model that passes, so each role keeps `inherit`.  So decision 0025 lowers no cost yet.  A lower cost of the mode may come from decisions 0024 and 0026.  The next big task after the merge measures it, and phase 6 compares the numbers.  The decision is unchanged.

Amended on 2026-11-01 by the maintainer's instruction: "for big tasks, what we agreed on is to have it ask important questions up front about design and implementation that hinge on my decisions".  So the one question of this decision is one message up front, before any work.  The message holds each design or implementation choice that hinges on the maintainer, each with its options, a recommendation and the reason, with the mode, the branch and the PR.  A big task's plan lists these choices in a section "Decisions for the maintainer" (`plans/PLANS.md`), and the session that writes the plan asks them before it calls the plan done.  A decision during the run stays the agent's, in the Decision log and the PR.

## Context

PR #34 and PR #39 ran hands-off: one orchestrator session, one team for each milestone, one PR.  The mode lives only as a "How the work runs" section that a person copies into each plan.  Harness phase 2 had no such section: 21 milestones took about 17 sessions, 18 PRs and at least 54 maintainer messages.  PR #39's planning session started the work inline and opened extra branches, until the maintainer stepped in three times.

A hook cannot start a skill ([hooks](https://code.claude.com/docs/en/hooks)).  The model loads a skill when the task matches its description ([skills](https://code.claude.com/docs/en/skills)).

## Options

* A: a skill that only a person starts, by name.
* B: a sizing rule in `CLAUDE.md` and in the skill's description; the agent proposes the mode and asks one question.
* C: the same rule, with no question.

## Decision

B.  A task is big when one of these holds at the start: a plan of more than 3 milestones; a decision point in more than one milestone; work longer than one session; or the maintainer says the run is to go on without them.  The agent proposes orchestrated mode in one question, then runs hands-off.

* `.claude/skills/orchestrate/SKILL.md` holds the generic recipe.  A plan's "How the work runs" section holds only what is particular to it.
* `docs-check` fails a plan in `plans/active/` of more than 3 milestones with no "How the work runs" section.  Completed plans are not checked.
* The team agents stay `general-purpose` with briefs.  No agent file gets write tools.

## Consequences

* A person types the kick-off once and answers the plan's decision points once.
* The mode costs more tokens than one session.  Decision 0025 lowers the cost for each role.
* Edits to `.claude/**` still need the maintainer's `autoMode.allow` entry in their own settings.
