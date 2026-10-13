// In-place softmax over contiguous rows: one workgroup of 256 per row.
struct Params { rows: u32, len: u32, groups_x: u32, _pad: u32 };
@group(0) @binding(0) var<storage, read_write> x: array<f32>;
@group(0) @binding(1) var<uniform> p: Params;
var<workgroup> red: array<f32, 256>;

@compute @workgroup_size(256, 1, 1)
fn main(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_id) lid: vec3<u32>) {
  let row = wg.y * p.groups_x + wg.x;
  if (row >= p.rows) { return; }  // uniform across the workgroup
  let base = row * p.len;
  let t = lid.x;
  var m = bitcast<f32>(0xff7fffffu);  // -FLT_MAX (Tint rejects a decimal literal that rounds past it)
  for (var i = t; i < p.len; i += 256u) { m = max(m, x[base + i]); }
  red[t] = m;
  workgroupBarrier();
  for (var s = 128u; s > 0u; s >>= 1u) {
    if (t < s) { red[t] = max(red[t], red[t + s]); }
    workgroupBarrier();
  }
  let mx = red[0];
  workgroupBarrier();
  var sum = 0.0;
  for (var i = t; i < p.len; i += 256u) {
    let e = exp(x[base + i] - mx);
    x[base + i] = e;
    sum += e;
  }
  red[t] = sum;
  workgroupBarrier();
  for (var s = 128u; s > 0u; s >>= 1u) {
    if (t < s) { red[t] = red[t] + red[t + s]; }
    workgroupBarrier();
  }
  let inv = 1.0 / red[0];
  for (var i = t; i < p.len; i += 256u) { x[base + i] = x[base + i] * inv; }
}
