//! Álbum do Nintendo Switch: `Nintendo/Album/<ano>/<mês>/<dia>/` cheio de
//! `20260909211403000-F1C11A22FAEE3B82F21B330E1B786A39.jpg`.
//!
//! Os 16 primeiros dígitos são a data local da captura (os dois últimos são um
//! contador dentro do mesmo segundo) e os 32 hex identificam o jogo — é um
//! hash do title ID, o mesmo para todo mundo que jogou o mesmo cartucho.
//! A tabela `switch-games.tsv` vem do projeto nxshot (MIT, pxdl/nxshot), que
//! coleta esses hashes da comunidade; hash fora da tabela vira pasta com o
//! próprio hash, para o usuário renomear à mão. Nome de jogo não se inventa.

use std::path::{Path, PathBuf};

use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

use super::{
    place_file, sha256_file, summarize, unique_path, DedupeIndex, GameSummary, ImportItem,
};
use crate::core::tools::{report, sanitize_name, ProgressFn};

const ID: &str = "switch-album";
const GAMES_TSV: &str = include_str!("switch-games.tsv");

fn games() -> &'static std::collections::HashMap<&'static str, &'static str> {
    static TABLE: once_cell::sync::Lazy<std::collections::HashMap<&'static str, &'static str>> =
        once_cell::sync::Lazy::new(|| {
            GAMES_TSV
                .lines()
                .filter_map(|line| line.split_once('\t'))
                .collect()
        });
    &TABLE
}

/// Nome do jogo para um hash do álbum, se a tabela conhecer.
pub fn game_name(game_id: &str) -> Option<&'static str> {
    games().get(game_id.to_ascii_uppercase().as_str()).copied()
}

/// O que dá para extrair do nome de um arquivo do álbum.
#[derive(Debug, Clone, PartialEq)]
pub struct Capture {
    pub taken: NaiveDateTime,
    /// Contador dentro do mesmo segundo (os dois últimos dígitos).
    pub seq: u32,
    pub game_id: String,
    pub ext: String,
    pub is_clip: bool,
}

/// `20260909211403000-<32 hex>.jpg|.mp4`. Qualquer outra coisa é ignorada —
/// é assim que o import não mexe em `.DS_Store`, thumbnail ou arquivo já
/// renomeado por uma importação anterior.
pub fn parse_capture_name(file_name: &str) -> Option<Capture> {
    let (stem, ext) = file_name.rsplit_once('.')?;
    let ext = ext.to_ascii_lowercase();
    let is_clip = match ext.as_str() {
        "jpg" | "jpeg" => false,
        "mp4" => true,
        _ => return None,
    };
    let (stamp, hash) = stem.split_once('-')?;
    if stamp.len() != 16 || !stamp.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if hash.len() != 32 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let num = |a: usize, b: usize| stamp[a..b].parse::<u32>().ok();
    let taken =
        chrono::NaiveDate::from_ymd_opt(stamp[0..4].parse::<i32>().ok()?, num(4, 6)?, num(6, 8)?)?
            .and_hms_opt(num(8, 10)?, num(10, 12)?, num(12, 14)?)?;
    Some(Capture {
        taken,
        seq: num(14, 16)?,
        game_id: hash.to_ascii_uppercase(),
        ext: if ext == "jpeg" { "jpg".into() } else { ext },
        is_clip,
    })
}

#[derive(Debug, Clone, Deserialize)]
pub struct SwitchOptions {
    /// Pasta de origem: o cartão inteiro, o `Nintendo/Album` ou um dia só.
    pub source: String,
    /// Biblioteca de destino.
    pub dest: String,
    /// "game" | "game-year" | "year-month" | "flat"
    #[serde(default = "default_layout")]
    pub layout: String,
    /// "copy" | "move"
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default = "default_true")]
    pub dedupe: bool,
    #[serde(default)]
    pub dry_run: bool,
    /// Lê duração e resolução dos clipes com o ffprobe gerido.
    #[serde(default = "default_true")]
    pub probe_clips: bool,
    /// "" | "webm" | "gif" — conversão extra dos clipes, o original fica.
    #[serde(default)]
    pub convert_clips: String,
    /// Só imagens, só clipes, ou tudo: "all" | "images" | "clips".
    #[serde(default = "default_kinds")]
    pub kinds: String,
}

fn default_layout() -> String {
    "game".into()
}
fn default_mode() -> String {
    "copy".into()
}
fn default_true() -> bool {
    true
}
fn default_kinds() -> String {
    "all".into()
}

#[derive(Debug, Clone, Serialize)]
pub struct SwitchResult {
    pub items: Vec<ImportItem>,
    pub games: Vec<GameSummary>,
    pub found: u64,
    pub imported: u64,
    pub skipped: u64,
    pub failed: u64,
    pub bytes_imported: u64,
    /// Hashes que a tabela não conhece — a pasta ficou com o hash no nome.
    pub unknown_ids: Vec<String>,
    pub dry_run: bool,
}

/// Pasta de destino de uma captura, conforme o arranjo escolhido.
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

/// `Hades 2026-09-09 21-14-03` — sem extensão, sem o contador, que só entra
/// quando dois prints caem no mesmo segundo (o `unique_path` resolve).
pub fn dest_stem(game: &str, taken: &NaiveDateTime) -> String {
    sanitize_name(&format!("{} {}", game, taken.format("%Y-%m-%d %H-%M-%S")))
}

fn collect(source: &Path, kinds: &str) -> Vec<(PathBuf, Capture)> {
    let mut out: Vec<(PathBuf, Capture)> = Vec::new();
    for entry in walkdir::WalkDir::new(source)
        .max_depth(8)
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(cap) = parse_capture_name(&name) else {
            continue;
        };
        let wanted = match kinds {
            "images" => !cap.is_clip,
            "clips" => cap.is_clip,
            _ => true,
        };
        if wanted {
            out.push((entry.path().to_path_buf(), cap));
        }
    }
    out.sort_by(|a, b| a.1.taken.cmp(&b.1.taken).then_with(|| a.0.cmp(&b.0)));
    out
}

/// Duração e resolução de um clipe, pelo ffprobe gerido. Sem ffprobe, o clipe
/// entra do mesmo jeito — só não mostra os números.
async fn probe(ffprobe: &Path, file: &Path) -> Option<(f64, u32, u32)> {
    let out = crate::core::process::command(ffprobe)
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(file)
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let duration = json
        .get("format")
        .and_then(|f| f.get("duration"))
        .and_then(|d| d.as_str())
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);
    let video = json
        .get("streams")
        .and_then(|s| s.as_array())
        .and_then(|list| {
            list.iter()
                .find(|s| s.get("codec_type").and_then(|c| c.as_str()) == Some("video"))
        });
    let width = video
        .and_then(|v| v.get("width"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let height = video
        .and_then(|v| v.get("height"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    Some((duration, width, height))
}

/// WebM VP9 do clipe de 30 s: cabe em qualquer chat sem virar GIF de 40 MB.
async fn to_webm(ffmpeg: &Path, input: &Path) -> anyhow::Result<PathBuf> {
    let output = input.with_extension("webm");
    let out = crate::core::process::command(ffmpeg)
        .args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(input)
        .args([
            "-c:v",
            "libvpx-vp9",
            "-crf",
            "34",
            "-b:v",
            "0",
            "-row-mt",
            "1",
            "-c:a",
            "libopus",
            "-b:a",
            "96k",
        ])
        .arg(&output)
        .output()
        .await
        .map_err(|e| anyhow::anyhow!("ffmpeg nao iniciou: {}", e))?;
    if !out.status.success() {
        anyhow::bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(output)
}

pub async fn run(opts: SwitchOptions, progress: ProgressFn) -> anyhow::Result<SwitchResult> {
    let source = PathBuf::from(opts.source.trim());
    if !source.is_dir() {
        anyhow::bail!("pasta de origem não encontrada: {}", source.display());
    }
    let dest_root = PathBuf::from(opts.dest.trim());
    if opts.dest.trim().is_empty() {
        anyhow::bail!("escolha a pasta da biblioteca de destino");
    }
    let move_it = opts.mode == "move";

    report(&progress, ID, "progress", 0, None, None);
    let found = collect(&source, &opts.kinds);
    let total = found.len() as u64;
    if total == 0 {
        report(&progress, ID, "done", 0, Some(0), None);
        return Ok(SwitchResult {
            items: Vec::new(),
            games: Vec::new(),
            found: 0,
            imported: 0,
            skipped: 0,
            failed: 0,
            bytes_imported: 0,
            unknown_ids: Vec::new(),
            dry_run: opts.dry_run,
        });
    }

    let mut index = if opts.dedupe {
        DedupeIndex::load(&dest_root)
    } else {
        DedupeIndex::disabled()
    };

    let has_clip = found.iter().any(|(_, c)| c.is_clip);
    let ffprobe = if opts.probe_clips && has_clip {
        crate::core::dependencies::find_tool("ffprobe").await
    } else {
        None
    };
    let convert = opts.convert_clips.trim().to_ascii_lowercase();
    let ffmpeg = if has_clip && (convert == "webm" || convert == "gif") && !opts.dry_run {
        Some(crate::core::dependencies::ensure_ffmpeg().await?)
    } else {
        None
    };

    let mut items: Vec<ImportItem> = Vec::new();
    let mut unknown: Vec<String> = Vec::new();
    let (mut imported, mut skipped, mut failed, mut bytes_imported) = (0u64, 0u64, 0u64, 0u64);

    for (i, (path, cap)) in found.iter().enumerate() {
        report(
            &progress,
            ID,
            "progress",
            i as u64,
            Some(total),
            Some(path.to_string_lossy().to_string()),
        );
        let known = game_name(&cap.game_id);
        let game = known.map(|s| s.to_string()).unwrap_or_else(|| {
            if !unknown.contains(&cap.game_id) {
                unknown.push(cap.game_id.clone());
            }
            cap.game_id.clone()
        });
        let bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        let mut item = ImportItem {
            source: path.to_string_lossy().to_string(),
            dest: None,
            game: game.clone(),
            game_id: cap.game_id.clone(),
            known_game: known.is_some(),
            taken_at: cap.taken.format("%Y-%m-%d %H:%M:%S").to_string(),
            bytes,
            kind: if cap.is_clip {
                "clip".into()
            } else {
                "image".into()
            },
            duration_s: None,
            width: None,
            height: None,
            extra: None,
            status: "imported".into(),
            reason: None,
        };

        if index.enabled() {
            match sha256_file(path) {
                Ok(hash) => {
                    if index.contains(&hash) {
                        item.status = "skipped".into();
                        item.reason = Some("duplicate".into());
                        skipped += 1;
                        items.push(item);
                        continue;
                    }
                    if !opts.dry_run {
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

        if cap.is_clip {
            if let Some(ff) = ffprobe.as_ref() {
                if let Some((d, w, h)) = probe(ff, path).await {
                    item.duration_s = Some(d);
                    item.width = Some(w);
                    item.height = Some(h);
                }
            }
        }

        let dir = dest_dir(&dest_root, &opts.layout, &game, &cap.taken);
        let stem = dest_stem(&game, &cap.taken);
        if opts.dry_run {
            item.dest = Some(
                dir.join(format!("{}.{}", stem, cap.ext))
                    .to_string_lossy()
                    .to_string(),
            );
            imported += 1;
            bytes_imported += bytes;
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
        let target = unique_path(&dir, &stem, &cap.ext);
        match place_file(path, &target, move_it) {
            Ok(()) => {
                item.dest = Some(target.to_string_lossy().to_string());
                imported += 1;
                bytes_imported += bytes;
            }
            Err(e) => {
                item.status = "error".into();
                item.reason = Some(e.to_string());
                failed += 1;
                items.push(item);
                continue;
            }
        }

        if cap.is_clip {
            if let Some(ff) = ffmpeg.as_ref() {
                let made = if convert == "gif" {
                    let gif = crate::core::tools::video_gif::GifOptions {
                        inputs: vec![target.to_string_lossy().to_string()],
                        start: 0.0,
                        duration: 0.0,
                        fps: 15,
                        width: 480,
                        format: "gif".into(),
                        dither: "sierra2_4a".into(),
                        max_colors: 192,
                        quality: 75,
                        output_dir: dir.to_string_lossy().to_string(),
                        suffix: String::new(),
                    };
                    crate::core::tools::video_gif::run(gif, crate::core::tools::noop_progress())
                        .await
                        .ok()
                        .and_then(|r| r.items.into_iter().next())
                        .and_then(|it| it.output)
                } else {
                    to_webm(ff, &target)
                        .await
                        .map(|p| p.to_string_lossy().to_string())
                        .map_err(|e| tracing::warn!("[{}] webm de {:?}: {}", ID, target, e))
                        .ok()
                };
                item.extra = made;
            }
        }
        items.push(item);
    }

    if !opts.dry_run {
        if let Err(e) = index.save() {
            tracing::warn!("[{}] índice de duplicados: {}", ID, e);
        }
    }
    let games = summarize(&items);
    report(&progress, ID, "done", total, Some(total), None);
    Ok(SwitchResult {
        items,
        games,
        found: total,
        imported,
        skipped,
        failed,
        bytes_imported,
        unknown_ids: unknown,
        dry_run: opts.dry_run,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir(tag: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("omniget-switch-{}-{}", tag, uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).expect("criar tempdir");
        d
    }

    #[test]
    fn parses_a_real_screenshot_name() {
        let c = parse_capture_name("2017030619573600-F1C11A22FAEE3B82F21B330E1B786A39.jpg")
            .expect("nome válido");
        assert_eq!(
            c.taken.format("%Y-%m-%d %H:%M:%S").to_string(),
            "2017-03-06 19:57:36"
        );
        assert_eq!(c.seq, 0);
        assert_eq!(c.game_id, "F1C11A22FAEE3B82F21B330E1B786A39");
        assert!(!c.is_clip);
        assert_eq!(c.ext, "jpg");
    }

    #[test]
    fn parses_a_clip_and_the_counter() {
        let c = parse_capture_name("2026090921140312-8AEDFF741E2D23FBED39474178692DAF.mp4")
            .expect("nome válido");
        assert!(c.is_clip);
        assert_eq!(c.seq, 12);
        assert_eq!(c.taken.format("%H-%M-%S").to_string(), "21-14-03");
    }

    #[test]
    fn refuses_names_that_are_not_from_the_album() {
        // Extensão que o álbum não usa.
        assert!(
            parse_capture_name("2017030619573600-F1C11A22FAEE3B82F21B330E1B786A39.png").is_none()
        );
        // Hash curto.
        assert!(
            parse_capture_name("2017030619573600-F1C11A22FAEE3B82F21B330E1B786A3.jpg").is_none()
        );
        // Hash com letra fora do hexadecimal.
        assert!(
            parse_capture_name("2017030619573600-Z1C11A22FAEE3B82F21B330E1B786A39.jpg").is_none()
        );
        // Carimbo curto.
        assert!(
            parse_capture_name("201703061957360-F1C11A22FAEE3B82F21B330E1B786A39.jpg").is_none()
        );
        // Data impossível (mês 13).
        assert!(
            parse_capture_name("2017130619573600-F1C11A22FAEE3B82F21B330E1B786A39.jpg").is_none()
        );
        // Hora impossível.
        assert!(
            parse_capture_name("2017030699573600-F1C11A22FAEE3B82F21B330E1B786A39.jpg").is_none()
        );
        // Já renomeado por uma importação anterior.
        assert!(parse_capture_name("Hades 2026-09-09 21-14-03.jpg").is_none());
        assert!(parse_capture_name("sem-extensao").is_none());
    }

    #[test]
    fn the_game_table_knows_the_classics_and_admits_what_it_does_not() {
        assert_eq!(
            game_name("F1C11A22FAEE3B82F21B330E1B786A39"),
            Some("The Legend of Zelda - Breath of the Wild")
        );
        // Minúsculas também casam.
        assert_eq!(
            game_name("f1c11a22faee3b82f21b330e1b786a39"),
            Some("The Legend of Zelda - Breath of the Wild")
        );
        assert_eq!(
            game_name("8AEDFF741E2D23FBED39474178692DAF"),
            Some("Super Mario Odyssey")
        );
        assert!(game_name("00000000000000000000000000000000").is_none());
        assert!(
            games().len() > 1500,
            "tabela pequena demais: {}",
            games().len()
        );
    }

    #[test]
    fn destination_follows_the_layout() {
        let root = Path::new("/lib");
        let c = parse_capture_name("2026090921140300-8AEDFF741E2D23FBED39474178692DAF.jpg")
            .expect("nome válido");
        assert_eq!(
            dest_dir(root, "game", "Super Mario Odyssey", &c.taken),
            root.join("Super Mario Odyssey")
        );
        assert_eq!(
            dest_dir(root, "game-year", "Super Mario Odyssey", &c.taken),
            root.join("Super Mario Odyssey").join("2026")
        );
        assert_eq!(
            dest_dir(root, "year-month", "X", &c.taken),
            root.join("2026").join("09")
        );
        assert_eq!(dest_dir(root, "flat", "X", &c.taken), root.to_path_buf());
        assert_eq!(
            dest_stem("Super Mario Odyssey", &c.taken),
            "Super Mario Odyssey 2026-09-09 21-14-03"
        );
    }

    #[test]
    fn destination_name_is_safe_for_the_filesystem() {
        let c = parse_capture_name("2026090921140300-8AEDFF741E2D23FBED39474178692DAF.jpg")
            .expect("nome válido");
        let stem = dest_stem("Jogo: o/retorno", &c.taken);
        assert!(!stem.contains('/'), "{}", stem);
        assert!(stem.starts_with("Jogo"), "{}", stem);
    }

    fn tree(dir: &Path, names: &[(&str, &[u8])]) {
        std::fs::create_dir_all(dir).expect("criar árvore");
        for (name, body) in names {
            std::fs::write(dir.join(name), body).expect("escrever captura");
        }
    }

    #[tokio::test]
    async fn imports_a_synthetic_card_into_folders_by_game() {
        let base = tempdir("import");
        let card = base
            .join("Nintendo")
            .join("Album")
            .join("2026")
            .join("09")
            .join("09");
        tree(
            &card,
            &[
                (
                    "2026090921140300-F1C11A22FAEE3B82F21B330E1B786A39.jpg",
                    b"zelda-1",
                ),
                (
                    "2026090921140400-F1C11A22FAEE3B82F21B330E1B786A39.jpg",
                    b"zelda-2",
                ),
                (
                    "2026090922000000-00000000000000000000000000000000.jpg",
                    b"desconhecido",
                ),
                ("nao-e-do-album.jpg", b"lixo"),
            ],
        );
        let lib = base.join("lib");
        let opts = SwitchOptions {
            source: base.to_string_lossy().to_string(),
            dest: lib.to_string_lossy().to_string(),
            layout: "game".into(),
            mode: "copy".into(),
            dedupe: true,
            dry_run: false,
            probe_clips: false,
            convert_clips: String::new(),
            kinds: "all".into(),
        };
        let r = run(opts.clone(), crate::core::tools::noop_progress())
            .await
            .expect("importar");
        assert_eq!(r.found, 3, "o arquivo fora do padrão não pode entrar");
        assert_eq!(r.imported, 3);
        assert_eq!(r.skipped, 0);
        assert_eq!(r.failed, 0);
        assert_eq!(
            r.unknown_ids,
            vec!["00000000000000000000000000000000".to_string()]
        );
        assert!(lib
            .join("The Legend of Zelda - Breath of the Wild")
            .join("The Legend of Zelda - Breath of the Wild 2026-09-09 21-14-03.jpg")
            .exists());
        assert!(lib.join("00000000000000000000000000000000").is_dir());
        // A origem continua lá: modo copiar.
        assert!(card
            .join("2026090921140300-F1C11A22FAEE3B82F21B330E1B786A39.jpg")
            .exists());

        // Segunda rodada: tudo já está na biblioteca, nada entra de novo.
        let again = run(opts, crate::core::tools::noop_progress())
            .await
            .expect("reimportar");
        assert_eq!(again.imported, 0);
        assert_eq!(again.skipped, 3);
        assert!(again
            .items
            .iter()
            .all(|i| i.reason.as_deref() == Some("duplicate")));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[tokio::test]
    async fn move_mode_empties_the_card_and_dry_run_does_not() {
        let base = tempdir("move");
        let card = base.join("card");
        tree(
            &card,
            &[(
                "2026090921140300-8AEDFF741E2D23FBED39474178692DAF.jpg",
                b"mario",
            )],
        );
        let lib = base.join("lib");
        let dry = SwitchOptions {
            source: card.to_string_lossy().to_string(),
            dest: lib.to_string_lossy().to_string(),
            layout: "game-year".into(),
            mode: "move".into(),
            dedupe: false,
            dry_run: true,
            probe_clips: false,
            convert_clips: String::new(),
            kinds: "all".into(),
        };
        let r = run(dry.clone(), crate::core::tools::noop_progress())
            .await
            .expect("simular");
        assert_eq!(r.imported, 1);
        assert!(r.dry_run);
        assert!(card
            .join("2026090921140300-8AEDFF741E2D23FBED39474178692DAF.jpg")
            .exists());
        assert!(!lib.exists(), "ensaio não pode criar a biblioteca");

        let real = SwitchOptions {
            dry_run: false,
            ..dry
        };
        let r = run(real, crate::core::tools::noop_progress())
            .await
            .expect("mover");
        assert_eq!(r.imported, 1);
        assert!(!card
            .join("2026090921140300-8AEDFF741E2D23FBED39474178692DAF.jpg")
            .exists());
        assert!(lib
            .join("Super Mario Odyssey")
            .join("2026")
            .join("Super Mario Odyssey 2026-09-09 21-14-03.jpg")
            .exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[tokio::test]
    async fn same_second_captures_do_not_overwrite_each_other() {
        let base = tempdir("collide");
        let card = base.join("card");
        tree(
            &card,
            &[
                (
                    "2026090921140300-8AEDFF741E2D23FBED39474178692DAF.jpg",
                    b"a",
                ),
                (
                    "2026090921140301-8AEDFF741E2D23FBED39474178692DAF.jpg",
                    b"b",
                ),
            ],
        );
        let lib = base.join("lib");
        let r = run(
            SwitchOptions {
                source: card.to_string_lossy().to_string(),
                dest: lib.to_string_lossy().to_string(),
                layout: "game".into(),
                mode: "copy".into(),
                dedupe: true,
                dry_run: false,
                probe_clips: false,
                convert_clips: String::new(),
                kinds: "all".into(),
            },
            crate::core::tools::noop_progress(),
        )
        .await
        .expect("importar");
        assert_eq!(r.imported, 2);
        let game = lib.join("Super Mario Odyssey");
        assert!(game
            .join("Super Mario Odyssey 2026-09-09 21-14-03.jpg")
            .exists());
        assert!(game
            .join("Super Mario Odyssey 2026-09-09 21-14-03 (2).jpg")
            .exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[tokio::test]
    async fn kinds_filter_leaves_the_clips_behind() {
        let base = tempdir("kinds");
        let card = base.join("card");
        tree(
            &card,
            &[
                (
                    "2026090921140300-8AEDFF741E2D23FBED39474178692DAF.jpg",
                    b"foto",
                ),
                (
                    "2026090921150000-8AEDFF741E2D23FBED39474178692DAF.mp4",
                    b"clipe",
                ),
            ],
        );
        let lib = base.join("lib");
        let r = run(
            SwitchOptions {
                source: card.to_string_lossy().to_string(),
                dest: lib.to_string_lossy().to_string(),
                layout: "flat".into(),
                mode: "copy".into(),
                dedupe: false,
                dry_run: false,
                probe_clips: false,
                convert_clips: String::new(),
                kinds: "images".into(),
            },
            crate::core::tools::noop_progress(),
        )
        .await
        .expect("importar");
        assert_eq!(r.found, 1);
        assert_eq!(r.games.len(), 1);
        assert_eq!(r.games[0].images, 1);
        assert_eq!(r.games[0].clips, 0);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[tokio::test]
    async fn refuses_a_source_that_is_not_there() {
        let r = run(
            SwitchOptions {
                source: "/caminho/que/nao/existe/mesmo".into(),
                dest: "/tmp".into(),
                layout: "game".into(),
                mode: "copy".into(),
                dedupe: false,
                dry_run: true,
                probe_clips: false,
                convert_clips: String::new(),
                kinds: "all".into(),
            },
            crate::core::tools::noop_progress(),
        )
        .await;
        assert!(r.is_err());
    }
}
