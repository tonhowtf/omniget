//! Page information segment parsing (7.4.8).

use crate::decode::CombinationOperator;
use crate::error::{ParseError, Result};
use crate::reader::Reader;

/// Parsed page information segment (7.4.8).
#[derive(Debug, Clone, Default)]
pub(crate) struct PageInformation {
    /// "This is a four-byte value containing the width in pixels of the page's
    /// bitmap." (7.4.8.1)
    pub(crate) width: u32,
    /// "This is a four-byte value containing the height in pixels of the page's
    /// bitmap." (7.4.8.2)
    pub(crate) height: u32,
    /// "This is a four-byte value containing the resolution of the original page
    /// medium, measured in pixels/metre in the horizontal direction. If this
    /// value is unknown, then this field must contain 0x00000000." (7.4.8.3)
    ///
    /// `None` means unknown resolution.
    pub(crate) _x_resolution: Option<u32>,
    /// "This is a four-byte value containing the resolution of the original page
    /// medium, measured in pixels/metre in the vertical direction. If this value
    /// is unknown, then this field must contain 0x00000000." (7.4.8.4)
    ///
    /// `None` means unknown resolution.
    pub(crate) _y_resolution: Option<u32>,
    /// Page segment flags (7.4.8.5).
    pub(crate) flags: PageFlags,
    /// Page striping information (7.4.8.6).
    pub(crate) _striping: PageStriping,
}

/// Page segment flags (7.4.8.5, Figure 56).
#[derive(Debug, Clone)]
#[allow(unused)]
#[derive(Default)]
pub(crate) struct PageFlags {
    /// "Bit 0: Page is eventually lossless. If this bit is 0, then the file does
    /// not contain a lossless representation of the original (pre-coding) page.
    /// If this bit is 1, then the file contains enough information to reconstruct
    /// the original page." (7.4.8.5)
    pub(crate) is_lossless: bool,
    /// "Bit 1: Page might contain refinements. If this bit is 0, then no
    /// refinement region segment may be associated with the page. If this bit
    /// is 1, then such segments may be associated with the page." (7.4.8.5)
    pub(crate) might_contain_refinements: bool,
    /// "Bit 2: Page default pixel value. This bit contains the initial value for
    /// every pixel in the page, before any region segments are decoded or drawn."
    /// (7.4.8.5)
    pub(crate) default_pixel: u8,
    /// "Bits 3-4: Page default combination operator." (7.4.8.5)
    pub(crate) default_combination_operator: CombinationOperator,
    /// "Bit 5: Page requires auxiliary buffers. If this bit is 0, then no region
    /// segment requiring an auxiliary buffer may be associated with the page."
    /// (7.4.8.5)
    pub(crate) requires_auxiliary_buffers: bool,
    /// "Bit 6: Page combination operator overridden. If this bit is 0, then every
    /// direct region segment associated with this page must use the page's default
    /// combination operator. If this bit is 1, then direct region segments
    /// associated with this page may use any combination operators." (7.4.8.5)
    pub(crate) combination_operator_overridden: bool,
    /// "Bit 7: Page might contain coloured segment. If this bit is 0, then no
    /// segment with colour extension may be associated with the page." (7.4.8.5)
    pub(crate) might_contain_coloured: bool,
}

/// Page striping information (7.4.8.6, Figure 57).
#[derive(Debug, Clone, Default)]
pub(crate) struct PageStriping {
    /// "Bit 15: Page is striped. If the 'page is striped' bit is 1, then the page
    /// may have end of stripe segments associated with it." (7.4.8.6)
    ///
    /// "If the page's bitmap height is unknown (indicated by a page bitmap height
    /// of 0xFFFFFFFF) then the 'page is striped' bit must be 1." (7.4.8.6)
    pub(crate) _is_striped: bool,
    /// "Bits 0-14: Maximum stripe size." (7.4.8.6)
    ///
    /// "The maximum size of each stripe (the distance between an end of stripe
    /// segment's end row and the end row of the previous end of stripe segment,
    /// or 0 in the case of the first end of stripe segment) must be no more than
    /// the page's maximum stripe size." (7.4.8.6)
    pub(crate) _max_stripe_size: u16,
}

/// Parse a page information segment (7.4.8).
pub(crate) fn parse_page_information(reader: &mut Reader<'_>) -> Result<PageInformation> {
    // 7.4.8.1: Page bitmap width
    let width = reader.read_u32().ok_or(ParseError::UnexpectedEof)?;

    // 7.4.8.2: Page bitmap height
    let height = reader.read_u32().ok_or(ParseError::UnexpectedEof)?;

    // 7.4.8.3: Page X resolution
    let x_resolution_raw = reader.read_u32().ok_or(ParseError::UnexpectedEof)?;
    let x_resolution = if x_resolution_raw == 0 {
        None
    } else {
        Some(x_resolution_raw)
    };

    // 7.4.8.4: Page Y resolution
    let y_resolution_raw = reader.read_u32().ok_or(ParseError::UnexpectedEof)?;
    let y_resolution = if y_resolution_raw == 0 {
        None
    } else {
        Some(y_resolution_raw)
    };

    // 7.4.8.5: Page segment flags
    let flags_byte = reader.read_byte().ok_or(ParseError::UnexpectedEof)?;
    let flags = parse_page_flags(flags_byte);

    // 7.4.8.6: Page striping information
    let striping_raw = reader.read_u16().ok_or(ParseError::UnexpectedEof)?;
    let striping = PageStriping {
        _is_striped: striping_raw & 0x8000 != 0,
        _max_stripe_size: striping_raw & 0x7FFF,
    };

    Ok(PageInformation {
        width,
        height,
        _x_resolution: x_resolution,
        _y_resolution: y_resolution,
        flags,
        _striping: striping,
    })
}

fn parse_page_flags(flags: u8) -> PageFlags {
    let combo_bits = (flags >> 3) & 0x03;
    let default_combination_operator = match combo_bits {
        0 => CombinationOperator::Or,
        1 => CombinationOperator::And,
        2 => CombinationOperator::Xor,
        3 => CombinationOperator::Xnor,
        _ => unreachable!(),
    };

    PageFlags {
        is_lossless: flags & 0x01 != 0,
        might_contain_refinements: flags & 0x02 != 0,
        default_pixel: (flags >> 2) & 0x01,
        default_combination_operator,
        requires_auxiliary_buffers: flags & 0x20 != 0,
        combination_operator_overridden: flags & 0x40 != 0,
        might_contain_coloured: flags & 0x80 != 0,
    }
}
