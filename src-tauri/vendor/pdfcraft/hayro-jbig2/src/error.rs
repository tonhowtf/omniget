//! Error types for JBIG2 decoding.

use core::fmt;

/// The main error type for JBIG2 decoding operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// Errors related to reading/parsing data.
    Parse(ParseError),
    /// Errors related to file structure.
    Format(FormatError),
    /// Errors related to segment processing.
    Segment(SegmentError),
    /// Errors related to Huffman decoding.
    Huffman(HuffmanError),
    /// Errors related to region parameters.
    Region(RegionError),
    /// Errors related to template configuration.
    Template(TemplateError),
    /// Errors related to symbol handling.
    Symbol(SymbolError),
    /// Arithmetic overflow in calculations.
    Overflow(OverflowError),
    /// Feature not yet implemented.
    Unsupported,
}

/// Errors related to reading/parsing data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    /// Unexpected end of input.
    UnexpectedEof,
}

/// Errors related to file structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatError {
    /// Invalid file header signature.
    InvalidHeader,
    /// Reserved bits are not zero.
    ReservedBits,
    /// Missing required page information segment.
    MissingPageInfo,
    /// Page height unknown with no stripe segments.
    UnknownPageHeight,
    /// Page has zero width or height.
    EmptyPage,
}

/// Errors related to segment processing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentError {
    /// Unknown or reserved segment type.
    UnknownType,
    /// Invalid referred-to segment count.
    InvalidReferredCount,
    /// Segment refers to a larger segment number.
    InvalidReference,
    /// Missing end marker for unknown-length region.
    MissingEndMarker,
    /// Missing required pattern dictionary.
    MissingPatternDictionary,
}

/// Errors related to Huffman decoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HuffmanError {
    /// Invalid Huffman code sequence.
    InvalidCode,
    /// Invalid Huffman table selection.
    InvalidSelection,
    /// Not enough referred Huffman tables.
    MissingTables,
    /// Unexpected out-of-band value.
    UnexpectedOob,
    /// Huffman tree has conflicting codes (one code is a prefix of another).
    ConflictingCodes,
    /// Prefix length exceeds maximum supported value.
    PrefixLengthTooLarge,
}

/// Errors related to region parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionError {
    /// Invalid combination operator value.
    InvalidCombinationOperator,
    /// Region with invalid dimension.
    InvalidDimension,
    /// MMR (Modified Modified Read) decoding failed.
    InvalidMmrData,
    /// Gray-scale value exceeds pattern count.
    GrayScaleOutOfRange,
}

/// Errors related to template configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateError {
    /// An invalid template value was used.
    Invalid,
    /// Invalid adaptive template pixel location.
    InvalidAtPixel,
}

/// Errors related to symbol handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolError {
    /// No symbols available for text region.
    NoSymbols,
    /// The symbol dictionary contains more symbols than expected.
    TooManySymbols,
    /// Symbol ID out of valid range.
    OutOfRange,
    /// Unexpected out-of-band value.
    UnexpectedOob,
    /// An invalid symbol was encountered.
    Invalid,
    /// Strip delta multiplication overflowed.
    InvalidStripDelta,
    /// Too many symbol instances in a text region.
    TooManyInstances,
}

/// Arithmetic overflow errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverflowError {
    /// Bitmap dimension computation overflowed.
    BitmapDimension,
    /// Halftone grid dimension computation overflowed.
    GridDimension,
    /// Placement coordinate computation overflowed.
    PlacementCoordinate,
    /// Reference offset computation overflowed.
    ReferenceOffset,
    /// Index computation overflowed.
    Index,
    /// A symbol dictionary has too many symbols, or a segment refers to too many.
    SymbolCount,
    /// PdfCraft patch: the segments retain more bitmaps than a page may hold.
    RetainedBitmaps,
    /// PdfCraft patch: a stream has more segments, or references, than it may.
    SegmentCount,
    /// PdfCraft patch: a stream asks for more decoding work than it may.
    DecodingWork,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(e) => write!(f, "{e}"),
            Self::Format(e) => write!(f, "{e}"),
            Self::Segment(e) => write!(f, "{e}"),
            Self::Huffman(e) => write!(f, "{e}"),
            Self::Region(e) => write!(f, "{e}"),
            Self::Template(e) => write!(f, "{e}"),
            Self::Symbol(e) => write!(f, "{e}"),
            Self::Overflow(e) => write!(f, "{e}"),
            Self::Unsupported => write!(f, "unsupported feature"),
        }
    }
}

impl fmt::Display for OverflowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BitmapDimension => write!(f, "overflow in bitmap dimension computation"),
            Self::PlacementCoordinate => write!(f, "overflow in placement coordinate computation"),
            Self::ReferenceOffset => write!(f, "overflow in reference offset computation"),
            Self::Index => write!(f, "overflow in index computation"),
            Self::GridDimension => write!(f, "overflow in grid dimension computation"),
            Self::SymbolCount => write!(f, "too many symbols in a dictionary or referred to"),
            Self::RetainedBitmaps => write!(f, "the segments retain too many bitmaps"),
            Self::SegmentCount => write!(f, "too many segments or references in a stream"),
            Self::DecodingWork => write!(f, "a stream asks for too much decoding work"),
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEof => write!(f, "unexpected end of input"),
        }
    }
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidHeader => write!(f, "invalid JBIG2 file header"),
            Self::ReservedBits => write!(f, "reserved bits must be zero"),
            Self::MissingPageInfo => write!(f, "missing page information segment"),
            Self::UnknownPageHeight => write!(f, "page height unknown with no stripe segments"),
            Self::EmptyPage => write!(f, "page has zero width or height"),
        }
    }
}

impl fmt::Display for SegmentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownType => write!(f, "unknown or reserved segment type"),
            Self::InvalidReferredCount => write!(f, "invalid referred-to segment count"),
            Self::InvalidReference => write!(f, "segment refers to larger segment number"),
            Self::MissingEndMarker => write!(f, "missing end marker for unknown-length region"),
            Self::MissingPatternDictionary => write!(f, "missing required pattern dictionary"),
        }
    }
}

impl fmt::Display for HuffmanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCode => write!(f, "invalid Huffman code"),
            Self::InvalidSelection => write!(f, "invalid Huffman table selection"),
            Self::MissingTables => write!(f, "not enough referred Huffman tables"),
            Self::UnexpectedOob => write!(f, "unexpected out-of-band value"),
            Self::ConflictingCodes => write!(f, "conflicting Huffman codes"),
            Self::PrefixLengthTooLarge => write!(f, "Huffman prefix length too large"),
        }
    }
}

impl fmt::Display for RegionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCombinationOperator => write!(f, "invalid combination operator"),
            Self::InvalidDimension => write!(f, "invalid dimension value"),
            Self::InvalidMmrData => write!(f, "invalid MMR-encoded data"),
            Self::GrayScaleOutOfRange => write!(f, "gray-scale value exceeds pattern count"),
        }
    }
}

impl fmt::Display for TemplateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid => write!(f, "invalid template value"),
            Self::InvalidAtPixel => write!(f, "invalid adaptive template pixel location"),
        }
    }
}

impl fmt::Display for SymbolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSymbols => write!(f, "no symbols available"),
            Self::OutOfRange => write!(f, "symbol ID out of range"),
            Self::UnexpectedOob => write!(f, "unexpected out-of-band value"),
            Self::TooManySymbols => write!(f, "symbol dictionary contains too many symbols"),
            Self::Invalid => write!(f, "invalid symbol encountered"),
            Self::InvalidStripDelta => write!(f, "strip delta overflow"),
            Self::TooManyInstances => write!(f, "too many symbol instances"),
        }
    }
}

impl core::error::Error for DecodeError {}
impl core::error::Error for ParseError {}
impl core::error::Error for FormatError {}
impl core::error::Error for SegmentError {}
impl core::error::Error for HuffmanError {}
impl core::error::Error for RegionError {}
impl core::error::Error for TemplateError {}
impl core::error::Error for SymbolError {}
impl core::error::Error for OverflowError {}

impl From<ParseError> for DecodeError {
    fn from(e: ParseError) -> Self {
        Self::Parse(e)
    }
}

impl From<FormatError> for DecodeError {
    fn from(e: FormatError) -> Self {
        Self::Format(e)
    }
}

impl From<SegmentError> for DecodeError {
    fn from(e: SegmentError) -> Self {
        Self::Segment(e)
    }
}

impl From<HuffmanError> for DecodeError {
    fn from(e: HuffmanError) -> Self {
        Self::Huffman(e)
    }
}

impl From<RegionError> for DecodeError {
    fn from(e: RegionError) -> Self {
        Self::Region(e)
    }
}

impl From<TemplateError> for DecodeError {
    fn from(e: TemplateError) -> Self {
        Self::Template(e)
    }
}

impl From<SymbolError> for DecodeError {
    fn from(e: SymbolError) -> Self {
        Self::Symbol(e)
    }
}

impl From<OverflowError> for DecodeError {
    fn from(e: OverflowError) -> Self {
        Self::Overflow(e)
    }
}

/// Result type for JBIG2 decoding operations.
pub type Result<T> = core::result::Result<T, DecodeError>;

macro_rules! bail {
    ($err:expr) => {
        return Err($err.into())
    };
}

macro_rules! err {
    ($err:expr) => {
        Err($err.into())
    };
}

pub(crate) use bail;
pub(crate) use err;
