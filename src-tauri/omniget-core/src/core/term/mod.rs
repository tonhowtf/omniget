//! Terminais: PTY, emulação (alacritty_terminal), wire OGT1, estado de agente.

pub mod agent;
pub mod engine;
pub mod input;
pub mod osc;
pub mod screen;
pub mod session;
pub mod wire;

pub use agent::{AgentKind, AgentSignal, AgentState, SignalSource};
pub use screen::ScreenSnapshot;
pub use session::{global, install_global, FrameSink, TermEvents, TermInfo, TermManager, TermOpen};
