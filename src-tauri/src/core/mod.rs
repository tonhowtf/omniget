pub use omniget_core::core::clipboard;
pub use omniget_core::core::cookie_parser;
pub use omniget_core::core::dependencies;
pub use omniget_core::core::direct_downloader;
pub use omniget_core::core::ffmpeg;
pub use omniget_core::core::filename;
pub use omniget_core::core::http_client;
pub use omniget_core::core::http_fetcher;
pub use omniget_core::core::hwaccel;
pub use omniget_core::core::impersonation;
pub use omniget_core::core::paths;
pub use omniget_core::core::pot_provider;
pub use omniget_core::core::process;
pub use omniget_core::core::redirect;
pub use omniget_core::core::registry;
pub use omniget_core::core::ytdlp;

pub mod artifact_validation;
pub mod awake;
pub mod binary_versions;
pub mod cas;
pub mod channel_poller;
pub mod channels;
pub mod db;
pub mod download_journal;
pub mod download_log;
pub mod flight_recorder;
pub mod host_limiter;
pub mod media_diff;
pub mod nfo_sidecar;
pub mod path_limits;
pub mod portable;
pub mod preflight;
pub mod queue;
pub mod queue_history;
pub mod queue_wal;
pub mod recovery;
pub mod root_cause;
pub mod rpc;
pub mod rules;
pub mod silence_map;
pub mod state_file;
pub mod trackers;
pub mod url_parser;
pub mod url_resolver;

/// Guards this crate's tests that point `OMNIGET_DATA_DIR` somewhere.
/// `cargo test` runs them as threads of one process, so an unguarded `set_var`
/// moves another test's files mid-run. Tests that share one folder hold a read
/// guard; a test that sets its own holds the write guard (`blocking_write` in
/// sync tests). omniget-core has its own, test-only, so it is not visible here.
#[cfg(test)]
pub(crate) fn test_env_lock() -> &'static tokio::sync::RwLock<()> {
    static LOCK: tokio::sync::RwLock<()> = tokio::sync::RwLock::const_new(());
    &LOCK
}
