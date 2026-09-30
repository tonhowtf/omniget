//! Canonical state of a canvas node and the fusion of its signals
//! (estudo 98/03 §B.9). Sources, most trusted first: hooks (through the
//! shim), the bus (native nodes, exact), terminal sequences (OSC/title), the
//! shadow screen and silence. The terminal engine (`core::term`) already ranks
//! hook > OSC/title > heuristic inside its `AgentSignal`; this module maps
//! that signal, plus what only the canvas knows (a stuck delivery, whether the
//! user has seen a finished turn, the process being gone), onto the states
//! the UI and the delivery gate use. Pure: the caller passes `now_ms`.

use serde::{Deserialize, Serialize};

use crate::core::term::agent::{AgentSignal, AgentState, SignalSource};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeState {
    Starting,
    Ready,
    Working,
    /// Permission, question, elicitation: the human is needed.
    NeedsYou,
    /// Finished a turn the user has not looked at yet.
    Done,
    /// A delivery did not take.
    Stuck,
    Exited,
    /// No hook and nothing conclusive on screen: never a guess.
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

/// How much the state can be trusted; the UI says "estimated" below `Reported`.
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
    /// Extra detail (`waiting_peer`, `failed`, the last tool...).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub since_ms: i64,
}

/// Everything the fusion looks at for one node.
#[derive(Debug, Clone, Default)]
pub struct Inputs {
    /// Terminal nodes: the engine's signal. Native nodes: built from the bus.
    pub signal: Option<AgentSignal>,
    /// The process is running (terminal) / the runtime is up (native).
    pub alive: bool,
    /// Epoch ms the node's process started.
    pub started_ms: i64,
    /// Native node: the bus is the source (exact).
    pub from_bus: bool,
    /// A delivery to this node gave up (`delivery_stuck`).
    pub stuck: bool,
    /// The user focused or marked the node after its last finished turn.
    pub seen: bool,
    /// A permission question of this node is pending in the broker.
    pub permission_pending: bool,
}

/// Grace after spawn during which "no signal" reads as "starting".
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
        AgentState::Success if !i.seen => mk(NodeState::Done, source, conf, msg),
        AgentState::Failed if !i.seen => mk(NodeState::Done, source, conf, Some("failed")),
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

/// Is the node free to receive a paste now?
///
/// - ready (idle or a finished turn) for at least `settle_ms` (the TUI redraws
///   its input box after `Stop`);
/// - `needs_you` only for a reply to that node's own question;
/// - with no signal at all, only when the screen has been still for
///   `quiet_ms` (silence is the last resort).
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

/// A turn ended: busy before, not busy now.
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
mod tests {
    use super::*;

    fn sig(state: AgentState, source: SignalSource, since_ms: i64) -> AgentSignal {
        AgentSignal {
            state,
            source,
            since_ms,
            message: None,
            progress: None,
        }
    }

    #[test]
    fn hooks_are_exact_and_screen_is_estimated() {
        let mut i = Inputs {
            alive: true,
            signal: Some(sig(AgentState::Working, SignalSource::Hook, 0)),
            ..Default::default()
        };
        let f = fuse(&i, 100_000);
        assert_eq!(
            (f.state, f.confidence),
            (NodeState::Working, Confidence::Exact)
        );
        i.signal = Some(sig(AgentState::Idle, SignalSource::Heuristic, 0));
        let f = fuse(&i, 100_000);
        assert_eq!(
            (f.state, f.confidence),
            (NodeState::Ready, Confidence::Estimated)
        );
    }

    #[test]
    fn done_until_seen_and_exit_wins() {
        let mut i = Inputs {
            alive: true,
            signal: Some(sig(AgentState::Success, SignalSource::Hook, 0)),
            ..Default::default()
        };
        assert_eq!(fuse(&i, 1).state, NodeState::Done);
        i.seen = true;
        assert_eq!(fuse(&i, 1).state, NodeState::Ready);
        i.alive = false;
        assert_eq!(fuse(&i, 1).state, NodeState::Exited);
    }

    #[test]
    fn no_signal_is_honest_after_the_grace() {
        let i = Inputs {
            alive: true,
            signal: Some(sig(AgentState::Unknown, SignalSource::None, 0)),
            ..Default::default()
        };
        assert_eq!(fuse(&i, 1_000).state, NodeState::Starting);
        assert_eq!(fuse(&i, STARTING_GRACE_MS + 1).state, NodeState::NoSignal);
    }

    #[test]
    fn delivery_waits_for_a_settled_ready_node() {
        let s = sig(AgentState::Idle, SignalSource::Hook, 1_000);
        assert!(!deliverable(&s, 5_000, 1_100, false));
        assert!(deliverable(&s, 5_000, 1_300, false));
        let b = sig(AgentState::Blocked, SignalSource::Hook, 0);
        assert!(!deliverable(&b, 5_000, 5_000, false));
        assert!(deliverable(&b, 5_000, 5_000, true));
        let u = sig(AgentState::Unknown, SignalSource::None, 0);
        assert!(!deliverable(&u, 1_000, 5_000, false));
        assert!(deliverable(&u, 2_000, 5_000, false));
        assert!(turn_ended(AgentState::Working, AgentState::Success));
        assert!(!turn_ended(AgentState::Idle, AgentState::Success));
    }
}
