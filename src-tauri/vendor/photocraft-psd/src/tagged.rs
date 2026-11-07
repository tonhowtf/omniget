//! Additional layer information ("tagged blocks").
//!
//! Every block is stored raw (signature, key, data and the exact padding that
//! followed it in the file) so unknown and partially-understood blocks
//! round-trip byte for byte. [`TaggedBlock::parsed`] offers typed views of
//! simple, well-known keys.

use crate::blend::BlendMode;
use crate::descriptor::UnicodeString;
use crate::error::{PsdError, Result};
use crate::header::Version;
use crate::io::{Reader, Sink, WriteExt};

/// Keys whose length field is 8 bytes in PSB files: the thirteen the Adobe
/// specification lists, plus keys Photoshop also writes with 8-byte lengths
/// although the specification omits them (observed in real PSB files, e.g.
/// `lnkE` and `cinf` in the ag-psd test set, and expected by other PSB
/// readers such as psd-tools). Reading one of these with a 4-byte length
/// misaligns every block after it (#200).
pub const PSB_LONG_KEYS: [&[u8; 4]; 21] = [
    b"LMsk", b"Lr16", b"Lr32", b"Layr", b"Mt16", b"Mt32", b"Mtrn", b"Alph", b"FMsk", b"lnk2", b"FEid", b"FXid", b"PxSD", // spec
    b"lnk3", b"lnkE", b"pths", b"extd", b"extn", b"FELS", b"cinf", b"artd", // observed
];

/// Returns `true` if `key` uses an 8-byte length in `version`.
pub fn uses_long_length(version: Version, key: &[u8; 4]) -> bool {
    version.is_psb() && PSB_LONG_KEYS.contains(&key)
}

/// A tagged block, stored raw.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TaggedBlock {
    /// `8BIM` or `8B64`.
    pub signature: [u8; 4],
    /// Four-character key.
    pub key: [u8; 4],
    /// Block data exactly as covered by the length field.
    pub data: Vec<u8>,
    /// Bytes that followed the data before the next block. `None` means
    /// "default": zero-pad to a multiple of four for global blocks, two for
    /// layer blocks. Parsing yields `None` whenever
    /// the stored padding equals the default, so the model stays canonical.
    pub padding: Option<Vec<u8>>,
}

/// Layer section divider type (`lsct`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum SectionType {
    /// 0: any other type of layer.
    Other,
    /// 1: open folder.
    OpenFolder,
    /// 2: closed folder.
    ClosedFolder,
    /// 3: bounding section divider (hidden in the UI; marks the group end
    /// in bottom-to-top order).
    BoundingDivider,
    /// Unknown value.
    Unknown(u32),
}

impl SectionType {
    /// From stored value.
    pub fn from_u32(v: u32) -> Self {
        match v {
            0 => SectionType::Other,
            1 => SectionType::OpenFolder,
            2 => SectionType::ClosedFolder,
            3 => SectionType::BoundingDivider,
            v => SectionType::Unknown(v),
        }
    }
    /// Stored value.
    pub fn as_u32(self) -> u32 {
        match self {
            SectionType::Other => 0,
            SectionType::OpenFolder => 1,
            SectionType::ClosedFolder => 2,
            SectionType::BoundingDivider => 3,
            SectionType::Unknown(v) => v,
        }
    }
    /// `true` for open or closed folders.
    pub fn is_folder(self) -> bool {
        matches!(self, SectionType::OpenFolder | SectionType::ClosedFolder)
    }
}

/// Section divider setting (`lsct` / `lsdk`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SectionDivider {
    /// Divider type.
    pub kind: SectionType,
    /// Group blend mode (present if length >= 12).
    pub blend_mode: Option<BlendMode>,
    /// Sub type: 0 normal, 1 scene group (present if length >= 16).
    pub sub_type: Option<u32>,
}

/// Typed view of a known block.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum BlockData<'a> {
    /// `luni`
    UnicodeName(String),
    /// `lsct` / `lsdk`
    SectionDivider(SectionDivider),
    /// `lyid`
    LayerId(u32),
    /// `lnsr`
    NameSource([u8; 4]),
    /// `clbl`
    BlendClippedAsGroup(bool),
    /// `infx`
    BlendInteriorElements(bool),
    /// `knko`: 0 none, 1 shallow, 2 deep.
    Knockout(u8),
    /// `tsly`: Transparency Shapes Layer.
    TransparencyShapes(bool),
    /// `lmgm`: Layer Mask Hides Effects ("layer mask as global mask").
    LayerMaskHidesEffects(bool),
    /// `vmgm`: Vector Mask Hides Effects.
    VectorMaskHidesEffects(bool),
    /// `lspf`: bit 0 transparency, bit 1 composite, bit 2 position locked.
    Protection(u32),
    /// `lclr`: sheet color index (0 none, 1 red, 2 orange, 3 yellow, 4 green,
    /// 5 blue, 6 violet, 7 gray).
    SheetColor(u16),
    /// `iOpa`
    FillOpacity(u8),
    /// `shmd`: metadata setting, raw.
    MetadataSetting(&'a [u8]),
}

impl TaggedBlock {
    /// A new `8BIM` block with default padding.
    pub fn new(key: [u8; 4], data: Vec<u8>) -> Self {
        TaggedBlock { signature: *b"8BIM", key, data, padding: None }
    }

    /// `true` for `MIB8` / `46B8`: the signatures [`crate::tiff`] gives a block whose data was
    /// kept verbatim from a little-endian TIFF because its layout is unknown. Such a block is
    /// written back exactly into a little-endian TIFF, and cannot go into a big-endian file
    /// (a PSD) without being dropped: see [`Self::is_foreign_order`].
    pub fn is_foreign_order_signature(b: &[u8]) -> bool {
        b.starts_with(b"MIB8") || b.starts_with(b"46B8")
    }

    /// Whether this block's data is in little-endian layout (see
    /// [`Self::is_foreign_order_signature`]).
    pub fn is_foreign_order(&self) -> bool {
        Self::is_foreign_order_signature(&self.signature)
    }

    /// `true` for a "Saving Merged Transparency" marker: the merged image's first extra channel
    /// is its transparency. The specification's keys are `Mtrn`, `Mt16` and `Mt32`; Photoshop
    /// (27.11) writes `MTrn`, with a positive layer count, in a layered TIFF saved with Save
    /// Transparency next to spot and alpha channels.
    pub fn is_merged_transparency_marker(&self) -> bool {
        matches!(&self.key, b"Mtrn" | b"MTrn" | b"Mt16" | b"Mt32")
    }

    /// Key as a string (lossy).
    pub fn key_str(&self) -> String {
        String::from_utf8_lossy(&self.key).into_owned()
    }

    pub(crate) fn default_padding(len: usize, alignment: usize) -> usize {
        (alignment - len % alignment) % alignment
    }

    /// Typed view for known keys. `None` for keys without a typed parser,
    /// `Some(Err)` if a known key's data is malformed.
    pub fn parsed(&self) -> Option<Result<BlockData<'_>>> {
        let d = &self.data[..];
        let mut r = Reader::new(d);
        Some(match &self.key {
            b"luni" => UnicodeString::read(&mut r).map(|s| BlockData::UnicodeName(s.to_string_lossy())),
            b"lsct" | b"lsdk" => parse_section(d).map(BlockData::SectionDivider),
            b"lyid" => r.u32().map(BlockData::LayerId),
            b"lnsr" => r.array().map(BlockData::NameSource),
            b"clbl" => r.u8().map(|v| BlockData::BlendClippedAsGroup(v != 0)),
            b"infx" => r.u8().map(|v| BlockData::BlendInteriorElements(v != 0)),
            b"knko" => r.u8().map(BlockData::Knockout),
            b"tsly" => r.u8().map(|v| BlockData::TransparencyShapes(v != 0)),
            b"lmgm" => r.u8().map(|v| BlockData::LayerMaskHidesEffects(v != 0)),
            b"vmgm" => r.u8().map(|v| BlockData::VectorMaskHidesEffects(v != 0)),
            b"lspf" => r.u32().map(BlockData::Protection),
            b"lclr" => r.u16().map(BlockData::SheetColor),
            b"iOpa" => r.u8().map(BlockData::FillOpacity),
            b"shmd" => Ok(BlockData::MetadataSetting(d)),
            _ => return None,
        })
    }

    /// `luni` block. Data is padded to a multiple of 4, as Photoshop does.
    pub fn unicode_name(name: &str) -> Self {
        let mut d = Vec::new();
        UnicodeString::new(name).write(&mut d);
        while d.len() % 4 != 0 {
            d.push(0);
        }
        Self::new(*b"luni", d)
    }

    /// `lsct` block with all fields.
    pub fn section_divider(kind: SectionType, blend_mode: Option<BlendMode>, sub_type: Option<u32>) -> Self {
        let mut d = Vec::new();
        d.put_u32(kind.as_u32());
        if let Some(b) = blend_mode {
            d.put(b"8BIM");
            d.put(&b.key());
            if let Some(s) = sub_type {
                d.put_u32(s);
            }
        }
        Self::new(*b"lsct", d)
    }

    /// `lyid` block.
    pub fn layer_id(id: u32) -> Self {
        Self::new(*b"lyid", id.to_be_bytes().to_vec())
    }
    /// `lnsr` block.
    pub fn name_source(src: [u8; 4]) -> Self {
        Self::new(*b"lnsr", src.to_vec())
    }
    fn flag(key: [u8; 4], v: u8) -> Self {
        Self::new(key, vec![v, 0, 0, 0])
    }
    /// `clbl` block.
    pub fn blend_clipped_as_group(v: bool) -> Self {
        Self::flag(*b"clbl", u8::from(v))
    }
    /// `infx` block.
    pub fn blend_interior_elements(v: bool) -> Self {
        Self::flag(*b"infx", u8::from(v))
    }
    /// `knko` block.
    pub fn knockout(v: u8) -> Self {
        Self::flag(*b"knko", v)
    }
    /// `tsly` block.
    pub fn transparency_shapes(v: bool) -> Self {
        Self::flag(*b"tsly", u8::from(v))
    }
    /// `lmgm` block.
    pub fn layer_mask_hides_effects(v: bool) -> Self {
        Self::flag(*b"lmgm", u8::from(v))
    }
    /// `vmgm` block.
    pub fn vector_mask_hides_effects(v: bool) -> Self {
        Self::flag(*b"vmgm", u8::from(v))
    }
    /// `lspf` block.
    pub fn protection(flags: u32) -> Self {
        Self::new(*b"lspf", flags.to_be_bytes().to_vec())
    }
    /// `lclr` block.
    pub fn sheet_color(color: u16) -> Self {
        let mut d = color.to_be_bytes().to_vec();
        d.extend_from_slice(&[0; 6]);
        Self::new(*b"lclr", d)
    }
    /// `iOpa` block.
    pub fn fill_opacity(v: u8) -> Self {
        Self::flag(*b"iOpa", v)
    }

    /// Strictly re-parses the internal structure of blocks whose data carries
    /// its own sizes, so a block that a strict reader would reject is caught
    /// here: `PlLd` (a `plcL` placed layer), `SoLd`/`SoLE` (`soLD` placed
    /// layer data), `lnk2`/`lnk3`/`lnkD` (length-prefixed linked files) and
    /// `lfx2` (object effects). Every inner length must stay inside the block
    /// and the parse must end at the end of the data, apart from up to three
    /// zero padding bytes. Other keys are not checked and return `Ok`.
    ///
    /// Layouts follow the Adobe Photoshop File Format specification
    /// ("Additional Layer Information": Placed Layer, Placed Layer Data,
    /// Linked Layer, Object Based Effects Layer Info).
    pub fn check_structure(&self) -> Result<()> {
        let d = &self.data[..];
        let mut r = Reader::new(d);
        match &self.key {
            b"PlLd" => {
                expect_kind(&mut r, b"plcL")?;
                let _version = r.u32()?;
                crate::io::read_pascal(&mut r, 1)?;
                // Page, total pages, anti-alias policy, layer type; transform (8 doubles).
                r.skip(4 * 4 + 8 * 8)?;
                let _warp_version = r.u32()?;
                crate::descriptor::VersionedDescriptor::read(&mut r)?;
            }
            b"SoLd" | b"SoLE" => {
                expect_kind(&mut r, b"soLD")?;
                let _version = r.u32()?;
                let d = crate::descriptor::VersionedDescriptor::read(&mut r)?;
                check_placed_descriptor(&d.descriptor).map_err(|e| PsdError::invalid(format!("{}: {e}", self.key_str())))?;
            }
            b"lfx2" => {
                let _effects_version = r.u32()?;
                crate::descriptor::VersionedDescriptor::read(&mut r)?;
            }
            b"lnk2" | b"lnk3" | b"lnkD" => {
                while r.remaining() >= 8 {
                    let len = r.u64()?;
                    let item = r.bytes_u64(len)?;
                    if !matches!(item.get(..4), Some(b"liFD" | b"liFE" | b"liFA")) {
                        return Err(PsdError::invalid(format!("{}: linked item of unknown type", self.key_str())));
                    }
                    if item.get(..4) == Some(b"liFD") {
                        check_embedded_item(item).map_err(|e| PsdError::invalid(format!("{}: embedded file: {e}", self.key_str())))?;
                    }
                    // Items are padded to a multiple of 4.
                    let pad = ((4 - len % 4) % 4).min(r.remaining() as u64);
                    r.skip(pad as usize)?;
                }
            }
            b"FEid" | b"FXid" => {
                crate::filter_effects::FilterEffects::parse(d).map_err(|e| PsdError::invalid(format!("{}: {e}", self.key_str())))?;
                return Ok(());
            }
            _ => return Ok(()),
        }
        let rest = r.peek_rest();
        if rest.len() > 3 || rest.iter().any(|&b| b != 0) {
            return Err(PsdError::invalid(format!("{}: {} unexpected bytes after the block structure", self.key_str(), rest.len())));
        }
        Ok(())
    }

    pub(crate) fn write(&self, out: &mut Vec<u8>, version: Version, alignment: usize) -> Result<()> {
        Self::write_parts(out, self.signature, self.key, &self.data, self.padding.as_deref(), version, alignment)
    }

    /// Writes a block from its parts (no owned [`TaggedBlock`] needed). `padding: None` writes
    /// zeros up to `alignment`.
    pub(crate) fn write_parts(
        out: &mut Vec<u8>,
        signature: [u8; 4],
        key: [u8; 4],
        data: &[u8],
        padding: Option<&[u8]>,
        version: Version,
        alignment: usize,
    ) -> Result<()> {
        out.put(&signature);
        out.put(&key);
        out.put_len(data.len() as u64, uses_long_length(version, &key))?;
        out.put(data);
        match padding {
            Some(p) => out.put(p),
            None => out.extend(std::iter::repeat_n(0u8, Self::default_padding(data.len(), alignment))),
        }
        Ok(())
    }

    /// Serialized size in bytes as a layer block (two-byte alignment).
    pub fn encoded_len(&self, version: Version) -> usize {
        self.encoded_len_aligned(version, 2)
    }

    /// Serialized size in bytes as [`Self::write`] writes it with `alignment`.
    pub(crate) fn encoded_len_aligned(&self, version: Version, alignment: usize) -> usize {
        let l = if uses_long_length(version, &self.key) { 8 } else { 4 };
        let p = self.padding.as_ref().map_or(Self::default_padding(self.data.len(), alignment), Vec::len);
        (8 + l + p).saturating_add(self.data.len())
    }
}

/// Strict check of a `soLD` descriptor: the unique id and transform Photoshop needs to place the
/// layer, and a well-formed smart filter stack (`filterFX`: a `filterFXList` of `filterFX`
/// objects, each with a name, blend options, an enabled flag and a filter id).
fn check_placed_descriptor(d: &crate::descriptor::Descriptor) -> Result<()> {
    use crate::descriptor::Value;
    if !matches!(d.get("Idnt"), Some(Value::Text(_))) {
        return Err(PsdError::invalid("no Idnt (unique id)"));
    }
    match d.get("Trnf") {
        Some(Value::List(l)) if l.len() == 8 && l.iter().all(|v| matches!(v, Value::Double(_))) => {}
        _ => return Err(PsdError::invalid("Trnf is not eight doubles")),
    }
    let Some(fx) = d.get("filterFX") else { return Ok(()) };
    let Value::Descriptor(fx) = fx else { return Err(PsdError::invalid("filterFX is not an object")) };
    let Some(Value::List(list)) = fx.get("filterFXList") else { return Err(PsdError::invalid("filterFX has no filterFXList")) };
    for (i, item) in list.iter().enumerate() {
        let Value::Descriptor(f) = item else { return Err(PsdError::invalid(format!("filter {i} is not an object"))) };
        let ok = matches!(f.get("Nm  "), Some(Value::Text(_)))
            && matches!(f.get("blendOptions"), Some(Value::Descriptor(_)))
            && matches!(f.get("enab"), Some(Value::Boolean(_)))
            && matches!(f.get("filterID"), Some(Value::Integer(_)))
            && f.get("Fltr").is_none_or(|v| matches!(v, Value::Descriptor(_)));
        if !ok {
            return Err(PsdError::invalid(format!("filter {i} lacks Nm/blendOptions/enab/filterID")));
        }
    }
    Ok(())
}

/// Strict check of one embedded (`liFD`) linked-layer item: type, version, unique id, file name,
/// file type and creator, data length, optional open descriptor, then the file data, which must
/// fit in the item (version 5+ fields follow it).
fn check_embedded_item(item: &[u8]) -> Result<()> {
    let mut r = Reader::new(item);
    r.skip(4)?;
    let version = r.u32()?;
    if !(1..=16).contains(&version) {
        return Err(PsdError::invalid(format!("version {version}")));
    }
    crate::io::read_pascal(&mut r, 1)?;
    crate::io::read_unicode_units(&mut r)?;
    r.skip(8)?; // file type, creator
    let len = r.u64()?;
    if r.u8()? != 0 {
        crate::descriptor::VersionedDescriptor::read(&mut r)?;
    }
    r.bytes_u64(len)?;
    Ok(())
}

fn expect_kind(r: &mut Reader<'_>, kind: &'static [u8; 4]) -> Result<()> {
    let found = r.array::<4>()?;
    if &found != kind {
        return Err(PsdError::invalid(format!("expected {} structure, found {:?}", String::from_utf8_lossy(kind), String::from_utf8_lossy(&found))));
    }
    Ok(())
}

fn parse_section(d: &[u8]) -> Result<SectionDivider> {
    let mut r = Reader::new(d);
    let kind = SectionType::from_u32(r.u32()?);
    let mut blend_mode = None;
    let mut sub_type = None;
    if r.remaining() >= 8 {
        let sig = r.array::<4>()?;
        if &sig != b"8BIM" {
            return Err(PsdError::InvalidSignature { expected: "8BIM", found: sig });
        }
        blend_mode = Some(BlendMode::from_key(r.array()?));
        if r.remaining() >= 4 {
            sub_type = Some(r.u32()?);
        }
    }
    Ok(SectionDivider { kind, blend_mode, sub_type })
}

/// `8BIM` / `8B64`, or the markers `MIB8` / `46B8` that [`crate::tiff`] gives a block kept
/// verbatim from a little-endian TIFF when its layout is unknown (its data is in that order).
pub(crate) fn is_sig(b: &[u8]) -> bool {
    b.starts_with(b"8BIM") || b.starts_with(b"8B64") || TaggedBlock::is_foreign_order_signature(b)
}

/// Reads tagged blocks until the reader is exhausted. Bytes that do not form
/// a block (fewer than 12 bytes or no valid signature) are returned as
/// trailing data.
pub(crate) fn read_blocks(r: &mut Reader<'_>, version: Version) -> Result<(Vec<TaggedBlock>, Vec<u8>)> {
    read_blocks_with(r, version, 2, |_, _, _, _, _| false)
}

/// The callback consumes selected payloads before they are copied into owned blocks.
pub(crate) fn read_blocks_with(
    r: &mut Reader<'_>,
    version: Version,
    alignment: usize,
    mut consume: impl FnMut(usize, [u8; 4], [u8; 4], &[u8], &Option<Vec<u8>>) -> bool,
) -> Result<(Vec<TaggedBlock>, Vec<u8>)> {
    let mut blocks = Vec::new();
    loop {
        let rest = r.peek_rest();
        if rest.len() < 12 || !is_sig(rest) {
            let trailing = rest.to_vec();
            r.skip(rest.len())?;
            return Ok((blocks, trailing));
        }
        let signature = r.array::<4>()?;
        let key = r.array::<4>()?;
        let len = r.len_field(uses_long_length(version, &key))?;
        let data = r.bytes_u64(len)?;
        // Detect padding: the smallest k in 0..=3 such that the next block
        // signature (or the end of the region) follows k bytes later.
        let rest = r.peek_rest();
        let mut pad = None;
        for k in 0..=3usize {
            if k == rest.len() || (k < rest.len() && is_sig(&rest[k..])) {
                pad = Some(k);
                break;
            }
        }
        // Opaque trailing bytes can follow canonical zero padding without another signature.
        let default = TaggedBlock::default_padding(data.len(), alignment);
        let pad = pad.unwrap_or_else(|| if rest.get(..default).is_some_and(|p| p.iter().all(|&b| b == 0)) { default } else { 0 });
        let pad_bytes = r.bytes(pad)?.to_vec();
        let padding =
            if pad_bytes.len() == TaggedBlock::default_padding(data.len(), alignment) && pad_bytes.iter().all(|&b| b == 0) { None } else { Some(pad_bytes) };
        if !consume(blocks.len(), signature, key, data, &padding) {
            blocks.push(TaggedBlock { signature, key, data: data.to_vec(), padding });
        }
    }
}

pub(crate) fn write_blocks(out: &mut Vec<u8>, blocks: &[TaggedBlock], version: Version, alignment: usize) -> Result<()> {
    for b in blocks {
        b.write(out, version, alignment)?;
    }
    Ok(())
}

/// [`write_blocks`] to a streaming [`Sink`]: each block's data goes straight through, so a large
/// opaque block (an embedded smart object) is never copied whole.
pub(crate) fn write_blocks_to<W: std::io::Write + std::io::Seek + ?Sized>(
    out: &mut Sink<'_, W>,
    blocks: &[TaggedBlock],
    version: Version,
    alignment: usize,
) -> std::io::Result<()> {
    for b in blocks {
        out.small(|o| {
            o.put(&b.signature);
            o.put(&b.key);
            o.put_len(b.data.len() as u64, uses_long_length(version, &b.key))
        })?;
        out.put(&b.data)?;
        match &b.padding {
            Some(p) => out.put(p)?,
            None => out.zeros(TaggedBlock::default_padding(b.data.len(), alignment))?,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt(blocks: &[TaggedBlock], version: Version) -> Vec<u8> {
        let mut out = Vec::new();
        write_blocks(&mut out, blocks, version, 2).unwrap();
        let (back, trailing) = read_blocks(&mut Reader::new(&out), version).unwrap();
        assert_eq!(back, blocks);
        assert!(trailing.is_empty());
        out
    }

    #[test]
    fn unicode_name_typed() {
        let b = TaggedBlock::unicode_name("Ebene \u{e4}\u{1F600}");
        assert_eq!(b.data.len() % 4, 0);
        assert_eq!(b.parsed(), Some(Ok(BlockData::UnicodeName("Ebene \u{e4}\u{1F600}".into()))));
        rt(&[b], Version::Psd);
    }

    #[test]
    fn section_divider_variants() {
        for (kind, bm, st, len) in [
            (SectionType::OpenFolder, None, None, 4),
            (SectionType::ClosedFolder, Some(BlendMode::PassThrough), None, 12),
            (SectionType::BoundingDivider, Some(BlendMode::Multiply), Some(1), 16),
        ] {
            let b = TaggedBlock::section_divider(kind, bm, st);
            assert_eq!(b.data.len(), len);
            match b.parsed() {
                Some(Ok(BlockData::SectionDivider(s))) => {
                    assert_eq!(s.kind, kind);
                    assert_eq!(s.blend_mode, bm);
                    assert_eq!(s.sub_type, st);
                }
                other => panic!("{other:?}"),
            }
        }
        let mut b = TaggedBlock::section_divider(SectionType::OpenFolder, Some(BlendMode::Normal), None);
        b.data[4] = b'X';
        assert!(matches!(b.parsed(), Some(Err(_))));
        let mut lsdk = TaggedBlock::section_divider(SectionType::OpenFolder, None, None);
        lsdk.key = *b"lsdk";
        assert!(matches!(lsdk.parsed(), Some(Ok(BlockData::SectionDivider(_)))));
    }

    #[test]
    fn section_type_values() {
        for v in 0..6 {
            assert_eq!(SectionType::from_u32(v).as_u32(), v);
        }
        assert!(SectionType::OpenFolder.is_folder());
        assert!(!SectionType::BoundingDivider.is_folder());
    }

    #[test]
    fn simple_typed_blocks() {
        assert_eq!(TaggedBlock::layer_id(7).parsed(), Some(Ok(BlockData::LayerId(7))));
        assert_eq!(TaggedBlock::name_source(*b"cont").parsed(), Some(Ok(BlockData::NameSource(*b"cont"))));
        assert_eq!(TaggedBlock::blend_clipped_as_group(true).parsed(), Some(Ok(BlockData::BlendClippedAsGroup(true))));
        assert_eq!(TaggedBlock::blend_interior_elements(false).parsed(), Some(Ok(BlockData::BlendInteriorElements(false))));
        assert_eq!(TaggedBlock::knockout(2).parsed(), Some(Ok(BlockData::Knockout(2))));
        assert_eq!(TaggedBlock::transparency_shapes(false).parsed(), Some(Ok(BlockData::TransparencyShapes(false))));
        assert_eq!(TaggedBlock::layer_mask_hides_effects(true).parsed(), Some(Ok(BlockData::LayerMaskHidesEffects(true))));
        assert_eq!(TaggedBlock::vector_mask_hides_effects(true).parsed(), Some(Ok(BlockData::VectorMaskHidesEffects(true))));
        assert_eq!(TaggedBlock::vector_mask_hides_effects(true).data, vec![1, 0, 0, 0]);
        assert!(matches!(TaggedBlock::new(*b"tsly", vec![]).parsed(), Some(Err(_))), "truncated flag is an error, not a panic");
        assert_eq!(TaggedBlock::protection(0x8000_0000).parsed(), Some(Ok(BlockData::Protection(0x8000_0000))));
        assert_eq!(TaggedBlock::sheet_color(3).parsed(), Some(Ok(BlockData::SheetColor(3))));
        assert_eq!(TaggedBlock::fill_opacity(128).parsed(), Some(Ok(BlockData::FillOpacity(128))));
        let shmd = TaggedBlock::new(*b"shmd", vec![0, 0, 0, 0]);
        assert_eq!(shmd.parsed(), Some(Ok(BlockData::MetadataSetting(&[0, 0, 0, 0]))));
        assert_eq!(TaggedBlock::new(*b"lfx2", vec![]).parsed(), None);
        assert!(matches!(TaggedBlock::new(*b"lyid", vec![1]).parsed(), Some(Err(_))));
    }

    #[test]
    fn default_padding_odd_data() {
        let b = TaggedBlock::new(*b"abcd", vec![1, 2, 3]);
        let out = rt(&[b.clone(), b], Version::Psd);
        assert_eq!(out.len(), 2 * (12 + 4));
    }

    #[test]
    fn explicit_padding_preserved() {
        let mut a = TaggedBlock::new(*b"abcd", vec![1, 2, 3]);
        a.padding = Some(vec![0; 3]); // pad-to-4 in a file that aligned to 4
        let mut b = TaggedBlock::new(*b"efgh", vec![1, 2]);
        b.padding = Some(vec![0, 0]);
        let mut c = TaggedBlock::new(*b"ijkl", vec![1]);
        c.padding = Some(vec![]); // no padding at all
        rt(&[a, b, c], Version::Psd);
    }

    #[test]
    fn signature_8b64_preserved() {
        let mut b = TaggedBlock::new(*b"Lr16", vec![0; 6]);
        b.signature = *b"8B64";
        rt(&[b], Version::Psb);
    }

    #[test]
    fn psb_long_keys() {
        for key in PSB_LONG_KEYS {
            assert!(uses_long_length(Version::Psb, key));
            assert!(!uses_long_length(Version::Psd, key));
            let b = TaggedBlock::new(*key, vec![7; 4]);
            let out = rt(std::slice::from_ref(&b), Version::Psb);
            assert_eq!(out.len(), 8 + 8 + 4);
            assert_eq!(b.encoded_len(Version::Psb), out.len());
            let out = rt(std::slice::from_ref(&b), Version::Psd);
            assert_eq!(out.len(), 8 + 4 + 4);
        }
        assert!(!uses_long_length(Version::Psb, b"luni"));
    }

    #[test]
    fn trailing_bytes_kept() {
        let mut out = Vec::new();
        write_blocks(&mut out, &[TaggedBlock::layer_id(1)], Version::Psd, 2).unwrap();
        out.extend_from_slice(&[0; 8]);
        let (blocks, trailing) = read_blocks(&mut Reader::new(&out), Version::Psd).unwrap();
        assert_eq!(blocks.len(), 1);
        // The detector attributes up to 3 bytes of zeros to padding only when
        // they reach the end; 8 zeros are not padding.
        assert_eq!(trailing, vec![0; 8]);
    }

    #[test]
    fn short_trailing_pad_attributed_to_block() {
        let mut out = Vec::new();
        write_blocks(&mut out, &[TaggedBlock::layer_id(1)], Version::Psd, 2).unwrap();
        out.extend_from_slice(&[0; 2]);
        let (blocks, trailing) = read_blocks(&mut Reader::new(&out), Version::Psd).unwrap();
        assert_eq!(blocks[0].padding, Some(vec![0, 0]));
        assert!(trailing.is_empty());
    }

    #[test]
    fn overlong_length_errors() {
        let mut out = Vec::new();
        out.put(b"8BIMabcd");
        out.put_u32(100);
        out.put(&[0; 4]);
        assert!(read_blocks(&mut Reader::new(&out), Version::Psd).is_err());
    }
}
