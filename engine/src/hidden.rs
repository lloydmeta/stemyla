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
//! plans the other formats.  The view starts after any leading ID3v2 tags,
//! and FLAC PICTURE, VORBIS_COMMENT and APPLICATION blocks show as
//! PADDING.

use std::io::{Read, Seek, SeekFrom};

use symphonia::core::io::MediaSource;

use crate::mp4_boxes::ReadAt;

/// A container of tags of at least this many bytes is always hidden.
pub(crate) const ALWAYS_HIDDEN: u64 = 4096;

/// The most containers of fewer than [`ALWAYS_HIDDEN`] bytes that a plan
/// hides.  So the plan holds at most about 64 KiB for them, however many
/// a crafted file has.  Symphonia reads each one past this count, which
/// holds less than [`ALWAYS_HIDDEN`] bytes.
pub(crate) const MOST_SMALL: usize = 4096;

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
}

impl Plan {
    /// Hides a container of `size` bytes (`None` when it runs to the end
    /// of the file), with `bytes` in place of the bytes at `at`.
    pub fn hide(&mut self, at: u64, bytes: &[u8], size: Option<u64>) {
        let small = size.is_some_and(|size| size < ALWAYS_HIDDEN);
        if small {
            if self.small >= MOST_SMALL {
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

/// The plan of a file that is not an MP4, from its headers in `source`.
/// It starts the view after any leading ID3v2 tags, and hides the tag
/// blocks of a FLAC.
pub(crate) fn plan_of(source: &mut dyn ReadAt) -> std::io::Result<Plan> {
    let mut plan = Plan {
        start: leading_id3_bytes(source)?,
        ..Plan::default()
    };
    let mut magic = [0; 4];
    if source.read_at(plan.start, &mut magic)? == 4 && magic == *b"fLaC" {
        flac_plan(source, plan.start + 4, &mut plan)?;
    }
    Ok(plan)
}

/// The bytes of the ID3v2 tags at the start of `source`, one after the
/// other.  `engine`'s Symphonia has no ID3v2 reader, so its probe gives up
/// past 1 MiB of a tag.  Each tag has a header of 10 bytes, a synchsafe
/// size, and a footer of 10 bytes when its flags say so.
fn leading_id3_bytes(source: &mut dyn ReadAt) -> std::io::Result<u64> {
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

    /// A plan keeps every container of `ALWAYS_HIDDEN` bytes or more, and
    /// at most `MOST_SMALL` smaller ones.
    /// The plan of a FLAC with tags: PADDING for each tag block, with the
    /// bit of the last block kept.
    #[test]
    fn the_plan_of_a_flac_hides_its_tag_blocks() {
        let flac = crate::test_audio::flac(&[vec![0.25; 4096], vec![0.5; 4096]], 44_100);
        let tagged = crate::test_audio::with_flac_tags(&flac, 100);
        let plan = plan_of(&mut &tagged[..]);
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
        let plain = plan_of(&mut &flac[..]).map(|plan| plan.is_empty());
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
        let plan = plan_of(&mut &file[..]);
        assert_eq!(plan.ok().map(|plan| plan.start), Some(110 + 40));
        let none = plan_of(&mut &b"\xFF\xFB\x90\x00"[..]);
        assert_eq!(none.ok().map(|plan| plan.is_empty()), Some(true));
    }

    #[test]
    fn a_plan_keeps_at_most_most_small_small_patches() {
        let mut plan = Plan::default();
        for at in 0..MOST_SMALL as u64 + 10 {
            plan.hide(at * 8, b"free", Some(8));
        }
        assert_eq!(plan.patches.len(), MOST_SMALL);
        plan.hide(1 << 40, b"free", Some(ALWAYS_HIDDEN));
        plan.hide(1 << 41, b"free", None);
        assert_eq!(plan.patches.len(), MOST_SMALL + 2);
        assert!(!plan.is_empty());
        assert!(Plan::default().is_empty());
    }
}
