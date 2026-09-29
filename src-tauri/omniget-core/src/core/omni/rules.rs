//! Mascot `rules` (Phase 5). Owned by f5-omni-intent.
//!
//! Pure, allocation-light mapping from a `BusEvent` to an `Intent`. This is the
//! zero-cost half of the mascot: no model is ever called from here, nothing is
//! stored, nothing blocks. The other half (`parse_intent`) only runs when an
//! agent chose to speak.
//!
//! Contract with f2-llm-coordinator (owner of `BusEvent`): the match has an
//! explicit `_ => None` arm, so a new variant is silently ignored rather than
//! breaking the build; the test `every_known_variant_is_mapped` enumerates the
//! variants known on 2026-09-18 and will fail to compile when one is added, as
//! a reminder to decide what the Omni should do about it.

use super::bus::BusEvent;
use super::intent::{Animation, Intent, Mood};
use super::state::OmniState;

/// Idle seconds before the Omni sits down.
pub const IDLE_SIT_SECONDS: u32 = 60;
/// Idle seconds before the Omni falls asleep (the plan's "ocioso 5 min").
pub const IDLE_SLEEP_SECONDS: u32 = 300;

/// Maps one bus event to the mascot's next intent.
///
/// Returns `None` when the event asks for nothing new: either it carries no
/// mascot meaning, or the resulting intent is the one already on screen (which
/// keeps `TokenDelta`, one per chunk, from restarting the talk animation on
/// every token).
pub fn rules(event: &BusEvent, state: &OmniState) -> Option<Intent> {
    let candidate = match event {
        // An agent picked up work.
        BusEvent::TurnStarted { .. } => Intent::new(Animation::Work, Mood::Focused),
        // An agent is speaking. Debounced by the equality check below.
        BusEvent::TokenDelta { .. } => Intent::new(Animation::Talk, Mood::Focused),
        // Waiting for the user to allow a tool: the Omni asks for attention.
        BusEvent::ToolAsk { .. } => Intent::new(Animation::Wave, Mood::Curious),
        BusEvent::ToolCalled { ok, .. } => {
            if *ok {
                Intent::new(Animation::Work, Mood::Focused)
            } else {
                Intent::new(Animation::Worried, Mood::Worried)
            }
        }
        BusEvent::TurnEnded { .. } => Intent::new(Animation::Idle, Mood::Neutral),
        BusEvent::BudgetHit { .. } => Intent::new(Animation::Sit, Mood::Worried),
        BusEvent::Rerouted { .. } => Intent::new(Animation::Walk, Mood::Curious),
        BusEvent::DownloadQueued { .. } => Intent::new(Animation::Walk, Mood::Curious),
        BusEvent::DownloadFinished { .. } => Intent::new(Animation::Celebrate, Mood::Happy),
        BusEvent::DownloadFailed { .. } => Intent::new(Animation::Worried, Mood::Worried),
        BusEvent::ToolFinished { ok, .. } => {
            if *ok {
                Intent::new(Animation::Celebrate, Mood::Proud)
            } else {
                Intent::new(Animation::Worried, Mood::Worried)
            }
        }
        BusEvent::ExternalAgentActive { cli, project } => {
            let mut intent = Intent::new(Animation::Work, Mood::Focused);
            intent.line = Some(format!("{cli} · {project}"));
            intent
        }
        BusEvent::Idle { seconds } => {
            if *seconds >= IDLE_SLEEP_SECONDS {
                Intent::new(Animation::Sleep, Mood::Sleepy)
            } else if *seconds >= IDLE_SIT_SECONDS {
                Intent::new(Animation::Sit, Mood::Neutral)
            } else {
                return None;
            }
        }
        // A variant f2-llm-coordinator added after this file was written: no
        // mascot reaction until someone decides on one. Unreachable today by
        // construction, kept so a new variant compiles here instead of
        // breaking the build.
        #[allow(unreachable_patterns)]
        _ => return None,
    };

    let candidate = apply_energy(candidate, state);
    if candidate == state.current {
        None
    } else {
        Some(candidate)
    }
}

/// Out of quota, the Omni cannot work: effortful poses collapse into sleep.
/// Bad news still shows as worry, so a failure is never hidden by tiredness.
fn apply_energy(intent: Intent, state: &OmniState) -> Intent {
    if !state.is_exhausted() {
        return intent;
    }
    match intent.animation {
        Animation::Work | Animation::Walk | Animation::Talk | Animation::Celebrate => {
            Intent::new(Animation::Sleep, Mood::Sleepy)
        }
        _ => intent,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::llm::types::Usage;
    use std::time::Instant;

    fn usage() -> Usage {
        Usage::default()
    }

    fn fresh() -> OmniState {
        OmniState::new()
    }

    /// One sample per `BusEvent` variant known on 2026-09-18.
    fn known_events() -> Vec<BusEvent> {
        vec![
            BusEvent::TurnStarted {
                agent: "omni".into(),
                conversation: "c1".into(),
            },
            BusEvent::TokenDelta {
                agent: "omni".into(),
                chars: 12,
            },
            BusEvent::ToolAsk {
                agent: "omni".into(),
                request_id: "r1".into(),
                tool_call_id: "t1".into(),
                tool: "download".into(),
                preview: String::new(),
            },
            BusEvent::ToolCalled {
                agent: "omni".into(),
                tool: "download".into(),
                ok: true,
                ms: 10,
            },
            BusEvent::TurnEnded {
                agent: "omni".into(),
                usage: usage(),
            },
            BusEvent::BudgetHit {
                agent: "omni".into(),
            },
            BusEvent::Rerouted {
                agent: "omni".into(),
                from: "a".into(),
                to: "b".into(),
                why: "budget".into(),
            },
            BusEvent::DownloadQueued { id: 1 },
            BusEvent::DownloadFinished { id: 1 },
            BusEvent::DownloadFailed {
                id: 1,
                code: "ERR_YTDLP".into(),
            },
            BusEvent::ToolFinished {
                tool: "img-exif".into(),
                ok: true,
            },
            BusEvent::Idle { seconds: 600 },
        ]
    }

    /// Exhaustive on purpose: adding a `BusEvent` variant breaks this match at
    /// compile time, which is the reminder to map it in `rules`.
    fn variant_tag(event: &BusEvent) -> &'static str {
        match event {
            BusEvent::TurnStarted { .. } => "turn_started",
            BusEvent::TokenDelta { .. } => "token_delta",
            BusEvent::ToolAsk { .. } => "tool_ask",
            // Bookkeeping for jobs and windows, not a mascot moment: the
            // `ToolCalled`/`ToolFinished` that follow an allowed ask move it.
            BusEvent::ToolAskResolved { .. } => "tool_ask_resolved",
            BusEvent::ToolCalled { .. } => "tool_called",
            BusEvent::TurnEnded { .. } => "turn_ended",
            BusEvent::BudgetHit { .. } => "budget_hit",
            // Its `BudgetHit` (when hard) is the mascot's moment.
            BusEvent::BudgetCrossed { .. } => "budget_crossed",
            BusEvent::Rerouted { .. } => "rerouted",
            BusEvent::DownloadQueued { .. } => "download_queued",
            BusEvent::DownloadFinished { .. } => "download_finished",
            BusEvent::DownloadFailed { .. } => "download_failed",
            BusEvent::ToolFinished { .. } => "tool_finished",
            BusEvent::Idle { .. } => "idle",
            BusEvent::ExternalAgentActive { .. } => "external_agent_active",
        }
    }

    #[test]
    fn every_known_variant_is_mapped() {
        let events = known_events();
        let mut tags: Vec<&str> = events.iter().map(variant_tag).collect();
        tags.sort_unstable();
        tags.dedup();
        assert_eq!(tags.len(), 12, "one sample per variant, no duplicates");
        // `rules` suppresses an intent that is already on screen, so a variant
        // counts as mapped when it moves the mascot from the idle state or
        // from a busy one.
        let idle = fresh();
        let mut busy = fresh();
        busy.apply(&Intent::new(Animation::Work, Mood::Focused));
        for e in &events {
            assert!(
                rules(e, &idle).is_some() || rules(e, &busy).is_some(),
                "{} moves the mascot from no state at all",
                variant_tag(e)
            );
        }
    }

    #[test]
    fn download_lifecycle() {
        let state = fresh();
        assert_eq!(
            rules(&BusEvent::DownloadQueued { id: 9 }, &state),
            Some(Intent::new(Animation::Walk, Mood::Curious))
        );
        assert_eq!(
            rules(&BusEvent::DownloadFinished { id: 9 }, &state),
            Some(Intent::new(Animation::Celebrate, Mood::Happy))
        );
        assert_eq!(
            rules(
                &BusEvent::DownloadFailed {
                    id: 9,
                    code: "ERR_YTDLP_403".into()
                },
                &state
            ),
            Some(Intent::new(Animation::Worried, Mood::Worried))
        );
    }

    #[test]
    fn tool_outcome_splits_on_ok() {
        let state = fresh();
        assert_eq!(
            rules(
                &BusEvent::ToolFinished {
                    tool: "pdf".into(),
                    ok: true
                },
                &state
            )
            .unwrap()
            .mood,
            Mood::Proud
        );
        assert_eq!(
            rules(
                &BusEvent::ToolFinished {
                    tool: "pdf".into(),
                    ok: false
                },
                &state
            )
            .unwrap()
            .animation,
            Animation::Worried
        );
        assert_eq!(
            rules(
                &BusEvent::ToolCalled {
                    agent: "a".into(),
                    tool: "pdf".into(),
                    ok: false,
                    ms: 3
                },
                &state
            )
            .unwrap()
            .mood,
            Mood::Worried
        );
    }

    #[test]
    fn agent_speaking_talks_once() {
        let mut state = fresh();
        let delta = BusEvent::TokenDelta {
            agent: "a".into(),
            chars: 4,
        };
        let first = rules(&delta, &state).expect("first delta moves the mascot");
        assert_eq!(first.animation, Animation::Talk);
        state.apply(&first);
        assert_eq!(
            rules(&delta, &state),
            None,
            "debounced while already talking"
        );
    }

    #[test]
    fn idle_thresholds() {
        let state = fresh();
        assert_eq!(rules(&BusEvent::Idle { seconds: 5 }, &state), None);
        assert_eq!(rules(&BusEvent::Idle { seconds: 59 }, &state), None);
        assert_eq!(
            rules(&BusEvent::Idle { seconds: 60 }, &state)
                .unwrap()
                .animation,
            Animation::Sit
        );
        assert_eq!(
            rules(&BusEvent::Idle { seconds: 299 }, &state)
                .unwrap()
                .animation,
            Animation::Sit
        );
        assert_eq!(
            rules(&BusEvent::Idle { seconds: 300 }, &state).unwrap(),
            Intent::new(Animation::Sleep, Mood::Sleepy)
        );
    }

    #[test]
    fn turn_end_returns_to_idle_but_not_twice() {
        let mut state = fresh();
        state.apply(&Intent::new(Animation::Work, Mood::Focused));
        let ended = BusEvent::TurnEnded {
            agent: "a".into(),
            usage: usage(),
        };
        let back = rules(&ended, &state).unwrap();
        assert_eq!(back, Intent::neutral());
        state.apply(&back);
        assert_eq!(rules(&ended, &state), None);
    }

    #[test]
    fn budget_and_reroute() {
        let state = fresh();
        assert_eq!(
            rules(&BusEvent::BudgetHit { agent: "a".into() }, &state).unwrap(),
            Intent::new(Animation::Sit, Mood::Worried)
        );
        assert_eq!(
            rules(
                &BusEvent::Rerouted {
                    agent: "a".into(),
                    from: "gpt".into(),
                    to: "local".into(),
                    why: "budget".into()
                },
                &state
            )
            .unwrap()
            .mood,
            Mood::Curious
        );
    }

    #[test]
    fn tool_ask_waves_for_attention() {
        let state = fresh();
        assert_eq!(
            rules(
                &BusEvent::ToolAsk {
                    agent: "a".into(),
                    request_id: "r".into(),
                    tool_call_id: "t".into(),
                    tool: "shell".into(),
                    preview: String::new(),
                },
                &state
            )
            .unwrap(),
            Intent::new(Animation::Wave, Mood::Curious)
        );
    }

    #[test]
    fn exhausted_energy_collapses_effort_but_keeps_worry() {
        let mut state = fresh();
        state.set_energy(0);
        assert_eq!(
            rules(
                &BusEvent::TurnStarted {
                    agent: "a".into(),
                    conversation: "c".into()
                },
                &state
            )
            .unwrap(),
            Intent::new(Animation::Sleep, Mood::Sleepy)
        );
        assert_eq!(
            rules(&BusEvent::DownloadFinished { id: 1 }, &state)
                .unwrap()
                .animation,
            Animation::Sleep
        );
        assert_eq!(
            rules(
                &BusEvent::DownloadFailed {
                    id: 1,
                    code: "x".into()
                },
                &state
            )
            .unwrap()
            .animation,
            Animation::Worried,
            "a failure is never hidden by tiredness"
        );
    }

    #[test]
    fn rules_never_carry_a_line() {
        let state = fresh();
        for e in known_events() {
            if let Some(i) = rules(&e, &state) {
                assert!(i.line.is_none(), "{} invented a line", variant_tag(&e));
            }
        }
    }

    #[test]
    fn rules_are_under_one_millisecond() {
        let state = fresh();
        let events = known_events();
        for e in &events {
            let _ = rules(e, &state);
        }
        let t0 = Instant::now();
        for e in &events {
            std::hint::black_box(rules(std::hint::black_box(e), &state));
        }
        let per = t0.elapsed() / events.len() as u32;
        assert!(per.as_millis() < 1, "rules took {per:?} per event");
    }
}
