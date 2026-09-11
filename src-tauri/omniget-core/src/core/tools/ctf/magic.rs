//! "Que arquivo é esse?" — assinatura de formato (magic bytes) e as cadeias
//! de texto legível de dentro do binário, que é por onde uma análise começa.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct TypeGuess {
    pub label: String,
    pub extension: String,
    pub mime: String,
    /// Onde a assinatura foi achada (0 = começo do arquivo).
    pub offset: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct MagicReport {
    pub path: String,
    pub bytes: u64,
    pub guess: Option<TypeGuess>,
    /// Assinaturas achadas depois do começo: arquivo embutido em arquivo.
    pub embedded: Vec<TypeGuess>,
    pub entropy: f64,
    pub strings: Vec<String>,
    /// Extensão do nome não bate com o conteúdo.
    pub extension_mismatch: bool,
}

/// (assinatura, deslocamento fixo, rótulo, extensão, mime)
const SIGNATURES: &[(&[u8], usize, &str, &str, &str)] = &[
    (b"\x89PNG\r\n\x1a\n", 0, "Imagem PNG", "png", "image/png"),
    (b"\xFF\xD8\xFF", 0, "Imagem JPEG", "jpg", "image/jpeg"),
    (b"GIF87a", 0, "Imagem GIF", "gif", "image/gif"),
    (b"GIF89a", 0, "Imagem GIF", "gif", "image/gif"),
    (b"BM", 0, "Imagem BMP", "bmp", "image/bmp"),
    (b"II*\x00", 0, "Imagem TIFF", "tiff", "image/tiff"),
    (b"MM\x00*", 0, "Imagem TIFF", "tiff", "image/tiff"),
    (
        b"RIFF",
        0,
        "Contêiner RIFF (WAV/AVI/WebP)",
        "riff",
        "application/octet-stream",
    ),
    (b"%PDF-", 0, "Documento PDF", "pdf", "application/pdf"),
    (
        b"PK\x03\x04",
        0,
        "Zip (ou docx/xlsx/jar/apk)",
        "zip",
        "application/zip",
    ),
    (b"PK\x05\x06", 0, "Zip vazio", "zip", "application/zip"),
    (
        b"Rar!\x1a\x07",
        0,
        "Arquivo RAR",
        "rar",
        "application/vnd.rar",
    ),
    (
        b"7z\xBC\xAF\x27\x1C",
        0,
        "Arquivo 7-Zip",
        "7z",
        "application/x-7z-compressed",
    ),
    (b"\x1F\x8B", 0, "Gzip", "gz", "application/gzip"),
    (b"BZh", 0, "Bzip2", "bz2", "application/x-bzip2"),
    (b"\xFD7zXZ\x00", 0, "XZ", "xz", "application/x-xz"),
    (b"ustar", 257, "Tar", "tar", "application/x-tar"),
    (
        b"\x7FELF",
        0,
        "Executável ELF (Linux)",
        "elf",
        "application/x-elf",
    ),
    (
        b"MZ",
        0,
        "Executável do Windows (PE)",
        "exe",
        "application/vnd.microsoft.portable-executable",
    ),
    (
        b"\xCF\xFA\xED\xFE",
        0,
        "Executável Mach-O (macOS)",
        "macho",
        "application/x-mach-binary",
    ),
    (
        b"\xCA\xFE\xBA\xBE",
        0,
        "Java class ou Mach-O universal",
        "class",
        "application/java-vm",
    ),
    (
        b"\x00\x61\x73\x6D",
        0,
        "WebAssembly",
        "wasm",
        "application/wasm",
    ),
    (b"OggS", 0, "Ogg", "ogg", "audio/ogg"),
    (b"fLaC", 0, "Áudio FLAC", "flac", "audio/flac"),
    (b"ID3", 0, "Áudio MP3 com ID3", "mp3", "audio/mpeg"),
    (b"ftyp", 4, "MP4/MOV/M4A", "mp4", "video/mp4"),
    (
        b"\x1A\x45\xDF\xA3",
        0,
        "Matroska (mkv/webm)",
        "mkv",
        "video/x-matroska",
    ),
    (
        b"SQLite format 3\x00",
        0,
        "Banco SQLite",
        "sqlite",
        "application/vnd.sqlite3",
    ),
    (
        b"-----BEGIN ",
        0,
        "Chave/certificado PEM",
        "pem",
        "application/x-pem-file",
    ),
    (
        b"\x25\x21PS",
        0,
        "PostScript",
        "ps",
        "application/postscript",
    ),
    (b"wOFF", 0, "Fonte WOFF", "woff", "font/woff"),
    (b"wOF2", 0, "Fonte WOFF2", "woff2", "font/woff2"),
    (
        b"\x00\x01\x00\x00\x00",
        0,
        "Fonte TrueType",
        "ttf",
        "font/ttf",
    ),
    (b"OTTO", 0, "Fonte OpenType", "otf", "font/otf"),
];

fn matches_at(data: &[u8], sig: &[u8], at: usize) -> bool {
    data.len() >= at + sig.len() && &data[at..at + sig.len()] == sig
}

pub fn identify(data: &[u8]) -> Option<TypeGuess> {
    SIGNATURES
        .iter()
        .find(|(sig, off, ..)| matches_at(data, sig, *off))
        .map(|(_, off, label, ext, mime)| TypeGuess {
            label: (*label).into(),
            extension: (*ext).into(),
            mime: (*mime).into(),
            offset: *off,
        })
}

/// Assinaturas depois do byte 1: é assim que se acha um zip escondido no fim
/// de um PNG, o truque de esteganografia mais comum de CTF.
pub fn find_embedded(data: &[u8], limit: usize) -> Vec<TypeGuess> {
    // Só formatos que fazem sentido "escondidos" — evita ruído de 2 bytes.
    const INTERESTING: &[&[u8]] = &[
        b"PK\x03\x04",
        b"Rar!\x1a\x07",
        b"7z\xBC\xAF\x27\x1C",
        b"\x89PNG\r\n\x1a\n",
        b"%PDF-",
        b"\xFF\xD8\xFF",
        b"\x1F\x8B",
        b"-----BEGIN ",
    ];
    let mut out = Vec::new();
    for i in 1..data.len() {
        if out.len() >= limit {
            break;
        }
        for sig in INTERESTING {
            if matches_at(data, sig, i) {
                if let Some((_, _, label, ext, mime)) =
                    SIGNATURES.iter().find(|(s, off, ..)| *off == 0 && s == sig)
                {
                    out.push(TypeGuess {
                        label: (*label).into(),
                        extension: (*ext).into(),
                        mime: (*mime).into(),
                        offset: i,
                    });
                }
            }
        }
    }
    out
}

/// Entropia de Shannon em bits por byte. Perto de 8 = comprimido ou cifrado;
/// perto de 4-5 = texto; perto de 0 = enchimento.
pub fn entropy(data: &[u8]) -> f64 {
    if data.is_empty() {
        return 0.0;
    }
    let mut counts = [0u64; 256];
    for b in data {
        counts[*b as usize] += 1;
    }
    let len = data.len() as f64;
    -counts
        .iter()
        .filter(|c| **c > 0)
        .map(|c| {
            let p = *c as f64 / len;
            p * p.log2()
        })
        .sum::<f64>()
}

/// O `strings` clássico: sequências ASCII imprimíveis de tamanho mínimo.
pub fn strings(data: &[u8], min_len: usize, limit: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = Vec::new();
    for b in data {
        if (0x20..0x7F).contains(b) || *b == b'\t' {
            cur.push(*b);
        } else {
            if cur.len() >= min_len {
                out.push(String::from_utf8_lossy(&cur).to_string());
                if out.len() >= limit {
                    return out;
                }
            }
            cur.clear();
        }
    }
    if cur.len() >= min_len && out.len() < limit {
        out.push(String::from_utf8_lossy(&cur).to_string());
    }
    out
}

pub fn report(path: &str, min_len: usize, limit: usize) -> anyhow::Result<MagicReport> {
    let data = std::fs::read(path)?;
    let guess = identify(&data);
    let ext = std::path::Path::new(path)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let mismatch = match (&guess, ext.is_empty()) {
        (Some(g), false) => {
            // zip cobre docx/xlsx/apk/jar; riff cobre wav/avi/webp.
            let family = matches!(g.extension.as_str(), "zip" | "riff" | "mp4");
            !family && g.extension != ext && !(g.extension == "jpg" && ext == "jpeg")
        }
        _ => false,
    };
    Ok(MagicReport {
        path: path.to_string(),
        bytes: data.len() as u64,
        embedded: find_embedded(&data, 20),
        entropy: entropy(&data),
        strings: strings(&data, min_len.max(4), limit.max(1)),
        extension_mismatch: mismatch,
        guess,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_is_recognized() {
        let g = identify(b"\x89PNG\r\n\x1a\n rest").unwrap();
        assert_eq!(g.extension, "png");
        assert_eq!(g.offset, 0);
    }

    #[test]
    fn mp4_signature_lives_at_offset_four() {
        let mut data = vec![0, 0, 0, 0x20];
        data.extend_from_slice(b"ftypisom");
        assert_eq!(identify(&data).unwrap().extension, "mp4");
    }

    #[test]
    fn unknown_bytes_have_no_guess() {
        assert!(identify(b"\x01\x02\x03\x04 nada").is_none());
    }

    #[test]
    fn a_zip_hidden_after_a_png_is_found() {
        let mut data = b"\x89PNG\r\n\x1a\nIHDR....IEND".to_vec();
        data.extend_from_slice(b"PK\x03\x04segredo.txt");
        let g = identify(&data).unwrap();
        assert_eq!(g.extension, "png", "o começo continua sendo PNG");
        let hidden = find_embedded(&data, 10);
        assert_eq!(hidden.len(), 1);
        assert_eq!(hidden[0].extension, "zip");
        assert_eq!(hidden[0].offset, 20, "8 do cabeçalho + 12 do corpo falso");
    }

    #[test]
    fn entropy_separates_random_from_repetitive() {
        assert!(entropy(&[0u8; 1000]) < 0.01, "tudo igual é entropia zero");
        let spread: Vec<u8> = (0..=255).cycle().take(4096).collect();
        assert!(entropy(&spread) > 7.9, "todos os bytes = 8 bits");
        assert!(entropy(b"").abs() < f64::EPSILON);
    }

    #[test]
    fn strings_finds_readable_runs() {
        let data = b"\x00\x01FLAG{isso_aqui}\x00\xFFab\x00OmniGet";
        let s = strings(data, 4, 10);
        assert_eq!(s, vec!["FLAG{isso_aqui}", "OmniGet"]);
        // "ab" tem 2 caracteres e fica de fora.
        assert!(!s.iter().any(|x| x == "ab"));
    }

    #[test]
    fn strings_respects_the_limit() {
        let data = b"aaaa\x00bbbb\x00cccc\x00dddd";
        assert_eq!(strings(data, 4, 2).len(), 2);
    }
}
