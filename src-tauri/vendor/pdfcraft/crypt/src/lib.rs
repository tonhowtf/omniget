//! pdfcraft-crypt — the PDF standard security handler (ISO 32000-2 §7.6), standalone (L0).
//!
//! Supports every revision of the standard handler:
//! - R2 (40-bit RC4), R3 (RC4 up to 128-bit), R4 (crypt filters: RC4 or AES-128);
//! - R5 (the deprecated AES-256 extension) and R6 (AES-256, PDF 2.0).
//!
//! It authenticates with the user or owner password (owner first, as viewers do) and derives
//! per-object keys. It decrypts and encrypts strings and streams, and honours crypt filters,
//! including `Identity` and embedded-file-only encryption. It can also *create* encryption
//! dictionaries, which security settings (M8) and our own test fixtures use.
//!
//! The crate knows nothing about the COS object model. Callers pass in the values of the
//! `/Encrypt` dictionary and the first element of the trailer `/ID`.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod aes_cbc;
mod rc4;

use md5::{Digest as _, Md5};
use sha2::{Sha256, Sha384, Sha512};

pub use aes_cbc::{aes_cbc_decrypt, aes_cbc_encrypt};
pub use rc4::rc4;

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum CryptError {
    #[error("the password is incorrect")]
    WrongPassword,
    #[error("unsupported security handler: {0}")]
    Unsupported(String),
    #[error("malformed encryption dictionary: {0}")]
    Malformed(String),
}

/// 32-byte padding string used by revisions 2–4 (§7.6.4.3.2, step a).
pub const PADDING: [u8; 32] = [
    0x28, 0xBF, 0x4E, 0x5E, 0x4E, 0x75, 0x8A, 0x41, 0x64, 0x00, 0x4E, 0x56, 0xFF, 0xFA, 0x01, 0x08, 0x2E, 0x2E, 0x00, 0xB6, 0xD0, 0x68, 0x3E, 0x80,
    0x2F, 0x0C, 0xA9, 0xFE, 0x64, 0x53, 0x69, 0x7A,
];

/// How a crypt filter encrypts (`/CFM`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    /// `None` / the `Identity` filter: data is not encrypted.
    Identity,
    /// `V2`: RC4.
    Rc4,
    /// `AESV2`: AES-128-CBC.
    Aes128,
    /// `AESV3`: AES-256-CBC.
    Aes256,
}

/// The values of an `/Encrypt` dictionary that the handler needs.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct EncryptDict {
    /// `/Filter` (must be `Standard`).
    pub filter: Vec<u8>,
    pub v: i64,
    pub r: i64,
    /// Key length in bits (`/Length`; 40 when absent).
    pub length_bits: i64,
    pub o: Vec<u8>,
    pub u: Vec<u8>,
    pub oe: Vec<u8>,
    pub ue: Vec<u8>,
    pub perms: Vec<u8>,
    pub p: i32,
    pub encrypt_metadata: bool,
    /// Crypt filters (`/CF`): name → (method, key length in bytes if given).
    pub crypt_filters: Vec<(Vec<u8>, Method)>,
    pub stm_f: Vec<u8>,
    pub str_f: Vec<u8>,
    pub ef_f: Vec<u8>,
}

/// Which password authenticated the document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Auth {
    Owner,
    User,
}

/// Permission flags from `/P`, read the way the handler's revision reads them (ISO 32000-1:2008,
/// Table 22). Owners have every permission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Permissions {
    pub bits: i32,
    pub owner: bool,
    /// The `/R` value of the handler that read `bits`.
    pub revision: i64,
}

impl Permissions {
    fn bit(&self, n: u32) -> bool {
        self.owner || (self.bits >> (n - 1)) & 1 == 1
    }
    /// Revision 3 and later read the full flag list of Table 22; revision 2 has its own.
    fn is_revision_3_or_later(&self) -> bool {
        self.revision >= 3
    }
    pub fn print(&self) -> bool {
        self.bit(3)
    }
    /// Modify contents other than the operations below.
    pub fn modify(&self) -> bool {
        self.bit(4)
    }
    pub fn copy(&self) -> bool {
        self.bit(5)
    }
    pub fn annotate(&self) -> bool {
        self.bit(6)
    }
    pub fn fill_forms(&self) -> bool {
        if self.is_revision_3_or_later() { self.bit(9) || self.bit(6) } else { self.bit(6) }
    }
    /// Revision 2 covers accessibility with the copy flag (bit 5); revision 3 and later with bit 10.
    pub fn extract_for_accessibility(&self) -> bool {
        if self.is_revision_3_or_later() { self.bit(10) } else { self.bit(5) }
    }
    /// Insert, rotate or delete pages; create bookmarks and thumbnails. Revision 3 and later grant
    /// it by bit 11 alone. Table 22 defines bit 11 for those revisions only, so at revision 2 the
    /// modify flag (bit 4) covers it.
    pub fn assemble(&self) -> bool {
        if self.is_revision_3_or_later() { self.bit(11) } else { self.bit(4) }
    }
    /// Revision 3 and later need bit 3 as well as bit 12. Table 22 defines bit 12 for those
    /// revisions only: at revision 2 the print flag (bit 3) is all there is.
    pub fn print_high_quality(&self) -> bool {
        if self.is_revision_3_or_later() { self.print() && self.bit(12) } else { self.print() }
    }
}

/// What kind of stream is being decrypted (selects the crypt filter).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamKind<'a> {
    Normal,
    /// An embedded file stream (uses `/EFF`).
    EmbeddedFile,
    /// The stream names its own crypt filter (`/Filter [/Crypt …] /DecodeParms << /Name … >>`).
    Named(&'a [u8]),
}

/// An authenticated security handler for one document.
#[derive(Clone, Debug)]
pub struct SecurityHandler {
    dict: EncryptDict,
    key: Vec<u8>,
    auth: Auth,
    recovered_user: Option<Vec<u8>>,
    methods: Methods,
}

/// The crypt filter methods of a handler, resolved once: a document controls both the number of
/// filters and the number of strings, so a search of `/CF` per string would multiply them.
#[derive(Clone, Debug)]
struct Methods {
    string: Method,
    stream: Method,
    embedded_file: Method,
    named: std::collections::HashMap<Vec<u8>, Method>,
}

impl Methods {
    fn new(dict: &EncryptDict) -> Self {
        let mut named = std::collections::HashMap::with_capacity(dict.crypt_filters.len());
        for (name, method) in &dict.crypt_filters {
            #[cfg(test)]
            tests::FILTER_COMPARES.with(|c| c.set(c.get() + 1));
            // The first of two filters with one name wins, as a search would find it.
            named.entry(name.clone()).or_insert(*method);
        }
        let mut methods = Methods { string: Method::Identity, stream: Method::Identity, embedded_file: Method::Identity, named };
        methods.string = methods.get(&dict.str_f);
        methods.stream = methods.get(&dict.stm_f);
        methods.embedded_file = methods.get(if dict.ef_f.is_empty() { &dict.stm_f } else { &dict.ef_f });
        methods
    }

    fn get(&self, name: &[u8]) -> Method {
        if name == b"Identity" || name.is_empty() {
            return Method::Identity;
        }
        #[cfg(test)]
        tests::FILTER_COMPARES.with(|c| c.set(c.get() + 1));
        self.named.get(name).copied().unwrap_or(Method::Identity)
    }
}

impl SecurityHandler {
    fn new(dict: EncryptDict, key: Vec<u8>, auth: Auth, recovered_user: Option<Vec<u8>>) -> Self {
        let methods = Methods::new(&dict);
        Self { dict, key, auth, recovered_user, methods }
    }

    /// Authenticate with `password` (tried as owner password, then user password). `None`
    /// tries the empty password, which opens documents that only have an owner password.
    pub fn open(dict: EncryptDict, id0: &[u8], password: Option<&str>) -> Result<Self, CryptError> {
        if !dict.filter.is_empty() && dict.filter != b"Standard" {
            return Err(CryptError::Unsupported(format!("/Filter /{}", String::from_utf8_lossy(&dict.filter))));
        }
        let pw = password.unwrap_or("");
        let mut recovered_user = None;
        let (key, auth) = match dict.r {
            2..=4 => {
                let mut found = None;
                for legacy in legacy_candidates(pw) {
                    if let Some((k, user)) = owner_key_r4(&dict, id0, &legacy) {
                        recovered_user = Some(user);
                        found = Some((k, Auth::Owner));
                    } else if let Some(k) = user_key_r4(&dict, id0, &legacy) {
                        found = Some((k, Auth::User));
                    }
                    if found.is_some() {
                        break;
                    }
                }
                found.ok_or(CryptError::WrongPassword)?
            }
            5 | 6 => {
                let pw = sasl_password(pw);
                if let Some(k) = key_r6(&dict, &pw, true)? {
                    (k, Auth::Owner)
                } else if let Some(k) = key_r6(&dict, &pw, false)? {
                    (k, Auth::User)
                } else {
                    return Err(CryptError::WrongPassword);
                }
            }
            r => return Err(CryptError::Unsupported(format!("revision {r}"))),
        };
        Ok(Self::new(dict, key, auth, recovered_user))
    }

    /// Upgrade to owner access with the owner (permissions) password, as when a restricted
    /// document is unlocked after opening. The user password, or any other, is `WrongPassword`.
    /// Both passwords unlock the same file key, so what was decrypted so far stays valid; a
    /// different key is refused too.
    pub fn authenticate_owner(&self, id0: &[u8], password: &str) -> Result<Self, CryptError> {
        let owner = Self::open(self.dict.clone(), id0, Some(password))?;
        if owner.auth != Auth::Owner || owner.key != self.key {
            return Err(CryptError::WrongPassword);
        }
        Ok(owner)
    }

    pub fn auth(&self) -> Auth {
        self.auth
    }

    /// For R2–R4 documents opened with the owner password: the user password, which owner
    /// authentication recovers (Algorithm 7). Lets components that only accept user passwords
    /// open the document too.
    pub fn recovered_user_password(&self) -> Option<Vec<u8>> {
        self.recovered_user.clone()
    }

    /// For R5/R6, `/P` is stored in clear and again in `/Perms`, encrypted with the file key
    /// (Algorithm 13). Where the two differ, a permission counts only when both grant it, so an
    /// edited `/P` grants nothing extra. Without a readable `/Perms`, `/P` applies as before.
    pub fn permissions(&self) -> Permissions {
        let bits = self.perms_p().map_or(self.dict.p, |p| self.dict.p & p);
        Permissions { bits, owner: self.auth == Auth::Owner, revision: self.dict.r }
    }

    /// `/P` as `/Perms` holds it: AES-256 ECB of one block (= CBC with a zero IV), marked `adb`.
    fn perms_p(&self) -> Option<i32> {
        if !matches!(self.dict.r, 5 | 6) {
            return None;
        }
        let plain = aes_cbc_decrypt(&self.key, &[0; 16], self.dict.perms.first_chunk::<16>()?, false);
        let p = i32::from_le_bytes(*plain.first_chunk::<4>()?);
        (plain.get(9..12) == Some(&b"adb"[..])).then_some(p)
    }

    pub fn dict(&self) -> &EncryptDict {
        &self.dict
    }

    /// The file encryption key (for tests and diagnostics).
    pub fn file_key(&self) -> &[u8] {
        &self.key
    }

    /// Whether `/Metadata` streams are encrypted.
    pub fn encrypts_metadata(&self) -> bool {
        self.dict.encrypt_metadata
    }

    /// The method for strings or a kind of stream.
    fn method(&self, stream: Option<StreamKind>) -> Method {
        if self.dict.v < 4 {
            return Method::Rc4;
        }
        match stream {
            None => self.methods.string,
            Some(StreamKind::Normal) => self.methods.stream,
            Some(StreamKind::EmbeddedFile) => self.methods.embedded_file,
            Some(StreamKind::Named(n)) => self.methods.get(n),
        }
    }

    /// Algorithm 1 (§7.6.3.3): the key for one object. AES-256 uses the file key directly.
    fn object_key(&self, method: Method, num: u32, generation: u16) -> Vec<u8> {
        if method == Method::Aes256 {
            return self.key.clone();
        }
        let mut h = Md5::new();
        h.update(&self.key);
        h.update(&num.to_le_bytes()[..3]);
        h.update(generation.to_le_bytes());
        if method == Method::Aes128 {
            h.update(b"sAlT");
        }
        let digest = h.finalize();
        digest[..(self.key.len() + 5).min(16)].to_vec()
    }

    fn apply(&self, method: Method, num: u32, generation: u16, data: &[u8], encrypt: bool) -> Vec<u8> {
        match method {
            Method::Identity => data.to_vec(),
            Method::Rc4 => rc4(&self.object_key(method, num, generation), data),
            Method::Aes128 | Method::Aes256 => {
                let key = self.object_key(method, num, generation);
                if encrypt {
                    // Deterministic IV derived from the key, object and content: saves are
                    // reproducible and no IV repeats for different plaintexts.
                    let mut h = Sha256::new();
                    h.update(&key);
                    h.update(num.to_le_bytes());
                    h.update(generation.to_le_bytes());
                    h.update(data);
                    let digest = h.finalize();
                    let mut iv = [0u8; 16];
                    for (d, s) in iv.iter_mut().zip(digest.iter()) {
                        *d = *s;
                    }
                    let mut out = iv.to_vec();
                    out.extend(aes_cbc_encrypt(&key, &iv, data, true));
                    out
                } else {
                    let Some((iv, body)) = data.split_first_chunk::<16>() else {
                        return Vec::new(); // only (part of) an IV: empty plaintext
                    };
                    aes_cbc_decrypt(&key, iv, body, true)
                }
            }
        }
    }

    pub fn decrypt_string(&self, num: u32, generation: u16, data: &[u8]) -> Vec<u8> {
        self.apply(self.method(None), num, generation, data, false)
    }

    pub fn encrypt_string(&self, num: u32, generation: u16, data: &[u8]) -> Vec<u8> {
        self.apply(self.method(None), num, generation, data, true)
    }

    pub fn decrypt_stream(&self, num: u32, generation: u16, data: &[u8], kind: StreamKind) -> Vec<u8> {
        self.apply(self.method(Some(kind)), num, generation, data, false)
    }

    pub fn encrypt_stream(&self, num: u32, generation: u16, data: &[u8], kind: StreamKind) -> Vec<u8> {
        self.apply(self.method(Some(kind)), num, generation, data, true)
    }

    /// Whether strings are encrypted at all (false with `/StrF /Identity`).
    pub fn encrypts_strings(&self) -> bool {
        self.method(None) != Method::Identity
    }
}

// ── Passwords ──────────────────────────────────────────────────────────────────────────────────

/// PDFDocEncoding (ISO 32000-2 Annex D) where it differs from Latin-1: bytes 0x18–0x1F and
/// 0x80–0xA0 (0x9F is undefined). Here rather than in pdfcraft-cos, which decodes text strings
/// with it, because R2–R4 passwords need it too and cos depends on this crate.
const PDFDOC_LOW: [char; 8] = ['˘', 'ˇ', 'ˆ', '˙', '˝', '˛', '˚', '˜'];
const PDFDOC_HIGH: [char; 33] = [
    '•', '†', '‡', '…', '—', '–', 'ƒ', '⁄', '‹', '›', '−', '‰', '„', '“', '”', '‘', '’', '‚', '™', 'ﬁ', 'ﬂ', 'Ł', 'Œ', 'Š', 'Ÿ', 'Ž', 'ı', 'ł', 'œ',
    'š', 'ž', '\u{FFFD}', '€',
];

/// PDFDocEncoding → Unicode.
pub fn pdfdoc_char(c: u8) -> char {
    match c {
        0x18..=0x1F => PDFDOC_LOW[usize::from(c - 0x18)],
        0x80..=0xA0 => PDFDOC_HIGH[usize::from(c - 0x80)],
        _ => char::from(c),
    }
}

/// Unicode → PDFDocEncoding, `None` for a character it lacks (ğ, ş, İ, Cyrillic, CJK, …).
pub fn pdfdoc_byte(c: char) -> Option<u8> {
    let at = |table: &[char], base: u8| table.iter().position(|&t| t == c && t != '\u{FFFD}').and_then(|i| u8::try_from(i).ok()).map(|i| base + i);
    at(&PDFDOC_LOW, 0x18).or_else(|| at(&PDFDOC_HIGH, 0x80)).or_else(|| match u32::from(c) {
        0x18..=0x1F | 0x80..=0xA0 => None,
        n => u8::try_from(n).ok(),
    })
}

/// Revisions 2–4: the password in PDFDocEncoding (§7.6.4.3.2, Algorithm 2), at most 32 bytes.
/// Characters PDFDocEncoding lacks are dropped.
fn legacy_password(pw: &str) -> Vec<u8> {
    pw.chars().filter_map(pdfdoc_byte).take(32).collect()
}

/// The bytes an R2–R4 password may have been written as: PDFDocEncoding, as the specification
/// asks; and for a password with characters PDFDocEncoding lacks (ğ, ş, İ, …), which it gives
/// no bytes, also its UTF-8 bytes, which qpdf and other tools write for such passwords.
fn legacy_candidates(pw: &str) -> Vec<Vec<u8>> {
    let mut out = vec![legacy_password(pw)];
    if pw.chars().any(|c| pdfdoc_byte(c).is_none()) {
        out.push(pw.bytes().take(32).collect());
    }
    out
}

/// Revisions 5–6: SASLprep (RFC 4013), UTF-8, at most 127 bytes.
fn sasl_password(pw: &str) -> Vec<u8> {
    let prepped = stringprep::saslprep(pw).map(|c| c.into_owned()).unwrap_or_else(|_| pw.to_string());
    let mut bytes = prepped.into_bytes();
    bytes.truncate(127);
    bytes
}

fn pad(pw: &[u8]) -> [u8; 32] {
    let mut out = PADDING;
    let n = pw.len().min(32);
    out[..n].copy_from_slice(&pw[..n]);
    out[n..].copy_from_slice(&PADDING[..32 - n]);
    out
}

fn key_len(d: &EncryptDict) -> usize {
    if d.r == 2 { 5 } else { (d.length_bits.clamp(40, 128) / 8) as usize }
}

// ── Revisions 2–4 ──────────────────────────────────────────────────────────────────────────────

/// Algorithm 2: file key from a (user) password.
fn file_key_r4(d: &EncryptDict, id0: &[u8], pw: &[u8]) -> Vec<u8> {
    let n = key_len(d);
    let mut h = Md5::new();
    h.update(pad(pw));
    h.update(&d.o[..d.o.len().min(32)]);
    h.update(d.p.to_le_bytes());
    h.update(id0);
    if d.r >= 4 && !d.encrypt_metadata {
        h.update([0xFF; 4]);
    }
    let mut digest = h.finalize().to_vec();
    if d.r >= 3 {
        for _ in 0..50 {
            digest = Md5::digest(&digest[..n]).to_vec();
        }
    }
    digest.truncate(n);
    digest
}

/// Algorithms 4/5: the `/U` value a key produces.
fn u_value_r4(d: &EncryptDict, id0: &[u8], key: &[u8]) -> Vec<u8> {
    if d.r == 2 {
        return rc4(key, &PADDING);
    }
    let mut h = Md5::new();
    h.update(PADDING);
    h.update(id0);
    let mut x = rc4(key, &h.finalize());
    for i in 1..=19u8 {
        let k: Vec<u8> = key.iter().map(|b| b ^ i).collect();
        x = rc4(&k, &x);
    }
    x.extend_from_slice(&[0u8; 16]);
    x
}

/// Algorithm 6: authenticate a user password; returns the file key.
///
/// Some producers ignore `/EncryptMetadata false` when deriving the key (they omit the
/// `FF FF FF FF` of Algorithm 2, step f). Viewers open those files, so if the strict derivation
/// fails we retry the other way.
fn user_key_r4(d: &EncryptDict, id0: &[u8], pw: &[u8]) -> Option<Vec<u8>> {
    let check = |d: &EncryptDict| {
        let key = file_key_r4(d, id0, pw);
        let u = u_value_r4(d, id0, &key);
        let n = if d.r == 2 { 32 } else { 16 };
        (d.u.len() >= n && u[..n] == d.u[..n]).then_some(key)
    };
    check(d).or_else(|| (d.r >= 4 && !d.encrypt_metadata).then(|| check(&EncryptDict { encrypt_metadata: true, ..d.clone() })).flatten())
}

/// The RC4 key derived from an owner password (Algorithm 3, steps a–d).
fn owner_rc4_key(d: &EncryptDict, owner_pw: &[u8]) -> Vec<u8> {
    let n = key_len(d);
    let mut digest = Md5::digest(pad(owner_pw)).to_vec();
    if d.r >= 3 {
        for _ in 0..50 {
            digest = Md5::digest(&digest).to_vec();
        }
    }
    digest.truncate(n);
    digest
}

/// Algorithm 7: authenticate an owner password (recover the user password from `/O`).
/// Returns the file key and the recovered user password (padding removed).
fn owner_key_r4(d: &EncryptDict, id0: &[u8], pw: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    let rk = owner_rc4_key(d, pw);
    let mut user = d.o[..d.o.len().min(32)].to_vec();
    if d.r == 2 {
        user = rc4(&rk, &user);
    } else {
        for i in (0..=19u8).rev() {
            let k: Vec<u8> = rk.iter().map(|b| b ^ i).collect();
            user = rc4(&k, &user);
        }
    }
    let key = user_key_r4(d, id0, &user)?;
    // The recovered value is the padded password: strip the padding suffix.
    let len = (0..=32).find(|&i| user.len() >= 32 && user[i..32] == PADDING[..32 - i]).unwrap_or(user.len().min(32));
    Some((key, user[..len].to_vec()))
}

// ── Revisions 5–6 ──────────────────────────────────────────────────────────────────────────────

/// The hash of a password for R5 (SHA-256) or R6 (Algorithm 2.B).
fn hash_r6(r: i64, pw: &[u8], salt: &[u8], udata: &[u8]) -> Vec<u8> {
    let mut k = {
        let mut h = Sha256::new();
        h.update(pw);
        h.update(salt);
        h.update(udata);
        h.finalize().to_vec()
    };
    if r == 5 {
        return k;
    }
    let mut round = 0u32;
    loop {
        let mut k1 = Vec::with_capacity(64 * (pw.len() + k.len() + udata.len()));
        for _ in 0..64 {
            k1.extend_from_slice(pw);
            k1.extend_from_slice(&k);
            k1.extend_from_slice(udata);
        }
        // `k` is always a SHA-2 digest (32, 48 or 64 bytes): key = its first 16 bytes, IV = the next 16.
        let Some((key, rest)) = k.split_first_chunk::<16>() else { break };
        let Some(iv) = rest.first_chunk::<16>() else { break };
        let e = aes_cbc_encrypt(key, iv, &k1, false);
        let m = e.iter().take(16).map(|b| u32::from(*b)).sum::<u32>() % 3;
        k = match m {
            0 => Sha256::digest(&e).to_vec(),
            1 => Sha384::digest(&e).to_vec(),
            _ => Sha512::digest(&e).to_vec(),
        };
        round += 1;
        if round >= 64 && e.last().is_none_or(|b| u32::from(*b) <= round - 32) {
            break;
        }
    }
    k.truncate(32);
    k
}

/// Algorithm 2.A: authenticate as owner (`owner = true`) or user; returns the file key.
fn key_r6(d: &EncryptDict, pw: &[u8], owner: bool) -> Result<Option<Vec<u8>>, CryptError> {
    if d.u.len() < 48 || d.o.len() < 48 {
        return Err(CryptError::Malformed("/O and /U must be 48 bytes for R5/R6".into()));
    }
    let (hashed, encrypted, udata) = if owner { (&d.o[..48], &d.oe, &d.u[..48]) } else { (&d.u[..48], &d.ue, &[][..]) };
    if hash_r6(d.r, pw, &hashed[32..40], udata) != hashed[..32] {
        return Ok(None);
    }
    if encrypted.len() < 32 {
        return Err(CryptError::Malformed("/OE or /UE must be 32 bytes".into()));
    }
    let k = hash_r6(d.r, pw, &hashed[40..48], udata);
    Ok(Some(aes_cbc_decrypt(&k, &[0; 16], &encrypted[..32], false)))
}

// ── Creating encryption ────────────────────────────────────────────────────────────────────────

/// Which encryption to create.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Algorithm {
    /// R2, 40-bit RC4 (legacy; for tests and compatibility only).
    Rc4_40,
    /// R3, 128-bit RC4.
    Rc4_128,
    /// R4, AES-128 crypt filters.
    Aes128,
    /// R6, AES-256 (PDF 2.0; the default for new documents).
    Aes256,
}

/// A password as [`create`] prepares it: for R2–R4 its PDFDocEncoding (characters it lacks
/// dropped), at most 32 bytes; for R6 its SASLprep (RFC 4013: soft hyphens and the like removed,
/// other spaces made plain, then NFKC; the password as given if SASLprep refuses it), in UTF-8,
/// at most 127 bytes.
fn prepared_password(algorithm: Algorithm, password: &str) -> Vec<u8> {
    match algorithm {
        Algorithm::Rc4_40 | Algorithm::Rc4_128 | Algorithm::Aes128 => legacy_password(password),
        Algorithm::Aes256 => sasl_password(password),
    }
}

/// The bytes new encryption with `algorithm` derives its keys from for `password`: for R2–R4 its
/// PDFDocEncoding cut or padded to exactly 32 bytes (Algorithm 2); for R6 its SASLprep (the
/// password as given if SASLprep refuses it), at most 127 bytes of UTF-8. Two passwords with the
/// same bytes are the same password to the file, and a password with the bytes of `""` opens it
/// without asking, so callers can refuse either before encrypting. Different bytes can still give
/// the same key (RC4-40 keeps 5 bytes of a hash). `create` still uses the user password for an
/// empty R2–R4 owner password.
pub fn effective_password(algorithm: Algorithm, password: &str) -> Vec<u8> {
    let prepared = prepared_password(algorithm, password);
    match algorithm {
        Algorithm::Rc4_40 | Algorithm::Rc4_128 | Algorithm::Aes128 => pad(&prepared).to_vec(),
        Algorithm::Aes256 => prepared,
    }
}

/// Parameters for new encryption.
#[derive(Clone, Debug)]
pub struct NewEncryption<'a> {
    pub algorithm: Algorithm,
    pub user_password: &'a str,
    pub owner_password: &'a str,
    pub permissions: i32,
    pub encrypt_metadata: bool,
    /// 32 bytes of entropy for keys and salts (the caller supplies randomness, so this crate
    /// needs no OS access and tests are deterministic).
    pub seed: [u8; 32],
}

/// Expand `seed` into `n` bytes (SHA-256 in counter mode).
fn expand(seed: &[u8; 32], label: &[u8], n: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut counter = 0u32;
    while out.len() < n {
        let mut h = Sha256::new();
        h.update(seed);
        h.update(label);
        h.update(counter.to_le_bytes());
        out.extend_from_slice(&h.finalize());
        counter = counter.saturating_add(1);
    }
    out.truncate(n);
    out
}

/// Sixteen bytes of file identifier from `seed` (SHA-256 in counter mode, label `id`).
///
/// `/ID` is written in the clear. Deriving it the same way as the file key keeps that public
/// value from being an unkeyed hash of the seed.
pub fn file_id(seed: &[u8; 32]) -> Vec<u8> {
    expand(seed, b"id", 16)
}

/// Build the `/Encrypt` values and an authenticated (owner) handler for a new encryption.
pub fn create(params: &NewEncryption, id0: &[u8]) -> Result<SecurityHandler, CryptError> {
    // Bits 7–8 and 13–32 must be 1; bits 1–2 must be 0 (§7.6.4.2).
    let p = (params.permissions | !0xFFF | 0xC0) & !0x3;
    let mut d = EncryptDict { filter: b"Standard".to_vec(), p, encrypt_metadata: params.encrypt_metadata, ..Default::default() };
    let user = prepared_password(params.algorithm, params.user_password);
    let owner = prepared_password(params.algorithm, params.owner_password);
    match params.algorithm {
        Algorithm::Rc4_40 | Algorithm::Rc4_128 | Algorithm::Aes128 => {
            (d.v, d.r, d.length_bits) = match params.algorithm {
                Algorithm::Rc4_40 => (1, 2, 40),
                Algorithm::Rc4_128 => (2, 3, 128),
                _ => (4, 4, 128),
            };
            if params.algorithm == Algorithm::Aes128 {
                d.crypt_filters = vec![(b"StdCF".to_vec(), Method::Aes128)];
                d.stm_f = b"StdCF".to_vec();
                d.str_f = b"StdCF".to_vec();
            } else {
                d.encrypt_metadata = true; // V < 4 always encrypts metadata
            }
            let owner = if params.owner_password.is_empty() { user.clone() } else { owner };
            // Algorithm 3: /O
            let rk = owner_rc4_key(&d, &owner);
            let mut o = rc4(&rk, &pad(&user));
            if d.r >= 3 {
                for i in 1..=19u8 {
                    let k: Vec<u8> = rk.iter().map(|b| b ^ i).collect();
                    o = rc4(&k, &o);
                }
            }
            d.o = o;
            let key = file_key_r4(&d, id0, &user);
            d.u = u_value_r4(&d, id0, &key);
            Ok(SecurityHandler::new(d, key, Auth::Owner, Some(user)))
        }
        Algorithm::Aes256 => {
            (d.v, d.r, d.length_bits) = (5, 6, 256);
            d.crypt_filters = vec![(b"StdCF".to_vec(), Method::Aes256)];
            d.stm_f = b"StdCF".to_vec();
            d.str_f = b"StdCF".to_vec();
            let key = expand(&params.seed, b"file key", 32);
            let salts = expand(&params.seed, b"salts", 32);
            // Algorithm 8: /U, /UE
            let mut u = hash_r6(6, &user, &salts[0..8], &[]);
            u.extend_from_slice(&salts[0..16]);
            let uk = hash_r6(6, &user, &salts[8..16], &[]);
            d.ue = aes_cbc_encrypt(&uk, &[0; 16], &key, false);
            // Algorithm 9: /O, /OE
            let mut o = hash_r6(6, &owner, &salts[16..24], &u);
            o.extend_from_slice(&salts[16..32]);
            let ok = hash_r6(6, &owner, &salts[24..32], &u);
            d.oe = aes_cbc_encrypt(&ok, &[0; 16], &key, false);
            d.u = u;
            d.o = o;
            // Algorithm 10: /Perms
            let mut perms = [0u8; 16];
            perms[..4].copy_from_slice(&p.to_le_bytes());
            perms[4..8].copy_from_slice(&[0xFF; 4]);
            perms[8] = if d.encrypt_metadata { b'T' } else { b'F' };
            perms[9..12].copy_from_slice(b"adb");
            perms[12..16].copy_from_slice(&expand(&params.seed, b"perms", 4));
            d.perms = aes_cbc_encrypt(&key, &[0; 16], &perms, false);
            Ok(SecurityHandler::new(d, key, Auth::Owner, None))
        }
    }
}

#[cfg(test)]
mod tests;
