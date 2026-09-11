//! Categoria Games: importar e organizar o que os consoles e os gravadores de
//! PC cospem em pastas sem nome, e responder "roda no Linux?" antes de comprar.
//!
//! Os três módulos compartilham o mesmo esqueleto: ler um nome de arquivo,
//! extrair jogo + data, decidir um destino legível e mover/copiar sem
//! reimportar o que já está lá. O que muda é o padrão do nome.

pub mod clip_organizer;
pub mod protondb;
pub mod steam;
pub mod switch_album;

use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// Nome do arquivo de índice que fica na raiz da biblioteca de destino. É o
/// que evita reler (e re-hashear) a biblioteca inteira a cada importação.
pub const INDEX_FILE: &str = ".omniget-import-index.txt";

/// Uma linha do relatório: o que aconteceu com um arquivo de origem.
#[derive(Debug, Clone, Serialize)]
pub struct ImportItem {
    pub source: String,
    pub dest: Option<String>,
    /// Nome do jogo, ou o próprio identificador quando não conhecemos.
    pub game: String,
    /// Identificador cru (hash do Switch, appid da Steam, vazio no resto).
    pub game_id: String,
    pub known_game: bool,
    /// "2026-09-09 21-14-03", ou vazio quando o nome não trouxe data.
    pub taken_at: String,
    pub bytes: u64,
    /// "image" | "clip"
    pub kind: String,
    pub duration_s: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Saída da conversão/recompressão opcional.
    pub extra: Option<String>,
    /// "imported" | "skipped" | "error"
    pub status: String,
    /// "duplicate" | "exists" | "unnamed" | mensagem de erro
    pub reason: Option<String>,
}

/// Quantos arquivos e quantos bytes por jogo, para o resumo da tela.
#[derive(Debug, Clone, Serialize)]
pub struct GameSummary {
    pub game: String,
    pub game_id: String,
    pub known_game: bool,
    pub clips: u64,
    pub images: u64,
    pub bytes: u64,
}

pub fn is_clip_ext(ext: &str) -> bool {
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "mp4" | "mkv" | "mov" | "webm" | "avi" | "m4v" | "flv" | "ts"
    )
}

pub fn is_image_ext(ext: &str) -> bool {
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "jpg" | "jpeg" | "png" | "bmp" | "webp" | "tif" | "tiff"
    )
}

/// SHA-256 do conteúdo, em blocos, para não carregar um clipe de 1 GB na RAM.
pub fn sha256_file(path: &Path) -> anyhow::Result<String> {
    use sha2::Digest;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = sha2::Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Um caminho livre dentro de `dir`. Se `nome.jpg` já existe, tenta
/// `nome (2).jpg` e assim por diante — nunca sobrescreve.
pub fn unique_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let first = dir.join(format!("{}.{}", stem, ext));
    if !first.exists() {
        return first;
    }
    for n in 2..10_000u32 {
        let candidate = dir.join(format!("{} ({}).{}", stem, n, ext));
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!("{} {}.{}", stem, uuid::Uuid::new_v4(), ext))
}

/// Copia ou move. `rename` só funciona dentro do mesmo volume — do cartão SD
/// para o HD ele falha, então o caminho de mover cai para copiar e apagar.
pub fn place_file(src: &Path, dest: &Path, move_it: bool) -> anyhow::Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if move_it {
        match std::fs::rename(src, dest) {
            Ok(()) => return Ok(()),
            Err(_) => {
                std::fs::copy(src, dest)?;
                std::fs::remove_file(src)?;
                return Ok(());
            }
        }
    }
    std::fs::copy(src, dest)?;
    Ok(())
}

/// Índice de conteúdo já importado. Guarda só os SHA-256, um por linha, na
/// raiz da biblioteca. Sem o arquivo, a primeira importação varre e hasheia o
/// que já está lá para não duplicar o que o usuário copiou na mão.
#[derive(Debug, Default)]
pub struct DedupeIndex {
    hashes: HashSet<String>,
    path: Option<PathBuf>,
    dirty: bool,
}

impl DedupeIndex {
    pub fn disabled() -> Self {
        Self::default()
    }

    pub fn load(dest: &Path) -> Self {
        let path = dest.join(INDEX_FILE);
        let mut hashes = HashSet::new();
        let mut found = false;
        if let Ok(text) = std::fs::read_to_string(&path) {
            found = true;
            for line in text.lines() {
                let h = line.trim();
                if h.len() == 64 {
                    hashes.insert(h.to_ascii_lowercase());
                }
            }
        }
        if !found && dest.exists() {
            for entry in walkdir::WalkDir::new(dest)
                .max_depth(6)
                .into_iter()
                .filter_map(Result::ok)
            {
                if !entry.file_type().is_file() {
                    continue;
                }
                let p = entry.path();
                let ext = p
                    .extension()
                    .map(|e| e.to_string_lossy().to_string())
                    .unwrap_or_default();
                if !is_image_ext(&ext) && !is_clip_ext(&ext) {
                    continue;
                }
                if let Ok(h) = sha256_file(p) {
                    hashes.insert(h);
                }
            }
        }
        Self {
            hashes,
            path: Some(path),
            dirty: !found,
        }
    }

    pub fn enabled(&self) -> bool {
        self.path.is_some()
    }

    pub fn contains(&self, hash: &str) -> bool {
        self.hashes.contains(hash)
    }

    pub fn insert(&mut self, hash: String) -> bool {
        let fresh = self.hashes.insert(hash);
        self.dirty |= fresh;
        fresh
    }

    pub fn len(&self) -> usize {
        self.hashes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.hashes.is_empty()
    }

    pub fn save(&mut self) -> anyhow::Result<()> {
        let Some(path) = self.path.clone() else {
            return Ok(());
        };
        if !self.dirty {
            return Ok(());
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut sorted: Vec<&String> = self.hashes.iter().collect();
        sorted.sort();
        let body: String = sorted
            .iter()
            .map(|h| format!("{}\n", h))
            .collect::<Vec<_>>()
            .concat();
        std::fs::write(&path, body)?;
        self.dirty = false;
        Ok(())
    }
}

/// Agrupa o relatório por jogo, do maior para o menor.
pub fn summarize(items: &[ImportItem]) -> Vec<GameSummary> {
    let mut map: std::collections::BTreeMap<String, GameSummary> =
        std::collections::BTreeMap::new();
    for it in items.iter().filter(|i| i.status != "error") {
        let entry = map.entry(it.game.clone()).or_insert_with(|| GameSummary {
            game: it.game.clone(),
            game_id: it.game_id.clone(),
            known_game: it.known_game,
            clips: 0,
            images: 0,
            bytes: 0,
        });
        if it.kind == "clip" {
            entry.clips += 1;
        } else {
            entry.images += 1;
        }
        entry.bytes += it.bytes;
    }
    let mut out: Vec<GameSummary> = map.into_values().collect();
    out.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.game.cmp(&b.game)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir(tag: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("omniget-games-{}-{}", tag, uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).expect("criar tempdir de teste");
        d
    }

    #[test]
    fn sha256_matches_the_reference_vector() {
        let dir = tempdir("sha");
        let f = dir.join("a.txt");
        std::fs::write(&f, b"abc").expect("escrever");
        assert_eq!(
            sha256_file(&f).expect("hash"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unique_path_never_overwrites() {
        let dir = tempdir("unique");
        let a = unique_path(&dir, "Zelda 2026-09-09 21-14-03", "jpg");
        assert!(a.ends_with("Zelda 2026-09-09 21-14-03.jpg"));
        std::fs::write(&a, b"x").expect("escrever");
        let b = unique_path(&dir, "Zelda 2026-09-09 21-14-03", "jpg");
        assert!(b.ends_with("Zelda 2026-09-09 21-14-03 (2).jpg"), "{:?}", b);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn move_creates_the_destination_folder_and_removes_the_source() {
        let dir = tempdir("place");
        let src = dir.join("origem.jpg");
        std::fs::write(&src, b"conteudo").expect("escrever");
        let dest = dir.join("lib").join("Jogo").join("foto.jpg");
        place_file(&src, &dest, true).expect("mover");
        assert!(dest.exists());
        assert!(!src.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn index_survives_a_round_trip() {
        let dir = tempdir("index");
        let mut idx = DedupeIndex::load(&dir);
        assert!(idx.is_empty());
        assert!(idx.insert("a".repeat(64)));
        assert!(!idx.insert("a".repeat(64)));
        idx.save().expect("salvar índice");
        let again = DedupeIndex::load(&dir);
        assert_eq!(again.len(), 1);
        assert!(again.contains(&"a".repeat(64)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn index_bootstraps_from_files_already_in_the_library() {
        let dir = tempdir("bootstrap");
        std::fs::create_dir_all(dir.join("Jogo")).expect("criar");
        std::fs::write(dir.join("Jogo").join("velha.jpg"), b"abc").expect("escrever");
        std::fs::write(dir.join("Jogo").join("nota.txt"), b"abc").expect("escrever");
        let idx = DedupeIndex::load(&dir);
        // Só a imagem entra; o .txt é ignorado.
        assert_eq!(idx.len(), 1);
        assert!(idx.contains("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn summary_groups_by_game_and_sorts_by_size() {
        let mk = |game: &str, kind: &str, bytes: u64| ImportItem {
            source: String::new(),
            dest: None,
            game: game.into(),
            game_id: String::new(),
            known_game: true,
            taken_at: String::new(),
            bytes,
            kind: kind.into(),
            duration_s: None,
            width: None,
            height: None,
            extra: None,
            status: "imported".into(),
            reason: None,
        };
        let items = vec![
            mk("Hades", "image", 10),
            mk("Zelda", "clip", 100),
            mk("Hades", "clip", 20),
            ImportItem {
                status: "error".into(),
                ..mk("Hades", "clip", 999)
            },
        ];
        let s = summarize(&items);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].game, "Zelda");
        assert_eq!(s[1].game, "Hades");
        assert_eq!(s[1].bytes, 30);
        assert_eq!(s[1].clips, 1);
        assert_eq!(s[1].images, 1);
    }
}
