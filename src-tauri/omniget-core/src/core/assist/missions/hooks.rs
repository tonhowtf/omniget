//! The broker's view of missions: which conversation belongs to a running
//! mission, the `before_tool` policies that may refuse a call, and the log
//! of tool calls of the current round (the progress guard reads it).
//!
//! The broker stays the only place tools run. It asks [`before_tool`] after
//! its own grant checks — a mission policy can only take a permission away,
//! never add one — and reports every result to [`after_tool`]. With no
//! mission attached to the conversation both are a map lookup.

use std::collections::HashMap;
use std::sync::RwLock;

use serde_json::{json, Value};

use super::policy::{glob, Policy, PolicyEffect, Trigger};

#[derive(Debug, Clone, Default)]
struct Attached {
    mission_id: String,
    deny: Vec<Policy>,
    calls: Vec<String>,
    failures: Vec<String>,
}

static ATTACHED: RwLock<Option<HashMap<String, Attached>>> = RwLock::new(None);

/// The driver attaches a mission to the conversation its task runs in.
pub fn attach(conversation: &str, mission_id: &str, policies: &[Policy]) {
    let deny: Vec<Policy> = policies
        .iter()
        .filter(|p| {
            p.trigger == Trigger::BeforeTool && matches!(p.effect, PolicyEffect::DenyTool { .. })
        })
        .cloned()
        .collect();
    let mut w = ATTACHED.write().unwrap_or_else(|e| e.into_inner());
    w.get_or_insert_with(HashMap::new).insert(
        conversation.to_string(),
        Attached {
            mission_id: mission_id.to_string(),
            deny,
            ..Default::default()
        },
    );
}

pub fn detach(conversation: &str) {
    let mut w = ATTACHED.write().unwrap_or_else(|e| e.into_inner());
    if let Some(m) = w.as_mut() {
        m.remove(conversation);
    }
}

pub fn mission_of(conversation: &str) -> Option<String> {
    ATTACHED
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|m| m.get(conversation).map(|a| a.mission_id.clone()))
}

/// `Some(reason)` when a mission policy refuses `tool` in `conversation`.
pub fn before_tool(conversation: &str, tool: &str, _input: &Value) -> Option<String> {
    let r = ATTACHED.read().unwrap_or_else(|e| e.into_inner());
    let a = r.as_ref()?.get(conversation)?;
    a.deny.iter().find_map(|p| {
        let hit = p
            .tool_glob
            .as_deref()
            .map(|g| glob(g, tool))
            .unwrap_or(true);
        match (&p.effect, hit) {
            (PolicyEffect::DenyTool { reason }, true) => {
                Some(format!("mission policy `{}`: {reason}", p.id))
            }
            _ => None,
        }
    })
}

/// Every brokered call of an attached conversation lands here.
pub fn after_tool(conversation: &str, tool: &str, input: &Value, ok: bool, ms: u32) {
    let mission = {
        let mut w = ATTACHED.write().unwrap_or_else(|e| e.into_inner());
        let Some(a) = w.as_mut().and_then(|m| m.get_mut(conversation)) else {
            return;
        };
        let sig = format!("{tool}:{}", short_digest(input));
        a.calls.push(sig);
        if a.calls.len() > 200 {
            a.calls.remove(0);
        }
        if !ok {
            a.failures.push(tool.to_string());
        }
        a.mission_id.clone()
    };
    if let Ok(db) = crate::core::assist::db::global() {
        let _ = super::append_event(
            &db,
            &mission,
            super::NewEvent::new(
                if ok { "after_tool" } else { "tool_failed" },
                json!({ "tool": tool, "ok": ok, "ms": ms, "conversation": conversation }),
            ),
        );
    }
}

/// Takes (and clears) the calls recorded since the last round.
pub fn take_calls(conversation: &str) -> (Vec<String>, Vec<String>) {
    let mut w = ATTACHED.write().unwrap_or_else(|e| e.into_inner());
    match w.as_mut().and_then(|m| m.get_mut(conversation)) {
        Some(a) => (
            std::mem::take(&mut a.calls),
            std::mem::take(&mut a.failures),
        ),
        None => (Vec::new(), Vec::new()),
    }
}

fn short_digest(v: &Value) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(&Sha256::digest(v.to_string().as_bytes())[..6])
}
