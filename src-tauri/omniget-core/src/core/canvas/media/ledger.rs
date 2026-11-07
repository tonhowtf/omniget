use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::Serialize;
use std::path::Path;
use std::sync::Mutex;

pub const QUOTE_MS: i64 = 5 * 60 * 1000;
pub const UPLOAD_TTL_MS: i64 = 24 * 60 * 60 * 1000;
pub const HOUR_MS: i64 = 60 * 60 * 1000;
pub const DEFAULT_AUTO_PER_GEN: i64 = 10_000_000;
pub const DEFAULT_AUTO_PER_HOUR: i64 = 3;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS canvas_media_budgets(workspace TEXT PRIMARY KEY, ceiling INTEGER NOT NULL DEFAULT 0 CHECK(ceiling>=0), reserved INTEGER NOT NULL DEFAULT 0 CHECK(reserved>=0), spent INTEGER NOT NULL DEFAULT 0 CHECK(spent>=0), auto_per_gen INTEGER NOT NULL DEFAULT 10000000 CHECK(auto_per_gen>=0), auto_per_hour INTEGER NOT NULL DEFAULT 3 CHECK(auto_per_hour>=0));
CREATE TABLE IF NOT EXISTS canvas_media_quotes(id TEXT PRIMARY KEY, workspace TEXT NOT NULL, node TEXT NOT NULL, fingerprint TEXT NOT NULL, credits INTEGER NOT NULL CHECK(credits>=0), balance INTEGER, account TEXT, expires_ms INTEGER NOT NULL, used INTEGER NOT NULL DEFAULT 0, created_ms INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS canvas_media_intents(id TEXT PRIMARY KEY, workspace TEXT NOT NULL, node TEXT NOT NULL, intent_key TEXT NOT NULL, fingerprint TEXT NOT NULL, quote_id TEXT NOT NULL UNIQUE REFERENCES canvas_media_quotes(id), reserved INTEGER NOT NULL CHECK(reserved>=0), actual INTEGER, actual_ref TEXT UNIQUE, provider_id TEXT UNIQUE, state TEXT NOT NULL, actor TEXT NOT NULL, model TEXT NOT NULL, prompt TEXT NOT NULL, version INTEGER, error TEXT, created_ms INTEGER NOT NULL, updated_ms INTEGER NOT NULL, UNIQUE(workspace,node,intent_key));
CREATE INDEX IF NOT EXISTS canvas_media_intents_node ON canvas_media_intents(workspace,node,created_ms);
CREATE TABLE IF NOT EXISTS canvas_media_uploads(hash TEXT NOT NULL, account TEXT NOT NULL, upload_id TEXT NOT NULL, created_ms INTEGER NOT NULL, PRIMARY KEY(hash,account));
";

fn db_error(e: rusqlite::Error) -> String {
    tracing::warn!("[canvas-media] storage: {e}");
    "PROVIDER_STORAGE_ERROR".into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Actor {
    Person,
    Agent,
}

impl Actor {
    fn as_str(self) -> &'static str {
        match self {
            Actor::Person => "person",
            Actor::Agent => "agent",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Budget {
    pub workspace: String,
    pub ceiling: i64,
    pub reserved: i64,
    pub spent: i64,
    pub auto_per_gen: i64,
    pub auto_per_hour: i64,
}

impl Budget {
    pub fn left(&self) -> i64 {
        (self.ceiling - self.reserved - self.spent).max(0)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Quote {
    pub id: String,
    pub workspace: String,
    pub node: String,
    pub fingerprint: String,
    pub credits: i64,
    pub balance: Option<i64>,
    pub expires_ms: i64,
    pub used: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Intent {
    pub id: String,
    pub workspace: String,
    pub node: String,
    pub key: String,
    pub fingerprint: String,
    pub quote_id: String,
    pub reserved: i64,
    pub actual: Option<i64>,
    pub provider_id: Option<String>,
    pub state: String,
    pub actor: String,
    pub model: String,
    pub prompt: String,
    pub version: Option<i64>,
    pub error: Option<String>,
    pub created_ms: i64,
    pub updated_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Assessment {
    pub fits_budget: bool,
    pub fits_balance: bool,
    pub needs_approval: bool,
    pub left: i64,
    pub reason: Option<String>,
}

pub struct Reserve<'a> {
    pub workspace: &'a str,
    pub node: &'a str,
    pub key: &'a str,
    pub quote: &'a str,
    pub fingerprint: &'a str,
    pub actor: Actor,
    pub model: &'a str,
    pub prompt: &'a str,
    pub balance: i64,
    pub now: i64,
}

pub struct Ledger {
    conn: Mutex<Connection>,
}

const INTENT_COLS: &str = "id,workspace,node,intent_key,fingerprint,quote_id,reserved,actual,provider_id,state,actor,model,prompt,version,error,created_ms,updated_ms";

fn intent_row(r: &rusqlite::Row) -> rusqlite::Result<Intent> {
    Ok(Intent {
        id: r.get(0)?,
        workspace: r.get(1)?,
        node: r.get(2)?,
        key: r.get(3)?,
        fingerprint: r.get(4)?,
        quote_id: r.get(5)?,
        reserved: r.get(6)?,
        actual: r.get(7)?,
        provider_id: r.get(8)?,
        state: r.get(9)?,
        actor: r.get(10)?,
        model: r.get(11)?,
        prompt: r.get(12)?,
        version: r.get(13)?,
        error: r.get(14)?,
        created_ms: r.get(15)?,
        updated_ms: r.get(16)?,
    })
}

fn budget_in(c: &Connection, ws: &str) -> Result<Budget, String> {
    Ok(c.query_row(
        "SELECT ceiling,reserved,spent,auto_per_gen,auto_per_hour FROM canvas_media_budgets WHERE workspace=?1",
        [ws],
        |r| {
            Ok(Budget {
                workspace: ws.to_string(),
                ceiling: r.get(0)?,
                reserved: r.get(1)?,
                spent: r.get(2)?,
                auto_per_gen: r.get(3)?,
                auto_per_hour: r.get(4)?,
            })
        },
    )
    .optional()
    .map_err(db_error)?
    .unwrap_or(Budget {
        workspace: ws.to_string(),
        ceiling: 0,
        reserved: 0,
        spent: 0,
        auto_per_gen: DEFAULT_AUTO_PER_GEN,
        auto_per_hour: DEFAULT_AUTO_PER_HOUR,
    }))
}

fn ensure_budget(tx: &Transaction, ws: &str) -> Result<(), String> {
    tx.execute(
        "INSERT OR IGNORE INTO canvas_media_budgets(workspace) VALUES(?1)",
        [ws],
    )
    .map_err(db_error)?;
    Ok(())
}

fn agent_recent(c: &Connection, ws: &str, now: i64) -> Result<i64, String> {
    c.query_row(
        "SELECT COUNT(*) FROM canvas_media_intents WHERE workspace=?1 AND actor='agent' AND created_ms>?2",
        params![ws, now - HOUR_MS],
        |r| r.get(0),
    )
    .map_err(db_error)
}

fn assess_in(
    c: &Connection,
    ws: &str,
    amount: i64,
    balance: Option<i64>,
    now: i64,
) -> Result<Assessment, String> {
    let b = budget_in(c, ws)?;
    let left = b.left();
    let fits_budget = amount <= left;
    let fits_balance = balance.is_none_or(|bal| amount <= bal);
    let recent = agent_recent(c, ws, now)?;
    let needs_approval = amount > b.auto_per_gen || recent >= b.auto_per_hour;
    let reason = if b.ceiling == 0 {
        Some("PROVIDER_BUDGET_NOT_SET: the workspace budget is 0; the person sets it in the generation node, or call canvas_generate_budget {workspaceId, ceiling}".to_string())
    } else if !fits_budget {
        Some(format!(
            "PROVIDER_BUDGET_EXCEEDED: needs {:.2} credits, {:.2} left in the workspace budget",
            amount as f64 / 1e6,
            left as f64 / 1e6
        ))
    } else if !fits_balance {
        Some(
            "PROVIDER_BALANCE_LOW: the Higgsfield account does not have enough credits".to_string(),
        )
    } else {
        None
    };
    Ok(Assessment {
        fits_budget,
        fits_balance,
        needs_approval,
        left,
        reason,
    })
}

impl Ledger {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("PROVIDER_STORAGE_ERROR: {e}"))?;
        }
        let conn = Connection::open(path).map_err(db_error)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(db_error)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")
            .map_err(db_error)?;
        conn.execute_batch(SCHEMA).map_err(db_error)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn with<T>(&self, f: impl FnOnce(&mut Connection) -> Result<T, String>) -> Result<T, String> {
        let mut c = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut c)
    }

    fn tx<T>(&self, f: impl FnOnce(&Transaction) -> Result<T, String>) -> Result<T, String> {
        self.with(|c| {
            let tx = c
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(db_error)?;
            let out = f(&tx)?;
            tx.commit().map_err(db_error)?;
            Ok(out)
        })
    }

    pub fn budget(&self, ws: &str) -> Result<Budget, String> {
        self.with(|c| budget_in(c, ws))
    }

    pub fn set_budget(
        &self,
        ws: &str,
        ceiling: Option<i64>,
        auto_per_gen: Option<i64>,
        auto_per_hour: Option<i64>,
    ) -> Result<Budget, String> {
        for v in [ceiling, auto_per_gen, auto_per_hour].into_iter().flatten() {
            if !(0..=1_000_000_000_000_000).contains(&v) {
                return Err("PROVIDER_INVALID_BUDGET".into());
            }
        }
        self.tx(|tx| {
            ensure_budget(tx, ws)?;
            if let Some(c) = ceiling {
                tx.execute(
                    "UPDATE canvas_media_budgets SET ceiling=?2 WHERE workspace=?1",
                    params![ws, c],
                )
                .map_err(db_error)?;
            }
            if let Some(v) = auto_per_gen {
                tx.execute(
                    "UPDATE canvas_media_budgets SET auto_per_gen=?2 WHERE workspace=?1",
                    params![ws, v],
                )
                .map_err(db_error)?;
            }
            if let Some(v) = auto_per_hour {
                tx.execute(
                    "UPDATE canvas_media_budgets SET auto_per_hour=?2 WHERE workspace=?1",
                    params![ws, v],
                )
                .map_err(db_error)?;
            }
            budget_in(tx, ws)
        })
    }

    pub fn assess(
        &self,
        ws: &str,
        amount: i64,
        balance: Option<i64>,
        now: i64,
    ) -> Result<Assessment, String> {
        self.with(|c| assess_in(c, ws, amount, balance, now))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn quote(
        &self,
        ws: &str,
        node: &str,
        fingerprint: &str,
        credits: i64,
        balance: Option<i64>,
        account: Option<&str>,
        now: i64,
    ) -> Result<Quote, String> {
        if credits < 0 {
            return Err("PROVIDER_COST_UNKNOWN".into());
        }
        let q = Quote {
            id: uuid::Uuid::new_v4().to_string(),
            workspace: ws.to_string(),
            node: node.to_string(),
            fingerprint: fingerprint.to_string(),
            credits,
            balance,
            expires_ms: now + QUOTE_MS,
            used: false,
        };
        self.tx(|tx| {
            tx.execute(
                "DELETE FROM canvas_media_quotes WHERE used=0 AND expires_ms<?1",
                [now - QUOTE_MS],
            )
            .map_err(db_error)?;
            tx.execute(
                "INSERT INTO canvas_media_quotes(id,workspace,node,fingerprint,credits,balance,account,expires_ms,used,created_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,0,?9)",
                params![q.id, ws, node, fingerprint, credits, balance, account, q.expires_ms, now],
            )
            .map_err(db_error)?;
            Ok(())
        })?;
        Ok(q)
    }

    pub fn reserve(&self, r: Reserve) -> Result<(Intent, bool), String> {
        if !super::schema::identifier(r.key) {
            return Err("PROVIDER_INVALID_IDEMPOTENCY_KEY".into());
        }
        self.tx(|tx| {
            let synchronous: i64 = tx
                .query_row("PRAGMA synchronous", [], |x| x.get(0))
                .map_err(db_error)?;
            if synchronous < 2 {
                return Err("PROVIDER_DURABILITY_REQUIRED".into());
            }
            let previous: Option<Intent> = tx
                .query_row(
                    &format!("SELECT {INTENT_COLS} FROM canvas_media_intents WHERE workspace=?1 AND node=?2 AND intent_key=?3"),
                    params![r.workspace, r.node, r.key],
                    intent_row,
                )
                .optional()
                .map_err(db_error)?;
            if let Some(p) = previous {
                if p.fingerprint != r.fingerprint {
                    return Err("PROVIDER_IDEMPOTENCY_CONFLICT: this key was used for a different request".into());
                }
                return Ok((p, false));
            }
            let amount: Option<i64> = tx
                .query_row(
                    "SELECT credits FROM canvas_media_quotes WHERE id=?1 AND workspace=?2 AND node=?3 AND fingerprint=?4 AND expires_ms>=?5 AND used=0",
                    params![r.quote, r.workspace, r.node, r.fingerprint, r.now],
                    |x| x.get(0),
                )
                .optional()
                .map_err(db_error)?;
            let amount = amount.ok_or(
                "PROVIDER_QUOTE_INVALID: the quote expired, was used, or the prompt, params or references changed; quote again",
            )?;
            let a = assess_in(tx, r.workspace, amount, Some(r.balance), r.now)?;
            if let Some(reason) = a.reason {
                return Err(reason);
            }
            if r.actor == Actor::Agent && a.needs_approval {
                return Err("PROVIDER_APPROVAL_REQUIRED: above the agent's automatic limit; the person approves it on the node".into());
            }
            ensure_budget(tx, r.workspace)?;
            let changed = tx
                .execute(
                    "UPDATE canvas_media_budgets SET reserved=reserved+?2 WHERE workspace=?1 AND ceiling-reserved-spent>=?2",
                    params![r.workspace, amount],
                )
                .map_err(db_error)?;
            if changed != 1 {
                return Err("PROVIDER_BUDGET_EXCEEDED".into());
            }
            let id = uuid::Uuid::new_v4().to_string();
            tx.execute(
                "INSERT INTO canvas_media_intents(id,workspace,node,intent_key,fingerprint,quote_id,reserved,state,actor,model,prompt,created_ms,updated_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,'unknown',?8,?9,?10,?11,?11)",
                params![id, r.workspace, r.node, r.key, r.fingerprint, r.quote, amount, r.actor.as_str(), r.model, r.prompt, r.now],
            )
            .map_err(db_error)?;
            tx.execute(
                "UPDATE canvas_media_quotes SET used=1 WHERE id=?1",
                [r.quote],
            )
            .map_err(db_error)?;
            let i = tx
                .query_row(
                    &format!("SELECT {INTENT_COLS} FROM canvas_media_intents WHERE id=?1"),
                    [&id],
                    intent_row,
                )
                .map_err(db_error)?;
            Ok((i, true))
        })
    }

    pub fn submitted(&self, id: &str, provider_id: &str, now: i64) -> Result<Intent, String> {
        if !super::schema::identifier(provider_id) {
            return Err("PROVIDER_INVALID_ID".into());
        }
        self.tx(|tx| {
            tx.execute(
                "UPDATE canvas_media_intents SET provider_id=?2, state='submitted', updated_ms=?3 WHERE id=?1 AND state='unknown' AND provider_id IS NULL",
                params![id, provider_id, now],
            )
            .map_err(db_error)?;
            intent_in(tx, id)
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn settle(
        &self,
        id: &str,
        completed: bool,
        actual: Option<i64>,
        actual_ref: Option<&str>,
        version: Option<i64>,
        error: Option<&str>,
        now: i64,
    ) -> Result<Intent, String> {
        self.tx(|tx| {
            let i = intent_in(tx, id)?;
            if !matches!(i.state.as_str(), "unknown" | "submitted") {
                return Ok(i);
            }
            let spent = if completed {
                actual.unwrap_or(i.reserved).max(0)
            } else {
                0
            };
            tx.execute(
                "UPDATE canvas_media_budgets SET reserved=MAX(reserved-?2,0), spent=spent+?3 WHERE workspace=?1",
                params![i.workspace, i.reserved, spent],
            )
            .map_err(db_error)?;
            tx.execute(
                "UPDATE canvas_media_intents SET state=?2, actual=?3, actual_ref=?4, version=?5, error=?6, updated_ms=?7 WHERE id=?1",
                params![
                    id,
                    if completed { "completed" } else { "failed" },
                    if completed { Some(spent) } else { Some(0) },
                    actual_ref,
                    version,
                    error,
                    now
                ],
            )
            .map_err(db_error)?;
            intent_in(tx, id)
        })
    }

    pub fn release(&self, id: &str, now: i64) -> Result<Intent, String> {
        self.tx(|tx| {
            let i = intent_in(tx, id)?;
            if i.state != "unknown" {
                return Err(format!(
                    "PROVIDER_RELEASE_DENIED: only an uncertain generation can be released (this one is {})",
                    i.state
                ));
            }
            tx.execute(
                "UPDATE canvas_media_budgets SET reserved=MAX(reserved-?2,0) WHERE workspace=?1",
                params![i.workspace, i.reserved],
            )
            .map_err(db_error)?;
            tx.execute(
                "UPDATE canvas_media_intents SET state='released', updated_ms=?2 WHERE id=?1",
                params![id, now],
            )
            .map_err(db_error)?;
            intent_in(tx, id)
        })
    }

    pub fn intent(&self, id: &str) -> Result<Intent, String> {
        self.with(|c| intent_in(c, id))
    }

    pub fn latest(&self, ws: &str, node: &str) -> Result<Option<Intent>, String> {
        self.with(|c| {
            c.query_row(
                &format!("SELECT {INTENT_COLS} FROM canvas_media_intents WHERE workspace=?1 AND node=?2 ORDER BY created_ms DESC, rowid DESC LIMIT 1"),
                params![ws, node],
                intent_row,
            )
            .optional()
            .map_err(db_error)
        })
    }

    pub fn open_intents(&self) -> Result<Vec<Intent>, String> {
        self.with(|c| {
            let mut st = c
                .prepare(&format!("SELECT {INTENT_COLS} FROM canvas_media_intents WHERE state IN ('unknown','submitted') ORDER BY created_ms"))
                .map_err(db_error)?;
            let rows = st.query_map([], intent_row).map_err(db_error)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(db_error)
        })
    }

    pub fn provider_claimed(&self, provider_id: &str) -> Result<bool, String> {
        self.with(|c| {
            c.query_row(
                "SELECT EXISTS(SELECT 1 FROM canvas_media_intents WHERE provider_id=?1)",
                [provider_id],
                |r| r.get(0),
            )
            .map_err(db_error)
        })
    }

    pub fn spend_claimed(&self, actual_ref: &str) -> Result<bool, String> {
        self.with(|c| {
            c.query_row(
                "SELECT EXISTS(SELECT 1 FROM canvas_media_intents WHERE actual_ref=?1)",
                [actual_ref],
                |r| r.get(0),
            )
            .map_err(db_error)
        })
    }

    pub fn upload_get(
        &self,
        hash: &str,
        account: &str,
        now: i64,
    ) -> Result<Option<String>, String> {
        self.with(|c| {
            c.query_row(
                "SELECT upload_id FROM canvas_media_uploads WHERE hash=?1 AND account=?2 AND created_ms>?3",
                params![hash, account, now - UPLOAD_TTL_MS],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_error)
        })
    }

    pub fn upload_put(
        &self,
        hash: &str,
        account: &str,
        upload: &str,
        now: i64,
    ) -> Result<(), String> {
        self.with(|c| {
            c.execute(
                "INSERT INTO canvas_media_uploads(hash,account,upload_id,created_ms) VALUES(?1,?2,?3,?4) ON CONFLICT(hash,account) DO UPDATE SET upload_id=excluded.upload_id, created_ms=excluded.created_ms",
                params![hash, account, upload, now],
            )
            .map_err(db_error)?;
            Ok(())
        })
    }
}

fn intent_in(c: &Connection, id: &str) -> Result<Intent, String> {
    c.query_row(
        &format!("SELECT {INTENT_COLS} FROM canvas_media_intents WHERE id=?1"),
        [id],
        intent_row,
    )
    .optional()
    .map_err(db_error)?
    .ok_or_else(|| "PROVIDER_INTENT_NOT_FOUND".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const WS: &str = "w1";
    const NODE: &str = "n1";
    const FP: &str = "fp";

    fn ledger() -> (Ledger, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        (Ledger::open(&dir.path().join("media.sqlite")).unwrap(), dir)
    }

    fn reserve<'a>(
        l: &Ledger,
        key: &'a str,
        quote: &'a str,
        actor: Actor,
        now: i64,
    ) -> Result<(Intent, bool), String> {
        l.reserve(Reserve {
            workspace: WS,
            node: NODE,
            key,
            quote,
            fingerprint: FP,
            actor,
            model: "m",
            prompt: "p",
            balance: 1_000_000_000,
            now,
        })
    }

    #[test]
    fn budget_starts_at_zero_and_blocks() {
        let (l, _d) = ledger();
        let b = l.budget(WS).unwrap();
        assert_eq!(
            (b.ceiling, b.auto_per_gen, b.auto_per_hour),
            (0, 10_000_000, 3)
        );
        let q = l
            .quote(WS, NODE, FP, 1_500_000, Some(10_000_000), None, 0)
            .unwrap();
        let e = reserve(&l, "k1", &q.id, Actor::Person, 1).unwrap_err();
        assert!(e.starts_with("PROVIDER_BUDGET_NOT_SET"), "{e}");
    }

    #[test]
    fn quote_is_bound_to_fingerprint_and_ttl() {
        let (l, _d) = ledger();
        l.set_budget(WS, Some(100_000_000), None, None).unwrap();
        let q = l
            .quote(WS, NODE, "other", 1_000_000, None, None, 0)
            .unwrap();
        assert!(reserve(&l, "k1", &q.id, Actor::Person, 1)
            .unwrap_err()
            .starts_with("PROVIDER_QUOTE_INVALID"));
        let q = l.quote(WS, NODE, FP, 1_000_000, None, None, 0).unwrap();
        assert!(reserve(&l, "k1", &q.id, Actor::Person, QUOTE_MS + 1)
            .unwrap_err()
            .starts_with("PROVIDER_QUOTE_INVALID"));
        let (i, go) = reserve(&l, "k1", &q.id, Actor::Person, 10).unwrap();
        assert!(go);
        assert_eq!(i.state, "unknown");
        assert_eq!(i.reserved, 1_000_000);
        assert!(reserve(&l, "k2", &q.id, Actor::Person, 11)
            .unwrap_err()
            .starts_with("PROVIDER_QUOTE_INVALID"));
        assert_eq!(l.budget(WS).unwrap().reserved, 1_000_000);
    }

    #[test]
    fn idempotency_key_returns_the_same_intent() {
        let (l, _d) = ledger();
        l.set_budget(WS, Some(100_000_000), None, None).unwrap();
        let q = l.quote(WS, NODE, FP, 2_000_000, None, None, 0).unwrap();
        let (a, go) = reserve(&l, "key-1", &q.id, Actor::Person, 1).unwrap();
        assert!(go);
        let (b, again) = reserve(&l, "key-1", &q.id, Actor::Person, 2).unwrap();
        assert!(!again);
        assert_eq!(a.id, b.id);
        assert_eq!(l.budget(WS).unwrap().reserved, 2_000_000);
        let conflict = l
            .reserve(Reserve {
                workspace: WS,
                node: NODE,
                key: "key-1",
                quote: &q.id,
                fingerprint: "changed",
                actor: Actor::Person,
                model: "m",
                prompt: "p",
                balance: 1_000_000_000,
                now: 3,
            })
            .unwrap_err();
        assert!(conflict.starts_with("PROVIDER_IDEMPOTENCY_CONFLICT"));
        assert!(reserve(&l, "bad key", &q.id, Actor::Person, 1).is_err());
    }

    #[test]
    fn reservation_fits_ceiling_and_balance() {
        let (l, _d) = ledger();
        l.set_budget(WS, Some(3_000_000), None, None).unwrap();
        let q1 = l.quote(WS, NODE, FP, 2_000_000, None, None, 0).unwrap();
        reserve(&l, "a", &q1.id, Actor::Person, 1).unwrap();
        let q2 = l.quote(WS, NODE, FP, 2_000_000, None, None, 0).unwrap();
        assert!(reserve(&l, "b", &q2.id, Actor::Person, 1)
            .unwrap_err()
            .starts_with("PROVIDER_BUDGET_EXCEEDED"));
        l.set_budget(WS, Some(100_000_000), None, None).unwrap();
        let low = l
            .reserve(Reserve {
                workspace: WS,
                node: NODE,
                key: "c",
                quote: &q2.id,
                fingerprint: FP,
                actor: Actor::Person,
                model: "m",
                prompt: "p",
                balance: 1_000_000,
                now: 2,
            })
            .unwrap_err();
        assert!(low.starts_with("PROVIDER_BALANCE_LOW"));
    }

    #[test]
    fn agent_is_limited_per_generation_and_per_hour() {
        let (l, _d) = ledger();
        l.set_budget(WS, Some(1_000_000_000), None, None).unwrap();
        let big = l.quote(WS, NODE, FP, 12_000_000, None, None, 0).unwrap();
        assert!(reserve(&l, "big", &big.id, Actor::Agent, 1)
            .unwrap_err()
            .starts_with("PROVIDER_APPROVAL_REQUIRED"));
        reserve(&l, "big", &big.id, Actor::Person, 1).unwrap();
        for k in ["a1", "a2", "a3"] {
            let q = l.quote(WS, NODE, FP, 1_000_000, None, None, 10).unwrap();
            reserve(&l, k, &q.id, Actor::Agent, 10).unwrap();
        }
        let q = l.quote(WS, NODE, FP, 1_000_000, None, None, 20).unwrap();
        assert!(reserve(&l, "a4", &q.id, Actor::Agent, 20)
            .unwrap_err()
            .starts_with("PROVIDER_APPROVAL_REQUIRED"));
        let a = l.assess(WS, 1_000_000, None, 20).unwrap();
        assert!(a.needs_approval);
        let later = l
            .quote(WS, NODE, FP, 1_000_000, None, None, HOUR_MS + 11)
            .unwrap();
        reserve(&l, "a5", &later.id, Actor::Agent, HOUR_MS + 11).unwrap();
    }

    #[test]
    fn settle_moves_reserved_to_spent() {
        let (l, _d) = ledger();
        l.set_budget(WS, Some(10_000_000), None, None).unwrap();
        let q = l.quote(WS, NODE, FP, 2_000_000, None, None, 0).unwrap();
        let (i, _) = reserve(&l, "a", &q.id, Actor::Person, 1).unwrap();
        let i = l.submitted(&i.id, "job-1", 2).unwrap();
        assert_eq!(i.state, "submitted");
        assert!(l.provider_claimed("job-1").unwrap());
        let i = l
            .settle(
                &i.id,
                true,
                Some(1_500_000),
                Some("spend-1"),
                Some(1),
                None,
                3,
            )
            .unwrap();
        assert_eq!(i.state, "completed");
        assert!(l.spend_claimed("spend-1").unwrap());
        let b = l.budget(WS).unwrap();
        assert_eq!((b.reserved, b.spent, b.left()), (0, 1_500_000, 8_500_000));
        let again = l.settle(&i.id, true, Some(9), None, None, None, 4).unwrap();
        assert_eq!(again.actual, Some(1_500_000));
        let q = l.quote(WS, NODE, FP, 2_000_000, None, None, 5).unwrap();
        let (f, _) = reserve(&l, "b", &q.id, Actor::Person, 5).unwrap();
        l.settle(&f.id, false, None, None, None, Some("nsfw"), 6)
            .unwrap();
        let b = l.budget(WS).unwrap();
        assert_eq!((b.reserved, b.spent), (0, 1_500_000));
    }

    #[test]
    fn only_uncertain_intents_are_released() {
        let (l, _d) = ledger();
        l.set_budget(WS, Some(10_000_000), None, None).unwrap();
        let q = l.quote(WS, NODE, FP, 2_000_000, None, None, 0).unwrap();
        let (i, _) = reserve(&l, "a", &q.id, Actor::Person, 1).unwrap();
        assert_eq!(l.open_intents().unwrap().len(), 1);
        let r = l.release(&i.id, 2).unwrap();
        assert_eq!(r.state, "released");
        assert_eq!(l.budget(WS).unwrap().reserved, 0);
        assert!(l.open_intents().unwrap().is_empty());
        let q = l.quote(WS, NODE, FP, 2_000_000, None, None, 3).unwrap();
        let (j, _) = reserve(&l, "b", &q.id, Actor::Person, 3).unwrap();
        l.submitted(&j.id, "job-2", 4).unwrap();
        assert!(l.release(&j.id, 5).is_err());
        assert_eq!(l.latest(WS, NODE).unwrap().unwrap().id, j.id);
    }

    #[test]
    fn upload_cache_expires() {
        let (l, _d) = ledger();
        l.upload_put("h", "acct", "u1", 0).unwrap();
        assert_eq!(
            l.upload_get("h", "acct", 10).unwrap().as_deref(),
            Some("u1")
        );
        assert_eq!(l.upload_get("h", "other", 10).unwrap(), None);
        assert_eq!(l.upload_get("h", "acct", UPLOAD_TTL_MS + 1).unwrap(), None);
    }
}
