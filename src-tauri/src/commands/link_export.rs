use std::collections::HashSet;

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
