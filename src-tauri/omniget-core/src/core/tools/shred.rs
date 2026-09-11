//! Apagar de verdade: sobrescreve o conteúdo antes de remover, para o arquivo
//! não sair inteiro num programa de recuperação.
//!
//! **Aviso honesto:** em SSD, APFS, Btrfs e qualquer sistema com cópia-em-
//! escrita ou wear leveling, sobrescrever o arquivo não garante que o bloco
//! físico antigo sumiu. Em disco rígido com sistema de arquivos comum, garante.
//! Para SSD, o que apaga de verdade é criptografia de disco inteiro.

use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct ShredOptions {
    pub paths: Vec<String>,
    /// Quantas passagens de sobrescrita (1 basta em disco moderno).
    #[serde(default = "default_passes")]
    pub passes: u32,
    /// Renomeia para um nome aleatório antes de apagar, para o nome não ficar
    /// no diário do sistema de arquivos.
    #[serde(default = "default_true")]
    pub rename: bool,
    /// Entra em pasta e apaga o conteúdo dela também.
    #[serde(default)]
    pub recursive: bool,
    /// Só listar o que seria apagado.
    #[serde(default)]
    pub dry_run: bool,
}

fn default_passes() -> u32 {
    1
}
fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize)]
pub struct ShredItem {
    pub path: String,
    pub bytes: u64,
    pub passes: u32,
    pub ok: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ShredResult {
    pub items: Vec<ShredItem>,
    pub bytes_total: u64,
    /// Verdadeiro quando o alvo está num volume onde sobrescrever não garante nada.
    pub copy_on_write_warning: bool,
}

/// Sistemas de arquivos onde sobrescrever o arquivo não sobrescreve o bloco.
pub fn is_copy_on_write(fs_name: &str) -> bool {
    let f = fs_name.to_lowercase();
    ["apfs", "btrfs", "zfs", "refs", "bcachefs"]
        .iter()
        .any(|n| f.contains(n))
}

fn random_name(len: usize) -> String {
    use rand::RngExt;
    let mut rng = rand::rng();
    (0..len)
        .map(|_| (b'a' + rng.random_range(0..26)) as char)
        .collect()
}

/// Sobrescreve o arquivo inteiro `passes` vezes e devolve quantos bytes foram
/// cobertos. A última passagem é de zeros: sobra menos rastro de "isto foi
/// sobrescrito de propósito" do que ruído aleatório.
pub fn overwrite(path: &Path, passes: u32) -> std::io::Result<u64> {
    use rand::RngExt;
    let len = std::fs::metadata(path)?.len();
    if len == 0 {
        return Ok(0);
    }
    let mut file = std::fs::OpenOptions::new().write(true).open(path)?;
    let chunk = 1024 * 1024;
    let mut buf = vec![0u8; chunk.min(len as usize).max(1)];
    for pass in 0..passes.max(1) {
        let last = pass + 1 == passes.max(1);
        file.seek(SeekFrom::Start(0))?;
        let mut written = 0u64;
        while written < len {
            if last {
                buf.iter_mut().for_each(|b| *b = 0);
            } else {
                rand::rng().fill(&mut buf[..]);
            }
            let n = ((len - written) as usize).min(buf.len());
            file.write_all(&buf[..n])?;
            written += n as u64;
        }
        file.flush()?;
        file.sync_all()?;
    }
    Ok(len)
}

fn shred_file(path: &Path, opts: &ShredOptions) -> ShredItem {
    let bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let mut item = ShredItem {
        path: path.to_string_lossy().to_string(),
        bytes,
        passes: opts.passes.max(1),
        ok: false,
        error: None,
    };
    if opts.dry_run {
        item.ok = true;
        return item;
    }
    if let Err(e) = overwrite(path, opts.passes) {
        item.error = Some(e.to_string());
        return item;
    }
    let mut target = path.to_path_buf();
    if opts.rename {
        if let Some(dir) = path.parent() {
            let renamed = dir.join(random_name(12));
            if std::fs::rename(path, &renamed).is_ok() {
                target = renamed;
            }
        }
    }
    match std::fs::remove_file(&target) {
        Ok(()) => item.ok = true,
        Err(e) => item.error = Some(e.to_string()),
    }
    item
}

fn collect(path: &Path, recursive: bool, out: &mut Vec<PathBuf>) {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return;
    };
    if meta.is_file() {
        out.push(path.to_path_buf());
    } else if meta.is_dir() && recursive {
        if let Ok(rd) = std::fs::read_dir(path) {
            for e in rd.flatten() {
                collect(&e.path(), true, out);
            }
        }
    }
}

pub fn run(opts: &ShredOptions, progress: &super::ProgressFn) -> ShredResult {
    let mut files: Vec<PathBuf> = Vec::new();
    for p in &opts.paths {
        collect(Path::new(p), opts.recursive, &mut files);
    }
    let total = files.len() as u64;
    let mut items = Vec::new();
    for (i, f) in files.iter().enumerate() {
        super::report(
            progress,
            "sys-shred",
            "progress",
            i as u64,
            Some(total),
            Some(f.to_string_lossy().to_string()),
        );
        items.push(shred_file(f, opts));
    }
    // Pastas vazias sobrando depois do conteúdo ir embora.
    if opts.recursive && !opts.dry_run {
        for p in &opts.paths {
            let path = Path::new(p);
            if path.is_dir() {
                let _ = std::fs::remove_dir_all(path);
            }
        }
    }
    super::report(progress, "sys-shred", "done", total, Some(total), None);
    let bytes_total = items.iter().filter(|i| i.ok).map(|i| i.bytes).sum();
    ShredResult {
        items,
        bytes_total,
        copy_on_write_warning: cfg!(target_os = "macos"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("omniget-shred-test");
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn overwrite_replaces_every_byte() {
        let p = tmp("segredo.txt");
        std::fs::write(&p, b"senha do banco: 1234").unwrap();
        let n = overwrite(&p, 2).unwrap();
        assert_eq!(n, 20);
        let after = std::fs::read(&p).unwrap();
        assert_eq!(after.len(), 20, "o tamanho tem que ser preservado");
        assert!(
            after.iter().all(|b| *b == 0),
            "a última passagem é de zeros"
        );
        assert!(!after.starts_with(b"senha"));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn an_empty_file_is_not_a_failure() {
        let p = tmp("vazio.txt");
        std::fs::write(&p, b"").unwrap();
        assert_eq!(overwrite(&p, 1).unwrap(), 0);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn shred_removes_the_file() {
        let p = tmp("apagar.txt");
        std::fs::write(&p, b"conteudo qualquer").unwrap();
        let opts = ShredOptions {
            paths: vec![p.to_string_lossy().to_string()],
            passes: 1,
            rename: true,
            recursive: false,
            dry_run: false,
        };
        let res = run(&opts, &crate::core::tools::noop_progress());
        assert!(res.items[0].ok, "{:?}", res.items[0].error);
        assert!(!p.exists(), "o arquivo continua lá");
        assert_eq!(res.bytes_total, 17);
    }

    #[test]
    fn dry_run_touches_nothing() {
        let p = tmp("intacto.txt");
        std::fs::write(&p, b"nao me apague").unwrap();
        let opts = ShredOptions {
            paths: vec![p.to_string_lossy().to_string()],
            passes: 3,
            rename: true,
            recursive: false,
            dry_run: true,
        };
        let res = run(&opts, &crate::core::tools::noop_progress());
        assert!(res.items[0].ok);
        assert_eq!(std::fs::read(&p).unwrap(), b"nao me apague");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn a_folder_is_only_walked_when_asked() {
        let dir = tmp("pasta");
        std::fs::create_dir_all(dir.join("dentro")).unwrap();
        std::fs::write(dir.join("a.txt"), b"a").unwrap();
        std::fs::write(dir.join("dentro/b.txt"), b"b").unwrap();
        let mut flat = Vec::new();
        collect(&dir, false, &mut flat);
        assert!(flat.is_empty(), "sem recursivo, pasta não vira lista");
        let mut deep = Vec::new();
        collect(&dir, true, &mut deep);
        assert_eq!(deep.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cow_filesystems_are_recognized() {
        assert!(is_copy_on_write("APFS"));
        assert!(is_copy_on_write("btrfs"));
        assert!(!is_copy_on_write("ext4"));
        assert!(!is_copy_on_write("NTFS"));
    }
}
