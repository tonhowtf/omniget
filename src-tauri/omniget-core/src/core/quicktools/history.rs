use serde::Serialize;

pub const MAX_ENTRIES: usize = 50;
pub const MAX_BYTES: usize = 64 * 1024;
const PREVIEW_CHARS: usize = 200;

struct Entry {
    id: u64,
    text: String,
    pinned: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EntryView {
    pub id: u64,
    pub preview: String,
    pub chars: usize,
    pub pinned: bool,
}

#[derive(Default)]
pub struct History {
    entries: Vec<Entry>,
    next_id: u64,
}

impl History {
    pub fn push(&mut self, text: &str) -> bool {
        if text.trim().is_empty() || text.len() > MAX_BYTES {
            return false;
        }
        if let Some(at) = self.entries.iter().position(|e| e.text == text) {
            if at == 0 {
                return false;
            }
            let entry = self.entries.remove(at);
            self.entries.insert(0, entry);
            return true;
        }
        self.next_id += 1;
        self.entries.insert(
            0,
            Entry {
                id: self.next_id,
                text: text.to_string(),
                pinned: false,
            },
        );
        while self.entries.len() > MAX_ENTRIES {
            match self.entries.iter().rposition(|e| !e.pinned) {
                Some(at) => {
                    self.entries.remove(at);
                }
                None => break,
            }
        }
        self.entries.first().is_some_and(|e| e.id == self.next_id)
    }

    pub fn text(&self, id: u64) -> Option<&str> {
        self.entries
            .iter()
            .find(|e| e.id == id)
            .map(|e| e.text.as_str())
    }

    pub fn set_pinned(&mut self, id: u64, pinned: bool) -> bool {
        match self.entries.iter_mut().find(|e| e.id == id) {
            Some(entry) => {
                entry.pinned = pinned;
                true
            }
            None => false,
        }
    }

    pub fn remove(&mut self, id: u64) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| e.id != id);
        self.entries.len() != before
    }

    pub fn clear_unpinned(&mut self) {
        self.entries.retain(|e| e.pinned);
    }

    pub fn wipe(&mut self) {
        self.entries = Vec::new();
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn views(&self) -> Vec<EntryView> {
        let view = |e: &Entry| EntryView {
            id: e.id,
            preview: preview(&e.text),
            chars: e.text.chars().count(),
            pinned: e.pinned,
        };
        let pinned = self.entries.iter().filter(|e| e.pinned).map(view);
        let rest = self.entries.iter().filter(|e| !e.pinned).map(view);
        pinned.chain(rest).collect()
    }
}

fn preview(text: &str) -> String {
    let mut out = String::new();
    for (count, ch) in text.trim().chars().enumerate() {
        if count == PREVIEW_CHARS {
            out.push('…');
            break;
        }
        out.push(if ch.is_whitespace() { ' ' } else { ch });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(h: &History) -> Vec<u64> {
        h.views().iter().map(|v| v.id).collect()
    }

    #[test]
    fn the_newest_copy_comes_first() {
        let mut h = History::default();
        assert!(h.push("one"));
        assert!(h.push("two"));
        assert_eq!(h.views()[0].preview, "two");
        assert_eq!(h.views()[1].preview, "one");
    }

    #[test]
    fn blank_and_oversized_text_is_skipped() {
        let mut h = History::default();
        assert!(!h.push(""));
        assert!(!h.push("  \n\t"));
        assert!(!h.push(&"x".repeat(MAX_BYTES + 1)));
        assert!(h.is_empty());
        assert!(h.push(&"x".repeat(MAX_BYTES)));
        assert_eq!(h.len(), 1);
    }

    #[test]
    fn copying_the_same_text_again_moves_it_up_instead_of_duplicating() {
        let mut h = History::default();
        h.push("a");
        h.push("b");
        assert!(!h.push("b"));
        assert!(h.push("a"));
        assert_eq!(h.len(), 2);
        assert_eq!(h.views()[0].preview, "a");
        assert_eq!(ids(&h), vec![1, 2]);
    }

    #[test]
    fn only_the_last_fifty_are_kept() {
        let mut h = History::default();
        for n in 0..MAX_ENTRIES + 10 {
            h.push(&format!("entry {n}"));
        }
        assert_eq!(h.len(), MAX_ENTRIES);
        assert_eq!(h.views()[0].preview, "entry 59");
        assert_eq!(h.views()[MAX_ENTRIES - 1].preview, "entry 10");
    }

    #[test]
    fn a_pinned_entry_survives_the_cap_and_clear_and_is_listed_first() {
        let mut h = History::default();
        h.push("keep me");
        assert!(h.set_pinned(1, true));
        for n in 0..MAX_ENTRIES + 5 {
            h.push(&format!("entry {n}"));
        }
        assert_eq!(h.len(), MAX_ENTRIES);
        assert_eq!(h.views()[0].preview, "keep me");
        assert!(h.views()[0].pinned);
        h.clear_unpinned();
        assert_eq!(ids(&h), vec![1]);
        h.wipe();
        assert!(h.is_empty());
    }

    #[test]
    fn when_every_slot_is_pinned_a_new_copy_is_not_stored() {
        let mut h = History::default();
        for n in 0..MAX_ENTRIES {
            h.push(&format!("entry {n}"));
            h.set_pinned(n as u64 + 1, true);
        }
        assert!(!h.push("one more"));
        assert_eq!(h.len(), MAX_ENTRIES);
        assert!(h.views().iter().all(|v| v.pinned));
    }

    #[test]
    fn delete_and_text_use_the_id_and_unknown_ids_change_nothing() {
        let mut h = History::default();
        h.push("secret");
        assert_eq!(h.text(1), Some("secret"));
        assert_eq!(h.text(7), None);
        assert!(!h.remove(7));
        assert!(!h.set_pinned(7, true));
        assert!(h.remove(1));
        assert!(h.is_empty());
    }

    #[test]
    fn the_preview_is_one_short_line_and_reports_the_real_length() {
        let mut h = History::default();
        h.push(&format!("  first\nsecond\t{}", "é".repeat(300)));
        let v = &h.views()[0];
        assert!(v.preview.starts_with("first second "));
        assert_eq!(v.preview.chars().count(), PREVIEW_CHARS + 1);
        assert!(v.preview.ends_with('…'));
        assert_eq!(v.chars, 2 + 5 + 1 + 6 + 1 + 300);
    }
}
