//! Serialization and saving (ISO 32000-2 §7.5).
//!
//! * `write_incremental` appends the edited objects, a cross-reference section in the same style
//!   as the file's last one (table or stream) and a trailer with `/Prev`. The original bytes are
//!   never modified, which keeps digital signatures over earlier revisions valid. With
//!   `SaveOptions::drop_unlinked`, objects added since the file was read that nothing refers to
//!   any more are left out (written as free).
//! * `write_full` writes only objects reachable from the trailer, renumbered densely, with a
//!   cross-reference table or stream — the "Save As" / garbage-collecting path. Unknown trailer
//!   entries are copied, including objects reachable only from them.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::io::Write;
use std::sync::Arc;

use crate::CosError;
use crate::document::{Document, ObjectReader, XrefEntry};
use crate::object::{Dict, ObjRef, Object, PdfString, Stream};

/// Serialize one object. Deterministic: same object → same bytes.
pub fn serialize(o: &Object, out: &mut Vec<u8>) {
    match o {
        Object::Null => out.extend_from_slice(b"null"),
        Object::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
        Object::Int(i) => {
            let _ = write!(out, "{i}");
        }
        Object::Real(r) => write_real(*r, out),
        Object::String(s) => write_string(s, out),
        Object::Name(n) => write_name(n, out),
        Object::Array(a) => {
            out.push(b'[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(b' ');
                }
                serialize(x, out);
            }
            out.push(b']');
        }
        Object::Dict(d) => write_dict(d, out),
        Object::Stream(s) => write_stream(s, out),
        Object::Ref(r) => {
            let _ = write!(out, "{} {} R", r.num, r.generation);
        }
    }
}

fn write_real(r: f64, out: &mut Vec<u8>) {
    if !r.is_finite() {
        out.push(b'0');
        return;
    }
    // A whole number is written as an integer only within the integer range Annex C gives
    // (±2³¹−1), so `4e9` or `1e30` stays a real token for readers with 32-bit integers.
    if r.fract() == 0.0 && r.abs() <= f64::from(i32::MAX) {
        let _ = write!(out, "{}", r as i32);
        return;
    }
    // Plain decimal notation (§7.3.3 has no exponents). Rust's `Display` for f64 prints the
    // shortest string that parses back to the same value, never in exponent form, so values
    // round-trip exactly (rounding to a fixed number of places visibly shifted shading colours).
    let s = format!("{r}");
    out.extend_from_slice(if s == "-0" { b"0" } else { s.as_bytes() });
    // A whole number past that range prints without a point; the trailing `.` keeps it a real.
    if !s.contains('.') && s != "-0" {
        out.push(b'.');
    }
}

fn write_name(n: &[u8], out: &mut Vec<u8>) {
    out.push(b'/');
    for &b in n {
        let regular = (b'!'..=b'~').contains(&b) && !matches!(b, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%' | b'#');
        if regular {
            out.push(b);
        } else {
            let _ = write!(out, "#{b:02X}");
        }
    }
}

fn write_string(s: &PdfString, out: &mut Vec<u8>) {
    let binary = s.bytes.iter().filter(|b| !(b' '..=b'~').contains(*b) && !matches!(b, b'\n' | b'\r' | b'\t')).count();
    if s.hex || binary * 4 > s.bytes.len().max(1) {
        out.push(b'<');
        for b in &s.bytes {
            let _ = write!(out, "{b:02X}");
        }
        out.push(b'>');
        return;
    }
    out.push(b'(');
    for &b in &s.bytes {
        match b {
            b'(' | b')' | b'\\' => {
                out.push(b'\\');
                out.push(b);
            }
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\r' => out.extend_from_slice(b"\\r"),
            b'\t' => out.extend_from_slice(b"\\t"),
            0x08 => out.extend_from_slice(b"\\b"),
            0x0C => out.extend_from_slice(b"\\f"),
            b' '..=b'~' => out.push(b),
            _ => {
                let _ = write!(out, "\\{b:03o}");
            }
        }
    }
    out.push(b')');
}

fn write_dict(d: &Dict, out: &mut Vec<u8>) {
    out.extend_from_slice(b"<<");
    for (k, v) in d.iter() {
        write_name(k, out);
        out.push(b' ');
        serialize(v, out);
    }
    out.extend_from_slice(b">>");
}

fn write_stream(s: &Stream, out: &mut Vec<u8>) {
    let mut d = s.dict.clone();
    d.set(b"Length".to_vec(), Object::Int(s.raw.len() as i64));
    write_dict(&d, out);
    out.extend_from_slice(b"\nstream\n");
    out.extend_from_slice(&s.raw);
    out.extend_from_slice(b"\nendstream");
}

/// The object as written: encrypted with its (output) number when the document is encrypted,
/// except the `/Encrypt` dictionary itself (`source_num` is its number in the source document).
/// `o` has the document's numbers in its references ([`crate::security::transform`]).
fn prepared<'a>(doc: &Document, source_num: u32, num: u32, generation: u16, o: &'a Object) -> Cow<'a, Object> {
    match doc.output_security() {
        (Some(h), encrypt_num) if Some(source_num) != encrypt_num && !doc.stored_as_is(source_num) => {
            let pass = if doc.encryption_changed() { crate::security::Pass::EncryptReplacing } else { crate::security::Pass::Encrypt };
            Cow::Owned(crate::security::transform(doc, h, o, num, generation, pass))
        }
        _ => Cow::Borrowed(o),
    }
}

fn write_indirect(num: u32, generation: u16, o: &Object, out: &mut Vec<u8>) {
    let _ = writeln!(out, "{num} {generation} obj");
    serialize(o, out);
    out.extend_from_slice(b"\nendobj\n");
}

/// Options for saving.
#[derive(Clone, Debug)]
pub struct SaveOptions {
    /// Replace or add `/Info /ModDate` with this PDF date string (e.g. from `pdf_date`).
    pub mod_date: Option<String>,
    /// Seed for a new file identifier when the document has none (deterministic in tests).
    pub id_seed: u64,
    /// Full saves: pack non-stream objects into compressed object streams with a cross-reference
    /// stream (PDF 1.5+, §7.5.7–7.5.8). Off writes a classic table that any reader accepts.
    pub object_streams: bool,
    /// Incremental saves: leave out what was added since the file was read and nothing refers to
    /// any more, such as a comment added and deleted again or a redaction mark added and cleared
    /// (written as free). Off by default, because a writer may add an object only it finds again
    /// by scanning, as a standalone document timestamp is; such a writer frees the leftovers
    /// before adding it, with [`Document::free_unlinked_additions`]. Full saves always leave
    /// these out.
    pub drop_unlinked: bool,
    /// Entropy for the file identifier of a rewrite that asks for a new one (see
    /// [`Document::require_full_save_with_new_id`]). Given, the identifier depends only on this and
    /// on the written bytes, so the same save gives the same file; tests use it to pin output.
    /// Without it, fresh entropy from the host (the operating system on native targets, the
    /// browser on WebAssembly) is mixed in.
    pub id_entropy: Option<[u8; 16]>,
}

impl Default for SaveOptions {
    fn default() -> Self {
        Self { mod_date: None, id_seed: 0x5052_494E_5443_5241, object_streams: true, drop_unlinked: false, id_entropy: None }
    }
}

/// Objects per object stream (Acrobat and qpdf use 100–200).
const OBJSTM_SIZE: usize = 100;
/// Keep a batch's serialized body small as well as limiting its object count. One object
/// may exceed this target; it is emitted in its own object stream without retaining a batch.
const OBJSTM_BYTES: usize = 8 * 1024 * 1024;

/// Where an object lives, for the cross-reference section.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Row {
    /// Free, with the generation the number gets when reused.
    Free(u16),
    /// At a byte offset.
    InFile(u64, u16),
    /// Inside object stream `stream`, at position `index`.
    InStream(u32, u32),
}

/// Append the document's edits to its original bytes (incremental update, §7.5.6).
///
/// A document that had to be reconstructed (no usable cross-reference chain) cannot be updated
/// incrementally — other readers would follow the broken chain — so it is rewritten in full,
/// which also repairs the file (what users expect after "the file was damaged and repaired").
pub fn write_incremental(doc: &Document, opts: &SaveOptions) -> Result<Vec<u8>, CosError> {
    if doc.stream_limited() {
        return Err(CosError::ReadOnlyLimit);
    }
    // Reconstructed files have no chain to append to; added or removed encryption must
    // rewrite every object; redaction must not leave the old revision behind. All need a full save.
    if doc.reconstructed() || doc.encryption_changed() || doc.full_save_required() {
        return write_full(doc, opts);
    }
    let mut doc = doc.clone();
    stamp_mod_date(&mut doc, opts);
    let original = doc.bytes().clone();
    // The revision is written on its own and joined to the original at the end, so the result is
    // allocated once at its exact size: growing a buffer that holds a large original past a
    // guessed capacity would copy the whole file again and keep up to as much again unused.
    let newline = !matches!(original.last(), Some(b'\n' | b'\r'));
    // A file with bytes before its `%PDF-` header may count its offsets from the header. The new
    // section then has to as well: readers take `/Prev` and `startxref` in one convention, and a
    // mix sends them into reconstruction (#2364).
    let shift = if doc.revisions().last().is_some_and(|r| r.header_relative) { doc.header_offset() } else { 0 };
    let base = (original.len() + usize::from(newline)).saturating_sub(shift) as u64;
    let mut out = Vec::new();
    let mut offsets: BTreeMap<u32, (u64, u16, bool)> = BTreeMap::new(); // num → (offset, gen, in use)
    let unlinked = if opts.drop_unlinked { unlinked_additions(&doc) } else { HashSet::new() };
    for (num, generation, obj) in doc.overlay_entries() {
        match obj {
            // Written as free, so `/Size` still covers every number this document handed out. The
            // generation stays: the number was never in use, and an undo can bring it back as is.
            Some(_) if unlinked.contains(&num) => {
                offsets.insert(num, (0, generation, false));
            }
            Some(o) => {
                offsets.insert(num, (base + out.len() as u64, generation, true));
                write_indirect(num, generation, &prepared(&doc, num, num, generation, &o), &mut out);
            }
            None => {
                offsets.insert(num, (0, generation, false));
            }
        }
    }
    let prev = doc.revisions().last().map(|r| r.xref_offset);
    let as_stream = doc.revisions().last().is_some_and(|r| r.is_stream);
    let mut trailer = doc.trailer().clone();
    if doc.output_security().0.is_none() {
        ensure_id(&mut trailer, opts, &original); // an encrypted file's ID is part of its key
    }
    if let Some(p) = prev {
        trailer.set(b"Prev".to_vec(), Object::Int(p as i64));
    }
    let size = doc.next_num().max(trailer.int(b"Size").map_or(0, |n| n.clamp(0, i64::from(u32::MAX)) as u32));
    if as_stream {
        let rows = offsets.iter().map(|(n, (o, g, used))| (*n, if *used { Row::InFile(*o, *g) } else { Row::Free(*g) })).collect();
        write_xref_stream(&mut out, base, &mut trailer, rows, size);
    } else {
        trailer.set(b"Size".to_vec(), Object::Int(size as i64));
        let xref_at = base + out.len() as u64;
        write_xref_table(&mut out, &offsets, prev.is_none());
        out.extend_from_slice(b"trailer\n");
        write_dict(&trailer, &mut out);
        let _ = write!(out, "\nstartxref\n{xref_at}\n%%EOF\n");
    }
    let mut file = Vec::with_capacity(original.len() + usize::from(newline) + out.len());
    file.extend_from_slice(&original);
    if newline {
        file.push(b'\n');
    }
    file.extend_from_slice(&out);
    Ok(file)
}

/// The objects [`write_full`] writes: everything reachable from the trailer's /Root, /Info and
/// /Encrypt and from its unknown entries, breadth-first, with the new number each gets
/// (aliases of one slot included) and, in visit order, whether it is a stream.
type SaveOrder = (HashMap<ObjRef, u32>, Vec<(ObjRef, bool)>);

fn full_save_order(reader: &mut ObjectReader, trailer_in: &Dict) -> Result<SaveOrder, CosError> {
    // Breadth-first walk from the trailer, assigning new numbers in visit order.
    let mut map: HashMap<ObjRef, u32> = HashMap::new();
    // Record the type while traversing so classifying packed objects needs no second parse.
    let mut order: Vec<(ObjRef, bool)> = Vec::new();
    let mut queue: VecDeque<ObjRef> = VecDeque::new();
    let visit_refs = |o: &Object, queue: &mut VecDeque<ObjRef>| collect_refs(o, &mut |r| queue.push_back(r));
    for key in [&b"Root"[..], b"Info", b"Encrypt"] {
        if let Some(r) = trailer_in.reference(key) {
            queue.push_back(r);
        }
    }
    // A direct /Encrypt can refer to objects, /CF or /StmF, say.
    if let Some(encrypt @ Object::Dict(_)) = trailer_in.get(b"Encrypt") {
        visit_refs(encrypt, &mut queue);
    }
    // Unknown trailer entries are document data. Objects they alone reference must be written
    // too, or a full rewrite would keep the key and drop its value.
    for (key, value) in trailer_in.iter() {
        if !trailer_key_rewritten(key) {
            collect_refs(value, &mut |r| queue.push_back(r));
        }
    }
    while let Some(r) = queue.pop_front() {
        if map.contains_key(&r) {
            continue;
        }
        let o = reader.get(r);
        if matches!(*o, Object::Null) {
            continue; // dangling reference: written as null by renumbering below
        }
        // Accepted xref/header aliases denote one slot, including page-tree cycles.
        // Check for null first: an unrelated or freed generation must remain dangling.
        let canonical = ObjRef::new(r.num, reader.document.generation(r.num));
        if let Some(&num) = map.get(&canonical) {
            map.insert(r, num);
            continue;
        }
        let num = output_number(order.len())?;
        map.insert(r, num);
        map.insert(canonical, num);
        order.push((canonical, matches!(*o, Object::Stream(_))));
        visit_refs(&o, &mut queue);
    }
    Ok((map, order))
}

/// The object numbers a full save of `doc` would write: exactly the objects [`write_full`]
/// reaches from the trailer, object streams included. For a check that something removed from a
/// document cannot reach the saved file through a reference nobody looked for.
pub fn reachable_objects(doc: &Document) -> Result<HashSet<u32>, CosError> {
    let mut reader = doc.object_reader();
    let trailer_in = reader.document.trailer().clone();
    let (_, order) = full_save_order(&mut reader, &trailer_in)?;
    Ok(order.into_iter().map(|(r, _)| r.num).collect())
}

/// Write only reachable objects, renumbered from 1, with a table or cross-reference stream.
pub fn write_full(doc: &Document, opts: &SaveOptions) -> Result<Vec<u8>, CosError> {
    if doc.stream_limited() {
        return Err(CosError::ReadOnlyLimit);
    }
    // A full traversal must not fill the editor's shared caches (which undo snapshots also
    // own). Parsed objects and decoded object streams are temporary working data here.
    let mut reader = doc.object_reader();
    stamp_mod_date(&mut reader.document, opts);
    let trailer_in = reader.document.trailer().clone();
    let (map, order) = full_save_order(&mut reader, &trailer_in)?;
    let version = reader.document.version().chars().take(3).collect::<String>();
    let version = if opts.object_streams && version.as_str() < "1.5" { "1.5".to_string() } else { version };
    let mut out = Vec::new();
    let _ = writeln!(out, "%PDF-{version}");
    // A binary comment (raw high bytes) marks the file as binary for transfer tools.
    out.extend_from_slice(&[b'%', 0xE2, 0xE3, 0xCF, 0xD3, b'\n']);
    // The /Encrypt dictionary and everything it references (e.g. indirect /CF crypt-filter
    // dictionaries) are needed before anything can be decrypted: never inside object streams.
    let mut encryption_objects: std::collections::HashSet<u32> = std::collections::HashSet::new();
    let mut pending: Vec<ObjRef> = Vec::new();
    if let Some(encrypt) = trailer_in.get(b"Encrypt") {
        collect_refs(encrypt, &mut |r| pending.push(r));
    }
    while let Some(r) = pending.pop() {
        if let Some(&num) = map.get(&r)
            && encryption_objects.insert(num)
        {
            collect_refs(&reader.get(r), &mut |x| pending.push(x));
        }
    }
    // Acrobat refuses an encrypted file whose catalog sits in an object stream ("the file is
    // damaged and could not be repaired", before it even asks for the password), while other
    // readers accept it. qpdf keeps the catalog out of object streams too. Verified with
    // Acrobat DC on macOS (#774).
    let root_outside =
        if reader.document.output_security().0.is_some() { trailer_in.reference(b"Root").and_then(|r| map.get(&r)).copied() } else { None };
    let mut rows: BTreeMap<u32, Row> = BTreeMap::new();
    // Keep only references, not renamed copies of the entire object graph. Standalone
    // objects still precede object streams, preserving the established output order.
    let mut packed: Vec<(u32, ObjRef)> = Vec::new();
    for (i, (r, is_stream)) in order.iter().enumerate() {
        let num = output_number(i)?;
        let in_stream = opts.object_streams && !is_stream && !encryption_objects.contains(&num) && root_outside != Some(num);
        if in_stream {
            packed.push((num, *r));
        } else {
            let o = reread(&mut reader, *r, *is_stream)?;
            let o = renumber(&prepared(&reader.document, r.num, num, 0, &o), &map);
            rows.insert(num, Row::InFile(out.len() as u64, 0));
            write_indirect(num, 0, &o, &mut out);
        }
    }
    // Object streams take the numbers after the last object. Their contents are not encrypted
    // individually: the whole stream is (§7.5.7, §7.6.2).
    let mut next = output_number(order.len())?;
    let mut batch = ObjectStreamBatch::default();
    for (num, reference) in packed {
        let mut encoded = Vec::new();
        serialize(&renumber(&*reread(&mut reader, reference, false)?, &map), &mut encoded);
        if batch.count > 0 && batch.body.len().saturating_add(encoded.len()).saturating_add(1) > OBJSTM_BYTES {
            batch.flush(&reader.document, &mut next, &mut rows, &mut out)?;
        }
        let _ = write!(batch.header, "{} {} ", num, batch.body.len());
        batch.body.extend_from_slice(&encoded);
        batch.body.push(b'\n');
        drop(encoded);
        rows.insert(num, Row::InStream(next, batch.count as u32));
        batch.count += 1;
        if batch.count == OBJSTM_SIZE || batch.body.len() >= OBJSTM_BYTES {
            batch.flush(&reader.document, &mut next, &mut rows, &mut out)?;
        }
    }
    batch.flush(&reader.document, &mut next, &mut rows, &mut out)?;
    let mut trailer = full_save_trailer(&reader.document, &trailer_in, &map, opts, &out);
    if opts.object_streams {
        write_xref_stream(&mut out, 0, &mut trailer, rows, next);
    } else {
        trailer.set(b"Size".to_vec(), Object::Int(next as i64));
        let offsets: BTreeMap<u32, (u64, u16, bool)> =
            rows.iter().filter_map(|(n, r)| if let Row::InFile(o, g) = r { Some((*n, (*o, *g, true))) } else { None }).collect();
        let xref_at = out.len();
        write_xref_table(&mut out, &offsets, true);
        out.extend_from_slice(b"trailer\n");
        write_dict(&trailer, &mut out);
        let _ = write!(out, "\nstartxref\n{xref_at}\n%%EOF\n");
    }
    // The size isn't known until the end, and the buffer grew by doubling. The result is often
    // kept as a document's working bytes for as long as it's open: drop the unused capacity.
    out.shrink_to_fit();
    Ok(out)
}

/// Assemble the full rewrite's trailer and apply its identifier policy.
fn full_save_trailer(doc: &Document, trailer_in: &Dict, map: &HashMap<ObjRef, u32>, opts: &SaveOptions, out: &[u8]) -> Dict {
    let mut trailer = Dict::new();
    for key in [&b"Root"[..], b"Info", b"Encrypt"] {
        if let Some(r) = trailer_in.reference(key).and_then(|r| map.get(&r)) {
            trailer.set(key.to_vec(), Object::Ref(ObjRef::new(*r, 0)));
        }
    }
    if let Some(encrypt @ Object::Dict(_)) = trailer_in.get(b"Encrypt") {
        trailer.set(b"Encrypt".to_vec(), renumber(encrypt, map));
    }
    if let Some(id) = trailer_in.get(b"ID") {
        trailer.set(b"ID".to_vec(), direct_id(doc, id));
    }
    for (key, value) in trailer_in.iter() {
        if !trailer_key_rewritten(key) {
            trailer.set(key.clone(), renumber(value, map));
        }
    }
    if doc.output_security().0.is_none() {
        ensure_id(&mut trailer, opts, doc.bytes());
    }
    if doc.new_id_required() {
        regenerate_id(&mut trailer, opts, out, doc.output_security().0.is_some());
    }
    trailer
}

/// Object `r` read again for writing. The traversal read it once and only kept its number, so
/// a second read that fails, finds nothing or finds a different kind of object must fail the
/// save rather than write `null` in its place.
fn reread(reader: &mut ObjectReader, r: ObjRef, is_stream: bool) -> Result<Arc<Object>, CosError> {
    let o = reader.try_get(r)?;
    if matches!(*o, Object::Null) || matches!(*o, Object::Stream(_)) != is_stream {
        return Err(CosError::Syntax { offset: 0, detail: format!("object {} read differently while saving; nothing was written", r.num) });
    }
    Ok(o)
}

#[derive(Default)]
struct ObjectStreamBatch {
    header: Vec<u8>,
    body: Vec<u8>,
    count: usize,
}

impl ObjectStreamBatch {
    fn flush(&mut self, doc: &Document, next: &mut u32, rows: &mut BTreeMap<u32, Row>, out: &mut Vec<u8>) -> Result<(), CosError> {
        if self.count == 0 {
            return Ok(());
        }
        let following = next.checked_add(1).ok_or_else(|| CosError::Syntax { offset: 0, detail: "too many objects to save".into() })?;
        // Release capacity after each batch too: an unusually large object must not leave
        // a large scratch buffer alive for the rest of the save.
        let Self { mut header, body, count } = std::mem::take(self);
        let first = header.len();
        header.extend_from_slice(&body);
        drop(body);
        let mut d = Dict::new();
        d.set(b"Type".to_vec(), Object::name("ObjStm"));
        d.set(b"N".to_vec(), Object::Int(count as i64));
        d.set(b"First".to_vec(), Object::Int(first as i64));
        let stm = Object::Stream(Stream::flate(d, &header));
        rows.insert(*next, Row::InFile(out.len() as u64, 0));
        write_indirect(*next, 0, &prepared(doc, u32::MAX, *next, 0, &stm), out);
        *next = following;
        Ok(())
    }
}

/// `/ID` for the trailer, direct, as it has to be with `/Encrypt`: nothing else in the file holds
/// what it refers to now. Given by reference it is two entries at most, however many references a
/// file gave, so that one large string isn't copied for each. An entry that isn't a string stays in
/// its place as an empty one, which is how the key read it.
fn direct_id(doc: &Document, id: &Object) -> Object {
    if matches!(id, Object::Array(a) if !a.iter().any(|s| matches!(s, Object::Ref(_)))) {
        return id.clone();
    }
    let entry = |s: &Object| match &*doc.resolve(s) {
        s @ Object::String(_) => s.clone(),
        _ => Object::String(crate::PdfString { bytes: Vec::new(), hex: true }),
    };
    let held = doc.resolve(id);
    Object::Array(held.as_array().into_iter().flatten().take(2).map(entry).collect())
}

/// Trailer keys a full rewrite generates itself. Everything else is copied, with references
/// renumbered, so a save does not drop metadata the reader accepted.
fn trailer_key_rewritten(key: &[u8]) -> bool {
    matches!(
        key,
        b"Size" | b"Prev" | b"XRefStm" | b"Type" | b"W" | b"Index" | b"Filter" | b"DecodeParms" | b"Length" | b"Root" | b"Info" | b"Encrypt" | b"ID"
    )
}

fn output_number(index: usize) -> Result<u32, CosError> {
    u32::try_from(index).ok().and_then(|n| n.checked_add(1)).ok_or_else(|| CosError::Syntax { offset: 0, detail: "too many objects to save".into() })
}

/// Objects added since the file was read (their numbers are not in use in it) that nothing an
/// incremental update writes refers to: what one edit added and a later one unlinked, such as a
/// deleted comment or a cleared redaction mark. The file never held them, so leaving them out
/// loses nothing, while writing them would keep what the user removed. Only the update is walked:
/// from the trailer and the file's changed objects through additions. An unchanged object of the
/// file is not followed: a reference it holds to such a number already reads as null in the file
/// (a damaged file's dangling reference), and it still does.
pub(crate) fn unlinked_additions(doc: &Document) -> HashSet<u32> {
    let added = |num: u32| matches!(doc.xref_entry(num), None | Some(XrefEntry::Free { .. }));
    let mut pending: HashMap<u32, Arc<Object>> =
        doc.overlay_entries().filter_map(|(num, _, o)| Some((num, o?))).filter(|(num, _)| added(*num)).collect();
    if pending.is_empty() {
        return HashSet::new();
    }
    let mut stack: Vec<ObjRef> = Vec::new();
    doc.trailer().iter().for_each(|(_, v)| collect_refs(v, &mut |r| stack.push(r)));
    for (num, _, o) in doc.overlay_entries() {
        if let Some(o) = o
            && !added(num)
        {
            collect_refs(&o, &mut |r| stack.push(r));
        }
    }
    while let Some(r) = stack.pop() {
        if let Some(o) = pending.remove(&r.num) {
            collect_refs(&o, &mut |r| stack.push(r));
        }
    }
    pending.into_keys().collect()
}

pub(crate) fn collect_refs(o: &Object, f: &mut impl FnMut(ObjRef)) {
    match o {
        Object::Ref(r) => f(*r),
        Object::Array(a) => a.iter().for_each(|x| collect_refs(x, f)),
        Object::Dict(d) => d.iter().for_each(|(_, v)| collect_refs(v, f)),
        Object::Stream(s) => s.dict.iter().for_each(|(_, v)| collect_refs(v, f)),
        _ => {}
    }
}

fn renumber(o: &Object, map: &HashMap<ObjRef, u32>) -> Object {
    match o {
        Object::Ref(r) => map.get(r).map(|n| Object::Ref(ObjRef::new(*n, 0))).unwrap_or(Object::Null),
        Object::Array(a) => Object::Array(a.iter().map(|x| renumber(x, map)).collect()),
        Object::Dict(d) => Object::Dict(d.iter().map(|(k, v)| (k.clone(), renumber(v, map))).collect()),
        Object::Stream(s) => Object::Stream(Stream { dict: s.dict.iter().map(|(k, v)| (k.clone(), renumber(v, map))).collect(), raw: s.raw.clone() }),
        other => other.clone(),
    }
}

/// Classic xref table with one subsection per run of consecutive numbers (§7.5.4).
fn write_xref_table(out: &mut Vec<u8>, offsets: &BTreeMap<u32, (u64, u16, bool)>, include_zero: bool) {
    out.extend_from_slice(b"xref\n");
    let mut all: Vec<(u32, (u64, u16, bool))> = offsets.iter().map(|(k, v)| (*k, *v)).collect();
    if include_zero {
        all.insert(0, (0, (0, 65535, false)));
    }
    // A section is one or more subsections. `xref` followed immediately by `trailer` is not a
    // table (qpdf: "xref syntax invalid") and is what an incremental save of a classic-table
    // file used to append when nothing was edited. A zero-length subsection names no objects,
    // so the free list and every in-use entry stay as they were, while a trailer-only change
    // (a missing `/ID`, or a caller edit of the trailer) is still written.
    if all.is_empty() {
        let _ = writeln!(out, "0 0");
        return;
    }
    let mut i = 0;
    while i < all.len() {
        let start = all[i].0;
        let mut j = i;
        while j + 1 < all.len() && all[j + 1].0 == all[j].0 + 1 {
            j += 1;
        }
        let _ = writeln!(out, "{} {}", start, j - i + 1);
        for (_, (off, generation, used)) in &all[i..=j] {
            let _ = writeln!(out, "{:010} {:05} {} ", off, generation, if *used { 'n' } else { 'f' });
        }
        i = j + 1;
    }
}

/// A cross-reference stream section (§7.5.8) that takes object number `num`, ending the file.
/// `out` holds the file from byte offset `base` on. Rows are Flate-compressed with the PNG Up
/// predictor, and field widths fit the largest value.
fn write_xref_stream(out: &mut Vec<u8>, base: u64, trailer: &mut Dict, mut rows: BTreeMap<u32, Row>, num: u32) {
    let xref_at = base + out.len() as u64;
    rows.insert(num, Row::InFile(xref_at, 0));
    if trailer.get(b"Prev").is_none() {
        rows.entry(0).or_insert(Row::Free(65535));
    }
    let fields = |r: &Row| match *r {
        Row::Free(g) => (0u64, 0u64, u64::from(g)),
        Row::InFile(o, g) => (1, o, u64::from(g)),
        Row::InStream(s, i) => (2, u64::from(s), u64::from(i)),
    };
    let width = |v: u64| ((64 - v.leading_zeros()).div_ceil(8)).max(1) as usize;
    let (w2, w3) = rows.values().map(fields).fold((1, 1), |(a, b), (_, x, y)| (width(x).max(a), width(y).max(b)));
    let cols = 1 + w2 + w3;
    let mut index = Vec::new();
    let mut numbers = rows.keys().copied().peekable();
    while let Some(start) = numbers.next() {
        let mut last = start;
        let mut count = 1i64;
        while numbers.peek().copied().is_some_and(|next| last.checked_add(1) == Some(next)) {
            if let Some(next) = numbers.next() {
                last = next;
                count += 1;
            }
        }
        index.push(Object::Int(i64::from(start)));
        index.push(Object::Int(count));
    }
    let mut data = Vec::with_capacity(rows.len().saturating_mul(cols + 1));
    // Type is one byte, and each remaining field is at most one u64. Reuse the
    // predictor row on the stack instead of allocating a Vec for every xref entry.
    let mut prev = [0u8; 17];
    for r in rows.values() {
        let (t, a, b) = fields(r);
        let a = a.to_be_bytes();
        let b = b.to_be_bytes();
        let fields = std::iter::once(t as u8).chain(a.into_iter().skip(8 - w2)).chain(b.into_iter().skip(8 - w3));
        data.push(2); // PNG "Up"
        for (value, previous) in fields.zip(&mut prev) {
            data.push(value.wrapping_sub(*previous));
            *previous = value;
        }
    }
    let mut d = trailer.clone();
    d.set(b"Type".to_vec(), Object::name("XRef"));
    d.set(b"Size".to_vec(), Object::Int(num as i64 + 1));
    d.set(b"W".to_vec(), Object::Array(vec![Object::Int(1), Object::Int(w2 as i64), Object::Int(w3 as i64)]));
    d.set(b"Index".to_vec(), Object::Array(index));
    let mut stream = Stream::flate(d, &data);
    let mut parms = Dict::new();
    parms.set(b"Predictor".to_vec(), Object::Int(12));
    parms.set(b"Columns".to_vec(), Object::Int(cols as i64));
    stream.dict.set(b"DecodeParms".to_vec(), Object::Dict(parms));
    write_indirect(num, 0, &Object::Stream(stream), out);
    let _ = write!(out, "startxref\n{xref_at}\n%%EOF\n");
    trailer.set(b"Size".to_vec(), Object::Int(num as i64 + 1));
}

fn stamp_mod_date(doc: &mut Document, opts: &SaveOptions) {
    let Some(date) = &opts.mod_date else { return };
    let date = Object::String(PdfString::literal(date.as_bytes().to_vec()));
    match doc.trailer().get(b"Info").cloned() {
        Some(Object::Ref(r)) => {
            let _ = doc.update_dict(r, |d| d.set(b"ModDate".to_vec(), date.clone()));
        }
        _ => {
            let mut info = Dict::new();
            info.set(b"ModDate".to_vec(), date);
            let r = doc.add(info);
            doc.trailer_mut().set(b"Info".to_vec(), Object::Ref(r));
        }
    }
}

/// Keep an existing `/ID`, or create one (two equal 16-byte strings, §14.4).
fn ensure_id(trailer: &mut Dict, opts: &SaveOptions, content: &[u8]) {
    if matches!(trailer.get(b"ID"), Some(Object::Array(a)) if a.len() == 2) {
        return;
    }
    // FNV-1a over a sample of the content and the seed: stable, dependency-free.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325 ^ opts.id_seed;
    for b in content.iter().step_by((content.len() / 4096).max(1)).chain(content.iter().rev().take(64)) {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    let mut id = Vec::with_capacity(16);
    id.extend_from_slice(&h.to_be_bytes());
    id.extend_from_slice(&h.rotate_left(29).wrapping_mul(0x9E37_79B9_7F4A_7C15).to_be_bytes());
    let s = Object::String(PdfString { bytes: id, hex: true });
    trailer.set(b"ID".to_vec(), Object::Array(vec![s.clone(), s]));
}

/// Give a rewritten file a new identifier (§14.4), so that a redacted or sanitized copy does not
/// claim to be its source. An encrypted file keeps the first string, which its key is derived from,
/// and gets a new second one. When there is no such string (no `/ID`, one entry, or a first entry
/// that is not a string) the reader takes the empty string as the key input (`Document::first_id`),
/// so the empty string is written first and the key stays valid.
fn regenerate_id(trailer: &mut Dict, opts: &SaveOptions, written: &[u8], encrypted: bool) {
    let fresh = Object::String(PdfString { bytes: new_id_bytes(opts, written), hex: true });
    let pair = if encrypted {
        let first = match trailer.get(b"ID") {
            Some(Object::Array(a)) => match a.first() {
                Some(Object::String(s)) => Object::String(s.clone()),
                _ => Object::String(PdfString { bytes: Vec::new(), hex: true }),
            },
            _ => Object::String(PdfString { bytes: Vec::new(), hex: true }),
        };
        vec![first, fresh]
    } else {
        vec![fresh.clone(), fresh]
    };
    trailer.set(b"ID".to_vec(), Object::Array(pair));
}

/// FNV-1a over 64 bits, with its published offset basis and prime. The identifier is built from it
/// rather than from `std`'s `DefaultHasher`, whose algorithm the standard library does not specify.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn fnv1a(state: u64, bytes: &[u8]) -> u64 {
    bytes.iter().fold(state, |h, b| (h ^ u64::from(*b)).wrapping_mul(FNV_PRIME))
}

/// Sixteen bytes for a file identifier, one 8-byte half per lane. The bytes are a pure function of
/// `SaveOptions::id_entropy`, `id_seed` and the written file: the hash is FNV-1a (above) and every
/// integer is written as a fixed-width big-endian value, so the same input gives the same identifier
/// on every platform and toolchain. Without entropy only the salt changes (see `id_salt`). FNV is
/// not collision-resistant against chosen content; the identifier only has to be unique, not secret.
/// Every written byte is hashed, once, in a single traversal of the output.
fn new_id_bytes(opts: &SaveOptions, written: &[u8]) -> Vec<u8> {
    let salt = id_salt(opts);
    let length = u64::try_from(written.len()).unwrap_or(u64::MAX);
    let mut halves = [0u8, 1].map(|lane| {
        let mut h = fnv1a(FNV_OFFSET, &salt.to_be_bytes());
        h = fnv1a(h, &[lane]);
        h = fnv1a(h, &opts.id_seed.to_be_bytes());
        fnv1a(h, &length.to_be_bytes())
    });
    for chunk in written.chunks(4096) {
        for h in &mut halves {
            *h = fnv1a(*h, chunk);
        }
    }
    halves.iter().flat_map(|h| h.to_be_bytes()).collect()
}

/// The salt of an identifier. With caller entropy it is a fixed hash of that entropy (deterministic,
/// for tests and callers who pin output). Without it the salt is fresh: on WebAssembly from the
/// browser ([`browser_salt`], so two fresh web apps that write the same bytes cannot share an
/// identifier), and elsewhere from the standard library's random hash keys — seeded by the operating
/// system, so separate processes differ — mixed with a process-wide counter, which separates
/// successive calls in one process.
fn id_salt(opts: &SaveOptions) -> u64 {
    use std::hash::BuildHasher;
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    match &opts.id_entropy {
        Some(entropy) => fnv1a(FNV_OFFSET, entropy),
        None => {
            #[cfg(target_arch = "wasm32")]
            if let Some(salt) = browser_salt() {
                return salt;
            }
            std::collections::hash_map::RandomState::new().hash_one(COUNTER.fetch_add(1, Ordering::Relaxed))
        }
    }
}

/// One byte from one `Math.random` draw: the draw (in [0, 1)) scaled to [0, 256), integer part
/// taken. The quotient is in [0, 256), so a saturating cast never wraps; a NaN or infinite draw
/// (which a conforming engine never returns) folds in as 0. No input can panic here.
#[cfg(any(target_arch = "wasm32", test))]
fn draw_byte(random: f64) -> u8 {
    (random * 256.0).floor().rem_euclid(256.0) as u8
}

/// The `Math.random` fallback's mixing: one salt byte per draw (see [`draw_byte`]), folded through
/// FNV-1a exactly like the Web Crypto bytes, so all sixteen bytes of the fold carry entropy from
/// the draws. Pure and platform-independent, so the native test target can exercise it; on wasm
/// each draw comes from `Math.random`.
#[cfg(any(target_arch = "wasm32", test))]
fn salt_from_draws(draws: &[f64; 16]) -> u64 {
    let mut bytes = [0u8; 16];
    for (byte, draw) in bytes.iter_mut().zip(draws) {
        *byte = draw_byte(*draw);
    }
    fnv1a(FNV_OFFSET, &bytes)
}

/// Fresh salt entropy from the browser, on WebAssembly. `crypto.getRandomValues` (Web Crypto, the
/// browser's secure generator, available in windows and workers alike) gives sixteen bytes, folded
/// into one salt. When that API is missing or refuses the call, `Math.random` is mixed instead
/// ([`salt_from_draws`]: sixteen draws, so the salt's full width carries entropy) — browsers seed
/// it afresh for every document, so a fresh instance still cannot reproduce another's salt. Only
/// where neither is reachable does the caller fall back to its `RandomState` salt, whose keys are
/// fixed at compile time on `wasm32-unknown-unknown` — the state before browser entropy was wired
/// in.
#[cfg(target_arch = "wasm32")]
fn browser_salt() -> Option<u64> {
    use wasm_bindgen::JsCast;
    let crypto = js_sys::Reflect::get(&js_sys::global(), &wasm_bindgen::JsValue::from_str("crypto"))
        .ok()
        .filter(|crypto| !crypto.is_undefined() && !crypto.is_null());
    if let Some(crypto) = crypto {
        let fill = js_sys::Reflect::get(&crypto, &wasm_bindgen::JsValue::from_str("getRandomValues"))
            .ok()
            .and_then(|fill| fill.dyn_into::<js_sys::Function>().ok());
        if let Some(fill) = fill {
            let bytes = js_sys::Uint8Array::new_with_length(16);
            if fill.call1(&crypto, &bytes).is_ok() {
                return Some(fnv1a(FNV_OFFSET, &bytes.to_vec()));
            }
        }
    }
    // Web Crypto is missing or refused the call: degrade to `Math.random` rather than giving up.
    let mut draws = [0.0; 16];
    for draw in &mut draws {
        *draw = js_sys::Math::random();
    }
    Some(salt_from_draws(&draws))
}

/// A PDF date string for "now" given seconds since the Unix epoch (UTC).
pub fn pdf_date(unix_secs: i64) -> String {
    let (days, secs) = (unix_secs.div_euclid(86_400), unix_secs.rem_euclid(86_400));
    let (y, m, d) = civil_from_days(days);
    // `{y:04}` rather than `%Y`, which writes a `+` before years past 9999 (and a PDF date is a
    // bare four-plus-digit year, never signed). No year clamp — the old code wrote `y` as-is.
    format!("D:{y:04}{m:02}{d:02}{:02}{:02}{:02}Z", secs / 3600, secs / 60 % 60, secs % 60)
}

/// Days since 1970-01-01 → proleptic Gregorian (year, month, day).
///
/// A `from_timestamp` epoch fallback would change dates past `NaiveDate`'s ±262143-year range,
/// which the old hand-rolled converter formatted directly. The Gregorian calendar repeats every
/// 400 years (146_097 days), so chrono converts the day within one cycle and whole cycles go back
/// into the year (an `i128` so the extremes can't overflow), keeping chrono as the only calendar
/// logic and reproducing the old result exactly. 1970-01-01 is `num_days_from_ce` 719_163.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    use chrono::Datelike;
    let (cycles, rest) = (days.div_euclid(146_097), days.rem_euclid(146_097));
    // `rest + 719_163` is 719_163..865_259, always a valid CE day.
    match i32::try_from(rest + 719_163).ok().and_then(chrono::NaiveDate::from_num_days_from_ce_opt) {
        Some(d) => {
            let y = i128::from(d.year()) + i128::from(cycles) * 400;
            (i64::try_from(y).unwrap_or(if y < 0 { i64::MIN } else { i64::MAX }), d.month(), d.day())
        }
        None => (1970, 1, 1), // unreachable
    }
}

#[cfg(test)]
mod tests {
    /// `/ID` by reference comes out direct, each entry where it was: the key is made from the first.
    #[test]
    fn a_file_identifier_keeps_its_entries_in_their_places() {
        let doc = Document::new_empty();
        let string = |bytes: &[u8]| Object::String(crate::PdfString { bytes: bytes.to_vec(), hex: true });
        let nowhere = Object::Ref(ObjRef::new(9_999, 0));
        assert_eq!(super::direct_id(&doc, &Object::Array(vec![nowhere, string(b"ab")])), Object::Array(vec![string(b""), string(b"ab")]));
    }

    use proptest::prelude::*;

    #[test]
    fn the_same_entropy_and_bytes_give_the_same_identifier_on_every_platform() {
        // Computed from the formula outside this crate: fixed-width big-endian fields and FNV-1a.
        let opts = super::SaveOptions { id_entropy: Some([0x42; 16]), id_seed: 7, ..super::SaveOptions::default() };
        let want = [0x96, 0xf8, 0xcb, 0x7d, 0x5b, 0xb0, 0x09, 0xc8, 0x92, 0xc9, 0xb7, 0x51, 0xdf, 0xe6, 0xab, 0xd1];
        assert_eq!(super::new_id_bytes(&opts, b"%PDF-1.7\n"), want);
    }

    #[test]
    fn fnv1a_is_the_published_64_bit_algorithm() {
        // Published FNV-1a (64 bit) test vectors, so the identifier can be reproduced outside this crate.
        assert_eq!(super::fnv1a(super::FNV_OFFSET, b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(super::fnv1a(super::FNV_OFFSET, b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(super::fnv1a(super::FNV_OFFSET, b"foobar"), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn math_random_draws_fill_the_whole_salt_width() {
        // Draw i = (i + 1/2)/256 scales to the integer i, so the fold's byte i proves draw i landed
        // there: all sixteen bytes carry their own draw's entropy (the fallback once shifted four
        // draws into one word, leaving the other four bytes of the fold constant).
        let draws: [f64; 16] = std::array::from_fn(|i| (i as f64 + 0.5) / 256.0);
        let bytes: Vec<u8> = (0..16).collect();
        assert_eq!(super::salt_from_draws(&draws), super::fnv1a(super::FNV_OFFSET, &bytes));
    }

    #[test]
    fn every_math_random_draw_reaches_the_salt() {
        let base = [0.5; 16];
        let baseline = super::salt_from_draws(&base);
        for i in 0..16 {
            let mut draws = base;
            draws[i] = 0.75;
            assert_ne!(super::salt_from_draws(&draws), baseline, "draw {i} did not reach the salt");
        }
    }

    #[test]
    fn a_math_random_draw_outside_its_range_still_mixes_without_panicking() {
        // A conforming `Math.random` stays in [0, 1), but the mixing must not panic outside it.
        assert_eq!(super::draw_byte(0.0), 0);
        assert_eq!(super::draw_byte(0.5), 128);
        assert_eq!(super::draw_byte(255.5 / 256.0), 255);
        assert_eq!(super::draw_byte(-0.5 / 256.0), 255);
        assert_eq!(super::draw_byte(-1.0), 0);
        assert_eq!(super::draw_byte(2.0), 0);
        assert_eq!(super::draw_byte(f64::NAN), 0);
        assert_eq!(super::draw_byte(f64::INFINITY), 0);
    }

    use super::*;
    use crate::Lexer;

    fn roundtrip(o: &Object) -> Object {
        let mut out = Vec::new();
        serialize(o, &mut out);
        Lexer::new(&out, 0).object().expect("parses")
    }

    #[test]
    fn a_second_read_that_differs_fails_the_save_instead_of_writing_null() {
        let mut doc = Document::new_empty();
        let dict = doc.add(Object::Dict(Dict::new()));
        let mut reader = doc.object_reader();
        assert!(reread(&mut reader, dict, false).is_ok());
        assert!(reread(&mut reader, dict, true).is_err(), "a dictionary where a stream was traversed");
        assert!(reread(&mut reader, ObjRef::new(9999, 0), false).is_err(), "nothing where an object was traversed");
    }

    #[test]
    fn xref_predictor_rows_preserve_variable_width_fields_and_sparse_runs() {
        for offset in [0, 255, 256, 65535, 65536, u32::MAX as u64, u32::MAX as u64 + 1, u64::MAX] {
            for index in [0, 255, 256, u32::MAX] {
                for incremental in [false, true] {
                    let mut trailer = Dict::new();
                    if incremental {
                        trailer.set(b"Prev".to_vec(), Object::Int(17));
                    }
                    let rows = BTreeMap::from([(1, Row::InFile(offset, 32768)), (2, Row::Free(65535)), (4, Row::InStream(65536, index))]);
                    let mut bytes = Vec::new();
                    write_xref_stream(&mut bytes, 0, &mut trailer, rows, 5);
                    let (_, object) = crate::parser::parse_indirect(&bytes, 0, &|_| None).unwrap();
                    let Object::Stream(stream) = object else { panic!("xref stream") };
                    let widths: Vec<_> = stream.dict.get(b"W").unwrap().as_array().unwrap().iter().map(|o| o.as_int().unwrap() as usize).collect();
                    assert_eq!(widths[0], 1);
                    assert!((3..=8).contains(&widths[1]));
                    assert!((2..=4).contains(&widths[2]));
                    let runs: Vec<_> = stream.dict.get(b"Index").unwrap().as_array().unwrap().iter().map(|o| o.as_int().unwrap()).collect();
                    assert_eq!(runs, if incremental { vec![1, 2, 4, 2] } else { vec![0, 3, 4, 2] });
                    let mut expected = vec![(1, offset, 32768), (0, 0, 65535), (2, 65536, u64::from(index)), (1, 0, 0)];
                    if !incremental {
                        expected.insert(0, (0, 0, 65535));
                    }
                    let decoded = stream.decoded().unwrap();
                    let columns: usize = widths.iter().sum();
                    assert_eq!(decoded.len(), columns * expected.len());
                    for (row, (kind, first, second)) in decoded.chunks_exact(columns).zip(expected) {
                        let values: Vec<_> = [0..1, 1..1 + widths[1], 1 + widths[1]..columns]
                            .into_iter()
                            .map(|range| row[range].iter().fold(0u64, |value, byte| value * 256 + u64::from(*byte)))
                            .collect();
                        assert_eq!(values, vec![kind, first, second]);
                    }
                }
            }
        }
    }

    #[test]
    fn pdf_dates_are_utc_and_byte_identical() {
        // `D:YYYYMMDDHHmmSSZ` (ISO 32000-2 §7.9.4), the same strings the hand-rolled code wrote.
        assert_eq!(pdf_date(0), "D:19700101000000Z");
        assert_eq!(pdf_date(1_790_000_000), "D:20260921141320Z");
        // Leap days, including the 400-year rule, and the last second of a year.
        assert_eq!(pdf_date(1_709_164_800), "D:20240229000000Z");
        assert_eq!(pdf_date(951_782_400), "D:20000229000000Z");
        assert_eq!(pdf_date(1_767_225_599), "D:20251231235959Z");
        // Before the epoch (seconds into a day are counted forward from midnight UTC).
        assert_eq!(pdf_date(-1), "D:19691231235959Z");
        // Years past 9999 keep the plain zero-padded year (no `+`), as the hand-rolled code did.
        assert_eq!(pdf_date(253_402_300_800), "D:100000101000000Z");
        // Past chrono's ±262143-year range the proleptic Gregorian date is kept, not the epoch.
        // Pinned to the old hand-rolled output (which had no year clamp — a negative year keeps
        // its sign, exactly as before).
        assert_eq!(pdf_date(10_000_000_000_000), "D:3188570520174640Z");
        assert_eq!(pdf_date(-10_000_000_000_000), "D:-3149180813061320Z");
    }

    #[test]
    fn reals_keep_full_precision_without_exponents() {
        for r in [0.1, -0.000_000_123_456_789, 1e-12, 123_456.789_012_345, 1.0 / 3.0, -2.5e7 + 0.25] {
            let mut out = Vec::new();
            serialize(&Object::Real(r), &mut out);
            assert!(!out.contains(&b'e') && !out.contains(&b'E'), "{}", String::from_utf8_lossy(&out));
            assert_eq!(roundtrip(&Object::Real(r)), Object::Real(r));
        }
    }

    #[test]
    fn whole_reals_past_the_integer_limit_stay_reals() {
        for (r, text) in [
            (2_147_483_647.0, "2147483647"),
            (-2_147_483_647.0, "-2147483647"),
            (2_147_483_648.0, "2147483648."),
            (4e9, "4000000000."),
            (-4e9, "-4000000000."),
            (1e30, "1000000000000000000000000000000."),
            (595.0, "595"),
            (-0.0, "0"),
        ] {
            let mut out = Vec::new();
            serialize(&Object::Real(r), &mut out);
            assert_eq!(String::from_utf8_lossy(&out), text, "{r}");
            let back = roundtrip(&Object::Real(r));
            if r.abs() > f64::from(i32::MAX) {
                assert_eq!(back, Object::Real(r), "{r} must read back as a real");
            }
        }
    }

    proptest! {
        #[test]
        fn any_finite_real_round_trips(r in proptest::num::f64::NORMAL | proptest::num::f64::SUBNORMAL | proptest::num::f64::ZERO) {
            let back = roundtrip(&Object::Real(r));
            match back {
                Object::Real(b) => prop_assert_eq!(b, r),
                Object::Int(i) => {
                    prop_assert!(i.unsigned_abs() <= i32::MAX as u64, "{r} came back as integer {i}");
                    prop_assert_eq!(i as f64, r);
                }
                other => prop_assert!(false, "unexpected {other:?}"),
            }
        }

        #[test]
        fn strings_and_names_round_trip(bytes in proptest::collection::vec(any::<u8>(), 0..64)) {
            let s = Object::String(PdfString { bytes: bytes.clone(), hex: false });
            prop_assert_eq!(roundtrip(&s).as_string().map(|s| s.bytes.clone()), Some(bytes.clone()));
            if !bytes.is_empty() && !bytes.contains(&0) {
                prop_assert_eq!(roundtrip(&Object::Name(bytes.clone())), Object::Name(bytes));
            }
        }
    }
}
