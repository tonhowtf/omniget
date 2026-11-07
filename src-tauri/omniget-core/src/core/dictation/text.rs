use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Replacement {
    pub find: String,
    pub replace: String,
}

#[derive(Debug, Clone, Default)]
pub struct PostProcess<'a> {
    pub language: &'a str,
    pub capitalize: bool,
    pub remove_fillers: bool,
    pub replacements: &'a [Replacement],
}

struct FillerSet {
    hard: &'static [&'static str],
    soft: &'static [&'static str],
    spaced: bool,
}

fn fillers_for(language: &str) -> Option<FillerSet> {
    let base = language
        .trim()
        .to_ascii_lowercase()
        .split(['-', '_'])
        .next()
        .unwrap_or("")
        .to_string();
    let set = match base.as_str() {
        "en" => FillerSet {
            hard: &["u+m+", "u+h+m*", "e+r+m+", "e+r+", "a+h+", "h+m+", "m+h*m+"],
            soft: &["like", "you know", "i mean", "so", "well"],
            spaced: true,
        },
        "pt" => FillerSet {
            hard: &[
                "h+ã+", "ã+h*", "a+h+n+", "h+u+m+", "é{2,}h*", "e+h+", "h+m+",
            ],
            soft: &["é", "tipo", "né", "então", "assim", "sabe"],
            spaced: true,
        },
        "es" => FillerSet {
            hard: &["e+h+m*", "e+m+", "m+h*m+", "a+h+", "h+m+"],
            soft: &["este", "o sea", "pues", "bueno", "como"],
            spaced: true,
        },
        "fr" => FillerSet {
            hard: &["e+u+h+", "h+e+i+n+", "b+a+h+", "h+m+", "m+h*m+"],
            soft: &["ben", "genre", "bon", "voilà", "quoi"],
            spaced: true,
        },
        "it" => FillerSet {
            hard: &["e+h+m*", "e+m+", "m+h*m+", "a+h+", "h+m+"],
            soft: &["cioè", "tipo", "allora", "insomma"],
            spaced: true,
        },
        "de" => FillerSet {
            hard: &["ä+h+m*", "ö+h+m*", "e+h+m*", "h+m+", "m+h*m+"],
            soft: &["also", "halt", "sozusagen"],
            spaced: true,
        },
        "ru" => FillerSet {
            hard: &["э+м*", "м+м+", "а+м{2,}", "h+m+"],
            soft: &["ну", "типа", "короче", "вот"],
            spaced: true,
        },
        "ja" => FillerSet {
            hard: &[
                "えーと",
                "ええと",
                "えっと",
                "えー",
                "あのー",
                "うーん",
                "んー",
            ],
            soft: &[],
            spaced: false,
        },
        "zh" => FillerSet {
            hard: &["嗯+", "呃+", "额+"],
            soft: &[],
            spaced: false,
        },
        _ => return None,
    };
    Some(set)
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '\''
}

fn word_bounded(text: &str, start: usize, end: usize) -> bool {
    let before = text[..start].chars().next_back();
    let after = text[end..].chars().next();
    !before.is_some_and(is_word_char) && !after.is_some_and(is_word_char)
}

fn insensitive(pattern: &str) -> Option<Regex> {
    RegexBuilder::new(pattern)
        .case_insensitive(true)
        .build()
        .ok()
}

fn replace_bounded(
    text: &str,
    re: &Regex,
    bounded: bool,
    mut accept: impl FnMut(&str, usize, usize) -> Option<(usize, String)>,
) -> String {
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    let mut pos = 0;
    while pos <= text.len() {
        let Some(m) = re.find_at(text, pos) else {
            break;
        };
        if m.start() == m.end() {
            pos = m.end() + text[m.end()..].chars().next().map_or(1, char::len_utf8);
            continue;
        }
        let ok = !bounded || word_bounded(text, m.start(), m.end());
        match ok.then(|| accept(text, m.start(), m.end())).flatten() {
            Some((end, replacement)) => {
                out.push_str(&text[copied..m.start()]);
                out.push_str(&replacement);
                copied = end;
                pos = end;
            }
            None => {
                pos = m.start() + text[m.start()..].chars().next().map_or(1, char::len_utf8);
            }
        }
    }
    out.push_str(&text[copied..]);
    out
}

pub fn apply_replacements(text: &str, replacements: &[Replacement]) -> String {
    let mut rules: Vec<(&str, &str)> = replacements
        .iter()
        .map(|r| (r.find.trim(), r.replace.as_str()))
        .filter(|(find, _)| !find.is_empty())
        .collect();
    rules.sort_by_key(|(find, _)| std::cmp::Reverse(find.chars().count()));
    let mut spans: Vec<(usize, usize, usize)> = Vec::new();
    for (order, (find, _)) in rules.iter().enumerate() {
        let Some(re) = insensitive(&regex::escape(find)) else {
            continue;
        };
        let mut pos = 0;
        while pos <= text.len() {
            let Some(m) = re.find_at(text, pos) else {
                break;
            };
            if m.start() < m.end() && word_bounded(text, m.start(), m.end()) {
                spans.push((m.start(), m.end(), order));
            }
            pos = m.start() + text[m.start()..].chars().next().map_or(1, char::len_utf8);
        }
    }
    spans.sort_by_key(|&(start, _, order)| (start, order));
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    for (start, end, order) in spans {
        if start < copied {
            continue;
        }
        out.push_str(&text[copied..start]);
        out.push_str(rules[order].1);
        copied = end;
    }
    out.push_str(&text[copied..]);
    out
}

fn trailing_pause(text: &str, end: usize) -> Option<usize> {
    let rest = &text[end..];
    for mark in ["...", "…", ",", "，", "、"] {
        if let Some(stripped) = rest.strip_prefix(mark) {
            return Some(text.len() - stripped.len());
        }
    }
    None
}

fn clause_start(text: &str, start: usize) -> bool {
    let before = text[..start].trim_end();
    before.is_empty()
        || before.ends_with([
            ',', '.', '!', '?', ';', ':', '…', '，', '、', '。', '！', '？', '\n',
        ])
}

fn skip_spaces(text: &str, end: usize) -> usize {
    let rest = &text[end..];
    end + (rest.len() - rest.trim_start().len())
}

pub fn remove_fillers(text: &str, language: &str) -> String {
    let Some(set) = fillers_for(language) else {
        return text.to_string();
    };
    let mut current = text.to_string();
    for pattern in set.hard {
        let Some(re) = insensitive(pattern) else {
            continue;
        };
        current = replace_bounded(&current, &re, set.spaced, |t, _, end| {
            let end = trailing_pause(t, end).unwrap_or(end);
            Some((skip_spaces(t, end), String::new()))
        });
    }
    for word in set.soft {
        let Some(re) = insensitive(&regex::escape(word)) else {
            continue;
        };
        current = replace_bounded(&current, &re, set.spaced, |t, start, end| {
            let whole = t[..start].trim().is_empty() && t[end..].trim().is_empty();
            match trailing_pause(t, end) {
                Some(after) if clause_start(t, start) => {
                    Some((skip_spaces(t, after), String::new()))
                }
                None if whole => Some((t.len(), String::new())),
                _ => None,
            }
        });
    }
    tidy(&current)
}

pub fn tidy(text: &str) -> String {
    let mut s = text.split_whitespace().collect::<Vec<_>>().join(" ");
    for (from, to) in [
        (" ,", ","),
        (" .", "."),
        (" !", "!"),
        (" ?", "?"),
        (" ;", ";"),
        (" :", ":"),
        (",,", ","),
        (", .", "."),
        (",.", "."),
        (", ?", "?"),
        (", !", "!"),
    ] {
        while s.contains(from) {
            s = s.replace(from, to);
        }
    }
    let s = s.trim_start_matches([',', '，', '、', ';', ' ']).trim();
    let s = s.strip_suffix(',').unwrap_or(s).trim_end();
    s.to_string()
}

fn tidy_lines(text: &str) -> String {
    text.split('\n').map(tidy).collect::<Vec<_>>().join("\n")
}

pub fn capitalize_sentences(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at_start = true;
    let mut after_terminator = false;
    for c in text.chars() {
        if at_start && c.is_alphabetic() {
            out.extend(c.to_uppercase());
            at_start = false;
            after_terminator = false;
            continue;
        }
        if matches!(c, '.' | '!' | '?' | '。' | '！' | '？') {
            after_terminator = true;
        } else if c.is_whitespace() {
            if after_terminator {
                at_start = true;
            }
        } else if !at_start {
            after_terminator = false;
        } else if c.is_alphanumeric() {
            at_start = false;
            after_terminator = false;
        }
        out.push(c);
    }
    out
}

pub fn postprocess(raw: &str, opts: &PostProcess<'_>) -> String {
    let mut text = tidy(raw.trim());
    if opts.remove_fillers {
        text = remove_fillers(&text, opts.language);
    }
    text = tidy_lines(&apply_replacements(&text, opts.replacements));
    if opts.capitalize {
        text = capitalize_sentences(&text);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(find: &str, replace: &str) -> Replacement {
        Replacement {
            find: find.into(),
            replace: replace.into(),
        }
    }

    #[test]
    fn replacements_are_case_insensitive_and_whole_word() {
        let rules = [rule("omniget", "OmniGet"), rule("cat", "dog")];
        assert_eq!(
            apply_replacements("I use omniget and OMNIGET daily", &rules),
            "I use OmniGet and OmniGet daily"
        );
        assert_eq!(
            apply_replacements("concatenate the cat", &rules),
            "concatenate the dog"
        );
    }

    #[test]
    fn replacements_prefer_longer_phrases_and_handle_symbols() {
        let rules = [
            rule("new line", "\n"),
            rule("new", "NEW"),
            rule("c++", "C++"),
        ];
        assert_eq!(
            apply_replacements("new line please, new idea", &rules),
            "\n please, NEW idea"
        );
        assert_eq!(apply_replacements("I like c++.", &rules), "I like C++.");
    }

    #[test]
    fn replacements_do_not_rescan_inserted_text() {
        let rules = [rule("a", "b"), rule("b", "c")];
        assert_eq!(apply_replacements("a b", &rules), "b c");
        let rules = [rule("js", "JavaScript"), rule("javascript", "JS")];
        assert_eq!(apply_replacements("js", &rules), "JavaScript");
    }

    #[test]
    fn replacements_fall_back_to_shorter_rule_at_same_spot() {
        let rules = [rule("new line", "\n"), rule("new", "NEW")];
        assert_eq!(apply_replacements("new lines", &rules), "NEW lines");
    }

    #[test]
    fn replacements_ignore_empty_rules_and_keep_accents() {
        let rules = [rule("  ", "x"), rule("ação", "Ação")];
        assert_eq!(
            apply_replacements("uma ação, reação", &rules),
            "uma Ação, reação"
        );
    }

    #[test]
    fn english_fillers_go_and_real_words_stay() {
        assert_eq!(
            remove_fillers("Um, I think, uh, we should ship it.", "en"),
            "I think, we should ship it."
        );
        assert_eq!(
            remove_fillers("Ummm so hmm I like it", "en"),
            "so I like it"
        );
        assert_eq!(remove_fillers("It was, like, huge", "en"), "It was, huge");
        assert_eq!(remove_fillers("Humble umbrella", "en"), "Humble umbrella");
    }

    #[test]
    fn portuguese_soft_fillers_need_a_pause() {
        assert_eq!(remove_fillers("É, tipo, isso é bom", "pt"), "isso é bom");
        assert_eq!(
            remove_fillers("Ééé eu acho que é tipo de música", "pt"),
            "eu acho que é tipo de música"
        );
        assert_eq!(remove_fillers("Hã, vamos lá", "pt-BR"), "vamos lá");
    }

    #[test]
    fn soft_fillers_that_are_real_words_mid_clause_stay() {
        assert_eq!(
            remove_fillers("O importante é, sem dúvida, a qualidade", "pt"),
            "O importante é, sem dúvida, a qualidade"
        );
        assert_eq!(
            remove_fillers("Eu fiz assim, depois parei", "pt"),
            "Eu fiz assim, depois parei"
        );
        assert_eq!(
            remove_fillers("I like, really like it", "en"),
            "I like, really like it"
        );
        assert_eq!(remove_fillers("Ok. Então, vamos", "pt"), "Ok. vamos");
    }

    #[test]
    fn cjk_fillers_without_spaces() {
        assert_eq!(remove_fillers("嗯，我觉得可以", "zh"), "我觉得可以");
        assert_eq!(remove_fillers("えーと、明日行きます", "ja"), "明日行きます");
    }

    #[test]
    fn unknown_language_is_untouched() {
        assert_eq!(remove_fillers("um uh", "xx"), "um uh");
    }

    #[test]
    fn capitalizes_sentence_starts() {
        assert_eq!(
            capitalize_sentences("hello there. how are you? fine! ok"),
            "Hello there. How are you? Fine! Ok"
        );
        assert_eq!(capitalize_sentences("3 dogs. éramos"), "3 dogs. Éramos");
        assert_eq!(capitalize_sentences("v1.2 is out"), "V1.2 is out");
    }

    #[test]
    fn tidy_cleans_spacing_and_stray_commas() {
        assert_eq!(tidy("  , hello ,  world  , "), "hello, world");
        assert_eq!(tidy("done , ."), "done.");
    }

    #[test]
    fn postprocess_runs_the_whole_chain() {
        let rules = [rule("omniget", "OmniGet")];
        let opts = PostProcess {
            language: "en",
            capitalize: true,
            remove_fillers: true,
            replacements: &rules,
        };
        assert_eq!(
            postprocess("  um, open omniget. uh, then download ", &opts),
            "Open OmniGet. Then download"
        );
        let plain = PostProcess {
            language: "en",
            ..Default::default()
        };
        assert_eq!(postprocess("  um, keep   it ", &plain), "um, keep it");
    }

    #[test]
    fn postprocess_keeps_newlines_from_replacements() {
        let rules = [rule("new line", "\n"), rule("drop", "")];
        let opts = PostProcess {
            language: "en",
            capitalize: true,
            replacements: &rules,
            ..Default::default()
        };
        assert_eq!(postprocess("first new line second", &opts), "First\nsecond");
        assert_eq!(postprocess("keep drop , this", &opts), "Keep, this");
    }
}
