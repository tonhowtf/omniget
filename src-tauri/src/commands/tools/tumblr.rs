//! Comandos da categoria Blogs para Tumblr e favoritos de arte: índice dos
//! likes, espelho offline de um blog e índice unificado de DeviantArt,
//! ArtStation e Flickr. A leitura é toda pós-processamento do gallery-dl.

use omniget_core::core::tools::tumblr::{art, backup, gdl, likes};

use super::{err, progress};

const TUMBLR: &str = "tumblr.com";

/// Cookies da conta escolhida no gerenciador (None = `_default`). Likes e
/// favoritos são páginas de conta: sem sessão o gallery-dl só enxerga o que
/// é público.
fn session_of(domain: &str, slug: Option<&str>) -> Option<String> {
    let slug = slug.unwrap_or("_default");
    let content = crate::cookies::storage::read_account_file(domain, slug).ok()?;
    if content.trim().is_empty() {
        return None;
    }
    crate::cookies::touch_last_used(domain, slug);
    Some(content)
}

/// As três plataformas de arte num arquivo Netscape só — é o que o
/// `--cookies` do gallery-dl aceita.
fn art_session(slug: Option<&str>) -> Option<String> {
    let parts: Vec<String> = art::domains()
        .into_iter()
        .filter_map(|d| session_of(d, slug))
        .collect();
    if parts.is_empty() {
        return None;
    }
    Some(gdl::merge_netscape(&parts))
}

#[tauri::command]
pub async fn tool_tumblr_likes(
    app: tauri::AppHandle,
    opts: likes::Options,
) -> Result<likes::LikesResult, String> {
    let mut opts = opts;
    opts.session_netscape = session_of(TUMBLR, opts.account_slug.as_deref());
    likes::run(&opts, &progress(&app)).await.map_err(err)
}

#[tauri::command]
pub async fn tool_tumblr_backup(
    app: tauri::AppHandle,
    opts: backup::Options,
) -> Result<backup::BackupResult, String> {
    let mut opts = opts;
    opts.session_netscape = session_of(TUMBLR, opts.account_slug.as_deref());
    backup::run(&opts, &progress(&app)).await.map_err(err)
}

#[tauri::command]
pub async fn tool_art_favorites(
    app: tauri::AppHandle,
    opts: art::Options,
) -> Result<art::ArtResult, String> {
    let mut opts = opts;
    opts.session_netscape = art_session(opts.account_slug.as_deref());
    art::run(&opts, &progress(&app)).await.map_err(err)
}
