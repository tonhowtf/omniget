//! The simulation: the tick, the routines, the mailbox and the sleep states.

pub mod mailbox;
pub mod routine;
pub mod sleep;
pub mod task_tick;
pub mod tasks;
pub mod tick;

pub use mailbox::{Decision, Mailbox, MAILBOX_DEPTH, MAX_SAY_BYTES};
pub use routine::{
    day_of, hm, minute_of_day, Routine, RoutineEntry, MINUTES_PER_DAY, TICKS_PER_DAY,
    TICKS_PER_GAME_MINUTE,
};
pub use sleep::{ticks_from_ms, SleepState, MAX_CATCH_UP_TICKS, TICK_MS};
pub use tasks::{
    affordance, EffectRequest, Reason, Reservation, Task, TaskAction, TaskBook, TaskId, TaskParams,
    TaskSource, TaskState, TaskStats, TraceEntry,
};
pub use tick::{SAY_TICKS, WAVE_TICKS, YAWN_ODDS, YAWN_TICKS};
