//! Tauri commands for OrcaRouter.
//!
//! This layer is deliberately thin: every decision that could be wrong lives in
//! `omniget_core::core::orcarouter` and `::orcarouter_login`, which carry the
//! unit tests. Here we only marshal IPC arguments and hold the per-app login
//! state.
//!
//! The API key never crosses this boundary. The frontend gets model metadata
//! and a masked tail; the catalog fetch and the inference request read the key
//! from the core store on the Rust side.

use omniget_core::core::ai::{self, AiConfigView, AiProvider};
use omniget_core::core::orcarouter_login::{
    self as login, CatalogView, LoginOutcome, LoginPending, LoginState,
};

/// Application-wide OrcaRouter state, managed by Tauri.
#[derive(Default)]
pub struct OrcarouterState {
    pub login: LoginState,
}

/// Start a sign-in and return the URL the user must approve.
///
/// The loopback listener stays inside the service; the frontend only ever sees
/// the URL, the callback address and the attempt id.
#[tauri::command]
pub async fn orcarouter_login_begin(
    state: tauri::State<'_, OrcarouterState>,
) -> Result<LoginPending, String> {
    login::begin_login(&state.login)
        .await
        .map(|pending| pending.info)
}

/// Finish a sign-in using the listener begun by `orcarouter_login_begin`.
#[tauri::command]
pub async fn orcarouter_login_finish(
    state: tauri::State<'_, OrcarouterState>,
    attempt: u64,
) -> Result<LoginOutcome, String> {
    let listener = state
        .login
        .take_listener()
        .ok_or_else(|| "No sign-in is waiting.".to_string())?;
    Ok(login::complete_login(&state.login, attempt, listener).await)
}

/// Cancel an in-flight sign-in. Releases the lock immediately.
#[tauri::command]
pub fn orcarouter_login_cancel(state: tauri::State<'_, OrcarouterState>) -> u64 {
    state.login.release()
}

/// Release login state on pagehide/unmount without waiting for a response.
///
/// Browsers may put the page into the back-forward cache, so busy state and the
/// pending attempt are cleared synchronously here. Relying on the invalidated
/// request's own cleanup would leave a restored page permanently busy, because
/// that cleanup correctly refuses to touch state.
#[tauri::command]
pub fn orcarouter_login_pagehide(state: tauri::State<'_, OrcarouterState>) -> u64 {
    state.login.release()
}

/// Save a pasted API key through the same credential seam the PKCE flow uses.
///
/// `key: None` keeps whatever is stored and only moves the provider selection;
/// `Some("")` clears it.
#[tauri::command]
pub async fn orcarouter_set_api_key(
    provider: AiProvider,
    model: String,
    key: Option<String>,
) -> Result<AiConfigView, String> {
    login::set_api_key(provider, model, key).await
}

/// Remove the OrcaRouter credential without touching the other providers.
#[tauri::command]
pub fn orcarouter_sign_out() -> AiConfigView {
    ai::clear_orcarouter_credential().view()
}

/// The capability-filtered model catalog for one entrance.
#[tauri::command]
pub async fn orcarouter_models(
    capability: String,
    modality: Option<String>,
    force: Option<bool>,
) -> Result<CatalogView, String> {
    login::models_for(&capability, modality.as_deref(), force.unwrap_or(false)).await
}

/// Re-validate a stored model id against the current catalog.
#[tauri::command]
pub async fn orcarouter_validate_model(
    model: String,
    capability: String,
    modality: Option<String>,
) -> Result<Option<String>, String> {
    login::validate_model(&model, &capability, modality.as_deref()).await
}
