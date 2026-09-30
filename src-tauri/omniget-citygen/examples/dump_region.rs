//! Dump a region for the renderer and for eyeballing the mesher.
//!
//! ```text
//! cargo run -p omniget-citygen --example dump_region -- [input.json] [out_dir] [px]
//! ```
//!
//! `input.json` is a city program (`omni.city.program/v1`, solved with the
//! example charter), a blueprint (`omni.city.blueprint/v1`) or `default` (the
//! solver's default program). Without an input (or `-`), or when the solver
//! fails, the mesher's built-in sample blueprint is used.
//! Writes `region.json` (blueprint), `instances.json`, `terrain.json`,
//! `kit-catalog.json` and `preview-{top,n,e,s,w}.png`.
//!
//! Triangle budget of the example city (the regions of the example charter's
//! districts; `[0,0]` gets the default program, the rest are empty):
//!
//! ```text
//! cargo run -p omniget-citygen --example dump_region -- city[:seed] <kit-tris.json>
//! ```
//!
//! `kit-tris.json` is `{key: {"lod0": n, "lod1": n}}` as written by
//! `omniget-codegeo`'s `kit_report`. Prints, per region and layer, instances
//! × kit triangles for the full kit and for `lod1`, then an estimate of what
//! the `three3d` renderer draws per tier with the default camera over the
//! centre of region `[0,0]` (the tier table of `render/tier.ts`: streaming
//! budget, region culling, detail distance, `lodDistance`, outlines). Add
//! `nolod` as a third argument to estimate the renderer before the far
//! variants (full modules and outlines at every distance).

use omniget_citygen::program::CityProgram;
use omniget_citygen::{charter, mesher, preview, solver, Blueprint};
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let input = args.first().filter(|a| a.as_str() != "-").cloned();
    let out =
        std::path::PathBuf::from(args.get(1).cloned().unwrap_or_else(|| "region-dump".into()));
    let px: u32 = args.get(2).and_then(|p| p.parse().ok()).unwrap_or(768);
    if let Some(seed) = input.as_deref().and_then(|i| i.strip_prefix("city")) {
        let seed = seed.strip_prefix(':').unwrap_or("omni");
        city_triangles(
            seed,
            args.get(1).map(String::as_str),
            args.get(2).is_some_and(|a| a == "nolod"),
        );
        return;
    }
    std::fs::create_dir_all(&out).expect("create out dir");

    let bp = load(input.as_deref());
    println!(
        "blueprint {} region {:?}, {} buildings, {} lots",
        bp.city_id,
        bp.region,
        bp.buildings.len(),
        bp.lots.len()
    );

    let t = Instant::now();
    let inst = mesher::mesh_region(&bp);
    let t_mesh = t.elapsed();
    let t = Instant::now();
    let terrain = mesher::terrain_mesh(&bp);
    let t_terrain = t.elapsed();
    let again = mesher::mesh_region(&bp);
    assert_eq!(inst.hash, again.hash, "mesher is not deterministic");

    let keys: std::collections::BTreeSet<&str> =
        inst.instances.iter().map(|i| i.key.as_str()).collect();
    println!(
        "instances {} ({} distinct keys) in {:?}, hash {}",
        inst.instances.len(),
        keys.len(),
        t_mesh,
        inst.hash
    );
    for (layer, n) in mesher::layer_counts(&inst) {
        println!("  {layer:8} {n}");
    }
    println!(
        "terrain {} triangles, {} vertices in {:?}",
        terrain.indices.len() / 3,
        terrain.slots.len(),
        t_terrain
    );

    let write = |name: &str, bytes: Vec<u8>| std::fs::write(out.join(name), bytes).expect("write");
    write("region.json", serde_json::to_vec_pretty(&bp).unwrap());
    write("instances.json", serde_json::to_vec(&inst).unwrap());
    write("terrain.json", serde_json::to_vec(&terrain).unwrap());
    write(
        "kit-catalog.json",
        serde_json::to_vec_pretty(&mesher::kit_catalog()).unwrap(),
    );
    let t = Instant::now();
    for (name, png) in preview::render_all(&bp, &inst, px) {
        write(&format!("preview-{name}.png"), png);
    }
    println!("previews in {:?} -> {}", t.elapsed(), out.display());
}

fn load(input: Option<&str>) -> Blueprint {
    let Some(path) = input else {
        println!("no input: sample blueprint");
        return mesher::sample::sample_blueprint();
    };
    let v: serde_json::Value = if path == "default" {
        serde_json::to_value(solver::default_program("omni", [0, 0])).unwrap()
    } else {
        serde_json::from_str(&std::fs::read_to_string(path).expect("read input")).expect("json")
    };
    let schema = v.get("schema").and_then(|s| s.as_str()).unwrap_or("");
    if schema.starts_with("omni.city.blueprint") {
        return serde_json::from_value(v).expect("blueprint");
    }
    match serde_json::from_value::<CityProgram>(v) {
        Ok(p) => {
            let c = charter::example(&p.city_id, "omni");
            match solver::solve(&c, &p) {
                Ok(bp) => bp,
                Err(v) => {
                    println!("solver failed ({} violations): sample blueprint", v.len());
                    for x in v.iter().take(8) {
                        println!("  {} {} {}", x.code, x.id, x.message);
                    }
                    mesher::sample::sample_blueprint()
                }
            }
        }
        Err(e) => {
            println!("not a program ({e}): sample blueprint");
            mesher::sample::sample_blueprint()
        }
    }
}

/// Kit triangles per key, `(lod0, lod1)`.
fn kit_tris(path: Option<&str>) -> std::collections::BTreeMap<String, (u64, u64)> {
    let Some(path) = path else {
        return Default::default();
    };
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).expect("read kit tris")).expect("json");
    v.as_object()
        .expect("object")
        .iter()
        .map(|(k, t)| {
            let l0 = t["lod0"].as_u64().unwrap_or(0);
            (k.clone(), (l0, t["lod1"].as_u64().unwrap_or(l0)))
        })
        .collect()
}

/// `render/tier.ts` → `TIER3D_BUDGETS`: draw, detail and LOD distances (m),
/// instance budget, outline.
const TIERS: [(f64, f64, f64, usize, bool); 4] = [
    (110.0, 36.0, 55.0, 3_000, false),
    (180.0, 70.0, 85.0, 12_000, false),
    (280.0, 130.0, 105.0, 40_000, true),
    (420.0, 220.0, 170.0, 120_000, true),
];
const OUTLINED: [&str; 6] = ["terreo", "parede", "telhado", "porta", "torre", "junta"];

struct Loaded {
    region: [i32; 2],
    inst: omniget_citygen::types::InstanceList,
    terrain: u64,
}

/// What the renderer draws per tier (upper bound: no frustum culling, no
/// shadow pass). Camera: `lookAt` default, 90 m from the centre of region
/// `[0,0]` at 52° from the vertical, looking north.
fn tier_estimate(
    regions: &[Loaded],
    tris: &std::collections::BTreeMap<String, (u64, u64)>,
    nolod: bool,
) {
    let target = [64.0f64, 0.0, 64.0];
    let a = 52f64.to_radians();
    let cam = [target[0], 90.0 * a.cos(), target[2] + 90.0 * a.sin()];
    let radius = (64f64 * 64.0 * 2.0 + 100.0).sqrt();
    for (tier, (draw, detail, lod, budget, outline)) in TIERS.iter().enumerate() {
        let mut want: Vec<(f64, &Loaded)> = regions
            .iter()
            .map(|l| {
                let c = [
                    l.region[0] as f64 * 128.0 + 64.0,
                    l.region[1] as f64 * 128.0 + 64.0,
                ];
                ((c[0] - target[0]).hypot(c[1] - target[2]), l)
            })
            .filter(|(d, _)| *d <= draw + 128.0 * 0.72)
            .collect();
        want.sort_by(|a, b| a.0.total_cmp(&b.0));
        let (mut loaded, mut n_inst) = (vec![], 0usize);
        for (_, l) in want {
            if n_inst >= *budget {
                break;
            }
            n_inst += l.inst.instances.len();
            loaded.push(l);
        }
        let (mut kit, mut ol, mut terrain, mut shown, mut near_n, mut far_n) =
            (0u64, 0u64, 0u64, 0usize, 0usize, 0usize);
        for l in &loaded {
            let (ox, oz) = (l.region[0] as f64 * 128.0, l.region[1] as f64 * 128.0);
            if ((ox + 64.0) - cam[0]).hypot((oz + 64.0) - cam[2]) - radius >= *draw {
                continue;
            }
            shown += 1;
            terrain += l.terrain;
            for i in &l.inst.instances {
                let layer = i.key.split('/').next().unwrap_or("");
                let is_detail = matches!(layer, "enfeite" | "prop");
                let size = if is_detail { 32.0 } else { 64.0 };
                let cx = ox + ((i.x_mm as f64 / 1000.0) / size).floor() * size + size / 2.0;
                let cz = oz + ((i.z_mm as f64 / 1000.0) / size).floor() * size + size / 2.0;
                let d = ((cx - cam[0]).powi(2) + cam[1].powi(2) + (cz - cam[2]).powi(2)).sqrt();
                if is_detail && d >= *detail {
                    continue;
                }
                let (l0, l1) = tris.get(&i.key).copied().unwrap_or((0, 0));
                let near = nolod || d < *lod;
                if near {
                    kit += l0;
                    near_n += 1;
                    if *outline && OUTLINED.contains(&layer) {
                        ol += l0;
                    }
                } else {
                    kit += l1;
                    far_n += 1;
                }
            }
        }
        println!(
            "tier {tier}: {} regions loaded, {shown} drawn; instances near {near_n} far {far_n}; kit {kit} + outline {ol} + terrain {terrain} = {} triangles",
            loaded.len(),
            kit + ol + terrain
        );
    }
}

fn city_triangles(seed: &str, tris_path: Option<&str>, nolod: bool) {
    let tris = kit_tris(tris_path);
    // Same search as the app's example founding: the first seed whose land
    // lets the default program solve.
    let (c, bp0) = (0..24)
        .find_map(|n| {
            let s = if n == 0 {
                seed.to_string()
            } else {
                format!("{seed}-{n}")
            };
            let c = charter::example("omni", &s);
            let bp = solver::solve(&c, &solver::default_program("omni", [0, 0])).ok()?;
            Some((c, bp))
        })
        .expect("a seed that solves");
    println!("city seed {}", c.seed);
    let mut regions: Vec<[i32; 2]> = c.districts.iter().flat_map(|d| d.regions.clone()).collect();
    regions.sort();
    regions.dedup();
    let (mut all0, mut all1, mut all_terrain, mut all_inst) = (0u64, 0u64, 0u64, 0usize);
    let mut unknown = std::collections::BTreeSet::new();
    let mut loaded = vec![];
    for r in regions {
        let bp = if r == [0, 0] {
            bp0.clone()
        } else {
            match solver::solve(&c, &CityProgram::empty("omni", r)) {
                Ok(bp) => bp,
                Err(v) => {
                    println!("region {r:?}: does not solve ({} violations)", v.len());
                    continue;
                }
            }
        };
        let inst = mesher::mesh_region(&bp);
        let terrain = mesher::terrain_mesh(&bp);
        let tt = (terrain.indices.len() / 3) as u64;
        let mut by_layer: std::collections::BTreeMap<String, (usize, u64, u64)> =
            Default::default();
        for i in &inst.instances {
            let layer = i.key.split('/').next().unwrap_or("").to_string();
            let (a, b) = match tris.get(&i.key) {
                Some(t) => *t,
                None => {
                    unknown.insert(i.key.clone());
                    (0, 0)
                }
            };
            let e = by_layer.entry(layer).or_default();
            e.0 += 1;
            e.1 += a;
            e.2 += b;
        }
        let (s0, s1): (u64, u64) = by_layer
            .values()
            .fold((0, 0), |a, v| (a.0 + v.1, a.1 + v.2));
        println!(
            "region {r:?}: {} instances, kit {s0} tris (lod1 {s1}), terrain {tt}, total {} (lod1 {})",
            inst.instances.len(),
            s0 + tt,
            s1 + tt
        );
        for (l, (n, a, b)) in &by_layer {
            println!("  {l:8} {n:6} inst {a:8} tris  lod1 {b:8}");
        }
        all0 += s0;
        all1 += s1;
        all_terrain += tt;
        all_inst += inst.instances.len();
        loaded.push(Loaded {
            region: r,
            inst,
            terrain: tt,
        });
    }
    println!(
        "city: {all_inst} instances, kit {all0} tris (lod1 {all1}), terrain {all_terrain}, total {} (lod1 {})",
        all0 + all_terrain,
        all1 + all_terrain
    );
    if !unknown.is_empty() {
        println!("keys without kit triangles: {unknown:?}");
    }
    if !tris.is_empty() {
        tier_estimate(&loaded, &tris, nolod);
    }
}
