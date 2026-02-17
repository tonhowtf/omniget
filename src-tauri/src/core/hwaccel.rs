use serde::{Deserialize, Serialize};
use tokio::sync::OnceCell;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HwAccelInfo {
    pub encoders: Vec<String>,
    pub decoders: Vec<String>,
    pub recommended_video_encoder: Option<String>,
    pub recommended_decoder: Option<String>,
}

static HW_ACCEL_CACHE: OnceCell<HwAccelInfo> = OnceCell::const_new();

const GPU_ENCODER_PRIORITY: &[&str] = &[
    "h264_nvenc",
    "h264_qsv",
    "h264_amf",
    "h264_videotoolbox",
    "h264_vaapi",
];

const GPU_DECODER_PRIORITY: &[&str] = &[
    "h264_cuvid",
    "h264_qsv",
    "h264_videotoolbox",
    "h264_vaapi",
];

pub async fn detect_hwaccel() -> HwAccelInfo {
    HW_ACCEL_CACHE
        .get_or_init(|| async { detect_hwaccel_inner().await })
        .await
        .clone()
}

async fn detect_hwaccel_inner() -> HwAccelInfo {
    let encoders = query_codecs("encoders").await;
    let decoders = query_codecs("decoders").await;

    let recommended_video_encoder = GPU_ENCODER_PRIORITY
        .iter()
        .find(|c| encoders.iter().any(|e| e == *c))
        .map(|s| s.to_string());

    let recommended_decoder = GPU_DECODER_PRIORITY
        .iter()
        .find(|c| decoders.iter().any(|d| d == *c))
        .map(|s| s.to_string());

    HwAccelInfo {
        encoders,
        decoders,
        recommended_video_encoder,
        recommended_decoder,
    }
}

async fn query_codecs(flag: &str) -> Vec<String> {
    let output = tokio::process::Command::new("ffmpeg")
        .args([&format!("-{}", flag), "-hide_banner"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await;

    let output = match output {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };

    let text = String::from_utf8_lossy(&output.stdout);
    let gpu_keywords = [
        "nvenc", "qsv", "amf", "videotoolbox", "vaapi", "cuvid", "cuda",
    ];

    text.lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            if parts.len() >= 2 {
                let codec_name = parts[1];
                if gpu_keywords.iter().any(|kw| codec_name.contains(kw)) {
                    return Some(codec_name.to_string());
                }
            }
            None
        })
        .collect()
}
