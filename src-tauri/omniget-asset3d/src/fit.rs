//! Passo 4: scale, up axis and pivot for the 2 m grid. The pivot ends at the
//! centre of the base with the feet on y = 0.

use crate::model::*;
use crate::types::{AssetClass, TILE_MM};
use crate::Error;

pub struct Fit {
    pub scale: f32,
    pub rotated_z_up: bool,
    pub bbox: Aabb,
}

/// Plausible size range (metres) of the measured dimension and the size used
/// when no unit guess lands inside it.
fn class_range(class: AssetClass) -> (f32, f32, f32) {
    match class {
        AssetClass::Character => (1.3, 2.05, 1.7),
        AssetClass::Prop => (0.2, 2.0, 1.0),
        AssetClass::Decoration => (0.05, 2.0, 0.5),
        AssetClass::Building => (2.0, 40.0, 8.0),
    }
}

fn measure(class: AssetClass, size: [f32; 3]) -> f32 {
    match class {
        AssetClass::Character => size[1],
        AssetClass::Building => size[0].max(size[2]),
        AssetClass::Prop | AssetClass::Decoration => size[0].max(size[1]).max(size[2]),
    }
}

/// Picks the scale: keep the file's units when they are plausible, else try
/// cm, mm and ×100, else scale to the class default.
pub fn choose_scale(class: AssetClass, size: [f32; 3]) -> f32 {
    let (lo, hi, def) = class_range(class);
    let m = measure(class, size);
    if m.is_nan() || m <= 1e-6 || !m.is_finite() {
        return 1.0;
    }
    let mut s = None;
    for f in [1.0f32, 0.01, 0.001, 100.0] {
        let v = m * f;
        if v >= lo && v <= hi {
            s = Some(f);
            break;
        }
    }
    let mut s = s.unwrap_or(def / m);
    if class == AssetClass::Building {
        // Snap the larger side to whole tiles.
        let tile = TILE_MM as f32 / 1000.0;
        let side = m * s;
        let snapped = ((side / tile).round().max(1.0)) * tile;
        s *= snapped / side;
    }
    s
}

pub fn fit(
    model: &mut Model,
    class: AssetClass,
    flattened: bool,
    may_be_z_up: bool,
) -> Result<Fit, Error> {
    let pts = model.world_positions();
    let bb = Aabb::of(&pts);
    if !bb.is_valid() {
        return Err(Error::Malformed("the model has no geometry".into()));
    }
    let size = bb.size();
    // Z-up characters (common in OBJ exports) lie on their back. glTF is
    // Y-up by spec, so only formats without an up axis are turned.
    let rotate = may_be_z_up
        && class == AssetClass::Character
        && size[2] > size[1] * 1.6
        && size[2] >= size[0];
    let rot: Mat4 = if rotate {
        // −90° about X: (x, y, z) → (x, z, −y).
        [
            1.0, 0.0, 0.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ]
    } else {
        IDENTITY
    };
    let rotated: Vec<[f32; 3]> = pts.iter().map(|p| transform_point(&rot, *p)).collect();
    let rb = Aabb::of(&rotated);
    let s = choose_scale(class, rb.size());
    let cx = (rb.min[0] + rb.max[0]) * 0.5;
    let cz = (rb.min[2] + rb.max[2]) * 0.5;
    let t = [-cx * s, -rb.min[1] * s, -cz * s];
    let sm: Mat4 = [
        s, 0.0, 0.0, 0.0, 0.0, s, 0.0, 0.0, 0.0, 0.0, s, 0.0, t[0], t[1], t[2], 1.0,
    ];
    let full = mul(&sm, &rot);

    if flattened {
        for mesh in &mut model.meshes {
            for p in &mut mesh.prims {
                for v in &mut p.pos {
                    *v = transform_point(&full, *v);
                }
                if let Some(n) = &mut p.nrm {
                    for v in n.iter_mut() {
                        *v = transform_normal(&full, *v);
                    }
                }
            }
        }
    } else {
        // Wrap the scene in one new root node, appended last so every index
        // a VRM extension points at stays the same.
        let q = if rotate {
            let h = std::f32::consts::FRAC_1_SQRT_2;
            [-h, 0.0, 0.0, h]
        } else {
            [0.0, 0.0, 0.0, 1.0]
        };
        let wrapper = Node {
            name: Some("omniget_root".into()),
            t,
            r: q,
            s: [s; 3],
            children: model.roots.clone(),
            ..Default::default()
        };
        model.nodes.push(wrapper);
        model.roots = vec![model.nodes.len() - 1];
    }
    let after = Aabb::of(&model.world_positions());
    Ok(Fit {
        scale: s,
        rotated_z_up: rotate,
        bbox: after,
    })
}
