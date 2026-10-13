// y = x / max(|x|, 1e-12) * g for rows of 256 (g already holds gamma*sqrt(256)).
struct Params { rows: u32, groups_x: u32, x_off: u32, y_off: u32 };
@group(0) @binding(0) var<storage, read> x: array<f32>;
@group(0) @binding(1) var<storage, read> g: array<f32>;
@group(0) @binding(2) var<storage, read_write> y: array<f32>;
@group(0) @binding(3) var<uniform> p: Params;
var<workgroup> red: array<f32, 256>;

@compute @workgroup_size(256, 1, 1)
fn main(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_id) lid: vec3<u32>) {
  let row = wg.y * p.groups_x + wg.x;
  if (row >= p.rows) { return; }
  let t = lid.x;
  let v = x[p.x_off + row * 256u + t];
  red[t] = v * v;
  workgroupBarrier();
  for (var s = 128u; s > 0u; s >>= 1u) {
    if (t < s) { red[t] = red[t] + red[t + s]; }
    workgroupBarrier();
  }
  let n = max(sqrt(red[0]), 1e-12);
  y[p.y_off + row * 256u + t] = v / n * g[t];
}
