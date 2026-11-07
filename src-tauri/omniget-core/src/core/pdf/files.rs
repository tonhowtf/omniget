use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use pdfcraft_cos::{write_full, Document, SaveOptions};
use serde::Serialize;

use super::PdfError;

pub const MAX_PDF_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
pub struct PdfInfo {
    pub path: String,
    pub name: String,
    pub size_bytes: u64,
    pub pages: usize,
    pub error: Option<String>,
}

pub fn read_pdf_bytes(path: &Path) -> Result<Vec<u8>, PdfError> {
    let meta = std::fs::metadata(path)?;
    if !meta.is_file() {
        return Err(PdfError::NotPdf);
    }
    if meta.len() > MAX_PDF_BYTES {
        return Err(PdfError::Damaged("file is larger than 2 GB".into()));
    }
    let bytes = std::fs::read(path)?;
    let head = &bytes[..bytes.len().min(1024)];
    if !head.windows(5).any(|w| w == b"%PDF-") {
        return Err(PdfError::NotPdf);
    }
    Ok(bytes)
}

pub fn open(path: &Path) -> Result<Document, PdfError> {
    let doc = Document::open(Arc::new(read_pdf_bytes(path)?))?;
    if pdfcraft_organize::page_count(&doc)? == 0 {
        return Err(PdfError::Empty);
    }
    Ok(doc)
}

pub fn inspect(path: &Path) -> PdfInfo {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let size_bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let counted = std::panic::catch_unwind(|| {
        open(path).and_then(|doc| Ok(pdfcraft_organize::page_count(&doc)?))
    })
    .unwrap_or_else(|_| Err(PdfError::Damaged("the reader stopped on this file".into())));
    let (pages, error) = match counted {
        Ok(n) => (n, None),
        Err(e) => (0, Some(e.code().to_string())),
    };
    PdfInfo {
        path: path.to_string_lossy().into_owned(),
        name,
        size_bytes,
        pages,
        error,
    }
}

pub fn unique_path(wanted: &Path) -> PathBuf {
    if !wanted.exists() {
        return wanted.to_path_buf();
    }
    let dir = wanted.parent().map(Path::to_path_buf).unwrap_or_default();
    let stem = wanted
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "document".to_string());
    let ext = wanted
        .extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_else(|| "pdf".to_string());
    (1u32..)
        .map(|n| dir.join(format!("{stem} ({n}).{ext}")))
        .find(|p| !p.exists())
        .unwrap_or_else(|| wanted.to_path_buf())
}

pub fn to_bytes(doc: &Document) -> Result<Vec<u8>, PdfError> {
    Ok(write_full(doc, &SaveOptions::default())?)
}

pub fn write_atomic(bytes: &[u8], path: &Path) -> Result<(), PdfError> {
    let is_pdf = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("pdf"));
    if !is_pdf {
        return Err(PdfError::Invalid("the output must be a .pdf file"));
    }
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut tmp = tempfile::Builder::new()
        .prefix(".omniget-pdf-")
        .suffix(".tmp")
        .tempfile_in(dir)?;
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| PdfError::Io(e.error))?;
    Ok(())
}

pub fn save(doc: &Document, wanted: &Path, overwrite: bool) -> Result<(PathBuf, u64), PdfError> {
    let bytes = to_bytes(doc)?;
    let path = if overwrite {
        wanted.to_path_buf()
    } else {
        unique_path(wanted)
    };
    write_atomic(&bytes, &path)?;
    Ok((path, bytes.len() as u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_path_counts_up_past_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        let wanted = dir.path().join("book.pdf");
        assert_eq!(unique_path(&wanted), wanted);
        std::fs::write(&wanted, b"x").unwrap();
        assert_eq!(unique_path(&wanted), dir.path().join("book (1).pdf"));
        std::fs::write(dir.path().join("book (1).pdf"), b"x").unwrap();
        assert_eq!(unique_path(&wanted), dir.path().join("book (2).pdf"));
    }

    #[test]
    fn non_pdf_files_are_refused_before_parsing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.pdf");
        std::fs::write(&path, b"just text").unwrap();
        assert!(matches!(open(&path), Err(PdfError::NotPdf)));
        let info = inspect(&path);
        assert_eq!(info.error.as_deref(), Some("not_pdf"));
        assert_eq!(info.pages, 0);
    }

    #[test]
    fn damaged_files_get_an_error_instead_of_a_crash() {
        let dir = tempfile::tempdir().unwrap();
        let mut doc = Document::new_empty();
        for i in 0..3 {
            pdfcraft_organize::insert_blank_page(&mut doc, i, 200.0, 300.0).unwrap();
        }
        let good = to_bytes(&doc).unwrap();
        let path = dir.path().join("broken.pdf");
        let mut variants: Vec<Vec<u8>> = (1..good.len())
            .step_by(7)
            .map(|cut| good[..cut].to_vec())
            .collect();
        variants.extend((0..good.len()).step_by(11).map(|at| {
            let mut bad = good.clone();
            bad[at] ^= 0x5a;
            bad
        }));
        for bytes in variants {
            std::fs::write(&path, &bytes).unwrap();
            let info = inspect(&path);
            assert!(info.pages > 0 || info.error.is_some());
        }
    }

    #[test]
    fn save_never_replaces_an_existing_file_unless_asked() {
        let dir = tempfile::tempdir().unwrap();
        let wanted = dir.path().join("out.pdf");
        std::fs::write(&wanted, b"original").unwrap();
        let mut doc = Document::new_empty();
        pdfcraft_organize::insert_blank_page(&mut doc, 0, 100.0, 100.0).unwrap();
        let (path, size) = save(&doc, &wanted, false).unwrap();
        assert_eq!(path, dir.path().join("out (1).pdf"));
        assert_eq!(std::fs::read(&wanted).unwrap(), b"original");
        assert_eq!(std::fs::metadata(&path).unwrap().len(), size);
        let (path, _) = save(&doc, &wanted, true).unwrap();
        assert_eq!(path, wanted);
        assert!(std::fs::read(&wanted).unwrap().starts_with(b"%PDF-"));
        let notes = dir.path().join("notes.txt");
        std::fs::write(&notes, b"keep").unwrap();
        assert!(matches!(
            save(&doc, &notes, true),
            Err(PdfError::Invalid(_))
        ));
        assert_eq!(std::fs::read(&notes).unwrap(), b"keep");
    }
}
