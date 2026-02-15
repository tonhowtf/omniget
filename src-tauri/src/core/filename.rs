use regex::Regex;

pub fn sanitize_path_component(name: &str) -> String {
    let name = name.trim().replace('\t', "").replace('\n', "");
    let name = Regex::new(r"\s+").unwrap().replace_all(&name, " ");
    let name = name.replace(" | ", "｜");

    let name = name.trim_end_matches(|c| matches!(c, ' ' | '-' | '.' | ';'));

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
