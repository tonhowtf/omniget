use std::collections::HashMap;

use serde::{Deserialize, Serialize};

pub const HOPS_CEILING: u32 = 10;
pub const REQUEST_TTL_MS: u64 = 60 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CanvasLimits {
    pub max_hops: u32,
    pub max_messages_per_chain: u32,
    pub max_pending_asks_per_node: u32,
    pub max_ask_ms: u64,
    pub max_pair_exchanges: u32,
}

impl Default for CanvasLimits {
    fn default() -> Self {
        Self {
            max_hops: 6,
            max_messages_per_chain: 32,
            max_pending_asks_per_node: 4,
            max_ask_ms: 30 * 60 * 1000,
            max_pair_exchanges: 6,
        }
    }
}

impl CanvasLimits {
    pub fn validate(&self) -> Result<(), String> {
        if self.max_hops == 0
            || self.max_messages_per_chain == 0
            || self.max_pending_asks_per_node == 0
        {
            return Err("CANVAS_LIMITS: limits must allow at least one message".into());
        }
        if self.max_hops > HOPS_CEILING
            || self.max_messages_per_chain > 256
            || self.max_pending_asks_per_node > 16
        {
            return Err("CANVAS_LIMITS: limits above the app's ceiling".into());
        }
        if self.max_ask_ms < 5_000 || self.max_ask_ms > 4 * 60 * 60 * 1000 {
            return Err("CANVAS_LIMITS: ask time must be between 5 s and 4 h".into());
        }
        if self.max_pair_exchanges < 2 {
            return Err("CANVAS_LIMITS: the ping-pong guard needs at least 2 exchanges".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    SelfSend,
    Hops,
    Chain,
    Pending,
    PingPong,
    Paused,
}

impl Refusal {
    pub fn code(self) -> &'static str {
        match self {
            Refusal::SelfSend => "self",
            Refusal::Hops => "limit_hops",
            Refusal::Chain => "limit_chain",
            Refusal::Pending => "limit_pending",
            Refusal::PingPong => "limit_ping_pong",
            Refusal::Paused => "chain_paused",
        }
    }

    pub fn message(self, l: &CanvasLimits) -> String {
        match self {
            Refusal::SelfSend => "a node cannot message itself".into(),
            Refusal::Hops => format!(
                "this conversation is already {} hops deep; stop delegating and answer with what you have",
                l.max_hops
            ),
            Refusal::Chain => format!(
                "this chain already has {} messages; wait for the user",
                l.max_messages_per_chain
            ),
            Refusal::Pending => format!(
                "you already have {} open asks; wait for an answer first",
                l.max_pending_asks_per_node
            ),
            Refusal::PingPong => format!(
                "you and this peer exchanged more than {} messages in a row; the chain is paused until the user steps in",
                l.max_pair_exchanges
            ),
            Refusal::Paused => "this chain is paused until the user steps in".into(),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct ChainStats {
    messages: u32,
    pairs: HashMap<(String, String), u32>,
    paused: bool,
    last_ms: u64,
}

fn pair(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.to_string(), b.to_string())
    } else {
        (b.to_string(), a.to_string())
    }
}

#[derive(Debug, Default)]
pub struct Ledger {
    chains: HashMap<String, ChainStats>,
    requests: HashMap<(String, String), (Vec<String>, u64)>,
}

#[derive(Debug, Clone)]
pub struct Admission<'a> {
    pub from: &'a str,
    pub to: &'a str,
    pub chain: &'a str,
    pub hop: u32,
    pub open_asks: u32,
    pub new_asks: u32,
}

impl Ledger {
    pub fn replay(&self, from: &str, request_id: &str) -> Option<Vec<String>> {
        if request_id.is_empty() {
            return None;
        }
        self.requests
            .get(&(from.to_string(), request_id.to_string()))
            .map(|(ids, _)| ids.clone())
    }

    pub fn check(&self, l: &CanvasLimits, a: &Admission<'_>) -> Result<(), Refusal> {
        if a.from == a.to {
            return Err(Refusal::SelfSend);
        }
        if a.hop > l.max_hops {
            return Err(Refusal::Hops);
        }
        if a.new_asks > 0 && a.open_asks + a.new_asks > l.max_pending_asks_per_node {
            return Err(Refusal::Pending);
        }
        if let Some(c) = self.chains.get(a.chain) {
            if c.paused {
                return Err(Refusal::Paused);
            }
            if c.messages >= l.max_messages_per_chain {
                return Err(Refusal::Chain);
            }
            if c.pairs.get(&pair(a.from, a.to)).copied().unwrap_or(0) >= l.max_pair_exchanges {
                return Err(Refusal::PingPong);
            }
        }
        Ok(())
    }

    pub fn admit(
        &mut self,
        l: &CanvasLimits,
        a: &Admission<'_>,
        now_ms: u64,
    ) -> Result<(), Refusal> {
        if let Err(r) = self.check(l, a) {
            if r == Refusal::PingPong {
                if let Some(c) = self.chains.get_mut(a.chain) {
                    c.paused = true;
                }
            }
            return Err(r);
        }
        let c = self.chains.entry(a.chain.to_string()).or_default();
        c.messages += 1;
        *c.pairs.entry(pair(a.from, a.to)).or_insert(0) += 1;
        c.last_ms = now_ms;
        Ok(())
    }

    pub fn remember(&mut self, from: &str, request_id: &str, ids: Vec<String>, now_ms: u64) {
        if request_id.is_empty() {
            return;
        }
        self.requests
            .insert((from.to_string(), request_id.to_string()), (ids, now_ms));
    }

    pub fn is_paused(&self, chain: &str) -> bool {
        self.chains.get(chain).is_some_and(|c| c.paused)
    }

    pub fn resume(&mut self, chain: &str) {
        self.chains.remove(chain);
    }

    pub fn gc(&mut self, now_ms: u64) {
        self.requests
            .retain(|_, (_, t)| now_ms.saturating_sub(*t) < REQUEST_TTL_MS);
        self.chains
            .retain(|_, c| c.paused || now_ms.saturating_sub(c.last_ms) < REQUEST_TTL_MS);
    }
}

pub fn new_chain_id() -> String {
    format!("ch_{}", &uuid::Uuid::new_v4().simple().to_string()[..12])
}
