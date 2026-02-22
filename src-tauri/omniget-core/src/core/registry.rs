use std::sync::Arc;

use crate::platforms::traits::PlatformDownloader;

pub struct PlatformRegistry {
    platforms: Vec<Arc<dyn PlatformDownloader>>,
}

impl PlatformRegistry {
    pub fn new() -> Self {
        Self {
            platforms: Vec::new(),
        }
    }

    pub fn register(&mut self, platform: Arc<dyn PlatformDownloader>) {
        self.platforms.push(platform);
    }

    pub fn find_platform(&self, url: &str) -> Option<Arc<dyn PlatformDownloader>> {
        self.platforms
            .iter()
            .find(|p| p.can_handle(url))
            .cloned()
    }
}

impl Default for PlatformRegistry {
    fn default() -> Self {
        Self::new()
    }
}
