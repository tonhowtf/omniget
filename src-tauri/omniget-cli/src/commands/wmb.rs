//! `omniget wmb`: washmybelly from the terminal, through the running app
//! (the local bridge). Scans and plans answer here; `execute` and `restore`
//! only ask: the person approves in the OmniGet window.

use std::time::Duration;

use anyhow::bail;
use serde_json::{json, Value};

#[derive(clap::Subcommand)]
pub enum WmbCommand {
    #[command(about = "Health score and a snapshot of CPU, memory, disk, temperature, battery")]
    Status,
    #[command(about = "What this computer is and which modules exist on it")]
    Caps,
    #[command(
        about = "Scan without changing anything: junk, artifacts, large, duplicates, empty, apps, startup, privacy, updates"
    )]
    Scan {
        kind: String,
        #[arg(help = "Folders to look in (artifacts, large, duplicates, empty)")]
        #[arg(long = "root")]
        roots: Vec<String>,
    },
    #[command(about = "Build a plan from item ids a scan returned")]
    Plan { kind: String, ids: Vec<String> },
    #[command(about = "Show a plan and its status")]
    Show { plan_id: String },
    #[command(about = "Ask to execute a plan (approved in the OmniGet window)")]
    Execute {
        plan_id: String,
        #[arg(help = "Wait for the answer and print the receipt")]
        #[arg(long)]
        wait: bool,
    },
    #[command(about = "Ask to undo a receipt (approved in the OmniGet window)")]
    Restore { receipt_id: String },
    #[command(about = "What earlier plans did")]
    Receipts,
}

fn scan_tool(kind: &str) -> anyhow::Result<&'static str> {
    Ok(match kind {
        "junk" | "cleanup" => "wmb_junk_scan",
        "artifacts" | "dev" => "wmb_dev_artifacts_scan",
        "large" => "wmb_find_large_files",
        "duplicates" | "dupes" => "wmb_find_duplicates",
        "empty" => "wmb_find_empty_folders",
        "apps" => "wmb_apps_list",
        "startup" => "wmb_startup_list",
        "privacy" => "wmb_privacy_catalog",
        "updates" => "wmb_updates_list",
        "disk" => "wmb_disk_usage",
        other => bail!("unknown scan kind {other}"),
    })
}

fn bytes(n: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1000.0 && i < units.len() - 1 {
        v /= 1000.0;
        i += 1;
    }
    format!("{v:.1} {}", units[i])
}

fn print(v: &Value, json_out: bool) {
    if json_out {
        println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
        return;
    }
    if let Some(items) = v["items"].as_array() {
        for it in items.iter().take(200) {
            println!(
                "{:>10}  {:<12} {}  [{}]",
                bytes(it["bytes"].as_u64().unwrap_or(0)),
                it["risk"].as_str().unwrap_or(""),
                it["label"].as_str().or(it["name"].as_str()).unwrap_or(""),
                it["id"].as_str().unwrap_or("")
            );
        }
        if let Some(total) = v["total_bytes"].as_u64() {
            println!("total {}", bytes(total));
        }
        if let Some(plan) = v["plan_id"].as_str() {
            println!("plan {plan}");
        }
        return;
    }
    println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
}

pub async fn execute(cmd: WmbCommand, json_out: bool) -> anyhow::Result<()> {
    let bridge = super::launch::connect().await?;
    let call = |tool: &str, input: Value| {
        let path = format!("/v1/wmb/{tool}");
        let bridge = &bridge;
        async move { bridge.post(&path, input).await }
    };
    let out = match cmd {
        WmbCommand::Status => call("wmb_system_status", json!({})).await?,
        WmbCommand::Caps => call("wmb_capabilities", json!({ "refresh": true })).await?,
        WmbCommand::Scan { kind, roots } => {
            let input = if roots.is_empty() {
                json!({})
            } else {
                json!({ "roots": roots })
            };
            call(scan_tool(&kind)?, input).await?
        }
        WmbCommand::Plan { kind, ids } => {
            call("wmb_plan", json!({ "kind": kind, "item_ids": ids })).await?
        }
        WmbCommand::Show { plan_id } => {
            call("wmb_plan_show", json!({ "plan_id": plan_id })).await?
        }
        WmbCommand::Execute { plan_id, wait } => {
            let asked = call("wmb_execute", json!({ "plan_id": plan_id })).await?;
            if !json_out {
                eprintln!("Waiting for approval in the OmniGet window...");
            }
            if !wait {
                asked
            } else {
                let mut last = asked;
                for _ in 0..300 {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    last = call("wmb_plan_show", json!({ "plan_id": plan_id })).await?;
                    if matches!(
                        last["status"].as_str(),
                        Some("executed" | "denied" | "expired")
                    ) {
                        break;
                    }
                }
                last
            }
        }
        WmbCommand::Restore { receipt_id } => {
            call("wmb_restore", json!({ "receipt_id": receipt_id })).await?
        }
        WmbCommand::Receipts => call("wmb_receipts", json!({})).await?,
    };
    print(&out, json_out);
    Ok(())
}
