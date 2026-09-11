// Portions adapted from cat-catch (js/m3u8.js)
// Copyright (c) xifangczy — https://github.com/xifangczy/cat-catch
// Licensed under GPL-3.0, same as this project.
//
//! Binary signature sniffing for downloaded media.
//!
//! A CDN that answers `200 OK` with an HTML error page is indistinguishable
//! from a real download until you look at the bytes: the file lands on disk
//! with the right name, the right extension and no way to play it. Reading the
//! first bytes tells the two apart before we hand the file to the user.
//!
//! The mirror of this module lives in `browser-extension/chrome/src/
//! media-signature.js`; the check order and the masks below are deliberately
//! identical on both sides, so keep them in sync.

/// Container formats we can recognise from their first bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaSignature {
    /// ISO-BMFF with a `ftyp` box: a plain MP4/M4A file.
    Mp4,
    /// ISO-BMFF fragment: `styp` (segment type) or `moof` (movie fragment).
    FragmentedMp4,
    /// Matroska/WebM (EBML header).
    WebM,
    /// MPEG audio, either with an `ID3` tag or a bare frame sync.
    Mp3,
    /// AAC in an ADTS frame.
    Aac,
    /// MPEG transport stream (the usual HLS segment).
    MpegTs,
}

/// Bytes scanned when looking for the MPEG-TS sync pattern.
const TS_SCAN_LIMIT: usize = 512;

/// MPEG-TS packets are 188 bytes long and each one starts with `0x47`.
const TS_PACKET_SIZE: usize = 188;

/// Minimum buffer we accept for a verdict. The ISO-BMFF box type only starts
/// at offset 4, so anything shorter cannot be classified without guessing.
const MIN_SNIFF_LEN: usize = 8;

/// Identify the container from the leading bytes of a download.
///
/// Returns `None` for anything we do not recognise, including HTML: an error
/// page is dropped here so it can never slip through a loose probe downstream.
/// Short and empty buffers are `None` as well — never a panic.
pub fn sniff_media_format(bytes: &[u8]) -> Option<MediaSignature> {
    if bytes.len() < MIN_SNIFF_LEN {
        return None;
    }

    // HTML first: an error page must never be classified as media.
    if looks_like_html(bytes) {
        return None;
    }

    // MPEG-TS before the byte-oriented audio checks. A transport stream can
    // legitimately start with any byte, so testing it last would let the loose
    // MP3 sync mask claim segments that are really TS.
    if is_mpeg_ts(bytes) {
        return Some(MediaSignature::MpegTs);
    }

    // ISO-BMFF: `<size><box type>`, so the type lives at bytes 4..8.
    match &bytes[4..8] {
        b"styp" | b"moof" => return Some(MediaSignature::FragmentedMp4),
        b"ftyp" => return Some(MediaSignature::Mp4),
        _ => {}
    }

    // Matroska/WebM EBML header.
    if bytes.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        return Some(MediaSignature::WebM);
    }

    // MP3 with an ID3 tag.
    if bytes.starts_with(b"ID3") {
        return Some(MediaSignature::Mp3);
    }

    // ADTS and a bare MP3 frame sync both start with 0xFF, so the order and
    // the masks here decide which one wins.
    //
    // ADTS is tested first with the *more specific* 0xF6 mask: it checks the
    // 12-bit sync and the two layer bits at once. ADTS always encodes layer
    // `00`; a real MPEG audio frame never does. So 0xF6 matches exactly
    // `F0 F1 F8 F9` (ADTS) and rejects `FA FB F2 F3` (MP3). The naive 0xF0
    // mask would swallow `FF FB`, which is the most common MPEG-1 Layer III
    // header, and turn every real MP3 into an `Aac`.
    if bytes[0] == 0xFF && (bytes[1] & 0xF6) == 0xF0 {
        return Some(MediaSignature::Aac);
    }

    // Whatever is left with a valid 11-bit sync is MPEG audio.
    if bytes[0] == 0xFF && (bytes[1] & 0xE0) == 0xE0 {
        return Some(MediaSignature::Mp3);
    }

    None
}

/// Whether the buffer opens like an HTML document.
///
/// Tolerates a UTF-8 BOM and leading whitespace, and is case-insensitive:
/// error pages come in every shape.
///
/// A leading `<?xml` is deliberately not enough on its own. Plenty of files we
/// are asked to download are XML and perfectly valid — an SVG exported by any
/// design tool opens with an XML declaration, and so does a DASH manifest.
/// Treating the declaration as proof of an error page would delete them.
///
/// What separates the two is the element that follows: an XHTML error page
/// carries `<html`, and the XML error bodies S3-style object stores return
/// carry `<Error`. Content roots (`<svg`, `<MPD`, `<rss`) carry neither.
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

/// A download worth keeping: it carries a signature we know and it is not an
/// error page dressed up as a media file.
pub fn is_plausible_media(bytes: &[u8]) -> bool {
    sniff_media_format(bytes).is_some() && !looks_like_html(bytes)
}

/// Two `0x47` sync bytes exactly one packet apart, within the scanned window.
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
        // An SVG exported by Inkscape or Illustrator opens exactly like this,
        // and `svg` is an extension the direct downloader accepts.
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
        // FF E3: 11-bit sync, layer bits set — not ADTS.
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
        // The pair the naive 0xF0 mask confuses: FF FB is the most common
        // MPEG-1 Layer III header, FF F9 is an ADTS frame.
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
        // The rest of the family, for good measure.
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
        // One stray 0x47 with nothing 188 bytes later proves nothing.
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
        // The body an S3-style object store returns for a denied or expired
        // link. It is XML, but it is still a page and not a file.
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
        let bytes = padded(&[0x50, 0x4B, 0x03, 0x04]); // a zip file
        assert_eq!(sniff_media_format(&bytes), None);
        assert!(!is_plausible_media(&bytes));
        // ...and it is not HTML either, which is what keeps the downloader
        // from rejecting containers it simply cannot sniff.
        assert!(!looks_like_html(&bytes));
    }
}
