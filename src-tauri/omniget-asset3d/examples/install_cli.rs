//! Installs a `skintokens-cli` package the way the app does
//! (`city3d_autorig_install_cli`): download, sha256, flat extraction into
//! `<root>/bin/`. Validates a package built by `scripts/skintokens/build.sh`
//! before it is published, served from a local HTTP server.
//! `cargo run -p omniget-asset3d --example install_cli -- <url> <sha256> <root>`

use omniget_asset3d::providers::skintokens::{download_cli, SkinTokensConfig};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [url, sha, root] = &args[..] else {
        eprintln!("usage: install_cli <url> <sha256> <root>");
        std::process::exit(2);
    };
    let cfg = SkinTokensConfig::under(std::path::Path::new(root));
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let t = std::time::Instant::now();
    rt.block_on(download_cli(&cfg, url, Some(sha), &|_, _| {}))
        .expect("install");
    println!(
        "installed {} in {} ms",
        cfg.cli.display(),
        t.elapsed().as_millis()
    );
}
