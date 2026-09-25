//! The local identity: one ed25519 seed, kept in the secret store, never handed
//! to the frontend.
//!
//! The seed is the same 32 bytes `omnidisc-mls` turns into an MLS signer, so a
//! machine that already ran OmniDisc keeps the key it published to its
//! instances: [`load_or_create_seed`] adopts an existing `device-key` before it
//! generates anything. Signing goes through `omnidisc_mls::ed25519_dalek` on
//! purpose — one copy of the crate, one signature format.

use base64::Engine;
use omnidisc_mls::ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

use crate::secrets::{self, PROFILE};

/// Account name the profile seed lives under inside the profile namespace.
pub const SEED_ACCOUNT: &str = "profile-seed";
/// Account suffix OmniDisc uses for its per-instance device key.
const OMNIDISC_DEVICE_KEY: &str = "device-key";
/// Where OmniDisc records which instance got which device id. It is the only
/// enumerable list of instance URLs, and the keyring cannot be enumerated at
/// all, so it is how the migration finds candidate accounts.
const OMNIDISC_DEVICE_IDS: &str = "device-ids.json";

pub const ERR_SECRET: &str = "ERR_PROFILE_SECRET";
pub const ERR_SIGN: &str = "ERR_PROFILE_SIGN";

fn b64() -> base64::engine::general_purpose::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

pub fn encode_seed(seed: &[u8; 32]) -> String {
    b64().encode(seed)
}

pub fn decode_seed(encoded: &str) -> Result<[u8; 32], String> {
    let bytes = b64()
        .decode(encoded.trim())
        .map_err(|_| format!("{ERR_SECRET}: the stored profile key is unreadable"))?;
    if bytes.len() != 32 {
        return Err(format!(
            "{ERR_SECRET}: the stored profile key has the wrong size"
        ));
    }
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&bytes);
    Ok(seed)
}

/// Instance URLs OmniDisc has a device id for, oldest file order aside. Missing
/// file, unreadable file and wrong shape all mean "nothing to migrate".
fn omnidisc_instances() -> Vec<String> {
    let Ok(dir) = secrets::base_dir(secrets::OMNIDISC) else {
        return Vec::new();
    };
    let Ok(bytes) = std::fs::read(dir.join(OMNIDISC_DEVICE_IDS)) else {
        return Vec::new();
    };
    let map: std::collections::HashMap<String, String> = match serde_json::from_slice(&bytes) {
        Ok(m) => m,
        Err(_) => return Vec::new(),
    };
    let mut urls: Vec<String> = map.into_keys().collect();
    urls.sort();
    urls
}

/// An OmniDisc device seed to adopt, if this machine has one.
///
/// WHY adopt instead of generate: the public half of that key is already
/// registered on the user's instances and is inside their MLS groups. A fresh
/// profile key would give the same person two identities and leave the MLS one
/// orphaned. The OmniDisc copy is left exactly where it is; this only takes a
/// copy.
fn adoptable_device_seed() -> Option<(String, [u8; 32])> {
    let mut accounts: Vec<String> = omnidisc_instances()
        .into_iter()
        .map(|url| secrets::secret_account(&url, OMNIDISC_DEVICE_KEY))
        .collect();
    // The file store can be read directly, which covers a machine whose
    // device-ids.json was lost but whose sessions.bin survived.
    for account in secrets::file_store_accounts(secrets::OMNIDISC) {
        if account.ends_with(&format!("#{OMNIDISC_DEVICE_KEY}")) && !accounts.contains(&account) {
            accounts.push(account);
        }
    }
    for account in accounts {
        match secrets::load_secret(secrets::OMNIDISC, &account) {
            Ok(Some(encoded)) => match decode_seed(&encoded) {
                Ok(seed) => return Some((account, seed)),
                Err(e) => tracing::warn!("[profile] ignoring unreadable device key: {}", e),
            },
            Ok(None) => {}
            Err(e) => tracing::warn!("[profile] could not read {}: {}", account, e),
        }
    }
    None
}

/// Load the profile seed, adopting an OmniDisc device key on first run if there
/// is one and generating a fresh key otherwise.
pub fn load_or_create_seed() -> Result<[u8; 32], String> {
    if let Some(encoded) =
        secrets::load_secret(PROFILE, SEED_ACCOUNT).map_err(|e| format!("{ERR_SECRET}: {e}"))?
    {
        return decode_seed(&encoded);
    }

    let (seed, origin) = match adoptable_device_seed() {
        Some((account, seed)) => (seed, Some(account)),
        None => {
            let mut seed = [0u8; 32];
            rand::Rng::fill_bytes(&mut rand::rng(), &mut seed);
            (seed, None)
        }
    };

    secrets::save_secret(PROFILE, SEED_ACCOUNT, &encode_seed(&seed))
        .map_err(|e| format!("{ERR_SECRET}: {e}"))?;

    match origin {
        Some(account) => tracing::info!(
            "[profile] adopted the existing OmniDisc device key from {} as the local identity {}",
            account,
            fingerprint(&public_key(&seed))
        ),
        None => tracing::info!(
            "[profile] generated a new local identity {}",
            fingerprint(&public_key(&seed))
        ),
    }
    Ok(seed)
}

pub fn public_key(seed: &[u8; 32]) -> [u8; 32] {
    SigningKey::from_bytes(seed).verifying_key().to_bytes()
}

pub fn sign(seed: &[u8; 32], msg: &[u8]) -> [u8; 64] {
    SigningKey::from_bytes(seed).sign(msg).to_bytes()
}

pub fn verify(public_key: &[u8; 32], msg: &[u8], sig: &[u8; 64]) -> bool {
    match VerifyingKey::from_bytes(public_key) {
        Ok(key) => key.verify(msg, &Signature::from_bytes(sig)).is_ok(),
        Err(_) => false,
    }
}

/// Same spelling the OmniDisc device list already shows, so one person reading
/// both screens sees one identity.
pub fn fingerprint(public_key: &[u8; 32]) -> String {
    omnidisc_mls::fingerprint(public_key)
}

pub fn encode_public_key(public_key: &[u8; 32]) -> String {
    b64().encode(public_key)
}

/// Decode a base64 payload coming from the frontend. Anything that is not
/// base64 is a caller bug, not a crypto failure, but it still has to carry a
/// code the UI can map.
pub fn decode_b64(raw: &str) -> Result<Vec<u8>, String> {
    b64()
        .decode(raw.trim())
        .map_err(|_| format!("{ERR_SIGN}: the payload is not base64"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures_round_trip_and_do_not_transfer() {
        let seed = [3u8; 32];
        let other = [4u8; 32];
        let msg = b"the world is a nice place";
        let sig = sign(&seed, msg);
        assert!(verify(&public_key(&seed), msg, &sig));
        assert!(!verify(&public_key(&other), msg, &sig));
        assert!(!verify(&public_key(&seed), b"another message", &sig));

        let mut broken = sig;
        broken[0] ^= 0xff;
        assert!(!verify(&public_key(&seed), msg, &broken));
    }

    /// The seed is the MLS seed: the key the profile shows has to be the key
    /// the MLS client publishes, or the migration is a lie.
    #[test]
    fn the_public_key_matches_the_mls_client_for_the_same_seed() {
        let seed = [11u8; 32];
        let client = omnidisc_mls::MlsClient::new("0", "od-test", &seed).expect("mls client");
        assert_eq!(public_key(&seed), client.public_key());
        assert_eq!(fingerprint(&public_key(&seed)), client.fingerprint());
    }

    #[test]
    fn seeds_round_trip_and_bad_ones_are_refused() {
        let seed = [9u8; 32];
        assert_eq!(decode_seed(&encode_seed(&seed)).expect("decode"), seed);
        assert!(decode_seed("not base64!!").is_err());
        assert!(decode_seed(&encode_seed(&[1u8; 32])[..8]).is_err());
        let short = base64::engine::general_purpose::STANDARD.encode([1u8; 16]);
        let err = decode_seed(&short).unwrap_err();
        assert!(err.starts_with(ERR_SECRET), "{err}");
    }

    #[test]
    fn a_payload_that_is_not_base64_is_refused_with_a_code() {
        let err = decode_b64("!!! not base64 !!!").unwrap_err();
        assert!(err.starts_with(ERR_SIGN), "{err}");
        assert_eq!(decode_b64("aGk=").unwrap(), b"hi");
    }
}
