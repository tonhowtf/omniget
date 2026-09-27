//! `omniget agentkit search|show|install|uninstall|list|doctor|import|export|profiles`:
//! the Catalog from the terminal. Talks to the running app through the local
//! bridge (`POST /v1/agentkit/<action>`, bearer from `settings.json`); the app
//! owns the plan, the backups and the lockfiles, so the CLI and the UI see the
//! same installs. `statusline-shim` runs locally (no app needed).
//!
//! Every install shows its plan first (tools, files, what is lost in the
//! conversion, commands that will run). `--dry-run` stops there and prints the
//! diffs; `--yes` applies without asking (required when stdin is not a terminal).

use std::collections::BTreeMap;
use std::io::{IsTerminal, Read, Write};
use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use serde_json::{json, Value};

struct Bridge {
    base: String,
    token: String,
    http: reqwest::Client,
}

impl Bridge {
    fn open() -> anyhow::Result<Self> {
        let dir =
            omniget_core::core::paths::app_data_dir().ok_or_else(|| anyhow!("no app data dir"))?;
        let raw = std::fs::read_to_string(dir.join("settings.json"))
            .context("settings.json not found: open the OmniGet app once")?;
        let json: Value = serde_json::from_str(&raw)?;
        let bridge = &json["app_settings"]["bridge"];
        let port = bridge["port"]
            .as_u64()
            .filter(|p| *p > 0)
            .ok_or_else(|| anyhow!("the app has no bridge port yet: open OmniGet once"))?;
        Ok(Self {
            base: format!("http://127.0.0.1:{port}"),
            token: bridge["token"].as_str().unwrap_or("").to_string(),
            // catalog downloads can take a while on the first install
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(300))
                .build()?,
        })
    }

    async fn call(&self, action: &str, body: Value) -> anyhow::Result<Value> {
        let res = self
            .http
            .post(format!("{}/v1/agentkit/{action}", self.base))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .context("the OmniGet app is not running (the bridge did not answer)")?;
        let status = res.status();
        let body: Value = res.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            bail!("{}", body["error"].as_str().unwrap_or(status.as_str()));
        }
        Ok(body)
    }
}

/// Options shared by everything that installs.
#[derive(clap::Args, Clone, Default)]
pub struct InstallOpts {
    /// Tools to install into (comma list). Default: the installed ones.
    #[arg(long, short = 't', value_delimiter = ',')]
    pub target: Vec<String>,
    /// project | global | local | managed (default: project with --project, else global)
    #[arg(long)]
    pub scope: Option<String>,
    /// Project folder (default: none = global install)
    #[arg(long)]
    pub project: Option<String>,
    /// Apply without asking (needed when stdin is not a terminal)
    #[arg(long, short = 'y')]
    pub yes: bool,
    /// Show the plan and the diffs, write nothing
    #[arg(long)]
    pub dry_run: bool,
    /// When a file or entry with the same name exists: rename | skip | overwrite
    #[arg(long, default_value = "rename")]
    pub on_conflict: String,
}

#[derive(clap::Subcommand)]
pub enum ProfilesCommand {
    /// List the profiles and what they set
    List,
    /// Apply a profile to several tools at once
    Apply {
        id: String,
        /// Profile parameter `key=value` (provider: protocol, base_url, key_env, model)
        #[arg(long = "param", short = 'p')]
        params: Vec<String>,
        #[command(flatten)]
        opts: InstallOpts,
    },
}

#[derive(clap::Subcommand)]
pub enum AgentkitCommand {
    /// Search the catalog
    Search {
        query: Vec<String>,
        /// Only these kinds (agent, command, skill, mcp, hook, setting, statusline, loop, plugin…)
        #[arg(long, short = 'k', value_delimiter = ',')]
        kind: Vec<String>,
        #[arg(long, default_value = "20")]
        limit: u64,
    },
    /// Show a catalog item and how it fits each tool
    Show {
        id: String,
        #[arg(long)]
        project: Option<String>,
    },
    /// Install catalog items (ids like cct:agents/development-team/frontend-developer,
    /// omniget:profiles/read-only or path:<kind>:<file>)
    Install {
        #[arg(required = true)]
        ids: Vec<String>,
        #[command(flatten)]
        opts: InstallOpts,
        /// Secret value for tools that cannot read an env var: NAME=value
        #[arg(long = "secret")]
        secrets: Vec<String>,
    },
    /// Remove an install (install id, or component id/name for every tool)
    Uninstall {
        id: String,
        #[arg(long, short = 't')]
        target: Option<String>,
        #[arg(long)]
        project: Option<String>,
        /// Remove even pieces edited since we wrote them
        #[arg(long)]
        force: bool,
        #[arg(long, short = 'y')]
        yes: bool,
        #[arg(long)]
        dry_run: bool,
    },
    /// What OmniGet installed (global + the project's lockfile)
    List {
        #[arg(long)]
        project: Option<String>,
    },
    /// Detected tools, installs and files edited since install
    Doctor {
        #[arg(long)]
        project: Option<String>,
    },
    /// Install a .omnistack file, or copy what one tool has into others (--from)
    Import {
        file: Option<String>,
        /// Tool whose installed agents/commands/skills/hooks/MCP servers are copied
        #[arg(long)]
        from: Option<String>,
        #[command(flatten)]
        opts: InstallOpts,
    },
    /// Write the installed catalog items as a .omnistack file
    Export {
        #[arg(long)]
        project: Option<String>,
        #[arg(long, short = 'o')]
        out: Option<String>,
        #[arg(long)]
        name: Option<String>,
    },
    /// OmniGet permission/provider profiles for every tool at once
    Profiles {
        #[command(subcommand)]
        command: Option<ProfilesCommand>,
    },
    /// Statusline stdin adapter (run by the tools, not by hand)
    #[command(hide = true)]
    StatuslineShim {
        #[arg(long)]
        tool: String,
        #[arg(last = true, required = true)]
        command: Vec<String>,
    },
}

fn cwd_project(p: Option<String>) -> Option<String> {
    p.map(|p| {
        let pb = std::path::PathBuf::from(&p);
        let abs = if pb.is_absolute() {
            pb
        } else {
            std::env::current_dir()
                .map(|d| d.join(pb))
                .unwrap_or_else(|_| p.clone().into())
        };
        abs.canonicalize()
            .unwrap_or(abs)
            .to_string_lossy()
            .to_string()
    })
}

fn plan_body(opts: &InstallOpts) -> Value {
    json!({
        "targets": opts.target,
        "scope": opts.scope,
        "project": cwd_project(opts.project.clone()),
        "policy": opts.on_conflict,
    })
}

fn short(s: &str, n: usize) -> String {
    let line = s.lines().next().unwrap_or("");
    if line.chars().count() > n {
        format!("{}…", line.chars().take(n - 1).collect::<String>())
    } else {
        line.to_string()
    }
}

fn print_plan(plan: &Value, diffs: bool) {
    let mut by_status: BTreeMap<String, usize> = BTreeMap::new();
    for u in plan["units"].as_array().into_iter().flatten() {
        let status = u["status"].as_str().unwrap_or("");
        *by_status.entry(status.to_string()).or_default() += 1;
        let compat = u["compat"]["status"].as_str().unwrap_or("");
        println!(
            "  {:<11} {:<10} {:<34} {}",
            status,
            u["target"].as_str().unwrap_or(""),
            short(u["install_name"].as_str().unwrap_or(""), 34),
            compat
        );
        for l in u["losses"].as_array().into_iter().flatten() {
            println!("      lost: {}", l.as_str().unwrap_or(""));
        }
        if let Some(e) = u["error"].as_str() {
            println!("      error: {e}");
        }
        if status == "unsupported" {
            if let Some(r) = u["compat"]["reason"].as_str() {
                println!("      {r}");
            }
        }
        for n in u["notes"].as_array().into_iter().flatten() {
            println!("      note: {}", n.as_str().unwrap_or(""));
        }
    }
    let files = plan["files"].as_array().cloned().unwrap_or_default();
    if !files.is_empty() {
        println!("files:");
    }
    for f in &files {
        println!(
            "  {:<9} {}",
            f["action"].as_str().unwrap_or(""),
            f["path"].as_str().unwrap_or("")
        );
        for c in f["conflicts"].as_array().into_iter().flatten() {
            println!("      conflict: {}", c.as_str().unwrap_or(""));
        }
        if diffs {
            if let Some(d) = f["diff"].as_str().filter(|d| !d.is_empty()) {
                for line in d.lines() {
                    println!("      {line}");
                }
            }
        }
    }
    let mut cmds: Vec<String> = Vec::new();
    for f in &files {
        for c in f["commands"].as_array().into_iter().flatten() {
            if let Some(c) = c.as_str() {
                if !cmds.iter().any(|x| x == c) {
                    cmds.push(c.to_string());
                }
            }
        }
    }
    if !cmds.is_empty() {
        println!("commands the tools will run:");
        for c in cmds {
            println!("  {c}");
        }
    }
    for w in plan["warnings"].as_array().into_iter().flatten() {
        println!("warning: {}", w.as_str().unwrap_or(""));
    }
    let summary: Vec<String> = by_status.iter().map(|(k, v)| format!("{v} {k}")).collect();
    println!(
        "plan {}: {}",
        plan["id"].as_str().unwrap_or(""),
        summary.join(", ")
    );
}

fn has_changes(plan: &Value) -> bool {
    plan["files"]
        .as_array()
        .map(|f| f.iter().any(|x| x["action"].as_str() != Some("unchanged")))
        .unwrap_or(false)
}

/// `--yes`, or a `y` typed on a terminal. Never waits on a pipe.
fn confirm(yes: bool, question: &str) -> anyhow::Result<bool> {
    if yes {
        return Ok(true);
    }
    if !std::io::stdin().is_terminal() {
        bail!("stdin is not a terminal: pass --yes to apply, or --dry-run to only see the plan");
    }
    eprint!("{question} [y/N] ");
    std::io::stderr().flush().ok();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(matches!(line.trim(), "y" | "Y" | "yes" | "s" | "sim"))
}

async fn plan_and_apply(
    bridge: &Bridge,
    action: &str,
    body: Value,
    opts: &InstallOpts,
    json_out: bool,
) -> anyhow::Result<()> {
    let plan = bridge.call(action, body).await?;
    if json_out && (opts.dry_run || !opts.yes) {
        println!("{}", serde_json::to_string_pretty(&plan)?);
    } else {
        print_plan(&plan, opts.dry_run);
    }
    if opts.dry_run {
        return Ok(());
    }
    if !has_changes(&plan) {
        if !json_out {
            println!("nothing to write");
        }
        return Ok(());
    }
    if !confirm(opts.yes, "apply this plan?")? {
        println!("cancelled; nothing written");
        return Ok(());
    }
    let report = bridge
        .call("apply", json!({ "plan_id": plan["id"] }))
        .await?;
    if json_out {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "applied (transaction {}): {} installs, {} files written; undo with the app (Central → Installed) or `omniget agentkit uninstall <install id>`",
            report["tx"].as_str().unwrap_or(""),
            report["installed"].as_array().map(|a| a.len()).unwrap_or(0),
            report["files_written"]
                .as_array()
                .map(|a| a.len())
                .or_else(|| report["files_written"].as_u64().map(|n| n as usize))
                .unwrap_or(0),
        );
        for r in report["installed"].as_array().into_iter().flatten() {
            println!(
                "  {}  {} → {}",
                r["install_id"].as_str().unwrap_or(""),
                r["component"]["id"].as_str().unwrap_or(""),
                r["target"].as_str().unwrap_or("")
            );
        }
    }
    Ok(())
}

fn print_json(v: &Value) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}

pub async fn execute(cmd: AgentkitCommand, json_out: bool) -> anyhow::Result<()> {
    if let AgentkitCommand::StatuslineShim { tool, command } = &cmd {
        let mut stdin = Vec::new();
        std::io::stdin().read_to_end(&mut stdin).ok();
        let line = if command.len() == 1 {
            command[0].clone()
        } else {
            command.join(" ")
        };
        let (code, out) = omniget_core::core::agentkit::convert::statusline_stdin::statusline_main(
            tool, &line, &stdin,
        );
        print!("{out}");
        std::io::stdout().flush().ok();
        std::process::exit(code);
    }
    let bridge = Bridge::open()?;
    match cmd {
        AgentkitCommand::StatuslineShim { .. } => unreachable!(),
        AgentkitCommand::Search { query, kind, limit } => {
            let r = bridge
                .call(
                    "search",
                    json!({ "query": query.join(" "), "kinds": kind, "limit": limit }),
                )
                .await?;
            if json_out {
                return print_json(&r);
            }
            for h in r["items"].as_array().into_iter().flatten() {
                println!(
                    "{:<58} {:<10} {}",
                    h["id"].as_str().unwrap_or(""),
                    h["kind"].as_str().unwrap_or(""),
                    short(h["description"].as_str().unwrap_or(""), 70)
                );
            }
            println!("{} results", r["total"]);
            Ok(())
        }
        AgentkitCommand::Show { id, project } => {
            let r = bridge
                .call("show", json!({ "id": id, "project": cwd_project(project) }))
                .await?;
            if json_out {
                return print_json(&r);
            }
            let item = &r["item"];
            let c = &r["compat"];
            println!(
                "{}  ({})",
                c["name"].as_str().unwrap_or(""),
                c["kind"].as_str().unwrap_or("")
            );
            println!("{}", c["id"].as_str().unwrap_or(""));
            if let Some(d) = c["description"].as_str() {
                println!("{d}");
            }
            if item.is_object() {
                println!(
                    "source: {}  license: {}  author: {}",
                    item["source"]["repo"]
                        .as_str()
                        .unwrap_or(item["source"]["id"].as_str().unwrap_or("")),
                    item["license"].as_str().unwrap_or("unknown"),
                    item["author"].as_str().unwrap_or("-")
                );
            }
            println!("tools:");
            for t in c["targets"].as_array().into_iter().flatten() {
                let cp = &t["compat"];
                let status = cp["status"].as_str().unwrap_or("");
                let detail = match status {
                    "degraded" => cp["lost"]
                        .as_array()
                        .map(|l| {
                            l.iter()
                                .filter_map(|x| x.as_str())
                                .collect::<Vec<_>>()
                                .join("; ")
                        })
                        .unwrap_or_default(),
                    "unsupported" => cp["reason"].as_str().unwrap_or("").to_string(),
                    _ => String::new(),
                };
                println!(
                    "  {:<10} {:<9} {:<12} {}",
                    t["id"].as_str().unwrap_or(""),
                    if t["installed"].as_bool() == Some(true) {
                        "installed"
                    } else {
                        ""
                    },
                    status,
                    short(&detail, 90)
                );
            }
            Ok(())
        }
        AgentkitCommand::Install { ids, opts, secrets } => {
            let mut body = plan_body(&opts);
            body["ids"] = json!(ids);
            let mut sv = serde_json::Map::new();
            for s in secrets {
                let (k, v) = s
                    .split_once('=')
                    .ok_or_else(|| anyhow!("--secret takes NAME=value"))?;
                sv.insert(k.trim().to_string(), json!(v));
            }
            body["secret_values"] = Value::Object(sv);
            plan_and_apply(&bridge, "plan", body, &opts, json_out).await
        }
        AgentkitCommand::Uninstall {
            id,
            target,
            project,
            force,
            yes,
            dry_run,
        } => {
            let project = cwd_project(project);
            let recs = bridge
                .call("installed", json!({ "project": project }))
                .await?;
            let hits: Vec<&Value> = recs
                .as_array()
                .into_iter()
                .flatten()
                .filter(|r| {
                    r["install_id"].as_str() == Some(&id)
                        || ((r["component"]["id"].as_str() == Some(&id)
                            || r["component"]["name"].as_str() == Some(&id))
                            && target
                                .as_deref()
                                .map(|t| r["target"].as_str() == Some(t))
                                .unwrap_or(true))
                })
                .collect();
            if hits.is_empty() {
                bail!("`{id}` is not installed (see `omniget agentkit list`)");
            }
            for r in &hits {
                println!(
                    "remove {}  {} from {}",
                    r["install_id"].as_str().unwrap_or(""),
                    r["component"]["id"].as_str().unwrap_or(""),
                    r["target"].as_str().unwrap_or("")
                );
                for f in r["files"].as_array().into_iter().flatten() {
                    println!("  {}", f["path"].as_str().unwrap_or(""));
                }
            }
            if dry_run || !confirm(yes, "remove these?")? {
                return Ok(());
            }
            let r = bridge
                .call(
                    "uninstall",
                    json!({ "id": id, "target": target, "project": project, "force": force }),
                )
                .await?;
            if json_out {
                return print_json(&r);
            }
            for rep in r.as_array().into_iter().flatten() {
                let kept = rep["kept"].as_array().map(|a| a.len()).unwrap_or(0);
                println!(
                    "removed (transaction {}){}",
                    rep["tx"].as_str().unwrap_or(""),
                    if kept > 0 {
                        format!("; {kept} edited pieces kept (use --force)")
                    } else {
                        String::new()
                    }
                );
            }
            Ok(())
        }
        AgentkitCommand::List { project } => {
            let r = bridge
                .call("installed", json!({ "project": cwd_project(project) }))
                .await?;
            if json_out {
                return print_json(&r);
            }
            for rec in r.as_array().into_iter().flatten() {
                println!(
                    "{:<14} {:<10} {:<8} {:<50} {}",
                    rec["install_id"].as_str().unwrap_or(""),
                    rec["target"].as_str().unwrap_or(""),
                    rec["scope"].as_str().unwrap_or(""),
                    rec["component"]["id"].as_str().unwrap_or(""),
                    rec["compat"]["status"].as_str().unwrap_or("")
                );
            }
            Ok(())
        }
        AgentkitCommand::Doctor { project } => {
            let r = bridge
                .call("doctor", json!({ "project": cwd_project(project) }))
                .await?;
            if json_out {
                return print_json(&r);
            }
            println!("tools:");
            for t in r["targets"].as_array().into_iter().flatten() {
                if t["installed"].as_bool() != Some(true) {
                    continue;
                }
                println!(
                    "  {:<10} {:<14} {}{}",
                    t["id"].as_str().unwrap_or(""),
                    t["version"].as_str().unwrap_or("-"),
                    t["binaries"][0]["path"].as_str().unwrap_or(""),
                    if t["beta"].as_bool() == Some(true) {
                        "  (beta)"
                    } else {
                        ""
                    }
                );
            }
            println!("installs: {}", r["installed"]);
            let drift = r["drift"].as_array().cloned().unwrap_or_default();
            if drift.is_empty() {
                println!("no drift: every file OmniGet wrote is as it left it");
            } else {
                println!("changed since install:");
                for d in drift {
                    println!(
                        "  {:<13} {:<10} {}  {}",
                        d["state"].as_str().unwrap_or(""),
                        d["target"].as_str().unwrap_or(""),
                        d["path"].as_str().unwrap_or(""),
                        d["detail"].as_str().unwrap_or("")
                    );
                }
            }
            if let Some(c) = r["catalog"].as_object() {
                println!(
                    "catalog: {} items, generated {}",
                    c.get("total").cloned().unwrap_or(Value::Null),
                    c.get("generated").and_then(|g| g.as_str()).unwrap_or("?")
                );
            }
            Ok(())
        }
        AgentkitCommand::Import { file, from, opts } => {
            let mut body = plan_body(&opts);
            match (file, from) {
                (Some(f), None) => {
                    let bytes = std::fs::read(&f).with_context(|| format!("reading {f}"))?;
                    let stack = omniget_core::core::catalog::collections::parse_stack(&bytes)
                        .map_err(|e| anyhow!("{e}"))?;
                    let ids: Vec<String> = stack.items.iter().map(|i| i.id.clone()).collect();
                    if opts.target.is_empty() && !stack.targets.is_empty() {
                        body["targets"] = json!(stack.targets);
                    }
                    if opts.scope.is_none() {
                        if let Some(s) = &stack.scope {
                            body["scope"] = json!(s);
                        }
                    }
                    println!("{}: {} items", stack.name, ids.len());
                    body["ids"] = json!(ids);
                }
                (None, Some(tool)) => {
                    let comps = bridge
                        .call(
                            "import_installed",
                            json!({ "from": tool, "scope": opts.scope, "project": cwd_project(opts.project.clone()) }),
                        )
                        .await?;
                    let n = comps.as_array().map(|a| a.len()).unwrap_or(0);
                    if n == 0 {
                        bail!("{tool} has nothing installed that OmniGet can copy");
                    }
                    println!("{tool}: {n} components");
                    if opts.target.is_empty() {
                        bail!("pass --target with the tools to copy into");
                    }
                    body["components"] = comps;
                }
                _ => bail!("give a .omnistack file or --from <tool>"),
            }
            plan_and_apply(&bridge, "plan", body, &opts, json_out).await
        }
        AgentkitCommand::Export { project, out, name } => {
            let stack = bridge
                .call(
                    "export",
                    json!({ "project": cwd_project(project), "name": name }),
                )
                .await?;
            let text = serde_json::to_string_pretty(&stack)?;
            match out {
                Some(p) => {
                    let mut p = std::path::PathBuf::from(p);
                    if p.extension().and_then(|e| e.to_str()) != Some("omnistack") {
                        p.set_extension("omnistack");
                    }
                    std::fs::write(&p, text + "\n")?;
                    println!(
                        "{} ({} items)",
                        p.display(),
                        stack["items"].as_array().map(|a| a.len()).unwrap_or(0)
                    );
                }
                None => println!("{text}"),
            }
            Ok(())
        }
        AgentkitCommand::Profiles { command } => match command.unwrap_or(ProfilesCommand::List) {
            ProfilesCommand::List => {
                let r = bridge.call("profiles", json!({})).await?;
                if json_out {
                    return print_json(&r);
                }
                for p in r.as_array().into_iter().flatten() {
                    println!(
                        "{:<20} {}",
                        p["id"].as_str().unwrap_or(""),
                        p["description"].as_str().unwrap_or("")
                    );
                    for par in p["params"].as_array().into_iter().flatten() {
                        println!(
                            "  --param {}=…{}  {}",
                            par["key"].as_str().unwrap_or(""),
                            if par["required"].as_bool() == Some(true) {
                                " (required)"
                            } else {
                                ""
                            },
                            par["example"]
                                .as_str()
                                .map(|e| format!("e.g. {e}"))
                                .unwrap_or_default()
                        );
                    }
                }
                Ok(())
            }
            ProfilesCommand::Apply { id, params, opts } => {
                let mut body = plan_body(&opts);
                let mut p = serde_json::Map::new();
                for kv in params {
                    let (k, v) = kv
                        .split_once('=')
                        .ok_or_else(|| anyhow!("--param takes key=value"))?;
                    p.insert(k.trim().to_string(), json!(v.trim()));
                }
                body["id"] = json!(id);
                body["params"] = Value::Object(p);
                plan_and_apply(&bridge, "profile_plan", body, &opts, json_out).await
            }
        },
    }
}
