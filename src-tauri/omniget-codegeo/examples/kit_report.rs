//! Builds the kit of every style, prints triangles and time per module, and
//! writes the GLBs, a contact sheet per style and the mesher's sample region
//! drawn with the kit, as PNG.
//!
//! `cargo run -p omniget-codegeo --example kit_report -- <out_dir> [style]`

use omniget_codegeo::{canon::CanonMesh, kit, palette, raster};
use std::time::Instant;

fn main() {
    let mut args = std::env::args().skip(1);
    let out = std::path::PathBuf::from(args.next().unwrap_or_else(|| "kit-report".into()));
    let only = args.next();
    std::fs::create_dir_all(&out).expect("out dir");
    let mut tris_of = serde_json::Map::new();
    let mut styles: Vec<&str> = kit::STYLES.to_vec();
    styles.push(kit::ANY);
    for style in styles {
        if only.as_deref().is_some_and(|o| o != style) {
            continue;
        }
        let t0 = Instant::now();
        let k = kit::build_kit(style).expect("kit");
        let total = t0.elapsed().as_secs_f64() * 1e3;
        println!(
            "== {style}: {} modules, {:.0} ms, GLB {} KB, hash {}, glb sha256 {}",
            k.modules.len(),
            total,
            k.glb.len() / 1024,
            &k.hash[..16],
            {
                use sha2::Digest;
                let d = sha2::Sha256::digest(&k.glb);
                d.iter()
                    .take(8)
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>()
            }
        );
        let mut by_layer: std::collections::BTreeMap<String, Vec<(usize, f64, usize)>> =
            Default::default();
        for m in &k.modules {
            let s = &m.stats;
            let layer = s.key.split('/').next().unwrap_or("").to_string();
            if style != kit::ANY && kit::is_shared_layer(&layer) {
                continue;
            }
            let flag = if s.within_budget { "" } else { "  BUDGET" };
            println!(
                "  {:48} {:4} tris (lod1 {:3}) {:6.2} ms{}{}",
                s.key,
                s.triangles,
                s.lod1_triangles,
                s.ms,
                flag,
                if s.warnings.is_empty() {
                    String::new()
                } else {
                    format!("  {:?}", s.warnings)
                }
            );
            by_layer
                .entry(layer)
                .or_default()
                .push((s.triangles, s.ms, s.lod1_triangles));
        }
        for (l, v) in &by_layer {
            let t: Vec<usize> = v.iter().map(|x| x.0).collect();
            let ms: f64 = v.iter().map(|x| x.1).sum();
            let l1: usize = v.iter().map(|x| x.2).sum();
            println!(
                "  layer {l:8} n {:3} tris min {:4} max {:4} avg {:6.1}  lod1 avg {:6.1}  time {:7.1} ms",
                t.len(),
                t.iter().min().unwrap(),
                t.iter().max().unwrap(),
                t.iter().sum::<usize>() as f64 / t.len() as f64,
                l1 as f64 / t.len() as f64,
                ms
            );
        }
        for (key, e) in &k.errors {
            println!("  ERROR {key}: {e}");
        }
        if !k.aliases.is_empty() {
            println!("  aliases: {:?}", k.aliases);
        }
        if !k.missing.is_empty() {
            println!("  missing: {:?}", k.missing);
        }
        for m in &k.modules {
            tris_of.insert(
                m.key.clone(),
                serde_json::json!({ "lod0": m.stats.triangles, "lod1": m.stats.lod1_triangles }),
            );
        }
        std::fs::write(out.join(format!("kit-{style}.glb")), &k.glb).unwrap();
        let pal = palette::default_palette(if style == kit::ANY {
            "vila-colonial"
        } else {
            style
        });
        let meshes: Vec<CanonMesh> = k
            .modules
            .iter()
            .filter(|m| {
                style == kit::ANY || !kit::is_shared_layer(m.key.split('/').next().unwrap_or(""))
            })
            .map(|m| m.mesh.clone())
            .collect();
        let sheet = raster::sheet(&meshes, 14, 3.5);
        let png = raster::render_png(&sheet, &pal, 2000, 1400, &raster::View::default());
        std::fs::write(out.join(format!("sheet-{style}.png")), png).unwrap();
        let far: Vec<CanonMesh> = k
            .modules
            .iter()
            .filter(|m| {
                style == kit::ANY || !kit::is_shared_layer(m.key.split('/').next().unwrap_or(""))
            })
            .map(|m| m.lod1.clone())
            .collect();
        let sheet = raster::sheet(&far, 14, 3.5);
        let png = raster::render_png(&sheet, &pal, 2000, 1400, &raster::View::default());
        std::fs::write(out.join(format!("sheet-{style}-lod1.png")), png).unwrap();
    }

    if only.is_none() {
        std::fs::write(
            out.join("kit-tris.json"),
            serde_json::to_vec_pretty(&tris_of).unwrap(),
        )
        .unwrap();
    }

    // The mesher's sample region, drawn with the kit.
    let bp = omniget_citygen::mesher::sample::sample_blueprint();
    let list = omniget_citygen::mesher::mesh_region(&bp);
    let terrain = omniget_citygen::mesher::terrain_mesh(&bp);
    println!("sample region: {} instances", list.instances.len());
    let pal = palette::default_palette("vila-colonial");
    for (name, win) in [
        ("region", None),
        ("row", Some([0, 22, 18, 31])),
        ("block", Some([22, 3, 38, 16])),
        ("south", Some([0, 32, 44, 44])),
    ] {
        let (mesh, missing) = kit::assemble(&list, Some(&terrain), win);
        if !missing.is_empty() {
            println!("  {name}: keys the kit could not build: {missing:?}");
        }
        let png = raster::render_png(&mesh, &pal, 1600, 1100, &raster::View::default());
        std::fs::write(out.join(format!("sample-{name}.png")), png).unwrap();
        let (far, _) = kit::assemble_lod(&list, Some(&terrain), win, 1);
        let png = raster::render_png(&far, &pal, 1600, 1100, &raster::View::default());
        std::fs::write(out.join(format!("sample-{name}-lod1.png")), png).unwrap();
    }

    for ex in omniget_codegeo::examples() {
        match omniget_codegeo::run_with_mesh(ex.script, ex.class) {
            Ok((o, mesh)) => {
                println!(
                    "example {}: {} tris, {:.1} ms, anchors {:?}",
                    ex.id, o.triangles, o.ms, o.anchors
                );
                std::fs::write(out.join(format!("example-{}.glb", ex.id)), &o.glb).unwrap();
                let png = raster::render_png(&mesh, &pal, 600, 600, &raster::View::default());
                std::fs::write(out.join(format!("example-{}.png", ex.id)), png).unwrap();
            }
            Err(e) => println!("example {}: ERRORS {e:?}", ex.id),
        }
    }
}
