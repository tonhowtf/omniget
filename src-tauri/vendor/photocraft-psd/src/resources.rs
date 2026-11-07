//! Image resources section (`8BIM` resource blocks).

use crate::descriptor::UnicodeString;
use crate::error::{PsdError, Result};
use crate::io::{Reader, WriteExt, read_pascal, write_pascal};

/// Well-known resource ids.
pub mod ids {
    /// Resolution info (`ResolutionInfo` structure).
    pub const RESOLUTION_INFO: u16 = 1005;
    /// Target layer index.
    pub const LAYER_STATE: u16 = 1024;
    /// Layer group ids (one u16 per layer).
    pub const LAYER_GROUP_INFO: u16 = 1026;
    /// Legacy (Photoshop 4) thumbnail, BGR.
    pub const THUMBNAIL_PS4: u16 = 1033;
    /// Thumbnail (JFIF).
    pub const THUMBNAIL: u16 = 1036;
    /// Global lighting angle.
    pub const GLOBAL_ANGLE: u16 = 1037;
    /// ICC profile.
    pub const ICC_PROFILE: u16 = 1039;
    /// Global altitude.
    pub const GLOBAL_ALTITUDE: u16 = 1049;
    /// Version info (has real merged data flag).
    pub const VERSION_INFO: u16 = 1057;
    /// EXIF data 1.
    pub const EXIF: u16 = 1058;
    /// XMP metadata.
    pub const XMP: u16 = 1060;
}

/// One image resource block.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ImageResource {
    /// Signature, normally `8BIM` (also `MeSa`, `AgHg`, `PHUT`, `DCSR` seen in the wild).
    pub signature: [u8; 4],
    /// Resource id.
    pub id: u16,
    /// Pascal name bytes (usually empty).
    pub name: Vec<u8>,
    /// Resource data (without the even padding).
    pub data: Vec<u8>,
}

/// Fixed-point resolution info (resource 1005).
///
/// `h_res_fixed` and `v_res_fixed` are always pixels per inch (16.16 fixed).
/// `h_res_unit` and `v_res_unit` only say how to display that number
/// (1 = pixels/inch, 2 = pixels/cm) and do not rescale the stored value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ResolutionInfo {
    /// Horizontal resolution, 16.16 fixed point, always pixels per inch.
    pub h_res_fixed: u32,
    /// Display unit for the horizontal resolution (1 = px/in, 2 = px/cm).
    pub h_res_unit: u16,
    /// Ruler unit for width (1 in, 2 cm, 3 pt, 4 pica, 5 column).
    pub width_unit: u16,
    /// Vertical resolution, 16.16 fixed point, always pixels per inch.
    pub v_res_fixed: u32,
    /// Display unit for the vertical resolution (1 = px/in, 2 = px/cm).
    pub v_res_unit: u16,
    /// Ruler unit for height (1 in, 2 cm, 3 pt, 4 pica, 5 column).
    pub height_unit: u16,
}

impl ResolutionInfo {
    /// Resolution info for `dpi` pixels per inch in both directions.
    pub fn from_dpi(dpi: f64) -> Self {
        let f = (dpi * 65536.0).round().clamp(0.0, u32::MAX as f64) as u32;
        ResolutionInfo { h_res_fixed: f, h_res_unit: 1, width_unit: 1, v_res_fixed: f, v_res_unit: 1, height_unit: 1 }
    }
    /// Horizontal resolution in pixels per inch.
    pub fn h_res(&self) -> f64 {
        f64::from(self.h_res_fixed) / 65536.0
    }
    /// Vertical resolution in pixels per inch.
    pub fn v_res(&self) -> f64 {
        f64::from(self.v_res_fixed) / 65536.0
    }
    /// Parses 16 bytes.
    pub fn from_bytes(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b);
        Ok(ResolutionInfo {
            h_res_fixed: r.u32()?,
            h_res_unit: r.u16()?,
            width_unit: r.u16()?,
            v_res_fixed: r.u32()?,
            v_res_unit: r.u16()?,
            height_unit: r.u16()?,
        })
    }
    /// Serializes to 16 bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(16);
        v.put_u32(self.h_res_fixed);
        v.put_u16(self.h_res_unit);
        v.put_u16(self.width_unit);
        v.put_u32(self.v_res_fixed);
        v.put_u16(self.v_res_unit);
        v.put_u16(self.height_unit);
        v
    }
}

/// Version info (resource 1057).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct VersionInfo {
    /// Version (1).
    pub version: u32,
    /// `false` when the merged image is a placeholder.
    pub has_real_merged_data: bool,
    /// Writer application name.
    pub writer: UnicodeString,
    /// Reader application name.
    pub reader: UnicodeString,
    /// File version.
    pub file_version: u32,
}

impl VersionInfo {
    /// Parses resource data.
    pub fn from_bytes(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b);
        Ok(VersionInfo {
            version: r.u32()?,
            has_real_merged_data: r.u8()? != 0,
            writer: UnicodeString::read(&mut r)?,
            reader: UnicodeString::read(&mut r)?,
            file_version: r.u32()?,
        })
    }
    /// Serializes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = Vec::new();
        v.put_u32(self.version);
        v.put_u8(u8::from(self.has_real_merged_data));
        self.writer.write(&mut v);
        self.reader.write(&mut v);
        v.put_u32(self.file_version);
        v
    }
}

/// Typed view of a known resource.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ResourceData<'a> {
    /// 1005
    ResolutionInfo(ResolutionInfo),
    /// 1024: index of the target layer.
    LayerState(u16),
    /// 1026: group id per layer.
    LayerGroupInfo(Vec<u16>),
    /// 1036 / 1033: raw thumbnail resource (28-byte header + JFIF/raw data).
    Thumbnail(&'a [u8]),
    /// 1037: global lighting angle in degrees.
    GlobalAngle(i32),
    /// 1039: ICC profile bytes.
    IccProfile(&'a [u8]),
    /// 1049: global altitude.
    GlobalAltitude(i32),
    /// 1057
    VersionInfo(VersionInfo),
    /// 1058: EXIF bytes.
    Exif(&'a [u8]),
    /// 1060: XMP packet (UTF-8).
    Xmp(&'a [u8]),
}

impl ImageResource {
    /// A new `8BIM` resource with an empty name.
    pub fn new(id: u16, data: Vec<u8>) -> Self {
        ImageResource { signature: *b"8BIM", id, name: Vec::new(), data }
    }

    /// Typed parse of well-known resources. Returns `None` for unknown ids,
    /// `Some(Err)` if the data is malformed.
    pub fn parsed(&self) -> Option<Result<ResourceData<'_>>> {
        let d = &self.data[..];
        let be_i32 = |d: &[u8]| -> Result<i32> { Reader::new(d).i32() };
        Some(match self.id {
            ids::RESOLUTION_INFO => ResolutionInfo::from_bytes(d).map(ResourceData::ResolutionInfo),
            ids::LAYER_STATE => Reader::new(d).u16().map(ResourceData::LayerState),
            ids::LAYER_GROUP_INFO => {
                if !d.len().is_multiple_of(2) {
                    Err(PsdError::invalid("layer group info has odd length"))
                } else {
                    Ok(ResourceData::LayerGroupInfo(d.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes([c[0], c[1]])).collect()))
                }
            }
            ids::THUMBNAIL | ids::THUMBNAIL_PS4 => Ok(ResourceData::Thumbnail(d)),
            ids::GLOBAL_ANGLE => be_i32(d).map(ResourceData::GlobalAngle),
            ids::ICC_PROFILE => Ok(ResourceData::IccProfile(d)),
            ids::GLOBAL_ALTITUDE => be_i32(d).map(ResourceData::GlobalAltitude),
            ids::VERSION_INFO => VersionInfo::from_bytes(d).map(ResourceData::VersionInfo),
            ids::EXIF => Ok(ResourceData::Exif(d)),
            ids::XMP => Ok(ResourceData::Xmp(d)),
            _ => return None,
        })
    }

    /// XMP packet as a string (resource 1060).
    pub fn xmp_str(&self) -> Option<&str> {
        (self.id == ids::XMP).then(|| std::str::from_utf8(&self.data).ok()).flatten()
    }

    pub(crate) fn read(r: &mut Reader<'_>) -> Result<Self> {
        let signature = r.array::<4>()?;
        if !matches!(&signature, b"8BIM" | b"MeSa" | b"AgHg" | b"PHUT" | b"DCSR") {
            return Err(PsdError::InvalidSignature { expected: "8BIM resource", found: signature });
        }
        let id = r.u16()?;
        let name = read_pascal(r, 2)?;
        let len = r.u32()?;
        let data = r.bytes_u64(u64::from(len))?.to_vec();
        if len % 2 == 1 {
            r.skip(1)?;
        }
        Ok(ImageResource { signature, id, name, data })
    }

    pub(crate) fn write(&self, out: &mut Vec<u8>) -> Result<()> {
        out.put(&self.signature);
        out.put_u16(self.id);
        write_pascal(out, &self.name, 2);
        out.put_len(self.data.len() as u64, false)?;
        out.put(&self.data);
        if self.data.len() % 2 == 1 {
            out.put_u8(0);
        }
        Ok(())
    }
}

pub(crate) fn read_section(r: &mut Reader<'_>) -> Result<Vec<ImageResource>> {
    let len = r.u32()?;
    let mut s = r.sub(u64::from(len))?;
    let mut v = Vec::new();
    while !s.is_empty() {
        v.push(ImageResource::read(&mut s)?);
    }
    Ok(v)
}

/// Serialized size of the resource section [`write_section`] writes, its length field included.
pub(crate) fn section_len(res: &[ImageResource]) -> u64 {
    res.iter().fold(4u64, |n, r| {
        let name = 1 + r.name.len().min(255);
        let name = name + name % 2;
        let data = r.data.len().saturating_add(r.data.len() % 2);
        n.saturating_add((4 + 2 + name + 4) as u64).saturating_add(data as u64)
    })
}

pub(crate) fn write_section(out: &mut Vec<u8>, res: &[ImageResource]) -> Result<()> {
    let at = out.begin_len(false);
    for r in res {
        r.write(out)?;
    }
    out.end_len(at, false)
}

/// Builds the data for a 1057 version info resource, with the writer and reader names Photoshop
/// 2026 writes ("Adobe Photoshop", "Adobe Photoshop 2026"), without a NUL terminator.
pub fn version_info_resource(has_real_merged_data: bool) -> ImageResource {
    ImageResource::new(
        ids::VERSION_INFO,
        VersionInfo {
            version: 1,
            has_real_merged_data,
            writer: UnicodeString::new("Adobe Photoshop"),
            reader: UnicodeString::new("Adobe Photoshop 2026"),
            file_version: 1,
        }
        .to_bytes(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(res: &[ImageResource]) -> Vec<u8> {
        let mut out = Vec::new();
        write_section(&mut out, res).unwrap();
        let back = read_section(&mut Reader::new(&out)).unwrap();
        assert_eq!(back, res);
        out
    }

    #[test]
    fn empty_section() {
        assert_eq!(roundtrip(&[]), vec![0, 0, 0, 0]);
    }

    #[test]
    fn odd_data_is_padded() {
        let out = roundtrip(&[ImageResource::new(1000, vec![1, 2, 3])]);
        // len(4) + sig(4) + id(2) + name(2) + size(4) + data(3) + pad(1)
        assert_eq!(out.len(), 4 + 4 + 2 + 2 + 4 + 4);
        assert_eq!(*out.last().unwrap(), 0);
    }

    #[test]
    fn named_resources() {
        let mut r = ImageResource::new(2000, vec![9; 4]);
        r.name = b"path".to_vec();
        let mut r2 = ImageResource::new(2001, vec![]);
        r2.name = b"abc".to_vec();
        r2.signature = *b"MeSa";
        roundtrip(&[r, r2]);
    }

    #[test]
    fn bad_signature() {
        let mut out = Vec::new();
        write_section(&mut out, &[ImageResource::new(1, vec![])]).unwrap();
        out[4] = b'X';
        assert!(read_section(&mut Reader::new(&out)).is_err());
    }

    #[test]
    fn truncation_errors() {
        let mut out = Vec::new();
        write_section(&mut out, &[ImageResource::new(1005, ResolutionInfo::from_dpi(72.0).to_bytes())]).unwrap();
        for cut in 0..out.len() {
            assert!(read_section(&mut Reader::new(&out[..cut])).is_err());
        }
    }

    #[test]
    fn typed_resolution() {
        let ri = ResolutionInfo::from_dpi(300.0);
        let r = ImageResource::new(ids::RESOLUTION_INFO, ri.to_bytes());
        match r.parsed() {
            Some(Ok(ResourceData::ResolutionInfo(p))) => {
                assert_eq!(p, ri);
                assert!((p.h_res() - 300.0).abs() < 1e-9);
                assert!((p.v_res() - 300.0).abs() < 1e-9);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn typed_misc() {
        let r = ImageResource::new(ids::GLOBAL_ANGLE, 120i32.to_be_bytes().to_vec());
        assert_eq!(r.parsed(), Some(Ok(ResourceData::GlobalAngle(120))));
        let r = ImageResource::new(ids::GLOBAL_ALTITUDE, 30i32.to_be_bytes().to_vec());
        assert_eq!(r.parsed(), Some(Ok(ResourceData::GlobalAltitude(30))));
        let r = ImageResource::new(ids::LAYER_STATE, vec![0, 2]);
        assert_eq!(r.parsed(), Some(Ok(ResourceData::LayerState(2))));
        let r = ImageResource::new(ids::LAYER_GROUP_INFO, vec![0, 1, 0, 2]);
        assert_eq!(r.parsed(), Some(Ok(ResourceData::LayerGroupInfo(vec![1, 2]))));
        let r = ImageResource::new(ids::LAYER_GROUP_INFO, vec![0]);
        assert!(matches!(r.parsed(), Some(Err(_))));
        let r = ImageResource::new(ids::ICC_PROFILE, vec![1]);
        assert_eq!(r.parsed(), Some(Ok(ResourceData::IccProfile(&[1]))));
        let r = ImageResource::new(ids::EXIF, vec![2]);
        assert_eq!(r.parsed(), Some(Ok(ResourceData::Exif(&[2]))));
        let r = ImageResource::new(ids::THUMBNAIL, vec![3]);
        assert_eq!(r.parsed(), Some(Ok(ResourceData::Thumbnail(&[3]))));
        let r = ImageResource::new(ids::XMP, b"<x/>".to_vec());
        assert_eq!(r.xmp_str(), Some("<x/>"));
        assert_eq!(ImageResource::new(4242, vec![]).parsed(), None);
    }

    #[test]
    fn version_info_roundtrip() {
        let r = version_info_resource(false);
        match r.parsed() {
            Some(Ok(ResourceData::VersionInfo(v))) => {
                assert!(!v.has_real_merged_data);
                assert_eq!(v.to_bytes(), r.data);
            }
            other => panic!("{other:?}"),
        }
    }

    /// Byte for byte what Photoshop 2026 writes (every Photoshop-authored oracle in the corpus):
    /// u32 count + UTF-16BE, no NUL terminator.
    #[test]
    fn version_info_matches_photoshop_bytes() {
        let mut want = vec![0, 0, 0, 1, 1];
        for s in ["Adobe Photoshop", "Adobe Photoshop 2026"] {
            want.extend_from_slice(&(s.len() as u32).to_be_bytes());
            want.extend(s.encode_utf16().flat_map(u16::to_be_bytes));
        }
        want.extend_from_slice(&[0, 0, 0, 1]);
        assert_eq!(version_info_resource(true).data, want);
    }
}
