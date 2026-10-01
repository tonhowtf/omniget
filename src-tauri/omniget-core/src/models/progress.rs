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

    pub fn short_vcodec(&self) -> Option<String> {
        self.vcodec
            .as_deref()
            .filter(|v| *v != "none")
            .map(|v| v.split('.').next().unwrap_or(v).to_string())
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
    pub stream: Option<StreamInfo>,
    pub fragment_index: Option<u32>,
    pub fragment_count: Option<u32>,
    pub planned_formats: Option<Vec<String>>,
    pub indeterminate: bool,
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

    pub fn indeterminate(mut self) -> Self {
        self.indeterminate = true;
        self
    }

    pub fn percent_value(&self) -> Option<f64> {
        (!self.indeterminate).then_some(self.percent)
    }

    pub fn has_real_metrics(&self) -> bool {
        self.downloaded_bytes.is_some() || self.speed_bps.is_some() || self.eta_seconds.is_some()
    }

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
