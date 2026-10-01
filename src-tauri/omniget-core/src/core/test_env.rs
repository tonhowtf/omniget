use std::cell::RefCell;
use std::ffi::{OsStr, OsString};
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

static DATA_DIR_ENV: RwLock<()> = RwLock::new(());

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

pub struct Exclusive {
    _write: RwLockWriteGuard<'static, ()>,
    previous: RefCell<Vec<(String, Option<OsString>)>>,
}

pub fn exclusive() -> Exclusive {
    Exclusive {
        _write: DATA_DIR_ENV.write().unwrap_or_else(|e| e.into_inner()),
        previous: RefCell::new(Vec::new()),
    }
}

impl Exclusive {
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
