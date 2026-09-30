//! Charter validation and example charters. Owner: agent A (terrain).
//!
//! Violation codes (all `P*`, structural, before anything is generated):
//! - `PC1` schema is not `omni.city.charter/v1`
//! - `PC2` empty seed, city id or name
//! - `PC3` relief value out of range
//! - `PC4` empty or unknown style
//! - `PC5` empty palette key, or a palette that is neither built in nor declared
//! - `PC6` declared palette with a missing slot or a colour that is not `#rrggbb`
//! - `PC7` invalid publish policy (`auto` | `review`)
//! - `PC8` inconsistent proximity rule
//! - `PC9` district without id, duplicated, or without regions
//! - `PC10` generator version this build does not run (warning)

use crate::types::*;
use std::collections::{BTreeMap, BTreeSet};

pub const CHARTER_SCHEMA: &str = "omni.city.charter/v1";

/// Site kinds a proximity rule may name (see `sites`).
pub const SITE_KINDS: &[&str] = &["town", "village", "farm", "landmark"];
/// Terrain features a proximity rule may be near.
pub const TERRAIN_FEATURES: &[&str] = &["water", "river", "lake", "sea", "coast"];
/// District kinds (the zone kinds of the city program).
pub const DISTRICT_KINDS: &[&str] = &["civico", "residencial", "comercial", "rural", "parque"];

/// Relief limits, inclusive.
pub const SEA_LEVEL_RANGE: (i32, i32) = (-50_000, 50_000);
pub const HILL_RANGE: (i32, i32) = (0, 80_000);
pub const MOUNTAIN_RANGE: (i32, i32) = (0, 250_000);

fn v(code: &str, id: &str, measure: String, message: String, severity: Severity) -> Violation {
    Violation {
        code: code.into(),
        id: id.into(),
        measure,
        message,
        severity,
    }
}

fn is_hex_colour(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit())
}

/// Structural checks of a charter (P-codes). Empty = valid.
pub fn validate(c: &Charter) -> Vec<Violation> {
    use Severity::*;
    let mut out = Vec::new();
    if c.schema != CHARTER_SCHEMA {
        out.push(v(
            "PC1",
            "schema",
            c.schema.clone(),
            format!("schema must be \"{}\"", CHARTER_SCHEMA),
            Error,
        ));
    }
    for (id, val) in [
        ("seed", &c.seed),
        ("city_id", &c.city_id),
        ("name", &c.name),
    ] {
        if val.trim().is_empty() {
            out.push(v(
                "PC2",
                id,
                String::new(),
                format!("{} must not be empty", id),
                Error,
            ));
        }
    }

    let r = &c.relief;
    let mut range = |id: &str, val: i64, lo: i64, hi: i64| {
        if val < lo || val > hi {
            out.push(v(
                "PC3",
                &format!("relief.{}", id),
                val.to_string(),
                format!("relief.{} must be between {} and {}", id, lo, hi),
                Error,
            ));
        }
    };
    range(
        "sea_level_mm",
        r.sea_level_mm as i64,
        SEA_LEVEL_RANGE.0 as i64,
        SEA_LEVEL_RANGE.1 as i64,
    );
    range(
        "hill_mm",
        r.hill_mm as i64,
        HILL_RANGE.0 as i64,
        HILL_RANGE.1 as i64,
    );
    range(
        "mountain_mm",
        r.mountain_mm as i64,
        MOUNTAIN_RANGE.0 as i64,
        MOUNTAIN_RANGE.1 as i64,
    );
    range("erosion", r.erosion as i64, 0, 1000);
    range("rivers", r.rivers as i64, 0, 1000);
    range("forest", r.forest as i64, 0, 1000);
    range("city_flatten_slope", r.city_flatten_slope as i64, 1, 1000);

    if c.style.trim().is_empty() {
        out.push(v(
            "PC4",
            "style",
            String::new(),
            "style must name a kit style".into(),
            Error,
        ));
    }
    for d in &c.districts {
        if let Some(s) = &d.style {
            if s.trim().is_empty() {
                out.push(v(
                    "PC4",
                    &d.id,
                    String::new(),
                    "district style is empty; omit it to use the city style".into(),
                    Error,
                ));
            }
        }
    }

    // Palettes: declared ones must be complete; the chosen one must exist.
    let builtin = builtin_palettes();
    let mut keys: BTreeSet<String> = builtin.iter().map(|p| p.key.clone()).collect();
    let mut seen = BTreeSet::new();
    for p in &c.palettes {
        if p.key.trim().is_empty() {
            out.push(v(
                "PC6",
                "palettes",
                String::new(),
                "palette without key".into(),
                Error,
            ));
            continue;
        }
        if !seen.insert(p.key.clone()) {
            out.push(v(
                "PC6",
                &p.key,
                String::new(),
                "palette declared twice".into(),
                Error,
            ));
        }
        keys.insert(p.key.clone());
        for slot in PALETTE_SLOTS {
            match p.slots.get(*slot) {
                None => out.push(v(
                    "PC6",
                    &p.key,
                    slot.to_string(),
                    format!("palette {} misses slot {}", p.key, slot),
                    Error,
                )),
                Some(col) if !is_hex_colour(col) => out.push(v(
                    "PC6",
                    &p.key,
                    format!("{}={}", slot, col),
                    format!("palette {} slot {} must be #rrggbb", p.key, slot),
                    Error,
                )),
                _ => {}
            }
        }
        for slot in p.slots.keys() {
            if !PALETTE_SLOTS.contains(&slot.as_str()) {
                out.push(v(
                    "PC6",
                    &p.key,
                    slot.clone(),
                    format!("palette {} has unknown slot {}", p.key, slot),
                    Warning,
                ));
            }
        }
    }
    if c.palette.trim().is_empty() {
        out.push(v(
            "PC5",
            "palette",
            String::new(),
            "palette must name a palette".into(),
            Error,
        ));
    } else if !keys.contains(&c.palette) {
        out.push(v(
            "PC5",
            "palette",
            c.palette.clone(),
            format!(
                "palette {} is neither built in nor declared in palettes",
                c.palette
            ),
            Error,
        ));
    }

    if c.publish_policy != "auto" && c.publish_policy != "review" {
        out.push(v(
            "PC7",
            "publish_policy",
            c.publish_policy.clone(),
            "publish_policy must be auto or review".into(),
            Error,
        ));
    }

    // Proximity.
    let mut bounds: BTreeMap<(String, String), (i32, i32)> = BTreeMap::new();
    for (k, rule) in c.proximity.iter().enumerate() {
        let id = format!("proximity[{}]", k);
        if !SITE_KINDS.contains(&rule.kind.as_str()) {
            out.push(v(
                "PC8",
                &id,
                rule.kind.clone(),
                format!("kind must be one of {}", SITE_KINDS.join(", ")),
                Error,
            ));
        }
        let near = match &rule.near {
            None => {
                out.push(v(
                    "PC8",
                    &id,
                    String::new(),
                    "near is required".into(),
                    Error,
                ));
                continue;
            }
            Some(n) => n.clone(),
        };
        if !SITE_KINDS.contains(&near.as_str()) && !TERRAIN_FEATURES.contains(&near.as_str()) {
            out.push(v(
                "PC8",
                &id,
                near.clone(),
                format!(
                    "near must be a site kind or one of {}",
                    TERRAIN_FEATURES.join(", ")
                ),
                Error,
            ));
        }
        if rule.min_tiles.is_none() && rule.max_tiles.is_none() {
            out.push(v(
                "PC8",
                &id,
                String::new(),
                "give min_tiles, max_tiles or both".into(),
                Error,
            ));
            continue;
        }
        let lo = rule.min_tiles.unwrap_or(0);
        let hi = rule.max_tiles.unwrap_or(i32::MAX);
        if lo < 0 || hi < 0 {
            out.push(v(
                "PC8",
                &id,
                format!("{}..{}", lo, hi),
                "distances must be positive".into(),
                Error,
            ));
        }
        if hi != i32::MAX && hi > 4096 {
            out.push(v(
                "PC8",
                &id,
                hi.to_string(),
                "max_tiles is at most 4096".into(),
                Error,
            ));
        }
        if lo > hi {
            out.push(v(
                "PC8",
                &id,
                format!("{}>{}", lo, hi),
                "min_tiles is greater than max_tiles".into(),
                Error,
            ));
        }
        if near == rule.kind && rule.max_tiles.is_some() && rule.min_tiles.is_none() {
            out.push(v(
                "PC8",
                &id,
                near.clone(),
                "a site is always near itself; use min_tiles to space sites of one kind".into(),
                Warning,
            ));
        }
        let e = bounds
            .entry((rule.kind.clone(), near))
            .or_insert((0, i32::MAX));
        e.0 = e.0.max(lo);
        e.1 = e.1.min(hi);
        if e.0 > e.1 {
            out.push(v(
                "PC8",
                &id,
                format!("{}>{}", e.0, e.1),
                format!(
                    "rules for {} contradict each other (min above max)",
                    rule.kind
                ),
                Error,
            ));
        }
    }

    let mut ids = BTreeSet::new();
    for (k, d) in c.districts.iter().enumerate() {
        let id = if d.id.is_empty() {
            format!("districts[{}]", k)
        } else {
            d.id.clone()
        };
        if d.id.trim().is_empty() || !ids.insert(d.id.clone()) {
            out.push(v(
                "PC9",
                &id,
                String::new(),
                "district id empty or duplicated".into(),
                Error,
            ));
        }
        if d.regions.is_empty() {
            out.push(v(
                "PC9",
                &id,
                String::new(),
                "district covers no region".into(),
                Error,
            ));
        }
        if !DISTRICT_KINDS.contains(&d.kind.as_str()) {
            out.push(v(
                "PC9",
                &id,
                d.kind.clone(),
                format!("district kind must be one of {}", DISTRICT_KINDS.join(", ")),
                Error,
            ));
        }
    }

    if c.generator_version != GENERATOR_VERSION {
        out.push(v(
            "PC10",
            "generator_version",
            c.generator_version.clone(),
            format!(
                "this build runs {}; the land will differ",
                GENERATOR_VERSION
            ),
            Warning,
        ));
    }
    out
}

fn palette(key: &str, cols: [&str; 16]) -> Palette {
    let slots = PALETTE_SLOTS
        .iter()
        .zip(cols.iter())
        .map(|(s, c)| (s.to_string(), c.to_string()))
        .collect();
    Palette {
        key: key.into(),
        slots,
    }
}

/// Built-in palettes over the style bible's base colours (creme #F1E7D0,
/// carvão #263C37, terracota #B9654C, verde folha #7C9B71, verde escuro
/// #436D59, água #679FA4, ocre #CBA568). Slot order is [`PALETTE_SLOTS`]:
/// wall, roof, trim, wood, stone, glass, door, ground, grass, water, street,
/// plaza, leaf, trunk, metal, cloth.
pub fn builtin_palettes() -> Vec<Palette> {
    vec![
        palette(
            "cal-e-terracota",
            [
                "#F1E7D0", "#B9654C", "#E4D6B6", "#7A5236", "#B8AE9C", "#9CC3C6", "#436D59",
                "#CBA568", "#7C9B71", "#679FA4", "#8C8577", "#D9C9A8", "#7C9B71", "#6B4A33",
                "#263C37", "#B9654C",
            ],
        ),
        palette(
            "azul-anil",
            [
                "#F4F1E8", "#B9654C", "#2F4F8F", "#6E4B32", "#A9A396", "#8FB8C9", "#2F4F8F",
                "#CBA568", "#7C9B71", "#5E97A6", "#857F74", "#D6CFBF", "#6F9468", "#5E4330",
                "#263C37", "#3C5DA0",
            ],
        ),
        palette(
            "vila-praiana",
            [
                "#F6EEDC", "#D08A5C", "#F9F6EF", "#A87B51", "#D9CDB4", "#A8D4D6", "#4E8C8F",
                "#E3D2A6", "#8FAE78", "#5FB0B5", "#C9BA98", "#EADFC4", "#6FA173", "#7D5A3C",
                "#3F5A58", "#E0785A",
            ],
        ),
        palette(
            "musgo",
            [
                "#D9D4C0", "#436D59", "#263C37", "#5A4632", "#8E9484", "#8AA9A2", "#7C9B71",
                "#7E7458", "#5F8358", "#5B8C8A", "#6E6C60", "#A8A48E", "#436D59", "#4A3A2A",
                "#263C37", "#CBA568",
            ],
        ),
        palette(
            "adobe-ocre",
            [
                "#E6CDA3", "#B9654C", "#F1E7D0", "#8A6440", "#C9A77C", "#9CB9B6", "#436D59",
                "#D8B98A", "#A6A56E", "#679FA4", "#BFA27A", "#E3CFA8", "#8C9A5E", "#6B4A33",
                "#263C37", "#CBA568",
            ],
        ),
    ]
}

fn rule(kind: &str, near: &str, min: Option<i32>, max: Option<i32>) -> ProximityRule {
    ProximityRule {
        kind: kind.into(),
        near: Some(near.into()),
        min_tiles: min,
        max_tiles: max,
    }
}

fn district(id: &str, kind: &str, regions: &[[i32; 2]], style: Option<&str>) -> District {
    District {
        id: id.into(),
        kind: kind.into(),
        regions: regions.to_vec(),
        style: style.map(|s| s.into()),
    }
}

/// A ready charter for tests and for founding without an LLM: a temperate
/// river town in whitewash and terracotta.
pub fn example(city_id: &str, seed: &str) -> Charter {
    Charter {
        schema: CHARTER_SCHEMA.into(),
        city_id: city_id.into(),
        name: "Vila Omni".into(),
        seed: seed.into(),
        generator_version: GENERATOR_VERSION.into(),
        story: "A market town that grew where the old road meets the river: whitewashed houses, \
                terracotta roofs, a church square and orchards up the hill."
            .into(),
        climate: Climate::Temperate,
        culture:
            "Portuguese colonial small town: church square, market days, porches facing the street"
                .into(),
        style: "vila-colonial".into(),
        palette: "cal-e-terracota".into(),
        palettes: builtin_palettes(),
        relief: Relief {
            sea_level_mm: 0,
            hill_mm: 12_000,
            mountain_mm: 40_000,
            erosion: 500,
            rivers: 400,
            forest: 450,
            city_flatten_slope: 80,
        },
        proximity: vec![
            rule("town", "river", None, Some(160)),
            rule("town", "town", Some(400), None),
            rule("village", "town", Some(160), Some(900)),
            rule("farm", "water", None, Some(240)),
        ],
        districts: vec![
            district("praca-da-matriz", "civico", &[[0, 0]], None),
            district(
                "beira-rio",
                "residencial",
                &[[1, 0], [1, 1]],
                Some("vila-praiana"),
            ),
            district("rua-do-comercio", "comercial", &[[0, 1]], None),
            district("pomar", "rural", &[[-1, 0], [-1, 1]], None),
        ],
        publish_policy: "auto".into(),
    }
}

/// A tropical coastal fishing village: high sea level so the coast is near,
/// lush forest, pastel houses.
pub fn example_tropical_coastal(city_id: &str, seed: &str) -> Charter {
    Charter {
        schema: CHARTER_SCHEMA.into(),
        city_id: city_id.into(),
        name: "Porto Anil".into(),
        seed: seed.into(),
        generator_version: GENERATOR_VERSION.into(),
        story: "A fishing village between the beach and the forest: boats on the sand, \
                a fish market by the pier, pastel houses with indigo shutters."
            .into(),
        climate: Climate::Tropical,
        culture: "Brazilian northeast coast: fish market, festa junina in the square, hammocks on porches".into(),
        style: "vila-praiana".into(),
        palette: "azul-anil".into(),
        palettes: builtin_palettes(),
        relief: Relief {
            sea_level_mm: 3_000,
            hill_mm: 9_000,
            mountain_mm: 15_000,
            erosion: 400,
            rivers: 500,
            forest: 750,
            city_flatten_slope: 60,
        },
        proximity: vec![
            rule("town", "coast", None, Some(120)),
            rule("village", "coast", None, Some(200)),
            rule("town", "town", Some(500), None),
            rule("farm", "sea", Some(60), None),
        ],
        districts: vec![
            district("mercado-do-peixe", "comercial", &[[0, 0]], None),
            district("orla", "residencial", &[[1, 0], [0, 1]], Some("vila-praiana")),
            district("capela", "civico", &[[1, 1]], None),
            district("coqueiral", "parque", &[[-1, 0]], None),
        ],
        publish_policy: "auto".into(),
    }
}

/// An arid highland town: few rivers, little forest, ochre adobe.
pub fn example_arid(city_id: &str, seed: &str) -> Charter {
    Charter {
        schema: CHARTER_SCHEMA.into(),
        city_id: city_id.into(),
        name: "Serra Ocre".into(),
        seed: seed.into(),
        generator_version: GENERATOR_VERSION.into(),
        story: "A dry market town around a well and a small reservoir, with thick ochre walls, \
                shaded arcades and goat pens on the slopes."
            .into(),
        climate: Climate::Arid,
        culture: "Sertão town: fair day by the church, leather and clay crafts, shade before noon"
            .into(),
        style: "vila-colonial".into(),
        palette: "adobe-ocre".into(),
        palettes: builtin_palettes(),
        relief: Relief {
            sea_level_mm: -20_000,
            hill_mm: 16_000,
            mountain_mm: 60_000,
            erosion: 700,
            rivers: 150,
            forest: 120,
            city_flatten_slope: 100,
        },
        proximity: vec![
            rule("town", "water", None, Some(320)),
            rule("town", "town", Some(600), None),
            rule("farm", "water", None, Some(160)),
        ],
        districts: vec![
            district("largo-da-feira", "civico", &[[0, 0]], None),
            district("casario", "residencial", &[[1, 0], [0, 1]], None),
            district("curral", "rural", &[[1, 1], [-1, 0]], Some("vila-colonial")),
        ],
        publish_policy: "review".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn examples_are_valid() {
        for c in [
            example("a", "s"),
            example_tropical_coastal("b", "s"),
            example_arid("c", "s"),
        ] {
            let v = validate(&c);
            assert!(v.is_empty(), "{:?}", v);
        }
    }

    #[test]
    fn catches_errors() {
        let mut c = example("a", "s");
        c.seed = " ".into();
        c.relief.erosion = 2000;
        c.publish_policy = "yes".into();
        c.palette = "nope".into();
        c.proximity.push(ProximityRule {
            kind: "town".into(),
            near: Some("river".into()),
            min_tiles: Some(300),
            max_tiles: None,
        });
        let codes: Vec<String> = validate(&c).into_iter().map(|v| v.code).collect();
        for k in ["PC2", "PC3", "PC5", "PC7", "PC8"] {
            assert!(codes.iter().any(|c| c == k), "{} missing in {:?}", k, codes);
        }
    }
}
