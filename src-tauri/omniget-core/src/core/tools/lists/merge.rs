//! Diário único: junta o que saiu do Letterboxd, do Trakt, do Goodreads e do
//! Spotify numa linha do tempo só de leu/assistiu/ouviu.
//!
//! Nada de rede aqui — é tudo arquivo que as outras três ferramentas já
//! gravaram (ou que o usuário baixou à mão). A ferramenta reconhece o
//! formato pelo cabeçalho, então tanto faz apontar o `letterboxd.json` que a
//! gente escreveu ou o `diary.csv` cru de dentro do ZIP oficial.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use super::{cell, header_index, iso_date, parse_csv, parse_year, Entry};
use crate::core::tools::{report, ProgressFn};

const TOOL_ID: &str = "media-diary";

#[derive(Debug, Clone, Deserialize)]
pub struct Options {
    /// Arquivos ou pastas com os exports (JSON/CSV) das outras ferramentas.
    pub inputs: Vec<String>,
    /// Export do Spotify: ZIP, pasta ou os `Streaming_History_*.json` soltos.
    #[serde(default)]
    pub spotify: Vec<String>,
    /// Escutas mais curtas que isso não entram na linha do tempo.
    #[serde(default = "default_min_ms")]
    pub min_ms: u64,
    /// Só as músicas com pelo menos tantas escutas no total (o histórico do
    /// Spotify tem dezenas de milhares de linhas; sem isso ele afoga o resto).
    #[serde(default = "default_min_plays")]
    pub min_plays: u32,
    pub dest: String,
    #[serde(default)]
    pub formats: Vec<String>,
    /// Juntar linhas iguais vindas de serviços diferentes.
    #[serde(default = "default_true")]
    pub dedup: bool,
    /// Deixar de fora o que não tem data (watchlist, "quero ler").
    #[serde(default)]
    pub only_dated: bool,
}

fn default_min_ms() -> u64 {
    30_000
}
fn default_min_plays() -> u32 {
    3
}
fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceCount {
    pub source: String,
    pub file: String,
    pub entries: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct YearCount {
    pub year: String,
    pub total: usize,
    pub filmes: usize,
    pub series: usize,
    pub livros: usize,
    pub musicas: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct MergeResult {
    pub entries: usize,
    pub merged: usize,
    pub sources: Vec<SourceCount>,
    pub years: Vec<YearCount>,
    pub first_date: String,
    pub last_date: String,
    pub files: Vec<String>,
    pub dest: String,
    pub sample: Vec<Entry>,
}

// ── Reconhecimento de formato ───────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// O CSV que as nossas ferramentas escrevem.
    Unified,
    /// Um CSV plano do export do Letterboxd (diary, ratings, watchlist…).
    Letterboxd,
    /// O `goodreads_library_export.csv` oficial.
    Goodreads,
    Unknown,
}

/// Descobre o formato pelo cabeçalho. Cada export tem uma coluna que só ele
/// tem, então não precisa adivinhar pelo nome do arquivo.
pub fn sniff(header: &[String]) -> Format {
    let low: Vec<String> = header.iter().map(|h| h.trim().to_lowercase()).collect();
    let has = |name: &str| low.iter().any(|h| h == name);
    if has("exclusive shelf") || has("book id") {
        return Format::Goodreads;
    }
    if has("letterboxd uri") {
        return Format::Letterboxd;
    }
    if has("kind") && has("title") && has("source") {
        return Format::Unified;
    }
    Format::Unknown
}

fn unified_row(row: &[String], idx: &HashMap<String, usize>) -> Option<Entry> {
    let title = cell(row, idx, &["title"]);
    if title.is_empty() {
        return None;
    }
    let kind = cell(row, idx, &["kind"]);
    let source = cell(row, idx, &["source"]);
    let mut e = Entry::new(
        if kind.is_empty() { "filme" } else { kind },
        if source.is_empty() { "import" } else { source },
        title,
    );
    e.date = iso_date(cell(row, idx, &["date"]));
    e.year = parse_year(cell(row, idx, &["year"]));
    e.creator = cell(row, idx, &["creator", "author", "director"]).to_string();
    e.rating = cell(row, idx, &["rating"])
        .parse::<f32>()
        .ok()
        .filter(|r| *r > 0.0);
    e.review = cell(row, idx, &["review"]).to_string();
    e.url = cell(row, idx, &["url"]).to_string();
    e.list = cell(row, idx, &["list"]).to_string();
    e.tags = cell(row, idx, &["tags"])
        .split(';')
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();
    e.tmdb_id = cell(row, idx, &["tmdb_id"]).parse::<u64>().ok();
    e.imdb_id = cell(row, idx, &["imdb_id"]).to_string();
    e.isbn = cell(row, idx, &["isbn"]).to_string();
    e.times = cell(row, idx, &["times"])
        .parse::<u32>()
        .unwrap_or(1)
        .max(1);
    Some(e)
}

/// Lê o conteúdo de um arquivo já carregado. Aceita o JSON do esquema comum,
/// o CSV do esquema comum e os CSVs crus do Letterboxd e do Goodreads.
pub fn parse_any(name: &str, text: &str) -> Vec<Entry> {
    let trimmed = text.trim_start();
    if trimmed.starts_with('[') || trimmed.starts_with('{') {
        // JSON: nosso próprio export, ou o array cru de itens do Trakt.
        if let Ok(list) = serde_json::from_str::<Vec<Entry>>(trimmed) {
            if list.iter().any(|e| !e.title.is_empty()) {
                return list.into_iter().filter(|e| !e.title.is_empty()).collect();
            }
        }
        if let Ok(list) = serde_json::from_str::<Vec<serde_json::Value>>(trimmed) {
            let label = Path::new(name)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "trakt".into());
            let out = super::trakt::parse_items(&list, &label);
            if !out.is_empty() {
                return out;
            }
        }
        return Vec::new();
    }
    let rows = parse_csv(text);
    let Some(header) = rows.first() else {
        return Vec::new();
    };
    let idx = header_index(header);
    match sniff(header) {
        Format::Unified => rows
            .iter()
            .skip(1)
            .filter_map(|r| unified_row(r, &idx))
            .collect(),
        Format::Letterboxd => {
            let label = Path::new(name)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "letterboxd".into());
            super::letterboxd::parse_films_csv(text, &label)
        }
        Format::Goodreads => super::goodreads::parse_export_csv(text),
        Format::Unknown => Vec::new(),
    }
}

/// Todos os arquivos que valem a pena tentar, de uma lista de arquivos e
/// pastas.
pub fn collect_files(inputs: &[String]) -> Vec<PathBuf> {
    let ok = |p: &Path| {
        p.extension()
            .map(|e| e.eq_ignore_ascii_case("csv") || e.eq_ignore_ascii_case("json"))
            .unwrap_or(false)
    };
    let mut out = Vec::new();
    for input in inputs {
        let p = PathBuf::from(input.trim());
        if p.is_dir() {
            for entry in walkdir::WalkDir::new(&p)
                .max_depth(4)
                .follow_links(false)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                if entry.file_type().is_file() && ok(entry.path()) {
                    out.push(entry.path().to_path_buf());
                }
            }
        } else if p.is_file() {
            out.push(p);
        }
    }
    out.sort();
    out.dedup();
    out
}

// ── Spotify ─────────────────────────────────────────────────────────────

/// O export do Spotify já tem leitor no app (`music::history`): aqui é só
/// converter as escutas para o esquema comum, agrupando por faixa para o
/// histórico não afogar filmes e livros na linha do tempo.
pub fn spotify_entries(
    plays: &[crate::core::tools::music::history::Play],
    min_plays: u32,
) -> Vec<Entry> {
    let mut by_track: HashMap<String, Entry> = HashMap::new();
    for play in plays {
        if play.track.is_empty() {
            continue;
        }
        let key = format!(
            "{}|{}",
            super::normalize_title(&play.artist),
            super::normalize_title(&play.track)
        );
        let date = chrono::DateTime::from_timestamp(play.ts, 0)
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_default();
        let slot = by_track.entry(key).or_insert_with(|| {
            let mut e = Entry::new("musica", "spotify", &play.track);
            e.creator = play.artist.clone();
            e.times = 0;
            e
        });
        slot.times += 1;
        // A data que vale é a primeira escuta: é o dia em que entrou na vida.
        if slot.date.is_empty() || (!date.is_empty() && date < slot.date) {
            slot.date = date;
        }
        if slot.list.is_empty() && !play.album.is_empty() {
            slot.list = play.album.clone();
        }
    }
    let mut out: Vec<Entry> = by_track
        .into_values()
        .filter(|e| e.times >= min_plays.max(1))
        .collect();
    out.sort_by(|a, b| b.date.cmp(&a.date).then(a.title.cmp(&b.title)));
    out
}

// ── Deduplicação ────────────────────────────────────────────────────────

/// Qual das duas linhas é a mais completa. Nota e review pesam mais que
/// qualquer outra coisa: é o que o usuário escreveu.
fn richness(e: &Entry) -> u32 {
    let mut score = 0;
    if e.rating.is_some() {
        score += 8;
    }
    if !e.review.is_empty() {
        score += 6;
    }
    if !e.date.is_empty() {
        score += 4;
    }
    if e.tmdb_id.is_some() {
        score += 2;
    }
    if !e.imdb_id.is_empty() || !e.isbn.is_empty() {
        score += 1;
    }
    if !e.creator.is_empty() {
        score += 1;
    }
    if !e.tags.is_empty() {
        score += 1;
    }
    score
}

/// Junta o que é a mesma obra: mantém a linha mais completa e traz dela o que
/// faltava na outra. Origem e lista viram uma lista separada por vírgula, e
/// a data que fica é a mais antiga (a primeira vez conta).
pub fn dedup(entries: Vec<Entry>) -> (Vec<Entry>, usize) {
    let mut order: Vec<String> = Vec::new();
    let mut map: HashMap<String, Entry> = HashMap::new();
    let mut merged = 0usize;
    for e in entries {
        let key = e.dedup_key();
        match map.get_mut(&key) {
            None => {
                order.push(key.clone());
                map.insert(key, e);
            }
            Some(cur) => {
                merged += 1;
                let (mut keep, other) = if richness(&e) > richness(cur) {
                    (e, cur.clone())
                } else {
                    (cur.clone(), e)
                };
                if keep.rating.is_none() {
                    keep.rating = other.rating;
                }
                if keep.review.is_empty() {
                    keep.review = other.review.clone();
                }
                if keep.creator.is_empty() {
                    keep.creator = other.creator.clone();
                }
                if keep.url.is_empty() {
                    keep.url = other.url.clone();
                }
                if keep.tmdb_id.is_none() {
                    keep.tmdb_id = other.tmdb_id;
                }
                if keep.imdb_id.is_empty() {
                    keep.imdb_id = other.imdb_id.clone();
                }
                if keep.isbn.is_empty() {
                    keep.isbn = other.isbn.clone();
                }
                if keep.year.is_none() {
                    keep.year = other.year;
                }
                // A mais antiga das datas conhecidas.
                if keep.date.is_empty() || (!other.date.is_empty() && other.date < keep.date) {
                    keep.date = other.date.clone();
                }
                keep.source = join_unique(&keep.source, &other.source);
                keep.list = join_unique(&keep.list, &other.list);
                for t in other.tags {
                    if !keep.tags.contains(&t) {
                        keep.tags.push(t);
                    }
                }
                keep.times = keep.times.max(1) + other.times.max(1);
                *cur = keep;
            }
        }
    }
    let mut out: Vec<Entry> = order.iter().filter_map(|k| map.remove(k)).collect();
    out.sort_by(|a, b| b.date.cmp(&a.date).then(a.title.cmp(&b.title)));
    (out, merged)
}

fn join_unique(a: &str, b: &str) -> String {
    if b.is_empty() || a == b {
        return a.to_string();
    }
    if a.is_empty() {
        return b.to_string();
    }
    if a.split(", ").any(|p| p == b) {
        return a.to_string();
    }
    format!("{}, {}", a, b)
}

// ── Linha do tempo ──────────────────────────────────────────────────────

pub fn years_of(entries: &[Entry]) -> Vec<YearCount> {
    let mut map: HashMap<String, YearCount> = HashMap::new();
    for e in entries {
        let y = e.year_of_date();
        if y.is_empty() {
            continue;
        }
        let slot = map.entry(y.clone()).or_insert_with(|| YearCount {
            year: y,
            total: 0,
            filmes: 0,
            series: 0,
            livros: 0,
            musicas: 0,
        });
        slot.total += 1;
        match e.kind.as_str() {
            "filme" => slot.filmes += 1,
            "serie" => slot.series += 1,
            "livro" => slot.livros += 1,
            "musica" => slot.musicas += 1,
            _ => {}
        }
    }
    let mut out: Vec<YearCount> = map.into_values().collect();
    out.sort_by(|a, b| b.year.cmp(&a.year));
    out
}

fn icon(kind: &str) -> &'static str {
    match kind {
        "filme" => "Filme",
        "serie" => "Série",
        "livro" => "Livro",
        "musica" => "Música",
        _ => "Outro",
    }
}

const MESES: [&str; 12] = [
    "janeiro",
    "fevereiro",
    "março",
    "abril",
    "maio",
    "junho",
    "julho",
    "agosto",
    "setembro",
    "outubro",
    "novembro",
    "dezembro",
];

fn month_name(month: &str) -> String {
    let n = month
        .split('-')
        .nth(1)
        .and_then(|m| m.parse::<usize>().ok())
        .unwrap_or(0);
    MESES.get(n.saturating_sub(1)).unwrap_or(&"").to_string()
}

/// A linha do tempo em Markdown: um capítulo por ano, um subtítulo por mês.
/// É o formato que as três ferramentas de export usam também.
pub fn timeline_markdown(title: &str, entries: &[Entry]) -> String {
    let mut out = format!("# {}\n\n", title);
    let dated: Vec<&Entry> = entries.iter().filter(|e| !e.date.is_empty()).collect();
    let undated: Vec<&Entry> = entries.iter().filter(|e| e.date.is_empty()).collect();
    out.push_str(&format!("{} linhas\n\n", entries.len()));

    let mut last_year = String::new();
    let mut last_month = String::new();
    for e in &dated {
        let y = e.year_of_date();
        if y != last_year {
            out.push_str(&format!("\n## {}\n", y));
            last_year = y;
            last_month.clear();
        }
        let m = e.month();
        if m != last_month {
            out.push_str(&format!("\n### {}\n\n", month_name(&m)));
            last_month = m;
        }
        out.push_str(&line_md(e));
    }
    if !undated.is_empty() {
        out.push_str("\n## Sem data\n\n");
        for e in &undated {
            out.push_str(&line_md(e));
        }
    }
    out
}

fn line_md(e: &Entry) -> String {
    let mut s = format!("- **{}**", e.title);
    if let Some(y) = e.year {
        s.push_str(&format!(" ({})", y));
    }
    s.push_str(&format!(" · {}", icon(&e.kind)));
    if !e.creator.is_empty() {
        s.push_str(&format!(" · {}", e.creator));
    }
    if let Some(r) = e.rating {
        s.push_str(&format!(" · {:.1}/10", r));
    }
    if !e.date.is_empty() {
        s.push_str(&format!(" · {}", e.date));
    }
    if e.times > 1 {
        s.push_str(&format!(" · {}×", e.times));
    }
    s.push('\n');
    if !e.review.is_empty() {
        let review = e.review.replace('\n', " ");
        s.push_str(&format!("  > {}\n", review.trim()));
    }
    s
}

// ── Execução ────────────────────────────────────────────────────────────

pub fn run(opts: &Options, p: &ProgressFn) -> Result<MergeResult> {
    if opts.dest.trim().is_empty() {
        return Err(anyhow!("escolha a pasta de destino"));
    }
    let files = collect_files(&opts.inputs);
    if files.is_empty() && opts.spotify.is_empty() {
        return Err(anyhow!(
            "aponte os exports do Letterboxd, do Trakt ou do Goodreads (JSON ou CSV)"
        ));
    }
    let total = (files.len() + opts.spotify.len().min(1)) as u64;
    let mut sources = Vec::new();
    let mut all: Vec<Entry> = Vec::new();

    for (i, f) in files.iter().enumerate() {
        let name = f.to_string_lossy().to_string();
        report(
            p,
            TOOL_ID,
            "progress",
            i as u64,
            Some(total),
            Some(name.clone()),
        );
        let Ok(text) = std::fs::read_to_string(f) else {
            continue;
        };
        let entries = parse_any(&name, &text);
        if entries.is_empty() {
            continue;
        }
        sources.push(SourceCount {
            source: entries[0].source.clone(),
            file: name,
            entries: entries.len(),
        });
        all.extend(entries);
    }

    if !opts.spotify.is_empty() {
        report(
            p,
            TOOL_ID,
            "progress",
            files.len() as u64,
            Some(total),
            Some("Spotify".into()),
        );
        let srcs = crate::core::tools::music::history::collect_sources(&opts.spotify)?;
        let mut plays = Vec::new();
        for (_, text) in &srcs {
            plays.extend(crate::core::tools::music::history::parse_history(
                text,
                opts.min_ms,
            ));
        }
        let entries = spotify_entries(&plays, opts.min_plays);
        if !entries.is_empty() {
            sources.push(SourceCount {
                source: "spotify".into(),
                file: opts.spotify.join(", "),
                entries: entries.len(),
            });
            all.extend(entries);
        }
    }

    if all.is_empty() {
        return Err(anyhow!(
            "nenhum dos arquivos apontados tem um formato que eu saiba ler"
        ));
    }
    if opts.only_dated {
        all.retain(|e| !e.date.is_empty());
    }
    let before = all.len();
    let (entries, merged) = if opts.dedup {
        dedup(all)
    } else {
        let mut a = all;
        a.sort_by(|x, y| y.date.cmp(&x.date).then(x.title.cmp(&y.title)));
        (a, 0)
    };
    tracing::debug!("media-diary: {} linhas, {} juntadas", before, merged);

    let dest = PathBuf::from(opts.dest.trim());
    let formats = if opts.formats.is_empty() {
        vec!["json".to_string(), "csv".to_string(), "md".to_string()]
    } else {
        opts.formats.clone()
    };
    let files_out = super::write_exports(&dest, "diario", &entries, &formats, |es| {
        timeline_markdown("Meu diário", es)
    })?;

    let dated: Vec<&Entry> = entries.iter().filter(|e| !e.date.is_empty()).collect();
    report(p, TOOL_ID, "done", total, Some(total), None);
    Ok(MergeResult {
        entries: entries.len(),
        merged,
        sources,
        years: years_of(&entries),
        first_date: dated.last().map(|e| e.date.clone()).unwrap_or_default(),
        last_date: dated.first().map(|e| e.date.clone()).unwrap_or_default(),
        files: files_out,
        dest: dest.to_string_lossy().to_string(),
        sample: entries.iter().take(40).cloned().collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filme(title: &str, year: i32, date: &str, source: &str) -> Entry {
        let mut e = Entry::new("filme", source, title);
        e.year = Some(year);
        e.date = date.to_string();
        e
    }

    #[test]
    fn cabecalho_diz_de_onde_o_csv_veio() {
        let lb = parse_csv("Date,Name,Year,Letterboxd URI,Rating\n");
        assert_eq!(sniff(&lb[0]), Format::Letterboxd);
        let gr = parse_csv("Book Id,Title,Author,My Rating,Exclusive Shelf\n");
        assert_eq!(sniff(&gr[0]), Format::Goodreads);
        let uni = parse_csv("date,kind,title,year,creator,rating,review,source,list,tags\n");
        assert_eq!(sniff(&uni[0]), Format::Unified);
        let nada = parse_csv("a,b,c\n");
        assert_eq!(sniff(&nada[0]), Format::Unknown);
    }

    #[test]
    fn le_json_e_csv_do_nosso_proprio_esquema() {
        let e = filme("Duna", 2021, "2021-10-22", "letterboxd");
        let json = serde_json::to_string(&vec![e.clone()]).unwrap_or_default();
        let back = parse_any("letterboxd.json", &json);
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].title, "Duna");
        let csv = super::super::entries_csv(&[e]);
        let back = parse_any("letterboxd.csv", &csv);
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].date, "2021-10-22");
        assert_eq!(back[0].source, "letterboxd");
    }

    #[test]
    fn le_o_csv_cru_do_letterboxd_e_o_json_cru_do_trakt() {
        let lb =
            "Date,Name,Year,Letterboxd URI,Rating\n2024-03-10,Stalker,1979,https://boxd.it/x,5\n";
        let e = parse_any("diary.csv", lb);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].rating, Some(10.0));
        let trakt = r#"[{"watched_at":"2014-03-31T09:28:53.000Z","type":"movie","movie":{"title":"The Dark Knight","year":2008,"ids":{"slug":"tdk","tmdb":155}}}]"#;
        let e = parse_any("historico.json", trakt);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].tmdb_id, Some(155));
        assert!(parse_any("x.csv", "nada,disso\n1,2\n").is_empty());
        assert!(parse_any("x.json", "{}").is_empty());
    }

    #[test]
    fn caminho_feliz_inteiro_grava_os_tres_formatos() {
        // Nada de rede: o merge é a única das quatro que roda inteira aqui.
        let base = crate::core::tools::temp_dir().join(format!(
            "lists-merge-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let entrada = base.join("entrada");
        let saida = base.join("saida");
        let _ = std::fs::create_dir_all(&entrada);
        let _ = std::fs::write(
            entrada.join("diary.csv"),
            "Date,Name,Year,Letterboxd URI,Rating,Rewatch,Tags,Watched Date\n\
2024-03-10,Dune: Part Two,2024,https://boxd.it/aBcD,4.5,No,,2024-03-07\n",
        );
        let _ = std::fs::write(
            entrada.join("goodreads_library_export.csv"),
            "Book Id,Title,Author,My Rating,Year Published,Original Publication Year,Date Read,Date Added,Bookshelves,Exclusive Shelf,My Review,Read Count\n\
234225,Duna,\"Frank Herbert\",5,2005,1965,2021/10/22,2021/09/01,favoritos,read,\"otimo\",1\n",
        );
        let _ = std::fs::write(
            entrada.join("trakt.json"),
            r#"[{"watched_at":"2024-03-05T10:00:00.000Z","type":"movie","movie":{"title":"Dune: Part Two","year":2024,"ids":{"slug":"dune-part-two-2024","tmdb":693134}}}]"#,
        );
        let opts = Options {
            inputs: vec![entrada.to_string_lossy().to_string()],
            spotify: Vec::new(),
            min_ms: 30_000,
            min_plays: 3,
            dest: saida.to_string_lossy().to_string(),
            formats: vec!["json".into(), "csv".into(), "md".into()],
            dedup: true,
            only_dated: false,
        };
        let r = match run(&opts, &crate::core::tools::noop_progress()) {
            Ok(r) => r,
            Err(e) => panic!("o merge devia ter rodado: {}", e),
        };
        // Duna (filme) do Letterboxd e do Trakt viram uma linha; o livro fica.
        assert_eq!(r.merged, 1);
        assert_eq!(r.entries, 2);
        assert_eq!(r.sources.len(), 3);
        assert_eq!(r.files.len(), 3);
        assert_eq!(r.last_date, "2024-03-05");
        assert_eq!(r.first_date, "2021-10-22");
        for f in &r.files {
            let texto = std::fs::read_to_string(f).unwrap_or_default();
            assert!(!texto.is_empty(), "{} saiu vazio", f);
        }
        let md = std::fs::read_to_string(saida.join("diario.md")).unwrap_or_default();
        assert!(md.contains("## 2024"));
        assert!(md.contains("Frank Herbert"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn dedup_junta_o_mesmo_filme_de_dois_servicos() {
        let mut a = filme("Duna: Parte Dois", 2024, "2024-03-10", "letterboxd");
        a.rating = Some(9.0);
        a.review = "areia".into();
        let mut b = filme("Duna", 2024, "2024-03-07", "trakt");
        b.tmdb_id = Some(693134);
        let (out, merged) = dedup(vec![a, b]);
        assert_eq!(merged, 1);
        assert_eq!(out.len(), 1);
        // Fica a linha mais completa, mas herda o que faltava da outra.
        assert_eq!(out[0].rating, Some(9.0));
        assert_eq!(out[0].review, "areia");
        assert_eq!(out[0].tmdb_id, Some(693134));
        // E a data mais antiga das duas.
        assert_eq!(out[0].date, "2024-03-07");
        assert_eq!(out[0].source, "letterboxd, trakt");
        assert_eq!(out[0].times, 2);
    }

    #[test]
    fn dedup_nao_junta_tipos_nem_anos_diferentes() {
        let filme_duna = filme("Duna", 2021, "2021-10-22", "letterboxd");
        let mut livro_duna = Entry::new("livro", "goodreads", "Duna");
        livro_duna.year = Some(2021);
        let outro_ano = filme("Duna", 1984, "2020-01-01", "trakt");
        let (out, merged) = dedup(vec![filme_duna, livro_duna, outro_ano]);
        assert_eq!(merged, 0);
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn colisao_de_titulo_normalizado_junta_de_proposito() {
        // "The Thing" e "Thing, The" do mesmo ano são o mesmo filme.
        let a = filme("The Thing", 1982, "2019-10-31", "letterboxd");
        let b = filme("Thing, The", 1982, "2020-10-31", "trakt");
        let (out, merged) = dedup(vec![a, b]);
        assert_eq!(merged, 1);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].date, "2019-10-31");
    }

    #[test]
    fn linha_sem_data_sobrevive_a_deduplicacao() {
        let mut wl = filme("Stalker", 1979, "", "letterboxd");
        wl.list = "watchlist".into();
        let visto = filme("Stalker", 1979, "2023-01-05", "trakt");
        let (out, _) = dedup(vec![wl, visto]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].date, "2023-01-05");
        // A linha datada ganha, mas a lembrança de que estava na watchlist fica.
        assert_eq!(out[0].list, "watchlist");
    }

    #[test]
    fn contagem_por_ano_separa_os_tipos() {
        let mut livro = Entry::new("livro", "goodreads", "Duna");
        livro.date = "2024-01-02".into();
        let mut musica = Entry::new("musica", "spotify", "Creep");
        musica.date = "2023-05-05".into();
        let anos = years_of(&[
            filme("A", 2024, "2024-03-10", "trakt"),
            filme("B", 2024, "2024-04-10", "trakt"),
            livro,
            musica,
        ]);
        assert_eq!(anos.len(), 2);
        assert_eq!(anos[0].year, "2024");
        assert_eq!(anos[0].total, 3);
        assert_eq!(anos[0].filmes, 2);
        assert_eq!(anos[0].livros, 1);
        assert_eq!(anos[1].musicas, 1);
    }

    #[test]
    fn markdown_agrupa_por_ano_e_mes() {
        let mut a = filme("Duna", 2021, "2021-10-22", "letterboxd");
        a.rating = Some(8.0);
        a.review = "gostei".into();
        let b = filme("Stalker", 1979, "2020-02-03", "trakt");
        let mut sem_data = filme("Sem data", 2000, "", "trakt");
        sem_data.list = "watchlist".into();
        let md = timeline_markdown("Meu diário", &[a, b, sem_data]);
        assert!(md.starts_with("# Meu diário"));
        assert!(md.contains("## 2021"));
        assert!(md.contains("### outubro"));
        assert!(md.contains("### fevereiro"));
        assert!(md.contains("**Duna** (2021)"));
        assert!(md.contains("8.0/10"));
        assert!(md.contains("> gostei"));
        assert!(md.contains("## Sem data"));
    }

    #[test]
    fn spotify_agrupa_por_faixa_e_respeita_o_minimo() {
        use crate::core::tools::music::history::Play;
        let play = |track: &str, ts: i64| Play {
            ts,
            artist: "Radiohead".into(),
            track: track.into(),
            album: "OK Computer".into(),
            ms: 200_000,
            skipped: false,
            shuffle: None,
            platform: None,
            country: None,
            reason_end: None,
        };
        let plays = vec![
            play("Creep", 1_600_000_000),
            play("Creep", 1_500_000_000),
            play("Creep", 1_700_000_000),
            play("No Surprises", 1_600_000_000),
        ];
        let e = spotify_entries(&plays, 3);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].title, "Creep");
        assert_eq!(e[0].kind, "musica");
        assert_eq!(e[0].times, 3);
        // A data é a da primeira escuta.
        assert_eq!(e[0].date, "2017-07-14");
        assert_eq!(spotify_entries(&plays, 1).len(), 2);
        assert!(spotify_entries(&[], 1).is_empty());
    }

    #[test]
    fn origem_e_lista_viram_uma_lista_sem_repetir() {
        assert_eq!(join_unique("letterboxd", "trakt"), "letterboxd, trakt");
        assert_eq!(join_unique("letterboxd", "letterboxd"), "letterboxd");
        assert_eq!(join_unique("", "trakt"), "trakt");
        assert_eq!(join_unique("a, b", "b"), "a, b");
    }
}
