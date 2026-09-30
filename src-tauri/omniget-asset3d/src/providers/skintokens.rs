//! skin-tokens.cpp (Apache-2.0 port of VAST-AI SkinTokens, MIT weights) as
//! an optional child process: `skintokens-cli skin MODEL mesh skeleton out`
//! with our standard VRM humanoid skeleton (estudo 05: `skin` with a known
//! skeleton is the robust path; free `rig` is experimental upstream).
//!
//! Binary and weights are downloaded on request only. Upstream has no
//! prebuilt binary, so `.github/workflows/skintokens.yml` builds one per
//! platform (with the ggml libraries next to it) into our own release; see
//! [`CLI_RELEASE`].

use super::async_trait_free::BoxFuture;
use super::Rigger;
use crate::model::*;
use crate::write_glb::{self, WriteOptions};
use sha2::{Digest, Sha256};
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const WEIGHTS_BASE: &str = "https://huggingface.co/LocalAI-io/SkinTokens-GGUF/resolve/main/F16";

/// F16 bundle (1.25 GB), sizes and sha256 from the Hugging Face LFS index.
pub const WEIGHTS: &[(&str, u64, &str)] = &[
    (
        "mesh-encoder.gguf",
        57_813_184,
        "532710809e3db6c54389dd6489c2aa3c768244868b9c197198d06fa664214527",
    ),
    (
        "skin-vae.gguf",
        244_014_784,
        "dcd5859ac89bae62bcfd6b7823f9d0e2393b22364bcb19f84c1109af8e07455e",
    ),
    (
        "tokenrig.gguf",
        948_562_656,
        "933529dc0e550fd499e2e1ae1c574c6d5852047c04832e7207304e184901ed82",
    ),
];

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct SkinTokensConfig {
    /// `skintokens-cli` (its ggml backend libraries next to it).
    pub cli: PathBuf,
    /// Folder with the three F16 `.gguf` files.
    pub model_dir: PathBuf,
    /// `auto`, `cpu` or `vulkan` (Metal is picked by `auto` on macOS).
    pub device: String,
    /// Scratch folder for the mesh, skeleton, output and log.
    pub work_dir: PathBuf,
    pub timeout_secs: u64,
}

impl SkinTokensConfig {
    pub fn under(root: &Path) -> Self {
        let exe = if cfg!(windows) {
            "skintokens-cli.exe"
        } else {
            "skintokens-cli"
        };
        SkinTokensConfig {
            cli: root.join("bin").join(exe),
            model_dir: root.join("F16"),
            device: "auto".into(),
            work_dir: root.join("work"),
            // 48 s for a 4.7k-triangle character with --beams 1 on an M5.
            timeout_secs: 600,
        }
    }

    pub fn is_ready(&self) -> bool {
        self.cli.is_file()
            && WEIGHTS.iter().all(|(f, size, _)| {
                std::fs::metadata(self.model_dir.join(f))
                    .map(|m| m.len() == *size)
                    .unwrap_or(false)
            })
    }
}

async fn download_verified(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    sha256: Option<&str>,
    progress: &(dyn Fn(u64, Option<u64>) + Send + Sync),
) -> Result<(), String> {
    let tmp = dest.with_extension("part");
    if let Some(p) = dest.parent() {
        std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
    }
    let mut resp = client.get(url).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("download {url}: HTTP {}", resp.status()));
    }
    let total = resp.content_length();
    let mut file = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut done = 0u64;
    while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
        use std::io::Write;
        file.write_all(&chunk).map_err(|e| e.to_string())?;
        hasher.update(&chunk);
        done += chunk.len() as u64;
        progress(done, total);
    }
    drop(file);
    let got = hex::encode(hasher.finalize());
    if let Some(want) = sha256 {
        if !got.eq_ignore_ascii_case(want) {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("download {url}: sha256 mismatch"));
        }
    }
    std::fs::rename(&tmp, dest).map_err(|e| e.to_string())
}

/// Downloads the F16 weights that are missing (1.25 GB in total).
pub async fn download_weights(
    cfg: &SkinTokensConfig,
    progress: &(dyn Fn(&str, u64, Option<u64>) + Send + Sync),
) -> Result<(), String> {
    let client = reqwest::Client::new();
    for (file, size, sha) in WEIGHTS {
        let dest = cfg.model_dir.join(file);
        if std::fs::metadata(&dest)
            .map(|m| m.len() == *size)
            .unwrap_or(false)
        {
            continue;
        }
        let url = format!("{WEIGHTS_BASE}/{file}");
        download_verified(&client, &url, &dest, Some(sha), &|d, t| {
            progress(file, d, t)
        })
        .await?;
    }
    Ok(())
}

/// Release made by `.github/workflows/skintokens.yml` from the commit pinned
/// in `scripts/skintokens/build.sh`: tag `skintokens-<short commit>`, one
/// flat `skintokens-cli-<platform>.tar.gz` per platform.
pub const CLI_RELEASE: &str = "skintokens-37b2828";
pub const CLI_RELEASE_BASE: &str = "https://github.com/tonhowtf/omniget/releases/download";

/// `(platform, sha256)` of each package in [`CLI_RELEASE`], copied from the
/// release's `SHA256SUMS`. An empty hash means the package is not published
/// yet, and the install refuses it (nothing runs unverified).
pub const CLI_PACKAGES: &[(&str, &str)] =
    &[("macos-arm64", ""), ("linux-x64", ""), ("windows-x64", "")];

/// The package name of the running platform, if the workflow builds one.
pub fn cli_platform() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("macos-arm64"),
        ("linux", "x86_64") => Some("linux-x64"),
        ("windows", "x86_64") => Some("windows-x64"),
        _ => None,
    }
}

/// URL and sha256 of the package for this platform, or why there is none.
pub fn cli_package() -> Result<(String, &'static str), String> {
    let platform = cli_platform().ok_or_else(|| {
        format!(
            "no skin-tokens.cpp package for {}-{}; point to a built skintokens-cli instead",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })?;
    let sha = CLI_PACKAGES
        .iter()
        .find(|(p, _)| *p == platform)
        .map(|(_, s)| *s)
        .unwrap_or("");
    if sha.is_empty() {
        return Err(format!(
            "the skin-tokens.cpp package for {platform} is not published yet; point to a built skintokens-cli instead"
        ));
    }
    Ok((
        format!("{CLI_RELEASE_BASE}/{CLI_RELEASE}/skintokens-cli-{platform}.tar.gz"),
        sha,
    ))
}

fn is_archive(url: &str) -> bool {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    path.ends_with(".tar.gz") || path.ends_with(".tgz")
}

/// Downloads the CLI, checked by sha256 when one is given. A raw executable
/// lands at `cfg.cli`; a `.tar.gz` package (the binary with its ggml
/// libraries) is extracted into the folder of `cfg.cli`, replacing it only
/// once the whole package is in place. Marks the binary executable on Unix.
pub async fn download_cli(
    cfg: &SkinTokensConfig,
    url: &str,
    sha256: Option<&str>,
    progress: &(dyn Fn(u64, Option<u64>) + Send + Sync),
) -> Result<(), String> {
    let client = reqwest::Client::new();
    if is_archive(url) {
        let bin = cfg.cli.parent().ok_or("CLI path has no folder")?;
        let root = bin.parent().ok_or("CLI folder has no parent")?;
        let archive = root.join("skintokens-cli.tar.gz");
        download_verified(&client, url, &archive, sha256, progress).await?;
        let exe = cfg
            .cli
            .file_name()
            .ok_or("CLI path has no file name")?
            .to_owned();
        let bin = bin.to_path_buf();
        let archive2 = archive.clone();
        let res = tokio::task::spawn_blocking(move || install_cli_archive(&archive2, &bin, &exe))
            .await
            .map_err(|e| e.to_string())?;
        let _ = std::fs::remove_file(&archive);
        return res;
    }
    download_verified(&client, url, &cfg.cli, sha256, progress).await?;
    mark_executable(&cfg.cli);
    Ok(())
}

fn mark_executable(_path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(_path, std::fs::Permissions::from_mode(0o755));
    }
}

/// Extracts a flat `.tar.gz` package into `bin_dir` (a sibling `.new`
/// folder first, then swapped in). Only plain file names are accepted:
/// no folders, no `..`, and symlinks only to another name in the package
/// (the versioned `.dylib`/`.so` links).
pub fn install_cli_archive(archive: &Path, bin_dir: &Path, exe_name: &OsStr) -> Result<(), String> {
    let name = bin_dir.file_name().ok_or("bad bin folder")?;
    let parent = bin_dir.parent().ok_or("bad bin folder")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut staging_name = name.to_owned();
    staging_name.push(".new");
    let staging = parent.join(staging_name);
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
    let result = extract_flat(archive, &staging).and_then(|()| {
        let exe = staging.join(exe_name);
        if !exe.is_file() {
            return Err(format!(
                "the package has no {}",
                Path::new(exe_name).display()
            ));
        }
        mark_executable(&exe);
        Ok(())
    });
    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e);
    }
    if bin_dir.exists() {
        std::fs::remove_dir_all(bin_dir).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&staging, bin_dir).map_err(|e| e.to_string())
}

fn plain_name(p: &Path) -> Option<String> {
    let mut comps = p.components().filter(|c| !matches!(c, Component::CurDir));
    match (comps.next(), comps.next()) {
        (Some(Component::Normal(n)), None) => n.to_str().map(str::to_string),
        _ => None,
    }
}

fn extract_flat(archive: &Path, dest: &Path) -> Result<(), String> {
    let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
    tar.set_preserve_permissions(true);
    let mut links: Vec<(String, String)> = vec![];
    let mut count = 0usize;
    for entry in tar.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path().map_err(|e| e.to_string())?.into_owned();
        let kind = entry.header().entry_type();
        if kind.is_dir() && plain_name(&path).is_none() {
            // `./` at the top of the archive.
            continue;
        }
        let name = plain_name(&path)
            .ok_or_else(|| format!("unexpected path in the package: {}", path.display()))?;
        if kind.is_symlink() {
            let target = entry
                .link_name()
                .map_err(|e| e.to_string())?
                .and_then(|t| plain_name(&t))
                .ok_or_else(|| format!("unexpected link in the package: {name}"))?;
            links.push((name, target));
        } else if kind.is_file() {
            entry
                .unpack(dest.join(&name))
                .map_err(|e| format!("{name}: {e}"))?;
            count += 1;
        } else {
            return Err(format!("unexpected entry in the package: {name}"));
        }
    }
    if count == 0 {
        return Err("the package is empty".into());
    }
    // Links as copies: they work the same for the loader and on Windows.
    // Chains (`libggml.dylib -> libggml.0.dylib -> libggml.0.20.2.dylib`)
    // resolve over several passes.
    while !links.is_empty() {
        let before = links.len();
        let mut rest = vec![];
        for (name, target) in links {
            if dest.join(&target).is_file() {
                std::fs::copy(dest.join(&target), dest.join(&name))
                    .map_err(|e| format!("{name} -> {target}: {e}"))?;
            } else {
                rest.push((name, target));
            }
        }
        if rest.len() == before {
            return Err(format!("dangling link in the package: {}", rest[0].0));
        }
        links = rest;
    }
    Ok(())
}

/// Kills a process by PID (the child handle's kill is tried first by the
/// caller; this also reaches a process whose handle was lost).
pub fn kill_pid(pid: u32) {
    #[cfg(unix)]
    {
        let _ = Command::new("kill").arg("-9").arg(pid.to_string()).status();
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .status();
    }
}

/// Runs `skin` synchronously with a deadline. On timeout the process is
/// killed by its PID and its PID file removed.
pub fn run_skin(
    cfg: &SkinTokensConfig,
    mesh_glb: &[u8],
    skeleton_glb: &[u8],
) -> Result<Vec<u8>, String> {
    if !cfg.is_ready() {
        return Err(
            "skin-tokens.cpp is not installed (binary and weights are downloaded on request)"
                .into(),
        );
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let job = format!("job-{}-{nanos}", std::process::id());
    let dir = cfg.work_dir.join(job);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mesh = dir.join("mesh.glb");
    let skel = dir.join("skeleton.glb");
    let out = dir.join("rigged.glb");
    let log = dir.join("skintokens.log");
    std::fs::write(&mesh, mesh_glb).map_err(|e| e.to_string())?;
    std::fs::write(&skel, skeleton_glb).map_err(|e| e.to_string())?;
    let log_file = std::fs::File::create(&log).map_err(|e| e.to_string())?;
    let log_err = log_file.try_clone().map_err(|e| e.to_string())?;
    let mut cmd = Command::new(&cfg.cli);
    cmd.arg("skin")
        .arg(&cfg.model_dir)
        .arg(&mesh)
        .arg(&skel)
        .arg(&out)
        .args(["--device", &cfg.device, "--fit", "global", "--beams", "1"])
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(log_err));
    if let Some(bin) = cfg.cli.parent() {
        // ggml loads its backends from next to the binary.
        cmd.env("GGML_BACKEND_PATH", bin);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("could not start skintokens-cli: {e}"))?;
    let pid = child.id();
    let pid_file = dir.join("pid");
    let _ = std::fs::write(&pid_file, pid.to_string());
    let deadline = Instant::now() + Duration::from_secs(cfg.timeout_secs.max(10));
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => {
                if Instant::now() > deadline {
                    let _ = child.kill();
                    kill_pid(pid);
                    let _ = child.wait();
                    let _ = std::fs::remove_file(&pid_file);
                    return Err(format!(
                        "skin-tokens.cpp took longer than {} s and was stopped",
                        cfg.timeout_secs
                    ));
                }
                std::thread::sleep(Duration::from_millis(250));
            }
            Err(e) => {
                kill_pid(pid);
                return Err(e.to_string());
            }
        }
    };
    let _ = std::fs::remove_file(&pid_file);
    if !status.success() {
        let tail = std::fs::read_to_string(&log).unwrap_or_default();
        let tail: String = tail
            .lines()
            .rev()
            .take(8)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
        return Err(format!("skin-tokens.cpp failed ({status}): {tail}"));
    }
    let bytes = std::fs::read(&out).map_err(|e| format!("no output from skin-tokens.cpp: {e}"))?;
    let _ = std::fs::remove_dir_all(&dir);
    Ok(bytes)
}

/// VRM humanoid bones in T-pose, `(name, parent, local translation)` for a
/// 1.7 m body facing +z (VRM 1.0), left = +x.
pub const VRM_SKELETON: &[(&str, Option<&str>, [f32; 3])] = &[
    ("hips", None, [0.0, 0.95, 0.0]),
    ("spine", Some("hips"), [0.0, 0.10, 0.0]),
    ("chest", Some("spine"), [0.0, 0.15, 0.0]),
    ("upperChest", Some("chest"), [0.0, 0.12, 0.0]),
    ("neck", Some("upperChest"), [0.0, 0.15, 0.0]),
    ("head", Some("neck"), [0.0, 0.08, 0.0]),
    ("leftShoulder", Some("upperChest"), [0.05, 0.10, 0.0]),
    ("leftUpperArm", Some("leftShoulder"), [0.10, 0.0, 0.0]),
    ("leftLowerArm", Some("leftUpperArm"), [0.26, 0.0, 0.0]),
    ("leftHand", Some("leftLowerArm"), [0.24, 0.0, 0.0]),
    ("rightShoulder", Some("upperChest"), [-0.05, 0.10, 0.0]),
    ("rightUpperArm", Some("rightShoulder"), [-0.10, 0.0, 0.0]),
    ("rightLowerArm", Some("rightUpperArm"), [-0.26, 0.0, 0.0]),
    ("rightHand", Some("rightLowerArm"), [-0.24, 0.0, 0.0]),
    ("leftUpperLeg", Some("hips"), [0.09, -0.05, 0.0]),
    ("leftLowerLeg", Some("leftUpperLeg"), [0.0, -0.42, 0.0]),
    ("leftFoot", Some("leftLowerLeg"), [0.0, -0.40, 0.0]),
    ("leftToes", Some("leftFoot"), [0.0, -0.05, 0.12]),
    ("rightUpperLeg", Some("hips"), [-0.09, -0.05, 0.0]),
    ("rightLowerLeg", Some("rightUpperLeg"), [0.0, -0.42, 0.0]),
    ("rightFoot", Some("rightLowerLeg"), [0.0, -0.40, 0.0]),
    ("rightToes", Some("rightFoot"), [0.0, -0.05, 0.12]),
];

/// Plain GLB (no compression) with the standard skeleton, a skin over it
/// and a one-triangle placeholder mesh, plus the `VRMC_vrm` humanoid map.
pub fn vrm_skeleton_glb(height_m: f32) -> Vec<u8> {
    let k = height_m / 1.7;
    let mut model = Model::default();
    let index = |name: &str| VRM_SKELETON.iter().position(|b| b.0 == name);
    let mut globals: Vec<[f32; 3]> = vec![];
    for (name, parent, t) in VRM_SKELETON {
        let t = [t[0] * k, t[1] * k, t[2] * k];
        let g = match parent.and_then(index) {
            Some(p) => [
                globals[p][0] + t[0],
                globals[p][1] + t[1],
                globals[p][2] + t[2],
            ],
            None => t,
        };
        globals.push(g);
        model.nodes.push(Node {
            name: Some(name.to_string()),
            t,
            ..Default::default()
        });
    }
    for (i, (_, parent, _)) in VRM_SKELETON.iter().enumerate() {
        if let Some(p) = parent.and_then(index) {
            model.nodes[p].children.push(i);
        }
    }
    let joints: Vec<usize> = (0..VRM_SKELETON.len()).collect();
    let ibm = globals
        .iter()
        .map(|g| from_trs([-g[0], -g[1], -g[2]], [0.0, 0.0, 0.0, 1.0], [1.0; 3]))
        .collect();
    model.skins.push(Skin {
        name: Some("vrm-humanoid".into()),
        joints,
        ibm: Some(ibm),
        skeleton: Some(0),
    });
    let h = globals[0];
    model.meshes.push(Mesh {
        name: Some("placeholder".into()),
        prims: vec![Prim {
            pos: vec![
                [h[0] - 0.01, h[1], h[2]],
                [h[0] + 0.01, h[1], h[2]],
                [h[0], h[1] + 0.01, h[2]],
            ],
            nrm: Some(vec![[0.0, 0.0, 1.0]; 3]),
            joints: Some(vec![[0, 0, 0, 0]; 3]),
            weights: Some(vec![[1.0, 0.0, 0.0, 0.0]; 3]),
            idx: vec![0, 1, 2],
            ..Default::default()
        }],
        weights: None,
        extras: None,
    });
    model.nodes.push(Node {
        name: Some("body".into()),
        mesh: Some(0),
        skin: Some(0),
        ..Default::default()
    });
    let body = model.nodes.len() - 1;
    model.roots = vec![0, body];
    let human: serde_json::Map<String, serde_json::Value> = VRM_SKELETON
        .iter()
        .enumerate()
        .map(|(i, (n, _, _))| (n.to_string(), serde_json::json!({"node": i})))
        .collect();
    model.root_extensions.insert(
        "VRMC_vrm".into(),
        serde_json::json!({
            "specVersion": "1.0",
            "meta": {"name": "OmniGet humanoid", "authors": ["OmniGet"], "licenseUrl": "https://vrm.dev/licenses/1.0/",
                     "avatarPermission": "everyone", "allowRedistribution": true, "modification": "allowModificationRedistribution"},
            "humanoid": {"humanBones": human}
        }),
    );
    write_glb::write(
        &model,
        &WriteOptions {
            quantize_positions: false,
            compress: false,
            asset_extras: serde_json::Value::Null,
            copyright: None,
        },
    )
}

/// skin-tokens.cpp writes the skin with our joint names but drops the
/// `VRMC_vrm` extension of the skeleton it was given, so three-vrm and the
/// normalizer would see a plain skinned mesh. This maps the joints back to
/// the humanoid by name and adds the same `meta` as [`vrm_skeleton_glb`].
pub fn attach_vrm_humanoid(glb: &[u8]) -> Result<Vec<u8>, String> {
    let (json, bin) = crate::load_gltf::split_glb(glb).map_err(|e| e.to_string())?;
    let mut doc: serde_json::Value = serde_json::from_slice(json).map_err(|e| e.to_string())?;
    let nodes = doc["nodes"].as_array().cloned().unwrap_or_default();
    let joints: Vec<usize> = doc["skins"][0]["joints"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_u64())
                .map(|v| v as usize)
                .collect()
        })
        .unwrap_or_default();
    let mut human = serde_json::Map::new();
    for &j in &joints {
        let Some(name) = nodes.get(j).and_then(|n| n["name"].as_str()) else {
            continue;
        };
        if VRM_SKELETON.iter().any(|b| b.0 == name) {
            human.insert(name.to_string(), serde_json::json!({ "node": j }));
        }
    }
    let missing: Vec<&str> = VRM_REQUIRED
        .iter()
        .copied()
        .filter(|b| !human.contains_key(*b))
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "skin-tokens.cpp output lacks humanoid bones: {}",
            missing.join(", ")
        ));
    }
    doc["extensions"]["VRMC_vrm"] = serde_json::json!({
        "specVersion": "1.0",
        "meta": {"name": "OmniGet humanoid", "authors": ["OmniGet"], "licenseUrl": "https://vrm.dev/licenses/1.0/",
                 "avatarPermission": "everyone", "allowRedistribution": true, "modification": "allowModificationRedistribution"},
        "humanoid": {"humanBones": human}
    });
    let used = doc["extensionsUsed"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if !used.iter().any(|v| v == "VRMC_vrm") {
        let mut used = used;
        used.push("VRMC_vrm".into());
        doc["extensionsUsed"] = serde_json::Value::Array(used);
    }
    let mut json = serde_json::to_vec(&doc).map_err(|e| e.to_string())?;
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let bin = bin.unwrap_or(&[]);
    let bin_pad = (4 - bin.len() % 4) % 4;
    let bin_len = if bin.is_empty() {
        0
    } else {
        8 + bin.len() + bin_pad
    };
    let total = 12 + 8 + json.len() + bin_len;
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(&0x4E4F_534Au32.to_le_bytes());
    out.extend_from_slice(&json);
    if !bin.is_empty() {
        out.extend_from_slice(&((bin.len() + bin_pad) as u32).to_le_bytes());
        out.extend_from_slice(&0x004E_4942u32.to_le_bytes());
        out.extend_from_slice(bin);
        out.extend(std::iter::repeat_n(0u8, bin_pad));
    }
    Ok(out)
}

/// The 15 bones VRM 1.0 requires.
const VRM_REQUIRED: &[&str] = &[
    "hips",
    "spine",
    "head",
    "leftUpperArm",
    "leftLowerArm",
    "leftHand",
    "rightUpperArm",
    "rightLowerArm",
    "rightHand",
    "leftUpperLeg",
    "leftLowerLeg",
    "leftFoot",
    "rightUpperLeg",
    "rightLowerLeg",
    "rightFoot",
];

pub struct SkinTokens {
    pub cfg: SkinTokensConfig,
}

impl Rigger for SkinTokens {
    fn id(&self) -> &'static str {
        "skin-tokens.cpp"
    }

    /// `static_glb` must be a plain, indexed GLB (see
    /// [`crate::plain_glb_for_rigging`]). The output has no textures; the
    /// caller normalizes it again as a character.
    fn rig(&self, static_glb: Vec<u8>) -> BoxFuture<'_, Result<Vec<u8>, String>> {
        let cfg = self.cfg.clone();
        Box::pin(async move {
            let skel = vrm_skeleton_glb(1.7);
            let rigged = tokio::task::spawn_blocking(move || run_skin(&cfg, &static_glb, &skel))
                .await
                .map_err(|e| e.to_string())??;
            attach_vrm_humanoid(&rigged)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn humanoid_is_reattached_by_joint_name() {
        let skel = vrm_skeleton_glb(1.7);
        let out = attach_vrm_humanoid(&skel).unwrap();
        let (json, _) = crate::load_gltf::split_glb(&out).unwrap();
        let doc: serde_json::Value = serde_json::from_slice(json).unwrap();
        let bones = doc["extensions"]["VRMC_vrm"]["humanoid"]["humanBones"]
            .as_object()
            .unwrap();
        assert_eq!(bones.len(), VRM_SKELETON.len());
        assert_eq!(bones["leftHand"]["node"], 9);
        assert!(load_glb_ok(&out));
    }

    fn tar_gz(entries: &[(&str, Option<&str>, &[u8])]) -> Vec<u8> {
        let mut b = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::fast(),
        ));
        for (name, link, data) in entries {
            let mut h = tar::Header::new_gnu();
            h.set_mode(0o755);
            match link {
                Some(t) => {
                    h.set_entry_type(tar::EntryType::Symlink);
                    h.set_size(0);
                    b.append_link(&mut h, name, t).unwrap();
                }
                None => {
                    h.set_size(data.len() as u64);
                    b.append_data(&mut h, name, *data).unwrap();
                }
            }
        }
        b.into_inner().unwrap().finish().unwrap()
    }

    fn scratch(tag: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("omniget-skintokens-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn package_extracts_flat_with_link_chains_and_replaces_bin() {
        let d = scratch("ok");
        let archive = d.join("p.tar.gz");
        std::fs::write(
            &archive,
            tar_gz(&[
                ("./skintokens-cli", None, b"exe"),
                ("libggml.dylib", Some("libggml.0.dylib"), b""),
                ("libggml.0.dylib", Some("libggml.0.20.2.dylib"), b""),
                ("libggml.0.20.2.dylib", None, b"lib"),
            ]),
        )
        .unwrap();
        let bin = d.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("old"), b"x").unwrap();
        install_cli_archive(&archive, &bin, OsStr::new("skintokens-cli")).unwrap();
        assert_eq!(std::fs::read(bin.join("skintokens-cli")).unwrap(), b"exe");
        assert_eq!(std::fs::read(bin.join("libggml.dylib")).unwrap(), b"lib");
        assert!(!bin.join("old").exists());
        assert!(!d.join("bin.new").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn package_with_escaping_paths_is_refused_and_keeps_bin() {
        for (i, entries) in [
            vec![("../evil", None, &b"x"[..])],
            vec![("sub/skintokens-cli", None, &b"x"[..])],
            vec![
                ("skintokens-cli", None, &b"x"[..]),
                ("libx.so", Some("/etc/passwd"), &b""[..]),
            ],
            vec![("libggml.so", None, &b"x"[..])],
        ]
        .into_iter()
        .enumerate()
        {
            let d = scratch(&format!("bad{i}"));
            let archive = d.join("p.tar.gz");
            // `tar::Builder` refuses `..` itself, so write that header by hand.
            let bytes = if i == 0 {
                let mut h = tar::Header::new_gnu();
                h.as_gnu_mut().unwrap().name[..8].copy_from_slice(b"../evil\0");
                h.set_size(1);
                h.set_mode(0o644);
                h.set_cksum();
                let mut raw = h.as_bytes().to_vec();
                raw.extend_from_slice(&[b'x'; 1]);
                raw.resize(512 * 2, 0);
                raw.resize(512 * 4, 0);
                let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
                std::io::Write::write_all(&mut gz, &raw).unwrap();
                gz.finish().unwrap()
            } else {
                tar_gz(&entries)
            };
            std::fs::write(&archive, bytes).unwrap();
            let bin = d.join("bin");
            std::fs::create_dir_all(&bin).unwrap();
            std::fs::write(bin.join("keep"), b"x").unwrap();
            assert!(
                install_cli_archive(&archive, &bin, OsStr::new("skintokens-cli")).is_err(),
                "case {i}"
            );
            assert!(bin.join("keep").exists(), "case {i}");
            assert!(!d.join("evil").exists(), "case {i}");
            assert!(!d.join("bin.new").exists(), "case {i}");
            let _ = std::fs::remove_dir_all(&d);
        }
    }

    #[test]
    fn archive_urls_are_recognized() {
        assert!(is_archive("https://x/y/skintokens-cli-linux-x64.tar.gz"));
        assert!(is_archive("https://x/y/a.tgz?download=1"));
        assert!(!is_archive("https://x/y/skintokens-cli"));
    }

    fn load_glb_ok(glb: &[u8]) -> bool {
        crate::load_glb(glb, &crate::Limits::default()).is_ok()
    }
}
