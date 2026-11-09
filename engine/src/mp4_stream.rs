//! The rule of [`crate::tag_stream::TagStream`] for an MP4 from a source
//! that cannot seek (finding 13-2 of review round 13, TD-88 closed, MEM-4).
//!
//! A source that cannot seek gets no walk of `engine::mp4_boxes`.  On such
//! a source, Symphonia 0.6.1 parses the top-level boxes up to the first
//! `mdat` or `moof` only (`symphonia-format-isomp4` `demuxer.rs`, lines
//! 210 to 225).  After that, it reads `moof` only (line 490).  So [`Mp4`]
//! follows the boxes of that head as the bytes pass, by the rules of the
//! walk:
//! * It shows each box that the walk hides as `free`, with its size: a
//!   `udta` of `moov`, and a top-level `meta`, `sidx`, later `moov` and
//!   later `ftyp`.  Symphonia skips a `free` box with `ignore_bytes`
//!   (`atoms/mod.rs`, line 722), which holds no buffer on a stream.
//! * It shows a first `ftyp` past `MOST_BOX_BYTES` as `mend_mp4` does, and
//!   an `mvhd` timescale of 0 as 1000.  The stream cannot read the
//!   timescale of the sound track first, since its `mdhd` comes later, and
//!   Symphonia uses the value for its `media_info` only.
//! * It refuses a track of the handler `vide`, a sound track of a protected
//!   entry, and tables that pass the bound of a stream, before Symphonia
//!   reads them.

use crate::mp4_boxes::{MOST_BOX_BYTES, PROTECTED_ENTRIES};
use crate::tag_stream::{Refusal, Rule};

/// The bound of the tables of `moov` that Symphonia holds, for a stream:
/// the stream cannot read the duration of the sound track first, so the
/// bound is that of the longest file that the decode opens.
fn most_held() -> u64 {
    crate::decode::most_moov_bytes_of(u64::MAX)
}

/// The rule of the head of an MP4 stream.
#[derive(Debug, Default)]
pub(crate) struct Mp4 {
    /// The open boxes that it follows into, with where each ends.
    open: Vec<([u8; 4], u64)>,
    ftyp: bool,
    moov: bool,
    /// The bytes of the boxes of `moov` that Symphonia holds, as the walk
    /// counts them (`Found::moov_held`).
    held: u64,
    /// The handler and the first sample entry of the open `trak`.
    handler: Option<[u8; 4]>,
    entry: Option<[u8; 4]>,
    /// The bytes of the leading ID3v2 tags that the stream read past.
    dropped: u64,
    /// The first chunk offset of the first sound track.
    first_chunk: Option<u64>,
}

impl Mp4 {
    /// The rule of an MP4 stream after `dropped` bytes of leading ID3v2
    /// tags.
    pub(crate) fn behind(dropped: u64) -> Self {
        Mp4 {
            dropped,
            ..Mp4::default()
        }
    }

    /// Refuses the first `mdat`, from `body` to `end`, when the first chunk
    /// offset counts from the start of the leading tags.  The stream read
    /// past them, so Symphonia's positions count from the MP4 start, as
    /// the view of a file that can seek does after a tag.  The rule of
    /// `decode::counts_from_mp4_start` decides, with the positions of the
    /// file (TD-89, closed by review round 13).
    fn check_offsets(&self, body: u64, end: u64) -> std::io::Result<()> {
        let Some(chunk) = self.first_chunk.filter(|_| self.dropped > 0) else {
            return Ok(());
        };
        let bodies = [(body + self.dropped, end.saturating_add(self.dropped))];
        if crate::decode::counts_from_mp4_start(chunk, self.dropped, &bodies) {
            return Ok(());
        }
        Err(Refusal::Id3Offsets.error())
    }
    fn hold(&mut self, size: u64) -> std::io::Result<()> {
        self.held = self.held.saturating_add(size);
        if self.held > most_held() {
            return Err(Refusal::Moov.error());
        }
        Ok(())
    }

    /// Refuses the open `trak` by the rules of the walk
    /// (`mp4_boxes::Walker::keep`).
    fn check(&self) -> std::io::Result<()> {
        match self.handler.as_ref() {
            Some(b"vide") => Err(Refusal::Video.error()),
            Some(b"soun") if self.entry.is_some_and(|e| PROTECTED_ENTRIES.contains(&&e)) => {
                Err(Refusal::Protected.error())
            }
            _ => Ok(()),
        }
    }
}

/// The 4 bytes at `at` of `bytes`, if it holds them.
fn four(bytes: &[u8], at: usize) -> Option<[u8; 4]> {
    bytes.get(at..at + 4)?.try_into().ok()
}

impl Rule for Mp4 {
    /// The size, the type, a size of 64 bits, and the body fields that the
    /// rules read: up to the timescale of an `mvhd` of version 1.
    const HEADER: usize = 40;

    fn change(&mut self, at: u64, header: &mut [u8]) -> std::io::Result<Option<u64>> {
        while self.open.last().is_some_and(|(_, end)| *end <= at) {
            self.open.pop();
        }
        // Each child stays inside its parent, as Symphonia's `next_header`
        // keeps it (`atoms/mod.rs`, lines 640 to 697; finding 14-1 of review
        // round 14).  With too few bytes left for a header, or a child that
        // runs past the end, Symphonia goes on at the end of the parent.
        let parent_end = self.open.last().map_or(u64::MAX, |(_, end)| *end);
        let (Some(size), Some(kind)) = (four(header, 0), four(header, 4)) else {
            return Ok(None);
        };
        let (head, size) = match u32::from_be_bytes(size) {
            1 => match header.get(8..16) {
                Some(large) => (16, u64::from_be_bytes(large.try_into().unwrap_or_default())),
                None => return Ok(None),
            },
            0 => (8, parent_end - at),
            size => (8, u64::from(size)),
        };
        if parent_end - at < head as u64 || size > parent_end - at {
            return Ok((parent_end < u64::MAX).then_some(parent_end));
        }
        if size < head as u64 {
            return Ok(None);
        }
        let end = at + size;
        let field = |from: usize| four(header, head + from);
        let mut follow = false;
        match (self.open.last().map(|(kind, _)| kind), &kind) {
            (None, b"mdat" | b"moof") if !self.moov => return Err(Refusal::MoovLast.error()),
            (None, b"mdat") => {
                self.check_offsets(at + head as u64, end)?;
                return Ok(None);
            }
            (None, b"moof") => return Ok(None),
            (None, b"ftyp") if !self.ftyp => {
                self.ftyp = true;
                if size > (8 + MOST_BOX_BYTES) as u64 && size != u64::MAX - at && header.len() >= 32
                {
                    header[..4].copy_from_slice(&16_u32.to_be_bytes());
                    header[16..20].copy_from_slice(&1_u32.to_be_bytes());
                    header[20..24].copy_from_slice(b"free");
                    header[24..32].copy_from_slice(&(size - 16).to_be_bytes());
                }
            }
            (None, b"moov") if !self.moov => {
                self.moov = true;
                follow = true;
            }
            (None, b"moov" | b"ftyp" | b"meta" | b"sidx") | (Some(b"moov"), b"udta") => {
                header[4..8].copy_from_slice(b"free");
            }
            (Some(b"moov"), b"mvhd") => {
                self.hold(size)?;
                let at = if header.get(head) == Some(&1) { 20 } else { 12 };
                if field(at) == Some([0; 4]) {
                    header[head + at..head + at + 4].copy_from_slice(&1000_u32.to_be_bytes());
                }
            }
            (Some(b"moov"), b"mvex") | (Some(b"trak"), b"tkhd" | b"edts") => self.hold(size)?,
            (Some(b"moov"), b"trak") => {
                (self.handler, self.entry) = (None, None);
                follow = true;
            }
            (Some(b"trak"), b"mdia") => {
                self.hold(size)?;
                follow = true;
            }
            (Some(b"mdia"), b"minf") | (Some(b"minf"), b"stbl") => follow = true,
            (Some(b"mdia"), b"hdlr") => {
                self.handler = field(8);
                self.check()?;
            }
            (Some(b"stbl"), b"stsd") => {
                self.entry = field(12);
                self.check()?;
            }
            // The version and flags, the count, then the first entry.
            (Some(b"stbl"), b"stco" | b"co64")
                if self.handler == Some(*b"soun") && self.first_chunk.is_none() =>
            {
                let count = field(4).map_or(0, u32::from_be_bytes);
                self.first_chunk = match (&kind, field(8), field(12)) {
                    (b"stco", Some(first), _) => Some(u64::from(u32::from_be_bytes(first))),
                    (_, Some(high), Some(low)) => Some(
                        u64::from(u32::from_be_bytes(high)) << 32
                            | u64::from(u32::from_be_bytes(low)),
                    ),
                    _ => None,
                }
                .filter(|_| count > 0);
            }
            _ => {}
        }
        if follow {
            self.open.push((kind, end));
            return Ok(Some(at + head as u64));
        }
        Ok((end < u64::MAX).then_some(end))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tag_stream::TagStream;
    use crate::test_m4a::bx;
    use std::io::{Cursor, Read};
    use symphonia::core::io::ReadOnlySource;

    /// A source that gives 1 byte for each read.
    struct OneByte(Cursor<Vec<u8>>);

    impl Read for OneByte {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let end = buf.len().min(1);
            self.0.read(&mut buf[..end])
        }
    }

    /// A box of `kind` and `body` with a size of 64 bits.
    fn large(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let size = (16 + body.len()) as u64;
        [&1_u32.to_be_bytes()[..], kind, &size.to_be_bytes(), body].concat()
    }

    /// The headers come in parts, a box of a size of 64 bits is followed,
    /// and the boxes after the first `mdat` pass as they are.
    #[test]
    fn the_head_of_an_mp4_is_followed_in_parts() {
        let udta = bx(b"udta", &[b"lloyd"]);
        let moov = bx(b"moov", &[&bx(b"mvhd", &[&[0; 20]]), &udta]);
        let file = [
            bx(b"ftyp", &[b"M4A ", &[0; 4]]),
            large(b"meta", b"acme"),
            moov,
            bx(b"mdat", &[b"beachape"]),
            bx(b"meta", &[b"after"]),
        ]
        .concat();
        let source = Box::new(ReadOnlySource::new(OneByte(Cursor::new(file.clone()))));
        let mut out = Vec::new();
        TagStream::new(source, Mp4::default(), 0)
            .read_to_end(&mut out)
            .unwrap();
        let mut want = file.clone();
        let free = |want: &mut Vec<u8>, at: usize| want[at + 4..at + 8].copy_from_slice(b"free");
        free(&mut want, 16);
        let moov = 16 + 20;
        free(&mut want, moov + 8 + 28);
        // The timescale of 0 of the `mvhd` shows as 1000.
        want[moov + 8 + 8 + 12..moov + 8 + 8 + 16].copy_from_slice(&1000_u32.to_be_bytes());
        assert!(out == want, "the head");
    }
}
