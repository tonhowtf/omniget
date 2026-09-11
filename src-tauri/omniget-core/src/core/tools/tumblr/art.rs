//! `art-favorites-index`: um índice só para os favoritos do DeviantArt, do
//! ArtStation e do Flickr.
//!
//! Os três têm formatos de metadado diferentes; aqui viram um esquema comum
//! com coluna de plataforma. O flag de duplicado reusa o dHash da categoria
//! Pinterest (`pinterest::analysis`), o mesmo que o `img-dupes` usa, então
//! "parecido" quer dizer a mesma coisa nas duas telas.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::super::pinterest::analysis::{dhash, hamming};
use super::csv_line;
use super::gdl::{self, Entry};
use super::likes::base_name;

pub const TOOL_ID: &str = "art-favorites-index";

pub const PLATFORMS: &[&str] = &["deviantart", "artstation", "flickr"];

const IMAGE_EXTS: &[&str] = &[
    "jpg", "jpeg", "png", "webp", "gif", "bmp", "tif", "tiff", "avif",
];

#[derive(Debug, Clone, Deserialize)]
pub struct Options {
    /// Usuário ou URL dos favoritos do DeviantArt (vazio = pular).
    #[serde(default)]
    pub deviantart: Option<String>,
    #[serde(default)]
    pub artstation: Option<String>,
    #[serde(default)]
    pub flickr: Option<String>,
    pub out_dir: String,
    /// Pasta local que já tem arte baixada, para marcar duplicado.
    #[serde(default)]
    pub local_dir: Option<String>,
    #[serde(default)]
    pub limit: Option<u64>,
    /// Baixa a prévia de cada favorito para comparar por dHash. Sem isso o
    /// duplicado só é detectado por nome de arquivo.
    #[serde(default)]
    pub check_dupes: bool,
    #[serde(default = "five")]
    pub threshold: u32,
    #[serde(default = "three_hundred")]
    pub max_check: u64,
    #[serde(default = "yes")]
    pub write_csv: bool,
    #[serde(default = "yes")]
    pub write_json: bool,
    /// Sessões da extensão (os favoritos costumam exigir login).
    #[serde(default)]
    pub session_netscape: Option<String>,
    #[serde(default)]
    pub account_slug: Option<String>,
}

fn yes() -> bool {
    true
}
fn five() -> u32 {
    5
}
fn three_hundred() -> u64 {
    300
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FavRow {
    pub platform: String,
    pub id: String,
    pub title: String,
    pub author: String,
    pub date: String,
    pub tags: Vec<String>,
    pub page_url: String,
    pub media_url: String,
    pub preview_url: String,
    pub width: i64,
    pub height: i64,
    /// "" · "name" (mesmo nome de arquivo) · "hash" (mesma imagem)
    pub dupe: String,
    pub local_file: String,
    pub distance: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArtResult {
    pub total: u64,
    pub by_platform: Vec<(String, u64)>,
    pub dupes: u64,
    pub checked: u64,
    pub used_session: bool,
    pub csv_path: Option<String>,
    pub json_path: Option<String>,
    pub out_dir: String,
    pub sample: Vec<FavRow>,
}

/// Usuário → URL dos favoritos da plataforma. URL completa passa direto.
pub fn source_url(platform: &str, input: &str) -> String {
    let s = input.trim().trim_end_matches('/');
    if s.is_empty() {
        return String::new();
    }
    if s.contains("://") {
        return s.to_string();
    }
    let user = s.trim_start_matches('@');
    match platform {
        "deviantart" => format!("https://www.deviantart.com/{}/favourites/all", user),
        "artstation" => format!("https://www.artstation.com/{}/likes", user),
        "flickr" => format!("https://www.flickr.com/photos/{}/favorites", user),
        _ => String::new(),
    }
}

/// Domínios de cada plataforma, para filtrar a sessão.
pub fn domains() -> Vec<&'static str> {
    vec!["deviantart.com", "artstation.com", "flickr.com"]
}

// ───────────────────────── unificação ─────────────────────────

/// Os três formatos viram a mesma linha. `hint` cobre o caso raro de o dump
/// não trazer `category`.
pub fn unify(hint: &str, e: &Entry) -> Option<FavRow> {
    if !e.is_url() {
        return None;
    }
    let category = {
        let c = e.text("category");
        if c.is_empty() {
            hint.to_string()
        } else {
            c
        }
    };
    let media_url = e.url.clone().unwrap_or_default();
    let row = match category.as_str() {
        "deviantart" => FavRow {
            platform: "deviantart".into(),
            id: e.first_text(&["deviationid", "index", "id"]),
            title: e.text("title"),
            author: e.first_text(&["author.username", "username"]),
            date: e.first_text(&["published_time", "date"]),
            tags: e.tags("tags"),
            page_url: e.first_text(&["url", "target.url"]),
            preview_url: e.first_text(&["preview.src", "thumbs.src", "content.src"]),
            width: e
                .number("content.width")
                .or_else(|| e.number("width"))
                .unwrap_or(0),
            height: e
                .number("content.height")
                .or_else(|| e.number("height"))
                .unwrap_or(0),
            media_url,
            dupe: String::new(),
            local_file: String::new(),
            distance: None,
        },
        "artstation" => FavRow {
            platform: "artstation".into(),
            id: e.first_text(&["hash_id", "asset.id", "id"]),
            title: e.text("title"),
            author: e.first_text(&["user.username", "user.full_name"]),
            date: e.first_text(&["created_at", "published_at", "date"]),
            tags: e.tags("tags"),
            page_url: e.first_text(&["permalink", "url"]),
            preview_url: e.first_text(&["asset.image_url", "cover.small_square_url"]),
            width: e
                .number("asset.width")
                .or_else(|| e.number("width"))
                .unwrap_or(0),
            height: e
                .number("asset.height")
                .or_else(|| e.number("height"))
                .unwrap_or(0),
            media_url,
            dupe: String::new(),
            local_file: String::new(),
            distance: None,
        },
        "flickr" => {
            let id = e.first_text(&["id", "photo_id"]);
            let owner = e.first_text(&["owner.nsid", "owner", "pathalias"]);
            FavRow {
                platform: "flickr".into(),
                id: id.clone(),
                title: e.text("title"),
                author: e.first_text(&["owner.username", "owner.realname", "username"]),
                date: e.first_text(&["date_upload", "datetaken", "date"]),
                tags: e.tags("tags"),
                page_url: if owner.is_empty() || id.is_empty() {
                    e.text("url")
                } else {
                    format!("https://www.flickr.com/photos/{}/{}", owner, id)
                },
                preview_url: e.first_text(&["url_m", "url_z"]),
                width: e.number("width").unwrap_or(0),
                height: e.number("height").unwrap_or(0),
                media_url,
                dupe: String::new(),
                local_file: String::new(),
                distance: None,
            }
        }
        _ => return None,
    };
    if row.id.is_empty() && row.media_url.is_empty() {
        return None;
    }
    Some(row)
}

/// Só as mensagens de arquivo do dump viram linha; diretório e fila caem fora.
pub fn rows_from_dump(hint: &str, entries: &[Entry]) -> Vec<FavRow> {
    entries.iter().filter_map(|e| unify(hint, e)).collect()
}

pub const CSV_HEADER: &[&str] = &[
    "platform",
    "id",
    "title",
    "author",
    "date",
    "tags",
    "page_url",
    "media_url",
    "width",
    "height",
    "dupe",
    "distance",
    "local_file",
];

pub fn csv(rows: &[FavRow]) -> String {
    let mut out = String::new();
    out.push_str(&CSV_HEADER.join(","));
    out.push('\n');
    for r in rows {
        out.push_str(&csv_line(&[
            r.platform.clone(),
            r.id.clone(),
            r.title.replace(['\n', '\r'], " "),
            r.author.clone(),
            r.date.clone(),
            r.tags.join(" "),
            r.page_url.clone(),
            r.media_url.clone(),
            r.width.to_string(),
            r.height.to_string(),
            r.dupe.clone(),
            r.distance.map(|d| d.to_string()).unwrap_or_default(),
            r.local_file.clone(),
        ]));
    }
    out
}

// ───────────────────────── duplicados ─────────────────────────

#[derive(Debug, Clone, Default)]
pub struct LocalArt {
    pub paths: Vec<String>,
    pub hashes: Vec<u64>,
    pub by_name: HashMap<String, String>,
}

impl LocalArt {
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty() && self.by_name.is_empty()
    }
}

fn is_image(path: &Path) -> bool {
    path.extension()
        .map(|e| IMAGE_EXTS.contains(&e.to_string_lossy().to_lowercase().as_str()))
        .unwrap_or(false)
}

/// Varre a pasta do usuário: nome de arquivo para o casamento barato e dHash
/// para o casamento por imagem.
pub fn scan_local(dir: &Path, with_hash: bool, progress: &super::super::ProgressFn) -> LocalArt {
    let mut out = LocalArt::default();
    let mut files: Vec<PathBuf> = Vec::new();
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
            } else if meta.is_file() && is_image(&path) {
                files.push(path);
            }
        }
    }
    let total = files.len() as u64;
    for (i, path) in files.iter().enumerate() {
        let display = path.to_string_lossy().to_string();
        out.by_name
            .entry(base_name(&display).to_lowercase())
            .or_insert_with(|| display.clone());
        if !with_hash {
            continue;
        }
        if i % 25 == 0 {
            super::super::report(progress, TOOL_ID, "progress", i as u64, Some(total), None);
        }
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let Some(h) = dhash(&bytes) else { continue };
        out.paths.push(display);
        out.hashes.push(h);
    }
    out
}

/// Arquivo local mais parecido, dentro do limiar. Empate vai para o mais
/// próximo; nada dentro do limiar devolve `None`.
pub fn best_match(hash: u64, locals: &[u64], threshold: u32) -> Option<(usize, u32)> {
    let mut best: Option<(usize, u32)> = None;
    for (i, h) in locals.iter().enumerate() {
        let d = hamming(hash, *h);
        if d > threshold {
            continue;
        }
        match best {
            Some((_, bd)) if bd <= d => {}
            _ => best = Some((i, d)),
        }
    }
    best
}

/// Casamento barato: mesmo nome de arquivo da URL de mídia.
pub fn match_by_name(rows: &mut [FavRow], local: &LocalArt) -> u64 {
    let mut hits = 0;
    for row in rows.iter_mut() {
        let name = base_name(&row.media_url).to_lowercase();
        if name.is_empty() {
            continue;
        }
        if let Some(path) = local.by_name.get(&name) {
            row.dupe = "name".into();
            row.local_file = path.clone();
            hits += 1;
        }
    }
    hits
}

async fn hash_of_url(client: &reqwest::Client, url: &str) -> Option<u64> {
    let resp = client.get(url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let bytes = resp.bytes().await.ok()?;
    dhash(&bytes)
}

// ───────────────────────── execução ─────────────────────────

pub async fn run(opts: &Options, progress: &super::super::ProgressFn) -> Result<ArtResult> {
    let sources: Vec<(&str, String)> = PLATFORMS
        .iter()
        .filter_map(|p| {
            let raw = match *p {
                "deviantart" => opts.deviantart.as_deref(),
                "artstation" => opts.artstation.as_deref(),
                "flickr" => opts.flickr.as_deref(),
                _ => None,
            }?;
            let url = source_url(p, raw);
            if url.is_empty() {
                None
            } else {
                Some((*p, url))
            }
        })
        .collect();
    if sources.is_empty() {
        anyhow::bail!("informe pelo menos um perfil de favoritos");
    }

    let cookies = match opts.session_netscape.as_deref() {
        Some(s) => gdl::write_cookies(s, &domains())?,
        None => None,
    };
    let used_session = cookies.is_some();
    let cookie_path = cookies.as_ref().map(|c| c.path.clone());

    let mut rows: Vec<FavRow> = Vec::new();
    let mut by_platform: Vec<(String, u64)> = Vec::new();
    for (platform, url) in &sources {
        super::super::report(progress, TOOL_ID, "started", 0, None, Some(url.clone()));
        let entries = gdl::dump(
            url,
            cookie_path.as_deref(),
            opts.limit,
            &[],
            progress,
            TOOL_ID,
        )
        .await?;
        let found = rows_from_dump(platform, &entries);
        by_platform.push((platform.to_string(), found.len() as u64));
        rows.extend(found);
    }
    if rows.is_empty() {
        anyhow::bail!("nenhum favorito foi lido: esses perfis costumam exigir a sessão do site");
    }

    let mut dupes = 0u64;
    let mut checked = 0u64;
    if let Some(dir) = opts.local_dir.as_deref().filter(|d| !d.trim().is_empty()) {
        let want_hash = opts.check_dupes;
        let path = PathBuf::from(dir);
        let p2 = progress.clone();
        let local = tokio::task::spawn_blocking(move || scan_local(&path, want_hash, &p2)).await?;
        if !local.is_empty() {
            dupes = match_by_name(&mut rows, &local);
            if want_hash && !local.hashes.is_empty() {
                let client = super::super::client()?;
                let total = rows.len().min(opts.max_check as usize) as u64;
                for (i, row) in rows.iter_mut().enumerate() {
                    if i as u64 >= opts.max_check {
                        break;
                    }
                    if !row.dupe.is_empty() {
                        continue;
                    }
                    let url = if row.preview_url.is_empty() {
                        row.media_url.clone()
                    } else {
                        row.preview_url.clone()
                    };
                    if url.is_empty() {
                        continue;
                    }
                    super::super::report(
                        progress,
                        TOOL_ID,
                        "progress",
                        i as u64,
                        Some(total),
                        Some(row.title.clone()),
                    );
                    let Some(h) = hash_of_url(&client, &url).await else {
                        continue;
                    };
                    checked += 1;
                    if let Some((idx, d)) = best_match(h, &local.hashes, opts.threshold) {
                        row.dupe = "hash".into();
                        row.distance = Some(d);
                        row.local_file = local.paths[idx].clone();
                        dupes += 1;
                    }
                }
            }
        }
    }

    let out_dir = PathBuf::from(&opts.out_dir);
    std::fs::create_dir_all(&out_dir)?;
    let mut csv_path = None;
    if opts.write_csv {
        let path = out_dir.join("art-favorites.csv");
        std::fs::write(&path, csv(&rows))?;
        csv_path = Some(path.to_string_lossy().to_string());
    }
    let mut json_path = None;
    if opts.write_json {
        let path = out_dir.join("art-favorites.json");
        std::fs::write(&path, serde_json::to_vec_pretty(&rows)?)?;
        json_path = Some(path.to_string_lossy().to_string());
    }

    let total = rows.len() as u64;
    super::super::report(progress, TOOL_ID, "done", total, Some(total), None);
    Ok(ArtResult {
        total,
        by_platform,
        dupes,
        checked,
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
      [2, {"category": "deviantart", "subcategory": "favorite"}],
      [3, "https://images-wixmp.com/f/abc/arte-do-dia.png",
        {"category": "deviantart", "deviationid": "AAA-111", "title": "Arte do dia",
         "author": {"username": "pintora"}, "published_time": "1700000000",
         "tags": [{"tag_name": "fantasia"}, {"tag_name": "azul"}],
         "url": "https://www.deviantart.com/pintora/art/arte-do-dia-1",
         "content": {"src": "https://images-wixmp.com/f/abc/arte-do-dia.png",
                     "width": 1920, "height": 1080},
         "preview": {"src": "https://images-wixmp.com/f/abc/prev.jpg"}}],
      [3, "https://cdna.artstation.com/p/assets/images/images/123/large/estudo.jpg",
        {"category": "artstation", "hash_id": "QwErTy", "title": "Estudo, com vírgula",
         "user": {"username": "modelador", "full_name": "Modelador"},
         "created_at": "2024-02-10T12:00:00.000-06:00",
         "tags": ["3d", "zbrush"],
         "permalink": "https://www.artstation.com/artwork/QwErTy",
         "asset": {"id": 123, "width": 2000, "height": 1200,
                   "image_url": "https://cdna.artstation.com/p/assets/images/images/123/small/estudo.jpg"}}],
      [3, "https://live.staticflickr.com/65535/5551212_o.jpg",
        {"category": "flickr", "id": "5551212", "title": "Pôr do sol",
         "owner": {"nsid": "99887766@N00", "username": "fotografo"},
         "date_upload": "1690000000", "tags": "sunset praia",
         "width": 4000, "height": 3000}]
    ]"#;

    fn rows() -> Vec<FavRow> {
        let entries = gdl::parse_dump(DUMP).expect("dump válido");
        rows_from_dump("", &entries)
    }

    #[test]
    fn os_tres_formatos_viram_o_mesmo_esquema() {
        let rows = rows();
        assert_eq!(rows.len(), 3, "a mensagem de diretório não vira linha");

        assert_eq!(rows[0].platform, "deviantart");
        assert_eq!(rows[0].id, "AAA-111");
        assert_eq!(rows[0].author, "pintora");
        assert_eq!(rows[0].tags, vec!["fantasia", "azul"]);
        assert_eq!(rows[0].width, 1920);
        assert_eq!(
            rows[0].page_url,
            "https://www.deviantart.com/pintora/art/arte-do-dia-1"
        );

        assert_eq!(rows[1].platform, "artstation");
        assert_eq!(rows[1].id, "QwErTy");
        assert_eq!(rows[1].author, "modelador");
        assert_eq!(rows[1].height, 1200);
        assert_eq!(
            rows[1].page_url,
            "https://www.artstation.com/artwork/QwErTy"
        );

        assert_eq!(rows[2].platform, "flickr");
        assert_eq!(rows[2].author, "fotografo");
        assert_eq!(
            rows[2].tags,
            vec!["sunset", "praia"],
            "o Flickr manda as tags como uma string só"
        );
        assert_eq!(
            rows[2].page_url,
            "https://www.flickr.com/photos/99887766@N00/5551212"
        );
    }

    #[test]
    fn csv_tem_coluna_de_plataforma_e_escapa_virgula() {
        let out = csv(&rows());
        let lines: Vec<&str> = out.trim_end().lines().collect();
        assert_eq!(lines.len(), 4);
        assert!(lines[0].starts_with("platform,id,title,author"));
        assert!(lines[1].starts_with("deviantart,"));
        assert!(lines[2].contains("\"Estudo, com vírgula\""));
        assert!(lines[3].starts_with("flickr,"));
    }

    #[test]
    fn duplicado_por_hash_pega_o_mais_proximo_dentro_do_limiar() {
        let locais = vec![0b0000_0000u64, 0b0000_0111u64, 0xFFFF_FFFF_FFFF_FFFF];
        assert_eq!(best_match(0b0000_0001, &locais, 5), Some((0, 1)));
        assert_eq!(best_match(0b0000_0110, &locais, 5), Some((1, 1)));
        assert_eq!(
            best_match(0b0000_1111, &locais, 0),
            None,
            "limiar zero só casa idêntico"
        );
        assert_eq!(best_match(0xFFFF_FFFF_FFFF_FFFF, &locais, 0), Some((2, 0)));
        assert_eq!(best_match(0b0000_0001, &[], 5), None);
    }

    #[test]
    fn duplicado_por_nome_de_arquivo_marca_a_linha() {
        let mut rows = rows();
        let mut local = LocalArt::default();
        local.by_name.insert(
            "arte-do-dia.png".into(),
            "/Users/a/Arte/arte-do-dia.png".into(),
        );
        let hits = match_by_name(&mut rows, &local);
        assert_eq!(hits, 1);
        assert_eq!(rows[0].dupe, "name");
        assert_eq!(rows[0].local_file, "/Users/a/Arte/arte-do-dia.png");
        assert!(rows[1].dupe.is_empty());
    }

    #[test]
    fn url_de_favoritos_por_plataforma() {
        assert_eq!(
            source_url("deviantart", "pintora"),
            "https://www.deviantart.com/pintora/favourites/all"
        );
        assert_eq!(
            source_url("artstation", "@modelador"),
            "https://www.artstation.com/modelador/likes"
        );
        assert_eq!(
            source_url("flickr", "99887766@N00"),
            "https://www.flickr.com/photos/99887766@N00/favorites"
        );
        assert_eq!(
            source_url("flickr", "https://www.flickr.com/photos/x/favorites/"),
            "https://www.flickr.com/photos/x/favorites"
        );
        assert_eq!(source_url("deviantart", "  "), "");
        assert_eq!(source_url("outro", "x"), "");
    }

    #[test]
    fn categoria_desconhecida_nao_vira_linha() {
        let e = Entry {
            kind: gdl::KIND_URL,
            url: Some("https://x/y.jpg".into()),
            meta: serde_json::json!({"category": "pixiv", "id": 1}),
        };
        assert!(unify("", &e).is_none());
        assert!(
            unify("flickr", &e).is_none(),
            "a categoria do dump ganha do palpite"
        );
    }

    #[test]
    fn palpite_cobre_dump_sem_categoria() {
        let e = Entry {
            kind: gdl::KIND_URL,
            url: Some("https://x/y.jpg".into()),
            meta: serde_json::json!({"id": "77", "title": "sem categoria",
                                     "owner": {"nsid": "1@N0", "username": "a"},
                                     "width": 10, "height": 10}),
        };
        let row = unify("flickr", &e).expect("linha do Flickr");
        assert_eq!(row.platform, "flickr");
        assert_eq!(row.id, "77");
    }

    #[test]
    #[ignore = "rede: exige gallery-dl instalado e sessão do DeviantArt/ArtStation/Flickr"]
    fn favoritos_reais_de_um_perfil_publico() {
        let opts = Options {
            deviantart: None,
            artstation: Some("artstation".into()),
            flickr: None,
            out_dir: std::env::temp_dir().to_string_lossy().to_string(),
            local_dir: None,
            limit: Some(5),
            check_dupes: false,
            threshold: 5,
            max_check: 300,
            write_csv: true,
            write_json: true,
            session_netscape: None,
            account_slug: None,
        };
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let out = rt
            .block_on(run(&opts, &super::super::super::noop_progress()))
            .expect("favoritos");
        assert!(out.total > 0);
    }
}
