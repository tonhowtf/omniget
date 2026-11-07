//! # photocraft-psd
//!
//! Faithful, lossless reader and writer for Adobe Photoshop PSD (version 1)
//! and PSB (version 2, "large document format") files.
//!
//! * [`PsdFile::from_bytes`] parses a file into a format-level model;
//!   [`PsdFile::to_bytes`] writes it back. Unmodified files round-trip byte
//!   for byte: channel data is kept in its original encoding, unknown tagged
//!   blocks and resources are kept raw, and padding is preserved.
//! * Channel data is decoded lazily ([`ChannelData::decode`],
//!   [`ImageData::decode`]) with all four compression methods supported for
//!   both decoding and encoding.
//! * [`PsdBuilder`] constructs new files from RGBA/gray/CMYK buffers; the
//!   caller supplies the merged composite (this crate never composites).
//! * [`descriptor`] parses and writes ActionDescriptors.
//!
//! Implemented clean-room from Adobe's public "Photoshop File Formats
//! Specification". Undocumented details that follow MIT-licensed psd-tools
//! or ag-psd behavior are noted in comments.
//!
//! The parser never panics on malformed input and enforces limits
//! (dimensions ≤ 300000, bounded allocations, bounded descriptor nesting).
//!
//! ```
//! use photocraft_psd::{LayerSpec, PixelData, PsdBuilder, PsdFile};
//!
//! let mut b = PsdBuilder::new(2, 1);
//! b.push_layer(LayerSpec::new("Red", 0, 0, 2, 1, PixelData::Rgba8(vec![255, 0, 0, 255, 255, 0, 0, 128])));
//! b.composite(PixelData::Rgba8(vec![255, 0, 0, 255, 255, 0, 0, 128]));
//! let bytes = b.to_bytes()?;
//!
//! let file = PsdFile::from_bytes(&bytes)?;
//! assert_eq!(file.to_bytes()?, bytes); // byte-stable
//! let layer = file.layer(0).unwrap();
//! assert_eq!(layer.name(), "Red");
//! assert_eq!(layer.rgba8()?.data[7], 128);
//! # Ok::<(), photocraft_psd::PsdError>(())
//! ```
#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod blend;
pub mod builder;
pub mod compression;
pub mod descriptor;
pub mod error;
pub mod file;
pub mod filter_effects;
#[cfg(any(test, feature = "testgen"))]
pub mod fixtures;
pub mod header;
pub mod image_data;
mod io;
pub mod layer;
mod lazy;
pub mod pixels;
pub mod resources;
pub mod rows;
pub mod source;
pub mod tagged;
#[cfg(any(test, feature = "testgen"))]
// Test-data generator (tests and fuzz seeds only): failing loudly is the point.
#[allow(clippy::expect_used)]
pub mod testgen;
pub mod tree;

pub use blend::BlendMode;
pub use builder::{GroupSpec, LayerSpec, MaskSpec, PixelData, PsdBuilder};
pub use compression::{Compression, PlaneLayout};
pub use descriptor::{Descriptor, VersionedDescriptor};
pub use error::{PsdError, Result};
pub use file::{GlobalLayerMask, LayerInfoPlacement, PsdFile};
pub use header::{ColorMode, Header, Version};
pub use image_data::ImageData;
pub use layer::{BlendingRanges, ChannelData, LayerFlags, LayerInfo, LayerMask, LayerRecord, MaskData, MaskParameters, RealMask, Rect};
pub use pixels::{GrayImage, Layer, RgbaImage};
pub use resources::{ImageResource, ResolutionInfo, ResourceData};
pub use rows::RowDecoder;
#[cfg(any(unix, windows))]
pub use source::FileSource;
pub use source::{ReadAt, Source, Stored};
pub use tagged::{BlockData, SectionDivider, SectionType, TaggedBlock};
pub use tree::LayerNode;
