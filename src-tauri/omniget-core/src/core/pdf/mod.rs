pub mod compress;
pub mod files;
pub mod images;
pub mod pages;

use std::sync::atomic::{AtomicBool, Ordering};

use pdfcraft_cos::CosError;
use pdfcraft_organize::OrganizeError;
use serde::Serialize;

pub use compress::{compress_bytes, Preset};
pub use files::{inspect, save, unique_path, PdfInfo};
pub use images::{expand_images, images_to_pdf, is_image, ImageLayout};
pub use pages::{write_outputs, PageSpec};

#[derive(Debug, thiserror::Error)]
pub enum PdfError {
    #[error("this file is not a PDF")]
    NotPdf,
    #[error("this PDF is protected by a password")]
    Password,
    #[error("this PDF is damaged: {0}")]
    Damaged(String),
    #[error("this PDF has no pages")]
    Empty,
    #[error("page {0} does not exist")]
    NoSuchPage(usize),
    #[error("invalid request: {0}")]
    Invalid(&'static str),
    #[error("could not read the image {0}")]
    Image(String),
    #[error("cancelled")]
    Cancelled,
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

impl PdfError {
    pub fn code(&self) -> &'static str {
        match self {
            PdfError::NotPdf => "not_pdf",
            PdfError::Password => "password",
            PdfError::Damaged(_) => "damaged",
            PdfError::Empty => "empty",
            PdfError::NoSuchPage(_) | PdfError::Invalid(_) => "invalid",
            PdfError::Image(_) => "image",
            PdfError::Cancelled => "cancelled",
            PdfError::Io(_) => "io",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PdfFailure {
    pub code: &'static str,
    pub message: String,
}

impl From<PdfError> for PdfFailure {
    fn from(e: PdfError) -> Self {
        Self {
            code: e.code(),
            message: e.to_string(),
        }
    }
}

impl From<CosError> for PdfError {
    fn from(e: CosError) -> Self {
        match e {
            CosError::NotPdf => PdfError::NotPdf,
            CosError::NeedsPassword | CosError::WrongPassword => PdfError::Password,
            other => PdfError::Damaged(other.to_string()),
        }
    }
}

impl From<OrganizeError> for PdfError {
    fn from(e: OrganizeError) -> Self {
        match e {
            OrganizeError::NoSuchPage(n) => PdfError::NoSuchPage(n),
            other => PdfError::Damaged(other.to_string()),
        }
    }
}

pub struct Control<'a> {
    pub cancel: &'a AtomicBool,
    pub progress: &'a (dyn Fn(usize, usize) + Sync),
}

impl Control<'_> {
    pub fn check(&self) -> Result<(), PdfError> {
        if self.cancel.load(Ordering::Relaxed) {
            Err(PdfError::Cancelled)
        } else {
            Ok(())
        }
    }
}
