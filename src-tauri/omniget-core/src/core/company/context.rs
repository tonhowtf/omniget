//! The heartbeat prompt.
//!
//! Two shapes, to keep the cached prefix stable and the fresh input small:
//! - **brief** (first run of an agent, or a new session): who you are, the
//!   company and its mission, your place in the org chart, the working
//!   agreement (how the `company_*` tools are used), then the wake;
//! - **delta** (resumed session): only why you woke, what changed on the
//!   issue since your last run, and your queue.
//!
//! The agent pulls anything else with the tools (issue details, documents,
//! search), like Paperclip's "thin" context mode — but the delta carries the
//! new comments inline, so a mention does not cost a tool round trip.

use std::fmt::Write as _;

use super::heartbeat::Claim;
use super::issues::IssueFilter;
use super::CompanyStore;

/// Characters of an issue body or comment copied into the prompt.
const CLIP: usize = 2400;

fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(n).collect();
        t.push_str(" …");
        t
    }
}

pub const WORKING_AGREEMENT: &str = "How you work here:
- Your work lives in issues. Use the `company_*` tools: `company_my_issues` for your queue, `company_issue_get` for details, `company_checkout` before working on an issue, `company_comment` to report progress, `company_issue_status` to move it (todo, in_progress, in_review, blocked with a reason, done).
- Finish every run by leaving a trace on the issue: a comment with what you did and what is next, and the right status. A run that leaves no trace is retried and then handed to the owner.
- Delegate by creating subtasks (`company_issue_create` with `parent` and `assignee`) for the people who report to you. Do not do your reports' jobs.
- Ask for what you cannot decide: `company_request_approval` (a hire, a strategy, anything the board must approve) or `company_ask_owner` for a choice between options. Then stop; you are woken with the answer.
- Plans and specs go in documents (`company_doc_put`, key `plan`, `spec`, `notes`…). Deliverables are work products (`company_work_product`).
- Mention someone with @name in a comment to wake them. Mention @owner to reach the human.
- Stay inside your role. Keep the company's mission in mind; say so when a task does not serve it.";

impl CompanyStore {
    /// Builds the prompt of a claimed run. `resumed` = the agent's session
    /// continues (send only the delta).
    pub fn heartbeat_prompt(&self, claim: &Claim, resumed: bool) -> Result<String, String> {
        let s = &claim.staff;
        let company = self.company_get(&s.company_id)?;
        let mut p = String::with_capacity(4096);
        if !resumed {
            let _ = writeln!(
                p,
                "You are {}, {} at {}.",
                s.name,
                if s.title.is_empty() {
                    &s.role
                } else {
                    &s.title
                },
                company.name
            );
            let _ = writeln!(
                p,
                "Company mission: {}",
                if company.mission.is_empty() {
                    &company.name
                } else {
                    &company.mission
                }
            );
            if !company.description.is_empty() {
                let _ = writeln!(p, "About the company: {}", clip(&company.description, 1200));
            }
            let chain = self
                .chain_of_command(&s.company_id, &s.id)
                .unwrap_or_default();
            match chain.first() {
                Some(m) => {
                    let _ = writeln!(p, "You report to {} ({}).", m.name, m.role);
                }
                None => {
                    let _ = writeln!(p, "You report to the owner (the human board).");
                }
            }
            let reports = self.staff_reports(&s.company_id, &s.id).unwrap_or_default();
            if !reports.is_empty() {
                let names: Vec<String> = reports
                    .iter()
                    .map(|r| format!("{} ({})", r.name, r.role))
                    .collect();
                let _ = writeln!(p, "Your reports: {}.", names.join(", "));
            }
            if s.can_hire {
                let _ = writeln!(
                    p,
                    "You may hire into your branch with `company_hire`{}.",
                    if company.require_hire_approval {
                        " (each hire waits for the owner's approval)"
                    } else {
                        ""
                    }
                );
            }
            if !s.capabilities.is_empty() {
                let _ = writeln!(p, "Your responsibilities: {}", clip(&s.capabilities, 1200));
            }
            if !s.instructions.is_empty() {
                let _ = writeln!(p, "\nYour instructions:\n{}", clip(&s.instructions, 6000));
            }
            let goals = self.goal_list(&s.company_id).unwrap_or_default();
            let open: Vec<String> = goals
                .iter()
                .filter(|g| g.level != "company" && g.status == "active")
                .take(8)
                .map(|g| format!("- [{}] {}", g.level, g.title))
                .collect();
            if !open.is_empty() {
                let _ = writeln!(p, "\nActive goals:\n{}", open.join("\n"));
            }
            if company.require_strategy_approval
                && company.strategy_state != "approved"
                && reports.len() + chain.len() > 0
                && chain.is_empty()
            {
                let _ = writeln!(p, "\nThe owner must approve a strategy before delegated work runs. Write it as the `plan` document of an issue and ask with `company_request_approval` kind `strategy`.");
            }
            let _ = writeln!(p, "\n{WORKING_AGREEMENT}\n");
        }

        // Why you woke.
        let _ = writeln!(p, "## Why you are awake");
        for w in &claim.wakes {
            let _ = writeln!(p, "- {} ({})", w.reason, w.source);
            if let Some(a) = w.context.get("approval_id").and_then(|v| v.as_str()) {
                if let Ok(ap) = self.approval_get(a) {
                    let _ = writeln!(
                        p,
                        "  Request \"{}\": {}{}{}",
                        ap.title,
                        ap.state,
                        ap.chosen
                            .as_deref()
                            .map(|c| format!(", chosen: {c}"))
                            .unwrap_or_default(),
                        ap.note
                            .as_deref()
                            .map(|n| format!(", note: {n}"))
                            .unwrap_or_default()
                    );
                }
            }
        }

        if let Some(i) = &claim.issue {
            let d = self.issue_detail(&s.company_id, &i.id)?;
            let _ = writeln!(p, "\n## {} — {}", i.identifier, i.title);
            let _ = writeln!(
                p,
                "Status {} · priority {} · mode {}{}",
                i.status,
                i.priority,
                i.work_mode,
                if i.review_policy != "none" {
                    format!(" · review: {}", i.review_policy)
                } else {
                    String::new()
                }
            );
            if i.work_mode == "planning" {
                let _ = writeln!(p, "Planning mode: write or revise the `plan` document only; do not implement. The owner approves the plan revision, then you continue.");
            } else if i.work_mode == "ask" {
                let _ = writeln!(p, "Ask mode: answer in a comment; do not change code or deliver an implementation.");
            }
            if (!resumed || claim.first_run) && !i.body.trim().is_empty() {
                let _ = writeln!(p, "\n{}", clip(&i.body, CLIP));
            }
            if !d.blocked_by.is_empty() {
                let b: Vec<String> = d
                    .blocked_by
                    .iter()
                    .map(|l| format!("{} ({})", l.identifier, l.status))
                    .collect();
                let _ = writeln!(p, "Blocked by: {}", b.join(", "));
            }
            if !d.children.is_empty() {
                let c: Vec<String> = d
                    .children
                    .iter()
                    .map(|l| format!("{} {} ({})", l.identifier, l.title, l.status))
                    .collect();
                let _ = writeln!(p, "Subtasks: {}", c.join("; "));
            }
            if !d.documents.is_empty() {
                let keys: Vec<String> = d
                    .documents
                    .iter()
                    .map(|x| format!("{} (rev {})", x.key, x.rev))
                    .collect();
                let _ = writeln!(p, "Documents: {}", keys.join(", "));
            }
            let since = if resumed {
                claim.last_run_ms.unwrap_or(0)
            } else {
                0
            };
            let fresh: Vec<_> = d.comments.iter().filter(|c| c.created_ms > since).collect();
            let shown = fresh.iter().rev().take(if resumed { 12 } else { 8 }).rev();
            let mut any = false;
            for c in shown {
                if !any {
                    let _ = writeln!(
                        p,
                        "\n{}:",
                        if resumed {
                            "New on this issue since your last run"
                        } else {
                            "Latest comments"
                        }
                    );
                    any = true;
                }
                let who = match (c.author_kind.as_str(), &c.author_staff_id) {
                    ("user", _) => "owner".to_string(),
                    ("agent", Some(id)) => self
                        .staff_get(&s.company_id, id)
                        .map(|x| x.name)
                        .unwrap_or_else(|_| "agent".into()),
                    _ => "system".into(),
                };
                let _ = writeln!(p, "- {who}: {}", clip(&c.body, 800));
            }
        }

        // Your queue (short: the tools give the rest).
        let queue = self
            .issue_list(
                &s.company_id,
                &IssueFilter {
                    assignee_staff_id: Some(s.id.clone()),
                    limit: Some(10),
                    ..Default::default()
                },
            )
            .unwrap_or_default();
        let other: Vec<String> = queue
            .iter()
            .filter(|q| claim.issue.as_ref().is_none_or(|i| i.id != q.id))
            .map(|q| {
                format!(
                    "- {} {} ({}, {})",
                    q.identifier, q.title, q.status, q.priority
                )
            })
            .collect();
        if !other.is_empty() {
            let _ = writeln!(p, "\n## Your other open issues\n{}", other.join("\n"));
        }
        if claim.issue.is_none() && queue.is_empty() {
            let _ = writeln!(p, "\nYou have no open issues. Review the goals and your reports' work; file issues for what should happen next, or reply that nothing is needed.");
        }
        let _ = writeln!(
            p,
            "\nAct now with the tools, then end with a one-paragraph summary of what you did."
        );
        Ok(p)
    }
}
