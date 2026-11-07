//! Reading and querying the xref table of a PDF file.

use crate::crypto::{DecryptionError, DecryptionTarget, Decryptor, get};
use crate::data::Data;
use crate::metadata::Metadata;
use crate::object::Name;
use crate::object::Number;
use crate::object::ObjectIdentifier;
use crate::object::Stream;
use crate::object::dict::keys::{
    AUTHOR, CREATION_DATE, CREATOR, ENCRYPT, FIRST, ID, INDEX, INFO, KEYWORDS, MOD_DATE, N,
    OCPROPERTIES, PAGES, PREV, PRODUCER, ROOT, SIZE, SUBJECT, TITLE, TYPE, VERSION, W, XREF_STM,
};
use crate::object::dict::{DictCache, probe_dict};
use crate::object::indirect::IndirectObject;
use crate::object::{Array, MaybeRef, ObjRef};
use crate::object::{DateTime, Dict};
use crate::object::{Object, ObjectLike};
use crate::pdf::PdfVersion;
use crate::reader::Reader;
use crate::reader::{Readable, ReaderContext, ReaderExt};
use crate::sync::{Arc, FxHashMap, Mutex, RwLock, RwLockExt};
use crate::trivia::is_white_space_character;
use crate::util::findr_needle;
use crate::{PdfData, object};
use alloc::collections::BTreeSet;
use alloc::vec;
use alloc::vec::Vec;
use core::cmp::max;
use core::iter;
use core::ops::Deref;

pub(crate) const XREF_ENTRY_LEN: usize = 20;

/// PdfCraft patch: (hayro-syntax (33)) how many references a read follows through objects that hold
/// nothing else. Past it, as any cycle goes, the object reads as a missing one.
const MAX_REFERENCE_HOPS: usize = 32;

/// PdfCraft patch: (hayro-syntax (33)) what [`XRef::fetch`] found at an object's place.
enum Fetched<T> {
    Object(T),
    /// All the object holds is a reference to this one.
    Reference(ObjectIdentifier),
}

#[derive(Debug, Copy, Clone)]
pub(crate) enum XRefError {
    Unknown,
    Encryption(DecryptionError),
}

/// Parse the "root" xref from the PDF.
pub(crate) fn root_xref(data: PdfData, password: &[u8]) -> Result<XRef, XRefError> {
    let mut xref_map = FxHashMap::default();
    let xref_pos = find_last_xref_pos(data.as_ref()).ok_or(XRefError::Unknown)?;
    let trailer =
        populate_xref_impl(data.as_ref(), xref_pos, &mut xref_map).ok_or(XRefError::Unknown)?;

    XRef::new(
        data.clone(),
        xref_map,
        XRefInput::TrailerDictData(trailer),
        false,
        password,
    )
}

/// Try to manually parse the PDF to build an xref table and trailer dictionary.
pub(crate) fn fallback(data: PdfData, password: &[u8]) -> Option<XRef> {
    warn!("xref table was invalid, trying to manually build xref table");
    let (xref_map, xref_input) = fallback_xref_map(&data, password);

    if let Some(xref_input) = xref_input {
        warn!("rebuild xref table with {} entries", xref_map.len());

        XRef::new(data.clone(), xref_map, xref_input, true, password).ok()
    } else {
        warn!("couldn't find trailer dictionary, failed to rebuild xref table");

        None
    }
}

/// PdfCraft patch: an `N G obj` header for the repair scan, taking a generation number that
/// cannot be read as 0.
///
/// A generation number is at most 65,535 (ISO 32000-2 §7.5.4), so `3 18446744073709551616 obj` is
/// malformed and [`ObjectIdentifier::read`] is right to refuse it everywhere else. This scan runs
/// only when a file's cross-reference table is unusable and its objects are being recovered by
/// reading the file, where refusing the header drops the object and whatever it holds (#5580).
/// Before #427 the digits wrapped, and 2^64 wrapped to exactly 0, which is the only reason such a
/// file used to open.
///
/// Only here: reading numbers anywhere else is unchanged.
fn read_object_header<'a>(r: &mut Reader<'a>, ctx: &ReaderContext<'a>) -> Option<ObjectIdentifier> {
    let mut strict = r.clone();
    if let Some(obj_id) = strict.read::<ObjectIdentifier>(ctx) {
        *r = strict;
        return Some(obj_id);
    }
    // On a clone as well, so a header that isn't one leaves `r` where it was for the caller.
    let mut lenient = r.clone();
    let obj_number = lenient.read_without_context::<i32>()?;
    lenient.skip_white_spaces_and_comments();
    // Any number at all in the generation position, including one no generation may be.
    lenient.skip::<Number>(false)?;
    lenient.skip_white_spaces_and_comments();
    lenient.forward_tag(b"obj")?;
    *r = lenient;
    Some(ObjectIdentifier::new(obj_number, 0))
}

/// PdfCraft patch: the body of the object at `offset` whose `N G obj` header cannot be read
/// strictly, for a table rebuilt by scanning the file (#5580). The steps are
/// `IndirectObject::read`'s, with [`read_object_header`] in place of the strict identifier.
fn read_repaired_body<'a, T>(
    data: &'a [u8],
    offset: usize,
    id: ObjectIdentifier,
    ctx: &ReaderContext<'a>,
) -> Option<T>
where
    T: ObjectLike<'a>,
{
    let mut r = Reader::new(data);
    r.jump(offset);
    // The number is what identifies the object; the generation is what could not be read.
    if read_object_header(&mut r, ctx)?.obj_number != id.obj_number {
        return None;
    }
    let mut ctx = ctx.clone();
    if ctx.parent_chain_contains(&id) {
        warn!("cycle detected in indirect object: {id:?}");

        return None;
    }
    ctx.set_obj_number(id);
    ctx.parent_chain_push(id);
    r.skip_white_spaces_and_comments();
    r.read_with_context::<T>(&ctx)
}

fn fallback_xref_map<'a>(data: &'a PdfData, password: &[u8]) -> (XrefMap, Option<XRefInput<'a>>) {
    fallback_xref_map_inner(data, ReaderContext::dummy(), true, password)
}

fn fallback_xref_map_inner<'a>(
    data: &'a PdfData,
    mut dummy_ctx: ReaderContext<'a>,
    recurse: bool,
    password: &[u8],
) -> (XrefMap, Option<XRefInput<'a>>) {
    let mut xref_map = FxHashMap::default();
    // PdfCraft patch: the generation-0 keys below that stand in for a header with another
    // generation, so a later header can replace one while a real `N 0 obj` is never displaced
    // (noted in review of #5580).
    let mut aliases: rustc_hash::FxHashSet<ObjectIdentifier> = rustc_hash::FxHashSet::default();
    let mut trailer_dicts = vec![];
    let mut root_ref = None;

    let mut r = Reader::new(data.as_ref());

    let mut last_obj_num = None;

    loop {
        let cur_pos = r.offset();

        let mut old_r = r.clone();

        // First try to check if we have an object identifier.
        if r.peek_byte().is_some_and(|b: u8| b.is_ascii_digit()) {
            if let Some(obj_id) = read_object_header(&mut r, &dummy_ctx) {
                let mut cloned = r.clone();
                // Check that the object following it is actually valid before inserting it.
                cloned.skip_white_spaces_and_comments();
                if cloned.skip::<Object<'_>>(false).is_some() {
                    xref_map.insert(obj_id, EntryType::Normal(cur_pos));
                    // PdfCraft patch: also reachable by object number alone, so a reference that
                    // names a different generation than the header still finds it. A rebuilt
                    // table has no revision history to tell two generations apart, and a file
                    // that needs rebuilding references everything as `N 0 R` whatever its headers
                    // say: `poppler-85140-0.pdf` has one page whose header reads
                    // `3 18446744073709551616 obj` with a page tree pointing at `3 0 R`, so the
                    // page was lost and the file did not open at all (#5580). `or_insert`, so a
                    // real `N 0 obj` is never displaced by this alias.
                    let by_number = ObjectIdentifier::new(obj_id.obj_number, 0);
                    if obj_id.gen_number == 0 {
                        // A real `N 0 obj`: it owns the key from here on, and the last one wins
                        // as every other entry does.
                        aliases.remove(&by_number);
                    } else if !xref_map.contains_key(&by_number) || aliases.contains(&by_number) {
                        // Stand in for it, and let a later header for the same number replace the
                        // stand-in -- the exact key moves on, so an alias left behind would send
                        // `N 0 R` to a body the exact key no longer names (noted in review).
                        xref_map.insert(by_number, EntryType::Normal(cur_pos));
                        aliases.insert(by_number);
                    }
                    last_obj_num = Some(obj_id);
                    dummy_ctx.set_obj_number(obj_id);
                }
            } else {
                // There must be a white space before the next object number.
                r.forward_while(|b| !is_white_space_character(b));
            }
        } else {
            // Then, try to check whether we have a dictionary, in particular a trailer
            // dictionary.
            let mut probe_reader = r.clone();
            if r.peek_bytes(2).is_some_and(|b| b == b"<<")
                && let Some(probe) =
                    { probe_dict(&mut probe_reader, &dummy_ctx, Some(b"<<"), b">>") }
            {
                r = probe_reader;
                if probe.has_root || probe.has_type {
                    let mut dict_reader = Reader::new(probe.data);
                    if let Some(dict) = dict_reader.read_with_context::<Dict<'_>>(&dummy_ctx) {
                        if probe.has_root && dict.contains_key(ROOT) {
                            trailer_dicts.push(dict.clone());
                        }

                        if dict
                            .get::<Name<'_>>(TYPE)
                            .is_some_and(|n| n.as_str() == "Catalog")
                        {
                            root_ref = last_obj_num;
                        }

                        if let Some(stream) = old_r.read::<Stream<'_>>(&dummy_ctx)
                            && dict.get::<Name<'_>>(TYPE).as_deref() == Some(b"ObjStm")
                            && let Some(data) = stream.decoded().ok()
                            && let Some(last_obj_num) = last_obj_num
                            && let Some(obj_stream) = ObjectStream::new(stream, &data, &dummy_ctx)
                        {
                            for (idx, (obj_num, _)) in obj_stream.offsets.iter().enumerate() {
                                let id = ObjectIdentifier::new(*obj_num as i32, 0);
                                // If we already found an entry for that object number that was not
                                // inside an object stream. Somewhat arbitrary and maybe
                                // we can do better, but that seems to work for the current
                                // set of tests.
                                if xref_map
                                    .get(&id)
                                    .is_none_or(|e| !matches!(e, &EntryType::Normal(_)))
                                {
                                    xref_map.insert(
                                        id,
                                        EntryType::ObjStream(
                                            last_obj_num.obj_number as u32,
                                            idx as u32,
                                        ),
                                    );
                                }
                            }
                        }
                    }
                }
            } else {
                // We can skip everything until the next white space character,
                // as there cannot possibly be any new dictionary/object identifier
                // until then.
                let old_pos = r.offset;
                r.forward_while(|b| !is_white_space_character(b));
                if r.offset == old_pos {
                    r.read_byte();
                }
            }
        }

        if r.at_end() {
            break;
        }
    }

    // Try to choose the right trailer dict by doing basic validation.
    let mut trailer_dict = None;

    for dict in trailer_dicts {
        if let Some(root_id) = dict.get_raw::<Dict<'_>>(ROOT) {
            let check = |dict: &Dict<'_>| -> bool { dict.contains_key(PAGES) };

            match root_id {
                MaybeRef::Ref(r) => match xref_map.get(&r.into()) {
                    Some(EntryType::Normal(offset)) => {
                        let mut reader = Reader::new(&data.as_ref()[*offset..]);

                        if let Some(obj) =
                            reader.read_with_context::<IndirectObject<Dict<'_>>>(&dummy_ctx)
                            && {
                                let obj = obj.get();
                                check(&obj)
                            }
                        {
                            trailer_dict = Some(dict);
                        }
                    }
                    Some(EntryType::ObjStream(obj_num, idx)) => {
                        if let Some(EntryType::Normal(offset)) =
                            xref_map.get(&ObjectIdentifier::new(*obj_num as i32, 0))
                        {
                            let mut reader = Reader::new(&data.as_ref()[*offset..]);

                            if let Some(stream) =
                                reader.read_with_context::<IndirectObject<Stream<'_>>>(&dummy_ctx)
                                && {
                                    let stream = stream.get();
                                    if let Some(data) = stream.decoded().ok()
                                        && let Some(object_stream) =
                                            ObjectStream::new(stream, &data, &dummy_ctx)
                                        && let Some(obj) = object_stream.get::<Dict<'_>>(*idx)
                                    {
                                        check(&obj)
                                    } else {
                                        false
                                    }
                                }
                            {
                                trailer_dict = Some(dict);
                            }
                        }
                    }
                    _ => {}
                },
                MaybeRef::NotRef(d) => {
                    if check(&d) {
                        trailer_dict = Some(dict);
                    }
                }
            }
        }
    }

    let has_encryption = trailer_dict
        .as_ref()
        .is_some_and(|t| t.contains_key(ENCRYPT));

    if has_encryption && recurse {
        // The problem is that in this case, we have used a dummy reader context which does not have
        // a decryptor. Therefore, we were unable to decrypt any of the object streams and missed
        // all objects that are inside of such a stream. Therefore, we need to redo the process
        // using a `ReaderContext` that does have the ability to decrypt.
        if let Ok(xref) = XRef::new(
            data.clone(),
            xref_map.clone(),
            XRefInput::TrailerDictData(trailer_dict.as_ref().map(|d| d.data()).unwrap()),
            true,
            password,
        ) {
            let ctx = ReaderContext::new(&xref, false);
            let (patched_map, _) = fallback_xref_map_inner(data, ctx, false, password);
            xref_map = patched_map;
        }
    }

    if let Some(trailer_dict_data) = trailer_dict.map(|d| d.data()) {
        (
            xref_map,
            Some(XRefInput::TrailerDictData(trailer_dict_data)),
        )
    } else if let Some(root_ref) = root_ref {
        (xref_map, Some(XRefInput::RootRef(root_ref)))
    } else {
        (xref_map, None)
    }
}

const DUMMY_XREF: XRef = XRef(Inner::Dummy);

/// An xref table.
#[derive(Debug, Clone)]
pub struct XRef(Inner);

impl XRef {
    // PdfCraft patch: keep the bounded dictionary-index cache local to this PDF.
    pub(crate) fn with_dict_cache<T>(&self, f: impl FnOnce(&mut DictCache) -> T) -> Option<T> {
        let Inner::Some(repr) = &self.0 else {
            return None;
        };
        #[cfg(feature = "std")]
        let mut cache = repr
            .dict_cache
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        #[cfg(not(feature = "std"))]
        let mut cache = repr.dict_cache.try_borrow_mut().ok()?;
        Some(f(&mut cache))
    }

    /// PdfCraft patch: (hayro-syntax (22)) run `f` on this PDF's parsed object stream indexes
    /// ([`ObjectStreamIndexes`]). `None` when there is none to use (the dummy xref, or a no_std
    /// cache already borrowed), and the caller then parses the index without it. A poisoned lock
    /// is still used: entries are complete `Arc`s inserted in one step, so a panic elsewhere
    /// cannot have left one half-written.
    fn with_object_stream_indexes<T>(
        &self,
        f: impl FnOnce(&mut ObjectStreamIndexes) -> T,
    ) -> Option<T> {
        let Inner::Some(repr) = &self.0 else {
            return None;
        };
        #[cfg(feature = "std")]
        let mut indexes = repr
            .object_stream_indexes
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        #[cfg(not(feature = "std"))]
        let mut indexes = repr.object_stream_indexes.try_borrow_mut().ok()?;
        Some(f(&mut indexes))
    }

    fn new(
        data: PdfData,
        xref_map: XrefMap,
        input: XRefInput<'_>,
        repaired: bool,
        password: &[u8],
    ) -> Result<Self, XRefError> {
        // This is a bit hacky, but the problem is we can't read the resolved trailer dictionary
        // before we actually created the xref struct. So we first create it using dummy data
        // and then populate the data.
        let trailer_data = TrailerData::dummy();

        let mut xref = Self(Inner::Some(Arc::new(SomeRepr {
            data: Arc::new(Data::new(data)),
            map: Arc::new(RwLock::new(MapRepr { xref_map, repaired })),
            decryptor: Arc::new(Decryptor::None),
            has_ocgs: false,
            metadata: Arc::new(Metadata::default()),
            trailer_data,
            password: password.to_vec(),
            dict_cache: Arc::new(Mutex::new(DictCache::default())),
            object_stream_indexes: Arc::new(Mutex::new(ObjectStreamIndexes::default())),
        })));

        // We read the trailer twice, once to determine the encryption used and then a second
        // time to resolve the catalog dictionary, etc. This allows us to support catalog dictionaries
        // that are stored in an encrypted object stream.

        let decryptor = {
            match input {
                XRefInput::TrailerDictData(trailer_dict_data) => {
                    let mut r = Reader::new(trailer_dict_data);

                    let trailer_dict = r
                        .read_with_context::<Dict<'_>>(&ReaderContext::new(&xref, false))
                        .ok_or(XRefError::Unknown)?;

                    get_decryptor(&trailer_dict, password)?
                }
                XRefInput::RootRef(_) => Decryptor::None,
            }
        };

        match &mut xref.0 {
            Inner::Dummy => unreachable!(),
            Inner::Some(r) => {
                let mutable = Arc::make_mut(r);
                mutable.decryptor = Arc::new(decryptor.clone());
            }
        }

        let (trailer_data, has_ocgs, metadata) = match input {
            XRefInput::TrailerDictData(trailer_dict_data) => {
                let mut r = Reader::new(trailer_dict_data);

                let trailer_dict = r
                    .read_with_context::<Dict<'_>>(&ReaderContext::new(&xref, false))
                    .ok_or(XRefError::Unknown)?;

                let root_ref = trailer_dict.get_ref(ROOT).ok_or(XRefError::Unknown)?;
                let root = trailer_dict
                    .get::<Dict<'_>>(ROOT)
                    .ok_or(XRefError::Unknown)?;
                let metadata = trailer_dict
                    .get::<Dict<'_>>(INFO)
                    .map(|d| parse_metadata(&d))
                    .unwrap_or_default();
                let pages_ref = root.get_ref(PAGES).ok_or(XRefError::Unknown)?;
                let has_ocgs = root.get::<Dict<'_>>(OCPROPERTIES).is_some();
                let version = root
                    .get::<Name<'_>>(VERSION)
                    .and_then(|v| PdfVersion::from_bytes(v.deref()));

                let td = TrailerData {
                    pages_ref: pages_ref.into(),
                    root_ref: root_ref.into(),
                    version,
                };

                (td, has_ocgs, metadata)
            }
            XRefInput::RootRef(root_ref) => {
                let root = xref.get::<Dict<'_>>(root_ref).ok_or(XRefError::Unknown)?;
                let pages_ref = root.get_ref(PAGES).ok_or(XRefError::Unknown)?;

                let td = TrailerData {
                    pages_ref: pages_ref.into(),
                    root_ref,
                    version: None,
                };

                (td, false, Metadata::default())
            }
        };

        match &mut xref.0 {
            Inner::Dummy => unreachable!(),
            Inner::Some(r) => {
                let mutable = Arc::make_mut(r);
                mutable.trailer_data = trailer_data;
                mutable.decryptor = Arc::new(decryptor);
                mutable.has_ocgs = has_ocgs;
                mutable.metadata = Arc::new(metadata);
            }
        }

        Ok(xref)
    }

    pub(crate) fn dummy() -> &'static Self {
        &DUMMY_XREF
    }

    pub(crate) fn len(&self) -> usize {
        match &self.0 {
            Inner::Dummy => 0,
            Inner::Some(r) => r.map.get().xref_map.len(),
        }
    }

    pub(crate) fn trailer_data(&self) -> &TrailerData {
        match &self.0 {
            Inner::Dummy => unreachable!(),
            Inner::Some(r) => &r.trailer_data,
        }
    }

    pub(crate) fn metadata(&self) -> &Metadata {
        match &self.0 {
            Inner::Dummy => unreachable!(),
            Inner::Some(r) => &r.metadata,
        }
    }

    /// Return the object ID of the root dictionary.
    pub fn root_id(&self) -> ObjectIdentifier {
        self.trailer_data().root_ref
    }

    /// Whether the PDF has optional content groups.
    pub fn has_optional_content_groups(&self) -> bool {
        match &self.0 {
            Inner::Dummy => false,
            Inner::Some(r) => r.has_ocgs,
        }
    }

    pub(crate) fn objects(&self) -> impl IntoIterator<Item = Object<'_>> + '_ {
        match &self.0 {
            Inner::Dummy => unimplemented!(),
            Inner::Some(r) => {
                let locked = r.map.get();
                let mut elements = locked
                    .xref_map
                    .iter()
                    .map(|(id, e)| {
                        let offset = match e {
                            EntryType::Normal(o) => (*o, 0),
                            EntryType::ObjStream(id, index) => {
                                if let Some(EntryType::Normal(offset)) =
                                    locked.xref_map.get(&ObjectIdentifier::new(*id as i32, 0))
                                {
                                    (*offset, *index)
                                } else {
                                    (usize::MAX, 0)
                                }
                            }
                        };

                        (*id, offset)
                    })
                    .collect::<Vec<_>>();

                // Try to yield in the order the objects appeared in the
                // PDF.
                elements.sort_by(|e1, e2| e1.1.cmp(&e2.1));
                // PdfCraft patch: one object per location. A rebuilt table can hold two keys for
                // one object -- its own generation and the generation-0 stand-in (#5580) -- and
                // enumeration yielded the object once for each, so a document recovered by
                // `Pages::new_brute_force` reported the page twice (noted in review). Both keys
                // stay for lookups; only this walk is deduplicated. The object number is part of
                // the key: an object stream at (offset, 0) and its first member, also
                // (offset, 0), are two objects.
                elements.dedup_by(|a, b| a.1 == b.1 && a.0.obj_number == b.0.obj_number);

                let mut iter = elements.into_iter();

                iter::from_fn(move || {
                    for next in iter.by_ref() {
                        // PdfCraft patch: (hayro-syntax (33)) an object that only refers to another
                        // isn't one of its own: followed, that one came out once more for each.
                        if let Some(Fetched::Object(obj)) =
                            self.fetch(next.0, &ReaderContext::new(self, false))
                        {
                            return Some(obj);
                        } else {
                            // Skip invalid objects.
                            continue;
                        }
                    }

                    None
                })
            }
        }
    }

    pub(crate) fn repair(&self) {
        let Inner::Some(r) = &self.0 else {
            return;
        };

        // All callers release read guards before following references/repairing.
        // Fallback reconstruction uses its own dummy/temporary xref, never this
        // locked map. Serialize the one repair and let concurrent readers wait.
        let mut locked = r.map.put();
        if locked.repaired {
            return;
        }
        // Set first so an unwinding fallback cannot trigger a reparse storm.
        // Assignment below publishes a complete replacement atomically.
        locked.repaired = true;
        let (xref_map, _) = fallback_xref_map(r.data.get(), &r.password);
        locked.xref_map = xref_map;
    }

    #[inline]
    pub(crate) fn needs_decryption(&self, ctx: &ReaderContext<'_>) -> bool {
        match &self.0 {
            Inner::Dummy => false,
            Inner::Some(r) => {
                if matches!(r.decryptor.as_ref(), Decryptor::None) {
                    false
                } else {
                    !ctx.in_content_stream() && !ctx.in_object_stream()
                }
            }
        }
    }

    #[inline]
    pub(crate) fn decrypt(
        &self,
        id: ObjectIdentifier,
        data: &[u8],
        target: DecryptionTarget<'_>,
    ) -> Option<Vec<u8>> {
        match &self.0 {
            Inner::Dummy => Some(data.to_vec()),
            Inner::Some(r) => r.decryptor.decrypt(id, data, target),
        }
    }

    /// Return the object with the given identifier.
    #[allow(private_bounds)]
    pub fn get<'a, T>(&'a self, id: ObjectIdentifier) -> Option<T>
    where
        T: ObjectLike<'a>,
    {
        let ctx = ReaderContext::new(self, false);
        self.get_with(id, &ctx)
    }

    /// Return the object with the given identifier.
    ///
    /// PdfCraft patch: (hayro-syntax (33)) an object that holds only a reference stands for what
    /// that names. Read as a number it was the reference's own object number, and as anything else
    /// it was lost. This is a loop, so a long chain takes no stack.
    #[allow(private_bounds)]
    pub(crate) fn get_with<'a, T>(
        &'a self,
        id: ObjectIdentifier,
        ctx: &ReaderContext<'a>,
    ) -> Option<T>
    where
        T: ObjectLike<'a>,
    {
        let mut id = id;
        for _ in 0..=MAX_REFERENCE_HOPS {
            match self.fetch::<T>(id, ctx)? {
                Fetched::Object(object) => return Some(object),
                Fetched::Reference(next) => id = next,
            }
        }
        warn!("references lead on past the limit, or in a cycle: {id:?}");

        None
    }

    /// PdfCraft patch: (hayro-syntax (33)) the object stream `id`. It has to be a stream stored as
    /// that object (ISO 32000-2 7.5.7), so this follows no reference: the check of where it is stored
    /// is of that object, and one reached through a reference could be a stream stored inside
    /// another.
    pub(crate) fn object_stream<'a>(
        &'a self,
        id: ObjectIdentifier,
        ctx: &ReaderContext<'a>,
    ) -> Option<Stream<'a>> {
        match self.fetch::<Stream<'a>>(id, ctx)? {
            Fetched::Object(stream) => Some(stream),
            Fetched::Reference(_) => None,
        }
    }

    /// What is stored at the object with the given identifier.
    fn fetch<'a, T>(&'a self, id: ObjectIdentifier, ctx: &ReaderContext<'a>) -> Option<Fetched<T>>
    where
        T: ObjectLike<'a>,
    {
        let Inner::Some(repr) = &self.0 else {
            return None;
        };

        let locked = repr.map.get();

        let mut r = Reader::new(repr.data.get().as_ref());

        let entry = *locked.xref_map.get(&id).or({
            // An indirect reference to an undefined object shall not be considered an error by a PDF processor; it
            // shall be treated as a reference to the null object.
            None
        })?;
        let repaired = locked.repaired;
        drop(locked);

        let mut ctx = ctx.clone();
        ctx.set_obj_number(id);
        ctx.set_in_content_stream(false);

        match entry {
            EntryType::Normal(offset) => {
                ctx.set_in_object_stream(false);
                r.jump(offset);

                if let Some(next) = held_reference(r.clone(), id) {
                    return Some(Fetched::Reference(next));
                }

                if let Some(object) = r.read_with_context::<IndirectObject<T>>(&ctx) {
                    // PdfCraft patch: in a table rebuilt by scanning the file, the object number
                    // is what identifies an object. There is no revision history to tell two
                    // generations apart, the scan already files objects found in object streams
                    // under generation 0, and a malformed generation in a header would otherwise
                    // make the object unreadable through the reference that names it (#5580). A
                    // table read from the file's own xref keeps the exact match.
                    if object.id() == &id || (repaired && object.id().obj_number == id.obj_number) {
                        return Some(Fetched::Object(object.get()));
                    }
                } else {
                    // PdfCraft patch: the strict read of the header failed. In a table rebuilt by
                    // scanning the file that can be a generation number no generation may be --
                    // `3 18446744073709551616 obj` -- and refusing it loses the object the
                    // reference names (#5580). The object number still has to be the one asked
                    // for; only an unreadable generation is tolerated, and only here.
                    if repaired
                        && let Some(object) =
                            read_repaired_body::<T>(repr.data.get().as_ref(), offset, id, &ctx)
                    {
                        return Some(Fetched::Object(object));
                    }
                    // There is a valid object at the offset, it's just not of the type the caller
                    // expected, which is fine.
                    if r.skip::<IndirectObject<Object<'_>>>(false).is_some() {
                        return None;
                    }
                };

                // The xref table is broken, try to repair if not already repaired.
                if repaired {
                    error!(
                        "attempt was made at repairing xref, but object {id:?} still couldn't be read"
                    );

                    None
                } else {
                    warn!("broken xref, attempting to repair");

                    self.repair();

                    // Now try reading again.
                    self.fetch::<T>(id, &ctx)
                }
            }
            EntryType::ObjStream(obj_stram_gen_num, index) => {
                // Generation number is implicitly 0.
                let obj_stream_id = ObjectIdentifier::new(obj_stram_gen_num as i32, 0);

                if obj_stream_id == id {
                    warn!("cycle detected in object stream");

                    return None;
                }

                // Stream objects cannot themselves be stored in object streams
                // (PDF 7.5.7). Reject malformed chains before recursively loading
                // them, including A-in-B/B-in-A and arbitrarily deep chains.
                if !matches!(
                    repr.map.get().xref_map.get(&obj_stream_id),
                    Some(EntryType::Normal(_))
                ) {
                    return None;
                }

                // PdfCraft patch: (hayro-syntax (22)) the stream's index of `objnum offset` pairs
                // is parsed once and kept, as its decoded data is: parsed again for every object
                // fetched from it, reading all of a stream of N objects took N² steps (one
                // Firefox page kept 65,542 objects in one stream and took 37 s to draw).
                let data = repr.data.get_with(obj_stream_id, &ctx)?;
                let known = self
                    .with_object_stream_indexes(|indexes| indexes.get(&obj_stream_id).cloned())
                    .flatten();
                let offsets = match known {
                    // A stream whose pairs don't parse stays unreadable, as it was.
                    Some(offsets) => offsets?,
                    None => {
                        // PdfCraft patch: (hayro-syntax (33)) this read can reach another object
                        // stream, whose `/N` or `/First` can be a member of the next: the gate's
                        // limit holds here too, or a chain of them ran out of stack. It reads the
                        // dictionary, which is all it needs: parsing the stream reads `/Length`
                        // too, a second way to the next stream's index, which doubled at each.
                        let _loading = repr.data.enter_loading(obj_stream_id)?;
                        let Fetched::Object(dict) = self.fetch::<Dict<'_>>(obj_stream_id, &ctx)?
                        else {
                            return None;
                        };
                        // `/N` and `/First` may be references that can't be followed while
                        // another stream loads: reading them fails without being remembered.
                        let num_objects = dict.get::<usize>(N)?;
                        let first_offset = dict.get::<usize>(FIRST)?;
                        let parsed =
                            read_object_stream_index(num_objects, first_offset, data).map(Arc::new);
                        self.with_object_stream_indexes(|indexes| {
                            indexes
                                .entry(obj_stream_id)
                                .or_insert_with(|| parsed.clone())
                                .clone()
                        })
                        .unwrap_or(parsed)?
                    }
                };
                let stream = ObjectStream::with_offsets(data, &ctx, offsets);
                if let Some(next) = stream.reference(index) {
                    return Some(Fetched::Reference(next));
                }

                stream.get(index).map(Fetched::Object)
            }
        }
    }
}

/// An input that is passed to the xref constructor so that we can fully resolve
/// the PDF.
#[derive(Debug, Copy, Clone)]
pub(crate) enum XRefInput<'a> {
    /// This option is going to be uesd in 99.999% of the case. It contains the
    /// raw data of the trailer dictionary which is then going to be processed.
    TrailerDictData(&'a [u8]),
    /// In case the trailer dictionary could not be read (for example because
    /// it is cut-off), we just pass the object ID of the root dictionary
    /// in case we have found one, and try our best to build the PDF just
    /// with the information we have there.
    ///
    /// Note that this won't work if the document is encrypted, as we
    /// can't access the crypto dictionary.
    RootRef(ObjectIdentifier),
}

pub(crate) fn find_last_xref_pos(data: &[u8]) -> Option<usize> {
    let needle = b"startxref";
    let pos = findr_needle(data, needle)?;
    let mut finder = Reader::new(data);
    finder.jump(pos);
    finder.forward_tag(needle)?;
    finder.skip_white_spaces_and_comments();
    finder.read_without_context::<i32>()?.try_into().ok()
}

/// A type of xref entry.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum EntryType {
    /// An indirect object that is at a specific offset in the original data.
    Normal(usize),
    /// An indirect object that is part of an object stream. First number indicates the object
    /// number of the _object stream_ (the generation number is always 0), the second number indicates
    /// the index in the object stream.
    ObjStream(u32, u32),
}

type XrefMap = FxHashMap<ObjectIdentifier, EntryType>;

/// Representation of a proper xref table.
#[derive(Debug)]
struct MapRepr {
    xref_map: XrefMap,
    repaired: bool,
}

#[derive(Debug, Copy, Clone)]
pub(crate) struct TrailerData {
    pub(crate) pages_ref: ObjectIdentifier,
    pub(crate) root_ref: ObjectIdentifier,
    pub(crate) version: Option<PdfVersion>,
}

impl TrailerData {
    pub(crate) fn dummy() -> Self {
        Self {
            pages_ref: ObjectIdentifier::new(0, 0),
            root_ref: ObjectIdentifier::new(0, 0),
            version: None,
        }
    }
}

#[derive(Debug, Clone)]
struct SomeRepr {
    dict_cache: Arc<Mutex<DictCache>>,
    object_stream_indexes: Arc<Mutex<ObjectStreamIndexes>>,
    data: Arc<Data>,
    map: Arc<RwLock<MapRepr>>,
    metadata: Arc<Metadata>,
    decryptor: Arc<Decryptor>,
    has_ocgs: bool,
    password: Vec<u8>,
    trailer_data: TrailerData,
}

#[derive(Debug, Clone)]
enum Inner {
    /// A dummy xref table that doesn't have any entries.
    Dummy,
    /// A proper xref table.
    Some(Arc<SomeRepr>),
}

#[derive(Debug)]
struct XRefEntry {
    offset: usize,
    gen_number: i32,
    used: bool,
}

impl XRefEntry {
    pub(crate) fn read(data: &[u8]) -> Option<Self> {
        #[inline(always)]
        fn parse_u32(data: &[u8]) -> Option<u32> {
            let mut accum = 0_u32;

            for byte in data {
                accum = accum.checked_mul(10)?;

                match *byte {
                    b'0'..=b'9' => accum = accum.checked_add((*byte - b'0') as u32)?,
                    _ => return None,
                }
            }

            Some(accum)
        }

        let offset = parse_u32(&data[0..10])? as usize;
        let gen_number = i32::try_from(parse_u32(&data[11..16])?).ok()?;

        let used = data[17] == b'n';

        Some(Self {
            offset,
            gen_number,
            used,
        })
    }
}

fn populate_xref_impl<'a>(data: &'a [u8], pos: usize, xref_map: &mut XrefMap) -> Option<&'a [u8]> {
    let mut visited = BTreeSet::new();
    populate_xref_impl_inner(data, pos, xref_map, &mut visited)
}

/// The offset of the previous xref section (`Prev`).
// PdfCraft patch: (#2366) some writers give `Prev` as a reference `N G R`. When this section's
// own entries (`own`) place that object and it holds an offset (iText's `indirectPrev.pdf`), that
// is the offset; otherwise its object number is (tiff2pdf, in JHOVE's PDF-HUL-157 files), as
// `pdfcraft-cos` and other readers take it.
fn prev_offset(data: &[u8], trailer: &Dict<'_>, own: &XrefMap) -> Option<usize> {
    if let Some(prev) = trailer.get::<i32>(PREV) {
        return usize::try_from(prev).ok();
    }

    let r = trailer.get_ref(PREV)?;
    let id = ObjectIdentifier::new(r.obj_number, r.gen_number);
    let held = match own.get(&id) {
        Some(EntryType::Normal(at)) => {
            let mut reader = Reader::new(data);
            reader.jump(*at);
            reader
                .read_with_context::<IndirectObject<i32>>(&ReaderContext::dummy())
                .filter(|o| o.id() == &id)
                .map(|o| o.get())
                .filter(|p| *p >= 0)
        }
        _ => None,
    };

    usize::try_from(held.unwrap_or(r.obj_number)).ok()
}

/// Maximum number of allowed xref `Prev` pointers before we abort.
const MAX_XREF_CHAIN_DEPTH: usize = 256;

fn populate_xref_impl_inner<'a>(
    data: &'a [u8],
    pos: usize,
    xref_map: &mut XrefMap,
    visited: &mut BTreeSet<usize>,
) -> Option<&'a [u8]> {
    if !visited.insert(pos) {
        warn!("circular xref PREV chain detected at offset {}", pos);

        return None;
    }

    if visited.len() > MAX_XREF_CHAIN_DEPTH {
        warn!(
            "xref PREV chain exceeds maximum depth of {}",
            MAX_XREF_CHAIN_DEPTH
        );

        return None;
    }

    let mut reader = Reader::new(data);
    reader.jump(pos);
    // In case the position points to before the object number of a xref stream.
    reader.skip_white_spaces_and_comments();

    let mut r2 = reader.clone();
    if reader
        .clone()
        .read_without_context::<ObjectIdentifier>()
        .is_some()
    {
        populate_from_xref_stream(data, &mut r2, xref_map, visited)
    } else {
        populate_from_xref_table(data, &mut r2, xref_map, visited)
    }
}

pub(super) struct SubsectionHeader {
    pub(super) start: u32,
    pub(super) num_entries: u32,
}

impl Readable<'_> for SubsectionHeader {
    fn read(r: &mut Reader<'_>, _: &ReaderContext<'_>) -> Option<Self> {
        r.skip_white_spaces();
        let start = r.read_without_context::<u32>()?;
        r.skip_white_spaces();
        let num_entries = r.read_without_context::<u32>()?;
        r.skip_white_spaces();

        Some(Self { start, num_entries })
    }
}

/// Populate the xref table, and return the trailer dict.
fn populate_from_xref_table<'a>(
    data: &'a [u8],
    reader: &mut Reader<'a>,
    insert_map: &mut XrefMap,
    visited: &mut BTreeSet<usize>,
) -> Option<&'a [u8]> {
    let trailer = {
        let mut reader = reader.clone();
        read_xref_table_trailer(&mut reader, &ReaderContext::dummy())?
    };

    reader.skip_white_spaces();
    reader.forward_tag(b"xref")?;
    reader.skip_white_spaces();

    let mut max_obj = 0;
    // PdfCraft patch: (#2366) this section's entries are read first, for `prev_offset`.
    let mut own = XrefMap::default();

    while let Some(header) = reader.read_without_context::<SubsectionHeader>() {
        reader.skip_white_spaces();

        let start = header.start;
        let end = start.checked_add(header.num_entries)?;

        for obj_number in start..end {
            max_obj = max(max_obj, obj_number);
            let bytes = reader.read_bytes(XREF_ENTRY_LEN)?;
            let entry = XRefEntry::read(bytes)?;

            // Specification says we should ignore any object number > SIZE, but probably
            // not important?
            if entry.used {
                own.insert(
                    ObjectIdentifier::new(obj_number as i32, entry.gen_number),
                    EntryType::Normal(entry.offset),
                );
            }
        }
    }

    if let Some(prev) = prev_offset(data, &trailer, &own) {
        // First insert the entries from any previous xref tables.
        populate_xref_impl_inner(data, prev, insert_map, visited)?;
    }

    // In hybrid files, entries in `XRefStm` should have higher priority, therefore we insert them
    // after looking at `PREV`.
    if let Some(xref_stm) = trailer.get::<i32>(XREF_STM) {
        populate_xref_impl_inner(data, xref_stm as usize, insert_map, visited)?;
    }

    insert_map.extend(own);

    Some(trailer.data())
}

fn populate_from_xref_stream<'a>(
    data: &'a [u8],
    reader: &mut Reader<'a>,
    insert_map: &mut XrefMap,
    visited: &mut BTreeSet<usize>,
) -> Option<&'a [u8]> {
    let stream = reader
        .read_with_context::<IndirectObject<Stream<'_>>>(&ReaderContext::dummy())?
        .get();

    let size = stream.dict().get::<u32>(SIZE)?;

    let [f1_len, f2_len, f3_len] = stream.dict().get::<[u8; 3]>(W)?;

    // PdfCraft patch: the type field is read at its full width, so it is bounded like the offset.
    if f1_len > size_of::<u64>() as u8 || f2_len > size_of::<u64>() as u8 {
        error!("xref field length is larger than the allowed limit");

        return None;
    }

    let xref_data = stream.decoded().ok()?;
    // PdfCraft patch: (#2366) this section's entries are read first, for `prev_offset`.
    let mut own = XrefMap::default();
    let mut xref_reader = Reader::new(xref_data.as_ref());

    if let Some(arr) = stream.dict().get::<Array<'_>>(INDEX) {
        let iter = arr.iter::<(u32, u32)>();

        for (start, num_elements) in iter {
            xref_stream_subsection(
                &mut xref_reader,
                start,
                num_elements,
                f1_len,
                f2_len,
                f3_len,
                &mut own,
            )?;
        }
    } else {
        xref_stream_subsection(&mut xref_reader, 0, size, f1_len, f2_len, f3_len, &mut own)?;
    }

    if let Some(prev) = prev_offset(data, stream.dict(), &own) {
        // First insert the entries from any previous xref tables.
        let _ = populate_xref_impl_inner(data, prev, insert_map, visited)?;
    }

    insert_map.extend(own);

    Some(stream.dict().data())
}

fn xref_stream_num(data: &[u8]) -> Option<u32> {
    Some(match data.len() {
        0 => return None,
        1 => u8::from_be(data[0]) as u32,
        2 => u16::from_be_bytes(data[0..2].try_into().ok()?) as u32,
        3 => u32::from_be_bytes([0, data[0], data[1], data[2]]),
        4 => u32::from_be_bytes(data[0..4].try_into().ok()?),
        8 => {
            if let Ok(num) = u32::try_from(u64::from_be_bytes(data[0..8].try_into().ok()?)) {
                return Some(num);
            } else {
                warn!("xref stream number is too large");

                return None;
            }
        }
        _n => {
            warn!("invalid xref stream number {_n}");

            return None;
        }
    })
}

fn xref_stream_subsection<'a>(
    xref_reader: &mut Reader<'a>,
    start: u32,
    num_elements: u32,
    f1_len: u8,
    f2_len: u8,
    f3_len: u8,
    insert_map: &mut XrefMap,
) -> Option<()> {
    // PdfCraft patch: every entry must be backed by stream bytes. With all widths zero an entry
    // reads nothing, so a tiny stream could declare billions of entries; and a count past the
    // decoded data is refused before any of it is inserted.
    let width = f1_len as usize + f2_len as usize + f3_len as usize;
    if width == 0 {
        warn!("xref stream entries have no width");

        return None;
    }
    let available = xref_reader.len().saturating_sub(xref_reader.offset());
    if (num_elements as usize)
        .checked_mul(width)
        .is_none_or(|need| need > available)
    {
        warn!("xref stream subsection declares more entries than its data holds");

        return None;
    }

    for i in 0..num_elements {
        // A zero-width type field defaults to 1 (ISO 32000-2 §7.5.8.2, `/W`).
        let f_type = if f1_len == 0 {
            1
        } else {
            // PdfCraft patch: the type field is read big-endian at its full `/W[0]` width.
            let bytes = xref_reader.read_bytes(f1_len as usize)?;
            bytes.iter().fold(0u64, |acc, b| (acc << 8) | u64::from(*b))
        };

        // PdfCraft patch: object numbers are stored as `i32`; a subsection running past
        // `i32::MAX` is refused instead of overflowing or turning negative.
        let obj_number = i32::try_from(start.checked_add(i)?).ok()?;

        match f_type {
            // We don't care about free objects.
            0 => {
                xref_reader.skip_bytes(f2_len as usize + f3_len as usize)?;
            }
            1 => {
                let offset = if f2_len > 0 {
                    let data = xref_reader.read_bytes(f2_len as usize)?;
                    xref_stream_num(data)?
                } else {
                    0
                };

                let gen_number = if f3_len > 0 {
                    let data = xref_reader.read_bytes(f3_len as usize)?;
                    xref_stream_num(data)?
                } else {
                    0
                };

                insert_map.insert(
                    ObjectIdentifier::new(obj_number, gen_number as i32),
                    EntryType::Normal(offset as usize),
                );
            }
            2 => {
                let obj_stream_number = {
                    let data = xref_reader.read_bytes(f2_len as usize)?;
                    xref_stream_num(data)?
                };
                let gen_number = 0;
                let index = if f3_len > 0 {
                    let data = xref_reader.read_bytes(f3_len as usize)?;
                    xref_stream_num(data)?
                } else {
                    0
                };

                insert_map.insert(
                    ObjectIdentifier::new(obj_number, gen_number),
                    EntryType::ObjStream(obj_stream_number, index),
                );
            }
            _ => {
                warn!("xref has unknown field type {f_type}");

                return None;
            }
        }
    }

    Some(())
}

fn read_xref_table_trailer<'a>(
    reader: &mut Reader<'a>,
    ctx: &ReaderContext<'a>,
) -> Option<Dict<'a>> {
    reader.skip_white_spaces();
    reader.forward_tag(b"xref")?;
    reader.skip_white_spaces();

    while let Some(header) = reader.read_without_context::<SubsectionHeader>() {
        let len = XREF_ENTRY_LEN.checked_mul(header.num_entries as usize)?;
        reader.jump(reader.offset().checked_add(len)?);
    }

    reader.skip_white_spaces();
    reader.forward_tag(b"trailer")?;
    reader.skip_white_spaces();

    reader.read_with_context::<Dict<'_>>(ctx)
}

fn get_decryptor(trailer_dict: &Dict<'_>, password: &[u8]) -> Result<Decryptor, XRefError> {
    if let Some(encryption_dict) = trailer_dict.get::<Dict<'_>>(ENCRYPT) {
        let id = if let Some(id) = trailer_dict
            .get::<Array<'_>>(ID)
            .and_then(|a| a.flex_iter().next::<object::String<'_>>())
        {
            id.to_vec()
        } else {
            // Assume an empty ID entry.
            vec![]
        };

        get(&encryption_dict, &id, password).map_err(XRefError::Encryption)
    } else {
        Ok(Decryptor::None)
    }
}

/// PdfCraft patch: (hayro-syntax (22)) each object stream's parsed `(object number, offset)`
/// pairs ([`read_object_stream_index`]) by the stream's identifier, `None` for pairs that don't
/// parse. Never cleared, like the decoded stream data they index (`Data`), which a repair keeps
/// too. Bounded by that data: every pair takes at least three of its bytes.
type ObjectStreamIndexes = FxHashMap<ObjectIdentifier, Option<Arc<Vec<(u32, usize)>>>>;

/// The `(object number, offset)` pairs at the start of an object stream's decoded `data`, the
/// offsets measured from the start of `data`. `None` when the pairs don't parse. Reads no
/// further than the data holds, whatever `/N` claims.
fn read_object_stream_index(
    num_objects: usize,
    first_offset: usize,
    data: &[u8],
) -> Option<Vec<(u32, usize)>> {
    let mut r = Reader::new(data);

    let mut offsets = vec![];

    for _ in 0..num_objects {
        r.skip_white_spaces_and_comments();
        // Skip object number
        let obj_num = r.read_without_context::<u32>()?;
        r.skip_white_spaces_and_comments();
        let relative_offset = r.read_without_context::<usize>()?;
        // PdfCraft patch: (hayro-syntax (22)) an offset past `usize` is malformed (it wrapped).
        offsets.push((obj_num, first_offset.checked_add(relative_offset)?));
    }

    Some(offsets)
}

struct ObjectStream<'a> {
    data: &'a [u8],
    ctx: ReaderContext<'a>,
    offsets: Arc<Vec<(u32, usize)>>,
}

impl<'a> ObjectStream<'a> {
    fn new(inner: Stream<'_>, data: &'a [u8], ctx: &ReaderContext<'a>) -> Option<Self> {
        let num_objects = inner.dict().get::<usize>(N)?;
        let first_offset = inner.dict().get::<usize>(FIRST)?;
        let offsets = read_object_stream_index(num_objects, first_offset, data)?;

        Some(Self::with_offsets(data, ctx, Arc::new(offsets)))
    }

    /// PdfCraft patch: (hayro-syntax (22)) an object stream whose index is already parsed.
    fn with_offsets(
        data: &'a [u8],
        ctx: &ReaderContext<'a>,
        offsets: Arc<Vec<(u32, usize)>>,
    ) -> Self {
        let mut ctx = ctx.clone();
        ctx.set_in_object_stream(true);

        Self { data, ctx, offsets }
    }

    fn get<T>(&self, index: u32) -> Option<T>
    where
        T: ObjectLike<'a>,
    {
        let offset = self.offsets.get(index as usize)?.1;
        let mut r = Reader::new(self.data);
        r.jump(offset);
        r.skip_white_spaces_and_comments();

        r.read_with_context::<T>(&self.ctx)
    }

    /// PdfCraft patch: (hayro-syntax (33)) what the object at `index` refers to, when a reference is
    /// all it holds.
    fn reference(&self, index: u32) -> Option<ObjectIdentifier> {
        let offset = self.offsets.get(index as usize)?.1;
        let mut r = Reader::new(self.data);
        r.jump(offset);
        r.skip_white_spaces_and_comments();

        Some(r.read_without_context::<ObjRef>()?.into())
    }
}

/// PdfCraft patch: (hayro-syntax (33)) what the indirect object `id` at `r` refers to, when a
/// reference is all it holds.
fn held_reference(mut r: Reader<'_>, id: ObjectIdentifier) -> Option<ObjectIdentifier> {
    if r.read_without_context::<ObjectIdentifier>()? != id {
        return None;
    }
    r.skip_white_spaces_and_comments();

    Some(r.read_without_context::<ObjRef>()?.into())
}

fn parse_metadata(info_dict: &Dict<'_>) -> Metadata {
    Metadata {
        creation_date: info_dict
            .get::<object::String<'_>>(CREATION_DATE)
            .and_then(|c| DateTime::from_bytes(&c)),
        modification_date: info_dict
            .get::<object::String<'_>>(MOD_DATE)
            .and_then(|c| DateTime::from_bytes(&c)),
        title: info_dict
            .get::<object::String<'_>>(TITLE)
            .map(|t| t.to_vec()),
        author: info_dict
            .get::<object::String<'_>>(AUTHOR)
            .map(|t| t.to_vec()),
        subject: info_dict
            .get::<object::String<'_>>(SUBJECT)
            .map(|t| t.to_vec()),
        keywords: info_dict
            .get::<object::String<'_>>(KEYWORDS)
            .map(|t| t.to_vec()),
        creator: info_dict
            .get::<object::String<'_>>(CREATOR)
            .map(|t| t.to_vec()),
        producer: info_dict
            .get::<object::String<'_>>(PRODUCER)
            .map(|t| t.to_vec()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Pdf;

    /// A file with an xref stream: `stored` are objects on their own, as they would follow
    /// `n 0 obj`, and `packed` says of each object in an object stream which stream and place.
    fn with_xref_stream(stored: &[(u32, Vec<u8>)], packed: &[(u32, u32, u16)]) -> Vec<u8> {
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = alloc::collections::BTreeMap::new();
        for (num, body) in stored {
            offsets.insert(*num, out.len());
            out.extend_from_slice(alloc::format!("{num} 0 obj ").as_bytes());
            out.extend_from_slice(body);
            out.extend_from_slice(b"\nendobj\n");
        }
        let last = stored.iter().map(|o| o.0).chain(packed.iter().map(|o| o.0)).max().unwrap_or(0);
        let (xref, size) = (last + 1, last + 2);
        let at = out.len();
        offsets.insert(xref, at);
        let packed: alloc::collections::BTreeMap<u32, (u32, u16)> = packed.iter().map(|o| (o.0, (o.1, o.2))).collect();
        let mut rows = Vec::new();
        for num in 0..size {
            if let Some(offset) = offsets.get(&num) {
                rows.push(1);
                rows.extend_from_slice(&(*offset as u32).to_be_bytes());
                rows.extend_from_slice(&[0, 0]);
            } else if let Some((stream, index)) = packed.get(&num) {
                rows.push(2);
                rows.extend_from_slice(&stream.to_be_bytes());
                rows.extend_from_slice(&index.to_be_bytes());
            } else {
                rows.extend_from_slice(&[0; 7]);
            }
        }
        out.extend_from_slice(
            alloc::format!(
                "{xref} 0 obj << /Type /XRef /Size {size} /W [1 4 2] /Root 1 0 R /Length {} >> stream\n",
                rows.len()
            )
            .as_bytes(),
        );
        out.extend_from_slice(&rows);
        out.extend_from_slice(alloc::format!("\nendstream endobj\nstartxref\n{at}\n%%EOF\n").as_bytes());
        out
    }

    /// `/Type /ObjStm` around `members`, each stored at its offset, and `entries` of its own.
    fn object_stream_of(members: &[(u32, &[u8])], entries: &str) -> Vec<u8> {
        let (mut head, mut body) = (alloc::string::String::new(), Vec::new());
        for (num, member) in members {
            head.push_str(&alloc::format!("{num} {} ", body.len()));
            body.extend_from_slice(member);
            body.push(b'\n');
        }
        head.push('\n');
        let dict = alloc::format!(
            "<< /Type /ObjStm /N {} /First {} /Length {} {entries} >> stream\n",
            members.len(),
            head.len(),
            head.len() + body.len()
        );
        let mut out = dict.into_bytes();
        out.extend_from_slice(head.as_bytes());
        out.extend_from_slice(&body);
        out.extend_from_slice(b"\nendstream");
        out
    }

    fn page_objects() -> Vec<(u32, Vec<u8>)> {
        vec![
            (1, b"<< /Type /Catalog /Pages 2 0 R >>".to_vec()),
            (2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec()),
            (3, b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] >>".to_vec()),
        ]
    }

    /// An object stream is a stream stored as that object, however many references lead to one.
    /// Object 10 holds a reference to 11, a stream stored inside object stream 12, and 31 claims to
    /// be in 10, with 11's `/N`, `/First` or `/Length` being 31: reading it recursed until the
    /// stack ran out. It reads as missing, and so does a member of such a stream.
    #[test]
    fn an_object_stream_isnt_reached_through_a_reference() {
        for key in ["N", "First", "Length"] {
            let inner = alloc::format!("<< /Type /ObjStm /{key} 31 0 R >> stream\n31 0 1\n\nendstream");
            let mut stored = page_objects();
            stored.push((10, b"11 0 R".to_vec()));
            stored.push((12, object_stream_of(&[(11, inner.as_bytes())], "")));
            let pdf = Pdf::new(with_xref_stream(&stored, &[(11, 12, 0), (31, 10, 0)])).unwrap();
            assert_eq!(pdf.xref().get::<i32>(ObjectIdentifier::new(31, 0)), None, "{key}");
        }
        let inner = b"<< /Type /ObjStm /N 1 /First 5 /Length 8 >> stream\n20 0 7\n\nendstream";
        let mut stored = page_objects();
        stored.push((10, b"11 0 R".to_vec()));
        stored.push((12, object_stream_of(&[(11, inner)], "")));
        let pdf = Pdf::new(with_xref_stream(&stored, &[(11, 12, 0), (20, 10, 0)])).unwrap();
        assert_eq!(pdf.xref().get::<i32>(ObjectIdentifier::new(20, 0)), None);
    }

    /// `count` object streams stored as themselves, the `keys` of each a member of the next, or
    /// with `ring` of the first after the last. Reading the first member reads them all, and it is
    /// `count + 100` to tell.
    #[cfg(feature = "std")]
    fn chain_of_object_streams(count: u32, keys: &[&str], ring: bool) -> Vec<u8> {
        let mut stored = page_objects();
        let mut packed = Vec::new();
        for i in 0..count {
            let (stream, member) = (100 + 2 * i, 101 + 2 * i);
            let value = if i == 0 { count + 100 } else { 1 };
            let data = alloc::format!("{member} 0 \n{value}\n");
            let (first, length) = (alloc::format!("{}", alloc::format!("{member} 0 \n").len()), alloc::format!("{}", data.len()));
            let next = match (i + 1 == count, ring) {
                (true, true) => "101 0 R".into(),
                (true, false) => "1".into(),
                _ => alloc::format!("{} 0 R", member + 2),
            };
            let entry = |key, own: &'_ str| if keys.contains(&key) { next.clone() } else { own.into() };
            let (n, first, length): (alloc::string::String, alloc::string::String, alloc::string::String) =
                (entry("N", "1"), entry("First", &first), entry("Length", &length));
            let body = alloc::format!("<< /Type /ObjStm /N {n} /First {first} /Length {length} >> stream\n{data}\nendstream");
            stored.push((stream, body.into_bytes()));
            packed.push((member, stream, 0));
        }
        with_xref_stream(&stored, &packed)
    }

    /// A chain of object streams, each with a member of the next for its `/N`, `/First` or
    /// `/Length`, had no limit: one read recursed through all of them, holding a copy of the chain
    /// at each, and 50,000 ran out of an 8 MiB stack. It stops at the gate's limit, on half the stack
    /// of a render worker. Past that `/N` and `/First` can't be read, so the member reads as missing,
    /// and a `/Length` that can't be read falls back on `endstream`, so it reads. A short chain reads.
    #[cfg(feature = "std")]
    #[test]
    fn a_chain_of_object_streams_ends_at_the_limit_of_nested_loads() {
        for key in ["N", "First", "Length"] {
            let pdf = chain_of_object_streams(50_000, &[key], false);
            std::thread::Builder::new()
                .stack_size(1 << 20)
                .spawn(move || {
                    let pdf = Pdf::new(pdf).unwrap();
                    let read = pdf.xref().get::<i32>(ObjectIdentifier::new(101, 0));
                    assert_eq!(read, (key == "Length").then_some(50_100), "{key}");
                })
                .unwrap()
                .join()
                .unwrap();
        }
        let pdf = Pdf::new(chain_of_object_streams(10, &["N"], false)).unwrap();
        assert_eq!(pdf.xref().get::<i32>(ObjectIdentifier::new(101, 0)), Some(110));
    }

    /// `/Length` and `/N` or `/First` both by reference were two ways to the next stream's index,
    /// and an index that fails isn't remembered, so the work doubled at each stream: 65 streams, one
    /// past the limit, in 8 KB, didn't finish, and other threads waited at the gate. The index reads
    /// the dictionary alone now. Without that this doesn't end, so the time allowed can't fail it.
    #[cfg(feature = "std")]
    #[test]
    fn a_chain_of_object_streams_takes_no_more_work_for_two_ways_on() {
        for (count, ring) in [(65, false), (20, true), (5_000, false)] {
            for keys in [&["Length", "N"][..], &["Length", "First"]] {
                let pdf = chain_of_object_streams(count, keys, ring);
                let (done, finished) = std::sync::mpsc::channel();
                std::thread::Builder::new()
                    .stack_size(1 << 20)
                    .spawn(move || {
                        let pdf = Pdf::new(pdf).unwrap();
                        let _ = pdf.xref().get::<i32>(ObjectIdentifier::new(101, 0));
                        let _ = done.send(());
                    })
                    .unwrap();
                assert!(finished.recv_timeout(std::time::Duration::from_secs(120)).is_ok(), "{keys:?} of {count}");
            }
        }
    }

    /// Object 5, a member of a real object stream, holds a reference to object 6 stored on its own,
    /// and 7, stored on its own, to 8, a member: each reads as what it names.
    #[test]
    fn a_reference_held_in_either_storage_is_followed() {
        let mut stored = page_objects();
        stored.push((6, b"42".to_vec()));
        stored.push((7, b"8 0 R".to_vec()));
        stored.push((9, object_stream_of(&[(5, b"6 0 R"), (8, b"43")], "")));
        let pdf = Pdf::new(with_xref_stream(&stored, &[(5, 9, 0), (8, 9, 1)])).unwrap();
        assert_eq!(pdf.xref().get::<i32>(ObjectIdentifier::new(5, 0)), Some(42));
        assert_eq!(pdf.xref().get::<i32>(ObjectIdentifier::new(7, 0)), Some(43));
    }

    /// With no page tree, every page object is a page, and one that another object only refers to
    /// is one page, not two.
    #[test]
    fn a_page_a_reference_is_held_to_is_found_once() {
        let mut stored = page_objects();
        stored[0].1 = b"<< /Type /Catalog /Pages 99 0 R >>".to_vec();
        stored[2].1 = b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] /Contents 4 0 R >>".to_vec();
        stored.push((4, b"<< /Length 0 >> stream\n\nendstream".to_vec()));
        stored.push((12, b"3 0 R".to_vec()));
        let pdf = Pdf::new(with_xref_stream(&stored, &[])).unwrap();
        assert_eq!(pdf.pages().len(), 1);
    }

    // PdfCraft patch tests (ported from craft-libs): xref stream entries are bounded by the
    // decoded data, and the type field is read at its full width.
    #[test]
    fn xref_stream_entries_need_backing_bytes() {
        let mut map = XrefMap::default();
        // All widths zero: an entry reads nothing, so none may be declared.
        let mut empty = Reader::new(&[]);
        assert!(xref_stream_subsection(&mut empty, 0, u32::MAX, 0, 0, 0, &mut map).is_none());
        assert!(map.is_empty());
        // Three entries of width 3 need nine bytes; eight are refused before any insert.
        let data = [1, 0, 9, 1, 0, 9, 1, 0];
        let mut short = Reader::new(&data);
        assert!(xref_stream_subsection(&mut short, 0, 3, 1, 1, 1, &mut map).is_none());
        assert!(map.is_empty());
        let mut whole = Reader::new(&data[..6]);
        assert!(xref_stream_subsection(&mut whole, 0, 2, 1, 1, 1, &mut map).is_some());
        assert_eq!(map.len(), 2);
        // A subsection running past object number i32::MAX is refused.
        for start in [i32::MAX as u32, u32::MAX] {
            let mut past_end = XrefMap::default();
            let mut wrap = Reader::new(&data[..6]);
            assert!(xref_stream_subsection(&mut wrap, start, 2, 1, 1, 1, &mut past_end).is_none());
            assert!(past_end.keys().all(|id| id.obj_number >= 0));
        }
    }

    #[test]
    fn xref_stream_type_field_uses_the_full_w0_width() {
        // /W [2 1 1]: two-byte type 0x0001 (normal), offset 7, gen 0.
        let mut map = XrefMap::default();
        let mut reader = Reader::new(&[0u8, 1, 7, 0]);
        assert!(xref_stream_subsection(&mut reader, 5, 1, 2, 1, 1, &mut map).is_some());
        assert!(matches!(
            map.get(&ObjectIdentifier::new(5, 0)),
            Some(EntryType::Normal(7))
        ));
    }

    #[test]
    fn xref_stream_loader_rejects_oversized_w0() {
        // One normal entry (offset 7, gen 0) whose type field is `type_len` bytes wide, so only
        // the width of /W[0] differs between the two streams.
        let load = |type_len: usize| {
            let mut entry = vec![0u8; type_len];
            entry[type_len - 1] = 1;
            entry.extend_from_slice(&[7, 0]);
            let mut data = format!(
                "1 0 obj\n<< /Type /XRef /Size 6 /Index [5 1] /W [{type_len} 1 1] /Length {} >>\nstream\n",
                entry.len()
            )
            .into_bytes();
            data.extend_from_slice(&entry);
            data.extend_from_slice(b"\nendstream\nendobj\n");
            let mut map = XrefMap::default();
            let got = populate_from_xref_stream(
                &data,
                &mut Reader::new(&data),
                &mut map,
                &mut BTreeSet::new(),
            );
            got.is_some()
        };
        assert!(load(8));
        assert!(!load(9));
    }

    // Contributor-original, MIT OR Apache-2.0. Matches the bounded, uncompressed
    // synthetic regression used to reproduce the cold-publication race. A second
    // stream optionally checks that racing registration never aliases two ids.
    fn object_stream_fixture(second: bool) -> Vec<u8> {
        let mut out = b"%PDF-1.5\n".to_vec();
        let mut offsets = alloc::collections::BTreeMap::new();
        fn object(
            out: &mut Vec<u8>,
            offsets: &mut alloc::collections::BTreeMap<u32, usize>,
            n: u32,
            body: &[u8],
        ) {
            offsets.insert(n, out.len());
            out.extend_from_slice(format!("{n} 0 obj\n").as_bytes());
            out.extend_from_slice(body);
            out.extend_from_slice(b"\nendobj\n");
        }
        object(
            &mut out,
            &mut offsets,
            1,
            b"<< /Type /Catalog /Pages 2 0 R >>",
        );
        object(
            &mut out,
            &mut offsets,
            2,
            b"<< /Type /Pages /Count 1 /Kids [3 0 R] >>",
        );
        object(
            &mut out,
            &mut offsets,
            3,
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] >>",
        );
        let mut payload = b"20 0 << /Marker 123 >>".to_vec();
        payload.resize(payload.len() + 4 * 1024 * 1024, b' ');
        let mut stream = format!(
            "<< /Type /ObjStm /N 1 /First 5 /Length {} >>\nstream\n",
            payload.len()
        )
        .into_bytes();
        stream.extend(payload);
        stream.extend_from_slice(b"\nendstream");
        object(&mut out, &mut offsets, 10, &stream);
        if second {
            let payload = b"21 0 << /Marker 456 >>";
            let mut stream = format!(
                "<< /Type /ObjStm /N 1 /First 5 /Length {} >>\nstream\n",
                payload.len()
            )
            .into_bytes();
            stream.extend_from_slice(payload);
            stream.extend_from_slice(b"\nendstream");
            object(&mut out, &mut offsets, 12, &stream);
        }
        let xref = out.len();
        offsets.insert(11, xref);
        let size = if second { 22 } else { 21 };
        let mut rows = Vec::new();
        for number in 0..size {
            let (kind, a, b): (u8, u32, u16) = match number {
                20 => (2, 10, 0),
                21 => (2, 12, 0),
                n => offsets
                    .get(&n)
                    .map_or((0, 0, 65535), |offset| (1, *offset as u32, 0)),
            };
            rows.push(kind);
            rows.extend_from_slice(&a.to_be_bytes());
            rows.extend_from_slice(&b.to_be_bytes());
        }
        let mut stream = format!(
            "<< /Type /XRef /Size {size} /Root 1 0 R /W [1 4 2] /Length {} >>\nstream\n",
            rows.len()
        )
        .into_bytes();
        stream.extend(rows);
        stream.extend_from_slice(b"\nendstream");
        object(&mut out, &mut offsets, 11, &stream);
        out.extend_from_slice(format!("startxref\n{xref}\n%%EOF\n").as_bytes());
        out
    }

    #[cfg(feature = "std")]
    #[test]
    fn concurrent_cold_object_stream_reads_all_succeed() {
        use std::sync::Barrier;
        let bytes = Arc::new(object_stream_fixture(true));
        for _ in 0..10 {
            let pdf = Arc::new(crate::Pdf::new(bytes.clone()).unwrap());
            let barrier = Arc::new(Barrier::new(16));
            let jobs: Vec<_> = (0..16)
                .map(|_| {
                    let (pdf, barrier) = (pdf.clone(), barrier.clone());
                    std::thread::spawn(move || {
                        barrier.wait();
                        for (id, marker) in [(20, 123), (21, 456), (20, 123)] {
                            let dict = pdf
                                .xref()
                                .get::<Dict<'_>>(ObjectIdentifier::new(id, 0))
                                .unwrap();
                            assert_eq!(dict.get::<i32>(b"Marker"), Some(marker));
                        }
                    })
                })
                .collect();
            for job in jobs {
                job.join().unwrap();
            }
        }
    }

    #[test]
    fn an_object_stream_and_its_first_member_are_both_enumerated() {
        // Both sit at (stream offset, 0) in `objects`' ordering; one object per location must
        // not cost either of them.
        let pdf = crate::Pdf::new(object_stream_fixture(false)).unwrap();
        let objects: Vec<_> = pdf.xref().objects().into_iter().collect();
        assert!(objects.iter().any(|o| matches!(o, Object::Stream(_))));
        assert!(objects.iter().any(
            |o| matches!(o, Object::Dict(d) if d.get::<i32>(b"Marker") == Some(123))
        ));
    }

    #[test]
    fn recursive_object_stream_locations_fail_without_recursing() {
        let pdf = crate::Pdf::new(object_stream_fixture(true)).unwrap();
        let Inner::Some(repr) = &pdf.xref().0 else {
            panic!("real xref")
        };
        {
            let mut map = repr.map.put();
            map.xref_map
                .insert(ObjectIdentifier::new(10, 0), EntryType::ObjStream(12, 0));
            map.xref_map
                .insert(ObjectIdentifier::new(12, 0), EntryType::ObjStream(10, 0));
        }
        for id in [10, 12, 20, 21] {
            assert!(
                pdf.xref()
                    .get::<Dict<'_>>(ObjectIdentifier::new(id, 0))
                    .is_none()
            );
        }
    }

    #[cfg(feature = "std")]
    #[test]
    fn concurrent_repair_and_read_retries_the_repaired_map() {
        let pdf = Arc::new(crate::Pdf::new(object_stream_fixture(false)).unwrap());
        let Inner::Some(repr) = &pdf.xref().0 else {
            panic!("real xref")
        };
        repr.map
            .put()
            .xref_map
            .insert(ObjectIdentifier::new(1, 0), EntryType::Normal(0));
        let barrier = Arc::new(std::sync::Barrier::new(16));
        let jobs: Vec<_> = (0..16)
            .map(|_| {
                let (pdf, barrier) = (pdf.clone(), barrier.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    for _ in 0..8 {
                        let root = pdf
                            .xref()
                            .get::<Dict<'_>>(ObjectIdentifier::new(1, 0))
                            .unwrap();
                        assert!(root.get_ref(PAGES).is_some());
                        pdf.xref().repair(); // already repaired is an idempotent no-op
                    }
                })
            })
            .collect();
        for job in jobs {
            job.join().unwrap();
        }
        assert!(repr.map.get().repaired);
    }

    #[cfg(feature = "std")]
    #[test]
    fn poisoned_repair_lock_does_not_cascade_or_retry_repair() {
        use std::panic::{AssertUnwindSafe, catch_unwind};
        let pdf = crate::Pdf::new(object_stream_fixture(false)).unwrap();
        let Inner::Some(repr) = &pdf.xref().0 else {
            panic!("real xref")
        };
        assert!(
            catch_unwind(AssertUnwindSafe(|| {
                let mut map = repr.map.put();
                map.repaired = true; // same state as an unwinding fallback before publication
                panic!("injected repair failure");
            }))
            .is_err()
        );
        pdf.xref().repair();
        assert!(
            pdf.xref()
                .get::<Dict<'_>>(ObjectIdentifier::new(1, 0))
                .is_some()
        );
        assert!(repr.map.get().repaired);
    }

    #[test]
    fn circular_prev_chain() {
        let mut pdf = b"%PDF-1.0\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n".to_vec();
        let expected_xref_pos = pdf.len();
        pdf.extend_from_slice(
            format!(
                "xref\n\
                 0 1\n\
                 0000000000 65535 f \r\n\
                 trailer\n<< /Size 1 /Root 1 0 R /Prev {expected_xref_pos} >>\n\
                 startxref\n{expected_xref_pos}\n%%EOF"
            )
            .as_bytes(),
        );

        let mut xref_map = FxHashMap::default();
        let xref_pos = find_last_xref_pos(pdf.as_ref()).unwrap();
        let _result = populate_xref_impl(pdf.as_ref(), xref_pos, &mut xref_map);
    }

    #[test]
    fn prev_as_reference_reaches_the_earlier_section() {
        let mut pdf = b"%PDF-1.0\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n".to_vec();
        let first_xref_pos = pdf.len();
        pdf.extend_from_slice(
            format!(
                "xref\n\
                 0 2\n\
                 0000000000 65535 f \n\
                 0000000009 00000 n \n\
                 trailer\n<< /Size 2 /Root 1 0 R >>\n\
                 startxref\n{first_xref_pos}\n%%EOF\n"
            )
            .as_bytes(),
        );
        let second_xref_pos = pdf.len();
        pdf.extend_from_slice(
            format!(
                "xref\n\
                 0 1\n\
                 0000000000 65535 f \n\
                 trailer\n<< /Size 2 /Root 1 0 R /Prev {first_xref_pos} 0 R >>\n\
                 startxref\n{second_xref_pos}\n%%EOF"
            )
            .as_bytes(),
        );

        let mut xref_map = FxHashMap::default();
        let xref_pos = find_last_xref_pos(pdf.as_ref()).unwrap();
        assert_eq!(xref_pos, second_xref_pos);
        populate_xref_impl(pdf.as_ref(), xref_pos, &mut xref_map).unwrap();
        assert!(xref_map.contains_key(&ObjectIdentifier::new(1, 0)));
    }

    #[test]
    fn prev_as_reference_to_an_object_holding_the_offset_reads_that_offset() {
        let mut pdf = b"%PDF-1.0\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n".to_vec();
        let first_xref_pos = pdf.len();
        pdf.extend_from_slice(
            format!(
                "xref\n\
                 0 2\n\
                 0000000000 65535 f \n\
                 0000000009 00000 n \n\
                 trailer\n<< /Size 2 /Root 1 0 R >>\n\
                 startxref\n{first_xref_pos}\n%%EOF\n"
            )
            .as_bytes(),
        );
        // Object 3 holds the offset; read as offset 3, `Prev` would point inside the header.
        let held_pos = pdf.len();
        pdf.extend_from_slice(format!("3 0 obj\n{first_xref_pos}\nendobj\n").as_bytes());
        let second_xref_pos = pdf.len();
        pdf.extend_from_slice(
            format!(
                "xref\n\
                 3 1\n\
                 {held_pos:010} 00000 n \n\
                 trailer\n<< /Size 4 /Root 1 0 R /Prev 3 0 R >>\n\
                 startxref\n{second_xref_pos}\n%%EOF"
            )
            .as_bytes(),
        );

        let mut xref_map = FxHashMap::default();
        let xref_pos = find_last_xref_pos(pdf.as_ref()).unwrap();
        assert_eq!(xref_pos, second_xref_pos);
        populate_xref_impl(pdf.as_ref(), xref_pos, &mut xref_map).unwrap();
        assert!(xref_map.contains_key(&ObjectIdentifier::new(1, 0)));
        assert!(xref_map.contains_key(&ObjectIdentifier::new(3, 0)));
    }

    #[test]
    fn find_last_xref_uses_last_startxref() {
        let pdf = b"%PDF-1.0\nstartxref\n5\n%%EOF\nstartxref\n42\n%%EOF";
        assert_eq!(find_last_xref_pos(pdf), Some(42));
    }

    #[test]
    fn xref_table_trailer_rejects_overflowing_entry_skip() {
        let data = b"xref\n0 999999999999999999999\ntrailer\n<<>>";
        let mut reader = Reader::new(data);
        assert!(read_xref_table_trailer(&mut reader, &ReaderContext::dummy()).is_none());
    }

    /// PdfCraft patch: (hayro-syntax (22)) an object's offset past `usize` is malformed: a plain
    /// `+` there panics in a debug build and wraps in a release one.
    #[test]
    fn an_object_stream_offset_past_usize_is_malformed_not_a_panic() {
        assert_eq!(read_object_stream_index(1, usize::MAX, b"5 1"), None);
        assert_eq!(read_object_stream_index(1, 10, b"5 1"), Some(vec![(5, 11)]));
    }
}
