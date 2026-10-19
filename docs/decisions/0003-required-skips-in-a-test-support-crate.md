# 0003: STEMYLA_REQUIRE turns a skipped test into a failure, through one helper in a test-support crate

Status: proposed (2026-10-19; the maintainer chose option (a) in `plans/active/harness-phase-2.md`, M3)

## Context

Stable libtest has no run-time skip.  A test that has no GPU adapter, no `timestamp-query` or no PyTorch fixture returns early and reports `ok`, and `cargo test` hides its "skipping" line.  So a run that checked nothing looks like a pass, which VERIFY-1 forbids as evidence (TD-8 in `tech-debt.md`).  The skips are in the tests of `roformer` and `roformer-gpu`.

## Options

* (a) A workspace crate `test-support`, a dev-dependency only, with one parser and its tests.
* (b) One file that each test target includes with `#[path]`.  Its tests run again in each target, and a source folder that is not a crate is new here.
* (c) A copy of the helper in each crate.  Two copies can drift, and no test sees it.
* (d) A module behind a `testing` feature in `roformer`.  Test code is then in a shipped crate's source.

## Decision

Option (a).  Every skip calls `test_support::skip(need, why)`.  The environment variable `STEMYLA_REQUIRE` lists the needs a run claims (`gpu`, `timestamps`, `fixtures`).  A skip of a listed need panics with VERIFY-1 and the fix.  An unknown word panics too.  `timestamps` implies `gpu`.

## Consequences

* The workspace has a tenth crate, in a new lowest layer.  Any crate but `dsp` (DSP-1) may take it as a dev-dependency, and none as a normal dependency (LAYER-1).
* Evidence on a GPU is `STEMYLA_REQUIRE=gpu cargo test -p roformer-gpu --release`.  The GPU's PyTorch comparison needs `gpu,fixtures`.
* The evidence holds only when a person or an agent sets the variable.  Phase 4's evaluator is to set it (TD-8).
* `xtask`'s check `every_skip_can_be_required` fails a skip message that does not go through the helper.
