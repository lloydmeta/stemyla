//! The bound of channels of an AIFF from a source that cannot seek
//! (finding 12-2 of review round 12, MEM-4).
//!
//! Symphonia's AIFF reader keeps a buffer for each channel of `COMM`, and
//! a source that cannot seek gives no `COMM` to read first.  So
//! [`AiffStream`] follows the chunk headers as the bytes go to Symphonia.
//! It fails the read that holds the channels of a `COMM` of more than
//! [`MOST_CHANNELS`], before Symphonia gets them.

use std::io::{Read, Seek, SeekFrom};

use symphonia::core::io::MediaSource;

use crate::decode::MOST_CHANNELS;

/// The bytes of a chunk header that the check reads: the ID, the size, and
/// the channels of a `COMM`.
const HEADER: usize = 10;

/// The error of a read that holds a `COMM` of this many channels, more than
/// [`MOST_CHANNELS`].
#[derive(Debug)]
pub(crate) struct TooManyChannels(pub(crate) u16);

impl std::fmt::Display for TooManyChannels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "a COMM of {} channels", self.0)
    }
}

impl std::error::Error for TooManyChannels {}

/// A source that cannot seek, of an AIFF, that gives its bytes as they are.
pub(crate) struct AiffStream {
    inner: Box<dyn MediaSource>,
    /// The position in the stream of the next byte that a read gives.
    at: u64,
    /// Where the next chunk header starts, until a `COMM` or an `SSND`
    /// passes: Symphonia reads no chunk header after them from a stream.
    next: Option<u64>,
    header: [u8; HEADER],
}

impl AiffStream {
    /// `inner` from its start, where the `FORM` of the AIFF is at `form`.
    pub(crate) fn new(inner: Box<dyn MediaSource>, form: u64) -> Self {
        AiffStream {
            inner,
            at: 0,
            next: Some(form + 12),
            header: [0; HEADER],
        }
    }

    /// Follows each chunk header in `bytes`, the bytes from `self.at`.
    fn follow(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        let end = self.at + bytes.len() as u64;
        while let Some(next) = self.next {
            let from = next.max(self.at);
            let to = (next + HEADER as u64).min(end);
            for at in from..to {
                self.header[(at - next) as usize] = bytes[(at - self.at) as usize];
            }
            if next + HEADER as u64 > end {
                return Ok(());
            }
            let [a, b, c, d, e, f, g, h, i, j] = self.header;
            let size = u64::from(u32::from_be_bytes([e, f, g, h]));
            self.next = match [a, b, c, d] {
                [b'C', b'O', b'M', b'M'] => {
                    let channels = u16::from_be_bytes([i, j]);
                    if channels > MOST_CHANNELS {
                        return Err(std::io::Error::other(TooManyChannels(channels)));
                    }
                    None
                }
                [b'S', b'S', b'N', b'D'] => None,
                _ => Some(next + 8 + size + (size & 1)),
            };
        }
        Ok(())
    }
}

impl Read for AiffStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buf)?;
        self.follow(&buf[..read])?;
        self.at += read as u64;
        Ok(read)
    }
}

impl Seek for AiffStream {
    fn seek(&mut self, _: SeekFrom) -> std::io::Result<u64> {
        Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
    }
}

impl MediaSource for AiffStream {
    fn is_seekable(&self) -> bool {
        false
    }

    fn byte_len(&self) -> Option<u64> {
        self.inner.byte_len()
    }
}

/// The channels that [`AiffStream`] refused, if it failed `error`.
pub(crate) fn refused_channels(error: &std::io::Error) -> Option<u16> {
    let refused = error.get_ref()?.downcast_ref::<TooManyChannels>()?;
    Some(refused.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_audio::{Aiff, aiff};
    use std::io::Cursor;
    use symphonia::core::io::ReadOnlySource;

    /// A source that gives 1 byte for each read, so each chunk header comes
    /// in parts.
    struct OneByte(Cursor<Vec<u8>>);

    impl Read for OneByte {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let end = buf.len().min(1);
            self.0.read(&mut buf[..end])
        }
    }

    /// Reads `file` whole through [`AiffStream`], 1 byte at a time.
    fn read_all(file: Vec<u8>) -> std::io::Result<Vec<u8>> {
        let source = Box::new(ReadOnlySource::new(OneByte(Cursor::new(file))));
        let mut out = Vec::new();
        AiffStream::new(source, 0).read_to_end(&mut out)?;
        Ok(out)
    }

    /// The bytes of a chunk header come in parts, and a chunk of an odd size
    /// has a pad byte.  A `COMM` of 128 channels passes whole; one of 129
    /// fails at its last byte of channels.
    #[test]
    fn each_chunk_header_is_followed_in_parts() {
        let tone = vec![0.25; 16];
        let plain = aiff(&vec![tone; 2], 44_100, 16, Aiff::Plain);
        let odd = [
            &plain[..12],
            b"JUNK",
            &3_u32.to_be_bytes(),
            b"abc\0",
            &plain[12..],
        ]
        .concat();
        let comm = (0..odd.len())
            .find(|at| odd[*at..].starts_with(b"COMM"))
            .unwrap();
        for (count, refused) in [(MOST_CHANNELS, false), (MOST_CHANNELS + 1, true)] {
            let mut file = odd.clone();
            file[comm + 8..comm + 10].copy_from_slice(&count.to_be_bytes());
            let read = read_all(file.clone());
            let error = read.as_ref().err().and_then(refused_channels);
            assert_eq!(error, refused.then_some(count), "{count} channels");
            if !refused {
                assert!(read.unwrap() == file, "{count} channels: the bytes");
            }
        }
    }
}
