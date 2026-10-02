use std::time::Duration;

use omniget_core::core::canvas::media::Actor;
use omniget_core::core::canvas::model::Point;
use serde_json::{json, Map, Value};
use tauri::AppHandle;

use super::{
    any, arr, def, destructive, i, list, locate_arg, locate_in, n, need, node, num, out, ready, s,
    st, strs, ty, ws, ws_floor, Def,
};
use crate::canvas::generate;

pub fn defs() -> Vec<Def> {
    let quote_out = out(
        json!({"quoteId": ty("string"), "credits": ty("number"), "left": ty("number"), "budgetLeft": ty("number"), "balance": ty("number"), "expiresAt": ty("integer"), "fitsBudget": ty("boolean"), "needsApproval": ty("boolean"), "reason": any(), "references": ty("integer")}),
        &["quoteId", "credits", "fitsBudget", "needsApproval"],
    );
    let status_out = out(
        json!({"nodeId": ty("string"), "workspaceId": ty("string"), "state": ty("string"), "progress": any(), "error": any(), "quote": any(), "version": ty("integer"), "resultNodeId": any(), "assetId": any(), "intent": any()}),
        &["nodeId", "state"],
    );
    vec![
        def("canvas_generate_models", "Higgsfield image models usable by a generation node (v1: images only); names shared by several models carry the id. The CLI lists no prices: canvas_generate_quote prices a request without spending. With model: that model's params (name, type, options, default, required, editable) and whether it takes reference images. Read-only, spends nothing.", false, json!({
            "model": s("Model id (job_type) to describe, e.g. nano_banana_flash")
        }), &[], out(json!({"models": arr(ty("object")), "schema": any()}), &[])),
        def("canvas_generate_create", "Place a Generation node (Higgsfield image) with a prompt, model and params. refs: image nodes (ids or titles) on the same floor corded in as reference images; they must be images stored in the workspace. usedIn: free text shown as \"will be used in: …\"; usedInNodeId adds a dashed cord to that node (a label only, never permission to spend). Spends nothing: follow with canvas_generate_quote then canvas_generate_submit.", true, json!({
            "workspaceId": ws(), "floorId": s("Floor id (default: the workspace's active floor)"),
            "model": s("Model id from canvas_generate_models"), "prompt": s("What to generate"),
            "params": super::schemas::free("Model params by name, e.g. {\"aspect_ratio\": \"16:9\", \"resolution\": \"2k\"}; media params are not accepted (use refs)"),
            "usedIn": s("Where the result will be used, e.g. \"login screen of the app\""), "usedInNodeId": s("Node the result is for (id or unique title); drawn as a dashed cord"),
            "refs": strs("Reference image nodes (ids or titles), corded into the generator"),
            "title": s("Node title (default Generation)"), "x": n("Left"), "y": n("Top")
        }), &["model", "prompt"], out(json!({"nodeId": ty("string"), "node": any(), "schema": any(), "next": ty("string")}), &["nodeId"])),
        def("canvas_generate_quote", "Price the node's current prompt, params and reference cords with `higgsfield generate cost` (reference images are uploaded first, nothing is generated). Returns a quote valid for 5 minutes, bound to exactly this request; the node shows it (\"≈ 12 credits · 88 left\"). needsApproval=true means an agent cannot submit it (above the workspace's automatic limit): the person approves it on the node.", true, json!({
            "nodeId": node()
        }), &["nodeId"], quote_out),
        def("canvas_generate_submit", "Spend credits: generate with a quote from canvas_generate_quote. The reservation must fit the workspace budget and the real account balance; an agent may only go alone below the automatic limit (default 10 credits per generation, 3 per hour), otherwise PROVIDER_APPROVAL_REQUIRED and the node waits for the person. Reuse the same idempotencyKey after an unclear outcome; never invent a new key to retry. Progress: canvas_generate_status.", true, json!({
            "nodeId": node(), "quoteId": s("quoteId from canvas_generate_quote"), "idempotencyKey": s("Your key for this attempt (letters, digits, - and _; default derived from the quote)")
        }), &["nodeId", "quoteId"], out(json!({"intentId": ty("string"), "state": ty("string"), "jobId": any(), "reserved": ty("number"), "actual": any(), "actor": ty("string"), "version": any(), "error": any()}), &["intentId", "state"])),
        def("canvas_generate_status", "State of a generation node: draft, quoted, awaiting_approval, running (progress.status), done (resultNodeId = the image node of the newest version, assetId = its image in the workspace library), uncertain (Higgsfield did not confirm: the person searches again or releases) or failed. waitMs waits up to 60 s while it runs.", false, json!({
            "nodeId": node(), "waitMs": i("Wait while running (ms)", 0, 60_000)
        }), &["nodeId"], status_out),
        def("canvas_generate_versions", "Every version a generation node made: version, assetPath (workspace file), assetId (library image, named \"<title> vN\", source generated), nodeId (its image node), credits, prompt, params, model.", false, json!({
            "nodeId": node()
        }), &["nodeId"], out(json!({"nodeId": ty("string"), "versions": arr(ty("object"))}), &["nodeId", "versions"])),
        destructive(def("canvas_generate_budget", "Read the workspace's Higgsfield budget (ceiling, reserved, spent, left, agent auto limit, real balance). Passing ceiling, autoLimit or autoPerHour changes it: that decides how many of the person's credits can be spent, so only do it when the person asked. The ceiling starts at 0 (nothing can be spent).", true, json!({
            "workspaceId": ws(), "ceiling": n("Total credits this workspace may spend"), "autoLimit": n("Max credits per generation an agent may spend without approval"), "autoPerHour": i("Max generations per hour an agent may start without approval", 0, 1000)
        }), &[], out(json!({"workspaceId": ty("string"), "ceiling": ty("number"), "reserved": ty("number"), "spent": ty("number"), "left": ty("number"), "autoLimit": ty("number"), "autoPerHour": ty("integer"), "balance": any()}), &["ceiling", "left"]))),
    ]
}

fn node_id(app: &AppHandle, a: &Value) -> Result<String, String> {
    let st = ready(app);
    Ok(locate_arg(&st, a, "nodeId")?.2.id)
}

pub async fn call(app: &AppHandle, name: &str, a: &Value) -> Result<Value, String> {
    match name {
        "canvas_generate_models" => match st(a, "model") {
            Some(m) => Ok(json!({ "schema": generate::schema_of(&m).await? })),
            None => Ok(json!({ "models": generate::models().await? })),
        },
        "canvas_generate_create" => {
            let store = ready(app);
            let (ws, floor) = ws_floor(&store, a)?;
            let same = |key: &str| -> Result<String, String> {
                let (w2, f2, n) = locate_in(&store, key, Some(&ws))?;
                if w2 != ws || f2 != floor {
                    return Err(format!("CANVAS_INVALID: '{key}' is not on the same floor"));
                }
                Ok(n.id)
            };
            let refs = list(a, "refs")
                .iter()
                .map(|r| same(r))
                .collect::<Result<Vec<_>, _>>()?;
            let used_in_node = match st(a, "usedInNodeId") {
                Some(k) => Some(same(&k)?),
                None => None,
            };
            let at = match (num(a, "x"), num(a, "y")) {
                (Some(x), Some(y)) => Some(Point { x, y }),
                _ => None,
            };
            let params: Map<String, Value> = a
                .get("params")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            let mut v = generate::create(
                &ws,
                &floor,
                generate::CreateSpec {
                    model: need(a, "model")?,
                    prompt: a["prompt"].as_str().unwrap_or("").to_string(),
                    params,
                    title: st(a, "title"),
                    used_in: st(a, "usedIn"),
                    used_in_node,
                    refs,
                    at,
                    by: Some(super::who::origin()),
                },
            )
            .await?;
            v["next"] = json!(
                "canvas_generate_quote {nodeId}, then canvas_generate_submit {nodeId, quoteId}"
            );
            super::navigate_window(app, &ws, &floor, v["nodeId"].as_str());
            Ok(v)
        }
        "canvas_generate_quote" => {
            let id = node_id(app, a)?;
            generate::requested_by(&id, &super::who::origin());
            generate::quote(&id, Actor::Agent).await
        }
        "canvas_generate_submit" => {
            let id = node_id(app, a)?;
            generate::requested_by(&id, &super::who::origin());
            generate::submit(
                &id,
                &need(a, "quoteId")?,
                st(a, "idempotencyKey"),
                Actor::Agent,
            )
            .await
        }
        "canvas_generate_status" => {
            let id = node_id(app, a)?;
            let wait = a
                .get("waitMs")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                .min(60_000);
            let until = std::time::Instant::now() + Duration::from_millis(wait);
            loop {
                let v = generate::status(&id)?;
                if v["state"] != "running" || std::time::Instant::now() >= until {
                    return Ok(v);
                }
                tokio::time::sleep(Duration::from_millis(1000)).await;
            }
        }
        "canvas_generate_versions" => generate::versions(&node_id(app, a)?),
        "canvas_generate_budget" => {
            let store = ready(app);
            let (ws, _) = ws_floor(&store, a)?;
            generate::budget(
                &ws,
                num(a, "ceiling"),
                num(a, "autoLimit"),
                a.get("autoPerHour").and_then(Value::as_i64),
                true,
            )
            .await
        }
        _ => Err("TOOL_NOT_AUTHORIZED".into()),
    }
}
