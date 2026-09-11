//! Bulk-download de emotes e badges de um canal da Twitch.
//!
//! Quatro fontes, todas públicas e sem login:
//! - **Twitch**: GraphQL do site (`user.subscriptionProducts.emotes`,
//!   `user.broadcastBadges`, `emoteSet(id: "0")` e `badges` para os globais).
//! - **BTTV**: `api.betterttv.net/3/cached/...`
//! - **FFZ**: `api.frankerfacez.com/v1/...`
//! - **7TV**: `7tv.io/v3/...`
//!
//! Cada emote é baixado na maior resolução que o provedor oferece; animados
//! ficam como vieram (GIF na Twitch, WebP nos outros). O resultado é uma
//! pasta por provedor, um `index.json` e, se pedido, um contact-sheet HTML.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::anyhow;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::super::{report, sanitize_name, ProgressFn};
use super::gql::Gql;

const ID: &str = "tw-emotes";
/// Ordem de preferência quando o mesmo código existe em mais de um provedor.
const PRIORITY: [&str; 4] = ["twitch", "7tv", "bttv", "ffz"];

#[derive(Debug, Clone, Deserialize)]
pub struct Options {
    /// Login do canal ou link; vazio baixa só os conjuntos globais.
    #[serde(default)]
    pub channel: String,
    pub out_dir: String,
    #[serde(default = "yes")]
    pub twitch: bool,
    #[serde(default = "yes")]
    pub bttv: bool,
    #[serde(default = "yes")]
    pub ffz: bool,
    #[serde(default = "yes")]
    pub seventv: bool,
    /// Inclui os conjuntos globais de cada provedor.
    #[serde(default = "yes")]
    pub global: bool,
    /// Badges da Twitch (globais e do canal).
    #[serde(default = "yes")]
    pub badges: bool,
    #[serde(default = "yes")]
    pub contact_sheet: bool,
    /// Refaz o download de arquivos que já existem.
    #[serde(default)]
    pub overwrite: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Emote {
    pub code: String,
    pub provider: String,
    /// "emote" ou "badge"
    pub kind: String,
    /// "channel" ou "global"
    pub scope: String,
    pub id: String,
    pub url: String,
    pub animated: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub title: String,
    /// Caminho relativo dentro da pasta de saída (preenchido no download).
    pub file: String,
    /// Outros provedores que têm o mesmo código.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub also_in: Vec<String>,
}

impl Emote {
    fn new(provider: &str, scope: &str, code: &str, id: &str, url: &str, animated: bool) -> Self {
        Self {
            code: code.to_string(),
            provider: provider.to_string(),
            kind: "emote".to_string(),
            scope: scope.to_string(),
            id: id.to_string(),
            url: url.to_string(),
            animated,
            title: String::new(),
            file: String::new(),
            also_in: Vec::new(),
        }
    }

    fn badge(provider: &str, scope: &str, code: &str, title: &str, url: &str) -> Self {
        let mut e = Self::new(provider, scope, code, code, url, false);
        e.kind = "badge".to_string();
        e.title = title.to_string();
        e
    }

    fn key(&self) -> String {
        format!("{}\u{1}{}", self.kind, self.code)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderCount {
    pub provider: String,
    pub emotes: usize,
    pub badges: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Result {
    pub channel: String,
    pub channel_id: String,
    pub dir: String,
    pub index_path: String,
    pub sheet_path: Option<String>,
    pub total: usize,
    pub downloaded: usize,
    pub skipped: usize,
    pub failed: usize,
    pub duplicates: usize,
    pub by_provider: Vec<ProviderCount>,
    pub items: Vec<Emote>,
}

// ───────────────────────── parsing por provedor ─────────────────────────

/// `https://static-cdn.jtvnw.net/emoticons/v2/<id>/default/dark/3.0` é a
/// maior versão; `default` devolve GIF quando o emote é animado.
pub fn twitch_emote_url(id: &str) -> String {
    format!(
        "https://static-cdn.jtvnw.net/emoticons/v2/{}/default/dark/3.0",
        id
    )
}

/// `user { subscriptionProducts { emotes { id token assetType } } }`
pub fn parse_twitch_emotes(user: &Value, scope: &str) -> Vec<Emote> {
    let mut out = Vec::new();
    let sets = user
        .get("subscriptionProducts")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let direct = user
        .get("emotes")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let lists = sets
        .iter()
        .filter_map(|s| s.get("emotes").and_then(|v| v.as_array()).cloned())
        .chain(if direct.is_empty() {
            None
        } else {
            Some(direct)
        });
    for list in lists {
        for e in list {
            let (Some(id), Some(token)) = (
                e.get("id").and_then(|v| v.as_str()),
                e.get("token").and_then(|v| v.as_str()),
            ) else {
                continue;
            };
            let animated = e
                .get("assetType")
                .and_then(|v| v.as_str())
                .map(|s| s.eq_ignore_ascii_case("ANIMATED"))
                .unwrap_or(false);
            out.push(Emote::new(
                "twitch",
                scope,
                token,
                id,
                &twitch_emote_url(id),
                animated,
            ));
        }
    }
    out
}

/// `broadcastBadges` (canal) ou `badges` (globais).
pub fn parse_twitch_badges(list: &Value, scope: &str) -> Vec<Emote> {
    let Some(arr) = list.as_array() else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|b| {
            let set = b.get("setID").and_then(|v| v.as_str())?;
            let version = b.get("version").and_then(|v| v.as_str()).unwrap_or("1");
            let url = b.get("imageURL").and_then(|v| v.as_str())?;
            if set.is_empty() || url.is_empty() {
                return None;
            }
            let title = b.get("title").and_then(|v| v.as_str()).unwrap_or(set);
            Some(Emote::badge(
                "twitch",
                scope,
                &format!("{}-{}", set, version),
                title,
                url,
            ))
        })
        .collect()
}

/// BTTV: `channelEmotes` + `sharedEmotes` do canal, ou o array dos globais.
pub fn parse_bttv(v: &Value, scope: &str) -> Vec<Emote> {
    let mut lists: Vec<&Vec<Value>> = Vec::new();
    if let Some(arr) = v.as_array() {
        lists.push(arr);
    }
    for key in ["channelEmotes", "sharedEmotes"] {
        if let Some(arr) = v.get(key).and_then(|x| x.as_array()) {
            lists.push(arr);
        }
    }
    let mut out = Vec::new();
    for arr in lists {
        for e in arr {
            let (Some(id), Some(code)) = (
                e.get("id").and_then(|v| v.as_str()),
                e.get("code").and_then(|v| v.as_str()),
            ) else {
                continue;
            };
            let image_type = e
                .get("imageType")
                .and_then(|v| v.as_str())
                .unwrap_or("png")
                .to_lowercase();
            let animated = e
                .get("animated")
                .and_then(|v| v.as_bool())
                .unwrap_or(image_type == "gif");
            let ext = if animated && image_type != "gif" {
                "webp"
            } else {
                image_type.as_str()
            };
            let url = format!("https://cdn.betterttv.net/emote/{}/3x.{}", id, ext);
            out.push(Emote::new("bttv", scope, code, id, &url, animated));
        }
    }
    out
}

/// FFZ: `sets{ <id>: { emoticons: [...] } }` serve para sala e globais.
pub fn parse_ffz(v: &Value, scope: &str) -> Vec<Emote> {
    let Some(sets) = v.get("sets").and_then(|s| s.as_object()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for set in sets.values() {
        let Some(arr) = set.get("emoticons").and_then(|x| x.as_array()) else {
            continue;
        };
        for e in arr {
            let Some(code) = e.get("name").and_then(|v| v.as_str()) else {
                continue;
            };
            let id = e
                .get("id")
                .map(|v| v.to_string().trim_matches('"').to_string())
                .unwrap_or_default();
            let animated_urls = e.get("animated").and_then(|v| v.as_object());
            let animated = animated_urls.is_some();
            let urls = animated_urls
                .or_else(|| e.get("urls").and_then(|v| v.as_object()))
                .cloned()
                .unwrap_or_default();
            let Some(url) = biggest_by_number(&urls) else {
                continue;
            };
            let url = if url.starts_with("//") {
                format!("https:{}", url)
            } else {
                url
            };
            out.push(Emote::new("ffz", scope, code, &id, &url, animated));
        }
    }
    out
}

/// Badges de mod/VIP da sala no FFZ (quando o canal tem arte própria).
pub fn parse_ffz_room_badges(v: &Value, scope: &str) -> Vec<Emote> {
    let Some(room) = v.get("room") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (key, code) in [("mod_urls", "ffz-moderator"), ("vip_badge", "ffz-vip")] {
        let Some(urls) = room.get(key).and_then(|x| x.as_object()) else {
            continue;
        };
        if let Some(url) = biggest_by_number(urls) {
            let url = if url.starts_with("//") {
                format!("https:{}", url)
            } else {
                url
            };
            out.push(Emote::badge("ffz", scope, code, code, &url));
        }
    }
    out
}

/// 7TV: o objeto `emote_set` (de `/users/twitch/<id>` ou `/emote-sets/<id>`).
pub fn parse_7tv(set: &Value, scope: &str) -> Vec<Emote> {
    let Some(arr) = set.get("emotes").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in arr {
        let Some(code) = e.get("name").and_then(|v| v.as_str()) else {
            continue;
        };
        let id = e.get("id").and_then(|v| v.as_str()).unwrap_or_default();
        let data = e.get("data").unwrap_or(e);
        let animated = data
            .get("animated")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let host = data.get("host");
        let base = host
            .and_then(|h| h.get("url"))
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let files = host
            .and_then(|h| h.get("files"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let Some(name) = best_7tv_file(&files) else {
            continue;
        };
        if base.is_empty() {
            continue;
        }
        let base = if base.starts_with("//") {
            format!("https:{}", base)
        } else {
            base.to_string()
        };
        out.push(Emote::new(
            "7tv",
            scope,
            code,
            id,
            &format!("{}/{}", base, name),
            animated,
        ));
    }
    out
}

/// Pega o maior arquivo WebP (o formato que todo mundo abre); se não houver,
/// o maior de qualquer formato.
pub fn best_7tv_file(files: &[Value]) -> Option<String> {
    let pick = |only_webp: bool| -> Option<(u64, String)> {
        files
            .iter()
            .filter_map(|f| {
                let name = f.get("name").and_then(|v| v.as_str())?;
                if only_webp && !name.ends_with(".webp") {
                    return None;
                }
                if name.ends_with(".avif") {
                    return None;
                }
                let w = f.get("width").and_then(|v| v.as_u64()).unwrap_or(0);
                Some((w, name.to_string()))
            })
            .max_by_key(|(w, _)| *w)
    };
    pick(true).or_else(|| pick(false)).map(|(_, n)| n)
}

/// Mapa `{"1": url, "2": url, "4": url}` → a maior chave numérica.
fn biggest_by_number(map: &serde_json::Map<String, Value>) -> Option<String> {
    map.iter()
        .filter_map(|(k, v)| Some((k.parse::<u32>().ok()?, v.as_str()?.to_string())))
        .max_by_key(|(k, _)| *k)
        .map(|(_, v)| v)
}

// ───────────────────────── dedupe e nomes ─────────────────────────

/// Mantém um emote por código, na ordem de `PRIORITY`, anotando em quais
/// outros provedores o mesmo código apareceu.
pub fn dedupe(items: Vec<Emote>) -> (Vec<Emote>, usize) {
    let rank = |p: &str| {
        PRIORITY
            .iter()
            .position(|x| *x == p)
            .unwrap_or(PRIORITY.len())
    };
    let mut kept: Vec<Emote> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut dupes = 0usize;
    for item in items {
        let key = item.key();
        match index.get(&key).copied() {
            None => {
                index.insert(key, kept.len());
                kept.push(item);
            }
            Some(i) => {
                dupes += 1;
                if rank(&item.provider) < rank(&kept[i].provider) {
                    let old = std::mem::replace(&mut kept[i], item);
                    let mut also = old.also_in;
                    also.push(old.provider);
                    kept[i].also_in = also;
                } else if !kept[i].also_in.contains(&item.provider)
                    && kept[i].provider != item.provider
                {
                    kept[i].also_in.push(item.provider.clone());
                }
            }
        }
    }
    for e in &mut kept {
        e.also_in.sort();
        e.also_in.dedup();
    }
    (kept, dupes)
}

/// Extensão pelo fim da URL; se a URL não disser, pelo `content-type`.
pub fn ext_from(url: &str, content_type: Option<&str>) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let tail = path.rsplit('/').next().unwrap_or("");
    if let Some((_, ext)) = tail.rsplit_once('.') {
        let ext = ext.to_lowercase();
        if matches!(
            ext.as_str(),
            "png" | "gif" | "webp" | "jpg" | "jpeg" | "avif"
        ) {
            return ext;
        }
    }
    match content_type.unwrap_or("").split(';').next().unwrap_or("") {
        "image/gif" => "gif".into(),
        "image/webp" => "webp".into(),
        "image/jpeg" => "jpg".into(),
        "image/avif" => "avif".into(),
        _ => "png".into(),
    }
}

/// Nome de arquivo único dentro da pasta do provedor (o `taken` é
/// minúsculo porque macOS e Windows não distinguem `Kappa` de `kappa`).
pub fn unique_name(code: &str, id: &str, ext: &str, taken: &mut HashSet<String>) -> String {
    let base = sanitize_name(code);
    let mut name = format!("{}.{}", base, ext);
    if taken.contains(&name.to_lowercase()) {
        let suffix = sanitize_name(id);
        let suffix = suffix.chars().take(12).collect::<String>();
        name = format!("{}-{}.{}", base, suffix, ext);
    }
    let mut n = 2;
    while taken.contains(&name.to_lowercase()) {
        name = format!("{}-{}.{}", base, n, ext);
        n += 1;
    }
    taken.insert(name.to_lowercase());
    name
}

/// Contact-sheet: uma página que abre offline e mostra tudo o que baixou.
pub fn contact_sheet(channel: &str, items: &[Emote]) -> String {
    let mut cards = String::new();
    for e in items {
        if e.file.is_empty() {
            continue;
        }
        let src = e.file.replace('\\', "/");
        cards.push_str(&format!(
            "<figure><img loading=\"lazy\" src=\"{}\" alt=\"{}\"><figcaption>{}<span>{}{}</span></figcaption></figure>\n",
            html_escape(&src),
            html_escape(&e.code),
            html_escape(&e.code),
            html_escape(&e.provider),
            if e.animated { " · anim" } else { "" }
        ));
    }
    format!(
        "<!doctype html><meta charset=\"utf-8\"><title>Emotes {ch}</title>\n\
<style>body{{background:#0e0e10;color:#efeff1;font:14px -apple-system,system-ui,sans-serif;margin:24px}}\n\
h1{{font-size:18px;font-weight:600}}\n\
.grid{{display:grid;grid-template-columns:repeat(auto-fill,minmax(112px,1fr));gap:12px;margin-top:16px}}\n\
figure{{margin:0;background:#18181b;border-radius:10px;padding:10px;text-align:center}}\n\
img{{height:56px;max-width:100%;object-fit:contain}}\n\
figcaption{{margin-top:8px;font-size:11px;overflow-wrap:anywhere}}\n\
figcaption span{{display:block;color:#adadb8;font-size:10px}}</style>\n\
<h1>{ch} · {n} emotes</h1>\n<div class=\"grid\">\n{cards}</div>\n",
        ch = html_escape(channel),
        n = items.len(),
        cards = cards
    )
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

// ───────────────────────── coleta e download ─────────────────────────

async fn json_get(http: &reqwest::Client, url: &str) -> anyhow::Result<Value> {
    let resp = http.get(url).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("{} respondeu HTTP {}", url, resp.status());
    }
    Ok(resp.json::<Value>().await?)
}

pub async fn run(opts: &Options, p: &ProgressFn) -> anyhow::Result<Result> {
    let gql = Gql::new()?;
    let http = super::super::client()?;

    let login = if opts.channel.trim().is_empty() {
        None
    } else {
        Some(
            super::gql::parse_channel(&opts.channel)
                .ok_or_else(|| anyhow!("não reconheci esse canal: {}", opts.channel))?,
        )
    };

    report(p, ID, "progress", 0, None, Some("lendo o canal".into()));
    let channel = match &login {
        Some(l) => Some(gql.channel(l).await?),
        None => None,
    };
    let channel_id = channel.as_ref().map(|c| c.id.clone()).unwrap_or_default();
    let label = channel
        .as_ref()
        .map(|c| c.login.clone())
        .unwrap_or_else(|| "global".to_string());

    let mut items: Vec<Emote> = Vec::new();

    if opts.twitch {
        if let Some(l) = &login {
            report(p, ID, "progress", 0, None, Some("emotes da Twitch".into()));
            let q = format!(
                r#"{{ user(login: "{}") {{ subscriptionProducts {{ emotes {{ id token assetType }} }} broadcastBadges {{ setID version title imageURL(size: QUADRUPLE) }} }} }}"#,
                l.replace('"', "")
            );
            match gql.query(&q).await {
                Ok(data) => {
                    if let Some(user) = data.get("user").filter(|v| !v.is_null()) {
                        items.extend(parse_twitch_emotes(user, "channel"));
                        if opts.badges {
                            if let Some(b) = user.get("broadcastBadges") {
                                items.extend(parse_twitch_badges(b, "channel"));
                            }
                        }
                    }
                }
                Err(e) => tracing::warn!("emotes do canal na Twitch falharam: {}", e),
            }
        }
        if opts.global {
            report(p, ID, "progress", 0, None, Some("emotes globais".into()));
            let q = if opts.badges {
                r#"{ emoteSet(id: "0") { emotes { id token assetType } } badges { setID version title imageURL(size: QUADRUPLE) } }"#
            } else {
                r#"{ emoteSet(id: "0") { emotes { id token assetType } } }"#
            };
            match gql.query(q).await {
                Ok(data) => {
                    if let Some(set) = data.get("emoteSet").filter(|v| !v.is_null()) {
                        items.extend(parse_twitch_emotes(set, "global"));
                    }
                    if let Some(b) = data.get("badges") {
                        items.extend(parse_twitch_badges(b, "global"));
                    }
                }
                Err(e) => tracing::warn!("emotes globais da Twitch falharam: {}", e),
            }
        }
    }

    if opts.bttv {
        if !channel_id.is_empty() {
            let url = format!(
                "https://api.betterttv.net/3/cached/users/twitch/{}",
                channel_id
            );
            match json_get(&http, &url).await {
                Ok(v) => items.extend(parse_bttv(&v, "channel")),
                Err(e) => tracing::warn!("BTTV do canal falhou: {}", e),
            }
        }
        if opts.global {
            match json_get(&http, "https://api.betterttv.net/3/cached/emotes/global").await {
                Ok(v) => items.extend(parse_bttv(&v, "global")),
                Err(e) => tracing::warn!("BTTV global falhou: {}", e),
            }
        }
    }

    if opts.ffz {
        if let Some(l) = &login {
            let url = format!("https://api.frankerfacez.com/v1/room/{}", l);
            match json_get(&http, &url).await {
                Ok(v) => {
                    items.extend(parse_ffz(&v, "channel"));
                    if opts.badges {
                        items.extend(parse_ffz_room_badges(&v, "channel"));
                    }
                }
                Err(e) => tracing::warn!("FFZ do canal falhou: {}", e),
            }
        }
        if opts.global {
            match json_get(&http, "https://api.frankerfacez.com/v1/set/global").await {
                Ok(v) => items.extend(parse_ffz(&v, "global")),
                Err(e) => tracing::warn!("FFZ global falhou: {}", e),
            }
        }
    }

    if opts.seventv {
        if !channel_id.is_empty() {
            let url = format!("https://7tv.io/v3/users/twitch/{}", channel_id);
            match json_get(&http, &url).await {
                Ok(v) => {
                    if let Some(set) = v.get("emote_set") {
                        items.extend(parse_7tv(set, "channel"));
                    }
                }
                Err(e) => tracing::warn!("7TV do canal falhou: {}", e),
            }
        }
        if opts.global {
            match json_get(&http, "https://7tv.io/v3/emote-sets/global").await {
                Ok(v) => items.extend(parse_7tv(&v, "global")),
                Err(e) => tracing::warn!("7TV global falhou: {}", e),
            }
        }
    }

    if items.is_empty() {
        return Err(anyhow!(
            "nenhum emote encontrado — confira o canal e deixe pelo menos um provedor ligado"
        ));
    }

    let (mut items, duplicates) = dedupe(items);
    items.sort_by(|a, b| {
        a.kind
            .cmp(&b.kind)
            .then_with(|| a.provider.cmp(&b.provider))
            .then_with(|| a.code.to_lowercase().cmp(&b.code.to_lowercase()))
    });

    // Nome de arquivo e pasta antes de baixar, para o download ser paralelo.
    let root = PathBuf::from(&opts.out_dir).join(format!("{}-emotes", sanitize_name(&label)));
    let mut taken: HashMap<String, HashSet<String>> = HashMap::new();
    let mut planned: Vec<(usize, PathBuf)> = Vec::new();
    for (i, e) in items.iter_mut().enumerate() {
        let folder = if e.kind == "badge" {
            format!("badges-{}", e.provider)
        } else {
            e.provider.clone()
        };
        let ext = ext_from(&e.url, None);
        let ext = if e.provider == "twitch" && e.kind == "emote" {
            // A CDN v2 não põe extensão na URL: animado é GIF, o resto PNG.
            if e.animated {
                "gif".to_string()
            } else {
                "png".to_string()
            }
        } else {
            ext
        };
        let taken_here = taken.entry(folder.clone()).or_default();
        let name = unique_name(&e.code, &e.id, &ext, taken_here);
        e.file = format!("{}/{}", folder, name);
        planned.push((i, root.join(&folder).join(&name)));
    }

    for folder in taken.keys() {
        std::fs::create_dir_all(root.join(folder))?;
    }

    let total = planned.len() as u64;
    report(p, ID, "started", 0, Some(total), None);
    let done = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let urls: Vec<(usize, PathBuf, String)> = planned
        .into_iter()
        .map(|(i, path)| (i, path, items[i].url.clone()))
        .collect();

    let results: Vec<(usize, std::result::Result<bool, String>)> = futures::stream::iter(urls)
        .map(|(i, path, url)| {
            let http = http.clone();
            let done = done.clone();
            let p = p.clone();
            async move {
                let outcome = if path.exists() && !opts.overwrite {
                    Ok(false)
                } else {
                    fetch_one(&http, &url, &path).await.map(|_| true)
                };
                let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                report(
                    &p,
                    ID,
                    "progress",
                    n,
                    Some(total),
                    path.file_name().map(|f| f.to_string_lossy().to_string()),
                );
                (i, outcome.map_err(|e| e.to_string()))
            }
        })
        .buffer_unordered(8)
        .collect()
        .await;

    let mut downloaded = 0usize;
    let mut skipped = 0usize;
    let mut failed_idx: Vec<usize> = Vec::new();
    for (i, outcome) in results {
        match outcome {
            Ok(true) => downloaded += 1,
            Ok(false) => skipped += 1,
            Err(e) => {
                tracing::warn!("emote {} falhou: {}", items[i].code, e);
                failed_idx.push(i);
            }
        }
    }
    let failed = failed_idx.len();
    for i in failed_idx {
        items[i].file = String::new();
    }

    let mut by: HashMap<String, (usize, usize)> = HashMap::new();
    for e in &items {
        let slot = by.entry(e.provider.clone()).or_insert((0, 0));
        if e.kind == "badge" {
            slot.1 += 1;
        } else {
            slot.0 += 1;
        }
    }
    let mut by_provider: Vec<ProviderCount> = by
        .into_iter()
        .map(|(provider, (emotes, badges))| ProviderCount {
            provider,
            emotes,
            badges,
        })
        .collect();
    by_provider.sort_by(|a, b| a.provider.cmp(&b.provider));

    let index = json!({
        "channel": label,
        "channel_id": channel_id,
        "generated_at": chrono::Utc::now().to_rfc3339(),
        "duplicates": duplicates,
        "by_provider": by_provider,
        "items": items,
    });
    let index_path = root.join("index.json");
    std::fs::write(&index_path, serde_json::to_vec_pretty(&index)?)?;

    let sheet_path = if opts.contact_sheet {
        let path = root.join("contact-sheet.html");
        std::fs::write(&path, contact_sheet(&label, &items))?;
        Some(path.to_string_lossy().to_string())
    } else {
        None
    };

    report(p, ID, "done", total, Some(total), None);
    Ok(Result {
        channel: label,
        channel_id,
        dir: root.to_string_lossy().to_string(),
        index_path: index_path.to_string_lossy().to_string(),
        sheet_path,
        total: items.len(),
        downloaded,
        skipped,
        failed,
        duplicates,
        by_provider,
        items,
    })
}

async fn fetch_one(http: &reqwest::Client, url: &str, path: &Path) -> anyhow::Result<()> {
    let mut wait = std::time::Duration::from_millis(500);
    let mut last = String::new();
    for attempt in 0..3 {
        if attempt > 0 {
            tokio::time::sleep(wait).await;
            wait *= 2;
        }
        let resp = match http.get(url).send().await {
            Ok(r) => r,
            Err(e) => {
                last = e.to_string();
                continue;
            }
        };
        let status = resp.status();
        if status.as_u16() == 429 || status.is_server_error() {
            last = format!("HTTP {}", status);
            continue;
        }
        if !status.is_success() {
            anyhow::bail!("HTTP {}", status);
        }
        let bytes = resp.bytes().await?;
        if bytes.is_empty() {
            anyhow::bail!("arquivo vazio");
        }
        std::fs::write(path, &bytes)?;
        return Ok(());
    }
    Err(anyhow!("{}", last))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_emotes_e_badges_da_twitch() {
        let user = json!({
            "subscriptionProducts": [
                { "emotes": [
                    { "id": "emotesv2_abc", "token": "xqcSlam", "assetType": "ANIMATED" },
                    { "id": "305535174", "token": "xqcOmega", "assetType": "STATIC" }
                ]}
            ],
            "broadcastBadges": [
                { "setID": "subscriber", "version": "0", "title": "Subscriber",
                  "imageURL": "https://static-cdn.jtvnw.net/badges/v1/abc/3" }
            ]
        });
        let emotes = parse_twitch_emotes(&user, "channel");
        assert_eq!(emotes.len(), 2);
        assert!(emotes[0].animated);
        assert_eq!(
            emotes[0].url,
            "https://static-cdn.jtvnw.net/emoticons/v2/emotesv2_abc/default/dark/3.0"
        );
        assert!(!emotes[1].animated);
        let badges = parse_twitch_badges(
            user.get("broadcastBadges").unwrap_or(&Value::Null),
            "channel",
        );
        assert_eq!(badges.len(), 1);
        assert_eq!(badges[0].code, "subscriber-0");
        assert_eq!(badges[0].kind, "badge");
    }

    #[test]
    fn le_bttv_do_canal_e_global() {
        let canal = json!({
            "channelEmotes": [{ "id": "1", "code": "CanalPog", "imageType": "png", "animated": false }],
            "sharedEmotes": [{ "id": "2", "code": "Shared", "imageType": "gif", "animated": true }]
        });
        let e = parse_bttv(&canal, "channel");
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].url, "https://cdn.betterttv.net/emote/1/3x.png");
        assert!(e[1].animated);
        assert_eq!(e[1].url, "https://cdn.betterttv.net/emote/2/3x.gif");

        let global =
            json!([{ "id": "54fa", "code": ":tf:", "imageType": "png", "animated": false }]);
        let g = parse_bttv(&global, "global");
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].code, ":tf:");
        assert_eq!(g[0].scope, "global");
    }

    #[test]
    fn le_ffz_pegando_a_maior_url() {
        let v = json!({
            "room": { "mod_urls": { "1": "https://cdn.frankerfacez.com/room-badge/mod/1" } },
            "sets": { "166907": { "emoticons": [
                { "id": 246878, "name": "WideHard",
                  "urls": { "1": "https://cdn.frankerfacez.com/emote/246878/1",
                            "2": "https://cdn.frankerfacez.com/emote/246878/2",
                            "4": "https://cdn.frankerfacez.com/emote/246878/4" } },
                { "id": 9, "name": "Anim",
                  "urls": { "1": "https://cdn.frankerfacez.com/emote/9/1" },
                  "animated": { "1": "https://cdn.frankerfacez.com/emote/9/animated/1",
                                "4": "https://cdn.frankerfacez.com/emote/9/animated/4" } }
            ] } }
        });
        let e = parse_ffz(&v, "channel");
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].url, "https://cdn.frankerfacez.com/emote/246878/4");
        assert!(!e[0].animated);
        assert_eq!(e[1].url, "https://cdn.frankerfacez.com/emote/9/animated/4");
        assert!(e[1].animated);
        let b = parse_ffz_room_badges(&v, "channel");
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].code, "ffz-moderator");
    }

    #[test]
    fn le_7tv_pegando_o_maior_webp() {
        let set = json!({ "emotes": [{
            "id": "01G3", "name": "GAMBA",
            "data": { "animated": true, "host": { "url": "//cdn.7tv.app/emote/01G3", "files": [
                { "name": "1x.avif", "width": 39 },
                { "name": "1x.webp", "width": 39 },
                { "name": "4x.webp", "width": 156 }
            ] } }
        }] });
        let e = parse_7tv(&set, "channel");
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].url, "https://cdn.7tv.app/emote/01G3/4x.webp");
        assert!(e[0].animated);
    }

    #[test]
    fn dedupe_mantem_o_provedor_de_maior_prioridade() {
        let items = vec![
            Emote::new("ffz", "channel", "Pog", "1", "u1", false),
            Emote::new("7tv", "channel", "Pog", "2", "u2", true),
            Emote::new("bttv", "channel", "Pog", "3", "u3", false),
            Emote::new("bttv", "channel", "Outro", "4", "u4", false),
        ];
        let (kept, dupes) = dedupe(items);
        assert_eq!(dupes, 2);
        assert_eq!(kept.len(), 2);
        let pog = kept
            .iter()
            .find(|e| e.code == "Pog")
            .expect("Pog no resultado");
        assert_eq!(pog.provider, "7tv");
        assert_eq!(pog.also_in, vec!["bttv".to_string(), "ffz".to_string()]);
    }

    #[test]
    fn dedupe_nao_mistura_emote_com_badge() {
        let mut badge = Emote::new("twitch", "global", "Pog", "1", "u", false);
        badge.kind = "badge".into();
        let (kept, dupes) = dedupe(vec![
            Emote::new("twitch", "global", "Pog", "2", "u", false),
            badge,
        ]);
        assert_eq!(dupes, 0);
        assert_eq!(kept.len(), 2);
    }

    #[test]
    fn extensao_pela_url_ou_pelo_content_type() {
        assert_eq!(
            ext_from("https://cdn.7tv.app/emote/x/4x.webp", None),
            "webp"
        );
        assert_eq!(
            ext_from("https://cdn.betterttv.net/emote/x/3x.gif", None),
            "gif"
        );
        assert_eq!(
            ext_from("https://cdn.frankerfacez.com/emote/9/4", None),
            "png"
        );
        assert_eq!(
            ext_from(
                "https://cdn.frankerfacez.com/emote/9/4",
                Some("image/webp; charset=x")
            ),
            "webp"
        );
        assert_eq!(
            ext_from(
                "https://static-cdn.jtvnw.net/emoticons/v2/1/default/dark/3.0",
                Some("image/gif")
            ),
            "gif"
        );
    }

    #[test]
    fn nomes_de_arquivo_nao_colidem() {
        let mut taken = HashSet::new();
        assert_eq!(unique_name("Kappa", "1", "png", &mut taken), "Kappa.png");
        assert_eq!(unique_name("kappa", "2", "png", &mut taken), "kappa-2.png");
        assert_eq!(unique_name("kappa", "2", "png", &mut taken), "kappa-3.png");
        assert_eq!(unique_name(":tf:", "3", "png", &mut taken), "tf.png");
    }

    #[test]
    fn contact_sheet_escapa_e_lista() {
        let mut e = Emote::new("bttv", "global", "<script>", "1", "u", true);
        e.file = "bttv/script.png".into();
        let html = contact_sheet("xqc", &[e]);
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("src=\"bttv/script.png\""));
        assert!(html.contains("· anim"));
    }

    #[tokio::test]
    #[ignore = "rede: baixa de verdade um conjunto pequeno de emotes"]
    async fn live_baixa_um_conjunto_pequeno() {
        let dir = super::super::super::temp_dir().join("tw-emotes-teste");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("pasta de teste");
        let opts = Options {
            channel: "xqc".into(),
            out_dir: dir.to_string_lossy().to_string(),
            twitch: false,
            bttv: false,
            ffz: true,
            seventv: false,
            global: false,
            badges: true,
            contact_sheet: true,
            overwrite: true,
        };
        let out = run(&opts, &super::super::super::noop_progress())
            .await
            .expect("download dos emotes");
        assert!(out.total > 0, "nenhum emote");
        assert_eq!(out.failed, 0, "algum download falhou");
        assert!(std::path::Path::new(&out.index_path).exists(), "index.json");
        let sheet = out.sheet_path.clone().expect("contact sheet");
        assert!(std::path::Path::new(&sheet).exists());
        for item in &out.items {
            if item.file.is_empty() {
                continue;
            }
            let path = std::path::Path::new(&out.dir)
                .join(item.file.replace('/', std::path::MAIN_SEPARATOR_STR));
            let meta = std::fs::metadata(&path).expect("arquivo baixado");
            assert!(meta.len() > 0, "arquivo vazio: {}", item.file);
        }
        println!(
            "{} arquivos em {} ({} baixados)",
            out.total, out.dir, out.downloaded
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    #[ignore = "rede: bate nas APIs públicas de BTTV/FFZ/7TV/Twitch"]
    async fn live_provedores_de_emote_respondem() {
        let gql = Gql::new().expect("cliente");
        let http = super::super::super::client().expect("http");
        let ch = gql.channel("xqc").await.expect("canal xqc");
        assert_eq!(ch.id, "71092938");

        let bttv = json_get(
            &http,
            &format!("https://api.betterttv.net/3/cached/users/twitch/{}", ch.id),
        )
        .await
        .expect("bttv canal");
        let bttv_global = json_get(&http, "https://api.betterttv.net/3/cached/emotes/global")
            .await
            .expect("bttv global");
        assert!(
            !parse_bttv(&bttv_global, "global").is_empty(),
            "bttv global vazio"
        );

        let ffz = json_get(&http, "https://api.frankerfacez.com/v1/room/xqc")
            .await
            .expect("ffz sala");
        assert!(!parse_ffz(&ffz, "channel").is_empty(), "ffz sala vazia");

        let sv = json_get(&http, &format!("https://7tv.io/v3/users/twitch/{}", ch.id))
            .await
            .expect("7tv canal");
        let set = sv.get("emote_set").cloned().unwrap_or(Value::Null);
        assert!(!parse_7tv(&set, "channel").is_empty(), "7tv canal vazio");

        let tw = gql
            .query(r#"{ emoteSet(id: "0") { emotes { id token assetType } } }"#)
            .await
            .expect("emotes globais da twitch");
        let globais = parse_twitch_emotes(tw.get("emoteSet").unwrap_or(&Value::Null), "global");
        assert!(!globais.is_empty(), "twitch global vazio");
        println!(
            "bttv canal {} · bttv global {} · ffz {} · 7tv {} · twitch global {}",
            parse_bttv(&bttv, "channel").len(),
            parse_bttv(&bttv_global, "global").len(),
            parse_ffz(&ffz, "channel").len(),
            parse_7tv(&set, "channel").len(),
            globais.len()
        );
    }
}
