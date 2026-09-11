//! Comandos da categoria Blogs: arquivo pessoal das newsletters do Substack e
//! exportação das histórias do próprio usuário no Medium.
//!
//! As duas tools leem a sessão do gerenciador de cookies antes de chamar o
//! core — é ela que faz o Substack listar as assinaturas da conta e o Medium
//! entregar os rascunhos. Sem cookie, as duas ainda funcionam no que é
//! público (arquivo de uma publicação, feed do perfil).

use omniget_core::core::tools::blogs::{medium, substack};

use super::{err, progress};

const SUBSTACK_DOMAIN: &str = "substack.com";
const MEDIUM_DOMAIN: &str = "medium.com";

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

#[tauri::command]
pub async fn tool_blog_substack(
    app: tauri::AppHandle,
    opts: substack::Options,
) -> Result<substack::ArchiveResult, String> {
    let mut opts = opts;
    opts.session_netscape = session_of(SUBSTACK_DOMAIN, opts.account_slug.as_deref());
    substack::run(&opts, progress(&app)).await.map_err(err)
}

#[tauri::command]
pub async fn tool_blog_medium(
    app: tauri::AppHandle,
    opts: medium::Options,
) -> Result<medium::ExportResult, String> {
    let mut opts = opts;
    opts.session_netscape = session_of(MEDIUM_DOMAIN, opts.account_slug.as_deref());
    medium::run(&opts, progress(&app)).await.map_err(err)
}
