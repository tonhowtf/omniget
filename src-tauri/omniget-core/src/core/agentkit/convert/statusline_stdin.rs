//! Statusline for tools with a Claude-like command statusline (plan §4.3,
//! estudo 06 §0C.9): Cursor CLI (`~/.cursor/cli-config.json`), Qwen
//! (`ui.statusLine`), Qoder, Droid (`statusLine` without `type`) and Copilot CLI.
//! Claude itself goes through the Claude converter.
//!
//! The command runs behind a **stdin adapter**: the tool's JSON is rewritten to
//! Claude's shape ([`adapt_stdin`]) before the script sees it, so the 32
//! statuslines of the catalog (written for Claude) read `model.display_name`,
//! `workspace.current_dir`, `cost.*`, `context_window.*` everywhere. The adapter
//! is the hook shim in statusline mode: `omniget-hook-shim --tool <tool>
//! --statusline -- '<command>'` ([`statusline_main`]).

use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

use super::{
    kind_path, place_support, rewrite_command, ConvertCtx, Converter, PatchOp, PlannedFile,
};
use crate::core::agentkit::edit::{keys, Seg};
use crate::core::agentkit::model::*;
use crate::core::agentkit::targets::TargetAdapter;
use crate::core::agentkit::{AgentkitError, Env, Result, Scope};

/// Statusline formats this module writes.
pub const FORMATS: &[&str] = &["qwen", "droid", "cursor_cli", "qoder", "copilot_cli"];

pub struct StatuslineConverter;

impl Converter for StatuslineConverter {
    fn id(&self) -> &'static str {
        "statusline_stdin"
    }

    fn supports(&self, kind: ComponentKind, target: &TargetAdapter) -> bool {
        kind == ComponentKind::Statusline
            && target
                .format(kind)
                .map(|f| FORMATS.contains(&f))
                .unwrap_or(false)
    }

    fn convert(
        &self,
        c: &Component,
        target: &TargetAdapter,
        scope: Scope,
        ctx: &ConvertCtx,
    ) -> Result<Vec<PlannedFile>> {
        let ComponentBody::Statusline(sl) = &c.body else {
            return Ok(vec![]);
        };
        let mut files = statusline_files(
            c,
            target,
            scope,
            ctx,
            &sl.command,
            sl.padding,
            &sl.supporting_files,
        )?;
        if !sl.extra.is_empty() {
            if let Some(f) = files.first_mut() {
                for k in sl.extra.keys() {
                    let l = format!("setting `{k}` that came with the statusline is Claude-only");
                    if !f.losses.contains(&l) {
                        f.losses.push(l);
                    }
                }
            }
        }
        Ok(files)
    }
}

// ------------------------------------------------------------------ adapter binary

/// `'<omniget-hook-shim>' --tool <tool> --statusline -- '<command>'`: the hook
/// shim (k2, [`super::hook_common::shim_path`]) in statusline mode, which calls
/// [`statusline_main`].
pub fn wrap_command(env: &Env, tool: &str, command: &str) -> String {
    let os = env.os;
    let q = |s: &str| super::hook_common::quote(s, os);
    [
        q(&super::hook_common::shim_path(env).display().to_string()),
        "--tool".into(),
        q(tool),
        "--statusline".into(),
        "--".into(),
        q(command),
    ]
    .join(" ")
}

/// Statusline mode of the shim: stdin of `tool` → Claude's JSON → `command`
/// (run by the shell, in the session's directory); returns (exit, stdout).
/// Used by `omniget-hook-shim --statusline` and `omniget-cli agentkit statusline-shim`.
pub fn statusline_main(tool: &str, command: &str, stdin: &[u8]) -> (i32, String) {
    let input: Value = serde_json::from_slice(stdin).unwrap_or_else(|_| json!({}));
    let cwd = std::env::current_dir().ok();
    let adapted = adapt_stdin(tool, &input, cwd.as_deref());
    let dir = adapted
        .get("workspace")
        .and_then(|w| w.get("current_dir"))
        .and_then(|d| d.as_str())
        .map(PathBuf::from);
    let bytes = serde_json::to_vec(&adapted).unwrap_or_default();
    let out = super::hook_shim::run_shell(
        command,
        &bytes,
        dir.as_deref(),
        Some(std::time::Duration::from_secs(10)),
    );
    (out.exit, out.stdout)
}

/// Undoes [`wrap_command`] (for import/inventory): the inner command.
pub fn unwrap_command(cmd: &str) -> Option<String> {
    let (_, rest) = cmd.split_once(" -- ")?;
    let rest = rest.trim();
    if let Some(inner) = rest.strip_prefix('\'').and_then(|r| r.strip_suffix('\'')) {
        return Some(inner.replace("'\\''", "'"));
    }
    if let Some(inner) = rest.strip_prefix('"').and_then(|r| r.strip_suffix('"')) {
        return Some(inner.replace("\\\"", "\""));
    }
    Some(rest.to_string())
}

// ------------------------------------------------------------------ stdin translation

fn pick<'a>(v: &'a Value, paths: &[&str]) -> Option<&'a Value> {
    paths.iter().find_map(|p| {
        let mut cur = v;
        for seg in p.split('.') {
            cur = cur.get(seg)?;
        }
        (!cur.is_null()).then_some(cur)
    })
}

fn pick_str(v: &Value, paths: &[&str]) -> Option<String> {
    pick(v, paths).and_then(|x| match x {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    })
}

fn pick_f64(v: &Value, paths: &[&str]) -> Option<f64> {
    pick(v, paths).and_then(|x| match x {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim_end_matches('%').trim().parse().ok(),
        _ => None,
    })
}

fn obj<'a>(m: &'a mut Map<String, Value>, k: &str) -> &'a mut Map<String, Value> {
    let slot = m.entry(k.to_string()).or_insert_with(|| json!({}));
    if !slot.is_object() {
        *slot = json!({});
    }
    slot.as_object_mut().expect("object")
}

fn set_default(m: &mut Map<String, Value>, k: &str, v: Value) {
    let missing = m.get(k).map(|x| x.is_null()).unwrap_or(true);
    if missing {
        m.insert(k.to_string(), v);
    }
}

/// Rewrites a tool's statusline stdin to Claude's shape (estudo 06 Claude (i)):
/// `model{id,display_name}`, `cwd`, `workspace{current_dir,project_dir}`,
/// `session_id`, `transcript_path`, `version`, `output_style.name`,
/// `cost{total_cost_usd,total_duration_ms,total_lines_added,total_lines_removed}`,
/// `context_window{used_percentage,remaining_percentage,context_window_size,…}`.
/// Fields already in Claude's shape are kept as they are; unknown fields stay.
/// `cwd_fallback` fills the directory when the tool sends none.
pub fn adapt_stdin(tool: &str, input: &Value, cwd_fallback: Option<&Path>) -> Value {
    let mut m = input.as_object().cloned().unwrap_or_default();
    let src = Value::Object(m.clone());

    // model
    let model_id = pick_str(
        &src,
        &[
            "model.id",
            "model.name",
            "model.model",
            "modelId",
            "model_id",
            "model",
        ],
    );
    let model_name = pick_str(
        &src,
        &[
            "model.display_name",
            "model.displayName",
            "model.name",
            "modelName",
            "model_name",
            "model.id",
            "model",
        ],
    );
    let model = obj(&mut m, "model");
    if let Some(id) = model_id.clone().or_else(|| model_name.clone()) {
        set_default(model, "id", json!(id));
    }
    if let Some(n) = model_name.or(model_id) {
        set_default(model, "display_name", json!(n));
    }

    // directories
    let dir = pick_str(
        &src,
        &[
            "workspace.current_dir",
            "workspace.currentDir",
            "cwd",
            "workspace.cwd",
            "currentDir",
            "current_dir",
            "workingDirectory",
            "working_directory",
            "project.path",
        ],
    )
    .or_else(|| cwd_fallback.map(|p| p.display().to_string()));
    let project = pick_str(
        &src,
        &[
            "workspace.project_dir",
            "workspace.projectDir",
            "workspace.root",
            "projectDir",
            "project_dir",
            "project_root",
            "workspaceRoot",
        ],
    )
    .or_else(|| dir.clone());
    if let Some(d) = &dir {
        set_default(&mut m, "cwd", json!(d));
    }
    let ws = obj(&mut m, "workspace");
    if let Some(d) = &dir {
        set_default(ws, "current_dir", json!(d));
    }
    if let Some(p) = project {
        set_default(ws, "project_dir", json!(p));
    }

    // session
    if let Some(s) = pick_str(
        &src,
        &[
            "session_id",
            "sessionId",
            "session.id",
            "conversation_id",
            "conversationId",
        ],
    ) {
        set_default(&mut m, "session_id", json!(s));
    }
    if let Some(t) = pick_str(
        &src,
        &[
            "transcript_path",
            "transcriptPath",
            "session.transcript_path",
        ],
    ) {
        set_default(&mut m, "transcript_path", json!(t));
    }
    if let Some(v) = pick_str(&src, &["version", "cli_version", "cliVersion"]) {
        set_default(&mut m, "version", json!(v));
    }
    let style = pick_str(&src, &["output_style.name", "outputStyle", "output_style"])
        .unwrap_or_else(|| "default".into());
    let os_ = obj(&mut m, "output_style");
    set_default(os_, "name", json!(style));

    // cost
    let cost_usd = pick_f64(
        &src,
        &[
            "cost.total_cost_usd",
            "cost.totalCostUsd",
            "total_cost_usd",
            "totalCostUsd",
            "cost_usd",
            "cost.usd",
        ],
    );
    let duration = pick_f64(
        &src,
        &[
            "cost.total_duration_ms",
            "total_duration_ms",
            "durationMs",
            "duration_ms",
        ],
    );
    let added = pick_f64(
        &src,
        &["cost.total_lines_added", "lines_added", "linesAdded"],
    );
    let removed = pick_f64(
        &src,
        &["cost.total_lines_removed", "lines_removed", "linesRemoved"],
    );
    let cost = obj(&mut m, "cost");
    set_default(cost, "total_cost_usd", json!(cost_usd.unwrap_or(0.0)));
    set_default(
        cost,
        "total_duration_ms",
        json!(duration.unwrap_or(0.0) as u64),
    );
    set_default(cost, "total_api_duration_ms", json!(0));
    set_default(
        cost,
        "total_lines_added",
        json!(added.unwrap_or(0.0) as u64),
    );
    set_default(
        cost,
        "total_lines_removed",
        json!(removed.unwrap_or(0.0) as u64),
    );

    // context window
    let used = pick_f64(
        &src,
        &[
            "context_window.used_percentage",
            "context.used_percentage",
            "context.usedPercentage",
            "contextUsage.percent",
            "context_usage.percent",
            "context_used_percentage",
            "contextUsedPercentage",
            "context.percent",
        ],
    );
    let size = pick_f64(
        &src,
        &[
            "context_window.context_window_size",
            "context.window_size",
            "context.size",
            "contextWindow",
            "context_window_size",
            "contextWindowSize",
        ],
    );
    let in_tok = pick_f64(
        &src,
        &[
            "context_window.total_input_tokens",
            "usage.input_tokens",
            "tokens.input",
            "inputTokens",
            "input_tokens",
            "tokenUsage.inputTokens",
        ],
    );
    let out_tok = pick_f64(
        &src,
        &[
            "context_window.total_output_tokens",
            "usage.output_tokens",
            "tokens.output",
            "outputTokens",
            "output_tokens",
            "tokenUsage.outputTokens",
        ],
    );
    let used = used.or_else(|| match (in_tok, size) {
        (Some(i), Some(s)) if s > 0.0 => Some((i / s * 100.0).min(100.0)),
        _ => None,
    });
    let cw = obj(&mut m, "context_window");
    if let Some(u) = used {
        let u = (u * 10.0).round() / 10.0;
        set_default(cw, "used_percentage", json!(u));
        set_default(
            cw,
            "remaining_percentage",
            json!(((100.0 - u) * 10.0).round() / 10.0),
        );
    }
    if let Some(s) = size {
        set_default(cw, "context_window_size", json!(s as u64));
    }
    if let Some(i) = in_tok {
        set_default(cw, "total_input_tokens", json!(i as u64));
    }
    if let Some(o) = out_tok {
        set_default(cw, "total_output_tokens", json!(o as u64));
    }
    set_default(&mut m, "hook_event_name", json!("Status"));
    let _ = tool;
    Value::Object(m)
}

// ------------------------------------------------------------------ writer shared with settings

/// Where the statusline's own scripts go for a tool without a scripts folder:
/// next to the file that holds the statusline (`~/.qwen/scripts/`).
fn scripts_dir(target: &TargetAdapter, scope: Scope, ctx: &ConvertCtx) -> Option<PathBuf> {
    if let Some(p) = kind_path(target, "statusline_scripts", scope, ctx) {
        return Some(p);
    }
    let file = kind_path(target, "statusline", scope, ctx)
        .or_else(|| kind_path(target, "settings", scope, ctx))?;
    Some(file.parent()?.join("scripts"))
}

/// `.claude/scripts/a/b.py` → `a/b.py`; `~/.claude/scripts/x.sh` → `x.sh`.
fn script_rel(dest: &str) -> String {
    let d = dest
        .trim()
        .trim_start_matches("~/")
        .trim_start_matches("./");
    let parts: Vec<&str> = d.split('/').filter(|s| !s.is_empty()).collect();
    if parts.len() >= 3 && parts[0] == ".claude" {
        parts[2..].join("/")
    } else {
        parts.last().map(|s| s.to_string()).unwrap_or_default()
    }
}

/// Files for one statusline command on a non-Claude tool: its scripts, and
/// the statusline entry merged into the tool's settings (wrapped by the stdin
/// adapter). Falls back to the global file when the tool keeps the statusline
/// only there (Cursor CLI, Qwen, Droid, Copilot).
pub fn statusline_files(
    c: &Component,
    target: &TargetAdapter,
    scope: Scope,
    ctx: &ConvertCtx,
    command: &str,
    padding: Option<i64>,
    supporting: &[SupportFile],
) -> Result<Vec<PlannedFile>> {
    let tid = target.id.as_str();
    let mut notes = Vec::new();
    let eff = if kind_path(target, "statusline", scope, ctx).is_some() {
        scope
    } else if target.supports_scope(Scope::Global)
        && target
            .path_template("statusline", Scope::Global, ctx.env.os)
            .is_some()
    {
        notes.push(format!(
            "{} keeps the statusline in its user settings only; written there",
            target.name
        ));
        Scope::Global
    } else {
        return Err(AgentkitError::new(
            "AGENTKIT_NO_PATH",
            format!("{} has no statusline location", target.name),
        ));
    };
    let file = kind_path(target, "statusline", eff, ctx).ok_or_else(|| {
        AgentkitError::new(
            "AGENTKIT_NO_PATH",
            format!("{} has no statusline file", target.name),
        )
    })?;
    let mut out = Vec::new();
    let mut cmd = command.to_string();
    for sf in supporting {
        if sf.source.starts_with("inline:") {
            continue;
        }
        let bytes = c.file(&sf.source).map(|f| f.bytes.clone()).ok_or_else(|| {
            AgentkitError::new(
                "AGENTKIT_PARSE",
                format!("support file `{}` is missing", sf.source),
            )
        })?;
        let (abs, shown) = if target
            .path_template("statusline_scripts", eff, ctx.env.os)
            .is_some()
        {
            place_support(
                &sf.destination,
                target,
                "statusline_scripts",
                Scope::Global,
                ctx,
            )
            .ok_or_else(|| AgentkitError::new("AGENTKIT_NO_PATH", "statusline script location"))?
        } else {
            let dir = scripts_dir(target, eff, ctx).ok_or_else(|| {
                AgentkitError::new("AGENTKIT_NO_PATH", "statusline script location")
            })?;
            let abs = script_rel(&sf.destination)
                .split('/')
                .fold(dir, |p, s| p.join(s));
            let shown = abs.display().to_string();
            (abs, shown)
        };
        let mut pf = PlannedFile::write(tid, c, abs, bytes, "statusline script");
        pf.executable = sf.executable;
        out.push(pf);
        let from = sf.destination.trim_start_matches("./").to_string();
        cmd = rewrite_command(&cmd, &from, &shown);
        if let Some(home_rel) = from.strip_prefix("~/") {
            cmd = rewrite_command(&cmd, &format!("~/{home_rel}"), &shown);
        }
    }
    let losses: Vec<String> = Vec::new();
    notes.push(format!(
        "the command runs behind omniget-hook-shim, which gives it Claude's statusline JSON ({} sends its own fields); it must stay at {}",
        target.name,
        super::hook_common::shim_path(ctx.env).display()
    ));
    let final_cmd = wrap_command(ctx.env, tid, &cmd);
    let shape = target
        .statusline
        .as_ref()
        .and_then(|s| s.shape.clone())
        .unwrap_or_else(|| "claude".into());
    let mut v = Map::new();
    if shape != "droid" {
        v.insert("type".into(), json!("command"));
    }
    v.insert("command".into(), json!(final_cmd));
    if let Some(p) = padding {
        v.insert("padding".into(), json!(p));
    }
    let key: Vec<Seg> = target
        .statusline
        .as_ref()
        .filter(|s| !s.key.is_empty())
        .map(|s| keys(s.key.iter().map(|k| k.as_str())))
        .unwrap_or_else(|| keys(["statusLine"]));
    let mut pf = PlannedFile::merge(
        tid,
        c,
        file.clone(),
        super::format_for_path(&file),
        vec![PatchOp::Set {
            path: key,
            value: Value::Object(v),
            rename_at: None,
        }],
        "statusline",
    );
    pf.commands = vec![final_cmd];
    pf.losses = losses;
    pf.notes = notes;
    let mut all = vec![pf];
    all.extend(out);
    Ok(all)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapts_sparse_input_to_claude_shape() {
        let v = adapt_stdin(
            "cursor",
            &json!({"model": {"display_name": "GPT-5"}, "context_window": {"used_percentage": 41.5}, "cwd": "/w"}),
            None,
        );
        assert_eq!(v["model"]["display_name"], "GPT-5");
        assert_eq!(v["model"]["id"], "GPT-5");
        assert_eq!(v["workspace"]["current_dir"], "/w");
        assert_eq!(v["context_window"]["remaining_percentage"], 58.5);
        assert_eq!(v["cost"]["total_cost_usd"], 0.0);
        assert_eq!(v["output_style"]["name"], "default");
        let s = adapt_stdin(
            "droid",
            &json!({"model": "claude-x", "sessionId": "s1"}),
            Some(Path::new("/p")),
        );
        assert_eq!(v["cwd"], "/w");
        assert_eq!(s["model"]["display_name"], "claude-x");
        assert_eq!(s["session_id"], "s1");
        assert_eq!(s["workspace"]["project_dir"], "/p");
    }

    #[test]
    fn wrap_and_unwrap() {
        let env = Env::sandbox(Path::new("/h"), crate::core::agentkit::Os::Macos);
        let w = wrap_command(&env, "cursor", "python3 /h/.cursor/it's.py");
        assert!(
            w.ends_with("--tool cursor --statusline -- 'python3 /h/.cursor/it'\\''s.py'"),
            "{w}"
        );
        assert_eq!(unwrap_command(&w).unwrap(), "python3 /h/.cursor/it's.py");
        assert_eq!(
            script_rel(".claude/scripts/context-monitor.py"),
            "context-monitor.py"
        );
    }
}
