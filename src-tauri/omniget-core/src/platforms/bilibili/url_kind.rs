use once_cell::sync::Lazy;
use regex::Regex;

use super::api::{ApiClient, BilibiliError, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlKind {
    Video {
        bvid_or_av: String,
        page: Option<u32>,
    },
    BangumiEpisode {
        ep_id: u64,
    },
    BangumiSeason {
        season_id: u64,
    },
    BangumiMedia {
        media_id: u64,
    },
    CheeseEpisode {
        ep_id: u64,
    },
    CheeseSeason {
        season_id: u64,
    },
    Space {
        mid: u64,
    },
    Favlist {
        fid: u64,
    },
    Collection {
        mid: u64,
        sid: u64,
    },
    Series {
        mid: u64,
        sid: u64,
    },
    PopularWeek {
        num: u32,
    },
    WatchLater,
    History,
    Festival {
        url: String,
    },
}

impl UrlKind {
    pub fn label_key(&self) -> &'static str {
        match self {
            UrlKind::Video { .. } => "platforms.bilibili.kind.video",
            UrlKind::BangumiEpisode { .. } => "platforms.bilibili.kind.bangumi_episode",
            UrlKind::BangumiSeason { .. } => "platforms.bilibili.kind.bangumi_season",
            UrlKind::BangumiMedia { .. } => "platforms.bilibili.kind.bangumi_media",
            UrlKind::CheeseEpisode { .. } => "platforms.bilibili.kind.cheese_episode",
            UrlKind::CheeseSeason { .. } => "platforms.bilibili.kind.cheese_season",
            UrlKind::Space { .. } => "platforms.bilibili.kind.space",
            UrlKind::Favlist { .. } => "platforms.bilibili.kind.favlist",
            UrlKind::Collection { .. } => "platforms.bilibili.kind.collection",
            UrlKind::Series { .. } => "platforms.bilibili.kind.series",
            UrlKind::PopularWeek { .. } => "platforms.bilibili.kind.popular",
            UrlKind::WatchLater => "platforms.bilibili.kind.watch_later",
            UrlKind::History => "platforms.bilibili.kind.history",
            UrlKind::Festival { .. } => "platforms.bilibili.kind.festival",
        }
    }
}

static RE_VIDEO_BV: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"bilibili\.com/video/(BV[a-zA-Z0-9]+|av\d+)").unwrap());
static RE_BARE_BV: Lazy<Regex> = Lazy::new(|| Regex::new(r"(BV[a-zA-Z0-9]{8,})").unwrap());
static RE_BARE_AV: Lazy<Regex> = Lazy::new(|| Regex::new(r"\bav(\d+)\b").unwrap());
static RE_BANGUMI: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"bilibili\.com/bangumi/(?:play|media)/(ss\d+|ep\d+|md\d+)").unwrap());
static RE_BARE_BANGUMI: Lazy<Regex> = Lazy::new(|| Regex::new(r"\b(ss\d+|ep\d+|md\d+)\b").unwrap());
static RE_CHEESE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"bilibili\.com/cheese/play/(ss\d+|ep\d+)").unwrap());
static RE_LIST_LISTS: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"space\.bilibili\.com/(\d+)/lists(?:/(\d+))?").unwrap());
static RE_FAVLIST_SPACE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"space\.bilibili\.com/\d+/favlist").unwrap());
static RE_FAVLIST_LIST: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"bilibili\.com/(?:medialist/detail/)?ml(\d+)|bilibili\.com/list/ml(\d+)").unwrap()
});
static RE_SPACE: Lazy<Regex> = Lazy::new(|| Regex::new(r"space\.bilibili\.com/(\d+)").unwrap());
static RE_MEDIALIST_PLAY: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"bilibili\.com/medialist/play/(\d+)").unwrap());
static RE_POPULAR: Lazy<Regex> = Lazy::new(|| Regex::new(r"bilibili\.com/v/popular").unwrap());
static RE_FESTIVAL: Lazy<Regex> = Lazy::new(|| Regex::new(r"bilibili\.com/festival").unwrap());
static RE_B23: Lazy<Regex> = Lazy::new(|| Regex::new(r"(b23\.tv|bili2233\.cn)").unwrap());
static RE_LIST_OLD: Lazy<Regex> = Lazy::new(|| Regex::new(r"bilibili\.com/list/(\d+)").unwrap());

pub fn detect(url: &str) -> Result<UrlKind> {
    detect_internal(url, None)
}

pub fn detect_with_query(url: &str, query: Option<&str>) -> Result<UrlKind> {
    detect_internal(url, query)
}

fn detect_internal(url: &str, query_override: Option<&str>) -> Result<UrlKind> {
    if url == "omniget://bilibili/watch-later" {
        return Ok(UrlKind::WatchLater);
    }
    if url == "omniget://bilibili/history" {
        return Ok(UrlKind::History);
    }

    if RE_VIDEO_BV.is_match(url) {
        if let Some(cap) = RE_VIDEO_BV.captures(url) {
            let id = cap
                .get(1)
                .map(|m| m.as_str().to_string())
                .unwrap_or_default();
            let page = extract_page_query(url, query_override);
            return Ok(UrlKind::Video {
                bvid_or_av: id,
                page,
            });
        }
    }

    if let Some(cap) = RE_BANGUMI.captures(url) {
        let raw = cap.get(1).map(|m| m.as_str()).unwrap_or("");
        return parse_bangumi_id(raw);
    }

    if let Some(cap) = RE_CHEESE.captures(url) {
        let raw = cap.get(1).map(|m| m.as_str()).unwrap_or("");
        if let Some(rest) = raw.strip_prefix("ss") {
            if let Ok(id) = rest.parse::<u64>() {
                return Ok(UrlKind::CheeseSeason { season_id: id });
            }
        }
        if let Some(rest) = raw.strip_prefix("ep") {
            if let Ok(id) = rest.parse::<u64>() {
                return Ok(UrlKind::CheeseEpisode { ep_id: id });
            }
        }
    }

    if let Some(cap) = RE_LIST_LISTS.captures(url) {
        let mid: u64 = cap
            .get(1)
            .and_then(|m| m.as_str().parse().ok())
            .unwrap_or(0);
        let sid: u64 = cap
            .get(2)
            .and_then(|m| m.as_str().parse().ok())
            .unwrap_or(0);
        let kind_q = extract_query_value(url, query_override, "type").unwrap_or_default();
        let resolved_sid = if sid == 0 {
            extract_query_value(url, query_override, "sid")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0)
        } else {
            sid
        };
        if kind_q == "series" {
            return Ok(UrlKind::Series {
                mid,
                sid: resolved_sid,
            });
        }
        return Ok(UrlKind::Collection {
            mid,
            sid: resolved_sid,
        });
    }

    if RE_FAVLIST_SPACE.is_match(url) {
        let fid = extract_query_value(url, query_override, "fid")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        return Ok(UrlKind::Favlist { fid });
    }

    if let Some(cap) = RE_FAVLIST_LIST.captures(url) {
        let id = cap
            .get(1)
            .or_else(|| cap.get(2))
            .and_then(|m| m.as_str().parse().ok())
            .unwrap_or(0);
        return Ok(UrlKind::Favlist { fid: id });
    }

    if let Some(cap) = RE_MEDIALIST_PLAY.captures(url) {
        if let Some(mid) = cap.get(1).and_then(|m| m.as_str().parse().ok()) {
            return Ok(UrlKind::Space { mid });
        }
    }

    if let Some(cap) = RE_SPACE.captures(url) {
        if let Some(mid) = cap.get(1).and_then(|m| m.as_str().parse().ok()) {
            return Ok(UrlKind::Space { mid });
        }
    }

    if let Some(cap) = RE_LIST_OLD.captures(url) {
        if let Some(sid) = cap.get(1).and_then(|m| m.as_str().parse().ok()) {
            return Ok(UrlKind::Collection { mid: 0, sid });
        }
    }

    if RE_POPULAR.is_match(url) {
        let num = extract_query_value(url, query_override, "num")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        return Ok(UrlKind::PopularWeek { num });
    }

    if RE_FESTIVAL.is_match(url) {
        return Ok(UrlKind::Festival {
            url: url.to_string(),
        });
    }

    if let Some(cap) = RE_BARE_BV.captures(url) {
        let id = cap
            .get(1)
            .map(|m| m.as_str().to_string())
            .unwrap_or_default();
        return Ok(UrlKind::Video {
            bvid_or_av: id,
            page: None,
        });
    }

    if let Some(cap) = RE_BARE_AV.captures(url) {
        let n = cap.get(1).map(|m| m.as_str()).unwrap_or("");
        return Ok(UrlKind::Video {
            bvid_or_av: format!("av{}", n),
            page: None,
        });
    }

    if let Some(cap) = RE_BARE_BANGUMI.captures(url) {
        return parse_bangumi_id(cap.get(1).map(|m| m.as_str()).unwrap_or(""));
    }

    Err(BilibiliError::ContentUnavailable)
}

fn parse_bangumi_id(raw: &str) -> Result<UrlKind> {
    if let Some(rest) = raw.strip_prefix("ep") {
        if let Ok(id) = rest.parse::<u64>() {
            return Ok(UrlKind::BangumiEpisode { ep_id: id });
        }
    }
    if let Some(rest) = raw.strip_prefix("ss") {
        if let Ok(id) = rest.parse::<u64>() {
            return Ok(UrlKind::BangumiSeason { season_id: id });
        }
    }
    if let Some(rest) = raw.strip_prefix("md") {
        if let Ok(id) = rest.parse::<u64>() {
            return Ok(UrlKind::BangumiMedia { media_id: id });
        }
    }
    Err(BilibiliError::ContentUnavailable)
}

fn extract_query_value(url: &str, query_override: Option<&str>, key: &str) -> Option<String> {
    if let Some(q) = query_override {
        for pair in q.split('&') {
            let mut it = pair.splitn(2, '=');
            let k = it.next()?;
            let v = it.next().unwrap_or("");
            if k == key {
                return Some(v.to_string());
            }
        }
    }
    if let Ok(parsed) = url::Url::parse(url) {
        for (k, v) in parsed.query_pairs() {
            if k == key {
                return Some(v.into_owned());
            }
        }
    }
    None
}

fn extract_page_query(url: &str, query_override: Option<&str>) -> Option<u32> {
    extract_query_value(url, query_override, "p").and_then(|s| s.parse().ok())
}

pub async fn resolve_b23(client: &ApiClient, short: &str) -> Result<String> {
    let resolved = client.resolve_redirect(short).await?;
    if resolved == short {
        return Err(BilibiliError::ContentUnavailable);
    }
    Ok(resolved)
}

pub fn is_b23_short(url: &str) -> bool {
    RE_B23.is_match(url)
}

pub fn av_to_bv(avid: u64) -> String {
    const TABLE: &[u8; 58] = b"fZodR9XQDSUm21yCkr6zBqiveYah8bt4xsWpHnJE7jL5VG3guMTKNPAwcF";
    const XOR: u64 = 177451812;
    const ADD: u64 = 8728348608;
    let s: [usize; 6] = [11, 10, 3, 8, 4, 6];
    let mut x = (avid ^ XOR).wrapping_add(ADD);
    let mut chars: [u8; 12] = *b"BV1  4 1 7  ";
    let mut idx: [usize; 6] = [0; 6];
    for i in 0..6 {
        idx[i] = ((x as f64 / 58f64.powi(i as i32)) as u64 % 58) as usize;
        chars[s[i]] = TABLE[idx[i]];
        let _ = &mut x;
    }
    String::from_utf8(chars.to_vec()).unwrap_or_default()
}

pub fn parse_video_id(input: &str) -> (Option<String>, Option<u64>) {
    if let Some(rest) = input.strip_prefix("av") {
        if let Ok(n) = rest.parse::<u64>() {
            return (None, Some(n));
        }
    }
    if input.starts_with("BV") {
        return (Some(input.to_string()), None);
    }
    (None, None)
}

pub fn extract_festival_bvid(html: &str) -> Option<String> {
    static RE_STATE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"window\.__INITIAL_STATE__\s*=\s*(\{.*?\});").unwrap());
    let cap = RE_STATE.captures(html)?;
    let json_str = cap.get(1)?.as_str();
    let v: serde_json::Value = serde_json::from_str(json_str).ok()?;
    v.get("videoInfo")
        .and_then(|x| x.get("bvid"))
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_video_bv() {
        let url = "https://www.bilibili.com/video/BV1xx411c7mu";
        assert_eq!(
            detect(url).unwrap(),
            UrlKind::Video {
                bvid_or_av: "BV1xx411c7mu".to_string(),
                page: None,
            }
        );
    }

    #[test]
    fn detects_video_av() {
        let url = "https://www.bilibili.com/video/av170001";
        assert_eq!(
            detect(url).unwrap(),
            UrlKind::Video {
                bvid_or_av: "av170001".to_string(),
                page: None,
            }
        );
    }

    #[test]
    fn detects_video_multipart() {
        let url = "https://www.bilibili.com/video/BV1xx411c7mu?p=3";
        match detect(url).unwrap() {
            UrlKind::Video { page, .. } => assert_eq!(page, Some(3)),
            _ => panic!("expected Video"),
        }
    }

    #[test]
    fn detects_bangumi_ep() {
        let url = "https://www.bilibili.com/bangumi/play/ep123456";
        assert_eq!(
            detect(url).unwrap(),
            UrlKind::BangumiEpisode { ep_id: 123456 }
        );
    }

    #[test]
    fn detects_bangumi_ss() {
        let url = "https://www.bilibili.com/bangumi/play/ss12345";
        assert_eq!(
            detect(url).unwrap(),
            UrlKind::BangumiSeason { season_id: 12345 }
        );
    }

    #[test]
    fn detects_bangumi_md() {
        let url = "https://www.bilibili.com/bangumi/media/md28223066";
        assert_eq!(
            detect(url).unwrap(),
            UrlKind::BangumiMedia { media_id: 28223066 }
        );
    }

    #[test]
    fn detects_cheese() {
        let url = "https://www.bilibili.com/cheese/play/ss1234";
        assert_eq!(
            detect(url).unwrap(),
            UrlKind::CheeseSeason { season_id: 1234 }
        );
    }

    #[test]
    fn detects_space() {
        let url = "https://space.bilibili.com/123456";
        assert_eq!(detect(url).unwrap(), UrlKind::Space { mid: 123456 });
    }

    #[test]
    fn detects_favlist_query() {
        let url = "https://space.bilibili.com/123/favlist?fid=789";
        assert_eq!(detect(url).unwrap(), UrlKind::Favlist { fid: 789 });
    }

    #[test]
    fn detects_favlist_short() {
        let url = "https://www.bilibili.com/list/ml42";
        assert_eq!(detect(url).unwrap(), UrlKind::Favlist { fid: 42 });
    }

    #[test]
    fn detects_collection() {
        let url = "https://space.bilibili.com/123/lists/456?type=season";
        assert_eq!(
            detect(url).unwrap(),
            UrlKind::Collection { mid: 123, sid: 456 }
        );
    }

    #[test]
    fn detects_series() {
        let url = "https://space.bilibili.com/123/lists/456?type=series";
        assert_eq!(detect(url).unwrap(), UrlKind::Series { mid: 123, sid: 456 });
    }

    #[test]
    fn detects_popular() {
        let url = "https://www.bilibili.com/v/popular?num=10";
        assert_eq!(detect(url).unwrap(), UrlKind::PopularWeek { num: 10 });
    }

    #[test]
    fn detects_festival() {
        let url = "https://www.bilibili.com/festival/2024_anime";
        match detect(url).unwrap() {
            UrlKind::Festival { .. } => {}
            _ => panic!("expected Festival"),
        }
    }

    #[test]
    fn detects_b23_short() {
        assert!(is_b23_short("https://b23.tv/abc"));
        assert!(is_b23_short("https://bili2233.cn/xyz"));
    }

    #[test]
    fn detects_internal_schemas() {
        assert_eq!(
            detect("omniget://bilibili/watch-later").unwrap(),
            UrlKind::WatchLater
        );
        assert_eq!(
            detect("omniget://bilibili/history").unwrap(),
            UrlKind::History
        );
    }
}
