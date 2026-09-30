//! Optional providers of the 3D city: text/image to 3D through Tripo (the
//! user's own key) and local auto-rig through skin-tokens.cpp. Both land in
//! the same asset store as any import, through the same normalizer.

use super::asset3d::{options, store};
use base64::Engine;
use omniget_asset3d::providers::skintokens::{self, SkinTokens, SkinTokensConfig};
use omniget_asset3d::providers::tripo::Tripo;
use omniget_asset3d::providers::{GenerateRequest, Rigger};
use omniget_asset3d::store::{AssetMeta, AssetSource};
use omniget_asset3d::{AssetClass, Input, License, LicenseKind};
use serde::Serialize;
use std::path::PathBuf;
use tauri::ipc::Channel;

const SECRET_NS: &str = "city3d";
const TRIPO_ACCOUNT: &str = "tripo_api_key";

fn tripo_key() -> Result<Option<String>, String> {
    omniget_core::core::secrets::get(SECRET_NS, TRIPO_ACCOUNT)
}

fn rig_root() -> Result<PathBuf, String> {
    Ok(crate::core::paths::app_data_dir()
        .ok_or("no app data folder")?
        .join("city3d")
        .join("skintokens"))
}

/// The configured CLI path wins over the default location under app data.
fn rig_config() -> Result<SkinTokensConfig, String> {
    let root = rig_root()?;
    let mut cfg = SkinTokensConfig::under(&root);
    if let Ok(p) = std::fs::read_to_string(root.join("cli-path.txt")) {
        let p = PathBuf::from(p.trim());
        if p.is_file() {
            cfg.cli = p;
        }
    }
    if let Ok(p) = std::fs::read_to_string(root.join("model-dir.txt")) {
        let p = PathBuf::from(p.trim());
        if p.is_dir() {
            cfg.model_dir = p;
        }
    }
    Ok(cfg)
}

#[derive(Serialize)]
pub struct ProvidersStatus {
    tripo_key: bool,
    rig_cli: bool,
    rig_weights: bool,
    rig_cli_path: String,
    rig_model_dir: String,
    /// A prebuilt package exists for this platform (`city3d_autorig_install_cli`).
    rig_cli_package: bool,
}

#[tauri::command]
pub async fn city3d_providers_status() -> Result<ProvidersStatus, String> {
    let cfg = rig_config()?;
    let weights = std::fs::read_dir(&cfg.model_dir)
        .map(|d| {
            d.filter_map(|e| e.ok())
                .filter(|e| e.path().extension().is_some_and(|x| x == "gguf"))
                .count()
                >= 3
        })
        .unwrap_or(false);
    Ok(ProvidersStatus {
        tripo_key: tripo_key()?.is_some_and(|k| !k.trim().is_empty()),
        rig_cli: cfg.cli.is_file(),
        rig_weights: weights,
        rig_cli_path: cfg.cli.display().to_string(),
        rig_model_dir: cfg.model_dir.display().to_string(),
        rig_cli_package: skintokens::cli_package().is_ok(),
    })
}

/// Stores (or clears, with an empty string) the user's Tripo API key in the
/// app's secret store.
#[tauri::command]
pub async fn city3d_tripo_key_set(key: String) -> Result<(), String> {
    omniget_core::core::secrets::put_or_delete(SECRET_NS, TRIPO_ACCOUNT, key.trim())
}

#[tauri::command]
pub async fn city3d_tripo_balance() -> Result<f64, String> {
    let key = tripo_key()?.ok_or("no Tripo key")?;
    Tripo::new(&key).balance().await
}

/// Text or image to 3D through Tripo, then the normal import.
#[tauri::command]
pub async fn city3d_generate3d(
    prompt: Option<String>,
    image_base64: Option<String>,
    class: AssetClass,
    fit_style: bool,
) -> Result<AssetMeta, String> {
    let key = tripo_key()?.ok_or("no Tripo key: add yours in the import panel")?;
    let image = match image_base64 {
        Some(b) => Some(
            base64::engine::general_purpose::STANDARD
                .decode(b.trim())
                .map_err(|_| "bad image base64".to_string())?,
        ),
        None => None,
    };
    if prompt.as_deref().map(str::trim).unwrap_or("").is_empty() && image.is_none() {
        return Err("describe the object or give an image".into());
    }
    let generated = Tripo::new(&key)
        .run(GenerateRequest {
            prompt: prompt.clone(),
            image,
            face_limit: None,
        })
        .await?;
    let name = prompt
        .as_deref()
        .map(|p| p.chars().take(40).collect::<String>())
        .filter(|p| !p.trim().is_empty())
        .unwrap_or_else(|| "generated".into());
    let license = License {
        kind: LicenseKind::Own,
        author: Some(format!("{} ({})", generated.provider, generated.model)),
        url: None,
    };
    let opts = options(
        &name,
        class,
        &license,
        fit_style,
        None,
        None,
        format!("{}:{}", generated.provider, generated.task_id),
    );
    let input = Input::bytes(generated.glb, "generated.glb");
    let asset = tokio::task::spawn_blocking(move || omniget_asset3d::normalize(input, opts))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let source = AssetSource {
        kind: "generated".into(),
        name,
        format: asset.report.format.clone(),
        source_hash: asset.source_hash.clone(),
        provider: Some(generated.provider),
    };
    store()?.save(&asset, license, source, fit_style)
}

/// Points the auto-rig at an existing `skintokens-cli` and/or weights folder
/// (the alternative to `city3d_autorig_install_cli`).
#[tauri::command]
pub async fn city3d_autorig_configure(
    cli_path: Option<String>,
    model_dir: Option<String>,
) -> Result<(), String> {
    let root = rig_root()?;
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    if let Some(p) = cli_path {
        if !p.trim().is_empty() && !PathBuf::from(p.trim()).is_file() {
            return Err("skintokens-cli not found at that path".into());
        }
        std::fs::write(root.join("cli-path.txt"), p.trim()).map_err(|e| e.to_string())?;
    }
    if let Some(d) = model_dir {
        std::fs::write(root.join("model-dir.txt"), d.trim()).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[derive(Clone, Serialize)]
pub struct DownloadProgress {
    file: String,
    done: u64,
    total: Option<u64>,
}

/// Downloads the missing skin-tokens weights (about 1.25 GB, checked by
/// sha256). Asked for explicitly by the person, never in the background.
#[tauri::command]
pub async fn city3d_autorig_install_weights(
    on_progress: Channel<DownloadProgress>,
) -> Result<(), String> {
    let cfg = rig_config()?;
    std::fs::create_dir_all(&cfg.model_dir).map_err(|e| e.to_string())?;
    skintokens::download_weights(&cfg, &move |file, done, total| {
        let _ = on_progress.send(DownloadProgress {
            file: file.to_string(),
            done,
            total,
        });
    })
    .await
}

/// Downloads the prebuilt `skintokens-cli` package of this platform (our
/// own release, see `.github/workflows/skintokens.yml`), checks its sha256
/// and extracts it, with the ggml libraries, into `skintokens/bin/`. A
/// manually configured path is dropped so the installed binary is used.
#[tauri::command]
pub async fn city3d_autorig_install_cli(
    on_progress: Channel<DownloadProgress>,
) -> Result<(), String> {
    let (url, sha) = skintokens::cli_package()?;
    let root = rig_root()?;
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let cfg = SkinTokensConfig::under(&root);
    let file = url
        .rsplit('/')
        .next()
        .unwrap_or("skintokens-cli")
        .to_string();
    skintokens::download_cli(&cfg, &url, Some(sha), &move |done, total| {
        let _ = on_progress.send(DownloadProgress {
            file: file.clone(),
            done,
            total,
        });
    })
    .await?;
    let _ = std::fs::remove_file(root.join("cli-path.txt"));
    Ok(())
}

/// Rigs a stored character (no skin yet) and saves the result as a new
/// asset of class `character`.
#[tauri::command]
pub async fn city3d_autorig(id: String) -> Result<AssetMeta, String> {
    let st = store()?;
    let meta = st.meta(&id)?;
    let glb = st.glb(&id)?;
    let plain = tokio::task::spawn_blocking(move || omniget_asset3d::plain_glb_for_rigging(&glb))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let rigger = SkinTokens { cfg: rig_config()? };
    let rigged = rigger.rig(plain).await?;
    let license = meta.license.clone();
    let name = format!("{} (rigged)", meta.name);
    let opts = options(
        &name,
        AssetClass::Character,
        &license,
        false,
        None,
        None,
        format!("autorig:{id}"),
    );
    let input = Input::bytes(rigged, "rigged.glb");
    let asset = tokio::task::spawn_blocking(move || omniget_asset3d::normalize(input, opts))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let source = AssetSource {
        kind: "autorig".into(),
        name,
        format: asset.report.format.clone(),
        source_hash: asset.source_hash.clone(),
        provider: Some(rigger.id().into()),
    };
    st.save(&asset, license, source, false)
}
