//! Canonical mesh: positions quantised to 0.1 mm, one palette slot per
//! triangle, triangles rotated to start at their smallest vertex and sorted.
//! Two runs of the same recipe give the same bytes, and two Manifold builds
//! that agree to 0.1 mm give the same mesh.

use manifold_rust::manifold::Manifold;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// Quantisation step: 1 unit = 0.1 mm.
pub const Q: f64 = 10_000.0;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CTri {
    pub slot: u8,
    pub v: [[i32; 3]; 3],
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CanonMesh {
    pub tris: Vec<CTri>,
}

fn q(x: f64) -> i32 {
    (x * Q).round() as i32
}

impl CanonMesh {
    pub fn from_manifold(m: &Manifold, slot_of: &HashMap<u32, u8>) -> Result<CanonMesh, String> {
        let mesh = m.get_mesh_gl(-1);
        let np = mesh.num_prop as usize;
        let vp = &mesh.vert_properties;
        let pos = |i: u32| -> [i32; 3] {
            let o = i as usize * np;
            [q(vp[o] as f64), q(vp[o + 1] as f64), q(vp[o + 2] as f64)]
        };
        let mut tris = Vec::with_capacity(mesh.tri_verts.len() / 3);
        let runs = mesh.run_index.len().saturating_sub(1);
        for r in 0..runs.max(1) {
            let (start, end) = if mesh.run_index.len() >= 2 {
                (mesh.run_index[r] as usize, mesh.run_index[r + 1] as usize)
            } else {
                (0, mesh.tri_verts.len())
            };
            let id = mesh.run_original_id.get(r).copied();
            let slot = match id.and_then(|id| slot_of.get(&id)) {
                Some(s) => *s,
                None => {
                    return Err(format!(
                        "internal: a surface lost its material (run {r}, id {id:?})"
                    ))
                }
            };
            for t in mesh.tri_verts[start..end].chunks(3) {
                let v = [pos(t[0]), pos(t[1]), pos(t[2])];
                if let Some(tri) = CTri::new(slot, v) {
                    tris.push(tri);
                }
            }
        }
        tris.sort();
        Ok(CanonMesh { tris })
    }

    pub fn len(&self) -> usize {
        self.tris.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tris.is_empty()
    }

    /// `(min, max)` in quantised units, or zeros for an empty mesh.
    pub fn bounds(&self) -> ([i32; 3], [i32; 3]) {
        if self.tris.is_empty() {
            return ([0; 3], [0; 3]);
        }
        let mut lo = [i32::MAX; 3];
        let mut hi = [i32::MIN; 3];
        for t in &self.tris {
            for v in &t.v {
                for k in 0..3 {
                    lo[k] = lo[k].min(v[k]);
                    hi[k] = hi[k].max(v[k]);
                }
            }
        }
        (lo, hi)
    }

    pub fn slots(&self) -> Vec<u8> {
        let mut s: Vec<u8> = self.tris.iter().map(|t| t.slot).collect();
        s.dedup();
        s.sort();
        s.dedup();
        s
    }

    pub fn hash(&self) -> String {
        let mut h = Sha256::new();
        for t in &self.tris {
            h.update([t.slot]);
            for v in &t.v {
                for c in v {
                    h.update(c.to_le_bytes());
                }
            }
        }
        hex(&h.finalize())
    }

    /// Rotated by quarter turns clockwise (seen from above) and moved, in
    /// quantised units. Used to compose previews.
    pub fn placed(&self, rot: u8, dx: i32, dy: i32, dz: i32) -> CanonMesh {
        let r = |p: [i32; 3]| -> [i32; 3] {
            let (x, z) = match rot % 4 {
                0 => (p[0], p[2]),
                1 => (-p[2], p[0]),
                2 => (-p[0], -p[2]),
                _ => (p[2], -p[0]),
            };
            [x + dx, p[1] + dy, z + dz]
        };
        let mut tris: Vec<CTri> = self
            .tris
            .iter()
            .filter_map(|t| CTri::new(t.slot, [r(t.v[0]), r(t.v[1]), r(t.v[2])]))
            .collect();
        tris.sort();
        CanonMesh { tris }
    }
}

impl CTri {
    /// Drops degenerate triangles and rotates the vertices so the smallest
    /// comes first (winding kept).
    pub fn new(slot: u8, v: [[i32; 3]; 3]) -> Option<CTri> {
        let n = normal_i(&v);
        if n == [0, 0, 0] {
            return None;
        }
        let m = (0..3).min_by_key(|&i| v[i]).unwrap_or(0);
        Some(CTri {
            slot,
            v: [v[m], v[(m + 1) % 3], v[(m + 2) % 3]],
        })
    }

    /// Unit normal from the quantised positions.
    pub fn normal(&self) -> [f64; 3] {
        let n = normal_i(&self.v);
        let (x, y, z) = (n[0] as f64, n[1] as f64, n[2] as f64);
        let l = (x * x + y * y + z * z).sqrt();
        [x / l, y / l, z / l]
    }
}

fn normal_i(v: &[[i32; 3]; 3]) -> [i128; 3] {
    let a = [
        (v[1][0] - v[0][0]) as i128,
        (v[1][1] - v[0][1]) as i128,
        (v[1][2] - v[0][2]) as i128,
    ];
    let b = [
        (v[2][0] - v[0][0]) as i128,
        (v[2][1] - v[0][1]) as i128,
        (v[2][2] - v[0][2]) as i128,
    ];
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
