use serde_json::Value;

use super::catalog_dir;
use super::model::{CatalogItem, MarketplaceInfo};

pub fn cached_listings() -> Vec<(MarketplaceInfo, Vec<CatalogItem>)> {
    let Ok(dir) = catalog_dir().map(|d| d.join("marketplaces")) else {
        return Vec::new();
    };
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let Ok(bytes) = std::fs::read(e.path()) else {
            continue;
        };
        let Ok(v) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        let info = serde_json::from_value::<MarketplaceInfo>(v["info"].clone()).ok();
        let items = serde_json::from_value::<Vec<CatalogItem>>(v["items"].clone()).ok();
        if let (Some(i), Some(it)) = (info, items) {
            out.push((i, it));
        }
    }
    out.sort_by(|a, b| a.0.repo.cmp(&b.0.repo));
    out
}
