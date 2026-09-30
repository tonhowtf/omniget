//! Passo 8: anchors. A geometric heuristic runs always (flat tops at seat
//! height become `sit`, at table height `work`, long ones `sleep`, the top
//! `stack`, a building's front `door`). [`anchor_prompt`] builds the request
//! for a vision LLM (G2 sends it with the user's provider) and
//! [`parse_anchor_reply`] validates the answer against the geometry.

use crate::model::*;
use crate::types::{AssetAnchor, AssetClass};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const CELL: f32 = 0.25;
const MAX_ANCHORS: usize = 8;

fn mm3(p: [f32; 3]) -> [i32; 3] {
    [
        (p[0] * 1000.0).round() as i32,
        (p[1] * 1000.0).round() as i32,
        (p[2] * 1000.0).round() as i32,
    ]
}

pub fn yaw_of(dir: &str) -> u8 {
    match dir {
        "n" => 0,
        "e" => 64,
        "s" => 128,
        _ => 192,
    }
}

struct Cluster {
    y: f32,
    area: f32,
    min: [f32; 2],
    max: [f32; 2],
    centroid: [f32; 2],
}

fn flat_clusters(tris: &[[[f32; 3]; 3]]) -> Vec<Cluster> {
    // (height band 5 cm, cell x, cell z) → area
    let mut cells: HashMap<(i32, i32, i32), f32> = HashMap::new();
    for t in tris {
        let n = cross(sub(t[1], t[0]), sub(t[2], t[0]));
        let a = len3(n) * 0.5;
        if a < 1e-5 || normalize3(n)[1] < 0.9 {
            continue;
        }
        let y = (t[0][1] + t[1][1] + t[2][1]) / 3.0;
        let band = (y / 0.05).round() as i32;
        // Spread the area over the cells the triangle's box touches.
        let minx = t[0][0].min(t[1][0]).min(t[2][0]);
        let maxx = t[0][0].max(t[1][0]).max(t[2][0]);
        let minz = t[0][2].min(t[1][2]).min(t[2][2]);
        let maxz = t[0][2].max(t[1][2]).max(t[2][2]);
        let (x0, x1) = ((minx / CELL).floor() as i32, (maxx / CELL).floor() as i32);
        let (z0, z1) = ((minz / CELL).floor() as i32, (maxz / CELL).floor() as i32);
        let span = ((x1 - x0 + 1) * (z1 - z0 + 1)).max(1) as f32;
        if span > 4096.0 {
            continue;
        }
        for cx in x0..=x1 {
            for cz in z0..=z1 {
                *cells.entry((band, cx, cz)).or_default() += a / span;
            }
        }
    }
    // Connected components per band.
    let mut seen: HashMap<(i32, i32, i32), bool> = HashMap::new();
    let mut keys: Vec<_> = cells.keys().copied().collect();
    keys.sort();
    let mut out = vec![];
    for k in keys {
        if seen.contains_key(&k) {
            continue;
        }
        let mut stack = vec![k];
        seen.insert(k, true);
        let (mut area, mut sx, mut sz) = (0f32, 0f32, 0f32);
        let (mut min, mut max) = ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]);
        while let Some(c) = stack.pop() {
            let a = cells[&c];
            let (px, pz) = ((c.1 as f32 + 0.5) * CELL, (c.2 as f32 + 0.5) * CELL);
            area += a;
            sx += px * a;
            sz += pz * a;
            min = [min[0].min(px - CELL / 2.0), min[1].min(pz - CELL / 2.0)];
            max = [max[0].max(px + CELL / 2.0), max[1].max(pz + CELL / 2.0)];
            for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let nk = (c.0, c.1 + dx, c.2 + dz);
                if cells.contains_key(&nk) && !seen.contains_key(&nk) {
                    seen.insert(nk, true);
                    stack.push(nk);
                }
            }
        }
        if area > 0.0 {
            out.push(Cluster {
                y: k.0 as f32 * 0.05,
                area,
                min,
                max,
                centroid: [sx / area, sz / area],
            });
        }
    }
    out
}

/// Direction a seated person faces: away from the geometry that rises
/// behind the seat (a backrest), else the asset's front.
fn seat_yaw(tris: &[[[f32; 3]; 3]], c: &Cluster, front: &str) -> u8 {
    let mut side = [0f32; 4]; // geometry above the seat towards n e s w
    for t in tris {
        for p in t {
            if p[1] < c.y + 0.15 || p[1] > c.y + 0.8 {
                continue;
            }
            let dx = p[0] - c.centroid[0];
            let dz = p[2] - c.centroid[1];
            if dx.abs().max(dz.abs()) > 0.6 {
                continue;
            }
            if dz.abs() >= dx.abs() {
                side[if dz < 0.0 { 0 } else { 2 }] += 1.0;
            } else {
                side[if dx > 0.0 { 1 } else { 3 }] += 1.0;
            }
        }
    }
    let (i, v) = side.iter().enumerate().fold(
        (0, 0f32),
        |acc, (i, v)| if *v > acc.1 { (i, *v) } else { acc },
    );
    if v < 3.0 {
        return yaw_of(front);
    }
    // Face away from the backrest.
    yaw_of(["s", "w", "n", "e"][i])
}

pub fn heuristic(
    class: AssetClass,
    tris: &[[[f32; 3]; 3]],
    bb: &Aabb,
    front: &str,
) -> Vec<AssetAnchor> {
    let mut out = vec![];
    let push = |out: &mut Vec<AssetAnchor>, action: &str, p: [f32; 3], yaw: u8, cap: u8| {
        if out.len() < MAX_ANCHORS {
            out.push(AssetAnchor {
                action: action.into(),
                pos_mm: mm3(p),
                yaw,
                capacity: cap,
                source: "heuristic".into(),
            });
        }
    };
    match class {
        AssetClass::Character | AssetClass::Decoration => return out,
        AssetClass::Building => {
            let (x, z) = match front {
                "n" => (0.0, bb.min[2]),
                "e" => (bb.max[0], 0.0),
                "w" => (bb.min[0], 0.0),
                _ => (0.0, bb.max[2]),
            };
            // The user stands outside and faces the building.
            let facing = match front {
                "n" => "s",
                "e" => "w",
                "w" => "e",
                _ => "n",
            };
            push(&mut out, "door", [x, 0.0, z], yaw_of(facing), 1);
            return out;
        }
        AssetClass::Prop => {}
    }
    let mut clusters = flat_clusters(tris);
    clusters.sort_by(|a, b| {
        b.area
            .partial_cmp(&a.area)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let top_y = bb.max[1];
    let mut used = vec![false; clusters.len()];
    for (i, c) in clusters.iter().enumerate() {
        let long = (c.max[0] - c.min[0]).max(c.max[1] - c.min[1]);
        let short = (c.max[0] - c.min[0]).min(c.max[1] - c.min[1]);
        let p = [c.centroid[0], c.y, c.centroid[1]];
        if (0.3..=0.65).contains(&c.y) && c.area >= 1.3 && long >= 1.6 {
            let cap = if short >= 1.2 { 2 } else { 1 };
            push(&mut out, "sleep", p, yaw_of(front), cap);
            used[i] = true;
        } else if (0.35..=0.6).contains(&c.y) && c.area >= 0.08 {
            let cap = ((long / 0.55).floor() as u8).clamp(1, 4);
            push(&mut out, "sit", p, seat_yaw(tris, c, front), cap);
            used[i] = true;
        } else if (0.65..=0.85).contains(&c.y) && c.area >= 0.25 {
            let cap =
                (((c.max[0] - c.min[0] + c.max[1] - c.min[1]) / 0.6).floor() as u8).clamp(1, 6);
            push(&mut out, "work", p, yaw_of(front), cap);
            used[i] = true;
        }
    }
    if let Some((i, c)) = clusters
        .iter()
        .enumerate()
        .filter(|(i, c)| !used[*i] && c.y >= 0.2 && (top_y - c.y) < 0.05 && c.area >= 0.2)
        .max_by(|a, b| {
            a.1.area
                .partial_cmp(&b.1.area)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    {
        let _ = i;
        push(
            &mut out,
            "stack",
            [c.centroid[0], c.y, c.centroid[1]],
            yaw_of(front),
            1,
        );
    }
    out
}

// ── Vision LLM hook ────────────────────────────────────────────────────────

/// Everything G2 needs to ask a vision model about an asset.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AnchorPrompt {
    pub system: String,
    pub user: String,
    /// PNG views: front, side, top, 3/4 (in that order).
    pub images_png: Vec<Vec<u8>>,
    /// JSON schema the reply must follow.
    pub schema: serde_json::Value,
}

/// Parsed and validated reply of the vision model.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct VisionAnnotation {
    #[serde(default)]
    pub class: Option<String>,
    #[serde(default)]
    pub front: Option<String>,
    #[serde(default)]
    pub anchors: Vec<AssetAnchor>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub nsfw: f32,
    #[serde(default)]
    pub violence: f32,
    #[serde(default)]
    pub trademark: f32,
    #[serde(default)]
    pub suggested_name: Option<String>,
    /// Anchors the geometry check threw away, with the reason.
    #[serde(default)]
    pub rejected: Vec<String>,
}

pub fn anchor_prompt(
    views_png: Vec<Vec<u8>>,
    class: AssetClass,
    size_mm: [i32; 3],
    heuristic: &[AssetAnchor],
) -> AnchorPrompt {
    let system = "You annotate 3D assets for a small toon city with a 2 m tile grid and 3 m storeys. \
Coordinates are millimetres in the asset's local space: x east, y up, z south, origin at the centre of the base, feet on y = 0. \
Answer with one JSON object only, no prose."
        .to_string();
    let user = format!(
        "Four views of the asset: front (looking north, from +z), side (from +x), top, three-quarter.\n\
Declared class: {class}. Bounding box: {x} × {y} × {z} mm (x × y × z).\n\
Anchors found by geometry (may be wrong): {h}.\n\
Return: class (prop|building|character|decoration), front (n|e|s|w: side the asset faces), \
anchors (list of {{action, pos_mm:[x,y,z], yaw 0-255 clockwise from north = direction the person faces, capacity}}) \
where action is one of sit, sleep, work, stand, stack, door, talk, plant, water, harvest; every anchor must lie on a surface of the asset \
and a door must touch the ground; tags (short nouns); nsfw, violence, trademark (confidence 0-1); suggested_name.",
        class = class.as_str(),
        x = size_mm[0],
        y = size_mm[1],
        z = size_mm[2],
        h = serde_json::to_string(heuristic).unwrap_or_default(),
    );
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "class": {"type": "string", "enum": ["prop", "building", "character", "decoration"]},
            "front": {"type": "string", "enum": ["n", "e", "s", "w"]},
            "anchors": {"type": "array", "items": {"type": "object", "properties": {
                "action": {"type": "string"},
                "pos_mm": {"type": "array", "items": {"type": "integer"}, "minItems": 3, "maxItems": 3},
                "yaw": {"type": "integer", "minimum": 0, "maximum": 255},
                "capacity": {"type": "integer", "minimum": 1, "maximum": 16}
            }, "required": ["action", "pos_mm"]}},
            "tags": {"type": "array", "items": {"type": "string"}},
            "nsfw": {"type": "number"}, "violence": {"type": "number"}, "trademark": {"type": "number"},
            "suggested_name": {"type": "string"}
        },
        "required": ["front", "anchors"]
    });
    AnchorPrompt {
        system,
        user,
        images_png: views_png,
        schema,
    }
}

const ACTIONS: &[&str] = &[
    "sit", "sleep", "work", "stand", "stack", "door", "talk", "plant", "water", "harvest",
];

/// Parses the model's JSON (tolerates code fences) and keeps only anchors
/// that lie within 15 cm of the asset's surface; a door must touch y = 0.
pub fn parse_anchor_reply(
    reply: &str,
    tris: &[[[f32; 3]; 3]],
    bb: &Aabb,
) -> Result<VisionAnnotation, String> {
    let s = reply.trim();
    let s = s
        .find('{')
        .and_then(|a| s.rfind('}').map(|b| &s[a..=b]))
        .ok_or("the reply has no JSON object")?;
    let v: serde_json::Value = serde_json::from_str(s).map_err(|e| e.to_string())?;
    let mut ann = VisionAnnotation {
        class: v.get("class").and_then(|x| x.as_str()).map(String::from),
        front: v
            .get("front")
            .and_then(|x| x.as_str())
            .filter(|f| ["n", "e", "s", "w"].contains(f))
            .map(String::from),
        tags: v
            .get("tags")
            .and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|t| t.as_str())
                    .take(16)
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default(),
        nsfw: v.get("nsfw").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32,
        violence: v.get("violence").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32,
        trademark: v.get("trademark").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32,
        suggested_name: v
            .get("suggested_name")
            .and_then(|x| x.as_str())
            .map(|s| s.chars().take(64).collect()),
        ..Default::default()
    };
    for a in v
        .get("anchors")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default()
    {
        let action = a
            .get("action")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let pos: Vec<i64> = a
            .get("pos_mm")
            .and_then(|x| x.as_array())
            .map(|p| {
                p.iter()
                    .filter_map(|n| n.as_f64())
                    .map(|f| f as i64)
                    .collect()
            })
            .unwrap_or_default();
        if !ACTIONS.contains(&action.as_str()) || pos.len() != 3 {
            ann.rejected
                .push(format!("{action}: unknown action or bad position"));
            continue;
        }
        let p = [
            pos[0] as f32 / 1000.0,
            pos[1] as f32 / 1000.0,
            pos[2] as f32 / 1000.0,
        ];
        let inside = (0..3).all(|k| p[k] >= bb.min[k] - 0.1 && p[k] <= bb.max[k] + 0.1);
        if !inside {
            ann.rejected.push(format!("{action}: outside the asset"));
            continue;
        }
        if action == "door" {
            if p[1].abs() > 0.2 {
                ann.rejected.push("door: does not touch the ground".into());
                continue;
            }
        } else if distance_to_mesh(tris, p) > 0.15 {
            ann.rejected.push(format!("{action}: not on a surface"));
            continue;
        }
        if ann.anchors.len() >= MAX_ANCHORS {
            break;
        }
        ann.anchors.push(AssetAnchor {
            action,
            pos_mm: [pos[0] as i32, pos[1] as i32, pos[2] as i32],
            yaw: a.get("yaw").and_then(|x| x.as_u64()).unwrap_or(0).min(255) as u8,
            capacity: a
                .get("capacity")
                .and_then(|x| x.as_u64())
                .unwrap_or(1)
                .clamp(1, 16) as u8,
            source: "llm".into(),
        });
    }
    Ok(ann)
}

pub fn distance_to_mesh(tris: &[[[f32; 3]; 3]], p: [f32; 3]) -> f32 {
    tris.iter()
        .map(|t| point_triangle_distance(p, t))
        .fold(f32::INFINITY, f32::min)
}

fn point_triangle_distance(p: [f32; 3], t: &[[f32; 3]; 3]) -> f32 {
    // Ericson, closest point on triangle.
    let (a, b, c) = (t[0], t[1], t[2]);
    let ab = sub(b, a);
    let ac = sub(c, a);
    let ap = sub(p, a);
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    let closest = if d1 <= 0.0 && d2 <= 0.0 {
        a
    } else {
        let bp = sub(p, b);
        let d3 = dot(ab, bp);
        let d4 = dot(ac, bp);
        if d3 >= 0.0 && d4 <= d3 {
            b
        } else {
            let vc = d1 * d4 - d3 * d2;
            if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
                let v = d1 / (d1 - d3);
                [a[0] + ab[0] * v, a[1] + ab[1] * v, a[2] + ab[2] * v]
            } else {
                let cp = sub(p, c);
                let d5 = dot(ab, cp);
                let d6 = dot(ac, cp);
                if d6 >= 0.0 && d5 <= d6 {
                    c
                } else {
                    let vb = d5 * d2 - d1 * d6;
                    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
                        let w = d2 / (d2 - d6);
                        [a[0] + ac[0] * w, a[1] + ac[1] * w, a[2] + ac[2] * w]
                    } else {
                        let va = d3 * d6 - d5 * d4;
                        if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
                            let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
                            let bc = sub(c, b);
                            [b[0] + bc[0] * w, b[1] + bc[1] * w, b[2] + bc[2] * w]
                        } else {
                            let denom = 1.0 / (va + vb + vc);
                            let v = vb * denom;
                            let w = vc * denom;
                            [
                                a[0] + ab[0] * v + ac[0] * w,
                                a[1] + ab[1] * v + ac[1] * w,
                                a[2] + ab[2] * v + ac[2] * w,
                            ]
                        }
                    }
                }
            }
        }
    };
    len3(sub(p, closest))
}
