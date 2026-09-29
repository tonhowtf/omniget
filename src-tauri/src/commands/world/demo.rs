//! `world_demo`: the house at work without spending a token.
//!
//! The route calls it from `/world?demo=1` and from the "Demo" button. Every
//! native resident gets a real job — queue, log, `/llm/jobs` and all — whose
//! conversation is pinned to a scripted provider: each round of the turn asks
//! for one harmless tool inside a throwaway workspace, a few seconds apart, so
//! the bus carries the same `TurnStarted` / `ToolCalled` / `ToolAsk` /
//! `TurnEnded` a paid turn would and the house reacts the way it always does.
//! One of the calls is an edit, which asks for permission: the agent waves, and
//! the demo says yes after a moment.
//!
//! Residents that cannot be scripted (a CLI or ACP agent runs its own model)
//! and the extra bodies of a crowded run get the same events straight on the
//! bus instead. Nothing here touches the simulation: it only ever talks to the
//! bus and to the job queue.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::BoxStream;
use omniget_core::core::llm::agent::RuntimeKind;
use omniget_core::core::llm::error::LlmError;
use omniget_core::core::llm::providers::fake::FakeProvider;
use omniget_core::core::llm::providers::Provider;
use omniget_core::core::llm::types::{
    FinishReason, ModelRef, ProviderId, TurnEvent, TurnRequest, Usage,
};
use omniget_core::core::omni::bus::BusEvent;
use serde_json::{json, Value};

/// Residents the demo drives when the caller does not say.
const DEFAULT_AGENTS: usize = 3;
/// The tier table stops at eight animated agents.
const MAX_AGENTS: usize = 8;
/// Scripted turns that go through the job queue, which is as many as it runs
/// at the same time (`MAX_PARALLEL_JOBS` in `jobs.rs`).
const QUEUED_JOBS: usize = 2;
/// Pause between two events of a scripted round. Six events a round, so a
/// round is about four seconds and a five-round turn films in about twenty.
const EVENT_DELAY: Duration = Duration::from_millis(700);
/// How long an agent waves before the demo answers its permission prompt.
const ASK_WAVE: Duration = Duration::from_millis(3200);
/// Everything the demo started is gone after this long, answered or not.
const DEMO_BUDGET: Duration = Duration::from_secs(60);

const CART_JS: &str = "export function total(items) {\n  return items.reduce((sum, item) => sum + item.price, 0);\n}\n";
const CART_TEST_JS: &str = "import { total } from '../cart.js';\n\nconst got = total([{ price: 2, qty: 3 }]);\nif (got !== 6) throw new Error(`expected 6, got ${got}`);\n";
const README: &str = "# Demo shop\n\nA cart with a bug: `total` forgets the quantity.\n";

/// A provider that plays a different script on every round of the turn: the
/// Coordinator calls the model again after each tool result, and a provider
/// that repeated itself would ask for the same tool until the tool limit.
struct ScriptedRounds {
    rounds: Vec<Vec<TurnEvent>>,
    next: AtomicUsize,
}

#[async_trait]
impl Provider for ScriptedRounds {
    async fn turn(&self, req: TurnRequest) -> Result<BoxStream<'static, TurnEvent>, LlmError> {
        let i = self
            .next
            .fetch_add(1, Ordering::SeqCst)
            .min(self.rounds.len() - 1);
        FakeProvider::new(self.rounds[i].clone())
            .with_delay(EVENT_DELAY)
            .turn(req)
            .await
    }
}

fn tool_round(n: usize, say: &str, tool: &str, input: Value) -> Vec<TurnEvent> {
    let id = format!("demo-call-{n}");
    vec![
        TurnEvent::Started {
            request_id: "demo".into(),
        },
        TurnEvent::TextDelta {
            text: format!("{say}\n"),
        },
        TurnEvent::ToolCallStart {
            id: id.clone(),
            name: tool.into(),
        },
        TurnEvent::ToolCallDelta {
            id: id.clone(),
            input_json_delta: input.to_string(),
        },
        TurnEvent::ToolCallEnd { id },
        TurnEvent::Finished {
            reason: FinishReason::ToolUse,
        },
    ]
}

fn last_round(say: &str) -> Vec<TurnEvent> {
    vec![
        TurnEvent::Started {
            request_id: "demo".into(),
        },
        TurnEvent::TextDelta { text: say.into() },
        TurnEvent::Usage {
            usage: Usage {
                input_tokens: 0,
                output_tokens: 0,
                ..Usage::default()
            },
        },
        TurnEvent::Finished {
            reason: FinishReason::Stop,
        },
    ]
}

/// `(prompt, rounds)` for the n-th scripted resident. Three parts, so that
/// three agents are busy with three different things at three different posts.
fn script(n: usize) -> (&'static str, Vec<Vec<TurnEvent>>) {
    match n % 3 {
        0 => (
            "Demo: plan the cart fix and check the workspace.",
            vec![
                tool_round(
                    0,
                    "Planning the fix.",
                    "todo_write",
                    json!({ "plan": [
                        { "step": "Read cart.js", "status": "in_progress" },
                        { "step": "Fix total()", "status": "pending" },
                        { "step": "Run the test", "status": "pending" },
                    ]}),
                ),
                tool_round(
                    1,
                    "Looking at the workspace.",
                    "fs_list",
                    json!({ "path": "." }),
                ),
                tool_round(
                    2,
                    "Reading the notes.",
                    "fs_read",
                    json!({ "path": "README.md" }),
                ),
                tool_round(
                    3,
                    "Checking the test.",
                    "fs_read",
                    json!({ "path": "test/cart.test.js" }),
                ),
                last_round("Plan ready: Builder fixes total(), Scout confirms where it is used."),
            ],
        ),
        1 => (
            "Demo: fix the bug in cart.js.",
            vec![
                tool_round(
                    0,
                    "Reading the cart.",
                    "fs_read",
                    json!({ "path": "cart.js" }),
                ),
                tool_round(
                    1,
                    "total() forgets the quantity.",
                    "fs_edit",
                    json!({
                        "path": "cart.js",
                        "old_string": "sum + item.price",
                        "new_string": "sum + item.price * item.qty",
                    }),
                ),
                tool_round(
                    2,
                    "Reading it back.",
                    "fs_read",
                    json!({ "path": "cart.js" }),
                ),
                last_round("Fixed: total() now multiplies the price by the quantity."),
            ],
        ),
        _ => (
            "Demo: find every use of total().",
            vec![
                tool_round(
                    0,
                    "Listing the sources.",
                    "fs_glob",
                    json!({ "pattern": "**/*.js" }),
                ),
                tool_round(
                    1,
                    "Searching for total().",
                    "fs_grep",
                    json!({ "pattern": "total" }),
                ),
                tool_round(
                    2,
                    "Reading the test.",
                    "fs_read",
                    json!({ "path": "test/cart.test.js" }),
                ),
                tool_round(
                    3,
                    "Reading the cart.",
                    "fs_read",
                    json!({ "path": "cart.js" }),
                ),
                last_round("total() is used in one place: test/cart.test.js."),
            ],
        ),
    }
}

/// The same turn for a body the demo cannot script, straight on the bus.
fn synthetic_tools(n: usize) -> &'static [&'static str] {
    match n % 3 {
        0 => &["fs_list", "fs_read", "todo_write"],
        1 => &["fs_read", "fs_grep", "fs_read", "shell_exec"],
        _ => &["fs_glob", "fs_read", "kb_search"],
    }
}

/// A throwaway project under `<app_data>/world/demo`, reset on every run so the
/// scripted edit always finds its line.
fn reset_workspace(root: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let dir = super::save::world_dir(root).join("demo");
    let write = |rel: &str, body: &str| -> Result<(), String> {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("ERR_WORLD_DEMO: {e}"))?;
        }
        std::fs::write(&path, body).map_err(|e| format!("ERR_WORLD_DEMO: {e}"))
    };
    write("cart.js", CART_JS)?;
    write("test/cart.test.js", CART_TEST_JS)?;
    write("README.md", README)?;
    Ok(dir)
}

/// Put the residents to work. `agents` is how many bodies should be busy (3 by
/// default, 8 at most); bodies the roster does not have are borrowed for the
/// length of the demo.
#[tauri::command]
pub async fn world_demo(
    agents: Option<u8>,
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::AppState>,
) -> Result<Value, String> {
    let manager = super::session::ensure_manager(&app, &state)?;
    if !manager.has_world() {
        return Err(super::ERR_NO_WORLD.to_string());
    }
    let want = (agents.map(|n| n as usize).unwrap_or(DEFAULT_AGENTS)).clamp(1, MAX_AGENTS);
    manager.sync_roster();
    let mut cast: Vec<String> = manager.residents().into_iter().map(|(n, _)| n).collect();
    let mut guests = Vec::new();
    while cast.len() < want {
        let name = format!("demo-{}", guests.len() + 1);
        if !cast.contains(&name) {
            manager.add_guest(&name);
            cast.push(name.clone());
        }
        guests.push(name);
    }
    cast.truncate(want);

    crate::commands::llm::ensure_wired(&app);
    let llm = state.llm.clone();
    let jobs = crate::jobs::get(&app)?;
    let workspace = reset_workspace(manager.root())?;
    let bus = llm.bus();

    let mut job_ids = Vec::new();
    let mut scripted = 0usize;
    for (n, name) in cast.iter().enumerate() {
        let native = llm
            .agent(name)
            .map(|a| matches!(a.runtime, RuntimeKind::Native))
            .unwrap_or(false);
        // Staggered, so the house fills up instead of jumping.
        let start = Duration::from_millis(900 * n as u64);
        if native && scripted < 3 {
            let (prompt, rounds) = script(scripted);
            scripted += 1;
            let provider = format!("world-demo-{n}");
            let conversation = format!("world-demo-{name}");
            llm.register_provider(
                &provider,
                Arc::new(ScriptedRounds {
                    rounds,
                    next: AtomicUsize::new(0),
                }),
            );
            let _ = llm.conversation_delete(&conversation);
            llm.switch_model(
                &conversation,
                ModelRef {
                    provider: ProviderId::new(provider),
                    model: "demo".into(),
                },
            )?;
            tokio::time::sleep(if n == 0 {
                Duration::ZERO
            } else {
                Duration::from_millis(900)
            })
            .await;
            if job_ids.len() < QUEUED_JOBS {
                let job = jobs.submit(
                    "run",
                    name,
                    prompt,
                    Some(workspace.to_string_lossy().to_string()),
                    Some(conversation),
                    None,
                )?;
                job_ids.push(job.id);
            } else {
                // The queue runs two jobs at a time and a third would wait for
                // a slot. It is the same turn through the same Coordinator,
                // only not queued, so all three are at work at once.
                omniget_core::core::llm::code_tools::set_conversation_workspace(
                    &conversation,
                    Some(workspace.clone()),
                )?;
                let llm = llm.clone();
                let agent = name.clone();
                tauri::async_runtime::spawn(async move {
                    use futures::StreamExt;
                    let Ok((request, _cancel, mut stream)) =
                        llm.turn_stream(&conversation, &agent, prompt).await
                    else {
                        return;
                    };
                    while let Some(event) = stream.next().await {
                        llm.note_event(&agent, &event);
                    }
                    llm.finish_turn(&request, &agent);
                });
            }
        } else {
            let bus = bus.clone();
            let agent = name.clone();
            let tools = synthetic_tools(n);
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(start).await;
                bus.emit(BusEvent::TurnStarted {
                    agent: agent.clone(),
                    conversation: "world-demo".into(),
                });
                for tool in tools {
                    tokio::time::sleep(Duration::from_millis(4200)).await;
                    bus.emit(BusEvent::ToolCalled {
                        agent: agent.clone(),
                        tool: (*tool).into(),
                        ok: true,
                        ms: 40,
                    });
                }
                tokio::time::sleep(Duration::from_millis(4200)).await;
                bus.emit(BusEvent::TurnEnded {
                    agent,
                    usage: Usage::default(),
                });
            });
        }
    }

    // The permission prompts of the demo's own jobs are answered by the demo,
    // after the wave had time to be seen. Nobody else's prompt is touched.
    {
        let llm = llm.clone();
        let jobs = jobs.clone();
        let ids = job_ids.clone();
        let manager = manager.clone();
        let guests = guests.clone();
        tauri::async_runtime::spawn(async move {
            let started = std::time::Instant::now();
            let mut seen: std::collections::BTreeMap<String, std::time::Instant> =
                Default::default();
            loop {
                tokio::time::sleep(Duration::from_millis(400)).await;
                let mine: Vec<String> = ids
                    .iter()
                    .filter_map(|id| jobs.job(id))
                    .filter_map(|j| j.request_id)
                    .collect();
                for ask in llm.pending_ask_list() {
                    let request = ask["request_id"].as_str().unwrap_or("");
                    let call = ask["tool_call_id"].as_str().unwrap_or("").to_string();
                    if !mine.iter().any(|r| r == request) {
                        continue;
                    }
                    let since = *seen
                        .entry(call.clone())
                        .or_insert_with(std::time::Instant::now);
                    if since.elapsed() >= ASK_WAVE {
                        let _ = llm.answer_tool(request, &call, true, false);
                    }
                }
                let running = ids.iter().filter_map(|id| jobs.job(id)).any(|j| {
                    !matches!(j.state.as_str(), "done" | "review" | "failed" | "cancelled")
                });
                let over = started.elapsed() >= DEMO_BUDGET;
                if over {
                    for id in &ids {
                        let _ = jobs.cancel(id);
                    }
                }
                if over || (!running && started.elapsed() >= Duration::from_secs(24)) {
                    break;
                }
            }
            for name in &guests {
                manager.remove_guest(name);
            }
        });
    }

    Ok(json!({ "agents": cast, "jobs": job_ids, "guests": guests }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_script_ends_in_a_round_that_asks_for_no_tool() {
        for n in 0..3 {
            let (_, rounds) = script(n);
            let (last, before) = rounds.split_last().unwrap();
            assert!(matches!(
                last.last(),
                Some(TurnEvent::Finished {
                    reason: FinishReason::Stop
                })
            ));
            for round in before {
                assert!(matches!(
                    round.last(),
                    Some(TurnEvent::Finished {
                        reason: FinishReason::ToolUse
                    })
                ));
            }
        }
    }

    #[test]
    fn the_workspace_is_reset_so_the_scripted_edit_always_matches() {
        let root = std::env::temp_dir().join(format!("omniget-world-demo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = reset_workspace(&root).unwrap();
        std::fs::write(dir.join("cart.js"), "edited").unwrap();
        let dir = reset_workspace(&root).unwrap();
        let cart = std::fs::read_to_string(dir.join("cart.js")).unwrap();
        assert!(cart.contains("sum + item.price"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
