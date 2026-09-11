//! Comandos da categoria Vimeo: vídeo privado/unlisted (link com hash ou
//! senha que o usuário já tem) e backup em lote de showcase, álbum e canal.
//!
//! A sessão do gerenciador de cookies entra aqui e só aqui — o front nunca
//! manda cookie, e a senha nunca volta em nada que o front receba.

use omniget_core::core::tools::vimeo;

use super::{err, progress};

const DOMAIN: &str = "vimeo.com";

/// Cookies da conta escolhida no gerenciador (None = `_default`). Sem sessão
/// o Vimeo só entrega o que é público; com ela, o showcase e o vídeo que só a
/// conta do usuário vê passam a abrir.
fn session_of(slug: Option<&str>) -> Option<String> {
    let slug = slug.unwrap_or("_default");
    let content = crate::cookies::storage::read_account_file(DOMAIN, slug).ok()?;
    if content.trim().is_empty() {
        return None;
    }
    crate::cookies::touch_last_used(DOMAIN, slug);
    Some(content)
}

#[tauri::command]
pub async fn tool_vm_list(
    app: tauri::AppHandle,
    opts: vimeo::ListOptions,
) -> Result<vimeo::Listing, String> {
    let mut opts = opts;
    opts.session_netscape = session_of(opts.account_slug.as_deref());
    vimeo::list(&opts, &progress(&app)).await.map_err(err)
}

#[tauri::command]
pub async fn tool_vm_showcase(
    app: tauri::AppHandle,
    opts: vimeo::BackupOptions,
) -> Result<vimeo::BackupResult, String> {
    let mut opts = opts;
    opts.session_netscape = session_of(opts.account_slug.as_deref());
    vimeo::backup(&opts, &progress(&app)).await.map_err(err)
}

#[tauri::command]
pub async fn tool_vm_private(
    app: tauri::AppHandle,
    opts: vimeo::PrivateOptions,
) -> Result<vimeo::PrivateResult, String> {
    let mut opts = opts;
    opts.session_netscape = session_of(opts.account_slug.as_deref());
    vimeo::private(&opts, &progress(&app)).await.map_err(err)
}
