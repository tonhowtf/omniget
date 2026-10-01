use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const PREFS_FILE: &str = "limits-strip.json";
pub const DIR_ENV: &str = "OMNIGET_LIMITS_DIR";

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default, PartialOrd, Ord,
)]
#[serde(rename_all = "lowercase")]
pub enum Edge {
    #[default]
    Top,
    Bottom,
    Left,
    Right,
}

impl Edge {
    pub fn as_str(self) -> &'static str {
        match self {
            Edge::Top => "top",
            Edge::Bottom => "bottom",
            Edge::Left => "left",
            Edge::Right => "right",
        }
    }

    pub fn parse(raw: &str) -> Option<Edge> {
        match raw {
            "top" => Some(Edge::Top),
            "bottom" => Some(Edge::Bottom),
            "left" => Some(Edge::Left),
            "right" => Some(Edge::Right),
            _ => None,
        }
    }

    pub fn is_vertical(self) -> bool {
        matches!(self, Edge::Left | Edge::Right)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderPref {
    pub id: String,
    pub enabled: bool,
    pub muted: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StripPrefs {
    pub enabled: bool,
    pub edge: Edge,
    pub along: BTreeMap<Edge, f64>,
    pub providers: Vec<ProviderPref>,
    pub notify_thresholds: bool,
    pub thresholds: Vec<u8>,
    pub notify_reset: bool,
    pub notify_pace: bool,
    pub size: Size,
    pub show_percent: bool,
    pub show_pace: bool,
    pub contrast: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Size {
    S,
    #[default]
    M,
    L,
}

impl Size {
    pub fn scale(self) -> f64 {
        match self {
            Size::S => 0.85,
            Size::M => 1.0,
            Size::L => 1.25,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct Look {
    pub size: Size,
    pub scale: f64,
    pub show_percent: bool,
    pub show_pace: bool,
    pub contrast: bool,
}

impl Default for StripPrefs {
    fn default() -> Self {
        Self {
            enabled: false,
            edge: Edge::Top,
            along: BTreeMap::new(),
            providers: Vec::new(),
            notify_thresholds: true,
            thresholds: vec![80, 95],
            notify_reset: true,
            notify_pace: true,
            size: Size::M,
            show_percent: true,
            show_pace: true,
            contrast: false,
        }
    }
}

impl StripPrefs {
    pub fn look(&self) -> Look {
        Look {
            size: self.size,
            scale: self.size.scale(),
            show_percent: self.show_percent,
            show_pace: self.show_pace,
            contrast: self.contrast,
        }
    }

    pub fn along_of(&self, edge: Edge) -> f64 {
        self.along
            .get(&edge)
            .copied()
            .unwrap_or(0.5)
            .clamp(0.0, 1.0)
    }

    pub fn provider(&self, id: &str) -> Option<&ProviderPref> {
        self.providers.iter().find(|p| p.id == id)
    }

    pub fn is_on(&self, id: &str) -> bool {
        self.enabled && self.provider(id).map(|p| p.enabled).unwrap_or(false)
    }

    pub fn is_muted(&self, id: &str) -> bool {
        self.provider(id).map(|p| p.muted).unwrap_or(false)
    }

    pub fn adopt<S: AsRef<str>>(&mut self, known: &[(S, bool)]) -> bool {
        let mut changed = false;
        for (id, on) in known {
            let id = id.as_ref();
            if self.provider(id).is_none() {
                self.providers.push(ProviderPref {
                    id: id.to_string(),
                    enabled: *on,
                    muted: false,
                });
                changed = true;
            }
        }
        let before = self.providers.len();
        self.providers
            .retain(|p| known.iter().any(|(id, _)| id.as_ref() == p.id));
        changed || self.providers.len() != before
    }

    pub fn sane_thresholds(&self) -> Vec<u8> {
        let mut t: Vec<u8> = self
            .thresholds
            .iter()
            .copied()
            .filter(|v| (1..=100).contains(v))
            .collect();
        t.sort_unstable();
        t.dedup();
        t.truncate(4);
        t
    }
}

pub fn prefs_from_str(raw: &str) -> StripPrefs {
    serde_json::from_str(raw).unwrap_or_default()
}

pub fn dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os(DIR_ENV) {
        if !dir.is_empty() {
            return Some(PathBuf::from(dir));
        }
    }
    crate::core::paths::app_data_dir().map(|d| d.join("llm").join("limits"))
}

pub fn load() -> StripPrefs {
    let Some(dir) = dir() else {
        return StripPrefs::default();
    };
    match std::fs::read_to_string(dir.join(PREFS_FILE)) {
        Ok(raw) => prefs_from_str(&raw),
        Err(_) => StripPrefs::default(),
    }
}

pub fn save(prefs: &StripPrefs) -> Result<(), String> {
    let dir = dir().ok_or("ERR_LIMITS_STORE: no data directory")?;
    save_in(&dir, prefs)
}

pub fn save_in(dir: &std::path::Path, prefs: &StripPrefs) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("ERR_LIMITS_STORE: {e}"))?;
    let body = serde_json::to_vec_pretty(prefs).map_err(|e| format!("ERR_LIMITS_STORE: {e}"))?;
    let tmp = dir.join(format!("{PREFS_FILE}.tmp"));
    std::fs::write(&tmp, &body).map_err(|e| format!("ERR_LIMITS_STORE: {e}"))?;
    std::fs::rename(&tmp, dir.join(PREFS_FILE)).map_err(|e| format!("ERR_LIMITS_STORE: {e}"))
}
