use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};

pub const LOCAL_USER: &str = "local-user";

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Scope {
    User,
    Bot { bot: String },
    Room { conversation: String },
}

impl Scope {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ContextKind {
    #[default]
    Projectless,
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
    pub fn is_user_ui(&self) -> bool {
        self.principal == LOCAL_USER
            && self.bot_id.is_none()
            && self.readable.is_empty()
            && self.writable.is_empty()
    }

    pub fn can_write(&self, scope: &Scope) -> bool {
        self.is_user_ui() || self.writable.contains(scope)
    }

    pub fn readable_keys(&self) -> Vec<String> {
        self.readable.iter().map(Scope::key).collect()
    }

    pub fn with_run(mut self, run_id: Option<String>) -> Self {
        self.run_id = run_id;
        self
    }
}

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

pub fn set_resolver(f: Resolver) {
    *RESOLVER.write().unwrap_or_else(|e| e.into_inner()) = Some(f);
}

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
