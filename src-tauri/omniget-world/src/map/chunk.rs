//! Chunks of 16x16 tiles. The renderer bakes one render texture per chunk
//! (`CHUNK_TILES = 16` in `src/lib/world/render/types.ts`), so the simulation
//! stores the map in exactly the same grain and a dirty chunk is a single
//! number on the wire.

use serde::{Deserialize, Serialize};

/// Tiles per chunk side. Frozen: the renderer and the packer share it.
pub const CHUNK_TILES: usize = 16;
/// Tiles in one chunk.
pub const CHUNK_AREA: usize = CHUNK_TILES * CHUNK_TILES;
/// Palette index meaning "nothing here", used by the wall and object layers.
pub const EMPTY: u8 = 255;

/// One chunk as written in `house-v1.json`.
///
/// The three layers are flat arrays of 256 palette indices in row-major order
/// (`i = ty * 16 + tx`, tile-local). `height` is atlas pixels, so it matches
/// the `height` field of the tiles it is derived from and the renderer's
/// `TILE_Z`. All four arrays may be omitted, and then the chunk is flat and
/// empty; a short array is padded with [`EMPTY`] (or 0 for height), which
/// keeps hand-written fixtures readable.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct ChunkDef {
    pub cx: i32,
    pub cy: i32,
    #[serde(default)]
    pub floor: Vec<u8>,
    #[serde(default)]
    pub wall: Vec<u8>,
    #[serde(default)]
    pub object: Vec<u8>,
    #[serde(default)]
    pub height: Vec<u8>,
    /// Per-cell floor colour, 0xRRGGBB in sRGB, multiplied over the floor
    /// sprite by the renderer; 0 (or a missing layer) means white. Visual
    /// only: not part of the map hash, collision or coordinates, so maps
    /// written before this field existed load unchanged.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tint: Vec<u32>,
}

/// A chunk in memory: fixed-size arrays, no allocation per access.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Chunk {
    pub cx: i32,
    pub cy: i32,
    pub floor: [u8; CHUNK_AREA],
    pub wall: [u8; CHUNK_AREA],
    pub object: [u8; CHUNK_AREA],
    pub height: [u8; CHUNK_AREA],
}

impl Chunk {
    pub fn empty(cx: i32, cy: i32) -> Chunk {
        Chunk {
            cx,
            cy,
            floor: [EMPTY; CHUNK_AREA],
            wall: [EMPTY; CHUNK_AREA],
            object: [EMPTY; CHUNK_AREA],
            height: [0; CHUNK_AREA],
        }
    }

    pub fn from_def(def: &ChunkDef) -> Chunk {
        let mut c = Chunk::empty(def.cx, def.cy);
        fill(&mut c.floor, &def.floor, EMPTY);
        fill(&mut c.wall, &def.wall, EMPTY);
        fill(&mut c.object, &def.object, EMPTY);
        fill(&mut c.height, &def.height, 0);
        c
    }

    pub fn to_def(&self) -> ChunkDef {
        ChunkDef {
            cx: self.cx,
            cy: self.cy,
            floor: self.floor.to_vec(),
            wall: self.wall.to_vec(),
            object: self.object.to_vec(),
            height: self.height.to_vec(),
            tint: Vec::new(),
        }
    }

    #[inline]
    pub const fn index(tx: usize, ty: usize) -> usize {
        ty * CHUNK_TILES + tx
    }
}

fn fill(dst: &mut [u8; CHUNK_AREA], src: &[u8], pad: u8) {
    let n = src.len().min(CHUNK_AREA);
    dst[..n].copy_from_slice(&src[..n]);
    for slot in dst.iter_mut().skip(n) {
        *slot = pad;
    }
}

/// Chunk a tile belongs to, with the floor-division that keeps negative
/// coordinates in the right chunk.
#[inline]
pub const fn chunk_of(x: i32, y: i32) -> (i32, i32) {
    (
        x.div_euclid(CHUNK_TILES as i32),
        y.div_euclid(CHUNK_TILES as i32),
    )
}

/// Tile-local coordinates inside its chunk.
#[inline]
pub const fn local_of(x: i32, y: i32) -> (usize, usize) {
    (
        x.rem_euclid(CHUNK_TILES as i32) as usize,
        y.rem_euclid(CHUNK_TILES as i32) as usize,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_maths_handles_negative_tiles() {
        assert_eq!(chunk_of(0, 0), (0, 0));
        assert_eq!(chunk_of(15, 15), (0, 0));
        assert_eq!(chunk_of(16, 0), (1, 0));
        assert_eq!(chunk_of(-1, -1), (-1, -1));
        assert_eq!(local_of(-1, -1), (15, 15));
        assert_eq!(local_of(17, 3), (1, 3));
    }

    #[test]
    fn short_arrays_are_padded_not_rejected() {
        let def = ChunkDef {
            cx: 0,
            cy: 0,
            floor: vec![1, 1, 1],
            wall: vec![],
            object: vec![],
            height: vec![32],
            tint: Vec::new(),
        };
        let c = Chunk::from_def(&def);
        assert_eq!(c.floor[0], 1);
        assert_eq!(c.floor[3], EMPTY);
        assert_eq!(c.wall[0], EMPTY);
        assert_eq!(c.height[0], 32);
        assert_eq!(c.height[1], 0);
    }

    #[test]
    fn long_arrays_are_truncated() {
        let def = ChunkDef {
            cx: 1,
            cy: 2,
            floor: vec![7; CHUNK_AREA + 40],
            wall: vec![],
            object: vec![],
            height: vec![],
            tint: Vec::new(),
        };
        let c = Chunk::from_def(&def);
        assert_eq!(c.floor.len(), CHUNK_AREA);
        assert_eq!(c.floor[CHUNK_AREA - 1], 7);
        assert_eq!((c.cx, c.cy), (1, 2));
    }

    #[test]
    fn def_round_trips() {
        let mut c = Chunk::empty(3, -4);
        c.floor[5] = 2;
        c.height[5] = 32;
        assert_eq!(Chunk::from_def(&c.to_def()), c);
    }

    #[test]
    fn index_is_row_major() {
        assert_eq!(Chunk::index(0, 0), 0);
        assert_eq!(Chunk::index(15, 0), 15);
        assert_eq!(Chunk::index(0, 1), 16);
        assert_eq!(Chunk::index(15, 15), CHUNK_AREA - 1);
    }
}
