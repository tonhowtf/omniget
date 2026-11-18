use std::path::Path;

use serde::{Deserialize, Serialize};

use super::QuickError;

pub const FILE: &str = "quick-tools.json";
pub const MAX_SNIPPETS: usize = 100;
pub const MAX_NAME_CHARS: usize = 60;
pub const MAX_TEXT_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snippet {
    pub id: u32,
    pub name: String,
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Store {
    pub clipboard_history: bool,
    pub snippets: Vec<Snippet>,
}

impl Store {
    pub fn save_snippet(
        &mut self,
        id: Option<u32>,
        name: &str,
        text: &str,
    ) -> Result<Snippet, QuickError> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
            return Err(QuickError::InvalidInput(format!(
                "name must have 1 to {MAX_NAME_CHARS} characters"
            )));
        }
        if text.trim().is_empty() || text.len() > MAX_TEXT_BYTES {
            return Err(QuickError::InvalidInput(format!(
                "text must have 1 to {MAX_TEXT_BYTES} bytes"
            )));
        }
        match id {
            Some(id) => {
                let found = self
                    .snippets
                    .iter_mut()
                    .find(|s| s.id == id)
                    .ok_or_else(|| QuickError::NotFound(format!("no snippet with id {id}")))?;
                found.name = name.to_string();
                found.text = text.to_string();
                Ok(found.clone())
            }
            None => {
                if self.snippets.len() >= MAX_SNIPPETS {
                    return Err(QuickError::InvalidInput(format!(
                        "the list already has {MAX_SNIPPETS} snippets; delete one first"
                    )));
                }
                let id = self.snippets.iter().map(|s| s.id).max().unwrap_or(0) + 1;
                let snippet = Snippet {
                    id,
                    name: name.to_string(),
                    text: text.to_string(),
                };
                self.snippets.push(snippet.clone());
                Ok(snippet)
            }
        }
    }

    pub fn delete_snippet(&mut self, id: u32) -> Result<(), QuickError> {
        let before = self.snippets.len();
        self.snippets.retain(|s| s.id != id);
        if self.snippets.len() == before {
            return Err(QuickError::NotFound(format!("no snippet with id {id}")));
        }
        Ok(())
    }

    pub fn snippet(&self, id: u32) -> Result<&Snippet, QuickError> {
        self.snippets
            .iter()
            .find(|s| s.id == id)
            .ok_or_else(|| QuickError::NotFound(format!("no snippet with id {id}")))
    }
}

fn dir() -> Result<std::path::PathBuf, QuickError> {
    crate::core::paths::app_data_dir().ok_or_else(|| QuickError::Store("no data directory".into()))
}

pub fn load() -> Store {
    dir().map(|d| load_from(&d)).unwrap_or_default()
}

pub fn save(store: &Store) -> Result<(), QuickError> {
    save_in(&dir()?, store)
}

pub fn load_from(dir: &Path) -> Store {
    let Ok(file) = std::fs::File::open(dir.join(FILE)) else {
        return Store::default();
    };
    let cap = (MAX_SNIPPETS * (MAX_TEXT_BYTES + 512)) as u64;
    let mut store: Store =
        serde_json::from_reader(std::io::Read::take(file, cap)).unwrap_or_default();
    store.snippets.truncate(MAX_SNIPPETS);
    store
}

pub fn save_in(dir: &Path, store: &Store) -> Result<(), QuickError> {
    let failed = |e: std::io::Error| QuickError::Store(e.to_string());
    std::fs::create_dir_all(dir).map_err(failed)?;
    let body = serde_json::to_vec_pretty(store).map_err(|e| QuickError::Store(e.to_string()))?;
    let tmp = dir.join(format!("{FILE}.tmp"));
    std::fs::write(&tmp, &body).map_err(failed)?;
    std::fs::rename(&tmp, dir.join(FILE)).map_err(failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_snippet_gets_the_next_id_and_trimmed_name() {
        let mut s = Store::default();
        let a = s.save_snippet(None, "  Email ", "me@example.com").unwrap();
        let b = s.save_snippet(None, "Address", "1 Main St").unwrap();
        assert_eq!((a.id, a.name.as_str()), (1, "Email"));
        assert_eq!(b.id, 2);
        s.delete_snippet(1).unwrap();
        assert_eq!(s.save_snippet(None, "Third", "x").unwrap().id, 3);
    }

    #[test]
    fn saving_with_an_id_edits_in_place() {
        let mut s = Store::default();
        s.save_snippet(None, "Email", "old").unwrap();
        let edited = s.save_snippet(Some(1), "Mail", "new").unwrap();
        assert_eq!(edited.text, "new");
        assert_eq!(s.snippets.len(), 1);
        assert_eq!(s.snippet(1).unwrap().name, "Mail");
    }

    #[test]
    fn empty_or_oversized_fields_are_refused() {
        let mut s = Store::default();
        for (name, text) in [
            ("", "x"),
            ("   ", "x"),
            ("ok", ""),
            ("ok", " \n"),
            (&"n".repeat(MAX_NAME_CHARS + 1), "x"),
            ("ok", &"x".repeat(MAX_TEXT_BYTES + 1)),
        ] {
            let err = s.save_snippet(None, name, text).unwrap_err();
            assert!(matches!(err, QuickError::InvalidInput(_)), "{name:?}");
        }
        assert!(s.snippets.is_empty());
    }

    #[test]
    fn unknown_ids_are_not_found_and_the_list_has_a_ceiling() {
        let mut s = Store::default();
        assert!(matches!(
            s.save_snippet(Some(4), "a", "b"),
            Err(QuickError::NotFound(_))
        ));
        assert!(matches!(s.delete_snippet(4), Err(QuickError::NotFound(_))));
        assert!(matches!(s.snippet(4), Err(QuickError::NotFound(_))));
        for n in 0..MAX_SNIPPETS {
            s.save_snippet(None, &format!("s{n}"), "x").unwrap();
        }
        assert!(matches!(
            s.save_snippet(None, "one more", "x"),
            Err(QuickError::InvalidInput(_))
        ));
    }

    #[test]
    fn the_store_survives_a_round_trip_and_a_broken_file_reads_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load_from(dir.path()), Store::default());
        let mut s = Store {
            clipboard_history: true,
            ..Store::default()
        };
        s.save_snippet(None, "Email", "me@example.com").unwrap();
        save_in(dir.path(), &s).unwrap();
        assert_eq!(load_from(dir.path()), s);
        std::fs::write(dir.path().join(FILE), "{not json").unwrap();
        assert_eq!(load_from(dir.path()), Store::default());
    }
}
