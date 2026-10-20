---
paths:
  - "dsp/**"
---

# dsp

`dsp/` is a first reading exercise in Rust (DSP-1 in `docs/invariants.md`):

* Use only `&[f32]`, `Vec<f32>`, `usize`, `for`, `while`, `if` and indexing.
* Do not use iterators, closures or `swap`; write the loop out.
* Write `x = x + y`, `len() > 0` and an `if` for each bound of a clamp.  Do not write a compound assignment (`x += y` and the others), `is_empty` or `clamp`.
* Fix a clippy lint with DSP-1's constructs.  If every fix needs another construct, add the lint to the `#![expect]` list in `dsp/src/lib.rs`.  Give a reason that cites DSP-1.  The lint must fire in lib code, not only in tests; otherwise fix it.  If DSP-1 does not name the form, write a `proposed` decision record, and ask the maintainer.
* Add no dependency, not even a dev-dependency.
* Avoid allocating in a function that the audio callback calls.  `double_speed` allocates today (TD-15 in `docs/tech-debt.md`).
