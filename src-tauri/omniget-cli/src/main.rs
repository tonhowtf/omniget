use clap::Parser;

mod commands;
mod cookies;
mod output;
mod reporter;

#[derive(Parser)]
#[command(name = "omniget", version, about = "Download media from 1800+ sites", long_about = None)]
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
        about = "The /llm audit log of the app: who did what to jobs, questions, grants, tokens and agents"
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
        about = "Meetings: list, read, search and import recordings (Teams/SharePoint or files); transcript, summary and action items"
    )]
    Meetings {
        #[command(subcommand)]
        command: commands::meetings::MeetingsCommand,
    },
    #[command(
        about = "Microsoft Teams through Graph: export chats and channels as Markdown/JSON, list calendar meetings, import official transcripts"
    )]
    Teams {
        #[command(subcommand)]
        command: commands::teams::TeamsCommand,
    },
    #[command(
        about = "washmybelly: scan, plan and clean this computer through the running app (execute asks in the window)"
    )]
    Wmb {
        #[command(subcommand)]
        command: commands::wmb::WmbCommand,
    },
    #[command(
        about = "Open Claude Code on one of the app's accounts (/llm → Accounts), skipping permission prompts"
    )]
    Claude {
        #[arg(
            help = "Account id, label or e-mail (a unique prefix works); without it, the main account (asks when there is none)"
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
            help = "Make ACCOUNT the main one, here and in the app (asks when ACCOUNT is missing); with --add, the new one"
        )]
        default: bool,

        #[arg(long, help = "Ask which account to open even when there is a main one")]
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
        Commands::Meetings { command } => {
            commands::meetings::execute(command, cli.json).await?;
        }
        Commands::Teams { command } => {
            commands::teams::execute(command, cli.json).await?;
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
