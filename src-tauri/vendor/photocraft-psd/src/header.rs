//! File header: signature, version, channels, dimensions, depth and color mode.

use crate::error::{PsdError, Result};
use crate::io::{Reader, WriteExt};

/// Maximum width/height accepted by the parser (PSB limit).
pub const MAX_DIMENSION: u32 = 300_000;
/// Maximum number of channels accepted (spec: 1..=56).
pub const MAX_CHANNELS: u16 = 56;

/// File format version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Version {
    /// Version 1: Photoshop Document (32-bit section lengths).
    Psd,
    /// Version 2: Large Document Format (64-bit lengths for some sections).
    Psb,
}

impl Version {
    /// `true` for PSB.
    pub fn is_psb(self) -> bool {
        matches!(self, Version::Psb)
    }
    /// Numeric version as stored in the header.
    pub fn as_u16(self) -> u16 {
        match self {
            Version::Psd => 1,
            Version::Psb => 2,
        }
    }
}

/// Document color mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ColorMode {
    /// 0
    Bitmap,
    /// 1
    Grayscale,
    /// 2
    Indexed,
    /// 3
    Rgb,
    /// 4
    Cmyk,
    /// 7
    Multichannel,
    /// 8
    Duotone,
    /// 9
    Lab,
    /// Any other value, preserved.
    Unknown(u16),
}

impl ColorMode {
    /// Converts from the stored numeric value.
    pub fn from_u16(v: u16) -> Self {
        match v {
            0 => ColorMode::Bitmap,
            1 => ColorMode::Grayscale,
            2 => ColorMode::Indexed,
            3 => ColorMode::Rgb,
            4 => ColorMode::Cmyk,
            7 => ColorMode::Multichannel,
            8 => ColorMode::Duotone,
            9 => ColorMode::Lab,
            other => ColorMode::Unknown(other),
        }
    }
    /// Numeric value as stored.
    pub fn as_u16(self) -> u16 {
        match self {
            ColorMode::Bitmap => 0,
            ColorMode::Grayscale => 1,
            ColorMode::Indexed => 2,
            ColorMode::Rgb => 3,
            ColorMode::Cmyk => 4,
            ColorMode::Multichannel => 7,
            ColorMode::Duotone => 8,
            ColorMode::Lab => 9,
            ColorMode::Unknown(v) => v,
        }
    }
    /// Number of color (non-alpha) channels for the mode, when fixed.
    pub fn color_channels(self) -> Option<u16> {
        match self {
            ColorMode::Bitmap | ColorMode::Grayscale | ColorMode::Indexed | ColorMode::Duotone => Some(1),
            ColorMode::Rgb | ColorMode::Lab => Some(3),
            ColorMode::Cmyk => Some(4),
            ColorMode::Multichannel | ColorMode::Unknown(_) => None,
        }
    }
}

/// The 26-byte file header.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Header {
    /// PSD or PSB.
    pub version: Version,
    /// Reserved bytes (spec: must be zero). Preserved for exact round-trip.
    pub reserved: [u8; 6],
    /// Number of channels in the merged image, including alpha channels (1..=56).
    pub channels: u16,
    /// Height in pixels.
    pub height: u32,
    /// Width in pixels.
    pub width: u32,
    /// Bits per channel: 1, 8, 16 or 32.
    pub depth: u16,
    /// Color mode.
    pub color_mode: ColorMode,
}

impl Header {
    /// Creates a header with zeroed reserved bytes.
    pub fn new(version: Version, width: u32, height: u32, channels: u16, depth: u16, color_mode: ColorMode) -> Self {
        Header { version, reserved: [0; 6], channels, height, width, depth, color_mode }
    }

    /// Bytes per row of one channel plane.
    pub fn row_bytes(&self) -> usize {
        row_bytes(self.width as usize, self.depth)
    }

    pub(crate) fn read(r: &mut Reader<'_>) -> Result<Self> {
        let sig = r.array::<4>()?;
        if &sig != b"8BPS" {
            return Err(PsdError::InvalidSignature { expected: "8BPS", found: sig });
        }
        let version = match r.u16()? {
            1 => Version::Psd,
            2 => Version::Psb,
            v => return Err(PsdError::UnsupportedVersion(v)),
        };
        let reserved = r.array::<6>()?;
        let channels = r.u16()?;
        let height = r.u32()?;
        let width = r.u32()?;
        let depth = r.u16()?;
        let color_mode = ColorMode::from_u16(r.u16()?);
        let h = Header { version, reserved, channels, height, width, depth, color_mode };
        h.validate()?;
        Ok(h)
    }

    /// Checks the header against spec constraints and safety limits.
    pub fn validate(&self) -> Result<()> {
        if self.channels == 0 || self.channels > MAX_CHANNELS {
            return Err(PsdError::invalid(format!("channel count {} out of range 1..=56", self.channels)));
        }
        if self.width == 0 || self.height == 0 {
            return Err(PsdError::invalid(format!("image dimensions {}x{} must be at least 1x1", self.width, self.height)));
        }
        if self.width > MAX_DIMENSION || self.height > MAX_DIMENSION {
            return Err(PsdError::LimitExceeded("image dimensions exceed 300000"));
        }
        if !matches!(self.depth, 1 | 8 | 16 | 32) {
            return Err(PsdError::invalid(format!("unsupported bit depth {}", self.depth)));
        }
        Ok(())
    }

    pub(crate) fn write(&self, out: &mut Vec<u8>) {
        out.put(b"8BPS");
        out.put_u16(self.version.as_u16());
        out.put(&self.reserved);
        out.put_u16(self.channels);
        out.put_u32(self.height);
        out.put_u32(self.width);
        out.put_u16(self.depth);
        out.put_u16(self.color_mode.as_u16());
    }
}

/// Bytes per row for `width` samples at `depth` bits.
pub fn row_bytes(width: usize, depth: u16) -> usize {
    match depth {
        1 => width.div_ceil(8),
        d => width.saturating_mul(usize::from(d / 8).max(1)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(h: &Header) -> Vec<u8> {
        let mut v = Vec::new();
        h.write(&mut v);
        v
    }

    #[test]
    fn header_roundtrip() {
        let h = Header::new(Version::Psb, 1000, 20, 4, 16, ColorMode::Cmyk);
        let b = bytes(&h);
        assert_eq!(b.len(), 26);
        assert_eq!(Header::read(&mut Reader::new(&b)).unwrap(), h);
    }

    #[test]
    fn bad_signature() {
        let mut b = bytes(&Header::new(Version::Psd, 1, 1, 1, 8, ColorMode::Grayscale));
        b[0] = b'X';
        assert!(matches!(Header::read(&mut Reader::new(&b)), Err(PsdError::InvalidSignature { .. })));
    }

    #[test]
    fn bad_version() {
        let mut b = bytes(&Header::new(Version::Psd, 1, 1, 1, 8, ColorMode::Grayscale));
        b[5] = 3;
        assert_eq!(Header::read(&mut Reader::new(&b)), Err(PsdError::UnsupportedVersion(3)));
    }

    #[test]
    fn limits() {
        for h in [
            Header::new(Version::Psd, 300_001, 1, 1, 8, ColorMode::Rgb),
            Header::new(Version::Psd, 0, 1, 1, 8, ColorMode::Rgb),
            Header::new(Version::Psd, 1, 0, 1, 8, ColorMode::Rgb),
            Header::new(Version::Psb, 0, 0, 1, 8, ColorMode::Rgb),
            Header::new(Version::Psd, 1, 1, 0, 8, ColorMode::Rgb),
            Header::new(Version::Psd, 1, 1, 57, 8, ColorMode::Rgb),
            Header::new(Version::Psd, 1, 1, 3, 12, ColorMode::Rgb),
        ] {
            let b = bytes(&h);
            assert!(Header::read(&mut Reader::new(&b)).is_err(), "{h:?}");
        }
    }

    #[test]
    fn color_modes_roundtrip() {
        for v in 0..20u16 {
            assert_eq!(ColorMode::from_u16(v).as_u16(), v);
        }
        assert_eq!(ColorMode::from_u16(5), ColorMode::Unknown(5));
        assert_eq!(ColorMode::Rgb.color_channels(), Some(3));
    }

    #[test]
    fn row_bytes_values() {
        assert_eq!(row_bytes(9, 1), 2);
        assert_eq!(row_bytes(8, 1), 1);
        assert_eq!(row_bytes(0, 1), 0);
        assert_eq!(row_bytes(3, 8), 3);
        assert_eq!(row_bytes(3, 16), 6);
        assert_eq!(row_bytes(3, 32), 12);
    }
}
