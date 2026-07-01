use chrono::{DateTime, Utc};
use std::collections::HashMap;

use super::parser::{ContentMetadata, EpisodeItem};
use super::url_kind::UrlKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamingKind {
    Video,
    MultiPart,
    Bangumi,
    Cheese,
    Collection,
}

pub fn classify(kind: &UrlKind, item: &EpisodeItem) -> NamingKind {
    match kind {
        UrlKind::BangumiEpisode { .. }
        | UrlKind::BangumiSeason { .. }
        | UrlKind::BangumiMedia { .. } => NamingKind::Bangumi,
        UrlKind::CheeseEpisode { .. } | UrlKind::CheeseSeason { .. } => NamingKind::Cheese,
        UrlKind::Space { .. }
        | UrlKind::Favlist { .. }
        | UrlKind::Collection { .. }
        | UrlKind::Series { .. }
        | UrlKind::PopularWeek { .. } => NamingKind::Collection,
        UrlKind::Video { .. } | UrlKind::Festival { .. } => {
            if item.page.is_some() {
                NamingKind::MultiPart
            } else {
                NamingKind::Video
            }
        }
        UrlKind::WatchLater | UrlKind::History => NamingKind::Video,
    }
}

pub const DEFAULT_VIDEO: &str = "{title}";
pub const DEFAULT_MULTI_PART: &str = "{parent_title}/P{page} - {leaf_title}";
pub const DEFAULT_BANGUMI: &str =
    "{series_title}/Season {season_number}/{episode_number_pad2} - {episode_title}";
pub const DEFAULT_CHEESE: &str =
    "{series_title}/{section_title}/{episode_number_pad2} - {episode_title}";
pub const DEFAULT_COLLECTION: &str = "{collection_title}/{title}";

pub struct NamingInputs<'a> {
    pub item: &'a EpisodeItem,
    pub metadata: &'a ContentMetadata,
    pub parsed_title: &'a str,
}

pub fn render(template: &str, inputs: &NamingInputs<'_>) -> String {
    let vars = build_vars(inputs);
    let rendered = substitute(template, &vars);
    sanitize_path(&rendered)
}

pub fn render_for_kind(
    kind: NamingKind,
    inputs: &NamingInputs<'_>,
    overrides: &TemplateSet,
) -> String {
    let template = overrides.template_for(kind);
    render(template, inputs)
}

#[derive(Debug, Clone)]
pub struct TemplateSet {
    pub video: String,
    pub multi_part: String,
    pub bangumi: String,
    pub cheese: String,
    pub collection: String,
}

impl Default for TemplateSet {
    fn default() -> Self {
        Self {
            video: DEFAULT_VIDEO.into(),
            multi_part: DEFAULT_MULTI_PART.into(),
            bangumi: DEFAULT_BANGUMI.into(),
            cheese: DEFAULT_CHEESE.into(),
            collection: DEFAULT_COLLECTION.into(),
        }
    }
}

impl TemplateSet {
    pub fn template_for(&self, kind: NamingKind) -> &str {
        match kind {
            NamingKind::Video => &self.video,
            NamingKind::MultiPart => &self.multi_part,
            NamingKind::Bangumi => &self.bangumi,
            NamingKind::Cheese => &self.cheese,
            NamingKind::Collection => &self.collection,
        }
    }
}

fn build_vars(inputs: &NamingInputs<'_>) -> HashMap<String, String> {
    let mut vars: HashMap<String, String> = HashMap::new();
    let item = inputs.item;
    let meta = inputs.metadata;

    vars.insert("title".into(), item.title.clone());
    vars.insert("leaf_title".into(), item.title.clone());
    vars.insert(
        "parent_title".into(),
        meta.series_title
            .clone()
            .or(meta.season_title.clone())
            .or(meta.collection_title.clone())
            .unwrap_or_else(|| inputs.parsed_title.to_string()),
    );

    if let Some(b) = item.bvid.as_deref() {
        vars.insert("bvid".into(), b.to_string());
    }
    if let Some(a) = item.aid {
        vars.insert("aid".into(), a.to_string());
    }
    if let Some(c) = item.cid {
        vars.insert("cid".into(), c.to_string());
    }
    if let Some(ep) = item.ep_id {
        vars.insert("ep_id".into(), ep.to_string());
    }
    if let Some(s) = item.season_id {
        vars.insert("season_id".into(), s.to_string());
    }
    if let Some(p) = item.page {
        vars.insert("page".into(), p.to_string());
        vars.insert("p".into(), p.to_string());
    }
    if let Some(num) = item.episode_number {
        vars.insert("episode_number".into(), num.to_string());
        vars.insert("episode_number_pad2".into(), format!("{:02}", num));
        vars.insert("episode_number_pad3".into(), format!("{:03}", num));
    }
    if let Some(section) = item.section_title.as_deref() {
        vars.insert("section_title".into(), section.to_string());
    }
    if let Some(page_title) = item.page_title.as_deref() {
        vars.insert("page_title".into(), page_title.to_string());
    }

    if let Some(s) = meta.series_title.as_deref() {
        vars.insert("series_title".into(), s.to_string());
    }
    if let Some(s) = meta.season_title.as_deref() {
        vars.insert("season_title".into(), s.to_string());
    }
    if let Some(n) = meta.season_number {
        vars.insert("season_number".into(), n.to_string());
        vars.insert("season_number_pad2".into(), format!("{:02}", n));
    }
    if let Some(s) = meta.collection_title.as_deref() {
        vars.insert("collection_title".into(), s.to_string());
    }
    if let Some(s) = meta.uploader.as_deref() {
        vars.insert("uploader".into(), s.to_string());
    }
    if let Some(u) = meta.uploader_uid {
        vars.insert("uploader_uid".into(), u.to_string());
    }
    if let Some(s) = meta.favorites_name.as_deref() {
        vars.insert("favorites_name".into(), s.to_string());
    }
    if let Some(s) = meta.favorites_owner.as_deref() {
        vars.insert("favorites_owner".into(), s.to_string());
    }
    if let Some(s) = meta.space_owner.as_deref() {
        vars.insert("space_owner".into(), s.to_string());
        vars.insert("uploader".into(), s.to_string());
    }

    let episode_title = item.title.clone();
    vars.insert("episode_title".into(), episode_title);

    if let Some(secs) = item.pub_time_secs.or(meta.premiered_secs) {
        if let Some(dt) = DateTime::<Utc>::from_timestamp(secs as i64, 0) {
            vars.insert("pub_year".into(), dt.format("%Y").to_string());
            vars.insert("pub_month".into(), dt.format("%m").to_string());
            vars.insert("pub_day".into(), dt.format("%d").to_string());
            vars.insert("pub_date".into(), dt.format("%Y-%m-%d").to_string());
        }
    }
    vars
}

fn substitute(template: &str, vars: &HashMap<String, String>) -> String {
    let mut out = String::with_capacity(template.len());
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '{' {
            let mut name = String::new();
            let mut closed = false;
            for nc in chars.by_ref() {
                if nc == '}' {
                    closed = true;
                    break;
                }
                name.push(nc);
            }
            if !closed {
                out.push('{');
                out.push_str(&name);
                continue;
            }
            let key = name.trim();
            if let Some(v) = vars.get(key) {
                out.push_str(v);
            } else {
                out.push_str("");
            }
        } else if c == '\\' {
            if let Some(&next) = chars.peek() {
                if next == '{' || next == '}' {
                    out.push(next);
                    chars.next();
                    continue;
                }
            }
            out.push(c);
        } else {
            out.push(c);
        }
    }
    out
}

pub fn sanitize_path(s: &str) -> String {
    let parts: Vec<&str> = s.split('/').collect();
    let cleaned: Vec<String> = parts
        .into_iter()
        .map(|p| sanitize_path_segment(p))
        .filter(|p| !p.is_empty())
        .collect();
    cleaned.join("/")
}

fn sanitize_path_segment(s: &str) -> String {
    let trimmed = s.trim();
    let mut out = String::with_capacity(trimmed.len());
    for c in trimmed.chars() {
        match c {
            '<' | '>' | ':' | '"' | '\\' | '|' | '?' | '*' => out.push('-'),
            '\0'..='\x1f' => out.push('-'),
            _ => out.push(c),
        }
    }
    let trimmed_end = out.trim_end_matches(['.', ' ']);
    let mut final_str = trimmed_end.to_string();
    if final_str.len() > 200 {
        final_str.truncate(200);
    }
    final_str
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item() -> EpisodeItem {
        EpisodeItem {
            title: "My Episode".into(),
            bvid: Some("BV111".into()),
            cid: Some(987),
            episode_number: Some(7),
            ..EpisodeItem::default()
        }
    }

    fn meta() -> ContentMetadata {
        ContentMetadata {
            series_title: Some("My Series".into()),
            season_number: Some(2),
            uploader: Some("Up".into()),
            ..ContentMetadata::default()
        }
    }

    #[test]
    fn renders_bangumi_default() {
        let inputs = NamingInputs {
            item: &item(),
            metadata: &meta(),
            parsed_title: "Whatever",
        };
        let out = render(DEFAULT_BANGUMI, &inputs);
        assert_eq!(out, "My Series/Season 2/07 - My Episode");
    }

    #[test]
    fn missing_variables_substituted_empty() {
        let inputs = NamingInputs {
            item: &item(),
            metadata: &ContentMetadata::default(),
            parsed_title: "T",
        };
        let out = render("{unknown_var}/{title}", &inputs);
        assert_eq!(out, "My Episode");
    }

    #[test]
    fn sanitizes_invalid_chars_per_segment() {
        let mut i = item();
        i.title = "Bad?<title>:|*".into();
        let inputs = NamingInputs {
            item: &i,
            metadata: &meta(),
            parsed_title: "T",
        };
        let out = render("{title}", &inputs);
        assert_eq!(out, "Bad--title----");
    }

    #[test]
    fn slash_in_template_creates_subfolder() {
        let inputs = NamingInputs {
            item: &item(),
            metadata: &meta(),
            parsed_title: "T",
        };
        let out = render("{series_title}/{episode_number_pad2}", &inputs);
        assert!(out.contains('/'));
        assert!(out.contains("07"));
    }

    #[test]
    fn escape_braces_with_backslash() {
        let inputs = NamingInputs {
            item: &item(),
            metadata: &meta(),
            parsed_title: "T",
        };
        let out = render(r"\{not a var\}", &inputs);
        assert_eq!(out, "{not a var}");
    }
}
