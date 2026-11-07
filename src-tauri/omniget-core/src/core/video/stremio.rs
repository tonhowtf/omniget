use serde::de::{DeserializeOwned, Deserializer};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

mod de {
    use super::*;

    pub fn string<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
        Ok(match Value::deserialize(d)? {
            Value::String(s) => s,
            Value::Number(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            _ => String::new(),
        })
    }

    pub fn opt_string<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
        Ok(match Value::deserialize(d)? {
            Value::String(s) if !s.trim().is_empty() => Some(s),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
    }

    pub fn vec<'de, D: Deserializer<'de>, T: DeserializeOwned>(d: D) -> Result<Vec<T>, D::Error> {
        Ok(match Value::deserialize(d)? {
            Value::Array(items) => items
                .into_iter()
                .filter_map(|v| serde_json::from_value(v).ok())
                .collect(),
            _ => Vec::new(),
        })
    }

    fn strings_of(v: Value) -> Vec<String> {
        match v {
            Value::Array(items) => items
                .into_iter()
                .filter_map(|v| match v {
                    Value::String(s) => Some(s),
                    Value::Number(n) => Some(n.to_string()),
                    _ => None,
                })
                .collect(),
            Value::String(s) if !s.is_empty() => vec![s],
            _ => Vec::new(),
        }
    }

    pub fn strings<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
        Ok(strings_of(Value::deserialize(d)?))
    }

    pub fn opt_strings<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<String>>, D::Error> {
        Ok(match Value::deserialize(d)? {
            Value::Null => None,
            v => Some(strings_of(v)),
        })
    }

    pub fn bool<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
        Ok(match Value::deserialize(d)? {
            Value::Bool(b) => b,
            Value::Number(n) => n.as_f64().is_some_and(|x| x != 0.0),
            Value::String(s) => matches!(s.trim().to_ascii_lowercase().as_str(), "true" | "1"),
            _ => false,
        })
    }

    pub fn opt_u64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
        Ok(match Value::deserialize(d)? {
            Value::Number(n) => n
                .as_u64()
                .or_else(|| n.as_f64().filter(|x| *x >= 0.0).map(|x| x as u64)),
            Value::String(s) => s.trim().parse().ok(),
            _ => None,
        })
    }

    pub fn opt_u32<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u32>, D::Error> {
        Ok(opt_u64(d)?.and_then(|x| u32::try_from(x).ok()))
    }

    pub fn string_map<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<BTreeMap<String, String>, D::Error> {
        Ok(match Value::deserialize(d)? {
            Value::Object(m) => m
                .into_iter()
                .filter_map(|(k, v)| match v {
                    Value::String(s) => Some((k, s)),
                    Value::Number(n) => Some((k, n.to_string())),
                    _ => None,
                })
                .collect(),
            _ => BTreeMap::new(),
        })
    }

    pub fn object<'de, D: Deserializer<'de>, T: DeserializeOwned + Default>(
        d: D,
    ) -> Result<T, D::Error> {
        Ok(serde_json::from_value(Value::deserialize(d)?).unwrap_or_default())
    }

    pub fn opt_object<'de, D: Deserializer<'de>, T: DeserializeOwned>(
        d: D,
    ) -> Result<Option<T>, D::Error> {
        Ok(serde_json::from_value(Value::deserialize(d)?).ok())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ManifestResource {
    Short(String),
    #[serde(rename_all = "camelCase")]
    Full {
        name: String,
        types: Option<Vec<String>>,
        id_prefixes: Option<Vec<String>>,
    },
}

impl ManifestResource {
    pub fn name(&self) -> &str {
        match self {
            ManifestResource::Short(n) => n,
            ManifestResource::Full { name, .. } => name,
        }
    }
}

impl<'de> Deserialize<'de> for ManifestResource {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Value::deserialize(d)?;
        match v {
            Value::String(s) if !s.is_empty() => Ok(ManifestResource::Short(s)),
            Value::Object(mut m) => {
                let name = match m.remove("name") {
                    Some(Value::String(s)) if !s.is_empty() => s,
                    _ => return Err(serde::de::Error::custom("resource without name")),
                };
                let list = |v: Option<Value>| -> Option<Vec<String>> {
                    match v? {
                        Value::Array(a) => Some(
                            a.into_iter()
                                .filter_map(|x| x.as_str().map(str::to_string))
                                .collect(),
                        ),
                        Value::String(s) => Some(vec![s]),
                        _ => None,
                    }
                };
                Ok(ManifestResource::Full {
                    name,
                    types: list(m.remove("types")),
                    id_prefixes: list(m.remove("idPrefixes")),
                })
            }
            _ => Err(serde::de::Error::custom("invalid resource")),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ExtraProp {
    #[serde(deserialize_with = "de::string")]
    pub name: String,
    #[serde(deserialize_with = "de::bool")]
    pub is_required: bool,
    #[serde(deserialize_with = "de::strings")]
    pub options: Vec<String>,
    #[serde(deserialize_with = "de::opt_u32")]
    pub options_limit: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ManifestCatalog {
    #[serde(rename = "type", deserialize_with = "de::string")]
    pub type_: String,
    #[serde(deserialize_with = "de::string")]
    pub id: String,
    #[serde(deserialize_with = "de::opt_string")]
    pub name: Option<String>,
    #[serde(deserialize_with = "de::vec")]
    pub extra: Vec<ExtraProp>,
    #[serde(deserialize_with = "de::strings")]
    pub extra_required: Vec<String>,
    #[serde(deserialize_with = "de::strings")]
    pub extra_supported: Vec<String>,
}

impl ManifestCatalog {
    pub fn extras(&self) -> Vec<ExtraProp> {
        let mut out = self.extra.clone();
        for n in self.extra_supported.iter().chain(&self.extra_required) {
            if !out.iter().any(|e| &e.name == n) {
                out.push(ExtraProp {
                    name: n.clone(),
                    is_required: self.extra_required.contains(n),
                    ..ExtraProp::default()
                });
            }
        }
        out
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ManifestBehaviorHints {
    #[serde(deserialize_with = "de::bool")]
    pub adult: bool,
    #[serde(deserialize_with = "de::bool")]
    pub p2p: bool,
    #[serde(deserialize_with = "de::bool")]
    pub configurable: bool,
    #[serde(deserialize_with = "de::bool")]
    pub configuration_required: bool,
    #[serde(deserialize_with = "de::bool")]
    pub epg_provider: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Manifest {
    #[serde(deserialize_with = "de::string")]
    pub id: String,
    #[serde(deserialize_with = "de::string")]
    pub version: String,
    #[serde(deserialize_with = "de::string")]
    pub name: String,
    #[serde(deserialize_with = "de::string")]
    pub description: String,
    #[serde(deserialize_with = "de::opt_string")]
    pub logo: Option<String>,
    #[serde(deserialize_with = "de::opt_string")]
    pub background: Option<String>,
    #[serde(deserialize_with = "de::strings")]
    pub types: Vec<String>,
    #[serde(deserialize_with = "de::vec")]
    pub resources: Vec<ManifestResource>,
    #[serde(deserialize_with = "de::opt_strings")]
    pub id_prefixes: Option<Vec<String>>,
    #[serde(deserialize_with = "de::vec")]
    pub catalogs: Vec<ManifestCatalog>,
    #[serde(deserialize_with = "de::object")]
    pub behavior_hints: ManifestBehaviorHints,
}

impl Manifest {
    pub fn resource_names(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for r in &self.resources {
            if !out.iter().any(|n| n == r.name()) {
                out.push(r.name().to_string());
            }
        }
        if !self.catalogs.is_empty() && !out.iter().any(|n| n == "catalog") {
            out.insert(0, "catalog".into());
        }
        out
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MetaPreview {
    #[serde(deserialize_with = "de::string")]
    pub id: String,
    #[serde(rename = "type", deserialize_with = "de::string")]
    pub type_: String,
    #[serde(deserialize_with = "de::string")]
    pub name: String,
    #[serde(deserialize_with = "de::opt_string")]
    pub poster: Option<String>,
    #[serde(deserialize_with = "de::opt_string")]
    pub poster_shape: Option<String>,
    #[serde(deserialize_with = "de::opt_string")]
    pub background: Option<String>,
    #[serde(deserialize_with = "de::opt_string")]
    pub logo: Option<String>,
    #[serde(deserialize_with = "de::opt_string")]
    pub description: Option<String>,
    #[serde(deserialize_with = "de::opt_string")]
    pub release_info: Option<String>,
    #[serde(deserialize_with = "de::strings")]
    pub genres: Vec<String>,
    #[serde(deserialize_with = "de::opt_string")]
    pub imdb_rating: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MetaBehaviorHints {
    #[serde(deserialize_with = "de::opt_string")]
    pub default_video_id: Option<String>,
    #[serde(deserialize_with = "de::bool")]
    pub is_live: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MetaDetail {
    #[serde(deserialize_with = "de::string")]
    pub id: String,
    #[serde(rename = "type", deserialize_with = "de::string")]
    pub type_: String,
    #[serde(deserialize_with = "de::string")]
    pub name: String,
    #[serde(deserialize_with = "de::opt_string")]
    pub poster: Option<String>,
    #[serde(deserialize_with = "de::opt_string")]
    pub background: Option<String>,
    #[serde(deserialize_with = "de::opt_string")]
    pub logo: Option<String>,
    #[serde(deserialize_with = "de::opt_string")]
    pub description: Option<String>,
    #[serde(deserialize_with = "de::opt_string")]
    pub release_info: Option<String>,
    #[serde(deserialize_with = "de::opt_string")]
    pub released: Option<String>,
    #[serde(deserialize_with = "de::opt_string")]
    pub runtime: Option<String>,
    #[serde(deserialize_with = "de::strings")]
    pub genres: Vec<String>,
    #[serde(deserialize_with = "de::opt_string")]
    pub imdb_rating: Option<String>,
    #[serde(deserialize_with = "de::vec")]
    pub videos: Vec<Video>,
    #[serde(deserialize_with = "de::object")]
    pub behavior_hints: MetaBehaviorHints,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Video {
    #[serde(deserialize_with = "de::string")]
    pub id: String,
    #[serde(deserialize_with = "de::string")]
    pub title: String,
    #[serde(deserialize_with = "de::string")]
    pub name: String,
    #[serde(deserialize_with = "de::opt_u32")]
    pub season: Option<u32>,
    #[serde(deserialize_with = "de::opt_u32")]
    pub episode: Option<u32>,
    #[serde(deserialize_with = "de::opt_u32")]
    pub number: Option<u32>,
    #[serde(deserialize_with = "de::opt_string")]
    pub released: Option<String>,
    #[serde(deserialize_with = "de::opt_string")]
    pub thumbnail: Option<String>,
    #[serde(deserialize_with = "de::opt_string")]
    pub overview: Option<String>,
    #[serde(deserialize_with = "de::opt_string")]
    pub description: Option<String>,
    #[serde(deserialize_with = "de::vec")]
    pub streams: Vec<Stream>,
}

impl Video {
    pub fn display_title(&self) -> &str {
        if self.title.is_empty() {
            &self.name
        } else {
            &self.title
        }
    }

    pub fn episode_no(&self) -> Option<u32> {
        self.episode.or(self.number)
    }

    pub fn summary(&self) -> Option<&str> {
        self.overview.as_deref().or(self.description.as_deref())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProxyHeaders {
    #[serde(deserialize_with = "de::string_map")]
    pub request: BTreeMap<String, String>,
    #[serde(deserialize_with = "de::string_map")]
    pub response: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StreamBehaviorHints {
    #[serde(deserialize_with = "de::bool")]
    pub not_web_ready: bool,
    #[serde(deserialize_with = "de::opt_string")]
    pub binge_group: Option<String>,
    #[serde(deserialize_with = "de::opt_object")]
    pub proxy_headers: Option<ProxyHeaders>,
    #[serde(deserialize_with = "de::opt_string")]
    pub filename: Option<String>,
    #[serde(deserialize_with = "de::opt_string")]
    pub video_hash: Option<String>,
    #[serde(deserialize_with = "de::opt_u64")]
    pub video_size: Option<u64>,
    #[serde(deserialize_with = "de::strings")]
    pub country_whitelist: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Stream {
    #[serde(deserialize_with = "de::opt_string")]
    pub url: Option<String>,
    #[serde(deserialize_with = "de::opt_string")]
    pub yt_id: Option<String>,
    #[serde(deserialize_with = "de::opt_string")]
    pub external_url: Option<String>,
    #[serde(deserialize_with = "de::opt_string")]
    pub info_hash: Option<String>,
    #[serde(deserialize_with = "de::opt_u32")]
    pub file_idx: Option<u32>,
    #[serde(deserialize_with = "de::opt_string")]
    pub nzb_url: Option<String>,
    #[serde(deserialize_with = "de::strings")]
    pub rar_urls: Vec<String>,
    #[serde(deserialize_with = "de::strings")]
    pub zip_urls: Vec<String>,
    #[serde(rename = "7zipUrls", deserialize_with = "de::strings")]
    pub sevenzip_urls: Vec<String>,
    #[serde(deserialize_with = "de::strings")]
    pub tgz_urls: Vec<String>,
    #[serde(deserialize_with = "de::strings")]
    pub tar_urls: Vec<String>,
    #[serde(deserialize_with = "de::opt_string")]
    pub player_frame_url: Option<String>,
    #[serde(deserialize_with = "de::string")]
    pub name: String,
    #[serde(deserialize_with = "de::string")]
    pub title: String,
    #[serde(deserialize_with = "de::string")]
    pub description: String,
    #[serde(deserialize_with = "de::opt_string")]
    pub thumbnail: Option<String>,
    #[serde(deserialize_with = "de::vec")]
    pub subtitles: Vec<Subtitle>,
    #[serde(deserialize_with = "de::object")]
    pub behavior_hints: StreamBehaviorHints,
}

impl Stream {
    pub fn label(&self) -> &str {
        if self.description.is_empty() {
            &self.title
        } else {
            &self.description
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Subtitle {
    #[serde(deserialize_with = "de::string")]
    pub id: String,
    #[serde(deserialize_with = "de::string")]
    pub url: String,
    #[serde(deserialize_with = "de::string")]
    pub lang: String,
    #[serde(deserialize_with = "de::opt_string")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CatalogResponse {
    #[serde(deserialize_with = "de::vec")]
    pub metas: Vec<MetaPreview>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MetaResponse {
    #[serde(deserialize_with = "de::opt_object")]
    pub meta: Option<MetaDetail>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StreamsResponse {
    #[serde(deserialize_with = "de::vec")]
    pub streams: Vec<Stream>,
}

pub fn normalize_manifest_url(u: &str) -> String {
    let t = u.trim();
    if let Some(rest) = t.strip_prefix("stremio://") {
        return format!("https://{rest}");
    }
    if !t.contains("://") && !t.is_empty() {
        return format!("https://{t}");
    }
    t.to_string()
}

fn split_base(manifest_url: &str) -> (String, Option<String>) {
    let u = normalize_manifest_url(manifest_url);
    let (path, query) = match u.split_once('?') {
        Some((p, q)) => (p.to_string(), Some(q.to_string()).filter(|q| !q.is_empty())),
        None => (u, None),
    };
    let path = path.split('#').next().unwrap_or("").to_string();
    let base = path
        .strip_suffix("/manifest.json")
        .unwrap_or(&path)
        .trim_end_matches('/')
        .to_string();
    (base, query)
}

pub fn addon_base(manifest_url: &str) -> String {
    split_base(manifest_url).0
}

pub fn encode_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn resource_url(
    base: &str,
    resource: &str,
    type_: &str,
    id: &str,
    extra: &[(String, String)],
) -> String {
    let (b, query) = split_base(base);
    let mut url = format!(
        "{b}/{}/{}/{}",
        encode_component(resource),
        encode_component(type_),
        encode_component(id)
    );
    let extra: Vec<String> = extra
        .iter()
        .filter(|(k, _)| !k.is_empty())
        .map(|(k, v)| format!("{}={}", encode_component(k), encode_component(v)))
        .collect();
    if !extra.is_empty() {
        url.push('/');
        url.push_str(&extra.join("&"));
    }
    url.push_str(".json");
    if let Some(q) = query {
        url.push('?');
        url.push_str(&q);
    }
    url
}

pub fn catalog_url(base: &str, type_: &str, id: &str, extra: &[(String, String)]) -> String {
    resource_url(base, "catalog", type_, id, extra)
}

pub fn meta_url(base: &str, type_: &str, id: &str) -> String {
    resource_url(base, "meta", type_, id, &[])
}

pub fn stream_url(base: &str, type_: &str, id: &str) -> String {
    resource_url(base, "stream", type_, id, &[])
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "lowercase")]
pub enum StreamKind {
    Url(String),
    YouTube(String),
    External(String),
    Refused(String),
}

fn scheme(u: &str) -> String {
    u.trim()
        .split_once(':')
        .map(|(s, _)| s.to_ascii_lowercase())
        .unwrap_or_default()
}

fn is_torrent_url(u: &str) -> bool {
    let path = u
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    path.ends_with(".torrent")
}

pub fn classify(stream: &Stream) -> StreamKind {
    let refused = |r: &str| StreamKind::Refused(r.to_string());
    if stream.info_hash.is_some() {
        return refused("torrent");
    }
    if stream.nzb_url.is_some() {
        return refused("usenet");
    }
    if !stream.rar_urls.is_empty()
        || !stream.zip_urls.is_empty()
        || !stream.sevenzip_urls.is_empty()
        || !stream.tgz_urls.is_empty()
        || !stream.tar_urls.is_empty()
    {
        return refused("archive");
    }
    if let Some(u) = stream.url.as_deref() {
        let u = u.trim();
        return match scheme(u).as_str() {
            "magnet" => refused("torrent"),
            "http" | "https" if is_torrent_url(u) => refused("torrent"),
            "http" | "https" => StreamKind::Url(u.to_string()),
            "stremio" => StreamKind::External(u.to_string()),
            _ => refused("unsupported"),
        };
    }
    if let Some(id) = stream.yt_id.as_deref() {
        let id = id.trim();
        if !id.is_empty()
            && id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return StreamKind::YouTube(id.to_string());
        }
        return refused("unsupported");
    }
    if let Some(u) = stream.external_url.as_deref() {
        let u = u.trim();
        return match scheme(u).as_str() {
            "http" | "https" | "stremio" => StreamKind::External(u.to_string()),
            "magnet" => refused("torrent"),
            _ => refused("unsupported"),
        };
    }
    if stream.player_frame_url.is_some() {
        return refused("unsupported");
    }
    refused("empty")
}

pub fn classify_from(manifest: &Manifest, stream: &Stream) -> StreamKind {
    if manifest_is_p2p(manifest) {
        return StreamKind::Refused("p2p".into());
    }
    classify(stream)
}

pub fn manifest_is_p2p(manifest: &Manifest) -> bool {
    manifest.behavior_hints.p2p
}

pub fn supports(manifest: &Manifest, resource: &str, type_: &str, id: &str) -> bool {
    if resource == "catalog" {
        return manifest
            .catalogs
            .iter()
            .any(|c| c.type_ == type_ && c.id == id);
    }
    manifest.resources.iter().any(|r| {
        let (types, prefixes) = match r {
            ManifestResource::Short(n) if n == resource => {
                (Some(&manifest.types), manifest.id_prefixes.as_ref())
            }
            ManifestResource::Full {
                name,
                types,
                id_prefixes,
            } if name == resource => (
                Some(types.as_ref().unwrap_or(&manifest.types)),
                id_prefixes.as_ref(),
            ),
            _ => (None, None),
        };
        let Some(types) = types else {
            return false;
        };
        types.iter().any(|t| t == type_)
            && prefixes.is_none_or(|p| p.is_empty() || p.iter().any(|x| id.starts_with(x.as_str())))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(json: &str) -> Stream {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn classify_policy() {
        assert_eq!(
            classify(&stream(r#"{"url":"https://cdn/x.mp4","name":"HD"}"#)),
            StreamKind::Url("https://cdn/x.mp4".into())
        );
        assert_eq!(
            classify(&stream(r#"{"ytId":"dQw4w9WgXcQ"}"#)),
            StreamKind::YouTube("dQw4w9WgXcQ".into())
        );
        assert_eq!(
            classify(&stream(r#"{"externalUrl":"https://site/watch"}"#)),
            StreamKind::External("https://site/watch".into())
        );
        let refused = |j: &str| match classify(&stream(j)) {
            StreamKind::Refused(r) => r,
            other => panic!("{j} gave {other:?}"),
        };
        assert_eq!(
            refused(r#"{"infoHash":"abcdef0123456789abcdef0123456789abcdef01","fileIdx":1}"#),
            "torrent"
        );
        assert_eq!(
            refused(r#"{"infoHash":"x","url":"https://cdn/x.mp4"}"#),
            "torrent"
        );
        assert_eq!(refused(r#"{"url":"magnet:?xt=urn:btih:abc"}"#), "torrent");
        assert_eq!(
            refused(r#"{"url":"https://x/file.torrent?a=1"}"#),
            "torrent"
        );
        assert_eq!(refused(r#"{"nzbUrl":"https://x/a.nzb"}"#), "usenet");
        assert_eq!(refused(r#"{"rarUrls":["https://x/a.rar"]}"#), "archive");
        assert_eq!(refused(r#"{"zipUrls":["https://x/a.zip"]}"#), "archive");
        assert_eq!(refused(r#"{"7zipUrls":["https://x/a.7z"]}"#), "archive");
        assert_eq!(refused(r#"{"url":"ftp://x/a.mp4"}"#), "unsupported");
        assert_eq!(refused(r#"{"playerFrameUrl":"https://x"}"#), "unsupported");
        assert_eq!(refused(r#"{"ytId":"bad id!"}"#), "unsupported");
        assert_eq!(refused(r#"{}"#), "empty");

        let m: Manifest = serde_json::from_str(r#"{"behaviorHints":{"p2p":true}}"#).unwrap();
        assert!(manifest_is_p2p(&m));
        assert_eq!(
            classify_from(&m, &stream(r#"{"url":"https://x"}"#)),
            StreamKind::Refused("p2p".into())
        );
        assert_eq!(
            serde_json::to_string(&StreamKind::YouTube("a".into())).unwrap(),
            r#"{"kind":"youtube","value":"a"}"#
        );
    }

    #[test]
    fn tolerant_manifest() {
        let json = r#"{
            "id": "org.example", "version": 1, "name": "Ex", "description": null,
            "logo": "", "types": "movie",
            "resources": ["catalog", {"name":"stream","types":["movie","series"],"idPrefixes":["tt"]},
                          {"types":["x"]}, 42, "meta"],
            "catalogs": [{"type":"movie","id":"top","extra":[{"name":"search","isRequired":"true"},{"name":"skip"}]},
                         {"type":"movie","id":"legacy","extraSupported":["genre"],"extraRequired":["genre"]},
                         "junk"],
            "behaviorHints": "oops",
            "unknown": {"deep": [1,2,3]}
        }"#;
        let m: Manifest = serde_json::from_str(json).unwrap();
        assert_eq!(m.version, "1");
        assert_eq!(m.description, "");
        assert_eq!(m.logo, None);
        assert_eq!(m.types, vec!["movie"]);
        assert_eq!(m.resources.len(), 3);
        assert_eq!(m.catalogs.len(), 2);
        assert!(!m.behavior_hints.p2p);
        assert_eq!(m.resource_names(), vec!["catalog", "stream", "meta"]);
        assert!(m.catalogs[0].extra[0].is_required);

        assert!(supports(&m, "catalog", "movie", "top"));
        assert!(!supports(&m, "catalog", "series", "top"));
        assert!(supports(&m, "stream", "series", "tt123:1:2"));
        assert!(!supports(&m, "stream", "series", "kitsu:1"));
        assert!(supports(&m, "meta", "movie", "anything"));
        assert!(!supports(&m, "meta", "series", "tt1"));
        assert!(!supports(&m, "subtitles", "movie", "tt1"));

        assert!(serde_json::from_str::<Manifest>("<html>").is_err());
        assert!(serde_json::from_str::<Manifest>("[]").map_or(true, |m| m.id.is_empty()));
        let empty: Manifest = serde_json::from_str("{}").unwrap();
        assert!(empty.resources.is_empty());
    }

    #[test]
    fn responses() {
        let c: CatalogResponse = serde_json::from_str(
            r#"{"metas":[{"id":"tt1","type":"movie","name":"A","poster":"p"},null,{"id":2,"name":"B","genres":null}]}"#,
        )
        .unwrap();
        assert_eq!(c.metas.len(), 2);
        assert_eq!(c.metas[1].id, "2");
        let c: CatalogResponse = serde_json::from_str(r#"{"metas":null}"#).unwrap();
        assert!(c.metas.is_empty());

        let m: MetaResponse = serde_json::from_str(
            r#"{"meta":{"id":"tt1","type":"series","name":"S","videos":[
                {"id":"tt1:1:1","name":"Pilot","title":"Pilot!","season":1,"number":1,"overview":"o","description":"d"},
                {"id":"tt1:1:2","name":"Two","season":"1","episode":2}
            ],"behaviorHints":{"defaultVideoId":null,"isLive":false}}}"#,
        )
        .unwrap();
        let meta = m.meta.unwrap();
        assert_eq!(meta.videos.len(), 2);
        assert_eq!(meta.videos[0].display_title(), "Pilot!");
        assert_eq!(meta.videos[0].episode_no(), Some(1));
        assert_eq!(meta.videos[0].summary(), Some("o"));
        assert_eq!(meta.videos[1].season, Some(1));
        assert_eq!(meta.videos[1].display_title(), "Two");

        let s: StreamsResponse = serde_json::from_str(
            r#"{"streams":[{"url":"https://x","title":"old label","behaviorHints":{"notWebReady":true,"proxyHeaders":{"request":{"User-Agent":"UA"}}}},{"infoHash":"h"}]}"#,
        )
        .unwrap();
        assert_eq!(s.streams.len(), 2);
        assert_eq!(s.streams[0].label(), "old label");
        let ph = s.streams[0].behavior_hints.proxy_headers.as_ref().unwrap();
        assert_eq!(ph.request.get("User-Agent").map(String::as_str), Some("UA"));
    }

    #[test]
    fn urls() {
        let base = "stremio://addon.example.com/cfg%7B%7D/manifest.json";
        assert_eq!(addon_base(base), "https://addon.example.com/cfg%7B%7D");
        assert_eq!(
            catalog_url(base, "movie", "top", &[]),
            "https://addon.example.com/cfg%7B%7D/catalog/movie/top.json"
        );
        assert_eq!(
            catalog_url(
                "https://a.com/manifest.json",
                "movie",
                "top",
                &[
                    ("search".into(), "game of thrones & co".into()),
                    ("skip".into(), "100".into())
                ]
            ),
            "https://a.com/catalog/movie/top/search=game%20of%20thrones%20%26%20co&skip=100.json"
        );
        assert_eq!(
            meta_url("https://a.com/", "series", "tt1:1 2"),
            "https://a.com/meta/series/tt1%3A1%202.json"
        );
        assert_eq!(
            stream_url("https://a.com/manifest.json?token=x", "movie", "tt1"),
            "https://a.com/stream/movie/tt1.json?token=x"
        );
        assert_eq!(
            normalize_manifest_url(" a.com/manifest.json "),
            "https://a.com/manifest.json"
        );
        assert_eq!(encode_component("a-_.!~*'()é/"), "a-_.!~*'()%C3%A9%2F");
    }
}
