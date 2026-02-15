use crate::platforms::hotmart::api;
use crate::platforms::hotmart::auth::{authenticate, delete_saved_session, load_saved_session, save_session};
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
            if let Err(e) = save_session(&session).await {
                tracing::warn!("Falha ao salvar sessão no disco: {}", e);
            }
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

    let token = session.token.clone();
    let email = session.email.clone();
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
        tracing::info!("Sessão Hotmart válida para {}", email);
        Ok(email)
    } else {
        tracing::info!("Sessão Hotmart expirada, limpando state e disco");
        state.hotmart_session.lock().await.take();
        let _ = delete_saved_session().await;
        Err("session_expired".to_string())
    }
}

#[tauri::command]
pub async fn hotmart_logout(
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    state.hotmart_session.lock().await.take();
    if let Err(e) = delete_saved_session().await {
        tracing::warn!("Falha ao deletar sessão do disco: {}", e);
    }
    tracing::info!("Sessão Hotmart removida (memória e disco)");
    Ok(())
}

#[tauri::command]
pub async fn hotmart_debug_auth(
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    tracing::info!("=== DEBUG AUTH INICIADO ===");

    let guard = state.hotmart_session.lock().await;
    let has_session = guard.is_some();
    drop(guard);

    if !has_session {
        tracing::info!("[DEBUG] Nenhuma sessão ativa, fazendo login...");
        match authenticate("ADRIANODESAPEREIRA@GMAIL.COM", "Cecilinda2019").await {
            Ok(session) => {
                tracing::info!("[DEBUG] Login bem-sucedido!");
                let mut guard = state.hotmart_session.lock().await;
                *guard = Some(session);
            }
            Err(e) => {
                let msg = format!("[DEBUG] FALHA no login: {}", e);
                tracing::error!("{}", msg);
                return Err(msg);
            }
        }
    }

    let guard = state.hotmart_session.lock().await;
    let session = guard.as_ref().unwrap();

    
    tracing::info!("[DEBUG] Token extraído: {}...", &session.token[..20.min(session.token.len())]);

    let cookie_names: Vec<&str> = session.cookies.iter().map(|(name, _)| name.as_str()).collect();
    tracing::info!("[DEBUG] Cookies na sessão: {:?}", cookie_names);

    let has_vlc = session.cookies.iter().any(|(name, _)| name == "hmVlcIntegration");
    tracing::info!("[DEBUG] hmVlcIntegration presente: {}", has_vlc);

    tracing::info!("[DEBUG] Chamando check_token...");
    match api::get_subdomains(session).await {
        Ok(subdomains) => {
            tracing::info!("[DEBUG] Subdomínios encontrados: {}", subdomains.len());
            for s in &subdomains {
                tracing::info!("[DEBUG]   productId: {} → subdomain: {}", s.product_id, s.subdomain);
            }
        }
        Err(e) => {
            tracing::error!("[DEBUG] FALHA check_token: {}", e);
            return Err(format!("check_token falhou: {}", e));
        }
    }

    tracing::info!("[DEBUG] Chamando list_courses...");
    let mut courses = match api::list_courses(session).await {
        Ok(c) => {
            tracing::info!("[DEBUG] Cursos encontrados: {}", c.len());
            c
        }
        Err(e) => {
            tracing::error!("[DEBUG] FALHA list_courses: {}", e);
            return Err(format!("list_courses falhou: {}", e));
        }
    };

    if let Ok(subdomains) = api::get_subdomains(session).await {
        api::merge_subdomains(&mut courses, &subdomains);
    }

    let mut output_lines = Vec::new();
    for course in &mut courses {
        let price = match api::get_course_price(session, course.id).await {
            Ok(p) => {
                course.price = Some(p);
                format!("R${:.2}", p)
            }
            Err(_) => "N/A".to_string(),
        };
        let line = format!(
            "- {} (ID: {}, slug: {:?}, preço: {})",
            course.name, course.id, course.slug, price
        );
        tracing::info!("[DEBUG] {}", line);
        output_lines.push(line);
    }

    tracing::info!("=== DEBUG AUTH CONCLUÍDO: {} cursos ===", courses.len());

    Ok(format!(
        "Debug concluído: {} cursos encontrados\n{}",
        courses.len(),
        output_lines.join("\n")
    ))
}
