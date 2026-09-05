//! Cliente da API web do Pinterest. Testado em 2026-09-05 sem login: pin,
//! board, seções, feed do board, perfil, boards do perfil, pins salvos e
//! criados, busca (pins/vídeos/boards), relacionados, typeahead e `pin.it`.

use std::time::Duration;

use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const ROOT: &str = "https://www.pinterest.com";
const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";
/// Página máxima aceita pelo `BoardFeedResource` (yt-dlp usa 250).
pub const PAGE: u32 = 100;

// ───────────────────────── alvos (URLs) ─────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Target {
    Pin {
        id: String,
    },
    Board {
        user: String,
        slug: String,
    },
    Section {
        user: String,
        slug: String,
        section: String,
    },
    User {
        username: String,
    },
    UserCreated {
        username: String,
    },
    Search {
        query: String,
        scope: String,
    },
    Short {
        code: String,
    },
}

const RESERVED: &[&str] = &[
    "pin",
    "search",
    "ideas",
    "today",
    "settings",
    "login",
    "signup",
    "business",
    "explore",
    "categories",
    "topics",
    "news_hub",
    "videos",
    "shopping",
    "help",
    "about",
    "policy",
    "resource",
    "oauth",
    "password",
    "notifications",
    "messages",
    "homefeed",
    "create",
    "pin-builder",
    "pin-creation-tool",
    "board",
    "boards",
    "_created",
    "_saved",
];

fn pinterest_host(host: &str) -> bool {
    let h = host.to_ascii_lowercase();
    h == "pinterest.com" || h.ends_with(".pinterest.com") || {
        // pinterest.fr, pinterest.co.uk, br.pinterest.com …
        let mut parts = h.rsplitn(4, '.').collect::<Vec<_>>();
        parts.reverse();
        h.contains("pinterest.") && parts.contains(&"pinterest")
    }
}

/// Reconhece pin, board, seção, perfil, busca e `pin.it`. Texto sem URL vira
/// busca de pins; só dígitos vira id de pin.
pub fn parse_target(input: &str) -> Option<Target> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }
    if s.chars().all(|c| c.is_ascii_digit()) && s.len() >= 8 {
        return Some(Target::Pin { id: s.to_string() });
    }
    let with_scheme = if s.contains("://") {
        s.to_string()
    } else {
        format!("https://{}", s)
    };
    let Ok(u) = url::Url::parse(&with_scheme) else {
        return Some(Target::Search {
            query: s.to_string(),
            scope: "pins".into(),
        });
    };
    let host = u.host_str().unwrap_or("").to_ascii_lowercase();
    if host == "pin.it" {
        let code = u.path().trim_matches('/').to_string();
        return if code.is_empty() {
            None
        } else {
            Some(Target::Short { code })
        };
    }
    if !pinterest_host(&host) {
        // não é URL do Pinterest: trata como texto de busca
        if !s.contains('.') || s.contains(' ') {
            return Some(Target::Search {
                query: s.to_string(),
                scope: "pins".into(),
            });
        }
        return None;
    }
    let segs: Vec<String> = u
        .path_segments()
        .map(|it| {
            it.filter(|p| !p.is_empty())
                .map(|p| p.to_string())
                .collect()
        })
        .unwrap_or_default();
    match segs.as_slice() {
        [] => None,
        [first, rest @ ..] if first == "pin" => {
            let raw = rest.first()?;
            let id = raw.rsplit("--").next().unwrap_or(raw);
            let id: String = id.chars().filter(|c| c.is_ascii_digit()).collect();
            if id.is_empty() {
                None
            } else {
                Some(Target::Pin { id })
            }
        }
        [first, scope, ..] if first == "search" => {
            let query = u
                .query_pairs()
                .find(|(k, _)| k == "q")
                .map(|(_, v)| v.to_string())
                .unwrap_or_default();
            let scope = match scope.as_str() {
                "videos" | "boards" => scope.to_string(),
                _ => "pins".to_string(),
            };
            Some(Target::Search { query, scope })
        }
        [user] if !RESERVED.contains(&user.as_str()) => Some(Target::User {
            username: user.clone(),
        }),
        [user, tab] if tab == "_created" => Some(Target::UserCreated {
            username: user.clone(),
        }),
        [user, tab] if tab == "_saved" || tab == "pins" => Some(Target::User {
            username: user.clone(),
        }),
        [user, slug] if !RESERVED.contains(&user.as_str()) => Some(Target::Board {
            user: user.clone(),
            slug: slug.clone(),
        }),
        [user, slug, section, ..] if !RESERVED.contains(&user.as_str()) => Some(Target::Section {
            user: user.clone(),
            slug: slug.clone(),
            section: section.clone(),
        }),
        _ => None,
    }
}

// ───────────────────────── modelo ─────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Media {
    pub url: String,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Video {
    /// playlist HLS (V_HLSV4), sempre presente quando é vídeo
    pub hls: Option<String>,
    /// MP4 direto quando o Pinterest expõe (720p / expMp4)
    pub mp4: Option<String>,
    pub width: u32,
    pub height: u32,
    pub duration_ms: u64,
    pub thumbnail: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Person {
    pub id: Option<String>,
    pub username: Option<String>,
    pub name: Option<String>,
    pub avatar: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BoardRef {
    pub id: Option<String>,
    pub name: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Rich {
    pub site_name: Option<String>,
    pub title: Option<String>,
    pub url: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Attribution {
    pub author_name: Option<String>,
    pub author_url: Option<String>,
    pub provider_name: Option<String>,
    pub title: Option<String>,
    pub url: Option<String>,
}

/// Item extra de carrossel ou página de story pin.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Extra {
    pub index: usize,
    /// "image" | "video"
    pub kind: String,
    pub image: Option<Media>,
    pub video: Option<Video>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AiInfo {
    /// o próprio Pinterest rotulou (ai_disclosures / gen_ai_topics)
    pub labeled: bool,
    pub topics: Vec<String>,
    /// 0 nada · 1 sinal fraco (só "ai") · 2 ferramenta citada (midjourney…) · 3 frase explícita ("ai generated")
    pub keyword_level: u8,
    pub keyword: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Pin {
    pub id: String,
    pub url: String,
    pub title: String,
    pub description: String,
    pub alt_text: String,
    /// link de destino ("Visit site")
    pub link: Option<String>,
    pub domain: Option<String>,
    pub created_at: Option<String>,
    /// "image" | "gif" | "video" | "carousel" | "story"
    pub kind: String,
    pub image_signature: Option<String>,
    /// original (`/originals/`), quando a API entrega
    pub image: Option<Media>,
    /// 736x, sempre existe para pins com imagem
    pub image_large: Option<Media>,
    /// 236x
    pub thumb: Option<String>,
    pub video: Option<Video>,
    pub extras: Vec<Extra>,
    pub pinner: Option<Person>,
    pub creator: Option<Person>,
    pub board: Option<BoardRef>,
    pub saves: u64,
    pub repins: u64,
    pub comments: u64,
    pub reactions: u64,
    pub is_promoted: bool,
    pub is_repin: bool,
    pub ai: AiInfo,
    pub dominant_color: Option<String>,
    pub rich: Option<Rich>,
    pub attribution: Option<Attribution>,
    /// seção do board de onde veio (preenchido pelo backup)
    #[serde(default)]
    pub section: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Board {
    pub id: String,
    pub name: String,
    pub url: String,
    pub description: String,
    pub pin_count: u64,
    pub section_count: u64,
    pub follower_count: u64,
    pub privacy: String,
    pub cover: Option<String>,
    pub owner: Option<Person>,
    pub is_collaborative: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Section {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub pin_count: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub username: String,
    pub name: String,
    pub about: String,
    pub website: Option<String>,
    pub avatar: Option<String>,
    pub pin_count: u64,
    pub board_count: u64,
    pub follower_count: u64,
    pub following_count: u64,
    pub is_verified_merchant: bool,
    pub is_private: bool,
}

/// Uma lista paginada de pins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Feed {
    Board {
        board_id: String,
        include_sections: bool,
    },
    Section {
        section_id: String,
    },
    UserPins {
        username: String,
    },
    UserCreated {
        username: String,
    },
    Search {
        query: String,
        scope: String,
    },
    Related {
        pin_id: String,
    },
    BoardIdeas {
        board_id: String,
    },
}

// ───────────────────────── helpers de JSON ─────────────────────────

fn s(v: &Value) -> Option<String> {
    match v {
        Value::String(x) => {
            let t = x.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        }
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn n(v: &Value) -> u64 {
    v.as_u64()
        .or_else(|| v.as_f64().map(|f| f.max(0.0) as u64))
        .unwrap_or(0)
}

fn media(v: &Value) -> Option<Media> {
    let url = s(&v["url"])?;
    Some(Media {
        url,
        width: n(&v["width"]) as u32,
        height: n(&v["height"]) as u32,
    })
}

fn person(v: &Value) -> Option<Person> {
    if !v.is_object() {
        return None;
    }
    let name = s(&v["full_name"]).or_else(|| {
        let f = s(&v["first_name"]).unwrap_or_default();
        let l = s(&v["last_name"]).unwrap_or_default();
        let t = format!("{} {}", f, l).trim().to_string();
        if t.is_empty() {
            None
        } else {
            Some(t)
        }
    });
    Some(Person {
        id: s(&v["id"]),
        username: s(&v["username"]),
        name,
        avatar: s(&v["image_medium_url"])
            .or_else(|| s(&v["image_large_url"]))
            .or_else(|| s(&v["image_small_url"])),
    })
}

/// Reescreve `…/236x/aa/bb/cc/hash.jpg` para `/originals/`.
pub fn to_originals(url: &str) -> String {
    let re = regex::Regex::new(r"^(https?://i\.pinimg\.com)/(?:\d+x\d*|originals)(/[0-9a-f]{2}/[0-9a-f]{2}/[0-9a-f]{2}/[^/?#]+)$").unwrap();
    match re.captures(url) {
        Some(c) => format!("{}/originals{}", &c[1], &c[2]),
        None => url.to_string(),
    }
}

fn video_from_list(list: &Value) -> Option<Video> {
    let obj = list.as_object()?;
    let mut out = Video::default();
    let mut best_mp4: Option<(u32, String)> = None;
    for (key, v) in obj {
        let Some(url) = s(&v["url"]) else { continue };
        let w = n(&v["width"]) as u32;
        let h = n(&v["height"]) as u32;
        if out.thumbnail.is_none() {
            out.thumbnail = s(&v["thumbnail"]);
        }
        if out.duration_ms == 0 {
            out.duration_ms = n(&v["duration"]);
        }
        if url.contains(".m3u8") {
            let rank = match key.as_str() {
                "V_HLSV4" => 3,
                "V_HLSV3_WEB" => 2,
                _ => 1,
            };
            let cur = out.hls.as_ref().map(|_| out.width).unwrap_or(0);
            if out.hls.is_none() || rank >= 3 || h > cur {
                out.hls = Some(url);
                out.width = w;
                out.height = h;
            }
        } else if url.contains(".mp4") && best_mp4.as_ref().map(|(bh, _)| h > *bh).unwrap_or(true) {
            best_mp4 = Some((h, url));
            if out.hls.is_none() {
                out.width = w;
                out.height = h;
            }
        }
    }
    out.mp4 = best_mp4.map(|(_, u)| u);
    if out.hls.is_none() && out.mp4.is_none() {
        return None;
    }
    Some(out)
}

fn largest_image(images: &Value) -> (Option<Media>, Option<Media>, Option<String>) {
    let obj = match images.as_object() {
        Some(o) => o,
        None => return (None, None, None),
    };
    let orig = obj
        .get("orig")
        .or_else(|| obj.get("originals"))
        .and_then(media);
    let large = obj.get("736x").and_then(media).or_else(|| {
        obj.iter()
            .filter(|(k, _)| k.ends_with('x'))
            .filter_map(|(_, v)| media(v))
            .max_by_key(|m| m.width)
    });
    let thumb = obj
        .get("236x")
        .and_then(media)
        .map(|m| m.url)
        .or_else(|| large.as_ref().map(|m| m.url.clone()));
    (orig, large, thumb)
}

fn detect_ai(v: &Value, title: &str, description: &str, alt: &str) -> AiInfo {
    let mut info = AiInfo::default();
    let disc = &v["ai_disclosures"];
    if disc.is_array() && !disc.as_array().map(|a| a.is_empty()).unwrap_or(true)
        || disc.is_object() && !disc.as_object().map(|o| o.is_empty()).unwrap_or(true)
        || disc.as_bool() == Some(true)
    {
        info.labeled = true;
    }
    if let Some(topics) = v["gen_ai_topics"].as_array() {
        for t in topics {
            if let Some(t) = s(t).or_else(|| s(&t["name"])) {
                info.topics.push(t);
            }
        }
        if !info.topics.is_empty() {
            info.labeled = true;
        }
    }
    if v["is_ai_generated"].as_bool() == Some(true) || v["is_gen_ai"].as_bool() == Some(true) {
        info.labeled = true;
    }
    let text = format!("{} \n {} \n {}", title, description, alt).to_lowercase();
    let (level, kw) = super::analysis::ai_keyword_level(&text);
    info.keyword_level = level;
    info.keyword = kw;
    info
}

/// Normaliza o JSON de um pin (qualquer `field_set_key`).
pub fn parse_pin(v: &Value) -> Option<Pin> {
    let id = s(&v["id"])?;
    if v["type"].as_str().map(|t| t != "pin").unwrap_or(false) {
        return None;
    }
    let title = s(&v["title"])
        .or_else(|| s(&v["grid_title"]))
        .unwrap_or_default();
    let description = s(&v["description"])
        .or_else(|| s(&v["closeup_unified_description"]))
        .or_else(|| s(&v["closeup_description"]))
        .or_else(|| s(&v["grid_description"]))
        .unwrap_or_default();
    let alt_text = s(&v["alt_text"])
        .or_else(|| s(&v["auto_alt_text"]))
        .or_else(|| s(&v["seo_alt_text"]))
        .unwrap_or_default();
    let (mut image, image_large, thumb) = largest_image(&v["images"]);
    let image_signature = s(&v["image_signature"]);
    if image.is_none() {
        if let (Some(sig), Some(l)) = (&image_signature, &image_large) {
            let ext = l.url.rsplit('.').next().unwrap_or("jpg");
            image = Some(Media {
                url: format!(
                    "https://i.pinimg.com/originals/{}/{}/{}/{}.{}",
                    &sig[0..2],
                    &sig[2..4],
                    &sig[4..6],
                    sig,
                    ext
                ),
                width: 0,
                height: 0,
            });
        }
    }

    let mut video = video_from_list(&v["videos"]["video_list"]);
    let mut extras = Vec::new();
    // story pins: páginas com blocos de imagem/vídeo
    if let Some(pages) = v["story_pin_data"]["pages"].as_array() {
        let mut idx = 0usize;
        for page in pages {
            let blocks = page["blocks"].as_array().cloned().unwrap_or_default();
            for block in blocks {
                let bt = block["block_type"].as_u64().unwrap_or(0);
                let btype = block["type"].as_str().unwrap_or("");
                if bt == 3 || btype.contains("video") {
                    if let Some(vv) = video_from_list(&block["video"]["video_list"]) {
                        if video.is_none() {
                            video = Some(vv.clone());
                        }
                        extras.push(Extra {
                            index: idx,
                            kind: "video".into(),
                            image: None,
                            video: Some(vv),
                        });
                        idx += 1;
                    }
                } else if bt == 2 || btype.contains("image") {
                    let (o, l, _) = largest_image(&block["image"]["images"]);
                    let m = o.or(l).or_else(|| {
                        let sig =
                            s(&block["image_signature"]).or_else(|| s(&page["image_signature"]))?;
                        Some(Media {
                            url: format!(
                                "https://i.pinimg.com/originals/{}/{}/{}/{}.jpg",
                                &sig[0..2],
                                &sig[2..4],
                                &sig[4..6],
                                sig
                            ),
                            width: 0,
                            height: 0,
                        })
                    });
                    if let Some(m) = m {
                        extras.push(Extra {
                            index: idx,
                            kind: "image".into(),
                            image: Some(m),
                            video: None,
                        });
                        idx += 1;
                    }
                }
            }
        }
    }
    // carrossel
    if let Some(slots) = v["carousel_data"]["carousel_slots"].as_array() {
        extras.clear();
        for (i, slot) in slots.iter().enumerate() {
            let (o, l, _) = largest_image(&slot["images"]);
            let m = o.or_else(|| {
                l.map(|m| Media {
                    url: to_originals(&m.url),
                    ..m
                })
            });
            if let Some(m) = m {
                extras.push(Extra {
                    index: i,
                    kind: "image".into(),
                    image: Some(m),
                    video: None,
                });
            }
        }
    }

    let is_gif = image
        .as_ref()
        .map(|m| m.url.to_lowercase().ends_with(".gif"))
        .unwrap_or(false);
    let kind = if v["carousel_data"]["carousel_slots"].is_array() && extras.len() > 1 {
        "carousel"
    } else if extras.len() > 1 {
        "story"
    } else if video.is_some() || v["is_video"].as_bool() == Some(true) {
        "video"
    } else if is_gif {
        "gif"
    } else {
        "image"
    };
    if extras.len() == 1 {
        extras.clear();
    }

    let stats = &v["aggregated_pin_data"]["aggregated_stats"];
    let reactions = v["reaction_counts"]
        .as_object()
        .map(|o| o.values().map(n).sum())
        .unwrap_or(0);
    let rich_v = if v["rich_metadata"].is_object() {
        &v["rich_metadata"]
    } else {
        &v["rich_summary"]
    };
    let rich = rich_v.as_object().map(|_| Rich {
        site_name: s(&rich_v["site_name"]),
        title: s(&rich_v["title"]),
        url: s(&rich_v["url"]),
        description: s(&rich_v["description"]),
    });
    let att_v = &v["attribution"];
    let attribution = att_v.as_object().map(|_| Attribution {
        author_name: s(&att_v["author_name"]),
        author_url: s(&att_v["author_url"]),
        provider_name: s(&att_v["provider_name"]),
        title: s(&att_v["title"]),
        url: s(&att_v["url"]),
    });
    let board_v = &v["board"];
    let board = board_v.as_object().map(|_| BoardRef {
        id: s(&board_v["id"]),
        name: s(&board_v["name"]),
        url: s(&board_v["url"]).map(|u| {
            if u.starts_with('/') {
                format!("{}{}", ROOT, u)
            } else {
                u
            }
        }),
    });
    let ai = detect_ai(v, &title, &description, &alt_text);

    Some(Pin {
        url: format!("{}/pin/{}/", ROOT, id),
        id,
        title,
        description,
        alt_text,
        link: s(&v["link"]),
        domain: s(&v["domain"]).or_else(|| s(&v["link_domain"])),
        created_at: s(&v["created_at"]),
        kind: kind.to_string(),
        image_signature,
        image,
        image_large,
        thumb,
        video,
        extras,
        pinner: person(&v["pinner"]),
        creator: person(&v["native_creator"]).or_else(|| person(&v["closeup_attribution"])),
        board,
        saves: n(&stats["saves"]),
        repins: n(&v["repin_count"]),
        comments: n(&v["comment_count"]),
        reactions,
        is_promoted: v["is_promoted"].as_bool().unwrap_or(false)
            || v["is_downstream_promotion"].as_bool().unwrap_or(false),
        is_repin: v["is_repin"].as_bool().unwrap_or(false),
        ai,
        dominant_color: s(&v["dominant_color"]),
        rich,
        attribution,
        section: None,
    })
}

fn parse_board(v: &Value) -> Option<Board> {
    let id = s(&v["id"])?;
    let url = s(&v["url"])
        .map(|u| {
            if u.starts_with('/') {
                format!("{}{}", ROOT, u)
            } else {
                u
            }
        })
        .unwrap_or_default();
    Some(Board {
        id,
        name: s(&v["name"]).unwrap_or_default(),
        url,
        description: s(&v["description"]).unwrap_or_default(),
        pin_count: n(&v["pin_count"]),
        section_count: n(&v["section_count"]),
        follower_count: n(&v["follower_count"]),
        privacy: s(&v["privacy"]).unwrap_or_else(|| "public".into()),
        cover: s(&v["image_cover_hd_url"])
            .or_else(|| s(&v["image_cover_url"]))
            .or_else(|| {
                v["images"]["474x"]
                    .as_array()
                    .and_then(|a| a.first())
                    .and_then(|m| s(&m["url"]))
            }),
        owner: person(&v["owner"]),
        is_collaborative: v["is_collaborative"].as_bool().unwrap_or(false),
    })
}

fn parse_user(v: &Value) -> Option<User> {
    let id = s(&v["id"])?;
    Some(User {
        id,
        username: s(&v["username"]).unwrap_or_default(),
        name: s(&v["full_name"]).unwrap_or_default(),
        about: s(&v["about"]).unwrap_or_default(),
        website: s(&v["website_url"]).or_else(|| s(&v["listed_website_url"])),
        avatar: s(&v["image_xlarge_url"]).or_else(|| s(&v["image_large_url"])),
        pin_count: n(&v["pin_count"]),
        board_count: n(&v["board_count"]),
        follower_count: n(&v["follower_count"]),
        following_count: n(&v["following_count"]),
        is_verified_merchant: v["is_verified_merchant"].as_bool().unwrap_or(false),
        is_private: v["is_private_profile"].as_bool().unwrap_or(false),
    })
}

// ───────────────────────── cliente ─────────────────────────

pub struct PinClient {
    http: reqwest::Client,
    cookie: Option<String>,
    csrf: Option<String>,
}

/// Interpreta o campo "cookies" da UI: caminho de um cookies.txt (Netscape)
/// ou a string crua `a=1; b=2`.
pub fn cookie_header(input: Option<&str>) -> Option<String> {
    let raw = input?.trim();
    if raw.is_empty() {
        return None;
    }
    let p = std::path::Path::new(raw);
    if p.exists() && p.is_file() {
        let content = std::fs::read_to_string(p).ok()?;
        return crate::platforms::cookie_provider::cookie_header_from_netscape_for_domain(
            &content,
            "pinterest.com",
        )
        .or_else(|| crate::platforms::cookie_provider::cookie_header_from_netscape(&content));
    }
    if raw.contains('\t') {
        return crate::platforms::cookie_provider::cookie_header_from_netscape_for_domain(
            raw,
            "pinterest.com",
        );
    }
    if raw.contains('=') {
        return Some(raw.trim_start_matches("Cookie:").trim().to_string());
    }
    None
}

impl PinClient {
    pub fn new(cookies: Option<&str>) -> anyhow::Result<Self> {
        use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, USER_AGENT};
        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static(UA));
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/json, text/javascript, */*, q=0.01"),
        );
        headers.insert(
            "X-Requested-With",
            HeaderValue::from_static("XMLHttpRequest"),
        );
        headers.insert("X-Pinterest-AppState", HeaderValue::from_static("active"));
        headers.insert(
            "X-Pinterest-PWS-Handler",
            HeaderValue::from_static("www/[username].js"),
        );
        headers.insert(
            "Accept-Language",
            HeaderValue::from_static("en-US,en;q=0.9"),
        );
        let http = crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
            .default_headers(headers)
            .timeout(Duration::from_secs(120))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        let cookie = cookie_header(cookies);
        let csrf = cookie.as_ref().and_then(|c| {
            c.split(';')
                .map(|p| p.trim())
                .find_map(|p| p.strip_prefix("csrftoken=").map(|v| v.to_string()))
        });
        Ok(Self { http, cookie, csrf })
    }

    pub fn has_session(&self) -> bool {
        self.cookie.is_some()
    }

    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    fn apply_cookie(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.cookie {
            Some(c) => rb.header(reqwest::header::COOKIE, c),
            None => rb,
        }
    }

    /// GET em um resource; devolve `data` e o próximo bookmark (None no fim).
    async fn resource(
        &self,
        name: &str,
        options: Value,
        source_url: &str,
    ) -> anyhow::Result<(Value, Option<String>)> {
        let url = format!("{}/resource/{}Resource/get/", ROOT, name);
        let data = json!({ "options": options, "context": {} }).to_string();
        let mut attempt = 0u32;
        loop {
            attempt += 1;
            let req = self
                .http
                .get(&url)
                .query(&[("source_url", source_url), ("data", data.as_str())])
                .header("X-Pinterest-Source-Url", source_url);
            let resp = self.apply_cookie(req).send().await?;
            let status = resp.status();
            if (status.as_u16() == 429 || status.is_server_error()) && attempt < 4 {
                tokio::time::sleep(Duration::from_millis(800 * attempt as u64 * attempt as u64))
                    .await;
                continue;
            }
            let body: Value = match resp.json().await {
                Ok(v) => v,
                Err(e) => {
                    return Err(anyhow!(
                        "Pinterest respondeu HTTP {} sem JSON ({})",
                        status,
                        e
                    ))
                }
            };
            let rr = &body["resource_response"];
            if rr["status"].as_str() != Some("success") {
                let msg = rr["message"]
                    .as_str()
                    .or_else(|| rr["error"]["message"].as_str())
                    .unwrap_or("resposta sem status de sucesso");
                let code = rr["error"]["http_status"]
                    .as_u64()
                    .unwrap_or(status.as_u16() as u64);
                if code == 404 {
                    return Err(anyhow!(
                        "nao encontrado no Pinterest (privado, removido ou URL errada): {}",
                        msg
                    ));
                }
                if code == 401 || code == 403 {
                    return Err(anyhow!(
                        "o Pinterest negou o acesso ({}); para boards secretos informe os cookies",
                        msg
                    ));
                }
                return Err(anyhow!("Pinterest: {} (HTTP {})", msg, code));
            }
            let bookmark = body["resource"]["options"]["bookmarks"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|b| b.as_str())
                .map(|b| b.to_string())
                .or_else(|| rr["bookmark"].as_str().map(|b| b.to_string()))
                .filter(|b| b != "-end-" && !b.starts_with("Y2JOb25lO"));
            return Ok((rr["data"].clone(), bookmark));
        }
    }

    pub async fn pin(&self, id: &str) -> anyhow::Result<Pin> {
        let (data, _) = self
            .resource(
                "Pin",
                json!({ "id": id, "field_set_key": "detailed" }),
                &format!("/pin/{}/", id),
            )
            .await?;
        parse_pin(&data).ok_or_else(|| anyhow!("o Pinterest nao devolveu esse pin"))
    }

    pub async fn board(&self, user: &str, slug: &str) -> anyhow::Result<Board> {
        let src = format!("/{}/{}/", user, slug);
        let (data, _) = self
            .resource(
                "Board",
                json!({ "username": user, "slug": slug, "field_set_key": "detailed" }),
                &src,
            )
            .await?;
        parse_board(&data).ok_or_else(|| anyhow!("board nao encontrado"))
    }

    pub async fn board_sections(&self, board_id: &str) -> anyhow::Result<Vec<Section>> {
        let mut out = Vec::new();
        let mut bookmark: Option<String> = None;
        loop {
            let mut opts = json!({ "board_id": board_id, "page_size": 100 });
            if let Some(b) = &bookmark {
                opts["bookmarks"] = json!([b]);
            }
            let (data, next) = self.resource("BoardSections", opts, "/").await?;
            for v in data.as_array().cloned().unwrap_or_default() {
                if let Some(id) = s(&v["id"]) {
                    out.push(Section {
                        id,
                        slug: s(&v["slug"]).unwrap_or_default(),
                        title: s(&v["title"]).unwrap_or_default(),
                        pin_count: n(&v["pin_count"]),
                    });
                }
            }
            match next {
                Some(b) if !out.is_empty() => bookmark = Some(b),
                _ => break,
            }
        }
        Ok(out)
    }

    pub async fn user(&self, username: &str) -> anyhow::Result<User> {
        let (data, _) = self
            .resource(
                "User",
                json!({ "username": username, "field_set_key": "profile" }),
                &format!("/{}/", username),
            )
            .await?;
        parse_user(&data).ok_or_else(|| anyhow!("perfil nao encontrado"))
    }

    pub async fn user_boards(&self, username: &str) -> anyhow::Result<Vec<Board>> {
        let mut out = Vec::new();
        let mut bookmark: Option<String> = None;
        loop {
            let mut opts = json!({
                "username": username, "page_size": 100, "privacy_filter": "all", "sort": "last_pinned_to",
                "field_set_key": "profile_grid_item", "filter_stories": false, "group_by": "visibility", "include_archived": true
            });
            if let Some(b) = &bookmark {
                opts["bookmarks"] = json!([b]);
            }
            let (data, next) = self
                .resource("Boards", opts, &format!("/{}/_saved/", username))
                .await?;
            let before = out.len();
            for v in data.as_array().cloned().unwrap_or_default() {
                if let Some(b) = parse_board(&v) {
                    if !out.iter().any(|x: &Board| x.id == b.id) {
                        out.push(b);
                    }
                }
            }
            match next {
                Some(b) if out.len() > before => bookmark = Some(b),
                _ => break,
            }
        }
        Ok(out)
    }

    /// Uma página de um feed.
    pub async fn feed_page(
        &self,
        feed: &Feed,
        bookmark: Option<&str>,
        page_size: u32,
    ) -> anyhow::Result<(Vec<Pin>, Option<String>, Vec<String>)> {
        let bm = bookmark.map(|b| json!([b])).unwrap_or_else(|| json!([]));
        let (name, opts, src) = match feed {
            Feed::Board {
                board_id,
                include_sections,
            } => (
                "BoardFeed",
                json!({ "board_id": board_id, "page_size": page_size, "bookmarks": bm, "field_set_key": "react_grid_pin", "filter_section_pins": !include_sections, "layout": "default", "redux_normalize_feed": true }),
                "/".to_string(),
            ),
            Feed::Section { section_id } => (
                "BoardSectionPins",
                json!({ "section_id": section_id, "page_size": page_size, "bookmarks": bm, "field_set_key": "react_grid_pin", "redux_normalize_feed": true }),
                "/".to_string(),
            ),
            Feed::UserPins { username } => (
                "UserPins",
                json!({ "username": username, "page_size": page_size, "bookmarks": bm, "field_set_key": "grid_item" }),
                format!("/{}/pins/", username),
            ),
            Feed::UserCreated { username } => (
                "UserActivityPins",
                json!({ "username": username, "page_size": page_size, "bookmarks": bm, "exclude_add_pin_rep": true, "field_set_key": "grid_item", "is_own_profile_pins": false }),
                format!("/{}/_created/", username),
            ),
            Feed::Search { query, scope } => (
                "BaseSearch",
                json!({ "query": query, "scope": scope, "page_size": page_size, "bookmarks": bm, "rs": "typed", "auto_correction_disabled": false, "redux_normalize_feed": true }),
                format!(
                    "/search/{}/?q={}&rs=typed",
                    scope,
                    urlencoding::encode(query)
                ),
            ),
            Feed::Related { pin_id } => (
                "RelatedPinFeed",
                json!({ "pin": pin_id, "page_size": page_size, "bookmarks": bm, "context_pin_ids": [], "search_query": "", "source": "deep_linking", "top_level_source": "deep_linking", "top_level_source_depth": 1, "is_pdp": false }),
                format!("/pin/{}/", pin_id),
            ),
            Feed::BoardIdeas { board_id } => (
                "BoardContentRecommendation",
                json!({ "id": board_id, "type": "board", "page_size": page_size, "bookmarks": bm }),
                "/".to_string(),
            ),
        };
        let (data, next) = self.resource(name, opts, &src).await?;
        let mut guides = Vec::new();
        let items: Vec<Value> = if let Some(a) = data.as_array() {
            a.clone()
        } else {
            if let Some(g) = data["rankedGuides"].as_array() {
                for x in g {
                    if let Some(t) = s(&x["term"])
                        .or_else(|| s(&x["display"]))
                        .or_else(|| s(&x["name"]))
                    {
                        guides.push(t);
                    }
                }
            }
            data["results"].as_array().cloned().unwrap_or_default()
        };
        let mut pins = Vec::new();
        for v in &items {
            if let Some(p) = parse_pin(v) {
                pins.push(p);
            }
        }
        Ok((pins, next, guides))
    }

    /// Percorre o feed até `limit` pins (0 = tudo), com callback por página.
    pub async fn collect(
        &self,
        feed: &Feed,
        limit: usize,
        mut on_page: impl FnMut(usize),
    ) -> anyhow::Result<Vec<Pin>> {
        let mut out: Vec<Pin> = Vec::new();
        let mut bookmark: Option<String> = None;
        let mut empty_pages = 0u32;
        loop {
            let want = if limit == 0 {
                PAGE
            } else {
                ((limit - out.len()).min(PAGE as usize)).max(1) as u32
            };
            let (pins, next, _) = self.feed_page(feed, bookmark.as_deref(), want).await?;
            let before = out.len();
            for p in pins {
                if !out.iter().any(|x| x.id == p.id) {
                    out.push(p);
                }
            }
            on_page(out.len());
            if out.len() == before {
                empty_pages += 1;
            } else {
                empty_pages = 0;
            }
            if limit > 0 && out.len() >= limit {
                out.truncate(limit);
                break;
            }
            match next {
                Some(b) if empty_pages < 2 => {
                    bookmark = Some(b);
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
                _ => break,
            }
        }
        Ok(out)
    }

    /// Busca de boards (scope "boards" devolve boards em vez de pins).
    pub async fn search_boards(&self, query: &str, limit: usize) -> anyhow::Result<Vec<Board>> {
        let mut out = Vec::new();
        let mut bookmark: Option<String> = None;
        loop {
            let bm = bookmark
                .as_ref()
                .map(|b| json!([b]))
                .unwrap_or_else(|| json!([]));
            let (data, next) = self
                .resource(
                    "BaseSearch",
                    json!({ "query": query, "scope": "boards", "page_size": 50, "bookmarks": bm, "rs": "typed" }),
                    &format!("/search/boards/?q={}&rs=typed", urlencoding::encode(query)),
                )
                .await?;
            let before = out.len();
            for v in data["results"].as_array().cloned().unwrap_or_default() {
                if let Some(b) = parse_board(&v) {
                    if !out.iter().any(|x: &Board| x.id == b.id) {
                        out.push(b);
                    }
                }
            }
            if out.len() >= limit || out.len() == before {
                break;
            }
            match next {
                Some(b) => bookmark = Some(b),
                None => break,
            }
        }
        out.truncate(limit.max(1));
        Ok(out)
    }

    pub async fn typeahead(&self, term: &str) -> anyhow::Result<Vec<String>> {
        let (data, _) = self
            .resource(
                "AdvancedTypeahead",
                json!({ "pin_scope": "pins", "count": 20, "term": term }),
                "/",
            )
            .await?;
        let mut out = Vec::new();
        for it in data["items"].as_array().cloned().unwrap_or_default() {
            if let Some(t) = s(&it["label"])
                .or_else(|| s(&it["title"]))
                .or_else(|| s(&it["query"]))
                .or_else(|| s(&it["term"]))
            {
                if !out.contains(&t) {
                    out.push(t);
                }
            }
        }
        Ok(out)
    }

    /// Expande `pin.it/xxxx` para a URL final.
    pub async fn resolve_short(&self, code: &str) -> anyhow::Result<String> {
        let url = format!("https://api.pinterest.com/url_shortener/{}/redirect/", code);
        let resp = self.http.get(&url).send().await?;
        let mut loc = resp
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.to_string())
            .ok_or_else(|| anyhow!("o pin.it nao redirecionou"))?;
        // às vezes há um segundo pulo (pinterest.com/pin/…/sent/?…)
        for _ in 0..3 {
            if parse_target(&loc)
                .map(|t| !matches!(t, Target::Short { .. }))
                .unwrap_or(false)
                && !loc.ends_with("pinterest.com/")
            {
                break;
            }
            let r = self.http.get(&loc).send().await?;
            match r
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
            {
                Some(l) => loc = l.to_string(),
                None => break,
            }
        }
        Ok(loc.split("/sent/").next().unwrap_or(&loc).to_string())
    }

    /// Desfaz o save de um pin seu (exige cookies com `csrftoken`).
    pub async fn unsave(&self, pin_id: &str) -> anyhow::Result<()> {
        let csrf = self.csrf.clone().ok_or_else(|| {
            anyhow!("para desfazer saves informe os cookies da sua sessao (com csrftoken)")
        })?;
        let data = json!({ "options": { "id": pin_id }, "context": {} }).to_string();
        let req = self
            .http
            .post(format!("{}/resource/PinResource/delete/", ROOT))
            .header("X-CSRFToken", csrf)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .form(&[("source_url", format!("/pin/{}/", pin_id)), ("data", data)]);
        let resp = self.apply_cookie(req).send().await?;
        let status = resp.status();
        let body: Value = resp.json().await.unwrap_or(Value::Null);
        if !status.is_success() || body["resource_response"]["status"].as_str() != Some("success") {
            let msg = body["resource_response"]["error"]["message"]
                .as_str()
                .unwrap_or("falhou");
            return Err(anyhow!(
                "nao consegui desfazer o save de {} (HTTP {}): {}",
                pin_id,
                status,
                msg
            ));
        }
        Ok(())
    }

    /// Feed a partir de um alvo já resolvido (board precisa do id).
    pub async fn feed_for(&self, target: &Target) -> anyhow::Result<(Feed, String)> {
        match target {
            Target::Board { user, slug } => {
                let b = self.board(user, slug).await?;
                Ok((
                    Feed::Board {
                        board_id: b.id.clone(),
                        include_sections: true,
                    },
                    b.name,
                ))
            }
            Target::Section {
                user,
                slug,
                section,
            } => {
                let b = self.board(user, slug).await?;
                let secs = self.board_sections(&b.id).await?;
                let sec = secs
                    .into_iter()
                    .find(|x| &x.slug == section || x.title.eq_ignore_ascii_case(section))
                    .ok_or_else(|| {
                        anyhow!("secao '{}' nao encontrada no board {}", section, b.name)
                    })?;
                Ok((
                    Feed::Section { section_id: sec.id },
                    format!("{} · {}", b.name, sec.title),
                ))
            }
            Target::User { username } => Ok((
                Feed::UserPins {
                    username: username.clone(),
                },
                username.clone(),
            )),
            Target::UserCreated { username } => Ok((
                Feed::UserCreated {
                    username: username.clone(),
                },
                format!("{} (criados)", username),
            )),
            Target::Search { query, scope } => Ok((
                Feed::Search {
                    query: query.clone(),
                    scope: scope.clone(),
                },
                query.clone(),
            )),
            Target::Pin { id } => Ok((
                Feed::Related { pin_id: id.clone() },
                format!("parecidos com {}", id),
            )),
            Target::Short { code } => {
                let url = self.resolve_short(code).await?;
                let t = parse_target(&url)
                    .ok_or_else(|| anyhow!("nao entendi o destino de pin.it/{}", code))?;
                Box::pin(self.feed_for(&t)).await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_targets() {
        assert_eq!(
            parse_target("https://www.pinterest.com/pin/8373949304441833/"),
            Some(Target::Pin {
                id: "8373949304441833".into()
            })
        );
        assert_eq!(
            parse_target("https://br.pinterest.com/pin/mid-century--8373949304441833/"),
            Some(Target::Pin {
                id: "8373949304441833".into()
            })
        );
        assert_eq!(
            parse_target("8373949304441833"),
            Some(Target::Pin {
                id: "8373949304441833".into()
            })
        );
        assert_eq!(
            parse_target("pin.it/2u3CBgh"),
            Some(Target::Short {
                code: "2u3CBgh".into()
            })
        );
        assert_eq!(
            parse_target("https://www.pinterest.com/pinterest/home-decor/"),
            Some(Target::Board {
                user: "pinterest".into(),
                slug: "home-decor".into()
            })
        );
        assert_eq!(
            parse_target("https://www.pinterest.fr/pinterest/home-decor/living-room/"),
            Some(Target::Section {
                user: "pinterest".into(),
                slug: "home-decor".into(),
                section: "living-room".into()
            })
        );
        assert_eq!(
            parse_target("https://www.pinterest.com/pinterest/"),
            Some(Target::User {
                username: "pinterest".into()
            })
        );
        assert_eq!(
            parse_target("https://www.pinterest.com/pinterest/_created/"),
            Some(Target::UserCreated {
                username: "pinterest".into()
            })
        );
        assert_eq!(
            parse_target("https://www.pinterest.com/search/videos/?q=cats&rs=typed"),
            Some(Target::Search {
                query: "cats".into(),
                scope: "videos".into()
            })
        );
        assert_eq!(
            parse_target("mid century living room"),
            Some(Target::Search {
                query: "mid century living room".into(),
                scope: "pins".into()
            })
        );
        assert_eq!(parse_target("https://www.pinterest.com/settings/"), None);
    }

    #[test]
    fn rewrites_to_originals() {
        assert_eq!(
            to_originals("https://i.pinimg.com/236x/3e/57/c1/3e57c1b723b8e9d39b8c09f6c5efbfb0.jpg"),
            "https://i.pinimg.com/originals/3e/57/c1/3e57c1b723b8e9d39b8c09f6c5efbfb0.jpg"
        );
        assert_eq!(
            to_originals("https://example.com/a.jpg"),
            "https://example.com/a.jpg"
        );
    }

    #[test]
    fn parses_pin_json() {
        let v: Value = serde_json::from_str(r#"{
            "id": "1", "type": "pin", "title": "T", "description": "made with midjourney", "link": "https://x.y/z", "domain": "x.y",
            "image_signature": "3e57c1b723b8e9d39b8c09f6c5efbfb0",
            "images": {"236x": {"url": "https://i.pinimg.com/236x/3e/57/c1/3e57c1b723b8e9d39b8c09f6c5efbfb0.jpg", "width": 236, "height": 300},
                       "736x": {"url": "https://i.pinimg.com/736x/3e/57/c1/3e57c1b723b8e9d39b8c09f6c5efbfb0.jpg", "width": 736, "height": 900},
                       "orig": {"url": "https://i.pinimg.com/originals/3e/57/c1/3e57c1b723b8e9d39b8c09f6c5efbfb0.jpg", "width": 1000, "height": 1300}},
            "videos": {"video_list": {"V_HLSV4": {"url": "https://v1.pinimg.com/videos/iht/hls/v2/a.m3u8", "width": 720, "height": 1280, "duration": 5000, "thumbnail": "https://i.pinimg.com/t.jpg"},
                                      "V_720P": {"url": "https://v1.pinimg.com/videos/mc/720p/a.mp4", "width": 720, "height": 1280}}},
            "aggregated_pin_data": {"aggregated_stats": {"saves": 12}}, "repin_count": 3, "comment_count": 1, "reaction_counts": {"1": 4, "2": 1},
            "is_promoted": true, "gen_ai_topics": ["art"], "pinner": {"id": "9", "username": "u", "full_name": "U"}, "board": {"id": "b", "name": "B", "url": "/u/b/"}
        }"#).unwrap();
        let p = parse_pin(&v).unwrap();
        assert_eq!(p.kind, "video");
        assert_eq!(p.image.unwrap().width, 1000);
        assert_eq!(
            p.video.as_ref().unwrap().mp4.as_deref(),
            Some("https://v1.pinimg.com/videos/mc/720p/a.mp4")
        );
        assert_eq!(
            p.video.unwrap().hls.as_deref(),
            Some("https://v1.pinimg.com/videos/iht/hls/v2/a.m3u8")
        );
        assert_eq!(p.saves, 12);
        assert_eq!(p.reactions, 5);
        assert!(p.is_promoted);
        assert!(p.ai.labeled);
        assert_eq!(p.ai.keyword_level, 2);
        assert_eq!(
            p.board.unwrap().url.unwrap(),
            "https://www.pinterest.com/u/b/"
        );
    }

    #[test]
    fn cookie_header_from_raw_string() {
        assert_eq!(
            cookie_header(Some(" csrftoken=abc; _pinterest_sess=xyz ")).as_deref(),
            Some("csrftoken=abc; _pinterest_sess=xyz")
        );
        assert_eq!(cookie_header(Some("")), None);
        assert_eq!(cookie_header(None), None);
    }
}

/// Testes contra o Pinterest de verdade (`cargo test -- --ignored live_`).
#[cfg(test)]
mod live {
    use super::*;

    #[tokio::test]
    #[ignore]
    async fn live_board_pin_search() {
        let c = PinClient::new(None).unwrap();
        let b = c.board("pinterest", "home-decor").await.unwrap();
        assert!(b.pin_count > 100, "{:?}", b);
        let secs = c.board_sections(&b.id).await.unwrap();
        assert!(!secs.is_empty());
        let (pins, next, _) = c
            .feed_page(
                &Feed::Board {
                    board_id: b.id.clone(),
                    include_sections: true,
                },
                None,
                10,
            )
            .await
            .unwrap();
        assert!(pins.len() >= 5, "{}", pins.len());
        assert!(next.is_some());
        let p = c.pin(&pins[0].id).await.unwrap();
        assert!(p.image.is_some() || p.video.is_some(), "{:?}", p);
        let u = c.user("pinterest").await.unwrap();
        assert!(u.board_count > 10);
        let boards = c.user_boards("pinterest").await.unwrap();
        assert!(boards.len() > 10, "{}", boards.len());
        let (pins, _, guides) = c
            .feed_page(
                &Feed::Search {
                    query: "mid century living room".into(),
                    scope: "pins".into(),
                },
                None,
                10,
            )
            .await
            .unwrap();
        assert!(pins.len() >= 5);
        assert!(!guides.is_empty(), "guides vazios");
        let (vids, _, _) = c
            .feed_page(
                &Feed::Search {
                    query: "cats".into(),
                    scope: "videos".into(),
                },
                None,
                5,
            )
            .await
            .unwrap();
        assert!(vids.iter().any(|v| v.video.is_some()), "nenhum video");
        let rel = c
            .collect(
                &Feed::Related {
                    pin_id: p.id.clone(),
                },
                8,
                |_| {},
            )
            .await
            .unwrap();
        assert!(rel.len() >= 4, "{}", rel.len());
        let sug = c.typeahead("mid century").await.unwrap();
        assert!(!sug.is_empty());
        let bs = c.search_boards("living room", 5).await.unwrap();
        assert!(!bs.is_empty());
        // originais + dHash + paleta
        let img = p.image.clone().or(p.image_large.clone()).unwrap();
        let (bytes, ext) = super::super::media::fetch_image(&c, &img.url)
            .await
            .unwrap();
        assert!(bytes.len() > 1000);
        println!(
            "orig {} bytes ext {} ({}x{})",
            bytes.len(),
            ext,
            img.width,
            img.height
        );
        let h = super::super::analysis::dhash(&bytes).unwrap();
        assert_ne!(h, 0);
        let samples = super::super::analysis::sample_pixels(&bytes, 500, true);
        let pal = super::super::analysis::kmeans_palette(&samples, 5);
        assert_eq!(pal.len(), 5);
        println!(
            "palette {:?}",
            pal.iter().map(|s| s.hex.clone()).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    #[ignore]
    async fn live_short_link() {
        let c = PinClient::new(None).unwrap();
        let (pins, _, _) = c
            .feed_page(
                &Feed::Search {
                    query: "cats".into(),
                    scope: "pins".into(),
                },
                None,
                1,
            )
            .await
            .unwrap();
        let p = &pins[0];
        assert!(p.url.contains("/pin/"));
    }
}
