//! Pure building blocks for the `/video` superpower (Watch): file naming,
//! natural order, course trees, subtitles, progress rules, IPTV playlists,
//! XMLTV guides, the open Stremio addon protocol and note export.

pub mod course;
pub mod m3u;
pub mod naming;
pub mod notes_md;
pub mod progress;
pub mod stremio;
pub mod subs;
pub mod xmltv;

pub use notes_md::fmt_ts;

#[cfg(test)]
mod fuzz {
    //! Throws deterministic garbage at every parser: none may panic.

    const ALPHABET: &[&str] = &[
        "a",
        "Z",
        "0",
        "1",
        "9",
        " ",
        ".",
        "-",
        "_",
        "/",
        "\\",
        "[",
        "]",
        "(",
        ")",
        "{",
        "}",
        ",",
        "=",
        "\"",
        "'",
        "|",
        "&",
        "#",
        ":",
        "\n",
        "\r",
        "é",
        "ç",
        "日",
        "→",
        "S",
        "E",
        "x",
        "-->",
        "#EXTINF:",
        "#EXTM3U",
        "#EXTVLCOPT:",
        "WEBVTT",
        "Dialogue:",
        "[Events]",
        "\\N",
        "{\\",
        "<i>",
        "<",
        ">",
        "00:00:01,000",
        "2019",
        "1080p",
        "\u{feff}",
    ];

    fn gen(seed: &mut u64, len: usize) -> String {
        let mut s = String::new();
        for _ in 0..len {
            *seed ^= *seed << 13;
            *seed ^= *seed >> 7;
            *seed ^= *seed << 17;
            s.push_str(ALPHABET[(*seed % ALPHABET.len() as u64) as usize]);
        }
        s
    }

    #[test]
    fn parsers_never_panic() {
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        for i in 0..3000 {
            let s = gen(&mut seed, 1 + i % 40);
            let _ = super::naming::parse(&s);
            let _ = super::naming::clean_title(&s);
            let _ = super::naming::natural_key(&s);
            let _ = super::subs::parse_auto(s.as_bytes());
            let _ = super::subs::parse_ass(&s);
            let _ = super::subs::to_vtt(&super::subs::parse_srt(&s), -1000);
            let _ = super::m3u::parse(&s);
            let _ = super::xmltv::parse_time(&s);
            let _ = super::xmltv::parse(s.as_bytes(), |_| {});
            let _ = super::stremio::catalog_url(&s, &s, &s, &[(s.clone(), s.clone())]);
            let _ = super::course::build(&[super::course::CourseFile { path: s.clone() }]);
            let bytes: Vec<u8> = s.bytes().map(|b| b.wrapping_mul(31)).collect();
            let _ = super::subs::parse_auto(&bytes);
        }
    }
}
