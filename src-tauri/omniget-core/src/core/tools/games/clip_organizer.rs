//! Clipes e prints do PC, que cada gravador nomeia do seu jeito:
//!
//! - Xbox Game Bar  `Hades 2026-09-09 21-14-03-123.mp4`
//! - NVIDIA ShadowPlay  `Hades/Hades 2026.09.09 - 21.14.03.01.DVR.mp4`
//! - OBS  `2026-09-09 21-14-03.mkv` (sem jogo no nome)
//! - Steam  `1145360_20260909211403_1.jpg` em `760/remote/<appid>/screenshots`
//!
//! Extrai jogo e data de cada padrão, agrupa em pastas, tira duplicado e
//! entrega o resumo. O appid da Steam vira nome lendo o `appmanifest` local —
//! nada de rede. Clipe grande demais vai para o compressor de vídeo que a
//! seção Video já usa, em vez de um segundo ffmpeg escrito à mão.

use std::path::{Path, PathBuf};

use chrono::NaiveDateTime;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};

use super::steam;
use super::{
    is_clip_ext, is_image_ext, place_file, sha256_file, summarize, unique_path, DedupeIndex,
    GameSummary, ImportItem,
};
use crate::core::tools::{report, sanitize_name, ProgressFn};

const ID: &str = "clip-organizer";

/// Pastas cujo nome não diz nada sobre o jogo — não viram rótulo.
const GENERIC_DIRS: &[&str] = &[
    "captures",
    "videos",
    "video",
    "screenshots",
    "screenshot",
    "clips",
    "clipes",
    "recordings",
    "gravacoes",
    "remote",
    "desktop",
    "movies",
    "obs",
    "downloads",
    "documents",
];

static GAMEBAR: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^(?P<game>.+) (\d{4})-(\d{2})-(\d{2}) (\d{2})-(\d{2})-(\d{2})(?:-(\d{2,3}))?$")
        .expect("regex do Game Bar")
});
static SHADOWPLAY: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"^(?P<game>.+) (\d{4})\.(\d{2})\.(\d{2}) - (\d{2})\.(\d{2})\.(\d{2})\.(\d{2})(?:\.DVR)?$",
    )
    .expect("regex do ShadowPlay")
});
static OBS: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^(\d{4})-(\d{2})-(\d{2}) (\d{2})-(\d{2})-(\d{2})$").expect("regex do OBS")
});
static STEAM: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^(\d{2,10})_(\d{14})_(\d{1,3})$").expect("regex da Steam"));

fn ymd_hms(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> Option<NaiveDateTime> {
    chrono::NaiveDate::from_ymd_opt(y, mo, d)?.and_hms_opt(h, mi, s)
}

fn cap_num<T: std::str::FromStr>(c: &regex::Captures, i: usize) -> Option<T> {
    c.get(i)?.as_str().parse::<T>().ok()
}

/// O que o nome do arquivo entrega.
#[derive(Debug, Clone, PartialEq)]
pub struct ClipName {
    pub game: Option<String>,
    pub taken: NaiveDateTime,
    /// "gamebar" | "shadowplay" | "obs" | "steam"
    pub source: String,
    pub app_id: Option<u32>,
    pub ext: String,
    pub is_clip: bool,
}

pub fn parse_clip_name(file_name: &str) -> Option<ClipName> {
    let (stem, ext) = file_name.rsplit_once('.')?;
    let ext = ext.to_ascii_lowercase();
    if !is_clip_ext(&ext) && !is_image_ext(&ext) {
        return None;
    }
    let is_clip = is_clip_ext(&ext);
    let mk = |game: Option<String>, taken: NaiveDateTime, source: &str, app_id: Option<u32>| {
        Some(ClipName {
            game: game.map(|g| g.trim().to_string()).filter(|g| !g.is_empty()),
            taken,
            source: source.to_string(),
            app_id,
            ext: ext.clone(),
            is_clip,
        })
    };

    if let Some(c) = SHADOWPLAY.captures(stem) {
        let taken = ymd_hms(
            cap_num(&c, 2)?,
            cap_num(&c, 3)?,
            cap_num(&c, 4)?,
            cap_num(&c, 5)?,
            cap_num(&c, 6)?,
            cap_num(&c, 7)?,
        )?;
        return mk(
            c.name("game").map(|m| m.as_str().to_string()),
            taken,
            "shadowplay",
            None,
        );
    }
    if let Some(c) = GAMEBAR.captures(stem) {
        let taken = ymd_hms(
            cap_num(&c, 2)?,
            cap_num(&c, 3)?,
            cap_num(&c, 4)?,
            cap_num(&c, 5)?,
            cap_num(&c, 6)?,
            cap_num(&c, 7)?,
        )?;
        return mk(
            c.name("game").map(|m| m.as_str().to_string()),
            taken,
            "gamebar",
            None,
        );
    }
    if let Some(c) = OBS.captures(stem) {
        let taken = ymd_hms(
            cap_num(&c, 1)?,
            cap_num(&c, 2)?,
            cap_num(&c, 3)?,
            cap_num(&c, 4)?,
            cap_num(&c, 5)?,
            cap_num(&c, 6)?,
        )?;
        return mk(None, taken, "obs", None);
    }
    if let Some(c) = STEAM.captures(stem) {
        let app_id: u32 = cap_num(&c, 1)?;
        let stamp = c.get(2)?.as_str();
        let n = |a: usize, b: usize| stamp[a..b].parse::<u32>().ok();
        let taken = ymd_hms(
            stamp[0..4].parse::<i32>().ok()?,
            n(4, 6)?,
            n(6, 8)?,
            n(8, 10)?,
            n(10, 12)?,
            n(12, 14)?,
        )?;
        return mk(None, taken, "steam", Some(app_id));
    }
    None
}

/// Rótulo do jogo: o nome que veio no arquivo, senão o `appmanifest` da
/// Steam, senão a pasta-mãe (o ShadowPlay guarda por jogo), senão o fallback.
pub fn game_label(
    path: &Path,
    parsed: &ClipName,
    apps: &[steam::SteamApp],
    fallback: &str,
) -> (String, String, bool) {
    if let Some(g) = parsed.game.as_ref() {
        return (g.clone(), String::new(), true);
    }
    if let Some(id) = parsed.app_id {
        if let Some(name) = steam::name_for_app(apps, id) {
            return (name, id.to_string(), true);
        }
        return (format!("Steam {}", id), id.to_string(), false);
    }
    let parent = path
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    if !parent.is_empty() && !GENERIC_DIRS.contains(&parent.to_lowercase().as_str()) {
        return (parent, String::new(), true);
    }
    (fallback.to_string(), String::new(), false)
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClipOptions {
    /// Pastas de captura para varrer.
    #[serde(default)]
    pub dirs: Vec<String>,
    /// Inclui as pastas de screenshot do cliente Steam.
    #[serde(default)]
    pub include_steam: bool,
    /// Pastas `steamapps` extras, para resolver appid → nome.
    #[serde(default)]
    pub steam_dirs: Vec<String>,
    /// Vazio = só analisar, sem mexer em arquivo.
    #[serde(default)]
    pub dest: String,
    /// "game" | "game-year" | "year-month" | "flat"
    #[serde(default = "default_layout")]
    pub layout: String,
    /// "scan" | "copy" | "move"
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default = "default_true")]
    pub dedupe: bool,
    #[serde(default)]
    pub dry_run: bool,
    /// Acima disso o clipe vai para o compressor; 0 desliga.
    #[serde(default)]
    pub recompress_over_mb: f64,
    #[serde(default = "default_target")]
    pub target_mb: f64,
    /// Rótulo do que não tem jogo no nome (o OBS, por exemplo).
    #[serde(default)]
    pub fallback_game: String,
}

fn default_layout() -> String {
    "game".into()
}
fn default_mode() -> String {
    "scan".into()
}
fn default_true() -> bool {
    true
}
fn default_target() -> f64 {
    25.0
}

#[derive(Debug, Clone, Serialize)]
pub struct BigClip {
    pub path: String,
    pub game: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClipResult {
    pub items: Vec<ImportItem>,
    pub games: Vec<GameSummary>,
    pub found: u64,
    pub imported: u64,
    pub skipped: u64,
    pub failed: u64,
    pub bytes_total: u64,
    /// Os dez maiores arquivos, para o usuário decidir o que recomprimir.
    pub largest: Vec<BigClip>,
    pub recompressed: Vec<crate::core::tools::video_compress::CompressItem>,
    pub mode: String,
    pub dry_run: bool,
}

pub fn dest_dir(root: &Path, layout: &str, game: &str, taken: &NaiveDateTime) -> PathBuf {
    let folder = sanitize_name(game);
    match layout {
        "flat" => root.to_path_buf(),
        "year-month" => root
            .join(taken.format("%Y").to_string())
            .join(taken.format("%m").to_string()),
        "game-year" => root.join(folder).join(taken.format("%Y").to_string()),
        _ => root.join(folder),
    }
}

pub fn dest_stem(game: &str, taken: &NaiveDateTime) -> String {
    sanitize_name(&format!("{} {}", game, taken.format("%Y-%m-%d %H-%M-%S")))
}

fn collect(dirs: &[PathBuf]) -> Vec<(PathBuf, ClipName)> {
    let mut out: Vec<(PathBuf, ClipName)> = Vec::new();
    for dir in dirs {
        for entry in walkdir::WalkDir::new(dir)
            .max_depth(6)
            .into_iter()
            .filter_map(Result::ok)
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let Some(parsed) = parse_clip_name(&name) else {
                continue;
            };
            let path = entry.path().to_path_buf();
            if !out.iter().any(|(p, _)| p == &path) {
                out.push((path, parsed));
            }
        }
    }
    out.sort_by(|a, b| a.1.taken.cmp(&b.1.taken).then_with(|| a.0.cmp(&b.0)));
    out
}

pub async fn run(opts: ClipOptions, progress: ProgressFn) -> anyhow::Result<ClipResult> {
    let mut dirs: Vec<PathBuf> = opts
        .dirs
        .iter()
        .map(|d| PathBuf::from(d.trim()))
        .filter(|d| d.is_dir())
        .collect();
    if opts.include_steam {
        dirs.extend(steam::screenshot_dirs());
    }
    dirs.sort();
    dirs.dedup();
    if dirs.is_empty() {
        anyhow::bail!("escolha ao menos uma pasta de capturas");
    }

    let apps = steam::scan_apps(&steam::steam_libraries(&opts.steam_dirs));
    let fallback = if opts.fallback_game.trim().is_empty() {
        "Outros".to_string()
    } else {
        opts.fallback_game.trim().to_string()
    };

    let organizing = opts.mode == "copy" || opts.mode == "move";
    let dest_root = PathBuf::from(opts.dest.trim());
    if organizing && opts.dest.trim().is_empty() {
        anyhow::bail!("escolha a pasta de destino para organizar");
    }
    let move_it = opts.mode == "move";

    report(&progress, ID, "progress", 0, None, None);
    let found = collect(&dirs);
    let total = found.len() as u64;

    // No modo análise não há índice em disco: o `seen` abaixo já acha cópia
    // repetida dentro da rodada sem escrever nada.
    let mut index = if opts.dedupe && organizing {
        DedupeIndex::load(&dest_root)
    } else {
        DedupeIndex::disabled()
    };
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    let mut items: Vec<ImportItem> = Vec::new();
    let (mut imported, mut skipped, mut failed, mut bytes_total) = (0u64, 0u64, 0u64, 0u64);

    for (i, (path, parsed)) in found.iter().enumerate() {
        report(
            &progress,
            ID,
            "progress",
            i as u64,
            Some(total),
            Some(path.to_string_lossy().to_string()),
        );
        let (game, game_id, known) = game_label(path, parsed, &apps, &fallback);
        let bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        bytes_total += bytes;
        let mut item = ImportItem {
            source: path.to_string_lossy().to_string(),
            dest: None,
            game: game.clone(),
            game_id,
            known_game: known,
            taken_at: parsed.taken.format("%Y-%m-%d %H:%M:%S").to_string(),
            bytes,
            kind: if parsed.is_clip {
                "clip".into()
            } else {
                "image".into()
            },
            duration_s: None,
            width: None,
            height: None,
            extra: Some(parsed.source.clone()),
            status: if organizing {
                "imported".into()
            } else {
                "scanned".into()
            },
            reason: None,
        };

        if opts.dedupe {
            match sha256_file(path) {
                Ok(hash) => {
                    if index.contains(&hash) || !seen.insert(hash.clone()) {
                        item.status = "skipped".into();
                        item.reason = Some("duplicate".into());
                        skipped += 1;
                        items.push(item);
                        continue;
                    }
                    if organizing && !opts.dry_run {
                        index.insert(hash);
                    }
                }
                Err(e) => {
                    item.status = "error".into();
                    item.reason = Some(e.to_string());
                    failed += 1;
                    items.push(item);
                    continue;
                }
            }
        }

        if !organizing {
            items.push(item);
            continue;
        }

        let dir = dest_dir(&dest_root, &opts.layout, &game, &parsed.taken);
        let stem = dest_stem(&game, &parsed.taken);
        if opts.dry_run {
            item.dest = Some(
                dir.join(format!("{}.{}", stem, parsed.ext))
                    .to_string_lossy()
                    .to_string(),
            );
            imported += 1;
            items.push(item);
            continue;
        }
        if let Err(e) = std::fs::create_dir_all(&dir) {
            item.status = "error".into();
            item.reason = Some(e.to_string());
            failed += 1;
            items.push(item);
            continue;
        }
        let target = unique_path(&dir, &stem, &parsed.ext);
        match place_file(path, &target, move_it) {
            Ok(()) => {
                item.dest = Some(target.to_string_lossy().to_string());
                imported += 1;
            }
            Err(e) => {
                item.status = "error".into();
                item.reason = Some(e.to_string());
                failed += 1;
            }
        }
        items.push(item);
    }

    if organizing && !opts.dry_run {
        if let Err(e) = index.save() {
            tracing::warn!("[{}] índice de duplicados: {}", ID, e);
        }
    }

    let mut largest: Vec<BigClip> = items
        .iter()
        .filter(|i| i.kind == "clip" && i.status != "error" && i.status != "skipped")
        .map(|i| BigClip {
            path: i.dest.clone().unwrap_or_else(|| i.source.clone()),
            game: i.game.clone(),
            bytes: i.bytes,
        })
        .collect();
    largest.sort_by_key(|c| std::cmp::Reverse(c.bytes));
    largest.truncate(10);

    let mut recompressed = Vec::new();
    let limit = (opts.recompress_over_mb.max(0.0) * 1024.0 * 1024.0) as u64;
    if limit > 0 && !opts.dry_run {
        let inputs: Vec<String> = items
            .iter()
            .filter(|i| i.kind == "clip" && i.status != "error" && i.status != "skipped")
            .filter(|i| i.bytes > limit)
            .map(|i| i.dest.clone().unwrap_or_else(|| i.source.clone()))
            .collect();
        if !inputs.is_empty() {
            let out = crate::core::tools::video_compress::run(
                crate::core::tools::video_compress::CompressOptions {
                    inputs,
                    target_mb: opts.target_mb.max(1.0),
                    codec: "h264".into(),
                    audio_kbps: 0,
                    max_height: 1080,
                    fps: 0,
                    preset: "veryfast".into(),
                    output_dir: String::new(),
                    suffix: "-menor".into(),
                },
                progress.clone(),
            )
            .await?;
            recompressed = out.items;
        }
    }

    let games = summarize(&items);
    report(&progress, ID, "done", total, Some(total), None);
    Ok(ClipResult {
        items,
        games,
        found: total,
        imported,
        skipped,
        failed,
        bytes_total,
        largest,
        recompressed,
        mode: opts.mode.clone(),
        dry_run: opts.dry_run,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir(tag: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("omniget-clips-{}-{}", tag, uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).expect("criar tempdir");
        d
    }

    #[test]
    fn game_bar_name_with_and_without_milliseconds() {
        let c = parse_clip_name("Hollow Knight Silksong 2026-09-09 21-14-03-123.mp4")
            .expect("nome do Game Bar");
        assert_eq!(c.source, "gamebar");
        assert_eq!(c.game.as_deref(), Some("Hollow Knight Silksong"));
        assert_eq!(
            c.taken.format("%Y-%m-%d %H:%M:%S").to_string(),
            "2026-09-09 21:14:03"
        );
        assert!(c.is_clip);

        let p = parse_clip_name("Forza Horizon 5 2026-01-05 08-00-00.png").expect("print");
        assert_eq!(p.source, "gamebar");
        assert_eq!(p.game.as_deref(), Some("Forza Horizon 5"));
        assert!(!p.is_clip);
    }

    #[test]
    fn shadowplay_name_with_and_without_dvr() {
        let c = parse_clip_name("Hades 2026.09.09 - 21.14.03.01.DVR.mp4").expect("ShadowPlay");
        assert_eq!(c.source, "shadowplay");
        assert_eq!(c.game.as_deref(), Some("Hades"));
        assert_eq!(c.taken.format("%H:%M:%S").to_string(), "21:14:03");

        let c = parse_clip_name("Cyberpunk 2077 2026.09.09 - 21.14.03.02.mp4").expect("ShadowPlay");
        assert_eq!(c.source, "shadowplay");
        assert_eq!(c.game.as_deref(), Some("Cyberpunk 2077"));
    }

    #[test]
    fn obs_name_has_no_game() {
        let c = parse_clip_name("2026-09-09 21-14-03.mkv").expect("OBS");
        assert_eq!(c.source, "obs");
        assert!(c.game.is_none());
        assert_eq!(c.ext, "mkv");
    }

    #[test]
    fn steam_screenshot_name_carries_the_appid() {
        let c = parse_clip_name("1145360_20260909211403_1.jpg").expect("Steam");
        assert_eq!(c.source, "steam");
        assert_eq!(c.app_id, Some(1_145_360));
        assert!(!c.is_clip);
        assert_eq!(
            c.taken.format("%Y-%m-%d %H:%M:%S").to_string(),
            "2026-09-09 21:14:03"
        );
    }

    #[test]
    fn refuses_what_is_not_a_capture() {
        assert!(parse_clip_name("relatorio.pdf").is_none());
        assert!(parse_clip_name("ferias 2026.mp4").is_none());
        // Data impossível.
        assert!(parse_clip_name("Hades 2026-13-09 21-14-03-123.mp4").is_none());
        assert!(parse_clip_name("2026-09-31 21-14-03.mkv").is_none());
        // Steam sem o contador.
        assert!(parse_clip_name("1145360_20260909211403.jpg").is_none());
        // Carimbo do Switch não é do PC.
        assert!(parse_clip_name("2026090921140300-8AEDFF741E2D23FBED39474178692DAF.jpg").is_none());
        assert!(parse_clip_name("sem-ponto").is_none());
    }

    #[test]
    fn shadowplay_wins_over_game_bar_when_both_could_match() {
        // O ponto separa a data no ShadowPlay; o Game Bar usa hífen. Um nome
        // com os dois formatos não pode cair no ramo errado.
        let c = parse_clip_name("Jogo 2026-09-09 21-14-03 2026.09.09 - 21.14.03.01.DVR.mp4")
            .expect("ShadowPlay");
        assert_eq!(c.source, "shadowplay");
        assert_eq!(c.game.as_deref(), Some("Jogo 2026-09-09 21-14-03"));
    }

    #[test]
    fn label_falls_back_to_the_parent_folder_then_to_the_default() {
        let parsed = parse_clip_name("2026-09-09 21-14-03.mkv").expect("OBS");
        let (game, _, known) = game_label(
            Path::new("/home/eu/Videos/Elden Ring/2026-09-09 21-14-03.mkv"),
            &parsed,
            &[],
            "Outros",
        );
        assert_eq!(game, "Elden Ring");
        assert!(known);

        let (game, _, known) = game_label(
            Path::new("/home/eu/Videos/Captures/2026-09-09 21-14-03.mkv"),
            &parsed,
            &[],
            "Outros",
        );
        assert_eq!(game, "Outros");
        assert!(!known);
    }

    #[test]
    fn label_resolves_the_appid_from_the_local_manifest() {
        let parsed = parse_clip_name("570_20260909211403_1.jpg").expect("Steam");
        let apps = vec![steam::SteamApp {
            app_id: 570,
            name: "Dota 2".into(),
            install_dir: String::new(),
            size_on_disk: 0,
            library: String::new(),
        }];
        let (game, id, known) = game_label(
            Path::new("/x/570_20260909211403_1.jpg"),
            &parsed,
            &apps,
            "Outros",
        );
        assert_eq!(game, "Dota 2");
        assert_eq!(id, "570");
        assert!(known);

        // Sem manifesto, o appid fica visível em vez de virar nome inventado.
        let (game, id, known) = game_label(
            Path::new("/x/570_20260909211403_1.jpg"),
            &parsed,
            &[],
            "Outros",
        );
        assert_eq!(game, "Steam 570");
        assert_eq!(id, "570");
        assert!(!known);
    }

    #[tokio::test]
    async fn scan_groups_by_game_without_touching_the_files() {
        let base = tempdir("scan");
        let caps = base.join("Captures");
        std::fs::create_dir_all(&caps).expect("criar");
        std::fs::write(
            caps.join("Hades 2026-09-09 21-14-03-100.mp4"),
            b"clipe-hades-1",
        )
        .expect("w");
        std::fs::write(
            caps.join("Hades 2026-09-09 21-20-03-100.mp4"),
            b"clipe-hades-2-maior",
        )
        .expect("w");
        std::fs::write(caps.join("Celeste 2026-09-08 10-00-00-000.mp4"), b"c").expect("w");
        std::fs::write(caps.join("naoconta.txt"), b"x").expect("w");
        let r = run(
            ClipOptions {
                dirs: vec![caps.to_string_lossy().to_string()],
                include_steam: false,
                steam_dirs: Vec::new(),
                dest: String::new(),
                layout: "game".into(),
                mode: "scan".into(),
                dedupe: true,
                dry_run: false,
                recompress_over_mb: 0.0,
                target_mb: 25.0,
                fallback_game: "Outros".into(),
            },
            crate::core::tools::noop_progress(),
        )
        .await
        .expect("analisar");
        assert_eq!(r.found, 3);
        assert_eq!(r.imported, 0);
        assert_eq!(r.games.len(), 2);
        assert_eq!(r.games[0].game, "Hades");
        assert_eq!(r.games[0].clips, 2);
        assert_eq!(r.largest.len(), 3);
        assert!(r.largest[0].bytes >= r.largest[1].bytes);
        assert!(caps.join("Hades 2026-09-09 21-14-03-100.mp4").exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[tokio::test]
    async fn organize_moves_into_folders_by_game_and_skips_duplicates() {
        let base = tempdir("org");
        let caps = base.join("Captures");
        let shadow = base.join("ShadowPlay").join("Elden Ring");
        std::fs::create_dir_all(&caps).expect("criar");
        std::fs::create_dir_all(&shadow).expect("criar");
        std::fs::write(caps.join("Hades 2026-09-09 21-14-03-100.mp4"), b"hades").expect("w");
        // Mesmo conteúdo com outro nome: entra uma vez só.
        std::fs::write(caps.join("Hades 2026-09-09 21-15-03-100.mp4"), b"hades").expect("w");
        std::fs::write(
            shadow.join("Elden Ring 2026.09.09 - 22.00.00.01.DVR.mp4"),
            b"elden",
        )
        .expect("w");
        let lib = base.join("lib");
        let r = run(
            ClipOptions {
                dirs: vec![
                    caps.to_string_lossy().to_string(),
                    shadow.to_string_lossy().to_string(),
                ],
                include_steam: false,
                steam_dirs: Vec::new(),
                dest: lib.to_string_lossy().to_string(),
                layout: "game-year".into(),
                mode: "move".into(),
                dedupe: true,
                dry_run: false,
                recompress_over_mb: 0.0,
                target_mb: 25.0,
                fallback_game: "Outros".into(),
            },
            crate::core::tools::noop_progress(),
        )
        .await
        .expect("organizar");
        assert_eq!(r.found, 3);
        assert_eq!(r.imported, 2);
        assert_eq!(r.skipped, 1);
        assert!(lib
            .join("Hades")
            .join("2026")
            .join("Hades 2026-09-09 21-14-03.mp4")
            .exists());
        assert!(lib
            .join("Elden Ring")
            .join("2026")
            .join("Elden Ring 2026-09-09 22-00-00.mp4")
            .exists());
        assert!(!caps.join("Hades 2026-09-09 21-14-03-100.mp4").exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[tokio::test]
    async fn refuses_when_there_is_nothing_to_scan() {
        let r = run(
            ClipOptions {
                dirs: vec!["/pasta/que/nao/existe".into()],
                include_steam: false,
                steam_dirs: Vec::new(),
                dest: String::new(),
                layout: "game".into(),
                mode: "scan".into(),
                dedupe: false,
                dry_run: false,
                recompress_over_mb: 0.0,
                target_mb: 25.0,
                fallback_game: String::new(),
            },
            crate::core::tools::noop_progress(),
        )
        .await;
        assert!(r.is_err());
    }
}
