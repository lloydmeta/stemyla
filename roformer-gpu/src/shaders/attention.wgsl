// Fused attention: o = gate * softmax(alpha * q . k^T) v for one head of
// one sequence, without writing the scores to memory (an online softmax,
// as in FlashAttention, Dao et al. 2022).  Replaces the scores matmul, the
// softmax and the p.v matmul (and, with flag 1, the gate kernel).
//
// Layout as in net.rs: qkv is [tokens, row] with q, k and v of head h at
// columns h*64, k_off + h*64 and v_off + h*64; o is [tokens, o_row] with
// head h at h*64; gates is [tokens, gate_row].  Sequence s is tokens
// s*seq ..< (s+1)*seq.  Grid: (query blocks, heads, sequences).
//
// A workgroup of 256 threads takes ROWS = 256 / LANES queries, LANES
// threads per query.  Each thread holds its query's 64 q values in
// registers and, for every tile of BKV keys, scores BKV / LANES of them,
// then accumulates 64 / LANES output dimensions.  The per-thread code is
// generated with constant indices (the Q_LOAD and other placeholders; see `Attention`
// in lib.rs), so every backend can keep q and the outputs in registers.
// Workgroup memory: one K or V tile, one tile of probabilities and a
// reduction buffer, under 10 KiB.

struct Params {
  seq: u32, row: u32, k_off: u32, v_off: u32,
  o_row: u32, gate_row: u32, flags: u32, alpha: f32,
};

@group(0) @binding(0) var<storage, read> qkv: array<f32>;
@group(0) @binding(1) var<storage, read> gates: array<f32>;
@group(0) @binding(2) var<storage, read_write> o: array<f32>;
@group(0) @binding(3) var<uniform> p: Params;

const LANES: u32 = {LANES}u;
const ROWS: u32 = 256u / LANES;
const BKV: u32 = {BKV}u;
const D: u32 = 64u;
const SK: u32 = D + 1u;    // padded row of the K/V tile
const SP: u32 = BKV + 1u;  // padded row of the probability tile
const NEG: f32 = -3.0e38;  // finite "minus infinity": exp(NEG - m) is 0, never NaN

var<workgroup> kv: array<f32, {KV_LEN}>; // BKV * SK
var<workgroup> ps: array<f32, {PS_LEN}>; // ROWS * SP
var<workgroup> red: array<f32, 256>;

@compute @workgroup_size(256, 1, 1)
fn main(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) t: u32) {
  let r = t / LANES;
  let lane = t % LANES;
  let qi = wg.x * ROWS + r;
  let h = wg.y;
  let tok0 = wg.z * p.seq;
  let q_ok = qi < p.seq;
  let qtok = tok0 + min(qi, p.seq - 1u);
  let qbase = qtok * p.row + h * D;
{Q_LOAD}
  var m = NEG;
  var l = 0.0;
{O_DECLS}
  let tiles = (p.seq + BKV - 1u) / BKV;
  for (var kt = 0u; kt < tiles; kt++) {
    let j0 = kt * BKV;
    for (var e = t; e < BKV * D; e += 256u) {
      let j = e / D;
      let ok = j0 + j < p.seq;
      kv[j * SK + e % D] = select(0.0, qkv[select(0u, (tok0 + j0 + j) * p.row + p.k_off + h * D + e % D, ok)], ok);
    }
    workgroupBarrier();
{SCORES}
    red[t] = mx;
    workgroupBarrier();
    var tm = red[r * LANES];
    for (var i = 1u; i < LANES; i++) { tm = max(tm, red[r * LANES + i]); }
    let mn = max(m, tm);
    let sc = exp(m - mn);
    m = mn;
{PROBS}
    for (var e = t; e < BKV * D; e += 256u) {
      let j = e / D;
      let ok = j0 + j < p.seq;
      kv[j * SK + e % D] = select(0.0, qkv[select(0u, (tok0 + j0 + j) * p.row + p.v_off + h * D + e % D, ok)], ok);
    }
    workgroupBarrier();
{PV}
    workgroupBarrier();
  }
  red[t] = l;
  workgroupBarrier();
  var lt = 0.0;
  for (var i = 0u; i < LANES; i++) { lt += red[r * LANES + i]; }
  var g = 1.0;
  if ((p.flags & 1u) != 0u) { g = 1.0 / (1.0 + exp(-gates[qtok * p.gate_row + h])); }
  let f = g / lt;
  if (q_ok) {
    let ob = qtok * p.o_row + h * D + lane;
{STORE}
  }
}
