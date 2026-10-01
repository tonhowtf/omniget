//! The local profile: an ed25519 identity, a nickname and a skin. No account,
//! no e-mail, no password, no network.
//!
//! The public key is the identifier the multiplayer layer already expects, so
//! the profile is the one place in the app that answers "who is this machine".
//! Everything here is lazy: [`ProfileManager::new`] takes no arguments and
//! touches nothing, because `AppState` is built before the Tauri app exists;
//! the first call that needs the identity loads it and caches it, and every
//! call after that is a read lock and a clone.
//!
//! Error strings are stable codes the UI maps directly: `ERR_PROFILE_NICKNAME`,
//! `ERR_PROFILE_STORE`, `ERR_PROFILE_SECRET`, `ERR_PROFILE_SIGN`, optionally
//! followed by `": "` and a detail.

pub mod identity;
pub mod store;

use serde::Serialize;
use std::sync::RwLock;

pub use identity::{fingerprint, verify};
pub use identity::{ERR_SECRET, ERR_SIGN};
pub use store::{ProfileFile, Skin, ERR_NICKNAME, ERR_STORE};

/// The profile as the rest of the Rust side sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    pub public_key: [u8; 32],
    pub nickname: String,
    pub skin: Skin,
    pub created_at_ms: u64,
}

impl Profile {
    pub fn fingerprint(&self) -> String {
        identity::fingerprint(&self.public_key)
    }

    /// The shape the frontend gets. The key is base64 and the fingerprint comes
    /// precomputed so no caller has to reimplement the base32 grouping.
    pub fn view(&self) -> ProfileView {
        ProfileView {
            public_key_b64: identity::encode_public_key(&self.public_key),
            fingerprint: self.fingerprint(),
            nickname: self.nickname.clone(),
            skin: self.skin.clone(),
            created_at_ms: self.created_at_ms,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ProfileView {
    pub public_key_b64: String,
    pub fingerprint: String,
    pub nickname: String,
    pub skin: Skin,
    pub created_at_ms: u64,
}

/// What the frontend needs to introduce this machine to another one.
#[derive(Clone, Debug, Serialize)]
pub struct ProfilePublic {
    pub public_key_b64: String,
    pub fingerprint: String,
}

#[derive(Clone)]
struct Loaded {
    seed: [u8; 32],
    /// Derived from the seed once. Deriving it is a scalar multiplication, and
    /// `get` is meant to be a read lock and a clone.
    public_key: [u8; 32],
    file: ProfileFile,
}

impl Loaded {
    fn profile(&self) -> Profile {
        Profile {
            public_key: self.public_key,
            nickname: self.file.nickname.clone(),
            skin: self.file.skin.clone(),
            created_at_ms: self.file.created_at_ms,
        }
    }
}

#[derive(Default)]
pub struct ProfileManager {
    cached: RwLock<Option<Loaded>>,
}

impl ProfileManager {
    pub fn new() -> Self {
        Self {
            cached: RwLock::new(None),
        }
    }

    /// Run `f` against the loaded profile, loading it first if this is the
    /// first call. A poisoned lock is not a reason to lose the identity: the
    /// inner value is still valid, so it is taken as-is.
    fn with<T>(&self, f: impl FnOnce(&Loaded) -> T) -> Result<T, String> {
        if let Some(loaded) = self
            .cached
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
        {
            return Ok(f(loaded));
        }
        let mut guard = self.cached.write().unwrap_or_else(|p| p.into_inner());
        if guard.is_none() {
            *guard = Some(load_from_disk()?);
        }
        let loaded = guard.as_ref().expect("just loaded");
        Ok(f(loaded))
    }

    /// Mutate the stored half of the profile and persist it. The seed is never
    /// touched here.
    fn update(
        &self,
        f: impl FnOnce(&mut ProfileFile) -> Result<(), String>,
    ) -> Result<Profile, String> {
        let mut guard = self.cached.write().unwrap_or_else(|p| p.into_inner());
        if guard.is_none() {
            *guard = Some(load_from_disk()?);
        }
        let loaded = guard.as_mut().expect("just loaded");
        let mut candidate = loaded.file.clone();
        f(&mut candidate)?;
        // Disk first: if the write fails the cache must keep matching the file.
        store::save(&candidate)?;
        loaded.file = candidate;
        Ok(loaded.profile())
    }

    /// The whole profile, creating it on the first call.
    pub fn get(&self) -> Result<Profile, String> {
        self.with(|loaded| loaded.profile())
    }

    pub fn set_nickname(&self, nick: &str) -> Result<Profile, String> {
        let nickname = store::normalize_nickname(nick)?;
        self.update(move |file| {
            file.nickname = nickname;
            Ok(())
        })
    }

    pub fn set_skin(&self, id: &str, tint: [u8; 3]) -> Result<Profile, String> {
        let id = store::normalize_skin_id(id)?;
        self.update(move |file| {
            file.skin = Skin { id, tint };
            Ok(())
        })
    }

    /// Sign with the local identity. The seed never leaves this module.
    pub fn sign(&self, msg: &[u8]) -> Result<[u8; 64], String> {
        self.with(|loaded| identity::sign(&loaded.seed, msg))
    }

    pub fn public(&self) -> Result<ProfilePublic, String> {
        self.with(|loaded| ProfilePublic {
            public_key_b64: identity::encode_public_key(&loaded.public_key),
            fingerprint: identity::fingerprint(&loaded.public_key),
        })
    }
}

/// Load the seed and `profile.json`, creating either one if it is missing.
fn load_from_disk() -> Result<Loaded, String> {
    let seed = identity::load_or_create_seed()?;
    let file = match store::load()? {
        Some(file) => file,
        None => {
            let file = ProfileFile::new(store::now_ms());
            store::save(&file)?;
            file
        }
    };
    Ok(Loaded {
        public_key: identity::public_key(&seed),
        seed,
        file,
    })
}
