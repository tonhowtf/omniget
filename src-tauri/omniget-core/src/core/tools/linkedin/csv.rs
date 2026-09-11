//! Parser de CSV escrito a mao para os exports do LinkedIn. O projeto nao tem
//! o crate `csv` e esta rodada nao pode adicionar dependencia, entao o basico
//! mora aqui: aspas, aspas escapadas (`""`), quebra de linha dentro do campo,
//! separador detectado (`,`, `;` ou tab), BOM e o preambulo "Notes:" que
//! alguns CSVs do LinkedIn trazem antes do cabecalho de verdade.

use std::collections::HashMap;

/// Normaliza um nome de coluna ou de arquivo: minusculas, so alfanumerico.
/// "First Name" e "FIRST_NAME" viram "firstname".
pub fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Quebra o texto em registros crus com o separador dado.
fn records(text: &str, sep: char) -> Vec<Vec<String>> {
    let mut out: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' => {
                if field.trim().is_empty() {
                    field.clear();
                }
                quoted = true;
            }
            _ if c == sep => row.push(std::mem::take(&mut field)),
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                row.push(std::mem::take(&mut field));
                out.push(std::mem::take(&mut row));
            }
            '\n' => {
                row.push(std::mem::take(&mut field));
                out.push(std::mem::take(&mut row));
            }
            _ => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        out.push(row);
    }
    out
}

fn empty_record(r: &[String]) -> bool {
    r.iter().all(|f| f.trim().is_empty())
}

/// Indice do registro que e o cabecalho de verdade.
fn header_index(recs: &[Vec<String>]) -> Option<usize> {
    let first = recs.iter().position(|r| !empty_record(r))?;
    let head = norm(recs[first].first().map(String::as_str).unwrap_or(""));
    if !head.starts_with("notes") {
        return Some(first);
    }
    // Preambulo "Notes:": segue ate a linha em branco que separa do cabecalho.
    let mut i = first + 1;
    while i < recs.len() && !empty_record(&recs[i]) {
        i += 1;
    }
    while i < recs.len() && empty_record(&recs[i]) {
        i += 1;
    }
    if i < recs.len() {
        return Some(i);
    }
    // Sem linha em branco: cai no primeiro registro com mais de uma coluna.
    recs.iter()
        .enumerate()
        .skip(first + 1)
        .find(|(_, r)| r.len() > 1)
        .map(|(i, _)| i)
}

/// Escolhe o separador testando os candidatos num pedaco do inicio do texto e
/// ficando com o que produz o cabecalho mais largo.
fn detect_sep(text: &str) -> char {
    let mut cut = text.len().min(64 * 1024);
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    let head = &text[..cut];
    let mut best = (',', 0usize);
    for sep in [',', ';', '\t'] {
        let recs = records(head, sep);
        let cols = header_index(&recs).map(|i| recs[i].len()).unwrap_or(0);
        if cols > best.1 {
            best = (sep, cols);
        }
    }
    best.0
}

/// Uma planilha ja com cabecalho separado das linhas.
#[derive(Debug, Clone, Default)]
pub struct Table {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    index: HashMap<String, usize>,
}

impl Table {
    pub fn parse(text: &str) -> Self {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let sep = detect_sep(text);
        let recs = records(text, sep);
        let Some(h) = header_index(&recs) else {
            return Self::default();
        };
        let headers: Vec<String> = recs[h].iter().map(|s| s.trim().to_string()).collect();
        let mut index = HashMap::new();
        for (i, name) in headers.iter().enumerate() {
            index.entry(norm(name)).or_insert(i);
        }
        let rows = recs
            .into_iter()
            .skip(h + 1)
            .filter(|r| !empty_record(r))
            .collect();
        Self {
            headers,
            rows,
            index,
        }
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Indice da primeira coluna que casa com algum dos nomes dados.
    pub fn col(&self, names: &[&str]) -> Option<usize> {
        names.iter().find_map(|n| self.index.get(&norm(n)).copied())
    }

    /// Valor de uma coluna numa linha, ja aparado. Coluna ausente vira "".
    pub fn get<'a>(&self, row: &'a [String], names: &[&str]) -> &'a str {
        self.col(names)
            .and_then(|i| row.get(i))
            .map(|s| s.trim())
            .unwrap_or("")
    }
}

/// Escapa um campo para escrever CSV.
pub fn esc(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Monta uma linha de CSV com quebra `\n`.
pub fn line(fields: &[&str]) -> String {
    let mut s = fields.iter().map(|f| esc(f)).collect::<Vec<_>>().join(",");
    s.push('\n');
    s
}

/// Data solta do export, reduzida a ano/mes/dia.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Ymd {
    pub y: i32,
    pub m: u32,
    pub d: u32,
}

impl Ymd {
    pub fn ym(&self) -> String {
        format!("{:04}-{:02}", self.y, self.m)
    }
    pub fn iso(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.y, self.m, self.d)
    }
}

fn month(name: &str) -> Option<u32> {
    let n: String = norm(name).chars().take(3).collect();
    let i = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ]
    .iter()
    .position(|m| *m == n)?;
    Some(i as u32 + 1)
}

/// Aceita os formatos que aparecem no export: `2021-08-06 14:22:31 UTC`,
/// `06 Aug 2021`, `Aug 6, 2021`, `Jan 2019`, `8/6/21` e `2021`.
pub fn parse_date(raw: &str) -> Option<Ymd> {
    let s = raw.trim().trim_matches('"').trim();
    if s.is_empty() {
        return None;
    }
    let head = s.split_whitespace().next().unwrap_or(s);
    // ISO: 2021-08-06 ou 2021-08
    if head.len() >= 7 && head.as_bytes().get(4) == Some(&b'-') {
        let mut p = head.split('-');
        let y = p.next()?.parse::<i32>().ok()?;
        let m = p.next()?.parse::<u32>().ok()?;
        let d = p.next().and_then(|v| v.parse::<u32>().ok()).unwrap_or(1);
        if (1..=12).contains(&m) {
            return Some(Ymd { y, m, d: d.max(1) });
        }
        return None;
    }
    // 8/6/21 ou 08/06/2021 (mes primeiro, padrao do LinkedIn em ingles)
    if head.contains('/') {
        let p: Vec<&str> = head.split('/').collect();
        if p.len() == 3 {
            let m = p[0].parse::<u32>().ok()?;
            let d = p[1].parse::<u32>().ok()?;
            let mut y = p[2].parse::<i32>().ok()?;
            if y < 100 {
                y += 2000;
            }
            if (1..=12).contains(&m) {
                return Some(Ymd { y, m, d });
            }
        }
        return None;
    }
    // Por palavras: "06 Aug 2021", "Aug 6, 2021", "Jan 2019", "2021"
    let parts: Vec<String> = s
        .split([' ', ','])
        .filter(|p| !p.trim().is_empty())
        .map(|p| p.trim().to_string())
        .collect();
    let mut y = None;
    let mut m = None;
    let mut d = None;
    for p in &parts {
        if let Some(mm) = month(p) {
            m.get_or_insert(mm);
        } else if let Ok(n) = p.parse::<i32>() {
            if n >= 1000 {
                y.get_or_insert(n);
            } else if (1..=31).contains(&n) {
                d.get_or_insert(n as u32);
            }
        }
    }
    let y = y?;
    Some(Ymd {
        y,
        m: m.unwrap_or(1),
        d: d.unwrap_or(1),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_cabecalho_simples() {
        let t = Table::parse("First Name,Last Name\nAna,Silva\nBruno,Souza\n");
        assert_eq!(t.headers.len(), 2);
        assert_eq!(t.len(), 2);
        assert_eq!(t.get(&t.rows[1], &["first name"]), "Bruno");
    }

    #[test]
    fn aceita_bom_e_preambulo_de_notas() {
        let raw = "\u{feff}Notes:\n\"Se voce baixou o arquivo, guarde com cuidado.\"\n\nFirst Name,Company\nAna,Acme\n";
        let t = Table::parse(raw);
        assert_eq!(t.headers, vec!["First Name", "Company"]);
        assert_eq!(t.len(), 1);
        assert_eq!(t.get(&t.rows[0], &["company"]), "Acme");
    }

    #[test]
    fn aceita_ponto_e_virgula() {
        let t = Table::parse("Nome;Empresa;Cargo\nAna;Acme;Dev\n");
        assert_eq!(t.headers.len(), 3);
        assert_eq!(t.get(&t.rows[0], &["cargo"]), "Dev");
    }

    #[test]
    fn campo_com_virgula_aspas_e_quebra_de_linha() {
        let raw = "A,B\n\"um, dois\",\"linha1\nlinha2\"\n\"ele disse \"\"oi\"\"\",x\n";
        let t = Table::parse(raw);
        assert_eq!(t.len(), 2);
        assert_eq!(t.get(&t.rows[0], &["a"]), "um, dois");
        assert_eq!(t.get(&t.rows[0], &["b"]), "linha1\nlinha2");
        assert_eq!(t.get(&t.rows[1], &["a"]), "ele disse \"oi\"");
    }

    #[test]
    fn coluna_ausente_vira_vazio() {
        let t = Table::parse("A,B\n1,2\n");
        assert_eq!(t.get(&t.rows[0], &["email address"]), "");
        assert!(t.col(&["email address"]).is_none());
    }

    #[test]
    fn coluna_por_nome_alternativo_e_caixa() {
        let t = Table::parse("CONVERSATION ID,FROM\nc1,Ana\n");
        assert_eq!(
            t.get(&t.rows[0], &["conversationid", "conversation id"]),
            "c1"
        );
    }

    #[test]
    fn linha_curta_nao_estoura() {
        let t = Table::parse("A,B,C\n1\n1,2,3\n");
        assert_eq!(t.len(), 2);
        assert_eq!(t.get(&t.rows[0], &["c"]), "");
    }

    #[test]
    fn datas_em_varios_formatos() {
        assert_eq!(
            parse_date("2021-08-06 14:22:31 UTC"),
            Some(Ymd {
                y: 2021,
                m: 8,
                d: 6
            })
        );
        assert_eq!(
            parse_date("06 Aug 2021"),
            Some(Ymd {
                y: 2021,
                m: 8,
                d: 6
            })
        );
        assert_eq!(
            parse_date("Aug 6, 2021"),
            Some(Ymd {
                y: 2021,
                m: 8,
                d: 6
            })
        );
        assert_eq!(
            parse_date("Jan 2019"),
            Some(Ymd {
                y: 2019,
                m: 1,
                d: 1
            })
        );
        assert_eq!(
            parse_date("8/6/21"),
            Some(Ymd {
                y: 2021,
                m: 8,
                d: 6
            })
        );
        assert_eq!(parse_date(""), None);
        assert_eq!(parse_date("sem data"), None);
        assert_eq!(
            parse_date("2021-08").map(|d| d.ym()),
            Some("2021-08".to_string())
        );
    }

    #[test]
    fn escapa_campo_de_csv() {
        assert_eq!(esc("simples"), "simples");
        assert_eq!(esc("um, dois"), "\"um, dois\"");
        assert_eq!(esc("aspas \" aqui"), "\"aspas \"\" aqui\"");
        assert_eq!(line(&["a", "b, c"]), "a,\"b, c\"\n");
    }
}
