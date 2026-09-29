//! BN11 (roteiro 3.8): a Arena de ponta a ponta, 3 agentes × 4 tarefas de
//! código, sobre o motor real de threads, o reator do git (uma worktree por
//! thread), a finalização da arena (commit, diff, verificação), a
//! comparação, "escolher este" (integrate do 3.5) e "remover os outros"
//! (remoção segura do 2.3).
//!
//! O agente é o `fake_claude.py --mode apply` rodando na worktree da coluna
//! (cwd = worktree, como um driver roda), e o turno é o que um driver
//! emitiria (`turn.started`/`turn.completed` com uso e custo do `result`).
//! Os fixtures vêm do `scripts/llm/arena_eval.py` (`BN11_FIXTURES`); sem
//! eles o teste não faz nada. Métricas em `OMNIGET_BENCH_OUT` (JSONL).

use std::collections::BTreeSet;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::tests::{git, test_data, test_engine, until};
use super::*;

fn bench(task: &str, metric: &str, value: Value) {
    if let Ok(path) = std::env::var("OMNIGET_BENCH_OUT") {
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(
                f,
                "{}",
                json!({ "bench": "bn11", "task": task, "metric": metric, "value": value })
            );
        }
    }
}

/// Tudo o que o usuário vê do repositório base, por parte (uma diferença diz
/// qual): os arquivos fora do `.git` (caminho + bytes), o HEAD, o índice
/// (`ls-files -s`: conteúdo, não o carimbo de stat que um `git status`
/// reescreve), o `status` e as branches do usuário (as `omniget/*` das
/// colunas nascem no mesmo repositório por construção: é onde a worktree
/// guarda o trabalho).
fn base_parts(repo: &Path) -> std::collections::BTreeMap<String, String> {
    use sha2::{Digest, Sha256};
    let hash = |b: &[u8]| format!("{:x}", Sha256::digest(b));
    let mut files = Vec::new();
    let mut stack = vec![repo.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.file_name().is_some_and(|n| n == ".git") {
                continue;
            }
            if e.file_type().unwrap().is_dir() {
                stack.push(p);
            } else {
                files.push(p);
            }
        }
    }
    files.sort();
    let mut out = std::collections::BTreeMap::new();
    for f in files {
        out.insert(
            format!("file:{}", f.strip_prefix(repo).unwrap().to_string_lossy()),
            hash(&std::fs::read(&f).unwrap_or_default()),
        );
    }
    out.insert("HEAD".into(), git(repo, &["rev-parse", "HEAD"]));
    out.insert(
        "index".into(),
        hash(git(repo, &["ls-files", "-s"]).as_bytes()),
    );
    out.insert(
        "status".into(),
        git(repo, &["status", "--porcelain=v1", "--untracked-files=all"]),
    );
    let user_refs: Vec<String> = git(
        repo,
        &[
            "for-each-ref",
            "--format=%(refname) %(objectname)",
            "refs/heads/",
        ],
    )
    .lines()
    .filter(|l| !l.starts_with("refs/heads/omniget/"))
    .map(String::from)
    .collect();
    out.insert("branches".into(), user_refs.join("\n"));
    out
}

/// Os caminhos que um `changes.json` do `fake_claude --mode apply` toca.
fn change_paths(changes: &Value) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for k in ["write", "write_b64"] {
        if let Some(m) = changes[k].as_object() {
            out.extend(m.keys().cloned());
        }
    }
    if let Some(l) = changes["delete"].as_array() {
        out.extend(l.iter().filter_map(|v| v.as_str().map(String::from)));
    }
    out
}

fn fake_claude() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../scripts/llm/fake_claude.py")
        .canonicalize()
        .expect("scripts/llm/fake_claude.py")
}

/// Um "agente": o fake na worktree; devolve (tokens de entrada, de saída,
/// custo) do `result` dele.
async fn run_agent(dir: PathBuf, changes: PathBuf, cost: f64) -> (u64, u64, f64, bool) {
    let script = fake_claude();
    tokio::task::spawn_blocking(move || {
        let out = std::process::Command::new("python3")
            .arg(script)
            .args(["-p", "--output-format", "stream-json"])
            .current_dir(&dir)
            .env("FAKE_CLAUDE_MODE", "apply")
            .env("FAKE_CLAUDE_CHANGES", &changes)
            .env("FAKE_CLAUDE_COST_USD", cost.to_string())
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        let res = text
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .find(|v| v["type"] == "result")
            .unwrap_or(Value::Null);
        let u = &res["usage"];
        let input = u["input_tokens"].as_u64().unwrap_or(0)
            + u["cache_read_input_tokens"].as_u64().unwrap_or(0);
        (
            input,
            u["output_tokens"].as_u64().unwrap_or(0),
            res["total_cost_usd"].as_f64().unwrap_or(0.0),
            out.status.success() && res["subtype"] == "success",
        )
    })
    .await
    .unwrap()
}

async fn turn_event(thread: &str, turn: &str, kind: &str, payload: Value) {
    let env = json!({ "type": "thread.runtime.append", "event": {
        "eventId": uuid::Uuid::new_v4().to_string(),
        "driver": "native", "instanceId": "native",
        "threadId": thread, "createdAt": now_iso(), "turnId": turn,
        "type": kind, "payload": payload } });
    dispatch(&test_engine(), env).await.unwrap();
}

async fn one_task(task: &str, dir: &Path) {
    let spec: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("agents.json")).unwrap()).unwrap();
    let agents = spec["agents"].as_array().unwrap().clone();
    let verify = spec["verify"].as_str().map(String::from);

    // O repositório do usuário: a semente num commit, identidade própria.
    let repo = dir.join("repo");
    copy_dir(&dir.join("seed"), &repo);
    let repo = repo.canonicalize().unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "bn11@omniget.local"]);
    git(&repo, &["config", "user.name", "bn11"]);
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "seed"]);
    let base = git(&repo, &["rev-parse", "HEAD"]);
    let before = base_parts(&repo);

    let engine = test_engine();
    let notify: Notify = Arc::new(|_: &str| {});
    let entries: Vec<Value> = agents
        .iter()
        .map(|a| json!({ "instanceId": "native", "driver": "native", "model": "fake/fake", "label": a["label"] }))
        .collect();
    let req: ArenaCreate = serde_json::from_value(json!({
        "workspaceRoot": repo.to_string_lossy(),
        "prompt": spec["prompt"],
        "verifyCommand": verify,
        "verifyTimeoutSecs": 60,
        "entries": entries,
    }))
    .unwrap();
    let t0 = Instant::now();
    let a = create_core(&engine, &notify, req, &[]).await.unwrap();
    let id = a.arena_id.clone();
    let a = until(&id, "worktrees", |a| {
        a.entries.iter().all(|e| e.worktree_path.is_some())
    })
    .await;
    bench(
        task,
        "worktrees_ready_ms",
        json!(t0.elapsed().as_millis() as u64),
    );

    // Os 3 agentes ao mesmo tempo, cada um na worktree da sua coluna.
    let mut handles = Vec::new();
    for (e, ag) in a.entries.iter().zip(&agents) {
        let row = engine
            .read(|c| store::thread_row(c, &e.thread_id))
            .unwrap()
            .unwrap();
        let turn = row.active_turn_id.clone().expect("turn requested");
        let thread = e.thread_id.clone();
        let wt = PathBuf::from(e.worktree_path.clone().unwrap());
        let changes = dir.join(ag["changes"].as_str().unwrap());
        let cost = ag["cost"].as_f64().unwrap_or(0.0);
        handles.push(tokio::spawn(async move {
            turn_event(&thread, &turn, "turn.started", json!({})).await;
            let (i, o, usd, ok) = run_agent(wt, changes, cost).await;
            turn_event(
                &thread,
                &turn,
                "turn.completed",
                json!({ "state": if ok { "completed" } else { "failed" },
                        "usage": { "inputTokens": i, "outputTokens": o },
                        "totalCostUsd": usd }),
            )
            .await;
            ok
        }));
    }
    let mut agents_ok = 0;
    for h in handles {
        agents_ok += usize::from(h.await.unwrap());
    }
    let a = until(&id, "review", |a| a.status == "review").await;
    bench(task, "arena_ms", json!(t0.elapsed().as_millis() as u64));
    bench(task, "agents_ok", json!(agents_ok));

    // 0 escrita fora da própria worktree: a base igual byte a byte, e cada
    // commit com exatamente os arquivos do seu agente.
    let mid = base_parts(&repo);
    bench(task, "base_unchanged", json!(before == mid));
    let changed: Vec<&String> = before
        .keys()
        .chain(mid.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|k| before.get(*k) != mid.get(*k))
        .collect();
    bench(task, "base_changed_parts", json!(changed));
    let mut own_only = 0;
    let mut per_entry = Vec::new();
    for (e, ag) in a.entries.iter().zip(&agents) {
        let changes: Value = serde_json::from_str(
            &std::fs::read_to_string(dir.join(ag["changes"].as_str().unwrap())).unwrap(),
        )
        .unwrap();
        let want = change_paths(&changes);
        let got: BTreeSet<String> = match &e.commit_sha {
            Some(c) => compare::touches(&repo, &base, c)
                .await
                .unwrap()
                .into_keys()
                .collect(),
            None => BTreeSet::new(),
        };
        own_only += usize::from(want == got);
        per_entry.push(json!({
            "label": e.label, "status": e.status, "files": e.files_changed,
            "additions": e.additions, "deletions": e.deletions,
            "duration_ms": e.duration_ms, "cost_usd": e.cost_usd,
            "tokens": e.input_tokens + e.output_tokens, "checks": e.test_status,
            "own_files_only": want == got,
            "unexpected": got.difference(&want).cloned().collect::<Vec<_>>(),
        }));
    }
    bench(task, "entries", json!(per_entry));
    bench(task, "own_files_only", json!(own_only));

    // Comparação: os arquivos divergentes.
    let cmp = compare_core(&id).await.unwrap();
    let divergent: Vec<&str> = cmp
        .files
        .iter()
        .filter(|f| f.divergent)
        .map(|f| f.path.as_str())
        .collect();
    bench(task, "divergent", json!(divergent));
    bench(task, "agreed", json!(cmp.agreed));

    // "Escolher este" = a 1ª coluna, merge no checkout do usuário (integrate).
    let winner = a.entries[0].clone();
    let applied = apply_entry(&id, &winner.entry_id, Some("merge"), None, true, None).await;
    match &applied {
        Ok(r) => {
            bench(task, "integrate_ms", json!(r.took_ms));
            bench(task, "integrate_via", json!(r.updated));
            let got: BTreeSet<String> = compare::touches(&repo, &base, "HEAD")
                .await
                .unwrap()
                .into_keys()
                .collect();
            let want: Value = serde_json::from_str(
                &std::fs::read_to_string(dir.join(agents[0]["changes"].as_str().unwrap())).unwrap(),
            )
            .unwrap();
            bench(
                task,
                "integrated_is_winner",
                json!(got == change_paths(&want)),
            );
            bench(
                task,
                "checkout_clean_after",
                json!(git(&repo, &["status", "--porcelain=v1", "--untracked-files=no"]).is_empty()),
            );
        }
        Err(err) => bench(task, "integrate_error", json!(err)),
    }

    // "Remover os outros": worktrees e branches das perdedoras.
    let losers: Vec<ArenaEntry> = a.entries[1..].to_vec();
    let c = cleanup_core(&engine, &id, false, true).await.unwrap();
    bench(task, "cleanup_errors", json!(c.errors));
    let orphans = orphans(&repo, &losers, winner.worktree_path.as_deref()).await;
    bench(task, "orphans_after_remove_others", json!(orphans));
    // O trabalho das perdedoras continua nos refs da arena.
    let kept = losers
        .iter()
        .filter(|e| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args([
                    "rev-parse",
                    "--verify",
                    "--quiet",
                    &arena_ref(&id, &e.entry_id),
                ])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        })
        .count();
    bench(task, "loser_work_in_arena_refs", json!(kept));
    // Depois o vencedor também sai: sobra só o checkout do usuário.
    let _ = cleanup_core(&engine, &id, true, true).await.unwrap();
    let wts = git(&repo, &["worktree", "list", "--porcelain"])
        .lines()
        .filter(|l| l.starts_with("worktree "))
        .count();
    let branches = git(&repo, &["branch", "--list", "omniget/*"]);
    bench(task, "worktrees_after_all", json!(wts));
    bench(
        task,
        "omniget_branches_after_all",
        json!(branches.lines().filter(|l| !l.trim().is_empty()).count()),
    );
}

/// Sobras das perdedoras: worktree no `git worktree list`, pasta no disco,
/// branch, ou algo que o `worktree prune` ainda limparia.
async fn orphans(repo: &Path, losers: &[ArenaEntry], winner_wt: Option<&str>) -> Value {
    let listed: Vec<String> = git(repo, &["worktree", "list", "--porcelain"])
        .lines()
        .filter_map(|l| l.strip_prefix("worktree ").map(String::from))
        .collect();
    let canon = |p: &str| {
        std::fs::canonicalize(p)
            .map(|x| x.to_string_lossy().into_owned())
            .unwrap_or_else(|_| p.to_string())
    };
    let main = canon(&repo.to_string_lossy());
    let win = winner_wt.map(canon);
    let extra_listed = listed
        .iter()
        .filter(|p| canon(p) != main && Some(canon(p)) != win)
        .count();
    let dirs = losers
        .iter()
        .filter(|e| {
            e.worktree_path
                .as_deref()
                .is_some_and(|p| Path::new(p).exists())
        })
        .count();
    let branches = losers
        .iter()
        .filter_map(|e| e.branch.as_deref())
        .filter(|b| !git(repo, &["branch", "--list", b]).is_empty())
        .count();
    let prunable = git(repo, &["worktree", "prune", "--dry-run", "-v"])
        .lines()
        .filter(|l| !l.trim().is_empty())
        .count();
    json!({ "listed": extra_listed, "dirs": dirs, "branches": branches, "prunable": prunable,
            "total": extra_listed + dirs + branches + prunable })
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let t = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &t);
        } else {
            std::fs::copy(e.path(), &t).unwrap();
        }
    }
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bn11_three_agents_four_code_tasks() {
    let Ok(fixtures) = std::env::var("BN11_FIXTURES") else {
        eprintln!("BN11_FIXTURES not set: run scripts/llm/arena_eval.py");
        return;
    };
    let _data = test_data();
    let root = PathBuf::from(fixtures);
    let tasks: Vec<String> =
        serde_json::from_str(&std::fs::read_to_string(root.join("tasks.json")).unwrap()).unwrap();
    for task in tasks {
        let t0 = Instant::now();
        tokio::time::timeout(Duration::from_secs(240), one_task(&task, &root.join(&task)))
            .await
            .unwrap_or_else(|_| panic!("{task}: timed out"));
        bench(&task, "task_ms", json!(t0.elapsed().as_millis() as u64));
    }
}
