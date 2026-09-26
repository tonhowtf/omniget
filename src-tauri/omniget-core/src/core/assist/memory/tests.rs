//! Acceptance tests A13–A16, B09 and the deterministic longitudinal
//! sequence of 04-ACEITE (the parts that belong to memory). Every test uses
//! its own database; none injects the expected answer into a prompt.

use serde_json::json;

use super::super::ctx::{self, AssistCtx, Scope};
use super::super::db::{tests_support::temp_db, AssistDb};
use super::*;
use crate::core::llm::agent::{
    AgentDef, AgentRole, Budget, GrantMode, ModelPolicy, RuntimeKind, ToolGrant, ToolSource,
};
use crate::core::llm::types::{ModelRef, ProviderId};

const T0: i64 = 1_790_000_000_000;
const DAY: i64 = 24 * 3_600_000;

fn bot(conv: &str) -> AssistCtx {
    ctx::direct("reader", Some(conv))
}

fn room_ctx(room: &str, bot: &str) -> AssistCtx {
    let s = vec![Scope::Room {
        conversation: room.into(),
    }];
    AssistCtx {
        principal: ctx::LOCAL_USER.into(),
        bot_id: Some(bot.into()),
        conversation_id: Some(format!("{room}~{bot}")),
        run_id: None,
        readable: s.clone(),
        writable: s,
    }
}

fn tool(
    db: &AssistDb,
    c: &AssistCtx,
    name: &str,
    input: serde_json::Value,
    now: i64,
) -> serde_json::Value {
    call_tool(db, c, name, input, now).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn ids(v: &serde_json::Value) -> Vec<String> {
    v["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap().to_string())
        .collect()
}

fn contents(recs: &[MemoryRecord]) -> String {
    recs.iter()
        .map(|r| r.content.clone())
        .collect::<Vec<_>>()
        .join(" | ")
}

fn fts_hits(db: &AssistDb, term: &str) -> i64 {
    db.with(|c| {
        c.query_row(
            "SELECT count(*) FROM memory_fts WHERE memory_fts MATCH ?1",
            [format!("\"{term}\"")],
            |r| r.get(0),
        )
    })
    .unwrap()
}

fn agent(model: &str, with_memory: bool) -> AgentDef {
    AgentDef {
        id: "reader".into(),
        name: "Reading companion".into(),
        role: AgentRole::Worker,
        system_prompt: "companion".into(),
        model: ModelPolicy::Fixed {
            model: ModelRef {
                provider: ProviderId::new("fake"),
                model: model.into(),
            },
        },
        tools: if with_memory {
            vec![ToolGrant {
                source: ToolSource::Internal {
                    name: "memory_recall".into(),
                },
                mode: GrantMode::Auto,
            }]
        } else {
            vec![]
        },
        skills: vec![],
        budget: Budget::default(),
        runtime: RuntimeKind::Native,
        skin: None,
    }
}

#[test]
fn tool_names_match_the_specs() {
    let names: Vec<String> = toolset().specs().into_iter().map(|s| s.name).collect();
    assert_eq!(
        names,
        TOOL_NAMES.iter().map(|s| s.to_string()).collect::<Vec<_>>()
    );
}

#[test]
fn fts_query_is_words_only() {
    assert_eq!(
        fts_query_for_test("\" OR * NEAR( x"),
        Some("\"near\"".to_string())
    );
    assert_eq!(fts_query_for_test("a é o"), None);
    let q = fts_query_for_test("relação familiar").unwrap();
    assert!(q.contains("\"relac\"*") && q.contains("\"famili\"*"), "{q}");
}

fn fts_query_for_test(s: &str) -> Option<String> {
    super::search::fts_query(s)
}

#[test]
fn same_origin_does_not_duplicate() {
    let db = AssistDb::open_in_memory().unwrap();
    let c = bot("conv-1");
    let input = json!({"content":"Watched Film A","kind":"observation","source_message_id":"m1"});
    let a = tool(&db, &c, "memory_remember", input.clone(), T0);
    let b = tool(&db, &c, "memory_remember", input, T0 + 5);
    assert_eq!(a["id"], b["id"]);
    assert_eq!(b["created"], json!(false));
    let n: i64 = db
        .with(|c| c.query_row("SELECT count(*) FROM memory_records", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(n, 1);
}

#[test]
fn a_guess_stays_a_candidate_and_never_displaces_a_stated_preference() {
    let db = AssistDb::open_in_memory().unwrap();
    let c = bot("conv-1");
    // Declared without the person saying so → candidate.
    let w = tool(
        &db,
        &c,
        "memory_remember",
        json!({"content":"Likes slow films","kind":"declared","subject":"pace"}),
        T0,
    );
    assert_eq!(w["kind"], json!("inference"));
    assert_eq!(w["note"], json!("downgraded_to_candidate"));
    // The person states it; the guess is superseded by the statement.
    let d = tool(
        &db,
        &c,
        "memory_remember",
        json!({"content":"Prefers fast-paced films","kind":"declared","subject":"pace","user_said_explicitly":true}),
        T0 + 1,
    );
    assert_eq!(d["kind"], json!("declared"));
    // A later guess on the same subject does not replace the statement.
    let g = tool(
        &db,
        &c,
        "memory_remember",
        json!({"content":"Maybe likes slow films after all","kind":"inference","subject":"pace"}),
        T0 + 2,
    );
    assert_eq!(g["note"], json!("declared_exists"));
    assert_eq!(g["id"], d["id"]);
    let active = list(&db, &AssistCtx::user(), None, false, T0 + 3).unwrap();
    let pace: Vec<_> = active
        .iter()
        .filter(|r| r.subject.as_deref() == Some("pace"))
        .collect();
    assert_eq!(pace.len(), 1, "one active preference per subject");
    // The UI confirm turns a guess into a statement, as a new version.
    let guess = remember(
        &db,
        &AssistCtx::user(),
        NewMemory::new(
            Scope::User,
            Category::Inference,
            "Enjoys family stories",
            Source::user(T0),
        )
        .evidence("liked Film A's family", 0.7),
        T0,
    )
    .unwrap();
    assert!(
        confirm(&db, &c, &guess.record.id, T0).is_err(),
        "a bot cannot confirm"
    );
    let ok = confirm(&db, &AssistCtx::user(), &guess.record.id, T0 + 4).unwrap();
    assert_eq!(ok.category, Category::Declared);
    assert_eq!(ok.supersedes.as_deref(), Some(guess.record.id.as_str()));
    assert_eq!(history(&db, &AssistCtx::user(), &ok.id).unwrap().len(), 2);
}

/// A13: three sessions, the file closed and reopened, the fact recalled from
/// storage with its origin.
#[test]
fn a13_memory_survives_three_sessions_and_a_restart() {
    let (db, path) = temp_db();
    tool(
        &db,
        &bot("conv-s1"),
        "memory_remember",
        json!({"content":"Reading Dom Casmurro, Penguin edition, at chapter 3","kind":"declared","subject":"book.progress","user_said_explicitly":true,"source_message_id":"s1-m4"}),
        T0,
    );
    drop(db);
    let db = std::sync::Arc::new(AssistDb::open(&path).unwrap());
    tool(
        &db,
        &bot("conv-s2"),
        "memory_remember",
        json!({"content":"Does not want spoilers","kind":"declared","subject":"spoilers","user_said_explicitly":true}),
        T0 + DAY,
    );
    drop(db);
    let db = std::sync::Arc::new(AssistDb::open(&path).unwrap());
    tool(
        &db,
        &bot("conv-s3"),
        "memory_remember",
        json!({"content":"Watched Film A and liked its humor","kind":"observation","data":{"movie":"Film A"}}),
        T0 + 2 * DAY,
    );
    drop(db);

    // A new process, a new conversation.
    let db = AssistDb::open(&path).unwrap();
    let c = bot("conv-s4");
    let found = tool(
        &db,
        &c,
        "memory_recall",
        json!({"query":"which chapter of the book"}),
        T0 + 3 * DAY,
    );
    let first = &found["results"][0];
    assert!(first["content"].as_str().unwrap().contains("chapter 3"));
    assert!(
        first["source"].as_str().unwrap().contains("conv-s1"),
        "{first}"
    );
    let rec = get(&db, &c, first["id"].as_str().unwrap()).unwrap();
    assert_eq!(rec.source.conversation.as_deref(), Some("conv-s1"));
    assert_eq!(rec.source.id.as_deref(), Some("s1-m4"));
    // The turn text comes from the store; the input does not carry the answer.
    let input = "where did I stop?";
    let text = augment_text(&db, &c, input, T0 + 3 * DAY).unwrap();
    assert!(!input.contains("chapter 3"));
    assert!(
        text.contains("chapter 3")
            && text.contains("Does not want spoilers")
            && text.contains("Film A"),
        "{text}"
    );
    assert!(text.contains("(source: "), "{text}");
    assert!(text.len() <= search::AUGMENT_BUDGET);
}

/// A14: a correction replaces the earlier record, also after a reindex.
#[test]
fn a14_a_correction_replaces_the_earlier_record_after_reindex_too() {
    let (db, path) = temp_db();
    let c = bot("conv-1");
    let old = tool(
        &db,
        &c,
        "memory_remember",
        json!({"content":"Did not like the ending of Film A","kind":"declared","subject":"film-a.ending","user_said_explicitly":true}),
        T0,
    );
    let new = tool(
        &db,
        &c,
        "memory_correct",
        json!({"id": old["id"], "content":"Actually liked the ending of Film A","user_said_explicitly":true}),
        T0 + 10,
    );
    assert_eq!(new["kind"], json!("declared"));
    let hits = recall(&db, &c, "ending Film A", 10, T0 + 20).unwrap();
    assert_eq!(hits.len(), 1, "{}", contents(&hits));
    assert!(hits[0].content.starts_with("Actually"));
    assert_eq!(
        get(&db, &c, old["id"].as_str().unwrap()).unwrap().status,
        Status::Superseded
    );
    // Correcting the old version again points to the current one.
    let err = call_tool(
        &db,
        &c,
        "memory_correct",
        json!({"id": old["id"], "content":"x","user_said_explicitly":true}),
        T0 + 21,
    )
    .unwrap_err();
    assert!(err.starts_with(ERR_MEMORY_INACTIVE) && err.contains(new["id"].as_str().unwrap()));
    reindex(&db).unwrap();
    drop(db);
    let db = AssistDb::open(&path).unwrap();
    let hits = recall(&db, &c, "did not like the ending", 10, T0 + 30).unwrap();
    assert!(
        hits.iter().all(|r| !r.content.starts_with("Did not")),
        "{}",
        contents(&hits)
    );
    let p = profile(&db, &c, T0 + 30).unwrap();
    assert!(
        p.text.contains("Actually liked") && !p.text.contains("Did not like"),
        "{}",
        p.text
    );
    let h = history(&db, &c, new["id"].as_str().unwrap()).unwrap();
    assert_eq!(h.len(), 2);
    assert_eq!(h[1].supersedes.as_deref(), old["id"].as_str());
}

/// A15: forgetting removes the record from recall, profile, index and
/// history; reindex, reopening and importing an older export do not bring it
/// back; tombstones hold no text.
#[test]
fn a15_forget_removes_everything_and_nothing_resurrects_it() {
    let (db, path) = temp_db();
    let c = bot("conv-1");
    let keep = tool(
        &db,
        &c,
        "memory_remember",
        json!({"content":"Liked the humor of Film A","kind":"observation"}),
        T0,
    );
    let gone = tool(
        &db,
        &c,
        "memory_remember",
        json!({"content":"Found the pacing of Zanzibarfilm dragging","kind":"observation","source_message_id":"m9"}),
        T0 + 1,
    );
    let v2 = tool(
        &db,
        &c,
        "memory_correct",
        json!({"id": gone["id"], "content":"Found the pacing of Zanzibarfilm dragging, mostly the middle"}),
        T0 + 2,
    );
    assert!(profile(&db, &c, T0 + 3)
        .unwrap()
        .text
        .contains("Zanzibarfilm"));
    let old_export = export_json(&db, &AssistCtx::user(), T0 + 3).unwrap();
    assert!(old_export.contains("Zanzibarfilm"));

    let out = tool(&db, &c, "memory_forget", json!({"id": v2["id"]}), T0 + 4);
    assert_eq!(out["forgotten"], json!(true));

    let check = |db: &AssistDb| {
        assert!(recall(db, &c, "Zanzibarfilm pacing", 10, T0 + 5)
            .unwrap()
            .iter()
            .all(|r| !r.content.contains("Zanzibarfilm")));
        assert!(!profile(db, &c, T0 + 5)
            .unwrap()
            .text
            .contains("Zanzibarfilm"));
        assert_eq!(fts_hits(db, "zanzibarfilm"), 0);
        let rows: i64 = db
            .with(|x| {
                x.query_row(
                    "SELECT count(*) FROM memory_records WHERE content LIKE '%Zanzibarfilm%'",
                    [],
                    |r| r.get(0),
                )
            })
            .unwrap();
        assert_eq!(rows, 0);
        let cached: i64 = db
            .with(|x| {
                x.query_row(
                    "SELECT count(*) FROM memory_profile_cache WHERE text LIKE '%Zanzibarfilm%'",
                    [],
                    |r| r.get(0),
                )
            })
            .unwrap();
        assert_eq!(cached, 0);
        for id in [&gone["id"], &v2["id"]] {
            assert_eq!(
                history(db, &c, id.as_str().unwrap()).unwrap_err(),
                not_found()
            );
        }
        assert!(
            get(db, &c, keep["id"].as_str().unwrap()).is_ok(),
            "other memories stay"
        );
    };
    check(&db);
    // Tombstones are hashes only.
    let tomb_text: Vec<String> = db
        .with(|x| {
            let mut s = x.prepare("SELECT hash || scope || kind FROM memory_tombstones")?;
            let r = s.query_map([], |r| r.get(0))?;
            r.collect()
        })
        .unwrap();
    assert!(tomb_text.len() >= 6);
    assert!(tomb_text
        .iter()
        .all(|t| !t.to_lowercase().contains("zanzibar")));

    // The bot re-deriving it from the same message is refused.
    let err = call_tool(&db, &c, "memory_remember", json!({"content":"Found the pacing of Zanzibarfilm dragging","kind":"observation","source_message_id":"m9"}), T0 + 6).unwrap_err();
    assert!(err.starts_with(ERR_MEMORY_FORGOTTEN), "{err}");

    reindex(&db).unwrap();
    check(&db);
    let rep = import_json(&db, &AssistCtx::user(), &old_export, false, T0 + 7).unwrap();
    assert_eq!(rep.skipped_forgotten, 2, "{rep:?}");
    check(&db);
    drop(db);
    let db = AssistDb::open(&path).unwrap();
    check(&db);
    reindex(&db).unwrap();
    check(&db);
    drop(db);
    // The text is not in the database file or its log either.
    for f in [path.clone(), path.with_extension("db-wal")] {
        if let Ok(bytes) = std::fs::read(&f) {
            let hay = String::from_utf8_lossy(&bytes).to_lowercase();
            assert!(
                !hay.contains("zanzibarfilm"),
                "{} still holds the text",
                f.display()
            );
        }
    }
    // The app's own backups are reachable for deletion.
    let db = AssistDb::open(&path).unwrap();
    db.backup_now().unwrap();
    assert_eq!(backup_files(&db).len(), 1);
    assert_eq!(delete_backups(&db).unwrap(), 1);
    assert!(backup_files(&db).is_empty());
}

/// A16: another room or bot gets no text, metadata, ids or counts, even
/// when asking by exact id and exact term.
#[test]
fn a16_other_rooms_and_bots_see_nothing_not_even_by_exact_id() {
    let db = AssistDb::open_in_memory().unwrap();
    let a = room_ctx("room-a", "curator");
    let secret = tool(
        &db,
        &a,
        "memory_remember",
        json!({"content":"Room A note: pineapple password","kind":"observation"}),
        T0,
    );
    let private = tool(
        &db,
        &bot("conv-1"),
        "memory_remember",
        json!({"content":"Private tangerine note","kind":"procedural"}),
        T0,
    );
    let sid = secret["id"].as_str().unwrap().to_string();
    let pid = private["id"].as_str().unwrap().to_string();

    let others = [
        room_ctx("room-b", "curator"),
        room_ctx("room-b", "reader"),
        ctx::direct("other-bot", Some("conv-9")),
    ];
    for o in &others {
        for term in ["pineapple", "tangerine", "password", ""] {
            let r = tool(
                &db,
                o,
                "memory_recall",
                json!({"query": if term.is_empty() { "zz" } else { term }}),
                T0 + 1,
            );
            let found = ids(&r);
            assert!(!found.contains(&sid) && !found.contains(&pid), "{r}");
            assert!(recall(&db, o, term, 50, T0 + 1)
                .unwrap()
                .iter()
                .all(|x| x.id != sid && x.id != pid));
        }
        let missing = get(&db, o, "no-such-id").unwrap_err();
        for id in [&sid, &pid] {
            assert_eq!(
                get(&db, o, id).unwrap_err(),
                missing,
                "same error as a missing id"
            );
            assert_eq!(history(&db, o, id).unwrap_err(), missing);
            for t in ["memory_forget", "memory_correct"] {
                let e = call_tool(
                    &db,
                    o,
                    t,
                    json!({"id": id, "content":"x", "retract": true}),
                    T0 + 1,
                )
                .unwrap_err();
                let e_missing = call_tool(
                    &db,
                    o,
                    t,
                    json!({"id": "no-such-id", "content":"x", "retract": true}),
                    T0 + 1,
                )
                .unwrap_err();
                assert_eq!(e, e_missing);
            }
        }
        let listed = list(&db, o, None, true, T0 + 1).unwrap();
        assert!(listed.iter().all(|x| x.id != sid && x.id != pid));
        assert!(list(
            &db,
            o,
            Some(&Scope::Room {
                conversation: "room-a".into()
            }),
            true,
            T0 + 1
        )
        .unwrap()
        .is_empty());
        let counts = scopes(&db, o, T0 + 1).unwrap();
        assert!(counts.iter().all(|s| s.scope
            != Scope::Room {
                conversation: "room-a".into()
            }
            && s.scope
                != Scope::Bot {
                    bot: "reader".into()
                }));
        let prof = tool(&db, o, "memory_profile", json!({}), T0 + 1).to_string();
        let exp = export_json(&db, o, T0 + 1).unwrap();
        for hay in [&prof, &exp] {
            assert!(
                !hay.contains("pineapple")
                    && !hay.contains("tangerine")
                    && !hay.contains(&sid)
                    && !hay.contains(&pid),
                "{hay}"
            );
        }
        assert!(augment_text(&db, o, "pineapple tangerine", T0 + 1)
            .map(|t| !t.contains("pineapple") && !t.contains("tangerine"))
            .unwrap_or(true));
    }
    // The owners still see their own.
    assert_eq!(get(&db, &a, &sid).unwrap().id, sid);
    assert_eq!(get(&db, &bot("conv-2"), &pid).unwrap().id, pid);
    // A room member cannot see the reader's private scope either.
    assert!(get(&db, &a, &pid).is_err());
}

/// B09: export → import in a new database keeps links, handles duplicates
/// and respects tombstones.
#[test]
fn b09_export_import_keeps_links_dedups_and_respects_forgetting() {
    let src = AssistDb::open_in_memory().unwrap();
    let u = AssistCtx::user();
    let c = bot("conv-1");
    let v1 = tool(
        &src,
        &c,
        "memory_remember",
        json!({"content":"Did not like the ending","kind":"declared","subject":"ending","user_said_explicitly":true}),
        T0,
    );
    let v2 = tool(
        &src,
        &c,
        "memory_correct",
        json!({"id": v1["id"], "content":"Liked the ending after all","user_said_explicitly":true}),
        T0 + 1,
    );
    let guess = tool(
        &src,
        &c,
        "memory_remember",
        json!({"content":"Enjoys family dramas","kind":"inference","evidence":"liked Film A's family","confidence":0.6}),
        T0 + 2,
    );
    tool(
        &src,
        &c,
        "memory_remember",
        json!({"content":"Something light today","kind":"temporary","valid_hours":2}),
        T0 + 3,
    );
    let doomed = tool(
        &src,
        &c,
        "memory_remember",
        json!({"content":"Quokkafilm reaction","kind":"observation"}),
        T0 + 4,
    );
    let before_forget = export_json(&src, &u, T0 + 5).unwrap();
    forget(&src, &u, doomed["id"].as_str().unwrap(), T0 + 6).unwrap();
    let file = export_json(&src, &u, T0 + 7).unwrap();
    assert!(!file.contains("Quokkafilm"));
    let md = export_markdown(&src, &u, T0 + 7).unwrap();
    assert!(md.contains("Liked the ending after all") && !md.contains("Did not like"));

    let dst = AssistDb::open_in_memory().unwrap();
    let rep = import_json(&dst, &u, &file, false, T0 + 8).unwrap();
    assert_eq!(rep.imported, 4, "{rep:?}");
    assert!(rep.rejected.is_empty(), "{rep:?}");
    assert_eq!(rep.tombstones_added, 3);
    let h = history(&dst, &u, v2["id"].as_str().unwrap()).unwrap();
    assert_eq!(h.len(), 2);
    assert_eq!(h[0].id, v1["id"].as_str().unwrap());
    assert_eq!(h[1].supersedes.as_deref(), v1["id"].as_str());
    assert_eq!(h[1].status, Status::Active);
    assert_eq!(
        get(&dst, &u, guess["id"].as_str().unwrap())
            .unwrap()
            .category,
        Category::Inference
    );
    assert_eq!(recall(&dst, &c, "ending", 5, T0 + 9).unwrap().len(), 1);

    // Importing twice changes nothing.
    let again = import_json(&dst, &u, &file, false, T0 + 9).unwrap();
    assert_eq!((again.imported, again.duplicates), (0, 4));

    // The older export still carries the forgotten record: skipped, unless
    // the person explicitly chooses to restore it.
    let old = import_json(&dst, &u, &before_forget, false, T0 + 10).unwrap();
    assert_eq!(old.skipped_forgotten, 1, "{old:?}");
    assert!(recall(&dst, &c, "Quokkafilm", 5, T0 + 10)
        .unwrap()
        .is_empty());
    let back = import_json(&dst, &u, &before_forget, true, T0 + 11).unwrap();
    assert_eq!(back.resurrected, 1);
    assert_eq!(recall(&dst, &c, "Quokkafilm", 5, T0 + 11).unwrap().len(), 1);

    // Validation.
    assert!(import_json(&dst, &u, "{\"hello\":1}", false, T0)
        .unwrap_err()
        .starts_with(ERR_MEMORY_IMPORT));
    let newer = before_forget.replacen("\"version\": 1", "\"version\": 99", 1);
    assert!(import_json(&dst, &u, &newer, false, T0)
        .unwrap_err()
        .contains("newer"));
    let mut broken: serde_json::Value = serde_json::from_str(&file).unwrap();
    let mut orphan = broken["records"][0].clone();
    orphan["id"] = json!("orphan-2");
    orphan["chain"] = json!("missing-root");
    orphan["supersedes"] = json!("missing-root");
    orphan["version"] = json!(2);
    orphan["content"] = json!("An orphan version");
    broken["records"] = json!([orphan]);
    let rep = import_json(
        &AssistDb::open_in_memory().unwrap(),
        &u,
        &broken.to_string(),
        false,
        T0,
    )
    .unwrap();
    assert_eq!(rep.imported, 0);
    assert_eq!(rep.rejected.len(), 1);
}

/// 04-ACEITE "Avaliação longitudinal de memória", steps 1–4, 7 and 8 as far
/// as memory goes.
#[test]
fn longitudinal_sequence() {
    let (db, path) = temp_db();
    // 1. Cold session: book, edition, progress, no spoilers, light today.
    let s1 = bot("conv-day1");
    tool(
        &db,
        &s1,
        "memory_remember",
        json!({"content":"Reading The Time and the Wind, Companhia das Letras edition, chapter 3","kind":"declared","subject":"book.progress","user_said_explicitly":true}),
        T0,
    );
    tool(
        &db,
        &s1,
        "memory_remember",
        json!({"content":"No spoilers of the book","kind":"declared","subject":"spoilers","user_said_explicitly":true}),
        T0,
    );
    let light = tool(
        &db,
        &s1,
        "memory_remember",
        json!({"content":"Wants a light round today","kind":"temporary","subject":"mood"}),
        T0,
    );
    assert_eq!(light["kind"], json!("temporary"));
    assert!(light["valid_until_ms"].as_i64().unwrap() <= T0 + DAY);
    let lasting = list(&db, &s1, None, false, T0).unwrap();
    assert!(
        lasting
            .iter()
            .filter(|r| r.category != Category::Temporary)
            .all(|r| !r.content.to_lowercase().contains("light")),
        "lightness is not a lasting preference"
    );

    // 2. Round and reactions: specific to the movie, not the genre.
    let s2 = bot("conv-day4");
    tool(
        &db,
        &s2,
        "memory_remember",
        json!({"content":"Watched Film A: liked the humor and the family relationship","kind":"observation","data":{"movie":"Film A","liked":["humor","family"]}}),
        T0 + 3 * DAY,
    );
    let pace_obs = tool(
        &db,
        &s2,
        "memory_remember",
        json!({"content":"Watched Film B: did not like its slow pace","kind":"observation","data":{"movie":"Film B","disliked":["pace"]}}),
        T0 + 3 * DAY,
    );
    let guess = tool(
        &db,
        &s2,
        "memory_remember",
        json!({"content":"Might dislike slow-paced films","kind":"declared","subject":"pace","evidence":"said Film B was slow"}),
        T0 + 3 * DAY,
    );
    assert_eq!(
        guess["kind"],
        json!("inference"),
        "not stated by the person"
    );
    let declared = list(&db, &s2, None, false, T0 + 3 * DAY).unwrap();
    assert!(declared
        .iter()
        .filter(|r| r.category == Category::Declared)
        .all(|r| !r.content.to_lowercase().contains("slow")));
    let prof = profile(&db, &s2, T0 + 3 * DAY).unwrap().text;
    let guesses_at = prof.find("Unconfirmed guesses").unwrap();
    assert!(prof[guesses_at..].contains("slow-paced"), "{prof}");

    // 3. New session, a later day: progress and reaction come back, the
    // light-round wish expired.
    drop(db);
    let db = AssistDb::open(&path).unwrap();
    let s3 = bot("conv-day10");
    let text = augment_text(&db, &s3, "two more films please", T0 + 9 * DAY).unwrap();
    assert!(
        text.contains("chapter 3") && text.contains("humor and the family"),
        "{text}"
    );
    assert!(!text.contains("light round"), "{text}");
    assert!(text.contains("conversation conv-day4"), "{text}");

    // 4. "It was not the pace; I was tired": the guess is withdrawn.
    tool(
        &db,
        &s3,
        "memory_correct",
        json!({"id": guess["id"], "retract": true}),
        T0 + 9 * DAY + 1,
    );
    tool(
        &db,
        &s3,
        "memory_remember",
        json!({"content":"Was tired when watching Film B; the pace was not the problem","kind":"observation","data":{"movie":"Film B"}}),
        T0 + 9 * DAY + 2,
    );
    reindex(&db).unwrap();
    let r = recall(&db, &s3, "slow paced films", 10, T0 + 9 * DAY + 3).unwrap();
    assert!(
        r.iter().all(|x| x.id != guess["id"].as_str().unwrap()),
        "{}",
        contents(&r)
    );
    assert!(!profile(&db, &s3, T0 + 9 * DAY + 3)
        .unwrap()
        .text
        .contains("slow-paced"));

    // 7. Forget one reaction, reindex, restart: it stays gone.
    tool(
        &db,
        &s3,
        "memory_forget",
        json!({"id": pace_obs["id"]}),
        T0 + 9 * DAY + 4,
    );
    reindex(&db).unwrap();
    drop(db);
    let db = AssistDb::open(&path).unwrap();
    let r = recall(&db, &s3, "Film B slow pace", 10, T0 + 9 * DAY + 5).unwrap();
    assert!(
        r.iter()
            .all(|x| !x.content.contains("did not like its slow pace")),
        "{}",
        contents(&r)
    );

    // 8. Changing the agent's model changes nothing in memory.
    let (a, b) = (agent("model-x", true), agent("model-y", true));
    assert!(has_recall_grant(&a) && has_recall_grant(&b));
    assert!(!has_recall_grant(&agent("model-x", false)));
    let ta = augment_text(
        &db,
        &ctx::direct(&a.id, Some("conv-day11")),
        "two more",
        T0 + 10 * DAY,
    );
    let tb = augment_text(
        &db,
        &ctx::direct(&b.id, Some("conv-day11")),
        "two more",
        T0 + 10 * DAY,
    );
    assert_eq!(ta, tb);
    assert!(ta.unwrap().contains("chapter 3"));
}
