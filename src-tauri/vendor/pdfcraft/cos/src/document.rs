//! Document: cross-reference data, lazy object loading, and a copy-on-write overlay of edits.
//!
//! `Document` is cheap to clone (the original bytes, xref index and parse caches are shared), so
//! a clone is a snapshot: undo/redo and background jobs hold clones while the UI edits another.
//! Edits never touch the original bytes; the writer appends them (incremental save) or rewrites
//! the file (full save).

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex, Weak};

use crate::CosError;
use crate::object::{Dict, MAX_DECODED, MAX_JBIG2_GLOBALS, ObjRef, Object, Stream};
use crate::parser::{EndstreamIndex, Lexer, is_whitespace, parse_indirect, parse_indirect_shared};

mod objects;
mod open;
mod reader;
mod state;
mod xref;

pub(crate) use objects::Held;
#[cfg(test)]
pub(crate) use reader::find;
#[cfg(test)]
use reader::rfind;
pub(crate) use state::ObjectReader;
use state::{ObjStm, Slot};
pub use state::{ReadState, SourceIdentity};
#[cfg(test)]
use state::{SECTION_READS, held_bytes, objstm_member};
use xref::XrefIndex;
pub use xref::{Revision, XrefEntry};

#[derive(Clone)]
pub struct Document {
    data: Arc<Vec<u8>>,
    /// Where `endstream` occurs in `data`; see [`EndstreamIndex`].
    endstreams: Arc<EndstreamIndex>,
    source_identity: Arc<()>,
    entries: Arc<XrefIndex>,
    trailer: Dict,
    revisions: Arc<Vec<Revision>>,
    /// See [`Self::xref_oddities`].
    xref_oddities: Arc<Vec<(u64, u32)>>,
    /// Where the file's object headers are, as (offset, generation) of the last header of each
    /// number. Filled only when the cross-reference data proved inconsistent; see
    /// `fill_xref_gaps`.
    located: Arc<BTreeMap<u32, (u64, u16)>>,
    repair_log: Arc<Vec<String>>,
    cache: Arc<Mutex<state::ObjectCache>>,
    objstms: Arc<Mutex<HashMap<u32, Arc<ObjStm>>>>,
    /// What `/JBIG2Globals` streams of the file decode to, by object number: the images of a scan
    /// share them, and each would otherwise decode them again (see `Document::jbig2_globals`).
    jbig2_globals: Arc<Mutex<HashMap<u32, Arc<Vec<u8>>>>>,
    stream_limit: usize,
    overlay: BTreeMap<u32, Slot>,
    next_num: u32,
    /// Position of the `%PDF-` header (some files have junk before it).
    header_offset: usize,
    version: String,
    /// The authenticated security handler of an encrypted document.
    security: Option<Arc<pdfcraft_crypt::SecurityHandler>>,
    /// Object number of the `/Encrypt` dictionary (never encrypted itself).
    encrypt_num: Option<u32>,
    /// What that dictionary and `/ID` refer to, which the key comes from, and so is as stored.
    encrypt_members: Arc<HashSet<u32>>,
    /// Object numbers a scan of the file didn't find: the file doesn't change, so it never will.
    unfound: Arc<Mutex<HashSet<u32>>>,
    /// Encryption was added, changed or removed since opening: only a full save can apply it.
    encryption_changed: bool,
    /// Set by edits that must not leave earlier revisions in the file (redaction).
    full_save: bool,
    /// The next full save also gives the file a new identifier (`/ID`), see
    /// [`Document::require_full_save_with_new_id`].
    new_id: bool,
    /// The handler and `/Encrypt` object number that saves use, when encryption changed.
    out_security: Option<Arc<pdfcraft_crypt::SecurityHandler>>,
    out_encrypt_num: Option<u32>,
}

impl std::fmt::Debug for Document {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Document").field("bytes", &self.data.len()).field("xref", &self.entries).field("edits", &self.overlay.len()).finish()
    }
}

#[cfg(test)]
mod tests;
