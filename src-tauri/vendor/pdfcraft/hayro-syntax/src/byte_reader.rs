//! A byte reader.

use core::cell::Cell;
use core::ops::Range;

/// PdfCraft patch: how deeply direct arrays and dictionaries may nest (the same cap as `pdfcraft-cos`; a
/// chain of 64 direct stitching functions is exactly this deep).
const MAX_CONTAINER_DEPTH: usize = 128;

/// A reader for reading bytes and PDF objects.
#[derive(Clone, Debug)]
pub struct Reader<'a> {
    /// The underlying data of the reader.
    pub data: &'a [u8],
    /// The current byte-offset.
    pub offset: usize,
    /// PdfCraft patch: set when a read needed a byte past the end of `data`. A decision made from
    /// such a read could change if more data followed (see `content::UntypedIter`).
    past_end: Cell<bool>,
    // PdfCraft patch: bound direct-object nesting while skipping arrays and dictionaries
    // and while parsing dictionaries (`Dict::skip`, `Array::skip`, `parse_dict_with`).
    skip_depth: usize,
}

impl<'a> Reader<'a> {
    /// Create a new reader.
    #[inline]
    pub fn new(data: &'a [u8]) -> Self {
        Self::new_with(data, 0)
    }

    /// Create a new reader at the given offset.
    #[inline]
    pub fn new_with(data: &'a [u8], offset: usize) -> Self {
        Self { data, offset, past_end: Cell::new(false), skip_depth: 0 }
    }

    /// PdfCraft patch: whether a read needed bytes past the end of the data since the flag was
    /// last cleared.
    #[inline]
    pub(crate) fn looked_past_end(&self) -> bool {
        self.past_end.get()
    }

    /// PdfCraft patch: clears the flag set by [`Reader::looked_past_end`].
    #[inline]
    pub(crate) fn clear_looked_past_end(&self) {
        self.past_end.set(false);
    }

    pub(crate) fn enter_container(&mut self) -> Option<()> {
        if self.skip_depth >= MAX_CONTAINER_DEPTH {
            return None;
        }
        self.skip_depth += 1;
        Some(())
    }

    pub(crate) fn leave_container(&mut self) {
        self.skip_depth -= 1;
    }

    /// Returns `true` if the reader has reached the end of the data.
    #[inline]
    pub fn at_end(&self) -> bool {
        let at_end = self.offset >= self.data.len();
        if at_end {
            // PdfCraft patch: a parser that stops at the end decides from it (see `past_end`).
            self.past_end.set(true);
        }

        at_end
    }

    /// PdfCraft patch: carries over whether `other`, a clone of this reader, looked past the end.
    #[inline]
    pub(crate) fn absorb_past_end(&self, other: &Reader<'_>) {
        if other.past_end.get() {
            self.past_end.set(true);
        }
    }

    /// Moves the reader offset to the end of the data.
    #[inline]
    pub fn jump_to_end(&mut self) {
        self.offset = self.data.len();
    }

    /// Moves the reader to the specified offset.
    #[inline]
    pub fn jump(&mut self, offset: usize) {
        self.offset = offset;
    }

    /// Returns the remaining data from the current offset to the end.
    #[inline]
    pub fn tail(&mut self) -> Option<&'a [u8]> {
        self.data.get(self.offset..)
    }

    /// Returns the total length of the underlying data.
    #[inline]
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Returns `true` if the underlying data is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Returns a slice of the data for the specified range.
    #[inline]
    pub fn range(&self, range: Range<usize>) -> Option<&'a [u8]> {
        self.data.get(range)
    }

    /// Returns the current offset of the reader.
    #[inline]
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// Reads the specified number of bytes and advances the offset.
    #[inline]
    pub fn read_bytes(&mut self, len: usize) -> Option<&'a [u8]> {
        let v = self.peek_bytes(len)?;
        self.offset += len;

        Some(v)
    }

    /// Reads a single byte and advances the offset.
    #[inline]
    pub fn read_byte(&mut self) -> Option<u8> {
        let v = self.peek_byte()?;
        self.offset += 1;

        Some(v)
    }

    /// Skips the specified number of bytes by advancing the offset.
    #[inline]
    pub fn skip_bytes(&mut self, len: usize) -> Option<()> {
        self.read_bytes(len).map(|_| {})
    }

    /// Peeks the specified number of bytes.
    #[inline]
    pub fn peek_bytes(&self, len: usize) -> Option<&'a [u8]> {
        let bytes = self
            .offset
            .checked_add(len)
            .and_then(|end| self.data.get(self.offset..end));
        if bytes.is_none() {
            self.past_end.set(true);
        }

        bytes
    }

    /// Peeks a single byte.
    #[inline]
    pub fn peek_byte(&self) -> Option<u8> {
        let byte = self.data.get(self.offset).copied();
        if byte.is_none() {
            self.past_end.set(true);
        }

        byte
    }

    /// Eat the next byte if it satisfies the condition.
    #[inline]
    pub fn eat(&mut self, f: impl Fn(u8) -> bool) -> Option<u8> {
        let val = self.peek_byte()?;
        if f(val) {
            self.forward();
            Some(val)
        } else {
            None
        }
    }

    /// Advances the offset by one byte.
    #[inline]
    pub fn forward(&mut self) {
        self.offset += 1;
    }

    /// Advances the offset by one byte if the current byte satisfies the predicate.
    #[inline]
    pub fn forward_if(&mut self, f: impl Fn(u8) -> bool) -> Option<()> {
        if f(self.peek_byte()?) {
            self.forward();

            Some(())
        } else {
            None
        }
    }

    /// Advances the offset while bytes satisfy the predicate, at least one time.
    #[inline]
    pub fn forward_while_1(&mut self, f: impl Fn(u8) -> bool) -> Option<()> {
        self.eat(&f)?;
        self.forward_while(f);
        Some(())
    }

    /// Advances the offset if the next bytes match the specified tag.
    #[inline]
    pub fn forward_tag(&mut self, tag: &[u8]) -> Option<()> {
        self.peek_tag(tag)?;
        self.offset += tag.len();

        Some(())
    }

    /// Advances the offset while the given byte satisfies the predicate.
    #[inline]
    pub fn forward_while(&mut self, f: impl Fn(u8) -> bool) {
        while let Some(b) = self.peek_byte() {
            if f(b) {
                self.forward();
            } else {
                break;
            }
        }
    }

    /// Checks if the next bytes match the specified tag.
    #[inline]
    pub fn peek_tag(&self, tag: &[u8]) -> Option<()> {
        let mut cloned = self.clone();

        for b in tag.iter().copied() {
            if cloned.peek_byte() == Some(b) {
                cloned.forward();
            } else {
                self.past_end.set(cloned.past_end.get());
                return None;
            }
        }

        Some(())
    }

    /// Read a u16 integer (in big endian order).
    #[inline]
    pub fn read_u16(&mut self) -> Option<u16> {
        let bytes = self.read_bytes(2)?;

        Some(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    /// Read a u32 integer (in big endian order).
    #[inline]
    pub fn read_u32(&mut self) -> Option<u32> {
        let bytes = self.read_bytes(4)?;

        Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// Read a u64 integer (in big endian order).
    #[inline]
    pub fn read_u64(&mut self) -> Option<u64> {
        let bytes = self.read_bytes(8)?;

        Some(u64::from_be_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }
}

#[cfg(test)]
mod tests {
    use super::Reader;

    #[test]
    fn peek_bytes_rejects_overflowing_len() {
        let reader = Reader::new(b"abc");
        assert!(reader.peek_bytes(usize::MAX).is_none());
    }
}
