//! Chaves de API (estudo 24, All API Hub — AGPL, só a ideia): um cofre local
//! de contas de IA (site → chave → modelos → saldo), teste de conectividade,
//! saldo onde a API dá (OpenRouter, DeepSeek, SiliconFlow, painéis New API)
//! e exportação para os clientes (.env, Claude Code, Cherry Studio, Codex,
//! opencode). Chaves ficam em `<app_data>/tools/ai-keys.json`, como o
//! `ai_config.json` do app; a UI só recebe o início e o fim de cada chave.

use std::sync::Mutex;

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KeyEntry {
    pub id: String,
    pub name: String,
    /// openai | anthropic | openrouter | deepseek | gemini | groq | xai | mistral | siliconflow | newapi | ollama | custom
    pub kind: String,
    pub base_url: String,
    #[serde(default)]
    pub key: String,
    /// New API: token de acesso do painel (Configurações → Token de acesso)
    #[serde(default)]
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

#[derive(Debug, Clone, Serialize)]
pub struct Kind {
    pub id: &'static str,
    pub name: &'static str,
    pub base_url: &'static str,
    pub balance: bool,
    pub env: &'static str,
}

pub const KINDS: &[Kind] = &[
    Kind { id: "openai", name: "OpenAI", base_url: "https://api.openai.com/v1", balance: false, env: "OPENAI_API_KEY" },
    Kind { id: "anthropic", name: "Anthropic", base_url: "https://api.anthropic.com/v1", balance: false, env: "ANTHROPIC_API_KEY" },
    Kind { id: "openrouter", name: "OpenRouter", base_url: "https://openrouter.ai/api/v1", balance: true, env: "OPENROUTER_API_KEY" },
    Kind { id: "deepseek", name: "DeepSeek", base_url: "https://api.deepseek.com", balance: true, env: "DEEPSEEK_API_KEY" },
    Kind { id: "gemini", name: "Google Gemini", base_url: "https://generativelanguage.googleapis.com/v1beta", balance: false, env: "GEMINI_API_KEY" },
    Kind { id: "groq", name: "Groq", base_url: "https://api.groq.com/openai/v1", balance: false, env: "GROQ_API_KEY" },
    Kind { id: "xai", name: "xAI (Grok)", base_url: "https://api.x.ai/v1", balance: false, env: "XAI_API_KEY" },
    Kind { id: "mistral", name: "Mistral", base_url: "https://api.mistral.ai/v1", balance: false, env: "MISTRAL_API_KEY" },
    Kind { id: "siliconflow", name: "SiliconFlow", base_url: "https://api.siliconflow.cn/v1", balance: true, env: "SILICONFLOW_API_KEY" },
    Kind { id: "newapi", name: "New API / One API (relay)", base_url: "https://seu-site.com/v1", balance: true, env: "OPENAI_API_KEY" },
    Kind { id: "ollama", name: "Ollama (local)", base_url: "http://localhost:11434/v1", balance: false, env: "" },
    Kind { id: "custom", name: "OpenAI-compatível", base_url: "https://…/v1", balance: false, env: "OPENAI_API_KEY" },
];

fn kind_of(id: &str) -> &'static Kind {
    KINDS.iter().find(|k| k.id == id).unwrap_or(&KINDS[KINDS.len() - 1])
}

// ── Armazenamento ──────────────────────────────────────────────────────

static LOCK: Mutex<()> = Mutex::new(());

fn file() -> Option<std::path::PathBuf> {
    super::tools_dir().map(|d| d.join("ai-keys.json"))
}

fn load() -> Vec<KeyEntry> {
    file().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn save(list: &[KeyEntry]) -> anyhow::Result<()> {
    let p = file().ok_or_else(|| anyhow!("sem pasta de dados"))?;
    std::fs::create_dir_all(p.parent().unwrap())?;
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(list)?)?;
    std::fs::rename(&tmp, &p)?;
    Ok(())
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
    load().into_iter().find(|e| e.id == id).ok_or_else(|| anyhow!("chave nao encontrada"))
}

fn update<F: FnOnce(&mut KeyEntry)>(id: &str, f: F) -> anyhow::Result<KeyView> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut list = load();
    let e = list.iter_mut().find(|e| e.id == id).ok_or_else(|| anyhow!("chave nao encontrada"))?;
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
    if let Some(existing) = list.iter_mut().find(|e| e.id == entry.id && !entry.id.is_empty()) {
        if entry.key.trim().is_empty() {
            entry.key = existing.key.clone();
        }
        if entry.access_token.trim().is_empty() {
            entry.access_token = existing.access_token.clone();
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
    list.retain(|e| e.id != id);
    save(&list)
}

// ── Rede ───────────────────────────────────────────────────────────────

fn client() -> anyhow::Result<reqwest::Client> {
    Ok(crate::core::http_client::apply_global_proxy(reqwest::Client::builder()).timeout(std::time::Duration::from_secs(30)).build()?)
}

/// Site do painel (New API): base sem o `/v1`.
fn site_of(base: &str) -> String {
    base.trim_end_matches('/').trim_end_matches("/v1").to_string()
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
        "gemini" => c.get(format!("{}/models?pageSize=1000&key={}", entry.base_url, entry.key)).send().await?.error_for_status().map_err(|e| anyhow!("Gemini: {}", e))?.json().await?,
        _ => {
            let mut req = c.get(format!("{}/models", entry.base_url));
            if !entry.key.is_empty() {
                req = req.bearer_auth(&entry.key);
            }
            let resp = req.send().await?;
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if !status.is_success() {
                return Err(anyhow!("HTTP {}: {}", status.as_u16(), text.chars().take(200).collect::<String>()));
            }
            serde_json::from_str(&text).map_err(|_| anyhow!("resposta nao e JSON: {}", text.chars().take(120).collect::<String>()))?
        }
    };
    let arr = json.get("data").or_else(|| json.get("models")).and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let mut ids: Vec<String> = arr
        .iter()
        .filter_map(|m| m.get("id").or_else(|| m.get("name")).and_then(|v| v.as_str()).map(|s| s.trim_start_matches("models/").to_string()))
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
            let j: serde_json::Value = c.get(format!("{}/credits", entry.base_url)).bearer_auth(&entry.key).send().await?.error_for_status()?.json().await?;
            let d = &j["data"];
            let total = d["total_credits"].as_f64().unwrap_or(0.0);
            let used = d["total_usage"].as_f64().unwrap_or(0.0);
            format!("{} ({} usados de {})", usd(total - used), usd(used), usd(total))
        }
        "deepseek" => {
            let j: serde_json::Value = c.get(format!("{}/user/balance", site_of(&entry.base_url))).bearer_auth(&entry.key).send().await?.error_for_status()?.json().await?;
            let info = &j["balance_infos"][0];
            format!("{} {}", info["total_balance"].as_str().unwrap_or("?"), info["currency"].as_str().unwrap_or(""))
        }
        "siliconflow" => {
            let j: serde_json::Value = c.get(format!("{}/user/info", entry.base_url)).bearer_auth(&entry.key).send().await?.error_for_status()?.json().await?;
            format!("{} CNY", j["data"]["balance"].as_str().or_else(|| j["data"]["totalBalance"].as_str()).unwrap_or("?"))
        }
        "newapi" => {
            if entry.access_token.is_empty() {
                return Err(anyhow!("informe o token de acesso e o ID de usuario do painel"));
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
                return Err(anyhow!("{}", j["message"].as_str().unwrap_or("painel recusou")));
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
    !matches!(kind, "anthropic" | "gemini")
}

pub fn export(format: &str, ids: &[String]) -> anyhow::Result<String> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let list: Vec<KeyEntry> = load().into_iter().filter(|e| ids.is_empty() || ids.contains(&e.id)).collect();
    if list.is_empty() {
        return Err(anyhow!("nenhuma chave selecionada"));
    }
    let slug = |s: &str| s.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' }).collect::<String>();
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

/// Usa esta chave como a IA do OmniGet (Ajustes → IA).
pub fn use_in_app(id: &str) -> anyhow::Result<()> {
    let e = get(id)?;
    use crate::core::ai::{self, AiProvider};
    match e.kind.as_str() {
        "openai" => {
            ai::set(AiProvider::Openai, e.model.clone(), String::new(), Some(e.key.clone()), None);
        }
        "anthropic" => {
            ai::set(AiProvider::Anthropic, e.model.clone(), String::new(), None, Some(e.key.clone()));
        }
        "gemini" => return Err(anyhow!("o chat do OmniGet fala OpenAI/Anthropic; use a rota OpenAI-compatível do Gemini (…/v1beta/openai) como personalizado")),
        _ => {
            ai::set(AiProvider::Local, e.model.clone(), e.base_url.clone(), Some(e.key.clone()), None);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hints() {
        assert_eq!(hint(""), "");
        assert_eq!(hint("abc"), "•••");
        assert_eq!(hint("sk-1234567890abcd"), "sk-1…abcd");
        assert_eq!(site_of("https://x.com/v1/"), "https://x.com");
        assert_eq!(site_of("https://x.com"), "https://x.com");
    }
}
