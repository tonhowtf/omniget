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
