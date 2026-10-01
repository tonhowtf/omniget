use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::state::AgentKind;

#[derive(Debug, Clone, Default)]
pub struct LaunchCtx {
    pub terminal_id: String,
    pub env: HashMap<String, String>,
    pub hook_shim: Option<PathBuf>,
    pub data_dir: Option<PathBuf>,
    pub work_dir: PathBuf,
    pub codex_notify: Option<Vec<String>>,
    pub args: Vec<String>,
    pub shell: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Launch {
    pub command: Vec<String>,
    pub env: HashMap<String, String>,
    pub files: Vec<PathBuf>,
}

const CLAUDE_EVENTS: [&str; 12] = [
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "Notification",
    "Stop",
    "StopFailure",
    "SessionStart",
    "SessionEnd",
    "PreCompact",
    "PostCompact",
    "SubagentStart",
    "SubagentStop",
];

fn quote(p: &str) -> String {
    if cfg!(windows) {
        format!("\"{}\"", p.replace('"', "\\\""))
    } else {
        format!("'{}'", p.replace('\'', "'\\''"))
    }
}

pub fn claude_settings(ctx: &LaunchCtx) -> Value {
    let mut settings = json!({
        "terminalProgressBarEnabled": true,
        "preferredNotifChannel": "iterm2_with_bell",
    });
    if let Some(shim) = &ctx.hook_shim {
        let mut hooks = serde_json::Map::new();
        for ev in CLAUDE_EVENTS {
            let mut cmd = format!(
                "{} --tool claude --event {ev} --observe --as claude",
                quote(&shim.to_string_lossy())
            );
            if let Some(dir) = &ctx.data_dir {
                cmd.push_str(&format!(" --data-dir {}", quote(&dir.to_string_lossy())));
            }
            let mut group =
                json!({ "hooks": [{ "type": "command", "command": cmd, "timeout": 10 }] });
            if matches!(ev, "PreToolUse" | "PostToolUse") {
                group["matcher"] = json!("*");
            }
            hooks.insert(ev.to_string(), json!([group]));
        }
        settings["hooks"] = Value::Object(hooks);
    }
    settings
}

fn write(path: &Path, body: &str, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, body)?;
    files.push(path.to_path_buf());
    Ok(())
}

pub fn recipe(kind: AgentKind, ctx: &LaunchCtx) -> std::io::Result<Launch> {
    let mut env = ctx.env.clone();
    env.insert("OMNIGET_TERMINAL_ID".into(), ctx.terminal_id.clone());
    let mut files = Vec::new();
    let command = match kind {
        AgentKind::Claude => {
            let path = ctx.work_dir.join("claude-settings.json");
            let body = serde_json::to_string_pretty(&claude_settings(ctx)).unwrap_or_default();
            write(&path, &body, &mut files)?;
            let mut c = vec![
                "claude".to_string(),
                "--settings".into(),
                path.to_string_lossy().into_owned(),
            ];
            c.extend(ctx.args.iter().cloned());
            c
        }
        AgentKind::Codex => {
            let mut c = vec![
                "codex".to_string(),
                "-c".into(),
                "tui.notification_method=\"osc9\"".into(),
                "-c".into(),
                "tui.notification_condition=\"always\"".into(),
            ];
            if let Some(notify) = ctx.codex_notify.as_ref().filter(|n| !n.is_empty()) {
                let arr = notify
                    .iter()
                    .map(|a| serde_json::to_string(a).unwrap_or_default())
                    .collect::<Vec<_>>()
                    .join(",");
                c.push("-c".into());
                c.push(format!("notify=[{arr}]"));
            }
            c.extend(ctx.args.iter().cloned());
            c
        }
        AgentKind::Gemini => [vec!["gemini".to_string()], ctx.args.clone()].concat(),
        AgentKind::Opencode => [vec!["opencode".to_string()], ctx.args.clone()].concat(),
        AgentKind::Ollama => [vec!["ollama".to_string()], ctx.args.clone()].concat(),
        AgentKind::Native => ctx.args.clone(),
        AgentKind::Shell | AgentKind::Unknown => shell_integration(ctx, &mut env, &mut files)?,
    };
    Ok(Launch {
        command,
        env,
        files,
    })
}

const ZSH_HOOKS: &str = r#"
__omniget_precmd() {
  local s=$?
  if [ -n "$__omniget_ran" ]; then printf '\e]133;D;%s\a' "$s"; fi
  __omniget_ran=
  printf '\e]7;file://%s%s\a' "${HOST:-localhost}" "${PWD// /%20}"
  printf '\e]133;A\a'
}
__omniget_preexec() { __omniget_ran=1; printf '\e]133;C\a'; }
autoload -Uz add-zsh-hook
add-zsh-hook precmd __omniget_precmd
add-zsh-hook preexec __omniget_preexec
"#;

const ZSH_PATH: &str = r#"
__omniget_path() {
  [ -n "$OMNIGET_CLI" ] || return 0
  local d="${OMNIGET_CLI:h}"
  path=("$d" "${(@)path:#$d}")
}
__omniget_path_once() {
  __omniget_path
  add-zsh-hook -d precmd __omniget_path_once
}
__omniget_path
add-zsh-hook precmd __omniget_path_once
"#;

pub const POSIX_PATH_FIX: &str = "if [ -n \"$OMNIGET_CLI\" ]; then __og_d=\"${OMNIGET_CLI%/*}\"; __og_r=\"$PATH\"; __og_p=; while [ -n \"$__og_r\" ]; do __og_e=\"${__og_r%%:*}\"; case \"$__og_r\" in *:*) __og_r=\"${__og_r#*:}\";; *) __og_r=;; esac; [ \"$__og_e\" = \"$__og_d\" ] || __og_p=\"$__og_p${__og_p:+:}$__og_e\"; done; PATH=\"$__og_d${__og_p:+:}$__og_p\"; export PATH; unset __og_d __og_r __og_p __og_e; fi";

const BASH_RC: &str = r#"[ -f ~/.bashrc ] && . ~/.bashrc
__omniget_path() {
  [ -n "$OMNIGET_CLI" ] || return 0
  local d="${OMNIGET_CLI%/*}" r="$PATH" p="" e
  while [ -n "$r" ]; do
    e="${r%%:*}"
    case "$r" in *:*) r="${r#*:}" ;; *) r="" ;; esac
    [ "$e" = "$d" ] || p="$p${p:+:}$e"
  done
  PATH="$d${p:+:}$p"
  export PATH
}
__omniget_path
__omniget_prompt() {
  local s=$?
  if [ -z "$__omniget_path_done" ]; then __omniget_path_done=1; __omniget_path; fi
  if [ -n "$__omniget_ran" ]; then printf '\e]133;D;%s\a' "$s"; fi
  __omniget_ran=
  printf '\e]7;file://%s%s\a' "${HOSTNAME:-localhost}" "${PWD// /%20}"
  printf '\e]133;A\a'
}
__omniget_debug() {
  [ -n "$COMP_LINE" ] && return
  [ -n "$__omniget_ran" ] && return
  case "$BASH_COMMAND" in __omniget_prompt*) return;; esac
  __omniget_ran=1
  printf '\e]133;C\a'
}
PROMPT_COMMAND="__omniget_prompt${PROMPT_COMMAND:+;$PROMPT_COMMAND}"
trap '__omniget_debug' DEBUG
"#;

const FISH_INIT: &str = "if set -q OMNIGET_CLI; set -l __og_d (string replace -r '/[^/]*$' '' -- $OMNIGET_CLI); set -gx PATH $__og_d (string match -v -- $__og_d $PATH); end; function __omniget_pre --on-event fish_preexec; printf '\\e]133;C\\a'; end; function __omniget_post --on-event fish_postexec; printf '\\e]133;D;%s\\a' $status; end; function __omniget_prompt --on-event fish_prompt; printf '\\e]7;file://%s%s\\a' (hostname) (string replace -a ' ' '%20' $PWD); printf '\\e]133;A\\a'; end";

fn zsh_passthrough(name: &str, ours: &Path) -> String {
    format!(
        "__omniget_zd=\"${{OMNIGET_USER_ZDOTDIR:-$HOME}}\"\n\
         [ -f \"$__omniget_zd/{name}\" ] && ZDOTDIR=\"$__omniget_zd\" . \"$__omniget_zd/{name}\"\n\
         ZDOTDIR={}\n",
        quote(&ours.to_string_lossy())
    )
}

fn shell_integration(
    ctx: &LaunchCtx,
    env: &mut HashMap<String, String>,
    files: &mut Vec<PathBuf>,
) -> std::io::Result<Vec<String>> {
    let shell = ctx
        .shell
        .clone()
        .or_else(|| std::env::var("SHELL").ok())
        .filter(|s| !s.is_empty());
    let Some(shell) = shell else {
        return Ok(Vec::new());
    };
    let name = super::super::session::foreground::label_for(&shell);
    match name.as_str() {
        "zsh" => {
            let dir = ctx.work_dir.join("zsh");
            let user_zd = std::env::var("ZDOTDIR")
                .ok()
                .filter(|s| !s.is_empty())
                .or_else(|| dirs::home_dir().map(|h| h.to_string_lossy().into_owned()))
                .unwrap_or_default();
            env.insert("OMNIGET_USER_ZDOTDIR".into(), user_zd);
            env.insert("ZDOTDIR".into(), dir.to_string_lossy().into_owned());
            write(
                &dir.join(".zshenv"),
                &zsh_passthrough(".zshenv", &dir),
                files,
            )?;
            write(
                &dir.join(".zprofile"),
                &zsh_passthrough(".zprofile", &dir),
                files,
            )?;
            let rc = format!(
                "__omniget_zd=\"${{OMNIGET_USER_ZDOTDIR:-$HOME}}\"\n\
                 ZDOTDIR=\"$__omniget_zd\"\n\
                 [ -f \"$__omniget_zd/.zshrc\" ] && . \"$__omniget_zd/.zshrc\"\n\
                 {ZSH_HOOKS}\n\
                 {ZSH_PATH}\n\
                 if [ \"$__omniget_zd\" = \"$HOME\" ]; then unset ZDOTDIR; else ZDOTDIR=\"$__omniget_zd\"; fi\n\
                 unset __omniget_zd\n"
            );
            write(&dir.join(".zshrc"), &rc, files)?;
            Ok(vec![shell, "-l".into(), "-i".into()])
        }
        "bash" => {
            let rc = ctx.work_dir.join("bashrc");
            write(&rc, BASH_RC, files)?;
            Ok(vec![
                shell,
                "--rcfile".into(),
                rc.to_string_lossy().into_owned(),
                "-i".into(),
            ])
        }
        "fish" => Ok(vec![shell, "-l".into(), "-C".into(), FISH_INIT.into()]),
        _ => Ok(Vec::new()),
    }
}
