//! Vimeo: dois caminhos que o yt-dlp já sabe fazer, mas que a UI do OmniGet
//! ainda não expunha direito.
//!
//! 1. **Acesso privado / unlisted** — o link com hash (`vimeo.com/<id>/<hash>`)
//!    já basta por si; o vídeo com senha precisa de `--video-password`. Aqui a
//!    senha é do usuário, que já tem o link e a senha legitimamente: nada de
//!    tentativa e erro, nada de lista de senhas, nada de repetição automática.
//!    Uma senha errada volta como erro e para.
//! 2. **Backup de showcase / álbum / canal** — o `VimeoAlbumIE` do yt-dlp
//!    enumera a coleção; aqui a enumeração vem do `--flat-playlist -J` e o
//!    laço de download é nosso, para dar progresso por item, resumo por item e
//!    retomada. O motor de download compartilhado (`get_media_info`) hoje
//!    assume vídeo único, então o fan-out de playlist mora neste módulo.
//!
//! Senha nunca aparece em log, em mensagem de erro nem no comando exibido: o
//! argv passa por `shell_words::redact` antes de virar texto e a saída do
//! yt-dlp passa por `scrub` antes de virar mensagem.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use super::{report, sanitize_name, ProgressFn};

const ID_PRIVATE: &str = "vm-private";
const ID_SHOWCASE: &str = "vm-showcase";

/// Marcadores do `--print` do yt-dlp: é assim que sabemos o arquivo final e o
/// título sem ter que adivinhar a extensão.
const MARK_FILE: &str = "OMNIGET_FILEPATH:";
const MARK_META: &str = "OMNIGET_META:";

/// Extensões que contam como "já baixado" na hora de retomar.
const MEDIA_EXTS: &[&str] = &[
    "mp4", "mkv", "webm", "mov", "m4v", "flv", "avi", "mp3", "m4a", "opus", "wav", "aac",
];

// ───────────────────────── alvo (as formas de URL) ─────────────────────────

/// O que um link do Vimeo aponta. `Video` cobre o privado/unlisted (o `hash`
/// é o segredo que vem no próprio link); os outros são coleções.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Video { id: String, hash: Option<String> },
    Showcase { id: String },
    Album { id: String },
    Channel { name: String },
    User { name: String },
}

impl Target {
    /// Rótulo estável para a UI e para o resultado.
    pub fn kind(&self) -> &'static str {
        match self {
            Target::Video { .. } => "video",
            Target::Showcase { .. } => "showcase",
            Target::Album { .. } => "album",
            Target::Channel { .. } => "channel",
            Target::User { .. } => "user",
        }
    }

    /// Coleção (tem vários vídeos dentro) ou vídeo solto.
    pub fn is_collection(&self) -> bool {
        !matches!(self, Target::Video { .. })
    }

    /// URL canônica para entregar ao yt-dlp. O hash do unlisted continua no
    /// caminho, que é como o extractor espera recebê-lo.
    pub fn canonical_url(&self) -> String {
        match self {
            Target::Video { id, hash: Some(h) } => format!("https://vimeo.com/{id}/{h}"),
            Target::Video { id, hash: None } => format!("https://vimeo.com/{id}"),
            Target::Showcase { id } => format!("https://vimeo.com/showcase/{id}"),
            Target::Album { id } => format!("https://vimeo.com/album/{id}"),
            Target::Channel { name } => format!("https://vimeo.com/channels/{name}"),
            Target::User { name } => format!("https://vimeo.com/{name}"),
        }
    }
}

fn is_numeric(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

/// Hash de unlisted: minúsculas e dígitos, curto. Serve para separar
/// `vimeo.com/123/abc123` de `vimeo.com/123/settings`.
fn looks_like_hash(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 16
        && s.chars().all(|c| c.is_ascii_alphanumeric())
        && s.chars().any(|c| c.is_ascii_digit())
}

/// Reconhece as formas de link que interessam. Devolve `None` para o que não
/// é Vimeo ou para caminho que a gente não sabe tratar.
pub fn parse_target(input: &str) -> Option<Target> {
    let raw = input.trim();
    if raw.is_empty() {
        return None;
    }
    // Só um número colado: é um id de vídeo.
    if is_numeric(raw) {
        return Some(Target::Video {
            id: raw.to_string(),
            hash: None,
        });
    }
    let with_scheme = if raw.contains("://") {
        raw.to_string()
    } else {
        format!("https://{raw}")
    };
    let url = url::Url::parse(&with_scheme).ok()?;
    let host = url.host_str()?.trim_start_matches("www.").to_lowercase();
    if host != "vimeo.com" && !host.ends_with(".vimeo.com") {
        return None;
    }
    let segs: Vec<String> = url
        .path_segments()
        .map(|s| s.filter(|p| !p.is_empty()).map(|p| p.to_string()).collect())
        .unwrap_or_default();
    let query_hash = url
        .query_pairs()
        .find(|(k, _)| k == "h")
        .map(|(_, v)| v.to_string())
        .filter(|v| looks_like_hash(v));

    // player.vimeo.com/video/<id>?h=<hash>
    if host == "player.vimeo.com" {
        let id = segs.iter().find(|s| is_numeric(s))?;
        return Some(Target::Video {
            id: id.clone(),
            hash: query_hash,
        });
    }

    // /showcase/<id>/video/<vid> e /album/<id>/video/<vid> são itens de dentro
    // da coleção, não a coleção.
    let inner_video = |segs: &[String]| -> Option<String> {
        if segs.get(2).map(|s| s.as_str()) != Some("video") {
            return None;
        }
        segs.get(3).filter(|s| is_numeric(s.as_str())).cloned()
    };

    match segs.first().map(|s| s.as_str()) {
        None => None,
        Some("showcase") | Some("album") => {
            let is_showcase = segs[0] == "showcase";
            let id = segs.get(1).filter(|s| is_numeric(s.as_str()))?.clone();
            match inner_video(&segs) {
                Some(vid) => Some(Target::Video {
                    id: vid,
                    hash: query_hash,
                }),
                None if is_showcase => Some(Target::Showcase { id }),
                None => Some(Target::Album { id }),
            }
        }
        Some("channels") => {
            let name = segs.get(1)?.clone();
            match segs.get(2).filter(|s| is_numeric(s.as_str())) {
                Some(vid) => Some(Target::Video {
                    id: vid.clone(),
                    hash: query_hash,
                }),
                None => Some(Target::Channel { name }),
            }
        }
        Some("groups") | Some("ondemand") | Some("categories") => None,
        Some(first) if is_numeric(first) => {
            let hash = segs.get(1).filter(|s| looks_like_hash(s.as_str())).cloned();
            Some(Target::Video {
                id: first.to_string(),
                hash: hash.or(query_hash),
            })
        }
        Some(first) => {
            // vimeo.com/<usuario> (perfil) — vídeo do Vimeo é sempre numérico.
            if segs.len() > 2 {
                return None;
            }
            if let Some(second) = segs.get(1) {
                if second != "videos" {
                    return None;
                }
            }
            Some(Target::User {
                name: first.to_string(),
            })
        }
    }
}

// ───────────────────────── entrada e saída ─────────────────────────

fn def_true() -> bool {
    true
}

/// Enumerar uma coleção. A senha aqui é a **do showcase** (a que o Vimeo pede
/// para abrir a coleção), que não é a mesma do vídeo.
#[derive(Debug, Clone, Deserialize)]
pub struct ListOptions {
    pub url: String,
    #[serde(default)]
    pub showcase_password: Option<String>,
    /// Conta do gerenciador de cookies (None = `_default`).
    #[serde(default)]
    pub account_slug: Option<String>,
    /// Conteúdo Netscape do bucket `vimeo.com`. Quem preenche é o comando
    /// Tauri, nunca o front — daí o `skip`.
    #[serde(skip)]
    pub session_netscape: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Entry {
    pub id: String,
    pub title: String,
    pub url: String,
    pub duration: Option<f64>,
    pub uploader: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParsedListing {
    pub id: String,
    pub title: String,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Listing {
    pub kind: String,
    pub id: String,
    pub title: String,
    pub entries: Vec<Entry>,
    pub used_session: bool,
    /// Comando rodado, já redigido — é o que a tela pode mostrar.
    pub command: String,
}

/// Backup em lote. `showcase_password` abre a coleção; `video_password` abre
/// cada item (quando o dono protegeu os vídeos, e não só o showcase).
#[derive(Debug, Clone, Deserialize)]
pub struct BackupOptions {
    pub url: String,
    pub dest: String,
    #[serde(default)]
    pub showcase_password: Option<String>,
    #[serde(default)]
    pub video_password: Option<String>,
    #[serde(default)]
    pub audio_only: bool,
    /// Pula o que já está no destino, íntegro. É a retomada.
    #[serde(default = "def_true")]
    pub skip_existing: bool,
    /// Grava o `.info.json` do yt-dlp ao lado de cada arquivo.
    #[serde(default)]
    pub write_info: bool,
    /// Só estes ids (vazio = a coleção inteira).
    #[serde(default)]
    pub only_ids: Vec<String>,
    #[serde(default)]
    pub account_slug: Option<String>,
    #[serde(skip)]
    pub session_netscape: Option<String>,
}

/// Uma tentativa de item: `ok`, `skipped` ou `failed`.
#[derive(Debug, Clone, Serialize)]
pub struct ItemResult {
    pub id: String,
    pub title: String,
    pub status: String,
    pub file: Option<String>,
    /// Código estável (`password_required`, `wrong_password`, …) ou o texto do
    /// yt-dlp já limpo de segredo.
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BackupResult {
    pub kind: String,
    pub title: String,
    pub dest: String,
    pub total: usize,
    pub ok: usize,
    pub skipped: usize,
    pub failed: usize,
    pub items: Vec<ItemResult>,
    pub used_session: bool,
    pub command: String,
}

/// Um vídeo só, privado ou unlisted.
#[derive(Debug, Clone, Deserialize)]
pub struct PrivateOptions {
    pub url: String,
    pub dest: String,
    /// Senha do vídeo, quando o dono pôs uma. O usuário já a tem.
    #[serde(default)]
    pub video_password: Option<String>,
    #[serde(default)]
    pub audio_only: bool,
    #[serde(default)]
    pub write_info: bool,
    #[serde(default = "def_true")]
    pub skip_existing: bool,
    #[serde(default)]
    pub account_slug: Option<String>,
    #[serde(skip)]
    pub session_netscape: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PrivateResult {
    pub id: String,
    pub title: String,
    pub file: Option<String>,
    pub skipped: bool,
    pub unlisted_hash: bool,
    pub used_password: bool,
    pub used_session: bool,
    pub command: String,
}

// ───────────────────────── segredo: redigir e limpar ─────────────────────────

/// Texto do comando para mostrar e para logar. `shell_words::redact` já
/// esconde `--video-password`, senha, proxy e cabeçalho; aqui acrescentamos o
/// caminho do arquivo de cookies, que é sessão do usuário e não interessa a
/// ninguém que leia o log.
pub fn safe_command_line(program: &str, args: &[String]) -> String {
    let redacted = crate::core::shell_words::redact(args);
    let mut parts = Vec::with_capacity(redacted.len() + 1);
    parts.push(program.to_string());
    let mut hide_next = false;
    for a in redacted {
        if hide_next {
            hide_next = false;
            parts.push("***".to_string());
            continue;
        }
        if a == "--cookies" || a == "--cookies-from-browser" {
            hide_next = true;
            parts.push(a);
            continue;
        }
        if let Some((flag, _)) = a.split_once('=') {
            if flag == "--cookies" || flag == "--cookies-from-browser" {
                parts.push(format!("{flag}=***"));
                continue;
            }
        }
        parts.push(a);
    }
    crate::core::shell_words::join(&parts)
}

/// Tira do texto qualquer ocorrência literal dos segredos. Rede de segurança
/// para a saída do yt-dlp, que a gente não controla.
pub fn scrub(text: &str, secrets: &[&str]) -> String {
    let mut out = text.to_string();
    for s in secrets.iter().filter(|s| !s.trim().is_empty()) {
        out = out.replace(*s, "***");
    }
    out
}

/// Traduz o erro do yt-dlp para um código estável, quando dá. `None` quando o
/// texto não bate com nada conhecido (aí a UI mostra o texto cru já limpo).
pub fn error_code(tail: &str) -> Option<&'static str> {
    let t = tail.to_lowercase();
    if t.contains("wrong video password") || t.contains("invalid password") {
        return Some("wrong_password");
    }
    if t.contains("--video-password")
        || t.contains("protected by a password")
        || t.contains("password protected")
        || t.contains("requires a password")
    {
        return Some("password_required");
    }
    if t.contains("private video") || t.contains("is private") || t.contains("http error 403") {
        return Some("private");
    }
    if t.contains("http error 404") || t.contains("does not exist") || t.contains("was deleted") {
        return Some("not_found");
    }
    if t.contains("not available in your") || (t.contains("geo") && t.contains("restrict")) {
        return Some("geo");
    }
    if t.contains("unable to download") || t.contains("unavailable") {
        return Some("unavailable");
    }
    None
}

// ───────────────────────── cookies da sessão ─────────────────────────

/// Deixa só o que é de `vimeo.com` no arquivo Netscape. O bucket já vem
/// separado por domínio, mas o arquivo pode ter vindo de uma captura ampla e
/// não há motivo para mandar cookie de terceiro para o yt-dlp.
pub fn filter_netscape(content: &str) -> (String, usize) {
    let mut out = String::from("# Netscape HTTP Cookie File\n");
    let mut n = 0;
    for raw in content.lines() {
        let line = raw.trim_end_matches(['\r', '\n']);
        let probe = line.trim();
        if probe.is_empty() {
            continue;
        }
        let body = probe.strip_prefix("#HttpOnly_").unwrap_or(probe);
        if probe.starts_with('#') && !probe.starts_with("#HttpOnly_") {
            continue;
        }
        let Some(domain) = body.split('\t').next() else {
            continue;
        };
        if body.split('\t').count() < 7 {
            continue;
        }
        let d = domain.trim_start_matches('.').to_lowercase();
        if d != "vimeo.com" && !d.ends_with(".vimeo.com") {
            continue;
        }
        out.push_str(line);
        out.push('\n');
        n += 1;
    }
    (out, n)
}

/// Arquivo temporário de cookies que se apaga sozinho.
struct CookieFile {
    path: PathBuf,
}

impl Drop for CookieFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn cookie_file(session: Option<&str>) -> Option<CookieFile> {
    let content = session?;
    let (filtered, n) = filter_netscape(content);
    if n == 0 {
        return None;
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let path = super::temp_dir().join(format!(
        "vimeo-cookies-{}-{}.txt",
        std::process::id(),
        stamp
    ));
    std::fs::write(&path, filtered).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Some(CookieFile { path })
}

// ───────────────────────── argumentos do yt-dlp ─────────────────────────

/// Argumentos da enumeração (`--flat-playlist -J`). O `-J` já implica
/// simulação: nada é baixado aqui.
pub fn list_args(
    url: &str,
    showcase_password: Option<&str>,
    cookies: Option<&Path>,
) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "--ignore-config".to_string(),
        "--no-warnings".to_string(),
        "--no-progress".to_string(),
        "--encoding".to_string(),
        "utf-8".to_string(),
        "--flat-playlist".to_string(),
        "-J".to_string(),
    ];
    if let Some(pw) = showcase_password.filter(|p| !p.is_empty()) {
        args.push("--video-password".to_string());
        args.push(pw.to_string());
    }
    if let Some(c) = cookies {
        args.push("--cookies".to_string());
        args.push(c.to_string_lossy().to_string());
    }
    args.push(url.to_string());
    args
}

/// Tudo que muda de um item para o outro no comando de download.
pub struct DownloadPlan<'a> {
    pub url: &'a str,
    pub dest: &'a Path,
    /// Nome-base do arquivo, sem extensão.
    pub base: &'a str,
    pub audio_only: bool,
    pub write_info: bool,
    pub video_password: Option<&'a str>,
    pub cookies: Option<&'a Path>,
    pub ffmpeg: Option<&'a Path>,
}

/// Argumentos de um download. `--no-playlist` porque cada item já vem
/// resolvido pela enumeração.
pub fn download_args(plan: &DownloadPlan) -> Vec<String> {
    let template = plan.dest.join(format!("{}.%(ext)s", plan.base));
    let mut args: Vec<String> = vec![
        "--ignore-config".to_string(),
        "--no-playlist".to_string(),
        "--newline".to_string(),
        "--no-quiet".to_string(),
        "--no-simulate".to_string(),
        "--progress".to_string(),
        "--encoding".to_string(),
        "utf-8".to_string(),
        "--no-warnings".to_string(),
        "-o".to_string(),
        template.to_string_lossy().to_string(),
        "--print".to_string(),
        format!("after_move:{}%(filepath)s", MARK_FILE),
        "--print".to_string(),
        format!("after_move:{}%(id)s\t%(title)s", MARK_META),
    ];
    if plan.audio_only {
        args.push("-x".to_string());
        args.push("--audio-format".to_string());
        args.push("mp3".to_string());
    } else {
        args.push("-f".to_string());
        args.push("bv*+ba/b".to_string());
        args.push("--merge-output-format".to_string());
        args.push("mp4".to_string());
    }
    if plan.write_info {
        args.push("--write-info-json".to_string());
    }
    if let Some(pw) = plan.video_password.filter(|p| !p.is_empty()) {
        args.push("--video-password".to_string());
        args.push(pw.to_string());
    }
    if let Some(c) = plan.cookies {
        args.push("--cookies".to_string());
        args.push(c.to_string_lossy().to_string());
    }
    if let Some(ff) = plan.ffmpeg {
        args.push("--ffmpeg-location".to_string());
        args.push(ff.to_string_lossy().to_string());
    }
    args.push(plan.url.to_string());
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

// ───────────────────────── leitura do -J ─────────────────────────

fn entry_from_json(v: &serde_json::Value) -> Option<Entry> {
    let id = v
        .get("id")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .unwrap_or_default();
    let raw_url = v
        .get("url")
        .and_then(|x| x.as_str())
        .or_else(|| v.get("webpage_url").and_then(|x| x.as_str()))
        .unwrap_or_default()
        .to_string();
    if id.is_empty() && raw_url.is_empty() {
        return None;
    }
    // Com `--flat-playlist` o `url` às vezes é só o id.
    let url = if raw_url.starts_with("http") {
        raw_url
    } else if !raw_url.is_empty() {
        format!("https://vimeo.com/{raw_url}")
    } else {
        format!("https://vimeo.com/{id}")
    };
    let id = if id.is_empty() {
        url.rsplit('/')
            .find(|s| is_numeric(s))
            .unwrap_or("")
            .to_string()
    } else {
        id
    };
    let title = v
        .get("title")
        .and_then(|x| x.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&id)
        .to_string();
    Some(Entry {
        id,
        title,
        url,
        duration: v.get("duration").and_then(|x| x.as_f64()),
        uploader: v
            .get("uploader")
            .or_else(|| v.get("channel"))
            .and_then(|x| x.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string()),
    })
}

fn collect_entries(v: &serde_json::Value, out: &mut Vec<Entry>) {
    let Some(list) = v.get("entries").and_then(|e| e.as_array()) else {
        return;
    };
    for item in list {
        let is_playlist = item.get("_type").and_then(|t| t.as_str()) == Some("playlist")
            || item.get("entries").is_some();
        if is_playlist {
            collect_entries(item, out);
            continue;
        }
        if let Some(e) = entry_from_json(item) {
            if !out.iter().any(|x| x.id == e.id && !e.id.is_empty()) {
                out.push(e);
            }
        }
    }
}

/// Lê o JSON do `--flat-playlist -J`. Aceita playlist (showcase, álbum,
/// canal), playlist aninhada e o caso degenerado de um vídeo só.
pub fn parse_listing(json: &str) -> Result<ParsedListing> {
    let v: serde_json::Value = serde_json::from_str(json.trim())
        .map_err(|e| anyhow!("o yt-dlp devolveu um JSON que não deu para ler: {e}"))?;
    let id = v
        .get("id")
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_string();
    let title = v
        .get("title")
        .and_then(|x| x.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&id)
        .to_string();
    let mut entries = Vec::new();
    collect_entries(&v, &mut entries);
    if entries.is_empty() && v.get("entries").is_none() {
        // Um vídeo só: o próprio objeto é a entrada.
        if let Some(e) = entry_from_json(&v) {
            entries.push(e);
        }
    }
    Ok(ParsedListing { id, title, entries })
}

// ───────────────────────── retomada ─────────────────────────

/// Nome-base do arquivo de um item. O id no fim é o que permite reconhecer o
/// arquivo depois, mesmo que o título mude.
pub fn base_name(entry: &Entry) -> String {
    let title = if entry.title.trim().is_empty() {
        entry.id.clone()
    } else {
        entry.title.trim().to_string()
    };
    let short: String = title.chars().take(90).collect();
    sanitize_name(&format!("{} [{}]", short.trim(), entry.id))
        .trim()
        .to_string()
}

/// Dado o conteúdo da pasta de destino (`nome`, `bytes`), diz qual arquivo já
/// é o download pronto deste id. Download pela metade (`.part`, `.ytdl`) ou
/// arquivo de zero byte não contam: nesse caso o item é baixado de novo.
pub fn pick_existing(files: &[(String, u64)], id: &str) -> Option<String> {
    if id.is_empty() {
        return None;
    }
    let tag = format!("[{id}]");
    let incomplete = files.iter().any(|(name, _)| {
        (name.ends_with(".part") || name.ends_with(".ytdl")) && name.contains(&tag)
    });
    if incomplete {
        return None;
    }
    files
        .iter()
        .find(|(name, bytes)| {
            if *bytes == 0 || !name.contains(&tag) {
                return false;
            }
            match name.rsplit_once('.') {
                Some((stem, ext)) => {
                    stem.trim_end().ends_with(&tag)
                        && MEDIA_EXTS.contains(&ext.to_lowercase().as_str())
                }
                None => false,
            }
        })
        .map(|(name, _)| name.clone())
}

fn dir_listing(dest: &Path) -> Vec<(String, u64)> {
    let Ok(rd) = std::fs::read_dir(dest) else {
        return Vec::new();
    };
    rd.filter_map(|e| e.ok())
        .map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let bytes = e.metadata().map(|m| m.len()).unwrap_or(0);
            (name, bytes)
        })
        .collect()
}

// ───────────────────────── execução ─────────────────────────

async fn ytdlp_bin() -> Result<PathBuf> {
    crate::core::ytdlp::ensure_ytdlp()
        .await
        .map_err(|e| anyhow!("o yt-dlp não está disponível: {e}"))
}

/// Roda o yt-dlp e devolve o stdout inteiro. Usado só pela enumeração.
async fn run_json(bin: &Path, args: &[String], secrets: &[&str]) -> Result<String> {
    let _slot = crate::core::ytdlp::acquire_ytdlp_slot("vimeo-list").await;
    let mut cmd = crate::core::ytdlp::ytdlp_command(bin);
    cmd.args(args).stdin(std::process::Stdio::null());
    let out = cmd
        .output()
        .await
        .map_err(|e| anyhow!("não foi possível iniciar o yt-dlp: {e}"))?;
    if !out.status.success() {
        let tail = scrub(&String::from_utf8_lossy(&out.stderr), secrets);
        let msg = tail
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .to_string();
        return Err(match error_code(&msg) {
            Some(code) => anyhow!("vimeo:{code}"),
            None => anyhow!("{}", msg),
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// O que sobrou de um download. `tail` já passou pelo `scrub`.
struct DownloadOutcome {
    file: Option<String>,
    meta: Option<(String, String)>,
    tail: String,
    success: bool,
}

/// Roda um download e conta o que aconteceu.
async fn run_download(
    bin: &Path,
    args: &[String],
    secrets: &[&str],
    on_line: impl Fn(f64) + Send + 'static,
) -> Result<DownloadOutcome> {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let _slot = crate::core::ytdlp::acquire_ytdlp_slot("vimeo-download").await;
    let mut cmd = crate::core::ytdlp::ytdlp_command(bin);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| anyhow!("não foi possível iniciar o yt-dlp: {e}"))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let out_task = tokio::spawn(async move {
        let mut file: Option<String> = None;
        let mut meta: Option<(String, String)> = None;
        if let Some(o) = stdout {
            let mut lines = BufReader::new(o).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let l = line.trim();
                if let Some(rest) = l.strip_prefix(MARK_FILE) {
                    if !rest.is_empty() {
                        file = Some(rest.to_string());
                    }
                } else if let Some(rest) = l.strip_prefix(MARK_META) {
                    let mut it = rest.splitn(2, '\t');
                    let id = it.next().unwrap_or("").to_string();
                    let title = it.next().unwrap_or("").to_string();
                    meta = Some((id, title));
                } else if let Some(p) = parse_progress(l) {
                    on_line(p);
                }
            }
        }
        (file, meta)
    });
    let err_task = tokio::spawn(async move {
        let mut tail = String::new();
        if let Some(e) = stderr {
            let mut lines = BufReader::new(e).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if !line.trim().is_empty() {
                    tail = line;
                }
            }
        }
        tail
    });
    let status = child.wait().await?;
    let (file, meta) = out_task.await.unwrap_or((None, None));
    let tail = scrub(&err_task.await.unwrap_or_default(), secrets);
    Ok(DownloadOutcome {
        file,
        meta,
        tail,
        success: status.success(),
    })
}

fn secret_refs<'a>(a: Option<&'a String>, b: Option<&'a String>) -> Vec<&'a str> {
    [a, b]
        .into_iter()
        .flatten()
        .map(|s| s.as_str())
        .filter(|s| !s.trim().is_empty())
        .collect()
}

/// Enumera um showcase, álbum, canal ou perfil.
pub async fn list(opts: &ListOptions, progress: &ProgressFn) -> Result<Listing> {
    let target = parse_target(&opts.url).ok_or_else(|| anyhow!("vimeo:bad_url"))?;
    if !target.is_collection() {
        return Err(anyhow!("vimeo:not_a_collection"));
    }
    let url = target.canonical_url();
    let bin = ytdlp_bin().await?;
    let cookies = cookie_file(opts.session_netscape.as_deref());
    let used_session = cookies.is_some();
    let secrets = secret_refs(opts.showcase_password.as_ref(), None);
    let args = list_args(
        &url,
        opts.showcase_password.as_deref(),
        cookies.as_ref().map(|c| c.path.as_path()),
    );
    let command = safe_command_line("yt-dlp", &args);
    tracing::info!("[vimeo] $ {}", command);
    report(
        progress,
        ID_SHOWCASE,
        "progress",
        0,
        None,
        Some(url.clone()),
    );
    let json = run_json(&bin, &args, &secrets).await?;
    let parsed = parse_listing(&json)?;
    report(
        progress,
        ID_SHOWCASE,
        "done",
        parsed.entries.len() as u64,
        Some(parsed.entries.len() as u64),
        None,
    );
    Ok(Listing {
        kind: target.kind().to_string(),
        id: if parsed.id.is_empty() {
            url.clone()
        } else {
            parsed.id
        },
        title: parsed.title,
        entries: parsed.entries,
        used_session,
        command,
    })
}

/// Backup em lote: enumera, filtra, baixa item a item com progresso e resumo.
pub async fn backup(opts: &BackupOptions, progress: &ProgressFn) -> Result<BackupResult> {
    let target = parse_target(&opts.url).ok_or_else(|| anyhow!("vimeo:bad_url"))?;
    let url = target.canonical_url();
    let dest = PathBuf::from(&opts.dest);
    std::fs::create_dir_all(&dest)?;
    let bin = ytdlp_bin().await?;
    let cookies = cookie_file(opts.session_netscape.as_deref());
    let used_session = cookies.is_some();
    let cookie_path = cookies.as_ref().map(|c| c.path.as_path());
    let secrets = secret_refs(
        opts.showcase_password.as_ref(),
        opts.video_password.as_ref(),
    );

    // 1. enumerar. Vídeo solto vira uma lista de um item, sem chamar o yt-dlp.
    report(
        progress,
        ID_SHOWCASE,
        "progress",
        0,
        None,
        Some(url.clone()),
    );
    let (title, mut entries) = if target.is_collection() {
        let args = list_args(&url, opts.showcase_password.as_deref(), cookie_path);
        tracing::info!("[vimeo] $ {}", safe_command_line("yt-dlp", &args));
        let parsed = parse_listing(&run_json(&bin, &args, &secrets).await?)?;
        (parsed.title, parsed.entries)
    } else {
        let id = match &target {
            Target::Video { id, .. } => id.clone(),
            _ => String::new(),
        };
        (
            id.clone(),
            vec![Entry {
                id,
                title: String::new(),
                url: url.clone(),
                duration: None,
                uploader: None,
            }],
        )
    };
    if !opts.only_ids.is_empty() {
        entries.retain(|e| opts.only_ids.iter().any(|i| i == &e.id));
    }
    if entries.is_empty() {
        return Err(anyhow!("vimeo:empty"));
    }

    // 2. baixar.
    let ffmpeg = crate::core::dependencies::find_tool("ffmpeg").await;
    let total = entries.len();
    let mut items: Vec<ItemResult> = Vec::with_capacity(total);
    let mut ok = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;
    let mut last_command = String::new();

    for (i, entry) in entries.iter().enumerate() {
        let done = i as u64;
        let label = if entry.title.trim().is_empty() {
            entry.id.clone()
        } else {
            entry.title.clone()
        };
        if opts.skip_existing {
            if let Some(found) = pick_existing(&dir_listing(&dest), &entry.id) {
                skipped += 1;
                items.push(ItemResult {
                    id: entry.id.clone(),
                    title: label.clone(),
                    status: "skipped".to_string(),
                    file: Some(dest.join(&found).to_string_lossy().to_string()),
                    reason: None,
                });
                report(
                    progress,
                    ID_SHOWCASE,
                    "progress",
                    done + 1,
                    Some(total as u64),
                    Some(label),
                );
                continue;
            }
        }
        let base = base_name(entry);
        let plan = DownloadPlan {
            url: &entry.url,
            dest: &dest,
            base: &base,
            audio_only: opts.audio_only,
            write_info: opts.write_info,
            video_password: opts.video_password.as_deref(),
            cookies: cookie_path,
            ffmpeg: ffmpeg.as_deref(),
        };
        let args = download_args(&plan);
        last_command = safe_command_line("yt-dlp", &args);
        tracing::info!("[vimeo] $ {}", last_command);
        let p = progress.clone();
        let l = label.clone();
        let res = run_download(&bin, &args, &secrets, move |pctg| {
            report(
                &p,
                ID_SHOWCASE,
                "progress",
                done,
                Some(total as u64),
                Some(format!("{l} · {pctg:.0}%")),
            );
        })
        .await;
        match res {
            Ok(out) => {
                let title = out
                    .meta
                    .as_ref()
                    .map(|(_, t)| t.clone())
                    .filter(|t| !t.trim().is_empty())
                    .unwrap_or_else(|| label.clone());
                if out.success && out.file.is_some() {
                    ok += 1;
                    items.push(ItemResult {
                        id: entry.id.clone(),
                        title,
                        status: "ok".to_string(),
                        file: out.file,
                        reason: None,
                    });
                } else {
                    failed += 1;
                    items.push(ItemResult {
                        id: entry.id.clone(),
                        title,
                        status: "failed".to_string(),
                        file: None,
                        reason: Some(
                            error_code(&out.tail)
                                .map(|c| c.to_string())
                                .unwrap_or_else(|| out.tail.clone()),
                        ),
                    });
                }
            }
            Err(e) => {
                failed += 1;
                items.push(ItemResult {
                    id: entry.id.clone(),
                    title: label.clone(),
                    status: "failed".to_string(),
                    file: None,
                    reason: Some(scrub(&e.to_string(), &secrets)),
                });
            }
        }
        report(
            progress,
            ID_SHOWCASE,
            "progress",
            done + 1,
            Some(total as u64),
            Some(label),
        );
    }
    report(
        progress,
        ID_SHOWCASE,
        "done",
        total as u64,
        Some(total as u64),
        None,
    );
    Ok(BackupResult {
        kind: target.kind().to_string(),
        title,
        dest: dest.to_string_lossy().to_string(),
        total,
        ok,
        skipped,
        failed,
        items,
        used_session,
        command: last_command,
    })
}

/// Um vídeo privado ou unlisted. O hash já vai na URL; a senha, quando existe,
/// vai em `--video-password` e nunca sai daqui.
pub async fn private(opts: &PrivateOptions, progress: &ProgressFn) -> Result<PrivateResult> {
    let target = parse_target(&opts.url).ok_or_else(|| anyhow!("vimeo:bad_url"))?;
    if target.is_collection() {
        return Err(anyhow!("vimeo:is_a_collection"));
    }
    let (id, unlisted_hash) = match &target {
        Target::Video { id, hash } => (id.clone(), hash.is_some()),
        _ => (String::new(), false),
    };
    let url = target.canonical_url();
    let dest = PathBuf::from(&opts.dest);
    std::fs::create_dir_all(&dest)?;
    let bin = ytdlp_bin().await?;
    let ffmpeg = crate::core::dependencies::find_tool("ffmpeg").await;
    let cookies = cookie_file(opts.session_netscape.as_deref());
    let used_session = cookies.is_some();
    let secrets = secret_refs(opts.video_password.as_ref(), None);
    let used_password = opts
        .video_password
        .as_deref()
        .map(|p| !p.is_empty())
        .unwrap_or(false);

    if opts.skip_existing {
        if let Some(found) = pick_existing(&dir_listing(&dest), &id) {
            report(progress, ID_PRIVATE, "done", 1, Some(1), None);
            return Ok(PrivateResult {
                id,
                title: found.clone(),
                file: Some(dest.join(&found).to_string_lossy().to_string()),
                skipped: true,
                unlisted_hash,
                used_password,
                used_session,
                command: String::new(),
            });
        }
    }

    let entry = Entry {
        id: id.clone(),
        title: String::new(),
        url: url.clone(),
        duration: None,
        uploader: None,
    };
    let base = base_name(&entry);
    let plan = DownloadPlan {
        url: &url,
        dest: &dest,
        base: &base,
        audio_only: opts.audio_only,
        write_info: opts.write_info,
        video_password: opts.video_password.as_deref(),
        cookies: cookies.as_ref().map(|c| c.path.as_path()),
        ffmpeg: ffmpeg.as_deref(),
    };
    let args = download_args(&plan);
    let command = safe_command_line("yt-dlp", &args);
    tracing::info!("[vimeo] $ {}", command);
    report(progress, ID_PRIVATE, "progress", 0, Some(100), None);
    let p = progress.clone();
    let out = run_download(&bin, &args, &secrets, move |pctg| {
        report(&p, ID_PRIVATE, "progress", pctg as u64, Some(100), None);
    })
    .await?;
    if !out.success || out.file.is_none() {
        return Err(match error_code(&out.tail) {
            Some(code) => anyhow!("vimeo:{code}"),
            None if out.tail.trim().is_empty() => anyhow!("vimeo:unavailable"),
            None => anyhow!("{}", out.tail),
        });
    }
    report(progress, ID_PRIVATE, "done", 100, Some(100), None);
    let (meta_id, title) = out.meta.unwrap_or_default();
    Ok(PrivateResult {
        id: if id.is_empty() { meta_id } else { id },
        title,
        file: out.file,
        skipped: false,
        unlisted_hash,
        used_password,
        used_session,
        command,
    })
}

// ───────────────────────── testes ─────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const SHOWCASE_JSON: &str = r#"{
      "_type": "playlist",
      "id": "10203040",
      "title": "Reel 2026",
      "extractor": "vimeo:album",
      "webpage_url": "https://vimeo.com/showcase/10203040",
      "entries": [
        {"_type":"url","ie_key":"Vimeo","id":"111111111","title":"Abertura","url":"https://vimeo.com/111111111","duration":61.0,"uploader":"Estudio"},
        {"_type":"url","ie_key":"Vimeo","id":"222222222","title":"Making of","url":"https://vimeo.com/222222222","duration":410.5,"uploader":"Estudio"}
      ]
    }"#;

    const ALBUM_JSON: &str = r#"{
      "_type": "playlist",
      "id": "5060708",
      "title": "Album antigo",
      "extractor": "vimeo:album",
      "entries": [
        {"_type":"url","id":"333333333","title":"Um","url":"333333333"},
        {"_type":"url","id":"444444444","url":"444444444"}
      ]
    }"#;

    const CHANNEL_JSON: &str = r#"{
      "_type": "playlist",
      "id": "staffpicks",
      "title": "Staff Picks",
      "extractor": "vimeo:channel",
      "entries": [
        {"_type":"playlist","id":"pagina1","entries":[
          {"_type":"url","id":"555555555","title":"Curta","url":"https://vimeo.com/555555555"}
        ]},
        {"_type":"url","id":"666666666","title":"Outro","url":"https://vimeo.com/666666666"}
      ]
    }"#;

    #[test]
    fn reconhece_as_tres_formas_de_colecao() {
        assert_eq!(
            parse_target("https://vimeo.com/showcase/10203040"),
            Some(Target::Showcase {
                id: "10203040".into()
            })
        );
        assert_eq!(
            parse_target("https://vimeo.com/album/5060708"),
            Some(Target::Album {
                id: "5060708".into()
            })
        );
        assert_eq!(
            parse_target("https://vimeo.com/channels/staffpicks"),
            Some(Target::Channel {
                name: "staffpicks".into()
            })
        );
        assert_eq!(
            parse_target("vimeo.com/showcase/10203040?share=copy")
                .as_ref()
                .map(|t| t.kind()),
            Some("showcase")
        );
        for t in [
            parse_target("https://vimeo.com/showcase/10203040"),
            parse_target("https://vimeo.com/album/5060708"),
            parse_target("https://vimeo.com/channels/staffpicks"),
        ] {
            assert!(t.map(|t| t.is_collection()).unwrap_or(false));
        }
    }

    #[test]
    fn reconhece_video_privado_com_hash() {
        assert_eq!(
            parse_target("https://vimeo.com/123456789/abc123def4"),
            Some(Target::Video {
                id: "123456789".into(),
                hash: Some("abc123def4".into())
            })
        );
        assert_eq!(
            parse_target("https://vimeo.com/123456789"),
            Some(Target::Video {
                id: "123456789".into(),
                hash: None
            })
        );
        assert_eq!(
            parse_target("https://player.vimeo.com/video/123456789?h=abc123def4"),
            Some(Target::Video {
                id: "123456789".into(),
                hash: Some("abc123def4".into())
            })
        );
        // Item dentro de um showcase e de um canal continua sendo vídeo.
        assert_eq!(
            parse_target("https://vimeo.com/channels/staffpicks/987654321"),
            Some(Target::Video {
                id: "987654321".into(),
                hash: None
            })
        );
        assert_eq!(
            parse_target("https://vimeo.com/showcase/10203040/video/987654321"),
            Some(Target::Video {
                id: "987654321".into(),
                hash: None
            })
        );
        // O hash volta para a URL canônica, senão o unlisted não abre.
        assert_eq!(
            parse_target("https://player.vimeo.com/video/123456789?h=abc123def4")
                .map(|t| t.canonical_url()),
            Some("https://vimeo.com/123456789/abc123def4".to_string())
        );
    }

    #[test]
    fn recusa_o_que_nao_e_vimeo() {
        assert_eq!(parse_target("https://youtube.com/watch?v=abc"), None);
        assert_eq!(parse_target("https://vimeo.com/ondemand/algo"), None);
        assert_eq!(parse_target(""), None);
        assert_eq!(
            parse_target("https://vimeo.com/estudio"),
            Some(Target::User {
                name: "estudio".into()
            })
        );
    }

    #[test]
    fn le_o_json_de_showcase() {
        let p = parse_listing(SHOWCASE_JSON).expect("json de showcase");
        assert_eq!(p.id, "10203040");
        assert_eq!(p.title, "Reel 2026");
        assert_eq!(p.entries.len(), 2);
        assert_eq!(p.entries[0].id, "111111111");
        assert_eq!(p.entries[0].title, "Abertura");
        assert_eq!(p.entries[0].url, "https://vimeo.com/111111111");
        assert_eq!(p.entries[1].duration, Some(410.5));
    }

    #[test]
    fn le_o_json_de_album_com_url_curta() {
        let p = parse_listing(ALBUM_JSON).expect("json de album");
        assert_eq!(p.entries.len(), 2);
        // `--flat-playlist` às vezes devolve só o id em `url`.
        assert_eq!(p.entries[0].url, "https://vimeo.com/333333333");
        // Sem título, o id serve de rótulo.
        assert_eq!(p.entries[1].title, "444444444");
    }

    #[test]
    fn le_o_json_de_canal_com_playlist_aninhada() {
        let p = parse_listing(CHANNEL_JSON).expect("json de canal");
        assert_eq!(p.title, "Staff Picks");
        let ids: Vec<&str> = p.entries.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["555555555", "666666666"]);
    }

    #[test]
    fn le_um_video_solto_como_lista_de_um() {
        let p = parse_listing(
            r#"{"id":"123456789","title":"Solto","webpage_url":"https://vimeo.com/123456789"}"#,
        )
        .expect("json de video");
        assert_eq!(p.entries.len(), 1);
        assert_eq!(p.entries[0].id, "123456789");
    }

    #[test]
    fn json_quebrado_vira_erro_e_nao_panico() {
        assert!(parse_listing("{isso nao e json").is_err());
    }

    #[test]
    fn monta_os_argumentos_da_enumeracao() {
        let args = list_args(
            "https://vimeo.com/showcase/10203040",
            Some("senha-do-showcase"),
            Some(Path::new("/tmp/c.txt")),
        );
        let linha = args.join(" ");
        assert!(linha.contains("--flat-playlist -J"));
        assert!(linha.contains("--video-password senha-do-showcase"));
        assert!(linha.contains("--cookies /tmp/c.txt"));
        assert_eq!(
            args.last().map(|s| s.as_str()),
            Some("https://vimeo.com/showcase/10203040")
        );
        // Sem senha, a flag nem aparece.
        let limpo = list_args("https://vimeo.com/album/1", None, None);
        assert!(!limpo.iter().any(|a| a == "--video-password"));
        assert!(!limpo.iter().any(|a| a == "--cookies"));
    }

    #[test]
    fn monta_os_argumentos_do_download() {
        let dest = Path::new("/tmp/saida");
        let plan = DownloadPlan {
            url: "https://vimeo.com/123456789/abc123def4",
            dest,
            base: "Abertura [111111111]",
            audio_only: false,
            write_info: true,
            video_password: Some("senha-do-video"),
            cookies: Some(Path::new("/tmp/c.txt")),
            ffmpeg: Some(Path::new("/usr/bin/ffmpeg")),
        };
        let args = download_args(&plan);
        let linha = args.join(" ");
        assert!(linha.contains("--no-playlist"));
        assert!(linha.contains("--merge-output-format mp4"));
        assert!(linha.contains("--write-info-json"));
        assert!(linha.contains("--ffmpeg-location /usr/bin/ffmpeg"));
        assert!(args
            .iter()
            .any(|a| a.contains("Abertura [111111111].%(ext)s")));
        assert_eq!(
            args.last().map(|s| s.as_str()),
            Some("https://vimeo.com/123456789/abc123def4")
        );
        let audio = download_args(&DownloadPlan {
            audio_only: true,
            write_info: false,
            video_password: None,
            cookies: None,
            ffmpeg: None,
            ..plan
        });
        assert!(audio.join(" ").contains("-x --audio-format mp3"));
        assert!(!audio.iter().any(|a| a == "--video-password"));
    }

    #[test]
    fn a_senha_nunca_aparece_no_comando_registrado() {
        const SENHA_VIDEO: &str = "S3nha-Do-Video!";
        const SENHA_SHOWCASE: &str = "outra-Senha-99";
        let dest = Path::new("/tmp/saida");
        let baixar = download_args(&DownloadPlan {
            url: "https://vimeo.com/123456789",
            dest,
            base: "x [123456789]",
            audio_only: false,
            write_info: false,
            video_password: Some(SENHA_VIDEO),
            cookies: Some(Path::new("/tmp/c.txt")),
            ffmpeg: None,
        });
        let enumerar = list_args(
            "https://vimeo.com/showcase/10203040",
            Some(SENHA_SHOWCASE),
            Some(Path::new("/tmp/c.txt")),
        );
        for args in [baixar, enumerar] {
            // O argv de verdade tem a senha (é o que o yt-dlp precisa)…
            assert!(args.iter().any(|a| a == SENHA_VIDEO || a == SENHA_SHOWCASE));
            // …mas o que vai para log, tela e resultado, não.
            let linha = safe_command_line("yt-dlp", &args);
            assert!(
                !linha.contains(SENHA_VIDEO) && !linha.contains(SENHA_SHOWCASE),
                "a senha vazou no comando exibido: {linha}"
            );
            assert!(linha.contains("--video-password"));
            // O caminho do arquivo de cookies também é segredo de sessão.
            assert!(!linha.contains("/tmp/c.txt"), "o cookie vazou: {linha}");
        }
    }

    #[test]
    fn scrub_tira_a_senha_da_saida_do_ytdlp() {
        let bruto = "ERROR: Wrong video password (tentei S3nha!) em https://vimeo.com/1";
        let limpo = scrub(bruto, &["S3nha!"]);
        assert!(!limpo.contains("S3nha!"));
        assert!(limpo.contains("***"));
        // Segredo vazio não vira substituição maluca.
        assert_eq!(scrub("texto", &["", "   "]), "texto");
    }

    #[test]
    fn classifica_os_erros_conhecidos() {
        assert_eq!(
            error_code(
                "ERROR: This album is protected by a password, use the --video-password option"
            ),
            Some("password_required")
        );
        assert_eq!(
            error_code("ERROR: Wrong video password"),
            Some("wrong_password")
        );
        assert_eq!(error_code("ERROR: Private video"), Some("private"));
        assert_eq!(
            error_code("ERROR: Unable to download webpage: HTTP Error 404"),
            Some("not_found")
        );
        assert_eq!(error_code("algo totalmente diferente"), None);
    }

    #[test]
    fn pula_o_que_ja_esta_baixado_e_integro() {
        let files = vec![
            ("Abertura [111111111].mp4".to_string(), 8_400_000u64),
            ("Abertura [111111111].info.json".to_string(), 900),
            ("Making of [222222222].mp4.part".to_string(), 120_000),
            ("Vazio [333333333].mp4".to_string(), 0),
        ];
        assert_eq!(
            pick_existing(&files, "111111111"),
            Some("Abertura [111111111].mp4".to_string())
        );
        // Download pela metade não conta como pronto.
        assert_eq!(pick_existing(&files, "222222222"), None);
        // Zero byte também não.
        assert_eq!(pick_existing(&files, "333333333"), None);
        // Id que não está na pasta.
        assert_eq!(pick_existing(&files, "444444444"), None);
        assert_eq!(pick_existing(&files, ""), None);
        // Áudio conta.
        let audio = vec![("Som [555555555].mp3".to_string(), 3_000u64)];
        assert_eq!(
            pick_existing(&audio, "555555555"),
            Some("Som [555555555].mp3".to_string())
        );
    }

    #[test]
    fn nome_base_leva_o_id_no_fim() {
        let e = Entry {
            id: "111111111".into(),
            title: "Abertura: um/dois".into(),
            url: "https://vimeo.com/111111111".into(),
            duration: None,
            uploader: None,
        };
        let base = base_name(&e);
        assert!(base.ends_with("[111111111]"));
        assert!(!base.contains('/'));
        // Sem título, o id vira o nome.
        let sem = base_name(&Entry {
            title: String::new(),
            ..e
        });
        assert_eq!(sem, "111111111 [111111111]");
    }

    #[test]
    fn so_o_cookie_de_vimeo_entra_no_arquivo() {
        let content = "# Netscape HTTP Cookie File\n\
            .vimeo.com\tTRUE\t/\tTRUE\t0\tvuid\t123\n\
            #HttpOnly_vimeo.com\tTRUE\t/\tTRUE\t0\tis_logged_in\t1\n\
            .google.com\tTRUE\t/\tTRUE\t0\tNID\txyz\n\
            player.vimeo.com\tTRUE\t/\tTRUE\t0\tplayer\tabc\n\
            linha quebrada sem colunas\n";
        let (out, n) = filter_netscape(content);
        assert_eq!(n, 3, "só os três cookies de vimeo.com entram");
        assert!(!out.contains("google.com"));
        assert!(out.contains("is_logged_in"));
        assert!(out.starts_with("# Netscape HTTP Cookie File"));
        let (_, vazio) = filter_netscape("# Netscape HTTP Cookie File\n");
        assert_eq!(vazio, 0);
    }

    #[test]
    fn le_o_percentual_do_download() {
        assert_eq!(
            parse_progress("[download]  12.3% of 40.00MiB at 1.00MiB/s"),
            Some(12.3)
        );
        assert_eq!(parse_progress("[info] Downloading 1 format(s)"), None);
    }

    // ── rede: precisam de internet e do yt-dlp, rodam só sob demanda ──

    #[tokio::test]
    #[ignore = "rede: enumera um showcase publico do Vimeo com o yt-dlp de verdade"]
    async fn enumera_showcase_publico_de_verdade() {
        let opts = ListOptions {
            url: "https://vimeo.com/channels/staffpicks".to_string(),
            showcase_password: None,
            account_slug: None,
            session_netscape: None,
        };
        let listing = list(&opts, &super::super::noop_progress())
            .await
            .expect("enumerar canal publico");
        assert!(!listing.entries.is_empty());
        assert!(!listing.command.contains("--video-password"));
    }

    #[tokio::test]
    #[ignore = "rede: baixa um video publico do Vimeo para a pasta temporaria"]
    async fn baixa_um_video_publico_de_verdade() {
        let dest = super::super::temp_dir().join("vimeo-teste");
        let opts = PrivateOptions {
            url: "https://vimeo.com/76979871".to_string(),
            dest: dest.to_string_lossy().to_string(),
            video_password: None,
            audio_only: false,
            write_info: false,
            skip_existing: false,
            account_slug: None,
            session_netscape: None,
        };
        let res = private(&opts, &super::super::noop_progress())
            .await
            .expect("baixar video publico");
        assert!(res.file.is_some());
    }
}
