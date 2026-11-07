pub mod api;
pub mod cache;
pub mod library;
pub mod service;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub use api::API_BASE;
pub use cache::Cache;
pub use library::{Library, NamedCollection, SavedIcon};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct IconSet {
    pub prefix: String,
    pub name: String,
    pub total: u32,
    pub author: Option<String>,
    pub author_url: Option<String>,
    pub license: Option<String>,
    pub license_spdx: Option<String>,
    pub license_url: Option<String>,
    pub category: Option<String>,
    pub palette: bool,
    pub height: Option<u32>,
    pub samples: Vec<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IconData {
    pub name: String,
    pub body: String,
    pub width: f64,
    pub height: f64,
    #[serde(default)]
    pub left: f64,
    #[serde(default)]
    pub top: f64,
    #[serde(default)]
    pub rotate: u8,
    #[serde(default)]
    pub h_flip: bool,
    #[serde(default)]
    pub v_flip: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CollectionsResult {
    pub sets: Vec<IconSet>,
    pub offline: bool,
    pub fetched_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    pub icons: Vec<String>,
    pub capped: bool,
    pub more: Vec<String>,
    pub next_start: u32,
    pub trimmed: usize,
    pub offline: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct IconsResult {
    pub prefix: String,
    pub icons: Vec<IconData>,
    pub missing: Vec<String>,
    pub offline: bool,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    Offline,
    Invalid,
    Http,
    Io,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct IconsError {
    pub kind: ErrorKind,
    pub code: &'static str,
    pub params: BTreeMap<&'static str, String>,
    pub message: String,
}

impl IconsError {
    pub fn new(kind: ErrorKind, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind,
            code,
            params: BTreeMap::new(),
            message: message.into(),
        }
    }

    pub fn offline(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Offline, code, message)
    }

    pub fn invalid(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Invalid, code, message)
    }

    pub fn http(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Http, code, message)
    }

    pub fn io(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Io, "io", message)
    }

    pub fn with(mut self, key: &'static str, value: impl ToString) -> Self {
        self.params.insert(key, value.to_string());
        self
    }

    pub fn is(&self, kind: ErrorKind) -> bool {
        self.kind == kind
    }
}

impl std::fmt::Display for IconsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for IconsError {}

impl From<std::io::Error> for IconsError {
    fn from(e: std::io::Error) -> Self {
        IconsError::io(e.to_string())
    }
}
