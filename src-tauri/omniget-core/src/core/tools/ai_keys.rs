//! Chaves de API (estudo 24, All API Hub — AGPL, só a ideia): um cofre local
//! de contas de IA (site → chave → modelos → saldo), teste de conectividade,
//! saldo onde a API dá (OpenRouter, DeepSeek, SiliconFlow, painéis New API)
//! e exportação para os clientes (.env, Claude Code, Cherry Studio, Codex,
//! opencode). A UI só recebe o início e o fim de cada chave.
//!
//! Storage, since the LLM expansion: `<app_data>/tools/ai-keys.json` holds
//! metadata only (name, kind, base URL, model, last check, balance). The secret
//! itself lives in [`crate::core::secrets`] under the `ai_keys` namespace —
//! keychain in the app, encrypted file in the CLI — keyed by `kind:id` (see
//! [`migrate`]). Files written by older builds are migrated on the first read,
//! with a 0600 backup beside them.
//!
//! `Kind` is the provider table and its `id` is the `ProviderId` string the LLM
//! layer uses ("openai", "anthropic", "openrouter", …).

use std::sync::{Mutex, OnceLock};

use anyhow::anyhow;
use base64::Engine as _;
use serde::{Deserialize, Serialize};

use crate::core::secrets::{self, AI_KEYS};

#[path = "ai_keys_migrate.rs"]
pub mod migrate;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KeyEntry {
    pub id: String,
    pub name: String,
    /// openai | anthropic | openrouter | deepseek | gemini | groq | xai | mistral | siliconflow | newapi | ollama | custom
    pub kind: String,
    pub base_url: String,
    /// Never serialised: the vault file holds metadata only. Deserialising it
    /// still works, because that is how the UI sends a new key in and how a
    /// pre-migration file is read.
    #[serde(default, skip_serializing)]
    pub key: String,
    /// New API: token de acesso do painel (Configurações → Token de acesso)
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

/// O que a UI vê: nunca a chave inteira.
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

/// One provider. `id` is the `ProviderId` the LLM layer routes on, which is why
/// this table — not an enum — is the single source of truth for a provider.
#[derive(Debug, Clone, Serialize)]
pub struct Kind {
    pub id: &'static str,
    pub name: &'static str,
    pub base_url: &'static str,
    pub balance: bool,
    pub env: &'static str,
    /// Which HTTP dialect this provider speaks: `openai`, `anthropic` or
    /// `gemini`. Everything not natively one of the other two is OpenAI-shaped.
    pub wire: &'static str,
    /// Token-by-token responses (SSE for the OpenAI and Anthropic wires,
    /// `streamGenerateContent` for Gemini).
    pub streaming: bool,
    /// Tool/function calling at the provider level. A given model may still not
    /// do it — an Ollama build with a small model is the usual case.
    pub tools: bool,
}

impl Kind {
    /// The base URL an entry gets when the user leaves the field empty.
    pub fn base_url_default(&self) -> &'static str {
        self.base_url
    }

    /// Whether the key field applies to this provider at all. A local server answers
    /// without one, so showing the field would invite the user to paste a key that is
    /// then sent nowhere.
    pub fn needs_key(&self) -> bool {
        matches!(self.id, "openai" | "anthropic")
            || (self.wire == "openai" && !matches!(self.id, "ollama" | "custom"))
            || self.wire == "gemini"
    }

    /// Whether the base URL has to be typed rather than taken from the table.
    ///
    /// Derived from the table instead of kept as a second list: a URL that only has a
    /// shape is a placeholder, and a provider behind one cannot be reached until the
    /// user replaces it. Keeping the list next to the table means a row added later
    /// cannot be forgotten by it.
    pub fn base_url_editable(&self) -> bool {
        matches!(self.id, "ollama" | "custom")
            || url::Url::parse(self.base_url)
                .map(|u| u.host_str() == Some("seu-site.com"))
                .unwrap_or(false)
    }

    /// The environment variable the ecosystem expects this key under, used by
    /// the `.env`/Codex exports. Empty when the provider needs no key.
    pub fn env_var(&self) -> &'static str {
        self.env
    }

    pub fn supports_streaming(&self) -> bool {
        self.streaming
    }

    pub fn supports_tools(&self) -> bool {
        self.tools
    }

    /// `true` when the provider talks the OpenAI dialect, whoever hosts it.
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
        // Provider-level yes; whether the pulled model does it is another
        // question, answered by the model roster and not by this table.
        tools: true,
    },
    Kind {
        id: "custom",
        name: "OpenAI-compatível",
        base_url: "https://…/v1",
        balance: false,
        env: "OPENAI_API_KEY",
        wire: "openai",
        streaming: true,
        tools: true,
    },
];

/// The table entry for a provider id, or `None` when nothing matches.
pub fn find_kind(id: &str) -> Option<&'static Kind> {
    KINDS.iter().find(|k| k.id == id)
}

/// The table entry, falling back to the OpenAI-compatible `custom` row.
pub fn kind_of(id: &str) -> &'static Kind {
    find_kind(id).unwrap_or(&KINDS[KINDS.len() - 1])
}

// ── Armazenamento ──────────────────────────────────────────────────────

static LOCK: Mutex<()> = Mutex::new(());

pub fn file() -> Option<std::path::PathBuf> {
    super::tools_dir().map(|d| d.join("ai-keys.json"))
}

/// Move the plaintext of a pre-LLM-expansion vault into the secret store, once
/// per process. Cheap after the first run: the check is one file read, and the
/// file is only rewritten when something was actually moved.
fn migrate_once() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        if let Some(path) = file() {
            migrate::migrate_vault_file(&path);
        }
    });
}

/// Metadata as it sits on disk, with the secret fields empty.
fn load_metadata() -> Vec<KeyEntry> {
    file()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Fill the secret fields from the store. One read per account; the app's store
/// caches, so a second `list()` in the same session costs nothing.
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

/// Write the vault: secrets to the store, everything else to the JSON. The
/// entries are cloned and blanked first, so the file can never carry a key even
/// if a caller hands us a hydrated list (which it always does).
fn save(list: &[KeyEntry]) -> anyhow::Result<()> {
    let p = file().ok_or_else(|| anyhow!("sem pasta de dados"))?;
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

/// Forget everything an entry owns in the secret store.
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

/// Entrada completa (com segredo) para uso interno do app.
pub fn entry_with_secret(id: &str) -> anyhow::Result<KeyEntry> {
    get(id)
}

fn get(id: &str) -> anyhow::Result<KeyEntry> {
    load()
        .into_iter()
        .find(|e| e.id == id)
        .ok_or_else(|| anyhow!("chave nao encontrada"))
}

fn update<F: FnOnce(&mut KeyEntry)>(id: &str, f: F) -> anyhow::Result<KeyView> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut list = load();
    let e = list
        .iter_mut()
        .find(|e| e.id == id)
        .ok_or_else(|| anyhow!("chave nao encontrada"))?;
    f(e);
    let v = e.view();
    save(&list)?;
    Ok(v)
}

/// Cria ou atualiza. Chave/token vazios mantêm o valor guardado.
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
        // The key is filed under `kind:id`: changing the provider of an entry
        // moves the account, so the old one has to be forgotten.
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
    // After the file is safely rewritten: dropping the row and leaving the key
    // in the keychain is the one failure mode worth avoiding here.
    for entry in &removed {
        forget_secrets(entry);
    }
    Ok(())
}

// ── Rede ───────────────────────────────────────────────────────────────

fn client() -> anyhow::Result<reqwest::Client> {
    Ok(
        crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
            .timeout(std::time::Duration::from_secs(30))
            .build()?,
    )
}

/// Site do painel (New API): base sem o `/v1`.
fn site_of(base: &str) -> String {
    base.trim_end_matches('/')
        .trim_end_matches("/v1")
        .to_string()
}

/// GET /models (ou equivalente) → ids dos modelos.
pub async fn models(entry: &KeyEntry) -> anyhow::Result<Vec<String>> {
    let c = client()?;
    let json: serde_json::Value = match entry.kind.as_str() {
        "anthropic" => {
            c.get(format!("{}/models?limit=1000", entry.base_url))
                .header("x-api-key", &entry.key)
                .header("anthropic-version", "2023-06-01")
                .send()
                .await?
                .error_for_status()
                .map_err(|e| anyhow!("Anthropic: {}", e))?
                .json()
                .await?
        }
        "gemini" => {
            c.get(format!(
                "{}/models?pageSize=1000&key={}",
                entry.base_url, entry.key
            ))
            .send()
            .await?
            .error_for_status()
            .map_err(|e| anyhow!("Gemini: {}", e))?
            .json()
            .await?
        }
        _ => {
            let mut req = c.get(format!("{}/models", entry.base_url));
            if !entry.key.is_empty() {
                req = req.bearer_auth(&entry.key);
            }
            let resp = req.send().await?;
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if !status.is_success() {
                return Err(anyhow!(
                    "HTTP {}: {}",
                    status.as_u16(),
                    text.chars().take(200).collect::<String>()
                ));
            }
            serde_json::from_str(&text).map_err(|_| {
                anyhow!(
                    "resposta nao e JSON: {}",
                    text.chars().take(120).collect::<String>()
                )
            })?
        }
    };
    let arr = json
        .get("data")
        .or_else(|| json.get("models"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut ids: Vec<String> = arr
        .iter()
        .filter_map(|m| {
            m.get("id")
                .or_else(|| m.get("name"))
                .and_then(|v| v.as_str())
                .map(|s| s.trim_start_matches("models/").to_string())
        })
        .collect();
    ids.sort();
    Ok(ids)
}

pub async fn test(id: &str) -> anyhow::Result<KeyView> {
    let entry = get(id)?;
    let now = chrono::Utc::now().timestamp();
    match models(&entry).await {
        Ok(ids) => update(id, |e| {
            e.last_ok = Some(true);
            e.last_checked = Some(now);
            e.models = ids.len();
            e.error = None;
        }),
        Err(err) => {
            let msg = err.to_string();
            let _ = update(id, |e| {
                e.last_ok = Some(false);
                e.last_checked = Some(now);
                e.error = Some(msg.clone());
            });
            Err(anyhow!(msg))
        }
    }
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
                "{} ({} usados de {})",
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
                    "informe o token de acesso e o ID de usuario do painel"
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
                    j["message"].as_str().unwrap_or("painel recusou")
                ));
            }
            let d = &j["data"];
            let quota = d["quota"].as_f64().unwrap_or(0.0) / 500_000.0;
            let used = d["used_quota"].as_f64().unwrap_or(0.0) / 500_000.0;
            format!("{} ({} usados)", usd(quota), usd(used))
        }
        _ => return Err(anyhow!("este provedor nao expoe saldo pela API")),
    };
    update(id, |e| e.balance = Some(text))
}

// ── Exportar ───────────────────────────────────────────────────────────

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
        return Err(anyhow!("nenhuma chave selecionada"));
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
            // ~/.claude/settings.json → "env"
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
        _ => return Err(anyhow!("formato desconhecido: {}", format)),
    })
}

/// The base URL the app's chat should call for this entry. Gemini is the one
/// provider whose native route is not OpenAI-shaped, so it goes through its own
/// OpenAI-compatible endpoint instead of being refused.
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

/// Usa esta chave como a IA do OmniGet (Ajustes → IA).
///
/// The entry's `kind` is kept as it is: the app config records the provider id,
/// and `AiProvider` is derived from the wire instead of every non-OpenAI entry
/// being filed as `Local`.
pub fn use_in_app(id: &str) -> anyhow::Result<()> {
    let e = get(id)?;
    crate::core::ai::set_from_key(
        &e.kind,
        &e.id,
        e.model.clone(),
        app_base_url(&e.kind, &e.base_url),
        e.key.clone(),
    );
    Ok(())
}

// ── Sign in with OpenRouter (OAuth PKCE, sem backend) ──────────────────

/// Where OpenRouter sends the browser back. The `omniget://` scheme is already
/// registered by the app (`tauri-plugin-deep-link`), so no local HTTP server and
/// no backend of ours is involved.
///
/// WHY the state rides in the callback URL: the OAuth PKCE flow documented at
/// <https://openrouter.ai/docs/use-cases/oauth-pkce> only promises to append
/// `code` to `callback_url`; it does not define a `state` parameter of its own.
/// A parameter we put in `callback_url` does come back intact, so that is the
/// one the check can rely on. `state` is also sent as a top-level query
/// parameter, which costs nothing and is echoed by any provider that does
/// implement it.
pub const OPENROUTER_CALLBACK: &str = "omniget://openrouter-auth";
const OPENROUTER_AUTH: &str = "https://openrouter.ai/auth";
const OPENROUTER_KEYS: &str = "https://openrouter.ai/api/v1/auth/keys";
/// How long a started sign-in stays valid. Long enough to create an account,
/// short enough that a forgotten verifier does not sit in memory all session.
const PKCE_TTL_SECS: i64 = 600;

/// What the UI needs to open the browser. The verifier never leaves the core.
#[derive(Debug, Clone, Serialize)]
pub struct PkceStart {
    pub url: String,
    pub state: String,
    pub callback_url: String,
}

struct Pending {
    state: String,
    verifier: String,
    started: i64,
}

static PENDING: Mutex<Option<Pending>> = Mutex::new(None);

/// 32 random bytes, base64url without padding: 43 chars, inside the 43–128 the
/// RFC 7636 verifier allows, and every character is already unreserved.
fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::Rng::fill_bytes(&mut rand::rng(), &mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// S256: base64url(sha256(verifier)), no padding.
pub fn pkce_challenge(verifier: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(h.finalize())
}

/// The callback OpenRouter is given, carrying the state it has to hand back.
/// Pure, so the deep link the app has to recognise is a test and not a comment.
pub fn openrouter_callback_url(state: &str) -> String {
    format!(
        "{}?state={}",
        OPENROUTER_CALLBACK,
        urlencoding::encode(state)
    )
}

/// The URL the browser opens. Pure, so the shape is a test and not a comment.
pub fn openrouter_auth_url(callback_url: &str, challenge: &str, state: &str) -> String {
    format!(
        "{}?callback_url={}&code_challenge={}&code_challenge_method=S256&state={}",
        OPENROUTER_AUTH,
        urlencoding::encode(callback_url),
        urlencoding::encode(challenge),
        urlencoding::encode(state)
    )
}

/// Start a sign-in. The last start wins: one browser window at a time.
pub fn pkce_start() -> PkceStart {
    let verifier = random_token();
    let state = random_token();
    let challenge = pkce_challenge(&verifier);
    if let Ok(mut slot) = PENDING.lock() {
        *slot = Some(Pending {
            state: state.clone(),
            verifier,
            started: chrono::Utc::now().timestamp(),
        });
    }
    let callback_url = openrouter_callback_url(&state);
    PkceStart {
        url: openrouter_auth_url(&callback_url, &challenge, &state),
        state,
        callback_url,
    }
}

/// Take the pending verifier, checking the state that came back through the
/// callback and the deadline. Consumed either way: a code is single use, and a
/// second attempt has to start a new sign-in.
///
/// The state is mandatory. A callback that arrives without one is not a callback
/// this process started — it is somebody else handing the app a code, which is
/// exactly what the parameter exists to refuse.
fn take_pending(state: Option<&str>) -> anyhow::Result<String> {
    let mut slot = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    let pending = slot.take().ok_or_else(|| anyhow!("ERR_PKCE_NO_START"))?;
    match state.map(str::trim).filter(|s| !s.is_empty()) {
        Some(state) if state == pending.state => {}
        _ => return Err(anyhow!("ERR_PKCE_STATE")),
    }
    if chrono::Utc::now().timestamp() - pending.started > PKCE_TTL_SECS {
        return Err(anyhow!("ERR_PKCE_EXPIRED"));
    }
    Ok(pending.verifier)
}

/// Exchange the code for a key and file it in the vault. The key goes straight
/// into the secret store; the caller only ever sees the masked view.
pub async fn pkce_finish(code: &str, state: Option<&str>) -> anyhow::Result<KeyView> {
    let code = code.trim();
    if code.is_empty() {
        return Err(anyhow!("ERR_PKCE_NO_CODE"));
    }
    let verifier = take_pending(state)?;
    let body = serde_json::json!({
        "code": code,
        "code_verifier": verifier,
        "code_challenge_method": "S256",
    });
    let resp = client()?.post(OPENROUTER_KEYS).json(&body).send().await?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(anyhow!(
            "ERR_PKCE_EXCHANGE: HTTP {} {}",
            status.as_u16(),
            text.chars().take(200).collect::<String>()
        ));
    }
    let json: serde_json::Value = serde_json::from_str(&text)
        .map_err(|_| anyhow!("ERR_PKCE_EXCHANGE: resposta nao e JSON"))?;
    let key = json
        .get("key")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    if key.is_empty() {
        return Err(anyhow!("ERR_PKCE_EXCHANGE: resposta sem chave"));
    }
    let existing = list()
        .into_iter()
        .find(|v| v.kind == "openrouter" && v.name == "OpenRouter");
    upsert(KeyEntry {
        id: existing.map(|v| v.id).unwrap_or_default(),
        name: "OpenRouter".to_string(),
        kind: "openrouter".to_string(),
        base_url: kind_of("openrouter").base_url_default().to_string(),
        key,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::{MutexGuard, OnceLock};

    #[test]
    fn hints() {
        assert_eq!(hint(""), "");
        assert_eq!(hint("abc"), "•••");
        assert_eq!(hint("sk-1234567890abcd"), "sk-1…abcd");
        assert_eq!(site_of("https://x.com/v1/"), "https://x.com");
        assert_eq!(site_of("https://x.com"), "https://x.com");
    }

    /// The table is the provider registry the LLM layer routes on, so every row
    /// has to answer the four questions and no id may repeat.
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
            // Only the local provider needs no key, so only it has no variable.
            assert_eq!(k.env_var().is_empty(), k.id == "ollama", "{}", k.id);
        }
        assert!(find_kind("openrouter").unwrap().supports_tools());
        assert!(find_kind("anthropic").unwrap().supports_tools());
        assert!(find_kind("nope").is_none());
        // An unknown id degrades to the OpenAI-compatible row instead of panicking.
        assert_eq!(kind_of("nope").id, "custom");
        assert!(kind_of("openrouter").is_openai_wire());
        assert!(!kind_of("anthropic").is_openai_wire());
        assert!(!kind_of("gemini").is_openai_wire());
    }

    /// `use_in_app` no longer refuses Gemini, and no longer flattens a provider
    /// into `Local` without a base URL.
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
        // Idempotent: a user who already pasted the compatible route keeps it.
        assert_eq!(
            app_base_url(
                "gemini",
                "https://generativelanguage.googleapis.com/v1beta/openai"
            ),
            "https://generativelanguage.googleapis.com/v1beta/openai"
        );
    }

    /// RFC 7636 appendix B, so the challenge is right without a network call.
    #[test]
    fn the_pkce_challenge_matches_the_rfc_vector() {
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        // The callback carries the state, because OpenRouter only promises to
        // append `code` to it — a parameter already in the URL comes back.
        assert_eq!(
            openrouter_callback_url("s+t/u"),
            "omniget://openrouter-auth?state=s%2Bt%2Fu"
        );
        let url = openrouter_auth_url(&openrouter_callback_url("st8"), "abc+/=", "st8");
        assert!(url.starts_with("https://openrouter.ai/auth?"), "{url}");
        assert!(
            url.contains("callback_url=omniget%3A%2F%2Fopenrouter-auth%3Fstate%3Dst8"),
            "{url}"
        );
        assert!(url.contains("code_challenge=abc%2B%2F%3D"), "{url}");
        assert!(url.contains("&code_challenge_method=S256"), "{url}");
        assert!(url.ends_with("&state=st8"), "{url}");
    }

    /// One test for the whole pending slot, because it is process-wide and two
    /// tests poking it in parallel would race.
    #[test]
    fn a_finish_is_refused_without_a_start_a_code_or_the_right_state() {
        // An empty code never even looks for a pending start.
        let err = futures::executor::block_on(pkce_finish("  ", Some("x")))
            .unwrap_err()
            .to_string();
        assert_eq!(err, "ERR_PKCE_NO_CODE");

        if let Ok(mut slot) = PENDING.lock() {
            *slot = None;
        }
        let err = futures::executor::block_on(pkce_finish("code", Some("x")))
            .unwrap_err()
            .to_string();
        assert_eq!(err, "ERR_PKCE_NO_START");

        // A callback whose state does not match the start is somebody else's.
        let started = pkce_start();
        assert!(!started.state.is_empty());
        // base64url needs no escaping, so the state is literal in the callback.
        assert!(started.callback_url.ends_with(&started.state));
        assert!(started.url.ends_with(&started.state));
        let err = futures::executor::block_on(pkce_finish("code", Some("not-the-state")))
            .unwrap_err()
            .to_string();
        assert_eq!(err, "ERR_PKCE_STATE");
        // ...and it consumed the start: the code is single use either way.
        let err = futures::executor::block_on(pkce_finish("code", Some(&started.state)))
            .unwrap_err()
            .to_string();
        assert_eq!(err, "ERR_PKCE_NO_START");

        // A callback with no state at all is refused too, which is the case the
        // old `if let` silently let through.
        pkce_start();
        let err = futures::executor::block_on(pkce_finish("code", None))
            .unwrap_err()
            .to_string();
        assert_eq!(err, "ERR_PKCE_STATE");
        let err = futures::executor::block_on(pkce_finish("code", Some("   ")))
            .unwrap_err()
            .to_string();
        assert_eq!(err, "ERR_PKCE_NO_START");
    }

    // ── Storage and migration ────────────────────────────────────────────

    /// `set_var` is process-global and cargo runs tests on threads, so every
    /// test that moves the data directory takes this lock.
    fn env_lock() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    }

    struct Sandbox {
        root: PathBuf,
        _guard: MutexGuard<'static, ()>,
        previous: Vec<(&'static str, Option<String>)>,
    }

    impl Sandbox {
        fn new(name: &str) -> Self {
            let guard = env_lock();
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
            let mut previous = Vec::new();
            for (key, value) in [
                ("OMNIGET_DATA_DIR", root.to_str().unwrap().to_string()),
                (
                    crate::core::secrets::SECRETS_DIR_ENV,
                    root.join("secrets").to_str().unwrap().to_string(),
                ),
            ] {
                previous.push((key, std::env::var(key).ok()));
                std::env::set_var(key, value);
            }
            Self {
                root,
                _guard: guard,
                previous,
            }
        }

        fn vault(&self) -> PathBuf {
            self.root.join("tools").join("ai-keys.json")
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            for (key, value) in self.previous.iter().rev() {
                match value {
                    Some(v) => std::env::set_var(key, v),
                    None => std::env::remove_var(key),
                }
            }
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    /// The whole point of the phase: a vault written by an older build loses its
    /// plaintext, keeps working, and does not lose the key.
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

        // The file no longer holds any of it...
        let after = std::fs::read_to_string(sb.vault()).unwrap();
        for secret in ["sk-or-legacy", "sk-relay", "panel-token"] {
            assert!(!after.contains(secret), "{secret} still in {after}");
        }
        // ...the backup does, at 0600...
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
        // ...and the secret is readable again through the store.
        assert_eq!(
            entry_with_secret("e1").unwrap().key,
            "sk-or-legacy".to_string()
        );
        let e2 = entry_with_secret("e2").unwrap();
        assert_eq!(e2.key, "sk-relay");
        assert_eq!(e2.access_token, "panel-token");

        // Idempotent: nothing left to move, no second backup.
        let stamp = std::fs::metadata(&backup).unwrap().modified().unwrap();
        let again = migrate::migrate_vault_file(&sb.vault());
        assert_eq!(again, migrate::Report::default());
        assert_eq!(
            std::fs::metadata(&backup).unwrap().modified().unwrap(),
            stamp
        );
    }

    /// The UI never gets more than four characters of a key, and deleting an
    /// entry forgets the secret instead of orphaning it in the store.
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

        // A save with an empty key keeps the stored one (the UI never echoes it).
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
