use std::path::{Path, PathBuf};

use chrono::{FixedOffset, TimeZone};

use crate::model::MessageKind;

pub fn sanitize(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        let bad =
            matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control();
        out.push(if bad { '_' } else { c });
    }
    let trimmed = out.trim().trim_matches('.').trim();
    let mut s: String = trimmed.chars().take(120).collect();
    if s.is_empty() {
        s = "_".into();
    }
    let upper = s.to_ascii_uppercase();
    let stem = upper.split('.').next().unwrap_or("");
    const RESERVED: &[&str] = &[
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if RESERVED.contains(&stem) {
        s.insert(0, '_');
    }
    s
}

pub fn outgoing(file_name: &str) -> (MessageKind, &'static str) {
    let ext = file_name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "jpg" | "jpeg" => (MessageKind::Image, "image/jpeg"),
        "png" => (MessageKind::Image, "image/png"),
        "webp" => (MessageKind::Image, "image/webp"),
        "mp4" | "m4v" => (MessageKind::Video, "video/mp4"),
        "mov" => (MessageKind::Video, "video/quicktime"),
        "mp3" => (MessageKind::Audio, "audio/mpeg"),
        "m4a" => (MessageKind::Audio, "audio/mp4"),
        "ogg" | "opus" => (MessageKind::Audio, "audio/ogg"),
        "wav" => (MessageKind::Audio, "audio/wav"),
        "pdf" => (MessageKind::Document, "application/pdf"),
        "txt" => (MessageKind::Document, "text/plain"),
        "csv" => (MessageKind::Document, "text/csv"),
        "zip" => (MessageKind::Document, "application/zip"),
        "doc" => (MessageKind::Document, "application/msword"),
        "docx" => (
            MessageKind::Document,
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        ),
        "xlsx" => (
            MessageKind::Document,
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        ),
        _ => (MessageKind::Document, "application/octet-stream"),
    }
}

pub fn ext_from_mime(mime: Option<&str>, kind: MessageKind) -> &'static str {
    let m = mime
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    match m.as_str() {
        "image/jpeg" | "image/jpg" => "jpg",
        "image/png" => "png",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "image/heic" => "heic",
        "video/mp4" => "mp4",
        "video/3gpp" => "3gp",
        "video/quicktime" => "mov",
        "video/webm" => "webm",
        "audio/ogg" | "audio/opus" => "ogg",
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/mp4" | "audio/m4a" | "audio/x-m4a" | "audio/aac" => "m4a",
        "audio/amr" => "amr",
        "audio/wav" | "audio/x-wav" => "wav",
        "application/pdf" => "pdf",
        "application/zip" => "zip",
        "text/plain" => "txt",
        "application/msword" => "doc",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => "docx",
        "application/vnd.ms-excel" => "xls",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => "xlsx",
        "application/vnd.ms-powerpoint" => "ppt",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => "pptx",
        _ => match kind {
            MessageKind::Image => "jpg",
            MessageKind::Video => "mp4",
            MessageKind::Audio | MessageKind::Voice => "ogg",
            MessageKind::Sticker => "webp",
            _ => "bin",
        },
    }
}

pub struct NameInput<'a> {
    pub ts: i64,
    pub id: &'a str,
    pub kind: MessageKind,
    pub mime: Option<&'a str>,
    pub file_name: Option<&'a str>,
}

pub fn media_file_name(input: &NameInput<'_>, offset: FixedOffset) -> String {
    let stamp = offset
        .timestamp_opt(input.ts, 0)
        .single()
        .map(|d| d.format("%Y-%m-%d %H.%M.%S").to_string())
        .unwrap_or_else(|| "0000-00-00 00.00.00".into());
    let short_id: String = sanitize(input.id).chars().take(10).collect();
    if let Some(original) = input.file_name.filter(|n| !n.trim().is_empty()) {
        let clean = sanitize(original);
        if Path::new(&clean).extension().is_some() {
            return format!("{stamp} {clean}");
        }
        return format!("{stamp} {clean}.{}", ext_from_mime(input.mime, input.kind));
    }
    let label = match input.kind {
        MessageKind::Voice => "voice",
        k => k.as_str(),
    };
    format!(
        "{stamp} {label} {short_id}.{}",
        ext_from_mime(input.mime, input.kind)
    )
}

pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let p = Path::new(name);
    let stem = p
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| name.to_string());
    let ext = p.extension().map(|e| e.to_string_lossy().into_owned());
    for n in 2.. {
        let candidate = match &ext {
            Some(e) => dir.join(format!("{stem} ({n}).{e}")),
            None => dir.join(format!("{stem} ({n})")),
        };
        if !candidate.exists() {
            return candidate;
        }
    }
    first
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc() -> FixedOffset {
        FixedOffset::east_opt(0).unwrap()
    }

    #[test]
    fn sanitize_strips_path_chars_and_reserved_names() {
        assert_eq!(sanitize("a/b\\c:d*e?"), "a_b_c_d_e_");
        assert_eq!(sanitize("  ..  "), "_");
        assert_eq!(sanitize("CON.txt"), "_CON.txt");
        assert_eq!(sanitize("Família 🎉"), "Família 🎉");
        assert_eq!(sanitize(&"x".repeat(300)).chars().count(), 120);
    }

    #[test]
    fn mime_maps_to_extension_with_kind_fallback() {
        assert_eq!(
            ext_from_mime(Some("audio/ogg; codecs=opus"), MessageKind::Voice),
            "ogg"
        );
        assert_eq!(ext_from_mime(Some("image/jpeg"), MessageKind::Image), "jpg");
        assert_eq!(ext_from_mime(None, MessageKind::Video), "mp4");
        assert_eq!(ext_from_mime(Some("weird/x"), MessageKind::Document), "bin");
        assert_eq!(ext_from_mime(None, MessageKind::Sticker), "webp");
    }

    #[test]
    fn file_name_uses_timestamp_kind_and_id() {
        let n = media_file_name(
            &NameInput {
                ts: 1_700_000_000,
                id: "3EB0ABCDEF1234567890",
                kind: MessageKind::Image,
                mime: Some("image/jpeg"),
                file_name: None,
            },
            utc(),
        );
        assert_eq!(n, "2023-11-14 22.13.20 image 3EB0ABCDEF.jpg");
    }

    #[test]
    fn file_name_keeps_document_name() {
        let n = media_file_name(
            &NameInput {
                ts: 1_700_000_000,
                id: "X",
                kind: MessageKind::Document,
                mime: Some("application/pdf"),
                file_name: Some("Boleto: março.pdf"),
            },
            utc(),
        );
        assert_eq!(n, "2023-11-14 22.13.20 Boleto_ março.pdf");
        let n = media_file_name(
            &NameInput {
                ts: 1_700_000_000,
                id: "X",
                kind: MessageKind::Document,
                mime: Some("application/pdf"),
                file_name: Some("contrato"),
            },
            utc(),
        );
        assert_eq!(n, "2023-11-14 22.13.20 contrato.pdf");
    }

    #[test]
    fn unique_path_appends_counter() {
        let dir = tempfile::tempdir().unwrap();
        let a = unique_path(dir.path(), "f.jpg");
        assert_eq!(a, dir.path().join("f.jpg"));
        std::fs::write(&a, b"1").unwrap();
        let b = unique_path(dir.path(), "f.jpg");
        assert_eq!(b, dir.path().join("f (2).jpg"));
    }
    #[test]
    fn outgoing_files_pick_kind_and_mime_by_extension() {
        assert_eq!(outgoing("Foto.JPG"), (MessageKind::Image, "image/jpeg"));
        assert_eq!(
            outgoing("clip.mov"),
            (MessageKind::Video, "video/quicktime")
        );
        assert_eq!(outgoing("nota.m4a"), (MessageKind::Audio, "audio/mp4"));
        assert_eq!(
            outgoing("a.b.pdf"),
            (MessageKind::Document, "application/pdf")
        );
        assert_eq!(
            outgoing("sem-extensao"),
            (MessageKind::Document, "application/octet-stream")
        );
        assert_eq!(
            outgoing("script.sh"),
            (MessageKind::Document, "application/octet-stream")
        );
    }
}
