//! `tumblr-likes-index`: índice pesquisável dos Likes do usuário.
//!
//! O ponto da tool é separar o índice da mídia: um CSV/JSON com post, blog de
//! origem, data, tipo, tags, URL do post e URLs de mídia, mais o caminho local
//! do arquivo quando ele já foi baixado. Assim dá para procurar sem abrir a
//! pasta de mídia.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::gdl::{self, Entry};
use super::{csv_line, safe_slug};

pub const TOOL_ID: &str = "tumblr-likes-index";

#[derive(Debug, Clone, Deserialize)]
pub struct Options {
    /// Nome do blog (`meublog`) ou a URL dos likes.
    pub blog: String,
    /// Onde gravar `likes.csv` / `likes.json`.
    pub out_dir: String,
    /// Pasta de mídia já baixada, para casar arquivo com post.
    #[serde(default)]
    pub media_dir: Option<String>,
    /// Máximo de itens (o gallery-dl corta com `--range`).
    #[serde(default)]
    pub limit: Option<u64>,
    #[serde(default = "yes")]
    pub write_csv: bool,
    #[serde(default = "yes")]
    pub write_json: bool,
    /// Sessão da extensão em formato Netscape (os likes exigem login).
    #[serde(default)]
    pub session_netscape: Option<String>,
    #[serde(default)]
    pub account_slug: Option<String>,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LikeRow {
    pub post_id: String,
    /// Blog de origem do post (quem publicou), não quem curtiu.
    pub blog: String,
    pub date: String,
    /// photo · text · video · audio · link · quote · chat · answer
    pub kind: String,
    pub tags: Vec<String>,
    pub post_url: String,
    pub summary: String,
    pub note_count: i64,
    pub media_urls: Vec<String>,
    pub local_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LikesResult {
    pub posts: u64,
    pub media: u64,
    pub matched_local: u64,
    pub used_session: bool,
    pub csv_path: Option<String>,
    pub json_path: Option<String>,
    pub out_dir: String,
    /// Primeiras linhas, só para a UI mostrar sem ler o arquivo.
    pub sample: Vec<LikeRow>,
}

/// `meublog` → URL dos likes. URL completa passa direto.
pub fn likes_url(input: &str) -> String {
    let s = input.trim().trim_end_matches('/');
    if s.is_empty() {
        return String::new();
    }
    if s.contains("://") || s.contains(".tumblr.com") {
        let base = if s.contains("://") {
            s.to_string()
        } else {
            format!("https://{}", s)
        };
        if base.trim_end_matches('/').ends_with("/likes") {
            return base;
        }
        return format!("{}/likes", base.trim_end_matches('/'));
    }
    let name = s.trim_start_matches('@');
    format!("https://{}.tumblr.com/likes", name)
}

// ───────────────────── índice da pasta de mídia ─────────────────────

/// Mapa da pasta local: nome de arquivo e id de post (lido dos `.json` que o
/// `--write-metadata` grava ao lado de cada arquivo).
#[derive(Debug, Clone, Default)]
pub struct LocalIndex {
    by_name: HashMap<String, Vec<String>>,
    by_post: HashMap<String, Vec<String>>,
}

impl LocalIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_file(&mut self, path: &str) {
        let name = base_name(path).to_lowercase();
        if name.is_empty() {
            return;
        }
        self.by_name.entry(name).or_default().push(path.to_string());
    }

    pub fn insert_post(&mut self, post_id: &str, path: &str) {
        if post_id.is_empty() {
            return;
        }
        self.by_post
            .entry(post_id.to_string())
            .or_default()
            .push(path.to_string());
    }

    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty() && self.by_post.is_empty()
    }

    /// Arquivo local de uma URL de mídia, casando pelo nome do arquivo.
    pub fn file_for_url(&self, url: &str) -> Option<String> {
        let name = base_name(url).to_lowercase();
        if name.is_empty() {
            return None;
        }
        self.by_name.get(&name).and_then(|v| v.first().cloned())
    }

    /// Arquivos locais de um post: primeiro pelo id (sidecar), senão pelo
    /// nome do arquivo da URL de mídia.
    pub fn files_for(&self, post_id: &str, media: &[String]) -> Vec<String> {
        if let Some(found) = self.by_post.get(post_id) {
            let mut out = found.clone();
            out.sort();
            out.dedup();
            return out;
        }
        let mut out: Vec<String> = Vec::new();
        for url in media {
            let name = base_name(url).to_lowercase();
            if let Some(found) = self.by_name.get(&name) {
                out.extend(found.iter().cloned());
            }
        }
        out.sort();
        out.dedup();
        out
    }
}

/// Último segmento de um caminho ou URL, sem query string.
pub fn base_name(path: &str) -> String {
    let no_query = path.split(['?', '#']).next().unwrap_or(path);
    no_query
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(no_query)
        .to_string()
}

/// Varre a pasta de mídia. Os `.json` do `--write-metadata` viram casamento
/// por id de post; os demais arquivos, casamento por nome.
pub fn index_dir(dir: &Path) -> LocalIndex {
    let mut index = LocalIndex::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in read.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                stack.push(path);
                continue;
            }
            if !meta.is_file() {
                continue;
            }
            let display = path.to_string_lossy().to_string();
            if display.to_lowercase().ends_with(".json") {
                if let Some(id) = post_id_of_sidecar(&path) {
                    let media = display.trim_end_matches(".json").trim_end_matches(".JSON");
                    let target = if Path::new(media).exists() {
                        media.to_string()
                    } else {
                        display.clone()
                    };
                    index.insert_post(&id, &target);
                }
                continue;
            }
            index.insert_file(&display);
        }
    }
    index
}

fn post_id_of_sidecar(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let id = value.get("id")?;
    match id {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

// ───────────────────── montagem do índice ─────────────────────

/// Agrupa as mensagens do dump por post: um post de fotos vira uma linha só,
/// com todas as URLs de mídia juntas.
pub fn rows_from_dump(entries: &[Entry]) -> Vec<LikeRow> {
    let mut order: Vec<String> = Vec::new();
    let mut rows: HashMap<String, LikeRow> = HashMap::new();
    for e in entries.iter().filter(|e| e.is_url()) {
        let post_id = e.first_text(&["id", "post.id", "post_id"]);
        let key = if post_id.is_empty() {
            e.url.clone().unwrap_or_default()
        } else {
            post_id.clone()
        };
        if key.is_empty() {
            continue;
        }
        let row = rows.entry(key.clone()).or_insert_with(|| {
            order.push(key.clone());
            LikeRow {
                post_id: post_id.clone(),
                blog: e.first_text(&["blog_name", "blog.name", "reblogged_root_name"]),
                date: e.first_text(&["date", "timestamp"]),
                kind: e.first_text(&["type", "post_type"]),
                tags: e.tags("tags"),
                post_url: e.first_text(&["post_url", "short_url", "url"]),
                summary: e.first_text(&["summary", "title", "caption"]),
                note_count: e.number("note_count").unwrap_or(0),
                media_urls: Vec::new(),
                local_files: Vec::new(),
            }
        });
        if let Some(url) = e.url.as_ref().filter(|u| !u.is_empty()) {
            if !row.media_urls.iter().any(|m| m == url) {
                row.media_urls.push(url.clone());
            }
        }
    }
    order
        .into_iter()
        .filter_map(|k| rows.remove(&k))
        .collect::<Vec<_>>()
}

/// Preenche `local_files` e devolve quantas linhas casaram.
pub fn match_local(rows: &mut [LikeRow], index: &LocalIndex) -> u64 {
    let mut hits = 0u64;
    for row in rows.iter_mut() {
        row.local_files = index.files_for(&row.post_id, &row.media_urls);
        if !row.local_files.is_empty() {
            hits += 1;
        }
    }
    hits
}

pub const CSV_HEADER: &[&str] = &[
    "post_id",
    "blog",
    "date",
    "kind",
    "tags",
    "post_url",
    "summary",
    "note_count",
    "media_urls",
    "local_files",
];

pub fn csv(rows: &[LikeRow]) -> String {
    let mut out = String::new();
    out.push_str(&CSV_HEADER.join(","));
    out.push('\n');
    for r in rows {
        out.push_str(&csv_line(&[
            r.post_id.clone(),
            r.blog.clone(),
            r.date.clone(),
            r.kind.clone(),
            r.tags.join(" "),
            r.post_url.clone(),
            r.summary.replace(['\n', '\r'], " "),
            r.note_count.to_string(),
            r.media_urls.join(" | "),
            r.local_files.join(" | "),
        ]));
    }
    out
}

pub async fn run(opts: &Options, progress: &super::super::ProgressFn) -> Result<LikesResult> {
    let url = likes_url(&opts.blog);
    if url.is_empty() {
        anyhow::bail!("informe o blog dos likes");
    }
    let cookies = match opts.session_netscape.as_deref() {
        Some(s) => gdl::write_cookies(s, &["tumblr.com"])?,
        None => None,
    };
    let used_session = cookies.is_some();

    super::super::report(progress, TOOL_ID, "started", 0, None, Some(url.clone()));
    let extra = vec![
        "-o".to_string(),
        "extractor.tumblr.posts=all".to_string(),
        "-o".to_string(),
        "extractor.tumblr.original=true".to_string(),
    ];
    let entries = gdl::dump(
        &url,
        cookies.as_ref().map(|c| c.path.as_path()),
        opts.limit,
        &extra,
        progress,
        TOOL_ID,
    )
    .await?;

    let mut rows = rows_from_dump(&entries);
    if rows.is_empty() {
        anyhow::bail!(
            "nenhum like foi lido: os likes só aparecem com a sessão do Tumblr e com a lista pública ligada"
        );
    }
    let media: u64 = rows.iter().map(|r| r.media_urls.len() as u64).sum();

    let index = match opts.media_dir.as_deref().filter(|d| !d.trim().is_empty()) {
        Some(dir) => index_dir(Path::new(dir)),
        None => LocalIndex::new(),
    };
    let matched_local = if index.is_empty() {
        0
    } else {
        match_local(&mut rows, &index)
    };

    let out_dir = PathBuf::from(&opts.out_dir);
    std::fs::create_dir_all(&out_dir)?;
    let stem = format!("tumblr-likes-{}", safe_slug(&opts.blog));
    let mut csv_path = None;
    if opts.write_csv {
        let path = out_dir.join(format!("{}.csv", stem));
        std::fs::write(&path, csv(&rows))?;
        csv_path = Some(path.to_string_lossy().to_string());
    }
    let mut json_path = None;
    if opts.write_json {
        let path = out_dir.join(format!("{}.json", stem));
        std::fs::write(&path, serde_json::to_vec_pretty(&rows)?)?;
        json_path = Some(path.to_string_lossy().to_string());
    }

    let posts = rows.len() as u64;
    super::super::report(progress, TOOL_ID, "done", posts, Some(posts), None);
    Ok(LikesResult {
        posts,
        media,
        matched_local,
        used_session,
        csv_path,
        json_path,
        out_dir: out_dir.to_string_lossy().to_string(),
        sample: rows.into_iter().take(30).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUMP: &str = r#"[
      [2, {"category": "tumblr", "subcategory": "likes"}],
      [3, "https://64.media.tumblr.com/aa/foto_1280.jpg",
        {"id": 700111222, "blog_name": "estudio", "type": "photo", "num": 1,
         "tags": ["arte", "aquarela"], "date": "2024-03-02 10:00:00",
         "note_count": 4210, "summary": "estudo de manhã",
         "post_url": "https://estudio.tumblr.com/post/700111222"}],
      [3, "https://64.media.tumblr.com/bb/foto2_1280.jpg",
        {"id": 700111222, "blog_name": "estudio", "type": "photo", "num": 2,
         "tags": ["arte", "aquarela"], "date": "2024-03-02 10:00:00",
         "note_count": 4210, "summary": "estudo de manhã",
         "post_url": "https://estudio.tumblr.com/post/700111222"}],
      [3, "https://va.media.tumblr.com/tumblr_zzz.mp4",
        {"id": 800222333, "blog_name": "cinema", "type": "video",
         "tags": [], "date": "2024-05-09 21:30:00", "note_count": 12,
         "summary": "cena, com vírgula",
         "post_url": "https://cinema.tumblr.com/post/800222333"}]
    ]"#;

    fn rows() -> Vec<LikeRow> {
        let entries = gdl::parse_dump(DUMP).expect("dump válido");
        rows_from_dump(&entries)
    }

    #[test]
    fn um_post_de_varias_fotos_vira_uma_linha_so() {
        let rows = rows();
        assert_eq!(rows.len(), 2, "dois posts, não três arquivos");
        assert_eq!(rows[0].post_id, "700111222");
        assert_eq!(rows[0].media_urls.len(), 2);
        assert_eq!(rows[0].blog, "estudio");
        assert_eq!(rows[0].kind, "photo");
        assert_eq!(rows[0].tags, vec!["arte", "aquarela"]);
        assert_eq!(rows[0].note_count, 4210);
        assert_eq!(rows[1].kind, "video");
    }

    #[test]
    fn a_ordem_do_dump_e_preservada() {
        let rows = rows();
        assert_eq!(rows[0].post_id, "700111222");
        assert_eq!(rows[1].post_id, "800222333");
    }

    #[test]
    fn csv_tem_cabecalho_uma_linha_por_post_e_escapa_virgula() {
        let out = csv(&rows());
        let lines: Vec<&str> = out.trim_end().lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("post_id,blog,date,kind,tags"));
        assert!(lines[1].contains("https://64.media.tumblr.com/aa/foto_1280.jpg | "));
        assert!(
            lines[2].contains("\"cena, com vírgula\""),
            "vírgula no resumo tem que sair entre aspas: {}",
            lines[2]
        );
    }

    #[test]
    fn casamento_local_pelo_nome_do_arquivo() {
        let mut rows = rows();
        let mut index = LocalIndex::new();
        index.insert_file("/Users/a/Tumblr/foto_1280.jpg");
        index.insert_file("/Users/a/Tumblr/nao-relacionado.png");
        let hits = match_local(&mut rows, &index);
        assert_eq!(hits, 1);
        assert_eq!(rows[0].local_files, vec!["/Users/a/Tumblr/foto_1280.jpg"]);
        assert!(rows[1].local_files.is_empty());
    }

    #[test]
    fn casamento_local_pelo_id_do_post_ganha_do_nome() {
        let mut rows = rows();
        let mut index = LocalIndex::new();
        index.insert_file("/Users/a/Tumblr/foto_1280.jpg");
        index.insert_post("700111222", "/Users/a/Tumblr/certo.jpg");
        match_local(&mut rows, &index);
        assert_eq!(rows[0].local_files, vec!["/Users/a/Tumblr/certo.jpg"]);
    }

    #[test]
    fn base_name_ignora_query_string() {
        assert_eq!(base_name("https://x/y/foto_1280.jpg?w=1"), "foto_1280.jpg");
        assert_eq!(base_name("C:\\fotos\\a.png"), "a.png");
        assert_eq!(base_name("simples.gif"), "simples.gif");
    }

    #[test]
    fn url_dos_likes_aceita_nome_e_url() {
        assert_eq!(likes_url("meublog"), "https://meublog.tumblr.com/likes");
        assert_eq!(likes_url("@meublog"), "https://meublog.tumblr.com/likes");
        assert_eq!(
            likes_url("https://meublog.tumblr.com"),
            "https://meublog.tumblr.com/likes"
        );
        assert_eq!(
            likes_url("https://meublog.tumblr.com/likes"),
            "https://meublog.tumblr.com/likes"
        );
        assert_eq!(
            likes_url("meublog.tumblr.com/"),
            "https://meublog.tumblr.com/likes"
        );
        assert_eq!(likes_url("  "), "");
    }

    #[test]
    #[ignore = "rede: exige gallery-dl instalado e sessão do Tumblr com likes públicos"]
    fn likes_reais_de_um_blog_com_sessao() {
        let opts = Options {
            blog: "staff".into(),
            out_dir: std::env::temp_dir().to_string_lossy().to_string(),
            media_dir: None,
            limit: Some(5),
            write_csv: true,
            write_json: true,
            session_netscape: None,
            account_slug: None,
        };
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let out = rt
            .block_on(run(&opts, &super::super::super::noop_progress()))
            .expect("likes");
        assert!(out.posts > 0);
    }
}
