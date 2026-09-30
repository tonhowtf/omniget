//! Context pruning, as the settings card sees it. The policy, the judges and
//! the sticky store live in `core::llm::prune`; this layer only turns the two
//! settings fields into a `ContextPruner` and swaps it into the Coordinator.
//!
//! The Jev key never touches settings: it goes to the secret store, and the
//! tab only ever learns whether one is stored.

use std::sync::Arc;

use omniget_core::core::llm::prune::jev::JEV_SECRET_ACCOUNT;
use omniget_core::core::llm::prune::{self, ContextPruner, PruneConfig, PruneJudge};
use omniget_core::core::secrets;
use omniget_core::models::settings::LlmSettings;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager, State};

use crate::storage::config;
use crate::storage::settings_effects::EffectId;
use crate::AppState;

pub const ERR_PRUNE_KEY: &str = "ERR_PRUNE_KEY";

/// The two persisted fields over the defaults ported from the originals.
pub fn config_from(settings: &LlmSettings) -> PruneConfig {
    PruneConfig {
        enabled: settings.prune_enabled,
        judge: PruneJudge::parse(&settings.prune_judge),
        min_tokens: settings.prune_min_tokens,
        ..PruneConfig::default()
    }
}

fn has_key() -> bool {
    matches!(secrets::get(secrets::AI_KEYS, JEV_SECRET_ACCOUNT), Ok(Some(k)) if !k.is_empty())
}

/// Builds the pruner the settings ask for and hands it to the Coordinator.
/// Jev selected with no key yields no judge, which is the same as off: no
/// request is ever made without a key.
pub fn install(app: &AppHandle) {
    install_with(app, &config::load_settings(app).llm);
}

/// [`install`] for a given snapshot: the settings effect `llm.pruning`.
pub fn install_with(app: &AppHandle, settings: &LlmSettings) {
    let coordinator = app.state::<AppState>().llm.coordinator();
    let cfg = config_from(settings);
    let judge = prune::judge_for(&cfg);
    let dir = coordinator.dir().map(|p| p.to_path_buf());
    coordinator.set_prune(Arc::new(ContextPruner::new(cfg, judge).with_dir(dir)));
}

#[tauri::command]
pub async fn llm_prune_status(app: AppHandle, state: State<'_, AppState>) -> Result<Value, String> {
    let settings = config::load_settings(&app).llm;
    Ok(json!({
        "enabled": settings.prune_enabled,
        "judge": PruneJudge::parse(&settings.prune_judge).as_str(),
        "has_key": has_key(),
        "omitted": state.llm.coordinator().pruner().omitted_total(),
        "min_tokens": settings.prune_min_tokens,
        "receipts": state.llm.prune_receipts(),
    }))
}

#[tauri::command]
pub async fn llm_prune_set_config(
    app: AppHandle,
    enabled: bool,
    judge: String,
    min_tokens: Option<u32>,
) -> Result<Value, String> {
    let judge = PruneJudge::parse(&judge).as_str().to_string();
    config::update_settings(&app, |current| {
        current.llm.prune_enabled = enabled;
        if let Some(n) = min_tokens {
            current.llm.prune_min_tokens = n.clamp(1_000, 1_000_000);
        }
        current.llm.prune_judge = judge;
    })
    .map_err(|e| format!("Save: {e}"))?;
    // The `llm.pruning` effect installs the pruner; answer once it ran.
    crate::storage::settings_effects::settle(&[EffectId::LlmPruning]);
    Ok(json!({ "ok": true }))
}

/// Stores the TypeSafe key, or forgets it when `key` is empty.
#[tauri::command]
pub async fn llm_prune_set_jev_key(app: AppHandle, key: String) -> Result<Value, String> {
    let key = key.trim();
    let done = if key.is_empty() {
        secrets::delete(secrets::AI_KEYS, JEV_SECRET_ACCOUNT)
    } else {
        secrets::set(secrets::AI_KEYS, JEV_SECRET_ACCOUNT, key)
    };
    done.map_err(|e| format!("{ERR_PRUNE_KEY}: {e}"))?;
    install(&app);
    Ok(json!({ "ok": true, "has_key": has_key() }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_settings_prune_nothing() {
        let cfg = config_from(&LlmSettings::default());
        assert!(!cfg.enabled);
        assert_eq!(cfg.judge, PruneJudge::Local);
        assert!(prune::judge_for(&cfg).is_none());
    }

    #[test]
    fn an_unknown_judge_never_becomes_the_remote_one() {
        let cfg = config_from(&LlmSettings {
            prune_enabled: true,
            prune_judge: "jevv".into(),
            ..LlmSettings::default()
        });
        assert_eq!(cfg.judge, PruneJudge::Local);
    }

    #[test]
    fn the_thresholds_stay_the_ported_ones() {
        let cfg = config_from(&LlmSettings {
            prune_enabled: true,
            prune_judge: "jev".into(),
            ..LlmSettings::default()
        });
        assert_eq!(
            (cfg.recent_turns, cfg.min_chars, cfg.min_tokens),
            (2, 1500, 50_000)
        );
    }
}
