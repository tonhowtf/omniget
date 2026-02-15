use crate::platforms::Platform;

#[derive(Debug, Clone)]
pub struct ParsedUrl {
    pub platform: Platform,
    pub url: String,
    pub content_id: Option<String>,
    pub content_type: ParsedContentType,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParsedContentType {
    Video,
    Audio,
    Image,
    Post,
    Profile,
    Course,
    Playlist,
    Clip,
    Reel,
    Short,
    Unknown,
}

pub fn parse_url(url_str: &str) -> Option<ParsedUrl> {
    let platform = Platform::from_url(url_str)?;
    let parsed = url::Url::parse(url_str).ok()?;
    let path = parsed.path();
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    let (content_id, content_type) = match platform {
        Platform::YouTube => parse_youtube(&parsed, &segments),
        Platform::Instagram => parse_instagram(&segments),
        Platform::TikTok => parse_tiktok(&segments),
        Platform::Twitter => parse_twitter(&segments),
        Platform::Reddit => parse_reddit(&segments),
        Platform::Twitch => parse_twitch(&parsed, &segments),
        Platform::Vimeo => parse_vimeo(&segments),
        Platform::Hotmart => parse_hotmart(&segments),
        Platform::Pinterest => parse_pinterest(&segments),
        Platform::Bluesky => parse_bluesky(&segments),
    };

    Some(ParsedUrl {
        platform,
        url: url_str.to_string(),
        content_id,
        content_type,
    })
}

fn parse_youtube(parsed: &url::Url, segments: &[&str]) -> (Option<String>, ParsedContentType) {
    if let Some(v) = parsed.query_pairs().find(|(k, _)| k == "v").map(|(_, v)| v.to_string()) {
        if parsed.query_pairs().any(|(k, _)| k == "list") {
            return (Some(v), ParsedContentType::Playlist);
        }
        return (Some(v), ParsedContentType::Video);
    }

    if let Some(host) = parsed.host_str() {
        if host.contains("youtu.be") {
            let id = segments.first().map(|s| s.to_string());
            return (id, ParsedContentType::Video);
        }
    }

    if segments.first() == Some(&"shorts") {
        let id = segments.get(1).map(|s| s.to_string());
        return (id, ParsedContentType::Short);
    }

    if segments.first() == Some(&"playlist") {
        let list_id = parsed.query_pairs().find(|(k, _)| k == "list").map(|(_, v)| v.to_string());
        return (list_id, ParsedContentType::Playlist);
    }

    if segments.first() == Some(&"channel") || segments.first() == Some(&"c") || segments.first().map(|s| s.starts_with('@')).unwrap_or(false) {
        let id = segments.first().map(|s| s.to_string());
        return (id, ParsedContentType::Profile);
    }

    (None, ParsedContentType::Unknown)
}

fn parse_instagram(segments: &[&str]) -> (Option<String>, ParsedContentType) {
    match segments.first() {
        Some(&"p") => {
            let id = segments.get(1).map(|s| s.to_string());
            (id, ParsedContentType::Post)
        }
        Some(&"reel") => {
            let id = segments.get(1).map(|s| s.to_string());
            (id, ParsedContentType::Reel)
        }
        Some(&"stories") => {
            let id = segments.get(2).map(|s| s.to_string());
            (id, ParsedContentType::Image)
        }
        Some(username) if !["explore", "accounts", "direct"].contains(username) => {
            (Some(username.to_string()), ParsedContentType::Profile)
        }
        _ => (None, ParsedContentType::Unknown),
    }
}

fn parse_tiktok(segments: &[&str]) -> (Option<String>, ParsedContentType) {
    if let Some(user) = segments.first() {
        if user.starts_with('@') {
            if segments.get(1) == Some(&"video") {
                let id = segments.get(2).map(|s| s.to_string());
                return (id, ParsedContentType::Video);
            }
            return (Some(user.to_string()), ParsedContentType::Profile);
        }
    }

    let id = segments.first().map(|s| s.to_string());
    (id, ParsedContentType::Video)
}

fn parse_twitter(segments: &[&str]) -> (Option<String>, ParsedContentType) {
    if segments.len() >= 3 && segments.get(1) == Some(&"status") {
        let id = segments.get(2).map(|s| s.to_string());
        return (id, ParsedContentType::Post);
    }

    if let Some(user) = segments.first() {
        if !["search", "explore", "settings", "i"].contains(user) {
            return (Some(user.to_string()), ParsedContentType::Profile);
        }
    }

    (None, ParsedContentType::Unknown)
}

fn parse_reddit(segments: &[&str]) -> (Option<String>, ParsedContentType) {
    if segments.len() >= 4 && segments.first() == Some(&"r") && segments.get(2) == Some(&"comments") {
        let id = segments.get(3).map(|s| s.to_string());
        return (id, ParsedContentType::Post);
    }

    if segments.first() == Some(&"comments") {
        let id = segments.get(1).map(|s| s.to_string());
        return (id, ParsedContentType::Post);
    }

    if segments.first() == Some(&"video") {
        let id = segments.get(1).map(|s| s.to_string());
        return (id, ParsedContentType::Video);
    }

    if segments.first() == Some(&"r") {
        if segments.len() >= 4 && segments.get(2) == Some(&"s") {
            let share_id = segments.get(3).map(|s| s.to_string());
            return (share_id, ParsedContentType::Post);
        }
        let sub = segments.get(1).map(|s| s.to_string());
        return (sub, ParsedContentType::Profile);
    }

    let id = segments.first().map(|s| s.to_string());
    (id, ParsedContentType::Video)
}

fn parse_twitch(parsed: &url::Url, segments: &[&str]) -> (Option<String>, ParsedContentType) {
    if segments.first() == Some(&"videos") {
        let id = segments.get(1).map(|s| s.to_string());
        return (id, ParsedContentType::Video);
    }

    if let Some(host) = parsed.host_str() {
        if host.contains("clips.twitch.tv") {
            let id = segments.first().map(|s| s.to_string());
            return (id, ParsedContentType::Clip);
        }
    }

    if segments.len() >= 2 && segments.get(1) == Some(&"clip") {
        let id = segments.get(2).map(|s| s.to_string());
        return (id, ParsedContentType::Clip);
    }

    if let Some(channel) = segments.first() {
        if !["directory", "settings", "downloads"].contains(channel) {
            return (Some(channel.to_string()), ParsedContentType::Profile);
        }
    }

    (None, ParsedContentType::Unknown)
}

fn parse_vimeo(segments: &[&str]) -> (Option<String>, ParsedContentType) {
    if let Some(first) = segments.first() {
        if first.chars().all(|c| c.is_ascii_digit()) {
            return (Some(first.to_string()), ParsedContentType::Video);
        }

        if segments.get(1) == Some(&"videos") {
            return (Some(first.to_string()), ParsedContentType::Profile);
        }

        return (Some(first.to_string()), ParsedContentType::Profile);
    }

    (None, ParsedContentType::Unknown)
}

fn parse_hotmart(segments: &[&str]) -> (Option<String>, ParsedContentType) {
    if segments.contains(&"club") || segments.contains(&"lesson") || segments.contains(&"course") {
        let id = segments.last().map(|s| s.to_string());
        return (id, ParsedContentType::Course);
    }

    (None, ParsedContentType::Unknown)
}

fn parse_pinterest(segments: &[&str]) -> (Option<String>, ParsedContentType) {
    if segments.first() == Some(&"pin") {
        let raw_id = segments.get(1).map(|s| {
            if s.contains("--") {
                s.split("--").last().unwrap_or(s).to_string()
            } else {
                s.to_string()
            }
        });
        return (raw_id, ParsedContentType::Image);
    }

    if segments.first() == Some(&"url_shortener") {
        let code = segments.get(1).map(|s| s.to_string());
        return (code, ParsedContentType::Unknown);
    }

    (None, ParsedContentType::Unknown)
}

fn parse_bluesky(segments: &[&str]) -> (Option<String>, ParsedContentType) {
    if segments.len() >= 4 && segments[0] == "profile" && segments[2] == "post" {
        let post_id = Some(segments[3].to_string());
        return (post_id, ParsedContentType::Post);
    }

    if segments.first() == Some(&"profile") {
        let user = segments.get(1).map(|s| s.to_string());
        return (user, ParsedContentType::Profile);
    }

    (None, ParsedContentType::Unknown)
}
