//! `omniget company …`: the companies of agents of the running app, through
//! the bridge's `POST /v1/company/rpc` (same methods as the window and the
//! MCP). A company is named by id, issue prefix or name; a person by id,
//! name or role. Owner decisions (approvals, budgets, firing) stay in the
//! window, as in the MCP.

use anyhow::{anyhow, bail};
use clap::Subcommand;
use serde_json::{json, Value};

use super::agent::Bridge;

#[derive(Subcommand, Debug)]
pub enum CompanyCommand {
    #[command(about = "Companies, with status and who is working")]
    List,
    #[command(about = "Built-in company templates")]
    Templates,
    #[command(about = "Create a company from a template (or blank)")]
    Create {
        name: String,
        #[arg(long)]
        template: Option<String>,
        #[arg(long)]
        mission: Option<String>,
        #[arg(help = "Folder the agents work in")]
        #[arg(long)]
        workspace: Option<String>,
    },
    #[command(about = "Overview: mission, org chart, issues, spend, open approvals")]
    Show { company: String },
    #[command(about = "The org chart")]
    Team { company: String },
    #[command(about = "Open issues (all with --all)")]
    Issues {
        company: String,
        #[arg(long)]
        status: Vec<String>,
        #[arg(long)]
        assignee: Option<String>,
        #[arg(long)]
        all: bool,
    },
    #[command(about = "One issue with its thread")]
    Issue { company: String, issue: String },
    #[command(about = "File an issue")]
    New {
        company: String,
        title: String,
        #[arg(long)]
        body: Option<String>,
        #[arg(long)]
        assignee: Option<String>,
        #[arg(long)]
        priority: Option<String>,
        #[arg(long)]
        parent: Option<String>,
    },
    #[command(about = "Comment on an issue (@name wakes that agent)")]
    Comment {
        company: String,
        issue: String,
        body: String,
    },
    #[command(about = "Move an issue (todo, in_progress, in_review, blocked, done, cancelled)")]
    Move {
        company: String,
        issue: String,
        status: String,
        #[arg(long)]
        note: Option<String>,
    },
    #[command(about = "Wake an agent now")]
    Wake {
        company: String,
        who: String,
        #[arg(long)]
        issue: Option<String>,
    },
    #[command(about = "What needs you: approvals, budget incidents, reviews")]
    Inbox { company: String },
    #[command(about = "Spend by agent, project and model")]
    Costs {
        company: String,
        #[arg(long, default_value = "month")]
        window: String,
    },
    #[command(about = "Recent heartbeat runs")]
    Runs {
        company: String,
        #[arg(long, default_value = "20")]
        limit: u32,
    },
    #[command(about = "Activity log (--verify checks the hash chain, --csv prints CSV)")]
    Audit {
        company: String,
        #[arg(long)]
        verify: bool,
        #[arg(long)]
        csv: bool,
    },
    #[command(about = "Print the company as a package (JSON) to move or share it")]
    Export {
        company: String,
        #[arg(long)]
        issues: bool,
    },
    #[command(about = "Import a package file: shows the plan, creates it with --yes")]
    Import {
        file: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        yes: bool,
    },
    #[command(
        about = "Any method of the company API: `omniget company rpc issue.list '{\"company\":\"ACME\"}'`"
    )]
    Rpc {
        method: String,
        args: Option<String>,
    },
}

async fn call(b: &Bridge, method: &str, args: Value) -> anyhow::Result<Value> {
    b.post("/v1/company/rpc", json!({"method": method, "args": args}))
        .await
}

async fn company_id(b: &Bridge, key: &str) -> anyhow::Result<String> {
    let list = call(b, "company.list", json!({})).await?;
    let all = list.as_array().cloned().unwrap_or_default();
    let k = key.to_lowercase();
    all.iter()
        .find(|c| c["id"].as_str() == Some(key))
        .or_else(|| {
            all.iter()
                .find(|c| c["issue_prefix"].as_str().map(str::to_lowercase) == Some(k.clone()))
        })
        .or_else(|| {
            all.iter()
                .find(|c| c["name"].as_str().map(str::to_lowercase) == Some(k.clone()))
        })
        .or_else(|| {
            all.iter().find(|c| {
                c["name"]
                    .as_str()
                    .is_some_and(|n| n.to_lowercase().starts_with(&k))
            })
        })
        .and_then(|c| c["id"].as_str().map(str::to_string))
        .ok_or_else(|| anyhow!("no company `{key}` (see `omniget company list`)"))
}

async fn staff_id(b: &Bridge, company: &str, who: &str) -> anyhow::Result<String> {
    let list = call(b, "staff.list", json!({"company": company})).await?;
    let all = list.as_array().cloned().unwrap_or_default();
    let w = who.to_lowercase();
    all.iter()
        .filter(|s| s["status"] != "terminated")
        .find(|s| {
            s["id"].as_str() == Some(who)
                || s["name"].as_str().map(str::to_lowercase) == Some(w.clone())
        })
        .or_else(|| {
            all.iter()
                .filter(|s| s["status"] != "terminated")
                .find(|s| s["role"].as_str().map(str::to_lowercase) == Some(w.clone()))
        })
        .and_then(|s| s["id"].as_str().map(str::to_string))
        .ok_or_else(|| anyhow!("nobody called `{who}` works there"))
}

fn usd(v: &Value) -> String {
    format!("US$ {:.4}", v.as_f64().unwrap_or(0.0))
}

fn print_chart(nodes: &Value, depth: usize) {
    for n in nodes.as_array().into_iter().flatten() {
        let s = &n["staff"];
        println!(
            "{}{} — {}{} [{}]",
            "  ".repeat(depth),
            s["name"].as_str().unwrap_or(""),
            s["role"].as_str().unwrap_or(""),
            s["title"]
                .as_str()
                .filter(|t| !t.is_empty())
                .map(|t| format!(", {t}"))
                .unwrap_or_default(),
            s["status"].as_str().unwrap_or("")
        );
        print_chart(&n["reports"], depth + 1);
    }
}

fn issue_line(i: &Value) -> String {
    format!(
        "{:<10} {:<12} {:<9} {}",
        i["identifier"].as_str().unwrap_or(""),
        i["status"].as_str().unwrap_or(""),
        i["priority"].as_str().unwrap_or(""),
        i["title"].as_str().unwrap_or("")
    )
}

pub async fn execute(cmd: CompanyCommand, json_out: bool) -> anyhow::Result<()> {
    let b = Bridge::open()?;
    let print = |v: &Value| println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
    match cmd {
        CompanyCommand::List => {
            let v = call(&b, "company.list", json!({})).await?;
            if json_out {
                {
                    print(&v);
                    return Ok(());
                }
            }
            for c in v.as_array().into_iter().flatten() {
                println!(
                    "{:<6} {:<9} {}  — {}",
                    c["issue_prefix"].as_str().unwrap_or(""),
                    c["status"].as_str().unwrap_or(""),
                    c["name"].as_str().unwrap_or(""),
                    c["mission"].as_str().unwrap_or("")
                );
            }
        }
        CompanyCommand::Templates => {
            let v = call(&b, "company.templates", json!({})).await?;
            if json_out {
                {
                    print(&v);
                    return Ok(());
                }
            }
            for t in v.as_array().into_iter().flatten() {
                println!(
                    "{:<16} {} — {}",
                    t["id"].as_str().unwrap_or(""),
                    t["name"].as_str().unwrap_or(""),
                    t["summary"].as_str().unwrap_or("")
                );
            }
        }
        CompanyCommand::Create {
            name,
            template,
            mission,
            workspace,
        } => {
            let v = match template {
                Some(t) => {
                    let options = json!({"name": name, "mission": mission, "workspace": workspace});
                    let plan = call(&b, "import.preview", json!({"template": t, "options": options})).await?;
                    call(&b, "import.apply", json!({"template": t, "options": options, "plan_hash": plan["plan_hash"]})).await?
                }
                None => call(&b, "company.create", json!({"company": {"name": name, "mission": mission.unwrap_or_default(), "workspace": workspace}})).await?,
            };
            if json_out {
                {
                    print(&v);
                    return Ok(());
                }
            }
            println!(
                "created {} ({})",
                v["name"].as_str().unwrap_or(""),
                v["issue_prefix"].as_str().unwrap_or("")
            );
        }
        CompanyCommand::Show { company } => {
            let c = company_id(&b, &company).await?;
            let d = call(&b, "company.dashboard", json!({"company": c})).await?;
            if json_out {
                {
                    print(&d);
                    return Ok(());
                }
            }
            let co = &d["company"];
            println!(
                "{} [{}] — {}",
                co["name"].as_str().unwrap_or(""),
                co["status"].as_str().unwrap_or(""),
                co["mission"].as_str().unwrap_or("")
            );
            println!("issues: {}", d["issue_counts"]);
            println!(
                "running agents: {} · open approvals: {} · spend this month: {}",
                d["running"],
                d["open_approvals"],
                usd(&d["costs"]["cost_usd"])
            );
            let chart = call(&b, "staff.chart", json!({"company": c})).await?;
            print_chart(&chart, 0);
        }
        CompanyCommand::Team { company } => {
            let c = company_id(&b, &company).await?;
            let chart = call(&b, "staff.chart", json!({"company": c})).await?;
            if json_out {
                {
                    print(&chart);
                    return Ok(());
                }
            }
            print_chart(&chart, 0);
        }
        CompanyCommand::Issues {
            company,
            status,
            assignee,
            all,
        } => {
            let c = company_id(&b, &company).await?;
            let assignee = match assignee {
                Some(a) => Some(staff_id(&b, &c, &a).await?),
                None => None,
            };
            let v = call(&b, "issue.list", json!({"company": c, "filter": {"status": status, "assignee_staff_id": assignee, "include_closed": all}})).await?;
            if json_out {
                {
                    print(&v);
                    return Ok(());
                }
            }
            for i in v.as_array().into_iter().flatten() {
                println!("{}", issue_line(i));
            }
        }
        CompanyCommand::Issue { company, issue } => {
            let c = company_id(&b, &company).await?;
            let d = call(&b, "issue.get", json!({"company": c, "issue": issue})).await?;
            if json_out {
                {
                    print(&d);
                    return Ok(());
                }
            }
            println!("{}", issue_line(&d["issue"]));
            if let Some(body) = d["issue"]["body"].as_str().filter(|s| !s.is_empty()) {
                println!("\n{body}\n");
            }
            for cm in d["comments"].as_array().into_iter().flatten() {
                println!(
                    "[{}] {}",
                    cm["author_kind"].as_str().unwrap_or(""),
                    cm["body"].as_str().unwrap_or("")
                );
            }
        }
        CompanyCommand::New {
            company,
            title,
            body,
            assignee,
            priority,
            parent,
        } => {
            let c = company_id(&b, &company).await?;
            let (a, me) = match assignee.as_deref() {
                Some("me") | Some("owner") => (None, true),
                Some(x) => (Some(staff_id(&b, &c, x).await?), false),
                None => (None, false),
            };
            let v = call(&b, "issue.create", json!({"company": c, "issue": {"title": title, "body": body.unwrap_or_default(),
                "assignee_staff_id": a, "assignee_user": me, "priority": priority, "parent_id": parent}})).await?;
            if json_out {
                {
                    print(&v);
                    return Ok(());
                }
            }
            println!("{}", issue_line(&v));
        }
        CompanyCommand::Comment {
            company,
            issue,
            body,
        } => {
            let c = company_id(&b, &company).await?;
            let v = call(
                &b,
                "issue.comment",
                json!({"company": c, "issue": issue, "body": body}),
            )
            .await?;
            if json_out {
                {
                    print(&v);
                    return Ok(());
                }
            }
            println!("commented");
        }
        CompanyCommand::Move {
            company,
            issue,
            status,
            note,
        } => {
            let c = company_id(&b, &company).await?;
            let v = call(
                &b,
                "issue.status",
                json!({"company": c, "issue": issue, "status": status, "note": note}),
            )
            .await?;
            if json_out {
                {
                    print(&v);
                    return Ok(());
                }
            }
            println!("{}", issue_line(&v));
        }
        CompanyCommand::Wake {
            company,
            who,
            issue,
        } => {
            let c = company_id(&b, &company).await?;
            let s = staff_id(&b, &c, &who).await?;
            call(
                &b,
                "staff.wake",
                json!({"company": c, "staff": s, "issue": issue, "reason": "woken from the CLI"}),
            )
            .await?;
            println!("woken");
        }
        CompanyCommand::Inbox { company } => {
            let c = company_id(&b, &company).await?;
            let v = call(&b, "inbox.list", json!({"company": c})).await?;
            if json_out {
                {
                    print(&v);
                    return Ok(());
                }
            }
            for a in v["approvals"].as_array().into_iter().flatten() {
                println!(
                    "approval  {:<16} {}",
                    a["kind"].as_str().unwrap_or(""),
                    a["title"].as_str().unwrap_or("")
                );
            }
            for i in v["incidents"].as_array().into_iter().flatten() {
                println!(
                    "budget    {:<16} {} of {}",
                    i["kind"].as_str().unwrap_or(""),
                    i["observed"],
                    i["limit_value"]
                );
            }
            for i in v["issues"].as_array().into_iter().flatten() {
                println!(
                    "{:<9} {}",
                    i["why"].as_str().unwrap_or(""),
                    issue_line(&i["issue"])
                );
            }
            println!("(decide approvals and budgets in the OmniGet window)");
        }
        CompanyCommand::Costs { company, window } => {
            let c = company_id(&b, &company).await?;
            let v = call(&b, "cost.summary", json!({"company": c, "window": window})).await?;
            if json_out {
                {
                    print(&v);
                    return Ok(());
                }
            }
            println!(
                "{}: {} · {} tokens · {} runs · {} unpriced requests",
                v["window_key"].as_str().unwrap_or(""),
                usd(&v["cost_usd"]),
                v["tokens"],
                v["runs"],
                v["unpriced_events"]
            );
            for r in v["by_agent"].as_array().into_iter().flatten() {
                println!(
                    "  {:<20} {:>14} {:>10} tokens",
                    r["name"].as_str().unwrap_or("-"),
                    usd(&r["cost_usd"]),
                    r["tokens"]
                );
            }
        }
        CompanyCommand::Runs { company, limit } => {
            let c = company_id(&b, &company).await?;
            let v = call(&b, "run.list", json!({"company": c, "limit": limit})).await?;
            if json_out {
                {
                    print(&v);
                    return Ok(());
                }
            }
            for r in v.as_array().into_iter().flatten() {
                println!(
                    "{:<10} {:<13} {:<12} {:>10} µ$ {}",
                    &r["id"].as_str().unwrap_or("")[..8.min(r["id"].as_str().unwrap_or("").len())],
                    r["state"].as_str().unwrap_or(""),
                    r["source"].as_str().unwrap_or(""),
                    r["cost_micros"],
                    r["liveness"].as_str().unwrap_or("")
                );
            }
        }
        CompanyCommand::Audit {
            company,
            verify,
            csv,
        } => {
            let c = company_id(&b, &company).await?;
            if verify {
                let v = call(&b, "audit.verify", json!({"company": c})).await?;
                println!("intact: {} lines", v["lines"]);
                return Ok(());
            }
            if csv {
                let v = call(&b, "audit.csv", json!({"company": c})).await?;
                print!("{}", v["csv"].as_str().unwrap_or(""));
                return Ok(());
            }
            let v = call(&b, "audit.list", json!({"company": c})).await?;
            if json_out {
                {
                    print(&v);
                    return Ok(());
                }
            }
            for a in v.as_array().into_iter().flatten() {
                println!(
                    "{:>6} {:<7} {:<22} {:<8} {}",
                    a["seq"],
                    a["actor_kind"].as_str().unwrap_or(""),
                    a["action"].as_str().unwrap_or(""),
                    a["entity_kind"].as_str().unwrap_or(""),
                    a["channel"].as_str().unwrap_or("")
                );
            }
        }
        CompanyCommand::Export { company, issues } => {
            let c = company_id(&b, &company).await?;
            let v = call(
                &b,
                "company.export",
                json!({"company": c, "include_issues": issues}),
            )
            .await?;
            print(&v);
        }
        CompanyCommand::Import { file, name, yes } => {
            let raw = std::fs::read_to_string(&file)?;
            let pkg: Value = serde_json::from_str(&raw)?;
            let options = json!({"name": name});
            let plan = call(
                &b,
                "import.preview",
                json!({"package": pkg, "options": options}),
            )
            .await?;
            print(&plan);
            if !yes {
                println!("(nothing created; run again with --yes to create it)");
                return Ok(());
            }
            let v = call(
                &b,
                "import.apply",
                json!({"package": pkg, "options": options, "plan_hash": plan["plan_hash"]}),
            )
            .await?;
            println!(
                "created {} ({})",
                v["name"].as_str().unwrap_or(""),
                v["issue_prefix"].as_str().unwrap_or("")
            );
        }
        CompanyCommand::Rpc { method, args } => {
            let args: Value = match args {
                Some(a) => {
                    serde_json::from_str(&a).map_err(|e| anyhow!("args must be JSON: {e}"))?
                }
                None => json!({}),
            };
            if !args.is_object() {
                bail!("args must be a JSON object");
            }
            print(&call(&b, &method, args).await?);
        }
    }
    Ok(())
}
