//! Terminais: PTY, emulação (alacritty_terminal), wire OGT1, estado de agente.

pub mod engine;
pub mod session;
pub mod osc;
pub mod agent;
pub mod input;
pub mod wire;
pub mod screen;

pub use agent::{AgentKind, AgentSignal, AgentState, SignalSource};
pub use screen::ScreenSnapshot;
pub use session::{global, install_global, FrameSink, TermEvents, TermInfo, TermManager, TermOpen};
