//! Hides the tags and the pictures of a file from Symphonia (MEM-4).
//!
//! Symphonia 0.6.1 reads tags and pictures whole, with no limit.  The app
//! reads none of them.  So the decode shows each box, block or chunk of
//! tags to Symphonia as one that Symphonia skips, and Symphonia skips a
//! large one with a seek (`MediaSourceStream::ignore_bytes`).  The bytes
//! keep their offsets, so the offsets in the file stay true.  A
//! [`Plan`] holds the changes, and [`Hidden`] gives them to each read.

use std::io::{Read, Seek, SeekFrom};

use symphonia::core::io::MediaSource;

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
