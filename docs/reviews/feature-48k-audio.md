# Review log

## Findings

| Date | Round | Commit | Finding | Reviewers | Severity | Kind | Rule | Where | Claim | Verdict | Repeat of | Action |
| - | - | - | - | - | - | - | - | - | - | - | - | - |
| 2026-10-24 | 1 | 9779b2c | 1-1 | memory-reviewer, reviewer | should-fix | bug | none | engine/src/resample.rs:64 | Rising rate: images near the input's Nyquist land in the audible band at -11 to -17 dB | confirmed | none | 3ac5db4 review round 1 fixes |
| 2026-10-24 | 1 | 9779b2c | 1-2 | memory-reviewer, wire-reviewer, docs-reviewer, reviewer | should-fix | weak-test | VERIFY-1 | engine/src/resample.rs:90-121 | No test checks the wasm32 overflow safety of the length and stepping | confirmed | none | 3ac5db4 review round 1 fixes |
| 2026-10-24 | 1 | 9779b2c | 1-3 | docs-reviewer | should-fix | doc | DOC-1 | docs/plans/active/48k-audio.md:42 | Plan claims 105-110 dB SNR to 18 kHz; 98.5 dB at 17.6 kHz; test checks 4 tones | confirmed | none | 3ac5db4 review round 1 fixes |
| 2026-10-24 | 1 | 9779b2c | 1-4 | docs-reviewer | should-fix | bug | MEM-4 | engine/src/resample.rs (Filter::new) | A huge header rate builds a filter table of about 1 GB | confirmed | none | 3ac5db4 review round 1 fixes |
| 2026-10-24 | 1 | 9779b2c | 1-5 | docs-reviewer | should-fix | doc | DOC-1 | app/src/store.rs:559 | Fixture comment says 8 kHz, fixture is 44.1 kHz | confirmed | none | 3ac5db4 review round 1 fixes |
| 2026-10-24 | 1 | 9779b2c | 1-6 | docs-reviewer, wire-reviewer | should-fix | doc | DOC-1 | xtask/src/e2e.rs:1041-1046 | CLIP_RATE splits clip from its doc comment | confirmed | none | 3ac5db4 review round 1 fixes |
| 2026-10-24 | 1 | 9779b2c | 1-7 | reviewer | should-fix | weak-test | VERIFY-1 | engine/src/resample.rs:99-110 | No test checks that input outside the song is silence | confirmed | none | 3ac5db4 review round 1 fixes |
| 2026-10-24 | 1 | 9779b2c | 1-8 | memory-reviewer, wire-reviewer, docs-reviewer | nit | doc | DOC-1 | docs/plans/active/48k-audio.md:125 | Overflow comes after about 10 minutes, not 7 | unvalidated | none | 3ac5db4 review round 1 fixes |
| 2026-10-24 | 1 | 9779b2c | 1-9 | memory-reviewer, docs-reviewer | nit | doc | DOC-1 | docs/plans/active/48k-audio.md:37 | Design says positions are computed; M2 names the wrong test | unvalidated | none | 3ac5db4 review round 1 fixes |
| 2026-10-24 | 1 | 9779b2c | 1-10 | memory-reviewer, docs-reviewer | nit | other | none | app/src/lib.rs:234 | Log says converted for a 44.1 kHz song; live.rs comment stale | unvalidated | none | 3ac5db4 review round 1 fixes |
| 2026-10-24 | 1 | 9779b2c | 1-11 | wire-reviewer, reviewer | nit | weak-test | VERIFY-1 | engine/src/resample.rs:266 | the_same_input_gives_the_same_output cannot fail | unvalidated | none | 3ac5db4 review round 1 fixes |
| 2026-10-24 | 1 | 9779b2c | 1-12 | docs-reviewer | nit | doc | none | README.md:14 | 'a standard rate' is not defined for README readers | unvalidated | none | 3ac5db4 review round 1 fixes |
| 2026-10-24 | 1 | 9779b2c | 1-13 | reviewer | nit | doc | none | engine/src/resample.rs:62 | Integer ratio keeps 64 taps for 64 kHz and 88 kHz, so 20 kHz droops | unvalidated | none | 3ac5db4 review round 1 fixes |
| 2026-10-24 | 2 | 3ac5db4 | 2-1 | reviewer, memory-reviewer, wire-reviewer, docs-reviewer | should-fix | weak-test | VERIFY-1 | engine/src/resample.rs:364 | Overflow test runs only on a 64-bit host | confirmed | none | e9c3aa7 review round 2 fixes; TD-51 |
| 2026-10-24 | 2 | 3ac5db4 | 2-2 | docs-reviewer, memory-reviewer | should-fix | weak-test | VERIFY-1 | engine/src/resample.rs:414 | Range check on `to` untested | confirmed | none | e9c3aa7 review round 2 fixes |
| 2026-10-24 | 2 | 3ac5db4 | 2-3 | wire-reviewer, memory-reviewer, reviewer | should-fix | weak-test | VERIFY-1 | engine/src/resample.rs:117 | Even rounding of the half-width (taps multiple of 4) untested | confirmed | none | e9c3aa7 review round 2 fixes |
| 2026-10-24 | 2 | 3ac5db4 | 2-4 | reviewer | should-fix | bug | none | app/src/store.rs:392 | Old mixed-rate session reopens with stems one frame short; player reads past the end | confirmed | none | e9c3aa7 review round 2 fixes |
| 2026-10-24 | 2 | 3ac5db4 | 2-5 | docs-reviewer | nit | doc | TD-21 | docs/plans/active/48k-audio.md:44 | SNR swing is 13.4 dB, not 10 dB | unvalidated | none | e9c3aa7 review round 2 fixes |
| 2026-10-24 | 2 | 3ac5db4 | 2-6 | docs-reviewer | nit | doc | none | engine/src/resample.rs:114 | About 2 kHz folds back, not 4.1 kHz | unvalidated | none | e9c3aa7 review round 2 fixes |
| 2026-10-24 | 2 | 3ac5db4 | 2-7 | docs-reviewer | nit | weak-test | none | docs/decisions/0012-one-song-rate.md:28 | Image measured against the input level, not the tone | unvalidated | none | e9c3aa7 review round 2 fixes (wording) |
| 2026-10-24 | 2 | 3ac5db4 | 2-8 | docs-reviewer | nit | doc | none | README.md:14 | Two sentences too long for the writing standard | unvalidated | none | e9c3aa7 review round 2 fixes |
| 2026-10-24 | 2 | 3ac5db4 | 2-9 | memory-reviewer | nit | doc | DOC-1 | docs/plans/active/48k-audio.md (Terms) | Standard rate definition lacks the 8 to 384 kHz range | unvalidated | none | e9c3aa7 review round 2 fixes |
| 2026-10-24 | 2 | 3ac5db4 | 2-10 | wire-reviewer | nit | weak-test | none | engine/src/resample.rs:407 | Range bounds can move without a test failing | unvalidated | none | e9c3aa7 review round 2 fixes |
| 2026-10-24 | 2 | 3ac5db4 | 2-11 | reviewer | nit | doc | DOC-1 | docs/plans/active/48k-audio.md:119 | M4 Progress quotes the old log text | unvalidated | none | e9c3aa7 review round 2 fixes |
| 2026-10-24 | 2 | 3ac5db4 | 2-12 | reviewer | nit | doc | DOC-1 | docs/plans/active/48k-audio.md:71 | M1 test list attribution | unvalidated | none | e9c3aa7 review round 2 fixes |
| 2026-10-24 | 3 | e9c3aa7 | 3-1 | reviewer, memory-reviewer, docs-reviewer | should-fix | doc | DOC-1 | docs/tech-debt.md:372 | TD-51 names a review cover that no agent file gives | confirmed | none | e352aee review round 3 fixes |
| 2026-10-24 | 3 | e9c3aa7 | 3-2 | reviewer | should-fix | weak-test | VERIFY-1 | engine/src/song.rs:259 | No test checks that the right channel is converted | confirmed | none | e352aee review round 3 fixes |
| 2026-10-24 | 3 | e9c3aa7 | 3-3 | reviewer, docs-reviewer | nit | doc | TD-21 | engine/src/resample.rs:265 | Test comment says 10 dB swing; plan says 13.4 dB | unvalidated | none | e352aee review round 3 fixes |
| 2026-10-24 | 3 | e9c3aa7 | 3-4 | docs-reviewer | nit | doc | TD-21 | docs/plans/active/48k-audio.md:91 | Old mixed-rate source opens up to one frame longer, not always | unvalidated | none | e352aee review round 3 fixes |
| 2026-10-24 | 3 | e9c3aa7 | 3-5 | docs-reviewer | nit | doc | DOC-1 | docs/invariants.md:149 | VERIFY-1 Gaps line lacks TD-51 | unvalidated | none | e352aee review round 3 fixes |
| 2026-10-24 | 3 | e9c3aa7 | 3-6 | docs-reviewer | nit | rule | none | docs/tech-debt.md:371 | A TD-51 sentence is over 25 words | unvalidated | none | e352aee review round 3 fixes |
| 2026-10-24 | 3 | e9c3aa7 | 3-7 | reviewer | nit | doc | MEM-4 | docs/plans/active/48k-audio.md:46 | 168 MB is the conversion peak, not the open peak | unvalidated | none | e352aee review round 3 fixes |
| 2026-10-24 | 3 | e9c3aa7 | 3-8 | memory-reviewer | nit | other | none | engine/src/resample.rs:150 | Length conversion error has no context | unvalidated | none | e352aee review round 3 fixes |

## Rounds

| Round | Agents | Tokens | Wall minutes | Interventions | Late review commands | Approvals | Mutants | Baseline agents | Baseline tokens | Baseline wall minutes | Baseline interventions | Baseline approvals | Baseline found |
| - | - | - | - | - | - | - | - | - | - | - | - | - | - |
| 1 | 11 | 453812 | 28.2 | 0 | 1 | 0 (auto mode; a fallback prompt is not counted) | mutants: none: tested 0 (caught 0, missed 0, unviable 0), timeouts 0, not tested 95; exit 4: the baseline failed | - | - | - | - | - | - |
| 2 | 8 | 402737 | 21.3 | 1 | 1 | 0 (auto mode; a fallback prompt is not counted) | mutants: none: tested 0 (caught 0, missed 0, unviable 0), timeouts 0, not tested 32; exit 4: the baseline failed | - | - | - | - | - | - |
| 3 | 5 | 263235 | 23.8 | 0 | 1 | 0 (auto mode; a fallback prompt is not counted) | mutants: full: tested 16 (caught 16, missed 0, unviable 0), timeouts 0, not tested 0 | - | - | - | - | - | - |
