//! Login: abre o terminal do sistema com o comando de login da ferramenta,
//! já apontado para a instância (conta) certa. O OmniGet nunca lê o que o
//! login grava.
//!
//! Mesmo desenho do `llm_accounts_login` (script + terminal da plataforma) e
//! o mesmo ambiente do runtime (`account_env` + `SCRUB_ENV`) quando há conta.
//! `Surface::Embedded` devolve só o que rodar, para o terminal embutido (PTY)
//! da rodada 2 abrir sem janela externa.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::table::CliTool;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    #[default]
    System,
    Embedded,
}

impl Surface {
    pub fn parse(s: Option<&str>) -> Surface {
        match s.map(|s| s.trim().to_ascii_lowercase()) {
            Some(s) if s == "embedded" || s == "pty" => Surface::Embedded,
            _ => Surface::System,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Launch {
    pub surface: Surface,
    /// O que roda dentro do terminal (binário + argumentos do login).
    pub command: String,
    pub command_args: Vec<String>,
    pub env: BTreeMap<String, String>,
    /// Variáveis que o terminal precisa apagar antes (as do runtime).
    pub unset: Vec<String>,
    /// Linha legível (`CLAUDE_CONFIG_DIR=… claude auth login`).
    pub display: String,
    /// Só em `System`: o script escrito e o programa que abre o terminal.
    pub script_path: Option<PathBuf>,
    pub script: Option<String>,
    pub program: Option<String>,
    pub args: Vec<String>,
    pub hint: Option<String>,
}

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Monta o lançamento (puro; quem escreve e abre é `open_system`).
pub fn login_launch(
    tool: &CliTool,
    bin: &str,
    env: BTreeMap<String, String>,
    unset: &[&str],
    tag: &str,
    script_dir: &Path,
    surface: Surface,
    have_wt: bool,
) -> Launch {
    let mut cmd_args = tool.login.clone();
    if cmd_args.is_empty() {
        cmd_args = tool.subcommand.clone();
    }
    let mut display = String::new();
    for (k, v) in &env {
        display.push_str(&format!("{k}={} ", sh_quote(v)));
    }
    display.push_str(&super::exec::display_command(bin, &cmd_args));
    let unset: Vec<String> = unset.iter().map(|s| s.to_string()).collect();
    let mut launch = Launch {
        surface,
        command: bin.to_string(),
        command_args: cmd_args.clone(),
        env: env.clone(),
        unset: unset.clone(),
        display,
        script_path: None,
        script: None,
        program: None,
        args: Vec::new(),
        hint: tool.login_hint.clone(),
    };
    if surface == Surface::Embedded {
        return launch;
    }
    let safe_tag: String = tag
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();

    if cfg!(target_os = "windows") {
        let script_path = script_dir.join(format!("login-{safe_tag}.cmd"));
        let mut script = String::from("@echo off\r\n");
        for (k, v) in &env {
            script.push_str(&format!("set \"{k}={v}\"\r\n"));
        }
        for k in &unset {
            script.push_str(&format!("set \"{k}=\"\r\n"));
        }
        script.push_str(&format!("title OmniGet - {} login\r\n", tool.name));
        let mut line = format!("\"{bin}\"");
        for a in &cmd_args {
            line.push_str(&format!(" \"{a}\""));
        }
        script.push_str(&line);
        script.push_str("\r\n");
        let sp = script_path.to_string_lossy().into_owned();
        let (program, args) = if have_wt {
            ("wt".to_string(), vec!["cmd".into(), "/K".into(), sp])
        } else {
            (
                "cmd".to_string(),
                vec![
                    "/C".into(),
                    "start".into(),
                    String::new(),
                    "cmd".into(),
                    "/K".into(),
                    sp,
                ],
            )
        };
        launch.script_path = Some(script_path);
        launch.script = Some(script);
        launch.program = Some(program);
        launch.args = args;
        return launch;
    }

    let ext = if cfg!(target_os = "macos") {
        "command"
    } else {
        "sh"
    };
    let script_path = script_dir.join(format!("login-{safe_tag}.{ext}"));
    let mut script = String::from(
        "#!/bin/sh\n\
         # Aberto pelo OmniGet para a ferramenta fazer o próprio login.\n\
         # O OmniGet nunca lê a credencial que este login grava.\n",
    );
    for (k, v) in &env {
        script.push_str(&format!("export {k}={}\n", sh_quote(v)));
    }
    if !unset.is_empty() {
        script.push_str(&format!("unset {}\n", unset.join(" ")));
    }
    if let Some(h) = &tool.login_hint {
        script.push_str(&format!("echo {}\n", sh_quote(h)));
    }
    let mut line = sh_quote(bin);
    for a in &cmd_args {
        line.push(' ');
        line.push_str(&sh_quote(a));
    }
    script.push_str(&format!("exec {line}\n"));
    let sp = script_path.to_string_lossy().into_owned();
    let (program, args) = if cfg!(target_os = "macos") {
        ("open".to_string(), vec!["-a".into(), "Terminal".into(), sp])
    } else {
        (
            "x-terminal-emulator".to_string(),
            vec!["-e".into(), "sh".into(), sp],
        )
    };
    launch.script_path = Some(script_path);
    launch.script = Some(script);
    launch.program = Some(program);
    launch.args = args;
    launch
}

/// Escreve o script e abre o terminal do sistema.
pub fn open_system(launch: &Launch) -> Result<(), String> {
    let (Some(path), Some(script), Some(program)) =
        (&launch.script_path, &launch.script, &launch.program)
    else {
        return Err("lançamento sem script".into());
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, script).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755));
    }
    crate::core::process::std_command(Path::new(program))
        .args(&launch.args)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::clitools::table;

    #[test]
    fn script_exporta_a_instancia_e_roda_o_login() {
        let t = table::get("codex").unwrap();
        let mut env = BTreeMap::new();
        env.insert("CODEX_HOME".to_string(), "/tmp/p x".to_string());
        let l = login_launch(
            t,
            "codex",
            env,
            &["OPENAI_API_KEY"],
            "codex-work",
            Path::new("/tmp"),
            Surface::System,
            false,
        );
        let s = l.script.clone().unwrap();
        if !cfg!(windows) {
            assert!(s.contains("export CODEX_HOME='/tmp/p x'"));
            assert!(s.contains("unset OPENAI_API_KEY"));
            assert!(s.contains("exec 'codex' 'login'"));
            assert!(!s.to_lowercase().contains("auth.json"));
        }
        assert_eq!(l.command_args, vec!["login"]);
    }

    #[test]
    fn embutido_nao_escreve_script() {
        let t = table::get("gemini").unwrap();
        let l = login_launch(
            t,
            "gemini",
            BTreeMap::new(),
            &[],
            "g",
            Path::new("/tmp"),
            Surface::Embedded,
            false,
        );
        assert!(l.script.is_none());
        assert_eq!(l.command, "gemini");
        assert!(l.hint.is_some());
    }
}
