pub mod http;
pub mod pack;
pub mod ports;
pub mod scrape;
pub mod util;

use serde_json::json;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum DevError {
    #[error("{0}")]
    InvalidInput(String),
    #[error("the input is larger than {limit} bytes")]
    InputTooLarge { limit: usize },
    #[error("no match for {what}")]
    NoMatch {
        what: String,
        candidates: Vec<String>,
    },
    #[error("the site blocked the request: {reason} (HTTP {status})")]
    Blocked {
        reason: &'static str,
        status: u16,
        retry_after: Option<u64>,
    },
    #[error("the server answered HTTP {0}")]
    HttpStatus(u16),
    #[error("{message}")]
    Network { message: String, timeout: bool },
    #[error("{0} already exists and is never overwritten")]
    OutputExists(PathBuf),
    #[error("cannot write {path}: {message}")]
    Write { path: PathBuf, message: String },
    #[error("cannot read {path}: {message}")]
    Read { path: PathBuf, message: String },
    #[error("no file is left to pack after the filters")]
    NoFiles,
    #[error("the folder has more than {limit} files")]
    TooManyFiles {
        limit: usize,
        folders: Vec<(String, usize)>,
    },
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Refused(String),
    #[error("{0}")]
    Unavailable(String),
}

impl DevError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidInput(_) => "INVALID_INPUT",
            Self::InputTooLarge { .. } => "INPUT_TOO_LARGE",
            Self::NoMatch { .. } => "NO_MATCH",
            Self::Blocked { .. } => "BLOCKED",
            Self::HttpStatus(_) => "HTTP_STATUS",
            Self::Network { .. } => "NETWORK_ERROR",
            Self::OutputExists(_) => "OUTPUT_EXISTS",
            Self::Write { .. } => "WRITE_FAILED",
            Self::Read { .. } => "READ_FAILED",
            Self::NoFiles => "NO_FILES",
            Self::TooManyFiles { .. } => "TOO_MANY_FILES",
            Self::NotFound(_) => "NOT_FOUND",
            Self::Refused(_) => "REFUSED",
            Self::Unavailable(_) => "UNAVAILABLE",
        }
    }

    pub fn retryable(&self) -> bool {
        match self {
            Self::Blocked { reason, .. } => *reason == "rate_limited",
            Self::HttpStatus(status) => *status >= 500,
            Self::Network { .. } => true,
            _ => false,
        }
    }

    pub fn next_step(&self) -> &'static str {
        match self {
            Self::InvalidInput(_) => "fix the argument named in the message and call again",
            Self::InputTooLarge { .. } => "send a smaller input, or a file path where the op accepts one",
            Self::NoMatch { candidates, .. } if candidates.is_empty() => {
                "nothing similar exists in the input; read it first (dev_scrape fetch or json_format) and choose again"
            }
            Self::NoMatch { .. } => "retry with one of the candidates",
            Self::Blocked { .. } => {
                "plain HTTP was the only level tried; open the page with gd_browser_open and read it with gd_snapshot, which use a real browser session"
            }
            Self::HttpStatus(_) => "check the URL; use dev_http to see the full response",
            Self::Network { timeout: true, .. } => "retry with a larger timeoutMs",
            Self::Network { .. } => "check the host name and the connection, then retry",
            Self::OutputExists(_) => {
                "pass a path that does not exist yet, or omit it to get a new file; nothing was written"
            }
            Self::Write { .. } => "choose a folder you can write to",
            Self::Read { .. } => "check that the path exists and is readable",
            Self::NoFiles => "widen include, drop exclude, or point path at a folder with text files",
            Self::TooManyFiles { .. } => {
                "point path at one of the folders listed, or narrow it with include or exclude"
            }
            Self::NotFound(_) => "call dev_ports with op=list to see what is listening; nothing was killed",
            Self::Refused(_) => "choose another target; nothing was killed",
            Self::Unavailable(_) => "this operation is not available on this machine",
        }
    }

    pub fn wire(&self) -> String {
        let mut detail = json!({
            "message": self.to_string(),
            "retryable": self.retryable(),
            "next_step": self.next_step(),
        });
        if let Self::NoMatch { candidates, .. } = self {
            detail["candidates"] = json!(candidates);
        }
        if let Self::TooManyFiles { folders, .. } = self {
            detail["folders"] = folders
                .iter()
                .map(|(path, files)| json!({ "path": path, "files": files }))
                .collect();
        }
        if let Self::Blocked {
            retry_after: Some(seconds),
            ..
        } = self
        {
            detail["retry_after_s"] = json!(seconds);
        }
        format!("{}: {detail}", self.code())
    }
}

pub fn estimate_tokens(text: &str) -> usize {
    text.chars().count().div_ceil(4)
}

pub fn cap_chars(text: &str, max: usize) -> (String, bool, usize) {
    let total = text.chars().count();
    if total <= max {
        return (text.to_string(), false, total);
    }
    (text.chars().take(max).collect(), true, total)
}

pub fn output_file(prefix: &str, extension: &str) -> PathBuf {
    std::env::temp_dir().join("omniget-devtools").join(format!(
        "{prefix}-{}.{extension}",
        uuid::Uuid::new_v4().simple()
    ))
}

pub fn write_new(path: &Path, bytes: &[u8]) -> Result<(), DevError> {
    use std::io::Write;
    let failed = |e: std::io::Error| DevError::Write {
        path: path.to_path_buf(),
        message: e.to_string(),
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(failed)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                DevError::OutputExists(path.to_path_buf())
            } else {
                failed(e)
            }
        })?;
    file.write_all(bytes).map_err(failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_text_starts_with_the_code_and_carries_the_recovery_fields() {
        let error = DevError::NoMatch {
            what: "selector .price".into(),
            candidates: vec![".product-price".into()],
        };
        let text = error.wire();
        let (code, detail) = text.split_once(": ").unwrap();
        assert_eq!(code, "NO_MATCH");
        let detail: serde_json::Value = serde_json::from_str(detail).unwrap();
        assert_eq!(detail["message"], "no match for selector .price");
        assert_eq!(detail["retryable"], false);
        assert_eq!(detail["next_step"], "retry with one of the candidates");
        assert_eq!(detail["candidates"][0], ".product-price");
    }

    #[test]
    fn a_rate_limit_carries_the_wait_in_seconds() {
        let text = DevError::Blocked {
            reason: "rate_limited",
            status: 429,
            retry_after: Some(30),
        }
        .wire();
        let detail: serde_json::Value =
            serde_json::from_str(text.split_once(": ").unwrap().1).unwrap();
        assert_eq!(detail["retry_after_s"], 30);
    }

    #[test]
    fn too_many_files_says_where_they_are() {
        let text = DevError::TooManyFiles {
            limit: 5,
            folders: vec![("node_modules".into(), 4), ("src".into(), 2)],
        }
        .wire();
        let detail: serde_json::Value =
            serde_json::from_str(text.split_once(": ").unwrap().1).unwrap();
        assert_eq!(detail["folders"][0]["path"], "node_modules");
        assert_eq!(detail["folders"][0]["files"], 4);
    }

    #[test]
    fn only_transient_failures_are_retryable() {
        assert!(DevError::HttpStatus(502).retryable());
        assert!(!DevError::HttpStatus(404).retryable());
        assert!(DevError::Blocked {
            reason: "rate_limited",
            status: 429,
            retry_after: None,
        }
        .retryable());
        assert!(!DevError::Blocked {
            reason: "http_403",
            status: 403,
            retry_after: None,
        }
        .retryable());
    }

    #[test]
    fn tokens_are_a_quarter_of_the_characters_rounded_up() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2);
        assert_eq!(estimate_tokens("ééééé"), 2);
    }

    #[test]
    fn capping_reports_the_full_length() {
        assert_eq!(cap_chars("héllo", 10), ("héllo".to_string(), false, 5));
        assert_eq!(cap_chars("héllo", 2), ("hé".to_string(), true, 5));
    }

    #[test]
    fn an_existing_file_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/out.txt");
        write_new(&path, b"one").unwrap();
        assert_eq!(
            write_new(&path, b"two"),
            Err(DevError::OutputExists(path.clone()))
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "one");
    }
}
