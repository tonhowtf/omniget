use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub const API_BASE: &str = "https://api.fontsource.org/v1/fonts";
const TIMEOUT: Duration = Duration::from_secs(25);

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LicenseClass {
    Open,
    Other,
}

pub fn classify_license(license: &str) -> LicenseClass {
    let l = license.trim().to_ascii_lowercase();
    let open = l.starts_with("ofl")
        || l.contains("open font license")
        || l.starts_with("apache")
        || l == "mit"
        || l.starts_with("mit ")
        || l.starts_with("mit-");
    if open {
        LicenseClass::Open
    } else {
        LicenseClass::Other
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiFont {
    id: String,
    family: String,
    #[serde(default)]
    subsets: Vec<String>,
    #[serde(default)]
    weights: Vec<u16>,
    #[serde(default)]
    styles: Vec<String>,
    #[serde(default)]
    def_subset: Option<String>,
    #[serde(default)]
    variable: bool,
    #[serde(default)]
    category: String,
    #[serde(default)]
    license: String,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    npm_version: Option<String>,
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    variants: BTreeMap<String, BTreeMap<String, BTreeMap<String, ApiVariant>>>,
}

#[derive(Debug, Clone, Deserialize)]
struct ApiVariant {
    url: ApiUrls,
}

#[derive(Debug, Clone, Deserialize)]
struct ApiUrls {
    #[serde(default)]
    woff2: Option<String>,
    #[serde(default)]
    ttf: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CatalogFont {
    pub id: String,
    pub family: String,
    pub category: String,
    pub subsets: Vec<String>,
    pub default_subset: String,
    pub weights: Vec<u16>,
    pub styles: Vec<String>,
    pub variable: bool,
    pub license: String,
    pub license_class: LicenseClass,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FontFile {
    pub weight: u16,
    pub style: String,
    pub subset: String,
    pub ttf: Option<String>,
    pub woff2: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FontDetails {
    #[serde(flatten)]
    pub font: CatalogFont,
    pub version: Option<String>,
    pub files: Vec<FontFile>,
}

fn to_catalog(api: &ApiFont) -> CatalogFont {
    let default_subset = api
        .def_subset
        .clone()
        .filter(|s| !s.is_empty())
        .or_else(|| api.subsets.iter().find(|s| s.as_str() == "latin").cloned())
        .or_else(|| api.subsets.first().cloned())
        .unwrap_or_else(|| "latin".to_string());
    CatalogFont {
        id: api.id.clone(),
        family: api.family.clone(),
        category: api.category.clone(),
        subsets: api.subsets.clone(),
        default_subset,
        weights: api.weights.clone(),
        styles: api.styles.clone(),
        variable: api.variable,
        license: api.license.clone(),
        license_class: classify_license(&api.license),
        source: if api.kind.is_empty() {
            "other".to_string()
        } else {
            api.kind.to_ascii_lowercase()
        },
    }
}

pub fn parse_list(json: &str) -> Result<Vec<CatalogFont>, String> {
    let list: Vec<ApiFont> =
        serde_json::from_str(json).map_err(|e| format!("Unexpected catalog format: {e}"))?;
    let mut fonts: Vec<CatalogFont> = list.iter().map(to_catalog).collect();
    fonts.sort_by_key(|f| f.family.to_lowercase());
    Ok(fonts)
}

pub fn parse_details(json: &str) -> Result<FontDetails, String> {
    let api: ApiFont =
        serde_json::from_str(json).map_err(|e| format!("Unexpected font format: {e}"))?;
    let mut files = Vec::new();
    for (weight, styles) in &api.variants {
        let Ok(weight) = weight.parse::<u16>() else {
            continue;
        };
        for (style, subsets) in styles {
            for (subset, variant) in subsets {
                files.push(FontFile {
                    weight,
                    style: style.clone(),
                    subset: subset.clone(),
                    ttf: variant.url.ttf.clone(),
                    woff2: variant.url.woff2.clone(),
                });
            }
        }
    }
    files.sort_by(|a, b| {
        (a.subset.as_str(), a.weight, a.style.as_str()).cmp(&(
            b.subset.as_str(),
            b.weight,
            b.style.as_str(),
        ))
    });
    Ok(FontDetails {
        font: to_catalog(&api),
        version: api.npm_version.clone().or(api.version.clone()),
        files,
    })
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 120
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

pub fn client() -> Result<reqwest::Client, String> {
    crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
        .timeout(TIMEOUT)
        .user_agent(concat!("OmniGet/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| format!("Could not start the network client: {e}"))
}

async fn get_text(client: &reqwest::Client, url: &str) -> Result<String, String> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| network_error(&e))?;
    let status = response.status();
    if status.as_u16() == 404 {
        return Err("Fontsource has no font with this id.".to_string());
    }
    if !status.is_success() {
        return Err(format!(
            "Fontsource answered {status}. Try again in a minute."
        ));
    }
    response.text().await.map_err(|e| network_error(&e))
}

pub fn network_error(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "Fontsource took too long to answer. Check your connection or proxy and try again."
            .to_string()
    } else if e.is_connect() {
        "Could not reach Fontsource. Check your internet connection or proxy settings.".to_string()
    } else {
        format!("Network error while talking to Fontsource: {e}")
    }
}

static LIST_CACHE: Mutex<Option<Vec<CatalogFont>>> = Mutex::new(None);
static DETAILS_CACHE: Mutex<Option<BTreeMap<String, FontDetails>>> = Mutex::new(None);

pub async fn list(refresh: bool) -> Result<Vec<CatalogFont>, String> {
    if !refresh {
        if let Some(cached) = LIST_CACHE.lock().ok().and_then(|g| g.clone()) {
            return Ok(cached);
        }
    }
    let client = client()?;
    let text = get_text(&client, API_BASE).await?;
    let fonts = parse_list(&text)?;
    if let Ok(mut g) = LIST_CACHE.lock() {
        *g = Some(fonts.clone());
    }
    Ok(fonts)
}

pub async fn details(id: &str) -> Result<FontDetails, String> {
    if !valid_id(id) {
        return Err("That is not a valid Fontsource id.".to_string());
    }
    if let Some(found) = DETAILS_CACHE
        .lock()
        .ok()
        .and_then(|g| g.as_ref().and_then(|m| m.get(id).cloned()))
    {
        return Ok(found);
    }
    let client = client()?;
    let text = get_text(&client, &format!("{API_BASE}/{id}")).await?;
    let details = parse_details(&text)?;
    if let Ok(mut g) = DETAILS_CACHE.lock() {
        g.get_or_insert_with(BTreeMap::new)
            .insert(id.to_string(), details.clone());
    }
    Ok(details)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_licenses() {
        assert_eq!(classify_license("OFL-1.1"), LicenseClass::Open);
        assert_eq!(classify_license("Apache-2.0"), LicenseClass::Open);
        assert_eq!(classify_license("mit"), LicenseClass::Open);
        assert_eq!(classify_license("MIT"), LicenseClass::Open);
        assert_eq!(classify_license("UFL-1.0"), LicenseClass::Other);
        assert_eq!(classify_license("CC0-1.0"), LicenseClass::Other);
        assert_eq!(classify_license(""), LicenseClass::Other);
        assert_eq!(classify_license("Proprietary"), LicenseClass::Other);
        assert_eq!(classify_license("Mitra License"), LicenseClass::Other);
    }

    #[test]
    fn parses_list_and_details() {
        let list = r#"[
          {"id":"inter","family":"Inter","subsets":["latin","cyrillic"],"weights":[400,700],"styles":["normal"],"defSubset":"latin","variable":true,"category":"sans-serif","license":"OFL-1.1","type":"google"},
          {"id":"abc","family":"ABC","subsets":["latin"],"weights":[400],"styles":["normal"],"variable":false,"category":"display","license":"UFL-1.0"}
        ]"#;
        let fonts = parse_list(list).unwrap();
        assert_eq!(fonts[0].id, "abc");
        assert_eq!(fonts[0].license_class, LicenseClass::Other);
        assert_eq!(fonts[0].default_subset, "latin");
        assert_eq!(fonts[1].license_class, LicenseClass::Open);
        assert_eq!(fonts[1].source, "google");
        assert_eq!(fonts[0].source, "other");

        let details = r#"{"id":"inter","family":"Inter","subsets":["latin"],"weights":[400,700],"styles":["normal","italic"],"defSubset":"latin","variable":true,"category":"sans-serif","license":"OFL-1.1","npmVersion":"5.2.0",
          "variants":{"700":{"normal":{"latin":{"url":{"woff2":"https://x/latin-700-normal.woff2","woff":"w","ttf":"https://x/latin-700-normal.ttf"}}}},
                      "400":{"italic":{"latin":{"url":{"woff2":"https://x/latin-400-italic.woff2"}}},"normal":{"latin":{"url":{"ttf":"https://x/latin-400-normal.ttf"}}}}}}"#;
        let d = parse_details(details).unwrap();
        assert_eq!(d.version.as_deref(), Some("5.2.0"));
        assert_eq!(d.files.len(), 3);
        assert_eq!(d.files[0].weight, 400);
        assert_eq!(d.files[0].style, "italic");
        assert!(d.files[0].ttf.is_none());
        assert_eq!(d.files[2].weight, 700);
        assert!(parse_details("{").is_err());
    }

    #[test]
    fn validates_ids() {
        assert!(valid_id("geist-mono"));
        assert!(!valid_id("../etc"));
        assert!(!valid_id(""));
        assert!(!valid_id("Inter"));
    }
}
