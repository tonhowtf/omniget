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
    /// Download a video/audio from URL
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
    /// Preview media info without downloading
    Info {
        url: String,
    },
    /// Batch download from a file (one URL per line)
    Batch {
        file: String,

        #[arg(short, long, default_value = "3")]
        max_concurrent: usize,

        #[arg(short, long)]
        output: Option<String>,
    },
    /// Import a cookies.txt file (Netscape format)
    ImportCookies {
        file: String,

        #[arg(short, long, help = "Account name (default: cookies.txt)")]
        name: Option<String>,

        #[arg(long, help = "Preview without importing")]
        dry_run: bool,
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
            commands::download::execute(url, quality, output, audio_only, subs, format, cli.proxy).await?;
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
        Commands::ImportCookies { file, name, dry_run } => {
            commands::import_cookies::execute(file, name, dry_run).await?;
        }
    }

    Ok(())
}
