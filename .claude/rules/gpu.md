---
paths:
  - "roformer-gpu/**"
  - "roformer/src/**"
  - "roformer/clippy.toml"
  - "xtask/src/e2e/perf.rs"
  - "xtask/perf-baselines.json"
---

# GPU kernels and the reference network

Read `docs/ARCHITECTURE.md` (the model; running it on the GPU) before changing a kernel.  The rules are GPU-1, GPU-2, GPU-3 and MEM-3 in `docs/invariants.md`.

* Limits: every kernel fits `docs/references/webgpu-limits.md`.  Workgroup z is at most 64, not 256.  `every_kernel_fits_the_default_limits` checks each shader with naga.  A new generated source goes into `generated_sources`, and a new shader file into `SHADER_FILES` or `GENERATED_FROM`; `every_shader_file_is_checked` fails until it does.  Each matmul kernel has an f16 variant (`Tile::f16_source`); `generated_sources` and the `want` list name it (`i64-f16` and so on).  The checker validates with `CHECK_CAPABILITIES`: naga's default and `SHADER_FLOAT16_IN_FLOAT32` for `unpack2x16float`.  Add no other capability.  `sources_are_complete` finds any unfilled `{NAME}` placeholder; `workgroup_memory_within_defaults` checks a new tile against its formula.
* Workgroup memory: write each vec4 whole, from one thread.  WGSL lets a write to one component of a vector access the whole vector ([§8.5.1.3](https://www.w3.org/TR/WGSL/#component-reference-from-vector-memory-view)).  So threads that write different components of one vec4 race, and they did on Metal (`docs/ARCHITECTURE.md`, "What varies by GPU is measured, once").
* Pinned WGSL: `current_kernels_keep_their_wgsl` pins the FNV-1a hash of each current kernel and shader file.  Calibration does not protect other GPUs from a changed current kernel.  Put a new kernel's names (with `-f16`) in `NEW_KERNELS`.  When it lands, move them to `PINNED_WGSL` with their hashes.
* Evidence before a default: `STEMYLA_REQUIRE=gpu,f16 cargo test -p roformer-gpu --release` on a real GPU (`gpu` alone on an adapter without `SHADER_F16_IN_F32`) (name the adapter in the PR; SwiftShader and lavapipe do not count), then `cargo run -p xtask -- e2e selftest --weights <model> --query mm=<kernel>` for a matmul kernel, or `--query attn=<policy>` for an attention kernel.  Without `STEMYLA_REQUIRE`, a test that cannot open a GPU prints "skipping" and passes, which proves nothing (TD-8).  A test that skips calls `test_support::skip`; `every_skip_can_be_required` fails one that does not.
* Buffers: make every buffer with `Gpu::buffer`, `Gpu::upload`, `Gpu::upload_weight` or `Gpu::upload_f16`, so it is a `GpuBuffer` that is destroyed when dropped.  Clippy fails `StagingBelt` and `DownloadBuffer`, and `wgpu::Device::create_buffer` and `wgpu::util::DeviceExt::create_buffer_init` outside `GpuBuffer`'s constructors (MEM-3).  Extend `roformer-gpu/tests/memory.rs` for a new allocation.
* Copies: Clippy fails `slice::to_vec` in `app`, `stem-worker`, `decode-worker`, `stem-protocol`, `engine`, `roformer`, `roformer-gpu`, `synth` and `m4a`, and `to_vec` on a `Uint8Array` or a `Float32Array` in the first four (MEM-1).  A needed copy gets `#[expect(clippy::disallowed_methods, reason = "MEM-1: why")]` on its statement, field or match arm.
* Calibration: a change to how kernels are chosen bumps `CALIBRATION` in `roformer-gpu/src/bench.rs`.
* Speed: `e2e perf` gates the lower quartile of the chunk times against `xtask/perf-baselines.json`.  Change a baseline only with `--update-baselines` from 5 runs or more.  A raise of the bound also needs `--raise`, with any browser version.  A kernel lands only with a shown gain, and `e2e perf --base-dist` shows no loss (`docs/references/local-browser-testing.md`).  An attention kernel's gain is on the whole-step figure, as calibration times it, not on the attention-only lines.
* A change to the model's shapes also touches `roformer-gpu/src/bench.rs`, which hard-codes them.
