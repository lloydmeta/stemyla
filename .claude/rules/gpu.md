---
paths:
  - "roformer-gpu/**"
  - "roformer/src/**"
---

# GPU kernels and the reference network

Read `docs/ARCHITECTURE.md` (the model; running it on the GPU) before changing a kernel.  The rules are GPU-1, GPU-2, GPU-3 and MEM-3 in `docs/invariants.md`.

* Limits: every kernel fits `docs/references/webgpu-limits.md`.  Workgroup z is at most 64, not 256.  `every_kernel_fits_the_default_limits` checks each shader with naga.  A new generated source goes into `generated_sources`, and a new shader file into that test's list.  `sources_are_complete` finds any unfilled `{NAME}` placeholder; `workgroup_memory_within_defaults` checks a new tile against its formula.
* Evidence before a default: `cargo test -p roformer-gpu --release` on a real GPU (name the adapter in the PR; SwiftShader and lavapipe do not count), then `cargo run -p xtask -- e2e selftest --weights <model> --query mm=<kernel>`.  A test that printed "skipping" proves nothing (TD-8).
* Buffers: make every buffer through `Gpu`, never `device.create_buffer` directly, so it is a `GpuBuffer` that is destroyed when dropped.  Extend `roformer-gpu/tests/memory.rs` for a new allocation.
* Calibration: a change to how kernels are chosen bumps `CALIBRATION` in `roformer-gpu/src/bench.rs`.
* A change to the model's shapes also touches `roformer-gpu/src/bench.rs`, which hard-codes them.
