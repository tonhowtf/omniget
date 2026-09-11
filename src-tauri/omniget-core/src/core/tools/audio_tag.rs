//! Etiquetas de música (ID3v2 / Vorbis Comments / MP4 ilst) em Rust puro.
//!
//! Toda a leitura e escrita passa pelo `lofty`, que normaliza os três mundos
//! num `Tag` genérico: o mesmo código serve MP3, FLAC, M4A, OGG, Opus e WAV,
//! sem ffmpeg e sem binário externo.
//!
//! São três etapas, na ordem em que a UI usa:
//!
//! 1. `scan` — lê uma pasta (ou uma lista de arquivos) e devolve uma tabela
//!    com formato, duração, bitrate e os campos de cada faixa.
//! 2. diagnóstico — `diagnose` aponta o que está errado em cada arquivo:
//!    campo obrigatório vazio, título e artista trocados, faixa gravada como
//!    "3/12" em texto, ano inválido, mojibake (UTF-8 lido como latin-1),
//!    capa faltando, capa gigante, capa duplicada, tag de formato errado.
//! 3. `edit` — aplica as correções em lote. O padrão é `dry_run`: a função
//!    devolve o diff campo a campo e NÃO toca no arquivo. Só grava quando o
//!    usuário manda `dry_run: false`.
//!
//! Não há identificação por impressão digital (AcoustID/MusicBrainz) aqui:
//! isso mandaria dado do usuário para fora da máquina e ficou de fora de
//! propósito.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context};
use lofty::config::WriteOptions as LoftyWriteOptions;
use lofty::file::{AudioFile, FileType, TaggedFileExt};
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::prelude::{Accessor, ItemKey};
use lofty::tag::{Tag, TagType};
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use super::{report, ProgressFn};

/// Identificador que a UI escuta no evento de progresso.
const TOOL_ID: &str = "audio-tag";

/// Acima disso a capa embutida atrapalha mais do que ajuda: infla cada
/// arquivo do álbum e trava aparelho antigo.
const COVER_HUGE_BYTES: u64 = 2 * 1024 * 1024;

/// Extensões que o `lofty` sabe abrir.
const AUDIO_EXTS: &[&str] = &[
    "mp3", "m4a", "m4b", "m4p", "mp4", "flac", "ogg", "oga", "opus", "spx", "wav", "wave", "aiff",
    "aif", "aifc", "aac", "ape", "wv", "mpc", "mp2", "mp1",
];

// ─────────────────────────── entrada e saída ────────────────────────────

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ScanOptions {
    /// Pastas a varrer.
    #[serde(default)]
    pub dirs: Vec<String>,
    /// Arquivos avulsos (somados às pastas).
    #[serde(default)]
    pub files: Vec<String>,
    /// Entrar nas subpastas.
    #[serde(default)]
    pub recursive: bool,
    /// Teto de arquivos, para não travar a UI numa biblioteca inteira.
    #[serde(default)]
    pub max_files: Option<usize>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct TrackTags {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album_artist: Option<String>,
    pub album: Option<String>,
    pub year: Option<String>,
    pub track: Option<u32>,
    pub track_total: Option<u32>,
    pub disc: Option<u32>,
    pub disc_total: Option<u32>,
    pub genre: Option<String>,
    pub comment: Option<String>,
    /// O que está gravado no campo de faixa, sem interpretar ("3/12").
    pub track_raw: Option<String>,
    /// Idem para o disco.
    pub disc_raw: Option<String>,
}

/// Um problema achado num arquivo. `fixable` diz se o `edit` desta mesma
/// tool resolve sozinho.
#[derive(Debug, Clone, Serialize)]
pub struct Issue {
    pub code: String,
    pub field: Option<String>,
    pub severity: String,
    pub detail: String,
    pub fixable: bool,
}

impl Issue {
    fn new(code: &str, field: Option<&str>, severity: &str, detail: String, fixable: bool) -> Self {
        Self {
            code: code.to_string(),
            field: field.map(str::to_string),
            severity: severity.to_string(),
            detail,
            fixable,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CoverInfo {
    pub count: usize,
    pub bytes: u64,
    pub mime: Option<String>,
    /// Quantas capas frontais existem (mais de uma é duplicata).
    pub front_count: usize,
    /// Duas imagens com bytes idênticos dentro da mesma tag.
    pub duplicated: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrackRow {
    pub path: String,
    pub file_name: String,
    pub format: String,
    /// Tag lida, no vocabulário do lofty ("Id3v2", "VorbisComments"…).
    pub tag_type: Option<String>,
    /// Tag que o contêiner espera.
    pub primary_tag_type: String,
    pub bytes: u64,
    pub duration_secs: f64,
    pub bitrate_kbps: Option<u32>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u8>,
    pub tags: TrackTags,
    pub cover: CoverInfo,
    pub issues: Vec<Issue>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScanResult {
    pub scanned: usize,
    pub failed: usize,
    /// Arquivos com pelo menos um problema.
    pub with_issues: usize,
    /// Contagem por código de problema, para a UI mostrar o resumo.
    pub issue_counts: BTreeMap<String, usize>,
    pub tracks: Vec<TrackRow>,
}

/// Uma mudança de campo, do jeito que o `dry_run` mostra ao usuário.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FieldDiff {
    pub field: String,
    pub from: Option<String>,
    pub to: Option<String>,
    /// Qual operação pediu a mudança ("set", "renumber", "mojibake"…).
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CoverDiff {
    /// "embed" | "remove" | "dedupe"
    pub action: String,
    pub from_bytes: u64,
    pub to_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileChange {
    pub path: String,
    pub file_name: String,
    pub diffs: Vec<FieldDiff>,
    pub cover: Option<CoverDiff>,
    /// Campos que o formato do arquivo não aceita (ex.: ano em AIFF).
    pub unsupported: Vec<String>,
    pub written: bool,
    pub backup: Option<String>,
    pub error: Option<String>,
}

fn default_true() -> bool {
    true
}

fn default_one() -> u32 {
    1
}

/// Edição em lote. O padrão é não gravar: `dry_run` só vira `false` quando o
/// front manda o campo explicitamente.
#[derive(Debug, Clone, Deserialize)]
pub struct EditOptions {
    /// Ordem importa: é ela que a renumeração usa quando `sort_by_name` é
    /// falso.
    pub files: Vec<String>,
    /// Campo → valor. String vazia limpa o campo.
    #[serde(default)]
    pub set: BTreeMap<String, String>,
    /// Campos a limpar.
    #[serde(default)]
    pub clear: Vec<String>,
    /// Reordenar a lista pelo nome do arquivo antes de renumerar.
    #[serde(default = "default_true")]
    pub sort_by_name: bool,
    /// Regravar a faixa como 1, 2, 3… na ordem da lista.
    #[serde(default)]
    pub renumber: bool,
    #[serde(default = "default_one")]
    pub renumber_start: u32,
    /// Escrever também o total de faixas.
    #[serde(default)]
    pub set_track_total: bool,
    /// Copiar o artista para o artista do álbum quando este estiver vazio.
    #[serde(default)]
    pub album_artist_from_artist: bool,
    /// Consertar texto UTF-8 que foi lido como latin-1.
    #[serde(default)]
    pub fix_mojibake: bool,
    /// Quebrar "3/12" em faixa 3 e total 12.
    #[serde(default)]
    pub split_track_slash: bool,
    /// Trocar título e artista de lugar.
    #[serde(default)]
    pub swap_title_artist: bool,
    /// Imagem a embutir como capa frontal.
    #[serde(default)]
    pub cover_path: Option<String>,
    /// Tirar todas as capas.
    #[serde(default)]
    pub remove_cover: bool,
    /// Deixar só uma capa quando houver repetida.
    #[serde(default)]
    pub dedupe_cover: bool,
    /// Copiar o arquivo para `<nome>.bak` antes de gravar.
    #[serde(default)]
    pub backup: bool,
    #[serde(default = "default_true")]
    pub dry_run: bool,
}

impl Default for EditOptions {
    fn default() -> Self {
        Self {
            files: Vec::new(),
            set: BTreeMap::new(),
            clear: Vec::new(),
            sort_by_name: true,
            renumber: false,
            renumber_start: 1,
            set_track_total: false,
            album_artist_from_artist: false,
            fix_mojibake: false,
            split_track_slash: false,
            swap_title_artist: false,
            cover_path: None,
            remove_cover: false,
            dedupe_cover: false,
            backup: false,
            dry_run: true,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct EditResult {
    pub dry_run: bool,
    /// Arquivos com pelo menos uma mudança planejada.
    pub changed: usize,
    pub written: usize,
    pub failed: usize,
    pub files: Vec<FileChange>,
}

// ───────────────────────────── campos ───────────────────────────────────

/// Campos que a tool edita, na ordem em que a tabela mostra.
pub const FIELDS: &[&str] = &[
    "title",
    "artist",
    "album_artist",
    "album",
    "year",
    "track",
    "track_total",
    "disc",
    "disc_total",
    "genre",
    "comment",
];

/// Campos de texto livre — os que podem sofrer mojibake.
const TEXT_FIELDS: &[&str] = &[
    "title",
    "artist",
    "album_artist",
    "album",
    "genre",
    "comment",
];

fn item_key(field: &str) -> Option<ItemKey> {
    Some(match field {
        "title" => ItemKey::TrackTitle,
        "artist" => ItemKey::TrackArtist,
        "album_artist" => ItemKey::AlbumArtist,
        "album" => ItemKey::AlbumTitle,
        "year" => ItemKey::Year,
        "track" => ItemKey::TrackNumber,
        "track_total" => ItemKey::TrackTotal,
        "disc" => ItemKey::DiscNumber,
        "disc_total" => ItemKey::DiscTotal,
        "genre" => ItemKey::Genre,
        "comment" => ItemKey::Comment,
        _ => return None,
    })
}

// ───────────────────────── helpers puros (testados) ─────────────────────

/// Byte que gerou este caractere quando bytes CP1252/latin-1 viraram texto.
fn cp1252_byte(c: char) -> Option<u8> {
    let code = c as u32;
    if code < 0x80 {
        return Some(code as u8);
    }
    // Faixa C1: no CP1252 esses bytes são pontuação tipográfica.
    const C1: &[(char, u8)] = &[
        ('\u{20AC}', 0x80),
        ('\u{201A}', 0x82),
        ('\u{0192}', 0x83),
        ('\u{201E}', 0x84),
        ('\u{2026}', 0x85),
        ('\u{2020}', 0x86),
        ('\u{2021}', 0x87),
        ('\u{02C6}', 0x88),
        ('\u{2030}', 0x89),
        ('\u{0160}', 0x8A),
        ('\u{2039}', 0x8B),
        ('\u{0152}', 0x8C),
        ('\u{017D}', 0x8E),
        ('\u{2018}', 0x91),
        ('\u{2019}', 0x92),
        ('\u{201C}', 0x93),
        ('\u{201D}', 0x94),
        ('\u{2022}', 0x95),
        ('\u{2013}', 0x96),
        ('\u{2014}', 0x97),
        ('\u{02DC}', 0x98),
        ('\u{2122}', 0x99),
        ('\u{0161}', 0x9A),
        ('\u{203A}', 0x9B),
        ('\u{0153}', 0x9C),
        ('\u{017E}', 0x9E),
        ('\u{0178}', 0x9F),
    ];
    if let Some((_, b)) = C1.iter().find(|(ch, _)| *ch == c) {
        return Some(*b);
    }
    if (0xA0..=0xFF).contains(&code) {
        return Some(code as u8);
    }
    None
}

/// Texto UTF-8 que foi decodificado como latin-1/CP1252 ("MÃºsica").
/// Devolve o texto consertado, ou `None` quando não é mojibake.
///
/// O teste é o próprio round-trip: só devolve algo se cada caractere couber
/// num byte CP1252 **e** a sequência resultante for UTF-8 válido. Texto que
/// já está certo ("Música") não sobrevive a esse round-trip, então não há
/// falso positivo silencioso.
pub fn fix_mojibake(s: &str) -> Option<String> {
    if s.is_ascii() || s.is_empty() {
        return None;
    }
    let mut bytes = Vec::with_capacity(s.len());
    for c in s.chars() {
        bytes.push(cp1252_byte(c)?);
    }
    let fixed = String::from_utf8(bytes).ok()?;
    if fixed == s || fixed.is_empty() || fixed.is_ascii() {
        return None;
    }
    if fixed
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return None;
    }
    Some(fixed)
}

/// Texto que perdeu bytes na conversão (virou `U+FFFD`) — dá para avisar,
/// mas não dá para consertar: a informação não existe mais no arquivo.
pub fn has_lost_bytes(s: &str) -> bool {
    s.contains('\u{FFFD}')
}

/// "3/12", "3 of 12", "03-12", "3" → (faixa, total).
pub fn parse_track_pair(s: &str) -> (Option<u32>, Option<u32>) {
    let lower = s.trim().to_lowercase();
    let norm = lower.replace(" of ", "/").replace(" de ", "/");
    let mut it = norm.splitn(2, ['/', '\\', '-']);
    let first = it.next().unwrap_or_default();
    let second = it.next();
    (leading_number(first), second.and_then(leading_number))
}

fn leading_number(s: &str) -> Option<u32> {
    let digits: String = s.trim().chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse::<u32>().ok()
}

/// Ano plausível dentro de um campo de data. Aceita "1998",
/// "1998-05-03", "1998-05-03T10:00:00" e "03/05/1998"; recusa "98", "0000"
/// e qualquer coisa sem quatro dígitos seguidos.
pub fn normalize_year(s: &str) -> Option<u16> {
    let chars: Vec<char> = s.trim().chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        if !chars[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && chars[i].is_ascii_digit() {
            i += 1;
        }
        if i - start == 4 {
            let run: String = chars[start..i].iter().collect();
            if let Ok(y) = run.parse::<u16>() {
                if (1000..=2999).contains(&y) {
                    return Some(y);
                }
            }
        }
    }
    None
}

/// Tira "01 - " / "01. " / "01 " do começo do nome do arquivo.
fn strip_leading_number(s: &str) -> &str {
    let t = s.trim_start();
    let digits = t.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 || digits > 3 {
        return t;
    }
    let rest = t[digits..].trim_start();
    rest.strip_prefix('-')
        .or_else(|| rest.strip_prefix('.'))
        .or_else(|| rest.strip_prefix('_'))
        .map(str::trim_start)
        .unwrap_or_else(|| if rest.len() < t.len() { rest } else { t })
}

fn loose_eq(a: &str, b: &str) -> bool {
    let norm = |s: &str| {
        s.trim()
            .to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>()
    };
    let (a, b) = (norm(a), norm(b));
    !a.is_empty() && a == b
}

/// Nome de arquivo no padrão "Artista - Título" que bate ao contrário com o
/// que está gravado na tag: título e artista foram trocados.
pub fn looks_swapped(file_stem: &str, title: Option<&str>, artist: Option<&str>) -> bool {
    let (Some(title), Some(artist)) = (title, artist) else {
        return false;
    };
    let stem = strip_leading_number(file_stem);
    let Some((left, right)) = stem.split_once(" - ") else {
        return false;
    };
    if right.contains(" - ") {
        return false;
    }
    // O nome diz "esquerda = artista, direita = título". Se a tag guarda o
    // contrário, e não é o caso de os dois serem iguais, está trocado.
    loose_eq(left, title) && loose_eq(right, artist) && !loose_eq(title, artist)
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum NatPart {
    Num(u64),
    Text(String),
}

/// Chave de ordenação "natural": "track2" vem antes de "track10".
fn natural_key(name: &str) -> Vec<NatPart> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut digits = String::new();
    for c in name.to_lowercase().chars() {
        if c.is_ascii_digit() {
            if !buf.is_empty() {
                out.push(NatPart::Text(std::mem::take(&mut buf)));
            }
            digits.push(c);
        } else {
            if !digits.is_empty() {
                let n = digits.parse::<u64>().unwrap_or(u64::MAX);
                digits.clear();
                out.push(NatPart::Num(n));
            }
            buf.push(c);
        }
    }
    if !digits.is_empty() {
        out.push(NatPart::Num(digits.parse::<u64>().unwrap_or(u64::MAX)));
    }
    if !buf.is_empty() {
        out.push(NatPart::Text(buf));
    }
    out
}

fn file_name_of(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string())
}

/// Ordena caminhos pelo nome do arquivo, em ordem natural.
pub fn sort_paths_by_name(paths: &mut [String]) {
    paths.sort_by(|a, b| {
        let (ka, kb) = (natural_key(&file_name_of(a)), natural_key(&file_name_of(b)));
        ka.cmp(&kb).then_with(|| a.cmp(b))
    });
}

// ───────────────────────────── leitura ──────────────────────────────────

fn format_label(ft: FileType) -> String {
    match ft {
        FileType::Aac => "AAC".into(),
        FileType::Aiff => "AIFF".into(),
        FileType::Ape => "APE".into(),
        FileType::Flac => "FLAC".into(),
        FileType::Mpeg => "MP3".into(),
        FileType::Mp4 => "MP4".into(),
        FileType::Mpc => "Musepack".into(),
        FileType::Opus => "Opus".into(),
        FileType::Vorbis => "Ogg Vorbis".into(),
        FileType::Speex => "Speex".into(),
        FileType::Wav => "WAV".into(),
        FileType::WavPack => "WavPack".into(),
        other => format!("{other:?}"),
    }
}

fn tag_label(tt: TagType) -> String {
    format!("{tt:?}")
}

fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| AUDIO_EXTS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

fn text_of(tag: &Tag, key: ItemKey) -> Option<String> {
    tag.get_string(key)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn read_tags(tag: &Tag) -> TrackTags {
    let track_raw = text_of(tag, ItemKey::TrackNumber);
    let disc_raw = text_of(tag, ItemKey::DiscNumber);
    let (track_from_raw, total_from_raw) = track_raw
        .as_deref()
        .map(parse_track_pair)
        .unwrap_or((None, None));
    let (disc_from_raw, disc_total_from_raw) = disc_raw
        .as_deref()
        .map(parse_track_pair)
        .unwrap_or((None, None));

    TrackTags {
        title: text_of(tag, ItemKey::TrackTitle),
        artist: text_of(tag, ItemKey::TrackArtist),
        album_artist: text_of(tag, ItemKey::AlbumArtist),
        album: text_of(tag, ItemKey::AlbumTitle),
        year: text_of(tag, ItemKey::Year).or_else(|| text_of(tag, ItemKey::RecordingDate)),
        track: tag.track().or(track_from_raw),
        track_total: tag
            .track_total()
            .or_else(|| text_of(tag, ItemKey::TrackTotal).and_then(|s| leading_number(&s)))
            .or(total_from_raw),
        disc: tag.disk().or(disc_from_raw),
        disc_total: tag
            .disk_total()
            .or_else(|| text_of(tag, ItemKey::DiscTotal).and_then(|s| leading_number(&s)))
            .or(disc_total_from_raw),
        genre: text_of(tag, ItemKey::Genre),
        comment: text_of(tag, ItemKey::Comment),
        track_raw,
        disc_raw,
    }
}

fn read_cover(tag: &Tag) -> CoverInfo {
    let pics = tag.pictures();
    let biggest = pics.iter().max_by_key(|p| p.data().len());
    let mut seen: HashSet<&[u8]> = HashSet::new();
    let mut duplicated = false;
    for p in pics {
        if !seen.insert(p.data()) {
            duplicated = true;
        }
    }
    CoverInfo {
        count: pics.len(),
        bytes: biggest.map(|p| p.data().len() as u64).unwrap_or(0),
        mime: biggest
            .and_then(|p| p.mime_type())
            .map(|m| m.as_str().to_string()),
        front_count: pics
            .iter()
            .filter(|p| p.pic_type() == PictureType::CoverFront)
            .count(),
        duplicated,
    }
}

fn empty_cover() -> CoverInfo {
    CoverInfo {
        count: 0,
        bytes: 0,
        mime: None,
        front_count: 0,
        duplicated: false,
    }
}

/// Lê um arquivo. Erro de leitura vira uma linha com `error` preenchido —
/// arquivo corrompido no meio da pasta é caso normal, não motivo de parar.
pub fn read_track(path: &Path) -> TrackRow {
    let path_str = path.to_string_lossy().to_string();
    let file_name = file_name_of(&path_str);
    let bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

    let mut row = TrackRow {
        path: path_str,
        file_name,
        format: String::new(),
        tag_type: None,
        primary_tag_type: String::new(),
        bytes,
        duration_secs: 0.0,
        bitrate_kbps: None,
        sample_rate: None,
        channels: None,
        tags: TrackTags::default(),
        cover: empty_cover(),
        issues: Vec::new(),
        error: None,
    };

    let tagged = match lofty::read_from_path(path) {
        Ok(t) => t,
        Err(e) => {
            row.error = Some(e.to_string());
            return row;
        }
    };

    let props = tagged.properties();
    row.format = format_label(tagged.file_type());
    row.primary_tag_type = tag_label(tagged.primary_tag_type());
    row.duration_secs = props.duration().as_secs_f64();
    row.bitrate_kbps = props.audio_bitrate().or_else(|| props.overall_bitrate());
    row.sample_rate = props.sample_rate();
    row.channels = props.channels();

    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
    if let Some(tag) = tag {
        row.tag_type = Some(tag_label(tag.tag_type()));
        row.tags = read_tags(tag);
        row.cover = read_cover(tag);
    }

    let other_tags: Vec<TagType> = tagged
        .tags()
        .iter()
        .map(Tag::tag_type)
        .filter(|t| *t != tagged.primary_tag_type())
        .collect();
    row.issues = diagnose(&row, tagged.primary_tag().is_none(), &other_tags);
    row
}

// ─────────────────────────── diagnóstico ────────────────────────────────

/// Lista de problemas de uma faixa. `no_primary` diz que o arquivo tem tag,
/// mas não a que o contêiner espera; `other_tags` são as tags "estrangeiras"
/// que ficaram no arquivo.
pub fn diagnose(row: &TrackRow, no_primary: bool, other_tags: &[TagType]) -> Vec<Issue> {
    let mut out = Vec::new();
    let t = &row.tags;

    if row.tag_type.is_none() {
        out.push(Issue::new(
            "no_tag",
            None,
            "error",
            "arquivo sem nenhuma tag".into(),
            false,
        ));
        return out;
    }

    for (field, value) in [
        ("title", &t.title),
        ("artist", &t.artist),
        ("album", &t.album),
    ] {
        if value.is_none() {
            out.push(Issue::new(
                "missing_field",
                Some(field),
                if field == "album" { "warn" } else { "error" },
                format!("campo {field} vazio"),
                false,
            ));
        }
    }

    if t.album_artist.is_none() && t.artist.is_some() {
        out.push(Issue::new(
            "missing_album_artist",
            Some("album_artist"),
            "info",
            "artista do álbum vazio; dá para copiar do artista".into(),
            true,
        ));
    }

    let stem = Path::new(&row.file_name)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| row.file_name.clone());
    if looks_swapped(&stem, t.title.as_deref(), t.artist.as_deref()) {
        out.push(Issue::new(
            "swapped_title_artist",
            Some("title"),
            "warn",
            "o nome do arquivo indica que título e artista estão trocados".into(),
            true,
        ));
    }

    for (field, raw) in [("track", &t.track_raw), ("disc", &t.disc_raw)] {
        if let Some(raw) = raw {
            if raw.contains('/') || raw.contains('\\') {
                out.push(Issue::new(
                    "pair_in_text",
                    Some(field),
                    "warn",
                    format!("{field} gravado como texto \"{raw}\""),
                    true,
                ));
            }
        }
    }

    if let Some(year) = &t.year {
        if normalize_year(year).is_none() {
            out.push(Issue::new(
                "bad_year",
                Some("year"),
                "warn",
                format!("ano \"{year}\" não tem um ano de quatro dígitos"),
                false,
            ));
        }
    }

    for field in TEXT_FIELDS {
        let value = match *field {
            "title" => &t.title,
            "artist" => &t.artist,
            "album_artist" => &t.album_artist,
            "album" => &t.album,
            "genre" => &t.genre,
            _ => &t.comment,
        };
        let Some(value) = value else { continue };
        if let Some(fixed) = fix_mojibake(value) {
            out.push(Issue::new(
                "mojibake",
                Some(field),
                "warn",
                format!("\"{value}\" parece ser \"{fixed}\""),
                true,
            ));
        } else if has_lost_bytes(value) {
            out.push(Issue::new(
                "lost_bytes",
                Some(field),
                "warn",
                format!("\"{value}\" perdeu caracteres na gravação"),
                false,
            ));
        }
    }

    if row.cover.count == 0 {
        out.push(Issue::new(
            "no_cover",
            Some("cover"),
            "info",
            "sem capa embutida".into(),
            true,
        ));
    } else {
        if row.cover.bytes > COVER_HUGE_BYTES {
            out.push(Issue::new(
                "cover_huge",
                Some("cover"),
                "warn",
                format!(
                    "capa de {} KB embutida em cada faixa",
                    row.cover.bytes / 1024
                ),
                false,
            ));
        }
        if row.cover.duplicated || row.cover.front_count > 1 {
            out.push(Issue::new(
                "cover_dupe",
                Some("cover"),
                "warn",
                format!("{} capas na mesma tag", row.cover.count),
                true,
            ));
        }
    }

    if no_primary && !other_tags.is_empty() {
        let names: Vec<String> = other_tags.iter().copied().map(tag_label).collect();
        out.push(Issue::new(
            "wrong_tag_type",
            None,
            "warn",
            format!(
                "{} guarda {} em vez de {}",
                row.format,
                names.join(", "),
                row.primary_tag_type
            ),
            false,
        ));
    }

    out
}

// ─────────────────────────────── scan ───────────────────────────────────

fn collect_paths(opts: &ScanOptions) -> Vec<PathBuf> {
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut out: Vec<PathBuf> = Vec::new();
    let push = |p: PathBuf, out: &mut Vec<PathBuf>, seen: &mut HashSet<PathBuf>| {
        let canon = std::fs::canonicalize(&p).unwrap_or_else(|_| p.clone());
        if seen.insert(canon) {
            out.push(p);
        }
    };

    for f in &opts.files {
        let p = PathBuf::from(f);
        if p.is_file() && is_audio(&p) {
            push(p, &mut out, &mut seen);
        }
    }
    for d in &opts.dirs {
        let depth = if opts.recursive { usize::MAX } else { 1 };
        let walker = WalkDir::new(d)
            .max_depth(depth)
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok);
        for entry in walker {
            if entry.file_type().is_file() && is_audio(entry.path()) {
                push(entry.path().to_path_buf(), &mut out, &mut seen);
            }
        }
    }

    let mut names: Vec<String> = out
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    sort_paths_by_name(&mut names);
    let mut sorted: Vec<PathBuf> = names.into_iter().map(PathBuf::from).collect();
    if let Some(max) = opts.max_files {
        sorted.truncate(max);
    }
    sorted
}

/// Lê tudo o que a UI pediu e devolve a tabela com o diagnóstico junto.
pub fn scan(opts: &ScanOptions, progress: &ProgressFn) -> anyhow::Result<ScanResult> {
    let paths = collect_paths(opts);
    let total = paths.len() as u64;
    report(progress, TOOL_ID, "started", 0, Some(total), None);

    let mut tracks = Vec::with_capacity(paths.len());
    let mut issue_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut failed = 0usize;
    let mut with_issues = 0usize;

    for (i, path) in paths.iter().enumerate() {
        let row = read_track(path);
        if row.error.is_some() {
            failed += 1;
        }
        if !row.issues.is_empty() {
            with_issues += 1;
        }
        for issue in &row.issues {
            *issue_counts.entry(issue.code.clone()).or_insert(0) += 1;
        }
        report(
            progress,
            TOOL_ID,
            "progress",
            i as u64 + 1,
            Some(total),
            Some(row.file_name.clone()),
        );
        tracks.push(row);
    }

    report(progress, TOOL_ID, "done", total, Some(total), None);
    Ok(ScanResult {
        scanned: tracks.len(),
        failed,
        with_issues,
        issue_counts,
        tracks,
    })
}

// ─────────────────────────── plano de edição ────────────────────────────

/// Estado de trabalho: campo → valor atual, como texto.
fn state_of(row: &TrackRow) -> BTreeMap<String, Option<String>> {
    let t = &row.tags;
    let num = |v: Option<u32>| v.map(|n| n.to_string());
    let mut m = BTreeMap::new();
    m.insert("title".into(), t.title.clone());
    m.insert("artist".into(), t.artist.clone());
    m.insert("album_artist".into(), t.album_artist.clone());
    m.insert("album".into(), t.album.clone());
    m.insert("year".into(), t.year.clone());
    // Faixa e disco entram com o texto cru: é ele que o usuário vê no diff
    // quando o arquivo guarda "3/12".
    m.insert("track".into(), t.track_raw.clone().or_else(|| num(t.track)));
    m.insert("track_total".into(), num(t.track_total));
    m.insert("disc".into(), t.disc_raw.clone().or_else(|| num(t.disc)));
    m.insert("disc_total".into(), num(t.disc_total));
    m.insert("genre".into(), t.genre.clone());
    m.insert("comment".into(), t.comment.clone());
    m
}

/// Diff de uma faixa, sem tocar em disco. É esta função que o `dry_run`
/// mostra e que o gravador aplica — as duas leem exatamente o mesmo plano.
///
/// `position` é o número que a renumeração vai gravar; `total` é o total de
/// faixas do lote.
pub fn plan_for(row: &TrackRow, position: u32, total: u32, opts: &EditOptions) -> Vec<FieldDiff> {
    let before = state_of(row);
    let mut after = before.clone();
    let mut reasons: BTreeMap<String, String> = BTreeMap::new();
    let put = |after: &mut BTreeMap<String, Option<String>>,
               reasons: &mut BTreeMap<String, String>,
               field: &str,
               value: Option<String>,
               reason: &str| {
        after.insert(field.to_string(), value);
        reasons.insert(field.to_string(), reason.to_string());
    };

    if opts.fix_mojibake {
        for field in TEXT_FIELDS {
            let current = after.get(*field).cloned().flatten();
            if let Some(fixed) = current.as_deref().and_then(fix_mojibake) {
                put(&mut after, &mut reasons, field, Some(fixed), "mojibake");
            }
        }
    }

    if opts.swap_title_artist {
        let title = after.get("title").cloned().flatten();
        let artist = after.get("artist").cloned().flatten();
        if title.is_some() || artist.is_some() {
            put(&mut after, &mut reasons, "title", artist, "swap");
            put(&mut after, &mut reasons, "artist", title, "swap");
        }
    }

    if opts.split_track_slash {
        for (field, total_field) in [("track", "track_total"), ("disc", "disc_total")] {
            let current = after.get(field).cloned().flatten();
            let Some(raw) = current else { continue };
            if !raw.contains('/') && !raw.contains('\\') {
                continue;
            }
            let (n, tot) = parse_track_pair(&raw);
            if let Some(n) = n {
                put(
                    &mut after,
                    &mut reasons,
                    field,
                    Some(n.to_string()),
                    "split_pair",
                );
            }
            if let Some(tot) = tot {
                put(
                    &mut after,
                    &mut reasons,
                    total_field,
                    Some(tot.to_string()),
                    "split_pair",
                );
            }
        }
    }

    for (field, value) in &opts.set {
        if !FIELDS.contains(&field.as_str()) {
            continue;
        }
        let value = value.trim();
        let value = if value.is_empty() {
            None
        } else {
            Some(value.to_string())
        };
        put(&mut after, &mut reasons, field, value, "set");
    }

    for field in &opts.clear {
        if FIELDS.contains(&field.as_str()) {
            put(&mut after, &mut reasons, field, None, "clear");
        }
    }

    if opts.album_artist_from_artist {
        let artist = after.get("artist").cloned().flatten();
        let album_artist = after.get("album_artist").cloned().flatten();
        if album_artist.is_none() && artist.is_some() {
            put(
                &mut after,
                &mut reasons,
                "album_artist",
                artist,
                "album_artist",
            );
        }
    }

    if opts.renumber {
        put(
            &mut after,
            &mut reasons,
            "track",
            Some(position.to_string()),
            "renumber",
        );
        if opts.set_track_total {
            put(
                &mut after,
                &mut reasons,
                "track_total",
                Some(total.to_string()),
                "renumber",
            );
        }
    }

    let mut diffs = Vec::new();
    for field in FIELDS {
        let from = before.get(*field).cloned().flatten();
        let to = after.get(*field).cloned().flatten();
        if from == to {
            continue;
        }
        diffs.push(FieldDiff {
            field: (*field).to_string(),
            from,
            to,
            reason: reasons
                .get(*field)
                .cloned()
                .unwrap_or_else(|| "set".to_string()),
        });
    }
    diffs
}

// ─────────────────────────────── capa ───────────────────────────────────

fn mime_of(bytes: &[u8]) -> Option<MimeType> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some(MimeType::Png)
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(MimeType::Jpeg)
    } else if bytes.starts_with(b"GIF8") {
        Some(MimeType::Gif)
    } else if bytes.starts_with(b"BM") {
        Some(MimeType::Bmp)
    } else {
        None
    }
}

/// Deixa só a primeira imagem de cada conteúdo (bytes iguais) e no máximo
/// uma capa frontal. Devolve quantas saíram.
fn dedupe_pictures(tag: &mut Tag) -> usize {
    let before = tag.pictures().len();
    let mut seen: HashSet<Vec<u8>> = HashSet::new();
    let mut front_seen = false;
    let keep: Vec<bool> = tag
        .pictures()
        .iter()
        .map(|p| {
            let is_front = p.pic_type() == PictureType::CoverFront;
            if is_front && front_seen {
                return false;
            }
            if !seen.insert(p.data().to_vec()) {
                return false;
            }
            if is_front {
                front_seen = true;
            }
            true
        })
        .collect();
    for i in (0..keep.len()).rev() {
        if !keep[i] {
            let _ = tag.remove_picture(i);
        }
    }
    before - tag.pictures().len()
}

// ────────────────────────────── gravação ────────────────────────────────

fn apply_diffs(tag: &mut Tag, diffs: &[FieldDiff]) -> Vec<String> {
    let mut unsupported = Vec::new();
    for diff in diffs {
        let Some(key) = item_key(&diff.field) else {
            continue;
        };
        match &diff.to {
            None => {
                tag.remove_key(key);
                // Faixa e disco no ID3v2 moram no mesmo frame do total.
                if diff.field == "track" {
                    tag.remove_key(ItemKey::TrackNumber);
                } else if diff.field == "disc" {
                    tag.remove_key(ItemKey::DiscNumber);
                }
                if diff.field == "year" {
                    tag.remove_key(ItemKey::RecordingDate);
                }
            }
            Some(value) => {
                let ok = tag.insert_text(key, value.clone());
                if !ok {
                    // O ID3v2 não tem "Year" — a data vai no TDRC.
                    let fallback = if diff.field == "year" {
                        tag.insert_text(ItemKey::RecordingDate, value.clone())
                    } else {
                        false
                    };
                    if !fallback {
                        unsupported.push(diff.field.clone());
                    }
                } else if diff.field == "year" {
                    // Mantém as duas leituras coerentes quando o formato
                    // aceita os dois campos.
                    let _ = tag.insert_text(ItemKey::RecordingDate, value.clone());
                }
            }
        }
    }
    unsupported
}

fn backup_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".bak");
    path.with_file_name(name)
}

fn edit_one(path: &Path, diffs: &[FieldDiff], opts: &EditOptions) -> anyhow::Result<FileChange> {
    let path_str = path.to_string_lossy().to_string();
    let mut change = FileChange {
        path: path_str.clone(),
        file_name: file_name_of(&path_str),
        diffs: diffs.to_vec(),
        cover: None,
        unsupported: Vec::new(),
        written: false,
        backup: None,
        error: None,
    };

    let cover_bytes = if let Some(cover) = opts.cover_path.as_deref().filter(|s| !s.is_empty()) {
        let data =
            std::fs::read(cover).with_context(|| format!("não consegui ler a capa em {cover}"))?;
        let mime =
            mime_of(&data).ok_or_else(|| anyhow!("a capa {cover} não é PNG, JPEG, GIF ou BMP"))?;
        Some((data, mime))
    } else {
        None
    };

    let mut tagged = lofty::read_from_path(path).map_err(|e| anyhow!(e.to_string()))?;
    let tag_type = tagged.primary_tag_type();
    if tagged.primary_tag_mut().is_none() {
        tagged.insert_tag(Tag::new(tag_type));
    }
    let tag = tagged
        .primary_tag_mut()
        .ok_or_else(|| anyhow!("o formato não aceita a tag {tag_type:?}"))?;

    let before_cover = tag.pictures().len();
    let before_bytes: u64 = tag.pictures().iter().map(|p| p.data().len() as u64).sum();

    change.unsupported = apply_diffs(tag, diffs);

    if opts.remove_cover {
        while !tag.pictures().is_empty() {
            let _ = tag.remove_picture(0);
        }
        change.cover = Some(CoverDiff {
            action: "remove".into(),
            from_bytes: before_bytes,
            to_bytes: 0,
        });
    } else if let Some((data, mime)) = cover_bytes {
        tag.remove_picture_type(PictureType::CoverFront);
        let to_bytes = data.len() as u64;
        tag.push_picture(
            Picture::unchecked(data)
                .pic_type(PictureType::CoverFront)
                .mime_type(mime)
                .build(),
        );
        change.cover = Some(CoverDiff {
            action: "embed".into(),
            from_bytes: before_bytes,
            to_bytes,
        });
    } else if opts.dedupe_cover && before_cover > 1 {
        let removed = dedupe_pictures(tag);
        if removed > 0 {
            change.cover = Some(CoverDiff {
                action: "dedupe".into(),
                from_bytes: before_bytes,
                to_bytes: tag.pictures().iter().map(|p| p.data().len() as u64).sum(),
            });
        }
    }

    let touched = !change.diffs.is_empty() || change.cover.is_some();
    if opts.dry_run || !touched {
        return Ok(change);
    }

    if opts.backup {
        let dest = backup_path(path);
        std::fs::copy(path, &dest)
            .with_context(|| format!("não consegui salvar a cópia em {}", dest.display()))?;
        change.backup = Some(dest.to_string_lossy().to_string());
    }
    tagged
        .save_to_path(path, LoftyWriteOptions::default())
        .map_err(|e| anyhow!(e.to_string()))?;
    change.written = true;
    Ok(change)
}

/// Aplica (ou apenas simula) a edição em lote. Com `dry_run` — o padrão —
/// nenhum byte do arquivo muda: o retorno é só o diff.
pub fn edit(opts: &EditOptions, progress: &ProgressFn) -> anyhow::Result<EditResult> {
    let mut files = opts.files.clone();
    if opts.sort_by_name {
        sort_paths_by_name(&mut files);
    }
    let total = files.len() as u64;
    report(progress, TOOL_ID, "started", 0, Some(total), None);

    let mut out = Vec::with_capacity(files.len());
    let mut changed = 0usize;
    let mut written = 0usize;
    let mut failed = 0usize;

    for (i, file) in files.iter().enumerate() {
        let path = PathBuf::from(file);
        let row = read_track(&path);
        let position = opts.renumber_start + i as u32;
        let diffs = plan_for(&row, position, files.len() as u32, opts);

        let change = if let Some(err) = row.error.clone() {
            FileChange {
                path: file.clone(),
                file_name: file_name_of(file),
                diffs: Vec::new(),
                cover: None,
                unsupported: Vec::new(),
                written: false,
                backup: None,
                error: Some(err),
            }
        } else {
            match edit_one(&path, &diffs, opts) {
                Ok(c) => c,
                Err(e) => FileChange {
                    path: file.clone(),
                    file_name: file_name_of(file),
                    diffs: diffs.clone(),
                    cover: None,
                    unsupported: Vec::new(),
                    written: false,
                    backup: None,
                    error: Some(e.to_string()),
                },
            }
        };

        if change.error.is_some() {
            failed += 1;
        } else {
            if !change.diffs.is_empty() || change.cover.is_some() {
                changed += 1;
            }
            if change.written {
                written += 1;
            }
        }
        report(
            progress,
            TOOL_ID,
            "progress",
            i as u64 + 1,
            Some(total),
            Some(change.file_name.clone()),
        );
        out.push(change);
    }

    report(progress, TOOL_ID, "done", total, Some(total), None);
    Ok(EditResult {
        dry_run: opts.dry_run,
        changed,
        written,
        failed,
        files: out,
    })
}

/// Capa de um arquivo, em `data:` URL, para a UI mostrar a miniatura.
pub fn cover_data_url(path: &str) -> anyhow::Result<Option<String>> {
    use base64::Engine as _;
    let tagged = lofty::read_from_path(path).map_err(|e| anyhow!(e.to_string()))?;
    let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) else {
        return Ok(None);
    };
    let pic = tag
        .get_picture_type(PictureType::CoverFront)
        .or_else(|| tag.pictures().first());
    let Some(pic) = pic else { return Ok(None) };
    let mime = pic
        .mime_type()
        .map(|m| m.as_str().to_string())
        .unwrap_or_else(|| "image/jpeg".to_string());
    let b64 = base64::engine::general_purpose::STANDARD.encode(pic.data());
    Ok(Some(format!("data:{mime};base64,{b64}")))
}

// ─────────────────────────────── testes ─────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use lofty::prelude::TagExt;

    /// FLAC mínimo válido: marcador + um bloco STREAMINFO de 1 segundo e
    /// nenhum quadro de áudio. É o bastante para o lofty abrir, ler as
    /// propriedades e gravar Vorbis Comments — sem binário no repositório.
    fn minimal_flac() -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"fLaC");
        v.push(0x80); // último bloco de metadado, tipo 0 (STREAMINFO)
        v.extend_from_slice(&[0x00, 0x00, 0x22]); // 34 bytes
        v.extend_from_slice(&4096u16.to_be_bytes()); // blocksize mínimo
        v.extend_from_slice(&4096u16.to_be_bytes()); // blocksize máximo
        v.extend_from_slice(&[0, 0, 0]); // framesize mínimo
        v.extend_from_slice(&[0, 0, 0]); // framesize máximo
                                         // 20 bits de taxa, 3 de canais (valor-1), 5 de bits (valor-1),
                                         // 36 de total de amostras.
        let packed: u64 = (44_100u64 << 44) | (1u64 << 41) | (15u64 << 36) | 44_100u64;
        v.extend_from_slice(&packed.to_be_bytes());
        v.extend_from_slice(&[0u8; 16]); // MD5 do áudio: zerado = desconhecido
        v
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("omniget-audio-tag-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("criar tempdir do teste");
        dir
    }

    fn write_flac(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, minimal_flac()).expect("gravar o flac do teste");
        p
    }

    fn row_with(tags: TrackTags, file_name: &str) -> TrackRow {
        TrackRow {
            path: format!("/musica/{file_name}"),
            file_name: file_name.to_string(),
            format: "FLAC".into(),
            tag_type: Some("VorbisComments".into()),
            primary_tag_type: "VorbisComments".into(),
            bytes: 1024,
            duration_secs: 1.0,
            bitrate_kbps: Some(900),
            sample_rate: Some(44_100),
            channels: Some(2),
            tags,
            cover: empty_cover(),
            issues: Vec::new(),
            error: None,
        }
    }

    // ── mojibake ──

    #[test]
    fn mojibake_conserta_utf8_lido_como_latin1() {
        assert_eq!(fix_mojibake("MÃºsica").as_deref(), Some("Música"));
        assert_eq!(fix_mojibake("CoraÃ§Ã£o").as_deref(), Some("Coração"));
        assert_eq!(fix_mojibake("BeyoncÃ©").as_deref(), Some("Beyoncé"));
        // Aspas tipográficas passam pela faixa C1 do CP1252.
        assert_eq!(fix_mojibake("Donâ€™t").as_deref(), Some("Don’t"));
    }

    #[test]
    fn mojibake_nao_mexe_em_texto_certo() {
        assert!(fix_mojibake("Música").is_none());
        assert!(fix_mojibake("Coração").is_none());
        assert!(fix_mojibake("Radiohead").is_none());
        assert!(fix_mojibake("").is_none());
        assert!(fix_mojibake("日本語").is_none());
    }

    #[test]
    fn bytes_perdidos_sao_detectados_mas_nao_consertados() {
        assert!(has_lost_bytes("Mus\u{FFFD}ica"));
        assert!(!has_lost_bytes("Musica"));
    }

    // ── faixa "3/12" ──

    #[test]
    fn parse_de_faixa_com_total() {
        assert_eq!(parse_track_pair("3/12"), (Some(3), Some(12)));
        assert_eq!(parse_track_pair("03/12"), (Some(3), Some(12)));
        assert_eq!(parse_track_pair(" 3 of 12 "), (Some(3), Some(12)));
        assert_eq!(parse_track_pair("3-12"), (Some(3), Some(12)));
        assert_eq!(parse_track_pair("7"), (Some(7), None));
        assert_eq!(parse_track_pair("A1"), (None, None));
        assert_eq!(parse_track_pair(""), (None, None));
    }

    #[test]
    fn ano_valido_ou_nao() {
        assert_eq!(normalize_year("1998"), Some(1998));
        assert_eq!(normalize_year("1998-05-03"), Some(1998));
        assert_eq!(normalize_year("2001-01-01T00:00:00"), Some(2001));
        assert_eq!(normalize_year("03/05/1998"), Some(1998));
        assert_eq!(normalize_year("98"), None);
        assert_eq!(normalize_year("0000"), None);
        assert_eq!(normalize_year("sem ano"), None);
    }

    // ── título/artista trocados ──

    #[test]
    fn detecta_titulo_e_artista_trocados() {
        assert!(looks_swapped(
            "Radiohead - Creep",
            Some("Radiohead"),
            Some("Creep")
        ));
        assert!(looks_swapped(
            "01 - Radiohead - Creep",
            Some("Radiohead"),
            Some("Creep")
        ));
        // Certo: o nome diz artista à esquerda e a tag concorda.
        assert!(!looks_swapped(
            "Radiohead - Creep",
            Some("Creep"),
            Some("Radiohead")
        ));
        assert!(!looks_swapped("faixa01", Some("Creep"), Some("Radiohead")));
        assert!(!looks_swapped("Radiohead - Creep", None, Some("Radiohead")));
    }

    // ── ordem natural ──

    #[test]
    fn ordena_por_nome_em_ordem_natural() {
        let mut v = vec![
            "/m/10 dez.flac".to_string(),
            "/m/2 dois.flac".to_string(),
            "/m/1 um.flac".to_string(),
        ];
        sort_paths_by_name(&mut v);
        assert_eq!(
            v,
            vec![
                "/m/1 um.flac".to_string(),
                "/m/2 dois.flac".to_string(),
                "/m/10 dez.flac".to_string()
            ]
        );
    }

    #[test]
    fn renumeracao_segue_a_ordem_de_nome() {
        let opts = EditOptions {
            renumber: true,
            set_track_total: true,
            ..Default::default()
        };
        let rows = [
            row_with(TrackTags::default(), "01 a.flac"),
            row_with(TrackTags::default(), "02 b.flac"),
            row_with(TrackTags::default(), "03 c.flac"),
        ];
        for (i, row) in rows.iter().enumerate() {
            let diffs = plan_for(row, i as u32 + 1, 3, &opts);
            let track = diffs.iter().find(|d| d.field == "track").expect("faixa");
            assert_eq!(track.to.as_deref(), Some((i + 1).to_string().as_str()));
            assert_eq!(track.reason, "renumber");
            let total = diffs
                .iter()
                .find(|d| d.field == "track_total")
                .expect("total");
            assert_eq!(total.to.as_deref(), Some("3"));
        }
    }

    // ── diff do dry_run ──

    #[test]
    fn diff_mostra_so_o_que_muda() {
        let mut tags = TrackTags {
            title: Some("Creep".into()),
            artist: Some("Radiohead".into()),
            ..Default::default()
        };
        tags.album = Some("Pablo Honey".into());
        let row = row_with(tags, "01 - Radiohead - Creep.flac");

        let mut set = BTreeMap::new();
        // Mesmo valor: não pode virar diff.
        set.insert("album".to_string(), "Pablo Honey".to_string());
        set.insert("genre".to_string(), "Rock".to_string());
        let opts = EditOptions {
            set,
            ..Default::default()
        };
        let diffs = plan_for(&row, 1, 1, &opts);
        assert_eq!(diffs.len(), 1, "só o gênero mudou: {diffs:?}");
        assert_eq!(diffs[0].field, "genre");
        assert_eq!(diffs[0].from, None);
        assert_eq!(diffs[0].to.as_deref(), Some("Rock"));
    }

    #[test]
    fn diff_quebra_faixa_em_texto_e_copia_artista_do_album() {
        let tags = TrackTags {
            title: Some("Creep".into()),
            artist: Some("Radiohead".into()),
            track_raw: Some("3/12".into()),
            ..Default::default()
        };
        let row = row_with(tags, "03 - Radiohead - Creep.flac");
        let opts = EditOptions {
            split_track_slash: true,
            album_artist_from_artist: true,
            ..Default::default()
        };
        let diffs = plan_for(&row, 3, 12, &opts);
        let by = |f: &str| diffs.iter().find(|d| d.field == f).cloned();

        let track = by("track").expect("faixa");
        assert_eq!(track.from.as_deref(), Some("3/12"));
        assert_eq!(track.to.as_deref(), Some("3"));
        assert_eq!(track.reason, "split_pair");
        assert_eq!(by("track_total").and_then(|d| d.to).as_deref(), Some("12"));
        assert_eq!(
            by("album_artist").and_then(|d| d.to).as_deref(),
            Some("Radiohead")
        );
    }

    #[test]
    fn diff_limpa_campo_e_troca_titulo_com_artista() {
        let tags = TrackTags {
            title: Some("Radiohead".into()),
            artist: Some("Creep".into()),
            comment: Some("rip por alguem".into()),
            ..Default::default()
        };
        let row = row_with(tags, "Radiohead - Creep.flac");
        let opts = EditOptions {
            swap_title_artist: true,
            clear: vec!["comment".into()],
            ..Default::default()
        };
        let diffs = plan_for(&row, 1, 1, &opts);
        let by = |f: &str| diffs.iter().find(|d| d.field == f).cloned();
        assert_eq!(by("title").and_then(|d| d.to).as_deref(), Some("Creep"));
        assert_eq!(
            by("artist").and_then(|d| d.to).as_deref(),
            Some("Radiohead")
        );
        let comment = by("comment").expect("comentário");
        assert_eq!(comment.to, None);
        assert_eq!(comment.reason, "clear");
    }

    #[test]
    fn diff_conserta_mojibake_dos_campos_de_texto() {
        let tags = TrackTags {
            title: Some("CoraÃ§Ã£o".into()),
            artist: Some("Radiohead".into()),
            ..Default::default()
        };
        let row = row_with(tags, "a.flac");
        let opts = EditOptions {
            fix_mojibake: true,
            ..Default::default()
        };
        let diffs = plan_for(&row, 1, 1, &opts);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].to.as_deref(), Some("Coração"));
        assert_eq!(diffs[0].reason, "mojibake");
    }

    // ── diagnóstico ──

    #[test]
    fn diagnostico_lista_os_problemas_da_faixa() {
        let tags = TrackTags {
            title: Some("Radiohead".into()),
            artist: Some("Creep".into()),
            year: Some("98".into()),
            track_raw: Some("3/12".into()),
            ..Default::default()
        };
        let row = row_with(tags, "Radiohead - Creep.flac");
        let issues = diagnose(&row, false, &[]);
        let codes: Vec<&str> = issues.iter().map(|i| i.code.as_str()).collect();
        assert!(codes.contains(&"missing_field"), "{codes:?}"); // álbum
        assert!(codes.contains(&"swapped_title_artist"), "{codes:?}");
        assert!(codes.contains(&"pair_in_text"), "{codes:?}");
        assert!(codes.contains(&"bad_year"), "{codes:?}");
        assert!(codes.contains(&"no_cover"), "{codes:?}");
        assert!(codes.contains(&"missing_album_artist"), "{codes:?}");
    }

    #[test]
    fn diagnostico_acusa_tag_estrangeira() {
        let row = row_with(TrackTags::default(), "a.flac");
        let issues = diagnose(&row, true, &[TagType::Id3v2]);
        assert!(issues.iter().any(|i| i.code == "wrong_tag_type"));
    }

    // ── ida e volta pelo lofty, num FLAC montado à mão ──

    #[test]
    fn le_um_flac_montado_a_mao() {
        let dir = temp_dir("read");
        let p = write_flac(&dir, "01 vazio.flac");
        let row = read_track(&p);
        assert!(row.error.is_none(), "erro: {:?}", row.error);
        assert_eq!(row.format, "FLAC");
        assert_eq!(row.sample_rate, Some(44_100));
        assert!((row.duration_secs - 1.0).abs() < 0.05, "{row:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_diagnostica_a_pasta_inteira() {
        let dir = temp_dir("scan");
        write_flac(&dir, "01 a.flac");
        write_flac(&dir, "02 b.flac");
        std::fs::write(dir.join("leia-me.txt"), b"nao e audio").expect("gravar txt");
        let opts = ScanOptions {
            dirs: vec![dir.to_string_lossy().to_string()],
            ..Default::default()
        };
        let out = scan(&opts, &super::super::noop_progress()).expect("scan");
        assert_eq!(out.scanned, 2, "o .txt não pode entrar");
        assert_eq!(out.failed, 0);
        // Arquivo sem tag nenhuma.
        assert!(out.tracks.iter().all(|t| t.tag_type.is_none()));
        assert!(out.issue_counts.contains_key("no_tag"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dry_run_nao_toca_no_arquivo_e_a_gravacao_toca() {
        let dir = temp_dir("write");
        let p = write_flac(&dir, "01 faixa.flac");
        let path = p.to_string_lossy().to_string();
        let antes = std::fs::read(&p).expect("ler antes");

        let mut set = BTreeMap::new();
        set.insert("title".to_string(), "Coração".to_string());
        set.insert("artist".to_string(), "Radiohead".to_string());
        let mut opts = EditOptions {
            files: vec![path.clone()],
            set,
            renumber: true,
            set_track_total: true,
            album_artist_from_artist: true,
            ..Default::default()
        };

        let seco = edit(&opts, &super::super::noop_progress()).expect("dry run");
        assert!(seco.dry_run);
        assert_eq!(seco.written, 0);
        assert_eq!(seco.changed, 1);
        assert_eq!(
            std::fs::read(&p).expect("ler depois do dry run"),
            antes,
            "dry_run não pode escrever byte nenhum"
        );
        let campos: Vec<&str> = seco.files[0]
            .diffs
            .iter()
            .map(|d| d.field.as_str())
            .collect();
        assert!(campos.contains(&"title"), "{campos:?}");
        assert!(campos.contains(&"album_artist"), "{campos:?}");
        assert!(campos.contains(&"track"), "{campos:?}");

        opts.dry_run = false;
        let gravado = edit(&opts, &super::super::noop_progress()).expect("gravar");
        assert_eq!(gravado.written, 1, "{:?}", gravado.files[0].error);

        let row = read_track(&p);
        assert_eq!(row.tags.title.as_deref(), Some("Coração"));
        assert_eq!(row.tags.artist.as_deref(), Some("Radiohead"));
        assert_eq!(row.tags.album_artist.as_deref(), Some("Radiohead"));
        assert_eq!(row.tags.track, Some(1));
        assert_eq!(row.tags.track_total, Some(1));

        // Rodar de novo com o mesmo plano não tem mais o que mudar.
        let repetido = edit(&opts, &super::super::noop_progress()).expect("segunda passada");
        let restantes: Vec<&str> = repetido.files[0]
            .diffs
            .iter()
            .map(|d| d.field.as_str())
            .collect();
        assert!(
            !restantes.contains(&"title"),
            "título já está certo: {restantes:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn capa_entra_sai_e_nao_duplica() {
        let dir = temp_dir("cover");
        let p = write_flac(&dir, "01 capa.flac");
        let path = p.to_string_lossy().to_string();
        // PNG 1x1 mínimo.
        let png: Vec<u8> = vec![
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
            0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78,
            0x9C, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00,
            0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
        ];
        let cover = dir.join("capa.png");
        std::fs::write(&cover, &png).expect("gravar png");

        let opts = EditOptions {
            files: vec![path.clone()],
            cover_path: Some(cover.to_string_lossy().to_string()),
            dry_run: false,
            ..Default::default()
        };
        let r = edit(&opts, &super::super::noop_progress()).expect("embutir capa");
        assert_eq!(r.written, 1, "{:?}", r.files[0].error);
        let row = read_track(&p);
        assert_eq!(row.cover.count, 1);
        assert_eq!(row.cover.mime.as_deref(), Some("image/png"));

        // Embutir de novo troca a capa frontal em vez de empilhar outra.
        let r = edit(&opts, &super::super::noop_progress()).expect("embutir de novo");
        assert_eq!(r.written, 1);
        assert_eq!(read_track(&p).cover.count, 1);

        let fora = EditOptions {
            files: vec![path],
            remove_cover: true,
            dry_run: false,
            ..Default::default()
        };
        edit(&fora, &super::super::noop_progress()).expect("tirar capa");
        assert_eq!(read_track(&p).cover.count, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dedupe_deixa_uma_capa_so() {
        let mut tag = Tag::new(TagType::VorbisComments);
        let data = vec![0x89, b'P', b'N', b'G', 1, 2, 3];
        for _ in 0..3 {
            tag.push_picture(
                Picture::unchecked(data.clone())
                    .pic_type(PictureType::CoverFront)
                    .mime_type(MimeType::Png)
                    .build(),
            );
        }
        let removidas = dedupe_pictures(&mut tag);
        assert_eq!(removidas, 2);
        assert_eq!(tag.pictures().len(), 1);
        assert!(!tag.is_empty() || tag.pictures().len() == 1);
    }

    #[test]
    fn backup_e_gravado_antes_de_escrever() {
        let dir = temp_dir("backup");
        let p = write_flac(&dir, "01 bkp.flac");
        let original = std::fs::read(&p).expect("ler original");
        let mut set = BTreeMap::new();
        set.insert("title".to_string(), "Teste".to_string());
        let opts = EditOptions {
            files: vec![p.to_string_lossy().to_string()],
            set,
            backup: true,
            dry_run: false,
            ..Default::default()
        };
        let r = edit(&opts, &super::super::noop_progress()).expect("gravar com backup");
        let bkp = r.files[0].backup.clone().expect("caminho do backup");
        assert_eq!(std::fs::read(&bkp).expect("ler backup"), original);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
