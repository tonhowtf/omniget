//! Comandos da categoria Listas: exportar diário e estante do Letterboxd, do
//! Trakt e do Goodreads, e juntar tudo numa linha do tempo só.
//!
//! Letterboxd e Goodreads não têm API pública — o caminho é a sessão que a
//! extensão capturou, cada um no seu bucket de cookie. O Trakt tem API
//! oficial e usa o device flow, com credenciais que o próprio usuário cria.

use omniget_core::core::tools::lists::{goodreads, letterboxd, merge, trakt};

use super::{err, progress};

/// Cookies da conta escolhida no gerenciador (None = `_default`).
fn session_of(domain: &str, slug: Option<&str>) -> Option<String> {
    let slug = slug.unwrap_or("_default");
    let content = crate::cookies::storage::read_account_file(domain, slug).ok()?;
    if content.trim().is_empty() {
        return None;
    }
    crate::cookies::touch_last_used(domain, slug);
    Some(content)
}

/// Se existe sessão salva para o domínio, para a UI dizer se está logado
/// antes de o usuário apertar qualquer botão.
#[tauri::command]
pub fn tool_lists_session(domain: String, account_slug: Option<String>) -> bool {
    let domain = match domain.as_str() {
        "goodreads" | "goodreads.com" => goodreads::DOMAIN,
        _ => letterboxd::DOMAIN,
    };
    session_of(domain, account_slug.as_deref()).is_some()
}

// ── Letterboxd ──────────────────────────────────────────────────────────

#[tauri::command]
pub async fn tool_lb_export(
    app: tauri::AppHandle,
    opts: letterboxd::Options,
) -> Result<letterboxd::ExportResult, String> {
    let mut opts = opts;
    opts.session_netscape = session_of(letterboxd::DOMAIN, opts.account_slug.as_deref());
    letterboxd::run(&opts, progress(&app)).await.map_err(err)
}

// ── Trakt ───────────────────────────────────────────────────────────────

#[tauri::command]
pub fn tool_trakt_creds() -> trakt::CredsView {
    trakt::creds_view()
}

#[tauri::command]
pub fn tool_trakt_save_app(
    client_id: String,
    client_secret: String,
) -> Result<trakt::CredsView, String> {
    trakt::save_app(&client_id, &client_secret).map_err(err)
}

#[tauri::command]
pub fn tool_trakt_disconnect(forget: bool) -> Result<trakt::CredsView, String> {
    if forget {
        trakt::forget().map_err(err)
    } else {
        trakt::disconnect().map_err(err)
    }
}

/// Passo 1 do device flow: o código que o usuário digita no site.
#[tauri::command]
pub async fn tool_trakt_device_code() -> Result<trakt::DeviceCode, String> {
    trakt::device_code().await.map_err(err)
}

/// Passo 2: espera o usuário liberar o acesso e guarda o token.
#[tauri::command]
pub async fn tool_trakt_connect(
    app: tauri::AppHandle,
    code: trakt::DeviceCode,
) -> Result<trakt::ConnectResult, String> {
    trakt::wait_for_token(&code, progress(&app))
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn tool_trakt_export(
    app: tauri::AppHandle,
    opts: trakt::Options,
) -> Result<trakt::ExportResult, String> {
    trakt::run(&opts, progress(&app)).await.map_err(err)
}

// ── Goodreads ───────────────────────────────────────────────────────────

#[tauri::command]
pub async fn tool_gr_export(
    app: tauri::AppHandle,
    opts: goodreads::Options,
) -> Result<goodreads::ExportResult, String> {
    let mut opts = opts;
    opts.session_netscape = session_of(goodreads::DOMAIN, opts.account_slug.as_deref());
    goodreads::run(&opts, progress(&app)).await.map_err(err)
}

// ── Diário único ────────────────────────────────────────────────────────

#[tauri::command]
pub async fn tool_media_diary(
    app: tauri::AppHandle,
    opts: merge::Options,
) -> Result<merge::MergeResult, String> {
    let p = progress(&app);
    tokio::task::spawn_blocking(move || merge::run(&opts, &p))
        .await
        .map_err(err)?
        .map_err(err)
}
