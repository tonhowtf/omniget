//! Reading and writing audio tags (ID3v2, Vorbis comments, MP4 atoms, APE)
//! through `lofty`, flattened into one struct the library and the tag
//! editor share.

use std::path::Path;

use lofty::config::WriteOptions;
use lofty::file::{AudioFile, FileType, TaggedFileExt};
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::tag::{Accessor, ItemKey, Tag};
use serde::{Deserialize, Serialize};

use super::loudness::{parse_gain_tag, r128_q78_to_replaygain_db};

pub const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "m4a", "m4b", "mp4", "aac", "alac", "ogg", "oga", "opus", "wav", "wave", "aif",
    "aiff", "aifc", "wv", "ape", "mpc", "wma", "webm", "mka", "spx", "dsf",
];

pub fn is_audio_path(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| AUDIO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TrackMeta {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub genre: String,
    pub composer: String,
    pub comment: String,
    pub year: Option<u32>,
    pub track: Option<u32>,
    pub track_total: Option<u32>,
    pub disc: Option<u32>,
    pub disc_total: Option<u32>,
    pub compilation: bool,
    pub bpm: Option<f32>,
    pub musical_key: String,
    pub rg_track_gain: Option<f64>,
    pub rg_track_peak: Option<f64>,
    pub rg_album_gain: Option<f64>,
    pub rg_album_peak: Option<f64>,
    pub lyrics: String,
    pub mb_recording_id: String,
    pub mb_release_id: String,
    // technical properties
    pub duration_ms: u64,
    pub bitrate: Option<u32>,
    pub sample_rate: Option<u32>,
    pub bit_depth: Option<u8>,
    pub channels: Option<u8>,
    pub codec: String,
    pub has_cover: bool,
}

#[derive(Debug, Clone)]
pub struct Cover {
    pub data: Vec<u8>,
    pub mime: String,
}

fn codec_name(ft: FileType, ext: &str) -> String {
    match ft {
        FileType::Mpeg => "MP3".into(),
        FileType::Flac => "FLAC".into(),
        FileType::Mp4 => match ext {
            "alac" => "ALAC".into(),
            _ => "AAC".into(),
        },
        FileType::Aac => "AAC".into(),
        FileType::Opus => "Opus".into(),
        FileType::Vorbis => "Vorbis".into(),
        FileType::Speex => "Speex".into(),
        FileType::Wav => "WAV".into(),
        FileType::Aiff => "AIFF".into(),
        FileType::WavPack => "WavPack".into(),
        FileType::Ape => "APE".into(),
        FileType::Mpc => "Musepack".into(),
        _ => ext.to_ascii_uppercase(),
    }
}

/// "01 - Artist - Title", "Artist - Title", "Title" from a file name.
pub fn guess_from_filename(path: &Path) -> (String, String) {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .replace('_', " ");
    // strip a leading track number ("01 ", "01. ", "01 - ")
    let trimmed = stem.trim_start_matches(|c: char| c.is_ascii_digit());
    let trimmed = if trimmed.len() < stem.len() {
        trimmed.trim_start_matches(['.', '-', ' ']).trim()
    } else {
        stem.trim()
    };
    // yt-dlp style "Title [videoid]"
    let trimmed = match trimmed.rfind(" [") {
        Some(i) if trimmed.ends_with(']') && trimmed.len() - i <= 16 => trimmed[..i].trim(),
        _ => trimmed,
    };
    match trimmed.split_once(" - ") {
        Some((a, t)) if !a.trim().is_empty() && !t.trim().is_empty() => {
            (a.trim().to_string(), t.trim().to_string())
        }
        _ => (String::new(), trimmed.to_string()),
    }
}

fn get_num(tag: &Tag, key: ItemKey) -> Option<f32> {
    tag.get_string(key)
        .and_then(|s| s.trim().split(['/', ' ']).next().map(str::to_string))
        .and_then(|s| s.parse::<f32>().ok())
}

pub fn read(path: &Path) -> anyhow::Result<(TrackMeta, Option<Cover>)> {
    let tagged = lofty::read_from_path(path)?;
    let props = tagged.properties();
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let mut m = TrackMeta {
        duration_ms: props.duration().as_millis() as u64,
        bitrate: props.audio_bitrate().or(props.overall_bitrate()),
        sample_rate: props.sample_rate(),
        bit_depth: props.bit_depth(),
        channels: props.channels(),
        codec: codec_name(tagged.file_type(), &ext),
        ..Default::default()
    };
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
    let mut cover = None;
    if let Some(tag) = tag {
        m.title = tag.title().map(|s| s.to_string()).unwrap_or_default();
        m.artist = tag.artist().map(|s| s.to_string()).unwrap_or_default();
        m.album = tag.album().map(|s| s.to_string()).unwrap_or_default();
        m.genre = tag.genre().map(|s| s.to_string()).unwrap_or_default();
        m.comment = tag.comment().map(|s| s.to_string()).unwrap_or_default();
        m.album_artist = tag
            .get_string(ItemKey::AlbumArtist)
            .unwrap_or_default()
            .to_string();
        m.composer = tag
            .get_string(ItemKey::Composer)
            .unwrap_or_default()
            .to_string();
        m.year = tag.date().map(|d| d.year as u32).or_else(|| {
            tag.get_string(ItemKey::Year)
                .and_then(|y| y.get(..4))
                .and_then(|y| y.parse().ok())
        });
        m.track = tag.track();
        m.track_total = tag.track_total();
        m.disc = tag.disk();
        m.disc_total = tag.disk_total();
        m.compilation = tag
            .get_string(ItemKey::FlagCompilation)
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        m.bpm = get_num(tag, ItemKey::Bpm).or_else(|| get_num(tag, ItemKey::IntegerBpm));
        m.musical_key = tag
            .get_string(ItemKey::InitialKey)
            .unwrap_or_default()
            .to_string();
        m.rg_track_gain = tag
            .get_string(ItemKey::ReplayGainTrackGain)
            .and_then(parse_gain_tag);
        m.rg_track_peak = tag
            .get_string(ItemKey::ReplayGainTrackPeak)
            .and_then(|s| s.trim().parse().ok());
        m.rg_album_gain = tag
            .get_string(ItemKey::ReplayGainAlbumGain)
            .and_then(parse_gain_tag);
        m.rg_album_peak = tag
            .get_string(ItemKey::ReplayGainAlbumPeak)
            .and_then(|s| s.trim().parse().ok());
        m.lyrics = tag
            .get_string(ItemKey::Lyrics)
            .or_else(|| tag.get_string(ItemKey::UnsyncLyrics))
            .unwrap_or_default()
            .to_string();
        m.mb_recording_id = tag
            .get_string(ItemKey::MusicBrainzRecordingId)
            .unwrap_or_default()
            .to_string();
        m.mb_release_id = tag
            .get_string(ItemKey::MusicBrainzReleaseId)
            .unwrap_or_default()
            .to_string();
        let pic = tag
            .get_picture_type(PictureType::CoverFront)
            .or_else(|| tag.pictures().first());
        if let Some(p) = pic {
            m.has_cover = true;
            cover = Some(Cover {
                data: p.data().to_vec(),
                mime: p
                    .mime_type()
                    .map(|m| m.as_str().to_string())
                    .unwrap_or_else(|| sniff_mime(p.data()).to_string()),
            });
        }
    }
    if tagged.file_type() == FileType::Opus && m.rg_track_gain.is_none() {
        let (t, a) = read_opus_r128(path);
        m.rg_track_gain = t;
        m.rg_album_gain = a;
    }
    if m.title.trim().is_empty() {
        let (a, t) = guess_from_filename(path);
        m.title = t;
        if m.artist.trim().is_empty() {
            m.artist = a;
        }
    }
    Ok((m, cover))
}

pub fn sniff_mime(data: &[u8]) -> &'static str {
    if data.starts_with(&[0x89, b'P', b'N', b'G']) {
        "image/png"
    } else if data.starts_with(b"RIFF") && data.get(8..12) == Some(b"WEBP") {
        "image/webp"
    } else if data.starts_with(b"GIF8") {
        "image/gif"
    } else {
        "image/jpeg"
    }
}

/// A partial edit: `None` leaves the field alone, `Some("")` removes it.
/// Numbers travel as strings so "clear" and "set" share one shape.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TagPatch {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub genre: Option<String>,
    pub composer: Option<String>,
    pub comment: Option<String>,
    pub year: Option<String>,
    pub track: Option<String>,
    pub track_total: Option<String>,
    pub disc: Option<String>,
    pub disc_total: Option<String>,
    pub bpm: Option<String>,
    pub musical_key: Option<String>,
    pub lyrics: Option<String>,
    pub rg_track_gain: Option<String>,
    pub rg_track_peak: Option<String>,
    pub rg_album_gain: Option<String>,
    pub rg_album_peak: Option<String>,
}

impl TagPatch {
    pub fn is_empty(&self) -> bool {
        serde_json::to_value(self)
            .ok()
            .and_then(|v| v.as_object().map(|o| o.values().all(|v| v.is_null())))
            .unwrap_or(true)
    }
}

fn set_text(tag: &mut Tag, key: ItemKey, v: &Option<String>) {
    if let Some(v) = v {
        let v = v.trim();
        if v.is_empty() {
            tag.remove_key(key);
        } else {
            tag.insert_text(key, v.to_string());
        }
    }
}

/// Track/disc numbers: `None` = untouched, `Some(None)` = remove.
fn position(v: &Option<String>) -> Option<Option<u32>> {
    let v = v.as_ref()?;
    Some(v.trim().parse::<u32>().ok().filter(|n| *n > 0))
}

pub enum CoverChange {
    Keep,
    Remove,
    Set(Vec<u8>),
}

pub fn write(path: &Path, patch: &TagPatch, cover: CoverChange) -> anyhow::Result<()> {
    let mut tagged = lofty::read_from_path(path)?;
    if tagged.primary_tag().is_none() {
        let tt = tagged.primary_tag_type();
        tagged.insert_tag(Tag::new(tt));
    }
    let tag = tagged
        .primary_tag_mut()
        .ok_or_else(|| anyhow::anyhow!("this format can't hold tags"))?;

    let text = |v: &Option<String>| v.as_ref().map(|s| s.trim().to_string());
    if let Some(v) = text(&patch.title) {
        if v.is_empty() {
            tag.remove_title()
        } else {
            tag.set_title(v)
        }
    }
    if let Some(v) = text(&patch.artist) {
        if v.is_empty() {
            tag.remove_artist()
        } else {
            tag.set_artist(v)
        }
    }
    if let Some(v) = text(&patch.album) {
        if v.is_empty() {
            tag.remove_album()
        } else {
            tag.set_album(v)
        }
    }
    if let Some(v) = text(&patch.genre) {
        if v.is_empty() {
            tag.remove_genre()
        } else {
            tag.set_genre(v)
        }
    }
    if let Some(v) = text(&patch.comment) {
        if v.is_empty() {
            tag.remove_comment()
        } else {
            tag.set_comment(v)
        }
    }
    set_text(tag, ItemKey::AlbumArtist, &patch.album_artist);
    set_text(tag, ItemKey::Composer, &patch.composer);
    set_text(tag, ItemKey::InitialKey, &patch.musical_key);
    set_text(tag, ItemKey::Lyrics, &patch.lyrics);
    set_text(tag, ItemKey::ReplayGainTrackGain, &patch.rg_track_gain);
    set_text(tag, ItemKey::ReplayGainTrackPeak, &patch.rg_track_peak);
    set_text(tag, ItemKey::ReplayGainAlbumGain, &patch.rg_album_gain);
    set_text(tag, ItemKey::ReplayGainAlbumPeak, &patch.rg_album_peak);
    if let Some(v) = &patch.year {
        let v = v.trim();
        tag.remove_key(ItemKey::Year);
        tag.remove_key(ItemKey::RecordingDate);
        if !v.is_empty() {
            tag.insert_text(ItemKey::RecordingDate, v.to_string());
        }
    }
    if let Some(v) = &patch.bpm {
        tag.remove_key(ItemKey::Bpm);
        tag.remove_key(ItemKey::IntegerBpm);
        if let Ok(b) = v.trim().parse::<f32>() {
            if b > 0.0 {
                tag.insert_text(ItemKey::IntegerBpm, format!("{}", b.round() as u32));
            }
        }
    }
    match position(&patch.track) {
        Some(Some(n)) => tag.set_track(n),
        Some(None) => tag.remove_track(),
        None => {}
    }
    match position(&patch.track_total) {
        Some(Some(n)) => tag.set_track_total(n),
        Some(None) => tag.remove_track_total(),
        None => {}
    }
    match position(&patch.disc) {
        Some(Some(n)) => tag.set_disk(n),
        Some(None) => tag.remove_disk(),
        None => {}
    }
    match position(&patch.disc_total) {
        Some(Some(n)) => tag.set_disk_total(n),
        Some(None) => tag.remove_disk_total(),
        None => {}
    }

    match cover {
        CoverChange::Keep => {}
        CoverChange::Remove => {
            while tag.picture_count() > 0 {
                tag.remove_picture(0);
            }
        }
        CoverChange::Set(data) => {
            tag.remove_picture_type(PictureType::CoverFront);
            let mime = MimeType::from_str(sniff_mime(&data));
            let pic = Picture::unchecked(data)
                .pic_type(PictureType::CoverFront)
                .mime_type(mime)
                .build();
            tag.push_picture(pic);
        }
    }

    tagged.save_to_path(path, WriteOptions::default())?;
    Ok(())
}

fn open_opus(path: &Path) -> Option<lofty::ogg::OpusFile> {
    let mut f = std::fs::File::open(path).ok()?;
    lofty::ogg::OpusFile::read_from(&mut f, lofty::config::ParseOptions::new()).ok()
}

/// Opus keeps gain as `R128_TRACK_GAIN` (Q7.8 relative to -23 LUFS) instead
/// of ReplayGain tags; returns them as ReplayGain dB (track, album).
pub fn read_opus_r128(path: &Path) -> (Option<f64>, Option<f64>) {
    let Some(f) = open_opus(path) else {
        return (None, None);
    };
    let vc = f.vorbis_comments();
    let get = |k: &str| {
        vc.get(k)
            .and_then(|v| v.trim().parse::<i32>().ok())
            .map(r128_q78_to_replaygain_db)
    };
    (get("R128_TRACK_GAIN"), get("R128_ALBUM_GAIN"))
}

/// loudgain's Opus convention: only `R128_*_GAIN` (no peak), and the
/// ReplayGain-style tags removed so players don't apply both.
pub fn write_opus_r128(
    path: &Path,
    track_db: Option<f64>,
    album_db: Option<f64>,
) -> anyhow::Result<()> {
    let mut f = open_opus(path).ok_or_else(|| anyhow::anyhow!("not an Opus file"))?;
    let vc = f.vorbis_comments_mut();
    for k in [
        "REPLAYGAIN_TRACK_GAIN",
        "REPLAYGAIN_TRACK_PEAK",
        "REPLAYGAIN_ALBUM_GAIN",
        "REPLAYGAIN_ALBUM_PEAK",
        "R128_TRACK_GAIN",
        "R128_ALBUM_GAIN",
    ] {
        let _ = vc.remove(k).count();
    }
    let q78 = |db: f64| {
        (((db - 5.0) * 256.0).round() as i64)
            .clamp(-32768, 32767)
            .to_string()
    };
    if let Some(t) = track_db {
        vc.insert("R128_TRACK_GAIN".into(), q78(t));
    }
    if let Some(a) = album_db {
        vc.insert("R128_ALBUM_GAIN".into(), q78(a));
    }
    f.save_to_path(path, WriteOptions::default())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guesses_artist_and_title() {
        assert_eq!(
            guess_from_filename(Path::new("/x/03 - Band - Song Name.mp3")),
            ("Band".into(), "Song Name".into())
        );
        assert_eq!(
            guess_from_filename(Path::new("Band - Song [dQw4w9WgXcQ].opus")),
            ("Band".into(), "Song".into())
        );
        assert_eq!(
            guess_from_filename(Path::new("Just_a_title.flac")),
            (String::new(), "Just a title".into())
        );
    }

    #[test]
    fn audio_extensions() {
        assert!(is_audio_path(Path::new("a.FLAC")));
        assert!(!is_audio_path(Path::new("a.jpg")));
    }

    #[test]
    fn empty_patch() {
        assert!(TagPatch::default().is_empty());
        let p = TagPatch {
            title: Some("x".into()),
            ..Default::default()
        };
        assert!(!p.is_empty());
    }
}
