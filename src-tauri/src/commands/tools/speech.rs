use omniget_core::core::subtitle_merge::{cues_to_srt, parse_cues};
use omniget_core::core::tools::{dub, edge_tts, srt_translate, whisper};
use serde::Serialize;

use super::{err, progress};

#[tauri::command]
pub async fn tool_whisper_status() -> whisper::WhisperStatus {
    whisper::status().await
}

#[tauri::command]
pub async fn tool_whisper_install(
    app: tauri::AppHandle,
    variant: String,
) -> Result<String, String> {
    whisper::install(&variant, progress(&app))
        .await
        .map(|p| p.to_string_lossy().to_string())
        .map_err(err)
}

#[tauri::command]
pub async fn tool_whisper_model_download(
    app: tauri::AppHandle,
    id: String,
) -> Result<String, String> {
    whisper::download_model(&id, progress(&app))
        .await
        .map(|p| p.to_string_lossy().to_string())
        .map_err(err)
}

#[tauri::command]
pub async fn tool_whisper_model_remove(id: String) -> Result<(), String> {
    whisper::remove_model(&id).map_err(err)
}

#[tauri::command]
pub async fn tool_whisper_transcribe(
    app: tauri::AppHandle,
    opts: whisper::TranscribeOptions,
) -> Result<whisper::TranscribeResult, String> {
    whisper::transcribe(opts, progress(&app)).await.map_err(err)
}

#[tauri::command]
pub async fn tool_tts_voices() -> Result<Vec<edge_tts::Voice>, String> {
    edge_tts::list_voices().await.map_err(err)
}

#[tauri::command]
pub async fn tool_tts_speak(
    app: tauri::AppHandle,
    opts: edge_tts::TtsOptions,
    output_path: String,
) -> Result<edge_tts::TtsResult, String> {
    edge_tts::synthesize(opts, std::path::Path::new(&output_path), progress(&app))
        .await
        .map_err(err)
}

#[derive(Serialize)]
pub struct SrtTranslateOut {
    pub output_path: String,
    pub cues: usize,
    pub failed: usize,
}

/// Traduz um arquivo de legenda inteiro e grava `<nome>.<lang>.srt`.
#[tauri::command]
pub async fn tool_srt_translate(
    app: tauri::AppHandle,
    srt_path: String,
    opts: srt_translate::TranslateOptions,
    bilingual: bool,
    output_path: Option<String>,
) -> Result<SrtTranslateOut, String> {
    let text = tokio::fs::read_to_string(&srt_path).await.map_err(err)?;
    let cues = parse_cues(&text);
    if cues.is_empty() {
        return Err("a legenda esta vazia ou num formato desconhecido".into());
    }
    let target = opts.target_lang.clone();
    let result = srt_translate::translate_cues(&cues, &opts, progress(&app))
        .await
        .map_err(err)?;
    let mut out = result.cues;
    if bilingual {
        for (i, c) in out.iter_mut().enumerate() {
            if cues[i].text != c.text {
                c.text = format!("{}\n{}", c.text, cues[i].text);
            }
        }
    }
    let out_path = output_path
        .filter(|p| !p.trim().is_empty())
        .unwrap_or_else(|| {
            let p = std::path::Path::new(&srt_path);
            let stem = p
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "legenda".into());
            p.with_file_name(format!("{}.{}.srt", stem, target))
                .to_string_lossy()
                .to_string()
        });
    tokio::fs::write(&out_path, cues_to_srt(&out))
        .await
        .map_err(err)?;
    Ok(SrtTranslateOut {
        output_path: out_path,
        cues: out.len(),
        failed: result.failed.len(),
    })
}

#[tauri::command]
pub async fn tool_dub(
    app: tauri::AppHandle,
    opts: dub::DubOptions,
) -> Result<dub::DubResult, String> {
    dub::dub(opts, progress(&app)).await.map_err(err)
}
