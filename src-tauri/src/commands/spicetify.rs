//! Comandos da ferramenta "Spotify → Spicetify" da seção Tools.

use omniget_core::core::spicetify::{self, CmdOutput, SpicetifyStatus};

fn err(e: anyhow::Error) -> String {
    e.to_string()
}

async fn bin() -> Result<std::path::PathBuf, String> {
    spicetify::locate()
        .await
        .map(|(p, _)| p)
        .ok_or_else(|| "spicetify nao esta instalado".to_string())
}

#[tauri::command]
pub async fn spicetify_status() -> Result<SpicetifyStatus, String> {
    Ok(spicetify::status().await)
}

#[tauri::command]
pub async fn spicetify_install() -> Result<String, String> {
    spicetify::install()
        .await
        .map(|p| p.to_string_lossy().to_string())
        .map_err(err)
}

/// Ações de um clique. A lista é fechada de propósito: o front não manda
/// argumentos livres para um binário que patcha outro app.
#[tauri::command]
pub async fn spicetify_action(action: String) -> Result<CmdOutput, String> {
    let args: &[&str] = match action.as_str() {
        "backup_apply" => &["backup", "apply"],
        "apply" => &["apply"],
        "refresh" => &["refresh"],
        "restore" => &["restore"],
        "restart" => &["restart"],
        "upgrade" => &["upgrade"],
        "enable_devtools" => &["enable-devtools"],
        "open_config_dir" => &["config-dir"],
        "block_updates" => &["spotify-updates", "block"],
        "unblock_updates" => &["spotify-updates", "unblock"],
        other => return Err(format!("acao desconhecida: {}", other)),
    };
    let bin = bin().await?;
    spicetify::run_ok(&bin, args).await.map_err(err)
}

#[tauri::command]
pub async fn spicetify_set_theme(theme: String, scheme: String) -> Result<CmdOutput, String> {
    let bin = bin().await?;
    let theme = theme.trim();
    let scheme = scheme.trim();
    if theme.contains(['/', '\\', '|']) || scheme.contains(['/', '\\', '|']) {
        return Err("nome de tema invalido".into());
    }
    spicetify::run_ok(
        &bin,
        &["config", "current_theme", theme, "color_scheme", scheme],
    )
    .await
    .map_err(err)?;
    let status = spicetify::status().await;
    let args: &[&str] = if status.applied {
        &["apply"]
    } else {
        &["backup", "apply"]
    };
    spicetify::run_ok(&bin, args).await.map_err(err)
}

/// Remove uma extensão ou custom app da config (`nome-` no CLI) e reaplica.
#[tauri::command]
pub async fn spicetify_remove_addon(kind: String, name: String) -> Result<CmdOutput, String> {
    let bin = bin().await?;
    let field = match kind.as_str() {
        "extension" => "extensions",
        "custom_app" => "custom_apps",
        _ => return Err("tipo invalido".into()),
    };
    let name = name.trim();
    if name.is_empty() || name.contains(['/', '\\', '|']) {
        return Err("nome invalido".into());
    }
    let removal = format!("{}-", name);
    spicetify::run_ok(&bin, &["config", field, &removal])
        .await
        .map_err(err)?;
    spicetify::run_ok(&bin, &["apply"]).await.map_err(err)
}

#[tauri::command]
pub async fn spicetify_install_marketplace() -> Result<CmdOutput, String> {
    let bin = bin().await?;
    let status = spicetify::status().await;
    let config_dir = status
        .config_dir
        .ok_or_else(|| "pasta de config do spicetify nao encontrada".to_string())?;
    spicetify::install_marketplace(&bin, std::path::Path::new(&config_dir))
        .await
        .map_err(err)
}
