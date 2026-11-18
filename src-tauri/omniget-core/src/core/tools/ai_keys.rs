use std::sync::{Mutex, OnceLock};

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

use crate::core::secrets::{self, AI_KEYS};

#[path = "ai_keys_migrate.rs"]
pub mod migrate;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KeyEntry {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub base_url: String,
    #[serde(default, skip_serializing)]
    pub key: String,
    #[serde(default, skip_serializing)]
    pub access_token: String,
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub created: i64,
    #[serde(default)]
    pub last_ok: Option<bool>,
    #[serde(default)]
    pub last_checked: Option<i64>,
    #[serde(default)]
    pub balance: Option<String>,
    #[serde(default)]
    pub models: usize,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct KeyView {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub base_url: String,
    pub key_hint: String,
    pub has_key: bool,
    pub has_access_token: bool,
    pub user_id: String,
    pub model: String,
    pub notes: String,
    pub created: i64,
    pub last_ok: Option<bool>,
    pub last_checked: Option<i64>,
    pub balance: Option<String>,
    pub models: usize,
    pub error: Option<String>,
}

impl KeyEntry {
    fn view(&self) -> KeyView {
        KeyView {
            id: self.id.clone(),
            name: self.name.clone(),
            kind: self.kind.clone(),
            base_url: self.base_url.clone(),
            key_hint: hint(&self.key),
            has_key: !self.key.is_empty(),
            has_access_token: !self.access_token.is_empty(),
            user_id: self.user_id.clone(),
            model: self.model.clone(),
            notes: self.notes.clone(),
            created: self.created,
            last_ok: self.last_ok,
            last_checked: self.last_checked,
            balance: self.balance.clone(),
            models: self.models,
            error: self.error.clone(),
        }
    }
}

pub fn hint(key: &str) -> String {
    let n = key.chars().count();
    if n == 0 {
        return String::new();
    }
    if n <= 8 {
        return "•".repeat(n);
    }
    let start: String = key.chars().take(4).collect();
    let end: String = key.chars().skip(n - 4).collect();
    format!("{}…{}", start, end)
}

#[derive(Debug, Clone, Serialize)]
pub struct Kind {
    pub id: &'static str,
    pub name: &'static str,
    pub base_url: &'static str,
    pub balance: bool,
    pub env: &'static str,
    pub wire: &'static str,
    pub streaming: bool,
    pub tools: bool,
}

impl Kind {
    pub fn base_url_default(&self) -> &'static str {
        self.base_url
    }

    pub fn needs_key(&self) -> bool {
        !self.env.is_empty() && self.id != "custom"
    }

    pub fn accepts_key(&self) -> bool {
        !self.env.is_empty()
    }

    pub fn base_url_editable(&self) -> bool {
        matches!(self.id, "ollama" | "custom")
            || url::Url::parse(self.base_url)
                .map(|u| u.host_str() == Some("seu-site.com"))
                .unwrap_or(true)
    }

    pub fn env_var(&self) -> &'static str {
        self.env
    }

    pub fn supports_streaming(&self) -> bool {
        self.streaming
    }

    pub fn supports_tools(&self) -> bool {
        self.tools
    }

    pub fn is_openai_wire(&self) -> bool {
        self.wire == "openai"
    }
}

pub const KINDS: &[Kind] = &[
    Kind {
        id: "openai",
        name: "OpenAI",
        base_url: "https://api.openai.com/v1",
        balance: false,
        env: "OPENAI_API_KEY",
        wire: "openai",
        streaming: true,
        tools: true,
    },
    Kind {
        id: "anthropic",
        name: "Anthropic",
        base_url: "https://api.anthropic.com/v1",
        balance: false,
        env: "ANTHROPIC_API_KEY",
        wire: "anthropic",
        streaming: true,
        tools: true,
    },
    Kind {
        id: "openrouter",
        name: "OpenRouter",
        base_url: "https://openrouter.ai/api/v1",
        balance: true,
        env: "OPENROUTER_API_KEY",
        wire: "openai",
        streaming: true,
        tools: true,
    },
    Kind {
        id: "deepseek",
        name: "DeepSeek",
        base_url: "https://api.deepseek.com",
        balance: true,
        env: "DEEPSEEK_API_KEY",
        wire: "openai",
        streaming: true,
        tools: true,
    },
    Kind {
        id: "gemini",
        name: "Google Gemini",
        base_url: "https://generativelanguage.googleapis.com/v1beta",
        balance: false,
        env: "GEMINI_API_KEY",
        wire: "gemini",
        streaming: true,
        tools: true,
    },
    Kind {
        id: "groq",
        name: "Groq",
        base_url: "https://api.groq.com/openai/v1",
        balance: false,
        env: "GROQ_API_KEY",
        wire: "openai",
        streaming: true,
        tools: true,
    },
    Kind {
        id: "xai",
        name: "xAI (Grok)",
        base_url: "https://api.x.ai/v1",
        balance: false,
        env: "XAI_API_KEY",
        wire: "openai",
        streaming: true,
        tools: true,
    },
    Kind {
        id: "mistral",
        name: "Mistral",
        base_url: "https://api.mistral.ai/v1",
        balance: false,
        env: "MISTRAL_API_KEY",
        wire: "openai",
        streaming: true,
        tools: true,
    },
    Kind {
        id: "siliconflow",
        name: "SiliconFlow",
        base_url: "https://api.siliconflow.cn/v1",
        balance: true,
        env: "SILICONFLOW_API_KEY",
        wire: "openai",
        streaming: true,
        tools: true,
    },
    Kind {
        id: "requesty",
        name: "Requesty",
        base_url: "https://router.requesty.ai/v1",
        balance: false,
        env: "REQUESTY_API_KEY",
        wire: "openai",
        streaming: true,
        tools: true,
    },
    Kind {
        id: "newapi",
        name: "New API / One API (relay)",
        base_url: "https://seu-site.com/v1",
        balance: true,
        env: "OPENAI_API_KEY",
        wire: "openai",
        streaming: true,
        tools: true,
    },
    Kind {
        id: "ollama",
        name: "Ollama (local)",
        base_url: "http://localhost:11434/v1",
        balance: false,
        env: "",
        wire: "openai",
        streaming: true,
        tools: true,
    },
    Kind {
        id: "custom",
        name: "OpenAI-compatible",
        base_url: "https://…/v1",
        balance: false,
        env: "OPENAI_API_KEY",
        wire: "openai",
        streaming: true,
        tools: true,
    },
];

pub fn find_kind(id: &str) -> Option<&'static Kind> {
    KINDS.iter().find(|k| k.id == id)
}

pub fn kind_of(id: &str) -> &'static Kind {
    find_kind(id).unwrap_or(&KINDS[KINDS.len() - 1])
}

static LOCK: Mutex<()> = Mutex::new(());

pub fn file() -> Option<std::path::PathBuf> {
    super::tools_dir().map(|d| d.join("ai-keys.json"))
}

fn migrate_once() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        if let Some(path) = file() {
            migrate::migrate_vault_file(&path);
        }
    });
}

fn load_metadata() -> Vec<KeyEntry> {
    file()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn hydrate(list: &mut [KeyEntry]) {
    for entry in list.iter_mut() {
        entry.key = secrets::get(AI_KEYS, &migrate::key_account(&entry.kind, &entry.id))
            .unwrap_or_else(|e| {
                tracing::warn!("[ai-keys] could not read the key of {}: {}", entry.id, e);
                None
            })
            .unwrap_or_default();
        entry.access_token = secrets::get(AI_KEYS, &migrate::token_account(&entry.kind, &entry.id))
            .unwrap_or_default()
            .unwrap_or_default();
    }
}

fn load() -> Vec<KeyEntry> {
    migrate_once();
    let mut list = load_metadata();
    hydrate(&mut list);
    list
}

fn save(list: &[KeyEntry]) -> anyhow::Result<()> {
    let p = file().ok_or_else(|| anyhow!("no data folder"))?;
    let mut list = list.to_vec();
    for (account, value) in migrate::take_secrets(&mut list) {
        secrets::put_or_delete(AI_KEYS, &account, &value).map_err(|e| anyhow!(e))?;
    }
    std::fs::create_dir_all(p.parent().unwrap())?;
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(&list)?)?;
    std::fs::rename(&tmp, &p)?;
    Ok(())
}

fn forget_secrets(entry: &KeyEntry) {
    for account in [
        migrate::key_account(&entry.kind, &entry.id),
        migrate::token_account(&entry.kind, &entry.id),
    ] {
        if let Err(e) = secrets::delete(AI_KEYS, &account) {
            tracing::warn!("[ai-keys] could not forget {}: {}", account, e);
        }
    }
}

pub fn list() -> Vec<KeyView> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    load().iter().map(KeyEntry::view).collect()
}

pub fn entry_with_secret(id: &str) -> anyhow::Result<KeyEntry> {
    get(id)
}

fn get(id: &str) -> anyhow::Result<KeyEntry> {
    load()
        .into_iter()
        .find(|e| e.id == id)
        .ok_or_else(|| anyhow!("key not found"))
}

fn update<F: FnOnce(&mut KeyEntry)>(id: &str, f: F) -> anyhow::Result<KeyView> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut list = load();
    let e = list
        .iter_mut()
        .find(|e| e.id == id)
        .ok_or_else(|| anyhow!("key not found"))?;
    f(e);
    let v = e.view();
    save(&list)?;
    Ok(v)
}

pub fn upsert(mut entry: KeyEntry) -> anyhow::Result<KeyView> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut list = load();
    entry.name = entry.name.trim().to_string();
    entry.base_url = entry.base_url.trim().trim_end_matches('/').to_string();
    if entry.base_url.is_empty() {
        entry.base_url = kind_of(&entry.kind).base_url.to_string();
    }
    if entry.name.is_empty() {
        entry.name = kind_of(&entry.kind).name.to_string();
    }
    if let Some(existing) = list
        .iter_mut()
        .find(|e| e.id == entry.id && !entry.id.is_empty())
    {
        if entry.key.trim().is_empty() {
            entry.key = existing.key.clone();
        }
        if entry.access_token.trim().is_empty() {
            entry.access_token = existing.access_token.clone();
        }
        if existing.kind != entry.kind {
            let stale = existing.clone();
            forget_secrets(&stale);
        }
        entry.created = existing.created;
        entry.last_ok = existing.last_ok;
        entry.last_checked = existing.last_checked;
        entry.balance = existing.balance.clone();
        entry.models = existing.models;
        *existing = entry.clone();
    } else {
        entry.id = uuid::Uuid::new_v4().to_string();
        entry.created = chrono::Utc::now().timestamp();
        list.push(entry.clone());
    }
    save(&list)?;
    Ok(entry.view())
}

pub fn delete(id: &str) -> anyhow::Result<()> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut list = load();
    let removed: Vec<KeyEntry> = list.iter().filter(|e| e.id == id).cloned().collect();
    list.retain(|e| e.id != id);
    save(&list)?;
    for entry in &removed {
        forget_secrets(entry);
    }
    Ok(())
}

fn client() -> anyhow::Result<reqwest::Client> {
    Ok(
        crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
            .timeout(std::time::Duration::from_secs(30))
            .build()?,
    )
}

fn site_of(base: &str) -> String {
    base.trim_end_matches('/')
        .trim_end_matches("/v1")
        .to_string()
}

fn usd(v: f64) -> String {
    if v.abs() < 0.01 {
        format!("${:.4}", v)
    } else {
        format!("${:.2}", v)
    }
}

pub async fn balance(id: &str) -> anyhow::Result<KeyView> {
    let entry = get(id)?;
    let c = client()?;
    let text: String = match entry.kind.as_str() {
        "openrouter" => {
            let j: serde_json::Value = c
                .get(format!("{}/credits", entry.base_url))
                .bearer_auth(&entry.key)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            let d = &j["data"];
            let total = d["total_credits"].as_f64().unwrap_or(0.0);
            let used = d["total_usage"].as_f64().unwrap_or(0.0);
            format!(
                "{} ({} used of {})",
                usd(total - used),
                usd(used),
                usd(total)
            )
        }
        "deepseek" => {
            let j: serde_json::Value = c
                .get(format!("{}/user/balance", site_of(&entry.base_url)))
                .bearer_auth(&entry.key)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            let info = &j["balance_infos"][0];
            format!(
                "{} {}",
                info["total_balance"].as_str().unwrap_or("?"),
                info["currency"].as_str().unwrap_or("")
            )
        }
        "siliconflow" => {
            let j: serde_json::Value = c
                .get(format!("{}/user/info", entry.base_url))
                .bearer_auth(&entry.key)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            format!(
                "{} CNY",
                j["data"]["balance"]
                    .as_str()
                    .or_else(|| j["data"]["totalBalance"].as_str())
                    .unwrap_or("?")
            )
        }
        "newapi" => {
            if entry.access_token.is_empty() {
                return Err(anyhow!(
                    "enter the access token and the user ID from the dashboard"
                ));
            }
            let j: serde_json::Value = c
                .get(format!("{}/api/user/self", site_of(&entry.base_url)))
                .bearer_auth(&entry.access_token)
                .header("New-Api-User", &entry.user_id)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            if j["success"].as_bool() == Some(false) {
                return Err(anyhow!(
                    "{}",
                    j["message"].as_str().unwrap_or("the dashboard refused")
                ));
            }
            let d = &j["data"];
            let quota = d["quota"].as_f64().unwrap_or(0.0) / 500_000.0;
            let used = d["used_quota"].as_f64().unwrap_or(0.0) / 500_000.0;
            format!("{} ({} used)", usd(quota), usd(used))
        }
        _ => {
            return Err(anyhow!(
                "this provider does not expose its balance through the API"
            ))
        }
    };
    update(id, |e| e.balance = Some(text))
}

fn is_openai_compatible(kind: &str) -> bool {
    kind_of(kind).is_openai_wire()
}

pub fn export(format: &str, ids: &[String]) -> anyhow::Result<String> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let list: Vec<KeyEntry> = load()
        .into_iter()
        .filter(|e| ids.is_empty() || ids.contains(&e.id))
        .collect();
    if list.is_empty() {
        return Err(anyhow!("no key selected"));
    }
    let slug = |s: &str| {
        s.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '_'
                }
            })
            .collect::<String>()
    };
    Ok(match format {
        "env" => {
            let mut out = String::new();
            for e in &list {
                let k = kind_of(&e.kind);
                out.push_str(&format!("# {} ({})\n", e.name, k.name));
                if !k.env.is_empty() {
                    out.push_str(&format!("{}={}\n", k.env, e.key));
                }
                if is_openai_compatible(&e.kind) && e.kind != "openai" {
                    out.push_str(&format!("OPENAI_BASE_URL={}\n", e.base_url));
                }
                if e.kind == "anthropic" || e.kind == "newapi" {
                    out.push_str(&format!("ANTHROPIC_BASE_URL={}\n", site_of(&e.base_url)));
                }
                if !e.model.is_empty() {
                    out.push_str(&format!("MODEL={}\n", e.model));
                }
                out.push('\n');
            }
            out
        }
        "json" => serde_json::to_string_pretty(
            &list.iter().map(|e| serde_json::json!({ "name": e.name, "kind": e.kind, "base_url": e.base_url, "api_key": e.key, "model": e.model })).collect::<Vec<_>>(),
        )?,
        "claude-code" => {
            let e = &list[0];
            let mut env = serde_json::Map::new();
            if e.kind == "anthropic" {
                env.insert("ANTHROPIC_API_KEY".into(), e.key.clone().into());
            } else {
                env.insert("ANTHROPIC_BASE_URL".into(), site_of(&e.base_url).into());
                env.insert("ANTHROPIC_AUTH_TOKEN".into(), e.key.clone().into());
            }
            if !e.model.is_empty() {
                env.insert("ANTHROPIC_MODEL".into(), e.model.clone().into());
            }
            serde_json::to_string_pretty(&serde_json::json!({ "env": env }))?
        }
        "cherry" => serde_json::to_string_pretty(
            &list
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "id": slug(&e.name),
                        "name": e.name,
                        "type": match e.kind.as_str() { "anthropic" => "anthropic", "gemini" => "gemini", _ => "openai" },
                        "apiKey": e.key,
                        "apiHost": if e.kind == "gemini" { site_of(&e.base_url) } else { e.base_url.clone() },
                        "models": if e.model.is_empty() { vec![] } else { vec![serde_json::json!({ "id": e.model, "name": e.model })] },
                        "enabled": true
                    })
                })
                .collect::<Vec<_>>(),
        )?,
        "codex" => {
            let mut out = String::new();
            for e in list.iter().filter(|e| is_openai_compatible(&e.kind)) {
                let id = slug(&e.name);
                let env_key = format!("{}_API_KEY", id.to_ascii_uppercase());
                out.push_str(&format!(
                    "# ~/.codex/config.toml\nmodel_provider = \"{id}\"\n{}[model_providers.{id}]\nname = \"{}\"\nbase_url = \"{}\"\nenv_key = \"{env_key}\"\n\n# export {env_key}={}\n\n",
                    if e.model.is_empty() { String::new() } else { format!("model = \"{}\"\n", e.model) },
                    e.name,
                    e.base_url,
                    e.key
                ));
            }
            out
        }
        "opencode" => {
            let mut providers = serde_json::Map::new();
            for e in &list {
                let npm = match e.kind.as_str() {
                    "anthropic" => "@ai-sdk/anthropic",
                    "gemini" => "@ai-sdk/google",
                    "openai" => "@ai-sdk/openai",
                    _ => "@ai-sdk/openai-compatible",
                };
                let mut models = serde_json::Map::new();
                if !e.model.is_empty() {
                    models.insert(e.model.clone(), serde_json::json!({ "name": e.model }));
                }
                providers.insert(slug(&e.name), serde_json::json!({ "npm": npm, "name": e.name, "options": { "baseURL": e.base_url, "apiKey": e.key }, "models": models }));
            }
            serde_json::to_string_pretty(&serde_json::json!({ "$schema": "https://opencode.ai/config.json", "provider": providers }))?
        }
        _ => return Err(anyhow!("unknown format: {}", format)),
    })
}

pub fn app_base_url(kind: &str, base_url: &str) -> String {
    let base = if base_url.trim().is_empty() {
        kind_of(kind).base_url_default()
    } else {
        base_url.trim()
    }
    .trim_end_matches('/')
    .to_string();
    if kind == "gemini" && !base.ends_with("/openai") {
        return format!("{}/openai", base);
    }
    base
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn hints() {
        assert_eq!(hint(""), "");
        assert_eq!(hint("abc"), "•••");
        assert_eq!(hint("sk-1234567890abcd"), "sk-1…abcd");
        assert_eq!(site_of("https://x.com/v1/"), "https://x.com");
        assert_eq!(site_of("https://x.com"), "https://x.com");
    }

    #[test]
    fn every_kind_answers_the_capability_questions() {
        let mut ids: Vec<&str> = KINDS.iter().map(|k| k.id).collect();
        let before = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), before, "duplicate provider id in KINDS");

        for k in KINDS {
            assert!(!k.base_url_default().is_empty(), "{}", k.id);
            assert!(k.supports_streaming(), "{}", k.id);
            assert!(
                matches!(k.wire, "openai" | "anthropic" | "gemini"),
                "{}",
                k.id
            );
            assert_eq!(k.env_var().is_empty(), k.id == "ollama", "{}", k.id);
        }
        assert!(find_kind("openrouter").unwrap().supports_tools());
        assert!(find_kind("anthropic").unwrap().supports_tools());
        assert!(find_kind("nope").is_none());
        assert_eq!(kind_of("nope").id, "custom");
        assert!(kind_of("openrouter").is_openai_wire());
        assert!(!kind_of("anthropic").is_openai_wire());
        assert!(!kind_of("gemini").is_openai_wire());
    }

    #[test]
    fn the_app_base_url_falls_back_to_the_table_and_routes_gemini() {
        assert_eq!(
            app_base_url("openrouter", ""),
            "https://openrouter.ai/api/v1"
        );
        assert_eq!(
            app_base_url("openai", "https://relay.example/v1/"),
            "https://relay.example/v1"
        );
        assert_eq!(
            app_base_url("gemini", ""),
            "https://generativelanguage.googleapis.com/v1beta/openai"
        );
        assert_eq!(
            app_base_url(
                "gemini",
                "https://generativelanguage.googleapis.com/v1beta/openai"
            ),
            "https://generativelanguage.googleapis.com/v1beta/openai"
        );
    }

    struct Sandbox {
        root: PathBuf,
        _env: crate::core::test_env::Exclusive,
    }

    impl Sandbox {
        fn new(name: &str) -> Self {
            let env = crate::core::test_env::exclusive();
            let root = std::env::temp_dir().join(format!(
                "omniget-ai-keys-{}-{}-{}",
                name,
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0)
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join("tools")).unwrap();
            env.set_data_dir(&root);
            env.set(
                crate::core::secrets::SECRETS_DIR_ENV,
                Some(root.join("secrets").as_os_str()),
            );
            Self { root, _env: env }
        }

        fn vault(&self) -> PathBuf {
            self.root.join("tools").join("ai-keys.json")
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn the_migration_moves_plaintext_out_and_is_idempotent() {
        let sb = Sandbox::new("migrate");
        let legacy = r#"[
          { "id": "e1", "name": "OpenRouter", "kind": "openrouter",
            "base_url": "https://openrouter.ai/api/v1", "key": "sk-or-legacy", "created": 1 },
          { "id": "e2", "name": "Painel", "kind": "newapi",
            "base_url": "https://p.example/v1", "key": "sk-relay",
            "access_token": "panel-token", "user_id": "7", "created": 2 }
        ]"#;
        std::fs::write(sb.vault(), legacy).unwrap();

        let report = migrate::migrate_vault_file(&sb.vault());
        assert_eq!(report.moved, 3, "two keys and one access token");
        assert!(report.ran());

        let after = std::fs::read_to_string(sb.vault()).unwrap();
        for secret in ["sk-or-legacy", "sk-relay", "panel-token"] {
            assert!(!after.contains(secret), "{secret} still in {after}");
        }
        let backup = report.backup.clone().unwrap();
        assert!(std::fs::read_to_string(&backup)
            .unwrap()
            .contains("sk-or-legacy"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&backup).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "backup mode was {:o}", mode);
        }
        assert_eq!(
            entry_with_secret("e1").unwrap().key,
            "sk-or-legacy".to_string()
        );
        let e2 = entry_with_secret("e2").unwrap();
        assert_eq!(e2.key, "sk-relay");
        assert_eq!(e2.access_token, "panel-token");

        let stamp = std::fs::metadata(&backup).unwrap().modified().unwrap();
        let again = migrate::migrate_vault_file(&sb.vault());
        assert_eq!(again, migrate::Report::default());
        assert_eq!(
            std::fs::metadata(&backup).unwrap().modified().unwrap(),
            stamp
        );
    }

    #[test]
    fn saving_keeps_the_secret_out_of_the_file_and_deleting_forgets_it() {
        let sb = Sandbox::new("roundtrip");
        let view = upsert(KeyEntry {
            name: "Minha conta".into(),
            kind: "openai".into(),
            key: "sk-proj-0123456789".into(),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(view.key_hint, "sk-p…6789");
        assert!(view.has_key);
        let json = serde_json::to_string(&view).unwrap();
        assert!(!json.contains("sk-proj-0123456789"), "{json}");

        let on_disk = std::fs::read_to_string(sb.vault()).unwrap();
        assert!(!on_disk.contains("sk-proj-0123456789"), "{on_disk}");
        assert!(on_disk.contains("Minha conta"));

        let again = upsert(KeyEntry {
            id: view.id.clone(),
            name: "Minha conta".into(),
            kind: "openai".into(),
            model: "gpt-4.1".into(),
            ..Default::default()
        })
        .unwrap();
        assert!(again.has_key);
        assert_eq!(
            entry_with_secret(&view.id).unwrap().key,
            "sk-proj-0123456789"
        );

        let account = migrate::key_account("openai", &view.id);
        assert_eq!(
            crate::core::secrets::get(AI_KEYS, &account)
                .unwrap()
                .as_deref(),
            Some("sk-proj-0123456789")
        );
        delete(&view.id).unwrap();
        assert!(list().is_empty());
        assert_eq!(crate::core::secrets::get(AI_KEYS, &account).unwrap(), None);
    }
}
