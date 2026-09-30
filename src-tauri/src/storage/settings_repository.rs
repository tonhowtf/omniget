//! The single owner of `settings.json`.
//!
//! One lock covers the whole transaction: read the latest bytes, compare the
//! revision, merge, validate, back up when needed, write atomically and only
//! then publish the confirmed snapshot. Effects run after the lock is
//! released (`settings_effects`). The file keeps the `app_settings` envelope
//! and every key this version does not know, and carries the revision as
//! `_settings_revision` next to it.
//!
//! The desktop process is the only writer of a profile: opening the
//! repository takes an OS file lock, and a second process on the same
//! profile gets a read-only repository whose mutations fail before touching
//! the disk. Standalone readers (`config::load_settings_standalone`) keep
//! reading the file directly.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use base64::Engine;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::models::settings::{AppSettings, FieldError};
use crate::storage::config::recover_settings;

pub const SETTINGS_FILE: &str = "settings.json";
pub const ENVELOPE_KEY: &str = "app_settings";
pub const REVISION_KEY: &str = "_settings_revision";
pub const RECOVERY_DIR: &str = "settings-recovery";
const LOCK_FILE: &str = ".settings.lock";
const TEMP_PREFIX: &str = ".settings-write-";

/// The schema 3 appearance fields that are omitted from the file while
/// unset; a merge must drop them when the typed value no longer has them.
const OPTIONAL_APPEARANCE_KEYS: [&str; 4] = ["mode", "preset", "legacy_theme", "migration_version"];

// ---------------------------------------------------------------------------
// Injectable I/O
// ---------------------------------------------------------------------------

/// A step of a write or of a backup, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Create,
    Write,
    Sync,
    Replace,
    /// After the rename: the directory entry is not durable yet.
    DirSync,
    BackupDir,
    BackupCreate,
    BackupWrite,
    BackupSync,
}

/// Checked before every file operation of the repository. The real
/// implementation never fails; tests inject a failure at any stage and the
/// repository runs its real code up to that point.
pub trait SettingsIo: Send + Sync {
    fn check(&self, _stage: Stage) -> io::Result<()> {
        Ok(())
    }
}

pub struct RealIo;
impl SettingsIo for RealIo {}

// ---------------------------------------------------------------------------
// Public states and errors
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LoadFailure {
    ReadFailed,
    InvalidDocument,
    InvalidRevision,
    BackupFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LoadStatus {
    Absent,
    Loaded,
    Recovered {
        fields: Vec<String>,
        #[serde(rename = "recoveryId")]
        recovery_id: String,
    },
    Failed {
        code: LoadFailure,
        #[serde(rename = "recoveryId")]
        recovery_id: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnavailableCode {
    ProfileReadOnly,
    LoadFailed,
    RecoveryRequired,
    BackupFailed,
    RevisionOverflow,
}

impl UnavailableCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProfileReadOnly => "profile_read_only",
            Self::LoadFailed => "load_failed",
            Self::RecoveryRequired => "recovery_required",
            Self::BackupFailed => "backup_failed",
            Self::RevisionOverflow => "revision_overflow",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersistenceCode {
    CreateFailed,
    WriteFailed,
    SyncFailed,
    ReplaceFailed,
}

impl PersistenceCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CreateFailed => "create_failed",
            Self::WriteFailed => "write_failed",
            Self::SyncFailed => "sync_failed",
            Self::ReplaceFailed => "replace_failed",
        }
    }
}

/// Why a mutation was not confirmed. Carries codes and schema paths only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepoError {
    Validation(Vec<FieldError>),
    Conflict {
        current: u64,
    },
    Unavailable(UnavailableCode),
    Persistence(PersistenceCode),
    Indeterminate {
        attempted: u64,
        observed: Option<u64>,
    },
}

impl RepoError {
    /// A stable code for the internal writers that still return strings.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Validation(_) => "invalid_request",
            Self::Conflict { .. } => "revision_conflict",
            Self::Unavailable(c) => c.as_str(),
            Self::Persistence(c) => c.as_str(),
            Self::Indeterminate { .. } => "post_replace_sync_failed",
        }
    }
}

impl std::error::Error for RepoError {}

impl std::fmt::Display for RepoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "settings: {}", self.code())
    }
}

/// The IPC form of [`RepoError`] (`06-api-contract.md`, `ApiError`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ApiError {
    Validation {
        code: &'static str,
        #[serde(rename = "fieldErrors")]
        field_errors: Vec<FieldError>,
    },
    Conflict {
        code: &'static str,
        #[serde(rename = "currentRevision")]
        current_revision: String,
        #[serde(rename = "fieldErrors")]
        field_errors: Vec<FieldError>,
    },
    Unavailable {
        code: &'static str,
        #[serde(rename = "fieldErrors")]
        field_errors: Vec<FieldError>,
    },
    Persistence {
        code: &'static str,
        committed: bool,
        #[serde(rename = "fieldErrors")]
        field_errors: Vec<FieldError>,
    },
    Indeterminate {
        code: &'static str,
        #[serde(rename = "attemptedRevision")]
        attempted_revision: String,
        #[serde(rename = "observedRevision")]
        observed_revision: Option<String>,
        #[serde(rename = "requiresReload")]
        requires_reload: bool,
        #[serde(rename = "fieldErrors")]
        field_errors: Vec<FieldError>,
    },
}

impl ApiError {
    pub fn validation(field_errors: Vec<FieldError>) -> Self {
        Self::Validation {
            code: "invalid_request",
            field_errors,
        }
    }

    pub fn unavailable(code: UnavailableCode) -> Self {
        Self::Unavailable {
            code: code.as_str(),
            field_errors: Vec::new(),
        }
    }
}

impl From<RepoError> for ApiError {
    fn from(e: RepoError) -> Self {
        match e {
            RepoError::Validation(errors) => Self::validation(errors),
            RepoError::Conflict { current } => Self::Conflict {
                code: "revision_conflict",
                current_revision: current.to_string(),
                field_errors: Vec::new(),
            },
            RepoError::Unavailable(code) => Self::unavailable(code),
            RepoError::Persistence(code) => Self::Persistence {
                code: code.as_str(),
                committed: false,
                field_errors: Vec::new(),
            },
            RepoError::Indeterminate {
                attempted,
                observed,
            } => Self::Indeterminate {
                code: "post_replace_sync_failed",
                attempted_revision: attempted.to_string(),
                observed_revision: observed.map(|r| r.to_string()),
                requires_reload: true,
                field_errors: Vec::new(),
            },
        }
    }
}

/// Parses an IPC revision: canonical decimal u64 (`0`, or digits without a
/// leading zero), nothing else.
pub fn parse_revision(s: &str) -> Option<u64> {
    let canonical = s == "0"
        || (!s.is_empty()
            && s.len() <= 20
            && s.bytes().all(|b| b.is_ascii_digit())
            && !s.starts_with('0'));
    if !canonical {
        return None;
    }
    s.parse::<u64>().ok()
}

/// A confirmed mutation.
#[derive(Debug, Clone)]
pub struct Commit {
    pub before: AppSettings,
    pub after: AppSettings,
    pub revision: u64,
    /// False when the mutation changed nothing: no write, no new revision.
    pub changed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportOutcome {
    Imported,
    AlreadyImported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupReason {
    AppearanceMigration,
    FieldRecovery,
}

impl BackupReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::AppearanceMigration => "appearance_migration",
            Self::FieldRecovery => "field_recovery",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Gate {
    Open,
    Failed {
        code: LoadFailure,
        recovery_id: Option<String>,
    },
    /// The rename happened but its durability is not confirmed: no new
    /// mutation, no "saved", no effect until a read reconciles.
    Indeterminate {
        attempted: u64,
        observed: Option<u64>,
    },
}

/// What readers see: the confirmed state.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub settings: AppSettings,
    pub revision: u64,
    pub status: LoadStatus,
    pub gate: Gate,
    pub writable: bool,
}

// ---------------------------------------------------------------------------
// Repository
// ---------------------------------------------------------------------------

struct Inner {
    /// Top-level keys of the file other than the settings themselves are
    /// kept here verbatim, together with the raw `app_settings` object.
    doc: Map<String, Value>,
    settings: AppSettings,
    revision: u64,
    status: LoadStatus,
    gate: Gate,
    /// The bytes last read or written (`None` = no file).
    disk: Option<Vec<u8>>,
}

pub struct SettingsRepository {
    dir: PathBuf,
    io: Arc<dyn SettingsIo>,
    inner: Mutex<Inner>,
    /// Held for the repository's lifetime; `None` = another process owns
    /// the profile and this repository is read-only.
    _lock: Option<File>,
}

impl SettingsRepository {
    pub fn open(dir: PathBuf) -> Self {
        Self::open_with_io(dir, Arc::new(RealIo))
    }

    pub fn open_with_io(dir: PathBuf, io: Arc<dyn SettingsIo>) -> Self {
        let lock = acquire_profile_lock(&dir);
        let repo = Self {
            dir,
            io,
            inner: Mutex::new(Inner {
                doc: Map::new(),
                settings: AppSettings::default(),
                revision: 0,
                status: LoadStatus::Absent,
                gate: Gate::Open,
                disk: None,
            }),
            _lock: lock,
        };
        {
            let mut inner = repo.guard();
            *inner = repo.load();
        }
        repo
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn path(&self) -> PathBuf {
        self.dir.join(SETTINGS_FILE)
    }

    pub fn is_writable(&self) -> bool {
        self._lock.is_some()
    }

    fn guard(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn snapshot(&self) -> Snapshot {
        let inner = self.guard();
        Snapshot {
            settings: inner.settings.clone(),
            revision: inner.revision,
            status: inner.status.clone(),
            gate: inner.gate.clone(),
            writable: self.is_writable(),
        }
    }

    /// The confirmed private settings (defaults while the load failed, so
    /// internal readers keep working; mutations stay blocked).
    pub fn settings(&self) -> AppSettings {
        self.guard().settings.clone()
    }

    pub fn revision(&self) -> u64 {
        self.guard().revision
    }

    // -- load ---------------------------------------------------------------

    fn load(&self) -> Inner {
        match fs::read(self.path()) {
            Ok(bytes) => self.from_bytes(bytes),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Inner {
                doc: Map::new(),
                settings: AppSettings::fresh_install(),
                revision: 0,
                status: LoadStatus::Absent,
                gate: Gate::Open,
                disk: None,
            },
            Err(_) => failed(LoadFailure::ReadFailed, None, None),
        }
    }

    fn from_bytes(&self, bytes: Vec<u8>) -> Inner {
        let doc = match serde_json::from_slice::<Value>(&bytes) {
            Ok(Value::Object(doc)) => doc,
            _ => return failed(LoadFailure::InvalidDocument, None, Some(bytes)),
        };
        let revision = match doc.get(REVISION_KEY) {
            None => 0,
            Some(v) => match v.as_u64() {
                Some(r) => r,
                None => return failed(LoadFailure::InvalidRevision, None, Some(bytes)),
            },
        };
        let (mut settings, status) = match doc.get(ENVELOPE_KEY) {
            None => (AppSettings::default(), LoadStatus::Loaded),
            Some(Value::Object(_)) => {
                let raw = &doc[ENVELOPE_KEY];
                match serde_json::from_value::<AppSettings>(raw.clone()) {
                    Ok(s) => (s, LoadStatus::Loaded),
                    Err(_) => {
                        let (s, fields) = recover_settings(raw);
                        // Recovery is only offered once the exact original
                        // bytes are safe in an exclusive backup.
                        match self.write_backup(
                            BackupReason::FieldRecovery,
                            Some(revision),
                            Some(&bytes),
                            None,
                        ) {
                            Ok(id) => (
                                s,
                                LoadStatus::Recovered {
                                    fields,
                                    recovery_id: id,
                                },
                            ),
                            Err(()) => return failed(LoadFailure::BackupFailed, None, Some(bytes)),
                        }
                    }
                }
            }
            Some(_) => return failed(LoadFailure::InvalidDocument, None, Some(bytes)),
        };
        settings.migrate();
        crate::models::settings::normalize_appearance(&mut settings);
        Inner {
            doc,
            settings,
            revision,
            status,
            gate: Gate::Open,
            disk: Some(bytes),
        }
    }

    /// Re-reads the file when it changed behind the repository's back (a
    /// hand edit), so a mutation always starts from the latest bytes.
    fn refresh_locked(&self, inner: &mut Inner) {
        let current = match fs::read(self.path()) {
            Ok(b) => Some(b),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(_) => {
                *inner = failed(LoadFailure::ReadFailed, None, inner.disk.clone());
                return;
            }
        };
        if current == inner.disk {
            return;
        }
        *inner = match current {
            Some(bytes) => self.from_bytes(bytes),
            None => self.load(),
        };
    }

    fn check_writable(&self, inner: &Inner) -> Result<(), RepoError> {
        if !self.is_writable() {
            return Err(RepoError::Unavailable(UnavailableCode::ProfileReadOnly));
        }
        match &inner.gate {
            Gate::Open => Ok(()),
            Gate::Failed {
                code: LoadFailure::BackupFailed,
                ..
            } => Err(RepoError::Unavailable(UnavailableCode::BackupFailed)),
            Gate::Failed { .. } => Err(RepoError::Unavailable(UnavailableCode::LoadFailed)),
            Gate::Indeterminate {
                attempted,
                observed,
            } => Err(RepoError::Indeterminate {
                attempted: *attempted,
                observed: *observed,
            }),
        }
    }

    // -- mutations ----------------------------------------------------------

    /// The one transaction every writer uses. `expected` = the revision the
    /// caller edited (`None` for internal writers, which always edit the
    /// latest). `f` receives the confirmed settings and returns the next
    /// ones; an error from `f` leaves disk and snapshot untouched.
    pub fn mutate<F>(&self, expected: Option<u64>, f: F) -> Result<Commit, RepoError>
    where
        F: FnOnce(&AppSettings) -> Result<AppSettings, RepoError>,
    {
        let mut inner = self.guard();
        if matches!(inner.gate, Gate::Open) {
            self.refresh_locked(&mut inner);
        }
        self.check_writable(&inner)?;
        if let Some(expected) = expected {
            if expected != inner.revision {
                return Err(RepoError::Conflict {
                    current: inner.revision,
                });
            }
        }
        let before = inner.settings.clone();
        let mut next = f(&before)?;
        next.migrate();
        crate::models::settings::normalize_appearance(&mut next);
        // A recovered document is written in full on the first explicit
        // edit, even when that edit equals the recovered values.
        let recovered = matches!(inner.status, LoadStatus::Recovered { .. });
        if !recovered && same(&before, &next) {
            return Ok(Commit {
                after: before.clone(),
                before,
                revision: inner.revision,
                changed: false,
            });
        }
        self.commit_locked(&mut inner, before, next)
    }

    /// The one-time appearance import: idempotent under the same lock, with
    /// an exclusive backup and the marker in the same commit.
    pub fn import_appearance(
        &self,
        raw_workspace: Option<&str>,
    ) -> Result<(ImportOutcome, Commit), RepoError> {
        let mut inner = self.guard();
        if matches!(inner.gate, Gate::Open) {
            self.refresh_locked(&mut inner);
        }
        self.check_writable(&inner)?;
        let before = inner.settings.clone();
        if before.appearance.migration_version >= 1 {
            return Ok((
                ImportOutcome::AlreadyImported,
                Commit {
                    after: before.clone(),
                    before,
                    revision: inner.revision,
                    changed: false,
                },
            ));
        }
        self.write_backup(
            BackupReason::AppearanceMigration,
            Some(inner.revision),
            inner.disk.as_deref(),
            Some(raw_workspace),
        )
        .map_err(|()| RepoError::Unavailable(UnavailableCode::BackupFailed))?;
        let next = crate::models::settings::import_appearance(&before, raw_workspace);
        let commit = self.commit_locked(&mut inner, before, next)?;
        Ok((ImportOutcome::Imported, commit))
    }

    fn commit_locked(
        &self,
        inner: &mut Inner,
        before: AppSettings,
        next: AppSettings,
    ) -> Result<Commit, RepoError> {
        let revision = inner
            .revision
            .checked_add(1)
            .ok_or(RepoError::Unavailable(UnavailableCode::RevisionOverflow))?;
        let mut doc = inner.doc.clone();
        let raw = doc
            .get(ENVELOPE_KEY)
            .cloned()
            .unwrap_or_else(|| Value::Object(Map::new()));
        let typed = serde_json::to_value(&next).map_err(|_| {
            RepoError::Validation(vec![FieldError::new(
                "",
                crate::models::settings::FieldErrorCode::InvalidValue,
            )])
        })?;
        doc.insert(ENVELOPE_KEY.into(), merge_known(&raw, typed));
        doc.insert(REVISION_KEY.into(), Value::from(revision));
        let bytes = serde_json::to_vec_pretty(&Value::Object(doc.clone()))
            .map_err(|_| RepoError::Persistence(PersistenceCode::WriteFailed))?;

        match self.write_atomic(&bytes) {
            Ok(()) => {
                inner.doc = doc;
                inner.settings = next.clone();
                inner.revision = revision;
                inner.status = LoadStatus::Loaded;
                inner.gate = Gate::Open;
                inner.disk = Some(bytes);
                Ok(Commit {
                    before,
                    after: next,
                    revision,
                    changed: true,
                })
            }
            Err(WriteFailure::Before(code)) => Err(RepoError::Persistence(code)),
            Err(WriteFailure::AfterReplace) => {
                // No rollback is possible or claimed: report what is on disk.
                let observed = fs::read(self.path())
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                    .and_then(|v| v.get(REVISION_KEY).and_then(Value::as_u64));
                inner.gate = Gate::Indeterminate {
                    attempted: revision,
                    observed,
                };
                Err(RepoError::Indeterminate {
                    attempted: revision,
                    observed,
                })
            }
        }
    }

    /// Called by reads: retries a failed load and settles an indeterminate
    /// write (directory sync again, then the bytes on disk become the
    /// confirmed state). Returns the adopted change, if any, so the caller
    /// can schedule its effects.
    pub fn reconcile(&self) -> Option<Commit> {
        let mut inner = self.guard();
        match inner.gate.clone() {
            Gate::Open => None,
            Gate::Failed { .. } => {
                let before = inner.settings.clone();
                let reloaded = self.load();
                let ok = matches!(reloaded.gate, Gate::Open);
                *inner = reloaded;
                ok.then(|| Commit {
                    before,
                    after: inner.settings.clone(),
                    revision: inner.revision,
                    changed: true,
                })
            }
            Gate::Indeterminate { .. } => {
                if self.sync_dir(Stage::DirSync).is_err() {
                    return None;
                }
                let Ok(bytes) = fs::read(self.path()) else {
                    return None;
                };
                let before = inner.settings.clone();
                let adopted = self.from_bytes(bytes);
                if !matches!(adopted.gate, Gate::Open) {
                    return None;
                }
                *inner = adopted;
                Some(Commit {
                    before,
                    after: inner.settings.clone(),
                    revision: inner.revision,
                    changed: true,
                })
            }
        }
    }

    // -- file operations ----------------------------------------------------

    fn write_atomic(&self, bytes: &[u8]) -> Result<(), WriteFailure> {
        use PersistenceCode::*;
        let before = |code: PersistenceCode| move |_: io::Error| WriteFailure::Before(code);
        self.io.check(Stage::Create).map_err(before(CreateFailed))?;
        fs::create_dir_all(&self.dir).map_err(before(CreateFailed))?;
        let mut tmp = tempfile::Builder::new()
            .prefix(TEMP_PREFIX)
            .tempfile_in(&self.dir)
            .map_err(before(CreateFailed))?;
        restrict_file(tmp.as_file()).map_err(before(CreateFailed))?;
        self.io.check(Stage::Write).map_err(before(WriteFailed))?;
        tmp.write_all(bytes).map_err(before(WriteFailed))?;
        self.io.check(Stage::Sync).map_err(before(SyncFailed))?;
        tmp.as_file().sync_all().map_err(before(SyncFailed))?;
        self.io
            .check(Stage::Replace)
            .map_err(before(ReplaceFailed))?;
        // On error the temp file comes back inside the error and is removed
        // when dropped; the destination was not touched.
        tmp.persist(self.path())
            .map_err(|_| WriteFailure::Before(ReplaceFailed))?;
        self.sync_dir(Stage::DirSync)
            .map_err(|_| WriteFailure::AfterReplace)
    }

    fn sync_dir(&self, stage: Stage) -> io::Result<()> {
        self.io.check(stage)?;
        sync_directory(&self.dir)
    }

    /// Writes `<profile>/settings-recovery/<uuid>.json` with exclusive
    /// creation. Base64 is an encoding, not encryption: the file is as
    /// sensitive as `settings.json` and stays private and local.
    fn write_backup(
        &self,
        reason: BackupReason,
        source_revision: Option<u64>,
        original: Option<&[u8]>,
        workspace: Option<Option<&str>>,
    ) -> Result<String, ()> {
        let dir = self.dir.join(RECOVERY_DIR);
        self.io.check(Stage::BackupDir).map_err(|_| ())?;
        fs::create_dir_all(&dir).map_err(|_| ())?;
        restrict_dir(&dir).map_err(|_| ())?;
        let id = uuid::Uuid::new_v4().to_string();
        let path = dir.join(format!("{id}.json"));
        let mut body = serde_json::json!({
            "version": 1,
            "reason": reason.as_str(),
            "sourceRevision": source_revision.map(|r| r.to_string()),
            "originalSettingsBase64": original
                .map(|b| base64::engine::general_purpose::STANDARD.encode(b)),
        });
        if let Some(ws) = workspace {
            body["originalWorkspace"] = ws.map(str::to_string).into();
        }
        let bytes = serde_json::to_vec_pretty(&body).map_err(|_| ())?;

        self.io.check(Stage::BackupCreate).map_err(|_| ())?;
        let mut file = create_private_new(&path).map_err(|_| ())?;
        let written = (|| {
            self.io.check(Stage::BackupWrite)?;
            file.write_all(&bytes)?;
            self.io.check(Stage::BackupSync)?;
            file.sync_all()
        })();
        if written.is_err() {
            // Only our own partial file goes; nothing else in the folder.
            drop(file);
            let _ = fs::remove_file(&path);
            return Err(());
        }
        let _ = sync_directory(&dir);
        Ok(id)
    }
}

enum WriteFailure {
    Before(PersistenceCode),
    AfterReplace,
}

fn failed(code: LoadFailure, recovery_id: Option<String>, disk: Option<Vec<u8>>) -> Inner {
    Inner {
        doc: Map::new(),
        settings: AppSettings::default(),
        revision: 0,
        status: LoadStatus::Failed {
            code,
            recovery_id: recovery_id.clone(),
        },
        gate: Gate::Failed { code, recovery_id },
        disk,
    }
}

fn same(a: &AppSettings, b: &AppSettings) -> bool {
    serde_json::to_value(a).ok() == serde_json::to_value(b).ok()
}

/// The typed settings laid over the raw object of the file: known fields
/// take the typed value, unknown keys (a newer version's fields) survive.
fn merge_known(raw: &Value, typed: Value) -> Value {
    fn merge(raw: &Value, typed: Value) -> Value {
        match (raw, typed) {
            (Value::Object(r), Value::Object(t)) => {
                let mut out = r.clone();
                for (k, v) in t {
                    let merged = match r.get(&k) {
                        Some(rv) => merge(rv, v),
                        None => v,
                    };
                    out.insert(k, merged);
                }
                Value::Object(out)
            }
            (_, t) => t,
        }
    }
    let typed_appearance = typed.get("appearance").cloned();
    let mut out = merge(raw, typed);
    if let (Some(Value::Object(a)), Some(Value::Object(ta))) =
        (out.get_mut("appearance"), typed_appearance)
    {
        for key in OPTIONAL_APPEARANCE_KEYS {
            if !ta.contains_key(key) {
                a.remove(key);
            }
        }
    }
    out
}

fn acquire_profile_lock(dir: &Path) -> Option<File> {
    fs::create_dir_all(dir).ok()?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join(LOCK_FILE))
        .ok()?;
    match file.try_lock() {
        Ok(()) => Some(file),
        Err(_) => None,
    }
}

#[cfg(unix)]
fn restrict_file(f: &File) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    f.set_permissions(fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn restrict_file(_f: &File) -> io::Result<()> {
    // Windows: the file inherits the ACL of the per-user profile folder.
    Ok(())
}

#[cfg(unix)]
fn restrict_dir(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn restrict_dir(_dir: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn create_private_new(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn create_private_new(path: &Path) -> io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

#[cfg(unix)]
fn sync_directory(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_dir: &Path) -> io::Result<()> {
    // Windows has no directory handle to flush; the rename is
    // MoveFileEx(REPLACE_EXISTING) done by tempfile's persist.
    Ok(())
}

// ---------------------------------------------------------------------------
// The process-wide instance
// ---------------------------------------------------------------------------

static GLOBAL: OnceLock<Arc<SettingsRepository>> = OnceLock::new();

/// The profile's repository, opened on first use for the app data dir
/// (which honors `OMNIGET_DATA_DIR`).
pub fn global() -> Arc<SettingsRepository> {
    GLOBAL
        .get_or_init(|| {
            let dir = crate::core::paths::app_data_dir().unwrap_or_else(|| PathBuf::from("."));
            let repo = SettingsRepository::open(dir);
            if !repo.is_writable() {
                tracing::warn!(
                    "settings: another process owns this profile; settings are read-only here"
                );
            }
            Arc::new(repo)
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::settings::FieldErrorCode;
    use serde_json::json;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Fails `stage` while `armed` is set.
    struct Faulty {
        stage: Stage,
        armed: AtomicBool,
    }

    impl Faulty {
        fn new(stage: Stage) -> Arc<Self> {
            Arc::new(Self {
                stage,
                armed: AtomicBool::new(true),
            })
        }
    }

    impl SettingsIo for Faulty {
        fn check(&self, stage: Stage) -> io::Result<()> {
            if stage == self.stage && self.armed.load(Ordering::SeqCst) {
                Err(io::Error::other("injected"))
            } else {
                Ok(())
            }
        }
    }

    fn tmp() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn write_doc(dir: &Path, doc: Value) {
        fs::write(
            dir.join(SETTINGS_FILE),
            serde_json::to_vec_pretty(&doc).unwrap(),
        )
        .unwrap();
    }

    fn stored_doc() -> Value {
        let mut s = AppSettings::default();
        s.download.video_quality = "1080p".into();
        s.bridge.token = "paired-token".into();
        let mut app = serde_json::to_value(s).unwrap();
        app["download"]["from_a_newer_version"] = json!("keep me");
        json!({ "app_settings": app, "_settings_revision": 7, "other_store_key": [1, 2] })
    }

    fn set_quality(q: &'static str) -> impl FnOnce(&AppSettings) -> Result<AppSettings, RepoError> {
        move |s| {
            let mut n = s.clone();
            n.download.video_quality = q.into();
            Ok(n)
        }
    }

    fn recovery_files(dir: &Path) -> Vec<PathBuf> {
        fs::read_dir(dir.join(RECOVERY_DIR))
            .map(|r| r.map(|e| e.unwrap().path()).collect())
            .unwrap_or_default()
    }

    // API-01
    #[test]
    fn revisions_are_canonical_decimal_u64() {
        for (s, v) in [
            ("0", 0),
            ("9", 9),
            ("10", 10),
            ("9007199254740993", 9_007_199_254_740_993),
            ("18446744073709551615", u64::MAX),
        ] {
            assert_eq!(parse_revision(s), Some(v));
            assert_eq!(v.to_string(), s);
        }
        for bad in [
            "",
            "01",
            "-1",
            "1.0",
            " 1",
            "1e3",
            "18446744073709551616",
            "abc",
            "0x10",
        ] {
            assert_eq!(parse_revision(bad), None, "{bad}");
        }
    }

    // API-01: overflow refuses the mutation without writing.
    #[test]
    fn a_revision_at_u64_max_refuses_new_mutations() {
        let dir = tmp();
        let mut doc = stored_doc();
        doc["_settings_revision"] = json!(u64::MAX);
        write_doc(dir.path(), doc);
        let before = fs::read(dir.path().join(SETTINGS_FILE)).unwrap();
        let repo = SettingsRepository::open(dir.path().into());
        assert_eq!(repo.revision(), u64::MAX);
        assert_eq!(
            repo.mutate(None, set_quality("480p")).unwrap_err(),
            RepoError::Unavailable(UnavailableCode::RevisionOverflow)
        );
        assert_eq!(fs::read(dir.path().join(SETTINGS_FILE)).unwrap(), before);
    }

    #[test]
    fn a_commit_keeps_the_envelope_and_unknown_keys_and_bumps_the_revision_once() {
        let dir = tmp();
        write_doc(dir.path(), stored_doc());
        let repo = SettingsRepository::open(dir.path().into());
        assert_eq!(repo.snapshot().status, LoadStatus::Loaded);
        let c = repo.mutate(Some(7), set_quality("480p")).unwrap();
        assert!(c.changed);
        assert_eq!(c.revision, 8);
        let disk: Value =
            serde_json::from_slice(&fs::read(dir.path().join(SETTINGS_FILE)).unwrap()).unwrap();
        assert_eq!(disk["_settings_revision"], json!(8));
        assert_eq!(disk["other_store_key"], json!([1, 2]));
        assert_eq!(disk["app_settings"]["download"]["video_quality"], "480p");
        assert_eq!(
            disk["app_settings"]["download"]["from_a_newer_version"],
            "keep me"
        );
        assert_eq!(disk["app_settings"]["bridge"]["token"], "paired-token");
        // Same values again: no write, no new revision.
        let c = repo.mutate(Some(8), set_quality("480p")).unwrap();
        assert!(!c.changed);
        assert_eq!(repo.revision(), 8);
        // No temp file is left behind.
        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with(TEMP_PREFIX))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn the_settings_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tmp();
        let repo = SettingsRepository::open(dir.path().into());
        repo.mutate(None, set_quality("480p")).unwrap();
        let mode = fs::metadata(dir.path().join(SETTINGS_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    // API-04 (backend half): one commit, one conflict, nothing lost.
    #[test]
    fn two_patches_on_the_same_revision_give_one_commit_and_one_conflict() {
        let dir = tmp();
        write_doc(dir.path(), stored_doc());
        let repo = Arc::new(SettingsRepository::open(dir.path().into()));
        let a = {
            let repo = repo.clone();
            std::thread::spawn(move || {
                repo.mutate(Some(7), |s| {
                    let mut n = s.clone();
                    n.download.video_quality = "480p".into();
                    Ok(n)
                })
            })
        };
        let b = {
            let repo = repo.clone();
            std::thread::spawn(move || {
                repo.mutate(Some(7), |s| {
                    let mut n = s.clone();
                    n.start_minimized = true;
                    Ok(n)
                })
            })
        };
        let results = [a.join().unwrap(), b.join().unwrap()];
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        assert!(results
            .iter()
            .any(|r| matches!(r, Err(RepoError::Conflict { current: 8 }))));
        // The explicit retry on the current revision keeps both intentions.
        let loser_wanted_quality = results[1].is_ok();
        let retry = if loser_wanted_quality {
            repo.mutate(Some(8), set_quality("480p"))
        } else {
            repo.mutate(Some(8), |s| {
                let mut n = s.clone();
                n.start_minimized = true;
                Ok(n)
            })
        };
        assert_eq!(retry.unwrap().revision, 9);
        let s = repo.settings();
        assert_eq!(s.download.video_quality, "480p");
        assert!(s.start_minimized);
    }

    // API-05
    #[test]
    fn a_failure_before_the_replace_keeps_bytes_and_snapshot() {
        for (stage, code) in [
            (Stage::Create, PersistenceCode::CreateFailed),
            (Stage::Write, PersistenceCode::WriteFailed),
            (Stage::Sync, PersistenceCode::SyncFailed),
            (Stage::Replace, PersistenceCode::ReplaceFailed),
        ] {
            let dir = tmp();
            write_doc(dir.path(), stored_doc());
            let original = fs::read(dir.path().join(SETTINGS_FILE)).unwrap();
            let repo = SettingsRepository::open_with_io(dir.path().into(), Faulty::new(stage));
            assert_eq!(
                repo.mutate(Some(7), set_quality("480p")).unwrap_err(),
                RepoError::Persistence(code),
                "{stage:?}"
            );
            assert_eq!(fs::read(dir.path().join(SETTINGS_FILE)).unwrap(), original);
            assert_eq!(repo.revision(), 7);
            assert_eq!(repo.settings().download.video_quality, "1080p");
            // The failed attempt left no temp file.
            let temps = fs::read_dir(dir.path())
                .unwrap()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_name().to_string_lossy().starts_with(TEMP_PREFIX))
                .count();
            assert_eq!(temps, 0, "{stage:?}");
        }
    }

    // API-05: after the rename nothing is promised; the gate stays shut
    // until a read reconciles.
    #[test]
    fn a_failure_after_the_replace_is_indeterminate_until_reconciled() {
        let dir = tmp();
        write_doc(dir.path(), stored_doc());
        let io = Faulty::new(Stage::DirSync);
        let repo = SettingsRepository::open_with_io(dir.path().into(), io.clone());
        let err = repo.mutate(Some(7), set_quality("480p")).unwrap_err();
        assert_eq!(
            err,
            RepoError::Indeterminate {
                attempted: 8,
                observed: Some(8)
            }
        );
        // Not published as saved.
        assert_eq!(repo.revision(), 7);
        assert_eq!(repo.settings().download.video_quality, "1080p");
        // No new mutation until reconciliation.
        assert!(matches!(
            repo.mutate(None, set_quality("360p")),
            Err(RepoError::Indeterminate { .. })
        ));
        // Durability still unconfirmed: reconcile refuses.
        assert!(repo.reconcile().is_none());
        io.armed.store(false, Ordering::SeqCst);
        let adopted = repo.reconcile().expect("reconciled");
        assert_eq!(adopted.revision, 8);
        assert_eq!(repo.settings().download.video_quality, "480p");
        assert_eq!(repo.snapshot().gate, Gate::Open);
        assert!(repo.mutate(Some(8), set_quality("360p")).is_ok());
    }

    // API-09
    #[test]
    fn an_absent_file_is_not_written_by_reading() {
        let dir = tmp();
        let repo = SettingsRepository::open(dir.path().into());
        let snap = repo.snapshot();
        assert_eq!(snap.status, LoadStatus::Absent);
        assert_eq!(snap.revision, 0);
        assert_eq!(
            snap.settings.appearance.preset.as_deref(),
            Some("omniget-desktop")
        );
        assert!(!dir.path().join(SETTINGS_FILE).exists());
    }

    // API-09
    #[test]
    fn an_unreadable_document_blocks_mutations_and_is_never_overwritten() {
        for content in [&b"{ not json"[..], b"[1,2]", br#"{"app_settings": 5}"#] {
            let dir = tmp();
            fs::write(dir.path().join(SETTINGS_FILE), content).unwrap();
            let repo = SettingsRepository::open(dir.path().into());
            assert!(matches!(
                repo.snapshot().status,
                LoadStatus::Failed {
                    code: LoadFailure::InvalidDocument,
                    ..
                }
            ));
            assert_eq!(
                repo.mutate(None, set_quality("480p")).unwrap_err(),
                RepoError::Unavailable(UnavailableCode::LoadFailed)
            );
            assert_eq!(fs::read(dir.path().join(SETTINGS_FILE)).unwrap(), content);
        }
        let dir = tmp();
        let mut doc = stored_doc();
        doc["_settings_revision"] = json!("7");
        write_doc(dir.path(), doc);
        let repo = SettingsRepository::open(dir.path().into());
        assert!(matches!(
            repo.snapshot().status,
            LoadStatus::Failed {
                code: LoadFailure::InvalidRevision,
                ..
            }
        ));
    }

    // API-09 + the audit contract: one bad field is recovered alone, after a
    // backup of the exact bytes, and nothing is written until an edit.
    #[test]
    fn a_bad_field_is_recovered_alone_after_an_exclusive_backup() {
        let dir = tmp();
        let mut doc = stored_doc();
        doc["app_settings"]["proxy"]["port"] = json!(70_000);
        write_doc(dir.path(), doc);
        let original = fs::read(dir.path().join(SETTINGS_FILE)).unwrap();
        let repo = SettingsRepository::open(dir.path().into());
        let snap = repo.snapshot();
        let LoadStatus::Recovered {
            fields,
            recovery_id,
        } = snap.status
        else {
            panic!("expected recovered, got {:?}", snap.status)
        };
        assert_eq!(fields, vec!["proxy.port".to_string()]);
        assert_eq!(snap.settings.download.video_quality, "1080p");
        assert_eq!(snap.settings.bridge.token, "paired-token");
        assert_eq!(snap.settings.proxy.port, 8080);
        // Original untouched; backup holds its exact bytes.
        assert_eq!(fs::read(dir.path().join(SETTINGS_FILE)).unwrap(), original);
        let backup: Value = serde_json::from_slice(
            &fs::read(
                dir.path()
                    .join(RECOVERY_DIR)
                    .join(format!("{recovery_id}.json")),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(backup["reason"], "field_recovery");
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(backup["originalSettingsBase64"].as_str().unwrap())
                .unwrap(),
            original
        );
        // The first explicit edit writes the whole recovered document.
        let c = repo.mutate(Some(7), |s| Ok(s.clone())).unwrap();
        assert!(c.changed);
        let disk: Value =
            serde_json::from_slice(&fs::read(dir.path().join(SETTINGS_FILE)).unwrap()).unwrap();
        assert_eq!(disk["app_settings"]["proxy"]["port"], json!(8080));
        assert_eq!(
            disk["app_settings"]["download"]["from_a_newer_version"],
            "keep me"
        );
        assert_eq!(repo.snapshot().status, LoadStatus::Loaded);
    }

    // API-08
    #[test]
    fn a_backup_failure_blocks_recovery_and_keeps_the_original() {
        for stage in [
            Stage::BackupDir,
            Stage::BackupCreate,
            Stage::BackupWrite,
            Stage::BackupSync,
        ] {
            let dir = tmp();
            let mut doc = stored_doc();
            doc["app_settings"]["proxy"]["port"] = json!(70_000);
            write_doc(dir.path(), doc);
            let original = fs::read(dir.path().join(SETTINGS_FILE)).unwrap();
            let repo = SettingsRepository::open_with_io(dir.path().into(), Faulty::new(stage));
            assert!(
                matches!(
                    repo.snapshot().status,
                    LoadStatus::Failed {
                        code: LoadFailure::BackupFailed,
                        ..
                    }
                ),
                "{stage:?}"
            );
            assert_eq!(
                repo.mutate(None, set_quality("480p")).unwrap_err(),
                RepoError::Unavailable(UnavailableCode::BackupFailed)
            );
            assert_eq!(fs::read(dir.path().join(SETTINGS_FILE)).unwrap(), original);
            assert!(recovery_files(dir.path()).is_empty(), "{stage:?}");
        }
    }

    // API-07 / API-08
    #[test]
    fn the_appearance_import_runs_once_with_a_backup_and_the_marker_in_one_commit() {
        let dir = tmp();
        let mut doc = stored_doc();
        doc["app_settings"]["appearance"]["theme"] = json!("dark");
        write_doc(dir.path(), doc);
        let original = fs::read(dir.path().join(SETTINGS_FILE)).unwrap();
        let repo = Arc::new(SettingsRepository::open(dir.path().into()));
        let raw = r#"{"preset":"amber-slate","mode":"light"}"#;
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let repo = repo.clone();
                std::thread::spawn(move || repo.import_appearance(Some(raw)).unwrap().0)
            })
            .collect();
        let mut outcomes: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        outcomes.sort_by_key(|o| *o == ImportOutcome::AlreadyImported);
        assert_eq!(
            outcomes,
            vec![ImportOutcome::Imported, ImportOutcome::AlreadyImported]
        );
        assert_eq!(repo.revision(), 8);
        let s = repo.settings();
        assert_eq!(s.appearance.migration_version, 1);
        assert_eq!(s.appearance.preset.as_deref(), Some("amber-slate"));
        assert_eq!(s.appearance.legacy_theme.as_deref(), Some("dark"));
        let backups = recovery_files(dir.path());
        assert_eq!(backups.len(), 1);
        let backup: Value = serde_json::from_slice(&fs::read(&backups[0]).unwrap()).unwrap();
        assert_eq!(backup["reason"], "appearance_migration");
        assert_eq!(backup["originalWorkspace"], raw);
        assert_eq!(backup["sourceRevision"], "7");
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(backup["originalSettingsBase64"].as_str().unwrap())
                .unwrap(),
            original
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let f = fs::metadata(&backups[0]).unwrap().permissions().mode() & 0o777;
            let d = fs::metadata(dir.path().join(RECOVERY_DIR))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!((f, d), (0o600, 0o700));
        }

        // The user edits after the import; a late import (lost response,
        // another WebView) never brings the old appearance back.
        repo.mutate(Some(8), |s| {
            crate::models::settings::apply_patch(s, &json!({ "appearance": { "mode": "dark" } }))
                .map_err(|_| RepoError::Validation(vec![]))
        })
        .unwrap();
        let (outcome, _) = repo.import_appearance(Some(r#"{"mode":"light"}"#)).unwrap();
        assert_eq!(outcome, ImportOutcome::AlreadyImported);
        assert_eq!(repo.settings().appearance.mode.as_deref(), Some("dark"));
        assert_eq!(recovery_files(dir.path()).len(), 1);

        // A restart reads the marker from disk.
        drop(repo);
        let reopened = SettingsRepository::open(dir.path().into());
        assert_eq!(
            reopened.import_appearance(None).unwrap().0,
            ImportOutcome::AlreadyImported
        );
    }

    // API-08: a failed backup blocks the migration.
    #[test]
    fn the_import_needs_its_backup() {
        let dir = tmp();
        write_doc(dir.path(), stored_doc());
        let original = fs::read(dir.path().join(SETTINGS_FILE)).unwrap();
        let repo =
            SettingsRepository::open_with_io(dir.path().into(), Faulty::new(Stage::BackupWrite));
        assert_eq!(
            repo.import_appearance(None).unwrap_err(),
            RepoError::Unavailable(UnavailableCode::BackupFailed)
        );
        assert_eq!(repo.settings().appearance.migration_version, 0);
        assert_eq!(fs::read(dir.path().join(SETTINGS_FILE)).unwrap(), original);
    }

    // API-08: an import on a new install backs up "no file" and still commits.
    #[test]
    fn a_new_install_import_records_the_missing_file() {
        let dir = tmp();
        let repo = SettingsRepository::open(dir.path().into());
        let (outcome, commit) = repo.import_appearance(None).unwrap();
        assert_eq!(outcome, ImportOutcome::Imported);
        assert_eq!(commit.revision, 1);
        assert_eq!(
            repo.settings().appearance.preset.as_deref(),
            Some("omniget-desktop")
        );
        let backups = recovery_files(dir.path());
        let backup: Value = serde_json::from_slice(&fs::read(&backups[0]).unwrap()).unwrap();
        assert_eq!(backup["originalSettingsBase64"], Value::Null);
        assert_eq!(backup["originalWorkspace"], Value::Null);
    }

    // API-11
    #[test]
    fn a_second_writer_of_the_same_profile_is_refused_before_writing() {
        let dir = tmp();
        write_doc(dir.path(), stored_doc());
        let owner = SettingsRepository::open(dir.path().into());
        let second = SettingsRepository::open(dir.path().into());
        assert!(owner.is_writable());
        assert!(!second.is_writable());
        let before = fs::read(dir.path().join(SETTINGS_FILE)).unwrap();
        assert_eq!(
            second.mutate(None, set_quality("480p")).unwrap_err(),
            RepoError::Unavailable(UnavailableCode::ProfileReadOnly)
        );
        assert_eq!(
            second.import_appearance(None).unwrap_err(),
            RepoError::Unavailable(UnavailableCode::ProfileReadOnly)
        );
        assert_eq!(fs::read(dir.path().join(SETTINGS_FILE)).unwrap(), before);
        // The second one still reads.
        assert_eq!(second.settings().download.video_quality, "1080p");
        drop(owner);
        drop(second);
        assert!(SettingsRepository::open(dir.path().into()).is_writable());
    }

    // API-11: the standalone reader observes the complete committed file.
    #[test]
    fn a_standalone_read_sees_the_committed_file() {
        let dir = tmp();
        write_doc(dir.path(), stored_doc());
        let repo = SettingsRepository::open(dir.path().into());
        repo.mutate(None, set_quality("480p")).unwrap();
        let read = crate::storage::config::read_settings_file(&dir.path().join(SETTINGS_FILE));
        assert_eq!(read.download.video_quality, "480p");
        assert_eq!(read.bridge.token, "paired-token");
    }

    #[test]
    fn a_hand_edit_is_picked_up_by_the_next_mutation() {
        let dir = tmp();
        write_doc(dir.path(), stored_doc());
        let repo = SettingsRepository::open(dir.path().into());
        let mut doc = stored_doc();
        doc["_settings_revision"] = json!(20);
        doc["app_settings"]["start_minimized"] = json!(true);
        write_doc(dir.path(), doc);
        assert_eq!(
            repo.mutate(Some(7), set_quality("480p")).unwrap_err(),
            RepoError::Conflict { current: 20 }
        );
        let c = repo.mutate(Some(20), set_quality("480p")).unwrap();
        assert_eq!(c.revision, 21);
        assert!(repo.settings().start_minimized);
    }

    #[test]
    fn api_errors_serialize_as_the_contract_says() {
        let v = serde_json::to_value(ApiError::from(RepoError::Conflict { current: u64::MAX }))
            .unwrap();
        assert_eq!(
            v,
            json!({ "kind": "conflict", "code": "revision_conflict",
                    "currentRevision": "18446744073709551615", "fieldErrors": [] })
        );
        let v = serde_json::to_value(ApiError::from(RepoError::Indeterminate {
            attempted: 9,
            observed: None,
        }))
        .unwrap();
        assert_eq!(
            v,
            json!({ "kind": "indeterminate", "code": "post_replace_sync_failed",
                    "attemptedRevision": "9", "observedRevision": null,
                    "requiresReload": true, "fieldErrors": [] })
        );
        let v = serde_json::to_value(ApiError::from(RepoError::Persistence(
            PersistenceCode::SyncFailed,
        )))
        .unwrap();
        assert_eq!(v["committed"], json!(false));
        let v = serde_json::to_value(ApiError::validation(vec![FieldError::new(
            "proxy.port",
            FieldErrorCode::OutOfRange,
        )]))
        .unwrap();
        assert_eq!(
            v,
            json!({ "kind": "validation", "code": "invalid_request",
                    "fieldErrors": [{ "path": "proxy.port", "code": "out_of_range" }] })
        );
        let v = serde_json::to_value(LoadStatus::Recovered {
            fields: vec!["proxy.port".into()],
            recovery_id: "id".into(),
        })
        .unwrap();
        assert_eq!(
            v,
            json!({ "kind": "recovered", "fields": ["proxy.port"], "recoveryId": "id" })
        );
    }
}
