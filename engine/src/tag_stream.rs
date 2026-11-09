//! The tags and the bound of channels of a source that cannot seek
//! (finding 12-2 and the tags of a stream, review round 12, MEM-4).
//!
//! A source that cannot seek gets no plan of `engine::hidden`, and gives no
//! `COMM` to read first.  So [`TagStream`] follows the chunk or block
//! headers of an AIFF, a WAV or a FLAC as the bytes go to Symphonia, by the
//! rules of `engine::hidden`.
//! * It shows each tag chunk as `JUNK` and each tag block as PADDING, which
//!   Symphonia reads past.
//! * It shows the block size of an AIFF `SSND` as 0, as the plan does.
//! * It fails the read that holds the channels of a `COMM` of more than
//!   [`MOST_CHANNELS`], before Symphonia gets them.

use std::io::{Read, Seek, SeekFrom};

use symphonia::core::io::MediaSource;

use crate::decode::MOST_CHANNELS;
use crate::hidden::{AIFF_TAG_CHUNKS, FLAC_PADDING, FLAC_TAG_BLOCKS, HIDDEN_CHUNK, WAV_TAG_CHUNKS};

/// The most bytes that a read from the inner source gives at once.  It is
/// small, since the window of the scan of a stream is held too.
const STAGE: usize = 4096;

/// The bytes of a chunk header that a rule reads: the ID, the size, the
/// channels of a `COMM` (8 to 10), and the block size of an `SSND` (12 to
/// 16).
const HEADER: usize = 16;

/// The form of a stream, which says its headers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Form {
    Aiff,
    Wav,
    Flac,
}

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

/// A source that cannot seek, which changes its headers as they pass.
pub(crate) struct TagStream {
    inner: Box<dyn MediaSource>,
    form: Form,
    /// The bytes from the inner source that the next reads give, with the
    /// headers in them changed.
    stage: Vec<u8>,
    /// The bytes of `stage` that the reads gave.
    given: usize,
    /// The position in the stream of the first byte of `stage`.
    at: u64,
    /// Where the next header starts, until the last header that Symphonia
    /// reads from a stream: `SSND`, `data`, or the last FLAC block.
    next: Option<u64>,
}

impl TagStream {
    /// `inner` from its start, whose first header of `form` is at `first`.
    pub(crate) fn new(inner: Box<dyn MediaSource>, form: Form, first: u64) -> Self {
        TagStream {
            inner,
            form,
            stage: Vec::with_capacity(STAGE + HEADER),
            given: 0,
            at: 0,
            next: Some(first),
        }
    }

    /// Reads into `stage` up to `len` bytes, and gives whether the source
    /// ended.
    fn fill(&mut self, len: usize) -> std::io::Result<bool> {
        let from = self.stage.len();
        self.stage.resize(len, 0);
        let read = self.inner.read(&mut self.stage[from..])?;
        self.stage.truncate(from + read);
        Ok(read == 0)
    }

    /// Reads the next bytes into `stage`, each header in them whole, and
    /// changes the headers.
    fn stage_next(&mut self) -> std::io::Result<()> {
        self.at += self.stage.len() as u64;
        self.stage.clear();
        self.given = 0;
        let mut ended = self.fill(STAGE)?;
        while let Some(next) = self.next {
            let end = self.at + self.stage.len() as u64;
            if next >= end {
                return Ok(());
            }
            let from = (next - self.at) as usize;
            while !ended && self.stage.len() < from + HEADER {
                ended = self.fill(from + HEADER)?;
            }
            let to = self.stage.len().min(from + HEADER);
            let form = self.form;
            self.next = change(form, &mut self.stage[from..to])?.map(|len| next + len);
        }
        Ok(())
    }
}

/// Changes `header`, of `form`, by the rules of `engine::hidden`, and gives
/// the bytes to the next header, if Symphonia reads one.
fn change(form: Form, header: &mut [u8]) -> std::io::Result<Option<u64>> {
    if let (Form::Flac, Some(&[a, b, c, d])) = (form, header.get(..4)) {
        let len = u64::from(u32::from_be_bytes([0, b, c, d]));
        if FLAC_TAG_BLOCKS.contains(&(a & 0x7F)) {
            header[0] = (a & 0x80) | FLAC_PADDING;
        }
        return Ok((a & 0x80 == 0).then_some(4 + len));
    }
    let Some(&[a, b, c, d, e, f, g, h]) = header.get(..8).filter(|_| form != Form::Flac) else {
        return Ok(None);
    };
    let id = [a, b, c, d];
    let (size, tags, last) = match form {
        Form::Wav => (
            u32::from_le_bytes([e, f, g, h]),
            &WAV_TAG_CHUNKS[..],
            *b"data",
        ),
        _ => (
            u32::from_be_bytes([e, f, g, h]),
            &AIFF_TAG_CHUNKS[..],
            *b"SSND",
        ),
    };
    if tags.iter().any(|tag| **tag == id) {
        header[..4].copy_from_slice(HIDDEN_CHUNK);
    }
    if form == Form::Aiff && id == *b"COMM" {
        let channels = header
            .get(8..10)
            .map_or(0, |n| u16::from_be_bytes([n[0], n[1]]));
        if channels > MOST_CHANNELS {
            return Err(std::io::Error::other(TooManyChannels(channels)));
        }
    }
    if id == last {
        if let Some(block) = header.get_mut(12..16).filter(|_| form == Form::Aiff) {
            block.fill(0);
        }
        return Ok(None);
    }
    let size = u64::from(size);
    Ok(Some(8 + size + (size & 1)))
}

impl Read for TagStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.given == self.stage.len() {
            self.stage_next()?;
        }
        let staged = &self.stage[self.given..];
        let len = staged.len().min(buf.len());
        buf[..len].copy_from_slice(&staged[..len]);
        self.given += len;
        Ok(len)
    }
}

impl Seek for TagStream {
    fn seek(&mut self, _: SeekFrom) -> std::io::Result<u64> {
        Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
    }
}

impl MediaSource for TagStream {
    fn is_seekable(&self) -> bool {
        false
    }

    fn byte_len(&self) -> Option<u64> {
        self.inner.byte_len()
    }
}

/// `source`, a stream of `container` whose reader starts at `at`, through
/// a [`TagStream`] if its form has tags that it can hide.
pub(crate) fn stream_of(
    source: Box<dyn MediaSource>,
    container: crate::hidden::Container,
    at: u64,
) -> Box<dyn MediaSource> {
    use crate::hidden::Container;
    match container {
        Container::Aiff => Box::new(TagStream::new(source, Form::Aiff, at + 12)),
        Container::Wav => Box::new(TagStream::new(source, Form::Wav, at + 12)),
        Container::Flac => Box::new(TagStream::new(source, Form::Flac, at + 4)),
        _ => source,
    }
}

/// The channels that [`TagStream`] refused, if it failed `error`.
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

    /// Reads `file` whole through [`TagStream`], 1 byte at a time.
    fn read_all(file: Vec<u8>) -> std::io::Result<Vec<u8>> {
        let source = Box::new(ReadOnlySource::new(OneByte(Cursor::new(file))));
        let mut out = Vec::new();
        TagStream::new(source, Form::Aiff, 12).read_to_end(&mut out)?;
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
