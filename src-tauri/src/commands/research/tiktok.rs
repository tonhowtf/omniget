use std::sync::OnceLock;
use std::time::Duration;

use regex::Regex;
use serde::Serialize;
use serde_json::{json, Value};

use super::{client, fail, stats, strings, uint, NULL};

const CREATOR_DEADLINE: Duration = Duration::from_secs(75);
const FEED_PAGES: usize = 3;
const MAX_QUERIES: usize = 5;
const EXCERPT_CHARS: usize = 240;

pub const CREATORS_GUIDE: &str = "Write the creator shortlist from this evidence only. For each creator give: why they fit the brief (quote a transcript excerpt and its source URL; if `transcript.available` is false say the fit rests on captions only), recent median views with the sample size and date range (show the mean beside it, and say confidence is low when the sample is under 10), engagement, and the public contact found. Never invent a quote, an email or a number, never compare creators by follower count alone, and list everything in `missing` as open points.";

pub const BRIEF_GUIDE: &str = "Write the content brief in two separate sections. Section 1, Evidence: for each video the source URL, the hook exactly as transcribed, the themes and the claims, each beside its URL; mark videos with `no_transcript` as caption-only and never summarise a video from its caption as if it were a transcript. Section 2, Original ideas: themes that repeat across sources, hook patterns and new content angles in your own words, each pointing back to the evidence it came from. List every `needs_checking` sentence under a heading \"Claims to verify before use\" and do not repeat those claims as facts; transcripts can be machine-made, so mark quotes as approximate.";

pub const CAVEATS: &[&str] = &[
    "Public data only, read through the ScrapeCreators API at the moment of the call.",
    "Views are compared against each creator's own recent median, never across creators.",
    "A sample under 10 videos is directional: confidence is low.",
    "TikTok search can return duplicates and is not exhaustive.",
];

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Video {
    pub id: String,
    pub url: String,
    pub caption: String,
    pub created: Option<i64>,
    pub plays: Option<u64>,
    pub likes: Option<u64>,
    pub comments: Option<u64>,
    pub shares: Option<u64>,
    pub saves: Option<u64>,
    pub duration_s: Option<u64>,
    pub handle: String,
    pub name: String,
    pub author_followers: Option<u64>,
}

pub fn video(item: &Value) -> Option<Video> {
    let v = if item["aweme_info"].is_object() {
        &item["aweme_info"]
    } else {
        item
    };
    let id = v["aweme_id"]
        .as_str()
        .map(str::to_string)
        .or_else(|| v["aweme_id"].as_u64().map(|n| n.to_string()))?;
    let handle = v["author"]["unique_id"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let url = v["url"]
        .as_str()
        .filter(|u| u.starts_with("http"))
        .map(str::to_string)
        .or_else(|| {
            (!handle.is_empty()).then(|| format!("https://www.tiktok.com/@{handle}/video/{id}"))
        })?;
    let created = v["create_time"].as_i64().filter(|t| *t > 0).or_else(|| {
        v["create_time_utc"]
            .as_str()
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
            .map(|t| t.timestamp())
    });
    let stat = |key: &str| uint(&v["statistics"][key]);
    Some(Video {
        id,
        url,
        caption: v["desc"].as_str().unwrap_or_default().to_string(),
        created,
        plays: stat("play_count"),
        likes: stat("digg_count"),
        comments: stat("comment_count"),
        shares: stat("share_count"),
        saves: stat("collect_count"),
        duration_s: uint(&v["video"]["duration"]).map(|ms| ms / 1000),
        handle,
        name: v["author"]["nickname"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        author_followers: uint(&v["author"]["follower_count"]).filter(|n| *n > 0),
    })
}

pub fn videos(body: &Value) -> Result<Vec<Video>, String> {
    let list = ["aweme_list", "search_item_list", "items"]
        .iter()
        .find_map(|key| body.get(*key));
    match list {
        Some(Value::Array(items)) => Ok(items.iter().filter_map(video).collect()),
        Some(Value::Null) => Ok(Vec::new()),
        _ => Err(fail(
            "UPSTREAM_CHANGED",
            "the TikTok answer has no video list",
            false,
            "call the endpoint with research_call to see the new shape and report it",
            Value::Null,
        )),
    }
}

pub fn iso(secs: i64) -> Option<String> {
    chrono::DateTime::from_timestamp(secs, 0)
        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
}

fn recent(videos: &[Video], sample: usize) -> Vec<&Video> {
    let mut sorted: Vec<&Video> = videos.iter().collect();
    sorted.sort_by_key(|v| std::cmp::Reverse(v.created.unwrap_or(0)));
    sorted.truncate(sample);
    sorted
}

fn floats(values: impl Iterator<Item = Option<u64>>) -> Vec<f64> {
    values.flatten().map(|n| n as f64).collect()
}

pub fn creator_stats(videos: &[Video], sample: usize) -> Value {
    let used = recent(videos, sample);
    let plays = floats(used.iter().map(|v| v.plays));
    let rates: Vec<f64> = used
        .iter()
        .filter_map(|v| {
            let plays = v.plays.filter(|p| *p > 0)? as f64;
            let acts = v.likes.unwrap_or(0) + v.comments.unwrap_or(0) + v.shares.unwrap_or(0);
            Some(acts as f64 / plays)
        })
        .collect();
    let dates: Vec<i64> = used.iter().filter_map(|v| v.created).collect();
    json!({
        "sample_asked": sample,
        "sample": used.len(),
        "confidence": stats::confidence(plays.len()),
        "date_from": dates.iter().min().and_then(|t| iso(*t)),
        "date_to": dates.iter().max().and_then(|t| iso(*t)),
        "views": stats::summary(&plays),
        "likes_median": stats::median(&floats(used.iter().map(|v| v.likes))),
        "comments_median": stats::median(&floats(used.iter().map(|v| v.comments))),
        "shares_median": stats::median(&floats(used.iter().map(|v| v.shares))),
        "engagement_rate_median": stats::median(&rates).map(|r| (r * 10_000.0).round() / 10_000.0),
        "engagement_rate_formula": "(likes + comments + shares) / views, per video",
    })
}

pub fn vtt_text(raw: &str) -> String {
    raw.lines()
        .map(str::trim)
        .filter(|line| {
            !line.is_empty()
                && !line.starts_with("WEBVTT")
                && !line.starts_with("NOTE")
                && !line.contains("-->")
                && !line.chars().all(|c| c.is_ascii_digit())
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn sentences(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        let ends = match c {
            '\n' => true,
            '.' | '!' | '?' => chars.peek().is_none_or(|(_, next)| next.is_whitespace()),
            _ => false,
        };
        if ends {
            let end = at + c.len_utf8();
            let piece = text[start..end].trim();
            if !piece.is_empty() {
                out.push(piece);
            }
            start = end;
        }
    }
    let rest = text[start..].trim();
    if !rest.is_empty() {
        out.push(rest);
    }
    out
}

fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let mut out: String = text.chars().take(max).collect();
        out.push('…');
        out
    }
}

pub fn hook(transcript: &str) -> Option<String> {
    sentences(transcript).first().map(|s| cut(s, 200))
}

const STOPWORDS: &[&str] = &[
    "the",
    "and",
    "for",
    "with",
    "that",
    "this",
    "from",
    "are",
    "our",
    "your",
    "you",
    "who",
    "have",
    "has",
    "will",
    "want",
    "need",
    "about",
    "into",
    "their",
    "they",
    "them",
    "can",
    "should",
    "would",
    "looking",
    "find",
    "campaign",
    "brief",
    "creator",
    "creators",
    "tiktok",
    "video",
    "videos",
    "brand",
    "new",
    "all",
    "any",
    "not",
    "but",
    "more",
    "most",
    "very",
    "para",
    "com",
    "que",
    "uma",
    "dos",
    "das",
    "nos",
    "nas",
    "por",
    "como",
    "mais",
    "sobre",
    "campanha",
    "criador",
    "criadores",
    "marca",
    "quero",
    "preciso",
    "sem",
    "tem",
    "ser",
    "são",
    "los",
    "las",
    "del",
    "una",
    "con",
];

fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 3 && !STOPWORDS.contains(w))
        .map(str::to_string)
        .collect()
}

pub fn terms(queries: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for word in queries.iter().flat_map(|q| words(q)) {
        if !out.contains(&word) {
            out.push(word);
        }
    }
    out
}

pub fn suggest_queries(brief: &str) -> Vec<String> {
    let all = words(brief);
    let mut ranked: Vec<(String, usize, usize)> = Vec::new();
    for (at, word) in all.iter().enumerate() {
        match ranked.iter_mut().find(|row| &row.0 == word) {
            Some(row) => row.1 += 1,
            None => ranked.push((word.clone(), 1, at)),
        }
    }
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.2.cmp(&b.2)));
    let top: Vec<&str> = ranked.iter().take(4).map(|row| row.0.as_str()).collect();
    let mut out: Vec<String> = Vec::new();
    let mut add = |parts: &[usize]| {
        let picked: Vec<&str> = parts.iter().filter_map(|i| top.get(*i).copied()).collect();
        if picked.len() == parts.len() {
            let query = picked.join(" ");
            if !out.contains(&query) {
                out.push(query);
            }
        }
    };
    add(&[0, 1]);
    add(&[0, 2]);
    add(&[1, 2]);
    add(&[0, 3]);
    add(&[0]);
    out.truncate(MAX_QUERIES);
    out
}

pub fn evidence(transcript: &str, terms: &[String]) -> Value {
    let lower = transcript.to_lowercase();
    let matched: Vec<&String> = terms
        .iter()
        .filter(|t| lower.contains(t.as_str()))
        .collect();
    let excerpts: Vec<String> = sentences(transcript)
        .into_iter()
        .filter(|s| {
            let s = s.to_lowercase();
            terms.iter().any(|t| s.contains(t.as_str()))
        })
        .take(3)
        .map(|s| cut(s, EXCERPT_CHARS))
        .collect();
    json!({ "matched_terms": matched, "excerpts": excerpts })
}

const HEALTH: &[&str] = &[
    "cure",
    "cures",
    "cured",
    "heal",
    "heals",
    "disease",
    "cancer",
    "diabetes",
    "weight",
    "fat",
    "diet",
    "supplement",
    "supplements",
    "vitamin",
    "vitamins",
    "doctor",
    "doctors",
    "medical",
    "medicine",
    "treatment",
    "symptom",
    "symptoms",
    "anxiety",
    "depression",
    "hormone",
    "hormones",
    "detox",
    "blood",
    "cholesterol",
    "calories",
    "health",
    "healthy",
    "cura",
    "curar",
    "doença",
    "emagrecer",
    "emagrecimento",
    "dieta",
    "suplemento",
    "vitamina",
    "médico",
    "remédio",
    "tratamento",
    "sintoma",
    "ansiedade",
    "depressão",
    "hormônio",
    "saúde",
];

const FINANCE: &[&str] = &[
    "invest",
    "investing",
    "investment",
    "investments",
    "profit",
    "profits",
    "income",
    "money",
    "earn",
    "earned",
    "earning",
    "returns",
    "stock",
    "stocks",
    "crypto",
    "bitcoin",
    "loan",
    "debt",
    "credit",
    "interest",
    "tax",
    "taxes",
    "rich",
    "salary",
    "passive",
    "revenue",
    "investir",
    "investimento",
    "lucro",
    "renda",
    "dinheiro",
    "ganhar",
    "ações",
    "cripto",
    "empréstimo",
    "dívida",
    "juros",
    "imposto",
    "salário",
    "rico",
    "faturamento",
];

pub fn claim_flags(text: &str) -> Vec<Value> {
    sentences(text)
        .into_iter()
        .filter_map(|sentence| {
            let lower = sentence.to_lowercase();
            let tokens: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric()).collect();
            let mut reasons = Vec::new();
            if sentence
                .chars()
                .any(|c| c.is_ascii_digit() || matches!(c, '%' | '$' | '€' | '£'))
            {
                reasons.push("number");
            }
            if tokens.iter().any(|t| HEALTH.contains(t)) {
                reasons.push("health");
            }
            if tokens.iter().any(|t| FINANCE.contains(t)) {
                reasons.push("finance");
            }
            (!reasons.is_empty())
                .then(|| json!({ "sentence": cut(sentence, EXCERPT_CHARS), "reasons": reasons }))
        })
        .take(8)
        .collect()
}

const BIO_HOSTS: &[(&str, Option<&str>)] = &[
    ("linktr.ee", Some("linktree")),
    ("komi.io", Some("komi")),
    ("pillar.io", Some("pillar")),
    ("lnk.bio", Some("linkbio")),
    ("link.me", Some("linkme")),
    ("beacons.ai", None),
    ("stan.store", None),
    ("bio.site", None),
    ("msha.ke", None),
    ("campsite.bio", None),
];

fn email_pattern() -> Option<&'static Regex> {
    static RE: OnceLock<Option<Regex>> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}").ok()
    })
    .as_ref()
}

fn link_pattern() -> Option<&'static Regex> {
    static RE: OnceLock<Option<Regex>> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)(?:https?://|www\.)[^\s<>]+|\b(?:linktr\.ee|komi\.io|pillar\.io|lnk\.bio|link\.me|beacons\.ai|stan\.store|bio\.site|msha\.ke|campsite\.bio)/[^\s<>]+").ok()
    })
    .as_ref()
}

pub fn emails_in(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let Some(pattern) = email_pattern() else {
        return out;
    };
    for found in pattern.find_iter(text) {
        let email = found.as_str().trim_end_matches('.').to_lowercase();
        let asset = ["png", "jpg", "jpeg", "gif", "webp", "svg"]
            .iter()
            .any(|ext| email.ends_with(&format!(".{ext}")));
        if !asset && !out.contains(&email) {
            out.push(email);
        }
    }
    out
}

fn host_of(link: &str) -> String {
    url::Url::parse(link)
        .ok()
        .and_then(|u| {
            u.host_str()
                .map(|h| h.trim_start_matches("www.").to_lowercase())
        })
        .unwrap_or_default()
}

pub fn bio_link_endpoint(link: &str) -> Option<&'static str> {
    let host = host_of(link);
    BIO_HOSTS
        .iter()
        .find(|(known, _)| host == *known)
        .and_then(|(_, endpoint)| *endpoint)
}

pub fn contacts(bio: &str, bio_link: Option<&str>) -> Value {
    let mut links: Vec<String> = Vec::new();
    let mut add = |raw: &str| {
        let trimmed = raw.trim().trim_end_matches(['.', ',', ';', ')', '!']);
        if trimmed.is_empty() {
            return;
        }
        let link = if trimmed.to_lowercase().starts_with("http") {
            trimmed.to_string()
        } else {
            format!("https://{trimmed}")
        };
        if !links.contains(&link) {
            links.push(link);
        }
    };
    if let Some(link) = bio_link {
        add(link);
    }
    if let Some(pattern) = link_pattern() {
        for found in pattern.find_iter(bio) {
            add(found.as_str());
        }
    }
    let link_in_bio = links
        .iter()
        .find(|link| BIO_HOSTS.iter().any(|(known, _)| host_of(link) == *known))
        .cloned();
    json!({
        "emails": emails_in(bio),
        "links": links,
        "link_in_bio": link_in_bio,
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub handle: String,
    pub queries: Vec<String>,
    pub matched: Vec<Video>,
}

pub fn rank(found: Vec<(String, Video)>, max: usize) -> (Vec<Candidate>, usize) {
    let mut out: Vec<Candidate> = Vec::new();
    for (query, video) in found {
        if video.handle.is_empty() {
            continue;
        }
        let at = match out.iter().position(|c| c.handle == video.handle) {
            Some(at) => at,
            None => {
                out.push(Candidate {
                    handle: video.handle.clone(),
                    queries: Vec::new(),
                    matched: Vec::new(),
                });
                out.len() - 1
            }
        };
        let candidate = &mut out[at];
        if !candidate.queries.contains(&query) {
            candidate.queries.push(query);
        }
        if !candidate.matched.iter().any(|v| v.id == video.id) {
            candidate.matched.push(video);
        }
    }
    let plays = |c: &Candidate| c.matched.iter().filter_map(|v| v.plays).sum::<u64>();
    out.sort_by(|a, b| {
        b.matched
            .len()
            .cmp(&a.matched.len())
            .then(plays(b).cmp(&plays(a)))
    });
    for candidate in &mut out {
        candidate
            .matched
            .sort_by_key(|v| std::cmp::Reverse(v.plays.unwrap_or(0)));
    }
    let total = out.len();
    out.truncate(max);
    (out, total)
}

fn transcript_of(body: &Value) -> Option<String> {
    body["transcript"]
        .as_str()
        .map(vtt_text)
        .filter(|t| !t.trim().is_empty())
}

fn code_of(error: &str) -> &str {
    error.split(':').next().unwrap_or("ERROR")
}

fn bounded(a: &Value, key: &str, default: u64, min: u64, max: u64) -> usize {
    a[key].as_u64().unwrap_or(default).clamp(min, max) as usize
}

async fn feed(handle: &str, sample: usize, region: Option<&str>) -> Result<Vec<Video>, String> {
    let mut all: Vec<Video> = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..FEED_PAGES {
        let body = client::get(
            "tiktok_profile_videos",
            json!({"handle": handle, "sort_by": "latest", "include_pinned": false, "region": region, "max_cursor": cursor}),
        )
        .await?;
        let page = videos(&body)?;
        let empty = page.is_empty();
        for video in page {
            if !all.iter().any(|v| v.id == video.id) {
                all.push(video);
            }
        }
        cursor = match &body["max_cursor"] {
            Value::String(s) if !s.is_empty() => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        };
        if all.len() >= sample || empty || cursor.is_none() || body["has_more"] == json!(false) {
            break;
        }
    }
    Ok(all)
}

async fn transcript(url: &str, ai: bool) -> Result<Option<String>, String> {
    let body = client::get(
        "tiktok_video_transcript",
        json!({"url": url, "use_ai_as_fallback": ai.then_some("true")}),
    )
    .await?;
    Ok(transcript_of(&body))
}

struct CreatorOptions {
    sample: usize,
    transcripts: usize,
    ai: bool,
    follow_bio_link: bool,
    region: Option<String>,
    terms: Vec<String>,
}

fn matched_row(
    video: &Video,
    median: Option<f64>,
    found: Option<&Result<Option<String>, String>>,
    terms: &[String],
) -> Value {
    let lift = video.plays.and_then(|p| stats::lift(p as f64, median));
    let transcript = match found {
        Some(Ok(Some(text))) => {
            let mut shown = evidence(text, terms);
            shown["available"] = json!(true);
            shown
        }
        Some(Ok(None)) => json!({"available": false, "reason": "no_captions"}),
        Some(Err(error)) => json!({"available": false, "reason": code_of(error)}),
        None => json!({"available": false, "reason": "not_requested"}),
    };
    json!({
        "url": video.url,
        "caption": cut(&video.caption, 300),
        "created": video.created.and_then(iso),
        "views": video.plays,
        "likes": video.likes,
        "comments": video.comments,
        "shares": video.shares,
        "lift_vs_own_median": lift,
        "lift_label": stats::lift_label(lift),
        "transcript": transcript,
    })
}

async fn creator(candidate: Candidate, options: &CreatorOptions) -> Value {
    let handle = candidate.handle.as_str();
    let wanted: Vec<&Video> = candidate.matched.iter().take(options.transcripts).collect();
    let (profile, recent_videos, transcripts) = tokio::join!(
        client::get("tiktok_profile", json!({ "handle": handle })),
        feed(handle, options.sample, options.region.as_deref()),
        futures::future::join_all(wanted.iter().map(|v| transcript(&v.url, options.ai))),
    );
    let mut missing: Vec<String> = Vec::new();
    let user = profile.as_ref().map_or(&NULL, |p| &p["user"]);
    if let Err(error) = &profile {
        missing.push(format!("profile_unavailable: {}", code_of(error)));
    }
    let recent_videos = recent_videos.unwrap_or_else(|error| {
        missing.push(format!("recent_videos_unavailable: {}", code_of(&error)));
        Vec::new()
    });
    let recent = creator_stats(&recent_videos, options.sample);
    let got = recent["sample"].as_u64().unwrap_or(0) as usize;
    if got < options.sample {
        missing.push(format!(
            "sample_smaller_than_asked: {got} of {}",
            options.sample
        ));
    }
    let median = recent["views"]["median"].as_f64();
    let matched: Vec<Value> = candidate
        .matched
        .iter()
        .enumerate()
        .map(|(i, video)| {
            let found = transcripts.get(i);
            if matches!(found, Some(Ok(None)) | Some(Err(_))) {
                missing.push(format!("no_transcript: {}", video.url));
            }
            matched_row(video, median, found, &options.terms)
        })
        .collect();
    let bio = user["signature"].as_str().unwrap_or_default();
    let mut contact = contacts(bio, user["bioLink"]["link"].as_str());
    contact["source"] = json!("bio");
    let no_email = contact["emails"].as_array().is_none_or(Vec::is_empty);
    let link = contact["link_in_bio"].as_str().map(str::to_string);
    if let (true, true, Some(link)) = (no_email, options.follow_bio_link, link) {
        if let Some(endpoint) = bio_link_endpoint(&link) {
            match client::get(endpoint, json!({ "url": link })).await {
                Ok(page) => {
                    let found = emails_in(&page.to_string());
                    if !found.is_empty() {
                        contact["emails"] = json!(found);
                        contact["source"] = json!("link_in_bio");
                    }
                }
                Err(error) => missing.push(format!("bio_link_unavailable: {}", code_of(&error))),
            }
        }
    }
    if contact["emails"].as_array().is_none_or(Vec::is_empty)
        && contact["links"].as_array().is_none_or(Vec::is_empty)
    {
        missing.push("no_public_contact".into());
    }
    let followers = uint(
        profile
            .as_ref()
            .map_or(&NULL, |p| &p["stats"]["followerCount"]),
    )
    .or_else(|| recent_videos.iter().find_map(|v| v.author_followers))
    .or_else(|| candidate.matched.iter().find_map(|v| v.author_followers));
    let name = user["nickname"]
        .as_str()
        .map(str::to_string)
        .or_else(|| candidate.matched.first().map(|v| v.name.clone()));
    json!({
        "handle": handle,
        "profile_url": format!("https://www.tiktok.com/@{handle}"),
        "name": name,
        "verified": user["verified"].as_bool(),
        "bio": cut(bio, 400),
        "followers": followers,
        "matched_queries": candidate.queries,
        "recent": recent,
        "matched_videos": matched,
        "contact": contact,
        "missing": missing,
    })
}

pub async fn creators(a: &Value) -> Result<Value, String> {
    let queries = strings(&a["queries"], MAX_QUERIES, 100);
    let brief = a["brief"].as_str().unwrap_or_default().trim();
    let suggested = suggest_queries(brief);
    if queries.is_empty() {
        if suggested.is_empty() {
            return Err(fail(
                "INVALID_PARAMS",
                "pass queries, or a brief to get suggested queries",
                false,
                "write 2 to 5 short TikTok search queries from the campaign brief and pass them as queries",
                Value::Null,
            ));
        }
        return Ok(json!({
            "creators": [],
            "total": 0,
            "returned": 0,
            "truncated": false,
            "reason": "no_queries",
            "suggested_queries": suggested,
            "next_step": "nothing was searched and no credit was spent; review or rewrite suggested_queries and call again with queries",
            "credits_used": 0,
            "untrusted": true,
        }));
    }
    let options = CreatorOptions {
        sample: bounded(a, "sample", 10, 3, 30),
        transcripts: bounded(a, "transcripts_per_creator", 2, 0, 5),
        ai: a["ai_transcripts"].as_bool() == Some(true),
        follow_bio_link: a["follow_bio_link"].as_bool() != Some(false),
        region: a["region"].as_str().map(str::to_string),
        terms: terms(&queries),
    };
    let max_creators = bounded(a, "max_creators", 5, 1, 15);
    let before = client::credits_spent();
    let searches = futures::future::join_all(queries.iter().map(|query| {
        client::get(
            "tiktok_search_keyword",
            json!({"query": query, "region": options.region, "date_posted": a["date_posted"]}),
        )
    }))
    .await;
    let mut found: Vec<(String, Video)> = Vec::new();
    let mut failed: Vec<Value> = Vec::new();
    let mut first_error: Option<String> = None;
    for (query, result) in queries.iter().zip(searches) {
        match result.and_then(|body| videos(&body)) {
            Ok(list) => found.extend(list.into_iter().map(|v| (query.clone(), v))),
            Err(error) => {
                failed.push(json!({"query": query, "error": code_of(&error)}));
                first_error.get_or_insert(error);
            }
        }
    }
    if failed.len() == queries.len() {
        if let Some(error) = first_error {
            return Err(error);
        }
    }
    let (candidates, total) = rank(found, max_creators);
    let rows = futures::future::join_all(candidates.into_iter().map(|candidate| {
        let handle = candidate.handle.clone();
        let options = &options;
        async move {
            match tokio::time::timeout(CREATOR_DEADLINE, creator(candidate, options)).await {
                Ok(row) => row,
                Err(_) => json!({"handle": handle, "missing": ["timed_out"]}),
            }
        }
    }))
    .await;
    let mut out = json!({
        "queries": queries,
        "sample": options.sample,
        "creators": rows,
        "total": total,
        "returned": rows.len(),
        "truncated": total > rows.len(),
        "failed_queries": failed,
        "credits_used": client::credits_spent() - before,
        "caveats": CAVEATS,
        "guide": CREATORS_GUIDE,
        "untrusted": true,
    });
    if !suggested.is_empty() {
        out["suggested_queries"] = json!(suggested);
    }
    if total == 0 {
        out["reason"] = json!("no_videos_matched");
        out["next_step"] = json!("try broader or different queries");
    }
    Ok(out)
}

pub fn brief_row(
    url: &str,
    video: Option<&Video>,
    transcript: Option<&str>,
    cap: usize,
    error: Option<&str>,
) -> Value {
    let caption = video.map(|v| v.caption.as_str()).unwrap_or_default();
    let checked = transcript.unwrap_or(caption);
    let total = transcript.map(|t| t.chars().count());
    json!({
        "url": url,
        "author": video.map(|v| json!({"handle": v.handle, "name": v.name})),
        "created": video.and_then(|v| v.created).and_then(iso),
        "caption": cut(caption, 400),
        "stats": video.map(|v| json!({"views": v.plays, "likes": v.likes, "comments": v.comments, "shares": v.shares, "saves": v.saves})),
        "hook": transcript.and_then(hook),
        "transcript": transcript.map(|t| t.chars().take(cap).collect::<String>()),
        "transcript_chars": total,
        "transcript_truncated": total.is_some_and(|n| n > cap),
        "flags": {
            "no_transcript": transcript.is_none(),
            "needs_checking": claim_flags(checked),
            "checked_text": if transcript.is_some() { "transcript" } else { "caption" },
        },
        "error": error.map(code_of),
    })
}

async fn discovered(a: &Value, count: usize) -> Result<Vec<Video>, String> {
    let region = a["region"].as_str();
    let text = |key: &str| a[key].as_str().map(str::trim).filter(|s| !s.is_empty());
    let (mut list, by_plays) = if let Some(query) = text("query") {
        let body = client::get(
            "tiktok_search_keyword",
            json!({"query": query, "region": region}),
        )
        .await?;
        (videos(&body)?, true)
    } else if let Some(hashtag) = text("hashtag") {
        let body = client::get(
            "tiktok_search_hashtag",
            json!({"hashtag": hashtag.trim_start_matches('#'), "region": region}),
        )
        .await?;
        (videos(&body)?, true)
    } else if let Some(handle) = text("handle") {
        (
            feed(handle.trim_start_matches('@'), count, region).await?,
            false,
        )
    } else {
        return Err(fail(
            "INVALID_PARAMS",
            "pass urls, or one of query, hashtag or handle",
            false,
            "pass urls with the TikTok video links, or query/hashtag/handle plus count",
            Value::Null,
        ));
    };
    let mut seen: Vec<String> = Vec::new();
    list.retain(|v| {
        let fresh = !seen.contains(&v.id);
        seen.push(v.id.clone());
        fresh
    });
    if by_plays {
        list.sort_by_key(|v| std::cmp::Reverse(v.plays.unwrap_or(0)));
    }
    list.truncate(count);
    Ok(list)
}

pub async fn brief(a: &Value) -> Result<Value, String> {
    let cap = bounded(a, "transcript_chars", 1500, 200, 6000);
    let ai = a["ai_transcripts"].as_bool() == Some(true);
    let urls = strings(&a["urls"], 20, 500);
    let before = client::credits_spent();
    let rows: Vec<Value> = if urls.is_empty() {
        let list = discovered(a, bounded(a, "count", 5, 1, 20)).await?;
        let found = futures::future::join_all(list.iter().map(|v| transcript(&v.url, ai))).await;
        list.iter()
            .zip(found)
            .map(|(video, found)| match found {
                Ok(text) => brief_row(&video.url, Some(video), text.as_deref(), cap, None),
                Err(error) => brief_row(&video.url, Some(video), None, cap, Some(&error)),
            })
            .collect()
    } else {
        futures::future::join_all(urls.iter().map(|url| async move {
            let detail =
                client::get("tiktok_video", json!({"url": url, "get_transcript": true})).await;
            match detail {
                Ok(body) => {
                    let video = video(&body["aweme_detail"]);
                    let mut text = transcript_of(&body);
                    if text.is_none() && ai {
                        text = transcript(url, true).await.ok().flatten();
                    }
                    brief_row(url, video.as_ref(), text.as_deref(), cap, None)
                }
                Err(error) => brief_row(url, None, None, cap, Some(&error)),
            }
        }))
        .await
    };
    let without = rows
        .iter()
        .filter(|r| r["flags"]["no_transcript"] == json!(true))
        .count();
    let mut out = json!({
        "videos": rows,
        "total": rows.len(),
        "returned": rows.len(),
        "truncated": false,
        "without_transcript": without,
        "credits_used": client::credits_spent() - before,
        "caveats": [
            "Public data only; transcripts are TikTok captions (or AI transcription when ai_transcripts is on) and can be wrong.",
            "needs_checking marks sentences with numbers or health and finance words; it is a prompt to verify, not a verdict.",
            "A video without a transcript is represented by its caption only.",
        ],
        "guide": BRIEF_GUIDE,
        "untrusted": true,
    });
    if rows.is_empty() {
        out["reason"] = json!("no_videos_found");
        out["next_step"] = json!("try another query, hashtag or handle");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, handle: &str, plays: u64, created: i64) -> Value {
        json!({
            "aweme_id": id,
            "desc": "Home gym setup under $300",
            "statistics": {"play_count": plays, "digg_count": plays / 10, "comment_count": 5, "share_count": 5, "collect_count": 7},
            "video": {"duration": 173_675},
            "author": {"unique_id": handle, "nickname": "Name", "follower_count": 4_424_699, "signature": "bio"},
            "create_time": created,
            "create_time_utc": "2025-09-03T18:37:38.000Z",
            "url": format!("https://www.tiktok.com/@{handle}/video/{id}"),
        })
    }

    fn vid(id: &str, handle: &str, plays: u64, created: i64) -> Video {
        video(&item(id, handle, plays, created)).unwrap()
    }

    #[test]
    fn a_feed_item_and_a_nested_search_item_parse_to_the_same_video() {
        let flat = item("7517", "branttakes", 559_925, 1_750_214_725);
        let parsed = video(&flat).unwrap();
        assert_eq!(parsed.id, "7517");
        assert_eq!(parsed.url, "https://www.tiktok.com/@branttakes/video/7517");
        assert_eq!(parsed.plays, Some(559_925));
        assert_eq!(parsed.likes, Some(55_992));
        assert_eq!(parsed.saves, Some(7));
        assert_eq!(parsed.duration_s, Some(173));
        assert_eq!(parsed.created, Some(1_750_214_725));
        assert_eq!(parsed.author_followers, Some(4_424_699));
        assert_eq!(video(&json!({ "aweme_info": flat })), Some(parsed));
    }

    #[test]
    fn a_video_without_create_time_uses_the_utc_string_and_builds_its_url() {
        let mut raw = item("9", "ana", 10, 0);
        raw.as_object_mut().unwrap().remove("create_time");
        raw.as_object_mut().unwrap().remove("url");
        let parsed = video(&raw).unwrap();
        assert_eq!(parsed.created, Some(1_756_924_658));
        assert_eq!(parsed.url, "https://www.tiktok.com/@ana/video/9");
        assert_eq!(video(&json!({"desc": "no id"})), None);
    }

    #[test]
    fn video_lists_are_read_from_every_known_key_and_an_unknown_shape_is_reported() {
        let one = item("1", "a", 1, 1);
        assert_eq!(
            videos(&json!({"aweme_list": [one.clone()]})).unwrap().len(),
            1
        );
        assert_eq!(
            videos(&json!({"search_item_list": [one.clone(), {"x": 1}]}))
                .unwrap()
                .len(),
            1
        );
        assert_eq!(videos(&json!({"items": [one]})).unwrap().len(), 1);
        assert!(videos(&json!({"aweme_list": null})).unwrap().is_empty());
        assert!(videos(&json!({"data": []}))
            .unwrap_err()
            .starts_with("UPSTREAM_CHANGED"));
    }

    #[test]
    fn recent_stats_use_the_newest_videos_of_the_same_sample_size() {
        let list: Vec<Video> = (1..=12)
            .map(|n| {
                vid(
                    &n.to_string(),
                    "a",
                    if n == 12 { 100_000 } else { n * 100 },
                    1_750_000_000 + n as i64 * 86_400,
                )
            })
            .collect();
        let stats = creator_stats(&list, 10);
        assert_eq!(stats["sample_asked"], 10);
        assert_eq!(stats["sample"], 10);
        assert_eq!(stats["confidence"], "medium");
        assert_eq!(stats["views"]["median"], 750.0);
        assert_eq!(stats["views"]["mean"], 10_630.0);
        assert_eq!(stats["views"]["min"], 300.0);
        assert_eq!(stats["date_from"], "2025-06-18T15:06:40Z");
        assert_eq!(stats["date_to"], "2025-06-27T15:06:40Z");
        assert!(stats["engagement_rate_median"].as_f64().unwrap() > 0.1);
        let small = creator_stats(&list[..4], 10);
        assert_eq!(small["sample"], 4);
        assert_eq!(small["confidence"], "low");
        let none = creator_stats(&[], 10);
        assert_eq!(none["sample"], 0);
        assert!(none["views"]["median"].is_null());
        assert!(none["date_from"].is_null());
    }

    #[test]
    fn a_webvtt_transcript_becomes_plain_text() {
        let raw = "WEBVTT\n\n\n00:00:00.120 --> 00:00:01.840\nAlright, pizza review time.\n\n2\n00:00:01.841 --> 00:00:03.000\nOne bite, everybody knows the rules.\n";
        assert_eq!(
            vtt_text(raw),
            "Alright, pizza review time. One bite, everybody knows the rules."
        );
        assert_eq!(vtt_text("plain words"), "plain words");
        assert_eq!(transcript_of(&json!({"transcript": null})), None);
        assert_eq!(transcript_of(&json!({"transcript": "WEBVTT\n\n"})), None);
    }

    #[test]
    fn sentences_split_on_ends_and_keep_decimals_together() {
        assert_eq!(
            sentences("It costs 3.5 dollars. Really? Yes!\nNext line"),
            vec!["It costs 3.5 dollars.", "Really?", "Yes!", "Next line"]
        );
        assert_eq!(
            hook("Stop scrolling. This matters."),
            Some("Stop scrolling.".into())
        );
        assert_eq!(hook("  "), None);
    }

    #[test]
    fn queries_are_suggested_from_the_most_frequent_brief_keywords() {
        let brief = "We want creators for a home gym campaign: compact home gym equipment for small apartments, gym beginners.";
        let queries = suggest_queries(brief);
        assert_eq!(queries[0], "gym home");
        assert!(queries.len() >= 3 && queries.len() <= 5, "{queries:?}");
        assert!(queries
            .iter()
            .all(|q| !q.contains("creators") && !q.contains("campaign")));
        assert!(suggest_queries("").is_empty());
        assert_eq!(suggest_queries("skincare"), vec!["skincare"]);
        assert_eq!(
            terms(&["home gym".into(), "gym for beginners".into()]),
            vec!["home", "gym", "beginners"]
        );
    }

    #[test]
    fn transcript_evidence_quotes_the_sentences_that_match_the_terms() {
        let text = "Welcome back. This home gym cost me less than a membership. Subscribe. My gym routine is simple.";
        let found = evidence(text, &["gym".into(), "protein".into()]);
        assert_eq!(found["matched_terms"], json!(["gym"]));
        assert_eq!(
            found["excerpts"],
            json!([
                "This home gym cost me less than a membership.",
                "My gym routine is simple."
            ])
        );
        assert_eq!(
            evidence("Nothing here.", &["gym".into()])["excerpts"],
            json!([])
        );
    }

    #[test]
    fn claims_with_numbers_or_health_and_finance_words_need_checking() {
        let text = "I love mornings. This supplement cures anxiety. You can earn passive income fast. It grew 300% in a week.";
        let flags = claim_flags(text);
        assert_eq!(flags.len(), 3);
        assert_eq!(flags[0]["reasons"], json!(["health"]));
        assert_eq!(flags[1]["reasons"], json!(["finance"]));
        assert_eq!(flags[2]["reasons"], json!(["number"]));
        assert_eq!(flags[2]["sentence"], "It grew 300% in a week.");
        assert!(claim_flags("Just a nice day at the beach.").is_empty());
        assert!(claim_flags("The fatigue is real, interesting.").is_empty());
    }

    #[test]
    fn public_contacts_come_from_the_bio_and_the_bio_link() {
        let bio = "Fitness coach 💪 collabs: Ana.Silva@Example.com. more at linktr.ee/anasilva, www.ana.fit";
        let found = contacts(bio, Some("https://www.barstoolsports.com/bios/x"));
        assert_eq!(found["emails"], json!(["ana.silva@example.com"]));
        assert_eq!(
            found["links"],
            json!([
                "https://www.barstoolsports.com/bios/x",
                "https://linktr.ee/anasilva",
                "https://www.ana.fit"
            ])
        );
        assert_eq!(found["link_in_bio"], "https://linktr.ee/anasilva");
        let none = contacts("just vibes", None);
        assert_eq!(none["emails"], json!([]));
        assert_eq!(none["links"], json!([]));
        assert!(none["link_in_bio"].is_null());
    }

    #[test]
    fn link_in_bio_pages_map_to_their_endpoint_and_assets_are_not_emails() {
        assert_eq!(bio_link_endpoint("https://linktr.ee/ana"), Some("linktree"));
        assert_eq!(bio_link_endpoint("https://www.komi.io/ana"), Some("komi"));
        assert_eq!(bio_link_endpoint("https://lnk.bio/ana"), Some("linkbio"));
        assert_eq!(bio_link_endpoint("https://beacons.ai/ana"), None);
        assert_eq!(bio_link_endpoint("https://example.com/linktr.ee"), None);
        let page = json!({"links": [{"url": "mailto:Booking@ana.fit"}], "img": "https://cdn.x/logo@2x.png"}).to_string();
        assert_eq!(emails_in(&page), vec!["booking@ana.fit"]);
    }

    #[test]
    fn creators_are_ranked_by_matched_videos_then_views_without_duplicates() {
        let found = vec![
            ("home gym".to_string(), vid("1", "big", 900_000, 1)),
            ("home gym".to_string(), vid("2", "focused", 1_000, 1)),
            ("gym setup".to_string(), vid("3", "focused", 5_000, 1)),
            ("gym setup".to_string(), vid("2", "focused", 1_000, 1)),
            ("gym setup".to_string(), vid("4", "small", 10, 1)),
        ];
        let (ranked, total) = rank(found, 2);
        assert_eq!(total, 3);
        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].handle, "focused");
        assert_eq!(ranked[0].queries, vec!["home gym", "gym setup"]);
        assert_eq!(
            ranked[0]
                .matched
                .iter()
                .map(|v| v.id.as_str())
                .collect::<Vec<_>>(),
            vec!["3", "2"]
        );
        assert_eq!(ranked[1].handle, "big");
    }

    #[test]
    fn a_matched_video_row_keeps_the_url_beside_the_excerpt_and_says_why_one_is_missing() {
        let video = vid("1", "a", 1500, 1_750_000_000);
        let ok = Ok(Some("My home gym is tiny. Bye.".to_string()));
        let row = matched_row(&video, Some(500.0), Some(&ok), &["gym".into()]);
        assert_eq!(row["url"], "https://www.tiktok.com/@a/video/1");
        assert_eq!(row["lift_vs_own_median"], 3.0);
        assert_eq!(row["lift_label"], "strong");
        assert_eq!(row["transcript"]["available"], true);
        assert_eq!(
            row["transcript"]["excerpts"],
            json!(["My home gym is tiny."])
        );
        let none = matched_row(&video, None, Some(&Ok(None)), &[]);
        assert_eq!(
            none["transcript"],
            json!({"available": false, "reason": "no_captions"})
        );
        let failed = Err(fail("NOT_FOUND", "x", false, "y", Value::Null));
        assert_eq!(
            matched_row(&video, None, Some(&failed), &[])["transcript"]["reason"],
            "NOT_FOUND"
        );
        assert_eq!(
            matched_row(&video, None, None, &[])["transcript"]["reason"],
            "not_requested"
        );
    }

    #[test]
    fn a_brief_row_separates_the_hook_caps_the_transcript_and_flags_what_is_missing() {
        let video = vid("1", "a", 1500, 1_750_000_000);
        let text =
            "Stop wasting money on gym memberships. I saved 900 dollars this year. Here is how.";
        let row = brief_row(&video.url, Some(&video), Some(text), 40, None);
        assert_eq!(row["url"], video.url);
        assert_eq!(row["hook"], "Stop wasting money on gym memberships.");
        assert_eq!(row["transcript"].as_str().unwrap().chars().count(), 40);
        assert_eq!(row["transcript_truncated"], true);
        assert_eq!(row["transcript_chars"], text.chars().count());
        assert_eq!(row["flags"]["no_transcript"], false);
        assert_eq!(row["flags"]["checked_text"], "transcript");
        assert_eq!(row["flags"]["needs_checking"].as_array().unwrap().len(), 2);
        assert_eq!(row["stats"]["views"], 1500);
        let bare = brief_row(&video.url, Some(&video), None, 40, None);
        assert!(bare["hook"].is_null());
        assert!(bare["transcript"].is_null());
        assert_eq!(bare["flags"]["no_transcript"], true);
        assert_eq!(bare["flags"]["checked_text"], "caption");
        assert_eq!(
            bare["flags"]["needs_checking"][0]["reasons"],
            json!(["number"])
        );
        let failed = brief_row(
            "https://www.tiktok.com/@a/video/2",
            None,
            None,
            40,
            Some("NOT_FOUND: {}"),
        );
        assert_eq!(failed["error"], "NOT_FOUND");
        assert!(failed["author"].is_null());
    }

    #[tokio::test]
    async fn a_brief_alone_returns_suggestions_and_spends_nothing() {
        let out = creators(
            &json!({"brief": "Home gym equipment for small apartments, home gym beginners"}),
        )
        .await
        .unwrap();
        assert_eq!(out["reason"], "no_queries");
        assert_eq!(out["credits_used"], 0);
        assert!(!out["suggested_queries"].as_array().unwrap().is_empty());
        assert!(out["next_step"].as_str().unwrap().contains("queries"));
        assert!(creators(&json!({}))
            .await
            .unwrap_err()
            .starts_with("INVALID_PARAMS"));
    }
}
