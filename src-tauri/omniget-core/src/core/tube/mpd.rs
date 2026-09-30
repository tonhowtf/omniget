//! Synthetic on-demand DASH manifest built from the `initRange`/`indexRange`
//! of adaptive formats, so dash.js can play separate video and audio
//! through the local proxy. Flow-Desktop `build_synthetic_dash_manifest`
//! (GPL-3.0).

use super::model::Format;

fn xml_escape(s: &str) -> String {
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

/// `PT3M32S` style duration (seconds precision is enough for SegmentBase).
fn iso_duration(s: u32) -> String {
    let (h, m, sec) = (s / 3600, (s % 3600) / 60, s % 60);
    let mut out = String::from("PT");
    if h > 0 {
        out.push_str(&format!("{h}H"));
    }
    if m > 0 {
        out.push_str(&format!("{m}M"));
    }
    out.push_str(&format!("{sec}S"));
    out
}

fn usable(f: &Format) -> bool {
    f.init_range.is_some() && f.index_range.is_some() && !f.codecs.is_empty()
}

fn representation(out: &mut String, f: &Format, attrs: &str, url_for: &dyn Fn(u32) -> String) {
    let (Some(init), Some(index)) = (&f.init_range, &f.index_range) else {
        return;
    };
    out.push_str(&format!(
        "      <Representation id=\"{}\" codecs=\"{}\" bandwidth=\"{}\"{}>\n",
        f.itag,
        xml_escape(&f.codecs),
        f.bitrate.max(1),
        attrs
    ));
    out.push_str(&format!(
        "        <BaseURL>{}</BaseURL>\n",
        xml_escape(&url_for(f.itag))
    ));
    out.push_str(&format!(
        "        <SegmentBase indexRange=\"{}-{}\" indexRangeExact=\"true\">\n",
        index.start, index.end
    ));
    out.push_str(&format!(
        "          <Initialization range=\"{}-{}\"/>\n",
        init.start, init.end
    ));
    out.push_str("        </SegmentBase>\n");
    out.push_str("      </Representation>\n");
}

/// Static MPD: one AdaptationSet per video container (every format passed
/// becomes a Representation) and one per audio (container, language).
/// Formats without init/index ranges are skipped. `url_for(itag)` returns
/// the proxy URL.
pub fn synthetic_mpd(
    duration_s: u32,
    video: &[Format],
    audio: &[Format],
    url_for: &dyn Fn(u32) -> String,
) -> String {
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(&format!(
        "<MPD xmlns=\"urn:mpeg:dash:schema:mpd:2011\" type=\"static\" profiles=\"urn:mpeg:dash:profile:isoff-on-demand:2011\" minBufferTime=\"PT1.5S\" mediaPresentationDuration=\"{}\">\n",
        iso_duration(duration_s.max(1))
    ));
    out.push_str("  <Period id=\"0\" start=\"PT0S\">\n");

    let mut set_id = 0u32;
    let mut video_groups: Vec<(&str, Vec<&Format>)> = Vec::new();
    for f in video.iter().filter(|f| usable(f)) {
        match video_groups.iter_mut().find(|(m, _)| *m == f.mime) {
            Some((_, v)) => v.push(f),
            None => video_groups.push((&f.mime, vec![f])),
        }
    }
    for (mime, reps) in &video_groups {
        let max_w = reps.iter().filter_map(|f| f.width).max().unwrap_or(0);
        let max_h = reps.iter().filter_map(|f| f.height).max().unwrap_or(0);
        out.push_str(&format!(
            "    <AdaptationSet id=\"{set_id}\" contentType=\"video\" mimeType=\"{}\" subsegmentAlignment=\"true\" subsegmentStartsWithSAP=\"1\" maxWidth=\"{max_w}\" maxHeight=\"{max_h}\">\n",
            xml_escape(mime)
        ));
        set_id += 1;
        let mut sorted = reps.clone();
        sorted.sort_by_key(|f| (f.height.unwrap_or(0), f.bitrate));
        for f in sorted {
            let attrs = format!(
                " width=\"{}\" height=\"{}\" frameRate=\"{}\" sar=\"1:1\"",
                f.width.unwrap_or(0),
                f.height.unwrap_or(0),
                f.fps.unwrap_or(30)
            );
            representation(&mut out, f, &attrs, url_for);
        }
        out.push_str("    </AdaptationSet>\n");
    }

    let mut audio_groups: Vec<((&str, String), Vec<&Format>)> = Vec::new();
    for f in audio.iter().filter(|f| usable(f)) {
        let key = (
            f.mime.as_str(),
            f.audio_lang.clone().unwrap_or_else(|| "und".into()),
        );
        match audio_groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, v)) => v.push(f),
            None => audio_groups.push((key, vec![f])),
        }
    }
    for ((mime, lang), reps) in &audio_groups {
        let main = reps.iter().any(|f| f.audio_default && !f.auto_dubbed);
        out.push_str(&format!(
            "    <AdaptationSet id=\"{set_id}\" contentType=\"audio\" mimeType=\"{}\" lang=\"{}\" subsegmentAlignment=\"true\" subsegmentStartsWithSAP=\"1\">\n",
            xml_escape(mime),
            xml_escape(lang)
        ));
        set_id += 1;
        out.push_str(&format!(
            "      <Role schemeIdUri=\"urn:mpeg:dash:role:2011\" value=\"{}\"/>\n",
            if main { "main" } else { "alternate" }
        ));
        if let Some(name) = reps.iter().find_map(|f| f.audio_name.as_deref()) {
            out.push_str(&format!("      <Label>{}</Label>\n", xml_escape(name)));
        }
        let mut sorted = reps.clone();
        sorted.sort_by_key(|f| f.bitrate);
        for f in sorted {
            representation(&mut out, f, " audioSamplingRate=\"48000\"", url_for);
        }
        out.push_str("    </AdaptationSet>\n");
    }

    out.push_str("  </Period>\n");
    out.push_str("</MPD>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::tube::model::{ByteRange, FormatKind};

    fn f(itag: u32, kind: FormatKind, mime: &str, codecs: &str, h: Option<u32>) -> Format {
        Format {
            itag,
            kind,
            mime: mime.into(),
            codecs: codecs.into(),
            width: h.map(|h| h * 16 / 9),
            height: h,
            fps: h.map(|_| 30),
            bitrate: 1_000 * itag as u64,
            content_length: Some(1_000_000),
            quality_label: None,
            audio_lang: None,
            audio_name: None,
            audio_default: true,
            auto_dubbed: false,
            hdr: false,
            drc: false,
            init_range: Some(ByteRange { start: 0, end: 700 }),
            index_range: Some(ByteRange {
                start: 701,
                end: 1600,
            }),
            url: "https://secret.googlevideo.com/videoplayback".into(),
        }
    }

    fn url_for(itag: u32) -> String {
        format!("http://127.0.0.1:1234/yt/s1/{itag}?k=tok&x=1")
    }

    #[test]
    fn valid_manifest_for_dashjs() {
        let video = vec![
            f(
                137,
                FormatKind::Video,
                "video/mp4",
                "avc1.640028",
                Some(1080),
            ),
            f(
                136,
                FormatKind::Video,
                "video/mp4",
                "avc1.4d401f",
                Some(720),
            ),
            f(248, FormatKind::Video, "video/webm", "vp9", Some(1080)),
        ];
        let mut a = f(140, FormatKind::Audio, "audio/mp4", "mp4a.40.2", None);
        a.audio_lang = Some("en".into());
        a.audio_name = Some("English <original>".into());
        let mut no_ranges = f(251, FormatKind::Audio, "audio/webm", "opus", None);
        no_ranges.index_range = None;
        let mpd = synthetic_mpd(212, &video, &[a, no_ranges], &url_for);

        let doc = roxmltree::Document::parse(&mpd).expect("well-formed XML");
        let root = doc.root_element();
        assert_eq!(root.tag_name().name(), "MPD");
        assert_eq!(root.attribute("type"), Some("static"));
        assert_eq!(root.attribute("mediaPresentationDuration"), Some("PT3M32S"));
        let sets: Vec<_> = root
            .descendants()
            .filter(|n| n.has_tag_name("AdaptationSet"))
            .collect();
        assert_eq!(sets.len(), 3);
        let reps: Vec<_> = root
            .descendants()
            .filter(|n| n.has_tag_name("Representation"))
            .collect();
        // Three video + one audio (251 had no index range).
        assert_eq!(reps.len(), 4);
        let r137 = reps
            .iter()
            .find(|r| r.attribute("id") == Some("137"))
            .unwrap();
        assert_eq!(r137.attribute("codecs"), Some("avc1.640028"));
        assert_eq!(r137.attribute("bandwidth"), Some("137000"));
        assert_eq!(r137.attribute("width"), Some("1920"));
        assert_eq!(r137.attribute("height"), Some("1080"));
        assert_eq!(r137.attribute("frameRate"), Some("30"));
        assert_eq!(
            r137.parent().unwrap().attribute("mimeType"),
            Some("video/mp4")
        );
        let base = r137
            .children()
            .find(|n| n.has_tag_name("BaseURL"))
            .unwrap()
            .text()
            .unwrap();
        assert_eq!(base, "http://127.0.0.1:1234/yt/s1/137?k=tok&x=1");
        let sb = r137
            .children()
            .find(|n| n.has_tag_name("SegmentBase"))
            .unwrap();
        assert_eq!(sb.attribute("indexRange"), Some("701-1600"));
        let init = sb
            .children()
            .find(|n| n.has_tag_name("Initialization"))
            .unwrap();
        assert_eq!(init.attribute("range"), Some("0-700"));
        let audio_set = sets
            .iter()
            .find(|s| s.attribute("contentType") == Some("audio"))
            .unwrap();
        assert_eq!(audio_set.attribute("lang"), Some("en"));
        assert!(mpd.contains("English &lt;original&gt;"));
        assert!(!mpd.contains("secret.googlevideo.com"));
    }

    #[test]
    fn durations() {
        assert_eq!(iso_duration(59), "PT59S");
        assert_eq!(iso_duration(3600), "PT1H0S");
        assert_eq!(iso_duration(3725), "PT1H2M5S");
    }
}
