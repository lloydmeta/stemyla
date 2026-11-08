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
//! FLAC PICTURE, VORBIS_COMMENT and APPLICATION blocks show as PADDING.
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

/// The bytes that Symphonia keeps for each tag past its container, as the
/// decode counts them.  Measured with `peak_of` on 2026-11-09, for 50,000
/// small containers past [`MOST_SMALL`]: an AIFF `NAME` chunk of 10 bytes
/// keeps 176 bytes (an overhead of 166), a FLAC APPLICATION block of 1,028
/// bytes keeps 1,168 (140), and an empty top-level M4A `meta` of 12 bytes
/// keeps about 1.  The count takes 256, above the worst.
pub(crate) const KEPT_OVERHEAD: u64 = 256;

/// The most bytes of tags that Symphonia may keep, counted as the size of
/// each small container past [`MOST_SMALL`] and [`KEPT_OVERHEAD`]: 16 MiB.
/// It is the room for tags that the page gives each file
/// (`engine::decode::TAG_ROOM`), far below the 1 GiB of samples of decision
/// 0013 and the 2 GB of a tab (MEM-4).  A file past it is refused before
/// the probe ([`Plan::too_much_kept`]).  So a file of 5000 small tags
/// opens, and a file of a million tags of 2 bytes, which would keep about
/// 140 MB, does not.
pub(crate) const MOST_KEPT_BYTES: u64 = 16 << 20;

/// One change: `len` bytes of `bytes` in place of the bytes at `at` of the
/// file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Patch {
    pub at: u64,
    pub bytes: [u8; 4],
    pub len: u8,
}

/// The changes to a file: where Symphonia's view of it starts, and each
/// [`Patch`], at an offset of the file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Plan {
    pub start: u64,
    pub patches: Vec<Patch>,
    small: usize,
    /// The bytes that Symphonia keeps of the small containers that the
    /// plan does not hide, with [`KEPT_OVERHEAD`] for each.
    pub kept: u64,
    /// [`Plan::kept`] passes [`MOST_KEPT_BYTES`], so the decode refuses the
    /// file.
    pub too_much_kept: bool,
}

impl Plan {
    /// Hides a container of `size` bytes (`None` when it runs to the end
    /// of the file), with `bytes` in place of the bytes at `at`.
    pub fn hide(&mut self, at: u64, bytes: &[u8], size: Option<u64>) {
        let small = size.is_some_and(|size| size < ALWAYS_HIDDEN);
        if small {
            if self.small >= MOST_SMALL {
                let size = size.unwrap_or(ALWAYS_HIDDEN);
                self.kept = self.kept.saturating_add(size + KEPT_OVERHEAD);
                self.too_much_kept = self.kept > MOST_KEPT_BYTES;
                return;
            }
            self.small += 1;
        }
        let mut patch = Patch {
            at,
            bytes: [0; 4],
            len: bytes.len().min(4) as u8,
        };
        patch.bytes[..usize::from(patch.len)].copy_from_slice(&bytes[..usize::from(patch.len)]);
        self.patches.push(patch);
    }

    /// Whether the plan changes nothing.
    pub fn is_empty(&self) -> bool {
        self.start == 0 && self.patches.is_empty()
    }
}

/// A source of Symphonia's view of a file: the file from `start` of its
/// plan, with each patch in place.
pub(crate) struct Hidden {
    inner: Box<dyn MediaSource>,
    start: u64,
    patches: Vec<Patch>,
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
            for k in 0..u64::from(patch.len) {
                let at = patch.at + k;
                if (from..to).contains(&at) {
                    buf[(at - from) as usize] = patch.bytes[k as usize];
                }
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
/// (`symphonia-bundle-flac` `demuxer.rs`): APPLICATION (2), VORBIS_COMMENT
/// (4) and PICTURE (6).
const FLAC_TAG_BLOCKS: [u8; 3] = [2, 4, 6];

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
        if last == Some(&id) {
            return Ok(());
        }
        at += 8 + size + (size & 1);
    }
}

/// The container of a file, which says the one Symphonia reader that the
/// decode opens.  The decode does not run Symphonia's probe, which scans up
/// to 1 MiB for any format, and can open a part of the file that the plan
/// never saw (findings 5-2 and 5-3 of review round 5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Container {
    Flac,
    Wav,
    Aiff,
    Mp4,
    Adts,
    Mp3,
}

/// The container that `bytes` (the start of a view) start with: `fLaC`,
/// `RIFF` with `WAVE` or `RF64`, `FORM` with `AIFF` or `AIFC`, an MP4 box
/// (`mp4_boxes::FIRST_BOXES`), or the frame sync of an ADTS stream (layer
/// 00) or of an MP3.  `None` for anything else.
pub(crate) fn container_at(bytes: &[u8]) -> Option<Container> {
    let magic = bytes.get(..4)?;
    let form = bytes.get(8..12);
    let mp4 = bytes
        .get(4..8)
        .is_some_and(|kind| crate::mp4_boxes::FIRST_BOXES.iter().any(|k| kind == &k[..]));
    match (magic, form) {
        (b"fLaC", _) => Some(Container::Flac),
        (b"RIFF", Some(b"WAVE")) | (b"RF64", _) => Some(Container::Wav),
        (b"FORM", Some(b"AIFF" | b"AIFC")) => Some(Container::Aiff),
        _ if mp4 => Some(Container::Mp4),
        _ if is_adts_sync(magic) => Some(Container::Adts),
        _ if is_mp3_sync(magic) => Some(Container::Mp3),
        _ => None,
    }
}

/// An ADTS frame sync: 12 bits of 1, the ID bit, then a layer of 00.
fn is_adts_sync(bytes: &[u8]) -> bool {
    matches!(bytes, [0xFF, second, ..] if second & 0xF6 == 0xF0)
}

/// An MPEG audio frame sync: 11 bits of 1, then a layer other than 00.
fn is_mp3_sync(bytes: &[u8]) -> bool {
    matches!(bytes, [0xFF, second, ..] if second & 0xE0 == 0xE0 && second & 0x06 != 0)
}

/// The bytes after the start of a view that the decode scans for an MP3
/// frame sync, when the view starts with no known container: 1 MiB, as
/// Symphonia's probe does (`max_probe_depth`, `formats/probe.rs`).
pub(crate) const MP3_SCAN_BYTES: u64 = 1 << 20;

/// Where Symphonia's view of `source` starts (after any leading ID3v2
/// tags), and its container there.  A view of no known container with an
/// MP3 frame sync in its first [`MP3_SCAN_BYTES`] is an MP3: the MP3 reader
/// skips junk before its first frame by itself (`read_mpeg_frame_strict`).
pub(crate) fn start_of(source: &mut dyn ReadAt) -> std::io::Result<(u64, Option<Container>)> {
    let view = leading_id3_bytes(source)?;
    let mut magic = [0; 12];
    let read = source.read_at(view, &mut magic)?;
    if let Some(container) = container_at(&magic[..read]) {
        return Ok((view, Some(container)));
    }
    // Blocks of 64 KiB, each with the last byte of the one before.
    const BLOCK: usize = 64 * 1024;
    let mut block = vec![0; BLOCK + 1];
    let mut at = view;
    while at < view + MP3_SCAN_BYTES {
        let read = source.read_at(at, &mut block)?;
        if block[..read].windows(2).any(is_mp3_sync) {
            return Ok((view, Some(Container::Mp3)));
        }
        if read < block.len() {
            break;
        }
        at += BLOCK as u64;
    }
    Ok((view, None))
}

/// The plan of a file of `container`, not an MP4, whose view starts at
/// `view`.  It hides the tag blocks of a FLAC and the tag chunks of a WAV
/// and an AIFF.
pub(crate) fn plan_of(
    source: &mut dyn ReadAt,
    view: u64,
    container: Option<Container>,
) -> std::io::Result<Plan> {
    let mut plan = Plan {
        start: view,
        ..Plan::default()
    };
    let chunks = view + 12;
    match container {
        Some(Container::Flac) => flac_plan(source, view + 4, &mut plan)?,
        Some(Container::Wav) => chunk_plan(
            source,
            chunks,
            true,
            &WAV_TAG_CHUNKS,
            Some(b"data"),
            &mut plan,
        )?,
        Some(Container::Aiff) => {
            chunk_plan(source, chunks, false, &AIFF_TAG_CHUNKS, None, &mut plan)?
        }
        _ => {}
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
        let (view, container) = start_of(&mut file)?;
        plan_of(&mut file, view, container)
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

    /// The rule of the container at the start of a view (findings 5-2 to
    /// 5-4 of review round 5).
    #[test]
    fn the_container_is_the_one_at_the_start_of_the_view() {
        use Container::*;
        let cases: [(&[u8], Option<Container>); 17] = [
            (b"fLaC\0\0\0\x22", Some(Flac)),
            (b"RIFF\0\0\0\0WAVE", Some(Wav)),
            (b"RF64\0\0\0\0WAVE", Some(Wav)),
            (b"RIFF\0\0\0\0AVI ", None),
            (b"FORM\0\0\0\0AIFF", Some(Aiff)),
            (b"FORM\0\0\0\0AIFC", Some(Aiff)),
            (b"FORM\0\0\0\0JUNK", None),
            (b"\0\0\0\x20ftypM4A ", Some(Mp4)),
            (b"\0\0\0\x08moov", Some(Mp4)),
            (b"\0\0\0\x08free", Some(Mp4)),
            (b"\0\0\0\x08uuid", Some(Mp4)),
            (b"\0\0\0\x08junk", None),
            (b"\xFF\xF1\x50\x80", Some(Adts)),
            (b"\xFF\xFB\x90\x00", Some(Mp3)),
            (b"\xFF\xE0\x00\x00", None),
            (b"\xFF\x00\x00\x00", None),
            (b"\x42\xE2\x00\x00", None),
        ];
        for (bytes, want) in cases {
            assert_eq!(container_at(bytes), want, "{bytes:02x?}");
        }
        assert_eq!(container_at(b"fLa"), None, "too short");
    }

    /// A view of no known container is an MP3 when an MP3 frame sync is in
    /// its first `MP3_SCAN_BYTES`, after any ID3v2 tags.
    #[test]
    fn a_view_of_no_known_container_is_an_mp3_only_with_a_sync_in_reach() {
        use crate::test_audio::id3_tag;
        let sync = [0xFF_u8, 0xFB, 0x90, 0x00];
        let start = |file: &[u8]| start_of(&mut { file }).ok();
        let near = [id3_tag(100), vec![0; 70_000], Vec::from(sync)].concat();
        assert_eq!(start(&near), Some((110, Some(Container::Mp3))));
        let edge = [vec![0; MP3_SCAN_BYTES as usize - 2], Vec::from(sync)].concat();
        assert_eq!(start(&edge), Some((0, Some(Container::Mp3))), "at the edge");
        let past = [
            vec![0; MP3_SCAN_BYTES as usize + 64 * 1024],
            Vec::from(sync),
        ]
        .concat();
        assert_eq!(start(&past), Some((0, None)), "past the reach");
        assert_eq!(start(&[0; 100]), Some((0, None)));
        assert_eq!(start(&[]), Some((0, None)));
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
        assert_eq!(plan.kept, 520 + KEPT_OVERHEAD, "the one chunk past the cap");
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
        assert_eq!(plan.kept, 10 * (8 + KEPT_OVERHEAD));
        assert!(!plan.too_much_kept);
        plan.hide(1 << 40, b"free", Some(ALWAYS_HIDDEN));
        plan.hide(1 << 41, b"free", None);
        assert_eq!(plan.patches.len(), MOST_SMALL + 2);
        assert!(!plan.is_empty());
        assert!(Plan::default().is_empty());
    }
}
