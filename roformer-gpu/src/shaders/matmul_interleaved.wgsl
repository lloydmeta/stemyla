// Register-tiled matmul with interleaved ownership: the same operation,
// addressing and parameters as matmul.wgsl, with the tile shape filled in
// by the driver (`{BM}` and so on are replaced before compiling; see
// `Tile` in lib.rs).
//
// A workgroup of 256 threads computes a BM x BN tile of C.  Thread
// (tx, ty) owns rows ty + TY*i and columns tx + TX*j (TX = BN / TN,
// TY = BM / TM), as in matmul.wgsl, rather than a TM x TN block as in
// matmul_tiled.wgsl.  So in the inner loop neighbouring threads read
// neighbouring floats of the B tile (no shared-memory bank conflicts), and
// in the epilogue they write neighbouring columns of C.  The accumulators
// are named scalars with constant indices (generated), as in
// matmul_tiled.wgsl, so every backend can keep them in registers.
//
// Tiles are scalar f32 arrays, rows padded by PAD floats.  When K is the
// contiguous axis (every nn.Linear operand), 32 consecutive threads store
// 32 / BK rows of BK values; PAD = 32 / BK spreads those over all 32 banks.
//
// With PREFETCH, each thread holds its share of the next K tile in
// registers while the current one is used, so global-memory latency
// overlaps arithmetic without a second tile in workgroup memory.
//
// Shapes in use stay within WebGPU's defaults: 256 invocations and
// BK * (BM + BN + 2 * PAD) * 4 bytes of workgroup memory (at most 8.3 KiB).

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
const TX: u32 = {TX}u;   // threads along n; TY along m; product 256
const TY: u32 = {TY}u;
const SA: u32 = BM + {PAD}u;
const SB: u32 = BN + {PAD}u;

const HAS_BIAS: u32 = 1u;
const ACCUMULATE: u32 = 2u;
const ACT_SHIFT: u32 = 2u; // 0 none, 1 gelu, 2 tanh

var<workgroup> as_: array<f32, {SA_LEN}>; // BK * SA, [kk][row]
var<workgroup> bs_: array<f32, {SB_LEN}>; // BK * SB, [kk][col]

// Element e (0 .. BM*BK) of the A tile at K offset k0: its value from
// global memory (zero outside the matrix) and its slot in as_.  Loads run
// along whichever axis is contiguous in memory.  Branch-free (select), so
// the unrolled copies in the prefetch variants stay straight-line code:
// with if/else here, SwiftShader took minutes and gigabytes to compile
// them.  Out of range, the load reads element 0 and discards it.
fn a_get(e: u32, k0: u32, row0: u32, base: u32) -> f32 {
  let fast = p.a_sk == 1u;
  let gi = row0 + select(e % BM, e / BK, fast);
  let gk = k0 + select(e / BM, e % BK, fast);
  let ok = gi < p.m && gk < p.k;
  let v = a[select(0u, base + gi * p.a_si + gk * p.a_sk, ok)];
  return select(0.0, v, ok);
}
fn a_slot(e: u32) -> u32 {
  let fast = p.a_sk == 1u;
  return select(e / BM, e % BK, fast) * SA + select(e % BM, e / BK, fast);
}
fn b_get(e: u32, k0: u32, col0: u32, base: u32) -> f32 {
  let fast = p.b_sk == 1u;
  let gj = col0 + select(e % BN, e / BK, fast);
  let gk = k0 + select(e / BN, e % BK, fast);
  let ok = gj < p.n && gk < p.k;
  let v = b[select(0u, base + gk * p.b_sk + gj * p.b_sj, ok)];
  return select(0.0, v, ok);
}
fn b_slot(e: u32) -> u32 {
  let fast = p.b_sk == 1u;
  return select(e / BN, e % BK, fast) * SB + select(e % BN, e / BK, fast);
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
{PREFETCH_FIRST}

  for (var k0 = 0u; k0 < p.k; k0 += BK) {
{STAGE}
    workgroupBarrier();
{PREFETCH_NEXT}
    for (var kk = 0u; kk < BK; kk++) {
      let ab = kk * SA + ty;
      let bb = kk * SB + tx;
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
