use std::collections::BTreeMap;
use std::sync::Arc;

use super::*;
use crate::core::assist::authority::{self, Ceiling};
use crate::core::llm::roster_store::RosterStore;

struct Fx {
    db: Arc<AssistDb>,
    roster: Arc<RosterStore>,
    _dir: tempfile::TempDir,
    grant_a: String,
}

const TOOLS: &[&str] = &["fs_read", "fs_list", "fs_write", "shell_exec"];

fn fixture() -> Fx {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    let db = Arc::new(AssistDb::open_in_memory().unwrap());
    let roster = Arc::new(RosterStore::at(dir.path().join("roster.json")));
    let mut exec = roster.get("builder").unwrap();
    exec.id = "exec-a".into();
    roster.create(exec.clone()).unwrap();
    let mut other = exec.clone();
    other.id = "exec-other".into();
    roster.create(other).unwrap();
    let rev = authority::agent_revision(&roster.get("exec-a").unwrap()).unwrap();
    let grant = |principal: &str| {
        authority::grant(
            &db,
            Ceiling {
                id: String::new(),
                principal: principal.into(),
                workspace_id: "ws".into(),
                workspace: ws.to_string_lossy().into_owned(),
                workspace_identity: None,
                bots: vec!["exec-a".into()],
                bot_revisions: BTreeMap::from([("exec-a".into(), rev.clone())]),
                tools: TOOLS.iter().map(|s| s.to_string()).collect(),
                max_tokens: 50_000,
                max_usd: None,
            },
        )
        .unwrap()
    };
    let grant_a = grant("A");
    grant("B");
    Fx {
        db,
        roster,
        _dir: dir,
        grant_a,
    }
}

fn bot_req(grant: &str, key: &str, tools: &[&str]) -> BotRequest {
    BotRequest {
        grant_id: grant.into(),
        source_executor_id: "exec-a".into(),
        name: format!("Derived {key}"),
        purpose: "limited helper".into(),
        instructions: "Write the files you are asked for.".into(),
        tools: tools.iter().map(|s| s.to_string()).collect(),
        idempotency_key: key.into(),
    }
}

fn ceiling(fx: &Fx, principal: &str, grant: &str) -> Ceiling {
    authority::get(&fx.db, grant, principal).unwrap()
}

fn count(db: &AssistDb, sql: &str) -> i64 {
    db.with(|c| c.query_row(sql, [], |r| r.get(0))).unwrap()
}

struct CrashAfterWrite<'a>(&'a RosterStore);
impl BotRoster for CrashAfterWrite<'_> {
    fn get(&self, id: &str) -> Option<AgentDef> {
        self.0.get(id)
    }
    fn apply_planned(
        &self,
        a: AgentDef,
        b: Option<AgentDef>,
        s: AgentDef,
    ) -> Result<AgentDef, String> {
        self.0.apply_planned(a, b, s).map_err(|e| e.to_string())?;
        Err("simulated crash".into())
    }
}

#[test]
fn recovery_uses_exact_hashes_and_never_overwrites_local_edits() {
    let fx = fixture();
    let req = bot_req(&fx.grant_a, "rec", &["fs_read"]);
    assert_eq!(
        prepare_bot_with(&fx.db, &CrashAfterWrite(&fx.roster), "A", &req).unwrap_err(),
        "EXTERNAL_ROSTER_WRITE_FAILED"
    );
    assert_eq!(
        count(
            &fx.db,
            "SELECT count(*) FROM external_config_intents WHERE state='pending'"
        ),
        1
    );
    assert_eq!(
        count(&fx.db, "SELECT count(*) FROM external_derived_bots"),
        0
    );
    let raw = ceiling(&fx, "A", &fx.grant_a);
    let planned: String = fx
        .db
        .with(|c| {
            c.query_row("SELECT target FROM external_config_intents", [], |r| {
                r.get(0)
            })
        })
        .unwrap();
    assert!(authority::for_bot_with(&fx.db, &raw, &planned, Some(fx.roster.as_ref())).is_err());
    let v = prepare_bot_with(&fx.db, fx.roster.as_ref(), "A", &req).unwrap();
    assert_eq!(v.bot_id, planned);
    assert_eq!(
        fx.roster.list().iter().filter(|x| x.id == planned).count(),
        1
    );

    let req2 = bot_req(&fx.grant_a, "rec2", &["fs_read"]);
    let _ = prepare_bot_with(&fx.db, &CrashAfterWrite(&fx.roster), "A", &req2).unwrap_err();
    let planned2: String = fx
        .db
        .with(|c| {
            c.query_row(
                "SELECT target FROM external_config_intents WHERE key='rec2'",
                [],
                |r| r.get(0),
            )
        })
        .unwrap();
    let mut local = fx.roster.get(&planned2).unwrap();
    local.name = "User rename".into();
    fx.roster.update(local).unwrap();
    assert_eq!(
        prepare_bot_with(&fx.db, fx.roster.as_ref(), "A", &req2).unwrap_err(),
        "EXTERNAL_BOT_CHANGED_LOCALLY"
    );
    assert_eq!(fx.roster.get(&planned2).unwrap().name, "User rename");
    assert_eq!(
        prepare_bot_with(&fx.db, fx.roster.as_ref(), "A", &req2).unwrap_err(),
        "EXTERNAL_BOT_CHANGED_LOCALLY"
    );
    assert!(profile::get(&fx.db, &planned2).unwrap().is_none());
}
