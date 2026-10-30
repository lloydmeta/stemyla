# Review log

## Findings

| Date | Round | Commit | Finding | Reviewers | Severity | Kind | Rule | Where | Claim | Verdict | Repeat of | Action |
| - | - | - | - | - | - | - | - | - | - | - | - | - |
| 2026-10-25 | 1 | 9bba659 | 1-1 | reviewer, memory-reviewer | blocker | bug | none | streamtest.rs:246-258 | AudioContext made after the writes, outside the tap | confirmed | none | 0c7583e review round 1 fixes |
| 2026-10-25 | 1 | 9bba659 | 1-2 | reviewer, memory-reviewer | should-fix | bug | none | streamtest.rs:100-104 | A failed read hangs the next seek | confirmed | none | 0c7583e review round 1 fixes |
| 2026-10-25 | 1 | 9bba659 | 1-3 | reviewer | should-fix | bug | none | streamtest.rs:274-281 | Each seek waits for a stale read | confirmed | none | 0c7583e review round 1 fixes |
| 2026-10-25 | 1 | 9bba659 | 1-4 | reviewer, memory-reviewer | should-fix | weak-test | VERIFY-1 | streamtest.rs:399-450 | Test checks no counted value; block never fires | confirmed | none | 0c7583e review round 1 fixes |
| 2026-10-25 | 1 | 9bba659 | 1-5 | reviewer, memory-reviewer | should-fix | other | none | streamtest.rs:260-263 | All runs read from frame 0 | confirmed | none | 0c7583e review round 1 fixes |
| 2026-10-25 | 1 | 9bba659 | 1-6 | reviewer, docs-reviewer | should-fix | weak-test | VERIFY-1 | streamtest.rs:33-38 | URL path untested | confirmed | none | 0c7583e review round 1 fixes |
| 2026-10-25 | 1 | 9bba659 | 1-7 | docs-reviewer | should-fix | weak-test | MEM-5 | streamtest.rs:404-406 | Full-block write path never spied | confirmed | none | 0c7583e review round 1 fixes |
| 2026-10-25 | 1 | 9bba659 | 1-8 | memory-reviewer | should-fix | bug | none | streamtest.rs:128-160 | Schedule resets to the clock, unlike cpal | confirmed | none | 0c7583e review round 1 fixes |
| 2026-10-25 | 1 | 9bba659 | 1-9 | reviewer, memory-reviewer, docs-reviewer | nit | doc | none | streamtest.rs:404 | 13 MB is 19 MB | unvalidated | none | 0c7583e review round 1 fixes |
| 2026-10-25 | 1 | 9bba659 | 1-10 | reviewer | nit | other | none | streamtest.rs:231 | Concurrent read ignores bytes | unvalidated | none | 0c7583e review round 1 fixes |
| 2026-10-25 | 1 | 9bba659 | 1-11 | docs-reviewer | nit | doc | DOC-1 | plan:109 | Plan says the spike reloads | unvalidated | none | 0c7583e review round 1 fixes |
| 2026-10-25 | 1 | 9bba659 | 1-12 | docs-reviewer | nit | doc | none | README.md:20 | Button wording, st_seeks is not a length | unvalidated | none | 0c7583e review round 1 fixes |
| 2026-10-25 | 1 | 9bba659 | 1-13 | docs-reviewer | nit | doc | none | 9bba659 message, streamtest.rs:7-9 | Sentences over 25 words | unvalidated | none | module doc fixed in 0c7583e; the commit message stays |
| 2026-10-25 | 1 | 9bba659 | 1-14 | memory-reviewer | nit | other | none | streamtest.rs:145-153 | take() counts an unscheduled buffer | unvalidated | none | 0c7583e review round 1 fixes |
| 2026-10-25 | 1 | 9bba659 | 1-15 to 1-21 | cargo-mutants | should-fix | weak-test | VERIFY-1 | streamtest.rs, lib.rs | 7 missed mutants in wasm32-only code | refuted | none | refuted; .cargo/mutants.toml leaves the file out in 0c7583e |
| 2026-10-25 | 2 | 0c7583e | 2-1 | reviewer, docs-reviewer | should-fix | bug | VERIFY-1 | streamtest.rs:216-230 | Timer clock has two buffers of slack, not one | confirmed | none | 60a409e review round 2 fixes |
| 2026-10-25 | 2 | 0c7583e | 2-2 | docs-reviewer | should-fix | doc | DOC-1 | streamtest.rs:168-170, :314 | Late buffers are not each silent | confirmed | none | 60a409e review round 2 fixes |
| 2026-10-25 | 2 | 0c7583e | 2-3 | memory-reviewer, reviewer, docs-reviewer | should-fix | weak-test | VERIFY-1 | streamtest.rs:566-570 | Concurrent check passes with no read compared | confirmed | none | 60a409e review round 2 fixes |
| 2026-10-25 | 2 | 0c7583e | 2-4 | memory-reviewer | should-fix | bug | none | streamtest.rs:298-302 | Audio context not closed on an early failure | confirmed | none | 60a409e review round 2 fixes |
| 2026-10-25 | 2 | 0c7583e | 2-5 | reviewer | should-fix | weak-test | VERIFY-1 | streamtest.rs:530-608 | Read, seek, late and right counts untested | confirmed | none | 60a409e review round 2 fixes |
| 2026-10-25 | 2 | 0c7583e | 2-6 | reviewer, memory-reviewer | should-fix | weak-test | VERIFY-1 | streamtest.rs:550 | Audio clock asserted in Safari too | confirmed | none | 60a409e review round 2 fixes |
| 2026-10-25 | 2 | 0c7583e | 2-7 | supply-chain-reviewer | should-fix | doc | DOC-1 | tech-debt.md:180 | TD-33 list misses streamtest.rs | confirmed | none | 60a409e review round 2 fixes |
| 2026-10-25 | 2 | 0c7583e | 2-8 | docs-reviewer | nit | doc | none | streamtest.rs:165-172 | Sentence over 25 words | unvalidated | none | 60a409e review round 2 fixes |
| 2026-10-25 | 2 | 0c7583e | 2-9 | supply-chain-reviewer | nit | doc | none | web_test.rs:77-78 | Timeout comment says 25 s | unvalidated | none | 60a409e review round 2 fixes |
| 2026-10-25 | 2 | 0c7583e | 2-10 | supply-chain-reviewer | nit | weak-test | VERIFY-1 | web_test.rs:77-81 | No test of the timeout default | unvalidated | none | not fixed: TD-39 covers web_test::run |
| 2026-10-25 | 2 | 0c7583e | 2-11 | memory-reviewer | nit | other | RT-1 | streamtest.rs:188-196 | Callback model starts reads | unvalidated | none | comment in 60a409e |
| 2026-10-25 | 2 | 0c7583e | 2-12 | reviewer | nit | other | none | streamtest.rs:315 | Tempo 1.5 hard-coded | unvalidated | none | 60a409e review round 2 fixes |
| 2026-10-25 | 2 | 0c7583e | 2-13 | cargo-mutants | should-fix | weak-test | VERIFY-1 | web_test.rs:28 | run replaced with Ok(()) is not caught | pre-existing | none | TD-39 lists it |
| 2026-10-25 | 3 | 60a409e | 3-1 | docs-reviewer, supply-chain-reviewer | blocker | doc | DOC-1 | invariants.md:145 | VERIFY-1 names one skip exception; there are two | confirmed | none | 9652649 review round 3 fixes |
| 2026-10-25 | 3 | 60a409e | 3-2 | reviewer | should-fix | weak-test | VERIFY-1 | streamtest.rs:208-346 | Late 0 without a block, p99 and max untested | confirmed | none | 9652649 review round 3 fixes |
| 2026-10-25 | 3 | 60a409e | 3-3 | reviewer | should-fix | doc | none | plan:213, streamtest.rs:199-234 | No iPhone run conditions; playback hangs if audio stops | confirmed | none | 9652649 review round 3 fixes |
| 2026-10-25 | 3 | 60a409e | 3-4 | docs-reviewer | nit | doc | none | plan:84 | late pair not defined | unvalidated | none | 9652649 review round 3 fixes |
| 2026-10-25 | 3 | 60a409e | 3-5 | docs-reviewer | nit | doc | none | web_test.rs:78-80, streamtest.rs:610-612 | Sentences over 25 words | unvalidated | none | 9652649 review round 3 fixes |
| 2026-10-25 | 3 | 60a409e | 3-6 | reviewer | nit | doc | none | 9bba659 message | Message says 50 ms; code uses 30 ms | unvalidated | none | no change: history; the PR text says 30 ms |
| 2026-10-25 | 3 | 60a409e | 3-7 | cargo-mutants | should-fix | weak-test | VERIFY-1 | web_test.rs:28 | run replaced with Ok(()) is not caught | pre-existing | none | TD-39 lists it |

## Rounds

| Round | Agents | Tokens | Wall minutes | Interventions | Late review commands | Approvals | Mutants | Baseline agents | Baseline tokens | Baseline wall minutes | Baseline interventions | Baseline approvals | Baseline found |
| - | - | - | - | - | - | - | - | - | - | - | - | - | - |
| 1 | 12 | 426326 | 32.1 | 1 | 2 | 0 (auto mode; a fallback prompt is not counted) | mutants: full: tested 9 (caught 1, missed 7, unviable 1), timeouts 0, not tested 0 | - | - | - | - | - | - |
| 2 | 12 | 485570 | 27.6 | 0 | 1 | 0 (auto mode; a fallback prompt is not counted) | mutants: full: tested 1 (caught 0, missed 1, unviable 0), timeouts 0, not tested 0 | - | - | - | - | - | - |
| 3 | 8 | 343297 | 30.1 | 0 | 2 | 0 (auto mode; a fallback prompt is not counted) | mutants: full: tested 1 (caught 0, missed 1, unviable 0), timeouts 0, not tested 0 | - | - | - | - | - | - |
