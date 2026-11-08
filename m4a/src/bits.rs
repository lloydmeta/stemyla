//! A bit writer that writes the most significant bit first.
//!
//! It keeps the bits that do not yet fill a byte in a 64-bit accumulator,
//! and pushes each whole byte into a `Vec<u8>`.  So it never writes past
//! the end of its buffer, which the `unsafe` bit writer of `alac-encoder`
//! 0.3.0 could do in a release build (D2 of `plans/active/m4a.md`).

/// A destination for bits.  The ALAC encoder writes its trial encodes to a
/// `BitCount`, and its packets to a `BitWriter`.
pub(crate) trait BitSink {
    /// Writes the low `bits` of `value`, 1 to 32 bits.  The other bits of
    /// `value` have no effect.
    fn write(&mut self, value: u32, bits: u32);
}

/// Appends bits to the end of a `Vec<u8>`.
pub(crate) struct BitWriter<'a> {
    bytes: &'a mut Vec<u8>,
    start: usize,
    pending: u64,
    pending_bits: u32,
}

impl<'a> BitWriter<'a> {
    /// A writer that appends after the bytes that `bytes` holds now.
    pub(crate) fn new(bytes: &'a mut Vec<u8>) -> Self {
        let start = bytes.len();
        BitWriter {
            bytes,
            start,
            pending: 0,
            pending_bits: 0,
        }
    }

    /// The number of bits written since `new`.
    pub(crate) fn bit_len(&self) -> usize {
        (self.bytes.len() - self.start) * 8 + self.pending_bits as usize
    }

    /// Writes zero bits up to the next byte boundary.
    pub(crate) fn byte_align(&mut self) {
        if self.pending_bits > 0 {
            self.write(0, 8 - self.pending_bits);
        }
    }
}

impl BitSink for BitWriter<'_> {
    fn write(&mut self, value: u32, bits: u32) {
        assert!((1..=32).contains(&bits), "cannot write {bits} bits");
        let low = u64::from(value) & (u64::MAX >> (64 - bits));
        self.pending = (self.pending << bits) | low;
        self.pending_bits += bits;
        while self.pending_bits >= 8 {
            self.pending_bits -= 8;
            self.bytes.push((self.pending >> self.pending_bits) as u8);
        }
        self.pending &= (1 << self.pending_bits) - 1;
    }
}

/// Counts bits and keeps none.  The trial encodes need only their size.
#[derive(Default)]
pub(crate) struct BitCount {
    bits: usize,
}

impl BitCount {
    pub(crate) fn bits(&self) -> usize {
        self.bits
    }
}

impl BitSink for BitCount {
    fn write(&mut self, _value: u32, bits: u32) {
        assert!((1..=32).contains(&bits), "cannot write {bits} bits");
        self.bits += bits as usize;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn written(writes: &[(u32, u32)]) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut writer = BitWriter::new(&mut bytes);
        for &(value, bits) in writes {
            writer.write(value, bits);
        }
        writer.byte_align();
        bytes
    }

    #[test]
    fn bits_are_written_most_significant_first() {
        assert_eq!(
            written(&[(0b101, 3), (0b1, 1), (0xABCD_EF01, 32), (0x3, 2)]),
            [0b1011_1010, 0xBC, 0xDE, 0xF0, 0b0001_1100]
        );
    }

    #[test]
    fn a_write_keeps_only_the_low_bits_of_its_value() {
        assert_eq!(written(&[(0xFFFF_FFFF, 3), (0, 5)]), [0b1110_0000]);
        assert_eq!(
            written(&[(0, 1), (-2_i16 as u32, 16), (0, 7)]),
            [0b0111_1111, 0b1111_1111, 0]
        );
    }

    #[test]
    fn byte_align_pads_with_zero_bits() {
        assert_eq!(written(&[(1, 1)]), [0b1000_0000]);
        assert_eq!(written(&[(0xFF, 8)]), [0xFF]);
    }

    #[test]
    fn a_writer_appends_after_the_bytes_already_there() {
        let mut bytes = vec![7, 8];
        let mut writer = BitWriter::new(&mut bytes);
        writer.write(0xA, 4);
        assert_eq!(writer.bit_len(), 4);
        writer.byte_align();
        assert_eq!(writer.bit_len(), 8);
        assert_eq!(bytes, [7, 8, 0xA0]);
    }

    #[test]
    fn a_count_adds_the_bits_of_each_write() {
        let mut count = BitCount::default();
        count.write(u32::MAX, 32);
        count.write(0, 3);
        assert_eq!(count.bits(), 35);
    }
}
