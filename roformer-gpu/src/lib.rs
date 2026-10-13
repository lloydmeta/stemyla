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

use std::num::NonZeroU64;
use std::sync::{Arc, Mutex};

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

/// Bytes reserved for one dispatch's parameters (WebGPU's default
/// `minUniformBufferOffsetAlignment`).
const PARAM_STRIDE: u64 = 256;
/// Dispatches one [`Rec`] may hold.
const MAX_DISPATCHES: u64 = 64;

/// A matmul kernel's tile shape: a workgroup of 256 threads computes a
/// `bm` x `bn` block of the output, each thread `tm` x `tn` of it, stepping
/// through K `bk` at a time.  The first entry of [`TILES`] is the original
/// hand-written kernel (`matmul.wgsl`); the others are `matmul_tiled.wgsl`
/// with these numbers filled in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tile {
    pub name: &'static str,
    pub bm: usize,
    pub bn: usize,
    pub bk: usize,
    pub tm: usize,
    pub tn: usize,
}

impl Tile {
    fn source(&self) -> String {
        if self.name == TILES[0].name {
            return include_str!("shaders/matmul.wgsl").to_string();
        }
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
pub const TILES: [Tile; 5] = [
    Tile {
        name: "s64",
        bm: 64,
        bn: 64,
        bk: 16,
        tm: 4,
        tn: 4,
    },
    Tile {
        name: "v64",
        bm: 64,
        bn: 64,
        bk: 16,
        tm: 4,
        tn: 4,
    },
    Tile {
        name: "v128",
        bm: 128,
        bn: 128,
        bk: 8,
        tm: 8,
        tn: 8,
    },
    Tile {
        name: "v128x64",
        bm: 128,
        bn: 64,
        bk: 8,
        tm: 8,
        tn: 4,
    },
    Tile {
        name: "v64x128",
        bm: 64,
        bn: 128,
        bk: 8,
        tm: 4,
        tn: 8,
    },
];

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
    kernels: Kernels,
    /// Bound where a kernel has an input it does not use for this dispatch.
    dummy: wgpu::Buffer,
    uniforms: wgpu::Buffer,
    errors: Arc<Mutex<Vec<String>>>,
}

struct Kernels {
    /// One pipeline per entry of [`TILES`].
    matmul: (Vec<wgpu::ComputePipeline>, wgpu::BindGroupLayout),
    softmax: (wgpu::ComputePipeline, wgpu::BindGroupLayout),
    rmsnorm: (wgpu::ComputePipeline, wgpu::BindGroupLayout),
    rope: wgpu::ComputePipeline,
    gate: wgpu::ComputePipeline,
    transpose: wgpu::ComputePipeline,
    glu: wgpu::ComputePipeline,
    elementwise: wgpu::BindGroupLayout,
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
        let matmul = TILES
            .iter()
            .map(|t| pipeline(device, &shader(device, t.name, &t.source()), "main", &mm))
            .collect();
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
        }
    }
}

impl Gpu {
    /// Opens a GPU with WebGPU's default limits.  `WGPU_BACKEND` and
    /// `WGPU_ADAPTER_NAME` select an adapter natively (tests use
    /// SwiftShader's Vulkan driver this way).
    pub async fn new(power: wgpu::PowerPreference) -> Result<Self, String> {
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
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("roformer"),
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
        Self {
            device,
            queue,
            info,
            policy: std::cell::Cell::new(MatMulPolicy::Fixed(0)),
            usable: std::cell::Cell::new([true; TILES.len()]),
            kernels,
            dummy,
            uniforms,
            errors,
        }
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

    pub fn rec(&self) -> Rec<'_> {
        Rec {
            gpu: self,
            params: Vec::new(),
            cmds: Vec::new(),
        }
    }
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
}

impl Rec<'_> {
    fn push(&mut self, kernel: Kernel, bufs: &[&wgpu::Buffer], params: &[u8], groups: [u32; 3]) {
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
        {
            let mut pass = enc.begin_compute_pass(&Default::default());
            let k = &gpu.kernels;
            for (kernel, bg, [x, y, z]) in &self.cmds {
                let p = match kernel {
                    Kernel::MatMul(t) => &k.matmul.0[*t],
                    Kernel::Softmax => &k.softmax.0,
                    Kernel::RmsNorm => &k.rmsnorm.0,
                    Kernel::Rope => &k.rope,
                    Kernel::Gate => &k.gate,
                    Kernel::Transpose => &k.transpose,
                    Kernel::Glu => &k.glu,
                };
                pass.set_pipeline(p);
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
