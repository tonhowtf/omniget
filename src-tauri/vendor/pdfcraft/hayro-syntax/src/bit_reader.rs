//! A simple bit reader and writer.

use core::fmt::Debug;
use smallvec::{SmallVec, smallvec};

/// A bit reader.
#[derive(Debug, Clone)]
pub struct BitReader<'a> {
    /// The underlying bytes of the bit reader.
    pub data: &'a [u8],
    cur_pos: usize,
}

impl<'a> BitReader<'a> {
    /// Create a new bit reader.
    #[inline]
    pub fn new(data: &'a [u8]) -> Self {
        Self::new_with(data, 0)
    }

    /// Create a new bit reader, and start at a specific bit offset.
    #[inline]
    pub fn new_with(data: &'a [u8], cur_pos: usize) -> Self {
        Self { data, cur_pos }
    }

    /// Align the reader to the next byte boundary.
    #[inline]
    pub fn align(&mut self) {
        let bit_pos = self.bit_pos();

        if !bit_pos.is_multiple_of(8) {
            self.cur_pos += 8 - bit_pos;
        }
    }

    /// Read the given number of bits from the byte stream.
    ///
    /// Returns `None` if `bit_size` > 32.
    #[inline(always)]
    pub fn read(&mut self, bit_size: u8) -> Option<u32> {
        if !(1..=32).contains(&bit_size) {
            return None;
        }

        let byte_pos = self.byte_pos();

        if byte_pos >= self.data.len() {
            return None;
        }

        let item = match bit_size {
            8 => {
                let item = self.data[byte_pos] as u32;
                self.cur_pos += 8;

                Some(item)
            }
            1..=32 => {
                let bit_pos = self.bit_pos();
                let end_byte_pos = (bit_pos + bit_size as usize - 1) / 8;
                let mut read = [0_u8; 8];

                for (i, r) in read.iter_mut().enumerate().take(end_byte_pos + 1) {
                    *r = *self.data.get(byte_pos + i)?;
                }

                let item = (u64::from_be_bytes(read) >> (64 - bit_pos - bit_size as usize)) as u32
                    & bit_mask(bit_size);
                self.cur_pos += bit_size as usize;

                Some(item)
            }
            _ => None,
        }?;

        Some(item)
    }

    /// Peak the given number of bits.
    pub fn peak(&mut self, bit_size: u8) -> Option<u32> {
        self.clone().read(bit_size)
    }

    /// Whether the bit reader has read all bytes.
    pub fn at_end(&self) -> bool {
        self.byte_pos() >= self.data.len()
    }

    /// Get the current byte position.
    #[inline]
    pub fn byte_pos(&self) -> usize {
        self.cur_pos / 8
    }

    /// Get the current position within the byte.
    #[inline]
    pub fn bit_pos(&self) -> usize {
        self.cur_pos % 8
    }

    /// Get the current position in bits.
    #[inline]
    pub fn cur_pos(&self) -> usize {
        self.cur_pos
    }

    /// Get the tail (aligned to the current byte position) of the data.
    #[inline]
    pub fn tail(&self) -> &'a [u8] {
        &self.data[self.byte_pos()..]
    }

    /// Get the full byte (aligned to the byte boundary) of the current position.
    #[inline]
    pub fn cur_byte(&self) -> Option<u8> {
        self.data.get(self.byte_pos()).copied()
    }
}

/// Get the mask for the given bit size.
pub fn bit_mask(bit_size: u8) -> u32 {
    match bit_size {
        0 => 0,
        1..=31 => (1_u32 << bit_size) - 1,
        _ => u32::MAX,
    }
}
/// A bit writer.
#[derive(Debug)]
pub struct BitWriter<'a> {
    data: &'a mut [u8],
    cur_pos: usize,
    bit_size: u8,
}

impl<'a> BitWriter<'a> {
    /// Create a new bit writer for a fixed bit size between 1 and 32 (inclusive).
    #[inline]
    pub fn new(data: &'a mut [u8], bit_size: u8) -> Option<Self> {
        if !(1..=32).contains(&bit_size) {
            return None;
        }

        Some(Self {
            data,
            bit_size,
            cur_pos: 0,
        })
    }

    /// Split off the already-written parts and return a new bit writer for the tail.
    #[inline]
    pub fn split_off(self) -> (&'a [u8], Self) {
        // Assumes that we are currently aligned to a byte boundary!
        let (left, right) = self.data.split_at_mut(self.cur_pos / 8);
        (
            left,
            BitWriter {
                data: right,
                cur_pos: 0,
                bit_size: self.bit_size,
            },
        )
    }

    /// Align the writer to the next byte boundary.
    #[inline]
    pub fn align(&mut self) {
        let bit_pos = self.bit_pos();

        if !bit_pos.is_multiple_of(8) {
            self.cur_pos += 8 - bit_pos;
        }
    }

    /// Return the number of written bits.
    #[inline]
    pub fn cur_pos(&self) -> usize {
        self.cur_pos
    }

    /// Return the whole underlying buffer.
    #[inline]
    pub fn get_data(&self) -> &[u8] {
        self.data
    }

    fn bit_pos(&self) -> usize {
        self.cur_pos % 8
    }

    /// Write the given number into the buffer.
    #[inline]
    pub fn write(&mut self, val: u32) -> Option<()> {
        let bit_size = self.bit_size as usize;
        let mut bits_left = bit_size;
        let value = val & bit_mask(self.bit_size);

        while bits_left > 0 {
            let already_advanced = bit_size - bits_left;
            let absolute_pos = self.cur_pos + already_advanced;
            let byte_pos = absolute_pos / 8;
            let bit_pos = absolute_pos % 8;
            let bits_in_byte = (8 - bit_pos).min(bits_left);
            let shift = bits_left - bits_in_byte;
            let chunk_mask = bit_mask(bits_in_byte as u8);
            let chunk = ((value >> shift) & chunk_mask) as u8;

            let shift_in_byte = 8 - bits_in_byte - bit_pos;
            let byte = self.data.get_mut(byte_pos)?;
            let byte_mask = (chunk_mask as u8) << shift_in_byte;

            *byte = (*byte & !byte_mask) | ((chunk << shift_in_byte) & byte_mask);

            bits_left -= bits_in_byte;
        }

        self.cur_pos += bit_size;

        Some(())
    }

    /// Fill bytes with a repeated value when at a byte boundary.
    #[inline]
    pub fn fill_bytes(&mut self, byte: u8, count: usize) -> Option<()> {
        debug_assert_eq!(self.cur_pos % 8, 0);

        let start = self.cur_pos / 8;
        let end = start + count;
        self.data.get_mut(start..end)?.fill(byte);
        self.cur_pos += count * 8;
        Some(())
    }

    /// Write multiple numbers at once.
    #[inline]
    pub fn write_bits(&mut self, bits: impl IntoIterator<Item = u32>) -> Option<()> {
        for bit in bits {
            let value: u32 = bit;
            self.write(value)?;
        }

        Some(())
    }
}

/// An iterator over bit chunks.
pub struct BitChunks<'a> {
    reader: BitReader<'a>,
    bit_size: u8,
    chunk_len: usize,
}

impl<'a> BitChunks<'a> {
    /// Create a new iterator over bit chunks.
    pub fn new(data: &'a [u8], bit_size: u8, chunk_len: usize) -> Option<Self> {
        if !(1..=16).contains(&bit_size) || chunk_len == 0 {
            return None;
        }

        let reader = BitReader::new(data);

        Some(Self {
            reader,
            bit_size,
            chunk_len,
        })
    }
}

impl Iterator for BitChunks<'_> {
    type Item = BitChunk;

    fn next(&mut self) -> Option<Self::Item> {
        let mut bits = SmallVec::new();

        for _ in 0..self.chunk_len {
            bits.push(self.reader.read(self.bit_size)? as u16);
        }

        Some(BitChunk { bits })
    }
}

/// A chunk of bits.
#[derive(Debug, Clone)]
pub struct BitChunk {
    bits: SmallVec<[u16; 4]>,
}

impl BitChunk {
    /// Return an iterator over the numbers in the chunk.
    pub fn iter(&self) -> impl Iterator<Item = u16> + '_ {
        self.bits.iter().copied()
    }

    /// Create a new bit chunk with the given value being repeated `count` times.
    pub fn new(val: u8, count: usize) -> Self {
        Self {
            bits: smallvec![val as u16; count],
        }
    }

    /// Create a new bit chunk from the given reader.
    pub fn from_reader(
        bit_reader: &mut BitReader<'_>,
        bit_size: u8,
        chunk_len: usize,
    ) -> Option<Self> {
        if !(1..=16).contains(&bit_size) || chunk_len == 0 {
            return None;
        }

        let mut bits = SmallVec::new();

        for _ in 0..chunk_len {
            bits.push(bit_reader.read(bit_size)? as u16);
        }

        Some(Self { bits })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bit_reader_16() {
        let data = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06];
        let mut reader = BitReader::new(&data);
        assert_eq!(
            reader.read(16).unwrap() as u16,
            u16::from_be_bytes([0x01, 0x02])
        );
        assert_eq!(
            reader.read(16).unwrap() as u16,
            u16::from_be_bytes([0x03, 0x04])
        );
        assert_eq!(
            reader.read(16).unwrap() as u16,
            u16::from_be_bytes([0x05, 0x06])
        );
    }

    #[test]
    fn bit_writer_16() {
        let mut buf = vec![0_u8; 6];
        let mut writer = BitWriter::new(&mut buf, 16).unwrap();
        writer
            .write(u16::from_be_bytes([0x01, 0x02]) as u32)
            .unwrap();
        writer
            .write(u16::from_be_bytes([0x03, 0x04]) as u32)
            .unwrap();
        writer
            .write(u16::from_be_bytes([0x05, 0x06]) as u32)
            .unwrap();

        assert_eq!(buf, [0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);
    }

    #[test]
    fn bit_reader_12() {
        let data = [0b10011000, 0b00011111, 0b10101001, 0b11101001, 0b00011010];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.read(12).unwrap(), 0b100110000001);
        assert_eq!(reader.read(12).unwrap(), 0b111110101001);
        assert_eq!(reader.read(12).unwrap(), 0b111010010001);
    }

    #[test]
    fn bit_reader_9() {
        let data = [0b10011000, 0b00011111, 0b10101001, 0b11101001, 0b00011010];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.read(9).unwrap(), 0b100110000);
        assert_eq!(reader.read(9).unwrap(), 0b001111110);
        assert_eq!(reader.read(9).unwrap(), 0b101001111);
        assert_eq!(reader.read(9).unwrap(), 0b010010001);
    }

    #[test]
    fn bit_writer_8() {
        let mut buf = vec![0_u8; 3];
        let mut writer = BitWriter::new(&mut buf, 8).unwrap();
        writer.write(0x01).unwrap();
        writer.write(0x02).unwrap();
        writer.write(0x03).unwrap();

        assert_eq!(buf, [0x01, 0x02, 0x03]);
    }

    #[test]
    fn bit_reader_8() {
        let data = [0x01, 0x02, 0x03];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.read(8).unwrap(), 0x01);
        assert_eq!(reader.read(8).unwrap(), 0x02);
        assert_eq!(reader.read(8).unwrap(), 0x03);
    }

    #[test]
    fn bit_writer_4() {
        let mut buf = vec![0_u8; 3];
        let mut writer = BitWriter::new(&mut buf, 4).unwrap();
        writer.write(0b1001).unwrap();
        writer.write(0b1000).unwrap();
        writer.write(0b0001).unwrap();
        writer.write(0b1111).unwrap();
        writer.write(0b1010).unwrap();
        writer.write(0b1001).unwrap();

        assert_eq!(buf, [0b10011000, 0b00011111, 0b10101001]);
    }

    #[test]
    fn bit_reader_4() {
        let data = [0b10011000, 0b00011111, 0b10101001];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.read(4).unwrap(), 0b1001);
        assert_eq!(reader.read(4).unwrap(), 0b1000);
        assert_eq!(reader.read(4).unwrap(), 0b0001);
        assert_eq!(reader.read(4).unwrap(), 0b1111);
        assert_eq!(reader.read(4).unwrap(), 0b1010);
        assert_eq!(reader.read(4).unwrap(), 0b1001);
    }

    #[test]
    fn bit_writer_2() {
        let mut buf = vec![0_u8; 2];
        let mut writer = BitWriter::new(&mut buf, 2).unwrap();
        writer.write(0b10).unwrap();
        writer.write(0b01).unwrap();
        writer.write(0b10).unwrap();
        writer.write(0b00).unwrap();
        writer.write(0b00).unwrap();
        writer.write(0b01).unwrap();
        writer.write(0b00).unwrap();
        writer.write(0b00).unwrap();

        assert_eq!(buf, [0b10011000, 0b00010000]);
    }

    #[test]
    fn bit_reader_2() {
        let data = [0b10011000, 0b00010000];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.read(2).unwrap(), 0b10);
        assert_eq!(reader.read(2).unwrap(), 0b01);
        assert_eq!(reader.read(2).unwrap(), 0b10);
        assert_eq!(reader.read(2).unwrap(), 0b00);
        assert_eq!(reader.read(2).unwrap(), 0b00);
        assert_eq!(reader.read(2).unwrap(), 0b01);
        assert_eq!(reader.read(2).unwrap(), 0b00);
        assert_eq!(reader.read(2).unwrap(), 0b00);
    }

    #[test]
    fn bit_writer_1() {
        let mut buf = vec![0_u8; 2];
        let mut writer = BitWriter::new(&mut buf, 1).unwrap();
        writer.write(0b1).unwrap();
        writer.write(0b0).unwrap();
        writer.write(0b0).unwrap();
        writer.write(0b1).unwrap();
        writer.write(0b1).unwrap();
        writer.write(0b0).unwrap();
        writer.write(0b0).unwrap();
        writer.write(0b0).unwrap();

        writer.write(0b0).unwrap();
        writer.write(0b0).unwrap();
        writer.write(0b0).unwrap();
        writer.write(0b1).unwrap();
        writer.write(0b0).unwrap();
        writer.write(0b0).unwrap();
        writer.write(0b0).unwrap();
        writer.write(0b0).unwrap();

        assert_eq!(buf, [0b10011000, 0b00010000]);
    }

    #[test]
    fn bit_reader_1() {
        let data = [0b10011000, 0b00010000];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.read(1).unwrap(), 0b1);
        assert_eq!(reader.read(1).unwrap(), 0b0);
        assert_eq!(reader.read(1).unwrap(), 0b0);
        assert_eq!(reader.read(1).unwrap(), 0b1);
        assert_eq!(reader.read(1).unwrap(), 0b1);
        assert_eq!(reader.read(1).unwrap(), 0b0);
        assert_eq!(reader.read(1).unwrap(), 0b0);
        assert_eq!(reader.read(1).unwrap(), 0b0);

        assert_eq!(reader.read(1).unwrap(), 0b0);
        assert_eq!(reader.read(1).unwrap(), 0b0);
        assert_eq!(reader.read(1).unwrap(), 0b0);
        assert_eq!(reader.read(1).unwrap(), 0b1);
        assert_eq!(reader.read(1).unwrap(), 0b0);
        assert_eq!(reader.read(1).unwrap(), 0b0);
        assert_eq!(reader.read(1).unwrap(), 0b0);
        assert_eq!(reader.read(1).unwrap(), 0b0);
    }

    #[test]
    fn bit_reader_align() {
        let data = [0b10011000, 0b00010000];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.read(1).unwrap(), 0b1);
        assert_eq!(reader.read(1).unwrap(), 0b0);
        assert_eq!(reader.read(1).unwrap(), 0b0);
        assert_eq!(reader.read(1).unwrap(), 0b1);
        reader.align();

        assert_eq!(reader.read(1).unwrap(), 0b0);
        assert_eq!(reader.read(1).unwrap(), 0b0);
        assert_eq!(reader.read(1).unwrap(), 0b0);
        assert_eq!(reader.read(1).unwrap(), 0b1);
        assert_eq!(reader.read(1).unwrap(), 0b0);
        assert_eq!(reader.read(1).unwrap(), 0b0);
        assert_eq!(reader.read(1).unwrap(), 0b0);
        assert_eq!(reader.read(1).unwrap(), 0b0);
    }

    #[test]
    fn bit_reader_chunks() {
        let data = [0b10011000, 0b00010000];
        let mut reader = BitChunks::new(&data, 1, 3).unwrap();
        assert_eq!(reader.next().unwrap().bits.as_ref(), &[0b1, 0b0, 0b0]);
        assert_eq!(reader.next().unwrap().bits.as_ref(), &[0b1, 0b1, 0b0]);
        assert_eq!(reader.next().unwrap().bits.as_ref(), &[0b0, 0b0, 0b0]);
        assert_eq!(reader.next().unwrap().bits.as_ref(), &[0b0, 0b0, 0b1]);
        assert_eq!(reader.next().unwrap().bits.as_ref(), &[0b0, 0b0, 0b0]);
    }

    #[test]
    fn bit_reader_varying_bit_sizes() {
        let data = [0b10011000, 0b00011111, 0b10101001];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.read(4).unwrap(), 0b1001);
        assert_eq!(reader.read(1).unwrap(), 0b1);
        assert_eq!(reader.read(4).unwrap(), 0b0000);
        assert_eq!(reader.read(5).unwrap(), 0b00111);
        assert_eq!(reader.read(1).unwrap(), 0b1);
        assert_eq!(reader.read(2).unwrap(), 0b11);
        assert_eq!(reader.read(7).unwrap(), 0b0101001);
    }

    #[test]
    fn bit_writer_rejects_invalid_sizes() {
        let mut buf = [0_u8; 4];
        assert!(BitWriter::new(&mut buf, 0).is_none());
        assert!(BitWriter::new(&mut buf, 33).is_none());
    }

    #[test]
    fn bit_reader_rejects_invalid_sizes() {
        let data = [0xff];
        let mut reader = BitReader::new(&data);
        assert!(reader.read(0).is_none());
        assert!(reader.read(33).is_none());
    }

    #[test]
    fn bit_writer_round_trip_all_bit_sizes() {
        for bit_size in 1_u8..=32 {
            let mask = bit_mask(bit_size);
            let sample_count = if bit_size > 16 { 4 } else { 6 };
            let values: Vec<u32> = (0..sample_count)
                .map(|i| {
                    let seed = 0x9E37_u32.wrapping_mul(i as u32 + 1) ^ (bit_size as u32 * 0x45);
                    seed & mask
                })
                .collect();

            let total_bits = bit_size as usize * values.len();
            let mut buf = vec![0_u8; total_bits.div_ceil(8)];
            let mut writer = BitWriter::new(&mut buf, bit_size).unwrap();

            for value in &values {
                writer.write(*value).unwrap();
            }

            let mut reader = BitReader::new(&buf);
            for expected in &values {
                assert_eq!(
                    reader.read(bit_size).unwrap(),
                    *expected,
                    "round-trip failed for bit size {bit_size}"
                );
            }
        }
    }

    #[test]
    fn bit_writer_round_trip_12_and_25_bits() {
        for &bit_size in &[12_u8, 25_u8] {
            let mask = bit_mask(bit_size);
            let values: Vec<u32> = (0..5)
                .map(|i| ((0xABCDE_u32.wrapping_mul(i + 1)) ^ (bit_size as u32)) & mask)
                .collect();

            let total_bits = bit_size as usize * values.len();
            let mut buf = vec![0_u8; total_bits.div_ceil(8)];
            let mut writer = BitWriter::new(&mut buf, bit_size).unwrap();

            for value in &values {
                writer.write(*value).unwrap();
            }

            let mut reader = BitReader::new(&buf);
            for expected in &values {
                assert_eq!(
                    reader.read(bit_size).unwrap(),
                    *expected,
                    "round-trip failed for bit size {bit_size}"
                );
            }
        }
    }
}
