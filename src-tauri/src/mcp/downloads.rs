//! External download projection over the desktop queue (not another engine).
use super::{
    policy::{self, Principal},
    ToolDef,
};
use omniget_core::platforms::PlatformDownloader;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

pub fn catalog(p: &Principal) -> Vec<Value> {
    tools(p).into_iter().map(|tool| {
        let writes = matches!(policy::scope(tool.name), Some("enqueue" | "control" | "orchestration_control" | "orchestration_create"))
            || matches!(tool.name, "auth_connection_request" | "diagnostic_bundle_create");
        let open_world = matches!(tool.name, "download_enqueue" | "downloads_batch_enqueue" | "download_retry" | "download_resume" | "media_inspect" | "media_collection_list");
        let mut value = serde_json::to_value(&tool).expect("static tool schema serializes");
        // MCP semantics: destructive = may undo or discard existing work
        // (cancel/pause); creating or enqueueing is additive and, with its
        // idempotency key, idempotent.
        let destructive = writes && matches!(tool.name, "download_cancel" | "download_pause" | "missions_cancel" | "missions_pause");
        value["annotations"] = json!({"readOnlyHint":!writes,"destructiveHint":destructive,"idempotentHint":true,"openWorldHint":open_world});
        value
    }).collect()
}

pub fn tools(p: &Principal) -> Vec<ToolDef> {
    let mut list = super::tools()
        .into_iter()
        .filter(|t| policy::allowed(p, t.name))
        .collect::<Vec<_>>();
    for (name,description,props,required) in [
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
    list.extend(
        super::orchestration::tools()
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
    if super::orchestration::tools().iter().any(|t| t.name == name) {
        if !policy::allowed(p, name) {
            return Err("TOOL_NOT_AUTHORIZED".into());
        }
        return super::orchestration::call(app, p, name, a).await;
    }
    validate(p, name, &a)?;
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
            let ids = policy::owned(p)?;
            let status = a["status"].as_str().unwrap_or("");
            let snapshot = super::queue_snapshot(app).await;
            let queued: std::collections::HashSet<u64> = snapshot.iter().map(|i| i.id).collect();
            let mut items = snapshot
                .into_iter()
                .filter(|i| {
                    ids.contains(&i.id)
                        && (status.is_empty() || super::status_key(&i.status) == status)
                })
                .map(|i| serde_json::to_value(i).map_err(|e| e.to_string()))
                .collect::<Result<Vec<_>, _>>()?;
            // Jobs the in-memory queue lost (crash, restart) stay visible
            // from the journal until they are reconciled (D-08).
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
                // Journal-only: nothing will change until the client acts.
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
                // A crash leaves no warning: the last progress lines are the
                // evidence of how far the attempt got.
                let n = page.events.len().saturating_sub(8);
                evidence = page.events[n..].iter().map(excerpt).collect();
            }
            // The durable terminal receipt is the same message the queue
            // status and download_retry admission were derived from; it leads
            // so the diagnosis class cannot disagree with them.
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
        "artifact_access" => super::artifact_access::call(app, p, &a).await?,
        "download_history" => {
            let ids = policy::owned(p)?;
            let mut items = crate::core::queue_history::list()
                .into_iter()
                .filter(|i| ids.contains(&i.id))
                .map(|i| serde_json::to_value(i).map_err(|e| e.to_string()))
                .collect::<Result<Vec<_>, _>>()?;
            // An interrupted job never reached history or the queue: list it
            // from the journal so it cannot silently disappear (D-08).
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
                // Every produced file of the job (a carousel writes several),
                // read from its exclusive folder; the engine result names only
                // the last one (bench D1). Each file gets its own grant/digest.
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
                    // A name the source forced to start with dots is still the
                    // output; offer a visible name to save it under (D-07).
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
            // Success with nothing to hand over is a failure to report, not
            // an empty list next to a success status (D-07).
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
                // Resume is a new attempt from zero; say so and make it true.
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
                // A refusal here (`cannot pause download … (complete)`) ran no
                // effect: `keyed` frees the key instead of poisoning it (N-4).
                let mut value = super::call(app, name, internal).await?;
                if name == "download_cancel" {
                    // Cancelling a paused job: its attempt was already settled as
                    // paused with partials kept; settle it as cancelled and clean.
                    // The cancel already happened, so nothing here may fail it.
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
/// `omniget_health` over HTTP: answering at all means the desktop was
/// reached. Same facts the stdio adapter adds (reachability, versions,
/// protocol), so a client sees one shape on both transports (D-14).
pub(crate) fn health(negotiated: &str) -> Value {
    json!({
        "app": "running",
        "reachable": true,
        "queue": "available",
        "remote": "disabled",
        "server": {"name": "OmniGet", "version": env!("CARGO_PKG_VERSION")},
        "protocolVersion": negotiated,
        "supportedProtocolVersions": super::SUPPORTED_PROTOCOLS,
    })
}
/// Reconcile durable authority with the one desktop queue. No URL lookup and
/// no personal downloader fallback are permitted for an external intent.
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
            // Same shape as the first answer (D-17): the item is the live queue
            // entry when there is one, else the journal view.
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
    // Execution and queue matching use the executable URL; only this process
    // memory holds it in clear.
    let intent = super::download_intents::hydrate(intent)?;
    // A prepared new attempt replaces only the inert terminal placeholder.
    // This also reconstructs the worker after app restart, never the Noop or
    // personal downloader stored in queue history.
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
        // A restored placeholder is not evidence that an interrupted effect
        // completed. Preserve the ID without making an execution claim.
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
/// Replay of an intent whose effect may have run: visible, never re-executed,
/// with the typed way out (D-08).
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
        // Legacy completed receipts remain readable; an unfinished legacy
        // receipt has no snapshot and reserve must return Unknown.
        if policy::receipt_exists(p, "download_enqueue", key)? {
            return policy::reserve(p, "download_enqueue", key, a)?.ok_or("OUTCOME_UNKNOWN".into());
        }
        let url = checked_url(a)?;
        check_network(p, &url).await?;
        // Only a live job manages its URL (same rule as the queue's has_url):
        // a failed or cancelled job must not dead-end a fresh enqueue.
        let snapshot = super::queue_snapshot_raw(app).await;
        if snapshot.iter().any(|i| i.url == url && live(&i.status)) {
            return Err("URL_ALREADY_MANAGED".into());
        }
        // A job interrupted by a crash may still hold this URL's output: a
        // second download must wait until that one is reconciled (D-08).
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
/// The queue entry when the in-memory queue holds the job, else a queue-shaped
/// view from the durable journal (`true` = live queue entry).
async fn current_item(app: &AppHandle, id: u64) -> Result<(Value, bool), String> {
    if let Ok(item) = super::queue_item(app, id).await {
        return Ok((serde_json::to_value(item).map_err(|e| e.to_string())?, true));
    }
    let intent =
        super::download_intents::load(id)?.ok_or_else(|| format!("no download with id {id}"))?;
    Ok((journal_item(&intent), false))
}
/// Queue-shaped view of a job the in-memory queue does not hold (restart,
/// crash). Only durable evidence; the URL is the redacted one.
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
/// Owned, unsettled intents that neither the queue nor history holds.
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
/// A login problem points at `auth_connection_request` (as the diagnostic
/// bundle does), placed before the generic local-action step. Honest about
/// the limit: MCP downloads run in the confined worker, which never reads
/// local logins, so connecting helps downloads the person starts in OmniGet
/// and a retry from this client still runs without the login.
fn with_auth_action(mut actions: Vec<Value>, code: &str, host: Option<&str>) -> Vec<Value> {
    if !matches!(code, "AUTH_REQUIRED" | "AUTH_EXPIRED") {
        return actions;
    }
    let mut args = json!({});
    let mut requires = vec!["reason", "idempotencyKey"];
    match host {
        // Only a host the tool accepts is suggested (N-6).
        Some(h) if super::auth::valid_host(h).is_some() => args["host"] = json!(h),
        // A literal address or `localhost` is a device, not a platform login:
        // the tool would refuse it, so there is no login to ask for.
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
/// Typed next steps derived from the durable state and the diagnosis (D-10).
/// Tools that change state also need a fresh `idempotencyKey`.
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
/// `download_retry` with `strategyId=reconcile`: settle a job interrupted by a
/// crash from its folder. Never downloads; idempotent under its key.
/// One idempotent mutation under `key`: a replay returns the stored receipt;
/// a refusal (`effect` failed, so it ran nothing) releases the key, so a
/// replay meets the same refusal instead of `OUTCOME_UNKNOWN` (N-4); a success
/// is sanitized and stored as the receipt.
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
        // Only a job no process of this app is running can be judged.
        if matches!(queued,Some(crate::core::queue::QueueStatus::Active|crate::core::queue::QueueStatus::Seeding|crate::core::queue::QueueStatus::Paused)) {
            return Err::<Value,String>("RECONCILE_NOT_NEEDED: the job is still managed by the queue".into());
        }
        let principal=p.clone();
        let (outcome,removed)=tokio::task::spawn_blocking(move||super::download_intents::reconcile(Some(&principal),id)).await.map_err(|e|e.to_string())??;
        let intent=super::download_intents::load(id)?.ok_or("DOWNLOAD_INTENT_MISSING")?;
        let receipt=super::download_intents::receipt(id)?;
        // The settled job returns to history and the queue (N-2).
        settle_reconciled(app,id).await;
        Ok(json!({"download_id":id,"attempt":intent.attempt,"outcome":outcome,"removedLeftovers":removed.len(),"item":journal_item(&intent),"receipt":receipt,"nextActions":actions_for(id,None)}))
    }).await
}
/// History entry of a job settled by crash reconciliation, from its durable
/// intent and receipt; `None` while it is not a reconciled terminal job.
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
/// A job settled by reconciliation (MCP `download_retry strategyId=reconcile`
/// or the UI "Resume downloads") leaves the journal-only view: it goes into
/// history with its outcome and back into the queue as a finished item, so
/// neither the MCP tools nor the UI lose it (N-2). Idempotent.
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
/// A job manages its URL only while it can still run or produce output.
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

#[cfg(test)]
mod tests {
    use super::*;
    fn principal() -> Principal {
        Principal {
            id: "test".into(),
            name: "test".into(),
            scopes: policy::SCOPES.iter().map(|s| s.to_string()).collect(),
        }
    }
    #[test]
    fn auth_failures_point_to_auth_connection_request_honestly() {
        let base = vec![
            json!({"action":"local_user_action","code":"open_local_account_flow","requiresLocalUser":true}),
        ];
        for code in ["AUTH_REQUIRED", "AUTH_EXPIRED"] {
            let out = with_auth_action(base.clone(), code, Some("cdn.example.com"));
            assert_eq!(out[0]["tool"], "auth_connection_request", "{code}");
            assert_eq!(out[0]["arguments"]["host"], "cdn.example.com");
            assert_eq!(out[0]["usedByExternalDownloads"], false);
            assert_eq!(out[0]["retryUsesLogin"], false);
            assert!(out[0]["note"].as_str().unwrap().contains("confined worker"));
            assert_eq!(out[1]["action"], "local_user_action");
        }
        // The diagnosis the classifier gives a platform 401 (D-09) gets it.
        let code = crate::core::root_cause::machine_diagnose("HTTP Error 401: Unauthorized").code;
        assert_eq!(
            with_auth_action(vec![], code, None)[0]["tool"],
            "auth_connection_request"
        );
        assert_eq!(with_auth_action(base.clone(), "RATE_LIMITED", None), base);
        // The action validates against the tool's own input schema keys.
        let schema = super::super::auth::tools()
            .into_iter()
            .find(|t| t.name == "auth_connection_request")
            .unwrap()
            .input_schema;
        let out = with_auth_action(vec![], "AUTH_REQUIRED", Some("x.example"));
        for key in out[0]["arguments"].as_object().unwrap().keys() {
            assert!(schema["properties"].get(key).is_some(), "{key}");
        }
    }
    #[test]
    fn http_health_says_reachable_with_versions_and_validates() {
        let h = health("2025-06-18");
        assert_eq!(h["reachable"], true);
        assert_eq!(h["app"], "running");
        assert_eq!(h["server"]["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(h["protocolVersion"], "2025-06-18");
        assert_eq!(
            h["supportedProtocolVersions"],
            json!(["2025-06-18", "2025-03-26"])
        );
        assert!(super::super::output_schemas::for_tool("omniget_health").is_some());
        // Outside an HTTP request (no header) the spec default applies.
        assert_eq!(super::super::request_protocol(), "2025-03-26");
    }
    #[tokio::test]
    async fn http_health_reports_the_request_header_revision() {
        let v = super::super::REQUEST_PROTOCOL
            .scope(Some("2025-06-18".into()), async {
                super::super::request_protocol()
            })
            .await;
        assert_eq!(v, "2025-06-18");
    }
    #[test]
    fn closed_schemas_block_ungranted_and_arbitrary_inputs() {
        let p = principal();
        assert!(validate(&p, "agent_delegate", &json!({})).is_err());
        assert!(validate(
            &p,
            "download_enqueue",
            &json!({"url":"https://example.com"})
        )
        .is_err());
        assert!(validate(
            &p,
            "download_enqueue",
            &json!({"url":"https://example.com","idempotencyKey":"a","args":["--exec","sh"]})
        )
        .is_err());
        assert!(validate(
            &p,
            "downloads_batch_enqueue",
            &json!({"urls":vec!["https://example.com";21],"idempotencyKey":"a"})
        )
        .is_err());
        assert!(validate(
            &p,
            "download_wait",
            &json!({"download_id":1,"timeoutMs":25001})
        )
        .is_err());
        assert!(validate(&p, "download_logs", &json!({"download_id":1,"limit":201})).is_err());
        assert!(validate(
            &p,
            "download_enqueue",
            &json!({"url":"https://example.com","idempotencyKey":"a","maxHeight":720})
        )
        .is_ok());
    }
    #[test]
    fn annotations_do_not_label_download_mutations_read_only() {
        let catalog = catalog(&principal());
        for name in ["download_enqueue", "download_cancel", "download_retry"] {
            let tool = catalog.iter().find(|t| t["name"] == name).unwrap();
            assert_eq!(tool["annotations"]["readOnlyHint"], false);
        }
        let status = catalog
            .iter()
            .find(|t| t["name"] == "download_status")
            .unwrap();
        assert_eq!(status["annotations"]["readOnlyHint"], true);
        assert_eq!(status["annotations"]["openWorldHint"], false);
    }
    #[test]
    fn failed_or_cancelled_jobs_do_not_hold_their_url() {
        use crate::core::queue::QueueStatus;
        for status in [
            QueueStatus::Queued,
            QueueStatus::Active,
            QueueStatus::Paused,
            QueueStatus::Seeding,
        ] {
            assert!(live(&status));
        }
        for status in [
            QueueStatus::Error {
                message: "BROKEN_SOURCE".into(),
                retryable: true,
            },
            QueueStatus::Error {
                message: "Cancelled".into(),
                retryable: false,
            },
            QueueStatus::Complete { success: false },
            QueueStatus::Complete { success: true },
        ] {
            assert!(!live(&status));
        }
    }
    #[test]
    fn next_actions_are_typed_from_state_and_diagnosis() {
        // D-10: nextActions used to be a constant [].
        use super::super::download_intents::TerminalReceipt;
        let receipt = |outcome: &str, retryable: bool| TerminalReceipt {
            outcome: outcome.into(),
            error: None,
            file_path: None,
            bytes: None,
            ended_at: 1,
            next_allowed_at: 42,
            retryable,
            files: vec![],
            reconciled: None,
        };
        let first = |v: Vec<Value>| v[0].clone();
        let interrupted = first(next_actions(7, "unknown", None, None, 0, "reconcile"));
        assert_eq!(
            (
                interrupted["action"].as_str(),
                interrupted["arguments"]["strategyId"].as_str()
            ),
            (Some("reconcile"), Some("reconcile"))
        );
        assert_eq!(interrupted["startsDownload"], false);
        let running = crate::core::queue::QueueStatus::Active;
        assert_eq!(
            first(next_actions(7, "executing", Some(&running), None, 0, ""))["action"],
            "wait"
        );
        assert_eq!(
            first(next_actions(7, "prepared", None, None, 0, ""))["action"],
            "replay_enqueue"
        );
        let retry = first(next_actions(
            7,
            "terminal",
            None,
            Some(&receipt("failed", true)),
            1,
            "retry_transient",
        ));
        assert_eq!(
            (retry["tool"].as_str(), retry["notBeforeMs"].as_u64()),
            (Some("download_retry"), Some(42))
        );
        assert_eq!(
            first(next_actions(
                7,
                "terminal",
                None,
                Some(&receipt("failed", true)),
                2,
                "retry_transient"
            ))["reason"],
            "RETRY_LIMIT_REACHED"
        );
        let resume = first(next_actions(
            7,
            "terminal",
            None,
            Some(&receipt("paused", true)),
            2,
            "",
        ));
        assert_eq!(
            (resume["tool"].as_str(), resume["continuation"].as_str()),
            (Some("download_resume"), Some("restart"))
        );
        let local = first(next_actions(
            7,
            "terminal",
            None,
            Some(&receipt("failed", false)),
            0,
            "open_local_account_flow",
        ));
        assert_eq!(
            (local["code"].as_str(), local["requiresLocalUser"].as_bool()),
            (Some("open_local_account_flow"), Some(true))
        );
        assert_eq!(
            first(next_actions(
                7,
                "terminal",
                None,
                Some(&receipt("success", false)),
                0,
                ""
            ))["tool"],
            "download_artifacts"
        );
        assert_eq!(
            first(next_actions(
                7,
                "terminal",
                None,
                Some(&receipt("cancelled", false)),
                0,
                ""
            ))["tool"],
            "download_enqueue"
        );
        let logs = next_actions(
            7,
            "terminal",
            None,
            Some(&receipt("failed", false)),
            0,
            "inspect_more_evidence",
        );
        assert!(logs.iter().any(|a| a["tool"] == "download_logs"));
    }
    #[test]
    fn retry_schema_offers_crash_reconciliation() {
        let tools = tools(&principal());
        let retry = tools.iter().find(|t| t.name == "download_retry").unwrap();
        assert!(retry.input_schema["properties"]["strategyId"]["enum"]
            .as_array()
            .unwrap()
            .contains(&json!("reconcile")));
    }
    #[test]
    fn auth_action_only_suggests_a_host_the_tool_accepts() {
        // N-6: a literal address was suggested and then refused with
        // `-32602 INVALID_ARGUMENT: host`.
        let base = vec![
            json!({"action":"local_user_action","code":"open_local_account_flow","requiresLocalUser":true}),
        ];
        for device in ["127.0.0.1", "10.0.0.2", "::1", "localhost"] {
            assert_eq!(
                with_auth_action(base.clone(), "AUTH_REQUIRED", Some(device)),
                base,
                "{device}"
            );
        }
        let out = with_auth_action(base.clone(), "AUTH_REQUIRED", Some("cdn.example.com"));
        assert!(
            super::super::auth::target(&out[0]["arguments"]).is_ok(),
            "{}",
            out[0]
        );
        // No host known: the action says the client must supply one.
        let out = with_auth_action(vec![], "AUTH_REQUIRED", None);
        assert_eq!(out[0]["arguments"], json!({}));
        assert!(out[0]["requires"]
            .as_array()
            .unwrap()
            .contains(&json!("host")));
    }
    #[tokio::test]
    async fn refused_mutation_leaves_its_key_reusable() {
        // N-4: a refused pause/cancel answered OPERATION_FAILED once and then
        // OUTCOME_UNKNOWN on every replay of the same key.
        let dir =
            std::env::temp_dir().join(format!("omniget-keyed-{}", uuid::Uuid::new_v4().simple()));
        super::super::policy::TEST_DIR.with(|d| *d.borrow_mut() = Some(dir.clone()));
        let p = principal();
        let a = json!({"download_id":7,"idempotencyKey":"k1"});
        let refuse = || async { Err::<Value, String>("cannot pause download 7 (complete)".into()) };
        for _ in 0..3 {
            let err = keyed(&p, "download_pause", "k1", &a, refuse())
                .await
                .unwrap_err();
            assert_eq!(err, "cannot pause download 7 (complete)");
        }
        // Once the operation succeeds, the key holds that receipt for good.
        let ok = keyed(&p, "download_pause", "k1", &a, async {
            Ok(json!({"paused":7}))
        })
        .await
        .unwrap();
        assert_eq!(ok, json!({"paused":7}));
        let replay = keyed(&p, "download_pause", "k1", &a, refuse())
            .await
            .unwrap();
        assert_eq!(replay, json!({"paused":7}));
        // Same key, other arguments: still a conflict.
        let other = json!({"download_id":8,"idempotencyKey":"k1"});
        assert_eq!(
            keyed(&p, "download_pause", "k1", &other, refuse())
                .await
                .unwrap_err(),
            "IDEMPOTENCY_CONFLICT"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
    #[test]
    fn reconciled_jobs_return_to_history_and_the_queue() {
        // N-2: after reconcile a job lived only in download_status.
        use super::super::download_intents::{Intent, TerminalReceipt};
        let intent: Intent = serde_json::from_value(json!({
            "job_id": 41, "principal": "test", "key": "k", "stage": "terminal",
            "destination": "/tmp/job-41", "destination_identity": null, "attempt": 1,
            "options": {"url":"https://cdn.example.com/v.mp4?token=SYNTHETIC_SECRET","sealed":false,"mode":null,"quality":null,
                "format_id":null,"audio_format":null,"subtitles":false,"auto_subtitles":false,"fragments":4,
                "parent":"/tmp","parent_identity":{"device":1,"inode":2}}
        })).unwrap_or_else(|e| panic!("intent fixture: {e}"));
        let receipt =
            |outcome: &str, error: Option<&str>, reconciled: Option<&str>| TerminalReceipt {
                outcome: outcome.into(),
                error: error.map(str::to_owned),
                file_path: (outcome == "success").then(|| "/tmp/job-41/v.mp4".into()),
                bytes: (outcome == "success").then_some(1234),
                ended_at: 1_700_000_000_000,
                next_allowed_at: 0,
                retryable: outcome != "success",
                files: vec![],
                reconciled: reconciled.map(str::to_owned),
            };
        // An ordinary terminal job already reached history through the queue.
        assert!(
            reconciled_history_entry(&intent, &receipt("failed", Some("HTTP 500"), None)).is_none()
        );
        let failed = reconciled_history_entry(
            &intent,
            &receipt(
                "failed",
                Some("INTERRUPTED: the app stopped"),
                Some("no_complete_artifact"),
            ),
        )
        .unwrap();
        assert!(!failed.success);
        assert_eq!(
            failed.error.as_deref(),
            Some("INTERRUPTED: the app stopped")
        );
        assert_eq!(failed.completed_at, 1_700_000_000);
        assert_eq!(failed.platform, "mcp_worker");
        let shown = serde_json::to_string(&failed).unwrap();
        assert!(!shown.contains("SYNTHETIC_SECRET"), "{shown}");
        let done = reconciled_history_entry(
            &intent,
            &receipt("success", None, Some("artifact_present_without_partials")),
        )
        .unwrap();
        assert!(done.success && done.error.is_none());
        assert_eq!(
            (done.file_path.as_deref(), done.file_size_bytes),
            (Some("/tmp/job-41/v.mp4"), Some(1234))
        );
        // Back into the queue once, with the receipt's retry verdict.
        let mut q = crate::core::queue::DownloadQueue::new(3);
        assert!(q.hydrate_entry(&failed, true));
        assert!(!q.hydrate_entry(&failed, true), "idempotent");
        let item = &q.get_state()[0];
        assert_eq!(item.id, 41);
        assert!(matches!(
            item.status,
            crate::core::queue::QueueStatus::Error {
                retryable: true,
                ..
            }
        ));
    }
    #[test]
    fn exports_redact_nested_structured_credentials() {
        let v = sanitize(
            json!({"data":{"Cookie":"SYNTHETIC_SECRET"},"log":["Authorization: Basic SYNTHETIC_SECRET"]}),
        );
        assert!(!v.to_string().contains("SYNTHETIC_SECRET"));
    }
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
mod network_tests {
    use super::*;
    #[test]
    fn private_and_mapped_networks_are_denied() {
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.1.1",
            "192.168.2.2",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "::1",
            "::ffff:127.0.0.1",
            "fe80::1",
            "fc00::1",
        ] {
            assert!(!public_ip(ip.parse().unwrap()), "{ip}");
        }
        assert!(public_ip("1.1.1.1".parse().unwrap()));
    }
}
