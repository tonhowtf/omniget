//! TikTok (categoria `tiktok`). Três ferramentas que dividem o mesmo alicerce:
//! leitura de link, freio de requisições, pote de cookies do bucket
//! `tiktok.com` e a chamada do yt-dlp já gerido pelo app.
//!
//! - `download.rs`: vídeo sem marca d'água, em lote (lista colada, `.txt` ou
//!   perfil inteiro), pulando o que já existe no destino.
//! - `sound.rs`: só o áudio — o "som" do vídeo — com a atribuição gravada nas
//!   tags do arquivo e num sidecar de texto.
//! - `favorites.rs`: índice (CSV/JSON) dos favoritos e da mídia do perfil,
//!   com download opcional.
//!
//! Sobre a marca d'água: o yt-dlp entrega o TikTok com um formato chamado
//! `download` (o `download_addr`, o arquivo que o próprio app oferece para
//! salvar — esse **tem** marca) e vários formatos vindos do `play_addr`
//! (`h264_720p_…`, `bytevc1_1080p_…`) que **não** têm. Por isso o seletor
//! padrão exclui o formato `download` em vez de confiar na ordem.

pub mod download;
pub mod favorites;
pub mod sound;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};

use crate::core::tools::{report, ProgressFn};

/// Bucket de cookies do gerenciador. Igual ao `PlatformKind::Tiktok`.
pub const DOMAIN: &str = "tiktok.com";

/// Marcador que o yt-dlp imprime com o caminho final de cada arquivo.
pub const MARK: &str = "OMNIGET_FILEPATH:";

// ───────────────────────── leitura de link ─────────────────────────

/// O que um link do TikTok aponta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// `/@usuario/video/<id>` e `/@usuario/photo/<id>`.
    Video { user: Option<String>, id: String },
    /// `vm.tiktok.com/<código>`, `vt.tiktok.com/<código>` e `/t/<código>`:
    /// só um GET (ou o próprio yt-dlp) diz para onde vai.
    Short { url: String },
    /// Perfil: `/@usuario`.
    User { name: String },
    /// Coleção salva: `/@usuario/collection/<slug>-<id>`.
    Collection { url: String },
    /// Página de um som: `/music/<slug>-<id>`.
    Music { url: String },
}

fn is_tiktok_host(host: &str) -> bool {
    host == "tiktok.com" || host.ends_with(".tiktok.com")
}

/// Deixa a entrada com esquema para o `url::Url` conseguir ler.
fn with_scheme(input: &str) -> String {
    let s = input.trim();
    if s.starts_with("http://") || s.starts_with("https://") {
        return s.to_string();
    }
    if let Some(rest) = s.strip_prefix("//") {
        return format!("https://{}", rest);
    }
    if s.starts_with('/') {
        return format!("https://www.tiktok.com{}", s);
    }
    format!("https://{}", s)
}

/// O id de um vídeo é um inteiro longo (Snowflake do ByteDance).
fn is_video_id(s: &str) -> bool {
    s.len() >= 15 && s.len() <= 25 && s.chars().all(|c| c.is_ascii_digit())
}

/// Lê qualquer forma de link do TikTok: permalink, link curto de
/// compartilhamento, perfil, coleção, som — ou só o id do vídeo. A query de
/// rastreio (`?is_from_webapp=1&sender_device=pc&…`) é ignorada.
pub fn parse_target(input: &str) -> Option<Target> {
    let raw = input.trim();
    if raw.is_empty() {
        return None;
    }
    // Só o id, colado da barra de endereço.
    if !raw.contains('/') && !raw.contains('.') && is_video_id(raw) {
        return Some(Target::Video {
            user: None,
            id: raw.to_string(),
        });
    }
    // `@usuario` colado sozinho.
    if let Some(name) = raw.strip_prefix('@') {
        if !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
        {
            return Some(Target::User {
                name: name.to_ascii_lowercase(),
            });
        }
    }
    let url = url::Url::parse(&with_scheme(raw)).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    if !is_tiktok_host(&host) {
        return None;
    }
    let segs: Vec<String> = url
        .path_segments()
        .map(|s| s.filter(|p| !p.is_empty()).map(|p| p.to_string()).collect())
        .unwrap_or_default();
    let at = |i: usize| segs.get(i).map(|s| s.as_str()).unwrap_or_default();

    // Links curtos: o host manda, o caminho é só o código.
    if host == "vm.tiktok.com" || host == "vt.tiktok.com" {
        if at(0).is_empty() {
            return None;
        }
        return Some(Target::Short {
            url: format!("https://{}/{}", host, at(0)),
        });
    }
    if at(0) == "t" && !at(1).is_empty() {
        return Some(Target::Short {
            url: format!("https://www.tiktok.com/t/{}/", at(1)),
        });
    }
    // `m.tiktok.com/v/<id>.html`
    if at(0) == "v" {
        let id = at(1).trim_end_matches(".html");
        if is_video_id(id) {
            return Some(Target::Video {
                user: None,
                id: id.to_string(),
            });
        }
    }
    if at(0) == "music" && !at(1).is_empty() {
        return Some(Target::Music {
            url: format!("https://www.tiktok.com/music/{}", at(1)),
        });
    }
    if let Some(user) = at(0).strip_prefix('@') {
        if user.is_empty() {
            return None;
        }
        let user = user.to_ascii_lowercase();
        return match at(1) {
            "video" | "photo" if is_video_id(at(2)) => Some(Target::Video {
                user: Some(user),
                id: at(2).to_string(),
            }),
            "collection" if !at(2).is_empty() => Some(Target::Collection {
                url: format!("https://www.tiktok.com/@{}/collection/{}", user, at(2)),
            }),
            "" => Some(Target::User { name: user }),
            _ => Some(Target::User { name: user }),
        };
    }
    None
}

/// O id do vídeo, quando o link já o carrega.
pub fn video_id(input: &str) -> Option<String> {
    match parse_target(input)? {
        Target::Video { id, .. } => Some(id),
        _ => None,
    }
}

/// A URL limpa (sem query de rastreio) que vai para o yt-dlp.
pub fn canonical_url(t: &Target) -> String {
    match t {
        Target::Video { user, id } => match user {
            Some(u) => format!("https://www.tiktok.com/@{}/video/{}", u, id),
            None => format!("https://www.tiktok.com/@_/video/{}", id),
        },
        Target::Short { url } => url.clone(),
        Target::User { name } => format!("https://www.tiktok.com/@{}", name),
        Target::Collection { url } => url.clone(),
        Target::Music { url } => url.clone(),
    }
}

/// Normaliza uma lista colada (uma URL por linha, vírgula ou espaço) em links
/// de **vídeo** do TikTok, sem repetir o mesmo vídeo duas vezes. Perfil,
/// coleção e som não entram: eles têm campo próprio, porque expandi-los custa
/// rede.
pub fn expand_inputs(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for token in text.split([',', '\n', '\r', ' ', '\t']) {
        let token = token.trim();
        if token.is_empty() || token.starts_with('#') {
            continue;
        }
        let Some(target) = parse_target(token) else {
            continue;
        };
        if !matches!(target, Target::Video { .. } | Target::Short { .. }) {
            continue;
        }
        let url = canonical_url(&target);
        // A chave de repetição é o id quando existe; senão a própria URL.
        let key = match &target {
            Target::Video { id, .. } => id.clone(),
            _ => url.clone(),
        };
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        out.push(url);
    }
    out
}

// ───────────────────────── nome de arquivo ─────────────────────────

/// `usuario-<id>` quando dá, só o id quando o link não diz o autor.
pub fn base_name(user: Option<&str>, id: &str) -> String {
    match user.map(|u| u.trim()).filter(|u| !u.is_empty()) {
        Some(u) => crate::core::tools::sanitize_name(&format!("{}-{}", u, id)),
        None => crate::core::tools::sanitize_name(id),
    }
}

/// Se já existe no destino um arquivo com esse nome-base (qualquer extensão).
/// É o que sustenta o "pular o que já baixei" do modo em lote.
pub fn has_stem(dest: &Path, stem: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(dest) else {
        return false;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == stem {
            return true;
        }
        if let Some(rest) = name.strip_prefix(stem) {
            // `.part` e `.ytdl` são sobra de download interrompido: não contam.
            if rest.starts_with('.') && !rest.ends_with(".part") && !rest.ends_with(".ytdl") {
                return true;
            }
        }
    }
    false
}

// ───────────────────────── som ─────────────────────────

/// Slug de título para a URL do som: minúsculas, só letras, números e hífen.
pub fn slugify(s: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            dash = false;
        } else if !out.is_empty() && !dash {
            out.push('-');
            dash = true;
        }
    }
    let s = out.trim_matches('-').to_string();
    if s.is_empty() {
        "sound".to_string()
    } else {
        s
    }
}

/// A página do som: `/music/<slug>-<id>`. O slug é decorativo — o que resolve
/// é o id — mas é o formato que o site publica.
pub fn sound_url(title: &str, id: &str) -> Option<String> {
    if id.trim().is_empty() {
        return None;
    }
    Some(format!(
        "https://www.tiktok.com/music/{}-{}",
        slugify(title),
        id.trim()
    ))
}

/// O objeto `music` embutido na página do vídeo.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Music {
    pub id: String,
    pub title: String,
    pub author: String,
    pub original: bool,
    pub duration: u64,
    pub play_url: String,
}

/// Recorta o primeiro objeto JSON equilibrado a partir de uma posição.
fn balanced_object(hay: &str, from: usize) -> Option<&str> {
    let bytes = hay.as_bytes();
    if bytes.get(from) != Some(&b'{') {
        return None;
    }
    let mut depth = 0usize;
    let mut in_str = false;
    let mut escaped = false;
    for (i, b) in bytes.iter().enumerate().skip(from) {
        if in_str {
            if escaped {
                escaped = false;
            } else if *b == b'\\' {
                escaped = true;
            } else if *b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return hay.get(from..=i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Lê `"music":{…}` de dentro do HTML da página do vídeo. É esse objeto que
/// carrega o id do som — sem ele não dá para montar o link de atribuição,
/// porque o yt-dlp só devolve o nome (`track`) e o autor (`artist`).
pub fn parse_music(html: &str) -> Option<Music> {
    let key = "\"music\":{";
    let at = html.find(key)? + key.len() - 1;
    let raw = balanced_object(html, at)?;
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;
    let s = |k: &str| {
        v.get(k)
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string()
    };
    let id = s("id");
    if id.is_empty() {
        return None;
    }
    Some(Music {
        id,
        title: s("title"),
        author: s("authorName"),
        original: v.get("original").and_then(|x| x.as_bool()).unwrap_or(false),
        duration: v.get("duration").and_then(|x| x.as_u64()).unwrap_or(0),
        play_url: s("playUrl"),
    })
}

// ───────────────────────── cookies ─────────────────────────

/// Só o que é do bucket `tiktok.com`. Cookie de outro domínio no mesmo
/// arquivo (o gerenciador grava um arquivo por conta, mas nada impede que o
/// usuário aponte um arquivo qualquer) não entra.
pub fn is_tiktok_cookie_domain(domain: &str) -> bool {
    let d = domain.trim_start_matches('.').to_ascii_lowercase();
    d == DOMAIN || d.ends_with(".tiktok.com")
}

/// Filtra os cookies do bucket `tiktok.com` de um arquivo Netscape e monta o
/// pote do cliente. Reusa o parser da sessão do Instagram — é o mesmo formato
/// que o gerenciador de cookies grava para todo mundo.
pub fn jar_from_netscape(content: &str) -> (Arc<reqwest::cookie::Jar>, usize) {
    let jar = reqwest::cookie::Jar::default();
    let mut n = 0;
    for c in crate::core::tools::instagram::parse_netscape(content) {
        if !is_tiktok_cookie_domain(&c.domain) {
            continue;
        }
        let domain = c.domain.trim_start_matches('.');
        let path = if c.path.is_empty() { "/" } else { &c.path };
        let scheme = if c.secure { "https" } else { "http" };
        let Ok(url) = format!("{}://{}{}", scheme, domain, path).parse::<reqwest::Url>() else {
            continue;
        };
        jar.add_cookie_str(
            &format!("{}={}; Domain={}; Path={}", c.name, c.value, c.domain, path),
            &url,
        );
        n += 1;
    }
    (Arc::new(jar), n)
}

/// Reescreve o arquivo Netscape ficando só com o bucket `tiktok.com`, no
/// formato que o yt-dlp lê. Sai como texto para o teste conferir sem disco.
pub fn netscape_for_ytdlp(content: &str) -> String {
    let mut out = String::from("# Netscape HTTP Cookie File\n");
    for raw in content.lines() {
        let line = raw.trim_end_matches(['\r', '\n']);
        let bare = line.trim_start().strip_prefix("#HttpOnly_").unwrap_or(line);
        if bare.trim().is_empty() || bare.trim_start().starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = bare.split('\t').collect();
        if cols.len() < 7 || !is_tiktok_cookie_domain(cols[0]) {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Grava o arquivo de cookies temporário que o yt-dlp recebe em `--cookies`.
/// Devolve `None` quando não sobrou cookie nenhum do TikTok.
pub fn write_cookie_file(content: &str) -> Option<PathBuf> {
    let text = netscape_for_ytdlp(content);
    if text.lines().filter(|l| !l.starts_with('#')).count() == 0 {
        return None;
    }
    let dir = crate::core::tools::temp_dir().join("tiktok");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("cookies-{}.txt", uuid::Uuid::new_v4()));
    std::fs::write(&path, text).ok()?;
    Some(path)
}

/// Apaga o arquivo de cookies temporário assim que o processo termina.
pub struct TempCookies(Option<PathBuf>);

impl TempCookies {
    pub fn new(session: Option<&str>) -> Self {
        Self(session.and_then(write_cookie_file))
    }
    pub fn path(&self) -> Option<&Path> {
        self.0.as_deref()
    }
    pub fn is_some(&self) -> bool {
        self.0.is_some()
    }
}

impl Drop for TempCookies {
    fn drop(&mut self) {
        if let Some(p) = &self.0 {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// Cliente com o pote de cookies do TikTok e cabeçalhos de navegador. O site
/// devolve uma casca vazia para quem não parece navegador.
pub fn cookie_client(session: Option<&str>) -> Result<(reqwest::Client, bool)> {
    use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, ACCEPT_LANGUAGE, USER_AGENT};
    let mut headers = HeaderMap::new();
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static(
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36",
        ),
    );
    headers.insert(
        ACCEPT,
        HeaderValue::from_static(
            "text/html,application/xhtml+xml,application/json;q=0.9,*/*;q=0.8",
        ),
    );
    headers.insert(
        ACCEPT_LANGUAGE,
        HeaderValue::from_static("en-US,en;q=0.9,pt-BR;q=0.8"),
    );
    let (jar, seeded) = match session {
        Some(content) => jar_from_netscape(content),
        None => (Arc::new(reqwest::cookie::Jar::default()), 0),
    };
    let client = crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
        .default_headers(headers)
        .cookie_provider(jar)
        .timeout(Duration::from_secs(120))
        .build()?;
    Ok((client, seeded > 0))
}

// ───────────────────────── paciência ─────────────────────────

/// Freio entre requisições, no espírito do `Fetcher` do Reddit: uma de cada
/// vez, com espera mínima entre elas. O TikTok responde com uma casca vazia
/// (ou nada) quando a gente insiste rápido demais.
pub struct Pacer {
    delay: Duration,
    last: tokio::sync::Mutex<Option<Instant>>,
    count: AtomicU32,
}

impl Pacer {
    pub fn new(delay_ms: u64) -> Self {
        Self {
            delay: Duration::from_millis(delay_ms.clamp(200, 20_000)),
            last: tokio::sync::Mutex::new(None),
            count: AtomicU32::new(0),
        }
    }

    pub async fn wait(&self) {
        let mut last = self.last.lock().await;
        if let Some(t) = *last {
            let since = t.elapsed();
            if since < self.delay {
                tokio::time::sleep(self.delay - since).await;
            }
        }
        *last = Some(Instant::now());
        self.count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn count(&self) -> u32 {
        self.count.load(Ordering::Relaxed)
    }
}

// ───────────────────────── yt-dlp ─────────────────────────

/// Seletor de formato.
///
/// O formato `download` do extrator de TikTok é o `download_addr`: o arquivo
/// que o app oferece para salvar, e é justamente ele que vem com a marca
/// d'água. Todo o resto (`h264_720p_…`, `bytevc1_1080p_…`) sai do `play_addr`
/// e é limpo. Por isso o padrão é **excluir** `download` em vez de escolher um
/// nome fixo de formato — se um dia o extrator renomear os limpos, a exclusão
/// continua valendo, e o `/bv*+ba/b` no fim é a rede de segurança.
pub fn format_selector(watermark: bool, quality: &str) -> String {
    if watermark {
        return "download/b".to_string();
    }
    match quality {
        // h264 para quem precisa de compatibilidade (o 1080p do TikTok é h265).
        "h264" => "bv*[vcodec^=avc][format_id!*=download]+ba/b[vcodec^=avc][format_id!*=download]/bv*[format_id!*=download]+ba/b[format_id!*=download]/bv*+ba/b".to_string(),
        _ => "bv*[format_id!*=download]+ba[format_id!*=download]/b[format_id!*=download]/bv*+ba/b".to_string(),
    }
}

/// O que se quer do yt-dlp: o vídeo (com o seletor de formato) ou só o som
/// (com a extensão do áudio).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode<'a> {
    Video { selector: &'a str },
    Audio { format: &'a str },
}

/// Argumentos do yt-dlp para um vídeo. Separado para o teste conferir sem
/// precisar do binário.
pub fn ytdlp_args(
    url: &str,
    dest: &Path,
    base: &str,
    mode: Mode<'_>,
    write_info_json: bool,
    ffmpeg: Option<&Path>,
    cookies: Option<&Path>,
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
    match mode {
        Mode::Audio { format } => {
            // O formato `audio` do extrator é o som do vídeo em mp3.
            args.push("-f".to_string());
            args.push("ba[format_id=audio]/ba/b".to_string());
            args.push("-x".to_string());
            args.push("--audio-format".to_string());
            args.push(format.to_string());
        }
        Mode::Video { selector } => {
            args.push("-f".to_string());
            args.push(selector.to_string());
            args.push("--merge-output-format".to_string());
            args.push("mp4".to_string());
        }
    }
    if write_info_json {
        args.push("--write-info-json".to_string());
    }
    if let Some(ff) = ffmpeg {
        args.push("--ffmpeg-location".to_string());
        args.push(ff.to_string_lossy().to_string());
    }
    if let Some(c) = cookies {
        args.push("--cookies".to_string());
        args.push(c.to_string_lossy().to_string());
    }
    args.push(url.to_string());
    args
}

/// Argumentos da listagem: metadados de um perfil/coleção inteiros numa só
/// chamada, sem baixar nada.
pub fn ytdlp_list_args(url: &str, limit: u32, cookies: Option<&Path>) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "--ignore-config".to_string(),
        "--no-warnings".to_string(),
        "--flat-playlist".to_string(),
        "-J".to_string(),
        "--encoding".to_string(),
        "utf-8".to_string(),
    ];
    if limit > 0 {
        args.push("--playlist-end".to_string());
        args.push(limit.to_string());
    }
    if let Some(c) = cookies {
        args.push("--cookies".to_string());
        args.push(c.to_string_lossy().to_string());
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

/// Última linha útil de erro do yt-dlp, para virar "motivo" no resumo.
pub fn short_reason(tail: &str) -> String {
    let line = tail
        .lines()
        .rev()
        .map(|l| l.trim())
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let line = line
        .trim_start_matches("ERROR:")
        .trim_start_matches("WARNING:")
        .trim();
    let line = match line.split_once("] ") {
        Some((head, rest)) if head.starts_with('[') => rest,
        _ => line,
    };
    let mut s: String = line.chars().take(220).collect();
    if s.is_empty() {
        s = "o yt-dlp não baixou nada".to_string();
    }
    s
}

async fn ytdlp_binary() -> Result<PathBuf> {
    crate::core::ytdlp::ensure_ytdlp()
        .await
        .map_err(|e| anyhow!("o yt-dlp não está disponível: {}", e))
}

/// Roda o yt-dlp e devolve (arquivos criados, últimas linhas de erro).
pub async fn run_ytdlp(
    args: &[String],
    id: &str,
    progress: &ProgressFn,
) -> Result<(Vec<String>, String)> {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let bin = ytdlp_binary().await?;
    let mut cmd = crate::core::ytdlp::ytdlp_command(&bin);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| anyhow!("não foi possível iniciar o yt-dlp: {}", e))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let p = progress.clone();
    let id_owned = id.to_string();
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
                    report(
                        &p,
                        &id_owned,
                        "progress",
                        pct.round() as u64,
                        Some(100),
                        None,
                    );
                }
            }
        }
        files
    });
    let err_task = tokio::spawn(async move {
        let mut tail: Vec<String> = Vec::new();
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
    let _ = child.wait().await?;
    let files = out_task.await.unwrap_or_default();
    let tail = err_task.await.unwrap_or_default();
    Ok((files, tail))
}

/// Roda o yt-dlp só para ler JSON (metadados ou listagem).
pub async fn ytdlp_json(args: &[String]) -> Result<serde_json::Value> {
    let bin = ytdlp_binary().await?;
    let mut cmd = crate::core::ytdlp::ytdlp_command(&bin);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let out = cmd
        .output()
        .await
        .map_err(|e| anyhow!("não foi possível iniciar o yt-dlp: {}", e))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text
        .lines()
        .map(|l| l.trim())
        .find(|l| l.starts_with('{'))
        .unwrap_or("");
    if line.is_empty() {
        let tail = String::from_utf8_lossy(&out.stderr).to_string();
        return Err(anyhow!("{}", short_reason(&tail)));
    }
    serde_json::from_str(line).map_err(|e| anyhow!("o JSON do yt-dlp não foi lido: {}", e))
}

// ───────────────────────── formatação ─────────────────────────

pub fn fmt_utc(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|d| d.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_default()
}

/// Escapa um campo de CSV do jeito que a planilha espera.
pub fn csv_escape(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

// ───────────────────────── testes ─────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_o_permalink_com_e_sem_query_de_rastreio() {
        let esperado = Target::Video {
            user: Some("tiktok".to_string()),
            id: "7683195368279985438".to_string(),
        };
        assert_eq!(
            parse_target("https://www.tiktok.com/@tiktok/video/7683195368279985438"),
            Some(esperado.clone())
        );
        assert_eq!(
            parse_target(
                "https://www.tiktok.com/@TikTok/video/7683195368279985438?is_from_webapp=1&sender_device=pc&web_id=123"
            ),
            Some(esperado.clone())
        );
        assert_eq!(
            parse_target("tiktok.com/@tiktok/video/7683195368279985438/"),
            Some(esperado)
        );
    }

    #[test]
    fn post_de_foto_conta_como_video() {
        assert_eq!(
            parse_target("https://www.tiktok.com/@alguem/photo/7123456789012345678"),
            Some(Target::Video {
                user: Some("alguem".to_string()),
                id: "7123456789012345678".to_string(),
            })
        );
    }

    #[test]
    fn le_os_tres_formatos_de_link_curto() {
        assert_eq!(
            parse_target("https://vm.tiktok.com/ZMhKq7Yd8/"),
            Some(Target::Short {
                url: "https://vm.tiktok.com/ZMhKq7Yd8".to_string()
            })
        );
        assert_eq!(
            parse_target("vt.tiktok.com/ZSjqRPQ2b"),
            Some(Target::Short {
                url: "https://vt.tiktok.com/ZSjqRPQ2b".to_string()
            })
        );
        assert_eq!(
            parse_target("https://www.tiktok.com/t/ZTdxyKQnp/?k=1"),
            Some(Target::Short {
                url: "https://www.tiktok.com/t/ZTdxyKQnp/".to_string()
            })
        );
    }

    #[test]
    fn le_perfil_colecao_som_e_id_solto() {
        assert_eq!(
            parse_target("https://www.tiktok.com/@nasa"),
            Some(Target::User {
                name: "nasa".to_string()
            })
        );
        assert_eq!(
            parse_target("@NASA"),
            Some(Target::User {
                name: "nasa".to_string()
            })
        );
        assert_eq!(
            parse_target("https://www.tiktok.com/@nasa/collection/espaco-7300000000000000000"),
            Some(Target::Collection {
                url: "https://www.tiktok.com/@nasa/collection/espaco-7300000000000000000"
                    .to_string()
            })
        );
        assert_eq!(
            parse_target("https://www.tiktok.com/music/original-sound-7683266637168610079"),
            Some(Target::Music {
                url: "https://www.tiktok.com/music/original-sound-7683266637168610079".to_string()
            })
        );
        assert_eq!(
            parse_target("7683195368279985438"),
            Some(Target::Video {
                user: None,
                id: "7683195368279985438".to_string()
            })
        );
        assert_eq!(
            parse_target("https://m.tiktok.com/v/7683195368279985438.html"),
            Some(Target::Video {
                user: None,
                id: "7683195368279985438".to_string()
            })
        );
    }

    #[test]
    fn recusa_o_que_nao_e_do_tiktok() {
        assert_eq!(parse_target("https://youtube.com/watch?v=abc"), None);
        assert_eq!(parse_target("   "), None);
        assert_eq!(parse_target("123"), None);
        assert_eq!(
            parse_target("https://tiktokfake.com/@x/video/7683195368279985438"),
            None
        );
    }

    #[test]
    fn extrai_o_id_do_video() {
        assert_eq!(
            video_id("https://www.tiktok.com/@a/video/7683195368279985438?x=1").as_deref(),
            Some("7683195368279985438")
        );
        assert_eq!(video_id("https://www.tiktok.com/@a"), None);
    }

    #[test]
    fn lista_colada_vira_urls_limpas_sem_repetir() {
        let texto = "https://www.tiktok.com/@a/video/7683195368279985438?is_from_webapp=1\n\
                     # comentário\n\
                     https://www.tiktok.com/@a/video/7683195368279985438\n\
                     vm.tiktok.com/ZMhKq7Yd8\n\
                     lixo\n\
                     https://www.tiktok.com/@perfil\n\
                     https://www.tiktok.com/@b/video/7681695065927912735";
        assert_eq!(
            expand_inputs(texto),
            vec![
                "https://www.tiktok.com/@a/video/7683195368279985438".to_string(),
                "https://vm.tiktok.com/ZMhKq7Yd8".to_string(),
                "https://www.tiktok.com/@b/video/7681695065927912735".to_string(),
            ]
        );
    }

    #[test]
    fn monta_o_vetor_de_argumentos_do_ytdlp() {
        let dest = Path::new("/tmp/tt");
        let args = ytdlp_args(
            "https://www.tiktok.com/@a/video/7683195368279985438",
            dest,
            "a-7683195368279985438",
            Mode::Video {
                selector: &format_selector(false, "best"),
            },
            false,
            Some(Path::new("/bin/ffmpeg")),
            Some(Path::new("/tmp/c.txt")),
        );
        let pos = |flag: &str| args.iter().position(|a| a == flag);
        assert_eq!(
            args.last().map(|s| s.as_str()),
            Some("https://www.tiktok.com/@a/video/7683195368279985438")
        );
        assert!(args.contains(&"--no-playlist".to_string()));
        let f = pos("-f")
            .and_then(|i| args.get(i + 1))
            .cloned()
            .unwrap_or_default();
        assert!(
            f.contains("format_id!*=download"),
            "seletor sem a exclusão da marca: {}",
            f
        );
        assert_eq!(
            pos("-o").and_then(|i| args.get(i + 1)).map(|s| s.as_str()),
            Some(
                dest.join("a-7683195368279985438.%(ext)s")
                    .to_string_lossy()
                    .to_string()
                    .as_str()
            )
        );
        assert_eq!(
            pos("--cookies")
                .and_then(|i| args.get(i + 1))
                .map(|s| s.as_str()),
            Some("/tmp/c.txt")
        );
        assert!(!args.contains(&"--write-info-json".to_string()));
    }

    #[test]
    fn seletor_de_marca_dagua_pede_o_formato_download() {
        assert_eq!(format_selector(true, "best"), "download/b");
        assert!(format_selector(false, "h264").contains("vcodec^=avc"));
        assert!(format_selector(false, "h264").contains("format_id!*=download"));
    }

    #[test]
    fn modo_audio_troca_o_seletor_e_extrai() {
        let args = ytdlp_args(
            "https://www.tiktok.com/@a/video/1",
            Path::new("/tmp"),
            "som",
            Mode::Audio { format: "mp3" },
            true,
            None,
            None,
        );
        assert!(args.contains(&"-x".to_string()));
        assert!(args.contains(&"ba[format_id=audio]/ba/b".to_string()));
        assert!(args.contains(&"--write-info-json".to_string()));
        assert!(!args.contains(&"--merge-output-format".to_string()));
    }

    #[test]
    fn argumentos_da_listagem() {
        let args = ytdlp_list_args("https://www.tiktok.com/@a", 40, None);
        assert!(args.contains(&"--flat-playlist".to_string()));
        assert!(args.contains(&"-J".to_string()));
        let i = args.iter().position(|a| a == "--playlist-end");
        assert_eq!(
            i.and_then(|i| args.get(i + 1)).map(|s| s.as_str()),
            Some("40")
        );
        assert!(!ytdlp_list_args("https://www.tiktok.com/@a", 0, None)
            .contains(&"--playlist-end".to_string()));
    }

    #[test]
    fn le_o_percentual_do_yt_dlp() {
        assert_eq!(parse_progress("[download]  12.3% of 5.53MiB"), Some(12.3));
        assert_eq!(parse_progress("[TikTok] baixando"), None);
    }

    #[test]
    fn motivo_curto_tira_o_prefixo_do_yt_dlp() {
        assert_eq!(
            short_reason("algo\nERROR: [TikTok] 123: Video not available"),
            "123: Video not available"
        );
        assert_eq!(short_reason(""), "o yt-dlp não baixou nada");
    }

    // ── som ──

    #[test]
    fn le_o_objeto_music_da_pagina() {
        let html = r#"<script>window.x={"itemInfo":{"itemStruct":{"id":"7683195368279985438","music":{"id":"7683266637168610079","title":"original sound","playUrl":"https://v58.tiktokcdn.com/a?b=1&c=2","authorName":"TikTok","original":true,"duration":68},"author":{"uniqueId":"tiktok"}}}}</script>"#;
        let m = parse_music(html).unwrap_or_default();
        assert_eq!(m.id, "7683266637168610079");
        assert_eq!(m.title, "original sound");
        assert_eq!(m.author, "TikTok");
        assert!(m.original);
        assert_eq!(m.duration, 68);
        assert!(m.play_url.starts_with("https://v58.tiktokcdn.com/"));
    }

    #[test]
    fn objeto_music_com_chave_e_aspas_escapadas_nao_confunde_o_recorte() {
        let html = r#"{"desc":"olha o \"som\" {aqui}","music":{"id":"42","title":"a } b","authorName":"x"},"depois":1}"#;
        let m = parse_music(html).unwrap_or_default();
        assert_eq!(m.id, "42");
        assert_eq!(m.title, "a } b");
    }

    #[test]
    fn pagina_sem_music_nao_inventa() {
        assert_eq!(parse_music("<html>nada aqui</html>"), None);
        assert_eq!(parse_music(r#"{"music":{"title":"sem id"}}"#), None);
    }

    #[test]
    fn monta_o_link_do_som() {
        assert_eq!(
            sound_url("original sound", "7683266637168610079").as_deref(),
            Some("https://www.tiktok.com/music/original-sound-7683266637168610079")
        );
        assert_eq!(
            sound_url("Beat  do  Ritmo!! (remix)", "1").as_deref(),
            Some("https://www.tiktok.com/music/beat-do-ritmo-remix-1")
        );
        assert_eq!(sound_url("x", "  ").as_deref(), None);
    }

    // ── cookies ──

    #[test]
    fn o_pote_so_aceita_cookie_do_tiktok() {
        let content = "# Netscape HTTP Cookie File\n\
            .tiktok.com\tTRUE\t/\tTRUE\t9999999999\tsessionid\tSEGREDO\n\
            #HttpOnly_.tiktok.com\tTRUE\t/\tTRUE\t9999999999\tsid_tt\tOUTRO\n\
            www.tiktok.com\tFALSE\t/\tFALSE\t9999999999\tttwid\tW1\n\
            .instagram.com\tTRUE\t/\tTRUE\t9999999999\tsessionid\tNAO\n\
            .tiktokfake.com\tTRUE\t/\tTRUE\t9999999999\tx\tNAO\n";
        let (_, n) = jar_from_netscape(content);
        assert_eq!(n, 3, "só os três do tiktok.com podem entrar");

        let filtrado = netscape_for_ytdlp(content);
        assert!(filtrado.contains("sessionid\tSEGREDO"));
        assert!(filtrado.contains("sid_tt"));
        assert!(filtrado.contains("ttwid"));
        assert!(!filtrado.contains("instagram"));
        assert!(!filtrado.contains("tiktokfake"));
        // O `#HttpOnly_` continua na frente da linha (é assim que o formato
        // marca o cookie, e o yt-dlp lê): a conta é de linhas de cookie, não
        // de linhas sem `#`.
        assert_eq!(filtrado.lines().count(), 4, "cabeçalho mais três cookies");
        assert!(filtrado.starts_with("# Netscape HTTP Cookie File\n"));
    }

    #[test]
    fn dominio_de_cookie_confere_o_sufixo_inteiro() {
        assert!(is_tiktok_cookie_domain(".tiktok.com"));
        assert!(is_tiktok_cookie_domain("www.tiktok.com"));
        assert!(is_tiktok_cookie_domain("TIKTOK.COM"));
        assert!(!is_tiktok_cookie_domain("tiktok.com.br"));
        assert!(!is_tiktok_cookie_domain("evil-tiktok.com"));
        assert!(!is_tiktok_cookie_domain("instagram.com"));
    }

    #[test]
    fn arquivo_de_cookies_sem_tiktok_nao_e_gravado() {
        assert!(write_cookie_file(".instagram.com\tTRUE\t/\tTRUE\t1\ta\tb\n").is_none());
    }

    // ── nomes e destino ──

    #[test]
    fn nome_base_com_e_sem_autor() {
        assert_eq!(base_name(Some("nasa"), "123"), "nasa-123");
        assert_eq!(base_name(None, "123"), "123");
        assert_eq!(base_name(Some(""), "123"), "123");
    }

    #[test]
    fn pular_o_que_ja_existe_ignora_sobra_de_download() {
        let dir = crate::core::tools::temp_dir().join(format!("tt-test-{}", uuid::Uuid::new_v4()));
        assert!(std::fs::create_dir_all(&dir).is_ok());
        assert!(!has_stem(&dir, "a-1"));
        assert!(std::fs::write(dir.join("a-1.mp4.part"), b"x").is_ok());
        assert!(
            !has_stem(&dir, "a-1"),
            "arquivo .part não conta como pronto"
        );
        assert!(std::fs::write(dir.join("a-1.mp4"), b"x").is_ok());
        assert!(has_stem(&dir, "a-1"));
        assert!(!has_stem(&dir, "a-2"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn csv_escapa_virgula_aspas_e_quebra() {
        assert_eq!(csv_escape("simples"), "simples");
        assert_eq!(csv_escape("a,b"), "\"a,b\"");
        assert_eq!(csv_escape("diz \"oi\""), "\"diz \"\"oi\"\"\"");
        assert_eq!(csv_escape("linha\nquebrada"), "\"linha\nquebrada\"");
    }

    #[test]
    fn data_utc_legivel() {
        assert_eq!(fmt_utc(1788883336), "2026-09-08 16:02 UTC");
        assert_eq!(fmt_utc(0), "1970-01-01 00:00 UTC");
    }
}
