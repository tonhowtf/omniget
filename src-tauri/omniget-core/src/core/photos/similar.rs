use std::cmp::Ordering;
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use serde::Serialize;

use super::library::{Hashed, Library};

pub const DEFAULT_DISTANCE: u32 = 10;
pub const MAX_DISTANCE: u32 = 16;
const FLAT_BITS: u32 = 4;

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DupKind {
    Exact,
    Similar,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct DupGroup {
    pub kind: DupKind,
    pub keep: String,
    pub paths: Vec<String>,
    pub bytes: u64,
}

pub fn content_hash(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn better(a: &Hashed, b: &Hashed) -> Ordering {
    let pixels = |h: &Hashed| u64::from(h.width) * u64::from(h.height);
    pixels(b)
        .cmp(&pixels(a))
        .then(b.size.cmp(&a.size))
        .then(
            a.taken_at
                .unwrap_or(a.modified_at)
                .cmp(&b.taken_at.unwrap_or(b.modified_at)),
        )
        .then(a.path.len().cmp(&b.path.len()))
        .then(a.path.cmp(&b.path))
}

fn informative(hash: u64) -> bool {
    let ones = hash.count_ones();
    (FLAT_BITS..=64 - FLAT_BITS).contains(&ones)
}

struct BkTree {
    nodes: Vec<(u64, usize, HashMap<u32, usize>)>,
}

impl BkTree {
    fn new() -> BkTree {
        BkTree { nodes: Vec::new() }
    }

    fn insert(&mut self, hash: u64, item: usize) {
        if self.nodes.is_empty() {
            self.nodes.push((hash, item, HashMap::new()));
            return;
        }
        let mut at = 0;
        loop {
            let d = (self.nodes[at].0 ^ hash).count_ones();
            match self.nodes[at].2.get(&d) {
                Some(&child) => at = child,
                None => {
                    let id = self.nodes.len();
                    self.nodes.push((hash, item, HashMap::new()));
                    self.nodes[at].2.insert(d, id);
                    return;
                }
            }
        }
    }

    fn within(&self, hash: u64, max: u32, out: &mut Vec<usize>) {
        if self.nodes.is_empty() {
            return;
        }
        let mut stack = vec![0usize];
        while let Some(at) = stack.pop() {
            let (h, item, children) = &self.nodes[at];
            let d = (h ^ hash).count_ones();
            if d <= max {
                out.push(*item);
            }
            let lo = d.saturating_sub(max);
            let hi = d + max;
            stack.extend(
                children
                    .iter()
                    .filter(|(k, _)| (lo..=hi).contains(*k))
                    .map(|(_, v)| *v),
            );
        }
    }
}

pub fn fill_content(lib: &Library, items: &mut [Hashed]) -> Result<(), String> {
    let mut by_size: HashMap<u64, usize> = HashMap::new();
    for h in items.iter() {
        *by_size.entry(h.size).or_default() += 1;
    }
    let mut fresh = Vec::new();
    for h in items.iter_mut() {
        if h.content.is_some() || by_size.get(&h.size).copied().unwrap_or(0) < 2 {
            continue;
        }
        if let Ok(c) = content_hash(Path::new(&h.path)) {
            fresh.push((h.path.clone(), c.clone()));
            h.content = Some(c);
        }
    }
    lib.set_content(&fresh)
}

pub fn group(items: &[Hashed], max_distance: u32) -> Vec<DupGroup> {
    let max_distance = max_distance.min(MAX_DISTANCE);
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by(|&a, &b| better(&items[a], &items[b]));

    let mut rep_of: Vec<usize> = (0..items.len()).collect();
    let mut by_content: HashMap<&str, usize> = HashMap::new();
    for &i in &order {
        if let Some(c) = items[i].content.as_deref() {
            match by_content.get(c) {
                Some(&rep) => rep_of[i] = rep,
                None => {
                    by_content.insert(c, i);
                }
            }
        }
    }
    let mut members: HashMap<usize, Vec<usize>> = HashMap::new();
    for &i in &order {
        members.entry(rep_of[i]).or_default().push(i);
    }
    let reps: Vec<usize> = order.iter().copied().filter(|&i| rep_of[i] == i).collect();

    let mut tree = BkTree::new();
    for &r in &reps {
        if let Some(h) = items[r].dhash.filter(|h| informative(*h)) {
            tree.insert(h, r);
        }
    }

    let mut taken = vec![false; items.len()];
    let mut groups = Vec::new();
    let mut near = Vec::new();
    for &leader in &reps {
        if taken[leader] {
            continue;
        }
        taken[leader] = true;
        let mut cluster = vec![leader];
        if max_distance > 0 {
            if let Some(h) = items[leader].dhash.filter(|h| informative(*h)) {
                near.clear();
                tree.within(h, max_distance, &mut near);
                near.sort_by(|&a, &b| better(&items[a], &items[b]));
                for &n in &near {
                    if !taken[n] {
                        taken[n] = true;
                        cluster.push(n);
                    }
                }
            }
        }
        let paths: Vec<usize> = cluster
            .iter()
            .flat_map(|r| members.get(r).cloned().unwrap_or_default())
            .collect();
        if paths.len() < 2 {
            continue;
        }
        let kind = if cluster.len() == 1 {
            DupKind::Exact
        } else {
            DupKind::Similar
        };
        groups.push(DupGroup {
            kind,
            keep: items[leader].path.clone(),
            bytes: paths.iter().skip(1).map(|&i| items[i].size).sum(),
            paths: paths.iter().map(|&i| items[i].path.clone()).collect(),
        });
    }
    groups.sort_by(|a, b| {
        (a.kind as u8)
            .cmp(&(b.kind as u8))
            .then(b.bytes.cmp(&a.bytes))
            .then(a.keep.cmp(&b.keep))
    });
    groups
}
