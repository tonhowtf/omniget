//! Appearance described as closed-vocabulary traits (what an LLM or the
//! creator emits) turned into a genome.
//!
//! Input: `{"attributes":[{"trait_type":"top","value":"shirt"}, ...],
//! "phenotypes":["idoso","alto"]}`. Every trait type and value must exist in
//! the catalog; unknown words are errors, never silently ignored. Whatever is
//! not described comes from `Genome::random(seed)`.

use crate::catalog::{nearest_color, Catalog};
use crate::genome::{self, pheno, q, uq, Genome, NO_FACE};
use serde_json::Value;

/// The trait types `from_traits` accepts, in schema order.
pub const TRAIT_TYPES: &[&str] = &[
    "presentation",
    "age_years",
    "height_cm",
    "build",
    "phenotype",
    "skin",
    "hair_style",
    "hair_color",
    "eye_color",
    "brows",
    "beard",
    "top",
    "bottom",
    "shoes",
    "accessory",
    "palette",
    "color_primary",
    "color_secondary",
    "color_accent",
    "face",
    "gait",
    "idle",
    "mood",
];

pub const PRESENTATIONS: &[&str] = &["feminine", "masculine", "neutral"];
pub const BUILDS: &[&str] = &[
    "thin", "slim", "average", "athletic", "strong", "stocky", "heavy",
];

pub fn from_traits(json: &Value, seed: u64) -> Result<Genome, Vec<String>> {
    from_traits_with(crate::catalog::catalog(), json, seed)
}

pub fn from_traits_with(cat: &Catalog, json: &Value, seed: u64) -> Result<Genome, Vec<String>> {
    let mut g = Genome::random(seed);
    let mut errors = Vec::new();
    let mut pending_height: Option<f32> = None;
    let mut explicit_bottom = false;
    let mut face_slot = 0usize;

    if let Some(words) = json.get("phenotypes") {
        match words.as_array() {
            Some(list) => {
                for w in list {
                    match w.as_str().and_then(|s| cat.word(s)) {
                        Some(targets) => apply_pheno(&mut g, &targets),
                        None => errors.push(format!("unknown phenotype word {w}")),
                    }
                }
            }
            None => errors.push("phenotypes must be a list of words".into()),
        }
    }

    let attrs = match json.get("attributes") {
        None => Vec::new(),
        Some(Value::Array(a)) => a.clone(),
        Some(_) => {
            errors.push("attributes must be a list".into());
            Vec::new()
        }
    };
    for a in &attrs {
        let tt = a.get("trait_type").and_then(Value::as_str).unwrap_or("");
        let val = a.get("value").cloned().unwrap_or(Value::Null);
        let s = val.as_str().map(|s| s.trim().to_string());
        let sv = s.as_deref().unwrap_or("");
        let bad = |errors: &mut Vec<String>, what: &str| {
            errors.push(format!("{tt}: unknown value {val} ({what})"));
        };
        match tt {
            "presentation" => match sv {
                "feminine" => g.phenotypes[pheno::GENDER] = q(0.95),
                "masculine" => g.phenotypes[pheno::GENDER] = q(0.05),
                "neutral" => g.phenotypes[pheno::GENDER] = q(0.5),
                _ => bad(&mut errors, "feminine, masculine or neutral"),
            },
            "age_years" => match val.as_f64() {
                Some(y) if (0.0..=120.0).contains(&y) => {
                    g.phenotypes[pheno::AGE] = q(genome::age_for_years(y as f32))
                }
                _ => bad(&mut errors, "a number of years from 0 to 120"),
            },
            "height_cm" => match val.as_f64() {
                Some(cm) if (40.0..=240.0).contains(&cm) => {
                    pending_height = Some(cm as f32 / 100.0)
                }
                _ => bad(&mut errors, "centimeters from 40 to 240"),
            },
            "build" => {
                let (m, w) = match sv {
                    "thin" => (0.3, 0.05),
                    "slim" => (0.4, 0.2),
                    "average" => (0.5, 0.45),
                    "athletic" => (0.8, 0.35),
                    "strong" => (0.95, 0.6),
                    "stocky" => (0.7, 0.75),
                    "heavy" => (0.5, 0.95),
                    _ => {
                        bad(&mut errors, "a build");
                        continue;
                    }
                };
                g.phenotypes[pheno::MUSCLE] = q(m);
                g.phenotypes[pheno::WEIGHT] = q(w);
            }
            "phenotype" => match cat.word(sv) {
                Some(t) => apply_pheno(&mut g, &t),
                None => bad(&mut errors, "a phenotype word of the catalog"),
            },
            "skin" => match Catalog::index_of(Catalog::named_ids(&cat.skin_tones), sv)
                .or_else(|| skin_word(cat, sv))
            {
                Some(i) => g.skin = i as u8,
                None => bad(&mut errors, "a skin tone"),
            },
            "hair_style" => set(
                &mut g.hair,
                Catalog::index_of(Catalog::named_ids(&cat.hair_styles), sv),
                &mut errors,
                tt,
                &val,
            ),
            "hair_color" => set(
                &mut g.hair_color,
                Catalog::index_of(Catalog::named_ids(&cat.hair_colors), sv),
                &mut errors,
                tt,
                &val,
            ),
            "eye_color" => set(
                &mut g.eyes,
                Catalog::index_of(Catalog::named_ids(&cat.eye_colors), sv),
                &mut errors,
                tt,
                &val,
            ),
            "brows" => set(
                &mut g.brows,
                Catalog::index_of(cat.brows.iter().map(String::as_str), sv),
                &mut errors,
                tt,
                &val,
            ),
            "beard" => set(
                &mut g.beard,
                Catalog::index_of(cat.beards.iter().map(String::as_str), sv),
                &mut errors,
                tt,
                &val,
            ),
            "top" => set(
                &mut g.top,
                Catalog::index_of(Catalog::piece_ids(&cat.tops), sv),
                &mut errors,
                tt,
                &val,
            ),
            "bottom" => {
                explicit_bottom = true;
                set(
                    &mut g.bottom,
                    Catalog::index_of(Catalog::piece_ids(&cat.bottoms), sv),
                    &mut errors,
                    tt,
                    &val,
                )
            }
            "shoes" => set(
                &mut g.shoes,
                Catalog::index_of(Catalog::piece_ids(&cat.shoes), sv),
                &mut errors,
                tt,
                &val,
            ),
            "accessory" => set(
                &mut g.accessory,
                Catalog::index_of(Catalog::piece_ids(&cat.accessories), sv),
                &mut errors,
                tt,
                &val,
            ),
            "palette" => match cat.palettes.iter().find(|p| p.id == sv) {
                Some(p) => {
                    let n = p.colors.len();
                    let s = g.seed as usize;
                    g.colors = [
                        p.colors[s % n],
                        p.colors[(s / 7 + 1) % n],
                        p.colors[(s / 49 + 2) % n],
                    ];
                }
                None => bad(&mut errors, "a palette"),
            },
            "color_primary" | "color_secondary" | "color_accent" => {
                let slot = match tt {
                    "color_primary" => 0,
                    "color_secondary" => 1,
                    _ => 2,
                };
                let idx = match &val {
                    Value::Number(n) => n
                        .as_u64()
                        .filter(|i| (*i as usize) < cat.cloth_colors.len())
                        .map(|i| i as usize),
                    Value::String(h) if is_hex(h) => {
                        nearest_color(cat.cloth_colors.iter().map(String::as_str), h)
                    }
                    Value::String(w) => color_word(w).and_then(|h| {
                        nearest_color(cat.cloth_colors.iter().map(String::as_str), h)
                    }),
                    _ => None,
                };
                match idx {
                    Some(i) => g.colors[slot] = i as u8,
                    None => bad(
                        &mut errors,
                        "a palette index, #rrggbb or a basic color name",
                    ),
                }
            }
            "face" => {
                // "nose-wide" or "nose-wide:-0.5"
                let (id, amount) = match sv.split_once(':') {
                    Some((a, b)) => (a.trim(), b.trim().parse::<f32>().ok()),
                    None => (sv, Some(0.6)),
                };
                match (cat.face_targets.iter().position(|t| t.id == id), amount) {
                    (Some(i), Some(v)) if face_slot < 4 && (-1.0..=1.0).contains(&v) => {
                        g.face[face_slot] = (i as u8, (v * 127.0).round() as i8);
                        face_slot += 1;
                    }
                    (Some(_), _) if face_slot >= 4 => {
                        errors.push("face: at most 4 local changes".into())
                    }
                    _ => bad(
                        &mut errors,
                        "a face target id, optionally ':value' from -1 to 1",
                    ),
                }
            }
            "gait" => set(
                &mut g.gait,
                Catalog::index_of(cat.gaits.iter().map(String::as_str), sv),
                &mut errors,
                tt,
                &val,
            ),
            "idle" => set(
                &mut g.idle,
                Catalog::index_of(cat.idles.iter().map(String::as_str), sv),
                &mut errors,
                tt,
                &val,
            ),
            "mood" => set(
                &mut g.mood,
                Catalog::index_of(cat.moods.iter().map(String::as_str), sv),
                &mut errors,
                tt,
                &val,
            ),
            other => errors.push(format!("unknown trait_type '{other}'")),
        }
    }
    if face_slot > 0 {
        for f in g.face.iter_mut().skip(face_slot) {
            *f = (NO_FACE, 0);
        }
    }
    if let Some(m) = pending_height {
        let h = genome::height_for(g.phenotype(pheno::GENDER), g.phenotype(pheno::AGE), m);
        g.phenotypes[pheno::HEIGHT] = q(h);
    }
    // Children do not grow beards whatever the text says.
    if uq(g.phenotypes[pheno::AGE]) < 0.5 {
        g.beard = 0;
    }
    let top_full = cat
        .tops
        .get(g.top as usize)
        .map(|t| t.full_body)
        .unwrap_or(false);
    if top_full && explicit_bottom && g.bottom != 0 {
        errors.push(format!(
            "top '{}' covers the legs; drop the bottom",
            cat.tops[g.top as usize].id
        ));
    }
    g.fix_restrictions(cat);
    errors.extend(g.validate(cat));
    if errors.is_empty() {
        Ok(g)
    } else {
        Err(errors)
    }
}

fn set(field: &mut u8, idx: Option<usize>, errors: &mut Vec<String>, tt: &str, val: &Value) {
    match idx {
        Some(i) => *field = i as u8,
        None => errors.push(format!("{tt}: unknown value {val}")),
    }
}

fn apply_pheno(g: &mut Genome, targets: &[(usize, f32)]) {
    for (i, v) in targets {
        g.phenotypes[*i] = q(*v);
    }
}

fn is_hex(s: &str) -> bool {
    let h = s.trim_start_matches('#');
    h.len() == 6 && h.chars().all(|c| c.is_ascii_hexdigit())
}

fn skin_word(cat: &Catalog, w: &str) -> Option<usize> {
    let n = cat.skin_tones.len();
    let f = match w {
        "very-light" | "muito clara" => 0.0,
        "light" | "clara" => 0.12,
        "medium-light" => 0.3,
        "medium" | "morena" | "parda" => 0.5,
        "medium-dark" => 0.68,
        "dark" | "escura" | "negra" | "preta" => 0.85,
        "very-dark" | "retinta" => 1.0,
        _ => return None,
    };
    Some(((n - 1) as f32 * f).round() as usize)
}

/// A few basic color names, so a model can say "blue" without knowing hex.
pub fn color_word(w: &str) -> Option<&'static str> {
    Some(match w.trim().to_lowercase().as_str() {
        "white" | "branco" | "branca" => "f4f1ea",
        "black" | "preto" | "preta" => "1e1d1c",
        "gray" | "grey" | "cinza" => "8f8a80",
        "navy" | "marinho" => "1f3a5f",
        "blue" | "azul" => "2980b9",
        "light-blue" | "azul-claro" => "8fb8e0",
        "teal" | "verde-azulado" => "2f8a7e",
        "green" | "verde" => "27ae60",
        "olive" | "oliva" => "4f7a3a",
        "yellow" | "amarelo" | "amarela" => "f1c40f",
        "mustard" | "mostarda" => "c6b05a",
        "orange" | "laranja" => "e67e22",
        "brown" | "marrom" => "6e5540",
        "beige" | "bege" => "d8c8a8",
        "red" | "vermelho" | "vermelha" => "c0392b",
        "pink" | "rosa" => "f0a8c8",
        "purple" | "roxo" | "roxa" => "8e44ad",
        "lilac" | "lilás" | "lilas" => "c8b8ec",
        _ => return None,
    })
}

pub const COLOR_WORDS: &[&str] = &[
    "white",
    "black",
    "gray",
    "navy",
    "blue",
    "light-blue",
    "teal",
    "green",
    "olive",
    "yellow",
    "mustard",
    "orange",
    "brown",
    "beige",
    "red",
    "pink",
    "purple",
    "lilac",
];
