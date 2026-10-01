use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::anyhow;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use serde::Serialize;

use crate::core::log_hook;
use crate::models::media::{DownloadResult, FormatInfo};
use crate::models::progress::{ProgressUpdate, StreamInfo};

type ExtCookiePathFn = Box<dyn Fn() -> PathBuf + Send + Sync>;
type GlobalCookieFileFn = Box<dyn Fn() -> Option<String> + Send + Sync>;
type CookiesFromBrowserFn = Box<dyn Fn() -> String + Send + Sync>;
type ManualCookieHeaderFn = Box<dyn Fn() -> String + Send + Sync>;
type ExtRefererFn = Box<dyn Fn(&str) -> Option<String> + Send + Sync>;
type IncludeAutoSubsFn = Box<dyn Fn() -> bool + Send + Sync>;
type CaptionLocaleFn = Box<dyn Fn() -> String + Send + Sync>;
type KeepVttFn = Box<dyn Fn() -> bool + Send + Sync>;
type TranslateMetadataFn = Box<dyn Fn() -> Option<String> + Send + Sync>;
type SponsorBlockFn = Box<dyn Fn() -> bool + Send + Sync>;
type SplitChaptersFn = Box<dyn Fn() -> bool + Send + Sync>;
type EmbedMetadataFn = Box<dyn Fn() -> bool + Send + Sync>;
type EmbedThumbnailFn = Box<dyn Fn() -> bool + Send + Sync>;
type SpeedLimitFn = Box<dyn Fn() -> Option<String> + Send + Sync>;
type LiveFromStartFn = Box<dyn Fn() -> bool + Send + Sync>;
type InsecureTlsFn = Box<dyn Fn() -> bool + Send + Sync>;
type ConcurrentFragmentsFn = Box<dyn Fn() -> u32 + Send + Sync>;
type UserAgentFn = Box<dyn Fn() -> Option<String> + Send + Sync>;
type SponsorBlockModeFn = Box<dyn Fn() -> String + Send + Sync>;
type SponsorBlockCategoriesFn = Box<dyn Fn() -> Vec<String> + Send + Sync>;
type PerDomainCookieFn = Box<dyn Fn(&str) -> Option<PathBuf> + Send + Sync>;
type ManagedCookiesOnlyFn = Box<dyn Fn() -> bool + Send + Sync>;

static EXT_COOKIE_PATH_FN: OnceLock<ExtCookiePathFn> = OnceLock::new();
static GLOBAL_COOKIE_FILE_FN: OnceLock<GlobalCookieFileFn> = OnceLock::new();
static COOKIES_FROM_BROWSER_FN: OnceLock<CookiesFromBrowserFn> = OnceLock::new();
static MANUAL_COOKIE_HEADER_FN: OnceLock<ManualCookieHeaderFn> = OnceLock::new();
static EXT_REFERER_FN: OnceLock<ExtRefererFn> = OnceLock::new();
static INCLUDE_AUTO_SUBS_FN: OnceLock<IncludeAutoSubsFn> = OnceLock::new();
static CAPTION_LOCALE_FN: OnceLock<CaptionLocaleFn> = OnceLock::new();
static KEEP_VTT_FN: OnceLock<KeepVttFn> = OnceLock::new();
static PER_DOMAIN_COOKIE_FN: OnceLock<PerDomainCookieFn> = OnceLock::new();
static MANAGED_COOKIES_ONLY_FN: OnceLock<ManagedCookiesOnlyFn> = OnceLock::new();
static TRANSLATE_METADATA_FN: OnceLock<TranslateMetadataFn> = OnceLock::new();
static SPONSORBLOCK_FN: OnceLock<SponsorBlockFn> = OnceLock::new();
static SPLIT_CHAPTERS_FN: OnceLock<SplitChaptersFn> = OnceLock::new();
static EMBED_METADATA_FN: OnceLock<EmbedMetadataFn> = OnceLock::new();
static EMBED_THUMBNAIL_FN: OnceLock<EmbedThumbnailFn> = OnceLock::new();
static SPEED_LIMIT_FN: OnceLock<SpeedLimitFn> = OnceLock::new();
static LIVE_FROM_START_FN: OnceLock<LiveFromStartFn> = OnceLock::new();
static INSECURE_TLS_FN: OnceLock<InsecureTlsFn> = OnceLock::new();
static CONCURRENT_FRAGMENTS_FN: OnceLock<ConcurrentFragmentsFn> = OnceLock::new();
static USER_AGENT_FN: OnceLock<UserAgentFn> = OnceLock::new();
static SPONSORBLOCK_MODE_FN: OnceLock<SponsorBlockModeFn> = OnceLock::new();
static SPONSORBLOCK_CATEGORIES_FN: OnceLock<SponsorBlockCategoriesFn> = OnceLock::new();

/// Protocolos que podem ser delegados ao aria2c. Manifesto fragmentado
/// (`m3u8*`, `dash_frag_urls`) fica de fora por causa da CVE-2026-50574 — o
/// yt-dlp removeu esse suporte na 2026.06.09 e recomenda `-N` no lugar.
pub(crate) const ARIA2C_ALLOWED_PROTOCOLS: &str = "http,ftp";

/// Ordem de fallback de `player_client` quando o YouTube devolve formato
/// SABR-only.
///
/// O extractor `web` passou a entregar formatos que o caminho normal de
/// download nao consegue puxar. Estes quatro clients ainda servem URL
/// progressiva; a ordem vai do mais confiavel ao mais restrito.
/// `tv_downgraded` vem primeiro porque é o fallback que o próprio yt-dlp usa
/// quando `visionos`/`web` falham (README 2026-08); `android` fica depois de
/// `ios` porque desde 2026-07 o YouTube exige PO token nele de forma
/// intermitente (comentário em `youtube/_base.py`).
pub(crate) const SABR_CLIENT_CASCADE: [&str; 4] = ["tv_downgraded", "ios", "android", "web_safari"];

/// Um `format_id` com `/` já é um seletor completo com as próprias
/// alternativas (o worker MCP manda assim); um id simples (`137`) não.
pub(crate) fn is_full_format_selector(format_id: &str) -> bool {
    format_id.contains('/')
}

/// Seletor `-f` do download. Um seletor completo passa intacto.
pub(crate) fn download_format_selector(
    format_id: Option<&str>,
    quality_height: Option<u32>,
    mode: &str,
    ffmpeg_available: bool,
) -> String {
    if let Some(fid) = format_id {
        if is_full_format_selector(fid) {
            // Seletor completo (o worker MCP manda o seu, com teto em toda
            // alternativa): anexar o fallback daqui punha um `/b` sem teto no
            // fim (benchmark 26/09, youtube-2).
            fid.to_string()
        } else if let Some(h) = quality_height.filter(|h| *h > 0) {
            let fallback = match mode {
                "audio" => "ba/b".to_string(),
                "mute" => format!("bv*[height<={}]/bv*/b", h),
                _ => {
                    if ffmpeg_available {
                        format!(
                            "bv*[height<={}]+ba[ext=m4a]/bv*[height<={}]+ba/b[height<={}]/b",
                            h, h, h
                        )
                    } else {
                        format!("b[height<={}]/bv*[height<={}]/b", h, h)
                    }
                }
            };
            format!("{}/{}", fid, fallback)
        } else {
            fid.to_string()
        }
    } else {
        match mode {
            "audio" => "ba/b".to_string(),
            "mute" => match quality_height {
                Some(h) if h > 0 => format!("bv*[height<={}]/bv*/b", h),
                _ => "bv*/b".to_string(),
            },
            _ => {
                if ffmpeg_available {
                    match quality_height {
                        Some(h) if h > 0 => format!(
                            "bv*[height<={}]+ba[ext=m4a]/bv*[height<={}]+ba/b[height<={}]/b",
                            h, h, h
                        ),
                        _ => "bv*+ba[ext=m4a]/bv*+ba/b".to_string(),
                    }
                } else {
                    tracing::warn!("[yt-dlp] ffmpeg not available, using fallback format selector");
                    match quality_height {
                        Some(h) if h > 0 => format!("b[height<={}]/bv*[height<={}]/b", h, h),
                        _ => "b/bv*".to_string(),
                    }
                }
            }
        }
    }
}

/// `formats=dashy` rebaixa os formatos `https` do YouTube a
/// `http_dash_segments`, que o yt-dlp ordena abaixo de `m3u8`: na mesma
/// resolução o HLS (MPEG-TS) passava a ganhar do DASH, e a fusão falhava
/// (benchmark 26/09, youtube-2 pegou 232 em vez de 136). Resolução e fps
/// continuam na frente; o protocolo só desempata.
pub(crate) fn dashy_format_sort_args() -> Vec<String> {
    vec![
        "-S".to_string(),
        "res,fps,hdr:12,proto:http_dash_segments".to_string(),
    ]
}

/// `Some(arg)` quando o stderr indica formato SABR-only e ainda resta client na
/// cascata; `None` quando nao e SABR ou a cascata se esgotou.
///
/// Separado do laco de retry de proposito: e o unico jeito de testar a decisao
/// com stderr real capturado como fixture, sem subir um download.
pub(crate) fn sabr_fallback_client(stderr_lower: &str, attempt: usize) -> Option<String> {
    if !stderr_lower.contains("sabr") {
        return None;
    }
    SABR_CLIENT_CASCADE
        .get(attempt)
        .map(|client| format!("youtube:player_client={client}"))
}

/// Piso de versão do yt-dlp. 2026.06.09 é a release que corrigiu
/// CVE-2026-50019 (vazamento de cookie no downloader curl), CVE-2026-50023
/// (escrita de `.desktop`/`.url`/`.webloc`) e CVE-2026-50574 (execução
/// arbitrária via manifesto com aria2c).
pub const MIN_YTDLP_VERSION: (u32, u32, u32) = (2026, 6, 9);

/// yt-dlp versiona como `YYYY.MM.DD` com sufixo opcional (`.N`, `-nightly…`).
/// Devolve `None` para qualquer coisa que não case com esse formato.
pub fn parse_ytdlp_version(raw: &str) -> Option<(u32, u32, u32)> {
    let head = raw.split_whitespace().next()?;
    let core = head.split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let year: u32 = parts.next()?.parse().ok()?;
    let month: u32 = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || year < 2000 {
        return None;
    }
    Some((year, month, day))
}

/// `None` quando a versão não pôde ser lida — nesse caso o chamador não deve
/// afirmar que está desatualizada.
pub fn ytdlp_version_is_supported(raw: &str) -> Option<bool> {
    parse_ytdlp_version(raw).map(|v| v >= MIN_YTDLP_VERSION)
}

/// Argumentos que delegam o download ao aria2c **apenas** para os protocolos
/// de `ARIA2C_ALLOWED_PROTOCOLS`. Manifesto fragmentado continua no downloader
/// nativo (CVE-2026-50574).
///
/// O proxy é validado antes de entrar na string de `--downloader-args`: um
/// espaço ali viraria argumento extra do aria2c.
pub(crate) fn aria2c_downloader_args(
    aria2c_path: &str,
    connections: u32,
    proxy: Option<&str>,
) -> Vec<String> {
    let conns = connections.max(1);
    let proxy_arg = match proxy.map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) if !p.contains(char::is_whitespace) => format!(" --all-proxy={}", p),
        Some(_) => {
            tracing::warn!("[aria2c] proxy com espaço ignorado no --downloader-args");
            String::new()
        }
        None => String::new(),
    };
    vec![
        "--downloader".to_string(),
        format!("{}:{}", ARIA2C_ALLOWED_PROTOCOLS, aria2c_path),
        "--downloader-args".to_string(),
        format!(
            "aria2c:-x {conns} -k 1M -j {conns} --min-split-size=1M \
             --file-allocation=none --optimize-concurrent-downloads=true \
             --auto-file-renaming=false --summary-interval=1 \
             --console-log-level=notice{proxy_arg}"
        ),
    ]
}

/// Argumentos do extractor `youtube` reunidos numa única flag.
///
/// O yt-dlp **substitui** o dicionário de um extractor a cada
/// `--extractor-args youtube:…` repetido (`options.py`,
/// `_dict_from_options_callback` com `append=False`). Antes, o
/// `youtube:lang=` de "traduzir metadados" apagava o `player_client`, e a
/// rotação de client dos retries apagava o `lang`. Agora tudo do YouTube sai
/// de um só lugar; os retries mudam só o campo que importa.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct YoutubeExtractorArgs {
    pub player_client: Option<String>,
    /// `formats=dashy`: transforma formato `https` em DASH fragmentado para o
    /// `-N` valer. Sem isso o `-N` no YouTube não faz nada, porque os formatos
    /// comuns (137/299/140…) descem por uma conexão em pedaços de 10 MB
    /// (estudo 55, benchmark 66).
    pub formats_dashy: bool,
    pub lang: Option<String>,
}

impl YoutubeExtractorArgs {
    /// Aceita `youtube:player_client=ios` ou só `ios`.
    pub(crate) fn set_client_from_flag(&mut self, flag_value: &str) {
        let client = flag_value.rsplit('=').next().unwrap_or(flag_value).trim();
        if !client.is_empty() {
            self.player_client = Some(client.to_string());
        }
    }

    pub(crate) fn to_flags(&self) -> Vec<String> {
        let mut parts: Vec<String> = Vec::new();
        if let Some(c) = &self.player_client {
            parts.push(format!("player_client={c}"));
        }
        if self.formats_dashy {
            parts.push("formats=dashy".to_string());
        }
        if let Some(l) = &self.lang {
            parts.push(format!("lang={l}"));
        }
        if parts.is_empty() {
            return Vec::new();
        }
        vec![
            "--extractor-args".to_string(),
            format!("youtube:{}", parts.join(";")),
        ]
    }
}

/// Registro do último comando rodado para um download: é o que a tela mostra
/// em "Comando" e o que a caixa "editar e tentar de novo" pré-preenche.
#[derive(Debug, Clone, Serialize)]
pub struct CommandRecord {
    pub program: String,
    /// argv já redigido (cookie, senha, proxy com credencial, po_token).
    pub args: Vec<String>,
    /// `programa args…` citado para shell, pronto para copiar ou editar.
    pub display: String,
    pub attempt: u32,
    pub max_attempts: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub player_client: Option<String>,
    /// `-N` efetivo desta tentativa (0 quando o comando veio do usuário).
    pub connections: u32,
    /// `native`, `aria2c` ou `custom`.
    pub engine: String,
    /// `true` quando o argv veio da caixa de edição do usuário.
    pub overridden: bool,
}

static COMMAND_BY_DOWNLOAD: OnceLock<Mutex<HashMap<u64, CommandRecord>>> = OnceLock::new();

fn command_map() -> &'static Mutex<HashMap<u64, CommandRecord>> {
    COMMAND_BY_DOWNLOAD.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn get_command(download_id: u64) -> Option<CommandRecord> {
    command_map().lock().ok()?.get(&download_id).cloned()
}

pub fn clear_command(download_id: u64) {
    if let Ok(mut m) = command_map().lock() {
        m.remove(&download_id);
    }
}

/// Operator-only receiver for every yt-dlp command a process without a
/// download id runs (the MCP worker). Before this, a worker failure left no
/// trace of the argv that produced it (benchmark 26/09, open problem 4).
pub type CommandSink = Arc<dyn Fn(&str) + Send + Sync + 'static>;

static COMMAND_SINK: Mutex<Option<CommandSink>> = Mutex::new(None);

/// Installs (or clears) the operator command sink. The worker installs one
/// that emits a `diagnostic` event; the desktop keeps the per-download log.
pub fn set_command_sink(sink: Option<CommandSink>) {
    if let Ok(mut s) = COMMAND_SINK.lock() {
        *s = sink;
    }
}

/// Redacted one-line form of an argv for the operator log: no cookies, auth
/// or CSRF headers, passwords, usernames, proxy credentials or PO tokens.
/// In worker mode the fixed worker flags are appended as they are run.
pub(crate) fn operator_command_line(program: &Path, args: &[String], stage: &str) -> String {
    let program_name = program
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .map(|n| n.trim_end_matches(".pyz").to_string())
        .unwrap_or_else(|| "yt-dlp".to_string());
    let mut argv: Vec<String> = args.to_vec();
    argv.extend(
        worker_retry_args(crate::core::dependencies::worker_mode())
            .into_iter()
            .map(String::from),
    );
    let mut parts = vec![program_name];
    parts.extend(super::shell_words::redact_for_log(&operator_order(&argv)));
    format!("[{stage}] $ {}", super::shell_words::join(&parts))
}

/// Flags whose value the operator-log redactor (flight_recorder) treats as the
/// start of a secret and cuts to the end of the line. Moving them last keeps the
/// rest of the command (extractor-args, URL) readable in the diagnostic.
const LOG_TRAILING_FLAGS: &[&str] = &[
    "--proxy",
    "--all-proxy",
    "--cookies",
    "--add-header",
    "--username",
    "--password",
];

fn operator_order(argv: &[String]) -> Vec<String> {
    let (mut head, mut tail) = (Vec::new(), Vec::new());
    let mut i = 0;
    while i < argv.len() {
        if LOG_TRAILING_FLAGS.contains(&argv[i].as_str()) {
            tail.push(argv[i].clone());
            if let Some(v) = argv.get(i + 1) {
                tail.push(v.clone());
            }
            i += 2;
        } else {
            head.push(argv[i].clone());
            i += 1;
        }
    }
    head.extend(tail);
    head
}

/// Sends the command to the operator sink when no per-download log owns it.
fn note_operator_command(program: &Path, args: &[String], stage: &str) {
    if log_hook::current_download_id().is_some() {
        return;
    }
    let sink = COMMAND_SINK.lock().ok().and_then(|s| s.clone());
    if let Some(sink) = sink {
        sink(&operator_command_line(program, args, stage));
    }
}

#[allow(clippy::too_many_arguments)]
fn record_command(
    program: &Path,
    args: &[String],
    attempt: u32,
    max_attempts: u32,
    player_client: Option<String>,
    connections: u32,
    engine: &str,
    overridden: bool,
) {
    let Some(id) = log_hook::current_download_id() else {
        note_operator_command(program, args, engine);
        return;
    };
    let program_name = program
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .map(|n| n.trim_end_matches(".pyz").to_string())
        .unwrap_or_else(|| "yt-dlp".to_string());
    let redacted = super::shell_words::redact(args);
    let mut display_parts: Vec<String> = Vec::with_capacity(redacted.len() + 1);
    display_parts.push(program_name.clone());
    display_parts.extend(redacted.iter().cloned());
    let display = super::shell_words::join(&display_parts);
    log_hook::emit_log(id, &format!("[omniget] $ {display}"));
    let rec = CommandRecord {
        program: program_name,
        args: redacted,
        display,
        attempt,
        max_attempts,
        player_client,
        connections,
        engine: engine.to_string(),
        overridden,
    };
    if let Ok(mut m) = command_map().lock() {
        m.insert(id, rec);
    }
}

/// `-N` de partida por host. Oito é onde o benchmark (estudo 66) encostou no
/// teto do link; o tuner sobe a partir daí só com ganho medido.
const DEFAULT_START_N: u32 = 8;

/// Teto do YouTube sem 429 na sessão. Dezesseis é o que os mantenedores
/// recomendam com `formats=dashy` (#7987); acima disso o ganho medido é zero
/// e a chance de 429 sobe.
const MAX_N_YOUTUBE: u32 = 16;

static LAST_N_BY_HOST: OnceLock<Mutex<HashMap<String, u32>>> = OnceLock::new();

fn last_n_map() -> &'static Mutex<HashMap<String, u32>> {
    LAST_N_BY_HOST.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Chave de host para o tuner: `googlevideo.com`, `youtu.be` e `youtube.com`
/// são o mesmo limite de taxa, então viram uma chave só.
pub(crate) fn tuner_host_key(url: &str) -> String {
    let host = url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase()))
        .unwrap_or_else(|| "unknown".to_string());
    let host = host.trim_start_matches("www.").to_string();
    if host.contains("youtube.com") || host.contains("youtu.be") || host.contains("googlevideo.com")
    {
        return "youtube".to_string();
    }
    host
}

/// `-N` desta tentativa: parte de `min(8, teto)` e deixa o tuner subir ou
/// descer pelo histórico do host. O número da Config é **teto**, não valor.
async fn tuned_concurrency(host: &str, ceiling: u32) -> u32 {
    let ceiling = ceiling.max(1);
    let current = last_n_map()
        .lock()
        .ok()
        .and_then(|m| m.get(host).copied())
        .unwrap_or_else(|| DEFAULT_START_N.min(ceiling));
    let suggested = super::adaptive_concurrency::global()
        .lock()
        .await
        .suggest(host, current, ceiling);
    suggested.max(1)
}

fn remember_concurrency(host: &str, n: u32) {
    if let Ok(mut m) = last_n_map().lock() {
        m.insert(host.to_string(), n);
    }
}

/// Registra a vazão observada. Só amostras que significam alguma coisa: menos
/// de 3 s ou de 5 MB é ruído de conexão, não capacidade.
async fn record_throughput(host: &str, n: u32, bytes: u64, secs: f64) {
    if secs < 3.0 || bytes < 5 * 1024 * 1024 {
        return;
    }
    let bps = bytes as f64 / secs;
    super::adaptive_concurrency::global()
        .lock()
        .await
        .record(host, n, bps);
    tracing::debug!(
        "[tuner] {} n={} throughput={:.0} B/s ({} bytes em {:.1}s)",
        host,
        n,
        bps,
        bytes,
        secs
    );
}

async fn record_host_rate_limit(host: &str) {
    super::adaptive_concurrency::global()
        .lock()
        .await
        .record_rate_limit(host);
}

/// Template de progresso. Além do progresso, pede `info.*` do formato em
/// download (o yt-dlp avalia o template sobre `{'info': format_dict,
/// 'progress': …}`, `downloader/common.py`), o que dá resolução, codec,
/// container e tamanho de cada stream sem um segundo processo.
pub(crate) const PROGRESS_TEMPLATE: &str = "download:%(progress._percent_str)s|eta:%(progress.eta)s|spd:%(progress.speed)s|dl:%(progress.downloaded_bytes)s|tot:%(progress.total_bytes)s|est:%(progress.total_bytes_estimate)s|fi:%(progress.fragment_index)s|fc:%(progress.fragment_count)s|el:%(progress.elapsed)s|fid:%(info.format_id)s|h:%(info.height)s|w:%(info.width)s|fps:%(info.fps)s|vc:%(info.vcodec)s|ac:%(info.acodec)s|ext:%(info.ext)s|fsz:%(info.filesize,info.filesize_approx)s|note:%(info.format_note)s";

/// Normaliza o argv escrito pelo usuário: tira o nome do programa, garante a
/// instrumentação que a tela precisa (`--newline`, `--progress`, template,
/// `--print after_move:…`, `--no-quiet`), um `-o` quando falta e a URL no
/// fim. Nada mais é alterado: o usuário pediu para rodar *aquele* comando.
pub(crate) fn prepare_override_args(
    argv: Vec<String>,
    url: &str,
    output_dir: &Path,
) -> Vec<String> {
    let mut args = argv;
    if let Some(first) = args.first() {
        let name = Path::new(first)
            .file_name()
            .map(|s| s.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if name.starts_with("yt-dlp") || name.starts_with("youtube-dl") {
            args.remove(0);
        }
    }
    let has = |flag: &str, args: &[String]| {
        args.iter()
            .any(|a| a == flag || a.starts_with(&format!("{flag}=")))
    };
    if !has("--newline", &args) {
        args.push("--newline".to_string());
    }
    if !has("--progress", &args) {
        args.push("--progress".to_string());
    }
    if !has("--no-quiet", &args) {
        args.push("--no-quiet".to_string());
    }
    if !has("--progress-template", &args) {
        args.push("--progress-template".to_string());
        args.push(PROGRESS_TEMPLATE.to_string());
    }
    if !args
        .iter()
        .any(|a| a.starts_with("after_move:OMNIGET_FILEPATH"))
    {
        args.push("--print".to_string());
        args.push("after_move:OMNIGET_FILEPATH:%(filepath)s".to_string());
    }
    let has_output = ["-o", "--output", "-P", "--paths"]
        .iter()
        .any(|f| has(f, &args));
    if !has_output {
        args.push("-o".to_string());
        args.push(
            output_dir
                .join("%(title).200s [%(id)s].%(ext)s")
                .to_string_lossy()
                .to_string(),
        );
    }
    if !args.iter().any(|a| a == url) {
        args.push(url.to_string());
    }
    args
}

pub fn set_ext_cookie_path_fn(f: impl Fn() -> PathBuf + Send + Sync + 'static) {
    let _ = EXT_COOKIE_PATH_FN.set(Box::new(f));
}

pub fn set_per_domain_cookie_fn(f: impl Fn(&str) -> Option<PathBuf> + Send + Sync + 'static) {
    let _ = PER_DOMAIN_COOKIE_FN.set(Box::new(f));
}

pub fn set_managed_cookies_only_fn(f: impl Fn() -> bool + Send + Sync + 'static) {
    let _ = MANAGED_COOKIES_ONLY_FN.set(Box::new(f));
}

fn managed_cookies_only() -> bool {
    MANAGED_COOKIES_ONLY_FN.get().map(|f| f()).unwrap_or(true)
}

pub fn set_global_cookie_file_fn(f: impl Fn() -> Option<String> + Send + Sync + 'static) {
    let _ = GLOBAL_COOKIE_FILE_FN.set(Box::new(f));
}

pub fn set_cookies_from_browser_fn(f: impl Fn() -> String + Send + Sync + 'static) {
    let _ = COOKIES_FROM_BROWSER_FN.set(Box::new(f));
}

pub fn set_manual_cookie_header_fn(f: impl Fn() -> String + Send + Sync + 'static) {
    let _ = MANUAL_COOKIE_HEADER_FN.set(Box::new(f));
}

pub fn set_ext_referer_fn(f: impl Fn(&str) -> Option<String> + Send + Sync + 'static) {
    let _ = EXT_REFERER_FN.set(Box::new(f));
}

pub fn set_include_auto_subs_fn(f: impl Fn() -> bool + Send + Sync + 'static) {
    let _ = INCLUDE_AUTO_SUBS_FN.set(Box::new(f));
}

fn include_auto_subs_setting() -> bool {
    INCLUDE_AUTO_SUBS_FN.get().map(|f| f()).unwrap_or(false)
}

pub fn set_caption_locale_fn(f: impl Fn() -> String + Send + Sync + 'static) {
    let _ = CAPTION_LOCALE_FN.set(Box::new(f));
}

fn caption_locale_setting() -> String {
    let lang = CAPTION_LOCALE_FN
        .get()
        .map(|f| f())
        .unwrap_or_else(|| "en".to_string());
    let lang = lang.trim();
    if lang.is_empty() {
        "en".to_string()
    } else {
        lang.to_string()
    }
}

fn requested_caption_locales() -> Vec<String> {
    caption_locale_setting()
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .collect()
}

pub fn set_keep_vtt_fn(f: impl Fn() -> bool + Send + Sync + 'static) {
    let _ = KEEP_VTT_FN.set(Box::new(f));
}

fn keep_vtt_setting() -> bool {
    KEEP_VTT_FN.get().map(|f| f()).unwrap_or(false)
}

pub fn set_translate_metadata_fn(f: impl Fn() -> Option<String> + Send + Sync + 'static) {
    let _ = TRANSLATE_METADATA_FN.set(Box::new(f));
}

fn translate_metadata_lang() -> Option<String> {
    TRANSLATE_METADATA_FN.get().and_then(|f| f())
}

static ACTIVE_PROCESS_PIDS: OnceLock<std::sync::Mutex<HashMap<u64, u32>>> = OnceLock::new();

fn active_process_pids() -> &'static std::sync::Mutex<HashMap<u64, u32>> {
    ACTIVE_PROCESS_PIDS.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

fn register_download_process(download_id: u64, pid: u32) {
    if let Ok(mut pids) = active_process_pids().lock() {
        pids.insert(download_id, pid);
    }
}

fn unregister_download_process(download_id: u64) {
    if let Ok(mut pids) = active_process_pids().lock() {
        pids.remove(&download_id);
    }
}

#[cfg(unix)]
fn signal_download_process(download_id: u64, signal: &str) -> bool {
    let pid = active_process_pids()
        .lock()
        .ok()
        .and_then(|pids| pids.get(&download_id).copied());
    let Some(pid) = pid else {
        return false;
    };
    std::process::Command::new("kill")
        .arg(signal)
        .arg(pid.to_string())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

// Issue #307: pausing on Windows always reported "failed to pause" because
// this arm was a stub returning false. Windows has no SIGSTOP; the equivalent
// is NtSuspendProcess/NtResumeProcess from ntdll (what Process Explorer uses).
// They are undocumented but ABI-stable since XP; resolved via GetProcAddress
// so nothing breaks at link time if they ever disappear.
#[cfg(windows)]
fn signal_download_process(download_id: u64, signal: &str) -> bool {
    use std::ffi::c_void;
    use std::os::raw::{c_char, c_int};

    type Handle = *mut c_void;

    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(desired_access: u32, inherit_handle: c_int, process_id: u32) -> Handle;
        fn CloseHandle(handle: Handle) -> c_int;
        fn GetModuleHandleA(module_name: *const c_char) -> Handle;
        fn GetProcAddress(module: Handle, proc_name: *const c_char) -> *mut c_void;
    }

    const PROCESS_SUSPEND_RESUME: u32 = 0x0800;

    let pid = active_process_pids()
        .lock()
        .ok()
        .and_then(|pids| pids.get(&download_id).copied());
    let Some(pid) = pid else {
        return false;
    };

    let suspend = signal == "-STOP";
    unsafe {
        let ntdll = GetModuleHandleA(c"ntdll.dll".as_ptr());
        if ntdll.is_null() {
            return false;
        }
        let proc_name = if suspend {
            c"NtSuspendProcess"
        } else {
            c"NtResumeProcess"
        };
        let addr = GetProcAddress(ntdll, proc_name.as_ptr());
        if addr.is_null() {
            return false;
        }
        let func: extern "system" fn(Handle) -> i32 = std::mem::transmute(addr);
        let handle = OpenProcess(PROCESS_SUSPEND_RESUME, 0, pid);
        if handle.is_null() {
            return false;
        }
        let status = func(handle);
        CloseHandle(handle);
        status >= 0
    }
}

#[cfg(not(any(unix, windows)))]
fn signal_download_process(_download_id: u64, _signal: &str) -> bool {
    false
}

pub fn pause_download_process(download_id: u64) -> bool {
    signal_download_process(download_id, "-STOP")
}

pub fn resume_download_process(download_id: u64) -> bool {
    signal_download_process(download_id, "-CONT")
}

fn normalize_youtube_lang(lang: &str) -> String {
    match lang.trim() {
        "zh" | "zh-CN" | "zh-cn" | "zh-Hans" | "zh-hans" => "zh-CN".to_string(),
        "zh-TW" | "zh-tw" | "zh-Hant" | "zh-hant" => "zh-TW".to_string(),
        "zh-HK" | "zh-hk" => "zh-HK".to_string(),
        other => other.to_string(),
    }
}

pub fn set_sponsorblock_fn(f: impl Fn() -> bool + Send + Sync + 'static) {
    let _ = SPONSORBLOCK_FN.set(Box::new(f));
}

fn sponsorblock_enabled() -> bool {
    SPONSORBLOCK_FN.get().map(|f| f()).unwrap_or(false)
}

pub fn set_sponsorblock_mode_fn(f: impl Fn() -> String + Send + Sync + 'static) {
    let _ = SPONSORBLOCK_MODE_FN.set(Box::new(f));
}

fn sponsorblock_mode() -> String {
    SPONSORBLOCK_MODE_FN
        .get()
        .map(|f| f())
        .unwrap_or_else(|| "remove".to_string())
}

pub fn set_sponsorblock_categories_fn(f: impl Fn() -> Vec<String> + Send + Sync + 'static) {
    let _ = SPONSORBLOCK_CATEGORIES_FN.set(Box::new(f));
}

fn sponsorblock_categories() -> Vec<String> {
    SPONSORBLOCK_CATEGORIES_FN
        .get()
        .map(|f| f())
        .unwrap_or_default()
}

pub fn set_split_chapters_fn(f: impl Fn() -> bool + Send + Sync + 'static) {
    let _ = SPLIT_CHAPTERS_FN.set(Box::new(f));
}

fn split_chapters_enabled() -> bool {
    SPLIT_CHAPTERS_FN.get().map(|f| f()).unwrap_or(false)
}

pub fn set_embed_metadata_fn(f: impl Fn() -> bool + Send + Sync + 'static) {
    let _ = EMBED_METADATA_FN.set(Box::new(f));
}

fn embed_metadata_enabled() -> bool {
    EMBED_METADATA_FN.get().map(|f| f()).unwrap_or(true)
}

pub fn set_embed_thumbnail_fn(f: impl Fn() -> bool + Send + Sync + 'static) {
    let _ = EMBED_THUMBNAIL_FN.set(Box::new(f));
}

fn embed_thumbnail_enabled() -> bool {
    EMBED_THUMBNAIL_FN.get().map(|f| f()).unwrap_or(true)
}

pub fn set_speed_limit_fn(f: impl Fn() -> Option<String> + Send + Sync + 'static) {
    let _ = SPEED_LIMIT_FN.set(Box::new(f));
}

fn speed_limit_value() -> Option<String> {
    SPEED_LIMIT_FN.get().and_then(|f| f())
}

pub fn set_insecure_tls_fn(f: impl Fn() -> bool + Send + Sync + 'static) {
    let _ = INSECURE_TLS_FN.set(Box::new(f));
}

/// Verificar certificado é o default. `Vec` vazio quando ligado, para o
/// chamador poder simplesmente estender a lista de argumentos.
pub fn insecure_tls_args() -> Vec<String> {
    if INSECURE_TLS_FN.get().map(|f| f()).unwrap_or(false) {
        vec!["--no-check-certificates".to_string()]
    } else {
        Vec::new()
    }
}

pub fn set_live_from_start_fn(f: impl Fn() -> bool + Send + Sync + 'static) {
    let _ = LIVE_FROM_START_FN.set(Box::new(f));
}

fn live_from_start_enabled() -> bool {
    LIVE_FROM_START_FN.get().map(|f| f()).unwrap_or(false)
}

pub fn set_concurrent_fragments_fn(f: impl Fn() -> u32 + Send + Sync + 'static) {
    let _ = CONCURRENT_FRAGMENTS_FN.set(Box::new(f));
}

fn concurrent_fragments_value() -> u32 {
    CONCURRENT_FRAGMENTS_FN.get().map(|f| f()).unwrap_or(1)
}

pub fn set_user_agent_fn(f: impl Fn() -> Option<String> + Send + Sync + 'static) {
    let _ = USER_AGENT_FN.set(Box::new(f));
}

fn user_agent_setting() -> Option<String> {
    USER_AGENT_FN.get().and_then(|f| f())
}

static EXT_UA_MAP: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

fn ext_ua_map() -> &'static Mutex<HashMap<String, String>> {
    EXT_UA_MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn register_ext_user_agent(url: String, ua: String) {
    if let Ok(mut map) = ext_ua_map().lock() {
        map.insert(url, ua);
    }
}

pub fn clear_ext_user_agent(url: &str) {
    if let Ok(mut map) = ext_ua_map().lock() {
        map.remove(url);
    }
}

fn ext_user_agent_for_url(url: &str) -> Option<String> {
    ext_ua_map().lock().ok().and_then(|m| m.get(url).cloned())
}

static ETA_BY_DOWNLOAD: OnceLock<Mutex<HashMap<u64, u64>>> = OnceLock::new();

fn eta_map() -> &'static Mutex<HashMap<u64, u64>> {
    ETA_BY_DOWNLOAD.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn record_eta(download_id: u64, eta_seconds: u64) {
    if let Ok(mut m) = eta_map().lock() {
        m.insert(download_id, eta_seconds);
    }
}

pub fn get_eta(download_id: u64) -> Option<u64> {
    eta_map()
        .lock()
        .ok()
        .and_then(|m| m.get(&download_id).copied())
}

pub fn clear_eta(download_id: u64) {
    if let Ok(mut m) = eta_map().lock() {
        m.remove(&download_id);
    }
}

static EXT_HEADERS_MAP: OnceLock<Mutex<HashMap<String, HashMap<String, String>>>> = OnceLock::new();

fn ext_headers_map() -> &'static Mutex<HashMap<String, HashMap<String, String>>> {
    EXT_HEADERS_MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn register_ext_headers(url: String, headers: HashMap<String, String>) {
    if let Ok(mut map) = ext_headers_map().lock() {
        map.insert(url, headers);
    }
}

pub fn clear_ext_headers(url: &str) {
    if let Ok(mut map) = ext_headers_map().lock() {
        map.remove(url);
    }
}

fn ext_headers_for_url(url: &str) -> Option<HashMap<String, String>> {
    ext_headers_map()
        .lock()
        .ok()
        .and_then(|m| m.get(url).cloned())
}

fn ext_referer_for_url(url: &str) -> Option<String> {
    EXT_REFERER_FN.get().and_then(|f| f(url))
}

fn cookies_from_browser_setting() -> String {
    COOKIES_FROM_BROWSER_FN
        .get()
        .map(|f| f())
        .unwrap_or_default()
}

fn manual_cookie_header_setting() -> Option<String> {
    let raw = MANUAL_COOKIE_HEADER_FN
        .get()
        .map(|f| f())
        .unwrap_or_default();
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let parsed = crate::core::cookie_parser::parse_cookie_input(trimmed, "");
    if !parsed.cookie_string.trim().is_empty() {
        Some(parsed.cookie_string)
    } else {
        Some(trimmed.to_string())
    }
}

pub fn ext_cookie_path() -> PathBuf {
    EXT_COOKIE_PATH_FN
        .get()
        .map(|f| f())
        .unwrap_or_else(|| PathBuf::from(""))
}

pub fn ext_cookie_path_if_fresh() -> Option<PathBuf> {
    let source = ext_cookie_path();
    if !source.exists() {
        return None;
    }
    let metadata = std::fs::metadata(&source).ok()?;
    let modified = metadata.modified().ok()?;
    if modified.elapsed().unwrap_or_default() >= std::time::Duration::from_secs(604800) {
        return None;
    }
    Some(source)
}

fn global_cookie_file() -> Option<String> {
    GLOBAL_COOKIE_FILE_FN.get().and_then(|f| f())
}

static YTDLP_UPDATING: AtomicBool = AtomicBool::new(false);
static YTDLP_UPDATE_CHECKED: AtomicBool = AtomicBool::new(false);
static YTDLP_PATH_CACHE: std::sync::RwLock<Option<Option<PathBuf>>> = std::sync::RwLock::new(None);
static FFMPEG_LOCATION_CACHE: std::sync::RwLock<Option<Option<String>>> =
    std::sync::RwLock::new(None);
static JS_RUNTIME_CACHE: std::sync::RwLock<Option<Option<String>>> = std::sync::RwLock::new(None);
static RATE_LIMIT_429_COUNT: AtomicU64 = AtomicU64::new(0);
static RATE_LIMIT_429_LAST_TS: AtomicU64 = AtomicU64::new(0);
static COOKIE_ERROR_FLAG: AtomicBool = AtomicBool::new(false);

pub fn has_cookie_error() -> bool {
    COOKIE_ERROR_FLAG.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn clear_cookie_error() {
    COOKIE_ERROR_FLAG.store(false, std::sync::atomic::Ordering::Relaxed);
}

fn rate_limit_429_count() -> u64 {
    let last = RATE_LIMIT_429_LAST_TS.load(Ordering::Relaxed);
    if last == 0 {
        return 0;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    if now.saturating_sub(last) > 1800 {
        RATE_LIMIT_429_COUNT.store(0, Ordering::Relaxed);
        RATE_LIMIT_429_LAST_TS.store(0, Ordering::Relaxed);
        return 0;
    }
    RATE_LIMIT_429_COUNT.load(Ordering::Relaxed)
}

fn rate_limit_429_increment() {
    RATE_LIMIT_429_COUNT.fetch_add(1, Ordering::Relaxed);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    RATE_LIMIT_429_LAST_TS.store(now, Ordering::Relaxed);
}

pub fn reset_ytdlp_cache() {
    if let Ok(mut cache) = YTDLP_PATH_CACHE.write() {
        *cache = None;
    }
}

pub fn reset_ffmpeg_location_cache() {
    if let Ok(mut cache) = FFMPEG_LOCATION_CACHE.write() {
        *cache = None;
    }
}

pub fn reset_js_runtime_cache() {
    if let Ok(mut cache) = JS_RUNTIME_CACHE.write() {
        *cache = None;
    }
}

/// Alvos de `--impersonate` deste binario de yt-dlp, consultados uma vez so.
///
/// A lista nao muda durante a execucao e a consulta custa um processo; sem o
/// cache, todo retry por fingerprint pagaria de novo.
static IMPERSONATE_TARGETS: OnceLock<Option<Vec<super::impersonation::ImpersonateTarget>>> =
    OnceLock::new();

async fn impersonate_targets() -> Option<Vec<super::impersonation::ImpersonateTarget>> {
    if let Some(cached) = IMPERSONATE_TARGETS.get() {
        return cached.clone();
    }

    let bin = ensure_ytdlp().await.ok()?;
    let _slot = acquire_ytdlp_slot("impersonate targets").await;
    let output = tokio::task::spawn_blocking(move || {
        ytdlp_std_command(&bin)
            .arg("--list-impersonate-targets")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
    })
    .await
    .ok()?
    .ok()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let targets = super::impersonation::parse_targets(&stdout);
    // Lista vazia e ausencia sao a mesma coisa para quem chama: nao ha o que
    // tentar. Guardar `None` evita repetir a consulta num build sem curl_cffi.
    let value = if targets.is_empty() {
        None
    } else {
        Some(targets)
    };
    let _ = IMPERSONATE_TARGETS.set(value.clone());
    value
}

/// URL do provedor de PO token, se o usuario tiver um rodando.
///
/// Fica em variavel de ambiente de proposito: o provedor e um servico externo
/// (Docker ou Node), a maioria dos usuarios nunca vai ter um, e um campo em
/// settings.json custaria migracao para todo mundo por causa de poucos.
/// Sonda o provedor antes de apontar o yt-dlp para ele.
///
/// O `extractor_args` so devolve argumentos para um provedor `Ready`, e essa
/// regra existe por um motivo concreto: um endereco configurado mas morto faz o
/// yt-dlp esperar o timeout em *todo* download, o que e pior do que nao ter
/// provedor. Timeout curto porque isto roda no caminho de um retry.
async fn probe_pot_provider(base_url: &str) -> super::pot_provider::ProviderHealth {
    let client = match super::http_client::apply_global_proxy(reqwest::Client::builder())
        .timeout(std::time::Duration::from_secs(3))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return super::pot_provider::ProviderHealth::Unhealthy {
                detail: e.to_string(),
            }
        }
    };

    match client.get(format!("{base_url}/ping")).send().await {
        Ok(resp) if resp.status().is_success() => {
            super::pot_provider::ProviderHealth::Ready { version: None }
        }
        Ok(resp) => super::pot_provider::ProviderHealth::Unhealthy {
            detail: format!("HTTP {}", resp.status()),
        },
        Err(_) => super::pot_provider::ProviderHealth::Unreachable {
            base_url: base_url.to_string(),
        },
    }
}

fn pot_provider_base_url() -> Option<String> {
    let raw = std::env::var("OMNIGET_POT_PROVIDER_URL").ok()?;
    super::pot_provider::normalize_base_url(&raw)
}

/// Resultado do `--update-to` do boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateCheck {
    Updated,
    UpToDate,
    /// Havia download rodando: trocar o binário agora quebraria o bootstrap
    /// do PyInstaller (B2). Chamar de novo mais tarde.
    Busy,
    AlreadyChecked,
}

pub async fn check_ytdlp_update_detailed(ytdlp: &Path) -> anyhow::Result<UpdateCheck> {
    if YTDLP_UPDATE_CHECKED.load(Ordering::Relaxed) {
        return Ok(UpdateCheck::AlreadyChecked);
    }
    if has_active_ytdlp_processes() {
        return Ok(UpdateCheck::Busy);
    }
    if YTDLP_UPDATE_CHECKED.swap(true, Ordering::Relaxed) {
        return Ok(UpdateCheck::AlreadyChecked);
    }

    let channel = match ytdlp_channel() {
        YtdlpChannel::Stable => "stable",
        YtdlpChannel::Nightly => "nightly",
    };
    let ytdlp = ytdlp.to_path_buf();
    let _slot = acquire_ytdlp_slot("update check").await;
    let output = tokio::task::spawn_blocking(move || {
        ytdlp_std_command(&ytdlp)
            .args(["--update-to", channel])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
    })
    .await??;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{}{}", stdout, stderr);

    if combined.contains("Updated yt-dlp") || combined.contains("Updating to") {
        tracing::info!("[ytdlp] updated: {}", combined.trim());
        reset_ytdlp_cache();
        Ok(UpdateCheck::Updated)
    } else {
        Ok(UpdateCheck::UpToDate)
    }
}

fn proxy_args() -> Vec<String> {
    match crate::core::http_client::proxy_url() {
        Some(url) => vec!["--proxy".to_string(), url],
        None => Vec::new(),
    }
}

fn redacted_proxy_url(url: &str) -> String {
    if let Some(at) = url.find('@') {
        if let Some(scheme_end) = url.find("://") {
            return format!("{}***{}", &url[..scheme_end + 3], &url[at..]);
        }
    }
    url.to_string()
}

fn proxy_log_label(proxy_url: Option<&str>) -> String {
    match proxy_url {
        Some(url) => format!("proxy={}", redacted_proxy_url(url)),
        None => {
            let proxy = crate::core::http_client::get_proxy_snapshot();
            if proxy.enabled && proxy.host.trim().is_empty() {
                "proxy=system/env fallback (enabled but host is empty)".to_string()
            } else if proxy.enabled {
                "proxy=system/env fallback (invalid proxy settings)".to_string()
            } else {
                "proxy=none (system/env honored if set)".to_string()
            }
        }
    }
}

fn has_explicit_cookie_header(args: &[String]) -> bool {
    args.windows(2).any(|pair| {
        pair[0] == "--add-headers" && pair[1].to_ascii_lowercase().starts_with("cookie:")
    })
}

fn append_cookie_header(args: &mut Vec<String>, cookie_header: &str) {
    args.push("--add-headers".to_string());
    args.push(format!("Cookie:{}", cookie_header));
}

enum MetadataCookieSource {
    PerDomain(PathBuf),
    ManualHeader(String),
    CookieFile(PathBuf),
    Browser(String),
    None,
    ExplicitHeader,
}

fn metadata_cookie_source(url: &str, extra_flags: &[String]) -> MetadataCookieSource {
    if has_explicit_cookie_header(extra_flags) {
        return MetadataCookieSource::ExplicitHeader;
    }

    if let Some(path) = per_domain_cookie_file(url) {
        return MetadataCookieSource::PerDomain(path);
    }

    if let Some(header) = manual_cookie_header_setting() {
        return MetadataCookieSource::ManualHeader(header);
    }

    if let Some(path) = extension_cookie_file() {
        return MetadataCookieSource::CookieFile(path);
    }

    if let Some(path) = global_cookie_file().map(PathBuf::from) {
        return MetadataCookieSource::CookieFile(path);
    }

    let browser = cookies_from_browser_setting();
    if !browser.is_empty() {
        return MetadataCookieSource::Browser(browser);
    }

    MetadataCookieSource::None
}

fn append_metadata_cookie_args(
    args: &mut Vec<String>,
    url: &str,
    extra_flags: &[String],
    context: &str,
) {
    match metadata_cookie_source(url, extra_flags) {
        MetadataCookieSource::PerDomain(path) => {
            args.push("--cookies".to_string());
            args.push(path.to_string_lossy().to_string());
            tracing::debug!("[yt-dlp] using per-domain cookies for {}", context);
        }
        MetadataCookieSource::ManualHeader(header) => {
            append_cookie_header(args, &header);
            tracing::debug!("[yt-dlp] using manual cookie header for {}", context);
        }
        MetadataCookieSource::CookieFile(path) => {
            args.push("--cookies".to_string());
            args.push(path.to_string_lossy().to_string());
        }
        MetadataCookieSource::Browser(browser) => {
            args.push("--cookies-from-browser".to_string());
            args.push(browser);
        }
        MetadataCookieSource::ExplicitHeader => {
            tracing::debug!(
                "[yt-dlp] skipping cookies-from-browser for {} because explicit Cookie header was provided",
                context
            );
        }
        MetadataCookieSource::None => {}
    }
}

struct YtRateLimiter {
    semaphore: tokio::sync::Semaphore,
    last_request_ns: AtomicU64,
}

impl YtRateLimiter {
    async fn acquire(&self) {
        let _permit = self
            .semaphore
            .acquire()
            .await
            .unwrap_or_else(|_| panic!("semaphore closed"));
        let min_interval_ns = 500_000_000u64;
        let now_ns = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        let last = self.last_request_ns.load(Ordering::Relaxed);
        let elapsed = now_ns.saturating_sub(last);
        if elapsed < min_interval_ns {
            let wait_ns = min_interval_ns - elapsed;
            let wait_duration = std::time::Duration::from_nanos(wait_ns);
            tokio::time::sleep(wait_duration).await;
        }
        self.last_request_ns.store(now_ns, Ordering::Relaxed);
    }
}

static YT_RATE_LIMITER: OnceLock<YtRateLimiter> = OnceLock::new();

/// Processos yt-dlp lançando ao mesmo tempo (B1).
///
/// O binário é PyInstaller onefile: cada lançamento extrai 72 MB para o temp
/// antes de rodar uma linha de Python. Com 6 a 16 processos simultâneos (info
/// de cada URL colada, sonda de legenda, download, `--version` do boot) o
/// bootstrap foi de 1 s para 13 a 94 s e o YouTube caía em "Timeout fetching
/// video info (90 s)". Três permits: processos curtos (info, playlist,
/// versão) seguram o permit até sair; um download segura só o bootstrap,
/// liberando na primeira linha de saída ou depois de `BOOT_SLOT_MAX_HOLD`.
pub const YTDLP_SLOTS: usize = 3;
const BOOT_SLOT_MAX_HOLD: std::time::Duration = std::time::Duration::from_secs(20);

static YTDLP_SLOT_SEMAPHORE: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();

fn ytdlp_slots() -> Arc<tokio::sync::Semaphore> {
    YTDLP_SLOT_SEMAPHORE
        .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(YTDLP_SLOTS)))
        .clone()
}

/// Permits livres agora (para testes e diagnóstico).
pub fn ytdlp_slots_available() -> usize {
    ytdlp_slots().available_permits()
}

/// Espera uma vaga para lançar um yt-dlp. A espera vai para o log do download
/// (`[omniget] waiting for a yt-dlp slot`) para a demora ter explicação.
pub async fn acquire_ytdlp_slot(what: &str) -> tokio::sync::OwnedSemaphorePermit {
    let sem = ytdlp_slots();
    if let Ok(permit) = sem.clone().try_acquire_owned() {
        return permit;
    }
    let started = std::time::Instant::now();
    let dl_id = log_hook::current_download_id();
    tracing::info!("[yt-dlp] waiting for a slot ({})", what);
    if let Some(id) = dl_id {
        log_hook::emit_log(
            id,
            &format!("[omniget] waiting for a yt-dlp slot ({})", what),
        );
    }
    let permit = sem
        .acquire_owned()
        .await
        .unwrap_or_else(|_| panic!("yt-dlp slot semaphore closed"));
    let waited = started.elapsed();
    tracing::info!("[yt-dlp] slot acquired after {:?} ({})", waited, what);
    if let Some(id) = dl_id {
        log_hook::emit_log(
            id,
            &format!(
                "[omniget] yt-dlp slot acquired after {:.1}s",
                waited.as_secs_f64()
            ),
        );
    }
    permit
}

/// Permit de bootstrap de um download, liberável de mais de um lugar (leitor
/// de stdout, leitor de stderr, timer, fim do processo).
type BootSlot = Arc<std::sync::Mutex<Option<tokio::sync::OwnedSemaphorePermit>>>;

async fn hold_boot_slot(what: &str) -> BootSlot {
    let permit = acquire_ytdlp_slot(what).await;
    let slot: BootSlot = Arc::new(std::sync::Mutex::new(Some(permit)));
    let timer_slot = slot.clone();
    tokio::spawn(async move {
        tokio::time::sleep(BOOT_SLOT_MAX_HOLD).await;
        release_boot_slot(&timer_slot);
    });
    slot
}

fn release_boot_slot(slot: &BootSlot) {
    if let Ok(mut guard) = slot.lock() {
        guard.take();
    }
}

/// Apaga as pastas `_MEI*` que o PyInstaller deixou no temp.
///
/// Cada yt-dlp onefile extrai 72 MB para `$TMPDIR/_MEIxxxxxx` e só apaga na
/// saída normal; processo morto (cancelamento, timeout, troca de binário)
/// deixa a pasta. Foram 31 pastas, 2,2 GB. Só pastas com mais de um dia,
/// para nunca tirar o chão de um processo vivo. Devolve quantas apagou.
pub fn cleanup_stale_pyinstaller_dirs() -> usize {
    let temp = std::env::temp_dir();
    let Ok(entries) = std::fs::read_dir(&temp) else {
        return 0;
    };
    let cutoff = std::time::Duration::from_secs(24 * 60 * 60);
    let mut removed = 0usize;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with("_MEI") {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_dir() {
            continue;
        }
        let old_enough = meta
            .modified()
            .ok()
            .and_then(|m| m.elapsed().ok())
            .map(|age| age > cutoff)
            .unwrap_or(false);
        if !old_enough {
            continue;
        }
        if std::fs::remove_dir_all(entry.path()).is_ok() {
            removed += 1;
        }
    }
    if removed > 0 {
        tracing::info!(
            "[yt-dlp] removed {} stale PyInstaller dirs from {}",
            removed,
            temp.display()
        );
    }
    removed
}

/// Há yt-dlp rodando para algum download?
pub fn has_active_ytdlp_processes() -> bool {
    active_process_pids()
        .lock()
        .map(|m| !m.is_empty())
        .unwrap_or(true)
}

fn yt_rate_limiter() -> &'static YtRateLimiter {
    YT_RATE_LIMITER.get_or_init(|| YtRateLimiter {
        semaphore: tokio::sync::Semaphore::new(3),
        last_request_ns: AtomicU64::new(0),
    })
}

const CHROME_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
pub const VIDEO_INFO_PROCESS_TIMEOUT_SECS: u64 = 90;
pub const YOUTUBE_VIDEO_INFO_TOTAL_TIMEOUT_SECS: u64 = 190;
pub const DEFAULT_VIDEO_INFO_TOTAL_TIMEOUT_SECS: u64 = 110;

mod binaries;
mod channel_list;
mod download;
mod info;
mod parse;
mod size;

pub use binaries::*;
pub use channel_list::*;
pub use download::*;
pub use info::*;
pub use parse::*;
pub use size::*;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod worker_retry_tests {
    #[test]
    fn worker_bounds_transport_retries_and_disables_extractor_retries() {
        assert!(super::worker_retry_args(false).is_empty());
        let args = super::worker_retry_args(true);
        let value = |key: &str| {
            let at = args.iter().position(|v| *v == key).unwrap();
            args[at + 1]
        };
        assert_eq!(value("--retries"), "3");
        assert_eq!(value("--fragment-retries"), "3");
        assert_eq!(value("--extractor-retries"), "0");
        assert_eq!(value("--file-access-retries"), "0");
        assert!(args.contains(&"--ignore-config"));
        assert!(args.contains(&"--no-plugin-dirs"));
    }
    #[test]
    fn a_conflicting_range_retry_drops_the_chunk_size() {
        let mut args: Vec<String> = [
            "-N",
            "4",
            "--http-chunk-size",
            "10M",
            "--buffer-size",
            "16M",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        super::strip_chunk_size(&mut args);
        assert_eq!(args, ["-N", "4", "--buffer-size", "16M"]);
        super::strip_chunk_size(&mut args);
        assert_eq!(args.len(), 4);
    }
}
