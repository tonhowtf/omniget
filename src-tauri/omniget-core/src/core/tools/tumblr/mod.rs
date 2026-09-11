//! Tumblr e favoritos de arte.
//!
//! Nada aqui reimplementa scraping: o gallery-dl já é um binário gerido do
//! projeto (`core/dependencies::ensure_gallerydl`) e já fala Tumblr,
//! DeviantArt, ArtStation e Flickr. O trabalho destas tools é pós-processar o
//! dump JSON dele e virar índice pesquisável (CSV/JSON) ou espelho HTML.

pub mod art;
pub mod backup;
pub mod gdl;
pub mod likes;

/// Escapa texto para caber dentro de um nó ou atributo HTML.
pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Célula de CSV no dialeto do resto do projeto (aspas dobradas).
pub fn csv_cell(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

pub fn csv_line(cells: &[String]) -> String {
    let mut line = cells
        .iter()
        .map(|c| csv_cell(c))
        .collect::<Vec<_>>()
        .join(",");
    line.push('\n');
    line
}

/// Nome de arquivo seguro e curto, para páginas geradas (tag, post, blog).
/// Só ASCII minúsculo, dígitos e hífen; nunca vazio, nunca `..`.
pub fn safe_slug(s: &str) -> String {
    let mut out = String::new();
    let mut last_dash = true;
    for c in s.trim().chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            out.push(c);
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
        if out.len() >= 60 {
            break;
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "sem-nome".to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapa_html_incluindo_aspas() {
        assert_eq!(
            esc(r#"<img src="x" onerror='a&b'>"#),
            "&lt;img src=&quot;x&quot; onerror=&#39;a&amp;b&#39;&gt;"
        );
    }

    #[test]
    fn csv_protege_virgula_aspas_e_quebra_de_linha() {
        assert_eq!(csv_cell("simples"), "simples");
        assert_eq!(csv_cell("a,b"), "\"a,b\"");
        assert_eq!(csv_cell("diz \"oi\""), "\"diz \"\"oi\"\"\"");
        assert_eq!(csv_cell("linha\nquebrada"), "\"linha\nquebrada\"");
    }

    #[test]
    fn slug_nunca_escapa_da_pasta() {
        assert_eq!(safe_slug("../../etc/passwd"), "etc-passwd");
        assert_eq!(safe_slug("Arte Digital / IA"), "arte-digital-ia");
        assert_eq!(safe_slug("   "), "sem-nome");
        assert_eq!(safe_slug("日本語"), "sem-nome");
        assert!(safe_slug(&"a".repeat(200)).len() <= 60);
    }
}
