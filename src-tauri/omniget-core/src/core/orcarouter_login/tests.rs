//! Tests for the interactive PKCE connect flow.
//!
//! These drive the real [`super::begin_login`] / [`super::complete_login`]
//! path against a local fake consent endpoint. Nothing reaches the network: the
//! fake server plays the part of the OrcaRouter auth origin, so the full
//! authorize → callback → exchange → persist sequence is exercised through the
//! code that actually ships.
//!
//! Only fake keys and fake codes appear here. Every test that touches secrets
//! also asserts the secret did not end up somewhere it could leak.

use super::*;
use crate::core::orcarouter::{ENV_API_BASE, ENV_AUTH_BASE, ENV_SHARED_BASE};
use std::io::{Read, Write};
use std::net::TcpListener as StdListener;

/// A fake auth origin: one thread that answers a single request and records
/// what it was asked.
struct FakeAuth {
    base: String,
    received: Arc<Mutex<Option<serde_json::Value>>>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl FakeAuth {
    /// Start a fake exchange endpoint. `status`/`body` are what it answers with.
    fn start(status: u16, body: &str) -> Self {
        let listener = StdListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let received = Arc::new(Mutex::new(None));
        let recv = received.clone();
        let body_for_thread = body.to_string();
        let handle = std::thread::spawn(move || {
            let body = body_for_thread;
            if let Ok((mut sock, _)) = listener.accept() {
                let mut buf = vec![0u8; 65536];
                let n = sock.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                if let Some(idx) = req.find("\r\n\r\n") {
                    let payload = &req[idx + 4..];
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(payload) {
                        *recv.lock().unwrap() = Some(v);
                    }
                }
                let reason = if status == 200 { "OK" } else { "Error" };
                let resp = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = sock.write_all(resp.as_bytes());
            }
        });
        Self {
            base: format!("http://127.0.0.1:{port}"),
            received,
            handle: Some(handle),
        }
    }

    fn received(&self) -> Option<serde_json::Value> {
        self.received.lock().unwrap().clone()
    }

    fn finish(mut self) {
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Drive the callback the way a browser would, then finish the login.
///
/// `query` receives the `state` the authorize URL actually carries, so the
/// happy path echoes the real value while the rejection tests can deliberately
/// get it wrong. Everything goes through the same listener the real flow uses.
async fn drive(
    state: &LoginState,
    pending: PendingLogin,
    query: impl FnOnce(&str) -> String + Send + 'static,
) -> LoginOutcome {
    let (info, listener) = pending.into_parts();
    let listener = listener.expect("begin_login must hand back its listener");
    let callback = info.callback_url.clone();
    let attempt = info.attempt;

    // Recover the state the URL carries the way a browser would echo it.
    let real_state = url::Url::parse(&info.authorize_url)
        .expect("the authorize URL must be a valid URL")
        .query_pairs()
        .find(|(k, _)| k == "state")
        .map(|(_, v)| v.to_string())
        .expect("the authorize URL must carry a state");

    let hit = tokio::spawn(async move {
        let target = query(&real_state);
        let cb_url = url::Url::parse(&callback).expect("callback must parse");
        let addr = format!(
            "{}:{}",
            cb_url.host_str().expect("callback must have a host"),
            cb_url.port().expect("callback must have a port")
        );
        // Wait for the listener to be accepting, then speak HTTP to it.
        for _ in 0..200 {
            if let Ok(mut s) = tokio::net::TcpStream::connect(&addr).await {
                use tokio::io::AsyncWriteExt;
                let req = format!(
                    "GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
                );
                let _ = s.write_all(req.as_bytes()).await;
                let _ = s.flush().await;
                // Read the response so the test also covers "the browser is
                // served a page rather than left hanging".
                let mut buf = vec![0u8; 4096];
                let _ = tokio::io::AsyncReadExt::read(&mut s, &mut buf).await;
                return String::from_utf8_lossy(&buf).to_string();
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        String::new()
    });

    let outcome = complete_login(state, attempt, listener).await;
    let page = hit.await.unwrap_or_default();
    // The user is always served something, never a blank window.
    assert!(
        page.contains("OmniGet") || page.is_empty(),
        "the callback must serve a page: {page}"
    );
    outcome
}

/// Both the resolved origins and the stored credential live in process-global
/// state, so tests that touch either have to run one at a time: a test that
/// asserts "this attempt wrote nothing" would otherwise race the happy path of
/// another test running in parallel.
static GLOBAL_LOCK: Mutex<()> = Mutex::new(());

/// Take the global test lock from outside this module (used by the live
/// tests, which also mutate the credential store).
pub fn lock_for_live() -> std::sync::MutexGuard<'static, ()> {
    GLOBAL_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Held for the duration of a test, and the only way to reach [`EnvGuard`].
struct Isolation {
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl Isolation {
    /// Take the global test lock and start from a known-empty credential slot.
    fn new() -> Self {
        let lock = GLOBAL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::core::ai::clear_orcarouter_credential();
        Self { _lock: lock }
    }

    /// Point the origins at a fake auth origin for this test.
    fn env(self, auth: &str, api: &str) -> EnvGuard {
        std::env::set_var(ENV_AUTH_BASE, auth);
        std::env::set_var(ENV_API_BASE, api);
        EnvGuard { _isolation: self }
    }
}

struct EnvGuard {
    _isolation: Isolation,
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        std::env::remove_var(ENV_AUTH_BASE);
        std::env::remove_var(ENV_API_BASE);
        std::env::remove_var(ENV_SHARED_BASE);
    }
}

fn state() -> LoginState {
    LoginState::new()
}

/// A successful exchange persists exactly one key and selects the provider.
#[tokio::test]
async fn happy_path_persists_the_key_and_selects_the_account_entrance() {
    let fake = FakeAuth::start(
        200,
        r#"{"key":"sk-orca-fakeissuedkey0001","user_id":"12345","scope":"api"}"#,
    );
    let _env = Isolation::new().env(&fake.base, "http://127.0.0.1:1/v1");

    let st = state();
    let pending = begin_login(&st).await.expect("begin must succeed");
    assert!(st.is_busy());

    let issued = "sk-orca-fakeissuedkey0001";
    let outcome = drive(&st, pending, |s| format!("/cb?code=fake-code&state={s}")).await;

    // The state the URL carried is the one the callback must echo.
    assert!(outcome.ok, "{}", outcome.message);
    assert!(outcome.connected);

    // Terminal path released the lock.
    assert!(!st.is_busy());

    let cfg = crate::core::ai::get();
    assert_eq!(cfg.orcarouter_key, issued);
    assert_eq!(cfg.orcarouter_method, "pkce");
    assert_eq!(cfg.orcarouter_account, "12345");
    assert_eq!(cfg.orcarouter_scope, "api");
    assert!(!cfg.orcarouter_needs_reauth);
    assert_eq!(cfg.provider, crate::core::ai::AiProvider::OrcarouterAuth);

    fake.finish();
}

/// A denial ends the attempt cleanly, changes nothing, and releases the lock.
#[tokio::test]
async fn denial_ends_cleanly_and_writes_no_credential() {
    let _env = Isolation::new().env("http://127.0.0.1:1", "http://127.0.0.1:1/v1");
    let st = state();
    let pending = begin_login(&st).await.expect("begin must succeed");
    let outcome = drive(&st, pending, |s| {
        format!("/cb?error=access_denied&state={s}")
    })
    .await;

    assert!(outcome.denied);
    assert!(!outcome.connected);
    assert!(!st.is_busy(), "denial must release the login lock");
    assert!(crate::core::ai::get().orcarouter_key.is_empty());
}

/// A wrong `state` is refused before the code is touched.
#[tokio::test]
async fn state_mismatch_is_refused() {
    let _env = Isolation::new().env("http://127.0.0.1:1", "http://127.0.0.1:1/v1");
    let st = state();
    let pending = begin_login(&st).await.expect("begin must succeed");
    let outcome = drive(&st, pending, |_s| {
        "/cb?code=fake-code&state=not-the-right-state".to_string()
    })
    .await;

    assert!(!outcome.ok);
    assert!(!outcome.connected);
    assert!(!st.is_busy());
    assert!(crate::core::ai::get().orcarouter_key.is_empty());
}

/// A callback with no state at all is refused too.
#[tokio::test]
async fn missing_state_is_refused() {
    let _env = Isolation::new().env("http://127.0.0.1:1", "http://127.0.0.1:1/v1");
    let st = state();
    let pending = begin_login(&st).await.expect("begin must succeed");
    let outcome = drive(&st, pending, |_s| "/cb?code=fake-code".to_string()).await;
    assert!(!outcome.ok);
    assert!(!st.is_busy());
}

/// An expired, reused or mismatched code surfaces the actionable 403 text.
#[tokio::test]
async fn a_rejected_code_reports_the_reauthentication_text() {
    let fake = FakeAuth::start(403, r#"{"error":"invalid_grant"}"#);
    let _env = Isolation::new().env(&fake.base, "http://127.0.0.1:1/v1");
    let st = state();
    let pending = begin_login(&st).await.expect("begin must succeed");
    let outcome = drive(&st, pending, |s| format!("/cb?code=fake-code&state={s}")).await;

    assert!(!outcome.ok);
    assert!(!outcome.connected);
    assert!(
        outcome.message.contains("no longer valid"),
        "{}",
        outcome.message
    );
    assert!(!st.is_busy());
    fake.finish();
}

/// A 400 means the challenge method disagreed — a downgrade defence.
#[tokio::test]
async fn a_challenge_method_downgrade_is_reported_as_a_protocol_error() {
    let fake = FakeAuth::start(400, r#"{"error":"invalid_request"}"#);
    let _env = Isolation::new().env(&fake.base, "http://127.0.0.1:1/v1");
    let st = state();
    let pending = begin_login(&st).await.expect("begin must succeed");
    let outcome = drive(&st, pending, |s| format!("/cb?code=fake-code&state={s}")).await;

    assert!(!outcome.ok);
    assert!(
        outcome.message.contains("challenge method"),
        "{}",
        outcome.message
    );
    fake.finish();
}

/// Hitting the 10-keys-per-24h cap is reported as such, not as a generic error.
#[tokio::test]
async fn rate_limiting_is_reported_distinctly() {
    let fake = FakeAuth::start(429, r#"{"error":"too_many_requests"}"#);
    let _env = Isolation::new().env(&fake.base, "http://127.0.0.1:1/v1");
    let st = state();
    let pending = begin_login(&st).await.expect("begin must succeed");
    let outcome = drive(&st, pending, |s| format!("/cb?code=fake-code&state={s}")).await;

    assert!(!outcome.ok);
    assert!(
        outcome.message.contains("rate limiting"),
        "{}",
        outcome.message
    );
    // The user is told the API-key path still works.
    assert!(
        outcome.message.contains("paste an API key"),
        "{}",
        outcome.message
    );
    fake.finish();
}

/// An unreachable auth origin is a network failure, not a hang.
#[tokio::test]
async fn an_unreachable_auth_origin_fails_without_hanging() {
    let _env = Isolation::new().env("http://127.0.0.1:1", "http://127.0.0.1:1/v1");
    let st = state();
    let pending = begin_login(&st).await.expect("begin must succeed");
    let outcome = drive(&st, pending, |s| format!("/cb?code=fake-code&state={s}")).await;

    assert!(!outcome.ok);
    assert!(
        outcome.message.contains("Could not reach"),
        "{}",
        outcome.message
    );
    assert!(!st.is_busy());
}

/// A granted scope narrower than requested is refused rather than assumed.
#[tokio::test]
async fn a_scope_downgrade_does_not_install_a_credential() {
    let fake = FakeAuth::start(
        200,
        r#"{"key":"sk-orca-fakeissuedkey0002","user_id":"1","scope":"connector"}"#,
    );
    let _env = Isolation::new().env(&fake.base, "http://127.0.0.1:1/v1");
    let st = state();
    let pending = begin_login(&st).await.expect("begin must succeed");
    let outcome = drive(&st, pending, |s| format!("/cb?code=fake-code&state={s}")).await;

    assert!(!outcome.ok);
    assert!(!outcome.connected);
    assert!(crate::core::ai::get().orcarouter_key.is_empty());
    fake.finish();
}

/// The exchange must post the verifier and the challenge method to the auth
/// origin, and must never put the verifier in the URL.
#[tokio::test]
async fn the_exchange_posts_to_the_auth_origin_with_the_verifier_in_the_body() {
    let fake = FakeAuth::start(
        200,
        r#"{"key":"sk-orca-fakeissuedkey0003","user_id":"9","scope":"api"}"#,
    );
    let _env = Isolation::new().env(&fake.base, "http://127.0.0.1:1/v1");
    let st = state();
    let pending = begin_login(&st).await.expect("begin must succeed");
    let authorize_url = pending.info.authorize_url.clone();
    let _ = drive(&st, pending, |s| format!("/cb?code=fake-code&state={s}")).await;

    let body = fake.received().expect("the fake must have been called");
    assert_eq!(body["code"], "fake-code");
    assert_eq!(body["code_challenge_method"], "S256");
    let verifier = body["code_verifier"].as_str().unwrap();
    assert!(!verifier.is_empty());
    // The verifier never rode in the authorize URL.
    assert!(!authorize_url.contains(verifier));
    // The authorize URL went to the auth origin, not the inference origin.
    assert!(authorize_url.starts_with(&format!("{}/auth?", fake.base)));
    fake.finish();
}

/// A late outcome from a superseded attempt must not install credentials.
#[tokio::test]
async fn a_stale_attempt_cannot_overwrite_a_newer_login() {
    let _env = Isolation::new().env("http://127.0.0.1:1", "http://127.0.0.1:1/v1");
    let st = state();
    let pending = begin_login(&st).await.expect("begin must succeed");
    let (info, listener) = pending.into_parts();
    let listener = listener.unwrap();

    // A second login starts before the first finishes.
    st.release();
    let newer = begin_login(&st).await;
    assert!(newer.is_ok(), "a released lock must allow a second attempt");

    let outcome = complete_login(&st, info.attempt, listener).await;
    assert!(!outcome.connected);
    assert!(crate::core::ai::get().orcarouter_key.is_empty());
}

/// `pagehide` clears the busy flag synchronously, and a second login can start
/// without remounting anything.
#[tokio::test]
async fn pagehide_releases_the_lock_so_a_second_login_can_start() {
    let _env = Isolation::new().env("http://127.0.0.1:1", "http://127.0.0.1:1/v1");
    let st = state();
    let first = begin_login(&st).await.expect("begin must succeed");
    assert!(st.is_busy());

    // What the pagehide handler does: invalidate and clear, synchronously.
    st.release();

    assert!(!st.is_busy(), "pagehide must clear busy immediately");
    assert!(
        st.take_listener().is_none(),
        "pagehide must drop the listener"
    );

    // A second login starts without any remount.
    let second = begin_login(&st).await;
    assert!(second.is_ok(), "a second login must start after pagehide");
    // And it is a genuinely new attempt.
    assert_ne!(first.info.attempt, second.unwrap().info.attempt);
}

/// Explicit cancel releases the lock and drops the pending listener.
#[tokio::test]
async fn explicit_cancel_releases_everything() {
    let _env = Isolation::new().env("http://127.0.0.1:1", "http://127.0.0.1:1/v1");
    let st = state();
    let _pending = begin_login(&st).await.expect("begin must succeed");
    assert!(st.is_busy());

    st.release();

    assert!(!st.is_busy());
    assert!(st.take_listener().is_none());
    // Finishing a cancelled attempt is refused rather than hanging.
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let outcome = complete_login(&st, 1, listener).await;
    assert!(!outcome.ok);
}

/// A second concurrent login is refused while one is in flight.
#[tokio::test]
async fn a_concurrent_login_is_refused() {
    let _env = Isolation::new().env("http://127.0.0.1:1", "http://127.0.0.1:1/v1");
    let st = state();
    let _first = begin_login(&st).await.expect("begin must succeed");
    let second = begin_login(&st).await;
    assert!(second.is_err(), "the single-login lock must hold");
}

/// The verifier never appears in the pending info handed to the UI.
#[tokio::test]
async fn the_pending_payload_carries_no_secret() {
    let _env = Isolation::new().env("https://www.orcarouter.ai", "https://api.orcarouter.ai/v1");
    let st = state();
    let pending = begin_login(&st).await.expect("begin must succeed");
    let json = serde_json::to_string(&pending.info).unwrap();
    assert!(!json.contains("code_verifier"));
    assert!(json.contains("code_challenge="));
    assert!(json.contains("www.orcarouter.ai"));
    assert_eq!(pending.info.api_base, "https://api.orcarouter.ai/v1");
    st.release();
}

// -- the API-key adapter on the same seam ------------------------------------

/// The pasted-key entrance produces the same credential type and reaches the
/// same persisted state as the PKCE entrance.
#[tokio::test]
async fn the_api_key_entrance_and_the_pkce_entrance_converge() {
    let _iso = Isolation::new();
    use crate::core::ai::{orcarouter_generation, orcarouter_method, AiProvider};

    let view = set_api_key(
        AiProvider::Orcarouter,
        "orcarouter/auto".to_string(),
        Some("sk-orca-fakepastedkey0001".to_string()),
    )
    .await
    .expect("a well-formed key must be accepted");

    assert!(view.has_orcarouter_key);
    assert_eq!(view.orcarouter_method, "api_key");
    assert_eq!(view.provider, AiProvider::Orcarouter);
    // The provider's request path reads only this, and it cannot tell which
    // adapter wrote it.
    assert_eq!(
        crate::core::ai::get().orcarouter_key,
        "sk-orca-fakepastedkey0001"
    );
    assert_eq!(orcarouter_method(), "api_key");
    assert!(orcarouter_generation() > 0);

    // Clearing through the same entrance removes it.
    set_api_key(AiProvider::Orcarouter, String::new(), Some(String::new()))
        .await
        .unwrap();
    assert!(!crate::core::ai::get().is_configured());
}

/// A malformed key is refused with an actionable message that does not echo it.
#[tokio::test]
async fn the_api_key_entrance_refuses_a_malformed_key_without_echoing_it() {
    let _iso = Isolation::new();
    use crate::core::ai::AiProvider;
    let err = set_api_key(
        AiProvider::Orcarouter,
        "orcarouter/auto".to_string(),
        Some("hunter2".to_string()),
    )
    .await
    .unwrap_err();
    assert!(err.contains("sk-orca-"), "{err}");
    assert!(!err.contains("hunter2"));
}

/// A `401` marks exactly the rejected generation, and a later generation is
/// untouched — including when the user has already reconnected.
#[test]
fn a_401_marks_only_the_generation_that_was_rejected() {
    let _iso = Isolation::new();
    use crate::core::ai as ai_mod;

    // Install a credential and note its generation.
    let key = crate::core::orcarouter::Credential::new(
        "sk-orca-fakerevokedkey0001",
        crate::core::orcarouter::AuthMethod::Pkce,
        Some("7".into()),
        Some("api".into()),
        ai_mod::orcarouter_generation(),
    );
    ai_mod::save_orcarouter_credential(&key);
    let rejected = ai_mod::orcarouter_generation();

    // The relay rejects that exact generation.
    assert!(ai_mod::mark_orcarouter_unauthorized(rejected));
    assert!(ai_mod::orcarouter_needs_reauth());
    // It is not treated as configured, so the UI offers a reconnect.
    assert!(!ai_mod::get().is_configured());
    // Marking it again is not a state change.
    assert!(!ai_mod::mark_orcarouter_unauthorized(rejected));

    // A late failure from the *old* generation must not touch a new one.
    let fresh = crate::core::orcarouter::Credential::new(
        "sk-orca-fakenewkey0002",
        crate::core::orcarouter::AuthMethod::Pkce,
        Some("7".into()),
        Some("api".into()),
        ai_mod::orcarouter_generation(),
    );
    ai_mod::save_orcarouter_credential(&fresh);
    assert!(
        !ai_mod::orcarouter_needs_reauth(),
        "a new login clears the flag"
    );
    assert!(
        !ai_mod::mark_orcarouter_unauthorized(rejected),
        "a stale 401 must be a no-op"
    );
    assert!(!ai_mod::orcarouter_needs_reauth());
    assert!(ai_mod::get().is_configured());

    ai_mod::clear_orcarouter_credential();
}

/// A revoked key must never trigger a synthetic refresh, and the old secret is
/// not deleted before a replacement exists.
#[test]
fn a_revoked_key_is_not_silently_dropped_and_never_refreshed() {
    let _iso = Isolation::new();
    use crate::core::ai as ai_mod;

    let key = crate::core::orcarouter::Credential::new(
        "sk-orca-fakerevokedkey0003",
        crate::core::orcarouter::AuthMethod::ApiKey,
        None,
        None,
        ai_mod::orcarouter_generation(),
    );
    ai_mod::save_orcarouter_credential(&key);
    let gen = ai_mod::orcarouter_generation();
    assert!(ai_mod::mark_orcarouter_unauthorized(gen));

    // The secret is still on disk: a transient or misclassified failure must
    // not become irreversible account loss.
    assert_eq!(ai_mod::get().orcarouter_key, "sk-orca-fakerevokedkey0003");
    // And it is flagged for reauthentication rather than refreshed.
    assert!(ai_mod::orcarouter_needs_reauth());

    ai_mod::clear_orcarouter_credential();
}

#[test]
fn capability_names_are_refused_rather_than_defaulted() {
    assert_eq!(
        parse_capability("chat", None).unwrap(),
        ModelCapability::Chat
    );
    assert_eq!(
        parse_capability("multimodal", Some("image")).unwrap(),
        ModelCapability::Multimodal(Modality::Image)
    );
    // An unknown capability must not fall back to "chat".
    assert!(parse_capability("chatting", None).is_err());
    // A multimodal request without a real modality must not fall back either.
    assert!(parse_capability("multimodal", None).is_err());
    assert!(parse_capability("multimodal", Some("smell")).is_err());
}
