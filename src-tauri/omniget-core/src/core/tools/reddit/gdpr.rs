//! Leitor do export oficial de dados do Reddit (reddit.com/settings/data-request).
//!
//! O Reddit manda um zip com uma pilha de CSV cujos nomes e colunas mudam de
//! safra para safra. Aqui nada é obrigatório: lê o que existir, ignora o que
//! faltar e devolve um resumo utilizável — totais, subs mais ativos, linha do
//! tempo por mês, destaques e a lista de salvos com permalink. Zero rede: o
//! arquivo é do usuário e não sai da máquina.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use crate::core::tools::{report, ProgressFn};

const ID: &str = "rd-gdpr";

// ───────────────────────── CSV à mão ─────────────────────────

/// Leitor de CSV no formato RFC 4180: aspas duplas, vírgula dentro do campo,
/// quebra de linha dentro do campo e `""` para uma aspa literal.
pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    let mut any = false;
    while let Some(c) = chars.next() {
        any = true;
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' if field.is_empty() => quoted = true,
            ',' => row.push(std::mem::take(&mut field)),
            '\r' => {}
            '\n' => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            _ => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    if !any {
        return Vec::new();
    }
    rows.retain(|r| !(r.len() == 1 && r[0].trim().is_empty()));
    rows
}

/// Uma tabela lida do export: cabeçalho em minúsculas e as linhas cruas.
#[derive(Debug, Clone, Default)]
pub struct Table {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

impl Table {
    pub fn from_text(text: &str) -> Self {
        let mut rows = parse_csv(text);
        if rows.is_empty() {
            return Self::default();
        }
        let headers = rows
            .remove(0)
            .iter()
            .map(|h| h.trim().trim_matches('"').to_ascii_lowercase())
            .collect();
        Self { headers, rows }
    }

    /// Índice da primeira coluna cujo nome bate com um dos apelidos.
    pub fn index(&self, names: &[&str]) -> Option<usize> {
        names
            .iter()
            .find_map(|n| self.headers.iter().position(|h| h == n))
            .or_else(|| {
                names.iter().find_map(|n| {
                    self.headers
                        .iter()
                        .position(|h| h.contains(n) || n.contains(h.as_str()))
                })
            })
    }

    pub fn get<'a>(&self, row: &'a [String], names: &[&str]) -> &'a str {
        self.index(names)
            .and_then(|i| row.get(i))
            .map(|s| s.trim())
            .unwrap_or("")
    }
}

const C_DATE: &[&str] = &["date", "created", "created_utc", "timestamp", "created_at"];
const C_SUB: &[&str] = &["subreddit", "subreddit_name", "sub", "channel_name"];
const C_LINK: &[&str] = &["permalink", "url", "link"];
const C_TITLE: &[&str] = &["title"];
const C_BODY: &[&str] = &["body", "message", "content", "text", "selftext"];
const C_SCORE: &[&str] = &["score", "karma", "points"];
const C_DIR: &[&str] = &["direction", "vote", "vote_direction"];

/// Mês (`AAAA-MM`) de uma data do export (`2024-03-01 12:00:00 UTC`, ISO ou
/// segundos desde a época).
pub fn month_of(date: &str) -> Option<String> {
    let d = date.trim();
    if d.len() >= 7 {
        let b = d.as_bytes();
        if b[0..4].iter().all(|c| c.is_ascii_digit())
            && b[4] == b'-'
            && b[5].is_ascii_digit()
            && b[6].is_ascii_digit()
        {
            return Some(d[0..7].to_string());
        }
    }
    let secs = d.split('.').next().unwrap_or(d).parse::<i64>().ok()?;
    chrono::DateTime::from_timestamp(secs, 0).map(|t| t.format("%Y-%m").to_string())
}

// ───────────────────────── resultado ─────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct FileInfo {
    pub name: String,
    pub rows: usize,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Totals {
    pub posts: usize,
    pub comments: usize,
    pub saved_posts: usize,
    pub saved_comments: usize,
    pub votes_up: usize,
    pub votes_down: usize,
    pub subscribed: usize,
    pub messages: usize,
    pub chat_messages: usize,
    pub hidden: usize,
    pub friends: usize,
    pub moderated: usize,
    pub drafts: usize,
    pub poll_votes: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct SubCount {
    pub subreddit: String,
    pub posts: usize,
    pub comments: usize,
    pub total: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct MonthCount {
    pub month: String,
    pub posts: usize,
    pub comments: usize,
    pub total: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Item {
    /// "post" | "comment" | "saved_post" | "saved_comment"
    pub kind: String,
    pub subreddit: String,
    pub date: String,
    pub score: Option<i64>,
    pub title: String,
    pub body: String,
    pub permalink: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Stat {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GdprResult {
    pub source: String,
    pub files: Vec<FileInfo>,
    pub totals: Totals,
    pub top_subreddits: Vec<SubCount>,
    pub timeline: Vec<MonthCount>,
    pub top_posts: Vec<Item>,
    pub top_comments: Vec<Item>,
    pub saved: Vec<Item>,
    pub subscribed: Vec<String>,
    pub statistics: Vec<Stat>,
    /// "score" quando a safra do export trouxe pontuação; senão "date".
    pub ranked_by: String,
    pub first_month: String,
    pub last_month: String,
    pub warnings: Vec<String>,
    pub exported: Vec<String>,
}

// ───────────────────────── leitura da fonte ─────────────────────────

fn is_csv(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".csv")
}

fn short_name(name: &str) -> String {
    name.rsplit(['/', '\\'])
        .next()
        .unwrap_or(name)
        .to_ascii_lowercase()
}

/// Lê os CSV de um zip ou de uma pasta já descompactada.
pub fn read_source(path: &Path) -> Result<HashMap<String, String>> {
    let mut out = HashMap::new();
    if path.is_dir() {
        for entry in walkdir::WalkDir::new(path)
            .max_depth(3)
            .into_iter()
            .flatten()
        {
            let p = entry.path();
            if p.is_file() && is_csv(&p.to_string_lossy()) {
                if let Ok(text) = read_text(p) {
                    out.insert(short_name(&p.to_string_lossy()), text);
                }
            }
        }
        if out.is_empty() {
            return Err(anyhow!(
                "nenhum .csv nesta pasta — aponte para a pasta do export ou para o zip"
            ));
        }
        return Ok(out);
    }
    let lower = path.to_string_lossy().to_ascii_lowercase();
    if !lower.ends_with(".zip") {
        return Err(anyhow!(
            "escolha o zip do export do Reddit ou a pasta onde ele foi descompactado"
        ));
    }
    let file = std::fs::File::open(path)?;
    let mut zip =
        zip::ZipArchive::new(file).map_err(|e| anyhow!("não foi possível abrir o zip: {}", e))?;
    for i in 0..zip.len() {
        let mut entry = match zip.by_index(i) {
            Ok(e) => e,
            Err(_) => continue,
        };
        if !entry.is_file() || !is_csv(entry.name()) {
            continue;
        }
        let name = short_name(entry.name());
        let mut buf = Vec::new();
        if entry.read_to_end(&mut buf).is_ok() {
            out.insert(name, String::from_utf8_lossy(&buf).to_string());
        }
    }
    if out.is_empty() {
        return Err(anyhow!("este zip não tem nenhum .csv do export do Reddit"));
    }
    Ok(out)
}

fn read_text(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)?;
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

// ───────────────────────── resumo ─────────────────────────

fn pick<'a>(files: &'a HashMap<String, String>, stem: &str) -> Option<&'a String> {
    files.get(&format!("{}.csv", stem)).or_else(|| {
        files
            .iter()
            .find(|(k, _)| k.starts_with(stem))
            .map(|(_, v)| v)
    })
}

fn item_from(t: &Table, row: &[String], kind: &str) -> Item {
    let score = t.get(row, C_SCORE).parse::<i64>().ok();
    Item {
        kind: kind.to_string(),
        subreddit: t.get(row, C_SUB).trim_start_matches("r/").to_string(),
        date: t.get(row, C_DATE).to_string(),
        score,
        title: t.get(row, C_TITLE).to_string(),
        body: t.get(row, C_BODY).chars().take(400).collect(),
        permalink: t.get(row, C_LINK).to_string(),
    }
}

/// Lê tudo o que existir e monta o resumo.
pub fn summarize(files: &HashMap<String, String>, source: &str, top: usize) -> GdprResult {
    let mut warnings = Vec::new();
    let mut totals = Totals::default();
    let mut by_sub: HashMap<String, (usize, usize)> = HashMap::new();
    let mut by_month: HashMap<String, (usize, usize)> = HashMap::new();
    let mut posts: Vec<Item> = Vec::new();
    let mut comments: Vec<Item> = Vec::new();
    let mut saved: Vec<Item> = Vec::new();
    let mut subscribed: Vec<String> = Vec::new();
    let mut statistics: Vec<Stat> = Vec::new();
    let mut has_score = false;

    let mut list: Vec<FileInfo> = Vec::new();
    for (name, text) in files {
        let rows = Table::from_text(text).rows.len();
        list.push(FileInfo {
            name: name.clone(),
            rows,
        });
    }
    list.sort_by(|a, b| a.name.cmp(&b.name));

    if let Some(text) = pick(files, "posts") {
        let t = Table::from_text(text);
        totals.posts = t.rows.len();
        for row in &t.rows {
            let item = item_from(&t, row, "post");
            has_score |= item.score.is_some();
            if !item.subreddit.is_empty() {
                by_sub.entry(item.subreddit.clone()).or_default().0 += 1;
            }
            if let Some(m) = month_of(&item.date) {
                by_month.entry(m).or_default().0 += 1;
            }
            posts.push(item);
        }
    } else {
        warnings.push("posts.csv não veio neste export".to_string());
    }

    if let Some(text) = pick(files, "comments") {
        let t = Table::from_text(text);
        totals.comments = t.rows.len();
        for row in &t.rows {
            let item = item_from(&t, row, "comment");
            has_score |= item.score.is_some();
            if !item.subreddit.is_empty() {
                by_sub.entry(item.subreddit.clone()).or_default().1 += 1;
            }
            if let Some(m) = month_of(&item.date) {
                by_month.entry(m).or_default().1 += 1;
            }
            comments.push(item);
        }
    } else {
        warnings.push("comments.csv não veio neste export".to_string());
    }

    for (stem, kind) in [
        ("saved_posts", "saved_post"),
        ("saved_comments", "saved_comment"),
    ] {
        if let Some(text) = pick(files, stem) {
            let t = Table::from_text(text);
            if kind == "saved_post" {
                totals.saved_posts = t.rows.len();
            } else {
                totals.saved_comments = t.rows.len();
            }
            for row in &t.rows {
                saved.push(item_from(&t, row, kind));
            }
        }
    }

    for (name, text) in files {
        if !name.contains("votes") {
            continue;
        }
        let t = Table::from_text(text);
        for row in &t.rows {
            match t.get(row, C_DIR).to_ascii_lowercase().as_str() {
                "up" | "upvote" | "1" => totals.votes_up += 1,
                "down" | "downvote" | "-1" => totals.votes_down += 1,
                _ => {}
            }
        }
        if name.starts_with("poll") {
            totals.poll_votes = t.rows.len();
        }
    }

    if let Some(text) = pick(files, "subscribed_subreddits") {
        let t = Table::from_text(text);
        totals.subscribed = t.rows.len();
        subscribed = t
            .rows
            .iter()
            .map(|r| t.get(r, C_SUB).trim_start_matches("r/").to_string())
            .filter(|s| !s.is_empty())
            .collect();
        subscribed.sort();
    }
    let linhas = |stem: &str| {
        pick(files, stem)
            .map(|t| Table::from_text(t).rows.len())
            .unwrap_or(0)
    };
    totals.messages = linhas("messages");
    totals.chat_messages = linhas("chat_history");
    totals.hidden = linhas("hidden_posts");
    totals.friends = linhas("friends");
    totals.moderated = linhas("moderated_subreddits");
    totals.drafts = linhas("drafts");

    if let Some(text) = pick(files, "statistics") {
        let t = Table::from_text(text);
        for row in &t.rows {
            let name = row.first().cloned().unwrap_or_default();
            let value = row.get(1).cloned().unwrap_or_default();
            if !name.trim().is_empty() {
                statistics.push(Stat {
                    name: name.trim().to_string(),
                    value: value.trim().to_string(),
                });
            }
        }
    }

    let mut top_subreddits: Vec<SubCount> = by_sub
        .into_iter()
        .map(|(subreddit, (p, c))| SubCount {
            subreddit,
            posts: p,
            comments: c,
            total: p + c,
        })
        .collect();
    top_subreddits.sort_by(|a, b| {
        b.total
            .cmp(&a.total)
            .then_with(|| a.subreddit.cmp(&b.subreddit))
    });
    top_subreddits.truncate(top.max(1));

    let mut timeline: Vec<MonthCount> = by_month
        .into_iter()
        .map(|(month, (p, c))| MonthCount {
            month,
            posts: p,
            comments: c,
            total: p + c,
        })
        .collect();
    timeline.sort_by(|a, b| a.month.cmp(&b.month));

    let ranked_by = if has_score { "score" } else { "date" };
    let rank = |v: &mut Vec<Item>| {
        if has_score {
            v.sort_by(|a, b| {
                b.score
                    .unwrap_or(i64::MIN)
                    .cmp(&a.score.unwrap_or(i64::MIN))
            });
        } else {
            v.sort_by(|a, b| b.date.cmp(&a.date));
        }
        v.truncate(top.max(1));
    };
    rank(&mut posts);
    rank(&mut comments);

    GdprResult {
        source: source.to_string(),
        files: list,
        totals,
        top_subreddits,
        first_month: timeline
            .first()
            .map(|m| m.month.clone())
            .unwrap_or_default(),
        last_month: timeline.last().map(|m| m.month.clone()).unwrap_or_default(),
        timeline,
        top_posts: posts,
        top_comments: comments,
        saved,
        subscribed,
        statistics,
        ranked_by: ranked_by.to_string(),
        warnings,
        exported: Vec::new(),
    }
}

// ───────────────────────── exportação ─────────────────────────

fn csv_cell(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn csv_line(cells: &[&str]) -> String {
    let row: Vec<String> = cells.iter().map(|c| csv_cell(c)).collect();
    format!("{}\n", row.join(","))
}

pub fn render_markdown(r: &GdprResult) -> String {
    let t = &r.totals;
    let mut out = String::new();
    out.push_str("# Meus dados do Reddit\n\n");
    out.push_str(&format!("Origem: `{}`\n\n", r.source));
    if !r.first_month.is_empty() {
        out.push_str(&format!(
            "Atividade de {} a {}\n\n",
            r.first_month, r.last_month
        ));
    }
    out.push_str("## Totais\n\n");
    out.push_str("| O quê | Quantos |\n| --- | ---: |\n");
    for (label, n) in [
        ("Posts", t.posts),
        ("Comentários", t.comments),
        ("Posts salvos", t.saved_posts),
        ("Comentários salvos", t.saved_comments),
        ("Votos positivos", t.votes_up),
        ("Votos negativos", t.votes_down),
        ("Subs inscritos", t.subscribed),
        ("Mensagens", t.messages),
        ("Mensagens de chat", t.chat_messages),
        ("Posts ocultos", t.hidden),
        ("Amigos", t.friends),
        ("Subs moderados", t.moderated),
        ("Rascunhos", t.drafts),
        ("Votos em enquete", t.poll_votes),
    ] {
        if n > 0 {
            out.push_str(&format!("| {} | {} |\n", label, n));
        }
    }
    if !r.top_subreddits.is_empty() {
        out.push_str("\n## Subs mais ativos\n\n| Sub | Posts | Comentários | Total |\n| --- | ---: | ---: | ---: |\n");
        for s in &r.top_subreddits {
            out.push_str(&format!(
                "| r/{} | {} | {} | {} |\n",
                s.subreddit, s.posts, s.comments, s.total
            ));
        }
    }
    if !r.timeline.is_empty() {
        out.push_str(
            "\n## Linha do tempo\n\n| Mês | Posts | Comentários |\n| --- | ---: | ---: |\n",
        );
        for m in &r.timeline {
            out.push_str(&format!("| {} | {} | {} |\n", m.month, m.posts, m.comments));
        }
    }
    let destaque = |titulo: &str, itens: &[Item], out: &mut String| {
        if itens.is_empty() {
            return;
        }
        out.push_str(&format!("\n## {}\n\n", titulo));
        for i in itens {
            let head = if i.title.is_empty() {
                i.body
                    .replace('\n', " ")
                    .chars()
                    .take(90)
                    .collect::<String>()
            } else {
                i.title.clone()
            };
            out.push_str(&format!(
                "- {}{} · r/{} · {}{}\n",
                head,
                i.score.map(|s| format!(" ({} pts)", s)).unwrap_or_default(),
                i.subreddit,
                i.date,
                if i.permalink.is_empty() {
                    String::new()
                } else {
                    format!(" · <{}>", i.permalink)
                }
            ));
        }
    };
    destaque("Posts em destaque", &r.top_posts, &mut out);
    destaque("Comentários em destaque", &r.top_comments, &mut out);
    if !r.saved.is_empty() {
        out.push_str(&format!("\n## Salvos ({})\n\n", r.saved.len()));
        for i in r.saved.iter().take(500) {
            out.push_str(&format!("- <{}>\n", i.permalink));
        }
    }
    if !r.subscribed.is_empty() {
        out.push_str("\n## Subs inscritos\n\n");
        out.push_str(
            &r.subscribed
                .iter()
                .map(|s| format!("r/{}", s))
                .collect::<Vec<_>>()
                .join(", "),
        );
        out.push('\n');
    }
    if !r.warnings.is_empty() {
        out.push_str("\n## Observações\n\n");
        for w in &r.warnings {
            out.push_str(&format!("- {}\n", w));
        }
    }
    out
}

fn export(r: &GdprResult, dir: &Path, formats: &[String]) -> Result<Vec<String>> {
    std::fs::create_dir_all(dir)?;
    let want = |f: &str| formats.iter().any(|x| x == f);
    let mut files = Vec::new();
    if want("json") {
        let path = dir.join("reddit-gdpr.json");
        std::fs::write(&path, serde_json::to_string_pretty(r)?)?;
        files.push(path.to_string_lossy().to_string());
    }
    if want("md") {
        let path = dir.join("reddit-gdpr.md");
        std::fs::write(&path, render_markdown(r))?;
        files.push(path.to_string_lossy().to_string());
    }
    if want("csv") {
        let path = dir.join("reddit-gdpr-subreddits.csv");
        let mut s = csv_line(&["subreddit", "posts", "comentarios", "total"]);
        for x in &r.top_subreddits {
            s.push_str(&csv_line(&[
                &x.subreddit,
                &x.posts.to_string(),
                &x.comments.to_string(),
                &x.total.to_string(),
            ]));
        }
        std::fs::write(&path, s)?;
        files.push(path.to_string_lossy().to_string());

        let path = dir.join("reddit-gdpr-linha-do-tempo.csv");
        let mut s = csv_line(&["mes", "posts", "comentarios", "total"]);
        for x in &r.timeline {
            s.push_str(&csv_line(&[
                &x.month,
                &x.posts.to_string(),
                &x.comments.to_string(),
                &x.total.to_string(),
            ]));
        }
        std::fs::write(&path, s)?;
        files.push(path.to_string_lossy().to_string());

        let path = dir.join("reddit-gdpr-salvos.csv");
        let mut s = csv_line(&["tipo", "subreddit", "data", "permalink"]);
        for x in &r.saved {
            s.push_str(&csv_line(&[&x.kind, &x.subreddit, &x.date, &x.permalink]));
        }
        std::fs::write(&path, s)?;
        files.push(path.to_string_lossy().to_string());
    }
    Ok(files)
}

// ───────────────────────── execução ─────────────────────────

fn def_top() -> usize {
    20
}

#[derive(Debug, Clone, Deserialize)]
pub struct Options {
    /// O zip do export ou a pasta onde ele foi descompactado.
    pub path: String,
    /// Quando vem preenchido, grava o resumo aqui.
    #[serde(default)]
    pub export_dir: Option<String>,
    /// "json", "csv", "md".
    #[serde(default)]
    pub formats: Vec<String>,
    #[serde(default = "def_top")]
    pub top: usize,
}

pub fn run(opts: &Options, progress: &ProgressFn) -> Result<GdprResult> {
    report(progress, ID, "started", 0, None, None);
    let path = PathBuf::from(&opts.path);
    if !path.exists() {
        return Err(anyhow!("não achei {}", opts.path));
    }
    let files = read_source(&path)?;
    report(
        progress,
        ID,
        "progress",
        files.len() as u64,
        Some(files.len() as u64),
        Some(format!("{} arquivos", files.len())),
    );
    let mut result = summarize(&files, &opts.path, opts.top);
    if let Some(dir) = opts.export_dir.as_ref().filter(|d| !d.trim().is_empty()) {
        let formats = if opts.formats.is_empty() {
            vec!["json".to_string(), "md".to_string()]
        } else {
            opts.formats.clone()
        };
        result.exported = export(&result, Path::new(dir), &formats)?;
    }
    report(progress, ID, "done", 1, Some(1), None);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("omniget-rdgdpr-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).expect("tempdir");
        d
    }

    const POSTS: &str = "id,permalink,date,ip,subreddit,gildings,title,url,body\n\
        p1,https://www.reddit.com/r/rust/comments/p1/a/,2024-03-02 10:00:00 UTC,1.1.1.1,rust,0,\"Titulo, com virgula\",,corpo\n\
        p2,https://www.reddit.com/r/pics/comments/p2/b/,2024-04-11 10:00:00 UTC,1.1.1.1,pics,0,Outro,,\n";
    const COMMENTS: &str = "id,permalink,date,ip,subreddit,gildings,link,parent,body\n\
        c1,https://www.reddit.com/r/rust/comments/p1/a/c1/,2024-03-03 11:00:00 UTC,1.1.1.1,rust,0,,,\"linha 1\nlinha 2\"\n\
        c2,https://www.reddit.com/r/rust/comments/p1/a/c2/,2024-05-09 11:00:00 UTC,1.1.1.1,rust,0,,,ok\n\
        c3,https://www.reddit.com/r/pics/comments/p2/b/c3/,2024-05-10 11:00:00 UTC,1.1.1.1,pics,0,,,ok\n";
    const SAVED: &str = "id,permalink\ns1,https://www.reddit.com/r/aww/comments/s1/x/\n";
    const VOTES: &str = "id,permalink,direction\nv1,x,up\nv2,y,down\nv3,z,up\n";
    const SUBS: &str = "subreddit\nrust\npics\n";
    const STATS: &str = "statistic,value\naccount_created,2019-01-01\n";

    fn amostra() -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("posts.csv".to_string(), POSTS.to_string());
        m.insert("comments.csv".to_string(), COMMENTS.to_string());
        m.insert("saved_posts.csv".to_string(), SAVED.to_string());
        m.insert("post_votes.csv".to_string(), VOTES.to_string());
        m.insert("subscribed_subreddits.csv".to_string(), SUBS.to_string());
        m.insert("statistics.csv".to_string(), STATS.to_string());
        m
    }

    #[test]
    fn csv_com_aspas_virgula_e_quebra_de_linha() {
        let rows = parse_csv("a,b\n\"x,1\",\"linha 1\nlinha 2\"\n\"aspas \"\"aqui\"\"\",z\n");
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1], vec!["x,1", "linha 1\nlinha 2"]);
        assert_eq!(rows[2], vec!["aspas \"aqui\"", "z"]);
        assert!(parse_csv("").is_empty());
    }

    #[test]
    fn cabecalho_e_colunas_toleram_maiuscula_e_apelido() {
        let t = Table::from_text("ID,Permalink,Date,Subreddit\n1,x,2024-01-02,rust\n");
        assert_eq!(t.headers, vec!["id", "permalink", "date", "subreddit"]);
        assert_eq!(t.get(&t.rows[0], C_SUB), "rust");
        assert_eq!(t.get(&t.rows[0], C_DATE), "2024-01-02");
        assert_eq!(t.get(&t.rows[0], &["nao_existe"]), "");
    }

    #[test]
    fn mes_sai_de_varios_formatos_de_data() {
        assert_eq!(
            month_of("2024-03-02 10:00:00 UTC").as_deref(),
            Some("2024-03")
        );
        assert_eq!(month_of("2024-03-02T10:00:00Z").as_deref(), Some("2024-03"));
        assert_eq!(month_of("1700000000").as_deref(), Some("2023-11"));
        assert_eq!(month_of("qualquer coisa"), None);
    }

    #[test]
    fn resume_o_export() {
        let r = summarize(&amostra(), "amostra", 20);
        assert_eq!(r.totals.posts, 2);
        assert_eq!(r.totals.comments, 3);
        assert_eq!(r.totals.saved_posts, 1);
        assert_eq!(r.totals.votes_up, 2);
        assert_eq!(r.totals.votes_down, 1);
        assert_eq!(r.totals.subscribed, 2);
        assert_eq!(r.top_subreddits[0].subreddit, "rust");
        assert_eq!(r.top_subreddits[0].total, 3);
        assert_eq!(r.first_month, "2024-03");
        assert_eq!(r.last_month, "2024-05");
        assert_eq!(r.timeline.len(), 3);
        assert_eq!(r.ranked_by, "date");
        assert_eq!(r.subscribed, vec!["pics".to_string(), "rust".to_string()]);
        assert_eq!(r.statistics[0].name, "account_created");
        assert!(r.top_posts.iter().any(|p| p.title == "Titulo, com virgula"));
    }

    #[test]
    fn export_incompleto_nao_explode() {
        let mut m = HashMap::new();
        m.insert("comments.csv".to_string(), COMMENTS.to_string());
        let r = summarize(&m, "so comentarios", 5);
        assert_eq!(r.totals.posts, 0);
        assert_eq!(r.totals.comments, 3);
        assert!(r.warnings.iter().any(|w| w.contains("posts.csv")));
    }

    #[test]
    fn usa_score_quando_a_safra_tem_a_coluna() {
        let mut m = HashMap::new();
        m.insert(
            "posts.csv".to_string(),
            "id,date,subreddit,title,score\na,2024-01-01,rust,Baixo,3\nb,2024-01-02,rust,Alto,99\n"
                .to_string(),
        );
        let r = summarize(&m, "com score", 5);
        assert_eq!(r.ranked_by, "score");
        assert_eq!(r.top_posts[0].title, "Alto");
    }

    #[test]
    fn le_pasta_e_zip_e_exporta() {
        let dir = temp();
        let pasta = dir.join("export");
        std::fs::create_dir_all(pasta.join("dentro")).expect("subpasta");
        std::fs::write(pasta.join("posts.csv"), POSTS).expect("posts");
        std::fs::write(pasta.join("dentro").join("comments.csv"), COMMENTS).expect("comments");
        let saida = dir.join("saida");
        let opts = Options {
            path: pasta.to_string_lossy().to_string(),
            export_dir: Some(saida.to_string_lossy().to_string()),
            formats: vec!["json".to_string(), "csv".to_string(), "md".to_string()],
            top: 10,
        };
        let r = run(&opts, &crate::core::tools::noop_progress()).expect("pasta");
        assert_eq!(r.totals.posts, 2);
        assert_eq!(r.totals.comments, 3);
        assert_eq!(r.exported.len(), 5);
        let md = std::fs::read_to_string(saida.join("reddit-gdpr.md")).expect("md");
        assert!(md.contains("| Posts | 2 |"));
        assert!(md.contains("| r/rust | 1 | 2 | 3 |"));

        // Agora o mesmo conteúdo dentro de um zip.
        let zip_path = dir.join("export.zip");
        {
            let file = std::fs::File::create(&zip_path).expect("zip");
            let mut w = zip::ZipWriter::new(file);
            let opt: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            use std::io::Write;
            w.start_file("export/posts.csv", opt).expect("entrada");
            w.write_all(POSTS.as_bytes()).expect("escreve");
            w.start_file("export/comments.csv", opt).expect("entrada");
            w.write_all(COMMENTS.as_bytes()).expect("escreve");
            w.start_file("export/leia-me.txt", opt).expect("entrada");
            w.write_all(b"nao e csv").expect("escreve");
            w.finish().expect("fecha");
        }
        let opts_zip = Options {
            path: zip_path.to_string_lossy().to_string(),
            export_dir: None,
            formats: Vec::new(),
            top: 10,
        };
        let rz = run(&opts_zip, &crate::core::tools::noop_progress()).expect("zip");
        assert_eq!(rz.totals.posts, 2);
        assert_eq!(rz.totals.comments, 3);
        assert_eq!(rz.files.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recusa_arquivo_que_nao_e_export() {
        let dir = temp();
        let f = dir.join("qualquer.txt");
        std::fs::write(&f, "nada").expect("arquivo");
        let opts = Options {
            path: f.to_string_lossy().to_string(),
            export_dir: None,
            formats: Vec::new(),
            top: 5,
        };
        assert!(run(&opts, &crate::core::tools::noop_progress()).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
