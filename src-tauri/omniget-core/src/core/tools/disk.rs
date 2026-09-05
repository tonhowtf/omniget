//! Analisador de disco (estudo 10, Kudu): volumes com espaço livre, árvore de
//! tamanhos de uma pasta (para o treemap) e os maiores arquivos. Uma única
//! varredura com `walkdir` alimenta as três coisas; a árvore devolvida é
//! podada (profundidade e número de filhos) para caber na tela.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Volume {
    pub path: String,
    pub name: String,
    pub total: u64,
    pub free: u64,
}

fn volume_at(path: &Path, name: &str) -> Option<Volume> {
    let total = fs4::total_space(path).ok()?;
    let free = fs4::available_space(path).ok()?;
    if total == 0 {
        return None;
    }
    Some(Volume { path: path.to_string_lossy().to_string(), name: name.to_string(), total, free })
}

pub fn volumes() -> Vec<Volume> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut push = |p: PathBuf, name: String| {
        if !p.exists() {
            return;
        }
        if let Some(v) = volume_at(&p, &name) {
            let key = (v.total, v.free);
            if seen.insert(key) {
                out.push(v);
            }
        }
    };
    if cfg!(target_os = "windows") {
        for letter in b'A'..=b'Z' {
            let p = PathBuf::from(format!("{}:\\", letter as char));
            push(p, format!("{}:", letter as char));
        }
    } else if cfg!(target_os = "macos") {
        push(PathBuf::from("/"), "Macintosh HD".into());
        if let Ok(rd) = std::fs::read_dir("/Volumes") {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                push(e.path(), name);
            }
        }
    } else {
        push(PathBuf::from("/"), "/".into());
        push(PathBuf::from("/home"), "/home".into());
        let user = std::env::var("USER").unwrap_or_default();
        for base in [format!("/media/{}", user), format!("/run/media/{}", user), "/mnt".to_string()] {
            if let Ok(rd) = std::fs::read_dir(&base) {
                for e in rd.flatten() {
                    let name = e.file_name().to_string_lossy().to_string();
                    push(e.path(), name);
                }
            }
        }
    }
    if let Some(home) = dirs::home_dir() {
        push(home, "Home".into());
    }
    out
}

#[derive(Debug, Clone, Serialize)]
pub struct Node {
    pub name: String,
    pub path: String,
    pub bytes: u64,
    pub is_dir: bool,
    pub files: u64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Node>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BigFile {
    pub path: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiskScan {
    pub root: Node,
    pub largest: Vec<BigFile>,
    pub scanned: u64,
    pub skipped: u64,
}

/// Varre `root` e devolve a árvore até `max_depth` níveis, com no máximo
/// `max_children` filhos por nó (o resto vira "outros").
pub fn scan(root: &str, max_depth: usize, max_children: usize, progress: &super::ProgressFn) -> anyhow::Result<DiskScan> {
    let root_path = PathBuf::from(root.trim());
    if !root_path.is_dir() {
        anyhow::bail!("pasta nao encontrada: {}", root_path.display());
    }
    let max_depth = max_depth.clamp(1, 8);
    let max_children = max_children.clamp(5, 200);

    // Os maiores arquivos e, por diretório, a lista de filhos diretos com
    // tamanho somado de baixo para cima.
    let mut largest: Vec<BigFile> = Vec::new();
    let mut scanned = 0u64;
    let mut skipped = 0u64;
    let mut last = std::time::Instant::now();

    let walker = walkdir::WalkDir::new(&root_path).follow_links(false).min_depth(1);
    // Pilha "caminho → bytes" para fechar diretórios ao sair deles: walkdir é
    // pré-ordem, então guardamos o acumulado de cada diretório aberto.
    let mut open: Vec<(PathBuf, u64, u64)> = vec![(root_path.clone(), 0, 0)];
    let mut immediate: HashMap<PathBuf, Vec<(String, u64, bool, u64)>> = HashMap::new();

    fn close_until(open: &mut Vec<(PathBuf, u64, u64)>, immediate: &mut HashMap<PathBuf, Vec<(String, u64, bool, u64)>>, keep: &Path) {
        while open.len() > 1 && !keep.starts_with(&open.last().unwrap().0) {
            let (p, b, f) = open.pop().unwrap();
            let parent = open.last_mut().unwrap();
            parent.1 += b;
            parent.2 += f;
            let name = p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            immediate.entry(parent.0.clone()).or_default().push((name, b, true, f));
        }
    }

    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        scanned += 1;
        if last.elapsed() > std::time::Duration::from_millis(250) {
            super::report(progress, "disk", "progress", scanned, None, Some(entry.path().to_string_lossy().to_string()));
            last = std::time::Instant::now();
        }
        let path = entry.path().to_path_buf();
        let parent = path.parent().unwrap_or(&root_path).to_path_buf();
        close_until(&mut open, &mut immediate, &parent);
        if entry.file_type().is_dir() {
            open.push((path, 0, 0));
        } else if entry.file_type().is_file() {
            let bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
            if let Some(top) = open.last_mut() {
                top.1 += bytes;
                top.2 += 1;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            immediate.entry(parent).or_default().push((name, bytes, false, 1));
            if largest.len() < 60 || bytes > largest.last().map(|b| b.bytes).unwrap_or(0) {
                largest.push(BigFile { path: path.to_string_lossy().to_string(), bytes });
                largest.sort_by_key(|b| std::cmp::Reverse(b.bytes));
                largest.truncate(60);
            }
        }
    }
    close_until(&mut open, &mut immediate, &root_path);
    let (_, root_bytes, root_files) = open.pop().unwrap_or((root_path.clone(), 0, 0));

    #[allow(clippy::too_many_arguments)]
    fn build(path: &Path, name: String, bytes: u64, files: u64, depth: usize, max_depth: usize, max_children: usize, immediate: &HashMap<PathBuf, Vec<(String, u64, bool, u64)>>) -> Node {
        let mut children = Vec::new();
        if depth < max_depth {
            if let Some(list) = immediate.get(path) {
                let mut list = list.clone();
                list.sort_by_key(|b| std::cmp::Reverse(b.1));
                let mut rest_bytes = 0u64;
                let mut rest_files = 0u64;
                let mut rest_n = 0usize;
                for (i, (n, b, is_dir, f)) in list.into_iter().enumerate() {
                    if i < max_children {
                        let child_path = path.join(&n);
                        if is_dir {
                            children.push(build(&child_path, n, b, f, depth + 1, max_depth, max_children, immediate));
                        } else {
                            children.push(Node { name: n, path: child_path.to_string_lossy().to_string(), bytes: b, is_dir: false, files: 1, children: vec![] });
                        }
                    } else {
                        rest_bytes += b;
                        rest_files += f;
                        rest_n += 1;
                    }
                }
                if rest_n > 0 {
                    children.push(Node { name: format!("… {} itens", rest_n), path: String::new(), bytes: rest_bytes, is_dir: false, files: rest_files, children: vec![] });
                }
            }
        }
        Node { name, path: path.to_string_lossy().to_string(), bytes, is_dir: true, files, children }
    }

    let name = root_path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| root_path.to_string_lossy().to_string());
    let root_node = build(&root_path, name, root_bytes, root_files, 0, max_depth, max_children, &immediate);
    super::report(progress, "disk", "done", scanned, Some(scanned), None);
    Ok(DiskScan { root: root_node, largest, scanned, skipped })
}

/// Move para a lixeira (nunca apaga direto).
pub fn trash_paths(paths: &[String]) -> (Vec<String>, Vec<String>) {
    let mut ok = Vec::new();
    let mut failed = Vec::new();
    for p in paths {
        if p.trim().is_empty() {
            continue;
        }
        match trash::delete(p) {
            Ok(_) => ok.push(p.clone()),
            Err(_) => failed.push(p.clone()),
        }
    }
    (ok, failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree() {
        let dir = std::env::temp_dir().join(format!("omniget-disk-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("a/b")).unwrap();
        std::fs::write(dir.join("a/x.bin"), vec![0u8; 300]).unwrap();
        std::fs::write(dir.join("a/b/y.bin"), vec![0u8; 200]).unwrap();
        std::fs::write(dir.join("z.bin"), vec![0u8; 100]).unwrap();
        let s = scan(&dir.to_string_lossy(), 3, 50, &super::super::noop_progress()).unwrap();
        assert_eq!(s.root.bytes, 600);
        assert_eq!(s.root.files, 3);
        let a = s.root.children.iter().find(|c| c.name == "a").unwrap();
        assert_eq!(a.bytes, 500);
        assert!(a.is_dir);
        let b = a.children.iter().find(|c| c.name == "b").unwrap();
        assert_eq!(b.bytes, 200);
        assert_eq!(s.largest[0].bytes, 300);
        assert!(!volumes().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
