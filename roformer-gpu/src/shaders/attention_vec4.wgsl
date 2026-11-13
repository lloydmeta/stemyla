// Fused attention with vec4 reads of workgroup memory (`vfused`): the same
// operation, layout, parameters and grid as attention_tiled.wgsl, and the
// same sum order, so it gives the same bits on a GPU that rounds each fma
// alike.  It differs in how it stages and reads workgroup memory:
//
//   scores: S = Q K^T, 16 dimensions at a time.  Q and K slices are
//           [64 tokens][16 dims] with a pad of one vec4 (R = 5 vec4s for
//           each token).  Each 4 dimensions take 4 Q and 4 K vec4 reads
//           for 64 multiply-adds; attention_tiled.wgsl takes 32 scalar
//           reads.
//   softmax: thread (tx, ty) writes the maxima of its 4 rows as one vec4
//           at [ty][tx] of the first 256 vec4s of `buf`, and at the end
//           its 4 row sums.  No separate `red`.
//   values: O += P V, 16 keys at a time.  P is [16 keys][16 ty], each a
//           vec4 of the queries ty + 16i, and V is [16 keys][16 tx], each
//           a vec4 of the dimensions tx + 16j.  Each key takes 1 P and 1 V
//           vec4 read for 16 multiply-adds.
//
// Each write to workgroup memory is a whole vec4 from one thread: on
// Metal, threads that write different components of one vec4 race.  Each
// score sums its dimensions in order, each output its keys in order, and
// each row sum its threads in order, one operation at a time.
//
// A workgroup of 256 threads (16 x 16) takes 64 queries of one head of one
// sequence.  Thread (tx, ty) owns queries ty + 16i (i < 4), keys or
// dimensions tx + 16j (j < 4).  Per-thread code with constant indices is
// generated (`attention_vec4_source` in lib.rs).  Workgroup memory:
// 2 x 64 x (16 + 4) floats, 10 KiB.

struct Params {
  seq: u32, row: u32, k_off: u32, v_off: u32,
  o_row: u32, gate_row: u32, flags: u32, alpha: f32,
};

@group(0) @binding(0) var<storage, read> qkv: array<f32>;
@group(0) @binding(1) var<storage, read> gates: array<f32>;
@group(0) @binding(2) var<storage, read_write> o: array<f32>;
@group(0) @binding(3) var<uniform> p: Params;

const D: u32 = 64u;
const R: u32 = 5u;       // vec4s in a padded row of 16 floats
const B2: u32 = 320u;    // the second half of buf: 64 * R
const NEG: f32 = -3.0e38;

var<workgroup> buf: array<vec4<f32>, 640>; // two halves of [64][R]

// Four floats of qkv from `at`, as a vec4; zero when `ok` is false.
fn load4(at: u32, ok: bool) -> vec4<f32> {
  let a = select(0u, at, ok);
  let v = vec4<f32>(qkv[a], qkv[a + 1u], qkv[a + 2u], qkv[a + 3u]);
  return select(vec4<f32>(0.0), v, ok);
}

// Thread t stages dimensions 4 (t % 4) .. + 4 of token t / 4 of the slice
// that starts at token t0 and column c0 (q or k of this head), zero past
// the sequence's end, as row t / 4 of a [64][R] half of buf at `base`.
fn stage_slice(t: u32, base: u32, tok0: u32, t0: u32, c0: u32) {
  let i = t / 4u;
  let dv = t % 4u;
  let ok = t0 + i < p.seq;
  buf[base + i * R + dv] = load4((tok0 + t0 + i) * p.row + c0 + 4u * dv, ok);
}

// The value of key j at column c, zero past the sequence's end.
fn value(tok0: u32, j: u32, c: u32) -> f32 {
  let ok = j < p.seq;
  return select(0.0, qkv[select(0u, (tok0 + j) * p.row + c, ok)], ok);
}

// Thread t stages the values of key j0 + t / 16 at the dimensions
// t % 16 + 16j (j < 4), as one vec4 at [t / 16][t % 16] from B2.
fn stage_v(t: u32, tok0: u32, j0: u32, c0: u32) {
  let j = j0 + t / 16u;
  let c = c0 + t % 16u;
  buf[B2 + t] = vec4<f32>(
    value(tok0, j, c), value(tok0, j, c + 16u), value(tok0, j, c + 32u), value(tok0, j, c + 48u));
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
      stage_slice(t, 0u, tok0, q0, qc + dc);
      stage_slice(t, B2, tok0, j0, kc + dc);
      workgroupBarrier();
      for (var dd = 0u; dd < 4u; dd++) {
{S_STEP}
      }
      workgroupBarrier();
    }
{SOFTMAX_1}
    workgroupBarrier();
{SOFTMAX_2}
    workgroupBarrier();
{VALUES}
    workgroupBarrier();
  }
{FINISH}
}
