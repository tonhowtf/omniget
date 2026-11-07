pub mod catalog;
pub mod install;
pub mod scan;

pub use catalog::{classify_license, CatalogFont, FontDetails, FontFile, LicenseClass};
pub use install::{FailedFile, FileState, InstallOutcome, PlannedFile};
pub use scan::{Axis, FaceInfo, InstalledFamily, InstalledStyle};
