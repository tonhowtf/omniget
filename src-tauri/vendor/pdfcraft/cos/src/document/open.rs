use super::reader::find;
use super::state::{Credentials, Slot};
use super::xref::XrefIndex;
use super::{Arc, BTreeMap, CosError, Dict, Document, HashSet, ObjRef, Object};

/// More objects than `/Encrypt` and `/ID` refer to in any file, so that one which refers to the page
/// tree, say, doesn't have a whole document read to open it.
const MAX_ENCRYPT_MEMBERS: usize = 64;

impl Document {
    /// A new document with an empty page tree (for split / extract / combine). It has no
    /// original bytes, so saving always writes a full file.
    pub fn new_empty() -> Self {
        let mut doc = Document {
            data: Arc::new(Vec::new()),
            endstreams: Arc::default(),
            source_identity: Arc::new(()),
            entries: Arc::new(XrefIndex::empty()),
            trailer: Dict::new(),
            revisions: Arc::new(Vec::new()),
            xref_oddities: Arc::default(),
            located: Arc::default(),
            repair_log: Arc::new(Vec::new()),
            cache: Arc::default(),
            objstms: Arc::default(),
            jbig2_globals: Arc::default(),
            stream_limit: crate::object::MAX_DECODED,
            overlay: BTreeMap::new(),
            next_num: 1,
            header_offset: 0,
            version: "1.7".into(),
            security: None,
            encrypt_num: None,
            encrypt_members: Arc::default(),
            unfound: Arc::default(),
            encryption_changed: false,
            full_save: false,
            new_id: false,
            out_security: None,
            out_encrypt_num: None,
        };
        let mut pages = Dict::new();
        pages.set(b"Type".to_vec(), Object::name("Pages"));
        pages.set(b"Kids".to_vec(), Object::Array(Vec::new()));
        pages.set(b"Count".to_vec(), Object::Int(0));
        let pages = doc.add(pages);
        let mut catalog = Dict::new();
        catalog.set(b"Type".to_vec(), Object::name("Catalog"));
        catalog.set(b"Pages".to_vec(), Object::Ref(pages));
        let root = doc.add(catalog);
        doc.trailer.set(b"Root".to_vec(), Object::Ref(root));
        doc
    }

    /// Parse a document. Damaged cross-reference data is reconstructed; see `repair_log`.
    /// Encrypted documents open only if their user password is empty; see `open_with_password`.
    pub fn open(data: Arc<Vec<u8>>) -> Result<Self, CosError> {
        Self::open_with_password(data, None)
    }

    /// Parse a document, authenticating encrypted ones with `password` (owner or user; `None`
    /// tries the empty password). Fails with `NeedsPassword` / `WrongPassword` as appropriate.
    pub fn open_with_password(data: Arc<Vec<u8>>, password: Option<&str>) -> Result<Self, CosError> {
        Self::open_with_stream_limit(data, password, crate::object::MAX_DECODED)
    }

    /// Open with a tighter decompression bound for object and cross-reference streams.
    /// Inspection can use this without changing the editing reader's default limit. Such a
    /// document is read-only: objects in a larger object stream read as missing, so the
    /// writers refuse to save it ([`CosError::ReadOnlyLimit`]).
    pub fn open_with_stream_limit(data: Arc<Vec<u8>>, password: Option<&str>, stream_limit: usize) -> Result<Self, CosError> {
        Self::open_as(data, Credentials::Password(password), stream_limit)
    }

    /// Open `data`, an earlier revision of this file (the bytes a signature covers) or another
    /// save of it, with this document's credentials: an encrypted one must have the same
    /// `/Encrypt` dictionary and `/ID` as this document, whose file key is then reused, so no
    /// password is needed or kept; unencrypted data opens as [`open`](Self::open) does. Data
    /// encrypted differently fails with `NeedsPassword`.
    pub fn open_with_same_credentials(&self, data: Arc<Vec<u8>>) -> Result<Self, CosError> {
        let credentials = match &self.security {
            Some(handler) => Credentials::Handler { handler, id0: self.first_id() },
            None => Credentials::Password(None),
        };
        Self::open_as(data, credentials, self.stream_limit)
    }

    fn open_as(data: Arc<Vec<u8>>, credentials: Credentials<'_>, stream_limit: usize) -> Result<Self, CosError> {
        // Viewers accept files whose header is missing or damaged as long as the body looks
        // like PDF; so do we (a note goes to the repair log).
        let header = find(&data, b"%PDF-", 0, 1024);
        let headerless = header.is_none();
        if headerless && find(&data, b" obj", 0, 4096).is_none() {
            return Err(CosError::NotPdf);
        }
        let header_offset = header.unwrap_or(0);
        let version = match header {
            Some(h) => String::from_utf8_lossy(&data[(h + 5).min(data.len())..(h + 8).min(data.len())]).trim().to_string(),
            None => "1.4".to_string(),
        };
        let mut doc = Document {
            data,
            endstreams: Arc::default(),
            source_identity: Arc::new(()),
            entries: Arc::new(XrefIndex::empty()),
            trailer: Dict::new(),
            revisions: Arc::new(Vec::new()),
            xref_oddities: Arc::default(),
            located: Arc::default(),
            repair_log: Arc::new(Vec::new()),
            cache: Arc::default(),
            objstms: Arc::default(),
            jbig2_globals: Arc::default(),
            stream_limit: stream_limit.min(crate::object::MAX_DECODED),
            overlay: BTreeMap::new(),
            next_num: 1,
            header_offset,
            version,
            security: None,
            encrypt_num: None,
            encrypt_members: Arc::default(),
            unfound: Arc::default(),
            encryption_changed: false,
            full_save: false,
            new_id: false,
            out_security: None,
            out_encrypt_num: None,
        };
        let mut log = Vec::new();
        if headerless {
            log.push("the %PDF- header is missing; reading the file as PDF 1.4".into());
        }
        let parsed = doc.read_xref_chain(&mut log);
        let mut authenticated = false;
        let ok = match parsed {
            Ok((entries, trailer, revisions, oddities)) => {
                doc.entries = Arc::new(entries);
                doc.trailer = trailer;
                doc.revisions = Arc::new(revisions);
                doc.xref_oddities = Arc::new(oddities);
                doc.fill_xref_gaps(&mut log);
                // Authenticate before checking the catalog: it may sit in an encrypted object
                // stream, which reads as garbage until the key is known.
                if let Some(enc) = doc.trailer.get(b"Encrypt").cloned() {
                    doc.authenticate(&enc, &credentials)?;
                    authenticated = true;
                }
                doc.root_is_catalog()
            }
            Err(e) => {
                log.push(format!("cross-reference data unreadable ({e}); rebuilding from object headers"));
                false
            }
        };
        if !ok {
            if !doc.entries.is_empty() {
                log.push("cross-reference data does not lead to a valid catalog; rebuilding from object headers".into());
            }
            doc.reconstruct(&mut log)?;
        }
        // Header generations are plaintext, including in encrypted files. Inspect only the
        // bounded headers, not object bodies, and retain the compact xref representation.
        doc.note_xref_generation_repairs(&mut log);
        doc.repair_log = Arc::new(log);
        // Reconstruction may have found a different trailer: (re)authenticate against it.
        if (!ok || !authenticated)
            && let Some(enc) = doc.trailer.get(b"Encrypt").cloned()
        {
            doc.authenticate(&enc, &credentials)?;
        }
        if Arc::make_mut(&mut doc.entries).remove_exhausted() {
            Arc::make_mut(&mut doc.repair_log).push("skipped exhausted xref object number 4294967295".into());
        }
        let max = doc.entries.max_num();
        let size = u32::try_from(doc.trailer.int(b"Size").unwrap_or(0).max(0)).unwrap_or(u32::MAX);
        if size == u32::MAX {
            Arc::make_mut(&mut doc.repair_log).push("exhausted trailer /Size; new objects use unused object numbers".into());
        }
        doc.next_num = max.saturating_add(1).max(size).max(1);
        Ok(doc)
    }

    fn authenticate(&mut self, enc: &Object, credentials: &Credentials<'_>) -> Result<(), CosError> {
        // Authentication can change how the same input is decoded.
        self.source_identity = Arc::new(());
        let (num, dict) = match enc {
            Object::Ref(r) => (Some(r.num), self.get(*r).as_dict().cloned()),
            Object::Dict(d) => (None, Some(d.clone())),
            _ => (None, None),
        };
        let Some(dict) = dict else {
            // A dangling /Encrypt: viewers treat the file as unencrypted, and so do we.
            let mut log = self.repair_log.as_ref().clone();
            log.push("the trailer names an /Encrypt dictionary that does not exist; reading the file as unencrypted".into());
            self.repair_log = Arc::new(log);
            self.trailer.remove(b"Encrypt");
            return Ok(());
        };
        let params = crate::security::encrypt_dict(self, &dict);
        let id0 = self.first_id();
        // Read here, while nothing is decrypted yet.
        let members = self.encryption_members(enc);
        let handler = match credentials {
            Credentials::Password(password) => Arc::new(pdfcraft_crypt::SecurityHandler::open(params, &id0, *password).map_err(|e| match e {
                pdfcraft_crypt::CryptError::WrongPassword if password.is_none() => CosError::NeedsPassword,
                pdfcraft_crypt::CryptError::WrongPassword => CosError::WrongPassword,
                other => CosError::Security(other.to_string()),
            })?),
            // The same encryption as the document whose key this is: that key opens it.
            Credentials::Handler { handler, id0: known } if *handler.dict() == params && *known == id0 => Arc::clone(handler),
            Credentials::Handler { .. } => return Err(CosError::NeedsPassword),
        };
        self.security = Some(handler);
        self.encrypt_num = num;
        self.encrypt_members = Arc::new(members);
        // Objects read while locating the catalog were read before decryption was possible.
        self.cache = Arc::default();
        self.objstms = Arc::default();
        Ok(())
    }

    /// The first file identifier, part of the encryption key (empty when there is none).
    fn first_id(&self) -> Vec<u8> {
        let id = self.trailer.get(b"ID").map(|id| self.resolve(id));
        let first = id.as_deref().and_then(Object::as_array).and_then(|a| a.first()).map(|s| self.resolve(s));
        first.as_deref().and_then(Object::as_string).map(|s| s.bytes.clone()).unwrap_or_default()
    }

    /// The objects `/Encrypt`, given as `enc`, and `/ID` refer to, the ones they refer to, and so
    /// on, up to [`MAX_ENCRYPT_MEMBERS`]. Their strings aren't encrypted (ISO 32000-2 §7.6.2): the
    /// key is made from them, so they have to read as stored, and stay so. A number with no object
    /// behind it, and a stream, which those dictionaries never hold, isn't one: an object added
    /// under that number later, or a page's content a hostile `/Encrypt` names, is encrypted.
    fn encryption_members(&self, enc: &Object) -> HashSet<u32> {
        let mut pending = Vec::new();
        crate::writer::collect_refs(enc, &mut |r| pending.push(r));
        if let Some(id) = self.trailer.get(b"ID") {
            crate::writer::collect_refs(id, &mut |r| pending.push(r));
        }
        let mut members = HashSet::new();
        while let Some(r) = pending.pop() {
            if members.len() >= MAX_ENCRYPT_MEMBERS || members.contains(&r.num) {
                continue;
            }
            let held = self.get(r);
            if matches!(&*held, Object::Null | Object::Stream(_)) {
                continue;
            }
            members.insert(r.num);
            crate::writer::collect_refs(&held, &mut |x| pending.push(x));
        }
        members
    }

    /// Whether object `num` is written as it is held, being one of [`Document::encryption_members`],
    /// unedited, and the save writing the same `/Encrypt`. What was edited or added is encrypted
    /// whatever its number.
    pub(crate) fn stored_as_is(&self, num: u32) -> bool {
        !self.encryption_changed && !self.is_edited(num) && self.encrypt_members.contains(&num)
    }

    /// Lift the restrictions of a document opened with its user password by entering the
    /// owner (permissions) password (§7.6.4). Fails with `WrongPassword` for any other
    /// password. Does nothing when the document is unencrypted or already opened as owner.
    /// Both passwords unlock the same file key, so nothing read so far is decoded again.
    pub fn unlock(&mut self, password: &str) -> Result<(), CosError> {
        let Some(current) = self.security.as_deref() else { return Ok(()) };
        if current.auth() == pdfcraft_crypt::Auth::Owner {
            return Ok(());
        }
        let owner = current.authenticate_owner(&self.first_id(), password).map_err(|e| match e {
            pdfcraft_crypt::CryptError::WrongPassword => CosError::WrongPassword,
            other => CosError::Security(other.to_string()),
        })?;
        self.security = Some(Arc::new(owner));
        Ok(())
    }

    /// The security handler, when the document is encrypted.
    pub fn security(&self) -> Option<&pdfcraft_crypt::SecurityHandler> {
        self.security.as_deref()
    }

    /// What the opening password allows (`None` for unencrypted documents: everything).
    pub fn permissions(&self) -> Option<pdfcraft_crypt::Permissions> {
        self.security().map(|s| s.permissions())
    }

    /// Make the next save a full rewrite: earlier revisions (which still hold what an edit
    /// removed) must not stay in the file. Redaction requires this.
    pub fn require_full_save(&mut self) {
        self.full_save = true;
    }

    /// Like [`Document::require_full_save`], and the rewritten file also gets a new file identifier
    /// (`/ID`), so that a redacted or sanitized copy never claims to be its source (ISO 32000-2
    /// §14.4). An encrypted file keeps the first string of its identifier, which its key depends on.
    pub fn require_full_save_with_new_id(&mut self) {
        self.full_save = true;
        self.new_id = true;
    }

    /// The next save must rewrite the whole file (see [`Document::require_full_save`]).
    /// Repair-aware edits retaining two accepted generations cannot use one incremental slot.
    pub fn full_save_required(&self) -> bool {
        self.full_save || self.overlay.values().any(|slot| matches!(slot, Slot::RepairedSet(..)))
    }

    /// The next full save gives the file a new identifier (see [`Document::require_full_save_with_new_id`]).
    pub fn new_id_required(&self) -> bool {
        self.new_id
    }

    /// Security was set or removed since the document was opened (the next save applies it).
    pub fn encryption_changed(&self) -> bool {
        self.encryption_changed
    }

    /// The handler used to write: the new one after `set_encryption` / `remove_encryption`,
    /// otherwise the one the document was opened with.
    pub(crate) fn output_security(&self) -> (Option<&pdfcraft_crypt::SecurityHandler>, Option<u32>) {
        if self.encryption_changed { (self.out_security.as_deref(), self.out_encrypt_num) } else { (self.security.as_deref(), self.encrypt_num) }
    }

    /// The security the next save writes: protection applied with `set_encryption`, none after
    /// `remove_encryption`, otherwise the security the document was opened with.
    pub fn output_handler(&self) -> Option<&pdfcraft_crypt::SecurityHandler> {
        self.output_security().0
    }

    /// Protect the document with a password (§7.6.4). Takes effect on the next save, which is
    /// always a full rewrite. Returns the handler (authenticated as owner).
    pub fn set_encryption(&mut self, params: &pdfcraft_crypt::NewEncryption) -> Result<(), CosError> {
        // The file identifier is part of the key; make sure it exists and keep it.
        let id = self.trailer.get(b"ID").map(|id| self.resolve(id));
        let id0 = match id.as_deref() {
            Some(Object::Array(a)) if a.len() == 2 => self.first_id(),
            _ => {
                let id = pdfcraft_crypt::file_id(&params.seed);
                let s = Object::String(crate::PdfString { bytes: id.clone(), hex: true });
                self.trailer.set(b"ID".to_vec(), Object::Array(vec![s.clone(), s]));
                id
            }
        };
        let h = pdfcraft_crypt::create(params, &id0).map_err(|e| CosError::Security(e.to_string()))?;
        let d = h.dict();
        let s = |b: &[u8]| Object::String(crate::PdfString { bytes: b.to_vec(), hex: true });
        let mut e = Dict::new();
        e.set(b"Filter".to_vec(), Object::name("Standard"));
        e.set(b"V".to_vec(), Object::Int(d.v));
        e.set(b"R".to_vec(), Object::Int(d.r));
        e.set(b"Length".to_vec(), Object::Int(d.length_bits));
        e.set(b"O".to_vec(), s(&d.o));
        e.set(b"U".to_vec(), s(&d.u));
        e.set(b"P".to_vec(), Object::Int(i64::from(d.p)));
        if d.v >= 4 {
            let mut cf = Dict::new();
            let mut std_cf = Dict::new();
            std_cf.set(b"Type".to_vec(), Object::name("CryptFilter"));
            let (cfm, len) = if d.v >= 5 { ("AESV3", 32) } else { ("AESV2", 16) };
            std_cf.set(b"CFM".to_vec(), Object::name(cfm));
            std_cf.set(b"Length".to_vec(), Object::Int(len));
            std_cf.set(b"AuthEvent".to_vec(), Object::name("DocOpen"));
            cf.set(b"StdCF".to_vec(), Object::Dict(std_cf));
            e.set(b"CF".to_vec(), Object::Dict(cf));
            e.set(b"StmF".to_vec(), Object::name("StdCF"));
            e.set(b"StrF".to_vec(), Object::name("StdCF"));
            if !d.encrypt_metadata {
                e.set(b"EncryptMetadata".to_vec(), Object::Bool(false));
            }
        }
        if d.v >= 5 {
            e.set(b"OE".to_vec(), s(&d.oe));
            e.set(b"UE".to_vec(), s(&d.ue));
            e.set(b"Perms".to_vec(), s(&d.perms));
        }
        if let Some(Object::Ref(old)) = self.trailer.get(b"Encrypt").cloned() {
            self.free(old);
        }
        let r = self.add(e);
        self.trailer.set(b"Encrypt".to_vec(), Object::Ref(r));
        // Objects are still read with the original handler; saves use the new one.
        self.out_security = Some(Arc::new(h));
        self.out_encrypt_num = Some(r.num);
        self.encryption_changed = true;
        Ok(())
    }

    /// Remove password protection (requires the document to be open — any authenticated
    /// password; callers should require the owner password, §7.6.4). Applied by a full save.
    pub fn remove_encryption(&mut self) {
        if let Some(Object::Ref(r)) = self.trailer.get(b"Encrypt").cloned() {
            self.free(r);
        }
        self.trailer.remove(b"Encrypt");
        self.out_security = None;
        self.out_encrypt_num = None;
        self.encryption_changed = true;
    }

    /// Decrypt a freshly parsed indirect object if the document is encrypted.
    pub(super) fn decrypted(&self, id: ObjRef, o: Object) -> Object {
        match &self.security {
            Some(h) if Some(id.num) != self.encrypt_num && !self.encrypt_members.contains(&id.num) => {
                crate::security::transform(self, h, &o, id.num, id.generation, crate::security::Pass::Decrypt)
            }
            _ => o,
        }
    }
}
