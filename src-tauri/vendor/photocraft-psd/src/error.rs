//! Error type for every fallible operation in this crate.

use thiserror::Error;

/// Errors produced while parsing, decoding, encoding or writing PSD/PSB data.
///
/// Parsers never panic on malformed input; every failure is reported through
/// this type.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PsdError {
    /// The input ended before a structure was complete.
    #[error("unexpected end of data at offset {offset} (needed {needed} more bytes)")]
    UnexpectedEof {
        /// Offset (relative to the current section) where the read started.
        offset: usize,
        /// Number of bytes that were requested but not available.
        needed: usize,
    },
    /// A four-byte signature did not match.
    #[error("invalid signature {found:?}, expected {expected}")]
    InvalidSignature {
        /// Human-readable description of the expected signature(s).
        expected: &'static str,
        /// The bytes that were found.
        found: [u8; 4],
    },
    /// Header version is neither 1 (PSD) nor 2 (PSB).
    #[error("unsupported file version {0}")]
    UnsupportedVersion(u16),
    /// A dimension, count or allocation exceeded the configured safety limits.
    #[error("limit exceeded: {0}")]
    LimitExceeded(&'static str),
    /// A buffer could not be allocated: the system has no memory for it. Unlike
    /// [`PsdError::LimitExceeded`] the data may be valid, so the caller can retry smaller.
    #[error("out of memory: {0}")]
    OutOfMemory(&'static str),
    /// Structurally invalid data.
    #[error("invalid data: {0}")]
    Invalid(String),
    /// A compressed stream could not be decoded.
    #[error("decompression failed: {0}")]
    Decompress(String),
    /// Reading from a [`crate::source::ReadAt`] source failed.
    #[error("read failed: {0}")]
    Read(String),
    /// An operation is not supported for this data (e.g. unknown compression).
    #[error("unsupported: {0}")]
    Unsupported(String),
}

impl PsdError {
    pub(crate) fn invalid(msg: impl Into<String>) -> Self {
        PsdError::Invalid(msg.into())
    }

    /// The error a streaming write ([`crate::PsdFile::write_to`]) failed with: the serializer's
    /// own error when it was one, otherwise the I/O error's text.
    pub(crate) fn from_io(e: std::io::Error) -> Self {
        let text = e.to_string();
        match e.into_inner().map(|inner| inner.downcast::<PsdError>()) {
            Some(Ok(e)) => *e,
            _ => PsdError::Invalid(format!("write failed: {text}")),
        }
    }
}

/// A serializer error inside a streaming write ([`crate::PsdFile::write_to`]), as `InvalidData`.
impl From<PsdError> for std::io::Error {
    fn from(e: PsdError) -> Self {
        std::io::Error::new(std::io::ErrorKind::InvalidData, e)
    }
}

/// Convenience alias.
pub type Result<T, E = PsdError> = std::result::Result<T, E>;
