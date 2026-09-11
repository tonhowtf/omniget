//! Categoria LinkedIn: tudo aqui roda em cima do export oficial que o usuario
//! baixa em Settings -> Data privacy -> Get a copy of your data. E dado
//! proprio, lido do disco, sem nenhuma requisicao de rede e sem tocar no site
//! — por isso a categoria existe.

pub mod checklist;
pub mod connections;
pub mod csv;
pub mod messages;
pub mod overview;
pub mod source;

use std::collections::HashMap;

use anyhow::Result;
use serde::Serialize;

/// Par nome/quantidade, usado nos rankings (empresas, cargos, contatos).
#[derive(Debug, Clone, Serialize)]
pub struct Count {
    pub name: String,
    pub count: usize,
}

/// Quantidade num periodo (`2021-08` ou `2021`).
#[derive(Debug, Clone, Serialize)]
pub struct Bucket {
    pub period: String,
    pub count: usize,
}

/// Ranking decrescente, empate resolvido pelo nome para a saida ser estavel.
pub fn top(map: &HashMap<String, usize>, limit: usize) -> Vec<Count> {
    let mut v: Vec<Count> = map
        .iter()
        .map(|(name, count)| Count {
            name: name.clone(),
            count: *count,
        })
        .collect();
    v.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
    v.truncate(limit);
    v
}

/// Serie temporal em ordem cronologica.
pub fn series(map: &HashMap<String, usize>) -> Vec<Bucket> {
    let mut v: Vec<Bucket> = map
        .iter()
        .map(|(period, count)| Bucket {
            period: period.clone(),
            count: *count,
        })
        .collect();
    v.sort_by(|a, b| a.period.cmp(&b.period));
    v
}

pub fn bump(map: &mut HashMap<String, usize>, key: &str) {
    if key.is_empty() {
        return;
    }
    *map.entry(key.to_string()).or_insert(0) += 1;
}

/// Pasta de saida dos arquivos exportados. Sem `out_dir` cai em
/// `<app_data>/tools/linkedin`.
pub fn out_dir(opt: Option<&str>) -> std::path::PathBuf {
    let dir = match opt {
        Some(d) if !d.trim().is_empty() => std::path::PathBuf::from(d),
        _ => super::tools_dir()
            .unwrap_or_else(super::temp_dir)
            .join("linkedin"),
    };
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Grava um arquivo de saida e devolve o caminho como string.
pub fn write_out(dir: &std::path::Path, name: &str, body: &str) -> Result<String> {
    let path = dir.join(super::sanitize_name(name));
    std::fs::write(&path, body)?;
    Ok(path.to_string_lossy().to_string())
}

/// Escapa texto para HTML (os exports viram pagina local).
pub fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
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

/// JSON embutido dentro de `<script>`: `<` vira `<` para nao fechar a tag.
pub fn json_for_script(v: &serde_json::Value) -> String {
    serde_json::to_string(v)
        .unwrap_or_else(|_| "null".to_string())
        .replace('<', "\\u003c")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranking_e_estavel_no_empate() {
        let mut m = HashMap::new();
        bump(&mut m, "Beta");
        bump(&mut m, "Acme");
        bump(&mut m, "Acme");
        bump(&mut m, "Ceta");
        bump(&mut m, "");
        let t = top(&m, 3);
        assert_eq!(t[0].name, "Acme");
        assert_eq!(t[0].count, 2);
        assert_eq!(t[1].name, "Beta");
        assert_eq!(t[2].name, "Ceta");
        assert_eq!(m.len(), 3);
    }

    #[test]
    fn serie_sai_em_ordem() {
        let mut m = HashMap::new();
        bump(&mut m, "2022-01");
        bump(&mut m, "2021-08");
        let s = series(&m);
        assert_eq!(s[0].period, "2021-08");
        assert_eq!(s[1].period, "2022-01");
    }

    #[test]
    fn escapa_html_e_script() {
        assert_eq!(
            html_escape("<b>&\"x\"</b>"),
            "&lt;b&gt;&amp;&quot;x&quot;&lt;/b&gt;"
        );
        let v = serde_json::json!({ "a": "</script>" });
        assert!(!json_for_script(&v).contains("</script>"));
    }
}
