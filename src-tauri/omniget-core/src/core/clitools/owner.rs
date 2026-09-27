//! Prova de dono: quem instalou o binário que está no PATH.
//!
//! Regra (estudo 76/01 §9.6): **nunca rodar um gerenciador contra uma
//! instalação que ele não criou**. Primeiro o caminho real sugere uma hipótese
//! (`classify`, pura e testada); depois `prove` confirma com o próprio
//! gerenciador (`npm prefix -g`, `brew --prefix <f>`, `uv tool dir`…). Só uma
//! hipótese provada vira comando executável; o resto é comando para copiar.
//!
//! Ordem: auto-updater nativo → bun → pnpm → npm → brew → pipx/uv/pip → cargo
//! → desconhecido.

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
    /// Binário baixado pelo próprio OmniGet (registro ACP).
    Omniget,
    /// App de GUI (atualiza sozinho ou pela loja).
    App,
    Unknown,
}

/// Um comando que pode rodar (ou só ser mostrado).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Texto exato mostrado antes de rodar.
    pub display: String,
    /// Chave do lock: instalações que dividem um prefixo serializam.
    pub lock_key: String,
    /// `curl … | bash` e afins: roda via shell.
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

    /// Linha de shell (instalador oficial). `display` é a própria linha.
    pub fn shell(line: impl Into<String>, lock_key: impl Into<String>) -> Self {
        let line = line.into();
        let (program, args) = if cfg!(windows) {
            (
                "powershell".to_string(),
                vec![
                    "-NoProfile".into(),
                    "-ExecutionPolicy".into(),
                    "Bypass".into(),
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
    /// A hipótese foi confirmada pelo próprio gerenciador.
    pub proven: bool,
    /// Prefixo/raiz provada (npm prefix, keg do brew, pasta do uv…).
    pub prefix: Option<PathBuf>,
    /// Frase curta com a evidência (ou por que não provou).
    pub evidence: String,
    /// Comando de atualização que pode rodar (só quando `proven`).
    pub update: Option<CommandSpec>,
    /// Comando para copiar quando não dá para rodar.
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

/// Hipótese tirada só do caminho.
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

/// Classifica pelo caminho resolvido (`resolved`, o que está no PATH) e pelo
/// real (`real`, sem symlink). Pura: nenhum processo, só `exists()` do shim
/// do Windows.
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

    // 1. Instalador do fornecedor.
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

    // OmniGet (binário do registro ACP).
    if let Some(data) = crate::core::paths::app_data_dir() {
        let agents = slashy(&exec::real_path(&data.join("agents").join("bin")));
        if real_s.starts_with(&format!("{agents}/")) {
            return Candidate::Omniget;
        }
    }

    // Homebrew (keg ou cask): um keg é dono de tudo que tem dentro, até de
    // um node_modules empacotado em libexec, então é conferido antes do npm.
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
        // 2. bun.
        if real_s.contains(&norm("/.bun/install/global/node_modules/")) && real_s.contains(&seg) {
            return Candidate::Bun { pkg: pkg.clone() };
        }
        // 3. pnpm.
        if real_s.contains("/pnpm/") && real_s.contains("/global/") && real_s.contains(&seg) {
            return Candidate::Pnpm { pkg: pkg.clone() };
        }
        // 4. npm: <prefix>/lib/node_modules/<pkg>/ (unix) ou <prefix>/node_modules/<pkg>/ (win).
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
            // Shim `.cmd` ao lado de `node_modules/<pkg>/package.json`.
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

    // 6. Python.
    if let Some(pkg) = &tool.pip {
        if real_s.contains(&norm(&format!("/pipx/venvs/{pkg}/"))) {
            return Candidate::Pipx { pkg: pkg.clone() };
        }
        if real_s.contains(&norm(&format!("/uv/tools/{pkg}/"))) {
            return Candidate::Uv { pkg: pkg.clone() };
        }
        return Candidate::Pip { pkg: pkg.clone() };
    }

    // 7. cargo.
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

/// Respostas dos gerenciadores, calculadas uma vez por varredura.
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

    /// Versão do npm do PATH (decide o `--allow-scripts`).
    pub async fn npm_version(&self) -> Option<String> {
        self.npm_version
            .get_or_init(|| async {
                let npm = self.npm().await?;
                line_of(&npm, &["--version"]).await
            })
            .await
            .clone()
    }

    /// `npm prefix -g` do npm do PATH.
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

/// npm 12 bloqueia o postinstall por padrão e ainda sai com 0; o pacote que
/// precisa dele recebe `--allow-scripts=<pkg>`.
pub fn needs_allow_scripts(npm_version: Option<&str>, tool: &CliTool) -> bool {
    tool.npm_scripts
        && npm_version
            .and_then(|v| v.split('.').next())
            .and_then(|m| m.trim().parse::<u32>().ok())
            .map(|major| major >= 12)
            .unwrap_or(false)
}

/// Comando npm global para `pkg` num prefixo (provado ou pedido).
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

/// Confirma a hipótese com o gerenciador e monta o comando.
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
                evidence: format!("caminho real dentro de {}", marker.display()),
                update,
                manual,
            }
        }
        Candidate::Omniget => Owner {
            method: OwnerMethod::Omniget,
            proven: true,
            prefix: crate::core::paths::app_data_dir().map(|d| d.join("agents").join("bin")),
            evidence: "baixado pelo OmniGet a partir do registro ACP".into(),
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
                    "caminho real no global do bun".into()
                } else {
                    "parece bun, mas o bun não está no PATH".into()
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
                    "`pnpm root -g` contém o caminho real".into()
                } else {
                    "parece pnpm, mas `pnpm root -g` não bate".into()
                },
                manual: update
                    .as_ref()
                    .map(|u| u.display.clone())
                    .or_else(|| Some(format!("pnpm add -g {pkg}@latest"))),
                update,
            }
        }
        Candidate::Npm { prefix, pkg } => {
            // O npm do próprio prefixo (nvm/volta têm um npm por versão) e,
            // se não houver, o do PATH. Os dois precisam responder o mesmo
            // prefixo que o caminho real indica.
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
                    (None, false) => "npm não respondeu `prefix -g`".into(),
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
                    (Some(k), true) => format!("keg conferido: {}", k.display()),
                    (Some(k), false) => format!("keg {} não contém o caminho real", k.display()),
                    (None, _) => "brew não respondeu o prefixo".into(),
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
                    "venv do pipx conferida".into()
                } else {
                    "parece pipx, mas a venv não bate".into()
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
                    "`uv tool dir` contém o caminho real".into()
                } else {
                    "parece uv, mas `uv tool dir` não bate".into()
                },
                update,
                manual: Some(format!("uv tool upgrade {pkg}")),
            }
        }
        Candidate::Pip { pkg } => {
            // Script de console do pip: a primeira linha é `#!<python>`.
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
                    "`pip show` do python do shebang confirma o pacote".into()
                } else {
                    "nenhum gerenciador reconheceu o binário".into()
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
                    ".crates2.json lista o crate".into()
                } else {
                    "o crate não aparece no .crates2.json".into()
                },
                update,
                manual: Some(format!("cargo install {krate}")),
            }
        }
        Candidate::Unknown => Owner::unknown(
            "nenhum gerenciador conhecido é dono deste caminho",
            fallback,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::clitools::table;

    fn t(id: &str) -> &'static CliTool {
        table::get(id).expect(id)
    }

    #[cfg(unix)]
    #[test]
    fn npm_global_vira_prefixo() {
        let real = Path::new(
            "/Users/x/.nvm/versions/node/v22.1.0/lib/node_modules/@anthropic-ai/claude-code/bin/claude.exe",
        );
        let res = Path::new("/Users/x/.nvm/versions/node/v22.1.0/bin/claude");
        assert_eq!(
            classify(t("claude"), res, real),
            Candidate::Npm {
                prefix: PathBuf::from("/Users/x/.nvm/versions/node/v22.1.0"),
                pkg: "@anthropic-ai/claude-code".into()
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn nativo_ganha_de_tudo() {
        let home = dirs::home_dir().unwrap();
        let real = home.join(".local/share/claude/versions/2.1.280");
        let res = home.join(".local/bin/claude");
        assert!(matches!(
            classify(t("claude"), &res, &real),
            Candidate::Native { .. }
        ));
    }

    #[cfg(unix)]
    #[test]
    fn brew_keg_e_cask() {
        let real = Path::new("/opt/homebrew/Cellar/gemini-cli/0.60.0/libexec/bin/gemini");
        assert_eq!(
            classify(t("gemini"), Path::new("/opt/homebrew/bin/gemini"), real),
            Candidate::Brew {
                name: "gemini-cli".into(),
                cask: false
            }
        );
        let real = Path::new("/opt/homebrew/Caskroom/codex/0.156.0/codex-aarch64-apple-darwin");
        assert_eq!(
            classify(t("codex"), Path::new("/opt/homebrew/bin/codex"), real),
            Candidate::Brew {
                name: "codex".into(),
                cask: true
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn keg_com_node_modules_empacotado_e_do_brew() {
        let real = Path::new(
            "/opt/homebrew/Cellar/gemini-cli/0.60.0/libexec/lib/node_modules/@google/gemini-cli/bundle/gemini.js",
        );
        assert_eq!(
            classify(t("gemini"), Path::new("/opt/homebrew/bin/gemini"), real),
            Candidate::Brew {
                name: "gemini-cli".into(),
                cask: false
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn uv_pipx_e_desconhecido() {
        let home = dirs::home_dir().unwrap();
        let real = home.join(".local/share/uv/tools/mistral-vibe/bin/vibe");
        assert_eq!(
            classify(t("vibe"), &home.join(".local/bin/vibe"), &real),
            Candidate::Uv {
                pkg: "mistral-vibe".into()
            }
        );
        let real = home.join(".local/pipx/venvs/aider-chat/bin/aider");
        assert_eq!(
            classify(t("aider"), &home.join(".local/bin/aider"), &real),
            Candidate::Pipx {
                pkg: "aider-chat".into()
            }
        );
        let real = Path::new("/usr/bin/kiro-cli");
        assert_eq!(classify(t("kiro"), real, real), Candidate::Unknown);
    }

    #[test]
    fn allow_scripts_so_no_npm_12_e_com_postinstall() {
        assert!(needs_allow_scripts(Some("12.0.1"), t("claude")));
        assert!(!needs_allow_scripts(Some("11.6.0"), t("claude")));
        assert!(!needs_allow_scripts(Some("12.0.1"), t("codex")));
        assert!(!needs_allow_scripts(None, t("claude")));
    }

    #[test]
    fn npm_spec_mostra_o_comando_exato() {
        let s = npm_install_spec(Path::new("npm"), Some(Path::new("/p")), "opencode-ai", true);
        assert_eq!(
            s.display,
            "npm install -g --prefix /p --allow-scripts=opencode-ai opencode-ai@latest"
        );
        assert_eq!(s.lock_key, "npm-global:/p");
    }
}
