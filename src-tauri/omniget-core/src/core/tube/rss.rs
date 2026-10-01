use roxmltree::{Document, Node};

use super::model::{ChannelRef, Item, ShortItem, Thumb, VideoItem};

fn child<'a, 'i>(n: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    n.children()
        .find(|c| c.is_element() && c.tag_name().name() == name)
}

fn child_text(n: Node, name: &str) -> Option<String> {
    child(n, name)
        .and_then(|c| c.text())
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

fn parse_entry(e: Node, feed_channel: &ChannelRef) -> Option<(Item, bool)> {
    let id = child_text(e, "videoId").or_else(|| {
        child_text(e, "id").and_then(|i| i.strip_prefix("yt:video:").map(str::to_string))
    })?;
    let title = child_text(e, "title").unwrap_or_default();
    let channel = ChannelRef {
        id: child_text(e, "channelId").unwrap_or_else(|| feed_channel.id.clone()),
        name: child(e, "author")
            .and_then(|a| child_text(a, "name"))
            .unwrap_or_else(|| feed_channel.name.clone()),
        ..Default::default()
    };
    let is_short = e
        .children()
        .filter(|c| c.is_element() && c.tag_name().name() == "link")
        .any(|l| l.attribute("href").is_some_and(|h| h.contains("/shorts/")));
    let group = child(e, "group");
    let thumbs: Vec<Thumb> = group
        .and_then(|g| child(g, "thumbnail"))
        .and_then(|t| {
            Some(Thumb {
                url: t.attribute("url")?.replacen("http://", "https://", 1),
                width: t
                    .attribute("width")
                    .and_then(|w| w.parse().ok())
                    .unwrap_or(0),
                height: t
                    .attribute("height")
                    .and_then(|h| h.parse().ok())
                    .unwrap_or(0),
            })
        })
        .into_iter()
        .collect();
    let views = group
        .and_then(|g| child(g, "community"))
        .and_then(|c| child(c, "statistics"))
        .and_then(|s| s.attribute("views"))
        .and_then(|v| v.parse::<u64>().ok());
    let published_ms = child_text(e, "published")
        .and_then(|p| chrono::DateTime::parse_from_rfc3339(&p).ok())
        .map(|d| d.timestamp_millis());
    let description = group
        .and_then(|g| child_text(g, "description"))
        .map(|d| d.chars().take(300).collect());
    let video = VideoItem {
        id,
        title,
        channel: Some(channel),
        thumbs,
        views,
        published_ms,
        description_snippet: description,
        ..Default::default()
    };
    Some((Item::Video(video), is_short))
}

fn parse(xml: &str) -> Option<(String, Vec<(Item, bool)>)> {
    let doc = Document::parse(xml).ok()?;
    let feed = doc.root_element();
    if feed.tag_name().name() != "feed" {
        return None;
    }
    let name = child(feed, "author")
        .and_then(|a| child_text(a, "name"))
        .or_else(|| child_text(feed, "title"))
        .unwrap_or_default();
    let feed_channel = ChannelRef {
        id: child_text(feed, "channelId").unwrap_or_default(),
        name: name.clone(),
        ..Default::default()
    };
    let entries = feed
        .children()
        .filter(|c| c.is_element() && c.tag_name().name() == "entry")
        .filter_map(|e| parse_entry(e, &feed_channel))
        .collect();
    Some((name, entries))
}

pub fn parse_channel_feed(xml: &str) -> Option<(String, Vec<VideoItem>)> {
    let (name, entries) = parse(xml)?;
    let videos = entries
        .into_iter()
        .filter_map(|(item, _)| match item {
            Item::Video(v) => Some(v),
            _ => None,
        })
        .collect();
    Some((name, videos))
}

pub fn parse_channel_feed_items(xml: &str) -> Option<(String, Vec<Item>)> {
    let (name, entries) = parse(xml)?;
    let items = entries
        .into_iter()
        .map(|(item, is_short)| match item {
            Item::Video(v) if is_short => Item::Short(ShortItem {
                id: v.id,
                title: v.title,
                thumbs: v.thumbs,
                views_text: None,
                channel: v.channel,
            }),
            other => other,
        })
        .collect();
    Some((name, items))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FEED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns:yt="http://www.youtube.com/xml/schemas/2015" xmlns:media="http://search.yahoo.com/mrss/" xmlns="http://www.w3.org/2005/Atom">
 <link rel="self" href="http://www.youtube.com/feeds/videos.xml?channel_id=UCabc"/>
 <id>yt:channel:abc</id>
 <yt:channelId>UCabc</yt:channelId>
 <title>Canal &amp; Cia</title>
 <author><name>Canal &amp; Cia</name><uri>https://www.youtube.com/channel/UCabc</uri></author>
 <entry>
  <id>yt:video:vid00000001</id>
  <yt:videoId>vid00000001</yt:videoId>
  <yt:channelId>UCabc</yt:channelId>
  <title>Primeiro vídeo</title>
  <link rel="alternate" href="https://www.youtube.com/watch?v=vid00000001"/>
  <author><name>Canal &amp; Cia</name></author>
  <published>2026-09-01T12:00:00+00:00</published>
  <media:group>
   <media:title>Primeiro vídeo</media:title>
   <media:thumbnail url="https://i1.ytimg.com/vi/vid00000001/hqdefault.jpg" width="480" height="360"/>
   <media:description>Descrição</media:description>
   <media:community>
    <media:starRating count="10" average="5.00" min="1" max="5"/>
    <media:statistics views="1234"/>
   </media:community>
  </media:group>
 </entry>
 <entry>
  <id>yt:video:short000001</id>
  <yt:videoId>short000001</yt:videoId>
  <title>Um short</title>
  <link rel="alternate" href="https://www.youtube.com/shorts/short000001"/>
  <published>2026-09-02T08:30:00-03:00</published>
  <media:group><media:community><media:statistics views="0"/></media:community></media:group>
 </entry>
</feed>"#;

    #[test]
    fn parses_feed() {
        let (name, videos) = parse_channel_feed(FEED).unwrap();
        assert_eq!(name, "Canal & Cia");
        assert_eq!(videos.len(), 2);
        let v = &videos[0];
        assert_eq!(v.id, "vid00000001");
        assert_eq!(v.title, "Primeiro vídeo");
        assert_eq!(v.views, Some(1234));
        assert_eq!(v.published_ms, Some(1_788_264_000_000));
        assert_eq!(v.channel.as_ref().unwrap().id, "UCabc");
        assert_eq!(v.thumbs[0].width, 480);
        assert_eq!(v.description_snippet.as_deref(), Some("Descrição"));
        assert_eq!(videos[1].channel.as_ref().unwrap().name, "Canal & Cia");
        assert_eq!(videos[1].published_ms, Some(1_788_348_600_000));
    }

    #[test]
    fn shorts_become_short_items() {
        let (_, items) = parse_channel_feed_items(FEED).unwrap();
        assert!(matches!(items[0], Item::Video(_)));
        assert!(matches!(&items[1], Item::Short(s) if s.id == "short000001"));
    }

    #[test]
    fn rejects_non_feed() {
        assert!(parse_channel_feed("<html><body>404</body></html>").is_none());
        assert!(parse_channel_feed("not xml").is_none());
    }
}
