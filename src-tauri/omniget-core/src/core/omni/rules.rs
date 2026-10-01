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
