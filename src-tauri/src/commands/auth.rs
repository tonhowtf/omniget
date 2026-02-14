use crate::platforms::hotmart::auth::authenticate;
use crate::AppState;

#[tauri::command]
pub async fn hotmart_login(
    state: tauri::State<'_, AppState>,
    email: String,
    password: String,
) -> Result<String, String> {
    tracing::info!("Comando hotmart_login invocado para {}", email);

    match authenticate(&email, &password).await {
        Ok(session) => {
            let token = session.token.clone();
            let mut guard = state.hotmart_session.lock().await;
            *guard = Some(session);
            tracing::info!("Sessão Hotmart salva no state global");

            Ok(serde_json::to_string(&serde_json::json!({
                "token": token,
                "success": true
            }))
            .unwrap())
        }
        Err(e) => {
            tracing::error!("Falha no login Hotmart: {}", e);
            Err(format!("Falha no login: {}", e))
        }
    }
}
