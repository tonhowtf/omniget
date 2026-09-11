//! Biblioteca Steam local, sem rede: o `appmanifest_<appid>.acf` e o
//! `libraryfolders.vdf` são texto no formato KeyValues da Valve. Ler os dois
//! resolve appid → nome do jogo e descobre em quais discos a biblioteca está.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// KeyValues da Valve: ou um texto, ou uma lista de pares (a ordem importa e
/// chaves repetidas existem, então não é um mapa).
#[derive(Debug, Clone, PartialEq)]
pub enum Vdf {
    Str(String),
    Map(Vec<(String, Vdf)>),
}

impl Vdf {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Vdf::Str(s) => Some(s),
            Vdf::Map(_) => None,
        }
    }

    pub fn entries(&self) -> &[(String, Vdf)] {
        match self {
            Vdf::Map(v) => v,
            Vdf::Str(_) => &[],
        }
    }

    /// As chaves do formato variam de maiúscula entre versões do cliente
    /// (`SizeOnDisk`, `sizeondisk`), então a busca ignora caixa.
    pub fn get(&self, key: &str) -> Option<&Vdf> {
        self.entries()
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v)
    }

    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(|v| v.as_str())
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Str(String),
    Open,
    Close,
}

fn tokenize(text: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '/' && i + 1 < chars.len() && chars[i + 1] == '/' {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '{' {
            out.push(Token::Open);
            i += 1;
            continue;
        }
        if c == '}' {
            out.push(Token::Close);
            i += 1;
            continue;
        }
        if c == '"' {
            i += 1;
            let mut s = String::new();
            while i < chars.len() && chars[i] != '"' {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    i += 1;
                    s.push(match chars[i] {
                        'n' => '\n',
                        't' => '\t',
                        other => other,
                    });
                } else {
                    s.push(chars[i]);
                }
                i += 1;
            }
            i += 1;
            out.push(Token::Str(s));
            continue;
        }
        // Token solto (sem aspas) — o formato aceita, alguns arquivos usam.
        let mut s = String::new();
        while i < chars.len()
            && !chars[i].is_whitespace()
            && chars[i] != '{'
            && chars[i] != '}'
            && chars[i] != '"'
        {
            s.push(chars[i]);
            i += 1;
        }
        out.push(Token::Str(s));
    }
    out
}

fn parse_map(tokens: &[Token], pos: &mut usize, depth: u32) -> Vdf {
    let mut entries: Vec<(String, Vdf)> = Vec::new();
    // Um arquivo corrompido não pode virar recursão infinita.
    if depth > 32 {
        return Vdf::Map(entries);
    }
    while *pos < tokens.len() {
        match &tokens[*pos] {
            Token::Close => {
                *pos += 1;
                break;
            }
            Token::Open => {
                // Bloco sem chave: ignora a abertura e segue.
                *pos += 1;
                let _ = parse_map(tokens, pos, depth + 1);
            }
            Token::Str(key) => {
                let key = key.clone();
                *pos += 1;
                match tokens.get(*pos) {
                    Some(Token::Open) => {
                        *pos += 1;
                        let value = parse_map(tokens, pos, depth + 1);
                        entries.push((key, value));
                    }
                    Some(Token::Str(v)) => {
                        let v = v.clone();
                        *pos += 1;
                        entries.push((key, Vdf::Str(v)));
                    }
                    _ => break,
                }
            }
        }
    }
    Vdf::Map(entries)
}

/// Raiz do arquivo. O `.acf` tem um único bloco no topo (`AppState`).
pub fn parse_vdf(text: &str) -> Vdf {
    let tokens = tokenize(text);
    let mut pos = 0usize;
    parse_map(&tokens, &mut pos, 0)
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SteamApp {
    pub app_id: u32,
    pub name: String,
    pub install_dir: String,
    pub size_on_disk: u64,
    /// Pasta `steamapps` onde o manifesto foi lido.
    pub library: String,
}

/// Lê um `appmanifest_<appid>.acf`. Sem appid ou sem nome, não serve.
pub fn parse_app_manifest(text: &str) -> Option<SteamApp> {
    let root = parse_vdf(text);
    let state = root.get("AppState").unwrap_or(&root);
    let app_id = state.get_str("appid")?.trim().parse::<u32>().ok()?;
    let name = state.get_str("name").unwrap_or_default().trim().to_string();
    if name.is_empty() {
        return None;
    }
    Some(SteamApp {
        app_id,
        name,
        install_dir: state.get_str("installdir").unwrap_or_default().to_string(),
        size_on_disk: state
            .get_str("SizeOnDisk")
            .and_then(|s| s.trim().parse::<u64>().ok())
            .unwrap_or(0),
        library: String::new(),
    })
}

/// Caminhos de biblioteca declarados no `libraryfolders.vdf`. O formato velho
/// guardava `"1" "D:\\Jogos"`; o novo guarda um bloco com `"path"`.
pub fn parse_library_folders(text: &str) -> Vec<String> {
    let root = parse_vdf(text);
    let list = root.get("libraryfolders").unwrap_or(&root);
    let mut out = Vec::new();
    for (key, value) in list.entries() {
        if !key.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        match value {
            Vdf::Str(p) => out.push(p.clone()),
            Vdf::Map(_) => {
                if let Some(p) = value.get_str("path") {
                    out.push(p.to_string());
                }
            }
        }
    }
    out
}

/// Onde o cliente Steam costuma estar instalado, por sistema.
pub fn steam_roots() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join(".steam").join("steam"));
        roots.push(home.join(".steam").join("root"));
        roots.push(home.join(".local").join("share").join("Steam"));
        roots.push(
            home.join(".var")
                .join("app")
                .join("com.valvesoftware.Steam")
                .join("data")
                .join("Steam"),
        );
        roots.push(
            home.join("Library")
                .join("Application Support")
                .join("Steam"),
        );
    }
    for var in ["ProgramFiles(x86)", "ProgramFiles", "ProgramW6432"] {
        if let Ok(base) = std::env::var(var) {
            roots.push(PathBuf::from(base).join("Steam"));
        }
    }
    roots.push(PathBuf::from("C:\\Program Files (x86)\\Steam"));
    roots.retain(|p| p.exists());
    roots.sort();
    roots.dedup();
    roots
}

/// Todas as pastas `steamapps` visíveis: as das raízes e as que o
/// `libraryfolders.vdf` aponta em outros discos. `extra` entra sempre.
pub fn steam_libraries(extra: &[String]) -> Vec<PathBuf> {
    fn push(p: PathBuf, libs: &mut Vec<PathBuf>) {
        let dir = if p.file_name().map(|n| n.eq_ignore_ascii_case("steamapps")) == Some(true) {
            p
        } else {
            p.join("steamapps")
        };
        if dir.is_dir() && !libs.contains(&dir) {
            libs.push(dir);
        }
    }
    let mut libs: Vec<PathBuf> = Vec::new();
    for e in extra {
        if e.trim().is_empty() {
            continue;
        }
        push(PathBuf::from(e.trim()), &mut libs);
    }
    for root in steam_roots() {
        push(root.clone(), &mut libs);
        let vdf = root.join("steamapps").join("libraryfolders.vdf");
        if let Ok(text) = std::fs::read_to_string(&vdf) {
            for p in parse_library_folders(&text) {
                push(PathBuf::from(p), &mut libs);
            }
        }
    }
    libs
}

/// Todos os jogos instalados nas bibliotecas dadas, sem repetir appid.
pub fn scan_apps(libraries: &[PathBuf]) -> Vec<SteamApp> {
    let mut out: Vec<SteamApp> = Vec::new();
    for lib in libraries {
        let Ok(rd) = std::fs::read_dir(lib) else {
            continue;
        };
        for entry in rd.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.starts_with("appmanifest_") || !name.ends_with(".acf") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            if let Some(mut app) = parse_app_manifest(&text) {
                app.library = lib.to_string_lossy().to_string();
                if !out.iter().any(|a| a.app_id == app.app_id) {
                    out.push(app);
                }
            }
        }
    }
    out.sort_by_key(|a| a.name.to_lowercase());
    out
}

/// Pastas de screenshot do cliente: `userdata/<id>/760/remote/<appid>/screenshots`.
pub fn screenshot_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for root in steam_roots() {
        let userdata = root.join("userdata");
        let Ok(users) = std::fs::read_dir(&userdata) else {
            continue;
        };
        for user in users.flatten() {
            let remote = user.path().join("760").join("remote");
            let Ok(apps) = std::fs::read_dir(&remote) else {
                continue;
            };
            for app in apps.flatten() {
                let shots = app.path().join("screenshots");
                if shots.is_dir() {
                    out.push(shots);
                }
            }
        }
    }
    out
}

/// Nome do jogo a partir do appid, procurando nos manifestos locais.
pub fn name_for_app(apps: &[SteamApp], app_id: u32) -> Option<String> {
    apps.iter()
        .find(|a| a.app_id == app_id)
        .map(|a| a.name.clone())
}

/// Appid a partir de um nome digitado. Casa exato primeiro, depois "contém".
pub fn app_for_name(apps: &[SteamApp], query: &str) -> Option<SteamApp> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return None;
    }
    if let Some(a) = apps.iter().find(|a| a.name.to_lowercase() == q) {
        return Some(a.clone());
    }
    apps.iter()
        .find(|a| a.name.to_lowercase().contains(&q))
        .cloned()
}

/// Appid escrito de qualquer jeito: número, URL da loja, `steam://`.
pub fn parse_app_id(input: &str) -> Option<u32> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }
    if s.chars().all(|c| c.is_ascii_digit()) {
        return s.parse::<u32>().ok();
    }
    let lower = s.to_lowercase();
    if !lower.contains("steam") && !lower.contains("/app/") {
        return None;
    }
    let parts: Vec<&str> = lower.split(['/', '?', '#', '&']).collect();
    for (i, part) in parts.iter().enumerate() {
        if (*part == "app" || *part == "apps") && i + 1 < parts.len() {
            let candidate = parts[i + 1];
            if candidate.chars().all(|c| c.is_ascii_digit()) && !candidate.is_empty() {
                return candidate.parse::<u32>().ok();
            }
        }
    }
    None
}

/// Caminho do `appmanifest` de um appid dentro de uma biblioteca.
pub fn manifest_path(library: &Path, app_id: u32) -> PathBuf {
    library.join(format!("appmanifest_{}.acf", app_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACF: &str = r#"
"AppState"
{
	"appid"		"570"
	"Universe"		"1"
	"name"		"Dota 2"
	"StateFlags"		"4"
	"installdir"		"dota 2 beta"
	"SizeOnDisk"		"48318382080"
	"UserConfig"
	{
		"language"		"english"
		"BetaKey"		""
	}
	"InstalledDepots"
	{
		"373301"
		{
			"manifest"		"1234567890"
		}
	}
}
"#;

    #[test]
    fn acf_gives_appid_name_and_size() {
        let app = parse_app_manifest(ACF).expect("manifesto válido");
        assert_eq!(app.app_id, 570);
        assert_eq!(app.name, "Dota 2");
        assert_eq!(app.install_dir, "dota 2 beta");
        assert_eq!(app.size_on_disk, 48_318_382_080);
    }

    #[test]
    fn acf_reads_nested_blocks_without_losing_the_top_level() {
        let root = parse_vdf(ACF);
        let state = root.get("AppState").expect("AppState");
        assert_eq!(
            state.get("UserConfig").and_then(|u| u.get_str("language")),
            Some("english")
        );
        assert_eq!(state.get_str("StateFlags"), Some("4"));
    }

    #[test]
    fn acf_ignores_comments_and_odd_casing() {
        let text = "// comentario\n\"AppState\"\n{\n\t\"AppID\" \"620\"\n\t\"Name\" \"Portal 2\"\n\t\"sizeondisk\" \"100\"\n}\n";
        let app = parse_app_manifest(text).expect("manifesto válido");
        assert_eq!(app.app_id, 620);
        assert_eq!(app.name, "Portal 2");
        assert_eq!(app.size_on_disk, 100);
    }

    #[test]
    fn acf_refuses_garbage() {
        assert!(parse_app_manifest("").is_none());
        assert!(parse_app_manifest("nao sou um manifesto").is_none());
        // Sem nome não dá para montar pasta.
        assert!(parse_app_manifest("\"AppState\"{\"appid\" \"1\"}").is_none());
        // Sem appid também não.
        assert!(parse_app_manifest("\"AppState\"{\"name\" \"X\"}").is_none());
    }

    #[test]
    fn library_folders_new_and_old_formats() {
        let novo = r#"
"libraryfolders"
{
	"0"
	{
		"path"		"C:\\Program Files (x86)\\Steam"
		"label"		""
	}
	"1"
	{
		"path"		"D:\\SteamLibrary"
	}
	"contentstatsid"		"-123"
}
"#;
        let paths = parse_library_folders(novo);
        assert_eq!(paths.len(), 2);
        assert!(paths[1].ends_with("SteamLibrary"));

        let velho =
            "\"LibraryFolders\"\n{\n\t\"TimeNextStatsReport\" \"0\"\n\t\"1\" \"D:\\\\Jogos\"\n}\n";
        assert_eq!(parse_library_folders(velho), vec!["D:\\Jogos".to_string()]);
    }

    #[test]
    fn app_id_comes_out_of_any_shape_of_input() {
        assert_eq!(parse_app_id("570"), Some(570));
        assert_eq!(parse_app_id("  292030 "), Some(292_030));
        assert_eq!(
            parse_app_id("https://store.steampowered.com/app/1091500/Cyberpunk_2077/"),
            Some(1_091_500)
        );
        assert_eq!(parse_app_id("steam://rungameid/570"), None);
        assert_eq!(parse_app_id("https://steamdb.info/app/620/"), Some(620));
        assert_eq!(parse_app_id("Hollow Knight"), None);
        assert_eq!(parse_app_id(""), None);
    }

    #[test]
    fn name_lookup_prefers_the_exact_match() {
        let apps = vec![
            SteamApp {
                app_id: 1,
                name: "Portal".into(),
                install_dir: String::new(),
                size_on_disk: 0,
                library: String::new(),
            },
            SteamApp {
                app_id: 2,
                name: "Portal 2".into(),
                install_dir: String::new(),
                size_on_disk: 0,
                library: String::new(),
            },
        ];
        assert_eq!(app_for_name(&apps, "portal").map(|a| a.app_id), Some(1));
        assert_eq!(app_for_name(&apps, "portal 2").map(|a| a.app_id), Some(2));
        assert_eq!(app_for_name(&apps, "porta").map(|a| a.app_id), Some(1));
        assert!(app_for_name(&apps, "half-life").is_none());
        assert_eq!(name_for_app(&apps, 2).as_deref(), Some("Portal 2"));
    }

    #[test]
    fn scan_reads_a_synthetic_library() {
        let dir = std::env::temp_dir().join(format!("omniget-steamlib-{}", uuid::Uuid::new_v4()));
        let lib = dir.join("steamapps");
        std::fs::create_dir_all(&lib).expect("criar biblioteca");
        std::fs::write(manifest_path(&lib, 570), ACF).expect("escrever manifesto");
        std::fs::write(lib.join("appmanifest_lixo.acf"), "nada").expect("escrever");
        std::fs::write(lib.join("outro.txt"), ACF).expect("escrever");
        let apps = scan_apps(std::slice::from_ref(&lib));
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].name, "Dota 2");
        assert_eq!(apps[0].library, lib.to_string_lossy());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
