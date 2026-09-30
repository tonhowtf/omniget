//! Rigs a static humanoid with skin-tokens.cpp through our provider and
//! normalizes the result as a character.
//! `cargo run -p omniget-asset3d --release --example autorig -- <static.glb> <skintokens-cli> <F16 dir> <out.glb>`

use omniget_asset3d::providers::skintokens::{SkinTokens, SkinTokensConfig};
use omniget_asset3d::providers::Rigger;
use omniget_asset3d::*;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [input, cli, model_dir, out] = &args[..] else {
        eprintln!("usage: autorig <static.glb> <skintokens-cli> <F16 dir> <out.glb>");
        std::process::exit(2);
    };
    let bytes = std::fs::read(input).expect("read input");
    let work = std::env::temp_dir().join("omniget-autorig");
    let cfg = SkinTokensConfig {
        cli: cli.into(),
        model_dir: model_dir.into(),
        device: "auto".into(),
        work_dir: work,
        timeout_secs: 600,
    };
    assert!(cfg.is_ready(), "binary or weights missing");

    let t = Instant::now();
    let plain = plain_glb_for_rigging(&bytes).expect("plain glb");
    println!(
        "plain: {} KB in {:.0} ms",
        plain.len() / 1024,
        t.elapsed().as_secs_f64() * 1000.0
    );

    let t = Instant::now();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let rigged = rt
        .block_on(SkinTokens { cfg }.rig(plain))
        .expect("skin-tokens");
    println!(
        "rig: {} KB in {:.1} s",
        rigged.len() / 1024,
        t.elapsed().as_secs_f64()
    );
    std::fs::write(format!("{out}.raw.glb"), &rigged).unwrap();

    let t = Instant::now();
    let mut opts = NormalizeOptions::new("autorig", AssetClass::Character);
    opts.license = Some(License {
        kind: LicenseKind::Own,
        author: None,
        url: None,
    });
    let a = normalize(Input::bytes(rigged, "rigged.glb"), opts).expect("normalize");
    println!(
        "normalize: {:?} {} KB, {} tris, h {} mm, collision {:?} in {:.0} ms",
        a.class,
        a.glb.len() / 1024,
        a.report.output.triangles,
        a.footprint.size_mm[1],
        a.collision,
        t.elapsed().as_secs_f64() * 1000.0
    );
    for w in &a.report.warnings {
        println!("  warn: {w}");
    }
    std::fs::write(out, &a.glb).unwrap();
}
