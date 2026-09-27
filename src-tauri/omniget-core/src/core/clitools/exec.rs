//! Utilitários de processo e versão: busca no PATH, execução curta com
//! timeout e comparação de versões. Tudo passa por `core::process`.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use super::table::expand_home;

/// Saída de um comando curto.
#[derive(Debug, Clone)]
pub struct Captured {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Captured {
    pub fn ok(&self) -> bool {
        self.code == Some(0)
    }
    pub fn first_line(&self) -> String {
        self.stdout
            .lines()
            .chain(self.stderr.lines())
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("")
            .to_string()
    }
}

/// Roda `program args` com timeout; `None` se não deu para iniciar ou estourou.
pub async fn run_capture(
    program: &Path,
    args: &[String],
    env: &BTreeMap<String, String>,
    timeout: Duration,
) -> Option<Captured> {
    let mut cmd = command_for(program);
    cmd.args(args)
        .envs(env)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let child = crate::core::process::spawn_retrying_busy(|| cmd.spawn()).ok()?;
    let out = tokio::time::timeout(timeout, child.wait_with_output())
        .await
        .ok()?
        .ok()?;
    Some(Captured {
        code: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

/// No Windows um `.cmd`/`.bat`/`.ps1` não executa sozinho: passa pelo `cmd /C`
/// ou pelo PowerShell. Nos outros sistemas é o próprio programa.
pub fn command_for(program: &Path) -> tokio::process::Command {
    let ext = program
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    if cfg!(windows) && (ext == "cmd" || ext == "bat") {
        let mut c = crate::core::process::command("cmd");
        c.arg("/C").arg(program);
        c
    } else if cfg!(windows) && ext == "ps1" {
        let mut c = crate::core::process::command("powershell");
        c.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(program);
        c
    } else {
        crate::core::process::command(program)
    }
}

/// Diretórios onde procurar binários: `OMNIGET_CLITOOLS_EXTRA_PATH` (testes),
/// o PATH do processo e as pastas que os instaladores oficiais usam (apps de
/// GUI no macOS herdam um PATH mínimo).
pub fn search_dirs(extra: &[PathBuf]) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(p) = std::env::var_os("OMNIGET_CLITOOLS_EXTRA_PATH") {
        dirs.extend(std::env::split_paths(&p));
    }
    dirs.extend(extra.iter().cloned());
    if let Some(p) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&p));
    }
    for d in [
        "~/.local/bin",
        "~/.opencode/bin",
        "~/.amp/bin",
        "~/.grok/bin",
        "~/.kimi-code/bin",
        "~/.bun/bin",
        "~/.cargo/bin",
        "~/.npm-global/bin",
        "~/.volta/bin",
        "~/Library/pnpm",
        "~/.local/share/pnpm",
    ] {
        dirs.push(expand_home(d));
    }
    if let Some(bin) = crate::core::paths::app_data_dir().map(|d| d.join("bin")) {
        dirs.push(bin);
    }
    if cfg!(target_os = "macos") {
        dirs.push(PathBuf::from("/opt/homebrew/bin"));
        dirs.push(PathBuf::from("/usr/local/bin"));
    } else if cfg!(target_os = "linux") {
        dirs.push(PathBuf::from("/usr/local/bin"));
        dirs.push(PathBuf::from("/home/linuxbrew/.linuxbrew/bin"));
    } else if cfg!(windows) {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            dirs.push(PathBuf::from(appdata).join("npm"));
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(
                PathBuf::from(local)
                    .join("Microsoft")
                    .join("WinGet")
                    .join("Links"),
            );
        }
        if let Some(home) = dirs::home_dir() {
            dirs.push(home.join("scoop").join("shims"));
        }
    }
    let mut seen = std::collections::HashSet::new();
    dirs.retain(|d| !d.as_os_str().is_empty() && seen.insert(d.clone()));
    dirs
}

fn candidates(name: &str) -> Vec<String> {
    if cfg!(windows) {
        let mut v: Vec<String> = [".exe", ".cmd", ".bat", ".ps1", ""]
            .iter()
            .map(|e| format!("{name}{e}"))
            .collect();
        v.dedup();
        v
    } else {
        vec![name.to_string()]
    }
}

/// Primeiro `name` executável em `dirs`.
pub fn which_in(name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    for d in dirs {
        for c in candidates(name) {
            let p = d.join(&c);
            if is_executable(&p) {
                return Some(p);
            }
        }
    }
    None
}

pub fn which(name: &str) -> Option<PathBuf> {
    which_in(name, &search_dirs(&[]))
}

fn is_executable(p: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(p) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Caminho real (sem symlink). No Windows tira o prefixo `\\?\`.
pub fn real_path(p: &Path) -> PathBuf {
    match std::fs::canonicalize(p) {
        Ok(r) => strip_verbatim(r),
        Err(_) => p.to_path_buf(),
    }
}

fn strip_verbatim(p: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        let s = p.to_string_lossy();
        if let Some(rest) = s.strip_prefix(r"\\?\") {
            if !rest.starts_with("UNC\\") {
                return PathBuf::from(rest);
            }
        }
    }
    p
}

/// Caminho com `/` para casar padrões (`/lib/node_modules/<pkg>/`).
pub fn slashy(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        s.to_ascii_lowercase()
    } else {
        s
    }
}

/// Primeira versão (`1.2.3`, `v1.2`, `2026.09.18-abc`) de um texto.
pub fn parse_version(text: &str) -> Option<String> {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"v?(\d+\.\d+(?:\.\d+)?(?:[-+][0-9A-Za-z.\-]+)?)").expect("regex")
    });
    re.captures(text).map(|c| c[1].to_string())
}

/// Compara versões pelos números (o sufixo `-pre` fica abaixo da final).
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    fn split(v: &str) -> (Vec<u64>, bool) {
        let v = v.trim().trim_start_matches('v');
        let (core, pre) = match v.find(['-', '+']) {
            Some(i) => (&v[..i], v[i..].starts_with('-')),
            None => (v, false),
        };
        (
            core.split('.')
                .map(|n| n.parse::<u64>().unwrap_or(0))
                .collect(),
            pre,
        )
    }
    let (mut x, xp) = split(a);
    let (mut y, yp) = split(b);
    let n = x.len().max(y.len());
    x.resize(n, 0);
    y.resize(n, 0);
    match x.cmp(&y) {
        Ordering::Equal => match (xp, yp) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            _ => Ordering::Equal,
        },
        o => o,
    }
}

/// Aspas para mostrar um comando copiável no shell do sistema.
pub fn shell_quote(arg: &str) -> String {
    let safe = !arg.is_empty()
        && arg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:=@%+,~".contains(c));
    if safe {
        return arg.to_string();
    }
    if cfg!(windows) {
        format!("\"{}\"", arg.replace('"', "\\\""))
    } else {
        format!("'{}'", arg.replace('\'', "'\\''"))
    }
}

pub fn display_command(program: &str, args: &[String]) -> String {
    let mut s = shell_quote(program);
    for a in args {
        s.push(' ');
        s.push_str(&shell_quote(a));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versao_sai_de_textos_reais() {
        assert_eq!(
            parse_version("2.1.280 (Claude Code)").as_deref(),
            Some("2.1.280")
        );
        assert_eq!(
            parse_version("codex-cli 0.156.0").as_deref(),
            Some("0.156.0")
        );
        assert_eq!(parse_version("goose v1.51.0").as_deref(), Some("1.51.0"));
        assert_eq!(
            parse_version("2026.09.18-9a7762b").as_deref(),
            Some("2026.09.18-9a7762b")
        );
        assert_eq!(parse_version("sem versão"), None);
    }

    #[test]
    fn comparacao_numerica_e_pre_release() {
        assert_eq!(compare_versions("1.10.0", "1.9.9"), Ordering::Greater);
        assert_eq!(compare_versions("v1.2", "1.2.0"), Ordering::Equal);
        assert_eq!(compare_versions("1.2.0-preview.1", "1.2.0"), Ordering::Less);
        assert_eq!(compare_versions("0.9.0", "0.10.0"), Ordering::Less);
    }

    #[test]
    fn aspas_so_quando_precisa() {
        assert_eq!(shell_quote("@openai/codex@latest"), "@openai/codex@latest");
        if !cfg!(windows) {
            assert_eq!(shell_quote("a b"), "'a b'");
            assert_eq!(shell_quote("it's"), "'it'\\''s'");
        }
    }
}
