//! Camada comum das ferramentas de Twitch: o GraphQL público que o próprio
//! site usa (Client-ID web, sem login, sem token de app) e o parsing dos
//! links que o usuário cola.
//!
//! Nada aqui exige conta: o Client-ID abaixo é o do player web, o mesmo que
//! `platforms/twitch` já usa para clipes.

use anyhow::{anyhow, bail};
use serde::Serialize;
use serde_json::{json, Value};

pub const GQL_URL: &str = "https://gql.twitch.tv/gql";
/// Client-ID público do player web da Twitch (o mesmo de `platforms/twitch`).
pub const CLIENT_ID: &str = "kimne78kx3ncx6brgo4mv6wki5h1ko";
/// Hash da persisted query `VideoCommentsByOffsetOrCursor` (replay de chat).
pub const COMMENTS_HASH: &str = "b70a3591ff0f4e0313d126c6a1502d79a1c02baebb288227c582044aa76adf6a";

/// O que o usuário colou quando pede o chat: um VOD ou um clipe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatTarget {
    Video(String),
    Clip(String),
}

/// Canal do link ou do texto: `xqc`, `@xqc`, `twitch.tv/xqc`,
/// `www.twitch.tv/xqc/videos`, `m.twitch.tv/xqc`.
pub fn parse_channel(input: &str) -> Option<String> {
    let raw = input.trim().trim_start_matches('@');
    if raw.is_empty() {
        return None;
    }
    let login = if raw.contains("twitch.tv") {
        let with_scheme = if raw.starts_with("http") {
            raw.to_string()
        } else {
            format!("https://{}", raw)
        };
        let url = url::Url::parse(&with_scheme).ok()?;
        let host = url.host_str()?.to_lowercase();
        if !host.ends_with("twitch.tv") {
            return None;
        }
        let seg = url
            .path_segments()?
            .find(|s| !s.is_empty() && !matches!(*s, "videos" | "clips" | "about"))?;
        seg.to_string()
    } else {
        raw.to_string()
    };
    let login = login.trim().to_lowercase();
    let ok = !login.is_empty()
        && login.len() <= 32
        && login.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if ok {
        Some(login)
    } else {
        None
    }
}

/// VOD ou clipe a partir do link: `twitch.tv/videos/123`, `123`, `v123`,
/// `twitch.tv/xqc/video/123`, `clips.twitch.tv/<slug>`,
/// `twitch.tv/xqc/clip/<slug>`.
pub fn parse_video(input: &str) -> Option<ChatTarget> {
    let raw = input.trim();
    if raw.is_empty() {
        return None;
    }
    let bare = raw.trim_start_matches('v');
    if !bare.is_empty() && bare.chars().all(|c| c.is_ascii_digit()) {
        return Some(ChatTarget::Video(bare.to_string()));
    }
    if !raw.contains("twitch.tv") {
        return None;
    }
    let with_scheme = if raw.starts_with("http") {
        raw.to_string()
    } else {
        format!("https://{}", raw)
    };
    let url = url::Url::parse(&with_scheme).ok()?;
    let host = url.host_str()?.to_lowercase();
    if !host.ends_with("twitch.tv") {
        return None;
    }
    let segs: Vec<&str> = url.path_segments()?.filter(|s| !s.is_empty()).collect();
    if host == "clips.twitch.tv" || host.ends_with(".clips.twitch.tv") {
        return segs.first().map(|s| ChatTarget::Clip(s.to_string()));
    }
    if let Some(i) = segs.iter().position(|s| *s == "clip") {
        return segs.get(i + 1).map(|s| ChatTarget::Clip(s.to_string()));
    }
    if let Some(i) = segs.iter().position(|s| *s == "videos" || *s == "video") {
        let id = segs.get(i + 1)?;
        if id.chars().all(|c| c.is_ascii_digit()) {
            return Some(ChatTarget::Video(id.to_string()));
        }
    }
    None
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Channel {
    pub id: String,
    pub login: String,
    pub display_name: String,
    pub avatar: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct VideoInfo {
    pub id: String,
    pub title: String,
    pub channel: String,
    pub channel_display: String,
    pub duration_seconds: f64,
    pub created_at: String,
    /// Preenchido quando o alvo era um clipe: janela dentro do VOD de origem.
    pub clip_offset: Option<f64>,
    pub clip_duration: Option<f64>,
}

/// Cliente do GraphQL público, com repetição em 429/5xx.
pub struct Gql {
    pub http: reqwest::Client,
}

impl Gql {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            http: super::super::client()?,
        })
    }

    async fn post(&self, body: &Value) -> anyhow::Result<Value> {
        let mut wait = std::time::Duration::from_millis(800);
        let mut last = String::new();
        for attempt in 0..5 {
            if attempt > 0 {
                tokio::time::sleep(wait).await;
                wait *= 2;
            }
            let sent = self
                .http
                .post(GQL_URL)
                .header("Client-ID", CLIENT_ID)
                .header("Origin", "https://www.twitch.tv")
                .header("Referer", "https://www.twitch.tv/")
                .json(body)
                .send()
                .await;
            let resp = match sent {
                Ok(r) => r,
                Err(e) => {
                    last = e.to_string();
                    continue;
                }
            };
            let status = resp.status();
            if status.as_u16() == 429 || status.is_server_error() {
                last = format!("HTTP {}", status);
                continue;
            }
            if !status.is_success() {
                bail!("Twitch GQL respondeu HTTP {}", status);
            }
            return Ok(resp.json::<Value>().await?);
        }
        bail!("Twitch GQL não respondeu depois de 5 tentativas: {}", last)
    }

    /// Query anônima em texto (as públicas aceitam sem persisted query).
    pub async fn query(&self, query: &str) -> anyhow::Result<Value> {
        let body = json!({ "query": query });
        let json = self.post(&body).await?;
        if let Some(msg) = first_error(&json) {
            bail!("Twitch GQL: {}", msg);
        }
        json.get("data")
            .cloned()
            .ok_or_else(|| anyhow!("resposta do Twitch GQL sem `data`"))
    }

    /// Persisted query (formato em lote, como o site manda). Devolve o
    /// primeiro elemento cru, para quem quiser ler `errors` também.
    pub async fn persisted(&self, op: &str, hash: &str, vars: Value) -> anyhow::Result<Value> {
        let body = json!([{
            "operationName": op,
            "variables": vars,
            "extensions": { "persistedQuery": { "version": 1, "sha256Hash": hash } },
        }]);
        let json = self.post(&body).await?;
        json.as_array()
            .and_then(|a| a.first())
            .cloned()
            .ok_or_else(|| anyhow!("resposta em lote inesperada do Twitch GQL"))
    }

    pub async fn channel(&self, login: &str) -> anyhow::Result<Channel> {
        let q = format!(
            r#"{{ user(login: "{}") {{ id login displayName profileImageURL(width: 300) }} }}"#,
            escape(login)
        );
        let data = self.query(&q).await?;
        let u = data.get("user").filter(|v| !v.is_null());
        let u = u.ok_or_else(|| anyhow!("canal não encontrado: {}", login))?;
        Ok(Channel {
            id: str_at(u, "id"),
            login: str_at(u, "login"),
            display_name: str_at(u, "displayName"),
            avatar: str_at(u, "profileImageURL"),
        })
    }

    pub async fn video(&self, id: &str) -> anyhow::Result<VideoInfo> {
        let q = format!(
            r#"{{ video(id: "{}") {{ id title lengthSeconds createdAt owner {{ login displayName }} }} }}"#,
            escape(id)
        );
        let data = self.query(&q).await?;
        let v = data.get("video").filter(|v| !v.is_null());
        let v = v.ok_or_else(|| anyhow!("VOD não encontrado (ou já expirou): {}", id))?;
        Ok(VideoInfo {
            id: str_at(v, "id"),
            title: str_at(v, "title"),
            channel: v
                .pointer("/owner/login")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string(),
            channel_display: v
                .pointer("/owner/displayName")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string(),
            duration_seconds: v.get("lengthSeconds").and_then(num).unwrap_or_default(),
            created_at: str_at(v, "createdAt"),
            clip_offset: None,
            clip_duration: None,
        })
    }

    /// Clipe → VOD de origem, com a janela do clipe dentro dele.
    pub async fn clip_video(&self, slug: &str) -> anyhow::Result<VideoInfo> {
        let q = format!(
            r#"{{ clip(slug: "{}") {{ durationSeconds videoOffsetSeconds video {{ id }} }} }}"#,
            escape(slug)
        );
        let data = self.query(&q).await?;
        let c = data.get("clip").filter(|v| !v.is_null());
        let c = c.ok_or_else(|| anyhow!("clipe não encontrado: {}", slug))?;
        let vid = c
            .pointer("/video/id")
            .and_then(|x| x.as_str())
            .ok_or_else(|| anyhow!("esse clipe não tem VOD de origem, então não há chat"))?;
        let mut info = self.video(vid).await?;
        info.clip_offset = c.get("videoOffsetSeconds").and_then(num);
        info.clip_duration = c.get("durationSeconds").and_then(num);
        Ok(info)
    }

    /// Resolve o que o usuário colou até chegar num VOD.
    pub async fn resolve_chat_target(&self, input: &str) -> anyhow::Result<VideoInfo> {
        let target = parse_video(input)
            .ok_or_else(|| anyhow!("cole o link de um VOD ou de um clipe da Twitch: {}", input))?;
        match target {
            ChatTarget::Video(id) => self.video(&id).await,
            ChatTarget::Clip(slug) => self.clip_video(&slug).await,
        }
    }
}

fn num(v: &Value) -> Option<f64> {
    v.as_f64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

pub(super) fn str_at(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_string()
}

/// Escapa o que vai dentro de aspas na query (login e id são simples, mas
/// não custa não deixar o usuário quebrar a query).
fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

pub(super) fn first_error(v: &Value) -> Option<String> {
    v.get("errors")
        .and_then(|e| e.as_array())
        .and_then(|a| a.first())
        .map(|e| {
            e.get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("erro sem mensagem")
                .to_string()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_canal_de_varias_formas() {
        for input in [
            "xqc",
            "@xqc",
            "XQC",
            "twitch.tv/xqc",
            "https://www.twitch.tv/xqc",
            "https://www.twitch.tv/xqc/videos",
            "https://m.twitch.tv/xqc?tt_content=x",
        ] {
            assert_eq!(parse_channel(input).as_deref(), Some("xqc"), "{}", input);
        }
        assert_eq!(parse_channel(""), None);
        assert_eq!(parse_channel("https://youtube.com/@xqc"), None);
        assert_eq!(parse_channel("nome com espaço"), None);
    }

    #[test]
    fn le_vod_e_clipe() {
        assert_eq!(
            parse_video("https://www.twitch.tv/videos/2869633607"),
            Some(ChatTarget::Video("2869633607".into()))
        );
        assert_eq!(
            parse_video("https://www.twitch.tv/xqc/video/123?t=1h2m3s"),
            Some(ChatTarget::Video("123".into()))
        );
        assert_eq!(
            parse_video("v2869633607"),
            Some(ChatTarget::Video("2869633607".into()))
        );
        assert_eq!(
            parse_video("2869633607"),
            Some(ChatTarget::Video("2869633607".into()))
        );
        assert_eq!(
            parse_video("https://clips.twitch.tv/SplendidResilientBeeOSfrog-mAG555tKEq6wC9lj"),
            Some(ChatTarget::Clip(
                "SplendidResilientBeeOSfrog-mAG555tKEq6wC9lj".into()
            ))
        );
        assert_eq!(
            parse_video("https://www.twitch.tv/xqc/clip/AbcDef-123"),
            Some(ChatTarget::Clip("AbcDef-123".into()))
        );
        assert_eq!(parse_video("https://www.twitch.tv/xqc"), None);
        assert_eq!(parse_video(""), None);
    }

    #[test]
    fn escapa_aspas_da_query() {
        assert_eq!(escape(r#"a"b"#), r#"a\"b"#);
    }

    #[test]
    fn le_primeiro_erro_do_graphql() {
        let v = serde_json::json!({ "errors": [{ "message": "failed integrity check" }] });
        assert_eq!(first_error(&v).as_deref(), Some("failed integrity check"));
        assert_eq!(first_error(&serde_json::json!({ "data": {} })), None);
    }
}
