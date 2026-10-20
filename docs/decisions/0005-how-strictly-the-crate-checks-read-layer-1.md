# 0005: how strictly the crate checks read LAYER-1

Status: proposed

## Context

LAYER-1 says that crates depend only on crates in a lower layer of the layers table.  It also says that the page side and the worker side meet only through `stem-protocol`.  Its "Why" says that the page does not link the model or the GPU code.  `crates_depend_only_downwards` compares the dependencies between workspace crates with the table.  Two choices set how strictly the checks read the rule.

## Options

* The table: (x) exact.  A row lists each workspace crate that its crate depends on, apart from dev-dependencies on `test-support`, and no other.  The cost: each added or removed dependency between workspace crates changes the table.
* The table: (y) a ceiling.  A row may list a crate that its crate does not depend on.  The cost: the table can go stale, and the check needs another guard against reading nothing.
* The two sides: (a) the workspace crates that `app` and `stem-worker` reach through normal dependencies, for any target, have only `stem-protocol` in common.  The cost: a crate that both sides need later, such as `dsp` in the worker, needs a change to LAYER-1 and a record.  A native-only dependency of `app` on `roformer` also fails.
* The two sides: (b) `app` reaches none of `roformer`, `roformer-gpu` and `stem-worker`, as LAYER-1's "Why" says.  The two sides may share pure crates such as `dsp`, so LAYER-1 no longer says that they meet only through `stem-protocol`.  The cost: the check names crates, so a new model crate needs a change to the check.
* The two sides: (c) no second check.  The cost: a change to `app`'s row, with the dependency, lets `app` take `roformer`.  LAYER-1's sentence on the two sides stays prose only.

## Decision

(x) and (a), chosen by the maintainer on 2026-10-20.

## Consequences

* The layers table lists each dependency between workspace crates, apart from dev-dependencies on `test-support`, and no other.
* LAYER-1's sentence on the two sides gains: the workspace crates that `app` and `stem-worker` reach through normal dependencies, for any target, have only `stem-protocol` in common.
* `the_page_and_the_worker_meet_only_through_stem_protocol` follows the normal dependencies from `app` and `stem-worker`, and fails on any shared workspace crate but `stem-protocol`.
* `stem-protocol` takes no workspace crate as a normal dependency, because both sides reach what it takes.
* A crate that both sides need is a change to LAYER-1, with its own record.
* `LAYERS` in `xtask/src/rules.rs` fixes the five layers.  A change of layers changes the table, `LAYERS` and LAYER-1 in one commit.

## Revisit if

The native app separates songs, on a thread in its own process.  Today only the web app separates, in a Web Worker, and the native app does not.  A native app would run the model on that thread, so `app` would take `roformer` and `roformer-gpu` for its native target, and (a) fails that.  The split exists for web reasons: each WebAssembly module has its own memory, which never shrinks, and iOS stops a tab at 2 GB (MEM-4).  A native thread shares the process's memory, so a crate boundary gives it nothing.  Then read the shared crates in the wasm32 trees only, as `synth_brings_no_crate_into_the_web_app` reads them.  The check can use M4's `web_app_tree`.
