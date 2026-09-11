//! Panorama do export inteiro: inventario dos arquivos, crescimento das
//! conexoes, volume de mensagens, posts, reacoes, comentarios, convites e o
//! que o LinkedIn guarda sobre voce em `Ad_Targeting.csv` — que costuma ser a
//! parte que mais surpreende. Exporta JSON e Markdown.

use std::collections::HashMap;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::csv::parse_date;
use super::source::Source;
use super::{bump, series, top, Bucket, Count};
use crate::core::tools::{report, ProgressFn};

const ID: &str = "li-overview";

/// Arquivos que a gente sabe ler; o que faltar vira aviso.
const KNOWN: &[&str] = &[
    "Connections.csv",
    "messages.csv",
    "Shares.csv",
    "Comments.csv",
    "Reactions.csv",
    "Invitations.csv",
    "Profile.csv",
    "Positions.csv",
    "Education.csv",
    "Skills.csv",
    "Ad_Targeting.csv",
    "Company Follows.csv",
];

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Options {
    pub path: String,
    pub out_dir: Option<String>,
    pub export_json: Option<bool>,
    pub export_markdown: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileInfo {
    pub name: String,
    pub rows: usize,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ConnStats {
    pub total: usize,
    pub with_email: usize,
    pub without_email: usize,
    pub first: String,
    pub last: String,
    pub by_month: Vec<Bucket>,
    pub by_year: Vec<Bucket>,
    pub top_companies: Vec<Count>,
    pub top_positions: Vec<Count>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct MsgStats {
    pub total: usize,
    pub conversations: usize,
    pub sent: usize,
    pub received: usize,
    pub me: String,
    pub by_year: Vec<Bucket>,
    pub top_contacts: Vec<Count>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Post {
    pub date: String,
    pub link: String,
    pub text: String,
    pub media: bool,
    pub visibility: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct PostStats {
    pub total: usize,
    pub with_media: usize,
    pub first: String,
    pub last: String,
    pub by_month: Vec<Bucket>,
    pub by_year: Vec<Bucket>,
    pub visibility: Vec<Count>,
    pub recent: Vec<Post>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Simple {
    pub total: usize,
    pub by_year: Vec<Bucket>,
    pub kinds: Vec<Count>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Invites {
    pub sent: usize,
    pub received: usize,
    pub by_year: Vec<Bucket>,
}

/// Um criterio que anunciantes usaram para te alcancar.
#[derive(Debug, Clone, Serialize)]
pub struct AdItem {
    pub field: String,
    pub values: Vec<String>,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Overview {
    pub path: String,
    pub files: Vec<FileInfo>,
    pub entries: usize,
    pub missing: Vec<String>,
    pub connections: ConnStats,
    pub messages: MsgStats,
    pub posts: PostStats,
    pub reactions: Simple,
    pub comments: Simple,
    pub invitations: Invites,
    pub follows: Simple,
    pub ad_targeting: Vec<AdItem>,
    pub ad_targeting_values: usize,
    pub exports: Vec<String>,
}

fn year_of(raw: &str) -> Option<String> {
    parse_date(raw).map(|d| d.y.to_string())
}

/// Le uma tabela simples "tem data e talvez um tipo" (reacoes, comentarios,
/// empresas seguidas).
fn simple(src: &Source, names: &[&str], date_cols: &[&str], kind_cols: &[&str]) -> Simple {
    let t = src.table_or_empty(names);
    let mut by_year = HashMap::new();
    let mut kinds = HashMap::new();
    for row in &t.rows {
        if let Some(y) = year_of(t.get(row, date_cols)) {
            bump(&mut by_year, &y);
        }
        if !kind_cols.is_empty() {
            bump(&mut kinds, t.get(row, kind_cols));
        }
    }
    Simple {
        total: t.len(),
        by_year: series(&by_year),
        kinds: top(&kinds, 20),
    }
}

/// `Ad_Targeting.csv` vem de dois jeitos: uma coluna por criterio com os
/// valores juntos, ou duas colunas (criterio, valor).
fn ad_targeting(src: &Source) -> Vec<AdItem> {
    let t = src.table_or_empty(&["Ad_Targeting.csv", "Ads Clicked.csv"]);
    if t.headers.is_empty() {
        return Vec::new();
    }
    let split = |v: &str| -> Vec<String> {
        v.split(['|', '\n', ';'])
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    };
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    let two_cols = t.headers.len() == 2 && t.col(&["Value", "Values"]).is_some();
    if two_cols {
        for row in &t.rows {
            let field = t.get(row, &[t.headers[0].as_str()]).to_string();
            let value = t.get(row, &["Value", "Values"]).to_string();
            if field.is_empty() {
                continue;
            }
            let e = map.entry(field.clone()).or_insert_with(|| {
                order.push(field.clone());
                Vec::new()
            });
            e.extend(split(&value));
        }
    } else {
        for (i, head) in t.headers.iter().enumerate() {
            if head.trim().is_empty() {
                continue;
            }
            let mut values = Vec::new();
            for row in &t.rows {
                if let Some(v) = row.get(i) {
                    values.extend(split(v));
                }
            }
            if values.is_empty() {
                continue;
            }
            order.push(head.clone());
            map.insert(head.clone(), values);
        }
    }
    let mut out: Vec<AdItem> = order
        .into_iter()
        .filter_map(|field| {
            let mut values = map.remove(&field)?;
            values.sort();
            values.dedup();
            let count = values.len();
            values.truncate(60);
            Some(AdItem {
                field,
                values,
                count,
            })
        })
        .collect();
    out.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.field.cmp(&b.field)));
    out
}

fn posts(src: &Source) -> PostStats {
    let t = src.table_or_empty(&["Shares.csv"]);
    let mut by_month = HashMap::new();
    let mut by_year = HashMap::new();
    let mut vis = HashMap::new();
    let mut with_media = 0usize;
    let mut all: Vec<Post> = Vec::with_capacity(t.len());
    for row in &t.rows {
        let raw = t.get(row, &["Date", "ShareDate"]);
        let d = parse_date(raw);
        if let Some(d) = d {
            bump(&mut by_month, &d.ym());
            bump(&mut by_year, &d.y.to_string());
        }
        let media = !t
            .get(row, &["MediaUrl", "Media Url", "SharedUrl"])
            .is_empty();
        if media {
            with_media += 1;
        }
        bump(&mut vis, t.get(row, &["Visibility"]));
        all.push(Post {
            date: d.map(|d| d.iso()).unwrap_or_else(|| raw.to_string()),
            link: t.get(row, &["ShareLink", "Share Link"]).to_string(),
            text: t
                .get(row, &["ShareCommentary", "Share Commentary"])
                .chars()
                .take(240)
                .collect(),
            media,
            visibility: t.get(row, &["Visibility"]).to_string(),
        });
    }
    all.sort_by(|a, b| b.date.cmp(&a.date));
    let first = all.last().map(|p| p.date.clone()).unwrap_or_default();
    let last = all.first().map(|p| p.date.clone()).unwrap_or_default();
    PostStats {
        total: all.len(),
        with_media,
        first,
        last,
        by_month: series(&by_month),
        by_year: series(&by_year),
        visibility: top(&vis, 10),
        recent: all.into_iter().take(20).collect(),
    }
}

fn invitations(src: &Source) -> Invites {
    let t = src.table_or_empty(&["Invitations.csv"]);
    let mut by_year = HashMap::new();
    let mut sent = 0usize;
    let mut received = 0usize;
    for row in &t.rows {
        if let Some(y) = year_of(t.get(row, &["Sent At", "SentAt", "Date"])) {
            bump(&mut by_year, &y);
        }
        let dir = t.get(row, &["Direction"]).to_ascii_uppercase();
        if dir.starts_with("OUT") {
            sent += 1;
        } else if dir.starts_with("IN") {
            received += 1;
        }
    }
    Invites {
        sent,
        received,
        by_year: series(&by_year),
    }
}

fn markdown(o: &Overview) -> String {
    let mut s = String::from("# LinkedIn\n\n");
    s.push_str(&format!("`{}`\n\n", o.path));
    s.push_str(&format!(
        "- Conexoes: **{}** ({} sem email), de {} a {}\n",
        o.connections.total, o.connections.without_email, o.connections.first, o.connections.last
    ));
    s.push_str(&format!(
        "- Mensagens: **{}** em {} conversas ({} enviadas)\n",
        o.messages.total, o.messages.conversations, o.messages.sent
    ));
    s.push_str(&format!(
        "- Posts: **{}** · reacoes: {} · comentarios: {}\n",
        o.posts.total, o.reactions.total, o.comments.total
    ));
    s.push_str(&format!(
        "- Convites: {} enviados, {} recebidos · empresas seguidas: {}\n\n",
        o.invitations.sent, o.invitations.received, o.follows.total
    ));

    s.push_str("## Empresas\n\n");
    for c in o.connections.top_companies.iter().take(15) {
        s.push_str(&format!("- {} — {}\n", c.name, c.count));
    }
    s.push_str("\n## Cargos\n\n");
    for c in o.connections.top_positions.iter().take(15) {
        s.push_str(&format!("- {} — {}\n", c.name, c.count));
    }
    s.push_str("\n## Conexoes por ano\n\n");
    for b in &o.connections.by_year {
        s.push_str(&format!("- {} — {}\n", b.period, b.count));
    }
    s.push_str("\n## Mensagens por ano\n\n");
    for b in &o.messages.by_year {
        s.push_str(&format!("- {} — {}\n", b.period, b.count));
    }
    if !o.ad_targeting.is_empty() {
        s.push_str(&format!(
            "\n## O que anunciantes usam sobre voce ({} valores)\n\n",
            o.ad_targeting_values
        ));
        for a in &o.ad_targeting {
            s.push_str(&format!(
                "- **{}** ({}): {}\n",
                a.field,
                a.count,
                a.values
                    .iter()
                    .take(12)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    if !o.missing.is_empty() {
        s.push_str(&format!(
            "\n## Nao veio no export\n\n- {}\n",
            o.missing.join("\n- ")
        ));
    }
    s
}

pub fn run(opts: &Options, p: &ProgressFn) -> Result<Overview> {
    report(p, ID, "started", 0, Some(6), None);
    let opened = Source::open(&opts.path)?;
    let src = &opened.source;

    let conns = super::connections::load(src);
    let mut cs = ConnStats {
        total: conns.len(),
        ..Default::default()
    };
    let mut companies = HashMap::new();
    let mut cpositions = HashMap::new();
    let mut cmonth = HashMap::new();
    let mut cyear = HashMap::new();
    let mut dates: Vec<&str> = Vec::new();
    for c in &conns {
        if c.email.trim().is_empty() {
            cs.without_email += 1;
        } else {
            cs.with_email += 1;
        }
        bump(&mut companies, c.company.trim());
        bump(&mut cpositions, c.position.trim());
        bump(&mut cmonth, &c.month);
        if c.date.len() >= 4 {
            bump(&mut cyear, &c.date[..4]);
            dates.push(&c.date);
        }
    }
    dates.sort_unstable();
    cs.first = dates.first().unwrap_or(&"").to_string();
    cs.last = dates.last().unwrap_or(&"").to_string();
    cs.by_month = series(&cmonth);
    cs.by_year = series(&cyear);
    cs.top_companies = top(&companies, 30);
    cs.top_positions = top(&cpositions, 30);
    report(p, ID, "progress", 1, Some(6), None);

    let (me, _, convs) = super::messages::load(src, None);
    let mut ms = MsgStats {
        conversations: convs.len(),
        me: me.clone(),
        ..Default::default()
    };
    let mut contacts = HashMap::new();
    let mut myear = HashMap::new();
    for c in &convs {
        ms.total += c.count;
        ms.sent += c.sent;
        for name in &c.participants {
            if !name.eq_ignore_ascii_case(me.trim()) {
                *contacts.entry(name.clone()).or_insert(0) += c.count;
            }
        }
        for m in &c.messages {
            if m.date.len() >= 4 {
                bump(&mut myear, &m.date[..4]);
            }
        }
    }
    ms.received = ms.total - ms.sent;
    ms.by_year = series(&myear);
    ms.top_contacts = top(&contacts, 25);
    report(p, ID, "progress", 2, Some(6), None);

    let ps = posts(src);
    report(p, ID, "progress", 3, Some(6), None);
    let reactions = simple(src, &["Reactions.csv"], &["Date"], &["Type"]);
    let comments = simple(src, &["Comments.csv"], &["Date"], &[]);
    let follows = simple(src, &["Company Follows.csv"], &["Followed On", "Date"], &[]);
    let invites = invitations(src);
    report(p, ID, "progress", 4, Some(6), None);

    let ads = ad_targeting(src);
    let ads_total = ads.iter().map(|a| a.count).sum();

    let mut files: Vec<FileInfo> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    for name in KNOWN {
        match src.table(&[name]) {
            Some(t) => files.push(FileInfo {
                name: (*name).to_string(),
                rows: t.len(),
            }),
            None => missing.push((*name).to_string()),
        }
    }
    report(p, ID, "progress", 5, Some(6), None);

    let mut out = Overview {
        path: opts.path.clone(),
        files,
        entries: opened.entries.len(),
        missing,
        connections: cs,
        messages: ms,
        posts: ps,
        reactions,
        comments,
        invitations: invites,
        follows,
        ad_targeting: ads,
        ad_targeting_values: ads_total,
        exports: Vec::new(),
    };

    if opts.export_json.unwrap_or(false) || opts.export_markdown.unwrap_or(false) {
        let dir = super::out_dir(opts.out_dir.as_deref());
        if opts.export_json.unwrap_or(false) {
            let body = serde_json::to_string_pretty(&out)?;
            out.exports
                .push(super::write_out(&dir, "linkedin-panorama.json", &body)?);
        }
        if opts.export_markdown.unwrap_or(false) {
            let body = markdown(&out);
            out.exports
                .push(super::write_out(&dir, "linkedin-panorama.md", &body)?);
        }
    }
    report(p, ID, "done", 6, Some(6), None);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::super::source::fixture;
    use super::*;
    use crate::core::tools::noop_progress;

    fn run_path(path: &str) -> Overview {
        run(
            &Options {
                path: path.to_string(),
                ..Default::default()
            },
            &noop_progress(),
        )
        .expect("roda")
    }

    #[test]
    fn faz_o_inventario_e_avisa_o_que_faltou() {
        let dir = fixture::dir();
        let o = run_path(&dir.to_string_lossy());
        assert_eq!(o.files.len(), 11);
        assert_eq!(o.missing, vec!["Comments.csv".to_string()]);
        assert_eq!(o.entries, 13);
    }

    #[test]
    fn conta_conexoes_mensagens_posts_e_convites() {
        let dir = fixture::dir();
        let o = run_path(&dir.to_string_lossy());
        assert_eq!(o.connections.total, 5);
        assert_eq!(o.connections.without_email, 3);
        assert_eq!(o.connections.by_year.len(), 2);
        assert_eq!(o.messages.total, 5);
        assert_eq!(o.messages.conversations, 2);
        assert_eq!(o.messages.me, "Tonho Dev");
        assert_eq!(o.posts.total, 2);
        assert_eq!(o.posts.with_media, 1);
        assert_eq!(o.reactions.total, 3);
        assert_eq!(o.reactions.kinds[0].name, "LIKE");
        assert_eq!(o.comments.total, 0);
        assert_eq!(o.invitations.sent, 1);
        assert_eq!(o.invitations.received, 1);
        assert_eq!(o.follows.total, 1);
    }

    #[test]
    fn destaca_o_ad_targeting_com_preambulo_de_notas() {
        let dir = fixture::dir();
        let o = run_path(&dir.to_string_lossy());
        assert_eq!(o.ad_targeting.len(), 4);
        assert_eq!(o.ad_targeting_values, 8);
        let interests = o
            .ad_targeting
            .iter()
            .find(|a| a.field == "Interests")
            .expect("interests");
        assert_eq!(interests.values, vec!["Linux", "Rust", "Svelte"]);
    }

    #[test]
    fn roda_igual_a_partir_do_zip() {
        let zip = fixture::zip();
        let o = run_path(&zip.to_string_lossy());
        assert_eq!(o.connections.total, 5);
        assert_eq!(o.messages.total, 5);
        assert_eq!(o.ad_targeting.len(), 4);
    }

    #[test]
    fn exporta_json_e_markdown() {
        let dir = fixture::dir();
        let out = dir.join("saida-panorama");
        let o = run(
            &Options {
                path: dir.to_string_lossy().to_string(),
                out_dir: Some(out.to_string_lossy().to_string()),
                export_json: Some(true),
                export_markdown: Some(true),
            },
            &noop_progress(),
        )
        .expect("roda");
        assert_eq!(o.exports.len(), 2);
        let md = std::fs::read_to_string(out.join("linkedin-panorama.md")).expect("md");
        assert!(md.contains("Conexoes: **5**"));
        assert!(md.contains("Acme"));
        assert!(md.contains("anunciantes"));
        let js = std::fs::read_to_string(out.join("linkedin-panorama.json")).expect("json");
        assert!(js.contains("\"ad_targeting\""));
    }
}
