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
//! Every buffer [`Gpu`] makes is a [`GpuBuffer`], destroyed when dropped.
//! [`net::Net`] holds the weights on the GPU and [`net::Job`] runs one
//! chunk as a list of [`net::Step`]s, so the caller can wait for the GPU
//! after each step and keep every command buffer short.

pub mod bench;
pub mod net;

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::num::NonZeroU64;
use std::ops::Deref;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

/// Bytes reserved for one dispatch's parameters (WebGPU's default
/// `minUniformBufferOffsetAlignment`).
const PARAM_STRIDE: u64 = 256;
/// Dispatches one [`Rec`] may hold.
const MAX_DISPATCHES: u64 = 64;

/// A matmul kernel: `matmul_interleaved.wgsl` with these numbers filled
/// in (or, with `hand_written`, `matmul.wgsl` as it stands, which has the
/// same numbers; with `vec4`, `matmul_vec4.wgsl`).  A workgroup of 256
/// threads computes a `bm` x `bn` block of the output, each thread `tm` x
/// `tn` of it (every `bm / tm`-th row and every `bn / tn`-th column),
/// stepping through K `bk` at a time; rows of the workgroup tiles are
/// padded by `pad` floats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tile {
    pub name: &'static str,
    pub bm: usize,
    pub bn: usize,
    pub bk: usize,
    pub tm: usize,
    pub tn: usize,
    pub pad: usize,
    pub hand_written: bool,
    /// Reads A and B as vec4s along K; runs only products that pass
    /// [`MatMul::vec4_ok`], and [`TILES`]`[0]` runs the rest.
    pub vec4: bool,
}

impl Tile {
    fn source(&self) -> String {
        if self.hand_written {
            return include_str!("shaders/matmul.wgsl").to_string();
        }
        if self.vec4 {
            return self.vec4_source();
        }
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
        // One K tile into workgroup memory, element tid + 256 l of each
        // tile.  A loop rather than unrolled copies: nothing here needs a
        // constant index, and SwiftShader compiled unrolled branchy loads
        // for minutes, with gigabytes of memory.
        let stage = format!(
            "    for (var l = 0u; l < {la}u; l++) {{ let e = tid + l * 256u; as_[a_slot(e)] = a_get(e, k0, row0, a_base); }}\n    for (var l = 0u; l < {lb}u; l++) {{ let e = tid + l * 256u; bs_[b_slot(e)] = b_get(e, k0, col0, b_base); }}"
        );
        include_str!("shaders/matmul_interleaved.wgsl")
            .replace("{BM}", &self.bm.to_string())
            .replace("{BN}", &self.bn.to_string())
            .replace("{BK}", &self.bk.to_string())
            .replace("{TX}", &tx.to_string())
            .replace("{TY}", &ty.to_string())
            .replace("{PAD}", &self.pad.to_string())
            .replace("{SA_LEN}", &(self.bk * (self.bm + self.pad)).to_string())
            .replace("{SB_LEN}", &(self.bk * (self.bn + self.pad)).to_string())
            .replace("{ACC_DECLS}", decls.trim_end())
            .replace("{STAGE}", &stage)
            .replace("{COMPUTE}", compute.trim_end())
            .replace("{EPILOGUE}", epilogue.trim_end())
    }

    /// The kernel of this tile for weights held in `format`: its f32
    /// kernel, or its f16 variant.
    pub fn source_for(&self, format: WeightFormat) -> String {
        match format {
            WeightFormat::F32 => self.source(),
            WeightFormat::F16 => self.f16_source(),
        }
    }

    /// The f16 variant: the f32 kernel with B declared as packed f16 values
    /// (`array<u32>`, or `array<vec2<u32>>` for a vec4 tile), and each B
    /// value unpacked with `unpack2x16float` where it is read.  The rest of
    /// the kernel, and so the order of the products and sums, is the same.
    fn f16_source(&self) -> String {
        const SCALAR_B: &str = "@group(0) @binding(1) var<storage, read> b: array<f32>;";
        const PACKED_B: &str =
            "@group(0) @binding(1) var<storage, read> b: array<u32>; // f16 pairs";
        let (decl, read) = if self.hand_written {
            (
                (SCALAR_B, PACKED_B),
                (
                    "      if (gj < p.n && gkb < p.k) { bv = b[b_base + gkb * p.b_sk + gj * p.b_sj]; }",
                    "      if (gj < p.n && gkb < p.k) { let e2 = b_base + gkb * p.b_sk + gj * p.b_sj; let pair = unpack2x16float(b[e2 / 2u]); bv = select(pair.x, pair.y, (e2 & 1u) == 1u); }",
                ),
            )
        } else if self.vec4 {
            (
                (
                    "@group(0) @binding(1) var<storage, read> b: array<vec4<f32>>;",
                    "@group(0) @binding(1) var<storage, read> b: array<vec2<u32>>; // f16 quads",
                ),
                (
                    "  let v = b[select(0u, (base + gj * p.b_sj + gk) / 4u, ok)];",
                    "  let q = b[select(0u, (base + gj * p.b_sj + gk) / 4u, ok)];\n  let v = vec4<f32>(unpack2x16float(q.x), unpack2x16float(q.y));",
                ),
            )
        } else {
            (
                (SCALAR_B, PACKED_B),
                (
                    "  let v = b[select(0u, base + gk * p.b_sk + gj * p.b_sj, ok)];",
                    "  let e2 = select(0u, base + gk * p.b_sk + gj * p.b_sj, ok);\n  let pair = unpack2x16float(b[e2 / 2u]);\n  let v = select(pair.x, pair.y, (e2 & 1u) == 1u);",
                ),
            )
        };
        let header = "// The f16 variant (decision 0018): B holds f16 weights, element 2 j in\n// the low half of u32 j, each unpacked to f32 where it is read.\n";
        let src = replace_once(&self.source(), decl.0, decl.1);
        format!("{header}{}", replace_once(&src, read.0, read.1))
    }

    /// `matmul_vec4.wgsl` for this tile: tiles stored `[row][k / 4]`, so
    /// one step of the inner loop covers 4 values of K.  The products are
    /// added in K order, component by component, as the scalar kernel adds
    /// them.
    fn vec4_source(&self) -> String {
        let (tx, ty) = (self.bn / self.tn, self.bm / self.tm);
        assert_eq!(tx * ty, 256, "{self:?}");
        assert!(
            self.bk.is_multiple_of(4) && self.pad.is_multiple_of(4),
            "{self:?}"
        );
        let (kv, sv) = (self.bk / 4, (self.bk + self.pad) / 4);
        assert!(
            (self.bm * kv).is_multiple_of(256) && (self.bn * kv).is_multiple_of(256),
            "{self:?}"
        );
        let (la, lb) = (self.bm * kv / 256, self.bn * kv / 256);
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
            compute += &format!("      let a{i} = as_[ab + {}u];\n", ty * i * sv);
        }
        for j in 0..self.tn {
            compute += &format!("      let b{j} = bs_[bb + {}u];\n", tx * j * sv);
        }
        for c in ["x", "y", "z", "w"] {
            for i in 0..self.tm {
                for j in 0..self.tn {
                    compute +=
                        &format!("      acc_{i}_{j} = fma(a{i}.{c}, b{j}.{c}, acc_{i}_{j});\n");
                }
            }
        }
        let stage = format!(
            "    for (var l = 0u; l < {la}u; l++) {{ let e = tid + l * 256u; as_[slot(e)] = a_get(e, k0, row0, a_base); }}\n    for (var l = 0u; l < {lb}u; l++) {{ let e = tid + l * 256u; bs_[slot(e)] = b_get(e, k0, col0, b_base); }}"
        );
        include_str!("shaders/matmul_vec4.wgsl")
            .replace("{BM}", &self.bm.to_string())
            .replace("{BN}", &self.bn.to_string())
            .replace("{BK}", &self.bk.to_string())
            .replace("{TX}", &tx.to_string())
            .replace("{TY}", &ty.to_string())
            .replace("{PADV}", &(self.pad / 4).to_string())
            .replace("{SA_LEN}", &(self.bm * sv).to_string())
            .replace("{SB_LEN}", &(self.bn * sv).to_string())
            .replace("{ACC_DECLS}", decls.trim_end())
            .replace("{STAGE}", &stage)
            .replace("{COMPUTE}", compute.trim_end())
            .replace("{EPILOGUE}", epilogue.trim_end())
    }
}

/// `src` with `from` replaced by `to`.  Panics unless `from` occurs exactly
/// once, so a change to a template cannot leave an f16 variant reading f32.
fn replace_once(src: &str, from: &str, to: &str) -> String {
    let count = src.matches(from).count();
    assert_eq!(count, 1, "{from:?} occurs {count} times, not once");
    src.replacen(from, to, 1)
}

/// How the GPU holds a weight matrix (decision 0018).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WeightFormat {
    /// One f32 value in each 4 bytes, as activations are held.
    F32,
    /// Two f16 values in each `u32`: element `2 j` in the low 16 bits of
    /// word `j`.  The f16 variants of the matmul kernels unpack each value
    /// to f32 and compute in f32.
    F16,
}

impl WeightFormat {
    pub const ALL: [Self; 2] = [Self::F32, Self::F16];

    /// `f32` or `f16`.
    pub fn name(self) -> &'static str {
        match self {
            Self::F32 => "f32",
            Self::F16 => "f16",
        }
    }

    fn index(self) -> usize {
        match self {
            Self::F32 => 0,
            Self::F16 => 1,
        }
    }
}

/// A weight matrix on the GPU: a [`GpuBuffer`] and the [`WeightFormat`] of
/// its values.  It has no `Deref`, so its buffer cannot pass as an
/// activation: a [`MatMul`] takes it as B and reads the format from it.
pub struct Weight {
    buffer: GpuBuffer,
    format: WeightFormat,
}

impl Weight {
    pub fn format(&self) -> WeightFormat {
        self.format
    }

    /// Bytes of its buffer.
    pub fn size(&self) -> u64 {
        self.buffer.size()
    }
}

/// The B operand of a [`MatMul`]: an f32 activation, or a [`Weight`] in
/// either format.
#[derive(Clone, Copy)]
pub enum Operand<'a> {
    Activation(&'a wgpu::Buffer),
    Weight(&'a Weight),
}

impl<'a> Operand<'a> {
    fn buffer(self) -> &'a wgpu::Buffer {
        match self {
            Operand::Activation(b) => b,
            Operand::Weight(w) => &w.buffer,
        }
    }

    /// The format of its values: an activation is f32.
    pub fn format(self) -> WeightFormat {
        match self {
            Operand::Activation(_) => WeightFormat::F32,
            Operand::Weight(w) => w.format,
        }
    }
}

impl<'a> From<&'a wgpu::Buffer> for Operand<'a> {
    fn from(b: &'a wgpu::Buffer) -> Self {
        Operand::Activation(b)
    }
}

impl<'a> From<&'a GpuBuffer> for Operand<'a> {
    fn from(b: &'a GpuBuffer) -> Self {
        Operand::Activation(b)
    }
}

impl<'a> From<&'a Weight> for Operand<'a> {
    fn from(w: &'a Weight) -> Self {
        Operand::Weight(w)
    }
}

/// The bytes that `Gpu::upload_f16` uploads for `bits`: element `2 j` in
/// the low half of the little-endian `u32` `j`, an odd count padded with
/// one zero value, and the size rounded up to whole 16-byte groups (at
/// least one), as [`Gpu::buffer`] rounds.  A whole number of groups on a
/// little-endian host is the bytes of `bits` as they are, with no copy.
fn f16_bytes(bits: &[u16]) -> Cow<'_, [u8]> {
    let size = (2 * bits.len()).max(16).next_multiple_of(16);
    if cfg!(target_endian = "little") && 2 * bits.len() == size {
        return Cow::Borrowed(bytemuck::cast_slice(bits));
    }
    let mut out: Vec<u8> = bits.iter().flat_map(|v| v.to_le_bytes()).collect();
    out.resize(size, 0);
    Cow::Owned(out)
}

/// How two outputs differ by their bits: how many values differ, and the
/// largest absolute difference (infinite where either is not finite).
/// `+0` and `-0` count as equal, since WGSL lets a GPU ignore the sign of
/// a zero.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BitDifferences {
    pub count: usize,
    pub largest: f32,
}

/// Compares `got` with `want` value by value (see [`BitDifferences`]).
pub fn bit_differences(got: &[f32], want: &[f32]) -> BitDifferences {
    assert_eq!(got.len(), want.len());
    let mut out = BitDifferences {
        count: 0,
        largest: 0.0,
    };
    for (g, w) in got.iter().zip(want) {
        let same = g.to_bits() == w.to_bits() || (*g == 0.0 && *w == 0.0);
        if !same {
            out.count += 1;
            let d = (g - w).abs();
            out.largest = out
                .largest
                .max(if d.is_finite() { d } else { f32::INFINITY });
        }
    }
    out
}

/// The matmul kernels, among which [`bench::calibrate`] measures once per
/// GPU.  `i64` and `i128x64` follow the same rules (accumulators in named
/// registers, neighbouring threads on neighbouring floats, conflict-free
/// workgroup memory, WebGPU's default limits) and differ only in the
/// block each thread keeps in registers: 4 x 4 or 8 x 4.  The larger block
/// gets more use from each workgroup-memory read (2.67 multiply-adds per
/// read against 2) but needs twice the accumulators, so fewer threads fit
/// at once to hide memory latency.  Which wins depends on the GPU's
/// register file and scheduler, which WebGPU does not report.  Against the
/// best kernel per shape, `i64` was 11% slower on a Radeon Pro 5500M, 22%
/// on an Intel UHD 630 and 3% on an iPad's GPU; `i128x64` 22%, 2% and 5%.
///
/// `s64`, the original hand-written kernel, has `i64`'s layout but keeps
/// its accumulators in a loop-indexed array.  It depends on the compiler
/// unrolling that loop into registers: AMD's does, and there it was the
/// fastest of the three (6% ahead of `i64` in `?bench`, 15% on the
/// matmuls of a profiled self-test); Intel's and Apple's do not, and there
/// it was 5 to 6 times slower, so calibration stops timing it after its
/// first round on such GPUs.
///
/// `v64` is `i64` with vector memory traffic: A and B come from storage
/// and from workgroup memory as vec4s along K, so the inner loop issues a
/// quarter of `i64`'s workgroup-memory reads for the same multiply-adds
/// (8 vec4 reads per 64, against 8 floats per 16).  Every load in the
/// scalar kernels is one f32; on an iPad's GPU they ran at 8 to 11% of
/// peak with little DRAM traffic, which looks issue-bound rather than
/// memory-bound.  It runs only products whose vec4s are whole and aligned
/// ([`MatMul::vec4_ok`]: every nn.Linear product, band split and mask
/// estimators included, and the attention scores); asked for any other
/// product (the p.v product: K = 62 or 1,151, B not K-contiguous), the
/// fallback `i64` runs it instead.
///
/// The first entry is the fallback: a GPU on which it fails is not used.
/// Rows are padded by 32 / `bk` floats (`s64`: 1; `v64`: one vec4, so
/// neighbouring threads' vec4 reads of the B tile fall in different
/// banks): when K is the contiguous axis (every nn.Linear operand), 32
/// consecutive threads store 32 / `bk` rows of `bk` values, and this
/// padding spreads them over all 32 banks.
pub const TILES: [Tile; 4] = [
    Tile {
        name: "i64",
        bm: 64,
        bn: 64,
        bk: 16,
        tm: 4,
        tn: 4,
        pad: 2,
        hand_written: false,
        vec4: false,
    },
    Tile {
        name: "i128x64",
        bm: 128,
        bn: 64,
        bk: 8,
        tm: 8,
        tn: 4,
        pad: 4,
        hand_written: false,
        vec4: false,
    },
    Tile {
        name: "s64",
        bm: 64,
        bn: 64,
        bk: 16,
        tm: 4,
        tn: 4,
        pad: 1,
        hand_written: true,
        vec4: false,
    },
    Tile {
        name: "v64",
        bm: 64,
        bn: 64,
        bk: 16,
        tm: 4,
        tn: 4,
        pad: 4,
        hand_written: false,
        vec4: true,
    },
];

/// Where [`net::Job`] runs fused attention (`attention_tiled.wgsl`:
/// scores, softmax, values and gate in one dispatch, the scores never in
/// memory) instead of the scores matmul, softmax, p.v matmul and gate.
/// The frequency axis (sequences of 62 bands) gains on every GPU measured,
/// 2.2 to 3.6 times; the time axis (1,151 frames) gained 11 to 22% on
/// Apple GPUs and lost 20% on an Intel UHD 630, so [`bench::calibrate`]
/// decides it per GPU.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttentionPolicy {
    pub time: bool,
    pub freq: bool,
}

impl AttentionPolicy {
    pub const SPLIT: Self = Self {
        time: false,
        freq: false,
    };
    pub const FUSED: Self = Self {
        time: true,
        freq: true,
    };

    /// `split`, `fused`, or `time` / `freq` for fused on that axis only.
    pub fn name(self) -> &'static str {
        match (self.time, self.freq) {
            (false, false) => "split",
            (true, true) => "fused",
            (true, false) => "time",
            (false, true) => "freq",
        }
    }

    /// The inverse of [`AttentionPolicy::name`].
    pub fn parse(s: &str) -> Option<Self> {
        [
            Self::SPLIT,
            Self::FUSED,
            Self {
                time: true,
                freq: false,
            },
            Self {
                time: false,
                freq: true,
            },
        ]
        .into_iter()
        .find(|p| p.name() == s)
    }
}

/// The fused attention kernel's WGSL: per-thread code with constant
/// indices, generated into `attention_tiled.wgsl`.
fn attention_source() -> String {
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

/// The `len` floats of a mapped read-back, copied once into wasm memory.
/// On the web, wgpu's `BufferView` copies the whole mapping into wasm
/// memory when it is first read as a slice.  So the web path copies
/// straight from the browser's mapped array instead (MEM-1,
/// `a_read_back_is_copied_into_wasm_memory_once` in the worker's tests).
#[cfg(target_arch = "wasm32")]
fn mapped_floats(view: &wgpu::BufferView, len: usize) -> Vec<f32> {
    use wgpu::web_sys::js_sys::Float32Array;
    let bytes = view.as_uint8array();
    assert_eq!(bytes.length() as usize, 4 * len);
    let floats = Float32Array::new_with_byte_offset_and_length(
        &bytes.buffer(),
        bytes.byte_offset(),
        len as u32,
    );
    let mut out = vec![0.0f32; len];
    floats.copy_to(&mut out);
    out
}

/// The `len` floats of a mapped read-back, copied once.
#[cfg(not(target_arch = "wasm32"))]
fn mapped_floats(view: &wgpu::BufferView, len: usize) -> Vec<f32> {
    let floats = bytemuck::cast_slice::<u8, f32>(view);
    assert_eq!(floats.len(), len);
    #[expect(
        clippy::disallowed_methods,
        reason = "MEM-1: the one copy of a read-back out of mapped GPU memory, which is unmapped next"
    )]
    floats.to_vec()
}

/// A buffer made by [`Gpu`], destroyed when dropped.  wgpu's web backend
/// does nothing when a `wgpu::Buffer` is dropped, so its memory stays
/// allocated until JavaScript collects the object, and Safari's collector
/// is not told how much memory WebGPU objects hold (WebKit bug 283321), so
/// it collects them late: on an iPhone, chunk buffers and models that were
/// dropped but not destroyed piled up until the tab was killed.
/// Destroying a buffer whose work was submitted is valid (the work
/// completes first), and a pending mapping is cancelled.  [`Rec`] borrows
/// its buffers until it is submitted, so none can be dropped while a
/// recorded dispatch still needs it.  Make buffers with [`Gpu::buffer`]
/// and [`Gpu::upload`] only.  Clippy fails five ways to make a buffer in
/// this crate outside `GpuBuffer`'s constructors (MEM-3).  No other crate reaches the device.  A
/// clone of the inner `wgpu::Buffer` is destroyed with it, so do not use
/// a clone after its `GpuBuffer` drops.
pub struct GpuBuffer {
    buffer: wgpu::Buffer,
    /// [`Gpu::live_bytes`], less this buffer's size once it is dropped.
    live: Arc<AtomicU64>,
}

impl GpuBuffer {
    fn new(device: &wgpu::Device, desc: &wgpu::BufferDescriptor, live: &Arc<AtomicU64>) -> Self {
        #[expect(
            clippy::disallowed_methods,
            reason = "MEM-3: the one call that makes a buffer from a descriptor.  The `GpuBuffer` destroys it when dropped"
        )]
        let buffer = device.create_buffer(desc);
        Self::counted(buffer, live)
    }

    fn with_contents(
        device: &wgpu::Device,
        desc: &wgpu::util::BufferInitDescriptor,
        live: &Arc<AtomicU64>,
    ) -> Self {
        #[expect(
            clippy::disallowed_methods,
            reason = "MEM-3: the one call that makes a buffer with contents.  The `GpuBuffer` destroys it when dropped"
        )]
        let buffer = device.create_buffer_init(desc);
        Self::counted(buffer, live)
    }

    fn counted(buffer: wgpu::Buffer, live: &Arc<AtomicU64>) -> Self {
        live.fetch_add(buffer.size(), Ordering::Relaxed);
        Self {
            buffer,
            live: live.clone(),
        }
    }
}

impl Deref for GpuBuffer {
    type Target = wgpu::Buffer;

    fn deref(&self) -> &wgpu::Buffer {
        &self.buffer
    }
}

impl Drop for GpuBuffer {
    fn drop(&mut self) {
        self.buffer.destroy();
        self.live.fetch_sub(self.buffer.size(), Ordering::Relaxed);
    }
}

/// The device, its kernels and the buffers it makes.  The device is
/// private, so no other crate can make a buffer on it that is not a
/// [`GpuBuffer`] (MEM-3).
///
/// ```no_run
/// fn limits(gpu: &roformer_gpu::Gpu) -> u32 {
///     gpu.limits().max_compute_invocations_per_workgroup
/// }
/// ```
///
/// ```compile_fail,E0616
/// fn limits(gpu: &roformer_gpu::Gpu) -> u32 {
///     gpu.device.limits().max_compute_invocations_per_workgroup
/// }
/// ```
///
/// ```compile_fail,E0624
/// fn wrap(
///     device: wgpu::Device,
///     queue: wgpu::Queue,
///     info: wgpu::AdapterInfo,
///     flags: wgpu::DownlevelFlags,
/// ) {
///     roformer_gpu::Gpu::from_device(device, queue, info, flags);
/// }
/// ```
pub struct Gpu {
    device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub info: wgpu::AdapterInfo,
    /// Which matmul kernel [`Rec::matmul`] uses unless a [`MatMul`] names one.
    pub policy: std::cell::Cell<MatMulPolicy>,
    /// Kernels of [`TILES`] that passed [`Gpu::check_kernels`] on this GPU
    /// (all, until it runs).  The policy never picks one that failed.
    pub usable: std::cell::Cell<[bool; TILES.len()]>,
    /// Where [`net::Job`] runs fused attention; split until set.  Set it
    /// to anything but split only when [`Gpu::fused_ok`] is true.
    pub attention: std::cell::Cell<AttentionPolicy>,
    /// Whether the fused attention kernel passed [`Gpu::check_attention`]
    /// (false until it runs).
    pub fused_ok: std::cell::Cell<bool>,
    /// Whether the f16 variant of every kernel that passed
    /// [`Gpu::check_kernels`] gave its f32 kernel's output bit for bit
    /// there (false until it runs, and on a GPU without
    /// [`Gpu::has_f16_unpack`]).  Weights go to the GPU as f16 only when
    /// it is true (decision 0022).
    pub f16_ok: std::cell::Cell<bool>,
    /// The adapter's downlevel flags.  `SHADER_F16_IN_F32` lets the
    /// device compile `unpack2x16float`, which the f16 variants call.
    downlevel: wgpu::DownlevelFlags,
    kernels: Kernels,
    /// Bound where a kernel has an input it does not use for this dispatch.
    dummy: GpuBuffer,
    uniforms: GpuBuffer,
    errors: Arc<Mutex<Vec<String>>>,
    /// GPU timestamps per dispatch, on a device opened for profiling.
    prof: Option<Prof>,
    /// Whether profiled submissions record timestamps now; benchmarks
    /// and calibration turn it off (see [`Gpu::set_recording`]).
    recording: std::cell::Cell<bool>,
    /// Bytes of storage buffers made by [`Gpu::buffer`] and
    /// [`Gpu::upload`] so far, for memory reports (on iOS a page's GPU
    /// buffers count towards its tab's memory limit).
    created: std::cell::Cell<u64>,
    /// Bytes of every [`GpuBuffer`] this GPU made that is not yet dropped.
    live: Arc<AtomicU64>,
}

/// Timestamp queries around every dispatch of a profiled [`Gpu`].  Each
/// profiled submission runs one compute pass per dispatch with a timestamp
/// at either end, resolves them, and copies them to its own readback
/// buffer; [`Gpu::collect_profile`] adds the finished ones to the totals.
/// Splitting a step into one pass per dispatch costs a little speed, so
/// only `?profile` runs open a device this way.
struct Prof {
    set: wgpu::QuerySet,
    resolve: GpuBuffer,
    /// Nanoseconds per timestamp tick.
    period: f64,
    /// Readback buffers of submissions not yet collected, with the label
    /// of each dispatch in them.
    pending: RefCell<Vec<(Vec<String>, GpuBuffer)>>,
    /// Dispatch count and GPU seconds per label.
    totals: RefCell<BTreeMap<String, (usize, f64)>>,
}

/// One matmul pipeline for each [`WeightFormat`], compiled on first use.
type FormatPipelines = [std::cell::OnceCell<wgpu::ComputePipeline>; 2];

struct Kernels {
    /// One pipeline per entry of [`TILES`] and [`WeightFormat`].  Both
    /// formats bind B as a read-only storage buffer, so they share a layout.
    matmul: (Vec<FormatPipelines>, wgpu::BindGroupLayout),
    softmax: (wgpu::ComputePipeline, wgpu::BindGroupLayout),
    rmsnorm: (wgpu::ComputePipeline, wgpu::BindGroupLayout),
    rope: wgpu::ComputePipeline,
    gate: wgpu::ComputePipeline,
    transpose: wgpu::ComputePipeline,
    glu: wgpu::ComputePipeline,
    elementwise: wgpu::BindGroupLayout,
    /// The fused attention kernel, compiled on first use; bound like the
    /// element-wise kernels (read, read, read-write, uniform).
    attention: std::cell::OnceCell<wgpu::ComputePipeline>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Bind {
    Read,
    ReadWrite,
    Uniform,
}

/// The bindings of each layout, in binding order, all in group 0.
/// `every_layout_matches_its_shaders` checks them against the shaders.
const MATMUL_BINDS: &[Bind] = &[
    Bind::Read,
    Bind::Read,
    Bind::ReadWrite,
    Bind::Read,
    Bind::Uniform,
];
const SOFTMAX_BINDS: &[Bind] = &[Bind::ReadWrite, Bind::Uniform];
const RMSNORM_BINDS: &[Bind] = &[Bind::Read, Bind::Read, Bind::ReadWrite, Bind::Uniform];
const ELEMENTWISE_BINDS: &[Bind] = &[Bind::Read, Bind::Read, Bind::ReadWrite, Bind::Uniform];

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
        let mm = bind_layout(device, "matmul", MATMUL_BINDS);
        let sm = bind_layout(device, "softmax", SOFTMAX_BINDS);
        let rn = bind_layout(device, "rmsnorm", RMSNORM_BINDS);
        let ew = bind_layout(device, "elementwise", ELEMENTWISE_BINDS);
        // Matmul pipelines compile on first use (see `matmul_pipeline`).
        let matmul = TILES.iter().map(|_| Default::default()).collect();
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
            attention: std::cell::OnceCell::new(),
        }
    }

    fn attention_pipeline(&self, device: &wgpu::Device) -> &wgpu::ComputePipeline {
        self.attention.get_or_init(|| {
            pipeline(
                device,
                &shader(device, "attention", &attention_source()),
                "main",
                &self.elementwise,
            )
        })
    }

    /// The pipeline of matmul kernel `t` for weights in `format`, compiled
    /// the first time it is needed: a device compiles only the kernels it
    /// runs (all of them once in [`Gpu::check_kernels`], where a kernel the
    /// compiler rejects is reported and never used again).  Compiling
    /// every kernel up front took SwiftShader over 4 GB per device.
    fn matmul_pipeline(
        &self,
        device: &wgpu::Device,
        t: usize,
        format: WeightFormat,
    ) -> &wgpu::ComputePipeline {
        self.matmul.0[t][format.index()].get_or_init(|| {
            let tile = &TILES[t];
            let label = format!("{} {}", tile.name, format.name());
            pipeline(
                device,
                &shader(device, &label, &tile.source_for(format)),
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
        let downlevel = adapter.get_downlevel_capabilities().flags;
        Ok(Self::from_device(
            device,
            queue,
            adapter.get_info(),
            downlevel,
        ))
    }

    fn from_device(
        device: wgpu::Device,
        queue: wgpu::Queue,
        info: wgpu::AdapterInfo,
        downlevel: wgpu::DownlevelFlags,
    ) -> Self {
        let errors: Arc<Mutex<Vec<String>>> = Default::default();
        let sink = errors.clone();
        device.on_uncaptured_error(Arc::new(move |e: wgpu::Error| {
            if let Ok(mut v) = sink.lock() {
                v.push(e.to_string());
            }
        }));
        let kernels = Kernels::new(&device);
        let live = Arc::new(AtomicU64::new(0));
        let make = |desc: &wgpu::BufferDescriptor| GpuBuffer::new(&device, desc, &live);
        let dummy = make(&wgpu::BufferDescriptor {
            label: Some("dummy"),
            size: 16,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let uniforms = make(&wgpu::BufferDescriptor {
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
                resolve: make(&wgpu::BufferDescriptor {
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
            attention: std::cell::Cell::new(AttentionPolicy::SPLIT),
            fused_ok: std::cell::Cell::new(false),
            f16_ok: std::cell::Cell::new(false),
            downlevel,
            kernels,
            dummy,
            uniforms,
            errors,
            prof,
            recording: std::cell::Cell::new(true),
            created: std::cell::Cell::new(0),
            live,
        }
    }

    /// A buffer described by `desc`, counted in [`Gpu::live_bytes`].
    fn create(&self, desc: &wgpu::BufferDescriptor) -> GpuBuffer {
        GpuBuffer::new(&self.device, desc, &self.live)
    }

    /// The device's limits: WebGPU's defaults.
    pub fn limits(&self) -> wgpu::Limits {
        self.device.limits()
    }

    /// The device's optional features: `timestamp-query` on a profiling
    /// device that has it, else none.
    pub fn features(&self) -> wgpu::Features {
        self.device.features()
    }

    /// True when this device records GPU time per dispatch.
    pub fn profiling(&self) -> bool {
        self.prof.is_some()
    }

    /// True when the adapter has `DownlevelFlags::SHADER_F16_IN_F32`, so
    /// the f16 variants compile.  Without it, weights stay f32.
    pub fn has_f16_unpack(&self) -> bool {
        self.downlevel
            .contains(wgpu::DownlevelFlags::SHADER_F16_IN_F32)
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

    /// Turns timestamp recording on a profiling device on or off and
    /// returns the previous setting.  Benchmarks and calibration turn it
    /// off: they submit many command buffers without collecting their
    /// timestamps, which would otherwise pile up as readback buffers and
    /// land in the next run's report.
    pub fn set_recording(&self, on: bool) -> bool {
        self.recording.replace(on)
    }

    /// Starts a fresh profile: waits for submitted work, reads back any
    /// timestamps not yet collected (start-up checks, earlier runs) and
    /// discards the totals, so a report covers only what follows.  Does
    /// nothing on a device that is not profiling.
    pub async fn reset_profile(&self) -> Result<(), String> {
        if self.prof.is_none() {
            return Ok(());
        }
        self.done().await;
        self.collect_profile().await?;
        self.take_profile();
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

    /// A storage buffer of `len` f32s, zeroed, rounded up to whole vec4s
    /// (16 bytes) so the vec4 kernels can read it (see
    /// [`MatMul::vec4_ok`]).
    pub fn buffer(&self, label: &str, len: usize) -> GpuBuffer {
        let size = (len.max(4).next_multiple_of(4) * 4) as u64;
        self.created.set(self.created.get() + size);
        self.create(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        })
    }

    /// A storage buffer holding `data`, rounded up to whole vec4s like
    /// [`Gpu::buffer`] (the extra floats zero).
    pub fn upload(&self, label: &str, data: &[f32]) -> GpuBuffer {
        if data.is_empty() {
            return self.buffer(label, 0);
        }
        if !data.len().is_multiple_of(4) {
            let buf = self.buffer(label, data.len());
            self.write(&buf, 0, data);
            return buf;
        }
        self.created
            .set(self.created.get() + std::mem::size_of_val(data) as u64);
        GpuBuffer::with_contents(
            &self.device,
            &wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytemuck::cast_slice(data),
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
            },
            &self.live,
        )
    }

    /// A weight matrix of f32 values, uploaded as [`Gpu::upload`] does.
    pub fn upload_weight(&self, label: &str, data: &[f32]) -> Weight {
        Weight {
            buffer: self.upload(label, data),
            format: WeightFormat::F32,
        }
    }

    /// A weight matrix of f16 values (`bits`, as the file holds them), two
    /// in each `u32`: element `2 j` in the low 16 bits of word `j`.  An odd
    /// count gets one zero value of padding, and the size rounds up to 16
    /// bytes as [`Gpu::buffer`] rounds, so [`MatMul::vec4_ok`] holds.  Only
    /// a GPU with [`Gpu::has_f16_unpack`] can read it.
    pub fn upload_f16(&self, label: &str, bits: &[u16]) -> Weight {
        assert!(
            self.has_f16_unpack(),
            "f16 weights on a GPU without SHADER_F16_IN_F32"
        );
        let bytes = f16_bytes(bits);
        self.created.set(self.created.get() + bytes.len() as u64);
        let buffer = GpuBuffer::with_contents(
            &self.device,
            &wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: &bytes,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
            },
            &self.live,
        );
        Weight {
            buffer,
            format: WeightFormat::F16,
        }
    }

    /// Bytes of storage buffers made by [`Gpu::buffer`] and
    /// [`Gpu::upload`] since this GPU opened (freed ones included).
    pub fn created_bytes(&self) -> u64 {
        self.created.get()
    }

    /// Bytes of every buffer this GPU made that is still allocated, each
    /// freed as it is dropped.
    pub fn live_bytes(&self) -> u64 {
        self.live.load(Ordering::Relaxed)
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
        // Destroyed on return, after `unmap` (or, on an error, with the
        // mapping cancelled): the copy into it was submitted by then.
        let staging = self.create(&wgpu::BufferDescriptor {
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
        let out = mapped_floats(&view, len);
        drop(view);
        staging.unmap();
        Ok(out)
    }

    /// Runs every matmul kernel on three small products (both operand
    /// layouts, bias, accumulate, ragged edges) and compares each with the
    /// CPU.  A kernel that disagrees is marked unusable on this GPU, so a
    /// compiler or driver bug in one kernel costs speed, not correctness.
    /// Returns one line per kernel that failed; the fallback kernel
    /// (`TILES[0]`, `i64`) failing is an error.  K is a multiple of 4 but
    /// not of any tile's `bk`, so cases 1 and 3 (both operands
    /// K-contiguous) run the vec4 kernel with a ragged last K tile, with
    /// bias and with an offset and accumulate; case 2 is not vec4-shaped
    /// and checks the fallback the vec4 kernel hands it to.
    ///
    /// On a GPU with [`Gpu::has_f16_unpack`] it then checks the f16
    /// variant of each kernel that passed (see [`Gpu::check_f16`]) and
    /// sets [`Gpu::f16_ok`].  A failure there adds a line and keeps the
    /// weights f32.
    pub async fn check_kernels(&self) -> Result<Vec<String>, String> {
        let (m, n, k) = (133, 141, 44);
        let rand = check_values;
        let (a, b, c0, bias) = (
            rand(m * k + 4, 1),
            rand(k * n, 2),
            rand(m * n, 3),
            rand(n, 4),
        );
        // Case 1: A row-major, B a torch-layout weight, bias.  Case 2: A
        // column-major, B row-major, accumulate into C.  Case 3: case 1's
        // layout with A 4 floats in, accumulating (the transformer's
        // residual products).
        let views = [
            (View::rows(0, k), View::t(0, k), false),
            (View::t(0, m), View::rows(0, n), true),
            (View::rows(4, k), View::t(0, k), true),
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
            let mut ran_itself = false;
            for (case, (va, vb, acc)) in views.iter().enumerate() {
                let cb = self.upload("check c", &c0);
                let mut mm = MatMul::new((&ab, *va), (&bb, *vb), (&cb, View::rows(0, n)), m, n, k);
                mm.tile = Some(t);
                ran_itself |= mm.kernel(t) == t;
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
            if usable[t] && !ran_itself {
                usable[t] = false;
                problems.push(format!(
                    "matmul kernel {} was not exercised by the start-up check; not used",
                    tile.name
                ));
            }
        }
        self.usable.set(usable);
        self.f16_ok.set(false);
        if !usable[0] {
            return Err(format!(
                "this GPU computes matrix products wrongly: {}",
                problems.join("; ")
            ));
        }
        if self.has_f16_unpack() {
            let f16_problems = self.check_f16(usable).await?;
            self.f16_ok.set(f16_problems.is_empty());
            problems.extend(f16_problems);
        }
        Ok(problems)
    }

    /// Runs the f16 variant of each kernel that `usable` marks, and its f32
    /// kernel on the same values as f32, and compares the two bit for bit
    /// (`+0` and `-0` equal, see [`bit_differences`]).  Returns one line
    /// for each kernel whose variant differs or fails.
    ///
    /// Cases 1 and 3 are the weight products of [`Gpu::check_kernels`]
    /// with f16-exact weights, about one in eight an f16 subnormal, and
    /// some `+0` and `-0`.  A sum of values near 1 can hide a subnormal
    /// that a GPU flushed to zero, so in case 4 each row of A is one-hot
    /// and B holds only f16 subnormals: each output is one subnormal, and
    /// a flush gives 0.  Its K is a multiple of 4 and its B K-contiguous,
    /// so the vec4 kernel runs its own variant.
    async fn check_f16(&self, usable: [bool; TILES.len()]) -> Result<Vec<String>, String> {
        let (m, n, k) = (133, 141, 44);
        let (m4, n4, k4) = (67, 70, 40);
        let a = check_values(m * k + 4, 1);
        let (c0, bias) = (check_values(m * n, 3), check_values(n, 4));
        let one_hot: Vec<f32> = (0..m4 * k4)
            .map(|e| if e % k4 == (e / k4) % k4 { 1.0 } else { 0.0 })
            .collect();
        let weights = f16_check_weights(k * n, 5, false);
        let subnormals = f16_check_weights(k4 * n4, 6, true);
        let as_f32 = |bits: &[u16]| -> Vec<f32> {
            bits.iter()
                .map(|&h| half::f16::from_bits(h).to_f32())
                .collect()
        };
        let (ab, a4b, biasb) = (
            self.upload("check a", &a),
            self.upload("check a one-hot", &one_hot),
            self.upload("check bias", &bias),
        );
        let w = [
            (
                self.upload_weight("check b f32", &as_f32(&weights)),
                self.upload_f16("check b f16", &weights),
            ),
            (
                self.upload_weight("check b4 f32", &as_f32(&subnormals)),
                self.upload_f16("check b4 f16", &subnormals),
            ),
        ];
        // (case, A, its view, weights, K, bias, accumulate, M, N)
        let cases = [
            (1, &ab, View::rows(0, k), &w[0], k, true, false, m, n),
            (3, &ab, View::rows(4, k), &w[0], k, false, true, m, n),
            (4, &a4b, View::rows(0, k4), &w[1], k4, false, false, m4, n4),
        ];
        let mut problems = Vec::new();
        for (t, tile) in TILES.iter().enumerate().filter(|(t, _)| usable[*t]) {
            for &(case, a_buf, va, (w32, w16), kk, with_bias, acc, mm_m, mm_n) in &cases {
                let mut outputs = Vec::new();
                for weight in [w32, w16] {
                    let cb = self.upload("check c", &c0[..mm_m * mm_n]);
                    let mut mm = MatMul::new(
                        (a_buf, va),
                        (weight, View::t(0, kk)),
                        (&cb, View::rows(0, mm_n)),
                        mm_m,
                        mm_n,
                        kk,
                    );
                    mm.tile = Some(t);
                    mm.accumulate = acc;
                    if with_bias {
                        mm.bias = Some((&biasb, 0, 0));
                    }
                    let mut r = self.rec();
                    r.matmul(&mm);
                    r.submit();
                    outputs.push(self.read(&cb, 0, mm_m * mm_n).await?);
                }
                let why = match self.take_errors().first() {
                    Some(e) => Some(e.clone()),
                    None => {
                        let d = bit_differences(&outputs[1], &outputs[0]);
                        (d.count > 0).then(|| {
                            format!(
                                "{} of {} values differ, the largest by {:e}",
                                d.count,
                                outputs[0].len(),
                                d.largest
                            )
                        })
                    }
                };
                if let Some(why) = why {
                    problems.push(format!(
                        "f16 weights: the f16 variant of matmul kernel {} does not give its f32 output on this GPU (case {case}: {why}); weights stay f32",
                        tile.name
                    ));
                    break;
                }
            }
        }
        Ok(problems)
    }

    /// Runs the fused attention kernel on a small ragged case (two
    /// sequences of 37 tokens, with gates) and compares it with
    /// [`attention_cpu`].  Sets [`Gpu::fused_ok`]; returns why it failed,
    /// if it did, and then the split path is the only one used.
    pub async fn check_attention(&self) -> Option<String> {
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
        let ob = self.buffer("check o", tokens * 512);
        let mut r = self.rec();
        r.attention(&qb, Some(&gb), &ob, seq, nseq, alpha);
        r.submit();
        let problem = match self.read(&ob, 0, tokens * 512).await {
            Err(e) => Some(format!("fused attention: {e}; using split attention")),
            Ok(got) => {
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
                    Some(format!(
                        "fused attention gives wrong results on this GPU ({why}); using split attention"
                    ))
                } else {
                    None
                }
            }
        };
        self.fused_ok.set(problem.is_none());
        if problem.is_some() {
            self.attention.set(AttentionPolicy::SPLIT);
        }
        problem
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

/// `len` values in [-1, 1] from a seed, for the start-up checks.
fn check_values(len: usize, seed: u32) -> Vec<f32> {
    let mut s = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
    (0..len)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s % 2001) as f32 / 1000.0 - 1.0
        })
        .collect()
}

/// `len` f16 values from a seed, for [`Gpu::check_f16`].  With
/// `subnormal`, each is a nonzero f16 subnormal.  Else each is f16-exact
/// and below 2 in magnitude: about one in eight a subnormal, one in 64 a
/// zero of either sign, the rest normal.
fn f16_check_weights(len: usize, seed: u32, subnormal: bool) -> Vec<u16> {
    let mut s = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
    (0..len)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            let sign = ((s >> 31) as u16) << 15;
            let mantissa = ((s >> 8) as u16 & 0x3ff).max(1);
            match (subnormal, s % 64) {
                (true, _) | (false, 1..=8) => sign | mantissa,
                (false, 0) => sign,
                (false, _) => sign | (1 + (s >> 18) as u16 % 15) << 10 | mantissa,
            }
        })
        .collect()
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
    /// An activation, or a [`Weight`] whose format picks the kernel's
    /// variant.
    pub b: (Operand<'a>, View),
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
        b: (impl Into<Operand<'a>>, View),
        c: (&'a wgpu::Buffer, View),
        m: usize,
        n: usize,
        k: usize,
    ) -> Self {
        Self {
            a,
            b: (b.0.into(), b.1),
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

    /// Whether a vec4 kernel can run this product: A and B both have K as
    /// the contiguous axis, K and every other offset and stride of A and B
    /// are multiples of 4 (so each vec4 is aligned and lies wholly inside
    /// or wholly outside the matrix), and both buffers hold a whole number
    /// of vec4s.  With the other rules each vec4 read ends on an element of
    /// the matrix, so none runs past the end of its buffer; the size rule is
    /// a cheap second guard (out of bounds, robust access would clamp the
    /// read to other data rather than return zero).
    pub fn vec4_ok(&self) -> bool {
        let (a, b) = (self.a.1, self.b.1);
        a.cs == 1
            && b.rs == 1
            && [self.k, a.off, a.rs, a.bo, a.bi, b.off, b.cs, b.bo, b.bi]
                .iter()
                .all(|x| x.is_multiple_of(4))
            && self.a.0.size().is_multiple_of(16)
            && self.b.0.buffer().size().is_multiple_of(16)
    }

    /// The entry of [`TILES`] that runs this product when `t` is asked
    /// for: `t`, or the fallback `TILES[0]` when `t` is a vec4 kernel and
    /// the product fails [`MatMul::vec4_ok`].
    pub fn kernel(&self, t: usize) -> usize {
        if TILES[t].vec4 && !self.vec4_ok() {
            0
        } else {
            t
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
    /// An entry of [`TILES`], and the format of its B.
    MatMul(usize, WeightFormat),
    Attention,
    Softmax,
    RmsNorm,
    Rope,
    Gate,
    Transpose,
    Glu,
}

/// The profile label of a matmul: its shape, batch and kernel, and `f16`
/// for an f16 variant (f32 labels stay as they were).
fn matmul_label([m, n, k]: [usize; 3], batch: usize, tile: Tile, format: WeightFormat) -> String {
    let variant = match format {
        WeightFormat::F32 => "",
        WeightFormat::F16 => " f16",
    };
    format!("matmul {m}x{n}x{k} x{batch} {}{variant}", tile.name)
}

/// Dispatches recorded for one submission.  Holds the borrows of the
/// buffers it binds (`'g`) until [`Rec::submit`], so a [`GpuBuffer`]
/// cannot be dropped, and destroyed, while a dispatch not yet submitted
/// still binds it.
///
/// ```no_run
/// fn step(gpu: &roformer_gpu::Gpu) {
///     let x = gpu.buffer("x", 256);
///     let mut r = gpu.rec();
///     r.softmax(&x, 4, 64);
///     r.submit();
///     drop(x);
/// }
/// ```
///
/// ```compile_fail,E0505
/// fn step(gpu: &roformer_gpu::Gpu) {
///     let x = gpu.buffer("x", 256);
///     let mut r = gpu.rec();
///     r.softmax(&x, 4, 64);
///     drop(x);
///     r.submit();
/// }
/// ```
pub struct Rec<'g> {
    gpu: &'g Gpu,
    params: Vec<u8>,
    cmds: Vec<(Kernel, wgpu::BindGroup, [u32; 3])>,
    /// One label per dispatch, kept only when the device is profiling.
    labels: Vec<String>,
}

impl<'g> Rec<'g> {
    fn push(&mut self, kernel: Kernel, bufs: &[&'g wgpu::Buffer], params: &[u8], groups: [u32; 3]) {
        if self.gpu.prof.is_some() {
            let label = match &kernel {
                Kernel::MatMul(t, _) => format!("matmul {}", TILES[*t].name),
                Kernel::Attention => "attention".into(),
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
            Kernel::MatMul(..) => &k.matmul.1,
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
            Kernel::MatMul(t, format) => k.matmul_pipeline(&gpu.device, *t, *format),
            Kernel::Attention => k.attention_pipeline(&gpu.device),
            Kernel::Softmax => &k.softmax.0,
            Kernel::RmsNorm => &k.rmsnorm.0,
            Kernel::Rope => &k.rope,
            Kernel::Gate => &k.gate,
            Kernel::Transpose => &k.transpose,
            Kernel::Glu => &k.glu,
        };
        if let Some(prof) = gpu.prof.as_ref().filter(|_| gpu.recording.get()) {
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
            let staging = gpu.create(&wgpu::BufferDescriptor {
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

    pub fn matmul(&mut self, mm: &MatMul<'g>) {
        let (a, b, c) = (mm.a.1, mm.b.1, mm.c.1);
        let (bias_buf, bias_off, bias_bo) = match mm.bias {
            Some((buf, off, bo)) => (buf, off, bo),
            None => (&*self.gpu.dummy, 0, 0),
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
        let t = mm.kernel(mm.tile.unwrap_or_else(|| {
            let t = self.gpu.policy.get().pick(mm.m, mm.n);
            if self.gpu.usable.get()[t] { t } else { 0 }
        }));
        let tile = TILES[t];
        let format = mm.b.0.format();
        let groups = [
            u(mm.n.div_ceil(tile.bn)),
            u(mm.m.div_ceil(tile.bm)),
            u(mm.outer * mm.inner),
        ];
        self.push(
            Kernel::MatMul(t, format),
            &[mm.a.0, mm.b.0.buffer(), mm.c.0, bias_buf],
            bytemuck::bytes_of(&p),
            groups,
        );
        if let Some(label) = self.labels.last_mut() {
            *label = matmul_label([mm.m, mm.n, mm.k], mm.outer * mm.inner, tile, format);
        }
    }

    /// In-place softmax over `rows` contiguous rows of `len`.
    pub fn softmax(&mut self, x: &'g wgpu::Buffer, rows: usize, len: usize) {
        let (gx, gy) = grid(rows);
        let p = [u(rows), u(len), gx, 0];
        self.push(Kernel::Softmax, &[x], bytemuck::cast_slice(&p), [gx, gy, 1]);
    }

    /// RMSNorm of `rows` rows of 256: `y[y_off..] = x[x_off..] / |x| * g`.
    pub fn rmsnorm(
        &mut self,
        x: &'g wgpu::Buffer,
        x_off: usize,
        g: &'g wgpu::Buffer,
        y: &'g wgpu::Buffer,
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
        bufs: [&'g wgpu::Buffer; 3],
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
    pub fn rope(
        &mut self,
        qkv: &'g wgpu::Buffer,
        table: &'g wgpu::Buffer,
        tokens: usize,
        seq: usize,
    ) {
        let dummy = &self.gpu.dummy;
        self.elementwise(Kernel::Rope, [dummy, table, qkv], tokens * 512, [seq, 0, 0]);
    }

    /// Fused attention for `nseq` sequences of `seq` tokens:
    /// `o[t, h*64 + d] = sigmoid(gates[t, h]) * sum_j softmax_j(alpha *
    /// q_t . k_j) v[j, h*64 + d]` per head, with q, k and v in a
    /// `[tokens, 1536]` qkv buffer at column offsets 0, 512 and 1024, `o`
    /// `[tokens, 512]` and `gates` `[tokens, 8]` (no gate when `None`).
    /// The same result as the scores matmul, softmax, p.v matmul and gate
    /// in [`net::Job`], in one dispatch.
    pub fn attention(
        &mut self,
        qkv: &'g wgpu::Buffer,
        gates: Option<&'g wgpu::Buffer>,
        o: &'g wgpu::Buffer,
        seq: usize,
        nseq: usize,
        alpha: f32,
    ) {
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
        let groups = [u(seq.div_ceil(64)), 8, u(nseq)];
        let g = gates.unwrap_or(&self.gpu.dummy);
        self.push(
            Kernel::Attention,
            &[qkv, g, o],
            bytemuck::bytes_of(&p),
            groups,
        );
    }

    /// `o[t, h*64 + d] *= sigmoid(gates[t, h])` for `tokens` rows.
    pub fn gate(&mut self, o: &'g wgpu::Buffer, gates: &'g wgpu::Buffer, tokens: usize) {
        let dummy = &self.gpu.dummy;
        self.elementwise(Kernel::Gate, [dummy, gates, o], tokens * 512, [0, 0, 0]);
    }

    /// `[a, b, 256]` in `src` to `[b, a, 256]` in `dst`.
    pub fn transpose(&mut self, src: &'g wgpu::Buffer, dst: &'g wgpu::Buffer, a: usize, b: usize) {
        let dummy = &self.gpu.dummy;
        self.elementwise(Kernel::Transpose, [src, dummy, dst], a * b * 256, [a, b, 0]);
    }

    /// GLU of `y` (`[n, frames, 2w]`) into `mask` (`[frames, 4100]`) at
    /// feature offset `off`: `mask[t, off + j*w + c] = y[j,t,c] * sigmoid(y[j,t,w+c])`.
    pub fn glu(
        &mut self,
        y: &'g wgpu::Buffer,
        mask: &'g wgpu::Buffer,
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

/// The WGSL of every kernel that calibration chooses between: each matmul
/// kernel, its f16 variant, and fused attention.
fn kernel_sources() -> Vec<String> {
    let f32_kernels = TILES.iter().map(Tile::source);
    let f16_variants = TILES.iter().map(Tile::f16_source);
    f32_kernels
        .chain(f16_variants)
        .chain(std::iter::once(attention_source()))
        .collect()
}

/// Identifies the compiled kernels that calibration chooses between
/// (FNV-1a over [`kernel_sources`]), so a per-GPU choice is redone
/// whenever they change.
pub fn kernels_id() -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for src in kernel_sources() {
        for b in src.bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    h
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::*;

    #[test]
    fn mem1_lint_is_on() {
        #[expect(
            clippy::disallowed_methods,
            reason = "MEM-1 canary: clippy fails here if this path is off"
        )]
        let copy = [1.0_f32].to_vec();
        assert_eq!(copy, [1.0]);
    }

    /// Clippy's check of this code, not the run, is the evidence: `raw` is
    /// compiled and linted, and never called, so no GPU is needed.
    #[test]
    fn mem3_lint_is_on() {
        fn raw(
            device: &wgpu::Device,
            queue: &wgpu::Queue,
            read: &wgpu::Buffer,
        ) -> [wgpu::Buffer; 2] {
            let desc = wgpu::BufferDescriptor {
                label: None,
                size: 16,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            };
            #[expect(
                clippy::disallowed_methods,
                reason = "MEM-3 canary: clippy fails here if this path is off"
            )]
            let plain = device.create_buffer(&desc);
            #[expect(
                clippy::disallowed_methods,
                reason = "MEM-3 canary: clippy fails here if this path is off"
            )]
            let _belt = wgpu::util::StagingBelt::new(device.clone(), 1024);
            #[expect(
                clippy::disallowed_methods,
                reason = "MEM-3 canary: clippy fails here if this path is off"
            )]
            let _belt_with_usages = wgpu::util::StagingBelt::new_with_buffer_usages(
                device.clone(),
                1024,
                wgpu::BufferUsages::MAP_WRITE | wgpu::BufferUsages::COPY_SRC,
            );
            #[expect(
                clippy::disallowed_methods,
                reason = "MEM-3 canary: clippy fails here if this path is off"
            )]
            wgpu::util::DownloadBuffer::read_buffer(device, queue, &read.slice(..), |_| {});
            #[expect(
                clippy::disallowed_methods,
                reason = "MEM-3 canary: clippy fails here if this path is off"
            )]
            let init = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: &[0; 16],
                usage: wgpu::BufferUsages::STORAGE,
            });
            [plain, init]
        }
        let _linted: fn(&wgpu::Device, &wgpu::Queue, &wgpu::Buffer) -> [wgpu::Buffer; 2] = raw;
    }

    /// Worst number of distinct addresses one bank serves in a single
    /// 32-lane access to workgroup memory, in RDNA's model of the LDS (32
    /// banks of 4 bytes; lanes reading the same address share one read).
    /// 1 means no bank conflict.  A model of the Radeon only: other GPUs
    /// bank their shared memory in their own ways, but neighbouring
    /// threads on neighbouring 4-byte words is conflict-free on any of
    /// them.
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
        let (sa, sb, bk, bm, bn) = (t.bm + t.pad, t.bn + t.pad, t.bk, t.bm, t.bn);
        let (tx, ty) = (t.bn / t.tn, t.bm / t.tm);
        [
            worst(1, |e, _| (e % bk) * sa + e / bk),
            worst(1, |e, _| (e / bm) * sa + e % bm),
            worst(1, |e, _| (e % bk) * sb + e / bk),
            worst(1, |e, _| (e / bn) * sb + e % bn),
            worst(t.tm, |id, q| id / tx + ty * q),
            worst(t.tn, |id, q| id % tx + tx * q),
        ]
    }

    /// Bank conflict degrees of a vec4 kernel, in vec4 slots: its stores
    /// of the A and B tiles (one layout: K contiguous), then the inner
    /// loop's reads of A and B.  A 16-byte access is served 8 lanes at a
    /// time (128 bytes, one word from each bank), so each group of 8 lanes
    /// is one access of 4 words per lane.
    fn conflicts_vec4(t: &Tile) -> [usize; 4] {
        let (kv, sv) = (t.bk / 4, (t.bk + t.pad) / 4);
        let (tx, ty) = (t.bn / t.tn, t.bm / t.tm);
        let worst8 = |n: usize, slot: &dyn Fn(usize, usize) -> usize| {
            let mut w = 1;
            for group in 0..32 {
                for q in 0..n {
                    let words = (8 * group..8 * group + 8)
                        .flat_map(|id| (0..4).map(move |c| (id, c)))
                        .map(|(id, c)| 4 * slot(id, q) + c);
                    w = w.max(degree(words));
                }
            }
            w
        };
        let store = |e: usize, _: usize| (e / kv) * sv + e % kv;
        [
            worst8(1, &store),
            worst8(1, &store),
            worst8(t.tm, &|id, q| (id / tx + ty * q) * sv),
            worst8(t.tn, &|id, q| (id % tx + tx * q) * sv),
        ]
    }

    /// The generated kernels are conflict-free in both load layouts and
    /// the inner loop; `s64`'s stores conflict 2-way when K is contiguous
    /// (rows padded by 1 float rather than 32 / bk).  The same model
    /// reproduced the Radeon ranking of the earlier blocked kernels (B
    /// reads 2-way at a thread stride of 4, 4-way at 8).  `v64` reads
    /// conflict-free; its stores, once per K tile against 32 reads, conflict
    /// 2-way, the price of the padding that keeps the reads free (unpadded,
    /// its B reads conflict 4-way).
    #[test]
    fn bank_conflicts() {
        for t in TILES.iter().filter(|t| !t.vec4) {
            let want = if t.hand_written {
                [2, 1, 2, 1, 1, 1]
            } else {
                [1; 6]
            };
            assert_eq!(conflicts(t), want, "{}", t.name);
        }
        for t in TILES.iter().filter(|t| t.vec4) {
            assert_eq!(conflicts_vec4(t), [2, 2, 1, 1], "{}", t.name);
            let unpadded = Tile { pad: 0, ..*t };
            assert_eq!(conflicts_vec4(&unpadded), [1, 1, 1, 4], "{}", t.name);
        }
    }

    /// Bytes of workgroup memory a WGSL source declares
    /// (`var<workgroup> name: array<f32, N>;` or `array<vec4<f32>, N>`
    /// lines).
    fn workgroup_bytes(src: &str) -> usize {
        src.lines()
            .filter(|l| l.starts_with("var<workgroup>"))
            .map(|l| {
                let (size, n) = match l.split_once("array<vec4<f32>, ") {
                    Some((_, n)) => (16, n),
                    None => (4, l.split("array<f32, ").nth(1).expect("f32 array")),
                };
                size * n[..n.find('>').expect(">")]
                    .parse::<usize>()
                    .expect("length")
            })
            .sum()
    }

    /// Every generated kernel fits WebGPU's default 16 KiB of workgroup
    /// memory.
    #[test]
    fn workgroup_memory_within_defaults() {
        for t in &TILES {
            let bytes = workgroup_bytes(&t.source());
            assert_eq!(workgroup_bytes(&t.f16_source()), bytes, "{} f16", t.name);
            let want = if t.vec4 {
                4 * (t.bm + t.bn) * (t.bk + t.pad)
            } else {
                4 * t.bk * (t.bm + t.bn + 2 * t.pad)
            };
            assert_eq!(bytes, want, "{}", t.name);
            assert!(bytes <= 16 * 1024, "{}: {bytes} bytes", t.name);
        }
        let attention = workgroup_bytes(&attention_source());
        assert!((12 * 1024..=16 * 1024).contains(&attention), "{attention}");
    }

    /// Every template placeholder in `src`: an upper-case name in braces,
    /// such as `{BM}`.  WGSL's own braces never hold such a name.
    fn unfilled_placeholders(src: &str) -> Vec<&str> {
        src.match_indices('{')
            .filter_map(|(open, _)| {
                let close = src[open + 1..].find('}')?;
                let name = &src[open + 1..open + 1 + close];
                let is_placeholder = !name.is_empty()
                    && name
                        .bytes()
                        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_');
                is_placeholder.then(|| &src[open..open + close + 2])
            })
            .collect()
    }

    /// The scan finds names with `_` and digits, and skips WGSL blocks.
    #[test]
    fn placeholders_are_found_and_wgsl_braces_are_not() {
        let src = "fn f() { return; }\nvar<workgroup> a: array<f32, {SA_LEN}>;\n{}{SOFTMAX_1}{x}";
        assert_eq!(unfilled_placeholders(src), ["{SA_LEN}", "{SOFTMAX_1}"]);
        let template = unfilled_placeholders(include_str!("shaders/matmul_interleaved.wgsl"));
        assert!(template.contains(&"{BM}"), "{template:?}");
    }

    /// Every generated source, by name: each entry of [`TILES`] (`s64` is
    /// `matmul.wgsl`), the f16 variant of each (`i64-f16` and so on), and
    /// fused attention.
    fn generated_sources() -> impl Iterator<Item = (String, String)> {
        let f32_kernels = TILES.iter().map(|t| (t.name.to_string(), t.source()));
        let f16_variants = TILES
            .iter()
            .map(|t| (format!("{}-f16", t.name), t.f16_source()));
        f32_kernels.chain(f16_variants).chain(std::iter::once((
            "attention".to_string(),
            attention_source(),
        )))
    }

    /// The generated sources have every placeholder filled.
    #[test]
    fn sources_are_complete() {
        for (name, src) in generated_sources() {
            let unfilled = unfilled_placeholders(&src);
            assert!(unfilled.is_empty(), "{name}: {unfilled:?}");
        }
    }

    const LIMITS_DOC: &str = "see docs/references/webgpu-limits.md";

    /// What [`limit_breaches`] lets a shader use: naga's default, and
    /// `SHADER_FLOAT16_IN_FLOAT32` for `unpack2x16float`, which the f16
    /// variants call (decision 0018).  The device has it when the adapter
    /// has `DownlevelFlags::SHADER_F16_IN_F32`; without the flag the f16
    /// variants never compile ([`Gpu::has_f16_unpack`]).  It needs no
    /// feature and no limit beyond WebGPU's defaults.
    const CHECK_CAPABILITIES: naga::valid::Capabilities =
        naga::valid::Capabilities::MULTISAMPLED_SHADING
            .union(naga::valid::Capabilities::CUBE_ARRAY_TEXTURES)
            .union(naga::valid::Capabilities::SHADER_FLOAT16_IN_FLOAT32);

    /// The GPU-1 breaches in one shader, as messages that name the limit
    /// and the fix.  Per entry point: each workgroup size axis, the
    /// invocations per workgroup, and the workgroup storage.  Workgroup
    /// storage is counted as WebGPU counts it: each `var<workgroup>` that
    /// the entry point statically uses, rounded up to 16 bytes.  Bind
    /// groups, and storage and uniform buffers, are counted over the whole
    /// module, because `Kernels::new` gives a module's pipelines one
    /// layout with every binding the module declares
    /// (`every_layout_matches_its_shaders`).
    fn limit_breaches(name: &str, src: &str, limits: &wgpu::Limits) -> Vec<String> {
        let module = match naga::front::wgsl::parse_str(src) {
            Ok(module) => module,
            Err(e) => {
                let error = e.emit_to_string_with_path(src, name);
                return vec![format!(
                    "GPU-1: {name} does not parse, so its limits are not checked:\n{error}Fix: correct the WGSL."
                )];
            }
        };
        if !module.overrides.is_empty() {
            return vec![format!(
                "GPU-1: {name} declares an `override`, so its sizes are not known before pipeline creation.  Fix: use `const`, so this test can check them."
            )];
        }
        let info = match naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            CHECK_CAPABILITIES,
        )
        .validate(&module)
        {
            Ok(info) => info,
            Err(e) => {
                let error = e.emit_to_string_with_path(src, name);
                return vec![format!(
                    "GPU-1: {name} is not valid WGSL, so its limits are not checked:\n{error}Fix: correct the WGSL."
                )];
            }
        };
        let mut layouter = naga::proc::Layouter::default();
        layouter
            .update(module.to_ctx())
            .expect("a validated module has a layout");

        let mut breaches = Vec::new();
        let buffers = [
            (
                "storage",
                "maxStorageBuffersPerShaderStage",
                limits.max_storage_buffers_per_shader_stage,
            ),
            (
                "uniform",
                "maxUniformBuffersPerShaderStage",
                limits.max_uniform_buffers_per_shader_stage,
            ),
        ];
        for (kind, limit, max) in buffers {
            let count = module
                .global_variables
                .iter()
                .filter(|(_, g)| match g.space {
                    naga::AddressSpace::Storage { .. } => kind == "storage",
                    naga::AddressSpace::Uniform => kind == "uniform",
                    _ => false,
                })
                .count();
            if count > max as usize {
                breaches.push(format!(
                    "GPU-1: {name} declares {count} {kind} buffers, over {limit} ({max}).  Fix: bind fewer buffers (pack them into one, or split the kernel); {LIMITS_DOC}."
                ));
            }
        }

        let max = limits.max_bind_groups;
        let groups: std::collections::BTreeSet<u32> = module
            .global_variables
            .iter()
            .filter_map(|(_, g)| g.binding.as_ref().map(|b| b.group))
            .collect();
        if let Some(&group) = groups.iter().find(|&&g| g >= max) {
            breaches.push(format!(
                "GPU-1: {name} uses @group({group}), over maxBindGroups ({max}).  Fix: use a group number below {max}; {LIMITS_DOC}."
            ));
        }

        let compute = module
            .entry_points
            .iter()
            .enumerate()
            .filter(|(_, ep)| ep.stage == naga::ShaderStage::Compute);
        for (index, ep) in compute {
            let entry = format!("{name} `{}`", ep.name);
            let axes = [
                (
                    "x",
                    "maxComputeWorkgroupSizeX",
                    limits.max_compute_workgroup_size_x,
                ),
                (
                    "y",
                    "maxComputeWorkgroupSizeY",
                    limits.max_compute_workgroup_size_y,
                ),
                (
                    "z",
                    "maxComputeWorkgroupSizeZ",
                    limits.max_compute_workgroup_size_z,
                ),
            ];
            for ((axis, limit, max), size) in axes.into_iter().zip(ep.workgroup_size) {
                if size > max {
                    breaches.push(format!(
                        "GPU-1: {entry}: workgroup size {axis} is {size}, over {limit} ({max}).  Fix: lower @workgroup_size, or move the work to another axis or a loop; {LIMITS_DOC}."
                    ));
                }
            }

            let invocations: u64 = ep.workgroup_size.iter().map(|&s| u64::from(s)).product();
            let max = limits.max_compute_invocations_per_workgroup;
            if invocations > u64::from(max) {
                let [x, y, z] = ep.workgroup_size;
                breaches.push(format!(
                    "GPU-1: {entry}: {x} * {y} * {z} = {invocations} invocations per workgroup, over maxComputeInvocationsPerWorkgroup ({max}).  Fix: lower @workgroup_size so that x * y * z is at most {max}; {LIMITS_DOC}."
                ));
            }

            let uses = info.get_entry_point(index);
            let bytes: u64 = module
                .global_variables
                .iter()
                .filter(|(handle, g)| {
                    g.space == naga::AddressSpace::WorkGroup && !uses[*handle].is_empty()
                })
                .map(|(_, g)| u64::from(layouter[g.ty].size.next_multiple_of(16)))
                .sum();
            let max = limits.max_compute_workgroup_storage_size;
            if bytes > u64::from(max) {
                breaches.push(format!(
                    "GPU-1: {entry}: {bytes} bytes of workgroup storage, over maxComputeWorkgroupStorageSize ({max}).  Fix: shrink the var<workgroup> arrays (smaller tiles or less padding); {LIMITS_DOC}."
                ));
            }
        }
        breaches
    }

    /// The device is opened with these limits, and
    /// `references/webgpu-limits.md` gives the same numbers.
    #[test]
    fn default_limits_match_the_reference() {
        let l = wgpu::Limits::defaults();
        let checked = [
            l.max_compute_workgroup_size_x,
            l.max_compute_workgroup_size_y,
            l.max_compute_workgroup_size_z,
            l.max_compute_invocations_per_workgroup,
            l.max_compute_workgroup_storage_size,
            l.max_storage_buffers_per_shader_stage,
            l.max_uniform_buffers_per_shader_stage,
            l.max_bind_groups,
        ];
        assert_eq!(checked, [256, 256, 64, 256, 16_384, 8, 12, 4]);
    }

    /// The shader files that are compiled as they are.
    const SHADER_FILES: [(&str, &str); 3] = [
        ("softmax.wgsl", include_str!("shaders/softmax.wgsl")),
        ("rmsnorm.wgsl", include_str!("shaders/rmsnorm.wgsl")),
        ("elementwise.wgsl", include_str!("shaders/elementwise.wgsl")),
    ];

    /// The shader files that [`generated_sources`] covers: three
    /// templates, and `matmul.wgsl`, which is `s64`'s source.
    const GENERATED_FROM: [&str; 4] = [
        "matmul_interleaved.wgsl",
        "matmul_vec4.wgsl",
        "attention_tiled.wgsl",
        "matmul.wgsl",
    ];

    /// Every file in `src/shaders/` is checked, as it is or through a
    /// generated source, so a new shader cannot be left out.
    #[test]
    fn every_shader_file_is_checked() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src/shaders");
        let mut on_disk: Vec<String> = std::fs::read_dir(dir)
            .expect("shaders folder")
            .map(|e| e.expect("entry").file_name().into_string().expect("UTF-8"))
            .filter(|f| f.ends_with(".wgsl"))
            .collect();
        on_disk.sort();
        let mut checked: Vec<String> = SHADER_FILES
            .iter()
            .map(|(name, _)| *name)
            .chain(GENERATED_FROM)
            .map(String::from)
            .collect();
        checked.sort();
        assert_eq!(on_disk, checked);
        let s64 = TILES.iter().find(|t| t.hand_written).expect("s64");
        assert_eq!(s64.source(), include_str!("shaders/matmul.wgsl"));
    }

    /// Every shader the GPU compiles fits WebGPU's default limits (GPU-1):
    /// the generated sources, and [`SHADER_FILES`].  The names are pinned,
    /// so a source cannot drop out of the check unseen.
    #[test]
    fn every_kernel_fits_the_default_limits() {
        let all: Vec<_> = generated_sources()
            .chain(SHADER_FILES.map(|(name, src)| (name.to_string(), src.to_string())))
            .collect();
        let names: Vec<&str> = all.iter().map(|(name, _)| name.as_str()).collect();
        let want = [
            "i64",
            "i128x64",
            "s64",
            "v64",
            "i64-f16",
            "i128x64-f16",
            "s64-f16",
            "v64-f16",
            "attention",
            "softmax.wgsl",
            "rmsnorm.wgsl",
            "elementwise.wgsl",
        ];
        assert_eq!(names, want);
        let limits = wgpu::Limits::defaults();
        let breaches: Vec<String> = all
            .iter()
            .flat_map(|(name, src)| limit_breaches(name, src, &limits))
            .collect();
        assert!(breaches.is_empty(), "{}", breaches.join("\n"));
    }

    /// Planted breaches: each is reported once, with the limit's name and
    /// GPU-1.
    #[test]
    fn limit_breaches_are_reported() {
        let limits = wgpu::Limits::defaults();
        let planted = "
            var<workgroup> big: array<f32, 5000>;
            @compute @workgroup_size(1, 1, 128)
            fn main(@builtin(local_invocation_index) i: u32) { big[i] = 0.0; }
        ";
        let breaches = limit_breaches("planted", planted, &limits);
        assert_eq!(breaches.len(), 2, "{breaches:#?}");
        let mut all = breaches.clone();
        assert!(breaches[0].starts_with("GPU-1: planted `main`: workgroup size z is 128, over maxComputeWorkgroupSizeZ (64).  Fix: "), "{}", breaches[0]);
        assert!(breaches[1].starts_with("GPU-1: planted `main`: 20000 bytes of workgroup storage, over maxComputeWorkgroupStorageSize (16384).  Fix: "), "{}", breaches[1]);

        let wide = "
            @compute @workgroup_size(512, 2, 1)
            fn main() {}
        ";
        let breaches = limit_breaches("wide", wide, &limits);
        assert_eq!(breaches.len(), 2, "{breaches:#?}");
        all.extend(breaches.clone());
        assert!(breaches[0].contains("x is 512, over maxComputeWorkgroupSizeX (256)"));
        assert!(breaches[1].contains("512 * 2 * 1 = 1024 invocations per workgroup, over maxComputeInvocationsPerWorkgroup (256)"));

        let square = "
            @compute @workgroup_size(16, 32, 1)
            fn main() {}
        ";
        let breaches = limit_breaches("square", square, &limits);
        assert_eq!(breaches.len(), 1, "{breaches:#?}");
        all.extend(breaches.clone());
        assert!(breaches[0].contains("maxComputeInvocationsPerWorkgroup (256)"));
        assert_eq!(all.len(), 5);
        let end = format!("{LIMITS_DOC}.");
        assert!(
            all.iter()
                .all(|b| b.starts_with("GPU-1: ") && b.contains(".  Fix: ") && b.ends_with(&end)),
            "{all:#?}"
        );
    }

    #[test]
    fn a_fifth_bind_group_is_reported() {
        let limits = wgpu::Limits::defaults();
        let src = "@group(4) @binding(0) var<uniform> u: vec4<f32>;\n@compute @workgroup_size(1) fn main() { _ = u; }";
        let breaches = limit_breaches("groups", src, &limits);
        assert_eq!(breaches.len(), 1, "{breaches:#?}");
        assert!(
            breaches[0].starts_with(
                "GPU-1: groups uses @group(4), over maxBindGroups (4).  Fix: use a group number below 4"
            ),
            "{}",
            breaches[0]
        );
        let in_range = src.replace("@group(4)", "@group(3)");
        assert_eq!(
            limit_breaches("groups", &in_range, &limits),
            Vec::<String>::new()
        );
    }

    /// The bindings a module declares, in binding order, as [`Bind`]s; all
    /// must be buffers in group 0, numbered from 0 with no gap.
    fn declared_binds(src: &str) -> Vec<Bind> {
        let module = naga::front::wgsl::parse_str(src).expect("parses");
        let mut binds: Vec<(u32, Bind)> = module
            .global_variables
            .iter()
            .filter_map(|(_, g)| {
                let b = g.binding.as_ref()?;
                assert_eq!(b.group, 0, "{:?}", g.name);
                let bind = match g.space {
                    naga::AddressSpace::Storage { access }
                        if access.contains(naga::StorageAccess::STORE) =>
                    {
                        Bind::ReadWrite
                    }
                    naga::AddressSpace::Storage { .. } => Bind::Read,
                    naga::AddressSpace::Uniform => Bind::Uniform,
                    other => panic!("{:?} is in {other:?}, not a buffer", g.name),
                };
                Some((b.binding, bind))
            })
            .collect();
        binds.sort_by_key(|(binding, _)| *binding);
        let numbers: Vec<u32> = binds.iter().map(|(binding, _)| *binding).collect();
        assert_eq!(numbers, (0..binds.len() as u32).collect::<Vec<_>>());
        binds.into_iter().map(|(_, bind)| bind).collect()
    }

    /// Each bind list binds exactly what the shaders it is paired with here
    /// declare, so the module-wide buffer counts in [`limit_breaches`] are
    /// the counts of the lists.  The test pairs them as `Kernels::new`
    /// does, but does not read it: only wgpu's validation sees a pipeline
    /// given the wrong list.  The entry points that `Kernels::new` names
    /// exist.
    #[test]
    fn every_layout_matches_its_shaders() {
        let file = |src: &str| src.to_string();
        let mut shaders: Vec<(String, String, &[Bind], &[&str])> = TILES
            .iter()
            .flat_map(|t| {
                WeightFormat::ALL.map(|f| {
                    let name = format!("{} {}", t.name, f.name());
                    (name, t.source_for(f), MATMUL_BINDS, &["main"][..])
                })
            })
            .collect();
        shaders.extend([
            (
                "attention".to_string(),
                attention_source(),
                ELEMENTWISE_BINDS,
                &["main"][..],
            ),
            (
                "softmax".to_string(),
                file(include_str!("shaders/softmax.wgsl")),
                SOFTMAX_BINDS,
                &["main"][..],
            ),
            (
                "rmsnorm".to_string(),
                file(include_str!("shaders/rmsnorm.wgsl")),
                RMSNORM_BINDS,
                &["main"][..],
            ),
            (
                "elementwise".to_string(),
                file(include_str!("shaders/elementwise.wgsl")),
                ELEMENTWISE_BINDS,
                &["rope", "gate", "transpose3", "glu"][..],
            ),
        ]);
        for (name, src, layout, entries) in shaders {
            assert_eq!(declared_binds(&src), layout, "{name}");
            let module = naga::front::wgsl::parse_str(&src).expect("parses");
            let found: Vec<&str> = module
                .entry_points
                .iter()
                .map(|ep| ep.name.as_str())
                .collect();
            for entry in entries {
                assert!(found.contains(entry), "{name}: no `{entry}` in {found:?}");
            }
        }
    }

    /// WebGPU counts each workgroup variable rounded up to 16 bytes, and
    /// only the ones the entry point uses, also through a function it calls.
    #[test]
    fn workgroup_storage_is_counted_as_webgpu_counts_it() {
        let limits = wgpu::Limits::defaults();
        let shader = |used_by_main: &str| {
            format!(
                "
                var<workgroup> a: array<f32, 4093>; // 16,372 bytes, 16,384 rounded up
                var<workgroup> b: f32;              // 4 bytes, 16 rounded up
                var<workgroup> unused: array<f32, 4096>;
                fn touch_b() {{ b = 1.0; }}
                @compute @workgroup_size(64)
                fn main() {{ a[0] = 1.0; {used_by_main} }}
                "
            )
        };
        assert_eq!(
            limit_breaches("a", &shader(""), &limits),
            Vec::<String>::new()
        );
        let breaches = limit_breaches("a and b", &shader("touch_b();"), &limits);
        assert_eq!(breaches.len(), 1, "{breaches:#?}");
        assert!(
            breaches[0].contains("16400 bytes of workgroup storage"),
            "{}",
            breaches[0]
        );
    }

    /// A value equal to its limit passes: 64 on z, 256 on x and y, 256
    /// invocations, 16,384 bytes, 8 storage and 12 uniform buffers, group 3.
    #[test]
    fn values_at_the_limits_pass() {
        let limits = wgpu::Limits::defaults();
        let storage: String = (0..8)
            .map(|i| {
                format!("@group(0) @binding({i}) var<storage, read_write> s{i}: array<f32>;\n")
            })
            .collect();
        let uniform: String = (0..12)
            .map(|i| format!("@group(3) @binding({i}) var<uniform> u{i}: vec4<f32>;\n"))
            .collect();
        let src = format!(
            "{storage}{uniform}
            var<workgroup> w: array<f32, 4096>;
            @compute @workgroup_size(4, 1, 64) fn z() {{ w[0] = s0[0]; _ = u0; }}
            @compute @workgroup_size(256, 1, 1) fn x() {{ w[0] = 1.0; }}
            @compute @workgroup_size(1, 256, 1) fn y() {{ w[0] = 1.0; }}"
        );
        assert_eq!(
            limit_breaches("at limits", &src, &limits),
            Vec::<String>::new()
        );
        let over = src.replace("array<f32, 4096>", "array<f32, 4097>");
        assert_eq!(
            limit_breaches("over", &over, &limits).len(),
            3,
            "one per entry point"
        );
    }

    /// Each entry point is checked with its own workgroup size and the
    /// workgroup variables it uses, not only the first.
    #[test]
    fn every_entry_point_is_checked() {
        let limits = wgpu::Limits::defaults();
        let src = "
            var<workgroup> small: array<f32, 16>;
            var<workgroup> big: array<f32, 5000>;
            @compute @workgroup_size(64) fn first() { small[0] = 1.0; }
            @compute @workgroup_size(1, 1, 128) fn second() { big[0] = 1.0; }
        ";
        let breaches = limit_breaches("two", src, &limits);
        assert_eq!(breaches.len(), 2, "{breaches:#?}");
        assert!(
            breaches[0].starts_with("GPU-1: two `second`: workgroup size z is 128"),
            "{}",
            breaches[0]
        );
        assert!(
            breaches[1].starts_with("GPU-1: two `second`: 20000 bytes of workgroup storage"),
            "{}",
            breaches[1]
        );
    }

    #[test]
    fn too_many_buffers_are_reported() {
        let limits = wgpu::Limits::defaults();
        let storage: String = (0..9)
            .map(|i| {
                format!("@group(0) @binding({i}) var<storage, read_write> s{i}: array<f32>;\n")
            })
            .collect();
        let uniform: String = (0..13)
            .map(|i| format!("@group(1) @binding({i}) var<uniform> u{i}: vec4<f32>;\n"))
            .collect();
        let src = format!("{storage}{uniform}@compute @workgroup_size(1) fn main() {{}}");
        let breaches = limit_breaches("buffers", &src, &limits);
        assert_eq!(breaches.len(), 2, "{breaches:#?}");
        assert!(breaches[0].starts_with("GPU-1: buffers declares 9 storage buffers, over maxStorageBuffersPerShaderStage (8).  Fix: "));
        assert!(breaches[1].starts_with("GPU-1: buffers declares 13 uniform buffers, over maxUniformBuffersPerShaderStage (12).  Fix: "));
    }

    /// A shader the checker cannot size fails, so it never passes unchecked.
    #[test]
    fn unchecked_shaders_fail() {
        let limits = wgpu::Limits::defaults();
        let cases = [
            (
                "template",
                include_str!("shaders/matmul_interleaved.wgsl"),
                "does not parse",
            ),
            (
                "invalid",
                "@compute @workgroup_size(1) fn main(@builtin(vertex_index) i: u32) {}",
                "is not valid WGSL",
            ),
            (
                "override",
                "override n: u32 = 64; @compute @workgroup_size(n) fn main() {}",
                "declares an `override`",
            ),
        ];
        for (name, src, want) in cases {
            let breaches = limit_breaches(name, src, &limits);
            assert_eq!(breaches.len(), 1, "{name}: {breaches:#?}");
            assert!(
                breaches[0].starts_with(&format!("GPU-1: {name} {want}")),
                "{}",
                breaches[0]
            );
        }
    }

    /// The checker accepts `unpack2x16float`, which the f16 variants call,
    /// and nothing more than naga's default.
    #[test]
    fn check_capabilities_add_only_the_f16_unpack() {
        use naga::valid::{Capabilities, ValidationFlags, Validator};
        let planted = "
            @group(0) @binding(0) var<storage, read_write> o: array<f32>;
            @compute @workgroup_size(1) fn main() { o[0] = unpack2x16float(0x3c00u).x; }
        ";
        let module = naga::front::wgsl::parse_str(planted).expect("parses");
        let valid = |caps| {
            Validator::new(ValidationFlags::all(), caps)
                .validate(&module)
                .is_ok()
        };
        assert!(!valid(Capabilities::default()));
        assert!(valid(CHECK_CAPABILITIES));
        assert!(CHECK_CAPABILITIES.contains(Capabilities::default()));
        assert_eq!(
            CHECK_CAPABILITIES.difference(Capabilities::default()),
            Capabilities::SHADER_FLOAT16_IN_FLOAT32
        );
        let limits = wgpu::Limits::defaults();
        assert_eq!(
            limit_breaches("planted", planted, &limits),
            Vec::<String>::new()
        );
    }

    /// Each f16 variant is its f32 kernel with two lines changed: the
    /// declaration of B and the one read of B, which unpacks the f16
    /// values.  So the products and sums are the same, in the same order.
    #[test]
    fn f16_variants_change_only_the_reads_of_b() {
        for t in &TILES {
            let (f32_src, f16_src) = (t.source(), t.f16_source());
            let old: Vec<&str> = f32_src.lines().collect();
            let new: Vec<&str> = f16_src.lines().collect();
            let removed: Vec<&str> = old.iter().filter(|l| !new.contains(l)).copied().collect();
            let added: Vec<&str> = new
                .iter()
                .filter(|l| !old.contains(l) && !l.starts_with("//"))
                .copied()
                .collect();
            assert_eq!(removed.len(), 2, "{}: {removed:#?}", t.name);
            assert!(
                removed[0].starts_with("@group(0) @binding(1) var<storage, read> b: array<"),
                "{}: {removed:#?}",
                t.name
            );
            assert!(removed[1].contains("b["), "{}: {removed:#?}", t.name);
            assert!(
                added[0].starts_with("@group(0) @binding(1) var<storage, read> b: array<"),
                "{}: {added:#?}",
                t.name
            );
            assert!(
                added[1..]
                    .iter()
                    .all(|l| !l.contains("acc") && !l.contains("fma")),
                "{}: {added:#?}",
                t.name
            );
            assert_eq!(
                new.len(),
                old.len() + added.len() - removed.len() + 2,
                "{}",
                t.name
            );
            let unpacks = if t.vec4 { 2 } else { 1 };
            assert_eq!(
                f16_src.matches("unpack2x16float(").count(),
                unpacks,
                "{}",
                t.name
            );
            assert_eq!(f32_src.matches("unpack2x16float(").count(), 0, "{}", t.name);
        }
    }

    /// A substitution that does not find its text exactly once fails, so a
    /// change to a template cannot leave an f16 variant reading f32.
    #[test]
    #[should_panic(expected = "occurs 2 times")]
    fn a_substitution_must_match_once() {
        replace_once("b b", "b", "c");
    }

    /// Calibration's key covers every kernel it can choose, f16 variants
    /// included.
    #[test]
    fn kernels_id_covers_the_f16_variants() {
        let sources = kernel_sources();
        for t in &TILES {
            for f in WeightFormat::ALL {
                assert!(
                    sources.contains(&t.source_for(f)),
                    "{} {}",
                    t.name,
                    f.name()
                );
            }
        }
        assert!(sources.contains(&attention_source()));
    }

    fn words(bytes: &[u8]) -> Vec<u32> {
        bytes
            .chunks_exact(4)
            .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
            .collect()
    }

    /// `upload_f16` puts element 2 j in the low half of word j, pads an odd
    /// count with one zero value, and rounds up to whole 16-byte groups.  A
    /// whole number of groups goes up without a copy.
    #[test]
    fn f16_packing_puts_element_2j_in_the_low_half() {
        let three = f16_bytes(&[0x1111, 0x2222, 0x3333]);
        assert_eq!(words(&three), [0x2222_1111, 0x0000_3333, 0, 0]);
        let eight: Vec<u16> = (1..=8).collect();
        let packed = f16_bytes(&eight);
        assert!(matches!(packed, Cow::Borrowed(_)));
        assert_eq!(
            words(&packed),
            [0x0002_0001, 0x0004_0003, 0x0006_0005, 0x0008_0007]
        );
        let ten: Vec<u16> = (1..=10).collect();
        assert_eq!(
            words(&f16_bytes(&ten)),
            [
                0x0002_0001,
                0x0004_0003,
                0x0006_0005,
                0x0008_0007,
                0x000a_0009,
                0,
                0,
                0
            ]
        );
        assert_eq!(words(&f16_bytes(&[])), [0; 4]);
    }

    /// A profile label names the format of an f16 variant only.
    #[test]
    fn matmul_labels_name_the_f16_variant() {
        let label = |format| matmul_label([1151, 1536, 256], 6, TILES[0], format);
        assert_eq!(label(WeightFormat::F32), "matmul 1151x1536x256 x6 i64");
        assert_eq!(label(WeightFormat::F16), "matmul 1151x1536x256 x6 i64 f16");
    }

    /// Values compare by their bits, except that `+0` equals `-0`: WGSL
    /// lets a GPU ignore the sign of a zero.
    #[test]
    fn bit_differences_ignore_only_the_sign_of_zero() {
        let got = [0.0, -0.0, 1.0, 2.0, f32::NAN, 1e-7, 5.0];
        let want = [-0.0, 0.0, 1.0, 2.5, 3.0, 0.0, 5.0];
        let d = bit_differences(&got, &want);
        assert_eq!(d.count, 3);
        assert_eq!(d.largest, f32::INFINITY);
        let d = bit_differences(&got[..4], &want[..4]);
        assert_eq!((d.count, d.largest), (1, 0.5));
        let d = bit_differences(&[1e-7], &[0.0]);
        assert_eq!((d.count, d.largest), (1, 1e-7));
        assert_eq!(bit_differences(&[f32::NAN], &[f32::NAN]).count, 0);
    }

    #[test]
    fn attention_policy_names() {
        for p in [
            AttentionPolicy::SPLIT,
            AttentionPolicy::FUSED,
            AttentionPolicy {
                time: false,
                freq: true,
            },
        ] {
            assert_eq!(AttentionPolicy::parse(p.name()), Some(p));
        }
        assert_eq!(AttentionPolicy::parse("a4"), None);
    }
}
