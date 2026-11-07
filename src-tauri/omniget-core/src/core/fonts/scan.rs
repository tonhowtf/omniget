use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;
use ttf_parser::name_id;
use ttf_parser::{Face, Language, PlatformId};

const FONT_EXTENSIONS: [&str; 4] = ["ttf", "otf", "ttc", "otc"];
const MAX_DEPTH: usize = 6;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Axis {
    pub tag: String,
    pub min: f32,
    pub default: f32,
    pub max: f32,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FaceInfo {
    pub path: String,
    pub index: u32,
    pub family: String,
    pub style: String,
    pub full_name: String,
    pub weight: u16,
    pub italic: bool,
    pub is_variable: bool,
    pub axes: Vec<Axis>,
    pub is_monospace: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct InstalledStyle {
    pub name: String,
    pub weight: u16,
    pub italic: bool,
    pub path: String,
    pub index: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct InstalledFamily {
    pub family: String,
    pub styles: Vec<InstalledStyle>,
    pub weights: Vec<u16>,
    pub has_italic: bool,
    pub is_variable: bool,
    pub axes: Vec<Axis>,
    pub is_monospace: bool,
    pub files: Vec<String>,
    pub user_installed: bool,
}

static CACHE: Mutex<Option<Vec<InstalledFamily>>> = Mutex::new(None);

pub fn system_font_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let home = dirs::home_dir();
    if cfg!(target_os = "macos") {
        dirs.push(PathBuf::from("/System/Library/Fonts"));
        dirs.push(PathBuf::from("/Library/Fonts"));
        if let Some(h) = &home {
            dirs.push(h.join("Library/Fonts"));
        }
    } else if cfg!(target_os = "windows") {
        let windir = std::env::var_os("WINDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("C:\\Windows"));
        dirs.push(windir.join("Fonts"));
        if let Some(local) = dirs::data_local_dir() {
            dirs.push(local.join("Microsoft").join("Windows").join("Fonts"));
        }
    } else {
        dirs.push(PathBuf::from("/usr/share/fonts"));
        dirs.push(PathBuf::from("/usr/local/share/fonts"));
        if let Some(h) = &home {
            dirs.push(h.join(".local/share/fonts"));
            dirs.push(h.join(".fonts"));
        }
    }
    dirs
}

pub fn user_font_dir() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        dirs::home_dir().map(|h| h.join("Library").join("Fonts"))
    } else if cfg!(target_os = "windows") {
        dirs::data_local_dir().map(|d| d.join("Microsoft").join("Windows").join("Fonts"))
    } else {
        dirs::data_dir().map(|d| d.join("fonts"))
    }
}

fn is_font_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| FONT_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

fn decode_name(name: &ttf_parser::name::Name) -> Option<String> {
    let text = if let Some(s) = name.to_string() {
        s
    } else if name.platform_id == PlatformId::Macintosh && name.encoding_id == 0 {
        name.name.iter().map(|&b| mac_roman(b)).collect()
    } else {
        return None;
    };
    let text = text.trim_matches(char::from(0)).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn mac_roman(b: u8) -> char {
    if b < 0x80 {
        b as char
    } else {
        '\u{FFFD}'
    }
}

fn name_rank(name: &ttf_parser::name::Name) -> u8 {
    let english = name.language() == Language::English_UnitedStates;
    match (name.platform_id, english) {
        (PlatformId::Windows, true) => 0,
        (PlatformId::Unicode, _) => 1,
        (PlatformId::Macintosh, true) => 2,
        (PlatformId::Windows, false) => 3,
        _ => 4,
    }
}

fn best_name(face: &Face, ids: &[u16]) -> Option<String> {
    for &id in ids {
        let mut found: Vec<(u8, String)> = face
            .names()
            .into_iter()
            .filter(|n| n.name_id == id)
            .filter_map(|n| decode_name(&n).map(|s| (name_rank(&n), s)))
            .collect();
        found.sort_by_key(|(rank, _)| *rank);
        if let Some((_, s)) = found.into_iter().next() {
            return Some(s);
        }
    }
    None
}

fn looks_monospace(face: &Face) -> bool {
    if face.is_monospaced() {
        return true;
    }
    let advance = |c: char| {
        face.glyph_index(c)
            .and_then(|g| face.glyph_hor_advance(g))
            .unwrap_or(0)
    };
    let (i, m, w) = (advance('i'), advance('M'), advance('W'));
    i > 0 && i == m && m == w
}

fn axis_tag(tag: ttf_parser::Tag) -> String {
    tag.to_bytes()
        .iter()
        .map(|&b| if b.is_ascii_graphic() { b as char } else { ' ' })
        .collect::<String>()
        .trim()
        .to_string()
}

pub fn parse_face(data: &[u8], index: u32, path: &str) -> Option<FaceInfo> {
    let face = Face::parse(data, index).ok()?;
    let family = best_name(&face, &[name_id::TYPOGRAPHIC_FAMILY, name_id::FAMILY])?;
    let style = best_name(&face, &[name_id::TYPOGRAPHIC_SUBFAMILY, name_id::SUBFAMILY])
        .unwrap_or_else(|| "Regular".to_string());
    let full_name =
        best_name(&face, &[name_id::FULL_NAME]).unwrap_or_else(|| format!("{family} {style}"));
    let axes: Vec<Axis> = face
        .variation_axes()
        .into_iter()
        .filter(|a| !a.hidden)
        .map(|a| Axis {
            tag: axis_tag(a.tag),
            min: a.min_value,
            default: a.def_value,
            max: a.max_value,
        })
        .collect();
    Some(FaceInfo {
        path: path.to_string(),
        index,
        family,
        style,
        full_name,
        weight: face.weight().to_number(),
        italic: face.is_italic() || face.is_oblique(),
        is_variable: face.is_variable(),
        axes,
        is_monospace: looks_monospace(&face),
    })
}

pub fn parse_file(path: &Path) -> Vec<FaceInfo> {
    let Ok(data) = std::fs::read(path) else {
        return Vec::new();
    };
    parse_bytes(&data, &path.to_string_lossy())
}

pub fn parse_bytes(data: &[u8], path: &str) -> Vec<FaceInfo> {
    let count = ttf_parser::fonts_in_collection(data).unwrap_or(1);
    (0..count)
        .filter_map(|i| parse_face(data, i, path))
        .collect()
}

fn walk(dir: &Path, depth: usize, seen: &mut HashSet<PathBuf>, out: &mut Vec<PathBuf>) {
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let real = if kind.is_symlink() {
            match std::fs::canonicalize(&path) {
                Ok(r) => r,
                Err(_) => continue,
            }
        } else {
            path.clone()
        };
        if real.is_dir() {
            if seen.insert(real.clone()) {
                walk(&real, depth + 1, seen, out);
            }
        } else if is_font_file(&real) && seen.insert(real.clone()) {
            out.push(real);
        }
    }
}

pub fn font_files(dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for dir in dirs {
        walk(dir, 0, &mut seen, &mut out);
    }
    out.sort();
    out
}

fn family_key(name: &str) -> String {
    name.to_lowercase()
}

pub fn axis_weights(min: f32, max: f32) -> Vec<u16> {
    if !min.is_finite() || !max.is_finite() || max < min {
        return Vec::new();
    }
    let lo = min.max(1.0);
    let hi = max.min(1000.0);
    (1..=10u16)
        .map(|n| n * 100)
        .filter(|w| f32::from(*w) >= lo && f32::from(*w) <= hi)
        .collect()
}

pub fn group_families(faces: Vec<FaceInfo>, user_dir: Option<&Path>) -> Vec<InstalledFamily> {
    let mut map: BTreeMap<String, InstalledFamily> = BTreeMap::new();
    for face in faces {
        if face.family.starts_with('.') {
            continue;
        }
        let user = user_dir
            .map(|d| Path::new(&face.path).starts_with(d))
            .unwrap_or(false);
        let entry = map
            .entry(family_key(&face.family))
            .or_insert_with(|| InstalledFamily {
                family: face.family.clone(),
                styles: Vec::new(),
                weights: Vec::new(),
                has_italic: false,
                is_variable: false,
                axes: Vec::new(),
                is_monospace: true,
                files: Vec::new(),
                user_installed: false,
            });
        entry.user_installed |= user;
        entry.is_monospace &= face.is_monospace;
        entry.has_italic |= face.italic;
        if face.is_variable {
            entry.is_variable = true;
            for axis in &face.axes {
                match entry.axes.iter_mut().find(|a| a.tag == axis.tag) {
                    Some(a) => {
                        a.min = a.min.min(axis.min);
                        a.max = a.max.max(axis.max);
                    }
                    None => entry.axes.push(axis.clone()),
                }
            }
        }
        if !entry.files.contains(&face.path) {
            entry.files.push(face.path.clone());
        }
        if !entry
            .styles
            .iter()
            .any(|s| s.name == face.style && s.italic == face.italic && s.weight == face.weight)
        {
            entry.styles.push(InstalledStyle {
                name: face.style.clone(),
                weight: face.weight,
                italic: face.italic,
                path: face.path.clone(),
                index: face.index,
            });
        }
        if !entry.weights.contains(&face.weight) {
            entry.weights.push(face.weight);
        }
    }
    let mut families: Vec<InstalledFamily> = map
        .into_values()
        .map(|mut f| {
            if let Some(axis) = f.axes.iter().find(|a| a.tag == "wght") {
                for w in axis_weights(axis.min, axis.max) {
                    if !f.weights.contains(&w) {
                        f.weights.push(w);
                    }
                }
            }
            f.weights.sort_unstable();
            f.styles.sort_by(|a, b| {
                (a.weight, a.italic, a.name.as_str()).cmp(&(b.weight, b.italic, b.name.as_str()))
            });
            f
        })
        .collect();
    families.sort_by_key(|f| family_key(&f.family));
    families
}

pub fn scan_dirs(dirs: &[PathBuf]) -> Vec<InstalledFamily> {
    let faces: Vec<FaceInfo> = font_files(dirs)
        .iter()
        .flat_map(|p| parse_file(p))
        .collect();
    group_families(faces, user_font_dir().as_deref())
}

pub fn installed(refresh: bool) -> Vec<InstalledFamily> {
    if !refresh {
        if let Ok(guard) = CACHE.lock() {
            if let Some(list) = guard.as_ref() {
                return list.clone();
            }
        }
    }
    let list = scan_dirs(&system_font_dirs());
    if let Ok(mut guard) = CACHE.lock() {
        *guard = Some(list.clone());
    }
    list
}

pub fn invalidate() {
    if let Ok(mut guard) = CACHE.lock() {
        *guard = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn system_font(names: &[&str]) -> Option<PathBuf> {
        names.iter().map(PathBuf::from).find(|p| p.exists())
    }

    #[test]
    fn parses_name_table_of_a_system_font() {
        let Some(path) = system_font(&[
            "/System/Library/Fonts/Supplemental/Arial.ttf",
            "/Library/Fonts/Arial.ttf",
            "C:\\Windows\\Fonts\\arial.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        ]) else {
            return;
        };
        let faces = parse_file(&path);
        assert_eq!(faces.len(), 1);
        let face = &faces[0];
        assert!(face.family == "Arial" || face.family == "DejaVu Sans");
        assert_eq!(face.weight, 400);
        assert!(!face.italic);
        assert!(!face.is_monospace);
    }

    #[test]
    fn detects_monospace_and_collections() {
        if let Some(path) = system_font(&["/System/Library/Fonts/Supplemental/Courier New.ttf"]) {
            let faces = parse_file(&path);
            assert!(faces.iter().all(|f| f.is_monospace));
        }
        if let Some(path) = system_font(&["/System/Library/Fonts/Menlo.ttc"]) {
            let faces = parse_file(&path);
            assert!(faces.len() >= 4);
            assert!(faces.iter().all(|f| f.family == "Menlo" && f.is_monospace));
            assert!(faces.iter().any(|f| f.italic));
            assert!(faces.iter().any(|f| f.weight >= 700));
        }
    }

    #[test]
    fn reads_variable_axes() {
        let Some(path) = system_font(&[
            "/System/Library/Fonts/SFNS.ttf",
            "/System/Library/Fonts/NewYork.ttf",
        ]) else {
            return;
        };
        let faces = parse_file(&path);
        assert!(!faces.is_empty());
        assert!(faces[0].is_variable);
        assert!(faces[0]
            .axes
            .iter()
            .any(|a| a.tag == "wght" && a.min < a.max));
    }

    #[test]
    fn skips_unreadable_and_garbage_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("broken.ttf"), b"not a font").unwrap();
        std::fs::write(dir.path().join("notes.txt"), b"hello").unwrap();
        assert!(parse_file(&dir.path().join("broken.ttf")).is_empty());
        assert!(parse_file(&dir.path().join("missing.otf")).is_empty());
        let files = font_files(&[dir.path().to_path_buf(), dir.path().join("nope")]);
        assert_eq!(files.len(), 1);
        assert!(scan_dirs(&[dir.path().to_path_buf()]).is_empty());
    }

    #[test]
    fn scans_the_system_font_folders() {
        if !Path::new("/System/Library/Fonts/Menlo.ttc").exists() {
            return;
        }
        let families = installed(true);
        assert!(families.len() > 20);
        assert!(families
            .iter()
            .any(|f| f.family == "Menlo" && f.is_monospace));
        assert!(families.iter().all(|f| !f.family.starts_with('.')));
        assert_eq!(installed(false), families);
    }

    fn face(family: &str, style: &str, weight: u16, italic: bool, path: &str) -> FaceInfo {
        FaceInfo {
            path: path.to_string(),
            index: 0,
            family: family.to_string(),
            style: style.to_string(),
            full_name: format!("{family} {style}"),
            weight,
            italic,
            is_variable: false,
            axes: Vec::new(),
            is_monospace: false,
        }
    }

    #[test]
    fn groups_faces_into_families() {
        let mut vf = face("Inter", "Regular", 400, false, "/u/Inter[wght].ttf");
        vf.is_variable = true;
        vf.axes = vec![Axis {
            tag: "wght".into(),
            min: 100.0,
            default: 400.0,
            max: 900.0,
        }];
        let faces = vec![
            face("Inter", "Bold", 700, false, "/s/Inter-Bold.ttf"),
            face("inter", "Regular", 400, false, "/s/Inter-Regular.ttf"),
            face("Inter", "Regular", 400, false, "/s/Inter-Regular.ttf"),
            face("Inter", "Italic", 400, true, "/s/Inter-Italic.ttf"),
            vf,
            face(".SF NS Hidden", "Regular", 400, false, "/s/hidden.ttf"),
            face("Arial", "Regular", 400, false, "/s/Arial.ttf"),
        ];
        let families = group_families(faces, Some(Path::new("/u")));
        assert_eq!(families.len(), 2);
        assert_eq!(families[0].family, "Arial");
        let inter = &families[1];
        assert_eq!(
            inter.weights,
            vec![100, 200, 300, 400, 500, 600, 700, 800, 900]
        );
        assert_eq!(families[0].weights, vec![400]);
        assert_eq!(axis_weights(250.0, 650.0), vec![300, 400, 500, 600]);
        assert!(axis_weights(f32::NAN, 900.0).is_empty());
        assert!(axis_weights(700.0, 300.0).is_empty());
        assert_eq!(inter.styles.len(), 3);
        assert!(inter.has_italic && inter.is_variable && inter.user_installed);
        assert_eq!(inter.files.len(), 4);
        assert!(!families[0].user_installed);
    }
}
