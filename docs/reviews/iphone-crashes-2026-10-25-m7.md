# Review log

## Findings

| Date | Round | Commit | Finding | Reviewers | Severity | Kind | Rule | Where | Claim | Verdict | Repeat of | Action |
| - | - | - | - | - | - | - | - | - | - | - | - | - |
| 2026-10-28 | 1 | db45675 | 1-1 | reviewer, memory-reviewer | should-fix | bug | none | app/src/lib.rs:2856 | memory path refuses a song with an in-memory original and stored stems (as_song) | confirmed | none | ff9c50e review round 1 fixes |
| 2026-10-28 | 1 | db45675 | 1-2 | memory-reviewer | should-fix | rule | MEM-1 | app/src/store.rs:1573 | a complete stem with no overview is read whole | confirmed | none | ff9c50e review round 1 fixes |
| 2026-10-28 | 1 | db45675 | 1-3 | memory-reviewer, docs-reviewer, supply-chain-reviewer | should-fix | doc | MEM-1 | docs/invariants.md:55 | MEM-1 Gaps do not name TD-54 and TD-58 | confirmed | none | ff9c50e review round 1 fixes |
| 2026-10-28 | 1 | db45675 | 1-4 | docs-reviewer, wire-reviewer, reviewer | blocker | doc | DOC-1 | docs/ARCHITECTURE.md:44 | the page "reads the session files back" after step 7 removed it | confirmed | none | ff9c50e review round 1 fixes |
| 2026-10-28 | 1 | db45675 | 1-5 | docs-reviewer | blocker | doc | DOC-1 | docs/ARCHITECTURE.md:240 | "every lane plays from memory" with no read-ahead; stored lanes play silence | confirmed | none | ff9c50e review round 1 fixes |
| 2026-10-28 | 1 | db45675 | 1-6 | docs-reviewer | should-fix | doc | DOC-1 | docs/ARCHITECTURE.md:234 | "the stems are on the page" is true only on the memory path | confirmed | none | ff9c50e review round 1 fixes |
| 2026-10-28 | 1 | db45675 | 1-7 | docs-reviewer, reviewer | should-fix | doc | DOC-1 | docs/ARCHITECTURE.md:217 | "while the page still keeps its samples" describes removed ReadBack | confirmed | none | ff9c50e review round 1 fixes |
| 2026-10-28 | 1 | db45675 | 1-8 | docs-reviewer | should-fix | doc | TD-21 | app/src/playback.rs:136 | Playback::start says stored lanes play from memory; they play silence | confirmed | none | ff9c50e review round 1 fixes |
| 2026-10-28 | 1 | db45675 | 1-9 | docs-reviewer, supply-chain-reviewer | should-fix | weak-test | VERIFY-1 | xtask/src/e2e/old_session.rs:106 | the 7 of 7 check survives two mutants | confirmed | none | ff9c50e review round 1 fixes |
| 2026-10-28 | 1 | db45675 | 1-10 | reviewer | should-fix | bug | none | app/src/lib.rs:267 | no status line when a stored song gets no original lane | confirmed | none | ff9c50e review round 1 fixes |
| 2026-10-28 | 1 | db45675 | 1-11 | reviewer | should-fix | bug | none | app/src/lib.rs:1772 | apply_inbox overwrites the "play silent" note | confirmed | none | ff9c50e review round 1 fixes |
| 2026-10-28 | 1 | db45675 | 1-12 | reviewer | should-fix | weak-test | VERIFY-1 | app/src/lib.rs:4186 | classic-drums generation test is flaky (real HPSS, 120 s clock) | confirmed | none | ff9c50e review round 1 fixes |
| 2026-10-28 | 1 | db45675 | 1-13 | memory-reviewer | nit | weak-test | MEM-4 | xtask/src/e2e/old_session.rs:165 | memory read once, at the first playback line | unvalidated | none | ff9c50e review round 1 fixes |
| 2026-10-28 | 1 | db45675 | 1-14 | wire-reviewer | nit | doc | DOC-1 | docs/ARCHITECTURE.md:135 | worker memory lines named for decode only | unvalidated | none | ff9c50e review round 1 fixes |
| 2026-10-28 | 1 | db45675 | 1-15 | wire-reviewer | nit | weak-test | VERIFY-1 | protocol/tests/web.rs | the overview reason check cannot fail | unvalidated | none | ff9c50e review round 1 fixes |
| 2026-10-28 | 1 | db45675 | 1-16 | wire-reviewer | nit | doc | none | app/src/store_meta.rs:53 | two requests of one generation share part paths | unvalidated | none | ff9c50e review round 1 fixes (claim weakened) |
| 2026-10-28 | 1 | db45675 | 1-17 | wire-reviewer | nit | doc | none | protocol/src/decode.rs:48 | doc line too long | unvalidated | none | ff9c50e review round 1 fixes |
| 2026-10-28 | 1 | db45675 | 1-18 | docs-reviewer, supply-chain-reviewer | nit | doc | none | docs/invariants.md:78 | MEM-4 "prose only" contradicts old-session | unvalidated | none | ff9c50e review round 1 fixes |
| 2026-10-28 | 1 | db45675 | 1-19 | docs-reviewer | nit | doc | none | docs/invariants.md:54 | "no more heap" vs 4 MB allowed | unvalidated | none | ff9c50e review round 1 fixes |
| 2026-10-28 | 1 | db45675 | 1-20 | docs-reviewer | nit | doc | none | README.md:15 | growth bound stated loosely | unvalidated | none | ff9c50e review round 1 fixes |
| 2026-10-28 | 1 | db45675 | 1-21 | docs-reviewer | nit | doc | none | docs/ARCHITECTURE.md:3 | intro, diagram and map omit overviews and classic | unvalidated | none | ff9c50e review round 1 fixes |
| 2026-10-28 | 1 | db45675 | 1-22 | docs-reviewer | nit | doc | none | app/src/store.rs:2422 | MEM-5 comment above the wrong item | unvalidated | none | ff9c50e review round 1 fixes |
| 2026-10-28 | 1 | db45675 | 1-23 | docs-reviewer | nit | doc | none | app/src/lanes.rs:613 | comments name removed read_back | unvalidated | none | ff9c50e review round 1 fixes |
| 2026-10-28 | 1 | db45675 | 1-24 | docs-reviewer | nit | doc | none | docs/invariants.md:51 | sentences over 25 words | unvalidated | none | ff9c50e review round 1 fixes |
| 2026-10-28 | 1 | db45675 | 1-25 | docs-reviewer | nit | other | none | commit db45675 | "every e2e scenario" overstates | unvalidated | none | no change: a past commit message |
| 2026-10-28 | 1 | db45675 | 1-26 | supply-chain-reviewer | nit | other | none | xtask/src/e2e/old_session.rs:36 | two MAX_GROWTH_MB constants | unvalidated | none | ff9c50e review round 1 fixes |
| 2026-10-28 | 1 | db45675 | 1-27 | supply-chain-reviewer | nit | other | none | .cargo/mutants.toml:34 | wasm32-only functions of steps 5b and 6b not excluded | unvalidated | none | ff9c50e review round 1 fixes |
| 2026-10-28 | 1 | db45675 | 1-28 | reviewer | nit | bug | none | app/src/lib.rs:1592 | held stems drop the stems' mix overview | unvalidated | none | ff9c50e review round 1 fixes |
| 2026-10-28 | 2 | 8b77dac | 2-1 | memory-reviewer, supply-chain-reviewer | should-fix | weak-test | MEM-1 | engine/src/hpss.rs:891 | HPSS memory test lets a 16-bit song-length buffer pass after the cut to 3 s / 12 s | confirmed | none | c89c618 review round 2 fixes |
| 2026-10-28 | 2 | 8b77dac | 2-2 | docs-reviewer, reviewer, memory-reviewer, supply-chain-reviewer | should-fix | doc | DOC-1 | docs/plans/active/stream-from-storage-m7.md:118 | step 1, its Decision log entry and the assert message give the old lengths | confirmed | none | c89c618 review round 2 fixes |
| 2026-10-28 | 2 | 8b77dac | 2-3 | docs-reviewer | should-fix | doc | DOC-1 | docs/plans/active/stream-from-storage-m7.md:328 | old-session reads memory twice; the Decision log says once | confirmed | none | c89c618 review round 2 fixes |
| 2026-10-28 | 2 | 8b77dac | 2-4 | docs-reviewer | should-fix | weak-test | VERIFY-1 | app/src/lib.rs:1689 | NO_ORIGINAL on the status line and the open's note in apply_inbox are untested | confirmed | none | c89c618 review round 2 fixes |
| 2026-10-28 | 2 | 8b77dac | 2-5 | docs-reviewer | should-fix | doc | MEM-1 | docs/invariants.md:54 | a test is said to show stems with no read; it checks they are not read whole | confirmed | none | c89c618 review round 2 fixes |
| 2026-10-28 | 2 | 8b77dac | 2-6 | reviewer | should-fix | weak-test | VERIFY-1 | app/src/lib.rs:750 | D2's check on the drop and pick path has no test | confirmed | none | c89c618 review round 2 fixes |
| 2026-10-28 | 2 | 8b77dac | 2-7 | docs-reviewer | nit | doc | none | app/src/lib.rs:4569 | D8 doc comment above the wrong test | unvalidated | none | c89c618 review round 2 fixes |
| 2026-10-28 | 2 | 8b77dac | 2-8 | docs-reviewer | nit | doc | DOC-1 | app/src/store.rs:1619 | "read them back" | unvalidated | none | c89c618 review round 2 fixes |
| 2026-10-28 | 2 | 8b77dac | 2-9 | docs-reviewer, wire-reviewer | nit | doc | none | xtask/src/e2e.rs:1317 | MAX_GROWTH_MB doc names only a separation | unvalidated | none | c89c618 review round 2 fixes |
| 2026-10-28 | 2 | 8b77dac | 2-10 | docs-reviewer | nit | doc | none | docs/invariants.md:55 | TD-54 and TD-58 name MEM-4 only | unvalidated | none | c89c618 review round 2 fixes |
| 2026-10-28 | 2 | 8b77dac | 2-11 | docs-reviewer | nit | doc | none | docs/ARCHITECTURE.md:238 | "stem worker" and "moves the sinks" unclear | unvalidated | none | c89c618 review round 2 fixes |
| 2026-10-28 | 2 | 8b77dac | 2-12 | docs-reviewer | nit | doc | none | README.md:134 | wording and reflow | unvalidated | none | c89c618 review round 2 fixes |
| 2026-10-28 | 2 | 8b77dac | 2-13 | wire-reviewer | nit | weak-test | VERIFY-1 | xtask/src/e2e/old_session.rs:174 | larger-of-two reads untested | unvalidated | none | c89c618 review round 2 fixes |
| 2026-10-28 | 2 | 8b77dac | 2-14 | supply-chain-reviewer | nit | doc | none | engine/src/hpss.rs:863 | block-count comment and a dead arm | unvalidated | none | c89c618 review round 2 fixes |
| 2026-10-28 | 2 | 8b77dac | 2-15 | reviewer | nit | doc | none | app/src/lib.rs:822 | D2 drop route says "browser storage failed" | unvalidated | none | c89c618 review round 2 fixes |
| 2026-10-28 | 2 | 8b77dac | 2-16 | reviewer | nit | doc | DOC-1 | app/src/lib.rs:2032 | start_classic_drums says it blocks the page | unvalidated | none | c89c618 review round 2 fixes |
| 2026-10-28 | 2 | 8b77dac | 2-17 | reviewer | nit | bug | none | app/src/lib.rs:1528 | stale restore or pick sets the inbox status | unvalidated | none | c89c618 review round 2 fixes |
| 2026-10-28 | 2 | 8b77dac | 2-18 | cargo-mutants | should-fix | weak-test | VERIFY-1 | xtask/src/e2e/old_session.rs:89 | page_memory_mb -> Ok(Some(-1.0)) survives | confirmed | none | c89c618 review round 2 fixes (pure memory_mb with a native test) |
| 2026-10-28 | 2 | 8b77dac | 2-19 | cargo-mutants | should-fix | weak-test | VERIFY-1 | xtask/src/e2e/old_session.rs:89 | page_memory_mb -> Ok(None) survives | confirmed | none | c89c618 review round 2 fixes (pure memory_mb with a native test) |
| 2026-10-28 | 2 | 8b77dac | 2-20 | cargo-mutants | should-fix | weak-test | VERIFY-1 | xtask/src/e2e/old_session.rs:92 | / with % in page_memory_mb survives | confirmed | none | c89c618 review round 2 fixes (pure memory_mb with a native test) |
| 2026-10-28 | 2 | 8b77dac | 2-21 | cargo-mutants | should-fix | weak-test | VERIFY-1 | app/src/lib.rs:3391 | apply_inbox -> () survives | refuted | none | refuted: wasm32 only; now excluded in .cargo/mutants.toml |
| 2026-10-28 | 2 | 8b77dac | 2-22 | cargo-mutants | should-fix | weak-test | VERIFY-1 | xtask/src/e2e/old_session.rs:92 | / with * in page_memory_mb survives | confirmed | none | c89c618 review round 2 fixes (pure memory_mb with a native test) |
| 2026-10-28 | 2 | 8b77dac | 2-23 | cargo-mutants | should-fix | weak-test | VERIFY-1 | app/src/lib.rs:3459 | delete ! in apply_inbox survives | refuted | none | refuted: wasm32 only; now excluded |
| 2026-10-28 | 2 | 8b77dac | 2-24 | cargo-mutants | should-fix | weak-test | VERIFY-1 | app/src/lib.rs:2893 | post_separation -> () survives | refuted | none | refuted: wasm32 only; now excluded |
| 2026-10-28 | 2 | 8b77dac | 2-25 | cargo-mutants | should-fix | weak-test | VERIFY-1 | app/src/lib.rs:1702 | stored_stems_arrived -> () survives | refuted | none | refuted: wasm32 only; now excluded |
| 2026-10-28 | 2 | 8b77dac | 2-26 | cargo-mutants | should-fix | weak-test | VERIFY-1 | app/src/lib.rs:3413 | == with != in apply_inbox survives | refuted | none | refuted: wasm32 only; now excluded |
| 2026-10-28 | 2 | 8b77dac | 2-27 | cargo-mutants | should-fix | weak-test | VERIFY-1 | xtask/src/e2e/old_session.rs:144 | run -> Ok(()) survives | confirmed | none | browser only: CI's browser-tests job runs e2e old-session; TD-39 names it (c89c618) |
| 2026-10-28 | 2 | 8b77dac | 2-28 | cargo-mutants | should-fix | weak-test | VERIFY-1 | xtask/src/e2e/old_session.rs:89 | page_memory_mb -> Ok(Some(1.0)) survives | confirmed | none | c89c618 review round 2 fixes (pure memory_mb with a native test) |
| 2026-10-28 | 2 | 8b77dac | 2-29 | cargo-mutants | should-fix | weak-test | VERIFY-1 | xtask/src/e2e/old_session.rs:89 | page_memory_mb -> Ok(Some(0.0)) survives | confirmed | none | c89c618 review round 2 fixes (pure memory_mb with a native test) |
| 2026-10-28 | 3 | 20672b0 | 3-1 | reviewer | should-fix | bug | none | app/src/lib.rs:1819 | a classic-drums lane that arrives later drops a save in progress | confirmed | none | cb13653 review round 3 fixes |
| 2026-10-28 | 3 | 20672b0 | 3-2 | docs-reviewer, wire-reviewer | should-fix | doc | DOC-1 | docs/plans/active/stream-from-storage-m7.md:321 | Decision log and M5's D12 give the old memory line text | confirmed | none | cb13653 review round 3 fixes |
| 2026-10-28 | 3 | 20672b0 | 3-3 | supply-chain-reviewer | should-fix | other | VERIFY-1 | .cargo/mutants.toml:34 | eleven wasm32-only functions not excluded | pre-existing | none | cb13653 review round 3 fixes (excluded, with a test that checks the list) |
| 2026-10-28 | 3 | 20672b0 | 3-4 | memory-reviewer, docs-reviewer | nit | doc | none | engine/src/hpss.rs:891 | the bound's reason names the wrong quantity | unvalidated | none | cb13653 review round 3 fixes |
| 2026-10-28 | 3 | 20672b0 | 3-5 | wire-reviewer | nit | doc | PROTO-1 | protocol/tests/web.rs:656 | fixture keeps the old log text | unvalidated | none | cb13653 review round 3 fixes |
| 2026-10-28 | 3 | 20672b0 | 3-6 | docs-reviewer | nit | doc | none | docs/plans/active/stream-from-storage-m7.md:201 | step 9 bullet too long | unvalidated | none | cb13653 review round 3 fixes |
| 2026-10-28 | 3 | 20672b0 | 3-7 | docs-reviewer | nit | doc | none | README.md:134 | 26-word sentence; jargon in TD-39 | unvalidated | none | cb13653 review round 3 fixes |
| 2026-10-28 | 3 | 20672b0 | 3-8 | docs-reviewer | nit | weak-test | VERIFY-1 | app/src/lib.rs:1535 | restore error-branch token guard untested | unvalidated | none | cb13653 review round 3 fixes |
| 2026-10-28 | 3 | 20672b0 | 3-9 | cargo-mutants | should-fix | weak-test | VERIFY-1 | xtask/src/e2e/old_session.rs:153 | run -> Ok(()) survives | confirmed | none | cb13653 review round 3 fixes (xtask/tests/e2e_cli.rs) |
| 2026-10-28 | 4 | 8e6d245 | 4-1 | reviewer | should-fix | bug | VERIFY-1 | app/src/lib.rs:3856 | a save whose storage read fails writes silence and says "Saved" | confirmed | none | 91ceb7a review round 4 fixes |
| 2026-10-28 | 4 | 8e6d245 | 4-2 | supply-chain-reviewer, docs-reviewer | should-fix | weak-test | VERIFY-1 | xtask/tests/mutants_excludes.rs:9 | the exclusion scan misses wasm32-only impl methods | confirmed | none | 91ceb7a review round 4 fixes (test removed; the mutants run and validators catch it) |
| 2026-10-28 | 4 | 8e6d245 | 4-3 | supply-chain-reviewer, docs-reviewer | should-fix | weak-test | VERIFY-1 | xtask/tests/mutants_excludes.rs:40 | a name in any exclude_re group passes | confirmed | none | 91ceb7a review round 4 fixes (test removed; the mutants run and validators catch it) |
| 2026-10-28 | 4 | 8e6d245 | 4-4 | supply-chain-reviewer | should-fix | doc | none | .cargo/mutants.toml:28 | exclude_re does not remove struct-field mutants in cargo-mutants 27.1.0 | pre-existing | none | 91ceb7a review round 4 fixes (comment and TD-33) |
| 2026-10-28 | 4 | 8e6d245 | 4-5 | docs-reviewer | nit | doc | none | .cargo/mutants.toml:28 | unclear "each one"; no rewrap | unvalidated | none | 91ceb7a review round 4 fixes |
| 2026-10-28 | 4 | 8e6d245 | 4-6 | docs-reviewer | nit | doc | DOC-1 | xtask/src/e2e.rs:1472 | memory_budget doc leaves out two checks | unvalidated | none | 91ceb7a review round 4 fixes |
| 2026-10-28 | 4 | 8e6d245 | 4-7 | docs-reviewer | nit | doc | none | xtask/tests/e2e_cli.rs:7 | "no network" wording | unvalidated | none | 91ceb7a review round 4 fixes |
| 2026-10-28 | 5 | 0a3e70a | 5-1 | memory-reviewer | should-fix | bug | none | app/src/lib.rs:3846 | a failed save reads to the song's end before it stops | confirmed | none | d850a58 review round 5 fixes |
| 2026-10-28 | 5 | 0a3e70a | 5-2 | supply-chain-reviewer | should-fix | doc | VERIFY-1 | docs/tech-debt.md:180 | TD-33 says mutants::skip needs a crate; the cfg_attr form does not | confirmed | none | d850a58 review round 5 fixes (text only) |
| 2026-10-28 | 5 | 0a3e70a | 5-3 | docs-reviewer | should-fix | doc | DOC-1 | docs/tech-debt.md:181 | TD-33 list leaves out the playback files | pre-existing | none | d850a58 review round 5 fixes |
| 2026-10-28 | 5 | 0a3e70a | 5-4 | reviewer | should-fix | doc | DOC-1 | docs/tech-debt.md:399 | TD-53 says the save functions have no test | confirmed | none | d850a58 review round 5 fixes |
| 2026-10-28 | 5 | 0a3e70a | 5-5 | memory-reviewer, docs-reviewer, reviewer | nit | weak-test | VERIFY-1 | app/src/playback_tests.rs:1077 | the failed-save test covers FLAC only | unvalidated | none | d850a58 review round 5 fixes |
| 2026-10-28 | 5 | 0a3e70a | 5-6 | wire-reviewer, docs-reviewer | nit | doc | none | xtask/src/e2e.rs:1472 | memory_budget doc leaves out two checks | unvalidated | none | d850a58 review round 5 fixes |
| 2026-10-28 | 5 | 0a3e70a | 5-7 | reviewer | nit | doc | DOC-1 | docs/plans/active/stream-from-storage-m7.md | step 6a entry misstates the open's fallback | unvalidated | none | d850a58 review round 5 fixes |
| 2026-10-28 | 6 | f6131c3 | 6-1 | reviewer | should-fix | bug | none | app/src/store.rs:777 | save_stems keeps old stem lines under a new generation, so stale overviews can be kept | confirmed | none | c352052 review round 6 fixes |
| 2026-10-28 | 6 | f6131c3 | 6-2 | docs-reviewer | should-fix | bug | none | app/src/store.rs:1191 | the page's overview fallback reads to the end after a failed read | confirmed | none | c352052 review round 6 fixes |
| 2026-10-28 | 6 | f6131c3 | 6-3 | docs-reviewer | nit | doc | none | app/src/export.rs:70 | run_while doc misses the stop at a failure | unvalidated | none | c352052 review round 6 fixes |
| 2026-10-28 | 6 | f6131c3 | 6-4 | docs-reviewer | nit | doc | TD-21 | docs/plans/active/stream-from-storage-m7.md:382 | 5-7 correction names only an open | unvalidated | none | c352052 review round 6 fixes |
| 2026-10-28 | 6 | f6131c3 | 6-5 | docs-reviewer | nit | doc | none | docs/tech-debt.md:399 | descriptions over 25 words | unvalidated | none | c352052 review round 6 fixes |
| 2026-10-28 | 7 | fc6badf | 7-1 | wire-reviewer | should-fix | weak-test | VERIFY-1 | app/src/store.rs:1195 | the overview-job stop is tested only at its helper | confirmed | none | 95747b1 review round 7 fixes |
| 2026-10-28 | 7 | fc6badf | 7-2 | reviewer | should-fix | weak-test | VERIFY-1 | xtask/src/e2e/old_session.rs:96 | old-session cannot tell the worker's stem overviews from the page's fallback | confirmed | none | 95747b1 review round 7 fixes |
| 2026-10-28 | 7 | fc6badf | 7-3 | memory-reviewer | nit | other | MEM-1 | app/src/decode_tests.rs:1332 | six separate 60 s test buffers | unvalidated | none | 95747b1 review round 7 fixes |
| 2026-10-28 | 7 | fc6badf | 7-4 | wire-reviewer, docs-reviewer | nit | doc | TD-21 | xtask/src/e2e.rs:1475 | memory_budget doc list reads as complete; leaves out decoder wasm | unvalidated | none | 95747b1 review round 7 fixes |
| 2026-10-28 | 7 | fc6badf | 7-5 | reviewer | nit | doc | MEM-1 | docs/invariants.md | classic drums' in-memory output lane not named | unvalidated | none | 95747b1 review round 7 fixes |
| 2026-10-28 | 8 | 893a98c | 8-1 | memory-reviewer | should-fix | other | none | app/src/store.rs:1551 | stem_lanes runs the page's overview fallback a second time, also during a close | pre-existing | none | 8030869 review round 8 fixes |
| 2026-10-28 | 8 | 893a98c | 8-2 | memory-reviewer | nit | weak-test | VERIFY-1 | app/src/decode_tests.rs:1609 | only an upper bound on the fallback's reads | unvalidated | none | 8030869 review round 8 fixes |
| 2026-10-28 | 8 | 893a98c | 8-3 | wire-reviewer | nit | weak-test | VERIFY-1 | xtask/src/e2e/old_session.rs:157 | the fallback's log text is not pinned | unvalidated | none | 8030869 review round 8 fixes |
| 2026-10-28 | 8 | 893a98c | 8-4 | wire-reviewer, docs-reviewer | nit | doc | DOC-1 | README.md:134 | old-session line leaves out the new checks | unvalidated | none | 8030869 review round 8 fixes |
| 2026-10-28 | 8 | 893a98c | 8-5 | docs-reviewer | nit | doc | none | app/src/decode_tests.rs:1332 | "six channels" for twelve | unvalidated | none | 8030869 review round 8 fixes |
| 2026-10-28 | 8 | 893a98c | 8-6 | reviewer | nit | bug | none | app/src/lib.rs:2215 | two bounded input reads do not stop at the first failure | unvalidated | none | 8030869 review round 8 fixes |
| 2026-10-28 | 8 | 893a98c | 8-7 | cargo-mutants | should-fix | weak-test | VERIFY-1 | app/src/lane_store.rs:194 | OpfsStore::read -> () survives | refuted | none | refuted: wasm32 only |

## Rounds

| Round | Agents | Tokens | Wall minutes | Interventions | Late review commands | Approvals | Mutants | Baseline agents | Baseline tokens | Baseline wall minutes | Baseline interventions | Baseline approvals | Baseline found |
| - | - | - | - | - | - | - | - | - | - | - | - | - | - |
| 1 | 17 | 1,177,828 | 51.3 | 0 | 1 | 0 (auto mode; a fallback prompt is not counted) | mutants: none: tested 0 (caught 0, missed 0, unviable 0), timeouts 0, not tested 946; exit 4: the baseline failed | - | - | - | - | - | - |
| 2 | 12 | 909,011 | 56.5 | 0 | 1 | 0 (auto mode; a fallback prompt is not counted) | mutants: full: tested 35 (caught 19, missed 12, unviable 4), timeouts 0, not tested 0 | - | - | - | - | - | - |
| 3 | 9 | 721,147 | 49.8 | 0 | 1 | 0 (auto mode; a fallback prompt is not counted) | mutants: full: tested 15 (caught 14, missed 1, unviable 0), timeouts 0, not tested 0 | - | - | - | - | - | - |
| 4 | 9 | 652,442 | 40.7 | 0 | 1 | 0 (auto mode; a fallback prompt is not counted) | mutants: full: tested 2 (caught 2, missed 0, unviable 0), timeouts 0, not tested 0 | - | - | - | - | - | - |
| 5 | 9 | 594,815 | 29.4 | 0 | 1 | 0 (auto mode; a fallback prompt is not counted) | mutants: none: tested 0 (caught 0, missed 0, unviable 0), timeouts 0, not tested 0; no mutants in the diff | - | - | - | - | - | - |
| 6 | 6 | 413,425 | 37.6 | 0 | 1 | 0 (auto mode; a fallback prompt is not counted) | mutants: partial: tested 3 (caught 2, missed 0, unviable 1), timeouts 1, not tested 0; timeouts have no result (the timeout is ExportJob::step -> false, which hangs a test under the mutant only) | - | - | - | - | - | - |
| 7 | 6 | 521,129 | 39.2 | 0 | 1 | 0 (auto mode; a fallback prompt is not counted) | mutants: full: tested 4 (caught 0, missed 0, unviable 4), timeouts 0, not tested 0 | - | - | - | - | - | - |
| 8 | 6 | 528,102 | 42.8 | 0 | 1 | 0 (auto mode; a fallback prompt is not counted) | mutants: full: tested 12 (caught 11, missed 1, unviable 0), timeouts 0, not tested 0 | - | - | - | - | - | - |
