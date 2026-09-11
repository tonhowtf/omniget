//! Baixar mídia de um post do Reddit.
//!
//! O problema real do Reddit é o v.redd.it: vídeo e áudio são duas faixas
//! separadas, e quase todo site de download entrega o vídeo mudo. Aqui quem
//! baixa é o yt-dlp já gerido pelo app, que junta as duas com o ffmpeg.
//!
//! Cada tipo de post tem um caminho: vídeo vai no yt-dlp, galeria sai direto
//! do `media_metadata` (o original em i.redd.it, sem passar por preview) com
//! o gallery-dl de reserva, imagem única é um GET, e link de fora
//! (redgifs, imgur, YouTube…) volta para o yt-dlp.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{fmt_utc, parse_target, Fetcher, Target};
use crate::core::tools::{report, sanitize_name, ProgressFn};

const ID: &str = "rd-download";

// ───────────────────────── entrada e saída ─────────────────────────

fn def_true() -> bool {
    true
}
fn def_info() -> String {
    "json".to_string()
}
fn def_delay() -> u64 {
    900
}

#[derive(Debug, Clone, Deserialize)]
pub struct Options {
    pub url: String,
    pub dest: String,
    /// Grava um arquivo ao lado com título, autor, sub e data.
    #[serde(default = "def_true")]
    pub write_info: bool,
    /// "json", "txt" ou "both".
    #[serde(default = "def_info")]
    pub info_format: String,
    /// Só a trilha de áudio (mp3), quando o post é vídeo.
    #[serde(default)]
    pub audio_only: bool,
    /// Caminho de um cookies.txt, para post de sub restrito.
    #[serde(default)]
    pub cookies: Option<String>,
    #[serde(default = "def_delay")]
    pub delay_ms: u64,
    /// Conta do gerenciador de cookies a usar (None = `_default`). Com a
    /// sessão da extensão o Reddit responde como responde ao navegador do
    /// usuário; sem ela, cai no modo anônimo com o desafio de JavaScript.
    #[serde(default)]
    pub account_slug: Option<String>,
    /// Conteúdo Netscape do bucket `reddit.com`. Quem preenche é o comando
    /// Tauri, nunca o front — daí o `skip`.
    #[serde(skip)]
    pub session_netscape: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct PostInfo {
    pub id: String,
    pub title: String,
    pub author: String,
    pub subreddit: String,
    pub created: String,
    pub created_utc: f64,
    pub score: i64,
    pub upvote_ratio: f64,
    pub num_comments: i64,
    pub permalink: String,
    pub url: String,
    pub domain: String,
    pub flair: String,
    pub over_18: bool,
    pub selftext: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DownloadResult {
    /// "video" | "gallery" | "image" | "external" | "text"
    pub kind: String,
    /// "yt-dlp" | "gallery-dl" | "direto" | "nenhum"
    pub source: String,
    pub files: Vec<String>,
    pub dest: String,
    pub info: PostInfo,
    pub log_tail: String,
    /// Se a leitura do post saiu com a sessão do usuário ou anônima.
    pub used_session: bool,
}

// ───────────────────────── decisão ─────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// v.redd.it e afins: o yt-dlp junta vídeo e áudio.
    Video { url: String },
    /// Galeria: uma lista de originais em i.redd.it.
    Gallery { urls: Vec<String> },
    /// Imagem única.
    Image { url: String },
    /// Link de fora do Reddit: o yt-dlp que resolva.
    External { url: String },
    /// Post só de texto: não há o que baixar.
    TextOnly,
}

fn s(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string()
}

fn is_image_url(url: &str) -> bool {
    let path = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .to_ascii_lowercase();
    ["jpg", "jpeg", "png", "gif", "webp", "bmp"]
        .iter()
        .any(|e| path.ends_with(&format!(".{}", e)))
}

fn ext_from_mime(mime: &str) -> &str {
    match mime.rsplit('/').next().unwrap_or("") {
        "jpg" | "jpeg" | "pjpg" => "jpg",
        "png" => "png",
        "gif" => "gif",
        "webp" => "webp",
        "mp4" => "mp4",
        _ => "jpg",
    }
}

/// Os originais de uma galeria. A ordem vem do `gallery_data`; o formato, do
/// `media_metadata`. `i.redd.it/<id>.<ext>` é o arquivo cru — a URL do
/// `preview.redd.it` que o JSON traz é uma reamostragem.
pub fn gallery_urls(post: &Value) -> Vec<String> {
    let meta = post.get("media_metadata").and_then(|m| m.as_object());
    let order: Vec<String> = post
        .pointer("/gallery_data/items")
        .and_then(|i| i.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.get("media_id").and_then(|m| m.as_str()))
                .map(|x| x.to_string())
                .collect()
        })
        .unwrap_or_default();
    let Some(meta) = meta else {
        return Vec::new();
    };
    let ids: Vec<String> = if order.is_empty() {
        meta.keys().cloned().collect()
    } else {
        order
    };
    let mut out = Vec::new();
    for id in ids {
        let Some(item) = meta.get(&id) else { continue };
        if s(item, "status") != "valid" && item.get("status").is_some() {
            continue;
        }
        let mime = s(item, "m");
        let ext = ext_from_mime(&mime);
        if s(item, "e") == "AnimatedImage" {
            if let Some(mp4) = item.pointer("/s/mp4").and_then(|x| x.as_str()) {
                out.push(mp4.replace("&amp;", "&"));
                continue;
            }
        }
        if mime.is_empty() {
            if let Some(u) = item.pointer("/s/u").and_then(|x| x.as_str()) {
                out.push(u.replace("&amp;", "&"));
            }
            continue;
        }
        out.push(format!("https://i.redd.it/{}.{}", id, ext));
    }
    out
}

/// Olha o post e diz por onde baixar.
pub fn plan_for(post: &Value) -> Plan {
    // Crosspost: o que interessa está no post original.
    if let Some(orig) = post
        .pointer("/crosspost_parent_list/0")
        .filter(|v| v.is_object())
    {
        let inner = plan_for(orig);
        if inner != Plan::TextOnly {
            return inner;
        }
    }
    let url = {
        let over = s(post, "url_overridden_by_dest");
        if over.is_empty() {
            s(post, "url")
        } else {
            over
        }
    };
    let domain = s(post, "domain");
    let hint = s(post, "post_hint");

    let gallery = gallery_urls(post);
    if !gallery.is_empty() {
        return Plan::Gallery { urls: gallery };
    }
    if post
        .get("is_video")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || post.pointer("/secure_media/reddit_video").is_some()
        || post.pointer("/media/reddit_video").is_some()
        || domain == "v.redd.it"
        || url.contains("v.redd.it")
    {
        let permalink = s(post, "permalink");
        return Plan::Video {
            url: if permalink.is_empty() {
                url
            } else {
                format!("https://www.reddit.com{}", permalink)
            },
        };
    }
    if !url.is_empty() && (domain == "i.redd.it" || hint == "image" || is_image_url(&url)) {
        return Plan::Image { url };
    }
    if post
        .get("is_self")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || url.is_empty()
    {
        return Plan::TextOnly;
    }
    if url.contains("reddit.com/") && url.contains("/comments/") {
        return Plan::TextOnly;
    }
    Plan::External { url }
}

pub fn post_info(post: &Value) -> PostInfo {
    let created_utc = post
        .get("created_utc")
        .and_then(|x| x.as_f64())
        .unwrap_or(0.0);
    PostInfo {
        id: s(post, "id"),
        title: s(post, "title"),
        author: s(post, "author"),
        subreddit: s(post, "subreddit"),
        created: fmt_utc(created_utc),
        created_utc,
        score: post.get("score").and_then(|x| x.as_i64()).unwrap_or(0),
        upvote_ratio: post
            .get("upvote_ratio")
            .and_then(|x| x.as_f64())
            .unwrap_or(0.0),
        num_comments: post
            .get("num_comments")
            .and_then(|x| x.as_i64())
            .unwrap_or(0),
        permalink: format!("https://www.reddit.com{}", s(post, "permalink")),
        url: s(post, "url"),
        domain: s(post, "domain"),
        flair: s(post, "link_flair_text"),
        over_18: post
            .get("over_18")
            .and_then(|x| x.as_bool())
            .unwrap_or(false),
        selftext: s(post, "selftext"),
    }
}

pub fn info_txt(info: &PostInfo) -> String {
    let mut out = String::new();
    out.push_str(&format!("Título: {}\n", info.title));
    out.push_str(&format!("Autor: u/{}\n", info.author));
    out.push_str(&format!("Sub: r/{}\n", info.subreddit));
    out.push_str(&format!("Data: {}\n", info.created));
    out.push_str(&format!(
        "Pontos: {} ({}% positivo)\n",
        info.score,
        (info.upvote_ratio * 100.0).round() as i64
    ));
    out.push_str(&format!("Comentários: {}\n", info.num_comments));
    if !info.flair.is_empty() {
        out.push_str(&format!("Flair: {}\n", info.flair));
    }
    if info.over_18 {
        out.push_str("Marcado como NSFW\n");
    }
    out.push_str(&format!("Link: {}\n", info.permalink));
    if !info.url.is_empty() && info.url != info.permalink {
        out.push_str(&format!("Destino: {}\n", info.url));
    }
    if !info.selftext.trim().is_empty() {
        out.push('\n');
        out.push_str(info.selftext.trim());
        out.push('\n');
    }
    out
}

/// Nome-base dos arquivos: título curto com o id do post no fim, para dois
/// posts de mesmo título não brigarem.
pub fn base_name(info: &PostInfo) -> String {
    let title = if info.title.trim().is_empty() {
        "post".to_string()
    } else {
        info.title.trim().to_string()
    };
    let short: String = title.chars().take(90).collect();
    let name = sanitize_name(&format!("{} [{}]", short.trim(), info.id));
    name.trim().to_string()
}

// ───────────────────────── yt-dlp ─────────────────────────

const MARK: &str = "OMNIGET_FILEPATH:";

/// Argumentos do yt-dlp para um post. Separado para o teste conferir sem
/// precisar do binário.
pub fn ytdlp_args(
    url: &str,
    dest: &Path,
    base: &str,
    audio_only: bool,
    ffmpeg: Option<&Path>,
    cookies: Option<&str>,
) -> Vec<String> {
    let template = dest.join(format!("{}.%(ext)s", base));
    let mut args: Vec<String> = vec![
        "--ignore-config".to_string(),
        "--no-playlist".to_string(),
        "--newline".to_string(),
        "--no-quiet".to_string(),
        "--progress".to_string(),
        "--encoding".to_string(),
        "utf-8".to_string(),
        "--no-warnings".to_string(),
        "-o".to_string(),
        template.to_string_lossy().to_string(),
        "--print".to_string(),
        format!("after_move:{}%(filepath)s", MARK),
    ];
    if audio_only {
        args.push("-x".to_string());
        args.push("--audio-format".to_string());
        args.push("mp3".to_string());
    } else {
        // O v.redd.it entrega vídeo e áudio separados; sem isto sai mudo.
        args.push("-f".to_string());
        args.push("bv*+ba/b".to_string());
        args.push("--merge-output-format".to_string());
        args.push("mp4".to_string());
    }
    if let Some(ff) = ffmpeg {
        args.push("--ffmpeg-location".to_string());
        args.push(ff.to_string_lossy().to_string());
    }
    if let Some(c) = cookies.filter(|c| !c.trim().is_empty()) {
        args.push("--cookies".to_string());
        args.push(c.to_string());
    }
    args.push(url.to_string());
    args
}

/// Percentual de uma linha `[download]  12.3% of ...`.
pub fn parse_progress(line: &str) -> Option<f64> {
    let l = line.trim();
    if !l.starts_with("[download]") {
        return None;
    }
    let pct = l.split_whitespace().find(|w| w.ends_with('%'))?;
    pct.trim_end_matches('%').parse::<f64>().ok()
}

async fn run_ytdlp(
    url: &str,
    dest: &Path,
    base: &str,
    opts: &Options,
    progress: &ProgressFn,
) -> Result<(Vec<String>, String)> {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let bin = crate::core::ytdlp::ensure_ytdlp()
        .await
        .map_err(|e| anyhow!("o yt-dlp não está disponível: {}", e))?;
    let ffmpeg = crate::core::dependencies::find_tool("ffmpeg").await;
    let args = ytdlp_args(
        url,
        dest,
        base,
        opts.audio_only,
        ffmpeg.as_deref(),
        opts.cookies.as_deref(),
    );
    let mut cmd = crate::core::ytdlp::ytdlp_command(&bin);
    cmd.args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| anyhow!("não foi possível iniciar o yt-dlp: {}", e))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let p = progress.clone();
    let out_task = tokio::spawn(async move {
        let mut files: Vec<String> = Vec::new();
        if let Some(o) = stdout {
            let mut lines = BufReader::new(o).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Some(path) = line.trim().strip_prefix(MARK) {
                    if !path.is_empty() && !files.iter().any(|f| f == path) {
                        files.push(path.to_string());
                    }
                    continue;
                }
                if let Some(pct) = parse_progress(&line) {
                    report(&p, ID, "progress", pct.round() as u64, Some(100), None);
                }
            }
        }
        files
    });
    let err_task = tokio::spawn(async move {
        let mut tail = Vec::new();
        if let Some(e) = stderr {
            let mut lines = BufReader::new(e).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tail.push(line);
                if tail.len() > 8 {
                    tail.remove(0);
                }
            }
        }
        tail.join("\n")
    });
    let status = child.wait().await?;
    let files = out_task.await.unwrap_or_default();
    let tail = err_task.await.unwrap_or_default();
    if files.is_empty() {
        return Err(anyhow!(
            "o yt-dlp não baixou nada{}{}",
            if status.success() { "" } else { " (falhou)" },
            if tail.is_empty() {
                String::new()
            } else {
                format!(": {}", tail)
            }
        ));
    }
    Ok((files, tail))
}

// ───────────────────────── execução ─────────────────────────

async fn fetch_post(fetcher: &Fetcher, id: &str) -> Result<Value> {
    let v = fetcher
        .get_json(&super::post_json_url(id, "top", 1))
        .await?;
    v.pointer("/0/data/children/0/data")
        .cloned()
        .ok_or_else(|| anyhow!("o post não veio na resposta (apagado, privado ou id errado)"))
}

async fn download_direct(
    fetcher: &Fetcher,
    urls: &[String],
    dest: &Path,
    base: &str,
    progress: &ProgressFn,
) -> Result<Vec<String>> {
    let mut files = Vec::new();
    let single = urls.len() == 1;
    for (idx, url) in urls.iter().enumerate() {
        let ext = url
            .split(['?', '#'])
            .next()
            .unwrap_or(url)
            .rsplit('.')
            .next()
            .filter(|e| e.len() <= 5 && e.chars().all(|c| c.is_ascii_alphanumeric()))
            .unwrap_or("jpg")
            .to_ascii_lowercase();
        let name = if single {
            format!("{}.{}", base, ext)
        } else {
            format!("{} {:02}.{}", base, idx + 1, ext)
        };
        let path = dest.join(name);
        report(
            progress,
            ID,
            "progress",
            idx as u64,
            Some(urls.len() as u64),
            Some(url.clone()),
        );
        crate::core::tools::download_to(fetcher.client(), url, &path, progress, ID).await?;
        files.push(path.to_string_lossy().to_string());
    }
    Ok(files)
}

/// Reserva da galeria: quando o `media_metadata` não serve, o gallery-dl
/// resolve o post inteiro.
async fn download_gallery_dl(
    url: &str,
    dest: &Path,
    progress: &ProgressFn,
) -> Result<(Vec<String>, String)> {
    let r =
        crate::core::tools::gallery::download(url, &dest.to_string_lossy(), None, progress.clone())
            .await?;
    Ok((r.files, r.log_tail))
}

pub async fn run(opts: &Options, progress: ProgressFn) -> Result<DownloadResult> {
    report(&progress, ID, "started", 0, None, None);
    let dest = PathBuf::from(&opts.dest);
    std::fs::create_dir_all(&dest)?;
    let fetcher = Fetcher::new(opts.delay_ms, opts.session_netscape.as_deref())?;

    let mut target = parse_target(&opts.url).ok_or_else(|| {
        anyhow!("cole o link de um post do Reddit, de um i.redd.it ou de um v.redd.it")
    })?;
    if let Target::Short { url } = &target {
        let final_url = fetcher.resolve(url).await?;
        target = parse_target(&final_url)
            .ok_or_else(|| anyhow!("o link curto não levou a um post ({})", final_url))?;
    }

    // Mídia solta e link de fora não têm post para consultar.
    let (post, plan) = match &target {
        Target::Post { id, .. } => {
            let post = fetch_post(&fetcher, id).await?;
            let plan = plan_for(&post);
            (Some(post), plan)
        }
        Target::Media { url, video: true } => (None, Plan::Video { url: url.clone() }),
        Target::Media { url, video: false } => (None, Plan::Image { url: url.clone() }),
        Target::External { url } => (None, Plan::External { url: url.clone() }),
        Target::Subreddit { .. } | Target::User { .. } => {
            return Err(anyhow!(
                "isto é um sub ou um perfil: cole o link de um post"
            ))
        }
        Target::Short { .. } => return Err(anyhow!("não foi possível resolver o link curto")),
    };

    let info = post.as_ref().map(post_info).unwrap_or_else(|| PostInfo {
        id: String::new(),
        title: opts
            .url
            .rsplit('/')
            .find(|s| !s.is_empty())
            .unwrap_or("reddit")
            .split('.')
            .next()
            .unwrap_or("reddit")
            .to_string(),
        ..PostInfo::default()
    });
    let base = base_name(&info);

    let (kind, source, mut files, log_tail) = match &plan {
        Plan::Video { url } => {
            let (f, tail) = run_ytdlp(url, &dest, &base, opts, &progress).await?;
            ("video", "yt-dlp", f, tail)
        }
        Plan::External { url } => {
            let (f, tail) = run_ytdlp(url, &dest, &base, opts, &progress).await?;
            ("external", "yt-dlp", f, tail)
        }
        Plan::Image { url } => {
            let f = download_direct(&fetcher, std::slice::from_ref(url), &dest, &base, &progress)
                .await?;
            ("image", "direto", f, String::new())
        }
        Plan::Gallery { urls } => {
            match download_direct(&fetcher, urls, &dest, &base, &progress).await {
                Ok(f) => ("gallery", "direto", f, String::new()),
                Err(e) => {
                    let (f, tail) = download_gallery_dl(&info.permalink, &dest, &progress)
                        .await
                        .map_err(|g| anyhow!("galeria falhou ({}); gallery-dl: {}", e, g))?;
                    ("gallery", "gallery-dl", f, tail)
                }
            }
        }
        Plan::TextOnly => ("text", "nenhum", Vec::new(), String::new()),
    };

    if opts.write_info && post.is_some() {
        let want_json = opts.info_format != "txt";
        let want_txt = opts.info_format == "txt" || opts.info_format == "both";
        if want_json {
            let path = dest.join(format!("{}.json", base));
            std::fs::write(&path, serde_json::to_string_pretty(&info)?)?;
            files.push(path.to_string_lossy().to_string());
        }
        if want_txt {
            let path = dest.join(format!("{}.txt", base));
            std::fs::write(&path, info_txt(&info))?;
            files.push(path.to_string_lossy().to_string());
        }
    }
    if files.is_empty() {
        return Err(anyhow!("este post não tem mídia para baixar (é só texto)"));
    }
    report(
        &progress,
        ID,
        "done",
        files.len() as u64,
        Some(files.len() as u64),
        None,
    );
    Ok(DownloadResult {
        kind: kind.to_string(),
        source: source.to_string(),
        files,
        dest: dest.to_string_lossy().to_string(),
        info,
        log_tail,
        used_session: fetcher.has_session(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn post(raw: &str) -> Value {
        serde_json::from_str(raw).expect("json de teste")
    }

    #[test]
    fn video_do_vreddit_vai_pelo_permalink() {
        let p = post(
            r#"{"id":"aa1","is_video":true,"domain":"v.redd.it","url":"https://v.redd.it/xyz",
                "permalink":"/r/videos/comments/aa1/titulo/","secure_media":{"reddit_video":{"fallback_url":"https://v.redd.it/xyz/DASH_720.mp4"}}}"#,
        );
        assert_eq!(
            plan_for(&p),
            Plan::Video {
                url: "https://www.reddit.com/r/videos/comments/aa1/titulo/".to_string()
            }
        );
    }

    #[test]
    fn galeria_sai_do_media_metadata_em_ordem() {
        let p = post(
            r#"{"id":"aa2","is_gallery":true,"url":"https://www.reddit.com/gallery/aa2",
                "gallery_data":{"items":[{"media_id":"bbb","id":1},{"media_id":"aaa","id":2}]},
                "media_metadata":{
                  "aaa":{"status":"valid","e":"Image","m":"image/png","s":{"u":"https://preview.redd.it/aaa.png?width=1"}},
                  "bbb":{"status":"valid","e":"Image","m":"image/jpg","s":{"u":"https://preview.redd.it/bbb.jpg?width=1"}}
                }}"#,
        );
        assert_eq!(
            plan_for(&p),
            Plan::Gallery {
                urls: vec![
                    "https://i.redd.it/bbb.jpg".to_string(),
                    "https://i.redd.it/aaa.png".to_string(),
                ]
            }
        );
    }

    #[test]
    fn gif_animado_da_galeria_vira_mp4() {
        let p = post(
            r#"{"media_metadata":{"ggg":{"status":"valid","e":"AnimatedImage","m":"image/gif",
                "s":{"mp4":"https://preview.redd.it/ggg.mp4?s=1&amp;t=2"}}}}"#,
        );
        assert_eq!(
            gallery_urls(&p),
            vec!["https://preview.redd.it/ggg.mp4?s=1&t=2".to_string()]
        );
    }

    #[test]
    fn imagem_unica_e_link_externo() {
        let img = post(
            r#"{"id":"aa3","domain":"i.redd.it","post_hint":"image","url":"https://i.redd.it/abc.jpg","permalink":"/r/pics/comments/aa3/x/"}"#,
        );
        assert_eq!(
            plan_for(&img),
            Plan::Image {
                url: "https://i.redd.it/abc.jpg".to_string()
            }
        );
        let ext = post(
            r#"{"id":"aa4","domain":"redgifs.com","post_hint":"rich:video","url":"https://redgifs.com/watch/abc","permalink":"/r/x/comments/aa4/y/"}"#,
        );
        assert_eq!(
            plan_for(&ext),
            Plan::External {
                url: "https://redgifs.com/watch/abc".to_string()
            }
        );
    }

    #[test]
    fn post_de_texto_nao_tem_o_que_baixar() {
        let p = post(
            r#"{"id":"aa5","is_self":true,"url":"https://www.reddit.com/r/x/comments/aa5/y/","selftext":"oi","permalink":"/r/x/comments/aa5/y/"}"#,
        );
        assert_eq!(plan_for(&p), Plan::TextOnly);
    }

    #[test]
    fn crosspost_usa_o_original() {
        let p = post(
            r#"{"id":"aa6","is_self":false,"url":"https://www.reddit.com/r/a/comments/zz/x/","permalink":"/r/b/comments/aa6/y/",
                "crosspost_parent_list":[{"id":"zz","is_video":true,"domain":"v.redd.it","url":"https://v.redd.it/q","permalink":"/r/a/comments/zz/x/"}]}"#,
        );
        assert_eq!(
            plan_for(&p),
            Plan::Video {
                url: "https://www.reddit.com/r/a/comments/zz/x/".to_string()
            }
        );
    }

    #[test]
    fn monta_os_argumentos_do_ytdlp() {
        let dest = Path::new("/tmp/saida");
        let args = ytdlp_args(
            "https://www.reddit.com/r/x/comments/aa1/t/",
            dest,
            "Titulo [aa1]",
            false,
            Some(Path::new("/opt/ffmpeg")),
            None,
        );
        let joined = args.join(" ");
        assert!(joined.contains("--merge-output-format mp4"));
        assert!(joined.contains("bv*+ba/b"));
        assert!(joined.contains("--ffmpeg-location /opt/ffmpeg"));
        assert!(joined.contains("after_move:OMNIGET_FILEPATH:%(filepath)s"));
        let template = dest.join("Titulo [aa1].%(ext)s");
        assert!(args.contains(&template.to_string_lossy().to_string()));
        assert_eq!(
            args.last().map(|s| s.as_str()),
            Some("https://www.reddit.com/r/x/comments/aa1/t/")
        );
        let audio = ytdlp_args("u", dest, "b", true, None, Some("/tmp/c.txt"));
        assert!(audio.join(" ").contains("-x --audio-format mp3"));
        assert!(audio.join(" ").contains("--cookies /tmp/c.txt"));
        assert!(!audio.join(" ").contains("--merge-output-format"));
    }

    #[test]
    fn le_a_porcentagem_do_ytdlp() {
        assert_eq!(
            parse_progress("[download]  12.3% of ~ 10.00MiB at 1.00MiB/s ETA 00:09"),
            Some(12.3)
        );
        assert_eq!(parse_progress("[download] 100% of 1.00MiB"), Some(100.0));
        assert_eq!(parse_progress("[Merger] Merging formats"), None);
    }

    #[test]
    fn info_em_texto_e_nome_base() {
        let p = post(
            r#"{"id":"aa7","title":"Um / titulo : com  simbolos","author":"ana","subreddit":"rust",
                "created_utc":1700000000,"score":10,"upvote_ratio":0.95,"num_comments":3,
                "permalink":"/r/rust/comments/aa7/x/","url":"https://v.redd.it/q","link_flair_text":"Info","over_18":false,"selftext":"corpo"}"#,
        );
        let info = post_info(&p);
        let txt = info_txt(&info);
        assert!(txt.contains("Autor: u/ana"));
        assert!(txt.contains("Sub: r/rust"));
        assert!(txt.contains("Data: 2023-11-14 22:13 UTC"));
        assert!(txt.contains("Pontos: 10 (95% positivo)"));
        assert!(txt.contains("corpo"));
        let base = base_name(&info);
        assert!(base.ends_with("[aa7]"));
        assert!(!base.contains('/'));
    }

    #[tokio::test]
    #[ignore = "rede + yt-dlp: baixa um video de verdade do v.redd.it"]
    async fn baixa_um_video_de_verdade() {
        let dir = std::env::temp_dir().join("omniget-rddl-live-video");
        let _ = std::fs::create_dir_all(&dir);
        let opts = Options {
            url: "https://www.reddit.com/r/aww/comments/1wbshkl/foster_kitten_playing_with_fire/"
                .to_string(),
            dest: dir.to_string_lossy().to_string(),
            write_info: true,
            info_format: "both".to_string(),
            audio_only: false,
            cookies: None,
            delay_ms: 900,
            account_slug: None,
            session_netscape: None,
        };
        let r = run(&opts, crate::core::tools::noop_progress())
            .await
            .expect("download real");
        println!("{} via {} -> {:?}", r.kind, r.source, r.files);
        assert_eq!(r.kind, "video");
        assert!(r.files.iter().any(|f| f.ends_with(".mp4")));
    }

    #[tokio::test]
    #[ignore = "rede: baixa uma galeria de verdade do i.redd.it"]
    async fn baixa_uma_galeria_de_verdade() {
        let dir = std::env::temp_dir().join("omniget-rddl-live-galeria");
        let _ = std::fs::create_dir_all(&dir);
        let opts = Options {
            url: "https://www.reddit.com/r/aww/comments/1wbksg3/".to_string(),
            dest: dir.to_string_lossy().to_string(),
            write_info: true,
            info_format: "json".to_string(),
            audio_only: false,
            cookies: None,
            delay_ms: 900,
            account_slug: None,
            session_netscape: None,
        };
        let r = run(&opts, crate::core::tools::noop_progress())
            .await
            .expect("galeria real");
        println!("{} via {} -> {} arquivos", r.kind, r.source, r.files.len());
        assert_eq!(r.kind, "gallery");
        assert!(r.files.len() > 1);
    }
}
