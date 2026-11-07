//! Region and dictionary segment parsing and decoding.

pub(crate) mod generic;
pub(crate) mod generic_refinement;
pub(crate) mod halftone;
pub(crate) mod pattern;
pub(crate) mod symbol;
pub(crate) mod text;

use crate::bitmap::Bitmap;
use crate::decode::RefinementTemplate::{Template0, Template1};
use crate::error::{ParseError, RegionError, Result, bail, err};
use crate::reader::Reader;
use alloc::vec;
use alloc::vec::Vec;

/// A decoded region bitmap with its combination operator.
#[derive(Debug, Clone)]
pub(crate) struct RegionBitmap {
    /// The decoded bitmap.
    pub(crate) bitmap: Bitmap,
    /// How to combine this region with the page bitmap.
    pub(crate) combination_operator: CombinationOperator,
}

/// "These operators describe how the segment's bitmap is to be combined with
/// the page bitmap." (7.4.1.5)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum CombinationOperator {
    /// 0 OR
    #[default]
    Or,
    /// 1 AND
    And,
    /// 2 XOR
    Xor,
    /// 3 XNOR
    Xnor,
    /// 4 REPLACE
    Replace,
}

// Not really a default, just used as a dummy placeholder.

impl CombinationOperator {
    pub(crate) fn from_value(value: u8) -> Result<Self> {
        match value & 0x07 {
            0 => Ok(Self::Or),
            1 => Ok(Self::And),
            2 => Ok(Self::Xor),
            3 => Ok(Self::Xnor),
            4 => Ok(Self::Replace),
            _ => err!(RegionError::InvalidCombinationOperator),
        }
    }
}

/// Template used for arithmetic of generic regions.
///
/// - Generic regions: `GBTEMPLATE` (7.4.6.2)
/// - Symbol dictionaries: `SDTEMPLATE` (7.4.2.1.1)
/// - Pattern dictionaries: `HDTEMPLATE` (7.4.4.1.1)
/// - Halftone regions: `HTEMPLATE` (7.4.5.1.1)
/// - Gray-scale images: `GSTEMPLATE` (Annex C)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Template {
    /// Template 0
    ///
    /// Context bits: 16 (Figure 3).
    Template0 = 0,
    /// Template 1
    ///
    /// Context bits: 13 (Figure 4).
    Template1 = 1,
    /// Template 2
    ///
    /// Context bits: 10 (Figure 5).
    Template2 = 2,
    /// Template 3
    ///
    /// Context bits: 10 (Figure 6).
    Template3 = 3,
}

impl Template {
    pub(crate) fn from_byte(value: u8) -> Self {
        match value & 0x03 {
            0 => Self::Template0,
            1 => Self::Template1,
            2 => Self::Template2,
            3 => Self::Template3,
            _ => unreachable!(),
        }
    }

    /// Number of context bits for this template (6.2.5.3).
    pub(crate) fn context_bits(self) -> usize {
        match self {
            Self::Template0 => 16,
            Self::Template1 => 13,
            Self::Template2 | Self::Template3 => 10,
        }
    }

    /// Number of adaptive template pixels (6.2.5.3).
    pub(crate) fn adaptive_template_pixels(&self) -> u8 {
        match self {
            Self::Template0 => 4,
            Self::Template1 | Self::Template2 | Self::Template3 => 1,
        }
    }
}

/// Parsed region segment information field (7.4.1).
///
/// "A region segment information field contains the following subfields, as
/// shown in Figure 30:
/// - Region segment bitmap width – see 7.4.1.1.
/// - Region segment bitmap height – see 7.4.1.2.
/// - Region segment bitmap X location – see 7.4.1.3.
/// - Region segment bitmap Y location – see 7.4.1.4.
/// - Region segment flags – see 7.4.1.5." (7.4.1)
#[derive(Debug, Clone)]
pub(crate) struct RegionSegmentInfo {
    /// "This four-byte field gives the width in pixels of the bitmap encoded
    /// in this segment." (7.4.1.1)
    pub(crate) width: u32,
    /// "This four-byte field gives the height in pixels of the bitmap encoded
    /// in this segment." (7.4.1.2)
    pub(crate) height: u32,
    /// "This four-byte field gives the horizontal offset in pixels of the bitmap
    /// encoded in this segment relative to the page bitmap." (7.4.1.3)
    pub(crate) x_location: u32,
    /// "This four-byte field gives the vertical offset in pixels of the bitmap
    /// encoded in this segment relative to the page bitmap." (7.4.1.4)
    pub(crate) y_location: u32,
    /// "Bits 0-2: External combination operator." (7.4.1.5)
    pub(crate) combination_operator: CombinationOperator,
    /// "Bit 3: Colour extension flag (COLEXTFLAG). This field specifies whether
    /// the region segment is extended to represent coloured bitmap." (7.4.1.5)
    pub(crate) _colour_extension: bool,
}

/// Parse the region segment information field (7.4.1).
pub(crate) fn parse_region_segment_info(reader: &mut Reader<'_>) -> Result<RegionSegmentInfo> {
    // 7.4.1.1: Region segment bitmap width
    let width = reader.read_u32().ok_or(ParseError::UnexpectedEof)?;
    // 7.4.1.2: Region segment bitmap height
    let height = reader.read_u32().ok_or(ParseError::UnexpectedEof)?;
    // 7.4.1.3: Region segment bitmap X location
    let x_location = reader.read_u32().ok_or(ParseError::UnexpectedEof)?;
    // 7.4.1.: Region segment bitmap Y location
    let y_location = reader.read_u32().ok_or(ParseError::UnexpectedEof)?;

    // 7.4.1.5: Region segment flags
    let flags = reader.read_byte().ok_or(ParseError::UnexpectedEof)?;

    // "Bits 0-2: External combination operator."
    let combination_operator = CombinationOperator::from_value(flags)?;

    // "Bit 3: Colour extension flag (COLEXTFLAG)."
    let colour_extension = flags & 0x08 != 0;

    // "Bits 4-7: Reserved; must be 0."
    if flags & 0xF0 != 0 {
        bail!(RegionError::InvalidCombinationOperator);
    }

    Ok(RegionSegmentInfo {
        width,
        height,
        x_location,
        y_location,
        combination_operator,
        _colour_extension: colour_extension,
    })
}

/// Adaptive template pixel position for generic and refinement regions (6.2.5.4, Figure 7).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct AdaptiveTemplatePixel {
    pub(crate) x: i8,
    pub(crate) y: i8,
}

/// Parse refinement adaptive template pixels (used by symbol dictionary and text region).
///
/// Used for:
/// - Symbol dictionary refinement AT flags (7.4.2.1.3): SDRATX1/SDRATY1, SDRATX2/SDRATY2
/// - Text region refinement AT flags (7.4.3.1.3): SBRATX1/SBRATY1, SBRATX2/SBRATY2
/// - Generic refinement region AT flags (7.4.7.3): GRATX1/GRATY1, GRATX2/GRATY2
pub(crate) fn parse_refinement_at_pixels(
    reader: &mut Reader<'_>,
) -> Result<Vec<AdaptiveTemplatePixel>> {
    let x1 = reader.read_byte().ok_or(ParseError::UnexpectedEof)? as i8;
    let y1 = reader.read_byte().ok_or(ParseError::UnexpectedEof)? as i8;

    let x2 = reader.read_byte().ok_or(ParseError::UnexpectedEof)? as i8;
    let y2 = reader.read_byte().ok_or(ParseError::UnexpectedEof)? as i8;

    Ok(vec![
        AdaptiveTemplatePixel { x: x1, y: y1 },
        AdaptiveTemplatePixel { x: x2, y: y2 },
    ])
}

/// Template used for refinement arithmetic coding (7.4.7.2).
///
/// - Generic refinement regions: `GRTEMPLATE` (7.4.7.2)
/// - Symbol dictionaries: `SDRTEMPLATE` (7.4.2.1.1)
/// - Text regions: `SBRTEMPLATE` (7.4.3.1.1)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RefinementTemplate {
    /// Template 0: 13 pixels (6.3.5.3, Figure 12)
    Template0 = 0,
    /// Template 1: 10 pixels (6.3.5.3, Figure 13)
    Template1 = 1,
}

impl RefinementTemplate {
    pub(crate) fn from_byte(value: u8) -> Self {
        if value & 0x01 == 0 {
            Template0
        } else {
            Template1
        }
    }

    /// Number of context bits for this template (6.3.5.3).
    pub(crate) fn context_bits(&self) -> usize {
        match self {
            Self::Template0 => 13,
            Self::Template1 => 10,
        }
    }
}
