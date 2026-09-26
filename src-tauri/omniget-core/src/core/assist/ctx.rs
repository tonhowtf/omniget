//! Who is asking and what they may see. Computed in the backend from the bot
//! and the conversation, never taken from the model's input.
//!
//! Default policy (spec 02, "Memória que cresce com o usuário"):
//! - a direct conversation with bot B reads and writes `User` and `Bot(B)`;
//! - a group room reads and writes `Room(conv)` only; a bot's private notes
//!   and the user profile reach a room only through an explicit share
//!   (`groups` installs the resolver that knows about rooms and shares);
//! - `Project` scopes are never granted by this module: the project KB is a
//!   separate store.

use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};

/// The only principal of a desktop install. Kept explicit so every query
/// carries it and a future multi-user build has one place to change.
pub const LOCAL_USER: &str = "local-user";

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Scope {
    /// The user's personal profile, shared by the user's direct bots.
    User,
    /// One bot's private memory.
    Bot { bot: String },
    /// Shared memory of one group room.
    Room { conversation: String },
}

impl Scope {
    /// Stable string stored in the `scope` columns.
    pub fn key(&self) -> String {
        match self {
            Scope::User => "user".into(),
            Scope::Bot { bot } => format!("bot:{bot}"),
            Scope::Room { conversation } => format!("room:{conversation}"),
        }
    }

    pub fn parse(key: &str) -> Option<Scope> {
        if key == "user" {
            return Some(Scope::User);
        }
        if let Some(b) = key.strip_prefix("bot:").filter(|s| !s.is_empty()) {
            return Some(Scope::Bot { bot: b.into() });
        }
        key.strip_prefix("room:")
            .filter(|s| !s.is_empty())
            .map(|c| Scope::Room {
                conversation: c.into(),
            })
    }
}

/// Whether a conversation may touch a project folder at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ContextKind {
    /// Personal conversation: no workspace, project tools absent.
    #[default]
    Projectless,
    /// A folder the user picked for this conversation.
    Project,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssistCtx {
    pub principal: String,
    pub bot_id: Option<String>,
    pub conversation_id: Option<String>,
    pub run_id: Option<String>,
    pub readable: Vec<Scope>,
    pub writable: Vec<Scope>,
}

impl AssistCtx {
    /// The user acting from the UI (memory screen, exports): every scope of
    /// the local user is theirs.
    pub fn user() -> Self {
        Self {
            principal: LOCAL_USER.into(),
            bot_id: None,
            conversation_id: None,
            run_id: None,
            readable: Vec::new(),
            writable: Vec::new(),
        }
    }

    /// True for the UI principal, which is not limited to a scope list.
    pub fn is_user_ui(&self) -> bool {
        self.principal == LOCAL_USER
            && self.bot_id.is_none()
            && self.readable.is_empty()
            && self.writable.is_empty()
    }

    pub fn can_read(&self, scope: &Scope) -> bool {
        self.is_user_ui() || self.readable.contains(scope)
    }

    pub fn can_write(&self, scope: &Scope) -> bool {
        self.is_user_ui() || self.writable.contains(scope)
    }

    /// Scope keys for SQL `IN (...)` filters. Empty for the UI principal,
    /// which callers must treat as "all of the local user's scopes".
    pub fn readable_keys(&self) -> Vec<String> {
        self.readable.iter().map(Scope::key).collect()
    }

    pub fn with_run(mut self, run_id: Option<String>) -> Self {
        self.run_id = run_id;
        self
    }
}

/// Direct-conversation policy; what [`resolve`] uses until `groups` installs
/// its own resolver.
pub fn direct(bot: &str, conversation: Option<&str>) -> AssistCtx {
    let scopes = vec![
        Scope::User,
        Scope::Bot {
            bot: bot.to_string(),
        },
    ];
    AssistCtx {
        principal: LOCAL_USER.into(),
        bot_id: Some(bot.to_string()),
        conversation_id: conversation.map(str::to_string),
        run_id: None,
        readable: scopes.clone(),
        writable: scopes,
    }
}

type Resolver = Arc<dyn Fn(&str, Option<&str>) -> AssistCtx + Send + Sync>;

static RESOLVER: RwLock<Option<Resolver>> = RwLock::new(None);

/// `groups` installs the room-aware resolver at boot.
pub fn set_resolver(f: Resolver) {
    *RESOLVER.write().unwrap_or_else(|e| e.into_inner()) = Some(f);
}

/// The scopes bot `bot` has in `conversation`.
pub fn resolve(bot: &str, conversation: Option<&str>) -> AssistCtx {
    if let Some(c) = conversation.filter(|c| super::authority::external(c)) {
        return super::authority::context(c, bot);
    }
    let r = RESOLVER.read().unwrap_or_else(|e| e.into_inner()).clone();
    match r {
        Some(f) => f(bot, conversation),
        None => direct(bot, conversation),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_keys_round_trip() {
        for s in [
            Scope::User,
            Scope::Bot { bot: "b1".into() },
            Scope::Room {
                conversation: "c1".into(),
            },
        ] {
            assert_eq!(Scope::parse(&s.key()), Some(s));
        }
        assert_eq!(Scope::parse("bot:"), None);
        assert_eq!(Scope::parse("project:/x"), None);
    }

    #[test]
    fn a_direct_bot_reads_its_own_scope_and_not_another_bots() {
        let c = direct("reader", Some("conv"));
        assert!(c.can_read(&Scope::Bot {
            bot: "reader".into()
        }));
        assert!(!c.can_read(&Scope::Bot {
            bot: "other".into()
        }));
        assert!(!c.can_read(&Scope::Room {
            conversation: "conv".into()
        }));
    }
}
