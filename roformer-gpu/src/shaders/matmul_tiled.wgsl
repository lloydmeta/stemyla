// Register-tiled matmul: the same operation, addressing and parameters as
// matmul.wgsl, with the tile shape filled in by the driver (`{BM}` and so
// on are replaced before compiling; see `Tile` in lib.rs).
//
// A workgroup of 256 threads computes a BM x BN tile of C.  Thread
// (tx, ty) owns the TM x TN block at rows ty*TM.., columns tx*TN.., held
// as vec4s, so each step of the inner loop reads TM + TN values from
// workgroup memory (as vec4s) for TM * TN multiply-adds.  K advances BK at
// a time through workgroup memory; A is stored transposed ([kk][row]) and
// both tiles' rows are padded by 4 floats, which spreads the stores over
// the memory banks.
//
// Shapes in use stay within WebGPU's defaults: 256 invocations and at most
// 2 * BK * (max(BM, BN) + 4) * 4 bytes = 8.25 KiB of workgroup memory.

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

const BM: u32 = {BM}u;
const BN: u32 = {BN}u;
const BK: u32 = {BK}u;
const TM: u32 = {TM}u;
const TN: u32 = {TN}u;
const TX: u32 = BN / TN;     // threads along n; BM / TM along m; product 256
const SA: u32 = BM + 4u;     // padded row length of the A tile
const SB: u32 = BN + 4u;
const LA: u32 = BM * BK / 256u; // A elements each thread loads per K tile
const LB: u32 = BN * BK / 256u;

const HAS_BIAS: u32 = 1u;
const ACCUMULATE: u32 = 2u;
const ACT_SHIFT: u32 = 2u; // 0 none, 1 gelu, 2 tanh

// Scalar arrays: every thread stores single floats; reads build vec4s.
// (Storing into single components of vec4 array elements gave wrong
// results on a Radeon Pro 5500M under Chrome and Metal, though not on
// SwiftShader; most likely each store became a read-modify-write of the
// whole vec4, racing with the neighbouring threads.)
var<workgroup> as_: array<f32, {SA_LEN}>; // BK * SA
var<workgroup> bs_: array<f32, {SB_LEN}>; // BK * SB

fn a4(i: u32) -> vec4<f32> { return vec4<f32>(as_[i], as_[i + 1u], as_[i + 2u], as_[i + 3u]); }
fn b4(i: u32) -> vec4<f32> { return vec4<f32>(bs_[i], bs_[i + 1u], bs_[i + 2u], bs_[i + 3u]); }

// erf by Abramowitz and Stegun 7.1.26 (absolute error below 1.5e-7).
fn erf_(x: f32) -> f32 {
  let s = sign(x);
  let ax = abs(x);
  let t = 1.0 / (1.0 + 0.3275911 * ax);
  let y = 1.0 - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t + 0.254829592) * t * exp(-ax * ax);
  return s * y;
}

// tanh from one exp of a non-positive argument: no overflow for large |x|.
fn tanh_(x: f32) -> f32 {
  let e = exp(-2.0 * abs(x));
  return sign(x) * (1.0 - e) / (1.0 + e);
}

@compute @workgroup_size(256, 1, 1)
fn main(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) tid: u32) {
  let bo = wg.z / p.inner;
  let bi = wg.z % p.inner;
  let row0 = wg.y * BM;
  let col0 = wg.x * BN;
  let tx = tid % TX;
  let ty = tid / TX;
  let a_base = p.a_off + bo * p.a_bo + bi * p.a_bi;
  let b_base = p.b_off + bo * p.b_bo + bi * p.b_bi;
  // Load along whichever axis is contiguous in memory.
  let a_k_fast = p.a_sk == 1u;
  let b_k_fast = p.b_sk == 1u;

  // Accumulators as named vec4 variables (generated): constant indices
  // only, so every backend can keep them in registers.
{ACC_DECLS}

  for (var k0 = 0u; k0 < p.k; k0 += BK) {
    for (var l = 0u; l < LA; l++) {
      let e = tid + l * 256u;
      var kk: u32; var r: u32;
      if (a_k_fast) { kk = e % BK; r = e / BK; } else { kk = e / BM; r = e % BM; }
      let gi = row0 + r;
      let gk = k0 + kk;
      var v = 0.0;
      if (gi < p.m && gk < p.k) { v = a[a_base + gi * p.a_si + gk * p.a_sk]; }
      as_[kk * SA + r] = v;
    }
    for (var l = 0u; l < LB; l++) {
      let e = tid + l * 256u;
      var kk: u32; var cc: u32;
      if (b_k_fast) { kk = e % BK; cc = e / BK; } else { kk = e / BN; cc = e % BN; }
      let gj = col0 + cc;
      let gk = k0 + kk;
      var v = 0.0;
      if (gj < p.n && gk < p.k) { v = b[b_base + gk * p.b_sk + gj * p.b_sj]; }
      bs_[kk * SB + cc] = v;
    }
    workgroupBarrier();
    for (var kk = 0u; kk < BK; kk++) {
      let ab = kk * SA + ty * TM;
      let bb = kk * SB + tx * TN;
{COMPUTE}
    }
    workgroupBarrier();
  }

  let r0 = row0 + ty * TM;
  let c0 = col0 + tx * TN;
  let ob = p.c_off + bo * p.c_bo + bi * p.c_bi;
  let bias_base = p.bias_off + bo * p.bias_bo;
{EPILOGUE}
}

// One output element: scale, bias, activation, accumulate, bounds.
fn store(row: u32, col: u32, ob: u32, bias_base: u32, x: f32) {
  if (row >= p.m || col >= p.n) { return; }
  var v = x * p.alpha;
  if ((p.flags & HAS_BIAS) != 0u) { v += bias[bias_base + col]; }
  let act = (p.flags >> ACT_SHIFT) & 3u;
  if (act == 1u) { v = 0.5 * v * (1.0 + erf_(v * 0.70710678118654752)); }
  else if (act == 2u) { v = tanh_(v); }
  let idx = ob + row * p.c_si + col * p.c_sj;
  if ((p.flags & ACCUMULATE) != 0u) { v += c[idx]; }
  c[idx] = v;
}
