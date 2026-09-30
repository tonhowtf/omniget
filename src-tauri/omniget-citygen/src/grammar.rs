//! Building grammar (CGA reduced to the grid, estudo 09 §2). Owner: agent B.
//!
//! ```text
//! Lot            -> Setback(zone, relative to the front)       { Buildable }
//! Buildable      -> Plan(shape: rect | L | U | patio)           { Mass }
//! Mass           -> Levels(n)                                    { Level(k) }
//! Level(k >= 2)  -> step back the rear row, except the stair core
//! Level(0)       -> Door(front face, most central front cell)
//! Mass           -> StairCore(most central rear cell, 1x2, 0..n-1)   if n > 1
//! Facade(face)   -> against another volume ? blind
//!                 : street face ? bay per tile (shop window / door / balcony)
//!                 : side bay per tile
//! Roof(kind)     -> ridge along the longer side of the plan
//! tower          -> 1x1 or 2x2 at the front corner nearest the landmark, +1 level
//! ```
//!
//! Everything is integer tiles and levels. A `typology` (M5 script) only
//! changes the look: the massing, door and stairs still come from here, so
//! the building code keeps holding.

use crate::program::{Setback, Zone};
use crate::types::*;

pub const USES: &[&str] = &["casa", "loja-sobrado", "civico", "templo", "galpao"];
pub const SHAPES: &[&str] = &["rect", "L", "U", "patio"];
pub const ROOFS: &[&str] = &["duas-aguas", "quatro-aguas", "plano", "conico"];
pub const BAY_KINDS: &[&str] = &["janela", "vitrine", "porta", "sacada", "cega"];
/// Hard ceiling of levels the grammar builds (the zone limit is the checker's).
pub const MAX_FLOORS: u8 = 12;

/// Accept the accented spellings an LLM tends to write.
pub fn normalize_key(s: &str) -> String {
    match s {
        "galpão" => "galpao".into(),
        "pátio" => "patio".into(),
        "cívico" => "civico".into(),
        "cônico" => "conico".into(),
        "duas-águas" => "duas-aguas".into(),
        "quatro-águas" => "quatro-aguas".into(),
        other => other.to_string(),
    }
}

/// Minimum front setback of a zone kind (R4): residential streets get a 2 m
/// front yard, civic and commercial fronts sit on the sidewalk.
pub fn min_front_setback(kind: &str) -> i32 {
    match kind {
        "residencial" | "rural" => 1,
        _ => 0,
    }
}

/// The zone's setback raised to the building code minimums (R4).
pub fn effective_setback(zone: Option<&Zone>) -> Setback {
    let (sb, kind) = match zone {
        Some(z) => (z.setback.clone(), z.kind.as_str()),
        None => (Setback::default(), ""),
    };
    Setback {
        front: sb.front.max(min_front_setback(kind)),
        side: sb.side.max(0),
        back: sb.back.max(1),
    }
}

/// Local frame of a lot: `u` runs along the front, `v` from the front to the
/// back.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub rect: [i32; 4],
    pub front: Dir,
}

impl Frame {
    pub fn new(rect: [i32; 4], front: Dir) -> Self {
        Frame { rect, front }
    }
    /// (U, V): extent along the front and depth.
    pub fn dims(&self) -> (i32, i32) {
        match self.front {
            Dir::N | Dir::S => (self.rect[2], self.rect[3]),
            Dir::E | Dir::W => (self.rect[3], self.rect[2]),
        }
    }
    pub fn to_uv(&self, x: i32, y: i32) -> (i32, i32) {
        let [x0, y0, w, h] = self.rect;
        match self.front {
            Dir::N => (x - x0, y - y0),
            Dir::S => (x - x0, y0 + h - 1 - y),
            Dir::W => (y - y0, x - x0),
            Dir::E => (y - y0, x0 + w - 1 - x),
        }
    }
    pub fn to_xy(&self, u: i32, v: i32) -> (i32, i32) {
        let [x0, y0, w, h] = self.rect;
        match self.front {
            Dir::N => (x0 + u, y0 + v),
            Dir::S => (x0 + u, y0 + h - 1 - v),
            Dir::W => (x0 + v, y0 + u),
            Dir::E => (x0 + w - 1 - v, y0 + u),
        }
    }
}

/// Buildable area in the lot frame after setbacks: `[u0, v0, du, dv]`
/// (du or dv can be <= 0 when nothing fits).
pub fn buildable_uv(frame: &Frame, sb: &Setback) -> [i32; 4] {
    let (u, v) = frame.dims();
    [sb.side, sb.front, u - 2 * sb.side, v - sb.front - sb.back]
}

/// Buildable area in region tiles `[x, y, w, h]` (w or h can be <= 0).
pub fn buildable_rect(frame: &Frame, sb: &Setback) -> [i32; 4] {
    let [u0, v0, du, dv] = buildable_uv(frame, sb);
    if du <= 0 || dv <= 0 {
        let (x, y) = frame.to_xy(u0.max(0), v0.max(0));
        let (w, h) = match frame.front {
            Dir::N | Dir::S => (du, dv),
            _ => (dv, du),
        };
        return [x, y, w, h];
    }
    let (ax, ay) = frame.to_xy(u0, v0);
    let (bx, by) = frame.to_xy(u0 + du - 1, v0 + dv - 1);
    [
        ax.min(bx),
        ay.min(by),
        (ax - bx).abs() + 1,
        (ay - by).abs() + 1,
    ]
}

/// Door cell of a migrated door in the lot frame: a tile outside the front
/// edge means the cell right behind it.
pub fn fixed_door_uv(frame: &Frame, d: [i32; 2]) -> (i32, i32) {
    let (u, v) = frame.to_uv(d[0], d[1]);
    (u, v.max(0))
}

/// Setback a building on this lot gets: the effective zone setback, with the
/// front pinned by a migrated door.
pub fn setback_for(lot: &Lot, zone: Option<&Zone>, fixed_door: Option<[i32; 2]>) -> Setback {
    let frame = Frame::new(lot.rect, lot.front);
    let mut sb = effective_setback(zone);
    if let Some(d) = fixed_door {
        let (_, lot_v) = frame.dims();
        sb.front = fixed_door_uv(&frame, d).1.clamp(0, (lot_v - 1).max(0));
    }
    sb
}

/// Everything the grammar needs about one building, already resolved by the
/// solver (lot chosen, enums defaulted).
pub struct Spec<'a> {
    pub id: &'a str,
    pub lot: &'a Lot,
    pub zone: Option<&'a Zone>,
    pub use_: String,
    pub floors: u8,
    pub shape: String,
    pub roof: String,
    pub style: String,
    pub palette: String,
    pub tower: bool,
    /// Door tile fixed by a migrated lot: inside the lot = the door cell,
    /// outside = the tile the door opens onto.
    pub fixed_door: Option<[i32; 2]>,
    /// Where the tower leans to (center of the landmark of `auto:near`).
    pub tower_toward: Option<[i32; 2]>,
    pub typology: Option<String>,
    pub progress: u8,
    pub owner_ref: Option<String>,
}

fn central(n: i32, u: i32) -> i32 {
    (2 * u - (n - 1)).abs()
}

/// Lot → building massing. Bays are filled later by [`facades`], once every
/// building of the region is known.
pub fn build(s: &Spec) -> Building {
    let frame = Frame::new(s.lot.rect, s.lot.front);
    let (lot_u, lot_v) = frame.dims();
    let sb = setback_for(s.lot, s.zone, s.fixed_door);
    // A migrated door pins the front of the building.
    let fixed = s.fixed_door.map(|d| fixed_door_uv(&frame, d));
    let [u0, v0, du, dv] = buildable_uv(&frame, &sb);
    let patio_ok = s.zone.map(|z| z.kind == "civico").unwrap_or(false) && du >= 8 && dv >= 8;

    // Plan: (u, v) in lot frame, lu/lv inside the buildable area.
    let mut plan: Vec<(i32, i32)> = Vec::new();
    for lv in 0..dv.max(0) {
        for lu in 0..du.max(0) {
            let keep = match s.shape.as_str() {
                "L" => !(lv >= dv / 2 && lu >= du / 2),
                "U" => !(lv >= dv / 2 && lu >= du / 3 && lu < du - du / 3),
                "patio" if patio_ok => !(lu >= 2 && lu < du - 2 && lv >= 2 && lv < dv - 2),
                _ => true,
            };
            if keep {
                plan.push((u0 + lu, v0 + lv));
            }
        }
    }

    // Door: the most central front cell, or the migrated one.
    let door_uv = match fixed {
        Some((u, v)) => {
            let uv = (u.clamp(0, lot_u - 1), v);
            if !plan.contains(&uv) {
                plan.push(uv);
            }
            uv
        }
        None => {
            let front_v = plan
                .iter()
                .map(|c| c.1)
                .min()
                .unwrap_or(v0.clamp(0, lot_v - 1));
            plan.iter()
                .filter(|c| c.1 == front_v)
                .min_by_key(|c| (central(lot_u, c.0), c.0))
                .copied()
                .unwrap_or(((lot_u - 1) / 2, front_v))
        }
    };
    let floors = s.floors.clamp(1, MAX_FLOORS);

    // Stair core: most central rear cell plus the one in front of it (1x2).
    let back_v = plan.iter().map(|c| c.1).max().unwrap_or(0);
    let front_min = plan.iter().map(|c| c.1).min().unwrap_or(0);
    let mut stair_uv: Vec<(i32, i32)> = Vec::new();
    if floors > 1 {
        if let Some(&c) = plan
            .iter()
            .filter(|c| c.1 == back_v)
            .min_by_key(|c| (central(lot_u, c.0), c.0))
        {
            stair_uv.push(c);
            let second = (c.0, c.1 - 1);
            if plan.contains(&second) && second != door_uv {
                stair_uv.push(second);
            }
        }
    }

    // Levels, with the step back from level 2 up.
    let to_xy = |c: &(i32, i32)| {
        let (x, y) = frame.to_xy(c.0, c.1);
        [x, y]
    };
    let mut levels: Vec<Vec<[i32; 2]>> = Vec::new();
    for k in 0..floors {
        let mut cells: Vec<[i32; 2]> = plan
            .iter()
            .filter(|c| k < 2 || c.1 != back_v || stair_uv.contains(c) || back_v == front_min)
            .map(to_xy)
            .collect();
        cells.sort_by_key(|c| (c[1], c[0]));
        levels.push(cells);
    }

    // Tower on the front corner nearest the landmark.
    let mut tower = None;
    if s.tower && !plan.is_empty() {
        let top = levels.last().cloned().unwrap_or_default();
        let front_v = plan.iter().map(|c| c.1).min().unwrap_or(0);
        let umin = plan
            .iter()
            .filter(|c| c.1 == front_v)
            .map(|c| c.0)
            .min()
            .unwrap_or(0);
        let umax = plan
            .iter()
            .filter(|c| c.1 == front_v)
            .map(|c| c.0)
            .max()
            .unwrap_or(0);
        let size = if du >= 6 && dv >= 6 { 2 } else { 1 };
        let corners = [umin, (umax - size + 1).max(umin)];
        let pick = match s.tower_toward {
            Some(t) => *corners
                .iter()
                .min_by_key(|&&u| {
                    let (x, y) = frame.to_xy(u, front_v);
                    let (dx, dy) = (2 * x + size - 1 - 2 * t[0], 2 * y + size - 1 - 2 * t[1]);
                    (dx * dx + dy * dy, u)
                })
                .unwrap(),
            None => corners[0],
        };
        let mut cells: Vec<[i32; 2]> = Vec::new();
        for dvv in 0..size {
            for duu in 0..size {
                let c = to_xy(&(pick + duu, front_v + dvv));
                if top.contains(&c) {
                    cells.push(c);
                }
            }
        }
        cells.sort_by_key(|c| (c[1], c[0]));
        if !cells.is_empty() {
            levels.push(cells.clone());
            tower = Some(Tower {
                cells,
                from_level: floors,
                levels: 1,
            });
        }
    }

    let dc = to_xy(&door_uv);
    let (ddx, ddy) = s.lot.front.delta();
    let door = Door {
        cell: dc,
        outside: [dc[0] + ddx, dc[1] + ddy],
        face: s.lot.front,
        level: 0,
    };
    let mut stairs: Vec<[i32; 2]> = stair_uv.iter().map(to_xy).collect();
    stairs.sort_by_key(|c| (c[1], c[0]));

    // Ridge along the longer side of the ground plan.
    let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for c in levels.first().map(|l| l.as_slice()).unwrap_or(&[]) {
        x0 = x0.min(c[0]);
        y0 = y0.min(c[1]);
        x1 = x1.max(c[0]);
        y1 = y1.max(c[1]);
    }
    let ridge_axis = if x1 - x0 >= y1 - y0 { Axis::X } else { Axis::Y };

    Building {
        id: s.id.to_string(),
        lot: s.lot.id.clone(),
        style: s.style.clone(),
        palette: s.palette.clone(),
        use_: s.use_.clone(),
        floors,
        levels,
        door,
        stairs,
        roof: Roof {
            kind: s.roof.clone(),
            ridge_axis,
        },
        bays: Vec::new(),
        tower,
        typology: s.typology.clone(),
        progress: s.progress.min(100),
        owner_ref: s.owner_ref.clone(),
    }
}

/// Is this use a shop at street level?
pub fn is_shop(use_: &str) -> bool {
    use_.starts_with("loja") || use_ == "mercado"
}

/// Bays per face of every exterior wall tile, per level.
/// `other(k, x, y)` says whether another building occupies that voxel (blind
/// wall), `public(x, y)` whether a tile is street, sidewalk, plaza or bridge.
pub fn facades(
    b: &Building,
    balcony: bool,
    other: &dyn Fn(usize, i32, i32) -> bool,
    public: &dyn Fn(i32, i32) -> bool,
) -> Vec<Bay> {
    let mut bays = Vec::new();
    let shop = is_shop(&b.use_);
    for (k, cells) in b.levels.iter().enumerate() {
        let tower_level = k >= b.floors as usize;
        for c in cells {
            for face in [Dir::N, Dir::E, Dir::S, Dir::W] {
                let (dx, dy) = face.delta();
                let (nx, ny) = (c[0] + dx, c[1] + dy);
                if cells.iter().any(|o| o[0] == nx && o[1] == ny) {
                    continue;
                }
                let kind = if other(k, nx, ny) {
                    "cega"
                } else if tower_level {
                    "janela"
                } else if face == b.door.face || public(nx, ny) {
                    if k == 0 && *c == b.door.cell && face == b.door.face {
                        "porta"
                    } else if k == 0 && shop {
                        "vitrine"
                    } else if k == 1 && balcony {
                        "sacada"
                    } else {
                        "janela"
                    }
                } else {
                    "janela"
                };
                bays.push(Bay {
                    level: k as u8,
                    cell: *c,
                    face,
                    kind: kind.into(),
                });
            }
        }
    }
    bays
}
