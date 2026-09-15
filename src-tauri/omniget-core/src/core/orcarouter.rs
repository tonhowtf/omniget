//! OrcaRouter provider support: two authentication entrances behind one
//! credential interface, plus bounded live model discovery with capability
//! filtering and a small verified cold-start seed.
//!
//! **Origins.** Authentication lives on a different public origin than
//! inference. Auth and code exchange use `https://www.orcarouter.ai`
//! (`/auth` for the consent screen, `/api/v1/auth/keys` for the exchange);
//! inference and the model catalog use `https://api.orcarouter.ai/v1`.
//! `https://api.orcarouter.ai/v1/auth/keys` is a 404 — the relay is at `/v1`,
//! the auth endpoints are not. Neither origin is ever derived from the other by
//! rewriting a hostname or blindly appending `/v1`.
//!
//! **Credential interface.** [`CredentialAdapter`] is the single seam. The
//! pasted-API-key adapter and the OAuth 2.0 + PKCE adapter are two
//! implementations of it and both yield the same [`Credential`]. Nothing
//! downstream — the chat adapter, the catalog fetch, the settings UI — reads
//! `Credential::method`; it only reads `Credential::key`.
//!
//! **No refresh grant.** PKCE exchanges an auth code for a long-lived
//! OrcaRouter API key, not for an access/refresh token pair. There is no
//! refresh endpoint. A `401` from the relay means the exact credential
//! generation that made the request must be reauthorized; it never triggers a
//! synthetic refresh.

use serde::{Deserialize, Serialize};
use std::sync::{Mutex, OnceLock};

// ---------------------------------------------------------------------------
// Public origins and endpoints
// ---------------------------------------------------------------------------

/// Consent screen origin. `/auth` is appended to this.
pub const DEFAULT_AUTH_BASE: &str = "https://www.orcarouter.ai";
/// Inference and model-catalog origin, including the `/v1` suffix.
pub const DEFAULT_API_BASE: &str = "https://api.orcarouter.ai/v1";

/// Fixed consent-screen path on the auth origin.
pub const AUTHORIZE_PATH: &str = "/auth";
/// Fixed code-exchange path on the auth origin. Note the `/api/v1/auth`
/// prefix: this is deliberately *not* `/v1/auth/keys`.
pub const EXCHANGE_PATH: &str = "/api/v1/auth/keys";

/// Label shown on the consent screen.
pub const APP_NAME: &str = "OmniGet";

/// Where a user can see and revoke every key this app was issued.
pub const KEY_DASHBOARD_URL: &str = "https://www.orcarouter.ai/console/authorized-apps";

/// Every OrcaRouter API key starts with this. Used only as a cheap input-shape
/// check — the prefix is not proof that a credential is valid.
pub const KEY_PREFIX: &str = "sk-orca-";

/// Env var holding a shared self-hosted base for both origins.
pub const ENV_SHARED_BASE: &str = "ORCA_BASE_URL";
/// Env var overriding the auth origin only.
pub const ENV_AUTH_BASE: &str = "ORCA_AUTH_BASE_URL";
/// Env var overriding the inference origin only.
pub const ENV_API_BASE: &str = "ORCA_API_BASE_URL";

/// The catalog advertises a route this client cannot speak. Kept explicit so a
/// catalog response can never make the client claim a capability it lacks.
const SUPPORTED_ENDPOINT_TYPES: &[&str] = &[
    "openai",
    "openai-response",
    "anthropic",
    "gemini",
    "embedding",
    "embeddings",
    "image-generation",
    "openai-video",
    "jina-rerank",
];

/// Endpoint types that mean "not a general text-generation route".
const NON_CHAT_ENDPOINT_TYPES: &[&str] = &[
    "image-generation",
    "openai-video",
    "jina-rerank",
    "embedding",
    "embeddings",
];

/// Endpoint types that satisfy the plain text chat/agent entrance.
const TEXT_ENDPOINT_TYPES: &[&str] = &["openai", "openai-response", "anthropic", "gemini"];

/// Bounds. A catalog response must not be able to consume unbounded memory.
const CATALOG_MAX_BYTES: usize = 2 * 1024 * 1024;
const CATALOG_MAX_ITEMS: usize = 2000;
const CATALOG_TIMEOUT_SECS: u64 = 10;
const EXCHANGE_MAX_BYTES: usize = 64 * 1024;
const EXCHANGE_TIMEOUT_SECS: u64 = 30;
/// Live catalogs are reused for this long before a refresh is attempted.
pub const CATALOG_TTL_MS: u64 = 15 * 60 * 1000;

const CATALOG_FILE: &str = "orcarouter_catalog.json";

// ---------------------------------------------------------------------------
// Origins
// ---------------------------------------------------------------------------

/// The resolved pair of OrcaRouter origins.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Origins {
    /// Auth origin without a trailing slash, e.g. `https://www.orcarouter.ai`.
    pub auth: String,
    /// Inference origin without a trailing slash, e.g. `https://api.orcarouter.ai/v1`.
    pub api: String,
}

impl Default for Origins {
    fn default() -> Self {
        Self {
            auth: DEFAULT_AUTH_BASE.to_string(),
            api: DEFAULT_API_BASE.to_string(),
        }
    }
}

impl Origins {
    /// Absolute URL for the consent screen.
    pub fn authorize_url(&self) -> String {
        format!("{}{}", self.auth, AUTHORIZE_PATH)
    }

    /// Absolute URL for the code exchange. Never the inference origin.
    pub fn exchange_url(&self) -> String {
        format!("{}{}", self.auth, EXCHANGE_PATH)
    }

    /// Absolute URL for a versioned inference path, e.g. `/chat/completions`.
    pub fn api_url(&self, path: &str) -> String {
        format!("{}{}", self.api, path)
    }
}

/// Resolve origins from the process environment.
pub fn origins() -> Origins {
    origins_from(|k| std::env::var(k).ok())
}

/// Resolve origins from an arbitrary lookup, so the precedence rules are
/// testable without mutating process-global state.
///
/// Precedence: an explicit per-origin override beats the shared self-hosted
/// base, which beats the public default. The two origins are resolved
/// independently; neither is derived from the other.
pub fn origins_from(get: impl Fn(&str) -> Option<String>) -> Origins {
    let shared = get(ENV_SHARED_BASE);
    let auth = get(ENV_AUTH_BASE).or_else(|| shared.clone());
    let api = get(ENV_API_BASE).or(shared);

    Origins {
        auth: normalize_auth_base(auth.as_deref().unwrap_or(DEFAULT_AUTH_BASE)),
        api: normalize_api_base(api.as_deref().unwrap_or(DEFAULT_API_BASE)),
    }
}

/// Trim a trailing slash from an auth base. The auth origin carries no version
/// suffix, so nothing is appended.
fn normalize_auth_base(raw: &str) -> String {
    let t = raw.trim().trim_end_matches('/');
    if t.is_empty() {
        DEFAULT_AUTH_BASE.to_string()
    } else {
        t.to_string()
    }
}

/// Normalize an inference base. An override that names only an origin gets the
/// documented `/v1` suffix; an override that already carries a path is used
/// verbatim. This is a normalization of the *API* override, not a derivation of
/// one origin from the other.
fn normalize_api_base(raw: &str) -> String {
    let t = raw.trim().trim_end_matches('/');
    if t.is_empty() {
        return DEFAULT_API_BASE.to_string();
    }
    match url::Url::parse(t) {
        Ok(u) if u.path().is_empty() || u.path() == "/" => format!("{}/v1", t),
        _ => t.to_string(),
    }
}

/// Whether an origin is acceptable: HTTPS for anything remote, plain HTTP only
/// for loopback development addresses.
pub fn origin_is_allowed(raw: &str) -> bool {
    let Ok(u) = url::Url::parse(raw) else {
        return false;
    };
    if !u.username().is_empty() || u.password().is_some() {
        return false;
    }
    match u.scheme() {
        "https" => true,
        "http" => matches!(
            u.host_str(),
            Some("localhost") | Some("127.0.0.1") | Some("[::1]")
        ),
        _ => false,
    }
}

/// Reject an origins pair that would send credentials somewhere unacceptable.
pub fn validate_origins(o: &Origins) -> Result<(), String> {
    if !origin_is_allowed(&o.auth) {
        return Err(format!(
            "OrcaRouter auth base must be HTTPS (or loopback HTTP): {}",
            o.auth
        ));
    }
    if !origin_is_allowed(&o.api) {
        return Err(format!(
            "OrcaRouter API base must be HTTPS (or loopback HTTP): {}",
            o.api
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// PKCE
// ---------------------------------------------------------------------------

/// A single PKCE attempt. The verifier never leaves the process until the code
/// exchange, and is never logged, serialized into an error, or placed in a URL.
pub struct PkceAttempt {
    verifier: String,
    pub challenge: String,
    pub state: String,
}

impl PkceAttempt {
    /// Consume the attempt, yielding the verifier for the exchange.
    pub fn verifier(&self) -> &str {
        &self.verifier
    }
}

impl std::fmt::Debug for PkceAttempt {
    /// Deliberately omits the verifier so this type can never leak it through a
    /// `{:?}` in a log line, a panic message, or a test snapshot.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PkceAttempt")
            .field("challenge", &self.challenge)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

/// base64url without padding, per RFC 7636.
fn b64url(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// `base64url(sha256(verifier))` with no padding. Always S256; `plain` is never
/// offered, because the consent screen lets the user ask for a displayed code
/// even when a callback URL was supplied.
pub fn challenge_for(verifier: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(verifier.as_bytes());
    b64url(&h.finalize())
}

/// Fresh verifier, challenge and state for one attempt, from the OS RNG.
///
/// The verifier is 32 bytes of operating-system entropy and the state is 16,
/// both base64url-encoded. Nothing is derived from a clock, a username, or a
/// constant, so two attempts in the same millisecond cannot collide.
pub fn new_attempt() -> Result<PkceAttempt, String> {
    let mut vbuf = [0u8; 32];
    let mut sbuf = [0u8; 16];
    fill_random(&mut vbuf)?;
    fill_random(&mut sbuf)?;
    let verifier = b64url(&vbuf);
    let challenge = challenge_for(&verifier);
    Ok(PkceAttempt {
        verifier,
        challenge,
        state: b64url(&sbuf),
    })
}

/// Fill from the operating system CSPRNG.
fn fill_random(dst: &mut [u8]) -> Result<(), String> {
    use rand::TryRng;
    rand::rngs::SysRng
        .try_fill_bytes(dst)
        .map_err(|_| "OrcaRouter: system RNG unavailable".to_string())
}

/// Whether a callback URL is one the consent endpoint will accept, so we never
/// build a URL that is rejected before the user sees a screen.
///
/// `https://` any host/port; `http://` only loopback; no userinfo, no fragment.
pub fn callback_url_allowed(raw: &str) -> bool {
    let Ok(u) = url::Url::parse(raw) else {
        return false;
    };
    if !u.username().is_empty() || u.password().is_some() || u.fragment().is_some() {
        return false;
    }
    match u.scheme() {
        "https" => true,
        "http" => matches!(
            u.host_str(),
            Some("localhost") | Some("127.0.0.1") | Some("[::1]")
        ),
        _ => false,
    }
}

/// Build the consent-screen URL for a loopback (Flow A) attempt.
///
/// Flow A is chosen because this client is a desktop app: it runs on the user's
/// own machine with a real browser and can bind an ephemeral loopback port, so
/// the code returns automatically and the user clicks once.
pub fn authorize_url_loopback(
    origins: &Origins,
    port: u16,
    attempt: &PkceAttempt,
) -> Result<String, String> {
    let callback = format!("http://127.0.0.1:{}/cb", port);
    if !callback_url_allowed(&callback) {
        return Err("OrcaRouter: refusing to send a callback URL the server rejects".to_string());
    }
    let mut u = url::Url::parse(&origins.authorize_url())
        .map_err(|_| "OrcaRouter: bad authorize URL".to_string())?;
    {
        let mut q = u.query_pairs_mut();
        q.append_pair("callback_url", &callback);
        q.append_pair("code_challenge", &attempt.challenge);
        q.append_pair("code_challenge_method", "S256");
        q.append_pair("state", &attempt.state);
        q.append_pair("app_name", APP_NAME);
        q.append_pair("scope", "api");
    }
    Ok(u.to_string())
}

/// Build the consent-screen URL for an out-of-band (Flow B) attempt. Kept so
/// the same tested URL builder covers the headless fallback; S256 is mandatory
/// there rather than merely advisable.
pub fn authorize_url_oob(origins: &Origins, attempt: &PkceAttempt) -> Result<String, String> {
    let mut u = url::Url::parse(&origins.authorize_url())
        .map_err(|_| "OrcaRouter: bad authorize URL".to_string())?;
    {
        let mut q = u.query_pairs_mut();
        q.append_pair("callback_url", "oob");
        q.append_pair("code_challenge", &attempt.challenge);
        q.append_pair("code_challenge_method", "S256");
        q.append_pair("state", &attempt.state);
        q.append_pair("app_name", APP_NAME);
        q.append_pair("scope", "api");
    }
    Ok(u.to_string())
}

// ---------------------------------------------------------------------------
// Credential interface
// ---------------------------------------------------------------------------

/// How a credential was obtained. Recorded for labels and account management
/// only — no request path branches on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMethod {
    /// The user pasted an existing `sk-orca-…` key.
    ApiKey,
    /// OAuth 2.0 + PKCE issued the key.
    Pkce,
}

impl AuthMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            AuthMethod::ApiKey => "api_key",
            AuthMethod::Pkce => "pkce",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "api_key" => Some(AuthMethod::ApiKey),
            "pkce" => Some(AuthMethod::Pkce),
            _ => None,
        }
    }
}

/// What every authentication entrance produces, and the only thing the chat
/// adapter and model discovery consume.
#[derive(Clone, PartialEq, Eq)]
pub struct Credential {
    key: String,
    /// Which entrance produced this. Diagnostic only.
    pub method: AuthMethod,
    /// OrcaRouter `user_id` from the exchange, when the entrance returned one.
    pub account: Option<String>,
    /// The `scope` actually granted, when known.
    pub scope: Option<String>,
    /// Bumped on every successful save. A late failure from an older attempt
    /// can only ever invalidate its own generation.
    pub generation: u64,
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credential")
            .field("key", &"<redacted>")
            .field("method", &self.method)
            .field("account", &self.account)
            .field("scope", &self.scope)
            .field("generation", &self.generation)
            .finish()
    }
}

impl Credential {
    pub fn new(
        key: impl Into<String>,
        method: AuthMethod,
        account: Option<String>,
        scope: Option<String>,
        generation: u64,
    ) -> Self {
        Self {
            key: key.into(),
            method,
            account,
            scope,
            generation,
        }
    }

    /// The bearer token. The only accessor that yields secret material.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Last four characters, for a status line. Never enough to use the key.
    pub fn masked(&self) -> String {
        let n = self.key.chars().count();
        if n <= 4 {
            return "•".repeat(n.max(1));
        }
        let tail: String = self.key.chars().skip(n - 4).collect();
        format!("{}{}", "•".repeat(8), tail)
    }
}

/// The single credential seam. Both entrances implement it and the provider
/// never learns which one ran.
#[async_trait::async_trait]
pub trait CredentialAdapter: Send + Sync {
    fn method(&self) -> AuthMethod;
    /// Produce a credential, or a message safe to show a user. Implementations
    /// must never include the verifier or the key in the error.
    async fn obtain(&self) -> Result<Credential, String>;
}

/// Adapter for a key the user pasted from the OrcaRouter console.
pub struct ApiKeyAdapter {
    pub key: String,
    pub generation: u64,
}

#[async_trait::async_trait]
impl CredentialAdapter for ApiKeyAdapter {
    fn method(&self) -> AuthMethod {
        AuthMethod::ApiKey
    }

    async fn obtain(&self) -> Result<Credential, String> {
        let key = self.key.trim().to_string();
        if !looks_like_key(&key) {
            return Err(format!(
                "OrcaRouter API key should start with \"{}\"",
                KEY_PREFIX
            ));
        }
        Ok(Credential::new(
            key,
            AuthMethod::ApiKey,
            None,
            None,
            self.generation,
        ))
    }
}

/// Adapter for a completed PKCE exchange. The interactive part (consent screen
/// and callback) happens in the login service; this adapter is what turns the
/// exchange result into the shared credential type.
pub struct PkceAdapter {
    pub exchanged: ExchangedKey,
    pub generation: u64,
}

#[async_trait::async_trait]
impl CredentialAdapter for PkceAdapter {
    fn method(&self) -> AuthMethod {
        AuthMethod::Pkce
    }

    async fn obtain(&self) -> Result<Credential, String> {
        Ok(Credential::new(
            self.exchanged.key.clone(),
            AuthMethod::Pkce,
            self.exchanged.user_id.clone(),
            Some(self.exchanged.scope.clone()),
            self.generation,
        ))
    }
}

/// Cheap input-shape check only. The prefix is not proof the key is valid, and
/// validating it would require a paid request, so validity stays "unknown"
/// until the first real call.
pub fn looks_like_key(key: &str) -> bool {
    let t = key.trim();
    t.starts_with(KEY_PREFIX) && t.len() > KEY_PREFIX.len() + 8
}

// ---------------------------------------------------------------------------
// Code exchange
// ---------------------------------------------------------------------------

/// The granted result of a successful code exchange.
#[derive(Clone)]
pub struct ExchangedKey {
    pub key: String,
    pub user_id: Option<String>,
    pub scope: String,
}

impl std::fmt::Debug for ExchangedKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExchangedKey")
            .field("key", &"<redacted>")
            .field("user_id", &self.user_id)
            .field("scope", &self.scope)
            .finish()
    }
}

/// Terminal classification for an exchange failure, so the UI can say
/// something actionable instead of "request failed".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExchangeError {
    /// 400 — the challenge method was unrecognised, or it differed from the one
    /// sent at authorize time (downgrade defence).
    Protocol,
    /// 403 — code unknown, expired, already used, or verifier mismatch.
    Rejected,
    /// 429 — too many PKCE keys issued recently for this account.
    RateLimited,
    /// Transport failure or malformed body.
    Network,
}

impl ExchangeError {
    /// User-facing text. Never contains the code or the verifier.
    pub fn message(self) -> &'static str {
        match self {
            ExchangeError::Protocol => {
                "OrcaRouter rejected the PKCE challenge method. Start the sign-in again."
            }
            ExchangeError::Rejected => {
                "This authorization code is no longer valid. Start the sign-in again."
            }
            ExchangeError::RateLimited => {
                "OrcaRouter is rate limiting new sign-ins for this account. Wait a few minutes, or paste an API key instead."
            }
            ExchangeError::Network => {
                "Could not reach OrcaRouter to finish sign-in. Check your connection and try again."
            }
        }
    }
}

/// Whether a granted scope satisfies what this client actually does. The
/// response says what was *granted*, which can be narrower than what was asked
/// for, so the requested scope is never assumed.
pub fn scope_satisfies(granted: &str, required: &str) -> bool {
    granted
        .split_whitespace()
        .any(|s| s.eq_ignore_ascii_case(required))
}

/// Exchange an auth code for an OrcaRouter API key.
///
/// Posts to `{auth}/api/v1/auth/keys` — never to the inference origin.
pub async fn exchange_code(
    origins: &Origins,
    code: &str,
    verifier: &str,
) -> Result<ExchangedKey, ExchangeError> {
    let client = crate::core::http_client::apply_global_proxy(
        reqwest::Client::builder().timeout(std::time::Duration::from_secs(EXCHANGE_TIMEOUT_SECS)),
    )
    .build()
    .map_err(|_| ExchangeError::Network)?;

    let body = serde_json::json!({
        "code": code,
        "code_verifier": verifier,
        "code_challenge_method": "S256",
    });

    let resp = client
        .post(origins.exchange_url())
        .json(&body)
        .send()
        .await
        .map_err(|_| ExchangeError::Network)?;

    let status = resp.status();
    let bytes = read_bounded(resp, EXCHANGE_MAX_BYTES)
        .await
        .map_err(|_| ExchangeError::Network)?;

    match status.as_u16() {
        200..=299 => {}
        400 => return Err(ExchangeError::Protocol),
        403 => return Err(ExchangeError::Rejected),
        429 => return Err(ExchangeError::RateLimited),
        _ => return Err(ExchangeError::Network),
    }

    let json: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| ExchangeError::Network)?;
    let key = json
        .get("key")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    if key.is_empty() {
        return Err(ExchangeError::Network);
    }
    let scope = json
        .get("scope")
        .and_then(|v| v.as_str())
        .unwrap_or("api")
        .to_string();
    let user_id = json
        .get("user_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Read the granted scope back. A narrower grant is not silently accepted.
    if !scope_satisfies(&scope, "api") {
        return Err(ExchangeError::Rejected);
    }

    Ok(ExchangedKey {
        key,
        user_id,
        scope,
    })
}

/// Read at most `cap` bytes, so a hostile or broken endpoint cannot exhaust
/// memory. Returns an error rather than truncating silently.
async fn read_bounded(mut resp: reqwest::Response, cap: usize) -> Result<Vec<u8>, String> {
    let mut out: Vec<u8> = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
        if out.len() + chunk.len() > cap {
            return Err("OrcaRouter: response exceeded the size cap".to_string());
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Model catalog
// ---------------------------------------------------------------------------

/// An input or output modality a model declares.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Modality {
    Text,
    Image,
    Audio,
    Video,
}

impl Modality {
    fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "text" => Some(Modality::Text),
            "image" => Some(Modality::Image),
            "audio" => Some(Modality::Audio),
            "video" => Some(Modality::Video),
            _ => None,
        }
    }
}

/// One model advertised by the OrcaRouter catalog, reduced to metadata this
/// client can act on. Unknown fields are dropped rather than guessed at.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelInfo {
    /// Kept verbatim, including the `vendor/` namespace.
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub context_length: Option<u64>,
    #[serde(default)]
    pub max_completion_tokens: Option<u64>,
    #[serde(default)]
    pub input_modalities: Vec<Modality>,
    #[serde(default)]
    pub output_modalities: Vec<Modality>,
    #[serde(default)]
    pub endpoint_types: Vec<String>,
    /// Only ever true when the catalog states it in a structured field.
    #[serde(default)]
    pub reasoning: bool,
    /// Only populated from a structured field or the verified seed.
    #[serde(default)]
    pub reasoning_efforts: Vec<String>,
}

/// An entrance that consumes models. Each one filters the catalog differently.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelCapability {
    /// Text chat/agent.
    Chat,
    /// Chat that additionally accepts the given non-text input modality.
    Multimodal(Modality),
    Embedding,
    ImageGeneration,
    VideoGeneration,
    Rerank,
}

impl ModelCapability {
    /// The `capability` query value this entrance asks the catalog for, when it
    /// has one. Multimodal entrances ask for `chat` and then filter locally,
    /// because the catalog's capability query does not express modality.
    pub fn query_value(self) -> &'static str {
        match self {
            ModelCapability::Chat | ModelCapability::Multimodal(_) => "chat",
            ModelCapability::Embedding => "embedding",
            ModelCapability::ImageGeneration => "image",
            ModelCapability::VideoGeneration => "video",
            ModelCapability::Rerank => "rerank",
        }
    }
}

/// Whether a model may be offered for an entrance.
///
/// Fails closed: a model whose metadata does not positively prove compatibility
/// is excluded, and nothing is inferred from the model's name.
pub fn satisfies(m: &ModelInfo, cap: ModelCapability) -> bool {
    let has = |t: &str| m.endpoint_types.iter().any(|e| e.eq_ignore_ascii_case(t));
    let is_chat = TEXT_ENDPOINT_TYPES.iter().any(|t| has(t))
        && !NON_CHAT_ENDPOINT_TYPES.iter().any(|t| has(t));
    match cap {
        ModelCapability::Chat => is_chat,
        ModelCapability::Multimodal(modality) => is_chat && m.input_modalities.contains(&modality),
        ModelCapability::Embedding => has("embedding") || has("embeddings"),
        ModelCapability::ImageGeneration => has("image-generation"),
        ModelCapability::VideoGeneration => has("openai-video"),
        ModelCapability::Rerank => has("jina-rerank"),
    }
}

/// Every model compatible with an entrance, in catalog order.
pub fn filter_for(models: &[ModelInfo], cap: ModelCapability) -> Vec<ModelInfo> {
    models
        .iter()
        .filter(|m| satisfies(m, cap))
        .cloned()
        .collect()
}

/// Re-validate a previously selected model against the current catalog.
///
/// A restored model ID that is no longer compatible must be cleared rather than
/// silently kept — the caller shows the user a "pick again" prompt.
pub fn resolve_selected(
    models: &[ModelInfo],
    cap: ModelCapability,
    selected: &str,
) -> Option<String> {
    let want = selected.trim();
    if want.is_empty() {
        return None;
    }
    models
        .iter()
        .any(|m| m.id == want && satisfies(m, cap))
        .then(|| want.to_string())
}

/// Where the catalog currently on screen came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogSource {
    /// `GET /v1/models` answered successfully. Authoritative.
    Live,
    /// The last successful live response, reused while the endpoint is down.
    LastKnownGood,
    /// The small verified seed. Only used when nothing better exists.
    Seed,
}

/// A catalog plus how much it can be trusted.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Catalog {
    pub models: Vec<ModelInfo>,
    pub source: CatalogSource,
    /// True whenever `source` is not `Live`; the UI must show this state.
    pub degraded: bool,
    #[serde(default)]
    pub fetched_at_ms: u64,
}

impl Catalog {
    /// The verified cold-start seed, used only when live discovery has never
    /// succeeded and no last-known-good catalog is on disk.
    ///
    /// This is deliberately small and deliberately conservative: the campaign's
    /// OrcaRouter integration spec names these five models as the practical
    /// seed, and states the reasoning-effort ladder for `openai/gpt-5.5`. Any
    /// metadata the spec does not state is left unset rather than guessed from
    /// the model name. Live discovery supplies the real, richer metadata (for
    /// example `deepseek/deepseek-v4.1-flash` genuinely advertises `text` and
    /// `image` input with a 1M context window).
    pub fn seed() -> Vec<ModelInfo> {
        let chat = vec!["openai".to_string()];
        let text = vec![Modality::Text];
        vec![
            ModelInfo {
                id: "openai/gpt-5.5".to_string(),
                name: "OpenAI: GPT-5.5".to_string(),
                context_length: None,
                max_completion_tokens: None,
                input_modalities: text.clone(),
                output_modalities: text.clone(),
                endpoint_types: chat.clone(),
                reasoning: true,
                reasoning_efforts: vec![
                    "low".into(),
                    "medium".into(),
                    "high".into(),
                    "xhigh".into(),
                ],
            },
            ModelInfo {
                id: "anthropic/claude-opus-4.8".to_string(),
                name: "Anthropic: Claude Opus 4.8".to_string(),
                context_length: None,
                max_completion_tokens: None,
                input_modalities: text.clone(),
                output_modalities: text.clone(),
                endpoint_types: chat.clone(),
                reasoning: true,
                reasoning_efforts: Vec::new(),
            },
            ModelInfo {
                id: "google/gemini-3.5-flash".to_string(),
                name: "Google: Gemini 3.5 Flash".to_string(),
                context_length: None,
                max_completion_tokens: None,
                input_modalities: text.clone(),
                output_modalities: text.clone(),
                endpoint_types: chat.clone(),
                reasoning: false,
                reasoning_efforts: Vec::new(),
            },
            ModelInfo {
                id: "deepseek/deepseek-v4-pro".to_string(),
                name: "DeepSeek: DeepSeek V4 Pro".to_string(),
                context_length: None,
                max_completion_tokens: None,
                input_modalities: text.clone(),
                output_modalities: text.clone(),
                endpoint_types: chat.clone(),
                reasoning: true,
                reasoning_efforts: Vec::new(),
            },
            ModelInfo {
                id: "orcarouter/auto".to_string(),
                name: "OrcaRouter: Auto".to_string(),
                context_length: None,
                max_completion_tokens: None,
                input_modalities: text.clone(),
                output_modalities: text,
                endpoint_types: chat,
                reasoning: false,
                reasoning_efforts: Vec::new(),
            },
        ]
    }

    pub fn seed_catalog() -> Catalog {
        Catalog {
            models: Self::seed(),
            source: CatalogSource::Seed,
            degraded: true,
            fetched_at_ms: 0,
        }
    }
}

/// Parse a raw `/v1/models` body into the bounded model list.
///
/// Drops anything whose shape is not understood, caps the item count, and
/// accepts only endpoint types and modalities this client can actually speak.
pub fn parse_catalog(bytes: &[u8]) -> Result<Vec<ModelInfo>, String> {
    let json: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|_| "OrcaRouter: catalog was not JSON".to_string())?;
    let items = json
        .get("data")
        .and_then(|d| d.as_array())
        .ok_or_else(|| "OrcaRouter: catalog had no data array".to_string())?;

    let mut out = Vec::new();
    for item in items.iter().take(CATALOG_MAX_ITEMS) {
        let Some(id) = item.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        let id = id.trim();
        if id.is_empty() {
            continue;
        }
        // A record that claims to be something other than a model is not one.
        if let Some(obj) = item.get("object").and_then(|v| v.as_str()) {
            if obj != "model" {
                continue;
            }
        }

        let endpoint_types: Vec<String> = item
            .get("supported_endpoint_types")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| s.trim().to_ascii_lowercase())
                    .filter(|s| SUPPORTED_ENDPOINT_TYPES.iter().any(|k| k == s))
                    .collect()
            })
            .unwrap_or_default();
        // A route this client cannot speak is not a route it may advertise.
        if endpoint_types.is_empty() {
            continue;
        }

        let arch = item.get("architecture");
        let read_mods = |key: &str| -> Vec<Modality> {
            arch.and_then(|a| a.get(key))
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str())
                        .filter_map(Modality::parse)
                        .collect()
                })
                .unwrap_or_default()
        };

        let name = item
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| id.to_string());

        // Reasoning is only ever true when the catalog states it structurally.
        let reasoning = item
            .get("reasoning")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
            || item
                .get("supported_parameters")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().any(|v| v.as_str() == Some("reasoning")))
                .unwrap_or(false);
        let reasoning_efforts = item
            .get("reasoning_efforts")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        out.push(ModelInfo {
            id: id.to_string(),
            name,
            context_length: item.get("context_length").and_then(|v| v.as_u64()),
            max_completion_tokens: item.get("max_completion_tokens").and_then(|v| v.as_u64()),
            input_modalities: read_mods("input_modalities"),
            output_modalities: read_mods("output_modalities"),
            endpoint_types,
            reasoning,
            reasoning_efforts,
        });
    }

    if items.len() > CATALOG_MAX_ITEMS {
        tracing::warn!(
            "[orcarouter] catalog had {} items; kept the first {}",
            items.len(),
            CATALOG_MAX_ITEMS
        );
    }
    if out.is_empty() {
        return Err("OrcaRouter: catalog had no usable models".to_string());
    }
    Ok(out)
}

/// Fetch and parse the live catalog. The API key, when present, is sent as a
/// bearer token so the response is scoped to the user's workspace.
pub async fn fetch_catalog(origins: &Origins, key: &str) -> Result<Vec<ModelInfo>, String> {
    let client = crate::core::http_client::apply_global_proxy(
        reqwest::Client::builder().timeout(std::time::Duration::from_secs(CATALOG_TIMEOUT_SECS)),
    )
    .build()
    .map_err(|_| "OrcaRouter: HTTP client error".to_string())?;

    let mut req = client.get(origins.api_url("/models"));
    if !key.is_empty() {
        req = req.bearer_auth(key);
    }
    let resp = req.send().await.map_err(|_| {
        // Deliberately not the underlying error: a transport error can echo the
        // request, and the request carries the key.
        "OrcaRouter: could not reach the model catalog".to_string()
    })?;

    if resp.status().as_u16() == 401 {
        return Err(NEEDS_REAUTH.to_string());
    }
    if !resp.status().is_success() {
        return Err(format!(
            "OrcaRouter: catalog returned HTTP {}",
            resp.status().as_u16()
        ));
    }

    let bytes = read_bounded(resp, CATALOG_MAX_BYTES).await?;
    parse_catalog(&bytes)
}

/// Error string used to signal that the exact credential generation must be
/// reauthorized. Kept as a constant so the UI, the tests and the 401 path agree.
pub const NEEDS_REAUTH: &str = "orcarouter_needs_reauth";

/// Where a user manages which models a given key may call.
pub const KEY_ACCESS_URL: &str = "https://www.orcarouter.ai/console/token";

/// Classify a failed relay response into something a user can act on.
///
/// A `401` is a credential problem. A `403 model_access_denied` is *not* — the
/// key is valid, it simply is not permitted to call that model, and the fix is
/// to pick another model (or widen the key's access). Reporting both as
/// "AI error (403)" would send the user to reconnect a working credential.
///
/// Returns `None` when the response is not a recognised OrcaRouter failure, so
/// the caller keeps its existing behaviour for every other provider.
pub fn classify_relay_error(status: u16, body: &str) -> Option<String> {
    let code = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            v.get("error")
                .and_then(|e| e.get("code"))
                .and_then(|c| c.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_default();

    match (status, code.as_str()) {
        (401, _) => Some(NEEDS_REAUTH.to_string()),
        (403, "model_access_denied") => Some(format!(
            "This OrcaRouter key is not allowed to use the selected model. Pick a different model, or grant this key access at {KEY_ACCESS_URL}"
        )),
        (403, _) => Some(format!(
            "OrcaRouter refused this request for the selected model. Check the key's model access at {KEY_ACCESS_URL}"
        )),
        (429, _) => Some(
            "OrcaRouter is rate limiting this key. Wait a moment and try again.".to_string(),
        ),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Last-known-good cache
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct CatalogFile {
    #[serde(default)]
    models: Vec<ModelInfo>,
    #[serde(default)]
    fetched_at_ms: u64,
}

fn catalog_path() -> Option<std::path::PathBuf> {
    crate::core::paths::app_data_dir().map(|d| d.join(CATALOG_FILE))
}

/// Persist the last successful live catalog. It is not secret, and it is what
/// keeps a fresh installation usable through a catalog outage.
pub fn save_last_known_good(models: &[ModelInfo], fetched_at_ms: u64) {
    let Some(path) = catalog_path() else { return };
    let file = CatalogFile {
        models: models.to_vec(),
        fetched_at_ms,
    };
    if let Ok(s) = serde_json::to_string(&file) {
        let _ = std::fs::write(path, s);
    }
}

/// Load the persisted last-known-good catalog, if any.
pub fn load_last_known_good() -> Option<Catalog> {
    let path = catalog_path()?;
    let raw = std::fs::read_to_string(path).ok()?;
    let file: CatalogFile = serde_json::from_str(&raw).ok()?;
    if file.models.is_empty() {
        return None;
    }
    Some(Catalog {
        models: file.models,
        source: CatalogSource::LastKnownGood,
        degraded: true,
        fetched_at_ms: file.fetched_at_ms,
    })
}

static CATALOG_CACHE: OnceLock<Mutex<Option<Catalog>>> = OnceLock::new();

fn catalog_cache() -> &'static Mutex<Option<Catalog>> {
    CATALOG_CACHE.get_or_init(|| Mutex::new(None))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Invalidate the in-memory catalog cache (used after a credential change).
pub fn invalidate_catalog_cache() {
    if let Ok(mut g) = catalog_cache().lock() {
        *g = None;
    }
}

/// Resolve the catalog for the given credential.
///
/// Live discovery is authoritative: when it succeeds its result is used whole,
/// with no seed entries mixed in. When it fails, the last-known-good catalog is
/// reused, and only if that is unavailable does the verified seed appear — both
/// marked `degraded` so the UI can say so.
pub async fn discover(origins: &Origins, key: &str, force: bool) -> Catalog {
    if !force {
        if let Ok(g) = catalog_cache().lock() {
            if let Some(c) = g.as_ref() {
                if c.source == CatalogSource::Live
                    && now_ms().saturating_sub(c.fetched_at_ms) < CATALOG_TTL_MS
                {
                    return c.clone();
                }
            }
        }
    }

    match fetch_catalog(origins, key).await {
        Ok(models) => {
            let cat = Catalog {
                models,
                source: CatalogSource::Live,
                degraded: false,
                fetched_at_ms: now_ms(),
            };
            save_last_known_good(&cat.models, cat.fetched_at_ms);
            if let Ok(mut g) = catalog_cache().lock() {
                *g = Some(cat.clone());
            }
            cat
        }
        Err(e) => {
            tracing::warn!("[orcarouter] live catalog unavailable: {}", e);
            let fallback = load_last_known_good().unwrap_or_else(Catalog::seed_catalog);
            if let Ok(mut g) = catalog_cache().lock() {
                *g = Some(fallback.clone());
            }
            fallback
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- origins -----------------------------------------------------------

    #[test]
    fn defaults_are_two_distinct_public_origins() {
        let o = origins_from(|_| None);
        assert_eq!(o.auth, "https://www.orcarouter.ai");
        assert_eq!(o.api, "https://api.orcarouter.ai/v1");
        // The auth endpoints are never on the inference origin.
        assert_eq!(
            o.exchange_url(),
            "https://www.orcarouter.ai/api/v1/auth/keys"
        );
        assert_ne!(o.exchange_url(), o.api_url("/auth/keys"));
    }

    /// The catalog is fetched from the inference origin, and the exchange is
    /// not: each entrance only ever talks to its own origin.
    #[test]
    fn auth_and_inference_requests_target_their_own_origins() {
        let o = origins_from(|_| None);
        assert_eq!(o.api_url("/models"), "https://api.orcarouter.ai/v1/models");
        assert_eq!(
            o.api_url("/chat/completions"),
            "https://api.orcarouter.ai/v1/chat/completions"
        );
        // Neither auth endpoint is reachable on the inference origin.
        assert!(!o.authorize_url().starts_with(&o.api));
        assert!(!o.exchange_url().starts_with(&o.api));
        // Nor is the catalog on the auth origin.
        assert!(!o.api_url("/models").starts_with(&o.auth));
    }

    #[test]
    fn an_api_override_moves_inference_without_moving_auth() {
        let o = origins_from(|k| {
            (k == ENV_API_BASE).then(|| "https://relay.internal/v1".to_string())
        });
        assert_eq!(o.api_url("/models"), "https://relay.internal/v1/models");
        // The auth origin is untouched, so credentials still go to the
        // documented place.
        assert_eq!(o.exchange_url(), "https://www.orcarouter.ai/api/v1/auth/keys");
    }

    #[test]
    fn shared_base_applies_to_both_origins() {
        let o = origins_from(|k| (k == ENV_SHARED_BASE).then(|| "https://orca.internal".into()));
        assert_eq!(o.auth, "https://orca.internal");
        assert_eq!(o.api, "https://orca.internal/v1");
    }

    #[test]
    fn explicit_overrides_beat_the_shared_base() {
        let o = origins_from(|k| match k {
            ENV_SHARED_BASE => Some("https://shared.internal".into()),
            ENV_AUTH_BASE => Some("https://auth.internal/".into()),
            ENV_API_BASE => Some("https://api.internal/v1".into()),
            _ => None,
        });
        assert_eq!(o.auth, "https://auth.internal");
        assert_eq!(o.api, "https://api.internal/v1");
    }

    #[test]
    fn api_override_without_a_path_gets_the_versioned_suffix() {
        let o = origins_from(|k| (k == ENV_API_BASE).then(|| "https://api.internal".into()));
        assert_eq!(o.api, "https://api.internal/v1");
        // The auth origin is untouched by an API-only override.
        assert_eq!(o.auth, DEFAULT_AUTH_BASE);
    }

    #[test]
    fn non_loopback_http_origins_are_rejected() {
        assert!(origin_is_allowed("https://api.orcarouter.ai"));
        assert!(origin_is_allowed("http://127.0.0.1:8080"));
        assert!(origin_is_allowed("http://localhost:3000"));
        assert!(!origin_is_allowed("http://api.orcarouter.ai"));
        assert!(!origin_is_allowed("ftp://api.orcarouter.ai"));
        assert!(!origin_is_allowed("https://user:pw@api.orcarouter.ai"));
        let bad = Origins {
            auth: "http://evil.example".into(),
            api: DEFAULT_API_BASE.into(),
        };
        assert!(validate_origins(&bad).is_err());
    }

    // -- PKCE --------------------------------------------------------------

    #[test]
    fn attempts_are_fresh_and_use_s256() {
        let a = new_attempt().unwrap();
        let b = new_attempt().unwrap();
        assert_ne!(a.verifier(), b.verifier());
        assert_ne!(a.state, b.state);
        assert_eq!(a.challenge, challenge_for(a.verifier()));
        // base64url, no padding.
        assert!(!a.challenge.contains('='));
        assert!(!a.challenge.contains('+'));
        assert!(!a.challenge.contains('/'));
        // 32 bytes -> 43 base64url chars; sha256 -> 43 chars.
        assert_eq!(a.verifier().len(), 43);
        assert_eq!(a.challenge.len(), 43);
        assert_eq!(a.state.len(), 22);
    }

    #[test]
    fn debug_never_prints_the_verifier() {
        let a = new_attempt().unwrap();
        let rendered = format!("{:?}", a);
        assert!(!rendered.contains(a.verifier()));
        assert!(rendered.contains(&a.challenge));
    }

    #[test]
    fn authorize_url_carries_the_challenge_but_not_the_verifier() {
        let origins = origins_from(|_| None);
        let a = new_attempt().unwrap();
        let url = authorize_url_loopback(&origins, 51733, &a).unwrap();
        assert!(url.starts_with("https://www.orcarouter.ai/auth?"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains(&a.challenge));
        assert!(url.contains(&a.state));
        assert!(url.contains("callback_url=http%3A%2F%2F127.0.0.1%3A51733%2Fcb"));
        // The verifier is the one thing that must never ride in a URL.
        assert!(!url.contains(a.verifier()));
    }

    #[test]
    fn oob_url_asks_for_the_out_of_band_code_with_s256() {
        let origins = origins_from(|_| None);
        let a = new_attempt().unwrap();
        let url = authorize_url_oob(&origins, &a).unwrap();
        assert!(url.contains("callback_url=oob"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(!url.contains(a.verifier()));
    }

    #[test]
    fn callback_rules_match_the_consent_endpoint() {
        assert!(callback_url_allowed("http://127.0.0.1:51733/cb"));
        assert!(callback_url_allowed("http://localhost:1/cb"));
        assert!(callback_url_allowed("https://example.com:8443/cb"));
        assert!(!callback_url_allowed("http://192.168.1.10:51733/cb"));
        assert!(!callback_url_allowed("http://user:pw@127.0.0.1/cb"));
        assert!(!callback_url_allowed("http://127.0.0.1/cb#frag"));
    }

    // -- credential interface ---------------------------------------------

    #[tokio::test]
    async fn api_key_adapter_yields_the_shared_credential_type() {
        let a = ApiKeyAdapter {
            key: "  sk-orca-abcdefghijklmnop  ".into(),
            generation: 7,
        };
        let c = a.obtain().await.unwrap();
        assert_eq!(c.method, AuthMethod::ApiKey);
        assert_eq!(c.key(), "sk-orca-abcdefghijklmnop");
        assert_eq!(c.generation, 7);
    }

    #[tokio::test]
    async fn api_key_adapter_rejects_a_malformed_key() {
        let a = ApiKeyAdapter {
            key: "not-a-key".into(),
            generation: 1,
        };
        let err = a.obtain().await.unwrap_err();
        assert!(err.contains(KEY_PREFIX));
        assert!(!err.contains("not-a-key"));
    }

    #[tokio::test]
    async fn pkce_adapter_yields_the_same_credential_type_as_the_api_key_adapter() {
        let p = PkceAdapter {
            exchanged: ExchangedKey {
                key: "sk-orca-pkceissuedkeyvalue".into(),
                user_id: Some("12345".into()),
                scope: "api".into(),
            },
            generation: 8,
        };
        let c = p.obtain().await.unwrap();
        assert_eq!(c.method, AuthMethod::Pkce);
        assert_eq!(c.key(), "sk-orca-pkceissuedkeyvalue");
        assert_eq!(c.account.as_deref(), Some("12345"));
        // Downstream only ever reads the key, so the two entrances are
        // interchangeable at the seam.
        assert_eq!(c.key(), "sk-orca-pkceissuedkeyvalue");
    }

    #[test]
    fn credential_debug_and_mask_never_expose_the_key() {
        let c = Credential::new(
            "sk-orca-supersecretvalue1234",
            AuthMethod::Pkce,
            Some("u".into()),
            Some("api".into()),
            3,
        );
        assert!(!format!("{:?}", c).contains("supersecret"));
        assert_eq!(c.masked(), "••••••••1234");
        assert!(!c.masked().contains("supersecret"));
    }

    #[test]
    fn key_shape_check_is_only_a_shape_check() {
        assert!(looks_like_key("sk-orca-abcdefghijklmnop"));
        assert!(!looks_like_key("sk-orca-"));
        assert!(!looks_like_key("sk-other-abcdefghijklmnop"));
        // A well-shaped key is still not claimed to be valid.
        assert!(looks_like_key("sk-orca-0000000000000000000"));
    }

    #[test]
    fn scope_downgrade_is_detected() {
        assert!(scope_satisfies("api", "api"));
        assert!(scope_satisfies("api connector", "api"));
        assert!(scope_satisfies("API", "api"));
        assert!(!scope_satisfies("connector", "api"));
        assert!(!scope_satisfies("", "api"));
    }

    #[test]
    fn exchange_errors_name_no_secret_material() {
        for e in [
            ExchangeError::Protocol,
            ExchangeError::Rejected,
            ExchangeError::RateLimited,
            ExchangeError::Network,
        ] {
            let m = e.message();
            assert!(!m.contains("sk-orca"));
            assert!(m.len() > 20, "message must be actionable: {m}");
        }
    }

    #[test]
    fn a_401_is_a_credential_problem() {
        let body = r#"{"error":{"code":"","message":"Invalid API key"}}"#;
        assert_eq!(classify_relay_error(401, body).unwrap(), NEEDS_REAUTH);
    }

    #[test]
    fn a_model_access_denial_is_not_reported_as_a_credential_problem() {
        // Observed from a real workspace: the key is valid, it simply may not
        // call that model. Reconnecting the key would not help.
        let body = r#"{"error":{"code":"model_access_denied","message":"This API key does not have access to model orcarouter/auto."}}"#;
        let msg = classify_relay_error(403, body).expect("403 must be classified");
        assert_ne!(msg, NEEDS_REAUTH);
        assert!(msg.contains("not allowed"), "{msg}");
        assert!(msg.contains("Pick a different model"), "{msg}");
        // It points at the place the access is actually managed.
        assert!(msg.contains(KEY_ACCESS_URL));
        // And it never echoes the key.
        assert!(!msg.contains("sk-orca"));
    }

    #[test]
    fn an_unrecognised_403_still_says_something_useful() {
        let msg = classify_relay_error(403, r#"{"error":{"code":"something_else"}}"#).unwrap();
        assert_ne!(msg, NEEDS_REAUTH);
        assert!(msg.contains(KEY_ACCESS_URL));
        // A body that is not even JSON must not panic.
        assert!(classify_relay_error(403, "not json").is_some());
    }

    #[test]
    fn rate_limiting_is_reported_as_transient() {
        let msg = classify_relay_error(429, "{}").unwrap();
        assert!(msg.contains("rate limiting"), "{msg}");
        assert_ne!(msg, NEEDS_REAUTH);
    }

    #[test]
    fn other_statuses_are_left_to_the_existing_path() {
        // Nothing here should claim to interpret a 500 or a 200.
        assert!(classify_relay_error(500, "{}").is_none());
        assert!(classify_relay_error(200, "{}").is_none());
        assert!(classify_relay_error(404, "{}").is_none());
    }

    // -- catalog -----------------------------------------------------------

    fn fixture() -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "object": "list",
            "data": [
                { "id": "deepseek/deepseek-v4-flash-0731", "object": "model",
                  "supported_endpoint_types": ["openai", "anthropic", "openai-response"],
                  "context_length": 1048576,
                  "architecture": { "input_modalities": ["text"], "output_modalities": ["text"] } },
                { "id": "deepseek/deepseek-v4.1-flash", "object": "model",
                  "name": "DeepSeek: DeepSeek V4.1 Flash",
                  "supported_endpoint_types": ["openai", "openai-response", "anthropic"],
                  "context_length": 1048576, "max_completion_tokens": 384000,
                  "architecture": { "input_modalities": ["text", "image"], "output_modalities": ["text"] } },
                { "id": "orcarouter/auto", "object": "model",
                  "supported_endpoint_types": ["openai", "openai-response", "anthropic", "gemini"] },
                { "id": "vendor/text-embed-3", "object": "model",
                  "supported_endpoint_types": ["embeddings"],
                  "architecture": { "input_modalities": ["text"] } },
                { "id": "vendor/img-gen-1", "object": "model",
                  "supported_endpoint_types": ["image-generation"],
                  "architecture": { "input_modalities": ["text"], "output_modalities": ["image"] } },
                { "id": "vendor/video-gen-1", "object": "model",
                  "supported_endpoint_types": ["openai-video"] },
                { "id": "vendor/rerank-1", "object": "model",
                  "supported_endpoint_types": ["jina-rerank"] },
                { "id": "vendor/exotic-1", "object": "model",
                  "supported_endpoint_types": ["some-unknown-transport"] },
                { "id": "", "object": "model", "supported_endpoint_types": ["openai"] },
                { "id": "vendor/not-a-model", "object": "embedding",
                  "supported_endpoint_types": ["openai"] }
            ]
        }))
        .unwrap()
    }

    #[test]
    fn catalog_parsing_drops_unknown_shapes() {
        let models = parse_catalog(&fixture()).unwrap();
        let ids: Vec<String> = models.iter().map(|m| m.id.clone()).collect();
        // Unknown transport, empty id, and non-model objects are all dropped.
        assert!(!ids.contains(&"vendor/exotic-1".to_string()));
        assert!(!ids.contains(&"vendor/not-a-model".to_string()));
        assert!(!ids.contains(&String::new()));
        assert!(ids.contains(&"orcarouter/auto".to_string()));
    }

    #[test]
    fn catalog_parsing_preserves_vendor_namespace_and_metadata() {
        let models = parse_catalog(&fixture()).unwrap();
        let m = models
            .iter()
            .find(|m| m.id == "deepseek/deepseek-v4.1-flash")
            .expect("model must survive parsing with its namespace intact");
        assert_eq!(m.name, "DeepSeek: DeepSeek V4.1 Flash");
        assert_eq!(m.context_length, Some(1048576));
        assert_eq!(m.max_completion_tokens, Some(384000));
        assert_eq!(m.input_modalities, vec![Modality::Text, Modality::Image]);
        assert_eq!(m.output_modalities, vec![Modality::Text]);
    }

    #[test]
    fn text_only_models_are_not_offered_as_multimodal() {
        let models = parse_catalog(&fixture()).unwrap();
        let multi: Vec<String> = filter_for(&models, ModelCapability::Multimodal(Modality::Image))
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(multi, vec!["deepseek/deepseek-v4.1-flash"]);
        // A model that never declared image input fails closed.
        assert!(!multi.contains(&"deepseek/deepseek-v4-flash-0731".to_string()));
        assert!(!multi.contains(&"orcarouter/auto".to_string()));
    }

    #[test]
    fn modality_filters_fail_closed_for_undeclared_modalities() {
        let models = parse_catalog(&fixture()).unwrap();
        assert!(filter_for(&models, ModelCapability::Multimodal(Modality::Audio)).is_empty());
        assert!(filter_for(&models, ModelCapability::Multimodal(Modality::Video)).is_empty());
    }

    #[test]
    fn each_capability_filters_to_its_own_models() {
        let models = parse_catalog(&fixture()).unwrap();
        let ids = |cap: ModelCapability| -> Vec<String> {
            filter_for(&models, cap).into_iter().map(|m| m.id).collect()
        };
        let chat = ids(ModelCapability::Chat);
        assert_eq!(
            chat,
            vec![
                "deepseek/deepseek-v4-flash-0731",
                "deepseek/deepseek-v4.1-flash",
                "orcarouter/auto"
            ]
        );
        // Non-text-dedicated routes must never leak into the chat dropdown.
        assert!(!chat.contains(&"vendor/img-gen-1".to_string()));
        assert!(!chat.contains(&"vendor/video-gen-1".to_string()));
        assert!(!chat.contains(&"vendor/rerank-1".to_string()));
        assert!(!chat.contains(&"vendor/text-embed-3".to_string()));

        assert_eq!(ids(ModelCapability::Embedding), vec!["vendor/text-embed-3"]);
        assert_eq!(
            ids(ModelCapability::ImageGeneration),
            vec!["vendor/img-gen-1"]
        );
        assert_eq!(
            ids(ModelCapability::VideoGeneration),
            vec!["vendor/video-gen-1"]
        );
        assert_eq!(ids(ModelCapability::Rerank), vec!["vendor/rerank-1"]);
    }

    #[test]
    fn capability_query_values_match_the_documented_catalog_filters() {
        assert_eq!(ModelCapability::Chat.query_value(), "chat");
        assert_eq!(
            ModelCapability::Multimodal(Modality::Image).query_value(),
            "chat"
        );
        assert_eq!(ModelCapability::Embedding.query_value(), "embedding");
        assert_eq!(ModelCapability::ImageGeneration.query_value(), "image");
        assert_eq!(ModelCapability::VideoGeneration.query_value(), "video");
        assert_eq!(ModelCapability::Rerank.query_value(), "rerank");
    }

    #[test]
    fn a_stale_selection_is_cleared_when_it_stops_being_compatible() {
        let models = parse_catalog(&fixture()).unwrap();
        // Still compatible: kept.
        assert_eq!(
            resolve_selected(&models, ModelCapability::Chat, "orcarouter/auto"),
            Some("orcarouter/auto".to_string())
        );
        // No longer in the catalog: cleared.
        assert_eq!(
            resolve_selected(&models, ModelCapability::Chat, "openai/gpt-5.5"),
            None
        );
        // In the catalog but not for this entrance: cleared.
        assert_eq!(
            resolve_selected(&models, ModelCapability::Chat, "vendor/img-gen-1"),
            None
        );
        assert_eq!(
            resolve_selected(
                &models,
                ModelCapability::Multimodal(Modality::Image),
                "orcarouter/auto"
            ),
            None
        );
        assert_eq!(resolve_selected(&models, ModelCapability::Chat, "  "), None);
    }

    #[test]
    fn catalog_parsing_rejects_unusable_bodies() {
        assert!(parse_catalog(b"not json").is_err());
        assert!(parse_catalog(b"{\"data\":[]}").is_err());
        assert!(parse_catalog(b"{\"object\":\"list\"}").is_err());
        assert!(parse_catalog(b"{\"data\":[{\"id\":\"x\"}]}").is_err());
    }

    #[test]
    fn the_verified_seed_keeps_its_reasoning_ladder_and_intact_ids() {
        let seed = Catalog::seed();
        let gpt = seed.iter().find(|m| m.id == "openai/gpt-5.5").unwrap();
        assert!(gpt.reasoning);
        assert_eq!(
            gpt.reasoning_efforts,
            vec!["low", "medium", "high", "xhigh"]
        );
        for id in [
            "openai/gpt-5.5",
            "anthropic/claude-opus-4.8",
            "google/gemini-3.5-flash",
            "deepseek/deepseek-v4-pro",
            "orcarouter/auto",
        ] {
            assert!(seed.iter().any(|m| m.id == id), "seed must keep {id}");
        }
        // Every seed entry is usable for the text entrance the app actually has.
        for m in &seed {
            assert!(
                satisfies(m, ModelCapability::Chat),
                "{} must be chat-capable",
                m.id
            );
        }
        // The seed never claims a modality it was not given.
        let audio = filter_for(&seed, ModelCapability::Multimodal(Modality::Audio));
        assert!(audio.is_empty());
    }

    #[test]
    fn the_seed_catalog_is_marked_degraded() {
        let c = Catalog::seed_catalog();
        assert!(c.degraded);
        assert_eq!(c.source, CatalogSource::Seed);
    }
}
