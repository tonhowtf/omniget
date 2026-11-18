use super::{
    policy::{self, Principal},
    ToolDef,
};
use omniget_core::platforms::PlatformDownloader;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

pub fn catalog(p: &Principal) -> Vec<Value> {
    tools(p).into_iter().map(|tool| {
        let writes = matches!(policy::scope(tool.name), Some("enqueue" | "control" | "company_control" | "orchestration_control" | "orchestration_create" | "app_control" | "app_capture" | "system_change" | "telegram_write" | "social_write" | "phonechat_write" | "approvals_respond" | "agents_run"))
            || matches!(tool.name, "auth_connection_request" | "diagnostic_bundle_create" | "catalog_install" | "memory_retain" | "omniget_invoke" | "dev_pack" | "dev_http" | "dev_crawl")
            || (matches!(tool.name, "dev_ports" | "dev_proxy") && devtools_write(p))
            || (tool.name == crate::commands::quicktools::TOOL && system_change(p))
            || matches!(tool.name, "transfer_get" | "transfer_put" | "transfer_manage")
            || (tool.name == "transfer_servers" && transfer_write(p))
            || tool.name == "research_meta_ads"
            || super::global_dl::writes(tool.name);
        let open_world = matches!(tool.name, "download_enqueue" | "downloads_batch_enqueue" | "download_retry" | "download_resume" | "media_inspect" | "media_collection_list" | "dev_http" | "dev_scrape" | "dev_crawl" | "dev_proxy" | "transfer_list" | "transfer_get" | "transfer_put" | "transfer_manage") || super::global_dl::open_world(tool.name) || crate::commands::social::tools::owns(tool.name) || (crate::commands::research::mcp::TOOLS.contains(&tool.name) && tool.name != "research_endpoints");
        let mut value = serde_json::to_value(&tool).expect("static tool schema serializes");
        let destructive = writes && (matches!(tool.name, "download_cancel" | "download_pause" | "missions_cancel" | "missions_pause" | "wmb_execute" | "job_cancel" | "tg_delete_messages" | "x_delete_post" | "dev_ports" | "transfer_manage") || super::canvas::is_destructive(tool.name));
        let idempotent = !matches!(tool.name,"transfer_get"|"transfer_put"|"transfer_manage"|"dev_pack"|"dev_http"|"dev_crawl"|"dev_proxy"|"omniget_app_capture"|"omniget_app_export"|"gd_download"|"gd_act"|"gd_cdp") && !(writes && super::canvas::owns(tool.name));
        value["annotations"] = json!({"readOnlyHint":!writes,"destructiveHint":destructive,"idempotentHint":idempotent,"openWorldHint":open_world});
        value
    }).collect()
}

fn devtools_write(p: &Principal) -> bool {
    p.scopes.iter().any(|s| s == "devtools_write")
}

fn system_change(p: &Principal) -> bool {
    p.scopes.iter().any(|s| s == "system_change")
}

fn transfer_write(p: &Principal) -> bool {
    p.scopes.iter().any(|s| s == "transfer_write")
}

pub fn tools(p: &Principal) -> Vec<ToolDef> {
    let mut list = super::tools()
        .into_iter()
        .filter(|t| policy::allowed(p, t.name))
        .collect::<Vec<_>>();
    for (name,description,props,required) in [
        ("omniget_commands","List everything the app can do: without arguments, the areas (music, video, tube, read, guildchat, phonechat, pdf, settings...); with area or query, the commands and the arguments omniget_invoke takes.",json!({"area":{"type":"string"},"query":{"type":"string"},"limit":{"type":"integer"},"cursor":{"type":"integer"}}),json!([])),
        ("omniget_invoke","Call any OmniGet command by name, exactly as the app's own interface does. Find it and its arguments with omniget_commands. The app must be open.",json!({"command":{"type":"string"},"args":{"type":"object"},"timeoutSeconds":{"type":"integer"},"windowId":{"type":"string"}}),json!(["command"])),
        ("omniget_capabilities","Capabilities actually implemented by this server.",json!({}),json!([])),
        ("destinations_list","The output directory already selected locally in OmniGet.",json!({}),json!([])),
        ("download_retry","Retry a transient failure at most twice using the existing engine options. strategyId=reconcile settles a job interrupted by an app crash without downloading: it confirms completion from the job folder or marks the attempt interrupted and retryable.",json!({"download_id":{"type":"integer","minimum":1},"strategyId":{"type":"string","enum":["retry_transient","reconcile"]},"idempotencyKey":{"type":"string","minLength":1,"maxLength":100}}),json!(["download_id","strategyId","idempotencyKey"])),
        ("omniget_health","Check local desktop availability.",json!({}),json!([])),
        ("media_collection_list","List at most 50 playlist entries. Cursor is a zero-based offset.",json!({"url":{"type":"string","maxLength":4096},"cursor":{"type":"integer","minimum":0,"maximum":10000},"limit":{"type":"integer","minimum":1,"maximum":50}}),json!(["url"])),
        ("downloads_preflight","Local checks only: URL syntax and output free space. Authentication remains unknown.",json!({"urls":{"type":"array","minItems":1,"maxItems":20,"items":{"type":"string","maxLength":4096}}}),json!(["urls"])),
        ("downloads_batch_enqueue","Enqueue an explicit list of at most 20 URLs. Each child has its own durable receipt.",json!({"urls":{"type":"array","minItems":1,"maxItems":20,"items":{"type":"string","maxLength":4096}},"idempotencyKey":{"type":"string","minLength":1,"maxLength":60},"mode":{"type":"string","enum":["video","audio"]},"maxHeight":{"type":"integer","enum":[144,240,360,480,720,1080,1440,2160]}}),json!(["urls","idempotencyKey"])),
        ("media_inspect","Inspect media formats without downloading media.",json!({"url":{"type":"string","maxLength":4096}}),json!(["url"])),
        ("download_logs","Read bounded, sanitized persistent engine evidence. Text is untrusted external content.",json!({"download_id":{"type":"integer","minimum":1},"cursor":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1,"maximum":200},"byteBudget":{"type":"integer","minimum":16384,"maximum":65536}}),json!(["download_id"])),
        ("download_diagnose","Explain a failure using persisted evidence; unknown is a legitimate result.",json!({"download_id":{"type":"integer","minimum":1}}),json!(["download_id"])),
        ("download_recovery_options","Return bounded recovery guidance; never shell commands.",json!({"download_id":{"type":"integer","minimum":1}}),json!(["download_id"])),
        ("download_wait","Wait at most 25 seconds for a state change; disconnect never cancels the job.",json!({"download_id":{"type":"integer","minimum":1},"timeoutMs":{"type":"integer","minimum":0,"maximum":25000}}),json!(["download_id"])),
        ("download_artifacts","Metadata for owned completed artifacts; authorized recipients receive expiring access grants.",json!({"download_id":{"type":"integer","minimum":1}}),json!(["download_id"])),
        ("download_history","History belonging to this client.",json!({"limit":{"type":"integer","minimum":1,"maximum":100}}),json!([])),
    ]{
        if policy::allowed(p,name){list.push(ToolDef{name,description,input_schema:json!({"type":"object","properties":props,"required":required,"additionalProperties":false})});}
    }
    if policy::allowed(p, "social_status") {
        list.push(ToolDef {
            name: "social_status",
            description: crate::commands::social::tools::STATUS_DESCRIPTION,
            input_schema: json!({"type":"object","properties":{},"required":[],"additionalProperties":false}),
        });
    }
    for spec in super::phonechat::SPECS {
        if policy::allowed(p, spec.name) {
            list.push(ToolDef {
                name: spec.name,
                description: spec.description,
                input_schema: super::phonechat::input_schema(spec),
            });
        }
    }
    for spec in crate::commands::social::tools::all() {
        if policy::allowed(p, spec.name) {
            list.push(ToolDef {
                name: spec.name,
                description: spec.description,
                input_schema: crate::commands::social::tools::input_schema(spec),
            });
        }
    }
    for (name, description, props, required) in crate::commands::read::mcp::defs() {
        if policy::allowed(p, name) {
            list.push(ToolDef{name,description,input_schema:json!({"type":"object","properties":props,"required":required,"additionalProperties":false})});
        }
    }
    for (name, description, mut props, required) in crate::commands::devtools::mcp::defs() {
        if policy::allowed(p, name) {
            if name == "dev_ports" && !devtools_write(p) {
                props["op"]["enum"] = json!(["list"]);
            }
            if name == "dev_proxy" && !devtools_write(p) {
                props["op"]["enum"] = json!(crate::commands::devtools::mcp::PROXY_READ_OPS);
            }
            list.push(ToolDef{name,description,input_schema:json!({"type":"object","properties":props,"required":required,"additionalProperties":false})});
        }
    }
    if policy::allowed(p, crate::commands::quicktools::TOOL) {
        let (name, description, props, required) =
            crate::commands::quicktools::def(system_change(p));
        list.push(ToolDef{name,description,input_schema:json!({"type":"object","properties":props,"required":required,"additionalProperties":false})});
    }
    for (name, description, props, required) in crate::commands::research::mcp::defs() {
        if policy::allowed(p, name) {
            list.push(ToolDef{name,description,input_schema:json!({"type":"object","properties":props,"required":required,"additionalProperties":false})});
        }
    }
    for (name, description, mut props, required) in crate::commands::transfer::mcp::defs() {
        if policy::allowed(p, name) {
            if name == "transfer_servers" && !transfer_write(p) {
                props["op"]["enum"] = json!(crate::commands::transfer::mcp::READ_SERVER_OPS);
            }
            list.push(ToolDef{name,description,input_schema:json!({"type":"object","properties":props,"required":required,"additionalProperties":false})});
        }
    }
    list.extend(
        super::company::tools()
            .into_iter()
            .filter(|t| policy::allowed(p, t.name)),
    );
    list.extend(
        super::orchestration::tools()
            .into_iter()
            .filter(|t| policy::allowed(p, t.name)),
    );
    list.extend(
        super::canvas::tools()
            .into_iter()
            .filter(|t| policy::allowed(p, t.name)),
    );
    list.extend(
        super::memory::tools()
            .into_iter()
            .filter(|t| policy::allowed(p, t.name)),
    );
    list.extend(
        super::auth::tools()
            .into_iter()
            .chain(super::bundle::tools())
            .chain(super::artifact_access::tools())
            .filter(|t| policy::allowed(p, t.name)),
    );
    for t in &mut list {
        if t.name == "download_enqueue" {
            t.input_schema["properties"]["maxHeight"] =
                json!({"type":"integer","enum":[144,240,360,480,720,1080,1440,2160]});
            t.input_schema["properties"]["destinationId"] =
                json!({"type":"string","enum":["default"]});
        }
        t.input_schema["additionalProperties"] = json!(false);
        if matches!(
            t.name,
            "download_enqueue" | "download_cancel" | "download_pause" | "download_resume"
        ) {
            t.input_schema["properties"]["idempotencyKey"] =
                json!({"type":"string","minLength":1,"maxLength":100});
            if let Some(r) = t.input_schema["required"].as_array_mut() {
                if !r.contains(&json!("idempotencyKey")) {
                    r.push(json!("idempotencyKey"));
                }
            }
        }
    }
    list
}
fn validate(p: &Principal, name: &str, a: &Value) -> Result<(), String> {
    let list = tools(p);
    let schema = &list
        .iter()
        .find(|t| t.name == name)
        .ok_or("TOOL_NOT_AUTHORIZED")?
        .input_schema;
    let obj = a.as_object().ok_or("INVALID_ARGUMENTS")?;
    let props = schema["properties"].as_object().ok_or("INVALID_SCHEMA")?;
    for key in schema["required"].as_array().into_iter().flatten() {
        if !obj.contains_key(key.as_str().unwrap_or("")) {
            return Err(format!("MISSING_ARGUMENT: {key}"));
        }
    }
    for (key, value) in obj {
        let rule = props
            .get(key)
            .ok_or_else(|| format!("UNKNOWN_ARGUMENT: {key}"))?;
        let valid = match rule["type"].as_str() {
            Some("integer") => value.is_u64(),
            Some("string") => value.is_string(),
            Some("boolean") => value.is_boolean(),
            Some("array") => value.is_array(),
            _ => true,
        };
        if !valid {
            return Err(format!("INVALID_ARGUMENT: {key}"));
        }
        if let Some(items) = value.as_array() {
            if items.is_empty()
                || items.len() > 20
                || items
                    .iter()
                    .any(|v| v.as_str().is_none_or(|s| s.len() > 4096))
            {
                return Err(format!("INVALID_ARGUMENT: {key}"));
            }
        }
        if let Some(options) = rule["enum"].as_array() {
            if !options.contains(value) {
                return Err(format!("INVALID_ARGUMENT: {key}"));
            }
        }
        if let Some(n) = value.as_u64() {
            if rule["minimum"].as_u64().is_some_and(|m| n < m)
                || rule["maximum"].as_u64().is_some_and(|m| n > m)
            {
                return Err(format!("INVALID_ARGUMENT: {key}"));
            }
        }
        if let Some(s) = value.as_str() {
            if rule["maxLength"]
                .as_u64()
                .is_some_and(|m| s.len() > m as usize)
                || rule["minLength"]
                    .as_u64()
                    .is_some_and(|m| s.len() < m as usize)
            {
                return Err(format!("INVALID_ARGUMENT: {key}"));
            }
        }
    }
    Ok(())
}
pub fn sanitize(value: Value) -> Value {
    match value {
        Value::String(s) => json!(crate::core::flight_recorder::redact(&s)),
        Value::Array(v) => Value::Array(v.into_iter().map(sanitize).collect()),
        Value::Object(v) => Value::Object(
            v.into_iter()
                .map(|(k, v)| {
                    let secret = [
                        "authorization",
                        "cookie",
                        "set-cookie",
                        "access_token",
                        "refresh_token",
                        "password",
                        "secret",
                    ]
                    .contains(&k.to_ascii_lowercase().as_str());
                    (
                        k,
                        if secret {
                            json!("[REDACTED]")
                        } else {
                            sanitize(v)
                        },
                    )
                })
                .collect(),
        ),
        v => v,
    }
}
async fn logs(
    id: u64,
    cursor: u64,
    limit: usize,
    budget: usize,
) -> Result<crate::core::download_journal::Page, String> {
    tokio::task::spawn_blocking(move || {
        crate::core::download_journal::page(id, cursor, limit, budget)
    })
    .await
    .map_err(|e| e.to_string())?
}
pub async fn call(app: &AppHandle, p: &Principal, name: &str, a: Value) -> Result<Value, String> {
    policy::active(p)?;
    if name.starts_with("omniget_app_") {
        validate(p, name, &a)?;
        return crate::native_agent::call(app, p, name, a).await;
    }
    if name == "omniget_commands" || name == "omniget_invoke" {
        if !policy::allowed(p, name) {
            return Err("TOOL_NOT_AUTHORIZED".into());
        }
        return if name == "omniget_commands" {
            super::invoke::commands(&a)
        } else {
            super::invoke::invoke(app, &a).await
        };
    }
    if name == "company_query" || name == "company_command" {
        if !policy::allowed(p, name) {
            return Err("TOOL_NOT_AUTHORIZED".into());
        }
        return super::company::call(p, name, a).await;
    }
    if super::orchestration::tools().iter().any(|t| t.name == name) {
        if !policy::allowed(p, name) {
            return Err("TOOL_NOT_AUTHORIZED".into());
        }
        return super::orchestration::call(app, p, name, a).await;
    }
    if super::canvas::owns(name) {
        return super::canvas::call(app, p, name, a).await;
    }
    if matches!(policy::scope(name), Some("memory_read" | "memory_write")) {
        return super::memory::call(app, p, name, a).await;
    }
    if policy::scope(name) == Some("catalog") {
        if !policy::allowed(p, name) {
            return Err("TOOL_NOT_AUTHORIZED".into());
        }
        return super::call_catalog(app, p, name, a).await;
    }
    if name.starts_with("tg_") {
        if !policy::allowed(p, name) {
            return Err("TOOL_NOT_AUTHORIZED".into());
        }
        return super::telegram::call(p, name, a).await;
    }
    if super::phonechat::owns(name) {
        if !policy::allowed(p, name) {
            return Err("TOOL_NOT_AUTHORIZED".into());
        }
        return super::phonechat::call(app, p, name, a).await;
    }
    if crate::commands::social::tools::owns(name) {
        if !policy::allowed(p, name) {
            return Err("TOOL_NOT_AUTHORIZED".into());
        }
        return super::social::call(app, p, name, a).await;
    }
    if name.starts_with("gd_") {
        if !policy::allowed(p, name) {
            return Err("TOOL_NOT_AUTHORIZED".into());
        }
        if !a.is_object() {
            return Err("INVALID_ARGUMENTS".into());
        }
        return super::call(app, name, a).await;
    }
    validate(p, name, &a)?;
    if name.starts_with("wmb_") {
        return super::washmybelly::call(app, p, name, a).await;
    }
    if matches!(name, "media_inspect" | "media_collection_list") {
        check_network(p, &checked_url(&a)?).await?;
    }
    if let Some(id) = a["download_id"].as_u64() {
        policy::own(p, id)?;
    }
    let id = a["download_id"].as_u64().unwrap_or(0);
    let result = match name {
        "omniget_capabilities" => {
            json!({"protocol":super::PROTOCOL,"scopes":p.scopes,"transports":["http","stdio_adapter"],"remote":false,"remoteGateway":super::gateway_grants::get(p)?,"executionIsolation":{"available":super::worker::isolation_available(),"backend":if cfg!(target_os="macos"){"macos-seatbelt-broker-v1"}else{"unavailable"},"workerWireVersion":1,"nativeMissionFiles":"create_only+revision_bound_edit"},"currentBuildClientValidation":"pending","historicallyValidatedClients":[{"name":"Codex CLI","version":"0.155.0-alpha.9.2","transport":"http"},{"name":"Claude Code","version":"2.1.282","transport":"http"},{"name":"@modelcontextprotocol/sdk","version":"1.30.1","transports":["http","stdio"]}],"limitations":["Compatibility is limited to the recorded client versions and tested flows","Artifact transfer requires a separate local root grant and the transfer scope","Downloads require the installed confined worker; execution isolation is currently macOS only","Native missions require a local execution grant; files are created new or edited only against the SHA-256 of the revision last read (max 1 MiB, macOS/Linux atomic swap); media tools only when explicitly granted"],"downloadLifecycle":{"resume":"restart_from_zero","resumeDiscardsPartials":true,"resumeCooldown":"none_after_pause","pauseThenCancel":"partials_removed","crashRecovery":"download_retry strategyId=reconcile"}})
        }
        "destinations_list" => {
            json!({"destinations":[{"destinationId":"default","name":"OmniGet output folder","transferAllowed":false}]})
        }
        "omniget_health" => health(&super::request_protocol()),
        "media_collection_list" => {
            let url = checked_url(&a)?;
            let offset = a["cursor"].as_u64().unwrap_or(0);
            let limit = a["limit"].as_u64().unwrap_or(20);
            let worker =
                super::worker::WorkerDownloader::new(p.clone(), url, None, Default::default())
                    .map_err(|e| e.to_string())?;
            worker
                .collection(offset, limit)
                .await
                .map_err(|e| e.to_string())?
        }
        "downloads_preflight" => {
            let items=a["urls"].as_array().unwrap().iter().map(|u|{
                let result=checked_url(&json!({"url":u}));
                json!({"url":u,"valid":result.is_ok(),"problem":result.err(),"authentication":"unknown","estimatedBytes":null})
            }).collect::<Vec<_>>();
            let settings = crate::storage::config::load_settings(app);
            json!({"items":items,"networkProbes":false,"availableBytes":crate::core::preflight::available_space(&settings.download.default_output_dir)})
        }
        "downloads_batch_enqueue" => {
            let key = a["idempotencyKey"].as_str().unwrap();
            if let Some(receipt) = policy::reserve(p, name, key, &a)? {
                return Ok(receipt);
            }
            let mut children = vec![];
            for (index, url) in a["urls"].as_array().unwrap().iter().enumerate() {
                let mut child = json!({"url":url,"idempotencyKey":format!("{key}-{index}")});
                for option in ["mode", "maxHeight"] {
                    if let Some(v) = a.get(option) {
                        child[option] = v.clone();
                    }
                }
                let item = match Box::pin(call(app, p, "download_enqueue", child)).await {
                    Ok(value) => json!({"accepted":true,"result":value}),
                    Err(e) => {
                        json!({"accepted":false,"error":crate::core::flight_recorder::redact(&e)})
                    }
                };
                children.push(item);
            }
            let value = json!({"batchId":key,"children":children});
            policy::finish(p, name, key, &value)?;
            value
        }
        "media_inspect" => {
            let url = checked_url(&a)?;
            let worker = super::worker::WorkerDownloader::new(
                p.clone(),
                url.clone(),
                None,
                Default::default(),
            )
            .map_err(|e| e.to_string())?;
            let info = worker
                .get_media_info(&url)
                .await
                .map_err(|e| e.to_string())?;
            json!({"title":info.title,"author":info.author,"duration":info.duration_seconds,"formats":info.available_qualities})
        }
        "downloads_queue" => {
            let every = super::access::sees_everything(p);
            let ids = policy::owned(p)?;
            let status = a["status"].as_str().unwrap_or("");
            let snapshot = super::queue_snapshot(app).await;
            let queued: std::collections::HashSet<u64> = snapshot.iter().map(|i| i.id).collect();
            let mut items = snapshot
                .into_iter()
                .filter(|i| {
                    (every || ids.contains(&i.id))
                        && (status.is_empty() || super::status_key(&i.status) == status)
                })
                .map(|i| serde_json::to_value(i).map_err(|e| e.to_string()))
                .collect::<Result<Vec<_>, _>>()?;
            if status.is_empty() || status == "unknown" {
                items.extend(orphaned(&ids, &queued).iter().map(journal_item));
            }
            json!({"total":items.len(),"items":items.into_iter().take(a["limit"].as_u64().unwrap_or(50).min(100)as usize).collect::<Vec<_>>()})
        }
        "download_status" => {
            let (item, queued) = current_item(app, id).await?;
            if queued {
                json!({"item":item,"nextPollAfterMs":1000})
            } else {
                json!({"item":item,"nextPollAfterMs":null,"nextActions":actions_for(id, None)})
            }
        }
        "download_wait" => {
            let (before, queued) = current_item(app, id).await?;
            if !queued {
                return Ok(sanitize(
                    json!({"item":before,"timedOut":false,"nextPollAfterMs":null,"nextActions":actions_for(id, None)}),
                ));
            }
            let end = tokio::time::Instant::now()
                + std::time::Duration::from_millis(
                    a["timeoutMs"].as_u64().unwrap_or(10000).min(25000),
                );
            loop {
                policy::active(p)?;
                let (item, _) = current_item(app, id).await?;
                let timeout = tokio::time::Instant::now() >= end;
                if item != before || timeout {
                    break json!({"item":item,"timedOut":timeout,"nextPollAfterMs":1000});
                }
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            }
        }
        "download_logs" => serde_json::to_value(
            logs(
                id,
                a["cursor"].as_u64().unwrap_or(0),
                a["limit"].as_u64().unwrap_or(100) as usize,
                a["byteBudget"].as_u64().unwrap_or(32768) as usize,
            )
            .await?,
        )
        .map_err(|e| e.to_string())?,
        "download_diagnose" | "download_recovery_options" => {
            let queued = super::queue_item(app, id).await.ok().map(|i| i.status);
            let intent = super::download_intents::load(id).ok().flatten();
            let stage = intent.as_ref().map(|i| i.stage.clone()).unwrap_or_default();
            let attempts = super::download_intents::attempts(id).unwrap_or_default();
            if matches!(
                queued,
                Some(crate::core::queue::QueueStatus::Complete { success: true })
            ) {
                return Ok(sanitize(
                    json!({"downloadId":id,"status":"completed","diagnosis":null,"attempts":attempts,"nextActions":actions_for(id, queued.as_ref())}),
                ));
            }
            let page = tokio::task::spawn_blocking(move || {
                crate::core::download_journal::diagnosis_page(id)
            })
            .await
            .map_err(|e| e.to_string())??;
            let excerpt = |e: &crate::core::download_journal::Event| json!({"eventId":e.event_id,"attemptId":e.attempt_id,"level":e.level,"excerpt":e.message.chars().take(512).collect::<String>()});
            let mut evidence = page
                .events
                .iter()
                .filter(|e| e.level != "info")
                .take(8)
                .map(excerpt)
                .collect::<Vec<_>>();
            let interrupted = queued.is_none() && matches!(stage.as_str(), "unknown" | "executing");
            if evidence.is_empty() && interrupted {
                let n = page.events.len().saturating_sub(8);
                evidence = page.events[n..].iter().map(excerpt).collect();
            }
            let receipt = super::download_intents::receipt(id).ok().flatten();
            let text = receipt
                .iter()
                .filter_map(|r| r.error.as_deref())
                .chain(
                    evidence
                        .iter()
                        .filter(|v| v["level"] != "info")
                        .filter_map(|v| v["excerpt"].as_str()),
                )
                .collect::<Vec<_>>()
                .join("\n");
            let diagnosis = if interrupted {
                json!({"code":"INTERRUPTED","confidence":"high","phase":"engine","retryable":"after_reconcile","nextAction":"reconcile","unknowns":["Whether the transfer finished before the app stopped; reconcile decides it from the job folder"]})
            } else {
                serde_json::to_value(crate::core::root_cause::machine_diagnose(&text))
                    .map_err(|e| e.to_string())?
            };
            let retries = super::download_intents::retries_used(id).unwrap_or(0);
            let attempt = intent.as_ref().map(|i| i.attempt);
            let retry = receipt.as_ref().zip(attempt).map(|(r, attempt)| json!({"allowed":r.retryable&&retries<super::download_intents::MAX_RETRIES,"retryable":r.retryable,"attempt":attempt,"retriesUsed":retries,"maxRetries":super::download_intents::MAX_RETRIES,"notBeforeMs":r.next_allowed_at}));
            let actions = next_actions(
                id,
                &stage,
                queued.as_ref(),
                receipt.as_ref(),
                retries,
                diagnosis["nextAction"]
                    .as_str()
                    .unwrap_or("inspect_more_evidence"),
            );
            let host = intent
                .as_ref()
                .and_then(|i| url::Url::parse(&i.options.url).ok())
                .and_then(|u| u.host_str().map(str::to_owned));
            let actions = with_auth_action(
                actions,
                diagnosis["code"].as_str().unwrap_or(""),
                host.as_deref(),
            );
            json!({"downloadId":id,"diagnosis":diagnosis,"retry":retry,"attempts":attempts,"evidence":evidence,"unknowns":["Rules cannot confirm remote account access or upstream service state"],"incomplete":page.has_more||page.evidence_incomplete||evidence.is_empty(),"nextCursor":page.next_cursor,"nextActions":actions,"externalContent":true})
        }
        "auth_connection_status" | "auth_connection_request" => {
            super::auth::call(app, p, name, &a).await?
        }
        "diagnostic_bundle_create" => super::bundle::create(app, p, id, &a).await?,
        n if crate::commands::read::mcp::TOOLS.contains(&n) => {
            crate::commands::read::mcp::call(n, &a).await?
        }
        n if crate::commands::devtools::mcp::TOOLS.contains(&n) => {
            if let Some(url) = crate::commands::devtools::mcp::url_of(n, &a) {
                check_network(p, url).await?;
            }
            crate::commands::devtools::mcp::call(n, &a, devtools_write(p)).await?
        }
        n if crate::commands::transfer::mcp::TOOLS.contains(&n) => {
            if let Some(url) = crate::commands::transfer::mcp::url_of(&a) {
                check_network(p, &url).await?;
            }
            crate::commands::transfer::mcp::call(n, &a, transfer_write(p)).await?
        }
        n if crate::commands::research::mcp::TOOLS.contains(&n) => {
            if let Some(url) = crate::commands::research::mcp::url_of(n, &a) {
                check_network(p, url).await?;
            }
            crate::commands::research::mcp::call(n, &a).await?
        }
        crate::commands::quicktools::TOOL => {
            crate::commands::quicktools::call(
                &a,
                system_change(p),
                crate::quick_panel::visible(app),
            )
            .await?
        }
        "artifact_access" => super::artifact_access::call(app, p, &a).await?,
        "download_history" => {
            let every = super::access::sees_everything(p);
            let ids = policy::owned(p)?;
            let mut items = crate::core::queue_history::list()
                .into_iter()
                .filter(|i| every || ids.contains(&i.id))
                .map(|i| serde_json::to_value(i).map_err(|e| e.to_string()))
                .collect::<Result<Vec<_>, _>>()?;
            let known: std::collections::HashSet<u64> = items
                .iter()
                .filter_map(|i| i["id"].as_u64())
                .chain(super::queue_snapshot(app).await.iter().map(|i| i.id))
                .collect();
            items.extend(orphaned(&ids, &known).iter().map(journal_item));
            json!({"items":items.into_iter().take(a["limit"].as_u64().unwrap_or(50).min(100)as usize).collect::<Vec<_>>()})
        }
        "download_artifacts" => {
            let (item, _) = current_item(app, id).await?;
            let mut files = vec![];
            let mut truncated = false;
            let complete =
                item["status"]["type"] == "Complete" && item["status"]["data"]["success"] == true;
            if complete && item["file_path"].is_string() {
                let intent = super::download_intents::load(id)?.ok_or("ARTIFACT_INTENT_MISSING")?;
                if intent.principal != p.id {
                    return Err("ARTIFACT_NOT_AUTHORIZED".into());
                }
                let identity = intent
                    .destination_identity
                    .clone()
                    .ok_or("ARTIFACT_ROOT_IDENTITY_MISSING")?;
                let root = std::path::PathBuf::from(&intent.destination);
                let primary = item["file_path"].as_str().map(std::path::PathBuf::from);
                if primary.as_ref().is_some_and(|f| !f.starts_with(&root)) {
                    return Err("ARTIFACT_NOT_AUTHORIZED".into());
                }
                let (names, more) = tokio::task::spawn_blocking(move || {
                    super::download_intents::job_files(&intent)
                })
                .await
                .map_err(|_| "ARTIFACT_UNAVAILABLE")??;
                truncated = more;
                for (index, name) in names.iter().enumerate() {
                    let file = root.join(name);
                    let is_primary = primary.as_deref() == Some(file.as_path());
                    let permit = super::artifacts::admit()?;
                    let sized = crate::core::artifact_validation::probe_authorized_sized(
                        &root, &identity, &file,
                    )
                    .await;
                    drop(permit);
                    let (bytes, validation) = match sized {
                        Ok(value) => value,
                        Err(error) => {
                            files.push(json!({"artifactId":format!("{id}:{index}"),"name":name,"primary":is_primary,"bytes":null,"validation":"unavailable","validationError":error,"transferAllowed":false}));
                            continue;
                        }
                    };
                    if bytes == 0 {
                        continue;
                    }
                    let (level, media, error) = match validation {
                        Ok(probe) => ("ffprobe", Some(probe), None),
                        Err(error) => ("existence_only", None, Some(error)),
                    };
                    let mut entry = json!({"artifactId":format!("{id}:{index}"),"bytes":bytes,"name":name,"primary":is_primary,"validation":level,"media":media,"validationError":error,"transferAllowed":false});
                    if name.starts_with('.') {
                        entry["suggestedName"] = json!(name.trim_start_matches('.'));
                    }
                    if p.scopes.iter().any(|s| s == "transfer") {
                        let permit = super::artifacts::admit()?;
                        let principal = p.clone();
                        match tokio::task::spawn_blocking(move || {
                            let _permit = permit;
                            super::artifacts::register(&principal, &file)
                        })
                        .await
                        {
                            Ok(Ok(grant)) => {
                                entry["access"] = serde_json::to_value(grant)
                                    .map_err(|_| "ARTIFACT_SERIALIZATION")?;
                                entry["transferAllowed"] = json!(true);
                            }
                            Ok(Err(e)) => entry["transferError"] = json!(e),
                            Err(_) => entry["transferError"] = json!("ARTIFACT_UNAVAILABLE"),
                        }
                    }
                    files.push(entry);
                }
            }
            if complete && files.is_empty() {
                return Err(
                    "ARTIFACT_MISSING: the job completed but its folder holds no produced file"
                        .into(),
                );
            }
            json!({"artifacts":files,"truncated":truncated,"maxArtifacts":super::download_intents::MAX_JOB_ARTIFACTS})
        }
        "download_enqueue" | "download_cancel" | "download_pause" | "download_resume"
        | "download_retry" => {
            static MUTATION: std::sync::OnceLock<tokio::sync::Mutex<()>> =
                std::sync::OnceLock::new();
            let _guard = MUTATION
                .get_or_init(|| tokio::sync::Mutex::new(()))
                .lock()
                .await;
            let key = a["idempotencyKey"]
                .as_str()
                .ok_or("IDEMPOTENCY_KEY_REQUIRED")?;
            if name == "download_enqueue" {
                return enqueue_durable(app, p, key, &a).await;
            }
            if name == "download_retry" && a["strategyId"] == "reconcile" {
                return reconcile_job(app, p, id, key, &a).await;
            }
            if matches!(name, "download_retry" | "download_resume") {
                if let Some(receipt) = super::download_intents::replay_receipt(p, name, key, &a)? {
                    return Ok(sanitize(receipt));
                }
                let intent = super::download_intents::prepare_retry(p, id, key, name, &a)?;
                let restart = if name == "download_resume" {
                    let snapshot = intent.clone();
                    Some(
                        tokio::task::spawn_blocking(move || {
                            super::download_intents::discard_for_restart(&snapshot)
                        })
                        .await
                        .map_err(|e| e.to_string())??,
                    )
                } else {
                    None
                };
                let mut value = restore_intent(app, intent).await?;
                if let Some((files, bytes)) = restart {
                    value["resume"] = json!({"continuation":"restart","discardedFiles":files,"discardedBytes":bytes,"cooldownApplied":false});
                }
                policy::finish(p, name, key, &value)?;
                return Ok(value);
            }
            let mut internal = a.clone();
            internal.as_object_mut().unwrap().remove("idempotencyKey");
            return keyed(p, name, key, &a, async {
                let mut value = super::call(app, name, internal).await?;
                if name == "download_cancel" {
                    let removed = tokio::task::spawn_blocking(move || {
                        super::download_intents::cancel_paused(id)
                    })
                    .await;
                    if let Ok(Ok(removed)) = removed {
                        value["removedLeftovers"] = json!(removed.len());
                    }
                }
                Ok(value)
            })
            .await;
        }
        _ => return Err("TOOL_NOT_AUTHORIZED".into()),
    };
    policy::active(p)?;
    Ok(sanitize(result))
}
pub(crate) fn health(negotiated: &str) -> Value {
    json!({
        "app": "running",
        "reachable": true,
        "machine": super::phonechat::machine(),
        "queue": "available",
        "remote": "disabled",
        "server": {"name": "OmniGet", "version": env!("CARGO_PKG_VERSION")},
        "protocolVersion": negotiated,
        "supportedProtocolVersions": super::SUPPORTED_PROTOCOLS,
    })
}
pub async fn restore_intent(
    app: &AppHandle,
    intent: super::download_intents::Intent,
) -> Result<Value, String> {
    let p = policy::principal(&intent.principal)?;
    if !policy::allowed(&p, "download_enqueue") {
        return Err("ENQUEUE_NOT_GRANTED".into());
    }
    if intent.stage == "unknown" {
        return Ok(sanitize(unknown_outcome(&intent)));
    }
    if intent.stage == "terminal" {
        if let Some(receipt) = super::download_intents::receipt(intent.job_id)? {
            let item = match super::queue_item(app, intent.job_id).await {
                Ok(item) => serde_json::to_value(item).map_err(|e| e.to_string())?,
                Err(_) => journal_item(&intent),
            };
            return Ok(sanitize(
                json!({"download_id":intent.job_id,"attempt":intent.attempt,"outcome":receipt.outcome,"item":item,"receipt":receipt}),
            ));
        }
        return Err("OUTCOME_UNKNOWN: terminal evidence missing".into());
    }
    let intent = super::download_intents::hydrate(intent)?;
    if intent.attempt > 0 && matches!(intent.stage.as_str(), "prepared" | "admitting" | "enqueued")
    {
        let state = app.state::<crate::AppState>();
        let mut q = state.download_queue.lock().await;
        if let Some(item) = q.items.iter().find(|i| i.id == intent.job_id) {
            if matches!(
                item.status,
                crate::core::queue::QueueStatus::Error { .. }
                    | crate::core::queue::QueueStatus::Paused
                    | crate::core::queue::QueueStatus::Complete { .. }
            ) {
                q.items.retain(|i| i.id != intent.job_id);
            }
        }
    }
    if let Some(item) = super::queue_snapshot_raw(app)
        .await
        .into_iter()
        .find(|i| i.id == intent.job_id)
    {
        policy::own(&p, item.id)?;
        if item.url != intent.options.url {
            return Err("DOWNLOAD_ID_CONFLICT".into());
        }
        let item = crate::core::queue::redacted_for_display(vec![item]).remove(0);
        if intent.stage == "executing"
            && item.started_at_ms.is_none()
            && !matches!(
                item.status,
                crate::core::queue::QueueStatus::Complete { .. }
            )
        {
            super::download_intents::unknown(intent.job_id)?;
            return Ok(sanitize(unknown_outcome(&intent)));
        }
        return Ok(sanitize(
            json!({"download_id":intent.job_id,"attempt":intent.attempt,"outcome":"already-queued","item":item}),
        ));
    }
    if !matches!(intent.stage.as_str(), "prepared" | "admitting" | "enqueued") {
        super::download_intents::unknown(intent.job_id)?;
        return Ok(sanitize(unknown_outcome(&intent)));
    }
    check_network(&p, &intent.options.url).await?;
    let url = intent.options.url.clone();
    let mut intent = super::download_intents::prepare_destination(&p, intent.job_id)?;
    intent.options.url = url;
    intent.options.sealed = false;
    let worker = super::worker::WorkerDownloader::new(
        p.clone(),
        intent.options.url.clone(),
        None,
        Default::default(),
    )
    .map_err(|e| e.to_string())?;
    crate::external_url::queue_url_with_executor(
        app,
        intent.options.url.clone(),
        false,
        intent.options.mode.clone(),
        intent.options.quality.clone(),
        Some((p.clone(), std::sync::Arc::new(worker), intent.clone())),
    )
    .await?;
    let item = super::queue_snapshot(app)
        .await
        .into_iter()
        .find(|i| i.id == intent.job_id)
        .ok_or("OUTCOME_UNKNOWN")?;
    Ok(sanitize(
        json!({"download_id":intent.job_id,"attempt":intent.attempt,"outcome":"queued","item":item}),
    ))
}
fn unknown_outcome(intent: &super::download_intents::Intent) -> Value {
    let mut view = intent.clone();
    view.stage = "unknown".into();
    json!({"download_id":intent.job_id,"attempt":intent.attempt,"outcome":"unknown","recovery":"local_reconciliation_required","item":journal_item(&view),"nextActions":next_actions(intent.job_id,"unknown",None,None,0,"reconcile")})
}
async fn enqueue_durable(
    app: &AppHandle,
    p: &Principal,
    key: &str,
    a: &Value,
) -> Result<Value, String> {
    let intent = if let Some(i) = super::download_intents::by_key(p, key, a)? {
        i
    } else {
        if policy::receipt_exists(p, "download_enqueue", key)? {
            return policy::reserve(p, "download_enqueue", key, a)?.ok_or("OUTCOME_UNKNOWN".into());
        }
        let url = checked_url(a)?;
        check_network(p, &url).await?;
        let snapshot = super::queue_snapshot_raw(app).await;
        if snapshot.iter().any(|i| i.url == url && live(&i.status)) {
            return Err("URL_ALREADY_MANAGED".into());
        }
        let running: Vec<u64> = snapshot
            .iter()
            .filter(|i| live(&i.status))
            .map(|i| i.id)
            .collect();
        if let Some(previous) = super::download_intents::unresolved_for_url(p, &url, &running)? {
            return Err(format!("URL_OUTCOME_UNKNOWN: download {previous} was interrupted; settle it with download_retry strategyId=reconcile first"));
        }
        let settings = crate::storage::config::load_settings(app);
        let parent = std::fs::canonicalize(&settings.download.default_output_dir)
            .map_err(|_| "DESTINATION_UNAVAILABLE")?;
        let root = omniget_core::core::secure_files::Root::open(&parent, None)
            .map_err(|_| "DESTINATION_UNSAFE")?;
        let mode = a["mode"].as_str().map(str::to_owned);
        let quality = a["maxHeight"]
            .as_u64()
            .map(|h| h.to_string())
            .or_else(|| Some(settings.download.video_quality.clone()));
        let format_id = quality
            .as_ref()
            .filter(|_| mode.as_deref() != Some("audio"))
            .and_then(|h| h.trim_end_matches('p').parse::<u32>().ok())
            .map(|h| {
                format!(
                    "bv*[height<={h}]+ba/b[height<={h}]/bv*[height<=?{h}]+ba/b[height<=?{h}]/ba"
                )
            });
        let audio_format = if mode.as_deref() == Some("audio") {
            Some(settings.download.music_audio_format.clone())
        } else {
            None
        };
        super::download_intents::reserve(
            p,
            key,
            a,
            super::download_intents::Options {
                url,
                sealed: false,
                mode,
                quality,
                format_id,
                audio_format,
                subtitles: settings.download.download_subtitles,
                auto_subtitles: settings.download.include_auto_subtitles,
                fragments: settings
                    .advanced
                    .concurrent_fragments
                    .clamp(1, super::download_intents::WORKER_MAX_FRAGMENTS),
                parent: parent.to_str().ok_or("DESTINATION_INVALID")?.into(),
                parent_identity: root.identity().clone(),
            },
        )?
    };
    let value = restore_intent(app, intent).await?;
    policy::finish(p, "download_enqueue", key, &value)?;
    Ok(value)
}
async fn current_item(app: &AppHandle, id: u64) -> Result<(Value, bool), String> {
    if let Ok(item) = super::queue_item(app, id).await {
        return Ok((serde_json::to_value(item).map_err(|e| e.to_string())?, true));
    }
    let intent =
        super::download_intents::load(id)?.ok_or_else(|| format!("no download with id {id}"))?;
    Ok((journal_item(&intent), false))
}
pub fn journal_item(i: &super::download_intents::Intent) -> Value {
    let receipt = super::download_intents::receipt(i.job_id).ok().flatten();
    let status = match (i.stage.as_str(), receipt.as_ref()) {
        ("terminal", Some(r)) if r.outcome == "success" => {
            json!({"type":"Complete","data":{"success":true}})
        }
        ("terminal", Some(r)) if r.outcome == "paused" => json!({"type":"Paused"}),
        ("terminal", Some(r)) => {
            json!({"type":"Error","data":{"message":r.error.clone().unwrap_or_else(||r.outcome.clone()),"retryable":r.retryable}})
        }
        (stage, _) => {
            json!({"type":"Unknown","data":{"stage":stage,"interrupted":true,"effectMayHaveRun":matches!(stage,"executing"|"unknown")}})
        }
    };
    json!({"id":i.job_id,"url":crate::core::flight_recorder::redact_url(&i.options.url),"title":null,"platform":"mcp_worker","status":status,"attempt":i.attempt,"file_path":receipt.as_ref().and_then(|r|r.file_path.clone()),"file_size_bytes":receipt.as_ref().and_then(|r|r.bytes),"source":"journal"})
}
fn orphaned(
    owned: &[u64],
    known: &std::collections::HashSet<u64>,
) -> Vec<super::download_intents::Intent> {
    super::download_intents::all()
        .unwrap_or_default()
        .into_iter()
        .filter(|i| owned.contains(&i.job_id) && !known.contains(&i.job_id))
        .collect()
}
fn actions_for(id: u64, queued: Option<&crate::core::queue::QueueStatus>) -> Vec<Value> {
    let stage = super::download_intents::load(id)
        .ok()
        .flatten()
        .map(|i| i.stage)
        .unwrap_or_default();
    let receipt = super::download_intents::receipt(id).ok().flatten();
    let retries = super::download_intents::retries_used(id).unwrap_or(0);
    let diagnosis = receipt
        .as_ref()
        .and_then(|r| r.error.as_deref())
        .map(crate::core::root_cause::machine_diagnose);
    let local = diagnosis
        .as_ref()
        .map(|d| d.next_action)
        .unwrap_or("inspect_more_evidence");
    let actions = next_actions(id, &stage, queued, receipt.as_ref(), retries, local);
    with_auth_action(
        actions,
        diagnosis.as_ref().map(|d| d.code).unwrap_or(""),
        None,
    )
}
fn with_auth_action(mut actions: Vec<Value>, code: &str, host: Option<&str>) -> Vec<Value> {
    if !matches!(code, "AUTH_REQUIRED" | "AUTH_EXPIRED") {
        return actions;
    }
    let mut args = json!({});
    let mut requires = vec!["reason", "idempotencyKey"];
    match host {
        Some(h) if super::auth::valid_host(h).is_some() => args["host"] = json!(h),
        Some(_) => return actions,
        None => requires.insert(0, "host"),
    }
    let action = json!({
        "action":"connect_account",
        "tool":"auth_connection_request",
        "arguments":args,
        "requires":requires,
        "requiresLocalInteraction":true,
        "usedByExternalDownloads":false,
        "retryUsesLogin":false,
        "note":"Asks the person to connect the account inside OmniGet; this client never receives cookies. Downloads started through this MCP server run in a confined worker that does not read local logins (usedByExternalDownloads:false), so a download_retry from here still runs without the login. The person can download it from OmniGet itself once connected."
    });
    let at = actions
        .iter()
        .position(|a| a["action"] == "local_user_action")
        .unwrap_or(actions.len());
    actions.insert(at, action);
    actions
}
fn next_actions(
    id: u64,
    stage: &str,
    queued: Option<&crate::core::queue::QueueStatus>,
    receipt: Option<&super::download_intents::TerminalReceipt>,
    retries_used: u32,
    local_action: &str,
) -> Vec<Value> {
    let key = json!(["idempotencyKey"]);
    let mut out = vec![];
    if stage != "terminal" {
        match (queued,stage) {
            (Some(_),_)=>out.push(json!({"action":"wait","tool":"download_wait","arguments":{"download_id":id}})),
            (None,"executing"|"unknown")=>out.push(json!({"action":"reconcile","tool":"download_retry","arguments":{"download_id":id,"strategyId":"reconcile"},"requires":key,"reason":"interrupted","startsDownload":false})),
            (None,_)=>out.push(json!({"action":"replay_enqueue","tool":"download_enqueue","reason":"never_started","note":"repeat the original download_enqueue call with the same idempotencyKey and arguments"})),
        }
        return out;
    }
    let Some(r) = receipt else {
        out.push(
            json!({"action":"inspect_logs","tool":"download_logs","arguments":{"download_id":id}}),
        );
        return out;
    };
    let exhausted = retries_used >= super::download_intents::MAX_RETRIES;
    match r.outcome.as_str() {
        "success"=>out.push(json!({"action":"fetch_artifacts","tool":"download_artifacts","arguments":{"download_id":id}})),
        "paused"=>out.push(json!({"action":"resume","tool":"download_resume","arguments":{"download_id":id},"requires":key,"continuation":"restart","cooldownApplied":false})),
        _ if r.retryable&&!exhausted=>out.push(json!({"action":"retry","tool":"download_retry","arguments":{"download_id":id,"strategyId":"retry_transient"},"requires":key,"notBeforeMs":r.next_allowed_at})),
        _ if r.retryable=>out.push(json!({"action":"stop","reason":"RETRY_LIMIT_REACHED"})),
        "cancelled"=>out.push(json!({"action":"enqueue_new","tool":"download_enqueue","requires":["url","idempotencyKey"],"note":"a cancelled job is final; enqueue the URL again with a new key"})),
        _=>out.push(json!({"action":"local_user_action","code":local_action,"requiresLocalUser":true})),
    }
    if local_action == "inspect_more_evidence" && r.outcome != "success" {
        out.push(
            json!({"action":"inspect_logs","tool":"download_logs","arguments":{"download_id":id}}),
        );
    }
    out
}
async fn keyed<F>(p: &Principal, op: &str, key: &str, a: &Value, effect: F) -> Result<Value, String>
where
    F: std::future::Future<Output = Result<Value, String>>,
{
    if let Some(receipt) = policy::reserve(p, op, key, a)? {
        return Ok(receipt);
    }
    match effect.await {
        Ok(value) => {
            let value = sanitize(value);
            policy::finish(p, op, key, &value)?;
            Ok(value)
        }
        Err(e) => {
            let _ = policy::release(p, op, key);
            Err(e)
        }
    }
}
async fn reconcile_job(
    app: &AppHandle,
    p: &Principal,
    id: u64,
    key: &str,
    a: &Value,
) -> Result<Value, String> {
    keyed(p,"download_retry",key,a,async{
        let queued=super::queue_item(app,id).await.ok().map(|i|i.status);
        if matches!(queued,Some(crate::core::queue::QueueStatus::Active|crate::core::queue::QueueStatus::Seeding|crate::core::queue::QueueStatus::Paused)) {
            return Err::<Value,String>("RECONCILE_NOT_NEEDED: the job is still managed by the queue".into());
        }
        let principal=p.clone();
        let (outcome,removed)=tokio::task::spawn_blocking(move||super::download_intents::reconcile(Some(&principal),id)).await.map_err(|e|e.to_string())??;
        let intent=super::download_intents::load(id)?.ok_or("DOWNLOAD_INTENT_MISSING")?;
        let receipt=super::download_intents::receipt(id)?;
        settle_reconciled(app,id).await;
        Ok(json!({"download_id":id,"attempt":intent.attempt,"outcome":outcome,"removedLeftovers":removed.len(),"item":journal_item(&intent),"receipt":receipt,"nextActions":actions_for(id,None)}))
    }).await
}
pub fn reconciled_history_entry(
    i: &super::download_intents::Intent,
    r: &super::download_intents::TerminalReceipt,
) -> Option<crate::core::queue_history::HistoryEntry> {
    if i.stage != "terminal" || r.reconciled.is_none() {
        return None;
    }
    let success = r.outcome == "success";
    Some(crate::core::queue_history::HistoryEntry {
        id: i.job_id,
        url: crate::core::flight_recorder::redact_url(&i.options.url),
        platform: "mcp_worker".into(),
        title: crate::core::flight_recorder::redact_url(&i.options.url),
        file_path: r.file_path.clone(),
        file_size_bytes: r.bytes,
        total_bytes: r.bytes,
        success,
        error: if success {
            None
        } else {
            Some(r.error.clone().unwrap_or_else(|| r.outcome.clone()))
        },
        completed_at: (r.ended_at / 1000) as i64,
        thumbnail_url: None,
        kind: Some(crate::core::queue::kind_from_platform("mcp_worker")),
    })
}
pub async fn settle_reconciled(app: &AppHandle, id: u64) {
    let Ok(Some(intent)) = super::download_intents::load(id) else {
        return;
    };
    let Ok(Some(receipt)) = super::download_intents::receipt(id) else {
        return;
    };
    let Some(entry) = reconciled_history_entry(&intent, &receipt) else {
        return;
    };
    crate::core::queue_history::record(entry.clone());
    let state = app.state::<crate::AppState>();
    let snapshot = {
        let mut q = state.download_queue.lock().await;
        if !q.hydrate_entry(&entry, receipt.retryable) {
            return;
        }
        q.get_state()
    };
    crate::core::queue::emit_queue_state_from_state(app, snapshot);
}
fn live(status: &crate::core::queue::QueueStatus) -> bool {
    use crate::core::queue::QueueStatus::*;
    matches!(status, Queued | Active | Paused | Seeding)
}
fn checked_url(a: &Value) -> Result<String, String> {
    let raw = a["url"].as_str().ok_or("INVALID_URL")?;
    if raw.len() > 4096 {
        return Err("INVALID_URL".into());
    }
    let url = url::Url::parse(raw).map_err(|_| "INVALID_URL")?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("INVALID_URL".into());
    }
    url.host_str().ok_or("INVALID_URL")?;
    Ok(url.to_string())
}

fn public_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v) => {
            let o = v.octets();
            !v.is_private()
                && !v.is_loopback()
                && !v.is_link_local()
                && !v.is_multicast()
                && !v.is_unspecified()
                && !v.is_broadcast()
                && o[0] != 0
                && o[0] < 224
                && !(o[0] == 100 && (64..128).contains(&o[1]))
                && !(o[0] == 198 && (o[1] == 18 || o[1] == 19))
        }
        std::net::IpAddr::V6(v) => v.to_ipv4_mapped().map(|v| public_ip(v.into())).unwrap_or(
            !v.is_loopback()
                && !v.is_unspecified()
                && !v.is_unique_local()
                && !v.is_unicast_link_local()
                && !v.is_multicast(),
        ),
    }
}
async fn check_network(p: &Principal, raw: &str) -> Result<(), String> {
    if p.scopes.iter().any(|s| s == "local_network") {
        return Ok(());
    }
    let url = url::Url::parse(raw).map_err(|_| "INVALID_URL")?;
    let host = url
        .host_str()
        .ok_or("INVALID_URL")?
        .trim_matches(['[', ']']);
    let addresses = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        tokio::net::lookup_host((host, url.port_or_known_default().ok_or("INVALID_URL")?)),
    )
    .await
    .map_err(|_| "DNS_FAILURE")?
    .map_err(|_| "DNS_FAILURE")?
    .collect::<Vec<_>>();
    if addresses.is_empty() || addresses.iter().any(|a| !public_ip(a.ip())) {
        return Err("LOCAL_NETWORK_NOT_GRANTED".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn principal(scopes: &[&str]) -> Principal {
        Principal {
            id: "t".into(),
            name: "T".into(),
            scopes: scopes.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn entry(catalog: &[Value], name: &str) -> Value {
        catalog
            .iter()
            .find(|t| t["name"] == name)
            .cloned()
            .unwrap_or_else(|| panic!("{name} is not listed"))
    }

    #[test]
    fn devtools_are_listed_with_output_schemas_and_honest_annotations() {
        let listed = catalog(&principal(&["devtools_read"]));
        assert_eq!(listed.len(), 7);
        for name in crate::commands::devtools::mcp::TOOLS {
            let tool = entry(&listed, name);
            assert_eq!(tool["inputSchema"]["additionalProperties"], false, "{name}");
            assert!(
                super::super::output_schemas::for_tool(name).is_some(),
                "{name}"
            );
            assert_eq!(tool["annotations"]["destructiveHint"], false, "{name}");
        }
        let hint = |name: &str, key: &str| entry(&listed, name)["annotations"][key].clone();
        assert_eq!(hint("dev_util", "readOnlyHint"), true);
        assert_eq!(hint("dev_util", "openWorldHint"), false);
        assert_eq!(hint("dev_scrape", "readOnlyHint"), true);
        assert_eq!(hint("dev_scrape", "openWorldHint"), true);
        assert_eq!(hint("dev_http", "readOnlyHint"), false);
        assert_eq!(hint("dev_http", "openWorldHint"), true);
        assert_eq!(hint("dev_http", "idempotentHint"), false);
        assert_eq!(hint("dev_pack", "readOnlyHint"), false);
        assert_eq!(hint("dev_ports", "readOnlyHint"), true);
        assert_eq!(hint("dev_crawl", "readOnlyHint"), false);
        assert_eq!(hint("dev_crawl", "openWorldHint"), true);
        assert_eq!(hint("dev_proxy", "readOnlyHint"), true);
        assert_eq!(hint("dev_proxy", "openWorldHint"), true);
    }

    #[test]
    fn the_proxy_pool_is_only_changed_with_the_write_scope() {
        let reader = principal(&["devtools_read"]);
        let proxy = entry(&catalog(&reader), "dev_proxy");
        assert_eq!(
            proxy["inputSchema"]["properties"]["op"]["enum"],
            json!(["status", "detect", "check"])
        );
        assert_eq!(
            validate(
                &reader,
                "dev_proxy",
                &json!({"op": "add", "proxies": ["127.0.0.1:7890"]})
            ),
            Err("INVALID_ARGUMENT: op".to_string())
        );
        let writer = principal(&["devtools_read", "devtools_write"]);
        let proxy = entry(&catalog(&writer), "dev_proxy");
        assert_eq!(proxy["annotations"]["readOnlyHint"], false);
        assert_eq!(
            validate(&writer, "dev_proxy", &json!({"op": "rotate"})),
            Ok(())
        );
    }

    #[test]
    fn research_tools_are_listed_with_output_schemas_and_honest_annotations() {
        let listed = catalog(&principal(&["research_read"]));
        assert_eq!(listed.len(), crate::commands::research::mcp::TOOLS.len());
        for name in crate::commands::research::mcp::TOOLS {
            let tool = entry(&listed, name);
            assert_eq!(tool["inputSchema"]["additionalProperties"], false, "{name}");
            assert!(
                super::super::output_schemas::for_tool(name).is_some(),
                "{name}"
            );
            assert_eq!(tool["annotations"]["destructiveHint"], false, "{name}");
        }
        let hint = |name: &str, key: &str| entry(&listed, name)["annotations"][key].clone();
        assert_eq!(hint("research_endpoints", "readOnlyHint"), true);
        assert_eq!(hint("research_endpoints", "openWorldHint"), false);
        assert_eq!(hint("research_call", "readOnlyHint"), true);
        assert_eq!(hint("research_call", "openWorldHint"), true);
        assert_eq!(hint("research_meta_ads", "readOnlyHint"), false);
        assert_eq!(hint("research_reels", "openWorldHint"), true);
        assert!(catalog(&principal(&["devtools_read", "social_read"]))
            .iter()
            .all(|tool| !tool["name"].as_str().unwrap().starts_with("research_")));
    }

    #[test]
    fn transfer_tools_are_listed_by_scope_with_output_schemas_and_honest_annotations() {
        let reader = principal(&["transfer_read"]);
        let listed = catalog(&reader);
        let names: Vec<&str> = listed.iter().filter_map(|t| t["name"].as_str()).collect();
        assert_eq!(
            names,
            vec!["transfer_servers", "transfer_list", "transfer_get"]
        );
        assert_eq!(
            entry(&listed, "transfer_servers")["inputSchema"]["properties"]["op"]["enum"],
            json!(["list"])
        );
        assert_eq!(
            validate(
                &reader,
                "transfer_servers",
                &json!({"op": "remove", "name": "nas"})
            ),
            Err("INVALID_ARGUMENT: op".to_string())
        );
        let hint =
            |list: &[Value], name: &str, key: &str| entry(list, name)["annotations"][key].clone();
        assert_eq!(hint(&listed, "transfer_servers", "readOnlyHint"), true);
        assert_eq!(hint(&listed, "transfer_list", "readOnlyHint"), true);
        assert_eq!(hint(&listed, "transfer_list", "openWorldHint"), true);
        assert_eq!(hint(&listed, "transfer_get", "readOnlyHint"), false);

        let writer = principal(&["transfer_read", "transfer_write"]);
        let listed = catalog(&writer);
        assert_eq!(listed.len(), 5);
        for name in crate::commands::transfer::mcp::TOOLS {
            let tool = entry(&listed, name);
            assert_eq!(tool["inputSchema"]["additionalProperties"], false, "{name}");
            assert!(
                super::super::output_schemas::for_tool(name).is_some(),
                "{name}"
            );
        }
        assert_eq!(hint(&listed, "transfer_servers", "readOnlyHint"), false);
        assert_eq!(hint(&listed, "transfer_manage", "destructiveHint"), true);
        assert_eq!(hint(&listed, "transfer_put", "destructiveHint"), false);
        assert_eq!(hint(&listed, "transfer_put", "idempotentHint"), false);
        assert_eq!(
            validate(
                &writer,
                "transfer_list",
                &json!({"server": "nas", "password": "x"})
            ),
            Err("UNKNOWN_ARGUMENT: password".to_string())
        );
        assert_eq!(
            validate(
                &writer,
                "transfer_manage",
                &json!({"server": "nas", "path": "/a"})
            ),
            Err("MISSING_ARGUMENT: \"op\"".to_string())
        );
        assert_eq!(catalog(&principal(&["transfer_write"])).len(), 2);
    }

    #[test]
    fn quick_tools_setters_exist_only_with_the_change_scope() {
        let reader = principal(&["system_read"]);
        let tool = entry(&catalog(&reader), "quick_tools");
        assert_eq!(
            tool["inputSchema"]["properties"]["op"]["enum"],
            json!(crate::commands::quicktools::READ_OPS)
        );
        assert_eq!(tool["annotations"]["readOnlyHint"], true);
        assert!(crate::mcp::output_schemas::for_tool("quick_tools").is_some());
        assert_eq!(
            validate(
                &reader,
                "quick_tools",
                &json!({"op": "awake_set", "mode": "1h"})
            ),
            Err("INVALID_ARGUMENT: op".to_string())
        );
        assert_eq!(
            validate(&reader, "quick_tools", &json!({"op": "clipboard_list"})),
            Err("INVALID_ARGUMENT: op".to_string())
        );
        let writer = principal(&["system_read", "system_change"]);
        let tool = entry(&catalog(&writer), "quick_tools");
        assert_eq!(tool["annotations"]["readOnlyHint"], false);
        assert_eq!(tool["annotations"]["destructiveHint"], false);
        assert_eq!(
            validate(
                &writer,
                "quick_tools",
                &json!({"op": "volume_set", "id": "42", "volume": 30})
            ),
            Ok(())
        );
        assert_eq!(
            validate(
                &writer,
                "quick_tools",
                &json!({"op": "volume_set", "id": "42", "volume": 101})
            ),
            Err("INVALID_ARGUMENT: volume".to_string())
        );
        assert_eq!(
            validate(&writer, "quick_tools", &json!({"op": "clipboard_list"})),
            Err("INVALID_ARGUMENT: op".to_string())
        );
        assert!(catalog(&principal(&["system_change"]))
            .iter()
            .all(|t| t["name"] != "quick_tools"));
    }

    #[test]
    fn kill_exists_only_with_the_write_scope_and_is_marked_destructive() {
        let reader = principal(&["devtools_read"]);
        let ports = entry(&catalog(&reader), "dev_ports");
        assert_eq!(
            ports["inputSchema"]["properties"]["op"]["enum"],
            json!(["list"])
        );
        assert_eq!(
            validate(&reader, "dev_ports", &json!({"op": "kill", "port": 3000})),
            Err("INVALID_ARGUMENT: op".to_string())
        );
        let writer = principal(&["devtools_read", "devtools_write"]);
        let ports = entry(&catalog(&writer), "dev_ports");
        assert_eq!(
            ports["inputSchema"]["properties"]["op"]["enum"],
            json!(["list", "kill"])
        );
        assert_eq!(ports["annotations"]["destructiveHint"], true);
        assert_eq!(ports["annotations"]["readOnlyHint"], false);
        assert_eq!(
            validate(&writer, "dev_ports", &json!({"op": "kill", "port": 3000})),
            Ok(())
        );
        assert!(catalog(&principal(&["devtools_write"])).is_empty());
    }

    #[test]
    fn op_is_required_and_unknown_arguments_are_rejected() {
        let reader = principal(&["devtools_read"]);
        assert_eq!(
            validate(&reader, "dev_util", &json!({"text": "a"})),
            Err("MISSING_ARGUMENT: \"op\"".to_string())
        );
        assert_eq!(
            validate(&reader, "dev_util", &json!({"op": "uuid", "nope": 1})),
            Err("UNKNOWN_ARGUMENT: nope".to_string())
        );
        assert_eq!(
            validate(
                &reader,
                "dev_util",
                &json!({"op": "timestamp", "value": 1700000000})
            ),
            Ok(())
        );
        assert_eq!(
            validate(
                &reader,
                "dev_http",
                &json!({"method": "GET", "url": "https://example.com", "headers": {"accept": "*/*"}})
            ),
            Ok(())
        );
    }
}
