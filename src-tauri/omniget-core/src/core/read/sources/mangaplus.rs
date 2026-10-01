use std::collections::HashMap;

use super::{Hit, Page, RemoteChapter, RemotePage};
use crate::core::read::protobuf::{fields, Field, Msg};

pub const API: &str = "https://jumpg-webapi.tokyo-cdn.com/api";

fn session() -> &'static str {
    static S: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    S.get_or_init(|| uuid::Uuid::new_v4().to_string())
}

pub fn mp_lang(lang: &str) -> &'static str {
    match lang.to_ascii_lowercase().as_str() {
        l if l.starts_with("pt") => "ptb",
        l if l.starts_with("es") => "esp",
        l if l.starts_with("fr") => "fra",
        l if l.starts_with("id") => "ind",
        l if l.starts_with("ru") => "rus",
        l if l.starts_with("th") => "tha",
        l if l.starts_with("vi") => "vie",
        l if l.starts_with("de") => "deu",
        _ => "eng",
    }
}

fn lang_of(code: u64) -> &'static str {
    match code {
        1 => "es",
        2 => "fr",
        3 => "id",
        4 => "pt-br",
        5 => "ru",
        6 => "th",
        7 => "de",
        9 => "vi",
        _ => "en",
    }
}

async fn call(client: &reqwest::Client, path: &str) -> Result<Vec<u8>, String> {
    let r = client
        .get(format!("{API}{path}"))
        .header("SESSION-TOKEN", session())
        .header("Origin", "https://mangaplus.shueisha.co.jp")
        .header("Referer", "https://mangaplus.shueisha.co.jp/")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !r.status().is_success() {
        return Err(format!("{} from MANGA Plus", r.status()));
    }
    Ok(r.bytes().await.map_err(|e| e.to_string())?.to_vec())
}

fn success(bytes: &[u8]) -> Result<HashMap<u32, Vec<Field<'_>>>, String> {
    let resp = fields(bytes);
    if let Some(err) = resp.msg(2) {
        let popup = err.msg(2).unwrap_or_default();
        let body = popup.str(2);
        return Err(if body.is_empty() {
            "MANGA Plus refused the request".into()
        } else {
            body
        });
    }
    resp.msg(1)
        .ok_or_else(|| "empty answer from MANGA Plus".into())
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Title {
    pub id: u64,
    pub name: String,
    pub author: String,
    pub cover: String,
    pub language: String,
    pub views: u64,
}

fn title(t: &HashMap<u32, Vec<Field<'_>>>) -> Title {
    Title {
        id: t.num(1),
        name: t.str(2),
        author: t.str(3),
        cover: t.str(4),
        language: lang_of(t.num(7)).into(),
        views: t.num(6),
    }
}

fn to_hit(t: &Title) -> Hit {
    Hit {
        source: "mangaplus".into(),
        key: t.id.to_string(),
        title: t.name.clone(),
        authors: t
            .author
            .split(" / ")
            .map(str::to_string)
            .filter(|a| !a.is_empty())
            .collect(),
        cover: (!t.cover.is_empty()).then(|| t.cover.clone()),
        language: t.language.clone(),
        kind: "series".into(),
        url: format!("https://mangaplus.shueisha.co.jp/titles/{}", t.id),
        extra: serde_json::json!({ "views": t.views, "streamOnly": true }),
        ..Default::default()
    }
}

pub async fn all_titles(client: &reqwest::Client) -> Result<Vec<Title>, String> {
    static CACHE: tokio::sync::Mutex<Option<(std::time::Instant, Vec<Title>)>> =
        tokio::sync::Mutex::const_new(None);
    let mut g = CACHE.lock().await;
    if let Some((t, v)) = g.as_ref() {
        if t.elapsed().as_secs() < 6 * 3600 {
            return Ok(v.clone());
        }
    }
    let bytes = call(client, "/title_list/allV2").await?;
    let ok = success(&bytes)?;
    let view = ok.msg(25).ok_or("no title list")?;
    let mut out = Vec::new();
    for group in view.msgs(1) {
        for t in group.msgs(2) {
            out.push(title(&t));
        }
    }
    *g = Some((std::time::Instant::now(), out.clone()));
    Ok(out)
}

pub async fn browse(
    client: &reqwest::Client,
    query: &str,
    mode: &str,
    lang: &str,
) -> Result<Page, String> {
    let want = match mp_lang(lang) {
        "ptb" => "pt-br",
        "esp" => "es",
        "fra" => "fr",
        "ind" => "id",
        "rus" => "ru",
        "tha" => "th",
        "vie" => "vi",
        "deu" => "de",
        _ => "en",
    };
    let hits: Vec<Hit> = match mode {
        "search" => {
            let q = query.to_lowercase();
            all_titles(client)
                .await?
                .iter()
                .filter(|t| t.language == want)
                .filter(|t| {
                    t.name.to_lowercase().contains(&q) || t.author.to_lowercase().contains(&q)
                })
                .map(to_hit)
                .collect()
        }
        "latest" => {
            let l = mp_lang(lang);
            let bytes = call(client, &format!("/web/web_homeV4?lang={l}&clang={l}")).await?;
            let ok = success(&bytes)?;
            let home = ok.msg(38).ok_or("no home view")?;
            let mut seen = std::collections::HashSet::new();
            let mut v = Vec::new();
            for g in home.msgs(2) {
                for entry in g.msgs(2) {
                    for ch in entry.msgs(3) {
                        if let Some(t) = ch.msg(1) {
                            let t = title(&t);
                            if seen.insert(t.id) {
                                v.push(to_hit(&t));
                            }
                        }
                    }
                }
            }
            v
        }
        _ => {
            let l = mp_lang(lang);
            let bytes = call(
                client,
                &format!("/title_list/rankingV2?lang={l}&type=hottest&clang={l}"),
            )
            .await?;
            let ok = success(&bytes)?;
            let view = ok.msg(37).ok_or("no ranking")?;
            view.msgs(3)
                .iter()
                .flat_map(|r| r.msgs(2))
                .map(|t| to_hit(&title(&t)))
                .collect()
        }
    };
    Ok(Page {
        total: Some(hits.len() as u64),
        next: None,
        hits,
    })
}

pub async fn details(
    client: &reqwest::Client,
    id: &str,
) -> Result<(Hit, Vec<RemoteChapter>), String> {
    let bytes = call(client, &format!("/title_detailV3?title_id={id}&clang=eng")).await?;
    let ok = success(&bytes)?;
    let v = ok.msg(8).ok_or("no title detail")?;
    let t = title(&v.msg(1).unwrap_or_default());
    let mut hit = to_hit(&t);
    hit.summary = v.str(3);
    let banner = v.str(2);
    if !banner.is_empty() && hit.cover.is_none() {
        hit.cover = Some(banner);
    }
    hit.subjects = v
        .msgs(31)
        .iter()
        .map(|g| g.str(1))
        .filter(|s| !s.is_empty())
        .collect();
    let note = v.str(7);
    hit.extra["viewingPeriod"] = serde_json::Value::String(note);
    let mut chapters = Vec::new();
    for g in v.msgs(28) {
        for list in [2u32, 4] {
            for c in g.msgs(list) {
                if !c.has(4) {
                    continue;
                }
                let name = c.str(3);
                let number = name
                    .trim_start_matches('#')
                    .trim_start_matches('0')
                    .parse()
                    .ok();
                chapters.push(RemoteChapter {
                    key: c.num(2).to_string(),
                    title: format!("{name} {}", c.str(4)).trim().to_string(),
                    number,
                    volume: None,
                    language: t.language.clone(),
                    scanlator: "Shueisha".into(),
                    published: Some(c.num(6) as i64 * 1000).filter(|x| *x > 0),
                    pages: None,
                    external_url: None,
                });
            }
        }
    }
    chapters.sort_by(|a, b| {
        a.number
            .partial_cmp(&b.number)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    chapters.dedup_by(|a, b| a.key == b.key);
    Ok((hit, chapters))
}

pub async fn pages(
    client: &reqwest::Client,
    chapter_id: &str,
    data_saver: bool,
) -> Result<Vec<RemotePage>, String> {
    let quality = if data_saver { "low" } else { "super_high" };
    let bytes = call(
        client,
        &format!(
            "/manga_viewer_v3?chapter_id={chapter_id}&split=no&img_quality={quality}&clang=eng"
        ),
    )
    .await?;
    let ok = success(&bytes)?;
    let viewer = ok.msg(10).ok_or("no viewer")?;
    let token = viewer.str(19);
    let mut out = Vec::new();
    for p in viewer.msgs(1) {
        let Some(mp) = p.msg(1) else { continue };
        let url = mp.str(1);
        if url.is_empty() {
            continue;
        }
        let key = mp.str(5);
        let mut headers = vec![(
            "Referer".to_string(),
            "https://mangaplus.shueisha.co.jp/".to_string(),
        )];
        if !token.is_empty() {
            headers.push(("Plus-Vw-Token".into(), token.clone()));
        }
        out.push(RemotePage {
            url,
            headers,
            xor_key: (!key.is_empty()).then_some(key),
        });
    }
    if out.is_empty() {
        return Err("this chapter isn't free to read right now".into());
    }
    Ok(out)
}
