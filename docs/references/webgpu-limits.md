# WebGPU default limits

The limits every kernel must fit (GPU-1).  `roformer-gpu` requests the device with `Limits::defaults()`, so these are the most it may use on any GPU.  Values are the WebGPU specification's defaults ([WebGPU, limits](https://gpuweb.github.io/gpuweb/#limits)), which `wgpu::Limits::default()` matches ([wgpu 30.0.1 `Limits`](https://docs.rs/wgpu/30.0.1/wgpu/struct.Limits.html)); checked 2026-10-19.

| Limit | Default | Where it bites here |
| - | - | - |
| `maxComputeInvocationsPerWorkgroup` | 256 | every `@workgroup_size`, whose x * y * z product must not exceed it |
| `maxComputeWorkgroupSizeX`, `maxComputeWorkgroupSizeY` | 256 | |
| `maxComputeWorkgroupSizeZ` | 64 | not 256 like x and y |
| `maxComputeWorkgroupStorageSize` | 16,384 bytes | the shared-memory tiles in the matmul and attention kernels |
| `maxComputeWorkgroupsPerDimension` | 65,535 | dispatch counts; `grid()` splits large counts across x and y |
| `maxStorageBuffersPerShaderStage` | 8 | kernels bind at most 4 today |
| `maxUniformBuffersPerShaderStage` | 12 | |
| `maxStorageBufferBindingSize` | 128 MiB | the step budget keeps every binding below it |
| `maxBufferSize` | 256 MiB | |
| `maxBindGroups` | 4 | |

wgpu enforces these when a pipeline or bind group is created on a real adapter; naga's validator does not check device limits, so a static test must assert them itself (TD-1).
