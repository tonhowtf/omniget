//! pdfcraft-cos — the PDF object layer (L1, standalone with `pdfcraft-filters`).
//!
//! Parses the COS object graph lazily and tolerantly, keeps edits in a copy-on-write overlay
//! (cheap snapshots for undo), and writes documents back either incrementally (original bytes
//! untouched) or as a full, garbage-collected rewrite. The PDF object graph *is* PdfCraft's
//! document model (plan/architecture.md §5, ADR-0010).
//!
//! Done: every cross-reference form (tables, streams, hybrid `/XRefStm`), object streams, repair
//! by scanning, encryption on load and save (R2–R6, crypt filters, `/EFF`), incremental writing,
//! and full saves that pack objects into compressed object streams with a cross-reference stream.
//! Not yet: streaming very large files within a memory budget (`ByteSource`,
//! `core.lazy-loading`), and linearized output (`core.linearization`, M11). See the README.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod budget;
mod bytes;
mod document;
mod object;
pub mod page_labels;
mod parser;
mod security;
mod writer;

pub use budget::{DecodeBudget, ImageBudget, MAX_PAGE_DECODED, MAX_PAGE_FORMS, MAX_REDACTED_IMAGE_PIXELS};
pub use bytes::Bytes;
pub use document::{Document, ReadState, Revision, SourceIdentity, XrefEntry};
pub use object::{Dict, MAX_DECODED, MAX_JBIG2_GLOBALS, Name, ObjRef, Object, PdfString, Stream};
pub use parser::{Lexer, is_whitespace, parse_indirect};
pub use pdfcraft_crypt::{
    Algorithm, Auth, Method as CryptMethod, NewEncryption, Permissions, SecurityHandler, effective_password, pdfdoc_byte, pdfdoc_char,
};
pub use writer::{SaveOptions, pdf_date, reachable_objects, serialize, write_full, write_incremental};

#[derive(Debug, thiserror::Error, Clone, PartialEq)]
pub enum CosError {
    #[error("not a PDF file (no %PDF- header)")]
    NotPdf,
    #[error("syntax error at byte {offset}: {detail}")]
    Syntax { offset: usize, detail: String },
    #[error("object {0} is missing")]
    MissingObject(u32),
    #[error("object {0} is not a dictionary")]
    NotADictionary(ObjRef),
    #[error("stream data could not be decoded: {0}")]
    Filter(String),
    #[error("the document is protected by a password")]
    NeedsPassword,
    #[error("the password is incorrect")]
    WrongPassword,
    #[error("unsupported security: {0}")]
    Security(String),
    #[error("internal lock poisoned")]
    Poisoned,
    #[error("this document was opened read-only with a decode limit and cannot be saved")]
    ReadOnlyLimit,
}
