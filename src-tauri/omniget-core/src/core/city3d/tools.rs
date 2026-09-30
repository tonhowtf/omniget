//! The `city_*` tools: any agent (Claude Code, Codex, Gemini, Ollama over
//! the OmniGet MCP, or a native /llm agent) inspects and changes the city
//! through the same pipeline the app uses. `city_codegeo` runs in the app
//! (the geometry crate is not a dependency of the core), the rest here.

use base64::Engine;
use omniget_citygen::program::{CityProgram, PatchOp, Proposal};
use omniget_citygen::Charter;
use serde_json::{json, Value};

use super::api;
use super::store::Author;
use crate::core::llm::tool_table::{BoxFut, CostHint, ToolEntry, ToolImpl, ToolRisk};

fn obj(props: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": props, "required": required })
}

fn region_schema() -> Value {
    json!({ "type": "array", "items": { "type": "integer" }, "minItems": 2, "maxItems": 2, "description": "[rx, ry]; default [0, 0]" })
}

fn ops_schema() -> Value {
    json!({ "type": "array", "items": { "type": "object" }, "description": "patch ops, e.g. {\"op\":\"add_building\",\"id\":\"padaria\",\"lot\":\"auto:zone:centro\",\"intent\":{\"use\":\"loja-sobrado\",\"floors\":2,\"shape\":\"rect\",\"roof\":\"duas-aguas\"}}" })
}

const OPS_HELP: &str = " Ops: add_zone {id,kind,rect,style,floors}, add_street {id,class,width,path}, add_landmark {id,kind,rect}, add_water {id,rect,bridges}, add_building {id,lot:'auto:zone:<z>'|'auto:near:<landmark>'|<lot id>,intent:{use,floors,shape,roof,palette,tower,balcony,awning,label}}, add_props {kind,preset,cells|scatter}, add_fill {zone,use,shape[],roof[]}, set_intent {id,patch}, move {id,lot|path}, remove {id}. Rects are [x,y,w,h] in 2 m tiles of a 64x64 region.";

struct Def {
    name: &'static str,
    description: &'static str,
    schema: Value,
    call: ToolImpl,
    cost: CostHint,
    risk: ToolRisk,
}

fn defs() -> Vec<Def> {
    use CostHint::{Cheap, Local};
    let core = |f: fn(Value) -> BoxFut| ToolImpl::Core(f);
    vec![
        Def {
            name: "city_inspect",
            description: "Read the 3D city: charter, a region's program and revision, the building-code report, valid options (uses, roofs, styles, palettes, free lots) and, with lot or building, that item. Call it first: its revision is the base_revision of city_propose/city_apply.",
            schema: obj(json!({
                "region": region_schema(),
                "lot": { "type": "string", "description": "a lot id to detail" },
                "building": { "type": "string", "description": "a building id to detail" },
                "include_program": { "type": "boolean", "description": "full program JSON (default true)" }
            }), &[]),
            call: core(|a| Box::pin(inspect(a))),
            cost: Cheap,
            risk: ToolRisk::Read,
        },
        Def {
            name: "city_validate",
            description: "Dry-run patch ops on a region: apply on a copy, solve and run the building code. Returns revision, violations ([code,id,measure]+message; errors block) and a summary. Writes nothing.",
            schema: obj(json!({ "region": region_schema(), "ops": ops_schema(), "base_revision": { "type": "string" } }), &["ops"]),
            call: core(|a| Box::pin(validate(a))),
            cost: Local,
            risk: ToolRisk::Read,
        },
        Def {
            name: "city_propose",
            description: "Validate patch ops and keep them as a draft in the city log (shown to people in the inspector). Use city_apply to publish.",
            schema: obj(json!({ "region": region_schema(), "base_revision": { "type": "string" }, "ops": ops_schema(), "reason": { "type": "string", "description": "one sentence for the log" }, "agent": { "type": "string", "description": "your name, for the log" } }), &["base_revision", "ops", "reason"]),
            call: core(|a| Box::pin(propose(a))),
            cost: Local,
            risk: ToolRisk::Reversible,
        },
        Def {
            name: "city_apply",
            description: "Publish patch ops on a region at base_revision (from city_inspect). Zero errors publishes under the 'auto' policy or waits for a person under 'review'; any error rejects with the violations. New buildings start as construction sites. Reuse idempotency_key on retries.",
            schema: obj(json!({ "region": region_schema(), "base_revision": { "type": "string" }, "ops": ops_schema(), "reason": { "type": "string" }, "idempotency_key": { "type": "string" }, "agent": { "type": "string" } }), &["base_revision", "ops", "reason"]),
            call: core(|a| Box::pin(apply(a))),
            cost: Local,
            risk: ToolRisk::Reversible,
        },
        Def {
            name: "city_render",
            description: "PNG picture of a region (view top, n, e, s or w; px 128..1024), optionally with unpublished ops applied, so a model with vision can check what it built.",
            schema: obj(json!({ "region": region_schema(), "view": { "type": "string", "enum": ["top", "n", "e", "s", "w"] }, "px": { "type": "integer", "minimum": 128, "maximum": 1024 }, "ops": ops_schema() }), &[]),
            call: core(|a| Box::pin(render(a))),
            cost: Local,
            risk: ToolRisk::Read,
        },
        Def {
            name: "city_found",
            description: "Found the city: a charter (omni.city.charter/v1: name, seed, climate, style, palette, palettes, relief, districts, publish_policy) and the program of region [0,0]. Founds only with zero building-code errors; otherwise returns the violations to fix. Refuses to replace an existing city unless replace is true.",
            schema: obj(json!({ "charter": { "type": "object" }, "program": { "type": "object" }, "replace": { "type": "boolean" }, "agent": { "type": "string" } }), &["charter", "program"]),
            call: core(|a| Box::pin(found(a))),
            cost: Local,
            risk: ToolRisk::Reversible,
        },
        Def {
            name: "city_codegeo",
            description: "Run a geometry script (Rhai, against the styled kit) for a new building typology or object; returns the GLB (base64), anchors, triangle count and errors. Reference the result in a building's typology.",
            schema: obj(json!({ "script": { "type": "string" }, "class": { "type": "string", "description": "building | prop | tree" } }), &["script"]),
            call: ToolImpl::Host,
            cost: Local,
            risk: ToolRisk::Read,
        },
    ]
}

/// The table rows (appended by `core::llm::tool_table`).
pub fn entries() -> Vec<ToolEntry> {
    defs()
        .into_iter()
        .map(|d| ToolEntry {
            name: d.name,
            description: leak_with_ops(d.name, d.description),
            input_schema: d.schema,
            call: d.call,
            category: "city",
            cost_hint: d.cost,
            platforms: &[],
            feature: None,
            risk: Some(d.risk),
            always_ask: false,
        })
        .collect()
}

/// Descriptions that take ops carry the op cheat-sheet; built once with the
/// table, so leaking the handful of strings is bounded.
fn leak_with_ops(name: &str, base: &'static str) -> &'static str {
    if matches!(name, "city_validate" | "city_propose" | "city_apply") {
        Box::leak(format!("{base}{OPS_HELP}").into_boxed_str())
    } else {
        base
    }
}

pub fn names() -> Vec<&'static str> {
    defs().into_iter().map(|d| d.name).collect()
}

// ── Argument helpers ───────────────────────────────────────────────────

fn region_of(a: &Value) -> Result<[i32; 2], String> {
    match a.get("region") {
        None | Some(Value::Null) => Ok([0, 0]),
        Some(v) => {
            let arr = v
                .as_array()
                .filter(|x| x.len() == 2)
                .ok_or("argument \"region\" must be [rx, ry] (two integers)".to_string())?;
            let n = |i: usize| {
                arr[i]
                    .as_i64()
                    .map(|x| x as i32)
                    .ok_or("argument \"region\" must be [rx, ry] (two integers)".to_string())
            };
            Ok([n(0)?, n(1)?])
        }
    }
}

fn ops_of(a: &Value) -> Result<Vec<PatchOp>, String> {
    let raw = a.get("ops").cloned().unwrap_or(json!([]));
    let arr = raw.as_array().cloned().unwrap_or_default();
    let mut out = Vec::with_capacity(arr.len());
    for (i, op) in arr.into_iter().enumerate() {
        out.push(
            serde_json::from_value(op)
                .map_err(|e| format!("ops[{i}] does not match the patch format: {e}.{OPS_HELP}"))?,
        );
    }
    Ok(out)
}

fn author_of(a: &Value) -> Author {
    let name = a["agent"].as_str().unwrap_or("").trim();
    Author {
        kind: "mcp".into(),
        name: if name.is_empty() {
            "mcp".into()
        } else {
            name.chars().take(60).collect()
        },
        role: None,
    }
}

async fn current_revision(region: [i32; 2]) -> Result<String, String> {
    Ok(api::region(region).await?["revision"]
        .as_str()
        .unwrap_or_default()
        .to_string())
}

// ── Bodies ─────────────────────────────────────────────────────────────

async fn inspect(a: Value) -> Result<Value, String> {
    let region = region_of(&a)?;
    let status = api::status().await?;
    if status["city"].is_null() {
        return Ok(json!({
            "mode": status["mode"],
            "founded": false,
            "message": "The city was not founded yet: call city_found with a charter and the program of region [0,0].",
        }));
    }
    let r = api::region(region).await?;
    let charter: Option<Charter> = serde_json::from_value(status["city"]["charter"].clone()).ok();
    let program: CityProgram = serde_json::from_value(r["program"].clone()).unwrap_or_default();
    let options = match charter.clone() {
        Some(c) => {
            let p = program.clone();
            tokio::task::spawn_blocking(move || super::docs::options(&c, &p))
                .await
                .map_err(|e| e.to_string())?
        }
        None => Value::Null,
    };
    let bp = &r["blueprint"];
    let lots: Vec<Value> = bp["lots"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|l| json!({ "id": l["id"], "rect": l["rect"], "zone": l["zone"], "front": l["front"], "building": l["building"], "owned": !l["owner_ref"].is_null() }))
        .collect();
    let buildings: Vec<Value> = bp["buildings"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|b| json!({ "id": b["id"], "lot": b["lot"], "use": b["use"], "floors": b["floors"], "roof": b["roof"]["kind"], "progress": b["progress"], "door": b["door"]["outside"] }))
        .collect();
    let violations = r["violations"].as_array().cloned().unwrap_or_default();
    let errors = violations
        .iter()
        .filter(|v| v["severity"] == "error")
        .count();
    let mut out = json!({
        "mode": status["mode"],
        "founded": true,
        "city": {
            "city_id": status["city"]["city_id"],
            "name": charter.as_ref().map(|c| c.name.clone()),
            "style": charter.as_ref().map(|c| c.style.clone()),
            "story": charter.as_ref().map(|c| c.story.clone()),
            "publish_policy": charter.as_ref().map(|c| c.publish_policy.clone()),
            "regions": status["city"]["regions"],
        },
        "region": region,
        "revision": r["revision"],
        "report": { "errors": errors, "warnings": violations.len() - errors, "violations": violations },
        "options": options,
        "lots": lots,
        "buildings": buildings,
        "streets": bp["streets"].as_array().map(|s| s.iter().map(|x| json!({ "id": x["id"], "class": x["class"], "width": x["width"] })).collect::<Vec<_>>()),
    });
    if a["include_program"].as_bool() != Some(false) {
        out["program"] = r["program"].clone();
    }
    if let Some(id) = a["lot"].as_str() {
        out["lot"] = bp["lots"]
            .as_array()
            .and_then(|l| l.iter().find(|x| x["id"] == id).cloned())
            .unwrap_or(Value::Null);
    }
    if let Some(id) = a["building"].as_str() {
        out["building"] = json!({
            "blueprint": bp["buildings"].as_array().and_then(|l| l.iter().find(|x| x["id"] == id).cloned()),
            "spec": program.buildings.iter().find(|b| b.id == id),
        });
    }
    Ok(out)
}

async fn validate(a: Value) -> Result<Value, String> {
    let region = region_of(&a)?;
    let ops = ops_of(&a)?;
    let base = match a["base_revision"].as_str().filter(|s| !s.is_empty()) {
        Some(b) => b.to_string(),
        None => current_revision(region).await?,
    };
    let r = api::validate(
        Proposal {
            region,
            base_revision: base,
            ops,
            admin: false,
        },
        author_of(&a),
    )
    .await?;
    let ok = !super::store::has_errors(&r.violations);
    let mut v = serde_json::to_value(r).map_err(|e| e.to_string())?;
    v["ok"] = json!(ok);
    Ok(v)
}

async fn propose(a: Value) -> Result<Value, String> {
    let region = region_of(&a)?;
    let out = api::propose(
        Proposal {
            region,
            base_revision: a["base_revision"].as_str().unwrap_or("").to_string(),
            ops: ops_of(&a)?,
            admin: false,
        },
        author_of(&a),
        a["reason"].as_str().unwrap_or("").to_string(),
    )
    .await?;
    serde_json::to_value(out).map_err(|e| e.to_string())
}

async fn apply(a: Value) -> Result<Value, String> {
    let region = region_of(&a)?;
    let out = api::apply(
        Proposal {
            region,
            base_revision: a["base_revision"].as_str().unwrap_or("").to_string(),
            ops: ops_of(&a)?,
            admin: false,
        },
        author_of(&a),
        a["reason"].as_str().unwrap_or("").to_string(),
        a["idempotency_key"].as_str().map(str::to_string),
        None,
    )
    .await?;
    serde_json::to_value(out).map_err(|e| e.to_string())
}

async fn render(a: Value) -> Result<Value, String> {
    let region = region_of(&a)?;
    let view = a["view"].as_str().unwrap_or("top").to_string();
    let px = a["px"].as_u64().unwrap_or(512) as u32;
    let ops = ops_of(&a)?;
    let (png, violations) = if ops.is_empty() {
        (api::preview_png(region, &view, px).await?, Vec::new())
    } else {
        api::preview_ops(region, ops, &view, px).await?
    };
    let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
    Ok(json!({
        "region": region,
        "view": view,
        "px": px.clamp(64, 1024),
        "mime": "image/png",
        "bytes": png.len(),
        "violations": violations,
        // The MCP server moves this into an image content block; the PNG is
        // not repeated as text.
        "_mcpImage": { "type": "image", "data": b64, "mimeType": "image/png" },
    }))
}

async fn found(a: Value) -> Result<Value, String> {
    let status = api::status().await?;
    if !status["city"].is_null() && a["replace"].as_bool() != Some(true) {
        return Err(format!(
            "{}_ALREADY_FOUNDED: a city exists ({}); pass replace: true only if the person asked to replace it",
            super::ERR,
            status["city"]["city_id"].as_str().unwrap_or("")
        ));
    }
    let seed = a["charter"]["seed"].as_str().unwrap_or("").to_string();
    let mut charter_v = a["charter"].clone();
    let city_id = charter_v["city_id"]
        .as_str()
        .filter(|s| super::store::safe_id(s))
        .map(str::to_string)
        .unwrap_or_else(|| format!("c{}", &uuid::Uuid::new_v4().simple().to_string()[..8]));
    charter_v["city_id"] = json!(city_id);
    let parsed = super::founding::parse_answer(
        &json!({ "charter": charter_v, "program": a["program"] }),
        &city_id,
        if seed.is_empty() { "omni" } else { &seed },
    );
    let (charter, program) = match parsed {
        Ok(x) => x,
        Err(v) => return Ok(json!({ "founded": false, "violations": v })),
    };
    let mut author = author_of(&a);
    author.role = Some("fundador".into());
    let r = api::found(charter, vec![program], author, None).await?;
    serde_json::to_value(r).map_err(|e| e.to_string())
}

/// `outputSchema` of the city tools (chained by the app's MCP server).
pub fn output_schema(name: &str) -> Option<Value> {
    let t = |x: &str| json!({ "type": x });
    let result = || {
        obj(
            json!({ "revision": t("string"), "violations": t("array"), "summary": t("object") }),
            &["revision", "violations"],
        )
    };
    Some(match name {
        "city_inspect" => obj(
            json!({ "mode": t("string"), "founded": t("boolean"), "revision": t("string"), "report": t("object"), "options": {}, "program": {} }),
            &["founded"],
        ),
        "city_validate" => {
            let mut r = result();
            r["properties"]["ok"] = t("boolean");
            r
        }
        "city_propose" | "city_apply" => {
            let mut r = result();
            r["properties"]["state"] = t("string");
            r["properties"]["log_id"] = json!({ "type": ["string", "null"] });
            r["required"] = json!(["revision", "violations", "state"]);
            r
        }
        "city_render" => obj(
            json!({ "mime": t("string"), "bytes": t("integer"), "view": t("string"), "violations": t("array") }),
            &["mime"],
        ),
        "city_found" => obj(
            json!({ "founded": t("boolean"), "city_id": t("string"), "violations": t("array") }),
            &["founded"],
        ),
        "city_codegeo" => obj(
            json!({ "glb_base64": t("string"), "anchors": t("array"), "triangles": t("integer"), "errors": t("array") }),
            &[],
        ),
        _ => return None,
    })
}
