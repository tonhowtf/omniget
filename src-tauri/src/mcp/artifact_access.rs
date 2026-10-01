use super::{
    policy::{self, Principal},
    ToolDef,
};
use serde_json::{json, Value};

pub fn tools() -> Vec<ToolDef> {
    vec![ToolDef {
        name: "artifact_access",
        description: "Metadata or protected access for ONE artifact. artifactId is either the `<downloadId>:<index>` id from download_artifacts or the grant id returned in its `access`. mode=metadata (default) returns name, size, validation and expiry without creating access. mode=transfer (needs the transfer scope and a folder granted in OmniGet) returns a short-lived, revocable grant: GET the returned downloadPath on the local server with your bearer token and `If-Match: \"<digest>\"`; Range is supported. The grant expires after 15 minutes, is bound to this client and this file revision, and stops working when the folder grant or the client is revoked. Files never leave the computer otherwise.",
        input_schema: json!({"type":"object","properties":{"artifactId":{"type":"string","minLength":1,"maxLength":64},"mode":{"type":"string","enum":["metadata","transfer"]}},"required":["artifactId"]}),
    }]
}

pub fn output_schema(name: &str) -> Option<Value> {
    (name == "artifact_access").then(|| {
        json!({"type":"object","properties":{
        "artifactId":{"type":"string"},
        "mode":{"type":"string","enum":["metadata","transfer"]},
        "name":{"type":"string"},
        "bytes":{"type":["integer","null"]},
        "validation":{"type":["string","null"]},
        "media":{},
        "transferAllowed":{"type":"boolean"},
        "access":{"anyOf":[{"type":"null"},{"type":"object","properties":{
            "artifactId":{"type":"string"},
            "digest":{"type":"string"},
            "bytes":{"type":"integer"},
            "name":{"type":"string"},
            "expiresAt":{"type":"integer"},
            "transferAllowed":{"type":"boolean"},
            "downloadPath":{"type":"string"}
        },"required":["artifactId","digest","expiresAt","downloadPath"]}]},
        "ifMatch":{"type":["string","null"]}
    },"required":["artifactId","mode","transferAllowed"]})
    })
}

#[derive(Debug, PartialEq, Eq)]
pub enum Ref {
    Job(u64, usize),
    Grant(String),
}

pub fn parse(raw: &str) -> Result<Ref, String> {
    if let Some((job, index)) = raw.split_once(':') {
        let job: u64 = job.parse().map_err(|_| "INVALID_ARGUMENT: artifactId")?;
        let index: usize = index.parse().map_err(|_| "INVALID_ARGUMENT: artifactId")?;
        if job == 0 || index > 10_000 {
            return Err("INVALID_ARGUMENT: artifactId".into());
        }
        return Ok(Ref::Job(job, index));
    }
    uuid::Uuid::parse_str(raw)
        .map(|u| Ref::Grant(u.hyphenated().to_string()))
        .map_err(|_| "INVALID_ARGUMENT: artifactId".into())
}

fn wants_transfer(a: &Value) -> bool {
    a["mode"].as_str() == Some("transfer")
}

pub fn admit(p: &Principal, r: &Ref, transfer: bool) -> Result<(), String> {
    policy::active(p)?;
    if !policy::allowed(p, "artifact_access") {
        return Err("TOOL_NOT_AUTHORIZED".into());
    }
    let has_transfer = p.scopes.iter().any(|s| s == "transfer");
    if (transfer || matches!(r, Ref::Grant(_))) && !has_transfer {
        return Err("TRANSFER_NOT_GRANTED".into());
    }
    if let Ref::Job(id, _) = r {
        policy::own(p, *id)?;
    }
    Ok(())
}

fn grant_view(p: &Principal, id: &str, transfer: bool) -> Result<Value, String> {
    let g = super::artifacts::describe(p, id)?;
    let access = serde_json::to_value(&g).map_err(|_| "ARTIFACT_SERIALIZATION")?;
    Ok(json!({
        "artifactId": g.artifact_id,
        "mode": if transfer { "transfer" } else { "metadata" },
        "name": g.name,
        "bytes": g.bytes,
        "validation": null,
        "transferAllowed": true,
        "access": if transfer { access } else { Value::Null },
        "ifMatch": if transfer { json!(format!("\"{}\"", g.digest)) } else { Value::Null },
        "expiresAt": g.expires_at,
    }))
}

pub async fn call(app: &tauri::AppHandle, p: &Principal, a: &Value) -> Result<Value, String> {
    let raw = a["artifactId"]
        .as_str()
        .ok_or("MISSING_ARGUMENT: artifactId")?;
    let r = parse(raw)?;
    let transfer = wants_transfer(a);
    admit(p, &r, transfer)?;
    match r {
        Ref::Grant(id) => grant_view(p, &id, transfer),
        Ref::Job(id, _) => {
            let listed = Box::pin(super::downloads::call(
                app,
                p,
                "download_artifacts",
                json!({"download_id": id}),
            ))
            .await?;
            let entry = listed["artifacts"]
                .as_array()
                .and_then(|list| list.iter().find(|e| e["artifactId"] == raw))
                .cloned()
                .ok_or("ARTIFACT_NOT_FOUND")?;
            let mut out = json!({
                "artifactId": raw,
                "mode": if transfer { "transfer" } else { "metadata" },
                "name": entry["name"],
                "bytes": entry["bytes"],
                "validation": entry["validation"],
                "media": entry["media"],
                "transferAllowed": entry["transferAllowed"].as_bool().unwrap_or(false),
                "access": null,
                "ifMatch": null,
            });
            if transfer {
                let access = entry
                    .get("access")
                    .filter(|v| v.is_object())
                    .ok_or_else(|| {
                        entry["transferError"]
                            .as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| "ROOT_NOT_GRANTED".into())
                    })?;
                out["ifMatch"] = json!(format!("\"{}\"", access["digest"].as_str().unwrap_or("")));
                out["access"] = access.clone();
            }
            Ok(out)
        }
    }
}
