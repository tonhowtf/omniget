use base64::Engine;
use data_encoding::BASE32_NOPAD;
use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};

use crate::secrets::{self, PROFILE};

pub const SEED_ACCOUNT: &str = "profile-seed";
const OMNIDISC_DEVICE_KEY: &str = "device-key";
const OMNIDISC_DEVICE_IDS: &str = "device-ids.json";

pub const ERR_SECRET: &str = "ERR_PROFILE_SECRET";

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

fn adoptable_device_seed() -> Option<(String, [u8; 32])> {
    let mut accounts: Vec<String> = omnidisc_instances()
        .into_iter()
        .map(|url| secrets::secret_account(&url, OMNIDISC_DEVICE_KEY))
        .collect();
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

pub fn fingerprint(public_key: &[u8; 32]) -> String {
    let digest = Sha256::digest(public_key);
    let encoded = BASE32_NOPAD.encode(&digest[..20]);
    encoded
        .as_bytes()
        .chunks(4)
        .map(|c| std::str::from_utf8(c).unwrap_or("????"))
        .collect::<Vec<_>>()
        .join("-")
}

pub fn encode_public_key(public_key: &[u8; 32]) -> String {
    b64().encode(public_key)
}
