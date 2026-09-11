use omniget_core::core::tools::{ryd, sponsorblock};

use super::err;

#[tauri::command]
pub async fn tool_sponsorblock(
    url: String,
    categories: Option<Vec<String>>,
) -> Result<sponsorblock::SponsorResult, String> {
    sponsorblock::segments(&url, &categories.unwrap_or_default())
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn tool_ryd(url: String) -> Result<ryd::Votes, String> {
    ryd::votes(&url).await.map_err(err)
}

#[tauri::command]
pub fn tool_yt_video_id(url: String) -> Option<String> {
    sponsorblock::video_id(&url)
}

/// Baixa uma URL simples (thumbnail, frame) para um arquivo.
#[tauri::command]
pub async fn tool_save_url(url: String, dest: String) -> Result<String, String> {
    let client = omniget_core::core::tools::client().map_err(err)?;
    let resp = client.get(&url).send().await.map_err(err)?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let bytes = resp.bytes().await.map_err(err)?;
    if let Some(parent) = std::path::Path::new(&dest).parent() {
        std::fs::create_dir_all(parent).map_err(err)?;
    }
    tokio::fs::write(&dest, &bytes).await.map_err(err)?;
    Ok(dest)
}

// ── Arquivo, notas e capítulos (rodada 3) ──────────────────────────────

use omniget_core::core::tools::{yt_archive, yt_chapters, yt_notes};

use super::progress;

const YT_DOMAIN: &str = "youtube.com";

/// Cookies da conta escolhida no gerenciador (None = `_default`). Watch Later
/// e playlist privada não existem sem sessão; vídeo público não precisa.
fn yt_session(slug: Option<&str>) -> Option<String> {
    let slug = slug.unwrap_or("_default");
    let content = crate::cookies::storage::read_account_file(YT_DOMAIN, slug).ok()?;
    if content.trim().is_empty() {
        return None;
    }
    crate::cookies::touch_last_used(YT_DOMAIN, slug);
    Some(content)
}

/// Só enumera a coleção e funde com o estado gravado — não baixa nada.
#[tauri::command]
pub async fn tool_yt_archive_scan(
    app: tauri::AppHandle,
    opts: yt_archive::Options,
) -> Result<yt_archive::ArchiveResult, String> {
    let mut opts = opts;
    opts.session_netscape = yt_session(opts.account_slug.as_deref());
    yt_archive::scan(&opts, &progress(&app)).await.map_err(err)
}

/// Continua o arquivo de onde parou.
#[tauri::command]
pub async fn tool_yt_archive_run(
    app: tauri::AppHandle,
    opts: yt_archive::Options,
) -> Result<yt_archive::ArchiveResult, String> {
    let mut opts = opts;
    opts.session_netscape = yt_session(opts.account_slug.as_deref());
    yt_archive::run(opts, progress(&app)).await.map_err(err)
}

/// Estado gravado na pasta, sem rede — é o que a tela mostra ao abrir.
#[tauri::command]
pub fn tool_yt_archive_state(dest: String) -> Result<yt_archive::ArchiveResult, String> {
    yt_archive::state(&dest).map_err(err)
}

#[tauri::command]
pub fn tool_yt_archive_cancel() {
    yt_archive::cancel();
}

#[tauri::command]
pub async fn tool_yt_notes(
    app: tauri::AppHandle,
    opts: yt_notes::Options,
) -> Result<yt_notes::NotesResult, String> {
    let mut opts = opts;
    opts.session_netscape = yt_session(opts.account_slug.as_deref());
    yt_notes::run(opts, progress(&app)).await.map_err(err)
}

#[tauri::command]
pub async fn tool_yt_chapters(
    app: tauri::AppHandle,
    opts: yt_chapters::Options,
) -> Result<yt_chapters::ChaptersResult, String> {
    yt_chapters::run(opts, progress(&app)).await.map_err(err)
}

/// Identificador estável da chave local do SponsorBlock. A chave em si nunca
/// sai do backend.
#[derive(serde::Serialize)]
pub struct SponsorUser {
    pub fingerprint: String,
}

#[tauri::command]
pub fn tool_sb_user() -> Result<SponsorUser, String> {
    let key = sponsorblock::local_user_id().map_err(err)?;
    Ok(SponsorUser {
        fingerprint: sponsorblock::public_fingerprint(&key)[..12].to_string(),
    })
}

/// Envia trechos ao SponsorBlock. `opts.confirmed` só pode vir da confirmação
/// explícita do usuário: o envio é público e vale para todo mundo.
#[tauri::command]
pub async fn tool_sb_submit(
    opts: sponsorblock::SubmitOptions,
) -> Result<sponsorblock::SubmitResult, String> {
    sponsorblock::submit(opts).await.map_err(err)
}
