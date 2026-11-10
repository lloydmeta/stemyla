# Review log

## Findings

| Date | Round | Commit | Finding | Reviewers | Severity | Significant | Kind | Rule | Where | Claim | Verdict | Repeat of | Action |
| - | - | - | - | - | - | - | - | - | - | - | - | - | - |
| 2026-11-10 | 1 | 573ed9f | 1-1 | reviewer, memory-reviewer, wire-reviewer, docs-reviewer | should-fix | yes | weak-test | VERIFY-1 | app/src/lib.rs:4842 | no test checks that a memory-path M4A save posts Request::Alac | confirmed | none | 0ab2bde review round 1 fixes |
| 2026-11-10 | 1 | 573ed9f | 1-2 | reviewer, docs-reviewer | nit | - | doc | DOC-1 | docs/invariants.md:54 | MEM-1's "do the same" now refers to the ALAC sentence | unvalidated | none | 0ab2bde review round 1 fixes |
| 2026-11-10 | 1 | 573ed9f | 1-3 | docs-reviewer, wire-reviewer | nit | - | doc | DOC-1 | docs/tech-debt.md:493 | TD-53 leaves out the stored M4A request, and has a sentence of 27 words | unvalidated | none | 0ab2bde review round 1 fixes |
| 2026-11-10 | 1 | 573ed9f | 1-4 | docs-reviewer | nit | - | doc | none | docs/invariants.md:62 | MEM-2 says a FLAC-only test covers FLAC, M4A and MP3 | unvalidated | none | 0ab2bde review round 1 fixes |
| 2026-11-10 | 1 | 573ed9f | 1-5 | docs-reviewer | nit | - | doc | none | app/src/separator.rs:364 | the comment on encode errors leaves out ALAC | unvalidated | none | 0ab2bde review round 1 fixes |
| 2026-11-10 | 1 | 573ed9f | 1-6 | docs-reviewer | nit | - | doc | none | docs/references/local-browser-testing.md:3 | the doc names two Macs but says "that machine" | unvalidated | none | 0ab2bde review round 1 fixes |
| 2026-11-10 | 1 | 573ed9f | 1-7 | docs-reviewer | nit | - | doc | none | docs/plans/active/m4a.md:260 | a sentence of 26 words, and blank lines of the milestone plan | unvalidated | none | 0ab2bde review round 1 fixes, and plan-round-1.diff for the parent plan |
| 2026-11-10 | 1 | 573ed9f | 1-8 | memory-reviewer | nit | - | weak-test | VERIFY-1, MEM-1 | worker/src/web/tests.rs:2023 | the 12-byte assert follows from the 4-byte assert, and no test measures the end of the encode | unvalidated | none | 0ab2bde review round 1 fixes |
| 2026-11-10 | 1 | 573ed9f | 1-9 | memory-reviewer, wire-reviewer | nit | - | weak-test | VERIFY-1 | worker/src/web/encode.rs:387 | no test checks the default reply name stem.m4a | unvalidated | none | 0ab2bde review round 1 fixes |
| 2026-11-10 | 1 | 573ed9f | 1-10 | supply-chain-reviewer | nit | - | doc | none | .github/workflows/ci.yml:172 | the comment on the job limit claims a spare with no measure | unvalidated | none | 0ab2bde review round 1 fixes |
| 2026-11-10 | 1 | 573ed9f | 1-11 | cargo-mutants | should-fix | no | weak-test | VERIFY-1 | protocol/src/lib.rs:440 | no test fails with this mutant: replace Request::kind with "" | pre-existing | none | 0ab2bde review round 1 fixes |
| 2026-11-10 | 1 | 573ed9f | 1-12 | cargo-mutants | should-fix | no | weak-test | VERIFY-1 | protocol/src/lib.rs:440 | no test fails with this mutant: replace Request::kind with "xyzzy" | pre-existing | none | 0ab2bde review round 1 fixes |
| 2026-11-10 | 1 | 573ed9f | 1-13 | cargo-mutants | should-fix | - | weak-test | VERIFY-1 | protocol/src/lib.rs:664 | no test fails with this mutant: replace Reply::to_js with defaults | refuted | none | refuted |
| 2026-11-10 | 1 | 573ed9f | 1-14 | cargo-mutants | should-fix | - | weak-test | VERIFY-1 | protocol/src/lib.rs:457 | no test fails with this mutant: replace Request::to_js with defaults | refuted | none | refuted |
| 2026-11-10 | 2 | 1248944 | 2-1 | reviewer, memory-reviewer, docs-reviewer | should-fix | yes | weak-test | VERIFY-1 | worker/src/web/tests.rs:2013 | no test checks the end of the ALAC encode, and no TD records it | confirmed | none | bfd885d review round 2 fixes |
| 2026-11-10 | 2 | 1248944 | 2-2 | docs-reviewer | nit | - | doc | none | worker/src/web/tests.rs:2010 | the test doc qualifies an implication that holds at every length | unvalidated | none | bfd885d review round 2 fixes |
| 2026-11-10 | 2 | 1248944 | 2-3 | docs-reviewer | nit | - | doc | none | docs/ARCHITECTURE.md:153 | "The end holds by construction" has no clear subject | unvalidated | none | bfd885d review round 2 fixes |
| 2026-11-10 | 2 | 1248944 | 2-4 | wire-reviewer | nit | - | doc | none | app/src/separator.rs:364 | the comment leaves out the other request kinds that take the arm | unvalidated | none | bfd885d review round 2 fixes |
| 2026-11-10 | 2 | 1248944 | 2-5 | wire-reviewer | nit | - | doc | none | worker/src/web/tests.rs:2055 | the test doc does not name its check of the default reply name | unvalidated | none | bfd885d review round 2 fixes |
| 2026-11-10 | 2 | 1248944 | 2-6 | supply-chain-reviewer | nit | - | doc | none | .github/workflows/ci.yml:175 | the base of the CI spare is from before M1 and M2 | unvalidated | none | bfd885d review round 2 fixes |
| 2026-11-10 | 2 | 1248944 | 2-7 | supply-chain-reviewer | nit | - | weak-test | VERIFY-1 | app/src/save_tests.rs:1450 | the memory-path test checks only the left array | unvalidated | none | bfd885d review round 2 fixes |
| 2026-11-10 | 3 | 5467677 | 3-1 | supply-chain-reviewer | nit | - | doc | none | .github/workflows/ci.yml:175 | the CI comment gives the wrong reason for the range of the spare | unvalidated | none | 0377e56 review round 3 fixes |
| 2026-11-10 | 3 | 5467677 | 3-2 | docs-reviewer | nit | - | doc | DOC-1 | docs/ARCHITECTURE.md:153 | the text says the size of moov, not the memory of the end, is fixed by construction | unvalidated | none | 0377e56 review round 3 fixes |
| 2026-11-10 | 3 | 5467677 | 3-3 | docs-reviewer | nit | - | doc | none | docs/invariants.md:54 | MEM-1 says the capacity test checks one buffer; it checks the capacity only | unvalidated | none | 0377e56 review round 3 fixes |
| 2026-11-10 | 4 | c738fdd | 4-1 | supply-chain-reviewer | nit | - | doc | none | .github/workflows/ci.yml:179 | the CI comment names a guessed cause of the gap between two runs | unvalidated | none | reported, not fixed |
| 2026-11-10 | 4 | c738fdd | 4-2 | wire-reviewer, docs-reviewer | nit | - | doc | DOC-1 | docs/plans/completed/m4a-m3.md:387 | the retrospective names blocks of the reserve with no path | unvalidated | none | reported, not fixed |
| 2026-11-10 | 4 | c738fdd | 4-3 | reviewer, docs-reviewer | nit | - | doc | none | docs/ARCHITECTURE.md:153 | "the memory of the end is fixed" reads as a constant | unvalidated | none | reported, not fixed |
| 2026-11-10 | 4 | c738fdd | 4-4 | reviewer | nit | - | doc | none | docs/plans/completed/m4a-m3.md:328 | the Progress lines count 3 rounds, before round 4 | unvalidated | none | reported, not fixed |
| 2026-11-10 | 4 | c738fdd | 4-5 | reviewer | nit | - | doc | none | docs/plans/completed/m4a-m3.md:326 | the Step 6 line names the WAV save, not the planted mix | unvalidated | none | reported, not fixed |
| 2026-11-10 | 4 | c738fdd | 4-6 | docs-reviewer | nit | - | doc | none | docs/plans/active/m4a.md:301 | two new sentences have more than 25 words | unvalidated | none | reported, not fixed |

## Rounds

| Round | Agents | Tokens | Wall minutes | Interventions | Late review commands | Approvals | Mutants | Baseline agents | Baseline tokens | Baseline wall minutes | Baseline interventions | Baseline approvals | Baseline found |
| - | - | - | - | - | - | - | - | - | - | - | - | - | - |
| 1 | 7 | 827348 | 20.7 | 0 | 1 | 0 (auto mode; a fallback prompt is not counted) | mutants: full: tested 64 (caught 52, missed 4, unviable 8), timeouts 0, not tested 0, known 0 | - | - | - | - | - | - |
| 2 | 6 | 556214 | 17.6 | 0 | 1 | 0 (auto mode; a fallback prompt is not counted) | mutants: none: tested 0 (caught 0, missed 0, unviable 0), timeouts 0, not tested 0, known 0; no mutants in the diff | - | - | - | - | - | - |
| 3 | 5 | 420006 | 7.5 | 0 | 1 | 0 (auto mode; a fallback prompt is not counted) | mutants: none: tested 0 (caught 0, missed 0, unviable 0), timeouts 0, not tested 0, known 0; no mutants in the diff | - | - | - | - | - | - |
| 4 | 5 | 373687 | 4.5 | 0 | 1 | 0 (auto mode; a fallback prompt is not counted) | mutants: none: tested 0 (caught 0, missed 0, unviable 0), timeouts 0, not tested 0, known 0; no mutants in the diff | - | - | - | - | - | - |
