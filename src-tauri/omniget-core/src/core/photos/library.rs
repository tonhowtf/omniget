use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};

use super::{Edits, Photo};

const SCHEMA: &str = "
PRAGMA journal_mode=WAL;
PRAGMA busy_timeout=5000;
CREATE TABLE IF NOT EXISTS photos (
    path TEXT PRIMARY KEY,
    folder TEXT NOT NULL,
    name TEXT NOT NULL,
    format TEXT NOT NULL,
    size INTEGER NOT NULL,
    modified_at INTEGER NOT NULL,
    taken_at INTEGER,
    width INTEGER NOT NULL DEFAULT 0,
    height INTEGER NOT NULL DEFAULT 0,
    thumb TEXT,
    dhash INTEGER,
    content TEXT,
    error TEXT
);
CREATE TABLE IF NOT EXISTS folders (path TEXT PRIMARY KEY, position INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS edits (path TEXT PRIMARY KEY, data TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
";

#[derive(Debug, Clone, PartialEq)]
pub struct Stamp {
    pub size: u64,
    pub modified_at: i64,
    pub done: bool,
    pub thumb: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Entry {
    pub path: String,
    pub folder: String,
    pub name: String,
    pub format: String,
    pub size: u64,
    pub modified_at: i64,
    pub taken_at: Option<i64>,
    pub width: u32,
    pub height: u32,
    pub thumb: Option<String>,
    pub dhash: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Hashed {
    pub path: String,
    pub size: u64,
    pub width: u32,
    pub height: u32,
    pub taken_at: Option<i64>,
    pub modified_at: i64,
    pub dhash: Option<u64>,
    pub content: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Library {
    dir: PathBuf,
}

fn db_err(e: rusqlite::Error) -> String {
    format!("The photo library could not be read: {e}.")
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn parent_text(path: &Path) -> String {
    path.parent().map(path_text).unwrap_or_default()
}

fn name_text(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

impl Library {
    pub fn new(dir: impl Into<PathBuf>) -> Library {
        Library { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn thumbs_dir(&self) -> PathBuf {
        self.dir.join("thumbs")
    }

    pub fn thumb_path(&self, key: &str) -> PathBuf {
        self.thumbs_dir().join(format!("{key}.jpg"))
    }

    pub fn open(&self) -> Result<Connection, String> {
        std::fs::create_dir_all(self.thumbs_dir())
            .map_err(|e| format!("The photo library folder could not be created: {e}."))?;
        let conn = Connection::open(self.dir.join("library.db")).map_err(db_err)?;
        conn.execute_batch(SCHEMA).map_err(db_err)?;
        Ok(conn)
    }

    pub fn folders(&self) -> Result<Option<Vec<PathBuf>>, String> {
        let conn = self.open()?;
        let chosen: Option<String> = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'folders_chosen'",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_err)?;
        if chosen.is_none() {
            return Ok(None);
        }
        let mut stmt = conn
            .prepare("SELECT path FROM folders ORDER BY position")
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(db_err)?
            .filter_map(Result::ok)
            .map(PathBuf::from)
            .collect();
        Ok(Some(rows))
    }

    pub fn set_folders(&self, folders: &[PathBuf]) -> Result<(), String> {
        let mut conn = self.open()?;
        let tx = conn.transaction().map_err(db_err)?;
        tx.execute("DELETE FROM folders", []).map_err(db_err)?;
        for (i, folder) in folders.iter().enumerate() {
            tx.execute(
                "INSERT OR IGNORE INTO folders (path, position) VALUES (?1, ?2)",
                params![path_text(folder), i as i64],
            )
            .map_err(db_err)?;
        }
        tx.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES ('folders_chosen', '1')",
            [],
        )
        .map_err(db_err)?;
        tx.commit().map_err(db_err)
    }

    pub fn stamps(&self, conn: &Connection) -> Result<HashMap<String, Stamp>, String> {
        let mut stmt = conn
            .prepare("SELECT path, size, modified_at, thumb, error FROM photos")
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |r| {
                let thumb: Option<String> = r.get(3)?;
                let error: Option<String> = r.get(4)?;
                Ok((
                    r.get::<_, String>(0)?,
                    Stamp {
                        size: r.get::<_, i64>(1)? as u64,
                        modified_at: r.get(2)?,
                        done: thumb.is_some() || error.is_some(),
                        thumb,
                    },
                ))
            })
            .map_err(db_err)?
            .filter_map(Result::ok)
            .collect();
        Ok(rows)
    }

    pub fn upsert(&self, conn: &Connection, e: &Entry) -> Result<(), String> {
        conn.execute(
            "INSERT INTO photos (path, folder, name, format, size, modified_at, taken_at, width, height, thumb, dhash, content, error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL, ?12)
             ON CONFLICT(path) DO UPDATE SET folder = ?2, name = ?3, format = ?4, size = ?5, modified_at = ?6,
               taken_at = ?7, width = ?8, height = ?9, thumb = ?10, dhash = ?11, content = NULL, error = ?12",
            params![
                e.path,
                e.folder,
                e.name,
                e.format,
                e.size as i64,
                e.modified_at,
                e.taken_at,
                e.width,
                e.height,
                e.thumb,
                e.dhash.map(|h| h as i64),
                e.error,
            ],
        )
        .map_err(db_err)?;
        Ok(())
    }

    pub fn list(&self) -> Result<Vec<Photo>, String> {
        let conn = self.open()?;
        let mut stmt = conn
            .prepare(
                "SELECT p.path, p.name, p.folder, p.format, p.size, p.modified_at, p.taken_at, p.width, p.height,
                        p.thumb, p.error, e.path IS NOT NULL
                 FROM photos p LEFT JOIN edits e ON e.path = p.path
                 ORDER BY COALESCE(p.taken_at, p.modified_at) DESC, p.path",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |r| {
                let thumb: Option<String> = r.get(9)?;
                Ok(Photo {
                    path: r.get(0)?,
                    name: r.get(1)?,
                    folder: r.get(2)?,
                    format: r.get(3)?,
                    size: r.get::<_, i64>(4)? as u64,
                    modified_at: r.get(5)?,
                    taken_at: r.get(6)?,
                    width: r.get(7)?,
                    height: r.get(8)?,
                    thumb: thumb.map(|k| path_text(&self.thumb_path(&k))),
                    error: r.get(10)?,
                    edited: r.get(11)?,
                })
            })
            .map_err(db_err)?
            .filter_map(Result::ok)
            .collect();
        Ok(rows)
    }

    pub fn format_of(&self, path: &str) -> Result<Option<String>, String> {
        let conn = self.open()?;
        conn.query_row("SELECT format FROM photos WHERE path = ?1", [path], |r| {
            r.get(0)
        })
        .optional()
        .map_err(db_err)
    }

    pub fn hashed(&self) -> Result<Vec<Hashed>, String> {
        let conn = self.open()?;
        let mut stmt = conn
            .prepare(
                "SELECT path, size, width, height, taken_at, modified_at, dhash, content
                 FROM photos WHERE error IS NULL AND thumb IS NOT NULL",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(Hashed {
                    path: r.get(0)?,
                    size: r.get::<_, i64>(1)? as u64,
                    width: r.get(2)?,
                    height: r.get(3)?,
                    taken_at: r.get(4)?,
                    modified_at: r.get(5)?,
                    dhash: r.get::<_, Option<i64>>(6)?.map(|h| h as u64),
                    content: r.get(7)?,
                })
            })
            .map_err(db_err)?
            .filter_map(Result::ok)
            .collect();
        Ok(rows)
    }

    pub fn set_content(&self, values: &[(String, String)]) -> Result<(), String> {
        let mut conn = self.open()?;
        let tx = conn.transaction().map_err(db_err)?;
        for (path, content) in values {
            tx.execute(
                "UPDATE photos SET content = ?2 WHERE path = ?1",
                params![path, content],
            )
            .map_err(db_err)?;
        }
        tx.commit().map_err(db_err)
    }

    pub fn edits(&self, path: &str) -> Result<Option<Edits>, String> {
        let conn = self.open()?;
        let data: Option<String> = conn
            .query_row("SELECT data FROM edits WHERE path = ?1", [path], |r| {
                r.get(0)
            })
            .optional()
            .map_err(db_err)?;
        Ok(data.and_then(|d| serde_json::from_str::<Edits>(&d).ok()))
    }

    pub fn set_edits(&self, path: &str, edits: Option<&Edits>) -> Result<(), String> {
        let conn = self.open()?;
        match edits.map(Edits::normalized).filter(|e| !e.is_identity()) {
            Some(e) => {
                let data = serde_json::to_string(&e).map_err(|e| e.to_string())?;
                conn.execute(
                    "INSERT OR REPLACE INTO edits (path, data) VALUES (?1, ?2)",
                    params![path, data],
                )
                .map_err(db_err)?;
            }
            None => {
                conn.execute("DELETE FROM edits WHERE path = ?1", [path])
                    .map_err(db_err)?;
            }
        }
        Ok(())
    }

    pub fn forget(&self, conn: &Connection, paths: &[String]) -> Result<(), String> {
        for path in paths {
            let thumb: Option<String> = conn
                .query_row("SELECT thumb FROM photos WHERE path = ?1", [path], |r| {
                    r.get(0)
                })
                .optional()
                .map_err(db_err)?
                .flatten();
            conn.execute("DELETE FROM photos WHERE path = ?1", [path])
                .map_err(db_err)?;
            conn.execute("DELETE FROM edits WHERE path = ?1", [path])
                .map_err(db_err)?;
            if let Some(key) = thumb {
                let _ = std::fs::remove_file(self.thumb_path(&key));
            }
        }
        Ok(())
    }

    pub fn forget_paths(&self, paths: &[String]) -> Result<(), String> {
        let mut conn = self.open()?;
        let tx = conn.transaction().map_err(db_err)?;
        self.forget(&tx, paths)?;
        tx.commit().map_err(db_err)
    }

    pub fn relocate(&self, from: &str, to: &Path) -> Result<(), String> {
        let conn = self.open()?;
        let to_text = path_text(to);
        conn.execute("DELETE FROM photos WHERE path = ?1", [&to_text])
            .map_err(db_err)?;
        conn.execute(
            "UPDATE photos SET path = ?2, folder = ?3, name = ?4 WHERE path = ?1",
            params![from, to_text, parent_text(to), name_text(to)],
        )
        .map_err(db_err)?;
        conn.execute(
            "UPDATE OR REPLACE edits SET path = ?2 WHERE path = ?1",
            params![from, to_text],
        )
        .map_err(db_err)?;
        Ok(())
    }
}

pub fn entry_for(path: &Path, format: &str, size: u64, modified_at: i64) -> Entry {
    Entry {
        path: path_text(path),
        folder: parent_text(path),
        name: name_text(path),
        format: format.to_string(),
        size,
        modified_at,
        ..Default::default()
    }
}
