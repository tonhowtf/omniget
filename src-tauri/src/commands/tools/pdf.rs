//! PDF (juntar, dividir, comprimir, converter, OCR, sanitizar). A lógica mora
//! em `omniget_core::core::tools::pdf`; aqui só se despacha para uma thread
//! de bloqueio porque o PDFium é síncrono e não é thread-safe.

use omniget_core::core::tools::{pdf, pdf_markdown, pdf_repair, pdf_write};

use super::{err, progress};

#[tauri::command]
pub async fn tool_pdf_status() -> pdf::PdfStatus {
    pdf::status().await
}

#[tauri::command]
pub async fn tool_pdf_info(path: String, password: Option<String>) -> Result<pdf::PdfInfo, String> {
    tokio::task::spawn_blocking(move || pdf::info(&path, password.as_deref()))
        .await
        .map_err(err)?
        .map_err(err)
}

#[tauri::command]
pub async fn tool_pdf_merge(
    app: tauri::AppHandle,
    opts: pdf::MergeOptions,
) -> Result<pdf::PdfOut, String> {
    let p = progress(&app);
    tokio::task::spawn_blocking(move || pdf::merge(&opts, &p))
        .await
        .map_err(err)?
        .map_err(err)
}

#[tauri::command]
pub async fn tool_pdf_split(
    app: tauri::AppHandle,
    opts: pdf::SplitOptions,
) -> Result<pdf::PdfOuts, String> {
    let p = progress(&app);
    tokio::task::spawn_blocking(move || pdf::split(&opts, &p))
        .await
        .map_err(err)?
        .map_err(err)
}

#[tauri::command]
pub async fn tool_pdf_render(
    app: tauri::AppHandle,
    opts: pdf::RenderOptions,
) -> Result<Vec<String>, String> {
    let p = progress(&app);
    tokio::task::spawn_blocking(move || pdf::render(&opts, &p))
        .await
        .map_err(err)?
        .map_err(err)
}

#[tauri::command]
pub async fn tool_pdf_text(
    input: String,
    pages: Option<String>,
    save: Option<bool>,
    output_dir: Option<String>,
) -> Result<pdf::TextResult, String> {
    tokio::task::spawn_blocking(move || {
        pdf::to_text(
            &input,
            pages.as_deref().unwrap_or(""),
            save.unwrap_or(false),
            output_dir.as_deref().unwrap_or(""),
        )
    })
    .await
    .map_err(err)?
    .map_err(err)
}

#[tauri::command]
pub async fn tool_pdf_from_images(
    app: tauri::AppHandle,
    inputs: Vec<String>,
    output: String,
    quality: Option<u8>,
) -> Result<pdf::PdfOut, String> {
    let p = progress(&app);
    tokio::task::spawn_blocking(move || {
        pdf::images_to_pdf(&inputs, &output, quality.unwrap_or(0), &p)
    })
    .await
    .map_err(err)?
    .map_err(err)
}

#[tauri::command]
pub async fn tool_pdf_compress(
    app: tauri::AppHandle,
    opts: pdf::CompressOptions,
) -> Result<pdf::CompressResult, String> {
    pdf::compress(opts, progress(&app)).await.map_err(err)
}

#[tauri::command]
pub async fn tool_pdf_sanitize(
    app: tauri::AppHandle,
    input: String,
    output_dir: Option<String>,
    dpi: Option<u32>,
    quality: Option<u8>,
) -> Result<pdf::PdfOut, String> {
    let p = progress(&app);
    tokio::task::spawn_blocking(move || {
        pdf::sanitize(
            &input,
            output_dir.as_deref().unwrap_or(""),
            dpi.unwrap_or(0),
            quality.unwrap_or(0),
            &p,
        )
    })
    .await
    .map_err(err)?
    .map_err(err)
}

#[tauri::command]
pub async fn tool_pdf_ocr(
    app: tauri::AppHandle,
    input: String,
    langs: String,
    output_dir: Option<String>,
    dpi: Option<u32>,
) -> Result<pdf::PdfOut, String> {
    pdf::ocr(
        input,
        langs,
        output_dir.unwrap_or_default(),
        dpi.unwrap_or(0),
        progress(&app),
    )
    .await
    .map_err(err)
}

#[tauri::command]
pub async fn tool_pdf_office(
    app: tauri::AppHandle,
    inputs: Vec<String>,
    target: String,
    output_dir: Option<String>,
) -> Result<Vec<String>, String> {
    pdf::office_convert(
        inputs,
        target,
        output_dir.unwrap_or_default(),
        progress(&app),
    )
    .await
    .map_err(err)
}

#[tauri::command]
pub async fn tool_pdf_repair(
    app: tauri::AppHandle,
    opts: pdf_repair::RepairOptions,
) -> Result<pdf_repair::RepairResult, String> {
    pdf_repair::run(opts, progress(&app)).await.map_err(err)
}

#[tauri::command]
pub async fn tool_pdf_redaction_check(
    input: String,
    dpi: Option<u32>,
    pages: Option<String>,
) -> Result<pdf::RedactionReport, String> {
    tokio::task::spawn_blocking(move || {
        pdf::redaction_check(&input, dpi.unwrap_or(110), pages.as_deref().unwrap_or(""))
    })
    .await
    .map_err(err)?
    .map_err(err)
}

// ── Escrita de estrutura (lopdf): senha, marca, corte, sumário ─────────

#[tauri::command]
pub async fn tool_pdf_password(
    app: tauri::AppHandle,
    opts: pdf_write::PasswordOptions,
) -> Result<pdf_write::WriteResult, String> {
    let p = progress(&app);
    tokio::task::spawn_blocking(move || pdf_write::password(&opts, &p))
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn tool_pdf_watermark(
    app: tauri::AppHandle,
    opts: pdf_write::WatermarkOptions,
) -> Result<pdf_write::WriteResult, String> {
    let p = progress(&app);
    tokio::task::spawn_blocking(move || pdf_write::watermark(&opts, &p))
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn tool_pdf_crop(
    app: tauri::AppHandle,
    opts: pdf_write::CropOptions,
) -> Result<pdf_write::WriteResult, String> {
    let p = progress(&app);
    tokio::task::spawn_blocking(move || pdf_write::crop(&opts, &p))
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn tool_pdf_outline_read(input: String) -> Result<Vec<pdf_write::OutlineEntry>, String> {
    tokio::task::spawn_blocking(move || pdf_write::read_outline(&input))
        .await
        .map_err(err)?
        .map_err(err)
}

#[tauri::command]
pub async fn tool_pdf_outline_write(
    opts: pdf_write::OutlineOptions,
) -> Result<pdf_write::WriteItem, String> {
    tokio::task::spawn_blocking(move || pdf_write::write_outline(&opts))
        .await
        .map_err(err)?
        .map_err(err)
}

// ── PDF → Markdown ─────────────────────────────────────────────────────

#[tauri::command]
pub async fn tool_pdf_markdown(
    app: tauri::AppHandle,
    opts: pdf_markdown::Options,
) -> Result<pdf_markdown::MdResult, String> {
    let p = progress(&app);
    tokio::task::spawn_blocking(move || pdf_markdown::run(&opts, &p))
        .await
        .map_err(err)?
        .map_err(err)
}

// ── Tarja, tabela e formulário ─────────────────────────────────────────

use omniget_core::core::tools::{pdf_form, pdf_redact, pdf_table};

#[tauri::command]
pub async fn tool_pdf_redact(
    app: tauri::AppHandle,
    opts: pdf_redact::Options,
) -> Result<pdf_redact::RedactResult, String> {
    let p = progress(&app);
    tokio::task::spawn_blocking(move || pdf_redact::run(&opts, &p))
        .await
        .map_err(err)?
        .map_err(err)
}

#[tauri::command]
pub async fn tool_pdf_tables(
    app: tauri::AppHandle,
    opts: pdf_table::Options,
) -> Result<pdf_table::TableResult, String> {
    let p = progress(&app);
    tokio::task::spawn_blocking(move || pdf_table::run(&opts, &p))
        .await
        .map_err(err)?
        .map_err(err)
}

#[tauri::command]
pub async fn tool_pdf_form_fields(
    input: String,
    password: Option<String>,
) -> Result<Vec<pdf_form::Field>, String> {
    tokio::task::spawn_blocking(move || {
        pdf_form::read_fields(&input, password.as_deref().unwrap_or(""))
    })
    .await
    .map_err(err)?
    .map_err(err)
}

#[tauri::command]
pub async fn tool_pdf_form_fill(
    app: tauri::AppHandle,
    opts: pdf_form::Options,
) -> Result<pdf_form::FillResult, String> {
    let p = progress(&app);
    tokio::task::spawn_blocking(move || pdf_form::fill(&opts, &p))
        .await
        .map_err(err)?
        .map_err(err)
}
