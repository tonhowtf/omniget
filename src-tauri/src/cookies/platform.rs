//! Map an HTTP domain to a `platform_kind` used by the UI and by plugin
//! consumers. The mapping is deterministic and intentionally narrow — only
//! platforms with first-class support get a dedicated kind; everything else
//! falls back to `generic` and is identified by domain alone.
//!
//! Kept separate from `storage.rs` so the platform list can evolve without
//! touching the on-disk format.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlatformKind {
    Youtube,
    YoutubeMusic,
    SoundCloud,
    Spotify,
    Twitch,
    Instagram,
    XTwitter,
    Vimeo,
    Tiktok,
    Bilibili,
    Douyin,
    Reddit,
    Pinterest,
    Bluesky,
    Generic,
}

impl PlatformKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            PlatformKind::Youtube => "youtube",
            PlatformKind::YoutubeMusic => "youtube_music",
            PlatformKind::SoundCloud => "soundcloud",
            PlatformKind::Spotify => "spotify",
            PlatformKind::Twitch => "twitch",
            PlatformKind::Instagram => "instagram",
            PlatformKind::XTwitter => "x_twitter",
            PlatformKind::Vimeo => "vimeo",
            PlatformKind::Tiktok => "tiktok",
            PlatformKind::Bilibili => "bilibili",
            PlatformKind::Douyin => "douyin",
            PlatformKind::Reddit => "reddit",
            PlatformKind::Pinterest => "pinterest",
            PlatformKind::Bluesky => "bluesky",
            PlatformKind::Generic => "generic",
        }
    }

    pub fn from_domain(domain: &str) -> PlatformKind {
        let host = domain.trim_start_matches('.').to_lowercase();
        if host == "music.youtube.com" {
            return PlatformKind::YoutubeMusic;
        }
        let root = root_domain_of(&host);
        match root.as_str() {
            "youtube.com" | "youtu.be" => PlatformKind::Youtube,
            "soundcloud.com" => PlatformKind::SoundCloud,
            "spotify.com" => PlatformKind::Spotify,
            "twitch.tv" => PlatformKind::Twitch,
            "instagram.com" => PlatformKind::Instagram,
            "x.com" | "twitter.com" => PlatformKind::XTwitter,
            "vimeo.com" => PlatformKind::Vimeo,
            "tiktok.com" => PlatformKind::Tiktok,
            "bilibili.com" => PlatformKind::Bilibili,
            "douyin.com" | "iesdouyin.com" => PlatformKind::Douyin,
            "reddit.com" => PlatformKind::Reddit,
            "pinterest.com" => PlatformKind::Pinterest,
            "bsky.app" | "bsky.social" => PlatformKind::Bluesky,
            _ => PlatformKind::Generic,
        }
    }
}

/// Second-level public suffixes where the registrable name is three labels
/// long (`app.rocketseat.com.br` -> `rocketseat.com.br`). Without this the
/// Cookie Manager filed every Brazilian site under one `com.br` bucket.
const TWO_LABEL_PUBLIC_SUFFIXES: &[&str] = &[
    "com.br", "net.br", "org.br", "edu.br", "gov.br", "art.br", "eng.br",
    "co.uk", "org.uk", "ac.uk", "gov.uk", "me.uk",
    "com.au", "net.au", "org.au", "edu.au",
    "co.jp", "ne.jp", "or.jp", "ac.jp",
    "com.mx", "org.mx", "com.ar", "com.co", "com.pe", "com.cl", "com.ve", "com.uy",
    "co.za", "co.nz", "co.in", "co.kr", "co.id", "com.sg", "com.hk", "com.tw", "com.my",
    "com.tr", "com.pt", "com.es", "com.pl", "com.ua", "com.ru", "com.cn", "com.ph",
];

pub fn root_domain_of(host: &str) -> String {
    let h = host.trim_start_matches('.').to_lowercase();
    let parts: Vec<&str> = h.split('.').collect();
    if parts.len() < 2 {
        return h;
    }
    let last_two = parts[parts.len() - 2..].join(".");
    if parts.len() >= 3 && TWO_LABEL_PUBLIC_SUFFIXES.contains(&last_two.as_str()) {
        return parts[parts.len() - 3..].join(".");
    }
    last_two
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn youtube_variants_map_correctly() {
        assert_eq!(
            PlatformKind::from_domain("youtube.com"),
            PlatformKind::Youtube
        );
        assert_eq!(
            PlatformKind::from_domain(".youtube.com"),
            PlatformKind::Youtube
        );
        assert_eq!(
            PlatformKind::from_domain("www.youtube.com"),
            PlatformKind::Youtube
        );
        assert_eq!(
            PlatformKind::from_domain("music.youtube.com"),
            PlatformKind::YoutubeMusic
        );
        assert_eq!(PlatformKind::from_domain("youtu.be"), PlatformKind::Youtube);
    }

    #[test]
    fn twitter_x_share_kind() {
        assert_eq!(PlatformKind::from_domain("x.com"), PlatformKind::XTwitter);
        assert_eq!(
            PlatformKind::from_domain("twitter.com"),
            PlatformKind::XTwitter
        );
    }

    #[test]
    fn unknown_falls_to_generic() {
        assert_eq!(
            PlatformKind::from_domain("example.com"),
            PlatformKind::Generic
        );
        assert_eq!(
            PlatformKind::from_domain("bandcamp.com"),
            PlatformKind::Generic
        );
    }

    #[test]
    fn root_strips_subdomain() {
        assert_eq!(root_domain_of("music.youtube.com"), "youtube.com");
        assert_eq!(root_domain_of("app.rocketseat.com.br"), "rocketseat.com.br");
        assert_eq!(root_domain_of(".rocketseat.com.br"), "rocketseat.com.br");
        assert_eq!(root_domain_of("com.br"), "com.br");
        assert_eq!(root_domain_of("www.bbc.co.uk"), "bbc.co.uk");
        assert_eq!(root_domain_of("hotmart.com"), "hotmart.com");
        assert_eq!(root_domain_of("api.soundcloud.com"), "soundcloud.com");
        assert_eq!(root_domain_of("localhost"), "localhost");
    }

    #[test]
    fn root_handles_leading_dot() {
        assert_eq!(root_domain_of(".youtube.com"), "youtube.com");
    }
}
