use std::io::BufReader;
use std::path::Path;
use std::time::SystemTime;

use chrono::{Datelike, Local, NaiveDateTime, TimeZone};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Exif {
    pub taken_at: Option<i64>,
    pub orientation: Option<u8>,
}

pub fn read_exif(path: &Path) -> Exif {
    let Ok(file) = std::fs::File::open(path) else {
        return Exif::default();
    };
    let Ok(exif) = exif::Reader::new().read_from_container(&mut BufReader::new(file)) else {
        return Exif::default();
    };
    let taken_at = [exif::Tag::DateTimeOriginal, exif::Tag::DateTimeDigitized]
        .iter()
        .find_map(|tag| {
            let field = exif.get_field(*tag, exif::In::PRIMARY)?;
            match &field.value {
                exif::Value::Ascii(parts) => parts.first().and_then(|b| parse_datetime(b)),
                _ => None,
            }
        });
    let orientation = exif
        .get_field(exif::Tag::Orientation, exif::In::PRIMARY)
        .and_then(|f| f.value.get_uint(0))
        .and_then(|v| u8::try_from(v).ok())
        .filter(|v| (1..=8).contains(v));
    Exif {
        taken_at,
        orientation,
    }
}

pub fn parse_datetime(bytes: &[u8]) -> Option<i64> {
    let text = std::str::from_utf8(bytes)
        .ok()?
        .trim_end_matches('\0')
        .trim();
    let head = text.get(..19)?;
    let parsed = NaiveDateTime::parse_from_str(head, "%Y:%m:%d %H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(head, "%Y-%m-%d %H:%M:%S"))
        .ok()?;
    if !(1900..=2200).contains(&parsed.year()) {
        return None;
    }
    Some(parsed.and_utc().timestamp())
}

pub fn wall_clock(time: SystemTime) -> i64 {
    let secs = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    Local
        .timestamp_opt(secs, 0)
        .single()
        .map(|t| t.naive_local().and_utc().timestamp())
        .unwrap_or(secs)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RawInfo {
    pub taken_at: Option<i64>,
    pub size: Option<(u32, u32)>,
}

pub fn raw_info(path: &Path, orientation: Option<u8>) -> RawInfo {
    let Ok(source) = rawler::rawsource::RawSource::new(path) else {
        return RawInfo::default();
    };
    let Ok(decoder) = rawler::get_decoder(&source) else {
        return RawInfo::default();
    };
    let params = rawler::decoders::RawDecodeParams::default();
    let taken_at = decoder
        .raw_metadata(&source, &params)
        .ok()
        .and_then(|m| m.exif.date_time_original)
        .and_then(|d| parse_datetime(d.as_bytes()));
    let size = decoder.raw_image(&source, &params, true).ok().map(|raw| {
        let (w, h) = raw
            .crop_area
            .or(raw.active_area)
            .map(|r| (r.width(), r.height()))
            .unwrap_or((raw.width, raw.height));
        let (w, h) = (w as u32, h as u32);
        if orientation.is_some_and(|o| o >= 5) {
            (h, w)
        } else {
            (w, h)
        }
    });
    RawInfo { taken_at, size }
}
