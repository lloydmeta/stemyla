---
paths:
  - "dsp/**"
---

# dsp

`dsp/` is a first reading exercise in Rust (DSP-1 in `docs/invariants.md`):

* Use only `&[f32]`, `Vec<f32>`, `usize`, `for`, `while`, `if` and indexing.
* Do not use iterators, closures or `swap`; write the loop out.
* Add no dependency, not even a dev-dependency.
* A function that the audio callback calls must not allocate (TD-15 in `docs/tech-debt.md`).
