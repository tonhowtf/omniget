use serde::{Deserialize, Serialize};

use super::{compatible_format_selector, download_format_selector};
use crate::models::media::FormatInfo;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioLanguage {
    pub code: String,
    pub label: String,
    pub original: bool,
}

pub fn valid_audio_language(code: &str) -> bool {
    (2..=12).contains(&code.len()) && code.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

const NOT_A_TRACK_NAME: [&str; 7] = [
    "low", "medium", "high", "ultralow", "default", "drc", "premium",
];

fn track_name(format_note: &str) -> Option<String> {
    let first = format_note
        .split(',')
        .next()
        .unwrap_or("")
        .replace("(default)", "")
        .replace("(original)", "");
    let mut name = first.trim();
    for suffix in ["original", "dubbed-auto", "dubbed"] {
        name = name.strip_suffix(suffix).unwrap_or(name).trim_end();
    }
    let name = name.trim_end_matches(['-', ' ']);
    (!name.is_empty()
        && !name.chars().any(|c| c.is_ascii_digit())
        && !NOT_A_TRACK_NAME
            .iter()
            .any(|word| name.eq_ignore_ascii_case(word)))
    .then(|| name.to_owned())
}

pub fn audio_languages(formats: &[FormatInfo]) -> Vec<AudioLanguage> {
    let mut found: Vec<AudioLanguage> = Vec::new();
    for format in formats.iter().filter(|f| f.has_audio) {
        let Some(code) = format
            .language
            .as_deref()
            .filter(|code| valid_audio_language(code))
        else {
            continue;
        };
        let note = format.format_note.as_deref().unwrap_or("");
        let original = note.to_ascii_lowercase().contains("original");
        let name = track_name(note).filter(|name| name != code);
        match found.iter_mut().find(|known| known.code == code) {
            Some(known) => {
                known.original |= original;
                if known.label == known.code {
                    if let Some(name) = name {
                        known.label = name;
                    }
                }
            }
            None => found.push(AudioLanguage {
                code: code.to_owned(),
                label: name.unwrap_or_else(|| code.to_owned()),
                original,
            }),
        }
    }
    found.sort_by_key(|language| !language.original);
    found
}

fn split_outside_brackets(selector: &str, separator: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (index, c) in selector.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => depth = depth.saturating_sub(1),
            c if c == separator && depth == 0 => {
                parts.push(&selector[start..index]);
                start = index + c.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&selector[start..]);
    parts
}

fn is_audio_part(part: &str) -> bool {
    let name = part.split('[').next().unwrap_or("");
    matches!(
        name.split('.').next().unwrap_or(""),
        "ba" | "ba*" | "bestaudio" | "bestaudio*" | "wa" | "wa*" | "worstaudio" | "worstaudio*"
    )
}

pub fn with_audio_language(selector: &str, code: &str) -> Option<String> {
    if !valid_audio_language(code) {
        return None;
    }
    let narrowed: Vec<String> = split_outside_brackets(selector, '/')
        .into_iter()
        .filter_map(|alternative| {
            let parts = split_outside_brackets(alternative, '+');
            parts.iter().any(|part| is_audio_part(part)).then(|| {
                parts
                    .iter()
                    .map(|part| {
                        if is_audio_part(part) {
                            format!("{part}[language^={code}]")
                        } else {
                            (*part).to_owned()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("+")
            })
        })
        .collect();
    if narrowed.is_empty() {
        return Some(selector.to_owned());
    }
    Some(format!("{}/{}", narrowed.join("/"), selector))
}

pub fn audio_language_format(
    format_id: Option<&str>,
    quality_height: Option<u32>,
    mode: &str,
    compatible_video: bool,
    ffmpeg_available: bool,
    code: &str,
) -> Option<String> {
    if mode == "mute" || !valid_audio_language(code) {
        return format_id.map(str::to_owned);
    }
    if let Some(chosen) = format_id {
        return with_audio_language(chosen, code);
    }
    let base = if mode != "audio" && compatible_video {
        compatible_format_selector(quality_height, "auto", ffmpeg_available)
    } else {
        let mode = if mode == "audio" { "audio" } else { "auto" };
        download_format_selector(None, quality_height, mode, ffmpeg_available)
    };
    with_audio_language(&base, code).filter(|narrowed| *narrowed != base)
}

pub fn cached_audio_languages(url: &str) -> Vec<AudioLanguage> {
    super::info::cached_video_info(url)
        .map(|json| audio_languages(&super::parse_formats(&json)))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<FormatInfo> {
        let json = serde_json::json!({ "formats": [
            { "format_id": "sb3", "ext": "mhtml", "vcodec": "none", "acodec": "none", "format_note": "storyboard" },
            { "format_id": "233-12", "ext": "mp4", "vcodec": "none", "acodec": "mp4a.40.5", "language": "pa", "format_note": "pa - dubbed" },
            { "format_id": "233-14", "ext": "mp4", "vcodec": "none", "acodec": "mp4a.40.5", "language": "pt", "format_note": "Português - dubbed" },
            { "format_id": "233-23", "ext": "mp4", "vcodec": "none", "acodec": "mp4a.40.5", "language": "en-US", "language_preference": 10, "format_note": "American English - original (original)" },
            { "format_id": "139-5", "ext": "m4a", "vcodec": "none", "acodec": "mp4a.40.5", "language": "pa", "language_preference": -1, "format_note": "Punjabi, low" },
            { "format_id": "139-10", "ext": "m4a", "vcodec": "none", "acodec": "mp4a.40.5", "language": "pt", "language_preference": -1, "format_note": "Portuguese, low" },
            { "format_id": "139-23", "ext": "m4a", "vcodec": "none", "acodec": "mp4a.40.5", "language": "en-US", "language_preference": 10, "format_note": "English (US) original (default), low" },
            { "format_id": "140-23", "ext": "m4a", "vcodec": "none", "acodec": "mp4a.40.2", "language": "en-US", "language_preference": 10, "format_note": "English (US) original (default), medium" },
            { "format_id": "251", "ext": "webm", "vcodec": "none", "acodec": "opus", "language": "none", "format_note": "medium" },
            { "format_id": "250", "ext": "webm", "vcodec": "none", "acodec": "opus", "language": "", "format_note": "low" },
            { "format_id": "137", "ext": "mp4", "vcodec": "avc1.640028", "acodec": "none", "height": 1080, "width": 1920, "language": "en-US", "format_note": "1080p" },
            { "format_id": "616", "ext": "mp4", "vcodec": "vp09.00.40.08", "acodec": "none", "height": 1080, "width": 1920, "format_note": "Premium" }
        ]});
        super::super::parse_formats(&json)
    }

    #[test]
    fn the_parser_keeps_the_language_and_drops_empty_or_none() {
        let formats = fixture();
        let language_of = |id: &str| {
            formats
                .iter()
                .find(|f| f.format_id == id)
                .and_then(|f| f.language.clone())
        };
        assert_eq!(language_of("139-10").as_deref(), Some("pt"));
        assert_eq!(language_of("140-23").as_deref(), Some("en-US"));
        assert_eq!(language_of("251"), None);
        assert_eq!(language_of("250"), None);
        assert_eq!(language_of("616"), None);
    }

    #[test]
    fn each_language_is_listed_once_with_the_original_first() {
        let languages = audio_languages(&fixture());
        assert_eq!(
            languages,
            vec![
                AudioLanguage {
                    code: "en-US".into(),
                    label: "American English".into(),
                    original: true
                },
                AudioLanguage {
                    code: "pa".into(),
                    label: "Punjabi".into(),
                    original: false
                },
                AudioLanguage {
                    code: "pt".into(),
                    label: "Português".into(),
                    original: false
                },
            ]
        );
    }

    #[test]
    fn a_video_format_with_a_language_is_not_an_audio_track() {
        let only_video: Vec<FormatInfo> = fixture()
            .into_iter()
            .filter(|f| f.format_id == "137")
            .collect();
        assert!(audio_languages(&only_video).is_empty());
    }

    #[test]
    fn a_language_without_a_readable_name_is_labelled_with_its_code() {
        let json = serde_json::json!({ "formats": [
            { "format_id": "a", "ext": "m4a", "vcodec": "none", "acodec": "mp4a", "language": "de", "format_note": "128k" },
            { "format_id": "b", "ext": "m4a", "vcodec": "none", "acodec": "mp4a", "language": "fr" }
        ]});
        let languages = audio_languages(&super::super::parse_formats(&json));
        assert_eq!(languages[0].label, "de");
        assert_eq!(languages[1].label, "fr");
        assert!(!languages[0].original && !languages[1].original);
    }

    #[test]
    fn a_language_code_is_letters_digits_and_hyphens_only() {
        for good in ["pt", "en-US", "zh-Hans", "es-419", "en-US-desc"] {
            assert!(valid_audio_language(good), "{good}");
        }
        for bad in [
            "",
            "p",
            "pt]",
            "pt]/b",
            "pt BR",
            "pt_BR",
            "pt\"",
            "pt;id",
            "$(id)",
            "pt\n",
            "português",
            "aaaaaaaaaaaaa",
        ] {
            assert!(!valid_audio_language(bad), "{bad:?}");
        }
    }

    #[test]
    fn every_audio_part_is_narrowed_and_the_plain_selector_stays_as_fallback() {
        assert_eq!(
            with_audio_language("bv*+ba/b", "pt").as_deref(),
            Some("bv*+ba[language^=pt]/bv*+ba/b")
        );
        assert_eq!(
            with_audio_language("ba/b", "en-US").as_deref(),
            Some("ba[language^=en-US]/ba/b")
        );
        assert_eq!(
            with_audio_language(
                "bv*[height<=720]+ba[ext=m4a]/bv*[height<=720]+ba/b[height<=720]/b",
                "pt"
            )
            .as_deref(),
            Some("bv*[height<=720]+ba[ext=m4a][language^=pt]/bv*[height<=720]+ba[language^=pt]/bv*[height<=720]+ba[ext=m4a]/bv*[height<=720]+ba/b[height<=720]/b")
        );
        assert_eq!(
            with_audio_language("137+bestaudio/best", "ja").as_deref(),
            Some("137+bestaudio[language^=ja]/137+bestaudio/best")
        );
    }

    #[test]
    fn a_selector_without_an_audio_part_is_left_alone() {
        assert_eq!(with_audio_language("b/bv*", "pt").as_deref(), Some("b/bv*"));
        assert_eq!(
            with_audio_language("137+140", "pt").as_deref(),
            Some("137+140")
        );
        assert_eq!(with_audio_language("bv*", "pt").as_deref(), Some("bv*"));
    }

    #[test]
    fn a_slash_inside_a_filter_does_not_split_the_selector() {
        assert_eq!(
            with_audio_language("bv*[format_note*=a/b]+ba/b", "pt").as_deref(),
            Some("bv*[format_note*=a/b]+ba[language^=pt]/bv*[format_note*=a/b]+ba/b")
        );
    }

    #[test]
    fn a_hostile_language_never_reaches_a_selector() {
        for bad in ["pt]/b+ba[ext=exe", "pt --exec id", "$(id)", "pt;id", ""] {
            assert_eq!(with_audio_language("bv*+ba/b", bad), None, "{bad:?}");
            assert_eq!(
                audio_language_format(None, Some(720), "auto", false, true, bad),
                None,
                "{bad:?}"
            );
        }
    }

    #[test]
    fn the_default_selection_is_narrowed_and_keeps_the_height() {
        let best = audio_language_format(None, None, "auto", false, true, "pt").unwrap();
        assert_eq!(
            best,
            "bv*+ba[ext=m4a][language^=pt]/bv*+ba[language^=pt]/bv*+ba[ext=m4a]/bv*+ba/b"
        );
        let capped = audio_language_format(None, Some(720), "auto", false, true, "pt").unwrap();
        assert!(capped.starts_with("bv*[height<=720]+ba[ext=m4a][language^=pt]/"));
        assert!(capped.ends_with("/b[height<=720]/b"));
    }

    #[test]
    fn the_compatible_setting_keeps_preferring_h264_with_the_chosen_language() {
        let selector = audio_language_format(None, Some(1080), "auto", true, true, "pt").unwrap();
        assert!(
            selector.starts_with("bv*[vcodec^=avc1][height<=1080]+ba[acodec^=mp4a][language^=pt]/")
        );
        assert!(super::super::prefers_compatible_video(&selector));
        assert!(!super::super::prefers_compatible_video(
            &audio_language_format(None, Some(1080), "auto", false, true, "pt").unwrap()
        ));
    }

    #[test]
    fn audio_only_downloads_pick_the_language_too() {
        assert_eq!(
            audio_language_format(None, None, "audio", true, true, "pt").as_deref(),
            Some("ba[language^=pt]/ba/b")
        );
    }

    #[test]
    fn a_muted_download_or_one_with_nothing_to_narrow_keeps_its_format() {
        assert_eq!(
            audio_language_format(None, Some(720), "mute", false, true, "pt"),
            None
        );
        assert_eq!(
            audio_language_format(None, Some(720), "auto", false, false, "pt"),
            None
        );
        assert_eq!(
            audio_language_format(Some("137+140"), None, "auto", false, true, "pt").as_deref(),
            Some("137+140")
        );
    }

    #[test]
    fn a_chosen_selector_with_an_audio_part_is_narrowed() {
        assert_eq!(
            audio_language_format(Some("137+ba"), None, "auto", true, true, "pt").as_deref(),
            Some("137+ba[language^=pt]/137+ba")
        );
    }
}
