use serde::{Deserialize, Serialize};

use crate::core::term::agent::{AgentSignal, AgentState, SignalSource};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeState {
    Starting,
    Ready,
    Working,
    NeedsYou,
    Done,
    Stuck,
    Exited,
    NoSignal,
}

impl NodeState {
    pub fn as_str(self) -> &'static str {
        match self {
            NodeState::Starting => "starting",
            NodeState::Ready => "ready",
            NodeState::Working => "working",
            NodeState::NeedsYou => "needs_you",
            NodeState::Done => "done",
            NodeState::Stuck => "stuck",
            NodeState::Exited => "exited",
            NodeState::NoSignal => "no_signal",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    None,
    Estimated,
    Reported,
    Exact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Hook,
    Bus,
    Osc,
    Screen,
    Exit,
    Canvas,
    None,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fused {
    pub state: NodeState,
    pub source: Source,
    pub confidence: Confidence,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub since_ms: i64,
}

#[derive(Debug, Clone, Default)]
pub struct Inputs {
    pub signal: Option<AgentSignal>,
    pub alive: bool,
    pub started_ms: i64,
    pub from_bus: bool,
    pub stuck: bool,
    pub seen: bool,
    pub permission_pending: bool,
    pub refused: bool,
}

pub const STARTING_GRACE_MS: i64 = 8_000;

fn confidence_of(src: SignalSource, from_bus: bool) -> (Source, Confidence) {
    if from_bus {
        return (Source::Bus, Confidence::Exact);
    }
    match src {
        SignalSource::Hook => (Source::Hook, Confidence::Exact),
        SignalSource::Exit => (Source::Exit, Confidence::Exact),
        SignalSource::Osc | SignalSource::Title => (Source::Osc, Confidence::Reported),
        SignalSource::Heuristic => (Source::Screen, Confidence::Estimated),
        SignalSource::None => (Source::None, Confidence::None),
    }
}

pub fn failure_detail(message: Option<&str>) -> &'static str {
    let lower = message.unwrap_or("").to_ascii_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
        .collect();
    let has = |codes: &[&str]| words.iter().any(|w| codes.contains(w));
    let pair = |a: &str, b: &str| words.windows(2).any(|w| w[0] == a && w[1] == b);
    if has(&[
        "rate_limit",
        "rate_limit_error",
        "usage_limit",
        "billing_error",
        "insufficient_quota",
        "quota_exceeded",
    ]) || pair("usage", "limit")
        || pair("rate", "limit")
    {
        "limit"
    } else if has(&["refusal"]) {
        "refused"
    } else {
        "failed"
    }
}

pub fn fuse(i: &Inputs, now_ms: i64) -> Fused {
    let since = i.signal.as_ref().map(|s| s.since_ms).unwrap_or(now_ms);
    let mk = |state, source, confidence, detail: Option<&str>| Fused {
        state,
        source,
        confidence,
        detail: detail.map(str::to_string),
        since_ms: since,
    };
    if !i.alive {
        return mk(NodeState::Exited, Source::Exit, Confidence::Exact, None);
    }
    if i.stuck {
        return mk(
            NodeState::Stuck,
            Source::Canvas,
            Confidence::Exact,
            Some("delivery_stuck"),
        );
    }
    if i.permission_pending {
        return mk(
            NodeState::NeedsYou,
            Source::Hook,
            Confidence::Exact,
            Some("permission"),
        );
    }
    let Some(sig) = &i.signal else {
        return if now_ms - i.started_ms < STARTING_GRACE_MS {
            mk(NodeState::Starting, Source::None, Confidence::None, None)
        } else {
            mk(NodeState::NoSignal, Source::None, Confidence::None, None)
        };
    };
    let (source, conf) = confidence_of(sig.source, i.from_bus);
    let msg = sig.message.as_deref();
    match sig.state {
        AgentState::Blocked => mk(NodeState::NeedsYou, source, conf, msg),
        AgentState::Thinking | AgentState::Working | AgentState::Looping => {
            mk(NodeState::Working, source, conf, msg)
        }
        AgentState::Compacting => mk(NodeState::Working, source, conf, Some("compacting")),
        AgentState::WaitingPeer => mk(NodeState::Working, source, conf, Some("waiting_peer")),
        AgentState::Success if !i.seen && i.refused => {
            mk(NodeState::Done, source, conf, Some("refused"))
        }
        AgentState::Success if !i.seen => mk(NodeState::Done, source, conf, msg),
        AgentState::Failed if !i.seen => {
            mk(NodeState::Done, source, conf, Some(failure_detail(msg)))
        }
        AgentState::Success | AgentState::Failed | AgentState::Idle => {
            mk(NodeState::Ready, source, conf, None)
        }
        AgentState::Unknown => {
            if now_ms - i.started_ms < STARTING_GRACE_MS {
                mk(NodeState::Starting, Source::None, Confidence::None, None)
            } else {
                mk(NodeState::NoSignal, Source::None, Confidence::None, None)
            }
        }
    }
}

pub fn deliverable(
    sig: &AgentSignal,
    screen_changed_ms_ago: i64,
    now_ms: i64,
    is_reply: bool,
) -> bool {
    const SETTLE_MS: i64 = 250;
    const QUIET_MS: i64 = 1_500;
    let settled = now_ms - sig.since_ms >= SETTLE_MS;
    match sig.state {
        AgentState::Idle | AgentState::Success | AgentState::Failed => {
            settled && screen_changed_ms_ago >= SETTLE_MS
        }
        AgentState::Blocked => is_reply && settled,
        AgentState::Unknown => screen_changed_ms_ago >= QUIET_MS,
        _ => false,
    }
}

pub fn turn_ended(before: AgentState, now: AgentState) -> bool {
    let busy = |s: AgentState| {
        matches!(
            s,
            AgentState::Thinking
                | AgentState::Working
                | AgentState::Compacting
                | AgentState::Looping
        )
    };
    busy(before)
        && matches!(
            now,
            AgentState::Idle | AgentState::Success | AgentState::Failed
        )
}

pub fn is_busy(s: AgentState) -> bool {
    matches!(
        s,
        AgentState::Thinking | AgentState::Working | AgentState::Compacting | AgentState::Looping
    )
}

#[cfg(test)]
mod failure_detail_tests {
    use super::*;

    fn done(state: AgentState, message: Option<&str>, refused: bool) -> Fused {
        fuse(
            &Inputs {
                signal: Some(AgentSignal {
                    state,
                    source: SignalSource::Hook,
                    since_ms: 0,
                    message: message.map(str::to_string),
                    progress: None,
                }),
                alive: true,
                refused,
                ..Default::default()
            },
            60_000,
        )
    }

    #[test]
    fn a_usage_or_billing_ceiling_is_a_limit_not_a_failure() {
        for why in [
            "rate_limit",
            "billing_error",
            "usage_limit",
            "Usage limit reached",
        ] {
            assert_eq!(
                done(AgentState::Failed, Some(why), false).detail.as_deref(),
                Some("limit"),
                "{why}"
            );
        }
    }

    #[test]
    fn any_other_error_stays_a_failure() {
        for why in [
            Some("server_error"),
            Some("exit code 1"),
            Some("invalid credentials"),
            Some("connection refused"),
            Some("ECONNREFUSED"),
            Some("quotation mark expected"),
            None,
        ] {
            assert_eq!(
                done(AgentState::Failed, why, false).detail.as_deref(),
                Some("failed")
            );
        }
    }

    #[test]
    fn a_turn_the_model_refused_is_named_so_until_seen() {
        let fused = done(AgentState::Success, Some("I can't help with that"), true);
        assert_eq!(
            (fused.state, fused.detail.as_deref()),
            (NodeState::Done, Some("refused"))
        );
        let plain = done(AgentState::Success, Some("done"), false);
        assert_eq!(plain.detail.as_deref(), Some("done"));
    }
}
