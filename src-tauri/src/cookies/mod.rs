//! Cookie Manager — multi-platform per-domain cookie storage.
//!
//! Replaces the legacy single-file `chrome-extension-cookies.txt` (see
//! `extension_storage`), which is only read now, to migrate old installs.
//!
//! Module layout:
//! * `platform` — domain → `PlatformKind` mapping (drives UI logo + copy)
//! * `parsers`  — Netscape and JSON cookie import
//! * `storage`  — on-disk layout, `_meta.json` registry, transactional writes
//! * `commands` — Tauri commands consumed by Settings → Cookies UI

pub mod commands;
pub mod parsers;
pub mod platform;
pub mod storage;

pub use platform::{root_domain_of, PlatformKind};
pub use storage::{
    account_path_for_consumer, ingest_batch, ingest_to_account, load_registry,
    migrate_legacy_if_needed, touch_last_used, AccountEntry, BucketEntry, CookieRegistry,
    IngestSource,
};
