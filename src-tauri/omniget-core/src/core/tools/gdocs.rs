//! Google Docs/Slides/Sheets públicos (estudo 51): o export oficial
//! `…/d/{id}/export?format=` dispensa API e OAuth para links compartilhados.

use std::path::PathBuf;

use anyhow::anyhow;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct GdocInfo {
    pub kind: String,
    pub id: String,
    pub formats: Vec<String>,
}

pub fn parse(url: &str) -> Option<GdocInfo> {
    let re = regex::Regex::new(r"docs\.google\.com/(document|presentation|spreadsheets)/d/([A-Za-z0-9_-]+)").ok()?;
    let c = re.captures(url)?;
    let kind = c[1].to_string();
    let formats = match kind.as_str() {
        "document" => vec!["pdf", "docx", "odt", "txt", "epub", "html"],
        "presentation" => vec!["pdf", "pptx", "odp", "txt"],
        _ => vec!["pdf", "xlsx", "ods", "csv"],
    };
    Some(GdocInfo { kind, id: c[2].to_string(), formats: formats.into_iter().map(String::from).collect() })
}

pub fn export_url(info: &GdocInfo, format: &str) -> String {
    format!("https://docs.google.com/{}/d/{}/export?format={}", info.kind, info.id, format)
}

fn filename_from_disposition(v: &str) -> Option<String> {
    // filename*=UTF-8''nome.pdf  ou  filename="nome.pdf"
    if let Some(i) = v.find("filename*=UTF-8''") {
        let s = &v[i + 17..];
        let end = s.find(';').unwrap_or(s.len());
        return urlencoding::decode(&s[..end]).ok().map(|c| c.to_string());
    }
    let i = v.find("filename=")?;
    let s = v[i + 9..].trim().trim_matches('"');
    let end = s.find(';').unwrap_or(s.len());
    Some(s[..end].trim_matches('"').to_string())
}

pub async fn download(url: &str, format: &str, dest_dir: &str, progress: super::ProgressFn) -> anyhow::Result<String> {
    let info = parse(url).ok_or_else(|| anyhow!("cole um link de docs.google.com (Documentos, Apresentações ou Planilhas)"))?;
    if !info.formats.iter().any(|f| f == format) {
        return Err(anyhow!("formato {} nao disponivel para {}", format, info.kind));
    }
    let client = super::client()?;
    let export = export_url(&info, format);
    let resp = client.get(&export).send().await?;
    if resp.status().as_u16() == 401 || resp.status().as_u16() == 403 {
        return Err(anyhow!("o arquivo nao e publico; abra no navegador e use Arquivo > Fazer download"));
    }
    if !resp.status().is_success() {
        return Err(anyhow!("Google Docs: HTTP {}", resp.status()));
    }
    let name = resp
        .headers()
        .get(reqwest::header::CONTENT_DISPOSITION)
        .and_then(|v| v.to_str().ok())
        .and_then(filename_from_disposition)
        .unwrap_or_else(|| format!("{}.{}", info.id, format));
    let dir = PathBuf::from(dest_dir);
    std::fs::create_dir_all(&dir)?;
    let dest = dir.join(super::sanitize_name(&name));
    let bytes = resp.bytes().await?;
    tokio::fs::write(&dest, &bytes).await?;
    super::report(&progress, "gdocs", "done", bytes.len() as u64, Some(bytes.len() as u64), None);
    Ok(dest.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_kinds() {
        let i = parse("https://docs.google.com/presentation/d/1AbC_d-e/edit#slide=1").unwrap();
        assert_eq!(i.kind, "presentation");
        assert_eq!(i.id, "1AbC_d-e");
        assert!(parse("https://drive.google.com/x").is_none());
        assert_eq!(export_url(&i, "pdf"), "https://docs.google.com/presentation/d/1AbC_d-e/export?format=pdf");
    }

    #[test]
    fn disposition() {
        assert_eq!(filename_from_disposition("attachment; filename=\"a b.pdf\"").as_deref(), Some("a b.pdf"));
        assert_eq!(filename_from_disposition("attachment; filename*=UTF-8''caf%C3%A9.pdf").as_deref(), Some("café.pdf"));
    }
}
