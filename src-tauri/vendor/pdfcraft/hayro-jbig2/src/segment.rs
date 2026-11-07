//! Segment parsing for JBIG2 bitstreams (Section 7.2).
//!
//! This module handles parsing of individual segment headers and defines
//! the segment types used in JBIG2.

use alloc::vec::Vec;

use crate::error::{ParseError, Result, SegmentError, bail, err};
use crate::reader::Reader;

/// PdfCraft patch: the most segments one segment may refer to. A long-form header can claim a
/// 29-bit count, which `Vec::with_capacity` below would turn into a multi-gigabyte reservation
/// before a single referred-to number is read; the allocation failure aborts the process. Real
/// files refer to a handful of segments.
pub(crate) const MAX_REFERRED_TO_SEGMENTS: u32 = u16::MAX as u32;

/// "The segment type is a number between 0 and 63, inclusive. Not all values
/// are allowed." (7.3)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SegmentType {
    /// Symbol dictionary – see 7.4.2. (type 0)
    SymbolDictionary,
    /// Intermediate text region – see 7.4.3. (type 4)
    IntermediateTextRegion,
    /// Immediate text region – see 7.4.3. (type 6)
    ImmediateTextRegion,
    /// Immediate lossless text region – see 7.4.3. (type 7)
    ImmediateLosslessTextRegion,
    /// Pattern dictionary – see 7.4.4. (type 16)
    PatternDictionary,
    /// Intermediate halftone region – see 7.4.5. (type 20)
    IntermediateHalftoneRegion,
    /// Immediate halftone region – see 7.4.5. (type 22)
    ImmediateHalftoneRegion,
    /// Immediate lossless halftone region – see 7.4.5. (type 23)
    ImmediateLosslessHalftoneRegion,
    /// Intermediate generic region – see 7.4.6. (type 36)
    IntermediateGenericRegion,
    /// Immediate generic region – see 7.4.6. (type 38)
    ImmediateGenericRegion,
    /// Immediate lossless generic region – see 7.4.6. (type 39)
    ImmediateLosslessGenericRegion,
    /// Intermediate generic refinement region – see 7.4.7. (type 40)
    IntermediateGenericRefinementRegion,
    /// Immediate generic refinement region – see 7.4.7. (type 42)
    ImmediateGenericRefinementRegion,
    /// Immediate lossless generic refinement region – see 7.4.7. (type 43)
    ImmediateLosslessGenericRefinementRegion,
    /// Page information – see 7.4.8. (type 48)
    PageInformation,
    /// End of page – see 7.4.9. (type 49)
    EndOfPage,
    /// End of stripe – see 7.4.10. (type 50)
    EndOfStripe,
    /// End of file – see 7.4.11. (type 51)
    EndOfFile,
    /// Profiles – see 7.4.12. (type 52)
    Profiles,
    /// Tables – see 7.4.13. (type 53)
    Tables,
    /// Colour palette – see 7.4.16. (type 54)
    ColourPalette,
    /// Extension - see 7.4.14. (type 62)
    Extension,
}

impl SegmentType {
    /// "All other segment types are reserved and must not be used." (7.3)
    fn from_type_value(value: u8) -> Result<Self> {
        match value {
            0 => Ok(Self::SymbolDictionary),
            4 => Ok(Self::IntermediateTextRegion),
            6 => Ok(Self::ImmediateTextRegion),
            7 => Ok(Self::ImmediateLosslessTextRegion),
            16 => Ok(Self::PatternDictionary),
            20 => Ok(Self::IntermediateHalftoneRegion),
            22 => Ok(Self::ImmediateHalftoneRegion),
            23 => Ok(Self::ImmediateLosslessHalftoneRegion),
            36 => Ok(Self::IntermediateGenericRegion),
            38 => Ok(Self::ImmediateGenericRegion),
            39 => Ok(Self::ImmediateLosslessGenericRegion),
            40 => Ok(Self::IntermediateGenericRefinementRegion),
            42 => Ok(Self::ImmediateGenericRefinementRegion),
            43 => Ok(Self::ImmediateLosslessGenericRefinementRegion),
            48 => Ok(Self::PageInformation),
            49 => Ok(Self::EndOfPage),
            50 => Ok(Self::EndOfStripe),
            51 => Ok(Self::EndOfFile),
            52 => Ok(Self::Profiles),
            53 => Ok(Self::Tables),
            54 => Ok(Self::ColourPalette),
            62 => Ok(Self::Extension),
            _ => err!(SegmentError::UnknownType),
        }
    }
}

/// A parsed segment header (7.2.1).
#[derive(Debug, Clone)]
pub(crate) struct SegmentHeader {
    /// "This four-byte field contains the segment's segment number. The valid
    /// range of segment numbers is 0 through 4294967295 (0xFFFFFFFF) inclusive."
    /// (7.2.2)
    pub(crate) segment_number: u32,
    /// "Bits 0-5: Segment type. See 7.3." (7.2.3)
    pub(crate) segment_type: SegmentType,
    /// "Bit 7: Deferred non-retain. If this bit is 1, this segment is flagged
    /// as retained only by itself and its attached extension segments." (7.2.3)
    pub(crate) _retain_flag: bool,
    /// "This field encodes the number of the page to which this segment belongs.
    /// The first page must be numbered '1'. This field may contain a value of
    /// zero; this value indicates that this segment is not associated with any
    /// page." (7.2.6)
    pub(crate) _page_association: u32,
    /// "This field contains the segment numbers of the segments that this segment
    /// refers to, if any." (7.2.5)
    pub(crate) referred_to_segments: Vec<u32>,
    /// "This 4-byte field contains the length of the segment's segment data part,
    /// in bytes." (7.2.7)
    ///
    /// `None` means the length was unknown (0xFFFFFFFF), which is only valid for
    /// immediate generic region segments in sequential organization.
    pub(crate) data_length: Option<u32>,
}

/// A parsed segment with its header and data.
#[derive(Debug)]
pub(crate) struct Segment<'a> {
    /// The segment header.
    pub(crate) header: SegmentHeader,
    /// The segment data (borrowed slice).
    pub(crate) data: &'a [u8],
}

/// Parse a segment header (7.2).
pub(crate) fn parse_segment_header(reader: &mut Reader<'_>) -> Result<SegmentHeader> {
    // 7.2.2: Segment number
    // "This four-byte field contains the segment's segment number. The valid
    // range of segment numbers is 0 through 4294967295 (0xFFFFFFFF) inclusive.
    // As mentioned before, it is possible for there to be gaps in the segment
    // numbering."
    let segment_number = reader.read_u32().ok_or(ParseError::UnexpectedEof)?;

    // 7.2.3: Segment header flags
    // "This is a 1-byte field."
    let flags = reader.read_byte().ok_or(ParseError::UnexpectedEof)?;

    // "Bits 0-5: Segment type. See 7.3."
    let segment_type = SegmentType::from_type_value(flags & 0x3F)?;

    // "Bit 6: Page association field size. See 7.2.6."
    let page_association_long = flags & 0x40 != 0;

    // "Bit 7: Deferred non-retain. If this bit is 1, this segment is flagged as
    // retained only by itself and its attached extension segments."
    let retain_flag = flags & 0x80 == 0;

    // 7.2.4: Referred-to segment count and retention flags
    // "This field contains one or more bytes indicating how many other segments
    // are referred to by this segment, and which segments contain data that is
    // needed after this segment."
    //
    // "The three most significant bits of the first byte in this field determine
    // the length of the field. If the value of this three-bit subfield is between
    // 0 and 4, then the field is one byte long. If the value of this three-bit
    // subfield is 7, then the field is at least five bytes long. This three-bit
    // subfield must not contain values of 5 and 6."
    let count_and_retention = reader.read_byte().ok_or(ParseError::UnexpectedEof)?;
    let short_count = (count_and_retention >> 5) & 0x07;

    if short_count == 5 || short_count == 6 {
        bail!(SegmentError::InvalidReferredCount);
    }

    let referred_to_count = if short_count < 7 {
        // Short form: "Bits 5-7: Count of referred-to segments. This field may
        // take on values between zero and four."
        short_count as u32
    } else {
        // Long form: "In the case where the field is in the long format (at least
        // five bytes long), it is composed of an initial four-byte field, followed
        // by a succession of one-byte fields."
        //
        // "Bits 0-28: Count of referred-to segments. This specifies the number of
        // segments that this segment refers to."
        // "Bits 29-31: Indication of long-form format. This field must contain the
        // value 7."
        let rest = reader.read_bytes(3).ok_or(ParseError::UnexpectedEof)?;
        u32::from_be_bytes([count_and_retention & 0x1F, rest[0], rest[1], rest[2]])
    };

    // PdfCraft patch: refuse an oversized count before reserving memory for it and before
    // skipping the retention-flag bytes it claims.
    if referred_to_count > MAX_REFERRED_TO_SEGMENTS {
        bail!(SegmentError::InvalidReferredCount);
    }

    // Skip retention flag bytes in long form.
    // "The first one-byte field following the initial four-byte field is formatted
    // as follows: Bit 0: Retain bit for this segment. Bit 1-7: Retain bits for
    // referred-to segments."
    if short_count == 7 {
        // Number of retention bytes: ceil((referred_to_count + 1) / 8)
        let retention_bytes = (referred_to_count as usize + 1).div_ceil(8);
        reader
            .skip_bytes(retention_bytes)
            .ok_or(ParseError::UnexpectedEof)?;
    }

    // 7.2.5: Referred-to segment numbers
    // "When the current segment's number is 256 or less, then each referred-to
    // segment number is one byte long. Otherwise, when the current segment's
    // number is 65536 or less, each referred-to segment number is two bytes long.
    // Otherwise, each referred-to segment number is four bytes long."
    let mut referred_to_segments = Vec::with_capacity(referred_to_count as usize);
    for _ in 0..referred_to_count {
        let referred = if segment_number <= 256 {
            reader.read_byte().ok_or(ParseError::UnexpectedEof)? as u32
        } else if segment_number <= 65536 {
            reader.read_u16().ok_or(ParseError::UnexpectedEof)? as u32
        } else {
            reader.read_u32().ok_or(ParseError::UnexpectedEof)?
        };

        // If a segment refers to other segments, it must refer to only segments
        // with lower segment numbers.
        if referred >= segment_number {
            bail!(SegmentError::InvalidReference);
        }

        referred_to_segments.push(referred);
    }

    // 7.2.6: Segment page association
    // "This field is one byte long if this segment's page association field size
    // flag bit is 0, and is four bytes long if this segment's page association
    // field size flag bit is 1."
    let page_association = if page_association_long {
        reader.read_u32().ok_or(ParseError::UnexpectedEof)?
    } else {
        reader.read_byte().ok_or(ParseError::UnexpectedEof)? as u32
    };

    // 7.2.7: Segment data length
    // "This 4-byte field contains the length of the segment's segment data part,
    // in bytes."
    //
    // "If the segment's type is 'Immediate generic region', then the length field
    // may contain the value 0xFFFFFFFF. This value is intended to mean that the
    // length of the segment's data part is unknown at the time that the segment
    // header is written."
    let data_length_raw = reader.read_u32().ok_or(ParseError::UnexpectedEof)?;
    let data_length = if data_length_raw == 0xFFFFFFFF {
        None
    } else {
        Some(data_length_raw)
    };

    Ok(SegmentHeader {
        segment_number,
        segment_type,
        _retain_flag: retain_flag,
        _page_association: page_association,
        referred_to_segments,
        data_length,
    })
}

/// Parse a complete segment (header + data).
pub(crate) fn parse_segment<'a>(reader: &mut Reader<'a>) -> Result<Segment<'a>> {
    let header = parse_segment_header(reader)?;
    parse_segment_data(reader, header)
}

/// Parse segment data for a previously parsed header.
///
/// "If the segment's type is 'Immediate generic region', then the length field
/// may contain the value 0xFFFFFFFF. This value is intended to mean that the
/// length of the segment's data part is unknown at the time that the segment
/// header is written." (7.2.7)
pub(crate) fn parse_segment_data<'a>(
    reader: &mut Reader<'a>,
    header: SegmentHeader,
) -> Result<Segment<'a>> {
    let data = if let Some(len) = header.data_length {
        reader
            .read_bytes(len as usize)
            .ok_or(ParseError::UnexpectedEof)?
    } else {
        // "In order for the decoder to correctly decode the segment, it needs to
        // read the four-byte row count field, which is stored in the last four
        // bytes of the segment's data part. These four bytes can be detected
        // without knowing the length of the data part in advance: if MMR is 1,
        // they are preceded by the two-byte sequence 0x00 0x00; if MMR is 0, they
        // are preceded by the two-byte sequence 0xFF 0xAC." (7.4.6.4)
        let len = scan_for_immediate_generic_region_size(reader)?;
        reader.read_bytes(len).ok_or(ParseError::UnexpectedEof)?
    };

    Ok(Segment { header, data })
}

/// Scan for the end of an immediate generic region segment with unknown length.
///
/// "The form of encoding used by the segment may be determined by examining
/// the eighteenth byte of its segment data part, and the end sequences can
/// occur anywhere after that eighteenth byte." (7.2.7)
fn scan_for_immediate_generic_region_size(reader: &Reader<'_>) -> Result<usize> {
    let mut scan = reader.clone();
    let start_offset = scan.byte_pos();

    scan.skip_bytes(17).ok_or(ParseError::UnexpectedEof)?;
    let flags = scan.read_byte().ok_or(ParseError::UnexpectedEof)?;
    let uses_mmr = (flags & 1) != 0;

    // "if MMR is 1, they are preceded by the two-byte sequence 0x00 0x00;
    // if MMR is 0, they are preceded by the two-byte sequence 0xFF 0xAC."
    let end_marker: [u8; 2] = if uses_mmr { [0x00, 0x00] } else { [0xFF, 0xAC] };

    // Search for the end marker. The marker is followed by a 4-byte row count.
    while let Some(bytes) = scan.peek_bytes(6) {
        if bytes[..2] == end_marker {
            // Found the marker. Total size is current offset + 2 (marker) + 4 (row count) - start.
            return Ok(scan.byte_pos() - start_offset + 2 + 4);
        }
        scan.skip_bytes(1).ok_or(ParseError::UnexpectedEof)?;
    }

    err!(SegmentError::MissingEndMarker)
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;

    #[test]
    fn test_segment_header_example_1() {
        // 7.2.8 Segment header example, EXAMPLE 1:
        // "A segment header consisting of the sequence of bytes:
        // 0x00 0x00 0x00 0x20 0x86 0x6B 0x02 0x1E 0x05 0x04"
        //
        // Plus 4 bytes for data length (not shown in example).
        let data = [
            0x00, 0x00, 0x00, 0x20, // Segment number = 32
            0x86, // Flags: type 6, page assoc 1 byte, deferred non-retain
            0x6B, // Refers to 3 segments, retention flags
            0x02, 0x1E, 0x05, // Referred segments: 2, 30, 5
            0x04, // Page association = 4
            0x00, 0x00, 0x00, 0x10, // Data length = 16 (added for complete header)
        ];

        let mut reader = Reader::new(&data);
        let header = parse_segment_header(&mut reader).unwrap();

        // "0x00 0x00 0x00 0x20: This segment's number is 0x00000020, or 32 decimal."
        assert_eq!(header.segment_number, 32);

        // "0x86: This segment's type is 6. Its page association field is one byte
        // long. It is retained by only its attached extension segments."
        assert_eq!(header.segment_type, SegmentType::ImmediateTextRegion);
        assert!(!header._retain_flag);

        // "0x6B: This segment refers to three other segments. It is referred to by
        // some other segment. This is the last reference to the second of the three
        // segments that it refers to."
        // "0x02 0x1E 0x05: The three segments that it refers to are numbers 2, 30, and 5."
        assert_eq!(header.referred_to_segments, vec![2, 30, 5]);

        // "0x04: This segment is associated with page number 4."
        assert_eq!(header._page_association, 4);

        assert_eq!(header.data_length, Some(16));
    }

    #[test]
    fn test_segment_header_example_2() {
        // 7.2.8 Segment header example, EXAMPLE 2:
        // "A segment header consisting of the sequence of bytes, in hexadecimal:
        // 00 00 02 34 40 E0 00 00 09 02 FD 01 00 00 02 00
        // 1E 00 05 02 00 02 01 02 02 02 03 02 04 00 00 04
        // 01"
        //
        // Plus 4 bytes for data length (not shown in example).
        #[rustfmt::skip]
        let data = [
            0x00, 0x00, 0x02, 0x34, // Segment number = 564
            0x40,                   // Flags: type 0, page assoc 4 bytes
            0xE0, 0x00, 0x00, 0x09, // Long form: refers to 9 segments
            0x02, 0xFD,             // Retention flags (2 bytes)
            0x01, 0x00,             // Referred segment 256
            0x00, 0x02,             // Referred segment 2
            0x00, 0x1E,             // Referred segment 30
            0x00, 0x05,             // Referred segment 5
            0x02, 0x00,             // Referred segment 512
            0x02, 0x01,             // Referred segment 513
            0x02, 0x02,             // Referred segment 514
            0x02, 0x03,             // Referred segment 515
            0x02, 0x04,             // Referred segment 516
            0x00, 0x00, 0x04, 0x01, // Page association = 1025
            0x00, 0x00, 0x00, 0x20, // Data length = 32 (added for complete header)
        ];

        let mut reader = Reader::new(&data);
        let header = parse_segment_header(&mut reader).unwrap();

        // "00 00 02 34: This segment's number is 0x00000234, or 564 decimal."
        assert_eq!(header.segment_number, 564);

        // "40: This segment's type is 0. Its page association field is four bytes long."
        assert_eq!(header.segment_type, SegmentType::SymbolDictionary);
        assert!(header._retain_flag);

        // "E0 00 00 09: This segment's referred-to segment count field is in the long
        // format. This segment refers to nine other segments."
        // "01 00 ... 02 04: The nine segments that it refers to are each identified by
        // two bytes, since this segment's number is between 256 and 65535. The segments
        // that it refers to are, in decimal, numbers 256, 2, 30, 5, 512, 513, 514, 515,
        // and 516."
        assert_eq!(
            header.referred_to_segments,
            vec![256, 2, 30, 5, 512, 513, 514, 515, 516]
        );

        // "00 00 04 01: This segment is associated with page number 1025."
        assert_eq!(header._page_association, 1025);

        assert_eq!(header.data_length, Some(32));
    }

    /// PdfCraft patch test (ported from craft-libs): a long-form referred-to count over
    /// `MAX_REFERRED_TO_SEGMENTS` is refused before its retention bytes are read.
    #[test]
    fn test_referred_to_count_is_capped() {
        use crate::error::DecodeError;

        let header = |count: u32| {
            let mut data = vec![
                0x00, 0x01, 0x00, 0x00, // Segment number = 65536
                0x86, // Flags: type 6, page assoc 1 byte, deferred non-retain
            ];
            // Long form: bits 29-31 are 7, bits 0-28 the count.
            data.extend_from_slice(&(0xE000_0000 | count).to_be_bytes());
            parse_segment_header(&mut Reader::new(&data)).unwrap_err()
        };

        // The 29-bit maximum, 536,870,911, would reserve about 2 GiB.
        for count in [0x1FFF_FFFF, MAX_REFERRED_TO_SEGMENTS + 1] {
            assert!(matches!(
                header(count),
                DecodeError::Segment(SegmentError::InvalidReferredCount)
            ));
        }
        // At the cap the header is read on and ends at its missing retention bytes.
        assert!(matches!(
            header(MAX_REFERRED_TO_SEGMENTS),
            DecodeError::Parse(ParseError::UnexpectedEof)
        ));
    }
}
