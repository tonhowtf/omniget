use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use super::store::{self, AuthorKind, Member, RoomDraft};
use super::tasks::{self, DelegateRequest, RoomDispatcher, RunEnd};
use super::*;
use crate::core::assist::db::{tests_support::temp_db, AssistDb};
use crate::core::llm::agent::{AgentDef, GrantMode, ToolGrant, ToolSource};
use crate::core::llm::code_tools::WorkspaceBindings;

// ── helpers ──────────────────────────────────────────────────────────────

#[derive(Default)]
struct FakeDispatcher {
    starts: Mutex<Vec<(String, String, String)>>,
    cancelled: Mutex<Vec<String>>,
    /// When set, every started run finishes by itself with this text.
    auto_answer: Option<(Arc<AssistDb>, String)>,
}

#[async_trait::async_trait]
impl RoomDispatcher for FakeDispatcher {
    async fn start(&self, room: &str, bot: &str, input: String) -> Result<String, String> {
        let mut s = self.starts.lock().unwrap();
        s.push((room.into(), bot.into(), input));
        let run = format!("run-{}-{}", bot, s.len());
        if let Some((db, text)) = &self.auto_answer {
            let (db, text, run2, bot) = (db.clone(), text.clone(), run.clone(), bot.to_string());
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                tasks::finish_run(
                    &db,
                    &run2,
                    RunEnd::Completed,
                    &format!("{text} ({bot})"),
                    None,
                    Some(100),
                )
                .unwrap();
            });
        }
        Ok(run)
    }
    fn cancel(&self, run_id: &str) {
        self.cancelled.lock().unwrap().push(run_id.into());
    }
}

fn members(ids: &[&str]) -> Vec<Member> {
    ids.iter()
        .map(|b| Member {
            bot: (*b).into(),
            role: String::new(),
        })
        .collect()
}

fn room3(db: &AssistDb, coordinator: Option<&str>) -> store::Room {
    store::create_room(
        db,
        &RoomDraft {
            title: "Leitura".into(),
            members: members(&["curator", "researcher", "companion"]),
            coordinator: coordinator.map(str::to_string),
            ..Default::default()
        },
    )
    .unwrap()
}

fn names() -> HashMap<String, String> {
    [
        ("curator", "Curador"),
        ("researcher", "Pesquisador de disponibilidade"),
        ("companion", "Companheiro"),
    ]
    .into_iter()
    .map(|(a, b)| (a.to_string(), b.to_string()))
    .collect()
}

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("omniget-groups-{tag}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&d).unwrap();
    d.canonicalize().unwrap()
}

fn agent_with_project_tools() -> AgentDef {
    let mut a: AgentDef = serde_json::from_value(serde_json::json!({
        "id": "reader", "name": "Reader", "role": "worker", "system_prompt": "",
        "model": { "policy": "fixed", "model": { "provider": "fake", "model": "m" } },
        "runtime": { "kind": "native" }
    }))
    .unwrap();
    for name in [
        "fs_read",
        "fs_edit",
        "shell_exec",
        "todo_write",
        "kb_search",
        "memory_recall",
    ] {
        a.tools.push(ToolGrant {
            source: ToolSource::Internal { name: name.into() },
            mode: GrantMode::Auto,
        });
    }
    a
}

fn internal_names(a: &AgentDef) -> Vec<String> {
    a.tools
        .iter()
        .filter_map(|g| match &g.source {
            ToolSource::Internal { name } => Some(name.clone()),
            _ => None,
        })
        .collect()
}

// ── context (A04, A05) ───────────────────────────────────────────────────

#[test]
fn a04_a_personal_conversation_loses_project_tools_in_the_effective_agent() {
    // A project folder is open in another conversation and globally.
    let project = temp_dir("a04");
    crate::core::llm::code_tools::set_conversation_workspace(
        "g-a04-project",
        Some(project.clone()),
    )
    .unwrap();
    crate::core::llm::code_tools::set_workspace(Some(project)).unwrap();

    let hook = augment();
    let mut personal = agent_with_project_tools();
    let note = hook.augment(&mut personal, "g-a04-personal", "oi");
    let left = internal_names(&personal);
    for t in [
        "fs_read",
        "fs_edit",
        "shell_exec",
        "todo_write",
        "kb_search",
    ] {
        assert!(
            !left.contains(&t.to_string()),
            "{t} must be absent in a personal chat"
        );
    }
    assert!(
        left.contains(&"memory_recall".to_string()),
        "non-project grants stay"
    );
    assert!(note.unwrap().contains("personal"));
    assert_eq!(
        context_of("g-a04-personal"),
        (ContextKind::Projectless, None)
    );

    let mut project_agent = agent_with_project_tools();
    hook.augment(&mut project_agent, "g-a04-project", "oi");
    assert!(internal_names(&project_agent).contains(&"fs_edit".to_string()));
    assert_eq!(context_of("g-a04-project").0, ContextKind::Project);
}

#[test]
fn a05_db_bindings_keep_one_folder_per_conversation_and_rooms_share_theirs() {
    let (db, _) = temp_db();
    let b = DbBindings { db: db.clone() };
    let (x, y) = (temp_dir("a05x"), temp_dir("a05y"));
    assert_eq!(b.get("c1"), None, "no row = personal");
    b.set("c1", Some(&x)).unwrap();
    b.set("c2", Some(&y)).unwrap();
    assert_eq!(b.get("c1"), Some(x.clone()));
    assert_eq!(b.get("c2"), Some(y.clone()));
    b.set("c1", None).unwrap();
    assert_eq!(b.get("c1"), None);
    assert_eq!(b.get("c2"), Some(y), "changing one never moves the other");

    // A room's folder is its members' folder.
    let room = room3(&db, None);
    b.set(&room.id, Some(&x)).unwrap();
    assert_eq!(b.get(&participant_id(&room.id, "curator")), Some(x.clone()));
    // Sanitised form (`~` → `_`) resolves to the same room.
    assert_eq!(b.get(&format!("{}_curator", room.id)), Some(x));

    // The legacy map is imported once and never overwrites a newer choice.
    let mut legacy = HashMap::new();
    legacy.insert("old-chat".to_string(), temp_dir("a05old"));
    legacy.insert("c2".to_string(), temp_dir("a05stale"));
    b.import_legacy(&legacy);
    assert!(b.get("old-chat").is_some());
    assert_ne!(b.get("c2"), legacy.get("c2").cloned());
}

// ── routing (B01) ────────────────────────────────────────────────────────

#[tokio::test]
async fn b01_a_mention_starts_only_that_member_and_keeps_authorship() {
    let (db, _) = temp_db();
    let room = room3(&db, Some("curator"));
    let d = FakeDispatcher::default();

    let out = tasks::send_user_message(
        &db,
        &d,
        &room.id,
        "@pesquisador-de-disponibilidade tem legenda?",
        &names(),
    )
    .await
    .unwrap();
    assert_eq!(out.started.len(), 1);
    assert_eq!(out.started[0].bot, "researcher");
    assert_eq!(
        d.starts.lock().unwrap().len(),
        1,
        "only the mentioned bot started a run"
    );
    let run = out.started[0].run_id.clone();

    tasks::finish_run(&db, &run, RunEnd::Completed, "Sim, na Max.", None, Some(80)).unwrap();
    let msgs = store::messages(&db, &room.id).unwrap();
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0].author, AuthorKind::User);
    assert_eq!(msgs[1].author, AuthorKind::Bot);
    assert_eq!(msgs[1].bot_id.as_deref(), Some("researcher"));
    assert_eq!(msgs[1].run_id.as_deref(), Some(run.as_str()));
    assert_eq!(msgs[1].reply_to.as_deref(), Some(out.message.id.as_str()));
    // A late event for the same run cannot write a second answer.
    tasks::finish_run(&db, &run, RunEnd::Failed, "late", None, None).unwrap();
    assert_eq!(store::messages(&db, &room.id).unwrap().len(), 2);

    // No mention → the coordinator only; mention by id works too.
    let out = tasks::send_user_message(&db, &d, &room.id, "e agora?", &names())
        .await
        .unwrap();
    assert_eq!(
        out.started
            .iter()
            .map(|s| s.bot.as_str())
            .collect::<Vec<_>>(),
        vec!["curator"]
    );
    let out = tasks::send_user_message(&db, &d, &room.id, "@companion e @curator", &names())
        .await
        .unwrap();
    // curator is still busy with the previous message: skipped, not doubled.
    assert_eq!(
        out.started
            .iter()
            .map(|s| s.bot.as_str())
            .collect::<Vec<_>>(),
        vec!["companion"]
    );
    assert_eq!(out.skipped[0].bot, "curator");
    assert!(out.skipped[0].reason.starts_with(ERR_GROUP_BUSY));
    // An e-mail is not a mention.
    assert!(tasks::mentions(
        "mail me at a@curator.com",
        &[("curator".into(), "Curador".into())]
    )
    .is_empty());
}

#[tokio::test]
async fn b01_without_coordinator_the_default_bot_answers_and_bot_text_never_routes() {
    let (db, _) = temp_db();
    let room = store::create_room(
        &db,
        &RoomDraft {
            title: "r".into(),
            members: members(&["a", "b", "c"]),
            default_bot: Some("b".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let d = FakeDispatcher::default();
    let out = tasks::send_user_message(&db, &d, &room.id, "olá", &HashMap::new())
        .await
        .unwrap();
    assert_eq!(out.started[0].bot, "b");
    // The bot answers mentioning others: nothing else starts.
    tasks::finish_run(
        &db,
        &out.started[0].run_id,
        RunEnd::Completed,
        "@a @c vejam isto",
        None,
        None,
    )
    .unwrap();
    assert_eq!(d.starts.lock().unwrap().len(), 1);
}

// ── delegation (B02, B03) ────────────────────────────────────────────────

#[tokio::test]
async fn b02_a_task_is_claimed_once_across_restarts() {
    let (db, path) = temp_db();
    let room = room3(&db, Some("curator"));
    let d = FakeDispatcher::default();
    let out = tasks::send_user_message(&db, &d, &room.id, "rodada", &names())
        .await
        .unwrap();
    let parent = out.started[0].run_id.clone();
    let req = DelegateRequest {
        to: "researcher".into(),
        question: "Onde assistir?".into(),
        ..Default::default()
    };
    let t = tasks::admit_delegation(
        &db,
        &room.id,
        "curator",
        Some(&parent),
        Some("orig-1"),
        &req,
    )
    .unwrap();
    // Replaying the same tool call returns the same task.
    let again = tasks::admit_delegation(
        &db,
        &room.id,
        "curator",
        Some(&parent),
        Some("orig-1"),
        &req,
    )
    .unwrap();
    assert_eq!(t.id, again.id);

    drop(db); // restart in the middle of the acceptance
    let db = Arc::new(AssistDb::open(&path).unwrap());
    let claimed = tasks::claim(&db, &t.id, "child-A").unwrap();
    assert_eq!(claimed.state, "claimed");
    let err = tasks::claim(&db, &t.id, "child-B").unwrap_err();
    assert!(err.starts_with(ERR_GROUP_CLAIMED), "{err}");

    drop(db); // and again after the claim
    let db = Arc::new(AssistDb::open(&path).unwrap());
    assert_eq!(
        tasks::claim(&db, &t.id, "child-A")
            .unwrap()
            .claim_run
            .as_deref(),
        Some("child-A")
    );
    assert!(tasks::claim(&db, &t.id, "child-B").is_err());
    let runs_for_task = tasks::runs(&db, &room.id)
        .unwrap()
        .into_iter()
        .filter(|r| r.task_id.as_deref() == Some(t.id.as_str()))
        .count();
    assert_eq!(runs_for_task, 1, "one specialist, never two");

    // The result is tied to the original task.
    tasks::finish_run(
        &db,
        "child-A",
        RunEnd::Completed,
        "Na Max, legendado.",
        None,
        Some(50),
    )
    .unwrap();
    let done = tasks::task(&db, &t.id).unwrap();
    assert_eq!(done.state, "completed");
    assert_eq!(done.result.as_deref(), Some("Na Max, legendado."));
    let msg = store::messages(&db, &room.id)
        .unwrap()
        .into_iter()
        .find(|m| m.task_id.as_deref() == Some(t.id.as_str()))
        .unwrap();
    assert_eq!(msg.bot_id.as_deref(), Some("researcher"));
    assert_eq!(done.message_id.as_deref(), Some(msg.id.as_str()));
}

#[tokio::test]
async fn b02_after_a_crash_nothing_is_re_run_on_its_own() {
    let (db, path) = temp_db();
    let room = room3(&db, Some("curator"));
    let d = FakeDispatcher::default();
    let out = tasks::send_user_message(&db, &d, &room.id, "x", &names())
        .await
        .unwrap();
    let t = tasks::admit_delegation(
        &db,
        &room.id,
        "curator",
        Some(&out.started[0].run_id),
        None,
        &DelegateRequest {
            to: "companion".into(),
            question: "q".into(),
            ..Default::default()
        },
    )
    .unwrap();
    tasks::claim(&db, &t.id, "child").unwrap();
    drop(db);
    let db = Arc::new(AssistDb::open(&path).unwrap());
    assert!(tasks::recover(&db).unwrap() >= 2);
    assert_eq!(tasks::task(&db, &t.id).unwrap().state, "interrupted");
    assert!(tasks::claim(&db, &t.id, "child-2").is_err());
    assert_eq!(d.starts.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn b03_endless_delegation_stops_at_the_backend_limits() {
    let (db, _) = temp_db();
    let room = room3(&db, Some("curator"));
    let d = FakeDispatcher::default();
    let out = tasks::send_user_message(&db, &d, &room.id, "vai", &names())
        .await
        .unwrap();
    let root = out.started[0].run_id.clone();
    let req = |to: &str| DelegateRequest {
        to: to.into(),
        question: "delegate again, forever".into(),
        ..Default::default()
    };

    // Depth: curator → researcher → companion → (curator) refused.
    let t1 = tasks::admit_delegation(
        &db,
        &room.id,
        "curator",
        Some(&root),
        None,
        &req("researcher"),
    )
    .unwrap();
    tasks::claim(&db, &t1.id, "r1").unwrap();
    let t2 = tasks::admit_delegation(
        &db,
        &room.id,
        "researcher",
        Some("r1"),
        None,
        &req("companion"),
    )
    .unwrap();
    assert_eq!(t2.depth, 2);
    tasks::claim(&db, &t2.id, "r2").unwrap();
    // max_concurrency (3) is reached here, so lift it to see the depth rule.
    let mut draft = RoomDraft {
        title: room.title.clone(),
        members: room.members.clone(),
        coordinator: room.coordinator.clone(),
        ..Default::default()
    };
    let mut limits = RoomLimits::default();
    limits.max_concurrency = 8;
    draft.limits = Some(limits);
    store::update_room(&db, &room.id, &draft).unwrap();
    // `curator` is busy (root run), so aim at a free target for the depth check.
    tasks::finish_run(&db, "r1", RunEnd::Completed, "ok", None, Some(10)).unwrap();
    let err = tasks::admit_delegation(
        &db,
        &room.id,
        "companion",
        Some("r2"),
        None,
        &req("researcher"),
    )
    .unwrap_err();
    assert!(err.contains("depth"), "{err}");

    // Rounds: the root keeps delegating siblings until the round is spent.
    tasks::finish_run(&db, "r2", RunEnd::Completed, "ok", None, Some(10)).unwrap();
    let mut accepted = 2;
    let mut last_err = String::new();
    for i in 0..50 {
        match tasks::admit_delegation(
            &db,
            &room.id,
            "curator",
            Some(&root),
            None,
            &req("companion"),
        ) {
            Ok(t) => {
                accepted += 1;
                let run = format!("sib-{i}");
                tasks::claim(&db, &t.id, &run).unwrap();
                tasks::finish_run(&db, &run, RunEnd::Completed, "ok", None, Some(10)).unwrap();
            }
            Err(e) => {
                last_err = e;
                break;
            }
        }
    }
    assert_eq!(accepted, RoomLimits::default().max_delegations_per_round);
    assert!(last_err.starts_with(ERR_GROUP_LIMIT), "{last_err}");

    // Tokens: a tight token cap refuses before a new turn starts.
    let mut limits = RoomLimits::default();
    limits.max_tokens_per_round = Some(100);
    draft.limits = Some(limits);
    store::update_room(&db, &room.id, &draft).unwrap();
    let out = tasks::send_user_message(&db, &d, &room.id, "@companion ?", &names())
        .await
        .unwrap();
    assert!(out.started.is_empty());
    assert!(out.skipped[0].reason.contains("tokens"));
}

#[tokio::test]
async fn delegate_runs_the_child_and_returns_its_authored_answer() {
    let (db, _) = temp_db();
    let room = room3(&db, Some("curator"));
    let d = FakeDispatcher {
        auto_answer: Some((db.clone(), "resposta".into())),
        ..Default::default()
    };
    let parent = FakeDispatcher::default();
    let out = tasks::send_user_message(&db, &parent, &room.id, "rodada", &names())
        .await
        .unwrap();
    let t = tasks::delegate(
        &db,
        &d,
        &room.id,
        "curator",
        Some(&out.started[0].run_id),
        Some("o-1"),
        &DelegateRequest {
            to: "researcher".into(),
            question: "disponível?".into(),
            context: "Filme X (2019)".into(),
            deliverable: "plataforma + legenda".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(t.state, "completed");
    assert_eq!(t.result.as_deref(), Some("resposta (researcher)"));
    let (_, bot, input) = d.starts.lock().unwrap()[0].clone();
    assert_eq!(bot, "researcher");
    assert!(input.contains("Filme X (2019)") && input.contains("plataforma + legenda"));
}

#[tokio::test]
async fn delegate_stops_a_child_at_the_time_limit() {
    let (db, _) = temp_db();
    let room = room3(&db, Some("curator"));
    let d = FakeDispatcher::default(); // never answers
    let out = tasks::send_user_message(&db, &FakeDispatcher::default(), &room.id, "x", &names())
        .await
        .unwrap();
    let mut req = DelegateRequest {
        to: "companion".into(),
        question: "q".into(),
        ..Default::default()
    };
    req.limit_s = Some(5);
    let started = std::time::Instant::now();
    let t = tasks::delegate(
        &db,
        &d,
        &room.id,
        "curator",
        Some(&out.started[0].run_id),
        None,
        &req,
    )
    .await
    .unwrap();
    assert!(started.elapsed() >= std::time::Duration::from_secs(5));
    assert_eq!(t.state, "failed");
    assert!(t.error.unwrap().contains("time"));
    assert_eq!(
        d.cancelled.lock().unwrap().len(),
        1,
        "the child run was cancelled"
    );
}

// ── cancel (B05) ─────────────────────────────────────────────────────────

#[tokio::test]
async fn b05_cancelling_the_room_stops_active_work_and_keeps_finished_results() {
    let (db, _) = temp_db();
    let room = room3(&db, Some("curator"));
    let d = FakeDispatcher::default();
    let out = tasks::send_user_message(&db, &d, &room.id, "rodada", &names())
        .await
        .unwrap();
    let root = out.started[0].run_id.clone();
    let req = |to: &str| DelegateRequest {
        to: to.into(),
        question: "q".into(),
        ..Default::default()
    };
    let done = tasks::admit_delegation(
        &db,
        &room.id,
        "curator",
        Some(&root),
        None,
        &req("researcher"),
    )
    .unwrap();
    tasks::claim(&db, &done.id, "child-done").unwrap();
    tasks::finish_run(
        &db,
        "child-done",
        RunEnd::Completed,
        "achei 3 opções",
        None,
        Some(20),
    )
    .unwrap();
    let active = tasks::admit_delegation(
        &db,
        &room.id,
        "curator",
        Some(&root),
        None,
        &req("companion"),
    )
    .unwrap();
    tasks::claim(&db, &active.id, "child-active").unwrap();

    let c = tasks::cancel_room(&db, &room.id, Some(&d)).unwrap();
    let mut runs = c.cancelled_runs.clone();
    runs.sort();
    assert_eq!(runs, vec!["child-active".to_string(), root.clone()]);
    assert_eq!(c.kept_tasks, vec![done.id.clone()]);
    assert_eq!(tasks::task(&db, &done.id).unwrap().state, "completed");
    assert_eq!(tasks::task(&db, &active.id).unwrap().state, "cancelled");
    let mut cancelled = d.cancelled.lock().unwrap().clone();
    cancelled.sort();
    assert_eq!(cancelled, runs);
    let msgs = store::messages(&db, &room.id).unwrap();
    assert!(msgs
        .iter()
        .any(|m| m.text == "achei 3 opções" && m.bot_id.as_deref() == Some("researcher")));
    assert!(msgs
        .iter()
        .any(|m| m.author == AuthorKind::System && m.text.contains("kept: 1")));
    // Late end of a cancelled run changes nothing.
    tasks::finish_run(
        &db,
        "child-active",
        RunEnd::Completed,
        "tarde demais",
        None,
        None,
    )
    .unwrap();
    assert!(!store::messages(&db, &room.id)
        .unwrap()
        .iter()
        .any(|m| m.text == "tarde demais"));
}

#[tokio::test]
async fn a_parent_that_fails_cancels_its_active_children() {
    let (db, _) = temp_db();
    let room = room3(&db, Some("curator"));
    let d = FakeDispatcher::default();
    let out = tasks::send_user_message(&db, &d, &room.id, "rodada", &names())
        .await
        .unwrap();
    let root = out.started[0].run_id.clone();
    let t = tasks::admit_delegation(
        &db,
        &room.id,
        "curator",
        Some(&root),
        None,
        &DelegateRequest {
            to: "companion".into(),
            question: "q".into(),
            ..Default::default()
        },
    )
    .unwrap();
    tasks::claim(&db, &t.id, "kid").unwrap();
    let (_, cascade) = tasks::finish_run(&db, &root, RunEnd::Cancelled, "", None, None).unwrap();
    assert_eq!(cascade, vec!["kid".to_string()]);
    assert_eq!(tasks::task(&db, &t.id).unwrap().state, "cancelled");
}

// ── memory in rooms (longitudinal step 6) ────────────────────────────────

#[test]
fn step6_a_room_member_cannot_read_private_notes_without_a_share() {
    use crate::core::assist::ctx::{AssistCtx, Scope};
    use crate::core::assist::memory::{self, Category, NewMemory, Source};
    let (db, _) = temp_db();
    let room = room3(&db, Some("curator"));
    let now = crate::core::assist::now_ms();
    let user = AssistCtx::user();
    let private = memory::remember(
        &db,
        &user,
        NewMemory::new(
            Scope::Bot {
                bot: "companion".into(),
            },
            Category::Declared,
            "Notas pessoais: chorei no capítulo 12 de Torto Arado",
            Source::user(now),
        ),
        now,
    )
    .unwrap()
    .record;
    let profile = memory::remember(
        &db,
        &user,
        NewMemory::new(
            Scope::User,
            Category::Declared,
            "Prefere legendas em português",
            Source::user(now),
        ),
        now,
    )
    .unwrap()
    .record;

    // The curator in the room asks for the companion's personal notes.
    let conv = participant_id(&room.id, "curator");
    let ctx = resolve_in(Some(&db), "curator", Some(&conv), &room.id);
    assert_eq!(
        ctx.readable,
        vec![Scope::Room {
            conversation: room.id.clone()
        }]
    );
    for q in ["notas pessoais", "Torto Arado", "capítulo 12", "legendas"] {
        assert!(
            memory::recall(&db, &ctx, q, 20, now).unwrap().is_empty(),
            "leaked for {q}"
        );
    }
    assert!(memory::get(&db, &ctx, &private.id).is_err());
    // Even the companion itself, inside the room, does not bring its private notes.
    let comp = resolve_in(
        Some(&db),
        "companion",
        Some(&participant_id(&room.id, "companion")),
        &room.id,
    );
    assert!(memory::recall(&db, &comp, "Torto Arado", 20, now)
        .unwrap()
        .is_empty());
    // A bot that is not a member gets nothing at all.
    let outsider = resolve_in(Some(&db), "stranger", Some(&room.id), &room.id);
    assert!(outsider.readable.is_empty() && outsider.writable.is_empty());

    // The user shares the profile (not the private notes) into the room.
    let share = store::add_share(&db, &room.id, &Scope::User, None, "legendas").unwrap();
    let ctx = resolve_in(Some(&db), "curator", Some(&conv), &room.id);
    assert!(memory::recall(&db, &ctx, "legendas", 20, now)
        .unwrap()
        .iter()
        .any(|r| r.id == profile.id));
    assert!(memory::recall(&db, &ctx, "Torto Arado", 20, now)
        .unwrap()
        .is_empty());
    assert!(!ctx.can_write(&Scope::User), "a share is read-only");
    // Revoking cuts future access.
    store::revoke_share(&db, &share.id).unwrap();
    let ctx = resolve_in(Some(&db), "curator", Some(&conv), &room.id);
    assert!(memory::recall(&db, &ctx, "legendas", 20, now)
        .unwrap()
        .is_empty());
}

#[test]
fn room_context_shows_authors_and_only_room_messages() {
    let (db, _) = temp_db();
    let room = room3(&db, Some("curator"));
    store::insert_message(
        &db,
        &store::NewMessage {
            room: &room.id,
            author: AuthorKind::Bot,
            bot_id: Some("researcher"),
            reply_to: None,
            run_id: Some("r"),
            task_id: None,
            round: 1,
            text: "Está na Max com legenda PT.",
            status: "done",
        },
    )
    .unwrap();
    let text = room_context(&db, &room.id, "curator", "nova pergunta").unwrap();
    assert!(text.contains("@researcher: Está na Max"));
    assert!(text.contains("You coordinate"));
    let text = room_context(&db, &room.id, "companion", "x").unwrap();
    assert!(!text.contains("You coordinate"));
    assert!(room_context(&db, &room.id, "stranger", "x").is_none());
}

#[test]
fn participant_ids_parse_and_stay_short() {
    let id = "grp-0123456789ab";
    assert!(store::is_room_id(id));
    assert_eq!(
        participant(&format!("{id}~agent-lk2j")),
        Some((id, "agent-lk2j"))
    );
    assert_eq!(participant(&format!("{id}_coder")), Some((id, "coder")));
    assert_eq!(participant("conv-123~x"), None);
    assert_eq!(participant(id), None);
    assert_eq!(room_of(id), Some(id));
    assert_eq!(
        tasks::handle_of("Pesquisador de disponibilidade"),
        "pesquisador-de-disponibilidade"
    );
}

// ── worktrees (B06) ──────────────────────────────────────────────────────

#[test]
fn b06_worktree_cleanup_refuses_folders_the_app_did_not_create() {
    let (db, _) = temp_db();
    let repo = temp_dir("repo");
    let git = |args: &[&str]| {
        let ok = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(args)
            .output()
            .unwrap();
        assert!(
            ok.status.success(),
            "{}",
            String::from_utf8_lossy(&ok.stderr)
        );
    };
    git(&["init", "-q"]);
    std::fs::write(repo.join("a.txt"), "a").unwrap();
    git(&["add", "."]);
    git(&[
        "-c",
        "user.email=t@t",
        "-c",
        "user.name=t",
        "commit",
        "-qm",
        "init",
    ]);

    let base = temp_dir("wt");
    let wt = worktree::create(&db, &repo, &base, Some("c1")).unwrap();
    assert!(wt.path.join("a.txt").exists());
    assert!(
        !wt.path.join(worktree::MARKER).exists(),
        "the marker is not in the working tree"
    );

    // A folder of the user's, even inside the same base, is refused and kept.
    let foreign = base.join("mine");
    std::fs::create_dir_all(&foreign).unwrap();
    std::fs::write(foreign.join("keep.txt"), "x").unwrap();
    let err = worktree::remove(&db, &foreign).unwrap_err();
    assert!(err.starts_with(ERR_GROUP_NOT_OWNED));
    assert!(foreign.join("keep.txt").exists());
    // The repository itself is refused too.
    assert!(worktree::remove(&db, &repo).is_err());
    assert!(repo.join("a.txt").exists());

    worktree::remove(&db, &wt.path).unwrap();
    assert!(!wt.path.exists());
    assert!(worktree::list(&db).unwrap().is_empty());
}

#[test]
fn b01_a_mention_by_the_display_name_shown_in_the_room_reaches_that_member() {
    let members = vec![
        ("omni".to_string(), "Omni".to_string()),
        (
            "bot-companheiro-de-leitura".to_string(),
            "Companheiro de leitura".to_string(),
        ),
        ("ana".to_string(), "Ana".to_string()),
        ("ana-maria".to_string(), "Ana Maria".to_string()),
        ("joao".to_string(), "João Crítico".to_string()),
    ];
    // What the room header shows, typed by hand, with spaces.
    assert_eq!(
        tasks::mentions("@Companheiro de leitura quais filmes eu vi?", &members),
        vec!["bot-companheiro-de-leitura"]
    );
    // What the autocomplete inserts.
    assert_eq!(
        tasks::mentions("@companheiro-de-leitura oi", &members),
        vec!["bot-companheiro-de-leitura"]
    );
    // Longest name wins; accents and case do not matter.
    assert_eq!(
        tasks::mentions("@ana maria, veja", &members),
        vec!["ana-maria"]
    );
    assert_eq!(tasks::mentions("@Ana, veja", &members), vec!["ana"]);
    assert_eq!(
        tasks::mentions("@joao critico opine", &members),
        vec!["joao"]
    );
    // A name must end at a word boundary.
    assert!(tasks::mentions("@Omnipresente", &members).is_empty());
    assert!(tasks::mentions("mail me at a@omni.com", &members).is_empty());
}
