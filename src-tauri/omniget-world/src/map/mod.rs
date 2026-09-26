//! The map: a `MapDef` in JSON on disk, a `Map` in memory.
//!
//! `MapDef` is the only thing in this crate `serde` touches. Everything that
//! travels between the tick thread and the webview goes through the binary
//! format in [`crate::snapshot`] instead.
//!
//! The house is a set of fixed rooms and walls plus **slots** (floor, wall,
//! table) where the player puts, moves and swaps catalogue objects — the
//! decision in the plan's §9.1. There is no free wall building, so the static
//! geometry of a map never changes at runtime and the pathfinding grid only
//! ever has to be patched where objects sit.

pub mod chunk;
pub mod height;
pub mod tile;

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::error::{Result, WorldError};
use crate::fixed::Fixed;
pub use chunk::{chunk_of, local_of, Chunk, ChunkDef, CHUNK_AREA, CHUNK_TILES, EMPTY};
pub use height::{px_to_z, z_to_px, HeightField, PX_PER_Z};
pub use tile::{Layer, Tile, TileDef};

/// Current `house-v1.json` format version.
pub const MAP_FORMAT_VERSION: u32 = 1;

/// Where a slot lives, which decides what the catalogue may put in it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SlotKind {
    /// Stands on the floor: furniture, plants, the workbench.
    Floor,
    /// Hangs on a wall: window, picture, the TV.
    Wall,
    /// Sits on top of another object: mug, book, lamp.
    Table,
}

/// An editable spot in the house.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct SlotDef {
    pub id: String,
    pub kind: SlotKind,
    pub tile: Tile,
    /// Atlas object keys this slot accepts. Empty means "any object of the
    /// right kind", which is what the first house ships with.
    #[serde(default)]
    pub accepts: Vec<String>,
    /// Object key placed here when the house is first created.
    #[serde(default)]
    pub default: Option<String>,
    /// Facing, 0..8 clockwise from S, matching the atlas directions.
    #[serde(default)]
    pub dir: u8,
}

/// An object the map ships with. Objects the player adds later arrive as
/// `Input::PlaceObject` and are not part of the `MapDef`.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct ObjectDef {
    pub id: u32,
    /// Atlas key, e.g. `object/workbench`.
    pub kind: String,
    pub tile: Tile,
    #[serde(default)]
    pub dir: u8,
    /// Slot this object occupies, when it sits in one.
    #[serde(default)]
    pub slot: Option<String>,
}

/// A named rectangle, used by routines ("go to the kitchen") and by the HUD.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct RoomDef {
    pub id: String,
    /// `[x, y, w, h]` in tiles.
    pub rect: [i32; 4],
}

impl RoomDef {
    pub fn contains(&self, t: Tile) -> bool {
        let [x, y, w, h] = self.rect;
        t.x >= x && t.y >= y && t.x < x + w && t.y < y + h
    }

    /// Tile an agent aims at when a routine says "go to this room".
    pub fn centre(&self) -> Tile {
        let [x, y, w, h] = self.rect;
        Tile::new(x + w / 2, y + h / 2)
    }
}

/// A named tile, for spawns and doors.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct MarkerDef {
    pub name: String,
    pub tile: Tile,
}

/// `static/world/house-v1.json`. Additive changes only, like the atlas.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct MapDef {
    pub version: u32,
    pub id: String,
    /// Path of the atlas this map draws with, relative to `static/`.
    pub atlas: String,
    /// Tiles per chunk side; must be [`CHUNK_TILES`].
    #[serde(default = "default_chunk_tiles")]
    pub chunk_tiles: u32,
    /// Palette. A chunk layer stores an index into this list; 255 is "empty".
    pub palette: Vec<TileDef>,
    pub chunks: Vec<ChunkDef>,
    #[serde(default)]
    pub slots: Vec<SlotDef>,
    #[serde(default)]
    pub objects: Vec<ObjectDef>,
    #[serde(default)]
    pub rooms: Vec<RoomDef>,
    #[serde(default)]
    pub markers: Vec<MarkerDef>,
}

fn default_chunk_tiles() -> u32 {
    CHUNK_TILES as u32
}

/// The map in memory. Immutable after `Map::load`: the only thing that changes
/// at runtime is which objects sit in which slots, and that lives in the
/// entity store.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Map {
    pub id: String,
    pub atlas: String,
    /// Tile coordinate of the lower-left corner of the map rectangle.
    pub origin: Tile,
    /// Size of the map rectangle in tiles (a whole number of chunks).
    pub w: usize,
    pub h: usize,
    pub palette: Vec<TileDef>,
    chunks: Vec<Chunk>,
    w_chunks: usize,
    h_chunks: usize,
    heights: HeightField,
    /// Static walkability from the chunk layers alone.
    walk: Vec<bool>,
    pub slots: Vec<SlotDef>,
    pub objects: Vec<ObjectDef>,
    pub rooms: Vec<RoomDef>,
    pub markers: Vec<MarkerDef>,
    slot_by_id: BTreeMap<String, usize>,
    hash: u64,
}

impl Map {
    /// Validate a `MapDef` and lay it out. Every failure is an
    /// `ERR_WORLD_MAP_INVALID` with the reason spelled out, because the person
    /// who sees it is editing JSON by hand.
    pub fn load(def: &MapDef) -> Result<Map> {
        if def.version != MAP_FORMAT_VERSION {
            return Err(WorldError::MapInvalid(format!(
                "version {} is not {MAP_FORMAT_VERSION}",
                def.version
            )));
        }
        if def.chunk_tiles as usize != CHUNK_TILES {
            return Err(WorldError::MapInvalid(format!(
                "chunk_tiles {} is not {CHUNK_TILES}",
                def.chunk_tiles
            )));
        }
        if def.palette.len() > EMPTY as usize {
            return Err(WorldError::MapInvalid(format!(
                "palette has {} entries, the limit is {}",
                def.palette.len(),
                EMPTY
            )));
        }
        if def.chunks.is_empty() {
            return Err(WorldError::MapInvalid("no chunks".into()));
        }

        let min_cx = def.chunks.iter().map(|c| c.cx).min().unwrap_or(0);
        let min_cy = def.chunks.iter().map(|c| c.cy).min().unwrap_or(0);
        let max_cx = def.chunks.iter().map(|c| c.cx).max().unwrap_or(0);
        let max_cy = def.chunks.iter().map(|c| c.cy).max().unwrap_or(0);
        let w_chunks = (max_cx - min_cx + 1) as usize;
        let h_chunks = (max_cy - min_cy + 1) as usize;
        if w_chunks * h_chunks > 4096 {
            return Err(WorldError::MapInvalid(format!(
                "{w_chunks}x{h_chunks} chunks is over the 4096 limit"
            )));
        }

        let mut chunks = Vec::with_capacity(w_chunks * h_chunks);
        for cy in min_cy..=max_cy {
            for cx in min_cx..=max_cx {
                chunks.push(Chunk::empty(cx, cy));
            }
        }
        for d in &def.chunks {
            let idx = (d.cy - min_cy) as usize * w_chunks + (d.cx - min_cx) as usize;
            chunks[idx] = Chunk::from_def(d);
        }

        let w = w_chunks * CHUNK_TILES;
        let h = h_chunks * CHUNK_TILES;
        let origin = Tile::new(min_cx * CHUNK_TILES as i32, min_cy * CHUNK_TILES as i32);

        let mut heights = HeightField::new(w, h);
        let mut walk = vec![false; w * h];
        for (ci, c) in chunks.iter().enumerate() {
            let base_x = (ci % w_chunks) * CHUNK_TILES;
            let base_y = (ci / w_chunks) * CHUNK_TILES;
            for ty in 0..CHUNK_TILES {
                for tx in 0..CHUNK_TILES {
                    let i = Chunk::index(tx, ty);
                    let (gx, gy) = (base_x + tx, base_y + ty);
                    heights.set_px(gx, gy, c.height[i]);
                    let mut walkable = false;
                    if let Some(t) = lookup(&def.palette, c.floor[i]) {
                        walkable = t.walkable;
                    }
                    for layer in [c.wall[i], c.object[i]] {
                        if let Some(t) = lookup(&def.palette, layer) {
                            if !t.walkable {
                                walkable = false;
                            }
                        }
                    }
                    walk[gy * w + gx] = walkable;
                }
            }
        }

        let mut slot_by_id = BTreeMap::new();
        for (i, s) in def.slots.iter().enumerate() {
            if slot_by_id.insert(s.id.clone(), i).is_some() {
                return Err(WorldError::MapInvalid(format!(
                    "duplicate slot id {}",
                    s.id
                )));
            }
        }
        for s in &def.slots {
            if !in_rect(origin, w, h, s.tile) {
                return Err(WorldError::MapInvalid(format!(
                    "slot {} at ({}, {}) is outside the map",
                    s.id, s.tile.x, s.tile.y
                )));
            }
        }
        let mut seen_obj = BTreeMap::new();
        for o in &def.objects {
            if seen_obj.insert(o.id, ()).is_some() {
                return Err(WorldError::MapInvalid(format!(
                    "duplicate object id {}",
                    o.id
                )));
            }
            if let Some(slot) = &o.slot {
                if !slot_by_id.contains_key(slot) {
                    return Err(WorldError::MapInvalid(format!(
                        "object {} sits in unknown slot {slot}",
                        o.id
                    )));
                }
            }
        }

        let mut map = Map {
            id: def.id.clone(),
            atlas: def.atlas.clone(),
            origin,
            w,
            h,
            palette: def.palette.clone(),
            chunks,
            w_chunks,
            h_chunks,
            heights,
            walk,
            slots: def.slots.clone(),
            objects: def.objects.clone(),
            rooms: def.rooms.clone(),
            markers: def.markers.clone(),
            slot_by_id,
            hash: 0,
        };
        map.hash = map.compute_hash();
        Ok(map)
    }

    /// FNV-1a over a canonical byte encoding of the static map. Two builds of
    /// the app on two operating systems must agree on this number, so it is
    /// computed from the laid-out arrays and never from the JSON text, whose
    /// key order and whitespace are not ours to control.
    fn compute_hash(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut feed = |bytes: &[u8]| {
            for b in bytes {
                h ^= *b as u64;
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        };
        feed(self.id.as_bytes());
        feed(&[0]);
        feed(&(self.w as u32).to_le_bytes());
        feed(&(self.h as u32).to_le_bytes());
        feed(&self.origin.x.to_le_bytes());
        feed(&self.origin.y.to_le_bytes());
        for t in &self.palette {
            feed(t.key.as_bytes());
            feed(&t.height.to_le_bytes());
            feed(&[
                t.occludes as u8,
                t.walkable as u8,
                t.footprint[0],
                t.footprint[1],
            ]);
        }
        for c in &self.chunks {
            feed(&c.floor);
            feed(&c.wall);
            feed(&c.object);
            feed(&c.height);
        }
        for s in &self.slots {
            feed(s.id.as_bytes());
            feed(&[s.kind as u8, s.dir]);
            feed(&s.tile.x.to_le_bytes());
            feed(&s.tile.y.to_le_bytes());
        }
        h
    }

    /// Identity of the static map, carried in every snapshot so a client can
    /// refuse a blob made for a different house.
    pub const fn hash(&self) -> u64 {
        self.hash
    }

    pub const fn chunk_count(&self) -> usize {
        self.w_chunks * self.h_chunks
    }

    pub fn chunks(&self) -> &[Chunk] {
        &self.chunks
    }

    /// Map rectangle index of a tile, or `None` outside the map.
    #[inline]
    pub fn index(&self, t: Tile) -> Option<usize> {
        let x = t.x - self.origin.x;
        let y = t.y - self.origin.y;
        if x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h {
            return None;
        }
        Some(y as usize * self.w + x as usize)
    }

    #[inline]
    pub fn contains(&self, t: Tile) -> bool {
        self.index(t).is_some()
    }

    /// Static walkability, before objects placed at runtime are considered.
    #[inline]
    pub fn walkable(&self, t: Tile) -> bool {
        self.index(t).map(|i| self.walk[i]).unwrap_or(false)
    }

    pub fn static_walk(&self) -> &[bool] {
        &self.walk
    }

    pub fn heights(&self) -> &HeightField {
        &self.heights
    }

    /// Standing height of a tile in world z.
    #[inline]
    pub fn z_at(&self, t: Tile) -> Fixed {
        match self.index(t) {
            Some(_) => {
                let x = (t.x - self.origin.x) as usize;
                let y = (t.y - self.origin.y) as usize;
                self.heights.z_at(x, y)
            }
            None => Fixed::ZERO,
        }
    }

    pub fn slot(&self, id: &str) -> Option<&SlotDef> {
        self.slot_by_id.get(id).map(|i| &self.slots[*i])
    }

    pub fn room_at(&self, t: Tile) -> Option<&RoomDef> {
        self.rooms.iter().find(|r| r.contains(t))
    }

    pub fn marker(&self, name: &str) -> Option<Tile> {
        self.markers.iter().find(|m| m.name == name).map(|m| m.tile)
    }

    /// Nearest walkable tile to `t`, searched in rings so the answer does not
    /// depend on iteration order. Used when a decision names a tile a wall
    /// stands on.
    pub fn nearest_walkable(&self, t: Tile, max_ring: i32) -> Option<Tile> {
        if self.walkable(t) {
            return Some(t);
        }
        for r in 1..=max_ring {
            let mut best: Option<Tile> = None;
            for dy in -r..=r {
                for dx in -r..=r {
                    if dx.abs() != r && dy.abs() != r {
                        continue;
                    }
                    let c = Tile::new(t.x + dx, t.y + dy);
                    if self.walkable(c) && best.is_none() {
                        best = Some(c);
                    }
                }
            }
            if best.is_some() {
                return best;
            }
        }
        None
    }
}

fn lookup(palette: &[TileDef], idx: u8) -> Option<&TileDef> {
    if idx == EMPTY {
        None
    } else {
        palette.get(idx as usize)
    }
}

fn in_rect(origin: Tile, w: usize, h: usize, t: Tile) -> bool {
    let x = t.x - origin.x;
    let y = t.y - origin.y;
    x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h
}

#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;

    /// A one-chunk room: wooden floor everywhere, a plaster wall along the top
    /// row, a workbench, a bed and a chair. Small enough to reason about in a
    /// test, shaped like the real house.
    pub fn one_room() -> MapDef {
        let mut floor = vec![0u8; CHUNK_AREA];
        let mut wall = vec![EMPTY; CHUNK_AREA];
        let mut height = vec![0u8; CHUNK_AREA];
        for tx in 0..CHUNK_TILES {
            let i = Chunk::index(tx, 0);
            wall[i] = 1;
            height[i] = 32;
            floor[i] = 0;
        }
        MapDef {
            version: MAP_FORMAT_VERSION,
            id: "test-one-room".into(),
            atlas: "world/tiles/casa-v1/atlas.json".into(),
            chunk_tiles: CHUNK_TILES as u32,
            palette: vec![
                TileDef {
                    key: "floor/wood".into(),
                    height: 0,
                    occludes: false,
                    footprint: [1, 1],
                    walkable: true,
                },
                TileDef {
                    key: "wall/plaster".into(),
                    height: 32,
                    occludes: true,
                    footprint: [1, 1],
                    walkable: false,
                },
                TileDef {
                    key: "object/workbench".into(),
                    height: 20,
                    occludes: false,
                    footprint: [2, 1],
                    walkable: false,
                },
            ],
            chunks: vec![ChunkDef {
                cx: 0,
                cy: 0,
                floor,
                wall,
                object: vec![],
                height,
                tint: Vec::new(),
            }],
            slots: vec![SlotDef {
                id: "bench".into(),
                kind: SlotKind::Floor,
                tile: Tile::new(4, 4),
                accepts: vec!["object/workbench".into()],
                default: Some("object/workbench".into()),
                dir: 0,
            }],
            objects: vec![ObjectDef {
                id: 1,
                kind: "object/workbench".into(),
                tile: Tile::new(4, 4),
                dir: 0,
                slot: Some("bench".into()),
            }],
            rooms: vec![RoomDef {
                id: "living".into(),
                rect: [0, 1, 16, 15],
            }],
            markers: vec![MarkerDef {
                name: "spawn".into(),
                tile: Tile::new(1, 1),
            }],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_the_one_room_fixture() {
        let m = Map::load(&fixtures::one_room()).unwrap();
        assert_eq!((m.w, m.h), (16, 16));
        assert_eq!(m.chunk_count(), 1);
        assert!(m.walkable(Tile::new(3, 3)));
        assert!(!m.walkable(Tile::new(3, 0)), "the wall row is not walkable");
        assert!(!m.walkable(Tile::new(99, 99)), "outside is not walkable");
    }

    #[test]
    fn height_comes_from_the_chunk_array() {
        let m = Map::load(&fixtures::one_room()).unwrap();
        assert_eq!(m.z_at(Tile::new(3, 0)), Fixed::ONE);
        assert_eq!(m.z_at(Tile::new(3, 3)), Fixed::ZERO);
    }

    #[test]
    fn hash_is_stable_and_sensitive() {
        let a = Map::load(&fixtures::one_room()).unwrap();
        let b = Map::load(&fixtures::one_room()).unwrap();
        assert_eq!(a.hash(), b.hash());
        let mut def = fixtures::one_room();
        def.chunks[0].floor[200] = EMPTY;
        let c = Map::load(&def).unwrap();
        assert_ne!(a.hash(), c.hash());
    }

    #[test]
    fn rejects_a_map_it_cannot_run() {
        let mut bad = fixtures::one_room();
        bad.version = 9;
        assert!(matches!(Map::load(&bad), Err(WorldError::MapInvalid(_))));

        let mut bad = fixtures::one_room();
        bad.chunk_tiles = 32;
        assert!(matches!(Map::load(&bad), Err(WorldError::MapInvalid(_))));

        let mut bad = fixtures::one_room();
        bad.chunks.clear();
        assert!(matches!(Map::load(&bad), Err(WorldError::MapInvalid(_))));

        let mut bad = fixtures::one_room();
        bad.slots.push(bad.slots[0].clone());
        assert!(matches!(Map::load(&bad), Err(WorldError::MapInvalid(_))));

        let mut bad = fixtures::one_room();
        bad.objects[0].slot = Some("nope".into());
        assert!(matches!(Map::load(&bad), Err(WorldError::MapInvalid(_))));

        let mut bad = fixtures::one_room();
        bad.slots[0].tile = Tile::new(999, 999);
        assert!(matches!(Map::load(&bad), Err(WorldError::MapInvalid(_))));
    }

    #[test]
    fn rooms_markers_and_slots_resolve() {
        let m = Map::load(&fixtures::one_room()).unwrap();
        assert_eq!(
            m.room_at(Tile::new(5, 5)).map(|r| r.id.as_str()),
            Some("living")
        );
        assert!(m.room_at(Tile::new(5, 0)).is_none());
        assert_eq!(m.marker("spawn"), Some(Tile::new(1, 1)));
        assert!(m.marker("nope").is_none());
        assert_eq!(m.slot("bench").map(|s| s.kind), Some(SlotKind::Floor));
        assert_eq!(m.rooms[0].centre(), Tile::new(8, 8));
    }

    #[test]
    fn nearest_walkable_walks_out_in_rings() {
        let m = Map::load(&fixtures::one_room()).unwrap();
        assert_eq!(
            m.nearest_walkable(Tile::new(3, 3), 4),
            Some(Tile::new(3, 3))
        );
        let near = m.nearest_walkable(Tile::new(3, 0), 4).unwrap();
        assert_eq!(near.chebyshev(Tile::new(3, 0)), 1);
        assert!(m.walkable(near));
        assert!(m.nearest_walkable(Tile::new(500, 500), 3).is_none());
    }

    #[test]
    fn negative_chunks_shift_the_origin() {
        let mut def = fixtures::one_room();
        def.chunks[0].cx = -1;
        def.chunks[0].cy = -1;
        def.slots[0].tile = Tile::new(-12, -12);
        def.objects[0].tile = Tile::new(-12, -12);
        def.rooms[0].rect = [-16, -15, 16, 15];
        def.markers[0].tile = Tile::new(-15, -15);
        let m = Map::load(&def).unwrap();
        assert_eq!(m.origin, Tile::new(-16, -16));
        assert!(m.walkable(Tile::new(-12, -12)));
        assert!(!m.contains(Tile::new(0, 0)));
    }
}
