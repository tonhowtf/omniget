use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use tokio::sync::OnceCell;

use super::exec::{self, run_capture, slashy};
use super::table::{expand_home, CliTool};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnerMethod {
    Native,
    Bun,
    Pnpm,
    Npm,
    Brew,
    Pipx,
    Uv,
    Pip,
    Cargo,
    Omniget,
    App,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    pub display: String,
    pub lock_key: String,
    #[serde(default)]
    pub via_shell: bool,
}

impl CommandSpec {
    pub fn new(program: impl Into<String>, args: Vec<String>, lock_key: impl Into<String>) -> Self {
        let program = program.into();
        let display = exec::display_command(&program, &args);
        Self {
            program,
            args,
            env: BTreeMap::new(),
            display,
            lock_key: lock_key.into(),
            via_shell: false,
        }
    }

    pub fn shell(line: impl Into<String>, lock_key: impl Into<String>) -> Self {
        let line = line.into();
        let (program, args) = if cfg!(windows) {
            (
                "powershell".to_string(),
                vec![
                    "-NoProfile".into(),
                    "-ExecutionPolicy".into(),
                    "RemoteSigned".into(),
                    "-Command".into(),
                    line.clone(),
                ],
            )
        } else {
            ("sh".to_string(), vec!["-c".into(), line.clone()])
        };
        Self {
            program,
            args,
            env: BTreeMap::new(),
            display: line,
            lock_key: lock_key.into(),
            via_shell: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Owner {
    pub method: OwnerMethod,
    pub proven: bool,
    pub prefix: Option<PathBuf>,
    pub evidence: String,
    pub update: Option<CommandSpec>,
    pub manual: Option<String>,
}

impl Owner {
    pub fn unknown(evidence: impl Into<String>, manual: Option<String>) -> Self {
        Self {
            method: OwnerMethod::Unknown,
            proven: false,
            prefix: None,
            evidence: evidence.into(),
            update: None,
            manual,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Candidate {
    Native { marker: PathBuf },
    Bun { pkg: String },
    Pnpm { pkg: String },
    Npm { prefix: PathBuf, pkg: String },
    Brew { name: String, cask: bool },
    Pipx { pkg: String },
    Uv { pkg: String },
    Pip { pkg: String },
    Cargo { krate: String },
    Omniget,
    Unknown,
}

pub fn classify(tool: &CliTool, resolved: &Path, real: &Path) -> Candidate {
    let real_s = slashy(real);
    let res_s = slashy(resolved);
    let norm = |s: &str| {
        if cfg!(windows) {
            s.to_ascii_lowercase()
        } else {
            s.to_string()
        }
    };

    if let Some(su) = &tool.self_update {
        for m in &su.markers {
            let marker = expand_home(m);
            let raw = slashy(&marker);
            let real_marker = slashy(&exec::real_path(&marker));
            let inside = |p: &str, ms: &str| p == ms || p.starts_with(&format!("{ms}/"));
            if inside(&real_s, &raw) || inside(&res_s, &raw) || inside(&real_s, &real_marker) {
                return Candidate::Native { marker };
            }
        }
    }

    if let Some(data) = crate::core::paths::app_data_dir() {
        let agents = slashy(&exec::real_path(&data.join("agents").join("bin")));
        if real_s.starts_with(&format!("{agents}/")) {
            return Candidate::Omniget;
        }
    }

    if let Some(b) = &tool.brew {
        let short = b.short();
        let keg = if b.cask {
            format!("/Caskroom/{short}/")
        } else {
            format!("/Cellar/{short}/")
        };
        if real_s.contains(&keg) {
            return Candidate::Brew {
                name: b.name.clone(),
                cask: b.cask,
            };
        }
    }

    if let Some(pkg) = &tool.npm {
        let seg = norm(&format!("/node_modules/{pkg}/"));
        if real_s.contains(&norm("/.bun/install/global/node_modules/")) && real_s.contains(&seg) {
            return Candidate::Bun { pkg: pkg.clone() };
        }
        if real_s.contains("/pnpm/") && real_s.contains("/global/") && real_s.contains(&seg) {
            return Candidate::Pnpm { pkg: pkg.clone() };
        }
        let lib_seg = norm(&format!("/lib/node_modules/{pkg}/"));
        if let Some(i) = real_s.find(&lib_seg) {
            return Candidate::Npm {
                prefix: PathBuf::from(&real_s[..i]),
                pkg: pkg.clone(),
            };
        }
        if cfg!(windows) {
            if let Some(i) = real_s.find(&seg) {
                return Candidate::Npm {
                    prefix: PathBuf::from(&real_s[..i]),
                    pkg: pkg.clone(),
                };
            }
            if let Some(dir) = resolved.parent() {
                let manifest = pkg
                    .split('/')
                    .fold(dir.join("node_modules"), |acc, s| acc.join(s))
                    .join("package.json");
                if manifest.exists() {
                    return Candidate::Npm {
                        prefix: dir.to_path_buf(),
                        pkg: pkg.clone(),
                    };
                }
            }
        }
    }

    if let Some(pkg) = &tool.pip {
        if real_s.contains(&norm(&format!("/pipx/venvs/{pkg}/"))) {
            return Candidate::Pipx { pkg: pkg.clone() };
        }
        if real_s.contains(&norm(&format!("/uv/tools/{pkg}/"))) {
            return Candidate::Uv { pkg: pkg.clone() };
        }
        return Candidate::Pip { pkg: pkg.clone() };
    }

    if let Some(krate) = &tool.cargo {
        let cargo_bin = std::env::var_os("CARGO_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| expand_home("~/.cargo"))
            .join("bin");
        if real_s.starts_with(&format!("{}/", slashy(&cargo_bin))) {
            return Candidate::Cargo {
                krate: krate.clone(),
            };
        }
    }

    Candidate::Unknown
}

#[derive(Default)]
pub struct Probes {
    npm: OnceCell<Option<PathBuf>>,
    npm_version: OnceCell<Option<String>>,
    brew: OnceCell<Option<PathBuf>>,
    brew_prefix: OnceCell<Option<PathBuf>>,
    uv_dir: OnceCell<Option<PathBuf>>,
    pipx_dir: OnceCell<Option<PathBuf>>,
    pnpm_root: OnceCell<Option<PathBuf>>,
    npm_prefix: OnceCell<Option<PathBuf>>,
}

const PROBE: Duration = Duration::from_secs(6);

async fn line_of(program: &Path, args: &[&str]) -> Option<String> {
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let out = run_capture(program, &args, &BTreeMap::new(), PROBE).await?;
    out.ok()
        .then(|| out.stdout.trim().to_string())
        .filter(|s| !s.is_empty())
}

impl Probes {
    pub async fn npm(&self) -> Option<PathBuf> {
        self.npm
            .get_or_init(|| async { exec::which("npm") })
            .await
            .clone()
    }

    pub async fn npm_version(&self) -> Option<String> {
        self.npm_version
            .get_or_init(|| async {
                let npm = self.npm().await?;
                line_of(&npm, &["--version"]).await
            })
            .await
            .clone()
    }

    pub async fn npm_prefix(&self) -> Option<PathBuf> {
        self.npm_prefix
            .get_or_init(|| async {
                let npm = self.npm().await?;
                line_of(&npm, &["prefix", "-g"]).await.map(PathBuf::from)
            })
            .await
            .clone()
    }

    pub async fn brew(&self) -> Option<PathBuf> {
        self.brew
            .get_or_init(|| async { exec::which("brew") })
            .await
            .clone()
    }

    pub async fn brew_prefix(&self) -> Option<PathBuf> {
        self.brew_prefix
            .get_or_init(|| async {
                let brew = self.brew().await?;
                line_of(&brew, &["--prefix"]).await.map(PathBuf::from)
            })
            .await
            .clone()
    }

    pub async fn uv_tool_dir(&self) -> Option<PathBuf> {
        self.uv_dir
            .get_or_init(|| async {
                let uv = exec::which("uv")?;
                line_of(&uv, &["tool", "dir"]).await.map(PathBuf::from)
            })
            .await
            .clone()
    }

    pub async fn pipx_venvs(&self) -> Option<PathBuf> {
        self.pipx_dir
            .get_or_init(|| async {
                let pipx = exec::which("pipx")?;
                line_of(&pipx, &["environment", "--value", "PIPX_LOCAL_VENVS"])
                    .await
                    .map(PathBuf::from)
            })
            .await
            .clone()
    }

    pub async fn pnpm_root(&self) -> Option<PathBuf> {
        self.pnpm_root
            .get_or_init(|| async {
                let pnpm = exec::which("pnpm")?;
                line_of(&pnpm, &["root", "-g"]).await.map(PathBuf::from)
            })
            .await
            .clone()
    }
}

pub fn needs_allow_scripts(npm_version: Option<&str>, tool: &CliTool) -> bool {
    tool.npm_scripts
        && npm_version
            .and_then(|v| v.split('.').next())
            .and_then(|m| m.trim().parse::<u32>().ok())
            .map(|major| major >= 12)
            .unwrap_or(false)
}

pub fn npm_install_spec(
    npm: &Path,
    prefix: Option<&Path>,
    pkg: &str,
    allow_scripts: bool,
) -> CommandSpec {
    let mut args = vec!["install".to_string(), "-g".to_string()];
    if let Some(p) = prefix {
        args.push("--prefix".into());
        args.push(p.to_string_lossy().into_owned());
    }
    if allow_scripts {
        args.push(format!("--allow-scripts={pkg}"));
    }
    args.push(format!("{pkg}@latest"));
    let lock = match prefix {
        Some(p) => format!("npm-global:{}", p.display()),
        None => "npm-global".into(),
    };
    CommandSpec::new(npm.to_string_lossy().into_owned(), args, lock)
}

fn same_dir(a: &Path, b: &Path) -> bool {
    exec::real_path(a) == exec::real_path(b)
}

pub async fn prove(
    tool: &CliTool,
    resolved: &Path,
    real: &Path,
    candidate: Candidate,
    probes: &Probes,
) -> Owner {
    let fallback = super::plan::manual_update_line(tool);
    match candidate {
        Candidate::Native { marker } => {
            let su = tool.self_update.clone().unwrap_or_default();
            let update = if let Some(args) = su.args.clone() {
                let mut full = tool.subcommand.clone();
                full.extend(args);
                Some(CommandSpec::new(
                    resolved.to_string_lossy().into_owned(),
                    full,
                    format!("native:{}", tool.id),
                ))
            } else if su.rerun_installer {
                super::plan::installer_spec(tool)
            } else {
                None
            };
            let manual = update.as_ref().map(|u| u.display.clone()).or(fallback);
            Owner {
                method: OwnerMethod::Native,
                proven: update.is_some(),
                prefix: Some(marker.clone()),
                evidence: format!("real path inside {}", marker.display()),
                update,
                manual,
            }
        }
        Candidate::Omniget => Owner {
            method: OwnerMethod::Omniget,
            proven: true,
            prefix: crate::core::paths::app_data_dir().map(|d| d.join("agents").join("bin")),
            evidence: "downloaded by OmniGet from the ACP registry".into(),
            update: None,
            manual: None,
        },
        Candidate::Bun { pkg } => {
            let root = std::env::var_os("BUN_INSTALL")
                .map(PathBuf::from)
                .unwrap_or_else(|| expand_home("~/.bun"))
                .join("install")
                .join("global")
                .join("node_modules");
            let pkg_root = pkg.split('/').fold(root.clone(), |a, s| a.join(s));
            let bun = exec::which("bun");
            let proven = real.starts_with(exec::real_path(&pkg_root)) && bun.is_some();
            let update = bun.filter(|_| proven).map(|b| {
                CommandSpec::new(
                    b.to_string_lossy().into_owned(),
                    vec!["add".into(), "-g".into(), format!("{pkg}@latest")],
                    "bun-global",
                )
            });
            Owner {
                method: OwnerMethod::Bun,
                proven,
                prefix: Some(root),
                evidence: if proven {
                    "real path in the bun global folder".into()
                } else {
                    "looks like bun, but bun is not on the PATH".into()
                },
                manual: update
                    .as_ref()
                    .map(|u| u.display.clone())
                    .or_else(|| Some(format!("bun add -g {pkg}@latest"))),
                update,
            }
        }
        Candidate::Pnpm { pkg } => {
            let root = probes.pnpm_root().await;
            let proven = root
                .as_ref()
                .map(|r| real.starts_with(exec::real_path(r)))
                .unwrap_or(false);
            let update = if proven {
                exec::which("pnpm").map(|p| {
                    CommandSpec::new(
                        p.to_string_lossy().into_owned(),
                        vec!["add".into(), "-g".into(), format!("{pkg}@latest")],
                        "pnpm-global",
                    )
                })
            } else {
                None
            };
            Owner {
                method: OwnerMethod::Pnpm,
                proven: update.is_some(),
                prefix: root,
                evidence: if proven {
                    "`pnpm root -g` contains the real path".into()
                } else {
                    "looks like pnpm, but `pnpm root -g` does not match".into()
                },
                manual: update
                    .as_ref()
                    .map(|u| u.display.clone())
                    .or_else(|| Some(format!("pnpm add -g {pkg}@latest"))),
                update,
            }
        }
        Candidate::Npm { prefix, pkg } => {
            let own_npm = if cfg!(windows) {
                prefix.join("npm.cmd")
            } else {
                prefix.join("bin").join("npm")
            };
            let (npm, reported) = if own_npm.exists() {
                let r = line_of(&own_npm, &["prefix", "-g"])
                    .await
                    .map(PathBuf::from);
                (Some(own_npm), r)
            } else {
                (probes.npm().await, probes.npm_prefix().await)
            };
            let proven = reported
                .as_ref()
                .map(|r| same_dir(r, &prefix))
                .unwrap_or(false);
            let npm_version = match &npm {
                Some(n) => line_of(n, &["--version"]).await,
                None => None,
            };
            let allow = needs_allow_scripts(npm_version.as_deref(), tool);
            let spec = npm
                .as_ref()
                .map(|n| npm_install_spec(n, Some(&prefix), &pkg, allow));
            let manual = spec.as_ref().map(|s| s.display.clone()).or_else(|| {
                Some(format!(
                    "npm install -g --prefix {} {pkg}@latest",
                    prefix.display()
                ))
            });
            Owner {
                method: OwnerMethod::Npm,
                proven,
                prefix: Some(prefix.clone()),
                evidence: match (&reported, proven) {
                    (_, true) => format!("`npm prefix -g` = {}", prefix.display()),
                    (Some(r), false) => {
                        format!("`npm prefix -g` = {} ≠ {}", r.display(), prefix.display())
                    }
                    (None, false) => "npm did not answer `prefix -g`".into(),
                },
                update: spec.filter(|_| proven),
                manual,
            }
        }
        Candidate::Brew { name, cask } => {
            let brew = probes.brew().await;
            let root = if cask {
                probes.brew_prefix().await.map(|p| {
                    p.join("Caskroom")
                        .join(name.rsplit('/').next().unwrap_or(&name))
                })
            } else {
                match &brew {
                    Some(b) => line_of(b, &["--prefix", &name]).await.map(PathBuf::from),
                    None => None,
                }
            };
            let keg = root.as_ref().map(|r| exec::real_path(r));
            let proven =
                brew.is_some() && keg.as_ref().map(|k| real.starts_with(k)).unwrap_or(false);
            let mut args = vec!["upgrade".to_string()];
            if cask {
                args.push("--cask".into());
            }
            args.push(name.clone());
            let update = brew.as_ref().filter(|_| proven).map(|b| {
                CommandSpec::new(b.to_string_lossy().into_owned(), args.clone(), "homebrew")
            });
            Owner {
                method: OwnerMethod::Brew,
                proven,
                prefix: keg.clone(),
                evidence: match (&keg, proven) {
                    (Some(k), true) => format!("keg checked: {}", k.display()),
                    (Some(k), false) => {
                        format!("keg {} does not contain the real path", k.display())
                    }
                    (None, _) => "brew did not answer with its prefix".into(),
                },
                update,
                manual: Some(exec::display_command("brew", &args)),
            }
        }
        Candidate::Pipx { pkg } => {
            let venvs = probes.pipx_venvs().await;
            let proven = venvs
                .as_ref()
                .map(|v| real.starts_with(exec::real_path(&v.join(&pkg))))
                .unwrap_or(false);
            let update = exec::which("pipx").filter(|_| proven).map(|p| {
                CommandSpec::new(
                    p.to_string_lossy().into_owned(),
                    vec!["upgrade".into(), pkg.clone()],
                    "pipx",
                )
            });
            Owner {
                method: OwnerMethod::Pipx,
                proven: update.is_some(),
                prefix: venvs,
                evidence: if proven {
                    "pipx venv checked".into()
                } else {
                    "looks like pipx, but the venv does not match".into()
                },
                update,
                manual: Some(format!("pipx upgrade {pkg}")),
            }
        }
        Candidate::Uv { pkg } => {
            let dir = probes.uv_tool_dir().await;
            let proven = dir
                .as_ref()
                .map(|v| real.starts_with(exec::real_path(&v.join(&pkg))))
                .unwrap_or(false);
            let update = exec::which("uv").filter(|_| proven).map(|p| {
                CommandSpec::new(
                    p.to_string_lossy().into_owned(),
                    vec!["tool".into(), "upgrade".into(), pkg.clone()],
                    "uv-tool",
                )
            });
            Owner {
                method: OwnerMethod::Uv,
                proven: update.is_some(),
                prefix: dir,
                evidence: if proven {
                    "`uv tool dir` contains the real path".into()
                } else {
                    "looks like uv, but `uv tool dir` does not match".into()
                },
                update,
                manual: Some(format!("uv tool upgrade {pkg}")),
            }
        }
        Candidate::Pip { pkg } => {
            let python = std::fs::read(resolved).ok().and_then(|bytes| {
                let head = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).into_owned();
                let line = head.lines().next()?.strip_prefix("#!")?.trim().to_string();
                let exe = line.split_whitespace().next()?.to_string();
                exe.contains("python").then(|| PathBuf::from(exe))
            });
            let shown = match &python {
                Some(py) => run_capture(
                    py,
                    &["-m".into(), "pip".into(), "show".into(), pkg.clone()],
                    &BTreeMap::new(),
                    PROBE,
                )
                .await
                .map(|o| o.ok())
                .unwrap_or(false),
                None => false,
            };
            let args = vec![
                "-m".to_string(),
                "pip".into(),
                "install".into(),
                "-U".into(),
                pkg.clone(),
            ];
            let update = python.as_ref().filter(|_| shown).map(|py| {
                CommandSpec::new(
                    py.to_string_lossy().into_owned(),
                    args.clone(),
                    format!("pip:{}", py.display()),
                )
            });
            Owner {
                method: if shown {
                    OwnerMethod::Pip
                } else {
                    OwnerMethod::Unknown
                },
                proven: update.is_some(),
                prefix: python.clone(),
                evidence: if shown {
                    "`pip show` from the shebang python confirms the package".into()
                } else {
                    "no package manager recognized the binary".into()
                },
                update,
                manual: Some(format!("python -m pip install -U {pkg}")),
            }
        }
        Candidate::Cargo { krate } => {
            let home = std::env::var_os("CARGO_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| expand_home("~/.cargo"));
            let listed = std::fs::read_to_string(home.join(".crates2.json"))
                .map(|s| s.contains(&format!("\"{krate} ")))
                .unwrap_or(false);
            let update = exec::which("cargo").filter(|_| listed).map(|c| {
                CommandSpec::new(
                    c.to_string_lossy().into_owned(),
                    vec!["install".into(), krate.clone()],
                    "cargo-install",
                )
            });
            Owner {
                method: OwnerMethod::Cargo,
                proven: update.is_some(),
                prefix: Some(home.join("bin")),
                evidence: if listed {
                    ".crates2.json lists the crate".into()
                } else {
                    "the crate is not in .crates2.json".into()
                },
                update,
                manual: Some(format!("cargo install {krate}")),
            }
        }
        Candidate::Unknown => Owner::unknown("no known package manager owns this path", fallback),
    }
}
