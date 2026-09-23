//! Hook dialects stored as TOML arrays of tables: Kimi Code (`[[hooks]]
//! event/matcher/command/timeout` in `~/.kimi-code/config.toml`) and Mistral
//! Vibe (`.vibe/hooks.toml` `[[hooks]] name/type/match/command/timeout`).

use serde_json::{json, Map, Value};

use super::hook_common::{
    annotate, legacy_env_loss, missing, num, prepare, shim_command, shim_note, support_files,
    unit_name, ShimCall,
};
use super::hook_json::tool_event;
use super::{kind_path, ConvertCtx, Dedupe, PatchOp, PlannedFile};
use crate::core::agentkit::edit::{keys, DocFormat};
use crate::core::agentkit::model::{Component, HookSpec};
use crate::core::agentkit::targets::TargetAdapter;
use crate::core::agentkit::{Result, Scope};

fn finish(
    c: &Component,
    ctx: &ConvertCtx,
    file: std::path::PathBuf,
    target: &TargetAdapter,
    entries: Vec<Value>,
    support: Vec<PlannedFile>,
    mut losses: Vec<String>,
    commands: Vec<String>,
) -> Vec<PlannedFile> {
    let mut files = Vec::new();
    if !entries.is_empty() {
        let ops = entries
            .into_iter()
            .map(|v| PatchOp::Append {
                path: keys(["hooks"]),
                value: v,
                // the same command may serve several events: compare whole entries
                dedupe: Dedupe::Equal,
            })
            .collect();
        files.push(PlannedFile::merge(
            &target.id,
            c,
            file,
            DocFormat::Toml,
            ops,
            "hook",
        ));
    }
    if let Some(l) = legacy_env_loss(c) {
        losses.push(l);
    }
    annotate(&mut files, losses, commands, Some(shim_note(ctx.env)));
    files.extend(support);
    files
}

pub fn kimi(
    c: &Component,
    h: &HookSpec,
    target: &TargetAdapter,
    scope: Scope,
    ctx: &ConvertCtx,
) -> Result<Vec<PlannedFile>> {
    let file = kind_path(target, "hooks", scope, ctx)
        .ok_or_else(|| missing(target, "hooks file", scope))?;
    let (support, rewrites) = support_files(c, h, target, scope, ctx)?;
    let (prep, losses, commands) = prepare(h, target, &rewrites, &[]);
    let mut entries = Vec::new();
    for p in &prep {
        for native in &p.natives {
            let tool = tool_event(&p.event);
            let call = ShimCall {
                tool: &target.id,
                event: &p.event,
                native_event: Some(native),
                matcher: if tool { None } else { p.matcher.as_deref() },
            };
            let mut e = Map::new();
            e.insert("event".into(), json!(native));
            if tool {
                if let Some(m) = p.native_matcher.as_deref().filter(|m| !m.is_empty()) {
                    e.insert("matcher".into(), json!(m));
                }
            }
            e.insert(
                "command".into(),
                json!(shim_command(ctx.env, &call, &p.command)),
            );
            if let Some(t) = p.timeout {
                e.insert("timeout".into(), num(t.clamp(1.0, 600.0).round()));
            }
            entries.push(Value::Object(e));
        }
    }
    Ok(finish(
        c, ctx, file, target, entries, support, losses, commands,
    ))
}

/// Vibe `match`: fnmatch glob, or `re:` + regex (case-insensitive).
fn vibe_match(m: &str) -> String {
    if m.contains('|') || m.contains('^') || m.contains('$') || m.contains(".*") {
        format!("re:{}", super::hook_json::anchored(m))
    } else {
        m.to_string()
    }
}

pub fn vibe(
    c: &Component,
    h: &HookSpec,
    target: &TargetAdapter,
    scope: Scope,
    ctx: &ConvertCtx,
) -> Result<Vec<PlannedFile>> {
    let file = kind_path(target, "hooks", scope, ctx)
        .ok_or_else(|| missing(target, "hooks file", scope))?;
    let (support, rewrites) = support_files(c, h, target, scope, ctx)?;
    let (prep, losses, commands) = prepare(h, target, &rewrites, &[]);
    let name = unit_name(c, ctx);
    let mut entries = Vec::new();
    for (i, p) in prep.iter().enumerate() {
        for native in &p.natives {
            let tool = tool_event(&p.event) && native != "post_agent";
            let call = ShimCall {
                tool: &target.id,
                event: &p.event,
                native_event: Some(native),
                matcher: if tool { None } else { p.matcher.as_deref() },
            };
            let mut e = Map::new();
            e.insert("name".into(), json!(format!("{name}-{}", i + 1)));
            e.insert("type".into(), json!(native));
            if tool {
                if let Some(m) = p.native_matcher.as_deref().filter(|m| !m.is_empty()) {
                    e.insert("match".into(), json!(vibe_match(m)));
                }
            }
            e.insert(
                "command".into(),
                json!(shim_command(ctx.env, &call, &p.command)),
            );
            if let Some(t) = p.timeout {
                e.insert("timeout".into(), json!(t));
            }
            entries.push(Value::Object(e));
        }
    }
    let mut files = finish(c, ctx, file, target, entries, support, losses, commands);
    if scope != Scope::Global {
        if let Some(f) = files.first_mut() {
            f.notes
                .push("Vibe loads project hooks only in a trusted folder".into());
        }
    }
    Ok(files)
}
