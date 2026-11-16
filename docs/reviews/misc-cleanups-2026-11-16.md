# Review log

## Findings

| Date | Round | Commit | Finding | Reviewers | Severity | Significant | Kind | Rule | Where | Claim | Verdict | Repeat of | Action |
| - | - | - | - | - | - | - | - | - | - | - | - | - | - |
| 2026-11-16 | 1 | 9630720 | 1-1 | reviewer, supply-chain-reviewer, docs-reviewer | should-fix | yes | weak-test | VERIFY-1 | xtask/tests/publish.rs:1269 | the publish gate test reads only needs; always(), continue-on-error or a commented job key pass it | confirmed | none | 85353a2 review round 1 fixes |
| 2026-11-16 | 1 | 9630720 | 1-2 | reviewer, supply-chain-reviewer, docs-reviewer | should-fix | yes | doc | DOC-1 | README.md:124 | publish options, offline publish, About-link 404 and "unpublished build" are lost | confirmed | none | 85353a2 review round 1 fixes |
| 2026-11-16 | 1 | 9630720 | 1-3 | reviewer, docs-reviewer | should-fix | no | doc | none | README.md:54 | the snapshot row drops the 5500M and that egui_kittest picks lavapipe | confirmed | none | 85353a2 review round 1 fixes |
| 2026-11-16 | 1 | 9630720 | 1-4 | docs-reviewer | should-fix | no | doc | none | README.md:53 | the cargo test row does not name ps, lsof and sleep | confirmed | none | 85353a2 review round 1 fixes |
| 2026-11-16 | 1 | 9630720 | 1-5 | docs-reviewer | should-fix | yes | doc | TD-21 (VERIFY-1) | README.md:84 | README omits that tier push can run the GPU test line, which needs a GPU | confirmed | none | 85353a2 review round 1 fixes |
| 2026-11-16 | 1 | 9630720 | 1-6 | reviewer | nit | - | doc | none | README.md:78 | "the checks that CI gates on" overstates tier push | unvalidated | none | 85353a2 review round 1 fixes |
| 2026-11-16 | 1 | 9630720 | 1-7 | reviewer, docs-reviewer | nit | - | doc | TD-21 (VERIFY-1) | README.md:18 | the lane share of the memory limit does not apply to a save; a cause is dropped | unvalidated | none | 85353a2 review round 1 fixes |
| 2026-11-16 | 1 | 9630720 | 1-8 | docs-reviewer | nit | - | doc | none | README.md:128 | "CI publishes each commit" is wrong | unvalidated | none | 85353a2 review round 1 fixes |
| 2026-11-16 | 1 | 9630720 | 1-9 | docs-reviewer | nit | - | doc | none | README.md:16 | the Windows AAC sentence is removed | unvalidated | none | 85353a2 review round 1 fixes |
| 2026-11-16 | 2 | 85353a2 | 2-1 | reviewer, supply-chain-reviewer, docs-reviewer | should-fix | yes | weak-test | VERIFY-1 | xtask/tests/publish.rs:1323 | the status check reads only the if: line and not success(); a folded if passes | confirmed | none | eb9a1d8 review round 2 fixes |
| 2026-11-16 | 2 | 85353a2 | 2-2 | supply-chain-reviewer | should-fix | yes | weak-test | VERIFY-1 | xtask/tests/publish.rs:1336 | continue-on-error as the first key of a step is not seen | confirmed | none | eb9a1d8 review round 2 fixes |
| 2026-11-16 | 2 | 85353a2 | 2-3 | docs-reviewer | should-fix | yes | doc | DOC-1 | README.md:84 | "It runs no browser test" is false: tier push can run the perf gate | confirmed | none | eb9a1d8 review round 2 fixes |
| 2026-11-16 | 2 | 85353a2 | 2-4 | supply-chain-reviewer | nit | - | weak-test | VERIFY-1 | xtask/tests/publish.rs:1329 | the continue_on_error input of an action is not matched | unvalidated | none | eb9a1d8 review round 2 fixes |
| 2026-11-16 | 2 | 85353a2 | 2-5 | supply-chain-reviewer | nit | - | weak-test | VERIFY-1 | xtask/tests/publish.rs:1272 | a quoted job name is not seen as a job | unvalidated | none | eb9a1d8 review round 2 fixes |
| 2026-11-16 | 2 | 85353a2 | 2-6 | reviewer, docs-reviewer | nit | - | doc | none | README.md:84 | the GPU test line sentence leaves out "no path changed" | unvalidated | none | eb9a1d8 review round 2 fixes |
| 2026-11-16 | 2 | 85353a2 | 2-7 | docs-reviewer | nit | - | doc | TD-21 (VERIFY-1) | README.md:130 | --skip-if-behind counts only builds of that branch | unvalidated | none | eb9a1d8 review round 2 fixes |
| 2026-11-16 | 2 | 85353a2 | 2-8 | docs-reviewer | nit | - | rule | none | README.md:18 | two added sentences are longer than 25 words | unvalidated | none | eb9a1d8 review round 2 fixes |
| 2026-11-16 | 2 | 85353a2 | 2-9 | reviewer | nit | - | doc | DOC-1 | docs/working-with-agents.md:177 | "README.md has the details" is only partly true | unvalidated | none | eb9a1d8 review round 2 fixes |
| 2026-11-16 | 2 | 85353a2 | 2-10 | reviewer | nit | - | doc | none | README.md:18 | "the mix" has no earlier meaning | unvalidated | none | eb9a1d8 review round 2 fixes |
| 2026-11-16 | 2 | 85353a2 | 2-11 | reviewer | nit | - | doc | none | README.md:112 | the time limits are in scenario_limit_secs | unvalidated | none | eb9a1d8 review round 2 fixes |
| 2026-11-16 | 3 | eb9a1d8 | 3-1 | reviewer, supply-chain-reviewer, docs-reviewer | should-fix | yes | weak-test | VERIFY-1 | xtask/tests/publish.rs:1328 | a job-level if: always() after the steps of publish passes the test | confirmed | none | 2ccac4c review round 3 fixes |
| 2026-11-16 | 3 | eb9a1d8 | 3-2 | reviewer, supply-chain-reviewer, docs-reviewer | should-fix | yes | doc | DOC-1 | README.md:84 | README and TD-70 leave out the path condition of the perf gate | confirmed | none | 2ccac4c review round 3 fixes |
| 2026-11-16 | 3 | eb9a1d8 | 3-3 | docs-reviewer | nit | - | doc | none | docs/tech-debt.md:390 | "but for" is not STE | unvalidated | none | 2ccac4c review round 3 fixes |
| 2026-11-16 | 3 | eb9a1d8 | 3-4 | docs-reviewer | nit | - | doc | TD-21 (VERIFY-1) | docs/working-with-agents.md:177 | not each xtask command has module docs | unvalidated | none | 2ccac4c review round 3 fixes |
| 2026-11-16 | 3 | eb9a1d8 | 3-5 | docs-reviewer | nit | - | doc | TD-21 (VERIFY-1) | README.md:130 | --skip-if-behind also publishes the newest published commit | unvalidated | none | 2ccac4c review round 3 fixes |
| 2026-11-16 | 4 | 2ccac4c | 4-1 | docs-reviewer, reviewer | should-fix | yes | doc | DOC-1 | docs/working-with-agents.md:183 | the Tools row gives the wrong paths for the perf gate of tier push | pre-existing | none | f3cacb4 review round 4 fixes |
| 2026-11-16 | 4 | 2ccac4c | 4-2 | docs-reviewer, reviewer | nit | - | doc | none | docs/tech-debt.md:390 | "Tier push runs only the perf gate" can read as one check only | unvalidated | none | reported, not fixed |
| 2026-11-16 | 4 | 2ccac4c | 4-3 | supply-chain-reviewer | nit | - | doc | DOC-1 | README.md:84 | the perf gate sentence leaves out "no path changed" | unvalidated | none | reported, not fixed |
| 2026-11-16 | 4 | 2ccac4c | 4-4 | supply-chain-reviewer | nit | - | weak-test | VERIFY-1 | xtask/tests/publish.rs:1328 | the status-function match is case-sensitive | unvalidated | none | reported, not fixed |

## Rounds

| Round | Agents | Tokens | Wall minutes | Interventions | Late review commands | Approvals | Mutants | Baseline agents | Baseline tokens | Baseline wall minutes | Baseline interventions | Baseline approvals | Baseline found |
| - | - | - | - | - | - | - | - | - | - | - | - | - | - |
| 1 | 8 | 456318 | 9.6 | 0 | 1 | 0 (auto mode; a fallback prompt is not counted) | mutants: none: tested 0 (caught 0, missed 0, unviable 0), timeouts 0, not tested 0, known 0; no mutants in the diff | - | - | - | - | - | - |
| 2 | 6 | 313887 | 8.0 | 0 | 1 | 0 (auto mode; a fallback prompt is not counted) | mutants: none: tested 0 (caught 0, missed 0, unviable 0), timeouts 0, not tested 0, known 0; no mutants in the diff | - | - | - | - | - | - |
| 3 | 5 | 280205 | 12.3 | 1 | 1 | 0 (auto mode; a fallback prompt is not counted) | mutants: none: tested 0 (caught 0, missed 0, unviable 0), timeouts 0, not tested 0, known 0; no mutants in the diff | - | - | - | - | - | - |
| 4 | 4 | 185023 | 5.7 | 0 | 1 | 0 (auto mode; a fallback prompt is not counted) | mutants: none: tested 0 (caught 0, missed 0, unviable 0), timeouts 0, not tested 0, known 0; no mutants in the diff | - | - | - | - | - | - |
| 5 | 2 | 132431 | 1.7 | 0 | 1 | 0 (auto mode; a fallback prompt is not counted) | mutants: none: tested 0 (caught 0, missed 0, unviable 0), timeouts 0, not tested 0, known 0; no mutants in the diff | - | - | - | - | - | - |
