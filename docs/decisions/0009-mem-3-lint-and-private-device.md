# 0009: how MEM-3's lint and the device keep buffers in `GpuBuffer`

Status: accepted (2026-10-21, by the maintainer)

## Context

Harness phase 2, M13, makes clippy's `disallowed-methods` fail each way to make a GPU buffer in `roformer-gpu` outside `GpuBuffer` (MEM-3).  wgpu 30.0.1's web backend does nothing when a buffer, a query set or a texture is dropped.  So a buffer that is not a `GpuBuffer` stays on the GPU until JavaScript collects it, and Safari collects late.  On an iPhone, this can push the tab over its 2 GB limit (MEM-4).  Two wgpu helpers, `StagingBelt` and `DownloadBuffer::read_buffer`, make buffers that nothing destroys.  `Gpu`'s `device` field was public, so `stem-worker` could make a buffer on it, where no MEM-3 lint applies.

## Options

* (a) `wgpu::Device::create_buffer` and `wgpu::util::DeviceExt::create_buffer_init` only.  `Gpu`'s device stays public.
* (b) (a), and the two helpers in three paths (`StagingBelt::new`, `StagingBelt::new_with_buffer_usages`, `DownloadBuffer::read_buffer`).  `Gpu`'s device and `Gpu::from_device` become private, with `Gpu::limits` and `Gpu::features` for the readers.
* (c) (b), and textures and query sets: `create_texture`, `create_texture_with_data` and `create_query_set`.

## Decision

(b), chosen by the maintainer on 2026-10-21, as the agent recommended.

## Consequences

* `roformer-gpu/clippy.toml` lists the five paths.  Each has a line in the canary `mem3_lint_is_on`, since clippy passes a path that does not resolve.
* `GpuBuffer::new` and `GpuBuffer::with_contents` are the only sites of the first two paths.
* `Gpu`'s device and `Gpu::from_device` are private.  Doctests on `Gpu` fail if either becomes public.  No later crate can wrap a device that it already has without a change to `roformer-gpu`.
* Textures and query sets stay out of the lint, as a gap (TD-27).  `roformer-gpu` makes no texture, and one query set of 1 KiB for each profiling device.
