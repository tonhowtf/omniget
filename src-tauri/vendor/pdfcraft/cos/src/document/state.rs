use super::xref::XrefEntry;
use super::{Arc, BTreeMap, CosError, Dict, Document, HashMap, ObjRef, Object, Weak};

thread_local! {
    /// The objects being loaded on this thread, innermost last (see `Document::try_get`).
    static LOADING: std::cell::RefCell<Vec<u32>> = const { std::cell::RefCell::new(Vec::new()) };
    /// Indirect `/Length` values resolved during the current outermost load: a damaged file
    /// retries a parse several ways, and each retry would resolve the same lengths again
    /// (exponential in the chain of streams whose lengths point at further streams).
    pub(super) static LENGTHS: std::cell::RefCell<HashMap<ObjRef, Option<i64>>> = std::cell::RefCell::new(HashMap::new());
}

/// Marks an object as being loaded on this thread. Loading can re-enter (indirect stream
/// `/Length`, object streams): an object already being loaded is a cycle, and the nesting is
/// bounded as well.
pub(super) struct LoadGuard;

impl LoadGuard {
    const MAX: usize = 32;

    pub(super) fn enter(num: u32) -> Option<Self> {
        LOADING.with(|l| {
            let mut l = l.borrow_mut();
            if l.len() >= Self::MAX || l.contains(&num) {
                return None;
            }
            l.push(num);
            Some(LoadGuard)
        })
    }
}

impl Drop for LoadGuard {
    fn drop(&mut self) {
        let outermost = LOADING.with(|l| {
            let mut l = l.borrow_mut();
            l.pop();
            l.is_empty()
        });
        if outermost {
            LENGTHS.with(|m| m.borrow_mut().clear());
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct ObjStm {
    /// One slot per member in stream order, since cross-reference entries name a member by its
    /// position: (object number, byte offset within `data`), or `None` for a member whose number
    /// or offset is unusable ([`objstm_member`]).
    pub(super) index: Vec<Option<(u32, usize)>>,
    pub(super) data: Arc<Vec<u8>>,
}

/// Where member `num` of an object stream starts: its `offset` past `/First` (`first`), when the
/// number is an object number and the start lies inside the decoded `len` bytes. Checked on every
/// target, so a 32-bit build neither truncates nor wraps a large `/First` or offset (#1815).
pub(super) fn objstm_member(num: i64, offset: i64, first: Option<usize>, len: usize) -> Option<(u32, usize)> {
    let num = u32::try_from(num).ok()?;
    let start = first?.checked_add(usize::try_from(offset).ok()?)?;
    (start < len).then_some((num, start))
}

/// Where an overlay slot came from.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Slot {
    Set(u16, Arc<Object>),
    /// Both generations accepted from a damaged live source slot survive a repair-aware edit.
    /// A full rewrite is required: a new xref slot cannot encode two generations.
    RepairedSet(u16, u16, Arc<Object>),
    Freed(u16),
}

/// An immutable input/decode context shared by document clones. Holding this identity
/// retains neither the input bytes nor the parsed document. Weak ownership prevents an
/// expired identity from matching a later allocation at the same address.
#[derive(Clone, Debug)]
pub struct SourceIdentity(pub(super) Weak<()>);

impl PartialEq for SourceIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.0.ptr_eq(&other.0)
    }
}

impl Eq for SourceIdentity {}

/// What a document reads as: its source ([`SourceIdentity`]), its trailer and every object changed
/// since it was opened. A clone, or a snapshot that undo restores, has the same state, and every
/// change makes a different one. Changed objects are compared as allocations, which the state
/// keeps alive, so a later object can't take an address one of them had.
#[derive(Clone)]
pub struct ReadState {
    pub(super) source: SourceIdentity,
    pub(super) trailer: Dict,
    pub(super) overlay: BTreeMap<u32, Slot>,
}

impl ReadState {
    /// Whether `doc` reads as this state.
    pub fn matches(&self, doc: &Document) -> bool {
        self.source == doc.source_identity()
            && self.trailer == doc.trailer
            && self.overlay.len() == doc.overlay.len()
            && self.overlay.iter().zip(&doc.overlay).all(|((a, x), (b, y))| {
                a == b
                    && match (x, y) {
                        (Slot::Set(g, o), Slot::Set(h, p)) => g == h && Arc::ptr_eq(o, p),
                        (Slot::RepairedSet(g, a, o), Slot::RepairedSet(h, b, p)) => g == h && a == b && Arc::ptr_eq(o, p),
                        (Slot::Freed(g), Slot::Freed(h)) => g == h,
                        _ => false,
                    }
            })
    }
}

impl std::fmt::Debug for ReadState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReadState").field("changed_objects", &self.overlay.len()).finish_non_exhaustive()
    }
}

/// A temporary reader for whole-document work. Its caches never populate the editor or
/// undo snapshots. Returned objects survive cache eviction; a stream read from the file is a
/// view into the input bytes (it keeps the whole input alive while held, not a copy).
pub(crate) struct ObjectReader {
    pub(crate) document: Document,
    pub(super) decoded_count: usize,
}

impl ObjectReader {
    pub(super) const OBJECTS: usize = 128;
    pub(super) const STREAM_BYTES: usize = 8 * 1024 * 1024;

    pub(crate) fn get(&mut self, reference: ObjRef) -> Arc<Object> {
        self.try_get(reference).unwrap_or_else(|_| Arc::new(Object::Null))
    }

    pub(crate) fn try_get(&mut self, reference: ObjRef) -> Result<Arc<Object>, CosError> {
        let object = self.document.try_get_reference(reference);
        // A missed eviction here only costs a re-parse, so never fail the read on a poisoned
        // lock. The editing-cache evictors propagate instead; they guard user edits.
        if let Ok(mut streams) = self.document.objstms.lock() {
            // Only insertion changes this private cache between reads. Most neighboring
            // objects reuse a decoded stream: do not sum the whole store for every object.
            if streams.len() != self.decoded_count {
                let bytes = streams.values().fold(0usize, |bytes, stream| {
                    bytes
                        .saturating_add(stream.data.capacity())
                        .saturating_add(stream.index.capacity().saturating_mul(std::mem::size_of::<Option<(u32, usize)>>()))
                });
                if bytes > Self::STREAM_BYTES || streams.len() >= Self::OBJECTS {
                    // Retain one oversized active stream so its next object does not require
                    // decoding it again. That stream still obeys the document's decode cap.
                    let current = match self.document.xref_entry(reference.num) {
                        Some(XrefEntry::InStream { stream, .. }) => Some(stream),
                        _ => None,
                    };
                    streams.retain(|num, _| Some(*num) == current);
                }
                self.decoded_count = streams.len();
            }
        }
        // Parsed objects stay cached (an indirect /Length or a dictionary read again costs a
        // lookup, not a parse) until the cache passes its object count or holds more than
        // STREAM_BYTES of data of its own. Streams read from the file are views into the
        // input bytes, which the document keeps anyway, so they don't count; decrypted
        // streams and strings do. Checked after failed loads too: resolving a /Length may
        // have cached large objects.
        if let Ok(mut cache) = self.document.cache.lock()
            && (cache.len() >= Self::OBJECTS || cache.owned() > Self::STREAM_BYTES)
        {
            cache.clear();
        }
        object
    }
}

#[cfg(test)]
thread_local! {
    /// Cross-reference sections read on this thread.
    pub(super) static SECTION_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Parsed objects by number, each with what it holds ([`held_bytes`]) measured once, when it goes
/// in, and their sum kept beside them.
///
/// Both caches are bounded by those bytes. Summing them on every read walked every cached object
/// whole each time: a file whose page dictionaries carry kilobytes of private data (InDesign's
/// `/PieceInfo`) took seconds to open, almost all of it re-measuring objects already measured
/// (#5873).
#[derive(Default)]
pub(crate) struct ObjectCache {
    entries: HashMap<u32, (Arc<Object>, usize)>,
    owned: usize,
}

impl ObjectCache {
    pub(crate) fn get(&self, num: &u32) -> Option<&Arc<Object>> {
        self.entries.get(num).map(|(object, _)| object)
    }

    #[cfg(test)]
    pub(crate) fn contains_key(&self, num: &u32) -> bool {
        self.entries.contains_key(num)
    }

    /// Keep `object` as `num`, which holds `owned` bytes (what [`held_bytes`] says of it).
    pub(crate) fn insert(&mut self, num: u32, object: Arc<Object>, owned: usize) {
        if let Some((_, before)) = self.entries.insert(num, (object, owned)) {
            self.owned = self.owned.saturating_sub(before);
        }
        self.owned = self.owned.saturating_add(owned);
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The bytes the cached objects hold between them.
    pub(crate) fn owned(&self) -> usize {
        self.owned
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.owned = 0;
    }

    pub(crate) fn retain(&mut self, mut keep: impl FnMut(&u32) -> bool) {
        self.entries.retain(|num, _| keep(num));
        self.owned = self.entries.values().fold(0usize, |n, (_, owned)| n.saturating_add(*owned));
    }

    #[cfg(test)]
    pub(crate) fn values(&self) -> impl Iterator<Item = &Arc<Object>> {
        self.entries.values().map(|(object, _)| object)
    }
}

/// Nesting [`held_bytes`] walks: past the parser's own nesting limit, so every parsed object
/// is measured in full.
const OWNED_BYTES_DEPTH: usize = 256;

/// What keeping `object` parsed costs: string and stream data that is not a view into `data`
/// (the input file), and the parsed tree around it -- an [`Object`] per value and a key per
/// dictionary entry. An object nested deeper than [`OWNED_BYTES_DEPTH`] cannot be measured and
/// counts as too large to keep.
///
/// The tree counts. Measured by its strings and streams alone, a page dictionary carrying
/// InDesign's `/PieceInfo` -- thousands of small numeric arrays, parsed into tens of thousands of
/// values -- weighed nothing, so the caches kept a thousand of them: a 35 MB file held 1.5 GB
/// (#5873).
pub(super) fn held_bytes(object: &Object, data: &Arc<Vec<u8>>, depth: usize) -> usize {
    if depth > OWNED_BYTES_DEPTH {
        return usize::MAX;
    }
    let node = std::mem::size_of::<Object>();
    // A key, its slot beside the value, and room in the index a large dictionary keeps.
    let entry = |key: &[u8]| key.len().saturating_add(std::mem::size_of::<(crate::Name, Object)>()).saturating_add(16);
    let dict =
        |d: &crate::Dict, start: usize| d.iter().fold(start, |n, (k, v)| n.saturating_add(entry(k)).saturating_add(held_bytes(v, data, depth + 1)));
    match object {
        Object::String(s) => node.saturating_add(s.bytes.len()),
        Object::Name(n) => node.saturating_add(n.len()),
        Object::Stream(s) => dict(&s.dict, node.saturating_add(if s.raw.shares(data) { 0 } else { s.raw.len() })),
        Object::Array(a) => a.iter().fold(node, |n, v| n.saturating_add(held_bytes(v, data, depth + 1))),
        Object::Dict(d) => dict(d, node),
        _ => node,
    }
}

/// How an encrypted document is opened: with a password, or with the file key of a document
/// that has the same encryption (an earlier revision of the same file).
pub(super) enum Credentials<'a> {
    Password(Option<&'a str>),
    Handler { handler: &'a Arc<pdfcraft_crypt::SecurityHandler>, id0: Vec<u8> },
}
