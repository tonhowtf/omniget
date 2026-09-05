//! Transcrição local com whisper.cpp (estudo 01).
//!
//! O binário `whisper-cli` vem da release oficial no Windows (x64, CPU ou
//! cuBLAS) e no Linux (Ubuntu x64/arm64). No macOS o upstream só publica o
//! xcframework, então lá o OmniGet usa o `whisper-cli` do sistema (Homebrew:
//! `brew install whisper-cpp`) ou um caminho escolhido pelo usuário.
//! Modelos GGML vêm do Hugging Face para `<app_data>/models/whisper/`.

use std::path::{Path, PathBuf};

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

use super::{github, report, ProgressFn};
use crate::core::dependencies::{self, bin_name};
use crate::core::subtitle_merge::{cues_to_srt, cues_to_vtt, Cue};

const REPO: &str = "ggml-org/whisper.cpp";
const HF_BASE: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

#[derive(Debug, Clone, Serialize)]
pub struct ModelInfo {
    pub id: String,
    pub label: String,
    pub size_mb: u64,
    pub note: String,
    pub installed: bool,
    pub path: Option<String>,
    pub size_bytes: u64,
}

/// (id, tamanho aproximado em MB, nota curta)
const MODELS: &[(&str, u64, &str)] = &[
    ("tiny-q5_1", 32, "rápido, baixa qualidade"),
    ("base-q5_1", 60, "rápido, qualidade básica"),
    ("small-q5_1", 190, "equilíbrio para máquinas fracas"),
    ("medium-q5_0", 540, "boa qualidade, mais lento"),
    (
        "large-v3-turbo-q5_0",
        574,
        "recomendado: quase large-v3, 8× mais rápido",
    ),
    ("large-v3-turbo", 1620, "turbo sem quantização"),
    ("large-v3-q5_0", 1080, "melhor qualidade quantizado"),
    ("large-v3", 3100, "melhor qualidade, mais pesado"),
];

pub fn models_dir() -> Option<PathBuf> {
    crate::core::paths::app_data_dir().map(|d| d.join("models").join("whisper"))
}

fn managed_dir() -> Option<PathBuf> {
    dependencies::managed_bin_dir().map(|d| d.join("whisper-cpp"))
}

pub fn list_models() -> Vec<ModelInfo> {
    let dir = models_dir();
    MODELS
        .iter()
        .map(|(id, mb, note)| {
            let path = dir.as_ref().map(|d| d.join(format!("ggml-{}.bin", id)));
            let size = path
                .as_ref()
                .and_then(|p| std::fs::metadata(p).ok())
                .map(|m| m.len())
                .unwrap_or(0);
            ModelInfo {
                id: id.to_string(),
                label: id
                    .replace('-', " ")
                    .replace("q5_0", "(q5)")
                    .replace("q5_1", "(q5)"),
                size_mb: *mb,
                note: note.to_string(),
                installed: size > 0,
                path: path
                    .filter(|_| size > 0)
                    .map(|p| p.to_string_lossy().to_string()),
                size_bytes: size,
            }
        })
        .collect()
}

pub async fn download_model(id: &str, progress: ProgressFn) -> anyhow::Result<PathBuf> {
    if !MODELS.iter().any(|(m, _, _)| *m == id) {
        return Err(anyhow!("modelo desconhecido: {}", id));
    }
    let dir = models_dir().ok_or_else(|| anyhow!("Could not determine data directory"))?;
    std::fs::create_dir_all(&dir)?;
    let dest = dir.join(format!("ggml-{}.bin", id));
    let url = format!("{}/ggml-{}.bin", HF_BASE, id);
    let client = super::client()?;
    super::download_to(
        &client,
        &url,
        &dest,
        &progress,
        &format!("whisper-model:{}", id),
    )
    .await?;
    Ok(dest)
}

pub fn remove_model(id: &str) -> anyhow::Result<()> {
    let dir = models_dir().ok_or_else(|| anyhow!("Could not determine data directory"))?;
    let p = dir.join(format!("ggml-{}.bin", id));
    if p.exists() {
        std::fs::remove_file(p)?;
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct WhisperStatus {
    pub installed: bool,
    pub path: Option<String>,
    pub source: String,
    pub version: Option<String>,
    /// Variantes que a release publica para este sistema (vazio no macOS).
    pub variants: Vec<String>,
    pub can_install: bool,
    pub install_hint: String,
    pub models: Vec<ModelInfo>,
    pub models_dir: Option<String>,
}

pub async fn locate() -> Option<(PathBuf, &'static str)> {
    if let Some(custom) = crate::core::binary_overrides::get("whisper-cli") {
        return Some((custom, "custom"));
    }
    if let Some(dir) = managed_dir() {
        if let Some(p) = github::find_file(&dir, &bin_name("whisper-cli")) {
            return Some((p, "managed"));
        }
    }
    if let Some(found) = dependencies::find_tool_with_source("whisper-cli").await {
        return Some(found);
    }
    // Homebrew instala como whisper-cli; builds antigas chamavam "main"/"whisper".
    for name in ["whisper-cpp", "whisper"] {
        if let Some(found) = dependencies::find_tool_with_source(name).await {
            return Some(found);
        }
    }
    None
}

fn variants() -> Vec<String> {
    if cfg!(target_os = "windows") && cfg!(target_arch = "x86_64") {
        vec!["cpu".into(), "cuda".into()]
    } else if cfg!(target_os = "linux")
        && (cfg!(target_arch = "x86_64") || cfg!(target_arch = "aarch64"))
    {
        vec!["cpu".into()]
    } else {
        vec![]
    }
}

fn asset_name(variant: &str) -> anyhow::Result<&'static str> {
    Ok(if cfg!(target_os = "windows") {
        match variant {
            "cuda" => "whisper-cublas-12.4.0-bin-x64.zip",
            _ => "whisper-bin-x64.zip",
        }
    } else if cfg!(target_os = "linux") {
        if cfg!(target_arch = "aarch64") {
            "whisper-bin-ubuntu-arm64.tar.gz"
        } else {
            "whisper-bin-ubuntu-x64.tar.gz"
        }
    } else {
        return Err(anyhow!(
            "o whisper.cpp nao publica binario para este sistema; instale com `brew install whisper-cpp` ou aponte o caminho em Configuracoes"
        ));
    })
}

pub async fn status() -> WhisperStatus {
    let located = locate().await;
    let (path, source) = match &located {
        Some((p, s)) => (Some(p.clone()), *s),
        None => (None, "missing"),
    };
    let version = match &path {
        Some(p) => dependencies::check_version_at_path(p, "whisper-cli").await,
        None => None,
    };
    let v = variants();
    WhisperStatus {
        installed: path.is_some(),
        path: path.map(|p| p.to_string_lossy().to_string()),
        source: source.to_string(),
        version,
        can_install: !v.is_empty(),
        variants: v,
        install_hint: if cfg!(target_os = "macos") {
            "brew install whisper-cpp".to_string()
        } else {
            String::new()
        },
        models: list_models(),
        models_dir: models_dir().map(|d| d.to_string_lossy().to_string()),
    }
}

pub async fn install(variant: &str, progress: ProgressFn) -> anyhow::Result<PathBuf> {
    let name = asset_name(variant)?;
    let dir = managed_dir().ok_or_else(|| anyhow!("Could not determine data directory"))?;
    let client = github::client()?;
    let asset = github::asset(&client, REPO, None, |n| n == name).await?;
    tracing::info!("[whisper] baixando {} ({})", asset.name, asset.tag);
    let data = github::download(&client, &asset, false, &progress, "whisper-cli").await?;
    let staging = dir.with_extension("new");
    let _ = std::fs::remove_dir_all(&staging);
    let staging_c = staging.clone();
    let asset_name_c = asset.name.clone();
    tokio::task::spawn_blocking(move || github::unpack(&data, &asset_name_c, &staging_c))
        .await
        .map_err(|e| anyhow!("Spawn blocking failed: {}", e))??;
    let Some(exe) = github::find_file(&staging, &bin_name("whisper-cli")) else {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(anyhow!("o pacote baixado nao contem o whisper-cli"));
    };
    github::make_executable(&exe);
    // As libs (.dll/.so) ficam ao lado do executável no mesmo pacote.
    github::swap_dir(&staging, &dir)?;
    github::strip_quarantine(&dir).await;
    let final_exe = github::find_file(&dir, &bin_name("whisper-cli"))
        .ok_or_else(|| anyhow!("whisper-cli sumiu depois de mover a pasta"))?;
    std::fs::write(dir.join("VERSION"), &asset.tag).ok();
    Ok(final_exe)
}

#[derive(Debug, Clone, Deserialize)]
pub struct TranscribeOptions {
    pub input: String,
    pub model: String,
    /// "auto" ou código ISO ("pt", "en").
    pub language: String,
    /// Traduzir a saída para inglês (recurso nativo do Whisper).
    #[serde(default)]
    pub translate: bool,
    /// Tamanho máximo de segmento em caracteres (0 = deixar o modelo decidir).
    #[serde(default)]
    pub max_len: u32,
    #[serde(default)]
    pub prompt: String,
    /// Pasta de saída; vazio = ao lado do arquivo de entrada.
    #[serde(default)]
    pub output_dir: String,
    #[serde(default)]
    pub threads: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct TranscribeResult {
    pub language: String,
    pub cues: Vec<Cue>,
    pub text: String,
    pub srt_path: String,
    pub vtt_path: String,
    pub txt_path: String,
    pub seconds: f64,
}

/// Áudio como o whisper.cpp quer: WAV 16 kHz mono PCM16.
async fn to_wav16k(input: &Path) -> anyhow::Result<PathBuf> {
    let ffmpeg = crate::core::dependencies::ensure_ffmpeg().await?;
    let out = super::temp_dir().join(format!("whisper-{}.wav", uuid::Uuid::new_v4()));
    let output = crate::core::process::command(&ffmpeg)
        .args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(input)
        .args(["-vn", "-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le"])
        .arg(&out)
        .output()
        .await?;
    if !output.status.success() {
        return Err(anyhow!(
            "ffmpeg nao conseguiu converter o audio: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(out)
}

fn parse_whisper_json(text: &str) -> anyhow::Result<(String, Vec<Cue>)> {
    let json: serde_json::Value = serde_json::from_str(text)?;
    let language = json["result"]["language"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let mut cues = Vec::new();
    if let Some(items) = json["transcription"].as_array() {
        for it in items {
            let from = it["offsets"]["from"].as_u64().unwrap_or(0);
            let to = it["offsets"]["to"].as_u64().unwrap_or(from);
            let t = it["text"].as_str().unwrap_or("").trim().to_string();
            if t.is_empty() {
                continue;
            }
            cues.push(Cue {
                start_ms: from,
                end_ms: to.max(from + 1),
                text: t,
            });
        }
    }
    Ok((language, cues))
}

pub async fn transcribe(
    opts: TranscribeOptions,
    progress: ProgressFn,
) -> anyhow::Result<TranscribeResult> {
    let (bin, _) = locate()
        .await
        .ok_or_else(|| anyhow!("whisper-cli nao esta instalado"))?;
    let model_path = models_dir()
        .ok_or_else(|| anyhow!("Could not determine data directory"))?
        .join(format!("ggml-{}.bin", opts.model));
    if !model_path.exists() {
        return Err(anyhow!("modelo {} nao esta baixado", opts.model));
    }
    let input = PathBuf::from(&opts.input);
    if !input.exists() {
        return Err(anyhow!("arquivo nao encontrado: {}", opts.input));
    }
    let id = format!("transcribe:{}", opts.input);
    report(&progress, &id, "convert", 0, Some(100), None);
    let started = std::time::Instant::now();
    let wav = to_wav16k(&input).await?;
    let out_prefix = super::temp_dir().join(format!("whisper-{}", uuid::Uuid::new_v4()));

    let mut cmd = crate::core::process::command(&bin);
    cmd.arg("-m").arg(&model_path).arg("-f").arg(&wav);
    cmd.args(["-oj", "-pp", "-np", "-of"]).arg(&out_prefix);
    let lang = if opts.language.trim().is_empty() {
        "auto"
    } else {
        opts.language.trim()
    };
    cmd.args(["-l", lang]);
    if opts.translate {
        cmd.arg("-tr");
    }
    if opts.max_len > 0 {
        cmd.args(["-ml", &opts.max_len.to_string(), "-sow"]);
    }
    if !opts.prompt.trim().is_empty() {
        cmd.args(["--prompt", opts.prompt.trim()]);
    }
    if opts.threads > 0 {
        cmd.args(["-t", &opts.threads.to_string()]);
    }
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| anyhow!("nao foi possivel iniciar o whisper-cli: {}", e))?;

    // `-pp` escreve "progress = 42%" no stderr.
    let stderr = child.stderr.take();
    let p2 = progress.clone();
    let id2 = id.clone();
    let stderr_task = tokio::spawn(async move {
        use tokio::io::{AsyncBufReadExt, BufReader};
        let mut tail = String::new();
        if let Some(err) = stderr {
            let mut lines = BufReader::new(err).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Some(rest) = line.trim().strip_prefix("progress =") {
                    if let Ok(pct) = rest.trim().trim_end_matches('%').trim().parse::<u64>() {
                        report(&p2, &id2, "transcribe", pct.min(100), Some(100), None);
                    }
                } else if !line.trim().is_empty() {
                    tail = line;
                }
            }
        }
        tail
    });
    let status = child.wait().await?;
    let tail = stderr_task.await.unwrap_or_default();
    let _ = tokio::fs::remove_file(&wav).await;
    if !status.success() {
        return Err(anyhow!("whisper-cli falhou: {}", tail));
    }
    let json_path = out_prefix.with_extension("json");
    let text = tokio::fs::read_to_string(&json_path).await?;
    let _ = tokio::fs::remove_file(&json_path).await;
    let (language, cues) = parse_whisper_json(&text)?;

    let out_dir = if opts.output_dir.trim().is_empty() {
        input
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."))
    } else {
        PathBuf::from(opts.output_dir.trim())
    };
    std::fs::create_dir_all(&out_dir)?;
    let stem = input
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "transcricao".into());
    let srt_path = out_dir.join(format!("{}.srt", stem));
    let vtt_path = out_dir.join(format!("{}.vtt", stem));
    let txt_path = out_dir.join(format!("{}.txt", stem));
    let plain = cues
        .iter()
        .map(|c| c.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    tokio::fs::write(&srt_path, cues_to_srt(&cues)).await?;
    tokio::fs::write(&vtt_path, cues_to_vtt(&cues)).await?;
    tokio::fs::write(&txt_path, &plain).await?;
    report(&progress, &id, "done", 100, Some(100), None);
    Ok(TranscribeResult {
        language,
        cues,
        text: plain,
        srt_path: srt_path.to_string_lossy().to_string(),
        vtt_path: vtt_path.to_string_lossy().to_string(),
        txt_path: txt_path.to_string_lossy().to_string(),
        seconds: started.elapsed().as_secs_f64(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_whisper_json() {
        let j = r#"{"result":{"language":"pt"},"transcription":[{"timestamps":{"from":"00:00:00,000","to":"00:00:01,500"},"offsets":{"from":0,"to":1500},"text":" Olá mundo"},{"offsets":{"from":1500,"to":1500},"text":"   "}]}"#;
        let (lang, cues) = parse_whisper_json(j).unwrap();
        assert_eq!(lang, "pt");
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].text, "Olá mundo");
        assert_eq!(cues[0].end_ms, 1500);
    }
}
