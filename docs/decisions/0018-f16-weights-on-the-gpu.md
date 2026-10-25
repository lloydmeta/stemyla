# 0018: the GPU keeps f16 weight matrices as f16, and computes in f32

Status: accepted (2026-10-25, by the maintainer)

## Context

The model file is f16 (`bs_roformer_sw.f16.safetensors`, 350 MB).  `roformer/src/weights.rs` converts each value to f32, and `roformer-gpu` uploads about 699 MB of f32 parameters.  Each f16 value converts to f32 exactly, so the GPU holds the f16 values at twice their size.  The tab's fixed memory is about 1720 MB of the 2048 MB that iOS allows on an iPhone 17 Pro (MEM-4, `plans/active/stream-from-storage.md`).  The maintainer chose on 2026-10-25 to fold this saving into that plan.

## Options

* Keep f32 weights.  No kernel changes, and no saving.
* Keep the f16 bits on the GPU, two in each `u32`, and unpack them to f32 in the shader with `unpack2x16float`.  The arithmetic stays f32.  WGSL has the built-in without the `shader-f16` feature (naga 26.0.0 validates it with no capabilities, checked on 2026-10-25).
* Compute in f16 with the `shader-f16` feature.  Sums in f16 lose precision and can overflow in attention, norms and softmax, so the stems would change.

## Decision

The second option.  Only the weight matrices of an f16 file are kept as f16.  Norms, biases and values that the upload scales stay f32.  An F32 or BF16 file keeps the f32 path: BF16 has a wider range than f16, and packing it as f16 can overflow.

## Consequences

* The parameters on the GPU fall from about 699 MB to about 350 MB.
* Each kernel gets the same f32 values as today, so its output can match today's bit for bit on the same GPU.  The tests check that.
* Every matrix kernel that reads weights changes.  GPU-1, GPU-2 and GPU-3 apply.
* Each weight load adds an unpack.  Half the memory traffic may make the kernels faster or slower; `?bench` measures it before the default changes.
