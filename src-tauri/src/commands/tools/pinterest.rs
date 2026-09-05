//! Comandos da categoria Pinterest (estudo 67). Todos aceitam `cookies`
//! opcional (caminho de cookies.txt ou string `a=1; b=2`) para boards
//! secretos; sem cookies só o conteúdo público.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::StreamExt;
use omniget_core::core::tools::pinterest::analysis::{self, DupeGroup, Swatch};
use omniget_core::core::tools::pinterest::api::Feed;
use omniget_core::core::tools::pinterest::media::{self, DownloadOptions};
use omniget_core::core::tools::pinterest::{export, parse_target, Board, Pin, PinClient, Section, Target, User};
use omniget_core::core::tools::{report, ProgressFn};
use serde::{Deserialize, Serialize};

use super::{err, progress};

fn client(cookies: &Option<String>) -> Result<Arc<PinClient>, String> {
    PinClient::new(cookies.as_deref()).map(Arc::new).map_err(err)
}

fn target_of(url: &str) -> Result<Target, String> {
    parse_target(url).ok_or_else(|| "cole um link do Pinterest (pin, board, seção, perfil ou pin.it) ou um texto para buscar".to_string())
}

async fn resolve(c: &PinClient, url: &str) -> Result<(Target, String), String> {
    let mut t = target_of(url)?;
    let mut resolved = url.trim().to_string();
    if let Target::Short { code } = &t {
        resolved = c.resolve_short(code).await.map_err(err)?;
        t = target_of(&resolved)?;
    }
    Ok((t, resolved))
}

// ───────────────────────── filtros ─────────────────────────

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Filters {
    #[serde(default)]
    pub skip_promoted: bool,
    /// 0 desliga; 1 esconde tudo com sinal fraco; 2 ferramentas citadas; 3 só frases explícitas/rótulo oficial
    #[serde(default)]
    pub ai_level: u8,
    /// "" | "image" | "video" | "gif"
    #[serde(default)]
    pub only_kind: String,
    #[serde(default)]
    pub min_width: u32,
}

fn keep(p: &Pin, f: &Filters) -> bool {
    if f.skip_promoted && p.is_promoted {
        return false;
    }
    if f.ai_level > 0 && (p.ai.labeled || (p.ai.keyword_level > 0 && p.ai.keyword_level >= f.ai_level)) {
        return false;
    }
    if !f.only_kind.is_empty() {
        let k = p.kind.as_str();
        let ok = match f.only_kind.as_str() {
            "video" => k == "video",
            "gif" => k == "gif",
            "image" => k == "image" || k == "carousel" || k == "story",
            _ => true,
        };
        if !ok {
            return false;
        }
    }
    if f.min_width > 0 {
        let w = p.image.as_ref().map(|m| m.width).filter(|w| *w > 0).or_else(|| p.image_large.as_ref().map(|m| m.width)).unwrap_or(0);
        if w > 0 && w < f.min_width {
            return false;
        }
    }
    true
}

// ───────────────────────── inspecionar ─────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct InspectOut {
    pub target: Target,
    pub resolved_url: String,
    pub pin: Option<Pin>,
    pub board: Option<Board>,
    pub section: Option<Section>,
    pub sections: Vec<Section>,
    pub user: Option<User>,
    pub boards: Vec<Board>,
    pub has_session: bool,
}

#[tauri::command]
pub async fn tool_pin_inspect(url: String, cookies: Option<String>) -> Result<InspectOut, String> {
    let c = client(&cookies)?;
    let (target, resolved_url) = resolve(&c, &url).await?;
    let mut out = InspectOut {
        target: target.clone(),
        resolved_url,
        pin: None,
        board: None,
        section: None,
        sections: vec![],
        user: None,
        boards: vec![],
        has_session: c.has_session(),
    };
    match &target {
        Target::Pin { id } => out.pin = Some(c.pin(id).await.map_err(err)?),
        Target::Board { user, slug } => {
            let b = c.board(user, slug).await.map_err(err)?;
            if b.section_count > 0 {
                out.sections = c.board_sections(&b.id).await.unwrap_or_default();
            }
            out.board = Some(b);
        }
        Target::Section { user, slug, section } => {
            let b = c.board(user, slug).await.map_err(err)?;
            let secs = c.board_sections(&b.id).await.unwrap_or_default();
            out.section = secs.iter().find(|x| &x.slug == section || x.title.eq_ignore_ascii_case(section)).cloned();
            out.sections = secs;
            out.board = Some(b);
        }
        Target::User { username } | Target::UserCreated { username } => {
            out.user = Some(c.user(username).await.map_err(err)?);
            out.boards = c.user_boards(username).await.unwrap_or_default();
        }
        Target::Search { .. } | Target::Short { .. } => {}
    }
    Ok(out)
}

// ───────────────────────── listar ─────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct ListOut {
    pub title: String,
    pub target: Target,
    pub pins: Vec<Pin>,
    /// termos relacionados (busca)
    pub guides: Vec<String>,
    /// quantos foram escondidos pelos filtros
    pub hidden: usize,
}

async fn list_pins(c: &PinClient, target: &Target, limit: usize, filters: &Filters, pid: &str, p: &ProgressFn) -> Result<(String, Vec<Pin>, Vec<String>, usize), String> {
    let (feed, title) = c.feed_for(target).await.map_err(err)?;
    let mut guides = Vec::new();
    if let Feed::Search { .. } = &feed {
        if let Ok((_, _, g)) = c.feed_page(&feed, None, 25).await {
            guides = g;
        }
    }
    // com filtro, pede um pouco mais para compensar o que cai
    let fetch = if limit == 0 { 0 } else if filters.skip_promoted || filters.ai_level > 0 || !filters.only_kind.is_empty() { limit * 2 } else { limit };
    let pins = c
        .collect(&feed, fetch, |n| report(p, pid, "list", n as u64, None, Some(title.clone())))
        .await
        .map_err(err)?;
    let before = pins.len();
    let mut kept: Vec<Pin> = pins.into_iter().filter(|x| keep(x, filters)).collect();
    if limit > 0 {
        kept.truncate(limit);
    }
    let hidden = before.saturating_sub(kept.len());
    Ok((title, kept, guides, hidden))
}

#[tauri::command]
pub async fn tool_pin_list(app: tauri::AppHandle, url: String, cookies: Option<String>, limit: Option<usize>, filters: Option<Filters>) -> Result<ListOut, String> {
    let c = client(&cookies)?;
    let (target, _) = resolve(&c, &url).await?;
    let filters = filters.unwrap_or_default();
    let p = progress(&app);
    let (title, pins, guides, hidden) = list_pins(&c, &target, limit.unwrap_or(100), &filters, "pinterest:list", &p).await?;
    Ok(ListOut { title, target, pins, guides, hidden })
}

/// Pins parecidos com um pin (o "More like this").
#[tauri::command]
pub async fn tool_pin_related(app: tauri::AppHandle, url: String, cookies: Option<String>, limit: Option<usize>, filters: Option<Filters>) -> Result<ListOut, String> {
    let c = client(&cookies)?;
    let (target, _) = resolve(&c, &url).await?;
    let Target::Pin { id } = &target else {
        return Err("cole o link de um pin".into());
    };
    let filters = filters.unwrap_or_default();
    let p = progress(&app);
    let feed = Feed::Related { pin_id: id.clone() };
    let pins = c
        .collect(&feed, limit.unwrap_or(60) * 2, |n| report(&p, "pinterest:related", "list", n as u64, None, None))
        .await
        .map_err(err)?;
    let before = pins.len();
    let mut kept: Vec<Pin> = pins.into_iter().filter(|x| x.id != *id && keep(x, &filters)).collect();
    kept.truncate(limit.unwrap_or(60));
    Ok(ListOut { title: format!("parecidos com {}", id), target, hidden: before - kept.len(), pins: kept, guides: vec![] })
}

/// Busca de boards por palavra.
#[tauri::command]
pub async fn tool_pin_boards_search(query: String, cookies: Option<String>, limit: Option<usize>) -> Result<Vec<Board>, String> {
    let c = client(&cookies)?;
    c.search_boards(&query, limit.unwrap_or(40)).await.map_err(err)
}

// ───────────────────────── baixar ─────────────────────────

#[tauri::command]
pub async fn tool_pin_download(app: tauri::AppHandle, url: String, opts: DownloadOptions, cookies: Option<String>) -> Result<media::PinFiles, String> {
    let c = client(&cookies)?;
    let (target, _) = resolve(&c, &url).await?;
    let Target::Pin { id } = &target else {
        return Err("para boards, seções e perfis use o backup".into());
    };
    let p = progress(&app);
    report(&p, "pinterest:pin", "download", 0, Some(1), None);
    let pin = c.pin(id).await.map_err(err)?;
    let dir = PathBuf::from(&opts.dest);
    let files = media::download_pin(&c, &pin, &dir, &opts).await.map_err(err)?;
    report(&p, "pinterest:pin", "done", 1, Some(1), None);
    Ok(media::PinFiles { id: pin.id.clone(), files, skipped: false, error: None })
}

#[derive(Debug, Clone, Serialize)]
pub struct ManyOut {
    pub dest: String,
    pub downloaded: usize,
    pub skipped: usize,
    pub failed: Vec<media::PinFiles>,
    pub files: usize,
}

async fn download_many(c: Arc<PinClient>, pins: Vec<Pin>, opts: &DownloadOptions, root: &Path, pid: &str, p: &ProgressFn) -> (HashMap<String, Vec<String>>, ManyOut) {
    let archive = if opts.skip_downloaded { media::load_archive(root) } else { Default::default() };
    let total = pins.len();
    let mut done = 0usize;
    let mut out = ManyOut { dest: root.to_string_lossy().to_string(), downloaded: 0, skipped: 0, failed: vec![], files: 0 };
    let mut files_by_id: HashMap<String, Vec<String>> = HashMap::new();
    let opts = Arc::new(opts.clone());
    let root_buf = root.to_path_buf();
    let jobs = pins.into_iter().map(|pin| {
        let c = c.clone();
        let opts = opts.clone();
        let root = root_buf.clone();
        let skip = archive.contains(&pin.id);
        async move {
            if skip {
                return (pin, Ok(Vec::new()), true);
            }
            let dir = match (&pin.section, opts.section_folders) {
                (Some(s), true) if !s.is_empty() => root.join(omniget_core::core::tools::sanitize_name(s)),
                _ => root.clone(),
            };
            let r = media::download_pin(&c, &pin, &dir, &opts).await;
            (pin, r, false)
        }
    });
    let mut stream = futures::stream::iter(jobs).buffer_unordered(4);
    while let Some((pin, r, skipped)) = stream.next().await {
        done += 1;
        if skipped {
            out.skipped += 1;
        } else {
            match r {
                Ok(files) => {
                    out.downloaded += 1;
                    out.files += files.len();
                    media::append_archive(root, &pin.id);
                    files_by_id.insert(pin.id.clone(), files);
                }
                Err(e) => out.failed.push(media::PinFiles { id: pin.id.clone(), files: vec![], skipped: false, error: Some(e.to_string()) }),
            }
        }
        report(p, pid, "download", done as u64, Some(total as u64), Some(pin.title.clone()));
    }
    (files_by_id, out)
}

/// Baixa uma lista de pins já listada (resultados de busca selecionados).
#[tauri::command]
pub async fn tool_pin_download_many(app: tauri::AppHandle, pins: Vec<Pin>, opts: DownloadOptions, cookies: Option<String>) -> Result<ManyOut, String> {
    let c = client(&cookies)?;
    let root = PathBuf::from(&opts.dest);
    std::fs::create_dir_all(&root).map_err(err)?;
    let p = progress(&app);
    let (_, out) = download_many(c, pins, &opts, &root, "pinterest:many", &p).await;
    report(&p, "pinterest:many", "done", out.downloaded as u64, Some(out.downloaded as u64), None);
    Ok(out)
}

#[derive(Debug, Clone, Deserialize)]
pub struct BackupOptions {
    pub url: String,
    pub download: DownloadOptions,
    #[serde(default)]
    pub cookies: Option<String>,
    /// 0 = tudo
    #[serde(default)]
    pub limit: usize,
    #[serde(default)]
    pub filters: Filters,
    /// grava pins.csv e pins.json
    #[serde(default = "yes")]
    pub metadata: bool,
    /// grava index.html (galeria offline)
    #[serde(default = "yes")]
    pub gallery: bool,
    /// perfil: também os pins criados
    #[serde(default)]
    pub include_created: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Serialize)]
pub struct BackupOut {
    pub title: String,
    pub dest: String,
    pub total: usize,
    pub hidden: usize,
    pub downloaded: usize,
    pub skipped: usize,
    pub failed: Vec<media::PinFiles>,
    pub files: usize,
    pub csv_path: Option<String>,
    pub json_path: Option<String>,
    pub html_path: Option<String>,
    /// boards processados (perfil)
    pub boards: usize,
}

/// Lista um board com as seções nomeadas (pins soltos + cada seção).
async fn board_with_sections(c: &PinClient, board: &Board, limit: usize, pid: &str, p: &ProgressFn) -> Result<Vec<Pin>, String> {
    let mut all: Vec<Pin> = Vec::new();
    let loose = c
        .collect(&Feed::Board { board_id: board.id.clone(), include_sections: false }, limit, |n| report(p, pid, "list", n as u64, None, Some(board.name.clone())))
        .await
        .map_err(err)?;
    all.extend(loose);
    if board.section_count > 0 && (limit == 0 || all.len() < limit) {
        for sec in c.board_sections(&board.id).await.unwrap_or_default() {
            let want = if limit == 0 { 0 } else { limit.saturating_sub(all.len()).max(1) };
            let pins = c
                .collect(&Feed::Section { section_id: sec.id.clone() }, want, |n| report(p, pid, "list", (all.len() + n) as u64, None, Some(sec.title.clone())))
                .await
                .unwrap_or_default();
            for mut x in pins {
                if !all.iter().any(|y| y.id == x.id) {
                    x.section = Some(sec.title.clone());
                    all.push(x);
                }
            }
            if limit > 0 && all.len() >= limit {
                break;
            }
        }
    }
    if limit > 0 {
        all.truncate(limit);
    }
    Ok(all)
}

async fn write_outputs(title: &str, dest: &Path, pins: &[Pin], files: &HashMap<String, Vec<String>>, metadata: bool, gallery: bool) -> (Option<String>, Option<String>, Option<String>) {
    let mut csv_path = None;
    let mut json_path = None;
    let mut html_path = None;
    if metadata {
        let p = dest.join("pins.csv");
        if tokio::fs::write(&p, export::csv(pins, files)).await.is_ok() {
            csv_path = Some(p.to_string_lossy().to_string());
        }
        let p = dest.join("pins.json");
        if let Ok(j) = serde_json::to_string_pretty(pins) {
            if tokio::fs::write(&p, j).await.is_ok() {
                json_path = Some(p.to_string_lossy().to_string());
            }
        }
    }
    if gallery {
        let p = dest.join("index.html");
        let sub = format!("{} pins · OmniGet · {}", pins.len(), chrono::Local::now().format("%Y-%m-%d %H:%M"));
        let html = export::html_gallery(title, &sub, pins, files, &dest.to_string_lossy());
        if tokio::fs::write(&p, html).await.is_ok() {
            html_path = Some(p.to_string_lossy().to_string());
        }
    }
    (csv_path, json_path, html_path)
}

/// Backup de board, seção, perfil (todos os boards), busca ou relacionados.
#[tauri::command]
pub async fn tool_pin_backup(app: tauri::AppHandle, opts: BackupOptions) -> Result<BackupOut, String> {
    let c = client(&opts.cookies)?;
    let (target, _) = resolve(&c, &opts.url).await?;
    let p = progress(&app);
    let pid = "pinterest:backup";
    let root = PathBuf::from(&opts.download.dest);
    std::fs::create_dir_all(&root).map_err(err)?;

    // perfil: um subdiretório por board
    if let Target::User { username } = &target {
        let user = c.user(username).await.map_err(err)?;
        let boards = c.user_boards(username).await.map_err(err)?;
        let mut agg = BackupOut {
            title: if user.name.is_empty() { username.clone() } else { user.name.clone() },
            dest: root.to_string_lossy().to_string(),
            total: 0,
            hidden: 0,
            downloaded: 0,
            skipped: 0,
            failed: vec![],
            files: 0,
            csv_path: None,
            json_path: None,
            html_path: None,
            boards: boards.len(),
        };
        let mut all_pins: Vec<Pin> = Vec::new();
        let mut all_files: HashMap<String, Vec<String>> = HashMap::new();
        for (i, b) in boards.iter().enumerate() {
            report(&p, pid, "board", i as u64, Some(boards.len() as u64), Some(b.name.clone()));
            let pins = match board_with_sections(&c, b, opts.limit, pid, &p).await {
                Ok(x) => x,
                Err(e) => {
                    agg.failed.push(media::PinFiles { id: b.id.clone(), files: vec![], skipped: false, error: Some(e) });
                    continue;
                }
            };
            let before = pins.len();
            let pins: Vec<Pin> = pins
                .into_iter()
                .filter(|x| keep(x, &opts.filters))
                .map(|mut x| {
                    x.board = Some(omniget_core::core::tools::pinterest::api::BoardRef { id: Some(b.id.clone()), name: Some(b.name.clone()), url: Some(b.url.clone()) });
                    x
                })
                .collect();
            agg.hidden += before - pins.len();
            agg.total += pins.len();
            let bdir = root.join(omniget_core::core::tools::sanitize_name(&b.name));
            std::fs::create_dir_all(&bdir).map_err(err)?;
            let (files, out) = download_many(c.clone(), pins.clone(), &opts.download, &bdir, pid, &p).await;
            agg.downloaded += out.downloaded;
            agg.skipped += out.skipped;
            agg.files += out.files;
            agg.failed.extend(out.failed);
            let _ = write_outputs(&b.name, &bdir, &pins, &files, opts.metadata, opts.gallery).await;
            all_files.extend(files);
            all_pins.extend(pins);
        }
        if opts.include_created {
            let pins = c
                .collect(&Feed::UserCreated { username: username.clone() }, opts.limit, |n| report(&p, pid, "list", n as u64, None, Some("criados".into())))
                .await
                .unwrap_or_default();
            let cdir = root.join("_criados");
            let (files, out) = download_many(c.clone(), pins.clone(), &opts.download, &cdir, pid, &p).await;
            agg.total += pins.len();
            agg.downloaded += out.downloaded;
            agg.skipped += out.skipped;
            agg.files += out.files;
            agg.failed.extend(out.failed);
            all_files.extend(files);
            all_pins.extend(pins);
        }
        let (csv, json, html) = write_outputs(&agg.title, &root, &all_pins, &all_files, opts.metadata, opts.gallery).await;
        agg.csv_path = csv;
        agg.json_path = json;
        agg.html_path = html;
        report(&p, pid, "done", agg.downloaded as u64, Some(agg.total as u64), None);
        return Ok(agg);
    }

    let (title, pins, hidden) = match &target {
        Target::Board { user, slug } => {
            let b = c.board(user, slug).await.map_err(err)?;
            let pins = board_with_sections(&c, &b, opts.limit, pid, &p).await?;
            let before = pins.len();
            let pins: Vec<Pin> = pins.into_iter().filter(|x| keep(x, &opts.filters)).collect();
            (b.name.clone(), pins.clone(), before - pins.len())
        }
        _ => {
            let (title, pins, _, hidden) = list_pins(&c, &target, opts.limit, &opts.filters, pid, &p).await?;
            (title, pins, hidden)
        }
    };
    let (files, out) = download_many(c.clone(), pins.clone(), &opts.download, &root, pid, &p).await;
    let (csv_path, json_path, html_path) = write_outputs(&title, &root, &pins, &files, opts.metadata, opts.gallery).await;
    report(&p, pid, "done", out.downloaded as u64, Some(pins.len() as u64), None);
    Ok(BackupOut {
        title,
        dest: out.dest,
        total: pins.len(),
        hidden,
        downloaded: out.downloaded,
        skipped: out.skipped,
        failed: out.failed,
        files: out.files,
        csv_path,
        json_path,
        html_path,
        boards: 1,
    })
}

// ───────────────────────── duplicados ─────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct DupeGroupOut {
    pub kind: String,
    pub distance: u32,
    pub pins: Vec<Pin>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DupesOut {
    pub title: String,
    pub scanned: usize,
    pub groups: Vec<DupeGroupOut>,
    pub has_session: bool,
}

#[tauri::command]
pub async fn tool_pin_dupes(app: tauri::AppHandle, url: String, cookies: Option<String>, limit: Option<usize>, threshold: Option<u32>) -> Result<DupesOut, String> {
    let c = client(&cookies)?;
    let (target, _) = resolve(&c, &url).await?;
    let p = progress(&app);
    let pid = "pinterest:dupes";
    let pins: Vec<Pin> = match &target {
        Target::Board { user, slug } => {
            let b = c.board(user, slug).await.map_err(err)?;
            board_with_sections(&c, &b, limit.unwrap_or(0), pid, &p).await?
        }
        _ => list_pins(&c, &target, limit.unwrap_or(0), &Filters::default(), pid, &p).await?.1,
    };
    let title = match &target {
        Target::Board { slug, .. } => slug.clone(),
        _ => url.clone(),
    };
    let total = pins.len();
    let inputs: Vec<(String, Option<String>, Option<String>)> = pins
        .iter()
        .map(|pin| (pin.id.clone(), pin.image_signature.clone(), pin.thumb.clone().or_else(|| pin.image_large.as_ref().map(|m| m.url.clone()))))
        .collect();
    let jobs = inputs.into_iter().map(|(id, sig, thumb)| {
        let c = c.clone();
        async move {
            let hash = match thumb {
                Some(u) => match c.http().get(&u).send().await {
                    Ok(r) if r.status().is_success() => r.bytes().await.ok().and_then(|b| analysis::dhash(&b)),
                    _ => None,
                },
                None => None,
            };
            (id, sig, hash)
        }
    });
    let mut items: Vec<(String, Option<String>, Option<u64>)> = Vec::with_capacity(total);
    let mut stream = futures::stream::iter(jobs).buffer_unordered(8);
    while let Some(it) = stream.next().await {
        items.push(it);
        report(&p, pid, "hash", items.len() as u64, Some(total as u64), None);
    }
    let groups: Vec<DupeGroup> = analysis::group_dupes(&items, threshold.unwrap_or(6));
    let by_id: HashMap<&str, &Pin> = pins.iter().map(|x| (x.id.as_str(), x)).collect();
    let groups = groups
        .into_iter()
        .map(|g| DupeGroupOut { kind: g.kind, distance: g.distance, pins: g.ids.iter().filter_map(|id| by_id.get(id.as_str()).map(|x| (*x).clone())).collect() })
        .collect();
    report(&p, pid, "done", total as u64, Some(total as u64), None);
    Ok(DupesOut { title, scanned: total, groups, has_session: c.has_session() })
}

#[derive(Debug, Clone, Serialize)]
pub struct UnsaveOut {
    pub done: Vec<String>,
    pub failed: Vec<(String, String)>,
}

/// Desfaz saves seus (exige cookies de sessão). Sequencial de propósito.
#[tauri::command]
pub async fn tool_pin_unsave(ids: Vec<String>, cookies: Option<String>) -> Result<UnsaveOut, String> {
    let c = client(&cookies)?;
    if !c.has_session() {
        return Err("informe os cookies da sua sessao do Pinterest para desfazer saves".into());
    }
    let mut out = UnsaveOut { done: vec![], failed: vec![] };
    for id in ids {
        match c.unsave(&id).await {
            Ok(()) => out.done.push(id),
            Err(e) => out.failed.push((id, e.to_string())),
        }
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    }
    Ok(out)
}

// ───────────────────────── paleta ─────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct PaletteOut {
    pub title: String,
    pub pins_used: usize,
    /// k-means sobre pixels dos thumbnails
    pub swatches: Vec<Swatch>,
    /// as `dominant_color` que o Pinterest calcula, agrupadas
    pub dominant: Vec<Swatch>,
}

#[tauri::command]
pub async fn tool_pin_palette(app: tauri::AppHandle, url: String, cookies: Option<String>, limit: Option<usize>, colors: Option<usize>, skip_extremes: Option<bool>) -> Result<PaletteOut, String> {
    let c = client(&cookies)?;
    let (target, _) = resolve(&c, &url).await?;
    let p = progress(&app);
    let pid = "pinterest:palette";
    let (title, pins) = match &target {
        Target::Pin { id } => {
            let pin = c.pin(id).await.map_err(err)?;
            (pin.title.clone(), vec![pin])
        }
        _ => {
            let (t, pins, _, _) = list_pins(&c, &target, limit.unwrap_or(60), &Filters::default(), pid, &p).await?;
            (t, pins)
        }
    };
    let k = colors.unwrap_or(8).clamp(2, 24);
    let skip = skip_extremes.unwrap_or(true);
    let total = pins.len();
    let inputs: Vec<Option<String>> = pins
        .iter()
        .map(|pin| if total == 1 { pin.image_large.as_ref().map(|m| m.url.clone()).or_else(|| pin.thumb.clone()) } else { pin.thumb.clone() })
        .collect();
    let jobs = inputs.into_iter().map(|u| {
        let c = c.clone();
        async move {
            match u {
                Some(u) => match c.http().get(&u).send().await {
                    Ok(r) if r.status().is_success() => r.bytes().await.ok().map(|b| analysis::sample_pixels(&b, if total == 1 { 2304 } else { 400 }, skip)).unwrap_or_default(),
                    _ => Vec::new(),
                },
                None => Vec::new(),
            }
        }
    });
    let mut samples: Vec<[u8; 3]> = Vec::new();
    let mut done = 0usize;
    let mut stream = futures::stream::iter(jobs).buffer_unordered(8);
    while let Some(s) = stream.next().await {
        samples.extend(s);
        done += 1;
        report(&p, pid, "sample", done as u64, Some(total as u64), None);
    }
    let swatches = analysis::kmeans_palette(&samples, k);
    let dom: Vec<[u8; 3]> = pins.iter().filter_map(|x| x.dominant_color.as_deref().and_then(analysis::parse_hex)).collect();
    let dominant = analysis::kmeans_palette(&dom, k.min(dom.len().max(1)));
    report(&p, pid, "done", total as u64, Some(total as u64), None);
    Ok(PaletteOut { title, pins_used: total, swatches, dominant })
}

// ───────────────────────── exportar ─────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct ExportOptions {
    pub url: String,
    pub dest: String,
    /// "html" | "pdf" | "csv" | "json"
    pub format: String,
    #[serde(default)]
    pub cookies: Option<String>,
    #[serde(default)]
    pub limit: usize,
    #[serde(default)]
    pub filters: Filters,
    /// html: baixar as imagens para a pasta (galeria 100% offline)
    #[serde(default)]
    pub offline: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportOut {
    pub title: String,
    pub path: String,
    pub pins: usize,
}

#[tauri::command]
pub async fn tool_pin_export(app: tauri::AppHandle, opts: ExportOptions) -> Result<ExportOut, String> {
    let c = client(&opts.cookies)?;
    let (target, _) = resolve(&c, &opts.url).await?;
    let p = progress(&app);
    let pid = "pinterest:export";
    let (title, pins) = match &target {
        Target::Board { user, slug } => {
            let b = c.board(user, slug).await.map_err(err)?;
            let pins = board_with_sections(&c, &b, opts.limit, pid, &p).await?;
            (b.name.clone(), pins.into_iter().filter(|x| keep(x, &opts.filters)).collect::<Vec<_>>())
        }
        _ => {
            let (t, pins, _, _) = list_pins(&c, &target, opts.limit, &opts.filters, pid, &p).await?;
            (t, pins)
        }
    };
    if pins.is_empty() {
        return Err("nenhum pin para exportar".into());
    }
    let dest = PathBuf::from(&opts.dest);
    std::fs::create_dir_all(&dest).map_err(err)?;
    let stem = omniget_core::core::tools::sanitize_name(&title);
    let path = match opts.format.as_str() {
        "csv" => {
            let path = dest.join(format!("{}.csv", stem));
            tokio::fs::write(&path, export::csv(&pins, &HashMap::new())).await.map_err(err)?;
            path
        }
        "json" => {
            let path = dest.join(format!("{}.json", stem));
            tokio::fs::write(&path, serde_json::to_string_pretty(&pins).map_err(err)?).await.map_err(err)?;
            path
        }
        "pdf" => {
            let work = omniget_core::core::tools::temp_dir().join(format!("pinterest-pdf-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&work).map_err(err)?;
            let mut images: Vec<Vec<u8>> = Vec::new();
            for (i, pin) in pins.iter().enumerate() {
                report(&p, pid, "download", i as u64, Some(pins.len() as u64), Some(pin.title.clone()));
                let Some(m) = pin.image.as_ref().or(pin.image_large.as_ref()) else { continue };
                let Ok((bytes, _)) = media::fetch_image(&c, &m.url).await else { continue };
                let jpeg = omniget_core::core::tools::pinterest::export::to_jpeg(bytes, &work, i).await.map_err(err)?;
                images.push(jpeg);
            }
            let _ = std::fs::remove_dir_all(&work);
            if images.is_empty() {
                return Err("nenhuma imagem baixada para o PDF".into());
            }
            let pdf = omniget_core::core::tools::jpeg_pdf::build_pdf(&images).map_err(err)?;
            let path = dest.join(format!("{}.pdf", stem));
            tokio::fs::write(&path, pdf).await.map_err(err)?;
            path
        }
        _ => {
            let folder = dest.join(&stem);
            std::fs::create_dir_all(&folder).map_err(err)?;
            let mut files: HashMap<String, Vec<String>> = HashMap::new();
            if opts.offline {
                let dl = DownloadOptions {
                    dest: folder.to_string_lossy().to_string(),
                    images: true,
                    videos: true,
                    convert_webp: false,
                    naming: "id".into(),
                    sidecar: false,
                    skip_downloaded: true,
                    section_folders: true,
                };
                let (f, _) = download_many(c.clone(), pins.clone(), &dl, &folder, pid, &p).await;
                files = f;
            }
            let sub = format!("{} pins · OmniGet · {}", pins.len(), chrono::Local::now().format("%Y-%m-%d %H:%M"));
            let html = export::html_gallery(&title, &sub, &pins, &files, &folder.to_string_lossy());
            let path = folder.join("index.html");
            tokio::fs::write(&path, html).await.map_err(err)?;
            path
        }
    };
    report(&p, pid, "done", pins.len() as u64, Some(pins.len() as u64), None);
    Ok(ExportOut { title, path: path.to_string_lossy().to_string(), pins: pins.len() })
}

// ───────────────────────── palavras-chave ─────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct KeywordsOut {
    pub term: String,
    pub suggestions: Vec<String>,
    /// refinamentos que o Pinterest mostra na barra da busca
    pub guides: Vec<String>,
    /// hashtags e palavras mais comuns nos títulos/descrições dos primeiros resultados
    pub common: Vec<(String, usize)>,
    pub sample: usize,
}

#[tauri::command]
pub async fn tool_pin_keywords(term: String, cookies: Option<String>) -> Result<KeywordsOut, String> {
    let c = client(&cookies)?;
    let term = term.trim().to_string();
    if term.is_empty() {
        return Err("digite um termo".into());
    }
    let suggestions = c.typeahead(&term).await.unwrap_or_default();
    let feed = Feed::Search { query: term.clone(), scope: "pins".into() };
    let (pins, _, guides) = c.feed_page(&feed, None, 50).await.map_err(err)?;
    let stop: &[&str] = &["the", "and", "for", "with", "your", "this", "that", "from", "you", "are", "how", "our", "para", "com", "que", "uma", "dos", "das", "los", "las", "por", "una", "more", "ideas", "idea", "pin", "pinterest", "http", "https", "www", "com"];
    let mut counts: HashMap<String, usize> = HashMap::new();
    for p in &pins {
        let text = format!("{} {} {}", p.title, p.description, p.alt_text).to_lowercase();
        let mut seen: Vec<String> = Vec::new();
        for w in text.split(|c: char| !(c.is_alphanumeric() || c == '#' || c == '-')) {
            let w = w.trim_matches('-');
            if w.len() < 3 || stop.contains(&w) || w.chars().all(|c| c.is_ascii_digit()) || seen.iter().any(|s| s == w) {
                continue;
            }
            seen.push(w.to_string());
            *counts.entry(w.to_string()).or_insert(0) += 1;
        }
    }
    let mut common: Vec<(String, usize)> = counts.into_iter().filter(|(_, n)| *n >= 2).collect();
    common.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    common.truncate(40);
    Ok(KeywordsOut { term, suggestions, guides, common, sample: pins.len() })
}

// ───────────────────────── fonte ─────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct LinkCheck {
    pub url: String,
    pub status: Option<u16>,
    pub final_url: Option<String>,
    pub ok: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceOut {
    pub pin: Pin,
    pub link: Option<LinkCheck>,
    pub wayback: Option<String>,
    /// [nome, url]
    pub reverse: Vec<(String, String)>,
    pub resolved_url: String,
}

async fn check_link(url: &str) -> LinkCheck {
    let Ok(client) = omniget_core::core::tools::client() else {
        return LinkCheck { url: url.into(), status: None, final_url: None, ok: false, error: Some("cliente".into()) };
    };
    let timeout = std::time::Duration::from_secs(15);
    let r = match client.head(url).timeout(timeout).send().await {
        Ok(r) if r.status().as_u16() != 405 && r.status().as_u16() != 403 => Ok(r),
        _ => client.get(url).timeout(timeout).send().await,
    };
    match r {
        Ok(r) => {
            let st = r.status().as_u16();
            LinkCheck { url: url.into(), status: Some(st), final_url: Some(r.url().to_string()), ok: (200..400).contains(&st), error: None }
        }
        Err(e) => LinkCheck { url: url.into(), status: None, final_url: None, ok: false, error: Some(e.to_string()) },
    }
}

async fn wayback(url: &str) -> Option<String> {
    let client = omniget_core::core::tools::client().ok()?;
    let r = client
        .get("https://archive.org/wayback/available")
        .query(&[("url", url)])
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .ok()?;
    let v: serde_json::Value = r.json().await.ok()?;
    v["archived_snapshots"]["closest"]["url"].as_str().map(|s| s.to_string())
}

#[tauri::command]
pub async fn tool_pin_source(url: String, cookies: Option<String>) -> Result<SourceOut, String> {
    let c = client(&cookies)?;
    let (target, resolved_url) = resolve(&c, &url).await?;
    let Target::Pin { id } = &target else {
        return Err("cole o link de um pin".into());
    };
    let pin = c.pin(id).await.map_err(err)?;
    let img = pin.image.as_ref().or(pin.image_large.as_ref()).map(|m| m.url.clone()).unwrap_or_default();
    let enc = urlencoding::encode(&img).to_string();
    let reverse = if img.is_empty() {
        vec![]
    } else {
        vec![
            ("Google Lens".to_string(), format!("https://lens.google.com/uploadbyurl?url={}", enc)),
            ("TinEye".to_string(), format!("https://tineye.com/search?url={}", enc)),
            ("Yandex".to_string(), format!("https://yandex.com/images/search?rpt=imageview&url={}", enc)),
            ("Bing".to_string(), format!("https://www.bing.com/images/search?view=detailv2&iss=sbi&q=imgurl:{}", enc)),
            ("SauceNAO".to_string(), format!("https://saucenao.com/search.php?url={}", enc)),
        ]
    };
    let link_url = pin.link.clone().or_else(|| pin.rich.as_ref().and_then(|r| r.url.clone()));
    let (link, wb) = match &link_url {
        Some(l) => {
            let chk = check_link(l).await;
            let wb = if chk.ok { None } else { wayback(l).await };
            (Some(chk), wb)
        }
        None => (None, None),
    };
    Ok(SourceOut { pin, link, wayback: wb, reverse, resolved_url })
}

/// Expande pin.it e devolve a URL final + alvo reconhecido.
#[tauri::command]
pub async fn tool_pin_expand(url: String) -> Result<(String, Target), String> {
    let c = client(&None)?;
    let (t, resolved) = resolve(&c, &url).await?;
    Ok((resolved, t))
}
