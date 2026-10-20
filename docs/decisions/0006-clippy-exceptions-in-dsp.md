# 0006: how dsp meets clippy

Status: accepted (2026-10-20, by the maintainer)

## Context

`dsp/src/lib.rs` had `#![allow(clippy::all)]`, so no clippy lint ran on `dsp`.  Four lints fire on its beginner code: `assign_op_pattern`, `len_zero`, `manual_clamp` and `manual_swap`.  DSP-1 forbids only the fix of `manual_swap`.

## Options

* (x) Keep all four, each as an `expect` with a reason.  DSP-1 names the beginner forms and forbids their shortcuts, so each reason cites DSP-1's text.  The gain: the code that a beginner reads stays as it is, and M7's allowlist needs no compound assignment, `is_empty` or `clamp`.  The cost: four exceptions, and DSP-1 gains a sentence that forbids `+=`, `-=`, `is_empty` and `clamp`.
* (y) Fix three, and keep only `manual_swap`.  The gain: one exception, which DSP-1's text already forces, and no new sentence in DSP-1.  The cost: the beginner text changes in 7 files, and M7's allowlist must permit `+=`, `-=`, `is_empty` and `clamp`.

The maintainer also weighed (z): fix only `assign_op_pattern`, because the Rust Book teaches `+=` and `-=` in its first loops.

## Decision

(x), chosen by the maintainer on 2026-10-20.

## Consequences

* DSP-1 forbids compound assignment (`+=`, `-=`, `*=` and the others), `is_empty` and `clamp` in `dsp`.  `assign_op_pattern` fires on `x = x * y` as on `x = x + y`, so the ban covers every operator.
* `dsp/src/lib.rs` expects the four lints; every other lint keeps its workspace level.
* A new lint in `dsp` is fixed with DSP-1's constructs, or gets an `expect` whose reason cites DSP-1.
* M7's allowlist fails the forbidden shortcuts.
