use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::edit::{DocFormat, Seg};
use super::model::{Compat, Component, ComponentKind, SourceRef};
use super::{AgentkitError, Env, Result, Scope};

pub const LOCK_VERSION: u32 = 1;
pub const PROJECT_LOCK: &str = "agentkit.lock.json";
pub const PROJECT_LOCK_DIR: &str = ".omniget";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum UndoOp {
    Remove {
        path: Vec<Seg>,
        prune_to: usize,
        sha256: String,
    },
    Restore {
        path: Vec<Seg>,
        prev: Value,
        sha256: String,
    },
    RemoveItem {
        path: Vec<Seg>,
        value: Value,
        prune_to: usize,
    },
    TextBlock {
        id: String,
        sha256: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WrittenAction {
    Created {
        sha256: String,
        #[serde(default)]
        executable: bool,
    },
    Merged {
        format: DocFormat,
        created_file: bool,
        undo: Vec<UndoOp>,
    },
    Linked {
        to: String,
        #[serde(default)]
        copied: bool,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        files: Vec<LinkedFile>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinkedFile {
    pub rel: String,
    pub sha256: String,
}

impl UndoOp {
    pub fn same_piece(&self, other: &UndoOp) -> bool {
        use UndoOp::*;
        match (self, other) {
            (
                Remove { path: a, .. } | Restore { path: a, .. },
                Remove { path: b, .. } | Restore { path: b, .. },
            ) => a == b,
            (
                RemoveItem {
                    path: a, value: va, ..
                },
                RemoveItem {
                    path: b, value: vb, ..
                },
            ) => a == b && va == vb,
            (TextBlock { id: a, .. }, TextBlock { id: b, .. }) => a == b,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WrittenFile {
    pub path: String,
    #[serde(flatten)]
    pub action: WrittenAction,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComponentMeta {
    pub id: String,
    pub kind: ComponentKind,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub origin_tool: String,
}

impl From<&Component> for ComponentMeta {
    fn from(c: &Component) -> Self {
        ComponentMeta {
            id: c.id.clone(),
            kind: c.kind,
            name: c.name.clone(),
            category: c.category.clone(),
            description: c.description.clone(),
            source: c.source.clone(),
            license: c.license.clone(),
            sha256: c.sha256.clone(),
            origin_tool: c.origin_tool.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstallRecord {
    pub install_id: String,
    pub component: ComponentMeta,
    pub target: String,
    pub scope: Scope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_dir: Option<String>,
    pub installed_name: String,
    pub compat: Compat,
    pub tx: String,
    pub installed_at: String,
    pub files: Vec<WrittenFile>,
    #[serde(default)]
    pub created_dirs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Lockfile {
    pub version: u32,
    #[serde(default)]
    pub installs: Vec<InstallRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub projects: Vec<String>,
}

pub fn lock_path(env: &Env, scope: Scope, project: Option<&Path>) -> Result<PathBuf> {
    if scope.is_project_bound() {
        let p = project.ok_or_else(|| {
            AgentkitError::new("AGENTKIT_SCOPE", "project scope needs a project folder")
        })?;
        Ok(p.join(PROJECT_LOCK_DIR).join(PROJECT_LOCK))
    } else {
        Ok(global_lock_path(env))
    }
}

pub fn global_lock_path(env: &Env) -> PathBuf {
    env.agentkit_dir().join("global.lock.json")
}

pub fn project_lock_path(project: &Path) -> PathBuf {
    project.join(PROJECT_LOCK_DIR).join(PROJECT_LOCK)
}

impl Lockfile {
    pub fn load(path: &Path) -> Result<Lockfile> {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| {
                AgentkitError::new("AGENTKIT_LOCK", format!("{}: {e}", path.display()))
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Lockfile {
                version: LOCK_VERSION,
                ..Default::default()
            }),
            Err(e) => Err(AgentkitError::io("reading", path, &e)),
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = serde_json::to_vec_pretty(self).unwrap_or_default();
        v.push(b'\n');
        v
    }

    pub fn is_empty(&self) -> bool {
        self.installs.is_empty() && self.projects.is_empty()
    }

    pub fn find(
        &self,
        component_id: &str,
        target: &str,
        scope: Scope,
        project: Option<&str>,
    ) -> Option<&InstallRecord> {
        self.installs.iter().find(|r| {
            r.component.id == component_id
                && r.target == target
                && r.scope == scope
                && r.project_dir.as_deref() == project
        })
    }

    pub fn by_id(&self, install_id: &str) -> Option<&InstallRecord> {
        self.installs.iter().find(|r| r.install_id == install_id)
    }

    pub fn created_elsewhere(&self, path: &str, except: &str) -> bool {
        self.installs.iter().any(|r| {
            r.install_id != except
                && r.files.iter().any(|f| {
                    f.path == path
                        && matches!(
                            f.action,
                            WrittenAction::Created { .. } | WrittenAction::Linked { .. }
                        )
                })
        })
    }

    pub fn merged_elsewhere<'a>(
        &'a self,
        path: &'a str,
        except: &'a str,
    ) -> impl Iterator<Item = &'a UndoOp> + 'a {
        self.installs
            .iter()
            .filter(move |r| r.install_id != except)
            .flat_map(|r| r.files.iter())
            .filter(move |f| f.path == path)
            .flat_map(|f| match &f.action {
                WrittenAction::Merged { undo, .. } => undo.iter(),
                _ => [].iter(),
            })
    }
}

pub fn all_locks(env: &Env, project: Option<&Path>) -> Vec<(PathBuf, Lockfile)> {
    let mut out = Vec::new();
    let g = global_lock_path(env);
    let global = Lockfile::load(&g).unwrap_or_default();
    let mut projects: Vec<PathBuf> = global.projects.iter().map(PathBuf::from).collect();
    out.push((g, global));
    if let Some(p) = project {
        if !projects.iter().any(|x| x == p) {
            projects.insert(0, p.to_path_buf());
        }
    }
    for p in projects {
        let lp = project_lock_path(&p);
        if let Ok(l) = Lockfile::load(&lp) {
            if !l.installs.is_empty() {
                out.push((lp, l));
            }
        }
    }
    out
}
