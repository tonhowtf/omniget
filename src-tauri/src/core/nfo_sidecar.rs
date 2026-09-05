use std::path::{Path, PathBuf};

use omniget_core::models::media::{MediaInfo, MediaType};

pub fn applies(info: &MediaInfo) -> bool {
    matches!(info.media_type, MediaType::Video | MediaType::Audio)
}

pub fn sidecar_path(file_path: &Path) -> PathBuf {
    file_path.with_extension("nfo")
}

pub fn render(info: &MediaInfo, source_url: &str) -> String {
    let mut buf = String::new();
    buf.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n");
    buf.push_str("<movie>\n");
    write_tag(&mut buf, "title", &info.title);
    if !info.author.trim().is_empty() {
        write_tag(&mut buf, "studio", &info.author);
    }
    if let Some(d) = info.duration_seconds {
        if d > 0.0 {
            write_tag(
                &mut buf,
                "runtime",
                &format!("{}", (d / 60.0).round().max(1.0) as i64),
            );
        }
    }
    if let Some(thumb) = info.thumbnail_url.as_deref() {
        write_tag(&mut buf, "thumb", thumb);
    }
    if !info.platform.trim().is_empty() {
        write_tag(&mut buf, "tag", &info.platform);
    }
    if !source_url.trim().is_empty() {
        buf.push_str("  <uniqueid type=\"url\" default=\"true\">");
        buf.push_str(&escape(source_url));
        buf.push_str("</uniqueid>\n");
    }
    write_tag(
        &mut buf,
        "dateadded",
        &chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
    );
    buf.push_str("</movie>\n");
    buf
}

pub async fn write(
    file_path: &Path,
    info: &MediaInfo,
    source_url: &str,
) -> std::io::Result<PathBuf> {
    let path = sidecar_path(file_path);
    tokio::fs::write(&path, render(info, source_url)).await?;
    Ok(path)
}

fn write_tag(buf: &mut String, name: &str, value: &str) {
    buf.push_str("  <");
    buf.push_str(name);
    buf.push('>');
    buf.push_str(&escape(value));
    buf.push_str("</");
    buf.push_str(name);
    buf.push_str(">\n");
}

fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn info(media_type: MediaType) -> MediaInfo {
        MediaInfo {
            title: "Tom & Jerry <Remastered>".into(),
            author: "Studio \"X\"".into(),
            platform: "youtube".into(),
            duration_seconds: Some(125.0),
            thumbnail_url: Some("https://img.example/t.jpg?a=1&b=2".into()),
            available_qualities: Vec::new(),
            media_type,
            file_size_bytes: None,
        }
    }

    #[test]
    fn escapes_xml_and_fills_known_fields() {
        let xml = render(&info(MediaType::Video), "https://youtu.be/abc?x=1&y=2");
        assert!(xml.starts_with("<?xml version=\"1.0\""));
        assert!(xml.contains("<title>Tom &amp; Jerry &lt;Remastered&gt;</title>"));
        assert!(xml.contains("<studio>Studio &quot;X&quot;</studio>"));
        assert!(xml.contains("<runtime>2</runtime>"));
        assert!(xml.contains("<thumb>https://img.example/t.jpg?a=1&amp;b=2</thumb>"));
        assert!(xml.contains("<tag>youtube</tag>"));
        assert!(xml.contains(
            "<uniqueid type=\"url\" default=\"true\">https://youtu.be/abc?x=1&amp;y=2</uniqueid>"
        ));
        assert!(xml.contains("<dateadded>"));
        assert!(xml.trim_end().ends_with("</movie>"));
    }

    #[test]
    fn skips_optional_fields_when_absent() {
        let mut i = info(MediaType::Audio);
        i.author = String::new();
        i.duration_seconds = None;
        i.thumbnail_url = None;
        let xml = render(&i, "");
        assert!(!xml.contains("<studio>"));
        assert!(!xml.contains("<runtime>"));
        assert!(!xml.contains("<thumb>"));
        assert!(!xml.contains("<uniqueid"));
    }

    #[test]
    fn only_video_and_audio_get_a_sidecar() {
        assert!(applies(&info(MediaType::Video)));
        assert!(applies(&info(MediaType::Audio)));
        assert!(!applies(&info(MediaType::Photo)));
        assert!(!applies(&info(MediaType::Carousel)));
        assert!(!applies(&info(MediaType::Course)));
    }

    #[test]
    fn sidecar_sits_next_to_the_media_file() {
        let p = sidecar_path(Path::new("/tmp/My.Video.mp4"));
        assert_eq!(p, PathBuf::from("/tmp/My.Video.nfo"));
    }
}
