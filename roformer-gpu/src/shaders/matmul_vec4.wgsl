// matmul_interleaved.wgsl with vector memory traffic: the same
// C = act(alpha * A.B + bias) (+ C when accumulating), the same Params,
// the same interleaved ownership of C (thread (tx, ty) owns rows
// ty + TY*i and columns tx + TX*j) and the same epilogue, but A and B are
// read from storage and from workgroup memory four floats at a time.
//
// It runs only where every vec4 is whole and aligned (`MatMul::vec4_ok` in
// lib.rs): A and B both K-contiguous (a_sk = b_sk = 1, every nn.Linear
// operand and the attention scores), K a multiple of 4, every other A and
// B offset and stride a multiple of 4, and both buffers a whole number of
// vec4s.  Then a vec4 at an in-range K offset is wholly inside the matrix.
// Elsewhere the driver runs the fallback kernel instead.
//
// Tiles keep K contiguous: as_ is [row][k / 4] and bs_ is [col][k / 4],
// rows padded by PADV vec4s, so staging copies vec4s from storage to
// workgroup memory unchanged, and each step of the inner loop reads TM + TN
// vec4s for 4 * TM * TN multiply-adds (the scalar kernel: TM + TN floats
// for TM * TN).  Each accumulator still adds its products in K order, one
// fma at a time, as the scalar kernel does.
//
// Workgroup memory: (BM + BN) * (BK + 4 * PADV) * 4 bytes (10 KiB for
// 64 x 64 with BK 16).

struct Params {
  m: u32, n: u32, k: u32, inner: u32,
  a_off: u32, a_si: u32, a_sk: u32, a_bo: u32,
  a_bi: u32, b_off: u32, b_sk: u32, b_sj: u32,
  b_bo: u32, b_bi: u32, c_off: u32, c_si: u32,
  c_sj: u32, c_bo: u32, c_bi: u32, bias_off: u32,
  bias_bo: u32, flags: u32, alpha: f32, _pad: u32,
};

@group(0) @binding(0) var<storage, read> a: array<vec4<f32>>;
@group(0) @binding(1) var<storage, read> b: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read_write> c: array<f32>;
@group(0) @binding(3) var<storage, read> bias: array<f32>;
@group(0) @binding(4) var<uniform> p: Params;

const BM: u32 = {BM}u;
const BN: u32 = {BN}u;
const BK: u32 = {BK}u;
const TX: u32 = {TX}u;   // threads along n; TY along m; product 256
const TY: u32 = {TY}u;
const KV: u32 = BK / 4u;          // vec4s per tile row
const SV: u32 = KV + {PADV}u;     // tile row stride in vec4s

const HAS_BIAS: u32 = 1u;
const ACCUMULATE: u32 = 2u;
const ACT_SHIFT: u32 = 2u; // 0 none, 1 gelu, 2 tanh

var<workgroup> as_: array<vec4<f32>, {SA_LEN}>; // BM * SV, [row][k / 4]
var<workgroup> bs_: array<vec4<f32>, {SB_LEN}>; // BN * SV, [col][k / 4]

// Vec4 e (0 .. BM*KV) of the A tile at K offset k0, zero outside the
// matrix; consecutive e run along K.  Branch-free (select): out of range,
// the load reads vec4 0 and discards it.
fn a_get(e: u32, k0: u32, row0: u32, base: u32) -> vec4<f32> {
  let gi = row0 + e / KV;
  let gk = k0 + 4u * (e % KV);
  let ok = gi < p.m && gk < p.k;
  let v = a[select(0u, (base + gi * p.a_si + gk) / 4u, ok)];
  return select(vec4<f32>(0.0), v, ok);
}
fn b_get(e: u32, k0: u32, col0: u32, base: u32) -> vec4<f32> {
  let gj = col0 + e / KV;
  let gk = k0 + 4u * (e % KV);
  let ok = gj < p.n && gk < p.k;
  let v = b[select(0u, (base + gj * p.b_sj + gk) / 4u, ok)];
  return select(vec4<f32>(0.0), v, ok);
}
// Slot of vec4 e in as_ or bs_.
fn slot(e: u32) -> u32 {
  return (e / KV) * SV + e % KV;
}

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

{ACC_DECLS}

  for (var k0 = 0u; k0 < p.k; k0 += BK) {
{STAGE}
    workgroupBarrier();
    for (var kv = 0u; kv < KV; kv++) {
      let ab = ty * SV + kv;
      let bb = tx * SV + kv;
{COMPUTE}
    }
    workgroupBarrier();
  }

  let r0 = row0 + ty;
  let c0 = col0 + tx;
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
