//! The source that the decoder reads a stored file through, in blocks
//! (D7a of `docs/plans/completed/stream-from-storage-m5.md`).
//!
//! Symphonia needs a source that is `Send + Sync`, which a value that holds
//! a JS object is not.  So the source holds only numbers: its position and
//! its length.  It reads through a function, which in the browser asks a
//! table on the JS side for a range of a `File` with `FileReaderSync`.
//!
//! A failed read is never the end of the file.  Symphonia takes an error
//! of kind `UnexpectedEof` as the end, so a failed read gives an error of
//! kind `Other`, and the source records it in a [`ReadFailure`].  The
//! worker checks that record first, and replies `decode-unstored`, not
//! `decoded` with a short song or `undecodable`.

use engine::decode::MediaSource;
use std::io::{Error, Read, Seek, SeekFrom};
use std::sync::{Arc, Mutex};

/// The first read that failed, if one did: shared by a source and the
/// worker.
#[derive(Clone, Debug, Default)]
pub struct ReadFailure(Arc<Mutex<Option<String>>>);

impl ReadFailure {
    /// Why the first failed read failed, or `None`.
    pub fn text(&self) -> Option<String> {
        self.0.lock().map_or(None, |text| text.clone())
    }

    fn record(&self, text: &str) {
        if let Ok(mut first) = self.0.lock() {
            first.get_or_insert_with(|| text.into());
        }
    }
}

/// A file of `len` bytes, read through `read_range`, which fills its
/// buffer with the bytes from a position, or fails.
pub struct BlockSource<F> {
    read_range: F,
    pos: u64,
    len: u64,
    failure: ReadFailure,
}

impl<F> BlockSource<F>
where
    F: FnMut(u64, &mut [u8]) -> Result<(), String> + Send + Sync,
{
    pub fn new(len: u64, read_range: F, failure: ReadFailure) -> Self {
        Self {
            read_range,
            pos: 0,
            len,
            failure,
        }
    }
}

impl<F> Read for BlockSource<F>
where
    F: FnMut(u64, &mut [u8]) -> Result<(), String> + Send + Sync,
{
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.pos >= self.len || buf.is_empty() {
            return Ok(0);
        }
        let n = usize::try_from(self.len - self.pos).map_or(buf.len(), |left| left.min(buf.len()));
        match (self.read_range)(self.pos, &mut buf[..n]) {
            Ok(()) => {
                self.pos += n as u64;
                Ok(n)
            }
            Err(text) => {
                let text = format!(
                    "bytes {} to {} could not be read: {text}",
                    self.pos,
                    self.pos + n as u64
                );
                self.failure.record(&text);
                Err(Error::other(text))
            }
        }
    }
}

impl<F> Seek for BlockSource<F>
where
    F: FnMut(u64, &mut [u8]) -> Result<(), String> + Send + Sync,
{
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        let from_here = |base: u64, by: i64| {
            base.checked_add_signed(by)
                .ok_or_else(|| Error::other(format!("a seek to {base} + {by} is before the start")))
        };
        self.pos = match to {
            SeekFrom::Start(at) => at,
            SeekFrom::End(by) => from_here(self.len, by)?,
            SeekFrom::Current(by) => from_here(self.pos, by)?,
        };
        Ok(self.pos)
    }
}

impl<F> MediaSource for BlockSource<F>
where
    F: FnMut(u64, &mut [u8]) -> Result<(), String> + Send + Sync,
{
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        Some(self.len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Reads = Box<dyn FnMut(u64, &mut [u8]) -> Result<(), String> + Send + Sync>;

    /// A source of `bytes` that fails each read that reaches `fail_from`.
    fn source(bytes: Vec<u8>, fail_from: u64) -> (BlockSource<Reads>, ReadFailure) {
        let failure = ReadFailure::default();
        let len = bytes.len() as u64;
        let read: Reads = Box::new(move |at: u64, buf: &mut [u8]| {
            if at + buf.len() as u64 > fail_from {
                return Err("NotReadableError: refused by the test".into());
            }
            buf.copy_from_slice(&bytes[at as usize..at as usize + buf.len()]);
            Ok(())
        });
        (BlockSource::new(len, read, failure.clone()), failure)
    }

    #[test]
    fn a_source_reads_its_bytes_in_blocks_and_then_ends() {
        let (mut s, failure) = source((0..10).collect(), u64::MAX);
        let mut buf = [0; 4];
        let mut got = Vec::new();
        loop {
            let n = s.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            got.extend_from_slice(&buf[..n]);
        }
        assert_eq!(got, (0..10).collect::<Vec<u8>>());
        assert_eq!(failure.text(), None);
        assert_eq!(s.byte_len(), Some(10));
    }

    #[test]
    fn a_failed_read_is_an_error_of_its_own_kind_and_is_recorded() {
        let (mut s, failure) = source((0..10).collect(), 6);
        let mut buf = [0; 4];
        assert_eq!(s.read(&mut buf).unwrap(), 4);
        let error = s
            .read(&mut buf)
            .expect_err("bytes 4 to 8 reach the failure");
        assert_eq!(error.kind(), std::io::ErrorKind::Other);
        let text = failure.text().expect("recorded");
        assert!(
            text.contains("bytes 4 to 8") && text.contains("NotReadableError"),
            "{text}"
        );
    }

    #[test]
    fn a_seek_moves_the_position_and_refuses_a_place_before_the_start() {
        let (mut s, _) = source((0..10).collect(), u64::MAX);
        assert_eq!(s.seek(SeekFrom::End(-3)).unwrap(), 7);
        let mut buf = [0; 4];
        assert_eq!(s.read(&mut buf).unwrap(), 3);
        assert_eq!(buf[..3], [7, 8, 9]);
        assert_eq!(s.seek(SeekFrom::Current(-5)).unwrap(), 5);
        assert!(s.seek(SeekFrom::Current(-6)).is_err());
        assert_eq!(s.seek(SeekFrom::Start(20)).unwrap(), 20);
        assert_eq!(s.read(&mut buf).unwrap(), 0, "past the end");
    }
}
