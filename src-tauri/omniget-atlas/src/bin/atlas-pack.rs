//! `atlas-pack <src>... <out-dir>` — source art into atlas pages.
//!
//! Exit codes: 0 packed, 1 the source is wrong, 2 the command line is wrong.

use std::path::PathBuf;
use std::process::ExitCode;

use omniget_atlas::pack::{pack, PackOptions};
use omniget_atlas::schema::PAGE_MAX;

const USAGE: &str = "\
atlas-pack <src>... <out-dir>

  <src>       a character source (<sheet>/<anim>/<DIR>/<n>.png, optional manifest.json)
              or a tile source (floor/ wall/ object/ plus tiles.json)
  <out-dir>   where <sheet>-<n>.png and atlas.json are written

Options:
  --page-max N   maximum page side, power of two, default 2048
  --padding N    gutter between frames, default 2
  --extrude N    duplicated edge pixels, default 1
  --fixture      write a synthetic source tree into <out-dir> instead of packing
";

fn main() -> ExitCode {
    let mut opts = PackOptions::default();
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut fixture = false;
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            "--fixture" => fixture = true,
            "--page-max" | "--padding" | "--extrude" => {
                let Some(value) = args.next().and_then(|v| v.parse::<u32>().ok()) else {
                    eprintln!("atlas-pack: {arg} needs a number\n\n{USAGE}");
                    return ExitCode::from(2);
                };
                match arg.as_str() {
                    "--page-max" => opts.page_max = value,
                    "--padding" => opts.padding = value,
                    _ => opts.extrude = value,
                }
            }
            other if other.starts_with('-') => {
                eprintln!("atlas-pack: unknown option {other}\n\n{USAGE}");
                return ExitCode::from(2);
            }
            other => paths.push(PathBuf::from(other)),
        }
    }

    if opts.page_max < 16 || opts.page_max > PAGE_MAX {
        eprintln!("atlas-pack: --page-max must be between 16 and {PAGE_MAX}");
        return ExitCode::from(2);
    }

    if fixture {
        let Some(out) = paths.first() else {
            eprintln!("atlas-pack: --fixture needs an output folder\n\n{USAGE}");
            return ExitCode::from(2);
        };
        return match write_fixture(out) {
            Ok(()) => {
                println!("atlas-pack: synthetic source written to {}", out.display());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("ERR_ATLAS_SRC_IO: {e}");
                ExitCode::from(1)
            }
        };
    }

    if paths.len() < 2 {
        eprintln!("atlas-pack: need at least one source and one output folder\n\n{USAGE}");
        return ExitCode::from(2);
    }
    let out_dir = paths.pop().expect("checked above");

    match pack(&paths, &out_dir, &opts) {
        Ok(report) => {
            println!(
                "atlas-pack: {} frames, {} anims, {} tiles -> {} page(s), {:.1}% filled",
                report.frame_count,
                report.anim_count,
                report.tile_count,
                report.page_paths.len(),
                report.fill * 100.0
            );
            for path in &report.page_paths {
                println!("  {}", path.display());
            }
            println!("  {}", report.atlas_path.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(1)
        }
    }
}

fn write_fixture(out: &std::path::Path) -> std::io::Result<()> {
    use omniget_atlas::fixture;
    let omni = out.join("omni-src");
    fixture::write_character(&omni, "omni", &[("idle", 4), ("walk", 8)], 48, 64)?;
    fixture::write_manifest(
        &omni,
        "{\n  \"default_fps\": 10,\n  \"anims\": {\n    \"omni/walk\": { \"fps\": 12, \"pivot\": [24, 62] }\n  }\n}\n",
    )?;
    fixture::write_tiles(&out.join("casa-v1-src"))
}
