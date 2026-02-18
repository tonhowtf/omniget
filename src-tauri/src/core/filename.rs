use regex::Regex;
use std::sync::LazyLock;
use unicode_normalization::UnicodeNormalization;

static WS_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());

pub fn sanitize_path_component(name: &str) -> String {
    let name: String = name.nfc().collect();
    let name = name.trim().replace(['\t', '\n'], "");
    let name = WS_RE.replace_all(&name, " ");
    let name = name.replace(" | ", "｜");

    let name = name.trim_end_matches([' ', '-', '.', ';']);

    let forbidden: &[(char, char)] = &[
        ('<', '＜'),
        ('>', '＞'),
        (':', '꞉'),
        ('"', '＂'),
        ('/', '⧸'),
        ('\\', '＼'),
        ('|', '｜'),
        ('?', '？'),
        ('*', ' '),
    ];

    let mut result = name.to_string();
    for (from, to) in forbidden {
        result = result.replace(*from, &to.to_string());
    }

    result.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_basic_forbidden_chars() {
        assert_eq!(sanitize_path_component("a:b?c"), "a꞉b？c");
    }

    #[test]
    fn sanitize_collapses_whitespace() {
        assert_eq!(sanitize_path_component("hello   world"), "hello world");
    }

    #[test]
    fn sanitize_trims_trailing_punctuation() {
        assert_eq!(sanitize_path_component("file name - "), "file name");
    }

    #[test]
    fn sanitize_unicode_nfc_normalization() {
        let decomposed = "e\u{0301}";
        let result = sanitize_path_component(decomposed);
        assert_eq!(result, "\u{00e9}");
    }

    #[test]
    fn sanitize_pipe_separator() {
        assert_eq!(sanitize_path_component("a | b"), "a｜b");
    }

    #[test]
    fn sanitize_long_path_with_special_chars() {
        let input = "Video: \"Best of 2024\" <HD> | 1080p";
        let result = sanitize_path_component(input);
        assert!(!result.contains(':'));
        assert!(!result.contains('"'));
        assert!(!result.contains('<'));
        assert!(!result.contains('>'));
    }

    #[test]
    fn sanitize_windows_forbidden_paths() {
        let chars = ['<', '>', ':', '"', '/', '\\', '|', '?', '*'];
        for c in chars {
            let input = format!("test{}file", c);
            let result = sanitize_path_component(&input);
            assert!(!result.contains(c), "char '{}' should be replaced", c);
        }
    }
}
