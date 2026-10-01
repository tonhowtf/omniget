use std::path::Path;

use anyhow::{anyhow, Context};
use base64::Engine;

use super::transcript::{parse_any, Segment};
use crate::core::tools::{report, ProgressFn};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TeamsInput {
    Manifest(String),
    SharePoint {
        host: String,
        site_path: String,
        item_url: String,
    },
    TranscriptUrl(String),
}

pub fn is_sharepoint_host(host: &str) -> bool {
    let h = host.to_ascii_lowercase();
    h.ends_with(".sharepoint.com") || h.ends_with(".sharepoint.us") || h.ends_with(".sharepoint.cn")
}

pub fn classify(raw: &str) -> Option<TeamsInput> {
    let raw = raw.trim();
    let url = url::Url::parse(raw).ok()?;
    if !matches!(url.scheme(), "https" | "http") {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    let path = url.path();
    if path.contains("/videomanifest") {
        return Some(TeamsInput::Manifest(trim_manifest(raw)));
    }
    if path.contains("/media/transcripts") {
        return Some(TeamsInput::TranscriptUrl(raw.to_string()));
    }
    if !is_sharepoint_host(&host) {
        return None;
    }
    let lower = path.to_ascii_lowercase();
    let item_url = if lower.contains("/_layouts/15/stream.aspx")
        || lower.contains("/_layouts/15/embed.aspx")
    {
        let id = url
            .query_pairs()
            .find(|(k, _)| k.eq_ignore_ascii_case("id"))
            .map(|(_, v)| v.to_string())?;
        if !id.starts_with('/') {
            return None;
        }
        format!("https://{host}{id}")
    } else if path.starts_with("/:v:/") || path.starts_with("/:u:/") || path.starts_with("/:f:/") {
        raw.to_string()
    } else if [".mp4", ".m4a", ".mp3", ".webm", ".mov", ".wav"]
        .iter()
        .any(|e| lower.ends_with(e))
    {
        format!("https://{host}{path}")
    } else {
        return None;
    };
    let item_path = url::Url::parse(&item_url)
        .ok()
        .map(|u| u.path().to_string())
        .unwrap_or_default();
    Some(TeamsInput::SharePoint {
        site_path: site_path_of(&item_path),
        host,
        item_url,
    })
}

fn site_path_of(path: &str) -> String {
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let segs: &[&str] = if segs.first().is_some_and(|s| s.starts_with(':')) {
        segs.get(2..).unwrap_or(&[])
    } else {
        &segs
    };
    match segs {
        [kind, name, ..] if matches!(*kind, "personal" | "sites" | "teams") => {
            format!("/{kind}/{name}")
        }
        _ => String::new(),
    }
}

pub fn trim_manifest(url: &str) -> String {
    const DASH: &str = "index&format=dash";
    if let Some(i) = url.find(DASH) {
        return url[..i + DASH.len()].to_string();
    }
    if let Some(i) = url.to_ascii_lowercase().find("&altmanifest") {
        return url[..i].to_string();
    }
    url.to_string()
}

pub fn share_id(url: &str) -> String {
    format!(
        "u!{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(url.trim_end_matches('/'))
    )
}

#[derive(Debug, Clone, Default)]
pub struct ResolvedRecording {
    pub name: String,
    pub download_url: Option<String>,
    pub transcript_url: Option<String>,
}

#[derive(Debug)]
pub struct AuthRequired {
    pub host: String,
    pub status: u16,
}

impl std::fmt::Display for AuthRequired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "SharePoint sign-in required for {} (HTTP {})",
            self.host, self.status
        )
    }
}

impl std::error::Error for AuthRequired {}

fn client() -> anyhow::Result<reqwest::Client> {
    crate::core::tools::client()
}

async fn get_json(
    client: &reqwest::Client,
    url: &str,
    host: &str,
    cookie: Option<&str>,
) -> anyhow::Result<serde_json::Value> {
    let mut req = client.get(url).header("Accept", "application/json");
    if let Some(c) = cookie {
        req = req.header("Cookie", c);
    }
    let resp = req.send().await?;
    let status = resp.status();
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err(AuthRequired {
            host: host.to_string(),
            status: status.as_u16(),
        }
        .into());
    }
    let is_json = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("json"));
    if !status.is_success() {
        return Err(anyhow!("SharePoint answered HTTP {}", status.as_u16()));
    }
    if !is_json {
        return Err(AuthRequired {
            host: host.to_string(),
            status: status.as_u16(),
        }
        .into());
    }
    Ok(resp.json().await?)
}

pub async fn resolve_sharepoint(
    input: &TeamsInput,
    cookie: Option<&str>,
) -> anyhow::Result<ResolvedRecording> {
    let TeamsInput::SharePoint {
        host,
        site_path,
        item_url,
    } = input
    else {
        return Err(anyhow!("not a SharePoint recording"));
    };
    let client = client()?;
    let base = format!("https://{host}{site_path}/_api/v2.1");
    let item = get_json(
        &client,
        &format!("{base}/shares/{}/driveItem", share_id(item_url)),
        host,
        cookie,
    )
    .await
    .context("could not open the recording on SharePoint")?;
    let name = item["name"].as_str().unwrap_or("recording").to_string();
    let download_url = item["@content.downloadUrl"]
        .as_str()
        .or_else(|| item["@microsoft.graph.downloadUrl"].as_str())
        .map(str::to_string);
    let item_id = item["id"].as_str().unwrap_or("");
    let drive_id = item["parentReference"]["driveId"].as_str().unwrap_or("");
    let mut transcript_url = None;
    if !item_id.is_empty() && !drive_id.is_empty() {
        let item_base = format!("{base}/drives/{drive_id}/items/{item_id}");
        let expanded = get_json(
            &client,
            &format!("{item_base}?select=media/transcripts&$expand=media/transcripts"),
            host,
            cookie,
        )
        .await
        .ok();
        transcript_url = expanded.as_ref().and_then(pick_transcript);
        if transcript_url.is_none() {
            if let Ok(list) = get_json(
                &client,
                &format!("{item_base}/media/transcripts"),
                host,
                cookie,
            )
            .await
            {
                transcript_url = pick_transcript(&list);
            }
        }
    }
    Ok(ResolvedRecording {
        name,
        download_url,
        transcript_url,
    })
}

fn pick_transcript(v: &serde_json::Value) -> Option<String> {
    let list = v["media"]["transcripts"]
        .as_array()
        .or_else(|| v["value"].as_array())
        .cloned()
        .unwrap_or_else(|| vec![v.clone()]);
    let chosen = list
        .iter()
        .find(|t| t["isDefault"].as_bool() == Some(true))
        .or_else(|| list.first())?;
    chosen["temporaryDownloadUrl"].as_str().map(str::to_string)
}

pub async fn fetch_transcript(url: &str, cookie: Option<&str>) -> anyhow::Result<Vec<Segment>> {
    let client = client()?;
    let json_url = if url.contains("format=") {
        url.to_string()
    } else if url.contains('?') {
        format!("{url}&format=json")
    } else {
        format!("{url}?format=json")
    };
    for candidate in [json_url.as_str(), url] {
        let mut req = client.get(candidate);
        if let Some(c) = cookie {
            req = req.header("Cookie", c);
        }
        let Ok(resp) = req.send().await else {
            continue;
        };
        if !resp.status().is_success() {
            continue;
        }
        let text = resp.text().await.unwrap_or_default();
        let segs = parse_any(&text);
        if !segs.is_empty() {
            return Ok(segs);
        }
    }
    Err(anyhow!("the Teams transcript could not be read"))
}

pub async fn download_audio(
    url: &str,
    dest: &Path,
    progress: ProgressFn,
    progress_id: &str,
) -> anyhow::Result<()> {
    if url.to_ascii_lowercase().contains("tempauth") && url.contains("videomanifest") {
        return Err(anyhow!(
            "This manifest URL is signed with tempauth and expires in minutes; copy the videomanifest request again from the player"
        ));
    }
    let ffmpeg = crate::core::dependencies::ensure_ffmpeg().await?;
    let run = |copy: bool| {
        let mut cmd = crate::core::process::command(&ffmpeg);
        cmd.args(["-y", "-hide_banner", "-loglevel", "error", "-nostats", "-progress", "pipe:1"])
            .args(["-user_agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36"])
            .arg("-i")
            .arg(url)
            .args(["-vn", "-map", "0:a:0?"]);
        if copy {
            cmd.args(["-c:a", "copy"]);
        } else {
            cmd.args(["-c:a", "aac", "-b:a", "96k"]);
        }
        cmd.arg(dest)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        cmd
    };
    for copy in [true, false] {
        let mut child = run(copy).spawn().context("could not start ffmpeg")?;
        let stdout = child.stdout.take();
        let p = progress.clone();
        let id = progress_id.to_string();
        let reader = tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, BufReader};
            if let Some(out) = stdout {
                let mut lines = BufReader::new(out).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if let Some(us) = line.strip_prefix("out_time_us=") {
                        if let Ok(us) = us.trim().parse::<u64>() {
                            report(&p, &id, "downloading", us / 1_000_000, None, None);
                        }
                    }
                }
            }
        });
        let output = child.wait_with_output().await?;
        let _ = reader.await;
        if output.status.success()
            && std::fs::metadata(dest)
                .map(|m| m.len() > 0)
                .unwrap_or(false)
        {
            return Ok(());
        }
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if !copy {
            let hint = if err.contains("401") || err.contains("403") {
                " (the link expired or needs a signed-in session)"
            } else {
                ""
            };
            return Err(anyhow!("ffmpeg could not pull the recording{hint}: {err}"));
        }
    }
    unreachable!()
}
