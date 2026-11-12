# 0031: tier push runs the perf gate when asked

Status: proposed (2026-11-12, by the agent, in Step 4 of `plans/completed/gpu-speed-m2.md`; amends decision 0026 and GIT-2)

## Context

Decision 0026 says that `xtask check --tier push` runs no `e2e` scenario, because CI runs the browser tests.  M2 of `plans/active/gpu-speed.md` adds `xtask e2e perf`, which separates the 1-minute synthetic song and gates the lower quartile of the chunk times against `xtask/perf-baselines.json`.  The gate needs a real GPU, the model file and an unlocked screen.  CI has no GPU and no model (ASSET-1), so CI cannot run it.  M3 to M6 change the kernels and the worker, and a later change must not make separation slower without a failure.

## Options

* A: no gate in `check`.  An agent runs `e2e perf --gate` by hand.  It is easy to forget, and nothing records that it ran.
* B: tier `push` runs the gate when `STEMYLA_PERF_WEIGHTS` names the model, and the changed paths can change the speed.  Else it prints a skip line with the reason.
* C: tier `commit` runs the gate too.  Most commits of M3 to M6 touch strict paths, so each commit would wait 3 to 4 minutes and need an unlocked screen.

## Decision

B.

* `with_perf_gate` in `xtask/src/check.rs` adds the gate only when tier `push` is asked, not when a strict path moves a commit up to tier `push`.
* The gate runs when `STEMYLA_PERF_WEIGHTS` is set and a path can change the speed.  Such a path is in `roformer/`, `roformer-gpu/` or `worker/`, or is a workspace file, or no path is known.
* Then two lines come after the Trunk build: `cargo run -p xtask -- e2e perf --gate --browser chrome --weights <model>`, then the same with `--browser safari`.
* With those paths and no `STEMYLA_PERF_WEIGHTS`: one skip line that says that the perf gate did not run.  Else one skip line with the reason of the paths.
* An agent sets `STEMYLA_PERF_WEIGHTS` (and `CHROMEDRIVER`) for the push check before a review round and before a push, on the maintainer's M2 Pro, whose baselines the file holds.
* A browser that cannot run fails its own line: `e2e` fails with no `CHROMEDRIVER`, and on a locked screen.

## Consequences

* The push check takes about 3 minutes more: one run in each browser (2.8 minutes measured on 2026-11-12).  The screen stays unlocked, and the Mac stays unused, during the Safari run (`references/local-browser-testing.md`).
* A machine without the variable prints the skip line, and runs as before.  CI does not set it.
* GIT-2 in `invariants.md`, `CLAUDE.md` and `.claude/rules/gpu.md` change in the same commit (DOC-1).
* The gate's tolerance comes from at least 5 runs in each browser, and is never under 10 % (`xtask/src/e2e/perf.rs`).
* The gate reads the chunk lower quartile, not the chunk median.  Interference only adds time to a chunk, and a slower kernel adds time to each chunk.  On 5 runs in each browser, the median gave a tolerance of 22 % (Chrome) and 28 % (Safari), and the lower quartile 10 % and 11 % (`plans/completed/gpu-speed-m2.md`, Step 5).
