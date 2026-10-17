// Fused attention as two register-blocked products: the same operation,
// layout and parameters as attention.wgsl (o = gate * softmax(alpha q.k^T)
// v per head, online softmax, scores never in memory), organised like the
// interleaved matmul so that every inner loop does 16 multiply-adds for 8
// workgroup-memory reads.  attention.wgsl does about one per read, because
// each K value there serves a single query.
//
// A workgroup of 256 threads (16 x 16) takes 64 queries of one head of one
// sequence and walks the keys 64 at a time.  Thread (tx, ty) owns queries
// ty + 16i (i < 4) and, for each key tile, scores with keys tx + 16j, then
// output dimensions tx + 16j: interleaved ownership, so neighbouring
// threads read neighbouring floats.
//
//   scores: S = Q K^T over the head dimension, 16 dimensions at a time
//           (Q and K slices [16][64 + PAD] in `buf`);
//   softmax: row maxima through `red`, then p = exp(s - m), with the
//           running sum and the output rescaled (online softmax);
//   values: O += P V, 16 keys at a time (P [16][64 + PAD] and V [16][64]
//           reuse `buf`).
//
// Per-thread code with constant indices is generated (see `Attention` in
// lib.rs).  Workgroup memory: (2 * 16 * 66 + 64 * 16) floats, 12.3 KiB.

struct Params {
  seq: u32, row: u32, k_off: u32, v_off: u32,
  o_row: u32, gate_row: u32, flags: u32, alpha: f32,
};

@group(0) @binding(0) var<storage, read> qkv: array<f32>;
@group(0) @binding(1) var<storage, read> gates: array<f32>;
@group(0) @binding(2) var<storage, read_write> o: array<f32>;
@group(0) @binding(3) var<uniform> p: Params;

const D: u32 = 64u;
const S: u32 = 66u;      // padded row of a 16-row slice (64 + 32 / 16)
const B2: u32 = 1056u;   // second slice in buf: 16 * S
const NEG: f32 = -3.0e38;

var<workgroup> buf: array<f32, 2112>; // two slices of 16 x S
var<workgroup> red: array<f32, 1024>; // [64 queries][16 threads]

// Element e (0 .. 1024) of a [64 tokens][16 dims] slice starting at token
// t0 and column c0 (q or k of this head), zero past the sequence's end,
// stored transposed ([dim][token]) at `base` in buf.
fn stage_slice(e: u32, base: u32, tok0: u32, t0: u32, c0: u32) {
  let d = e % 16u;
  let i = e / 16u;
  let ok = t0 + i < p.seq;
  let v = qkv[select(0u, (tok0 + t0 + i) * p.row + c0 + d, ok)];
  buf[base + d * S + i] = select(0.0, v, ok);
}

// Element e (0 .. 1024) of 16 value rows [16 keys][64 dims] from key j0,
// stored row-major at B2.
fn stage_v(e: u32, tok0: u32, j0: u32, c0: u32) {
  let j = e / D;
  let d = e % D;
  let ok = j0 + j < p.seq;
  let v = qkv[select(0u, (tok0 + j0 + j) * p.row + c0 + d, ok)];
  buf[B2 + j * D + d] = select(0.0, v, ok);
}

@compute @workgroup_size(256, 1, 1)
fn main(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) t: u32) {
  let tx = t % 16u;
  let ty = t / 16u;
  let q0 = wg.x * 64u;
  let h = wg.y;
  let tok0 = wg.z * p.seq;
  let qc = h * D;
  let kc = p.k_off + h * D;
  let vc = p.v_off + h * D;
{DECLS}
  let tiles = (p.seq + 63u) / 64u;
  for (var kt = 0u; kt < tiles; kt++) {
    let j0 = kt * 64u;
{S_ZERO}
    for (var dc = 0u; dc < D; dc += 16u) {
      for (var l = 0u; l < 4u; l++) {
        stage_slice(t + l * 256u, 0u, tok0, q0, qc + dc);
        stage_slice(t + l * 256u, B2, tok0, j0, kc + dc);
      }
      workgroupBarrier();
      for (var kk = 0u; kk < 16u; kk++) {
        let ab = kk * S + ty;
        let bb = B2 + kk * S + tx;
{S_STEP}
      }
      workgroupBarrier();
    }
{SOFTMAX_1}
    workgroupBarrier();
{SOFTMAX_2}
{VALUES}
    workgroupBarrier();
  }
{FINISH}
}
