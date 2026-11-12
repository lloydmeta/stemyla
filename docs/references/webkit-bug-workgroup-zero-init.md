# WebKit bug text: workgroup memory set to zero by one thread

This is a bug text for the maintainer to file at [bugs.webkit.org](https://bugs.webkit.org), component WebGPU.  It is not filed yet (2026-11-13).  M3 of `plans/active/gpu-speed.md` measured it (`plans/active/gpu-speed-m3.md`, Steps 1, 4 and 5).  `browser-quirks.md` gives the quirk and what the code here does about it.

Title: WebGPU: workgroup memory zero-initialisation runs on one thread, byte by byte, and makes a tiled matrix product 1.29 times slower

## Summary

WebKit's WGSL compiler sets each `var<workgroup>` to zero at the start of a compute entry point.  Thread 0 alone does it, one byte at a time, then all threads wait at a barrier.  Chrome spreads the stores over the threads of the workgroup.

For 10 KiB of workgroup memory on an Apple M2 Pro, the zero-initialisation costs about 0.19 us of GPU time for each workgroup in Safari.  In Chrome it costs about 0.006 us.  The test case below takes 23 times as long in Safari as in Chrome.  In a tiled matrix product (2,064 workgroups, K = 256), WebKit's form makes the kernel 1.29 times slower.  With WebKit's bounds clamps as well, it is 1.49 times slower.

## Environment

Safari 27.2 (22625.2.8.11.9), macOS 27.2.1 (26B131), Apple M2 Pro (19-core GPU, Apple8 family).  Chrome 157.0.8103.51 on the same Mac, for comparison.  Checked 2026-11-13.

## Source

WebKit `main` at `5447ffbd5817eb8a1de9c0540bb8a6fe37db555d`, read on 2026-11-13.  Safari 27.2 ships from a release branch, which can differ from `main`.

* GlobalVariableRewriter.cpp in WebKit's Source/WebGPU/WGSL, lines 2552 to 2594 (`initializeVariables`): `if (local_invocation_index == 0) { ... }`, then `workgroupBarrier()` ([source](https://github.com/WebKit/WebKit/blob/5447ffbd5817eb8a1de9c0540bb8a6fe37db555d/Source/WebGPU/WGSL/GlobalVariableRewriter.cpp#L2552-L2594)).
* The same file, lines 2666 to 2786 (`storeInitialValue`): a loop over each array element calls `__wgslZeroWorkgroupVar` ([source](https://github.com/WebKit/WebKit/blob/5447ffbd5817eb8a1de9c0540bb8a6fe37db555d/Source/WebGPU/WGSL/GlobalVariableRewriter.cpp#L2666-L2786)).
* MetalFunctionWriter.cpp in WebKit's Source/WebGPU/WGSL/Metal, lines 1081 to 1095 ([source](https://github.com/WebKit/WebKit/blob/5447ffbd5817eb8a1de9c0540bb8a6fe37db555d/Source/WebGPU/WGSL/Metal/MetalFunctionWriter.cpp#L1081-L1095)):

  ```
  template<typename T>
  static void __wgslZeroWorkgroupVar(threadgroup T& v)
  {
      threadgroup char* p = (threadgroup char*)(&v);
      for (uint i = 0u; i < sizeof(T); i++)
          p[i] = 0;
  }
  ```

* For comparison, Tint (Chrome) spreads the stores over the workgroup.  The source is zero_init_workgroup_memory.cc in Dawn's src/tint/lang/core/ir/transform, lines 136 to 185, at `85b7901759827252adf542adf7a1d2858515b12e` ([source](https://dawn.googlesource.com/dawn/+/85b7901759827252adf542adf7a1d2858515b12e/src/tint/lang/core/ir/transform/zero_init_workgroup_memory.cc#136)).

## Test case

The WGSL below is dispatched with 2,064 workgroups (`dispatchWorkgroups(24, 86, 1)`).  Each command buffer holds 16 dispatches.

```wgsl
struct Params { n: u32, a: u32, b: u32, c: u32, d: u32, e: u32, f: u32, g: u32 };
@group(0) @binding(0) var<storage, read> x: array<f32>;
@group(0) @binding(1) var<storage, read> unused: array<f32>;
@group(0) @binding(2) var<storage, read_write> y: array<f32>;
@group(0) @binding(3) var<uniform> p: Params;   // p.a = 1
var<workgroup> w: array<vec4<f32>, 640>;        // 10 KiB

@compute @workgroup_size(256, 1, 1)
fn main(@builtin(local_invocation_index) t: u32) {
  if (t == 0u) { w[p.a] = vec4<f32>(1.0); }
  workgroupBarrier();
  let v = w[(t * 7u) % 640u].x;
  if (v == 12345.678) { y[0] = v; }
}
```

The reference is the same kernel without `w`.  It keeps the barrier, and sets `v` to `f32(t) + f32(p.a)`.

## Results

Wall time for each dispatch, median of 5 rounds, in 2 pages of each browser.  Each range is from the start and the end of each page:

| | Safari 27.2 | Chrome 157 |
| - | - | - |
| With 10 KiB of workgroup memory | 408 to 410 us | 17.8 to 17.9 us |
| No workgroup memory | 10.9 to 11.1 us | 6.3 us |

A real kernel: Chrome's MSL for a 64 x 64 tiled matrix product (10 KiB of workgroup memory, 256 threads).  It was compiled and timed through Metal directly on the same Mac.  The only change was WebKit's zero-initialisation in place of Tint's.  Each dispatch at 5,456 x 1,536 x 256 then took 2.512 ms in place of 1.948 ms (1.29 times).  With WebKit's bounds clamps as well, it took 2.905 ms (1.49 times).  The whole product is 1.76 times slower in Safari than in Chrome.

## Expected

The zero-initialisation spreads the stores over the threads of the workgroup, with 4-byte or 16-byte stores, as Tint does.  A later improvement can leave it out where the shader writes each element before any read.

## Possible fix

In `storeInitialValue`, emit a loop outside `if (local_invocation_index == 0)`.  The loop starts at `local_invocation_index` and steps by the workgroup size.  It sets each element to zero with one store of the element type, not one byte at a time.
