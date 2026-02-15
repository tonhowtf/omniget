pub mod traits;
pub mod hotmart;
pub mod pinterest;
pub mod tiktok;
pub mod twitter;
pub mod twitch;
pub mod bluesky;
pub mod reddit;
pub mod vimeo;

use std::fmt;
use std::str::FromStr;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Hotmart,
    YouTube,
    Instagram,
    TikTok,
    Twitter,
    Reddit,
    Facebook,
    Twitch,
    Vimeo,
    Pinterest,
    Bluesky,
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Platform::Hotmart => "hotmart",
            Platform::YouTube => "youtube",
            Platform::Instagram => "instagram",
            Platform::TikTok => "tiktok",
            Platform::Twitter => "twitter",
            Platform::Reddit => "reddit",
            Platform::Facebook => "facebook",
            Platform::Twitch => "twitch",
            Platform::Vimeo => "vimeo",
            Platform::Pinterest => "pinterest",
            Platform::Bluesky => "bluesky",
        };
        write!(f, "{}", name)
    }
}

impl FromStr for Platform {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "hotmart" => Ok(Platform::Hotmart),
            "youtube" | "yt" => Ok(Platform::YouTube),
            "instagram" | "ig" => Ok(Platform::Instagram),
            "tiktok" | "tt" => Ok(Platform::TikTok),
            "twitter" | "x" => Ok(Platform::Twitter),
            "reddit" => Ok(Platform::Reddit),
            "facebook" | "fb" => Ok(Platform::Facebook),
            "twitch" => Ok(Platform::Twitch),
            "vimeo" => Ok(Platform::Vimeo),
            "pinterest" => Ok(Platform::Pinterest),
            "bluesky" | "bsky" => Ok(Platform::Bluesky),
            _ => Err(format!("Unknown platform: {}", s)),
        }
    }
}

impl Platform {
    pub fn from_url(url_str: &str) -> Option<Self> {
        let parsed = url::Url::parse(url_str).ok()?;
        let host = parsed.host_str()?.to_lowercase();

        let matches = |domain: &str| -> bool {
            host == domain || host.ends_with(&format!(".{}", domain))
        };

        if matches("hotmart.com") {
            Some(Platform::Hotmart)
        } else if matches("youtube.com") || matches("youtu.be") {
            Some(Platform::YouTube)
        } else if matches("instagram.com") {
            Some(Platform::Instagram)
        } else if matches("tiktok.com") {
            Some(Platform::TikTok)
        } else if matches("twitter.com") || matches("x.com") || matches("vxtwitter.com") || matches("fixvx.com") {
            Some(Platform::Twitter)
        } else if matches("reddit.com") || host == "v.redd.it" || host == "redd.it" {
            Some(Platform::Reddit)
        } else if matches("facebook.com") || matches("fb.watch") || matches("fb.com") {
            Some(Platform::Facebook)
        } else if matches("twitch.tv") {
            Some(Platform::Twitch)
        } else if matches("vimeo.com") {
            Some(Platform::Vimeo)
        } else if host == "pin.it" || host.contains("pinterest.") {
            Some(Platform::Pinterest)
        } else if host == "bsky.app" || host.ends_with(".bsky.app") {
            Some(Platform::Bluesky)
        } else {
            None
        }
    }

    pub fn all() -> &'static [Platform] {
        &[
            Platform::Hotmart,
            Platform::YouTube,
            Platform::Instagram,
            Platform::TikTok,
            Platform::Twitter,
            Platform::Reddit,
            Platform::Facebook,
            Platform::Twitch,
            Platform::Vimeo,
            Platform::Pinterest,
            Platform::Bluesky,
        ]
    }
}
