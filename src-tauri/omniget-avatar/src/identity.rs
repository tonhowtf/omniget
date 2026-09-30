//! Who a resident is: name, short bio, Big Five, profession and home. The
//! deterministic generator here needs no LLM; the LLM version (agent G2) asks
//! a model for the same struct using [`identity_prompt_schema`] and converts
//! its `appearance` with [`crate::traits::from_traits`].

use crate::catalog::Catalog;
use crate::genome::{pheno, Genome};
use crate::rng::{seed_of, Rng};
use crate::traits::{BUILDS, COLOR_WORDS, PRESENTATIONS, TRAIT_TYPES};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub name: String,
    pub bio: String,
    /// Openness, conscientiousness, extraversion, agreeableness, neuroticism.
    pub big_five: [u8; 5],
    pub profession: String,
    /// Home reference. The deterministic generator emits `home-NNN`; the city
    /// layer maps it to a building (or replaces it with a building id).
    pub home: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resident {
    pub npc_id: u32,
    pub identity: Identity,
    pub genome: Genome,
}

const FIRST_F: &[&str] = &[
    "Ana", "Beatriz", "Clara", "Dandara", "Elisa", "Fernanda", "Gabriela", "Helena", "Iara",
    "Joana", "Karina", "Luana", "Marina", "Nina", "Olívia", "Paula", "Raquel", "Sofia", "Tainá",
    "Vitória", "Yara", "Zoe", "Amara", "Mei", "Leila", "Ingrid", "Aiko", "Nadia", "Rosa", "Lúcia",
];
const FIRST_M: &[&str] = &[
    "André", "Bruno", "Caio", "Davi", "Emanuel", "Felipe", "Gustavo", "Heitor", "Igor", "João",
    "Kauã", "Lucas", "Mateus", "Nícolas", "Otávio", "Pedro", "Rafael", "Samuel", "Tiago",
    "Vicente", "Wagner", "Yuri", "Kofi", "Hiro", "Omar", "Anders", "Tomás", "Raj", "Benedito",
    "Luís",
];
const FIRST_N: &[&str] = &[
    "Alex", "Ariel", "Cris", "Dani", "Jordan", "Kim", "Noa", "Rene", "Sam", "Val",
];
const SURNAMES: &[&str] = &[
    "Silva",
    "Santos",
    "Oliveira",
    "Souza",
    "Lima",
    "Pereira",
    "Costa",
    "Rodrigues",
    "Almeida",
    "Nascimento",
    "Araújo",
    "Ferreira",
    "Carvalho",
    "Gomes",
    "Martins",
    "Rocha",
    "Ribeiro",
    "Barbosa",
    "Moreira",
    "Cardoso",
    "Teixeira",
    "Mendes",
    "Nunes",
    "Campos",
    "Tanaka",
    "Okafor",
    "Novak",
    "Haddad",
    "Kowalski",
    "Lindqvist",
    "Mendoza",
    "Chen",
    "Duarte",
    "Freitas",
    "Pinto",
    "Vieira",
];

/// Professions, with the appearance they suggest (trait overrides).
const PROFESSIONS: &[(&str, &[(&str, &str)])] = &[
    (
        "farmer",
        &[
            ("top", "overalls"),
            ("shoes", "work-boots"),
            ("accessory", "straw-hat"),
        ],
    ),
    (
        "gardener",
        &[
            ("top", "t-shirt"),
            ("bottom", "bermuda"),
            ("shoes", "boots"),
        ],
    ),
    (
        "baker",
        &[
            ("top", "apron-shirt"),
            ("bottom", "pants"),
            ("color_primary", "white"),
        ],
    ),
    (
        "builder",
        &[
            ("top", "jacket"),
            ("bottom", "jeans"),
            ("shoes", "work-boots"),
            ("accessory", "cap"),
        ],
    ),
    (
        "carpenter",
        &[
            ("top", "shirt"),
            ("bottom", "jeans"),
            ("shoes", "work-boots"),
        ],
    ),
    ("teacher", &[("top", "sweater"), ("accessory", "glasses")]),
    (
        "fisher",
        &[
            ("top", "long-sleeve"),
            ("shoes", "boots"),
            ("accessory", "beanie"),
        ],
    ),
    (
        "merchant",
        &[("top", "shirt"), ("bottom", "pants"), ("shoes", "shoes")],
    ),
    ("cook", &[("top", "apron-shirt"), ("shoes", "shoes")]),
    ("tailor", &[("top", "shirt"), ("accessory", "glasses")]),
    ("smith", &[("top", "apron-shirt"), ("shoes", "work-boots")]),
    ("librarian", &[("top", "sweater"), ("accessory", "glasses")]),
    ("doctor", &[("top", "coat"), ("color_primary", "white")]),
    ("musician", &[("top", "jacket"), ("accessory", "scarf")]),
    ("painter", &[("top", "shirt"), ("accessory", "headband")]),
    (
        "courier",
        &[
            ("top", "t-shirt"),
            ("bottom", "shorts"),
            ("shoes", "sneakers"),
            ("accessory", "backpack"),
        ],
    ),
    ("barista", &[("top", "apron-shirt"), ("shoes", "sneakers")]),
    (
        "beekeeper",
        &[("top", "long-sleeve"), ("accessory", "straw-hat")],
    ),
];

const HOBBIES: &[&str] = &[
    "keeps a small herb garden",
    "collects old maps",
    "plays the guitar at dusk",
    "bakes bread on Sundays",
    "knows every bird by its song",
    "fixes neighbors' bicycles",
    "writes letters nobody reads",
    "paints the harbor in watercolor",
    "runs along the river at dawn",
    "tells long stories at the market",
    "keeps bees on the roof",
    "carves little wooden animals",
    "reads under the big tree",
    "teaches children to swim",
    "trades seeds with every newcomer",
    "never misses a festival",
];

const TRAIT_WORDS: [(&str, &str); 5] = [
    ("curious", "set in their ways"),
    ("meticulous", "easygoing"),
    ("outgoing", "quiet"),
    ("warm", "blunt"),
    ("anxious", "calm"),
];

pub fn npc_identities(city_id: &str, count: usize, seed: u64) -> Vec<Resident> {
    let cat = crate::catalog::catalog();
    let base = seed_of(city_id) ^ seed.rotate_left(17);
    let mut home_rng = Rng::new(base ^ 0x686f_6d65);
    let mut home = 0u32;
    let mut left_in_home = 0u32;
    (0..count)
        .map(|i| {
            if left_in_home == 0 {
                home += 1;
                left_in_home = 1 + home_rng.below(4);
            }
            left_in_home -= 1;
            resident(cat, base, i as u32, format!("home-{home:03}"))
        })
        .collect()
}

fn resident(cat: &Catalog, base: u64, i: u32, home: String) -> Resident {
    let s = base ^ (i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    let mut r = Rng::new(s);
    let mut genome = Genome::random(s);
    let gender = genome.phenotype(pheno::GENDER);
    let age = genome.phenotype(pheno::AGE);
    let first = if gender > 0.6 {
        *r.pick(FIRST_F)
    } else if gender < 0.4 {
        *r.pick(FIRST_M)
    } else {
        *r.pick(FIRST_N)
    };
    let name = format!("{first} {}", r.pick(SURNAMES));
    let big_five = [0; 5].map(|_: u8| (r.bell(0.5, 0.2) * 255.0).round() as u8);
    let years = crate::genome::years_for_age(age).round() as u32;
    let (profession, overrides) = if years < 16 {
        ("student", &[][..])
    } else if years > 70 && r.below(2) == 0 {
        ("retired", &[][..])
    } else {
        let p = r.pick(PROFESSIONS);
        (p.0, p.1)
    };
    // dress the part: apply the profession's traits on top of the random genome
    if !overrides.is_empty() {
        let attrs: Vec<Value> = overrides
            .iter()
            .map(|(t, v)| json!({"trait_type": t, "value": v}))
            .collect();
        let mut traits = json!({ "attributes": attrs });
        // keep the body: pin phenotypes by re-deriving from the same seed
        if let Ok(g) = crate::traits::from_traits_with(cat, &traits, s) {
            genome = g;
        } else {
            if let Some(a) = traits["attributes"].as_array_mut() {
                a.retain(|x| x["trait_type"] != "bottom");
            }
            if let Ok(g) = crate::traits::from_traits_with(cat, &traits, s) {
                genome = g;
            }
        }
    }
    let adj: Vec<&str> = big_five
        .iter()
        .zip(TRAIT_WORDS.iter())
        .filter_map(|(v, (hi, lo))| match v {
            0..=70 => Some(*lo),
            185..=255 => Some(*hi),
            _ => None,
        })
        .take(2)
        .collect();
    let temper = match adj.as_slice() {
        [] => "even-tempered".to_string(),
        [a] => a.to_string(),
        [a, b, ..] => format!("{a} and {b}"),
    };
    let first_name = name.split(' ').next().unwrap_or(&name);
    let bio = format!(
        "{first_name} is a {temper} {profession}, {years} years old, who {}.",
        r.pick(HOBBIES)
    );
    Resident {
        npc_id: i + 1,
        identity: Identity {
            name,
            bio,
            big_five,
            profession: profession.to_string(),
            home,
        },
        genome,
    }
}

/// JSON Schema of one resident as an LLM should write it: the identity plus
/// the appearance in the closed catalog vocabulary. Feed `appearance` to
/// [`crate::traits::from_traits`].
pub fn identity_prompt_schema() -> Value {
    let cat = crate::catalog::catalog();
    let ids = |v: Vec<&str>| Value::from(v.into_iter().map(String::from).collect::<Vec<_>>());
    let strs = |v: &[String]| Value::from(v.to_vec());
    let face: Vec<String> = cat.face_targets.iter().map(|t| t.id.clone()).collect();
    let mut words: Vec<String> = cat.words.keys().cloned().collect();
    words.sort();
    let mut colors: Vec<String> = COLOR_WORDS.iter().map(|s| s.to_string()).collect();
    colors.extend(cat.cloth_colors.iter().map(|h| format!("#{h}")));
    let enum_attr = |t: &str, values: Value, desc: &str| {
        json!({
            "type": "object", "additionalProperties": false, "required": ["trait_type", "value"],
            "description": desc,
            "properties": { "trait_type": { "const": t }, "value": { "enum": values } }
        })
    };
    let num_attr = |t: &str, min: f64, max: f64, desc: &str| {
        json!({
            "type": "object", "additionalProperties": false, "required": ["trait_type", "value"],
            "description": desc,
            "properties": { "trait_type": { "const": t }, "value": { "type": "number", "minimum": min, "maximum": max } }
        })
    };
    let attrs = vec![
        enum_attr(
            "presentation",
            ids(PRESENTATIONS.to_vec()),
            "How the body reads",
        ),
        num_attr("age_years", 0.0, 120.0, "Age in years"),
        num_attr("height_cm", 40.0, 240.0, "Standing height"),
        enum_attr("build", ids(BUILDS.to_vec()), "Body build"),
        enum_attr(
            "skin",
            ids(Catalog::named_ids(&cat.skin_tones)),
            "Skin tone, light to dark",
        ),
        enum_attr(
            "hair_style",
            ids(Catalog::named_ids(&cat.hair_styles)),
            "Hair style",
        ),
        enum_attr(
            "hair_color",
            ids(Catalog::named_ids(&cat.hair_colors)),
            "Hair color",
        ),
        enum_attr(
            "eye_color",
            ids(Catalog::named_ids(&cat.eye_colors)),
            "Eye color",
        ),
        enum_attr("brows", strs(&cat.brows), "Eyebrows"),
        enum_attr("beard", strs(&cat.beards), "Facial hair"),
        enum_attr(
            "top",
            ids(Catalog::piece_ids(&cat.tops)),
            "Upper garment (dress, overalls, robe cover the legs)",
        ),
        enum_attr(
            "bottom",
            ids(Catalog::piece_ids(&cat.bottoms)),
            "Lower garment; omit with a full-body top",
        ),
        enum_attr("shoes", ids(Catalog::piece_ids(&cat.shoes)), "Footwear"),
        enum_attr(
            "accessory",
            ids(Catalog::piece_ids(&cat.accessories)),
            "One accessory",
        ),
        enum_attr(
            "palette",
            ids(cat.palettes.iter().map(|p| p.id.as_str()).collect()),
            "Clothing color scheme",
        ),
        enum_attr(
            "color_primary",
            Value::from(colors.clone()),
            "Main clothing color",
        ),
        enum_attr(
            "color_secondary",
            Value::from(colors.clone()),
            "Second clothing color",
        ),
        enum_attr("color_accent", Value::from(colors), "Accent color"),
        json!({
            "type": "object", "additionalProperties": false, "required": ["trait_type", "value"],
            "description": "A face feature, 'id' or 'id:amount' with amount from -1 to 1 (at most 4)",
            "properties": { "trait_type": { "const": "face" },
                "value": { "type": "string", "pattern": format!("^({})(:-?(0(\\.\\d+)?|1(\\.0+)?))?$", face.join("|")) } }
        }),
        enum_attr("gait", strs(&cat.gaits), "Way of walking"),
        enum_attr("idle", strs(&cat.idles), "What they do when standing"),
        enum_attr("mood", strs(&cat.moods), "Base mood"),
    ];
    debug_assert_eq!(attrs.len() + 1, TRAIT_TYPES.len());
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "OmniGet city resident",
        "type": "object",
        "additionalProperties": false,
        "required": ["identity", "appearance"],
        "properties": {
            "identity": {
                "type": "object", "additionalProperties": false,
                "required": ["name", "bio", "big_five", "profession", "home"],
                "properties": {
                    "name": { "type": "string", "minLength": 1, "maxLength": 60 },
                    "bio": { "type": "string", "maxLength": 400, "description": "Two sentences at most, in the city's culture" },
                    "big_five": { "type": "array", "minItems": 5, "maxItems": 5,
                        "description": "openness, conscientiousness, extraversion, agreeableness, neuroticism",
                        "items": { "type": "integer", "minimum": 0, "maximum": 255 } },
                    "profession": { "type": "string", "maxLength": 40 },
                    "home": { "type": "string", "maxLength": 60, "description": "Building id or home reference" }
                }
            },
            "appearance": {
                "type": "object", "additionalProperties": false,
                "required": ["attributes"],
                "properties": {
                    "attributes": { "type": "array", "maxItems": 30, "items": { "oneOf": attrs } },
                    "phenotypes": { "type": "array", "maxItems": 8,
                        "description": "Body words: age, height, build, presentation",
                        "items": { "enum": words } }
                }
            }
        },
        "x-catalog-hash": crate::catalog::catalog_hash(),
    })
}
