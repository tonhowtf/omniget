use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::persist;
use super::store::CanvasStore;
use crate::core::vcs::runner::Invocation;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum EnvKind {
    #[default]
    Local,
    Tmux,
    Ssh,
    Docker,
    Sandbox,
    Custom,
    Wsl,
}

impl EnvKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Tmux => "tmux",
            Self::Ssh => "ssh",
            Self::Docker => "docker",
            Self::Sandbox => "sandbox",
            Self::Custom => "custom",
            Self::Wsl => "wsl",
        }
    }

    pub fn parse(s: &str) -> Option<EnvKind> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "local" | "" => Self::Local,
            "tmux" | "local-tmux" | "local_tmux" => Self::Tmux,
            "ssh" => Self::Ssh,
            "docker" | "container" => Self::Docker,
            "sandbox" | "sbx" | "docker-sandbox" => Self::Sandbox,
            "custom" | "runtime" => Self::Custom,
            "wsl" => Self::Wsl,
            _ => return None,
        })
    }

    pub fn is_remote(&self) -> bool {
        !matches!(self, Self::Local | Self::Tmux)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct EnvRecord {
    pub kind: EnvKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_options: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persistent: Option<bool>,
    #[serde(skip)]
    pub runtime_def: Option<CustomRuntime>,
    #[serde(skip)]
    pub notice: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CustomRuntime {
    pub exec: String,
    #[serde(default)]
    pub args: String,
    #[serde(default)]
    pub interactive: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bridge_host: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EnvProviders {
    pub tmux: bool,
    pub ssh: bool,
    pub docker: bool,
    pub sandbox: bool,
    pub custom: bool,
    pub wsl: bool,
    pub runtimes: BTreeMap<String, CustomRuntime>,
}

impl Default for EnvProviders {
    fn default() -> Self {
        Self {
            tmux: true,
            ssh: true,
            docker: true,
            sandbox: true,
            custom: true,
            wsl: cfg!(windows),
            runtimes: BTreeMap::new(),
        }
    }
}

impl EnvProviders {
    pub fn enabled(&self, k: EnvKind) -> bool {
        match k {
            EnvKind::Local => true,
            EnvKind::Tmux => self.tmux,
            EnvKind::Ssh => self.ssh,
            EnvKind::Docker => self.docker,
            EnvKind::Sandbox => self.sandbox,
            EnvKind::Custom => self.custom,
            EnvKind::Wsl => self.wsl,
        }
    }
}

fn providers_path(store: &CanvasStore) -> PathBuf {
    store.root().join("environments.json")
}

pub fn providers(store: &CanvasStore) -> EnvProviders {
    std::fs::read(providers_path(store))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn set_providers(store: &CanvasStore, patch: &Value) -> Result<EnvProviders, String> {
    let cur = providers(store);
    let mut v = serde_json::to_value(&cur).map_err(|e| e.to_string())?;
    let Some(p) = patch.as_object() else {
        return Err("CANVAS_INVALID: providers patch must be an object".into());
    };
    for (k, val) in p {
        if k == "runtimes" {
            let rt = v["runtimes"].as_object_mut().ok_or("bad runtimes")?;
            for (name, def) in val
                .as_object()
                .ok_or("CANVAS_INVALID: runtimes is an object")?
            {
                if def.is_null() {
                    rt.remove(name);
                } else {
                    let d: CustomRuntime = serde_json::from_value(def.clone())
                        .map_err(|e| format!("CANVAS_INVALID: runtime {name}: {e}"))?;
                    if d.exec.trim().is_empty() || d.exec.trim().starts_with('-') {
                        return Err(format!(
                            "CANVAS_INVALID: runtime {name} needs an executable"
                        ));
                    }
                    rt.insert(
                        name.clone(),
                        serde_json::to_value(d).map_err(|e| e.to_string())?,
                    );
                }
            }
        } else if k == "local" {
            if val.as_bool() == Some(false) {
                return Err("CANVAS_INVALID: Local is always on".into());
            }
        } else if matches!(
            k.as_str(),
            "tmux" | "ssh" | "docker" | "sandbox" | "custom" | "wsl"
        ) {
            v[k] = json!(val
                .as_bool()
                .ok_or_else(|| format!("CANVAS_INVALID: {k} is true or false"))?);
        } else {
            return Err(format!("CANVAS_INVALID: unknown environment switch '{k}'"));
        }
    }
    let next: EnvProviders = serde_json::from_value(v).map_err(|e| e.to_string())?;
    let bytes = serde_json::to_vec_pretty(&next).map_err(|e| e.to_string())?;
    persist::write_atomic(&providers_path(store), &bytes)?;
    Ok(next)
}

fn path(store: &CanvasStore, ws: &str) -> PathBuf {
    store.workspace_dir(ws).join("environment.json")
}

pub fn load(store: &CanvasStore, ws: &str) -> EnvRecord {
    std::fs::read(path(store, ws))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn load_for(store: &CanvasStore, ws: &str, node_id: Option<&str>) -> EnvRecord {
    let own = node_id.and_then(|id| {
        let (w, floor) = store.locate_node(id)?;
        store
            .read(&w, &floor, |d| {
                d.node(id).and_then(|n| n.data.get("environment").cloned())
            })
            .ok()
            .flatten()
    });
    let rec = match own.as_ref().and_then(node_override) {
        Some(r) => r,
        None => load(store, ws),
    };
    resolve(store, rec)
}

pub fn node_override(v: &Value) -> Option<EnvRecord> {
    match v {
        Value::String(s) if s == "workspace" || s.is_empty() => None,
        Value::String(s) => EnvKind::parse(s).map(|kind| EnvRecord {
            kind,
            ..Default::default()
        }),
        Value::Object(_) => serde_json::from_value(v.clone()).ok(),
        _ => None,
    }
}

pub fn resolve(store: &CanvasStore, mut rec: EnvRecord) -> EnvRecord {
    let p = providers(store);
    if !p.enabled(rec.kind) {
        let was = rec.kind.as_str();
        return EnvRecord {
            notice: Some(format!(
                "omniget: the {was} environment is turned off in Settings → World → Canvas → Environments; this terminal runs on this machine"
            )),
            ..Default::default()
        };
    }
    if rec.kind == EnvKind::Custom {
        let name = rec.runtime.clone().unwrap_or_default();
        match p.runtimes.get(&name) {
            Some(d) => rec.runtime_def = Some(d.clone()),
            None => {
                return EnvRecord {
                    notice: Some(format!(
                        "omniget: the custom runtime '{name}' does not exist anymore; this terminal runs on this machine"
                    )),
                    ..Default::default()
                }
            }
        }
    }
    rec
}

pub fn validate(store: &CanvasStore, rec: &EnvRecord) -> Result<(), String> {
    let p = providers(store);
    if !p.enabled(rec.kind) {
        return Err(format!(
            "CANVAS_INVALID: the {} environment is turned off (Settings → World → Canvas → Environments)",
            rec.kind.as_str()
        ));
    }
    let needs_host = matches!(rec.kind, EnvKind::Ssh | EnvKind::Docker | EnvKind::Sandbox);
    if needs_host && rec.host.as_deref().is_none_or(|h| h.trim().is_empty()) {
        return Err(
            "CANVAS_INVALID: this environment needs a host (SSH destination, container or sandbox)"
                .into(),
        );
    }
    if let Some(h) = &rec.host {
        if h.trim().starts_with('-') {
            return Err("CANVAS_INVALID: host cannot start with '-'".into());
        }
    }
    if rec.kind == EnvKind::Custom {
        let name = rec.runtime.as_deref().unwrap_or("");
        if !p.runtimes.contains_key(name) {
            return Err(format!(
                "CANVAS_INVALID: no custom runtime named '{name}' (define it in Settings → World → Canvas → Environments)"
            ));
        }
    }
    Ok(())
}

pub fn save(store: &CanvasStore, ws: &str, rec: EnvRecord) -> Result<EnvRecord, String> {
    let meta = store.workspace(ws)?;
    validate(store, &rec)?;
    let isolated = meta.floors.iter().any(|f| f.is_git_isolated);
    if rec.kind.is_remote() && isolated {
        return Err(
            "CANVAS_INVALID: remove the workspace's isolated floors before making it remote".into(),
        );
    }
    let bytes = serde_json::to_vec_pretty(&rec).map_err(|e| e.to_string())?;
    persist::write_atomic(&path(store, ws), &bytes)?;
    let mut patch = Map::new();
    patch.insert(
        "environment".into(),
        if rec.kind == EnvKind::Local {
            Value::Null
        } else {
            json!({
                "kind": rec.kind.as_str(),
                "name": rec.host.clone().or(rec.runtime.clone()).unwrap_or_default(),
            })
        },
    );
    store.update_workspace(ws, patch)?;
    Ok(rec)
}

pub fn record_from_value(v: &Value) -> Result<EnvRecord, String> {
    if v.is_null() {
        return Ok(EnvRecord::default());
    }
    if let Some(s) = v.as_str() {
        let (k, h) = s.split_once(':').unwrap_or((s, ""));
        let kind = EnvKind::parse(k)
            .ok_or_else(|| format!("CANVAS_INVALID: unknown environment '{k}'"))?;
        return Ok(EnvRecord {
            kind,
            host: Some(h.to_string()).filter(|h| !h.is_empty()),
            ..Default::default()
        });
    }
    let mut o = v
        .as_object()
        .cloned()
        .ok_or("CANVAS_INVALID: environment is an object, a string or null")?;
    if let Some(k) = o.get("kind").and_then(|k| k.as_str()) {
        let kind = EnvKind::parse(k)
            .ok_or_else(|| format!("CANVAS_INVALID: unknown environment '{k}'"))?;
        o.insert("kind".into(), json!(kind.as_str()));
    }
    if !o.contains_key("host") {
        if let Some(n) = o.remove("name") {
            o.insert("host".into(), n);
        }
    }
    serde_json::from_value(Value::Object(o))
        .map_err(|e| format!("CANVAS_INVALID: environment: {e}"))
}

pub fn check_env_overrides(env: &Value) -> Result<(), String> {
    let Some(o) = env.as_object() else {
        return Ok(());
    };
    let bad: Vec<&str> = o
        .keys()
        .map(String::as_str)
        .filter(|k| {
            let u = k.to_ascii_uppercase();
            u == "PATH"
                || u == "HOME"
                || u.starts_with("OMNIGET_")
                || super::accounts::CONFIG_VARS.contains(&u.as_str())
        })
        .collect();
    if bad.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "CANVAS_INVALID: env overrides cannot set {} (PATH, HOME and OMNIGET_* belong to the terminal; CLAUDE_CONFIG_DIR and CODEX_HOME come from the node's account)",
            bad.join(", ")
        ))
    }
}

pub fn sh_quote(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./=:@%+,".contains(c))
    {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', r"'\''"))
}

pub fn bridge_port(env: &BTreeMap<String, String>) -> Option<u16> {
    let url = env.get("OMNIGET_BRIDGE_URL")?;
    let rest = url.split("://").nth(1).unwrap_or(url);
    let hostport = rest.split('/').next()?;
    hostport.rsplit(':').next()?.parse().ok()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Wrapped {
    pub command: Vec<String>,
    pub cwd: Option<String>,
    pub env: BTreeMap<String, String>,
}

fn local_only(k: &str) -> bool {
    matches!(
        k,
        "OMNIGET_CLI" | "PATH" | "HOME" | "SHELL" | "TMPDIR" | "USER"
    )
}

const BUNDLE: &str = "OMNIGET_ENV_BUNDLE";

pub const CLI_SHIM: &str = r#"#!/bin/sh
# OmniGet CLI shim: runs `omniget canvas ...` on the host app through the
# canvas bridge, as this terminal (its OMNIGET_CANVAS_TOKEN).
if [ "$1" != "canvas" ]; then
  echo "omniget: only 'omniget canvas ...' runs in this environment" >&2
  exit 2
fi
if [ -z "$OMNIGET_BRIDGE_URL" ] || [ -z "$OMNIGET_CANVAS_TOKEN" ]; then
  echo "omniget: not inside an OmniGet canvas terminal" >&2
  exit 2
fi
esc() { printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g' -e 's/	/\\t/g' -e 's/\r/\\r/g' | awk 'BEGIN{ORS=""} NR>1{print "\\n"} {print}'; }
body='{"method":"env.cli","args":{"argv":['
sep=''
for a in "$@"; do body="$body$sep\"$(esc "$a")\""; sep=','; done
body="$body],\"token\":\"$OMNIGET_CANVAS_TOKEN\"}}"
url="$OMNIGET_BRIDGE_URL/v1/canvas/rpc"
tmp=$(mktemp 2>/dev/null || echo "/tmp/omniget-shim-$$")
( umask 077; printf 'Authorization: Bearer %s\n' "$OMNIGET_CANVAS_TOKEN" > "$tmp.h"; printf '%s' "$body" > "$tmp.b" )
if command -v curl >/dev/null 2>&1; then
  resp=$(curl -sS -X POST -H @"$tmp.h" -H 'Content-Type: application/json' --data-binary @"$tmp.b" "$url")
elif command -v wget >/dev/null 2>&1; then
  resp=$(wget -qO- --header="$(cat "$tmp.h")" --header='Content-Type: application/json' --post-file="$tmp.b" "$url")
else
  rm -f "$tmp" "$tmp.h" "$tmp.b"
  echo "omniget: needs curl or wget in this environment" >&2
  exit 1
fi
rc=$?
rm -f "$tmp" "$tmp.h" "$tmp.b"
[ $rc -eq 0 ] || { echo "omniget: the bridge at $OMNIGET_BRIDGE_URL did not answer" >&2; exit 1; }
code=$(printf '%s' "$resp" | sed -n 's/.*"code":\(-*[0-9]*\).*/\1/p')
out=$(printf '%s' "$resp" | sed -n 's/.*"out":"\([^"]*\)".*/\1/p')
err=$(printf '%s' "$resp" | sed -n 's/.*"err":"\([^"]*\)".*/\1/p')
if [ -z "$code" ]; then printf '%s\n' "$resp" >&2; exit 1; fi
[ -n "$out" ] && printf '%s' "$out" | base64 -d 2>/dev/null
[ -n "$err" ] && printf '%s' "$err" | base64 -d >&2 2>/dev/null
exit "$code"
"#;

fn shim_install() -> String {
    format!(
        "mkdir -p \"$HOME/.omniget/bin\" 2>/dev/null && cat > \"$HOME/.omniget/bin/omniget\" <<'OMNIGET_SHIM_EOF'\n{CLI_SHIM}OMNIGET_SHIM_EOF\nchmod +x \"$HOME/.omniget/bin/omniget\" 2>/dev/null\n# A login shell resets PATH from /etc/profile: also a link on the system PATH\n# when writable, and the profiles (Maestri \"Add to PATH\").\n[ -w /usr/local/bin ] && ln -sf \"$HOME/.omniget/bin/omniget\" /usr/local/bin/omniget 2>/dev/null\nfor p in \"$HOME/.profile\" \"$HOME/.bashrc\" \"$HOME/.zshrc\"; do [ -f \"$p\" ] || [ \"$p\" = \"$HOME/.profile\" ] || continue; grep -q '.omniget/bin' \"$p\" 2>/dev/null || printf '\\nexport PATH=\"$HOME/.omniget/bin:$PATH\"\\n' >> \"$p\" 2>/dev/null; done\nexport PATH=\"$HOME/.omniget/bin:$PATH\"\nexport OMNIGET_CLI=\"$HOME/.omniget/bin/omniget\"\n"
    )
}

fn final_exec(session: Option<&str>) -> String {
    let plain = "if [ $# -gt 0 ]; then exec \"$@\"; else exec \"${SHELL:-/bin/sh}\" -l; fi";
    match session {
        Some(s) => format!(
            "if command -v tmux >/dev/null 2>&1; then\n  if [ $# -gt 0 ]; then exec tmux new-session -A -s {s} \"$@\"; else exec tmux new-session -A -s {s}; fi\nfi\n{plain}",
            s = sh_quote(s)
        ),
        None => plain.to_string(),
    }
}

fn session_name(env: &BTreeMap<String, String>) -> String {
    let id = env
        .get("OMNIGET_TERMINAL_ID")
        .or(env.get("OMNIGET_CANVAS_NODE"))
        .cloned()
        .unwrap_or_else(|| "terminal".into());
    let clean: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(48)
        .collect();
    format!("omniget-{clean}")
}

fn bundle(vars: &[(String, String)]) -> String {
    vars.iter()
        .map(|(k, v)| format!("export {k}={}\n", sh_quote(v)))
        .collect()
}

fn local_sh(script: String, argv: &[String]) -> Vec<String> {
    let mut c = vec![
        "/bin/sh".to_string(),
        "-c".into(),
        script,
        "omniget-env".into(),
    ];
    c.extend(argv.iter().cloned());
    c
}

fn say(msg: &str) -> String {
    format!("printf '%s\\r\\n' {} >&2\n", sh_quote(msg))
}

pub fn wrap(
    rec: &EnvRecord,
    argv: &[String],
    cwd: Option<&str>,
    env: &BTreeMap<String, String>,
) -> Wrapped {
    let dir = rec
        .dir
        .clone()
        .filter(|d| !d.trim().is_empty())
        .or(cwd.map(str::to_string));
    let host = rec.host.clone().unwrap_or_default();
    let port = bridge_port(env);
    let session = session_name(env);
    let persistent = rec.persistent == Some(true);
    let bridge_host = match rec.kind {
        EnvKind::Docker | EnvKind::Sandbox => Some("host.docker.internal".to_string()),
        EnvKind::Custom => Some(
            rec.runtime_def
                .as_ref()
                .and_then(|d| d.bridge_host.clone())
                .filter(|h| !h.trim().is_empty())
                .unwrap_or_else(|| "host.containers.internal".into()),
        ),
        _ => None,
    };
    let remote_env: Vec<(String, String)> = env
        .iter()
        .filter(|(k, _)| !local_only(k))
        .map(|(k, v)| {
            let v = match (&bridge_host, k.as_str()) {
                (Some(h), "OMNIGET_BRIDGE_URL") => {
                    v.replace("127.0.0.1", h).replace("localhost", h)
                }
                _ => v.clone(),
            };
            (k.clone(), v)
        })
        .collect();
    let notice = rec.notice.as_deref().map(say).unwrap_or_default();
    match rec.kind {
        EnvKind::Local => {
            if notice.is_empty() {
                Wrapped {
                    command: argv.to_vec(),
                    cwd: cwd.map(str::to_string),
                    env: env.clone(),
                }
            } else {
                let script = format!("{notice}{}", final_exec(None));
                Wrapped {
                    command: local_sh(script, argv),
                    cwd: cwd.map(str::to_string),
                    env: env.clone(),
                }
            }
        }
        EnvKind::Tmux => {
            let mut wenv = env.clone();
            wenv.insert(BUNDLE.into(), bundle(&remote_env));
            let d = dir.clone().unwrap_or_else(|| "$HOME".into());
            let inner = "f=\"$HOME/.omniget/env-$OMNIGET_TMUX_SESSION\"; [ -f \"$f\" ] && . \"$f\"; rm -f \"$f\"; if [ $# -gt 0 ]; then exec \"$@\"; else exec \"${SHELL:-/bin/sh}\" -l; fi";
            let script = format!(
                "{notice}command -v tmux >/dev/null 2>&1 || {{ {miss}; {plain} }}\nS={s}\nmkdir -p \"$HOME/.omniget\"\n( umask 077; printf '%s' \"${BUNDLE}\" > \"$HOME/.omniget/env-$S\"; printf '%s' \"${BUNDLE}\" | sed -n 's/^export \\([A-Za-z_][A-Za-z0-9_]*\\)=\\(.*\\)$/set-environment -t '\"$S\"' \\1 \\2/p' > \"$HOME/.omniget/tmux-$S.conf\" )\nif tmux has-session -t \"=$S\" 2>/dev/null; then\n  tmux source-file \"$HOME/.omniget/tmux-$S.conf\" 2>/dev/null; rm -f \"$HOME/.omniget/tmux-$S.conf\" \"$HOME/.omniget/env-$S\"\n  exec tmux attach-session -t \"=$S\"\nfi\nrm -f \"$HOME/.omniget/tmux-$S.conf\"\ncd {d} 2>/dev/null\nexec tmux new-session -s \"$S\" -e OMNIGET_TMUX_SESSION=\"$S\" sh -c {inner} omniget-env \"$@\"",
                miss = say("omniget: tmux is not installed; running without a persistent session").trim_end(),
                plain = "if [ $# -gt 0 ]; then exec \"$@\"; else exec \"${SHELL:-/bin/sh}\" -l; fi;",
                s = sh_quote(&session),
                d = if d == "$HOME" { "\"$HOME\"".to_string() } else { sh_quote(&d) },
                inner = sh_quote(inner),
            );
            Wrapped {
                command: local_sh(script, argv),
                cwd: cwd.map(str::to_string),
                env: wenv,
            }
        }
        EnvKind::Ssh => {
            let mut wenv = env.clone();
            wenv.insert(BUNDLE.into(), bundle(&remote_env));
            let envfile = format!(".omniget/.env-{session}");
            let mut ssh_base = vec!["ssh".to_string()];
            if let Some(o) = &rec.ssh_options {
                ssh_base.extend(o.split_whitespace().map(str::to_string));
            }
            let push = {
                let mut c = ssh_base.clone();
                c.extend([
                    "-o".into(),
                    "BatchMode=yes".into(),
                    "--".into(),
                    host.clone(),
                    format!("umask 077; mkdir -p \"$HOME/.omniget\" && cat > \"$HOME/{envfile}\""),
                ]);
                c
            };
            let mut inside = format!(
                "f=\"$HOME/{envfile}\"; [ -f \"$f\" ] && . \"$f\"; rm -f \"$f\"\n{}",
                shim_install()
            );
            if let Some(d) = &dir {
                inside.push_str(&format!("cd {} || exit 1\n", sh_quote(d)));
            }
            inside.push_str(&final_exec(persistent.then_some(session.as_str())));
            let mut remote_cmd = format!("sh -c {} omniget-env", sh_quote(&inside));
            for a in argv {
                remote_cmd.push(' ');
                remote_cmd.push_str(&sh_quote(a));
            }
            let mut login = ssh_base.clone();
            login.push("-t".into());
            if let Some(p) = port {
                login.push("-R".into());
                login.push(format!("{p}:127.0.0.1:{p}"));
                login.push("-o".into());
                login.push("ExitOnForwardFailure=no".into());
            }
            login.push("--".into());
            login.push(host.clone());
            login.push(remote_cmd);
            let q = |v: &[String]| v.iter().map(|a| sh_quote(a)).collect::<Vec<_>>().join(" ");
            let script = format!(
                "{notice}{}printf '%s' \"${BUNDLE}\" | {} || {{ {}; exit 1; }}\nexec {}",
                say(&format!("omniget: connecting to {host}…")),
                q(&push),
                say("omniget: could not reach the host (keys, BatchMode). Try the environment test.").trim_end(),
                q(&login),
            );
            Wrapped {
                command: local_sh(script, &[]),
                cwd: None,
                env: wenv,
            }
        }
        EnvKind::Docker | EnvKind::Sandbox => {
            let exe = if rec.kind == EnvKind::Docker {
                "docker"
            } else {
                "sbx"
            };
            let mut command = vec![exe.to_string(), "exec".into(), "-it".into()];
            if let Some(d) = &dir {
                command.push("-w".into());
                command.push(d.clone());
            }
            if let Some(u) = rec.user.as_ref().filter(|u| !u.is_empty()) {
                command.push("-u".into());
                command.push(u.clone());
            }
            let mut wenv = env.clone();
            for (k, v) in &remote_env {
                command.push("-e".into());
                command.push(k.clone());
                wenv.insert(k.clone(), v.clone());
            }
            command.push(host.clone());
            let inside = format!(
                "{}{}",
                shim_install(),
                final_exec(persistent.then_some(session.as_str()))
            );
            command.extend(["sh".to_string(), "-c".into(), inside, "omniget-env".into()]);
            command.extend(argv.iter().cloned());
            if notice.is_empty() {
                Wrapped {
                    command,
                    cwd: None,
                    env: wenv,
                }
            } else {
                let script = format!("{notice}exec \"$@\"");
                Wrapped {
                    command: local_sh(script, &command),
                    cwd: None,
                    env: wenv,
                }
            }
        }
        EnvKind::Custom => {
            let def = rec.runtime_def.clone().unwrap_or_default();
            let exe = resolve_exec(&def.exec);
            let shell = def
                .shell
                .clone()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "/bin/sh".into());
            let mut wenv = env.clone();
            wenv.insert(BUNDLE.into(), bundle(&remote_env));
            let envfile = format!(".omniget/.env-{session}");
            let mut push = vec![exe.clone()];
            push.extend(def.args.split_whitespace().map(str::to_string));
            push.extend([
                shell.clone(),
                "-c".into(),
                format!("umask 077; mkdir -p \"$HOME/.omniget\" && cat > \"$HOME/{envfile}\""),
            ]);
            let mut inside = format!(
                "f=\"$HOME/{envfile}\"; [ -f \"$f\" ] && . \"$f\"; rm -f \"$f\"\n{}",
                shim_install()
            );
            if let Some(d) = &dir {
                inside.push_str(&format!("cd {} || exit 1\n", sh_quote(d)));
            }
            inside.push_str(&final_exec(persistent.then_some(session.as_str())));
            let mut login = vec![exe];
            login.extend(def.interactive.split_whitespace().map(str::to_string));
            login.extend([shell, "-c".into(), inside, "omniget-env".into()]);
            login.extend(argv.iter().cloned());
            let q = |v: &[String]| v.iter().map(|a| sh_quote(a)).collect::<Vec<_>>().join(" ");
            let script = format!(
                "{notice}printf '%s' \"${BUNDLE}\" | {} || {{ {}; exit 1; }}\nexec {}",
                q(&push),
                say("omniget: the runtime did not answer; check it with the environment test.")
                    .trim_end(),
                q(&login),
            );
            Wrapped {
                command: local_sh(script, &[]),
                cwd: None,
                env: wenv,
            }
        }
        EnvKind::Wsl => {
            let mut command = vec!["wsl.exe".to_string()];
            if !host.is_empty() {
                command.push("-d".into());
                command.push(host);
            }
            if let Some(u) = rec.user.as_ref().filter(|u| !u.is_empty()) {
                command.push("-u".into());
                command.push(u.clone());
            }
            if let Some(d) = &dir {
                command.push("--cd".into());
                command.push(d.clone());
            }
            let mut wenv = env.clone();
            let names: Vec<String> = remote_env.iter().map(|(k, _)| format!("{k}/u")).collect();
            let prior = env.get("WSLENV").cloned().unwrap_or_default();
            let joined = if prior.is_empty() {
                names.join(":")
            } else {
                format!("{prior}:{}", names.join(":"))
            };
            wenv.insert("WSLENV".into(), joined);
            for (k, v) in &remote_env {
                wenv.insert(k.clone(), v.clone());
            }
            command.extend(["--".to_string(), "sh".into(), "-c".into()]);
            command.push(format!("{}{}", shim_install(), final_exec(None)));
            command.push("omniget-env".into());
            command.extend(argv.iter().cloned());
            Wrapped {
                command,
                cwd: None,
                env: wenv,
            }
        }
    }
}

pub fn resolve_exec(exec: &str) -> String {
    let e = exec.trim();
    if e.contains('/') {
        return e.to_string();
    }
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    for d in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"] {
        dirs.push(PathBuf::from(d));
    }
    dirs.iter()
        .map(|d| d.join(e))
        .find(|p| p.is_file())
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| e.to_string())
}

pub fn preview(rec: &EnvRecord) -> String {
    let mut env = BTreeMap::new();
    env.insert("OMNIGET_BRIDGE_URL".into(), "http://127.0.0.1:PORT".into());
    env.insert("OMNIGET_TERMINAL_ID".into(), "T".into());
    let w = wrap(rec, &[], None, &env);
    let tail = w.command.last().cloned().unwrap_or_default();
    if w.command.first().map(String::as_str) == Some("/bin/sh") {
        return w.command[2]
            .lines()
            .rev()
            .find(|l| l.starts_with("exec "))
            .map(|l| l.trim_start_matches("exec ").to_string())
            .unwrap_or(tail);
    }
    w.command
        .iter()
        .map(|a| sh_quote(a))
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvTest {
    pub ok: bool,
    pub output: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<EnvCheck>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvCheck {
    pub name: String,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
}

const PROBE: &str = r#"pwd >/dev/null 2>&1 && echo "dir=ok $(pwd)" || echo "dir=fail"
[ -w "$HOME" ] && echo "home=ok $HOME" || echo "home=fail $HOME is not writable"
[ -x /bin/sh ] && echo "shell=ok" || echo "shell=fail no /bin/sh"
command -v bash >/dev/null 2>&1 && echo "bash=ok" || echo "bash=warn not installed"
(command -v curl >/dev/null 2>&1 || command -v wget >/dev/null 2>&1) && echo "curl=ok" || echo "curl=fail the omniget CLI needs curl or wget"
command -v tmux >/dev/null 2>&1 && echo "tmux=ok" || echo "tmux=warn not installed (no persistent sessions)"
"#;

fn parse_probe(text: &str) -> Vec<EnvCheck> {
    text.lines()
        .filter_map(|l| {
            let (name, rest) = l.split_once('=')?;
            let (state, detail) = rest.split_once(' ').unwrap_or((rest, ""));
            Some(EnvCheck {
                name: name.trim().to_string(),
                ok: state != "fail",
                detail: if state == "warn" {
                    format!("warning: {detail}")
                } else {
                    detail.to_string()
                },
            })
        })
        .collect()
}

fn bridge_probe(url: &str) -> String {
    let u = sh_quote(url);
    format!(
        "i=0; b=fail; while [ $i -lt 3 ]; do if command -v curl >/dev/null 2>&1; then c=$(curl -s -o /dev/null -m 4 -w '%{{http_code}}' {u} 2>/dev/null); [ -n \"$c\" ] && [ \"$c\" != 000 ] && b=ok; elif command -v wget >/dev/null 2>&1; then wget -q -T 4 -O /dev/null {u} >/dev/null 2>&1; r=$?; {{ [ $r -eq 0 ] || [ $r -eq 8 ]; }} && b=ok; else b=skip; fi; [ $b = fail ] || break; i=$((i+1)); sleep 1; done\ncase $b in ok) echo \"bridge=ok {url}\";; skip) echo \"bridge=warn no curl or wget to check {url}\";; *) echo \"bridge=fail the OmniGet bridge at {url} does not answer from there\";; esac\n",
        url = url.replace(['"', '`', '$', '\\'], "")
    )
}

pub async fn test_bridge(rec: &EnvRecord, bridge_port: Option<u16>) -> EnvTest {
    let host = rec.host.clone().unwrap_or_default();
    let cd = rec
        .dir
        .as_deref()
        .filter(|d| !d.trim().is_empty())
        .map(|d| {
            format!(
                "cd {} || {{ echo 'dir=fail no folder {}'; exit 3; }}\n",
                sh_quote(d),
                d.replace('\'', "")
            )
        })
        .unwrap_or_default();
    let bridge_host = match rec.kind {
        EnvKind::Ssh | EnvKind::Wsl => Some("127.0.0.1".to_string()),
        EnvKind::Docker | EnvKind::Sandbox => Some("host.docker.internal".to_string()),
        EnvKind::Custom => Some(
            rec.runtime_def
                .as_ref()
                .and_then(|d| d.bridge_host.clone())
                .filter(|h| !h.trim().is_empty())
                .unwrap_or_else(|| "host.containers.internal".into()),
        ),
        EnvKind::Local | EnvKind::Tmux => None,
    };
    let probe_bridge = match (bridge_port, &bridge_host) {
        (Some(p), Some(h)) => bridge_probe(&format!("http://{h}:{p}/")),
        _ => String::new(),
    };
    let script = format!("{cd}{PROBE}{probe_bridge}");
    let inv = match rec.kind {
        EnvKind::Local => {
            let ok = rec
                .dir
                .as_deref()
                .is_none_or(|d| std::path::Path::new(d).is_dir());
            return EnvTest {
                ok,
                output: rec.dir.clone().unwrap_or_else(|| "local".into()),
                checks: vec![],
            };
        }
        EnvKind::Tmux => Invocation::new(&resolve_exec("tmux")).arg("-V"),
        EnvKind::Ssh => {
            let mut i = Invocation::new("ssh").args([
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=8",
                "-o",
                "StrictHostKeyChecking=accept-new",
            ]);
            if let Some(o) = &rec.ssh_options {
                i = i.args(o.split_whitespace());
            }
            if let Some(p) = bridge_port {
                i = i
                    .arg("-R")
                    .arg(format!("{p}:127.0.0.1:{p}"))
                    .args(["-o", "ExitOnForwardFailure=no"]);
            }
            i.arg("--")
                .arg(&host)
                .arg(format!("sh -c {}", sh_quote(&script)))
        }
        EnvKind::Docker | EnvKind::Sandbox => {
            let exe = if rec.kind == EnvKind::Docker {
                resolve_exec("docker")
            } else {
                resolve_exec("sbx")
            };
            let running = Invocation::new(&exe)
                .args(["inspect", "-f", "{{.State.Running}}", &host])
                .timeout(Some(Duration::from_secs(10)))
                .unchecked()
                .run()
                .await;
            if rec.kind == EnvKind::Docker {
                match running {
                    Ok(o) if o.trimmed() == "true" => {}
                    Ok(o) if o.ok() => {
                        return EnvTest {
                            ok: false,
                            output: format!("the container {host} is not running"),
                            checks: vec![EnvCheck {
                                name: "running".into(),
                                ok: false,
                                detail: "start it with docker start".into(),
                            }],
                        }
                    }
                    Ok(o) => {
                        return EnvTest {
                            ok: false,
                            output: format!("{}{}", o.text(), o.err_text()).trim().to_string(),
                            checks: vec![],
                        }
                    }
                    Err(e) => {
                        return EnvTest {
                            ok: false,
                            output: format!("docker is not available: {e}"),
                            checks: vec![],
                        }
                    }
                }
            }
            let mut i = Invocation::new(&exe).arg("exec");
            if let Some(u) = rec.user.as_ref().filter(|u| !u.is_empty()) {
                i = i.args(["-u", u]);
            }
            i.args([host.as_str(), "sh", "-c", &script])
        }
        EnvKind::Custom => {
            let def = rec.runtime_def.clone().unwrap_or_default();
            if def.exec.trim().is_empty() {
                return EnvTest {
                    ok: false,
                    output: "no runtime definition".into(),
                    checks: vec![],
                };
            }
            let shell = def
                .shell
                .clone()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "/bin/sh".into());
            Invocation::new(&resolve_exec(&def.exec))
                .args(def.args.split_whitespace())
                .args([shell.as_str(), "-c", &script])
        }
        EnvKind::Wsl => {
            let mut i = Invocation::new("wsl.exe");
            if !host.is_empty() {
                i = i.args(["-d", &host]);
            }
            if let Some(u) = rec.user.as_ref().filter(|u| !u.is_empty()) {
                i = i.args(["-u", u]);
            }
            i.args(["--", "sh", "-c", &script])
        }
    };
    match inv
        .timeout(Some(Duration::from_secs(20)))
        .unchecked()
        .run()
        .await
    {
        Ok(o) if o.ok() => {
            let text = o.trimmed();
            let mut checks = parse_probe(&text);
            let err = o.err_text().to_ascii_lowercase();
            if let Some(c) = checks.iter_mut().find(|c| c.name == "bridge" && !c.ok) {
                if err.contains("remote port forwarding failed")
                    || err.contains("administratively prohibited")
                {
                    c.detail = "the SSH server refused the reverse tunnel for the bridge (AllowTcpForwarding no in its sshd_config?): the omniget CLI cannot reach the app from there".into();
                } else if rec.kind == EnvKind::Ssh {
                    c.detail.push_str(" (the reverse tunnel did not carry it: check AllowTcpForwarding on the server)");
                } else if matches!(
                    rec.kind,
                    EnvKind::Docker | EnvKind::Sandbox | EnvKind::Custom
                ) {
                    c.detail.push_str(" (the container cannot reach this machine: check the bridge host and the network mode)");
                }
            }
            let ok = checks.iter().all(|c| c.ok);
            let output = checks
                .iter()
                .find(|c| c.name == "dir")
                .map(|c| c.detail.clone())
                .unwrap_or(text);
            EnvTest { ok, output, checks }
        }
        Ok(o) => {
            let err = format!("{}{}", o.text(), o.err_text()).trim().to_string();
            EnvTest {
                ok: false,
                output: explain(rec.kind, &err),
                checks: parse_probe(&o.text()),
            }
        }
        Err(e) => EnvTest {
            ok: false,
            output: explain(rec.kind, &e.to_string()),
            checks: vec![],
        },
    }
}

fn explain(kind: EnvKind, err: &str) -> String {
    let e = err.to_ascii_lowercase();
    let hint = if e.contains("permission denied (publickey") {
        "the host refused the key: add yours with ssh-copy-id"
    } else if e.contains("host key verification failed")
        || e.contains("remote host identification has changed")
    {
        "the host key changed: check ~/.ssh/known_hosts before trusting it"
    } else if e.contains("could not resolve") || e.contains("no route") || e.contains("timed out") {
        "the host is not reachable from this machine"
    } else if e.contains("no such container") || e.contains("not running") {
        "the container is not running (start it first)"
    } else if e.contains("not found") || e.contains("no such file") {
        match kind {
            EnvKind::Custom => "the runtime executable was not found",
            EnvKind::Sandbox => "sbx is not installed",
            _ => "a command is missing",
        }
    } else {
        ""
    };
    if hint.is_empty() {
        err.to_string()
    } else {
        format!("{err}\n{hint}")
    }
}

fn missing_tool(program: &str, e: &crate::core::vcs::runner::VcsError) -> String {
    if e.code() == crate::core::vcs::runner::ERR_GIT_MISSING {
        format!("CANVAS_ENV_MISSING: {program} is not installed on this machine")
    } else {
        e.to_string()
    }
}

pub fn session_targets(store: &CanvasStore) -> Vec<EnvRecord> {
    let mut out: Vec<EnvRecord> = vec![EnvRecord {
        kind: EnvKind::Tmux,
        ..Default::default()
    }];
    let mut push = |rec: EnvRecord| {
        let rec = resolve(store, rec);
        let keeps = rec.kind == EnvKind::Tmux
            || (rec.kind.is_remote() && rec.kind != EnvKind::Wsl && rec.persistent == Some(true));
        if keeps && !out.iter().any(|x| target_label(x) == target_label(&rec)) {
            out.push(rec);
        }
    };
    for w in store.workspaces() {
        let ws = w.workspace.id.clone();
        push(load(store, &ws));
        for f in &w.workspace.floors {
            let recs: Vec<EnvRecord> = store
                .read(&ws, &f.id, |d| {
                    d.nodes
                        .iter()
                        .filter_map(|n| n.data.get("environment").and_then(node_override))
                        .collect()
                })
                .unwrap_or_default();
            for r in recs {
                push(r);
            }
        }
    }
    out
}

pub fn target_label(rec: &EnvRecord) -> String {
    let host = rec.host.clone().unwrap_or_default();
    match rec.kind {
        EnvKind::Local | EnvKind::Tmux => "local".into(),
        EnvKind::Custom => format!(
            "custom:{}{}",
            rec.runtime.clone().unwrap_or_default(),
            if host.is_empty() {
                String::new()
            } else {
                format!("/{host}")
            }
        ),
        k => format!("{}:{host}", k.as_str()),
    }
}

fn target_invocation(rec: &EnvRecord, script: &str) -> Result<(String, Invocation), String> {
    let host = rec.host.clone().unwrap_or_default();
    Ok(match rec.kind {
        EnvKind::Local | EnvKind::Tmux => {
            ("sh".into(), Invocation::new("/bin/sh").args(["-c", script]))
        }
        EnvKind::Ssh => {
            let mut i =
                Invocation::new("ssh").args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=6"]);
            if let Some(o) = &rec.ssh_options {
                i = i.args(o.split_whitespace());
            }
            (
                "ssh".into(),
                i.arg("--")
                    .arg(&host)
                    .arg(format!("sh -c {}", sh_quote(script))),
            )
        }
        EnvKind::Docker | EnvKind::Sandbox => {
            let name = if rec.kind == EnvKind::Docker {
                "docker"
            } else {
                "sbx"
            };
            let mut i = Invocation::new(&resolve_exec(name)).args(["exec", "-i"]);
            if let Some(u) = rec.user.as_ref().filter(|u| !u.is_empty()) {
                i = i.args(["-u", u]);
            }
            (name.into(), i.args([host.as_str(), "sh", "-c", script]))
        }
        EnvKind::Custom => {
            let def = rec
                .runtime_def
                .clone()
                .ok_or("CANVAS_INVALID: the custom runtime does not exist anymore")?;
            let shell = def
                .shell
                .clone()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "/bin/sh".into());
            (
                def.exec.clone(),
                Invocation::new(&resolve_exec(&def.exec))
                    .args(def.args.split_whitespace())
                    .args([shell.as_str(), "-c", script]),
            )
        }
        EnvKind::Wsl => {
            let mut i = Invocation::new("wsl.exe");
            if !host.is_empty() {
                i = i.args(["-d", &host]);
            }
            if let Some(u) = rec.user.as_ref().filter(|u| !u.is_empty()) {
                i = i.args(["-u", u]);
            }
            ("wsl".into(), i.args(["--", "sh", "-c", script]))
        }
    })
}

pub async fn run_script(
    rec: &EnvRecord,
    script: &str,
    stdin: Option<Vec<u8>>,
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    let (program, mut inv) = target_invocation(rec, script)?;
    if let Some(b) = stdin {
        inv = inv.stdin(b);
    }
    let o = inv
        .timeout(Some(timeout))
        .unchecked()
        .run()
        .await
        .map_err(|e| missing_tool(&program, &e))?;
    if o.ok() {
        Ok(o.stdout.clone())
    } else {
        Err(format!(
            "CANVAS_ENV_FAILED: {}",
            explain(rec.kind, o.err_text().trim())
        ))
    }
}

pub fn ssh_config_hosts() -> Vec<Value> {
    let Some(home) = dirs::home_dir() else {
        return vec![];
    };
    let ssh = home.join(".ssh");
    let mut files = vec![ssh.join("config")];
    let mut out: Vec<Value> = Vec::new();
    let mut seen_files = std::collections::HashSet::new();
    while let Some(f) = files.pop() {
        if !seen_files.insert(f.clone()) || seen_files.len() > 32 {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&f) else {
            continue;
        };
        let (hosts, includes) = parse_ssh_config(&text);
        for inc in includes {
            let inc = inc.replace('~', &home.to_string_lossy());
            let base = if inc.starts_with('/') {
                PathBuf::from(&inc)
            } else {
                ssh.join(&inc)
            };
            if inc.contains('*') {
                let dir = base
                    .parent()
                    .map(PathBuf::from)
                    .unwrap_or_else(|| ssh.clone());
                let pat = base
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let (pre, post) = pat.split_once('*').unwrap_or((&pat, ""));
                if let Ok(rd) = std::fs::read_dir(&dir) {
                    for e in rd.flatten() {
                        let n = e.file_name().to_string_lossy().into_owned();
                        if n.starts_with(pre) && n.ends_with(post) {
                            files.push(e.path());
                        }
                    }
                }
            } else {
                files.push(base);
            }
        }
        for h in hosts {
            if !out.iter().any(|x| x["host"] == h["host"]) {
                out.push(h);
            }
        }
    }
    out
}

pub fn parse_ssh_config(text: &str) -> (Vec<Value>, Vec<String>) {
    let mut hosts: Vec<Value> = Vec::new();
    let mut includes = Vec::new();
    let mut current: Vec<usize> = Vec::new();
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        let (k, v) = match l.split_once(|c: char| c.is_whitespace() || c == '=') {
            Some((k, v)) => (
                k.to_ascii_lowercase(),
                v.trim().trim_start_matches('=').trim(),
            ),
            None => continue,
        };
        match k.as_str() {
            "host" => {
                current.clear();
                for alias in v.split_whitespace() {
                    if alias.contains(['*', '?', '!']) {
                        continue;
                    }
                    current.push(hosts.len());
                    hosts.push(json!({ "host": alias }));
                }
            }
            "match" => current.clear(),
            "include" => includes.extend(v.split_whitespace().map(str::to_string)),
            "hostname" | "user" | "port" | "proxyjump" => {
                let key = match k.as_str() {
                    "hostname" => "hostName",
                    "proxyjump" => "proxyJump",
                    other => other,
                };
                for &i in &current {
                    if hosts[i].get(key).is_none() {
                        hosts[i][key] = json!(v);
                    }
                }
            }
            _ => {}
        }
    }
    (hosts, includes)
}

pub const DROPS_DIR: &str = "/tmp/omniget-drops";

pub async fn push_file(rec: &EnvRecord, local: &std::path::Path) -> Result<String, String> {
    match rec.kind {
        EnvKind::Local | EnvKind::Tmux => return Ok(local.to_string_lossy().into_owned()),
        EnvKind::Wsl => return Ok(wsl_path(&local.to_string_lossy())),
        _ => {}
    }
    let meta = std::fs::metadata(local).map_err(|e| format!("CANVAS_INVALID: {e}"))?;
    if !meta.is_file() {
        return Err("CANVAS_INVALID: only files can go to a remote terminal".into());
    }
    if meta.len() > 64 * 1024 * 1024 {
        return Err("CANVAS_INVALID: the file is larger than 64 MB".into());
    }
    let bytes = std::fs::read(local).map_err(|e| e.to_string())?;
    let name: String = local
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "drop".into())
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "._-".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    let name = name.trim_start_matches('.').to_string();
    let name = if name.is_empty() {
        "drop".to_string()
    } else {
        name
    };
    let remote = format!("{DROPS_DIR}/{name}");
    let script = format!(
        "mkdir -p {d} && chmod 1777 {d} 2>/dev/null; cat > {r}",
        d = DROPS_DIR,
        r = sh_quote(&remote)
    );
    let (program, inv) = target_invocation(rec, &script)?;
    let o = inv
        .stdin(bytes)
        .timeout(Some(Duration::from_secs(60)))
        .unchecked()
        .run()
        .await
        .map_err(|e| missing_tool(&program, &e))?;
    if o.ok() {
        Ok(remote)
    } else {
        Err(format!(
            "CANVAS_ENV_FAILED: {}",
            explain(rec.kind, o.err_text().trim())
        ))
    }
}

pub fn wsl_path(p: &str) -> String {
    let b = p.as_bytes();
    if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        let rest = p[2..].replace('\\', "/");
        return format!("/mnt/{}{}", (b[0] as char).to_ascii_lowercase(), rest);
    }
    p.replace('\\', "/")
}

pub async fn list_files(
    rec: &EnvRecord,
    dir: Option<&str>,
    query: &str,
) -> Result<(String, Vec<(String, bool)>), String> {
    let mut cands: Vec<String> = Vec::new();
    for d in [dir.map(str::to_string), rec.dir.clone()]
        .into_iter()
        .flatten()
    {
        let d = d.trim().to_string();
        if !d.is_empty() && !cands.contains(&d) {
            cands.push(d);
        }
    }
    let cd = cands
        .iter()
        .map(|d| format!("cd {} 2>/dev/null ||", sh_quote(d)))
        .collect::<Vec<_>>()
        .join(" ");
    let q: String = query
        .chars()
        .filter(|c| !matches!(c, '*' | '?' | '[' | ']' | '\\'))
        .take(80)
        .collect();
    let body = if q.trim().is_empty() {
        "ls -1Ap 2>/dev/null | head -200".to_string()
    } else {
        format!(
            "find . -maxdepth 5 \\( -name .git -o -name node_modules -o -name target \\) -prune -o -iname {} -print 2>/dev/null | head -80 | while IFS= read -r f; do if [ -d \"$f\" ]; then printf '%s/\\n' \"$f\"; else printf '%s\\n' \"$f\"; fi; done",
            sh_quote(&format!("*{}*", q.trim()))
        )
    };
    let script = format!("{cd} cd \"$HOME\"; pwd; {body}");
    let (program, inv) = target_invocation(rec, &script)?;
    let o = inv
        .timeout(Some(Duration::from_secs(10)))
        .unchecked()
        .run()
        .await
        .map_err(|e| missing_tool(&program, &e))?;
    if !o.ok() {
        return Err(format!(
            "CANVAS_ENV_FAILED: {}",
            explain(rec.kind, o.err_text().trim())
        ));
    }
    let text = o.text();
    let mut lines = text.lines();
    let root = lines.next().unwrap_or("").trim().to_string();
    let hits = lines
        .map(|l| l.trim_start_matches("./"))
        .filter(|l| !l.is_empty() && *l != "." && *l != "./")
        .map(|l| match l.strip_suffix('/') {
            Some(d) => (d.to_string(), true),
            None => (l.to_string(), false),
        })
        .collect();
    Ok((root, hits))
}

const LIST_SESSIONS: &str = "command -v tmux >/dev/null 2>&1 || { echo OMNIGET_NO_TMUX; exit 0; }\ntmux list-sessions -F '#{session_name}|#{session_created}|#{session_attached}|#{pane_current_command}' 2>/dev/null; exit 0";

pub async fn target_sessions(rec: &EnvRecord) -> Result<Vec<Value>, String> {
    let (program, inv) = target_invocation(rec, LIST_SESSIONS)?;
    let o = inv
        .timeout(Some(Duration::from_secs(10)))
        .unchecked()
        .run()
        .await
        .map_err(|e| missing_tool(&program, &e))?;
    if !o.ok() {
        return Err(explain(rec.kind, o.err_text().trim()));
    }
    let label = target_label(rec);
    Ok(o.text()
        .lines()
        .filter(|l| l.starts_with("omniget-"))
        .map(|l| {
            let p: Vec<&str> = l.splitn(4, '|').collect();
            json!({
                "session": p.first().copied().unwrap_or(""),
                "createdS": p.get(1).and_then(|s| s.parse::<u64>().ok()),
                "attached": p.get(2).map(|s| *s != "0").unwrap_or(false),
                "command": p.get(3).copied().unwrap_or(""),
                "env": label,
                "kind": rec.kind.as_str(),
                "host": rec.host.clone().unwrap_or_default(),
            })
        })
        .collect())
}

pub async fn sessions(store: &CanvasStore) -> Vec<Value> {
    let mut out = Vec::new();
    for rec in session_targets(store) {
        match target_sessions(&rec).await {
            Ok(rows) => out.extend(rows),
            Err(e) if rec.kind == EnvKind::Tmux => {
                tracing::debug!("[canvas] local tmux sessions: {e}");
            }
            Err(e) => out
                .push(json!({ "env": target_label(&rec), "kind": rec.kind.as_str(), "error": e })),
        }
    }
    out
}

pub async fn session_kill(
    store: &CanvasStore,
    session: &str,
    env: Option<&str>,
) -> Result<(), String> {
    if !session.starts_with("omniget-")
        || !session
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("CANVAS_INVALID: only omniget-* sessions".into());
    }
    let targets = session_targets(store);
    let mut chosen: Option<EnvRecord> = None;
    if let Some(label) = env.filter(|e| !e.is_empty()) {
        chosen = targets.iter().find(|r| target_label(r) == label).cloned();
        if chosen.is_none() {
            return Err(format!("CANVAS_NOT_FOUND: no environment '{label}' in use"));
        }
    } else {
        for rec in &targets {
            if let Ok(rows) = target_sessions(rec).await {
                if rows.iter().any(|r| r["session"] == session) {
                    chosen = Some(rec.clone());
                    break;
                }
            }
        }
    }
    let rec = chosen.ok_or_else(|| format!("CANVAS_NOT_FOUND: no session {session}"))?;
    let script = format!(
        "command -v tmux >/dev/null 2>&1 || {{ echo 'tmux is not installed there' >&2; exit 127; }}\ntmux kill-session -t {}",
        sh_quote(&format!("={session}"))
    );
    let (program, inv) = target_invocation(&rec, &script)?;
    let o = inv
        .timeout(Some(Duration::from_secs(10)))
        .unchecked()
        .run()
        .await
        .map_err(|e| missing_tool(&program, &e))?;
    if o.ok() {
        Ok(())
    } else if rec.kind == EnvKind::Tmux && o.err_text().contains("not installed") {
        Err("CANVAS_ENV_MISSING: tmux is not installed on this machine".into())
    } else {
        Err(format!("CANVAS_ENV_FAILED: {}", o.err_text().trim()))
    }
}
