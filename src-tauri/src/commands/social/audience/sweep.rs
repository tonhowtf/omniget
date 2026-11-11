use serde_json::{json, Value};

pub const QUERIES: &str = "abcdefghijklmnopqrstuvwxyz0123456789_.";
pub const BURST: u32 = 4;
pub const REST: i64 = 300;
pub const BLOCK_WAIT: i64 = 3600;
pub const MAX_REQUESTS: u32 = 120;
const TAG: &str = "sweep";

#[derive(Clone, Debug, PartialEq)]
pub struct State {
    pub query: usize,
    pub used: u32,
    pub not_before: i64,
    pub cursor: Option<String>,
}

pub fn begin(total: i64, expected: Option<i64>) -> Option<String> {
    expected.filter(|e| total < *e).map(|_| {
        State {
            query: 0,
            used: 0,
            not_before: 0,
            cursor: None,
        }
        .encode()
    })
}

pub fn reached(total: i64, expected: Option<i64>) -> bool {
    expected.is_some_and(|e| total >= e)
}

impl State {
    pub fn parse(raw: &str) -> Option<Self> {
        let mut parts = raw.splitn(5, ':');
        if parts.next()? != TAG {
            return None;
        }
        let query = parts.next()?.parse().ok().filter(|q| *q < QUERIES.len())?;
        let used = parts.next()?.parse().ok()?;
        let not_before = parts.next()?.parse().ok()?;
        let cursor = parts.next().filter(|c| !c.is_empty()).map(String::from);
        Some(Self {
            query,
            used,
            not_before,
            cursor,
        })
    }

    pub fn encode(&self) -> String {
        format!(
            "{TAG}:{}:{}:{}:{}",
            self.query,
            self.used,
            self.not_before,
            self.cursor.as_deref().unwrap_or_default()
        )
    }

    pub fn args(&self) -> Value {
        let mut a = json!({ "query": &QUERIES[self.query..self.query + 1] });
        if let Some(c) = &self.cursor {
            a["cursor"] = json!(c);
        }
        a
    }

    pub fn wait(&self, now: i64) -> Option<u64> {
        (self.not_before > now).then(|| (self.not_before - now) as u64)
    }

    pub fn after(&self, page_next: Option<String>, now: i64) -> Option<Self> {
        let used = self.used + 1;
        let query = if page_next.is_some() {
            self.query
        } else {
            self.query + 1
        };
        if query >= QUERIES.len() || used >= MAX_REQUESTS {
            return None;
        }
        Some(Self {
            query,
            used,
            not_before: if used.is_multiple_of(BURST) {
                now + REST
            } else {
                self.not_before
            },
            cursor: page_next,
        })
    }

    pub fn blocked(&self, now: i64) -> Self {
        Self {
            not_before: now + BLOCK_WAIT,
            ..self.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start() -> State {
        State::parse(&begin(198, Some(222)).unwrap()).unwrap()
    }

    #[test]
    fn the_sweep_starts_only_when_the_list_is_below_the_profile_count() {
        assert_eq!(begin(222, Some(222)), None);
        assert_eq!(begin(230, Some(222)), None);
        assert_eq!(begin(10, None), None);
        assert_eq!(begin(221, Some(222)).as_deref(), Some("sweep:0:0:0:"));
        assert!(reached(222, Some(222)));
        assert!(!reached(221, Some(222)));
        assert!(!reached(500, None));
    }

    #[test]
    fn the_state_survives_the_cursor_column() {
        let s = State {
            query: 3,
            used: 7,
            not_before: 1_700_000_000,
            cursor: Some("123~QVFE:abc==".into()),
        };
        assert_eq!(s.encode(), "sweep:3:7:1700000000:123~QVFE:abc==");
        assert_eq!(State::parse(&s.encode()), Some(s));
        assert_eq!(State::parse("123~50"), None);
        assert_eq!(State::parse("sweep:99:0:0:"), None);
        assert_eq!(State::parse("sweep:x:0:0:"), None);
    }

    #[test]
    fn each_query_is_one_character_with_its_own_paging() {
        assert_eq!(QUERIES.len(), 38);
        let s = start();
        assert_eq!(s.args(), json!({ "query": "a" }));
        let paged = s.after(Some("9~c1".into()), 100).unwrap();
        assert_eq!(paged.args(), json!({ "query": "a", "cursor": "9~c1" }));
        let b = paged.after(None, 100).unwrap();
        assert_eq!(b.args(), json!({ "query": "b" }));
        assert_eq!(b.used, 2);
        let last = State {
            query: 37,
            ..start()
        };
        assert_eq!(last.args(), json!({ "query": "." }));
        assert_eq!(last.after(None, 100), None);
        assert!(last.after(Some("9~c".into()), 100).is_some());
    }

    #[test]
    fn it_rests_after_each_burst_and_stops_at_the_hard_cap() {
        let mut s = start();
        assert_eq!(s.wait(1000), None);
        for _ in 0..BURST - 1 {
            s = s.after(None, 1000).unwrap();
            assert_eq!(s.wait(1000), None);
        }
        s = s.after(None, 1000).unwrap();
        assert_eq!(s.wait(1000), Some(REST as u64));
        assert_eq!(s.wait(1000 + REST), None);
        let blocked = s.blocked(5000);
        assert_eq!(blocked.wait(5000), Some(3600));
        assert_eq!(blocked.query, s.query);
        let spent = State {
            used: MAX_REQUESTS - 1,
            ..start()
        };
        assert_eq!(spent.after(Some("9~c".into()), 0), None);
    }
}
