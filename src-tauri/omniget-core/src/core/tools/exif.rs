//! Ver e limpar metadados de imagem (EXIF/XMP/IPTC), com foco no que vaza:
//! GPS, número de série da câmera, nome do dono, data e software.
//!
//! A leitura usa o `kamadak-exif` (Rust puro). A limpeza NÃO reencoda nada:
//! reescreve o contêiner sem os segmentos de metadado, então o pixel sai
//! byte a byte igual ao original — o oposto do que um "salvar como" faz.

use std::path::{Path, PathBuf};

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
pub struct ExifField {
    pub tag: String,
    pub label: String,
    pub group: String,
    pub value: String,
    /// Campo que identifica pessoa, lugar ou equipamento.
    pub sensitive: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExifReport {
    pub path: String,
    pub bytes: u64,
    pub has_exif: bool,
    pub fields: Vec<ExifField>,
    /// (latitude, longitude) em graus decimais, quando existe GPS.
    pub gps: Option<(f64, f64)>,
    pub camera: Option<String>,
    pub taken: Option<String>,
    pub software: Option<String>,
    /// Quantos campos sensíveis foram achados (GPS conta como um).
    pub sensitive_count: usize,
    pub error: Option<String>,
}

const SENSITIVE_PREFIXES: &[&str] = &["GPS"];
const SENSITIVE_TAGS: &[&str] = &[
    "BodySerialNumber",
    "CameraOwnerName",
    "LensSerialNumber",
    "SerialNumber",
    "Artist",
    "Copyright",
    "ImageUniqueID",
    "OwnerName",
];

fn is_sensitive(tag: &str) -> bool {
    SENSITIVE_PREFIXES.iter().any(|p| tag.starts_with(p)) || SENSITIVE_TAGS.contains(&tag)
}

/// Graus/minutos/segundos + hemisfério → graus decimais.
pub fn dms_to_deg(dms: &[f64], reference: &str) -> Option<f64> {
    if dms.is_empty() {
        return None;
    }
    let d = dms.first().copied().unwrap_or(0.0);
    let m = dms.get(1).copied().unwrap_or(0.0);
    let s = dms.get(2).copied().unwrap_or(0.0);
    let deg = d + m / 60.0 + s / 3600.0;
    let r = reference.trim().to_uppercase();
    Some(if r.starts_with('S') || r.starts_with('W') {
        -deg
    } else {
        deg
    })
}

fn rationals(field: &exif::Field) -> Vec<f64> {
    match &field.value {
        exif::Value::Rational(v) => v.iter().map(|r| r.to_f64()).collect(),
        exif::Value::SRational(v) => v.iter().map(|r| r.to_f64()).collect(),
        _ => Vec::new(),
    }
}

fn text_of(exif: &exif::Exif, tag: exif::Tag) -> Option<String> {
    exif.get_field(tag, exif::In::PRIMARY).map(|f| {
        f.display_value()
            .to_string()
            .trim_matches('"')
            .trim()
            .to_string()
    })
}

pub fn read(path: &str) -> ExifReport {
    let bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let mut report = ExifReport {
        path: path.to_string(),
        bytes,
        has_exif: false,
        fields: Vec::new(),
        gps: None,
        camera: None,
        taken: None,
        software: None,
        sensitive_count: 0,
        error: None,
    };
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) => {
            report.error = Some(e.to_string());
            return report;
        }
    };
    let mut reader = std::io::BufReader::new(&file);
    let exif = match exif::Reader::new().read_from_container(&mut reader) {
        Ok(e) => e,
        Err(e) => {
            // "sem EXIF" é resposta legítima, não falha.
            if !matches!(e, exif::Error::NotFound(_)) {
                report.error = Some(e.to_string());
            }
            return report;
        }
    };

    for f in exif.fields() {
        let tag = f.tag.to_string();
        let sensitive = is_sensitive(&tag);
        if sensitive {
            report.sensitive_count += 1;
        }
        report.fields.push(ExifField {
            label: f.tag.description().unwrap_or("").to_string(),
            group: if tag.starts_with("GPS") {
                "GPS".into()
            } else if f.ifd_num == exif::In::THUMBNAIL {
                "Thumbnail".into()
            } else {
                "Image".into()
            },
            value: f.display_value().with_unit(&exif).to_string(),
            tag,
            sensitive,
        });
    }
    report.has_exif = !report.fields.is_empty();

    let lat = exif.get_field(exif::Tag::GPSLatitude, exif::In::PRIMARY);
    let lon = exif.get_field(exif::Tag::GPSLongitude, exif::In::PRIMARY);
    if let (Some(lat), Some(lon)) = (lat, lon) {
        let lat_ref = text_of(&exif, exif::Tag::GPSLatitudeRef).unwrap_or_else(|| "N".into());
        let lon_ref = text_of(&exif, exif::Tag::GPSLongitudeRef).unwrap_or_else(|| "E".into());
        if let (Some(a), Some(b)) = (
            dms_to_deg(&rationals(lat), &lat_ref),
            dms_to_deg(&rationals(lon), &lon_ref),
        ) {
            report.gps = Some((a, b));
        }
    }

    let make = text_of(&exif, exif::Tag::Make);
    let model = text_of(&exif, exif::Tag::Model);
    report.camera = match (make, model) {
        (Some(a), Some(b)) if b.starts_with(&a) => Some(b),
        (Some(a), Some(b)) => Some(format!("{} {}", a, b)),
        (a, b) => a.or(b),
    };
    report.taken =
        text_of(&exif, exif::Tag::DateTimeOriginal).or_else(|| text_of(&exif, exif::Tag::DateTime));
    report.software = text_of(&exif, exif::Tag::Software);
    report
}

pub fn read_many(inputs: &[String]) -> Vec<ExifReport> {
    inputs.iter().map(|p| read(p)).collect()
}

// ── Limpeza ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Jpeg,
    Png,
    Webp,
}

pub fn kind_of(data: &[u8]) -> Option<Kind> {
    if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(Kind::Jpeg)
    } else if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(Kind::Png)
    } else if data.len() > 12 && data.starts_with(b"RIFF") && &data[8..12] == b"WEBP" {
        Some(Kind::Webp)
    } else {
        None
    }
}

/// Marcadores JPEG jogados fora: APP1 (EXIF e XMP), APP13 (IPTC/Photoshop) e
/// o comentário. APP0 (JFIF), APP2 (perfil ICC) e APP14 (Adobe) ficam — são
/// o que mantém a cor certa.
fn strip_jpeg(data: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut out = Vec::with_capacity(data.len());
    out.extend_from_slice(&data[0..2]);
    let mut i = 2usize;
    while i + 1 < data.len() {
        if data[i] != 0xFF {
            // Fora de sincronia: copia o resto como está em vez de corromper.
            out.extend_from_slice(&data[i..]);
            return Ok(out);
        }
        let marker = data[i + 1];
        if marker == 0xD8 || (0xD0..=0xD7).contains(&marker) || marker == 0x01 || marker == 0xFF {
            out.extend_from_slice(&data[i..i + 2]);
            i += 2;
            continue;
        }
        if marker == 0xDA || marker == 0xD9 {
            out.extend_from_slice(&data[i..]);
            return Ok(out);
        }
        if i + 3 >= data.len() {
            out.extend_from_slice(&data[i..]);
            return Ok(out);
        }
        let len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
        let end = (i + 2 + len).min(data.len());
        let drop = marker == 0xE1 || marker == 0xED || marker == 0xFE;
        if !drop {
            out.extend_from_slice(&data[i..end]);
        }
        i = end;
    }
    Ok(out)
}

const PNG_DROP: &[&[u8; 4]] = &[b"tEXt", b"zTXt", b"iTXt", b"eXIf", b"tIME", b"dSIG"];

fn strip_png(data: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut out = Vec::with_capacity(data.len());
    out.extend_from_slice(&data[0..8]);
    let mut i = 8usize;
    while i + 8 <= data.len() {
        let len = u32::from_be_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]) as usize;
        let ctype: [u8; 4] = [data[i + 4], data[i + 5], data[i + 6], data[i + 7]];
        let end = i
            .checked_add(12)
            .and_then(|v| v.checked_add(len))
            .ok_or_else(|| anyhow!("PNG com tamanho de chunk inválido"))?;
        if end > data.len() {
            return Err(anyhow!("PNG truncado"));
        }
        if !PNG_DROP.iter().any(|d| d.as_slice() == ctype) {
            out.extend_from_slice(&data[i..end]);
        }
        i = end;
        if &ctype == b"IEND" {
            break;
        }
    }
    Ok(out)
}

fn strip_webp(data: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut body: Vec<u8> = Vec::with_capacity(data.len());
    body.extend_from_slice(b"WEBP");
    let mut i = 12usize;
    while i + 8 <= data.len() {
        let fourcc: [u8; 4] = [data[i], data[i + 1], data[i + 2], data[i + 3]];
        let len = u32::from_le_bytes([data[i + 4], data[i + 5], data[i + 6], data[i + 7]]) as usize;
        let padded = len + (len & 1);
        let end = (i + 8 + padded).min(data.len());
        let drop = &fourcc == b"EXIF" || &fourcc == b"XMP ";
        if !drop {
            let start = body.len();
            body.extend_from_slice(&data[i..end]);
            // VP8X guarda em flags quais chunks existem; sem limpar os bits o
            // decodificador procura um EXIF que não está mais lá.
            if &fourcc == b"VP8X" && body.len() > start + 8 {
                body[start + 8] &= !(0x08 | 0x04);
            }
        }
        i = end;
    }
    let mut out = Vec::with_capacity(body.len() + 8);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

pub fn strip_bytes(data: &[u8]) -> anyhow::Result<Vec<u8>> {
    match kind_of(data) {
        Some(Kind::Jpeg) => strip_jpeg(data),
        Some(Kind::Png) => strip_png(data),
        Some(Kind::Webp) => strip_webp(data),
        None => Err(anyhow!(
            "formato sem limpeza sem perda aqui — só JPEG, PNG e WebP"
        )),
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct StripOptions {
    pub inputs: Vec<String>,
    /// Sobrescreve o arquivo original em vez de criar uma cópia limpa.
    #[serde(default)]
    pub in_place: bool,
    #[serde(default)]
    pub output_dir: String,
    #[serde(default)]
    pub suffix: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct StripItem {
    pub input: String,
    pub output: Option<String>,
    pub bytes_before: u64,
    pub bytes_after: u64,
    pub ok: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StripResult {
    pub items: Vec<StripItem>,
}

pub fn strip(opts: &StripOptions, progress: &super::ProgressFn) -> StripResult {
    let total = opts.inputs.len() as u64;
    let mut items = Vec::new();
    for (i, input) in opts.inputs.iter().enumerate() {
        super::report(
            progress,
            "img-exif",
            "progress",
            i as u64,
            Some(total),
            Some(input.clone()),
        );
        items.push(strip_one(opts, input));
    }
    super::report(progress, "img-exif", "done", total, Some(total), None);
    StripResult { items }
}

fn strip_one(opts: &StripOptions, input: &str) -> StripItem {
    let inp = Path::new(input);
    let before = std::fs::metadata(inp).map(|m| m.len()).unwrap_or(0);
    let fail = |e: String| StripItem {
        input: input.to_string(),
        output: None,
        bytes_before: before,
        bytes_after: 0,
        ok: false,
        error: Some(e),
    };
    let data = match std::fs::read(inp) {
        Ok(d) => d,
        Err(e) => return fail(e.to_string()),
    };
    let cleaned = match strip_bytes(&data) {
        Ok(c) => c,
        Err(e) => return fail(e.to_string()),
    };
    let out = if opts.in_place {
        inp.to_path_buf()
    } else {
        let dir = if opts.output_dir.trim().is_empty() {
            inp.parent().map(|p| p.to_path_buf()).unwrap_or_default()
        } else {
            PathBuf::from(opts.output_dir.trim())
        };
        if let Err(e) = std::fs::create_dir_all(&dir) {
            return fail(e.to_string());
        }
        let stem = inp
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "imagem".into());
        let ext = inp
            .extension()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "jpg".into());
        let suffix = if opts.suffix.is_empty() {
            "-clean"
        } else {
            opts.suffix.as_str()
        };
        dir.join(format!("{}{}.{}", stem, suffix, ext))
    };
    if let Err(e) = std::fs::write(&out, &cleaned) {
        return fail(e.to_string());
    }
    StripItem {
        input: input.to_string(),
        output: Some(out.to_string_lossy().to_string()),
        bytes_before: before,
        bytes_after: cleaned.len() as u64,
        ok: true,
        error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dms_handles_hemispheres() {
        let sp = dms_to_deg(&[23.0, 33.0, 0.0], "S").unwrap();
        assert!((sp + 23.55).abs() < 1e-9);
        assert!(dms_to_deg(&[10.0, 30.0, 0.0], "E").unwrap() > 0.0);
        assert!(dms_to_deg(&[], "N").is_none());
    }

    #[test]
    fn sensitive_covers_gps_and_serials() {
        assert!(is_sensitive("GPSLatitude"));
        assert!(is_sensitive("BodySerialNumber"));
        assert!(!is_sensitive("ExposureTime"));
    }

    fn jpeg_with_exif() -> Vec<u8> {
        let mut v = vec![0xFF, 0xD8];
        // APP0 (JFIF) fica.
        v.extend_from_slice(&[0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00]);
        // APP1 (Exif) sai.
        v.extend_from_slice(&[0xFF, 0xE1, 0x00, 0x08]);
        v.extend_from_slice(b"Exif\0\0");
        // Comentário sai.
        v.extend_from_slice(&[0xFF, 0xFE, 0x00, 0x05, b'o', b'i', b'!']);
        // SOS e "dados".
        v.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x03, 0x00, 0xAA, 0xBB, 0xFF, 0xD9]);
        v
    }

    #[test]
    fn jpeg_loses_exif_and_keeps_pixels() {
        let src = jpeg_with_exif();
        let out = strip_bytes(&src).unwrap();
        assert_eq!(kind_of(&out), Some(Kind::Jpeg));
        assert!(!out.windows(4).any(|w| w == b"Exif"));
        assert!(!out.windows(2).any(|w| w == [0xFF, 0xFE]));
        // APP0 preservado e o scan intacto até o EOI.
        assert!(out.windows(2).any(|w| w == [0xFF, 0xE0]));
        assert!(out.ends_with(&[0xAA, 0xBB, 0xFF, 0xD9]));
        // Rodar de novo não muda mais nada.
        assert_eq!(strip_bytes(&out).unwrap(), out);
    }

    fn png_chunk(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut v = (payload.len() as u32).to_be_bytes().to_vec();
        v.extend_from_slice(kind);
        v.extend_from_slice(payload);
        v.extend_from_slice(&[0, 0, 0, 0]);
        v
    }

    #[test]
    fn png_loses_text_and_keeps_idat() {
        let mut src = b"\x89PNG\r\n\x1a\n".to_vec();
        src.extend(png_chunk(b"IHDR", &[0; 13]));
        src.extend(png_chunk(b"tEXt", b"Comment\0segredo"));
        src.extend(png_chunk(b"eXIf", b"II*\0"));
        src.extend(png_chunk(b"IDAT", &[1, 2, 3]));
        src.extend(png_chunk(b"IEND", b""));
        let out = strip_bytes(&src).unwrap();
        assert!(!out.windows(4).any(|w| w == b"tEXt"));
        assert!(!out.windows(4).any(|w| w == b"eXIf"));
        assert!(out.windows(4).any(|w| w == b"IDAT"));
        assert!(out.windows(4).any(|w| w == b"IHDR"));
        assert!(out.ends_with(b"IEND\0\0\0\0"));
    }

    fn riff_chunk(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut v = kind.to_vec();
        v.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        v.extend_from_slice(payload);
        if payload.len() % 2 == 1 {
            v.push(0);
        }
        v
    }

    #[test]
    fn webp_drops_exif_and_clears_vp8x_flag() {
        let mut body = b"WEBP".to_vec();
        // VP8X com o bit de EXIF (0x08) ligado.
        body.extend(riff_chunk(b"VP8X", &[0x08, 0, 0, 0, 0, 0, 0, 0, 0, 0]));
        body.extend(riff_chunk(b"VP8 ", &[9, 9, 9]));
        body.extend(riff_chunk(b"EXIF", b"II*\0segredo"));
        let mut src = b"RIFF".to_vec();
        src.extend_from_slice(&(body.len() as u32).to_le_bytes());
        src.extend_from_slice(&body);

        let out = strip_bytes(&src).unwrap();
        assert_eq!(kind_of(&out), Some(Kind::Webp));
        assert!(!out.windows(4).any(|w| w == b"EXIF"));
        assert!(out.windows(4).any(|w| w == b"VP8 "));
        let size = u32::from_le_bytes([out[4], out[5], out[6], out[7]]) as usize;
        assert_eq!(size, out.len() - 8, "tamanho do RIFF tem que bater");
        let vp8x = out.windows(4).position(|w| w == b"VP8X").unwrap();
        assert_eq!(out[vp8x + 8] & 0x08, 0, "o bit de EXIF ficou ligado");
    }

    /// Monta um JPEG com um APP1/EXIF de verdade: IFD0 com Make e Model, e
    /// um IFD de GPS com latitude sul e longitude oeste (São Paulo).
    fn jpeg_with_gps() -> Vec<u8> {
        fn entry(out: &mut Vec<u8>, tag: u16, kind: u16, count: u32, value: [u8; 4]) {
            out.extend_from_slice(&tag.to_le_bytes());
            out.extend_from_slice(&kind.to_le_bytes());
            out.extend_from_slice(&count.to_le_bytes());
            out.extend_from_slice(&value);
        }
        // Mapa de offsets (relativos ao início do cabeçalho TIFF).
        const MAKE_AT: u32 = 50;
        const MODEL_AT: u32 = 56;
        const GPS_IFD_AT: u32 = 64;
        const LAT_AT: u32 = 118;
        const LON_AT: u32 = 142;

        let mut t: Vec<u8> = Vec::new();
        t.extend_from_slice(b"II*\0");
        t.extend_from_slice(&8u32.to_le_bytes());
        t.extend_from_slice(&3u16.to_le_bytes());
        entry(&mut t, 0x010F, 2, 6, MAKE_AT.to_le_bytes());
        entry(&mut t, 0x0110, 2, 5, MODEL_AT.to_le_bytes());
        entry(&mut t, 0x8825, 4, 1, GPS_IFD_AT.to_le_bytes());
        t.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(t.len(), MAKE_AT as usize);
        t.extend_from_slice(b"OmniG\0");
        t.extend_from_slice(b"Test\0");
        while t.len() < GPS_IFD_AT as usize {
            t.push(0);
        }

        t.extend_from_slice(&4u16.to_le_bytes());
        entry(&mut t, 0x0001, 2, 2, *b"S\0\0\0");
        entry(&mut t, 0x0002, 5, 3, LAT_AT.to_le_bytes());
        entry(&mut t, 0x0003, 2, 2, *b"W\0\0\0");
        entry(&mut t, 0x0004, 5, 3, LON_AT.to_le_bytes());
        t.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(t.len(), LAT_AT as usize);
        for (num, den) in [(23u32, 1u32), (33, 1), (0, 1)] {
            t.extend_from_slice(&num.to_le_bytes());
            t.extend_from_slice(&den.to_le_bytes());
        }
        assert_eq!(t.len(), LON_AT as usize);
        for (num, den) in [(46u32, 1u32), (38, 1), (0, 1)] {
            t.extend_from_slice(&num.to_le_bytes());
            t.extend_from_slice(&den.to_le_bytes());
        }

        let mut app1 = b"Exif\0\0".to_vec();
        app1.extend_from_slice(&t);
        let mut jpeg = vec![0xFF, 0xD8];
        jpeg.extend_from_slice(&[0xFF, 0xE1]);
        jpeg.extend_from_slice(&((app1.len() + 2) as u16).to_be_bytes());
        jpeg.extend_from_slice(&app1);
        jpeg.extend_from_slice(&[0xFF, 0xD9]);
        jpeg
    }

    #[test]
    fn reads_gps_and_camera_then_strips_them() {
        let dir = std::env::temp_dir().join("omniget-exif-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("com-gps.jpg");
        std::fs::write(&path, jpeg_with_gps()).unwrap();
        let p = path.to_string_lossy().to_string();

        let r = read(&p);
        assert!(r.error.is_none(), "erro na leitura: {:?}", r.error);
        assert!(r.has_exif);
        assert_eq!(r.camera.as_deref(), Some("OmniG Test"));
        let (lat, lon) = r.gps.expect("não achou o GPS");
        assert!((lat + 23.55).abs() < 1e-6, "latitude: {}", lat);
        assert!((lon + 46.6333333).abs() < 1e-5, "longitude: {}", lon);
        assert!(
            r.sensitive_count >= 4,
            "campos de GPS não marcados como sensíveis"
        );

        let opts = StripOptions {
            inputs: vec![p.clone()],
            in_place: false,
            output_dir: String::new(),
            suffix: String::new(),
        };
        let res = strip(&opts, &crate::core::tools::noop_progress());
        let item = &res.items[0];
        assert!(item.ok, "falhou: {:?}", item.error);
        let clean = read(item.output.as_ref().unwrap());
        assert!(!clean.has_exif, "o EXIF sobreviveu à limpeza");
        assert!(clean.gps.is_none());
        assert!(item.bytes_after < item.bytes_before);
    }

    #[test]
    fn unknown_container_is_refused() {
        assert!(strip_bytes(b"not an image at all").is_err());
    }
}
