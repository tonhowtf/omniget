//! OmniGet's own observation hook (plan §5.3, F5): an opt-in hook component,
//! generated per tool, whose command is `omniget-hook-shim --observe`. The shim
//! posts every event to `POST /v1/observe/<tool>` on the local bridge with a
//! bearer of its own per tool, and on a permission request waits for the owner
//! to answer in the pet/UI (nobody answering = the tool asks as usual).
//!
//! It is an ordinary hook component, so it goes through the same plan/apply/
//! uninstall as the catalog, and through the same dialect converters (the shim
//! of the tool's dialect wraps the observe command).

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{json, Value};

use super::hook_common::{quote, shim_path};
use super::hook_shim::{read_token, tokens_path, DEFAULT_WAIT_SECS, MAX_WAIT_SECS};
use crate::core::agentkit::model::*;
use crate::core::agentkit::targets::TargetAdapter;
use crate::core::agentkit::{AgentkitError, Env, Result};

/// Tag carried by the observe component (`HookSpec.tags`).
pub const OBSERVE_TAG: &str = "omniget:observe";

/// Canonical events the observer subscribes to (those the tool has).
pub const OBSERVE_EVENTS: &[&str] = &[
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PostToolUseFailure",
    "PermissionRequest",
    "Notification",
    "Stop",
    "SessionEnd",
];

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ObserveOptions {
    /// Also hold `PreToolUse` for an answer (tools without a permission event).
    #[serde(default)]
    pub gate: bool,
    /// Seconds to wait for the owner (capped at 115).
    #[serde(default)]
    pub wait_secs: Option<u64>,
}

fn random_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// The token of `tool`, created on first use (`<app_data>/agentkit/observe/tokens.json`).
pub fn ensure_token(env: &Env, tool: &str) -> Result<String> {
    if let Some(t) = read_token(&env.app_data, tool) {
        return Ok(t);
    }
    let path = tokens_path(&env.app_data);
    let mut all: BTreeMap<String, Value> = std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let token = random_token();
    all.insert(
        tool.to_string(),
        json!({ "token": token, "created_at": crate::core::agentkit::now_iso() }),
    );
    write_private(&path, &serde_json::to_vec_pretty(&all).unwrap_or_default())?;
    Ok(token)
}

/// Forgets the token of `tool` (the observe hook, if still installed, stops
/// being accepted by the bridge).
pub fn revoke_token(env: &Env, tool: &str) -> Result<bool> {
    let path = tokens_path(&env.app_data);
    let mut all: BTreeMap<String, Value> = match std::fs::read(&path) {
        Ok(b) => serde_json::from_slice(&b).unwrap_or_default(),
        Err(_) => return Ok(false),
    };
    let had = all.remove(tool).is_some();
    if had {
        write_private(&path, &serde_json::to_vec_pretty(&all).unwrap_or_default())?;
    }
    Ok(had)
}

/// Tools that have an observe token.
pub fn observed_tools(env: &Env) -> Vec<String> {
    std::fs::read(tokens_path(&env.app_data))
        .ok()
        .and_then(|b| serde_json::from_slice::<BTreeMap<String, Value>>(&b).ok())
        .map(|m| m.into_keys().collect())
        .unwrap_or_default()
}

/// Constant-time check of a bearer against the stored token of `tool`.
pub fn check_token(data_dir: &Path, tool: &str, provided: &str) -> bool {
    let Some(expected) = read_token(data_dir, tool) else {
        return false;
    };
    let (a, b) = (expected.as_bytes(), provided.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| AgentkitError::io("create", dir, &e))?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes).map_err(|e| AgentkitError::io("write", &tmp, &e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, path).map_err(|e| AgentkitError::io("rename", path, &e))
}

/// The observe command for `tool` (Claude dialect: tools that are not Claude
/// get it wrapped by their own shim call during conversion).
pub fn observe_command(env: &Env, tool: &str, opts: &ObserveOptions) -> String {
    let os = env.os;
    let wait = opts
        .wait_secs
        .unwrap_or(DEFAULT_WAIT_SECS)
        .min(MAX_WAIT_SECS);
    let mut parts = vec![
        quote(&shim_path(env).display().to_string(), os),
        "--tool".into(),
        "claude".into(),
        "--observe".into(),
        "--as".into(),
        quote(tool, os),
        "--data-dir".into(),
        quote(&env.app_data.display().to_string(), os),
        "--wait".into(),
        wait.to_string(),
    ];
    if opts.gate {
        parts.push("--gate".into());
    }
    parts.join(" ")
}

/// Builds the observe component for one tool. Pure: the token is created by
/// [`ensure_token`] (the host does it before planning).
pub fn observe_component(env: &Env, target: &TargetAdapter, opts: &ObserveOptions) -> Component {
    let cmd = observe_command(env, &target.id, opts);
    let wait = opts
        .wait_secs
        .unwrap_or(DEFAULT_WAIT_SECS)
        .min(MAX_WAIT_SECS) as f64;
    let reads_claude = target.reads_claude(ComponentKind::Hook);
    let mut entries = Vec::new();
    for ev in OBSERVE_EVENTS {
        // tools that read Claude's settings get Claude's event names
        let has = if reads_claude || target.id == "claude" {
            true
        } else {
            !target.map_event(ev).is_empty()
        };
        if !has {
            continue;
        }
        let holds = *ev == "PermissionRequest" || (opts.gate && *ev == "PreToolUse");
        entries.push(HookEntry {
            event: ev.to_string(),
            matcher: None,
            handler: HookHandler {
                kind: "command".into(),
                command: Some(cmd.clone()),
                url: None,
                prompt: None,
                timeout: Some(if holds { wait + 15.0 } else { 10.0 }),
                extra: Default::default(),
            },
        });
    }
    let hooks_json = {
        let mut m = serde_json::Map::new();
        for e in &entries {
            let g = json!({ "hooks": [{ "type": "command", "command": cmd, "timeout": e.handler.timeout }] });
            if let Some(a) = m
                .entry(e.event.clone())
                .or_insert_with(|| json!([]))
                .as_array_mut()
            {
                a.push(g);
            }
        }
        json!({ "description": "OmniGet observation hook", "tags": [OBSERVE_TAG], "hooks": m })
    };
    let entry_name = "omniget-observe.json".to_string();
    let mut c = Component {
        id: format!("omniget:hooks/observe/{}", target.id),
        kind: ComponentKind::Hook,
        name: "omniget-observe".into(),
        category: Some("omniget".into()),
        description: format!(
            "Reports {} sessions, tool calls and permission requests to OmniGet (pet, activity, analytics).",
            target.name
        ),
        source: Some(SourceRef {
            id: "omniget".into(),
            ..Default::default()
        }),
        license: None,
        author: Some("OmniGet".into()),
        sha256: String::new(),
        origin_tool: "omniget".into(),
        security: None,
        compat: BTreeMap::new(),
        body: ComponentBody::Hook(HookSpec {
            entries,
            supporting_files: vec![],
            tags: vec![OBSERVE_TAG.into()],
        }),
        files: vec![ComponentFile {
            path: entry_name.clone(),
            bytes: serde_json::to_vec_pretty(&hooks_json).unwrap_or_default(),
            executable: false,
        }],
        entry: entry_name,
    };
    c.rehash();
    c
}
