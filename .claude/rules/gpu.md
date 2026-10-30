---
paths:
  - "roformer-gpu/**"
  - "roformer/src/**"
  - "roformer/clippy.toml"
---

# GPU kernels and the reference network

Read `docs/ARCHITECTURE.md` (the model; running it on the GPU) before changing a kernel.  The rules are GPU-1, GPU-2, GPU-3 and MEM-3 in `docs/invariants.md`.

* Limits: every kernel fits `docs/references/webgpu-limits.md`.  Workgroup z is at most 64, not 256.  `every_kernel_fits_the_default_limits` checks each shader with naga.  A new generated source goes into `generated_sources`, and a new shader file into `SHADER_FILES` or `GENERATED_FROM`; `every_shader_file_is_checked` fails until it does.  Each matmul kernel has an f16 variant (`Tile::f16_source`); `generated_sources` and the `want` list name it (`i64-f16` and so on).  The checker validates with `CHECK_CAPABILITIES`: naga's default and `SHADER_FLOAT16_IN_FLOAT32` for `unpack2x16float`.  Add no other capability.  `sources_are_complete` finds any unfilled `{NAME}` placeholder; `workgroup_memory_within_defaults` checks a new tile against its formula.
* Evidence before a default: `STEMYLA_REQUIRE=gpu,f16 cargo test -p roformer-gpu --release` on a real GPU (`gpu` alone on an adapter without `SHADER_F16_IN_F32`) (name the adapter in the PR; SwiftShader and lavapipe do not count), then `cargo run -p xtask -- e2e selftest --weights <model> --query mm=<kernel>`.  Without `STEMYLA_REQUIRE`, a test that cannot open a GPU prints "skipping" and passes, which proves nothing (TD-8).  A test that skips calls `test_support::skip`; `every_skip_can_be_required` fails one that does not.
* Buffers: make every buffer with `Gpu::buffer`, `Gpu::upload`, `Gpu::upload_weight` or `Gpu::upload_f16`, so it is a `GpuBuffer` that is destroyed when dropped.  Clippy fails `StagingBelt` and `DownloadBuffer`, and `wgpu::Device::create_buffer` and `wgpu::util::DeviceExt::create_buffer_init` outside `GpuBuffer`'s constructors (MEM-3).  Extend `roformer-gpu/tests/memory.rs` for a new allocation.
* Copies: Clippy fails `slice::to_vec` in `app`, `stem-worker`, `decode-worker`, `stem-protocol`, `engine`, `roformer` and `roformer-gpu`, and `to_vec` on a `Uint8Array` or a `Float32Array` in the first four (MEM-1).  A needed copy gets `#[expect(clippy::disallowed_methods, reason = "MEM-1: why")]` on its statement, field or match arm.
* Calibration: a change to how kernels are chosen bumps `CALIBRATION` in `roformer-gpu/src/bench.rs`.
* A change to the model's shapes also touches `roformer-gpu/src/bench.rs`, which hard-codes them.
