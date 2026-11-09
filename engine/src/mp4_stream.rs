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
use crate::tag_stream::{Insert, Refusal, Rule};

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
    /// The bytes before the MP4 start: the leading ID3v2 tags that the
    /// stream read past, then other bytes, which the decode reads past.
    tags: u64,
    junk: u64,
    /// The first chunk offset of the open `trak`, and of the first sound
    /// track, as the walk takes them (`mp4_boxes::Walker::trak`).
    trak_chunk: Option<u64>,
    first_chunk: Option<u64>,
    sound_seen: bool,
    /// The bytes to put before the first chunk, for [`Rule::insert`].
    insert: Option<Insert>,
}

impl Mp4 {
    /// The rule of an MP4 stream after `tags` bytes of leading ID3v2 tags,
    /// then `junk` other bytes, before the MP4 start.
    pub(crate) fn behind(tags: u64, junk: u64) -> Self {
        Mp4 {
            tags,
            junk,
            ..Mp4::default()
        }
    }

    /// Keeps the first chunk of the `trak` that ends, if it is the first
    /// sound track.
    fn end_trak(&mut self) {
        if self.handler == Some(*b"soun") && !self.sound_seen {
            self.sound_seen = true;
            self.first_chunk = self.trak_chunk;
        }
    }

    /// The first `mdat` of the stream, from `at` with a header of `head`
    /// bytes and a body to `end`.  The positions of the stream count from
    /// the MP4 start.  When the chunk offsets count from before it, from
    /// the file start or from the end of the tags as `decode::offsets_base`
    /// chooses, the stream gives the bytes that they count past just
    /// before the first chunk (findings 14-3 and 14-4 of review round 14).
    /// * 8 bytes or more: a `free` box before `mdat`.
    /// * Fewer: zeros after the header of `mdat`, whose size grows to hold
    ///   them.  A size of 32 bits that cannot grow shows as 0, to the end.
    fn first_mdat(&mut self, at: u64, head: usize, end: u64, header: &mut [u8]) {
        let Some(chunk) = self.first_chunk else {
            return;
        };
        // The file positions of the MP4 start, of the end of the tags, and
        // of the body of `mdat`.  `at` counts from the end of the tags.
        let start = self.tags + self.junk;
        let body = (self.tags + at + head as u64, end.saturating_add(self.tags));
        let base = crate::decode::offsets_base(chunk, &[start, self.tags, 0], &[body]);
        let missing = start - base;
        self.insert = match missing {
            0 => None,
            8.. => {
                let (header, head) = free_header(missing);
                Some(Insert {
                    at,
                    header,
                    head,
                    zeros: missing - head as u64,
                })
            }
            _ => {
                match u32::from_be_bytes(header[..4].try_into().unwrap_or_default()) {
                    0 => {}
                    1 => {
                        let size = u64::from_be_bytes(header[8..16].try_into().unwrap_or_default());
                        header[8..16].copy_from_slice(&size.saturating_add(missing).to_be_bytes());
                    }
                    size => {
                        let grown = u32::try_from(u64::from(size) + missing).unwrap_or(0);
                        header[..4].copy_from_slice(&grown.to_be_bytes());
                    }
                }
                Some(Insert {
                    at: at + head as u64,
                    header: [0; 16],
                    head: 0,
                    zeros: missing,
                })
            }
        };
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

/// The header of a `free` box of `len` bytes, at least 8, and its length:
/// 8 bytes, or 16 with a size of 64 bits past `u32::MAX`.
pub(crate) fn free_header(len: u64) -> ([u8; 16], usize) {
    let mut header = [0; 16];
    header[4..8].copy_from_slice(b"free");
    match u32::try_from(len) {
        Ok(size) => {
            header[..4].copy_from_slice(&size.to_be_bytes());
            (header, 8)
        }
        Err(_) => {
            header[..4].copy_from_slice(&1_u32.to_be_bytes());
            header[8..].copy_from_slice(&len.to_be_bytes());
            (header, 16)
        }
    }
}

/// The 4 bytes at `at` of `bytes`, if it holds them.
fn four(bytes: &[u8], at: usize) -> Option<[u8; 4]> {
    bytes.get(at..at + 4)?.try_into().ok()
}

impl Rule for Mp4 {
    fn insert(&mut self) -> Option<Insert> {
        self.insert.take()
    }

    /// The size, the type, a size of 64 bits, and the body fields that the
    /// rules read: up to the timescale of an `mvhd` of version 1.
    const HEADER: usize = 40;

    fn change(&mut self, at: u64, header: &mut [u8]) -> std::io::Result<Option<u64>> {
        while self.open.last().is_some_and(|(_, end)| *end <= at) {
            if self.open.pop().is_some_and(|(kind, _)| kind == *b"trak") {
                self.end_trak();
            }
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
                self.first_mdat(at, head, end, header);
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
                (self.handler, self.entry, self.trak_chunk) = (None, None, None);
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
            // The version and flags, the count, then the first entry.  The
            // last of them in a `trak` is the one, as in the walk.
            (Some(b"stbl"), b"stco" | b"co64") => {
                let count = field(4).map_or(0, u32::from_be_bytes);
                self.trak_chunk = match (&kind, field(8), field(12)) {
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

    /// Finding 14-3 of review round 14: the header of a `free` box of 10
    /// bytes, of `u32::MAX` bytes, and of one byte more, of 64 bits.
    #[test]
    fn the_free_box_has_a_header_of_its_length() {
        let (ten, head) = free_header(10);
        assert_eq!(
            (&ten[..head], head),
            (&[0, 0, 0, 10, b'f', b'r', b'e', b'e'][..], 8)
        );
        let (most, head) = free_header(u64::from(u32::MAX));
        assert_eq!(
            (&most[..head], head),
            (&[0xFF, 0xFF, 0xFF, 0xFF, b'f', b'r', b'e', b'e'][..], 8)
        );
        let more = u64::from(u32::MAX) + 1;
        let (large, head) = free_header(more);
        let want = [&1_u32.to_be_bytes()[..], b"free", &more.to_be_bytes()].concat();
        assert_eq!((&large[..head], head), (&want[..], 16));
    }

    /// A small MP4 behind `tags` bytes of tags, its first chunk at `first`
    /// in a `co64` or an `stco`, and its first `mdat` at the end.
    fn head_of(first: u64, co64: bool) -> Vec<u8> {
        head_with(first, co64, &bx(b"mdat", &[b"beachape"]))
    }

    /// [`head_of`] with `mdat` as its last box.
    fn head_with(first: u64, co64: bool, mdat: &[u8]) -> Vec<u8> {
        let hdlr = bx(b"hdlr", &[&[0; 8], b"soun", &[0; 12]]);
        let offsets = if co64 {
            bx(
                b"co64",
                &[&[0; 4], &1_u32.to_be_bytes(), &first.to_be_bytes()],
            )
        } else {
            bx(
                b"stco",
                &[&[0; 4], &1_u32.to_be_bytes(), &(first as u32).to_be_bytes()],
            )
        };
        let stbl = bx(b"stbl", &[&offsets]);
        let mdia = bx(b"mdia", &[&hdlr, &bx(b"minf", &[&stbl])]);
        let moov = bx(b"moov", &[&bx(b"trak", &[&mdia])]);
        [bx(b"ftyp", &[b"M4A ", &[0; 4]]), moov, Vec::from(mdat)].concat()
    }

    /// Reads `file` through the rule of a stream behind `tags` bytes, 1 byte
    /// at a time.
    fn through(file: &[u8], tags: u64) -> (Vec<u8>, Option<u64>) {
        through_junk(file, tags, 0)
    }

    /// [`through`] of `file`, which starts with `junk` bytes before the MP4.
    fn through_junk(file: &[u8], tags: u64, junk: u64) -> (Vec<u8>, Option<u64>) {
        let source = Box::new(ReadOnlySource::new(OneByte(Cursor::new(file.to_owned()))));
        let mut stream = TagStream::new(source, Mp4::behind(tags, junk), junk);
        let mut out = Vec::new();
        stream.read_to_end(&mut out).unwrap();
        (out, stream.rule().first_chunk)
    }

    /// Findings 14-3, 14-20 and 14-23 of review round 14: behind 100 bytes
    /// of tags, with its first chunk at the start of the body of `mdat`
    /// counted from the file start, a `free` box of 100 bytes comes just
    /// before `mdat`, with reads of 1 byte.  An `stco` and a `co64` give
    /// the same, and a `co64` past 4 GiB gives its 64 bits.
    #[test]
    fn the_free_box_comes_just_before_the_first_mdat() {
        let tags = 100;
        for co64 in [false, true] {
            let probe = head_of(0, co64);
            let mdat = probe.len() - 16;
            let file = head_of(tags + mdat as u64 + 8, co64);
            let (out, first) = through(&file, tags);
            assert_eq!(
                first,
                Some(tags + mdat as u64 + 8),
                "co64 {co64}: the first chunk"
            );
            let free = [
                &(tags as u32).to_be_bytes()[..],
                b"free",
                &vec![0; tags as usize - 8],
            ]
            .concat();
            let want = [&file[..mdat], &free, &file[mdat..]].concat();
            assert!(out == want, "co64 {co64}: the bytes");
            let (out, _) = through(&head_of(mdat as u64 + 8, co64), tags);
            assert!(
                out == head_of(mdat as u64 + 8, co64),
                "co64 {co64}: from the MP4 start, no box"
            );
        }
        let past = (1_u64 << 32) + 8;
        let (_, first) = through(&head_of(past, true), tags);
        assert_eq!(first, Some(past), "a co64 past 4 GiB");
    }

    /// Finding 15-1 and missed mutant 15-13 of review round 15: behind 4
    /// bytes of junk, with the first chunk at the start of the body of
    /// `mdat` from the file start, 4 bytes are missing: fewer than a box.
    /// So the `mdat` grows by 4 bytes of zeros after its header, with a
    /// size of 32 bits, of 64 bits, and of 0, which stays 0.  The growth
    /// keeps the boxes after `mdat` where a stream that skips `mdat` by its
    /// size looks for them.
    #[test]
    fn fewer_than_8_missing_bytes_grow_the_first_mdat() {
        let junk = 4_u64;
        let body = b"beachape";
        let large = [
            &1_u32.to_be_bytes()[..],
            b"mdat",
            &24_u64.to_be_bytes(),
            body,
        ]
        .concat();
        let open = [&0_u32.to_be_bytes()[..], b"mdat", body].concat();
        for (case, mdat, head, grown) in [
            ("32 bits", bx(b"mdat", &[body]), 8, Some((0..4, 20_u64))),
            ("64 bits", large, 16, Some((8..16, 28))),
            ("0", open, 8, None),
        ] {
            let probe = head_with(0, false, &mdat);
            let at = probe.len() - mdat.len();
            let first = junk + (at + head) as u64;
            let file = [vec![b'j'; junk as usize], head_with(first, false, &mdat)].concat();
            let (out, _) = through_junk(&file, 0, junk);
            let mdat_at = junk as usize + at;
            let mut header = Vec::from(&file[mdat_at..mdat_at + head]);
            if let Some((place, size)) = grown {
                let bytes = size.to_be_bytes();
                header[place.clone()].copy_from_slice(&bytes[8 - place.len()..]);
            }
            let want = [&file[..mdat_at], &header, &[0; 4], &file[mdat_at + head..]].concat();
            assert!(out == want, "a size of {case}: {out:?}");
        }
    }

    /// Missed mutant 15-10 of review round 15: an `mdat` with an empty body,
    /// behind 100 bytes of tags, with the first chunk at its body from the
    /// file start, gets the `free` box of 100 bytes.
    #[test]
    fn an_empty_mdat_behind_tags_gets_the_free_box() {
        let tags = 100;
        let mdat = bx(b"mdat", &[]);
        let at = head_with(0, false, &mdat).len() - 8;
        let file = head_with(tags + at as u64 + 8, false, &mdat);
        let (out, _) = through(&file, tags);
        let free = [
            &(tags as u32).to_be_bytes()[..],
            b"free",
            &vec![0; tags as usize - 8],
        ]
        .concat();
        assert!(
            out == [&file[..at], &free, &file[at..]].concat(),
            "the free box"
        );
    }

    /// Missed mutant 15-12 of review round 15: a first box of size 0 at the
    /// start of the stream passes with no panic.
    #[test]
    fn a_first_box_of_size_0_passes() {
        let file = [&[0_u8; 4][..], b"ftyp", b"M4A ", &[0; 4]].concat();
        let (out, _) = through(&file, 0);
        assert!(out == file, "the bytes");
    }
}
