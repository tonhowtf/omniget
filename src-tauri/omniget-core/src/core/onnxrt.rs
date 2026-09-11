//! ONNX Runtime como binário gerido, no mesmo padrão do PDFium.
//!
//! O crate `ort` está com a feature `load-dynamic`: nada de nativo entra no
//! build do OmniGet. A biblioteca do ONNX Runtime (`libonnxruntime.dylib`,
//! `onnxruntime.dll`, `libonnxruntime.so`) é baixada sob demanda dos releases
//! oficiais do `microsoft/onnxruntime` e guardada no diretório de dados do app.
//! Quem não quiser baixar aponta um arquivo que já tem — igual ao PDFium.
//!
//! A versão é fixa e casa com a API que o `ort` espera (`api-27`, ou seja
//! ONNX Runtime 1.27.x). Uma lib mais velha que isso o `ort` recusa na hora de
//! carregar, então o download vem com sha256 conferido: um arquivo truncado ou
//! trocado no meio do caminho falha aqui, não lá dentro do `dlopen`.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{anyhow, Context};
use serde::Serialize;

/// Versão do ONNX Runtime que o `ort` 2.0.0-rc.13 (`api-27`) pede.
pub const RUNTIME_VERSION: &str = "1.27.1";

const RELEASE_BASE: &str = "https://github.com/microsoft/onnxruntime/releases/download";

/// Um pacote oficial de release. `sha256` e `bytes` foram conferidos contra o
/// digest publicado pela API do GitHub em 2026-09-09.
struct Asset {
    id: &'static str,
    label: &'static str,
    file: &'static str,
    sha256: &'static str,
    bytes: u64,
    os: &'static str,
}

const ASSETS: &[Asset] = &[
    Asset {
        id: "osx-arm64",
        label: "macOS ARM64 (Apple Silicon)",
        file: "onnxruntime-osx-arm64-1.27.1.tgz",
        sha256: "e42b77a7281cc6e55141bf44fcfbac2c782b823a491bbb6ac33c781dd991f8a6",
        bytes: 31_959_937,
        os: "macos",
    },
    Asset {
        id: "linux-x64",
        label: "Linux x64",
        file: "onnxruntime-linux-x64-1.27.1.tgz",
        sha256: "25b1ef1fea1acd210d63f8f24dc870ad6e077795ce1f54876252c6d3803c15af",
        bytes: 8_828_892,
        os: "linux",
    },
    Asset {
        id: "linux-aarch64",
        label: "Linux ARM64",
        file: "onnxruntime-linux-aarch64-1.27.1.tgz",
        sha256: "33c67e33d1e25b816878366ea276589a024f71f000e7ff955c4b33224d639edd",
        bytes: 7_812_402,
        os: "linux",
    },
    Asset {
        id: "win-x64",
        label: "Windows x64",
        file: "onnxruntime-win-x64-1.27.1.zip",
        sha256: "2e00414a63fdef0914cd5a5ede6c707844878e0c08e1b6693842f0451b2df2a1",
        bytes: 77_242_362,
        os: "windows",
    },
    Asset {
        id: "win-arm64",
        label: "Windows ARM64",
        file: "onnxruntime-win-arm64-1.27.1.zip",
        sha256: "6e22c2061ba6400b42a59663d700c8694e4e8fe654cf452c4700c24237407ae1",
        bytes: 78_590_093,
        os: "windows",
    },
];

/// Nome com que a lib fica salva no disco. É o nome que o `ort` procura
/// quando não recebe caminho, então serve tanto ao `init_from` quanto a quem
/// preferir mexer na variável de ambiente.
pub fn lib_filename() -> &'static str {
    if cfg!(target_os = "windows") {
        "onnxruntime.dll"
    } else if cfg!(target_os = "macos") {
        "libonnxruntime.dylib"
    } else {
        "libonnxruntime.so"
    }
}

fn current_os() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}

/// Variante recomendada para este SO + arquitetura, ou `None` quando a
/// Microsoft não publica build oficial (macOS Intel, desde a 1.26).
pub fn auto_variant_id() -> Option<&'static str> {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        return Some("win-x64");
    }
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    {
        return Some("win-arm64");
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        return Some("osx-arm64");
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        return Some("linux-x64");
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        return Some("linux-aarch64");
    }
    #[allow(unreachable_code)]
    None
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeVariant {
    pub id: String,
    pub label: String,
    pub bytes: u64,
    pub recommended: bool,
}

/// Variantes que fazem sentido oferecer neste SO.
pub fn list_variants() -> Vec<RuntimeVariant> {
    let os = current_os();
    let auto = auto_variant_id();
    ASSETS
        .iter()
        .filter(|a| a.os == os)
        .map(|a| RuntimeVariant {
            id: a.id.to_string(),
            label: a.label.to_string(),
            bytes: a.bytes,
            recommended: Some(a.id) == auto,
        })
        .collect()
}

fn asset_by_id(id: &str) -> Option<&'static Asset> {
    ASSETS.iter().find(|a| a.id == id)
}

fn pick_asset(variant: Option<&str>) -> anyhow::Result<&'static Asset> {
    let wanted = variant
        .map(|s| s.trim())
        .filter(|s| !s.is_empty() && *s != "auto");
    if let Some(id) = wanted {
        return asset_by_id(id)
            .ok_or_else(|| anyhow!("variante de ONNX Runtime desconhecida: {id}"));
    }
    let auto = auto_variant_id().ok_or_else(|| {
        anyhow!(
            "a Microsoft não publica ONNX Runtime pronto para este sistema; \
             instale a partir de um arquivo local (pacote `onnxruntime` do pip, por exemplo)"
        )
    })?;
    asset_by_id(auto).ok_or_else(|| anyhow!("variante {auto} sumiu do catálogo"))
}

pub fn target_dir() -> Option<PathBuf> {
    crate::core::paths::app_data_dir().map(|d| d.join("onnxruntime"))
}

pub fn target_path() -> Option<PathBuf> {
    target_dir().map(|d| d.join(lib_filename()))
}

pub fn version_marker_path() -> Option<PathBuf> {
    target_dir().map(|d| d.join("onnxruntime.version"))
}

/// A lib em uso e de onde ela veio: `"env"` quando o usuário apontou
/// `ORT_DYLIB_PATH`, `"managed"` quando é a que o OmniGet baixou.
pub fn resolve_with_source() -> Option<(PathBuf, &'static str)> {
    if let Ok(raw) = std::env::var("ORT_DYLIB_PATH") {
        let p = PathBuf::from(raw.trim());
        if p.is_file() {
            return Some((p, "env"));
        }
    }
    let managed = target_path()?;
    if managed.is_file() {
        return Some((managed, "managed"));
    }
    None
}

pub fn resolve_path() -> Option<PathBuf> {
    resolve_with_source().map(|(p, _)| p)
}

pub fn is_installed() -> bool {
    resolve_with_source().is_some()
}

pub fn read_version_marker() -> Option<String> {
    let p = version_marker_path()?;
    let s = std::fs::read_to_string(&p).ok()?;
    let t = s.trim().to_string();
    if t.is_empty() {
        None
    } else {
        Some(t)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeStatus {
    pub installed: bool,
    pub path: Option<String>,
    /// "env" | "managed"
    pub source: Option<String>,
    pub version: Option<String>,
    pub target_version: String,
    pub lib_filename: String,
    /// `false` quando não há build oficial para este SO/arquitetura.
    pub can_download: bool,
    pub variants: Vec<RuntimeVariant>,
}

pub fn status() -> RuntimeStatus {
    let found = resolve_with_source();
    RuntimeStatus {
        installed: found.is_some(),
        path: found.as_ref().map(|(p, _)| p.to_string_lossy().to_string()),
        source: found.as_ref().map(|(_, s)| (*s).to_string()),
        version: read_version_marker(),
        target_version: RUNTIME_VERSION.to_string(),
        lib_filename: lib_filename().to_string(),
        can_download: auto_variant_id().is_some(),
        variants: list_variants(),
    }
}

/// Mensagem de erro que diz o que fazer, em vez de só reclamar.
fn missing_runtime_error() -> anyhow::Error {
    let where_to = target_dir()
        .map(|d| d.display().to_string())
        .unwrap_or_else(|| "<pasta de dados do app>".into());
    if auto_variant_id().is_some() {
        anyhow!(
            "o ONNX Runtime {} ainda não está instalado. Instale pela tela de Modelos \
             (o download vai para {}) ou aponte um {} que você já tenha.",
            RUNTIME_VERSION,
            where_to,
            lib_filename()
        )
    } else {
        anyhow!(
            "não existe build oficial do ONNX Runtime para este sistema. \
             Aponte um {} que você já tenha (o pacote `onnxruntime` do pip traz um) — \
             ele é copiado para {}.",
            lib_filename(),
            where_to
        )
    }
}

/// Caminho que o `ort` já está usando, quando o carregamento deu certo uma vez.
static READY: OnceLock<PathBuf> = OnceLock::new();

/// Aponta o `ort` para a lib resolvida. Idempotente: o `ort` guarda o handle
/// num `OnceLock` próprio, então a segunda chamada não troca nada — por isso
/// guardamos o caminho que venceu e devolvemos ele.
///
/// Falta de lib vira erro acionável, nunca panic: o `ort` só entra em pânico
/// se alguém tocar na API dele sem passar por aqui.
pub fn init() -> anyhow::Result<PathBuf> {
    if let Some(p) = READY.get() {
        return Ok(p.clone());
    }
    let path = resolve_path().ok_or_else(missing_runtime_error)?;
    ort::init_from(&path)
        .map_err(|e| anyhow!("não consegui carregar {}: {}", path.display(), e))?
        .with_name("omniget")
        .commit();
    let _ = READY.set(path.clone());
    Ok(path)
}

fn sha256_of(path: &Path) -> anyhow::Result<String> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path)
        .with_context(|| format!("abrindo {} para conferir o sha256", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Nome de arquivo que conta como biblioteca do ONNX Runtime neste SO.
/// Deixado público para o teste puro de seleção não precisar de rede.
pub fn is_runtime_lib(name: &str) -> bool {
    if name.starts_with('.') {
        return false;
    }
    if cfg!(target_os = "windows") {
        name.ends_with(".dll")
    } else if cfg!(target_os = "macos") {
        name.starts_with("libonnxruntime") && name.ends_with(".dylib")
    } else {
        name.starts_with("libonnxruntime.so")
    }
}

/// Só o que está dentro de `lib/` interessa; o `.dSYM` do macOS carrega um
/// arquivo com o mesmo nome e não é biblioteca carregável.
fn is_lib_entry(path: &Path) -> bool {
    let mut has_lib = false;
    for comp in path.components() {
        let s = comp.as_os_str().to_string_lossy();
        if s == "dSYM" || s.ends_with(".dSYM") {
            return false;
        }
        if s == "lib" {
            has_lib = true;
        }
    }
    if !has_lib {
        return false;
    }
    path.file_name()
        .and_then(|s| s.to_str())
        .map(is_runtime_lib)
        .unwrap_or(false)
}

fn write_atomic(dir: &Path, name: &str, bytes: &[u8]) -> anyhow::Result<PathBuf> {
    let dest = dir.join(name);
    let tmp = dir.join(format!(".{name}.tmp"));
    std::fs::write(&tmp, bytes).with_context(|| format!("gravando {}", tmp.display()))?;
    if dest.exists() {
        let _ = std::fs::remove_file(&dest);
    }
    std::fs::rename(&tmp, &dest).with_context(|| format!("movendo para {}", dest.display()))?;
    Ok(dest)
}

/// Tira do pacote as bibliotecas de `lib/` e devolve o nome da maior — que é
/// sempre o `libonnxruntime` de verdade, e não um shim de execution provider.
fn extract_libs(archive: &Path, is_zip: bool, dir: &Path) -> anyhow::Result<String> {
    let mut biggest: Option<(String, u64)> = None;
    let mut note = |name: &str, len: u64| {
        if biggest.as_ref().map(|(_, b)| len > *b).unwrap_or(true) {
            biggest = Some((name.to_string(), len));
        }
    };

    if is_zip {
        let file = std::fs::File::open(archive)
            .with_context(|| format!("abrindo {}", archive.display()))?;
        let mut zip = zip::ZipArchive::new(std::io::BufReader::new(file))?;
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i)?;
            if !entry.is_file() {
                continue;
            }
            let Some(path) = entry.enclosed_name() else {
                continue;
            };
            if !is_lib_entry(&path) {
                continue;
            }
            let Some(name) = path.file_name().and_then(|s| s.to_str()).map(String::from) else {
                continue;
            };
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf)?;
            write_atomic(dir, &name, &buf)?;
            note(&name, buf.len() as u64);
        }
    } else {
        let file = std::fs::File::open(archive)
            .with_context(|| format!("abrindo {}", archive.display()))?;
        let gz = flate2::read::GzDecoder::new(std::io::BufReader::new(file));
        let mut tar = tar::Archive::new(gz);
        for entry in tar.entries()? {
            let mut entry = entry?;
            // Symlink (`libonnxruntime.dylib` -> versionado) não tem conteúdo.
            if !entry.header().entry_type().is_file() {
                continue;
            }
            let path = entry.path()?.to_path_buf();
            if !is_lib_entry(&path) {
                continue;
            }
            let Some(name) = path.file_name().and_then(|s| s.to_str()).map(String::from) else {
                continue;
            };
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf)?;
            write_atomic(dir, &name, &buf)?;
            note(&name, buf.len() as u64);
        }
    }

    let (name, _) = biggest
        .ok_or_else(|| anyhow!("o pacote não traz nenhuma biblioteca do ONNX Runtime em lib/"))?;
    Ok(name)
}

/// Garante que o nome canônico (`libonnxruntime.dylib` etc.) existe apontando
/// para o arquivo versionado que veio no pacote.
fn make_canonical(dir: &Path, extracted: &str) -> anyhow::Result<PathBuf> {
    let canonical = dir.join(lib_filename());
    if extracted == lib_filename() {
        return Ok(canonical);
    }
    let src = dir.join(extracted);
    let tmp = dir.join(format!(".{}.tmp", lib_filename()));
    std::fs::copy(&src, &tmp)
        .with_context(|| format!("copiando {} → {}", src.display(), tmp.display()))?;
    if canonical.exists() {
        let _ = std::fs::remove_file(&canonical);
    }
    std::fs::rename(&tmp, &canonical)
        .with_context(|| format!("movendo para {}", canonical.display()))?;
    Ok(canonical)
}

/// Baixa (conferindo o sha256), extrai e deixa a lib pronta para o `init()`.
pub async fn ensure_runtime(
    variant: Option<String>,
    progress: &crate::core::tools::ProgressFn,
) -> anyhow::Result<PathBuf> {
    if let Some(p) = resolve_path() {
        return Ok(p);
    }
    install_runtime(variant, progress).await
}

/// Instala mesmo que já exista — é o "atualizar" da tela de Modelos.
pub async fn install_runtime(
    variant: Option<String>,
    progress: &crate::core::tools::ProgressFn,
) -> anyhow::Result<PathBuf> {
    let asset = pick_asset(variant.as_deref())?;
    let dir = target_dir().ok_or_else(|| anyhow!("não achei o diretório de dados do app"))?;
    std::fs::create_dir_all(&dir).with_context(|| format!("criando {}", dir.display()))?;

    let url = format!("{}/v{}/{}", RELEASE_BASE, RUNTIME_VERSION, asset.file);
    let tmp = crate::core::tools::temp_dir().join(asset.file);
    let client = crate::core::tools::client()?;
    crate::core::tools::download_to(&client, &url, &tmp, progress, "onnxruntime").await?;

    let asset_file = asset.file.to_string();
    let expected = asset.sha256.to_string();
    let is_zip = asset_file.ends_with(".zip");
    let dir_for_task = dir.clone();
    let tmp_for_task = tmp.clone();
    let p = progress.clone();
    let out = tokio::task::spawn_blocking(move || -> anyhow::Result<PathBuf> {
        crate::core::tools::report(
            &p,
            "onnxruntime",
            "verify",
            0,
            None,
            Some("conferindo sha256".into()),
        );
        let got = sha256_of(&tmp_for_task)?;
        if got != expected {
            let _ = std::fs::remove_file(&tmp_for_task);
            return Err(anyhow!(
                "o pacote do ONNX Runtime baixado não confere: esperava sha256 {expected}, veio {got}"
            ));
        }
        crate::core::tools::report(
            &p,
            "onnxruntime",
            "extract",
            0,
            None,
            Some("extraindo".into()),
        );
        let extracted = extract_libs(&tmp_for_task, is_zip, &dir_for_task)?;
        let canonical = make_canonical(&dir_for_task, &extracted)?;
        let _ = std::fs::remove_file(&tmp_for_task);
        Ok(canonical)
    })
    .await
    .map_err(|e| anyhow!("tarefa de extração falhou: {e}"))??;

    if let Some(marker) = version_marker_path() {
        let _ = std::fs::write(&marker, format!("{} ({})", RUNTIME_VERSION, asset.file));
    }
    strip_quarantine(&out).await;
    crate::core::tools::report(progress, "onnxruntime", "done", 1, Some(1), None);
    Ok(out)
}

/// Instala a partir de um arquivo que o usuário já tem no disco.
pub fn install_from_path(source: &Path) -> anyhow::Result<PathBuf> {
    if !source.is_file() {
        return Err(anyhow!("{} não é um arquivo", source.display()));
    }
    let name = source
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    if !is_runtime_lib(name) {
        return Err(anyhow!(
            "{} não parece a biblioteca do ONNX Runtime deste sistema (esperava algo como {})",
            name,
            lib_filename()
        ));
    }
    let dir = target_dir().ok_or_else(|| anyhow!("não achei o diretório de dados do app"))?;
    std::fs::create_dir_all(&dir).with_context(|| format!("criando {}", dir.display()))?;
    let bytes = std::fs::read(source).with_context(|| format!("lendo {}", source.display()))?;
    let dest = write_atomic(&dir, lib_filename(), &bytes)?;
    if let Some(marker) = version_marker_path() {
        let _ = std::fs::write(&marker, format!("local ({name})"));
    }
    Ok(dest)
}

/// Apaga a lib gerida (a apontada por env var não é nossa para mexer).
pub fn remove_managed() -> anyhow::Result<()> {
    let dir = target_dir().ok_or_else(|| anyhow!("não achei o diretório de dados do app"))?;
    if dir.is_dir() {
        std::fs::remove_dir_all(&dir).with_context(|| format!("apagando {}", dir.display()))?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
async fn strip_quarantine(path: &Path) {
    let p = path.to_path_buf();
    let _ = tokio::task::spawn_blocking(move || {
        crate::core::process::std_command("xattr")
            .args(["-d", "com.apple.quarantine"])
            .arg(&p)
            .output()
    })
    .await;
}

#[cfg(not(target_os = "macos"))]
async fn strip_quarantine(_path: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cada_asset_do_catalogo_tem_sha256_e_tamanho() {
        assert!(!ASSETS.is_empty());
        for a in ASSETS {
            assert_eq!(a.sha256.len(), 64, "sha256 de {} tem tamanho errado", a.id);
            assert!(
                a.sha256.chars().all(|c| c.is_ascii_hexdigit()),
                "sha256 de {} não é hex",
                a.id
            );
            assert!(a.bytes > 1_000_000, "{} parece pequeno demais", a.id);
            assert!(
                a.file.contains(RUNTIME_VERSION),
                "{} não é da versão fixada",
                a.file
            );
            assert!(a.file.ends_with(".tgz") || a.file.ends_with(".zip"));
        }
    }

    #[test]
    fn ids_de_variante_sao_unicos() {
        let mut ids: Vec<&str> = ASSETS.iter().map(|a| a.id).collect();
        ids.sort_unstable();
        let antes = ids.len();
        ids.dedup();
        assert_eq!(antes, ids.len(), "id de variante repetido");
    }

    #[test]
    fn a_lista_de_variantes_e_so_deste_sistema() {
        let os = current_os();
        for v in list_variants() {
            let a = asset_by_id(&v.id).expect("variante listada tem que existir no catálogo");
            assert_eq!(a.os, os);
        }
    }

    #[test]
    fn a_variante_automatica_esta_no_catalogo_e_e_a_recomendada() {
        if let Some(auto) = auto_variant_id() {
            let a = asset_by_id(auto).expect("variante automática fora do catálogo");
            assert_eq!(a.os, current_os());
            let recs: Vec<String> = list_variants()
                .into_iter()
                .filter(|v| v.recommended)
                .map(|v| v.id)
                .collect();
            assert_eq!(recs, vec![auto.to_string()]);
        }
    }

    #[test]
    fn variante_desconhecida_da_erro_em_vez_de_cair_no_automatico() {
        let e = pick_asset(Some("plan9-risc"));
        assert!(e.is_err());
    }

    #[test]
    fn auto_e_vazio_caem_na_variante_do_sistema() {
        if auto_variant_id().is_some() {
            assert_eq!(pick_asset(None).map(|a| a.id).ok(), auto_variant_id());
            assert_eq!(
                pick_asset(Some("auto")).map(|a| a.id).ok(),
                auto_variant_id()
            );
            assert_eq!(pick_asset(Some("  ")).map(|a| a.id).ok(), auto_variant_id());
        }
    }

    #[test]
    fn o_nome_canonico_conta_como_biblioteca() {
        assert!(is_runtime_lib(lib_filename()));
        assert!(!is_runtime_lib("LICENSE"));
        assert!(!is_runtime_lib("VERSION_NUMBER"));
    }

    #[test]
    fn so_a_lib_dentro_de_lib_entra_e_o_dsym_fica_de_fora() {
        let base = format!("onnxruntime-osx-arm64-{}", RUNTIME_VERSION);
        let versioned = if cfg!(target_os = "windows") {
            "onnxruntime.dll".to_string()
        } else if cfg!(target_os = "macos") {
            format!("libonnxruntime.{}.dylib", RUNTIME_VERSION)
        } else {
            format!("libonnxruntime.so.{}", RUNTIME_VERSION)
        };
        assert!(is_lib_entry(&PathBuf::from(format!(
            "{base}/lib/{versioned}"
        ))));
        assert!(!is_lib_entry(&PathBuf::from(format!(
            "{base}/lib/libonnxruntime.1.27.1.dylib.dSYM/Contents/Resources/DWARF/{versioned}"
        ))));
        assert!(!is_lib_entry(&PathBuf::from(format!("{base}/{versioned}"))));
        assert!(!is_lib_entry(&PathBuf::from(format!("{base}/lib/LICENSE"))));
    }

    #[test]
    fn a_mensagem_de_runtime_faltando_diz_o_que_fazer() {
        let msg = missing_runtime_error().to_string();
        assert!(msg.contains(lib_filename()) || msg.contains("Modelos"));
        assert!(msg.len() > 40, "mensagem curta demais para ser acionável");
    }

    /// Precisa de rede: baixa o pacote oficial e confere o sha256 fixado.
    #[test]
    #[ignore = "rede: baixa o ONNX Runtime oficial e instala no diretorio de dados"]
    fn baixa_e_instala_o_runtime_de_verdade() {
        let rt = tokio::runtime::Runtime::new().expect("runtime tokio");
        let p = crate::core::tools::noop_progress();
        let path = rt
            .block_on(install_runtime(None, &p))
            .expect("instalação do ONNX Runtime");
        assert!(path.is_file());
        assert!(is_installed());
        init().expect("carregar o ONNX Runtime recém-instalado");
    }
}
