//! `omniget meetings`: the meetings of the desktop app, straight from the
//! data folder (the app does not have to be running). `import` runs the whole
//! pipeline here: download or convert, Whisper, summary, markdown.

use std::sync::Arc;

use anyhow::bail;
use omniget_core::core::meetings::{
    self, pipeline, publish, settings, teams, vexa, Meeting, MeetingSource, MeetingStatus,
};
use omniget_core::core::tools::ToolProgress;

#[derive(clap::Subcommand)]
pub enum MeetingsCommand {
    /// List meetings, newest first
    List {
        #[arg(long, default_value = "30")]
        limit: usize,
    },
    /// Print a meeting as markdown (summary, action items, transcript)
    Show {
        id: String,
        /// Leave the transcript out
        #[arg(long)]
        no_transcript: bool,
    },
    /// Search titles, summaries, action items and transcripts
    Search {
        query: String,
        #[arg(long, default_value = "20")]
        limit: usize,
    },
    /// Import an audio/video/transcript file or a Teams/SharePoint recording URL
    Import {
        source: String,
        #[arg(long)]
        title: Option<String>,
    },
    /// Summarize a meeting again with the configured AI provider
    Summarize { id: String },
    /// Post a meeting's summary to the Teams webhook set in the app
    Post { id: String },
    /// Open action items across every meeting (the follow-up list)
    Todo {
        /// Only items whose owner contains this
        #[arg(long)]
        owner: Option<String>,
        /// Include the ones already done
        #[arg(long)]
        all: bool,
    },
    /// Mark an action item done (index from `omniget meetings todo`)
    Done {
        id: String,
        index: usize,
        /// Open it again instead
        #[arg(long)]
        undo: bool,
    },
    /// Send the Vexa bot to a Teams / Meet / Zoom call (key set in the app)
    Bot {
        /// The meeting's join link
        url: String,
        #[arg(long)]
        title: Option<String>,
    },
    /// Pull what the bot transcribed; once the call ended (or with --stop) store it and summarize
    BotPull {
        id: String,
        /// Make the bot leave now and keep what it has
        #[arg(long)]
        stop: bool,
    },
}

fn line(m: &Meeting) -> String {
    let date = chrono::DateTime::from_timestamp_millis(m.created_at_ms as i64)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default();
    let actions = m
        .summary
        .as_ref()
        .map(|s| s.action_items.len())
        .unwrap_or(0);
    format!(
        "{}  {}  {:<12} {:>8}  {} action item(s)  {}",
        m.id,
        date,
        pipeline::status_name(m.status),
        meetings::transcript::fmt_clock(m.duration_ms),
        actions,
        m.title
    )
}

fn printer() -> omniget_core::core::tools::ProgressFn {
    Arc::new(|p: ToolProgress| match p.stage.as_str() {
        "status" => eprintln!("… {}", p.message.unwrap_or_default()),
        "transcribing" if p.total.is_some() => eprint!("\r  transcribing {:>3}%", p.done),
        "downloading" => eprint!(
            "\r  downloaded {}",
            meetings::transcript::fmt_clock(p.done * 1000)
        ),
        _ => {}
    })
}

pub async fn execute(cmd: MeetingsCommand, json: bool) -> anyhow::Result<()> {
    match cmd {
        MeetingsCommand::List { limit } => {
            let list: Vec<Meeting> = meetings::list().into_iter().take(limit).collect();
            if json {
                println!("{}", serde_json::to_string_pretty(&list)?);
            } else if list.is_empty() {
                println!("No meetings yet.");
            } else {
                for m in &list {
                    println!("{}", line(m));
                }
            }
        }
        MeetingsCommand::Show { id, no_transcript } => {
            let m = meetings::load(&id)?;
            let mut md = meetings::read_markdown(&id)?;
            if no_transcript {
                if let Some(i) = md.find("\n## Transcript\n") {
                    md.truncate(i + 1);
                }
            }
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &serde_json::json!({ "meeting": m, "markdown": md })
                    )?
                );
            } else {
                print!("{md}");
            }
        }
        MeetingsCommand::Search { query, limit } => {
            let hits = meetings::search(&query, limit);
            if json {
                println!("{}", serde_json::to_string_pretty(&hits)?);
            } else if hits.is_empty() {
                println!("Nothing found.");
            } else {
                for h in hits {
                    let at = h
                        .at_ms
                        .map(|ms| format!(" [{}]", meetings::transcript::fmt_clock(ms)))
                        .unwrap_or_default();
                    println!("{}  {}  {}{}: {}", h.id, h.title, h.field, at, h.snippet);
                }
            }
        }
        MeetingsCommand::Import { source, title } => {
            let src = if source.starts_with("http://") || source.starts_with("https://") {
                if teams::classify(&source).is_none() {
                    bail!("not a Teams/SharePoint recording URL; for other sites use `omniget download --audio-only` and import the file");
                }
                MeetingSource::Teams { url: source }
            } else {
                let path = std::fs::canonicalize(&source)?;
                MeetingSource::File {
                    path: path.to_string_lossy().to_string(),
                }
            };
            let m = meetings::create(title.as_deref(), src, MeetingStatus::Queued)?;
            eprintln!("Meeting {}", m.id);
            let done = pipeline::process(&m.id, pipeline::Ctx::new(printer())).await;
            eprintln!();
            let done = done?;
            if json {
                println!("{}", serde_json::to_string_pretty(&done)?);
            } else {
                println!("{}", line(&done));
                if let Some(e) = &done.error {
                    eprintln!("{e}");
                }
                println!(
                    "{}",
                    meetings::dir_of(&done.id)?
                        .join(meetings::MARKDOWN_FILE)
                        .display()
                );
            }
        }
        MeetingsCommand::Summarize { id } => {
            let m = pipeline::resummarize(&id, pipeline::Ctx::new(printer())).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&m.summary)?);
            } else {
                print!(
                    "{}",
                    meetings::read_markdown(&id)?
                        .split("\n## Transcript\n")
                        .next()
                        .unwrap_or("")
                );
            }
        }
        MeetingsCommand::Todo { owner, all } => {
            let items = meetings::action_items(all, owner.as_deref());
            if json {
                println!("{}", serde_json::to_string_pretty(&items)?);
            } else if items.is_empty() {
                println!("No open action items.");
            } else {
                for i in items {
                    let mut extra = Vec::new();
                    if let Some(o) = &i.item.owner {
                        extra.push(format!("@{o}"));
                    }
                    if let Some(d) = &i.item.due {
                        extra.push(format!("due {d}"));
                    }
                    println!(
                        "[{}] {}  {}{}  ({} #{})",
                        if i.item.done { "x" } else { " " },
                        i.item.task,
                        extra.join(" "),
                        if extra.is_empty() { "" } else { " " },
                        i.meeting_title,
                        i.index
                    );
                    println!("      omniget meetings done {} {}", i.meeting_id, i.index);
                }
            }
        }
        MeetingsCommand::Done { id, index, undo } => {
            let m = meetings::set_action_done(&id, index, !undo)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&m.summary)?);
            } else {
                println!(
                    "{} #{index} marked {}.",
                    m.title,
                    if undo { "open" } else { "done" }
                );
            }
        }
        MeetingsCommand::Bot { url, title } => {
            let link = vexa::parse_link(&url)
                .ok_or_else(|| anyhow::anyhow!("not a Teams, Google Meet or Zoom meeting link"))?;
            vexa::send_bot(&link, &url, Some(settings::load().language.as_str())).await?;
            let m = meetings::create(
                title.as_deref(),
                MeetingSource::Bot {
                    platform: link.platform.clone(),
                    meeting_url: url,
                    native_id: link.native_id.clone(),
                },
                MeetingStatus::BotLive,
            )?;
            if json {
                println!("{}", serde_json::to_string_pretty(&m)?);
            } else {
                println!(
                    "Bot sent. Meeting {0}: pull it later with `omniget meetings bot-pull {0}`",
                    m.id
                );
            }
        }
        MeetingsCommand::BotPull { id, stop } => {
            let m = meetings::load(&id)?;
            let MeetingSource::Bot {
                platform,
                meeting_url,
                native_id,
            } = &m.source
            else {
                bail!("{id} is not a bot meeting");
            };
            let link = vexa::parse_link(meeting_url).unwrap_or(vexa::MeetingLink {
                platform: platform.clone(),
                native_id: native_id.clone(),
                passcode: None,
            });
            if stop {
                if let Err(e) = vexa::stop_bot(&link).await {
                    eprintln!("stopping the bot: {e:#}");
                }
            }
            let t = vexa::transcript(&link).await?;
            if (t.finished || stop) && m.status == MeetingStatus::BotLive {
                let done = pipeline::ingest_segments(
                    &id,
                    t.segments,
                    "vexa",
                    pipeline::Ctx::new(printer()),
                )
                .await?;
                eprintln!();
                if json {
                    println!("{}", serde_json::to_string_pretty(&done)?);
                } else {
                    println!("{}", line(&done));
                }
            } else if json {
                println!("{}", serde_json::to_string_pretty(&t)?);
            } else {
                println!(
                    "Bot status: {} · {} line(s) so far",
                    t.status,
                    t.segments.len()
                );
                print!("{}", meetings::transcript::to_plain(&t.segments));
            }
        }
        MeetingsCommand::Post { id } => {
            publish::post_to_teams(&meetings::load(&id)?).await?;
            println!("Posted to Teams.");
        }
    }
    Ok(())
}
