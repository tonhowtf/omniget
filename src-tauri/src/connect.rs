use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use axum::{
    extract::{Path, Query},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use omniget_core::core::agentkit::{
    self as ak,
    model::{Component, ComponentKind},
    plan::{ConflictPolicy, PlanRequest, UnitStatus},
    Env, Scope,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

const PENDING_MS: i64 = 5 * 60 * 1000;
const KEEP_MS: i64 = 15 * 60 * 1000;
const MAX_PENDING: usize = 4;
const MAX_WAIT_S: u64 = 25;
const GRANT_MAX_TOKENS: u64 = 2_000_000;

pub const SKILL_NAME: &str = "omniget-connect";

const DOWNLOAD_SCOPES: &[&str] = &[
    "discover",
    "enqueue",
    "control",
    "diagnostics",
    "artifacts",
    "history",
];
const AGENT_SCOPES: &[&str] = &[
    "agents_run",
    "orchestration_read",
    "orchestration_control",
    "company_read",
    "company_control",
    "catalog",
    "app_observe",
    "app_control",
    "app_capture",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Pending,
    Approved,
    Denied,
    Expired,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConnectRequest {
    pub id: String,
    pub client: String,
    pub client_name: String,
    pub workspace: Option<String>,
    pub browser: bool,
    pub code: String,
    pub created_ms: i64,
    pub status: Status,
    pub message: String,
}

fn store() -> &'static Mutex<Vec<ConnectRequest>> {
    static S: OnceLock<Mutex<Vec<ConnectRequest>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(Vec::new()))
}

fn changed() -> &'static tokio::sync::Notify {
    static N: OnceLock<tokio::sync::Notify> = OnceLock::new();
    N.get_or_init(tokio::sync::Notify::new)
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn sweep(list: &mut Vec<ConnectRequest>) {
    let now = now_ms();
    for r in list.iter_mut() {
        if r.status == Status::Pending && now - r.created_ms > PENDING_MS {
            r.status = Status::Expired;
            r.message = "The request expired. Ask again.".into();
        }
    }
    list.retain(|r| now - r.created_ms < KEEP_MS);
}

fn code() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let bytes = uuid::Uuid::new_v4();
    let s: String = bytes
        .as_bytes()
        .iter()
        .take(6)
        .map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char)
        .collect();
    format!("{}-{}", &s[..3], &s[3..])
}

fn connectable(id: &str) -> Option<&'static ak::targets::TargetAdapter> {
    ak::targets::target(id).filter(|t| t.default_mcp().next().is_some())
}

fn connectable_ids() -> Vec<&'static str> {
    ak::targets::all_targets()
        .iter()
        .filter(|t| t.default_mcp().next().is_some())
        .map(|t| t.id.as_str())
        .collect()
}

fn connection_name(tool_name: &str) -> String {
    format!("{tool_name} · connected")
}

fn port(app: &AppHandle) -> u16 {
    crate::storage::config::load_settings(app).bridge.port
}

pub fn router<S>(app: AppHandle) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    let a1 = app.clone();
    let a2 = app.clone();
    Router::new()
        .route(
            "/connect",
            get(move |q: Query<BTreeMap<String, String>>| {
                let app = a1.clone();
                async move { instructions(&app, q.get("client").map(String::as_str)) }
            }),
        )
        .route(
            "/v1/connect",
            post(
                move |q: Query<BTreeMap<String, String>>,
                      headers: HeaderMap,
                      body: axum::body::Bytes| {
                    let app = a2.clone();
                    async move { request(&app, q.0, &headers, &body) }
                },
            ),
        )
        .route(
            "/v1/connect/{id}",
            get(
                |Path(id): Path<String>, q: Query<BTreeMap<String, String>>| async move {
                    let wait = q
                        .get("wait")
                        .and_then(|w| w.parse::<u64>().ok())
                        .unwrap_or(0)
                        .min(MAX_WAIT_S);
                    poll(&id, wait).await
                },
            ),
        )
}

fn from_browser(headers: &HeaderMap) -> bool {
    headers.contains_key("origin")
        || headers.contains_key("sec-fetch-mode")
        || headers.contains_key("sec-fetch-site")
}

fn refuse(status: StatusCode, code: &str, message: String) -> Response {
    (
        status,
        Json(json!({ "status": "error", "error": code, "message": message })),
    )
        .into_response()
}

fn instructions(app: &AppHandle, client: Option<&str>) -> Response {
    let port = port(app);
    let text = match client.and_then(connectable) {
        Some(t) => instructions_for(port, &t.id, &t.name),
        None => format!(
            "# Connect to OmniGet\n\nOmniGet is open on this computer. Find the id of the AI tool you are in this \
             list, then get your steps with `curl -s \"http://127.0.0.1:{port}/connect?client=<id>\"`.\n\n{}\n",
            ak::targets::all_targets()
                .iter()
                .filter(|t| t.default_mcp().next().is_some())
                .map(|t| format!("- `{}`: {}", t.id, t.name))
                .collect::<Vec<_>>()
                .join("\n")
        ),
    };
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/markdown; charset=utf-8",
        )],
        text,
    )
        .into_response()
}

pub fn instructions_for(port: u16, id: &str, name: &str) -> String {
    format!(
        r#"# Connect {name} to OmniGet

OmniGet is open on this computer. Connecting gives {name} OmniGet's MCP tools: downloads,
the person's AI agents as background jobs, missions and companies of agents, and, when the
person ticks it, the Global Downloader (its browser, site connectors and whole-site downloads).
OmniGet connects you by itself, unless the person turned on asking first. Any access token goes from OmniGet straight into {name}'s
own config file and never through this chat, so do not look for it.

1. Ask for access. Run this from the folder the person is working in (their agents may work there):

   curl -s -X POST "http://127.0.0.1:{port}/v1/connect?client={id}" --data-urlencode "workspace=$PWD"

   On Windows PowerShell: curl.exe -s -X POST "http://127.0.0.1:{port}/v1/connect?client={id}" --data-urlencode "workspace=$($PWD.Path)"

   Add `--data-urlencode "browser=1"` when the person wants you to download from websites, courses
   or anything behind a login, or to write a connector: OmniGet then offers the Global Downloader too.

   The answer is JSON with `code` and `wait_url`.
2. Run `curl -s "<wait_url>"`. It answers within 25 seconds. Run it again while `status` is `pending`
   (the request expires after 5 minutes). If it stays `pending`, tell the person, in their language:
   "Click Allow in the OmniGet window. It shows the code <code>."
3. `approved`: tell the person what `message` says (which file changed and how to reload {name}).
   `denied`: stop and say so. `expired`: start again at step 1. `failed`: show `message`.
"#
    )
}

#[derive(Deserialize, Default)]
struct RequestBody {
    client: Option<String>,
    workspace: Option<String>,
    browser: Option<String>,
}

fn request(
    app: &AppHandle,
    query: BTreeMap<String, String>,
    headers: &HeaderMap,
    body: &[u8],
) -> Response {
    if from_browser(headers) {
        return refuse(
            StatusCode::FORBIDDEN,
            "BROWSER_REFUSED",
            "Connect from the AI tool's terminal, not from a web page.".into(),
        );
    }
    let json_body = headers
        .get("content-type")
        .and_then(|h| h.to_str().ok())
        .is_some_and(|c| c.starts_with("application/json"));
    let parsed: RequestBody = if body.is_empty() {
        RequestBody::default()
    } else if json_body {
        serde_json::from_slice(body).unwrap_or_default()
    } else {
        let form: BTreeMap<String, String> = url::form_urlencoded::parse(body)
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        RequestBody {
            client: form.get("client").cloned(),
            workspace: form.get("workspace").cloned(),
            browser: form.get("browser").cloned(),
        }
    };
    let client = parsed
        .client
        .or_else(|| query.get("client").cloned())
        .unwrap_or_default();
    let Some(target) = connectable(client.trim()) else {
        return refuse(
            StatusCode::BAD_REQUEST,
            "UNKNOWN_CLIENT",
            format!(
                "Pass client=<id of the tool you are>. Ids: {}",
                connectable_ids().join(", ")
            ),
        );
    };
    let workspace = parsed
        .workspace
        .or_else(|| query.get("workspace").cloned())
        .map(|w| w.trim().to_string())
        .filter(|w| {
            let p = std::path::Path::new(w);
            p.is_absolute() && p.is_dir()
        });
    let browser = parsed
        .browser
        .or_else(|| query.get("browser").cloned())
        .is_some_and(|b| matches!(b.trim(), "1" | "true" | "yes"));
    let req = {
        let Ok(mut list) = store().lock() else {
            return refuse(StatusCode::INTERNAL_SERVER_ERROR, "LOCK", "busy".into());
        };
        sweep(&mut list);
        for r in list.iter_mut() {
            if r.client == target.id && r.status == Status::Pending {
                r.status = Status::Expired;
                r.message = "Replaced by a newer request.".into();
            }
        }
        if list.iter().filter(|r| r.status == Status::Pending).count() >= MAX_PENDING {
            return refuse(
                StatusCode::TOO_MANY_REQUESTS,
                "TOO_MANY_REQUESTS",
                "Other tools are waiting for an answer in OmniGet. Try again in a few minutes."
                    .into(),
            );
        }
        let req = ConnectRequest {
            id: uuid::Uuid::new_v4().to_string(),
            client: target.id.clone(),
            client_name: target.name.clone(),
            workspace,
            browser,
            code: code(),
            created_ms: now_ms(),
            status: Status::Pending,
            message: String::new(),
        };
        list.push(req.clone());
        req
    };
    changed().notify_waiters();
    let ask_first = crate::mcp::access::get().ask_first;
    if ask_first {
        let _ = app.emit("connect-request", &req);
        crate::tray::show_window(app);
        if let Some(w) = app.get_webview_window("main") {
            let _ = w.request_user_attention(Some(tauri::UserAttentionType::Critical));
        }
    } else {
        let (a, id) = (app.clone(), req.id.clone());
        tauri::async_runtime::spawn(async move {
            let _ = connect_answer(a, id, true, true, true, Some(true)).await;
        });
    }
    let port = port(app);
    (
        StatusCode::OK,
        Json(json!({
            "id": req.id,
            "status": "pending",
            "code": req.code,
            "client": req.client,
            "workspace": req.workspace,
            "wait_url": format!("http://127.0.0.1:{port}/v1/connect/{}?wait={MAX_WAIT_S}", req.id),
            "message": if ask_first { format!("Tell the person to click Allow in the OmniGet window, which shows the code {}. Then poll wait_url while status is pending.", req.code) } else { "Connecting now. Poll wait_url until status is approved.".to_string() },
        })),
    )
        .into_response()
}

fn view(r: &ConnectRequest) -> Value {
    json!({
        "status": r.status,
        "code": r.code,
        "client": r.client,
        "message": r.message,
    })
}

async fn poll(id: &str, wait_s: u64) -> Response {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(wait_s);
    loop {
        let notified = changed().notified();
        let current = store().lock().ok().and_then(|mut list| {
            sweep(&mut list);
            list.iter().find(|r| r.id == id).cloned()
        });
        let Some(r) = current else {
            return refuse(
                StatusCode::NOT_FOUND,
                "UNKNOWN_REQUEST",
                "No such request (it may be too old). Ask again.".into(),
            );
        };
        if r.status != Status::Pending || tokio::time::Instant::now() >= deadline {
            return (StatusCode::OK, Json(view(&r))).into_response();
        }
        let nap = deadline
            .saturating_duration_since(tokio::time::Instant::now())
            .min(Duration::from_secs(5));
        let _ = tokio::time::timeout(nap, notified).await;
    }
}

fn set_answer(id: &str, status: Status, message: String) {
    if let Ok(mut list) = store().lock() {
        if let Some(r) = list.iter_mut().find(|r| r.id == id) {
            r.status = status;
            r.message = message;
        }
    }
    changed().notify_waiters();
}

#[derive(Debug, Clone, Serialize)]
pub struct Connected {
    pub client: String,
    pub client_name: String,
    pub files: Vec<String>,
    pub agents: Vec<String>,
    pub workspace: Option<String>,
    pub message: String,
}

fn reload_hint(id: &str, name: &str) -> String {
    match id {
        "claude" => "Start a new Claude Code session (or restart this one) to load the omniget MCP server; `/mcp` lists it.".into(),
        "codex" => "Restart Codex to load the omniget MCP server; `/mcp` lists it.".into(),
        "gemini" => "Restart Gemini CLI to load the omniget MCP server; `/mcp` lists it.".into(),
        _ => format!("Restart {name} (or reload its MCP servers) to load the omniget server."),
    }
}

fn mcp_component(url: &str, token: &str) -> Result<Component, String> {
    let record = json!({
        "mcpServers": {
            "omniget": {
                "type": "http",
                "url": url,
                "headers": { "Authorization": format!("Bearer {token}") },
            }
        }
    });
    let mut files = ak::parse::RawFiles::new();
    files.insert(
        "omniget.json".into(),
        serde_json::to_vec_pretty(&record).map_err(|e| e.to_string())?,
    );
    let mut c =
        ak::parse::parse_raw(ComponentKind::Mcp, "omniget.json", &files).map_err(String::from)?;
    c.id = "omniget:connect/mcp".into();
    c.name = "omniget".into();
    c.description = "OmniGet on this computer: downloads, agents, jobs and missions.".into();
    Ok(c)
}

fn install(env: &Env, target: &str, components: Vec<Component>) -> Result<Vec<String>, String> {
    let plan = ak::plan::plan(
        env,
        PlanRequest {
            components,
            targets: vec![target.to_string()],
            scope: Some(Scope::Global),
            project_dir: None,
            policy: ConflictPolicy::Overwrite,
            secret_values: Default::default(),
        },
    )
    .map_err(String::from)?;
    if let Some(bad) = plan.units.iter().find(|u| {
        matches!(
            u.status,
            UnitStatus::Unsupported | UnitStatus::Error | UnitStatus::Conflict
        )
    }) {
        return Err(bad
            .error
            .clone()
            .unwrap_or_else(|| format!("{} cannot hold it ({:?})", bad.target_name, bad.status)));
    }
    let report = ak::writer::apply(env, &plan).map_err(String::from)?;
    let mut files = report.files_written;
    files.extend(report.unchanged);
    Ok(files)
}

fn executors(app: &AppHandle) -> Vec<omniget_core::core::llm::agent::AgentDef> {
    let state = app.state::<crate::AppState>();
    if let Some(a) = state.llm.roster().into_iter().find(|a| {
        matches!(&a.runtime, omniget_core::core::llm::agent::RuntimeKind::Cli { cli, .. } if cli == "claude")
    }) {
        let _ = tauri::async_runtime::block_on(omniget_core::core::llm::caps::probe(&a.runtime));
    }
    let derived = omniget_core::core::assist::db::global()
        .ok()
        .and_then(|db| omniget_core::core::assist::external_config::derived_bot_marks(&db).ok())
        .unwrap_or_default();
    state
        .llm
        .roster()
        .into_iter()
        .filter(|a| !derived.contains_key(&a.id))
        .filter(|a| omniget_core::core::assist::authority::external_runtime(a).is_ok())
        .collect()
}

fn add_cli_agents(app: &AppHandle) -> Vec<String> {
    use omniget_core::core::llm::agent::{AgentDef, AgentRole, ModelPolicy, RuntimeKind};
    use omniget_core::core::llm::cli_runtime::accounts::CliKind;
    use omniget_core::core::llm::types::{ModelRef, ProviderId};
    let state = app.state::<crate::AppState>();
    let roster = state.llm.roster();
    let mut added = Vec::new();
    for kind in CliKind::ALL {
        if tauri::async_runtime::block_on(omniget_core::core::dependencies::find_tool(kind.bin()))
            .is_none()
        {
            continue;
        }
        let Ok(account) =
            crate::commands::central::run::terminal_account(&state.llm, kind.as_str(), None)
        else {
            continue;
        };
        let mine: Vec<&AgentDef> = roster
            .iter()
            .filter(|a| matches!(&a.runtime, RuntimeKind::Cli { cli, .. } if cli == kind.as_str()))
            .collect();
        if !mine.is_empty() {
            for a in mine {
                if matches!(&a.runtime, RuntimeKind::Cli { account, .. } if account.is_empty()) {
                    let mut fixed = a.clone();
                    fixed.runtime = RuntimeKind::Cli {
                        cli: kind.as_str().into(),
                        account: account.clone(),
                    };
                    let _ = state.llm.roster_update(fixed);
                }
            }
            continue;
        }
        let (id, name) = match kind {
            CliKind::Claude => ("claude-code", "Claude Code"),
            CliKind::Codex => ("codex", "Codex"),
        };
        if roster.iter().any(|a| a.id == id) {
            continue;
        }
        let agent = AgentDef {
            id: id.into(),
            name: name.into(),
            role: AgentRole::Worker,
            system_prompt: String::new(),
            model: ModelPolicy::Fixed {
                model: ModelRef {
                    provider: ProviderId::new(kind.as_str()),
                    model: "default".into(),
                },
            },
            tools: Vec::new(),
            skills: Vec::new(),
            budget: Default::default(),
            runtime: RuntimeKind::Cli {
                cli: kind.as_str().into(),
                account,
            },
            skin: None,
        };
        if state.llm.roster_create(agent).is_ok() {
            added.push(name.to_string());
        }
    }
    added
}

pub fn connect(
    app: &AppHandle,
    client: &str,
    workspace: Option<&str>,
    agents: bool,
    approvals: bool,
    browser: bool,
) -> Result<Connected, String> {
    use omniget_core::core::assist::authority;
    let target = connectable(client).ok_or("UNKNOWN_CLIENT")?;
    if !crate::mcp::enabled() {
        crate::mcp::set_enabled(true)?;
    }
    let new_agents = if agents {
        add_cli_agents(app)
    } else {
        Vec::new()
    };
    let bots = if agents && workspace.is_some() {
        executors(app)
    } else {
        Vec::new()
    };
    let access = crate::mcp::access::get();
    let mut scopes: Vec<String> = DOWNLOAD_SCOPES.iter().map(|s| s.to_string()).collect();
    if agents {
        scopes.extend(AGENT_SCOPES.iter().map(|s| s.to_string()));
        if !bots.is_empty() {
            scopes.push("orchestration_create".into());
        }
    }
    if approvals && agents {
        scopes.push("approvals_respond".into());
    }
    if browser {
        scopes.push("global_downloader".into());
    }
    if !access.ask_first {
        scopes = access.scopes();
    }
    let name = connection_name(&target.name);
    let previous: Vec<String> = crate::mcp::policy::list()?
        .into_iter()
        .filter(|p| p.name == name)
        .map(|p| p.id)
        .collect();
    let grant = crate::mcp::policy::create(name, scopes)?;
    let principal = grant.principal.id.clone();
    let undo = |e: String| {
        let _ = crate::mcp::policy::revoke(&principal);
        e
    };
    let mut agent_names = Vec::new();
    if let (Some(ws), false) = (workspace, bots.is_empty()) {
        let mut revisions = BTreeMap::new();
        for bot in &bots {
            revisions.insert(
                bot.id.clone(),
                authority::agent_revision(bot).map_err(undo)?,
            );
            agent_names.push(bot.name.clone());
        }
        let database = omniget_core::core::assist::db::global().map_err(undo)?;
        authority::grant(
            &database,
            authority::Ceiling {
                id: String::new(),
                principal: principal.clone(),
                workspace_id: uuid::Uuid::new_v4().to_string(),
                workspace: ws.to_string(),
                workspace_identity: None,
                bots: bots.iter().map(|b| b.id.clone()).collect(),
                bot_revisions: revisions,
                tools: [
                    "fs_read",
                    "fs_list",
                    "fs_glob",
                    "fs_grep",
                    "fs_write",
                    "fs_edit",
                    "fs_apply_patch",
                ]
                .into_iter()
                .map(str::to_owned)
                .collect(),
                max_tokens: GRANT_MAX_TOKENS,
                max_usd: None,
            },
        )
        .map_err(undo)?;
    }
    let url = format!("http://127.0.0.1:{}/mcp", port(app));
    let env = Env::system().map_err(String::from).map_err(undo)?;
    let files = mcp_component(&url, &grant.token)
        .and_then(|c| install(&env, &target.id, vec![c]))
        .map_err(undo)?;
    for id in previous {
        let _ = crate::mcp::policy::revoke(&id);
    }
    let mut message = format!(
        "{} is connected to OmniGet. Changed: {}. {}",
        target.name,
        if files.is_empty() {
            "nothing".to_string()
        } else {
            files.join(", ")
        },
        reload_hint(&target.id, &target.name)
    );
    if !new_agents.is_empty() {
        message.push_str(&format!(
            " Added to OmniGet as agents, on their terminal login: {}.",
            new_agents.join(", ")
        ));
    }
    if agents && workspace.is_some() && agent_names.is_empty() {
        message.push_str(
            " No AI agent is set up in OmniGet yet, so it cannot start agents; add one in OmniGet → LLM.",
        );
    } else if !agent_names.is_empty() {
        message.push_str(&format!(
            " Agents it may run in {}: {}.",
            workspace.unwrap_or_default(),
            agent_names.join(", ")
        ));
    }
    let _ = app.emit("connect-changed", ());
    Ok(Connected {
        client: target.id.clone(),
        client_name: target.name.clone(),
        files,
        agents: agent_names,
        workspace: workspace.map(str::to_owned),
        message,
    })
}

pub fn skill_text() -> String {
    format!(
        r##"---
name: {SKILL_NAME}
description: Connect this AI tool to the OmniGet desktop app on this computer (downloads, the Global Downloader for courses and sites behind a login, site connectors, the person's AI agents as background jobs, missions) through OmniGet's local MCP server. Use when the person asks to connect, set up or configure OmniGet, to use OmniGet from here, or to download a course or a whole site with OmniGet when its tools are not available yet.
---

# Connect to OmniGet

1. Get the current steps from the OmniGet app on this computer. It listens on one of the
   ports 47720 to 47729; use the first answer that starts with `# Connect`:

   for p in 47720 47721 47722 47723 47724 47725 47726 47727 47728 47729; do r=$(curl -s -m 2 "http://127.0.0.1:$p/connect"); case "$r" in "# Connect"*) echo "$r"; break;; esac; done

   (Windows PowerShell: try `curl.exe -s "http://127.0.0.1:<port>/connect"` on each port.)
   If no port answers that way, OmniGet is not open or is older than this feature: ask the
   person to open or update it (https://getomniget.com/download) and try again.
2. Follow the steps it returns. If the person wants to download from websites, courses or
   anything behind a login, or to write a connector, ask for the Global Downloader too
   (`browser=1` in the request).
"##
    )
}

fn connect_components() -> Result<(Component, Component), String> {
    let text = skill_text();
    let mut files = ak::parse::RawFiles::new();
    files.insert("SKILL.md".into(), text.clone().into_bytes());
    let mut skill =
        ak::parse::parse_raw(ComponentKind::Skill, "SKILL.md", &files).map_err(String::from)?;
    skill.id = "omniget:connect/skill".into();
    let mut files = ak::parse::RawFiles::new();
    files.insert(format!("{SKILL_NAME}.md"), text.into_bytes());
    let mut command =
        ak::parse::parse_raw(ComponentKind::Command, &format!("{SKILL_NAME}.md"), &files)
            .map_err(String::from)?;
    command.id = "omniget:connect/command".into();
    Ok((skill, command))
}

fn install_skill(env: &Env, id: &str) -> Result<Vec<String>, String> {
    let (skill, command) = connect_components()?;
    install(env, id, vec![skill]).or_else(|_| install(env, id, vec![command]))
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolView {
    pub id: String,
    pub name: String,
    pub installed: bool,
    pub connected: bool,
    pub skill: bool,
}

async fn blocking<T, F>(f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn connect_pending() -> Vec<ConnectRequest> {
    store()
        .lock()
        .map(|mut list| {
            sweep(&mut list);
            list.iter()
                .filter(|r| r.status == Status::Pending)
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

#[tauri::command]
pub async fn connect_answer(
    app: AppHandle,
    id: String,
    allow: bool,
    agents: bool,
    approvals: bool,
    browser: Option<bool>,
) -> Result<Value, String> {
    let req = store()
        .lock()
        .map_err(|_| "LOCK")?
        .iter()
        .find(|r| r.id == id && r.status == Status::Pending)
        .cloned()
        .ok_or("ERR_CONNECT_GONE")?;
    if !allow {
        set_answer(&id, Status::Denied, "The person said no in OmniGet.".into());
        return Ok(json!({ "status": "denied" }));
    }
    let a = app.clone();
    let result = blocking(move || {
        connect(
            &a,
            &req.client,
            req.workspace.as_deref(),
            agents,
            approvals,
            browser.unwrap_or(false),
        )
    })
    .await;
    match result {
        Ok(done) => {
            set_answer(&id, Status::Approved, done.message.clone());
            Ok(serde_json::to_value(done).map_err(|e| e.to_string())?)
        }
        Err(e) => {
            set_answer(
                &id,
                Status::Failed,
                format!("OmniGet could not write the connection: {e}"),
            );
            Err(e)
        }
    }
}

#[tauri::command]
pub async fn connect_tools() -> Result<Vec<ToolView>, String> {
    blocking(|| {
        let env = Env::system().map_err(String::from)?;
        let connected: Vec<String> = crate::mcp::policy::list()
            .unwrap_or_default()
            .into_iter()
            .map(|p| p.name)
            .collect();
        let mut out: Vec<ToolView> = ak::detect::detect_all(&env, None)
            .into_iter()
            .filter(|d| connectable(&d.id).is_some())
            .map(|d| {
                let skill = [
                    ak::targets::target(&d.id)
                        .and_then(|t| t.path("skills", Scope::Global, &env, None))
                        .map(|p| p.join(SKILL_NAME).join("SKILL.md")),
                    ak::targets::target(&d.id)
                        .and_then(|t| t.path("commands", Scope::Global, &env, None))
                        .map(|p| p.join(format!("{SKILL_NAME}.md"))),
                ]
                .into_iter()
                .flatten()
                .any(|p: PathBuf| p.exists());
                ToolView {
                    connected: connected.contains(&connection_name(&d.name)),
                    id: d.id,
                    name: d.name,
                    installed: d.installed,
                    skill,
                }
            })
            .collect();
        out.sort_by(|a, b| {
            b.installed
                .cmp(&a.installed)
                .then(b.connected.cmp(&a.connected))
                .then(a.name.cmp(&b.name))
        });
        Ok(out)
    })
    .await
}

#[tauri::command]
pub async fn connect_tool(
    app: AppHandle,
    client: String,
    workspace: Option<String>,
    agents: bool,
    approvals: bool,
    browser: Option<bool>,
) -> Result<Connected, String> {
    let workspace = workspace.filter(|w| {
        let p = std::path::Path::new(w);
        p.is_absolute() && p.is_dir()
    });
    blocking(move || {
        connect(
            &app,
            &client,
            workspace.as_deref(),
            agents,
            approvals,
            browser.unwrap_or(false),
        )
    })
    .await
}

#[tauri::command]
pub async fn connect_install_skill(targets: Vec<String>) -> Result<Value, String> {
    blocking(move || {
        let env = Env::system().map_err(String::from)?;
        let ids: Vec<String> = if targets.is_empty() {
            ak::detect::detect_all(&env, None)
                .into_iter()
                .filter(|d| d.installed && connectable(&d.id).is_some())
                .map(|d| d.id)
                .collect()
        } else {
            targets
                .iter()
                .filter_map(|id| connectable(id).map(|t| t.id.clone()))
                .collect()
        };
        let mut out = serde_json::Map::new();
        for id in ids {
            out.insert(
                id.clone(),
                match install_skill(&env, &id) {
                    Ok(files) => json!({ "ok": true, "files": files }),
                    Err(e) => json!({ "ok": false, "error": e }),
                },
            );
        }
        Ok(Value::Object(out))
    })
    .await
}

#[tauri::command]
pub fn connect_prompt(app: AppHandle) -> String {
    format!(
        "Connect to OmniGet: run curl -s \"http://127.0.0.1:{}/connect\" and follow the steps.",
        port(&app)
    )
}
