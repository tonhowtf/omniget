//! Histórico completo do export do Spotify — o que o Wrapped não mostra.
//!
//! Dois formatos chegam na caixa do usuário: `StreamingHistory_music_*.json`
//! (últimos 12 meses, campos curtos) e `Streaming_History_Audio_*.json` do
//! pacote estendido (a vida inteira, com `ms_played`, `skipped`, `shuffle`,
//! `platform` e companhia). Lemos os dois, de um zip ou de uma pasta.
//! Zero rede: é tudo arquivo local.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, TimeZone, Timelike, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::tools::{report, ProgressFn};

const TOOL_ID: &str = "music-history";
const DAY: i64 = 86_400;

#[derive(Debug, Clone, Deserialize)]
pub struct HistoryOptions {
    /// Zip do export, pasta descompactada ou os `.json` soltos.
    pub inputs: Vec<String>,
    /// Abaixo disso conta como pulada (o padrão do Spotify é 30 s).
    #[serde(default = "default_min_ms")]
    pub min_ms: u64,
    /// Onde gravar os arquivos de export. Vazio = não exporta.
    #[serde(default)]
    pub out_dir: Option<String>,
    /// "json", "csv", "md".
    #[serde(default)]
    pub formats: Vec<String>,
    #[serde(default = "default_top")]
    pub top: usize,
}

fn default_min_ms() -> u64 {
    30_000
}
fn default_top() -> usize {
    25
}

/// Uma escuta, já no mesmo formato venha de que arquivo vier.
#[derive(Debug, Clone, Serialize)]
pub struct Play {
    pub ts: i64,
    pub artist: String,
    pub track: String,
    pub album: String,
    pub ms: u64,
    pub skipped: bool,
    pub shuffle: Option<bool>,
    pub platform: Option<String>,
    pub country: Option<String>,
    pub reason_end: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Bucket {
    pub key: String,
    pub plays: u64,
    pub ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArtistStat {
    pub name: String,
    pub plays: u64,
    pub ms: u64,
    pub skips: u64,
    /// 0 a 100.
    pub skip_rate: u32,
    pub first_ts: String,
    pub first_track: String,
    pub last_ts: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrackStat {
    pub track: String,
    pub artist: String,
    pub plays: u64,
    pub ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct YearTop {
    pub year: String,
    pub artist: String,
    pub track: String,
    pub ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Streaks {
    pub longest_days: u64,
    pub longest_start: String,
    pub longest_end: String,
    pub current_days: u64,
    pub active_days: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Dropped {
    pub name: String,
    pub before_ms: u64,
    pub after_ms: u64,
    pub last_ts: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoryResult {
    pub files: Vec<String>,
    pub plays: u64,
    pub total_ms: u64,
    pub distinct_artists: usize,
    pub distinct_tracks: usize,
    pub first_ts: String,
    pub last_ts: String,
    pub by_year: Vec<Bucket>,
    pub by_month: Vec<Bucket>,
    /// Índice 0 = segunda-feira.
    pub by_weekday: Vec<Bucket>,
    pub by_hour: Vec<Bucket>,
    pub top_artists: Vec<ArtistStat>,
    pub top_tracks: Vec<TrackStat>,
    pub top_albums: Vec<TrackStat>,
    pub year_top: Vec<YearTop>,
    /// Quem você mais pula, entre os artistas com escuta relevante.
    pub skipped_artists: Vec<ArtistStat>,
    /// Descobertas recentes: a primeira vez que ouvi cada artista, do mais
    /// novo para o mais antigo.
    pub firsts: Vec<ArtistStat>,
    pub streaks: Streaks,
    /// Sumiram: ouvia muito no ano anterior e quase nada no último.
    pub dropped: Vec<Dropped>,
    pub exports: Vec<String>,
}

// ── Leitura ─────────────────────────────────────────────────────────────

fn parse_ts(v: &Value) -> Option<i64> {
    let s = v.as_str()?;
    if let Ok(d) = DateTime::parse_from_rfc3339(s) {
        return Some(d.with_timezone(&Utc).timestamp());
    }
    for fmt in ["%Y-%m-%d %H:%M", "%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M:%S"] {
        if let Ok(n) = NaiveDateTime::parse_from_str(s, fmt) {
            return Some(n.and_utc().timestamp());
        }
    }
    None
}

fn s_of(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(|x| x.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Lê um arquivo de histórico nos dois formatos. Podcasts (sem
/// `master_metadata_track_name`) ficam de fora: aqui é música.
pub fn parse_history(text: &str, min_ms: u64) -> Vec<Play> {
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    let arr = match v {
        Value::Array(a) => a,
        Value::Object(ref o) => o
            .values()
            .find_map(|x| x.as_array().cloned())
            .unwrap_or_default(),
        _ => return Vec::new(),
    };
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let ts = item
            .get("ts")
            .and_then(parse_ts_opt)
            .or_else(|| item.get("endTime").and_then(parse_ts_opt));
        let Some(ts) = ts else { continue };
        let track = s_of(&item, "master_metadata_track_name").or_else(|| s_of(&item, "trackName"));
        let Some(track) = track else { continue };
        let artist = s_of(&item, "master_metadata_album_artist_name")
            .or_else(|| s_of(&item, "artistName"))
            .unwrap_or_default();
        let ms = item
            .get("ms_played")
            .or_else(|| item.get("msPlayed"))
            .and_then(|x| x.as_u64())
            .unwrap_or(0);
        let reason_end = s_of(&item, "reason_end");
        let flag = item.get("skipped").and_then(|x| x.as_bool());
        let skipped = flag.unwrap_or(false)
            || reason_end.as_deref() == Some("fwdbtn")
            || (flag.is_none() && ms < min_ms);
        out.push(Play {
            ts,
            artist,
            track,
            album: s_of(&item, "master_metadata_album_album_name").unwrap_or_default(),
            ms,
            skipped,
            shuffle: item.get("shuffle").and_then(|x| x.as_bool()),
            platform: s_of(&item, "platform"),
            country: s_of(&item, "conn_country"),
            reason_end,
        });
    }
    out
}

fn parse_ts_opt(v: &Value) -> Option<i64> {
    parse_ts(v)
}

fn looks_like_history(name: &str) -> bool {
    let n = name.to_lowercase();
    n.ends_with(".json")
        && (n.contains("streaminghistory")
            || n.contains("streaming_history")
            || n.contains("audio"))
}

/// Junta o conteúdo dos arquivos de histórico de zips, pastas e `.json`
/// soltos. Devolve (nome mostrado, conteúdo).
pub fn collect_sources(inputs: &[String]) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    for input in inputs {
        let p = PathBuf::from(input);
        if p.is_dir() {
            for entry in walkdir::WalkDir::new(&p)
                .follow_links(false)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let f = entry.path();
                if entry.file_type().is_file()
                    && looks_like_history(&f.file_name().unwrap_or_default().to_string_lossy())
                {
                    if let Ok(t) = std::fs::read_to_string(f) {
                        out.push((f.to_string_lossy().to_string(), t));
                    }
                }
            }
        } else if p
            .extension()
            .map(|e| e.eq_ignore_ascii_case("zip"))
            .unwrap_or(false)
        {
            let file = std::fs::File::open(&p)
                .with_context(|| format!("não consegui abrir {}", p.display()))?;
            let mut zip = zip::ZipArchive::new(file)?;
            for i in 0..zip.len() {
                let mut e = zip.by_index(i)?;
                let name = e.name().to_string();
                if !e.is_file() || !looks_like_history(&name) {
                    continue;
                }
                let mut buf = String::new();
                if e.read_to_string(&mut buf).is_ok() {
                    out.push((name, buf));
                }
            }
        } else if p.is_file() {
            if let Ok(t) = std::fs::read_to_string(&p) {
                out.push((p.to_string_lossy().to_string(), t));
            }
        }
    }
    Ok(out)
}

// ── Análise ─────────────────────────────────────────────────────────────

fn iso(ts: i64) -> String {
    Utc.timestamp_opt(ts, 0)
        .single()
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

fn day_of(ts: i64) -> i64 {
    ts.div_euclid(DAY)
}

fn sorted_buckets(map: HashMap<String, (u64, u64)>) -> Vec<Bucket> {
    let mut v: Vec<Bucket> = map
        .into_iter()
        .map(|(key, (plays, ms))| Bucket { key, plays, ms })
        .collect();
    v.sort_by(|a, b| a.key.cmp(&b.key));
    v
}

#[derive(Default)]
struct Acc {
    plays: u64,
    ms: u64,
    skips: u64,
    first: i64,
    last: i64,
    first_track: String,
}

/// Todas as contas do histórico. Recebe as escutas já normalizadas para o
/// teste poder montar dados sintéticos sem tocar em arquivo.
pub fn analyze(plays: &[Play], top: usize) -> HistoryResult {
    let mut by_year: HashMap<String, (u64, u64)> = HashMap::new();
    let mut by_month: HashMap<String, (u64, u64)> = HashMap::new();
    let mut weekday = [(0u64, 0u64); 7];
    let mut hour = [(0u64, 0u64); 24];
    let mut artists: HashMap<String, Acc> = HashMap::new();
    let mut tracks: HashMap<(String, String), (u64, u64)> = HashMap::new();
    let mut albums: HashMap<(String, String), (u64, u64)> = HashMap::new();
    let mut year_artist: HashMap<(String, String), u64> = HashMap::new();
    let mut year_track: HashMap<(String, String), u64> = HashMap::new();
    let mut days: HashSet<i64> = HashSet::new();
    let mut total_ms = 0u64;
    let mut first_ts = i64::MAX;
    let mut last_ts = i64::MIN;

    for p in plays {
        total_ms += p.ms;
        first_ts = first_ts.min(p.ts);
        last_ts = last_ts.max(p.ts);
        days.insert(day_of(p.ts));
        let Some(dt) = Utc.timestamp_opt(p.ts, 0).single() else {
            continue;
        };
        let y = dt.format("%Y").to_string();
        let m = dt.format("%Y-%m").to_string();
        let e = by_year.entry(y.clone()).or_default();
        e.0 += 1;
        e.1 += p.ms;
        let e = by_month.entry(m).or_default();
        e.0 += 1;
        e.1 += p.ms;
        let wd = dt.weekday().num_days_from_monday() as usize;
        weekday[wd].0 += 1;
        weekday[wd].1 += p.ms;
        let h = dt.hour() as usize;
        hour[h].0 += 1;
        hour[h].1 += p.ms;

        let a = artists.entry(p.artist.clone()).or_insert_with(|| Acc {
            first: p.ts,
            last: p.ts,
            first_track: p.track.clone(),
            ..Default::default()
        });
        a.plays += 1;
        a.ms += p.ms;
        if p.skipped {
            a.skips += 1;
        }
        if p.ts < a.first {
            a.first = p.ts;
            a.first_track = p.track.clone();
        }
        a.last = a.last.max(p.ts);

        let t = tracks
            .entry((p.track.clone(), p.artist.clone()))
            .or_default();
        t.0 += 1;
        t.1 += p.ms;
        if !p.album.is_empty() {
            let al = albums
                .entry((p.album.clone(), p.artist.clone()))
                .or_default();
            al.0 += 1;
            al.1 += p.ms;
        }
        *year_artist
            .entry((y.clone(), p.artist.clone()))
            .or_default() += p.ms;
        *year_track.entry((y, p.track.clone())).or_default() += p.ms;
    }

    let stat = |name: &str, a: &Acc| ArtistStat {
        name: name.to_string(),
        plays: a.plays,
        ms: a.ms,
        skips: a.skips,
        skip_rate: if a.plays == 0 {
            0
        } else {
            ((a.skips as f64 / a.plays as f64) * 100.0).round() as u32
        },
        first_ts: iso(a.first),
        first_track: a.first_track.clone(),
        last_ts: iso(a.last),
    };

    let mut top_artists: Vec<ArtistStat> = artists.iter().map(|(n, a)| stat(n, a)).collect();
    top_artists.sort_by(|a, b| b.ms.cmp(&a.ms).then_with(|| a.name.cmp(&b.name)));

    // Pulados: só quem tem escuta suficiente para o número dizer algo.
    let mut skipped_artists: Vec<ArtistStat> = top_artists
        .iter()
        .filter(|a| a.plays >= 10)
        .cloned()
        .collect();
    skipped_artists.sort_by(|a, b| {
        b.skip_rate
            .cmp(&a.skip_rate)
            .then_with(|| b.plays.cmp(&a.plays))
    });
    skipped_artists.truncate(top);

    let mut firsts: Vec<ArtistStat> = top_artists
        .iter()
        .filter(|a| a.plays >= 3)
        .cloned()
        .collect();
    firsts.sort_by(|a, b| {
        b.first_ts
            .cmp(&a.first_ts)
            .then_with(|| a.name.cmp(&b.name))
    });
    firsts.truncate(top);

    let mut top_tracks: Vec<TrackStat> = tracks
        .into_iter()
        .map(|((track, artist), (plays, ms))| TrackStat {
            track,
            artist,
            plays,
            ms,
        })
        .collect();
    top_tracks.sort_by(|a, b| b.ms.cmp(&a.ms).then_with(|| a.track.cmp(&b.track)));
    top_tracks.truncate(top);

    let mut top_albums: Vec<TrackStat> = albums
        .into_iter()
        .map(|((track, artist), (plays, ms))| TrackStat {
            track,
            artist,
            plays,
            ms,
        })
        .collect();
    top_albums.sort_by(|a, b| b.ms.cmp(&a.ms).then_with(|| a.track.cmp(&b.track)));
    top_albums.truncate(top);

    // Campeão de cada ano.
    let mut best_a: HashMap<String, (String, u64)> = HashMap::new();
    for ((y, a), ms) in year_artist {
        let e = best_a.entry(y).or_insert_with(|| (a.clone(), 0));
        if ms > e.1 || (ms == e.1 && a < e.0) {
            *e = (a, ms);
        }
    }
    let mut best_t: HashMap<String, (String, u64)> = HashMap::new();
    for ((y, t), ms) in year_track {
        let e = best_t.entry(y).or_insert_with(|| (t.clone(), 0));
        if ms > e.1 || (ms == e.1 && t < e.0) {
            *e = (t, ms);
        }
    }
    let mut year_top: Vec<YearTop> = best_a
        .into_iter()
        .map(|(year, (artist, ms))| YearTop {
            track: best_t.get(&year).map(|x| x.0.clone()).unwrap_or_default(),
            year,
            artist,
            ms,
        })
        .collect();
    year_top.sort_by(|a, b| a.year.cmp(&b.year));

    let streaks = streaks_of(&days);

    // Sumiram: comparação dos últimos 365 dias com os 365 anteriores.
    let mut dropped: Vec<Dropped> = Vec::new();
    if last_ts > i64::MIN {
        let cut = last_ts - 365 * DAY;
        let prev_cut = cut - 365 * DAY;
        let mut before: HashMap<&str, u64> = HashMap::new();
        let mut after: HashMap<&str, u64> = HashMap::new();
        for p in plays {
            if p.ts >= cut {
                *after.entry(p.artist.as_str()).or_default() += p.ms;
            } else if p.ts >= prev_cut {
                *before.entry(p.artist.as_str()).or_default() += p.ms;
            }
        }
        for (name, b) in before {
            let a = after.get(name).copied().unwrap_or(0);
            if b >= 3_600_000 && a * 10 < b {
                dropped.push(Dropped {
                    name: name.to_string(),
                    before_ms: b,
                    after_ms: a,
                    last_ts: artists.get(name).map(|x| iso(x.last)).unwrap_or_default(),
                });
            }
        }
        dropped.sort_by_key(|d| std::cmp::Reverse(d.before_ms - d.after_ms));
        dropped.truncate(top);
    }

    let mut result = HistoryResult {
        files: Vec::new(),
        plays: plays.len() as u64,
        total_ms,
        distinct_artists: artists.len(),
        distinct_tracks: plays
            .iter()
            .map(|p| (p.track.as_str(), p.artist.as_str()))
            .collect::<HashSet<_>>()
            .len(),
        first_ts: if plays.is_empty() {
            String::new()
        } else {
            iso(first_ts)
        },
        last_ts: if plays.is_empty() {
            String::new()
        } else {
            iso(last_ts)
        },
        by_year: sorted_buckets(by_year),
        by_month: sorted_buckets(by_month),
        by_weekday: Vec::new(),
        by_hour: Vec::new(),
        top_artists,
        top_tracks,
        top_albums,
        year_top,
        skipped_artists,
        firsts,
        streaks,
        dropped,
        exports: Vec::new(),
    };
    result.by_weekday = weekday
        .iter()
        .enumerate()
        .map(|(i, (plays, ms))| Bucket {
            key: i.to_string(),
            plays: *plays,
            ms: *ms,
        })
        .collect();
    result.by_hour = hour
        .iter()
        .enumerate()
        .map(|(i, (plays, ms))| Bucket {
            key: i.to_string(),
            plays: *plays,
            ms: *ms,
        })
        .collect();
    result.top_artists.truncate(top);
    result
}

/// Dias seguidos com pelo menos uma escuta.
pub fn streaks_of(days: &HashSet<i64>) -> Streaks {
    let mut v: Vec<i64> = days.iter().copied().collect();
    v.sort_unstable();
    let mut best = (0u64, 0i64, 0i64);
    let mut cur = 0u64;
    let mut start = 0i64;
    for (i, d) in v.iter().enumerate() {
        if i > 0 && *d == v[i - 1] + 1 {
            cur += 1;
        } else {
            cur = 1;
            start = *d;
        }
        if cur > best.0 {
            best = (cur, start, *d);
        }
    }
    // A sequência corrente é a que termina no último dia com escuta.
    let mut current = 0u64;
    for (i, d) in v.iter().enumerate().rev() {
        if i + 1 == v.len() || v[i + 1] == *d + 1 {
            current += 1;
        } else {
            break;
        }
    }
    Streaks {
        longest_days: best.0,
        longest_start: if best.0 == 0 {
            String::new()
        } else {
            iso(best.1 * DAY)
        },
        longest_end: if best.0 == 0 {
            String::new()
        } else {
            iso(best.2 * DAY)
        },
        current_days: current,
        active_days: v.len() as u64,
    }
}

// ── Export ──────────────────────────────────────────────────────────────

fn csv_cell(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn hours(ms: u64) -> f64 {
    (ms as f64 / 3_600_000.0 * 10.0).round() / 10.0
}

pub fn artists_csv(r: &HistoryResult) -> String {
    let mut s = String::from("artista,plays,horas,pulos,skip_rate,primeira,ultima\n");
    for a in &r.top_artists {
        s.push_str(&format!(
            "{},{},{},{},{},{},{}\n",
            csv_cell(&a.name),
            a.plays,
            hours(a.ms),
            a.skips,
            a.skip_rate,
            a.first_ts,
            a.last_ts
        ));
    }
    s
}

pub fn tracks_csv(r: &HistoryResult) -> String {
    let mut s = String::from("faixa,artista,plays,horas\n");
    for t in &r.top_tracks {
        s.push_str(&format!(
            "{},{},{},{}\n",
            csv_cell(&t.track),
            csv_cell(&t.artist),
            t.plays,
            hours(t.ms)
        ));
    }
    s
}

pub fn months_csv(r: &HistoryResult) -> String {
    let mut s = String::from("mes,plays,horas\n");
    for b in &r.by_month {
        s.push_str(&format!("{},{},{}\n", b.key, b.plays, hours(b.ms)));
    }
    s
}

/// Relatório em Markdown para o usuário guardar ou colar em algum lugar.
pub fn report_markdown(r: &HistoryResult) -> String {
    let mut s = String::new();
    s.push_str("# Histórico do Spotify\n\n");
    s.push_str(&format!(
        "- Período: {} a {}\n- Escutas: {}\n- Horas: {}\n- Artistas: {}\n- Faixas: {}\n- Dias com música: {} (maior sequência: {} dias)\n\n",
        r.first_ts, r.last_ts, r.plays, hours(r.total_ms), r.distinct_artists, r.distinct_tracks,
        r.streaks.active_days, r.streaks.longest_days
    ));
    s.push_str("## Por ano\n\n| Ano | Horas | Escutas |\n| --- | ---: | ---: |\n");
    for b in &r.by_year {
        s.push_str(&format!("| {} | {} | {} |\n", b.key, hours(b.ms), b.plays));
    }
    s.push_str("\n## Top artistas\n\n| # | Artista | Horas | Escutas | Pulos |\n| ---: | --- | ---: | ---: | ---: |\n");
    for (i, a) in r.top_artists.iter().enumerate() {
        s.push_str(&format!(
            "| {} | {} | {} | {} | {}% |\n",
            i + 1,
            a.name,
            hours(a.ms),
            a.plays,
            a.skip_rate
        ));
    }
    s.push_str("\n## Top faixas\n\n| # | Faixa | Artista | Horas |\n| ---: | --- | --- | ---: |\n");
    for (i, t) in r.top_tracks.iter().enumerate() {
        s.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            i + 1,
            t.track,
            t.artist,
            hours(t.ms)
        ));
    }
    if !r.dropped.is_empty() {
        s.push_str("\n## Sumiram\n\n| Artista | Antes (h) | Depois (h) | Última vez |\n| --- | ---: | ---: | --- |\n");
        for d in &r.dropped {
            s.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                d.name,
                hours(d.before_ms),
                hours(d.after_ms),
                d.last_ts
            ));
        }
    }
    s
}

fn write_exports(r: &mut HistoryResult, out_dir: &Path, formats: &[String]) -> Result<()> {
    if formats.is_empty() {
        return Ok(());
    }
    std::fs::create_dir_all(out_dir)?;
    for f in formats {
        match f.as_str() {
            "json" => {
                let p = out_dir.join("historico-spotify.json");
                std::fs::write(&p, serde_json::to_vec_pretty(&r)?)?;
                r.exports.push(p.to_string_lossy().to_string());
            }
            "csv" => {
                for (name, body) in [
                    ("historico-artistas.csv", artists_csv(r)),
                    ("historico-faixas.csv", tracks_csv(r)),
                    ("historico-meses.csv", months_csv(r)),
                ] {
                    let p = out_dir.join(name);
                    std::fs::write(&p, body)?;
                    r.exports.push(p.to_string_lossy().to_string());
                }
            }
            "md" => {
                let p = out_dir.join("historico-spotify.md");
                std::fs::write(&p, report_markdown(r))?;
                r.exports.push(p.to_string_lossy().to_string());
            }
            _ => {}
        }
    }
    Ok(())
}

pub fn run(opts: &HistoryOptions, p: &ProgressFn) -> Result<HistoryResult> {
    report(p, TOOL_ID, "started", 0, None, None);
    let sources = collect_sources(&opts.inputs)?;
    if sources.is_empty() {
        anyhow::bail!("não achei nenhum StreamingHistory_*.json / Streaming_History_Audio_*.json");
    }
    let total = sources.len() as u64;
    let mut plays = Vec::new();
    let mut files = Vec::new();
    for (i, (name, text)) in sources.iter().enumerate() {
        report(
            p,
            TOOL_ID,
            "progress",
            i as u64 + 1,
            Some(total),
            Some(name.clone()),
        );
        let mut got = parse_history(text, opts.min_ms);
        if !got.is_empty() {
            files.push(name.clone());
            plays.append(&mut got);
        }
    }
    if plays.is_empty() {
        anyhow::bail!("os arquivos não tinham nenhuma escuta de música");
    }
    plays.sort_by_key(|x| x.ts);

    let mut result = analyze(&plays, opts.top.max(1));
    result.files = files;
    if let Some(dir) = &opts.out_dir {
        if !dir.is_empty() {
            write_exports(&mut result, Path::new(dir), &opts.formats)?;
        }
    }
    report(p, TOOL_ID, "done", total, Some(total), None);
    Ok(result)
}

/// Conveniência para a UI: quantas horas um bloco representa.
pub fn ms_to_hours(ms: u64) -> f64 {
    hours(ms)
}

/// Data (YYYY-MM-DD) de um timestamp, exposta para os testes e para o front.
pub fn day_string(ts: i64) -> String {
    iso(ts)
}

/// Timestamp de uma data ISO, para montar fixture sem depender de fuso.
pub fn ts_of(date: &str, hour: u32) -> i64 {
    NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(hour, 0, 0))
        .map(|d| d.and_utc().timestamp())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::tools::noop_progress;

    fn play(date: &str, hour: u32, artist: &str, track: &str, ms: u64, skipped: bool) -> Play {
        Play {
            ts: ts_of(date, hour),
            artist: artist.to_string(),
            track: track.to_string(),
            album: format!("{artist} album"),
            ms,
            skipped,
            shuffle: None,
            platform: None,
            country: None,
            reason_end: None,
        }
    }

    #[test]
    fn le_formato_antigo() {
        let json = r#"[
          {"endTime":"2021-05-13 14:33","artistName":"Radiohead","trackName":"Creep","msPlayed":238000},
          {"endTime":"2021-05-13 14:34","artistName":"Radiohead","trackName":"No Surprises","msPlayed":9000}
        ]"#;
        let plays = parse_history(json, 30_000);
        assert_eq!(plays.len(), 2);
        assert_eq!(plays[0].artist, "Radiohead");
        assert_eq!(plays[0].ms, 238_000);
        assert!(!plays[0].skipped);
        // Sem o campo `skipped`, o corte é o tempo tocado.
        assert!(plays[1].skipped);
        assert_eq!(day_string(plays[0].ts), "2021-05-13");
    }

    #[test]
    fn le_formato_estendido() {
        let json = r#"[
          {"ts":"2016-02-03T18:20:11Z","ms_played":210000,
           "master_metadata_track_name":"Paranoid Android",
           "master_metadata_album_artist_name":"Radiohead",
           "master_metadata_album_album_name":"OK Computer",
           "spotify_track_uri":"spotify:track:x","reason_start":"trackdone",
           "reason_end":"trackdone","skipped":false,"shuffle":true,
           "platform":"android","conn_country":"BR"},
          {"ts":"2016-02-03T18:24:00Z","ms_played":120000,
           "master_metadata_track_name":"Karma Police",
           "master_metadata_album_artist_name":"Radiohead",
           "skipped":true,"reason_end":"fwdbtn","platform":"android"},
          {"ts":"2016-02-03T19:00:00Z","ms_played":900000,
           "episode_name":"Podcast qualquer"}
        ]"#;
        let plays = parse_history(json, 30_000);
        // O podcast (sem nome de faixa) fica de fora.
        assert_eq!(plays.len(), 2);
        assert_eq!(plays[0].album, "OK Computer");
        assert_eq!(plays[0].country.as_deref(), Some("BR"));
        assert_eq!(plays[0].shuffle, Some(true));
        assert!(plays[1].skipped);
    }

    #[test]
    fn metricas_com_dados_sinteticos() {
        // 2 dias seguidos + 1 dia solto depois.
        let plays = vec![
            play("2023-01-02", 8, "A", "a1", 200_000, false), // segunda
            play("2023-01-02", 9, "A", "a2", 100_000, false),
            play("2023-01-03", 8, "B", "b1", 300_000, false), // terça
            play("2023-01-10", 22, "A", "a1", 400_000, true),
        ];
        let r = analyze(&plays, 10);
        assert_eq!(r.plays, 4);
        assert_eq!(r.total_ms, 1_000_000);
        assert_eq!(r.distinct_artists, 2);
        assert_eq!(r.distinct_tracks, 3);
        assert_eq!(r.first_ts, "2023-01-02");
        assert_eq!(r.last_ts, "2023-01-10");
        assert_eq!(r.by_year.len(), 1);
        assert_eq!(r.by_year[0].key, "2023");
        assert_eq!(r.by_year[0].ms, 1_000_000);
        assert_eq!(r.by_month[0].key, "2023-01");
        // Segunda = índice 0. 2023-01-03 e 2023-01-10 caem numa terça.
        assert_eq!(r.by_weekday[0].plays, 2);
        assert_eq!(r.by_weekday[1].plays, 2);
        assert_eq!(r.by_weekday[2].plays, 0);
        assert_eq!(r.by_hour[8].plays, 2);
        assert_eq!(r.by_hour[22].plays, 1);
        // A é o artista com mais tempo.
        assert_eq!(r.top_artists[0].name, "A");
        assert_eq!(r.top_artists[0].ms, 700_000);
        assert_eq!(r.top_artists[0].plays, 3);
        assert_eq!(r.top_artists[0].skips, 1);
        assert_eq!(r.top_artists[0].skip_rate, 33);
        assert_eq!(r.top_artists[0].first_ts, "2023-01-02");
        assert_eq!(r.top_artists[0].first_track, "a1");
        assert_eq!(r.top_tracks[0].track, "a1");
        assert_eq!(r.top_tracks[0].ms, 600_000);
        assert_eq!(r.top_albums[0].track, "A album");
        assert_eq!(r.year_top[0].artist, "A");
        assert_eq!(r.streaks.longest_days, 2);
        assert_eq!(r.streaks.longest_start, "2023-01-02");
        assert_eq!(r.streaks.longest_end, "2023-01-03");
        assert_eq!(r.streaks.current_days, 1);
        assert_eq!(r.streaks.active_days, 3);
    }

    #[test]
    fn artistas_que_sumiram() {
        let mut plays = Vec::new();
        // Ouvia muito em 2022, nada em 2023.
        for d in 1..=20 {
            plays.push(play(
                &format!("2022-03-{d:02}"),
                12,
                "Sumiu",
                "x",
                600_000,
                false,
            ));
        }
        // E continua ouvindo o outro.
        for d in 1..=20 {
            plays.push(play(
                &format!("2023-06-{d:02}"),
                12,
                "Continua",
                "y",
                600_000,
                false,
            ));
            plays.push(play(
                &format!("2022-06-{d:02}"),
                12,
                "Continua",
                "y",
                600_000,
                false,
            ));
        }
        let r = analyze(&plays, 10);
        let nomes: Vec<&str> = r.dropped.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(nomes, vec!["Sumiu"]);
        assert_eq!(r.dropped[0].after_ms, 0);
        assert_eq!(r.dropped[0].last_ts, "2022-03-20");
    }

    #[test]
    fn sequencia_de_dias() {
        let days: HashSet<i64> = [0i64, 1, 2, 3, 10, 11, 20]
            .into_iter()
            .collect::<HashSet<_>>();
        let s = streaks_of(&days);
        assert_eq!(s.longest_days, 4);
        assert_eq!(s.active_days, 7);
        assert_eq!(s.current_days, 1);
    }

    #[test]
    fn exports_csv_e_markdown() {
        let plays = vec![play("2023-01-02", 8, "A, Inc", "a\"1", 200_000, false)];
        let r = analyze(&plays, 10);
        let csv = artists_csv(&r);
        assert!(csv.starts_with("artista,plays,horas,pulos,skip_rate,primeira,ultima\n"));
        assert!(csv.contains("\"A, Inc\""));
        assert!(tracks_csv(&r).contains("\"a\"\"1\""));
        assert!(months_csv(&r).contains("2023-01,1,0.1"));
        let md = report_markdown(&r);
        assert!(md.contains("# Histórico do Spotify"));
        assert!(md.contains("| 2023 |"));
    }

    #[test]
    fn run_le_pasta_com_os_dois_formatos() {
        let dir = crate::core::tools::temp_dir().join(format!("mhist-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(
            dir.join("StreamingHistory_music_0.json"),
            r#"[{"endTime":"2021-05-13 14:33","artistName":"A","trackName":"a1","msPlayed":200000}]"#,
        )
        .expect("w1");
        std::fs::write(
            dir.join("Streaming_History_Audio_2016_1.json"),
            r#"[{"ts":"2016-02-03T18:20:11Z","ms_played":210000,"master_metadata_track_name":"b1","master_metadata_album_artist_name":"B"}]"#,
        )
        .expect("w2");
        let opts = HistoryOptions {
            inputs: vec![dir.to_string_lossy().to_string()],
            min_ms: 30_000,
            out_dir: Some(dir.to_string_lossy().to_string()),
            formats: vec!["json".into(), "csv".into(), "md".into()],
            top: 10,
        };
        let r = run(&opts, &noop_progress()).expect("run");
        assert_eq!(r.plays, 2);
        assert_eq!(r.files.len(), 2);
        assert_eq!(r.exports.len(), 5);
        assert_eq!(r.by_year.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
