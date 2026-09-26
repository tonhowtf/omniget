//! `artifact_access`: the existing artifact access (per-recipient grants,
//! If-Match digest, 15-minute expiry, root revocation) exposed as its own
//! tool. It never widens access: it resolves only artifacts the caller could
//! already reach through `download_artifacts`, and transfer mode requires the
//! `transfer` scope plus a locally granted root, exactly like that tool.
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

/// Output schema for the protocol fixer's `outputSchema` table.
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
    /// `<downloadId>:<index>` from `download_artifacts`.
    Job(u64, usize),
    /// A transfer grant id (UUID) issued to this client.
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

/// Scope rule: `artifacts` always; `transfer` for transfer mode or a grant id
/// (grants exist only for transfer recipients).
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

/// Called by `downloads::call` after schema validation.
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
            // Same listing, validation and grant rules as download_artifacts;
            // metadata mode strips any access it produced.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_are_strict() {
        assert_eq!(parse("12:0").unwrap(), Ref::Job(12, 0));
        let id = uuid::Uuid::new_v4().to_string();
        assert_eq!(parse(&id).unwrap(), Ref::Grant(id.clone()));
        for bad in [
            "",
            "0:1",
            "a:1",
            "1:-1",
            "../etc/passwd",
            "1:2:3",
            "/mcp/artifacts/x",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn scopes_owner_and_grant_recipient_are_enforced() {
        let dir = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("omniget-artifact-access-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        policy::TEST_DIR.with(|p| *p.borrow_mut() = Some(dir.join("policy")));
        let root = dir.join("files");
        std::fs::create_dir(&root).unwrap();
        let file = root.join("clip.mp4");
        std::fs::write(&file, b"bytes").unwrap();
        let a = policy::create("A".into(), vec!["artifacts".into(), "transfer".into()])
            .unwrap()
            .principal;
        let b = policy::create("B".into(), vec!["artifacts".into(), "transfer".into()])
            .unwrap()
            .principal;
        let meta_only = policy::create("M".into(), vec!["artifacts".into()])
            .unwrap()
            .principal;
        let none = policy::create("N".into(), vec!["discover".into()])
            .unwrap()
            .principal;
        policy::claim(&a, 5).unwrap();
        // Job references: scope, then ownership; transfer needs the transfer scope.
        assert!(admit(&a, &Ref::Job(5, 0), false).is_ok());
        assert!(admit(&a, &Ref::Job(5, 0), true).is_ok());
        assert_eq!(
            admit(&b, &Ref::Job(5, 0), false).unwrap_err(),
            "DOWNLOAD_NOT_FOUND"
        );
        assert_eq!(
            admit(&none, &Ref::Job(5, 0), false).unwrap_err(),
            "TOOL_NOT_AUTHORIZED"
        );
        assert_eq!(
            admit(&meta_only, &Ref::Job(5, 0), true).unwrap_err(),
            "TRANSFER_NOT_GRANTED"
        );
        // Grant references: only the recipient, only while live.
        super::super::artifacts::grant_root(&a.id, &root).unwrap();
        let grant = super::super::artifacts::register(&a, &file.canonicalize().unwrap()).unwrap();
        let meta = grant_view(&a, &grant.artifact_id, false).unwrap();
        assert!(meta["access"].is_null());
        assert!(meta["ifMatch"].is_null());
        let t = grant_view(&a, &grant.artifact_id, true).unwrap();
        assert_eq!(t["access"]["digest"], json!(grant.digest));
        assert_eq!(t["ifMatch"], json!(format!("\"{}\"", grant.digest)));
        assert!(
            !t.to_string().contains(dir.to_str().unwrap()),
            "local path leaked"
        );
        let schema = output_schema("artifact_access").unwrap();
        for key in schema["required"].as_array().unwrap() {
            assert!(
                t.get(key.as_str().unwrap()).is_some() && meta.get(key.as_str().unwrap()).is_some(),
                "missing {key}"
            );
        }
        // The export pass (`downloads::sanitize`) must not destroy the If-Match digest.
        let exported = super::super::downloads::sanitize(t.clone());
        assert_eq!(exported["access"]["digest"], t["access"]["digest"]);
        assert_eq!(
            exported["access"]["downloadPath"],
            t["access"]["downloadPath"]
        );
        assert!(grant_view(&b, &grant.artifact_id, false).is_err());
        assert_eq!(
            admit(&meta_only, &Ref::Grant(grant.artifact_id.clone()), false).unwrap_err(),
            "TRANSFER_NOT_GRANTED"
        );
        policy::db()
            .unwrap()
            .execute(
                "UPDATE artifact_grants SET expires=0 WHERE id=?1",
                [&grant.artifact_id],
            )
            .unwrap();
        assert!(grant_view(&a, &grant.artifact_id, false).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
