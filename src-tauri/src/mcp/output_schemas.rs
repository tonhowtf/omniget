use serde_json::{json, Value};

pub fn apply(mut tools: Vec<Value>) -> Vec<Value> {
    for tool in &mut tools {
        let schema = tool["name"].as_str().and_then(for_tool);
        if let (Some(schema), Some(obj)) = (schema, tool.as_object_mut()) {
            obj.insert("outputSchema".into(), schema);
        }
    }
    tools
}

pub fn for_tool(name: &str) -> Option<Value> {
    own(name)
        .or_else(|| crate::native_agent::output_schema(name))
        .or_else(|| super::auth::output_schema(name))
        .or_else(|| super::bundle::output_schema(name))
        .or_else(|| super::artifact_access::output_schema(name))
        .or_else(|| super::catalog_output_schema(name))
        .or_else(|| super::washmybelly::output_schema(name))
        .or_else(|| super::telegram::output_schema(name))
        .or_else(|| super::phonechat::output_schema(name))
        .or_else(|| crate::commands::social::tools::output_schema(name))
        .or_else(|| crate::commands::research::mcp::output_schema(name))
        .or_else(|| super::memory::output_schema(name))
        .or_else(|| super::canvas::output_schema(name))
        .map(with_error_branch)
}

fn error_envelope() -> Value {
    json!({"type":"object","properties":{"code":{"type":"string"},"message":{"type":"string"}},"required":["code","message"]})
}

fn with_error_branch(mut schema: Value) -> Value {
    let combined = schema.get("anyOf").is_some()
        || schema.get("oneOf").is_some()
        || schema.get("allOf").is_some();
    if combined || !schema["properties"].is_object() {
        return json!({"type":"object","anyOf":[schema,{"type":"object","properties":{"error":error_envelope()},"required":["error"]}]});
    }
    let required = schema
        .as_object_mut()
        .and_then(|o| o.remove("required"))
        .unwrap_or_else(|| json!([]));
    schema["properties"]["error"] = error_envelope();
    schema["anyOf"] = json!([{"required": required}, {"required": ["error"]}]);
    schema
}

fn ty(t: &str) -> Value {
    json!({"type": t})
}
fn nullable(t: &str) -> Value {
    json!({"type": [t, "null"]})
}
fn or_null(schema: Value) -> Value {
    json!({"anyOf": [{"type": "null"}, schema]})
}
fn arr(items: Value) -> Value {
    json!({"type": "array", "items": items})
}
fn obj(properties: Value, required: &[&str]) -> Value {
    json!({"type": "object", "properties": properties, "required": required})
}

fn item() -> Value {
    obj(
        json!({
            "id": ty("integer"),
            "url": ty("string"),
            "platform": ty("string"),
            "title": nullable("string"),
            "status": obj(json!({"type": ty("string"), "data": ty("object")}), &["type"]),
            "percent": nullable("number"),
            "speed_bytes_per_sec": nullable("number"),
            "downloaded_bytes": ty("integer"),
            "total_bytes": nullable("integer"),
            "file_path": nullable("string"),
            "file_size_bytes": nullable("integer"),
            "file_count": nullable("integer"),
            "thumbnail_url": nullable("string"),
            "kind": ty("string"),
            "external": ty("boolean"),
            "eta_seconds": ty("integer"),
            "quality": ty("string"),
            "download_mode": ty("string"),
            "author": ty("string"),
            "duration_seconds": nullable("number"),
            "phase": ty("string"),
            "stream": ty("object"),
            "streams_done": arr(ty("object")),
            "planned_formats": arr(ty("string")),
            "fragment_index": ty("integer"),
            "fragment_count": ty("integer"),
            "started_at_ms": ty("integer"),
            "finished_at_ms": ty("integer"),
            "command": ty("object"),
            "attempt": ty("integer"),
            "source": ty("string"),
        }),
        &["id", "url", "status"],
    )
}
fn next_actions() -> Value {
    arr(ty("object"))
}
fn intent_receipt() -> Value {
    obj(
        json!({
            "download_id": ty("integer"),
            "attempt": ty("integer"),
            "outcome": ty("string"),
            "item": or_null(item()),
            "receipt": or_null(ty("object")),
            "recovery": ty("string"),
            "resume": ty("object"),
            "removedLeftovers": ty("integer"),
            "nextActions": next_actions(),
        }),
        &["outcome"],
    )
}
fn diagnosis() -> Value {
    obj(
        json!({
            "code": ty("string"),
            "confidence": ty("string"),
            "phase": ty("string"),
            "retryable": ty("string"),
            "nextAction": ty("string"),
            "unknowns": arr(ty("string")),
        }),
        &["code", "retryable"],
    )
}
fn mission_summary() -> Value {
    obj(
        json!({
            "missionId": ty("string"),
            "state": ty("string"),
            "objective": ty("string"),
            "criteriaVersion": ty("integer"),
            "revision": ty("integer"),
            "createdMs": ty("integer"),
            "updatedMs": ty("integer"),
            "eventsDropped": ty("integer"),
            "verification": ty("string"),
        }),
        &["missionId", "state"],
    )
}
fn derived_bot() -> Value {
    obj(
        json!({
            "botId": ty("string"),
            "grantId": ty("string"),
            "sourceExecutorId": ty("string"),
            "tools": arr(ty("string")),
            "state": ty("string"),
            "reason": ty("string"),
        }),
        &["botId", "state"],
    )
}
fn room() -> Value {
    obj(
        json!({
            "roomId": ty("string"),
            "grantId": ty("string"),
            "title": ty("string"),
            "memberBotIds": arr(ty("string")),
            "coordinatorId": nullable("string"),
            "limits": ty("object"),
            "grantActive": ty("boolean"),
        }),
        &["roomId", "memberBotIds"],
    )
}

fn own(name: &str) -> Option<Value> {
    Some(match name {
        "omniget_capabilities" => obj(
            json!({
                "protocol": ty("string"),
                "scopes": arr(ty("string")),
                "transports": arr(ty("string")),
                "remote": ty("boolean"),
                "remoteGateway": ty("object"),
                "executionIsolation": ty("object"),
                "currentBuildClientValidation": ty("string"),
                "historicallyValidatedClients": arr(ty("object")),
                "limitations": arr(ty("string")),
                "downloadLifecycle": ty("object"),
            }),
            &["protocol", "scopes", "transports"],
        ),
        "dev_pack" => {
            let plan = json!({
                "root": ty("string"),
                "files": arr(obj(json!({"path": ty("string"), "tokens": ty("integer"), "mode": ty("string")}), &["path", "tokens", "mode"])),
                "total": ty("integer"),
                "truncated": ty("boolean"),
                "excluded": arr(obj(json!({"path": ty("string"), "reason": ty("string"), "summary": ty("string")}), &["path", "reason"])),
                "secrets": arr(obj(json!({"path": ty("string"), "line": nullable("integer"), "rule": ty("string")}), &["path", "rule"])),
                "reduced": arr(obj(json!({"path": ty("string"), "to": ty("string"), "tokensBefore": ty("integer"), "tokensAfter": ty("integer")}), &["path", "to"])),
                "degradeOrder": arr(ty("string")),
                "totals": obj(json!({"files": ty("integer"), "tokens": ty("integer"), "excluded": ty("integer"), "secrets": ty("integer"), "notIncluded": ty("integer"), "overBudget": ty("boolean")}), &["files", "tokens"]),
            });
            let mut all = plan.clone();
            all["path"] = ty("string");
            all["tokens"] = ty("integer");
            all["bytes"] = ty("integer");
            all["sha256"] = ty("string");
            all["manifest"] = obj(plan, &["files", "totals"]);
            obj(all, &[])
        }
        "dev_scrape" => obj(
            json!({
                "url": ty("string"), "finalUrl": ty("string"), "status": ty("integer"), "via": ty("string"),
                "title": nullable("string"), "format": ty("string"), "content": ty("string"), "tokens": ty("integer"),
                "note": nullable("string"), "removed": ty("object"),
                "hash": ty("string"), "unchanged": ty("boolean"),
                "pages": arr(ty("object")), "failed": ty("integer"),
                "structured": arr(obj(json!({"kind": ty("string"), "id": nullable("string"), "chars": ty("integer"), "truncated": ty("boolean"), "data": {}, "preview": nullable("string")}), &["kind", "truncated"])),
                "matches": arr(ty("string")),
                "relocated": ty("string"),
                "proxy": ty("string"), "attempts": ty("integer"),
                "found": arr(obj(json!({"text": ty("string"), "selector": ty("string")}), &["text", "selector"])),
                "selector": ty("string"), "selectorMatches": ty("integer"),
                "total": ty("integer"), "truncated": ty("boolean"), "untrusted": ty("boolean"),
            }),
            &["total", "truncated", "untrusted"],
        ),
        "dev_crawl" => obj(
            json!({
                "output": ty("string"), "start": ty("string"),
                "pages": arr(obj(json!({"url": ty("string"), "title": nullable("string"), "file": ty("string"), "tokens": ty("integer"), "depth": ty("integer")}), &["url", "file", "tokens", "depth"])),
                "total": ty("integer"), "truncated": ty("boolean"), "savedNow": ty("integer"),
                "failed": arr(obj(json!({"url": ty("string"), "code": ty("string")}), &["url", "code"])),
                "failedTotal": ty("integer"), "duplicates": ty("integer"), "skippedByRobots": ty("integer"),
                "dropped": ty("integer"), "queued": ty("integer"), "done": ty("boolean"),
                "stopped": ty("string"), "retryAfterS": ty("integer"), "nextStep": ty("string"), "untrusted": ty("boolean"),
            }),
            &[
                "output",
                "pages",
                "total",
                "truncated",
                "queued",
                "done",
                "nextStep",
                "untrusted",
            ],
        ),
        "dev_proxy" => obj(
            json!({
                "enabled": ty("boolean"), "rotation": ty("string"),
                "proxies": arr(obj(json!({
                    "id": ty("string"), "proxy": ty("string"), "current": ty("boolean"), "fails": ty("integer"), "restingSecs": ty("integer"),
                    "checked": obj(json!({"ok": ty("boolean"), "ms": nullable("integer"), "ip": nullable("string"), "country": nullable("string"), "status": nullable("integer"), "error": nullable("string")}), &["ok"]),
                }), &["id", "proxy", "current", "fails", "restingSecs"])),
                "total": ty("integer"), "truncated": ty("boolean"), "available": ty("integer"), "reason": ty("string"),
                "added": ty("integer"), "duplicates": ty("integer"),
                "rejected": arr(obj(json!({"input": ty("string"), "reason": ty("string")}), &["input", "reason"])),
                "removed": ty("integer"), "movedTo": nullable("string"),
                "checked": ty("integer"), "unchecked": ty("integer"), "next_step": ty("string"),
                "found": arr(obj(json!({"proxy": ty("string"), "app": ty("string"), "in_pool": ty("boolean")}), &["proxy", "app", "in_pool"])),
            }),
            &["total"],
        ),
        "dev_http" => obj(
            json!({
                "status": ty("integer"), "headers": ty("object"),
                "redirects": arr(obj(json!({"status": ty("integer"), "url": ty("string")}), &["status", "url"])),
                "finalUrl": ty("string"),
                "timing": obj(json!({"totalMs": ty("integer"), "firstByteMs": ty("integer")}), &["totalMs", "firstByteMs"]),
                "body": obj(json!({"preview": ty("string"), "truncated": ty("boolean"), "total": ty("integer"), "json": ty("boolean"), "binary": ty("boolean"), "parseError": ty("string")}), &["preview", "truncated", "total"]),
                "bodyBytes": ty("integer"), "bodyComplete": ty("boolean"), "bodyPath": nullable("string"),
                "request": obj(json!({"method": ty("string"), "url": ty("string"), "headers": ty("object")}), &["method", "url", "headers"]),
                "curl": ty("string"), "proxy": ty("string"), "untrusted": ty("boolean"),
            }),
            &[
                "status",
                "headers",
                "redirects",
                "timing",
                "body",
                "request",
                "curl",
                "untrusted",
            ],
        ),
        "dev_ports" => {
            let target = obj(
                json!({"pid": ty("integer"), "process": ty("string"), "command": nullable("string"), "ports": arr(ty("integer"))}),
                &["pid", "ports"],
            );
            obj(
                json!({
                    "listeners": arr(obj(json!({"port": ty("integer"), "pid": ty("integer"), "process": ty("string"), "command": nullable("string"), "user": nullable("string"), "addresses": arr(ty("string"))}), &["port", "pid", "process"])),
                    "total": ty("integer"), "truncated": ty("boolean"), "reason": ty("string"),
                    "dryRun": ty("boolean"), "targets": arr(target), "killed": arr(ty("integer")),
                    "failed": arr(obj(json!({"pid": ty("integer"), "reason": ty("string")}), &["pid", "reason"])),
                }),
                &[],
            )
        }
        "quick_tools" => {
            let awake = obj(
                json!({"mode": ty("string"), "active": ty("boolean"), "until_ms": nullable("integer"), "remaining_secs": nullable("integer")}),
                &["mode", "active"],
            );
            let snippet = obj(
                json!({"id": ty("integer"), "name": ty("string"), "chars": ty("integer"), "preview": ty("string"), "truncated": ty("boolean")}),
                &["id", "name"],
            );
            obj(
                json!({
                    "capabilities": obj(json!({"clipboard_history": ty("boolean"), "volume": ty("string")}), &["volume"]),
                    "awake": awake,
                    "snippets": {},
                    "snippet": obj(json!({"id": ty("integer"), "name": ty("string"), "text": ty("string")}), &["id", "name", "text"]),
                    "saved": snippet, "deleted": ty("integer"),
                    "total": ty("integer"), "truncated": ty("boolean"), "reason": ty("string"),
                    "cpu_percent": nullable("number"), "memory_used": ty("integer"), "memory_total": ty("integer"),
                    "disk_free": nullable("integer"), "disk_total": nullable("integer"),
                    "net_down_per_s": ty("integer"), "net_up_per_s": ty("integer"),
                    "top": arr(obj(json!({"pid": ty("integer"), "name": ty("string"), "cpu_percent": ty("number")}), &["pid", "name", "cpu_percent"])),
                    "kind": ty("string"),
                    "targets": arr(obj(json!({"id": ty("string"), "name": ty("string"), "pid": nullable("integer"), "volume": ty("integer"), "muted": ty("boolean")}), &["id", "name", "volume", "muted"])),
                    "id": ty("string"), "volume": nullable("integer"), "muted": nullable("boolean"), "done": ty("boolean"),
                }),
                &[],
            )
        }
        "transfer_servers" => {
            let server = obj(
                json!({"name": ty("string"), "protocol": ty("string"), "host": ty("string"), "port": ty("integer"), "user": ty("string"), "startPath": ty("string"), "keyPath": nullable("string"), "hasSecret": ty("boolean")}),
                &["name", "protocol", "host", "port", "hasSecret"],
            );
            obj(
                json!({
                    "servers": arr(server.clone()), "total": ty("integer"), "reason": ty("string"),
                    "saved": server, "removed": ty("string"), "forgotten": ty("integer"),
                }),
                &[],
            )
        }
        "transfer_list" => obj(
            json!({
                "path": ty("string"),
                "entries": arr(obj(json!({"name": ty("string"), "kind": ty("string"), "size": nullable("integer"), "modified": nullable("string")}), &["name", "kind"])),
                "total": ty("integer"), "truncated": ty("boolean"), "skipped": ty("integer"),
                "untrusted": ty("boolean"), "reason": ty("string"),
            }),
            &["path", "entries", "total", "truncated", "untrusted"],
        ),
        "transfer_get" | "transfer_put" => obj(
            json!({
                "transferId": ty("string"), "direction": ty("string"), "status": ty("string"),
                "bytes": ty("integer"), "total": nullable("integer"), "remote": ty("string"), "local": ty("string"),
                "next_step": ty("string"), "untrusted": ty("boolean"),
            }),
            &["transferId", "status", "bytes", "remote", "local"],
        ),
        "transfer_manage" => obj(
            json!({
                "op": ty("string"), "path": ty("string"), "to": ty("string"), "done": ty("boolean"), "dryRun": ty("boolean"),
                "target": obj(json!({"path": ty("string"), "kind": ty("string"), "size": nullable("integer")}), &["path", "kind"]),
                "would": ty("string"),
            }),
            &["op", "path", "done", "dryRun"],
        ),
        "dev_util" => obj(
            json!({
                "text": nullable("string"), "truncated": ty("boolean"), "total": ty("integer"),
                "value": {}, "type": ty("string"), "hex": ty("string"), "note": ty("string"),
                "header": ty("object"), "payload": ty("object"), "expiresAt": nullable("string"), "expired": nullable("boolean"), "verified": ty("boolean"),
                "hash": ty("string"), "algorithm": ty("string"), "bytes": ty("integer"), "source": ty("string"),
                "uuid": ty("string"), "unix": ty("integer"), "unixMs": ty("integer"), "iso": ty("string"),
                "matches": arr(ty("object")), "reason": ty("string"),
                "added": ty("integer"), "removed": ty("integer"), "identical": ty("boolean"),
            }),
            &[],
        ),
        "read_library_search" => obj(
            json!({"books": arr(obj(json!({"id": ty("integer"), "title": ty("string"), "authors": arr(ty("string")), "format": ty("string"), "status": ty("string"), "progress": ty("number")}), &["id", "title"]))}),
            &["books"],
        ),
        "read_book_get" => obj(
            json!({"id": ty("integer"), "title": ty("string"), "annotations": arr(ty("object"))}),
            &["id", "title", "annotations"],
        ),
        "read_book_text" => obj(
            json!({"id": ty("integer"), "chapter": ty("integer"), "chapters": ty("integer"), "title": ty("string"), "text": ty("string"), "nextOffset": nullable("integer"), "totalChars": ty("integer")}),
            &["id", "chapter", "chapters", "text", "totalChars"],
        ),
        "read_updates" => obj(json!({"unread": arr(ty("object"))}), &["unread"]),
        "omniget_health" => obj(
            json!({
                "app": ty("string"),
                "queue": ty("string"),
                "remote": ty("string"),
                "reachable": ty("boolean"),
                "machine": obj(json!({"hostname": ty("string"), "os": ty("string"), "arch": ty("string")}), &[]),
                "message": ty("string"),
                "server": obj(json!({"name": ty("string"), "version": ty("string")}), &["version"]),
                "protocolVersion": ty("string"),
                "supportedProtocolVersions": arr(ty("string")),
                "adapter": obj(
                    json!({"name": ty("string"), "version": ty("string"), "protocolVersion": ty("string"), "reachable": ty("boolean")}),
                    &["version", "protocolVersion"],
                ),
            }),
            &["app"],
        ),
        "destinations_list" => obj(
            json!({"destinations": arr(obj(
                json!({"destinationId": ty("string"), "name": ty("string"), "transferAllowed": ty("boolean")}),
                &["destinationId"],
            ))}),
            &["destinations"],
        ),
        "media_inspect" => obj(
            json!({
                "title": ty("string"),
                "author": ty("string"),
                "duration": nullable("number"),
                "formats": arr(obj(
                    json!({"label": ty("string"), "width": ty("integer"), "height": ty("integer"), "url": ty("string"), "format": ty("string")}),
                    &["label", "format"],
                )),
                "audioLanguages": arr(obj(
                    json!({"code": ty("string"), "label": ty("string"), "original": ty("boolean")}),
                    &["code", "label", "original"],
                )),
            }),
            &["title", "formats"],
        ),
        "media_collection_list" => obj(
            json!({
                "title": nullable("string"),
                "items": arr(obj(json!({"title": ty("string"), "url": ty("string")}), &["url"])),
                "nextCursor": nullable("integer"),
                "total": nullable("integer"),
            }),
            &["items"],
        ),
        "downloads_preflight" => obj(
            json!({
                "items": arr(obj(
                    json!({"url": ty("string"), "valid": ty("boolean"), "problem": nullable("string"), "authentication": ty("string"), "estimatedBytes": nullable("integer")}),
                    &["url", "valid"],
                )),
                "networkProbes": ty("boolean"),
                "availableBytes": nullable("integer"),
            }),
            &["items"],
        ),
        "downloads_batch_enqueue" => obj(
            json!({
                "batchId": ty("string"),
                "children": arr(obj(
                    json!({"accepted": ty("boolean"), "result": intent_receipt(), "error": ty("string")}),
                    &["accepted"],
                )),
            }),
            &["batchId", "children"],
        ),
        "download_enqueue" | "download_retry" | "download_resume" => intent_receipt(),
        "download_cancel" | "download_pause" => obj(
            json!({
                "download_id": ty("integer"),
                "action": ty("string"),
                "item": or_null(item()),
                "removedLeftovers": ty("integer"),
            }),
            &["download_id", "action"],
        ),
        "downloads_queue" => obj(
            json!({"total": ty("integer"), "items": arr(item())}),
            &["total", "items"],
        ),
        "download_status" => obj(
            json!({"item": item(), "nextPollAfterMs": nullable("integer"), "nextActions": next_actions()}),
            &["item"],
        ),
        "download_wait" => obj(
            json!({"item": item(), "timedOut": ty("boolean"), "nextPollAfterMs": nullable("integer"), "nextActions": next_actions()}),
            &["item", "timedOut"],
        ),
        "download_logs" => obj(
            json!({
                "events": arr(obj(
                    json!({
                        "eventId": ty("integer"),
                        "timestamp": ty("integer"),
                        "downloadId": ty("integer"),
                        "attemptId": ty("string"),
                        "phase": ty("string"),
                        "level": ty("string"),
                        "message": ty("string"),
                        "externalContent": ty("boolean"),
                    }),
                    &["eventId", "message"],
                )),
                "nextCursor": ty("integer"),
                "hasMore": ty("boolean"),
                "dropped": ty("integer"),
                "oldestAvailable": nullable("integer"),
                "truncated": ty("integer"),
                "evidenceIncomplete": ty("boolean"),
                "accountingPending": ty("boolean"),
                "priorSessionGaps": ty("integer"),
                "accountingNote": ty("string"),
            }),
            &["events", "nextCursor", "hasMore"],
        ),
        "download_diagnose" | "download_recovery_options" => obj(
            json!({
                "downloadId": ty("integer"),
                "status": ty("string"),
                "diagnosis": or_null(diagnosis()),
                "retry": or_null(obj(
                    json!({"allowed": ty("boolean"), "retryable": ty("boolean"), "attempt": ty("integer"), "retriesUsed": ty("integer"), "maxRetries": ty("integer"), "notBeforeMs": nullable("integer")}),
                    &["allowed", "retryable"],
                )),
                "attempts": {},
                "evidence": arr(obj(
                    json!({"eventId": ty("integer"), "attemptId": ty("string"), "level": ty("string"), "excerpt": ty("string")}),
                    &["eventId", "excerpt"],
                )),
                "unknowns": arr(ty("string")),
                "incomplete": ty("boolean"),
                "nextCursor": ty("integer"),
                "nextActions": next_actions(),
                "externalContent": ty("boolean"),
            }),
            &["downloadId", "diagnosis", "nextActions"],
        ),
        "download_artifacts" => obj(
            json!({
                "artifacts": arr(obj(
                    json!({
                        "artifactId": ty("string"),
                        "name": ty("string"),
                        "primary": ty("boolean"),
                        "bytes": nullable("integer"),
                        "validation": ty("string"),
                        "media": {},
                        "validationError": {},
                        "suggestedName": ty("string"),
                        "transferAllowed": ty("boolean"),
                        "access": ty("object"),
                        "transferError": ty("string"),
                    }),
                    &["artifactId", "name", "transferAllowed"],
                )),
                "truncated": ty("boolean"),
                "maxArtifacts": ty("integer"),
            }),
            &["artifacts", "truncated"],
        ),
        "download_history" => obj(
            json!({"items": arr(obj(
                json!({
                    "id": ty("integer"),
                    "url": ty("string"),
                    "platform": ty("string"),
                    "title": nullable("string"),
                    "file_path": nullable("string"),
                    "file_size_bytes": nullable("integer"),
                    "total_bytes": nullable("integer"),
                    "success": ty("boolean"),
                    "error": nullable("string"),
                    "completed_at": ty("integer"),
                    "thumbnail_url": nullable("string"),
                    "kind": nullable("string"),
                    "status": ty("object"),
                    "source": ty("string"),
                }),
                &["id", "url"],
            ))}),
            &["items"],
        ),
        "agents_list" => obj(
            json!({
                "items": arr(obj(
                    json!({"grantId": ty("string"), "executorIds": arr(ty("string")), "maxTokens": ty("integer"), "executionEnabled": ty("boolean")}),
                    &["grantId", "executorIds"],
                )),
                "executionEnabled": ty("boolean"),
            }),
            &["items"],
        ),
        "workspaces_list" => obj(
            json!({
                "items": arr(obj(json!({"grantId": ty("string"), "workspaceId": ty("string")}), &["grantId", "workspaceId"])),
                "executionEnabled": ty("boolean"),
            }),
            &["items"],
        ),
        "missions_list" => obj(
            json!({"items": arr(mission_summary()), "nextCursor": ty("string"), "hasMore": ty("boolean")}),
            &["items", "hasMore"],
        ),
        "missions_get" => {
            let mut s = mission_summary();
            s["properties"]["tasks"] = arr(obj(
                json!({"taskId": ty("string"), "state": ty("string"), "attempts": ty("integer"), "updatedMs": ty("integer"), "jobs": {}}),
                &["taskId", "state"],
            ));
            s["properties"]["tasksTruncated"] = ty("boolean");
            s["properties"]["criteria"] = arr(ty("object"));
            s["properties"]["verdict"] = json!({});
            s["properties"]["block"] = json!({});
            s["required"] = json!(["missionId", "state", "tasks"]);
            s
        }
        "missions_events" => obj(
            json!({
                "events": arr(obj(
                    json!({"eventId": ty("string"), "seq": ty("integer"), "kind": ty("string"), "revision": ty("integer"), "timestampMs": ty("integer"), "payloadOmitted": ty("boolean")}),
                    &["eventId", "seq"],
                )),
                "nextCursor": ty("integer"),
                "hasMore": ty("boolean"),
            }),
            &["events", "hasMore"],
        ),
        "missions_create" | "missions_cancel" | "missions_pause" | "missions_resume" => {
            mission_summary()
        }
        "missions_artifacts" => obj(
            json!({
                "missionId": ty("string"),
                "state": {},
                "items": arr(obj(
                    json!({"path": ty("string"), "criterionId": nullable("string"), "receipt": or_null(ty("object")), "current": ty("object"), "transfer": ty("object")}),
                    &["path"],
                )),
                "note": ty("string"),
            }),
            &["missionId", "items"],
        ),
        "missions_diagnostics" => obj(
            json!({
                "missionId": ty("string"),
                "state": {},
                "revision": ty("integer"),
                "block": {},
                "verdict": {},
                "completion": {},
                "criteria": arr(ty("object")),
                "tasks": arr(ty("object")),
                "effects": arr(ty("object")),
                "spent": ty("object"),
                "evidenceGaps": ty("array"),
            }),
            &["missionId", "tasks"],
        ),
        "company_query" | "company_command" => obj(
            json!({"method": ty("string"), "result": {}}),
            &["method", "result"],
        ),
        "job_diff" => obj(
            json!({
                "jobId": ty("string"),
                "state": ty("string"),
                "available": ty("boolean"),
                "reason": nullable("string"),
                "detail": nullable("string"),
                "files": arr(obj(
                    json!({"path": ty("string"), "status": ty("string"), "additions": nullable("integer"), "deletions": nullable("integer"), "binary": ty("boolean")}),
                    &["path", "status", "binary"],
                )),
                "fileCount": ty("integer"),
                "filesTruncated": ty("boolean"),
                "additions": ty("integer"),
                "deletions": ty("integer"),
                "clipped": ty("boolean"),
                "complete": ty("boolean"),
                "headOid": nullable("string"),
                "patch": ty("string"),
                "patchClipped": ty("boolean"),
            }),
            &[
                "jobId",
                "available",
                "files",
                "fileCount",
                "additions",
                "deletions",
            ],
        ),
        "approvals_list" => obj(
            json!({
                "missionId": ty("string"),
                "pending": arr(obj(
                    json!({"requestId": ty("string"), "runId": ty("string"), "action": ty("string"), "scope": ty("string"), "preview": ty("string"), "deadlineMs": {}}),
                    &["requestId"],
                )),
                "remoteApproval": ty("boolean"),
                "approveIn": ty("string"),
            }),
            &["missionId", "pending"],
        ),
        "approval_respond" => obj(
            json!({"approvalId": ty("string"), "state": ty("string"), "decision": nullable("string"), "jobId": nullable("string")}),
            &["approvalId", "state"],
        ),
        "agent_run" => obj(
            json!({"commandId": ty("string"), "jobId": nullable("string"), "state": ty("string"), "stateSeq": ty("integer"), "replayed": ty("boolean"), "next": ty("string")}),
            &["commandId", "jobId", "state", "stateSeq", "replayed"],
        ),
        "job_get" | "job_wait" => obj(
            json!({
                "id": ty("string"), "state": ty("string"), "state_seq": ty("integer"), "kind": ty("string"),
                "agent_id": ty("string"), "title": nullable("string"), "run_mode": nullable("string"),
                "created_ms": ty("integer"), "started_ms": nullable("integer"), "finished_ms": nullable("integer"),
                "wait_reason": nullable("string"), "failure_reason": nullable("string"), "jobError": nullable("string"),
                "outcome": or_null(obj(json!({"status": ty("string"), "summary": ty("string")}), &["status"])),
                "branch": nullable("string"), "head_oid": nullable("string"), "files_changed": nullable("integer"),
                "additions": nullable("integer"), "deletions": nullable("integer"), "result": nullable("string"),
                "changed": ty("boolean"),
                "pendingApprovals": arr(obj(json!({"approvalId": ty("string"), "tool": nullable("string"), "preview": nullable("string")}), &["approvalId"])),
            }),
            &["id", "state", "state_seq", "pendingApprovals"],
        ),
        "job_report" => obj(
            json!({
                "jobId": ty("string"), "state": ty("string"), "stateSeq": ty("integer"), "verdict": ty("string"),
                "title": nullable("string"), "agentId": ty("string"), "runMode": ty("string"), "isolation": ty("string"),
                "outcome": or_null(obj(json!({"status": ty("string"), "summary": ty("string")}), &["status"])),
                "result": nullable("string"), "jobError": nullable("string"), "failureReason": nullable("string"),
                "diff": obj(json!({"available": ty("boolean"), "filesChanged": nullable("integer"), "files": arr(obj(json!({"path": ty("string"), "status": ty("string")}), &["path", "status"]))}), &["available", "files"]),
                "proof": obj(json!({"checks": {}, "tokens": {}}), &[]),
                "workflow": {},
                "changed": ty("boolean"),
                "next": ty("string"),
            }),
            &[
                "jobId", "state", "stateSeq", "verdict", "diff", "proof", "next",
            ],
        ),
        "job_cancel" => obj(
            json!({"jobId": nullable("string"), "state": ty("string"), "replayed": ty("boolean")}),
            &["jobId", "state", "replayed"],
        ),
        "jobs_state" => obj(
            json!({
                "jobs": arr(obj(json!({"jobId": ty("string"), "state": ty("string"), "stateSeq": ty("integer"), "title": nullable("string"), "createdMs": ty("integer")}), &["jobId", "state", "stateSeq"])),
                "active": ty("integer"),
            }),
            &["jobs", "active"],
        ),
        "agents_prepare" => derived_bot(),
        "agents_derived_list" => obj(json!({"items": arr(derived_bot())}), &["items"]),
        "groups_prepare" => room(),
        "groups_get" => obj(
            json!({
                "room": room(),
                "tasks": arr(obj(
                    json!({"taskId": ty("string"), "recipient": ty("string"), "state": ty("string"), "limitMs": ty("integer"), "createdMs": ty("integer"), "finishedMs": nullable("integer"), "result": nullable("string"), "error": nullable("string")}),
                    &["taskId", "state"],
                )),
            }),
            &["room", "tasks"],
        ),
        "group_tasks_create" => obj(
            json!({"taskId": ty("string"), "roomId": ty("string"), "recipient": ty("string"), "state": ty("string"), "limitMs": ty("integer"), "execution": ty("string")}),
            &["taskId", "state"],
        ),
        _ => return None,
    })
}
