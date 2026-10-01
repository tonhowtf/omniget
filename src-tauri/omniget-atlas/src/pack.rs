//! Deterministic packer: source PNGs -> atlas pages + `atlas.json`.
//!
//! Determinism is a hard requirement, not a nicety: the art is regenerated often
//! and a byte-identical output is what keeps a rebuild out of the diff. So every
//! map is ordered, the placement order is a total order over the frame ids, the
//! PNG encoder is pinned to one compression and filter setting, and nothing
//! writes a timestamp.
//!
//! Sources are two shapes:
//! * character: `<src>/<sheet>/<anim>/<DIR>/<n>.png` plus an optional
//!   `manifest.json` (fps, loop, pivot and z_base per animation);
//! * tiles: `<src>/{floor,wall,object}/<name>.png` plus a required `tiles.json`.
//!   The sheet name is the folder name with a trailing `-src` removed.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use image::{ImageEncoder, Rgba, RgbaImage};
use serde::Deserialize;

use crate::schema::{
    mirror_source, to_json, Anim, Atlas, Dirs, Frame, Tile, DEFAULT_EXTRUDE, DEFAULT_FPS,
    DEFAULT_PADDING, PAGE_MAX, UNIQUE_DIRS,
};
use crate::skyline::Skyline;

#[derive(Debug, Clone, Copy)]
pub struct PackOptions {
    /// Maximum width and height of a page, in pixels.
    pub page_max: u32,
    /// Transparent gutter between two frames.
    pub padding: u32,
    /// Edge pixels duplicated around each frame.
    pub extrude: u32,
}

impl Default for PackOptions {
    fn default() -> Self {
        Self {
            page_max: PAGE_MAX,
            padding: DEFAULT_PADDING,
            extrude: DEFAULT_EXTRUDE,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PackError {
    #[error("ERR_ATLAS_SRC_IO: {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("ERR_ATLAS_SRC_IMAGE: {path}: {message}")]
    Image { path: String, message: String },
    #[error("ERR_ATLAS_SRC_JSON: {path}: {message}")]
    Json { path: String, message: String },
    #[error("ERR_ATLAS_SRC_EMPTY: {path}: no frame found")]
    Empty { path: String },
    #[error("ERR_ATLAS_SRC_FRAME_NAME: {path}: frame files must be named <n>.png")]
    FrameName { path: String },
    #[error("ERR_ATLAS_MIRROR_SHEET: {anim}: direction {dir} has its own art; it must be a mirror of {source_dir}")]
    MirrorSheet {
        anim: String,
        dir: String,
        source_dir: String,
    },
    #[error("ERR_ATLAS_MISSING_DIR: {anim}: direction {dir} is missing (S, SW, W, NW and N are required)")]
    MissingDir { anim: String, dir: String },
    #[error("ERR_ATLAS_FRAME_TOO_BIG: {id}: {w}x{h} does not fit a {page_max}x{page_max} page with padding and extrusion")]
    FrameTooBig {
        id: String,
        w: u32,
        h: u32,
        page_max: u32,
    },
    #[error("ERR_ATLAS_DUPLICATE_FRAME: {id} comes from two sources")]
    DuplicateFrame { id: String },
    #[error("ERR_ATLAS_SRC_LAYOUT: {message}")]
    Layout { message: String },
}

impl PackError {
    /// Stable `ERR_ATLAS_*` code, the part the UI and the CI can match on.
    pub fn code(&self) -> &'static str {
        match self {
            PackError::Io { .. } => "ERR_ATLAS_SRC_IO",
            PackError::Image { .. } => "ERR_ATLAS_SRC_IMAGE",
            PackError::Json { .. } => "ERR_ATLAS_SRC_JSON",
            PackError::Empty { .. } => "ERR_ATLAS_SRC_EMPTY",
            PackError::FrameName { .. } => "ERR_ATLAS_SRC_FRAME_NAME",
            PackError::MirrorSheet { .. } => "ERR_ATLAS_MIRROR_SHEET",
            PackError::MissingDir { .. } => "ERR_ATLAS_MISSING_DIR",
            PackError::FrameTooBig { .. } => "ERR_ATLAS_FRAME_TOO_BIG",
            PackError::DuplicateFrame { .. } => "ERR_ATLAS_DUPLICATE_FRAME",
            PackError::Layout { .. } => "ERR_ATLAS_SRC_LAYOUT",
        }
    }
}

type Result<T> = std::result::Result<T, PackError>;

#[derive(Debug, Clone)]
pub struct PackReport {
    pub atlas_path: PathBuf,
    pub page_paths: Vec<PathBuf>,
    pub frame_count: usize,
    pub anim_count: usize,
    pub tile_count: usize,
    /// Fraction of the pages' pixels covered by frames, 0.0 to 1.0.
    pub fill: f32,
}

// ---------------------------------------------------------------- source model

#[derive(Debug, Default, Deserialize)]
struct Manifest {
    #[serde(default)]
    default_fps: Option<u32>,
    #[serde(default)]
    anims: BTreeMap<String, AnimMeta>,
}

#[derive(Debug, Default, Clone, Deserialize)]
struct AnimMeta {
    #[serde(default)]
    fps: Option<u32>,
    #[serde(default, rename = "loop")]
    looping: Option<bool>,
    #[serde(default)]
    pivot: Option<[i32; 2]>,
    #[serde(default)]
    z_base: Option<i32>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum TilesFile {
    Wrapped { tiles: BTreeMap<String, TileMeta> },
    Bare(BTreeMap<String, TileMeta>),
}

impl TilesFile {
    fn into_map(self) -> BTreeMap<String, TileMeta> {
        match self {
            TilesFile::Wrapped { tiles } => tiles,
            TilesFile::Bare(map) => map,
        }
    }
}

#[derive(Debug, Default, Clone, Deserialize)]
struct TileMeta {
    #[serde(default)]
    height: Option<u32>,
    #[serde(default)]
    occludes: Option<bool>,
    #[serde(default)]
    footprint: Option<[u32; 2]>,
    #[serde(default)]
    walkable: Option<bool>,
    #[serde(default)]
    pivot: Option<[i32; 2]>,
    #[serde(default)]
    z_base: Option<i32>,
}

/// One frame ready to be placed on a page.
struct Entry {
    id: String,
    pivot: [i32; 2],
    z_base: i32,
    image: RgbaImage,
}

#[derive(Default)]
struct Collected {
    entries: Vec<Entry>,
    anims: BTreeMap<String, Anim>,
    tiles: BTreeMap<String, Tile>,
    first_sheet: Option<String>,
}

// ------------------------------------------------------------------- public API

/// Packs every source folder into `out_dir`, writing `<sheet>-<n>.png` pages and
/// `atlas.json`. The page name prefix comes from the first sheet seen.
pub fn pack(sources: &[PathBuf], out_dir: &Path, opts: &PackOptions) -> Result<PackReport> {
    if sources.is_empty() {
        return Err(PackError::Layout {
            message: "no source folder given".into(),
        });
    }
    let mut collected = Collected::default();
    for src in sources {
        if src.join("tiles.json").is_file() {
            collect_tiles(src, &mut collected)?;
        } else {
            collect_character(src, &mut collected)?;
        }
    }
    if collected.entries.is_empty() {
        return Err(PackError::Empty {
            path: display(&sources[0]),
        });
    }
    let prefix = collected
        .first_sheet
        .clone()
        .unwrap_or_else(|| "atlas".into());
    write_atlas(collected, &prefix, out_dir, opts)
}

// -------------------------------------------------------------------- scanning

fn collect_character(src: &Path, out: &mut Collected) -> Result<()> {
    let manifest = read_manifest(src)?;
    let sheets = child_dirs(src)?;
    if sheets.is_empty() {
        return Err(PackError::Layout {
            message: format!("{}: expected <sheet>/<anim>/<DIR>/<n>.png", display(src)),
        });
    }
    for sheet_dir in sheets {
        let sheet = file_name(&sheet_dir);
        if out.first_sheet.is_none() {
            out.first_sheet = Some(sheet.clone());
        }
        for anim_dir in child_dirs(&sheet_dir)? {
            let anim_name = file_name(&anim_dir);
            let anim_key = format!("{sheet}/{anim_name}");
            let meta = manifest
                .anims
                .get(&anim_key)
                .or_else(|| manifest.anims.get(&anim_name))
                .cloned()
                .unwrap_or_default();

            let present: BTreeSet<String> = child_dirs(&anim_dir)?
                .iter()
                .map(|p| file_name(p))
                .collect();
            for dir in present.iter() {
                if let Some(source_dir) = mirror_source(dir) {
                    return Err(PackError::MirrorSheet {
                        anim: anim_key.clone(),
                        dir: dir.clone(),
                        source_dir: source_dir.into(),
                    });
                }
                if !UNIQUE_DIRS.contains(&dir.as_str()) {
                    return Err(PackError::Layout {
                        message: format!("{}: '{dir}' is not a direction", display(&anim_dir)),
                    });
                }
            }
            let mut lists: Vec<Vec<String>> = Vec::with_capacity(5);
            for dir in UNIQUE_DIRS {
                if !present.contains(dir) {
                    return Err(PackError::MissingDir {
                        anim: anim_key.clone(),
                        dir: dir.into(),
                    });
                }
                let dir_path = anim_dir.join(dir);
                let files = numbered_pngs(&dir_path)?;
                let mut ids = Vec::with_capacity(files.len());
                for (n, path) in files {
                    let id = format!("{anim_key}/{dir}/{n}");
                    let image = load_png(&path)?;
                    let pivot = meta.pivot.unwrap_or_else(|| default_pivot(&image));
                    push_entry(
                        out,
                        Entry {
                            id: id.clone(),
                            pivot,
                            z_base: meta.z_base.unwrap_or(0),
                            image,
                        },
                    )?;
                    ids.push(id);
                }
                lists.push(ids);
            }
            let mut lists = lists.into_iter();
            let dirs = Dirs::from_unique(
                lists.next().unwrap_or_default(),
                lists.next().unwrap_or_default(),
                lists.next().unwrap_or_default(),
                lists.next().unwrap_or_default(),
                lists.next().unwrap_or_default(),
            );
            let fps = meta.fps.or(manifest.default_fps).unwrap_or(DEFAULT_FPS);
            out.anims.insert(
                anim_key,
                Anim {
                    fps,
                    looping: meta.looping.unwrap_or(true),
                    dirs,
                },
            );
        }
    }
    Ok(())
}

fn collect_tiles(src: &Path, out: &mut Collected) -> Result<()> {
    let sheet = file_name(src).trim_end_matches("-src").to_string();
    if out.first_sheet.is_none() {
        out.first_sheet = Some(sheet.clone());
    }
    let path = src.join("tiles.json");
    let text = std::fs::read_to_string(&path).map_err(|source| PackError::Io {
        path: display(&path),
        source,
    })?;
    let meta: BTreeMap<String, TileMeta> = serde_json::from_str::<TilesFile>(&text)
        .map_err(|e| PackError::Json {
            path: display(&path),
            message: e.to_string(),
        })?
        .into_map();

    for kind in ["floor", "wall", "object"] {
        let kind_dir = src.join(kind);
        if !kind_dir.is_dir() {
            continue;
        }
        for path in pngs(&kind_dir)? {
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let tile_key = format!("{kind}/{name}");
            let frame_id = format!("{sheet}/{kind}/{name}");
            let tile_meta = meta.get(&tile_key).cloned().unwrap_or_default();
            let image = load_png(&path)?;
            let pivot = tile_meta.pivot.unwrap_or_else(|| default_pivot(&image));
            push_entry(
                out,
                Entry {
                    id: frame_id.clone(),
                    pivot,
                    z_base: tile_meta.z_base.unwrap_or(0),
                    image,
                },
            )?;
            out.tiles.insert(
                tile_key,
                Tile {
                    frame: frame_id,
                    height: tile_meta.height.unwrap_or(0),
                    occludes: tile_meta.occludes.unwrap_or(false),
                    footprint: tile_meta.footprint.unwrap_or([1, 1]),
                    walkable: tile_meta.walkable.unwrap_or(true),
                },
            );
        }
    }
    if out.tiles.is_empty() {
        return Err(PackError::Empty { path: display(src) });
    }
    Ok(())
}

fn push_entry(out: &mut Collected, entry: Entry) -> Result<()> {
    if out.entries.iter().any(|e| e.id == entry.id) {
        return Err(PackError::DuplicateFrame { id: entry.id });
    }
    out.entries.push(entry);
    Ok(())
}

/// Ground contact point when nothing declares one: bottom centre of the frame.
fn default_pivot(image: &RgbaImage) -> [i32; 2] {
    [
        (image.width() / 2) as i32,
        image.height().saturating_sub(1) as i32,
    ]
}

fn read_manifest(src: &Path) -> Result<Manifest> {
    let path = src.join("manifest.json");
    if !path.is_file() {
        return Ok(Manifest::default());
    }
    let text = std::fs::read_to_string(&path).map_err(|source| PackError::Io {
        path: display(&path),
        source,
    })?;
    serde_json::from_str(&text).map_err(|e| PackError::Json {
        path: display(&path),
        message: e.to_string(),
    })
}

fn load_png(path: &Path) -> Result<RgbaImage> {
    let bytes = std::fs::read(path).map_err(|source| PackError::Io {
        path: display(path),
        source,
    })?;
    let image =
        image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).map_err(|e| {
            PackError::Image {
                path: display(path),
                message: e.to_string(),
            }
        })?;
    Ok(image.to_rgba8())
}

/// Sorted child directories, so the scan order never depends on the filesystem.
fn child_dirs(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out: Vec<PathBuf> = read_dir(dir)?.into_iter().filter(|p| p.is_dir()).collect();
    out.sort();
    Ok(out)
}

fn pngs(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out: Vec<PathBuf> = read_dir(dir)?
        .into_iter()
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")))
        .collect();
    out.sort();
    Ok(out)
}

/// PNGs named `<n>.png`, sorted numerically (so `10.png` follows `9.png`).
fn numbered_pngs(dir: &Path) -> Result<Vec<(u32, PathBuf)>> {
    let mut out = Vec::new();
    for path in pngs(dir)? {
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let n: u32 = stem.parse().map_err(|_| PackError::FrameName {
            path: display(&path),
        })?;
        out.push((n, path));
    }
    if out.is_empty() {
        return Err(PackError::Empty { path: display(dir) });
    }
    out.sort_by_key(|(n, _)| *n);
    Ok(out)
}

fn read_dir(dir: &Path) -> Result<Vec<PathBuf>> {
    let iter = std::fs::read_dir(dir).map_err(|source| PackError::Io {
        path: display(dir),
        source,
    })?;
    let mut out = Vec::new();
    for entry in iter {
        let entry = entry.map_err(|source| PackError::Io {
            path: display(dir),
            source,
        })?;
        out.push(entry.path());
    }
    Ok(out)
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn display(path: &Path) -> String {
    path.display().to_string()
}

// --------------------------------------------------------------------- packing

struct Placement {
    index: usize,
    page: usize,
    x: u32,
    y: u32,
}

/// Total order over the frames: tallest first, then widest, then by id. The id
/// tiebreak is what makes two runs byte-identical.
fn placement_order(entries: &[Entry]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..entries.len()).collect();
    order.sort_by(|&a, &b| {
        let ea = &entries[a];
        let eb = &entries[b];
        eb.image
            .height()
            .cmp(&ea.image.height())
            .then(eb.image.width().cmp(&ea.image.width()))
            .then(ea.id.cmp(&eb.id))
    });
    order
}

fn next_pot(v: u32) -> u32 {
    let mut p = 16u32;
    while p < v {
        p = p.saturating_mul(2);
    }
    p
}

fn write_atlas(
    collected: Collected,
    prefix: &str,
    out_dir: &Path,
    opts: &PackOptions,
) -> Result<PackReport> {
    let Collected {
        entries,
        anims,
        tiles,
        ..
    } = collected;
    let order = placement_order(&entries);
    let alloc = |e: &Entry| -> (u32, u32) {
        (
            e.image.width() + 2 * opts.extrude + opts.padding,
            e.image.height() + 2 * opts.extrude + opts.padding,
        )
    };

    for e in &entries {
        let (aw, ah) = alloc(e);
        if aw > opts.page_max || ah > opts.page_max {
            return Err(PackError::FrameTooBig {
                id: e.id.clone(),
                w: e.image.width(),
                h: e.image.height(),
                page_max: opts.page_max,
            });
        }
    }

    // Prefer one small page: try square powers of two until everything fits.
    let mut placements: Vec<Placement> = Vec::new();
    let mut pages: Vec<Skyline> = Vec::new();
    let mut size = 16u32;
    loop {
        let mut sky = Skyline::new(size, size);
        let mut ok = true;
        let mut tentative = Vec::with_capacity(order.len());
        for &i in &order {
            let (aw, ah) = alloc(&entries[i]);
            match sky.insert(aw, ah) {
                Some((x, y)) => tentative.push(Placement {
                    index: i,
                    page: 0,
                    x,
                    y,
                }),
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            placements = tentative;
            pages = vec![sky];
            break;
        }
        if size >= opts.page_max {
            break;
        }
        size = (size * 2).min(opts.page_max);
    }

    // Did not fit in one page: spill into as many full-size pages as needed.
    if pages.is_empty() {
        for &i in &order {
            let (aw, ah) = alloc(&entries[i]);
            let mut done = false;
            for (page, sky) in pages.iter_mut().enumerate() {
                if let Some((x, y)) = sky.insert(aw, ah) {
                    placements.push(Placement {
                        index: i,
                        page,
                        x,
                        y,
                    });
                    done = true;
                    break;
                }
            }
            if !done {
                let mut sky = Skyline::new(opts.page_max, opts.page_max);
                let (x, y) = sky.insert(aw, ah).ok_or_else(|| PackError::FrameTooBig {
                    id: entries[i].id.clone(),
                    w: entries[i].image.width(),
                    h: entries[i].image.height(),
                    page_max: opts.page_max,
                })?;
                placements.push(Placement {
                    index: i,
                    page: pages.len(),
                    x,
                    y,
                });
                pages.push(sky);
            }
        }
    }

    // Crop each page to the smallest power of two that still holds it.
    let page_sizes: Vec<[u32; 2]> = pages
        .iter()
        .map(|sky| {
            let w = if pages.len() == 1 {
                next_pot(sky.extent_x())
            } else {
                sky.width()
            };
            [w.max(16), next_pot(sky.extent_y()).max(16)]
        })
        .collect();

    let mut buffers: Vec<RgbaImage> = page_sizes
        .iter()
        .map(|[w, h]| RgbaImage::from_pixel(*w, *h, Rgba([0, 0, 0, 0])))
        .collect();

    let mut atlas = Atlas::empty();
    atlas.padding = Some(opts.padding);
    atlas.extrude = Some(opts.extrude);

    for p in &placements {
        let entry = &entries[p.index];
        let fx = p.x + opts.extrude;
        let fy = p.y + opts.extrude;
        blit(&mut buffers[p.page], &entry.image, fx, fy, opts.extrude);
        atlas.frames.insert(
            entry.id.clone(),
            Frame {
                page: p.page,
                x: fx,
                y: fy,
                w: entry.image.width(),
                h: entry.image.height(),
                pivot: entry.pivot,
                z_base: entry.z_base,
            },
        );
    }

    atlas.anims = anims;
    atlas.tiles = tiles;
    atlas.pages = (0..buffers.len())
        .map(|i| format!("{prefix}-{i}.png"))
        .collect();
    atlas.page_sizes = page_sizes.clone();

    std::fs::create_dir_all(out_dir).map_err(|source| PackError::Io {
        path: display(out_dir),
        source,
    })?;
    let mut page_paths = Vec::with_capacity(buffers.len());
    for (i, buffer) in buffers.iter().enumerate() {
        let path = out_dir.join(&atlas.pages[i]);
        let bytes = encode_png(buffer)?;
        std::fs::write(&path, bytes).map_err(|source| PackError::Io {
            path: display(&path),
            source,
        })?;
        page_paths.push(path);
    }

    let atlas_path = out_dir.join("atlas.json");
    let json = to_json(&atlas).map_err(|e| PackError::Json {
        path: display(&atlas_path),
        message: e.to_string(),
    })?;
    std::fs::write(&atlas_path, json).map_err(|source| PackError::Io {
        path: display(&atlas_path),
        source,
    })?;

    let total: u64 = page_sizes
        .iter()
        .map(|[w, h]| u64::from(*w) * u64::from(*h))
        .sum();
    let used: u64 = entries
        .iter()
        .map(|e| u64::from(e.image.width()) * u64::from(e.image.height()))
        .sum();
    Ok(PackReport {
        frame_count: atlas.frames.len(),
        anim_count: atlas.anims.len(),
        tile_count: atlas.tiles.len(),
        fill: if total == 0 {
            0.0
        } else {
            used as f32 / total as f32
        },
        atlas_path,
        page_paths,
    })
}

/// Copies `src` to `(x, y)` and repeats its border `extrude` pixels outwards, so
/// a linear filter sampling just outside the rect still reads the frame's own
/// colour instead of the neighbour's.
fn blit(page: &mut RgbaImage, src: &RgbaImage, x: u32, y: u32, extrude: u32) {
    let (w, h) = (src.width(), src.height());
    for sy in 0..h {
        for sx in 0..w {
            page.put_pixel(x + sx, y + sy, *src.get_pixel(sx, sy));
        }
    }
    let e = extrude as i64;
    for d in 1..=e {
        for sx in 0..w {
            let top = *src.get_pixel(sx, 0);
            let bottom = *src.get_pixel(sx, h - 1);
            put(page, x as i64 + sx as i64, y as i64 - d, top);
            put(
                page,
                x as i64 + sx as i64,
                y as i64 + h as i64 - 1 + d,
                bottom,
            );
        }
        for sy in 0..h {
            let left = *src.get_pixel(0, sy);
            let right = *src.get_pixel(w - 1, sy);
            put(page, x as i64 - d, y as i64 + sy as i64, left);
            put(
                page,
                x as i64 + w as i64 - 1 + d,
                y as i64 + sy as i64,
                right,
            );
        }
    }
    // Corners.
    for dy in 1..=e {
        for dx in 1..=e {
            put(page, x as i64 - dx, y as i64 - dy, *src.get_pixel(0, 0));
            put(
                page,
                x as i64 + w as i64 - 1 + dx,
                y as i64 - dy,
                *src.get_pixel(w - 1, 0),
            );
            put(
                page,
                x as i64 - dx,
                y as i64 + h as i64 - 1 + dy,
                *src.get_pixel(0, h - 1),
            );
            put(
                page,
                x as i64 + w as i64 - 1 + dx,
                y as i64 + h as i64 - 1 + dy,
                *src.get_pixel(w - 1, h - 1),
            );
        }
    }
}

fn put(page: &mut RgbaImage, x: i64, y: i64, pixel: Rgba<u8>) {
    if x < 0 || y < 0 || x >= i64::from(page.width()) || y >= i64::from(page.height()) {
        return;
    }
    page.put_pixel(x as u32, y as u32, pixel);
}

/// Encodes a page with pinned settings. `image` writes no `tIME` and no colour
/// chunks, so the bytes depend only on the pixels.
fn encode_png(image: &RgbaImage) -> Result<Vec<u8>> {
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};
    let mut out = Vec::new();
    let encoder =
        PngEncoder::new_with_quality(&mut out, CompressionType::Best, FilterType::Adaptive);
    encoder
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|e| PackError::Image {
            path: "<page>".into(),
            message: e.to_string(),
        })?;
    Ok(out)
}

/// `atlas-pack` without the filesystem: useful to the tests and to anyone who
/// wants the placement without writing pages.
#[cfg(test)]
pub(crate) fn placement_order_ids(ids_and_sizes: &[(&str, u32, u32)]) -> Vec<String> {
    let entries: Vec<Entry> = ids_and_sizes
        .iter()
        .map(|(id, w, h)| Entry {
            id: (*id).to_string(),
            pivot: [0, 0],
            z_base: 0,
            image: RgbaImage::from_pixel(*w, *h, Rgba([0, 0, 0, 0])),
        })
        .collect();
    placement_order(&entries)
        .into_iter()
        .map(|i| entries[i].id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placement_order_is_tallest_then_widest_then_by_id() {
        let order = placement_order_ids(&[
            ("b", 10, 10),
            ("a", 10, 10),
            ("tall", 4, 40),
            ("wide", 40, 10),
        ]);
        assert_eq!(order, vec!["tall", "wide", "a", "b"]);
    }

    #[test]
    fn next_pot_never_goes_below_sixteen() {
        assert_eq!(next_pot(1), 16);
        assert_eq!(next_pot(16), 16);
        assert_eq!(next_pot(17), 32);
        assert_eq!(next_pot(1025), 2048);
    }

    #[test]
    fn default_pivot_is_the_bottom_centre() {
        let img = RgbaImage::from_pixel(48, 64, Rgba([0, 0, 0, 0]));
        assert_eq!(default_pivot(&img), [24, 63]);
    }

    #[test]
    fn blit_repeats_the_border_outwards() {
        let mut page = RgbaImage::from_pixel(8, 8, Rgba([0, 0, 0, 0]));
        let mut src = RgbaImage::from_pixel(2, 2, Rgba([9, 9, 9, 255]));
        src.put_pixel(0, 0, Rgba([1, 2, 3, 255]));
        blit(&mut page, &src, 2, 2, 1);
        assert_eq!(*page.get_pixel(2, 2), Rgba([1, 2, 3, 255]));
        // The extruded ring copies the nearest edge pixel.
        assert_eq!(*page.get_pixel(1, 2), Rgba([1, 2, 3, 255]));
        assert_eq!(*page.get_pixel(2, 1), Rgba([1, 2, 3, 255]));
        assert_eq!(*page.get_pixel(1, 1), Rgba([1, 2, 3, 255]), "corner");
        assert_eq!(
            *page.get_pixel(4, 4),
            Rgba([9, 9, 9, 255]),
            "bottom-right corner"
        );
        assert_eq!(
            *page.get_pixel(5, 5),
            Rgba([0, 0, 0, 0]),
            "padding stays empty"
        );
    }

    #[test]
    fn encode_png_is_byte_stable() {
        let img = RgbaImage::from_pixel(9, 7, Rgba([3, 4, 5, 200]));
        let a = encode_png(&img).expect("encode");
        let b = encode_png(&img).expect("encode");
        assert_eq!(a, b);
        let info = crate::png::inspect(&a).expect("inspect");
        assert!(
            info.color_profile_chunks().is_empty(),
            "the encoder must not embed a profile"
        );
    }
}
