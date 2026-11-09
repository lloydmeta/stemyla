//! An ADTS stream with each frame of one raw data block (finding 8-3 of
//! review round 8, TD-86).  Symphonia 0.6.1 refuses a frame of more than one
//! block (`symphonia-codec-aac` `adts.rs`, line 192).  A frame with a CRC
//! states where each of its blocks starts: `raw_data_block_position[i]` is
//! the offset in bytes of block i from the start of block 0, and a CRC of
//! 16 bits ends each block (ISO/IEC 13818-7, `adts_header_error_check` and
//! `adts_raw_data_block_error_check`; the Fraunhofer FDK reader,
//! `tpdec_adts.cpp`, `adtsRead_DecodeHeader`).  So [`Split`] writes each
//! block as its own frame, with no CRC.  A frame of more blocks with no CRC
//! states no position: only a parse of its AAC bitstream finds where a block
//! ends, so it is an error with `decode::ADTS_BLOCKS_TEXT`.

use std::io::{self, Read};

/// The bytes of an ADTS header with no CRC.
const HEADER: usize = 7;

/// A reader of the ADTS stream of `inner`, with each frame of more blocks
/// and a CRC as one frame for each block.  It holds one frame at a time.
/// Bytes that do not start a frame go through as they are; Symphonia's
/// reader finds the next sync.
pub(crate) struct Split<R> {
    inner: R,
    out: Vec<u8>,
    at: usize,
    /// The stream no longer starts a frame here, so the rest goes through.
    through: bool,
}

impl<R: Read> Split<R> {
    pub(crate) fn new(inner: R) -> Split<R> {
        Split {
            inner,
            out: Vec::new(),
            at: 0,
            through: false,
        }
    }

    /// Reads up to `count` bytes, fewer only at the end of `inner`.
    fn take(&mut self, count: usize) -> io::Result<Vec<u8>> {
        let mut bytes = Vec::with_capacity(count);
        (&mut self.inner)
            .take(count as u64)
            .read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    /// Puts the next frame, or frames, in `out`.  `false` at the end.
    fn next_frames(&mut self) -> io::Result<bool> {
        self.out.clear();
        self.at = 0;
        let header = self.take(HEADER)?;
        if header.is_empty() {
            return Ok(false);
        }
        let synced = header.len() == HEADER && header[0] == 0xFF && header[1] & 0xF6 == 0xF0;
        if !synced {
            self.through = true;
            self.out = header;
            return Ok(true);
        }
        let frame_len = (usize::from(header[3] & 0x03) << 11)
            | (usize::from(header[4]) << 3)
            | usize::from(header[5] >> 5);
        let blocks = usize::from(header[6] & 0x03) + 1;
        let crc = header[1] & 0x01 == 0;
        let rest = self.take(frame_len.saturating_sub(HEADER))?;
        if blocks == 1 {
            self.out = [header, rest].concat();
            return Ok(true);
        }
        if !crc {
            return Err(io::Error::other(crate::decode::ADTS_BLOCKS_TEXT));
        }
        self.out = split_frame(&header, &rest, blocks)?;
        Ok(true)
    }
}

/// The frames of one block each of the frame of `blocks` blocks with a CRC
/// whose header is `header` and whose bytes after the header are `rest`.
fn split_frame(header: &[u8], rest: &[u8], blocks: usize) -> io::Result<Vec<u8>> {
    let bad = || io::Error::other("an ADTS frame states raw data blocks that it does not hold");
    // The positions of blocks 1 to N, then the header CRC.
    let first = 2 * (blocks - 1) + 2;
    let payload = rest.get(first..).ok_or_else(bad)?;
    let mut starts = vec![0];
    for k in 0..blocks - 1 {
        starts.push(usize::from(u16::from_be_bytes([
            rest[2 * k],
            rest[2 * k + 1],
        ])));
    }
    starts.push(payload.len());
    let mut out = Vec::with_capacity(payload.len() + HEADER * blocks);
    for pair in starts.windows(2) {
        let (start, end) = (pair[0], pair[1]);
        // Each block ends with its CRC of 2 bytes.
        let block = end
            .checked_sub(2)
            .filter(|block_end| start < *block_end)
            .and_then(|block_end| payload.get(start..block_end))
            .ok_or_else(bad)?;
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
    Ok(out)
}

impl<R: Read> Read for Split<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.through && self.at == self.out.len() {
            return self.inner.read(buf);
        }
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
}
