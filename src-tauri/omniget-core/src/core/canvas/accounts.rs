use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use crate::core::llm::cli_runtime::accounts::{account_env, CliAccount, CliKind, CREDENTIAL_SCRUB};

pub const ERR_ACCOUNT: &str = "CANVAS_ACCOUNT";
pub const CONFIG_VARS: [&str; 2] = ["CLAUDE_CONFIG_DIR", "CODEX_HOME"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeAccount {
    Environment,
    Account(CliAccount),
}

impl NodeAccount {
    pub fn id(&self) -> Option<&str> {
        match self {
            NodeAccount::Environment => None,
            NodeAccount::Account(a) => Some(a.id.as_str()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AccountRef {
    pub id: String,
    pub label: String,
    pub cli: String,
    pub disabled: bool,
}

impl From<&CliAccount> for AccountRef {
    fn from(a: &CliAccount) -> Self {
        Self {
            id: a.id.clone(),
            label: a.label.clone(),
            cli: a.cli.as_str().to_string(),
            disabled: a.disabled,
        }
    }
}

pub fn account_of(data: &Value) -> Option<String> {
    data.get("account")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub fn cli_of_agent(agent: &str) -> Option<CliKind> {
    match agent {
        "claude" => Some(CliKind::Claude),
        "codex" => Some(CliKind::Codex),
        _ => None,
    }
}

pub fn takes_account(agent: &str) -> bool {
    matches!(agent, "claude" | "codex" | "shell")
}

pub fn resolve(
    accounts: &[CliAccount],
    id: Option<&str>,
    agent: &str,
) -> Result<NodeAccount, String> {
    let Some(id) = id.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(NodeAccount::Environment);
    };
    let Some(acc) = accounts.iter().find(|a| a.id == id) else {
        return Err(format!(
            "{ERR_ACCOUNT}: the account '{id}' no longer exists; pick another account or the environment"
        ));
    };
    if acc.disabled {
        return Err(format!(
            "{ERR_ACCOUNT}: the account '{}' is disabled; enable it or pick another",
            acc.label
        ));
    }
    if !takes_account(agent) {
        return Err(format!(
            "{ERR_ACCOUNT}: accounts work with Claude, Codex and shell terminals, not {agent}"
        ));
    }
    if let Some(cli) = cli_of_agent(agent) {
        if acc.cli != cli {
            return Err(format!(
                "{ERR_ACCOUNT}: '{}' is a {} account and this terminal runs {agent}",
                acc.label, acc.cli
            ));
        }
    }
    Ok(NodeAccount::Account(acc.clone()))
}

pub fn legacy_account(
    accounts: &[CliAccount],
    env: Option<&Value>,
    agent: &str,
) -> Option<CliAccount> {
    let env = env?.as_object()?;
    let cli = cli_of_agent(agent)?;
    let dir = env.get(cli.config_env())?.as_str()?.trim();
    if dir.is_empty() {
        return None;
    }
    accounts
        .iter()
        .find(|a| a.cli == cli && !a.disabled && a.config_dir == Path::new(dir))
        .cloned()
}

pub fn strip_config_vars(env: &mut HashMap<String, String>) {
    for v in CONFIG_VARS {
        env.remove(v);
    }
}

pub fn merge_remove(into: &mut Vec<String>, more: impl IntoIterator<Item = String>) {
    for k in more {
        if !into.contains(&k) {
            into.push(k);
        }
    }
}

pub fn apply(acc: &NodeAccount, env: &mut HashMap<String, String>, remove: &mut Vec<String>) {
    let NodeAccount::Account(a) = acc else {
        return;
    };
    for k in CREDENTIAL_SCRUB {
        env.remove(*k);
    }
    merge_remove(remove, CREDENTIAL_SCRUB.iter().map(|k| k.to_string()));
    let own = account_env(a);
    remove.retain(|k| !own.contains_key(k));
    env.extend(own);
}

pub fn config_dir(acc: &NodeAccount, agent: &str) -> Option<PathBuf> {
    match acc {
        NodeAccount::Account(a) if !a.config_dir.as_os_str().is_empty() => {
            Some(a.config_dir.clone())
        }
        _ => match cli_of_agent(agent)? {
            CliKind::Claude => super::adapters::claude::default_config_dir(),
            CliKind::Codex => super::adapters::codex::default_home(),
        },
    }
}

pub fn session_in(agent: &str, sid: &str, dir: Option<&Path>) -> bool {
    match agent {
        "claude" => super::adapters::claude::session_exists_in(sid, dir),
        "codex" => super::adapters::codex::session_exists_in(sid, dir),
        _ => false,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resume {
    Yes,
    Fresh { previous: Option<Option<String>> },
}

pub fn decide(
    agent: &str,
    current: Option<&str>,
    remembered: Option<Option<String>>,
    here: bool,
    elsewhere: Option<Option<String>>,
) -> Resume {
    if let Some(prev) = remembered {
        if prev.as_deref() != current {
            return Resume::Fresh {
                previous: Some(prev),
            };
        }
    }
    if here {
        return Resume::Yes;
    }
    if let Some(other) = elsewhere.filter(|o| o.as_deref() != current) {
        return Resume::Fresh {
            previous: Some(other),
        };
    }
    if agent == "codex" && current.is_none() {
        return Resume::Yes;
    }
    Resume::Fresh { previous: None }
}

pub fn locate_session(
    accounts: &[CliAccount],
    agent: &str,
    sid: &str,
    current: Option<&str>,
) -> Option<Option<String>> {
    let cli = cli_of_agent(agent)?;
    if current.is_some()
        && session_in(
            agent,
            sid,
            config_dir(&NodeAccount::Environment, agent).as_deref(),
        )
    {
        return Some(None);
    }
    accounts
        .iter()
        .filter(|a| a.cli == cli && Some(a.id.as_str()) != current)
        .filter(|a| !a.config_dir.as_os_str().is_empty())
        .find(|a| session_in(agent, sid, Some(&a.config_dir)))
        .map(|a| Some(a.id.clone()))
}

fn marker_path(tid: &str) -> Option<PathBuf> {
    crate::core::term::session::history::path_for(tid).map(|p| p.with_extension("account"))
}

pub fn remember(tid: &str, account: Option<&str>) {
    let Some(p) = marker_path(tid) else { return };
    let body = account.unwrap_or("");
    if std::fs::read_to_string(&p).ok().as_deref() == Some(body) {
        return;
    }
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(&p, body);
}

pub fn remembered(tid: &str) -> Option<Option<String>> {
    let s = std::fs::read_to_string(marker_path(tid)?).ok()?;
    let s = s.trim();
    Some((!s.is_empty()).then(|| s.to_string()))
}

pub fn match_label<'a, T>(
    items: &'a [T],
    key: &str,
    id: fn(&T) -> &str,
    label: fn(&T) -> &str,
) -> Result<&'a T, Vec<&'a T>> {
    let key = key.trim();
    if let Some(x) = items.iter().find(|x| id(x) == key) {
        return Ok(x);
    }
    let hits: Vec<&T> = items
        .iter()
        .filter(|x| label(x).trim().eq_ignore_ascii_case(key))
        .collect();
    if hits.len() == 1 {
        Ok(hits[0])
    } else {
        Err(hits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::llm::cli_runtime::accounts::SandboxMode;

    fn acc(id: &str, cli: CliKind, dir: &Path) -> CliAccount {
        CliAccount {
            id: id.into(),
            cli,
            config_dir: dir.join(id),
            label: id.to_uppercase(),
            disabled: false,
            sandbox: SandboxMode::Full,
        }
    }

    #[test]
    fn no_account_means_the_environment() {
        let r = resolve(&[], None, "claude").unwrap();
        assert_eq!(r, NodeAccount::Environment);
        assert_eq!(
            resolve(&[], Some("  "), "claude").unwrap(),
            NodeAccount::Environment
        );
    }

    #[test]
    fn a_missing_disabled_or_foreign_account_fails_closed() {
        let root = Path::new("/tmp/omniget-canvas-accounts");
        let mut off = acc("off", CliKind::Claude, root);
        off.disabled = true;
        let list = vec![
            acc("work", CliKind::Claude, root),
            acc("cx", CliKind::Codex, root),
            off,
        ];
        assert!(resolve(&list, Some("gone"), "claude")
            .unwrap_err()
            .starts_with(ERR_ACCOUNT));
        assert!(resolve(&list, Some("off"), "claude")
            .unwrap_err()
            .contains("disabled"));
        assert!(resolve(&list, Some("cx"), "claude")
            .unwrap_err()
            .contains("codex account"));
        assert!(resolve(&list, Some("work"), "gemini").is_err());
        assert_eq!(
            resolve(&list, Some("work"), "claude").unwrap().id(),
            Some("work")
        );
        assert_eq!(
            resolve(&list, Some("cx"), "codex").unwrap().id(),
            Some("cx")
        );
        assert_eq!(
            resolve(&list, Some("work"), "shell").unwrap().id(),
            Some("work")
        );
    }

    #[test]
    fn the_account_env_goes_on_top_and_credentials_are_scrubbed() {
        let a = acc("work", CliKind::Claude, Path::new("/tmp/p"));
        let mut env = HashMap::from([
            ("ANTHROPIC_API_KEY".to_string(), "x".to_string()),
            ("CLAUDE_CONFIG_DIR".to_string(), "/elsewhere".to_string()),
            ("OMNIGET_TOKEN".to_string(), "keep".to_string()),
            ("OMNIGET_BRIDGE_URL".to_string(), "keep".to_string()),
        ]);
        strip_config_vars(&mut env);
        let mut remove = vec!["CLAUDECODE".to_string()];
        apply(&NodeAccount::Account(a), &mut env, &mut remove);
        assert_eq!(
            env.get("CLAUDE_CONFIG_DIR").map(String::as_str),
            Some(Path::new("/tmp/p").join("work").to_str().unwrap())
        );
        assert!(!env.contains_key("ANTHROPIC_API_KEY"));
        assert_eq!(env.get("OMNIGET_TOKEN").map(String::as_str), Some("keep"));
        assert!(remove.contains(&"CLAUDECODE".to_string()));
        assert!(remove.contains(&"ANTHROPIC_API_KEY".to_string()));
        assert!(remove.contains(&"OPENAI_API_KEY".to_string()));
        assert!(!remove.iter().any(|k| k.starts_with("OMNIGET_")));
        assert!(!remove.contains(&"CLAUDE_CONFIG_DIR".to_string()));
    }

    #[test]
    fn the_environment_keeps_the_env_but_not_the_config_override() {
        let mut env = HashMap::from([
            ("CODEX_HOME".to_string(), "/x".to_string()),
            ("ANTHROPIC_API_KEY".to_string(), "x".to_string()),
        ]);
        strip_config_vars(&mut env);
        let mut remove = Vec::new();
        apply(&NodeAccount::Environment, &mut env, &mut remove);
        assert!(!env.contains_key("CODEX_HOME"));
        assert!(env.contains_key("ANTHROPIC_API_KEY"));
        assert!(remove.is_empty());
    }

    #[test]
    fn credential_scrub_is_part_of_scrub_env_and_leaves_omniget_alone() {
        use crate::core::llm::cli_runtime::accounts::SCRUB_ENV;
        for k in CREDENTIAL_SCRUB {
            assert!(SCRUB_ENV.contains(k));
            assert!(!k.starts_with("OMNIGET_"));
        }
    }

    #[test]
    fn merge_remove_keeps_both_lists_once() {
        let mut a = vec!["A".to_string(), "B".to_string()];
        merge_remove(&mut a, ["B".to_string(), "C".to_string()]);
        assert_eq!(a, vec!["A", "B", "C"]);
    }

    #[test]
    fn a_legacy_config_dir_in_env_maps_to_its_account() {
        let root = Path::new("/tmp/legacy");
        let list = vec![acc("work", CliKind::Claude, root)];
        let env = serde_json::json!({"CLAUDE_CONFIG_DIR": root.join("work").to_str().unwrap()});
        assert_eq!(
            legacy_account(&list, Some(&env), "claude").map(|a| a.id),
            Some("work".to_string())
        );
        assert!(legacy_account(&list, Some(&env), "codex").is_none());
        let other = serde_json::json!({"CLAUDE_CONFIG_DIR": "/nope"});
        assert!(legacy_account(&list, Some(&other), "claude").is_none());
    }

    #[test]
    fn resume_only_in_the_account_that_holds_the_conversation() {
        assert_eq!(
            decide("claude", Some("a"), Some(Some("a".into())), true, None),
            Resume::Yes
        );
        assert_eq!(
            decide("claude", Some("b"), Some(Some("a".into())), false, None),
            Resume::Fresh {
                previous: Some(Some("a".into()))
            }
        );
        assert_eq!(
            decide("claude", None, Some(Some("a".into())), true, None),
            Resume::Fresh {
                previous: Some(Some("a".into()))
            }
        );
        assert_eq!(
            decide("claude", Some("a"), None, false, Some(None)),
            Resume::Fresh {
                previous: Some(None)
            }
        );
        assert_eq!(
            decide("claude", Some("a"), None, false, None),
            Resume::Fresh { previous: None }
        );
        assert_eq!(decide("codex", None, None, false, None), Resume::Yes);
        assert_eq!(
            decide("codex", Some("a"), None, false, None),
            Resume::Fresh { previous: None }
        );
    }

    #[test]
    fn sessions_are_looked_up_in_the_account_dir() {
        let root = tempfile::tempdir().unwrap();
        let claude = root.path().join("c");
        std::fs::create_dir_all(claude.join("projects").join("-tmp-x")).unwrap();
        std::fs::write(
            claude.join("projects").join("-tmp-x").join("abc-1.jsonl"),
            "{}",
        )
        .unwrap();
        assert!(session_in("claude", "abc-1", Some(&claude)));
        assert!(!session_in("claude", "abc-2", Some(&claude)));
        assert!(!session_in("claude", "../abc-1", Some(&claude)));
        let codex = root.path().join("x");
        let day = codex.join("sessions").join("2026").join("10").join("01");
        std::fs::create_dir_all(&day).unwrap();
        std::fs::write(
            day.join("rollout-2026-10-01T10-00-00-0199-abcd.jsonl"),
            "{}",
        )
        .unwrap();
        assert!(session_in("codex", "0199-abcd", Some(&codex)));
        assert!(!session_in("codex", "0199-ffff", Some(&codex)));
    }

    #[test]
    fn labels_match_exactly_by_id_or_uniquely_by_label() {
        let items = vec![("a1", "Work"), ("a2", "Personal"), ("a3", "work ")];
        fn id<'a>(x: &'a (&'static str, &'static str)) -> &'a str {
            x.0
        }
        fn label<'a>(x: &'a (&'static str, &'static str)) -> &'a str {
            x.1
        }
        assert_eq!(match_label(&items, "a2", id, label).unwrap().0, "a2");
        assert_eq!(match_label(&items, "personal", id, label).unwrap().0, "a2");
        assert_eq!(match_label(&items, "WORK", id, label).unwrap_err().len(), 2);
        assert!(match_label(&items, "nope", id, label)
            .unwrap_err()
            .is_empty());
    }
}
