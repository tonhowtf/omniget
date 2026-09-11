//! Tabela filtravel do `Connections.csv`: normaliza, busca, filtra por
//! periodo, agrupa por empresa e cargo, acha duplicados e marca quem veio sem
//! email. Exporta o resultado filtrado em CSV ou JSON.

use std::collections::HashMap;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::csv::{self, parse_date, Table};
use super::source::Source;
use super::{bump, series, top, Bucket, Count};
use crate::core::tools::{report, ProgressFn};

const ID: &str = "li-connections";

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Options {
    /// Zip do export ou pasta ja extraida.
    pub path: String,
    /// Busca livre em nome, empresa e cargo.
    pub query: Option<String>,
    pub company: Option<String>,
    pub position: Option<String>,
    /// Recorte por data de conexao, em `YYYY-MM-DD` ou `YYYY-MM`.
    pub from: Option<String>,
    pub to: Option<String>,
    pub only_without_email: Option<bool>,
    pub only_duplicates: Option<bool>,
    /// "date" (padrao), "name" ou "company".
    pub sort: Option<String>,
    pub limit: Option<usize>,
    /// "csv" ou "json" para gravar o filtrado.
    pub export: Option<String>,
    pub out_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Connection {
    pub name: String,
    pub first_name: String,
    pub last_name: String,
    pub url: String,
    pub email: String,
    pub company: String,
    pub position: String,
    /// Data como veio no CSV.
    pub connected_on: String,
    /// Mesma data em `YYYY-MM-DD`, vazio quando nao deu para ler.
    pub date: String,
    pub month: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DupGroup {
    pub reason: String,
    pub items: Vec<Connection>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConnectionsResult {
    pub path: String,
    pub total: usize,
    pub matched: usize,
    pub returned: usize,
    pub without_email: usize,
    pub first: String,
    pub last: String,
    pub items: Vec<Connection>,
    pub companies: Vec<Count>,
    pub positions: Vec<Count>,
    pub by_month: Vec<Bucket>,
    pub by_year: Vec<Bucket>,
    pub duplicates: Vec<DupGroup>,
    pub exports: Vec<String>,
}

/// Slug do perfil (`/in/<slug>`), que e o identificador estavel da conexao.
fn slug(url: &str) -> String {
    let u = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .trim_end_matches('/');
    match u.rfind("/in/") {
        Some(i) => csv::norm(&u[i + 4..]),
        None => String::new(),
    }
}

/// Le o `Connections.csv` ja normalizado.
pub fn load(src: &Source) -> Vec<Connection> {
    let t: Table = src.table_or_empty(&["Connections.csv"]);
    let mut out = Vec::with_capacity(t.len());
    for row in &t.rows {
        let first = t.get(row, &["First Name", "FirstName"]).to_string();
        let last = t.get(row, &["Last Name", "LastName"]).to_string();
        let name = format!("{} {}", first, last).trim().to_string();
        let raw = t
            .get(row, &["Connected On", "ConnectedOn", "Date"])
            .to_string();
        let parsed = parse_date(&raw);
        if name.is_empty() && raw.is_empty() {
            continue;
        }
        out.push(Connection {
            name,
            first_name: first,
            last_name: last,
            url: t
                .get(row, &["URL", "Profile URL", "ProfileUrl"])
                .to_string(),
            email: t
                .get(row, &["Email Address", "EmailAddress", "Email"])
                .to_string(),
            company: t.get(row, &["Company", "Company Name"]).to_string(),
            position: t.get(row, &["Position", "Title"]).to_string(),
            connected_on: raw,
            date: parsed.map(|d| d.iso()).unwrap_or_default(),
            month: parsed.map(|d| d.ym()).unwrap_or_default(),
        });
    }
    out
}

/// Conjuntos disjuntos simples para juntar duplicados que casam por chaves
/// diferentes (mesmo perfil, mesmo nome+empresa, mesmo email).
struct Dsu(Vec<usize>);

impl Dsu {
    fn new(n: usize) -> Self {
        Self((0..n).collect())
    }
    fn find(&mut self, a: usize) -> usize {
        let mut r = a;
        while self.0[r] != r {
            r = self.0[r];
        }
        let mut c = a;
        while self.0[c] != c {
            let next = self.0[c];
            self.0[c] = r;
            c = next;
        }
        r
    }
    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.0[ra] = rb;
        }
    }
}

/// Agrupa duplicados. A razao do grupo diz por que eles casaram.
pub fn duplicates(items: &[Connection]) -> Vec<DupGroup> {
    let mut dsu = Dsu::new(items.len());
    let mut reasons: HashMap<usize, &str> = HashMap::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for (i, c) in items.iter().enumerate() {
        let mut keys: Vec<(String, &str)> = Vec::new();
        let s = slug(&c.url);
        if !s.is_empty() {
            keys.push((format!("u:{}", s), "profile"));
        }
        if !c.email.is_empty() {
            keys.push((format!("e:{}", c.email.to_lowercase()), "email"));
        }
        if !c.name.is_empty() {
            keys.push((
                format!("n:{}|{}", csv::norm(&c.name), csv::norm(&c.company)),
                "name",
            ));
        }
        for (k, why) in keys {
            match seen.get(&k) {
                Some(&j) => {
                    dsu.union(i, j);
                    reasons.entry(i).or_insert(why);
                    reasons.entry(j).or_insert(why);
                }
                None => {
                    seen.insert(k, i);
                }
            }
        }
    }
    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..items.len() {
        let r = dsu.find(i);
        groups.entry(r).or_default().push(i);
    }
    let mut out: Vec<DupGroup> = groups
        .into_values()
        .filter(|g| g.len() > 1)
        .map(|g| DupGroup {
            reason: g
                .iter()
                .find_map(|i| reasons.get(i).copied())
                .unwrap_or("name")
                .to_string(),
            items: g.iter().filter_map(|i| items.get(*i).cloned()).collect(),
        })
        .collect();
    out.sort_by(|a, b| {
        b.items
            .len()
            .cmp(&a.items.len())
            .then_with(|| a.items[0].name.cmp(&b.items[0].name))
    });
    out
}

fn in_range(date: &str, from: Option<&str>, to: Option<&str>) -> bool {
    match (from, to) {
        (None, None) => true,
        _ => {
            if date.is_empty() {
                return false;
            }
            if let Some(f) = from.filter(|s| !s.trim().is_empty()) {
                if date < f.trim() {
                    return false;
                }
            }
            if let Some(t) = to.filter(|s| !s.trim().is_empty()) {
                // `to` em `YYYY-MM` pega o mes inteiro.
                let t = t.trim();
                let cut = if t.len() <= 7 {
                    format!("{}-31", t)
                } else {
                    t.to_string()
                };
                if date > cut.as_str() {
                    return false;
                }
            }
            true
        }
    }
}

fn matches(c: &Connection, opts: &Options, dup: bool) -> bool {
    if let Some(q) = opts.query.as_deref().filter(|q| !q.trim().is_empty()) {
        let q = q.trim().to_lowercase();
        let hay = format!("{} {} {} {}", c.name, c.company, c.position, c.email).to_lowercase();
        if !hay.contains(&q) {
            return false;
        }
    }
    if let Some(co) = opts.company.as_deref().filter(|s| !s.trim().is_empty()) {
        if csv::norm(&c.company) != csv::norm(co) {
            return false;
        }
    }
    if let Some(po) = opts.position.as_deref().filter(|s| !s.trim().is_empty()) {
        if !c
            .position
            .to_lowercase()
            .contains(&po.trim().to_lowercase())
        {
            return false;
        }
    }
    if !in_range(&c.date, opts.from.as_deref(), opts.to.as_deref()) {
        return false;
    }
    if opts.only_without_email.unwrap_or(false) && !c.email.is_empty() {
        return false;
    }
    if opts.only_duplicates.unwrap_or(false) && !dup {
        return false;
    }
    true
}

fn to_csv(items: &[Connection]) -> String {
    let mut s = csv::line(&[
        "First Name",
        "Last Name",
        "URL",
        "Email Address",
        "Company",
        "Position",
        "Connected On",
    ]);
    for c in items {
        s.push_str(&csv::line(&[
            &c.first_name,
            &c.last_name,
            &c.url,
            &c.email,
            &c.company,
            &c.position,
            &c.connected_on,
        ]));
    }
    s
}

pub fn run(opts: &Options, p: &ProgressFn) -> Result<ConnectionsResult> {
    report(p, ID, "started", 0, Some(3), None);
    let opened = Source::open(&opts.path)?;
    let all = load(&opened.source);
    report(
        p,
        ID,
        "progress",
        1,
        Some(3),
        Some(format!("{}", all.len())),
    );

    let dups = duplicates(&all);
    let dup_keys: std::collections::HashSet<String> = dups
        .iter()
        .flat_map(|g| {
            g.items
                .iter()
                .map(|c| format!("{}|{}", c.name, c.connected_on))
        })
        .collect();

    let mut items: Vec<Connection> = all
        .iter()
        .filter(|c| {
            let d = dup_keys.contains(&format!("{}|{}", c.name, c.connected_on));
            matches(c, opts, d)
        })
        .cloned()
        .collect();

    match opts.sort.as_deref().unwrap_or("date") {
        "name" => items.sort_by_key(|c| c.name.to_lowercase()),
        "company" => items.sort_by(|a, b| {
            a.company
                .to_lowercase()
                .cmp(&b.company.to_lowercase())
                .then_with(|| a.name.cmp(&b.name))
        }),
        _ => items.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| a.name.cmp(&b.name))),
    }
    report(p, ID, "progress", 2, Some(3), None);

    let mut companies = HashMap::new();
    let mut positions = HashMap::new();
    let mut by_month = HashMap::new();
    let mut by_year = HashMap::new();
    let mut without_email = 0usize;
    for c in &items {
        bump(&mut companies, c.company.trim());
        bump(&mut positions, c.position.trim());
        bump(&mut by_month, &c.month);
        if c.date.len() >= 4 {
            bump(&mut by_year, &c.date[..4]);
        }
        if c.email.trim().is_empty() {
            without_email += 1;
        }
    }
    let mut dates: Vec<&str> = items
        .iter()
        .map(|c| c.date.as_str())
        .filter(|d| !d.is_empty())
        .collect();
    dates.sort_unstable();

    let matched = items.len();
    let limit = opts.limit.unwrap_or(500).max(1);
    let mut exports = Vec::new();
    if let Some(kind) = opts.export.as_deref().filter(|k| !k.trim().is_empty()) {
        let dir = super::out_dir(opts.out_dir.as_deref());
        let body = if kind.eq_ignore_ascii_case("json") {
            serde_json::to_string_pretty(&items)?
        } else {
            to_csv(&items)
        };
        let name = if kind.eq_ignore_ascii_case("json") {
            "connections-filtradas.json"
        } else {
            "connections-filtradas.csv"
        };
        exports.push(super::write_out(&dir, name, &body)?);
    }
    let out = ConnectionsResult {
        path: opts.path.clone(),
        total: all.len(),
        matched,
        returned: matched.min(limit),
        without_email,
        first: dates.first().unwrap_or(&"").to_string(),
        last: dates.last().unwrap_or(&"").to_string(),
        items: items.into_iter().take(limit).collect(),
        companies: top(&companies, 40),
        positions: top(&positions, 40),
        by_month: series(&by_month),
        by_year: series(&by_year),
        duplicates: dups,
        exports,
    };
    report(p, ID, "done", 3, Some(3), None);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::super::source::fixture;
    use super::*;
    use crate::core::tools::noop_progress;

    fn opts(dir: &std::path::Path) -> Options {
        Options {
            path: dir.to_string_lossy().to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn le_as_cinco_conexoes_do_export() {
        let dir = fixture::dir();
        let r = run(&opts(&dir), &noop_progress()).expect("roda");
        assert_eq!(r.total, 5);
        assert_eq!(r.matched, 5);
        assert_eq!(r.without_email, 3);
        assert_eq!(r.first, "2021-08-06");
        assert_eq!(r.last, "2022-03-15");
        assert_eq!(r.companies[0].name, "Acme");
        assert_eq!(r.companies[0].count, 3);
    }

    #[test]
    fn acha_o_duplicado_por_perfil() {
        let dir = fixture::dir();
        let r = run(&opts(&dir), &noop_progress()).expect("roda");
        assert_eq!(r.duplicates.len(), 1);
        assert_eq!(r.duplicates[0].items.len(), 2);
        assert_eq!(r.duplicates[0].items[0].name, "Ana Silva");
    }

    #[test]
    fn busca_e_periodo_filtram() {
        let dir = fixture::dir();
        let mut o = opts(&dir);
        o.query = Some("designer".into());
        let r = run(&o, &noop_progress()).expect("roda");
        assert_eq!(r.matched, 2);

        let mut o = opts(&dir);
        o.from = Some("2022-01-01".into());
        let r = run(&o, &noop_progress()).expect("roda");
        assert_eq!(r.matched, 2);

        let mut o = opts(&dir);
        o.to = Some("2021-12".into());
        let r = run(&o, &noop_progress()).expect("roda");
        assert_eq!(r.matched, 3);

        let mut o = opts(&dir);
        o.only_without_email = Some(true);
        let r = run(&o, &noop_progress()).expect("roda");
        assert_eq!(r.matched, 3);
    }

    #[test]
    fn cargo_com_virgula_sobrevive() {
        let dir = fixture::dir();
        let r = run(&opts(&dir), &noop_progress()).expect("roda");
        assert!(r
            .items
            .iter()
            .any(|c| c.position == "Engenheira de Software, Senior"));
    }

    #[test]
    fn exporta_csv_filtrado() {
        let dir = fixture::dir();
        let out = dir.join("saida");
        let mut o = opts(&dir);
        o.query = Some("Beta".into());
        o.export = Some("csv".into());
        o.out_dir = Some(out.to_string_lossy().to_string());
        let r = run(&o, &noop_progress()).expect("roda");
        assert_eq!(r.exports.len(), 1);
        let body = std::fs::read_to_string(&r.exports[0]).expect("le");
        assert_eq!(body.lines().count(), 3);
        assert!(body.contains("carla@beta.io"));
    }

    #[test]
    fn export_sem_connections_nao_estoura() {
        let tmp = std::env::temp_dir().join(format!("omniget-li-vazio-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let _ = std::fs::write(tmp.join("Skills.csv"), "Name\nRust\n");
        let r = run(&opts(&tmp), &noop_progress()).expect("roda");
        assert_eq!(r.total, 0);
        assert!(r.duplicates.is_empty());
    }

    #[test]
    fn slug_ignora_barra_e_query() {
        assert_eq!(
            slug("https://www.linkedin.com/in/ana-silva-123/?x=1"),
            "anasilva123"
        );
        assert_eq!(slug("sem url"), "");
    }
}
