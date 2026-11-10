//! An MP4 from a source that cannot seek whose `mdat` or `moof` comes
//! before its `moov` (finding 16-2 of review round 16).
//!
//! Symphonia 0.6.1 stops its walk of a stream at the first `mdat` or
//! `moof` (`symphonia-format-isomp4` `demuxer.rs`, lines 210 to 225), and
//! then has no `moov` (lines 246 to 248).  So the decode holds the stream
//! whole, in blocks of 1 MiB with no copy, up to the limit of its samples,
//! and opens [`Held`] as a source that can seek: the walk, the hiding and
//! the checks then apply.  Only a native `decode_path` of a FIFO is such a
//! source, and it holds the decoded song as well.

use std::io::{Read, Seek, SeekFrom};

use symphonia::core::io::MediaSource;

/// The bytes of a block of the hold.
const BLOCK: usize = 1 << 20;

/// The most bytes of one ID3v2 tag, with its header: a synchsafe size of
/// 28 bits.
const MOST_TAG: u64 = 10 + 0x0FFF_FFFF;

/// A stream held whole, after leading ID3v2 tags that the scan read past.
/// The tags show as tags of zeros of the same length, so the positions
/// stay those of the file.
pub(crate) struct Held {
    /// Where each tag of zeros starts, and its bytes.
    tags: Vec<(u64, u64)>,
    tag_bytes: u64,
    blocks: Vec<Box<[u8]>>,
    len: u64,
    at: u64,
}

impl Held {
    /// Reads `source` to its end, after `tags` bytes of leading ID3v2 tags.
    /// `None` when it holds more than `most` bytes.
    pub(crate) fn read(
        source: &mut dyn Read,
        tags: u64,
        most: u64,
    ) -> std::io::Result<Option<Self>> {
        let (mut blocks, mut len) = (Vec::new(), 0_u64);
        loop {
            let mut block = vec![0; BLOCK];
            let mut filled = 0;
            while filled < BLOCK {
                match source.read(&mut block[filled..])? {
                    0 => break,
                    read => filled += read,
                }
            }
            len += filled as u64;
            if len > most {
                return Ok(None);
            }
            if filled > 0 {
                block.truncate(filled);
                blocks.push(block.into_boxed_slice());
            }
            if filled < BLOCK {
                break;
            }
        }
        let mut pieces = Vec::new();
        let mut at = 0;
        while at < tags {
            let left = tags - at;
            let piece = if left <= MOST_TAG {
                left
            } else {
                (MOST_TAG).min(left / 2)
            };
            pieces.push((at, piece));
            at += piece;
        }
        Ok(Some(Held {
            tags: pieces,
            tag_bytes: tags,
            blocks,
            len: tags + len,
            at: 0,
        }))
    }

    /// The byte at `at` of the tags of zeros.
    fn byte(&self, at: u64) -> u8 {
        let Some(&(start, bytes)) = self.tags.iter().rev().find(|(start, _)| *start <= at) else {
            return 0;
        };
        let size = bytes - 10;
        let synchsafe = [21, 14, 7, 0].map(|shift| ((size >> shift) & 0x7F) as u8);
        let header = [
            b'I',
            b'D',
            b'3',
            4,
            0,
            0,
            synchsafe[0],
            synchsafe[1],
            synchsafe[2],
            synchsafe[3],
        ];
        header.get((at - start) as usize).copied().unwrap_or(0)
    }
}

impl Read for Held {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let count = (self.len.saturating_sub(self.at)).min(buf.len() as u64) as usize;
        let mut done = 0;
        while done < count {
            let at = self.at + done as u64;
            if at < self.tag_bytes {
                buf[done] = self.byte(at);
                done += 1;
                continue;
            }
            let at = at - self.tag_bytes;
            let block = &self.blocks[(at / BLOCK as u64) as usize][(at % BLOCK as u64) as usize..];
            let len = block.len().min(count - done);
            buf[done..done + len].copy_from_slice(&block[..len]);
            done += len;
        }
        self.at += count as u64;
        Ok(count)
    }
}

impl Seek for Held {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        let at = match to {
            SeekFrom::Start(at) => Some(at),
            SeekFrom::End(delta) => self.len.checked_add_signed(delta),
            SeekFrom::Current(delta) => self.at.checked_add_signed(delta),
        };
        self.at = at.ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
        Ok(self.at)
    }
}

impl MediaSource for Held {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        Some(self.len)
    }
}

/// Whether the MP4 whose first box is at `at` of `head` has an `mdat` or a
/// `moof` before any `moov`, by its top-level boxes in `head`.  `false`
/// when `head` ends first.
pub(crate) fn media_before_moov(head: &[u8], mut at: usize) -> bool {
    loop {
        let Some(header) = at.checked_add(8).and_then(|end| head.get(at..end)) else {
            return false;
        };
        let (size, kind) = header.split_at(4);
        match kind {
            b"moov" => return false,
            b"mdat" | b"moof" => return true,
            _ => {}
        }
        let size = match u32::from_be_bytes(size.try_into().unwrap_or_default()) {
            1 => head.get(at + 8..at + 16).map_or(0, |large| {
                u64::from_be_bytes(large.try_into().unwrap_or_default())
            }),
            size => u64::from(size),
        };
        match usize::try_from(size).ok().filter(|size| *size >= 8) {
            Some(size) => match at.checked_add(size) {
                Some(next) => at = next,
                None => return false,
            },
            None => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_m4a::bx;

    /// Finding 17-1 of review round 17: a box of a size of 64 bits near
    /// `u64::MAX` ends the walk, with no overflow.
    #[test]
    fn a_box_size_near_u64_max_ends_the_walk() {
        let ftyp = bx(b"ftyp", &[b"M4A ", &[0; 4]]);
        let head = [&ftyp[..], &[0, 0, 0, 1], b"free", &[0xFF; 8], &[0; 64]].concat();
        assert!(!media_before_moov(&head, 0));
    }

    /// Finding 17-3 of review round 17: a `moof` before any `moov` is media,
    /// with no `mdat` in the head.
    #[test]
    fn a_moof_before_moov_is_media() {
        let head = [bx(b"ftyp", &[b"M4A ", &[0; 4]]), bx(b"moof", &[&[0; 8]])].concat();
        assert!(media_before_moov(&head, 0));
    }
}
