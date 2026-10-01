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
