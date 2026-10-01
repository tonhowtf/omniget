use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const PAGE_MAX: u32 = 2048;
pub const DEFAULT_PADDING: u32 = 2;
pub const DEFAULT_EXTRUDE: u32 = 1;
pub const DEFAULT_FPS: u32 = 10;

pub const UNIQUE_DIRS: [&str; 5] = ["S", "SW", "W", "NW", "N"];
pub const MIRROR_DIRS: [(&str, &str); 3] = [("SE", "SW"), ("E", "W"), ("NE", "NW")];

pub fn mirror_source(dir: &str) -> Option<&'static str> {
    MIRROR_DIRS
        .iter()
        .find(|(name, _)| *name == dir)
        .map(|(_, src)| *src)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Atlas {
    pub version: u32,
    pub pages: Vec<String>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    pub page: usize,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub pivot: [i32; 2],
    #[serde(default)]
    pub z_base: i32,
}

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tile {
    pub frame: String,
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
