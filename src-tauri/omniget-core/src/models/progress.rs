/// O stream (formato) que o yt-dlp está baixando neste instante.
///
/// Vem de `%(info.*)s` no `--progress-template`: em `bv+ba` cada stream chega
/// com o próprio dict, então dá para dizer "1080p60 · avc1" e depois
/// "áudio · mp4a" sem um segundo processo.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StreamInfo {
    pub format_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fps: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vcodec: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub acodec: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ext: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filesize: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format_note: Option<String>,
}

impl StreamInfo {
    pub fn has_video(&self) -> bool {
        self.vcodec.as_deref().is_some_and(|v| v != "none")
    }

    pub fn has_audio(&self) -> bool {
        self.acodec.as_deref().is_some_and(|a| a != "none")
    }

    /// Nome curto do codec sem o perfil (`avc1.64002a` → `avc1`).
    pub fn short_vcodec(&self) -> Option<String> {
        self.vcodec
            .as_deref()
            .filter(|v| *v != "none")
            .map(|v| v.split('.').next().unwrap_or(v).to_string())
    }

    pub fn short_acodec(&self) -> Option<String> {
        self.acodec
            .as_deref()
            .filter(|a| *a != "none")
            .map(|a| a.split('.').next().unwrap_or(a).to_string())
    }
}

#[derive(Debug, Clone, Default)]
pub struct ProgressUpdate {
    pub percent: f64,
    pub downloaded_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
    pub speed_bps: Option<f64>,
    pub eta_seconds: Option<u64>,
    pub phase: Option<String>,
    /// Stream em download agora (só o yt-dlp preenche).
    pub stream: Option<StreamInfo>,
    pub fragment_index: Option<u32>,
    pub fragment_count: Option<u32>,
    /// Formatos que o yt-dlp anunciou que vai baixar (`Downloading 2 format(s): 299+140`).
    pub planned_formats: Option<Vec<String>>,
}

impl ProgressUpdate {
    pub fn percent(percent: f64) -> Self {
        Self {
            percent,
            ..Default::default()
        }
    }

    pub fn phase(phase: &str, percent: f64) -> Self {
        Self {
            percent,
            phase: Some(phase.to_string()),
            ..Default::default()
        }
    }

    pub fn rich(
        percent: f64,
        downloaded_bytes: Option<u64>,
        total_bytes: Option<u64>,
        speed_bps: Option<f64>,
        eta_seconds: Option<u64>,
    ) -> Self {
        Self {
            percent,
            downloaded_bytes,
            total_bytes,
            speed_bps,
            eta_seconds,
            ..Default::default()
        }
    }

    pub fn has_real_metrics(&self) -> bool {
        self.downloaded_bytes.is_some() || self.speed_bps.is_some() || self.eta_seconds.is_some()
    }

    /// Muda o *estado* mostrado (fase, stream, plano de formatos, fragmento),
    /// não só o número. Nunca pode cair no throttle de 250 ms: perder um
    /// "trocou para o áudio" deixa a tela mentindo até o próximo tick.
    pub fn is_structural(&self) -> bool {
        self.phase.is_some()
            || self.stream.is_some()
            || self.planned_formats.is_some()
            || self.fragment_index.is_some()
    }
}

impl From<f64> for ProgressUpdate {
    fn from(percent: f64) -> Self {
        Self::percent(percent)
    }
}
