//! Stream data that can share a larger buffer instead of copying out of it.

use std::ops::{Deref, Range};
use std::sync::Arc;

/// A stream's bytes: a buffer of their own, or a range of a larger shared buffer. Streams read
/// from a file point into the file's bytes, which the document keeps anyway, so caching a
/// parsed stream costs its dictionary and not a second copy of its data.
#[derive(Clone, Default)]
pub struct Bytes {
    buf: Arc<Vec<u8>>,
    range: Range<usize>,
}

impl Bytes {
    /// `range` of `buf`, without copying; `None` when the range is not within `buf`.
    pub(crate) fn view(buf: &Arc<Vec<u8>>, range: Range<usize>) -> Option<Self> {
        (range.start <= range.end && range.end <= buf.len()).then(|| Self { buf: buf.clone(), range })
    }

    /// Whether these bytes keep alive a larger buffer than themselves that isn't `owner`.
    pub(crate) fn borrows_other_than(&self, owner: &Arc<Vec<u8>>) -> bool {
        self.range.len() < self.buf.len() && !Arc::ptr_eq(&self.buf, owner)
    }

    /// Whether these bytes are (a view into) `buf`, holding no memory of their own.
    pub(crate) fn shares(&self, buf: &Arc<Vec<u8>>) -> bool {
        Arc::ptr_eq(&self.buf, buf)
    }

    /// The same bytes, in a buffer of their own.
    pub(crate) fn detached(&self) -> Self {
        Self::from(self.to_vec())
    }
}

impl From<Vec<u8>> for Bytes {
    fn from(v: Vec<u8>) -> Self {
        let range = 0..v.len();
        Self { buf: Arc::new(v), range }
    }
}

impl From<Arc<Vec<u8>>> for Bytes {
    fn from(buf: Arc<Vec<u8>>) -> Self {
        let range = 0..buf.len();
        Self { buf, range }
    }
}

impl Deref for Bytes {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        // `view` and `From` only build in-bounds ranges; an empty slice keeps this panic-free.
        self.buf.get(self.range.clone()).unwrap_or_default()
    }
}

impl AsRef<[u8]> for Bytes {
    fn as_ref(&self) -> &[u8] {
        self
    }
}

impl PartialEq for Bytes {
    fn eq(&self, other: &Self) -> bool {
        (Arc::ptr_eq(&self.buf, &other.buf) && self.range == other.range) || **self == **other
    }
}

impl Eq for Bytes {}

impl std::fmt::Debug for Bytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        (**self).fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn views_share_their_buffer_and_compare_by_content() {
        let file = Arc::new(b"0123456789".to_vec());
        let view = Bytes::view(&file, 2..5).expect("in range");
        assert_eq!(&*view, b"234");
        assert_eq!(view.as_ptr(), file[2..].as_ptr(), "no copy");
        assert_eq!(view, Bytes::from(b"234".to_vec()));
        assert!(Bytes::view(&file, 5..11).is_none() && Bytes::view(&file, Range { start: 6, end: 5 }).is_none());
        let other = Arc::new(Vec::new());
        assert!(view.borrows_other_than(&other) && !view.borrows_other_than(&file));
        let own = view.detached();
        assert!(!own.borrows_other_than(&other) && own == view && own.as_ptr() != view.as_ptr());
        assert!(!Bytes::from(file.clone()).borrows_other_than(&other), "a whole buffer is its own");
    }
}
