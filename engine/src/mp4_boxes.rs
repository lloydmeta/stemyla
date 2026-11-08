//! A reader of the few MP4 boxes that the decode needs (M2 of
//! `plans/active/m4a.md`): the handler and the sample entry of each track,
//! the times of the first sound track, and the `iTunSMPB` tag.  The `m4a`
//! crate writes MP4 boxes; this module reads them.
//!
//! One walker reads from a [`ReadAt`]: the head of a file, which can be cut
//! off, or a whole source that can seek.  It reads only the boxes that it
//! needs, at most [`MOST_BOX_BYTES`] of each, and skips the others by their
//! size.  So it never reads `mdat`, `stsz`, `stco` or cover art, and a
//! crafted file cannot make it hold more (MEM-4).  It keeps the values of
//! one track: the first sound track.  Of the others it keeps two flags.  A
//! box of a size that does not fit ends the walk with what it found.  It
//! never panics, also where `usize` has 32 bits.
//!
//! Symphonia's MP4 reader reads `moov/udta`, each top-level `meta` and each
//! `moov` whole, cover art too, and keeps an entry for each top-level
//! `sidx`.  So the walker plans to hide each of them from Symphonia
//! (`engine::hidden`).  The rest of the first `moov` that
//! Symphonia holds, its sample tables, has a bound (`most_moov_bytes`).

use std::io::{Read, Seek, SeekFrom};

/// The most bytes that the walker reads of one box.  An `elst` larger than
/// this counts as no `elst`.
pub(crate) const MOST_BOX_BYTES: usize = 4 * 1024;

/// The deepest nesting of the boxes that the walker reads:
/// `moov/trak/mdia/minf/stbl/stsd`.
const MOST_DEPTH: usize = 6;

/// What reads bytes at an offset.
pub(crate) trait ReadAt {
    /// Fills `buf` from `at`, or gives the bytes up to the end.  An error
    /// is a failed read, not the end.
    fn read_at(&mut self, at: u64, buf: &mut [u8]) -> std::io::Result<usize>;
}

impl ReadAt for &[u8] {
    fn read_at(&mut self, at: u64, buf: &mut [u8]) -> std::io::Result<usize> {
        let Some(rest) = usize::try_from(at).ok().and_then(|at| self.get(at..)) else {
            return Ok(0);
        };
        let count = rest.len().min(buf.len());
        buf[..count].copy_from_slice(&rest[..count]);
        Ok(count)
    }
}

/// A source that reads with a seek, as the decode does.
pub(crate) struct Seeking<'a, S: Read + Seek + ?Sized>(pub &'a mut S);

impl<S: Read + Seek + ?Sized> ReadAt for Seeking<'_, S> {
    fn read_at(&mut self, at: u64, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.seek(SeekFrom::Start(at))?;
        let mut filled = 0;
        while filled < buf.len() {
            match self.0.read(&mut buf[filled..]) {
                Ok(0) => break,
                Ok(count) => filled += count,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(filled)
    }
}

/// One `elst` entry, in the timescales of its file: `segment_duration` in
/// the movie timescale, `media_time` in the media timescale, and `rate` as
/// 16.16 fixed point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EditEntry {
    pub segment_duration: u64,
    pub media_time: i64,
    pub rate: i32,
}

/// The values of the first sound track.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SoundTrack {
    /// The `track_ID` of its `tkhd`.
    pub track_id: Option<u32>,
    /// The type of its first sample entry, such as `mp4a` or `alac`.
    pub entry: Option<[u8; 4]>,
    /// The timescale of its `mdhd`.
    pub media_timescale: Option<u32>,
    /// The duration of its `mdhd`, in the media timescale.
    pub duration: Option<u64>,
    /// The entries of its `elst`.  `None` when it has no `elst`, or one
    /// larger than [`MOST_BOX_BYTES`].
    pub edits: Option<Vec<EditEntry>>,
}

/// What the walker found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Found {
    /// A track whose handler is `vide`.
    pub video: bool,
    /// A sound track whose sample entry is `drms`, `drmi` or `enca`.
    pub protected: bool,
    /// The timescale of `mvhd`.
    pub movie_timescale: Option<u32>,
    /// The first sound track.
    pub sound: Option<SoundTrack>,
    /// The text of the `iTunSMPB` tag of `moov/udta/meta/ilst`.
    pub itunsmpb: Option<String>,
    /// Where the first `moov` is.
    pub moov: Option<MoovAt>,
    /// The bytes of the children of the first `moov` that Symphonia reads
    /// and holds: `mvhd`, each `trak` and `mvex`.
    pub moov_held: u64,
    /// The start of such a child that runs to the end of the file.
    pub moov_held_to_end_from: Option<u64>,
    /// The boxes that the decode hides from Symphonia: each `moov/udta`,
    /// each top-level `meta` and `sidx`, and each `moov` and `ftyp` after
    /// the first.
    pub hidden: crate::hidden::Plan,
    /// The size of the first `ftyp`, which Symphonia needs and reads whole
    /// (`u64::MAX` when it runs to the end of the file).
    pub ftyp_bytes: Option<u64>,
}

/// Where a `moov` box is: its start, and its end, or `None` when it runs
/// to the end of the file (a size of 0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MoovAt {
    pub start: u64,
    pub end: Option<u64>,
}

/// The sample entries of a protected sound track.
const PROTECTED_ENTRIES: [&[u8; 4]; 3] = [b"drms", b"drmi", b"enca"];

/// The box types that can start an MP4 file.  Symphonia's MP4 reader
/// reads the top-level boxes in order from the first one.
pub(crate) const FIRST_BOXES: [&[u8; 4]; 7] = [
    b"ftyp", b"moov", b"free", b"wide", b"skip", b"mdat", b"uuid",
];

/// One box: its type, its start, and its body from `body` to `end`.
#[derive(Clone, Copy, Debug)]
struct BoxAt {
    kind: [u8; 4],
    start: u64,
    body: u64,
    end: u64,
}

/// Walks the boxes of `source`.  `None` when the file does not start with
/// an MP4 box, such as `ftyp` or `moov`; then it read only 8 bytes.
pub(crate) fn walk(source: &mut dyn ReadAt) -> std::io::Result<Option<Found>> {
    walk_from(source, 0)
}

/// Walks the boxes of `source` from `start`, as [`walk`] does from 0.
pub(crate) fn walk_from(source: &mut dyn ReadAt, start: u64) -> std::io::Result<Option<Found>> {
    let mut first = [0; 8];
    if source.read_at(start, &mut first)? < 8 || !FIRST_BOXES.iter().any(|k| first[4..] == k[..]) {
        return Ok(None);
    }
    let mut walker = Walker {
        source,
        found: Found::default(),
    };
    walker.top(start)?;
    Ok(Some(walker.found))
}

/// The size of `found`, or `None` when it runs to the end of the file.
fn size_of(found: BoxAt) -> Option<u64> {
    (found.end != u64::MAX).then(|| found.end - found.start)
}

/// A big-endian `u32` at `at` of `bytes`.
fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// A big-endian `u64` at `at` of `bytes`.
fn u64_at(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_be_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
}

/// The values of one track, while the walker reads it.
#[derive(Default)]
struct Trak {
    handler: Option<[u8; 4]>,
    sound: SoundTrack,
}

struct Walker<'a> {
    source: &'a mut dyn ReadAt,
    found: Found,
}

impl Walker<'_> {
    /// The box at `at`, within its parent's end `end` (`u64::MAX` for the
    /// file).  `None` when no header fits, or its size does not fit.
    fn box_at(&mut self, at: u64, end: u64) -> std::io::Result<Option<BoxAt>> {
        let mut header = [0; 16];
        let read = self.source.read_at(at, &mut header)?;
        if read < 8 || at.checked_add(8).is_none_or(|e| e > end) {
            return Ok(None);
        }
        let size = u64::from(u32::from_be_bytes([
            header[0], header[1], header[2], header[3],
        ]));
        let kind = [header[4], header[5], header[6], header[7]];
        let (header_len, size) = match size {
            0 => (8, None),
            1 if read >= 16 => (16, u64_at(&header, 8)),
            1 => return Ok(None),
            size => (8, Some(size)),
        };
        let box_end = match size {
            // To the end of the parent, or of the file.
            None => end,
            Some(size) if size < header_len => return Ok(None),
            Some(size) => match at.checked_add(size) {
                Some(box_end) if box_end <= end => box_end,
                _ => return Ok(None),
            },
        };
        Ok(Some(BoxAt {
            kind,
            start: at,
            body: at + header_len,
            end: box_end,
        }))
    }

    /// Calls `each` with each child box from `start` to `end`, until a box
    /// does not fit or `each` says to stop.
    fn children(
        &mut self,
        start: u64,
        end: u64,
        mut each: impl FnMut(&mut Self, BoxAt) -> std::io::Result<bool>,
    ) -> std::io::Result<bool> {
        let mut at = start;
        while at < end {
            let Some(found) = self.box_at(at, end)? else {
                return Ok(false);
            };
            if !each(self, found)? {
                return Ok(false);
            }
            if found.end == u64::MAX {
                return Ok(true);
            }
            at = found.end;
        }
        Ok(true)
    }

    /// At most [`MOST_BOX_BYTES`] of the body of `at`.  `None` when the
    /// body is larger and `whole` is set.
    fn body(&mut self, at: BoxAt, whole: bool) -> std::io::Result<Option<Vec<u8>>> {
        let len = at.end.saturating_sub(at.body);
        if whole && len > MOST_BOX_BYTES as u64 {
            return Ok(None);
        }
        let mut body = vec![0; len.min(MOST_BOX_BYTES as u64) as usize];
        let read = self.source.read_at(at.body, &mut body)?;
        body.truncate(read);
        Ok(Some(body))
    }

    /// Walks the top-level boxes.  It reads the first `moov`, and reads
    /// only the header of each other box.  Symphonia reads each `moov`,
    /// `ftyp` and top-level `meta` whole, so it hides each `meta`, and each
    /// `moov` and `ftyp` after the first.  Symphonia also keeps an entry for
    /// each `sidx` of its own `reference_id` (`symphonia-format-isomp4`
    /// `demuxer.rs`, line 189), only for the duration of a fragmented file.
    /// The decode reads in order, so it hides each `sidx` too (finding 6-2
    /// of review round 6).
    fn top(&mut self, start: u64) -> std::io::Result<()> {
        self.children(start, u64::MAX, |walker, found| {
            match &found.kind {
                b"moov" if walker.found.moov.is_none() => {
                    walker.found.moov = Some(MoovAt {
                        start: found.start,
                        end: size_of(found).map(|size| found.start + size),
                    });
                    walker.moov(found)?;
                }
                b"ftyp" if walker.found.ftyp_bytes.is_none() => {
                    walker.found.ftyp_bytes = Some(size_of(found).unwrap_or(u64::MAX));
                }
                b"moov" | b"meta" | b"ftyp" | b"sidx" => walker.hide(found),
                _ => {}
            }
            Ok(true)
        })?;
        Ok(())
    }

    /// Counts `found` in the bytes of `moov` that Symphonia holds.
    fn hold(&mut self, found: BoxAt) {
        match size_of(found) {
            Some(size) => self.found.moov_held += size,
            None => self.found.moov_held_to_end_from = Some(found.start),
        }
    }

    /// Hides `found` from Symphonia: it gives `free` for its type.
    fn hide(&mut self, found: BoxAt) {
        self.found
            .hidden
            .hide(found.start + 4, b"free", size_of(found));
    }

    fn moov(&mut self, moov: BoxAt) -> std::io::Result<()> {
        self.children(moov.body, moov.end, |walker, found| {
            // Symphonia reads these children of `moov` (`atoms/moov.rs`).
            // Of a `trak`, it reads `tkhd`, `edts` and `mdia` (`trak`).
            if matches!(&found.kind, b"mvhd" | b"mvex") {
                walker.hold(found);
            }
            match &found.kind {
                b"mvhd" => {
                    let body = walker.body(found, false)?.unwrap_or_default();
                    let at = if body.first() == Some(&1) { 20 } else { 12 };
                    walker.found.movie_timescale = u32_at(&body, at);
                }
                b"trak" => {
                    let mut trak = Trak::default();
                    walker.trak(found, &mut trak, 1)?;
                    walker.keep(trak);
                }
                // Symphonia reads `udta` whole (`atoms/moov.rs`).
                b"udta" => {
                    walker.udta(found, 1)?;
                    walker.hide(found);
                }
                _ => {}
            }
            Ok(true)
        })?;
        Ok(())
    }

    /// Keeps what the walk of one track found: its flags, and its values
    /// if it is the first sound track.
    fn keep(&mut self, trak: Trak) {
        match trak.handler {
            Some([b'v', b'i', b'd', b'e']) => self.found.video = true,
            Some([b's', b'o', b'u', b'n']) => {
                if trak
                    .sound
                    .entry
                    .is_some_and(|entry| PROTECTED_ENTRIES.iter().any(|k| entry == **k))
                {
                    self.found.protected = true;
                }
                if self.found.sound.is_none() {
                    self.found.sound = Some(trak.sound);
                }
            }
            _ => {}
        }
    }

    /// Reads the boxes of a `trak` that the decode needs, at `depth`.
    fn trak(&mut self, parent: BoxAt, trak: &mut Trak, depth: usize) -> std::io::Result<()> {
        if depth >= MOST_DEPTH {
            return Ok(());
        }
        self.children(parent.body, parent.end, |walker, found| {
            if depth == 1 && matches!(&found.kind, b"tkhd" | b"edts" | b"mdia") {
                walker.hold(found);
            }
            match &found.kind {
                b"tkhd" => {
                    let body = walker.body(found, false)?.unwrap_or_default();
                    let at = if body.first() == Some(&1) { 20 } else { 12 };
                    trak.sound.track_id = u32_at(&body, at);
                }
                b"mdhd" => {
                    let body = walker.body(found, false)?.unwrap_or_default();
                    // A duration of all ones is unknown (ISO/IEC 14496-12,
                    // 8.4.2.3).
                    let (scale, duration) = if body.first() == Some(&1) {
                        (
                            u32_at(&body, 20),
                            u64_at(&body, 24).filter(|d| *d != u64::MAX),
                        )
                    } else {
                        let duration = u32_at(&body, 16).filter(|d| *d != u32::MAX);
                        (u32_at(&body, 12), duration.map(u64::from))
                    };
                    trak.sound.media_timescale = scale;
                    trak.sound.duration = duration;
                }
                // The media handler.  A QuickTime `minf` has a data handler
                // (`alis`) too, which is not the kind of the track.
                b"hdlr" if parent.kind == *b"mdia" => {
                    let body = walker.body(found, false)?.unwrap_or_default();
                    trak.handler = body.get(8..12).and_then(|k| k.try_into().ok());
                }
                b"elst" => {
                    trak.sound.edits = walker.body(found, true)?.and_then(|b| edits(&b));
                }
                b"stsd" => {
                    let body = walker.body(found, false)?.unwrap_or_default();
                    trak.sound.entry = body.get(12..16).and_then(|k| k.try_into().ok());
                }
                b"edts" | b"mdia" | b"minf" | b"stbl" => walker.trak(found, trak, depth + 1)?,
                _ => {}
            }
            Ok(true)
        })?;
        Ok(())
    }

    /// Reads `udta/meta/ilst/----` for the `iTunSMPB` tag.
    fn udta(&mut self, parent: BoxAt, depth: usize) -> std::io::Result<()> {
        if depth >= MOST_DEPTH {
            return Ok(());
        }
        self.children(parent.body, parent.end, |walker, found| {
            match &found.kind {
                b"meta" => {
                    // `meta` is a full box in ISO files and in iTunes' tags,
                    // and a plain box in some QuickTime files.
                    let mut peek = [0; 8];
                    let read = walker.source.read_at(found.body, &mut peek)?;
                    let skip = if read == 8 && peek[4..] == *b"hdlr" {
                        0
                    } else {
                        4
                    };
                    let inner = BoxAt {
                        body: found.body.saturating_add(skip).min(found.end),
                        ..found
                    };
                    walker.udta(inner, depth + 1)?;
                }
                b"ilst" => walker.udta(found, depth + 1)?,
                b"----" => walker.freeform(found)?,
                _ => {}
            }
            Ok(true)
        })?;
        Ok(())
    }

    /// Reads one `----` item, and keeps its text if it is `iTunSMPB`.
    fn freeform(&mut self, item: BoxAt) -> std::io::Result<()> {
        let (mut mean, mut name, mut data) = (None, None, None);
        self.children(item.body, item.end, |walker, found| {
            let body = walker.body(found, false)?.unwrap_or_default();
            match &found.kind {
                b"mean" => mean = body.get(4..).map(Vec::from),
                b"name" => name = body.get(4..).map(Vec::from),
                b"data" => data = body.get(8..).map(Vec::from),
                _ => {}
            }
            Ok(true)
        })?;
        if mean.as_deref() == Some(b"com.apple.iTunes")
            && name.as_deref() == Some(b"iTunSMPB")
            && let Some(text) = data.and_then(|data| String::from_utf8(data).ok())
        {
            self.found.itunsmpb = Some(text);
        }
        Ok(())
    }
}

/// The entries of an `elst` body of version 0 or 1, or `None` if its count
/// does not fit the body.
fn edits(body: &[u8]) -> Option<Vec<EditEntry>> {
    let version = *body.first()?;
    let count = u32_at(body, 4)?;
    let size: usize = if version == 1 { 20 } else { 12 };
    // In `u64`, so the size cannot wrap where `usize` has 32 bits.
    let needed = u64::from(count) * size as u64 + 8;
    if needed > body.len() as u64 {
        return None;
    }
    let count = count as usize;
    let mut entries = Vec::with_capacity(count.min(body.len() / size));
    for i in 0..count {
        let at = 8 + i * size;
        let entry = if version == 1 {
            EditEntry {
                segment_duration: u64_at(body, at)?,
                media_time: u64_at(body, at + 8)? as i64,
                rate: u32_at(body, at + 16)? as i32,
            }
        } else {
            EditEntry {
                segment_duration: u64::from(u32_at(body, at)?),
                media_time: i64::from(u32_at(body, at + 4)? as i32),
                rate: u32_at(body, at + 8)? as i32,
            }
        };
        entries.push(entry);
    }
    Some(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_alloc::peak_of;
    use crate::test_m4a::{
        aac_m4a, alac_m4a, bx, chirp, file_of, patched, top_box, version_1, with_entry,
        with_second_trak,
    };
    use m4a::{Codec, Edit, Gapless, Layout, Order, SampleEntry};

    fn walked(file: &[u8]) -> Option<Found> {
        let found = walk(&mut { file });
        assert!(found.is_ok(), "{found:?}");
        found.unwrap()
    }

    fn tone(frames: usize) -> Vec<f32> {
        chirp(0, frames, 44_100)
    }

    /// An ALAC file of 3 packets of tone, with an edit of Apple's delay.
    fn alac(order: Order) -> Vec<u8> {
        let tone = tone(3 * 4096);
        let edit = Edit {
            skip: 2112,
            frames: 10_000,
        };
        alac_m4a(&tone, &tone, 44_100, Some(edit), order)
    }

    #[test]
    fn the_walker_reads_the_values_of_the_sound_track() {
        for order in [Order::FastStart, Order::MoovLast] {
            let file = alac(order);
            let (start, size) = top_box(&file, b"moov");
            let found = walked(&file);
            assert_eq!(
                found,
                Some(Found {
                    video: false,
                    protected: false,
                    movie_timescale: Some(44_100),
                    sound: Some(SoundTrack {
                        track_id: Some(1),
                        entry: Some(*b"alac"),
                        media_timescale: Some(44_100),
                        duration: Some(3 * 4096),
                        edits: Some(vec![EditEntry {
                            segment_duration: 10_000,
                            media_time: 2112,
                            rate: 0x1_0000,
                        }]),
                    }),
                    itunsmpb: None,
                    moov: Some(MoovAt {
                        start: start as u64,
                        end: Some((start + size) as u64),
                    }),
                    // `mvhd`, and `tkhd`, `edts` and `mdia` of the one
                    // `trak`: all of `moov` but the headers of `moov` and
                    // `trak`.
                    moov_held: size as u64 - 8 - 8,
                    moov_held_to_end_from: None,
                    hidden: crate::hidden::Plan::default(),
                    ftyp_bytes: Some(28),
                }),
                "{order:?}"
            );
        }
    }

    #[test]
    fn the_walker_reads_the_itunsmpb_tag() {
        let tone = tone(4096);
        let channels = [tone.clone(), tone];
        let frames = crate::test_m4a::aac_track_frames(4096);
        let gapless = Gapless {
            delay: 1024,
            padding: (frames - 1024 - 4096) as u32,
            frames: 4096,
        };
        for order in [Order::FastStart, Order::MoovLast] {
            let file = aac_m4a(&channels, 44_100, None, Some(gapless), order);
            let found = walked(&file).expect("an MP4 file");
            assert_eq!(
                found.itunsmpb.as_deref(),
                Some(
                    " 00000000 00000400 00000000 0000000000001000 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000"
                ),
                "{order:?}"
            );
            let sound = found.sound.expect("a sound track");
            assert_eq!(sound.entry, Some(*b"mp4a"));
            assert_eq!(sound.edits, None);
        }
    }

    #[test]
    fn a_file_of_another_format_reads_8_bytes() {
        let wav = crate::wav::encode_pcm16(&[0.0; 10], &[0.0; 10], 44_100);
        assert_eq!(walked(&wav), None);
        assert_eq!(walked(b"ID3"), None);
        assert_eq!(walked(&[]), None);
    }

    #[test]
    fn the_walker_finds_a_video_track_and_a_protected_entry() {
        let single = patched(alac(Order::FastStart), b"soun", b"vide");
        assert!(walked(&single).unwrap().video);
        assert_eq!(walked(&single).unwrap().sound, None);
        let second = with_second_trak(alac(Order::MoovLast), b"vide", 2);
        let found = walked(&second).unwrap();
        assert!(found.video);
        assert_eq!(found.sound.and_then(|s| s.track_id), Some(1));
        for kind in [b"drms", b"drmi", b"enca"] {
            let found = walked(&with_entry(alac(Order::FastStart), kind)).unwrap();
            assert!(found.protected, "{kind:?}");
            assert!(!found.video, "{kind:?}");
        }
    }

    /// The walker keeps the first sound track and its `track_ID`.
    #[test]
    fn the_walker_keeps_the_first_sound_track() {
        let file = with_second_trak(alac(Order::MoovLast), b"soun", 7);
        let found = walked(&file).unwrap();
        assert_eq!(found.sound.and_then(|s| s.track_id), Some(1));
        let file = with_second_trak(with_entry(alac(Order::MoovLast), b"lpcm"), b"soun", 7);
        assert_eq!(
            walked(&file).unwrap().sound.and_then(|s| s.entry),
            Some(*b"lpcm")
        );
    }

    #[test]
    fn the_walker_reads_version_1_boxes() {
        let found = walked(&version_1()).unwrap();
        assert_eq!(found.movie_timescale, Some(1000));
        assert_eq!(
            found.sound,
            Some(SoundTrack {
                track_id: Some(9),
                entry: Some(*b"mp4a"),
                media_timescale: Some(48_000),
                duration: Some(3 << 32),
                edits: Some(vec![EditEntry {
                    segment_duration: 5 << 32,
                    media_time: 1024 << 32,
                    rate: 0x1_0000,
                }]),
            })
        );
    }

    /// An `elst` of more than 4 KiB counts as none, and a negative
    /// `media_time` of version 0 keeps its sign.
    #[test]
    fn an_elst_past_4_kib_counts_as_none() {
        let full = [0_u8; 4];
        let entry = [
            &5_u32.to_be_bytes()[..],
            &(-1_i32).to_be_bytes(),
            &0x1_0000_u32.to_be_bytes(),
        ]
        .concat();
        let elst = |count: usize| {
            let entries = entry.repeat(count);
            bx(b"elst", &[&full, &(count as u32).to_be_bytes(), &entries])
        };
        let file = |elst: Vec<u8>| {
            let hdlr = bx(b"hdlr", &[&full, &[0; 4], b"soun"]);
            bx(
                b"moov",
                &[&bx(
                    b"trak",
                    &[&bx(b"edts", &[&elst]), &bx(b"mdia", &[&hdlr])],
                )],
            )
        };
        let edits = |elst: Vec<u8>| {
            let sound = walked(&file(elst)).and_then(|found| found.sound);
            assert!(sound.is_some(), "a sound track");
            sound.unwrap().edits
        };
        // 8 + 340 entries of 12 bytes is 4088 bytes of body.
        let most = edits(elst(340));
        assert_eq!(most.as_ref().map(Vec::len), Some(340));
        assert_eq!(most.map(|m| m[0].media_time), Some(-1));
        assert_eq!(edits(elst(341)), None);
        // A count that the body does not hold.
        let mut short = elst(2);
        short[12..16].copy_from_slice(&3_u32.to_be_bytes());
        assert_eq!(edits(short), None);
    }

    /// The walker's memory does not grow with the count of tracks.
    #[test]
    fn a_moov_of_10000_tracks_keeps_the_values_of_one() {
        let mut few = alac(Order::MoovLast);
        for id in 2..10 {
            few = with_second_trak(few, b"soun", id);
        }
        let mut many = few.clone();
        for id in 10..10_000 {
            many = with_second_trak(many, if id % 2 == 0 { b"soun" } else { b"vide" }, id);
        }
        let (few_peak, few_found) = peak_of(|| walked(&few));
        let (many_peak, many_found) = peak_of(|| walked(&many));
        assert_eq!(few_found.unwrap().sound.and_then(|s| s.track_id), Some(1));
        let many_found = many_found.unwrap();
        assert_eq!(many_found.sound.and_then(|s| s.track_id), Some(1));
        assert!(many_found.video);
        assert!(
            many_peak <= few_peak + 1024,
            "10 tracks peak at {few_peak} bytes, 10,000 at {many_peak}"
        );
    }

    /// Counts the bytes that it reads.
    struct Counting {
        file: std::io::Cursor<Vec<u8>>,
        read: u64,
    }

    impl Read for Counting {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let count = self.file.read(buf)?;
            self.read += count as u64;
            Ok(count)
        }
    }

    impl Seek for Counting {
        fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
            self.file.seek(to)
        }
    }

    /// On a file of `moov` after 1 MiB of `mdat`, the walker seeks past
    /// `mdat` to read `moov`.
    #[test]
    fn the_walker_seeks_past_mdat_to_a_moov_last() {
        let layout = Layout::new(Codec::Alac, 64 * 4096, 44_100, None, Order::MoovLast).unwrap();
        let packets = vec![vec![7_u8; 16_384]; 64];
        let entry = SampleEntry::Alac { cookie: [0; 24] };
        let file = file_of(&layout, &entry, &packets);
        let (_, moov_size) = top_box(&file, b"moov");
        let mut source = Counting {
            file: std::io::Cursor::new(file),
            read: 0,
        };
        let found = walk(&mut Seeking(&mut source));
        assert!(found.is_ok(), "{found:?}");
        let sound = found.unwrap().and_then(|f| f.sound);
        assert_eq!(sound.and_then(|s| s.duration), Some(64 * 4096));
        assert!(
            source.read < 64 * 1024 + moov_size as u64,
            "{} bytes read, moov has {moov_size}",
            source.read
        );
    }

    /// The fixtures of the walker's tests that hold each box it reads.
    fn fixtures() -> Vec<Vec<u8>> {
        let tone = tone(1024);
        let channels = [tone.clone(), tone];
        let frames = crate::test_m4a::aac_track_frames(1024);
        let gapless = Gapless {
            delay: 1024,
            padding: (frames - 2048) as u32,
            frames: 1024,
        };
        let edit = Some(Edit {
            skip: 1024,
            frames: 1024,
        });
        vec![
            aac_m4a(&channels, 44_100, edit, Some(gapless), Order::FastStart),
            aac_m4a(&channels, 44_100, edit, Some(gapless), Order::MoovLast),
            version_1(),
        ]
    }

    /// The walker ends with no panic on every prefix of a fixture, and on
    /// each box with a size of 0, 1, 7 or `u32::MAX`.
    #[test]
    fn the_walker_ends_on_any_prefix_or_box_size() {
        const KINDS: [&[u8; 4]; 19] = [
            b"ftyp", b"moov", b"mvhd", b"trak", b"tkhd", b"edts", b"elst", b"mdia", b"mdhd",
            b"hdlr", b"minf", b"stbl", b"stsd", b"mdat", b"udta", b"meta", b"ilst", b"----",
            b"data",
        ];
        for (k, file) in fixtures().iter().enumerate() {
            for end in 0..=file.len() {
                walked(&file[..end]);
            }
            let mut sizes = 0;
            for at in 4..file.len().saturating_sub(4) {
                if !KINDS.iter().any(|kind| file[at..at + 4] == kind[..]) {
                    continue;
                }
                for size in [0, 1, 7, u32::MAX] {
                    let mut bad = file.clone();
                    bad[at - 4..at].copy_from_slice(&size.to_be_bytes());
                    walked(&bad);
                    sizes += 1;
                }
            }
            assert!(sizes >= 4 * 12, "fixture {k}: {sizes} sizes");
            // Finding 1-5 of review round 1: the entry count of each `elst`
            // too, with the counts whose size overflows 32 bits.
            let mut counts = 0;
            for at in 4..file.len().saturating_sub(12) {
                if file[at..at + 4] != *b"elst" {
                    continue;
                }
                for count in [0, 1, 357_913_941, 214_748_365, u32::MAX] {
                    let mut bad = file.clone();
                    bad[at + 8..at + 12].copy_from_slice(&count.to_be_bytes());
                    walked(&bad);
                    counts += 1;
                }
            }
            assert!(counts >= 5, "fixture {k}: {counts} counts");
        }
    }

    /// An `elst` body of version `version`, with `count` in its header and
    /// `held` entries of zeros.
    fn elst_body(version: u8, count: u32, held: usize) -> Vec<u8> {
        let size = if version == 1 { 20 } else { 12 };
        let mut body = vec![version, 0, 0, 0];
        body.extend_from_slice(&count.to_be_bytes());
        body.resize(8 + held * size, 0);
        body
    }

    /// Finding 1-1 of review round 1: a count whose entries pass 2^32
    /// bytes counts as no `elst`, also where `usize` has 32 bits.  The
    /// browser test `an_elst_count_past_32_bits_ends_the_head_check` of
    /// `decode-worker` checks it on wasm32.
    #[test]
    fn an_elst_count_past_32_bits_counts_as_none() {
        for (version, count) in [
            (0, 357_913_941),
            (0, 357_913_942),
            (0, u32::MAX),
            (1, 214_748_365),
            (1, u32::MAX),
        ] {
            assert_eq!(
                edits(&elst_body(version, count, 2)),
                None,
                "{version}, {count}"
            );
        }
        assert_eq!(edits(&elst_body(0, 3, 2)), None, "a count past the body");
        assert_eq!(edits(&elst_body(0, 2, 2)).map(|e| e.len()), Some(2));
        assert_eq!(edits(&elst_body(1, 2, 2)).map(|e| e.len()), Some(2));
        assert_eq!(edits(&elst_body(1, 0, 0)), Some(Vec::new()));
    }

    const FULL: [u8; 4] = [0; 4];

    /// An `hdlr` of `pre_defined` and the handler type `handler`.
    fn hdlr(pre_defined: &[u8; 4], handler: &[u8; 4]) -> Vec<u8> {
        bx(b"hdlr", &[&FULL, pre_defined, handler, &[0; 12], &[0]])
    }

    /// A QuickTime track: the media handler in `mdia`, then a data handler
    /// (`dhlr`, `alis`) in `minf`.
    fn quicktime_trak(handler: &[u8; 4]) -> Vec<u8> {
        let mdhd = bx(
            b"mdhd",
            &[
                &FULL,
                &[0; 8],
                &44_100_u32.to_be_bytes(),
                &4096_u32.to_be_bytes(),
                &[0; 4],
            ],
        );
        let entry = bx(b"mp4a", &[&[0; 28]]);
        let stsd = bx(b"stsd", &[&FULL, &1_u32.to_be_bytes(), &entry]);
        let minf = bx(b"minf", &[&hdlr(b"dhlr", b"alis"), &bx(b"stbl", &[&stsd])]);
        let mdia = bx(b"mdia", &[&mdhd, &hdlr(b"mhlr", handler), &minf]);
        bx(b"trak", &[&mdia])
    }

    /// Finding 1-2 of review round 1: the data handler of a QuickTime
    /// `minf` does not hide the media handler of `mdia`.
    #[test]
    fn a_data_handler_in_minf_does_not_hide_the_media_handler() {
        let sound = walked(&bx(b"moov", &[&quicktime_trak(b"soun")])).unwrap();
        assert!(!sound.video);
        assert!(sound.sound.is_some(), "a sound track");
        let track = sound.sound.unwrap();
        assert_eq!((track.entry, track.duration), (Some(*b"mp4a"), Some(4096)));
        let video = walked(&bx(b"moov", &[&quicktime_trak(b"vide")])).unwrap();
        assert!(video.video);
        assert_eq!(video.sound, None);
    }

    /// Finding 1-3 of review round 1: an `mdhd` duration of all ones is
    /// unknown (ISO/IEC 14496-12, 8.4.2.3), for each version.
    #[test]
    fn an_mdhd_duration_of_all_ones_is_unknown() {
        let mdhd = bx(
            b"mdhd",
            &[
                &FULL,
                &[0; 8],
                &44_100_u32.to_be_bytes(),
                &u32::MAX.to_be_bytes(),
                &[0; 4],
            ],
        );
        let file = bx(
            b"moov",
            &[&bx(
                b"trak",
                &[&bx(b"mdia", &[&mdhd, &hdlr(b"\0\0\0\0", b"soun")])],
            )],
        );
        let sound = walked(&file).unwrap().sound.expect("a sound track");
        assert_eq!(
            (sound.media_timescale, sound.duration),
            (Some(44_100), None)
        );
        let mut v1 = version_1();
        let at = crate::test_m4a::place_once(&v1, b"mdhd") + 4 + 24;
        v1[at..at + 8].copy_from_slice(&u64::MAX.to_be_bytes());
        let sound = walked(&v1).unwrap().sound.expect("a sound track");
        assert_eq!(
            (sound.media_timescale, sound.duration),
            (Some(48_000), None)
        );
    }

    /// Finding 1-19 of review round 1: a box of a 64-bit size of 1 GiB, in
    /// a `moov` of size 0, is read 4 KiB at most, from a slice and from a
    /// source that seeks.
    #[test]
    fn a_box_of_1_gib_is_read_4_kib_at_most() {
        let large = |kind: &[u8; 4], body: &[u8]| {
            let mut out = Vec::from(1_u32.to_be_bytes());
            out.extend_from_slice(kind);
            out.extend_from_slice(&(1_u64 << 30).to_be_bytes());
            out.extend_from_slice(body);
            out
        };
        let size_0 = |kind: &[u8; 4], body: &[u8]| {
            let mut out = Vec::from(0_u32.to_be_bytes());
            out.extend_from_slice(kind);
            out.extend_from_slice(body);
            out
        };
        let mvhd = large(
            b"mvhd",
            &[&FULL[..], &[0; 8], &1000_u32.to_be_bytes(), &[0; 8]].concat(),
        );
        let item = size_0(
            b"----",
            &[
                bx(b"mean", &[&FULL, b"com.apple.iTunes"]),
                bx(b"name", &[&FULL, b"iTunSMPB"]),
                large(b"data", &[&[0, 0, 0, 1, 0, 0, 0, 0][..], b"lloyd"].concat()),
            ]
            .concat(),
        );
        let meta = size_0(b"meta", &[&FULL[..], &size_0(b"ilst", &item)].concat());
        for file in [
            size_0(b"moov", &mvhd),
            size_0(b"moov", &size_0(b"udta", &meta)),
        ] {
            let (peak, found) = peak_of(|| walked(&file));
            assert!(peak < 8 * 1024, "{peak} bytes from a slice");
            let mut source = std::io::Cursor::new(file.clone());
            let (peak, seeking) = peak_of(|| walk(&mut Seeking(&mut source)).ok().flatten());
            assert!(peak < 8 * 1024, "{peak} bytes from a source");
            assert_eq!(found, seeking);
            let found = found.expect("an MP4 file");
            assert!(
                found.movie_timescale == Some(1000) || found.itunsmpb.as_deref() == Some("lloyd"),
                "{found:?}"
            );
        }
    }

    /// A `----` item of the `iTunSMPB` text "lloyd".
    fn freeform() -> Vec<u8> {
        bx(
            b"----",
            &[
                &bx(b"mean", &[&FULL, b"com.apple.iTunes"]),
                &bx(b"name", &[&FULL, b"iTunSMPB"]),
                &bx(b"data", &[&[0, 0, 0, 1, 0, 0, 0, 0], b"lloyd"]),
            ],
        )
    }

    /// Missed mutants 1-21 and 1-24 of review round 1: a `meta` in a
    /// `meta`, and a QuickTime `meta` with no version and flags.
    #[test]
    fn the_tag_is_read_from_a_nested_or_a_plain_meta() {
        let ilst = bx(b"ilst", &[&freeform()]);
        let inner = bx(b"meta", &[&FULL, &ilst]);
        let nested = bx(b"moov", &[&bx(b"udta", &[&bx(b"meta", &[&FULL, &inner])])]);
        assert_eq!(walked(&nested).unwrap().itunsmpb.as_deref(), Some("lloyd"));
        let plain_meta = bx(b"meta", &[&hdlr(b"\0\0\0\0", b"mdir"), &ilst]);
        let plain = bx(b"moov", &[&bx(b"udta", &[&plain_meta])]);
        assert_eq!(walked(&plain).unwrap().itunsmpb.as_deref(), Some("lloyd"));
    }

    fn mvhd(scale: u32) -> Vec<u8> {
        bx(b"mvhd", &[&FULL, &[0; 8], &scale.to_be_bytes(), &[0; 80]])
    }

    /// Missed mutant 1-22 of review round 1: a box of a 64-bit size whose
    /// header is cut at the end of the head ends the walk, and keeps what
    /// the walk found.
    #[test]
    fn a_box_header_cut_in_its_64_bit_size_ends_the_walk() {
        let mut file = Vec::from(0_u32.to_be_bytes());
        file.extend_from_slice(b"moov");
        file.extend_from_slice(&mvhd(44_100));
        file.extend_from_slice(&1_u32.to_be_bytes());
        file.extend_from_slice(b"mvhd");
        file.extend_from_slice(&[0, 0, 0, 1]);
        assert_eq!(walked(&file).unwrap().movie_timescale, Some(44_100));
    }

    /// Missed mutant 1-23 of review round 1: an `elst` body of exactly
    /// 4096 bytes is read.
    #[test]
    fn an_elst_body_of_4096_bytes_is_read() {
        let entry = [
            &5_u32.to_be_bytes()[..],
            &0_u32.to_be_bytes(),
            &0x1_0000_u32.to_be_bytes(),
        ]
        .concat();
        let elst = bx(
            b"elst",
            &[&FULL, &340_u32.to_be_bytes(), &entry.repeat(340), &[0; 8]],
        );
        assert_eq!(elst.len() - 8, MOST_BOX_BYTES);
        let mdia = bx(b"mdia", &[&hdlr(b"\0\0\0\0", b"soun")]);
        let file = bx(b"moov", &[&bx(b"trak", &[&bx(b"edts", &[&elst]), &mdia])]);
        let edits = walked(&file).unwrap().sound.expect("a sound track").edits;
        assert_eq!(edits.map(|e| e.len()), Some(340));
    }

    /// Missed mutant 1-26 of review round 1: an `hdlr` deeper than
    /// `MOST_DEPTH` is not read.
    #[test]
    fn a_box_deeper_than_the_depth_limit_is_not_read() {
        let mut nested = hdlr(b"\0\0\0\0", b"soun");
        for _ in 0..5 {
            nested = bx(b"mdia", &[&nested]);
        }
        let deep = bx(b"moov", &[&bx(b"trak", &[&nested])]);
        assert_eq!(walked(&deep).unwrap().sound, None);
        let mut shallow = hdlr(b"\0\0\0\0", b"soun");
        for _ in 0..4 {
            shallow = bx(b"mdia", &[&shallow]);
        }
        let file = bx(b"moov", &[&bx(b"trak", &[&shallow])]);
        assert!(walked(&file).unwrap().sound.is_some(), "4 mdia boxes deep");
    }
}
