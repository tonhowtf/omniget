use omniget_core::core::canvas::accounts::{self as core_acc, AccountRef};
use omniget_core::core::canvas::model::{CanvasNode, NodeKind};
use omniget_core::core::canvas::ops::Op;
use omniget_core::core::llm::cli_runtime::accounts::CliKind;
use serde_json::{json, Map, Value};

use super::{arr, def, out, ty, Def};
use crate::portal::sessions::{self, PortalSession};

pub const LIST: &str = "canvas_accounts_list";

fn nullable(desc: &str) -> Value {
    json!({"type": ["string", "null"], "description": desc})
}

pub fn augment(mut defs: Vec<Def>) -> Vec<Def> {
    for d in defs.iter_mut() {
        let Some(props) = d.tool.input_schema["properties"].as_object_mut() else {
            continue;
        };
        let account = || {
            nullable("CLI account of a Claude/Codex/shell terminal: id or unique label from canvas_accounts_list; null or \"\" = the environment's account. Applies on the next start")
        };
        let session = || {
            nullable("Named portal session (a logged-in browser profile the person made): id or unique label from canvas_accounts_list; null or \"\" = the portal's own storage")
        };
        match d.tool.name {
            "canvas_node_spawn" => {
                props.insert("account".into(), account());
                props.insert("session".into(), session());
            }
            "canvas_node_update" => {
                props.insert("account".into(), account());
                props.insert("session".into(), session());
            }
            "canvas_terminal_settings_set" => {
                props.insert("account".into(), account());
            }
            "canvas_portal_open" => {
                props.insert("session".into(), session());
            }
            _ => {}
        }
    }
    defs.push(def(
        LIST,
        "The CLI accounts a terminal node can run on ({id, label, cli, disabled, loggedIn: true|false|null when unknown}) and the named portal sessions a portal can use ({id, label}). Pick one with account on canvas_node_spawn / canvas_node_update / canvas_terminal_settings_set, or session on canvas_portal_open. Only the person creates accounts and sessions and logs in; no paths, e-mails, tokens or cookies are shown.",
        false,
        json!({}),
        &[],
        out(
            json!({"accounts": arr(ty("object")), "sessions": arr(ty("object"))}),
            &["accounts", "sessions"],
        ),
    ));
    defs
}

pub fn list() -> Value {
    let sessions: Vec<Value> = sessions::list()
        .into_iter()
        .map(|s| json!({ "id": s.id, "label": s.label }))
        .collect();
    let accounts: Vec<Value> = crate::canvas::accounts::accounts()
        .iter()
        .map(|a| {
            let mut v = serde_json::to_value(AccountRef::from(a)).unwrap_or_default();
            v["loggedIn"] = json!(core_acc::logged_in(a));
            v
        })
        .collect();
    json!({
        "accounts": accounts,
        "sessions": sessions,
        "namedSessionsSupported": sessions::supported(),
    })
}

fn describe_accounts(list: &[&AccountRef]) -> String {
    if list.is_empty() {
        return "none (the person adds accounts in the LLM accounts screen)".into();
    }
    list.iter()
        .map(|a| {
            format!(
                "{} ({}, {}{})",
                a.id,
                a.label,
                a.cli,
                if a.disabled { ", disabled" } else { "" }
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn describe_sessions(list: &[&PortalSession]) -> String {
    if list.is_empty() {
        return "none (the person creates sessions in a portal or in Settings → Canvas)".into();
    }
    list.iter()
        .map(|s| format!("{} ({})", s.id, s.label))
        .collect::<Vec<_>>()
        .join("; ")
}

fn cleared(v: &Value) -> bool {
    v.is_null() || v.as_str().is_some_and(|s| s.trim().is_empty())
}

pub fn resolve_account(v: &Value, cli: Option<CliKind>) -> Result<Value, String> {
    if cleared(v) {
        return Ok(Value::Null);
    }
    let key = v.as_str().ok_or("INVALID_ARGUMENT: account is a string")?;
    let all = crate::canvas::accounts::refs();
    let fit: Vec<AccountRef> = all
        .into_iter()
        .filter(|a| cli.is_none_or(|c| a.cli == c.as_str()))
        .collect();
    let refs: Vec<&AccountRef> = fit.iter().collect();
    match core_acc::match_label(&fit, key, |a| &a.id, |a| &a.label) {
        Ok(a) if a.disabled => Err(format!(
            "CANVAS_ACCOUNT: '{}' is disabled; valid accounts: {}",
            a.label,
            describe_accounts(
                &refs
                    .iter()
                    .filter(|a| !a.disabled)
                    .copied()
                    .collect::<Vec<_>>()
            )
        )),
        Ok(a) => Ok(json!(a.id)),
        Err(hits) if hits.len() > 1 => Err(format!(
            "CANVAS_ACCOUNT: '{key}' matches {} accounts; pass the id: {}",
            hits.len(),
            describe_accounts(&hits)
        )),
        Err(_) => Err(format!(
            "CANVAS_ACCOUNT: no account '{key}'{}; valid accounts: {}",
            cli.map(|c| format!(" for {c}")).unwrap_or_default(),
            describe_accounts(&refs)
        )),
    }
}

pub fn resolve_session(v: &Value) -> Result<Value, String> {
    if cleared(v) {
        return Ok(Value::Null);
    }
    let key = v.as_str().ok_or("INVALID_ARGUMENT: session is a string")?;
    if !sessions::supported() {
        return Err(
            "PORTAL_SESSION_UNSUPPORTED: named portal sessions need macOS 14 or later".into(),
        );
    }
    let all = sessions::list();
    let refs: Vec<&PortalSession> = all.iter().collect();
    match core_acc::match_label(&all, key, |s| &s.id, |s| &s.label) {
        Ok(s) => Ok(json!(s.id)),
        Err(hits) if hits.len() > 1 => Err(format!(
            "PORTAL_SESSION: '{key}' matches {} sessions; pass the id: {}",
            hits.len(),
            describe_sessions(&hits)
        )),
        Err(_) => Err(format!(
            "PORTAL_SESSION: no session '{key}'; valid sessions: {}",
            describe_sessions(&refs)
        )),
    }
}

fn cli_for(agent: &str) -> Option<CliKind> {
    core_acc::cli_of_agent(agent)
}

fn merge_data(a: &mut Value, key: &str, v: Value) {
    if !a.get("data").is_some_and(Value::is_object) {
        a["data"] = json!({});
    }
    a["data"][key] = v;
}

fn take(a: &mut Value, key: &str) -> Option<Value> {
    a.as_object_mut().and_then(|o| o.remove(key))
}

fn set_on_node(ws: &str, floor: &str, node_id: &str, key: &str, v: Value) -> Result<(), String> {
    let mut data = Map::new();
    data.insert(key.into(), v);
    if key == "account" {
        data.insert("accountNotice".into(), Value::Null);
    }
    let op = Op::update_node(node_id, json!({ "data": data }));
    crate::canvas::doc::host()
        .store()
        .apply(ws, floor, vec![op], super::ORIGIN, None)
        .map(|_| ())
}

fn check_terminal(n: &CanvasNode) -> Result<Option<CliKind>, String> {
    if !crate::canvas::launch::is_terminal(n) {
        return Err(format!(
            "CANVAS_INVALID: '{}' is not a terminal; account is for terminals (portals take session)",
            n.id
        ));
    }
    let agent = crate::canvas::launch::agent_of(n);
    if !core_acc::takes_account(&agent) {
        return Err(format!(
            "CANVAS_ACCOUNT: accounts work with Claude, Codex and shell terminals, not {agent}"
        ));
    }
    Ok(cli_for(&agent))
}

fn check_portal(n: &CanvasNode) -> Result<(), String> {
    if n.kind != NodeKind::Portal {
        return Err(format!(
            "CANVAS_INVALID: '{}' is not a portal; session is for portals (terminals take account)",
            n.id
        ));
    }
    Ok(())
}

pub fn before(
    s: &omniget_core::core::canvas::store::CanvasStore,
    name: &str,
    a: &mut Value,
) -> Result<(), String> {
    match name {
        "canvas_node_spawn" => {
            let kind = a["kind"].as_str().unwrap_or("").to_string();
            let terminal = matches!(kind.as_str(), "terminal" | "agent");
            if let Some(v) = take(a, "account") {
                if !terminal {
                    return Err(
                        "CANVAS_INVALID: account is for terminal nodes (portals take session)"
                            .into(),
                    );
                }
                let agent = a["agent"].as_str().unwrap_or("").to_string();
                if super::AGENTS.contains(&agent.as_str()) && !core_acc::takes_account(&agent) {
                    return Err(format!(
                        "CANVAS_ACCOUNT: accounts work with Claude, Codex and shell terminals, not {agent}"
                    ));
                }
                let id = resolve_account(&v, cli_for(&agent))?;
                merge_data(a, "account", id);
            }
            if let Some(v) = take(a, "session") {
                if kind != "portal" {
                    return Err(
                        "CANVAS_INVALID: session is for portal nodes (terminals take account)"
                            .into(),
                    );
                }
                let id = resolve_session(&v)?;
                merge_data(a, "session", id);
            }
        }
        "canvas_node_update" => {
            let acc = take(a, "account");
            let ses = take(a, "session");
            if acc.is_none() && ses.is_none() {
                return Ok(());
            }
            let (_, _, n) = super::locate_arg(s, a, "nodeId")?;
            if let Some(v) = acc {
                let cli = check_terminal(&n)?;
                let id = resolve_account(&v, cli)?;
                merge_data(a, "account", id);
                a["data"]["accountNotice"] = Value::Null;
            }
            if let Some(v) = ses {
                check_portal(&n)?;
                let id = resolve_session(&v)?;
                merge_data(a, "session", id);
            }
        }
        "canvas_terminal_settings_set" => {
            if let Some(v) = take(a, "account") {
                let (ws, floor, n) = super::locate_arg(s, a, "nodeId")?;
                let cli = check_terminal(&n)?;
                let id = resolve_account(&v, cli)?;
                set_on_node(&ws, &floor, &n.id, "account", id)?;
            }
        }
        "canvas_portal_open" => {
            if let Some(v) = take(a, "session") {
                let id = resolve_session(&v)?;
                if super::st(a, "nodeId").is_some() {
                    let (ws, floor, n) = super::locate_arg(s, a, "nodeId")?;
                    check_portal(&n)?;
                    set_on_node(&ws, &floor, &n.id, "session", id)?;
                } else {
                    merge_data(a, "session", id);
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn acts_on_node(name: &str) -> bool {
    name.starts_with("canvas_terminal_")
        || name.starts_with("canvas_portal_")
        || matches!(name, "canvas_node_spawn" | "canvas_node_update")
}

fn node_id_in(v: &Value) -> Option<String> {
    v.get("nodeId")
        .and_then(Value::as_str)
        .or_else(|| {
            v.get("node")
                .and_then(|n| n.get("id"))
                .and_then(Value::as_str)
        })
        .or_else(|| {
            v.get("settings")
                .and_then(|n| n.get("nodeId"))
                .and_then(Value::as_str)
        })
        .map(str::to_string)
}

pub fn session_json(n: &CanvasNode) -> Value {
    match sessions::session_of(&n.data) {
        Some(id) => match sessions::get(&id) {
            Some(s) => json!({ "id": s.id, "label": s.label }),
            None => json!({ "id": id, "label": null, "error": "missing" }),
        },
        None => json!({
            "id": null,
            "label": crate::portal::storage::scope_of(n).as_str(),
        }),
    }
}

pub fn echo(name: &str, r: Result<Value, String>) -> Result<Value, String> {
    let mut v = r?;
    if name == "canvas_terminals_list" {
        list_accounts(&mut v);
        return Ok(v);
    }
    if !acts_on_node(name) || !v.is_object() {
        return Ok(v);
    }
    let Some(id) = node_id_in(&v) else {
        return Ok(v);
    };
    let store = crate::canvas::doc::host().store();
    let Some((ws, floor)) = store.locate_node(&id) else {
        return Ok(v);
    };
    let Ok(Some(n)) = store.read(&ws, &floor, |d| d.node(&id).cloned()) else {
        return Ok(v);
    };
    if crate::canvas::launch::is_terminal(&n) {
        let mut acc = crate::canvas::accounts::node_account_json(&n);
        if let Some(tid) = crate::canvas::launch::terminal_of(&n) {
            if let Some(running) = crate::canvas::accounts::running_account(&tid) {
                let alive = crate::pty::term().info(&tid).is_some_and(|i| i.alive);
                if alive && running.as_deref() != core_acc::account_of(&n.data).as_deref() {
                    acc["restartToApply"] = json!(true);
                }
            }
        }
        v["account"] = acc;
        if matches!(
            name,
            "canvas_node_spawn"
                | "canvas_node_update"
                | "canvas_terminal_settings_set"
                | "canvas_terminal_ensure"
                | "canvas_terminal_restart"
        ) {
            if let Some(w) = crate::canvas::accounts::login_warning(&n) {
                v["warning"] = match v.get("warning").and_then(Value::as_str) {
                    Some(old) => json!(format!("{old}; {w}")),
                    None => json!(w),
                };
            }
        }
    } else if n.kind == NodeKind::Portal {
        v["session"] = session_json(&n);
    }
    Ok(v)
}

fn list_accounts(v: &mut Value) {
    let store = crate::canvas::doc::host().store();
    let Some(items) = v.get_mut("terminals").and_then(Value::as_array_mut) else {
        return;
    };
    for item in items {
        let Some(id) = item
            .get("nodeId")
            .or_else(|| item.get("id"))
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            continue;
        };
        let Some((ws, floor)) = store.locate_node(&id) else {
            continue;
        };
        if let Ok(Some(n)) = store.read(&ws, &floor, |d| d.node(&id).cloned()) {
            if crate::canvas::launch::is_terminal(&n) {
                item["account"] = crate::canvas::accounts::node_account_json(&n);
            }
        }
    }
}
