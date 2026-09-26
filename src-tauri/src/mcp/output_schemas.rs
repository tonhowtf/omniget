//! `outputSchema` of every MCP tool (revision 2025-06-18), applied where
//! `tools/list` is built. Each schema describes the tool's `structuredContent`
//! and also admits the business-error envelope `{"error":{code,message}}`
//! that `tools/call` returns with `isError: true` (clients such as the
//! official SDK validate `structuredContent` even on errors).
//!
//! Objects stay open (no `additionalProperties: false`): a new field is not a
//! breaking change, while declared fields and `required` keys are checked by
//! tests against real `structuredContent`.
//!
//! Adding a schema for a new tool: return it from the tool module's
//! `output_schema(name) -> Option<Value>` (see `auth`, `bundle`,
//! `artifact_access`) and chain that function in [`for_tool`], or add an arm
//! to `own`. A tool without an entry is listed without `outputSchema`, which
//! MCP allows; `every_catalog_tool_declares_an_output_schema` names it.
use serde_json::{json, Value};

/// Adds `outputSchema` to each listed tool that has one; others unchanged.
pub fn apply(mut tools: Vec<Value>) -> Vec<Value> {
    for tool in &mut tools {
        let schema = tool["name"].as_str().and_then(for_tool);
        if let (Some(schema), Some(obj)) = (schema, tool.as_object_mut()) {
            obj.insert("outputSchema".into(), schema);
        }
    }
    tools
}

/// Output schema of one tool, with the error envelope admitted.
pub fn for_tool(name: &str) -> Option<Value> {
    own(name)
        .or_else(|| super::auth::output_schema(name))
        .or_else(|| super::bundle::output_schema(name))
        .or_else(|| super::artifact_access::output_schema(name))
        .map(with_error_branch)
}

fn error_envelope() -> Value {
    json!({"type":"object","properties":{"code":{"type":"string"},"message":{"type":"string"}},"required":["code","message"]})
}

/// `required` keys hold for a success; an error result carries only `error`.
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

/// A queue entry (`QueueItemInfo`) or its journal view when the in-memory
/// queue lost the job (`source: "journal"`, `status.type: "Unknown"`).
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
/// `download_enqueue`/`retry`/`resume` receipt (first answer and replays).
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
        "omniget_health" => obj(
            json!({
                "app": ty("string"),
                "queue": ty("string"),
                "remote": ty("string"),
                "reachable": ty("boolean"),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::policy::{self, Principal};

    /// Draft 2020-12 subset used by these schemas (and the ones tool modules
    /// contribute). An unsupported keyword fails loudly so a schema never
    /// passes here while a client's validator would read it differently.
    fn validate(schema: &Value, v: &Value, path: &str) -> Result<(), String> {
        let Some(s) = schema.as_object() else {
            return Err(format!("{path}: schema is not an object"));
        };
        for key in s.keys() {
            if !matches!(
                key.as_str(),
                "type"
                    | "properties"
                    | "required"
                    | "items"
                    | "enum"
                    | "const"
                    | "anyOf"
                    | "additionalProperties"
                    | "description"
                    | "minimum"
                    | "maximum"
                    | "minLength"
                    | "maxLength"
                    | "title"
                    | "default"
            ) {
                return Err(format!("{path}: unsupported keyword {key}"));
            }
        }
        if let Some(t) = s.get("type") {
            let types: Vec<&str> = match t {
                Value::String(t) => vec![t.as_str()],
                Value::Array(ts) => ts.iter().filter_map(Value::as_str).collect(),
                _ => return Err(format!("{path}: bad type")),
            };
            let ok = types.iter().any(|t| match *t {
                "object" => v.is_object(),
                "array" => v.is_array(),
                "string" => v.is_string(),
                "boolean" => v.is_boolean(),
                "null" => v.is_null(),
                "number" => v.is_number(),
                "integer" => {
                    v.is_i64() || v.is_u64() || v.as_f64().is_some_and(|f| f.fract() == 0.0)
                }
                _ => false,
            });
            if !ok {
                return Err(format!("{path}: expected {t}, got {v}"));
            }
        }
        if let Some(e) = s.get("enum").and_then(Value::as_array) {
            if !e.contains(v) {
                return Err(format!("{path}: {v} not in enum"));
            }
        }
        if let Some(c) = s.get("const") {
            if c != v {
                return Err(format!("{path}: {v} != const {c}"));
            }
        }
        if let Some(branches) = s.get("anyOf").and_then(Value::as_array) {
            let errors: Vec<String> = branches
                .iter()
                .filter_map(|b| validate(b, v, path).err())
                .collect();
            if errors.len() == branches.len() {
                return Err(format!("{path}: no anyOf branch matched: {errors:?}"));
            }
        }
        if let Some(o) = v.as_object() {
            for key in s
                .get("required")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if !o.contains_key(key.as_str().unwrap_or_default()) {
                    return Err(format!("{path}: missing required {key}"));
                }
            }
            let props = s.get("properties").and_then(Value::as_object);
            for (k, child) in o {
                match props.and_then(|p| p.get(k)) {
                    Some(sub) => validate(sub, child, &format!("{path}.{k}"))?,
                    None => match s.get("additionalProperties") {
                        Some(Value::Bool(false)) => return Err(format!("{path}: unexpected {k}")),
                        Some(sub @ Value::Object(_)) => {
                            validate(sub, child, &format!("{path}.{k}"))?
                        }
                        _ => {}
                    },
                }
            }
        }
        if let (Some(items), Some(a)) = (s.get("items"), v.as_array()) {
            for (i, child) in a.iter().enumerate() {
                validate(items, child, &format!("{path}[{i}]"))?;
            }
        }
        Ok(())
    }

    fn check(tool: &str, sc: &Value) {
        let schema = for_tool(tool).unwrap_or_else(|| panic!("{tool} has no outputSchema"));
        if let Err(e) = validate(&schema, sc, tool) {
            panic!("{tool}: real structuredContent does not validate: {e}\n{sc}");
        }
    }

    fn everyone() -> Principal {
        Principal {
            id: "t".into(),
            name: "t".into(),
            scopes: policy::SCOPES.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn every_catalog_tool_declares_an_output_schema() {
        let listed = apply(super::super::downloads::catalog(&everyone()));
        assert!(!listed.is_empty());
        for tool in &listed {
            let name = tool["name"].as_str().unwrap();
            let schema = tool
                .get("outputSchema")
                .unwrap_or_else(|| panic!("{name} has no outputSchema"));
            // MCP: an output schema is a JSON Schema object whose root is an object.
            assert_eq!(schema["type"], "object", "{name}");
            // The error envelope of `isError: true` results always validates.
            validate(schema, &json!({"error":{"code":"X","message":"X"}}), name).unwrap();
        }
    }

    #[test]
    fn tools_without_an_entry_are_listed_unchanged() {
        let listed = apply(vec![
            json!({"name":"future_tool","inputSchema":{"type":"object"}}),
            json!({"inputSchema":{}}),
        ]);
        assert!(listed.iter().all(|t| t.get("outputSchema").is_none()));
    }

    /// Real `structuredContent` captured from the running desktop by the
    /// final gate verification (MCP over HTTP, 2026-09-25); paths anonymized.
    #[test]
    fn real_structured_content_validates() {
        let samples: serde_json::Map<String, Value> =
            serde_json::from_str(include_str!("output_schema_samples.json")).unwrap();
        let mut tools = std::collections::BTreeSet::new();
        for (key, values) in &samples {
            let tool = key.split('|').next().unwrap();
            tools.insert(tool.to_string());
            for sc in values.as_array().unwrap() {
                check(tool, sc);
            }
        }
        assert!(tools.len() >= 20, "{tools:?}");
    }

    #[test]
    fn schemas_are_not_vacuous() {
        let bad = [
            ("download_status", json!({"item": 5})),
            ("download_status", json!({"nextPollAfterMs": 1000})),
            (
                "download_wait",
                json!({"item": {"id": 1, "url": "u", "status": {"type": "Active"}}}),
            ),
            ("download_enqueue", json!({"download_id": 1})),
            (
                "download_logs",
                json!({"events": [{"eventId": "x", "message": "m"}], "nextCursor": 0, "hasMore": false}),
            ),
            (
                "download_diagnose",
                json!({"downloadId": 1, "diagnosis": {"code": 1, "retryable": "never"}, "nextActions": []}),
            ),
            (
                "download_artifacts",
                json!({"artifacts": [{"artifactId": "1:0"}], "truncated": false}),
            ),
            ("omniget_health", json!({"queue": "available"})),
            ("download_status", json!({"error": {"code": "X"}})),
        ];
        for (tool, sc) in bad {
            assert!(
                validate(&for_tool(tool).unwrap(), &sc, tool).is_err(),
                "{tool} accepted {sc}"
            );
        }
    }

    /// Shapes of branches the gate run did not reach, built from the same
    /// types and literals the handlers use.
    #[test]
    fn code_shaped_content_validates() {
        // `downloads::journal_item` literal (it reads the durable receipt, so
        // it is mirrored here instead of called).
        let journal = json!({"id":1,"url":"https://example.com/v?token=[REDACTED]","title":null,"platform":"mcp_worker","status":{"type":"Unknown","data":{"stage":"executing","interrupted":true,"effectMayHaveRun":true}},"attempt":0,"file_path":null,"file_size_bytes":null,"source":"journal"});
        check(
            "download_status",
            &json!({"item": journal, "nextPollAfterMs": null, "nextActions": [{"action":"reconcile","tool":"download_retry"}]}),
        );
        // HTTP health (D-14) and a live item whose total is unknown (D-04).
        check(
            "omniget_health",
            &super::super::downloads::health("2025-06-18"),
        );
        check(
            "download_status",
            &json!({"item": {"id": 2, "url": "https://example.com/v", "status": {"type": "Active"}, "percent": null, "downloaded_bytes": 2400000, "total_bytes": null}, "nextPollAfterMs": 1000}),
        );
        check(
            "download_wait",
            &json!({"item": journal, "timedOut": false, "nextPollAfterMs": null, "nextActions": []}),
        );
        check("downloads_queue", &json!({"total": 1, "items": [journal]}));
        check("download_history", &json!({"items": [journal]}));
        check(
            "download_retry",
            &json!({"download_id": 1, "attempt": 1, "outcome": "unknown", "recovery": "local_reconciliation_required", "item": journal, "nextActions": []}),
        );
        check(
            "download_retry",
            &json!({"download_id": 1, "attempt": 1, "outcome": "interrupted", "removedLeftovers": 0, "item": journal, "receipt": null, "nextActions": []}),
        );
        check(
            "download_resume",
            &json!({"download_id": 1, "attempt": 2, "outcome": "queued", "item": journal, "resume": {"continuation": "restart", "discardedFiles": 1, "discardedBytes": 10, "cooldownApplied": false}}),
        );
        let diagnosis = serde_json::to_value(crate::core::root_cause::machine_diagnose(
            "HTTP 503 Service Unavailable",
        ))
        .unwrap();
        check(
            "download_diagnose",
            &json!({"downloadId": 1, "diagnosis": diagnosis, "retry": {"allowed": true, "retryable": true, "attempt": 0, "retriesUsed": 0, "maxRetries": 2, "notBeforeMs": 5}, "attempts": [], "evidence": [], "unknowns": [], "incomplete": true, "nextCursor": 0, "nextActions": [], "externalContent": true}),
        );
        let page = crate::core::download_journal::Page {
            events: vec![crate::core::download_journal::Event {
                event_id: 1,
                timestamp: 2,
                download_id: 3,
                attempt_id: "a".into(),
                phase: "engine".into(),
                level: "error".into(),
                message: "m".into(),
                external_content: true,
            }],
            next_cursor: 1,
            has_more: false,
            dropped: 0,
            oldest_available: None,
            truncated: 0,
            evidence_incomplete: false,
            accounting_pending: false,
            prior_session_gaps: 0,
            accounting_note: "n",
        };
        check("download_logs", &serde_json::to_value(page).unwrap());
        let quality = omniget_core::models::media::VideoQuality {
            label: "original".into(),
            width: 0,
            height: 0,
            url: String::new(),
            format: "ytdlp".into(),
        };
        check(
            "media_inspect",
            &json!({"title": "t", "author": "a", "duration": null, "formats": [quality]}),
        );
        check(
            "download_cancel",
            &json!({"download_id": 1, "action": "cancel", "item": null, "removedLeftovers": 2}),
        );
        // Orchestration projections (orchestration.rs literals).
        let summary = json!({"missionId":"m","state":"running","objective":"o","criteriaVersion":1,"revision":2,"createdMs":3,"updatedMs":4,"eventsDropped":0,"verification":"historical_state_only"});
        check(
            "agents_list",
            &json!({"items":[{"grantId":"g","executorIds":["b"],"maxTokens":10,"executionEnabled":true}],"executionEnabled":true}),
        );
        check(
            "workspaces_list",
            &json!({"items":[{"grantId":"g","workspaceId":"w"}],"executionEnabled":false}),
        );
        check(
            "missions_list",
            &json!({"items":[summary],"nextCursor":"m","hasMore":false}),
        );
        for tool in [
            "missions_create",
            "missions_cancel",
            "missions_pause",
            "missions_resume",
        ] {
            check(tool, &summary);
        }
        let mut got = summary.clone();
        got["tasks"] =
            json!([{"taskId":"t","state":"done","attempts":1,"updatedMs":5,"jobs":["j"]}]);
        got["tasksTruncated"] = json!(false);
        got["criteria"] = json!([{"id":"c","title":"t","status":"passed","severity":"required","receiptId":null}]);
        got["verdict"] = json!({"passed":true});
        got["block"] = Value::Null;
        check("missions_get", &got);
        check(
            "missions_events",
            &json!({"events":[{"eventId":"e","seq":1,"kind":"k","revision":1,"timestampMs":2,"payloadOmitted":true}],"nextCursor":1,"hasMore":false}),
        );
        check(
            "missions_artifacts",
            &json!({"missionId":"m","state":"completed","items":[{"path":"a.txt","criterionId":null,"receipt":null,"current":{"available":false,"reason":"X"}}],"note":"n"}),
        );
        check(
            "missions_diagnostics",
            &json!({"missionId":"m","state":"blocked","revision":1,"block":null,"verdict":{},"completion":0.5,"criteria":[],"tasks":[],"effects":[],"spent":{"tokens":1},"evidenceGaps":[]}),
        );
        check(
            "approvals_list",
            &json!({"missionId":"m","pending":[{"requestId":"r","runId":"u","action":"a","scope":"s","preview":"p","deadlineMs":null}],"remoteApproval":false,"approveIn":"OmniGet window"}),
        );
    }

    /// C02 tools called for real against an in-memory store (same setup as
    /// `orchestration_config::tests`).
    #[test]
    fn orchestration_config_structured_content_validates() {
        use omniget_core::core::assist::{authority, db::AssistDb, external_config::BotRoster};
        use omniget_core::core::llm::roster_store::RosterStore;
        let call = super::super::orchestration_config::call_with;
        let dir = std::env::temp_dir().join(format!(
            "omniget-outschema-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let dir = std::fs::canonicalize(dir).unwrap();
        let db = AssistDb::open_in_memory().unwrap();
        let roster = RosterStore::at(dir.join("roster.json"));
        let mut exec = roster.get("builder").unwrap();
        exec.id = "exec-a".into();
        roster.create(exec.clone()).unwrap();
        let grant = authority::grant(
            &db,
            authority::Ceiling {
                id: String::new(),
                principal: "A".into(),
                workspace_id: "ws".into(),
                workspace: dir.to_string_lossy().into_owned(),
                workspace_identity: None,
                bots: vec!["exec-a".into()],
                bot_revisions: std::collections::BTreeMap::from([(
                    "exec-a".into(),
                    authority::agent_revision(&exec).unwrap(),
                )]),
                tools: vec!["fs_read".into(), "fs_write".into()],
                max_tokens: 10_000,
                max_usd: None,
            },
        )
        .unwrap();
        let a = Principal {
            id: "A".into(),
            name: "A".into(),
            scopes: vec![],
        };
        let r: Option<&dyn BotRoster> = Some(&roster);
        let bot = |key: &str| json!({"grantId":grant,"sourceExecutorId":"exec-a","name":key,"instructions":"work","tools":["fs_read"],"idempotencyKey":key});
        let lead = call(&db, r, &a, "agents_prepare", bot("lead")).unwrap();
        let worker = call(&db, r, &a, "agents_prepare", bot("worker")).unwrap();
        check("agents_prepare", &lead);
        check(
            "agents_derived_list",
            &call(&db, r, &a, "agents_derived_list", json!({})).unwrap(),
        );
        let room = call(&db, r, &a, "groups_prepare", json!({"grantId":grant,"title":"t","memberBotIds":[lead["botId"],worker["botId"]],"coordinatorId":lead["botId"],"idempotencyKey":"room"})).unwrap();
        check("groups_prepare", &room);
        let task = call(&db, r, &a, "group_tasks_create", json!({"roomId":room["roomId"],"to":worker["botId"],"question":"q","idempotencyKey":"t1"})).unwrap();
        check("group_tasks_create", &task);
        check(
            "groups_get",
            &call(&db, r, &a, "groups_get", json!({"roomId":room["roomId"]})).unwrap(),
        );
        authority::revoke_principal(&db, "A").unwrap();
        check(
            "agents_derived_list",
            &call(&db, r, &a, "agents_derived_list", json!({})).unwrap(),
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
