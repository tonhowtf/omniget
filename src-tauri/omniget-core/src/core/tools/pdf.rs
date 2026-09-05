//! PDF: juntar, dividir, comprimir, converter, OCR e "PDF seguro". Tudo em
//! cima do PDFium que o app já gerencia (Dependências → PDFium), carregado em
//! tempo de execução com `libloading`, então não entra nenhuma crate de PDF.
//! Estudo 3 (Stirling-PDF) deu o catálogo de operações; 35 (Dangerzone) deu a
//! remontagem a partir dos pixels; Ghostscript e LibreOffice são opcionais e
//! só entram quando já estão na máquina.

use std::ffi::{c_char, c_int, c_ulong, c_void, CString};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

use super::jpeg_pdf;

type Doc = *mut c_void;
type Page = *mut c_void;
type Bitmap = *mut c_void;
type TextPage = *mut c_void;

const FPDF_ANNOT: c_int = 0x01;

#[repr(C)]
struct FileWrite {
    version: c_int,
    write_block: unsafe extern "C" fn(*mut FileWrite, *const c_void, c_ulong) -> c_int,
}

#[repr(C)]
struct Writer {
    fw: FileWrite,
    buf: Vec<u8>,
}

unsafe extern "C" fn write_block(
    this: *mut FileWrite,
    data: *const c_void,
    size: c_ulong,
) -> c_int {
    let w = &mut *(this as *mut Writer);
    let slice = std::slice::from_raw_parts(data as *const u8, size as usize);
    w.buf.extend_from_slice(slice);
    1
}

/// Ponteiros das funções do PDFium que usamos. Os nomes são a API C pública
/// (`fpdfview.h`, `fpdf_text.h`, `fpdf_ppo.h`, `fpdf_save.h`, `fpdf_doc.h`).
struct Api {
    _lib: libloading::Library,
    load_mem: unsafe extern "C" fn(*const c_void, usize, *const c_char) -> Doc,
    last_error: unsafe extern "C" fn() -> c_ulong,
    page_count: unsafe extern "C" fn(Doc) -> c_int,
    load_page: unsafe extern "C" fn(Doc, c_int) -> Page,
    page_w: unsafe extern "C" fn(Page) -> f32,
    page_h: unsafe extern "C" fn(Page) -> f32,
    close_page: unsafe extern "C" fn(Page),
    close_doc: unsafe extern "C" fn(Doc),
    bmp_create: unsafe extern "C" fn(c_int, c_int, c_int) -> Bitmap,
    bmp_fill: unsafe extern "C" fn(Bitmap, c_int, c_int, c_int, c_int, c_ulong),
    render: unsafe extern "C" fn(Bitmap, Page, c_int, c_int, c_int, c_int, c_int, c_int),
    bmp_buffer: unsafe extern "C" fn(Bitmap) -> *mut c_void,
    bmp_stride: unsafe extern "C" fn(Bitmap) -> c_int,
    bmp_destroy: unsafe extern "C" fn(Bitmap),
    text_load: unsafe extern "C" fn(Page) -> TextPage,
    text_count: unsafe extern "C" fn(TextPage) -> c_int,
    text_get: unsafe extern "C" fn(TextPage, c_int, c_int, *mut u16) -> c_int,
    text_close: unsafe extern "C" fn(TextPage),
    new_doc: unsafe extern "C" fn() -> Doc,
    import_pages: unsafe extern "C" fn(Doc, Doc, *const c_char, c_int) -> c_int,
    save_copy: unsafe extern "C" fn(Doc, *mut FileWrite, c_ulong) -> c_int,
    meta_text: unsafe extern "C" fn(Doc, *const c_char, *mut c_void, c_ulong) -> c_ulong,
}

unsafe impl Send for Api {}
unsafe impl Sync for Api {}

static API: Mutex<Option<&'static Api>> = Mutex::new(None);
/// O PDFium não é thread-safe: toda operação segura este lock.
static OPS: Mutex<()> = Mutex::new(());

unsafe fn sym<T: Copy>(lib: &libloading::Library, name: &[u8]) -> anyhow::Result<T> {
    Ok(*lib
        .get::<T>(name)
        .map_err(|e| anyhow!("PDFium sem {}: {}", String::from_utf8_lossy(name), e))?)
}

fn api() -> anyhow::Result<&'static Api> {
    let mut guard = API.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(a) = *guard {
        return Ok(a);
    }
    let path = crate::core::pdfium::resolve_path()
        .ok_or_else(|| anyhow!("PDFium nao esta instalado (Ajustes → Dependencias → PDFium)"))?;
    let lib = unsafe { libloading::Library::new(&path) }
        .map_err(|e| anyhow!("nao carregou o PDFium em {}: {}", path.display(), e))?;
    let api = unsafe {
        let init: unsafe extern "C" fn() = sym(&lib, b"FPDF_InitLibrary\0")?;
        init();
        Api {
            load_mem: sym(&lib, b"FPDF_LoadMemDocument64\0")?,
            last_error: sym(&lib, b"FPDF_GetLastError\0")?,
            page_count: sym(&lib, b"FPDF_GetPageCount\0")?,
            load_page: sym(&lib, b"FPDF_LoadPage\0")?,
            page_w: sym(&lib, b"FPDF_GetPageWidthF\0")?,
            page_h: sym(&lib, b"FPDF_GetPageHeightF\0")?,
            close_page: sym(&lib, b"FPDF_ClosePage\0")?,
            close_doc: sym(&lib, b"FPDF_CloseDocument\0")?,
            bmp_create: sym(&lib, b"FPDFBitmap_Create\0")?,
            bmp_fill: sym(&lib, b"FPDFBitmap_FillRect\0")?,
            render: sym(&lib, b"FPDF_RenderPageBitmap\0")?,
            bmp_buffer: sym(&lib, b"FPDFBitmap_GetBuffer\0")?,
            bmp_stride: sym(&lib, b"FPDFBitmap_GetStride\0")?,
            bmp_destroy: sym(&lib, b"FPDFBitmap_Destroy\0")?,
            text_load: sym(&lib, b"FPDFText_LoadPage\0")?,
            text_count: sym(&lib, b"FPDFText_CountChars\0")?,
            text_get: sym(&lib, b"FPDFText_GetText\0")?,
            text_close: sym(&lib, b"FPDFText_ClosePage\0")?,
            new_doc: sym(&lib, b"FPDF_CreateNewDocument\0")?,
            import_pages: sym(&lib, b"FPDF_ImportPages\0")?,
            save_copy: sym(&lib, b"FPDF_SaveAsCopy\0")?,
            meta_text: sym(&lib, b"FPDF_GetMetaText\0")?,
            _lib: lib,
        }
    };
    let leaked: &'static Api = Box::leak(Box::new(api));
    *guard = Some(leaked);
    Ok(leaked)
}

pub fn available() -> bool {
    crate::core::pdfium::is_installed()
}

struct Document {
    api: &'static Api,
    doc: Doc,
    // O PDFium lê da memória enquanto o documento estiver aberto.
    _data: Vec<u8>,
}

impl Document {
    fn open(api: &'static Api, path: &Path, password: Option<&str>) -> anyhow::Result<Self> {
        let data = std::fs::read(path).map_err(|e| anyhow!("nao leu {}: {}", path.display(), e))?;
        let pw = CString::new(password.unwrap_or("")).unwrap_or_default();
        let doc =
            unsafe { (api.load_mem)(data.as_ptr() as *const c_void, data.len(), pw.as_ptr()) };
        if doc.is_null() {
            let code = unsafe { (api.last_error)() };
            let why = match code {
                2 => "arquivo nao encontrado ou ilegivel",
                3 => "nao e um PDF valido",
                4 => "senha incorreta ou ausente",
                5 => "esquema de seguranca nao suportado",
                6 => "pagina invalida",
                _ => "erro desconhecido",
            };
            return Err(anyhow!("{}: {}", path.display(), why));
        }
        Ok(Document {
            api,
            doc,
            _data: data,
        })
    }

    fn new(api: &'static Api) -> anyhow::Result<Self> {
        let doc = unsafe { (api.new_doc)() };
        if doc.is_null() {
            return Err(anyhow!("nao criou o documento"));
        }
        Ok(Document {
            api,
            doc,
            _data: Vec::new(),
        })
    }

    fn pages(&self) -> usize {
        unsafe { (self.api.page_count)(self.doc) }.max(0) as usize
    }

    fn page(&self, index: usize) -> anyhow::Result<PageRef<'_>> {
        let page = unsafe { (self.api.load_page)(self.doc, index as c_int) };
        if page.is_null() {
            return Err(anyhow!("pagina {} nao abriu", index + 1));
        }
        Ok(PageRef {
            api: self.api,
            page,
            _doc: self,
        })
    }

    /// Copia páginas de `src` (números 1-based, na ordem dada) para o fim.
    fn import(&self, src: &Document, pages: &[usize]) -> anyhow::Result<()> {
        let spec = pages
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let spec = CString::new(spec)?;
        let at = self.pages() as c_int;
        let ok = unsafe { (self.api.import_pages)(self.doc, src.doc, spec.as_ptr(), at) };
        if ok == 0 {
            return Err(anyhow!("nao importou as paginas {:?}", pages));
        }
        Ok(())
    }

    fn to_bytes(&self) -> anyhow::Result<Vec<u8>> {
        let mut w = Writer {
            fw: FileWrite {
                version: 1,
                write_block,
            },
            buf: Vec::new(),
        };
        let ok = unsafe { (self.api.save_copy)(self.doc, &mut w.fw as *mut FileWrite, 0) };
        if ok == 0 {
            return Err(anyhow!("nao gravou o PDF"));
        }
        Ok(w.buf)
    }

    fn save(&self, path: &Path) -> anyhow::Result<u64> {
        let bytes = self.to_bytes()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, &bytes)?;
        Ok(bytes.len() as u64)
    }

    fn meta(&self, tag: &str) -> Option<String> {
        let tag = CString::new(tag).ok()?;
        let len = unsafe { (self.api.meta_text)(self.doc, tag.as_ptr(), std::ptr::null_mut(), 0) }
            as usize;
        if len < 4 {
            return None;
        }
        let mut buf = vec![0u8; len];
        unsafe {
            (self.api.meta_text)(
                self.doc,
                tag.as_ptr(),
                buf.as_mut_ptr() as *mut c_void,
                len as c_ulong,
            )
        };
        let units: Vec<u16> = buf
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|u| *u != 0)
            .collect();
        let s = String::from_utf16_lossy(&units).trim().to_string();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    }
}

impl Drop for Document {
    fn drop(&mut self) {
        unsafe { (self.api.close_doc)(self.doc) }
    }
}

struct PageRef<'a> {
    api: &'static Api,
    page: Page,
    _doc: &'a Document,
}

impl PageRef<'_> {
    fn size_pt(&self) -> (f32, f32) {
        unsafe { ((self.api.page_w)(self.page), (self.api.page_h)(self.page)) }
    }

    fn render(&self, dpi: u32) -> anyhow::Result<image::RgbImage> {
        let (w_pt, h_pt) = self.size_pt();
        let scale = dpi.max(24) as f32 / 72.0;
        let w = ((w_pt * scale).round() as i32).clamp(1, 20_000);
        let h = ((h_pt * scale).round() as i32).clamp(1, 20_000);
        let bmp = unsafe { (self.api.bmp_create)(w, h, 0) };
        if bmp.is_null() {
            return Err(anyhow!("sem memoria para {}x{}", w, h));
        }
        let mut img = image::RgbImage::new(w as u32, h as u32);
        unsafe {
            (self.api.bmp_fill)(bmp, 0, 0, w, h, 0xFFFF_FFFF);
            (self.api.render)(bmp, self.page, 0, 0, w, h, 0, FPDF_ANNOT);
            let stride = (self.api.bmp_stride)(bmp) as usize;
            let buf = (self.api.bmp_buffer)(bmp) as *const u8;
            let src = std::slice::from_raw_parts(buf, stride * h as usize);
            for y in 0..h as usize {
                let row = &src[y * stride..y * stride + w as usize * 4];
                for x in 0..w as usize {
                    let p = &row[x * 4..x * 4 + 4]; // BGRx
                    img.put_pixel(x as u32, y as u32, image::Rgb([p[2], p[1], p[0]]));
                }
            }
            (self.api.bmp_destroy)(bmp);
        }
        Ok(img)
    }

    fn text(&self) -> String {
        unsafe {
            let tp = (self.api.text_load)(self.page);
            if tp.is_null() {
                return String::new();
            }
            let count = (self.api.text_count)(tp).max(0);
            let mut buf = vec![0u16; count as usize + 1];
            let n = (self.api.text_get)(tp, 0, count, buf.as_mut_ptr());
            (self.api.text_close)(tp);
            let n = (n.max(1) - 1) as usize;
            String::from_utf16_lossy(&buf[..n.min(buf.len())])
                .replace("\r\n", "\n")
                .replace('\r', "\n")
        }
    }
}

impl Drop for PageRef<'_> {
    fn drop(&mut self) {
        unsafe { (self.api.close_page)(self.page) }
    }
}

/// "1-3, 5, 8-" → páginas 1-based na ordem dada. Vazio ou "all" = todas.
pub fn parse_ranges(spec: &str, total: usize) -> anyhow::Result<Vec<usize>> {
    let spec = spec.trim();
    if spec.is_empty() || spec.eq_ignore_ascii_case("all") || spec == "*" {
        return Ok((1..=total).collect());
    }
    let mut out = Vec::new();
    for part in spec
        .split([',', ' '])
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        if let Some((a, b)) = part.split_once('-') {
            let a: usize = if a.trim().is_empty() {
                1
            } else {
                a.trim()
                    .parse()
                    .map_err(|_| anyhow!("intervalo invalido: {}", part))?
            };
            let b: usize = if b.trim().is_empty() {
                total
            } else {
                b.trim()
                    .parse()
                    .map_err(|_| anyhow!("intervalo invalido: {}", part))?
            };
            if a == 0 || b == 0 || a > total || b > total {
                return Err(anyhow!(
                    "pagina fora do documento ({} paginas): {}",
                    total,
                    part
                ));
            }
            if a <= b {
                out.extend(a..=b);
            } else {
                out.extend((b..=a).rev());
            }
        } else {
            let n: usize = part
                .parse()
                .map_err(|_| anyhow!("pagina invalida: {}", part))?;
            if n == 0 || n > total {
                return Err(anyhow!(
                    "pagina fora do documento ({} paginas): {}",
                    total,
                    n
                ));
            }
            out.push(n);
        }
    }
    if out.is_empty() {
        return Err(anyhow!("nenhuma pagina selecionada"));
    }
    Ok(out)
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "documento".into())
}

fn out_dir_for(input: &Path, output_dir: &str) -> PathBuf {
    if output_dir.trim().is_empty() {
        input
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    } else {
        PathBuf::from(output_dir.trim())
    }
}

/// Não sobrescreve: `nome.pdf`, `nome (2).pdf`, …
fn unique(path: PathBuf) -> PathBuf {
    if !path.exists() {
        return path;
    }
    let stem = stem(&path);
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_default();
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    for i in 2..1000 {
        let cand = dir.join(if ext.is_empty() {
            format!("{} ({})", stem, i)
        } else {
            format!("{} ({}).{}", stem, i, ext)
        });
        if !cand.exists() {
            return cand;
        }
    }
    path
}

fn encode(img: &image::RgbImage, format: &str, quality: u8) -> anyhow::Result<Vec<u8>> {
    let mut buf = Vec::new();
    if format == "png" {
        img.write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png)?;
    } else {
        let mut enc =
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, quality.clamp(10, 100));
        enc.encode_image(img)?;
    }
    Ok(buf)
}

fn report(p: &super::ProgressFn, stage: &str, done: u64, total: Option<u64>, msg: Option<String>) {
    super::report(p, "pdf", stage, done, total, msg);
}

// ── Status ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct PdfStatus {
    pub pdfium: bool,
    pub pdfium_version: Option<String>,
    pub ghostscript: Option<String>,
    pub libreoffice: Option<String>,
    pub tesseract: bool,
    pub tesseract_langs: Vec<String>,
}

async fn find_gs() -> Option<PathBuf> {
    for name in ["gs", "gswin64c", "gswin32c"] {
        if let Some(p) = crate::core::dependencies::find_tool(name).await {
            return Some(p);
        }
    }
    if cfg!(target_os = "windows") {
        for base in [r"C:\Program Files\gs", r"C:\Program Files (x86)\gs"] {
            if let Ok(rd) = std::fs::read_dir(base) {
                for e in rd.flatten() {
                    for exe in ["gswin64c.exe", "gswin32c.exe"] {
                        let p = e.path().join("bin").join(exe);
                        if p.exists() {
                            return Some(p);
                        }
                    }
                }
            }
        }
    }
    None
}

async fn find_soffice() -> Option<PathBuf> {
    if let Some(p) = crate::core::dependencies::find_tool("soffice").await {
        return Some(p);
    }
    let candidates: &[&str] = if cfg!(target_os = "windows") {
        &[
            r"C:\Program Files\LibreOffice\program\soffice.exe",
            r"C:\Program Files (x86)\LibreOffice\program\soffice.exe",
        ]
    } else if cfg!(target_os = "macos") {
        &["/Applications/LibreOffice.app/Contents/MacOS/soffice"]
    } else {
        &[
            "/usr/bin/soffice",
            "/usr/bin/libreoffice",
            "/snap/bin/libreoffice",
            "/var/lib/flatpak/exports/bin/org.libreoffice.LibreOffice",
        ]
    };
    candidates.iter().map(PathBuf::from).find(|p| p.exists())
}

pub async fn status() -> PdfStatus {
    let ocr = super::ocr::status().await;
    PdfStatus {
        pdfium: available(),
        pdfium_version: crate::core::pdfium::read_version_marker(),
        ghostscript: find_gs().await.map(|p| p.to_string_lossy().to_string()),
        libreoffice: find_soffice()
            .await
            .map(|p| p.to_string_lossy().to_string()),
        tesseract: ocr.installed,
        tesseract_langs: ocr.languages,
    }
}

// ── Info ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct PdfInfo {
    pub path: String,
    pub pages: usize,
    pub bytes: u64,
    pub width_pt: f32,
    pub height_pt: f32,
    pub title: Option<String>,
    pub author: Option<String>,
    pub has_text: bool,
}

pub fn info(path: &str, password: Option<&str>) -> anyhow::Result<PdfInfo> {
    let api = api()?;
    let _g = OPS.lock().unwrap_or_else(|p| p.into_inner());
    let p = Path::new(path);
    let doc = Document::open(api, p, password)?;
    let pages = doc.pages();
    let (w, h, has_text) = if pages > 0 {
        let page = doc.page(0)?;
        let (w, h) = page.size_pt();
        (w, h, !page.text().trim().is_empty())
    } else {
        (0.0, 0.0, false)
    };
    Ok(PdfInfo {
        path: path.to_string(),
        pages,
        bytes: std::fs::metadata(p).map(|m| m.len()).unwrap_or(0),
        width_pt: w,
        height_pt: h,
        title: doc.meta("Title"),
        author: doc.meta("Author"),
        has_text,
    })
}

// ── Juntar ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct MergeOptions {
    pub inputs: Vec<String>,
    pub output: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PdfOut {
    pub output: String,
    pub pages: usize,
    pub bytes: u64,
}

pub fn merge(opts: &MergeOptions, progress: &super::ProgressFn) -> anyhow::Result<PdfOut> {
    if opts.inputs.len() < 2 {
        return Err(anyhow!("escolha pelo menos dois PDFs"));
    }
    let api = api()?;
    let _g = OPS.lock().unwrap_or_else(|p| p.into_inner());
    let dest = Document::new(api)?;
    let total = opts.inputs.len() as u64;
    for (i, input) in opts.inputs.iter().enumerate() {
        report(
            progress,
            "progress",
            i as u64,
            Some(total),
            Some(input.clone()),
        );
        let src = Document::open(api, Path::new(input), None)?;
        let all: Vec<usize> = (1..=src.pages()).collect();
        if !all.is_empty() {
            dest.import(&src, &all)?;
        }
    }
    let out = unique(PathBuf::from(opts.output.trim()));
    let bytes = dest.save(&out)?;
    report(progress, "done", total, Some(total), None);
    Ok(PdfOut {
        output: out.to_string_lossy().to_string(),
        pages: dest.pages(),
        bytes,
    })
}

// ── Dividir ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct SplitOptions {
    pub input: String,
    /// "each" (uma página por arquivo) | "every" (blocos de N) | "ranges"
    /// ("1-3; 4-10", um arquivo por trecho) | "extract" (um arquivo com "1,3,5-7").
    pub mode: String,
    #[serde(default)]
    pub every: usize,
    #[serde(default)]
    pub ranges: String,
    #[serde(default)]
    pub output_dir: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PdfOuts {
    pub outputs: Vec<String>,
    pub pages: usize,
}

pub fn split(opts: &SplitOptions, progress: &super::ProgressFn) -> anyhow::Result<PdfOuts> {
    let api = api()?;
    let _g = OPS.lock().unwrap_or_else(|p| p.into_inner());
    let input = Path::new(opts.input.trim());
    let src = Document::open(api, input, None)?;
    let total = src.pages();
    if total == 0 {
        return Err(anyhow!("o PDF nao tem paginas"));
    }
    let groups: Vec<(String, Vec<usize>)> = match opts.mode.as_str() {
        "each" => (1..=total)
            .map(|p| (format!("p{:03}", p), vec![p]))
            .collect(),
        "every" => {
            let n = opts.every.max(1);
            (1..=total)
                .collect::<Vec<_>>()
                .chunks(n)
                .map(|c| (format!("p{:03}-{:03}", c[0], c[c.len() - 1]), c.to_vec()))
                .collect()
        }
        "ranges" => opts
            .ranges
            .split(';')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| {
                parse_ranges(s, total).map(|pages| (s.replace(' ', "").replace(',', "_"), pages))
            })
            .collect::<anyhow::Result<Vec<_>>>()?,
        _ => vec![(
            opts.ranges.replace(' ', "").replace(',', "_"),
            parse_ranges(&opts.ranges, total)?,
        )],
    };
    if groups.is_empty() {
        return Err(anyhow!("nenhum trecho para extrair"));
    }
    let dir = out_dir_for(input, &opts.output_dir);
    std::fs::create_dir_all(&dir)?;
    let base = stem(input);
    let mut outputs = Vec::new();
    let n = groups.len() as u64;
    for (i, (label, pages)) in groups.iter().enumerate() {
        report(progress, "progress", i as u64, Some(n), Some(label.clone()));
        let dest = Document::new(api)?;
        dest.import(&src, pages)?;
        let path = unique(dir.join(format!("{} {}.pdf", base, label)));
        dest.save(&path)?;
        outputs.push(path.to_string_lossy().to_string());
    }
    report(progress, "done", n, Some(n), None);
    Ok(PdfOuts {
        outputs,
        pages: total,
    })
}

// ── Renderizar (PDF → imagens) ─────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct RenderOptions {
    pub input: String,
    #[serde(default)]
    pub pages: String,
    #[serde(default)]
    pub dpi: u32,
    /// "png" | "jpg"
    #[serde(default)]
    pub format: String,
    #[serde(default)]
    pub quality: u8,
    #[serde(default)]
    pub output_dir: String,
}

pub fn render(opts: &RenderOptions, progress: &super::ProgressFn) -> anyhow::Result<Vec<String>> {
    let api = api()?;
    let _g = OPS.lock().unwrap_or_else(|p| p.into_inner());
    let input = Path::new(opts.input.trim());
    let doc = Document::open(api, input, None)?;
    let pages = parse_ranges(&opts.pages, doc.pages())?;
    let dpi = if opts.dpi == 0 { 150 } else { opts.dpi };
    let format = if opts.format == "png" { "png" } else { "jpg" };
    let quality = if opts.quality == 0 { 90 } else { opts.quality };
    let dir = out_dir_for(input, &opts.output_dir).join(format!("{} - imagens", stem(input)));
    std::fs::create_dir_all(&dir)?;
    let mut outputs = Vec::new();
    let n = pages.len() as u64;
    for (i, p) in pages.iter().enumerate() {
        report(
            progress,
            "progress",
            i as u64,
            Some(n),
            Some(format!("pagina {}", p)),
        );
        let img = doc.page(p - 1)?.render(dpi)?;
        let bytes = encode(&img, format, quality)?;
        let path = dir.join(format!("{} - {:03}.{}", stem(input), p, format));
        std::fs::write(&path, bytes)?;
        outputs.push(path.to_string_lossy().to_string());
    }
    report(progress, "done", n, Some(n), None);
    Ok(outputs)
}

// ── Texto ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct TextResult {
    pub text: String,
    pub output: Option<String>,
    pub pages: usize,
}

pub fn to_text(
    input: &str,
    pages: &str,
    save: bool,
    output_dir: &str,
) -> anyhow::Result<TextResult> {
    let api = api()?;
    let _g = OPS.lock().unwrap_or_else(|p| p.into_inner());
    let path = Path::new(input.trim());
    let doc = Document::open(api, path, None)?;
    let pages = parse_ranges(pages, doc.pages())?;
    let mut text = String::new();
    for p in &pages {
        let t = doc.page(p - 1)?.text();
        if !text.is_empty() {
            text.push_str("\n\n");
        }
        text.push_str(t.trim_end());
    }
    let output = if save {
        let out = unique(out_dir_for(path, output_dir).join(format!("{}.txt", stem(path))));
        std::fs::write(&out, &text)?;
        Some(out.to_string_lossy().to_string())
    } else {
        None
    };
    Ok(TextResult {
        text,
        output,
        pages: pages.len(),
    })
}

// ── Imagens → PDF ──────────────────────────────────────────────────────

pub fn images_to_pdf(
    inputs: &[String],
    output: &str,
    quality: u8,
    progress: &super::ProgressFn,
) -> anyhow::Result<PdfOut> {
    if inputs.is_empty() {
        return Err(anyhow!("escolha pelo menos uma imagem"));
    }
    let quality = if quality == 0 { 90 } else { quality };
    let mut jpegs = Vec::with_capacity(inputs.len());
    let n = inputs.len() as u64;
    for (i, input) in inputs.iter().enumerate() {
        report(progress, "progress", i as u64, Some(n), Some(input.clone()));
        let data = std::fs::read(input).map_err(|e| anyhow!("nao leu {}: {}", input, e))?;
        if jpeg_pdf::is_jpeg(&data) {
            jpegs.push(data);
        } else {
            let img = image::load_from_memory(&data)
                .map_err(|e| anyhow!("{}: {}", input, e))?
                .to_rgb8();
            jpegs.push(encode(&img, "jpg", quality)?);
        }
    }
    let pdf = jpeg_pdf::build_pdf(&jpegs)?;
    let out = unique(PathBuf::from(output.trim()));
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&out, &pdf)?;
    report(progress, "done", n, Some(n), None);
    Ok(PdfOut {
        output: out.to_string_lossy().to_string(),
        pages: jpegs.len(),
        bytes: pdf.len() as u64,
    })
}

// ── Rasterizar de volta para PDF (comprimir e sanitizar) ───────────────

fn rasterize(
    input: &Path,
    output: &Path,
    dpi: u32,
    quality: u8,
    progress: &super::ProgressFn,
) -> anyhow::Result<PdfOut> {
    let api = api()?;
    let _g = OPS.lock().unwrap_or_else(|p| p.into_inner());
    let doc = Document::open(api, input, None)?;
    let total = doc.pages();
    let mut jpegs = Vec::with_capacity(total);
    for i in 0..total {
        report(
            progress,
            "progress",
            i as u64,
            Some(total as u64),
            Some(format!("pagina {}", i + 1)),
        );
        let img = doc.page(i)?.render(dpi)?;
        jpegs.push(encode(&img, "jpg", quality)?);
    }
    drop(doc);
    let pdf = jpeg_pdf::build_pdf(&jpegs)?;
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(output, &pdf)?;
    report(progress, "done", total as u64, Some(total as u64), None);
    Ok(PdfOut {
        output: output.to_string_lossy().to_string(),
        pages: total,
        bytes: pdf.len() as u64,
    })
}

#[derive(Debug, Clone, Deserialize)]
pub struct CompressOptions {
    pub input: String,
    #[serde(default)]
    pub output_dir: String,
    /// "auto" (Ghostscript se existir, senao imagens) | "gs" | "raster"
    #[serde(default)]
    pub mode: String,
    /// Ghostscript: "screen" (72 dpi) | "ebook" (150) | "printer" (300)
    #[serde(default)]
    pub preset: String,
    #[serde(default)]
    pub dpi: u32,
    #[serde(default)]
    pub quality: u8,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompressResult {
    pub output: String,
    pub before: u64,
    pub after: u64,
    pub method: String,
    pub pages: usize,
}

pub async fn compress(
    opts: CompressOptions,
    progress: super::ProgressFn,
) -> anyhow::Result<CompressResult> {
    let input = PathBuf::from(opts.input.trim());
    let before = std::fs::metadata(&input)?.len();
    let output = unique(
        out_dir_for(&input, &opts.output_dir).join(format!("{} (comprimido).pdf", stem(&input))),
    );
    let gs = if opts.mode == "raster" {
        None
    } else {
        find_gs().await
    };
    if let Some(gs) = gs {
        let preset = match opts.preset.as_str() {
            "screen" | "printer" | "prepress" => opts.preset.clone(),
            _ => "ebook".to_string(),
        };
        report(&progress, "progress", 0, None, Some("ghostscript".into()));
        let o = crate::core::process::command(&gs)
            .args([
                "-sDEVICE=pdfwrite",
                "-dCompatibilityLevel=1.5",
                "-dNOPAUSE",
                "-dQUIET",
                "-dBATCH",
                "-dDetectDuplicateImages=true",
            ])
            .arg(format!("-dPDFSETTINGS=/{}", preset))
            .arg(format!("-sOutputFile={}", output.display()))
            .arg(&input)
            .output()
            .await?;
        if o.status.success() && output.exists() {
            let after = std::fs::metadata(&output)?.len();
            let pages = tokio::task::spawn_blocking({
                let out = output.clone();
                move || {
                    info(&out.to_string_lossy(), None)
                        .map(|i| i.pages)
                        .unwrap_or(0)
                }
            })
            .await
            .unwrap_or(0);
            report(&progress, "done", 1, Some(1), None);
            return Ok(CompressResult {
                output: output.to_string_lossy().to_string(),
                before,
                after,
                method: "ghostscript".into(),
                pages,
            });
        }
        if opts.mode == "gs" {
            return Err(anyhow!(
                "ghostscript falhou: {}",
                String::from_utf8_lossy(&o.stderr).trim()
            ));
        }
        let _ = std::fs::remove_file(&output);
    } else if opts.mode == "gs" {
        return Err(anyhow!("Ghostscript nao encontrado"));
    }
    let dpi = if opts.dpi == 0 { 110 } else { opts.dpi };
    let quality = if opts.quality == 0 { 60 } else { opts.quality };
    let out =
        tokio::task::spawn_blocking(move || rasterize(&input, &output, dpi, quality, &progress))
            .await??;
    Ok(CompressResult {
        output: out.output,
        before,
        after: out.bytes,
        method: "raster".into(),
        pages: out.pages,
    })
}

/// Dangerzone sem contêiner: cada página vira pixels e o PDF é remontado
/// só com imagens. Scripts, formulários, links e anexos não sobrevivem.
pub fn sanitize(
    input: &str,
    output_dir: &str,
    dpi: u32,
    quality: u8,
    progress: &super::ProgressFn,
) -> anyhow::Result<PdfOut> {
    let input = PathBuf::from(input.trim());
    let output =
        unique(out_dir_for(&input, output_dir).join(format!("{} (seguro).pdf", stem(&input))));
    let dpi = if dpi == 0 { 150 } else { dpi };
    let quality = if quality == 0 { 85 } else { quality };
    rasterize(&input, &output, dpi, quality, progress)
}

// ── OCR (PDF pesquisável) ──────────────────────────────────────────────

pub async fn ocr(
    input: String,
    langs: String,
    output_dir: String,
    dpi: u32,
    progress: super::ProgressFn,
) -> anyhow::Result<PdfOut> {
    let tesseract = super::ocr::locate()
        .await
        .ok_or_else(|| anyhow!("tesseract nao esta instalado"))?;
    let input_path = PathBuf::from(input.trim());
    let langs = if langs.trim().is_empty() {
        "eng".to_string()
    } else {
        langs.trim().to_string()
    };
    let dpi = if dpi == 0 { 300 } else { dpi };
    let work = super::temp_dir().join(format!("pdf-ocr-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&work)?;

    // 1) páginas → PNG
    let (pngs, pages) = {
        let work = work.clone();
        let input = input_path.clone();
        let progress = progress.clone();
        tokio::task::spawn_blocking(move || -> anyhow::Result<(Vec<PathBuf>, usize)> {
            let api = api()?;
            let _g = OPS.lock().unwrap_or_else(|p| p.into_inner());
            let doc = Document::open(api, &input, None)?;
            let total = doc.pages();
            let mut out = Vec::with_capacity(total);
            for i in 0..total {
                report(
                    &progress,
                    "render",
                    i as u64,
                    Some(total as u64),
                    Some(format!("pagina {}", i + 1)),
                );
                let img = doc.page(i)?.render(dpi)?;
                let p = work.join(format!("{:04}.png", i + 1));
                std::fs::write(&p, encode(&img, "png", 100)?)?;
                out.push(p);
            }
            Ok((out, total))
        })
        .await??
    };

    // 2) tesseract com lista de imagens → um PDF só, com camada de texto
    let list = work.join("pages.txt");
    std::fs::write(
        &list,
        pngs.iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join("\n"),
    )?;
    let base = work.join("ocr");
    report(&progress, "ocr", 0, Some(pages as u64), Some(langs.clone()));
    let o = crate::core::process::command(&tesseract)
        .arg(&list)
        .arg(&base)
        .args(["-l", &langs, "--dpi", &dpi.to_string(), "pdf"])
        .output()
        .await?;
    if !o.status.success() {
        let _ = std::fs::remove_dir_all(&work);
        return Err(anyhow!(
            "tesseract falhou: {}",
            String::from_utf8_lossy(&o.stderr).trim()
        ));
    }
    let produced = base.with_extension("pdf");
    let output = unique(
        out_dir_for(&input_path, &output_dir).join(format!("{} (OCR).pdf", stem(&input_path))),
    );
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if std::fs::rename(&produced, &output).is_err() {
        std::fs::copy(&produced, &output)?;
    }
    let bytes = std::fs::metadata(&output).map(|m| m.len()).unwrap_or(0);
    let _ = std::fs::remove_dir_all(&work);
    report(&progress, "done", pages as u64, Some(pages as u64), None);
    Ok(PdfOut {
        output: output.to_string_lossy().to_string(),
        pages,
        bytes,
    })
}

// ── LibreOffice (PDF ↔ Office) ─────────────────────────────────────────

pub async fn office_convert(
    inputs: Vec<String>,
    target: String,
    output_dir: String,
    progress: super::ProgressFn,
) -> anyhow::Result<Vec<String>> {
    let soffice = find_soffice()
        .await
        .ok_or_else(|| anyhow!("LibreOffice nao encontrado"))?;
    let target = target.trim().trim_start_matches('.').to_ascii_lowercase();
    if !matches!(
        target.as_str(),
        "docx" | "odt" | "pptx" | "xlsx" | "pdf" | "html" | "txt" | "epub"
    ) {
        return Err(anyhow!("formato nao suportado: {}", target));
    }
    let mut outputs = Vec::new();
    let n = inputs.len() as u64;
    for (i, input) in inputs.iter().enumerate() {
        report(
            &progress,
            "progress",
            i as u64,
            Some(n),
            Some(input.clone()),
        );
        let input_path = PathBuf::from(input.trim());
        let dir = out_dir_for(&input_path, &output_dir);
        std::fs::create_dir_all(&dir)?;
        let is_pdf = input_path
            .extension()
            .map(|e| e.eq_ignore_ascii_case("pdf"))
            .unwrap_or(false);
        let mut cmd = crate::core::process::command(&soffice);
        cmd.args(["--headless", "--norestore"]);
        if is_pdf && target != "pdf" {
            cmd.arg("--infilter=writer_pdf_import");
        }
        cmd.arg("--convert-to")
            .arg(&target)
            .arg("--outdir")
            .arg(&dir)
            .arg(&input_path);
        let o = cmd.output().await?;
        let produced = dir.join(format!("{}.{}", stem(&input_path), target));
        if !o.status.success() || !produced.exists() {
            return Err(anyhow!(
                "LibreOffice nao converteu {}: {}",
                input,
                String::from_utf8_lossy(if o.stderr.is_empty() {
                    &o.stdout
                } else {
                    &o.stderr
                })
                .trim()
            ));
        }
        outputs.push(produced.to_string_lossy().to_string());
    }
    report(&progress, "done", n, Some(n), None);
    Ok(outputs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        assert_eq!(parse_ranges("", 3).unwrap(), vec![1, 2, 3]);
        assert_eq!(parse_ranges("1-2, 5", 5).unwrap(), vec![1, 2, 5]);
        assert_eq!(parse_ranges("4-", 5).unwrap(), vec![4, 5]);
        assert_eq!(parse_ranges("3-1", 5).unwrap(), vec![3, 2, 1]);
        assert!(parse_ranges("9", 5).is_err());
        assert!(parse_ranges("0", 5).is_err());
    }

    #[test]
    fn unique_names() {
        let dir = std::env::temp_dir().join(format!("omniget-pdf-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.pdf");
        assert_eq!(unique(p.clone()), p);
        std::fs::write(&p, b"x").unwrap();
        assert_eq!(unique(p.clone()), dir.join("a (2).pdf"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Exercita o FFI de verdade: baixa o PDFium gerido se faltar, monta um
    /// PDF de 3 páginas com JPEGs, junta, divide, renderiza e extrai texto.
    /// `cargo test -p omniget-core --lib tools::pdf::tests::live -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn live() {
        if !available() {
            crate::core::pdfium::ensure_pdfium().await.expect("pdfium");
        }
        let dir = std::env::temp_dir().join(format!("omniget-pdf-live-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut jpegs = Vec::new();
        for i in 0..3u8 {
            let img =
                image::RgbImage::from_fn(200, 300, |x, _| image::Rgb([x as u8, 80 + i * 40, 200]));
            jpegs.push(encode(&img, "jpg", 80).unwrap());
        }
        let a = dir.join("a.pdf");
        std::fs::write(&a, jpeg_pdf::build_pdf(&jpegs).unwrap()).unwrap();
        let p = super::super::noop_progress();
        let i = info(&a.to_string_lossy(), None).unwrap();
        assert_eq!(i.pages, 3);
        assert!(!i.has_text);
        let merged = merge(
            &MergeOptions {
                inputs: vec![
                    a.to_string_lossy().to_string(),
                    a.to_string_lossy().to_string(),
                ],
                output: dir.join("m.pdf").to_string_lossy().to_string(),
            },
            &p,
        )
        .unwrap();
        assert_eq!(merged.pages, 6);
        let sp = split(
            &SplitOptions {
                input: merged.output.clone(),
                mode: "every".into(),
                every: 4,
                ranges: String::new(),
                output_dir: String::new(),
            },
            &p,
        )
        .unwrap();
        assert_eq!(sp.outputs.len(), 2);
        assert_eq!(info(&sp.outputs[1], None).unwrap().pages, 2);
        let imgs = render(
            &RenderOptions {
                input: merged.output.clone(),
                pages: "1,6".into(),
                dpi: 50,
                format: "png".into(),
                quality: 0,
                output_dir: String::new(),
            },
            &p,
        )
        .unwrap();
        assert_eq!(imgs.len(), 2);
        let png = image::open(&imgs[0]).unwrap().to_rgb8();
        assert!(
            png.width() > 100 && png.get_pixel(png.width() - 1, 10)[2] > 150,
            "render azul"
        );
        let t = to_text(&merged.output, "", false, "").unwrap();
        assert_eq!(t.pages, 6);
        let s = sanitize(&merged.output, "", 40, 50, &p).unwrap();
        assert_eq!(s.pages, 6);
        let c = compress(
            CompressOptions {
                input: merged.output.clone(),
                output_dir: String::new(),
                mode: "raster".into(),
                preset: String::new(),
                dpi: 30,
                quality: 30,
            },
            p.clone(),
        )
        .await
        .unwrap();
        assert!(c.after < c.before, "{} < {}", c.after, c.before);
        println!(
            "ok: {} {} {:?} {}",
            merged.output, sp.outputs[0], imgs, c.output
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
