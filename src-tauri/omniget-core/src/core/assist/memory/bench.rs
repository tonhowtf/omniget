//! Recall benchmark in the spirit of LongMemEval: a personal memory of ~60
//! facts (Portuguese and English, with dates, entities, updates and
//! distractors) and questions of five kinds — paraphrase, entity, time,
//! knowledge update and multi-hop. Scores recall@5 and MRR for the old
//! keyword-only recall and for the hybrid recall with and without vectors.
//!
//! Needs the local model; downloads it (≈ 23 MB + the ONNX runtime) into a
//! throwaway data dir. Run with:
//!   OMNIGET_DATA_DIR=$(mktemp -d) cargo test -p omniget-core --lib \
//!     assist::memory::bench -- --ignored --nocapture

use std::collections::HashMap;

use super::super::db::AssistDb;
use super::retain::{store, Extracted, RetainInput};
use super::{recall, retrieve};

struct F {
    key: &'static str,
    text: &'static str,
    kind: &'static str,
    subject: Option<&'static str>,
    when: Option<&'static str>,
    entities: &'static [&'static str],
}

const fn f(
    key: &'static str,
    text: &'static str,
    kind: &'static str,
    when: Option<&'static str>,
    entities: &'static [&'static str],
) -> F {
    F {
        key,
        text,
        kind,
        subject: None,
        when,
        entities,
    }
}

/// Facts in the order they were said (the later ones supersede by subject).
fn facts() -> Vec<F> {
    let mut v = vec![
        f("city", "A pessoa mora em Recife, no bairro de Casa Forte.", "declared", None, &["Recife", "Casa Forte"]),
        f("dog", "A pessoa tem uma cachorra chamada Pipoca, uma vira-lata caramelo.", "declared", None, &["Pipoca"]),
        f("ana", "Ana é a irmã mais nova da pessoa e estuda medicina na UFPE.", "declared", None, &["Ana", "UFPE"]),
        f("ana_bday", "O aniversário da Ana é em 14 de agosto.", "declared", None, &["Ana"]),
        f("lactose", "A pessoa é intolerante à lactose e evita queijo e leite.", "declared", None, &[]),
        f("coffee", "The person drinks black coffee every morning before work.", "declared", None, &[]),
        f("marathon", "A pessoa correu a meia maratona do Recife em 2026-03-15 e terminou em 2h05.", "observation", Some("2026-03-15"), &["Recife"]),
        f("dune", "A pessoa assistiu Duna: Parte Dois no cinema em 2026-03-22 e achou a trilha sonora incrível.", "observation", Some("2026-03-22"), &["Duna: Parte Dois"]),
        f("lisbon", "The person travelled to Lisbon with Ana from 2025-12-20 to 2025-12-30.", "observation", Some("2025-12-20"), &["Lisbon", "Ana"]),
        f("pasteis", "In Lisbon the person loved the pastéis de nata at Manteigaria.", "observation", Some("2025-12-22"), &["Lisbon", "Manteigaria"]),
        f("rust", "A pessoa está aprendendo Rust para reescrever o backend do app dela.", "declared", None, &["Rust"]),
        f("omniget", "A pessoa mantém um app open source chamado OmniGet.", "declared", None, &["OmniGet"]),
        f("knee", "A pessoa machucou o joelho esquerdo jogando futebol em 2026-05-10.", "observation", Some("2026-05-10"), &[]),
        f("physio", "A pessoa começou fisioterapia duas vezes por semana com o Dr. Paulo.", "observation", Some("2026-05-20"), &["Dr. Paulo"]),
        f("guitar", "The person plays acoustic guitar, mostly bossa nova.", "declared", None, &[]),
        f("book", "A pessoa está lendo O Nome do Vento, de Patrick Rothfuss.", "observation", Some("2026-08-01"), &["O Nome do Vento", "Patrick Rothfuss"]),
        f("mom", "A mãe da pessoa se chama Lúcia e mora em Caruaru.", "declared", None, &["Lúcia", "Caruaru"]),
        f("mom_visit", "A pessoa visitou a mãe, Lúcia, em Caruaru no São João de 2026-06-24.", "observation", Some("2026-06-24"), &["Lúcia", "Caruaru"]),
        f("allergy", "The person is allergic to shrimp.", "declared", None, &[]),
        f("vegan_try", "A pessoa tentou ficar sem carne durante janeiro de 2026.", "observation", Some("2026-01"), &[]),
        f("car", "The person bought a used Honda Fit in 2025-09.", "observation", Some("2025-09"), &["Honda Fit"]),
        f("style", "A pessoa prefere respostas curtas, diretas e sem tabelas.", "procedural", None, &[]),
        f("editor", "The person uses Neovim as their main editor.", "declared", None, &["Neovim"]),
        f("bank", "A pessoa abriu conta no Nubank em 2024.", "observation", Some("2024"), &["Nubank"]),
        f("sister_exam", "Ana passou na prova de residência em 2026-02-10.", "observation", Some("2026-02-10"), &["Ana"]),
        f("concert", "The person saw Gilberto Gil live at Classic Hall on 2026-04-18.", "observation", Some("2026-04-18"), &["Gilberto Gil", "Classic Hall"]),
        f("plant", "A pessoa cuida de uma costela-de-adão na varanda.", "declared", None, &[]),
        f("sleep", "A pessoa costuma dormir depois da meia-noite.", "declared", None, &[]),
        f("language", "The person is studying Japanese on Duolingo, 200-day streak.", "observation", None, &["Duolingo"]),
        f("team", "A pessoa torce para o Sport Club do Recife.", "declared", None, &["Sport"]),
        // Distractors: near topics, other people.
        f("d1", "Bruno, colega de trabalho, mora em Olinda.", "observation", None, &["Bruno", "Olinda"]),
        f("d2", "Bruno has a cat named Mingau.", "observation", None, &["Bruno", "Mingau"]),
        f("d3", "A pessoa comentou que Recife estava muito quente em 2026-01-05.", "observation", Some("2026-01-05"), &["Recife"]),
        f("d4", "The person watched Oppenheimer at home in 2025-11-02 and found it too long.", "observation", Some("2025-11-02"), &["Oppenheimer"]),
        f("d5", "A pessoa pediu uma receita de bolo de rolo em 2026-06-10.", "observation", Some("2026-06-10"), &[]),
        f("d6", "Carla, prima da pessoa, mora em Lisboa desde 2020.", "observation", None, &["Carla", "Lisboa"]),
        f("d7", "The person's laptop is a MacBook Pro M5.", "declared", None, &["MacBook Pro"]),
        f("d8", "A pessoa assistiu a um jogo do Sport na Ilha do Retiro em 2026-03-29.", "observation", Some("2026-03-29"), &["Sport", "Ilha do Retiro"]),
        f("d9", "A pessoa comprou tênis novos de corrida em 2026-02-20.", "observation", Some("2026-02-20"), &[]),
        f("d10", "Dr. Paulo recomendou gelo e alongamento diário.", "observation", Some("2026-05-20"), &["Dr. Paulo"]),
    ];
    // Knowledge updates: same subject, later statement wins.
    let mut job_old = f(
        "job_old",
        "A pessoa trabalha como desenvolvedora na Accenture.",
        "declared",
        None,
        &["Accenture"],
    );
    job_old.subject = Some("job.employer");
    let mut job_new = f(
        "job_new",
        "A pessoa agora trabalha na CESAR como engenheira de software.",
        "declared",
        None,
        &["CESAR"],
    );
    job_new.subject = Some("job.employer");
    let mut phone_old = f(
        "phone_old",
        "The person's phone is an iPhone 12.",
        "declared",
        None,
        &["iPhone 12"],
    );
    phone_old.subject = Some("phone");
    let mut phone_new = f(
        "phone_new",
        "The person switched to a Pixel 9 phone.",
        "declared",
        None,
        &["Pixel 9"],
    );
    phone_new.subject = Some("phone");
    v.insert(5, job_old);
    v.push(job_new);
    v.insert(8, phone_old);
    v.push(phone_new);
    v
}

/// (question, gold keys, kind)
fn questions() -> Vec<(&'static str, &'static [&'static str], &'static str)> {
    vec![
        ("Em qual cidade eu moro?", &["city"], "paraphrase"),
        ("What is my dog's name?", &["dog"], "paraphrase"),
        (
            "Posso comer pizza com muito queijo?",
            &["lactose"],
            "paraphrase",
        ),
        ("What do I drink in the morning?", &["coffee"], "paraphrase"),
        ("Que instrumento eu toco?", &["guitar"], "paraphrase"),
        ("Which foods am I allergic to?", &["allergy"], "paraphrase"),
        (
            "Qual linguagem de programação estou estudando?",
            &["rust"],
            "paraphrase",
        ),
        ("What code editor do I use?", &["editor"], "paraphrase"),
        ("Que livro estou lendo agora?", &["book"], "paraphrase"),
        ("Pra qual time eu torço?", &["team"], "paraphrase"),
        (
            "O que você sabe sobre a Ana?",
            &["ana", "ana_bday", "sister_exam", "lisbon"],
            "entity",
        ),
        ("Quando é o aniversário da Ana?", &["ana_bday"], "entity"),
        ("Tell me about Lúcia", &["mom", "mom_visit"], "entity"),
        ("What did Dr. Paulo say?", &["d10", "physio"], "entity"),
        (
            "O que eu fiz em março de 2026?",
            &["marathon", "dune"],
            "temporal",
        ),
        (
            "What did I do in December 2025?",
            &["lisbon", "pasteis"],
            "temporal",
        ),
        (
            "O que aconteceu em maio de 2026?",
            &["knee", "physio"],
            "temporal",
        ),
        (
            "Which concert did I go to in April 2026?",
            &["concert"],
            "temporal",
        ),
        ("O que eu fiz em junho de 2026?", &["mom_visit"], "temporal"),
        ("Where do I work now?", &["job_new"], "update"),
        ("Qual é o meu celular?", &["phone_new"], "update"),
        ("Onde eu trabalho?", &["job_new"], "update"),
        (
            "Por que estou fazendo fisioterapia?",
            &["knee", "physio"],
            "multihop",
        ),
        (
            "What did I eat on the trip with my sister?",
            &["pasteis"],
            "multihop",
        ),
        (
            "Que filme vi no cinema depois da meia maratona?",
            &["dune"],
            "multihop",
        ),
    ]
}

struct Score {
    hit5: usize,
    rr: f32,
    n: usize,
}

impl Score {
    fn add(&mut self, ranked: &[String], gold: &[String]) {
        self.n += 1;
        if let Some(pos) = ranked.iter().position(|id| gold.contains(id)) {
            self.rr += 1.0 / (pos as f32 + 1.0);
            if pos < 5 {
                self.hit5 += 1;
            }
        }
    }
    fn line(&self) -> String {
        format!(
            "R@5 {:>5.1}%  MRR {:.3}  (n={})",
            100.0 * self.hit5 as f32 / self.n.max(1) as f32,
            self.rr / self.n.max(1) as f32,
            self.n
        )
    }
}

fn build(db: &AssistDb, now: i64) -> HashMap<&'static str, String> {
    let ctx = super::super::ctx::direct("bench-bot", Some("bench-conv"));
    let mut ids: HashMap<&str, String> = HashMap::new();
    for (i, fact) in facts().iter().enumerate() {
        let ex = Extracted {
            text: fact.text.into(),
            kind: fact.kind.into(),
            about: Some("person".into()),
            subject: fact.subject.map(str::to_string),
            quote: Some(fact.text.into()),
            occurred_start: fact.when.map(str::to_string),
            entities: fact
                .entities
                .iter()
                .map(|e| serde_json::json!({"name": e, "kind": "thing"}))
                .collect(),
            ..Default::default()
        };
        let at = now - (60 - i as i64) * 86_400_000;
        let input = RetainInput {
            person: fact.text.into(),
            other: String::new(),
            event_ms: at,
            source_kind: "conversation".into(),
            source_id: Some(format!("m{i}")),
            context: None,
        };
        let r = store(db, &ctx, &input, &[ex], at).unwrap();
        if let Some(id) = r.created.first().or(r.reinforced.first()) {
            ids.insert(fact.key, id.clone());
        }
    }
    // Causal link knee → physio, as extraction would give within one turn.
    db.with(|c| {
        super::graph::add_link(c, &ids["physio"], &ids["knee"], "causal", 1.0)?;
        super::graph::add_link(c, &ids["knee"], &ids["physio"], "causal", 0.8)
    })
    .unwrap();
    ids
}

fn eval(label: &str, keyword_only: bool, verbose: bool) {
    let db = AssistDb::open_in_memory().unwrap();
    let now = super::super::now_ms();
    let t0 = std::time::Instant::now();
    let ids = build(&db, now);
    let stored = t0.elapsed();
    let ctx = super::super::ctx::direct("bench-bot", Some("bench-conv"));
    let mut by: HashMap<&str, Score> = HashMap::new();
    let mut lat = Vec::new();
    for (q, gold, kind) in questions() {
        let gold: Vec<String> = gold.iter().filter_map(|k| ids.get(k).cloned()).collect();
        let t = std::time::Instant::now();
        let ranked: Vec<String> = if keyword_only {
            recall(&db, &ctx, q, 10, now)
                .unwrap()
                .into_iter()
                .map(|r| r.id)
                .collect()
        } else {
            let qvec = retrieve::embed_query(q);
            let h = retrieve::hybrid(&db, &ctx, q, qvec.as_deref(), 10, now).unwrap();
            let miss = !h.hits.iter().take(5).any(|x| gold.contains(&x.record.id));
            if miss {
                let top: Vec<String> = h
                    .hits
                    .iter()
                    .take(3)
                    .map(|x| {
                        format!(
                            "{} [{}{}]",
                            x.record.content.chars().take(40).collect::<String>(),
                            x.why.arms.join("+"),
                            x.why
                                .rerank
                                .map(|r| format!(" ce={r:.2}"))
                                .unwrap_or_default()
                        )
                    })
                    .collect();
                let pool = h.hits.iter().position(|x| gold.contains(&x.record.id));
                println!(
                    "  MISS {kind} | {q} (gold at {pool:?})\n       → {}",
                    top.join(" · ")
                );
            }
            if verbose && false {
                let top: Vec<String> = h
                    .hits
                    .iter()
                    .take(2)
                    .map(|x| {
                        format!(
                            "{} [{}]",
                            x.record.content.chars().take(44).collect::<String>(),
                            x.why.arms.join("+")
                        )
                    })
                    .collect();
                println!("{kind:>10} | {q}\n           → {}", top.join(" · "));
            }
            h.hits.into_iter().map(|x| x.record.id).collect()
        };
        lat.push(t.elapsed());
        for k in [kind, "all"] {
            by.entry(k)
                .or_insert(Score {
                    hit5: 0,
                    rr: 0.0,
                    n: 0,
                })
                .add(&ranked, &gold);
        }
    }
    lat.sort();
    println!(
        "\n== {label}  (store {:?}, query p50 {:?} p95 {:?})",
        stored,
        lat[lat.len() / 2],
        lat[lat.len() * 95 / 100]
    );
    for k in [
        "paraphrase",
        "entity",
        "temporal",
        "update",
        "multihop",
        "all",
    ] {
        println!("   {k:>10}  {}", by[k].line());
    }
}

#[tokio::test]
#[ignore = "downloads the local models (≈ 280 MB)"]
async fn recall_benchmark() {
    use super::retrieve::bench_knobs::{MINILM, NO_RERANK};
    use std::sync::atomic::Ordering::Relaxed;
    let progress = crate::core::tools::noop_progress();
    if !crate::core::onnxrt::is_installed() {
        crate::core::onnxrt::install_runtime(None, &progress)
            .await
            .expect("onnx runtime");
    }
    crate::core::onnxrt::init().expect("onnx runtime init");
    crate::core::embed::ensure_model(&progress)
        .await
        .expect("minilm");
    use crate::core::embed::xlmr::{ensure, Model};
    ensure(Model::E5, &progress).await.expect("e5");
    ensure(Model::Reranker, &progress).await.expect("reranker");

    eval("keyword only (the memory before this work)", true, false);
    MINILM.store(true, Relaxed);
    NO_RERANK.store(true, Relaxed);
    eval("hybrid, English MiniLM, no reranker", false, false);
    MINILM.store(false, Relaxed);
    eval("hybrid, multilingual e5, no reranker", false, false);
    NO_RERANK.store(false, Relaxed);
    eval("hybrid, multilingual e5 + cross-encoder", false, true);
}

#[tokio::test]
#[ignore = "needs the local models already downloaded"]
async fn similarity_calibration() {
    crate::core::onnxrt::init().expect("onnx runtime init");
    let texts: Vec<&str> = facts().iter().map(|f| f.text).collect();
    let dupes = [
        (
            "A pessoa mora em Recife, no bairro de Casa Forte.",
            "The person lives in Recife, in the Casa Forte neighbourhood.",
        ),
        (
            "A pessoa é intolerante à lactose e evita queijo e leite.",
            "A pessoa não pode tomar leite, tem intolerância à lactose.",
        ),
        (
            "The person drinks black coffee every morning before work.",
            "Every morning the person has a black coffee before work.",
        ),
        (
            "A pessoa tem uma cachorra chamada Pipoca, uma vira-lata caramelo.",
            "Pipoca é a cachorra vira-lata caramelo da pessoa.",
        ),
    ];
    for (label, e5) in [("minilm", false), ("e5", true)] {
        let embed = |t: &[&str], q: bool| -> Vec<Vec<f32>> {
            if e5 {
                crate::core::embed::xlmr::embed(t, q).unwrap()
            } else {
                crate::core::embed::embed_batch(t)
                    .unwrap()
                    .into_iter()
                    .map(|d| d.to_vec())
                    .collect()
            }
        };
        let v = embed(&texts, false);
        let mut pairs = Vec::new();
        for i in 0..v.len() {
            for j in i + 1..v.len() {
                pairs.push(super::graph::dot(&v[i], &v[j]));
            }
        }
        pairs.sort_by(|a, b| a.total_cmp(b));
        let q = |p: f32| pairs[((pairs.len() - 1) as f32 * p) as usize];
        let d: Vec<f32> = dupes
            .iter()
            .map(|(a, b)| {
                let x = embed(&[a, b], false);
                super::graph::dot(&x[0], &x[1])
            })
            .collect();
        let qv = embed(&["query: Em qual cidade eu moro?"[7..].trim()], true);
        let rel = super::graph::dot(&qv[0], &v[0]);
        let unrel: f32 = v
            .iter()
            .skip(1)
            .map(|x| super::graph::dot(&qv[0], x))
            .fold(0.0, f32::max);
        println!(
            "{label}: fact pairs p50 {:.3} p90 {:.3} p99 {:.3} max {:.3} | paraphrase dupes {:?} | query→gold {:.3} best other {:.3}",
            q(0.5), q(0.9), q(0.99), q(1.0), d, rel, unrel
        );
    }
}

#[tokio::test]
#[ignore = "needs the local models already downloaded"]
async fn why_misses() {
    crate::core::onnxrt::init().expect("onnx runtime init");
    let all: Vec<&str> = facts().iter().map(|f| f.text).collect();
    let strip = |t: &str| -> String {
        for p in [
            "A pessoa ",
            "The person's ",
            "The person ",
            "A mãe da pessoa ",
            "O aniversário da ",
        ] {
            if let Some(r) = t.strip_prefix(p) {
                if p.starts_with("A pessoa") || p.starts_with("The person") {
                    return r.to_string();
                }
            }
        }
        t.to_string()
    };
    let stripped: Vec<String> = all.iter().map(|t| strip(t)).collect();
    let sref: Vec<&str> = stripped.iter().map(String::as_str).collect();
    let v = if std::env::var("STRIP").is_ok() {
        crate::core::embed::xlmr::embed(&sref, false).unwrap()
    } else {
        crate::core::embed::xlmr::embed(&all, false).unwrap()
    };
    for (q, gold) in [
        (
            "Que instrumento eu toco?",
            "The person plays acoustic guitar, mostly bossa nova.",
        ),
        (
            "Qual é o meu celular?",
            "The person switched to a Pixel 9 phone.",
        ),
        (
            "What did I eat on the trip with my sister?",
            "In Lisbon the person loved the pastéis de nata at Manteigaria.",
        ),
    ] {
        let qv = &crate::core::embed::xlmr::embed(&[q], true).unwrap()[0];
        let mut sims: Vec<(f32, &str)> = v
            .iter()
            .zip(&all)
            .map(|(x, t)| (super::graph::dot(qv, x), *t))
            .collect();
        sims.sort_by(|a, b| b.0.total_cmp(&a.0));
        let rank = sims.iter().position(|s| s.1 == gold);
        let ce = crate::core::embed::xlmr::rerank(q, &[gold, sims[0].1]).unwrap();
        println!(
            "{q}\n  gold sim rank {rank:?} ({:.3}), top {:.3} {:?}\n  ce gold {:.3} vs top {:.3}",
            sims[rank.unwrap()].0,
            sims[0].0,
            &sims[0].1[..30.min(sims[0].1.len())],
            ce[0],
            ce[1]
        );
    }
}

// ── LoCoMo, end to end (the same answer and judge prompts as the Hindsight
// run in estudos/91-hindsight/eval/) ─────────────────────────────────────

pub const ANSWER_SYSTEM: &str = "You answer a question about two people's long chat history using ONLY the memories given. Each memory has the date it happened or was said; resolve relative time with those dates. Give a short answer: a few words or one sentence. If the memories do not contain the answer, give your best guess from them.";

pub const JUDGE_SYSTEM: &str = "Your task is to label an answer to a question as CORRECT or WRONG. You get a question, a gold (ground truth) answer and a generated answer. Be generous: if the generated answer touches the same topic and contains the key information of the gold answer, it is CORRECT. For time questions, the same date or period in another format is CORRECT. Return ONLY JSON {\"label\":\"CORRECT\"} or {\"label\":\"WRONG\"}.";

fn session_ms(s: &str) -> i64 {
    use chrono::{Local, NaiveDateTime, TimeZone};
    let s = s.trim();
    for f in [
        "%I:%M %p on %d %B, %Y",
        "%I:%M %P on %d %B, %Y",
        "%H:%M on %d %B, %Y",
    ] {
        if let Ok(d) = NaiveDateTime::parse_from_str(s, f) {
            return Local
                .from_local_datetime(&d)
                .earliest()
                .map(|d| d.timestamp_millis())
                .unwrap_or(0);
        }
    }
    panic!("unparsed date {s}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "calls the Claude CLI; LOCOMO_PATH=<locomo10.json>"]
async fn locomo_eval() {
    use futures::StreamExt;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    crate::core::onnxrt::init().expect("onnx runtime init");
    let path = std::env::var("LOCOMO_PATH").expect("LOCOMO_PATH");
    let every: usize = std::env::var("LOCOMO_EVERY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2);
    let data: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let conv = &data[0]["conversation"];
    let (a, b) = (
        conv["speaker_a"].as_str().unwrap(),
        conv["speaker_b"].as_str().unwrap(),
    );
    let calls = Arc::new(AtomicUsize::new(0));
    let gate = Arc::new(tokio::sync::Semaphore::new(4));
    let llm: super::retain::Llm = {
        let calls = calls.clone();
        let gate = gate.clone();
        Arc::new(move |sys: String, user: String| {
            let calls = calls.clone();
            let gate = gate.clone();
            Box::pin(async move {
                let _p = gate.acquire().await.unwrap();
                calls.fetch_add(1, Ordering::Relaxed);
                super::retain::claude_cli(&sys, &user)
                    .await
                    .unwrap_or_else(|| Err("no claude".into()))
            })
        })
    };

    // LOCOMO_DB=<file> keeps the ingested memory: a second run with the same
    // file skips retain and consolidation and only answers.
    let reuse = std::env::var("LOCOMO_DB").ok();
    let reused = reuse
        .as_ref()
        .is_some_and(|p| std::path::Path::new(p).exists());
    let db = std::sync::Arc::new(match &reuse {
        Some(p) => AssistDb::open(std::path::Path::new(p)).unwrap(),
        None => AssistDb::open_in_memory().unwrap(),
    });
    let ctx = super::super::ctx::direct("locomo", Some("locomo"));
    let mut sessions = Vec::new();
    for i in 1..=40 {
        let key = format!("session_{i}");
        let Some(turns) = conv[&key].as_array() else {
            continue;
        };
        let when = session_ms(conv[&format!("{key}_date_time")].as_str().unwrap());
        let text: String = turns
            .iter()
            .map(|t| {
                let mut line = format!(
                    "{}: {}",
                    t["speaker"].as_str().unwrap_or(""),
                    t["text"].as_str().unwrap_or("")
                );
                if let Some(c) = t["blip_caption"].as_str() {
                    line.push_str(&format!(" [shares a photo: {c}]"));
                }
                line
            })
            .collect::<Vec<_>>()
            .join("\n");
        sessions.push((i, when, text));
    }
    let last = sessions.iter().map(|s| s.1).max().unwrap();
    let t0 = std::time::Instant::now();
    let todo = if reused { Vec::new() } else { sessions.clone() };
    let reports: Vec<_> = futures::stream::iter(todo)
        .map(|(i, when, text)| {
            let db = db.clone();
            let ctx = ctx.clone();
            let llm = llm.clone();
            async move {
                let input = super::retain::RetainInput {
                    person: text,
                    other: String::new(),
                    event_ms: when,
                    source_kind: "conversation".into(),
                    source_id: Some(format!("session_{i}")),
                    context: Some(format!("a chat between {a} and {b}")),
                };
                super::retain::retain(db, ctx, input, llm).await
            }
        })
        .buffer_unordered(4)
        .collect()
        .await;
    let retain_calls = calls.load(Ordering::Relaxed);
    let created: usize = reports
        .iter()
        .filter_map(|r| r.as_ref().ok())
        .map(|r| r.created.len())
        .sum();
    let failed = reports.iter().filter(|r| r.is_err()).count();
    println!(
        "retain: {} sessions, {created} facts, {failed} failed, {retain_calls} model calls, {:?}",
        sessions.len(),
        t0.elapsed()
    );
    let t1 = std::time::Instant::now();
    for _ in 0..if reused { 0 } else { 30 } {
        let n = super::consolidate::run(db.clone(), llm.clone())
            .await
            .unwrap_or(0);
        if n == 0 {
            break;
        }
    }
    let consolidate_calls = calls.load(Ordering::Relaxed) - retain_calls;
    println!(
        "consolidate: {consolidate_calls} model calls, {:?}",
        t1.elapsed()
    );

    let qa: Vec<serde_json::Value> = data[0]["qa"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|q| q["category"].as_u64() != Some(5))
        .step_by(every)
        .cloned()
        .collect();
    let results: Vec<serde_json::Value> = futures::stream::iter(qa)
        .map(|q| {
            let db = db.clone();
            let ctx = ctx.clone();
            let llm = llm.clone();
            async move {
                let question = q["question"].as_str().unwrap().to_string();
                let gold = q["answer"].to_string();
                let t = std::time::Instant::now();
                let q2 = question.clone();
                let (probe, notes) = {
                    let db = db.clone();
                    let ctx = ctx.clone();
                    tokio::task::spawn_blocking(move || {
                        let qv = retrieve::embed_query(&q2);
                        let k: usize = std::env::var("LOCOMO_K").ok().and_then(|v| v.parse().ok()).unwrap_or(20);
                        let p = retrieve::hybrid(&db, &ctx, &q2, qv.as_deref(), k, last + 86_400_000).unwrap();
                        let n = retrieve::notes_for(&db, &ctx, &q2).unwrap_or_default();
                        (p, n)
                    })
                    .await
                    .unwrap()
                };
                let recall_ms = t.elapsed().as_millis() as u64;
                let mut mem = String::new();
                for h in &probe.hits {
                    let when = h.why.occurred_start.unwrap_or(h.record.source.at_ms);
                    mem.push_str(&format!("- [{}] {}\n", super::search::day(when), h.record.content));
                }
                for (n, t) in &notes {
                    mem.push_str(&format!("- About {n}: {}\n", t.replace('\n', " ")));
                }
                let today = super::search::day(last + 86_400_000);
                let answer = llm(ANSWER_SYSTEM.into(), format!("Today: {today}\n\nMEMORIES:\n{mem}\nQUESTION: {question}\nANSWER:"))
                    .await
                    .unwrap_or_default();
                let verdict = llm(
                    JUDGE_SYSTEM.into(),
                    format!("QUESTION: {question}\nGOLD ANSWER: {gold}\nGENERATED ANSWER: {answer}"),
                )
                .await
                .unwrap_or_default();
                let correct = verdict.contains("CORRECT") && !verdict.contains("WRONG");
                serde_json::json!({"category": q["category"], "question": question, "gold": gold, "answer": answer, "correct": correct, "recall_ms": recall_ms})
            }
        })
        .buffered(4)
        .collect()
        .await;
    let mut by: std::collections::BTreeMap<u64, (usize, usize)> = Default::default();
    for r in &results {
        let e = by.entry(r["category"].as_u64().unwrap()).or_default();
        e.1 += 1;
        if r["correct"].as_bool().unwrap() {
            e.0 += 1;
        }
    }
    let ok: usize = by.values().map(|v| v.0).sum();
    let mut lat: Vec<u64> = results
        .iter()
        .map(|r| r["recall_ms"].as_u64().unwrap())
        .collect();
    lat.sort();
    println!(
        "\nLoCoMo conv-26, {} questions: {:.1}% correct",
        results.len(),
        100.0 * ok as f32 / results.len() as f32
    );
    for (c, (k, n)) in &by {
        let name = match c {
            1 => "multi-hop",
            2 => "temporal",
            3 => "open-domain",
            4 => "single-hop",
            _ => "?",
        };
        println!("  {name:>11}: {k}/{n}");
    }
    println!(
        "recall p50 {} ms p95 {} ms; model calls total {}",
        lat[lat.len() / 2],
        lat[lat.len() * 95 / 100],
        calls.load(Ordering::Relaxed)
    );
    if let Ok(out) = std::env::var("LOCOMO_OUT") {
        std::fs::write(out, serde_json::to_string_pretty(&results).unwrap()).unwrap();
    }
}
