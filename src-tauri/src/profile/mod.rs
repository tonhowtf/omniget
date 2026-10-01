pub mod identity;
pub mod store;

use serde::Serialize;
use std::sync::RwLock;

pub use identity::{fingerprint, verify};
pub use identity::{ERR_SECRET, ERR_SIGN};
pub use store::{ProfileFile, Skin, ERR_NICKNAME, ERR_STORE};

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

#[derive(Clone, Debug, Serialize)]
pub struct ProfilePublic {
    pub public_key_b64: String,
    pub fingerprint: String,
}

#[derive(Clone)]
struct Loaded {
    seed: [u8; 32],
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
        store::save(&candidate)?;
        loaded.file = candidate;
        Ok(loaded.profile())
    }

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
