use async_trait::async_trait;

use crate::platforms::traits::PlatformDownloader;
use omniget_core::models::media::{DownloadOptions, DownloadResult, MediaInfo};

pub struct NoopDownloader;

impl NoopDownloader {
    pub fn new() -> Self {
        Self
    }
}

impl Default for NoopDownloader {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PlatformDownloader for NoopDownloader {
    fn name(&self) -> &str {
        "external"
    }

    fn can_handle(&self, _url: &str) -> bool {
        false
    }

    async fn get_media_info(&self, _url: &str) -> anyhow::Result<MediaInfo> {
        Err(anyhow::anyhow!(
            "NoopDownloader cannot fetch media info; it only stands in for items restored from history"
        ))
    }

    async fn download(
        &self,
        _info: &MediaInfo,
        _opts: &DownloadOptions,
        _progress: tokio::sync::mpsc::Sender<omniget_core::models::progress::ProgressUpdate>,
    ) -> anyhow::Result<DownloadResult> {
        Err(anyhow::anyhow!(
            "NoopDownloader cannot drive download; it only stands in for items restored from history"
        ))
    }
}
