//! Reading the exported safetensors file (`tools/roformer_export.py`).
//!
//! Tensors keep the PyTorch parameter names.  F32, F16 and BF16 files are
//! accepted; values are widened to f32 one tensor at a time while the model
//! is built, so peak memory is the file plus one tensor.  The file can stay
//! outside WebAssembly memory (see [`Source`]); then only the header and
//! one tensor at a time are copied in.

use std::borrow::Cow;
use std::fmt;

use half::{bf16, f16};
use safetensors::tensor::Metadata;
use safetensors::{Dtype, SafeTensorError};

#[derive(Debug)]
pub enum WeightError {
    Parse(String),
    Missing(String),
    Shape {
        name: String,
        expected: Vec<usize>,
        found: Vec<usize>,
    },
    Dtype(String),
    Unused(Vec<String>),
    /// The file could not be read; it may well be a good model.
    Read(String),
}

impl fmt::Display for WeightError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WeightError::Parse(e) => write!(f, "not a safetensors file: {e}"),
            WeightError::Missing(n) => write!(f, "missing tensor {n}"),
            WeightError::Shape {
                name,
                expected,
                found,
            } => {
                write!(
                    f,
                    "tensor {name} has shape {found:?}, expected {expected:?}"
                )
            }
            WeightError::Dtype(d) => write!(f, "unsupported dtype {d}"),
            WeightError::Read(e) => write!(f, "could not read the file: {e}"),
            WeightError::Unused(n) => {
                write!(
                    f,
                    "{} tensors not used by the model, e.g. {}",
                    n.len(),
                    n[0]
                )
            }
        }
    }
}

impl std::error::Error for WeightError {}

/// Where a safetensors file's bytes come from.  A slice in memory, or (in
/// the browser) the stored or picked file itself, or a `Uint8Array` it was
/// read into, so the whole file never has to be copied into WebAssembly
/// memory: only the header and one tensor at a time are.
pub trait Source {
    /// Bytes in the file.
    fn size(&self) -> usize;
    /// Bytes `start..end`; both are within `0..=size()`.  An error means
    /// the file could not be read (it changed, or the browser failed), not
    /// that its contents are wrong.
    fn read(&self, start: usize, end: usize) -> Result<Cow<'_, [u8]>, String>;
}

impl Source for &[u8] {
    fn size(&self) -> usize {
        self.len()
    }

    fn read(&self, start: usize, end: usize) -> Result<Cow<'_, [u8]>, String> {
        Ok(Cow::Borrowed(&self[start..end]))
    }
}

/// Bytes `start..end` of `src`, all of them.
fn read_exact(src: &dyn Source, start: usize, end: usize) -> Result<Cow<'_, [u8]>, WeightError> {
    let bytes = src.read(start, end).map_err(WeightError::Read)?;
    if bytes.len() != end - start {
        return Err(WeightError::Read(format!(
            "got {} bytes at {start}, expected {}",
            bytes.len(),
            end - start
        )));
    }
    Ok(bytes)
}

/// The largest header `safetensors` itself accepts.
const MAX_HEADER: usize = 100_000_000;

/// The header of a safetensors file, checked as
/// `SafeTensors::read_metadata` does, without its tensor data: returns the
/// header and where the tensor data starts.
fn read_header(src: &dyn Source) -> Result<(Metadata, usize), WeightError> {
    let parse = |e: SafeTensorError| WeightError::Parse(e.to_string());
    let len = src.size();
    if len < 8 {
        return Err(parse(SafeTensorError::HeaderTooSmall));
    }
    let size: [u8; 8] = read_exact(src, 0, 8)?[..].try_into().expect("8 bytes");
    let n: usize = u64::from_le_bytes(size)
        .try_into()
        .map_err(|_| parse(SafeTensorError::HeaderTooLarge))?;
    if n > MAX_HEADER {
        return Err(parse(SafeTensorError::HeaderTooLarge));
    }
    let stop = n
        .checked_add(8)
        .filter(|&stop| stop <= len)
        .ok_or(parse(SafeTensorError::InvalidHeaderLength))?;
    let header = read_exact(src, 8, stop)?;
    let text =
        std::str::from_utf8(&header).map_err(|e| parse(SafeTensorError::InvalidHeader(e)))?;
    let meta: Metadata = serde_json::from_str(text)
        .map_err(|e| parse(SafeTensorError::InvalidHeaderDeserialization(e)))?;
    if meta.data_len().checked_add(stop) != Some(len) {
        return Err(parse(SafeTensorError::MetadataIncompleteBuffer));
    }
    Ok((meta, stop))
}

/// Parsed safetensors file that hands out f32 copies of tensors and keeps
/// track of which names were read.
pub struct Store<'a> {
    src: Box<dyn Source + 'a>,
    meta: Metadata,
    /// Offset of the tensor data in the file.
    data: usize,
    used: std::collections::HashSet<String>,
}

impl<'a> Store<'a> {
    pub fn new(bytes: &'a [u8]) -> Result<Self, WeightError> {
        Self::from_source(bytes)
    }

    pub fn from_source(src: impl Source + 'a) -> Result<Self, WeightError> {
        let (meta, data) = read_header(&src)?;
        Ok(Self {
            src: Box::new(src),
            meta,
            data,
            used: Default::default(),
        })
    }

    /// The tensor `name` as f32, checked against `shape`.
    pub fn get(&mut self, name: &str, shape: &[usize]) -> Result<Vec<f32>, WeightError> {
        let info = self
            .meta
            .info(name)
            .ok_or_else(|| WeightError::Missing(name.to_string()))?;
        if info.shape != shape {
            return Err(WeightError::Shape {
                name: name.to_string(),
                expected: shape.to_vec(),
                found: info.shape.clone(),
            });
        }
        let (s, e) = info.data_offsets;
        let dtype = info.dtype;
        let raw = read_exact(self.src.as_ref(), self.data + s, self.data + e)?;
        let out = match dtype {
            Dtype::F32 => raw
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect(),
            Dtype::F16 => raw
                .chunks_exact(2)
                .map(|c| f16::from_le_bytes([c[0], c[1]]).to_f32())
                .collect(),
            Dtype::BF16 => raw
                .chunks_exact(2)
                .map(|c| bf16::from_le_bytes([c[0], c[1]]).to_f32())
                .collect(),
            other => return Err(WeightError::Dtype(format!("{other:?}"))),
        };
        self.used.insert(name.to_string());
        Ok(out)
    }

    /// Fails if the file holds tensors the model never asked for.
    pub fn finish(self) -> Result<(), WeightError> {
        let mut unused: Vec<String> = self
            .meta
            .offset_keys()
            .into_iter()
            .filter(|n| !self.used.contains(n))
            .collect();
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

#[cfg(test)]
mod tests {
    use super::*;
    use safetensors::tensor::TensorView;

    fn file() -> Vec<u8> {
        let data: Vec<u8> = [1.0f32, 2.0, 3.0, 4.0]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let view = TensorView::new(Dtype::F32, vec![2, 2], &data).unwrap();
        safetensors::serialize([("w", view)], None).unwrap()
    }

    #[test]
    fn whole_file_parses() {
        let f = file();
        let mut store = Store::new(&f).unwrap();
        assert_eq!(store.get("w", &[2, 2]).unwrap(), vec![1.0, 2.0, 3.0, 4.0]);
    }

    /// What a browser leaves behind when it creates the file but never
    /// finishes writing it.
    #[test]
    fn empty_file_is_not_safetensors() {
        let e = Store::new(&[]).err().unwrap();
        assert_eq!(e.to_string(), "not a safetensors file: header too small");
    }

    #[test]
    fn every_truncation_is_not_safetensors() {
        let f = file();
        for len in 0..f.len() {
            assert!(
                matches!(Store::new(&f[..len]), Err(WeightError::Parse(_))),
                "{len} of {} bytes parsed",
                f.len()
            );
        }
    }

    /// Several tensors of each dtype, as the exporter writes them.
    fn mixed() -> Vec<u8> {
        let a: Vec<u8> = (0..12)
            .flat_map(|i| (i as f32 * 0.5 - 2.0).to_le_bytes())
            .collect();
        let b: Vec<u8> = (0..6)
            .flat_map(|i| f16::from_f32(i as f32 / 3.0).to_le_bytes())
            .collect();
        let c: Vec<u8> = (0..4)
            .flat_map(|i| bf16::from_f32(1.5 * i as f32).to_le_bytes())
            .collect();
        safetensors::serialize(
            [
                ("a", TensorView::new(Dtype::F32, vec![3, 4], &a).unwrap()),
                ("b", TensorView::new(Dtype::F16, vec![6], &b).unwrap()),
                ("c", TensorView::new(Dtype::BF16, vec![2, 2], &c).unwrap()),
            ],
            None,
        )
        .unwrap()
    }

    /// Reads through [`Source`] only, as the browser does, and counts the
    /// bytes asked for.
    struct Counting<'a>(&'a [u8], std::cell::Cell<usize>);

    impl Source for &Counting<'_> {
        fn size(&self) -> usize {
            self.0.len()
        }
        fn read(&self, start: usize, end: usize) -> Result<Cow<'_, [u8]>, String> {
            self.1.set(self.1.get() + end - start);
            Ok(Cow::Owned(self.0[start..end].to_vec()))
        }
    }

    /// A file that fails, or comes back short, from byte `at` on, as a
    /// browser's file read can.
    struct Failing<'a> {
        bytes: &'a [u8],
        at: usize,
        short: bool,
    }

    impl Source for &Failing<'_> {
        fn size(&self) -> usize {
            self.bytes.len()
        }
        fn read(&self, start: usize, end: usize) -> Result<Cow<'_, [u8]>, String> {
            if end <= self.at {
                Ok(Cow::Borrowed(&self.bytes[start..end]))
            } else if self.short {
                Ok(Cow::Borrowed(&self.bytes[start..end - 1]))
            } else {
                Err("NotReadableError".into())
            }
        }
    }

    #[test]
    fn a_read_that_fails_or_comes_back_short_is_a_read_error_not_a_bad_file() {
        let f = mixed();
        let header = 8 + u64::from_le_bytes(f[..8].try_into().unwrap()) as usize;
        for short in [false, true] {
            for at in [0, 8, header, f.len() - 1] {
                let src = Failing {
                    bytes: &f,
                    at,
                    short,
                };
                let got = Store::from_source(&src).and_then(|mut s| {
                    s.get("a", &[3, 4])?;
                    s.get("b", &[6])?;
                    s.get("c", &[2, 2])
                });
                assert!(
                    matches!(got, Err(WeightError::Read(_))),
                    "failing at {at} (short {short}): {got:?}"
                );
            }
        }
    }

    #[test]
    fn a_source_gives_the_same_tensors_as_safetensors() {
        let f = mixed();
        let st = safetensors::SafeTensors::deserialize(&f).unwrap();
        let src = Counting(&f, Default::default());
        let mut store = Store::from_source(&src).unwrap();
        for (name, shape) in [("a", vec![3, 4]), ("b", vec![6]), ("c", vec![2, 2])] {
            let view = st.tensor(name).unwrap();
            let want: Vec<f32> = match view.dtype() {
                Dtype::F32 => view
                    .data()
                    .chunks_exact(4)
                    .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
                    .collect(),
                Dtype::F16 => view
                    .data()
                    .chunks_exact(2)
                    .map(|c| f16::from_le_bytes([c[0], c[1]]).to_f32())
                    .collect(),
                _ => view
                    .data()
                    .chunks_exact(2)
                    .map(|c| bf16::from_le_bytes([c[0], c[1]]).to_f32())
                    .collect(),
            };
            assert_eq!(store.get(name, &shape).unwrap(), want, "{name}");
        }
        store.finish().unwrap();
        assert_eq!(src.1.get(), f.len(), "every byte read once, none twice");
    }

    #[test]
    fn unused_wrong_shape_and_missing_tensors_are_reported() {
        let f = mixed();
        let mut store = Store::new(&f).unwrap();
        assert!(matches!(
            store.get("a", &[4, 3]),
            Err(WeightError::Shape { .. })
        ));
        assert!(matches!(store.get("z", &[1]), Err(WeightError::Missing(_))));
        store.get("b", &[6]).unwrap();
        match store.finish() {
            Err(WeightError::Unused(n)) => assert_eq!(n, vec!["a".to_string(), "c".to_string()]),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn rejects_what_safetensors_rejects() {
        let f = mixed();
        let mut longer = f.clone();
        longer.push(0);
        let mut bad_json = f.clone();
        bad_json[8] = b'[';
        for (what, bytes) in [("one byte too many", longer), ("not json", bad_json)] {
            let theirs = safetensors::SafeTensors::deserialize(&bytes).err().unwrap();
            let ours = Store::new(&bytes).err().unwrap();
            assert_eq!(
                ours.to_string(),
                format!("not a safetensors file: {theirs}"),
                "{what}"
            );
        }
        for len in 0..f.len() {
            let theirs = safetensors::SafeTensors::deserialize(&f[..len])
                .err()
                .unwrap();
            let ours = Store::new(&f[..len]).err().unwrap();
            assert_eq!(
                ours.to_string(),
                format!("not a safetensors file: {theirs}"),
                "{len} bytes"
            );
        }
    }
}
