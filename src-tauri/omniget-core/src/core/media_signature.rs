// Copyright (c) xifangczy — https://github.com/xifangczy/cat-catch
// Licensed under GPL-3.0, same as this project.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaSignature {
    Mp4,
    FragmentedMp4,
    WebM,
    Mp3,
    Aac,
    MpegTs,
}

const TS_SCAN_LIMIT: usize = 512;

const TS_PACKET_SIZE: usize = 188;

const MIN_SNIFF_LEN: usize = 8;

pub fn sniff_media_format(bytes: &[u8]) -> Option<MediaSignature> {
    if bytes.len() < MIN_SNIFF_LEN {
        return None;
    }

    if looks_like_html(bytes) {
        return None;
    }

    if is_mpeg_ts(bytes) {
        return Some(MediaSignature::MpegTs);
    }

    match &bytes[4..8] {
        b"styp" | b"moof" => return Some(MediaSignature::FragmentedMp4),
        b"ftyp" => return Some(MediaSignature::Mp4),
        _ => {}
    }

    if bytes.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        return Some(MediaSignature::WebM);
    }

    if bytes.starts_with(b"ID3") {
        return Some(MediaSignature::Mp3);
    }

    if bytes[0] == 0xFF && (bytes[1] & 0xF6) == 0xF0 {
        return Some(MediaSignature::Aac);
    }

    if bytes[0] == 0xFF && (bytes[1] & 0xE0) == 0xE0 {
        return Some(MediaSignature::Mp3);
    }

    None
}

pub fn looks_like_html(bytes: &[u8]) -> bool {
    let mut rest = bytes;
    if let Some(stripped) = rest.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        rest = stripped;
    }
    let start = rest
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(rest.len());
    let rest = &rest[start..];

    for marker in [b"<!doctype".as_slice(), b"<html".as_slice()] {
        if rest.len() >= marker.len() && rest[..marker.len()].eq_ignore_ascii_case(marker) {
            return true;
        }
    }

    if starts_with_ignore_ascii_case(rest, b"<?xml") {
        return contains_ignore_ascii_case(rest, b"<html")
            || contains_ignore_ascii_case(rest, b"<error");
    }

    false
}

fn starts_with_ignore_ascii_case(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.len() >= needle.len() && haystack[..needle.len()].eq_ignore_ascii_case(needle)
}

fn contains_ignore_ascii_case(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return false;
    }
    haystack
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle))
}

pub fn is_plausible_media(bytes: &[u8]) -> bool {
    sniff_media_format(bytes).is_some() && !looks_like_html(bytes)
}

fn is_mpeg_ts(bytes: &[u8]) -> bool {
    let limit = bytes.len().min(TS_SCAN_LIMIT);
    bytes
        .iter()
        .take(limit)
        .enumerate()
        .any(|(i, &b)| b == 0x47 && bytes.get(i + TS_PACKET_SIZE) == Some(&0x47))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_declaration_alone_is_not_an_error_page() {
        let svg = br#"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"></svg>"#;
        assert!(!looks_like_html(svg));

        let mpd = br#"<?xml version="1.0"?><MPD xmlns="urn:mpeg:dash:schema:mpd:2011"></MPD>"#;
        assert!(!looks_like_html(mpd));

        let rss = br#"<?xml version="1.0"?><rss version="2.0"><channel></channel></rss>"#;
        assert!(!looks_like_html(rss));
    }

    #[test]
    fn xhtml_error_page_is_still_caught() {
        let xhtml = br#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html PUBLIC "-//W3C//DTD XHTML 1.0 Strict//EN">
<html xmlns="http://www.w3.org/1999/xhtml"><body>403 Forbidden</body></html>"#;
        assert!(looks_like_html(xhtml));

        let upper = br#"<?XML version="1.0"?><HTML><body>nope</body></HTML>"#;
        assert!(looks_like_html(upper));
    }

    fn padded(prefix: &[u8]) -> Vec<u8> {
        let mut v = prefix.to_vec();
        v.resize(64, 0);
        v
    }

    fn iso_bmff(box_type: &[u8]) -> Vec<u8> {
        let mut v = vec![0x00, 0x00, 0x00, 0x20];
        v.extend_from_slice(box_type);
        v.resize(64, 0);
        v
    }

    #[test]
    fn detects_plain_mp4() {
        let bytes = iso_bmff(b"ftyp");
        assert_eq!(sniff_media_format(&bytes), Some(MediaSignature::Mp4));
        assert!(is_plausible_media(&bytes));
    }

    #[test]
    fn detects_fragmented_mp4_styp() {
        assert_eq!(
            sniff_media_format(&iso_bmff(b"styp")),
            Some(MediaSignature::FragmentedMp4)
        );
    }

    #[test]
    fn detects_fragmented_mp4_moof() {
        assert_eq!(
            sniff_media_format(&iso_bmff(b"moof")),
            Some(MediaSignature::FragmentedMp4)
        );
    }

    #[test]
    fn detects_webm() {
        let bytes = padded(&[0x1A, 0x45, 0xDF, 0xA3]);
        assert_eq!(sniff_media_format(&bytes), Some(MediaSignature::WebM));
    }

    #[test]
    fn detects_mp3_by_id3_tag() {
        let bytes = padded(b"ID3\x03\x00\x00\x00\x00");
        assert_eq!(sniff_media_format(&bytes), Some(MediaSignature::Mp3));
    }

    #[test]
    fn detects_mp3_by_frame_sync() {
        let bytes = padded(&[0xFF, 0xE3, 0x00, 0x00]);
        assert_eq!(sniff_media_format(&bytes), Some(MediaSignature::Mp3));
    }

    #[test]
    fn detects_aac_adts() {
        let bytes = padded(&[0xFF, 0xF1, 0x50, 0x80]);
        assert_eq!(sniff_media_format(&bytes), Some(MediaSignature::Aac));
    }

    #[test]
    fn adts_mask_does_not_steal_common_mp3_header() {
        assert_eq!(
            sniff_media_format(&padded(&[0xFF, 0xFB, 0x90, 0x00])),
            Some(MediaSignature::Mp3),
            "FF FB is MP3, not AAC"
        );
        assert_eq!(
            sniff_media_format(&padded(&[0xFF, 0xF9, 0x50, 0x80])),
            Some(MediaSignature::Aac),
            "FF F9 is ADTS"
        );
        for b1 in [0xF0u8, 0xF1, 0xF8, 0xF9] {
            assert_eq!(
                sniff_media_format(&padded(&[0xFF, b1, 0x50, 0x80])),
                Some(MediaSignature::Aac),
                "FF {b1:02X} should be ADTS"
            );
        }
        for b1 in [0xFAu8, 0xFB, 0xF2, 0xF3] {
            assert_eq!(
                sniff_media_format(&padded(&[0xFF, b1, 0x90, 0x00])),
                Some(MediaSignature::Mp3),
                "FF {b1:02X} should be MP3"
            );
        }
    }

    #[test]
    fn detects_mpeg_ts() {
        let mut bytes = vec![0u8; 400];
        bytes[0] = 0x47;
        bytes[188] = 0x47;
        bytes[376] = 0x47;
        assert_eq!(sniff_media_format(&bytes), Some(MediaSignature::MpegTs));
    }

    #[test]
    fn single_ts_sync_byte_is_not_enough() {
        let mut bytes = vec![0u8; 400];
        bytes[0] = 0x47;
        assert_eq!(sniff_media_format(&bytes), None);
    }

    #[test]
    fn html_error_page_is_rejected() {
        let page = b"<!DOCTYPE html>\n<html><body>403 Forbidden</body></html>";
        assert_eq!(sniff_media_format(page), None);
        assert!(looks_like_html(page));
        assert!(!is_plausible_media(page));
    }

    #[test]
    fn html_with_bom_and_leading_whitespace() {
        let page = b"\xEF\xBB\xBF\r\n   <HtMl><head></head></html>";
        assert!(looks_like_html(page));
        assert_eq!(sniff_media_format(page), None);
    }

    #[test]
    fn xml_error_document_is_html_like() {
        let page = b"<?xml version=\"1.0\"?><Error><Code>AccessDenied</Code></Error>";
        assert!(looks_like_html(page));
        assert_eq!(sniff_media_format(page), None);
    }

    #[test]
    fn real_media_is_not_html() {
        assert!(!looks_like_html(&iso_bmff(b"ftyp")));
        assert!(!looks_like_html(&padded(&[0xFF, 0xFB, 0x90, 0x00])));
    }

    #[test]
    fn short_buffers_never_panic() {
        assert_eq!(sniff_media_format(&[]), None);
        assert_eq!(sniff_media_format(&[0xFF, 0xFB, 0x90]), None);
        assert_eq!(sniff_media_format(&[0x00, 0x00, 0x00, 0x20, b'f']), None);
        assert!(!looks_like_html(&[]));
        assert!(!looks_like_html(&[0xEF, 0xBB]));
        assert!(!looks_like_html(b"<"));
        assert!(!is_plausible_media(&[]));
        assert!(!is_plausible_media(&[0x47]));
    }

    #[test]
    fn unknown_binary_is_not_media() {
        let bytes = padded(&[0x50, 0x4B, 0x03, 0x04]);
        assert_eq!(sniff_media_format(&bytes), None);
        assert!(!is_plausible_media(&bytes));
        assert!(!looks_like_html(&bytes));
    }
}
