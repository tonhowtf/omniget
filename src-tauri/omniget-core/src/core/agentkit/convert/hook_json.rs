//! Hook dialects stored as JSON: Cursor (flat camelCase, `version: 1`),
//! Copilot native (`.github/hooks/*.json`, `bash`/`powershell`, `timeoutSec`),
//! Gemini (own event names, ms), Kiro v1 (`{version:"v1",hooks:[{trigger,
//! matcher,action}]}`), Windsurf Cascade (snake_case, no matcher), Goose (Open
//! Plugins hooks.json), Devin CLI `hooks.v1.json` and Crush.

use serde_json::{json, Map, Value};

use super::hook_common::{
    annotate, legacy_env_loss, missing, num, prepare, shim_command, shim_command_ps, shim_note,
    support_files, unit_name, Prepared, ShimCall,
};
use super::{format_for_path, kind_path, ConvertCtx, Dedupe, PatchOp, PlannedFile};
use crate::core::agentkit::edit::{keys, DocFormat, Seg};
use crate::core::agentkit::model::{Component, HookSpec};
use crate::core::agentkit::targets::TargetAdapter;
use crate::core::agentkit::{Os, Result, Scope};

/// Events whose matcher is a tool name (the rest match a source/reason string).
pub fn tool_event(event: &str) -> bool {
    matches!(
        event,
        "PreToolUse" | "PostToolUse" | "PostToolUseFailure" | "PermissionRequest"
    )
}

/// `Edit|Write` style list → `^(Edit|Write)$`; a real regex stays as is.
pub fn anchored(m: &str) -> String {
    let simple = m
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '|' | '.' | ':' | '@' | '/'));
    if simple && !m.contains(".*") {
        format!("^({})$", m.replace('.', "\\."))
    } else {
        m.to_string()
    }
}

/// The matcher to write natively, and the one the shim must check instead.
fn split_matcher<'p>(p: &'p Prepared, native_ok: bool) -> (Option<String>, Option<&'p str>) {
    match &p.matcher {
        None => (None, None),
        Some(m) if native_ok && tool_event(&p.event) => {
            let nm = p.native_matcher.clone().unwrap_or_else(|| m.clone());
            if nm.is_empty() {
                // every name mapped to "" (no such tool there): let the shim filter
                (None, Some(m.as_str()))
            } else {
                (Some(nm), None)
            }
        }
        Some(m) => (None, Some(m.as_str())),
    }
}

fn hook_file(target: &TargetAdapter, scope: Scope, ctx: &ConvertCtx) -> Result<std::path::PathBuf> {
    kind_path(target, "hooks", scope, ctx).ok_or_else(|| missing(target, "hooks file", scope))
}

fn finish(
    c: &Component,
    ctx: &ConvertCtx,
    mut files: Vec<PlannedFile>,
    support: Vec<PlannedFile>,
    mut losses: Vec<String>,
    commands: Vec<String>,
) -> Vec<PlannedFile> {
    if let Some(l) = legacy_env_loss(c) {
        losses.push(l);
    }
    annotate(&mut files, losses, commands, Some(shim_note(ctx.env)));
    files.extend(support);
    files
}

// ---------------------------------------------------------------- Cursor

pub fn cursor(
    c: &Component,
    h: &HookSpec,
    target: &TargetAdapter,
    scope: Scope,
    ctx: &ConvertCtx,
) -> Result<Vec<PlannedFile>> {
    let file = hook_file(target, scope, ctx)?;
    let (support, rewrites) = support_files(c, h, target, scope, ctx)?;
    let (prep, losses, commands) = prepare(h, target, &rewrites, &["prompt"]);
    let mut ops = vec![PatchOp::Set {
        path: keys(["version"]),
        value: target.hooks.version.clone().unwrap_or(json!(1)),
        rename_at: None,
    }];
    for p in &prep {
        let (native_m, shim_m) = split_matcher(p, true);
        for native in &p.natives {
            let mut e = Map::new();
            if p.entry.handler.kind == "prompt" {
                e.insert("type".into(), json!("prompt"));
                e.insert(
                    "prompt".into(),
                    json!(p.entry.handler.prompt.clone().unwrap_or_default()),
                );
            } else {
                let call = ShimCall {
                    tool: &target.id,
                    event: &p.event,
                    native_event: Some(native),
                    matcher: shim_m,
                };
                e.insert(
                    "command".into(),
                    json!(shim_command(ctx.env, &call, &p.command)),
                );
            }
            if let Some(m) = &native_m {
                e.insert("matcher".into(), json!(m));
            }
            if let Some(t) = p.timeout {
                e.insert("timeout".into(), num(t));
            }
            ops.push(PatchOp::Append {
                path: keys(["hooks", native.as_str()]),
                value: Value::Object(e),
                dedupe: Dedupe::FlatHook,
            });
        }
    }
    if ops.len() == 1 {
        return Ok(finish(c, ctx, vec![], support, losses, commands));
    }
    let pf = PlannedFile::merge(
        &target.id,
        c,
        file.clone(),
        format_for_path(&file),
        ops,
        "hook",
    );
    Ok(finish(c, ctx, vec![pf], support, losses, commands))
}

// ---------------------------------------------------------------- Copilot

pub fn copilot(
    c: &Component,
    h: &HookSpec,
    target: &TargetAdapter,
    scope: Scope,
    ctx: &ConvertCtx,
) -> Result<Vec<PlannedFile>> {
    // every *.json in .github/hooks (or ~/.copilot/hooks) loads: one file per component
    let base_scope = if scope == Scope::Local {
        Scope::Project
    } else {
        scope
    };
    let dir = hook_file(target, base_scope, ctx)?
        .parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| missing(target, "hooks folder", scope))?;
    let (support, rewrites) = support_files(c, h, target, scope, ctx)?;
    let (prep, losses, commands) = prepare(h, target, &rewrites, &[]);
    let mut hooks: Map<String, Value> = Map::new();
    for p in &prep {
        let (native_m, shim_m) = split_matcher(p, true);
        for native in &p.natives {
            let call = ShimCall {
                tool: &target.id,
                event: &p.event,
                native_event: Some(native),
                matcher: shim_m,
            };
            let mut e = Map::new();
            e.insert("type".into(), json!("command"));
            if let Some(m) = &native_m {
                e.insert("matcher".into(), json!(m));
            }
            e.insert(
                "bash".into(),
                json!(shim_command(ctx.env, &call, &p.command)),
            );
            if ctx.env.os == Os::Windows {
                e.insert(
                    "powershell".into(),
                    json!(shim_command_ps(ctx.env, &call, &p.command)),
                );
            }
            if let Some(t) = p.timeout {
                e.insert("timeoutSec".into(), num(t));
            }
            if let Some(a) = hooks
                .entry(native.clone())
                .or_insert_with(|| json!([]))
                .as_array_mut()
            {
                a.push(Value::Object(e));
            }
        }
    }
    if hooks.is_empty() {
        return Ok(finish(c, ctx, vec![], support, losses, commands));
    }
    let doc =
        json!({ "version": target.hooks.version.clone().unwrap_or(json!(1)), "hooks": hooks });
    let text = serde_json::to_string_pretty(&doc).unwrap_or_default() + "\n";
    let name = unit_name(c, ctx);
    let mut pf = PlannedFile::write(
        &target.id,
        c,
        dir.join(format!("{name}.json")),
        text,
        "hook",
    );
    pf.primary = true;
    Ok(finish(c, ctx, vec![pf], support, losses, commands))
}

// ---------------------------------------------------------------- Gemini

pub fn gemini(
    c: &Component,
    h: &HookSpec,
    target: &TargetAdapter,
    scope: Scope,
    ctx: &ConvertCtx,
) -> Result<Vec<PlannedFile>> {
    let file = hook_file(target, scope, ctx)?;
    let (support, rewrites) = support_files(c, h, target, scope, ctx)?;
    let (prep, losses, commands) = prepare(h, target, &rewrites, &[]);
    let groups = gemini_groups(&prep, target, ctx, &unit_name(c, ctx));
    let ops: Vec<PatchOp> = groups
        .into_iter()
        .map(|(native, g)| PatchOp::Append {
            path: keys(["hooks", native.as_str()]),
            value: g,
            dedupe: Dedupe::ClaudeHook,
        })
        .collect();
    if ops.is_empty() {
        return Ok(finish(c, ctx, vec![], support, losses, commands));
    }
    let mut pf = PlannedFile::merge(
        &target.id,
        c,
        file.clone(),
        format_for_path(&file),
        ops,
        "hook",
    );
    pf.notes.push(
        "Gemini fingerprints project hooks: a new or changed hook is untrusted until you accept it in /hooks".into(),
    );
    Ok(finish(c, ctx, vec![pf], support, losses, commands))
}

/// Gemini groups `{matcher, hooks:[{name,type,command,timeout(ms)}]}` per native event.
pub fn gemini_groups(
    prep: &[Prepared],
    target: &TargetAdapter,
    ctx: &ConvertCtx,
    name: &str,
) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    for p in prep {
        let (native_m, shim_m) = split_matcher(p, true);
        for native in &p.natives {
            let call = ShimCall {
                tool: &target.id,
                event: &p.event,
                native_event: Some(native),
                matcher: shim_m,
            };
            let mut hk = Map::new();
            hk.insert("name".into(), json!(name));
            hk.insert("type".into(), json!("command"));
            hk.insert(
                "command".into(),
                json!(shim_command(ctx.env, &call, &p.command)),
            );
            if let Some(t) = p.timeout {
                let ms = if target.hooks.timeout_unit.as_deref() == Some("ms") {
                    t * 1000.0
                } else {
                    t
                };
                hk.insert("timeout".into(), num(ms));
            }
            let mut g = Map::new();
            if let Some(m) = &native_m {
                g.insert("matcher".into(), json!(m));
            }
            g.insert("hooks".into(), json!([Value::Object(hk)]));
            out.push((native.clone(), Value::Object(g)));
        }
    }
    out
}

// ---------------------------------------------------------------- Kiro

pub fn kiro(
    c: &Component,
    h: &HookSpec,
    target: &TargetAdapter,
    scope: Scope,
    ctx: &ConvertCtx,
) -> Result<Vec<PlannedFile>> {
    // every *.json under .kiro/hooks loads: one file per component
    let dir = hook_file(target, scope, ctx)?
        .parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| missing(target, "hooks folder", scope))?;
    let (support, rewrites) = support_files(c, h, target, scope, ctx)?;
    let (prep, losses, commands) = prepare(h, target, &rewrites, &["prompt", "agent"]);
    let name = unit_name(c, ctx);
    let mut hooks = Vec::new();
    for (i, p) in prep.iter().enumerate() {
        let (native_m, shim_m) = split_matcher(p, true);
        for native in &p.natives {
            let action = if p.entry.handler.kind == "command" {
                let call = ShimCall {
                    tool: &target.id,
                    event: &p.event,
                    native_event: Some(native),
                    matcher: shim_m,
                };
                json!({ "type": "command", "command": shim_command(ctx.env, &call, &p.command) })
            } else {
                json!({ "type": "agent", "prompt": p.entry.handler.prompt.clone().unwrap_or_default() })
            };
            let mut e = Map::new();
            e.insert(
                "name".into(),
                json!(format!("{name} {} {}", p.event, i + 1)),
            );
            e.insert("trigger".into(), json!(native));
            if let Some(m) = &native_m {
                e.insert("matcher".into(), json!(m));
            }
            e.insert("action".into(), action);
            if let Some(t) = p.timeout {
                e.insert("timeout".into(), num(t));
            }
            e.insert("enabled".into(), json!(true));
            hooks.push(Value::Object(e));
        }
    }
    if hooks.is_empty() {
        return Ok(finish(c, ctx, vec![], support, losses, commands));
    }
    let doc =
        json!({ "version": target.hooks.version.clone().unwrap_or(json!("v1")), "hooks": hooks });
    let text = serde_json::to_string_pretty(&doc).unwrap_or_default() + "\n";
    let mut pf = PlannedFile::write(
        &target.id,
        c,
        dir.join(format!("{name}.json")),
        text,
        "hook",
    );
    pf.primary = true;
    Ok(finish(c, ctx, vec![pf], support, losses, commands))
}

// ---------------------------------------------------------------- Windsurf Cascade

/// The Claude tool a Cascade event is about.
fn cascade_subject(native: &str) -> &'static str {
    match native.split_once('_').map(|x| x.1).unwrap_or("") {
        "run_command" => "Bash",
        "write_code" => "Edit",
        "read_code" => "Read",
        "mcp_tool_use" => "mcp__x__y",
        _ => "",
    }
}

pub fn cascade(
    c: &Component,
    h: &HookSpec,
    target: &TargetAdapter,
    scope: Scope,
    ctx: &ConvertCtx,
) -> Result<Vec<PlannedFile>> {
    let file = hook_file(target, scope, ctx)?;
    let (support, rewrites) = support_files(c, h, target, scope, ctx)?;
    let (prep, mut losses, commands) = prepare(h, target, &rewrites, &[]);
    let mut ops = Vec::new();
    for p in &prep {
        for native in &p.natives {
            // no matcher field: keep only the events the matcher can fire on,
            // and let the shim check the rest
            let subject = cascade_subject(native);
            if let Some(m) = &p.matcher {
                if !subject.is_empty() && tool_event(&p.event) {
                    let hit = super::hook_common::matcher_matches(m, subject)
                        || (subject == "Edit" && super::hook_common::matcher_matches(m, "Write"))
                        || (subject == "Edit"
                            && super::hook_common::matcher_matches(m, "MultiEdit"));
                    if !hit {
                        continue;
                    }
                }
            }
            let call = ShimCall {
                tool: &target.id,
                event: &p.event,
                native_event: Some(native),
                matcher: p.matcher.as_deref().filter(|_| subject != "Edit"),
            };
            let mut e = Map::new();
            e.insert(
                "command".into(),
                json!(shim_command(ctx.env, &call, &p.command)),
            );
            if ctx.env.os == Os::Windows {
                e.insert(
                    "powershell".into(),
                    json!(shim_command_ps(ctx.env, &call, &p.command)),
                );
            }
            e.insert("show_output".into(), json!(false));
            ops.push(PatchOp::Append {
                path: keys(["hooks", native.as_str()]),
                value: Value::Object(e),
                dedupe: Dedupe::FlatHook,
            });
        }
        if p.timeout.is_some() {
            let l = "hook timeout (Cascade has no timeout field)".to_string();
            if !losses.contains(&l) {
                losses.push(l);
            }
        }
    }
    if ops.is_empty() {
        return Ok(finish(c, ctx, vec![], support, losses, commands));
    }
    let pf = PlannedFile::merge(
        &target.id,
        c,
        file.clone(),
        format_for_path(&file),
        ops,
        "hook",
    );
    Ok(finish(c, ctx, vec![pf], support, losses, commands))
}

// ---------------------------------------------------------------- Goose

pub fn goose(
    c: &Component,
    h: &HookSpec,
    target: &TargetAdapter,
    scope: Scope,
    ctx: &ConvertCtx,
) -> Result<Vec<PlannedFile>> {
    let file = hook_file(target, scope, ctx)?;
    let (support, rewrites) = support_files(c, h, target, scope, ctx)?;
    let (prep, losses, commands) = prepare(h, target, &rewrites, &[]);
    let mut ops = Vec::new();
    for p in &prep {
        let (native_m, shim_m) = split_matcher(p, true);
        for native in &p.natives {
            let call = ShimCall {
                tool: &target.id,
                event: &p.event,
                native_event: Some(native),
                matcher: shim_m,
            };
            let mut hk = Map::new();
            hk.insert("type".into(), json!("command"));
            hk.insert(
                "command".into(),
                json!(shim_command(ctx.env, &call, &p.command)),
            );
            if let Some(t) = p.timeout {
                hk.insert("timeout".into(), num(t));
            }
            let mut g = Map::new();
            // a bare `*` makes goose skip the rule: no matcher = everything
            if let Some(m) = &native_m {
                g.insert("matcher".into(), json!(anchored(m)));
            }
            g.insert("hooks".into(), json!([Value::Object(hk)]));
            ops.push(PatchOp::Append {
                path: keys(["hooks", native.as_str()]),
                value: Value::Object(g),
                dedupe: Dedupe::ClaudeHook,
            });
        }
    }
    if ops.is_empty() {
        return Ok(finish(c, ctx, vec![], support, losses, commands));
    }
    let mut pf = PlannedFile::merge(&target.id, c, file.clone(), DocFormat::Json, ops, "hook");
    pf.notes.push(
        "goose loads hooks from the `omniget` plugin folder; turn it off with disabledPlugins"
            .into(),
    );
    Ok(finish(c, ctx, vec![pf], support, losses, commands))
}

// ---------------------------------------------------------------- Devin CLI

pub fn devin(
    c: &Component,
    h: &HookSpec,
    target: &TargetAdapter,
    scope: Scope,
    ctx: &ConvertCtx,
) -> Result<Vec<PlannedFile>> {
    let file = hook_file(target, scope, ctx)?;
    // .devin/hooks.v1.json is the bare hooks object; config.json keeps it under "hooks"
    let bare = file
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.ends_with("hooks.v1.json"))
        .unwrap_or(false);
    let (support, rewrites) = support_files(c, h, target, scope, ctx)?;
    let (prep, losses, commands) = prepare(h, target, &rewrites, &["prompt"]);
    let mut ops = Vec::new();
    for p in &prep {
        let (native_m, shim_m) = split_matcher(p, true);
        for native in &p.natives {
            let mut hk = Map::new();
            if p.entry.handler.kind == "prompt" {
                hk.insert("type".into(), json!("prompt"));
                hk.insert(
                    "prompt".into(),
                    json!(p.entry.handler.prompt.clone().unwrap_or_default()),
                );
            } else {
                let call = ShimCall {
                    tool: &target.id,
                    event: &p.event,
                    native_event: Some(native),
                    matcher: shim_m,
                };
                hk.insert("type".into(), json!("command"));
                hk.insert(
                    "command".into(),
                    json!(shim_command(ctx.env, &call, &p.command)),
                );
            }
            if let Some(t) = p.timeout {
                hk.insert("timeout".into(), num(t));
            }
            let mut g = Map::new();
            if let Some(m) = &native_m {
                g.insert("matcher".into(), json!(anchored(m)));
            }
            g.insert("hooks".into(), json!([Value::Object(hk)]));
            let mut path: Vec<Seg> = if bare { vec![] } else { keys(["hooks"]) };
            path.push(Seg::key(native.as_str()));
            ops.push(PatchOp::Append {
                path,
                value: Value::Object(g),
                dedupe: Dedupe::ClaudeHook,
            });
        }
    }
    if ops.is_empty() {
        return Ok(finish(c, ctx, vec![], support, losses, commands));
    }
    let pf = PlannedFile::merge(
        &target.id,
        c,
        file.clone(),
        format_for_path(&file),
        ops,
        "hook",
    );
    Ok(finish(c, ctx, vec![pf], support, losses, commands))
}

// ---------------------------------------------------------------- Crush

pub fn crush(
    c: &Component,
    h: &HookSpec,
    target: &TargetAdapter,
    scope: Scope,
    ctx: &ConvertCtx,
) -> Result<Vec<PlannedFile>> {
    let file = hook_file(target, scope, ctx)?;
    let (support, rewrites) = support_files(c, h, target, scope, ctx)?;
    let (prep, losses, commands) = prepare(h, target, &rewrites, &[]);
    let name = unit_name(c, ctx);
    let mut ops = Vec::new();
    for (i, p) in prep.iter().enumerate() {
        let (native_m, shim_m) = split_matcher(p, true);
        for native in &p.natives {
            let call = ShimCall {
                tool: &target.id,
                event: &p.event,
                native_event: Some(native),
                matcher: shim_m,
            };
            let mut e = Map::new();
            e.insert("name".into(), json!(format!("{name}-{}", i + 1)));
            if let Some(m) = &native_m {
                e.insert("matcher".into(), json!(anchored(m)));
            }
            e.insert(
                "command".into(),
                json!(shim_command(ctx.env, &call, &p.command)),
            );
            if let Some(t) = p.timeout {
                e.insert("timeout".into(), num(t));
            }
            ops.push(PatchOp::Append {
                path: keys(["hooks", native.as_str()]),
                value: Value::Object(e),
                dedupe: Dedupe::FlatHook,
            });
        }
    }
    if ops.is_empty() {
        return Ok(finish(c, ctx, vec![], support, losses, commands));
    }
    let pf = PlannedFile::merge(
        &target.id,
        c,
        file.clone(),
        format_for_path(&file),
        ops,
        "hook",
    );
    Ok(finish(c, ctx, vec![pf], support, losses, commands))
}
