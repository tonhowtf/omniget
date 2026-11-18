use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

pub trait TelegramHost: Send + Sync + 'static {
    fn emit(&self, event: &str, payload: serde_json::Value);
    fn data_dir(&self) -> PathBuf;
    fn settings(&self) -> serde_json::Value;
    fn save_settings(&self, settings: serde_json::Value);
    fn default_output_dir(&self) -> PathBuf;
}

static HOST: OnceLock<Arc<dyn TelegramHost>> = OnceLock::new();

pub fn install(host: Arc<dyn TelegramHost>) {
    let _ = HOST.set(host);
}

pub fn get() -> Option<Arc<dyn TelegramHost>> {
    HOST.get().cloned()
}

fn resolved_dir() -> Option<PathBuf> {
    get()
        .map(|h| h.data_dir())
        .or_else(|| crate::core::paths::app_data_dir().map(|d| d.join("telegram")))
}

fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn session_dir_in(dir: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    let dir = dir.ok_or_else(|| {
        anyhow::anyhow!("app data directory not found; cannot store the Telegram session")
    })?;
    create_private_dir(&dir)?;
    Ok(dir)
}

pub fn session_dir() -> anyhow::Result<PathBuf> {
    session_dir_in(resolved_dir())
}

pub fn data_dir() -> PathBuf {
    let dir = resolved_dir().unwrap_or_else(|| std::env::temp_dir().join("telegram"));
    let _ = create_private_dir(&dir);
    dir
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_dir_is_an_error_without_an_app_data_dir() {
        assert!(session_dir_in(None).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn session_dir_is_created_private_and_tightens_an_existing_one() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("telegram");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        let got = session_dir_in(Some(dir.clone())).unwrap();
        assert_eq!(got, dir);
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }
}
