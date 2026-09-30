//! Runs the normalizer on files and prints before/after.
//! `cargo run -p omniget-asset3d --release --example bench -- <class> <file>... [--fit] [--out DIR]`

use omniget_asset3d::*;

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let fit = args.iter().any(|a| a == "--fit");
    args.retain(|a| a != "--fit");
    let out = args.iter().position(|a| a == "--out").map(|i| {
        let d = args[i + 1].clone();
        args.drain(i..=i + 1);
        d
    });
    let class: AssetClass =
        serde_json::from_value(serde_json::json!(args.remove(0))).expect("class");
    for f in &args {
        let bytes = std::fs::read(f).expect("read");
        let name = std::path::Path::new(f)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let mut opts = NormalizeOptions::new(&name, class);
        opts.fit_style = fit;
        let mut input = Input::bytes(bytes, &name);
        if let Some(dir) = std::path::Path::new(f).parent() {
            input.resolver = Some(folder_resolver(dir.to_path_buf(), 64 << 20));
        }
        match normalize(input, opts) {
            Ok(a) => {
                let r = &a.report;
                println!(
                    "{name}\t{} KB -> {} KB\t{} -> {} tris\t{:.0} ms\tscale {:.3}\tfoot {}x{} h {} mm\tanchors {}\tfront {}\thash {}",
                    r.input.bytes / 1024,
                    r.output.bytes / 1024,
                    r.input.triangles,
                    r.output.triangles,
                    r.total_ms,
                    r.scale,
                    a.footprint.w_tiles,
                    a.footprint.d_tiles,
                    a.footprint.size_mm[1],
                    a.anchors.len(),
                    a.front,
                    &a.hash[..12]
                );
                for s in &r.steps {
                    print!("  {}={:.1}", s.step, s.ms);
                }
                println!();
                for w in &r.warnings {
                    println!("  warn: {w}");
                }
                if let Some(v) = &a.vrm {
                    println!("  vrm {} {:?}", v.version, v.permissions);
                }
                if let Some(d) = &out {
                    std::fs::create_dir_all(d).unwrap();
                    std::fs::write(format!("{d}/{name}.norm.glb"), &a.glb).unwrap();
                    std::fs::write(format!("{d}/{name}.thumb.png"), &a.thumbnail_png).unwrap();
                    if let Ok(v) = export_vox(&a.glb, 125) {
                        std::fs::write(format!("{d}/{name}.vox"), &v.bytes).unwrap();
                        println!(
                            "  vox {} voxels {:?} at {} mm",
                            v.voxels, v.size, v.voxel_mm
                        );
                    }
                    match validate_glb(&a.glb, 64 << 20, 2_000_000) {
                        Ok(s) => println!("  reread ok: {} tris", s.triangles),
                        Err(e) => println!("  reread FAILED: {e}"),
                    }
                }
            }
            Err(e) => println!("{name}\tERROR {e}"),
        }
    }
}
