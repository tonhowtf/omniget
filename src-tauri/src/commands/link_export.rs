//! Downloads › Tools › Export links: save the links of the history (all,
//! finished or failed) as a .txt, one per line, to retry them later or open
//! them in another program. Downloads › Tools › Link router opens the same
//! file back.
//!
//! The idea comes from Vividl (Bluegrams/Vividl), which imports and exports
//! its download list as a plain text file of links. Reimplemented; no code
//! was copied.

use std::collections::HashSet;

/// One link per line, in the given order, without repeats, after a `#`
/// comment line that link lists (and Link router) skip.
pub fn link_list_text(header: &str, urls: &[String]) -> String {
    let mut seen = HashSet::new();
    let mut out = format!("# {}\n", header.replace('\n', " "));
    for u in urls {
        let u = u.trim();
        if u.is_empty() || u.contains('\n') || !seen.insert(u.to_string()) {
            continue;
        }
        out.push_str(u);
        out.push('\n');
    }
    out
}

#[tauri::command]
pub fn export_link_list(path: String, header: String, urls: Vec<String>) -> Result<usize, String> {
    let p = std::path::Path::new(&path);
    let is_txt = p
        .extension()
        .map(|e| e.eq_ignore_ascii_case("txt"))
        .unwrap_or(false);
    if !is_txt {
        return Err("Save the list as a .txt file".to_string());
    }
    let text = link_list_text(&header, &urls);
    std::fs::write(p, &text).map_err(|e| e.to_string())?;
    Ok(text.lines().filter(|l| !l.starts_with('#')).count())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_link_per_line_without_repeats() {
        let urls = vec![
            "https://a/1".to_string(),
            " https://a/2 ".to_string(),
            "https://a/1".to_string(),
            String::new(),
            "bad\nline".to_string(),
        ];
        assert_eq!(
            link_list_text("OmniGet, failed", &urls),
            "# OmniGet, failed\nhttps://a/1\nhttps://a/2\n"
        );
    }

    #[test]
    fn only_txt_files() {
        let err = export_link_list("/tmp/list.sh".into(), "x".into(), vec![]).unwrap_err();
        assert!(err.contains(".txt"));
    }
}
