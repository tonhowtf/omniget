//! Large-file stress fixtures (#216, #375, #5107), generated at test time and never committed.
//!
//! Two kinds:
//!
//! * [`Streamed`]: multi-gigabyte PSD/PSB files written straight from the Adobe PSD spec, row by
//!   row, independent of [`PsdBuilder`] (so they don't inherit its limits, and the generator needs
//!   a few megabytes of memory however large the file). One full-canvas layer (or none) plus the
//!   merged image; every sample follows [`sample`], so a reader can check any pixel without the
//!   file. [`PSD_2GB`] is the largest file Photoshop saves as PSD; [`PSB_CASES`] are 2.5–5 GB.
//! * [`Grid`]: layered documents built with [`PsdBuilder`]: a background and a grid of layers in
//!   groups, with masks, a hidden layer, a closed group and drop shadows. [`LAYERED_20MP`] is the
//!   20 MP photo-sized document, [`LAYOUT_150`] the 150-layer A4 layout. [`Grid::expected`] is
//!   the merged colour at any point away from the shadows ([`Grid::sample_points`] lists some).
//!
//! Photoshop's limits, which the presets respect: a PSD is at most 30,000 px a side and 2 GB; a
//! PSB at most 300,000 px a side.

use std::io::{self, Write};

use crate::builder::{GroupSpec, LayerSpec, MaskSpec, PixelData, PsdBuilder};
use crate::descriptor::{Descriptor, Id, Value, VersionedDescriptor};
use crate::error::Result;
use crate::file::PsdFile;
use crate::header::Version;
use crate::layer::Rect;
use crate::tagged::TaggedBlock;

/// Photoshop's PSD size cap: files of 2 GB or more must be PSB.
pub const PSD_MAX_FILE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Photoshop's PSD side cap (px).
pub const PSD_MAX_SIDE: u32 = 30_000;

/// The 8-bit sample of channel `c` (0..3 colour, 3 = alpha) at (`x`, `y`) in a [`Streamed`] file.
/// RLE files ignore `x` (constant rows compress to almost nothing). 16-bit files store `v * 257`.
pub fn sample(x: u32, y: u32, c: u32, rle: bool) -> u8 {
    if c == 3 {
        return 255;
    }
    let x = if rle { 0 } else { x };
    (x.wrapping_mul(7).wrapping_add(y.wrapping_mul(13)).wrapping_add(c.wrapping_mul(101)) & 0xff) as u8
}

/// A large RGB file written row by row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Streamed {
    /// File stem, e.g. `layer8-25k-raw`.
    pub name: &'static str,
    /// PSD (v1) or PSB (v2).
    pub version: Version,
    /// Width (px).
    pub width: u32,
    /// Height (px).
    pub height: u32,
    /// 8 or 16.
    pub depth: u16,
    /// RLE (PackBits) or raw.
    pub rle: bool,
    /// One full-canvas RGBA layer, or a flattened file (merged image only).
    pub layered: bool,
}

const fn psb(name: &'static str, side: u32, depth: u16, rle: bool, layered: bool) -> Streamed {
    Streamed { name, version: Version::Psb, width: side, height: side, depth, rle, layered }
}

/// A 2.02 GB layered PSD just under Photoshop's 2 GB cap: 17,000² px, one raw full-canvas layer
/// (289 MB per channel) and the raw merged image.
pub const PSD_2GB: Streamed =
    Streamed { name: "psd-layer8-17k-raw", version: Version::Psd, width: 17_000, height: 17_000, depth: 8, rle: false, layered: true };

/// The 4–5 GB PSB generated for #375 and #216.
pub const PSB_CASES: [Streamed; 5] = [
    // Composite of 2.52 GB in one decode.
    psb("flat8-29k-rle", 29_000, 8, true, false),
    // A 4.8 GB file: offsets past 4 GiB, composite of 4.8 GB.
    psb("flat8-40k-raw", 40_000, 8, false, false),
    // 4.38 GB file: layer channels of 625 MB, composite of 1.9 GB, data past 4 GiB.
    PSB_4GB,
    // Layer channels of 900 MB, composite of 2.7 GB.
    psb("layer8-30k-rle", 30_000, 8, true, true),
    // Layer channels of 2.18 GB each.
    psb("layer16-33k-rle", 33_000, 16, true, true),
];

/// The layered 4.38 GB PSB (25,000² px, raw): layer data and the merged image past 4 GiB.
pub const PSB_4GB: Streamed = psb("layer8-25k-raw", 25_000, 8, false, true);

/// PackBits for a row of `n` equal bytes: runs of 128, then the remainder.
fn rle_row(n: u64, b: u8) -> Vec<u8> {
    let mut v = Vec::new();
    let mut left = n;
    while left > 0 {
        let run = left.min(128);
        // `run` is 1..=128, so 257 - run is 129..=256 and only 256 (run 1) needs the literal form.
        if run == 1 {
            v.extend_from_slice(&[0, b]);
        } else {
            v.extend_from_slice(&[(257 - run) as u8, b]);
        }
        left -= run;
    }
    v
}

fn too_big(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, format!("{what} does not fit a PSD (v1) length field"))
}

impl Streamed {
    /// File name with the right extension.
    pub fn file_name(&self) -> String {
        format!("{}.{}", self.name, if self.version == Version::Psb { "psb" } else { "psd" })
    }

    fn psb(&self) -> bool {
        self.version == Version::Psb
    }

    fn row_bytes(&self) -> u64 {
        u64::from(self.width) * u64::from(self.depth / 8)
    }

    /// Bytes of one RLE row (all rows have the same length).
    fn rle_len(&self) -> u64 {
        rle_row(self.row_bytes(), 0).len() as u64
    }

    /// Size of a channel block (compression word, RLE counts, rows) of `planes` planes.
    fn channel_len(&self, planes: u64) -> u64 {
        let rows = planes * u64::from(self.height);
        let count = if self.psb() { 4 } else { 2 };
        if self.rle { 2 + rows * count + rows * self.rle_len() } else { 2 + rows * self.row_bytes() }
    }

    /// Writes a length field: 8 bytes in PSB, 4 in PSD.
    fn put_len(&self, out: &mut Vec<u8>, v: u64, what: &str) -> io::Result<()> {
        if self.psb() {
            out.extend_from_slice(&v.to_be_bytes());
        } else {
            out.extend_from_slice(&u32::try_from(v).map_err(|_| too_big(what))?.to_be_bytes());
        }
        Ok(())
    }

    /// The layer record of the one layer: transparency (-1), then R, G, B.
    fn layer_record(&self) -> io::Result<Vec<u8>> {
        let ch_len = self.channel_len(1);
        let name = b"Layer 1";
        let mut rec = Vec::new();
        for v in [0, 0, self.height, self.width] {
            rec.extend_from_slice(&v.to_be_bytes());
        }
        rec.extend_from_slice(&4u16.to_be_bytes());
        for id in [-1i16, 0, 1, 2] {
            rec.extend_from_slice(&id.to_be_bytes());
            self.put_len(&mut rec, ch_len, "a layer channel")?;
        }
        rec.extend_from_slice(b"8BIMnorm");
        rec.extend_from_slice(&[255, 0, 0, 0]);
        // Extra data: no mask, no blending ranges, the Pascal name padded to 4.
        let mut extra = Vec::new();
        extra.extend_from_slice(&0u32.to_be_bytes());
        extra.extend_from_slice(&0u32.to_be_bytes());
        extra.push(name.len() as u8);
        extra.extend_from_slice(name);
        while extra.len() % 4 != 0 {
            extra.push(0);
        }
        rec.extend_from_slice(&(extra.len() as u32).to_be_bytes());
        rec.extend_from_slice(&extra);
        Ok(rec)
    }

    /// The header, the empty colour-mode and resource sections and the layer section's head
    /// (everything before the first channel's data), plus the padding after the layer channels.
    fn head(&self) -> io::Result<(Vec<u8>, u64)> {
        let mut h = Vec::new();
        h.extend_from_slice(b"8BPS");
        h.extend_from_slice(&(if self.psb() { 2u16 } else { 1 }).to_be_bytes());
        h.extend_from_slice(&[0; 6]);
        h.extend_from_slice(&3u16.to_be_bytes());
        h.extend_from_slice(&self.height.to_be_bytes());
        h.extend_from_slice(&self.width.to_be_bytes());
        h.extend_from_slice(&self.depth.to_be_bytes());
        h.extend_from_slice(&3u16.to_be_bytes());
        // Colour mode data and image resources: empty.
        h.extend_from_slice(&0u32.to_be_bytes());
        h.extend_from_slice(&0u32.to_be_bytes());
        if !self.layered {
            self.put_len(&mut h, 0, "the layer section")?;
            return Ok((h, 0));
        }
        let rec = self.layer_record()?;
        let mut layer_info = 2 + rec.len() as u64 + 4 * self.channel_len(1);
        let pad = layer_info % 2;
        layer_info += pad;
        let len_field = if self.psb() { 8 } else { 4 };
        // Layer and mask information: layer info (with its length) + empty global mask (4 bytes).
        self.put_len(&mut h, len_field + layer_info + 4, "the layer section")?;
        self.put_len(&mut h, layer_info, "the layer info")?;
        h.extend_from_slice(&1i16.to_be_bytes());
        h.extend_from_slice(&rec);
        Ok((h, pad))
    }

    /// The exact size of the file [`write_to`](Self::write_to) writes, without writing it.
    pub fn file_len(&self) -> io::Result<u64> {
        let (head, pad) = self.head()?;
        let layers = if self.layered { 4 * self.channel_len(1) + pad + 4 } else { 0 };
        Ok(head.len() as u64 + layers + self.channel_len(3))
    }

    /// The bytes of one row of channel `ch`.
    fn row(&self, y: u32, ch: u32, buf: &mut Vec<u8>) {
        buf.clear();
        if self.rle {
            buf.extend(rle_row(self.row_bytes(), sample(0, y, ch, true)));
            return;
        }
        for x in 0..self.width {
            let s = sample(x, y, ch, false);
            buf.push(s);
            if self.depth == 16 {
                buf.push(s);
            }
        }
    }

    /// Writes the compression word, the counts and the rows of `chans`.
    fn write_planes(&self, w: &mut impl Write, chans: &[u32]) -> io::Result<()> {
        w.write_all(&u16::from(self.rle).to_be_bytes())?;
        if self.rle {
            let n = self.rle_len();
            let one = if self.psb() {
                u32::try_from(n).map_err(|_| too_big("an RLE row"))?.to_be_bytes().to_vec()
            } else {
                u16::try_from(n).map_err(|_| too_big("an RLE row"))?.to_be_bytes().to_vec()
            };
            for _ in 0..chans.len() as u64 * u64::from(self.height) {
                w.write_all(&one)?;
            }
        }
        let mut buf = Vec::new();
        for &ch in chans {
            for y in 0..self.height {
                self.row(y, ch, &mut buf);
                w.write_all(&buf)?;
            }
        }
        Ok(())
    }

    /// Streams the file into `w` (buffer it). Returns the bytes written. A PSD whose lengths
    /// don't fit its 32-bit fields is refused before anything is written.
    pub fn write_to(&self, w: &mut impl Write) -> io::Result<u64> {
        let len = self.file_len()?;
        let (head, pad) = self.head()?;
        w.write_all(&head)?;
        if self.layered {
            // Transparency, then R, G, B.
            for ch in [3, 0, 1, 2] {
                self.write_planes(w, &[ch])?;
            }
            w.write_all(&vec![0; pad as usize])?;
            w.write_all(&0u32.to_be_bytes())?;
        }
        self.write_planes(w, &[0, 1, 2])?;
        w.flush()?;
        Ok(len)
    }

    /// Writes the file to `path` (created or truncated). Returns its size.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn write_file(&self, path: &std::path::Path) -> io::Result<u64> {
        let mut w = io::BufWriter::with_capacity(8 << 20, std::fs::File::create(path)?);
        self.write_to(&mut w)
    }

    /// The 8-bit value of channel `c` (3 = alpha) at (`x`, `y`): the layer's and the merged
    /// image's (they are the same).
    pub fn sample(&self, x: u32, y: u32, c: u32) -> u8 {
        sample(x, y, c, self.rle)
    }
}

/// A layered document: a background and a grid of layers, one group per grid row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grid {
    /// Canvas width (px).
    pub width: u32,
    /// Canvas height (px).
    pub height: u32,
    /// Layers in the grid (the background is extra).
    pub layers: u32,
    /// Grid columns; a group per row of `cols` layers.
    pub cols: u32,
}

/// A 20 MP (5472 × 3648) layered photo: background, 5 layers in 2 groups (one closed), a
/// mask and a drop shadow.
pub const LAYERED_20MP: Grid = Grid { width: 5472, height: 3648, layers: 5, cols: 3 };

/// A 150-layer A4 layout at 300 ppi (2480 × 3508): background + 149 layers in 15 groups, with
/// masks, hidden layers, closed groups and drop shadows.
pub const LAYOUT_150: Grid = Grid { width: 2480, height: 3508, layers: 149, cols: 10 };

/// What a grid layer carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridLayer {
    /// Bounds in document coordinates.
    pub rect: Rect,
    /// Base colour; the layer adds its local x to it (a horizontal ramp).
    pub base: [u8; 3],
    /// Has a mask showing only the left half.
    pub masked: bool,
    /// Hidden.
    pub hidden: bool,
    /// Has a drop shadow (`lfx2`).
    pub shadow: bool,
}

/// The `lfx2` descriptor of a drop shadow: multiply, black, 75 %, 120°, distance 5, size 5.
fn drop_shadow() -> Vec<u8> {
    let px = |v: f64| Value::UnitFloat { unit: *b"#Pxl", value: v };
    let color = Descriptor::new("RGBC").with("Rd  ", Value::Double(0.0)).with("Grn ", Value::Double(0.0)).with("Bl  ", Value::Double(0.0));
    let shadow = Descriptor::new("DrSh")
        .with("enab", Value::Boolean(true))
        .with("Md  ", Value::Enumerated { type_id: Id::new("BlnM"), value: Id::new("Mltp") })
        .with("Clr ", Value::Descriptor(color))
        .with("Opct", Value::UnitFloat { unit: *b"#Prc", value: 75.0 })
        .with("uglg", Value::Boolean(false))
        .with("lagl", Value::UnitFloat { unit: *b"#Ang", value: 120.0 })
        .with("Dstn", px(5.0))
        .with("Ckmt", px(0.0))
        .with("blur", px(5.0));
    let root = Descriptor::new("null")
        .with("Scl ", Value::UnitFloat { unit: *b"#Prc", value: 100.0 })
        .with("masterFXSwitch", Value::Boolean(true))
        .with("DrSh", Value::Descriptor(shadow));
    let mut data = 0u32.to_be_bytes().to_vec();
    data.extend(VersionedDescriptor::new(root).to_bytes());
    data
}

impl Grid {
    /// Grid rows (= groups).
    pub fn rows(&self) -> u32 {
        self.layers.div_ceil(self.cols.max(1))
    }

    fn cell(&self) -> (u32, u32) {
        (self.width / self.cols.max(1), self.height / self.rows().max(1))
    }

    /// Gap between a layer and its cell's edge (larger than any shadow's reach).
    fn inset(&self) -> u32 {
        let (cw, ch) = self.cell();
        (cw.min(ch) / 8).max(1)
    }

    /// Layer `i` of the grid (0-based, bottom to top), or `None` past the end.
    pub fn layer(&self, i: u32) -> Option<GridLayer> {
        if i >= self.layers {
            return None;
        }
        let (cw, ch) = self.cell();
        let inset = self.inset();
        let (col, row) = (i % self.cols.max(1), i / self.cols.max(1));
        let rect =
            Rect::from_xywh((col * cw + inset) as i32, (row * ch + inset) as i32, cw.saturating_sub(2 * inset).max(1), ch.saturating_sub(2 * inset).max(1));
        let masked = i % 5 == 1;
        let hidden = i % 13 == 12;
        Some(GridLayer {
            rect,
            base: [(i * 37 + 20) as u8, (i * 91 + 50) as u8, (i * 53 + 90) as u8],
            masked,
            hidden,
            shadow: i % 7 == 3 && !masked && !hidden,
        })
    }

    /// Name of grid layer `i`.
    pub fn layer_name(i: u32) -> String {
        format!("Layer {}", i + 1)
    }

    /// Name of the group of grid row `r`.
    pub fn group_name(r: u32) -> String {
        format!("Row {}", r + 1)
    }

    /// Groups of rows `r % 4 == 3` are closed in the layers panel.
    pub fn group_open(r: u32) -> bool {
        r % 4 != 3
    }

    /// The background's colour at (`x`, `y`): [`sample`] in runs of 16 px (RLE has runs to find).
    pub fn background(x: u32, y: u32) -> [u8; 3] {
        [0, 1, 2].map(|c| sample(x / 16, y, c, false))
    }

    /// Grid layer `i`'s pixel colour at document (`x`, `y`), ignoring visibility and mask.
    fn layer_color(l: &GridLayer, x: u32) -> [u8; 3] {
        let lx = x.wrapping_sub(l.rect.left as u32);
        l.base.map(|b| (u32::from(b) + lx) as u8)
    }

    /// The merged colour at (`x`, `y`). Exact except within a few pixels of a shadowed layer's
    /// edges (the merged image this generator stores leaves shadows out).
    pub fn expected(&self, x: u32, y: u32) -> [u8; 3] {
        let (cw, ch) = self.cell();
        let (col, row) = (x / cw.max(1), y / ch.max(1));
        if col < self.cols
            && let Some(l) = self.layer(row * self.cols + col)
        {
            let (xi, yi) = (x as i32, y as i32);
            let inside = xi >= l.rect.left && xi < l.rect.right && yi >= l.rect.top && yi < l.rect.bottom;
            let shown = !l.masked || i64::from(xi) < i64::from(l.rect.left) + l.rect.width() / 2;
            if inside && shown && !l.hidden {
                return Self::layer_color(&l, x);
            }
        }
        Self::background(x, y)
    }

    /// Points to check, away from every shadow: in each of a few layers, its shown half, its
    /// masked-out half, and the gap at its cell's corner.
    pub fn sample_points(&self) -> Vec<(u32, u32)> {
        let step = (self.layers / 12).max(1);
        let mut v = vec![(0, 0), (self.width - 1, self.height - 1)];
        for i in (0..self.layers).step_by(step as usize) {
            let Some(l) = self.layer(i) else { continue };
            let (w, h) = (l.rect.width() as u32, l.rect.height() as u32);
            let (x0, y0) = (l.rect.left as u32, l.rect.top as u32);
            v.push((x0 + w / 4, y0 + h / 2));
            v.push((x0 + w * 3 / 4, y0 + h / 2));
            v.push((x0 - self.inset() / 2, y0 - self.inset() / 2));
        }
        v
    }

    /// Builds the file (8-bit RGB, RLE): layers bottom to top, the merged image from
    /// [`expected`](Self::expected).
    pub fn build(&self, version: Version) -> Result<PsdFile> {
        let (w, h) = (self.width, self.height);
        let mut b = PsdBuilder::new(w, h).version(version);
        let mut bg = Vec::with_capacity(w as usize * h as usize * 4);
        for y in 0..h {
            for x in 0..w {
                bg.extend(Self::background(x, y));
                bg.push(255);
            }
        }
        b.push_layer(LayerSpec::new("Background", 0, 0, w, h, PixelData::Rgba8(bg)));
        for row in 0..self.rows() {
            b.begin_group(GroupSpec { open: Self::group_open(row), ..GroupSpec::new(Self::group_name(row)) });
            for i in row * self.cols..((row + 1) * self.cols).min(self.layers) {
                let Some(l) = self.layer(i) else { continue };
                let (lw, lh) = (l.rect.width() as u32, l.rect.height() as u32);
                let mut px = Vec::with_capacity(lw as usize * lh as usize * 4);
                for _ in 0..lh {
                    for lx in 0..lw {
                        px.extend(Self::layer_color(&l, l.rect.left as u32 + lx));
                        px.push(255);
                    }
                }
                let mut spec = LayerSpec::new(Self::layer_name(i), l.rect.left, l.rect.top, lw, lh, PixelData::Rgba8(px));
                spec.visible = !l.hidden;
                if l.masked {
                    let data = (0..lh).flat_map(|_| (0..lw).map(move |x| if x < lw / 2 { 255 } else { 0 })).collect();
                    spec.mask = Some(MaskSpec { rect: l.rect, data, default_color: 0, disabled: false });
                }
                if l.shadow {
                    spec.extra_blocks.push(TaggedBlock::new(*b"lfx2", drop_shadow()));
                }
                b.push_layer(spec);
            }
            b.end_group()?;
        }
        let mut merged = Vec::with_capacity(w as usize * h as usize * 4);
        for y in 0..h {
            for x in 0..w {
                merged.extend(self.expected(x, y));
                merged.push(255);
            }
        }
        b.composite(PixelData::Rgba8(merged));
        b.build()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn presets_respect_photoshop_limits() {
        let psd = PSD_2GB.file_len().unwrap();
        assert!(psd < PSD_MAX_FILE_BYTES && psd > 2_000_000_000, "{psd}");
        const { assert!(PSD_2GB.width <= PSD_MAX_SIDE && PSD_2GB.height <= PSD_MAX_SIDE) };
        for c in PSB_CASES {
            assert!(c.file_len().is_ok() && c.width <= 300_000, "{}", c.name);
        }
        assert!(PSB_4GB.file_len().unwrap() > 4 << 30);
        // A PSD too big for its 32-bit lengths is refused, not wrapped.
        let huge = Streamed { width: 33_000, height: 33_000, ..PSD_2GB };
        assert!(huge.file_len().is_err());
        assert!(huge.write_to(&mut io::sink()).is_err());
    }

    /// Small streamed files parse, and their pixels are [`sample`]'s.
    #[test]
    fn small_streamed_files_parse() {
        for version in [Version::Psd, Version::Psb] {
            for (rle, layered, depth) in [(false, true, 8), (true, true, 8), (true, false, 16), (false, false, 8)] {
                let c = Streamed { name: "t", version, width: 300, height: 7, depth, rle, layered };
                let mut out = Vec::new();
                let n = c.write_to(&mut out).unwrap();
                assert_eq!(n, out.len() as u64, "{c:?}");
                let file = PsdFile::from_bytes(&out).unwrap();
                let merged = file.image_data.decode(&file.header).unwrap();
                let bps = usize::from(depth / 8);
                let plane = 300 * 7 * bps;
                for (x, y, ch) in [(0u32, 0u32, 0u32), (299, 6, 2), (150, 3, 1)] {
                    let i = ch as usize * plane + (y as usize * 300 + x as usize) * bps;
                    assert_eq!(merged[i], c.sample(x, y, ch), "{c:?} ({x}, {y}) {ch}");
                }
                assert_eq!(file.layers().len(), usize::from(layered), "{c:?}");
            }
        }
    }

    #[test]
    fn grid_layout_is_consistent() {
        for g in [LAYERED_20MP, LAYOUT_150] {
            let last = g.layer(g.layers - 1).unwrap();
            assert!(last.rect.right <= g.width as i32 && last.rect.bottom <= g.height as i32);
            assert!(g.layer(g.layers).is_none());
            let all: Vec<_> = (0..g.layers).filter_map(|i| g.layer(i)).collect();
            assert!(all.iter().any(|l| l.masked) && all.iter().any(|l| l.shadow));
            for (x, y) in g.sample_points() {
                assert!(x < g.width && y < g.height);
            }
        }
        assert!((0..LAYOUT_150.layers).filter_map(|i| LAYOUT_150.layer(i)).any(|l| l.hidden));
    }

    /// A small grid builds, parses, and its merged image is [`Grid::expected`].
    #[test]
    fn small_grid_builds() {
        let g = Grid { width: 120, height: 90, layers: 14, cols: 4 };
        let file = g.build(Version::Psd).unwrap();
        let bytes = file.to_bytes().unwrap();
        let file = PsdFile::from_bytes(&bytes).unwrap();
        // Background + 14 layers + 4 groups (two records each).
        assert_eq!(file.layers().len(), 1 + 14 + 8);
        let merged = file.image_data.decode(&file.header).unwrap();
        for (x, y) in g.sample_points() {
            let want = g.expected(x, y);
            for c in 0..3 {
                assert_eq!(merged[c * 120 * 90 + (y * 120 + x) as usize], want[c], "({x}, {y}) {c}");
            }
        }
    }
}
