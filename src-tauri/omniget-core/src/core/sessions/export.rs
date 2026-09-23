//! Exportação de sessão: JSON neutro (reimportável), Markdown legível e
//! "contexto" (Markdown feito para colar como primeira mensagem em qualquer
//! outra ferramenta, com orçamento de tamanho).

use serde::{Deserialize, Serialize};

use super::model::{Role, Session};
use super::util;

pub const FORMAT_ID: &str = "omniget.session";
pub const FORMAT_VERSION: u32 = 1;

/// Envelope do JSON neutro.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NeutralExport {
    pub format: String,
    pub version: u32,
    pub exported_at: String,
    pub exported_by: String,
    pub session: Session,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportOut {
    pub filename: String,
    pub mime: String,
    pub content: String,
    pub turns_included: usize,
    pub turns_total: usize,
}

fn slug(s: &str) -> String {
    let mut out: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    while out.contains("--") {
        out = out.replace("--", "-");
    }
    util::truncate_chars(out.trim_matches('-'), 40)
        .trim_end_matches('…')
        .to_string()
}

fn base_name(s: &Session) -> String {
    let project = s
        .meta
        .project_path
        .as_deref()
        .and_then(|p| std::path::Path::new(p).file_name())
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "session".into());
    let date = s
        .meta
        .started
        .as_deref()
        .map(|d| d.chars().take(10).collect::<String>())
        .unwrap_or_default();
    let short: String = s.meta.id.chars().take(8).collect();
    format!("{}-{}-{}-{}", s.meta.tool, slug(&project), date, short)
}

pub fn to_json(s: &Session) -> ExportOut {
    let env = NeutralExport {
        format: FORMAT_ID.into(),
        version: FORMAT_VERSION,
        exported_at: util::ms_to_rfc3339(util::now_ms()),
        exported_by: format!("OmniGet {}", env!("CARGO_PKG_VERSION")),
        session: s.clone(),
    };
    ExportOut {
        filename: format!("{}.json", base_name(s)),
        mime: "application/json".into(),
        content: serde_json::to_string_pretty(&env).unwrap_or_default(),
        turns_included: s.turns.len(),
        turns_total: s.turns.len(),
    }
}

fn role_label(r: Role) -> &'static str {
    match r {
        Role::User => "User",
        Role::Assistant => "Assistant",
        Role::System => "System",
        Role::Tool => "Tool",
    }
}

fn fence(body: &str) -> String {
    // Cerca maior que qualquer sequência de crases dentro do corpo.
    let mut n = 3;
    let mut run = 0;
    for c in body.chars() {
        if c == '`' {
            run += 1;
            n = n.max(run + 1);
        } else {
            run = 0;
        }
    }
    "`".repeat(n)
}

fn render_turn(out: &mut String, i: usize, t: &super::model::Turn, result_max: usize) {
    out.push_str(&format!("### {} · {}", i + 1, role_label(t.role)));
    if !t.ts.is_empty() {
        out.push_str(&format!(" · {}", t.ts));
    }
    if let Some(m) = &t.model {
        out.push_str(&format!(" · `{m}`"));
    }
    out.push_str("\n\n");
    if !t.text.trim().is_empty() {
        out.push_str(t.text.trim());
        out.push_str("\n\n");
    }
    for c in &t.tool_calls {
        let input = serde_json::to_string_pretty(&c.input).unwrap_or_default();
        let f = fence(&input);
        out.push_str(&format!(
            "**Tool `{}`**\n\n{f}json\n{input}\n{f}\n\n",
            c.name_raw
        ));
        if let Some(r) = &c.result {
            let r = util::truncate_chars(r, result_max);
            let f = fence(&r);
            let label = match c.status {
                super::model::ToolStatus::Error => "Result (error)",
                _ => "Result",
            };
            out.push_str(&format!("{label}:\n\n{f}\n{r}\n{f}\n\n"));
        }
    }
}

fn header(s: &Session, out: &mut String) {
    out.push_str(&format!(
        "# {}\n\n",
        s.meta
            .title
            .clone()
            .unwrap_or_else(|| format!("{} session {}", s.meta.tool, s.meta.id))
    ));
    out.push_str(&format!(
        "- Tool: `{}`\n- Session: `{}`\n",
        s.meta.tool, s.meta.id
    ));
    if let Some(p) = &s.meta.project_path {
        out.push_str(&format!("- Project: `{p}`\n"));
    }
    if let Some(b) = &s.meta.git_branch {
        out.push_str(&format!("- Branch: `{b}`\n"));
    }
    if let (Some(a), Some(b)) = (&s.meta.started, &s.meta.ended) {
        out.push_str(&format!("- Time: {a} → {b}\n"));
    }
    if !s.meta.models.is_empty() {
        out.push_str(&format!("- Models: {}\n", s.meta.models.join(", ")));
    }
    out.push_str(&format!("- Messages: {}\n\n", s.turns.len()));
}

pub fn to_markdown(s: &Session) -> ExportOut {
    let mut out = String::new();
    header(s, &mut out);
    out.push_str("---\n\n");
    for (i, t) in s.turns.iter().enumerate() {
        render_turn(&mut out, i, t, 4000);
    }
    ExportOut {
        filename: format!("{}.md", base_name(s)),
        mime: "text/markdown".into(),
        content: out,
        turns_included: s.turns.len(),
        turns_total: s.turns.len(),
    }
}

/// Markdown de contexto para continuar o trabalho em outra ferramenta.
/// Mantém as mensagens mais recentes que cabem em `budget_chars`.
pub fn to_context(s: &Session, target: Option<&str>, budget_chars: usize) -> ExportOut {
    let mut preamble = String::new();
    preamble.push_str("# Context from a previous session\n\n");
    preamble.push_str(&format!(
        "The conversation below happened in `{}`{}. Read it as background: it records what was asked, what was done and which tools ran. Continue the work from where it stopped; do not repeat steps that already succeeded.\n\n",
        s.meta.tool,
        target.map(|t| format!(" and is being continued in `{t}`")).unwrap_or_default()
    ));
    header(s, &mut preamble);
    // Monta de trás para frente até o orçamento.
    let mut blocks: Vec<String> = Vec::new();
    let mut used = preamble.len();
    for (i, t) in s.turns.iter().enumerate().rev() {
        if t.role == Role::System && t.text.trim().is_empty() {
            continue;
        }
        let mut b = String::new();
        render_turn(&mut b, i, t, 600);
        if used + b.len() > budget_chars && !blocks.is_empty() {
            break;
        }
        used += b.len();
        blocks.push(b);
    }
    blocks.reverse();
    let included = blocks.len();
    let mut out = preamble;
    if included < s.turns.len() {
        out.push_str(&format!(
            "> Only the last {included} of {} messages fit; older ones were left out.\n\n",
            s.turns.len()
        ));
    }
    out.push_str("---\n\n");
    for b in blocks {
        out.push_str(&b);
    }
    out.push_str("---\n\nContinue from here.\n");
    ExportOut {
        filename: format!("{}-context.md", base_name(s)),
        mime: "text/markdown".into(),
        content: out,
        turns_included: included,
        turns_total: s.turns.len(),
    }
}
