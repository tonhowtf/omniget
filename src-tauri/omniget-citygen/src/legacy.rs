//! The 2D city running in production → charter + program (M8). Owner: agent G1.
//!
//! Two inputs are accepted, both as JSON:
//! - the workshop's `offline-world.v1` (`world.json`) with its chunks inlined
//!   under `chunks` (an array of `{tiles, objects}` or a map of them), or
//!   `{"world": <world.json>, "chunks": …}`;
//! - the server's flat bundle `omni.city.legacy/v1`, assembled from what the
//!   server imported (`plots`, region maps, `farm_cells`, `homes`):
//!   `{schema, city:{id,name,seed}, plots:[{id,rect,door,kind,region?,
//!   building_id?,style_variant?,name?}], tiles:[{xy,frame}],
//!   objects:[{id,asset,xy,kind?,plot_id?,footprint?}], farm:[{id,plot_id,
//!   xy,crop?,stage}], homes:[{plot_id,home_id}], landmarks:[{id,rect,entry?}]}`.
//!
//! Nothing moves: every plot becomes an explicit lot with the same id, rect,
//! front and door, every house a building on it, the owner (`home:<id>`) is
//! carried as `owner_ref`. Roads, water and plazas become streets, water
//! bodies and landmarks by exact rectangle decomposition (street widths use
//! the prototype's convention: tiles `path + lo..=path + lo + w - 1` with
//! `lo = -((w - 1) >> 1)`). Trees, fences, rocks, flowers, the well, lamp and
//! bench become props; farm beds become `canteiro` props (anchors
//! plant/water/harvest) on their exact tiles. Coordinates in the program are
//! region-local (global − region × 64).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::program::*;
use crate::types::*;

pub const LEGACY_BUNDLE_SCHEMA: &str = "omni.city.legacy/v1";

/// Legacy palettes of the house finish (`house_variant` 0..2).
pub const HOUSE_PALETTES: [&str; 3] = ["terracotta", "breeze", "sand"];

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct LegacyReport {
    pub city_id: String,
    pub regions: Vec<RegionReport>,
    pub lots: usize,
    pub buildings: usize,
    pub canteiros: usize,
    pub doors: Vec<LotDoor>,
    pub owners: Vec<LotOwner>,
    /// Anything the conversion could not carry exactly, in words.
    pub differences: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct RegionReport {
    pub region: [i32; 2],
    pub lots: usize,
    pub buildings: usize,
    pub streets: usize,
    pub road_tiles: usize,
    pub water: usize,
    pub water_tiles: usize,
    pub landmarks: usize,
    pub props: usize,
    pub canteiros: usize,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LotDoor {
    pub lot: String,
    pub region: [i32; 2],
    /// Global tile.
    pub door: [i32; 2],
    pub front: Dir,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LotOwner {
    pub lot: String,
    pub owner_ref: String,
}

#[derive(Clone, Debug)]
pub struct LegacyConversion {
    pub charter: Charter,
    pub programs: Vec<CityProgram>,
    pub report: LegacyReport,
}

pub fn from_legacy(package: &serde_json::Value) -> Result<(Charter, Vec<CityProgram>), String> {
    convert(package).map(|c| (c.charter, c.programs))
}

// ── Normalised input ───────────────────────────────────────────────────────

#[derive(Clone, Debug)]
struct LPlot {
    id: String,
    rect: [i32; 4],
    door: [i32; 2],
    kind: String,
    region: Option<[i32; 2]>,
    building_id: Option<String>,
    style_variant: i32,
    name: Option<String>,
    house: Option<[i32; 2]>,
}

#[derive(Clone, Debug)]
struct LObject {
    asset: String,
    xy: [i32; 2],
    kind: String,
    plot_id: Option<String>,
}

#[derive(Clone, Debug)]
struct LCell {
    xy: [i32; 2],
    crop: Option<String>,
}

#[derive(Default)]
struct Legacy {
    city_id: String,
    name: String,
    seed: String,
    plots: Vec<LPlot>,
    tiles: BTreeMap<(i32, i32), String>,
    objects: Vec<LObject>,
    cells: Vec<LCell>,
    homes: BTreeMap<String, String>,
    landmarks: Vec<(String, [i32; 4])>,
}

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(|x| x.as_str()).map(|x| x.to_string())
}

fn pair(v: &Value) -> Option<[i32; 2]> {
    let a = v.as_array()?;
    Some([a.first()?.as_i64()? as i32, a.get(1)?.as_i64()? as i32])
}

fn quad(v: &Value) -> Option<[i32; 4]> {
    let a = v.as_array()?;
    if a.len() < 4 {
        return None;
    }
    Some([
        a[0].as_i64()? as i32,
        a[1].as_i64()? as i32,
        a[2].as_i64()? as i32,
        a[3].as_i64()? as i32,
    ])
}

/// `"city/r:1:-2"` → `[1, -2]`.
fn parse_region(id: &str) -> Option<[i32; 2]> {
    let tail = id.rsplit("/r:").next()?;
    let mut it = tail.split(':');
    Some([it.next()?.parse().ok()?, it.next()?.parse().ok()?])
}

/// What a legacy object is, from its explicit kind or its asset key.
fn object_kind(kind: Option<&str>, asset: &str) -> String {
    if let Some(k) = kind {
        if !k.is_empty() {
            return k.to_string();
        }
    }
    if asset.starts_with("crop/") {
        return "crop".into();
    }
    let leaf = asset.rsplit('/').next().unwrap_or(asset);
    match leaf {
        "soil-empty" => "crop".into(),
        other => other.to_string(),
    }
}

fn normalise(v: &Value) -> Result<Legacy, String> {
    let mut l = Legacy::default();
    if v.get("schema").and_then(|s| s.as_str()) == Some(LEGACY_BUNDLE_SCHEMA) {
        let city = v.get("city").ok_or("bundle has no city")?;
        l.city_id = s(city, "id").ok_or("bundle city has no id")?;
        l.name = s(city, "name").unwrap_or_else(|| l.city_id.clone());
        l.seed = s(city, "seed").unwrap_or_else(|| l.city_id.clone());
        for p in v
            .get("plots")
            .and_then(|p| p.as_array())
            .into_iter()
            .flatten()
        {
            l.plots.push(LPlot {
                id: s(p, "id").ok_or("plot without id")?,
                rect: p.get("rect").and_then(quad).ok_or("plot without rect")?,
                door: p.get("door").and_then(pair).ok_or("plot without door")?,
                kind: s(p, "kind").unwrap_or_else(|| "residential".into()),
                region: s(p, "region")
                    .and_then(|r| parse_region(&r))
                    .or_else(|| p.get("region").and_then(pair)),
                building_id: s(p, "building_id"),
                style_variant: p.get("style_variant").and_then(|x| x.as_i64()).unwrap_or(0) as i32,
                name: s(p, "name"),
                house: p.get("house").and_then(pair),
            });
        }
        for t in v
            .get("tiles")
            .and_then(|t| t.as_array())
            .into_iter()
            .flatten()
        {
            if let (Some(xy), Some(frame)) = (t.get("xy").and_then(pair), s(t, "frame")) {
                l.tiles.insert((xy[0], xy[1]), frame);
            }
        }
        for o in v
            .get("objects")
            .and_then(|o| o.as_array())
            .into_iter()
            .flatten()
        {
            let asset = s(o, "asset").unwrap_or_default();
            let xy = o
                .get("footprint")
                .and_then(quad)
                .map(|f| [f[0], f[1]])
                .or_else(|| o.get("xy").and_then(pair));
            let Some(xy) = xy else { continue };
            l.objects.push(LObject {
                kind: object_kind(o.get("kind").and_then(|k| k.as_str()), &asset),
                asset,
                xy,
                plot_id: s(o, "plot_id"),
            });
        }
        for c in v
            .get("farm")
            .and_then(|c| c.as_array())
            .into_iter()
            .flatten()
        {
            if let Some(xy) = c.get("xy").and_then(pair) {
                l.cells.push(LCell {
                    xy,
                    crop: s(c, "crop"),
                });
            }
        }
        for h in v
            .get("homes")
            .and_then(|h| h.as_array())
            .into_iter()
            .flatten()
        {
            if let Some(plot) = s(h, "plot_id") {
                let id = h
                    .get("home_id")
                    .map(|x| match x {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    })
                    .unwrap_or_default();
                if !id.is_empty() {
                    l.homes.insert(plot, id);
                }
            }
        }
        for m in v
            .get("landmarks")
            .and_then(|m| m.as_array())
            .into_iter()
            .flatten()
        {
            if let (Some(id), Some(rect)) = (s(m, "id"), m.get("rect").and_then(quad)) {
                l.landmarks.push((id, rect));
            }
        }
        return Ok(l);
    }

    // offline-world.v1, bare or wrapped.
    let world = v.get("world").unwrap_or(v);
    let city = world.get("city").ok_or("world.json has no city")?;
    l.city_id = s(city, "id").ok_or("world city has no id")?;
    l.name = s(city, "name").unwrap_or_else(|| l.city_id.clone());
    l.seed = s(city, "seed").unwrap_or_else(|| l.city_id.clone());
    let chunks: Vec<&Value> = match v.get("chunks").or_else(|| world.get("chunks")) {
        Some(Value::Array(a)) => a.iter().collect(),
        Some(Value::Object(m)) => m.values().collect(),
        _ => Vec::new(),
    };
    let mut house_of: BTreeMap<String, [i32; 2]> = BTreeMap::new();
    for c in &chunks {
        for t in c
            .get("tiles")
            .and_then(|t| t.as_array())
            .into_iter()
            .flatten()
        {
            if let (Some(xy), Some(frame)) = (t.get("xy").and_then(pair), s(t, "frame")) {
                l.tiles.insert((xy[0], xy[1]), frame);
            }
        }
        for o in c
            .get("objects")
            .and_then(|o| o.as_array())
            .into_iter()
            .flatten()
        {
            let asset = s(o, "asset").unwrap_or_default();
            let kind = object_kind(o.get("kind").and_then(|k| k.as_str()), &asset);
            let xy = o
                .get("footprint")
                .and_then(quad)
                .map(|f| [f[0], f[1]])
                .or_else(|| o.get("xy").and_then(pair));
            let Some(xy) = xy else { continue };
            if kind == "house" {
                if let Some(p) = s(o, "plot_id") {
                    house_of.insert(p, xy);
                }
            }
            l.objects.push(LObject {
                asset,
                xy,
                kind,
                plot_id: s(o, "plot_id"),
            });
        }
    }
    for p in world
        .get("plots")
        .and_then(|p| p.as_array())
        .into_iter()
        .flatten()
    {
        let id = s(p, "id").ok_or("plot without id")?;
        for c in p
            .get("farm")
            .and_then(|f| f.get("cells"))
            .and_then(|c| c.as_array())
            .into_iter()
            .flatten()
        {
            if let Some(xy) = c.get("xy").and_then(pair) {
                l.cells.push(LCell {
                    xy,
                    crop: s(c, "crop"),
                });
            }
        }
        l.plots.push(LPlot {
            house: house_of.get(&id).copied(),
            rect: p.get("rect").and_then(quad).ok_or("plot without rect")?,
            door: p.get("door").and_then(pair).ok_or("plot without door")?,
            kind: s(p, "kind").unwrap_or_else(|| "residential".into()),
            region: None,
            building_id: s(p, "building_id"),
            style_variant: p.get("style_variant").and_then(|x| x.as_i64()).unwrap_or(0) as i32,
            name: s(p, "name"),
            id,
        });
    }
    // The chunk crop sprites duplicate the plots' farm cells; keep the plots'.
    if !l.cells.is_empty() {
        l.objects.retain(|o| o.kind != "crop");
    }
    for m in world
        .get("landmarks")
        .and_then(|m| m.as_array())
        .into_iter()
        .flatten()
    {
        if let (Some(id), Some(rect)) = (s(m, "id"), m.get("rect").and_then(quad)) {
            l.landmarks.push((id, rect));
        }
    }
    if l.plots.is_empty() {
        return Err("no plots in the legacy package".into());
    }
    Ok(l)
}

// ── Conversion ─────────────────────────────────────────────────────────────

fn block_of(x: i32, y: i32) -> [i32; 2] {
    [x.div_euclid(REGION_TILES), y.div_euclid(REGION_TILES)]
}

fn local(g: [i32; 2], region: [i32; 2]) -> [i32; 2] {
    [
        g[0] - region[0] * REGION_TILES,
        g[1] - region[1] * REGION_TILES,
    ]
}

/// Which side of the rect the door is on.
fn front_of(rect: [i32; 4], door: [i32; 2]) -> Dir {
    let [x, y, w, h] = rect;
    let [dx, dy] = door;
    if dy >= y + h {
        Dir::S
    } else if dy < y {
        Dir::N
    } else if dx >= x + w {
        Dir::E
    } else if dx < x {
        Dir::W
    } else {
        // Inside the rect: the nearest edge.
        let d = [
            (dy - y, Dir::N),
            (y + h - 1 - dy, Dir::S),
            (dx - x, Dir::W),
            (x + w - 1 - dx, Dir::E),
        ];
        d.iter()
            .min_by_key(|(n, _)| *n)
            .map(|(_, d)| *d)
            .unwrap_or(Dir::S)
    }
}

/// Exact cover of a tile set by maximal rectangles, row-major greedy:
/// deterministic for a given set. `[x, y, w, h]`.
pub fn rectangles(tiles: &BTreeSet<(i32, i32)>) -> Vec<[i32; 4]> {
    let mut left: BTreeSet<(i32, i32)> = tiles.clone();
    let mut out = Vec::new();
    // Row-major order: sort by (y, x).
    let order: Vec<(i32, i32)> = {
        let mut v: Vec<(i32, i32)> = tiles.iter().copied().collect();
        v.sort_by_key(|(x, y)| (*y, *x));
        v
    };
    for (x0, y0) in order {
        if !left.contains(&(x0, y0)) {
            continue;
        }
        let mut w = 1;
        while left.contains(&(x0 + w, y0)) {
            w += 1;
        }
        let mut h = 1;
        'grow: loop {
            for x in x0..x0 + w {
                if !left.contains(&(x, y0 + h)) {
                    break 'grow;
                }
            }
            h += 1;
        }
        for y in y0..y0 + h {
            for x in x0..x0 + w {
                left.remove(&(x, y));
            }
        }
        out.push([x0, y0, w, h]);
    }
    out
}

/// A rectangle of road as a street polyline with the prototype's width
/// convention (`lo = -((w - 1) >> 1)`).
fn street_of(id: String, r: [i32; 4]) -> Street {
    let [x, y, w, h] = r;
    let (width, path) = if w >= h {
        let row = y + ((h - 1) >> 1);
        (h, vec![[x, row], [x + w - 1, row]])
    } else {
        let col = x + ((w - 1) >> 1);
        (w, vec![[col, y], [col, y + h - 1]])
    };
    // The 2D city reaches houses by long one-tile paths: a `trilha` has no
    // length limit, a `beco` does.
    let class = match width {
        1 => "trilha",
        2 => "rua",
        _ => "avenida",
    };
    Street {
        id,
        class: class.into(),
        width,
        path,
    }
}

fn prop_kind(kind: &str) -> Option<(&'static str, &'static str)> {
    Some(match kind {
        "tree" => ("arvore", "oak-estilizada"),
        "fence" => ("cerca", "madeira"),
        "rock" => ("pedra", "seixo"),
        "flowers" => ("flores", "canteiro-baixo"),
        "lamp" => ("poste", "lampiao"),
        "well" => ("poco", "pedra"),
        "bench" => ("banco", "madeira"),
        _ => return None,
    })
}

fn palettes() -> Vec<Palette> {
    let mk = |key: &str, wall: &str, roof: &str| {
        let mut slots = BTreeMap::new();
        for (slot, color) in [
            ("wall", wall),
            ("roof", roof),
            ("trim", "#f4efe4"),
            ("wood", "#8a5a3b"),
            ("stone", "#9a948a"),
            ("glass", "#a9c6d6"),
            ("door", "#6b4431"),
            ("ground", "#b9a27a"),
            ("grass", "#7fa35a"),
            ("water", "#4f8fb8"),
            ("street", "#c9b99a"),
            ("plaza", "#d8ccb4"),
            ("leaf", "#5f8f3e"),
            ("trunk", "#6e4b32"),
            ("metal", "#5a5f66"),
            ("cloth", "#c7584b"),
        ] {
            slots.insert(slot.to_string(), color.to_string());
        }
        Palette {
            key: key.into(),
            slots,
        }
    };
    vec![
        mk(HOUSE_PALETTES[0], "#ffffff", "#b5563a"),
        mk(HOUSE_PALETTES[1], "#cddfee", "#7a5c4a"),
        mk(HOUSE_PALETTES[2], "#efd3b8", "#9b4a32"),
    ]
}

/// Convert, with the report the migration shows before it writes anything.
pub fn convert(package: &Value) -> Result<LegacyConversion, String> {
    let l = normalise(package)?;
    let mut report = LegacyReport {
        city_id: l.city_id.clone(),
        ..Default::default()
    };

    // Region of each plot: the server's, else the house's block (the rule
    // the importer used), else the rect's corner + 1.
    let plot_region = |p: &LPlot| -> [i32; 2] {
        if let Some(r) = p.region {
            return r;
        }
        let h = p.house.unwrap_or([p.rect[0] + 1, p.rect[1] + 1]);
        block_of(h[0], h[1])
    };

    let mut regions: BTreeSet<[i32; 2]> = BTreeSet::new();
    for p in &l.plots {
        regions.insert(plot_region(p));
    }
    for (x, y) in l.tiles.keys() {
        regions.insert(block_of(*x, *y));
    }

    let mut programs = Vec::new();
    for &region in &regions {
        let mut prog = CityProgram::empty(&l.city_id, region);
        let mut rr = RegionReport {
            region,
            ..Default::default()
        };
        let zone_id = format!("bairro:{}:{}", region[0], region[1]);
        prog.zones.push(Zone {
            id: zone_id.clone(),
            kind: "residencial".into(),
            rect: [0, 0, REGION_TILES, REGION_TILES],
            style: "vila-colonial".into(),
            floors: [1, 2],
            density: 500,
            setback: Setback::default(),
            palette: Some(HOUSE_PALETTES[0].into()),
        });
        prog.lots.auto = false;

        // Tiles of this region by class, local coordinates.
        let mut road = BTreeSet::new();
        let mut bridge = BTreeSet::new();
        let mut water = BTreeSet::new();
        let mut plaza = BTreeSet::new();
        for ((x, y), frame) in &l.tiles {
            if block_of(*x, *y) != region {
                continue;
            }
            let t = local([*x, *y], region);
            let t = (t[0], t[1]);
            // A path tile inside a plot is the plot's own walk from the door
            // to the house: it stays yard, never a public street.
            let in_plot = l.plots.iter().any(|p| {
                *x >= p.rect[0]
                    && *x < p.rect[0] + p.rect[2]
                    && *y >= p.rect[1]
                    && *y < p.rect[1] + p.rect[3]
            });
            if frame.starts_with("ground/road") {
                if !in_plot {
                    road.insert(t);
                }
            } else if frame.starts_with("ground/bridge") {
                road.insert(t);
                bridge.insert(t);
            } else if frame.starts_with("ground/water") {
                water.insert(t);
            } else if frame.starts_with("ground/plaza") {
                plaza.insert(t);
            }
        }
        rr.road_tiles = road.len();
        rr.water_tiles = water.len();
        for (n, r) in rectangles(&road).into_iter().enumerate() {
            prog.streets
                .push(street_of(format!("rua:{}:{}:{n}", region[0], region[1]), r));
        }
        for (n, r) in rectangles(&water).into_iter().enumerate() {
            // Streets with a bridge tile touching this body cross it.
            let [wx, wy, ww, wh] = r;
            let mut bridges = Vec::new();
            for st in &prog.streets {
                let touches = bridge.iter().any(|(bx, by)| {
                    *bx >= wx - 1
                        && *bx <= wx + ww
                        && *by >= wy - 1
                        && *by <= wy + wh
                        && on_street(st, *bx, *by)
                });
                if touches {
                    bridges.push(st.id.clone());
                }
            }
            prog.water.push(WaterBody {
                id: format!("agua:{}:{}:{n}", region[0], region[1]),
                rect: r,
                bridges,
            });
        }
        // Plazas: the named landmarks first, then any plaza tile they miss.
        for (id, rect) in &l.landmarks {
            if block_of(rect[0], rect[1]) != region {
                continue;
            }
            let o = local([rect[0], rect[1]], region);
            let lr = [o[0], o[1], rect[2], rect[3]];
            for y in lr[1]..lr[1] + lr[3] {
                for x in lr[0]..lr[0] + lr[2] {
                    plaza.remove(&(x, y));
                }
            }
            prog.landmarks.push(Landmark {
                id: id.clone(),
                kind: "praca".into(),
                rect: lr,
            });
        }
        for (n, r) in rectangles(&plaza).into_iter().enumerate() {
            prog.landmarks.push(Landmark {
                id: format!("praca:{}:{}:{n}", region[0], region[1]),
                kind: "praca".into(),
                rect: r,
            });
        }

        // Lots and houses.
        let mut plots: Vec<&LPlot> = l
            .plots
            .iter()
            .filter(|p| plot_region(p) == region)
            .collect();
        plots.sort_by(|a, b| a.id.cmp(&b.id));
        let houses: BTreeSet<&str> = l
            .objects
            .iter()
            .filter(|o| o.kind == "house")
            .filter_map(|o| o.plot_id.as_deref())
            .collect();
        for p in plots {
            let o = local([p.rect[0], p.rect[1]], region);
            let rect = [o[0], o[1], p.rect[2], p.rect[3]];
            let door = local(p.door, region);
            let front = front_of(p.rect, p.door);
            if rect[0] < 0
                || rect[1] < 0
                || rect[0] + rect[2] > REGION_TILES
                || rect[1] + rect[3] > REGION_TILES
            {
                report.differences.push(format!(
                    "lot {} crosses the edge of region {:?}",
                    p.id, region
                ));
            }
            if door[0] < 0 || door[1] < 0 || door[0] >= REGION_TILES || door[1] >= REGION_TILES {
                report.differences.push(format!(
                    "door of lot {} is in the next region ({:?})",
                    p.id, p.door
                ));
            }
            let owner_ref = l.homes.get(&p.id).map(|h| format!("home:{h}"));
            if let Some(o) = &owner_ref {
                report.owners.push(LotOwner {
                    lot: p.id.clone(),
                    owner_ref: o.clone(),
                });
            }
            report.doors.push(LotDoor {
                lot: p.id.clone(),
                region,
                door: p.door,
                front,
            });
            prog.lots.explicit.push(ExplicitLot {
                id: p.id.clone(),
                rect,
                front,
                zone: Some(zone_id.clone()),
                owner_ref: owner_ref.clone(),
                door: Some(door),
            });
            rr.lots += 1;
            let has_house =
                houses.contains(p.id.as_str()) || p.building_id.is_some() || houses.is_empty();
            if has_house {
                let palette = HOUSE_PALETTES[(p.style_variant.clamp(0, 2)) as usize];
                prog.buildings.push(BuildingSpec {
                    id: p
                        .building_id
                        .clone()
                        .unwrap_or_else(|| format!("building:{}", p.id)),
                    lot: p.id.clone(),
                    owner_ref,
                    intent: Intent {
                        use_: "casa".into(),
                        floors: Some(1),
                        shape: Some("rect".into()),
                        roof: Some("duas-aguas".into()),
                        palette: Some(palette.into()),
                        style: None,
                        label: p.name.clone(),
                        chimney: p.kind == "farm",
                        ..Default::default()
                    },
                    typology: None,
                    progress: 100,
                });
                rr.buildings += 1;
            } else {
                report
                    .differences
                    .push(format!("lot {} has no house", p.id));
            }
        }

        // Props, grouped by (kind, preset), cells sorted.
        let mut groups: BTreeMap<(String, String), BTreeSet<[i32; 2]>> = BTreeMap::new();
        let mut unknown: BTreeMap<String, usize> = BTreeMap::new();
        for o in &l.objects {
            if block_of(o.xy[0], o.xy[1]) != region || o.kind == "house" || o.kind == "crop" {
                continue;
            }
            match prop_kind(&o.kind) {
                Some((k, preset)) => {
                    groups
                        .entry((k.into(), preset.into()))
                        .or_default()
                        .insert(local(o.xy, region));
                }
                None => {
                    *unknown
                        .entry(format!("{} ({})", o.kind, o.asset))
                        .or_default() += 1
                }
            }
        }
        for (what, n) in unknown {
            report.differences.push(format!(
                "{n} object(s) of unknown kind {what} in region {:?} were not carried",
                region
            ));
        }
        for c in &l.cells {
            if block_of(c.xy[0], c.xy[1]) != region {
                continue;
            }
            let preset = c.crop.clone().unwrap_or_else(|| "vazio".into());
            groups
                .entry(("canteiro".into(), preset))
                .or_default()
                .insert(local(c.xy, region));
            rr.canteiros += 1;
        }
        for ((kind, preset), cells) in groups {
            rr.props += cells.len();
            prog.props.push(PropSpec {
                kind,
                preset,
                cells: cells.into_iter().collect(),
                scatter: None,
            });
        }

        rr.streets = prog.streets.len();
        rr.water = prog.water.len();
        rr.landmarks = prog.landmarks.len();
        report.lots += rr.lots;
        report.buildings += rr.buildings;
        report.canteiros += rr.canteiros;
        report.regions.push(rr);
        programs.push(prog);
    }

    let charter = Charter {
        schema: "omni.city.charter/v1".into(),
        city_id: l.city_id.clone(),
        name: l.name.clone(),
        seed: l.seed.clone(),
        generator_version: GENERATOR_VERSION.into(),
        story: format!(
            "{} was a 2D city before the 3D one. It was converted lot by lot: every door, owner and farm bed stayed where it was.",
            l.name
        ),
        climate: Climate::Temperate,
        culture: String::new(),
        style: "vila-colonial".into(),
        palette: HOUSE_PALETTES[0].into(),
        palettes: palettes(),
        relief: Relief {
            sea_level_mm: -2000,
            hill_mm: 0,
            mountain_mm: 0,
            erosion: 0,
            rivers: 0,
            forest: 0,
            city_flatten_slope: 1000,
        },
        proximity: Vec::new(),
        districts: regions
            .iter()
            .map(|r| District {
                id: format!("bairro:{}:{}", r[0], r[1]),
                kind: "residencial".into(),
                regions: vec![*r],
                style: None,
            })
            .collect(),
        publish_policy: "review".into(),
    };
    Ok(LegacyConversion {
        charter,
        programs,
        report,
    })
}

/// Whether a local tile is covered by a street under the width convention.
fn on_street(st: &Street, x: i32, y: i32) -> bool {
    let lo = -((st.width - 1) >> 1);
    let hi = lo + st.width - 1;
    st.path.windows(2).any(|w| {
        let (a, b) = (w[0], w[1]);
        if a[1] == b[1] {
            let (x0, x1) = (a[0].min(b[0]), a[0].max(b[0]));
            x >= x0 && x <= x1 && y >= a[1] + lo && y <= a[1] + hi
        } else if a[0] == b[0] {
            let (y0, y1) = (a[1].min(b[1]), a[1].max(b[1]));
            y >= y0 && y <= y1 && x >= a[0] + lo && x <= a[0] + hi
        } else {
            false
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn bundle() -> Value {
        let mut tiles = Vec::new();
        for x in 0..10 {
            for y in 0..3 {
                tiles.push(json!({"xy": [x, 10 + y], "frame": "ground/road"}));
            }
        }
        tiles.push(json!({"xy": [3, 20], "frame": "ground/water-0"}));
        json!({
            "schema": LEGACY_BUNDLE_SCHEMA,
            "city": {"id": "c", "name": "C", "seed": "s"},
            "plots": [
                {"id": "plot:a", "rect": [1, 2, 6, 6], "door": [3, 8], "kind": "residential", "region": "c/r:0:0"},
                {"id": "plot:b", "rect": [-8, 2, 6, 6], "door": [-5, 8], "kind": "farm", "region": "c/r:-1:0", "style_variant": 2}
            ],
            "tiles": tiles,
            "objects": [
                {"id": "t1", "asset": "slice/object/tree", "xy": [8, 3]},
                {"id": "h", "asset": "slice/object/house", "xy": [2, 3], "kind": "house", "plot_id": "plot:a"},
                {"id": "h2", "asset": "slice/object/house", "xy": [-7, 3], "kind": "house", "plot_id": "plot:b"}
            ],
            "farm": [{"id": "bed:1", "plot_id": "plot:b", "xy": [-6, 4], "crop": "carrot", "stage": "ripe"}],
            "homes": [{"plot_id": "plot:a", "home_id": "42"}]
        })
    }

    #[test]
    fn converts_without_moving_doors_or_owners() {
        let c = convert(&bundle()).unwrap();
        assert_eq!(c.programs.len(), 2);
        let r0 = c.programs.iter().find(|p| p.region == [0, 0]).unwrap();
        let lot = &r0.lots.explicit[0];
        assert_eq!(lot.rect, [1, 2, 6, 6]);
        assert_eq!(lot.door, Some([3, 8]));
        assert_eq!(lot.front, Dir::S);
        assert_eq!(lot.owner_ref.as_deref(), Some("home:42"));
        assert_eq!(r0.streets.len(), 1);
        assert_eq!(r0.streets[0].width, 3);
        assert_eq!(r0.streets[0].path, vec![[0, 11], [9, 11]]);
        let rm = c.programs.iter().find(|p| p.region == [-1, 0]).unwrap();
        assert_eq!(rm.lots.explicit[0].rect, [56, 2, 6, 6]);
        let bed = rm.props.iter().find(|p| p.kind == "canteiro").unwrap();
        assert_eq!(bed.cells, vec![[58, 4]]);
        assert_eq!(c.report.canteiros, 1);
        assert_eq!(c.report.owners.len(), 1);
        // Deterministic.
        let again = convert(&bundle()).unwrap();
        assert_eq!(
            crate::hash::canonical_hash(&c.programs),
            crate::hash::canonical_hash(&again.programs)
        );
    }
}
