use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use omniget_core::core::convert::{self, ConvertOptions};
use tokio_util::sync::CancellationToken;

use crate::output;

pub struct Args {
    pub files: Vec<String>,
    pub to: Option<String>,
    pub output: Option<String>,
    pub list: bool,
    pub tools: bool,
    pub options: ConvertOptions,
}

fn engines(route: &convert::Route) -> String {
    route
        .steps
        .iter()
        .map(|s| format!("{}→{} ({})", s.from, s.to, engine_name(s.engine)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn engine_name(engine: convert::Engine) -> &'static str {
    match engine {
        convert::Engine::Native => "built-in",
        convert::Engine::Ffmpeg => "ffmpeg",
        convert::Engine::Pandoc => "pandoc",
        convert::Engine::ImageMagick => "imagemagick",
        convert::Engine::LibreOffice => "libreoffice",
    }
}

fn resolve_target(raw: &str) -> Option<&'static str> {
    let lower = raw.trim().trim_start_matches('.').to_ascii_lowercase();
    convert::formats::by_id(&lower)
        .or_else(|| convert::formats::by_extension(&lower))
        .map(|f| f.id)
}

pub async fn execute(args: Args) -> Result<()> {
    let tools = convert::detect_tools().await;

    if args.tools {
        if output::is_json_mode() {
            output::print_json(&tools);
        } else {
            println!("ffmpeg       {}", found(tools.ffmpeg));
            println!("pandoc       {}", found(tools.pandoc));
            println!("imagemagick  {}", found(tools.imagemagick));
            println!("libreoffice  {}", found(tools.libreoffice));
        }
        return Ok(());
    }

    if args.files.is_empty() {
        bail!("Pass at least one file. Example: omniget convert notes.md --to html");
    }

    if args.list || args.to.is_none() {
        let mut all = Vec::new();
        for file in &args.files {
            all.push(convert::inspect(Path::new(file), &tools).await);
        }
        if output::is_json_mode() {
            output::print_json(&all);
            return Ok(());
        }
        for res in &all {
            match (&res.format, &res.error) {
                (_, Some(err)) => println!("{}: {}", res.name, err),
                (Some(fmt), None) => {
                    println!("{} ({})", res.name, fmt.label);
                    for t in &res.targets {
                        match t.missing_tool {
                            Some(tool) => println!("  {:<6} needs {}", t.format, engine_name(tool)),
                            None => println!("  {:<6} {}", t.format, engines(&t.route)),
                        }
                    }
                }
                (None, None) => println!("{}: unknown format", res.name),
            }
        }
        if args.to.is_none() && !args.list {
            println!("\nChoose one with --to <format>.");
        }
        return Ok(());
    }

    let raw_target = args.to.unwrap_or_default();
    let Some(target) = resolve_target(&raw_target) else {
        bail!("Unknown format \"{raw_target}\". Run omniget convert <file> to see the formats it can become.");
    };

    let out_dir = args.output.map(PathBuf::from);
    if let Some(dir) = &out_dir {
        std::fs::create_dir_all(dir)?;
    }

    let cancel = CancellationToken::new();
    let ctrl = cancel.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            ctrl.cancel();
        }
    });

    let pairs: Vec<(PathBuf, String)> = args
        .files
        .iter()
        .map(|f| (PathBuf::from(f), target.to_string()))
        .collect();
    let planned = convert::plan_outputs(
        &pairs,
        out_dir.as_deref(),
        args.options.overwrite.unwrap_or(false),
    );
    let mut outcomes = Vec::new();
    let mut failed = 0usize;
    let json = output::is_json_mode();
    let live = !json && std::io::stderr().is_terminal();
    for (index, file) in args.files.iter().enumerate() {
        let input = Path::new(file);
        let name = input
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| file.clone());
        let label = name.clone();
        let report = move |fraction: f64| {
            if live && fraction > 0.0 && fraction < 1.0 {
                eprint!("\r{label}: {:>3.0}%", fraction * 100.0);
                let _ = std::io::stderr().flush();
            }
        };
        let outcome = match planned.get(index).cloned().flatten() {
            Some(output) => {
                convert::run_to(
                    input,
                    target,
                    &output,
                    &args.options,
                    &tools,
                    cancel.clone(),
                    report,
                )
                .await
            }
            None => {
                convert::run(
                    input,
                    target,
                    out_dir.as_deref(),
                    &args.options,
                    &tools,
                    cancel.clone(),
                    report,
                )
                .await
            }
        };
        if live {
            eprint!("\r\x1b[2K");
        }
        if !json {
            if outcome.skipped {
                println!("{name}: already {target}, skipped");
            } else if outcome.ok {
                println!("{name} → {}", outcome.output.as_deref().unwrap_or_default());
            } else {
                println!(
                    "{name}: {}",
                    outcome.error.as_deref().unwrap_or("conversion failed")
                );
            }
        }
        if !outcome.ok {
            failed += 1;
        }
        outcomes.push(outcome);
        if cancel.is_cancelled() {
            break;
        }
    }

    if json {
        output::print_json(&outcomes);
    }
    if failed > 0 {
        std::process::exit(1);
    }
    Ok(())
}

fn found(ok: bool) -> &'static str {
    if ok {
        "found"
    } else {
        "not found"
    }
}
