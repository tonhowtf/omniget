//! `OMNIGET_DATA_DIR` for tests. The env is process-wide and cargo runs the
//! tests of this crate on threads, so a test that moved the data dir while
//! another read it made both flaky. Every test that sets it goes through
//! here, for its whole length:
//!
//! - [`shared_data_dir`]: the dir the git tests (vcs, threads, review, the
//!   job gate, snapshots) agree on. Many hold it at once.
//! - [`exclusive`]: any other value (or other env of the same kind). It waits
//!   for every holder of the shared dir and puts the previous values back
//!   when dropped.

use std::cell::RefCell;
use std::ffi::{OsStr, OsString};
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::{Mutex, RwLock, RwLockReadGuard, RwLockWriteGuard};

static DATA_DIR_ENV: RwLock<()> = RwLock::new(());
/// Readers set the shared value one at a time (only the first one after an
/// exclusive holder that had none actually writes).
static SETTING: Mutex<()> = Mutex::new(());

const KEY: &str = "OMNIGET_DATA_DIR";

pub struct Shared {
    path: PathBuf,
    _read: RwLockReadGuard<'static, ()>,
}

impl Deref for Shared {
    type Target = PathBuf;
    fn deref(&self) -> &PathBuf {
        &self.path
    }
}

/// `<tmp>/omniget-vcs-tests-data` as the data dir while the guard lives.
/// Never call it twice in one test: the lock is not reentrant.
pub fn shared_data_dir() -> Shared {
    let read = DATA_DIR_ENV.read().unwrap_or_else(|e| e.into_inner());
    let path = std::env::temp_dir().join("omniget-vcs-tests-data");
    std::fs::create_dir_all(&path).unwrap();
    {
        let _one = SETTING.lock().unwrap_or_else(|e| e.into_inner());
        if std::env::var_os(KEY).as_deref() != Some(path.as_os_str()) {
            std::env::set_var(KEY, &path);
        }
    }
    Shared { path, _read: read }
}

pub struct Exclusive {
    _write: RwLockWriteGuard<'static, ()>,
    previous: RefCell<Vec<(String, Option<OsString>)>>,
}

/// The env to change as one likes until the guard drops.
pub fn exclusive() -> Exclusive {
    Exclusive {
        _write: DATA_DIR_ENV.write().unwrap_or_else(|e| e.into_inner()),
        previous: RefCell::new(Vec::new()),
    }
}

impl Exclusive {
    /// Sets (`Some`) or removes (`None`) `key`; the old value comes back on drop.
    pub fn set(&self, key: &str, value: Option<&OsStr>) {
        self.previous
            .borrow_mut()
            .push((key.to_string(), std::env::var_os(key)));
        match value {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
    }

    pub fn set_data_dir(&self, dir: &std::path::Path) {
        self.set(KEY, Some(dir.as_os_str()));
    }
}

impl Drop for Exclusive {
    fn drop(&mut self) {
        for (key, value) in self.previous.borrow().iter().rev() {
            match value {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }
    }
}
