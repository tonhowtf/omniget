use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Condition {
    HostIs { value: String },
    UrlContains { value: String },
    PlatformIs { value: String },
}

impl Condition {
    pub fn matches(&self, url: &str, platform: Option<&str>) -> bool {
        match self {
            Condition::HostIs { value } => host_of(url)
                .is_some_and(|h| h.eq_ignore_ascii_case(value.trim_start_matches("www."))),
            Condition::UrlContains { value } => url.to_lowercase().contains(&value.to_lowercase()),
            Condition::PlatformIs { value } => {
                platform.is_some_and(|p| p.eq_ignore_ascii_case(value))
            }
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Actions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_only: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitles: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Rule {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub name: String,
    pub when: Condition,
    pub then: Actions,
}

fn default_enabled() -> bool {
    true
}

pub fn host_of(url: &str) -> Option<String> {
    let after_scheme = url.split("://").nth(1).unwrap_or(url);
    let host = after_scheme
        .split(['/', '?', '#'])
        .next()?
        .split('@')
        .next_back()?
        .split(':')
        .next()?;
    if host.is_empty() {
        return None;
    }
    Some(host.trim_start_matches("www.").to_lowercase())
}

pub fn first_match<'a>(rules: &'a [Rule], url: &str, platform: Option<&str>) -> Option<&'a Rule> {
    rules
        .iter()
        .filter(|r| r.enabled)
        .find(|r| r.when.matches(url, platform))
}
