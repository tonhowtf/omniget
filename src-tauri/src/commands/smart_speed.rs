use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Stdio;

use crate::core::silence_map::{needs_recompute, savings_secs, SilenceMap, CURRENT_VERSION};

const MAPS_FILE: &str = "silence-maps.json";

fn file_path() -> Option<PathBuf> {
    crate::core::paths::app_data_dir().map(|d| d.join(MAPS_FILE))
}

fn load() -> BTreeMap<String, SilenceMap> {
    let Some(path) = file_path() else {
        return BTreeMap::new();
    };
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return BTreeMap::new();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

fn save(map: &BTreeMap<String, SilenceMap>) {
    let Some(path) = file_path() else { return };
    if let Some(parent) = path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
    }
    let Ok(json) = serde_json::to_string(map) else {
        return;
    };
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, json).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

fn parse_duration_secs(stderr: &str) -> Option<f64> {
    let idx = stderr.find("Duration:")?;
    let resto = &stderr[idx + "Duration:".len()..];
    let bruto = resto.split(',').next()?.trim();
    let mut partes = bruto.split(':');
    let h: f64 = partes.next()?.trim().parse().ok()?;
    let m: f64 = partes.next()?.trim().parse().ok()?;
    let s: f64 = partes.next()?.trim().parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + s)
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SilenceMapInfo {
    pub map: SilenceMap,
    pub savings_secs: f64,
    pub from_cache: bool,
}

#[tauri::command]
pub async fn compute_silence_map(
    path: String,
    force: Option<bool>,
) -> Result<SilenceMapInfo, String> {
    let force = force.unwrap_or(false);
    let arquivo = PathBuf::from(&path);
    if !arquivo.is_file() {
        return Err("O arquivo nao existe".to_string());
    }

    if !force {
        if let Some(cached) = load().get(&path) {
            if !needs_recompute(cached) {
                return Ok(SilenceMapInfo {
                    savings_secs: savings_secs(cached),
                    map: cached.clone(),
                    from_cache: true,
                });
            }
        }
    }

    let ffmpeg = crate::core::dependencies::find_tool("ffmpeg")
        .await
        .ok_or_else(|| "FFmpeg nao encontrado. Instale em Config → Dependências.".to_string())?;

    let alvo = arquivo.clone();
    let stderr = tokio::task::spawn_blocking(move || {
        let mut cmd = crate::core::process::std_command(&ffmpeg);
        cmd.arg("-i").arg(&alvo);
        for a in omniget_core::core::ffmpeg_ops::silence_probe_args() {
            cmd.arg(a);
        }
        cmd.arg("-")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;

    let texto = String::from_utf8_lossy(&stderr.stderr);
    let duracao = parse_duration_secs(&texto)
        .ok_or_else(|| "Nao foi possivel ler a duracao do arquivo".to_string())?;

    let mapa = SilenceMap {
        version: CURRENT_VERSION,
        media_duration_secs: duracao,
        spans: crate::core::silence_map::parse_spans(&texto),
    };

    let mut todos = load();
    todos.insert(path, mapa.clone());
    save(&todos);

    Ok(SilenceMapInfo {
        savings_secs: savings_secs(&mapa),
        map: mapa,
        from_cache: false,
    })
}
