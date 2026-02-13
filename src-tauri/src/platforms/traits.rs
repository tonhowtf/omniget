use async_trait::async_trait;

use crate::models::media::{DownloadOptions, DownloadResult, MediaInfo};

#[async_trait]
pub trait PlatformDownloader: Send + Sync {
    fn name(&self) -> &str;
    fn can_handle(&self, url: &str) -> bool;
    async fn get_media_info(&self, url: &str) -> anyhow::Result<MediaInfo>;
    async fn download(
        &self,
        info: &MediaInfo,
        opts: &DownloadOptions,
        progress: tokio::sync::mpsc::Sender<f64>,
    ) -> anyhow::Result<DownloadResult>;
}
