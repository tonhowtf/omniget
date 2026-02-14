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
            let response_email = session.email.clone();
            let mut guard = state.hotmart_session.lock().await;
            *guard = Some(session);
            tracing::info!("Sessão Hotmart salva no state global");
            Ok(response_email)
        }
        Err(e) => {
            tracing::error!("Falha no login Hotmart: {}", e);
            Err(format!("Falha no login: {}", e))
        }
    }
}

#[tauri::command]
pub async fn hotmart_check_session(
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    let guard = state.hotmart_session.lock().await;
    let session = guard
        .as_ref()
        .ok_or_else(|| "not_authenticated".to_string())?;

    let token = session.token.clone();
    let email = session.email.clone();
    let client = session.client.clone();
    drop(guard);

    let resp = client
        .get(format!(
            "https://api-sec-vlc.hotmart.com/security/oauth/check_token?token={}",
            token
        ))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| format!("Erro na validação: {}", e))?;

    if resp.status().is_success() {
        tracing::info!("Sessão Hotmart válida para {}", email);
        Ok(email)
    } else {
        tracing::info!("Sessão Hotmart expirada, limpando state");
        state.hotmart_session.lock().await.take();
        Err("session_expired".to_string())
    }
}

#[tauri::command]
pub async fn hotmart_logout(
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    state.hotmart_session.lock().await.take();
    tracing::info!("Sessão Hotmart removida");
    Ok(())
}
