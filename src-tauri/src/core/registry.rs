use crate::platforms::traits::PlatformDownloader;

pub struct PlatformRegistry {
    platforms: Vec<Box<dyn PlatformDownloader>>,
}

impl PlatformRegistry {
    pub fn new() -> Self {
        Self {
            platforms: Vec::new(),
        }
    }

    pub fn register(&mut self, platform: Box<dyn PlatformDownloader>) {
        self.platforms.push(platform);
    }

    pub fn find_platform(&self, url: &str) -> Option<&dyn PlatformDownloader> {
        self.platforms
            .iter()
            .find(|p| p.can_handle(url))
            .map(|p| p.as_ref())
    }
}

impl Default for PlatformRegistry {
    fn default() -> Self {
        Self::new()
    }
}
