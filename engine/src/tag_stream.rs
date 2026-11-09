//! The tags and the bound of channels of a source that cannot seek
//! (finding 12-2 and the tags of a stream, review round 12, MEM-4).
//!
//! A source that cannot seek gets no plan of `engine::hidden`, and gives no
//! `COMM` to read first.  So [`TagStream`] follows the headers of the
//! stream as the bytes go to Symphonia, and a [`Rule`] changes each header.
//! [`Chunks`] is the rule of an AIFF, a WAV or a FLAC, by the rules of
//! `engine::hidden`:
//! * It shows each tag chunk as `JUNK` and each tag block as PADDING, which
//!   Symphonia reads past.
//! * It shows the block size of an AIFF `SSND` as 0, as the plan does.
//! * It fails the read that holds the channels of a `COMM` of more than
//!   [`MOST_CHANNELS`], before Symphonia gets them.

use std::io::{Read, Seek, SeekFrom};

use symphonia::core::io::MediaSource;

use crate::decode::MOST_CHANNELS;
use crate::hidden::{AIFF_TAG_CHUNKS, FLAC_PADDING, FLAC_TAG_BLOCKS, HIDDEN_CHUNK, WAV_TAG_CHUNKS};

/// The bytes that a refill of the stage reads from the inner source at
/// most.  It is small, since the window of the scan of a stream is held
/// too.  The stage holds at most `STAGE + Rule::HEADER` bytes: a part
/// header at its end waits for the next refill (finding 13-1 of review
/// round 13).
pub(crate) const STAGE: usize = 4096;

/// The headers of a stream, and how each changes as it passes.
pub(crate) trait Rule {
    /// The bytes of a header that [`Rule::change`] reads.
    const HEADER: usize;

    /// Changes `header`, the first bytes of the header at `at`: `HEADER`
    /// bytes, or fewer at the end of the stream.  Gives where the next
    /// header starts, or `None` when Symphonia reads no more headers.
    fn change(&mut self, at: u64, header: &mut [u8]) -> std::io::Result<Option<u64>>;
}

/// A source that cannot seek, which changes its headers by `R` as they
/// pass.
pub(crate) struct TagStream<R> {
    inner: Box<dyn MediaSource>,
    rule: R,
    /// The bytes from the inner source that the next reads give, with the
    /// headers in them changed, then a part header.
    stage: Vec<u8>,
    /// The bytes of `stage` that the reads gave.
    given: usize,
    /// The bytes of `stage` that the reads can give: the rest is a part
    /// header.
    ready: usize,
    /// The position in the stream of the first byte of `stage`.
    at: u64,
    /// Where the next header starts, until the last one that Symphonia
    /// reads.
    next: Option<u64>,
    ended: bool,
}

impl<R: Rule> TagStream<R> {
    /// `inner` from its start, whose first header is at `first`.
    pub(crate) fn new(inner: Box<dyn MediaSource>, rule: R, first: u64) -> Self {
        TagStream {
            inner,
            rule,
            stage: Vec::with_capacity(STAGE + R::HEADER),
            given: 0,
            ready: 0,
            at: 0,
            next: Some(first),
            ended: false,
        }
    }

    /// Reads into the free room of `stage`, at most `STAGE + HEADER` bytes.
    fn fill(&mut self) -> std::io::Result<()> {
        let from = self.stage.len();
        self.stage.resize(STAGE + R::HEADER, 0);
        let read = self.inner.read(&mut self.stage[from..])?;
        self.stage.truncate(from + read);
        self.ended = read == 0;
        Ok(())
    }

    /// Moves the part header to the front of `stage`, reads after it, and
    /// changes each whole header.  The bytes before a part header are
    /// ready.
    fn stage_next(&mut self) -> std::io::Result<()> {
        self.stage.drain(..self.ready);
        self.at += self.ready as u64;
        self.given = 0;
        self.ready = 0;
        while self.ready == 0 && !(self.ended && self.stage.is_empty()) {
            self.fill()?;
            self.ready = self.change_whole()?;
        }
        Ok(())
    }

    /// Changes each header in `stage` that is whole, or that the end of the
    /// stream cuts, and gives the bytes before the first part header.
    fn change_whole(&mut self) -> std::io::Result<usize> {
        while let Some(next) = self.next {
            let end = self.at + self.stage.len() as u64;
            if next >= end && !self.ended {
                return Ok(self.stage.len());
            }
            let from = usize::try_from(next - self.at).unwrap_or(usize::MAX);
            if from >= self.stage.len() {
                return Ok(self.stage.len());
            }
            if from + R::HEADER > self.stage.len() && !self.ended {
                return Ok(from);
            }
            let to = self.stage.len().min(from + R::HEADER);
            self.next = self.rule.change(next, &mut self.stage[from..to])?;
        }
        Ok(self.stage.len())
    }
}

/// The rule of the chunks of an AIFF or a WAV, or the blocks of a FLAC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Chunks {
    Aiff,
    Wav,
    Flac,
}

/// Why a read of a stream failed before Symphonia got a header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// A `COMM` of this many channels, more than [`MOST_CHANNELS`].
    Channels(u16),
    /// A track of the handler `vide`.
    Video,
    /// A sound track of a protected sample entry.
    Protected,
    /// A `moov` whose tables pass the bound of a stream.
    Moov,
    /// An MP4 behind a leading ID3v2 tag whose chunk offsets count from the
    /// start of the tag, which the stream read past.
    Id3Offsets,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "a stream refused for {self:?}")
    }
}

impl std::error::Error for Refusal {}

impl Refusal {
    /// The error of a read for this refusal.
    pub(crate) fn error(self) -> std::io::Error {
        std::io::Error::other(self)
    }
}

impl Rule for Chunks {
    /// The ID, the size, the channels of a `COMM` (8 to 10), and the block
    /// size of an `SSND` (12 to 16).
    const HEADER: usize = 16;

    fn change(&mut self, at: u64, header: &mut [u8]) -> std::io::Result<Option<u64>> {
        let form = *self;
        if let (Chunks::Flac, Some(&[a, b, c, d])) = (form, header.get(..4)) {
            let len = u64::from(u32::from_be_bytes([0, b, c, d]));
            if FLAC_TAG_BLOCKS.contains(&(a & 0x7F)) {
                header[0] = (a & 0x80) | FLAC_PADDING;
            }
            return Ok((a & 0x80 == 0).then_some(at + 4 + len));
        }
        let Some(&[a, b, c, d, e, f, g, h]) = header.get(..8).filter(|_| form != Chunks::Flac)
        else {
            return Ok(None);
        };
        let id = [a, b, c, d];
        let (size, tags, last) = match form {
            Chunks::Wav => (
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
        if form == Chunks::Aiff && id == *b"COMM" {
            let channels = header
                .get(8..10)
                .map_or(0, |n| u16::from_be_bytes([n[0], n[1]]));
            if channels > MOST_CHANNELS {
                return Err(Refusal::Channels(channels).error());
            }
        }
        if id == last {
            if let Some(block) = header.get_mut(12..16).filter(|_| form == Chunks::Aiff) {
                block.fill(0);
            }
            return Ok(None);
        }
        let size = u64::from(size);
        Ok(Some(at + 8 + size + (size & 1)))
    }
}

impl<R: Rule> Read for TagStream<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.given == self.ready {
            self.stage_next()?;
        }
        let staged = &self.stage[self.given..self.ready];
        let len = staged.len().min(buf.len());
        buf[..len].copy_from_slice(&staged[..len]);
        self.given += len;
        Ok(len)
    }
}

impl<R> Seek for TagStream<R> {
    fn seek(&mut self, _: SeekFrom) -> std::io::Result<u64> {
        Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
    }
}

impl<R: Rule + Send + Sync> MediaSource for TagStream<R> {
    fn is_seekable(&self) -> bool {
        false
    }

    fn byte_len(&self) -> Option<u64> {
        self.inner.byte_len()
    }
}

/// `source`, a stream of `container` whose reader starts at `at`, after
/// `dropped` bytes of leading ID3v2 tags, through a [`TagStream`] if its
/// form has headers that it changes.
pub(crate) fn stream_of(
    source: Box<dyn MediaSource>,
    container: crate::hidden::Container,
    at: u64,
    dropped: u64,
) -> Box<dyn MediaSource> {
    use crate::hidden::Container;
    match container {
        Container::Aiff => Box::new(TagStream::new(source, Chunks::Aiff, at + 12)),
        Container::Wav => Box::new(TagStream::new(source, Chunks::Wav, at + 12)),
        Container::Flac => Box::new(TagStream::new(source, Chunks::Flac, at + 4)),
        Container::Mp4 => Box::new(TagStream::new(
            source,
            crate::mp4_stream::Mp4::behind(dropped),
            at,
        )),
        _ => source,
    }
}

/// Why [`TagStream`] failed `error`, if it did.
pub(crate) fn refusal_of(error: &std::io::Error) -> Option<Refusal> {
    error.get_ref()?.downcast_ref::<Refusal>().copied()
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
        TagStream::new(source, Chunks::Aiff, 12).read_to_end(&mut out)?;
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
            let error = read.as_ref().err().and_then(refusal_of);
            assert_eq!(
                error,
                refused.then_some(Refusal::Channels(count)),
                "{count} channels"
            );
            if !refused {
                assert!(read.unwrap() == file, "{count} channels: the bytes");
            }
        }
    }

    /// Finding 13-1 of review round 13: over a run of 1,000,000 empty
    /// chunks, each shorter than a header, the stage stays at most
    /// `STAGE + Chunks::HEADER` bytes, and the bytes pass as they are.
    #[test]
    fn the_stage_has_a_bound() {
        let tone = vec![0.25; 16];
        let plain = aiff(&vec![tone; 2], 44_100, 16, Aiff::Plain);
        let empty = [&b"JUNK"[..], &[0; 4]].concat();
        let file = [&plain[..12], &empty.repeat(1_000_000), &plain[12..]].concat();
        let source = Box::new(ReadOnlySource::new(Cursor::new(file.clone())));
        let mut stream = TagStream::new(source, Chunks::Aiff, 12);
        let (mut out, mut buf) = (Vec::new(), [0; 16]);
        loop {
            let read = stream.read(&mut buf).unwrap();
            assert!(
                stream.stage.capacity() <= STAGE + Chunks::HEADER,
                "the stage holds {}",
                stream.stage.capacity()
            );
            if read == 0 {
                break;
            }
            out.extend_from_slice(&buf[..read]);
        }
        assert!(out == file, "the bytes");
    }
}
