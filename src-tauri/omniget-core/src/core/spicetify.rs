//! Spicetify: personalização do cliente oficial do Spotify.
//!
//! O OmniGet não reimplementa o Spicetify; ele baixa o binário oficial do
//! release do GitHub (hash conferido) e o chama. A pasta de config é a
//! padrão do Spicetify (`~/.config/spicetify`, `%APPDATA%\spicetify`), de
//! propósito: quem já usa o CLI no terminal vê o mesmo estado aqui, e o
//! `spicetify upgrade` continua atualizando o próprio binário.
//!
//! O tarball do release traz mais que o executável (`jsHelper/`, `Themes/`,
//! `Extensions/`, `CustomApps/`, `css-map.json`), e o CLI lê tudo isso da
//! pasta onde está. Por isso a instalação gerida vive numa pasta própria,
//! `<app_data>/bin/spicetify-cli/`, e não solta em `bin/`.

use std::path::{Path, PathBuf};

use anyhow::anyhow;
use serde::Serialize;

use crate::core::dependencies::{self, bin_name, integrity};

const CLI_REPO: &str = "spicetify/cli";
const MARKETPLACE_REPO: &str = "spicetify/marketplace";
const MARKETPLACE_COLOR_INI: &str =
    "https://raw.githubusercontent.com/spicetify/marketplace/main/resources/color.ini";

fn managed_dir() -> Option<PathBuf> {
    dependencies::managed_bin_dir().map(|d| d.join("spicetify-cli"))
}

/// Onde o binário está: override do usuário, pasta gerida, PATH, ou a pasta
/// que o `install.sh` oficial usa (`~/.spicetify`, que nem sempre está no PATH).
pub async fn locate() -> Option<(PathBuf, &'static str)> {
    if let Some(custom) = crate::core::binary_overrides::get("spicetify") {
        return Some((custom, "custom"));
    }
    if let Some(managed) = managed_dir().map(|d| d.join(bin_name("spicetify"))) {
        if managed.exists() {
            return Some((managed, "managed"));
        }
    }
    if let Some(found) = dependencies::find_tool_with_source("spicetify").await {
        return Some(found);
    }
    if let Some(home) = dirs::home_dir() {
        let p = home.join(".spicetify").join(bin_name("spicetify"));
        if p.exists() {
            return Some((p, "system"));
        }
    }
    None
}

#[derive(Debug, Clone, Serialize)]
pub struct CmdOutput {
    pub ok: bool,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// O Spicetify colore a saída com ANSI; na UI isso vira lixo.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
            }
            while let Some(&n) = chars.peek() {
                chars.next();
                if n.is_ascii_alphabetic() {
                    break;
                }
            }
        } else if c != '\r' {
            out.push(c);
        }
    }
    out
}

pub async fn run(bin: &Path, args: &[&str]) -> anyhow::Result<CmdOutput> {
    let mut cmd = crate::core::process::command(bin);
    cmd.args(args)
        .env("NO_COLOR", "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let output = tokio::time::timeout(std::time::Duration::from_secs(240), cmd.output())
        .await
        .map_err(|_| anyhow!("spicetify {} nao respondeu em 4 minutos", args.join(" ")))??;
    Ok(CmdOutput {
        ok: output.status.success(),
        code: output.status.code(),
        stdout: strip_ansi(&String::from_utf8_lossy(&output.stdout)),
        stderr: strip_ansi(&String::from_utf8_lossy(&output.stderr)),
    })
}

/// Falha vira `Err` com a mensagem que o Spicetify imprimiu, para a UI
/// mostrar o motivo real ("Spotify not found", "already patched"…).
pub async fn run_ok(bin: &Path, args: &[&str]) -> anyhow::Result<CmdOutput> {
    let out = run(bin, args).await?;
    if out.ok {
        return Ok(out);
    }
    let msg = if out.stderr.trim().is_empty() {
        out.stdout.trim().to_string()
    } else {
        out.stderr.trim().to_string()
    };
    Err(anyhow!(
        "spicetify {} falhou{}",
        args.join(" "),
        if msg.is_empty() {
            String::new()
        } else {
            format!(": {}", msg)
        }
    ))
}

// ---------- config.ini ----------

#[derive(Debug, Default, Clone, Serialize)]
pub struct SpicetifyConfig {
    pub spotify_path: String,
    pub prefs_path: String,
    pub current_theme: String,
    pub color_scheme: String,
    pub extensions: Vec<String>,
    pub custom_apps: Vec<String>,
    pub backup_version: String,
}

fn parse_ini(text: &str) -> Vec<(String, String, String)> {
    let mut section = String::new();
    let mut out = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = name.trim().to_string();
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            out.push((section.clone(), k.trim().to_string(), v.trim().to_string()));
        }
    }
    out
}

fn split_list(v: &str) -> Vec<String> {
    v.split('|')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

pub fn read_config(config_path: &Path) -> SpicetifyConfig {
    let Ok(text) = std::fs::read_to_string(config_path) else {
        return SpicetifyConfig::default();
    };
    let mut cfg = SpicetifyConfig::default();
    for (section, key, value) in parse_ini(&text) {
        match (section.as_str(), key.as_str()) {
            ("Setting", "spotify_path") => cfg.spotify_path = value,
            ("Setting", "prefs_path") => cfg.prefs_path = value,
            ("Setting", "current_theme") => cfg.current_theme = value,
            ("Setting", "color_scheme") => cfg.color_scheme = value,
            ("AdditionalOptions", "extensions") => cfg.extensions = split_list(&value),
            ("AdditionalOptions", "custom_apps") => cfg.custom_apps = split_list(&value),
            ("Backup", "version") => cfg.backup_version = value,
            _ => {}
        }
    }
    cfg
}

#[derive(Debug, Clone, Serialize)]
pub struct ThemeInfo {
    pub name: String,
    pub schemes: Vec<String>,
}

/// Temas em `<config>/Themes/<nome>/color.ini`; cada seção do ini é um esquema.
pub fn list_themes(config_dir: &Path) -> Vec<ThemeInfo> {
    let mut themes = Vec::new();
    let Ok(entries) = std::fs::read_dir(config_dir.join("Themes")) else {
        return themes;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let schemes = std::fs::read_to_string(path.join("color.ini"))
            .map(|text| {
                text.lines()
                    .map(str::trim)
                    .filter_map(|l| l.strip_prefix('[').and_then(|l| l.strip_suffix(']')))
                    .map(|s| s.trim().to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        themes.push(ThemeInfo { name, schemes });
    }
    themes.sort_by_key(|a| a.name.to_lowercase());
    themes
}

// ---------- status ----------

#[derive(Debug, Clone, Serialize)]
pub struct SpicetifyStatus {
    pub installed: bool,
    pub version: Option<String>,
    pub source: String,
    pub path: Option<String>,
    pub config_dir: Option<String>,
    pub config_path: Option<String>,
    pub spotify_path: Option<String>,
    pub spotify_error: Option<String>,
    pub applied: bool,
    pub current_theme: String,
    pub color_scheme: String,
    pub extensions: Vec<String>,
    pub custom_apps: Vec<String>,
    pub marketplace_installed: bool,
    pub themes: Vec<ThemeInfo>,
    pub flatpak: bool,
}

pub async fn status() -> SpicetifyStatus {
    let flatpak = dependencies::is_flatpak();
    let Some((bin, source)) = locate().await else {
        return SpicetifyStatus {
            installed: false,
            version: None,
            source: "missing".into(),
            path: None,
            config_dir: None,
            config_path: None,
            spotify_path: None,
            spotify_error: None,
            applied: false,
            current_theme: String::new(),
            color_scheme: String::new(),
            extensions: vec![],
            custom_apps: vec![],
            marketplace_installed: false,
            themes: vec![],
            flatpak,
        };
    };

    let version = run(&bin, &["-v"])
        .await
        .ok()
        .filter(|o| o.ok)
        .map(|o| o.stdout.trim().to_string())
        .filter(|v| !v.is_empty());

    // `-c` imprime o caminho do config.ini e o cria se não existir.
    // Na primeira execução o CLI imprime "Default config generated" antes
    // do caminho, então o caminho é a última linha, não a saída inteira.
    let config_path = run(&bin, &["-c"])
        .await
        .ok()
        .filter(|o| o.ok)
        .and_then(|o| {
            o.stdout
                .lines()
                .map(str::trim)
                .rfind(|l| !l.is_empty())
                .map(PathBuf::from)
        })
        .filter(|p| p.exists());
    let config_dir = config_path
        .as_ref()
        .and_then(|p| p.parent().map(Path::to_path_buf));

    let cfg = config_path
        .as_ref()
        .map(|p| read_config(p))
        .unwrap_or_default();
    // O CLI detecta o Spotify sozinho e grava em `spotify_path`; vazio ou
    // apontando para algo que não existe significa "não encontrado".
    let spotify_path = Some(cfg.spotify_path.trim().to_string())
        .filter(|p| !p.is_empty() && Path::new(p).exists());
    let spotify_error = if spotify_path.is_none() && config_path.is_some() {
        Some("spotify_not_found".to_string())
    } else {
        None
    };
    let backup_nonempty = config_dir
        .as_ref()
        .and_then(|d| std::fs::read_dir(d.join("Backup")).ok())
        .map(|mut it| it.next().is_some())
        .unwrap_or(false);
    let applied = !cfg.backup_version.is_empty() || backup_nonempty;
    // O CLI procura temas na pasta de config e, se não achar, na pasta do
    // próprio executável (é onde vive o SpicetifyDefault do release).
    let mut themes = config_dir
        .as_ref()
        .map(|d| list_themes(d))
        .unwrap_or_default();
    if let Some(exe_dir) = bin.parent() {
        for theme in list_themes(exe_dir) {
            if !themes.iter().any(|t| t.name == theme.name) {
                themes.push(theme);
            }
        }
        themes.sort_by_key(|a| a.name.to_lowercase());
    }
    let marketplace_installed = cfg.custom_apps.iter().any(|a| a == "marketplace")
        && config_dir
            .as_ref()
            .map(|d| d.join("CustomApps").join("marketplace").is_dir())
            .unwrap_or(false);

    SpicetifyStatus {
        installed: true,
        version,
        source: source.to_string(),
        path: Some(bin.to_string_lossy().to_string()),
        config_dir: config_dir.map(|p| p.to_string_lossy().to_string()),
        config_path: config_path.map(|p| p.to_string_lossy().to_string()),
        spotify_path,
        spotify_error,
        applied,
        current_theme: cfg.current_theme,
        color_scheme: cfg.color_scheme,
        extensions: cfg.extensions,
        custom_apps: cfg.custom_apps,
        marketplace_installed,
        themes,
        flatpak,
    }
}

// ---------- download do release ----------

fn github_client() -> anyhow::Result<reqwest::Client> {
    use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, USER_AGENT};
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, HeaderValue::from_static("OmniGet"));
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("application/vnd.github+json"),
    );
    Ok(
        crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(300))
            .build()?,
    )
}

struct ReleaseAsset {
    tag: String,
    name: String,
    url: String,
    digest: Option<String>,
}

/// Asset do último release cujo nome termina com `suffix`, com o `digest`
/// que a API do GitHub publica.
async fn latest_asset(
    client: &reqwest::Client,
    repo: &str,
    pick: impl Fn(&str) -> bool,
) -> anyhow::Result<ReleaseAsset> {
    let url = format!("https://api.github.com/repos/{}/releases/latest", repo);
    let response = client.get(&url).send().await?;
    if !response.status().is_success() {
        return Err(anyhow!(
            "nao foi possivel consultar releases de {}: HTTP {}",
            repo,
            response.status()
        ));
    }
    let json: serde_json::Value = response.json().await?;
    let tag = json["tag_name"].as_str().unwrap_or("").to_string();
    let assets = json["assets"]
        .as_array()
        .ok_or_else(|| anyhow!("release de {} sem assets", repo))?;
    for asset in assets {
        let name = asset["name"].as_str().unwrap_or("");
        if pick(name) {
            return Ok(ReleaseAsset {
                tag: tag.clone(),
                name: name.to_string(),
                url: asset["browser_download_url"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
                digest: asset["digest"]
                    .as_str()
                    .and_then(integrity::parse_github_digest),
            });
        }
    }
    Err(anyhow!(
        "release {} de {} nao tem um asset para este sistema",
        tag,
        repo
    ))
}

async fn download_verified(client: &reqwest::Client, asset: &ReleaseAsset) -> anyhow::Result<Vec<u8>> {
    let response = client.get(&asset.url).send().await?;
    if !response.status().is_success() {
        return Err(anyhow!(
            "download de {} falhou: HTTP {}",
            asset.name,
            response.status()
        ));
    }
    let bytes = response.bytes().await?.to_vec();
    // O GitHub publica o digest de todo asset; sem ele algo está errado na
    // resposta, e o binário vai ser executado. Fail-closed.
    let expected = asset
        .digest
        .as_deref()
        .ok_or_else(|| anyhow!("{} veio sem digest da API do GitHub; download descartado", asset.name))?;
    integrity::verify_sha256(&bytes, expected, &asset.name)?;
    Ok(bytes)
}

/// Sufixo do asset do CLI para este sistema. Linux só tem amd64 no release.
fn cli_asset_suffix() -> anyhow::Result<&'static str> {
    Ok(if cfg!(target_os = "windows") {
        if cfg!(target_arch = "aarch64") {
            "windows-arm64.zip"
        } else if cfg!(target_pointer_width = "32") {
            "windows-x32.zip"
        } else {
            "windows-x64.zip"
        }
    } else if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            "darwin-arm64.tar.gz"
        } else {
            "darwin-amd64.tar.gz"
        }
    } else if cfg!(target_arch = "x86_64") {
        "linux-amd64.tar.gz"
    } else {
        return Err(anyhow!(
            "o Spicetify nao publica binario para Linux nesta arquitetura; instale pelo gerenciador de pacotes"
        ));
    })
}

fn unpack_zip(data: &[u8], dest: &Path) -> anyhow::Result<()> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(data))
        .map_err(|e| anyhow!("zip invalido: {}", e))?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let Some(rel) = file.enclosed_name() else {
            continue;
        };
        let out = dest.join(rel);
        if file.is_dir() {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut w = std::fs::File::create(&out)?;
        std::io::copy(&mut file, &mut w)?;
    }
    Ok(())
}

fn unpack_tar_gz(data: &[u8], dest: &Path) -> anyhow::Result<()> {
    let decoder = flate2::read::GzDecoder::new(std::io::Cursor::new(data));
    let mut archive = tar::Archive::new(decoder);
    archive.set_preserve_permissions(true);
    archive
        .unpack(dest)
        .map_err(|e| anyhow!("tar.gz invalido: {}", e))?;
    Ok(())
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755));
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) {}

/// Baixa o último release do CLI para `<app_data>/bin/spicetify-cli/`.
/// A pasta antiga só sai depois que a nova está inteira no disco.
pub async fn install() -> anyhow::Result<PathBuf> {
    if dependencies::is_flatpak() {
        return Err(anyhow!(
            "dentro do Flatpak o Spicetify nao consegue alterar o Spotify do sistema"
        ));
    }
    let suffix = cli_asset_suffix()?;
    let dir = managed_dir().ok_or_else(|| anyhow!("Could not determine data directory"))?;
    let client = github_client()?;
    let asset = latest_asset(&client, CLI_REPO, |n| n.ends_with(suffix)).await?;
    tracing::info!("[spicetify] baixando {} ({})", asset.name, asset.tag);
    let data = download_verified(&client, &asset).await?;

    let staging = dir.with_extension("new");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)?;
    let is_zip = asset.name.ends_with(".zip");
    let staging_clone = staging.clone();
    tokio::task::spawn_blocking(move || {
        if is_zip {
            unpack_zip(&data, &staging_clone)
        } else {
            unpack_tar_gz(&data, &staging_clone)
        }
    })
    .await
    .map_err(|e| anyhow!("Spawn blocking failed: {}", e))??;

    let exe = staging.join(bin_name("spicetify"));
    if !exe.exists() {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(anyhow!("o arquivo baixado nao contem o executavel do spicetify"));
    }
    make_executable(&exe);

    let old = dir.with_extension("old");
    let _ = std::fs::remove_dir_all(&old);
    if dir.exists() {
        std::fs::rename(&dir, &old)?;
    }
    if let Err(e) = std::fs::rename(&staging, &dir) {
        if old.exists() {
            let _ = std::fs::rename(&old, &dir);
        }
        return Err(anyhow!("nao foi possivel mover a instalacao para o lugar: {}", e));
    }
    let _ = std::fs::remove_dir_all(&old);

    let final_exe = dir.join(bin_name("spicetify"));
    #[cfg(target_os = "macos")]
    {
        let quarantine_target = dir.clone();
        let _ = tokio::task::spawn_blocking(move || {
            crate::core::process::std_command("xattr")
                .args(["-dr", "com.apple.quarantine"])
                .arg(&quarantine_target)
                .output()
        })
        .await;
    }
    tracing::info!("[spicetify] instalado em {}", final_exe.display());
    Ok(final_exe)
}

// ---------- Marketplace ----------

/// Repete o `install.sh` oficial do Marketplace: zip do release em
/// `CustomApps/marketplace`, registra o custom app e, sem tema ativo, usa o
/// tema "marketplace" que o próprio Marketplace espera.
pub async fn install_marketplace(bin: &Path, config_dir: &Path) -> anyhow::Result<CmdOutput> {
    let client = github_client()?;
    let asset = latest_asset(&client, MARKETPLACE_REPO, |n| n == "marketplace.zip").await?;
    let data = download_verified(&client, &asset).await?;

    let apps = config_dir.join("CustomApps");
    std::fs::create_dir_all(&apps)?;
    let staging = apps.join("marketplace-tmp");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)?;
    let staging_clone = staging.clone();
    tokio::task::spawn_blocking(move || unpack_zip(&data, &staging_clone))
        .await
        .map_err(|e| anyhow!("Spawn blocking failed: {}", e))??;
    let dist = staging.join("marketplace-dist");
    let src = if dist.is_dir() { dist } else { staging.clone() };
    let target = apps.join("marketplace");
    let _ = std::fs::remove_dir_all(&target);
    std::fs::rename(&src, &target)?;
    let _ = std::fs::remove_dir_all(&staging);

    let color_ini = client.get(MARKETPLACE_COLOR_INI).send().await?;
    if color_ini.status().is_success() {
        let theme_dir = config_dir.join("Themes").join("marketplace");
        std::fs::create_dir_all(&theme_dir)?;
        std::fs::write(theme_dir.join("color.ini"), color_ini.bytes().await?)?;
    }

    run_ok(bin, &["config", "custom_apps", "spicetify-marketplace-"]).await.ok();
    run_ok(bin, &["config", "custom_apps", "marketplace"]).await?;
    run_ok(bin, &["config", "inject_css", "1", "replace_colors", "1"]).await?;
    let cfg = read_config(&config_dir.join("config-xpui.ini"));
    if cfg.current_theme.trim().len() <= 3 {
        run_ok(
            bin,
            &["config", "current_theme", "marketplace", "color_scheme", "marketplace"],
        )
        .await?;
    }
    let applied = !cfg.backup_version.is_empty();
    if applied {
        run_ok(bin, &["apply"]).await
    } else {
        run_ok(bin, &["backup", "apply"]).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ini_le_listas_e_secoes() {
        let text = "[Setting]\ncurrent_theme = Dribbblish\ncolor_scheme = dark\n\n[AdditionalOptions]\nextensions = a.js|b.js\ncustom_apps = marketplace\n[Backup]\nversion = 1.2.3\n";
        let dir = std::env::temp_dir().join(format!("omniget-spicetify-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config-xpui.ini");
        std::fs::write(&path, text).unwrap();
        let cfg = read_config(&path);
        assert_eq!(cfg.current_theme, "Dribbblish");
        assert_eq!(cfg.extensions, vec!["a.js", "b.js"]);
        assert_eq!(cfg.custom_apps, vec!["marketplace"]);
        assert_eq!(cfg.backup_version, "1.2.3");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ansi_e_removido() {
        assert_eq!(strip_ansi("\x1b[32msuccess\x1b[0m ok\r\n"), "success ok\n");
    }
}

#[cfg(test)]
mod real_tests {
    //! Roda de verdade contra o GitHub e o disco. Só com `--ignored`.
    use super::*;

    #[tokio::test]
    #[ignore]
    async fn le_status() {
        let st = status().await;
        println!("{}", serde_json::to_string_pretty(&st).unwrap());
    }

    #[tokio::test]
    #[ignore]
    async fn instala_e_le_status() {
        let exe = install().await.expect("install");
        assert!(exe.exists());
        let st = status().await;
        println!("{}", serde_json::to_string_pretty(&st).unwrap());
        assert!(st.installed);
        assert!(st.version.is_some());
    }
}
