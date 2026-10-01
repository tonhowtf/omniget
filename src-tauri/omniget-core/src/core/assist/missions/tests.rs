use serde_json::json;

use super::*;
use crate::core::assist::db::AssistDb;

fn artifact_criterion(id: &str, path: &str, contains: &str) -> Criterion {
    Criterion {
        id: id.into(),
        version: 1,
        kind: CriterionKind::Artifact,
        severity: Severity::Required,
        title: format!("{path} contains {contains}"),
        spec: json!({ "path": path, "contains": [contains] }),
        origin: Origin::User,
        acceptance: Acceptance::Auto,
    }
}

#[test]
fn a28_the_missions_migration_applies_over_an_old_file_and_keeps_its_data() {
    use crate::core::assist::db::Migration;
    let dir = std::env::temp_dir().join(format!("omniget-mig-{}", crate::core::assist::new_id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("assist.db");
    let old: Vec<Migration> = crate::core::assist::db::all_migrations()
        .into_iter()
        .filter(|m| m.module != "missions")
        .collect();
    {
        let db = AssistDb::open_with(&path, &old).unwrap();
        db.with(|c| c.execute("INSERT INTO bots_skill_bindings(bot_id, skill, hash, created_at, updated_at) VALUES ('b','s','h',1,1)", [])).unwrap();
    }
    let mut broken = old.clone();
    broken.push(Migration {
        module: "missions",
        version: 1,
        sql: "CREATE TABLE missions_missions(id TEXT); INSERT INTO nowhere VALUES (1);",
    });
    assert!(AssistDb::open_with(&path, &broken).is_err());
    let db = AssistDb::open(&path).unwrap();
    let n: i64 = db
        .with(|c| c.query_row("SELECT count(*) FROM bots_skill_bindings", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(n, 1);
    let backups = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().contains(".bak-"))
        .count();
    assert!(backups >= 1);
    let d = create(
        &db,
        NewMission {
            objective: "x".into(),
            criteria: vec![artifact_criterion("c1", "a", "b")],
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(d.mission.state, MissionState::Draft);
}

mod reading_mission {}
