//! File format parsing for JBIG2 bitstreams (Annex D of ITU-T T.88).
//!
//! This module handles parsing of standalone JBIG2 files in both sequential
//! and random-access organization formats.

use alloc::vec::Vec;

use crate::error::{FormatError, OverflowError, ParseError, Result, bail};
use crate::reader::Reader;
use crate::segment::{
    Segment, SegmentType, parse_segment, parse_segment_data, parse_segment_header,
};

/// PdfCraft patch: the most segments a stream, with its globals, may have. They are all parsed and
/// sorted before the first is decoded, at about 64 bytes for 11 of input, so behind nested Flate a
/// 4,765-byte PDF reached 5 GB and failed only afterwards.
const MAX_SEGMENTS: usize = 1 << 20;

/// PdfCraft patch: the most references those segments may make, 4 bytes each for 1 of input.
const MAX_REFERENCES: usize = 1 << 22;

/// PdfCraft patch: the segments of a stream so far (see [`MAX_SEGMENTS`], [`MAX_REFERENCES`]).
#[derive(Default)]
pub(crate) struct SegmentCount {
    segments: usize,
    references: usize,
}

impl SegmentCount {
    /// Count one more segment, making `references` references.
    fn add(&mut self, references: usize) -> Result<()> {
        self.segments = self.segments.saturating_add(1);
        self.references = self.references.saturating_add(references);
        if self.segments > MAX_SEGMENTS || self.references > MAX_REFERENCES {
            bail!(OverflowError::SegmentCount);
        }
        Ok(())
    }
}

/// "There are two standalone file organizations possible for a JBIG2 bitstream."
/// (Annex D)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileOrganization {
    /// "This is a standalone file organization. This organization is intended for
    /// streaming applications, where the decoder is guaranteed to begin at the start
    /// of the bitstream and decode everything up to the end of the bitstream."
    /// (D.1)
    Sequential,
    /// "This is a standalone file organization. This organization is intended for
    /// random-access applications, where the decoder might want to process parts of
    /// the file in an arbitrary order." (D.2)
    RandomAccess,
}

/// Parsed file header.
///
/// "A file header contains the following fields, in order:
/// ID string – see D.4.1.
/// File header flags – see D.4.2.
/// Number of pages – see D.4.3." (D.4)
#[derive(Debug, Clone)]
pub(crate) struct FileHeader {
    /// The file organization type.
    pub(crate) organization: FileOrganization,
    /// The number of pages in the file, if known.
    pub(crate) _number_of_pages: Option<u32>,
    /// "If this bit is 0, no generic region segments uses the templates with
    /// 12 AT pixels. If the file contains one or more generic region segments
    /// using such templates, this bit must be 1." (D.4.2, Bit 2)
    pub(crate) _uses_extended_templates: bool,
    /// "If this bit is 0, no region segment is extended to be coloured. If the
    /// file contains one or more coloured region segments, this bit must be 1."
    /// (D.4.2, Bit 3)
    pub(crate) _contains_coloured_regions: bool,
}

/// A parsed JBIG2 file.
#[derive(Debug)]
pub(crate) struct File<'a> {
    /// The file header.
    pub(crate) _header: FileHeader,
    /// The segments in the file.
    pub(crate) segments: Vec<Segment<'a>>,
}

/// "This is an 8-byte sequence containing 0x97 0x4A 0x42 0x32 0x0D 0x0A 0x1A 0x0A."
/// (D.4.1)
const FILE_HEADER_ID: [u8; 8] = [0x97, 0x4A, 0x42, 0x32, 0x0D, 0x0A, 0x1A, 0x0A];

/// Parse a standalone JBIG2 file.
pub(crate) fn parse_file(data: &[u8]) -> Result<File<'_>> {
    let mut reader = Reader::new(data);

    let header = parse_file_header(&mut reader)?;
    let mut segments = parse_segments(&mut reader, header.organization)?;

    // Technically shouldn't be necessary because the spec mandates that segments
    // are in sorted order, but just to be safe.
    segments.sort_by_key(|seg| seg.header.segment_number);

    Ok(File {
        _header: header,
        segments,
    })
}

fn parse_file_header(reader: &mut Reader<'_>) -> Result<FileHeader> {
    // D.4.1: ID string
    let id = reader.read_bytes(8).ok_or(ParseError::UnexpectedEof)?;
    if id != FILE_HEADER_ID {
        bail!(FormatError::InvalidHeader);
    }

    // D.4.2: File header flags
    let flags = reader.read_byte().ok_or(ParseError::UnexpectedEof)?;

    // "Bit 0: File organization type. If this bit is 0, the file uses the
    // random-access organization. If this bit is 1, the file uses the
    // sequential organization." (D.4.2)
    let organization = if flags & 0x01 != 0 {
        FileOrganization::Sequential
    } else {
        FileOrganization::RandomAccess
    };

    // "Bit 1: Unknown number of pages. If this bit is 0, then the number of
    // pages contained in the file is known. If this bit is 1, then the number
    // of pages contained in the file was not known at the time that the file
    // header was coded." (D.4.2)
    let unknown_page_count = flags & 0x02 != 0;

    // "Bit 2: If this bit is 0, no generic region segments uses the templates
    // with 12 AT pixels." (D.4.2)
    let uses_extended_templates = flags & 0x04 != 0;

    // "Bit 3: If this bit is 0, no region segment is extended to be coloured."
    // (D.4.2)
    let contains_coloured_regions = flags & 0x08 != 0;

    // "Bits 4-7: Reserved; must be 0." (D.4.2)
    if flags & 0xF0 != 0 {
        bail!(FormatError::ReservedBits);
    }

    // D.4.3: Number of pages
    // "This is a 4-byte field, and is not present if the 'unknown number of
    // pages' bit was 1." (D.4.3)
    let number_of_pages = if unknown_page_count {
        None
    } else {
        Some(reader.read_u32().ok_or(ParseError::UnexpectedEof)?)
    };

    Ok(FileHeader {
        organization,
        _number_of_pages: number_of_pages,
        _uses_extended_templates: uses_extended_templates,
        _contains_coloured_regions: contains_coloured_regions,
    })
}

fn parse_segments<'a>(
    reader: &mut Reader<'a>,
    organization: FileOrganization,
) -> Result<Vec<Segment<'a>>> {
    let mut segments = Vec::new();

    match organization {
        FileOrganization::Sequential => {
            parse_segments_sequential(reader, &mut segments, &mut SegmentCount::default())?;
        }
        FileOrganization::RandomAccess => parse_segments_random_access(reader, &mut segments)?,
    }

    Ok(segments)
}

/// Parse segments in sequential organization.
///
/// "In this organization, the file structure looks like Figure D.1. A file header
/// is followed by a sequence of segments. The two parts of each segment are stored
/// together: first the segment header then the segment data." (D.1)
pub(crate) fn parse_segments_sequential<'a>(
    reader: &mut Reader<'a>,
    segments: &mut Vec<Segment<'a>>,
    count: &mut SegmentCount,
) -> Result<()> {
    loop {
        if reader.at_end() {
            break;
        }

        let segment = parse_segment(reader)?;
        count.add(segment.header.referred_to_segments.len())?;

        // "If a file contains an end of file segment, it must be the last segment."
        // (7.4.11)
        let is_eof = matches!(segment.header.segment_type, SegmentType::EndOfFile);
        segments.push(segment);

        if is_eof {
            break;
        }
    }

    Ok(())
}

/// Parse segments in random-access organization.
///
/// "In this organization, the file structure looks like Figure D.2. A file header
/// is followed by a sequence of segments headers; the last segment header is
/// followed by the data for the first segment, then the data for the second
/// segment, and so on." (D.2)
fn parse_segments_random_access<'a>(
    reader: &mut Reader<'a>,
    segments: &mut Vec<Segment<'a>>,
) -> Result<()> {
    let mut headers = Vec::new();
    let mut count = SegmentCount::default();

    loop {
        if reader.at_end() {
            break;
        }

        let header = parse_segment_header(reader)?;
        count.add(header.referred_to_segments.len())?;

        // "If a file contains an end of file segment, it must be the last segment."
        // (7.4.11)
        let is_eof = matches!(header.segment_type, SegmentType::EndOfFile);
        headers.push(header);

        if is_eof {
            break;
        }
    }

    // Then, read all segment data.
    for header in headers {
        segments.push(parse_segment_data(reader, header)?);
    }

    Ok(())
}
