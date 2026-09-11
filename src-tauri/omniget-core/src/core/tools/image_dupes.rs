//! Fotos duplicadas e parecidas. O `files-dupes` acha arquivo byte a byte;
//! este acha a mesma foto salva duas vezes com qualidade diferente, o print
//! recortado, o WhatsApp que recomprimiu — coisas que o hash de bytes não vê.
//!
//! Reusa o dHash que a categoria Pinterest já usa, então "parecido" quer dizer
//! a mesma coisa nas duas telas.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::pinterest::analysis::{dhash, hamming};

const IMAGE_EXTS: &[&str] = &[
    "jpg", "jpeg", "png", "webp", "gif", "bmp", "tif", "tiff", "avif", "heic",
];

#[derive(Debug, Clone, Deserialize)]
pub struct ImgDupesOptions {
    pub dirs: Vec<String>,
    /// 0 = só idênticas; 5 é o padrão do Pinterest para "parecidas".
    #[serde(default = "default_threshold")]
    pub threshold: u32,
    #[serde(default = "default_min")]
    pub min_size: u64,
    #[serde(default = "default_true")]
    pub skip_hidden: bool,
}

fn default_threshold() -> u32 {
    5
}
fn default_min() -> u64 {
    16 * 1024
}
fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize)]
pub struct ImgFile {
    pub path: String,
    pub bytes: u64,
    pub width: u32,
    pub height: u32,
    /// A cópia que vale a pena manter do grupo.
    pub best: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImgGroup {
    /// "exact" = bytes idênticos · "same" = mesma imagem reencodada · "near"
    pub kind: String,
    pub distance: u32,
    pub files: Vec<ImgFile>,
    pub wasted_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImgDupesResult {
    pub scanned: u64,
    pub hashed: u64,
    pub groups: Vec<ImgGroup>,
    pub wasted_bytes: u64,
}

fn is_image(p: &Path) -> bool {
    p.extension()
        .map(|e| IMAGE_EXTS.contains(&e.to_string_lossy().to_lowercase().as_str()))
        .unwrap_or(false)
}

fn walk(root: &Path, skip_hidden: bool, min_size: u64, out: &mut Vec<(PathBuf, u64)>) {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in rd.flatten() {
            let p = entry.path();
            if skip_hidden && entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                stack.push(p);
            } else if meta.is_file() && meta.len() >= min_size && is_image(&p) {
                out.push((p, meta.len()));
            }
        }
    }
}

/// A melhor cópia é a de maior resolução; empatou, a de maior arquivo (menos
/// recomprimida). É a que fica marcada para o usuário não apagar sem querer.
pub fn pick_best(files: &[ImgFile]) -> usize {
    let mut best = 0usize;
    for (i, f) in files.iter().enumerate() {
        let a = (f.width as u64 * f.height as u64, f.bytes);
        let b = (
            files[best].width as u64 * files[best].height as u64,
            files[best].bytes,
        );
        if a > b {
            best = i;
        }
    }
    best
}

/// União-busca sobre as distâncias de dHash. Devolve o índice do grupo de cada
/// arquivo, ou `None` para quem ficou sozinho.
pub fn cluster(hashes: &[u64], threshold: u32) -> Vec<Option<usize>> {
    let n = hashes.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(p: &mut [usize], i: usize) -> usize {
        let mut i = i;
        while p[i] != i {
            p[i] = p[p[i]];
            i = p[i];
        }
        i
    }
    for i in 0..n {
        for j in (i + 1)..n {
            if hamming(hashes[i], hashes[j]) <= threshold {
                let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                if a != b {
                    parent[a] = b;
                }
            }
        }
    }
    let mut sizes = vec![0usize; n];
    let roots: Vec<usize> = (0..n).map(|i| find(&mut parent, i)).collect();
    for &r in &roots {
        sizes[r] += 1;
    }
    let mut index = vec![usize::MAX; n];
    let mut next = 0usize;
    roots
        .iter()
        .map(|&r| {
            if sizes[r] < 2 {
                return None;
            }
            if index[r] == usize::MAX {
                index[r] = next;
                next += 1;
            }
            Some(index[r])
        })
        .collect()
}

pub fn scan(opts: &ImgDupesOptions, progress: &super::ProgressFn) -> ImgDupesResult {
    let mut found: Vec<(PathBuf, u64)> = Vec::new();
    for dir in &opts.dirs {
        walk(Path::new(dir), opts.skip_hidden, opts.min_size, &mut found);
    }
    let scanned = found.len() as u64;

    let mut paths: Vec<PathBuf> = Vec::new();
    let mut sizes: Vec<u64> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let mut dims: Vec<(u32, u32)> = Vec::new();
    let mut exact: Vec<String> = Vec::new();
    for (i, (path, size)) in found.iter().enumerate() {
        if i % 25 == 0 {
            super::report(
                progress,
                "img-dupes",
                "progress",
                i as u64,
                Some(scanned),
                Some(path.to_string_lossy().to_string()),
            );
        }
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let Some(h) = dhash(&bytes) else { continue };
        let dim = image::load_from_memory(&bytes)
            .map(|i| (i.width(), i.height()))
            .unwrap_or((0, 0));
        use sha2::Digest;
        exact.push(hex::encode(sha2::Sha256::digest(&bytes)));
        paths.push(path.clone());
        sizes.push(*size);
        hashes.push(h);
        dims.push(dim);
    }
    let hashed = paths.len() as u64;

    let clusters = cluster(&hashes, opts.threshold);
    let count = clusters.iter().flatten().copied().max().map(|m| m + 1);
    let mut groups: Vec<ImgGroup> = Vec::new();
    for gi in 0..count.unwrap_or(0) {
        let members: Vec<usize> = clusters
            .iter()
            .enumerate()
            .filter(|(_, c)| **c == Some(gi))
            .map(|(i, _)| i)
            .collect();
        if members.len() < 2 {
            continue;
        }
        let mut files: Vec<ImgFile> = members
            .iter()
            .map(|&i| ImgFile {
                path: paths[i].to_string_lossy().to_string(),
                bytes: sizes[i],
                width: dims[i].0,
                height: dims[i].1,
                best: false,
            })
            .collect();
        let best = pick_best(&files);
        files[best].best = true;
        let mut maxd = 0u32;
        for a in 0..members.len() {
            for b in (a + 1)..members.len() {
                maxd = maxd.max(hamming(hashes[members[a]], hashes[members[b]]));
            }
        }
        let all_same_bytes = members.iter().all(|&i| exact[i] == exact[members[0]]);
        let all_same_pixels = members
            .iter()
            .all(|&i| dims[i] == dims[members[0]] && maxd == 0);
        let wasted: u64 = files
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != best)
            .map(|(_, f)| f.bytes)
            .sum();
        groups.push(ImgGroup {
            kind: if all_same_bytes {
                "exact".into()
            } else if all_same_pixels {
                "same".into()
            } else {
                "near".into()
            },
            distance: maxd,
            files,
            wasted_bytes: wasted,
        });
    }
    groups.sort_by_key(|g| std::cmp::Reverse(g.wasted_bytes));
    let wasted_bytes = groups.iter().map(|g| g.wasted_bytes).sum();
    super::report(progress, "img-dupes", "done", scanned, Some(scanned), None);
    ImgDupesResult {
        scanned,
        hashed,
        groups,
        wasted_bytes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(path: &str, w: u32, h: u32, bytes: u64) -> ImgFile {
        ImgFile {
            path: path.into(),
            bytes,
            width: w,
            height: h,
            best: false,
        }
    }

    #[test]
    fn best_is_the_biggest_resolution() {
        let files = vec![f("a", 800, 600, 900_000), f("b", 1600, 1200, 100_000)];
        assert_eq!(
            pick_best(&files),
            1,
            "resolução ganha do tamanho do arquivo"
        );
    }

    #[test]
    fn tie_on_resolution_goes_to_the_bigger_file() {
        let files = vec![f("a", 800, 600, 100_000), f("b", 800, 600, 400_000)];
        assert_eq!(pick_best(&files), 1);
    }

    #[test]
    fn cluster_groups_by_distance() {
        // 0 e 1 diferem em 1 bit; 2 está longe.
        let hashes = vec![0b1010, 0b1011, 0xFFFF_FFFF_FFFF_FFFF];
        let c = cluster(&hashes, 2);
        assert_eq!(c[0], c[1]);
        assert!(c[0].is_some());
        assert_eq!(c[2], None, "sozinho não vira grupo");
    }

    #[test]
    fn threshold_zero_only_groups_identical_hashes() {
        let hashes = vec![0b1010, 0b1011, 0b1010];
        let c = cluster(&hashes, 0);
        assert_eq!(c[0], c[2]);
        assert_eq!(c[1], None);
    }

    #[test]
    fn transitive_grouping_holds() {
        // a~b e b~c, mesmo com a e c a 4 bits de distância.
        let hashes = vec![0b0000, 0b0011, 0b1111];
        let c = cluster(&hashes, 2);
        assert_eq!(c[0], c[2], "o grupo tem que ser transitivo");
    }

    #[test]
    fn only_image_extensions_are_walked() {
        assert!(is_image(Path::new("/a/foto.JPG")));
        assert!(is_image(Path::new("/a/foto.heic")));
        assert!(!is_image(Path::new("/a/video.mp4")));
        assert!(!is_image(Path::new("/a/sem-extensao")));
    }
}
