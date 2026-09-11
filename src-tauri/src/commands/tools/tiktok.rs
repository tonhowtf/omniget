//! Comandos da categoria TikTok: vídeo sem marca d'água em lote, o som com
//! atribuição e o índice dos favoritos/da mídia do perfil.
//!
//! O vídeo e o som são conteúdo público — a sessão só entra como reforço,
//! para conta privada e para o site não tratar a gente como robô. Os
//! favoritos, esses, não existem sem a sessão do usuário.

use omniget_core::core::tools::tiktok::{download, favorites, sound};

use super::{err, progress};

const DOMAIN: &str = "tiktok.com";

/// Cookies da conta escolhida no gerenciador (None = `_default`).
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
pub async fn tool_tt_download(
    app: tauri::AppHandle,
    opts: download::Options,
) -> Result<download::DownloadResult, String> {
    let mut opts = opts;
    opts.session_netscape = session_of(opts.account_slug.as_deref());
    download::run(&opts, progress(&app)).await.map_err(err)
}

#[tauri::command]
pub async fn tool_tt_sound(
    app: tauri::AppHandle,
    opts: sound::Options,
) -> Result<sound::SoundResult, String> {
    let mut opts = opts;
    opts.session_netscape = session_of(opts.account_slug.as_deref());
    sound::run(&opts, progress(&app)).await.map_err(err)
}

#[tauri::command]
pub async fn tool_tt_favorites(
    app: tauri::AppHandle,
    opts: favorites::Options,
) -> Result<favorites::FavoritesResult, String> {
    let mut opts = opts;
    opts.session_netscape = session_of(opts.account_slug.as_deref());
    favorites::run(&opts, progress(&app)).await.map_err(err)
}
