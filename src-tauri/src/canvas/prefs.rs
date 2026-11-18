use omniget_core::core::canvas::model::{CanvasNode, NodeKind};
use omniget_core::models::settings::{canvas_token, AgentPreset, CanvasSettings};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

pub const EVENT: &str = "canvas://settings";

pub fn get(app: &AppHandle) -> CanvasSettings {
    crate::storage::config::load_settings(app).canvas
}

pub fn replace(app: &AppHandle, mut canvas: CanvasSettings) -> Result<CanvasSettings, String> {
    canvas.normalize();
    let next = canvas.clone();
    crate::storage::config::update_settings(app, |all| all.canvas = next)
        .map_err(|e| format!("Save: {e}"))?;
    let _ = app.emit(EVENT, &canvas);
    super::power::refresh(app);
    Ok(canvas)
}

fn allowed(path: &[String]) -> Option<&'static [&'static str]> {
    let p: Vec<&str> = path.iter().map(String::as_str).collect();
    Some(match p.as_slice() {
        ["background"] => CanvasSettings::BACKGROUNDS,
        ["zoom_modifier"] => CanvasSettings::ZOOM_MODIFIERS,
        ["selection_style"] => CanvasSettings::SELECTION_STYLES,
        ["node_material"] => CanvasSettings::MATERIALS,
        ["cord_style"] => CanvasSettings::CORD_STYLES,
        ["rope_routing"] => CanvasSettings::ROUTINGS,
        ["power", "prevent_sleep"] => CanvasSettings::SLEEP_MODES,
        ["defaults", "note", "render"] => &["formatted", "raw"],
        ["defaults", "agent_placement"] => CanvasSettings::AGENT_PLACEMENTS,
        ["defaults", "portal_placement"] => CanvasSettings::PORTAL_PLACEMENTS,
        ["images", "quality"] => CanvasSettings::IMAGE_QUALITIES,
        ["performance", "rendering"] => CanvasSettings::RENDERINGS,
        _ => return None,
    })
}

fn preset_value(v: Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(
            m.into_iter()
                .map(|(k, v)| {
                    let k = if k == "env" { k } else { canvas_token(&k) };
                    (k, v)
                })
                .collect(),
        ),
        Value::String(cmd) => json!({ "command": cmd }),
        other => other,
    }
}

fn size_value(v: Value) -> Value {
    match &v {
        Value::Object(m) => match (
            m.get("w").and_then(Value::as_f64),
            m.get("h").and_then(Value::as_f64),
        ) {
            (Some(w), Some(h)) => json!(format!("{w}x{h}")),
            _ => v,
        },
        Value::Array(a) if a.len() == 2 => match (a[0].as_f64(), a[1].as_f64()) {
            (Some(w), Some(h)) => json!(format!("{w}x{h}")),
            _ => v,
        },
        _ => v,
    }
}

pub fn set_path(app: &AppHandle, path: &[String], value: Value) -> Result<Value, String> {
    if path.is_empty() {
        let c: CanvasSettings =
            serde_json::from_value(value).map_err(|e| format!("SETTING_INVALID: {e}"))?;
        return serde_json::to_value(replace(app, c)?).map_err(|e| e.to_string());
    }
    let dotted = path.join(".");
    if let Some(ok) = allowed(path) {
        let t = value.as_str().map(canvas_token).unwrap_or_default();
        let t = match (path[0].as_str(), t.as_str()) {
            ("zoom_modifier", "option" | "opt") => "alt".to_string(),
            ("zoom_modifier", "control") => "ctrl".to_string(),
            ("zoom_modifier", "command" | "meta") => "cmd".to_string(),
            _ => t,
        };
        if !ok.contains(&t.as_str()) {
            return Err(format!(
                "SETTING_INVALID: canvas.{dotted} must be one of {}",
                ok.join(", ")
            ));
        }
        if path[0] == "zoom_modifier" && t == "cmd" {
            return Err(
                "SETTING_CONFLICT: ⌘ + scroll switches workspaces; pick alt or ctrl for zoom"
                    .into(),
            );
        }
    }
    let mut value = value;
    let leaf = path.last().map(String::as_str).unwrap_or("");
    if leaf == "size" {
        value = size_value(value);
    }
    let mut doc = serde_json::to_value(get(app)).map_err(|e| e.to_string())?;
    let in_shortcuts = path[0] == "shortcuts" && path.len() == 2;
    if in_shortcuts {
        let combo = shortcut_combo(value.as_str().unwrap_or(""));
        if let Some(map) = doc["shortcuts"].as_object_mut() {
            for (k, v) in map.iter_mut() {
                if k != &path[1]
                    && !combo.is_empty()
                    && shortcut_combo(v.as_str().unwrap_or("")) == combo
                {
                    *v = json!("");
                }
            }
        }
        value = json!(combo);
    }
    let in_presets = path[0] == "agent_presets";
    if in_presets && path.len() == 2 {
        value = preset_value(value);
    }
    {
        let mut cur = &mut doc;
        for (i, seg) in path.iter().enumerate() {
            let last = i + 1 == path.len();
            let obj = cur
                .as_object_mut()
                .ok_or_else(|| format!("SETTING_UNKNOWN: canvas.{dotted}"))?;
            if last {
                if in_presets && path.len() == 2 && value.is_null() {
                    obj.remove(seg);
                } else if obj.contains_key(seg) || (in_presets && path.len() <= 3) || in_shortcuts {
                    obj.insert(seg.clone(), value.clone());
                } else {
                    return Err(format!("SETTING_UNKNOWN: canvas.{dotted}"));
                }
                break;
            }
            if !obj.contains_key(seg) {
                if in_presets && i == 1 {
                    obj.insert(seg.clone(), json!({}));
                } else {
                    return Err(format!("SETTING_UNKNOWN: canvas.{dotted}"));
                }
            }
            cur = obj.get_mut(seg).expect("inserted");
        }
    }
    if in_presets && path.len() >= 2 {
        let name = &path[1];
        let has_cmd = doc["agent_presets"][name]["command"]
            .as_str()
            .is_some_and(|c| !c.trim().is_empty());
        if !doc["agent_presets"][name].is_null() && !has_cmd {
            return Err(format!("SETTING_INVALID: preset '{name}' needs a command"));
        }
    }
    let c: CanvasSettings = serde_json::from_value(doc)
        .map_err(|e| format!("SETTING_INVALID: canvas.{dotted}: {e}"))?;
    let after = serde_json::to_value(replace(app, c)?).map_err(|e| e.to_string())?;
    let mut now = &after;
    for seg in path {
        now = &now[seg.as_str()];
    }
    Ok(now.clone())
}

pub fn shortcut_combo(raw: &str) -> String {
    let parts: Vec<String> = raw
        .split('+')
        .map(|p| p.trim().to_lowercase())
        .filter(|p| !p.is_empty())
        .collect();
    let Some((key, mods)) = parts.split_last() else {
        return String::new();
    };
    let has = |names: &[&str]| mods.iter().any(|m| names.contains(&m.as_str()));
    let mut out: Vec<&str> = Vec::new();
    if has(&[
        "mod", "cmd", "command", "meta", "primary", "ctrl", "control",
    ]) {
        out.push("mod");
    }
    if has(&["alt", "option", "opt"]) {
        out.push("alt");
    }
    if has(&["shift"]) {
        out.push("shift");
    }
    let key = match key.as_str() {
        "right" => "arrowright",
        "left" => "arrowleft",
        "up" => "arrowup",
        "down" => "arrowdown",
        k => k,
    };
    out.push(key);
    out.join("+")
}

pub fn default_size(app: &AppHandle, kind: &str) -> Option<(f64, f64)> {
    let c = get(app);
    let s = match kind {
        "terminal" | "agent" => &c.defaults.terminal.size,
        "note" => &c.defaults.note.size,
        "text" => &c.defaults.text.size,
        "portal" => &c.defaults.portal.size,
        _ => return None,
    };
    omniget_core::models::settings::parse_canvas_size(s)
}

pub fn preset(app: &AppHandle, name: &str) -> Option<(String, AgentPreset)> {
    let c = get(app);
    if let Some(p) = c.agent_presets.get(name) {
        return Some((name.to_string(), p.clone()));
    }
    c.agent_presets
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(k, p)| (k.clone(), p.clone()))
}

pub fn preset_agent(p: &AgentPreset) -> &'static str {
    let prog = p.command.split_whitespace().next().unwrap_or("");
    let base = std::path::Path::new(prog)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .trim_end_matches(".exe")
        .to_ascii_lowercase();
    match base.as_str() {
        "claude" => "claude",
        "codex" => "codex",
        "gemini" => "gemini",
        "opencode" => "opencode",
        "ollama" => "ollama",
        _ => "shell",
    }
}

pub fn apply_preset(n: &mut CanvasNode, name: &str, p: &AgentPreset) {
    let mut command: Vec<String> = p.command.split_whitespace().map(str::to_string).collect();
    command.extend(p.args.iter().cloned());
    n.data["command"] = json!(command);
    n.data["agent"] = json!(preset_agent(p));
    n.data["presetId"] = json!(name);
    if !p.env.is_empty() {
        n.data["env"] = json!(p.env);
    }
    if let Some(cwd) = p.cwd.as_deref().filter(|c| !c.trim().is_empty()) {
        n.data["cwd"] = json!(cwd);
    }
    if p.lead {
        n.data["isManager"] = json!(true);
    }
    if p.start_in_chat_mode {
        n.data["prefersChatMode"] = json!(true);
    }
    if let Some(c) = p.color.as_deref().filter(|c| !c.is_empty()) {
        n.color = Some(c.to_string());
    }
    if let Some(i) = p.icon.as_deref().filter(|c| !c.is_empty()) {
        n.icon = Some(i.to_string());
    }
    n.kind = NodeKind::Agent;
}

#[tauri::command]
pub fn canvas_settings_replace(
    app: AppHandle,
    canvas: CanvasSettings,
) -> Result<CanvasSettings, String> {
    replace(&app, canvas)
}
