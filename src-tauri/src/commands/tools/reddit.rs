//! Comandos da categoria Reddit. Tudo conteúdo público: baixar a mídia de um
//! post (vídeo do v.redd.it com áudio, galeria, imagem, link de fora),
//! arquivar uma thread inteira com os comentários e ler o export oficial de
//! dados do usuário (esse último sem tocar na rede).

use omniget_core::core::tools::reddit::{download, gdpr, thread};

use super::{err, progress};

const DOMAIN: &str = "reddit.com";

/// Cookies da conta escolhida no gerenciador (None = `_default`). O Reddit
/// passou a responder 403 em todo `.json` anônimo; com a sessão da extensão
/// ele responde como responde ao navegador do usuário, e o desafio de
/// JavaScript da porta de entrada vira só a rede de segurança de quem não
/// capturou cookie nenhum.
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
pub async fn tool_rd_download(
    app: tauri::AppHandle,
    opts: download::Options,
) -> Result<download::DownloadResult, String> {
    let mut opts = opts;
    opts.session_netscape = session_of(opts.account_slug.as_deref());
    download::run(&opts, progress(&app)).await.map_err(err)
}

#[tauri::command]
pub async fn tool_rd_thread(
    app: tauri::AppHandle,
    opts: thread::Options,
) -> Result<thread::ThreadResult, String> {
    let mut opts = opts;
    opts.session_netscape = session_of(opts.account_slug.as_deref());
    thread::run(&opts, progress(&app)).await.map_err(err)
}

#[tauri::command]
pub async fn tool_rd_gdpr(
    app: tauri::AppHandle,
    opts: gdpr::Options,
) -> Result<gdpr::GdprResult, String> {
    let p = progress(&app);
    tokio::task::spawn_blocking(move || gdpr::run(&opts, &p))
        .await
        .map_err(err)?
        .map_err(err)
}
