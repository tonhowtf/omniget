//! Reading document metadata.

use crate::object::DateTime;
use alloc::vec::Vec;

#[derive(Clone, Default, Debug, PartialEq, Eq)]
/// The metadata of a PDF document.
pub struct Metadata {
    /// The creation date of the document.
    pub creation_date: Option<DateTime>,
    /// The modification date of the document.
    pub modification_date: Option<DateTime>,
    /// The title of the document.
    ///
    /// In the vast majority of cases, this is going to be an ASCII string, but it doesn't have to
    /// be.
    pub title: Option<Vec<u8>>,
    /// The author of the document.
    ///
    /// In the vast majority of cases, this is going to be an ASCII string, but it doesn't have to
    /// be.
    pub author: Option<Vec<u8>>,
    /// The subject of the document.
    ///
    /// In the vast majority of cases, this is going to be an ASCII string, but it doesn't have to
    /// be.
    pub subject: Option<Vec<u8>>,
    /// The keywords of the document.
    ///
    /// In the vast majority of cases, this is going to be an ASCII string, but it doesn't have to
    /// be.
    pub keywords: Option<Vec<u8>>,
    /// The creator of the document.
    ///
    /// In the vast majority of cases, this is going to be an ASCII string, but it doesn't have to
    /// be.
    pub creator: Option<Vec<u8>>,
    /// The producer of the document.
    ///
    /// In the vast majority of cases, this is going to be an ASCII string, but it doesn't have to
    /// be.
    pub producer: Option<Vec<u8>>,
}
