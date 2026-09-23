//! E2B provider over its HTTP API (no SDK): create a sandbox with the user's
//! key from the vault, upload a copy of the project, install the CLI, run it
//! through envd's `process.Process/Start` (Connect protocol, JSON envelopes),
//! download what changed into the local copy, kill the sandbox. The local copy
//! then gives the same reviewable diff as the Docker provider.

use std::collections::BTreeMap;
use std::time::Duration;

use base64::Engine as _;
use futures::StreamExt;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::super::runner::{self, Collector, RunOutput, ToolRun};
use super::{docker, key_env, run_dir, save_record, tree, RunRecord, SandboxOpts, ERR_SANDBOX};

const ENVD_PORT: u16 = 49983;
const DOMAINS: &[&str] = &["e2b.app", "e2b.dev"];
const WORK: &str = "/home/user/work";
const MAX_UPLOAD_FILES: usize = 3000;

struct Sbx {
    id: String,
    domain: String,
    api_key: String,
    token: Option<String>,
    http: reqwest::Client,
}

impl Sbx {
    fn envd(&self) -> String {
        format!("https://{ENVD_PORT}-{}.{}", self.id, self.domain)
    }

    fn auth(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let rb = rb.header(
            "Authorization",
            format!(
                "Basic {}",
                base64::engine::general_purpose::STANDARD.encode("user:")
            ),
        );
        match &self.token {
            Some(t) => rb.header("X-Access-Token", t),
            None => rb,
        }
    }

    async fn kill(&self) {
        let _ = self
            .http
            .delete(format!("https://api.{}/sandboxes/{}", self.domain, self.id))
            .header("X-API-Key", &self.api_key)
            .send()
            .await;
    }

    async fn upload(&self, path: &str, bytes: Vec<u8>) -> Result<(), String> {
        let part = reqwest::multipart::Part::bytes(bytes).file_name("file");
        let form = reqwest::multipart::Form::new().part("file", part);
        let res = self
            .auth(self.http.post(format!("{}/files", self.envd())))
            .query(&[("path", path), ("username", "user")])
            .multipart(form)
            .send()
            .await
            .map_err(|e| format!("{ERR_SANDBOX}: e2b upload {path}: {e}"))?;
        if !res.status().is_success() {
            return Err(format!(
                "{ERR_SANDBOX}: e2b upload {path}: HTTP {}",
                res.status()
            ));
        }
        Ok(())
    }

    async fn download(&self, path: &str) -> Result<Vec<u8>, String> {
        let res = self
            .auth(self.http.get(format!("{}/files", self.envd())))
            .query(&[("path", path), ("username", "user")])
            .send()
            .await
            .map_err(|e| format!("{ERR_SANDBOX}: e2b download {path}: {e}"))?;
        if !res.status().is_success() {
            return Err(format!(
                "{ERR_SANDBOX}: e2b download {path}: HTTP {}",
                res.status()
            ));
        }
        Ok(res.bytes().await.map_err(|e| e.to_string())?.to_vec())
    }

    /// Runs `bash -lc <cmd>`; stdout lines go to `on_stdout`. Returns the exit code.
    async fn exec(
        &self,
        cmd: &str,
        envs: &BTreeMap<String, String>,
        cancel: &CancellationToken,
        on_stdout: &mut (dyn FnMut(&str) + Send),
        stderr: &mut String,
    ) -> Result<Option<i32>, String> {
        let body = json!({
            "process": { "cmd": "/bin/bash", "args": ["-l", "-c", cmd], "envs": envs, "cwd": WORK }
        });
        let res = self
            .auth(
                self.http
                    .post(format!("{}/process.Process/Start", self.envd())),
            )
            .header("Content-Type", "application/connect+json")
            .header("Connect-Protocol-Version", "1")
            .timeout(Duration::from_secs(60 * 60))
            .body(envelope(0, &body))
            .send()
            .await
            .map_err(|e| format!("{ERR_SANDBOX}: e2b process: {e}"))?;
        if !res.status().is_success() {
            let t = res.text().await.unwrap_or_default();
            return Err(format!(
                "{ERR_SANDBOX}: e2b process: {}",
                t.chars().take(300).collect::<String>()
            ));
        }
        let mut stream = res.bytes_stream();
        let mut buf: Vec<u8> = Vec::new();
        let mut line = String::new();
        let mut exit = None;
        loop {
            let chunk = tokio::select! {
                _ = cancel.cancelled() => return Ok(None),
                c = stream.next() => c,
            };
            let Some(chunk) = chunk else { break };
            buf.extend_from_slice(&chunk.map_err(|e| format!("{ERR_SANDBOX}: e2b stream: {e}"))?);
            while let Some((flags, msg, used)) = take_envelope(&buf) {
                buf.drain(..used);
                if flags & 0x02 != 0 {
                    if let Some(err) = msg.get("error") {
                        return Err(format!(
                            "{ERR_SANDBOX}: e2b: {}",
                            err["message"].as_str().unwrap_or("stream error")
                        ));
                    }
                    continue;
                }
                let ev = &msg["event"];
                if let Some(d) = ev["data"]["stdout"].as_str() {
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(d)
                        .unwrap_or_default();
                    line.push_str(&String::from_utf8_lossy(&bytes));
                    while let Some(i) = line.find('\n') {
                        let l: String = line.drain(..=i).collect();
                        on_stdout(l.trim_end_matches(['\n', '\r']));
                    }
                }
                if let Some(d) = ev["data"]["stderr"].as_str() {
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(d)
                        .unwrap_or_default();
                    stderr.push_str(&String::from_utf8_lossy(&bytes));
                    if stderr.len() > 8192 {
                        let cut = stderr.len() - 4096;
                        let cut = (cut..stderr.len())
                            .find(|i| stderr.is_char_boundary(*i))
                            .unwrap_or(0);
                        stderr.drain(..cut);
                    }
                }
                if ev["end"].is_object() {
                    exit = Some(ev["end"]["exitCode"].as_i64().unwrap_or(0) as i32);
                }
            }
        }
        if !line.trim().is_empty() {
            on_stdout(line.trim_end());
        }
        Ok(exit)
    }
}

/// Connect envelope: flags byte + big-endian length + JSON.
pub fn envelope(flags: u8, v: &Value) -> Vec<u8> {
    let body = serde_json::to_vec(v).unwrap_or_default();
    let mut out = Vec::with_capacity(body.len() + 5);
    out.push(flags);
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(&body);
    out
}

/// One complete envelope at the start of `buf`: (flags, json, bytes used).
pub fn take_envelope(buf: &[u8]) -> Option<(u8, Value, usize)> {
    if buf.len() < 5 {
        return None;
    }
    let len = u32::from_be_bytes([buf[1], buf[2], buf[3], buf[4]]) as usize;
    if buf.len() < 5 + len {
        return None;
    }
    let v = serde_json::from_slice(&buf[5..5 + len]).unwrap_or(Value::Null);
    Some((buf[0], v, 5 + len))
}

pub fn shell_quote(a: &str) -> String {
    if !a.is_empty()
        && a.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./=:@,+".contains(c))
    {
        return a.to_string();
    }
    format!("'{}'", a.replace('\'', "'\\''"))
}

async fn create(http: &reqwest::Client, key: &str) -> Result<Sbx, String> {
    let mut last = String::new();
    for domain in DOMAINS {
        let res = http
            .post(format!("https://api.{domain}/sandboxes"))
            .header("X-API-Key", key)
            .json(&json!({ "templateID": "base", "timeout": 1800 }))
            .send()
            .await;
        match res {
            Ok(r) if r.status().is_success() => {
                let v: Value = r
                    .json()
                    .await
                    .map_err(|e| format!("{ERR_SANDBOX}: e2b: {e}"))?;
                let id = v["sandboxID"].as_str().unwrap_or("").to_string();
                if id.is_empty() {
                    return Err(format!("{ERR_SANDBOX}: e2b answered without a sandboxID"));
                }
                return Ok(Sbx {
                    id,
                    domain: v["domain"]
                        .as_str()
                        .filter(|d| !d.is_empty())
                        .unwrap_or(domain)
                        .to_string(),
                    api_key: key.to_string(),
                    token: v["envdAccessToken"].as_str().map(str::to_string),
                    http: http.clone(),
                });
            }
            Ok(r) if r.status().as_u16() == 401 || r.status().as_u16() == 403 => {
                return Err(format!(
                    "{ERR_SANDBOX}: E2B refused the key (HTTP {})",
                    r.status()
                ));
            }
            Ok(r) => last = format!("HTTP {} from api.{domain}", r.status()),
            Err(e) => last = format!("api.{domain}: {e}"),
        }
    }
    Err(format!(
        "{ERR_SANDBOX}: could not create an E2B sandbox ({last})"
    ))
}

pub async fn run(
    id: &str,
    run: &ToolRun,
    opts: &SandboxOpts,
    cancel: CancellationToken,
    on_log: &mut (dyn FnMut(&str) + Send),
) -> Result<(RunOutput, RunRecord), String> {
    let e2b_id = opts
        .e2b_key_id
        .as_deref()
        .ok_or_else(|| format!("{ERR_SANDBOX}: pick the vault entry that holds your E2B key"))?;
    let e2b_key = crate::core::tools::ai_keys::entry_with_secret(e2b_id)
        .map_err(|e| format!("{ERR_SANDBOX}: E2B key: {e}"))?
        .key;
    if e2b_key.is_empty() {
        return Err(format!("{ERR_SANDBOX}: the E2B vault entry has no key"));
    }
    if opts.bind_original {
        return Err(format!(
            "{ERR_SANDBOX}: E2B always works on a copy (bind mode is Docker only)"
        ));
    }
    let project = run
        .cwd
        .clone()
        .ok_or_else(|| format!("{ERR_SANDBOX}: a sandbox run needs a project folder"))?;
    let pkg = docker::package_of(&run.tool)
        .ok_or_else(|| format!("{ERR_SANDBOX}: no E2B recipe for `{}`", run.tool))?;
    let (target, rc) = runner::runner_of(&run.tool)?;
    let launch = runner::launch(target, rc, run);
    let env: BTreeMap<String, String> = key_env(&run.tool, opts.key_id.as_deref())?
        .into_iter()
        .collect();

    let dir = run_dir(id)?;
    let work = dir.join("work");
    if work.exists() {
        let _ = std::fs::remove_dir_all(&work);
    }
    let base = tree::copy_tree(&project, &work)?;
    if base.len() > MAX_UPLOAD_FILES {
        return Err(format!(
            "{ERR_SANDBOX}: {} files is too many to upload to E2B (max {MAX_UPLOAD_FILES}); use Docker",
            base.len()
        ));
    }
    let record = RunRecord {
        id: id.to_string(),
        provider: "e2b".into(),
        tool: run.tool.clone(),
        project: project.clone(),
        work: work.clone(),
        bind_original: false,
        image: Some("e2b:base".into()),
        env_names: env.keys().cloned().collect(),
        argv_preview: std::iter::once(launch.program.clone())
            .chain(launch.args.iter().cloned())
            .collect(),
        applied: vec![],
    };
    save_record(&dir, &record, Some(&base))?;

    let http = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;
    on_log("· creating the E2B sandbox");
    let sbx = create(&http, &e2b_key).await?;
    let result = async {
        on_log(&format!("· uploading {} files", base.len()));
        for rel in base.keys() {
            if cancel.is_cancelled() {
                return Err(format!("{ERR_SANDBOX}: cancelled"));
            }
            let bytes = std::fs::read(work.join(rel)).map_err(|e| e.to_string())?;
            sbx.upload(&format!("{WORK}/{rel}"), bytes).await?;
        }
        let mut sink = String::new();
        on_log(&format!("· installing {pkg}"));
        let install = format!(
            "command -v {} >/dev/null 2>&1 || sudo npm install -g {pkg} >/tmp/og-install.log 2>&1 || npm install -g {pkg} >>/tmp/og-install.log 2>&1",
            shell_quote(&launch.program)
        );
        sbx.exec(&install, &BTreeMap::new(), &cancel, &mut |_l: &str| {}, &mut sink).await?;
        let cmdline = format!(
            "touch /tmp/.og-start && {}",
            std::iter::once(launch.program.clone())
                .chain(launch.args.iter().cloned())
                .map(|a| shell_quote(&a))
                .collect::<Vec<_>>()
                .join(" ")
        );
        let mut col = Collector::default();
        let mut stderr = String::new();
        let exit = {
            let mut feed = |l: &str| {
                for log in col.feed(l) {
                    on_log(&log);
                }
            };
            sbx.exec(&cmdline, &env, &cancel, &mut feed, &mut stderr).await?
        };
        // Bring back what changed.
        let mut listing = String::new();
        let mut collect = |l: &str| {
            listing.push_str(l);
            listing.push('\n');
        };
        let skip = tree::SKIP
            .iter()
            .map(|d| format!("-not -path '*/{d}/*'"))
            .collect::<Vec<_>>()
            .join(" ");
        sbx.exec(
            &format!("cd {WORK} && find . -type f -newer /tmp/.og-start {skip} -not -path './.git/*'; echo '@@ALL@@'; find . -type f {skip}"),
            &BTreeMap::new(),
            &CancellationToken::new(),
            &mut collect,
            &mut sink,
        )
        .await?;
        let (changed, all) = listing.split_once("@@ALL@@").unwrap_or((&listing, ""));
        let norm = |l: &str| l.trim().trim_start_matches("./").to_string();
        for rel in changed.lines().map(norm).filter(|l| !l.is_empty()) {
            if rel.split('/').any(|s| s == "..") {
                continue;
            }
            let bytes = sbx.download(&format!("{WORK}/{rel}")).await?;
            let dest = work.join(&rel);
            if let Some(p) = dest.parent() {
                std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
            }
            std::fs::write(&dest, bytes).map_err(|e| e.to_string())?;
        }
        let remote: std::collections::BTreeSet<String> = all.lines().map(norm).filter(|l| !l.is_empty()).collect();
        if !remote.is_empty() {
            for rel in base.keys() {
                if !remote.contains(rel) && !rel.starts_with(".git/") {
                    let _ = std::fs::remove_file(work.join(rel));
                }
            }
        }
        let text = col.text();
        let error = col.error.clone().or_else(|| {
            (exit.map(|c| c != 0).unwrap_or(true) && text.trim().is_empty())
                .then(|| format!("exit {}: {}", exit.map(|c| c.to_string()).unwrap_or_else(|| "?".into()), stderr.lines().last().unwrap_or("")))
        });
        Ok(RunOutput {
            text,
            exit_code: exit,
            usage: col.usage.clone(),
            session_id: col.session_id.clone(),
            error,
            stderr_tail: stderr,
            cancelled: cancel.is_cancelled(),
        })
    }
    .await;
    sbx.kill().await;
    on_log("· E2B sandbox closed");
    result.map(|o| (o, record))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_envelopes_round_trip() {
        let e = envelope(0, &json!({"a": 1}));
        let mut two = e.clone();
        two.extend(envelope(2, &json!({})));
        let (f, v, used) = take_envelope(&two).unwrap();
        assert_eq!((f, v["a"].as_i64(), used), (0, Some(1), e.len()));
        let (f2, _, _) = take_envelope(&two[used..]).unwrap();
        assert_eq!(f2, 2);
        assert!(take_envelope(&e[..4]).is_none());
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote("--model"), "--model");
    }
}
