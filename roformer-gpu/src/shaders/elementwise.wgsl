// Small per-element kernels, one thread per output element.
struct Params { count: u32, groups_x: u32, a: u32, b: u32, c: u32, d: u32, e: u32, f: u32 };
@group(0) @binding(0) var<storage, read> src: array<f32>;
@group(0) @binding(1) var<storage, read> aux: array<f32>;
@group(0) @binding(2) var<storage, read_write> dst: array<f32>;
@group(0) @binding(3) var<uniform> p: Params;

fn index(wg: vec3<u32>, lid: vec3<u32>) -> u32 { return (wg.y * p.groups_x + wg.x) * 256u + lid.x; }

// Rotary embedding in place on q and k of a qkv buffer [tokens, 1536]
// (interleaved pairs).  One thread per (token, q or k, head, pair).
// p.a = sequence length; aux = [seq, 32] cos then [seq, 32] sin.
@compute @workgroup_size(256, 1, 1)
fn rope(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_id) lid: vec3<u32>) {
  let i = index(wg, lid);
  if (i >= p.count) { return; }
  let pair = i % 32u;
  let head = (i / 32u) % 8u;
  let which = (i / 256u) % 2u;
  let token = i / 512u;
  let pos = token % p.a;
  let col = which * 512u + head * 64u + pair * 2u;
  let x0 = dst[token * 1536u + col];
  let x1 = dst[token * 1536u + col + 1u];
  let cs = aux[pos * 32u + pair];
  let sn = aux[p.a * 32u + pos * 32u + pair];
  dst[token * 1536u + col] = x0 * cs - x1 * sn;
  dst[token * 1536u + col + 1u] = x1 * cs + x0 * sn;
}

// o[t, h*64 + d] *= sigmoid(gates[t, h]); o is [tokens, 512], gates [tokens, 8].
@compute @workgroup_size(256, 1, 1)
fn gate(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_id) lid: vec3<u32>) {
  let i = index(wg, lid);
  if (i >= p.count) { return; }
  let t = i / 512u;
  let h = (i % 512u) / 64u;
  dst[i] = dst[i] * (1.0 / (1.0 + exp(-aux[t * 8u + h])));
}

// [A, B, 256] -> [B, A, 256]; p.a = A, p.b = B.
@compute @workgroup_size(256, 1, 1)
fn transpose3(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_id) lid: vec3<u32>) {
  let i = index(wg, lid);
  if (i >= p.count) { return; }
  let d = i % 256u;
  let rest = i / 256u;
  let ai = rest % p.a;   // output is [B, A, 256]: rest = b * A + a
  let bi = rest / p.a;
  dst[i] = src[(ai * p.b + bi) * 256u + d];
}

// Gated linear unit, scattered into the mask: src is [n, frames, 2w] for
// n bands of width w; dst[t * 4100 + off + j * w + c] = a * sigmoid(b).
// p.a = w, p.b = frames, p.c = off (in dst).
@compute @workgroup_size(256, 1, 1)
fn glu(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_id) lid: vec3<u32>) {
  let i = index(wg, lid);
  if (i >= p.count) { return; }
  let w = p.a;
  let c = i % w;
  let t = (i / w) % p.b;
  let j = i / (w * p.b);
  let row = (j * p.b + t) * 2u * w;
  let av = src[row + c];
  let bv = src[row + w + c];
  dst[t * 4100u + p.c + j * w + c] = av * (1.0 / (1.0 + exp(-bv)));
}
