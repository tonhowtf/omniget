//! Bloqueio de anúncio e telemetria pelo arquivo `hosts` do sistema.
//!
//! Sem servidor, sem driver, sem proxy: o sistema já resolve nome pelo
//! `hosts` antes de perguntar ao DNS, então apontar um domínio para
//! `0.0.0.0` mata a conexão na origem.
//!
//! A regra dura deste módulo é **nunca tocar em nada fora do bloco**. Tudo
//! que o OmniGet escreve vive entre dois marcadores; o texto antes e depois
//! deles é preservado byte a byte. Escrever no `hosts` exige admin/root: se
//! não der, o módulo não tenta escalar privilégio — ele grava a versão
//! pronta num arquivo de teste e devolve o comando exato para o usuário
//! rodar.

use std::path::{Path, PathBuf};

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

pub const BEGIN: &str = "# >>> OmniGet block list >>>";
pub const END: &str = "# <<< OmniGet block list <<<";

const ID: &str = "sys-hosts";

/// Telemetria da Microsoft: lista curta e conservadora (só o que é
/// diagnóstico/anúncio, nada de Update, Store ou ativação).
pub const MICROSOFT_TELEMETRY: &[&str] = &[
    "vortex.data.microsoft.com",
    "vortex-win.data.microsoft.com",
    "telecommand.telemetry.microsoft.com",
    "telemetry.microsoft.com",
    "settings-win.data.microsoft.com",
    "watson.telemetry.microsoft.com",
    "watson.microsoft.com",
    "telemetry.urs.microsoft.com",
    "oca.telemetry.microsoft.com",
    "sqm.telemetry.microsoft.com",
    "df.telemetry.microsoft.com",
    "reports.wes.df.telemetry.microsoft.com",
    "services.wes.df.telemetry.microsoft.com",
    "statsfe2.ws.microsoft.com",
    "statsfe1.ws.microsoft.com",
    "survey.watson.microsoft.com",
    "redir.metaservices.microsoft.com",
    "choice.microsoft.com",
    "feedback.windows.com",
    "feedback.microsoft-hohm.com",
    "feedback.search.microsoft.com",
    "scorecardresearch.com",
];

/// Fontes conhecidas. `None` de URL = lista embutida, não baixa nada.
pub fn source_url(id: &str) -> Option<&'static str> {
    match id {
        "stevenblack" => Some("https://raw.githubusercontent.com/StevenBlack/hosts/master/hosts"),
        "adaway" => Some("https://adaway.org/hosts.txt"),
        _ => None,
    }
}

pub fn known_sources() -> Vec<&'static str> {
    vec!["stevenblack", "adaway", "microsoft"]
}

// ── Caminho do arquivo ──

pub fn default_hosts_path() -> PathBuf {
    if cfg!(windows) {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
        PathBuf::from(root)
            .join("System32")
            .join("drivers")
            .join("etc")
            .join("hosts")
    } else {
        PathBuf::from("/etc/hosts")
    }
}

fn resolve_path(path: &Option<String>) -> PathBuf {
    match path {
        Some(p) if !p.trim().is_empty() => PathBuf::from(p.trim()),
        _ => default_hosts_path(),
    }
}

/// Testa permissão de escrita abrindo o arquivo em modo escrita **sem**
/// truncar. Não muda nada e é a única checagem que vale nos três sistemas.
pub fn is_writable(path: &Path) -> bool {
    std::fs::OpenOptions::new().write(true).open(path).is_ok()
}

/// Comando pronto para o usuário aplicar por fora, quando falta permissão.
pub fn permission_hint(staging: &Path, target: &Path) -> String {
    if cfg!(windows) {
        format!(
            "Abra o OmniGet como administrador, ou rode num Prompt elevado: copy /Y \"{}\" \"{}\"",
            staging.display(),
            target.display()
        )
    } else {
        format!(
            "Falta permissão de root. Rode: sudo cp '{}' '{}'",
            staging.display(),
            target.display()
        )
    }
}

// ── Parse e normalização ──

/// Domínios que nunca podem entrar no bloco: quebrariam a própria máquina.
fn is_reserved(domain: &str) -> bool {
    matches!(
        domain,
        "localhost"
            | "localhost.localdomain"
            | "local"
            | "broadcasthost"
            | "ip6-localhost"
            | "ip6-loopback"
            | "ip6-localnet"
            | "ip6-mcastprefix"
            | "ip6-allnodes"
            | "ip6-allrouters"
            | "ip6-allhosts"
            | "0.0.0.0"
    )
}

fn looks_like_ip(token: &str) -> bool {
    token == "::"
        || token == "::1"
        || token.starts_with("fe80::")
        || (token.split('.').count() == 4
            && token
                .split('.')
                .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())))
}

/// Deixa o domínio na forma canônica ou devolve `None` se não for domínio.
pub fn normalize_domain(raw: &str) -> Option<String> {
    let d = raw.trim().trim_end_matches('.').to_ascii_lowercase();
    if d.len() < 4 || d.len() > 253 || !d.contains('.') {
        return None;
    }
    if d.starts_with('.') || d.starts_with('-') || d.ends_with('-') {
        return None;
    }
    if !d
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_')
    {
        return None;
    }
    if d.split('.').any(|label| label.is_empty()) {
        return None;
    }
    if looks_like_ip(&d) || is_reserved(&d) {
        return None;
    }
    Some(d)
}

/// Lê uma lista nos dois formatos que circulam por aí: `0.0.0.0 dominio`
/// (formato hosts) e só o domínio por linha. Comentário com `#` cai fora.
pub fn parse_list(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut tokens = line.split_whitespace();
        let Some(first) = tokens.next() else { continue };
        if looks_like_ip(first) {
            for token in tokens {
                if let Some(d) = normalize_domain(token) {
                    out.push(d);
                }
            }
        } else if let Some(d) = normalize_domain(first) {
            out.push(d);
        }
    }
    out
}

/// `true` quando o domínio está na whitelist, direto ou como subdomínio.
pub fn whitelisted(domain: &str, whitelist: &[String]) -> bool {
    whitelist.iter().any(|w| {
        let w = w.trim().trim_end_matches('.').to_ascii_lowercase();
        !w.is_empty() && (domain == w || domain.ends_with(&format!(".{}", w)))
    })
}

/// Junta tudo, tira repetido, tira whitelist e ordena. Devolve
/// `(domínios, quantos a whitelist barrou)`.
pub fn merge(lists: &[Vec<String>], whitelist: &[String]) -> (Vec<String>, usize) {
    let mut set = std::collections::BTreeSet::new();
    let mut skipped = 0usize;
    for list in lists {
        for d in list {
            if whitelisted(d, whitelist) {
                skipped += 1;
                continue;
            }
            set.insert(d.clone());
        }
    }
    (set.into_iter().collect(), skipped)
}

// ── O bloco ──

fn newline_of(text: &str) -> &'static str {
    if text.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

/// Faixa de bytes ocupada pelo bloco, incluindo as duas linhas de marcador.
///
/// Se o `BEGIN` existe e o `END` não, o bloco vai até o fim do arquivo: é
/// resto de escrita interrompida, e tudo depois do marcador foi escrito por
/// nós. Assim uma reaplicação conserta o arquivo em vez de duplicar o bloco.
fn block_span(text: &str) -> Option<(usize, usize)> {
    let mut begin: Option<usize> = None;
    let mut idx = 0usize;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']).trim();
        if begin.is_none() {
            if trimmed == BEGIN {
                begin = Some(idx);
            }
        } else if trimmed == END {
            return Some((begin?, idx + line.len()));
        }
        idx += line.len();
    }
    begin.map(|b| (b, text.len()))
}

pub fn has_block(text: &str) -> bool {
    block_span(text).is_some()
}

/// Só o miolo do bloco (sem os marcadores), se ele existir.
pub fn block_body(text: &str) -> Option<&str> {
    let (start, end) = block_span(text)?;
    let inner = &text[start..end];
    let after_begin = inner.find('\n').map_or_else(|| inner.len(), |i| i + 1);
    let body = &inner[after_begin..];
    let before_end = body.rfind(END).map_or_else(
        || body.len(),
        |i| body[..i].rfind('\n').map_or(0, |j| j + 1),
    );
    Some(&body[..before_end])
}

/// Quantos domínios o bloco atual bloqueia.
pub fn count_blocked(text: &str) -> usize {
    block_body(text).map(|b| parse_list(b).len()).unwrap_or(0)
}

/// Data que o cabeçalho do bloco anuncia, se houver.
pub fn block_updated_at(text: &str) -> Option<String> {
    let body = block_body(text)?;
    body.lines().find_map(|l| {
        l.trim()
            .strip_prefix("# atualizado em ")
            .map(str::to_string)
    })
}

/// Monta o bloco inteiro, marcadores incluídos, terminado em quebra de linha.
pub fn render_block(domains: &[String], ip: &str, updated_at: &str, nl: &str) -> String {
    let ip = if ip.trim().is_empty() {
        "0.0.0.0"
    } else {
        ip.trim()
    };
    let mut s = String::with_capacity(domains.len() * 32 + 256);
    s.push_str(BEGIN);
    s.push_str(nl);
    s.push_str("# gerado pelo OmniGet — não edite aqui dentro");
    s.push_str(nl);
    s.push_str(&format!("# atualizado em {}", updated_at));
    s.push_str(nl);
    s.push_str(&format!("# {} domínios", domains.len()));
    s.push_str(nl);
    for d in domains {
        s.push_str(&format!("{} {}", ip, d));
        s.push_str(nl);
    }
    s.push_str(END);
    s.push_str(nl);
    s
}

/// Troca (ou acrescenta) o bloco. O texto do usuário sai intacto: a única
/// coisa que pode mudar fora do bloco é ganhar a quebra de linha final que
/// faltava, senão a última linha do usuário grudaria no marcador.
pub fn apply_to_text(current: &str, domains: &[String], ip: &str, updated_at: &str) -> String {
    let nl = newline_of(current);
    let block = render_block(domains, ip, updated_at, nl);
    match block_span(current) {
        Some((start, end)) => {
            let mut out = String::with_capacity(current.len() + block.len());
            out.push_str(&current[..start]);
            out.push_str(&block);
            out.push_str(&current[end..]);
            out
        }
        None => {
            let mut out = String::with_capacity(current.len() + block.len() + 2);
            out.push_str(current);
            if !current.is_empty() && !current.ends_with('\n') {
                out.push_str(nl);
            }
            out.push_str(&block);
            out
        }
    }
}

/// Tira o bloco e devolve o resto exatamente como estava.
pub fn remove_from_text(current: &str) -> String {
    match block_span(current) {
        Some((start, end)) => {
            let mut out = String::with_capacity(current.len());
            out.push_str(&current[..start]);
            out.push_str(&current[end..]);
            out
        }
        None => current.to_string(),
    }
}

// ── Estado ──

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ReadOptions {
    /// Vazio = o `hosts` do sistema.
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BackupInfo {
    pub path: String,
    pub bytes: u64,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct HostsState {
    pub path: String,
    pub exists: bool,
    pub writable: bool,
    pub bytes: u64,
    /// Linhas fora do bloco (o que é do usuário), incluindo comentário.
    pub user_lines: usize,
    /// Domínios dentro do bloco do OmniGet.
    pub blocked: usize,
    pub block_present: bool,
    pub updated_at: Option<String>,
    /// Prévia do que é do usuário, para a UI mostrar sem abrir o arquivo.
    pub user_preview: Vec<String>,
    pub backups: Vec<BackupInfo>,
    pub sources: Vec<String>,
}

fn backups_dir() -> Option<PathBuf> {
    super::tools_dir().map(|d| d.join("hosts_backups"))
}

fn list_backups() -> Vec<BackupInfo> {
    let Some(dir) = backups_dir() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<BackupInfo> = entries
        .flatten()
        .filter(|e| e.path().is_file())
        .map(|e| BackupInfo {
            name: e.file_name().to_string_lossy().to_string(),
            bytes: e.metadata().map(|m| m.len()).unwrap_or(0),
            path: e.path().to_string_lossy().to_string(),
        })
        .collect();
    // O nome carrega a data, então ordem alfabética invertida = mais novo antes.
    out.sort_by(|a, b| b.name.cmp(&a.name));
    out.truncate(30);
    out
}

pub fn read_state(opts: &ReadOptions) -> anyhow::Result<HostsState> {
    let path = resolve_path(&opts.path);
    let exists = path.exists();
    let text = if exists {
        std::fs::read_to_string(&path).unwrap_or_default()
    } else {
        String::new()
    };
    let outside = remove_from_text(&text);
    let user_lines = outside.lines().filter(|l| !l.trim().is_empty()).count();
    let user_preview: Vec<String> = outside
        .lines()
        .filter(|l| !l.trim().is_empty())
        .take(40)
        .map(str::to_string)
        .collect();
    Ok(HostsState {
        path: path.to_string_lossy().to_string(),
        exists,
        writable: exists && is_writable(&path),
        bytes: text.len() as u64,
        user_lines,
        blocked: count_blocked(&text),
        block_present: has_block(&text),
        updated_at: block_updated_at(&text),
        user_preview,
        backups: list_backups(),
        sources: known_sources().into_iter().map(str::to_string).collect(),
    })
}

// ── Aplicar ──

fn yes() -> bool {
    true
}

fn default_ip() -> String {
    "0.0.0.0".into()
}

fn default_sources() -> Vec<String> {
    vec!["stevenblack".into(), "microsoft".into()]
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApplyOptions {
    /// Vazio = o `hosts` do sistema.
    #[serde(default)]
    pub path: Option<String>,
    /// Ids de `known_sources()`; o resto é tratado como URL solta.
    #[serde(default = "default_sources")]
    pub sources: Vec<String>,
    /// Domínios extras do usuário.
    #[serde(default)]
    pub extra: Vec<String>,
    /// Domínios que nunca entram (o subdomínio também é poupado).
    #[serde(default)]
    pub whitelist: Vec<String>,
    #[serde(default = "default_ip")]
    pub ip: String,
    /// Padrão seguro: só mostra o que aconteceria.
    #[serde(default = "yes")]
    pub dry_run: bool,
    /// Escrever aqui em vez do `hosts` (saída para quem não tem permissão).
    #[serde(default)]
    pub output_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceStat {
    pub id: String,
    pub url: Option<String>,
    pub domains: usize,
    pub ok: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApplyResult {
    pub path: String,
    pub dry_run: bool,
    pub written: bool,
    pub blocked: usize,
    pub blocked_before: usize,
    pub skipped_whitelist: usize,
    pub bytes: u64,
    pub backup: Option<String>,
    pub updated_at: String,
    pub sources: Vec<SourceStat>,
    pub needs_admin: bool,
    /// Cópia pronta do arquivo, quando faltou permissão.
    pub staged: Option<String>,
    pub hint: Option<String>,
    pub error: Option<String>,
}

fn now_stamp() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M").to_string()
}

fn backup_now(path: &Path, text: &str) -> Option<String> {
    let dir = backups_dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    let stem = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "hosts".into());
    let name = format!(
        "{}-{}.bak",
        stem,
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    );
    let dest = dir.join(name);
    std::fs::write(&dest, text).ok()?;
    Some(dest.to_string_lossy().to_string())
}

fn stage_file(path: &Path, text: &str) -> Option<PathBuf> {
    let dir = super::tools_dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    let dest = dir.join(format!(
        "{}-proposto.txt",
        path.file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "hosts".into())
    ));
    std::fs::write(&dest, text).ok()?;
    Some(dest)
}

async fn fetch_source(client: &reqwest::Client, id: &str) -> anyhow::Result<Vec<String>> {
    if id == "microsoft" {
        return Ok(MICROSOFT_TELEMETRY
            .iter()
            .copied()
            .filter_map(normalize_domain)
            .collect());
    }
    let url = source_url(id)
        .map(str::to_string)
        .or_else(|| id.starts_with("http").then(|| id.to_string()))
        .ok_or_else(|| anyhow!("fonte desconhecida: {}", id))?;
    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        return Err(anyhow!("HTTP {}", resp.status()));
    }
    let text = resp.text().await?;
    Ok(parse_list(&text))
}

pub async fn apply(
    opts: &ApplyOptions,
    progress: &super::ProgressFn,
) -> anyhow::Result<ApplyResult> {
    let path = resolve_path(&opts.path);
    let client = super::client()?;
    let total = opts.sources.len() as u64 + 1;

    let mut lists: Vec<Vec<String>> = Vec::new();
    let mut stats: Vec<SourceStat> = Vec::new();
    for (i, id) in opts.sources.iter().enumerate() {
        super::report(
            progress,
            ID,
            "progress",
            i as u64,
            Some(total),
            Some(id.clone()),
        );
        match fetch_source(&client, id).await {
            Ok(list) => {
                stats.push(SourceStat {
                    id: id.clone(),
                    url: source_url(id).map(str::to_string),
                    domains: list.len(),
                    ok: true,
                    error: None,
                });
                lists.push(list);
            }
            Err(e) => {
                tracing::warn!("[sys-hosts] fonte {}: {}", id, e);
                stats.push(SourceStat {
                    id: id.clone(),
                    url: source_url(id).map(str::to_string),
                    domains: 0,
                    ok: false,
                    error: Some(e.to_string()),
                });
            }
        }
    }
    let extra: Vec<String> = opts
        .extra
        .iter()
        .filter_map(|d| normalize_domain(d.as_str()))
        .collect();
    if !extra.is_empty() {
        stats.push(SourceStat {
            id: "extra".into(),
            url: None,
            domains: extra.len(),
            ok: true,
            error: None,
        });
        lists.push(extra);
    }
    if lists.iter().all(|l| l.is_empty()) {
        return Err(anyhow!("nenhuma lista veio: nada a bloquear"));
    }

    let (domains, skipped) = merge(&lists, &opts.whitelist);
    let current = std::fs::read_to_string(&path).unwrap_or_default();
    let blocked_before = count_blocked(&current);
    let updated_at = now_stamp();
    let next = apply_to_text(&current, &domains, &opts.ip, &updated_at);
    super::report(progress, ID, "progress", total - 1, Some(total), None);

    let mut result = ApplyResult {
        path: path.to_string_lossy().to_string(),
        dry_run: opts.dry_run,
        written: false,
        blocked: domains.len(),
        blocked_before,
        skipped_whitelist: skipped,
        bytes: next.len() as u64,
        backup: None,
        updated_at,
        sources: stats,
        needs_admin: false,
        staged: None,
        hint: None,
        error: None,
    };

    // Saída alternativa: gerar o arquivo onde o usuário mandou, sem admin.
    if let Some(out) = opts.output_path.as_ref().filter(|s| !s.trim().is_empty()) {
        let out = PathBuf::from(out.trim());
        if !opts.dry_run {
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&out, &next)?;
            result.written = true;
        }
        result.path = out.to_string_lossy().to_string();
        super::report(progress, ID, "done", total, Some(total), None);
        return Ok(result);
    }

    if opts.dry_run {
        super::report(progress, ID, "done", total, Some(total), None);
        return Ok(result);
    }

    if path.exists() && !is_writable(&path) {
        let staged = stage_file(&path, &next);
        result.needs_admin = true;
        result.hint = staged.as_ref().map(|s| permission_hint(s, &path));
        result.staged = staged.map(|s| s.to_string_lossy().to_string());
        result.error = Some(if cfg!(windows) {
            "sem permissão para escrever no hosts: o OmniGet precisa estar aberto como administrador".into()
        } else {
            "sem permissão para escrever no hosts: precisa de root".into()
        });
        super::report(progress, ID, "done", total, Some(total), None);
        return Ok(result);
    }

    if !current.is_empty() {
        result.backup = backup_now(&path, &current);
    }
    if let Err(e) = std::fs::write(&path, &next) {
        let staged = stage_file(&path, &next);
        result.needs_admin = e.kind() == std::io::ErrorKind::PermissionDenied;
        result.hint = staged.as_ref().map(|s| permission_hint(s, &path));
        result.staged = staged.map(|s| s.to_string_lossy().to_string());
        result.error = Some(format!("não consegui escrever: {}", e));
    } else {
        result.written = true;
    }
    super::report(progress, ID, "done", total, Some(total), None);
    Ok(result)
}

// ── Desfazer ──

#[derive(Debug, Clone, Deserialize)]
pub struct RestoreOptions {
    #[serde(default)]
    pub path: Option<String>,
    /// Caminho de um backup; vazio = só remover o bloco.
    #[serde(default)]
    pub backup: Option<String>,
    #[serde(default = "yes")]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct RestoreResult {
    pub path: String,
    pub dry_run: bool,
    pub written: bool,
    pub removed_block: bool,
    pub restored_from: Option<String>,
    pub blocked_before: usize,
    pub bytes: u64,
    pub backup: Option<String>,
    pub needs_admin: bool,
    pub staged: Option<String>,
    pub hint: Option<String>,
    pub error: Option<String>,
}

pub fn restore(opts: &RestoreOptions) -> anyhow::Result<RestoreResult> {
    let path = resolve_path(&opts.path);
    let current = std::fs::read_to_string(&path).unwrap_or_default();
    let blocked_before = count_blocked(&current);

    let (next, restored_from) = match opts.backup.as_ref().filter(|s| !s.trim().is_empty()) {
        Some(b) => {
            let text = std::fs::read_to_string(b.trim())
                .map_err(|e| anyhow!("não li o backup {}: {}", b.trim(), e))?;
            (text, Some(b.trim().to_string()))
        }
        None => (remove_from_text(&current), None),
    };

    let mut result = RestoreResult {
        path: path.to_string_lossy().to_string(),
        dry_run: opts.dry_run,
        written: false,
        removed_block: restored_from.is_none() && has_block(&current),
        restored_from,
        blocked_before,
        bytes: next.len() as u64,
        backup: None,
        needs_admin: false,
        staged: None,
        hint: None,
        error: None,
    };
    if opts.dry_run {
        return Ok(result);
    }
    if path.exists() && !is_writable(&path) {
        let staged = stage_file(&path, &next);
        result.needs_admin = true;
        result.hint = staged.as_ref().map(|s| permission_hint(s, &path));
        result.staged = staged.map(|s| s.to_string_lossy().to_string());
        result.error = Some("sem permissão para escrever no hosts".into());
        return Ok(result);
    }
    if !current.is_empty() {
        result.backup = backup_now(&path, &current);
    }
    match std::fs::write(&path, &next) {
        Ok(()) => result.written = true,
        Err(e) => {
            let staged = stage_file(&path, &next);
            result.needs_admin = e.kind() == std::io::ErrorKind::PermissionDenied;
            result.hint = staged.as_ref().map(|s| permission_hint(s, &path));
            result.staged = staged.map(|s| s.to_string_lossy().to_string());
            result.error = Some(format!("não consegui escrever: {}", e));
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Nenhum teste aqui pode chegar perto do `hosts` de verdade: tudo roda
    /// num arquivo temporário próprio.
    fn tempdir(tag: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("omniget-hosts-{}-{}", tag, uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).expect("criar tempdir de teste");
        d
    }

    const USER: &str = "127.0.0.1\tlocalhost\n255.255.255.255\tbroadcasthost\n::1 localhost\n\n# minhas coisas\n10.0.0.7 servidor.casa\n";

    fn dominios(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn block_is_inserted_from_scratch() {
        let out = apply_to_text(
            USER,
            &dominios(&["ads.example.com"]),
            "0.0.0.0",
            "2026-01-01 10:00",
        );
        assert!(
            out.starts_with(USER),
            "o texto do usuário tem que abrir o arquivo intacto"
        );
        assert!(out.contains(BEGIN) && out.contains(END));
        assert!(out.contains("0.0.0.0 ads.example.com"));
        assert_eq!(count_blocked(&out), 1);
    }

    #[test]
    fn updating_the_block_never_touches_the_user_text() {
        let first = apply_to_text(
            USER,
            &dominios(&["a.example.com"]),
            "0.0.0.0",
            "2026-01-01 10:00",
        );
        // O usuário mexe no arquivo depois, antes e depois do bloco.
        let mexido = format!("# linha nova do usuário\n{}\n10.0.0.9 outro.casa\n", first);
        let antes = remove_from_text(&mexido);

        let second = apply_to_text(
            &mexido,
            &dominios(&["b.example.com", "c.example.com"]),
            "0.0.0.0",
            "2026-02-02 11:11",
        );
        let depois = remove_from_text(&second);
        assert_eq!(
            antes.as_bytes(),
            depois.as_bytes(),
            "o que está fora do bloco mudou"
        );
        assert_eq!(count_blocked(&second), 2);
        assert!(
            !second.contains("a.example.com"),
            "o bloco velho tinha que sumir"
        );
        // E não duplicou o bloco.
        assert_eq!(second.matches(BEGIN).count(), 1);
        assert_eq!(second.matches(END).count(), 1);
    }

    #[test]
    fn removing_the_block_gives_the_file_back() {
        let out = apply_to_text(
            USER,
            &dominios(&["ads.example.com"]),
            "0.0.0.0",
            "2026-01-01 10:00",
        );
        assert_eq!(remove_from_text(&out).as_bytes(), USER.as_bytes());
        assert!(!has_block(&remove_from_text(&out)));
        // Remover de um arquivo sem bloco é no-op.
        assert_eq!(remove_from_text(USER), USER);
    }

    #[test]
    fn applying_twice_gives_the_same_file() {
        let one = apply_to_text(
            USER,
            &dominios(&["a.example.com", "b.example.com"]),
            "0.0.0.0",
            "2026-01-01 10:00",
        );
        let two = apply_to_text(
            &one,
            &dominios(&["a.example.com", "b.example.com"]),
            "0.0.0.0",
            "2026-01-01 10:00",
        );
        assert_eq!(one, two, "aplicar duas vezes tem que dar o mesmo arquivo");
    }

    #[test]
    fn an_unterminated_block_is_repaired_not_duplicated() {
        let quebrado = format!("{}{}\n0.0.0.0 sobra.example.com\n", USER, BEGIN);
        let out = apply_to_text(
            &quebrado,
            &dominios(&["novo.example.com"]),
            "0.0.0.0",
            "2026-01-01 10:00",
        );
        assert_eq!(out.matches(BEGIN).count(), 1);
        assert!(!out.contains("sobra.example.com"));
        assert_eq!(remove_from_text(&out).as_bytes(), USER.as_bytes());
    }

    #[test]
    fn both_list_formats_parse() {
        let hosts_fmt = "# comentario\n0.0.0.0 ads.example.com\n127.0.0.1 trk.example.net # inline\n0.0.0.0 localhost\n\n";
        let plain_fmt = "ads.example.com\ntrk.example.net\n# comentario\n\n";
        assert_eq!(
            parse_list(hosts_fmt),
            dominios(&["ads.example.com", "trk.example.net"])
        );
        assert_eq!(
            parse_list(plain_fmt),
            dominios(&["ads.example.com", "trk.example.net"])
        );
        // Uma linha hosts com vários domínios conta todos.
        assert_eq!(parse_list("0.0.0.0 a.example.com b.example.com").len(), 2);
        // Lixo não vira domínio.
        assert!(parse_list("nao-eh-dominio\n192.168.0.1\n::1 ip6-localhost\n").is_empty());
    }

    #[test]
    fn merge_dedupes_and_respects_the_whitelist() {
        let a = dominios(&["b.example.com", "a.example.com", "ads.google.com"]);
        let b = dominios(&["a.example.com", "cdn.ads.google.com"]);
        let (out, skipped) = merge(&[a, b], &dominios(&["ads.google.com"]));
        assert_eq!(
            out,
            dominios(&["a.example.com", "b.example.com"]),
            "ordenado e sem repetido"
        );
        assert_eq!(skipped, 2, "o domínio e o subdomínio dele saem");
        assert!(whitelisted(
            "cdn.ads.google.com",
            &dominios(&["ads.google.com"])
        ));
        assert!(!whitelisted(
            "notads.google.com",
            &dominios(&["ads.google.com"])
        ));
    }

    #[test]
    fn crlf_files_stay_crlf() {
        let win = "127.0.0.1\tlocalhost\r\n# comentario\r\n";
        let out = apply_to_text(
            win,
            &dominios(&["ads.example.com"]),
            "0.0.0.0",
            "2026-01-01 10:00",
        );
        assert!(out.contains("0.0.0.0 ads.example.com\r\n"));
        assert!(
            !out.contains("ads.example.com\n\r"),
            "quebra de linha misturada"
        );
        assert_eq!(remove_from_text(&out).as_bytes(), win.as_bytes());
    }

    #[test]
    fn reserved_names_never_enter_the_block() {
        for name in ["localhost", "broadcasthost", "ip6-allnodes", "0.0.0.0"] {
            assert!(
                normalize_domain(name).is_none(),
                "{} não podia passar",
                name
            );
        }
        assert_eq!(
            normalize_domain("  ADS.Example.COM. "),
            Some("ads.example.com".into())
        );
    }

    #[test]
    fn dry_run_is_the_default_and_writes_nothing() {
        let opts: ApplyOptions = serde_json::from_str("{}").expect("options mínimas");
        assert!(opts.dry_run, "dry_run tem que vir ligado por padrão");
        let r: RestoreOptions = serde_json::from_str("{}").expect("options mínimas");
        assert!(r.dry_run);
    }

    #[test]
    fn restore_removes_the_block_on_a_temp_file() {
        let dir = tempdir("restore");
        let file = dir.join("hosts");
        let com_bloco = apply_to_text(
            USER,
            &dominios(&["ads.example.com"]),
            "0.0.0.0",
            "2026-01-01 10:00",
        );
        std::fs::write(&file, &com_bloco).expect("escrever hosts de teste");

        let seco = restore(&RestoreOptions {
            path: Some(file.to_string_lossy().to_string()),
            backup: None,
            dry_run: true,
        })
        .expect("restore dry");
        assert!(!seco.written);
        assert_eq!(seco.blocked_before, 1);
        assert_eq!(
            std::fs::read_to_string(&file).expect("reler"),
            com_bloco,
            "dry_run não podia escrever"
        );

        let molhado = restore(&RestoreOptions {
            path: Some(file.to_string_lossy().to_string()),
            backup: None,
            dry_run: false,
        })
        .expect("restore real");
        assert!(molhado.written && molhado.removed_block);
        assert_eq!(std::fs::read_to_string(&file).expect("reler"), USER);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn state_separates_user_lines_from_ours() {
        let dir = tempdir("state");
        let file = dir.join("hosts");
        let com_bloco = apply_to_text(
            USER,
            &dominios(&["a.example.com", "b.example.com"]),
            "0.0.0.0",
            "2026-03-03 09:09",
        );
        std::fs::write(&file, &com_bloco).expect("escrever hosts de teste");
        let st = read_state(&ReadOptions {
            path: Some(file.to_string_lossy().to_string()),
        })
        .expect("ler estado");
        assert!(st.exists && st.block_present);
        assert_eq!(st.blocked, 2);
        assert_eq!(st.user_lines, 5, "as cinco linhas do usuário, sem o bloco");
        assert_eq!(st.updated_at.as_deref(), Some("2026-03-03 09:09"));
        assert_eq!(st.bytes, com_bloco.len() as u64);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_file_reads_as_empty_not_as_an_error() {
        let dir = tempdir("vazio");
        let st = read_state(&ReadOptions {
            path: Some(dir.join("nao-existe").to_string_lossy().to_string()),
        })
        .expect("estado de arquivo ausente");
        assert!(!st.exists && !st.writable && !st.block_present);
        assert_eq!(st.blocked, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Baixa as listas de verdade e monta o bloco num arquivo temporário.
    /// `cargo test -p omniget-core --lib -- --ignored live_hosts`
    #[tokio::test]
    #[ignore]
    async fn live_hosts_sources_download_and_merge() {
        let dir = tempdir("live");
        let file = dir.join("hosts");
        std::fs::write(&file, USER).expect("escrever hosts de teste");
        let res = apply(
            &ApplyOptions {
                path: Some(file.to_string_lossy().to_string()),
                sources: vec!["stevenblack".into(), "adaway".into(), "microsoft".into()],
                extra: vec![],
                whitelist: vec!["googleadservices.com".into()],
                ip: "0.0.0.0".into(),
                dry_run: false,
                output_path: None,
            },
            &crate::core::tools::noop_progress(),
        )
        .await
        .expect("apply");
        assert!(res.written, "{:?}", res.error);
        assert!(res.blocked > 50_000, "veio pouca coisa: {}", res.blocked);
        let text = std::fs::read_to_string(&file).expect("reler");
        assert_eq!(remove_from_text(&text).as_bytes(), USER.as_bytes());
        assert!(!text.contains(" googleadservices.com"));
        eprintln!("bloqueados: {} · {} bytes", res.blocked, res.bytes);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
