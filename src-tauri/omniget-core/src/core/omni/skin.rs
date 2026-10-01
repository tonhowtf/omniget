//! Mascot `skin` (Phase 5). Owned by f5-omni-intent.
//!
//! Reads the real art produced in round 1 — `static/omni/skins/<id>/pet.json`
//! plus the atlas it points at (`static/world/omni/atlas.json`, the format in
//! `static/world/atlas.schema.json`) — and hands the pet window and the rail
//! avatar exactly what they need to draw: the page to sample, the rects, the
//! pivot, the fps and whether the direction is a horizontal mirror.
//!
//! Only the parts of the atlas a character needs are modelled; `tiles` and any
//! future key are ignored, which keeps this loader compatible with additive
//! schema changes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::intent::Animation;

/// The eight on-screen directions. `SE`, `E` and `NE` are never drawn: they are
/// the horizontal flip of `SW`, `W` and `NW` around the pivot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Dir {
    S,
    SW,
    W,
    NW,
    N,
    SE,
    E,
    NE,
}

impl Dir {
    pub const ALL: [Dir; 8] = [
        Dir::S,
        Dir::SW,
        Dir::W,
        Dir::NW,
        Dir::N,
        Dir::SE,
        Dir::E,
        Dir::NE,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Dir::S => "S",
            Dir::SW => "SW",
            Dir::W => "W",
            Dir::NW => "NW",
            Dir::N => "N",
            Dir::SE => "SE",
            Dir::E => "E",
            Dir::NE => "NE",
        }
    }

    /// The direction actually drawn on the sheet, and whether the caller has to
    /// flip it horizontally.
    pub fn source(self) -> (Dir, bool) {
        match self {
            Dir::SE => (Dir::SW, true),
            Dir::E => (Dir::W, true),
            Dir::NE => (Dir::NW, true),
            other => (other, false),
        }
    }
}

/// `static/omni/skins/<id>/pet.json`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PetManifest {
    pub skin: String,
    /// Webview-absolute path of the atlas, e.g. `/world/omni/atlas.json`.
    pub atlas: String,
    /// Single-frame poses by name (`idle`, `wave`, `sleep`, `work`): each value
    /// is a frame id in the atlas. Used by the rail avatar, which never animates.
    #[serde(default)]
    pub poses: HashMap<String, String>,
    /// Optional tint applied by the UI (`SKIN_TINTS` from the profile). Absent
    /// in `omni-default`; additive, so older manifests still load.
    #[serde(default)]
    pub tint: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AtlasFrame {
    pub page: usize,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// Ground contact point, in pixels from the frame's top-left.
    pub pivot: [i32; 2],
    #[serde(default)]
    pub z_base: i32,
}

/// `dirs` entry: either the frame ids, or a mirror declaration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum DirEntry {
    Frames(Vec<String>),
    Mirror { mirror_of: String },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AtlasAnim {
    pub fps: u32,
    #[serde(default = "loop_default", rename = "loop")]
    pub looping: bool,
    pub dirs: HashMap<String, DirEntry>,
}

fn loop_default() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Atlas {
    pub version: u32,
    pub pages: Vec<String>,
    #[serde(default)]
    pub page_sizes: Vec<[u32; 2]>,
    #[serde(default)]
    pub frames: HashMap<String, AtlasFrame>,
    #[serde(default)]
    pub anims: HashMap<String, AtlasAnim>,
}

/// One frame, ready to blit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FrameView {
    pub id: String,
    /// Webview-absolute URL of the page PNG, e.g. `/world/omni/omni-0.png`.
    pub page_url: String,
    pub page_size: Option<[u32; 2]>,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub pivot: [i32; 2],
    pub z_base: i32,
    /// True when the caller must flip horizontally around the pivot.
    pub mirrored: bool,
}

/// One animation in one direction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AnimView {
    /// Atlas key actually used, e.g. `omni/wave` (may be a fallback).
    pub anim: String,
    pub dir: &'static str,
    pub fps: u32,
    pub looping: bool,
    pub mirrored: bool,
    pub frames: Vec<FrameView>,
}

impl AnimView {
    /// Frame index at `elapsed_ms` since the animation started.
    pub fn frame_at(&self, elapsed_ms: u64) -> usize {
        if self.frames.is_empty() || self.fps == 0 {
            return 0;
        }
        let idx = (elapsed_ms * self.fps as u64) / 1000;
        if self.looping {
            (idx % self.frames.len() as u64) as usize
        } else {
            (idx as usize).min(self.frames.len() - 1)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkinError {
    /// pet.json missing or unreadable.
    ManifestRead,
    ManifestParse,
    AtlasRead,
    AtlasParse,
    /// The atlas carries no animation for the sheet the manifest points at.
    EmptyAtlas,
}

impl SkinError {
    /// Stable code the UI can map, in the `StreamError::code()` mould.
    pub fn code(self) -> &'static str {
        match self {
            SkinError::ManifestRead => "ERR_OMNI_SKIN_MANIFEST_READ",
            SkinError::ManifestParse => "ERR_OMNI_SKIN_MANIFEST_PARSE",
            SkinError::AtlasRead => "ERR_OMNI_SKIN_ATLAS_READ",
            SkinError::AtlasParse => "ERR_OMNI_SKIN_ATLAS_PARSE",
            SkinError::EmptyAtlas => "ERR_OMNI_SKIN_EMPTY_ATLAS",
        }
    }
}

impl std::fmt::Display for SkinError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for SkinError {}

/// A loaded skin: manifest + atlas + the sheet prefix inside the atlas.
#[derive(Debug, Clone)]
pub struct Skin {
    pub manifest: PetManifest,
    pub atlas: Atlas,
    /// `omni` for `omni/idle`, derived from the atlas' animation keys.
    pub sheet: String,
    /// Webview-absolute directory of the atlas, e.g. `/world/omni`.
    atlas_dir_url: String,
}

impl Skin {
    /// Loads `<static_root>/omni/skins/<skin_id>/pet.json` and the atlas it
    /// points at. `static_root` is the app's `static/` directory.
    pub fn load(static_root: &Path, skin_id: &str) -> Result<Skin, SkinError> {
        let manifest_path = static_root
            .join("omni")
            .join("skins")
            .join(skin_id)
            .join("pet.json");
        Skin::load_manifest_path(static_root, &manifest_path)
    }

    pub fn load_manifest_path(static_root: &Path, manifest_path: &Path) -> Result<Skin, SkinError> {
        let raw = std::fs::read_to_string(manifest_path).map_err(|_| SkinError::ManifestRead)?;
        let manifest: PetManifest =
            serde_json::from_str(&raw).map_err(|_| SkinError::ManifestParse)?;

        let atlas_path = resolve_static(static_root, &manifest.atlas);
        let atlas_raw = std::fs::read_to_string(&atlas_path).map_err(|_| SkinError::AtlasRead)?;
        let atlas: Atlas = serde_json::from_str(&atlas_raw).map_err(|_| SkinError::AtlasParse)?;

        let sheet = atlas
            .anims
            .keys()
            .filter_map(|k| k.split('/').next())
            .min()
            .ok_or(SkinError::EmptyAtlas)?
            .to_string();

        let atlas_dir_url = url_dir(&manifest.atlas);

        Ok(Skin {
            manifest,
            atlas,
            sheet,
            atlas_dir_url,
        })
    }

    pub fn id(&self) -> &str {
        &self.manifest.skin
    }

    fn page_url(&self, page: usize) -> String {
        match self.atlas.pages.get(page) {
            Some(name) => format!("{}/{}", self.atlas_dir_url, name),
            None => String::new(),
        }
    }

    fn page_size(&self, page: usize) -> Option<[u32; 2]> {
        self.atlas.page_sizes.get(page).copied()
    }

    /// One frame by atlas id, e.g. `omni/wave/S/3`.
    pub fn frame(&self, id: &str) -> Option<FrameView> {
        self.frame_inner(id, false)
    }

    fn frame_inner(&self, id: &str, mirrored: bool) -> Option<FrameView> {
        let f = self.atlas.frames.get(id)?;
        Some(FrameView {
            id: id.to_string(),
            page_url: self.page_url(f.page),
            page_size: self.page_size(f.page),
            x: f.x,
            y: f.y,
            w: f.w,
            h: f.h,
            pivot: f.pivot,
            z_base: f.z_base,
            mirrored,
        })
    }

    /// A named single-frame pose from `pet.json` (`idle`, `wave`, ...).
    pub fn pose(&self, name: &str) -> Option<FrameView> {
        let id = self.manifest.poses.get(name)?;
        self.frame(id)
    }

    /// The frames for `animation` facing `dir`, resolving the fallback chain
    /// (`Celebrate` -> wave, `Worried` -> idle) and the `mirror_of` indirection.
    pub fn anim(&self, animation: Animation, dir: Dir) -> Option<AnimView> {
        for name in atlas_names(animation) {
            let key = format!("{}/{}", self.sheet, name);
            if let Some(view) = self.anim_by_key(&key, dir) {
                return Some(view);
            }
        }
        None
    }

    /// Same, by raw atlas key (`omni/walk`). Follows one level of `mirror_of`.
    pub fn anim_by_key(&self, key: &str, dir: Dir) -> Option<AnimView> {
        let clip = self.atlas.anims.get(key)?;
        let (source_dir, mut mirrored) = dir.source();
        let mut entry = clip.dirs.get(source_dir.as_str())?;
        if let DirEntry::Mirror { mirror_of } = entry {
            mirrored = true;
            entry = clip.dirs.get(mirror_of.as_str())?;
        }
        let ids = match entry {
            DirEntry::Frames(ids) => ids,
            // A mirror pointing at a mirror: the schema forbids it.
            DirEntry::Mirror { .. } => return None,
        };
        let frames: Vec<FrameView> = ids
            .iter()
            .filter_map(|id| self.frame_inner(id, mirrored))
            .collect();
        if frames.len() != ids.len() || frames.is_empty() {
            return None;
        }
        Some(AnimView {
            anim: key.to_string(),
            dir: dir.as_str(),
            fps: clip.fps.max(1),
            looping: clip.looping,
            mirrored,
            frames,
        })
    }
}

/// Atlas animation names to try for a mascot animation, best first. The sheet
/// drawn in round 1 has idle, walk, sit, wave, work, sleep and talk; the two
/// extra moods reuse the closest pose.
pub fn atlas_names(animation: Animation) -> &'static [&'static str] {
    match animation {
        Animation::Idle => &["idle"],
        Animation::Walk => &["walk", "idle"],
        Animation::Sit => &["sit", "idle"],
        Animation::Wave => &["wave", "idle"],
        Animation::Work => &["work", "idle"],
        Animation::Sleep => &["sleep", "sit", "idle"],
        Animation::Talk => &["talk", "idle"],
        Animation::Celebrate => &["celebrate", "wave", "idle"],
        // One table for the whole app: `commands/pet.rs` and `src/lib/omni/skins.ts`
        // resolve `Worried` to idle too (the rail strip has no `sit`).
        Animation::Worried => &["worried", "idle"],
    }
}

/// `/world/omni/atlas.json` -> `<static_root>/world/omni/atlas.json`.
fn resolve_static(static_root: &Path, url: &str) -> PathBuf {
    let rel = url.trim_start_matches('/');
    let mut path = static_root.to_path_buf();
    for part in rel.split('/').filter(|p| !p.is_empty() && *p != ".") {
        path.push(part);
    }
    path
}

/// `/world/omni/atlas.json` -> `/world/omni`.
fn url_dir(url: &str) -> String {
    match url.rfind('/') {
        Some(0) | None => String::new(),
        Some(i) => url[..i].to_string(),
    }
}
