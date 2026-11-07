//! Photoshop ActionDescriptor structures (OSType-tagged key/value trees).
//!
//! These appear inside many tagged blocks (`lfx2`, `SoCo`, `GdFl`, `TySh`,
//! `PlLd`, …), usually wrapped in a [`VersionedDescriptor`] (version 16).
//! Parsing and writing are exact: `write(parse(b)) == b` for well-formed input.

use crate::error::{PsdError, Result};
use crate::io::{Reader, WriteExt, read_unicode_units, write_unicode_units};

/// Maximum nesting depth accepted when parsing (fuzz safety).
pub const MAX_DEPTH: usize = 64;

/// A Photoshop Unicode string: UTF-16 code units, preserved exactly
/// (including any trailing NUL and unpaired surrogates).
#[derive(Debug, Clone, PartialEq, Eq, Default, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct UnicodeString(pub Vec<u16>);

impl UnicodeString {
    /// Encodes `s` as UTF-16 without a trailing NUL.
    pub fn new(s: &str) -> Self {
        UnicodeString(s.encode_utf16().collect())
    }
    /// Encodes `s` as UTF-16 followed by a NUL terminator (as Photoshop
    /// commonly writes in descriptors).
    pub fn new_nul(s: &str) -> Self {
        let mut v: Vec<u16> = s.encode_utf16().collect();
        v.push(0);
        UnicodeString(v)
    }
    /// Lossy conversion, stripping trailing NULs.
    pub fn to_string_lossy(&self) -> String {
        let mut end = self.0.len();
        while end > 0 && self.0[end - 1] == 0 {
            end -= 1;
        }
        String::from_utf16_lossy(&self.0[..end])
    }
    pub(crate) fn read(r: &mut Reader<'_>) -> Result<Self> {
        Ok(UnicodeString(read_unicode_units(r)?))
    }
    pub(crate) fn write(&self, out: &mut Vec<u8>) {
        write_unicode_units(out, &self.0);
    }
}

/// Four-character class and key ids Photoshop writes as length-4 strings, never as codes
/// (`00000004 'warp'`, not `00000000 'warp'`).
pub const STRING_IDS: [&str; 3] = ["warp", "list", "time"];

/// A class / key / type identifier: a length-prefixed string, where a stored
/// length of 0 means a 4-character code follows.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Id {
    /// Stored as length 0 + 4 bytes.
    Code([u8; 4]),
    /// Stored as explicit length + bytes (e.g. `"artboardRect"`).
    Str(Vec<u8>),
}

impl Id {
    /// Convenience constructor: 4-byte strings become [`Id::Code`], others
    /// [`Id::Str`]. The four-character names Photoshop stores as length-4 strings
    /// ([`STRING_IDS`]: `warp`, `list`, `time`) are [`Id::Str`] too: Photoshop treats the
    /// string and the code as different keys and doesn't recognise the code form.
    pub fn new(s: &str) -> Self {
        let b = s.as_bytes();
        match b {
            [a, b, c, d] if !STRING_IDS.contains(&s) => Id::Code([*a, *b, *c, *d]),
            _ => Id::Str(b.to_vec()),
        }
    }
    /// The identifier's bytes.
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Id::Code(c) => c,
            Id::Str(s) => s,
        }
    }
    /// `true` if the identifier's bytes equal `s`.
    pub fn is(&self, s: &str) -> bool {
        self.as_bytes() == s.as_bytes()
    }
    pub(crate) fn read(r: &mut Reader<'_>) -> Result<Self> {
        let n = r.u32()?;
        if n == 0 { Ok(Id::Code(r.array()?)) } else { Ok(Id::Str(r.bytes_u64(u64::from(n))?.to_vec())) }
    }
    pub(crate) fn write(&self, out: &mut Vec<u8>) {
        match self {
            Id::Code(c) => {
                out.put_u32(0);
                out.put(c);
            }
            Id::Str(s) => {
                out.put_u32(s.len() as u32);
                out.put(s);
            }
        }
    }
}

/// A class reference: display name + class id.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Class {
    /// Display name.
    pub name: UnicodeString,
    /// Class identifier.
    pub class_id: Id,
}

impl Class {
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        Ok(Class { name: UnicodeString::read(r)?, class_id: Id::read(r)? })
    }
    fn write(&self, out: &mut Vec<u8>) {
        self.name.write(out);
        self.class_id.write(out);
    }
}

/// One element of an `'obj '` reference.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ReferenceItem {
    /// `prop`
    Property {
        /// Class.
        class: Class,
        /// Property key.
        key: Id,
    },
    /// `Clss`
    Class(Class),
    /// `Enmr`
    Enumerated {
        /// Class.
        class: Class,
        /// Enum type.
        type_id: Id,
        /// Enum value.
        value: Id,
    },
    /// `rele`
    Offset {
        /// Class.
        class: Class,
        /// Offset.
        offset: i32,
    },
    /// `Idnt`: a class plus an identifier (Photoshop's layer ids in action references).
    Identifier {
        /// Class.
        class: Class,
        /// Identifier.
        id: u32,
    },
    /// `indx`: a class plus a zero-based index.
    Index {
        /// Class.
        class: Class,
        /// Index.
        index: u32,
    },
    /// `name`
    Name {
        /// Class.
        class: Class,
        /// Name.
        name: UnicodeString,
    },
}

/// A descriptor value, tagged by its OSType.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Value {
    /// `obj `
    Reference(Vec<ReferenceItem>),
    /// `Objc`
    Descriptor(Descriptor),
    /// `GlbO`
    GlobalObject(Descriptor),
    /// `VlLs`
    List(Vec<Value>),
    /// `doub`
    Double(f64),
    /// `UntF`: unit (`#Ang`, `#Rsl`, `#Rlt`, `#Nne`, `#Prc`, `#Pxl`, …) + value.
    UnitFloat {
        /// Unit code.
        unit: [u8; 4],
        /// Value.
        value: f64,
    },
    /// `UnFl`: unit + array of doubles (used in `ObAr`).
    UnitFloats {
        /// Unit code.
        unit: [u8; 4],
        /// Values.
        values: Vec<f64>,
    },
    /// `TEXT`
    Text(UnicodeString),
    /// `enum`
    Enumerated {
        /// Enum type.
        type_id: Id,
        /// Enum value.
        value: Id,
    },
    /// `long`
    Integer(i32),
    /// `comp`
    LargeInteger(i64),
    /// `bool`
    Boolean(bool),
    /// `type`
    Class(Class),
    /// `GlbC`
    GlobalClass(Class),
    /// `alis`: raw alias data.
    Alias(Vec<u8>),
    /// `Pth `: raw file path data (length-prefixed).
    Path(Vec<u8>),
    /// `tdta`: raw data.
    RawData(Vec<u8>),
    /// `ObAr`: object array (best effort; see [`ObjectArray`]).
    ObjectArray(ObjectArray),
}

/// `ObAr` object array. Undocumented in the Adobe spec; the layout follows
/// ag-psd (MIT): a u32 (observed as the element count / version 16) followed
/// by a descriptor-shaped body whose values are usually `UnFl`.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ObjectArray {
    /// Leading u32, preserved.
    pub prefix: u32,
    /// Body.
    pub body: Descriptor,
}

/// An ActionDescriptor: class plus ordered key/value items.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Descriptor {
    /// Display name of the class (often empty).
    pub name: UnicodeString,
    /// Class id.
    pub class_id: Id,
    /// Items in stored order.
    pub items: Vec<(Id, Value)>,
}

impl Default for Descriptor {
    fn default() -> Self {
        Descriptor::new("null")
    }
}

impl Descriptor {
    /// Creates an empty descriptor of the given class.
    pub fn new(class_id: &str) -> Self {
        Descriptor { name: UnicodeString::default(), class_id: Id::new(class_id), items: Vec::new() }
    }

    /// Builder-style item append.
    pub fn with(mut self, key: &str, value: Value) -> Self {
        self.items.push((Id::new(key), value));
        self
    }

    /// Looks up the first item with key `key`.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.items.iter().find(|(k, _)| k.is(key)).map(|(_, v)| v)
    }

    /// Parses a descriptor from `data`, which must be consumed entirely.
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let d = Self::read(&mut r)?;
        if !r.is_empty() {
            return Err(PsdError::invalid("trailing bytes after descriptor"));
        }
        Ok(d)
    }

    /// Serializes the descriptor.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = Vec::new();
        self.write(&mut v);
        v
    }

    pub(crate) fn read(r: &mut Reader<'_>) -> Result<Self> {
        read_descriptor(r, 0)
    }

    pub(crate) fn write(&self, out: &mut Vec<u8>) {
        self.name.write(out);
        self.class_id.write(out);
        out.put_u32(self.items.len() as u32);
        for (k, v) in &self.items {
            k.write(out);
            write_value(v, out);
        }
    }
}

/// A descriptor prefixed by a u32 version (16 in all current Photoshop files).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct VersionedDescriptor {
    /// Descriptor version (16).
    pub version: u32,
    /// The descriptor.
    pub descriptor: Descriptor,
}

impl VersionedDescriptor {
    /// Wraps a descriptor with version 16.
    pub fn new(descriptor: Descriptor) -> Self {
        VersionedDescriptor { version: 16, descriptor }
    }
    /// Parses from bytes (must be fully consumed).
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let v = Self::read(&mut r)?;
        if !r.is_empty() {
            return Err(PsdError::invalid("trailing bytes after versioned descriptor"));
        }
        Ok(v)
    }
    /// Parses a versioned descriptor at the start of `data`, returning it and
    /// the number of bytes consumed (trailing data is allowed, e.g. the warp
    /// descriptor after the text descriptor in `TySh`).
    pub fn parse_prefix(data: &[u8]) -> Result<(Self, usize)> {
        let mut r = Reader::new(data);
        let v = Self::read(&mut r)?;
        Ok((v, r.pos()))
    }
    /// Serializes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = Vec::new();
        self.write(&mut v);
        v
    }
    pub(crate) fn read(r: &mut Reader<'_>) -> Result<Self> {
        let version = r.u32()?;
        if version != 16 {
            return Err(PsdError::invalid(format!("descriptor version {version}, expected 16")));
        }
        Ok(VersionedDescriptor { version, descriptor: Descriptor::read(r)? })
    }
    pub(crate) fn write(&self, out: &mut Vec<u8>) {
        out.put_u32(self.version);
        self.descriptor.write(out);
    }
}

/// Most items preallocated for a file-declared count. `check_count` only
/// guarantees a few bytes per item while parsed items are far larger, so the
/// count must not drive the up-front reservation (or abort on allocation).
const MAX_PREALLOC: usize = 4096;

pub(crate) fn vec_for_count<T>(n: u32) -> Vec<T> {
    Vec::with_capacity((n as usize).min(MAX_PREALLOC))
}

fn read_descriptor(r: &mut Reader<'_>, depth: usize) -> Result<Descriptor> {
    if depth > MAX_DEPTH {
        return Err(PsdError::LimitExceeded("descriptor nesting too deep"));
    }
    let name = UnicodeString::read(r)?;
    let class_id = Id::read(r)?;
    let n = r.u32()?;
    // Each item needs at least 5 (1-byte string key) + 4 (type) + 1 (bool).
    r.check_count(u64::from(n), 10)?;
    let mut items = vec_for_count(n);
    for _ in 0..n {
        let key = Id::read(r)?;
        let v = read_value(r, depth + 1)?;
        items.push((key, v));
    }
    Ok(Descriptor { name, class_id, items })
}

fn read_value(r: &mut Reader<'_>, depth: usize) -> Result<Value> {
    if depth > MAX_DEPTH {
        return Err(PsdError::LimitExceeded("descriptor nesting too deep"));
    }
    let ty = r.array::<4>()?;
    read_typed_value(r, &ty, depth)
}

fn read_typed_value(r: &mut Reader<'_>, ty: &[u8; 4], depth: usize) -> Result<Value> {
    Ok(match ty {
        b"obj " => {
            let n = r.u32()?;
            r.check_count(u64::from(n), 8)?;
            let mut items = vec_for_count(n);
            for _ in 0..n {
                items.push(read_reference_item(r)?);
            }
            Value::Reference(items)
        }
        b"Objc" => Value::Descriptor(read_descriptor(r, depth + 1)?),
        b"GlbO" => Value::GlobalObject(read_descriptor(r, depth + 1)?),
        b"VlLs" => {
            let n = r.u32()?;
            r.check_count(u64::from(n), 4)?;
            let mut items = vec_for_count(n);
            for _ in 0..n {
                items.push(read_value(r, depth + 1)?);
            }
            Value::List(items)
        }
        b"doub" => Value::Double(r.f64()?),
        b"UntF" => Value::UnitFloat { unit: r.array()?, value: r.f64()? },
        b"UnFl" => {
            let unit = r.array()?;
            let n = r.u32()?;
            r.check_count(u64::from(n), 8)?;
            let mut values = vec_for_count(n);
            for _ in 0..n {
                values.push(r.f64()?);
            }
            Value::UnitFloats { unit, values }
        }
        b"TEXT" => Value::Text(UnicodeString::read(r)?),
        b"enum" => Value::Enumerated { type_id: Id::read(r)?, value: Id::read(r)? },
        b"long" => Value::Integer(r.i32()?),
        b"comp" => Value::LargeInteger(r.i64()?),
        b"bool" => Value::Boolean(r.u8()? != 0),
        b"type" => Value::Class(Class::read(r)?),
        b"GlbC" => Value::GlobalClass(Class::read(r)?),
        b"alis" => {
            let n = r.u32()?;
            Value::Alias(r.bytes_u64(u64::from(n))?.to_vec())
        }
        b"Pth " => {
            let n = r.u32()?;
            Value::Path(r.bytes_u64(u64::from(n))?.to_vec())
        }
        b"tdta" => {
            let n = r.u32()?;
            Value::RawData(r.bytes_u64(u64::from(n))?.to_vec())
        }
        b"ObAr" => {
            let prefix = r.u32()?;
            Value::ObjectArray(ObjectArray { prefix, body: read_descriptor(r, depth + 1)? })
        }
        other => {
            return Err(PsdError::Unsupported(format!("descriptor OSType {:?}", String::from_utf8_lossy(other))));
        }
    })
}

fn read_reference_item(r: &mut Reader<'_>) -> Result<ReferenceItem> {
    let ty = r.array::<4>()?;
    Ok(match &ty {
        b"prop" => ReferenceItem::Property { class: Class::read(r)?, key: Id::read(r)? },
        b"Clss" => ReferenceItem::Class(Class::read(r)?),
        b"Enmr" => ReferenceItem::Enumerated { class: Class::read(r)?, type_id: Id::read(r)?, value: Id::read(r)? },
        b"rele" => ReferenceItem::Offset { class: Class::read(r)?, offset: r.i32()? },
        b"Idnt" => ReferenceItem::Identifier { class: Class::read(r)?, id: r.u32()? },
        b"indx" => ReferenceItem::Index { class: Class::read(r)?, index: r.u32()? },
        b"name" => ReferenceItem::Name { class: Class::read(r)?, name: UnicodeString::read(r)? },
        other => {
            return Err(PsdError::Unsupported(format!("reference item type {:?}", String::from_utf8_lossy(other))));
        }
    })
}

fn value_type(v: &Value) -> &'static [u8; 4] {
    match v {
        Value::Reference(_) => b"obj ",
        Value::Descriptor(_) => b"Objc",
        Value::GlobalObject(_) => b"GlbO",
        Value::List(_) => b"VlLs",
        Value::Double(_) => b"doub",
        Value::UnitFloat { .. } => b"UntF",
        Value::UnitFloats { .. } => b"UnFl",
        Value::Text(_) => b"TEXT",
        Value::Enumerated { .. } => b"enum",
        Value::Integer(_) => b"long",
        Value::LargeInteger(_) => b"comp",
        Value::Boolean(_) => b"bool",
        Value::Class(_) => b"type",
        Value::GlobalClass(_) => b"GlbC",
        Value::Alias(_) => b"alis",
        Value::Path(_) => b"Pth ",
        Value::RawData(_) => b"tdta",
        Value::ObjectArray(_) => b"ObAr",
    }
}

fn write_value(v: &Value, out: &mut Vec<u8>) {
    out.put(value_type(v));
    match v {
        Value::Reference(items) => {
            out.put_u32(items.len() as u32);
            for it in items {
                write_reference_item(it, out);
            }
        }
        Value::Descriptor(d) | Value::GlobalObject(d) => d.write(out),
        Value::List(items) => {
            out.put_u32(items.len() as u32);
            for it in items {
                write_value(it, out);
            }
        }
        Value::Double(d) => out.put_f64(*d),
        Value::UnitFloat { unit, value } => {
            out.put(unit);
            out.put_f64(*value);
        }
        Value::UnitFloats { unit, values } => {
            out.put(unit);
            out.put_u32(values.len() as u32);
            for v in values {
                out.put_f64(*v);
            }
        }
        Value::Text(s) => s.write(out),
        Value::Enumerated { type_id, value } => {
            type_id.write(out);
            value.write(out);
        }
        Value::Integer(i) => out.put_i32(*i),
        Value::LargeInteger(i) => out.put_i64(*i),
        Value::Boolean(b) => out.put_u8(u8::from(*b)),
        Value::Class(c) | Value::GlobalClass(c) => c.write(out),
        Value::Alias(b) | Value::Path(b) | Value::RawData(b) => {
            out.put_u32(b.len() as u32);
            out.put(b);
        }
        Value::ObjectArray(o) => {
            out.put_u32(o.prefix);
            o.body.write(out);
        }
    }
}

fn write_reference_item(it: &ReferenceItem, out: &mut Vec<u8>) {
    match it {
        ReferenceItem::Property { class, key } => {
            out.put(b"prop");
            class.write(out);
            key.write(out);
        }
        ReferenceItem::Class(c) => {
            out.put(b"Clss");
            c.write(out);
        }
        ReferenceItem::Enumerated { class, type_id, value } => {
            out.put(b"Enmr");
            class.write(out);
            type_id.write(out);
            value.write(out);
        }
        ReferenceItem::Offset { class, offset } => {
            out.put(b"rele");
            class.write(out);
            out.put_i32(*offset);
        }
        ReferenceItem::Identifier { class, id } => {
            out.put(b"Idnt");
            class.write(out);
            out.put_u32(*id);
        }
        ReferenceItem::Index { class, index } => {
            out.put(b"indx");
            class.write(out);
            out.put_u32(*index);
        }
        ReferenceItem::Name { class, name } => {
            out.put(b"name");
            class.write(out);
            name.write(out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn class(name: &str, id: &str) -> Class {
        Class { name: UnicodeString::new(name), class_id: Id::new(id) }
    }

    #[test]
    fn declared_counts_do_not_drive_preallocation() {
        assert!(vec_for_count::<Value>(u32::MAX).capacity() <= MAX_PREALLOC);
        // A 1M-item list of booleans parses (items grow the vector as needed).
        let n = 1_000_000u32;
        let mut data = n.to_be_bytes().to_vec();
        for _ in 0..n {
            data.extend_from_slice(b"bool\x01");
        }
        let mut r = Reader::new(&data);
        match read_typed_value(&mut r, b"VlLs", 0).unwrap() {
            Value::List(v) => assert_eq!(v.len(), n as usize),
            other => panic!("expected list, got {other:?}"),
        }
    }

    pub(crate) fn sample() -> Descriptor {
        Descriptor::new("null")
            .with("Nm  ", Value::Text(UnicodeString::new_nul("Hello \u{1F600}")))
            .with("Opct", Value::UnitFloat { unit: *b"#Prc", value: 75.5 })
            .with("enab", Value::Boolean(true))
            .with("long", Value::Integer(-5))
            .with("comp", Value::LargeInteger(1 << 40))
            .with("doub", Value::Double(0.25))
            .with("Md  ", Value::Enumerated { type_id: Id::new("BlnM"), value: Id::new("Mltp") })
            .with("artboardRect", Value::Descriptor(Descriptor::new("classFloatRect").with("Top ", Value::Double(1.0))))
            .with("list", Value::List(vec![Value::Integer(1), Value::Text(UnicodeString::new("x"))]))
            .with(
                "null",
                Value::Reference(vec![
                    ReferenceItem::Property { class: class("", "Lyr "), key: Id::new("Nm  ") },
                    ReferenceItem::Class(class("", "Dcmn")),
                    ReferenceItem::Enumerated { class: class("", "Lyr "), type_id: Id::new("Ordn"), value: Id::new("Trgt") },
                    ReferenceItem::Offset { class: class("", "Lyr "), offset: -1 },
                    ReferenceItem::Identifier { class: class("", "Lyr "), id: 7 },
                    ReferenceItem::Index { class: class("\0", "Lyr "), index: 2 },
                    ReferenceItem::Name { class: class("", "Lyr "), name: UnicodeString::new("Bg") },
                ]),
            )
            .with("type", Value::Class(class("c", "Clr ")))
            .with("glbc", Value::GlobalClass(class("c", "RGBC")))
            .with("glbo", Value::GlobalObject(Descriptor::default()))
            .with("alis", Value::Alias(vec![1, 2, 3]))
            .with("tdta", Value::RawData(vec![0xde, 0xad]))
            .with("pth ", Value::Path(vec![9, 9]))
            .with(
                "ObAr",
                Value::ObjectArray(ObjectArray {
                    prefix: 16,
                    body: Descriptor::new("rationalPoint").with("Hrzn", Value::UnitFloats { unit: *b"#Pxl", values: vec![1.0, 2.0] }),
                }),
            )
    }

    #[test]
    fn roundtrip_all_types() {
        let d = sample();
        let b = d.to_bytes();
        let p = Descriptor::from_bytes(&b).unwrap();
        assert_eq!(p, d);
        assert_eq!(p.to_bytes(), b);
    }

    /// `warp`, `list` and `time` are written as length-4 strings, as Photoshop writes them, both
    /// as keys and as class ids; other four-character names stay codes. Reading accepts both
    /// forms, and a code read from a file is written back as a code.
    #[test]
    fn photoshop_string_ids_are_written_as_strings() {
        let str_id = |s: &str| [&[0, 0, 0, 4][..], s.as_bytes()].concat();
        let code_id = |s: &str| [&[0, 0, 0, 0][..], s.as_bytes()].concat();
        for name in STRING_IDS {
            assert_eq!(Id::new(name), Id::Str(name.as_bytes().to_vec()));
            let d = Descriptor::new(name).with(name, Value::Integer(1));
            let b = d.to_bytes();
            // Name (empty unicode string), then the class id; then the item count and the key.
            assert_eq!(&b[4..12], &str_id(name)[..], "class {name}");
            assert_eq!(&b[16..24], &str_id(name)[..], "key {name}");
            let back = Descriptor::from_bytes(&b).unwrap();
            assert_eq!(back, d);
            assert!(back.class_id.is(name) && back.get(name).is_some());
            // A file that wrote the code form keeps it, and lookups still find the key.
            let mut coded = b.clone();
            coded.splice(16..24, code_id(name));
            let back = Descriptor::from_bytes(&coded).unwrap();
            assert_eq!(back.get(name), Some(&Value::Integer(1)));
            assert_eq!(back.to_bytes(), coded);
        }
        assert_eq!(Id::new("view"), Id::Code(*b"view"));
        assert_eq!(Id::new("Txt "), Id::Code(*b"Txt "));
    }

    #[test]
    fn versioned_roundtrip() {
        let v = VersionedDescriptor::new(sample());
        let b = v.to_bytes();
        assert_eq!(&b[..4], &[0, 0, 0, 16]);
        assert_eq!(VersionedDescriptor::from_bytes(&b).unwrap(), v);
    }

    #[test]
    fn versioned_parse_prefix() {
        let v = VersionedDescriptor::new(sample());
        let mut b = v.to_bytes();
        let n = b.len();
        b.extend_from_slice(&[1, 2, 3]);
        let (p, used) = VersionedDescriptor::parse_prefix(&b).unwrap();
        assert_eq!(p, v);
        assert_eq!(used, n);
    }

    #[test]
    fn versioned_rejects_other_version() {
        let mut b = VersionedDescriptor::new(Descriptor::new("null")).to_bytes();
        b[3] = 15;
        assert!(VersionedDescriptor::from_bytes(&b).is_err());
    }

    #[test]
    fn id_encoding() {
        let mut v = Vec::new();
        Id::new("Nm  ").write(&mut v);
        assert_eq!(v, [0, 0, 0, 0, b'N', b'm', b' ', b' ']);
        let mut v = Vec::new();
        Id::new("abc").write(&mut v);
        assert_eq!(v, [0, 0, 0, 3, b'a', b'b', b'c']);
        // A 4-byte explicit-length id stays explicit.
        let data = [0, 0, 0, 4, b'a', b'b', b'c', b'd'];
        let id = Id::read(&mut Reader::new(&data)).unwrap();
        assert_eq!(id, Id::Str(b"abcd".to_vec()));
        let mut v = Vec::new();
        id.write(&mut v);
        assert_eq!(v, data);
    }

    #[test]
    fn get_and_lookup() {
        let d = sample();
        assert_eq!(d.get("long"), Some(&Value::Integer(-5)));
        assert!(d.get("nope").is_none());
    }

    #[test]
    fn unicode_lossy() {
        assert_eq!(UnicodeString::new_nul("ab").to_string_lossy(), "ab");
        assert_eq!(UnicodeString(vec![0xd800, 0x41]).to_string_lossy(), "\u{fffd}A");
    }

    #[test]
    fn truncations_error() {
        let b = sample().to_bytes();
        for cut in 0..b.len() {
            assert!(Descriptor::from_bytes(&b[..cut]).is_err(), "cut {cut}");
        }
    }

    #[test]
    fn trailing_bytes_error() {
        let mut b = sample().to_bytes();
        b.push(0);
        assert!(Descriptor::from_bytes(&b).is_err());
    }

    #[test]
    fn unknown_ostype_errors() {
        let mut b = Descriptor::new("null").to_bytes();
        let n = b.len();
        b[n - 1] = 1; // one item
        b.extend_from_slice(&[0, 0, 0, 0, b'k', b'e', b'y', b' ']);
        b.extend_from_slice(b"????");
        assert!(matches!(Descriptor::from_bytes(&b), Err(PsdError::Unsupported(_))));
    }

    #[test]
    fn deep_nesting_is_limited() {
        let mut d = Descriptor::new("null");
        for _ in 0..(MAX_DEPTH + 5) {
            d = Descriptor::new("null").with("in  ", Value::Descriptor(d));
        }
        let b = d.to_bytes();
        assert!(matches!(Descriptor::from_bytes(&b), Err(PsdError::LimitExceeded(_))));
    }

    #[test]
    fn huge_counts_rejected_without_alloc() {
        let mut b = Vec::new();
        b.put_u32(0);
        b.put_u32(0);
        b.put(b"null");
        b.put_u32(u32::MAX);
        assert!(Descriptor::from_bytes(&b).is_err());
    }
}

#[cfg(test)]
pub(crate) mod proptests {
    use super::*;
    use proptest::prelude::*;

    fn arb_id() -> impl Strategy<Value = Id> {
        prop_oneof![any::<[u8; 4]>().prop_map(Id::Code), proptest::collection::vec(any::<u8>(), 1..12).prop_map(Id::Str),]
    }
    fn arb_ustr() -> impl Strategy<Value = UnicodeString> {
        proptest::collection::vec(any::<u16>(), 0..8).prop_map(UnicodeString)
    }
    fn arb_class() -> impl Strategy<Value = Class> {
        (arb_ustr(), arb_id()).prop_map(|(name, class_id)| Class { name, class_id })
    }
    fn arb_ref() -> impl Strategy<Value = ReferenceItem> {
        prop_oneof![
            (arb_class(), arb_id()).prop_map(|(class, key)| ReferenceItem::Property { class, key }),
            arb_class().prop_map(ReferenceItem::Class),
            (arb_class(), arb_id(), arb_id()).prop_map(|(class, type_id, value)| ReferenceItem::Enumerated { class, type_id, value }),
            (arb_class(), any::<i32>()).prop_map(|(class, offset)| ReferenceItem::Offset { class, offset }),
            (arb_class(), any::<u32>()).prop_map(|(class, id)| ReferenceItem::Identifier { class, id }),
            (arb_class(), any::<u32>()).prop_map(|(class, index)| ReferenceItem::Index { class, index }),
            (arb_class(), arb_ustr()).prop_map(|(class, name)| ReferenceItem::Name { class, name }),
        ]
    }
    fn finite() -> impl Strategy<Value = f64> {
        -1e12f64..1e12
    }
    fn arb_leaf() -> impl Strategy<Value = Value> {
        prop_oneof![
            finite().prop_map(Value::Double),
            (any::<[u8; 4]>(), finite()).prop_map(|(unit, value)| Value::UnitFloat { unit, value }),
            (any::<[u8; 4]>(), proptest::collection::vec(finite(), 0..4)).prop_map(|(unit, values)| Value::UnitFloats { unit, values }),
            arb_ustr().prop_map(Value::Text),
            (arb_id(), arb_id()).prop_map(|(type_id, value)| Value::Enumerated { type_id, value }),
            any::<i32>().prop_map(Value::Integer),
            any::<i64>().prop_map(Value::LargeInteger),
            any::<bool>().prop_map(Value::Boolean),
            arb_class().prop_map(Value::Class),
            arb_class().prop_map(Value::GlobalClass),
            proptest::collection::vec(any::<u8>(), 0..8).prop_map(Value::Alias),
            proptest::collection::vec(any::<u8>(), 0..8).prop_map(Value::RawData),
            proptest::collection::vec(any::<u8>(), 0..8).prop_map(Value::Path),
            proptest::collection::vec(arb_ref(), 0..3).prop_map(Value::Reference),
        ]
    }
    pub(crate) fn arb_descriptor() -> impl Strategy<Value = Descriptor> {
        let leaf = arb_leaf();
        let value = leaf.prop_recursive(3, 24, 4, |inner| {
            prop_oneof![
                proptest::collection::vec(inner.clone(), 0..4).prop_map(Value::List),
                (arb_ustr(), arb_id(), proptest::collection::vec((arb_id(), inner.clone()), 0..4))
                    .prop_map(|(name, id, items)| Value::Descriptor(Descriptor { name, class_id: id, items })),
                (arb_ustr(), arb_id(), proptest::collection::vec((arb_id(), inner.clone()), 0..3))
                    .prop_map(|(name, id, items)| Value::GlobalObject(Descriptor { name, class_id: id, items })),
                (any::<u32>(), arb_id(), proptest::collection::vec((arb_id(), inner), 0..3)).prop_map(|(prefix, id, items)| Value::ObjectArray(ObjectArray {
                    prefix,
                    body: Descriptor { name: UnicodeString::default(), class_id: id, items }
                })),
            ]
        });
        (arb_ustr(), arb_id(), proptest::collection::vec((arb_id(), value), 0..6)).prop_map(|(name, id, items)| Descriptor { name, class_id: id, items })
    }

    proptest! {
        #[test]
        fn descriptor_roundtrip(d in arb_descriptor()) {
            let b = d.to_bytes();
            let p = Descriptor::from_bytes(&b).unwrap();
            prop_assert_eq!(&p, &d);
            prop_assert_eq!(p.to_bytes(), b);
        }

        #[test]
        fn descriptor_parse_never_panics(data in proptest::collection::vec(any::<u8>(), 0..256)) {
            let _ = Descriptor::from_bytes(&data);
            let _ = VersionedDescriptor::from_bytes(&data);
        }
    }
}
