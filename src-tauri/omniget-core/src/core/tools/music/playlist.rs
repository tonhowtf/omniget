//! Playlist exportada -> `.m3u8` / `.pls` apontando para os arquivos que o
//! usuário tem no disco. O trabalho de verdade é o casamento: o CSV do
//! Exportify traz "Song - Remastered 2011" e a pasta tem
//! "03 - Artist - Song.flac". O que não casar volta numa lista separada, que
//! é justamente o que o usuário quer ver.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::norm::{
    norm_artist, norm_title, primary_artist, similarity, simplify, strip_track_number,
    strip_track_number_loose,
};
use crate::core::tools::{report, ProgressFn};

const AUDIO_EXTS: &[&str] = &[
    "mp3", "m4a", "flac", "wav", "ogg", "opus", "aac", "wma", "aiff", "aif", "alac", "m4b", "ape",
    "wv",
];

const TOOL_ID: &str = "music-playlist";

#[derive(Debug, Clone, Deserialize)]
pub struct PlaylistOptions {
    /// CSV/JSON/TXT com a lista de faixas.
    pub source: String,
    /// Pastas onde procurar os arquivos de áudio.
    pub music_dirs: Vec<String>,
    /// Onde escrever os playlists.
    pub out_dir: String,
    #[serde(default = "default_name")]
    pub name: String,
    /// Caminho relativo ao playlist em vez de absoluto.
    #[serde(default)]
    pub relative: bool,
    #[serde(default = "default_true")]
    pub write_pls: bool,
    /// Semelhança mínima (0-100) para aceitar um quase-igual.
    #[serde(default = "default_threshold")]
    pub threshold: u32,
    /// Copiar as faixas casadas para esta pasta.
    #[serde(default)]
    pub copy_to: Option<String>,
}

fn default_name() -> String {
    "playlist".to_string()
}
fn default_true() -> bool {
    true
}
fn default_threshold() -> u32 {
    82
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Track {
    pub artist: String,
    pub title: String,
    #[serde(default)]
    pub album: String,
    #[serde(default)]
    pub duration_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MatchedTrack {
    pub artist: String,
    pub title: String,
    pub path: String,
    /// 100 = chave idêntica; abaixo disso é o quase-igual.
    pub score: u32,
    pub duration_secs: Option<u64>,
    pub copied_to: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlaylistResult {
    pub total: usize,
    pub scanned_files: usize,
    pub matched: Vec<MatchedTrack>,
    pub missing: Vec<Track>,
    pub m3u_path: String,
    pub pls_path: Option<String>,
    pub copied: usize,
}

// ── Leitura da lista de faixas ──────────────────────────────────────────

/// Divisor de CSV que respeita aspas e `""` escapado.
pub fn split_csv_line(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    cur.push('"');
                    chars.next();
                } else {
                    quoted = false;
                }
            } else {
                cur.push(c);
            }
        } else if c == '"' {
            quoted = true;
        } else if c == ',' {
            out.push(cur.trim().to_string());
            cur = String::new();
        } else {
            cur.push(c);
        }
    }
    out.push(cur.trim().to_string());
    out
}

fn find_col(header: &[String], names: &[&str]) -> Option<usize> {
    for name in names {
        for (i, h) in header.iter().enumerate() {
            if simplify(h) == simplify(name) {
                return Some(i);
            }
        }
    }
    None
}

/// CSV do Exportify e parecidos: acha as colunas pelo nome do cabeçalho.
pub fn parse_csv(text: &str) -> Vec<Track> {
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let Some(head) = lines.next() else {
        return Vec::new();
    };
    let header = split_csv_line(head);
    let ci_title = find_col(
        &header,
        &["Track Name", "track_name", "name", "title", "Track"],
    );
    let ci_artist = find_col(
        &header,
        &[
            "Artist Name(s)",
            "Artist Name",
            "artist_name",
            "artist",
            "artists",
            "Album Artist",
        ],
    );
    let ci_album = find_col(&header, &["Album Name", "album_name", "album"]);
    let ci_dur = find_col(
        &header,
        &["Duration (ms)", "duration_ms", "Duration", "duration"],
    );
    let Some(ci_title) = ci_title else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for line in lines {
        let cols = split_csv_line(line);
        let get = |i: Option<usize>| -> String {
            i.and_then(|i| cols.get(i)).cloned().unwrap_or_default()
        };
        let title = get(Some(ci_title));
        if title.is_empty() {
            continue;
        }
        let raw_dur = get(ci_dur);
        out.push(Track {
            artist: get(ci_artist),
            title,
            album: get(ci_album),
            duration_secs: parse_duration(&raw_dur),
        });
    }
    out
}

/// Aceita segundos ("213"), milissegundos ("213000") e "3:33".
pub fn parse_duration(raw: &str) -> Option<u64> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    if let Some((m, s)) = raw.split_once(':') {
        let m: u64 = m.trim().parse().ok()?;
        let s: u64 = s.trim().split('.').next()?.parse().ok()?;
        return Some(m * 60 + s);
    }
    let n: f64 = raw.parse().ok()?;
    if n <= 0.0 {
        return None;
    }
    // Acima de 10 mil é quase certo que veio em milissegundos.
    Some(if n > 10_000.0 {
        (n / 1000.0).round() as u64
    } else {
        n.round() as u64
    })
}

fn json_str(v: &Value, keys: &[&str]) -> String {
    for k in keys {
        match v.get(*k) {
            Some(Value::String(s)) => return s.clone(),
            Some(Value::Array(a)) => {
                let names: Vec<String> = a
                    .iter()
                    .filter_map(|x| match x {
                        Value::String(s) => Some(s.clone()),
                        Value::Object(_) => {
                            x.get("name").and_then(|n| n.as_str()).map(String::from)
                        }
                        _ => None,
                    })
                    .collect();
                if !names.is_empty() {
                    return names.join(", ");
                }
            }
            Some(Value::Object(_)) => {
                if let Some(n) = v
                    .get(*k)
                    .and_then(|o| o.get("name"))
                    .and_then(|n| n.as_str())
                {
                    return n.to_string();
                }
            }
            _ => {}
        }
    }
    String::new()
}

/// JSON de export do Spotify, da API, do Exportify ou uma lista simples de
/// objetos. Desce por `items`/`tracks`/`track` sozinho.
pub fn parse_json(text: &str) -> Result<Vec<Track>> {
    let v: Value = serde_json::from_str(text).context("JSON inválido")?;
    let arr = find_array(&v).unwrap_or_default();
    let mut out = Vec::new();
    for item in arr {
        let item = match item.get("track") {
            Some(t) if t.is_object() => t.clone(),
            _ => item,
        };
        let title = json_str(
            &item,
            &[
                "name",
                "track_name",
                "trackName",
                "title",
                "Track Name",
                "master_metadata_track_name",
            ],
        );
        if title.is_empty() {
            continue;
        }
        let artist = json_str(
            &item,
            &[
                "artists",
                "artist",
                "artist_name",
                "artistName",
                "Artist Name(s)",
                "master_metadata_album_artist_name",
                "albumArtist",
            ],
        );
        let album = json_str(
            &item,
            &[
                "album",
                "album_name",
                "albumName",
                "Album Name",
                "master_metadata_album_album_name",
            ],
        );
        let dur = item
            .get("duration_ms")
            .or_else(|| item.get("durationMs"))
            .or_else(|| item.get("Duration (ms)"))
            .or_else(|| item.get("duration"))
            .map(|d| match d {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .unwrap_or_default();
        out.push(Track {
            artist,
            title,
            album,
            duration_secs: parse_duration(&dur),
        });
    }
    Ok(out)
}

fn find_array(v: &Value) -> Option<Vec<Value>> {
    match v {
        Value::Array(a) => Some(a.clone()),
        Value::Object(_) => {
            for k in ["items", "tracks", "playlist", "songs", "data"] {
                if let Some(inner) = v.get(k) {
                    if let Some(a) = find_array(inner) {
                        return Some(a);
                    }
                }
            }
            None
        }
        _ => None,
    }
}

/// Uma faixa por linha, "Artista - Título" (ou só o título).
pub fn parse_lines(text: &str) -> Vec<Track> {
    let mut out = Vec::new();
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        let l = strip_track_number(l);
        let normalized = l.replace(" – ", " - ").replace(" — ", " - ");
        match normalized.split_once(" - ") {
            Some((a, t)) if !t.trim().is_empty() => out.push(Track {
                artist: a.trim().to_string(),
                title: t.trim().to_string(),
                ..Default::default()
            }),
            _ => out.push(Track {
                title: normalized.trim().to_string(),
                ..Default::default()
            }),
        }
    }
    out
}

/// Escolhe o leitor pelo conteúdo, não só pela extensão.
pub fn parse_source(text: &str, ext: &str) -> Result<Vec<Track>> {
    let trimmed = text.trim_start();
    if ext.eq_ignore_ascii_case("json") || trimmed.starts_with('[') || trimmed.starts_with('{') {
        return parse_json(text);
    }
    if ext.eq_ignore_ascii_case("csv") || (trimmed.contains(',') && trimmed.lines().count() > 1) {
        let csv = parse_csv(text);
        if !csv.is_empty() {
            return Ok(csv);
        }
    }
    Ok(parse_lines(text))
}

// ── Índice dos arquivos de áudio ────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct AudioFile {
    pub path: PathBuf,
    /// Chaves "artista título" candidatas.
    pub full_keys: Vec<String>,
    /// Chaves só de título.
    pub title_keys: Vec<String>,
    /// Onde procurar o artista: nome do arquivo mais as duas pastas acima.
    pub hay: String,
}

fn push_uniq(v: &mut Vec<String>, s: String) {
    if !s.is_empty() && !v.contains(&s) {
        v.push(s);
    }
}

impl AudioFile {
    pub fn from_parts(path: PathBuf, stem: &str, parents: &str) -> Self {
        let mut full_keys = Vec::new();
        let mut title_keys = Vec::new();
        let variants = [
            stem.to_string(),
            strip_track_number(stem),
            strip_track_number_loose(stem),
        ];
        for v in variants.iter() {
            push_uniq(&mut full_keys, norm_title(v));
            let dashed = v
                .replace(" – ", " - ")
                .replace(" — ", " - ")
                .replace('_', " ");
            if let Some((a, t)) = dashed.split_once(" - ") {
                let key = format!("{} {}", norm_artist(a), norm_title(t));
                push_uniq(&mut full_keys, key.trim().to_string());
                push_uniq(&mut title_keys, norm_title(t));
                // Alguns arquivos vêm "Título - Artista".
                let flip = format!("{} {}", norm_artist(t), norm_title(a));
                push_uniq(&mut full_keys, flip.trim().to_string());
            } else {
                push_uniq(&mut title_keys, norm_title(v));
            }
        }
        let hay = simplify(&format!("{stem} {parents}"));
        AudioFile {
            path,
            full_keys,
            title_keys,
            hay,
        }
    }

    pub fn from_path(path: &Path) -> Self {
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let mut parents = String::new();
        let mut p = path.parent();
        for _ in 0..2 {
            if let Some(dir) = p {
                if let Some(n) = dir.file_name() {
                    parents.push(' ');
                    parents.push_str(&n.to_string_lossy());
                }
                p = dir.parent();
            }
        }
        AudioFile::from_parts(path.to_path_buf(), &stem, &parents)
    }
}

fn is_audio(p: &Path) -> bool {
    p.extension()
        .map(|e| AUDIO_EXTS.contains(&e.to_string_lossy().to_lowercase().as_str()))
        .unwrap_or(false)
}

pub fn index_dirs(dirs: &[String]) -> Vec<AudioFile> {
    let mut out = Vec::new();
    for d in dirs {
        for entry in walkdir::WalkDir::new(d)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if entry.file_type().is_file() && is_audio(entry.path()) {
                out.push(AudioFile::from_path(entry.path()));
            }
        }
    }
    out
}

// ── Casamento ───────────────────────────────────────────────────────────

/// Chaves de busca de uma faixa da playlist.
#[derive(Debug, Clone)]
pub struct Wanted {
    pub full: String,
    pub title: String,
    pub artist: String,
    pub primary: String,
}

pub fn wanted_keys(t: &Track) -> Wanted {
    let artist = norm_artist(&t.artist);
    let title = norm_title(&t.title);
    let full = format!("{artist} {title}").trim().to_string();
    Wanted {
        primary: primary_artist(&t.artist),
        full,
        title,
        artist,
    }
}

/// Corta antes de calcular Levenshtein: a diferença de tamanho já é um piso
/// da distância, então dá para descartar sem contar.
fn too_different(a: &str, b: &str, threshold: u32) -> bool {
    let la = a.chars().count();
    let lb = b.chars().count();
    let max = la.max(lb);
    if max == 0 {
        return true;
    }
    let diff = la.abs_diff(lb);
    ((max - diff) as f64 / max as f64) * 100.0 < threshold as f64
}

/// Só aceitamos casar por título quando o artista aparece em algum lugar do
/// caminho — senão "Song.mp3" de qualquer pasta casaria com tudo.
pub fn artist_ok(w: &Wanted, f: &AudioFile) -> bool {
    w.artist.is_empty()
        || f.hay.contains(&w.artist)
        || (!w.primary.is_empty() && f.hay.contains(&w.primary))
}

/// Semelhança de uma faixa com um arquivo, de 0 a 100.
pub fn score(w: &Wanted, f: &AudioFile, threshold: u32) -> u32 {
    let mut best = 0u32;
    for k in &f.full_keys {
        if k == &w.full {
            return 100;
        }
        if !too_different(&w.full, k, threshold) {
            best = best.max(similarity(&w.full, k));
        }
    }
    if artist_ok(w, f) {
        for k in &f.title_keys {
            if k == &w.title {
                return 100;
            }
            if !too_different(&w.title, k, threshold) {
                best = best.max(similarity(&w.title, k));
            }
        }
    }
    best
}

/// Casa cada faixa com o melhor arquivo ainda livre. Devolve, na ordem da
/// playlist, o índice do arquivo e a nota — `None` para o que não achou.
pub fn match_tracks(
    tracks: &[Track],
    files: &[AudioFile],
    threshold: u32,
    p: &ProgressFn,
) -> Vec<Option<(usize, u32)>> {
    // Índice exato: a maioria casa aqui, sem nenhuma conta de distância.
    let mut exact: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, f) in files.iter().enumerate() {
        for k in f.full_keys.iter().chain(f.title_keys.iter()) {
            exact.entry(k.as_str()).or_default().push(i);
        }
    }

    let mut used = vec![false; files.len()];
    let mut out = Vec::with_capacity(tracks.len());
    let total = tracks.len() as u64;
    for (n, t) in tracks.iter().enumerate() {
        let w = wanted_keys(t);
        let mut hit: Option<(usize, u32)> = None;
        if !w.full.is_empty() {
            if let Some(cands) = exact.get(w.full.as_str()) {
                if let Some(&i) = cands.iter().find(|&&i| !used[i]) {
                    hit = Some((i, 100));
                }
            }
        }
        if hit.is_none() && !w.title.is_empty() {
            if let Some(cands) = exact.get(w.title.as_str()) {
                if let Some(&i) = cands
                    .iter()
                    .find(|&&i| !used[i] && artist_ok(&w, &files[i]))
                {
                    hit = Some((i, 100));
                }
            }
        }
        if hit.is_none() {
            let mut best: Option<(usize, u32)> = None;
            for (i, f) in files.iter().enumerate() {
                if used[i] {
                    continue;
                }
                let s = score(&w, f, threshold);
                if s >= threshold && best.map(|(_, bs)| s > bs).unwrap_or(true) {
                    best = Some((i, s));
                    if s == 100 {
                        break;
                    }
                }
            }
            hit = best;
        }
        if let Some((i, _)) = hit {
            used[i] = true;
        }
        out.push(hit);
        if n % 10 == 0 || n + 1 == tracks.len() {
            report(
                p,
                TOOL_ID,
                "progress",
                n as u64 + 1,
                Some(total),
                Some(t.title.clone()),
            );
        }
    }
    out
}

// ── Escrita dos playlists ───────────────────────────────────────────────

/// Caminho de `to` visto de dentro de `from_dir`, com barra normal.
pub fn relative_path(from_dir: &Path, to: &Path) -> String {
    let from: Vec<_> = from_dir.components().collect();
    let dest: Vec<_> = to.components().collect();
    let common = from
        .iter()
        .zip(dest.iter())
        .take_while(|(a, b)| a == b)
        .count();
    if common == 0 {
        return to.to_string_lossy().replace('\\', "/");
    }
    let mut parts: Vec<String> = vec!["..".to_string(); from.len() - common];
    for c in &dest[common..] {
        parts.push(c.as_os_str().to_string_lossy().to_string());
    }
    parts.join("/")
}

/// Uma linha do playlist já pronta para escrever.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub path: String,
    pub artist: String,
    pub title: String,
    pub duration_secs: Option<u64>,
}

/// `.m3u8` em UTF-8. Duração desconhecida vira `-1`, que é o combinado do
/// formato.
pub fn build_m3u(entries: &[Entry]) -> String {
    let mut s = String::from("#EXTM3U\n");
    for e in entries {
        let secs = e.duration_secs.map(|d| d as i64).unwrap_or(-1);
        let who = if e.artist.is_empty() {
            e.title.clone()
        } else {
            format!("{} - {}", e.artist, e.title)
        };
        s.push_str(&format!("#EXTINF:{secs},{who}\n"));
        s.push_str(&e.path);
        s.push('\n');
    }
    s
}

pub fn build_pls(entries: &[Entry]) -> String {
    let mut s = String::from("[playlist]\n");
    for (i, e) in entries.iter().enumerate() {
        let n = i + 1;
        let who = if e.artist.is_empty() {
            e.title.clone()
        } else {
            format!("{} - {}", e.artist, e.title)
        };
        s.push_str(&format!("File{n}={}\n", e.path));
        s.push_str(&format!("Title{n}={who}\n"));
        s.push_str(&format!(
            "Length{n}={}\n",
            e.duration_secs.map(|d| d as i64).unwrap_or(-1)
        ));
    }
    s.push_str(&format!("NumberOfEntries={}\n", entries.len()));
    s.push_str("Version=2\n");
    s
}

// ── Execução ────────────────────────────────────────────────────────────

pub fn run(opts: &PlaylistOptions, p: &ProgressFn) -> Result<PlaylistResult> {
    report(p, TOOL_ID, "started", 0, None, None);
    let src = PathBuf::from(&opts.source);
    let text = std::fs::read_to_string(&src)
        .with_context(|| format!("não consegui ler {}", src.display()))?;
    let ext = src
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_default();
    let tracks = parse_source(&text, &ext)?;
    if tracks.is_empty() {
        anyhow::bail!("não achei nenhuma faixa em {}", src.display());
    }

    report(
        p,
        TOOL_ID,
        "progress",
        0,
        Some(tracks.len() as u64),
        Some("lendo a pasta".to_string()),
    );
    let files = index_dirs(&opts.music_dirs);
    let threshold = opts.threshold.min(100);
    let hits = match_tracks(&tracks, &files, threshold, p);

    let out_dir = PathBuf::from(&opts.out_dir);
    std::fs::create_dir_all(&out_dir)?;
    let copy_dir = opts.copy_to.as_ref().map(PathBuf::from);
    if let Some(d) = &copy_dir {
        std::fs::create_dir_all(d)?;
    }

    let mut entries = Vec::new();
    let mut matched = Vec::new();
    let mut missing = Vec::new();
    let mut copied = 0usize;
    for (t, hit) in tracks.iter().zip(hits.iter()) {
        let Some((i, s)) = hit else {
            missing.push(t.clone());
            continue;
        };
        let f = &files[*i];
        let path_str = if opts.relative {
            relative_path(&out_dir, &f.path)
        } else {
            f.path.to_string_lossy().to_string()
        };
        let mut copied_to = None;
        if let Some(d) = &copy_dir {
            let name = f
                .path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| format!("{}.mp3", t.title));
            let dest = d.join(crate::core::tools::sanitize_name(&format!(
                "{:03} - {name}",
                matched.len() + 1
            )));
            if std::fs::copy(&f.path, &dest).is_ok() {
                copied += 1;
                copied_to = Some(dest.to_string_lossy().to_string());
            }
        }
        entries.push(Entry {
            path: path_str.clone(),
            artist: t.artist.clone(),
            title: t.title.clone(),
            duration_secs: t.duration_secs,
        });
        matched.push(MatchedTrack {
            artist: t.artist.clone(),
            title: t.title.clone(),
            path: f.path.to_string_lossy().to_string(),
            score: *s,
            duration_secs: t.duration_secs,
            copied_to,
        });
    }

    let base = crate::core::tools::sanitize_name(&opts.name);
    let m3u_path = out_dir.join(format!("{base}.m3u8"));
    std::fs::write(&m3u_path, build_m3u(&entries))?;
    let pls_path = if opts.write_pls {
        let pls = out_dir.join(format!("{base}.pls"));
        std::fs::write(&pls, build_pls(&entries))?;
        Some(pls.to_string_lossy().to_string())
    } else {
        None
    };

    report(
        p,
        TOOL_ID,
        "done",
        matched.len() as u64,
        Some(tracks.len() as u64),
        None,
    );
    Ok(PlaylistResult {
        total: tracks.len(),
        scanned_files: files.len(),
        matched,
        missing,
        m3u_path: m3u_path.to_string_lossy().to_string(),
        pls_path,
        copied,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::tools::noop_progress;

    fn af(path: &str) -> AudioFile {
        AudioFile::from_path(Path::new(path))
    }

    #[test]
    fn csv_do_exportify() {
        let csv = "\"Track URI\",\"Track Name\",\"Artist Name(s)\",\"Album Name\",\"Duration (ms)\"\n\
                   \"spotify:track:1\",\"Bad Guy\",\"Billie Eilish\",\"When We All Fall Asleep\",\"194087\"\n\
                   \"spotify:track:2\",\"Come Together - Remastered 2009\",\"The Beatles\",\"Abbey Road\",\"259947\"\n";
        let t = parse_csv(csv);
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].title, "Bad Guy");
        assert_eq!(t[0].artist, "Billie Eilish");
        assert_eq!(t[0].duration_secs, Some(194));
        assert_eq!(t[1].artist, "The Beatles");
    }

    #[test]
    fn csv_com_virgula_dentro_de_aspas() {
        let cols = split_csv_line("\"a, b\",c,\"d\"\"e\"");
        assert_eq!(cols, vec!["a, b", "c", "d\"e"]);
    }

    #[test]
    fn json_da_api_do_spotify() {
        let j = r#"{"items":[
            {"track":{"name":"Californication","artists":[{"name":"Red Hot Chili Peppers"}],
             "album":{"name":"Californication"},"duration_ms":329733}},
            {"track":{"name":"Stay (feat. Justin Bieber)","artists":[{"name":"The Kid LAROI"},{"name":"Justin Bieber"}],"duration_ms":141805}}
        ]}"#;
        let t = parse_json(j).expect("json");
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].artist, "Red Hot Chili Peppers");
        assert_eq!(t[0].duration_secs, Some(330));
        assert_eq!(t[1].artist, "The Kid LAROI, Justin Bieber");
    }

    #[test]
    fn lista_texto_artista_titulo() {
        let t = parse_lines("# minha lista\nDaft Punk - Around the World\n01 - Air - La Femme d'Argent\nInstrumental\n");
        assert_eq!(t.len(), 3);
        assert_eq!(t[0].artist, "Daft Punk");
        assert_eq!(t[1].artist, "Air");
        assert_eq!(t[1].title, "La Femme d'Argent");
        assert_eq!(t[2].artist, "");
        assert_eq!(t[2].title, "Instrumental");
    }

    #[test]
    fn duracao_em_varios_formatos() {
        assert_eq!(parse_duration("194087"), Some(194));
        assert_eq!(parse_duration("213"), Some(213));
        assert_eq!(parse_duration("3:33"), Some(213));
        assert_eq!(parse_duration(""), None);
        assert_eq!(parse_duration("0"), None);
    }

    #[test]
    fn casa_exato_e_quase_igual() {
        let files = vec![
            af("/m/The Beatles/Abbey Road/01 - The Beatles - Come Together.flac"),
            af("/m/Billie Eilish/bad guy.mp3"),
            af("/m/Outros/Random Noise.mp3"),
        ];
        let tracks = vec![
            Track {
                artist: "The Beatles".into(),
                title: "Come Together - Remastered 2009".into(),
                ..Default::default()
            },
            Track {
                artist: "Billie Eilish".into(),
                title: "Bad Guy".into(),
                ..Default::default()
            },
            Track {
                artist: "Ninguém".into(),
                title: "Faixa Que Não Existe".into(),
                ..Default::default()
            },
        ];
        let hits = match_tracks(&tracks, &files, 82, &noop_progress());
        assert_eq!(hits[0].map(|(i, _)| i), Some(0));
        assert_eq!(hits[1].map(|(i, _)| i), Some(1));
        assert!(hits[2].is_none());
    }

    #[test]
    fn quase_igual_com_erro_de_digitacao() {
        let files = vec![af("/m/Imagine Dragons - Beleiver.mp3")];
        let tracks = vec![Track {
            artist: "Imagine Dragons".into(),
            title: "Believer".into(),
            ..Default::default()
        }];
        let hits = match_tracks(&tracks, &files, 82, &noop_progress());
        let (i, s) = hits[0].expect("devia casar");
        assert_eq!(i, 0);
        assert!((82..100).contains(&s), "nota {s}");
    }

    #[test]
    fn nao_casa_o_mesmo_arquivo_duas_vezes() {
        let files = vec![af("/m/Artist - Song.mp3")];
        let tracks = vec![
            Track {
                artist: "Artist".into(),
                title: "Song".into(),
                ..Default::default()
            },
            Track {
                artist: "Artist".into(),
                title: "Song".into(),
                ..Default::default()
            },
        ];
        let hits = match_tracks(&tracks, &files, 82, &noop_progress());
        assert!(hits[0].is_some());
        assert!(hits[1].is_none());
    }

    #[test]
    fn titulo_sozinho_precisa_do_artista_por_perto() {
        let files = vec![af("/m/Outro Artista/Song.mp3")];
        let tracks = vec![Track {
            artist: "Artista Certo".into(),
            title: "Song".into(),
            ..Default::default()
        }];
        let hits = match_tracks(&tracks, &files, 82, &noop_progress());
        assert!(hits[0].is_none());
    }

    #[test]
    fn m3u8_byte_a_byte() {
        let entries = vec![
            Entry {
                path: "/m/a.mp3".into(),
                artist: "A".into(),
                title: "Um".into(),
                duration_secs: Some(210),
            },
            Entry {
                path: "../b.mp3".into(),
                artist: String::new(),
                title: "Dois".into(),
                duration_secs: None,
            },
        ];
        assert_eq!(
            build_m3u(&entries),
            "#EXTM3U\n#EXTINF:210,A - Um\n/m/a.mp3\n#EXTINF:-1,Dois\n../b.mp3\n"
        );
    }

    #[test]
    fn pls_byte_a_byte() {
        let entries = vec![Entry {
            path: "/m/a.mp3".into(),
            artist: "A".into(),
            title: "Um".into(),
            duration_secs: Some(210),
        }];
        assert_eq!(
            build_pls(&entries),
            "[playlist]\nFile1=/m/a.mp3\nTitle1=A - Um\nLength1=210\nNumberOfEntries=1\nVersion=2\n"
        );
    }

    #[test]
    fn caminho_relativo() {
        assert_eq!(
            relative_path(Path::new("/m/lists"), Path::new("/m/rock/a.mp3")),
            "../rock/a.mp3"
        );
        assert_eq!(
            relative_path(Path::new("/m"), Path::new("/m/a.mp3")),
            "a.mp3"
        );
    }
}
