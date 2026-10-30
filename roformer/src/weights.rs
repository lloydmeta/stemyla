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
pub use safetensors::Dtype;
use safetensors::SafeTensorError;
use safetensors::tensor::Metadata;

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

/// The largest header accepted.  `safetensors` allows 100 MB, but this
/// model's is about 200 KB, and a header is copied whole into WebAssembly
/// memory, which never shrinks: a picked file that only claims a huge one
/// would otherwise cost that much for good.
const MAX_HEADER: usize = 1 << 20;

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

    /// The bytes of tensor `name` and its dtype, checked against `shape`.
    /// The caller marks the tensor as read once it has converted them.
    fn raw(&self, name: &str, shape: &[usize]) -> Result<(Cow<'_, [u8]>, Dtype), WeightError> {
        let info = self
            .meta
            .info(name)
            .ok_or_else(|| WeightError::Missing(name.to_string()))?;
        if info.shape != shape {
            return Err(WeightError::Shape {
                name: name.to_string(),
                #[expect(
                    clippy::disallowed_methods,
                    reason = "MEM-1: a tensor's shape, for the error"
                )]
                expected: shape.to_vec(),
                found: info.shape.clone(),
            });
        }
        let (s, e) = info.data_offsets;
        let raw = read_exact(self.src.as_ref(), self.data + s, self.data + e)?;
        Ok((raw, info.dtype))
    }

    /// The tensor `name` as f32, checked against `shape`.
    pub fn get(&mut self, name: &str, shape: &[usize]) -> Result<Vec<f32>, WeightError> {
        let (raw, dtype) = self.raw(name, shape)?;
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

    /// The dtype of tensor `name`, or `None` when the file lacks it.
    pub fn dtype(&self, name: &str) -> Option<Dtype> {
        self.meta.info(name).map(|info| info.dtype)
    }

    /// The values of the F16 tensor `name` as the file holds them (IEEE-754
    /// binary16 bits), checked against `shape`.  It marks the tensor as
    /// read, as [`Store::get`] does.  Any other dtype is an error, and the
    /// tensor stays unread.
    pub fn get_f16_bits(&mut self, name: &str, shape: &[usize]) -> Result<Vec<u16>, WeightError> {
        if let Some(dtype) = self.dtype(name).filter(|d| *d != Dtype::F16) {
            return Err(WeightError::Dtype(format!(
                "{dtype:?} for {name}, where F16 was asked for"
            )));
        }
        let (raw, _) = self.raw(name, shape)?;
        let out = raw
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
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
#[expect(
    clippy::disallowed_methods,
    reason = "MEM-1: test code, which does not ship"
)]
mod tests {
    use super::*;
    use safetensors::tensor::TensorView;

    #[test]
    fn mem1_lint_is_on() {
        #[expect(
            clippy::disallowed_methods,
            reason = "MEM-1 canary: clippy fails here if this path is off"
        )]
        let copy = [1.0_f32].to_vec();
        assert_eq!(copy, [1.0]);
    }

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
    fn a_header_over_a_megabyte_is_refused_before_it_is_read() {
        let mut f = ((1u64 << 20) + 1).to_le_bytes().to_vec();
        f.resize(8 + (1 << 20) + 1, b' ');
        let src = Counting(&f, Default::default());
        let e = Store::from_source(&src).err().unwrap();
        assert_eq!(e.to_string(), "not a safetensors file: header too large");
        assert_eq!(src.1.get(), 8, "only the length was read");
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

    /// The dtype of each tensor, and none for a name the file lacks.
    #[test]
    fn dtypes_are_read_from_the_header() {
        let f = mixed();
        let store = Store::new(&f).unwrap();
        assert_eq!(store.dtype("a"), Some(Dtype::F32));
        assert_eq!(store.dtype("b"), Some(Dtype::F16));
        assert_eq!(store.dtype("c"), Some(Dtype::BF16));
        assert_eq!(store.dtype("z"), None);
    }

    /// A raw read gives an F16 tensor's values as the file holds them, and
    /// marks the tensor as read, as `get` does.
    #[test]
    fn raw_f16_reads_give_the_bits_and_mark_the_tensor_as_read() {
        let f = mixed();
        let src = Counting(&f, Default::default());
        let mut store = Store::from_source(&src).unwrap();
        let want: Vec<u16> = (0..6)
            .map(|i| f16::from_f32(i as f32 / 3.0).to_bits())
            .collect();
        assert_eq!(store.get_f16_bits("b", &[6]).unwrap(), want);
        store.get("a", &[3, 4]).unwrap();
        store.get("c", &[2, 2]).unwrap();
        store.finish().unwrap();
        assert_eq!(src.1.get(), f.len(), "every byte read once, none twice");
    }

    /// A raw f16 read of an F32 or BF16 tensor fails and does not mark it,
    /// and a wrong shape or a missing name fails as `get` fails.
    #[test]
    fn raw_f16_reads_refuse_other_dtypes() {
        let f = mixed();
        let mut store = Store::new(&f).unwrap();
        for (name, shape) in [("a", vec![3, 4]), ("c", vec![2, 2])] {
            match store.get_f16_bits(name, &shape) {
                Err(WeightError::Dtype(d)) => assert!(d.contains("F16"), "{d}"),
                other => panic!("{name}: {other:?}"),
            }
        }
        assert!(matches!(
            store.get_f16_bits("b", &[3, 2]),
            Err(WeightError::Shape { .. })
        ));
        assert!(matches!(
            store.get_f16_bits("z", &[1]),
            Err(WeightError::Missing(_))
        ));
        store.get_f16_bits("b", &[6]).unwrap();
        match store.finish() {
            Err(WeightError::Unused(n)) => assert_eq!(n, vec!["a".to_string(), "c".to_string()]),
            other => panic!("{other:?}"),
        }
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
