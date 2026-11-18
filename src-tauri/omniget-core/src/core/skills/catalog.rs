#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct CatalogEntry {
    pub name: &'static str,
    pub description: &'static str,
    pub subdir: &'static str,
    pub skill_md_sha256: &'static str,
    pub files: u32,
    pub bytes: u64,
}

pub fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}
