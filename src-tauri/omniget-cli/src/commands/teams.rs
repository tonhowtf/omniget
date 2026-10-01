//! `omniget teams`: Microsoft Teams through Microsoft Graph, as the person.
//! Chats and channels come out as Markdown an LLM reads (or JSON with
//! `--json`), the calendar lists online meetings with their join links, and
//! `transcript` turns a meeting's official transcript into an OmniGet
//! meeting. The sign-in is shared with the desktop app.

use std::path::PathBuf;

use anyhow::bail;
use omniget_core::core::msgraph::{self, calendar, chats, transcripts};

#[derive(clap::Subcommand)]
pub enum TeamsCommand {
    #[command(about = "Sign-in state and the app registration in use")]
    Status,
    #[command(about = "Set the Entra app registration (Application (client) ID) to sign in with")]
    Setup {
        #[arg(long)]
        client_id: String,
        #[arg(help = "\"organizations\" (default), \"common\", or your tenant id / domain")]
        #[arg(long, default_value = "organizations")]
        tenant: String,
        #[arg(
            help = "Also ask for channel messages and meeting transcripts (an admin approves them)"
        )]
        #[arg(long)]
        admin_scopes: bool,
    },
    #[command(about = "Sign in with a device code (open the link, type the code)")]
    Login,
    #[command(about = "Forget the Microsoft 365 session")]
    Logout,
    #[command(about = "List chats, most recent first")]
    Chats {
        #[arg(help = "Filter by chat name or member")]
        #[arg(short, long)]
        query: Option<String>,
        #[arg(long, default_value = "30")]
        limit: usize,
    },
    #[command(about = "Print one chat as Markdown (what an LLM should read)")]
    Read {
        chat_id: String,
        #[arg(help = "How far back: 30m, 12h, 7d, 2w or a date (default: everything)")]
        #[arg(long)]
        since: Option<String>,
        #[arg(help = "Newest N messages (0 = all)")]
        #[arg(long, default_value = "500")]
        limit: usize,
    },
    #[command(about = "Export chats to files (Markdown, or JSON with --format json)")]
    Export {
        #[arg(help = "Chat ids (see `omniget teams chats`); none with --all exports every chat")]
        chat_ids: Vec<String>,
        #[arg(long)]
        all: bool,
        #[arg(long)]
        since: Option<String>,
        #[arg(long, default_value = "md", value_parser = ["md", "json"])]
        format: String,
        #[arg(help = "Output folder (default: <OmniGet data>/teams-export)")]
        #[arg(short, long)]
        output: Option<String>,
    },
    #[command(about = "List the channels of the teams you joined")]
    Channels,
    #[command(about = "Print a channel's posts and replies as Markdown (needs --admin-scopes)")]
    Channel {
        team_id: String,
        channel_id: String,
        #[arg(long)]
        since: Option<String>,
        #[arg(long, default_value = "200")]
        limit: usize,
    },
    #[command(about = "Online meetings on your calendar, with their join links")]
    Calendar {
        #[arg(long, default_value = "7", help = "Days ahead")]
        days: i64,
        #[arg(long, default_value = "1", help = "Days back")]
        past: i64,
    },
    #[command(
        about = "Import a meeting's official Teams transcript into Meetings (needs --admin-scopes)"
    )]
    Transcript {
        #[arg(help = "The meeting's join link (from `omniget teams calendar`)")]
        join_url: String,
        #[arg(long)]
        title: Option<String>,
    },
}

fn since(s: &Option<String>) -> anyhow::Result<Option<chrono::DateTime<chrono::Utc>>> {
    s.as_deref().map(msgraph::parse_since).transpose()
}

fn local(iso: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(iso)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|_| iso.to_string())
}

fn print<T: serde::Serialize>(v: &T) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}

pub async fn execute(cmd: TeamsCommand, json: bool) -> anyhow::Result<()> {
    match cmd {
        TeamsCommand::Status => {
            let s = msgraph::status();
            if json {
                return print(&s);
            }
            if !s.configured {
                println!("Not set up. Register an app in Entra ID (public client flows on) and run:\n  omniget teams setup --client-id <Application (client) ID>");
            } else if let Some(a) = &s.account {
                println!(
                    "Signed in as {} <{}> (tenant {}, admin scopes {})",
                    a.name, a.upn, s.tenant, s.admin_scopes
                );
            } else {
                println!(
                    "Set up with client {} but not signed in: run `omniget teams login`",
                    s.client_id
                );
            }
        }
        TeamsCommand::Setup {
            client_id,
            tenant,
            admin_scopes,
        } => {
            let c = msgraph::configure(&client_id, &tenant, admin_scopes)?;
            if json {
                return print(&c);
            }
            println!("Saved. Now run `omniget teams login`.");
        }
        TeamsCommand::Login => {
            let code = msgraph::start_sign_in().await?;
            // The instructions go to stderr so `--json` stays parseable.
            eprintln!("{}", code.message);
            let account = msgraph::finish_sign_in(&code).await?;
            if json {
                return print(&account);
            }
            println!("Signed in as {} <{}>", account.name, account.upn);
        }
        TeamsCommand::Logout => {
            msgraph::sign_out()?;
            println!("Signed out.");
        }
        TeamsCommand::Chats { query, limit } => {
            let list = chats::list_chats(limit, query.as_deref()).await?;
            if json {
                return print(&list);
            }
            for c in list {
                println!(
                    "{}  {:<9} {}  {}",
                    c.last_updated.as_deref().map(local).unwrap_or_default(),
                    c.kind,
                    c.id,
                    c.name
                );
            }
        }
        TeamsCommand::Read {
            chat_id,
            since: s,
            limit,
        } => {
            let info = chats::chat(&chat_id).await?;
            let msgs = chats::chat_messages(&chat_id, since(&s)?, limit).await?;
            if json {
                return print(&serde_json::json!({ "chat": info, "messages": msgs }));
            }
            print!(
                "{}",
                chats::to_markdown(&info.name, &info.kind, &info.members, &msgs)
            );
        }
        TeamsCommand::Export {
            chat_ids,
            all,
            since: s,
            format,
            output,
        } => {
            let ids: Vec<String> = if all {
                chats::list_chats(10_000, None)
                    .await?
                    .into_iter()
                    .map(|c| c.id)
                    .collect()
            } else if chat_ids.is_empty() {
                bail!("name chat ids (see `omniget teams chats`) or pass --all");
            } else {
                chat_ids
            };
            let dir = output
                .map(PathBuf::from)
                .map(Ok)
                .unwrap_or_else(chats::export_dir)?;
            let s = since(&s)?;
            let mut done = Vec::new();
            for id in &ids {
                match chats::export_chat(id, s, format == "json", &dir).await {
                    Ok(e) => {
                        if !json {
                            println!("{:>5} messages  {}", e.messages, e.path);
                        }
                        done.push(e);
                    }
                    Err(e) => eprintln!("{id}: {e:#}"),
                }
            }
            if json {
                return print(&done);
            }
            eprintln!("{} chat(s) exported to {}", done.len(), dir.display());
        }
        TeamsCommand::Channels => {
            let list = chats::list_channels().await?;
            if json {
                return print(&list);
            }
            for c in list {
                println!("{} / {}  {} {}", c.team, c.channel, c.team_id, c.channel_id);
            }
        }
        TeamsCommand::Channel {
            team_id,
            channel_id,
            since: s,
            limit,
        } => {
            let msgs = chats::channel_messages(&team_id, &channel_id, since(&s)?, limit).await?;
            if json {
                return print(&msgs);
            }
            print!(
                "{}",
                chats::to_markdown(&format!("Channel {channel_id}"), "channel", &[], &msgs)
            );
        }
        TeamsCommand::Calendar { days, past } => {
            let list = calendar::meetings(past, days, true).await?;
            if json {
                return print(&list);
            }
            for m in list {
                println!(
                    "{}  {}  {}\n    {}",
                    local(&m.start),
                    m.subject,
                    if m.organizer.is_empty() {
                        String::new()
                    } else {
                        format!("({})", m.organizer)
                    },
                    m.join_url.unwrap_or_default()
                );
            }
        }
        TeamsCommand::Transcript { join_url, title } => {
            let progress = std::sync::Arc::new(|p: omniget_core::core::tools::ToolProgress| {
                if p.stage == "status" {
                    eprintln!("… {}", p.message.unwrap_or_default());
                }
            });
            let m = transcripts::import(&join_url, title.as_deref(), progress).await?;
            if json {
                return print(&m);
            }
            println!("Meeting {}  {}", m.id, m.title);
            if let Some(e) = &m.error {
                eprintln!("{e}");
            }
            println!("Read it with: omniget meetings show {}", m.id);
        }
    }
    Ok(())
}
