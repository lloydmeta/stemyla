---
paths:
  - "engine/**"
  - "app/src/**"
  - "app/clippy.toml"
  - "worker/**"
---

# Memory and the audio callback

The page and the worker run in WebAssembly, whose memory never shrinks, in a tab an iPhone kills at 2 GB.  The rules are MEM-1 to MEM-5 and RT-1 in `docs/invariants.md`; the design is in `docs/ARCHITECTURE.md` (memory on phones).

* Before adding `to_vec`, `clone` on a `Vec<f32>`, `Vec::from`, `collect` or `copy_to` of audio, model or file bytes: is the buffer song- or model-sized?  If so, borrow it or share an `engine::Samples`; a copy needs a stated reason and a test that pins the sharing.  Clippy fails `slice::to_vec` in `app`, `stem-worker`, `stem-protocol`, `engine`, `roformer` and `roformer-gpu`, and `to_vec` on a `Uint8Array` or a `Float32Array` in the first three (MEM-1).  A needed copy gets `#[expect(clippy::disallowed_methods, reason = "MEM-1: why")]` on its statement, field or match arm.
* A typed array sent between page and worker goes in the transfer list, and the receiver does not copy it again.
* Every OPFS write gets a whole array, never a view (`docs/references/browser-quirks.md`).
* Nothing on the audio callback's path (`engine/src/slot.rs` `render_interleaved` and below) waits, locks without `try_lock`, logs, drops a `Player` or `Song`, or allocates after warm-up.  `engine/tests/realtime.rs` counts allocations; size a new buffer in `Player::with_config` or `Stretcher::warm_up`.
* Known breaches are TD-12 to TD-14 in `docs/tech-debt.md`, and TD-17 is a risk; do not copy their pattern.
