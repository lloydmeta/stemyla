//! Hides the tags and the pictures of a file from Symphonia (MEM-4).
//!
//! Symphonia 0.6.1 reads tags and pictures whole, with no limit.  The app
//! reads none of them.  So the decode shows each box, block or chunk of
//! tags to Symphonia as one that Symphonia skips, and Symphonia skips a
//! large one with a seek (`MediaSourceStream::ignore_bytes`).  The bytes
//! keep their offsets, so the offsets in the file stay true.  A
//! [`Plan`] holds the changes, and [`Hidden`] gives them to each read.
//!
//! The walk of an MP4 plans its boxes (`engine::mp4_boxes`).  [`plan_of`]
//! plans the other formats.  The view starts after any leading ID3v2 tags.
//! FLAC PICTURE, VORBIS_COMMENT, APPLICATION and SEEKTABLE blocks show as
//! PADDING.
//! A WAV `LIST` chunk, and AIFF `MARK`, `APPL`, `COMT`, text and `ID3 `
//! chunks, show as `JUNK`.

use std::io::{Read, Seek, SeekFrom};

use symphonia::core::io::MediaSource;

use crate::mp4_boxes::ReadAt;

/// A container of tags of at least this many bytes is always hidden.
pub(crate) const ALWAYS_HIDDEN: u64 = 4096;

/// The most containers of fewer than [`ALWAYS_HIDDEN`] bytes that a plan
/// hides.  So the plan holds at most about 64 KiB for them.  Symphonia
/// reads and keeps each small one past this count ([`Plan::kept`]).  No
/// real file has thousands of tag containers; one has tens.
pub(crate) const MOST_SMALL: usize = 4096;

/// The fewest bytes of one entry (one tag) of any container of tags.  A
/// FLAC VORBIS_COMMENT holds a length of 4 bytes for each comment; each
/// other form needs more.  So a container of `size` bytes holds at most
/// `size / MIN_ENTRY_BYTES` entries ([`kept_charge`]).
pub(crate) const MIN_ENTRY_BYTES: u64 = 4;

/// The bytes that the decode counts for each entry that a small container
/// past [`MOST_SMALL`] can hold ([`kept_charge`]).  What Symphonia keeps
/// grows with the entries of a container, not with the container
/// (finding 5-1 of review round 5).  Measured with `peak_of` on
/// 2026-11-09, with no container hidden, as the worst over counts of 1 to
/// 257 containers (100,000 for `NAME`), for each entry that the charge
/// counts:
///
/// * a FLAC VORBIS_COMMENT of 4,012 bytes of 800 "=" comments: 90,512 to
///   167,387 bytes for each container, so 163 for each entry past its
///   bytes (Symphonia's lists grow by doubling);
/// * a WAV `LIST`/`INFO` of 4,012 bytes of 400 subchunks of 2 bytes:
///   55,123, so 51;
/// * a top-level M4A `meta` of 3,803 bytes of 150 `ilst` items: 25,780,
///   so 23;
/// * an AIFF `NAME` chunk of 10 bytes: 268, so 129;
/// * an empty WAV `LIST` of 12 bytes: 274, so 87;
/// * an AIFF keeps one `COMT` and the last `ID3 ` chunk only, and refuses
///   a second `COMT`.
///
/// The count takes 256, more than 1.5 times the worst.  A test holds the
/// peak of each form at the limit under [`MOST_KEPT_BYTES`].
pub(crate) const KEPT_OVERHEAD: u64 = 256;

/// The bytes that the decode counts for a small container of `size` bytes
/// that Symphonia reads: its bytes, and [`KEPT_OVERHEAD`] for each entry
/// that it can hold, one for each [`MIN_ENTRY_BYTES`], at least one.
pub(crate) fn kept_charge(size: u64) -> u64 {
    let entries = (size / MIN_ENTRY_BYTES).max(1);
    size.saturating_add(entries.saturating_mul(KEPT_OVERHEAD))
}

/// The most bytes of tags that Symphonia may keep, counted as the
/// [`kept_charge`] of each small container past [`MOST_SMALL`]: 16 MiB.
/// It is the room for tags that the page gives each file
/// (`engine::decode::TAG_ROOM`), far below the 1 GiB of samples of decision
/// 0013 and the 2 GB of a tab (MEM-4).  A file past it is refused before
/// Symphonia opens it ([`Plan::too_much_kept`]).  So a file of 5000 small
/// tags opens.  A file of 4,200 dense VORBIS_COMMENT blocks of 4 KiB, which
/// would keep about 500 MB, does not.
pub(crate) const MOST_KEPT_BYTES: u64 = 16 << 20;

/// One change: `len` bytes of `bytes` in place of the bytes at `at` of the
/// file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Patch {
    pub at: u64,
    pub bytes: [u8; 4],
    pub len: u8,
}

impl Patch {
    /// At most 4 bytes of `bytes` at `at`.
    fn of(at: u64, bytes: &[u8]) -> Patch {
        let mut patch = Patch {
            at,
            bytes: [0; 4],
            len: bytes.len().min(4) as u8,
        };
        patch.bytes[..usize::from(patch.len)].copy_from_slice(&bytes[..usize::from(patch.len)]);
        patch
    }

    /// Puts the bytes of the patch that fall in `from..from + buf.len()`
    /// in `buf`.
    fn apply(&self, from: u64, buf: &mut [u8]) {
        let to = from + buf.len() as u64;
        for k in 0..u64::from(self.len) {
            let at = self.at + k;
            if (from..to).contains(&at) {
                buf[(at - from) as usize] = self.bytes[k as usize];
            }
        }
    }
}

/// The changes to a file: where Symphonia's view of it starts, and each
/// [`Patch`], at an offset of the file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Plan {
    pub start: u64,
    pub patches: Vec<Patch>,
    /// The patches that the view shows only after a seek to its start,
    /// over the other patches.
    pub late: Vec<Patch>,
    small: usize,
    /// The bytes that Symphonia keeps of the small containers that the
    /// plan does not hide, as the [`kept_charge`] of each.
    pub kept: u64,
    /// [`Plan::kept`] passes [`MOST_KEPT_BYTES`], so the decode refuses the
    /// file.
    pub too_much_kept: bool,
}

impl Plan {
    /// Hides a container of `size` bytes (`None` when it runs to the end
    /// of the file), with `bytes` in place of the bytes at `at`.  It gives
    /// `false` when it keeps the container visible: a small one past
    /// [`MOST_SMALL`], which it charges to [`Plan::kept`].
    pub fn hide(&mut self, at: u64, bytes: &[u8], size: Option<u64>) -> bool {
        let small = size.is_some_and(|size| size < ALWAYS_HIDDEN);
        if small {
            if self.small >= MOST_SMALL {
                let size = size.unwrap_or(ALWAYS_HIDDEN);
                self.kept = self.kept.saturating_add(kept_charge(size));
                self.too_much_kept = self.kept > MOST_KEPT_BYTES;
                return false;
            }
            self.small += 1;
        }
        self.patch(at, bytes);
        true
    }

    /// Shows at most 4 bytes of `bytes` in place of the bytes at `at`.  It
    /// is not a container, so it counts towards no limit.
    pub fn patch(&mut self, at: u64, bytes: &[u8]) {
        self.patches.push(Patch::of(at, bytes));
    }

    /// Shows a `free` box header of `size` bytes at the start of the view,
    /// only after a seek to that start.
    pub fn free_after_return(&mut self, size: u32) {
        self.late.push(Patch::of(self.start, &size.to_be_bytes()));
        self.late.push(Patch::of(self.start + 4, b"free"));
    }

    /// Whether the plan changes nothing.
    pub fn is_empty(&self) -> bool {
        self.start == 0 && self.patches.is_empty() && self.late.is_empty()
    }
}

/// A source of Symphonia's view of a file: the file from `start` of its
/// plan, with each patch in place.
pub(crate) struct Hidden {
    inner: Box<dyn MediaSource>,
    start: u64,
    patches: Vec<Patch>,
    /// The late patches of the plan, and whether a seek to the start of the
    /// view has shown them.
    late: Vec<Patch>,
    late_shown: bool,
    /// The offset of the next read, in the file.
    at: u64,
}

impl Hidden {
    /// Symphonia's view of `inner` by `plan`.  It seeks `inner` to the
    /// start of the view.
    pub fn new(mut inner: Box<dyn MediaSource>, plan: Plan) -> std::io::Result<Hidden> {
        let mut patches = plan.patches;
        patches.sort_by_key(|patch| patch.at);
        inner.seek(SeekFrom::Start(plan.start))?;
        Ok(Hidden {
            inner,
            start: plan.start,
            patches,
            late: plan.late,
            late_shown: false,
            at: plan.start,
        })
    }
}

impl Read for Hidden {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let count = self.inner.read(buf)?;
        let (from, to) = (self.at, self.at + count as u64);
        // The patches that can touch `from..to` start at most 4 bytes before it.
        let first = self.patches.partition_point(|patch| patch.at + 4 <= from);
        for patch in &self.patches[first..] {
            if patch.at >= to {
                break;
            }
            patch.apply(from, &mut buf[..count]);
        }
        if self.late_shown {
            for patch in &self.late {
                patch.apply(from, &mut buf[..count]);
            }
        }
        self.at = to;
        Ok(count)
    }
}

impl Seek for Hidden {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        let to = match to {
            SeekFrom::Start(at) => SeekFrom::Start(self.start.saturating_add(at)),
            other => other,
        };
        let at = self.inner.seek(to)?;
        if at < self.start {
            self.inner.seek(SeekFrom::Start(self.at))?;
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "a seek before the start of the view",
            ));
        }
        self.at = at;
        // Symphonia's MP4 reader walks the top-level boxes again from the
        // start of its stream when `moov` is last (`demuxer.rs`, line 257).
        if at == self.start {
            self.late_shown = true;
        }
        Ok(at - self.start)
    }
}

impl MediaSource for Hidden {
    fn is_seekable(&self) -> bool {
        self.inner.is_seekable()
    }

    fn byte_len(&self) -> Option<u64> {
        self.inner
            .byte_len()
            .map(|len| len.saturating_sub(self.start))
    }
}

/// The FLAC metadata blocks that Symphonia 0.6.1 reads whole
/// (`symphonia-bundle-flac` `demuxer.rs`, lines 106 to 140): APPLICATION
/// (2), SEEKTABLE (3, line 112), VORBIS_COMMENT (4) and PICTURE (6).  The decode reads in order, so it
/// needs no seek table (finding 6-3 of review round 6).
const FLAC_TAG_BLOCKS: [u8; 4] = [2, 3, 4, 6];

/// The type of a FLAC PADDING block, which Symphonia skips with a seek.
const FLAC_PADDING: u8 = 1;

/// The ID that a hidden RIFF or AIFF chunk shows to Symphonia: one that
/// it does not know, so it skips the chunk with `ignore_bytes`
/// (`symphonia-format-riff` `common.rs`, `ChunksReader::next`).
const HIDDEN_CHUNK: &[u8; 4] = b"JUNK";

/// The WAV chunk that Symphonia 0.6.1 reads whole: `LIST`, whose `INFO`
/// form it parses into tags (`wave/chunks.rs`, line 602).
const WAV_TAG_CHUNKS: [&[u8; 4]; 1] = [b"LIST"];

/// The AIFF chunks that Symphonia 0.6.1 reads whole (`aiff/chunks.rs`,
/// `ParseChunkTag`): `MARK`, `APPL`, `COMT`, the text chunks and `ID3 `.
const AIFF_TAG_CHUNKS: [&[u8; 4]; 8] = [
    b"MARK", b"APPL", b"COMT", b"NAME", b"AUTH", b"(c) ", b"ANNO", b"ID3 ",
];

/// Hides each chunk of `tags` of a RIFF or AIFF file, from the first chunk
/// at `at`.  A chunk's body has an even length: an odd one has a pad
/// byte.  The walk ends at the chunk `last`, if any, and at a chunk that
/// runs past the file.  Symphonia's WAV reader reads no chunk after
/// `data`; its AIFF reader reads the chunks after `SSND` too.
fn chunk_plan(
    source: &mut dyn ReadAt,
    mut at: u64,
    little_endian: bool,
    tags: &[&[u8; 4]],
    last: Option<&[u8; 4]>,
    plan: &mut Plan,
) -> std::io::Result<()> {
    loop {
        let mut header = [0; 8];
        if source.read_at(at, &mut header)? < 8 {
            return Ok(());
        }
        let id: [u8; 4] = [header[0], header[1], header[2], header[3]];
        let size = [header[4], header[5], header[6], header[7]];
        let size = u64::from(if little_endian {
            u32::from_le_bytes(size)
        } else {
            u32::from_be_bytes(size)
        });
        if tags.iter().any(|tag| **tag == id) {
            plan.hide(at, HIDDEN_CHUNK, Some(8 + size));
        }
        // Symphonia 0.6.1 refuses an AIFF SSND block size other than 0
        // (`aiff/chunks.rs`, line 321).  The block size only aligns the
        // samples, which start at the offset, so the view shows 0 (B6 of
        // review round 7).
        if id == *b"SSND" {
            let mut block = [0; 4];
            let read = source.read_at(at + 12, &mut block)?;
            if read == 4 && block != [0; 4] {
                plan.patch(at + 12, &[0; 4]);
            }
        }
        if last == Some(&id) {
            return Ok(());
        }
        at += 8 + size + (size & 1);
    }
}

/// The AIFC compression types that Symphonia 0.6.1 decodes
/// (`symphonia-format-riff` `aiff/chunks.rs`, lines 276 to 286), in either
/// case.
const AIFC_TYPES: [&[u8; 4]; 11] = [
    b"none", b"alaw", b"ulaw", b"in24", b"in32", b"23ni", b"fl32", b"fl64", b"sowt", b"twos",
    b"raw ",
];

/// Whether the AIFC file whose `FORM` starts at `at` has a compression
/// type that Symphonia 0.6.1 does not decode, by its `COMM` chunk.  An
/// AIFF, or an AIFC with no `COMM`, gives `false`.
pub(crate) fn is_aifc_of_other_type(source: &mut dyn ReadAt, at: u64) -> std::io::Result<bool> {
    let mut form = [0; 4];
    if source.read_at(at + 8, &mut form)? < 4 || form != *b"AIFC" {
        return Ok(false);
    }
    let mut chunk = at + 12;
    loop {
        let mut header = [0; 8];
        if source.read_at(chunk, &mut header)? < 8 {
            return Ok(false);
        }
        let size = u64::from(u32::from_be_bytes([
            header[4], header[5], header[6], header[7],
        ]));
        if header[..4] == *b"COMM" {
            // The channels, the frames, the sample size, the rate of 10
            // bytes, then the compression type.
            let mut kind = [0; 4];
            if source.read_at(chunk + 8 + 18, &mut kind)? < 4 {
                return Ok(false);
            }
            kind.make_ascii_lowercase();
            return Ok(!AIFC_TYPES.iter().any(|known| **known == kind));
        }
        chunk += 8 + size + (size & 1);
    }
}

/// The container of a file, which says the one Symphonia reader that the
/// decode opens.  The decode does not run Symphonia's probe.  It finds the
/// marker that the probe would find ([`start_of`]), and opens that reader
/// there, where the plan and the walk start (findings 5-2 and 5-3 of review
/// round 5, and 6-1 of review round 6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Container {
    Flac,
    Wav,
    Aiff,
    Mp4,
    Adts,
    Mp3,
}

/// The bytes of a view in which Symphonia 0.6.1's probe looks for a
/// marker: `max_probe_depth`, 1 MiB (`symphonia-core` `formats/probe.rs`,
/// line 304).
pub(crate) const SCAN_BYTES: usize = 1 << 20;

/// The last offset in a view at which the probe tests a marker.  The probe
/// reads a byte, counts it, stops past `max_probe_depth`, and then tests the
/// 2 bytes that end at that byte (`Probe::next`, lines 553 to 586).
pub(crate) const LAST_MARKER: usize = SCAN_BYTES - 2;

/// The most bytes from a marker that a score reads: an ADTS frame of up to
/// 8,191 bytes, and the next header, with its CRC (9).
pub(crate) const MOST_SCORE_BYTES: usize = 8191 + 9;

/// The offsets of a view that [`start_of`] tests in one read.  A file
/// that starts with its marker reads about 12 KiB, as the probe reads a
/// few KiB, so a read that fails later in the file fails the decode, not
/// the open.
const SCAN_BLOCK: usize = 4 * 1024;

/// The container whose marker Symphonia's probe accepts at `at` of
/// `bytes`, if any.  It tests each reader of `engine`'s Symphonia in the
/// order of `register_enabled_formats` (`symphonia` `lib.rs`, lines 262 to
/// 283), with the checks of its `score`.  A score that reads past the end
/// of `bytes` fails, as an end of the stream does.
pub(crate) fn marker_at(bytes: &[u8], at: usize) -> Option<Container> {
    let rest = bytes.get(at..)?;
    let form = rest.get(8..12);
    if adts_accepts(rest) {
        Some(Container::Adts)
    } else if rest.starts_with(b"fLaC") {
        // `FlacReader::score` accepts each marker (`symphonia-bundle-flac`
        // `demuxer.rs`, line 202).
        Some(Container::Flac)
    } else if rest.starts_with(b"ftyp") {
        // `IsoMp4Reader::score` accepts each marker (`symphonia-format-isomp4`
        // `demuxer.rs`, line 586).
        Some(Container::Mp4)
    } else if mpa_accepts(rest) {
        Some(Container::Mp3)
    } else if rest.starts_with(b"FORM") && matches!(form, Some(b"AIFF" | b"AIFC")) {
        // `AiffReader::score` (`symphonia-format-riff` `aiff/mod.rs`, line
        // 321).
        Some(Container::Aiff)
    } else if rest.starts_with(b"RIFF") && form == Some(b"WAVE") {
        // `WavReader::score` (`wave/mod.rs`, line 185).  The marker is
        // `RIFF` only: the reader of 0.6.1 refuses `RF64` (line 69).
        Some(Container::Wav)
    } else {
        None
    }
}

/// Where the reader of `container` starts, for its marker at `marker`.
/// `IsoMp4Reader::try_new` steps back 4 bytes from its marker `ftyp`, to the
/// size of the box, and no further than the start of the stream.
pub(crate) fn reader_start(container: Container, marker: u64) -> u64 {
    match container {
        Container::Mp4 => marker.saturating_sub(4),
        _ => marker,
    }
}

/// The first marker that Symphonia's probe accepts at an offset of
/// `offsets` in `bytes`, and its offset.
pub(crate) fn first_marker(
    bytes: &[u8],
    offsets: std::ops::Range<usize>,
) -> Option<(Container, usize)> {
    offsets
        .take_while(|at| *at < bytes.len())
        .find_map(|at| marker_at(bytes, at).map(|container| (container, at)))
}

/// The length of the ADTS frame whose header starts `bytes`, as
/// `AdtsHeader::read_no_resync` reads it (`symphonia-codec-aac` `adts.rs`,
/// lines 138 to 226).  `None` when Symphonia refuses the header.
fn adts_frame_bytes(bytes: &[u8]) -> Option<usize> {
    adts_header(bytes)
        .filter(|(_, raw_data_blocks)| *raw_data_blocks == 0)
        .map(|(frame_len, _)| frame_len)
}

/// Whether `bytes` start with an ADTS header that Symphonia 0.6.1 refuses
/// only for more than one raw data block (`adts.rs`, line 192).
pub(crate) fn is_adts_of_raw_blocks(bytes: &[u8]) -> bool {
    adts_header(bytes).is_some_and(|(_, raw_data_blocks)| raw_data_blocks > 0)
}

/// The frame length and the field of the raw data blocks of the ADTS
/// header that starts `bytes`, when Symphonia takes its other fields.
fn adts_header(bytes: &[u8]) -> Option<(usize, u8)> {
    let sync = u16::from_be_bytes([*bytes.first()?, *bytes.get(1)?]);
    if sync & 0xFFF6 != 0xFFF0 {
        return None;
    }
    let header_len = if sync & 1 == 0 { 9 } else { 7 };
    let header = bytes.get(..header_len)?;
    let rate_index = (header[2] >> 2) & 0x0F;
    let frame_len = (usize::from(header[3] & 0x03) << 11)
        | (usize::from(header[4]) << 3)
        | usize::from(header[5] >> 5);
    let raw_data_blocks = header[6] & 0x03;
    let accepted = rate_index <= 12 && frame_len >= header_len;
    accepted.then_some((frame_len, raw_data_blocks))
}

/// `AdtsReader::score` (`adts.rs`, line 91): a header, and a second header
/// after its frame.  The score has 16 KiB, more than a frame of 8,191 bytes
/// and a header, so it never gives its partial score.
fn adts_accepts(bytes: &[u8]) -> bool {
    adts_frame_bytes(bytes)
        .and_then(|frame| bytes.get(frame..))
        .and_then(adts_frame_bytes)
        .is_some()
}

/// Bit rates of MPEG audio, by version and layer, for the indexes 1 to 14
/// (`symphonia-bundle-mp3` `header.rs`, lines 19 to 47).
const MPEG1_L1: [u32; 14] = [
    32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448,
];
const MPEG1_L2: [u32; 14] = [
    32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384,
];
const MPEG1_L3: [u32; 14] = [
    32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
];
const MPEG2_L1: [u32; 14] = [
    32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256,
];
const MPEG2_L23: [u32; 14] = [8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];

/// The bytes after the header of the MPEG audio frame whose header is
/// `header`, as `parse_frame_header` gives them (`header.rs`, lines 102 to
/// 243).  `None` when Symphonia refuses the header.
fn mpa_frame_bytes(header: u32) -> Option<usize> {
    // Version: 0 is 2.5, 2 is 2, 3 is 1; 1 is refused.
    let version = (header >> 19) & 0x3;
    // Layer: 1 is layer 3, 2 is layer 2, 3 is layer 1; 0 is refused.
    let layer = (header >> 17) & 0x3;
    let rate_index = ((header >> 12) & 0xF) as usize;
    let sample_index = (header >> 10) & 0x3;
    if version == 1 || layer == 0 || rate_index == 0 || rate_index == 15 || sample_index == 3 {
        return None;
    }
    let table = match (version, layer) {
        (3, 3) => &MPEG1_L1,
        (3, 2) => &MPEG1_L2,
        (3, _) => &MPEG1_L3,
        (_, 3) => &MPEG2_L1,
        _ => &MPEG2_L23,
    };
    let bit_rate = table[rate_index - 1] * 1000;
    let base_rate = [11_025, 12_000, 8_000][sample_index as usize];
    let sample_rate = base_rate << [0, 0, 1, 2][version as usize];
    let mono = (header >> 6) & 0x3 == 3;
    let layer2_refused = if mono {
        [224_000, 256_000, 320_000, 384_000].contains(&bit_rate)
    } else {
        [32_000, 48_000, 56_000, 80_000].contains(&bit_rate)
    };
    if layer == 2 && layer2_refused {
        return None;
    }
    let factor = match (version, layer) {
        (_, 3) => 12,
        (3, _) | (_, 2) => 144,
        _ => 72,
    };
    let slot_bytes = if layer == 3 { 4 } else { 1 };
    let padding = ((header >> 9) & 1) as usize;
    let slots = (factor * bit_rate / sample_rate) as usize + padding;
    (slots * slot_bytes).checked_sub(4)
}

/// `MpaReader::score` (`symphonia-bundle-mp3` `demuxer.rs`, line 51): a
/// frame header after a frame sync of 11 bits of 1, and a second synced
/// header after its frame.  The score has 16 KiB, more than a frame of
/// 2,881 bytes and a header, so it never gives its partial score.
fn mpa_accepts(bytes: &[u8]) -> bool {
    let header_at = |at: usize| -> Option<u32> {
        Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
    };
    let synced = |header: u32| header & 0xFFE0_0000 == 0xFFE0_0000;
    let Some(first) = header_at(0).filter(|header| synced(*header)) else {
        return false;
    };
    let Some(frame) = mpa_frame_bytes(first) else {
        return false;
    };
    header_at(4 + frame)
        .filter(|second| synced(*second))
        .and_then(mpa_frame_bytes)
        .is_some()
}

/// Where Symphonia's view of `source` starts (after any leading ID3v2
/// tags), and the container whose reader opens, with where that reader
/// starts in the file.  It scans the view as Symphonia's probe does: each
/// offset up to [`LAST_MARKER`], in order, until a marker that a score
/// accepts ([`marker_at`]).  `None` when no marker is accepted.  It reads
/// [`SCAN_BLOCK`] offsets at a time, with the bytes that their scores read.
pub(crate) fn start_of(
    source: &mut dyn ReadAt,
) -> std::io::Result<(u64, Option<(Container, u64)>)> {
    let view = leading_id3_bytes(source)?;
    // The last offset of a block, and the bytes that its score reads.
    let mut block = vec![0; SCAN_BLOCK - 1 + MOST_SCORE_BYTES];
    let mut base = 0;
    while base <= LAST_MARKER {
        let read = source.read_at(view + base as u64, &mut block)?;
        let offsets = 0..SCAN_BLOCK.min(LAST_MARKER + 1 - base);
        if let Some((container, at)) = first_marker(&block[..read], offsets) {
            // The reader steps back from its marker no further than the
            // start of the view, which is the start of its stream (finding
            // 7-1 of review round 7).
            let marker = (base + at) as u64;
            let start = view + reader_start(container, marker);
            return Ok((view, Some((container, start))));
        }
        // The next block starts at `SCAN_BLOCK` of this one.
        if read <= SCAN_BLOCK {
            break;
        }
        base += SCAN_BLOCK;
    }
    let after_boxes = ftyp_after_boxes(source, view)?;
    Ok((view, after_boxes.map(|at| (Container::Mp4, at))))
}

/// The most top-level boxes that [`ftyp_after_boxes`] steps over.
const MOST_CHAIN_BOXES: usize = 1024;

/// The start of the first `ftyp` of a chain of top-level boxes from the
/// start of the view, past the window of the probe (B6 of review round 7).
/// Each box before it must be one that can start an MP4
/// (`mp4_boxes::FIRST_BOXES`), of a size that fits.  The MP4 reader then
/// walks the boxes before it again from the start of the view.
fn ftyp_after_boxes(source: &mut dyn ReadAt, view: u64) -> std::io::Result<Option<u64>> {
    let mut at = view;
    for _ in 0..MOST_CHAIN_BOXES {
        let mut header = [0; 16];
        let read = source.read_at(at, &mut header)?;
        if read < 8 {
            return Ok(None);
        }
        let kind = &header[4..8];
        if kind == b"ftyp" {
            return Ok(Some(at));
        }
        if !crate::mp4_boxes::FIRST_BOXES
            .iter()
            .any(|first| kind == &first[..])
        {
            return Ok(None);
        }
        // A size of 1 gives a 64-bit size after the type.  A size of 0 runs
        // to the end, so no `ftyp` follows.
        let small = u64::from(u32::from_be_bytes([
            header[0], header[1], header[2], header[3],
        ]));
        let (size, header_len) = match small {
            1 if read == 16 => (
                u64::from_be_bytes(header[8..16].try_into().unwrap_or_default()),
                16,
            ),
            small => (small, 8),
        };
        let Some(next) = at.checked_add(size).filter(|_| size >= header_len) else {
            return Ok(None);
        };
        at = next;
    }
    Ok(None)
}

/// The plan of a file of `container`, not an MP4, whose view starts at
/// `view` and whose reader starts at `at`.  It hides the tag blocks of a
/// FLAC and the tag chunks of a WAV and an AIFF.
pub(crate) fn plan_of(
    source: &mut dyn ReadAt,
    view: u64,
    at: u64,
    container: Container,
) -> std::io::Result<Plan> {
    let mut plan = Plan {
        start: view,
        ..Plan::default()
    };
    let chunks = at + 12;
    match container {
        Container::Flac => flac_plan(source, at + 4, &mut plan)?,
        Container::Wav => chunk_plan(
            source,
            chunks,
            true,
            &WAV_TAG_CHUNKS,
            Some(b"data"),
            &mut plan,
        )?,
        Container::Aiff => chunk_plan(source, chunks, false, &AIFF_TAG_CHUNKS, None, &mut plan)?,
        Container::Mp4 | Container::Adts | Container::Mp3 => {}
    }
    Ok(plan)
}

/// The bytes of the ID3v2 tags at the start of `source`, one after the
/// other.  `engine`'s Symphonia has no ID3v2 reader, so its probe gives up
/// past 1 MiB of a tag.  Each tag has a header of 10 bytes, a synchsafe
/// size, and a footer of 10 bytes when its flags say so.
pub(crate) fn leading_id3_bytes(source: &mut dyn ReadAt) -> std::io::Result<u64> {
    let mut at = 0_u64;
    loop {
        let mut header = [0; 10];
        if source.read_at(at, &mut header)? < 10 || header[..3] != *b"ID3" {
            return Ok(at);
        }
        let size = header[6..10]
            .iter()
            .fold(0_u64, |size, byte| (size << 7) | u64::from(byte & 0x7F));
        let footer = if header[5] & 0x10 != 0 { 10 } else { 0 };
        at += 10 + size + footer;
    }
}

/// Hides each tag block of the FLAC metadata from `at`: it gives PADDING
/// for its type, and keeps its bit of the last block.
fn flac_plan(source: &mut dyn ReadAt, mut at: u64, plan: &mut Plan) -> std::io::Result<()> {
    loop {
        let mut header = [0; 4];
        if source.read_at(at, &mut header)? < 4 {
            return Ok(());
        }
        let last = header[0] & 0x80;
        let kind = header[0] & 0x7F;
        let len = u64::from(u32::from_be_bytes([0, header[1], header[2], header[3]]));
        if FLAC_TAG_BLOCKS.contains(&kind) {
            plan.hide(at, &[last | FLAC_PADDING], Some(4 + len));
        }
        if last != 0 {
            return Ok(());
        }
        at += 4 + len;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// The plan of the whole `file`, as the decode makes it.
    fn plan_of_file(mut file: &[u8]) -> std::io::Result<Plan> {
        match start_of(&mut file)? {
            (view, Some((container, at))) => plan_of(&mut file, view, at, container),
            (view, None) => Ok(Plan {
                start: view,
                ..Plan::default()
            }),
        }
    }

    /// 32 bytes, each its own offset.
    fn file() -> Vec<u8> {
        (0..32).collect()
    }

    /// A view of [`file`] from `start`, with `free` at 10 to 13.
    fn view(start: u64) -> Hidden {
        let mut plan = Plan {
            start,
            ..Plan::default()
        };
        plan.hide(10, b"free", None);
        let hidden = Hidden::new(Box::new(Cursor::new(file())), plan);
        assert!(hidden.is_ok());
        hidden.unwrap()
    }

    /// `count` bytes of `view` from its offset `at`.
    fn bytes_at(view: &mut Hidden, at: u64, count: usize) -> Vec<u8> {
        assert_eq!(view.seek(SeekFrom::Start(at)).ok(), Some(at));
        let mut buf = vec![0; count];
        assert_eq!(view.read(&mut buf).ok(), Some(count));
        buf
    }

    /// The bytes of [`file`] from `from` to `to`, with the patch in place.
    fn want(from: u8, to: u8) -> Vec<u8> {
        (from..to)
            .map(|at| match at {
                10..14 => b"free"[usize::from(at - 10)],
                _ => at,
            })
            .collect()
    }

    #[test]
    fn a_read_gives_the_patch_and_leaves_the_other_bytes() {
        let mut view = view(0);
        assert_eq!(bytes_at(&mut view, 8, 8), want(8, 16), "a read across it");
        assert_eq!(
            bytes_at(&mut view, 12, 4),
            want(12, 16),
            "a read that starts in it"
        );
        assert_eq!(
            bytes_at(&mut view, 6, 5),
            want(6, 11),
            "a read that ends in it"
        );
        assert_eq!(bytes_at(&mut view, 11, 2), want(11, 13), "a read in it");
        assert_eq!(bytes_at(&mut view, 20, 4), want(20, 24), "a read after it");
        assert_eq!(bytes_at(&mut view, 0, 10), want(0, 10), "a read before it");
    }

    /// The view of a plan that starts at 4: offset 0 of the view is offset
    /// 4 of the file.
    #[test]
    fn a_view_seeks_and_states_its_length_from_its_start() {
        let mut view = view(4);
        assert_eq!(view.byte_len(), Some(28));
        assert!(view.is_seekable());
        assert_eq!(bytes_at(&mut view, 4, 8), want(8, 16));
        assert_eq!(view.seek(SeekFrom::Current(-6)).ok(), Some(6));
        let mut buf = [0; 2];
        assert_eq!(view.read(&mut buf).ok(), Some(2));
        assert_eq!(
            buf[..],
            want(10, 12)[..],
            "a read after a seek from the current offset"
        );
        assert_eq!(view.seek(SeekFrom::End(-2)).ok(), Some(26));
        assert_eq!(view.read(&mut buf).ok(), Some(2));
        assert_eq!(buf, [30, 31]);
        assert!(
            view.seek(SeekFrom::Current(-29)).is_err(),
            "a seek before the start"
        );
        assert_eq!(view.stream_position().ok(), Some(28), "the offset stays");
    }

    /// The plan of a FLAC with tags: PADDING for each tag block, with the
    /// bit of the last block kept.
    #[test]
    fn the_plan_of_a_flac_hides_its_tag_blocks() {
        let flac = crate::test_audio::flac(&[vec![0.25; 4096], vec![0.5; 4096]], 44_100);
        let tagged = crate::test_audio::with_flac_tags(&flac, 100);
        let plan = plan_of_file(&tagged[..]);
        assert!(plan.is_ok(), "{plan:?}");
        let plan = plan.unwrap();
        // STREAMINFO at 4, then VORBIS_COMMENT at 42, then APPLICATION.
        let kinds: Vec<(u64, [u8; 4], u8)> = plan
            .patches
            .iter()
            .map(|p| (p.at, p.bytes, p.len))
            .collect();
        let vorbis_len = 4 + 4 + 4 + 4 + 4 + 11;
        let application = 42 + vorbis_len;
        let picture = application + 4 + 8;
        let last = if tagged[picture as usize] & 0x80 != 0 {
            0x80
        } else {
            0
        };
        assert_eq!(
            kinds,
            [
                (42, [1, 0, 0, 0], 1),
                (application, [1, 0, 0, 0], 1),
                (picture, [last | 1, 0, 0, 0], 1),
            ]
        );
        let plain = plan_of_file(&flac[..]).map(|plan| plan.is_empty());
        assert_eq!(plain.ok(), Some(true), "a FLAC with no tags");
    }

    /// The plan of a file after ID3v2 tags starts after them, also after a
    /// tag with a footer.
    #[test]
    fn the_plan_starts_after_the_leading_id3_tags() {
        let mut with_footer = crate::test_audio::id3_tag(20);
        with_footer[5] = 0x10;
        with_footer.extend_from_slice(&[0; 10]);
        let file = [
            crate::test_audio::id3_tag(100),
            with_footer,
            Vec::from(*b"\xFF\xFB\x90\x00"),
        ]
        .concat();
        let plan = plan_of_file(&file);
        assert_eq!(plan.ok().map(|plan| plan.start), Some(110 + 40));
        let none = plan_of_file(&b"\xFF\xFB\x90\x00"[..]);
        assert_eq!(none.ok().map(|plan| plan.is_empty()), Some(true));
    }

    /// The plan of a WAV and an AIFF with tags: `JUNK` for each tag chunk.
    #[test]
    fn the_plan_of_a_wav_and_an_aiff_hides_their_tag_chunks() {
        use crate::test_audio::{Aiff, aiff, with_aiff_tags, with_wav_tags};
        let wav = crate::wav::encode_pcm16(&[0.25; 100], &[0.5; 100], 44_100);
        let patches = |file: &[u8]| {
            let plan = plan_of_file(file);
            assert!(plan.is_ok(), "{plan:?}");
            let plan = plan.unwrap();
            plan.patches
                .iter()
                .map(|p| (p.at, p.bytes, p.len))
                .collect::<Vec<_>>()
        };
        assert_eq!(patches(&with_wav_tags(&wav, 100)), [(36, *b"JUNK", 4)]);
        assert_eq!(patches(&wav), []);
        let plain = aiff(&[vec![0.25; 100], vec![0.5; 100]], 44_100, 16, Aiff::Plain);
        // NAME (8 + 5 + 1), ANNO (8 + 8), APPL (8 + 4 + 100), then ID3.
        // Finding 3-10 of review round 3: and AUTH (8 + 4) after SSND.
        let tagged = with_aiff_tags(&plain, 100);
        let at = [12, 26, 42, 154, tagged.len() as u64 - 12];
        let want: Vec<_> = at.iter().map(|&at| (at, *b"JUNK", 4)).collect();
        assert_eq!(patches(&tagged), want);
        assert_eq!(patches(&plain), []);
    }

    /// Findings 5-1 and 5-5 of review round 5: the charge of a small
    /// container counts each entry that it can hold.  A plan that keeps
    /// exactly `MOST_KEPT_BYTES` is not refused; one that keeps one byte
    /// more is.
    #[test]
    fn a_plan_is_refused_only_past_the_kept_limit() {
        assert_eq!(kept_charge(0), KEPT_OVERHEAD, "no entries is one entry");
        assert_eq!(kept_charge(3), 3 + KEPT_OVERHEAD);
        assert_eq!(kept_charge(4), 4 + KEPT_OVERHEAD);
        assert_eq!(kept_charge(4012), 4012 + 1003 * KEPT_OVERHEAD);
        // Sizes whose charges add to `MOST_KEPT_BYTES` exactly, and the
        // same with one size of 3 bytes made 4, one byte more.
        let at_the_limit = [vec![3; 64], vec![4092; 63], vec![60]].concat();
        let one_more = [vec![3; 63], vec![4; 1], vec![4092; 63], vec![60]].concat();
        let plan_of_sizes = |sizes: &[u64]| {
            let mut plan = Plan::default();
            for at in 0..MOST_SMALL as u64 {
                plan.hide(at, b"JUNK", Some(8));
            }
            for size in sizes {
                plan.hide(0, b"JUNK", Some(*size));
            }
            plan
        };
        let at = plan_of_sizes(&at_the_limit);
        assert_eq!((at.kept, at.too_much_kept), (MOST_KEPT_BYTES, false));
        let past = plan_of_sizes(&one_more);
        assert_eq!((past.kept, past.too_much_kept), (MOST_KEPT_BYTES + 1, true));
    }

    /// An ADTS frame header of `frame_len` bytes, stereo at 44.1 kHz, with
    /// a CRC when `crc` is set, and its payload of zeros.
    fn adts_frame(frame_len: usize, crc: bool) -> Vec<u8> {
        let len = frame_len as u32;
        let mut frame = vec![
            0xFF,
            if crc { 0xF0 } else { 0xF1 },
            (1 << 6) | (4 << 2),
            (0b10 << 6) | ((len >> 11) & 0x03) as u8,
            (len >> 3) as u8,
            (((len & 0x07) << 5) as u8) | 0x1F,
            0xFC,
        ];
        frame.resize(frame_len, 0);
        frame
    }

    /// Two MPEG audio frames of `header`, the first `frame` bytes long after
    /// its header, and the second header only.
    fn mpa_pair(header: u32, frame: usize) -> Vec<u8> {
        let mut pair = Vec::from(header.to_be_bytes());
        pair.resize(4 + frame, 0);
        pair.extend_from_slice(&header.to_be_bytes());
        pair
    }

    /// Finding 6-1 of review round 6: a marker is taken only where the
    /// score of its Symphonia 0.6.1 reader accepts it.
    #[test]
    fn a_marker_is_taken_only_where_its_score_accepts_it() {
        use Container::*;
        let adts = [adts_frame(16, false), adts_frame(16, false)].concat();
        let adts_crc = [adts_frame(20, true), adts_frame(20, true)].concat();
        let adts_crc_2 = adts_crc.clone();
        // The ID bit of each header set: MPEG-2 (finding 7-5 of review
        // round 7).
        let mpeg2 = |pair: &[u8], second: u8| {
            let mut pair = Vec::from(pair);
            let frame = pair.len() / 2;
            pair[1] = second;
            pair[frame + 1] = second;
            pair
        };
        let mut bad_rate = adts.clone();
        bad_rate[2] = (1 << 6) | (13 << 2);
        let mut two_blocks = adts.clone();
        two_blocks[6] = 0xFD;
        let short_frame = [adts_frame(16, false), adts_frame(6, false)].concat();
        let mut rate_12 = adts.clone();
        rate_12[2] = (1 << 6) | (12 << 2);
        let shortest = [adts_frame(7, false), adts_frame(7, false)].concat();
        let short_crc = [adts_frame(8, true), adts_frame(16, true)].concat();
        let unsynced = [
            &mpa_pair(0xFFFB_9000, 413)[..417],
            &0x00FB_9000_u32.to_be_bytes(),
        ]
        .concat();
        let cases: Vec<(&str, Vec<u8>, Option<Container>)> = vec![
            ("fLaC", Vec::from(*b"fLaC"), Some(Flac)),
            ("WAVE", Vec::from(*b"RIFF\0\0\0\0WAVE"), Some(Wav)),
            ("RF64", Vec::from(*b"RF64\0\0\0\0WAVE"), None),
            ("AVI", Vec::from(*b"RIFF\0\0\0\0AVI "), None),
            ("AIFF", Vec::from(*b"FORM\0\0\0\0AIFF"), Some(Aiff)),
            ("AIFC", Vec::from(*b"FORM\0\0\0\0AIFC"), Some(Aiff)),
            ("FORM JUNK", Vec::from(*b"FORM\0\0\0\0JUNK"), None),
            ("FORM cut", Vec::from(*b"FORM\0\0\0\0AIF"), None),
            ("ftyp", Vec::from(*b"ftypM4A "), Some(Mp4)),
            ("free", Vec::from(*b"free"), None),
            ("ADTS", adts.clone(), Some(Adts)),
            ("ADTS with CRC", adts_crc, Some(Adts)),
            ("MPEG-2 ADTS", mpeg2(&adts, 0xF9), Some(Adts)),
            ("MPEG-2 ADTS with CRC", mpeg2(&adts_crc_2, 0xF8), Some(Adts)),
            ("one ADTS header", adts_frame(16, false), None),
            ("an ADTS rate of 13", bad_rate, None),
            ("two ADTS blocks", two_blocks, None),
            ("an ADTS frame shorter than its header", short_frame, None),
            ("an ADTS rate of 12", rate_12, Some(Adts)),
            ("ADTS frames of only a header", shortest, Some(Adts)),
            ("an ADTS frame shorter than its CRC header", short_crc, None),
            ("MPEG2 layer 2", mpa_pair(0xFFF5_8000, 413), Some(Mp3)),
            ("a second header with no sync", unsynced, None),
            ("MPEG1 layer 3", mpa_pair(0xFFFB_9000, 413), Some(Mp3)),
            (
                "MPEG1 layer 3, off by one",
                mpa_pair(0xFFFB_9000, 414),
                None,
            ),
            (
                "MPEG1 layer 3 padded",
                mpa_pair(0xFFFB_9200, 414),
                Some(Mp3),
            ),
            ("MPEG2 layer 3", mpa_pair(0xFFF3_9000, 257), Some(Mp3)),
            ("MPEG2.5 layer 3", mpa_pair(0xFFE3_9000, 518), Some(Mp3)),
            ("MPEG1 layer 1", mpa_pair(0xFFFF_9000, 308), Some(Mp3)),
            ("MPEG2 layer 1", mpa_pair(0xFFF7_5000, 168), Some(Mp3)),
            ("MPEG1 layer 2", mpa_pair(0xFFFD_B000, 727), Some(Mp3)),
            (
                "MPEG1 layer 2, mono at 224k",
                mpa_pair(0xFFFD_B0C0, 727),
                None,
            ),
            ("MPEG1 layer 2 at 32k", mpa_pair(0xFFFD_1000, 100), None),
            (
                "MPEG1 layer 2 mono at 32k",
                mpa_pair(0xFFFD_10C0, 100),
                Some(Mp3),
            ),
            ("a free bit rate", mpa_pair(0xFFFB_0000, 413), None),
            ("a bit rate of 15", mpa_pair(0xFFFB_F000, 413), None),
            ("a sample rate of 3", mpa_pair(0xFFFB_9C00, 413), None),
            ("MPEG version 1", mpa_pair(0xFFEB_9000, 413), None),
            ("layer 0", mpa_pair(0xFFF9_9000, 413), None),
            (
                "one MPEG header",
                Vec::from(0xFFFB_9000_u32.to_be_bytes()),
                None,
            ),
            ("no sync", mpa_pair(0xFF1B_9000, 413), None),
        ];
        for (case, bytes, want) in cases {
            assert_eq!(marker_at(&bytes, 0), want, "{case}");
            let behind = [vec![0x42], bytes].concat();
            assert_eq!(marker_at(&behind, 1), want, "{case} behind a byte");
        }
        assert_eq!(reader_start(Mp4, 20), 16);
        assert_eq!(reader_start(Mp4, 2), 0);
        assert_eq!(reader_start(Flac, 20), 20);
    }

    /// Findings 6-1 and 6-6 of review round 6: the scan tests each offset
    /// that Symphonia's probe tests, up to `LAST_MARKER`, after any ID3v2
    /// tags, across the edges of its blocks.
    #[test]
    fn the_scan_tests_each_offset_of_the_probe() {
        use crate::test_audio::id3_tag;
        let start = |file: &[u8]| start_of(&mut { file }).ok();
        let at = |offset: usize, marker: &[u8]| [vec![0; offset], Vec::from(marker)].concat();
        let flac = *b"fLaC";
        let mp3 = mpa_pair(0xFFFB_9000, 413);
        let adts = [adts_frame(8191, true), adts_frame(9, true)].concat();
        let found = |container, offset: usize| Some((0, Some((container, offset as u64))));
        assert_eq!(start(&flac), found(Container::Flac, 0));
        assert_eq!(
            start(&at(LAST_MARKER, &flac)),
            found(Container::Flac, LAST_MARKER)
        );
        assert_eq!(
            LAST_MARKER, 1_048_574,
            "2 bytes up to the 1 MiB of the probe"
        );
        assert_eq!(
            start(&at(LAST_MARKER + 1, &flac)),
            Some((0, None)),
            "past the window"
        );
        let past = at(SCAN_BYTES + SCAN_BLOCK, &flac);
        assert_eq!(start(&past), Some((0, None)), "a block past the window");
        for edge in [
            SCAN_BLOCK - 3,
            SCAN_BLOCK - 1,
            SCAN_BLOCK,
            5 * SCAN_BLOCK - 1,
        ] {
            assert_eq!(
                start(&at(edge, &flac)),
                found(Container::Flac, edge),
                "fLaC at {edge}"
            );
            assert_eq!(
                start(&at(edge, &mp3)),
                found(Container::Mp3, edge),
                "MP3 at {edge}"
            );
            assert_eq!(
                start(&at(edge, &adts)),
                found(Container::Adts, edge),
                "ADTS at {edge}"
            );
        }
        let ftyp = at(SCAN_BLOCK + 2, b"ftyp");
        assert_eq!(start(&ftyp), found(Container::Mp4, SCAN_BLOCK - 2));
        let behind_tag = [id3_tag(100), at(70_000, &flac)].concat();
        assert_eq!(
            start(&behind_tag),
            Some((110, Some((Container::Flac, 70_110))))
        );
        assert_eq!(start(&[0; 100]), Some((0, None)));
        assert_eq!(start(&[]), Some((0, None)));
    }

    /// Finding 7-1 of review round 7: an `ftyp` at offsets 0 to 3 of a view
    /// after an ID3v2 tag gives a reader start at the view, where the MP4
    /// reader's step back stops, and never inside the tag.
    #[test]
    fn an_mp4_reader_starts_no_earlier_than_the_view() {
        use crate::test_audio::id3_tag;
        for offset in 0..4 {
            let file = [id3_tag(100), vec![0; offset], Vec::from(*b"ftyp")].concat();
            let start = start_of(&mut { file.as_slice() }).ok();
            assert_eq!(start, Some((110, Some((Container::Mp4, 110)))), "{offset}");
        }
    }

    /// B1 of review round 7: a late patch shows only after a seek to the
    /// start of the view, and then over the other patches.
    #[test]
    fn a_late_patch_shows_after_a_seek_to_the_start() {
        let mut plan = Plan {
            start: 2,
            ..Plan::default()
        };
        plan.patch(4, b"ab");
        plan.free_after_return(9);
        let hidden = Hidden::new(Box::new(Cursor::new(file())), plan);
        assert!(hidden.is_ok());
        let mut view = hidden.unwrap();
        let mut first = [0; 10];
        assert_eq!(view.read(&mut first).ok(), Some(10));
        assert_eq!(
            first,
            [2, 3, b'a', b'b', 6, 7, 8, 9, 10, 11],
            "before the seek"
        );
        assert_eq!(bytes_at(&mut view, 4, 2), [6, 7], "a seek elsewhere");
        let late = [0, 0, 0, 9, b'f', b'r', b'e', b'e', 10, 11];
        assert_eq!(
            bytes_at(&mut view, 0, 10),
            late,
            "after the seek to the start"
        );
    }

    /// B6 of review round 7: the chain of top-level boxes to an `ftyp`.
    #[test]
    fn an_ftyp_after_a_chain_of_boxes_is_found() {
        let header = |size: u32, kind: &[u8; 4]| [&size.to_be_bytes()[..], kind].concat();
        let large = [
            &1_u32.to_be_bytes()[..],
            b"free",
            &24_u64.to_be_bytes(),
            &[0; 8],
        ]
        .concat();
        let after = |prefix: Vec<u8>| {
            let file = [prefix, header(16, b"ftyp"), vec![0; 8]].concat();
            ftyp_after_boxes(&mut { file.as_slice() }, 0).ok().flatten()
        };
        assert_eq!(after(vec![]), Some(0));
        assert_eq!(after([header(12, b"free"), vec![0; 4]].concat()), Some(12));
        assert_eq!(
            after([header(8, b"uuid"), header(8, b"skip")].concat()),
            Some(16)
        );
        assert_eq!(after(large), Some(24), "a 64-bit size");
        assert_eq!(
            after([header(12, b"junk"), vec![0; 4]].concat()),
            None,
            "an unknown box"
        );
        assert_eq!(after(header(0, b"free")), None, "a size of 0");
        assert_eq!(after(header(7, b"free")), None, "a size under its header");
        let short_large = [&1_u32.to_be_bytes()[..], b"free", &8_u64.to_be_bytes()].concat();
        assert_eq!(after(short_large), None, "a 64-bit size under its header");
        let file = [header(8, b"free"), vec![0; 4]].concat();
        assert_eq!(
            ftyp_after_boxes(&mut { file.as_slice() }, 0).ok().flatten(),
            None,
            "a cut header"
        );
    }

    /// Missed mutants 3-24 and 3-25 of review round 3: a header of 9 bytes
    /// is no ID3v2 tag, and the walk of a FLAC cut in its metadata ends.
    #[test]
    fn a_cut_id3_header_or_flac_ends_the_plan() {
        let cut_id3 = plan_of_file(&b"ID3\x04\x00\x00\x00\x00\x01"[..]);
        assert_eq!(cut_id3.ok().map(|plan| plan.start), Some(0));
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let flac = crate::test_audio::flac(&[vec![0.25; 100], vec![0.5; 100]], 44_100);
            let mut not_last = Vec::from(&flac[..42]);
            not_last[4] &= 0x7F;
            for file in [Vec::from(*b"fLaC"), not_last] {
                let plan = plan_of_file(&file[..]).map(|plan| plan.is_empty());
                let _ = sender.send(plan.ok());
            }
        });
        for case in ["fLaC alone", "metadata with no last block"] {
            let ended = receiver.recv_timeout(std::time::Duration::from_secs(10));
            assert_eq!(ended, Ok(Some(true)), "{case}");
        }
    }

    /// Missed mutant 3-27 of review round 3: a `LIST` chunk of 512 bytes
    /// counts as small.
    #[test]
    fn small_wav_tag_chunks_count_against_the_cap() {
        let wav = crate::wav::encode_pcm16(&[0.25; 100], &[0.5; 100], 44_100);
        let file = crate::test_audio::with_small_wav_tags(&wav, MOST_SMALL + 1);
        let plan = plan_of_file(&file);
        assert!(plan.is_ok(), "{plan:?}");
        let plan = plan.unwrap();
        assert_eq!(plan.patches.len(), MOST_SMALL);
        assert_eq!(plan.kept, kept_charge(520), "the one chunk past the cap");
        assert!(!plan.too_much_kept);
    }

    /// A plan keeps every container of `ALWAYS_HIDDEN` bytes or more, and
    /// at most `MOST_SMALL` smaller ones.  One more small one makes it
    /// counts in `kept`.
    #[test]
    fn a_plan_keeps_at_most_most_small_small_patches() {
        let mut plan = Plan::default();
        for at in 0..MOST_SMALL as u64 + 10 {
            plan.hide(at * 8, b"free", Some(8));
        }
        assert_eq!(plan.patches.len(), MOST_SMALL);
        assert_eq!(plan.kept, 10 * kept_charge(8));
        assert!(!plan.too_much_kept);
        plan.hide(1 << 40, b"free", Some(ALWAYS_HIDDEN));
        plan.hide(1 << 41, b"free", None);
        assert_eq!(plan.patches.len(), MOST_SMALL + 2);
        assert!(!plan.is_empty());
        assert!(Plan::default().is_empty());
    }
}
