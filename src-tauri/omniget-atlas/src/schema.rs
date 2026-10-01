//! Serde types for `atlas.json`, the world's visual contract.
//!
//! The normative document is `static/world/atlas.schema.json`; this module is its
//! Rust mirror. Field order in each struct is the field order written to disk, and
//! every map is a `BTreeMap`, so serialisation is byte-for-byte deterministic.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Hard ceiling for a page's width and height, in pixels.
pub const PAGE_MAX: u32 = 2048;
/// Transparent gutter left between two packed frames, in pixels.
pub const DEFAULT_PADDING: u32 = 2;
/// Edge pixels duplicated around a frame so linear filtering cannot bleed.
pub const DEFAULT_EXTRUDE: u32 = 1;
/// Frames per second used when the source manifest does not say.
pub const DEFAULT_FPS: u32 = 10;

/// The five directions that carry their own art.
pub const UNIQUE_DIRS: [&str; 5] = ["S", "SW", "W", "NW", "N"];
/// The three directions produced by a horizontal flip, and their source.
pub const MIRROR_DIRS: [(&str, &str); 3] = [("SE", "SW"), ("E", "W"), ("NE", "NW")];

/// Returns the direction a mirrored direction must point at, if it is mirrored.
pub fn mirror_source(dir: &str) -> Option<&'static str> {
    MIRROR_DIRS
        .iter()
        .find(|(name, _)| *name == dir)
        .map(|(_, src)| *src)
}

/// Root of `atlas.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Atlas {
    pub version: u32,
    pub pages: Vec<String>,
    /// `[w, h]` of each page, same order as `pages`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub page_sizes: Vec<[u32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub padding: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extrude: Option<u32>,
    pub frames: BTreeMap<String, Frame>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub anims: BTreeMap<String, Anim>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tiles: BTreeMap<String, Tile>,
}

impl Atlas {
    /// An empty v1 atlas with the packer's defaults declared.
    pub fn empty() -> Self {
        Self {
            version: 1,
            pages: Vec::new(),
            page_sizes: Vec::new(),
            padding: Some(DEFAULT_PADDING),
            extrude: Some(DEFAULT_EXTRUDE),
            frames: BTreeMap::new(),
            anims: BTreeMap::new(),
            tiles: BTreeMap::new(),
        }
    }
}

/// A rectangle on a page. The rect excludes the extrusion border.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    pub page: usize,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// Ground contact point (centre of the feet) relative to the frame's top-left.
    pub pivot: [i32; 2],
    #[serde(default)]
    pub z_base: i32,
}

/// One animation clip of one sheet, eight directions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Anim {
    pub fps: u32,
    #[serde(rename = "loop", default = "default_true")]
    pub looping: bool,
    pub dirs: Dirs,
}

fn default_true() -> bool {
    true
}

/// The eight directions. `S`/`SW`/`W`/`NW`/`N` carry frame lists, the other three
/// are mirrors; `atlas-check` is what enforces that, so both shapes parse here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dirs {
    #[serde(rename = "S")]
    pub s: DirEntry,
    #[serde(rename = "SW")]
    pub sw: DirEntry,
    #[serde(rename = "W")]
    pub w: DirEntry,
    #[serde(rename = "NW")]
    pub nw: DirEntry,
    #[serde(rename = "N")]
    pub n: DirEntry,
    #[serde(rename = "SE")]
    pub se: DirEntry,
    #[serde(rename = "E")]
    pub e: DirEntry,
    #[serde(rename = "NE")]
    pub ne: DirEntry,
}

impl Dirs {
    /// `(name, entry)` in the on-disk order.
    pub fn iter(&self) -> [(&'static str, &DirEntry); 8] {
        [
            ("S", &self.s),
            ("SW", &self.sw),
            ("W", &self.w),
            ("NW", &self.nw),
            ("N", &self.n),
            ("SE", &self.se),
            ("E", &self.e),
            ("NE", &self.ne),
        ]
    }

    /// Builds the three mirrored entries automatically around five frame lists.
    pub fn from_unique(
        s: Vec<String>,
        sw: Vec<String>,
        w: Vec<String>,
        nw: Vec<String>,
        n: Vec<String>,
    ) -> Self {
        Self {
            s: DirEntry::Frames(s),
            sw: DirEntry::Frames(sw),
            w: DirEntry::Frames(w),
            nw: DirEntry::Frames(nw),
            n: DirEntry::Frames(n),
            se: DirEntry::Mirror(Mirror {
                mirror_of: "SW".into(),
            }),
            e: DirEntry::Mirror(Mirror {
                mirror_of: "W".into(),
            }),
            ne: DirEntry::Mirror(Mirror {
                mirror_of: "NW".into(),
            }),
        }
    }
}

/// Either the frames of a direction, or a pointer at the direction it mirrors.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DirEntry {
    Frames(Vec<String>),
    Mirror(Mirror),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mirror {
    pub mirror_of: String,
}

/// A floor, wall or object tile: one frame plus how it behaves in the world.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tile {
    pub frame: String,
    /// Vertical extent above the floor plane, in pixels (0 for floors).
    pub height: u32,
    #[serde(default)]
    pub occludes: bool,
    #[serde(default = "default_footprint")]
    pub footprint: [u32; 2],
    #[serde(default = "default_true")]
    pub walkable: bool,
}

fn default_footprint() -> [u32; 2] {
    [1, 1]
}

/// Serialises an atlas exactly the way `atlas-pack` writes it: pretty JSON,
/// trailing newline, maps in key order.
pub fn to_json(atlas: &Atlas) -> Result<String, serde_json::Error> {
    let mut s = serde_json::to_string_pretty(atlas)?;
    s.push('\n');
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirror_source_maps_the_three_flipped_directions() {
        assert_eq!(mirror_source("SE"), Some("SW"));
        assert_eq!(mirror_source("E"), Some("W"));
        assert_eq!(mirror_source("NE"), Some("NW"));
        for d in UNIQUE_DIRS {
            assert_eq!(mirror_source(d), None, "{d} must carry its own sheet");
        }
    }

    #[test]
    fn json_round_trip_keeps_every_field() {
        let mut atlas = Atlas::empty();
        atlas.pages.push("omni-0.png".into());
        atlas.page_sizes.push([256, 256]);
        atlas.frames.insert(
            "omni/walk/S/0".into(),
            Frame {
                page: 0,
                x: 1,
                y: 1,
                w: 48,
                h: 64,
                pivot: [24, 63],
                z_base: 0,
            },
        );
        atlas.anims.insert(
            "omni/walk".into(),
            Anim {
                fps: 10,
                looping: true,
                dirs: Dirs::from_unique(
                    vec!["omni/walk/S/0".into()],
                    vec!["omni/walk/SW/0".into()],
                    vec!["omni/walk/W/0".into()],
                    vec!["omni/walk/NW/0".into()],
                    vec!["omni/walk/N/0".into()],
                ),
            },
        );
        atlas.tiles.insert(
            "wall/brick".into(),
            Tile {
                frame: "casa-v1/wall/brick".into(),
                height: 32,
                occludes: true,
                footprint: [1, 1],
                walkable: false,
            },
        );

        let json = to_json(&atlas).expect("serialise");
        assert!(json.ends_with("}\n"));
        assert!(
            json.contains("\"loop\""),
            "the loop flag keeps its JSON name"
        );
        assert!(json.contains("\"mirror_of\": \"SW\""));
        let back: Atlas = serde_json::from_str(&json).expect("parse");
        assert_eq!(back, atlas);
    }

    #[test]
    fn dir_entry_parses_both_shapes() {
        let frames: DirEntry = serde_json::from_str(r#"["a","b"]"#).expect("frames");
        assert_eq!(frames, DirEntry::Frames(vec!["a".into(), "b".into()]));
        let mirror: DirEntry = serde_json::from_str(r#"{"mirror_of":"W"}"#).expect("mirror");
        assert_eq!(
            mirror,
            DirEntry::Mirror(Mirror {
                mirror_of: "W".into()
            })
        );
    }

    #[test]
    fn tile_defaults_match_the_schema_defaults() {
        let tile: Tile =
            serde_json::from_str(r#"{"frame":"casa-v1/floor/wood","height":0}"#).expect("tile");
        assert!(!tile.occludes);
        assert_eq!(tile.footprint, [1, 1]);
        assert!(tile.walkable);
    }
}
