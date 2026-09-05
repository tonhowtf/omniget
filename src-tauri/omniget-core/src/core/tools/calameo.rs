//! Calameo (estudos 48, 49): cada página é um `p{n}.svgz` (ou `.jpg`) num
//! CDN previsível a partir do `og:image`. Salva as páginas descomprimidas
//! numa pasta; a montagem em PDF fica para quando houver um renderizador SVG.

use std::io::Read;
use std::path::PathBuf;

use anyhow::anyhow;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct CalameoResult {
    pub title: String,
    pub pages: usize,
    pub folder: String,
    pub format: String,
}

pub fn page_prefix(html: &str) -> Option<String> {
    let re = regex::Regex::new(r#"<meta[^>]*property="og:image"[^>]*content="([^"]+)""#).ok()?;
    let img = re.captures(html)?[1].to_string();
    let re2 = regex::Regex::new(r"^(.*?/)p?1\.(svgz|jpg|jpeg|png)").ok()?;
    re2.captures(&img).map(|c| c[1].to_string())
}

pub async fn download(
    url: &str,
    dest_dir: &str,
    progress: super::ProgressFn,
) -> anyhow::Result<CalameoResult> {
    if !url.contains("calameo.com") {
        return Err(anyhow!("cole um link do calameo.com"));
    }
    let client = super::client()?;
    let html = client
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let prefix = page_prefix(&html)
        .ok_or_else(|| anyhow!("nao achei as paginas desse documento (privado ou layout novo)"))?;
    let title = super::slides::og_title(&html).unwrap_or_else(|| "calameo".to_string());
    let folder = PathBuf::from(dest_dir).join(super::sanitize_name(&title));
    std::fs::create_dir_all(&folder)?;
    let id = format!("calameo:{}", url);
    let mut n = 0usize;
    let mut format = "svg".to_string();
    for page in 1..=2000usize {
        super::report(&progress, &id, "download", page as u64, None, None);
        let svg_url = format!("{}p{}.svgz", prefix, page);
        let resp = client.get(&svg_url).send().await?;
        if resp.status().is_success() {
            let bytes = resp.bytes().await?;
            let mut dec = flate2::read::GzDecoder::new(&bytes[..]);
            let mut svg = Vec::new();
            if dec.read_to_end(&mut svg).is_err() {
                svg = bytes.to_vec();
            }
            tokio::fs::write(folder.join(format!("p{:04}.svg", page)), &svg).await?;
            n = page;
            continue;
        }
        let jpg_url = format!("{}p{}.jpg", prefix, page);
        let resp = client.get(&jpg_url).send().await?;
        if resp.status().is_success() {
            format = "jpg".to_string();
            let bytes = resp.bytes().await?;
            tokio::fs::write(folder.join(format!("p{:04}.jpg", page)), &bytes).await?;
            n = page;
            continue;
        }
        break;
    }
    if n == 0 {
        return Err(anyhow!(
            "nenhuma pagina baixada; o documento pode exigir assinatura de URL"
        ));
    }
    if format == "jpg" {
        // com JPEG dá para montar o PDF na hora
        let mut imgs = Vec::new();
        for page in 1..=n {
            imgs.push(tokio::fs::read(folder.join(format!("p{:04}.jpg", page))).await?);
        }
        let pdf = super::jpeg_pdf::build_pdf(&imgs)?;
        tokio::fs::write(folder.with_extension("pdf"), pdf).await?;
    }
    super::report(&progress, &id, "done", n as u64, Some(n as u64), None);
    Ok(CalameoResult {
        title,
        pages: n,
        folder: folder.to_string_lossy().to_string(),
        format,
    })
}

#[cfg(test)]
mod tests {
    use super::page_prefix;

    #[test]
    fn prefix_from_og_image() {
        let html =
            r#"<meta property="og:image" content="https://i.calameoassets.com/240101/abc/p1.jpg">"#;
        assert_eq!(
            page_prefix(html).as_deref(),
            Some("https://i.calameoassets.com/240101/abc/")
        );
    }
}
