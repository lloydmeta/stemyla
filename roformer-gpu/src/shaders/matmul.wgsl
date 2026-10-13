// C = act(alpha * A·B + bias) (+ C when accumulating), for a batch of
// strided matrices.  64x64 output tile per workgroup, 16x16 threads, each
// computing 4x4 outputs; 16-deep K tiles in workgroup memory (8.1 KiB).
// Stays within WebGPU's default limits (256 invocations, 16 KiB).
//
// Element addressing (all strides in f32 elements):
//   A(i, k) = a[a_off + bo*a_bo + bi*a_bi + i*a_si + k*a_sk]
//   B(k, j) = b[b_off + bo*b_bo + bi*b_bi + k*b_sk + j*b_sj]
//   C(i, j) = c[c_off + bo*c_bo + bi*c_bi + i*c_si + j*c_sj]
//   bias(j) = bias[bias_off + bo*bias_bo + j]
// where the batch index z = bo*inner + bi.

struct Params {
  m: u32, n: u32, k: u32, inner: u32,
  a_off: u32, a_si: u32, a_sk: u32, a_bo: u32,
  a_bi: u32, b_off: u32, b_sk: u32, b_sj: u32,
  b_bo: u32, b_bi: u32, c_off: u32, c_si: u32,
  c_sj: u32, c_bo: u32, c_bi: u32, bias_off: u32,
  bias_bo: u32, flags: u32, alpha: f32, _pad: u32,
};

@group(0) @binding(0) var<storage, read> a: array<f32>;
@group(0) @binding(1) var<storage, read> b: array<f32>;
@group(0) @binding(2) var<storage, read_write> c: array<f32>;
@group(0) @binding(3) var<storage, read> bias: array<f32>;
@group(0) @binding(4) var<uniform> p: Params;

const HAS_BIAS: u32 = 1u;
const ACCUMULATE: u32 = 2u;
const ACT_SHIFT: u32 = 2u; // 0 none, 1 gelu, 2 tanh

var<workgroup> as_: array<f32, 1040>; // [kk * 65 + row]; 65 avoids bank conflicts
var<workgroup> bs_: array<f32, 1040>; // [kk * 65 + col]

// erf by Abramowitz and Stegun 7.1.26 (absolute error below 1.5e-7).
fn erf_(x: f32) -> f32 {
  let s = sign(x);
  let ax = abs(x);
  let t = 1.0 / (1.0 + 0.3275911 * ax);
  let y = 1.0 - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t + 0.254829592) * t * exp(-ax * ax);
  return s * y;
}

// tanh from one exp of a non-positive argument: no overflow for large |x|
// (some drivers' built-in tanh returns NaN there).
fn tanh_(x: f32) -> f32 {
  let e = exp(-2.0 * abs(x));
  return sign(x) * (1.0 - e) / (1.0 + e);
}

@compute @workgroup_size(16, 16, 1)
fn main(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_id) lid: vec3<u32>) {
  let bo = wg.z / p.inner;
  let bi = wg.z % p.inner;
  let row0 = wg.y * 64u;
  let col0 = wg.x * 64u;
  let tid = lid.y * 16u + lid.x;
  let a_base = p.a_off + bo * p.a_bo + bi * p.a_bi;
  let b_base = p.b_off + bo * p.b_bo + bi * p.b_bi;
  // Load along whichever axis is contiguous in memory.
  let a_k_fast = p.a_sk == 1u;
  let b_k_fast = p.b_sk == 1u;

  var acc: array<f32, 16>;
  for (var i = 0u; i < 16u; i++) { acc[i] = 0.0; }

  for (var k0 = 0u; k0 < p.k; k0 += 16u) {
    for (var l = 0u; l < 4u; l++) {
      let e = tid + l * 256u;
      var kk: u32; var r: u32;
      if (a_k_fast) { kk = e % 16u; r = e / 16u; } else { kk = e / 64u; r = e % 64u; }
      let gi = row0 + r;
      let gk = k0 + kk;
      var av = 0.0;
      if (gi < p.m && gk < p.k) { av = a[a_base + gi * p.a_si + gk * p.a_sk]; }
      as_[kk * 65u + r] = av;

      var kb: u32; var cc: u32;
      if (b_k_fast) { kb = e % 16u; cc = e / 16u; } else { kb = e / 64u; cc = e % 64u; }
      let gj = col0 + cc;
      let gkb = k0 + kb;
      var bv = 0.0;
      if (gj < p.n && gkb < p.k) { bv = b[b_base + gkb * p.b_sk + gj * p.b_sj]; }
      bs_[kb * 65u + cc] = bv;
    }
    workgroupBarrier();
    for (var kk = 0u; kk < 16u; kk++) {
      var bvals: array<f32, 4>;
      for (var j = 0u; j < 4u; j++) { bvals[j] = bs_[kk * 65u + lid.x + 16u * j]; }
      for (var i = 0u; i < 4u; i++) {
        let av = as_[kk * 65u + lid.y + 16u * i];
        for (var j = 0u; j < 4u; j++) { acc[i * 4u + j] += av * bvals[j]; }
      }
    }
    workgroupBarrier();
  }

  let act = (p.flags >> ACT_SHIFT) & 3u;
  let c_base = p.c_off + bo * p.c_bo + bi * p.c_bi;
  for (var i = 0u; i < 4u; i++) {
    let row = row0 + lid.y + 16u * i;
    if (row >= p.m) { continue; }
    for (var j = 0u; j < 4u; j++) {
      let col = col0 + lid.x + 16u * j;
      if (col >= p.n) { continue; }
      var v = acc[i * 4u + j] * p.alpha;
      if ((p.flags & HAS_BIAS) != 0u) { v += bias[p.bias_off + bo * p.bias_bo + col]; }
      if (act == 1u) { v = 0.5 * v * (1.0 + erf_(v * 0.70710678118654752)); }
      else if (act == 2u) { v = tanh_(v); }
      let idx = c_base + row * p.c_si + col * p.c_sj;
      if ((p.flags & ACCUMULATE) != 0u) { v += c[idx]; }
      c[idx] = v;
    }
  }
}
