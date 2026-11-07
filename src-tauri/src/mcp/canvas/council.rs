use std::future::Future;
use std::pin::Pin;

use serde_json::{json, Value};
use tauri::AppHandle;

use super::{need, num, ready, st, ws_floor};

type Fut<'a> = Pin<Box<dyn Future<Output = Result<Value, String>> + Send + 'a>>;

fn sub<'a>(app: &'a AppHandle, name: &'a str, a: Value) -> Fut<'a> {
    Box::pin(super::dispatch(app, name, a))
}

pub const METHODS: &[&str] = &["brainstorm", "dialectic", "arena", "simulation"];

struct Member {
    name: String,
    agent: String,
    perspective: String,
}

fn default_members(method: &str) -> Vec<(&'static str, &'static str)> {
    match method {
        "dialectic" => vec![
            ("Thesis", "You defend the most promising answer to the question with your best arguments and evidence."),
            ("Antithesis", "You attack the thesis: find the weakest premise, the counterexample and the cost nobody counted."),
        ],
        "arena" => vec![
            ("Arena A", "Answer the question on your own, as well as you can."),
            ("Arena B", "Answer the question on your own, as well as you can."),
            ("Arena C", "Answer the question on your own, as well as you can."),
        ],
        "simulation" => vec![
            ("Layperson", "A person with no technical background who just wants it to work."),
            ("Power user", "Someone who uses it every day and hates friction."),
            ("Skeptic", "Someone who doubts every claim and looks for what breaks."),
        ],
        _ => vec![
            ("Pragmatist", "Ship the smallest thing that works; MVP first."),
            ("Cautious", "Risks, what can break, what is hard to undo."),
            ("User", "The person who will use the result, and what they actually need."),
        ],
    }
}

fn chair_script(method: &str, rounds: u64, members: &[Member], q: &str) -> String {
    let names: Vec<String> = members.iter().map(|m| format!("\"{}\"", m.name)).collect();
    let batch_hint = format!(
        "omniget canvas ask --batch '{{{}}}'",
        members
            .iter()
            .map(|m| format!("\"{}\":\"...\"", m.name))
            .collect::<Vec<_>>()
            .join(",")
    );
    let common = format!(
        "You chair a council on the OmniGet canvas. Question: {q}\n\
         Members (corded to you): {}. Notes corded to you: \"Question\", \"Board\" (the shared board) and \"Decision\".\n\
         Tools: run `omniget canvas list` first; ask all members at once with {batch_hint} (give Bash a timeout of 10 minutes); \
         read and write notes with `omniget canvas note read|edit|write \"Board\" ...`. Write in the language of the question.\n\
         After every round, append to \"Board\" a section `## Round N` with what each member said (short, attributed). \
         Never edit project files: this is a decision, not an implementation.\n",
        names.join(", ")
    );
    let steps = match method {
        "dialectic" => format!(
            "Method (dialectic). Round 1: ask the thesis member for its position. Round 2: send that position to the antithesis member and ask for the strongest objection. \
             Round 3: send the objection back to the thesis member for a rebuttal. Repeat rounds 2-3 up to {rounds} times while new arguments appear. \
             Then write \"Decision\": the premises both accept, the contradictions, which arguments survived, the synthesis (your conclusion) and what still needs the person's call."
        ),
        "arena" => format!(
            "Method (arena). Round 1: ask every member the question independently (one batch). Round 2: label the answers A, B, C... without names, send all of them to every member \
             and ask each to rank them best to worst with one line per answer (they must not rank their own first just because it is theirs). \
             Aggregate the ranks (Borda). Up to {rounds} rounds in total. Then write \"Decision\": the ranking with scores, the winning answer, \
             the best ideas grafted from the others, and the open risks."
        ),
        "simulation" => format!(
            "Method (simulation, MiroFish style). Write the question as the seed post in \"Board\". For each of {rounds} rounds, ask every member (one batch) \
             to read the board with `omniget canvas note read \"Board\"` and react in character: agree, disagree, add what they would do. \
             Append their reactions to \"Board\" under `## Round N`. Stop early when two rounds bring nothing new. \
             Then write \"Decision\" as an analyst: where they agree, where they diverge (quote them), what surprised you, and a recommendation."
        ),
        _ => format!(
            "Method (brainstorm, from the superpowers brainstorming skill). Round 1: ask every member (one batch) to restate the problem in one line, \
             list the questions that matter, and propose 2-3 approaches with trade-offs. Round 2: share all approaches (labelled, without names) \
             and ask each member to critique them and pick one, with the reason. Up to {rounds} rounds. Then write \"Decision\": the recommended approach, \
             the alternatives with their trade-offs, the risks, and the questions the person must answer before anything is built. \
             The decision is a proposal: the person approves it."
        ),
    };
    format!(
        "{common}\n{steps}\nWhen \"Decision\" is written, run `omniget canvas say \"Council finished: see the Decision note\"` and stop."
    )
}

fn member_prompt(m: &Member, method: &str, chair: &str, q: &str) -> String {
    format!(
        "You are \"{}\", a member of a council on the OmniGet canvas (method: {method}). Your perspective: {}\n\
         The question being decided: {q}\n\
         The chair \"{chair}\" will send you questions through the canvas; answer each one directly, in the language of the question, in up to about 250 words, \
         with arguments and concrete examples. You may read files to support a point, but never edit project files. \
         If you are corded to the note \"Board\", you can read it with `omniget canvas note read \"Board\"`. Wait for the chair now.",
        m.name, m.perspective
    )
}

fn slug(text: &str) -> String {
    let s: String = text
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let s = s
        .split('-')
        .filter(|p| !p.is_empty())
        .take(6)
        .collect::<Vec<_>>()
        .join("-");
    if s.is_empty() {
        "council".into()
    } else {
        s
    }
}

fn node_id(v: &Value) -> Result<String, String> {
    v.get("node")
        .and_then(|n| n.get("id"))
        .or_else(|| v.get("nodeId"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "CANVAS_COUNCIL: a node was not created".to_string())
}

pub async fn call(app: &AppHandle, name: &str, a: &Value) -> Result<Value, String> {
    match name {
        "canvas_council" => start(app, a).await,
        "canvas_council_status" => status(app, a).await,
        _ => Err("TOOL_NOT_AUTHORIZED".into()),
    }
}

async fn start(app: &AppHandle, a: &Value) -> Result<Value, String> {
    let s = ready(app);
    let (ws, floor) = ws_floor(&s, a)?;
    let question = need(a, "question")?;
    let method = st(a, "method").unwrap_or_else(|| "brainstorm".into());
    if !METHODS.contains(&method.as_str()) {
        return Err(format!(
            "INVALID_ARGUMENT: method '{method}' is not one of {}",
            METHODS.join(", ")
        ));
    }
    let dir = s
        .workspace(&ws)?
        .working_directory
        .filter(|d| !d.trim().is_empty())
        .ok_or("CANVAS_INVALID: a council writes its notes in the workspace folder; give the workspace one with workspace_update {dir} first")?;
    let rounds = num(a, "rounds")
        .map(|r| r.clamp(1.0, 6.0) as u64)
        .unwrap_or(match method.as_str() {
            "simulation" => 3,
            "dialectic" => 2,
            _ => 2,
        });
    let default_agent = st(a, "agent").unwrap_or_else(|| "claude".into());
    let members: Vec<Member> = match a.get("members").and_then(Value::as_array) {
        Some(list) if !list.is_empty() => list
            .iter()
            .enumerate()
            .map(|(i, m)| Member {
                name: m["name"]
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("Member {}", i + 1)),
                agent: m["agent"]
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| default_agent.clone()),
                perspective: m["perspective"]
                    .as_str()
                    .unwrap_or("Answer with your own view.")
                    .to_string(),
            })
            .collect(),
        _ => default_members(&method)
            .into_iter()
            .map(|(n, p)| Member {
                name: n.into(),
                agent: default_agent.clone(),
                perspective: p.into(),
            })
            .collect(),
    };
    if members.len() > 6 {
        return Err("INVALID_ARGUMENT: at most 6 members (each one is a running agent)".into());
    }
    let id = format!(
        "{}-{}",
        slug(&question),
        &super::new_id().replace('-', "")[..6]
    );
    let folder = std::path::Path::new(&dir)
        .join(".omniget-council")
        .join(&id);
    std::fs::create_dir_all(&folder).map_err(|e| format!("CANVAS_COUNCIL: {e}"))?;
    let file = |n: &str| folder.join(n).to_string_lossy().into_owned();
    let write = |n: &str, body: String| {
        std::fs::write(folder.join(n), body).map_err(|e| format!("CANVAS_COUNCIL: {e}"))
    };
    write(
        "pergunta.md",
        format!("# Question\n\n{question}\n\nMethod: {method}, up to {rounds} rounds.\n"),
    )?;
    write(
        "quadro.md",
        "# Board\n\n_The chair records each round here._\n".into(),
    )?;
    write(
        "decisao.md",
        "# Decision\n\n_Still under discussion._\n".into(),
    )?;

    let (x0, y0) = match (num(a, "x"), num(a, "y")) {
        (Some(x), Some(y)) => (x, y),
        _ => {
            let w = 820.0 * (members.len().max(3) as f64);
            let p = s.read(&ws, &floor, |d| {
                omniget_core::core::canvas::layout::free_spot_in(d, w, 1600.0, None, &[])
            })?;
            (p.x, p.y + 120.0)
        }
    };
    let wsv = json!(ws);
    let title = sub(
        app,
        "canvas_node_spawn",
        json!({
            "workspaceId": wsv, "kind": "text", "text": format!("Council ({method}): {question}"),
            "fontSize": 40, "fontWeight": "bold", "x": x0, "y": y0 - 110.0,
            "data": { "council": { "id": id, "method": method } }
        }),
    )
    .await?;
    let title_id = node_id(&title)?;
    let mut notes = Vec::new();
    for (i, (label, f)) in [
        ("Question", "pergunta.md"),
        ("Board", "quadro.md"),
        ("Decision", "decisao.md"),
    ]
    .iter()
    .enumerate()
    {
        let n = sub(
            app,
            "canvas_node_spawn",
            json!({
                "workspaceId": wsv, "kind": "note", "title": label, "path": file(f),
                "x": x0 + 820.0 * i as f64, "y": y0, "w": 760, "h": 440
            }),
        )
        .await?;
        notes.push((label.to_string(), node_id(&n)?));
    }
    let chair_name = st(a, "chairName").unwrap_or_else(|| "Chair".into());
    let chair_agent = st(a, "chairAgent").unwrap_or_else(|| default_agent.clone());
    let mut member_ids = Vec::new();
    for (i, m) in members.iter().enumerate() {
        let n = sub(app, "canvas_node_spawn", json!({
            "workspaceId": wsv, "kind": "terminal", "agent": m.agent, "title": m.name, "cwd": dir,
            "prompt": member_prompt(m, &method, &chair_name, &question),
            "x": x0 + 820.0 * i as f64, "y": y0 + 1040.0, "w": 760, "h": 440
        })).await?;
        member_ids.push(node_id(&n)?);
    }
    let chair = sub(app, "canvas_node_spawn", json!({
        "workspaceId": wsv, "kind": "terminal", "agent": chair_agent, "title": chair_name, "cwd": dir,
        "maestro": true, "start": false,
        "x": x0, "y": y0 + 500.0, "w": 1580, "h": 480
    })).await?;
    let chair_id = node_id(&chair)?;
    for mid in &member_ids {
        sub(app, "canvas_cord_create", json!({ "workspaceId": wsv, "from": chair_id, "to": mid, "kind": "ask", "label": "council" })).await?;
    }
    for (_, nid) in &notes {
        sub(
            app,
            "canvas_cord_create",
            json!({ "workspaceId": wsv, "from": chair_id, "to": nid, "kind": "note" }),
        )
        .await?;
    }
    if method == "simulation" {
        let board = &notes[1].1;
        for mid in &member_ids {
            sub(
                app,
                "canvas_cord_create",
                json!({ "workspaceId": wsv, "from": mid, "to": board, "kind": "note" }),
            )
            .await?;
        }
    }
    sub(
        app,
        "canvas_node_update",
        json!({
            "workspaceId": wsv, "nodeId": chair_id,
            "data": { "initialPrompt": chair_script(&method, rounds, &members, &question) }
        }),
    )
    .await?;
    sub(
        app,
        "canvas_terminal_ensure",
        json!({ "workspaceId": wsv, "nodeId": chair_id }),
    )
    .await?;
    let meta = json!({
        "id": id, "method": method, "rounds": rounds, "question": question,
        "chair": chair_id, "members": member_ids, "notes": notes.iter().map(|(l, n)| json!({ "title": l, "nodeId": n })).collect::<Vec<_>>(),
        "folder": folder.to_string_lossy(),
    });
    sub(
        app,
        "canvas_node_update",
        json!({ "workspaceId": wsv, "nodeId": title_id, "data": { "council": meta } }),
    )
    .await?;
    Ok(json!({
        "council": meta,
        "next": "canvas_council_status {councilId} shows the board and the decision; the chair runs the rounds by itself (watch it on the canvas)",
    }))
}

async fn status(app: &AppHandle, a: &Value) -> Result<Value, String> {
    let s = ready(app);
    let (ws, floor) = ws_floor(&s, a)?;
    let id = need(a, "councilId")?;
    let meta = s
        .read(&ws, &floor, |d| {
            d.nodes.iter().find_map(|n| {
                n.data
                    .get("council")
                    .filter(|c| c["id"] == json!(id))
                    .cloned()
            })
        })?
        .ok_or_else(|| format!("CANVAS_NOT_FOUND: council {id} in workspace {ws}"))?;
    let folder = std::path::PathBuf::from(meta["folder"].as_str().unwrap_or_default());
    let read = |n: &str| std::fs::read_to_string(folder.join(n)).unwrap_or_default();
    let decision = read("decisao.md");
    let board = read("quadro.md");
    let decided = !decision.contains("_Still under discussion._")
        && !decision.contains("_Ainda em discussão._");
    let mut agents = Vec::new();
    let ids = meta["members"].as_array().cloned().unwrap_or_default();
    for nid in std::iter::once(meta["chair"].clone()).chain(ids) {
        if let Ok(v) = sub(
            app,
            "canvas_terminal_status",
            json!({ "workspaceId": ws, "nodeId": nid }),
        )
        .await
        {
            agents.push(json!({ "nodeId": nid, "name": v["name"], "state": v["state"] }));
        }
    }
    // WHY: councils created before the rename logged their rounds as "## Rodada".
    let rounds_logged = board.matches("## Round").count() + board.matches("## Rodada").count();
    Ok(json!({
        "councilId": id,
        "decided": decided,
        "roundsLogged": rounds_logged,
        "agents": agents,
        "decision": if decided { Value::String(decision) } else { Value::Null },
        "boardTail": board.chars().rev().take(1500).collect::<String>().chars().rev().collect::<String>(),
    }))
}
