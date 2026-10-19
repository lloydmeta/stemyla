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

wgpu enforces these when a pipeline or bind group is created on a real adapter.  naga's validator does not check device limits.  So `every_kernel_fits_the_default_limits` (`roformer-gpu/src/lib.rs`) checks them without a GPU.  It parses each shader with naga 30.0.1 ([`front::wgsl::parse_str`](https://docs.rs/naga/30.0.1/naga/front/wgsl/fn.parse_str.html)), and counts as the WebGPU specification does ([compute pipeline creation](https://gpuweb.github.io/gpuweb/#dom-gpudevice-createcomputepipeline)):

* Workgroup size: each axis of `@workgroup_size` ([`EntryPoint::workgroup_size`](https://docs.rs/naga/30.0.1/naga/ir/struct.EntryPoint.html)), and the product x * y * z.
* Workgroup storage: the sum of `roundUp(16, size)` over each `var<workgroup>` that the entry point statically uses.  naga's validator gives the use ([`FunctionInfo`](https://docs.rs/naga/30.0.1/naga/valid/struct.FunctionInfo.html)), and [`proc::Layouter`](https://docs.rs/naga/30.0.1/naga/proc/struct.Layouter.html) gives the size.
* Buffers: every storage and uniform buffer that the module declares ([`GlobalVariable::space`](https://docs.rs/naga/30.0.1/naga/ir/struct.GlobalVariable.html)).  The specification counts the entries of the pipeline layout.  `Kernels::new` gives each module one layout with an entry for each binding that the module declares, so the counts are the same.

A shader that does not parse or validate, or that declares an `override`, fails the test, so no shader passes without a check.
