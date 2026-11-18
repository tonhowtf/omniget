use serde::Serialize;

use super::model::CatalogItem;

#[derive(Debug, Clone, Default, Serialize)]
pub struct ScanReport {
    pub items: Vec<CatalogItem>,
    pub excluded: Vec<(String, String)>,
    pub skipped: Vec<(String, String)>,
}
