use std::collections::HashSet;

use super::reader::find;
use super::state::{LENGTHS, LoadGuard, ObjStm, ObjectReader, ReadState, Slot, SourceIdentity, held_bytes, objstm_member};
use super::xref::{Revision, XrefEntry, XrefIndex};
use super::{
    Arc, CosError, Cow, Dict, Document, Lexer, MAX_DECODED, MAX_JBIG2_GLOBALS, ObjRef, Object, Stream, is_whitespace, parse_indirect_shared,
};

/// The most decoded `/JBIG2Globals` a document keeps for its images to share (see
/// `Document::jbig2_globals`): two streams at [`MAX_JBIG2_GLOBALS`].
const JBIG2_GLOBALS_HELD: usize = 2 * MAX_JBIG2_GLOBALS;

impl Document {
    pub fn bytes(&self) -> &Arc<Vec<u8>> {
        &self.data
    }

    /// Identity of the original bytes, xref and authenticated decode context. Edits and
    /// clones preserve it; reopening (including after any save) produces a new identity.
    pub fn source_identity(&self) -> SourceIdentity {
        SourceIdentity(Arc::downgrade(&self.source_identity))
    }

    /// What the document reads as now (see [`ReadState`]): unlike [`Self::source_identity`], it
    /// changes with every edit, so a value derived from the document can be cached against it.
    pub fn read_state(&self) -> ReadState {
        ReadState { source: self.source_identity(), trailer: self.trailer.clone(), overlay: self.overlay.clone() }
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    /// Set the PDF header version (`"1.4"`, `"1.7"`, `"2.0"`, …) written by a full save.
    pub fn set_version(&mut self, version: impl Into<String>) {
        self.version = version.into();
    }

    pub fn trailer(&self) -> &Dict {
        &self.trailer
    }

    pub fn trailer_mut(&mut self) -> &mut Dict {
        &mut self.trailer
    }

    pub fn revisions(&self) -> &[Revision] {
        &self.revisions
    }

    /// Position of the `%PDF-` header in the file: 0 unless bytes come before it.
    pub fn header_offset(&self) -> usize {
        self.header_offset
    }

    /// There is no cross-reference chain to append to: the index was rebuilt from object
    /// headers because the file's own was unusable (see [`Self::repair_log`]), or the document
    /// was built from scratch ([`Self::new_empty`]). A save writes such a document in full, even
    /// when nothing was edited: for a repaired file that is what puts the repair on disk.
    pub fn reconstructed(&self) -> bool {
        self.revisions.is_empty()
    }

    /// Where each revision of the file ends (oldest first): the byte after the `%%EOF` (and its
    /// end-of-line) that closes its cross-reference section. `data[..end]` is that revision as it
    /// was saved. A linearized file's first-page section belongs to the revision it precedes, so
    /// ends only ever increase. Empty for a reconstructed file.
    pub fn revision_ends(&self) -> Vec<usize> {
        let d = &self.data[..];
        let mut ends: Vec<usize> = Vec::with_capacity(self.revisions.len());
        for r in self.revisions.iter() {
            let from = (r.xref_offset as usize).saturating_add(self.header_offset).min(d.len());
            let Some(at) = d[from..].windows(5).position(|w| w == b"%%EOF") else { continue };
            let mut end = from + at + 5;
            while end < d.len() && matches!(d[end], b'\r' | b'\n') && end - (from + at + 5) < 2 {
                end += 1;
            }
            match ends.last_mut() {
                Some(last) if end <= *last => {}
                _ => ends.push(end),
            }
        }
        ends
    }

    pub fn repair_log(&self) -> &[String] {
        &self.repair_log
    }

    /// Add a line to [`Self::repair_log`]: a leniency applied while reading that the user should
    /// know about (fidelity: never silent).
    pub fn note_repair(&mut self, line: String) {
        let mut log = self.repair_log.as_ref().clone();
        log.push(line);
        self.repair_log = Arc::new(log);
    }

    /// `true` when there are unsaved edits. Removing an `/Encrypt` dictionary written inline in
    /// the trailer (as MuPDF writes it) changes no object, and is an edit all the same.
    pub fn is_modified(&self) -> bool {
        !self.overlay.is_empty() || self.encryption_changed
    }

    /// Object numbers changed since the document was opened (or last saved).
    pub fn modified_objects(&self) -> Vec<u32> {
        self.overlay.keys().copied().collect()
    }

    /// Where object `num` is stored in the file (ignoring unsaved edits).
    pub fn xref_entry(&self, num: u32) -> Option<XrefEntry> {
        self.entries.lookup(num, &self.data)
    }

    /// Cross-reference entries this reader did not take at face value, as (byte offset of the
    /// section, object number): an in-use entry at offset 0, a second and different entry for
    /// one object in the same section, a reserved cross-reference stream entry type, or a
    /// hybrid /XRefStm entry that disagrees with the table. The reader sets such an entry aside
    /// and keeps what it already had for the object. Another reader, the page renderer among
    /// them, may follow it instead, so the two can then show different objects for one number.
    /// Code that compares revisions (signature validation) must not call such an object
    /// unchanged.
    pub fn xref_oddities(&self) -> &[(u64, u32)] {
        &self.xref_oddities
    }

    /// Whether the cross-reference entry of `num` locates that object itself: its `num G obj`
    /// header at exactly the recorded offset, or the member with that number at the recorded
    /// position of its object stream. When it does not, [`Self::get`] still finds the object by
    /// searching, but another reader may resolve the number differently. Ignores unsaved edits.
    pub fn xref_entry_is_exact(&self, num: u32) -> bool {
        match self.xref_entry(num) {
            None | Some(XrefEntry::Free { .. }) => true,
            Some(XrefEntry::InFile { offset, generation }) => {
                let at = |off: usize| matches!(parse_indirect_shared(&self.data, off, &|_| None, &self.endstreams), Ok((id, _)) if id == ObjRef::new(num, generation));
                // Offsets may count from a header that does not start the file.
                usize::try_from(offset).is_ok_and(|off| at(off) || (self.header_offset != 0 && at(off.saturating_add(self.header_offset))))
            }
            Some(XrefEntry::InStream { stream, index }) => self
                .objstm(stream)
                .is_ok_and(|stm| usize::try_from(index).ok().and_then(|i| stm.index.get(i)).is_some_and(|m| m.is_some_and(|(n, _)| n == num))),
        }
    }

    /// Object `num` has an unsaved edit.
    pub fn is_edited(&self, num: u32) -> bool {
        self.overlay.contains_key(&num)
    }

    pub fn root(&self) -> Option<ObjRef> {
        self.trailer.reference(b"Root")
    }

    // ── object access ───────────────────────────────────────────────────────────────────────

    /// Fetch an object by reference. Undefined references are `Null` (ISO 32000-2 §7.3.10).
    /// A live file slot also accepts its actual header generation when a damaged xref row
    /// disagrees; the disagreement is recorded in [`Self::repair_log`]. Free slots never do.
    pub fn get(&self, r: ObjRef) -> Arc<Object> {
        self.try_get_reference(r).unwrap_or_else(|_| Arc::new(Object::Null))
    }

    pub(super) fn reference_matches(&self, r: ObjRef) -> bool {
        match self.overlay.get(&r.num) {
            Some(Slot::Set(generation, _)) => r.generation == *generation,
            Some(Slot::RepairedSet(generation, alias, _)) => r.generation == *generation || r.generation == *alias,
            Some(Slot::Freed(_)) => false,
            None => match self.entries.lookup(r.num, &self.data) {
                Some(XrefEntry::InFile { offset, generation }) => {
                    r.generation == generation || self.indexed_header_reference(r.num, offset) == Some(r)
                }
                Some(XrefEntry::InStream { .. }) => r.generation == 0,
                None | Some(XrefEntry::Free { .. }) => false,
            },
        }
    }

    /// The generations a reference to `num` may carry and still resolve (see [`Self::get`]),
    /// sorted: none for a free or unknown slot, else the slot's generation, and for a damaged
    /// file slot (or a repair-aware edit of one) also the generation its object header carries.
    /// Code comparing revisions (signature validation) uses it to tell whether the references
    /// that reached an object in one revision still reach the same slot in another.
    pub fn accepted_generations(&self, num: u32) -> Vec<u16> {
        // Mirrors `reference_matches`, which stays allocation-free for every lookup.
        let mut generations = match self.overlay.get(&num) {
            Some(Slot::Set(generation, _)) => vec![*generation],
            Some(Slot::RepairedSet(generation, alias, _)) => vec![*generation, *alias],
            Some(Slot::Freed(_)) => Vec::new(),
            None => match self.entries.lookup(num, &self.data) {
                Some(XrefEntry::InFile { offset, generation }) => {
                    std::iter::once(generation).chain(self.indexed_header_reference(num, offset).map(|h| h.generation)).collect()
                }
                Some(XrefEntry::InStream { .. }) => vec![0],
                None | Some(XrefEntry::Free { .. }) => Vec::new(),
            },
        };
        generations.sort_unstable();
        generations.dedup();
        generations
    }

    pub(super) fn try_get_reference(&self, r: ObjRef) -> Result<Arc<Object>, CosError> {
        if !self.reference_matches(r) {
            return Ok(Arc::new(Object::Null));
        }
        self.try_get(r.num)
    }

    /// A repair probe, not a whole-file search: only the indexed header (or the same offset
    /// relative to a shifted PDF header) can establish this additional generation. A small
    /// window bounds hostile whitespace/tokens; ordinary xref-matching reads are unchanged.
    pub(super) fn indexed_header_reference(&self, num: u32, offset: u64) -> Option<ObjRef> {
        let at = |off: usize| {
            let bytes = self.data.get(off..off.saturating_add(128).min(self.data.len()))?;
            let mut lexer = Lexer::new(bytes, 0);
            lexer.skip_ws();
            let number = std::str::from_utf8(lexer.token()).ok()?.parse::<u32>().ok()?;
            lexer.skip_ws();
            let generation = std::str::from_utf8(lexer.token()).ok()?.parse::<u16>().ok()?;
            (number == num && lexer.eat_keyword(b"obj")).then_some(ObjRef::new(number, generation))
        };
        let off = usize::try_from(offset).ok()?;
        at(off).or_else(|| if self.header_offset == 0 { None } else { off.checked_add(self.header_offset).and_then(at) })
    }

    /// Add at most one bounded diagnostic per object. False stops diagnostic probing only;
    /// parsers must still read the remaining xref rows needed to build their index.
    pub(super) fn inspect_xref_generation_repair(&self, num: u32, entry: XrefEntry, recorded: &mut HashSet<u32>, log: &mut Vec<String>) -> bool {
        let XrefEntry::InFile { offset, generation } = entry else { return true };
        if recorded.contains(&num) {
            return true;
        }
        let Some(header) = self.indexed_header_reference(num, offset) else { return true };
        if header.generation == generation {
            return true;
        }
        if recorded.len() == 256 {
            log.push("additional xref/header generation disagreements omitted after 256 objects".into());
            return false;
        }
        recorded.insert(num);
        log.push(format!("object {num}: xref generation {generation} disagrees with object header generation {}", header.generation));
        true
    }

    /// Diagnose once at open without expanding compact ranges or decoding an xref twice.
    /// Materialized readers retain bounded notes while they read accepted sections, before
    /// newer rows hide older ones. Discarded parse attempts/rebuilt indexes carry no notes.
    pub(super) fn note_xref_generation_repairs(&self, log: &mut Vec<String>) {
        match &*self.entries {
            XrefIndex::Materialized(_, notes) => log.extend(notes.iter().cloned()),
            XrefIndex::Compact { revisions, .. } => {
                let mut recorded = HashSet::new();
                for revision in revisions {
                    for section in std::iter::once(&revision.primary).chain(revision.hybrid.iter()) {
                        for num in section.numbers() {
                            if let Some(entry) = section.lookup(num, &self.data)
                                && !self.inspect_xref_generation_repair(num, entry, &mut recorded, log)
                            {
                                return;
                            }
                        }
                    }
                }
            }
        }
    }

    /// Follow a reference if `o` is one; otherwise return `o` itself. An object can be nothing but
    /// a reference to another, which is followed too, up to [`MAX_REFERENCE_HOPS`] of them.
    pub fn resolve(&self, o: &Object) -> Arc<Object> {
        let Object::Ref(r) = o else { return Arc::new(o.clone()) };
        let mut held = self.get(*r);
        for _ in 0..MAX_REFERENCE_HOPS {
            let Object::Ref(r) = &*held else { break };
            held = self.get(*r);
        }
        held
    }

    /// `s` with the `/DecodeParms` its filters use made direct, so [`Stream::decoded`] reads them:
    /// an indirect dictionary resolved, and of an array (itself perhaps indirect), the entries at
    /// the filters' places (`/Crypt` counts, ISO 32000-2 §7.4.1) resolved, the rest dropped. Of
    /// each dictionary only the entries the decoders read are kept, numbers and flags given by
    /// reference (`/Predictor 7 0 R`) resolved; `/JBIG2Globals` stays a reference. A `/Crypt`
    /// entry, which no decoder reads ([`Stream::filters`] leaves it out), is null. `s` itself
    /// when none of those is indirect, or when the other filters are more than any decoder takes
    /// ([`pdfcraft_filters::MAX_CHAIN`]). Only small values are copied, so a file can't have a
    /// large object copied once for each key and filter.
    pub fn with_direct_parms<'s>(&self, s: &'s Stream) -> Cow<'s, Stream> {
        let Some(p) = s.dict.get(b"DecodeParms") else { return Cow::Borrowed(s) };
        let crypt = |f: &Object| matches!(f.as_name(), Some(b"Crypt"));
        let filters: Vec<bool> = match s.dict.get(b"Filter") {
            Some(Object::Array(a)) => a.iter().map(crypt).collect(),
            Some(f) => vec![crypt(f)],
            None => Vec::new(),
        };
        if filters.iter().filter(|c| !**c).count() > pdfcraft_filters::MAX_CHAIN {
            return Cow::Borrowed(s);
        }
        let mut changed = matches!(p, Object::Ref(_));
        // Its numbers and flags kept, resolved; anything else is no filter's parameters (null).
        let one = |e: &Object, changed: &mut bool| -> Object {
            *changed |= matches!(e, Object::Ref(_));
            let Object::Dict(d) = &*self.held(e) else { return Object::Null };
            let mut made = Dict::new();
            for key in DECODE_PARMS_KEYS {
                let Some(v) = d.get(key) else { continue };
                *changed |= matches!(v, Object::Ref(_));
                let v = self.held(v);
                if matches!(*v, Object::Int(_) | Object::Real(_) | Object::Bool(_)) {
                    made.set(key.to_vec(), (*v).clone());
                }
            }
            if let Some(g @ Object::Ref(_)) = d.get(b"JBIG2Globals") {
                made.set(b"JBIG2Globals".to_vec(), g.clone());
            }
            Object::Dict(made)
        };
        let made = match &*self.held(p) {
            Object::Array(a) => {
                Object::Array(a.iter().zip(&filters).map(|(e, crypt)| if *crypt { Object::Null } else { one(e, &mut changed) }).collect())
            }
            _ => one(p, &mut changed),
        };
        if !changed {
            return Cow::Borrowed(s);
        }
        let mut s = s.clone();
        s.dict.set(b"DecodeParms".to_vec(), made);
        Cow::Owned(s)
    }

    /// `o`, or the object it refers to, without copying either.
    pub(crate) fn held<'o>(&self, o: &'o Object) -> Held<'o> {
        match o {
            Object::Ref(_) => Held::Resolved(self.resolve(o)),
            o => Held::Direct(o),
        }
    }

    /// Resolve and return a dictionary (a stream's dictionary counts).
    pub fn dict(&self, o: &Object) -> Option<Dict> {
        self.resolve(o).as_dict().cloned()
    }

    /// The CCITT parameters of image `s` when its last filter is `CCITTFaxDecode`: its
    /// `/DecodeParms` (resolved; the dictionary of a lone filter, or its entry in an array), with the
    /// defaults of ISO 32000-2 Table 11 for what that leaves out. `None` for another image, and for
    /// one whose `/Filter` isn't a name or an array of names.
    pub fn fax_params(&self, s: &Stream) -> Option<pdfcraft_filters::CcittParams> {
        let chain = s.readable_filters().ok()?;
        if !matches!(chain.last(), Some((pdfcraft_filters::Filter::CcittFax, _))) {
            return None;
        }
        let parms = self.last_filter_parms(s);
        let int = |key: &[u8], default: i64| parms.get(key).and_then(|v| self.resolve(v).as_int()).unwrap_or(default);
        let flag = |key: &[u8], default: bool| match parms.get(key).map(|v| self.resolve(v)).as_deref() {
            Some(Object::Bool(b)) => *b,
            _ => default,
        };
        let d = pdfcraft_filters::CcittParams::default();
        Some(pdfcraft_filters::CcittParams {
            k: int(b"K", d.k),
            columns: int(b"Columns", d.columns),
            rows: int(b"Rows", d.rows),
            end_of_line: flag(b"EndOfLine", d.end_of_line),
            encoded_byte_align: flag(b"EncodedByteAlign", d.encoded_byte_align),
            end_of_block: flag(b"EndOfBlock", d.end_of_block),
            black_is_1: flag(b"BlackIs1", d.black_is_1),
        })
    }

    /// The `/DecodeParms` dictionary of stream `s`'s last filter, resolved: that of a lone filter,
    /// or its entry in an array. Empty when there is none.
    fn last_filter_parms(&self, s: &Stream) -> Dict {
        // Its place in /Filter, /Crypt included, pairs it with its /DecodeParms entry (§7.4.1): the
        // last filter but a /Crypt, which `filters` leaves out wherever it stands.
        let index = match s.dict.get(b"Filter") {
            Some(Object::Array(a)) => a.iter().rposition(|f| f.as_name() != Some(b"Crypt".as_slice())).unwrap_or(0),
            _ => 0,
        };
        match s.dict.get(b"DecodeParms").map(|p| self.resolve(p)).as_deref() {
            Some(Object::Array(a)) => a.get(index).and_then(|p| self.dict(p)),
            Some(p) if index == 0 => p.as_dict().cloned(),
            _ => None,
        }
        .unwrap_or_default()
    }

    /// [`Document::decoded_image_spending`] with no limit on the work, for tests of what an image
    /// decodes to: everything else decodes images through an [`ImageBudget`](crate::ImageBudget).
    #[cfg(test)]
    pub(crate) fn decoded_image(&self, s: &Stream) -> Result<Vec<u8>, CosError> {
        self.decoded_image_within(s, MAX_DECODED)
    }

    /// [`Document::decoded_image`] within `max` bytes.
    #[cfg(test)]
    pub(crate) fn decoded_image_within(&self, s: &Stream, max: usize) -> Result<Vec<u8>, CosError> {
        let mut unbounded = u64::MAX;
        self.decoded_image_spending(s, max, &mut unbounded)
    }

    /// The samples of image `s`, decoded within `max` bytes ([`MAX_DECODED`] at most), spending from
    /// `work`: see [`ImageBudget::decode`](crate::ImageBudget::decode), the way in from outside this
    /// crate, which says what this decodes and how.
    pub(crate) fn decoded_image_spending(&self, s: &Stream, max: usize, work: &mut u64) -> Result<Vec<u8>, CosError> {
        let max = max.min(MAX_DECODED);
        let s = &*self.with_direct_parms(s);
        let filter = |e: pdfcraft_filters::FilterError| CosError::Filter(e.to_string());
        let chain = s.readable_filters().map_err(filter)?;
        // The filters before the image codec, such as ASCII85Decode, are undone first.
        let undo = |before: &[(pdfcraft_filters::Filter, pdfcraft_filters::Params)], work: &mut u64| match before {
            [] => Ok(Cow::Borrowed(&s.raw[..])),
            _ => pdfcraft_filters::decode_tolerant_within(before, &s.raw, max, work).map(Cow::Owned).map_err(filter),
        };
        // `/Width` and `/Height` are read as given directly, as every caller reads them first: one
        // given by reference has the image refused there, before it gets here.
        let height = s.dict.int(b"Height").and_then(|h| u32::try_from(h).ok());
        if let Some(((pdfcraft_filters::Filter::Jbig2, _), before)) = chain.split_last() {
            let (Some(columns), Some(height)) = (s.dict.int(b"Width").and_then(|w| u32::try_from(w).ok()), height) else {
                return Err(CosError::Filter("a JBIG2 image's /Width or /Height isn't a size".into()));
            };
            // Nothing to read for what can't be decoded: the images of a file share its globals.
            let globals = if *work == 0 { None } else { self.jbig2_globals(s, work)? };
            let data = if *work == 0 { Cow::Borrowed(&[][..]) } else { undo(before, work)? };
            return pdfcraft_filters::decode_jbig2_within(&data, globals.as_deref().map(Vec::as_slice), columns, height, max, work).map_err(filter);
        }
        let Some(p) = self.fax_params(s) else { return s.decoded_spending(max, work) };
        // As many rows as the image has: a /Rows below /Height would cut the picture short, and
        // rows past /Height aren't drawn.
        let p = pdfcraft_filters::CcittParams { rows: 0, ..p };
        if s.dict.int(b"Width") != Some(p.columns) {
            return Err(CosError::Filter("a fax image's /Columns aren't its /Width".into()));
        }
        let stride = usize::try_from(p.columns).unwrap_or(usize::MAX).div_ceil(8).max(1);
        let rows = u32::try_from(max / stride).unwrap_or(u32::MAX).min(height.unwrap_or(0));
        let data = undo(chain.split_last().map_or(&[][..], |(_, before)| before), work)?;
        pdfcraft_filters::decode_ccitt_within(&data, &p, rows, work).map_err(filter)
    }

    /// The `/JBIG2Globals` stream a JBIG2 image `s` names by reference in its last filter's
    /// `/DecodeParms`: `None` for a stream whose last filter isn't `JBIG2Decode`, or one without
    /// globals. Redaction finds with it the other images that share a redacted image's globals.
    pub fn jbig2_globals_ref(&self, s: &Stream) -> Option<ObjRef> {
        let chain = s.readable_filters().ok()?;
        if !matches!(chain.last(), Some((pdfcraft_filters::Filter::Jbig2, _))) {
            return None;
        }
        self.last_filter_parms(s).get(b"JBIG2Globals").and_then(Object::as_ref)
    }

    /// What the `/JBIG2Globals` stream of JBIG2 image `s` (in its last filter's `/DecodeParms`)
    /// decodes to, within [`MAX_JBIG2_GLOBALS`]; `None` without one (a reference to no object is
    /// one to null, ISO 32000-2 §7.3.10, which is an absent entry, §7.3.7). The image is then read
    /// without them, as the renderer draws it, so an image that needed them mostly fails to decode
    /// and can decode to other pixels. A stream that is there and can't be read is an error, rather
    /// than the image decoded without it.
    ///
    /// The images of a scan share one globals stream, so a stream of the file, not edited since
    /// it was read (the same in every clone) and with its `/DecodeParms` direct (by reference they
    /// are another object, which can change while this one doesn't), has its filters undone once a
    /// document and is kept, up to [`JBIG2_GLOBALS_HELD`] in all; others are read each time. That
    /// saves the filters only: the segments it holds are decoded again for every image. Undoing
    /// the filters spends from `work`, so globals read for every image pay every time.
    fn jbig2_globals(&self, s: &Stream, work: &mut u64) -> Result<Option<Arc<Vec<u8>>>, CosError> {
        let Some(entry) = self.last_filter_parms(s).get(b"JBIG2Globals").cloned() else { return Ok(None) };
        let held = match entry {
            Object::Ref(r) if !self.overlay.contains_key(&r.num) => Some(r.num),
            _ => None,
        };
        if let Some(hit) = held.and_then(|num| self.jbig2_globals.lock().ok()?.get(&num).cloned()) {
            return Ok(Some(hit));
        }
        let Object::Stream(g) = &*self.resolve(&entry) else { return Ok(None) };
        let readable = self.with_direct_parms(g);
        let held = held.filter(|_| matches!(readable, Cow::Borrowed(_)));
        let mut data = readable.decoded_spending(MAX_JBIG2_GLOBALS, work).map_err(|e| {
            let detail = match e {
                CosError::Filter(detail) => detail,
                e => e.to_string(),
            };
            CosError::Filter(format!("the JBIG2 globals can't be read: {detail}"))
        })?;
        // What is kept counts by its length, so that's all it may hold on to.
        data.shrink_to_fit();
        let data = Arc::new(data);
        // A poisoned lock only means nothing is kept.
        if let (Some(num), Ok(mut kept)) = (held, self.jbig2_globals.lock()) {
            let total = kept.values().map(|g| g.len()).fold(data.len(), usize::saturating_add);
            if total <= JBIG2_GLOBALS_HELD {
                kept.insert(num, data.clone());
            }
        }
        Ok(Some(data))
    }

    pub fn try_get(&self, num: u32) -> Result<Arc<Object>, CosError> {
        match self.overlay.get(&num) {
            Some(Slot::Set(_, o) | Slot::RepairedSet(_, _, o)) => return Ok(o.clone()),
            Some(Slot::Freed(_)) => return Ok(Arc::new(Object::Null)),
            None => {}
        }
        if let Some(o) = self.cache.lock().map_err(|_| CosError::Poisoned)?.get(&num) {
            return Ok(o.clone());
        }
        // Loading can re-enter (indirect stream `/Length`, object streams); damaged files make
        // cycles such as `6 0 obj << /Length 6 0 R >>`. Bound the nesting on every path.
        let _guard =
            LoadGuard::enter(num).ok_or_else(|| CosError::Syntax { offset: 0, detail: format!("reference cycle while loading object {num}") })?;
        let _scope = crate::security::FollowScope::enter();
        let obj = self.load(num, 0)?;
        let obj = Arc::new(obj);
        // The editing cache is bounded like the scan reader's: re-parsing a file-backed view
        // is cheap, and returned objects outlive eviction.
        let owned = held_bytes(&obj, &self.data, 0);
        self.cache.lock().map_err(|_| CosError::Poisoned)?.insert(num, obj.clone(), owned);
        self.evict_editor_cache_if_over_budget(num)?;
        Ok(obj)
    }

    /// Bounds for the editing cache: roomier than the scan reader's (128 objects / 8 MB)
    /// so interactive editing does not thrash, but still capped so touching a large
    /// document cannot pin its whole object graph. Only owned (decrypted/copied) bytes
    /// count; file-backed views re-parse for free.
    ///
    /// Worst case per document: input bytes + `EDITOR_CACHE_BYTES` (32 MB) of owned object
    /// data + `EDITOR_OBJSTM_BYTES` (32 MB) + unsaved `overlay` edits. Each object is measured
    /// once, as it is cached ([`ObjectCache`](super::state::ObjectCache)), so both caps hold
    /// after every insertion.
    /// Clones share these caches, so one clone's eviction drops another's hot set;
    /// that only costs re-parsing, never correctness. Eviction keeps just the newcomer:
    /// file-backed views re-parse cheaply and editor working sets stay far below 1024.
    pub(super) const EDITOR_CACHE_OBJECTS: usize = 1024;

    pub(super) const EDITOR_CACHE_BYTES: usize = 32 * 1024 * 1024;

    /// Object streams decode to megabytes each, so fewer are kept than plain objects.
    const EDITOR_OBJSTM_OBJECTS: usize = 256;

    const EDITOR_OBJSTM_BYTES: usize = 32 * 1024 * 1024;

    fn evict_editor_cache_if_over_budget(&self, fresh_num: u32) -> Result<(), CosError> {
        let mut cache = self.cache.lock().map_err(|_| CosError::Poisoned)?;
        if cache.len() <= Self::EDITOR_CACHE_OBJECTS && cache.owned() <= Self::EDITOR_CACHE_BYTES {
            return Ok(());
        }
        // Eviction keeps just the newcomer: file-backed views re-parse cheaply.
        cache.retain(|n| *n == fresh_num);
        Ok(())
    }

    fn evict_editor_streams_if_over_budget(&self, current: u32) -> Result<(), CosError> {
        // The caller inserts `current` first, so retaining it keeps a live entry.
        let over = {
            let streams = self.objstms.lock().map_err(|_| CosError::Poisoned)?;
            let bytes = streams.values().fold(0usize, |bytes, stream| {
                bytes
                    .saturating_add(stream.data.capacity())
                    .saturating_add(stream.index.capacity().saturating_mul(std::mem::size_of::<Option<(u32, usize)>>()))
            });
            bytes > Self::EDITOR_OBJSTM_BYTES || streams.len() > Self::EDITOR_OBJSTM_OBJECTS
        };
        if over {
            self.objstms.lock().map_err(|_| CosError::Poisoned)?.retain(|n, _| *n == current);
        }
        Ok(())
    }

    /// Visit every current object without filling this document's editing cache.
    ///
    /// Full-document searches (for example, standalone timestamps) need to inspect objects
    /// that may never be used again. Keep their parse caches private and release them in
    /// batches. Returned objects own their data and remain valid after the next batch.
    pub fn scan_objects(&self) -> impl Iterator<Item = (ObjRef, Arc<Object>)> + use<> {
        let numbers = self.object_numbers();
        let mut reader = self.object_reader();
        numbers.into_iter().map(move |num| {
            let reference = ObjRef::new(num, reader.document.generation(num));
            (reference, reader.get(reference))
        })
    }

    /// Like [`Self::scan_objects`], preserving read errors for callers that must distinguish
    /// incomplete discovery from an actual null object. Caches remain private and bounded.
    pub fn scan_objects_checked(&self) -> impl Iterator<Item = (ObjRef, Result<Arc<Object>, CosError>)> + use<> {
        self.scan_objects_subset(self.object_numbers())
    }

    /// Inspect selected current objects with the same bounded cache as a whole-file scan.
    /// Missing/freed numbers yield `Null`; malformed objects return their read error.
    pub fn scan_objects_subset(&self, numbers: Vec<u32>) -> impl Iterator<Item = (ObjRef, Result<Arc<Object>, CosError>)> + use<> {
        let mut reader = self.object_reader();
        numbers.into_iter().map(move |num| {
            let reference = ObjRef::new(num, reader.document.generation(num));
            (reference, reader.try_get(reference))
        })
    }

    /// Inspect selected objects in the immutable source, ignoring all unsaved edits. This
    /// allows a consumer to check an edit's dependencies without retaining an old document.
    pub fn scan_original_objects_subset(&self, numbers: Vec<u32>) -> impl Iterator<Item = (ObjRef, Result<Arc<Object>, CosError>)> + use<> {
        let mut reader = self.object_reader();
        reader.document.overlay.clear();
        numbers.into_iter().map(move |num| {
            let reference = ObjRef::new(num, reader.document.generation(num));
            (reference, reader.try_get(reference))
        })
    }

    /// Opened with a tighter stream limit than editing uses ([`Document::open_with_stream_limit`]).
    pub(crate) fn stream_limited(&self) -> bool {
        self.stream_limit < crate::object::MAX_DECODED
    }

    pub(crate) fn object_reader(&self) -> ObjectReader {
        let mut document = self.clone();
        document.cache = Arc::default();
        document.objstms = Arc::default();
        ObjectReader { document, decoded_count: 0 }
    }

    fn load(&self, num: u32, depth: u32) -> Result<Object, CosError> {
        if depth > 16 {
            return Err(CosError::Syntax { offset: 0, detail: "reference cycle while loading".into() });
        }
        match self.entries.lookup(num, &self.data) {
            None | Some(XrefEntry::Free { .. }) => Ok(Object::Null),
            Some(XrefEntry::InFile { offset, .. }) => {
                let resolve = |r: ObjRef| {
                    if let Some(known) = LENGTHS.with(|m| m.borrow().get(&r).copied()) {
                        return known;
                    }
                    let length = self.try_get_reference(r).ok().and_then(|o| o.as_int());
                    LENGTHS.with(|m| m.borrow_mut().insert(r, length));
                    length
                };
                let off = offset as usize;
                let at = |off: usize| match parse_indirect_shared(&self.data, off, &resolve, &self.endstreams) {
                    Ok((id, o)) if id.num == num => Some(self.decrypted(id, o)),
                    _ => None,
                };
                // Offsets relative to a shifted header, or simply wrong: try both fixes.
                at(off)
                    .or_else(|| if self.header_offset == 0 { None } else { at(off.saturating_add(self.header_offset)) })
                    .or_else(|| self.located.get(&num).and_then(|(o, _)| usize::try_from(*o).ok()).and_then(at))
                    .or_else(|| self.scan_for(num).map(|(id, o)| self.decrypted(id, o)))
                    .ok_or(CosError::MissingObject(num))
            }
            Some(XrefEntry::InStream { stream, index }) => {
                let stm = self.objstm(stream)?;
                // The member at the entry's position, else wherever the stream lists `num`.
                let at = usize::try_from(index).ok().and_then(|i| stm.index.get(i)).copied().flatten().filter(|(n, _)| *n == num);
                let (_, off) = at.or_else(|| stm.index.iter().flatten().find(|(n, _)| *n == num).copied()).ok_or(CosError::MissingObject(num))?;
                Lexer::new(&stm.data, off).object()
            }
        }
    }

    pub(super) fn objstm(&self, num: u32) -> Result<Arc<ObjStm>, CosError> {
        if let Some(s) = self.objstms.lock().map_err(|_| CosError::Poisoned)?.get(&num) {
            return Ok(s.clone());
        }
        // A file entry that cannot place the stream (one that says it is itself compressed
        // inside another stream, as in #2367, or points nowhere) is tried by the stream's own
        // header, as a plain object with a wrong offset is. An edited or freed stream, in the
        // overlay or in a saved revision's table, is what that says it is: the file's copy
        // must not come back.
        let loaded = match self.try_get(num) {
            Ok(o) if matches!(*o, Object::Stream(_)) => o,
            _ if self.overlay.contains_key(&num) || matches!(self.xref_entry(num), Some(XrefEntry::Free { .. })) => {
                return Err(CosError::MissingObject(num));
            }
            _ => Arc::new(self.scan_for(num).map(|(id, o)| self.decrypted(id, o)).ok_or(CosError::MissingObject(num))?),
        };
        let Object::Stream(s) = &*loaded else { return Err(CosError::MissingObject(num)) };
        let n = s.dict.int(b"N").unwrap_or(0).clamp(0, 1_000_000) as usize;
        let first = usize::try_from(s.dict.int(b"First").unwrap_or(0).max(0)).ok();
        // Its parameters can be given by reference, and can sit in another object stream. A scan of
        // the file's object streams comes here without a read around it.
        let _scope = crate::security::FollowScope::enter();
        let readable = crate::security::following(Cow::Borrowed(s), || self.with_direct_parms(s));
        let data = readable.decoded_within(self.stream_limit)?;
        let mut lx = Lexer::new(&data, 0);
        let mut index = Vec::with_capacity(n);
        for _ in 0..n {
            let (Some(Object::Int(onum)), Some(Object::Int(off))) = (lx.object().ok(), lx.object().ok()) else { break };
            index.push(objstm_member(onum, off, first, data.len()));
        }
        let stm = Arc::new(ObjStm { index, data: Arc::new(data) });
        // Same bound as the object cache: decoded object streams can be megabytes each.
        // Insert first so the retained `num` is actually in the map.
        self.objstms.lock().map_err(|_| CosError::Poisoned)?.insert(num, stm.clone());
        self.evict_editor_streams_if_over_budget(num)?;
        Ok(stm)
    }

    /// Last-resort lookup: scan the file for `num G obj`. What isn't there is remembered: each miss
    /// reads the whole file, and a file can refer to such an object from everywhere.
    fn scan_for(&self, num: u32) -> Option<(ObjRef, Object)> {
        if self.unfound.lock().is_ok_and(|unfound| unfound.contains(&num)) {
            return None;
        }
        #[cfg(test)]
        super::tests::FILE_SCANS.with(|scans| scans.set(scans.get() + 1));
        let needle = format!("{num} ");
        let data = &self.data;
        let mut found = None;
        let mut i = 0;
        while let Some(p) = find(data, needle.as_bytes(), i, data.len()) {
            i = p + 1;
            if p > 0 && !is_whitespace(data[p - 1]) {
                continue;
            }
            if let Ok((id, o)) = parse_indirect_shared(data, p, &|_| None, &self.endstreams)
                && id.num == num
            {
                found = Some((id, o)); // keep the last (newest) definition
            }
        }
        if found.is_none()
            && let Ok(mut unfound) = self.unfound.lock()
        {
            unfound.insert(num);
        }
        found
    }

    // ── editing ─────────────────────────────────────────────────────────────────────────────

    /// Stream data still pointing into another document's file would keep that whole file
    /// alive for as long as this edit exists: give it a buffer of its own (just its bytes).
    fn adopt(&self, obj: Object) -> Object {
        match obj {
            Object::Stream(mut s) if s.raw.borrows_other_than(&self.data) => {
                s.raw = s.raw.detached();
                Object::Stream(s)
            }
            other => other,
        }
    }

    /// Replace an object (keeping its generation).
    pub fn set(&mut self, r: ObjRef, obj: impl Into<Object>) {
        let obj = self.adopt(obj.into());
        self.overlay.insert(r.num, Slot::Set(r.generation, Arc::new(obj)));
        if r.num >= self.next_num {
            self.next_num = r.num.saturating_add(1);
        }
    }

    /// Replace an accepted existing reference without losing a damaged source slot's aliases.
    /// Unlike [`Self::set`], this preserves both xref/header generations when they disagree.
    /// Such an edit forces a full rewrite so the output has consistent references, rather than
    /// appending a new contradictory xref row. Missing, free and stale references are errors.
    pub fn set_existing(&mut self, r: ObjRef, obj: impl Into<Object>) -> Result<(), CosError> {
        if !self.reference_matches(r) {
            return Err(CosError::MissingObject(r.num));
        }
        let alias = self.alternate_generation(r);
        if let Some(alias) = alias {
            let obj = self.adopt(obj.into());
            self.overlay.insert(r.num, Slot::RepairedSet(r.generation, alias, Arc::new(obj)));
        } else {
            self.set(r, obj);
        }
        Ok(())
    }

    /// Other accepted generation for a known-live reference to a damaged slot.
    pub(super) fn alternate_generation(&self, r: ObjRef) -> Option<u16> {
        match self.overlay.get(&r.num) {
            Some(Slot::RepairedSet(generation, alias, _)) => Some(if r.generation == *generation { *alias } else { *generation }),
            Some(_) => None,
            None => match self.entries.lookup(r.num, &self.data) {
                Some(XrefEntry::InFile { offset, generation }) => self.indexed_header_reference(r.num, offset).and_then(|header| {
                    (header.generation != generation).then_some(if r.generation == generation { header.generation } else { generation })
                }),
                _ => None,
            },
        }
    }

    /// Add a new indirect object and return its reference.
    pub fn add(&mut self, obj: impl Into<Object>) -> ObjRef {
        let r = ObjRef::new(self.allocate_num(), 0);
        let obj = self.adopt(obj.into());
        self.overlay.insert(r.num, Slot::Set(0, Arc::new(obj)));
        r
    }

    /// Find a never-used number, retaining the high-water mark for incremental /Size.
    /// Exhausted hints fall back to gaps without replacing source or overlay objects.
    fn allocate_num(&mut self) -> u32 {
        let mut num = if self.next_num == u32::MAX { 1 } else { self.next_num.max(1) };
        while self.entries.lookup(num, &self.data).is_some() || self.overlay.contains_key(&num) {
            num = if num == u32::MAX - 1 { 1 } else { num + 1 };
        }
        self.next_num = self.next_num.max(num.saturating_add(1));
        num
    }

    /// Delete an object (its number becomes free with the next generation).
    pub fn free(&mut self, r: ObjRef) {
        self.overlay.insert(r.num, Slot::Freed(r.generation.saturating_add(1)));
    }

    /// Free what was added since the file was read and nothing refers to any more: the objects
    /// [`SaveOptions::drop_unlinked`](crate::SaveOptions::drop_unlinked) leaves out of an
    /// incremental save, freed as it writes them (the generation stays). For a writer about to
    /// add an object that nothing refers to either, which that option would leave out too.
    pub fn free_unlinked_additions(&mut self) {
        for num in crate::writer::unlinked_additions(self) {
            let generation = self.generation(num);
            self.overlay.insert(num, Slot::Freed(generation));
        }
    }

    /// Record `current`'s object graph as unsaved edits of this document.
    ///
    /// Used when the working snapshot was restored from an earlier revision and the next
    /// incremental save must still append to this file (the last saved revision). Objects
    /// that already match are left untouched, except members of an object stream of this file
    /// whose stream number `current` frees or reuses: those are written again on their own.
    /// Reads go through private readers so neither document's editing cache is filled.
    pub fn overlay_changes_from(&mut self, current: &Document) -> Result<(), CosError> {
        let saved_nums = self.object_numbers();
        let current_nums = current.object_numbers();
        let mut freed = Vec::new();
        let mut changed = Vec::new();
        {
            let mut saved_reader = self.object_reader();
            let mut current_reader = current.object_reader();
            for &num in &saved_nums {
                if current_nums.binary_search(&num).is_err() {
                    freed.push(num);
                }
            }
            for &num in &current_nums {
                let generation = current.generation(num);
                let same = saved_nums.binary_search(&num).is_ok()
                    && self.generation(num) == generation
                    && self.alternate_generation(ObjRef::new(num, generation)) == current.alternate_generation(ObjRef::new(num, generation))
                    && saved_reader.try_get(ObjRef::new(num, generation))? == current_reader.try_get(ObjRef::new(num, generation))?;
                if !same {
                    changed.push(num);
                }
            }
        }
        // An object stream of this file that is freed or replaced takes its members with it, so
        // a member that is otherwise unchanged is written again on its own.
        let gone = |n: u32| freed.binary_search(&n).is_ok() || changed.binary_search(&n).is_ok();
        let orphaned: Vec<u32> = current_nums
            .iter()
            .copied()
            .filter(|&num| changed.binary_search(&num).is_err())
            .filter(|&num| {
                let packed_in = match self.xref_entry(num) {
                    Some(XrefEntry::InStream { stream, .. }) => Some(stream),
                    _ => None,
                };
                packed_in.is_some_and(gone)
            })
            .collect();
        changed.extend(orphaned);
        for num in freed {
            let generation = self.generation(num);
            self.free(ObjRef::new(num, generation));
        }
        let mut current_reader = current.object_reader();
        for num in changed {
            let generation = current.generation(num);
            let object = current_reader.try_get(ObjRef::new(num, generation))?;
            if let Some(alias) = current.alternate_generation(ObjRef::new(num, generation)) {
                let object = self.adopt((*object).clone());
                self.overlay.insert(num, Slot::RepairedSet(generation, alias, Arc::new(object)));
                self.next_num = self.next_num.max(num.saturating_add(1));
            } else {
                self.set(ObjRef::new(num, generation), (*object).clone());
            }
        }
        *self.trailer_mut() = current.trailer().clone();
        Ok(())
    }

    /// Edit a dictionary (or stream dictionary) object in place.
    pub fn update_dict(&mut self, r: ObjRef, f: impl FnOnce(&mut Dict)) -> Result<(), CosError> {
        let mut obj = (*self.get(r)).clone();
        let d = obj.as_dict_mut().ok_or(CosError::NotADictionary(r))?;
        f(d);
        self.set_existing(r, obj)
    }

    /// Current generation of an object number.
    pub fn generation(&self, num: u32) -> u16 {
        match self.overlay.get(&num) {
            Some(Slot::Set(g, _) | Slot::RepairedSet(g, _, _)) => *g,
            Some(Slot::Freed(g)) => *g,
            None => match self.entries.lookup(num, &self.data) {
                Some(XrefEntry::InFile { generation, .. }) => generation,
                Some(XrefEntry::Free { next_generation }) => next_generation,
                _ => 0,
            },
        }
    }

    pub(crate) fn overlay_entries(&self) -> impl Iterator<Item = (u32, u16, Option<Arc<Object>>)> + '_ {
        self.overlay.iter().map(|(n, s)| match s {
            Slot::Set(g, o) | Slot::RepairedSet(g, _, o) => (*n, *g, Some(o.clone())),
            Slot::Freed(g) => (*n, *g, None),
        })
    }

    pub(crate) fn next_num(&self) -> u32 {
        self.next_num
    }

    /// All object numbers known (file and overlay), excluding freed ones.
    pub fn object_numbers(&self) -> Vec<u32> {
        let source_numbers = self.entries.object_numbers(&self.data);
        let mut numbers = Vec::with_capacity(source_numbers.len().max(self.overlay.len()));
        let mut source = source_numbers.into_iter().peekable();
        for (&num, slot) in &self.overlay {
            while source.peek().is_some_and(|n| *n < num) {
                if let Some(n) = source.next() {
                    numbers.push(n);
                }
            }
            if source.peek().is_some_and(|n| *n == num) {
                source.next();
            }
            if matches!(slot, Slot::Set(..) | Slot::RepairedSet(..)) {
                numbers.push(num);
            }
        }
        numbers.extend(source);
        numbers
    }

    /// Rebase this document on bytes written by the writer (after a save): edits become part of
    /// the file and the overlay is cleared, so the next incremental save appends only new edits.
    pub fn reopen_after_save(&self, bytes: Arc<Vec<u8>>) -> Result<Self, CosError> {
        Document::open(bytes)
    }

    // ── cross-reference parsing ─────────────────────────────────────────────────────────────

    pub(super) fn root_is_catalog(&self) -> bool {
        let Some(r) = self.root() else { return false };
        matches!(self.try_get_reference(r).as_deref(), Ok(Object::Dict(d)) if d.name(b"Type") == Some(b"Catalog") || d.contains(b"Pages"))
    }
}

/// The `/DecodeParms` entries the decoders read (ISO 32000-2 Tables 8, 11 and 13: the Flate and
/// LZW predictors, CCITT, DCT's colour transform): numbers and flags a file may give by
/// reference, which [`Dict::int`] doesn't follow.
const DECODE_PARMS_KEYS: [&[u8]; 13] = [
    b"Predictor",
    b"Colors",
    b"BitsPerComponent",
    b"Columns",
    b"EarlyChange",
    b"K",
    b"Rows",
    b"EndOfLine",
    b"EncodedByteAlign",
    b"EndOfBlock",
    b"BlackIs1",
    b"DamagedRowsBeforeError",
    b"ColorTransform",
];

/// How many references in a row [`Document::resolve`] follows: enough for any file, and it ends a
/// cycle of them.
const MAX_REFERENCE_HOPS: usize = 32;

/// An object read in place, or the object a reference names as the document holds it: either way
/// nothing is copied ([`Document::held`]).
pub(crate) enum Held<'o> {
    Direct(&'o Object),
    Resolved(Arc<Object>),
}

impl std::ops::Deref for Held<'_> {
    type Target = Object;

    fn deref(&self) -> &Object {
        match self {
            Held::Direct(o) => o,
            Held::Resolved(o) => o,
        }
    }
}
