pub mod awake;
pub mod clipseq;
pub mod history;
pub mod monitor;
pub mod snippets;
pub mod volume;

use serde::Serialize;
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum QuickError {
    #[error("{0}")]
    InvalidInput(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Unavailable(String),
    #[error("cannot save the quick tools file: {0}")]
    Store(String),
    #[error("{0}")]
    Os(String),
}

impl QuickError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidInput(_) => "INVALID_INPUT",
            Self::NotFound(_) => "NOT_FOUND",
            Self::Unavailable(_) => "UNAVAILABLE",
            Self::Store(_) => "STORE_FAILED",
            Self::Os(_) => "OS_ERROR",
        }
    }

    pub fn retryable(&self) -> bool {
        matches!(self, Self::Os(_))
    }

    pub fn next_step(&self) -> &'static str {
        match self {
            Self::InvalidInput(_) => "fix the argument named in the message and call again",
            Self::NotFound(_) => {
                "list again (snippets_list or volume_list) and use an id from the answer"
            }
            Self::Unavailable(_) => {
                "this computer has no such control; quick_tools op=status lists what is available"
            }
            Self::Store(_) => "check that the app data folder is writable",
            Self::Os(_) => "list again and retry once; the app or device may have gone away",
        }
    }

    pub fn wire(&self) -> String {
        format!(
            "{}: {}",
            self.code(),
            json!({
                "message": self.to_string(),
                "retryable": self.retryable(),
                "next_step": self.next_step(),
            })
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Capabilities {
    pub clipboard_history: bool,
    pub volume: volume::Kind,
}

pub fn capabilities() -> Capabilities {
    Capabilities {
        clipboard_history: true,
        volume: volume::kind(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_error_on_the_wire_carries_code_retryable_and_next_step() {
        let wire = QuickError::NotFound("no snippet 9".into()).wire();
        let (code, detail) = wire.split_once(": ").unwrap();
        assert_eq!(code, "NOT_FOUND");
        let detail: serde_json::Value = serde_json::from_str(detail).unwrap();
        assert_eq!(detail["message"], "no snippet 9");
        assert_eq!(detail["retryable"], false);
        assert!(detail["next_step"].is_string());
        assert!(QuickError::Os("gone".into()).retryable());
    }
}
