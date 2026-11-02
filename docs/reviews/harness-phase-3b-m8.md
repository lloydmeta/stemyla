# Review log

## Findings

| Date | Round | Commit | Finding | Reviewers | Severity | Significant | Kind | Rule | Where | Claim | Verdict | Repeat of | Action |
| - | - | - | - | - | - | - | - | - | - | - | - | - | - |
| 2026-11-02 | 1 | ab10c34 | 1-1 | reviewer, docs-reviewer | should-fix | yes | doc | DOC-1 | docs/working-with-agents.md:60 | The guide and P5 give a pause and resume that the orchestrate skill does not have | confirmed | none | 7028cdc review round 1 fixes |
| 2026-11-02 | 1 | ab10c34 | 1-2 | reviewer, docs-reviewer | should-fix | no | doc | none | docs/harness.md:58 | The harness.md rows of xtask attention and xtask pr-body name gaps that no TD records | confirmed | none | 7028cdc review round 1 fixes |
| 2026-11-02 | 1 | ab10c34 | 1-3 | docs-reviewer | nit | - | doc | none | docs/harness.md:89 | The PR #39 comparison leaves out its M8 log with no reason | unvalidated | none | 7028cdc review round 1 fixes |
| 2026-11-02 | 1 | ab10c34 | 1-4 | docs-reviewer | nit | - | rule | none | docs/plans/active/harness-engineering.md:144 | Some new sentences break the STE word limits, and an index line has a double colon | unvalidated | none | 7028cdc review round 1 fixes; e88f11f, the orchestrator's commit of m8/r1-claude.diff; no change: the orchestrator declined the two long lines of the parent plan, for the 08:00 end |
| 2026-11-02 | 1 | ab10c34 | 1-5 | supply-chain-reviewer | nit | - | weak-test | VERIFY-1 | xtask/src/attention.rs:1308 | No test row has the comma form for team leader, implementer or orchestrator | unvalidated | none | 7028cdc review round 1 fixes |
| 2026-11-02 | 1 | ab10c34 | 1-6 | reviewer | nit | - | doc | none | docs/harness.md:86 | Phase 3b, measured says M1 to M8, and its counts name no source | unvalidated | none | 7028cdc review round 1 fixes |
| 2026-11-02 | 1 | ab10c34 | 1-7 | reviewer | nit | - | doc | DOC-1 | docs/README.md:71 | The index line of decision 0023 still says one question | unvalidated | none | 7028cdc review round 1 fixes |
| 2026-11-02 | 2 | e88f11f | 2-1 | wire-reviewer, docs-reviewer, supply-chain-reviewer | nit | - | doc | none | docs/tech-debt.md:496 | TD-76 states the hand-run rule loosely, against M7's Decision log | unvalidated | none | 4743321 review round 2 fixes |
| 2026-11-02 | 2 | e88f11f | 2-2 | wire-reviewer, reviewer | nit | - | doc | PROTO-1 | docs/tech-debt.md:494 | TD-76 says it concerns PROTO-1, but PROTO-1's Gaps line does not list it | unvalidated | none | 4743321 review round 2 fixes |
| 2026-11-02 | 2 | e88f11f | 2-3 | docs-reviewer | nit | - | rule | none | docs/plans/active/harness-phase-3b-m8.md:30 | Four new or changed sentences are over 25 words | unvalidated | none | 4743321 review round 2 fixes |
| 2026-11-02 | 2 | e88f11f | 2-4 | docs-reviewer | nit | - | doc | none | docs/harness.md:88 | Phase 3b, measured says the orchestrator ran M1 to M8 | unvalidated | none | 4743321 review round 2 fixes |
| 2026-11-02 | 2 | e88f11f | 2-5 | reviewer | nit | - | doc | none | docs/harness.md:50 | The known-missed row is false for M3 round 1, which touched check in docs.rs | unvalidated | none | 4743321 review round 2 fixes |
| 2026-11-02 | 2 | e88f11f | 2-6 | reviewer | nit | - | doc | none | docs/tech-debt.md:470 | TD-73 counts 13 late bugs in PR #39's logs, but they come from the 14 older logs | unvalidated | none | 4743321 review round 2 fixes |
| 2026-11-02 | 2 | e88f11f | 2-7 | reviewer | nit | - | doc | none | xtask/src/attention.rs:21 | The module doc names the M7 plan's old judgement rule | unvalidated | none | 4743321 review round 2 fixes |
| 2026-11-02 | 3 | 8acc3f1 | 3-1 | docs-reviewer | should-fix | no | rule | VERIFY-1 | docs/plans/active/harness-phase-3b-m8.md:64 | A refused git worktree add left a reviewer's mutant with no run, and no TD records it | confirmed | none | b644e04 review round 3 fixes |
| 2026-11-02 | 3 | 8acc3f1 | 3-2 | reviewer | should-fix | yes | doc | none | .claude/skills/orchestrate/SKILL.md:177 | Close-out step 6 records a refused review in the plan after the comments, so their Decision log lines move | pre-existing | none | m8/r3-claude.diff (the orchestrator's commit) |
| 2026-11-02 | 3 | 8acc3f1 | 3-3 | docs-reviewer | nit | - | doc | none | docs/harness.md:89 | The approximate count is tied to the rounds, not to the fixes | unvalidated | none | b644e04 review round 3 fixes |
| 2026-11-02 | 3 | 8acc3f1 | 3-4 | docs-reviewer | nit | - | doc | none | docs/tech-debt.md:496 | "in any case" in TD-76 reads as "anyway" | unvalidated | none | b644e04 review round 3 fixes |
| 2026-11-02 | 3 | 8acc3f1 | 3-5 | docs-reviewer | nit | - | doc | none | docs/tech-debt.md:470 | A changed sentence of TD-73 has 30 words | unvalidated | none | b644e04 review round 3 fixes |
| 2026-11-02 | 3 | 8acc3f1 | 3-6 | docs-reviewer | nit | - | doc | none | docs/harness.md:50 | The known-missed claim reads as a contradiction, and names no rounds | unvalidated | none | b644e04 review round 3 fixes |
| 2026-11-02 | 4 | 65a5f26 | 4-1 | supply-chain-reviewer, docs-reviewer | nit | - | doc | none | .claude/skills/orchestrate/SKILL.md:177 | The new instruction of step 6 has 21 words | unvalidated | none | reported, not fixed |
| 2026-11-02 | 4 | 65a5f26 | 4-2 | supply-chain-reviewer, docs-reviewer | nit | - | doc | none | docs/tech-debt.md:379 | The new sentence of TD-50 has 27 words | unvalidated | none | reported, not fixed |
| 2026-11-02 | 4 | 65a5f26 | 4-3 | reviewer, wire-reviewer | nit | - | doc | none | .claude/skills/orchestrate/SKILL.md:177 | Step 6 names no section of the PR body | unvalidated | none | reported, not fixed |
| 2026-11-02 | 4 | 65a5f26 | 4-4 | reviewer, docs-reviewer, wire-reviewer | nit | - | doc | none | docs/tech-debt.md:373 | The heading and scope line of TD-50 name harness files only | unvalidated | none | reported, not fixed |
| 2026-11-02 | 4 | 65a5f26 | 4-5 | wire-reviewer | should-fix | no | other | none | docs/plans/active/harness-phase-3b-m8.md:43 | The step 3 diff no longer applied at HEAD after the round 3 fixes | confirmed | none | reported, not fixed |
| 2026-11-02 | 4 | 65a5f26 | 4-6 | wire-reviewer | nit | - | doc | none | xtask/src/attention.rs:504 | "in upper or lower case" is narrower than the code | unvalidated | none | reported, not fixed |

## Rounds

| Round | Agents | Tokens | Wall minutes | Interventions | Late review commands | Approvals | Mutants | Baseline agents | Baseline tokens | Baseline wall minutes | Baseline interventions | Baseline approvals | Baseline found |
| - | - | - | - | - | - | - | - | - | - | - | - | - | - |
| 1 | 8 | 474123 | 11.4 | 0 | 1 | 0 (auto mode; a fallback prompt is not counted) | mutants: full: tested 14 (caught 14, missed 0, unviable 0), timeouts 0, not tested 0, known 0 | - | - | - | - | - | - |
| 2 | 6 | 374998 | 8.6 | 0 | 1 | 0 (auto mode; a fallback prompt is not counted) | mutants: none: tested 0 (caught 0, missed 0, unviable 0), timeouts 0, not tested 0, known 0; no mutants in the diff | - | - | - | - | - | - |
| 3 | 4 | 189504 | 7.4 | 0 | 1 | 0 (auto mode; a fallback prompt is not counted) | mutants: none: tested 0 (caught 0, missed 0, unviable 0), timeouts 0, not tested 0, known 0; no mutants in the diff | - | - | - | - | - | - |
| 4 | 7 | 286544 | 5.5 | 0 | 1 | 0 (auto mode; a fallback prompt is not counted) | mutants: none: tested 0 (caught 0, missed 0, unviable 0), timeouts 0, not tested 0, known 0; no mutants in the diff | - | - | - | - | - | - |
