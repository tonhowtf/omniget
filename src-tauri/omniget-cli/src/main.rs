use clap::Parser;

mod commands;
mod cookies;
mod output;
mod reporter;

#[derive(Parser)]
#[command(name = "omniget", version, about = "Download media from about 1,800 sites", long_about = None)]
struct Cli {
    #[arg(long, global = true, help = "Output in JSON format")]
    json: bool,

    #[arg(long, global = true, help = "Proxy URL (e.g. http://127.0.0.1:7897)")]
    proxy: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(clap::Subcommand)]
enum Commands {
    #[command(about = "Download a video/audio from URL")]
    Download {
        url: String,

        #[arg(short, long, help = "Video quality height (e.g. 720, 1080)")]
        quality: Option<u32>,

        #[arg(short, long, help = "Output directory")]
        output: Option<String>,

        #[arg(long, help = "Download audio only")]
        audio_only: bool,

        #[arg(long, help = "Subtitle languages (e.g. en,zh-Hans)")]
        subs: Option<String>,

        #[arg(long, help = "Format preference (mp4/mkv/webm)")]
        format: Option<String>,
    },
    #[command(about = "Preview media info without downloading")]
    Info { url: String },
    #[command(about = "Batch download from a file (one URL per line)")]
    Batch {
        file: String,

        #[arg(short, long, default_value = "3")]
        max_concurrent: usize,

        #[arg(short, long)]
        output: Option<String>,
    },
    #[command(
        about = "Agents of the desktop app: run a task, loop until a check passes, list jobs"
    )]
    Agent {
        #[command(subcommand)]
        command: commands::agent::AgentCommand,
    },
    #[command(
        about = "Companies of agents: org chart, issues, heartbeats, budgets and approvals of the running app"
    )]
    Company {
        #[command(subcommand)]
        command: commands::company::CompanyCommand,
    },
    #[command(
        about = "The audit log of the app: who did what to jobs, questions, grants, tokens and agents"
    )]
    Audit {
        #[arg(long, default_value = "1d", help = "How far back: 30m, 12h, 1d, 2w")]
        since: String,
        #[arg(
            long,
            help = "Only actions starting with this (job., ask.answered, token.)"
        )]
        action: Option<String>,
        #[arg(long, default_value = "200")]
        limit: usize,
    },
    #[command(about = "Short reference texts: `omniget guide agents` (delegating to your agents)")]
    Guide {
        #[arg(help = "Topic (none = the list)")]
        topic: Option<String>,
    },
    #[command(
        about = "Call a dev_*, gd_*, transfer_* or research_* tool of the running app: `omniget tool` lists them, `omniget tool dev_ports op=list` calls one"
    )]
    Tool {
        #[arg(help = "Tool name (none = the list)")]
        name: Option<String>,
        #[arg(help = "Input as key=value pairs or one JSON object")]
        args: Vec<String>,
    },
    #[command(
        about = "washmybelly: scan, plan and clean this computer through the running app (execute asks in the window)"
    )]
    Wmb {
        #[command(subcommand)]
        command: commands::wmb::WmbCommand,
    },
    #[command(
        about = "Open Claude Code on one of the app's accounts (Settings → AI & agents), skipping permission prompts"
    )]
    Claude {
        #[arg(
            help = "Account id, label or e-mail (a unique prefix works); without it, a list of the accounts with the main one preselected (the main one directly when not in a terminal)"
        )]
        account: Option<String>,

        #[arg(long, help = "List the Claude accounts (e-mail and plan) and exit")]
        list: bool,

        #[arg(
            long,
            help = "Add a new account: log in with Claude Code and name it after the e-mail (or ACCOUNT)"
        )]
        add: bool,

        #[arg(
            long,
            help = "Make ACCOUNT the main one, here, in the app and for plain `claude` in the terminal (asks when ACCOUNT is missing); with --add, the new one"
        )]
        default: bool,

        #[arg(
            long,
            help = "Share settings, CLAUDE.md, skills, commands, agents, plugins and MCP servers of ~/.claude with the other accounts (only ACCOUNT when given)"
        )]
        sync: bool,

        #[arg(
            long,
            help = "Always ask which account to open, even outside a terminal"
        )]
        pick: bool,

        #[arg(
            long,
            help = "Remove ACCOUNT from OmniGet; its login folder stays on disk"
        )]
        remove: bool,

        #[arg(
            long,
            help = "Keep Claude Code's permission prompts (no --dangerously-skip-permissions)"
        )]
        safe: bool,

        #[arg(
            long,
            help = "Do not print the OmniGet splash (also OMNIGET_NO_SPLASH=1)"
        )]
        no_splash: bool,

        #[arg(help = "Extra arguments for claude, after `--` (e.g. -- -c)")]
        #[arg(last = true)]
        extra: Vec<String>,
    },
    #[command(
        about = "Usage monitor: 5 h / 7 d windows and spend of each Claude Code / Codex account"
    )]
    Usage {
        #[arg(long, default_value = "7", help = "Days of history to sum")]
        days: u32,

        #[arg(long, value_name = "SECONDS", help = "Keep refreshing every N seconds")]
        watch: Option<u64>,
    },
    #[command(
        about = "Catalog of agents, commands, skills, MCP servers, hooks and settings for every coding tool (Claude Code, Codex, Gemini, Cursor, Copilot, OpenCode…)"
    )]
    Agentkit {
        #[command(subcommand)]
        command: commands::agentkit::AgentkitCommand,
    },
    #[command(
        about = "Cords of the OmniGet canvas: talk to the agents, notes and portals wired to this terminal (`omniget canvas help`)"
    )]
    #[command(disable_help_flag = true)]
    Canvas {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    #[command(
        about = "Hooks OmniGet installs in a coding tool's turn (run by the tool, not by hand)"
    )]
    #[command(hide = true)]
    Hook {
        #[command(subcommand)]
        command: HookCommand,
    },
    #[command(
        about = "Telegram: chats, media, downloads from links or whole chats, auto-save rules, export"
    )]
    Tg {
        #[arg(
            long,
            global = true,
            help = "Use the session from this process even when the app is running"
        )]
        local: bool,

        #[command(subcommand)]
        command: commands::tg::TgCommand,
    },
    #[command(
        about = "Convert files between formats: data (JSON, YAML, TOML, CSV, XML), documents (Markdown, HTML, EPUB, DOCX, PDF), images (including HEIC, JPEG XL, PSD and camera RAW), CAD drawings and audio/video",
        long_about = "Convert files between formats. Data, Markdown, HTML, EPUB, images (HEIC, JPEG XL, Photoshop, camera RAW, SVG), picture to SVG tracing and DXF/DWG drawings convert with no extra tools; audio and video use ffmpeg; other documents use Pandoc or LibreOffice and AVIF uses ImageMagick when installed.\n\nExamples:\n  omniget convert notes.md --to html\n  omniget convert data.json --to yaml --sort-keys\n  omniget convert *.png --to webp --max-width 1600 -o out/\n  omniget convert IMG_0001.HEIC --to jpg\n  omniget convert art.psd --to layers   (one PNG per layer)\n  omniget convert logo.png --to svg --style bw\n  omniget convert plan.dwg --to pdf\n  omniget convert clip.mp4            (lists what it can become)\n  omniget convert --tools"
    )]
    Convert {
        #[arg(help = "Files to convert")]
        files: Vec<String>,
        #[arg(
            short,
            long,
            help = "Target format (id or extension: yaml, md, jpg, mp3...)"
        )]
        to: Option<String>,
        #[arg(short, long, help = "Output folder (default: next to each file)")]
        output: Option<String>,
        #[arg(long, help = "List the formats each file can become")]
        list: bool,
        #[arg(long, help = "Show which external engines were found")]
        tools: bool,
        #[arg(long, help = "Compact data output instead of pretty printing")]
        compact: bool,
        #[arg(long, help = "Sort object keys in data output")]
        sort_keys: bool,
        #[arg(long, help = "JPEG quality, 1 to 100")]
        quality: Option<u8>,
        #[arg(long, help = "Shrink images wider than this (pixels)")]
        max_width: Option<u32>,
        #[arg(long, help = "Shrink images taller than this (pixels)")]
        max_height: Option<u32>,
        #[arg(
            long,
            help = "Replace an existing output file instead of adding (1), (2)"
        )]
        overwrite: bool,
        #[arg(
            long,
            value_parser = ["bw", "colors", "lineart"],
            help = "Style when turning a picture into SVG: bw (black and white), colors (few colors) or lineart"
        )]
        style: Option<String>,
    },
    #[command(about = "Books and manga: metadata, legal catalogs, OPDS, text to EPUB")]
    Read {
        #[command(subcommand)]
        command: commands::read::ReadCommand,
    },
    #[command(about = "Import a cookies.txt file (Netscape format)")]
    ImportCookies {
        file: String,

        #[arg(short, long, help = "Account name (default: cookies.txt)")]
        name: Option<String>,

        #[arg(long, help = "Preview without importing")]
        dry_run: bool,
    },
}

#[derive(clap::Subcommand)]
enum HookCommand {
    #[command(
        about = "Claude Code `PreToolUse`/`PostToolUse`: OmniGet's permission rules judge the CLI's own tools (reads the hook JSON on stdin)"
    )]
    Claude {
        #[arg(long)]
        event: String,
        #[arg(help = "The turn's private folder (policy.json)")]
        #[arg(long)]
        turn: Option<String>,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if let Some(code) = omniget_core::core::convert::cad::worker_entry() {
        std::process::exit(code);
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "omniget_cli=info".into()),
        )
        .init();

    let cli = Cli::parse();

    if cli.json {
        output::set_json_mode(true);
    }

    match cli.command {
        Commands::Download {
            url,
            quality,
            output,
            audio_only,
            subs,
            format,
        } => {
            commands::download::execute(url, quality, output, audio_only, subs, format, cli.proxy)
                .await?;
        }
        Commands::Convert {
            files,
            to,
            output,
            list,
            tools,
            compact,
            sort_keys,
            quality,
            max_width,
            max_height,
            overwrite,
            style,
        } => {
            commands::convert::execute(commands::convert::Args {
                files,
                to,
                output,
                list,
                tools,
                options: omniget_core::core::convert::ConvertOptions {
                    pretty: Some(!compact),
                    sort_keys: Some(sort_keys),
                    quality,
                    max_width,
                    max_height,
                    overwrite: Some(overwrite),
                    vector_preset: style,
                },
            })
            .await?;
        }
        Commands::Info { url } => {
            commands::info::execute(url, cli.proxy).await?;
        }
        Commands::Batch {
            file,
            max_concurrent,
            output,
        } => {
            commands::batch::execute(file, max_concurrent, output, cli.proxy).await?;
        }
        Commands::Agent { command } => {
            if let Err(e) = commands::agent::execute(command, cli.json).await {
                if let Some(down) = e.downcast_ref::<commands::launch::AppNotRunning>() {
                    if cli.json {
                        println!("{}", down.to_json());
                    }
                    eprintln!("Error: {down}");
                    std::process::exit(commands::launch::EXIT_APP_NOT_RUNNING);
                }
                return Err(e);
            }
        }
        Commands::Hook {
            command: HookCommand::Claude { event, turn },
        } => {
            let mut args = vec!["--event".to_string(), event];
            if let Some(turn) = turn {
                args.extend(["--turn".to_string(), turn]);
            }
            std::process::exit(omniget_core::core::llm::claude_hooks::main_with_args(&args));
        }
        Commands::Canvas { args } => {
            commands::canvas::execute(args, cli.json);
        }
        Commands::Guide { topic } => {
            commands::guide::execute(topic)?;
        }
        Commands::Company { command } => {
            commands::company::execute(command, cli.json).await?;
        }
        Commands::Tool { name, args } => {
            commands::tool::execute(name, args, cli.json).await?;
        }
        Commands::Wmb { command } => {
            commands::wmb::execute(command, cli.json).await?;
        }
        Commands::Audit {
            since,
            action,
            limit,
        } => {
            commands::audit::execute(since, action, limit, cli.json).await?;
        }
        Commands::Claude {
            account,
            list,
            add,
            default,
            sync,
            pick,
            remove,
            safe,
            no_splash,
            extra,
        } => {
            let flags = commands::claude::Flags {
                list,
                add,
                default,
                sync,
                pick,
                remove,
                safe,
                no_splash,
            };
            commands::claude::execute(account, flags, extra)?;
        }
        Commands::Usage { days, watch } => {
            commands::usage::execute(days, watch, cli.json).await?;
        }
        Commands::Agentkit { command } => {
            commands::agentkit::execute(command, cli.json).await?;
        }
        Commands::Tg { local, command } => {
            commands::tg::execute(command, local, cli.json).await?;
        }
        Commands::Read { command } => {
            commands::read::execute(command, cli.json).await?;
        }
        Commands::ImportCookies {
            file,
            name,
            dry_run,
        } => {
            commands::import_cookies::execute(file, name, dry_run).await?;
        }
    }

    Ok(())
}
