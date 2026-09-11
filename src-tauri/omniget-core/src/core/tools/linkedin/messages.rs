//! Visualizador do `messages.csv`: agrupa por conversa, ordena por data,
//! descobre quem e "voce" e gera uma pagina HTML local navegavel, um Markdown
//! por conversa e o JSON cru. Sem rede: e o proprio export no disco.

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::csv::parse_date;
use super::source::Source;
use super::{json_for_script, top, Count};
use crate::core::tools::{report, sanitize_name, ProgressFn};

const ID: &str = "li-messages";

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Options {
    pub path: String,
    /// Nome de quem exportou. Vazio: a gente deduz.
    pub me: Option<String>,
    /// Busca livre em participante, assunto e texto.
    pub query: Option<String>,
    /// Quantas conversas voltam com as mensagens para a UI.
    pub limit: Option<usize>,
    pub out_dir: Option<String>,
    pub export_html: Option<bool>,
    pub export_markdown: Option<bool>,
    pub export_json: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Message {
    pub from: String,
    pub to: String,
    pub date: String,
    pub subject: String,
    pub content: String,
    /// Verdadeiro quando quem mandou e voce.
    pub mine: bool,
    /// Mensagem sem texto (costuma ser anexo ou convite).
    pub empty: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub participants: Vec<String>,
    pub count: usize,
    pub sent: usize,
    pub received: usize,
    pub first: String,
    pub last: String,
    pub messages: Vec<Message>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MessagesResult {
    pub path: String,
    pub me: String,
    pub me_guessed: bool,
    pub total: usize,
    pub sent: usize,
    pub received: usize,
    pub empty: usize,
    pub conversations_total: usize,
    pub matched: usize,
    pub conversations: Vec<Conversation>,
    pub top_contacts: Vec<Count>,
    pub by_year: Vec<super::Bucket>,
    pub exports: Vec<String>,
}

/// Data em `YYYY-MM-DD HH:MM`, quando der. Sem data, devolve o cru.
fn stamp(raw: &str) -> String {
    match parse_date(raw) {
        Some(d) => {
            let time = raw
                .split_whitespace()
                .nth(1)
                .filter(|t| t.contains(':'))
                .map(|t| t.chars().take(5).collect::<String>())
                .unwrap_or_default();
            if time.is_empty() {
                d.iso()
            } else {
                format!("{} {}", d.iso(), time)
            }
        }
        None => raw.trim().to_string(),
    }
}

/// Quem aparece como remetente em mais conversas distintas e "voce".
pub fn guess_me(rows: &[(String, String, String)]) -> String {
    let mut convs: HashMap<&str, HashSet<&str>> = HashMap::new();
    let mut totals: HashMap<&str, usize> = HashMap::new();
    for (conv, from, _) in rows {
        if from.trim().is_empty() {
            continue;
        }
        convs.entry(from).or_default().insert(conv);
        *totals.entry(from).or_insert(0) += 1;
    }
    let mut best: Option<(&str, usize, usize)> = None;
    for (name, set) in &convs {
        let cand = (*name, set.len(), totals.get(name).copied().unwrap_or(0));
        let better = match best {
            None => true,
            Some((bn, bc, bt)) => {
                (cand.1, cand.2) > (bc, bt) || ((cand.1, cand.2) == (bc, bt) && cand.0 < bn)
            }
        };
        if better {
            best = Some(cand);
        }
    }
    best.map(|b| b.0.to_string()).unwrap_or_default()
}

fn split_names(raw: &str) -> Vec<String> {
    raw.split(['|', ';'])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

pub fn load(src: &Source, me_hint: Option<&str>) -> (String, bool, Vec<Conversation>) {
    let t = src.table_or_empty(&["messages.csv", "Messages.csv"]);
    let mut raw: Vec<(String, String, String, String, String, String)> =
        Vec::with_capacity(t.len());
    for row in &t.rows {
        let conv = t
            .get(row, &["CONVERSATION ID", "ConversationId"])
            .to_string();
        let from = t.get(row, &["FROM", "Sender", "From"]).to_string();
        let to = t.get(row, &["TO", "Recipient", "To"]).to_string();
        let date = t.get(row, &["DATE", "Date", "SentAt"]).to_string();
        let subject = t.get(row, &["SUBJECT", "Subject"]).to_string();
        let content = t.get(row, &["CONTENT", "Content", "Message"]).to_string();
        if from.is_empty() && to.is_empty() && content.is_empty() {
            continue;
        }
        // Sem CONVERSATION ID, a dupla de participantes vira a chave.
        let key = if conv.is_empty() {
            let mut names = split_names(&from);
            names.extend(split_names(&to));
            names.sort();
            names.dedup();
            names.join(" & ")
        } else {
            conv
        };
        raw.push((key, from, to, date, subject, content));
    }

    let pairs: Vec<(String, String, String)> = raw
        .iter()
        .map(|r| (r.0.clone(), r.1.clone(), r.3.clone()))
        .collect();
    let guessed = guess_me(&pairs);
    let me = me_hint
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| guessed.clone());
    let me_guessed = me_hint.map(str::trim).unwrap_or("").is_empty();

    let mut order: Vec<String> = Vec::new();
    let mut map: HashMap<String, Conversation> = HashMap::new();
    for (key, from, to, date, subject, content) in raw {
        let mine = !me.is_empty() && from.trim().eq_ignore_ascii_case(me.trim());
        let msg = Message {
            empty: content.trim().is_empty(),
            mine,
            from,
            to,
            date: stamp(&date),
            subject,
            content,
        };
        let conv = map.entry(key.clone()).or_insert_with(|| {
            order.push(key.clone());
            Conversation {
                id: key.clone(),
                title: String::new(),
                participants: Vec::new(),
                count: 0,
                sent: 0,
                received: 0,
                first: String::new(),
                last: String::new(),
                messages: Vec::new(),
            }
        });
        for n in split_names(&msg.from)
            .into_iter()
            .chain(split_names(&msg.to))
        {
            if !conv.participants.iter().any(|p| p.eq_ignore_ascii_case(&n)) {
                conv.participants.push(n);
            }
        }
        conv.messages.push(msg);
    }

    let mut convs: Vec<Conversation> = order.into_iter().filter_map(|k| map.remove(&k)).collect();
    for c in &mut convs {
        c.messages.sort_by(|a, b| a.date.cmp(&b.date));
        c.count = c.messages.len();
        c.sent = c.messages.iter().filter(|m| m.mine).count();
        c.received = c.count - c.sent;
        c.first = c
            .messages
            .first()
            .map(|m| m.date.clone())
            .unwrap_or_default();
        c.last = c
            .messages
            .last()
            .map(|m| m.date.clone())
            .unwrap_or_default();
        let others: Vec<String> = c
            .participants
            .iter()
            .filter(|p| !p.eq_ignore_ascii_case(me.trim()))
            .cloned()
            .collect();
        c.title = if others.is_empty() {
            c.participants.join(", ")
        } else {
            others.join(", ")
        };
    }
    convs.sort_by(|a, b| b.last.cmp(&a.last).then_with(|| a.title.cmp(&b.title)));
    (me, me_guessed, convs)
}

fn markdown(c: &Conversation) -> String {
    let mut s = format!("# {}\n\n", c.title);
    s.push_str(&format!("{} · {} → {}\n\n", c.count, c.first, c.last));
    for m in &c.messages {
        s.push_str(&format!("**{}** · {}\n\n", m.from, m.date));
        if !m.subject.trim().is_empty() {
            s.push_str(&format!("*{}*\n\n", m.subject.trim()));
        }
        if m.empty {
            s.push_str("—\n\n");
        } else {
            for line in m.content.lines() {
                s.push_str(&format!("> {}\n", line));
            }
            s.push('\n');
        }
    }
    s
}

const HTML_HEAD: &str = r#"<meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>LinkedIn</title><style>
:root{color-scheme:light dark;--bg:#fff;--fg:#1d1d1f;--dim:#6e6e73;--line:#e5e5ea;--mine:#0a66c2;--card:#f5f5f7}
@media(prefers-color-scheme:dark){:root{--bg:#1c1c1e;--fg:#f5f5f7;--dim:#98989d;--line:#3a3a3c;--card:#2c2c2e}}
*{box-sizing:border-box}body{margin:0;font:14px/1.5 -apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,sans-serif;background:var(--bg);color:var(--fg)}
.wrap{display:flex;height:100vh}
.side{width:300px;flex:none;border-right:1px solid var(--line);display:flex;flex-direction:column}
.side input{margin:12px;padding:8px 10px;border:1px solid var(--line);border-radius:8px;background:var(--card);color:var(--fg);font:inherit}
.list{overflow:auto;flex:1}
.item{padding:10px 14px;border-bottom:1px solid var(--line);cursor:pointer}
.item.on{background:var(--card)}
.item b{display:block;font-weight:600}
.item span{color:var(--dim);font-size:12px}
.main{flex:1;overflow:auto;padding:20px}
.msg{max-width:640px;margin:0 0 14px;padding:10px 14px;border-radius:14px;background:var(--card);white-space:pre-wrap;overflow-wrap:anywhere}
.msg.mine{background:var(--mine);color:#fff;margin-left:auto}
.meta{font-size:12px;color:var(--dim);margin-bottom:4px}
.msg.mine .meta{color:#dbeafe}
.head{font-size:20px;font-weight:600;margin:0 0 16px}
@media(max-width:700px){.wrap{flex-direction:column;height:auto}.side{width:100%;border-right:0;border-bottom:1px solid var(--line)}.list{max-height:40vh}}
</style>"#;

const HTML_TAIL: &str = r#"<div class="wrap"><div class="side"><input id="q" type="search" placeholder="search"><div class="list" id="list"></div></div><div class="main"><h1 class="head" id="head"></h1><div id="pane"></div></div></div>
<script>
const D=window.__LI__,L=document.getElementById("list"),P=document.getElementById("pane"),H=document.getElementById("head"),Q=document.getElementById("q");
let cur=0;
function esc(s){const d=document.createElement("div");d.textContent=s;return d.innerHTML}
function draw(i){cur=i;const c=D.conversations[i];if(!c)return;H.textContent=c.title;
P.innerHTML=c.messages.map(m=>'<div class="msg'+(m.mine?" mine":"")+'"><div class="meta">'+esc(m.from)+" · "+esc(m.date)+(m.subject?" · "+esc(m.subject):"")+'</div>'+(m.empty?"—":esc(m.content))+"</div>").join("");
[...L.children].forEach((el,k)=>el.classList.toggle("on",k===i));}
function list(){const q=Q.value.toLowerCase();
L.innerHTML=D.conversations.map((c,i)=>{const hit=!q||c.title.toLowerCase().includes(q)||c.messages.some(m=>(m.content||"").toLowerCase().includes(q));
return hit?'<div class="item" data-i="'+i+'"><b>'+esc(c.title)+"</b><span>"+c.count+" · "+esc(c.last)+"</span></div>":""}).join("");
[...L.children].forEach(el=>el.onclick=()=>draw(+el.dataset.i));
const first=L.firstElementChild;if(first)draw(+first.dataset.i);}
Q.oninput=list;list();
</script>"#;

fn html(res: &MessagesResult, convs: &[Conversation]) -> String {
    let data = serde_json::json!({ "me": res.me, "conversations": convs });
    format!(
        "{}\n<script>window.__LI__={};</script>\n{}",
        HTML_HEAD,
        json_for_script(&data),
        HTML_TAIL
    )
}

fn hit(c: &Conversation, q: &str) -> bool {
    if q.is_empty() {
        return true;
    }
    if c.title.to_lowercase().contains(q) {
        return true;
    }
    c.messages.iter().any(|m| {
        m.content.to_lowercase().contains(q)
            || m.subject.to_lowercase().contains(q)
            || m.from.to_lowercase().contains(q)
    })
}

pub fn run(opts: &Options, p: &ProgressFn) -> Result<MessagesResult> {
    report(p, ID, "started", 0, Some(3), None);
    let opened = Source::open(&opts.path)?;
    let (me, me_guessed, all) = load(&opened.source, opts.me.as_deref());
    report(
        p,
        ID,
        "progress",
        1,
        Some(3),
        Some(format!("{}", all.len())),
    );

    let q = opts.query.as_deref().unwrap_or("").trim().to_lowercase();
    let matched: Vec<Conversation> = all.iter().filter(|c| hit(c, &q)).cloned().collect();

    let mut total = 0usize;
    let mut sent = 0usize;
    let mut empty = 0usize;
    let mut contacts: HashMap<String, usize> = HashMap::new();
    let mut by_year: HashMap<String, usize> = HashMap::new();
    for c in &all {
        total += c.count;
        sent += c.sent;
        empty += c.messages.iter().filter(|m| m.empty).count();
        for name in &c.participants {
            if !name.eq_ignore_ascii_case(me.trim()) {
                *contacts.entry(name.clone()).or_insert(0) += c.count;
            }
        }
        for m in &c.messages {
            if m.date.len() >= 4 {
                *by_year.entry(m.date[..4].to_string()).or_insert(0) += 1;
            }
        }
    }
    report(p, ID, "progress", 2, Some(3), None);

    let limit = opts.limit.unwrap_or(200).max(1);
    let mut res = MessagesResult {
        path: opts.path.clone(),
        me,
        me_guessed,
        total,
        sent,
        received: total - sent,
        empty,
        conversations_total: all.len(),
        matched: matched.len(),
        conversations: matched.iter().take(limit).cloned().collect(),
        top_contacts: top(&contacts, 30),
        by_year: super::series(&by_year),
        exports: Vec::new(),
    };

    if opts.export_html.unwrap_or(false)
        || opts.export_markdown.unwrap_or(false)
        || opts.export_json.unwrap_or(false)
    {
        let dir = super::out_dir(opts.out_dir.as_deref());
        if opts.export_html.unwrap_or(false) {
            let body = html(&res, &matched);
            res.exports
                .push(super::write_out(&dir, "mensagens.html", &body)?);
        }
        if opts.export_json.unwrap_or(false) {
            let body = serde_json::to_string_pretty(&matched)?;
            res.exports
                .push(super::write_out(&dir, "mensagens.json", &body)?);
        }
        if opts.export_markdown.unwrap_or(false) {
            let sub = dir.join("conversas");
            std::fs::create_dir_all(&sub)?;
            for (i, c) in matched.iter().enumerate() {
                let name = format!(
                    "{:04}-{}.md",
                    i + 1,
                    sanitize_name(&c.title).chars().take(60).collect::<String>()
                );
                let _ = std::fs::write(sub.join(name), markdown(c));
            }
            res.exports.push(sub.to_string_lossy().to_string());
        }
    }
    report(p, ID, "done", 3, Some(3), None);
    Ok(res)
}

#[cfg(test)]
mod tests {
    use super::super::source::fixture;
    use super::*;
    use crate::core::tools::noop_progress;

    fn opts(dir: &std::path::Path) -> Options {
        Options {
            path: dir.to_string_lossy().to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn agrupa_duas_conversas_e_acha_voce() {
        let dir = fixture::dir();
        let r = run(&opts(&dir), &noop_progress()).expect("roda");
        assert_eq!(r.conversations_total, 2);
        assert_eq!(r.total, 5);
        assert_eq!(r.me, "Tonho Dev");
        assert!(r.me_guessed);
        assert_eq!(r.sent, 2);
        assert_eq!(r.received, 3);
        assert_eq!(r.empty, 1);
    }

    #[test]
    fn conversa_vem_ordenada_e_com_quebra_de_linha() {
        let dir = fixture::dir();
        let r = run(&opts(&dir), &noop_progress()).expect("roda");
        let c = r
            .conversations
            .iter()
            .find(|c| c.id == "c1")
            .expect("conversa c1");
        assert_eq!(c.count, 3);
        assert_eq!(c.title, "Ana Silva");
        assert_eq!(c.first, "2021-08-07 10:00");
        assert_eq!(c.last, "2021-08-07 10:06");
        assert!(c.messages[0].content.contains('\n'));
        assert!(c.messages[1].mine);
        assert!(c.messages[2].empty);
    }

    #[test]
    fn nome_informado_manda_no_lugar_do_palpite() {
        let dir = fixture::dir();
        let mut o = opts(&dir);
        o.me = Some("Ana Silva".into());
        let r = run(&o, &noop_progress()).expect("roda");
        assert_eq!(r.me, "Ana Silva");
        assert!(!r.me_guessed);
        assert_eq!(r.sent, 2);
    }

    #[test]
    fn busca_filtra_conversa() {
        let dir = fixture::dir();
        let mut o = opts(&dir);
        o.query = Some("Fechado".into());
        let r = run(&o, &noop_progress()).expect("roda");
        assert_eq!(r.matched, 1);
        assert_eq!(r.conversations[0].id, "c2");
    }

    #[test]
    fn exporta_html_markdown_e_json() {
        let dir = fixture::dir();
        let out = dir.join("saida-msg");
        let mut o = opts(&dir);
        o.out_dir = Some(out.to_string_lossy().to_string());
        o.export_html = Some(true);
        o.export_markdown = Some(true);
        o.export_json = Some(true);
        let r = run(&o, &noop_progress()).expect("roda");
        assert_eq!(r.exports.len(), 3);
        let page = std::fs::read_to_string(out.join("mensagens.html")).expect("html");
        assert!(page.contains("window.__LI__"));
        assert!(page.contains("Ana Silva"));
        assert!(page.contains("class=\"wrap\""));
        let md: Vec<_> = std::fs::read_dir(out.join("conversas"))
            .expect("pasta")
            .filter_map(std::result::Result::ok)
            .collect();
        assert_eq!(md.len(), 2);
    }

    #[test]
    fn export_sem_mensagens_nao_estoura() {
        let tmp = std::env::temp_dir().join(format!("omniget-li-msgvazio-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let _ = std::fs::write(tmp.join("Skills.csv"), "Name\nRust\n");
        let r = run(&opts(&tmp), &noop_progress()).expect("roda");
        assert_eq!(r.total, 0);
        assert_eq!(r.me, "");
    }
}
