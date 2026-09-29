//! `POST /v1/company/rpc` on the local bridge (same bearer): the CLI's door
//! to the companies of agents. Body `{method, args}`; the answer is the
//! method's JSON. Owner decisions are refused here (they are taken in the
//! window), whatever the token.

use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use omniget_core::core::company::{self as co, rpc};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::local_bridge_llm::check_bearer;

#[derive(Deserialize)]
struct Body {
    method: String,
    #[serde(default)]
    args: Value,
}

pub fn router<S>(token: std::sync::Arc<String>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new().route(
        "/v1/company/rpc",
        post(move |headers: HeaderMap, Json(body): Json<Body>| {
            let token = token.clone();
            async move {
                if !check_bearer(&headers, &token) {
                    return (
                        StatusCode::UNAUTHORIZED,
                        Json(json!({"error": "bad or missing bearer token"})),
                    )
                        .into_response();
                }
                let out = tauri::async_runtime::spawn_blocking(move || {
                    let st = co::store()?;
                    let args = if body.args.is_null() {
                        json!({})
                    } else {
                        body.args
                    };
                    let r = rpc::dispatch_external(&st, &body.method, &args, "bridge", true);
                    if r.is_ok() && !rpc::is_read_only(&body.method) {
                        if let Some(e) = crate::company::get() {
                            e.poke();
                        }
                    }
                    r
                })
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r);
                respond(out)
            }
        }),
    )
}

fn respond(r: Result<Value, String>) -> Response {
    match r {
        Ok(v) => Json(v).into_response(),
        Err(e) => {
            let status = if e.starts_with(co::ERR_COMPANY_NOT_FOUND) {
                StatusCode::NOT_FOUND
            } else if e.starts_with(co::ERR_COMPANY_FORBIDDEN) {
                StatusCode::FORBIDDEN
            } else if e.starts_with(co::ERR_COMPANY_CONFLICT) {
                StatusCode::CONFLICT
            } else {
                StatusCode::BAD_REQUEST
            };
            (status, Json(json!({"error": e}))).into_response()
        }
    }
}

/// BN13-http: the same API calls the Paperclip baseline made over HTTP, through
/// this router, an axum server on loopback and a reqwest client.
/// `cargo test -p omniget --lib local_bridge_company::bench -- --nocapture --test-threads=1`
#[cfg(test)]
mod bench {
    use std::time::Instant;

    use serde_json::{json, Value};

    fn emit(metric: &str, value: f64, unit: &str) {
        let line = json!({"bench": "bn13-http", "metric": metric, "value": value, "unit": unit});
        println!("{line}");
        if let Ok(path) = std::env::var("OMNIGET_BENCH_OUT") {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let _ = writeln!(f, "{line}");
            }
        }
    }

    fn pct(v: &mut [f64], p: f64) -> f64 {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v[((v.len() as f64 - 1.0) * p).round() as usize]
    }

    #[test]
    fn bn13_http_api() {
        let dir = std::env::temp_dir().join(format!("omniget-bn13-http-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("OMNIGET_DATA_DIR", &dir);
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(8)
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let t_boot = Instant::now();
            let token = std::sync::Arc::new("bench-token".to_string());
            let app: axum::Router = super::router(token.clone());
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            let http = reqwest::Client::builder().pool_max_idle_per_host(64).build().unwrap();
            let url = format!("http://{addr}/v1/company/rpc");
            let call = |method: &'static str, args: Value| {
                let http = http.clone();
                let url = url.clone();
                async move {
                    let r = http.post(&url).bearer_auth("bench-token").json(&json!({"method": method, "args": args})).send().await.unwrap();
                    let ok = r.status().is_success();
                    (ok, r.json::<Value>().await.unwrap_or(Value::Null))
                }
            };
            let (_, list) = call("company.list", json!({})).await;
            assert!(list.is_array());
            emit("health_first_answer_ms", t_boot.elapsed().as_secs_f64() * 1000.0, "ms");
            let t = Instant::now();
            let (_, c) = call("company.create", json!({"company": {"name": "Bench Co", "require_hire_approval": false}})).await;
            emit("create_company_ms", t.elapsed().as_secs_f64() * 1000.0, "ms");
            let cid = c["id"].as_str().unwrap().to_string();
            let mut staff = Vec::new();
            let mut hire = Vec::new();
            for k in 0..20 {
                let t = Instant::now();
                let (ok, s) = call("staff.hire", json!({"company": cid, "staff": {"agent_id": format!("b{k}"), "name": format!("Agent {k}"), "role": "Engineer"}})).await;
                hire.push(t.elapsed().as_secs_f64() * 1000.0);
                assert!(ok, "{s}");
                staff.push(s["id"].as_str().unwrap().to_string());
            }
            emit("create_agent_p50_ms", pct(&mut hire, 0.5), "ms");
            let mut create = Vec::new();
            let mut ids = Vec::new();
            let t_all = Instant::now();
            for n in 0..1000 {
                let t = Instant::now();
                let (ok, i) = call("issue.create", json!({"company": cid, "issue": {"title": format!("Issue {n} about the login page"), "body": "Steps: open, click sign in."}})).await;
                create.push(t.elapsed().as_secs_f64() * 1000.0);
                assert!(ok, "{i}");
                ids.push(i["id"].as_str().unwrap().to_string());
            }
            emit("create_1000_total_s", t_all.elapsed().as_secs_f64(), "s");
            emit("create_p50_ms", pct(&mut create.clone(), 0.5), "ms");
            emit("create_p99_ms", pct(&mut create, 0.99), "ms");
            let mut list = Vec::new();
            for _ in 0..50 {
                let t = Instant::now();
                let (_, l) = call("issue.list", json!({"company": cid, "filter": {"limit": 500}})).await;
                list.push(t.elapsed().as_secs_f64() * 1000.0);
                assert_eq!(l.as_array().map(|a| a.len()), Some(500));
            }
            emit("list500_p50_ms", pct(&mut list.clone(), 0.5), "ms");
            emit("list500_p99_ms", pct(&mut list, 0.99), "ms");
            let (mut get, mut comment, mut status) = (Vec::new(), Vec::new(), Vec::new());
            for id in ids.iter().take(200) {
                let t = Instant::now();
                call("issue.get", json!({"company": cid, "issue": id})).await;
                get.push(t.elapsed().as_secs_f64() * 1000.0);
                let t = Instant::now();
                call("issue.comment", json!({"company": cid, "issue": id, "body": "looking"})).await;
                comment.push(t.elapsed().as_secs_f64() * 1000.0);
                let t = Instant::now();
                call("issue.status", json!({"company": cid, "issue": id, "status": "cancelled"})).await;
                status.push(t.elapsed().as_secs_f64() * 1000.0);
            }
            emit("get_p50_ms", pct(&mut get.clone(), 0.5), "ms");
            emit("get_p99_ms", pct(&mut get, 0.99), "ms");
            emit("comment_p50_ms", pct(&mut comment.clone(), 0.5), "ms");
            emit("comment_p99_ms", pct(&mut comment, 0.99), "ms");
            emit("status_p50_ms", pct(&mut status.clone(), 0.5), "ms");
            emit("status_p99_ms", pct(&mut status, 0.99), "ms");
            // 50 concurrent HTTP checkouts of one issue, 30 rounds.
            let mut resolve = Vec::new();
            for r in 0..30 {
                let (_, i) = call("issue.create", json!({"company": cid, "issue": {"title": format!("race {r}")}})).await;
                let iid = i["id"].as_str().unwrap().to_string();
                let t = Instant::now();
                let futs: Vec<_> = (0..50)
                    .map(|k| call("issue.checkout", json!({"company": cid, "issue": iid, "staff": staff[k % staff.len()], "run": format!("r{r}-{k}")})))
                    .collect();
                let res = futures::future::join_all(futs).await;
                resolve.push(t.elapsed().as_secs_f64() * 1000.0);
                assert_eq!(res.iter().filter(|(ok, _)| *ok).count(), 1, "round {r}");
            }
            emit("checkout50_exactly_one", 1.0, "bool");
            emit("checkout50_resolve_p50_ms", pct(&mut resolve.clone(), 0.5), "ms");
            emit("checkout50_resolve_max_ms", pct(&mut resolve, 1.0), "ms");
        });
        // Resident memory of this whole test process (server + client + store).
        let rss = sysinfo_rss_mb();
        emit("process_rss_after_1000_issues_mb", rss, "MB");
        let db = dir.join("llm").join("company.db");
        let size = std::fs::metadata(&db).map(|m| m.len()).unwrap_or(0) as f64 / 1e6;
        emit("company_db_mb_after_1000_issues", size, "MB");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn sysinfo_rss_mb() -> f64 {
        let out = std::process::Command::new("ps")
            .args(["-o", "rss=", "-p", &std::process::id().to_string()])
            .output();
        out.ok()
            .and_then(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .trim()
                    .parse::<f64>()
                    .ok()
            })
            .map(|kb| kb / 1024.0)
            .unwrap_or(0.0)
    }
}
