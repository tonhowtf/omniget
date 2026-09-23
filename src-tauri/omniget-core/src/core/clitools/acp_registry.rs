//! Registro ACP (`cdn.agentclientprotocol.com/registry/v1/latest/registry.json`):
//! listar agentes e instalar pela distribuição declarada.
//!
//! - `npx`/`uvx`: nada é baixado; o comando é registrado e devolvido.
//! - `binary`: o arquivo da plataforma é baixado para
//!   `<app_data>/agents/bin/<id>/<versão>/`, conferido pelo sha256 do registro
//!   (fail-closed, padrão do `dependencies.rs`) e extraído. Entrada sem sha256
//!   só instala com `allow_unverified`.
//!
//! O resultado é o comando ACP pronto (`AcpLaunch`) para o driver ACP.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::exec;
use super::run::{Progress, ProgressFn, RunState};
use super::table;
use super::{ERR_ACP, ERR_NOT_FOUND, ERR_UNVERIFIED};
use crate::core::dependencies::integrity;

pub const REGISTRY_URL: &str =
    "https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json";
pub const REGISTRY_TTL: Duration = Duration::from_secs(3600);

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PkgDist {
    pub package: String,
    #[serde(default)]
    pub args: Option<Vec<String>>,
    #[serde(default)]
    pub env: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BinDist {
    pub archive: String,
    pub cmd: String,
    #[serde(default)]
    pub args: Option<Vec<String>>,
    #[serde(default)]
    pub env: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Distribution {
    #[serde(default)]
    pub npx: Option<PkgDist>,
    #[serde(default)]
    pub uvx: Option<PkgDist>,
    #[serde(default)]
    pub binary: Option<BTreeMap<String, BinDist>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryAgent {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub website: Option<String>,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub distribution: Distribution,
}

#[derive(Debug, Clone, Deserialize)]
struct RegistryFile {
    #[serde(default)]
    version: Option<String>,
    agents: Vec<RegistryAgent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheFile {
    fetched_at: u64,
    body: String,
}

/// Comando ACP pronto para o driver.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcpLaunch {
    pub agent_id: String,
    pub name: String,
    pub version: String,
    /// `npx`, `uvx` ou `binary`.
    pub distribution: String,
    pub command: String,
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Pasta da instalação (só `binary`).
    #[serde(default)]
    pub dir: Option<PathBuf>,
    #[serde(default)]
    pub sha256: Option<String>,
    pub sha256_verified: bool,
    pub installed_at: u64,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentView {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub repository: Option<String>,
    pub website: Option<String>,
    pub license: Option<String>,
    pub icon: Option<String>,
    pub methods: Vec<String>,
    /// Há binário para esta plataforma.
    pub binary_here: bool,
    /// O binário desta plataforma tem sha256 no registro.
    pub binary_verified: bool,
    /// Ferramenta da tabela que aponta para este agente.
    pub tool_id: Option<String>,
    pub installed: Option<AcpLaunch>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RegistryView {
    pub version: Option<String>,
    pub fetched_at: u64,
    pub stale: bool,
    pub error: Option<String>,
    pub platform: String,
    pub agents: Vec<AgentView>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Chave de plataforma do registro (`darwin-aarch64`, `windows-x86_64`…).
pub fn platform_key() -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "aarch64",
        "x86_64" => "x86_64",
        other => other,
    };
    format!("{os}-{arch}")
}

fn agents_root() -> Result<PathBuf, String> {
    crate::core::paths::app_data_dir()
        .map(|d| d.join("agents"))
        .ok_or_else(|| format!("{ERR_ACP}: sem pasta de dados"))
}

fn cache_path() -> Option<PathBuf> {
    Some(
        crate::core::paths::app_data_dir()?
            .join("clitools")
            .join("acp-registry.json"),
    )
}

fn installed_path() -> Result<PathBuf, String> {
    Ok(agents_root()?.join("acp-installed.json"))
}

pub fn installed() -> BTreeMap<String, AcpLaunch> {
    installed_path()
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_installed(map: &BTreeMap<String, AcpLaunch>) -> Result<(), String> {
    let p = installed_path()?;
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("{ERR_ACP}: {e}"))?;
    }
    let text = serde_json::to_string_pretty(map).map_err(|e| e.to_string())?;
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("{ERR_ACP}: {e}"))?;
    std::fs::rename(&tmp, &p).map_err(|e| format!("{ERR_ACP}: {e}"))
}

fn parse(body: &str) -> Result<RegistryFile, String> {
    serde_json::from_str(body).map_err(|e| format!("{ERR_ACP}: registro inválido: {e}"))
}

/// Registro cru, com cache de 1 h (em disco). Sem rede, usa o último.
async fn load(force: bool) -> Result<(RegistryFile, u64, bool, Option<String>), String> {
    let cached: Option<CacheFile> = cache_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok());
    if let (false, Some(c)) = (force, &cached) {
        if now().saturating_sub(c.fetched_at) < REGISTRY_TTL.as_secs() {
            if let Ok(r) = parse(&c.body) {
                return Ok((r, c.fetched_at, false, None));
            }
        }
    }
    let fetched = async {
        let client = crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
            .timeout(Duration::from_secs(15))
            .user_agent("OmniGet-clitools")
            .build()
            .map_err(|e| e.to_string())?;
        let resp = client
            .get(REGISTRY_URL)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status()));
        }
        resp.text().await.map_err(|e| e.to_string())
    }
    .await;
    match fetched {
        Ok(body) => {
            let reg = parse(&body)?;
            let at = now();
            if let Some(p) = cache_path() {
                if let Some(d) = p.parent() {
                    let _ = std::fs::create_dir_all(d);
                }
                if let Ok(t) = serde_json::to_string(&CacheFile {
                    fetched_at: at,
                    body,
                }) {
                    let _ = std::fs::write(p, t);
                }
            }
            Ok((reg, at, false, None))
        }
        Err(e) => match cached {
            Some(c) => Ok((parse(&c.body)?, c.fetched_at, true, Some(e))),
            None => Err(format!("{ERR_ACP}: registro indisponível: {e}")),
        },
    }
}

fn view(agent: &RegistryAgent, inst: &BTreeMap<String, AcpLaunch>) -> AgentView {
    let d = &agent.distribution;
    let mut methods = Vec::new();
    if d.npx.is_some() {
        methods.push("npx".to_string());
    }
    if d.uvx.is_some() {
        methods.push("uvx".to_string());
    }
    if d.binary.is_some() {
        methods.push("binary".to_string());
    }
    let here = d.binary.as_ref().and_then(|b| b.get(&platform_key()));
    AgentView {
        id: agent.id.clone(),
        name: agent.name.clone(),
        version: agent.version.clone(),
        description: agent.description.clone(),
        repository: agent.repository.clone(),
        website: agent.website.clone(),
        license: agent.license.clone(),
        icon: agent.icon.clone(),
        methods,
        binary_here: here.is_some(),
        binary_verified: here.and_then(|b| b.sha256.as_ref()).is_some(),
        tool_id: table::all()
            .iter()
            .find(|t| t.acp.registry_id.as_deref() == Some(agent.id.as_str()))
            .map(|t| t.id.clone()),
        installed: inst.get(&agent.id).cloned(),
    }
}

pub async fn registry(force: bool) -> Result<RegistryView, String> {
    let (reg, at, stale, error) = load(force).await?;
    let inst = installed();
    Ok(RegistryView {
        version: reg.version.clone(),
        fetched_at: at,
        stale,
        error,
        platform: platform_key(),
        agents: reg.agents.iter().map(|a| view(a, &inst)).collect(),
    })
}

/// Escolha da distribuição: pedida, senão binário verificado → npx → uvx →
/// binário sem sha256.
pub fn choose(agent: &RegistryAgent, wanted: Option<&str>) -> Option<&'static str> {
    let d = &agent.distribution;
    let here = d.binary.as_ref().and_then(|b| b.get(&platform_key()));
    match wanted {
        Some("npx") => return d.npx.as_ref().map(|_| "npx"),
        Some("uvx") => return d.uvx.as_ref().map(|_| "uvx"),
        Some("binary") => return here.map(|_| "binary"),
        _ => {}
    }
    if here.and_then(|b| b.sha256.as_ref()).is_some() {
        Some("binary")
    } else if d.npx.is_some() {
        Some("npx")
    } else if d.uvx.is_some() {
        Some("uvx")
    } else if here.is_some() {
        Some("binary")
    } else {
        None
    }
}

/// Caminho do executável dentro da pasta extraída (`./bin/x`, `.\\a\\b.exe`).
pub fn cmd_path(dir: &Path, cmd: &str) -> Result<PathBuf, String> {
    let mut out = dir.to_path_buf();
    for seg in cmd.split(['/', '\\']) {
        match seg {
            "" | "." => {}
            ".." => return Err(format!("{ERR_ACP}: cmd sai da pasta: {cmd}")),
            s => out.push(s),
        }
    }
    Ok(out)
}

fn emit(
    on: &ProgressFn,
    run_id: &str,
    agent: &str,
    kind: &str,
    line: impl Into<String>,
    state: RunState,
) {
    on(Progress {
        run_id: run_id.to_string(),
        tool_id: format!("acp:{agent}"),
        kind: kind.into(),
        stream: Some("stdout".into()),
        line: Some(line.into()),
        state,
        command: None,
    });
}

pub async fn install(
    agent_id: &str,
    wanted: Option<&str>,
    allow_unverified: bool,
    on: ProgressFn,
) -> Result<AcpLaunch, String> {
    let (reg, _, _, _) = load(false).await?;
    let agent = reg
        .agents
        .iter()
        .find(|a| a.id == agent_id)
        .cloned()
        .ok_or_else(|| format!("{ERR_NOT_FOUND}: agente {agent_id} não está no registro"))?;
    let method = choose(&agent, wanted).ok_or_else(|| {
        format!(
            "{ERR_ACP}: {agent_id} não tem distribuição para {}",
            platform_key()
        )
    })?;
    let run_id = uuid::Uuid::new_v4().to_string();
    let mut warnings = Vec::new();

    let launch = match method {
        "npx" | "uvx" => {
            let d = if method == "npx" {
                agent.distribution.npx.clone()
            } else {
                agent.distribution.uvx.clone()
            }
            .unwrap_or_default();
            if exec::which(method).is_none() {
                warnings.push(format!(
                    "`{method}` não está no PATH; instale {} antes de abrir o agente.",
                    if method == "npx" { "o Node.js" } else { "o uv" }
                ));
            }
            let mut args = Vec::new();
            if method == "npx" {
                args.push("-y".to_string());
            }
            args.push(d.package.clone());
            args.extend(d.args.clone().unwrap_or_default());
            emit(
                &on,
                &run_id,
                agent_id,
                "start",
                format!("registrando {method} {}", d.package),
                RunState::Running,
            );
            AcpLaunch {
                agent_id: agent.id.clone(),
                name: agent.name.clone(),
                version: agent.version.clone(),
                distribution: method.into(),
                command: method.into(),
                args,
                env: d.env.clone().unwrap_or_default(),
                dir: None,
                sha256: None,
                sha256_verified: false,
                installed_at: now(),
                warnings: warnings.clone(),
            }
        }
        _ => install_binary(&agent, allow_unverified, &run_id, &on).await?,
    };

    let mut map = installed();
    // Remove versões antigas do mesmo agente (só as que o OmniGet baixou).
    if let (Some(old), Some(new_dir)) = (map.get(agent_id), &launch.dir) {
        if let Some(old_dir) = &old.dir {
            if old_dir != new_dir && old_dir.starts_with(agents_root()?.join("bin")) {
                let _ = std::fs::remove_dir_all(old_dir);
            }
        }
    }
    map.insert(agent_id.to_string(), launch.clone());
    save_installed(&map)?;
    emit(
        &on,
        &run_id,
        agent_id,
        "end",
        format!("pronto: {} {}", launch.command, launch.args.join(" ")),
        RunState::Succeeded,
    );
    Ok(launch)
}

async fn install_binary(
    agent: &RegistryAgent,
    allow_unverified: bool,
    run_id: &str,
    on: &ProgressFn,
) -> Result<AcpLaunch, String> {
    let plat = platform_key();
    let bin = agent
        .distribution
        .binary
        .as_ref()
        .and_then(|b| b.get(&plat))
        .cloned()
        .ok_or_else(|| format!("{ERR_ACP}: sem binário para {plat}"))?;
    if bin.sha256.is_none() && !allow_unverified {
        return Err(format!(
            "{ERR_UNVERIFIED}: o registro não publica sha256 para {} ({plat}); confirme para instalar sem verificação",
            agent.id
        ));
    }
    let safe_version: String = agent
        .version
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || ".-_+".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    let dir = agents_root()?
        .join("bin")
        .join(&agent.id)
        .join(&safe_version);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{ERR_ACP}: {e}"))?;

    emit(
        on,
        run_id,
        &agent.id,
        "start",
        format!("baixando {}", bin.archive),
        RunState::Running,
    );
    let client = crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
        .timeout(Duration::from_secs(600))
        .user_agent("OmniGet-clitools")
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(&bin.archive)
        .send()
        .await
        .map_err(|e| format!("{ERR_ACP}: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!(
            "{ERR_ACP}: HTTP {} em {}",
            resp.status(),
            bin.archive
        ));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("{ERR_ACP}: {e}"))?
        .to_vec();
    emit(
        on,
        run_id,
        &agent.id,
        "line",
        format!("{} bytes baixados", bytes.len()),
        RunState::Running,
    );

    let actual = integrity::sha256_hex(&bytes);
    let verified = match &bin.sha256 {
        Some(expected) => {
            integrity::verify_sha256(&bytes, expected, &agent.id).map_err(|e| {
                let _ = std::fs::remove_dir_all(&dir);
                format!("{ERR_UNVERIFIED}: {e}")
            })?;
            emit(
                on,
                run_id,
                &agent.id,
                "line",
                "sha256 confere",
                RunState::Running,
            );
            true
        }
        None => {
            emit(
                on,
                run_id,
                &agent.id,
                "line",
                format!("sem sha256 no registro; calculado {actual}"),
                RunState::Running,
            );
            false
        }
    };

    let name = bin
        .archive
        .rsplit('/')
        .next()
        .unwrap_or("download")
        .split('?')
        .next()
        .unwrap_or("download")
        .to_string();
    let dir2 = dir.clone();
    let name2 = name.clone();
    tokio::task::spawn_blocking(move || extract(&bytes, &name2, &dir2))
        .await
        .map_err(|e| format!("{ERR_ACP}: {e}"))??;
    emit(
        on,
        run_id,
        &agent.id,
        "line",
        format!("extraído em {}", dir.display()),
        RunState::Running,
    );

    let exe = cmd_path(&dir, &bin.cmd)?;
    if !exe.exists() {
        return Err(format!(
            "{ERR_ACP}: {} não apareceu depois da extração",
            exe.display()
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755));
    }
    let mut warnings = Vec::new();
    if !verified {
        warnings.push("Instalado sem verificação de sha256 (o registro não publica).".into());
    }
    Ok(AcpLaunch {
        agent_id: agent.id.clone(),
        name: agent.name.clone(),
        version: agent.version.clone(),
        distribution: "binary".into(),
        command: exe.to_string_lossy().into_owned(),
        args: bin.args.clone().unwrap_or_default(),
        env: bin.env.clone().unwrap_or_default(),
        dir: Some(dir),
        sha256: Some(actual),
        sha256_verified: verified,
        installed_at: now(),
        warnings,
    })
}

/// Extrai por extensão. Entradas que escapariam da pasta são recusadas.
pub fn extract(bytes: &[u8], name: &str, dir: &Path) -> Result<(), String> {
    let lower = name.to_ascii_lowercase();
    let err = |e: std::io::Error| format!("{ERR_ACP}: extração: {e}");
    if lower.ends_with(".zip") {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
            .map_err(|e| format!("{ERR_ACP}: zip: {e}"))?;
        for i in 0..zip.len() {
            let mut f = zip
                .by_index(i)
                .map_err(|e| format!("{ERR_ACP}: zip: {e}"))?;
            let Some(rel) = f.enclosed_name() else {
                return Err(format!("{ERR_ACP}: entrada insegura no zip: {}", f.name()));
            };
            let out = dir.join(rel);
            if f.is_dir() {
                std::fs::create_dir_all(&out).map_err(err)?;
                continue;
            }
            if let Some(p) = out.parent() {
                std::fs::create_dir_all(p).map_err(err)?;
            }
            let mut buf = Vec::new();
            f.read_to_end(&mut buf).map_err(err)?;
            std::fs::write(&out, buf).map_err(err)?;
            #[cfg(unix)]
            if let Some(mode) = f.unix_mode() {
                use std::os::unix::fs::PermissionsExt;
                let _ =
                    std::fs::set_permissions(&out, std::fs::Permissions::from_mode(mode & 0o777));
            }
        }
        return Ok(());
    }
    let unpack = |reader: Box<dyn Read>| -> Result<(), String> {
        let mut ar = tar::Archive::new(reader);
        ar.set_preserve_permissions(true);
        for entry in ar.entries().map_err(err)? {
            let mut entry = entry.map_err(err)?;
            // `unpack_in` recusa caminhos que saem de `dir`.
            entry.unpack_in(dir).map_err(err)?;
        }
        Ok(())
    };
    if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        return unpack(Box::new(flate2::read::GzDecoder::new(
            std::io::Cursor::new(bytes.to_vec()),
        )));
    }
    if lower.ends_with(".tar.xz") || lower.ends_with(".txz") {
        return unpack(Box::new(xz2::read::XzDecoder::new(std::io::Cursor::new(
            bytes.to_vec(),
        ))));
    }
    if lower.ends_with(".tar") {
        return unpack(Box::new(std::io::Cursor::new(bytes.to_vec())));
    }
    if lower.ends_with(".tar.bz2") || lower.ends_with(".tbz2") {
        // Sem codec bzip2 no crate: o `tar` do sistema (macOS, Linux e
        // Windows 10+) descompacta.
        let tmp = dir.join(".archive.tar.bz2");
        std::fs::write(&tmp, bytes).map_err(err)?;
        let status = crate::core::process::std_command("tar")
            .arg("-xjf")
            .arg(&tmp)
            .arg("-C")
            .arg(dir)
            .status()
            .map_err(err)?;
        let _ = std::fs::remove_file(&tmp);
        if !status.success() {
            return Err(format!("{ERR_ACP}: tar -xjf falhou ({status})"));
        }
        return Ok(());
    }
    // Binário cru.
    let out = dir.join(name);
    std::fs::write(&out, bytes).map_err(err)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(json: &str) -> RegistryAgent {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn registro_real_parseia() {
        let body = r#"{"version":"1.0.0","agents":[{"id":"goose","name":"goose","version":"1.51.0","distribution":{"binary":{"darwin-aarch64":{"archive":"https://x/goose.tar.bz2","cmd":"./goose","args":["acp"],"sha256":"ab"}}}},{"id":"gemini","name":"Gemini CLI","version":"0.60.0","distribution":{"npx":{"package":"@google/gemini-cli@0.60.0","args":["--acp"]}}}],"extensions":[]}"#;
        let r = parse(body).unwrap();
        assert_eq!(r.agents.len(), 2);
        assert_eq!(
            r.agents[1]
                .distribution
                .npx
                .as_ref()
                .unwrap()
                .args
                .as_ref()
                .unwrap()[0],
            "--acp"
        );
    }

    #[test]
    fn escolha_prefere_binario_verificado_depois_npx() {
        let plat = platform_key();
        let a = agent(&format!(
            r#"{{"id":"k","name":"k","version":"1","distribution":{{"npx":{{"package":"k@1"}},"binary":{{"{plat}":{{"archive":"u","cmd":"./k","sha256":"x"}}}}}}}}"#
        ));
        assert_eq!(choose(&a, None), Some("binary"));
        assert_eq!(choose(&a, Some("npx")), Some("npx"));
        let b = agent(&format!(
            r#"{{"id":"k","name":"k","version":"1","distribution":{{"npx":{{"package":"k@1"}},"binary":{{"{plat}":{{"archive":"u","cmd":"./k"}}}}}}}}"#
        ));
        assert_eq!(choose(&b, None), Some("npx"));
        let c = agent(r#"{"id":"k","name":"k","version":"1","distribution":{}}"#);
        assert_eq!(choose(&c, None), None);
    }

    #[test]
    fn cmd_path_normaliza_e_recusa_saida() {
        let d = Path::new("/a");
        assert_eq!(
            cmd_path(d, "./bin/devin").unwrap(),
            Path::new("/a").join("bin").join("devin")
        );
        assert_eq!(
            cmd_path(d, "./goose-package\\goose.exe").unwrap(),
            Path::new("/a").join("goose-package").join("goose.exe")
        );
        assert!(cmd_path(d, "../x").is_err());
    }

    #[test]
    fn extrai_tar_gz_e_zip() {
        let tmp = std::env::temp_dir().join(format!("omniget-acp-x-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        // tar.gz com ./bin/agent
        let mut tar_bytes = Vec::new();
        {
            let gz = flate2::write::GzEncoder::new(&mut tar_bytes, flate2::Compression::fast());
            let mut b = tar::Builder::new(gz);
            let data = b"#!/bin/sh\necho ok\n";
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o755);
            h.set_cksum();
            b.append_data(&mut h, "bin/agent", &data[..]).unwrap();
            b.into_inner().unwrap().finish().unwrap();
        }
        extract(&tar_bytes, "a.tar.gz", &tmp).unwrap();
        assert!(tmp.join("bin").join("agent").exists());
        // zip com x/y.txt
        let mut zbytes = Vec::new();
        {
            let mut z = zip::ZipWriter::new(std::io::Cursor::new(&mut zbytes));
            z.start_file("x/y.txt", zip::write::SimpleFileOptions::default())
                .unwrap();
            std::io::Write::write_all(&mut z, b"oi").unwrap();
            z.finish().unwrap();
        }
        extract(&zbytes, "a.zip", &tmp).unwrap();
        assert_eq!(std::fs::read(tmp.join("x").join("y.txt")).unwrap(), b"oi");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
