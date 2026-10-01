use super::bus::BusEvent;
use super::intent::{Animation, Intent, Mood};
use super::state::OmniState;

pub const IDLE_SIT_SECONDS: u32 = 60;
pub const IDLE_SLEEP_SECONDS: u32 = 300;

pub fn rules(event: &BusEvent, state: &OmniState) -> Option<Intent> {
    let candidate = match event {
        BusEvent::TurnStarted { .. } => Intent::new(Animation::Work, Mood::Focused),
        BusEvent::TokenDelta { .. } => Intent::new(Animation::Talk, Mood::Focused),
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
