//! PDF: juntar, dividir, comprimir, converter, OCR e "PDF seguro". Tudo em
//! cima do PDFium que o app já gerencia (Dependências → PDFium), carregado em
//! tempo de execução com `libloading`, então não entra nenhuma crate de PDF.
//! Estudo 3 (Stirling-PDF) deu o catálogo de operações; 35 (Dangerzone) deu a
//! remontagem a partir dos pixels; Ghostscript e LibreOffice são opcionais e
//! só entram quando já estão na máquina.

use std::ffi::{c_char, c_int, c_uint, c_ulong, c_void, CString};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

use super::jpeg_pdf;

type Doc = *mut c_void;
type Page = *mut c_void;
type Object = *mut c_void;
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
    text_charbox:
        unsafe extern "C" fn(TextPage, c_int, *mut f64, *mut f64, *mut f64, *mut f64) -> c_int,
    text_unicode: unsafe extern "C" fn(TextPage, c_int) -> c_uint,
    new_doc: unsafe extern "C" fn() -> Doc,
    import_pages: unsafe extern "C" fn(Doc, Doc, *const c_char, c_int) -> c_int,
    save_copy: unsafe extern "C" fn(Doc, *mut FileWrite, c_ulong) -> c_int,
    meta_text: unsafe extern "C" fn(Doc, *const c_char, *mut c_void, c_ulong) -> c_ulong,
    // Opcionais: builds antigos do PDFium podem não exportar. Quem precisa
    // (o PDF → Markdown) checa e devolve erro claro em vez de derrubar o
    // carregamento inteiro da biblioteca.
    text_fontsize: Option<unsafe extern "C" fn(TextPage, c_int) -> f64>,
    text_fontinfo:
        Option<unsafe extern "C" fn(TextPage, c_int, *mut c_void, c_ulong, *mut c_int) -> c_ulong>,
    text_fontweight: Option<unsafe extern "C" fn(TextPage, c_int) -> c_int>,
    /// Diz se o caractere foi inventado pelo PDFium (espaço de vão, quebra de
    /// linha) em vez de existir no fluxo do PDF. A tarja precisa disso para
    /// casar caractere com byte do content stream.
    text_generated: Option<unsafe extern "C" fn(TextPage, c_int) -> c_int>,
    obj_count: Option<unsafe extern "C" fn(Page) -> c_int>,
    obj_get: Option<unsafe extern "C" fn(Page, c_int) -> Object>,
    obj_type: Option<unsafe extern "C" fn(Object) -> c_int>,
    obj_bounds:
        Option<unsafe extern "C" fn(Object, *mut f32, *mut f32, *mut f32, *mut f32) -> c_int>,
    img_bitmap: Option<unsafe extern "C" fn(Object) -> Bitmap>,
    bmp_format: Option<unsafe extern "C" fn(Bitmap) -> c_int>,
    bmp_w: Option<unsafe extern "C" fn(Bitmap) -> c_int>,
    bmp_h: Option<unsafe extern "C" fn(Bitmap) -> c_int>,
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

/// Igual ao `sym`, mas o símbolo pode faltar: devolve `None` em vez de erro.
unsafe fn opt_sym<T: Copy>(lib: &libloading::Library, name: &[u8]) -> Option<T> {
    lib.get::<T>(name).ok().map(|s| *s)
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
            text_charbox: sym(&lib, b"FPDFText_GetCharBox\0")?,
            text_unicode: sym(&lib, b"FPDFText_GetUnicode\0")?,
            new_doc: sym(&lib, b"FPDF_CreateNewDocument\0")?,
            import_pages: sym(&lib, b"FPDF_ImportPages\0")?,
            save_copy: sym(&lib, b"FPDF_SaveAsCopy\0")?,
            meta_text: sym(&lib, b"FPDF_GetMetaText\0")?,
            text_fontsize: opt_sym(&lib, b"FPDFText_GetFontSize\0"),
            text_fontinfo: opt_sym(&lib, b"FPDFText_GetFontInfo\0"),
            text_fontweight: opt_sym(&lib, b"FPDFText_GetFontWeight\0"),
            text_generated: opt_sym(&lib, b"FPDFText_IsGenerated\0"),
            obj_count: opt_sym(&lib, b"FPDFPage_CountObjects\0"),
            obj_get: opt_sym(&lib, b"FPDFPage_GetObject\0"),
            obj_type: opt_sym(&lib, b"FPDFPageObj_GetType\0"),
            obj_bounds: opt_sym(&lib, b"FPDFPageObj_GetBounds\0"),
            img_bitmap: opt_sym(&lib, b"FPDFImageObj_GetBitmap\0"),
            bmp_format: opt_sym(&lib, b"FPDFBitmap_GetFormat\0"),
            bmp_w: opt_sym(&lib, b"FPDFBitmap_GetWidth\0"),
            bmp_h: opt_sym(&lib, b"FPDFBitmap_GetHeight\0"),
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

pub async fn find_gs() -> Option<PathBuf> {
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

/// Retângulo do que está desenhado em cada página, em pontos e com origem no
/// canto inferior esquerdo — a "caixa de tinta". `None` quer dizer página em
/// branco. Usado para cortar margem sem chutar valor.
pub fn ink_boxes(path: &str, dpi: u32, threshold: u8) -> anyhow::Result<Vec<Option<[f32; 4]>>> {
    let api = api()?;
    let _g = OPS.lock().unwrap_or_else(|p| p.into_inner());
    let doc = Document::open(api, Path::new(path), None)?;
    let dpi = dpi.clamp(36, 200);
    let scale = dpi as f32 / 72.0;
    let mut out = Vec::with_capacity(doc.pages());
    for i in 0..doc.pages() {
        let page = doc.page(i)?;
        let (_, h_pt) = page.size_pt();
        let img = page.render(dpi)?;
        let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
        for (x, y, px) in img.enumerate_pixels() {
            let p = px.0;
            // Qualquer canal escuro o bastante conta como tinta.
            if p[0] < threshold || p[1] < threshold || p[2] < threshold {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
        out.push(if x0 == u32::MAX {
            None
        } else {
            // Pixel → ponto, virando o eixo Y (a imagem cresce para baixo).
            Some([
                x0 as f32 / scale,
                h_pt - (y1 + 1) as f32 / scale,
                (x1 + 1) as f32 / scale,
                h_pt - y0 as f32 / scale,
            ])
        });
    }
    Ok(out)
}

// ── Tarja mal feita ────────────────────────────────────────────────────

/// Uma sequência de caracteres que continua no PDF embaixo de uma área de cor
/// chapada — o texto que a "tarja" só escondeu do olho.
#[derive(Debug, Clone, Serialize)]
pub struct HiddenRun {
    pub page: usize,
    pub text: String,
    /// Cor média por baixo, em hex, para a UI dizer "barra preta" ou "caixa branca".
    pub cover: String,
    /// Retângulo em pontos do PDF (x, y, largura, altura), origem em baixo.
    pub rect: (f32, f32, f32, f32),
}

#[derive(Debug, Clone, Serialize)]
pub struct RedactionReport {
    pub path: String,
    pub pages: usize,
    pub pages_checked: usize,
    pub chars_total: usize,
    pub chars_hidden: usize,
    pub runs: Vec<HiddenRun>,
}

/// Um caractere está escondido quando a área dele, na página renderizada, é de
/// cor chapada: o glifo existe no texto mas não aparece no pixel.
fn covered(img: &image::RgbImage, x0: f64, y0: f64, x1: f64, y1: f64) -> Option<[u8; 3]> {
    // Encolhe a caixa: a borda pega antialias do que está em volta.
    let (dx, dy) = ((x1 - x0) * 0.2, (y1 - y0) * 0.2);
    let (a, b) = ((x0 + dx).floor().max(0.0), (y0 + dy).floor().max(0.0));
    let (c, d) = (
        (x1 - dx).ceil().min(img.width() as f64 - 1.0),
        (y1 - dy).ceil().min(img.height() as f64 - 1.0),
    );
    if c - a < 1.0 || d - b < 1.0 {
        return None;
    }
    let (mut lo, mut hi) = (255i32, 0i32);
    let (mut sr, mut sg, mut sb, mut n) = (0u64, 0u64, 0u64, 0u64);
    for y in (b as u32)..=(d as u32) {
        for x in (a as u32)..=(c as u32) {
            let p = img.get_pixel(x, y).0;
            let luma = (p[0] as i32 * 299 + p[1] as i32 * 587 + p[2] as i32 * 114) / 1000;
            lo = lo.min(luma);
            hi = hi.max(luma);
            sr += p[0] as u64;
            sg += p[1] as u64;
            sb += p[2] as u64;
            n += 1;
        }
    }
    // Faixa de luminância quase nula = nada foi desenhado ali por cima do fundo.
    if n >= 4 && hi - lo <= 8 {
        return Some([(sr / n) as u8, (sg / n) as u8, (sb / n) as u8]);
    }
    None
}

pub fn redaction_check(path: &str, dpi: u32, pages_spec: &str) -> anyhow::Result<RedactionReport> {
    let api = api()?;
    let _g = OPS.lock().unwrap_or_else(|p| p.into_inner());
    let doc = Document::open(api, Path::new(path), None)?;
    let total = doc.pages();
    let wanted = parse_ranges(pages_spec, total)?;
    let dpi = dpi.clamp(48, 300);
    let scale = dpi as f64 / 72.0;

    let mut report = RedactionReport {
        path: path.to_string(),
        pages: total,
        pages_checked: wanted.len(),
        chars_total: 0,
        chars_hidden: 0,
        runs: Vec::new(),
    };

    for page_no in wanted {
        let page = doc.page(page_no - 1)?;
        let (_, h_pt) = page.size_pt();
        let img = page.render(dpi)?;
        unsafe {
            let tp = (api.text_load)(page.page);
            if tp.is_null() {
                continue;
            }
            let count = (api.text_count)(tp).max(0);
            report.chars_total += count as usize;
            let mut run = String::new();
            let mut cover: Option<[u8; 3]> = None;
            let mut bounds: Option<(f64, f64, f64, f64)> = None;
            for i in 0..count {
                let ch = char::from_u32((api.text_unicode)(tp, i)).unwrap_or('\0');
                let (mut l, mut r, mut b, mut t) = (0f64, 0f64, 0f64, 0f64);
                let got = (api.text_charbox)(tp, i, &mut l, &mut r, &mut b, &mut t) != 0;
                let hidden = if got && !ch.is_whitespace() && ch != '\0' {
                    covered(
                        &img,
                        l * scale,
                        (h_pt as f64 - t) * scale,
                        r * scale,
                        (h_pt as f64 - b) * scale,
                    )
                } else {
                    None
                };
                match hidden {
                    Some(c) => {
                        report.chars_hidden += 1;
                        cover = Some(c);
                        run.push(ch);
                        bounds = Some(match bounds {
                            Some((x0, y0, x1, y1)) => (x0.min(l), y0.min(b), x1.max(r), y1.max(t)),
                            None => (l, b, r, t),
                        });
                    }
                    None => {
                        // Espaço entre dois trechos escondidos continua o mesmo trecho.
                        if !run.is_empty() && ch.is_whitespace() {
                            run.push(' ');
                        } else if !run.trim().is_empty() {
                            let c = cover.unwrap_or([0, 0, 0]);
                            let (x0, y0, x1, y1) = bounds.unwrap_or((0.0, 0.0, 0.0, 0.0));
                            report.runs.push(HiddenRun {
                                page: page_no,
                                text: run.trim().to_string(),
                                cover: format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2]),
                                rect: (x0 as f32, y0 as f32, (x1 - x0) as f32, (y1 - y0) as f32),
                            });
                            run.clear();
                            bounds = None;
                        } else {
                            run.clear();
                            bounds = None;
                        }
                    }
                }
            }
            if !run.trim().is_empty() {
                let c = cover.unwrap_or([0, 0, 0]);
                let (x0, y0, x1, y1) = bounds.unwrap_or((0.0, 0.0, 0.0, 0.0));
                report.runs.push(HiddenRun {
                    page: page_no,
                    text: run.trim().to_string(),
                    cover: format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2]),
                    rect: (x0 as f32, y0 as f32, (x1 - x0) as f32, (y1 - y0) as f32),
                });
            }
            (api.text_close)(tp);
        }
    }
    Ok(report)
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

// ── Leitura posicionada (base do PDF → Markdown) ───────────────────────

/// Um caractere com a caixa dele em pontos do PDF (origem embaixo à
/// esquerda, `y` crescendo para cima), o corpo da fonte e duas marcas de
/// estilo que o Markdown usa: monoespaçada vira bloco de código, negrito
/// ajuda a achar título.
#[derive(Debug, Clone)]
pub struct TextChar {
    pub ch: char,
    pub x0: f32,
    pub x1: f32,
    pub y0: f32,
    pub y1: f32,
    pub size: f32,
    pub mono: bool,
    pub bold: bool,
    /// Veio um espaço (ou quebra de linha) antes deste caractere no fluxo do
    /// PDF. É o sinal mais confiável de separação de palavra: a caixa do
    /// glifo é justa demais para servir de régua sozinha.
    pub space_before: bool,
}

/// Imagem embutida na página, já em PNG, com a caixa em pontos.
#[derive(Debug, Clone)]
pub struct PageImage {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub png: Vec<u8>,
}

#[derive(Debug, Clone, Default)]
pub struct PageText {
    /// Número 1-based da página no documento original.
    pub number: usize,
    pub width: f32,
    pub height: f32,
    pub chars: Vec<TextChar>,
    pub images: Vec<PageImage>,
}

const FPDF_PAGEOBJ_IMAGE: c_int = 3;

/// Estilo da fonte do caractere: `(monoespaçada, negrito)`. Usa os flags do
/// descritor de fonte (bit 1 = passo fixo, bit 19 = negrito forçado) e cai no
/// nome da fonte quando os flags vêm zerados, que é comum.
unsafe fn font_traits(api: &Api, tp: TextPage, idx: c_int) -> (bool, bool) {
    let mut mono = false;
    let mut bold = false;
    if let Some(info) = api.text_fontinfo {
        let mut flags: c_int = 0;
        let mut buf = [0u8; 96];
        let n = info(
            tp,
            idx,
            buf.as_mut_ptr() as *mut c_void,
            buf.len() as c_ulong,
            &mut flags,
        ) as usize;
        let n = n.min(buf.len()).saturating_sub(1);
        let name = String::from_utf8_lossy(&buf[..n]).to_ascii_lowercase();
        mono = flags & 1 != 0
            || ["mono", "courier", "consol", "menlo", "monaco"]
                .iter()
                .any(|k| name.contains(k));
        bold = flags & (1 << 18) != 0
            || ["bold", "black", "heavy", "semib"]
                .iter()
                .any(|k| name.contains(k));
    }
    if let Some(weight) = api.text_fontweight {
        if weight(tp, idx) >= 600 {
            bold = true;
        }
    }
    (mono, bold)
}

/// Imagens desenhadas na página, convertidas para PNG. Ignora as minúsculas
/// (fio, marca d'água de 1 px) e devolve vazio se o PDFium instalado não
/// expõe a API de objetos.
fn page_images(api: &'static Api, page: &PageRef<'_>, min_pt: f32) -> Vec<PageImage> {
    let (count, get, kind, bounds, bitmap_of, fmt, bw, bh) = match (
        api.obj_count,
        api.obj_get,
        api.obj_type,
        api.obj_bounds,
        api.img_bitmap,
        api.bmp_format,
        api.bmp_w,
        api.bmp_h,
    ) {
        (Some(a), Some(b), Some(c), Some(d), Some(e), Some(f), Some(g), Some(h)) => {
            (a, b, c, d, e, f, g, h)
        }
        _ => return Vec::new(),
    };
    let mut out = Vec::new();
    unsafe {
        let n = count(page.page).max(0);
        for i in 0..n {
            let obj = get(page.page, i);
            if obj.is_null() || kind(obj) != FPDF_PAGEOBJ_IMAGE {
                continue;
            }
            let (mut l, mut b, mut r, mut t) = (0f32, 0f32, 0f32, 0f32);
            if bounds(obj, &mut l, &mut b, &mut r, &mut t) == 0 {
                continue;
            }
            if (r - l) < min_pt || (t - b) < min_pt {
                continue;
            }
            let bmp = bitmap_of(obj);
            if bmp.is_null() {
                continue;
            }
            let (w, h) = (bw(bmp).max(0), bh(bmp).max(0));
            let stride = (api.bmp_stride)(bmp).max(0) as usize;
            let buf = (api.bmp_buffer)(bmp) as *const u8;
            if w == 0 || h == 0 || stride == 0 || buf.is_null() {
                (api.bmp_destroy)(bmp);
                continue;
            }
            let channels = match fmt(bmp) {
                1 => 1usize, // cinza
                2 => 3,      // BGR
                3 | 4 => 4,  // BGRx / BGRA
                _ => {
                    (api.bmp_destroy)(bmp);
                    continue;
                }
            };
            let src = std::slice::from_raw_parts(buf, stride * h as usize);
            let mut img = image::RgbaImage::new(w as u32, h as u32);
            for y in 0..h as usize {
                let row = &src[y * stride..y * stride + (w as usize) * channels];
                for x in 0..w as usize {
                    let p = &row[x * channels..x * channels + channels];
                    let px = match channels {
                        1 => image::Rgba([p[0], p[0], p[0], 255]),
                        3 => image::Rgba([p[2], p[1], p[0], 255]),
                        _ => {
                            image::Rgba([p[2], p[1], p[0], if fmt(bmp) == 4 { p[3] } else { 255 }])
                        }
                    };
                    img.put_pixel(x as u32, y as u32, px);
                }
            }
            (api.bmp_destroy)(bmp);
            let mut png = Vec::new();
            if image::DynamicImage::ImageRgba8(img)
                .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
                .is_ok()
            {
                out.push(PageImage {
                    x0: l,
                    y0: b,
                    x1: r,
                    y1: t,
                    png,
                });
            }
        }
    }
    out
}

/// Lê as páginas pedidas com posição, corpo de fonte e estilo de cada
/// caractere — a matéria-prima do PDF → Markdown. `on_page` recebe
/// `(feitas, total)` antes de cada página, para o progresso.
pub fn read_pages(
    input: &str,
    password: Option<&str>,
    pages_spec: &str,
    want_images: bool,
    mut on_page: impl FnMut(usize, usize),
) -> anyhow::Result<Vec<PageText>> {
    let api = api()?;
    let font_size = api.text_fontsize.ok_or_else(|| {
        anyhow!("este PDFium nao expoe FPDFText_GetFontSize; atualize em Ajustes → Dependencias")
    })?;
    let _g = OPS.lock().unwrap_or_else(|p| p.into_inner());
    let path = Path::new(input.trim());
    let doc = Document::open(api, path, password)?;
    let wanted = parse_ranges(pages_spec, doc.pages())?;
    let total = wanted.len();
    let mut out = Vec::with_capacity(total);
    for (done, no) in wanted.iter().enumerate() {
        on_page(done, total);
        let page = doc.page(no - 1)?;
        let (width, height) = page.size_pt();
        let mut chars = Vec::new();
        unsafe {
            let tp = (api.text_load)(page.page);
            if !tp.is_null() {
                let count = (api.text_count)(tp).max(0);
                chars.reserve(count as usize);
                // Alguns glifos (hífen de fim de linha, por exemplo) voltam sem
                // caixa; em vez de sumir com o caractere, ele cola no fim do
                // anterior — é onde ele foi desenhado.
                let mut last: Option<(f32, f32, f32)> = None;
                let mut space = false;
                for idx in 0..count {
                    // O PDFium devolve 0x02 no lugar do hífen que quebra a
                    // palavra no fim da linha: é hífen mesmo, não controle.
                    let ch = match (api.text_unicode)(tp, idx) {
                        2 => '-',
                        u => match char::from_u32(u) {
                            Some(c) if c != '\0' && !c.is_control() && !c.is_whitespace() => c,
                            _ => {
                                space = true;
                                continue;
                            }
                        },
                    };
                    let (mut l, mut r, mut b, mut t) = (0f64, 0f64, 0f64, 0f64);
                    let boxed = (api.text_charbox)(tp, idx, &mut l, &mut r, &mut b, &mut t) != 0
                        && (r - l).abs() + (t - b).abs() > 0.0;
                    if !boxed {
                        match last {
                            Some((x, y0, y1)) => {
                                l = x as f64;
                                r = x as f64;
                                b = y0 as f64;
                                t = y1 as f64;
                            }
                            None => continue,
                        }
                    }
                    let size = font_size(tp, idx) as f32;
                    let size = if size.is_finite() && size.abs() > 0.01 {
                        size.abs()
                    } else {
                        (t - b) as f32
                    };
                    let (mono, bold) = font_traits(api, tp, idx);
                    last = Some((r as f32, b as f32, t as f32));
                    let space_before = std::mem::take(&mut space);
                    chars.push(TextChar {
                        ch,
                        x0: l as f32,
                        x1: r as f32,
                        y0: b as f32,
                        y1: t as f32,
                        size,
                        mono,
                        bold,
                        space_before,
                    });
                }
                (api.text_close)(tp);
            }
        }
        let images = if want_images {
            page_images(api, &page, 24.0)
        } else {
            Vec::new()
        };
        out.push(PageText {
            number: *no,
            width,
            height,
            chars,
            images,
        });
    }
    on_page(total, total);
    Ok(out)
}

// ── Leitura crua de caracteres (base da tarja) ─────────────────────────

/// Caractere como o PDFium o vê, sem filtro nenhum: espaço em branco entra,
/// caractere inventado entra marcado. A tarja precisa da lista inteira e na
/// ordem original para casar cada glifo com o byte dele no content stream.
#[derive(Debug, Clone)]
pub struct RawChar {
    pub ch: char,
    pub x0: f32,
    pub x1: f32,
    pub y0: f32,
    pub y1: f32,
    /// `true` quando o PDFium inventou o caractere (espaço de vão, quebra de
    /// linha): ele não tem byte correspondente no fluxo do PDF.
    pub generated: bool,
}

impl RawChar {
    /// Centro da caixa, que é o ponto usado para decidir se o caractere cai
    /// dentro da região a tarjar.
    pub fn center(&self) -> (f32, f32) {
        ((self.x0 + self.x1) / 2.0, (self.y0 + self.y1) / 2.0)
    }

    pub fn empty_box(&self) -> bool {
        (self.x1 - self.x0).abs() < 0.01 && (self.y1 - self.y0).abs() < 0.01
    }
}

#[derive(Debug, Clone, Default)]
pub struct RawPage {
    /// Número 1-based da página no documento.
    pub number: usize,
    pub width: f32,
    pub height: f32,
    pub chars: Vec<RawChar>,
    /// `false` quando o PDFium instalado não expõe `FPDFText_IsGenerated` —
    /// aí não dá para confiar no casamento glifo↔byte.
    pub generated_known: bool,
}

/// Lê todos os caracteres das páginas pedidas, na ordem do documento.
pub fn read_raw_chars(
    input: &str,
    password: Option<&str>,
    pages_spec: &str,
) -> anyhow::Result<Vec<RawPage>> {
    let api = api()?;
    let _g = OPS.lock().unwrap_or_else(|p| p.into_inner());
    let doc = Document::open(api, Path::new(input.trim()), password)?;
    let wanted = parse_ranges(pages_spec, doc.pages())?;
    let mut out = Vec::with_capacity(wanted.len());
    for no in wanted {
        let page = doc.page(no - 1)?;
        let (width, height) = page.size_pt();
        let mut chars = Vec::new();
        unsafe {
            let tp = (api.text_load)(page.page);
            if !tp.is_null() {
                let count = (api.text_count)(tp).max(0);
                chars.reserve(count as usize);
                for idx in 0..count {
                    let ch = match (api.text_unicode)(tp, idx) {
                        // O PDFium devolve 0x02 no lugar do hífen de quebra.
                        2 => '-',
                        u => char::from_u32(u).unwrap_or('\u{fffd}'),
                    };
                    let (mut l, mut r, mut b, mut t) = (0f64, 0f64, 0f64, 0f64);
                    if (api.text_charbox)(tp, idx, &mut l, &mut r, &mut b, &mut t) == 0 {
                        l = 0.0;
                        r = 0.0;
                        b = 0.0;
                        t = 0.0;
                    }
                    let generated = match api.text_generated {
                        Some(f) => f(tp, idx) != 0,
                        None => false,
                    };
                    chars.push(RawChar {
                        ch,
                        x0: l as f32,
                        x1: r as f32,
                        y0: b as f32,
                        y1: t as f32,
                        generated,
                    });
                }
                (api.text_close)(tp);
            }
        }
        out.push(RawPage {
            number: no,
            width,
            height,
            chars,
            generated_known: api.text_generated.is_some(),
        });
    }
    Ok(out)
}

/// Renderiza uma página avulsa em JPEG, para quem precisa trocar o conteúdo
/// da página por pixel (a tarja usa isso quando não dá para editar o texto).
pub fn page_jpeg(
    input: &str,
    password: Option<&str>,
    page: usize,
    dpi: u32,
    quality: u8,
) -> anyhow::Result<(Vec<u8>, f32, f32)> {
    let api = api()?;
    let _g = OPS.lock().unwrap_or_else(|p| p.into_inner());
    let doc = Document::open(api, Path::new(input.trim()), password)?;
    if page == 0 || page > doc.pages() {
        return Err(anyhow!("pagina {} fora do documento", page));
    }
    let p = doc.page(page - 1)?;
    let (w, h) = p.size_pt();
    let img = p.render(dpi.clamp(48, 400))?;
    Ok((encode(&img, "jpg", quality)?, w, h))
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

    /// PDF montado à mão: uma linha visível e outra coberta por um retângulo
    /// preto desenhado por cima — a "tarja" que não apaga nada.
    /// `cargo test -p omniget-core --lib -- --ignored live_redaction`
    #[test]
    #[ignore]
    fn live_redaction_check_finds_text_under_the_bar() {
        let content = b"BT /F1 24 Tf 40 150 Td (VISIVEL) Tj ET\nBT /F1 24 Tf 40 100 Td (SEGREDO) Tj ET\n0 0 0 rg 34 94 140 34 re f\n";
        let mut body = String::from("%PDF-1.4\n");
        body.push_str("1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        body.push_str("2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
        body.push_str(
            "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>\nendobj\n",
        );
        body.push_str("4 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n");
        body.push_str(&format!(
            "5 0 obj\n<< /Length {} >>\nstream\n{}endstream\nendobj\n",
            content.len(),
            String::from_utf8_lossy(content)
        ));
        // A xref sai da própria tool de reparo — de quebra, valida ela também.
        let (bytes, _) = super::super::pdf_repair::rebuild_xref(body.as_bytes()).unwrap();

        let dir = std::env::temp_dir().join("omniget-redaction-live");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tarja.pdf");
        std::fs::write(&path, &bytes).unwrap();
        let p = path.to_string_lossy().to_string();

        let info = info(&p, None).expect("o PDF montado tem que abrir");
        assert_eq!(info.pages, 1);

        let report = redaction_check(&p, 110, "").unwrap();
        assert!(
            report.chars_total >= 14,
            "leu {} caracteres",
            report.chars_total
        );
        let found: Vec<&str> = report.runs.iter().map(|r| r.text.as_str()).collect();
        assert!(
            found.iter().any(|t| t.contains("SEGREDO")),
            "não achou o texto sob a tarja: {:?}",
            found
        );
        assert!(
            !found.iter().any(|t| t.contains("VISIVEL")),
            "acusou texto que está à vista: {:?}",
            found
        );
        let bar = report
            .runs
            .iter()
            .find(|r| r.text.contains("SEGREDO"))
            .unwrap();
        assert_eq!(bar.cover, "#000000", "a cobertura era preta");
        assert_eq!(bar.page, 1);
        eprintln!("{} caracteres escondidos: {:?}", report.chars_hidden, found);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
