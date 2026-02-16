use std::time::{Duration, Instant};

use crate::platforms::hotmart::auth::{authenticate, delete_saved_session, load_saved_session, save_session};
use crate::AppState;

const SESSION_COOLDOWN: Duration = Duration::from_secs(5 * 60);

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
            if let Err(e) = save_session(&session).await {
                tracing::warn!("Falha ao salvar sessão no disco: {}", e);
            }
            let mut guard = state.hotmart_session.lock().await;
            *guard = Some(session);
            *state.session_validated_at.lock().await = Some(Instant::now());
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
    let has_memory_session = state.hotmart_session.lock().await.is_some();

    if !has_memory_session {
        tracing::info!("Nenhuma sessão em memória, tentando restaurar do disco...");
        match load_saved_session().await {
            Ok(session) => {
                tracing::info!("Sessão restaurada do disco para {}", session.email);
                let mut guard = state.hotmart_session.lock().await;
                *guard = Some(session);
            }
            Err(e) => {
                tracing::info!("Sem sessão salva no disco: {}", e);
                return Err("not_authenticated".to_string());
            }
        }
    }

    let guard = state.hotmart_session.lock().await;
    let session = guard
        .as_ref()
        .ok_or_else(|| "not_authenticated".to_string())?;
    let email = session.email.clone();

    {
        let validated_at = state.session_validated_at.lock().await;
        if let Some(at) = *validated_at {
            if at.elapsed() < SESSION_COOLDOWN {
                tracing::info!("Sessão validada há {:?}, usando cache", at.elapsed());
                return Ok(email);
            }
        }
    }

    let token = session.token.clone();
    let client = session.client.clone();
    drop(guard);

    let resp = client
        .get(format!(
            "https://api-sec-vlc.hotmart.com/security/oauth/check_token?token={}",
            token
        ))
        .send()
        .await
        .map_err(|e| format!("Erro na validação: {}", e))?;

    if resp.status().is_success() {
        *state.session_validated_at.lock().await = Some(Instant::now());
        tracing::info!("Sessão Hotmart válida para {}", email);
        Ok(email)
    } else {
        tracing::info!("Sessão Hotmart expirada, limpando state e disco");
        state.hotmart_session.lock().await.take();
        *state.session_validated_at.lock().await = None;
        *state.courses_cache.lock().await = None;
        let _ = delete_saved_session().await;
        Err("session_expired".to_string())
    }
}

#[tauri::command]
pub async fn hotmart_logout(
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    state.hotmart_session.lock().await.take();
    *state.session_validated_at.lock().await = None;
    *state.courses_cache.lock().await = None;
    if let Err(e) = delete_saved_session().await {
        tracing::warn!("Falha ao deletar sessão do disco: {}", e);
    }
    tracing::info!("Sessão Hotmart removida (memória e disco)");
    Ok(())
}

