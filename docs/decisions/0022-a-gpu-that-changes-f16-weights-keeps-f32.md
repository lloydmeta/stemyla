# 0022: a GPU whose f16 kernels do not give the f32 output keeps f32 weights

Status: proposed (2026-10-30, by the agent, in M9 of `plans/active/stream-from-storage.md`)

## Context

Decision 0018 keeps the weight matrices of an f16 model file as f16 on the GPU, two values in each `u32`.  Each matmul kernel gets an f16 variant that unpacks each weight with `unpack2x16float` and computes in f32.  So on a correct GPU, the stems do not change.

WGSL lets an implementation flush to zero the results of `unpack2x16float` ([WGSL, 15.7.2 Differences from IEEE-754](https://www.w3.org/TR/WGSL/#differences-from-ieee754)).  The model file has about 1.04 million f16 subnormal values in its weight matrices (0.6%).  A GPU that flushes them gives other stems.  A compiler bug in one variant can also give other stems.  Decision 0018 does not say what such a GPU does, and the parent plan leaves a flush to the maintainer.

## Options

* Use f16 weights on every GPU that can compile the variants.  The stems can change on some GPUs, and no check shows it.
* Check the variants when the GPU opens, and keep f32 weights on a GPU where a variant does not give its f32 kernel's output bit for bit.  The stems stay the same on every GPU.  Such a GPU needs about 350 MB more for the parameters.
* Unpack f16 in integer arithmetic, which WGSL does not let a GPU flush.  It changes decision 0018's means, and costs more instructions in each kernel.

## Decision

The second option.  `Gpu::check_kernels` runs the f16 variant of each kernel that passed its f32 check.  It runs the f32 kernel on the same values as f32, and compares the two outputs bit for bit, with `+0` and `-0` equal.  The values include f16 subnormals, and one case makes each output a single subnormal weight, so a flush shows.  A variant whose shader or pipeline the browser or driver rejects is caught in an error scope, and counts as a failure.  `Gpu::f16_ok` is true only when every such variant passes.  Else the weights stay f32, and the check gives a line that names the kernel, how many values differ and the largest difference.  An adapter without `DownlevelFlags::SHADER_F16_IN_F32` never compiles the variants and keeps f32 weights.

## Consequences

* The stems of the f16 path equal the stems of the f32 path with the same kernels, on every GPU that uses it.
* A GPU that flushes f16 subnormals, or that has a bug in a variant, needs about 350 MB more on the GPU.  An iPhone that flushes may not fit the budget of MEM-4.  M10 of the parent plan checks the iPhone.
* The start-up check compiles and runs four more kernels, on small products.
* If a GPU that matters flushes, the third option is the next step.  It needs the maintainer, since it changes decision 0018's means.
