//! Letterboxd — o export oficial, automatizado.
//!
//! O site já tem um botão "Export your data" em `/settings/data/` que devolve
//! um ZIP com um CSV por assunto (diário, notas, reviews, watchlist, listas,
//! curtidas). Não existe API pública, e raspar o perfil página a página seria
//! lento e frágil: com a sessão do usuário a gente baixa o mesmo ZIP que o
//! navegador dele baixaria e só normaliza o conteúdo.
//!
//! O único scraping que sobra é o ID do TMDB, que o export não traz: ele está
//! na página do filme, e só é buscado para os títulos que o usuário pedir,
//! com freio entre requisições.

use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

use super::{cell, header_index, iso_date, parse_csv, parse_year, Entry, Fetcher};
use crate::core::tools::{report, ProgressFn};

const TOOL_ID: &str = "lb-export";
pub const DOMAIN: &str = "letterboxd.com";
/// O mesmo endereço que o botão da página de dados aponta.
pub const EXPORT_URL: &str = "https://letterboxd.com/data/export/";

#[derive(Debug, Clone, Deserialize)]
pub struct Options {
    /// Pasta onde gravar os arquivos normalizados.
    pub dest: String,
    /// ZIP já baixado à mão. Vazio = baixa de `/data/export/` com a sessão.
    #[serde(default)]
    pub zip_path: Option<String>,
    #[serde(default)]
    pub account_slug: Option<String>,
    #[serde(default)]
    pub session_netscape: Option<String>,
    /// `diary`, `ratings`, `reviews`, `watched`, `watchlist`, `lists`,
    /// `likes`. Vazio = tudo.
    #[serde(default)]
    pub parts: Vec<String>,
    /// `json`, `csv`, `md`.
    #[serde(default)]
    pub formats: Vec<String>,
    /// Buscar o ID do TMDB na página de cada filme.
    #[serde(default)]
    pub tmdb: bool,
    /// Teto de páginas de filme visitadas quando `tmdb` está ligado.
    #[serde(default = "default_tmdb_limit")]
    pub tmdb_limit: usize,
    #[serde(default = "default_delay")]
    pub delay_ms: u64,
    /// Guardar também o ZIP cru na pasta de destino.
    #[serde(default)]
    pub keep_zip: bool,
}

fn default_tmdb_limit() -> usize {
    120
}
fn default_delay() -> u64 {
    1200
}

#[derive(Debug, Clone, Serialize)]
pub struct PartCount {
    pub part: String,
    pub entries: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportResult {
    pub used_session: bool,
    /// De onde veio o ZIP: o endereço do export ou o caminho local.
    pub source: String,
    pub entries: usize,
    pub by_part: Vec<PartCount>,
    pub tmdb_found: usize,
    pub requests: u32,
    pub files: Vec<String>,
    pub dest: String,
    /// As primeiras linhas, para a UI mostrar sem carregar tudo.
    pub sample: Vec<Entry>,
}

// ── Leitura dos CSVs ────────────────────────────────────────────────────

fn rating_10(s: &str) -> Option<f32> {
    let v = s.trim().parse::<f32>().ok()?;
    if v <= 0.0 {
        return None;
    }
    // O export do Letterboxd grava de 0,5 a 5,0.
    Some((v * 2.0).clamp(0.0, 10.0))
}

fn tags_of(s: &str) -> Vec<String> {
    s.split(',')
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

/// Uma linha de filme dos CSVs "planos": `Date,Name,Year,Letterboxd URI` mais
/// as colunas extras que cada arquivo acrescenta.
fn film_entry(row: &[String], idx: &HashMap<String, usize>, list: &str) -> Option<Entry> {
    let name = cell(row, idx, &["name", "title", "film"]);
    if name.is_empty() {
        return None;
    }
    let mut e = Entry::new("filme", "letterboxd", name);
    e.list = list.to_string();
    e.url = cell(row, idx, &["letterboxd uri", "uri", "url"]).to_string();
    e.year = parse_year(cell(row, idx, &["year"]));
    // No diário a data que importa é a de assistir, não a de registrar.
    let watched = cell(row, idx, &["watched date"]);
    let date = if watched.is_empty() {
        cell(row, idx, &["date"])
    } else {
        watched
    };
    e.date = iso_date(date);
    e.rating = rating_10(cell(row, idx, &["rating"]));
    e.review = cell(row, idx, &["review"]).to_string();
    e.tags = tags_of(cell(row, idx, &["tags"]));
    if cell(row, idx, &["rewatch"]).eq_ignore_ascii_case("yes") {
        e.tags.push("rewatch".to_string());
    }
    Some(e)
}

/// Qualquer CSV plano do export (diary, watched, ratings, reviews,
/// watchlist, likes/films).
pub fn parse_films_csv(text: &str, list: &str) -> Vec<Entry> {
    let rows = parse_csv(text);
    let Some(header) = rows.first() else {
        return Vec::new();
    };
    let idx = header_index(header);
    rows.iter()
        .skip(1)
        .filter_map(|r| film_entry(r, &idx, list))
        .collect()
}

/// As listas do usuário vêm em `lists/<slug>.csv`, com dois blocos: o
/// cabeçalho da lista (data, nome, tags, endereço, descrição), uma linha em
/// branco e só então a tabela dos filmes. Devolve `(nome da lista, filmes)`.
pub fn parse_list_csv(text: &str, fallback_name: &str) -> (String, Vec<Entry>) {
    let rows = parse_csv(text);
    let mut name = fallback_name.to_string();
    let mut list_url = String::new();
    // Bloco 1: cabeçalho com "Name" e sem "Position".
    let mut start = 0usize;
    for (i, row) in rows.iter().enumerate() {
        let low: Vec<String> = row.iter().map(|c| c.trim().to_lowercase()).collect();
        if low.iter().any(|c| c == "position") && low.iter().any(|c| c == "name") {
            start = i;
            break;
        }
        if low.iter().any(|c| c == "name") && low.iter().any(|c| c == "date") {
            if let Some(meta) = rows.get(i + 1) {
                let idx = header_index(row);
                let n = cell(meta, &idx, &["name"]);
                if !n.is_empty() {
                    name = n.to_string();
                }
                list_url = cell(meta, &idx, &["url", "letterboxd uri"]).to_string();
            }
        }
    }
    if start == 0 {
        return (name, Vec::new());
    }
    let idx = header_index(&rows[start]);
    let label = format!("lista: {}", name);
    let mut out = Vec::new();
    for row in rows.iter().skip(start + 1) {
        let Some(mut e) = film_entry(row, &idx, &label) else {
            continue;
        };
        if e.url.is_empty() {
            e.url = list_url.clone();
        }
        // A descrição da linha da lista é a nota do usuário sobre o filme.
        let desc = cell(row, &idx, &["description"]);
        if !desc.is_empty() {
            e.review = desc.to_string();
        }
        out.push(e);
    }
    (name, out)
}

/// A que parte do export um arquivo do ZIP pertence. `None` = ignorar.
///
/// O ZIP traz `deleted/` e `orphaned/` com os mesmos nomes de arquivo da
/// raiz: são o que o usuário apagou e o que perdeu o filme de origem. Nada
/// disso entra no diário, senão o que foi apagado volta do túmulo.
pub fn part_of(path: &str) -> Option<(&'static str, String)> {
    let p = path.replace('\\', "/").to_lowercase();
    if p.starts_with("deleted/")
        || p.starts_with("orphaned/")
        || p.contains("/deleted/")
        || p.contains("/orphaned/")
    {
        return None;
    }
    let file = p.rsplit('/').next().unwrap_or(&p).to_string();
    if p.contains("/lists/") || p.starts_with("lists/") {
        let stem = file.trim_end_matches(".csv").to_string();
        return Some(("lists", stem));
    }
    if p.contains("likes/") {
        return match file.as_str() {
            "films.csv" => Some(("likes", "curtidas".into())),
            _ => None,
        };
    }
    match file.as_str() {
        "diary.csv" => Some(("diary", "diario".into())),
        "ratings.csv" => Some(("ratings", "notas".into())),
        "reviews.csv" => Some(("reviews", "reviews".into())),
        "watched.csv" => Some(("watched", "assistidos".into())),
        "watchlist.csv" => Some(("watchlist", "watchlist".into())),
        _ => None,
    }
}

/// Lê o ZIP inteiro e devolve as linhas por parte.
pub fn parse_zip(bytes: &[u8], parts: &[String]) -> Result<Vec<(String, Vec<Entry>)>> {
    let cursor = std::io::Cursor::new(bytes);
    let mut zip = zip::ZipArchive::new(cursor)
        .context("o arquivo baixado não é um ZIP — a sessão do Letterboxd pode ter expirado")?;
    let want = |p: &str| parts.is_empty() || parts.iter().any(|x| x.eq_ignore_ascii_case(p));
    let mut out: Vec<(String, Vec<Entry>)> = Vec::new();
    for i in 0..zip.len() {
        let mut e = zip.by_index(i)?;
        if !e.is_file() {
            continue;
        }
        let name = e.name().to_string();
        let Some((part, label)) = part_of(&name) else {
            continue;
        };
        if !want(part) {
            continue;
        }
        let mut buf = String::new();
        if e.read_to_string(&mut buf).is_err() {
            continue;
        }
        if part == "lists" {
            let (list_name, entries) = parse_list_csv(&buf, &label);
            if !entries.is_empty() {
                out.push((format!("lista: {}", list_name), entries));
            }
        } else {
            let entries = parse_films_csv(&buf, &label);
            if !entries.is_empty() {
                out.push((label, entries));
            }
        }
    }
    Ok(out)
}

// ── TMDB pela página do filme ───────────────────────────────────────────

/// O ID do TMDB na página do filme.
///
/// A ordem importa: o botão "TMDB" do rodapé (`themoviedb.org/movie/<id>/`
/// ou `/tv/<id>/`) é o que sempre está certo. O `data-tmdb-id` do `<body>`
/// vem **vazio em série** — e ainda por cima com `data-tmdb-type="movie"` —,
/// então ele fica só como reserva para filme.
pub fn tmdb_from_html(html: &str) -> Option<u64> {
    for marker in ["themoviedb.org/movie/", "themoviedb.org/tv/"] {
        if let Some(at) = html.find(marker) {
            let rest = &html[at + marker.len()..];
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(n) = digits.parse::<u64>() {
                if n > 0 {
                    return Some(n);
                }
            }
        }
    }
    let v = super::attr_after(html, "data-tmdb-id", "data-tmdb-id")?;
    v.trim().parse::<u64>().ok().filter(|n| *n > 0)
}

/// O caminho canônico do filme dentro de uma página que não é a dele.
///
/// As URIs de `diary.csv` e `reviews.csv` são links longos do `boxd.it` que
/// levam à página **da entrada do diário** (`/<usuário>/film/<slug>/`), e
/// essa página não tem TMDB nenhum. Ela tem, porém, o link para a página
/// canônica do filme — é dele que a segunda visita sai.
pub fn canonical_film_path(html: &str) -> Option<String> {
    let at = html.find("href=\"/film/")? + "href=\"".len();
    let rest = &html[at..];
    let end = rest.find('"')?;
    let path = &rest[..end];
    let slug = path.trim_start_matches("/film/").trim_end_matches('/');
    if slug.is_empty() || slug.contains('/') {
        return None;
    }
    Some(format!("https://letterboxd.com/film/{}/", slug))
}

/// O Cloudflare do Letterboxd responde com uma página de desafio em vez do
/// conteúdo quando não reconhece o cliente. Sem os cookies do navegador do
/// usuário (o `cf_clearance` entre eles) não há o que fazer daqui.
pub fn is_cloudflare_wall(body: &str) -> bool {
    let low = body.to_lowercase();
    low.contains("just a moment")
        || low.contains("cf-mitigated")
        || low.contains("challenge-platform")
        || low.contains("enable javascript and cookies to continue")
}

/// O IMDb também aparece no mesmo rodapé; sai de graça na mesma página.
pub fn imdb_from_html(html: &str) -> Option<String> {
    let at = html.find("imdb.com/title/")? + "imdb.com/title/".len();
    let id: String = html[at..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect();
    if id.starts_with("tt") && id.len() >= 7 {
        Some(id)
    } else {
        None
    }
}

// ── Execução ────────────────────────────────────────────────────────────

/// Junta as partes numa lista só, sem perder de que parte cada linha veio.
fn flatten(parts: Vec<(String, Vec<Entry>)>) -> (Vec<Entry>, Vec<PartCount>) {
    let mut counts = Vec::new();
    let mut all = Vec::new();
    for (label, entries) in parts {
        counts.push(PartCount {
            part: label,
            entries: entries.len(),
        });
        all.extend(entries);
    }
    all.sort_by(|a, b| b.date.cmp(&a.date).then(a.title.cmp(&b.title)));
    (all, counts)
}

pub async fn run(opts: &Options, p: ProgressFn) -> Result<ExportResult> {
    let dest = PathBuf::from(opts.dest.trim());
    if opts.dest.trim().is_empty() {
        return Err(anyhow!("escolha a pasta de destino"));
    }
    std::fs::create_dir_all(&dest)?;
    let f = Fetcher::new(opts.delay_ms, opts.session_netscape.as_deref(), DOMAIN)?;

    // 1. O ZIP: local ou baixado com a sessão.
    let (bytes, source) = match opts.zip_path.as_deref().map(str::trim) {
        Some(path) if !path.is_empty() => {
            report(&p, TOOL_ID, "progress", 0, Some(3), Some(path.to_string()));
            (
                std::fs::read(path).with_context(|| format!("não consegui ler {}", path))?,
                path.to_string(),
            )
        }
        _ => {
            if !f.has_session() {
                return Err(anyhow!(
                    "sem sessão do Letterboxd: capture os cookies de letterboxd.com na extensão, ou aponte um ZIP já baixado de letterboxd.com/settings/data/"
                ));
            }
            report(
                &p,
                TOOL_ID,
                "progress",
                0,
                Some(3),
                Some(EXPORT_URL.to_string()),
            );
            let (b, ctype) = f.get_bytes(EXPORT_URL).await?;
            if ctype.contains("text/html") || b.len() < 200 {
                let body = String::from_utf8_lossy(&b);
                if is_cloudflare_wall(&body) {
                    return Err(anyhow!(
                        "o Cloudflare do Letterboxd barrou o download. Abra letterboxd.com no navegador, passe pelo \"Just a moment\" e capture os cookies de novo na extensão — o cf_clearance precisa vir junto"
                    ));
                }
                return Err(anyhow!(
                    "o Letterboxd devolveu uma página em vez do ZIP — a sessão salva expirou; capture os cookies de novo"
                ));
            }
            (b, EXPORT_URL.to_string())
        }
    };
    if opts.keep_zip {
        let _ = std::fs::write(dest.join("letterboxd-export.zip"), &bytes);
    }

    // 2. Normalização.
    report(
        &p,
        TOOL_ID,
        "progress",
        1,
        Some(3),
        Some("lendo o export".into()),
    );
    let parts = parse_zip(&bytes, &opts.parts)?;
    let (mut entries, by_part) = flatten(parts);
    if entries.is_empty() {
        return Err(anyhow!("o export veio sem nenhuma linha que eu saiba ler"));
    }

    // 3. TMDB, só para quem pediu e só até o teto.
    let mut tmdb_found = 0usize;
    if opts.tmdb {
        let mut cache: HashMap<String, (Option<u64>, Option<String>)> = HashMap::new();
        let mut visited = 0usize;
        let total = entries.len() as u64;
        for (i, e) in entries.iter_mut().enumerate() {
            if e.url.is_empty() || visited >= opts.tmdb_limit {
                continue;
            }
            let key = e.url.clone();
            if !cache.contains_key(&key) {
                visited += 1;
                report(
                    &p,
                    TOOL_ID,
                    "progress",
                    i as u64,
                    Some(total),
                    Some(e.title.clone()),
                );
                let mut found = match f.get_text(&key).await {
                    Ok(html) => {
                        let ids = (tmdb_from_html(&html), imdb_from_html(&html));
                        // Diário e reviews apontam para a página da entrada,
                        // que não tem os ids: de lá se chega à página do filme.
                        if ids.0.is_none() {
                            match canonical_film_path(&html) {
                                Some(canon) if canon != key => (canon, ids),
                                _ => (String::new(), ids),
                            }
                        } else {
                            (String::new(), ids)
                        }
                    }
                    Err(err) => {
                        tracing::debug!("lb-export: {} sem TMDB: {}", key, err);
                        (String::new(), (None, None))
                    }
                };
                if !found.0.is_empty() && visited < opts.tmdb_limit {
                    visited += 1;
                    if let Ok(html) = f.get_text(&found.0).await {
                        found.1 = (tmdb_from_html(&html), imdb_from_html(&html));
                    }
                }
                cache.insert(key.clone(), found.1);
            }
            if let Some((tmdb, imdb)) = cache.get(&key) {
                e.tmdb_id = *tmdb;
                if let Some(id) = imdb {
                    e.imdb_id = id.clone();
                }
                if tmdb.is_some() {
                    tmdb_found += 1;
                }
            }
        }
    }

    // 4. Arquivos.
    let formats = if opts.formats.is_empty() {
        vec!["json".to_string(), "csv".to_string()]
    } else {
        opts.formats.clone()
    };
    let files = super::write_exports(&dest, "letterboxd", &entries, &formats, |es| {
        super::merge::timeline_markdown("Letterboxd", es)
    })?;

    report(&p, TOOL_ID, "done", 3, Some(3), None);
    Ok(ExportResult {
        used_session: f.has_session(),
        source,
        entries: entries.len(),
        by_part,
        tmdb_found,
        requests: f.requests(),
        files,
        dest: dest.to_string_lossy().to_string(),
        sample: entries.iter().take(40).cloned().collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIARY: &str = "Date,Name,Year,Letterboxd URI,Rating,Rewatch,Tags,Watched Date\n\
2024-03-10,Dune: Part Two,2024,https://boxd.it/aBcD,4.5,No,\"imax, cinema\",2024-03-07\n\
2024-02-01,Amélie,2001,https://boxd.it/xYz,5,Yes,,2024-01-30\n";

    const RATINGS: &str = "Date,Name,Year,Letterboxd URI,Rating\n\
2023-11-02,Cidade de Deus,2002,https://boxd.it/1234,5\n";

    const REVIEWS: &str = "Date,Name,Year,Letterboxd URI,Rating,Rewatch,Review,Tags,Watched Date\n\
2024-03-11,Dune: Part Two,2024,https://boxd.it/aBcD,4.5,No,\"Areia, muita areia.\",,2024-03-07\n";

    const WATCHLIST: &str = "Date,Name,Year,Letterboxd URI\n\
2025-01-05,O Auto da Compadecida 2,2024,https://boxd.it/zzz\n";

    const LIST: &str = "Date,Name,Tags,URL,Description\n\
2022-06-01,Favoritos de sempre,,https://letterboxd.com/fulano/list/favoritos/,\"os que eu revejo\"\n\
\n\
Position,Name,Year,URL,Description\n\
1,Stalker,1979,https://boxd.it/2a,\"o melhor\"\n\
2,Blade Runner 2049,2017,https://boxd.it/2b,\n";

    #[test]
    fn diario_le_nota_data_de_assistir_e_tags() {
        let e = parse_films_csv(DIARY, "diario");
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].title, "Dune: Part Two");
        assert_eq!(e[0].year, Some(2024));
        // A data que vale é a de assistir, não a de registrar no diário.
        assert_eq!(e[0].date, "2024-03-07");
        assert_eq!(e[0].rating, Some(9.0));
        assert_eq!(e[0].tags, vec!["imax", "cinema"]);
        assert_eq!(e[0].kind, "filme");
        assert_eq!(e[0].source, "letterboxd");
        assert_eq!(e[1].date, "2024-01-30");
        assert_eq!(e[1].rating, Some(10.0));
        assert!(e[1].tags.contains(&"rewatch".to_string()));
    }

    #[test]
    fn notas_e_reviews_saem_dos_csvs_certos() {
        let r = parse_films_csv(RATINGS, "notas");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].rating, Some(10.0));
        assert_eq!(r[0].list, "notas");
        let rv = parse_films_csv(REVIEWS, "reviews");
        assert_eq!(rv[0].review, "Areia, muita areia.");
        assert_eq!(rv[0].date, "2024-03-07");
    }

    #[test]
    fn watchlist_fica_sem_nota_e_sem_review() {
        let w = parse_films_csv(WATCHLIST, "watchlist");
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].rating, None);
        assert_eq!(w[0].review, "");
        assert_eq!(w[0].date, "2025-01-05");
    }

    #[test]
    fn lista_le_os_dois_blocos_do_csv() {
        let (name, films) = parse_list_csv(LIST, "fallback");
        assert_eq!(name, "Favoritos de sempre");
        assert_eq!(films.len(), 2);
        assert_eq!(films[0].title, "Stalker");
        assert_eq!(films[0].year, Some(1979));
        assert_eq!(films[0].review, "o melhor");
        assert_eq!(films[0].list, "lista: Favoritos de sempre");
        assert_eq!(films[1].review, "");
    }

    #[test]
    fn lista_sem_bloco_de_filmes_nao_quebra() {
        let (_, films) = parse_list_csv(
            "Date,Name,Tags,URL,Description\n2022-06-01,Vazia,,x,\n",
            "x",
        );
        assert!(films.is_empty());
        assert!(parse_films_csv("", "diario").is_empty());
    }

    #[test]
    fn caminho_do_zip_vira_parte() {
        assert_eq!(part_of("diary.csv").map(|x| x.0), Some("diary"));
        assert_eq!(
            part_of("letterboxd-fulano/ratings.csv").map(|x| x.0),
            Some("ratings")
        );
        assert_eq!(part_of("likes/films.csv").map(|x| x.0), Some("likes"));
        // likes/reviews.csv e likes/lists.csv são "Date,Content": só link.
        assert_eq!(part_of("likes/reviews.csv"), None);
        assert_eq!(part_of("likes/lists.csv"), None);
        assert_eq!(part_of("profile.csv"), None);
        assert_eq!(part_of("comments.csv"), None);
        let (part, stem) = part_of("lists/favoritos.csv").unwrap_or(("", String::new()));
        assert_eq!(part, "lists");
        assert_eq!(stem, "favoritos");
    }

    #[test]
    fn o_que_foi_apagado_nao_volta_do_tumulo() {
        assert_eq!(part_of("deleted/diary.csv"), None);
        assert_eq!(part_of("deleted/reviews.csv"), None);
        assert_eq!(part_of("deleted/lists/antiga.csv"), None);
        assert_eq!(part_of("orphaned/diary.csv"), None);
        assert_eq!(part_of("letterboxd-fulano/deleted/diary.csv"), None);
    }

    #[test]
    fn caminho_canonico_do_filme_sai_da_pagina_do_diario() {
        let entrada = r#"<a href="/fulano/film/past-lives/" class="x">…</a><a href="/film/past-lives/">Past Lives</a>"#;
        assert_eq!(
            canonical_film_path(entrada),
            Some("https://letterboxd.com/film/past-lives/".into())
        );
        // Um caminho mais fundo (/film/x/reviews/) não serve de canônico.
        assert_eq!(
            canonical_film_path(r#"<a href="/film/x/reviews/">r</a>"#),
            None
        );
        assert_eq!(canonical_film_path("<html></html>"), None);
    }

    #[test]
    fn muro_do_cloudflare_e_reconhecido() {
        assert!(is_cloudflare_wall("<title>Just a moment...</title>"));
        assert!(is_cloudflare_wall("<div id=\"challenge-platform\"></div>"));
        assert!(!is_cloudflare_wall(
            "<html><body class=\"film\"></body></html>"
        ));
    }

    #[test]
    fn nota_do_letterboxd_vira_escala_de_dez() {
        assert_eq!(rating_10("0.5"), Some(1.0));
        assert_eq!(rating_10("3"), Some(6.0));
        assert_eq!(rating_10("5"), Some(10.0));
        assert_eq!(rating_10(""), None);
        assert_eq!(rating_10("0"), None);
    }

    #[test]
    fn tmdb_sai_do_botao_do_rodape_ou_do_body() {
        let body = r#"<body id="film-page" data-tmdb-id="693134" data-tmdb-type="movie">"#;
        assert_eq!(tmdb_from_html(body), Some(693134));
        let link = r#"<a href="https://www.themoviedb.org/movie/438631/" data-track-action="TMDB">TMDB</a>"#;
        assert_eq!(tmdb_from_html(link), Some(438631));
        assert_eq!(tmdb_from_html("<html></html>"), None);
        let imdb = r#"<a href="http://www.imdb.com/title/tt1160419/maindetails">IMDb</a>"#;
        assert_eq!(imdb_from_html(imdb), Some("tt1160419".into()));
        assert_eq!(imdb_from_html("nada"), None);
    }

    #[test]
    fn serie_tem_o_body_mentindo_e_o_rodape_certo() {
        // Verificado ao vivo: em série o Letterboxd manda data-tmdb-id vazio
        // e data-tmdb-type="movie". Quem vale é o botão do rodapé.
        let html = r#"<body class="film" data-tmdb-type="movie" data-tmdb-id="">
            <a href="https://www.themoviedb.org/tv/89905/" data-track-action="TMDB">TMDB</a></body>"#;
        assert_eq!(tmdb_from_html(html), Some(89905));
    }

    #[test]
    fn partes_viram_uma_lista_ordenada_por_data() {
        let parts = vec![
            ("notas".to_string(), parse_films_csv(RATINGS, "notas")),
            ("diario".to_string(), parse_films_csv(DIARY, "diario")),
        ];
        let (all, counts) = flatten(parts);
        assert_eq!(all.len(), 3);
        assert_eq!(counts.len(), 2);
        assert_eq!(counts[0].entries, 1);
        assert_eq!(all[0].date, "2024-03-07");
        assert_eq!(all[2].date, "2023-11-02");
    }

    #[test]
    #[ignore = "rede + sessão real: baixa o ZIP de letterboxd.com/data/export/ com os cookies do gerenciador"]
    fn baixa_o_export_de_verdade() {
        // Exercitado à mão com uma conta logada; o CI não tem cookie.
    }
}
