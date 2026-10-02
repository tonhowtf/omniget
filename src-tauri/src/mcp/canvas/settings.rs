use omniget_core::models::settings::canvas_token;
use serde_json::{json, Value};
use tauri::AppHandle;

const CANVAS_ROOTS: &[&str] = &[
    "background",
    "zoom_step",
    "zoom_modifier",
    "right_drag_pan",
    "focus_on_hover",
    "selection_style",
    "node_material",
    "cord_style",
    "rope_routing",
    "spell_check",
    "default_agent",
    "defaults",
    "terminal",
    "notifications",
    "power",
    "agent_presets",
    "shortcuts",
    "skills",
    "images",
    "performance",
];

pub fn canvas_path(key: &str) -> Option<Vec<String>> {
    let raw: Vec<&str> = key.trim().split('.').filter(|s| !s.is_empty()).collect();
    if raw.is_empty() {
        return None;
    }
    let mut segs: Vec<String> = Vec::new();
    let mut i = 0;
    if canvas_token(raw[0]) == "canvas" {
        i = 1;
        if raw.len() == 1 {
            return Some(vec![]);
        }
    }
    let first = canvas_token(raw[i]);
    let (head, rest_from) = match first.as_str() {
        "agent" if raw.get(i + 1).map(|s| canvas_token(s)).as_deref() == Some("presets") => {
            ("agent_presets".to_string(), i + 2)
        }
        "agent" if raw.get(i + 1).map(|s| canvas_token(s)).as_deref() == Some("default") => {
            ("default_agent".to_string(), i + 2)
        }
        "presets" | "agent_presets" => ("agent_presets".to_string(), i + 1),
        "grid" | "canvas_background" => ("background".to_string(), i + 1),
        "prevent_sleep" => {
            return Some(vec!["power".into(), "prevent_sleep".into()]);
        }
        other => (other.to_string(), i + 1),
    };
    if i == 0 && !CANVAS_ROOTS.contains(&head.as_str()) {
        return None;
    }
    segs.push(head.clone());
    for (k, s) in raw[rest_from..].iter().enumerate() {
        let keep = head == "agent_presets"
            && (k == 0 || (k == 2 && segs.get(2).map(String::as_str) == Some("env")));
        segs.push(if keep { s.to_string() } else { canvas_token(s) });
    }
    Some(segs)
}

fn side_get(key: &str) -> Option<Result<Value, String>> {
    let k = key.trim();
    if let Some(rest) = k.strip_prefix("environments") {
        let p = serde_json::to_value(omniget_core::core::canvas::envs::providers(
            &omniget_core::core::canvas::store::shared(),
        ))
        .unwrap_or(Value::Null);
        let rest = rest.trim_start_matches('.');
        let v = if rest.is_empty() {
            Some(p)
        } else {
            p.get(rest).cloned()
        };
        return Some(
            v.map(|v| json!({ "key": k, "value": v }))
                .ok_or_else(|| format!("SETTING_UNKNOWN: {k}")),
        );
    }
    if matches!(
        canvas_token(k).as_str(),
        "remote.private_only" | "remote.privateonly"
    ) || k == "remote.privateOnly"
    {
        let v = crate::local_bridge_remote::hub().config().private_only;
        return Some(Ok(json!({ "key": k, "value": v })));
    }
    None
}

fn side_set(key: &str, value: &Value) -> Option<Result<Value, String>> {
    let k = key.trim();
    if let Some(rest) = k.strip_prefix("environments") {
        let rest = rest.trim_start_matches('.');
        let patch = if rest.is_empty() {
            value.clone()
        } else {
            json!({ rest: value })
        };
        let r = omniget_core::core::canvas::envs::set_providers(
            &omniget_core::core::canvas::store::shared(),
            &patch,
        )
        .map(|p| {
            let p = serde_json::to_value(p).unwrap_or(Value::Null);
            let now = if rest.is_empty() {
                p
            } else {
                p.get(rest).cloned().unwrap_or(Value::Null)
            };
            json!({ "key": k, "value": now, "applied": super::super::canvas_ui::same_value(&now, value) })
        });
        return Some(r);
    }
    if matches!(
        canvas_token(k).as_str(),
        "remote.private_only" | "remote.privateonly"
    ) || k == "remote.privateOnly"
    {
        let Some(on) = value.as_bool() else {
            return Some(Err(
                "SETTING_INVALID: remote.private_only is true or false".into()
            ));
        };
        let h = crate::local_bridge_remote::hub();
        h.set_private_only(on);
        return Some(Ok(
            json!({ "key": k, "value": h.config().private_only, "applied": true }),
        ));
    }
    None
}

const LABELS: &str = include_str!("../../../../src/lib/i18n/en.json");

enum Values {
    Toggle,
    Range(u32, u32),
    Choice(&'static [(&'static str, &'static str)]),
}

const DESCRIBED: &[(&str, &str, Option<&str>, Values)] = &[
    (
        "canvas.images.any_zoom",
        "imagesAnyZoom",
        Some("imagesAnyZoomHint"),
        Values::Toggle,
    ),
    (
        "canvas.images.quality",
        "imagesQuality",
        Some("imagesQualityHint"),
        Values::Choice(&[
            ("light", "imagesLight"),
            ("balanced", "imagesBalanced"),
            ("sharp", "imagesSharp"),
        ]),
    ),
    (
        "canvas.images.download_web",
        "imagesDownload",
        Some("imagesDownloadHint"),
        Values::Toggle,
    ),
    (
        "canvas.performance.mounted_terminals",
        "mountedTerminals",
        Some("mountedTerminalsHint"),
        Values::Range(1, 64),
    ),
    (
        "canvas.performance.live_portals",
        "livePortals",
        Some("livePortalsHint"),
        Values::Range(0, 16),
    ),
    (
        "canvas.performance.force_canvas2d",
        "forceCanvas2d",
        Some("forceCanvas2dHint"),
        Values::Toggle,
    ),
    (
        "canvas.zoom_modifier",
        "zoomModifier",
        None,
        Values::Choice(&[("alt", "modAlt"), ("ctrl", "modCtrl"), ("cmd", "modCmd")]),
    ),
    (
        "canvas.right_drag_pan",
        "rightDragPan",
        None,
        Values::Toggle,
    ),
    (
        "canvas.focus_on_hover",
        "focusOnHover",
        None,
        Values::Toggle,
    ),
    ("canvas.spell_check", "spellCheck", None, Values::Toggle),
];

fn label(id: &str) -> String {
    static ALL: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    ALL.get_or_init(|| serde_json::from_str(LABELS).unwrap_or(Value::Null))["canvas"]["settings"]
        [id]
        .as_str()
        .unwrap_or(id)
        .to_string()
}

pub fn describe(app: &AppHandle, key: Option<&str>) -> Result<Value, String> {
    let want = key
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .map(|k| match canvas_path(k) {
            Some(p) if !p.is_empty() => format!("canvas.{}", p.join(".")),
            Some(_) => "canvas".to_string(),
            None => k.to_string(),
        });
    let all = serde_json::to_value(crate::canvas::prefs::get(app)).map_err(|e| e.to_string())?;
    let list: Vec<Value> = DESCRIBED
        .iter()
        .filter(|(k, ..)| {
            want.as_deref()
                .is_none_or(|w| *k == w || k.starts_with(&format!("{w}.")))
        })
        .map(|(k, l, h, values)| {
            let mut cur = &all;
            for seg in k.trim_start_matches("canvas.").split('.') {
                cur = cur.get(seg).unwrap_or(&Value::Null);
            }
            let values = match values {
                Values::Toggle => json!({ "type": "boolean" }),
                Values::Range(min, max) => json!({ "type": "integer", "min": min, "max": max }),
                Values::Choice(opts) => json!({
                    "type": "choice",
                    "options": opts.iter().map(|(v, l)| json!({ "value": v, "label": label(l) })).collect::<Vec<_>>(),
                }),
            };
            json!({
                "key": k,
                "label": label(l),
                "help": h.map(label),
                "values": values,
                "value": cur,
            })
        })
        .collect();
    if list.is_empty() {
        return Err(format!(
            "SETTING_UNKNOWN: no described setting under {} (try canvas.images or canvas.performance)",
            want.unwrap_or_default()
        ));
    }
    Ok(json!({ "settings": list }))
}

pub fn get(app: &AppHandle, key: Option<&str>) -> Result<Value, String> {
    let Some(k) = key.filter(|k| !k.trim().is_empty()) else {
        return super::super::canvas_ui::settings_get(app, None);
    };
    if let Some(r) = side_get(k) {
        return r;
    }
    if let Ok(v) = super::super::canvas_ui::settings_get(app, Some(k)) {
        return Ok(v);
    }
    let Some(path) = canvas_path(k) else {
        return super::super::canvas_ui::settings_get(app, Some(k));
    };
    if path == ["terminal", "themes"] {
        return Ok(
            json!({ "key": k, "path": "canvas.terminal.themes", "value": crate::canvas::term::theme_ids() }),
        );
    }
    let all = serde_json::to_value(crate::canvas::prefs::get(app)).map_err(|e| e.to_string())?;
    let mut v = &all;
    for seg in &path {
        v = v
            .get(seg.as_str())
            .ok_or_else(|| format!("SETTING_UNKNOWN: {k}"))?;
    }
    Ok(
        json!({ "key": k, "path": format!("canvas.{}", path.join(".")).trim_end_matches('.'), "value": v }),
    )
}

pub fn set(app: &AppHandle, key: &str, value: Value) -> Result<Value, String> {
    if let Some(r) = side_set(key, &value) {
        return r;
    }
    let root_known = super::super::canvas_ui::settings_get(app, Some(key)).is_ok();
    match canvas_path(key) {
        Some(path) if !root_known || key.trim().starts_with("canvas") => {
            let now = crate::canvas::prefs::set_path(app, &path, value.clone())?;
            let applied = super::super::canvas_ui::same_value(&now, &value)
                || matches!((&now, &value), (Value::String(a), Value::String(b)) if canvas_token(a) == canvas_token(b))
                || matches!((&now, &value), (Value::String(a), Value::String(b))
                    if path.first().map(String::as_str) == Some("shortcuts")
                        && *a == crate::canvas::prefs::shortcut_combo(b));
            Ok(json!({
                "key": key,
                "path": format!("canvas.{}", path.join(".")),
                "value": now,
                "applied": applied,
            }))
        }
        _ => super::super::canvas_ui::settings_set(app, key, value),
    }
}
