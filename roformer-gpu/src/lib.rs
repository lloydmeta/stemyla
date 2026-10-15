//! BS-RoFormer SW on WebGPU with hand-written WGSL kernels.
//!
//! The browser runs the network with these kernels rather than a general
//! tensor library: Burn's WebGPU backend (CubeCL) gave wrong results and
//! GPU timeouts on a Radeon Pro 5500M under Metal.  Every kernel stays
//! within WebGPU's default limits (256 invocations per workgroup, 16 KiB
//! of workgroup memory, 128 MiB per storage binding, 65,535 workgroups per
//! dimension), and the device is requested with exactly those limits, so a
//! run that works on one GPU asks nothing more of another.
//!
//! [`Gpu`] owns the device, the compiled kernels and the small uniform
//! buffer the kernels read their parameters from.  [`Rec`] records one
//! step: a list of dispatches submitted together as one command buffer.
//! [`net::Net`] holds the weights on the GPU and [`net::Job`] runs one
//! chunk as a list of [`net::Step`]s, so the caller can wait for the GPU
//! after each step and keep every command buffer short.

pub mod bench;
pub mod net;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::num::NonZeroU64;
use std::sync::{Arc, Mutex};

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

/// Bytes reserved for one dispatch's parameters (WebGPU's default
/// `minUniformBufferOffsetAlignment`).
const PARAM_STRIDE: u64 = 256;
/// Dispatches one [`Rec`] may hold.
const MAX_DISPATCHES: u64 = 64;

/// How a matmul kernel's threads share out the output tile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// The original hand-written kernel, `matmul.wgsl` (64 x 64, each
    /// thread every 16th row and column, loop-indexed accumulators).
    Original,
    /// `matmul_tiled.wgsl`: each thread a `tm` x `tn` block, accumulators
    /// in named `vec4`s.
    Blocked,
    /// `matmul_interleaved.wgsl`: each thread every `bm / tm`-th row and
    /// `bn / tn`-th column, accumulators in named scalars; rows of the
    /// workgroup tiles padded by `pad` floats.
    Interleaved { pad: usize, prefetch: bool },
}

/// A matmul kernel's tile shape: a workgroup of 256 threads computes a
/// `bm` x `bn` block of the output, each thread `tm` x `tn` of it, stepping
/// through K `bk` at a time.  The first entry of [`TILES`] is the original
/// hand-written kernel (`matmul.wgsl`); the others fill these numbers into
/// a template chosen by `layout`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tile {
    pub name: &'static str,
    pub bm: usize,
    pub bn: usize,
    pub bk: usize,
    pub tm: usize,
    pub tn: usize,
    pub layout: Layout,
}

impl Tile {
    fn source(&self) -> String {
        match self.layout {
            Layout::Original => include_str!("shaders/matmul.wgsl").to_string(),
            Layout::Blocked => self.blocked_source(),
            Layout::Interleaved { pad, prefetch } => self.interleaved_source(pad, prefetch),
        }
    }

    fn interleaved_source(&self, pad: usize, prefetch: bool) -> String {
        let (tx, ty) = (self.bn / self.tn, self.bm / self.tm);
        assert_eq!(tx * ty, 256, "{self:?}");
        assert!(
            (self.bm * self.bk).is_multiple_of(256) && (self.bn * self.bk).is_multiple_of(256),
            "{self:?}"
        );
        let (la, lb) = (self.bm * self.bk / 256, self.bn * self.bk / 256);
        let mut decls = String::new();
        let mut compute = String::new();
        let mut epilogue = String::new();
        for i in 0..self.tm {
            for j in 0..self.tn {
                decls += &format!("  var acc_{i}_{j} = 0.0;\n");
                epilogue += &format!(
                    "  store(r0 + {}u, c0 + {}u, ob, bias_base, acc_{i}_{j});\n",
                    ty * i,
                    tx * j
                );
            }
        }
        for i in 0..self.tm {
            compute += &format!("      let a{i} = as_[ab + {}u];\n", ty * i);
        }
        for j in 0..self.tn {
            compute += &format!("      let b{j} = bs_[bb + {}u];\n", tx * j);
        }
        for i in 0..self.tm {
            for j in 0..self.tn {
                compute += &format!("      acc_{i}_{j} = fma(a{i}, b{j}, acc_{i}_{j});\n");
            }
        }
        // Moving one K tile into workgroup memory: element tid + 256 l of
        // each tile, straight from global memory or via registers.
        let (mut first, mut stage, mut next) = (String::new(), String::new(), String::new());
        for l in 0..la {
            let e = format!("tid + {}u", 256 * l);
            if prefetch {
                first += &format!("  var pa{l} = a_get({e}, 0u, row0, a_base);\n");
                stage += &format!("    as_[a_slot({e})] = pa{l};\n");
                next += &format!("    pa{l} = a_get({e}, k0 + BK, row0, a_base);\n");
            }
        }
        if !prefetch {
            // A loop rather than unrolled copies: nothing here needs a
            // constant index, and SwiftShader compiled the unrolled form
            // for minutes, with gigabytes of memory.
            stage += &format!(
                "    for (var l = 0u; l < {la}u; l++) {{ let e = tid + l * 256u; as_[a_slot(e)] = a_get(e, k0, row0, a_base); }}\n"
            );
            stage += &format!(
                "    for (var l = 0u; l < {lb}u; l++) {{ let e = tid + l * 256u; bs_[b_slot(e)] = b_get(e, k0, col0, b_base); }}\n"
            );
        }
        for l in 0..lb {
            let e = format!("tid + {}u", 256 * l);
            if prefetch {
                first += &format!("  var pb{l} = b_get({e}, 0u, col0, b_base);\n");
                stage += &format!("    bs_[b_slot({e})] = pb{l};\n");
                next += &format!("    pb{l} = b_get({e}, k0 + BK, col0, b_base);\n");
            }
        }
        include_str!("shaders/matmul_interleaved.wgsl")
            .replace("{BM}", &self.bm.to_string())
            .replace("{BN}", &self.bn.to_string())
            .replace("{BK}", &self.bk.to_string())
            .replace("{TX}", &tx.to_string())
            .replace("{TY}", &ty.to_string())
            .replace("{PAD}", &pad.to_string())
            .replace("{SA_LEN}", &(self.bk * (self.bm + pad)).to_string())
            .replace("{SB_LEN}", &(self.bk * (self.bn + pad)).to_string())
            .replace("{ACC_DECLS}", decls.trim_end())
            .replace("{PREFETCH_FIRST}", first.trim_end())
            .replace("{STAGE}", stage.trim_end())
            .replace("{PREFETCH_NEXT}", next.trim_end())
            .replace("{COMPUTE}", compute.trim_end())
            .replace("{EPILOGUE}", epilogue.trim_end())
    }

    fn blocked_source(&self) -> String {
        assert_eq!((self.bm / self.tm) * (self.bn / self.tn), 256, "{self:?}");
        assert!(
            self.tm.is_multiple_of(4)
                && self.tn.is_multiple_of(4)
                && (self.bm * self.bk).is_multiple_of(256)
                && (self.bn * self.bk).is_multiple_of(256)
        );
        // Register code, unrolled here so that every index is a constant.
        const LANE: [&str; 4] = ["x", "y", "z", "w"];
        let (tm, tn4) = (self.tm, self.tn / 4);
        let mut decls = String::new();
        let mut compute = String::new();
        let mut epilogue = String::new();
        for i in 0..tm {
            for j in 0..tn4 {
                decls += &format!("  var acc_{i}_{j} = vec4<f32>(0.0);\n");
            }
        }
        for i in 0..tm / 4 {
            compute += &format!("      let a{i} = a4(ab + {}u);\n", 4 * i);
        }
        for j in 0..tn4 {
            compute += &format!("      let b{j} = b4(bb + {}u);\n", 4 * j);
        }
        for i in 0..tm {
            for j in 0..tn4 {
                compute += &format!(
                    "      acc_{i}_{j} = fma(vec4<f32>(a{}.{}), b{j}, acc_{i}_{j});\n",
                    i / 4,
                    LANE[i % 4]
                );
            }
        }
        for i in 0..tm {
            for j in 0..tn4 {
                for (c, lane) in LANE.iter().enumerate() {
                    epilogue += &format!(
                        "  store(r0 + {i}u, c0 + {}u, ob, bias_base, acc_{i}_{j}.{lane});\n",
                        4 * j + c
                    );
                }
            }
        }
        include_str!("shaders/matmul_tiled.wgsl")
            .replace("{BM}", &self.bm.to_string())
            .replace("{BN}", &self.bn.to_string())
            .replace("{BK}", &self.bk.to_string())
            .replace("{TM}", &self.tm.to_string())
            .replace("{TN}", &self.tn.to_string())
            .replace("{SA_LEN}", &(self.bk * (self.bm + 4)).to_string())
            .replace("{SB_LEN}", &(self.bk * (self.bn + 4)).to_string())
            .replace("{ACC_DECLS}", decls.trim_end())
            .replace("{COMPUTE}", compute.trim_end())
            .replace("{EPILOGUE}", epilogue.trim_end())
    }
}

/// Every matmul kernel the GPU compiles.  [`MatMulPolicy`] chooses
/// among them by the shape of the product.
pub const TILES: [Tile; 10] = [
    Tile {
        name: "s64",
        bm: 64,
        bn: 64,
        bk: 16,
        tm: 4,
        tn: 4,
        layout: Layout::Original,
    },
    Tile {
        name: "v64",
        bm: 64,
        bn: 64,
        bk: 16,
        tm: 4,
        tn: 4,
        layout: Layout::Blocked,
    },
    Tile {
        name: "v128",
        bm: 128,
        bn: 128,
        bk: 8,
        tm: 8,
        tn: 8,
        layout: Layout::Blocked,
    },
    Tile {
        name: "v128x64",
        bm: 128,
        bn: 64,
        bk: 8,
        tm: 8,
        tn: 4,
        layout: Layout::Blocked,
    },
    Tile {
        name: "v64x128",
        bm: 64,
        bn: 128,
        bk: 8,
        tm: 4,
        tn: 8,
        layout: Layout::Blocked,
    },
    Tile {
        name: "i64",
        bm: 64,
        bn: 64,
        bk: 16,
        tm: 4,
        tn: 4,
        layout: Layout::Interleaved {
            pad: 2,
            prefetch: false,
        },
    },
    Tile {
        name: "i64p",
        bm: 64,
        bn: 64,
        bk: 16,
        tm: 4,
        tn: 4,
        layout: Layout::Interleaved {
            pad: 2,
            prefetch: true,
        },
    },
    Tile {
        name: "i128x64",
        bm: 128,
        bn: 64,
        bk: 8,
        tm: 8,
        tn: 4,
        layout: Layout::Interleaved {
            pad: 4,
            prefetch: false,
        },
    },
    Tile {
        name: "i128x64p",
        bm: 128,
        bn: 64,
        bk: 8,
        tm: 8,
        tn: 4,
        layout: Layout::Interleaved {
            pad: 4,
            prefetch: true,
        },
    },
    Tile {
        name: "i128",
        bm: 128,
        bn: 128,
        bk: 8,
        tm: 8,
        tn: 8,
        layout: Layout::Interleaved {
            pad: 4,
            prefetch: false,
        },
    },
];

/// A fused attention kernel (`attention.wgsl`): `lanes` threads per query,
/// key tiles of `bkv`.  Replaces the scores matmul, softmax, p.v matmul
/// and gate of one attention block when [`Gpu::attention`] names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attention {
    pub name: &'static str,
    pub lanes: usize,
    pub bkv: usize,
    /// `attention_tiled.wgsl` instead: 64 queries per workgroup, each
    /// thread a 4 x 4 block of scores and of outputs (`lanes` and `bkv`
    /// unused).
    pub tiled: bool,
}

/// Every fused attention kernel; [`Gpu::check_attention`] checks each.
pub const ATTENTIONS: [Attention; 3] = [
    Attention {
        name: "a4",
        lanes: 4,
        bkv: 16,
        tiled: false,
    },
    Attention {
        name: "a8",
        lanes: 8,
        bkv: 16,
        tiled: false,
    },
    Attention {
        name: "t64",
        lanes: 0,
        bkv: 64,
        tiled: true,
    },
];

impl Attention {
    /// Queries per workgroup.
    fn rows(&self) -> usize {
        if self.tiled { 64 } else { 256 / self.lanes }
    }

    fn tiled_source(&self) -> String {
        let (r4, c4) = (0..4, 0..4);
        let mut decls = String::new();
        let mut s_zero = String::new();
        let mut s_step = String::new();
        let mut soft1 = String::new();
        let mut soft2 = String::new();
        let mut values = String::new();
        let mut finish = String::new();
        for i in r4.clone() {
            decls += &format!("  var m{i} = NEG;\n  var l{i} = 0.0;\n");
            for j in c4.clone() {
                decls += &format!("  var o{i}_{j} = 0.0;\n  var s{i}_{j} = 0.0;\n");
                s_zero += &format!("    s{i}_{j} = 0.0;\n");
            }
        }
        for i in r4.clone() {
            s_step += &format!("        let a{i} = buf[ab + {}u];\n", 16 * i);
        }
        for j in c4.clone() {
            s_step += &format!("        let b{j} = buf[bb + {}u];\n", 16 * j);
        }
        for i in r4.clone() {
            for j in c4.clone() {
                s_step += &format!("        s{i}_{j} = fma(a{i}, b{j}, s{i}_{j});\n");
            }
        }
        for i in r4.clone() {
            for j in c4.clone() {
                soft1 += &format!(
                    "    s{i}_{j} = select(NEG, s{i}_{j} * p.alpha, j0 + tx + {}u < p.seq);\n",
                    16 * j
                );
            }
            soft1 += &format!(
                "    red[(ty + {}u) * 16u + tx] = max(max(s{i}_0, s{i}_1), max(s{i}_2, s{i}_3));\n",
                16 * i
            );
        }
        for i in r4.clone() {
            soft2 += &format!(
                "    var tm{i} = red[(ty + {r}u) * 16u];\n    for (var k = 1u; k < 16u; k++) {{ tm{i} = max(tm{i}, red[(ty + {r}u) * 16u + k]); }}\n    let mn{i} = max(m{i}, tm{i});\n    let sc{i} = exp(m{i} - mn{i});\n    m{i} = mn{i};\n",
                r = 16 * i
            );
            for j in c4.clone() {
                soft2 += &format!(
                    "    s{i}_{j} = exp(s{i}_{j} - mn{i});\n    o{i}_{j} = o{i}_{j} * sc{i};\n"
                );
            }
            soft2 += &format!("    l{i} = l{i} * sc{i} + s{i}_0 + s{i}_1 + s{i}_2 + s{i}_3;\n");
        }
        for c in c4.clone() {
            if c > 0 {
                values += "    workgroupBarrier();\n";
            }
            for i in r4.clone() {
                values += &format!("    buf[tx * S + ty + {}u] = s{i}_{c};\n", 16 * i);
            }
            values += &format!(
                "    for (var l = 0u; l < 4u; l++) {{ stage_v(t + l * 256u, tok0, j0 + {}u, vc); }}\n    workgroupBarrier();\n    for (var kk = 0u; kk < 16u; kk++) {{\n      let pb = kk * S + ty;\n      let vb = B2 + kk * D + tx;\n",
                16 * c
            );
            for i in r4.clone() {
                values += &format!("      let pk{i} = buf[pb + {}u];\n", 16 * i);
            }
            for j in c4.clone() {
                values += &format!("      let v{j} = buf[vb + {}u];\n", 16 * j);
            }
            for i in r4.clone() {
                for j in c4.clone() {
                    values += &format!("      o{i}_{j} = fma(pk{i}, v{j}, o{i}_{j});\n");
                }
            }
            values += "    }\n";
        }
        for i in r4.clone() {
            finish += &format!("  red[(ty + {}u) * 16u + tx] = l{i};\n", 16 * i);
        }
        finish += "  workgroupBarrier();\n";
        for i in r4 {
            finish += &format!(
                "  var lt{i} = 0.0;\n  for (var k = 0u; k < 16u; k++) {{ lt{i} += red[(ty + {r}u) * 16u + k]; }}\n  let qi{i} = q0 + ty + {r}u;\n  if (qi{i} < p.seq) {{\n    let tok = tok0 + qi{i};\n    var g = 1.0;\n    if ((p.flags & 1u) != 0u) {{ g = 1.0 / (1.0 + exp(-gates[tok * p.gate_row + h])); }}\n    let f = g / lt{i};\n    let ob = tok * p.o_row + h * D + tx;\n",
                r = 16 * i
            );
            for j in c4.clone() {
                finish += &format!("    o[ob + {}u] = o{i}_{j} * f;\n", 16 * j);
            }
            finish += "  }\n";
        }
        include_str!("shaders/attention_tiled.wgsl")
            .replace("{DECLS}", decls.trim_end())
            .replace("{S_ZERO}", s_zero.trim_end())
            .replace("{S_STEP}", s_step.trim_end())
            .replace("{SOFTMAX_1}", soft1.trim_end())
            .replace("{SOFTMAX_2}", soft2.trim_end())
            .replace("{VALUES}", values.trim_end())
            .replace("{FINISH}", finish.trim_end())
    }

    fn source(&self) -> String {
        if self.tiled {
            return self.tiled_source();
        }
        const D: usize = 64;
        let (l, b) = (self.lanes, self.bkv);
        assert!(
            256usize.is_multiple_of(l) && b.is_multiple_of(l) && D.is_multiple_of(l),
            "{self:?}"
        );
        let (kpl, dpl) = (b / l, D / l);
        let mut q_load = String::new();
        for d in 0..D {
            q_load += &format!("  let q{d} = qkv[qbase + {d}u];\n");
        }
        let mut o_decls = String::new();
        let mut store = String::new();
        for n in 0..dpl {
            o_decls += &format!("  var o{n} = 0.0;\n");
            store += &format!("    o[ob + {}u] = o{n} * f;\n", l * n);
        }
        let mut scores = String::new();
        for k in 0..kpl {
            scores += &format!("    let kj{k} = lane + {}u;\n    var s{k} = 0.0;\n", l * k);
            for d in 0..D {
                scores += &format!("    s{k} = fma(q{d}, kv[kj{k} * SK + {d}u], s{k});\n");
            }
            scores += &format!("    s{k} = select(NEG, s{k} * p.alpha, j0 + kj{k} < p.seq);\n");
        }
        let mut mx = "s0".to_string();
        for k in 1..kpl {
            mx = format!("max({mx}, s{k})");
        }
        scores += &format!("    let mx = {mx};\n");
        let mut probs = String::new();
        let mut sum = "l * sc".to_string();
        for k in 0..kpl {
            probs += &format!("    let p{k} = exp(s{k} - mn);\n    ps[r * SP + kj{k}] = p{k};\n");
            sum += &format!(" + p{k}");
        }
        probs += &format!("    l = {sum};\n");
        for n in 0..dpl {
            probs += &format!("    o{n} = o{n} * sc;\n");
        }
        let mut pv = String::new();
        for j in 0..b {
            pv += &format!("    let pj{j} = ps[r * SP + {j}u];\n");
            for n in 0..dpl {
                pv += &format!(
                    "    o{n} = fma(pj{j}, kv[{j}u * SK + lane + {}u], o{n});\n",
                    l * n
                );
            }
        }
        include_str!("shaders/attention.wgsl")
            .replace("{LANES}", &l.to_string())
            .replace("{BKV}", &b.to_string())
            .replace("{KV_LEN}", &(b * (D + 1)).to_string())
            .replace("{PS_LEN}", &((256 / l) * (b + 1)).to_string())
            .replace("{Q_LOAD}", q_load.trim_end())
            .replace("{O_DECLS}", o_decls.trim_end())
            .replace("{SCORES}", scores.trim_end())
            .replace("{PROBS}", probs.trim_end())
            .replace("{PV}", pv.trim_end())
            .replace("{STORE}", store.trim_end())
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct AttentionParams {
    seq: u32,
    row: u32,
    k_off: u32,
    v_off: u32,
    o_row: u32,
    gate_row: u32,
    flags: u32,
    alpha: f32,
}

/// Which matmul kernel to use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatMulPolicy {
    /// Always this entry of [`TILES`].
    Fixed(usize),
    /// One kernel for products with both sides over 64 and another for the
    /// rest (frequency attention, gates), as [`bench::calibrate`] chose for
    /// this GPU.
    Split { large: usize, small: usize },
}

impl MatMulPolicy {
    /// Index into [`TILES`] for an `m` x `n` output.
    pub fn pick(self, m: usize, n: usize) -> usize {
        match self {
            MatMulPolicy::Fixed(t) => t,
            MatMulPolicy::Split { large, small } => {
                if m > 64 && n > 64 {
                    large
                } else {
                    small
                }
            }
        }
    }

    /// A kernel name from [`TILES`], or two joined by a comma (large, small).
    pub fn parse(s: &str) -> Option<Self> {
        let find = |name: &str| TILES.iter().position(|t| t.name == name);
        match s.split_once(',') {
            Some((l, m)) => Some(MatMulPolicy::Split {
                large: find(l)?,
                small: find(m)?,
            }),
            None => find(s).map(MatMulPolicy::Fixed),
        }
    }

    /// The inverse of [`MatMulPolicy::parse`].
    pub fn name(self) -> String {
        match self {
            MatMulPolicy::Fixed(t) => TILES[t].name.to_string(),
            MatMulPolicy::Split { large, small } => {
                format!("{},{}", TILES[large].name, TILES[small].name)
            }
        }
    }
}

pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub info: wgpu::AdapterInfo,
    /// Which matmul kernel [`Rec::matmul`] uses unless a [`MatMul`] names one.
    pub policy: std::cell::Cell<MatMulPolicy>,
    /// Kernels of [`TILES`] that passed [`Gpu::check_kernels`] on this GPU
    /// (all, until it runs).  The policy never picks one that failed.
    pub usable: std::cell::Cell<[bool; TILES.len()]>,
    /// The fused attention kernel (index into [`ATTENTIONS`]) that
    /// [`net::Job`] uses, or `None` for the separate scores matmul,
    /// softmax, p.v matmul and gate.  Set it only to a kernel that
    /// [`Gpu::check_attention`] passed on this GPU.
    pub attention: std::cell::Cell<Option<usize>>,
    /// Fused attention kernels that passed [`Gpu::check_attention`] (none,
    /// until it runs).
    pub attention_usable: std::cell::Cell<[bool; ATTENTIONS.len()]>,
    kernels: Kernels,
    /// Bound where a kernel has an input it does not use for this dispatch.
    dummy: wgpu::Buffer,
    uniforms: wgpu::Buffer,
    errors: Arc<Mutex<Vec<String>>>,
    /// GPU timestamps per dispatch, on a device opened for profiling.
    prof: Option<Prof>,
}

/// Timestamp queries around every dispatch of a profiled [`Gpu`].  Each
/// profiled submission runs one compute pass per dispatch with a timestamp
/// at either end, resolves them, and copies them to its own readback
/// buffer; [`Gpu::collect_profile`] adds the finished ones to the totals.
/// Splitting a step into one pass per dispatch costs a little speed, so
/// only `?profile` runs open a device this way.
struct Prof {
    set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    /// Nanoseconds per timestamp tick.
    period: f64,
    /// Readback buffers of submissions not yet collected, with the label
    /// of each dispatch in them.
    pending: RefCell<Vec<(Vec<String>, wgpu::Buffer)>>,
    /// Dispatch count and GPU seconds per label.
    totals: RefCell<BTreeMap<String, (usize, f64)>>,
}

struct Kernels {
    /// One pipeline per entry of [`TILES`].
    matmul: (
        Vec<std::cell::OnceCell<wgpu::ComputePipeline>>,
        wgpu::BindGroupLayout,
    ),
    softmax: (wgpu::ComputePipeline, wgpu::BindGroupLayout),
    rmsnorm: (wgpu::ComputePipeline, wgpu::BindGroupLayout),
    rope: wgpu::ComputePipeline,
    gate: wgpu::ComputePipeline,
    transpose: wgpu::ComputePipeline,
    glu: wgpu::ComputePipeline,
    elementwise: wgpu::BindGroupLayout,
    /// One per entry of [`ATTENTIONS`], compiled on first use; bound like
    /// the element-wise kernels (read, read, read-write, uniform).
    attention: Vec<std::cell::OnceCell<wgpu::ComputePipeline>>,
}

#[derive(Clone, Copy)]
enum Bind {
    Read,
    ReadWrite,
    Uniform,
}

fn bind_layout(device: &wgpu::Device, label: &str, binds: &[Bind]) -> wgpu::BindGroupLayout {
    let entries: Vec<_> = binds
        .iter()
        .enumerate()
        .map(|(i, b)| wgpu::BindGroupLayoutEntry {
            binding: i as u32,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: match b {
                    Bind::Read => wgpu::BufferBindingType::Storage { read_only: true },
                    Bind::ReadWrite => wgpu::BufferBindingType::Storage { read_only: false },
                    Bind::Uniform => wgpu::BufferBindingType::Uniform,
                },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        })
        .collect();
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(label),
        entries: &entries,
    })
}

fn pipeline(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    entry: &str,
    layout: &wgpu::BindGroupLayout,
) -> wgpu::ComputePipeline {
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(entry),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(entry),
        layout: Some(&pl),
        module,
        entry_point: Some(entry),
        compilation_options: Default::default(),
        cache: None,
    })
}

fn shader(device: &wgpu::Device, label: &str, src: &str) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(src.into()),
    })
}

impl Kernels {
    fn new(device: &wgpu::Device) -> Self {
        use Bind::*;
        let mm = bind_layout(device, "matmul", &[Read, Read, ReadWrite, Read, Uniform]);
        let sm = bind_layout(device, "softmax", &[ReadWrite, Uniform]);
        let rn = bind_layout(device, "rmsnorm", &[Read, Read, ReadWrite, Uniform]);
        let ew = bind_layout(device, "elementwise", &[Read, Read, ReadWrite, Uniform]);
        // Matmul pipelines compile on first use (see `matmul_pipeline`).
        let matmul = TILES.iter().map(|_| std::cell::OnceCell::new()).collect();
        let s = shader(device, "softmax", include_str!("shaders/softmax.wgsl"));
        let r = shader(device, "rmsnorm", include_str!("shaders/rmsnorm.wgsl"));
        let e = shader(
            device,
            "elementwise",
            include_str!("shaders/elementwise.wgsl"),
        );
        Self {
            matmul: (matmul, mm),
            softmax: (pipeline(device, &s, "main", &sm), sm),
            rmsnorm: (pipeline(device, &r, "main", &rn), rn),
            rope: pipeline(device, &e, "rope", &ew),
            gate: pipeline(device, &e, "gate", &ew),
            transpose: pipeline(device, &e, "transpose3", &ew),
            glu: pipeline(device, &e, "glu", &ew),
            elementwise: ew,
            attention: ATTENTIONS
                .iter()
                .map(|_| std::cell::OnceCell::new())
                .collect(),
        }
    }

    fn attention_pipeline(&self, device: &wgpu::Device, a: usize) -> &wgpu::ComputePipeline {
        self.attention[a].get_or_init(|| {
            let at = &ATTENTIONS[a];
            pipeline(
                device,
                &shader(device, at.name, &at.source()),
                "main",
                &self.elementwise,
            )
        })
    }

    /// The pipeline of matmul kernel `t`, compiled the first time it is
    /// needed: a device compiles only the kernels it runs (all of them once
    /// in [`Gpu::check_kernels`], where a kernel the compiler rejects is
    /// reported and never used again).  Compiling every kernel up front
    /// took SwiftShader over 4 GB per device.
    fn matmul_pipeline(&self, device: &wgpu::Device, t: usize) -> &wgpu::ComputePipeline {
        self.matmul.0[t].get_or_init(|| {
            let tile = &TILES[t];
            pipeline(
                device,
                &shader(device, tile.name, &tile.source()),
                "main",
                &self.matmul.1,
            )
        })
    }
}

impl Gpu {
    /// Opens a GPU with WebGPU's default limits.  `WGPU_BACKEND` and
    /// `WGPU_ADAPTER_NAME` select an adapter natively (tests use
    /// SwiftShader's Vulkan driver this way).
    pub async fn new(power: wgpu::PowerPreference) -> Result<Self, String> {
        Self::open(power, false).await
    }

    /// Like [`Gpu::new`], but also asks for `timestamp-query` when the
    /// adapter offers it, so that [`Gpu::profiling`] is true.  The limits
    /// are the same defaults; only the optional feature differs.
    pub async fn new_profiling(power: wgpu::PowerPreference) -> Result<Self, String> {
        Self::open(power, true).await
    }

    async fn open(power: wgpu::PowerPreference, profile: bool) -> Result<Self, String> {
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = wgpu::Backends::PRIMARY | wgpu::Backends::BROWSER_WEBGPU;
        let instance = wgpu::Instance::new(desc.with_env());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: power,
                force_fallback_adapter: false,
                compatible_surface: None,
                ..Default::default()
            })
            .await
            .map_err(|e| format!("no GPU adapter: {e}"))?;
        let features = if profile {
            adapter.features() & wgpu::Features::TIMESTAMP_QUERY
        } else {
            wgpu::Features::empty()
        };
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("roformer"),
                required_features: features,
                required_limits: wgpu::Limits::defaults(),
                ..Default::default()
            })
            .await
            .map_err(|e| format!("GPU device: {e}"))?;
        Ok(Self::from_device(device, queue, adapter.get_info()))
    }

    pub fn from_device(device: wgpu::Device, queue: wgpu::Queue, info: wgpu::AdapterInfo) -> Self {
        let errors: Arc<Mutex<Vec<String>>> = Default::default();
        let sink = errors.clone();
        device.on_uncaptured_error(Arc::new(move |e: wgpu::Error| {
            if let Ok(mut v) = sink.lock() {
                v.push(e.to_string());
            }
        }));
        let kernels = Kernels::new(&device);
        let dummy = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("dummy"),
            size: 16,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("params"),
            size: PARAM_STRIDE * MAX_DISPATCHES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let prof = device
            .features()
            .contains(wgpu::Features::TIMESTAMP_QUERY)
            .then(|| Prof {
                set: device.create_query_set(&wgpu::QuerySetDescriptor {
                    label: Some("profile"),
                    ty: wgpu::QueryType::Timestamp,
                    count: (2 * MAX_DISPATCHES) as u32,
                }),
                resolve: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("profile resolve"),
                    size: 16 * MAX_DISPATCHES,
                    usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                }),
                period: f64::from(queue.get_timestamp_period()),
                pending: RefCell::new(Vec::new()),
                totals: RefCell::new(BTreeMap::new()),
            });
        Self {
            device,
            queue,
            info,
            policy: std::cell::Cell::new(MatMulPolicy::Fixed(0)),
            usable: std::cell::Cell::new([true; TILES.len()]),
            attention: std::cell::Cell::new(None),
            attention_usable: std::cell::Cell::new([false; ATTENTIONS.len()]),
            kernels,
            dummy,
            uniforms,
            errors,
            prof,
        }
    }

    /// True when this device records GPU time per dispatch.
    pub fn profiling(&self) -> bool {
        self.prof.is_some()
    }

    /// Reads the timestamps of every profiled submission so far into the
    /// totals.  Call after [`Gpu::done`]; does nothing on a device that is
    /// not profiling.
    pub async fn collect_profile(&self) -> Result<(), String> {
        let Some(prof) = &self.prof else {
            return Ok(());
        };
        let pending = std::mem::take(&mut *prof.pending.borrow_mut());
        for (labels, staging) in pending {
            let (tx, rx) = futures_channel::oneshot::channel();
            staging.map_async(wgpu::MapMode::Read, .., move |r| {
                let _ = tx.send(r);
            });
            self.poll();
            rx.await
                .map_err(|_| "profile readback cancelled".to_string())?
                .map_err(|e| format!("profile readback: {e}"))?;
            let view = staging
                .get_mapped_range(..)
                .map_err(|e| format!("profile readback: {e}"))?;
            let ticks: &[u64] = bytemuck::cast_slice(&view);
            let mut totals = prof.totals.borrow_mut();
            for (i, label) in labels.into_iter().enumerate() {
                let (start, end) = (ticks[2 * i], ticks[2 * i + 1]);
                let secs = end.saturating_sub(start) as f64 * prof.period * 1e-9;
                let e = totals.entry(label).or_insert((0, 0.0));
                e.0 += 1;
                e.1 += secs;
            }
            drop(view);
            staging.unmap();
        }
        Ok(())
    }

    /// GPU seconds per dispatch label since the last call, and clears them.
    /// Labels name the kernel, and for matmuls the shape and tile.
    pub fn take_profile(&self) -> BTreeMap<String, (usize, f64)> {
        self.prof
            .as_ref()
            .map(|p| std::mem::take(&mut *p.totals.borrow_mut()))
            .unwrap_or_default()
    }

    /// WebGPU errors reported since the last call (validation, out of
    /// memory, device lost).
    pub fn take_errors(&self) -> Vec<String> {
        self.errors
            .lock()
            .map(|mut v| std::mem::take(&mut *v))
            .unwrap_or_default()
    }

    /// A storage buffer of `len` f32s, zeroed.
    pub fn buffer(&self, label: &str, len: usize) -> wgpu::Buffer {
        self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: (len.max(4) * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        })
    }

    /// A storage buffer holding `data`.
    pub fn upload(&self, label: &str, data: &[f32]) -> wgpu::Buffer {
        if data.is_empty() {
            return self.buffer(label, 0);
        }
        self.device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytemuck::cast_slice(data),
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
            })
    }

    pub fn write(&self, buf: &wgpu::Buffer, offset: usize, data: &[f32]) {
        self.queue
            .write_buffer(buf, (offset * 4) as u64, bytemuck::cast_slice(data));
    }

    /// Resolves once the GPU has finished all submitted work.
    pub async fn done(&self) {
        let (tx, rx) = futures_channel::oneshot::channel();
        self.queue.on_submitted_work_done(move || {
            let _ = tx.send(());
        });
        self.poll();
        let _ = rx.await;
    }

    /// Native: drive the device until all callbacks have run.  On the web
    /// the browser does this and the call returns at once.
    fn poll(&self) {
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
    }

    /// Copies `len` f32s from `buf` (starting at element `offset`) back.
    pub async fn read(
        &self,
        buf: &wgpu::Buffer,
        offset: usize,
        len: usize,
    ) -> Result<Vec<f32>, String> {
        let size = (len * 4) as u64;
        let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(buf, (offset * 4) as u64, &staging, 0, size);
        self.queue.submit([enc.finish()]);
        let (tx, rx) = futures_channel::oneshot::channel();
        staging.map_async(wgpu::MapMode::Read, .., move |r| {
            let _ = tx.send(r);
        });
        self.poll();
        rx.await
            .map_err(|_| "readback cancelled".to_string())?
            .map_err(|e| format!("readback: {e}"))?;
        let view = staging
            .get_mapped_range(..)
            .map_err(|e| format!("readback: {e}"))?;
        let out = bytemuck::cast_slice::<u8, f32>(&view).to_vec();
        drop(view);
        staging.unmap();
        Ok(out)
    }

    /// Runs every matmul kernel on two small products (both operand
    /// layouts, bias, accumulate, ragged edges) and compares each with the
    /// CPU.  A kernel that disagrees is marked unusable on this GPU, so a
    /// compiler or driver bug in one kernel costs speed, not correctness.
    /// Returns one line per kernel that failed; the original kernel
    /// (`s64`) failing is an error.
    pub async fn check_kernels(&self) -> Result<Vec<String>, String> {
        let (m, n, k) = (133, 141, 45);
        let rand = |len: usize, seed: u32| -> Vec<f32> {
            let mut s = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
            (0..len)
                .map(|_| {
                    s ^= s << 13;
                    s ^= s >> 17;
                    s ^= s << 5;
                    (s % 2001) as f32 / 1000.0 - 1.0
                })
                .collect()
        };
        let (a, b, c0, bias) = (rand(m * k, 1), rand(k * n, 2), rand(m * n, 3), rand(n, 4));
        // Case 1: A row-major, B a torch-layout weight, bias.  Case 2: A
        // column-major, B row-major, accumulate into C.
        let views = [
            (View::rows(0, k), View::t(0, k), false),
            (View::t(0, m), View::rows(0, n), true),
        ];
        let mut want = Vec::new();
        for (va, vb, acc) in views {
            let mut out = c0.clone();
            for i in 0..m {
                for j in 0..n {
                    let mut s = 0.0f64;
                    for kk in 0..k {
                        s += a[va.off + i * va.rs + kk * va.cs] as f64
                            * b[vb.off + kk * vb.rs + j * vb.cs] as f64;
                    }
                    out[i * n + j] = if acc {
                        (out[i * n + j] as f64 + s) as f32
                    } else {
                        (s + bias[j] as f64) as f32
                    };
                }
            }
            want.push(out);
        }
        let (ab, bb, biasb) = (
            self.upload("check a", &a),
            self.upload("check b", &b),
            self.upload("check bias", &bias),
        );
        let mut usable = [true; TILES.len()];
        let mut problems = Vec::new();
        for (t, tile) in TILES.iter().enumerate() {
            for (case, (va, vb, acc)) in views.iter().enumerate() {
                let cb = self.upload("check c", &c0);
                let mut mm = MatMul::new((&ab, *va), (&bb, *vb), (&cb, View::rows(0, n)), m, n, k);
                mm.tile = Some(t);
                mm.accumulate = *acc;
                if !*acc {
                    mm.bias = Some((&biasb, 0, 0));
                }
                let mut r = self.rec();
                r.matmul(&mm);
                r.submit();
                let got = self.read(&cb, 0, m * n).await?;
                let worst = got
                    .iter()
                    .zip(&want[case])
                    .map(|(g, w)| {
                        if g.is_finite() {
                            (g - w).abs() / (1.0 + w.abs())
                        } else {
                            f32::INFINITY
                        }
                    })
                    .fold(0.0f32, f32::max);
                let errors = self.take_errors();
                if worst > 1e-4 || !errors.is_empty() {
                    usable[t] = false;
                    let why = errors
                        .first()
                        .cloned()
                        .unwrap_or_else(|| format!("relative error {worst:.1e}"));
                    problems.push(format!("matmul kernel {} gives wrong results on this GPU (case {}: {why}); not used", tile.name, case + 1));
                    break;
                }
            }
        }
        self.usable.set(usable);
        if !usable[0] {
            return Err(format!(
                "this GPU computes matrix products wrongly: {}",
                problems.join("; ")
            ));
        }
        Ok(problems)
    }

    /// Runs every fused attention kernel on a small ragged case (two
    /// sequences of 37 tokens, with gates) and compares it with
    /// [`attention_cpu`].  A kernel that disagrees, or that the browser
    /// rejects, is marked unusable on this GPU; returns one line per
    /// failure.  Sets [`Gpu::attention_usable`].
    pub async fn check_attention(&self) -> Vec<String> {
        let (seq, nseq) = (37, 2);
        let tokens = seq * nseq;
        let mut st = 0x9e37_79b9u32;
        let mut rand = |n: usize| -> Vec<f32> {
            (0..n)
                .map(|_| {
                    st ^= st << 13;
                    st ^= st >> 17;
                    st ^= st << 5;
                    (st % 2001) as f32 / 1000.0 - 1.0
                })
                .collect()
        };
        let (qkv, gates) = (rand(tokens * 1536), rand(tokens * 8));
        let alpha = 0.125;
        let want = attention_cpu(&qkv, Some(&gates), seq, nseq, alpha);
        let (qb, gb) = (
            self.upload("check qkv", &qkv),
            self.upload("check gates", &gates),
        );
        let mut usable = [false; ATTENTIONS.len()];
        let mut problems = Vec::new();
        for (a, at) in ATTENTIONS.iter().enumerate() {
            let ob = self.buffer("check o", tokens * 512);
            let mut r = self.rec();
            r.attention(a, &qb, Some(&gb), &ob, seq, nseq, alpha);
            r.submit();
            let got = match self.read(&ob, 0, tokens * 512).await {
                Ok(v) => v,
                Err(e) => {
                    problems.push(format!("attention kernel {}: {e}; not used", at.name));
                    continue;
                }
            };
            let worst = got
                .iter()
                .zip(&want)
                .map(|(g, w)| {
                    if g.is_finite() {
                        (g - w).abs() / (1.0 + w.abs())
                    } else {
                        f32::INFINITY
                    }
                })
                .fold(0.0f32, f32::max);
            let errors = self.take_errors();
            if worst > 1e-4 || !errors.is_empty() {
                let why = errors
                    .first()
                    .cloned()
                    .unwrap_or_else(|| format!("relative error {worst:.1e}"));
                problems.push(format!(
                    "attention kernel {} gives wrong results on this GPU ({why}); not used",
                    at.name
                ));
            } else {
                usable[a] = true;
            }
        }
        self.attention_usable.set(usable);
        problems
    }

    pub fn rec(&self) -> Rec<'_> {
        Rec {
            gpu: self,
            params: Vec::new(),
            cmds: Vec::new(),
            labels: Vec::new(),
        }
    }
}

/// Attention as [`Rec::attention`] computes it, on the CPU in f64: `qkv`
/// is `[nseq * seq, 1536]`, the result `[nseq * seq, 512]`.  The reference
/// for the kernel checks.
pub fn attention_cpu(
    qkv: &[f32],
    gates: Option<&[f32]>,
    seq: usize,
    nseq: usize,
    alpha: f32,
) -> Vec<f32> {
    let mut o = vec![0.0f32; nseq * seq * 512];
    let mut sc = vec![0.0f64; seq];
    for s in 0..nseq {
        for h in 0..8 {
            for i in 0..seq {
                let qi = (s * seq + i) * 1536 + h * 64;
                let mut mx = f64::NEG_INFINITY;
                for (j, v) in sc.iter_mut().enumerate() {
                    let kj = (s * seq + j) * 1536 + 512 + h * 64;
                    let mut d = 0.0f64;
                    for c in 0..64 {
                        d += f64::from(qkv[qi + c]) * f64::from(qkv[kj + c]);
                    }
                    *v = d * f64::from(alpha);
                    mx = mx.max(*v);
                }
                let mut sum = 0.0;
                for v in sc.iter_mut() {
                    *v = (*v - mx).exp();
                    sum += *v;
                }
                let g = gates.map_or(1.0, |g| {
                    1.0 / (1.0 + (-f64::from(g[(s * seq + i) * 8 + h])).exp())
                });
                for c in 0..64 {
                    let mut acc = 0.0f64;
                    for (j, v) in sc.iter().enumerate() {
                        acc += v * f64::from(qkv[(s * seq + j) * 1536 + 1024 + h * 64 + c]);
                    }
                    o[(s * seq + i) * 512 + h * 64 + c] = (g * acc / sum) as f32;
                }
            }
        }
    }
    o
}

/// Report lines for [`Gpu::take_profile`] totals: GPU time per label,
/// largest first, with its share of the total, then the total itself.
pub fn profile_report(totals: &BTreeMap<String, (usize, f64)>) -> Vec<String> {
    let all: f64 = totals.values().map(|v| v.1).sum();
    let mut rows: Vec<_> = totals.iter().collect();
    rows.sort_by(|a, b| b.1.1.total_cmp(&a.1.1));
    let mut out: Vec<String> = rows
        .iter()
        .map(|(label, (n, secs))| {
            format!(
                "profile: {label}: {secs:.3} s GPU in {n} dispatches ({:.1}%)",
                100.0 * secs / all.max(1e-12)
            )
        })
        .collect();
    out.push(format!("profile: total GPU time {all:.3} s"));
    out
}

/// One strided operand of a [`MatMul`]: element `(r, c)` of batch item
/// `(bo, bi)` sits at `off + bo*bo_stride + bi*bi_stride + r*rs + c*cs`.
#[derive(Clone, Copy, Debug, Default)]
pub struct View {
    pub off: usize,
    pub rs: usize,
    pub cs: usize,
    pub bo: usize,
    pub bi: usize,
}

impl View {
    /// Row-major `[rows, cols]` starting at `off`.
    pub fn rows(off: usize, cols: usize) -> Self {
        Self {
            off,
            rs: cols,
            cs: 1,
            ..Default::default()
        }
    }
    /// The transpose of a row-major `[rows, cols]` matrix (a torch
    /// `nn.Linear` weight `[out, in]` read as `[in, out]`).
    pub fn t(off: usize, cols: usize) -> Self {
        Self {
            off,
            rs: 1,
            cs: cols,
            ..Default::default()
        }
    }
    pub fn batch(self, bo: usize, bi: usize) -> Self {
        Self { bo, bi, ..self }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    None,
    Gelu,
    Tanh,
}

/// `C = act(alpha * A B + bias)`, or `C += ...` with `accumulate`, over a
/// batch of `outer * inner` matrices.  `A` is `[m, k]`, `B` `[k, n]`.
pub struct MatMul<'a> {
    pub a: (&'a wgpu::Buffer, View),
    pub b: (&'a wgpu::Buffer, View),
    pub c: (&'a wgpu::Buffer, View),
    /// Buffer, offset and stride per outer batch item.
    pub bias: Option<(&'a wgpu::Buffer, usize, usize)>,
    pub m: usize,
    pub n: usize,
    pub k: usize,
    pub outer: usize,
    pub inner: usize,
    pub alpha: f32,
    pub act: Act,
    pub accumulate: bool,
    /// Index into [`TILES`]; `None` follows [`Gpu::policy`].
    pub tile: Option<usize>,
}

impl<'a> MatMul<'a> {
    pub fn new(
        a: (&'a wgpu::Buffer, View),
        b: (&'a wgpu::Buffer, View),
        c: (&'a wgpu::Buffer, View),
        m: usize,
        n: usize,
        k: usize,
    ) -> Self {
        Self {
            a,
            b,
            c,
            bias: None,
            m,
            n,
            k,
            outer: 1,
            inner: 1,
            alpha: 1.0,
            act: Act::None,
            accumulate: false,
            tile: None,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MatMulParams {
    m: u32,
    n: u32,
    k: u32,
    inner: u32,
    a_off: u32,
    a_si: u32,
    a_sk: u32,
    a_bo: u32,
    a_bi: u32,
    b_off: u32,
    b_sk: u32,
    b_sj: u32,
    b_bo: u32,
    b_bi: u32,
    c_off: u32,
    c_si: u32,
    c_sj: u32,
    c_bo: u32,
    c_bi: u32,
    bias_off: u32,
    bias_bo: u32,
    flags: u32,
    alpha: f32,
    pad: u32,
}

fn u(x: usize) -> u32 {
    u32::try_from(x).expect("index exceeds u32")
}

/// Splits `groups` workgroups over x and y so neither exceeds 65,535.
fn grid(groups: usize) -> (u32, u32) {
    let x = groups.clamp(1, 65_535);
    (u(x), u(groups.div_ceil(x).max(1)))
}

enum Kernel {
    MatMul(usize),
    Attention(usize),
    Softmax,
    RmsNorm,
    Rope,
    Gate,
    Transpose,
    Glu,
}

/// Dispatches recorded for one submission.
pub struct Rec<'g> {
    gpu: &'g Gpu,
    params: Vec<u8>,
    cmds: Vec<(Kernel, wgpu::BindGroup, [u32; 3])>,
    /// One label per dispatch, kept only when the device is profiling.
    labels: Vec<String>,
}

impl Rec<'_> {
    fn push(&mut self, kernel: Kernel, bufs: &[&wgpu::Buffer], params: &[u8], groups: [u32; 3]) {
        if self.gpu.prof.is_some() {
            let label = match &kernel {
                Kernel::MatMul(t) => format!("matmul {}", TILES[*t].name),
                Kernel::Attention(a) => format!("attention {}", ATTENTIONS[*a].name),
                Kernel::Softmax => "softmax".into(),
                Kernel::RmsNorm => "rmsnorm".into(),
                Kernel::Rope => "rope".into(),
                Kernel::Gate => "gate".into(),
                Kernel::Transpose => "transpose".into(),
                Kernel::Glu => "glu".into(),
            };
            self.labels.push(label);
        }
        let offset = self.params.len() as u64;
        assert!(
            offset < PARAM_STRIDE * MAX_DISPATCHES,
            "too many dispatches in one step"
        );
        assert!(
            groups.iter().all(|&g| (1..=65_535).contains(&g)),
            "workgroup count {groups:?}"
        );
        self.params.extend_from_slice(params);
        self.params.resize((offset + PARAM_STRIDE) as usize, 0);
        let k = &self.gpu.kernels;
        let layout = match kernel {
            Kernel::MatMul(_) => &k.matmul.1,
            Kernel::Softmax => &k.softmax.1,
            Kernel::RmsNorm => &k.rmsnorm.1,
            _ => &k.elementwise,
        };
        let mut entries: Vec<_> = bufs
            .iter()
            .enumerate()
            .map(|(i, b)| wgpu::BindGroupEntry {
                binding: i as u32,
                resource: b.as_entire_binding(),
            })
            .collect();
        entries.push(wgpu::BindGroupEntry {
            binding: bufs.len() as u32,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: &self.gpu.uniforms,
                offset,
                size: NonZeroU64::new(params.len() as u64),
            }),
        });
        let bg = self
            .gpu
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout,
                entries: &entries,
            });
        self.cmds.push((kernel, bg, groups));
    }

    pub fn is_empty(&self) -> bool {
        self.cmds.is_empty()
    }

    /// Submits everything recorded as one command buffer.
    pub fn submit(self) {
        if self.cmds.is_empty() {
            return;
        }
        let gpu = self.gpu;
        gpu.queue.write_buffer(&gpu.uniforms, 0, &self.params);
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        let k = &gpu.kernels;
        let pipeline = |kernel: &Kernel| match kernel {
            Kernel::MatMul(t) => k.matmul_pipeline(&gpu.device, *t),
            Kernel::Attention(a) => k.attention_pipeline(&gpu.device, *a),
            Kernel::Softmax => &k.softmax.0,
            Kernel::RmsNorm => &k.rmsnorm.0,
            Kernel::Rope => &k.rope,
            Kernel::Gate => &k.gate,
            Kernel::Transpose => &k.transpose,
            Kernel::Glu => &k.glu,
        };
        if let Some(prof) = &gpu.prof {
            // One pass per dispatch, timestamped at both ends.
            for (i, (kernel, bg, [x, y, z])) in self.cmds.iter().enumerate() {
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: None,
                    timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                        query_set: &prof.set,
                        beginning_of_pass_write_index: Some(2 * i as u32),
                        end_of_pass_write_index: Some(2 * i as u32 + 1),
                    }),
                });
                pass.set_pipeline(pipeline(kernel));
                pass.set_bind_group(0, bg, &[]);
                pass.dispatch_workgroups(*x, *y, *z);
            }
            let n = 2 * self.cmds.len() as u32;
            let bytes = 8 * u64::from(n);
            let staging = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("profile readback"),
                size: bytes,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            enc.resolve_query_set(&prof.set, 0..n, &prof.resolve, 0);
            enc.copy_buffer_to_buffer(&prof.resolve, 0, &staging, 0, bytes);
            prof.pending.borrow_mut().push((self.labels, staging));
        } else {
            let mut pass = enc.begin_compute_pass(&Default::default());
            for (kernel, bg, [x, y, z]) in &self.cmds {
                pass.set_pipeline(pipeline(kernel));
                pass.set_bind_group(0, bg, &[]);
                pass.dispatch_workgroups(*x, *y, *z);
            }
        }
        gpu.queue.submit([enc.finish()]);
    }

    pub fn matmul(&mut self, mm: &MatMul) {
        let (a, b, c) = (mm.a.1, mm.b.1, mm.c.1);
        let (bias_buf, bias_off, bias_bo) = match mm.bias {
            Some((buf, off, bo)) => (buf, off, bo),
            None => (&self.gpu.dummy, 0, 0),
        };
        let act = match mm.act {
            Act::None => 0,
            Act::Gelu => 1,
            Act::Tanh => 2,
        };
        let p = MatMulParams {
            m: u(mm.m),
            n: u(mm.n),
            k: u(mm.k),
            inner: u(mm.inner),
            a_off: u(a.off),
            a_si: u(a.rs),
            a_sk: u(a.cs),
            a_bo: u(a.bo),
            a_bi: u(a.bi),
            b_off: u(b.off),
            b_sk: u(b.rs),
            b_sj: u(b.cs),
            b_bo: u(b.bo),
            b_bi: u(b.bi),
            c_off: u(c.off),
            c_si: u(c.rs),
            c_sj: u(c.cs),
            c_bo: u(c.bo),
            c_bi: u(c.bi),
            bias_off: u(bias_off),
            bias_bo: u(bias_bo),
            flags: u32::from(mm.bias.is_some()) | (u32::from(mm.accumulate) << 1) | (act << 2),
            alpha: mm.alpha,
            pad: 0,
        };
        let t = mm.tile.unwrap_or_else(|| {
            let t = self.gpu.policy.get().pick(mm.m, mm.n);
            if self.gpu.usable.get()[t] { t } else { 0 }
        });
        let tile = TILES[t];
        let groups = [
            u(mm.n.div_ceil(tile.bn)),
            u(mm.m.div_ceil(tile.bm)),
            u(mm.outer * mm.inner),
        ];
        self.push(
            Kernel::MatMul(t),
            &[mm.a.0, mm.b.0, mm.c.0, bias_buf],
            bytemuck::bytes_of(&p),
            groups,
        );
        if let Some(label) = self.labels.last_mut() {
            *label = format!(
                "matmul {}x{}x{} x{} {}",
                mm.m,
                mm.n,
                mm.k,
                mm.outer * mm.inner,
                tile.name
            );
        }
    }

    /// In-place softmax over `rows` contiguous rows of `len`.
    pub fn softmax(&mut self, x: &wgpu::Buffer, rows: usize, len: usize) {
        let (gx, gy) = grid(rows);
        let p = [u(rows), u(len), gx, 0];
        self.push(Kernel::Softmax, &[x], bytemuck::cast_slice(&p), [gx, gy, 1]);
    }

    /// RMSNorm of `rows` rows of 256: `y[y_off..] = x[x_off..] / |x| * g`.
    pub fn rmsnorm(
        &mut self,
        x: &wgpu::Buffer,
        x_off: usize,
        g: &wgpu::Buffer,
        y: &wgpu::Buffer,
        y_off: usize,
        rows: usize,
    ) {
        let (gx, gy) = grid(rows);
        let p = [u(rows), gx, u(x_off), u(y_off)];
        self.push(
            Kernel::RmsNorm,
            &[x, g, y],
            bytemuck::cast_slice(&p),
            [gx, gy, 1],
        );
    }

    fn elementwise(
        &mut self,
        kernel: Kernel,
        bufs: [&wgpu::Buffer; 3],
        count: usize,
        args: [usize; 3],
    ) {
        let (gx, gy) = grid(count.div_ceil(256));
        let p = [u(count), gx, u(args[0]), u(args[1]), u(args[2]), 0, 0, 0];
        self.push(kernel, &bufs, bytemuck::cast_slice(&p), [gx, gy, 1]);
    }

    /// Rotary embedding of q and k in a `[tokens, 1536]` qkv buffer whose
    /// tokens are sequences of `seq`; `table` is `[seq, 32]` cosines then
    /// `[seq, 32]` sines.
    pub fn rope(&mut self, qkv: &wgpu::Buffer, table: &wgpu::Buffer, tokens: usize, seq: usize) {
        let dummy = &self.gpu.dummy;
        self.elementwise(Kernel::Rope, [dummy, table, qkv], tokens * 512, [seq, 0, 0]);
    }

    /// `o[t, h*64 + d] *= sigmoid(gates[t, h])` for `tokens` rows.
    /// Fused attention for `nseq` sequences of `seq` tokens with kernel
    /// `a` of [`ATTENTIONS`]: `o[t, h*64 + d] = sigmoid(gates[t, h]) *
    /// sum_j softmax_j(alpha * q_t . k_j) v[j, h*64 + d]` per head, with
    /// q, k and v in a `[tokens, 1536]` qkv buffer at column offsets 0,
    /// 512 and 1024, `o` `[tokens, 512]` and `gates` `[tokens, 8]` (no
    /// gate when `None`).  The same result as the scores matmul, softmax,
    /// p.v matmul and gate in [`net::Job`], in one dispatch.
    #[allow(clippy::too_many_arguments)]
    pub fn attention(
        &mut self,
        a: usize,
        qkv: &wgpu::Buffer,
        gates: Option<&wgpu::Buffer>,
        o: &wgpu::Buffer,
        seq: usize,
        nseq: usize,
        alpha: f32,
    ) {
        let at = ATTENTIONS[a];
        let p = AttentionParams {
            seq: u(seq),
            row: 1536,
            k_off: 512,
            v_off: 1024,
            o_row: 512,
            gate_row: 8,
            flags: u32::from(gates.is_some()),
            alpha,
        };
        let groups = [u(seq.div_ceil(at.rows())), 8, u(nseq)];
        let g = gates.unwrap_or(&self.gpu.dummy);
        self.push(
            Kernel::Attention(a),
            &[qkv, g, o],
            bytemuck::bytes_of(&p),
            groups,
        );
    }

    pub fn gate(&mut self, o: &wgpu::Buffer, gates: &wgpu::Buffer, tokens: usize) {
        let dummy = &self.gpu.dummy;
        self.elementwise(Kernel::Gate, [dummy, gates, o], tokens * 512, [0, 0, 0]);
    }

    /// `[a, b, 256]` in `src` to `[b, a, 256]` in `dst`.
    pub fn transpose(&mut self, src: &wgpu::Buffer, dst: &wgpu::Buffer, a: usize, b: usize) {
        let dummy = &self.gpu.dummy;
        self.elementwise(Kernel::Transpose, [src, dummy, dst], a * b * 256, [a, b, 0]);
    }

    /// GLU of `y` (`[n, frames, 2w]`) into `mask` (`[frames, 4100]`) at
    /// feature offset `off`: `mask[t, off + j*w + c] = y[j,t,c] * sigmoid(y[j,t,w+c])`.
    pub fn glu(
        &mut self,
        y: &wgpu::Buffer,
        mask: &wgpu::Buffer,
        n: usize,
        frames: usize,
        w: usize,
        off: usize,
    ) {
        let dummy = &self.gpu.dummy;
        self.elementwise(
            Kernel::Glu,
            [y, dummy, mask],
            n * frames * w,
            [w, frames, off],
        );
    }
}

/// Identifies the compiled matmul kernels (FNV-1a over their WGSL), so a
/// per-GPU kernel choice is redone whenever the kernels change.
pub fn kernels_id() -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for t in &TILES {
        for b in t.source().bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Worst number of distinct addresses one bank serves in a single
    /// 32-lane access to workgroup memory, in RDNA's model of the LDS (32
    /// banks of 4 bytes; lanes reading the same address share one read).
    /// 1 means no bank conflict.  A model of the Radeon only: Intel's
    /// shared local memory is banked differently.
    fn degree(addrs: impl Iterator<Item = usize>) -> usize {
        let mut banks: Vec<Vec<usize>> = vec![Vec::new(); 32];
        for a in addrs {
            let b = &mut banks[a % 32];
            if !b.contains(&a) {
                b.push(a);
            }
        }
        banks.iter().map(Vec::len).max().unwrap_or(0)
    }

    /// Worst degree over the 8 waves of a workgroup and the `n` accesses
    /// of one kind, given each lane's address for access `q`.
    fn worst(n: usize, addr: impl Fn(usize, usize) -> usize) -> usize {
        let mut w = 1;
        for wave in 0..8 {
            for q in 0..n {
                w = w.max(degree((32 * wave..32 * wave + 32).map(|t| addr(t, q))));
            }
        }
        w
    }

    /// Bank conflict degrees of one kernel: stores of the A and B tiles
    /// when K is the contiguous axis (every nn.Linear) and when it is not,
    /// then the inner loop's reads of A and B.
    fn conflicts(t: &Tile) -> [usize; 6] {
        let pad = match t.layout {
            Layout::Original => 1,
            Layout::Blocked => 4,
            Layout::Interleaved { pad, .. } => pad,
        };
        let (sa, sb, bk, bm, bn) = (t.bm + pad, t.bn + pad, t.bk, t.bm, t.bn);
        let (tx, ty) = (t.bn / t.tn, t.bm / t.tm);
        let store_a_fast = worst(1, |e, _| (e % bk) * sa + e / bk);
        let store_a_slow = worst(1, |e, _| (e / bm) * sa + e % bm);
        let store_b_fast = worst(1, |e, _| (e % bk) * sb + e / bk);
        let store_b_slow = worst(1, |e, _| (e / bn) * sb + e % bn);
        let (read_a, read_b) = match t.layout {
            Layout::Blocked => (
                worst(t.tm, |id, q| (id / tx) * t.tm + q),
                worst(t.tn, |id, q| (id % tx) * t.tn + q),
            ),
            _ => (
                worst(t.tm, |id, q| id / tx + ty * q),
                worst(t.tn, |id, q| id % tx + tx * q),
            ),
        };
        [
            store_a_fast,
            store_a_slow,
            store_b_fast,
            store_b_slow,
            read_a,
            read_b,
        ]
    }

    /// The model reproduces the Radeon ranking of the older kernels (B
    /// reads: s64 none, stride-4 kernels 2-way, stride-8 kernels 4-way)
    /// and finds 2-way conflicts on s64's and v64's k-contiguous stores.
    /// Every interleaved kernel is conflict-free throughout.
    #[test]
    fn bank_conflicts() {
        let by_name = |n: &str| TILES.iter().find(|t| t.name == n).expect(n);
        assert_eq!(conflicts(by_name("s64")), [2, 1, 2, 1, 1, 1]);
        assert_eq!(conflicts(by_name("v64")), [2, 1, 2, 1, 1, 2]);
        assert_eq!(conflicts(by_name("v128x64")), [1, 1, 1, 1, 1, 2]);
        assert_eq!(conflicts(by_name("v64x128")), [1, 1, 1, 1, 1, 4]);
        assert_eq!(conflicts(by_name("v128")), [1, 1, 1, 1, 1, 4]);
        for t in TILES
            .iter()
            .filter(|t| matches!(t.layout, Layout::Interleaved { .. }))
        {
            assert_eq!(conflicts(t), [1; 6], "{}", t.name);
        }
    }

    /// Every kernel fits WebGPU's default 16 KiB of workgroup memory.
    #[test]
    fn workgroup_memory_within_defaults() {
        for t in &TILES {
            let pad = match t.layout {
                Layout::Original => 1,
                Layout::Blocked => 4,
                Layout::Interleaved { pad, .. } => pad,
            };
            let bytes = 4 * t.bk * (t.bm + t.bn + 2 * pad);
            assert!(bytes <= 16 * 1024, "{}: {bytes} bytes", t.name);
        }
    }

    /// The generated interleaved source has every placeholder filled.
    #[test]
    fn interleaved_source_is_complete() {
        for t in &TILES {
            let src = t.source();
            assert!(!src.contains('{') || !src.contains("{BM}"), "{}", t.name);
            for p in [
                "{ACC_DECLS}",
                "{STAGE}",
                "{COMPUTE}",
                "{EPILOGUE}",
                "{PAD}",
                "{TX}",
            ] {
                assert!(!src.contains(p), "{}: {p}", t.name);
            }
        }
    }
}
