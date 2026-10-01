pub mod claude;
pub mod codex;
pub mod copilot;
pub mod cursor;
pub mod grok;
pub mod kimi;
pub mod lmstudio;
pub mod ollama;
pub mod opencode;

use super::{AccountTag, UsageProvider};
use omniget_core::core::llm::cli_runtime::accounts::{AccountStore, CliAccount, CliKind};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

pub const OMNIGET_ID: &str = "omniget";

pub fn all() -> Vec<Arc<dyn UsageProvider>> {
    let accounts = AccountStore::default_store()
        .map(|s| (*s.list()).clone())
        .unwrap_or_default();
    let out = all_with(&accounts);
    remember(&out);
    out
}

const CACHE_TTL: Duration = Duration::from_secs(15);

type Cache = std::sync::Mutex<Option<(std::time::Instant, Vec<Arc<dyn UsageProvider>>)>>;

fn cache() -> &'static Cache {
    static CACHE: std::sync::OnceLock<Cache> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(None))
}

fn remember(readers: &[Arc<dyn UsageProvider>]) {
    if let Ok(mut g) = cache().lock() {
        *g = Some((std::time::Instant::now(), readers.to_vec()));
    }
}

pub fn cached() -> Vec<Arc<dyn UsageProvider>> {
    if let Ok(g) = cache().lock() {
        if let Some((at, list)) = g.as_ref() {
            if at.elapsed() < CACHE_TTL {
                return list.clone();
            }
        }
    }
    all()
}

pub fn all_with(accounts: &[CliAccount]) -> Vec<Arc<dyn UsageProvider>> {
    let mut out: Vec<Arc<dyn UsageProvider>> = Vec::new();
    for p in profiles(
        CliKind::Claude,
        accounts,
        claude::config_dir().as_deref(),
        &claude::email_of,
    ) {
        out.push(Arc::new(claude::Claude::new(p)));
    }
    for p in profiles(
        CliKind::Codex,
        accounts,
        codex::codex_home().as_deref(),
        &|_| None,
    ) {
        out.push(Arc::new(codex::Codex::new(p)));
    }
    let rest: [Arc<dyn UsageProvider>; 7] = [
        Arc::new(cursor::Cursor),
        Arc::new(copilot::Copilot),
        Arc::new(grok::Grok),
        Arc::new(kimi::Kimi),
        Arc::new(opencode::OpenCode),
        Arc::new(ollama::Ollama),
        Arc::new(lmstudio::LmStudio),
    ];
    out.extend(rest);
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub id: String,
    pub dir: Option<PathBuf>,
    pub tag: AccountTag,
}

fn same_dir(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| {
        let s = p.to_string_lossy();
        let t = s.trim_end_matches(['/', '\\']);
        std::fs::canonicalize(t).unwrap_or_else(|_| PathBuf::from(t))
    };
    norm(a) == norm(b)
}

pub fn profiles(
    cli: CliKind,
    accounts: &[CliAccount],
    default_dir: Option<&Path>,
    email_of: &dyn Fn(Option<&Path>) -> Option<String>,
) -> Vec<Profile> {
    let base = cli.as_str();
    let mine: Vec<&CliAccount> = accounts.iter().filter(|a| a.cli == cli).collect();
    let is_default = |a: &CliAccount| {
        a.config_dir.as_os_str().is_empty()
            || default_dir
                .map(|d| same_dir(&a.config_dir, d))
                .unwrap_or(false)
    };
    let default_label = mine
        .iter()
        .filter(|a| is_default(a))
        .min_by_key(|a| a.disabled)
        .map(|a| a.label.clone());
    let mut out = vec![Profile {
        id: base.to_string(),
        dir: None,
        tag: AccountTag {
            label: default_label,
            email: email_of(None),
        },
    }];
    for a in mine.iter().filter(|a| !a.disabled && !is_default(a)) {
        out.push(Profile {
            id: format!("{base}:{}", a.id),
            dir: Some(a.config_dir.clone()),
            tag: AccountTag {
                label: Some(a.label.clone()),
                email: email_of(Some(&a.config_dir)),
            },
        });
    }
    let mut kept: Vec<Profile> = Vec::new();
    for p in out {
        let twin = kept.iter().any(|k| {
            let same_email = match (&k.tag.email, &p.tag.email) {
                (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
                _ => false,
            };
            let same_dir = match (&k.dir, &p.dir) {
                (Some(a), Some(b)) => same_dir(a, b),
                _ => false,
            };
            same_email || same_dir
        });
        if !twin {
            kept.push(p);
        }
    }
    kept
}

pub fn loopback() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .no_proxy()
        .build()
        .unwrap_or_default()
}

pub fn not_running() -> super::Reading {
    super::Reading {
        note: Some("not running".into()),
        ..Default::default()
    }
}

pub fn installed(home_dir: &str, cli: &str) -> bool {
    super::home()
        .map(|h| h.join(home_dir).is_dir())
        .unwrap_or(false)
        || which::which(cli).is_ok()
}

pub fn xdg_data_home() -> Option<PathBuf> {
    if let Some(v) = std::env::var_os("XDG_DATA_HOME") {
        if !v.is_empty() {
            return Some(PathBuf::from(v));
        }
    }
    super::home().map(|h| h.join(".local").join("share"))
}

pub fn string_or_field(v: &serde_json::Value, fields: &[&str]) -> Option<String> {
    if let Some(s) = v.as_str() {
        let s = s.trim();
        return (!s.is_empty()).then(|| s.to_string());
    }
    fields.iter().find_map(|f| {
        v.get(*f)
            .and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    })
}

pub async fn run_cli(
    program: &std::path::Path,
    args: &[&str],
    envs: &[(&str, &str)],
    deadline: Duration,
) -> Option<(bool, String, String)> {
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .current_dir(std::env::temp_dir())
        .kill_on_drop(true);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    #[cfg(windows)]
    {
        cmd.creation_flags(0x0800_0000);
    }
    let child = cmd.spawn().ok()?;
    let out = tokio::time::timeout(deadline, child.wait_with_output())
        .await
        .ok()?
        .ok()?;
    Some((
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    ))
}

pub fn open_sqlite_ro(path: &std::path::Path) -> Option<rusqlite::Connection> {
    use rusqlite::{Connection, OpenFlags};
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI;
    let p = path.to_string_lossy().replace('\\', "/");
    let p = if p.starts_with('/') {
        p
    } else {
        format!("/{p}")
    };
    let enc: String = p
        .chars()
        .map(|c| match c {
            ' ' => "%20".to_string(),
            '#' => "%23".to_string(),
            '?' => "%3f".to_string(),
            '%' => "%25".to_string(),
            c => c.to_string(),
        })
        .collect();
    for suffix in ["mode=ro", "immutable=1"] {
        let uri = format!("file:{enc}?{suffix}");
        if let Ok(conn) = Connection::open_with_flags(&uri, flags) {
            let _ = conn.busy_timeout(Duration::from_millis(500));
            let probe = conn
                .prepare("SELECT name FROM sqlite_master LIMIT 1")
                .and_then(|mut s| s.query([]).map(|_| ()));
            if probe.is_ok() {
                return Some(conn);
            }
        }
    }
    None
}

pub fn vscdb_value(conn: &rusqlite::Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM ItemTable WHERE key = ?1", [key], |r| {
        r.get::<_, rusqlite::types::Value>(0)
    })
    .ok()
    .and_then(|v| match v {
        rusqlite::types::Value::Text(s) => Some(s),
        rusqlite::types::Value::Blob(b) => String::from_utf8(b).ok(),
        _ => None,
    })
}
