//! Arquivos de configuração reais de uma ferramenta (aba "Configuração" do
//! `/llm/tools`): regras/`AGENTS.md`, agentes, comandos, skills, hooks, MCP e
//! settings, por escopo (global e do projeto escolhido), a partir dos
//! manifestos do `agentkit` e dos `config_paths` da tabela de CLIs.
//!
//! Só caminho, existência e tamanho: nada aqui abre arquivo. Arquivos com
//! nome de credencial (`auth.json`, `credentials*`, `.credentials.json`,
//! `oauth*`, `*token*`) nem entram na lista de entrada da varredura/stats;
//! aparecem só como "pulados".

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::core::agentkit::{targets, Env, Scope};

use super::table;

/// Limite de arquivos contados dentro de uma pasta (tamanho/contagem).
const DIR_WALK_LIMIT: usize = 5000;
/// Profundidade máxima ao somar uma pasta.
const DIR_WALK_DEPTH: usize = 6;

#[derive(Debug, Clone, Serialize)]
pub struct ConfigEntry {
    /// `global | project | local`.
    pub scope: String,
    /// Chave do manifesto (`rules`, `agents`, `mcp`, `settings`…) ou `config`
    /// (caminho da tabela de CLIs).
    pub key: String,
    /// Tipo para `guard_config_stats` (`rule|agent|command|skill|hook|setting|mcp`),
    /// quando o arquivo entra nas estatísticas.
    pub stat_kind: Option<String>,
    pub path: String,
    /// Modelo do manifesto (`~/.claude/agents`).
    pub template: String,
    pub exists: bool,
    pub is_dir: bool,
    /// Bytes (arquivo) ou soma dos arquivos (pasta, sem os de credencial).
    pub size: u64,
    /// Arquivos dentro da pasta (1 para arquivo).
    pub files: u32,
    /// A contagem parou no limite.
    pub truncated: bool,
    /// Pasta que contém outras entradas do mesmo escopo (ex.: `~/.cursor` como
    /// pasta de scripts): não é somada nem varrida, para não pegar a casa
    /// inteira da ferramenta.
    pub container: bool,
    /// Local alternativo que a ferramenta também lê.
    pub alt: bool,
    /// Superfície do MCP (`cli`, `vscode`…).
    pub surface: Option<String>,
    /// Formato do arquivo (`json`, `toml`…) quando conhecido.
    pub format: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct ConfigFiles {
    pub tool: String,
    pub name: String,
    /// Manifesto do agentkit usado (mesmo id), se houver.
    pub target: Option<String>,
    pub project_dir: Option<String>,
    pub entries: Vec<ConfigEntry>,
    /// Entrada pronta para `guard_config_stats`, por escopo: tipo → caminhos
    /// existentes (só JSON/JSONC para hook/setting/mcp).
    pub stats_input: BTreeMap<String, BTreeMap<String, Vec<String>>>,
    /// Entrada pronta para `guard_scan_installed`, por escopo.
    pub scan_paths: BTreeMap<String, Vec<String>>,
    /// Arquivos pulados por nome de credencial (nunca abertos).
    pub skipped: Vec<String>,
    pub total_bytes: u64,
}

/// Nome de arquivo que pode guardar credencial: nunca abrir, nem varrer.
pub fn is_credential_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n == "auth.json"
        || n == ".credentials.json"
        || n.starts_with("credentials")
        || n.starts_with(".credentials")
        || n.starts_with("oauth")
        || n.contains("token")
        || n.ends_with(".pem")
        || n.ends_with(".key")
        || n == ".env"
        || n.starts_with(".env.")
}

fn is_credential_path(p: &Path) -> bool {
    p.file_name()
        .and_then(|n| n.to_str())
        .map(is_credential_name)
        .unwrap_or(false)
}

/// Chave do manifesto → tipo das estatísticas.
pub fn stat_kind_of(key: &str) -> Option<&'static str> {
    match key {
        "rules" | "scoped_rules" => Some("rule"),
        "agents" => Some("agent"),
        "commands" | "prompts" => Some("command"),
        "skills" => Some("skill"),
        "hooks" => Some("hook"),
        "settings" | "statusline" | "permissions" => Some("setting"),
        "mcp" => Some("mcp"),
        _ => None,
    }
}

/// Ordem de exibição das chaves.
fn key_rank(key: &str) -> usize {
    const ORDER: &[&str] = &[
        "rules",
        "scoped_rules",
        "agents",
        "commands",
        "prompts",
        "skills",
        "hooks",
        "hook_scripts",
        "mcp",
        "settings",
        "permissions",
        "statusline",
        "statusline_scripts",
        "plugins",
        "mods",
        "loops",
        "workflows",
        "policies",
        "config",
    ];
    ORDER.iter().position(|k| *k == key).unwrap_or(ORDER.len())
}

fn is_json_file(p: &Path) -> bool {
    matches!(
        p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()),
        Some(ref e) if e == "json" || e == "jsonc"
    )
}

/// Tamanho e contagem sem abrir nada (só metadados); pula credenciais.
fn measure(path: &Path, skipped: &mut BTreeSet<String>) -> (bool, bool, u64, u32, bool) {
    let Ok(meta) = std::fs::metadata(path) else {
        return (false, false, 0, 0, false);
    };
    if meta.is_file() {
        return (true, false, meta.len(), 1, false);
    }
    let mut size = 0u64;
    let mut files = 0u32;
    let mut truncated = false;
    let walker = walkdir::WalkDir::new(path)
        .max_depth(DIR_WALK_DEPTH)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            let n = e.file_name().to_string_lossy();
            !(e.depth() > 0 && (n == "node_modules" || n == ".git"))
        });
    for e in walker.flatten() {
        if !e.file_type().is_file() {
            continue;
        }
        if is_credential_path(e.path()) {
            skipped.insert(e.path().display().to_string());
            continue;
        }
        if files as usize >= DIR_WALK_LIMIT {
            truncated = true;
            break;
        }
        files += 1;
        size += e.metadata().map(|m| m.len()).unwrap_or(0);
    }
    (true, true, size, files, truncated)
}

struct Candidate {
    scope: Scope,
    key: String,
    template: String,
    path: PathBuf,
    alt: bool,
    surface: Option<String>,
    format: Option<String>,
}

/// Arquivos de configuração de `tool_id` nos escopos global, local e do projeto.
pub fn config_files(tool_id: &str, project: Option<&Path>) -> Result<ConfigFiles, String> {
    let env = Env::system().map_err(|e| e.to_string())?;
    config_files_in(&env, tool_id, project)
}

/// Mesma coisa com um `Env` explícito (testes usam `Env::sandbox`).
pub fn config_files_in(
    env: &Env,
    tool_id: &str,
    project: Option<&Path>,
) -> Result<ConfigFiles, String> {
    let cli = table::get(tool_id);
    let target = targets::target(tool_id);
    if cli.is_none() && target.is_none() {
        return Err(format!("{}: ferramenta {tool_id}", super::ERR_NOT_FOUND));
    }
    let name = cli
        .map(|c| c.name.clone())
        .or_else(|| target.map(|t| t.name.clone()))
        .unwrap_or_else(|| tool_id.to_string());

    let mut cands: Vec<Candidate> = Vec::new();
    let mut scopes = vec![Scope::Global];
    if project.is_some() {
        scopes.push(Scope::Project);
        scopes.push(Scope::Local);
    }
    if let Some(t) = target {
        for scope in &scopes {
            for (key, spec) in &t.paths {
                if let Some(tpl) = spec.template(*scope, env.os) {
                    if let Some(p) = env.expand(tpl, project) {
                        cands.push(Candidate {
                            scope: *scope,
                            key: key.clone(),
                            template: tpl.to_string(),
                            path: p,
                            alt: false,
                            surface: None,
                            format: None,
                        });
                    }
                }
                if *scope != Scope::Local {
                    for tpl in spec.alternatives(*scope) {
                        if let Some(p) = env.expand(tpl, project) {
                            cands.push(Candidate {
                                scope: *scope,
                                key: key.clone(),
                                template: tpl.clone(),
                                path: p,
                                alt: true,
                                surface: None,
                                format: None,
                            });
                        }
                    }
                }
            }
            for m in &t.mcp {
                if *scope == Scope::Local {
                    continue;
                }
                if let Some(tpl) = m.template(*scope, env.os) {
                    if let Some(p) = env.expand(tpl, project) {
                        cands.push(Candidate {
                            scope: *scope,
                            key: "mcp".into(),
                            template: tpl.to_string(),
                            path: p,
                            alt: !m.default,
                            surface: (!m.surface.is_empty()).then(|| m.surface.clone()),
                            format: Some(m.format.clone()),
                        });
                    }
                }
            }
        }
    }
    if let Some(c) = cli {
        for raw in &c.config_paths {
            // Pelo `Env` (não pelo HOME do processo): o sandbox dos testes vale.
            let p = env
                .expand(raw, None)
                .unwrap_or_else(|| table::expand_home(raw));
            if p.is_absolute() {
                cands.push(Candidate {
                    scope: Scope::Global,
                    key: "config".into(),
                    template: raw.clone(),
                    path: p,
                    alt: false,
                    surface: None,
                    format: None,
                });
            }
        }
    }

    // Um caminho aparece uma vez por escopo; a chave mais específica vence
    // (settings.json serve hooks, settings e statusline: fica "hooks").
    cands.sort_by(|a, b| {
        (a.scope, key_rank(&a.key), a.alt).cmp(&(b.scope, key_rank(&b.key), b.alt))
    });
    let mut seen: BTreeSet<(Scope, PathBuf)> = BTreeSet::new();
    let mut skipped: BTreeSet<String> = BTreeSet::new();
    let mut out = ConfigFiles {
        tool: tool_id.to_string(),
        name,
        target: target.map(|t| t.id.clone()),
        project_dir: project.map(|p| p.display().to_string()),
        ..Default::default()
    };
    let all_paths: Vec<(Scope, PathBuf)> =
        cands.iter().map(|c| (c.scope, c.path.clone())).collect();
    for c in cands {
        if !seen.insert((c.scope, c.path.clone())) {
            continue;
        }
        if is_credential_path(&c.path) {
            skipped.insert(c.path.display().to_string());
            continue;
        }
        let container = all_paths
            .iter()
            .any(|(s, p)| *s == c.scope && p != &c.path && p.starts_with(&c.path));
        let (exists, is_dir, size, files, truncated) = if container {
            (c.path.is_dir(), true, 0, 0, false)
        } else {
            measure(&c.path, &mut skipped)
        };
        let scope = c.scope.as_str().to_string();
        let mut stat_kind = (!container)
            .then(|| stat_kind_of(&c.key).map(str::to_string))
            .flatten();
        if let Some(k) = stat_kind.as_deref() {
            // hook/setting/mcp só são lidos pelas estatísticas como JSON.
            if matches!(k, "hook" | "setting" | "mcp") && (is_dir || !is_json_file(&c.path)) {
                stat_kind = None;
            }
        }
        if exists {
            out.total_bytes += size;
            if let Some(k) = &stat_kind {
                out.stats_input
                    .entry(scope.clone())
                    .or_default()
                    .entry(k.clone())
                    .or_default()
                    .push(c.path.display().to_string());
            }
            let scannable = !container
                && (stat_kind_of(&c.key).is_some()
                    || matches!(
                        c.key.as_str(),
                        "hook_scripts" | "statusline_scripts" | "plugins" | "mods"
                    ));
            if scannable {
                out.scan_paths
                    .entry(scope.clone())
                    .or_default()
                    .push(c.path.display().to_string());
            }
        }
        out.entries.push(ConfigEntry {
            scope,
            key: c.key,
            stat_kind,
            path: c.path.display().to_string(),
            template: c.template,
            exists,
            is_dir,
            size,
            files,
            truncated,
            container,
            alt: c.alt,
            surface: c.surface,
            format: c.format,
        });
    }
    out.skipped = skipped.into_iter().collect();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::agentkit::Os;

    #[test]
    fn nomes_de_credencial() {
        for n in [
            "auth.json",
            ".credentials.json",
            "credentials",
            "credentials.toml",
            "oauth_creds.json",
            "github_token",
            "access-token.json",
            ".env",
        ] {
            assert!(is_credential_name(n), "{n}");
        }
        for n in [
            "settings.json",
            "config.toml",
            "AGENTS.md",
            "mcp.json",
            "hooks.json",
        ] {
            assert!(!is_credential_name(n), "{n}");
        }
    }

    #[test]
    fn lista_claude_sem_credencial() {
        let home = std::env::temp_dir().join(format!("omniget-cfgfiles-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let claude = home.join(".claude");
        std::fs::create_dir_all(claude.join("agents")).unwrap();
        std::fs::write(claude.join("agents").join("a.md"), "---\nname: a\n---\nhi").unwrap();
        std::fs::write(claude.join("settings.json"), "{}").unwrap();
        std::fs::write(claude.join(".credentials.json"), "secret").unwrap();
        std::fs::write(claude.join("agents").join("oauth.json"), "secret").unwrap();
        let project = home.join("proj");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("CLAUDE.md"), "# rules").unwrap();

        let env = Env::sandbox(&home, Os::Macos);
        let cf = config_files_in(&env, "claude", Some(&project)).unwrap();
        let agents = cf
            .entries
            .iter()
            .find(|e| e.scope == "global" && e.key == "agents")
            .unwrap();
        assert!(agents.exists && agents.is_dir);
        assert_eq!(agents.files, 1);
        // settings.json aparece uma vez (como hooks), com tipo hook.
        let settings: Vec<_> = cf
            .entries
            .iter()
            .filter(|e| e.scope == "global" && e.path.ends_with("settings.json"))
            .collect();
        assert_eq!(settings.len(), 1);
        assert_eq!(settings[0].stat_kind.as_deref(), Some("hook"));
        assert!(cf.skipped.iter().any(|s| s.ends_with("oauth.json")));
        assert!(cf
            .stats_input
            .get("project")
            .and_then(|m| m.get("rule"))
            .is_some_and(|v| v.iter().any(|p| p.ends_with("CLAUDE.md"))));
        assert!(!cf
            .scan_paths
            .values()
            .flatten()
            .any(|p| p.contains("credentials")));
        let _ = std::fs::remove_dir_all(&home);
    }

    /// Máquina real, só metadados: `cargo test -p omniget-core --lib real_config_files -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_config_files() {
        for tool in ["claude", "codex", "cursor"] {
            let cf = config_files(tool, std::env::current_dir().ok().as_deref()).unwrap();
            let existing = cf.entries.iter().filter(|e| e.exists).count();
            println!(
                "{tool}: {} entradas, {existing} existem, {} bytes, {} puladas, stats {:?}",
                cf.entries.len(),
                cf.total_bytes,
                cf.skipped.len(),
                cf.stats_input
                    .iter()
                    .map(|(k, v)| (k.clone(), v.keys().cloned().collect::<Vec<_>>()))
                    .collect::<Vec<_>>()
            );
            for s in &cf.skipped {
                println!("  pulado: {s}");
            }
        }
    }
}
