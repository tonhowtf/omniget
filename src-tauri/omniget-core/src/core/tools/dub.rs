//! Dublagem a partir de uma legenda (estudo 06, planner do KrillinAI em
//! versão curta): cada cue vira um MP3 do Edge TTS, o áudio é acelerado até
//! caber no slot (`atempo` ≤ 1,3) e tudo é montado com um único `ffmpeg`
//! (`adelay` + `amix`). Com vídeo de entrada, a faixa nova substitui a
//! original (`-map 0:v -map 1:a`).

use std::path::{Path, PathBuf};

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

use super::edge_tts::{self, TtsOptions};
use crate::core::subtitle_merge::{parse_cues, Cue};

#[derive(Debug, Clone, Deserialize)]
pub struct DubOptions {
    pub srt_path: String,
    #[serde(default)]
    pub video_path: String,
    pub voice: String,
    #[serde(default = "default_rate")]
    pub rate: String,
    /// Aceleração máxima antes de deixar a fala invadir o próximo cue.
    #[serde(default = "default_speed")]
    pub max_speed: f64,
    #[serde(default)]
    pub output_dir: String,
    /// Volume da trilha original mantida por baixo (0 = remove).
    #[serde(default)]
    pub keep_original_volume: f64,
}

fn default_rate() -> String {
    "+0%".into()
}
fn default_speed() -> f64 {
    1.3
}

#[derive(Debug, Clone, Serialize)]
pub struct DubResult {
    pub audio_path: String,
    pub video_path: Option<String>,
    pub cues: usize,
    pub sped_up: usize,
}

/// `atempo` só aceita 0,5–2,0 por filtro; encadeia como o KrillinAI/pyvideotrans.
pub fn atempo_chain(speed: f64) -> String {
    let mut parts = Vec::new();
    let mut s = speed;
    while s > 2.0 {
        parts.push("atempo=2.0".to_string());
        s /= 2.0;
    }
    while s < 0.5 {
        parts.push("atempo=0.5".to_string());
        s /= 0.5;
    }
    parts.push(format!("atempo={:.4}", s));
    parts.join(",")
}

pub async fn dub(opts: DubOptions, progress: super::ProgressFn) -> anyhow::Result<DubResult> {
    let id = "dub";
    let srt = tokio::fs::read_to_string(&opts.srt_path).await?;
    let cues: Vec<Cue> = parse_cues(&srt).into_iter().filter(|c| !c.text.trim().is_empty()).collect();
    if cues.is_empty() {
        return Err(anyhow!("a legenda nao tem falas"));
    }
    let ffmpeg = crate::core::dependencies::ensure_ffmpeg().await?;
    let work = super::temp_dir().join(format!("dub-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&work)?;
    let out_dir = if opts.output_dir.trim().is_empty() {
        Path::new(&opts.srt_path).parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."))
    } else {
        PathBuf::from(opts.output_dir.trim())
    };
    std::fs::create_dir_all(&out_dir)?;
    let stem = Path::new(&opts.srt_path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "dublagem".into());

    // 1) sintetiza cada cue
    let mut pieces: Vec<(PathBuf, u64, u64)> = Vec::new(); // (mp3, start_ms, slot_ms)
    let total = cues.len() as u64;
    for (i, cue) in cues.iter().enumerate() {
        super::report(&progress, id, "synthesize", i as u64, Some(total), Some(cue.text.clone()));
        let mp3 = work.join(format!("{:05}.mp3", i));
        let tts = TtsOptions {
            text: cue.text.replace('\n', " "),
            voice: opts.voice.clone(),
            rate: opts.rate.clone(),
            pitch: "+0Hz".into(),
            volume: "+0%".into(),
        };
        edge_tts::synthesize(tts, &mp3, super::noop_progress()).await?;
        let slot_end = cues.get(i + 1).map(|n| n.start_ms).unwrap_or(cue.end_ms).max(cue.end_ms);
        pieces.push((mp3, cue.start_ms, slot_end.saturating_sub(cue.start_ms).max(300)));
    }

    // 2) mede duração real e decide a velocidade
    let mut sped_up = 0usize;
    let mut speeds = Vec::with_capacity(pieces.len());
    for (mp3, _, slot) in &pieces {
        let dur_ms = crate::core::ffmpeg::get_duration_us(mp3).await.unwrap_or(0) / 1000;
        let speed = if dur_ms > *slot {
            sped_up += 1;
            (dur_ms as f64 / *slot as f64).min(opts.max_speed.max(1.0))
        } else {
            1.0
        };
        speeds.push(speed);
    }

    // 3) monta em grupos de 120 entradas (limite de linha de comando)
    super::report(&progress, id, "mix", 0, Some(1), None);
    let mut group_files = Vec::new();
    for (gi, group) in pieces.chunks(120).enumerate() {
        let out = work.join(format!("group-{:03}.wav", gi));
        let mut cmd = crate::core::process::command(&ffmpeg);
        cmd.args(["-y", "-hide_banner", "-loglevel", "error"]);
        let mut filter = String::new();
        let mut labels = String::new();
        for (j, (mp3, start, _)) in group.iter().enumerate() {
            cmd.arg("-i").arg(mp3);
            let speed = speeds[gi * 120 + j];
            filter.push_str(&format!(
                "[{j}:a]{tempo},adelay={d}|{d},aresample=48000[a{j}];",
                tempo = atempo_chain(speed),
                d = start
            ));
            labels.push_str(&format!("[a{j}]"));
        }
        filter.push_str(&format!(
            "{}amix=inputs={}:dropout_transition=0:normalize=0[out]",
            labels,
            group.len()
        ));
        cmd.args(["-filter_complex", &filter, "-map", "[out]", "-c:a", "pcm_s16le"]).arg(&out);
        let output = cmd.output().await?;
        if !output.status.success() {
            return Err(anyhow!("ffmpeg (mix) falhou: {}", String::from_utf8_lossy(&output.stderr).trim()));
        }
        group_files.push(out);
    }
    let dub_audio = out_dir.join(format!("{}.dub.m4a", stem));
    {
        let mut cmd = crate::core::process::command(&ffmpeg);
        cmd.args(["-y", "-hide_banner", "-loglevel", "error"]);
        for g in &group_files {
            cmd.arg("-i").arg(g);
        }
        let mut filter = String::new();
        for j in 0..group_files.len() {
            filter.push_str(&format!("[{j}:a]"));
        }
        filter.push_str(&format!("amix=inputs={}:dropout_transition=0:normalize=0[out]", group_files.len()));
        cmd.args(["-filter_complex", &filter, "-map", "[out]", "-c:a", "aac", "-b:a", "160k"]).arg(&dub_audio);
        let output = cmd.output().await?;
        if !output.status.success() {
            return Err(anyhow!("ffmpeg (final) falhou: {}", String::from_utf8_lossy(&output.stderr).trim()));
        }
    }

    // 4) opcional: troca o áudio do vídeo
    let mut video_out = None;
    if !opts.video_path.trim().is_empty() {
        super::report(&progress, id, "mux", 0, Some(1), None);
        let video = Path::new(opts.video_path.trim());
        let ext = video.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_else(|| "mp4".into());
        let out = out_dir.join(format!("{}.dub.{}", stem, ext));
        let mut cmd = crate::core::process::command(&ffmpeg);
        cmd.args(["-y", "-hide_banner", "-loglevel", "error", "-i"]).arg(video).arg("-i").arg(&dub_audio);
        if opts.keep_original_volume > 0.0 {
            let f = format!(
                "[0:a]volume={:.2}[o];[1:a]volume=1.0[d];[o][d]amix=inputs=2:dropout_transition=0:normalize=0[a]",
                opts.keep_original_volume.min(1.0)
            );
            cmd.args(["-filter_complex", &f, "-map", "0:v", "-map", "[a]"]);
        } else {
            cmd.args(["-map", "0:v", "-map", "1:a"]);
        }
        cmd.args(["-c:v", "copy", "-c:a", "aac", "-shortest"]).arg(&out);
        let output = cmd.output().await?;
        if !output.status.success() {
            return Err(anyhow!("ffmpeg (mux) falhou: {}", String::from_utf8_lossy(&output.stderr).trim()));
        }
        video_out = Some(out.to_string_lossy().to_string());
    }
    let _ = std::fs::remove_dir_all(&work);
    super::report(&progress, id, "done", 1, Some(1), None);
    Ok(DubResult {
        audio_path: dub_audio.to_string_lossy().to_string(),
        video_path: video_out,
        cues: cues.len(),
        sped_up,
    })
}

#[cfg(test)]
mod tests {
    use super::atempo_chain;

    #[test]
    fn chains_large_speeds() {
        assert_eq!(atempo_chain(1.2), "atempo=1.2000");
        assert_eq!(atempo_chain(3.0), "atempo=2.0,atempo=1.5000");
        assert_eq!(atempo_chain(0.4), "atempo=0.5,atempo=0.8000");
    }
}
