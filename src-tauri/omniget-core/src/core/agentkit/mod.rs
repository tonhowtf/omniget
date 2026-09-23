//! `agentkit`: the universal component installer of the Central.
//!
//! One canonical shape per component ([`model`]), one data manifest per coding tool
//! ([`targets`], TOML files embedded at build time), converters from the canonical
//! shape to each tool's files ([`convert`]), and a transactional writer that plans,
//! diffs, backs up, writes atomically, records a lockfile and can undo exactly what
//! it wrote ([`plan`], [`writer`], [`lock`]). Format editors in [`edit`] keep the
//! user's comments, key order and indentation.
//!
//! Every function that touches the disk takes an [`Env`], so tests (and the
//! `OMNIGET_DATA_DIR` test driver) run against a fake home without touching `~`.

pub mod convert;
pub mod detect;
pub mod edit;
pub mod lock;
pub mod model;
pub mod parse;
pub mod plan;
pub mod plugins;
pub mod profiles;
pub mod targets;
pub mod writer;

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

/// Error with a stable code, shown to the front as `"CODE: message"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentkitError {
    pub code: &'static str,
    pub message: String,
}

impl AgentkitError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn io(what: &str, path: &Path, err: &std::io::Error) -> Self {
        Self::new("AGENTKIT_IO", format!("{what} {}: {err}", path.display()))
    }
}

impl std::fmt::Display for AgentkitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for AgentkitError {}

impl From<AgentkitError> for String {
    fn from(e: AgentkitError) -> String {
        e.to_string()
    }
}

pub type Result<T> = std::result::Result<T, AgentkitError>;

/// Operating system a path table is resolved for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Os {
    Macos,
    Linux,
    Windows,
}

impl Os {
    pub fn current() -> Os {
        if cfg!(target_os = "macos") {
            Os::Macos
        } else if cfg!(windows) {
            Os::Windows
        } else {
            Os::Linux
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Os::Macos => "macos",
            Os::Linux => "linux",
            Os::Windows => "windows",
        }
    }
}

/// Where the installer looks and writes. [`Env::system`] is the real machine;
/// tests build one around a temporary home with [`Env::sandbox`].
#[derive(Debug, Clone)]
pub struct Env {
    pub home: PathBuf,
    /// OmniGet's own data dir (`core::paths::app_data_dir`): backups, global lock.
    pub app_data: PathBuf,
    pub os: Os,
    pub xdg_config: PathBuf,
    pub xdg_data: PathBuf,
    /// macOS `~/Library/Application Support`, Linux `xdg_config`, Windows `%APPDATA%`.
    pub app_support: PathBuf,
    /// macOS `~/Library/Application Support`, Linux `xdg_data`, Windows `%LOCALAPPDATA%`.
    pub local_app_data: PathBuf,
    /// `PATH` used to find binaries. `None` = do not look at PATH at all.
    pub path_var: Option<std::ffi::OsString>,
    /// Where `/Applications`-style bundles live (macOS).
    pub applications_dirs: Vec<PathBuf>,
    /// Run `<bin> --version` during detection.
    pub probe_versions: bool,
}

impl Env {
    /// The real machine. Never used by tests.
    pub fn system() -> Result<Env> {
        let home = dirs::home_dir()
            .ok_or_else(|| AgentkitError::new("AGENTKIT_ENV", "no home directory"))?;
        let app_data = crate::core::paths::app_data_dir()
            .ok_or_else(|| AgentkitError::new("AGENTKIT_ENV", "no app data directory"))?;
        let os = Os::current();
        let xdg_config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| home.join(".config"));
        let xdg_data = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| home.join(".local").join("share"));
        let (app_support, local_app_data) = match os {
            Os::Macos => {
                let lib = home.join("Library").join("Application Support");
                (lib.clone(), lib)
            }
            Os::Linux => (xdg_config.clone(), xdg_data.clone()),
            Os::Windows => (
                std::env::var_os("APPDATA")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| home.join("AppData").join("Roaming")),
                std::env::var_os("LOCALAPPDATA")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| home.join("AppData").join("Local")),
            ),
        };
        let mut applications_dirs = Vec::new();
        if os == Os::Macos {
            applications_dirs.push(PathBuf::from("/Applications"));
            applications_dirs.push(home.join("Applications"));
        }
        Ok(Env {
            home,
            app_data,
            os,
            xdg_config,
            xdg_data,
            app_support,
            local_app_data,
            path_var: std::env::var_os("PATH"),
            applications_dirs,
            probe_versions: true,
        })
    }

    /// A self-contained environment rooted at `home` (nothing outside it is read
    /// or written). PATH lookups are off unless the caller sets `path_var`.
    pub fn sandbox(home: &Path, os: Os) -> Env {
        let home = home.to_path_buf();
        let xdg_config = home.join(".config");
        let xdg_data = home.join(".local").join("share");
        let (app_support, local_app_data) = match os {
            Os::Macos => {
                let lib = home.join("Library").join("Application Support");
                (lib.clone(), lib)
            }
            Os::Linux => (xdg_config.clone(), xdg_data.clone()),
            Os::Windows => (
                home.join("AppData").join("Roaming"),
                home.join("AppData").join("Local"),
            ),
        };
        Env {
            app_data: home.join(".omniget-data"),
            applications_dirs: vec![home.join("Applications")],
            home,
            os,
            xdg_config,
            xdg_data,
            app_support,
            local_app_data,
            path_var: None,
            probe_versions: false,
        }
    }

    pub fn agentkit_dir(&self) -> PathBuf {
        self.app_data.join("agentkit")
    }

    /// Expands a path template from a target manifest. Project-relative paths
    /// (no `~`, no `{var}`, not absolute) are joined to `project`; returns `None`
    /// when a project path is asked for without a project.
    pub fn expand(&self, template: &str, project: Option<&Path>) -> Option<PathBuf> {
        let t = template.trim();
        if t.is_empty() {
            return None;
        }
        let (base, rest): (PathBuf, &str) = if t == "~" {
            (self.home.clone(), "")
        } else if let Some(rest) = t.strip_prefix("~/") {
            (self.home.clone(), rest)
        } else if t.starts_with('{') {
            let end = t.find('}')?;
            let var = &t[1..end];
            let rest = t[end + 1..].trim_start_matches('/');
            let base = match var {
                "home" => self.home.clone(),
                "app_support" => self.app_support.clone(),
                "xdg_config" => self.xdg_config.clone(),
                "xdg_data" => self.xdg_data.clone(),
                "local_app_data" => self.local_app_data.clone(),
                "vscode_user" => self.app_support.join("Code").join("User"),
                "vscode_global_storage" => self
                    .app_support
                    .join("Code")
                    .join("User")
                    .join("globalStorage"),
                "documents" => self.home.join("Documents"),
                "project" => project?.to_path_buf(),
                _ => return None,
            };
            (base, rest)
        } else if is_absolute_template(t) {
            return Some(PathBuf::from(t));
        } else {
            (project?.to_path_buf(), t)
        };
        let mut out = base;
        for part in rest.split('/').filter(|p| !p.is_empty() && *p != ".") {
            out.push(part);
        }
        Some(out)
    }
}

fn is_absolute_template(t: &str) -> bool {
    t.starts_with('/')
        || (t.len() > 2 && t.as_bytes()[1] == b':' && t.as_bytes()[0].is_ascii_alphabetic())
}

/// Install scope.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    Project,
    Global,
    Local,
    Managed,
}

impl Scope {
    pub fn as_str(self) -> &'static str {
        match self {
            Scope::Project => "project",
            Scope::Global => "global",
            Scope::Local => "local",
            Scope::Managed => "managed",
        }
    }

    pub fn parse(s: &str) -> Result<Scope> {
        match s.trim().to_ascii_lowercase().as_str() {
            "project" => Ok(Scope::Project),
            "global" | "user" => Ok(Scope::Global),
            "local" => Ok(Scope::Local),
            "managed" | "enterprise" => Ok(Scope::Managed),
            other => Err(AgentkitError::new(
                "AGENTKIT_SCOPE",
                format!("unknown scope `{other}`"),
            )),
        }
    }

    /// Scopes that live under a project directory.
    pub fn is_project_bound(self) -> bool {
        matches!(self, Scope::Project | Scope::Local)
    }
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(bytes))
}

pub(crate) fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}
