//! A tabela de CLIs: um registro por ferramenta, carregado de `data/tools.json`
//! (embutido no binário). Só dados; quem decide é `detect`, `plan` e `doctor`.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus {
    Active,
    Maintenance,
    Discontinued,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    Cli,
    App,
    #[serde(rename = "cli+app")]
    CliApp,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Installer {
    /// Script oficial para macOS/Linux (`curl -fsSL <url> | <unix_shell>`).
    #[serde(default)]
    pub unix: Option<String>,
    #[serde(default = "default_shell")]
    pub unix_shell: String,
    /// Script oficial para Windows (`irm <url> | iex`).
    #[serde(default)]
    pub windows: Option<String>,
}

fn default_shell() -> String {
    "bash".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Brew {
    pub name: String,
    #[serde(default)]
    pub cask: bool,
}

impl Brew {
    /// Nome sem o tap (`charmbracelet/tap/crush` → `crush`): é o que aparece
    /// em `Cellar/<nome>` e `Caskroom/<nome>`.
    pub fn short(&self) -> &str {
        self.name.rsplit('/').next().unwrap_or(&self.name)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AcpInfo {
    /// Id no registro ACP (`cdn.agentclientprotocol.com`).
    #[serde(default)]
    pub registry_id: Option<String>,
    /// Argumentos do modo ACP nativo do próprio binário.
    #[serde(default)]
    pub native_args: Option<Vec<String>>,
    /// Adaptador fora do registro (programa + argumentos).
    #[serde(default)]
    pub adapter: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SelfUpdate {
    /// Subcomando do auto-updater nativo (`claude update`).
    #[serde(default)]
    pub args: Option<Vec<String>>,
    /// Sem subcomando: rodar de novo o instalador oficial é o update.
    #[serde(default)]
    pub rerun_installer: bool,
    /// Caminhos que provam que o instalador do fornecedor é dono do binário
    /// (o caminho real precisa estar dentro de um deles).
    #[serde(default)]
    pub markers: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Apps {
    #[serde(default)]
    pub macos: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Deprecated {
    /// `~/…` (home), `bin:<nome>` (binário no PATH), `npm:<pacote>` (informativo)
    /// ou caminho relativo (arquivo de projeto).
    pub path: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliTool {
    pub id: String,
    pub name: String,
    pub kind: ToolKind,
    pub status: ToolStatus,
    #[serde(default)]
    pub status_note: Option<String>,
    pub docs: String,
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub binaries: Vec<String>,
    /// Subcomando fixo antes de tudo (Rovo: `acli rovodev …`).
    #[serde(default)]
    pub subcommand: Vec<String>,
    #[serde(default = "default_version_args")]
    pub version_args: Vec<String>,
    #[serde(default)]
    pub apps: Apps,
    #[serde(default)]
    pub npm: Option<String>,
    /// O pacote precisa do postinstall (npm 12 bloqueia e sai com 0).
    #[serde(default)]
    pub npm_scripts: bool,
    #[serde(default)]
    pub pip: Option<String>,
    #[serde(default)]
    pub cargo: Option<String>,
    #[serde(default)]
    pub installer: Installer,
    #[serde(default)]
    pub brew: Option<Brew>,
    #[serde(default)]
    pub winget: Option<String>,
    #[serde(default)]
    pub scoop: Option<String>,
    #[serde(default)]
    pub acp: AcpInfo,
    /// Argumentos do login (vazio = só abrir a ferramenta; ver `login_hint`).
    #[serde(default)]
    pub login: Vec<String>,
    #[serde(default)]
    pub login_hint: Option<String>,
    /// Comando da própria ferramenta que diz se há login.
    #[serde(default)]
    pub auth_status: Option<Vec<String>>,
    /// Saída 0 do `auth_status` significa "logado".
    #[serde(default)]
    pub auth_status_exit_means_auth: bool,
    /// Arquivos cuja **presença** indica login. Nunca são lidos.
    #[serde(default)]
    pub auth_files: Vec<String>,
    /// Variável que move a pasta de config (instâncias/contas).
    #[serde(default)]
    pub config_env: Option<String>,
    #[serde(default)]
    pub self_update: Option<SelfUpdate>,
    #[serde(default)]
    pub auto_updates: bool,
    #[serde(default)]
    pub config_paths: Vec<String>,
    #[serde(default)]
    pub log_paths: Vec<String>,
    #[serde(default)]
    pub deprecated: Vec<Deprecated>,
}

fn default_version_args() -> Vec<String> {
    vec!["--version".into()]
}

impl CliTool {
    /// De onde vem a "versão mais nova".
    pub fn latest_source(&self) -> LatestSource {
        if let Some(p) = &self.npm {
            LatestSource::Npm(p.clone())
        } else if let Some(p) = &self.pip {
            LatestSource::Pypi(p.clone())
        } else if let Some(r) = &self.repo {
            LatestSource::Github(r.clone())
        } else {
            LatestSource::None
        }
    }

    pub fn has_cli(&self) -> bool {
        !self.binaries.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "key", rename_all = "snake_case")]
pub enum LatestSource {
    Npm(String),
    Pypi(String),
    Github(String),
    None,
}

#[derive(Deserialize)]
struct TableFile {
    tools: Vec<CliTool>,
}

const DATA: &str = include_str!("data/tools.json");

/// Todas as ferramentas, na ordem da tabela.
pub fn all() -> &'static [CliTool] {
    static TABLE: OnceLock<Vec<CliTool>> = OnceLock::new();
    TABLE.get_or_init(|| {
        serde_json::from_str::<TableFile>(DATA)
            .map(|t| t.tools)
            .unwrap_or_else(|e| {
                tracing::error!("[clitools] tabela inválida: {e}");
                Vec::new()
            })
    })
}

pub fn get(id: &str) -> Option<&'static CliTool> {
    all().iter().find(|t| t.id == id)
}

/// Expande `~/` para o HOME (respeita `HOME` no unix, útil para testes).
pub fn expand_home(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return rest.split('/').fold(home, |acc, seg| acc.join(seg));
        }
    }
    if path == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    PathBuf::from(path)
}

/// `path` fica dentro de `root` (comparação por componentes, sem symlink).
pub fn is_within(path: &Path, root: &Path) -> bool {
    path.starts_with(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tabela_carrega_e_os_ids_sao_os_do_briefing() {
        let ids: Vec<&str> = all().iter().map(|t| t.id.as_str()).collect();
        assert!(ids.len() >= 30, "tabela curta: {}", ids.len());
        let briefing = [
            "claude",
            "codex",
            "gemini",
            "qwen",
            "opencode",
            "kilo",
            "crush",
            "cursor",
            "copilot",
            "cline",
            "roo",
            "continue",
            "goose",
            "zed",
            "droid",
            "kimi",
            "pi",
            "amp",
            "aider",
            "junie",
            "auggie",
            "grok",
            "kiro",
            "devin",
            "windsurf",
            "warp",
            "trae",
            "qoder",
            "vibe",
            "letta",
            "rovo",
            "antigravity",
            "codebuff",
            "openhands",
        ];
        for id in ids.iter() {
            assert!(briefing.contains(id), "id fora do briefing: {id}");
        }
        let mut sorted = ids.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len(), "id repetido");
    }

    #[test]
    fn nenhum_auth_file_e_lido_so_apontado() {
        // Regra do dono: auth_files são caminhos para checar presença; o
        // registro não guarda conteúdo nenhum.
        for t in all() {
            for f in &t.auth_files {
                assert!(f.starts_with("~/"), "{}: {f}", t.id);
            }
        }
    }

    #[test]
    fn brew_short_tira_o_tap() {
        let b = Brew {
            name: "charmbracelet/tap/crush".into(),
            cask: false,
        };
        assert_eq!(b.short(), "crush");
    }
}
