//! Full JSON Schemas of the nested arguments of the canvas tools (the serde
//! structs of `omniget_core::core::canvas::*`, written by hand), and the
//! flattening of the common forms (`partitura_create {name, nodeIds}`,
//! `routine_create {nodeId, prompt, every}`) into those structs.
//!
//! Rule checked by `super::tests`: every object in an input schema has
//! `properties` or a typed `additionalProperties`; a free-form map (patches,
//! payloads, rpc args) is marked `additionalProperties: true` and listed in
//! [`FREE_FORM`] with the reason.

use serde_json::{json, Map, Value};

fn obj(props: Value, required: &[&str], desc: &str) -> Value {
    json!({"type": "object", "properties": props, "required": required, "additionalProperties": false, "description": desc})
}

fn st(desc: &str) -> Value {
    json!({"type": "string", "description": desc})
}

fn num(desc: &str) -> Value {
    json!({"type": "number", "description": desc})
}

fn boolean(desc: &str) -> Value {
    json!({"type": "boolean", "description": desc})
}

fn strs(desc: &str) -> Value {
    json!({"type": "array", "items": {"type": "string"}, "description": desc})
}

/// A free-form map (see [`FREE_FORM`]).
pub fn free(desc: &str) -> Value {
    json!({"type": "object", "additionalProperties": true, "description": desc})
}

/// A map of strings (`{"KEY": "value"}`).
pub fn string_map(desc: &str) -> Value {
    json!({"type": "object", "additionalProperties": {"type": "string"}, "description": desc})
}

pub fn point(desc: &str) -> Value {
    obj(json!({"x": num("X"), "y": num("Y")}), &["x", "y"], desc)
}

pub fn rect(desc: &str) -> Value {
    obj(
        json!({"x": num("Left"), "y": num("Top"), "w": num("Width"), "h": num("Height")}),
        &["x", "y", "w", "h"],
        desc,
    )
}

fn size() -> Value {
    obj(
        json!({"w": num("Width"), "h": num("Height")}),
        &["w", "h"],
        "Size",
    )
}

fn anchor(desc: &str) -> Value {
    obj(
        json!({"drawingId": st("Drawing the arrow end is attached to"), "edge": {"type": "string", "enum": ["top", "bottom", "left", "right"]}}),
        &["drawingId", "edge"],
        desc,
    )
}

/// `model::Drawing` (id optional on add).
pub fn drawing() -> Value {
    obj(
        json!({
            "id": st("Id (generated when absent)"),
            "tool": {"type": "string", "enum": omniget_core::core::canvas::model::DRAWING_TOOLS, "description": "Drawing tool; shapes fill the box of their points (or size), text uses textContent", "examples": ["pen", "arrow", "rectangle"]},
            "points": {"type": "array", "items": {"type": "number"}, "description": "Flat [x, y, ...] local to origin"},
            "origin": point("Top-left of the drawing in world units"),
            "originalSize": size(), "size": size(),
            "color": st("Color #rrggbb"), "lineWidth": num("Stroke width"), "opacity": num("0..1"),
            "z": num("Stacking order"), "locked": boolean("Locked"), "rotation": num("Degrees"),
            "textContent": st("Text of a text drawing or inside a shape"), "textFontSize": num("Font size"),
            "sourceConnection": anchor("Arrow start attached to a shape"), "targetConnection": anchor("Arrow end attached to a shape"),
            "strokePattern": {"type": "string", "enum": ["solid", "dashed", "dotted"]},
            "fillStyle": {"type": "string", "enum": ["none", "noFill", "hachure", "crossHatch", "tint", "solid", "solidOpaque"]},
            "cornerRadius": num("Corner radius"), "controlPointOffset": num("Curve of a line/arrow"), "groupId": st("Group id")
        }),
        &["tool", "origin"],
        "Drawing, e.g. {\"tool\":\"rectangle\",\"origin\":{\"x\":0,\"y\":0},\"points\":[0,0,200,120],\"color\":\"#ff8800\"}",
    )
}

/// `routines::Schedule`.
pub fn schedule() -> Value {
    json!({
        "description": "When it runs: {\"kind\":\"interval\",\"seconds\":30} or {\"kind\":\"interval\",\"minutes\":30} (minimum 10 s; starts right away) | {\"kind\":\"once\",\"at\":\"2026-10-01T09:00:00-03:00\"} | {\"kind\":\"daily\",\"time\":\"09:00\"} or {\"kind\":\"daily\",\"hour\":9,\"minute\":0} | {\"kind\":\"weekly\",\"days\":[1,3,5],\"time\":\"09:00\"} (0 = Sunday) or Maestri's {\"kind\":\"weekly\",\"weekdays\":[2,4,6],\"hour\":9,\"minute\":0} (1 = Sunday)",
        "oneOf": [
            obj(json!({"kind": {"type": "string", "enum": ["once"]}, "at": st("RFC 3339")}), &["kind", "at"], "Once"),
            obj(json!({"kind": {"type": "string", "enum": ["interval"]}, "minutes": {"type": "integer", "minimum": 0}, "seconds": {"type": "integer", "minimum": 10}, "startAt": st("RFC 3339 (default: now)")}), &["kind"], "Every N seconds or minutes"),
            obj(json!({"kind": {"type": "string", "enum": ["daily"]}, "time": st("HH:MM local"), "hour": {"type": "integer", "minimum": 0, "maximum": 23}, "minute": {"type": "integer", "minimum": 0, "maximum": 59}}), &["kind"], "Every day"),
            obj(json!({"kind": {"type": "string", "enum": ["weekly"]}, "days": {"type": "array", "items": {"type": "integer", "minimum": 0, "maximum": 6}}, "weekdays": {"type": "array", "items": {"type": "integer", "minimum": 1, "maximum": 7}, "description": "Maestri: 1 = Sunday"}, "time": st("HH:MM local"), "hour": {"type": "integer", "minimum": 0, "maximum": 23}, "minute": {"type": "integer", "minimum": 0, "maximum": 59}}), &["kind"], "Weekly"),
        ]
    })
}

/// `routines::End`.
pub fn end() -> Value {
    json!({
        "description": "When it stops: {\"kind\":\"never\"} (default) | {\"kind\":\"afterCount\",\"count\":3} | {\"kind\":\"onDate\",\"date\":\"2026-12-31\"}",
        "oneOf": [
            obj(json!({"kind": {"type": "string", "enum": ["never"]}}), &["kind"], "Never"),
            obj(json!({"kind": {"type": "string", "enum": ["afterCount"]}, "count": {"type": "integer", "minimum": 1}}), &["kind", "count"], "After N runs"),
            obj(json!({"kind": {"type": "string", "enum": ["onDate"]}, "date": st("Date or RFC 3339")}), &["kind", "date"], "On a date"),
        ]
    })
}

/// `routines::RoutineInput`.
pub fn routine_input(with_id: bool) -> Value {
    let mut required = vec!["name", "prompt", "schedule"];
    if with_id {
        required.push("id");
    }
    obj(
        json!({
            "id": st("Routine id (update)"), "name": st("Name"), "workspaceId": st("Workspace (default: the target's)"),
            "floorId": st("Floor (default: the target's)"), "nodeId": st("Target terminal (id or unique title); absent = reminder"),
            "prompt": st("Text typed into the target; a line with only && splits it into messages sent in turn; {{output}} = preRun stdout"),
            "schedule": schedule(), "end": end(), "preRun": st("Shell script run first (non-zero exit skips)"),
            "skipIfBusy": boolean("Skip while the target works (default true)"), "notify": boolean("Also send a system notification"),
            "enabled": boolean("Enabled (default true)"), "createdBy": st("Author")
        }),
        &required,
        "RoutineInput (nested form; the flat arguments are simpler)",
    )
}

fn budget() -> Value {
    obj(
        json!({"maxUsd": num("Max spend"), "maxTokens": {"type": "integer", "minimum": 0}, "maxMinutes": {"type": "integer", "minimum": 0}}),
        &[],
        "Budget",
    )
}

fn permissions() -> Value {
    obj(
        json!({"mode": {"type": "string", "enum": ["supervised", "acceptEdits", "auto", "full"]}, "allowRaw": boolean("Cords may type raw keys"), "allowEvaluate": boolean("Portals may run JS")}),
        &[],
        "Permissions",
    )
}

fn setup_spec() -> Value {
    obj(
        json!({"setup": st("Setup script"), "run": st("Run script"), "teardown": st("Teardown script")}),
        &[],
        "Floor scripts",
    )
}

fn commands_map() -> Value {
    json!({"type": "object", "additionalProperties": {"type": ["string", "null"]}, "description": "Per terminal node id: new startup command, or null to drop it"})
}

/// `partituras::SaveRequest` (nested form).
pub fn save_request() -> Value {
    obj(
        json!({
            "selection": obj(json!({"nodeIds": strs("Nodes"), "drawingIds": strs("Drawings")}), &[], "What to save"),
            "options": obj(json!({
                "name": st("Name (default: the first node's title, else \"Partitura\")"), "summary": st("Description"), "icon": st("Icon"), "color": st("Color"),
                "includeStartupCommands": boolean("Keep terminal commands (default true)"), "blankNotes": boolean("Ship notes empty")
            }), &[], "Options"),
            "scope": st("\"global\" or a workspace id (default: the source workspace)"),
            "updateExisting": st("Replace this partitura's layout, keep its identity"),
            "includeChat": boolean("Carry the terminals' chat history (format 2)"),
            "commands": commands_map(), "budget": budget(), "permissions": permissions(), "setup": setup_spec()
        }),
        &[],
        "SaveRequest (nested form; the flat arguments are simpler)",
    )
}

/// `partituras::StampOptions`.
pub fn stamp_options() -> Value {
    obj(
        json!({
            "commands": commands_map(),
            "roles": {"type": "object", "additionalProperties": {"type": "string", "enum": ["keepMine", "useTheirs", "rename"]}, "description": "Per role id of the partitura: what to do on a conflict"},
            "applyHooks": boolean("Append the partitura's scripts to the floor hooks"),
            "acceptChat": boolean("Copy a format-2 partitura's chat history into the new terminals"),
            "origin": st("Origin recorded on the writes")
        }),
        &[],
        "StampOptions",
    )
}

fn hook_list(desc: &str) -> Value {
    json!({"type": "array", "description": desc, "items": obj(json!({"id": st("Id (generated)"), "command": st("Shell command"), "name": st("Label"), "enabled": boolean("Enabled (default true)")}), &["command"], "Hook")})
}

/// `floors::FloorHooks`.
pub fn floor_hooks() -> Value {
    obj(
        json!({"setup": hook_list("Run after a floor is created"), "run": hook_list("Run on demand"), "teardown": hook_list("Run before a floor is deleted"), "autoRunSetup": boolean("Run setup right after creation (default true)")}),
        &[],
        "Floor hooks, e.g. {\"setup\":[{\"command\":\"pnpm install\"}],\"run\":[{\"command\":\"pnpm dev\"}],\"autoRunSetup\":true}",
    )
}

/// `companion::CompanionSettings`.
pub fn companion_settings() -> Value {
    obj(
        json!({
            "enabled": boolean("Omni on"), "minWorkSecs": {"type": "integer", "minimum": 5, "maximum": 3600, "description": "Seconds of work before a finished turn is summarized"},
            "modelMode": {"type": "string", "enum": ["auto", "local", "cheap", "heuristic"]}, "ollamaUrl": st("Ollama URL"),
            "ollamaModel": {"type": ["string", "null"], "description": "Pinned Ollama model"}, "cheapAgentId": {"type": ["string", "null"], "description": "Agent whose cheapest model is the fallback"},
            "petBalloon": boolean("Show summaries in the pet's balloon"), "onDone": boolean("Summarize finished turns"), "onBlocked": boolean("Tell when an agent needs you"),
            "onFailed": boolean("Tell when an agent fails"), "language": {"type": ["string", "null"], "description": "Summary language (en, pt...)"},
            "cliFallback": boolean("Without a local model, ask an installed agent CLI headless (cheapest model)"), "cliAgent": {"type": ["string", "null"], "enum": ["claude", "codex", "gemini", null], "description": "Agent CLI for the fallback (null = the one used on the canvas)"}
        }),
        &[],
        "Omni settings (omni_settings_get shape)",
    )
}

/// Appearance of a terminal node (stored as given; the known fields).
pub fn appearance() -> Value {
    json!({
        "type": ["object", "null"],
        "properties": {
            "theme": {"type": "string", "description": "system, dark, light or custom"}, "customThemeId": st("Theme id (e.g. dracula)"),
            "fontFamily": st("Terminal font"), "fontSize": num("Terminal font size"), "accentColor": st("Accent #rrggbb"),
            "icon": st("Icon"), "chatFontFamily": st("Chat font"), "chatFontSize": num("Chat font size")
        },
        "additionalProperties": true,
        "description": "Per-terminal appearance; absent fields follow the global settings"
    })
}

// ── Flat forms ─────────────────────────────────────────────────────────

/// Seconds of an `every` shorthand: `30s`, `5m`, `2h`, `1d`, `15` (minutes).
/// Below the routine minimum it is refused, never silently rounded.
pub fn every_seconds(v: &str) -> Result<u32, String> {
    let t = v.trim().to_lowercase();
    let (n, unit) = t.split_at(t.find(|c: char| !c.is_ascii_digit()).unwrap_or(t.len()));
    let n: u32 = n
        .parse()
        .map_err(|_| format!("INVALID_ARGUMENT: every '{v}' (use 30s, 5m, 2h, 1d)"))?;
    let secs = match unit.trim() {
        "" | "m" | "min" | "mins" | "minute" | "minutes" => n * 60,
        "s" | "sec" | "secs" | "seconds" => n,
        "h" | "hour" | "hours" => n * 3600,
        "d" | "day" | "days" => n * 86_400,
        _ => {
            return Err(format!(
                "INVALID_ARGUMENT: every '{v}' (use 30s, 5m, 2h, 1d)"
            ))
        }
    };
    let min = omniget_core::core::canvas::routines::MIN_INTERVAL_SECS;
    if secs < min {
        return Err(format!(
            "INVALID_ARGUMENT: every '{v}' is below the minimum of {min} seconds"
        ));
    }
    Ok(secs)
}

/// Builds `RoutineInput` JSON from the flat arguments (nested `routine`
/// wins when given).
pub fn routine_from_flat(a: &Value) -> Result<Value, String> {
    if let Some(r) = a.get("routine").filter(|r| r.is_object()) {
        return Ok(r.clone());
    }
    let mut r = Map::new();
    for k in [
        "id",
        "name",
        "workspaceId",
        "floorId",
        "nodeId",
        "prompt",
        "preRun",
        "createdBy",
    ] {
        if let Some(v) = a.get(k).filter(|v| v.is_string()) {
            r.insert(k.into(), v.clone());
        }
    }
    if let Some(v) = a.get("preRunScript").filter(|v| v.is_string()) {
        r.insert("preRun".into(), v.clone());
    }
    for k in ["skipIfBusy", "notify", "enabled"] {
        if let Some(v) = a.get(k).filter(|v| v.is_boolean()) {
            r.insert(k.into(), v.clone());
        }
    }
    let prompt = r.get("prompt").and_then(Value::as_str).unwrap_or("");
    if prompt.trim().is_empty() {
        return Err("MISSING_ARGUMENT: prompt".into());
    }
    if !r.contains_key("name") {
        let first: String = prompt
            .lines()
            .next()
            .unwrap_or("")
            .chars()
            .take(40)
            .collect();
        r.insert("name".into(), json!(first.trim()));
    }
    let schedule = if let Some(s) = a.get("schedule").filter(|v| v.is_object()) {
        s.clone()
    } else if let Some(e) = a.get("every").and_then(Value::as_str) {
        {
            let secs = every_seconds(e)?;
            json!({"kind": "interval", "minutes": secs / 60, "seconds": secs})
        }
    } else if let Some(d) = a.get("daily").and_then(Value::as_str) {
        json!({"kind": "daily", "time": d.trim().trim_start_matches("daily").trim()})
    } else if let Some(at) = a.get("at").and_then(Value::as_str) {
        json!({"kind": "once", "at": at})
    } else if let Some(w) = a.get("weekly").filter(|v| v.is_object()) {
        json!({"kind": "weekly", "days": w["days"], "time": w["time"]})
    } else {
        return Err("MISSING_ARGUMENT: every, daily, at, weekly or schedule".into());
    };
    r.insert("schedule".into(), schedule);
    if let Some(end) = a.get("end").filter(|v| v.is_object()) {
        r.insert("end".into(), end.clone());
    } else if let Some(n) = a.get("endAfter").and_then(Value::as_u64) {
        r.insert("end".into(), json!({"kind": "afterCount", "count": n}));
    } else if let Some(d) = a.get("endOn").and_then(Value::as_str) {
        r.insert("end".into(), json!({"kind": "onDate", "date": d}));
    }
    Ok(Value::Object(r))
}

/// Builds `SaveRequest` JSON from the flat arguments (nested `request`
/// wins, with `options` defaulted). `selection` = the node/drawing ids
/// already resolved by the caller; `first_title` names it when no name.
pub fn save_request_from_flat(
    a: &Value,
    selection: Option<(Vec<String>, Vec<String>)>,
    first_title: Option<String>,
) -> Value {
    let mut req = a
        .get("request")
        .filter(|r| r.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));
    if req.get("selection").is_none_or(|s| !s.is_object()) {
        req["selection"] = json!({});
    }
    if let Some((nodes, drawings)) = selection {
        req["selection"]["nodeIds"] = json!(nodes);
        if !drawings.is_empty() {
            req["selection"]["drawingIds"] = json!(drawings);
        }
    }
    if req.get("options").is_none_or(|o| !o.is_object()) {
        req["options"] = json!({});
    }
    let flat_opts = [
        ("name", "name"),
        ("description", "summary"),
        ("summary", "summary"),
        ("icon", "icon"),
        ("color", "color"),
        ("includeStartupCommands", "includeStartupCommands"),
        ("blankNotes", "blankNotes"),
    ];
    for (from, to) in flat_opts {
        if let Some(v) = a.get(from).filter(|v| !v.is_null()) {
            req["options"][to] = v.clone();
        }
    }
    for k in [
        "scope",
        "updateExisting",
        "includeChat",
        "commands",
        "budget",
        "permissions",
        "setup",
    ] {
        if let Some(v) = a.get(k).filter(|v| !v.is_null()) {
            req[k] = v.clone();
        }
    }
    let named = req["options"]["name"]
        .as_str()
        .is_some_and(|n| !n.trim().is_empty());
    if !named {
        req["options"]["name"] = json!(first_title
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| "Partitura".into()));
    }
    req
}
