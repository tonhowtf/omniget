//! Server mode: the store's operations over the instance REST
//! (`/api/world/city3d/*`, agent G1). The app resolves the instance URL and
//! the OmniDisc session token (`commands/world/city.rs`) and hands them over;
//! the token never goes to disk from here.
//!
//! Routes (omnidisc-server `src/world/city3d/rest.rs`):
//! - `GET cities` → `{cities:[{city_id,name,policy,regions,…}]}`;
//! - `POST cities` `{charter, programs}` → 201 `{founded:true, regions, violations}`
//!   or 422 `{founded:false, violations}`, 409 `city_exists`;
//! - `GET {city}/program/{rx}/{ry}` → `{program, revision, seq}`;
//! - `GET {city}/blueprint/{rx}/{ry}` → `{blueprint|null, violations, revision}`;
//! - `POST {city}/propose` (a flat `Proposal` + `role`, `principal`, `reason`)
//!   → `ProposalResult` (the server keeps a draft; `summary.stale` flags an
//!   old base);
//! - `POST {city}/apply` (same + `idempotency_key`, required) →
//!   `{proposal_id, state, replayed, seq, result}`; 409 `stale_revision` with
//!   the current revision in `message`;
//! - `POST {city}/review/{proposal}` `{approve, reason?}` → same as apply;
//! - `GET {city}/preview/{rx}/{ry}?view=top|n|e|s|w&px=` → `image/png`;
//! - `GET {city}/log?limit=` → `{proposals, works}` in the server's shape,
//!   turned into the local log's shape by [`log_of`].
//!
//! Errors come as `{code, message}`; the token is the OmniDisc session bearer
//! (`/api/auth/login`), the same one `/api/world` takes.

use std::sync::RwLock;
use std::time::Duration;

use omniget_citygen::program::Proposal;
use omniget_citygen::Violation;
use serde::Serialize;
use serde_json::{json, Value};

use super::store::Author;
use super::ERR;

#[derive(Clone, Debug, Serialize)]
pub struct ServerConn {
    pub base: String,
    pub city: String,
    #[serde(skip)]
    pub token: String,
}

static CONN: RwLock<Option<ServerConn>> = RwLock::new(None);

pub fn set(conn: Option<ServerConn>) {
    if let Ok(mut g) = CONN.write() {
        *g = conn;
    }
    super::store::clear_cache();
}

pub fn current() -> Option<ServerConn> {
    CONN.read().ok().and_then(|g| g.clone())
}

impl ServerConn {
    fn url(&self, path: &str) -> String {
        format!(
            "{}/api/world/city3d/{}",
            self.base.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }

    fn client() -> Result<reqwest::Client, String> {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| format!("{ERR}_SERVER: {e}"))
    }

    async fn send(&self, req: reqwest::RequestBuilder) -> Result<reqwest::Response, String> {
        let res = req
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| format!("{ERR}_SERVER_UNREACHABLE: {e}"))?;
        if res.status().is_success() {
            return Ok(res);
        }
        let status = res.status();
        let text = res.text().await.unwrap_or_default();
        let code = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| v["code"].as_str().map(str::to_string))
            .unwrap_or_default();
        Err(format!(
            "{ERR}_SERVER_{}: {} {}",
            status.as_u16(),
            code,
            text.chars().take(300).collect::<String>()
        ))
    }

    /// Any answer, with its status: the caller decides what an error means.
    pub async fn post_status(&self, path: &str, body: &Value) -> Result<(u16, Value), String> {
        let res = Self::client()?
            .post(self.url(path))
            .json(body)
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| format!("{ERR}_SERVER_UNREACHABLE: {e}"))?;
        let status = res.status().as_u16();
        let text = res.text().await.unwrap_or_default();
        let v = if text.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text).unwrap_or(Value::String(text))
        };
        Ok((status, v))
    }

    pub async fn get(&self, path: &str) -> Result<Value, String> {
        let res = self.send(Self::client()?.get(self.url(path))).await?;
        res.json()
            .await
            .map_err(|e| format!("{ERR}_SERVER_BODY: {e}"))
    }

    pub async fn post(&self, path: &str, body: &Value) -> Result<Value, String> {
        let res = self
            .send(Self::client()?.post(self.url(path)).json(body))
            .await?;
        let text = res.text().await.unwrap_or_default();
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&text).map_err(|e| format!("{ERR}_SERVER_BODY: {e}"))
    }

    /// PNG bytes, whether the server answers `image/png` or `{png_base64}`.
    pub async fn get_png(&self, path: &str) -> Result<Vec<u8>, String> {
        use base64::Engine;
        let res = self.send(Self::client()?.get(self.url(path))).await?;
        let is_json = res
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains("json"));
        let bytes = res
            .bytes()
            .await
            .map_err(|e| format!("{ERR}_SERVER_BODY: {e}"))?;
        if !is_json {
            return Ok(bytes.to_vec());
        }
        let v: Value =
            serde_json::from_slice(&bytes).map_err(|e| format!("{ERR}_SERVER_BODY: {e}"))?;
        let b64 = v["png_base64"].as_str().unwrap_or_default();
        base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(|e| format!("{ERR}_SERVER_BODY: {e}"))
    }

    pub fn city_path(&self, rest: &str) -> String {
        format!("{}/{}", self.city, rest)
    }
}

/// `field` of an answer, or the answer itself when it is not wrapped.
pub fn unwrap_field(v: Value, field: &str) -> Value {
    match v.get(field) {
        Some(inner) if !inner.is_null() => inner.clone(),
        _ => v,
    }
}

/// The server's city list, normalised to the local status' city fields
/// (`city_id`, `name`, `publish_policy`, `regions`, …).
pub fn cities_of(v: Value) -> Vec<Value> {
    let list = unwrap_field(v, "cities");
    list.as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|c| {
            let id = c["city_id"]
                .as_str()
                .or_else(|| c["id"].as_str())
                .unwrap_or_default()
                .to_string();
            json!({
                "city_id": id,
                "name": c["name"],
                "charter": c["charter"],
                "publish_policy": c["policy"],
                "regions": c["regions"],
                "source": c["source"],
                "runtime_3d": c["runtime_3d"],
                "founded_by": c["founded_by"],
            })
        })
        .collect()
}

/// The error string `send` builds, for an answer read by `post_status`.
pub fn error_of(status: u16, v: &Value) -> String {
    format!(
        "{ERR}_SERVER_{status}: {} {}",
        v["code"].as_str().unwrap_or_default(),
        v["message"].as_str().unwrap_or_default()
    )
}

/// The server's `view` query value: `top`, `n`, `e`, `s` or `w`.
pub fn view_param(view: &str) -> &'static str {
    use omniget_citygen::preview::View;
    use omniget_citygen::Dir;
    match super::api::parse_view(view) {
        View::Top => "top",
        View::Oblique(Dir::N) => "n",
        View::Oblique(Dir::E) => "e",
        View::Oblique(Dir::W) => "w",
        _ => "s",
    }
}

/// `role` the server knows (`prefeitura | conselho | morador | pessoa`), or
/// none to let it pick (city hall for a city admin, `pessoa` otherwise).
fn role_of(author: &Author) -> Option<&'static str> {
    match author.role.as_deref() {
        Some("prefeitura") => Some("prefeitura"),
        Some("conselho") => Some("conselho"),
        Some("morador") => Some("morador"),
        _ if author.is_machine() => Some("pessoa"),
        _ => None,
    }
}

/// `agent:<name>` for an LLM or MCP author; the server records people by
/// their user id.
fn principal_of(author: &Author) -> Option<String> {
    if !author.is_machine() {
        return None;
    }
    let name: String = author
        .name
        .chars()
        .filter(|c| !c.is_control())
        .take(58)
        .collect();
    Some(format!(
        "agent:{}",
        if name.is_empty() { "llm" } else { &name }
    ))
}

/// Body of `propose` and `apply`: the proposal's fields at the top level.
pub fn proposal_body(
    proposal: &Proposal,
    author: &Author,
    reason: &str,
    idempotency_key: Option<&str>,
) -> Value {
    let mut body = serde_json::to_value(proposal).unwrap_or_else(|_| json!({}));
    let o = body.as_object_mut().expect("a proposal is an object");
    if let Some(role) = role_of(author) {
        o.insert("role".into(), json!(role));
    }
    if let Some(p) = principal_of(author) {
        o.insert("principal".into(), json!(p));
    }
    if !reason.trim().is_empty() {
        o.insert(
            "reason".into(),
            json!(reason.chars().take(280).collect::<String>()),
        );
    }
    if let Some(key) = idempotency_key {
        o.insert("idempotency_key".into(), json!(key));
    }
    body
}

/// A stale base as the local store reports it: a `P-REVISION` error.
pub fn stale_violation(base: &str, current: &str) -> Violation {
    super::store::violation(
        "P-REVISION",
        "base_revision",
        &format!("{base} != {current}"),
        "the region changed since you read it: read it again and redo the ops on the current revision",
    )
}

fn ms_of(v: &Value) -> u64 {
    v.as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.timestamp_millis().max(0) as u64)
        .unwrap_or(0)
}

/// The server's log in the local log's shape (`LogEntry`, `Work`), so the
/// panels read one format.
pub fn log_of(v: Value) -> Value {
    let proposals: Vec<Value> = v["proposals"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|p| {
            let principal = p["principal"].as_str().unwrap_or_default();
            let role = p["role"].as_str().unwrap_or_default();
            let (kind, name) = match principal.strip_prefix("agent:") {
                Some(agent) => ("llm", agent.to_string()),
                None => ("person", principal.to_string()),
            };
            let role = match role {
                "prefeitura" | "conselho" | "morador" => Value::from(role),
                _ => Value::Null,
            };
            let at = if p["decided_at"].is_string() {
                ms_of(&p["decided_at"])
            } else {
                ms_of(&p["created_at"])
            };
            json!({
                "id": p["id"],
                "at_ms": at,
                "region": p["region"],
                "author": { "kind": kind, "name": name, "role": role },
                "reason": p["reason"].as_str().unwrap_or_default(),
                "ops": p["ops"].as_array().cloned().unwrap_or_default(),
                "base_revision": p["base_revision"].as_str().unwrap_or_default(),
                "revision": p["result_revision"],
                "violations": p["violations"].as_array().cloned().unwrap_or_default(),
                "state": p["state"],
                "summary": p["summary"],
                "admin": false,
                "idempotency_key": null,
                "tokens": null,
                "resolution": p["decided_by"].as_str().map(|u| format!("decided by user {u}")),
                "proposal_id": p["id"],
            })
        })
        .collect();
    let works: Vec<Value> = v["works"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|w| w["state"] == "building")
        .map(|w| {
            json!({
                "region": w["region"],
                "id": w["building"],
                "lot": "",
                "use": "",
                "label": null,
                "progress": w["progress"],
                "work_id": w["id"],
                "stages": w["stages"],
            })
        })
        .collect();
    json!({ "proposals": proposals, "works": works })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::city3d::api;
    use omniget_citygen::program::{BuildingSpec, Intent, PatchOp};

    /// End to end against a running omnidisc-server with the city3d routes:
    /// `OMNIGET_CITY3D_SERVER=http://127.0.0.1:8080 OMNIGET_CITY3D_TOKEN=od1.…
    /// cargo test -p omniget-core city3d_server_e2e -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn city3d_server_e2e() {
        let base = std::env::var("OMNIGET_CITY3D_SERVER").expect("OMNIGET_CITY3D_SERVER");
        let token = std::env::var("OMNIGET_CITY3D_TOKEN").expect("OMNIGET_CITY3D_TOKEN");
        let city_id = format!("e2e-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
        set(Some(ServerConn {
            base,
            city: city_id.clone(),
            token,
        }));
        assert_eq!(api::mode(), "server");

        // Found: example charter + default program.
        let (charter, program, local) = api::example_founding(&city_id, "omni-e2e");
        assert!(!super::super::store::has_errors(&local), "{local:?}");
        let found = api::found(charter, vec![program], Author::person(), None)
            .await
            .expect("found");
        eprintln!("found {} → {:?}", found.city_id, found.revisions);
        assert!(found.founded, "{:?}", found.violations);
        assert_eq!(found.revisions.len(), 1);

        let status = api::status().await.expect("status");
        assert_eq!(status["city"]["city_id"], city_id.as_str());
        assert!(status["city"]["charter"].is_object());

        // Read the program.
        let region = api::region([0, 0]).await.expect("region");
        let revision = region["revision"].as_str().unwrap().to_string();
        assert_eq!(revision, found.revisions[0].1);
        let prog: CityProgram_ = serde_json::from_value(region["program"].clone()).unwrap();
        assert_eq!(prog.revision(), revision);
        assert!(region["blueprint"].is_object(), "blueprint missing");

        // A proposal with a violation stays a rejection.
        let bad = Proposal {
            region: [0, 0],
            base_revision: revision.clone(),
            ops: vec![PatchOp::Remove {
                id: "no-such-thing".into(),
            }],
            admin: false,
        };
        let out = api::propose(bad, Author::person(), "e2e bad".into())
            .await
            .expect("propose");
        eprintln!("bad proposal → {} {:?}", out.state, out.result.violations);
        assert_eq!(out.state, "rejected");
        assert!(super::super::store::has_errors(&out.result.violations));
        assert!(out.log_id.is_some());

        // A stale base is reported as P-REVISION, not a transport error.
        let old = Proposal {
            region: [0, 0],
            base_revision: "sha256:old".into(),
            ops: vec![PatchOp::Remove {
                id: "no-such-thing".into(),
            }],
            admin: false,
        };
        let v = api::validate(old, Author::person())
            .await
            .expect("validate");
        assert!(v.violations.iter().any(|x| x.code == "P-REVISION"));

        // A valid work: a new house in the first zone, else a change of
        // intent on an existing building.
        let mut candidates = Vec::new();
        for z in &prog.zones {
            candidates.push(vec![PatchOp::AddBuilding(BuildingSpec {
                id: "casa-e2e".into(),
                lot: format!("auto:zone:{}", z.id),
                owner_ref: None,
                intent: Intent {
                    use_: "casa".into(),
                    floors: Some(1),
                    ..Default::default()
                },
                typology: None,
                progress: 100,
            })]);
        }
        if let Some(b) = prog.buildings.first() {
            candidates.push(vec![PatchOp::SetIntent {
                id: b.id.clone(),
                patch: serde_json::json!({ "awning": true }),
            }]);
        }
        let mut applied = None;
        for ops in candidates {
            let p = Proposal {
                region: [0, 0],
                base_revision: revision.clone(),
                ops,
                admin: false,
            };
            let dry = api::validate(p.clone(), Author::person())
                .await
                .expect("validate");
            if super::super::store::has_errors(&dry.violations) {
                eprintln!("candidate refused: {:?}", dry.violations);
                continue;
            }
            let key = uuid::Uuid::new_v4().to_string();
            let out = api::apply(
                p.clone(),
                Author::person(),
                "e2e work".into(),
                Some(key.clone()),
                None,
            )
            .await
            .expect("apply");
            eprintln!("apply → {} {}", out.state, out.result.revision);
            assert_eq!(out.state, "applied", "{:?}", out.result.violations);
            // Same key: the server replays the first answer.
            let again = api::apply(
                p.clone(),
                Author::person(),
                "e2e work".into(),
                Some(key),
                None,
            )
            .await
            .expect("replay");
            assert_eq!(again.result.revision, out.result.revision);
            // Old base after a real change: stale.
            let stale = api::apply(p, Author::person(), "e2e stale".into(), None, None)
                .await
                .expect("stale apply");
            assert_eq!(stale.state, "stale", "{:?}", stale.result.violations);
            applied = Some(out);
            break;
        }
        let applied = applied.expect("no valid work among the candidates");

        // Blueprint of the new revision.
        let region = api::region([0, 0]).await.expect("region after");
        // The works ticker may have moved progress on since; that is fine.
        eprintln!(
            "revision after apply: {} (apply said {})",
            region["revision"], applied.result.revision
        );
        let bp: omniget_citygen::Blueprint =
            serde_json::from_value(region["blueprint"].clone()).expect("blueprint");
        eprintln!(
            "blueprint: {} buildings, {} lots",
            bp.buildings.len(),
            bp.lots.len()
        );
        let mesh = api::region_mesh([0, 0]).await.expect("mesh");
        assert!(mesh["instances"].is_object() || mesh["instances"].is_array());

        // Preview PNG.
        let png = api::preview_png([0, 0], "oblique_s", 128)
            .await
            .expect("preview");
        assert!(
            png.starts_with(b"\x89PNG"),
            "not a PNG ({} bytes)",
            png.len()
        );
        let top = api::preview_png([0, 0], "top", 64)
            .await
            .expect("preview top");
        assert!(top.starts_with(b"\x89PNG"));

        // Log in the local shape.
        let log = api::log(50).await.expect("log");
        let proposals = log["proposals"].as_array().expect("proposals");
        eprintln!(
            "log: {} proposals, {} works",
            proposals.len(),
            log["works"].as_array().map(Vec::len).unwrap_or(0)
        );
        assert!(proposals.iter().any(|p| p["state"] == "applied"));
        assert!(proposals.iter().any(|p| p["state"] == "rejected"));
        assert!(proposals
            .iter()
            .all(|p| p["author"]["kind"].is_string() && p["at_ms"].is_u64()));
        let entries: Vec<super::super::store::LogEntry> =
            serde_json::from_value(log["proposals"].clone()).expect("LogEntry shape");
        assert!(!entries.is_empty());
        set(None);
    }

    type CityProgram_ = omniget_citygen::program::CityProgram;
}
