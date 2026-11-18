use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::detect::now;
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcpLaunch {
    pub agent_id: String,
    pub name: String,
    pub version: String,
    pub distribution: String,
    pub command: String,
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
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
    pub binary_here: bool,
    pub binary_verified: bool,
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

pub fn platform_key() -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    };
    let arch = std::env::consts::ARCH;
    format!("{os}-{arch}")
}

fn agents_root() -> Result<PathBuf, String> {
    crate::core::paths::app_data_dir()
        .map(|d| d.join("agents"))
        .ok_or_else(|| format!("{ERR_ACP}: no data folder"))
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
    serde_json::from_str(body).map_err(|e| format!("{ERR_ACP}: invalid registry: {e}"))
}

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
            None => Err(format!("{ERR_ACP}: registry unavailable: {e}")),
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

pub fn cmd_path(dir: &Path, cmd: &str) -> Result<PathBuf, String> {
    let mut out = dir.to_path_buf();
    for seg in cmd.split(['/', '\\']) {
        match seg {
            "" | "." => {}
            ".." => return Err(format!("{ERR_ACP}: cmd leaves the folder: {cmd}")),
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
        .ok_or_else(|| format!("{ERR_NOT_FOUND}: agent {agent_id} is not in the registry"))?;
    let method = choose(&agent, wanted).ok_or_else(|| {
        format!(
            "{ERR_ACP}: {agent_id} has no distribution for {}",
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
                    "`{method}` is not on your PATH; install {} before opening the agent.",
                    if method == "npx" { "Node.js" } else { "uv" }
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
                format!("registering {method} {}", d.package),
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
        format!("ready: {} {}", launch.command, launch.args.join(" ")),
        RunState::Succeeded,
    );
    Ok(launch)
}

fn is_safe_agent_id(id: &str) -> bool {
    !id.is_empty() && id != "." && !id.contains("..") && !id.contains(['/', '\\', ':', '\0'])
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
        .ok_or_else(|| format!("{ERR_ACP}: no binary for {plat}"))?;
    if bin.sha256.is_none() && !allow_unverified {
        return Err(format!(
            "{ERR_UNVERIFIED}: the registry does not publish a sha256 for {} ({plat}); confirm to install without verification",
            agent.id
        ));
    }
    if !is_safe_agent_id(&agent.id) {
        return Err(format!("{ERR_ACP}: invalid agent id in the registry"));
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
        format!("downloading {}", bin.archive),
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
            "{ERR_ACP}: HTTP {} at {}",
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
        format!("{} bytes downloaded", bytes.len()),
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
                "sha256 matches",
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
                format!("no sha256 in the registry; computed {actual}"),
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
        format!("extracted to {}", dir.display()),
        RunState::Running,
    );

    let exe = cmd_path(&dir, &bin.cmd)?;
    if !exe.exists() {
        return Err(format!(
            "{ERR_ACP}: {} did not appear after extraction",
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
        warnings.push(
            "Installed without sha256 verification (the registry does not publish one).".into(),
        );
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

pub fn extract(bytes: &[u8], name: &str, dir: &Path) -> Result<(), String> {
    let lower = name.to_ascii_lowercase();
    let err = |e: std::io::Error| format!("{ERR_ACP}: extraction: {e}");
    if lower.ends_with(".zip") {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
            .map_err(|e| format!("{ERR_ACP}: zip: {e}"))?;
        for i in 0..zip.len() {
            let mut f = zip
                .by_index(i)
                .map_err(|e| format!("{ERR_ACP}: zip: {e}"))?;
            let Some(rel) = f.enclosed_name() else {
                return Err(format!("{ERR_ACP}: unsafe entry in the zip: {}", f.name()));
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
            return Err(format!("{ERR_ACP}: tar -xjf failed ({status})"));
        }
        return Ok(());
    }
    let out = dir.join(name);
    std::fs::write(&out, bytes).map_err(err)?;
    Ok(())
}

#[cfg(test)]
mod agent_id_tests {
    use super::is_safe_agent_id;

    #[test]
    fn a_plain_registry_id_is_accepted() {
        assert!(is_safe_agent_id("claude-code-acp"));
        assert!(is_safe_agent_id("gemini.cli_2"));
    }

    #[test]
    fn an_id_that_escapes_the_agents_folder_is_rejected() {
        for id in [
            "",
            ".",
            "..",
            "../../Documents",
            "a/b",
            "a\\b",
            "/etc",
            "C:evil",
            "a/../b",
        ] {
            assert!(!is_safe_agent_id(id), "{id:?}");
        }
    }
}
