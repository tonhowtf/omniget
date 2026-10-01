// WHY: people with yt-dlp or FFmpeg installed do not want a second copy (#222).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const OVERRIDES_FILE: &str = "binary-overrides.json";

pub const SUPPORTED: [&str; 2] = ["yt-dlp", "FFmpeg"];

pub fn is_supported(name: &str) -> bool {
    SUPPORTED.contains(&name)
}

fn file_path() -> Option<PathBuf> {
    super::paths::app_data_dir().map(|d| d.join(OVERRIDES_FILE))
}

pub fn load() -> BTreeMap<String, String> {
    let Some(path) = file_path() else {
        return BTreeMap::new();
    };
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return BTreeMap::new();
    };
    match serde_json::from_str(&raw) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("[overrides] {} ilegivel, ignorando: {}", path.display(), e);
            BTreeMap::new()
        }
    }
}

fn save(map: &BTreeMap<String, String>) -> std::io::Result<()> {
    let Some(path) = file_path() else {
        return Err(std::io::Error::other("sem diretorio de dados"));
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(map).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, &path)
}

pub fn get(name: &str) -> Option<PathBuf> {
    let raw = load().get(name)?.clone();
    let path = PathBuf::from(raw);
    if path.exists() {
        Some(path)
    } else {
        tracing::warn!(
            "[overrides] {} aponta para {} que nao existe mais — usando o gerenciado",
            name,
            path.display()
        );
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rejeicao {
    NaoSuportado,
    NaoExiste,
    NaoEArquivo,
}

impl Rejeicao {
    pub fn mensagem(&self, name: &str) -> String {
        match self {
            Rejeicao::NaoSuportado => {
                format!("{name} nao aceita um caminho customizado")
            }
            Rejeicao::NaoExiste => "O arquivo escolhido nao existe".to_string(),
            Rejeicao::NaoEArquivo => "O caminho escolhido e uma pasta, nao um arquivo".to_string(),
        }
    }
}

pub fn validar(name: &str, path: &Path) -> Result<(), Rejeicao> {
    if !is_supported(name) {
        return Err(Rejeicao::NaoSuportado);
    }
    if !path.exists() {
        return Err(Rejeicao::NaoExiste);
    }
    if !path.is_file() {
        return Err(Rejeicao::NaoEArquivo);
    }
    Ok(())
}

pub fn set(name: &str, path: &Path) -> Result<(), String> {
    validar(name, path).map_err(|r| r.mensagem(name))?;
    let mut map = load();
    map.insert(name.to_string(), path.to_string_lossy().to_string());
    save(&map).map_err(|e| e.to_string())
}

pub fn clear(name: &str) -> Result<(), String> {
    let mut map = load();
    if map.remove(name).is_none() {
        return Ok(());
    }
    save(&map).map_err(|e| e.to_string())
}
