use std::path::{Path, PathBuf};

use super::api::{ApiClient, BilibiliError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverFormat {
    Jpg,
    Png,
    Webp,
    Avif,
}

impl CoverFormat {
    pub fn extension(self) -> &'static str {
        match self {
            CoverFormat::Jpg => "jpg",
            CoverFormat::Png => "png",
            CoverFormat::Webp => "webp",
            CoverFormat::Avif => "avif",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "png" => CoverFormat::Png,
            "webp" => CoverFormat::Webp,
            "avif" => CoverFormat::Avif,
            _ => CoverFormat::Jpg,
        }
    }
}

pub fn typed_url(base_url: &str, format: CoverFormat) -> String {
    let stripped = base_url.split('@').next().unwrap_or(base_url);
    format!("{}@.{}", stripped, format.extension())
}

pub async fn download(
    client: &ApiClient,
    cover_url: &str,
    output_dir: &Path,
    filename: &str,
    format: CoverFormat,
) -> Result<PathBuf> {
    let typed = typed_url(cover_url, format);
    let bytes = client.get_bytes(&typed).await?;
    let path = output_dir.join(format!("{}.{}", filename, format.extension()));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(&path, bytes).map_err(|_| BilibiliError::ContentUnavailable)?;
    Ok(path)
}

pub async fn download_to(
    client: &ApiClient,
    cover_url: &str,
    path: &Path,
    format: CoverFormat,
) -> Result<PathBuf> {
    let typed = typed_url(cover_url, format);
    let bytes = client.get_bytes(&typed).await?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(path, bytes).map_err(|_| BilibiliError::ContentUnavailable)?;
    Ok(path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_url_strips_existing_at_suffix() {
        let url = "https://i0.hdslb.com/bfs/archive/abc.jpg@.webp";
        let out = typed_url(url, CoverFormat::Png);
        assert_eq!(out, "https://i0.hdslb.com/bfs/archive/abc.jpg@.png");
    }

    #[test]
    fn typed_url_appends_when_no_suffix() {
        let url = "https://i0.hdslb.com/bfs/archive/abc.jpg";
        let out = typed_url(url, CoverFormat::Webp);
        assert_eq!(out, "https://i0.hdslb.com/bfs/archive/abc.jpg@.webp");
    }

    #[test]
    fn cover_format_from_str_falls_back_to_jpg() {
        assert_eq!(CoverFormat::from_str("avif"), CoverFormat::Avif);
        assert_eq!(CoverFormat::from_str("PNG"), CoverFormat::Png);
        assert_eq!(CoverFormat::from_str("invalid"), CoverFormat::Jpg);
    }
}
