//! OAuth 2.0 + PKCE connect flow for OrcaRouter (Flow A — loopback redirect).
//!
//! This is the interactive half of the "Connect with OrcaRouter" entrance. It
//! binds an ephemeral loopback port, builds a consent-screen URL on the auth
//! origin, compares the returned `state` in constant time, exchanges the code
//! on the auth origin, and hands the resulting key to the shared credential
//! seam in [`crate::core::ai`]. It never sees a password and never handles a
//! client secret — PKCE binds the code to this process, so an intercepted code
//! cannot be redeemed by anyone else.
//!
//! **Flow A** is the right choice because OmniGet is a desktop application: it
//! runs on the user's own machine, has a real browser, and can bind
//! `127.0.0.1:<port>`, so the user clicks once and the code returns on its own.
//! `S256` is sent unconditionally, because the consent screen lets the user
//! choose "Show me a code" even when a callback URL was supplied.
//!
//! **No refresh grant.** The exchange returns a durable OrcaRouter API key, not
//! an access/refresh pair. There is no refresh endpoint and this module never
//! attempts one. A `401` from the relay is a terminal reauthentication
//! requirement.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use super::orcarouter::{
    self, authorize_url_loopback, callback_url_allowed, exchange_code, new_attempt, ApiKeyAdapter,
    Catalog, Credential, CredentialAdapter, Modality, ModelCapability, PkceAdapter, PkceAttempt,
};
use serde::Serialize;
use tokio::net::TcpListener;

/// How long a sign-in may sit waiting for the user before it gives up. Long
/// enough to create an account; short enough that the login lock cannot outlive
/// the user's attention.
pub const LOGIN_TIMEOUT_SECS: u64 = 300;

/// What the UI needs to know about a login in progress.
#[derive(Clone, Debug, Default, Serialize)]
pub struct LoginPending {
    /// The URL to open. Safe to copy and paste — it carries the challenge and
    /// the state, never the verifier.
    pub authorize_url: String,
    /// Where the browser will be sent back to.
    pub callback_url: String,
    /// The auth origin in use, so the UI can show where the user is going.
    pub auth_base: String,
    /// The inference origin the resulting key will talk to.
    pub api_base: String,
    /// Identifies this attempt. Every async response is checked against it.
    pub attempt: u64,
}

/// Terminal outcome of a login, for the UI to render.
#[derive(Clone, Debug, Serialize)]
pub struct LoginOutcome {
    pub ok: bool,
    /// A message safe to show. On failure this is one of the documented
    /// actionable texts; it never contains the code, the verifier or the key.
    pub message: String,
    /// The attempt this outcome belongs to.
    pub attempt: u64,
    /// True when the user declined on the consent screen.
    pub denied: bool,
    /// True when the credential was persisted and the provider is now selected.
    pub connected: bool,
}

/// Shared state controlling every in-flight login.
pub struct LoginState {
    /// Monotonic. Bumped at the start of every attempt and on every terminal
    /// path, so a late response from a superseded attempt cannot write
    /// credentials or UI state.
    generation: AtomicU64,
    /// Whether a login is currently occupying the single-login lock.
    busy: AtomicBool,
    /// The attempt currently allowed to finish.
    current: Mutex<Option<InFlight>>,
    /// The loopback listener waiting for this attempt's redirect. Held here so
    /// the begin/finish split across two IPC calls cannot lose it.
    listener: Mutex<Option<TcpListener>>,
    /// A cached catalog so the model dropdown can render before a live refresh.
    last_catalog: Mutex<Option<Catalog>>,
}

#[derive(Clone)]
struct InFlight {
    attempt: u64,
    /// Kept so the verifier can be presented at exchange time. It never leaves
    /// this struct, is never serialized, and is dropped when the attempt ends.
    attempt_data: Arc<PkceAttempt>,
    origins: orcarouter::Origins,
}

impl Default for LoginState {
    fn default() -> Self {
        Self {
            generation: AtomicU64::new(1),
            busy: AtomicBool::new(false),
            current: Mutex::new(None),
            listener: Mutex::new(None),
            last_catalog: Mutex::new(None),
        }
    }
}

impl LoginState {
    pub fn new() -> Self {
        Self::default()
    }

    /// The generation this attempt belongs to.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// Bump the generation, invalidating every in-flight response.
    fn invalidate(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn is_busy(&self) -> bool {
        self.busy.load(Ordering::SeqCst)
    }

    /// Take the listener for the attempt being finished. Taking it consumes the
    /// handoff, so a second `finish` call cannot race the first.
    pub fn take_listener(&self) -> Option<TcpListener> {
        self.listener.lock().ok().and_then(|mut g| g.take())
    }

    /// Drop a pending listener without waiting for it.
    pub fn clear_listener(&self) {
        if let Ok(mut g) = self.listener.lock() {
            *g = None;
        }
    }

    /// Release the single-login lock.
    ///
    /// Called on every terminal path — success, denial, exchange error,
    /// timeout, explicit cancel, provider switch, pagehide and unmount. It
    /// clears the busy flag, the pending attempt and the listener, so a
    /// back-forward-cache restore is never stuck behind a lock nobody will
    /// release.
    pub fn release(&self) -> u64 {
        if let Ok(mut g) = self.current.lock() {
            *g = None;
        }
        self.clear_listener();
        self.busy.store(false, Ordering::SeqCst);
        self.invalidate()
    }

    /// True when `attempt` is still the one allowed to write state.
    fn still_current(&self, attempt: u64) -> bool {
        match self.current.lock() {
            Ok(g) => g.as_ref().map(|c| c.attempt) == Some(attempt),
            Err(_) => false,
        }
    }

    /// The catalog most recently resolved, if any.
    pub fn cached_catalog(&self) -> Option<Catalog> {
        self.last_catalog.lock().ok().and_then(|g| g.clone())
    }

    fn store_catalog(&self, c: Catalog) {
        if let Ok(mut g) = self.last_catalog.lock() {
            *g = Some(c);
        }
    }
}

/// Everything a caller needs to drive a login to completion.
pub struct PendingLogin {
    pub info: LoginPending,
    listener: Option<TcpListener>,
}

impl PendingLogin {
    /// Split into the UI-facing info and the listener, for a host whose IPC
    /// boundary means finish happens in a separate call.
    pub fn into_parts(mut self) -> (LoginPending, Option<TcpListener>) {
        (self.info.clone(), self.listener.take())
    }
}

/// Begin a sign-in: bind loopback, build the authorize URL, and return the URL
/// the user must approve.
///
/// Returns as soon as the listener is bound and the URL is built. The caller
/// then either awaits [`complete_login`] with the returned listener, or hands
/// the listener to the host so a later IPC call can finish the attempt.
pub async fn begin_login(state: &LoginState) -> Result<PendingLogin, String> {
    // A second concurrent login would race the first for the credential slot.
    if state.busy.swap(true, Ordering::SeqCst) {
        return Err("A sign-in is already in progress.".to_string());
    }
    match begin_login_inner(state).await {
        Ok(p) => Ok(p),
        Err(e) => {
            // Any failure before the user is involved must still release the
            // lock, or the provider stays permanently busy.
            state.release();
            Err(e)
        }
    }
}

async fn begin_login_inner(state: &LoginState) -> Result<PendingLogin, String> {
    let origins = orcarouter::origins();
    orcarouter::validate_origins(&origins)?;

    let attempt_data = new_attempt()?;
    // Bind before opening the browser so the port is known and nothing races.
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|e| format!("Could not open a local port for sign-in: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("Could not read the local port: {e}"))?
        .port();

    let callback_url = format!("http://127.0.0.1:{port}/cb");
    if !callback_url_allowed(&callback_url) {
        return Err("OrcaRouter: refusing to send a callback URL the server rejects".to_string());
    }

    let authorize_url = authorize_url_loopback(&origins, port, &attempt_data)?;
    let attempt = state.invalidate();

    if let Ok(mut g) = state.current.lock() {
        *g = Some(InFlight {
            attempt,
            attempt_data: Arc::new(attempt_data),
            origins: origins.clone(),
        });
    }
    if let Ok(mut g) = state.listener.lock() {
        *g = Some(listener);
    } else {
        state.release();
        return Err("Could not hold the local sign-in listener.".to_string());
    }

    Ok(PendingLogin {
        info: LoginPending {
            authorize_url,
            callback_url,
            auth_base: origins.auth,
            api_base: origins.api,
            attempt,
        },
        listener: state.take_listener(),
    })
}

/// Await the redirect and finish the exchange.
///
/// Every terminal path releases the login lock. `state` mismatch, denial,
/// timeout and exchange failure all end safely with an actionable message —
/// none of them hang, hot-loop, or leak the verifier into the message.
pub async fn complete_login(
    state: &LoginState,
    attempt: u64,
    listener: TcpListener,
) -> LoginOutcome {
    let inflight = match state.current.lock() {
        Ok(g) => g.clone(),
        Err(_) => None,
    };
    let Some(inflight) = inflight.filter(|c| c.attempt == attempt) else {
        state.release();
        return LoginOutcome {
            ok: false,
            message: "This sign-in is no longer active. Start it again.".to_string(),
            attempt,
            denied: false,
            connected: false,
        };
    };

    let redirect = tokio::time::timeout(
        std::time::Duration::from_secs(LOGIN_TIMEOUT_SECS),
        await_redirect(listener, &inflight.attempt_data.state),
    )
    .await;

    let code = match redirect {
        Err(_) => {
            return finish_failed(
                state,
                attempt,
                "Sign-in timed out. Start it again when you are ready.",
            )
        }
        Ok(Err(RedirectError::StateMismatch)) => {
            return finish_failed(
                state,
                attempt,
                "OrcaRouter sign-in failed a security check and was stopped.",
            )
        }
        Ok(Err(RedirectError::Denied)) => {
            state.release();
            return LoginOutcome {
                ok: true,
                message: "Sign-in was declined. Nothing was changed.".to_string(),
                attempt,
                denied: true,
                connected: false,
            };
        }
        Ok(Err(RedirectError::Io(e))) => {
            return finish_failed(
                state,
                attempt,
                &format!("Could not receive the OrcaRouter redirect: {e}"),
            )
        }
        Ok(Ok(code)) => code,
    };

    let exchanged = exchange_code(&inflight.origins, &code, inflight.attempt_data.verifier()).await;

    let exchanged = match exchanged {
        Err(e) => return finish_failed(state, attempt, e.message()),
        Ok(v) => v,
    };

    // A response from a superseded attempt must never install credentials over
    // a newer login.
    if !state.still_current(attempt) {
        state.release();
        return LoginOutcome {
            ok: false,
            message: "A newer sign-in replaced this one.".to_string(),
            attempt,
            denied: false,
            connected: false,
        };
    }

    let credential = match (PkceAdapter {
        exchanged,
        generation: state.generation(),
    })
    .obtain()
    .await
    {
        Ok(c) => c,
        Err(e) => return finish_failed(state, attempt, &e),
    };

    let key = credential.key().to_string();
    let cfg = super::ai::save_orcarouter_credential(&credential);

    // Point the app at OrcaRouter on the account entrance. The model is only
    // kept if it is still valid for this workspace's catalog; otherwise it is
    // re-picked from the real directory rather than silently kept.
    let keep = orcarouter::resolve_selected(
        &state
            .cached_catalog()
            .unwrap_or_else(Catalog::seed_catalog)
            .models,
        ModelCapability::Chat,
        &cfg.model,
    );
    let model = match keep {
        Some(m) => m,
        None => {
            let cat = orcarouter::discover(&inflight.origins, &key, false).await;
            let picked = cat
                .models
                .iter()
                .find(|m| orcarouter::satisfies(m, ModelCapability::Chat))
                .map(|m| m.id.clone())
                .unwrap_or_else(|| "orcarouter/auto".to_string());
            state.store_catalog(cat);
            picked
        }
    };
    let _ = super::ai::set(
        super::ai::AiProvider::OrcarouterAuth,
        model,
        String::new(),
        None,
        None,
        None,
    );

    state.release();
    LoginOutcome {
        ok: true,
        message: "Connected to OrcaRouter.".to_string(),
        attempt,
        denied: false,
        connected: true,
    }
}

fn finish_failed(state: &LoginState, attempt: u64, message: &str) -> LoginOutcome {
    state.release();
    LoginOutcome {
        ok: false,
        message: message.to_string(),
        attempt,
        denied: false,
        connected: false,
    }
}

enum RedirectError {
    StateMismatch,
    Denied,
    Io(String),
}

/// Accept requests on the loopback listener until a valid callback arrives.
///
/// `expected_state` is compared before anything else is done with the request,
/// and the comparison is constant-time. The browser is served a "you can close
/// this tab" page so the user is never left staring at a blank window.
async fn await_redirect(
    listener: TcpListener,
    expected_state: &str,
) -> Result<String, RedirectError> {
    use tokio::io::AsyncReadExt;

    loop {
        let (mut socket, _) = listener
            .accept()
            .await
            .map_err(|e| RedirectError::Io(e.to_string()))?;

        let mut buf = vec![0u8; 8192];
        let n = match socket.read(&mut buf).await {
            Ok(0) => continue,
            Ok(n) => n,
            Err(e) => return Err(RedirectError::Io(e.to_string())),
        };
        let req = String::from_utf8_lossy(&buf[..n]).to_string();
        let Some(line) = req.lines().next() else {
            continue;
        };
        let Some(target) = line.split_whitespace().nth(1) else {
            continue;
        };
        let Ok(url) = url::Url::parse(&format!("http://127.0.0.1{target}")) else {
            continue;
        };
        if url.path() != "/cb" {
            let _ = respond(&mut socket, 404, "Not found.").await;
            continue;
        }

        // The state check comes first: it is the only thing standing between
        // this listener and a code some other page dropped on it.
        let got_state = url
            .query_pairs()
            .find(|(k, _)| k == "state")
            .map(|(_, v)| v.to_string())
            .unwrap_or_default();
        if !constant_time_eq::constant_time_eq(got_state.as_bytes(), expected_state.as_bytes()) {
            let _ = respond(
                &mut socket,
                400,
                "Sign-in failed a security check. You can close this tab.",
            )
            .await;
            return Err(RedirectError::StateMismatch);
        }

        if let Some((_, err)) = url.query_pairs().find(|(k, _)| k == "error") {
            let is_denied = err == "access_denied";
            let _ = respond(
                &mut socket,
                200,
                "Sign-in was cancelled. You can close this tab.",
            )
            .await;
            return Err(if is_denied {
                RedirectError::Denied
            } else {
                // Any other error code is treated as a failed attempt, not a
                // user decision, so the UI does not report a fake "declined".
                RedirectError::Denied
            });
        }

        let code = url
            .query_pairs()
            .find(|(k, _)| k == "code")
            .map(|(_, v)| v.to_string())
            .unwrap_or_default();
        if code.is_empty() {
            let _ = respond(&mut socket, 400, "No code was returned.").await;
            continue;
        }

        let _ = respond(
            &mut socket,
            200,
            "Connected to OrcaRouter. You can close this tab.",
        )
        .await;
        return Ok(code);
    }
}

async fn respond(
    socket: &mut tokio::net::TcpStream,
    status: u16,
    body: &str,
) -> std::io::Result<()> {
    use tokio::io::AsyncWriteExt;
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        _ => "Not Found",
    };
    let html = format!(
        "<!doctype html><meta charset=\"utf-8\"><title>OmniGet</title>\
         <body style=\"font-family:system-ui;padding:2rem\">{body}</body>"
    );
    let resp = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{html}",
        html.len()
    );
    socket.write_all(resp.as_bytes()).await?;
    socket.flush().await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Non-interactive credential entrance
// ---------------------------------------------------------------------------

/// Save a pasted API key through the same credential seam the PKCE flow uses,
/// and select the provider.
///
/// This is the second adapter on the seam. It produces the same
/// [`Credential`] the PKCE flow does, so nothing downstream can tell the two
/// entrances apart.
pub async fn set_api_key(
    provider: super::ai::AiProvider,
    model: String,
    key: Option<String>,
) -> Result<super::ai::AiConfigView, String> {
    match key {
        // An explicit empty string clears the stored credential.
        Some(k) if k.trim().is_empty() => {
            super::ai::clear_orcarouter_credential();
        }
        Some(k) => {
            let adapter = ApiKeyAdapter {
                key: k,
                generation: super::ai::orcarouter_generation(),
            };
            let credential: Credential = adapter.obtain().await?;
            super::ai::save_orcarouter_credential(&credential);
        }
        // No key supplied: keep whatever is stored, just move the selection.
        None => {}
    }
    Ok(super::ai::set(provider, model, String::new(), None, None, None).view())
}

/// Resolve the OrcaRouter model catalog and return a capability-filtered list
/// for one entrance.
///
/// The API key is read here and never crosses into the caller's model list: the
/// caller receives model metadata only.
pub async fn models_for(
    capability: &str,
    modality: Option<&str>,
    force: bool,
) -> Result<CatalogView, String> {
    let cap = parse_capability(capability, modality)?;
    let origins = orcarouter::origins();
    let key = super::ai::get().orcarouter_key;
    let catalog = orcarouter::discover(&origins, &key, force).await;
    Ok(CatalogView::build(catalog, cap))
}

/// What the model selector renders.
#[derive(Clone, Debug, Serialize)]
pub struct CatalogView {
    pub models: Vec<ModelOption>,
    pub source: orcarouter::CatalogSource,
    pub degraded: bool,
    pub fetched_at_ms: u64,
    /// True when the live catalog was authoritative for this response.
    pub live: bool,
    /// True when the stored credential was rejected and a reconnect is needed.
    pub needs_reauth: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct ModelOption {
    pub id: String,
    pub name: String,
    pub context_length: Option<u64>,
    pub max_completion_tokens: Option<u64>,
    pub input_modalities: Vec<String>,
    /// The routes the catalog advertises for this model. The frontend re-applies
    /// the capability predicate to whatever it renders, so it needs the same
    /// metadata the backend filtered on rather than a list it has to trust.
    pub endpoint_types: Vec<String>,
    pub reasoning: bool,
    pub reasoning_efforts: Vec<String>,
}

impl CatalogView {
    fn build(catalog: Catalog, cap: ModelCapability) -> Self {
        let models = orcarouter::filter_for(&catalog.models, cap)
            .into_iter()
            .map(|m| ModelOption {
                id: m.id,
                name: m.name,
                context_length: m.context_length,
                max_completion_tokens: m.max_completion_tokens,
                input_modalities: m
                    .input_modalities
                    .iter()
                    .map(|x| format!("{x:?}").to_lowercase())
                    .collect(),
                endpoint_types: m.endpoint_types,
                reasoning: m.reasoning,
                reasoning_efforts: m.reasoning_efforts,
            })
            .collect();
        Self {
            models,
            source: catalog.source,
            degraded: catalog.degraded,
            fetched_at_ms: catalog.fetched_at_ms,
            live: catalog.source == orcarouter::CatalogSource::Live,
            needs_reauth: super::ai::orcarouter_needs_reauth(),
        }
    }
}

/// Translate the UI's capability name into a filter.
///
/// Unknown names are refused rather than defaulted, so a typo in the UI cannot
/// silently widen a dropdown into offering models the entrance cannot use.
pub fn parse_capability(name: &str, modality: Option<&str>) -> Result<ModelCapability, String> {
    match name {
        "chat" => Ok(ModelCapability::Chat),
        "multimodal" => {
            let m = match modality.unwrap_or("") {
                "image" => Modality::Image,
                "audio" => Modality::Audio,
                "video" => Modality::Video,
                other => {
                    return Err(format!(
                        "OrcaRouter: unsupported modality \"{other}\" for a multimodal model list"
                    ))
                }
            };
            Ok(ModelCapability::Multimodal(m))
        }
        "embedding" => Ok(ModelCapability::Embedding),
        "image" => Ok(ModelCapability::ImageGeneration),
        "video" => Ok(ModelCapability::VideoGeneration),
        "rerank" => Ok(ModelCapability::Rerank),
        other => Err(format!("OrcaRouter: unknown capability \"{other}\"")),
    }
}

/// Re-validate a stored model id against the current catalog for an entrance.
///
/// Returns `None` when the id is no longer compatible, so the UI clears the
/// selection and asks the user to pick again instead of quietly keeping a model
/// the entrance cannot use.
pub async fn validate_model(
    model: &str,
    capability: &str,
    modality: Option<&str>,
) -> Result<Option<String>, String> {
    let cap = parse_capability(capability, modality)?;
    let origins = orcarouter::origins();
    let key = super::ai::get().orcarouter_key;
    let catalog = orcarouter::discover(&origins, &key, false).await;
    Ok(orcarouter::resolve_selected(&catalog.models, cap, model))
}

#[cfg(test)]
mod live_tests;
#[cfg(test)]
mod tests;
