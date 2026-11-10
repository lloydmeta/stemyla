//! An ADTS stream with each frame of one raw data block (finding 8-3 of
//! review round 8, TD-86).  Symphonia 0.6.1 refuses a frame of more than one
//! block (`symphonia-codec-aac` `adts.rs`, line 192).  A frame with a CRC
//! states where each of its blocks starts: `raw_data_block_position[i]` is
//! the offset in bytes of block i from the start of block 0, and a CRC of
//! 16 bits ends each block (ISO/IEC 13818-7, `adts_header_error_check` and
//! `adts_raw_data_block_error_check`; the Fraunhofer FDK reader,
//! `tpdec_adts.cpp`, `adtsRead_DecodeHeader`).  So [`Split`] writes each
//! block as its own frame, with no CRC, in each frame of the stream, also
//! after bytes that are not a frame.  A cut last frame gives its whole
//! blocks.  A frame of more blocks with no CRC
//! states no position: only a parse of its AAC bitstream finds where a block
//! ends, so it is an error with `decode::ADTS_BLOCKS_TEXT`.

use std::io::{self, Read};

/// The bytes of an ADTS header with no CRC.
const HEADER: usize = 7;

/// A reader of the ADTS stream of `inner`, with each frame of more blocks
/// and a CRC as one frame for each block.  It holds one frame at a time.
/// A byte that does not start a valid ADTS header goes on as it is, and the
/// next byte is tried, so a frame after junk bytes splits too; Symphonia's
/// reader skips the junk to the next sync (finding 9-2 of review round 9).
pub(crate) struct Split<R> {
    inner: R,
    /// Bytes read from `inner` and not yet taken.
    pending: Vec<u8>,
    out: Vec<u8>,
    at: usize,
    /// The stream ends after `out`: its last frame was cut.
    ended: bool,
}

impl<R: Read> Split<R> {
    pub(crate) fn new(inner: R) -> Split<R> {
        Split {
            inner,
            pending: Vec::new(),
            out: Vec::new(),
            at: 0,
            ended: false,
        }
    }

    /// The next `count` bytes, without taking them: fewer only at the end of
    /// `inner`.
    fn peek(&mut self, count: usize) -> io::Result<&[u8]> {
        if self.pending.len() < count {
            let want = (count - self.pending.len()) as u64;
            (&mut self.inner)
                .take(want)
                .read_to_end(&mut self.pending)?;
        }
        Ok(&self.pending[..count.min(self.pending.len())])
    }

    /// Takes the next `count` bytes, fewer only at the end of `inner`.
    fn take(&mut self, count: usize) -> io::Result<Vec<u8>> {
        self.peek(count)?;
        let count = count.min(self.pending.len());
        Ok(self.pending.drain(..count).collect())
    }

    /// Puts the next frame, or frames, or a byte that starts none, in
    /// `out`.  `false` at the end.
    fn next_frames(&mut self) -> io::Result<bool> {
        self.out.clear();
        self.at = 0;
        if self.ended {
            return Ok(false);
        }
        // A header with a CRC has 9 bytes.
        let header = self.peek(HEADER + 2)?;
        if header.is_empty() {
            return Ok(false);
        }
        let Some((frame_len, blocks)) = crate::hidden::adts_header_of(header) else {
            if header.len() < HEADER {
                // A cut header at the end of the stream goes on as it is.
                self.out = self.take(HEADER)?;
            } else {
                self.out = self.take(1)?;
            }
            return Ok(true);
        };
        let blocks = usize::from(blocks) + 1;
        let crc = header[1] & 0x01 == 0;
        let frame = self.take(frame_len)?;
        if blocks == 1 {
            self.out = frame;
            return Ok(true);
        }
        if !crc {
            return Err(io::Error::other(crate::decode::ADTS_BLOCKS_TEXT));
        }
        let (out, cut) = split_frame(
            &frame[..HEADER],
            &frame[HEADER..],
            frame_len - HEADER,
            blocks,
        )?;
        self.out = out;
        self.ended = cut;
        Ok(true)
    }
}

/// The frames of one block each of the frame of `blocks` blocks with a CRC,
/// whose header is `header`, whose header states `stated` bytes after it,
/// and whose bytes after the header are `rest`.  Each block ends with its
/// CRC of 2 bytes, and has at least one byte.  When `rest` is cut, it gives
/// each whole block, and `true`.
fn split_frame(
    header: &[u8],
    rest: &[u8],
    stated: usize,
    blocks: usize,
) -> io::Result<(Vec<u8>, bool)> {
    let bad = || io::Error::other("an ADTS frame states raw data blocks that it does not hold");
    // The positions of blocks 1 to N, then the header CRC.
    let first = 2 * (blocks - 1) + 2;
    let cut = rest.len() < stated;
    let Some(payload) = rest.get(first..) else {
        return if cut {
            Ok((Vec::new(), true))
        } else {
            Err(bad())
        };
    };
    let mut starts = vec![0];
    for k in 0..blocks - 1 {
        starts.push(usize::from(u16::from_be_bytes([
            rest[2 * k],
            rest[2 * k + 1],
        ])));
    }
    starts.push(stated.checked_sub(first).ok_or_else(bad)?);
    let mut out = Vec::with_capacity(payload.len() + HEADER * blocks);
    for pair in starts.windows(2) {
        let (start, end) = (pair[0], pair[1]);
        if start >= end.saturating_sub(2) {
            return Err(bad());
        }
        let Some(block) = payload.get(start..end - 2).filter(|_| end <= payload.len()) else {
            // A cut block: the stream ends after the whole ones.
            return if cut { Ok((out, true)) } else { Err(bad()) };
        };
        let len = (HEADER + block.len()) as u32;
        let mut one = [0; HEADER];
        one.copy_from_slice(&header[..HEADER]);
        one[1] |= 0x01;
        one[3] = (one[3] & 0xFC) | ((len >> 11) & 0x03) as u8;
        one[4] = (len >> 3) as u8;
        one[5] = (one[5] & 0x1F) | (((len & 0x07) << 5) as u8);
        one[6] &= 0xFC;
        out.extend_from_slice(&one);
        out.extend_from_slice(block);
    }
    Ok((out, cut))
}

impl<R: Read> Read for Split<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        while self.at == self.out.len() {
            if !self.next_frames()? {
                return Ok(0);
            }
        }
        let count = buf.len().min(self.out.len() - self.at);
        buf[..count].copy_from_slice(&self.out[self.at..self.at + count]);
        self.at += count;
        Ok(count)
    }
}

/// [`Split`] as a source that cannot seek, with the length of the bytes
/// that it reads from, so that Symphonia estimates the length of the stream
/// from its first 16 KiB (`adts.rs`, line 424).
pub(crate) struct SplitSource {
    split: Split<Box<dyn symphonia::core::io::MediaSource>>,
    len: Option<u64>,
}

impl SplitSource {
    pub(crate) fn new(inner: Box<dyn symphonia::core::io::MediaSource>, len: Option<u64>) -> Self {
        SplitSource {
            split: Split::new(inner),
            len,
        }
    }
}

impl Read for SplitSource {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.split.read(buf)
    }
}

impl io::Seek for SplitSource {
    fn seek(&mut self, _: io::SeekFrom) -> io::Result<u64> {
        Err(io::Error::other("a split ADTS stream cannot seek"))
    }
}

impl symphonia::core::io::MediaSource for SplitSource {
    fn is_seekable(&self) -> bool {
        false
    }

    fn byte_len(&self) -> Option<u64> {
        self.len
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All that `Split` reads from `bytes`, or its error.
    fn split(bytes: &[u8]) -> io::Result<Vec<u8>> {
        let mut out = Vec::new();
        Split::new(bytes).read_to_end(&mut out)?;
        Ok(out)
    }

    /// Finding 8-3 of review round 8: each frame of one block goes through
    /// byte for byte, each frame of more blocks with a CRC splits, and bad
    /// positions are an error.
    #[test]
    fn a_frame_splits_into_one_frame_for_each_block() {
        use crate::test_m4a::adts_in_blocks;
        let blocks: [&[u8]; 3] = [&[1, 2, 3], &[4, 5], &[6, 7, 8, 9]];
        let one = adts_in_blocks(&blocks, 44_100, 2, &[1], false);
        assert_eq!(split(&one).ok(), Some(one.clone()), "one block");
        let three = adts_in_blocks(&blocks, 44_100, 2, &[3], true);
        assert_eq!(
            split(&three).ok(),
            Some(one.clone()),
            "three blocks with a CRC"
        );
        let with_crc = adts_in_blocks(&blocks, 44_100, 2, &[1], true);
        assert_eq!(
            split(&with_crc).ok(),
            Some(with_crc.clone()),
            "one block, no CRC to drop"
        );
        let no_crc = adts_in_blocks(&blocks, 44_100, 2, &[2], false);
        let error = split(&no_crc).err().map(|e| e.to_string());
        assert_eq!(error.as_deref(), Some(crate::decode::ADTS_BLOCKS_TEXT));
        // Block 1 starts past the end of the frame.
        let mut far = three.clone();
        far[7..9].copy_from_slice(&200_u16.to_be_bytes());
        assert!(split(&far).is_err(), "a position past the end");
        // Block 1 starts before the CRC of block 0 ends.
        let mut early = three.clone();
        early[7..9].copy_from_slice(&1_u16.to_be_bytes());
        assert!(split(&early).is_err(), "a block shorter than its CRC");
        // A stream cut inside a frame gives the bytes that it holds.
        assert_eq!(
            split(&one[..5]).ok(),
            Some(Vec::from(&one[..5])),
            "a cut header"
        );
        assert_eq!(
            split(&one[..9]).ok(),
            Some(Vec::from(&one[..9])),
            "a cut frame"
        );
        // Bytes that are not a frame go through.
        let junk = [&[0x42; 3][..], &one].concat();
        assert_eq!(split(&junk).ok(), Some(junk.clone()), "junk first");
    }

    /// Findings 9-2, 9-6, 9-24 and missed mutant 9-27 of review round 9.
    #[test]
    fn the_split_takes_junk_a_cut_frame_and_reads_of_any_size() {
        use crate::test_m4a::adts_in_blocks;
        let blocks: [&[u8]; 6] = [&[1, 2, 3], &[4, 5], &[6, 7, 8, 9], &[10], &[11, 12], &[13]];
        let one = adts_in_blocks(&blocks, 44_100, 2, &[1], false);
        let three = adts_in_blocks(&blocks, 44_100, 2, &[3], true);
        let junk = [&one[..10], &[0x42; 3], &three].concat();
        let want = [&one[..10], &[0x42; 3], &one[..]].concat();
        assert_eq!(
            split(&junk).ok(),
            Some(want.clone()),
            "junk, then frames of 3 blocks"
        );
        let mut by_one = Vec::new();
        let mut reader = Split::new(junk.as_slice());
        let mut byte = [0; 1];
        while reader.read(&mut byte).ok() == Some(1) {
            by_one.push(byte[0]);
        }
        assert_eq!(by_one, want, "reads of 1 byte");
        // The last frame of 3 blocks, cut: only its whole blocks, each with
        // its CRC.
        let frame = adts_in_blocks(&blocks[3..], 44_100, 2, &[3], true);
        let first = adts_in_blocks(&blocks[..3], 44_100, 2, &[3], true);
        let whole_first = adts_in_blocks(&blocks[..3], 44_100, 2, &[1], false);
        let ones = adts_in_blocks(&blocks[3..], 44_100, 2, &[1], false);
        // The frames of one block each: 7 + 1, 7 + 2 and 7 + 1 bytes.
        for (cut, held) in [(1, 2), (2, 2), (3, 2), (5, 1), (frame.len() - 9, 0)] {
            let file = [&first[..], &frame[..frame.len() - cut]].concat();
            let want = [&whole_first[..], &ones[..[0, 8, 17][held]]].concat();
            assert_eq!(split(&file).ok(), Some(want), "cut by {cut}");
        }
        // A block of no bytes before its CRC.
        let mut empty = three.clone();
        empty[7..9].copy_from_slice(&2_u16.to_be_bytes());
        assert!(split(&empty).is_err(), "a block of 0 bytes");
    }
}
