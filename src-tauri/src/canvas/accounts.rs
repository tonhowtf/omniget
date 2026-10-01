use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use omniget_core::core::canvas::accounts::{self as core_acc, AccountRef, NodeAccount, Resume};
use omniget_core::core::canvas::model::CanvasNode;
use omniget_core::core::canvas::ops::Op;
use omniget_core::core::llm::cli_runtime::accounts::{AccountStore, CliAccount};
use serde_json::{json, Map, Value};

pub const EVENT_ACCOUNTS: &str = "canvas://accounts";
const ORIGIN: &str = "canvas.launch";

pub fn accounts() -> Vec<CliAccount> {
    AccountStore::default_store()
        .map(|s| s.list().to_vec())
        .unwrap_or_default()
}

pub fn refs() -> Vec<AccountRef> {
    accounts().iter().map(AccountRef::from).collect()
}

fn running_map() -> &'static Mutex<HashMap<String, Option<String>>> {
    static M: OnceLock<Mutex<HashMap<String, Option<String>>>> = OnceLock::new();
    M.get_or_init(Default::default)
}

pub fn running() -> HashMap<String, Option<String>> {
    running_map().lock().map(|m| m.clone()).unwrap_or_default()
}

pub fn running_account(tid: &str) -> Option<Option<String>> {
    running_map().lock().ok().and_then(|m| m.get(tid).cloned())
}

fn patch_data(ws: &str, floor: &str, node_id: &str, data: Map<String, Value>) {
    if ws.is_empty() {
        return;
    }
    let op = Op::update_node(node_id, json!({ "data": data }));
    if let Err(e) = super::doc::host()
        .store()
        .apply(ws, floor, vec![op], ORIGIN, None)
    {
        tracing::warn!("[canvas.accounts] {node_id}: {e}");
    }
}

pub fn prepare(
    ws: &str,
    floor: &str,
    node: &CanvasNode,
    agent: &str,
) -> Result<(NodeAccount, Vec<CliAccount>), String> {
    let list = accounts();
    let id = core_acc::account_of(&node.data);
    if id.is_none() {
        if let Some(acc) = core_acc::legacy_account(&list, node.data.get("env"), agent) {
            let mut env = Map::new();
            env.insert(acc.cli.config_env().to_string(), Value::Null);
            let mut data = Map::new();
            data.insert("account".into(), json!(acc.id));
            data.insert("env".into(), Value::Object(env));
            patch_data(ws, floor, &node.id, data);
            tracing::info!(
                "[canvas.accounts] {}: env {} became account {}",
                node.id,
                acc.cli.config_env(),
                acc.id
            );
            return Ok((NodeAccount::Account(acc), list));
        }
        if let Some(env) = node.data.get("env").and_then(Value::as_object) {
            if let Some(var) = core_acc::CONFIG_VARS.iter().find(|v| env.contains_key(**v)) {
                tracing::warn!(
                    "[canvas.accounts] {}: env {var} is ignored; pick an account for this terminal",
                    node.id
                );
            }
        }
    }
    let acc = core_acc::resolve(&list, id.as_deref(), agent)?;
    Ok((acc, list))
}

fn label_of(list: &[CliAccount], id: &Option<String>) -> Value {
    match id {
        None => Value::Null,
        Some(id) => json!(list
            .iter()
            .find(|a| &a.id == id)
            .map(|a| a.label.clone())
            .unwrap_or_else(|| id.clone())),
    }
}

pub struct ResumeCtx<'a> {
    pub ws: &'a str,
    pub floor: &'a str,
    pub node_id: &'a str,
    pub tid: &'a str,
    pub agent: &'a str,
}

pub fn may_resume(c: &ResumeCtx, sid: &str, acc: &NodeAccount, list: &[CliAccount]) -> bool {
    if !matches!(c.agent, "claude" | "codex") {
        return false;
    }
    let current = acc.id();
    let dir = core_acc::config_dir(acc, c.agent);
    let here = core_acc::session_in(c.agent, sid, dir.as_deref());
    let elsewhere = if here {
        None
    } else {
        core_acc::locate_session(list, c.agent, sid, current)
    };
    match core_acc::decide(
        c.agent,
        current,
        core_acc::remembered(c.tid),
        here,
        elsewhere,
    ) {
        Resume::Yes => true,
        Resume::Fresh { previous } => {
            if let Some(prev) = previous {
                omniget_core::core::term::session::history::forget_agent_session(c.tid);
                let mut data = Map::new();
                data.insert(
                    "accountNotice".into(),
                    json!({
                        "kind": "fresh",
                        "previous": prev,
                        "previousLabel": label_of(list, &prev),
                        "at": chrono::Utc::now().timestamp_millis(),
                    }),
                );
                patch_data(c.ws, c.floor, c.node_id, data);
            }
            false
        }
    }
}

pub fn launched(tid: &str, acc: &NodeAccount) {
    let id = acc.id().map(str::to_string);
    core_acc::remember(tid, id.as_deref());
    if let Ok(mut m) = running_map().lock() {
        m.insert(tid.to_string(), id.clone());
    }
    super::chat::emit(EVENT_ACCOUNTS, json!({ "terminalId": tid, "account": id }));
}

pub fn node_account_json(node: &CanvasNode) -> Value {
    let Some(id) = core_acc::account_of(&node.data) else {
        return json!({ "id": null, "label": "environment" });
    };
    let list = accounts();
    match list.iter().find(|a| a.id == id) {
        Some(a) if a.disabled => {
            json!({ "id": a.id, "label": a.label, "error": "disabled" })
        }
        Some(a) => json!({ "id": a.id, "label": a.label }),
        None => json!({ "id": id, "label": null, "error": "missing" }),
    }
}
