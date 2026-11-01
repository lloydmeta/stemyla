# Review log

## Findings

| Date | Round | Commit | Finding | Reviewers | Severity | Significant | Kind | Rule | Where | Claim | Verdict | Repeat of | Action |
| - | - | - | - | - | - | - | - | - | - | - | - | - | - |
| 2026-11-01 | 1 | d077706 | 1-1 | reviewer | should-fix | yes | doc | none | docs/plans/active/harness-phase-3b-m5.md:407 | the recorded scripts read data files, a parser and an A/B rule that the plan does not record, and name the session scratchpad | confirmed | none | 686cfe1 review round 1 fixes |
| 2026-11-01 | 1 | d077706 | 1-2 | docs-reviewer | nit | - | doc | none | docs/plans/active/harness-phase-3b-m5.md:229 | many new sentences of the M5 plan are over 25 words | unvalidated | none | 686cfe1 review round 1 fixes |
| 2026-11-01 | 1 | d077706 | 1-3 | docs-reviewer | nit | - | doc | DOC-1 | docs/plans/active/harness-phase-3b.md:66 | the parent says agents_folder_problems requires `model: inherit` today; step 2 and Surprises disagree on which plans stay | unvalidated | none | 686cfe1 review round 1 fixes; the parent's line as a diff for the orchestrator |
| 2026-11-01 | 1 | d077706 | 1-4 | docs-reviewer | nit | - | doc | none | xtask/tests/rules.rs:1155 | the pin's failure text for a row with no effort is a double negative | unvalidated | none | 686cfe1 review round 1 fixes |
| 2026-11-01 | 1 | d077706 | 1-5 | docs-reviewer | nit | - | doc | none | xtask/src/rules.rs:3036 | one_of says two values or more, but fails only on an empty list, and one value gives a leading space | unvalidated | none | 686cfe1 review round 1 fixes |
| 2026-11-01 | 1 | d077706 | 1-6 | supply-chain-reviewer | nit | - | doc | none | xtask/src/rules.rs:3212 | the effort fix text says the agent takes the main session's effort | unvalidated | none | 686cfe1 review round 1 fixes |
| 2026-11-01 | 2 | ed5f898 | 2-1 | reviewer | should-fix | yes | doc | none | docs/plans/active/harness-phase-3b-m5.md:311 | the results table gives the haiku arm 4 of 11 R and Mr items refuted; its three cases hold 10 | confirmed | none | b2626ff review round 2 fixes |
| 2026-11-01 | 2 | ed5f898 | 2-2 | reviewer | nit | - | doc | DOC-1 | docs/plans/active/harness-phase-3b.md:66 | the parent line names agents_folder_problems near line 3159; the check is in agent_problems | unvalidated | none | the parent's line as a diff for the orchestrator |
| 2026-11-01 | 2 | ed5f898 | 2-3 | supply-chain-reviewer | nit | - | doc | none | xtask/src/rules.rs:3215 | the effort fix text names a source of the effort that no doc backs | unvalidated | none | b2626ff review round 2 fixes |
| 2026-11-01 | 3 | 9d705bd | 3-1 | reviewer | nit | - | doc | none | xtask/src/rules.rs:3210 | the model fix text says that decision 0025 gives the model of each role; AGENT_MODELS_MEASURED does | unvalidated | none | 8613d58 review round 3 fixes |
| 2026-11-01 | 3 | 9d705bd | 3-2 | reviewer | nit | - | doc | none | docs/plans/active/harness-phase-3b-m5.md:311 | the haiku row of the results table and TD-71 give no interval, but 14 of 25 has one | unvalidated | none | 8613d58 review round 3 fixes |

## Rounds

| Round | Agents | Tokens | Wall minutes | Interventions | Late review commands | Approvals | Mutants | Baseline agents | Baseline tokens | Baseline wall minutes | Baseline interventions | Baseline approvals | Baseline found |
| - | - | - | - | - | - | - | - | - | - | - | - | - | - |
| 1 | 4 | 298416 | 10.1 | 0 | 1 | 0 (auto mode; a fallback prompt is not counted) | mutants: full: tested 10 (caught 9, missed 0, unviable 1), timeouts 0, not tested 0, known 0 | - | - | - | - | - | - |
| 2 | 3 | 165452 | 5.5 | 0 | 1 | 0 (auto mode; a fallback prompt is not counted) | mutants: full: tested 4 (caught 3, missed 0, unviable 1), timeouts 0, not tested 0, known 0 | - | - | - | - | - | - |
| 3 | 2 | 109354 | 5.5 | 0 | 1 | 0 (auto mode; a fallback prompt is not counted) | mutants: full: tested 2 (caught 1, missed 0, unviable 1), timeouts 0, not tested 0, known 0 | - | - | - | - | - | - |
