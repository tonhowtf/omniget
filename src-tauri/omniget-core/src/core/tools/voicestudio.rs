//! Ponte com o VoiceStudio (lote 2, AGPL-3 → só a API HTTP, nada copiado):
//! o app dele sobe um backend FastAPI em `http://localhost:3900`, sem
//! autenticação para loopback. Daqui saem clonagem de voz (`POST /profiles`
//! kind=clone + `POST /generate`), design de voz (`POST /design/describe` →
//! instruct → `/generate`) e isolamento vocal (`POST /clean-audio`, Demucs).
//! Sem o VoiceStudio instalado, a tela oferece o download do release.

use std::path::{Path, PathBuf};

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

pub const DEFAULT_BASE: &str = "http://127.0.0.1:3900";
pub const RELEASES: &str = "https://github.com/debpalash/VoiceStudio/releases/latest";

#[derive(Debug, Clone, Serialize)]
pub struct VsStatus {
    pub base_url: String,
    pub running: bool,
    pub installed: bool,
    pub app_path: Option<String>,
    pub version: Option<String>,
    pub engine: Option<String>,
    pub profiles: Vec<Profile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub instruct: String,
}

fn base(url: &str) -> String {
    let u = url.trim().trim_end_matches('/');
    if u.is_empty() {
        DEFAULT_BASE.to_string()
    } else {
        u.to_string()
    }
}

fn client(secs: u64) -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(secs))
        .no_proxy()
        .build()?)
}

fn find_app() -> Option<PathBuf> {
    let candidates: Vec<PathBuf> = if cfg!(target_os = "macos") {
        vec![
            PathBuf::from("/Applications/VoiceStudio.app"),
            dirs::home_dir()
                .unwrap_or_default()
                .join("Applications/VoiceStudio.app"),
        ]
    } else if cfg!(target_os = "windows") {
        let mut v = Vec::new();
        for var in ["LOCALAPPDATA", "ProgramFiles"] {
            if let Ok(b) = std::env::var(var) {
                v.push(
                    PathBuf::from(&b)
                        .join("VoiceStudio")
                        .join("VoiceStudio.exe"),
                );
                v.push(
                    PathBuf::from(&b)
                        .join("Programs")
                        .join("VoiceStudio")
                        .join("VoiceStudio.exe"),
                );
            }
        }
        v
    } else {
        vec![
            PathBuf::from("/usr/bin/voicestudio"),
            PathBuf::from("/opt/VoiceStudio/voicestudio"),
            dirs::home_dir()
                .unwrap_or_default()
                .join("Applications/VoiceStudio.AppImage"),
            dirs::home_dir()
                .unwrap_or_default()
                .join(".local/bin/voicestudio"),
        ]
    };
    candidates.into_iter().find(|p| p.exists())
}

pub async fn status(base_url: &str) -> VsStatus {
    let b = base(base_url);
    let app = find_app();
    let mut s = VsStatus {
        base_url: b.clone(),
        running: false,
        installed: app.is_some(),
        app_path: app.map(|p| p.to_string_lossy().to_string()),
        version: None,
        engine: None,
        profiles: vec![],
    };
    let Ok(c) = client(4) else { return s };
    if let Ok(r) = c.get(format!("{}/health", b)).send().await {
        if r.status().is_success() {
            s.running = true;
            if let Ok(j) = r.json::<serde_json::Value>().await {
                s.version = j.get("version").and_then(|v| v.as_str()).map(String::from);
            }
        }
    }
    if s.running {
        if let Ok(r) = c.get(format!("{}/engines/tts", b)).send().await {
            if let Ok(j) = r.json::<serde_json::Value>().await {
                s.engine = j
                    .get("active")
                    .and_then(|v| v.as_str())
                    .map(String::from)
                    .or_else(|| j.get("selected").and_then(|v| v.as_str()).map(String::from));
            }
        }
        s.profiles = profiles(&b).await.unwrap_or_default();
    }
    s
}

pub async fn profiles(base_url: &str) -> anyhow::Result<Vec<Profile>> {
    let c = client(10)?;
    let j: serde_json::Value = c
        .get(format!("{}/profiles", base(base_url)))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let arr = j
        .as_array()
        .cloned()
        .or_else(|| j.get("profiles").and_then(|v| v.as_array()).cloned())
        .unwrap_or_default();
    Ok(arr
        .iter()
        .filter_map(|p| {
            Some(Profile {
                id: p.get("id")?.as_str()?.to_string(),
                name: p
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                kind: p
                    .get("kind")
                    .and_then(|v| v.as_str())
                    .unwrap_or("clone")
                    .to_string(),
                language: p
                    .get("language")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                instruct: p
                    .get("instruct")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            })
        })
        .collect())
}

/// Abre o app do VoiceStudio (que sobe o backend).
pub async fn launch() -> anyhow::Result<()> {
    let app = find_app().ok_or_else(|| anyhow!("VoiceStudio nao encontrado"))?;
    if cfg!(target_os = "macos") {
        crate::core::process::command("open")
            .arg(&app)
            .output()
            .await?;
    } else {
        crate::core::process::command(&app).spawn()?;
    }
    Ok(())
}

async fn wav_from(resp: reqwest::Response, output: &Path) -> anyhow::Result<PathBuf> {
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(anyhow!(
            "VoiceStudio HTTP {}: {}",
            status.as_u16(),
            text.chars().take(300).collect::<String>()
        ));
    }
    let bytes = resp.bytes().await?;
    if bytes.len() < 100 {
        return Err(anyhow!("resposta vazia"));
    }
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(output, &bytes)?;
    Ok(output.to_path_buf())
}

fn stamp() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

fn out_path(output_dir: &str, name: &str) -> PathBuf {
    let dir = if output_dir.trim().is_empty() {
        dirs::audio_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_default()
            .join("OmniGet")
    } else {
        PathBuf::from(output_dir.trim())
    };
    dir.join(format!("{} {}.wav", name, stamp()))
}

#[derive(Debug, Clone, Deserialize)]
pub struct CloneOptions {
    #[serde(default)]
    pub base_url: String,
    /// amostra de 3 a 15 s (qualquer formato que o backend aceite)
    #[serde(default)]
    pub sample: String,
    /// o que a amostra diz (opcional, melhora a clonagem)
    #[serde(default)]
    pub sample_text: String,
    /// usar um perfil já salvo em vez da amostra
    #[serde(default)]
    pub profile_id: String,
    /// salvar a amostra como perfil com este nome
    #[serde(default)]
    pub save_as: String,
    pub text: String,
    #[serde(default)]
    pub language: String,
    #[serde(default = "one")]
    pub speed: f64,
    #[serde(default)]
    pub output_dir: String,
}

fn one() -> f64 {
    1.0
}

#[derive(Debug, Clone, Serialize)]
pub struct SpeechResult {
    pub output: String,
    pub profile_id: Option<String>,
    pub seconds: Option<f64>,
}

/// Clona a voz da amostra (ou usa o perfil) e fala o texto.
pub async fn clone_speak(
    opts: CloneOptions,
    progress: super::ProgressFn,
) -> anyhow::Result<SpeechResult> {
    let b = base(&opts.base_url);
    let c = client(900)?;
    let mut profile_id = if opts.profile_id.trim().is_empty() {
        None
    } else {
        Some(opts.profile_id.trim().to_string())
    };
    if profile_id.is_none() && !opts.save_as.trim().is_empty() {
        super::report(
            &progress,
            "voicestudio",
            "profile",
            0,
            None,
            Some(opts.save_as.clone()),
        );
        let sample = std::fs::read(&opts.sample).map_err(|e| anyhow!("amostra: {}", e))?;
        let file_name = Path::new(&opts.sample)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "sample.wav".into());
        let form = reqwest::multipart::Form::new()
            .text("name", opts.save_as.trim().to_string())
            .text("kind", "clone")
            .text("ref_text", opts.sample_text.clone())
            .text(
                "language",
                if opts.language.is_empty() {
                    "Auto".to_string()
                } else {
                    opts.language.clone()
                },
            )
            .part(
                "ref_audio",
                reqwest::multipart::Part::bytes(sample).file_name(file_name),
            );
        let j: serde_json::Value = c
            .post(format!("{}/profiles", b))
            .multipart(form)
            .send()
            .await?
            .error_for_status()
            .map_err(|e| anyhow!("criar perfil: {}", e))?
            .json()
            .await?;
        profile_id = j
            .get("id")
            .or_else(|| j.get("profile_id"))
            .and_then(|v| v.as_str())
            .map(String::from);
    }
    super::report(&progress, "voicestudio", "generate", 0, None, None);
    let mut form = reqwest::multipart::Form::new()
        .text("text", opts.text.clone())
        .text("speed", opts.speed.to_string())
        .text("stream", "false");
    if !opts.language.is_empty() {
        form = form.text("language", opts.language.clone());
    }
    if let Some(pid) = &profile_id {
        form = form.text("profile_id", pid.clone());
    } else {
        let sample = std::fs::read(&opts.sample).map_err(|e| anyhow!("amostra: {}", e))?;
        let file_name = Path::new(&opts.sample)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "sample.wav".into());
        form = form.part(
            "ref_audio",
            reqwest::multipart::Part::bytes(sample).file_name(file_name),
        );
        if !opts.sample_text.trim().is_empty() {
            form = form.text("ref_text", opts.sample_text.clone());
        }
    }
    let resp = c
        .post(format!("{}/generate", b))
        .multipart(form)
        .send()
        .await?;
    let seconds = resp
        .headers()
        .get("x-audio-duration")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<f64>().ok());
    let out = wav_from(resp, &out_path(&opts.output_dir, "Voz clonada")).await?;
    super::report(&progress, "voicestudio", "done", 1, Some(1), None);
    Ok(SpeechResult {
        output: out.to_string_lossy().to_string(),
        profile_id,
        seconds,
    })
}

#[derive(Debug, Clone, Deserialize)]
pub struct DesignOptions {
    #[serde(default)]
    pub base_url: String,
    /// "mulher idosa, voz grave, sotaque britânico, calma"
    pub description: String,
    pub text: String,
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub save_as: String,
    #[serde(default)]
    pub output_dir: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DesignResult {
    pub output: String,
    pub instruct: String,
    pub matched: Vec<String>,
    pub unmatched: Vec<String>,
    pub profile_id: Option<String>,
}

/// Descrição em texto → atributos → voz nova falando o texto.
pub async fn design_speak(
    opts: DesignOptions,
    progress: super::ProgressFn,
) -> anyhow::Result<DesignResult> {
    let b = base(&opts.base_url);
    let c = client(900)?;
    super::report(&progress, "voicestudio", "describe", 0, None, None);
    let j: serde_json::Value = c
        .post(format!("{}/design/describe", b))
        .json(&serde_json::json!({ "description": opts.description }))
        .send()
        .await?
        .error_for_status()
        .map_err(|e| anyhow!("describe: {}", e))?
        .json()
        .await?;
    let instruct = j
        .get("instruct")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let matched: Vec<String> = j
        .get("matched")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|m| m.get("phrase").and_then(|p| p.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let unmatched: Vec<String> = j
        .get("unmatched")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|m| m.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let mut profile_id = None;
    if !opts.save_as.trim().is_empty() {
        let attrs = j.get("attrs").cloned().unwrap_or(serde_json::json!({}));
        let form = reqwest::multipart::Form::new()
            .text("name", opts.save_as.trim().to_string())
            .text("kind", "design")
            .text("instruct", instruct.clone())
            .text("vd_states", attrs.to_string())
            .text(
                "language",
                if opts.language.is_empty() {
                    "Auto".to_string()
                } else {
                    opts.language.clone()
                },
            );
        if let Ok(r) = c
            .post(format!("{}/profiles", b))
            .multipart(form)
            .send()
            .await
        {
            if let Ok(pj) = r.json::<serde_json::Value>().await {
                profile_id = pj.get("id").and_then(|v| v.as_str()).map(String::from);
            }
        }
    }
    super::report(&progress, "voicestudio", "generate", 0, None, None);
    let mut form = reqwest::multipart::Form::new()
        .text("text", opts.text.clone())
        .text("instruct", instruct.clone())
        .text("stream", "false");
    if !opts.language.is_empty() {
        form = form.text("language", opts.language.clone());
    }
    if let Some(pid) = &profile_id {
        form = form.text("profile_id", pid.clone());
    }
    let resp = c
        .post(format!("{}/generate", b))
        .multipart(form)
        .send()
        .await?;
    let out = wav_from(resp, &out_path(&opts.output_dir, "Voz criada")).await?;
    super::report(&progress, "voicestudio", "done", 1, Some(1), None);
    Ok(DesignResult {
        output: out.to_string_lossy().to_string(),
        instruct,
        matched,
        unmatched,
        profile_id,
    })
}

/// Demucs no VoiceStudio: devolve só a voz. O instrumental é a diferença,
/// feita aqui com o FFmpeg (voz invertida somada ao original).
pub async fn isolate(
    base_url: &str,
    input: &str,
    output_dir: &str,
    instrumental: bool,
    progress: super::ProgressFn,
) -> anyhow::Result<Vec<String>> {
    let b = base(base_url);
    let c = client(1800)?;
    super::report(&progress, "voicestudio", "upload", 0, None, None);
    let data = std::fs::read(input).map_err(|e| anyhow!("entrada: {}", e))?;
    let file_name = Path::new(input)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "audio.wav".into());
    let form = reqwest::multipart::Form::new().part(
        "audio",
        reqwest::multipart::Part::bytes(data).file_name(file_name),
    );
    super::report(&progress, "voicestudio", "separate", 0, None, None);
    let resp = c
        .post(format!("{}/clean-audio", b))
        .multipart(form)
        .send()
        .await?;
    let stem = Path::new(input)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "audio".into());
    let dir = if output_dir.trim().is_empty() {
        Path::new(input)
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default()
    } else {
        PathBuf::from(output_dir.trim())
    };
    std::fs::create_dir_all(&dir)?;
    let vocals = dir.join(format!("{} (voz).wav", stem));
    wav_from(resp, &vocals).await?;
    let mut outputs = vec![vocals.to_string_lossy().to_string()];
    if instrumental {
        super::report(&progress, "voicestudio", "instrumental", 0, None, None);
        let ffmpeg = crate::core::dependencies::ensure_ffmpeg().await?;
        let inst = dir.join(format!("{} (instrumental).wav", stem));
        let o = crate::core::process::command(&ffmpeg)
            .args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
            .arg(input)
            .arg("-i")
            .arg(&vocals)
            .args(["-filter_complex", "[0:a]aformat=sample_fmts=fltp:sample_rates=24000:channel_layouts=mono[a];[1:a]aformat=sample_fmts=fltp:sample_rates=24000:channel_layouts=mono,aeval=-val(0)[b];[a][b]amix=inputs=2:normalize=0[out]", "-map", "[out]"])
            .arg(&inst)
            .output()
            .await?;
        if o.status.success() {
            outputs.push(inst.to_string_lossy().to_string());
        }
    }
    super::report(&progress, "voicestudio", "done", 1, Some(1), None);
    Ok(outputs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bases() {
        assert_eq!(base(""), DEFAULT_BASE);
        assert_eq!(base("http://x:1/"), "http://x:1");
        assert!(out_path("", "Voz").to_string_lossy().ends_with(".wav"));
    }
}
