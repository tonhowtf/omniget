//! Random-access byte sources, for parsing a file without reading all of it.
//!
//! [`PsdFile::from_source`](crate::PsdFile::from_source) reads the structure of a file (header,
//! resources, layer records, tagged blocks) and leaves the bulk of it, the encoded channel data
//! and the merged image, where it is: each [`ChannelData`](crate::ChannelData) records a
//! [`Stored`] range instead of a copy, and the bytes are read when the channel is decoded. A
//! 2 GB layered file then never needs 2 GB of memory just to be opened.

use std::borrow::Cow;
use std::fmt;
use std::sync::Arc;

use crate::error::{PsdError, Result};

/// Bytes that can be read at any offset, from any thread.
///
/// Implemented for byte buffers and, on native targets, for [`std::fs::File`] (as
/// [`FileSource`]). The source must not change while a file parsed from it is in use: a
/// [`Stored`] range is an offset into it.
pub trait ReadAt: Send + Sync {
    /// Total length in bytes.
    fn len(&self) -> u64;

    /// `true` when the source holds no bytes.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Fills `buf` with the bytes at `offset..offset + buf.len()`. Fails, without a partial
    /// result, when the range is not all inside the source or the read fails.
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<()>;
}

fn out_of_range(offset: u64, needed: usize, len: u64) -> PsdError {
    PsdError::UnexpectedEof {
        offset: usize::try_from(offset).unwrap_or(usize::MAX),
        needed: needed.saturating_sub(usize::try_from(len.saturating_sub(offset)).unwrap_or(usize::MAX)),
    }
}

impl ReadAt for [u8] {
    fn len(&self) -> u64 {
        <[u8]>::len(self) as u64
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<()> {
        let src = usize::try_from(offset)
            .ok()
            .and_then(|start| self.get(start..start.checked_add(buf.len())?))
            .ok_or_else(|| out_of_range(offset, buf.len(), ReadAt::len(self)))?;
        buf.copy_from_slice(src);
        Ok(())
    }
}

impl ReadAt for Vec<u8> {
    fn len(&self) -> u64 {
        self.as_slice().len() as u64
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<()> {
        self.as_slice().read_at(offset, buf)
    }
}

impl<T: ReadAt + ?Sized> ReadAt for Arc<T> {
    fn len(&self) -> u64 {
        (**self).len()
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<()> {
        (**self).read_at(offset, buf)
    }
}

/// Largest single read from a [`FileSource`].
#[cfg(any(unix, windows))]
const MAX_READ: usize = 64 << 20;

/// An open file read at offsets (`pread` on Unix, `seek_read` on Windows), with its length taken
/// when it was opened. Reads from several threads at once need no lock.
#[cfg(any(unix, windows))]
#[derive(Debug)]
pub struct FileSource {
    file: std::fs::File,
    len: u64,
}

#[cfg(any(unix, windows))]
impl FileSource {
    /// Opens `path` for reading.
    pub fn open(path: &std::path::Path) -> std::io::Result<Self> {
        Self::new(std::fs::File::open(path)?)
    }

    /// Wraps an open file.
    pub fn new(file: std::fs::File) -> std::io::Result<Self> {
        let len = file.metadata()?.len();
        Ok(FileSource { file, len })
    }

    /// The file.
    pub fn file(&self) -> &std::fs::File {
        &self.file
    }
}

#[cfg(any(unix, windows))]
impl ReadAt for FileSource {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<()> {
        if offset.checked_add(buf.len() as u64).is_none_or(|end| end > self.len) {
            return Err(out_of_range(offset, buf.len(), self.len));
        }
        let n = buf.len();
        let failed = |e: std::io::Error| PsdError::Read(format!("reading {n} bytes at offset {offset}: {e}"));
        // Bounded reads, as `photocraft-format`'s `read_file` makes them: some Windows file
        // systems reject a single read of gigabytes (#375).
        let mut done = 0usize;
        while done < n {
            let Some(rest) = buf.get_mut(done..n.min(done + MAX_READ)) else { break };
            let at = offset + done as u64;
            #[cfg(unix)]
            let read = {
                use std::os::unix::fs::FileExt;
                self.file.read_at(rest, at)
            };
            #[cfg(windows)]
            let read = {
                use std::os::windows::fs::FileExt;
                self.file.seek_read(rest, at)
            };
            match read {
                Ok(0) => return Err(failed(std::io::ErrorKind::UnexpectedEof.into())),
                Ok(k) => done += k,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(failed(e)),
            }
        }
        Ok(())
    }
}

/// A shared [`ReadAt`]: what [`Stored`] ranges point into. Two handles are equal when they are
/// the same source.
#[derive(Clone)]
pub struct Source(Arc<dyn ReadAt>);

impl Source {
    /// Shares `source`.
    pub fn new(source: impl ReadAt + 'static) -> Self {
        Source(Arc::new(source))
    }

    /// Shares an already shared source.
    pub fn from_arc(source: Arc<dyn ReadAt>) -> Self {
        Source(source)
    }

    /// The source.
    pub fn get(&self) -> &dyn ReadAt {
        &*self.0
    }

    /// Reads `len` bytes at `offset` into a new buffer. The buffer is reserved fallibly: a
    /// range larger than memory is an error, not an abort.
    pub fn read_vec(&self, offset: u64, len: u64) -> Result<Vec<u8>> {
        let n = usize::try_from(len).map_err(|_| PsdError::LimitExceeded("length exceeds address space"))?;
        if offset.checked_add(len).is_none_or(|end| end > self.0.len()) {
            return Err(out_of_range(offset, n, self.0.len()));
        }
        let mut out = Vec::new();
        out.try_reserve_exact(n).map_err(|_| PsdError::OutOfMemory("not enough memory to read the data"))?;
        out.resize(n, 0);
        self.0.read_at(offset, &mut out)?;
        Ok(out)
    }
}

impl fmt::Debug for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Source({} bytes)", self.0.len())
    }
}

impl PartialEq for Source {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for Source {}

/// Encoded bytes left in their [`Source`] by a lazy parse: `len` bytes at `offset`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stored {
    /// Where the bytes are.
    pub source: Source,
    /// Offset of the first byte.
    pub offset: u64,
    /// Number of bytes.
    pub len: u64,
}

impl Stored {
    /// Reads the bytes.
    pub fn read(&self) -> Result<Vec<u8>> {
        self.source.read_vec(self.offset, self.len)
    }

    /// Reads `len` bytes starting `at` bytes into the range.
    pub fn read_part(&self, at: u64, len: u64) -> Result<Vec<u8>> {
        if at.checked_add(len).is_none_or(|end| end > self.len) {
            return Err(PsdError::invalid("read past the end of stored data"));
        }
        self.source.read_vec(self.offset + at, len)
    }
}

/// The bytes of a value that is either in memory (`data`) or still in its source (`stored`).
pub(crate) fn encoded<'a>(data: &'a [u8], stored: Option<&Stored>) -> Result<Cow<'a, [u8]>> {
    match stored {
        Some(s) => s.read().map(Cow::Owned),
        None => Ok(Cow::Borrowed(data)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slice_reads_are_bounds_checked() {
        let v = vec![1u8, 2, 3, 4];
        let mut b = [0u8; 2];
        v.read_at(2, &mut b).unwrap();
        assert_eq!(b, [3, 4]);
        assert!(v.read_at(3, &mut b).is_err());
        assert!(v.read_at(u64::MAX, &mut b).is_err());
        let s = Source::new(v);
        assert_eq!(s.read_vec(1, 2).unwrap(), vec![2, 3]);
        assert!(s.read_vec(1, u64::MAX).is_err());
        let st = Stored { source: s.clone(), offset: 1, len: 3 };
        assert_eq!(st.read().unwrap(), vec![2, 3, 4]);
        assert_eq!(st.read_part(1, 2).unwrap(), vec![3, 4]);
        assert!(st.read_part(2, 2).is_err());
        assert_eq!(st.source, s);
        assert_ne!(st.source, Source::new(vec![1u8, 2, 3, 4]));
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn file_source_reads_at_offsets() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("photocraft-psd-source-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("f.bin");
        std::fs::File::create(&path).unwrap().write_all(&[9, 8, 7, 6, 5]).unwrap();
        let f = FileSource::open(&path).unwrap();
        assert_eq!(ReadAt::len(&f), 5);
        let mut b = [0u8; 3];
        f.read_at(2, &mut b).unwrap();
        assert_eq!(b, [7, 6, 5]);
        assert!(f.read_at(3, &mut b).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
