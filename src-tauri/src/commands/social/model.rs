use serde::Serialize;
use serde_json::Value;

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    X,
    Instagram,
    Tiktok,
}

impl Platform {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "x" | "twitter" => Some(Self::X),
            "instagram" | "ig" => Some(Self::Instagram),
            "tiktok" | "tt" => Some(Self::Tiktok),
            _ => None,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::X => "x",
            Self::Instagram => "instagram",
            Self::Tiktok => "tiktok",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::X => "X",
            Self::Instagram => "Instagram",
            Self::Tiktok => "TikTok",
        }
    }

    pub fn origin(self) -> &'static str {
        match self {
            Self::X => "https://x.com",
            Self::Instagram => "https://www.instagram.com",
            Self::Tiktok => "https://www.tiktok.com",
        }
    }

    pub fn login_url(self) -> &'static str {
        match self {
            Self::X => "https://x.com/i/flow/login",
            Self::Instagram => "https://www.instagram.com/accounts/login/",
            Self::Tiktok => "https://www.tiktok.com/login",
        }
    }

    pub fn prefix(self) -> &'static str {
        match self {
            Self::X => "x_",
            Self::Instagram => "ig_",
            Self::Tiktok => "tt_",
        }
    }
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct Author {
    pub id: String,
    pub handle: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    pub verified: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Image,
    Video,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Media {
    pub kind: MediaKind,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumb: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u64>,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct Stats {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub likes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replies: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reposts: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub views: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub saves: Option<u64>,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct Post {
    pub id: String,
    pub url: String,
    pub author: Author,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<i64>,
    pub media: Vec<Media>,
    pub stats: Stats,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub liked: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reposted: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reposted_by: Option<Author>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quoted: Option<Box<Post>>,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct Profile {
    pub id: String,
    pub handle: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    pub bio: String,
    pub url: String,
    pub verified: bool,
    pub private: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub followers: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub following: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub posts: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub likes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub followed_by_me: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub follows_me: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secondary_id: Option<String>,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct Comment {
    pub id: String,
    pub author: Author,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub likes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replies: Option<u64>,
}

pub fn page<T: Serialize>(items: Vec<T>, cursor: Option<String>) -> Value {
    serde_json::json!({
        "count": items.len(),
        "items": items,
        "next_cursor": cursor.filter(|c| !c.is_empty()),
    })
}

pub fn u(v: &Value) -> Option<u64> {
    match v {
        Value::Number(n) => n.as_u64().or_else(|| n.as_f64().map(|f| f.max(0.0) as u64)),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

pub fn s(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

pub fn opt_s(v: &Value) -> Option<String> {
    Some(s(v)).filter(|x| !x.is_empty())
}

pub fn b(v: &Value) -> bool {
    v.as_bool().unwrap_or(false)
}
