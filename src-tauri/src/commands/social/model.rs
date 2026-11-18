use serde::{Serialize, Serializer};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Platform {
    ShortPost,
    PhotoFeed,
    ClipFeed,
    Forums,
    PinBoard,
    CodeHub,
}

impl Serialize for Platform {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.id())
    }
}

impl Platform {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "x" | "twitter" | "shortpost" => Some(Self::ShortPost),
            "instagram" | "ig" | "photofeed" => Some(Self::PhotoFeed),
            "tiktok" | "tt" | "clipfeed" => Some(Self::ClipFeed),
            "reddit" | "rd" | "forums" => Some(Self::Forums),
            "pinterest" | "pin" | "pinboard" => Some(Self::PinBoard),
            "github" | "gh" | "codehub" => Some(Self::CodeHub),
            _ => None,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::ShortPost => "x",
            Self::PhotoFeed => "instagram",
            Self::ClipFeed => "tiktok",
            Self::Forums => "reddit",
            Self::PinBoard => "pinterest",
            Self::CodeHub => "github",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::ShortPost => "Short Posts",
            Self::PhotoFeed => "Photo Feed",
            Self::ClipFeed => "Clip Feed",
            Self::Forums => "Forums",
            Self::PinBoard => "Pin Boards",
            Self::CodeHub => "Code Hub",
        }
    }

    pub fn origin(self) -> &'static str {
        match self {
            Self::ShortPost => "https://x.com",
            Self::PhotoFeed => "https://www.instagram.com",
            Self::ClipFeed => "https://www.tiktok.com",
            Self::Forums => "https://www.reddit.com",
            Self::PinBoard => "https://www.pinterest.com",
            Self::CodeHub => "https://github.com",
        }
    }

    pub fn login_url(self) -> &'static str {
        match self {
            Self::ShortPost => "https://x.com/i/flow/login",
            Self::PhotoFeed => "https://www.instagram.com/accounts/login/",
            Self::ClipFeed => "https://www.tiktok.com/login",
            Self::Forums => "https://www.reddit.com/login/",
            Self::PinBoard => "https://www.pinterest.com/login/",
            Self::CodeHub => "https://github.com/login",
        }
    }

    pub fn prefix(self) -> &'static str {
        match self {
            Self::ShortPost => "x_",
            Self::PhotoFeed => "ig_",
            Self::ClipFeed => "tt_",
            Self::Forums => "rd_",
            Self::PinBoard => "pin_",
            Self::CodeHub => "gh_",
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

pub fn with_extra<T: Serialize>(item: &T, extra: Value) -> Value {
    let mut v = serde_json::to_value(item).unwrap_or(Value::Null);
    let kept: serde_json::Map<String, Value> = match extra {
        Value::Object(o) => o.into_iter().filter(|(_, x)| !x.is_null()).collect(),
        _ => Default::default(),
    };
    if let Some(o) = v.as_object_mut() {
        if !kept.is_empty() {
            o.insert("extra".into(), Value::Object(kept));
        }
    }
    v
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
