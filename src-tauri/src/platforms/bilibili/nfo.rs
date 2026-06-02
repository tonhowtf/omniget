use chrono::{DateTime, Utc};
use serde_json::Value;

use super::api::{check_api_response, ApiClient};
use super::parser::{ContentMetadata, EpisodeItem};
use super::url_kind::UrlKind;

const TAG_DETAIL_URL: &str = "https://api.bilibili.com/x/web-interface/view/detail/tag";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NfoKind {
    Movie,
    TvShow,
    Episode,
}

pub fn classify(kind: &UrlKind) -> Option<NfoKind> {
    match kind {
        UrlKind::Video { .. } | UrlKind::Festival { .. } => Some(NfoKind::Movie),
        UrlKind::BangumiEpisode { .. } | UrlKind::CheeseEpisode { .. } => Some(NfoKind::Episode),
        UrlKind::BangumiSeason { .. }
        | UrlKind::BangumiMedia { .. }
        | UrlKind::CheeseSeason { .. } => Some(NfoKind::Episode),
        _ => None,
    }
}

pub fn season_uses_tvshow(kind: &UrlKind) -> bool {
    matches!(
        kind,
        UrlKind::BangumiSeason { .. } | UrlKind::BangumiMedia { .. } | UrlKind::CheeseSeason { .. }
    )
}

pub async fn fetch_tags(client: &ApiClient, bvid: &str) -> Vec<String> {
    let url = format!("{}?bvid={}", TAG_DETAIL_URL, urlencoding::encode(bvid));
    let raw = match client.get_json(&url).await {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let data = match check_api_response(&raw) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let arr = match data.as_array() {
        Some(a) => a,
        None => return Vec::new(),
    };
    arr.iter()
        .filter_map(|t| t.get("tag_name").and_then(Value::as_str).map(String::from))
        .collect()
}

pub fn render(
    nfo_kind: NfoKind,
    item: &EpisodeItem,
    metadata: &ContentMetadata,
    tags: &[String],
) -> String {
    match nfo_kind {
        NfoKind::Movie => render_movie(item, metadata, tags),
        NfoKind::TvShow => render_tvshow(metadata),
        NfoKind::Episode => render_episode(item, metadata),
    }
}

fn render_movie(item: &EpisodeItem, metadata: &ContentMetadata, tags: &[String]) -> String {
    let mut buf = String::new();
    buf.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n");
    buf.push_str("<movie>\n");
    write_tag(&mut buf, "title", &item.title);
    if let Some(plot) = metadata.description.as_deref() {
        write_tag(&mut buf, "plot", plot);
    }
    if let Some(d) = item.duration_seconds {
        write_tag(
            &mut buf,
            "runtime",
            &format!("{}", (d / 60.0).round() as i64),
        );
    }
    if let Some((year, ymd)) = parse_date(item.pub_time_secs.or(metadata.premiered_secs)) {
        write_tag(&mut buf, "premiered", &ymd);
        write_tag(&mut buf, "year", &year.to_string());
    }
    write_tag(&mut buf, "studio", "Bilibili");
    if let Some(uname) = metadata.uploader.as_deref() {
        write_tag(&mut buf, "director", uname);
    }
    for tag in tags.iter().take(20) {
        write_tag(&mut buf, "tag", tag);
        write_tag(&mut buf, "genre", tag);
    }
    if let Some(b) = item.bvid.as_deref() {
        buf.push_str(&format!(
            "  <uniqueid type=\"bilibili\">{}</uniqueid>\n",
            escape(b)
        ));
    }
    buf.push_str("</movie>\n");
    buf
}

fn render_tvshow(metadata: &ContentMetadata) -> String {
    let title = metadata
        .series_title
        .as_deref()
        .or(metadata.season_title.as_deref())
        .unwrap_or("Bilibili")
        .to_string();
    let mut buf = String::new();
    buf.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n");
    buf.push_str("<tvshow>\n");
    write_tag(&mut buf, "title", &title);
    if let Some(plot) = metadata.description.as_deref() {
        write_tag(&mut buf, "plot", plot);
    }
    if let Some((year, ymd)) = parse_date(metadata.premiered_secs) {
        write_tag(&mut buf, "premiered", &ymd);
        write_tag(&mut buf, "year", &year.to_string());
    }
    write_tag(&mut buf, "studio", "Bilibili");
    if let Some(uname) = metadata.uploader.as_deref() {
        write_tag(&mut buf, "director", uname);
    }
    if let Some(actors) = metadata.actors.as_deref() {
        for actor in actors
            .split(['\n', ',', '/', '、'])
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            buf.push_str("  <actor>\n");
            write_tag_indented(&mut buf, "    ", "name", actor);
            buf.push_str("  </actor>\n");
        }
    }
    if let Some(score) = metadata.rating {
        write_tag(&mut buf, "rating", &format!("{:.1}", score));
    }
    for style in &metadata.styles {
        write_tag(&mut buf, "genre", style);
    }
    for area in &metadata.areas {
        write_tag(&mut buf, "country", area);
    }
    if let Some(sid) = metadata.season_id {
        buf.push_str(&format!(
            "  <uniqueid type=\"bilibili-ss\">{}</uniqueid>\n",
            sid
        ));
    }
    if let Some(mid) = metadata.media_id {
        buf.push_str(&format!(
            "  <uniqueid type=\"bilibili-md\">{}</uniqueid>\n",
            mid
        ));
    }
    buf.push_str("</tvshow>\n");
    buf
}

fn render_episode(item: &EpisodeItem, metadata: &ContentMetadata) -> String {
    let mut buf = String::new();
    buf.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n");
    buf.push_str("<episodedetails>\n");
    write_tag(&mut buf, "title", &item.title);
    if let Some(plot) = metadata.description.as_deref() {
        write_tag(&mut buf, "plot", plot);
    }
    if let Some(d) = item.duration_seconds {
        write_tag(
            &mut buf,
            "runtime",
            &format!("{}", (d / 60.0).round() as i64),
        );
    }
    if let Some((year, ymd)) = parse_date(item.pub_time_secs.or(metadata.premiered_secs)) {
        write_tag(&mut buf, "premiered", &ymd);
        write_tag(&mut buf, "year", &year.to_string());
    }
    write_tag(&mut buf, "studio", "Bilibili");
    if let Some(num) = item.episode_number {
        write_tag(&mut buf, "episode", &num.to_string());
    }
    if let Some(season) = metadata.season_number {
        write_tag(&mut buf, "season", &season.to_string());
    }
    if let Some(uname) = metadata.uploader.as_deref() {
        write_tag(&mut buf, "director", uname);
    }
    for style in &metadata.styles {
        write_tag(&mut buf, "genre", style);
    }
    for area in &metadata.areas {
        write_tag(&mut buf, "country", area);
    }
    if let Some(ep_id) = item.ep_id {
        buf.push_str(&format!(
            "  <uniqueid type=\"bilibili-ep\">{}</uniqueid>\n",
            ep_id
        ));
    }
    buf.push_str("</episodedetails>\n");
    buf
}

fn write_tag(buf: &mut String, name: &str, value: &str) {
    buf.push_str(&format!("  <{}>{}</{}>\n", name, escape(value), name));
}

fn write_tag_indented(buf: &mut String, indent: &str, name: &str, value: &str) {
    buf.push_str(&format!(
        "{}<{}>{}</{}>\n",
        indent,
        name,
        escape(value),
        name
    ));
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

fn parse_date(secs: Option<u64>) -> Option<(i32, String)> {
    let s = secs?;
    let dt: DateTime<Utc> = DateTime::from_timestamp(s as i64, 0)?;
    let year = dt.format("%Y").to_string().parse::<i32>().ok()?;
    let ymd = dt.format("%Y-%m-%d").to_string();
    Some((year, ymd))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_item() -> EpisodeItem {
        EpisodeItem {
            episode_id: "video:BV123".into(),
            title: "My Title".into(),
            bvid: Some("BV123".into()),
            duration_seconds: Some(180.0),
            pub_time_secs: Some(1700000000),
            episode_number: Some(3),
            ..EpisodeItem::default()
        }
    }

    fn dummy_meta() -> ContentMetadata {
        ContentMetadata {
            series_title: Some("My Series".into()),
            description: Some("Some <plot> & more".into()),
            uploader: Some("UpName".into()),
            premiered_secs: Some(1700000000),
            season_number: Some(2),
            styles: vec!["Anime".into()],
            areas: vec!["Japan".into()],
            season_id: Some(42),
            media_id: Some(99),
            ..ContentMetadata::default()
        }
    }

    #[test]
    fn movie_includes_basics_and_escapes() {
        let xml = render_movie(&dummy_item(), &dummy_meta(), &["Tag1".into()]);
        assert!(xml.contains("<title>My Title</title>"));
        assert!(xml.contains("Some &lt;plot&gt; &amp; more"));
        assert!(xml.contains("<studio>Bilibili</studio>"));
        assert!(xml.contains("<runtime>3</runtime>"));
        assert!(xml.contains("<year>2023</year>"));
        assert!(xml.contains("<tag>Tag1</tag>"));
        assert!(xml.contains("<uniqueid type=\"bilibili\">BV123</uniqueid>"));
    }

    #[test]
    fn tvshow_includes_series_and_genres() {
        let xml = render_tvshow(&dummy_meta());
        assert!(xml.contains("<title>My Series</title>"));
        assert!(xml.contains("<genre>Anime</genre>"));
        assert!(xml.contains("<country>Japan</country>"));
        assert!(xml.contains("<uniqueid type=\"bilibili-ss\">42</uniqueid>"));
    }

    #[test]
    fn episode_has_season_and_episode() {
        let xml = render_episode(&dummy_item(), &dummy_meta());
        assert!(xml.contains("<season>2</season>"));
        assert!(xml.contains("<episode>3</episode>"));
    }
}
