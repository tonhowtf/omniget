use std::path::PathBuf;

pub trait AppPaths: Send + Sync {
    fn downloads_dir(&self) -> PathBuf;
    fn data_dir(&self) -> PathBuf;
    fn cache_dir(&self) -> PathBuf;
    fn bin_dir(&self) -> Option<PathBuf>;
}

pub struct DesktopPaths;

impl AppPaths for DesktopPaths {
    fn downloads_dir(&self) -> PathBuf {
        dirs::download_dir().unwrap_or_else(|| PathBuf::from("."))
    }

    fn data_dir(&self) -> PathBuf {
        dirs::data_dir()
            .map(|d| d.join("omniget"))
            .unwrap_or_else(|| PathBuf::from("."))
    }

    fn cache_dir(&self) -> PathBuf {
        dirs::cache_dir()
            .map(|d| d.join("omniget"))
            .unwrap_or_else(|| PathBuf::from("."))
    }

    fn bin_dir(&self) -> Option<PathBuf> {
        dirs::data_dir().map(|d| d.join("omniget").join("bin"))
    }
}
