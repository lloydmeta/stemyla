//! Reading the exported safetensors file (`tools/roformer_export.py`).
//!
//! Tensors keep the PyTorch parameter names.  F32, F16 and BF16 files are
//! accepted; values are widened to f32 one tensor at a time while the model
//! is built, so peak memory is the file plus one tensor.

use std::fmt;

use half::{bf16, f16};
use safetensors::{Dtype, SafeTensors};

#[derive(Debug)]
pub enum WeightError {
    Parse(String),
    Missing(String),
    Shape { name: String, expected: Vec<usize>, found: Vec<usize> },
    Dtype(String),
    Unused(Vec<String>),
}

impl fmt::Display for WeightError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WeightError::Parse(e) => write!(f, "not a safetensors file: {e}"),
            WeightError::Missing(n) => write!(f, "missing tensor {n}"),
            WeightError::Shape { name, expected, found } => {
                write!(f, "tensor {name} has shape {found:?}, expected {expected:?}")
            }
            WeightError::Dtype(d) => write!(f, "unsupported dtype {d}"),
            WeightError::Unused(n) => {
                write!(f, "{} tensors not used by the model, e.g. {}", n.len(), n[0])
            }
        }
    }
}

impl std::error::Error for WeightError {}

/// Parsed safetensors file that hands out f32 copies of tensors and keeps
/// track of which names were read.
pub struct Store<'a> {
    st: SafeTensors<'a>,
    used: std::collections::HashSet<String>,
}

impl<'a> Store<'a> {
    pub fn new(bytes: &'a [u8]) -> Result<Self, WeightError> {
        let st = SafeTensors::deserialize(bytes).map_err(|e| WeightError::Parse(e.to_string()))?;
        Ok(Self { st, used: Default::default() })
    }

    /// The tensor `name` as f32, checked against `shape`.
    pub fn get(&mut self, name: &str, shape: &[usize]) -> Result<Vec<f32>, WeightError> {
        let view = self.st.tensor(name).map_err(|_| WeightError::Missing(name.to_string()))?;
        if view.shape() != shape {
            return Err(WeightError::Shape {
                name: name.to_string(),
                expected: shape.to_vec(),
                found: view.shape().to_vec(),
            });
        }
        let raw = view.data();
        let out = match view.dtype() {
            Dtype::F32 => {
                raw.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
            }
            Dtype::F16 => raw.chunks_exact(2).map(|c| f16::from_le_bytes([c[0], c[1]]).to_f32()).collect(),
            Dtype::BF16 => {
                raw.chunks_exact(2).map(|c| bf16::from_le_bytes([c[0], c[1]]).to_f32()).collect()
            }
            other => return Err(WeightError::Dtype(format!("{other:?}"))),
        };
        self.used.insert(name.to_string());
        Ok(out)
    }

    /// Fails if the file holds tensors the model never asked for.
    pub fn finish(self) -> Result<(), WeightError> {
        let mut unused: Vec<String> =
            self.st.names().into_iter().filter(|n| !self.used.contains(*n)).map(String::from).collect();
        if unused.is_empty() {
            Ok(())
        } else {
            unused.sort();
            Err(WeightError::Unused(unused))
        }
    }
}

/// `[rows, cols]` row-major to `[cols, rows]` row-major.
pub fn transpose(x: &[f32], rows: usize, cols: usize) -> Vec<f32> {
    let mut out = vec![0.0; x.len()];
    for r in 0..rows {
        for c in 0..cols {
            out[c * rows + r] = x[r * cols + c];
        }
    }
    out
}
