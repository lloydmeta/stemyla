//! The BS-RoFormer network on a Burn backend.
//!
//! Layouts (no batch dimension; one chunk at a time):
//! * input features `[T, FEATURES]`, per frame ordered (bin, channel, re/im)
//! * hidden state `[BANDS, T, DIM]` (band-major), transposed to
//!   `[T, BANDS, DIM]` for the frequency transformers
//! * output masks `[STEMS, T, FEATURES]`, same feature order as the input
//!
//! Bands of equal width are processed together as batched matmuls
//! ([`crate::band_groups`]).  Transformer inputs are split along the batch
//! axis so that no intermediate tensor exceeds [`Options::max_elems`]; this
//! is exact, since attention never mixes batch items.

use burn::tensor::activation::{gelu, glu, sigmoid, softmax};
use burn::tensor::backend::Backend;
use burn::tensor::module::{attention, linear};
use burn::tensor::{Tensor, TensorData};

use crate::weights::{Store, WeightError, transpose};
use crate::{BandGroup, DEPTH, DIM, DIM_HEAD, FEATURES, HEADS, HIDDEN, STEMS, band_groups};

const INNER: usize = HEADS * DIM_HEAD;

/// How attention is computed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttentionKind {
    /// `burn::tensor::module::attention` (a fused flash-attention kernel on
    /// CubeCL backends, a naive one elsewhere).
    Backend,
    /// Explicit `softmax(q k^T / sqrt(d)) v`.
    Naive,
}

#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Upper bound on the elements of any intermediate tensor (attention
    /// scores, q/k/v, feed-forward hidden layer).  Bounds memory and keeps
    /// buffers under WebGPU's binding size limits.
    pub max_elems: usize,
    pub attention: AttentionKind,
}

impl Default for Options {
    fn default() -> Self {
        // 32 M f32 elements = 128 MiB, WebGPU's default maxStorageBufferBindingSize.
        Self {
            max_elems: 32 << 20,
            attention: AttentionKind::Naive,
        }
    }
}

struct Norm<B: Backend> {
    /// gamma * sqrt(dim)
    g: Tensor<B, 1>,
}

impl<B: Backend> Norm<B> {
    fn load(s: &mut Store, name: &str, dim: usize, dev: &B::Device) -> Result<Self, WeightError> {
        let scale = (dim as f32).sqrt();
        let g: Vec<f32> = s
            .get(name, &[dim])?
            .into_iter()
            .map(|v| v * scale)
            .collect();
        Ok(Self {
            g: Tensor::from_data(TensorData::new(g, [dim]), dev),
        })
    }

    fn forward<const D: usize>(&self, x: Tensor<B, D>) -> Tensor<B, D> {
        rms_norm(x, self.g.clone().unsqueeze::<D>())
    }
}

/// `F.normalize(x, dim=-1) * g`, with `g` broadcastable to `x`.
fn rms_norm<B: Backend, const D: usize>(x: Tensor<B, D>, g: Tensor<B, D>) -> Tensor<B, D> {
    let n = (x.clone() * x.clone())
        .sum_dim(D - 1)
        .sqrt()
        .clamp_min(1e-12);
    x / n * g
}

/// Torch `nn.Linear` weight `[out, in]` as a Burn `[in, out]` matrix.
fn lin_w<B: Backend>(
    s: &mut Store,
    name: &str,
    out: usize,
    inp: usize,
    dev: &B::Device,
) -> Result<Tensor<B, 2>, WeightError> {
    let w = transpose(&s.get(name, &[out, inp])?, out, inp);
    Ok(Tensor::from_data(TensorData::new(w, [inp, out]), dev))
}

fn vec1<B: Backend>(
    s: &mut Store,
    name: &str,
    n: usize,
    dev: &B::Device,
) -> Result<Tensor<B, 1>, WeightError> {
    Ok(Tensor::from_data(
        TensorData::new(s.get(name, &[n])?, [n]),
        dev,
    ))
}

struct Attention<B: Backend> {
    norm: Norm<B>,
    qkv: Tensor<B, 2>,
    gates_w: Tensor<B, 2>,
    gates_b: Tensor<B, 1>,
    out: Tensor<B, 2>,
}

struct FeedForward<B: Backend> {
    norm: Norm<B>,
    w1: Tensor<B, 2>,
    b1: Tensor<B, 1>,
    w2: Tensor<B, 2>,
    b2: Tensor<B, 1>,
}

/// One `Transformer(depth=1)`: attention then feed-forward, both residual.
struct Block<B: Backend> {
    attn: Attention<B>,
    ff: FeedForward<B>,
}

impl<B: Backend> Block<B> {
    fn load(s: &mut Store, p: &str, dev: &B::Device) -> Result<Self, WeightError> {
        let a = format!("{p}.layers.0.0");
        let f = format!("{p}.layers.0.1");
        Ok(Self {
            attn: Attention {
                norm: Norm::load(s, &format!("{a}.norm.gamma"), DIM, dev)?,
                qkv: lin_w(s, &format!("{a}.to_qkv.weight"), 3 * INNER, DIM, dev)?,
                gates_w: lin_w(s, &format!("{a}.to_gates.weight"), HEADS, DIM, dev)?,
                gates_b: vec1(s, &format!("{a}.to_gates.bias"), HEADS, dev)?,
                out: lin_w(s, &format!("{a}.to_out.0.weight"), DIM, INNER, dev)?,
            },
            ff: FeedForward {
                norm: Norm::load(s, &format!("{f}.net.0.gamma"), DIM, dev)?,
                w1: lin_w(s, &format!("{f}.net.1.weight"), HIDDEN, DIM, dev)?,
                b1: vec1(s, &format!("{f}.net.1.bias"), HIDDEN, dev)?,
                w2: lin_w(s, &format!("{f}.net.4.weight"), DIM, HIDDEN, dev)?,
                b2: vec1(s, &format!("{f}.net.4.bias"), DIM, dev)?,
            },
        })
    }

    /// `x`: `[batch, seq, DIM]`; attention runs along `seq`.
    fn forward(&self, x: Tensor<B, 3>, rope: &Rope<B>, opts: &Options) -> Tensor<B, 3> {
        let [batch, seq, _] = x.dims();
        let step = batch_step(batch, seq, opts);
        if step == batch {
            return self.forward_all(x, rope, opts);
        }
        let mut parts = Vec::with_capacity(batch.div_ceil(step));
        let mut i = 0;
        while i < batch {
            let n = step.min(batch - i);
            parts.push(self.forward_all(x.clone().narrow(0, i, n), rope, opts));
            i += n;
        }
        Tensor::cat(parts, 0)
    }

    fn forward_all(&self, x: Tensor<B, 3>, rope: &Rope<B>, opts: &Options) -> Tensor<B, 3> {
        let [b, n, _] = x.dims();
        let a = &self.attn;
        let xn = a.norm.forward(x.clone());
        let qkv = linear(xn.clone(), a.qkv.clone(), None).reshape([b, n, 3, HEADS, DIM_HEAD]);
        let part = |i: usize| {
            qkv.clone()
                .narrow(2, i, 1)
                .reshape([b, n, HEADS, DIM_HEAD])
                .swap_dims(1, 2)
        };
        let (q, k, v) = (rope.apply(part(0)), rope.apply(part(1)), part(2));
        let out = match opts.attention {
            AttentionKind::Backend => attention(q, k, v, None),
            AttentionKind::Naive => {
                let scale = (DIM_HEAD as f64).powf(-0.5);
                let scores = q.matmul(k.swap_dims(2, 3)).mul_scalar(scale);
                softmax(scores, 3).matmul(v)
            }
        };
        let gates = sigmoid(linear(xn, a.gates_w.clone(), Some(a.gates_b.clone())));
        let out = out * gates.swap_dims(1, 2).unsqueeze_dim::<4>(3);
        let out = out.swap_dims(1, 2).reshape([b, n, INNER]);
        let x = x + linear(out, a.out.clone(), None);

        let f = &self.ff;
        let h = gelu(linear(
            f.norm.forward(x.clone()),
            f.w1.clone(),
            Some(f.b1.clone()),
        ));
        x + linear(h, f.w2.clone(), Some(f.b2.clone()))
    }
}

/// Batch items per sub-batch so that no intermediate exceeds `max_elems`.
fn batch_step(batch: usize, seq: usize, opts: &Options) -> usize {
    let mut per_item = seq * (3 * INNER).max(HIDDEN);
    if opts.attention == AttentionKind::Naive {
        per_item = per_item.max(HEADS * seq * seq);
    }
    (opts.max_elems / per_item).clamp(1, batch)
}

/// Which transformer of a layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    /// Attention along time; input `[BANDS, T, DIM]`.
    Time,
    /// Attention along frequency; input `[T, BANDS, DIM]`.
    Freq,
}

/// Rotary position embedding with interleaved pairs, as
/// `rotary_embedding_torch.RotaryEmbedding(dim=64)` applies it.
struct Rope<B: Backend> {
    cos: Tensor<B, 4>,
    sin: Tensor<B, 4>,
}

impl<B: Backend> Rope<B> {
    fn new(seq: usize, dev: &B::Device) -> Self {
        let mut cos = Vec::with_capacity(seq * DIM_HEAD);
        let mut sin = Vec::with_capacity(seq * DIM_HEAD);
        for p in 0..seq {
            for i in 0..DIM_HEAD / 2 {
                let freq = (1.0 / 10000f64.powf((2 * i) as f64 / DIM_HEAD as f64)) as f32;
                // torch forms the angle in f32 before taking cos and sin
                let angle = (p as f32 * freq) as f64;
                for _ in 0..2 {
                    cos.push(angle.cos() as f32);
                    sin.push(angle.sin() as f32);
                }
            }
        }
        let shape = [1, 1, seq, DIM_HEAD];
        Self {
            cos: Tensor::from_data(TensorData::new(cos, shape), dev),
            sin: Tensor::from_data(TensorData::new(sin, shape), dev),
        }
    }

    /// `x`: `[batch, heads, seq, DIM_HEAD]`.
    fn apply(&self, x: Tensor<B, 4>) -> Tensor<B, 4> {
        let [b, h, n, d] = x.dims();
        let pairs = x.clone().reshape([b, h, n, d / 2, 2]);
        let x1 = pairs.clone().narrow(4, 0, 1);
        let x2 = pairs.narrow(4, 1, 1);
        let rotated = Tensor::cat(vec![x2.neg(), x1], 4).reshape([b, h, n, d]);
        x * self.cos.clone() + rotated * self.sin.clone()
    }
}

/// Band-split input projection for one [`BandGroup`].
struct SplitGroup<B: Backend> {
    g: BandGroup,
    gamma: Tensor<B, 3>, // [count, 1, width]
    w: Tensor<B, 3>,     // [count, width, DIM]
    b: Tensor<B, 3>,     // [count, 1, DIM]
}

/// Mask-estimator MLP for one [`BandGroup`] of one stem.
struct MaskGroup<B: Backend> {
    g: BandGroup,
    w1: Tensor<B, 3>, // [count, DIM, HIDDEN]
    b1: Tensor<B, 3>, // [count, 1, HIDDEN]
    w2: Tensor<B, 3>, // [count, HIDDEN, 2 * width]
    b2: Tensor<B, 3>, // [count, 1, 2 * width]
}

/// Stack per-band `nn.Linear` weights `[out, in]` into `[count, in, out]`.
fn stacked_w<B: Backend>(
    s: &mut Store,
    names: &[String],
    out: usize,
    inp: usize,
    dev: &B::Device,
) -> Result<Tensor<B, 3>, WeightError> {
    let mut all = Vec::with_capacity(names.len() * out * inp);
    for n in names {
        all.extend(transpose(&s.get(n, &[out, inp])?, out, inp));
    }
    Ok(Tensor::from_data(
        TensorData::new(all, [names.len(), inp, out]),
        dev,
    ))
}

/// Stack per-band vectors `[n]` into `[count, 1, n]`, each scaled by `scale`.
fn stacked_v<B: Backend>(
    s: &mut Store,
    names: &[String],
    n: usize,
    scale: f32,
    dev: &B::Device,
) -> Result<Tensor<B, 3>, WeightError> {
    let mut all = Vec::with_capacity(names.len() * n);
    for name in names {
        all.extend(s.get(name, &[n])?.into_iter().map(|v| v * scale));
    }
    Ok(Tensor::from_data(
        TensorData::new(all, [names.len(), 1, n]),
        dev,
    ))
}

pub struct BsRoformer<B: Backend> {
    split: Vec<SplitGroup<B>>,
    layers: Vec<(Block<B>, Block<B>)>,
    final_norm: Norm<B>,
    masks: Vec<Vec<MaskGroup<B>>>,
    pub opts: Options,
    device: B::Device,
}

impl<B: Backend> BsRoformer<B> {
    /// Builds the network from an exported safetensors file.  Every tensor
    /// in the file must be used and have the expected shape.
    pub fn load(bytes: &[u8], device: &B::Device, opts: Options) -> Result<Self, WeightError> {
        let mut store = Store::new(bytes)?;
        let s = &mut store;
        let dev = device;
        let groups = band_groups();

        let mut split = Vec::new();
        for g in &groups {
            let names = |suffix: &str| -> Vec<String> {
                (g.first..g.first + g.count)
                    .map(|k| format!("band_split.to_features.{k}.{suffix}"))
                    .collect()
            };
            split.push(SplitGroup {
                g: *g,
                gamma: stacked_v(s, &names("0.gamma"), g.width, (g.width as f32).sqrt(), dev)?,
                w: stacked_w(s, &names("1.weight"), DIM, g.width, dev)?,
                b: stacked_v(s, &names("1.bias"), DIM, 1.0, dev)?,
            });
        }

        let mut layers = Vec::with_capacity(DEPTH);
        for i in 0..DEPTH {
            layers.push((
                Block::load(s, &format!("layers.{i}.0"), dev)?,
                Block::load(s, &format!("layers.{i}.1"), dev)?,
            ));
        }
        let final_norm = Norm::load(s, "final_norm.gamma", DIM, dev)?;

        let mut masks = Vec::with_capacity(STEMS.len());
        for stem in 0..STEMS.len() {
            let mut per_group = Vec::new();
            for g in &groups {
                let names = |suffix: &str| -> Vec<String> {
                    (g.first..g.first + g.count)
                        .map(|k| format!("mask_estimators.{stem}.to_freqs.{k}.0.{suffix}"))
                        .collect()
                };
                per_group.push(MaskGroup {
                    g: *g,
                    w1: stacked_w(s, &names("0.weight"), HIDDEN, DIM, dev)?,
                    b1: stacked_v(s, &names("0.bias"), HIDDEN, 1.0, dev)?,
                    w2: stacked_w(s, &names("2.weight"), 2 * g.width, HIDDEN, dev)?,
                    b2: stacked_v(s, &names("2.bias"), 2 * g.width, 1.0, dev)?,
                });
            }
            masks.push(per_group);
        }
        store.finish()?;
        Ok(Self {
            split,
            layers,
            final_norm,
            masks,
            opts,
            device: device.clone(),
        })
    }

    pub fn device(&self) -> &B::Device {
        &self.device
    }

    /// Features `[T, FEATURES]` to complex masks `[STEMS, T, FEATURES]`.
    /// The stacked result exceeds [`Options::max_elems`] for full chunks
    /// (113 M elements at 1,151 frames); use [`Self::hidden`] and
    /// [`Self::mask_one`] there.
    pub fn forward(&self, features: Tensor<B, 2>) -> Tensor<B, 3> {
        self.mask(self.hidden(features))
    }

    /// Features `[T, FEATURES]` to the normalised hidden state
    /// `[BANDS, T, DIM]` that every mask estimator reads; pair with
    /// [`Self::mask_one`] to fetch one stem at a time.
    pub fn hidden(&self, features: Tensor<B, 2>) -> Tensor<B, 3> {
        let mut x = self.band_split(features);
        for i in 0..self.layers.len() {
            x = self.layer(i, x);
        }
        self.final_norm(x)
    }

    /// `[T, FEATURES]` to `[BANDS, T, DIM]`.
    pub fn band_split(&self, features: Tensor<B, 2>) -> Tensor<B, 3> {
        let [t, f] = features.dims();
        assert_eq!(f, FEATURES);
        let parts = self
            .split
            .iter()
            .map(|sg| {
                let g = sg.g;
                let x = features
                    .clone()
                    .narrow(1, g.offset, g.count * g.width)
                    .reshape([t, g.count, g.width])
                    .swap_dims(0, 1);
                let x = rms_norm(x, sg.gamma.clone());
                x.matmul(sg.w.clone()) + sg.b.clone()
            })
            .collect();
        Tensor::cat(parts, 0)
    }

    /// One time transformer then one frequency transformer.
    /// `[BANDS, T, DIM]` in and out.
    pub fn layer(&self, i: usize, x: Tensor<B, 3>) -> Tensor<B, 3> {
        let [bands, t, _] = x.dims();
        let (time, freq) = &self.layers[i];
        let x = time.forward(x, &Rope::new(t, &self.device), &self.opts);
        let x = freq.forward(
            x.swap_dims(0, 1),
            &Rope::new(bands, &self.device),
            &self.opts,
        );
        x.swap_dims(0, 1)
    }

    /// Sub-batch size [`Self::layer`] uses for one transformer on an input
    /// of `[batch, seq, DIM]`.  Lets a caller run a layer piece by piece
    /// with [`Self::transformer_part`] and wait for the GPU in between.
    pub fn sub_batch(&self, batch: usize, seq: usize) -> usize {
        batch_step(batch, seq, &self.opts)
    }

    /// One transformer of layer `i` on a sub-batch `[n, seq, DIM]`
    /// (`seq` is time for [`Axis::Time`], bands for [`Axis::Freq`]).
    /// Concatenating the parts along axis 0 gives what [`Self::layer`]
    /// computes for that transformer.
    pub fn transformer_part(&self, i: usize, axis: Axis, x: Tensor<B, 3>) -> Tensor<B, 3> {
        let [_, seq, _] = x.dims();
        let (time, freq) = &self.layers[i];
        let block = if axis == Axis::Time { time } else { freq };
        block.forward_all(x, &Rope::new(seq, &self.device), &self.opts)
    }

    pub fn final_norm(&self, x: Tensor<B, 3>) -> Tensor<B, 3> {
        self.final_norm.forward(x)
    }

    /// `[BANDS, T, DIM]` to `[STEMS, T, FEATURES]`.
    pub fn mask(&self, x: Tensor<B, 3>) -> Tensor<B, 3> {
        let stems = (0..self.masks.len())
            .map(|s| self.mask_one(s, x.clone()))
            .collect();
        Tensor::stack(stems, 0)
    }

    /// Mask for one stem, `[T, FEATURES]`.
    pub fn mask_one(&self, stem: usize, x: Tensor<B, 3>) -> Tensor<B, 2> {
        let [_, t, _] = x.dims();
        let mut parts = Vec::new();
        for mg in &self.masks[stem] {
            let g = mg.g;
            let per_band = t * HIDDEN.max(2 * g.width);
            let step = (self.opts.max_elems / per_band).clamp(1, g.count);
            let mut k = 0;
            let mut outs = Vec::new();
            while k < g.count {
                let n = step.min(g.count - k);
                let xg = x.clone().narrow(0, g.first + k, n);
                let h = (xg.matmul(mg.w1.clone().narrow(0, k, n)) + mg.b1.clone().narrow(0, k, n))
                    .tanh();
                let y = h.matmul(mg.w2.clone().narrow(0, k, n)) + mg.b2.clone().narrow(0, k, n);
                outs.push(glu(y, 2)); // [n, T, width]
                k += n;
            }
            let y = Tensor::cat(outs, 0)
                .swap_dims(0, 1)
                .reshape([t, g.count * g.width]);
            parts.push(y);
        }
        Tensor::cat(parts, 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use burn::backend::NdArray;

    #[test]
    fn rope_matches_hand_rotation() {
        let dev = Default::default();
        let rope = Rope::<NdArray>::new(3, &dev);
        let x: Vec<f32> = (0..3 * DIM_HEAD).map(|i| (i % 7) as f32 - 3.0).collect();
        let t =
            Tensor::<NdArray, 4>::from_data(TensorData::new(x.clone(), [1, 1, 3, DIM_HEAD]), &dev);
        let y: Vec<f32> = rope.apply(t).into_data().to_vec().unwrap();
        for p in 0..3 {
            for i in 0..DIM_HEAD / 2 {
                let theta = p as f64 / 10000f64.powf(2.0 * i as f64 / DIM_HEAD as f64);
                let (a, b) = (
                    x[p * DIM_HEAD + 2 * i] as f64,
                    x[p * DIM_HEAD + 2 * i + 1] as f64,
                );
                let e0 = a * theta.cos() - b * theta.sin();
                let e1 = b * theta.cos() + a * theta.sin();
                assert!((y[p * DIM_HEAD + 2 * i] as f64 - e0).abs() < 1e-5);
                assert!((y[p * DIM_HEAD + 2 * i + 1] as f64 - e1).abs() < 1e-5);
            }
        }
    }

    #[test]
    fn rms_norm_is_unit_rms_times_gamma() {
        let dev = Default::default();
        let x = Tensor::<NdArray, 2>::from_data(
            TensorData::new(vec![3.0f32, 4.0, 0.0, 0.0], [2, 2]),
            &dev,
        );
        let g = Tensor::<NdArray, 2>::from_data(TensorData::new(vec![2.0f32, 2.0], [1, 2]), &dev);
        let y: Vec<f32> = rms_norm(x, g).into_data().to_vec().unwrap();
        assert_eq!(y, vec![1.2, 1.6, 0.0, 0.0]);
    }
}
