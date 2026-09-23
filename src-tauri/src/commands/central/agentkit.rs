//! Comandos `agentkit_*` da Central: catálogo universal → arquivos de cada
//! ferramenta (plano, diff, aplicar, desinstalar, drift, restaurar, importar).
//! Toda a lógica mora em `omniget_core::core::agentkit`; aqui só há a ponte.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::Value;

use omniget_core::core::agentkit::{
    self as ak,
    model::{Component, ComponentKind},
    plan::{ConflictPolicy, PlanRequest},
    Env, Scope,
};

pub(crate) fn env() -> Result<Env, String> {
    Env::system().map_err(String::from)
}

pub(crate) fn project(p: Option<String>) -> Option<PathBuf> {
    p.filter(|s| !s.trim().is_empty()).map(PathBuf::from)
}

pub(crate) fn to_value<T: serde::Serialize>(v: T) -> Result<Value, String> {
    serde_json::to_value(v).map_err(|e| format!("AGENTKIT_JSON: {e}"))
}

async fn blocking<T, F>(f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| format!("AGENTKIT_TASK: {e}"))?
}

/// Every tool adapter (embedded manifests + user overrides).
#[tauri::command]
pub async fn agentkit_targets() -> Result<Value, String> {
    let env = env()?;
    to_value(ak::targets::load_targets(&env))
}

/// Which tools are installed here, their version and the scopes available for
/// `project_dir`.
#[tauri::command]
pub async fn agentkit_detect(project_dir: Option<String>) -> Result<Value, String> {
    blocking(move || {
        let env = env()?;
        to_value(ak::detect::detect_all(
            &env,
            project(project_dir).as_deref(),
        ))
    })
    .await
}

/// Parses a component without installing it, with its compatibility per tool.
/// Give either `path` (file or folder on disk), or `entry` + `files` (text
/// contents by relative path), or a catalog `item` + `files`.
#[tauri::command]
pub async fn agentkit_parse_preview(
    kind: Option<String>,
    path: Option<String>,
    entry: Option<String>,
    files: Option<BTreeMap<String, String>>,
    item: Option<Value>,
) -> Result<Value, String> {
    blocking(move || {
        let raw: Option<ak::parse::RawFiles> =
            files.map(|m| m.into_iter().map(|(k, v)| (k, v.into_bytes())).collect());
        let mut c: Component = match (item, path, entry, raw) {
            (Some(item), _, _, Some(raw)) => ak::parse::parse_item(&item, &raw)?,
            (_, Some(p), _, _) => {
                let k = kind
                    .as_deref()
                    .and_then(ComponentKind::parse)
                    .ok_or("AGENTKIT_KIND: kind is required with path")?;
                ak::parse::parse_path(k, &PathBuf::from(p))?
            }
            (_, _, Some(e), Some(raw)) => {
                let k = kind
                    .as_deref()
                    .and_then(ComponentKind::parse)
                    .ok_or("AGENTKIT_KIND: kind is required with entry")?;
                ak::parse::parse_raw(k, &e, &raw)?
            }
            _ => return Err("AGENTKIT_ARGS: pass path, entry+files or item+files".into()),
        };
        let env = env()?;
        c.compat = ak::convert::compute_compat(&c, &ak::targets::load_targets(&env));
        to_value(c)
    })
    .await
}

/// Plans an install. Components come as canonical JSON (`components_json`) and/or
/// catalog ids (`catalog_ids`, resolved by the catalog; `path:<kind>:<path>`
/// works without one). `policy`: `rename` (default) | `skip` | `overwrite`.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn agentkit_plan(
    components_json: Option<Value>,
    catalog_ids: Option<Vec<String>>,
    targets: Vec<String>,
    scope: Option<String>,
    project_dir: Option<String>,
    policy: Option<String>,
    secret_values: Option<BTreeMap<String, String>>,
) -> Result<Value, String> {
    blocking(move || {
        let mut components: Vec<Component> = match components_json {
            Some(Value::Array(a)) => a
                .into_iter()
                .map(|v| serde_json::from_value(v).map_err(|e| format!("AGENTKIT_JSON: {e}")))
                .collect::<Result<_, _>>()?,
            Some(Value::Object(o)) => vec![serde_json::from_value(Value::Object(o))
                .map_err(|e| format!("AGENTKIT_JSON: {e}"))?],
            Some(Value::String(s)) => serde_json::from_str::<Vec<Component>>(&s)
                .map_err(|e| format!("AGENTKIT_JSON: {e}"))?,
            _ => vec![],
        };
        for id in catalog_ids.unwrap_or_default() {
            components.push(resolve_any(&id)?);
        }
        if components.is_empty() {
            return Err("AGENTKIT_ARGS: nothing to install".into());
        }
        let policy = match policy.as_deref().unwrap_or("rename") {
            "skip" => ConflictPolicy::Skip,
            "overwrite" => ConflictPolicy::Overwrite,
            _ => ConflictPolicy::Rename,
        };
        let scope = scope.as_deref().map(Scope::parse).transpose()?;
        let env = env()?;
        let plan = ak::plan::plan(
            &env,
            PlanRequest {
                components,
                targets,
                scope,
                project_dir: project(project_dir),
                policy,
                secret_values: secret_values.unwrap_or_default(),
            },
        )?;
        to_value(plan)
    })
    .await
}

/// Applies a plan made by `agentkit_plan` (kept in memory by id).
#[tauri::command]
pub async fn agentkit_apply(plan_id: String) -> Result<Value, String> {
    blocking(move || {
        let plan = ak::plan::get(&plan_id)
            .ok_or_else(|| format!("AGENTKIT_PLAN: plan `{plan_id}` not found (plan again)"))?;
        let env = env()?;
        let report = ak::writer::apply(&env, &plan)?;
        ak::plan::take(&plan_id);
        to_value(report)
    })
    .await
}

/// Removes one install: only the files and merged pieces we wrote.
#[tauri::command]
pub async fn agentkit_uninstall(
    install_id: String,
    project_dir: Option<String>,
    force: Option<bool>,
) -> Result<Value, String> {
    blocking(move || {
        let env = env()?;
        to_value(ak::writer::uninstall(
            &env,
            &install_id,
            project(project_dir).as_deref(),
            force.unwrap_or(false),
        )?)
    })
    .await
}

/// Installs recorded in the global lock and in the project's lock.
#[tauri::command]
pub async fn agentkit_installed(project_dir: Option<String>) -> Result<Value, String> {
    blocking(move || {
        let env = env()?;
        to_value(ak::writer::installed(&env, project(project_dir).as_deref()))
    })
    .await
}

/// Files or merged pieces edited by the user since we wrote them.
#[tauri::command]
pub async fn agentkit_drift(project_dir: Option<String>) -> Result<Value, String> {
    blocking(move || {
        let env = env()?;
        to_value(ak::writer::drift(&env, project(project_dir).as_deref())?)
    })
    .await
}

/// Rolls a transaction (install or uninstall) back from its backups.
#[tauri::command]
pub async fn agentkit_restore(tx: String) -> Result<Value, String> {
    blocking(move || {
        let env = env()?;
        to_value(ak::writer::restore(&env, &tx)?)
    })
    .await
}

/// Backup transactions on disk, newest first.
#[tauri::command]
pub async fn agentkit_transactions() -> Result<Value, String> {
    blocking(move || {
        let env = env()?;
        to_value(ak::writer::transactions(&env))
    })
    .await
}

/// What a tool already has installed, as canonical components (to copy it to
/// another tool with `agentkit_plan`).
#[tauri::command]
pub async fn agentkit_import_installed(
    target: String,
    scope: Option<String>,
    project_dir: Option<String>,
) -> Result<Value, String> {
    blocking(move || {
        let env = env()?;
        let project = project(project_dir);
        let scope = match scope.as_deref() {
            Some(s) => Scope::parse(s)?,
            None if project.is_some() => Scope::Project,
            None => Scope::Global,
        };
        to_value(ak::convert::import_installed(
            &env,
            &target,
            scope,
            project.as_deref(),
        )?)
    })
    .await
}

// ------------------------------------------------------------------ catálogo, compat, perfis, loops

/// Id do catálogo (`cct:agents/…`), perfil (`omniget:profiles/<id>`) ou
/// `path:<kind>:<caminho>` → componente. Instala o resolvedor do catálogo na
/// primeira vez. Rodar em thread bloqueante (o catálogo baixa com `block_on`).
pub fn resolve_any(id: &str) -> Result<Component, String> {
    super::catalog::install_catalog_resolver();
    if let Some(p) = id.strip_prefix("omniget:profiles/") {
        return Ok(ak::profiles::component(p, &Value::Null)?);
    }
    Ok(ak::plan::resolve_id(id)?)
}

/// Compatibilidade de um item com cada ferramenta (nativo / convertido /
/// convertido com perdas / sem suporte), detectadas primeiro, para a UI.
#[tauri::command]
pub async fn agentkit_compat(
    item_id: String,
    project_dir: Option<String>,
) -> Result<Value, String> {
    blocking(move || compat_report(&item_id, project(project_dir).as_deref())).await
}

/// Corpo de [`agentkit_compat`] (também usado pela ponte).
pub fn compat_report(
    item_id: &str,
    project_dir: Option<&std::path::Path>,
) -> Result<Value, String> {
    let c = resolve_any(item_id)?;
    let env = env()?;
    let all = ak::targets::load_targets(&env);
    let compat: BTreeMap<String, ak::model::Compat> = if c.id.starts_with("omniget:profiles/") {
        ak::profiles::compat(&c, &all).into_iter().collect()
    } else {
        ak::convert::compute_compat(&c, &all)
    };
    let detected = ak::detect::detect_all(&env, project_dir);
    let mut rows: Vec<Value> = detected
        .iter()
        .map(|d| {
            let cp = compat
                .get(&d.id)
                .cloned()
                .unwrap_or(ak::model::Compat::Unsupported {
                    reason: "unknown tool".into(),
                });
            serde_json::json!({
                "id": d.id,
                "name": d.name,
                "installed": d.installed,
                "beta": d.beta,
                "enabled_by_default": d.enabled_by_default,
                "tier": d.tier,
                "compat": cp,
            })
        })
        .collect();
    rows.sort_by_key(|r| {
        (
            !r["installed"].as_bool().unwrap_or(false),
            r["compat"]["status"].as_str() == Some("unsupported"),
            r["tier"].as_u64().unwrap_or(9),
        )
    });
    Ok(serde_json::json!({
        "id": c.id,
        "name": c.name,
        "kind": c.kind,
        "description": c.description,
        "targets": rows,
    }))
}

/// Perfis do OmniGet (somente leitura, dev, sem rede, autônomo com guarda,
/// provider alternativo) com os settings que cada um grava.
#[tauri::command]
pub async fn agentkit_profiles() -> Result<Value, String> {
    to_value(ak::profiles::list())
}

/// Planeja um perfil em várias ferramentas de uma vez (`targets` vazio = todas
/// as instaladas que aceitam). Aplicar com `agentkit_apply(plan.id)`.
#[tauri::command]
pub async fn agentkit_profile_plan(
    id: String,
    params: Option<Value>,
    targets: Option<Vec<String>>,
    scope: Option<String>,
    project_dir: Option<String>,
    policy: Option<String>,
) -> Result<Value, String> {
    blocking(move || {
        let env = env()?;
        let scope = scope.as_deref().map(Scope::parse).transpose()?;
        let plan = ak::profiles::plan_profile(
            &env,
            ak::profiles::ProfileRequest {
                id,
                params: params.unwrap_or(Value::Null),
                targets: targets.filter(|t| !t.is_empty()),
                scope,
                project_dir: project(project_dir),
                policy: parse_policy(policy.as_deref()),
            },
        )?;
        to_value(plan)
    })
    .await
}

/// Loop do catálogo → plano de Loop do OmniGet (prompt, intervalo, checagem de
/// parada, orçamento) para o motor de Loops rodar com `runner`.
#[tauri::command]
pub async fn agentkit_loop_plan(
    item_id: String,
    runner: String,
    workspace: Option<String>,
) -> Result<Value, String> {
    blocking(move || {
        let c = resolve_any(&item_id)?;
        to_value(ak::convert::loop_runbook::loop_plan(
            &c,
            &runner,
            workspace.as_deref(),
        )?)
    })
    .await
}

pub fn parse_policy(p: Option<&str>) -> ConflictPolicy {
    match p.unwrap_or("rename") {
        "skip" => ConflictPolicy::Skip,
        "overwrite" => ConflictPolicy::Overwrite,
        _ => ConflictPolicy::Rename,
    }
}

/// Plugins/extensions de cada ferramenta (Claude, Copilot, Droid, Cursor,
/// Gemini, Qwen, OpenCode): ligado/desligado, origem e o que cada um traz.
#[tauri::command]
pub async fn agentkit_plugins_list(project_dir: Option<String>) -> Result<Value, String> {
    blocking(move || {
        let env = env()?;
        to_value(ak::plugins::list(&env, project(project_dir).as_deref()))
    })
    .await
}

/// Liga ou desliga um plugin no arquivo da própria ferramenta (`scope`:
/// user | project | local). Desfazível com `agentkit_restore(tx)`.
#[tauri::command]
pub async fn agentkit_plugin_set_enabled(
    tool: String,
    plugin: String,
    enabled: bool,
    scope: Option<String>,
    project_dir: Option<String>,
) -> Result<Value, String> {
    blocking(move || {
        let env = env()?;
        to_value(ak::plugins::set_enabled(
            &env,
            &tool,
            &plugin,
            enabled,
            scope.as_deref().unwrap_or("user"),
            project(project_dir).as_deref(),
        )?)
    })
    .await
}
