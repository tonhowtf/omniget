//! Live verification through the real provider code path.
//!
//! Ignored by default so the suite stays offline and deterministic; run with
//! `ORCAROUTER_API_KEY=… cargo test -p omniget-core --lib -- --ignored live`
//! against a real workspace.
//!
//! These are the checks the campaign requires to go through the provider that
//! ships, not through a standalone curl: they call the same `ai::chat` and the
//! same `orcarouter::discover` the application uses.

use super::*;
use crate::core::ai::{self, AiProvider};
use crate::core::orcarouter::{self, CatalogSource, ModelCapability};

/// Skip rather than fail when no key is present, so the default suite is green.
fn key() -> Option<String> {
    let k = std::env::var("ORCAROUTER_API_KEY").ok()?;
    let k = k.trim().to_string();
    (!k.is_empty()).then_some(k)
}

/// Persist a real key through the API-key adapter, exactly as the settings UI
/// does, and return the model to use.
async fn install_real_key(k: &str) -> String {
    ai::clear_orcarouter_credential();
    let adapter = crate::core::orcarouter::ApiKeyAdapter {
        key: k.to_string(),
        generation: 0,
    };
    let credential = adapter
        .obtain()
        .await
        .expect("a real sk-orca- key must pass the shape check");
    ai::save_orcarouter_credential(&credential);

    let origins = orcarouter::origins();
    let catalog = orcarouter::discover(&origins, credential.key(), true).await;
    assert_eq!(
        catalog.source,
        CatalogSource::Live,
        "live discovery must succeed with a real key"
    );
    let text = orcarouter::filter_for(&catalog.models, ModelCapability::Chat);
    assert!(
        !text.is_empty(),
        "the workspace must expose at least one text model"
    );
    // Prefer a vendor-namespaced model: the catalog is the workspace-wide
    // router directory, and an individual key may be scoped to a subset of it,
    // so a model that actually answers is the one that proves the path.
    let picked = text
        .iter()
        .find(|m| m.id.starts_with("deepseek/"))
        .or_else(|| text.iter().find(|m| !m.id.starts_with("orcarouter/")))
        .unwrap_or(&text[0]);
    picked.id.clone()
}

/// The model catalog really comes back from `{api}/v1/models`, parsed by the
/// shipping parser, with vendor namespaces intact.
#[tokio::test]
#[ignore = "requires a real ORCAROUTER_API_KEY"]
async fn live_catalog_is_fetched_and_parsed() {
    let Some(k) = key() else { return };
    let _iso = crate::core::orcarouter_login::tests::lock_for_live();
    let model = install_real_key(&k).await;

    let origins = orcarouter::origins();
    assert_eq!(origins.auth, "https://www.orcarouter.ai");
    assert_eq!(origins.api, "https://api.orcarouter.ai/v1");

    let catalog = orcarouter::discover(&origins, &k, true).await;
    assert!(!catalog.degraded);
    assert!(catalog.models.len() > 1);
    // Namespaced ids survive the parser verbatim.
    assert!(
        catalog.models.iter().any(|m| m.id.contains('/')),
        "model ids must keep their vendor namespace"
    );
    eprintln!(
        "[live] catalog: {} models, picked {model}",
        catalog.models.len()
    );

    // Every option offered for a text entrance is a text-generation route.
    let chat = orcarouter::filter_for(&catalog.models, ModelCapability::Chat);
    for m in &chat {
        assert!(
            m.endpoint_types.iter().any(|e| {
                matches!(
                    e.as_str(),
                    "openai" | "openai-response" | "anthropic" | "gemini"
                )
            }),
            "{} must expose a text-generation endpoint",
            m.id
        );
    }
    eprintln!("[live] text-capable options: {}", chat.len());
}

/// A real chat completion through `ai::chat` → the OrcaRouter relay.
#[tokio::test]
#[ignore = "requires a real ORCAROUTER_API_KEY"]
async fn live_chat_completion_through_the_provider() {
    let Some(k) = key() else { return };
    let _iso = crate::core::orcarouter_login::tests::lock_for_live();
    let model = install_real_key(&k).await;

    ai::set(
        AiProvider::Orcarouter,
        model.clone(),
        String::new(),
        None,
        None,
        None,
    );

    let out = ai::chat(
        "You are a connectivity test. Reply with the single word: ok.",
        "ping",
    )
    .await
    .expect("a real completion must succeed through the provider path");
    eprintln!("[live] model={model} reply={:?}", out.trim());
    assert!(!out.trim().is_empty());

    // The request must not have flipped the credential into needs-reauth.
    assert!(!ai::orcarouter_needs_reauth());
    assert!(ai::get().is_configured());

    ai::clear_orcarouter_credential();
}

/// The documented anti-pattern really is one: the exchange path lives on the
/// auth origin, and the inference origin 404s for it.
#[tokio::test]
#[ignore = "requires network access"]
async fn live_the_two_origins_are_genuinely_distinct() {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .unwrap();

    // The relay has no /auth/keys.
    let wrong = client
        .post("https://api.orcarouter.ai/v1/auth/keys")
        .json(&serde_json::json!({}))
        .send()
        .await
        .expect("the relay must answer");
    assert_eq!(
        wrong.status().as_u16(),
        404,
        "the inference origin must not serve the auth exchange"
    );

    // Auth lives on the www origin.
    let origins = orcarouter::origins();
    assert_eq!(
        origins.exchange_url(),
        "https://www.orcarouter.ai/api/v1/auth/keys"
    );
    assert_eq!(origins.authorize_url(), "https://www.orcarouter.ai/auth");
}

/// A revoked or bogus key is classified as a terminal reauthentication, and
/// never retried as if it were a refreshable token.
#[tokio::test]
#[ignore = "requires network access"]
async fn live_a_bad_key_is_a_terminal_reauth_not_a_refresh() {
    let _iso = crate::core::orcarouter_login::tests::lock_for_live();
    let origins = orcarouter::origins();
    let err = orcarouter::fetch_catalog(&origins, "sk-orca-definitelynotavalidkey")
        .await
        .expect_err("a bogus key must be refused");
    assert_eq!(err, orcarouter::NEEDS_REAUTH);

    // Through the provider path the same thing happens, and no refresh is
    // attempted: the call fails and the credential is flagged, once.
    ai::clear_orcarouter_credential();
    ai::save_orcarouter_credential(&crate::core::orcarouter::Credential::new(
        "sk-orca-definitelynotavalidkey",
        crate::core::orcarouter::AuthMethod::ApiKey,
        None,
        None,
        0,
    ));
    ai::set(
        AiProvider::Orcarouter,
        "orcarouter/auto".to_string(),
        String::new(),
        None,
        None,
        None,
    );
    let out = ai::chat("connectivity test", "ping").await;
    assert!(out.is_err());
    assert_eq!(out.unwrap_err(), orcarouter::NEEDS_REAUTH);
    assert!(ai::orcarouter_needs_reauth());
    // The secret is still stored: a rejected key is not silently deleted.
    assert!(!ai::get().orcarouter_key.is_empty());
    ai::clear_orcarouter_credential();
}
