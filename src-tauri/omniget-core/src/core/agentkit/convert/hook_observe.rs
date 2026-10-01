use std::collections::BTreeMap;
use std::path::Path;

use serde_json::Value;

use super::hook_shim::{read_token, tokens_path};
use crate::core::agentkit::{AgentkitError, Env, Result};

pub fn revoke_token(env: &Env, tool: &str) -> Result<bool> {
    let path = tokens_path(&env.app_data);
    let mut all: BTreeMap<String, Value> = match std::fs::read(&path) {
        Ok(b) => serde_json::from_slice(&b).unwrap_or_default(),
        Err(_) => return Ok(false),
    };
    let had = all.remove(tool).is_some();
    if had {
        write_private(&path, &serde_json::to_vec_pretty(&all).unwrap_or_default())?;
    }
    Ok(had)
}

pub fn check_token(data_dir: &Path, tool: &str, provided: &str) -> bool {
    let Some(expected) = read_token(data_dir, tool) else {
        return false;
    };
    let (a, b) = (expected.as_bytes(), provided.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| AgentkitError::io("create", dir, &e))?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes).map_err(|e| AgentkitError::io("write", &tmp, &e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, path).map_err(|e| AgentkitError::io("rename", path, &e))
}
