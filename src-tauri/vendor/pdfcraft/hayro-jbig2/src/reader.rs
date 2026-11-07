/// A reader for reading bits and bytes from a byte stream.
#[derive(Debug, Clone)]
pub(crate) struct Reader<'a> {
    /// The underlying data.
    data: &'a [u8],
    /// The position in bits.
    cur_pos: usize,
}

impl<'a> Reader<'a> {
    #[inline(always)]
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, cur_pos: 0 }
    }

    #[inline(always)]
    pub(crate) fn align(&mut self) {
        let bit_pos = self.bit_pos();

        if !bit_pos.is_multiple_of(8) {
            self.cur_pos += 8 - bit_pos;
        }
    }

    #[inline(always)]
    pub(crate) fn at_end(&self) -> bool {
        self.byte_pos() >= self.data.len()
    }

    #[inline(always)]
    pub(crate) fn tail(&self) -> Option<&'a [u8]> {
        self.data.get(self.byte_pos()..)
    }

    /// Read the given number of bytes.
    ///
    /// Assumes that the reader is currently byte-aligned.
    #[inline(always)]
    pub(crate) fn read_bytes(&mut self, len: usize) -> Option<&'a [u8]> {
        debug_assert_eq!(self.bit_pos(), 0);

        let bytes = self.peek_bytes(len)?;
        self.cur_pos += len * 8;

        Some(bytes)
    }

    /// Read a single byte.
    ///
    /// Assumes that the reader is currently byte-aligned.
    #[inline(always)]
    pub(crate) fn read_byte(&mut self) -> Option<u8> {
        debug_assert_eq!(self.bit_pos(), 0);

        let byte = self.peek_byte()?;
        self.cur_pos += 8;

        Some(byte)
    }

    /// Read a single byte, returning `None` if it's zero.
    #[inline(always)]
    pub(crate) fn read_nonzero_byte(&mut self) -> Option<u8> {
        let byte = self.read_byte()?;

        if byte == 0 { None } else { Some(byte) }
    }

    /// Skip the given number of bytes.
    ///
    /// Assumes that the reader is currently byte-aligned.
    #[inline(always)]
    pub(crate) fn skip_bytes(&mut self, len: usize) -> Option<()> {
        debug_assert_eq!(self.bit_pos(), 0);

        self.read_bytes(len).map(|_| ())
    }

    /// Peek the given number of bytes.
    ///
    /// Assumes that the reader is currently byte-aligned.
    #[inline(always)]
    pub(crate) fn peek_bytes(&self, len: usize) -> Option<&'a [u8]> {
        debug_assert_eq!(self.bit_pos(), 0);

        let start = self.byte_pos();
        let end = start.checked_add(len)?;
        self.data.get(start..end)
    }

    /// Peek the next byte.
    ///
    /// Assumes that the reader is currently byte-aligned.
    #[inline(always)]
    pub(crate) fn peek_byte(&self) -> Option<u8> {
        debug_assert_eq!(self.bit_pos(), 0);

        self.data.get(self.byte_pos()).copied()
    }

    /// Read an u16 number.
    ///
    /// Assumes that the reader is currently byte-aligned.
    #[inline(always)]
    pub(crate) fn read_u16(&mut self) -> Option<u16> {
        Some(u16::from_be_bytes(self.read_bytes(2)?.try_into().ok()?))
    }

    /// Read an u32 number.
    ///
    /// Assumes that the reader is currently byte-aligned.
    #[inline(always)]
    pub(crate) fn read_u32(&mut self) -> Option<u32> {
        Some(u32::from_be_bytes(self.read_bytes(4)?.try_into().ok()?))
    }

    /// Read an i32 number.
    ///
    /// Assumes that the reader is currently byte-aligned.
    #[inline(always)]
    pub(crate) fn read_i32(&mut self) -> Option<i32> {
        Some(i32::from_be_bytes(self.read_bytes(4)?.try_into().ok()?))
    }

    #[inline(always)]
    pub(crate) fn read_bit(&mut self) -> Option<u8> {
        let byte = self.cur_byte()?;
        let shift = 7 - (self.bit_pos());
        self.cur_pos += 1;
        Some((byte >> shift) & 1)
    }

    #[inline(always)]
    pub(crate) fn read_bits(&mut self, count: u8) -> Option<u32> {
        debug_assert!(count <= 32);

        let mut value = 0_u32;
        let mut remaining = count;

        while remaining > 0 {
            let bit_offset = self.bit_pos();
            let byte = self.cur_byte()? as u32;

            let available = (8 - bit_offset) as u8;
            let take = remaining.min(available);

            let shift = available - take;
            let mask = (1 << take) - 1;
            let bits = (byte >> shift) & mask;

            value = (value << take) | bits;
            self.cur_pos += take as usize;
            remaining -= take;
        }

        Some(value)
    }

    #[inline(always)]
    pub(crate) fn byte_pos(&self) -> usize {
        self.cur_pos >> 3
    }

    #[inline(always)]
    pub(crate) fn cur_byte(&self) -> Option<u8> {
        self.data.get(self.byte_pos()).copied()
    }

    #[inline(always)]
    pub(crate) fn bit_pos(&self) -> usize {
        self.cur_pos & 7
    }
}
