//! Tauri commands for the local profile.
//!
//! Thin on purpose: every one of these resolves the manager from `AppState`,
//! converts base64 at the boundary and hands back the view shape. The seed
//! never crosses this line — the frontend can ask for a signature, never for
//! the key that made it.
//!
//! Errors are the stable codes from `crate::profile`: `ERR_PROFILE_NICKNAME`,
//! `ERR_PROFILE_STORE`, `ERR_PROFILE_SECRET`, `ERR_PROFILE_SIGN`, optionally
//! followed by `": "` and a detail, so the UI can map on the prefix.

use tauri::State;

use crate::profile::ProfileView;
use crate::AppState;

#[tauri::command]
pub async fn profile_get(state: State<'_, AppState>) -> Result<ProfileView, String> {
    state.profile.get().map(|p| p.view())
}

#[tauri::command]
pub async fn profile_set_nickname(
    state: State<'_, AppState>,
    nick: String,
) -> Result<ProfileView, String> {
    state.profile.set_nickname(&nick).map(|p| p.view())
}

#[tauri::command]
pub async fn profile_set_skin(
    state: State<'_, AppState>,
    id: String,
    tint: [u8; 3],
) -> Result<ProfileView, String> {
    state.profile.set_skin(&id, tint).map(|p| p.view())
}
