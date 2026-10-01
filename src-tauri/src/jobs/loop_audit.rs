//! The auditor of a Loop round (roadmap 3.7, S22, 84#P1): a side call of
//! the Loop's own agent, on the small model of its provider/account
//! (`TaskKind::Side`, 84#P3), that reads the objective, what the round's
//! tools did (`goal_audit::Evidence`) and its last reply, and returns a
//! verdict. The rules that act on the verdict and the
//! text of the rounds are in [`omniget_core::core::llm::goal_audit`].

use super::*;
use omniget_core::core::llm::goal_audit::{self, Audit};
use omniget_core::core::llm::router::{self, SideReason};

/// Ceiling of one audit: a verdict in one JSON line.
const AUDIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// Audits kept on a Loop row (newest last).
pub const AUDITS_KEPT: usize = 50;

/// One audited round, as the Loops screen shows it.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct LoopAudit {
    pub round: u32,
    /// `continue|complete|blocked`; `None` = the audit gave no verdict.
    #[serde(default)]
    pub verdict: Option<String>,
    #[serde(default)]
    pub note: String,
    /// The agent's own claim (a `DONE` line or `Missing: nothing`): kept to
    /// compare with the verdict, never acted on.
    #[serde(default)]
    pub claimed_done: bool,
    /// The side model the audit ran on.
    #[serde(default)]
    pub model: String,
    /// Why that model (`provider_small`, `session_model`, …); `…+fallback`
    /// when the small model failed and the session's own answered.
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub cost_usd: Option<f64>,
    /// Billable tokens of the audit (`billable_of`).
    #[serde(default)]
    pub tokens: u64,
    #[serde(default)]
    pub ms: u64,
    /// The round it judged, for the overhead (audit ÷ round).
    #[serde(default)]
    pub round_cost_usd: Option<f64>,
    #[serde(default)]
    pub round_tokens: u64,
    /// The round went out as the short continuation guide (83#P7).
    #[serde(default)]
    pub guided: bool,
}

/// What one side call returned.
struct SideAnswer {
    text: String,
    usage: Option<Value>,
    error: Option<String>,
    model: router::SideModel,
}

impl Jobs {
    /// Whether the next round of this Loop resumes the provider's session
    /// (Claude `--resume`, Codex `thread/resume`): its session record has a
    /// handle and the runtime resumes natively. Only then does round ≥ 2 go
    /// out as the guide (83#P7).
    pub(super) fn loop_resumable(&self, l: &LoopDef) -> bool {
        if l.agent_id.starts_with(TOOL_PREFIX) {
            return false;
        }
        omniget_core::core::assist::runs::active()
            .and_then(|reg| reg.session(&sanitize_id(&l.conversation_id), &l.agent_id))
            .is_some_and(|s| s.native_resume && s.provider_handle.is_some())
    }

    async fn side_call(
        &self,
        loop_id: &str,
        agent_id: &str,
        input: &str,
        requested: Option<&str>,
    ) -> SideAnswer {
        let conv = format!(
            "{}audit-{}",
            router::SIDE_PREFIX,
            uuid::Uuid::new_v4().simple()
        );
        // The audit is spent on the Loop (3.3): its budget policies see and
        // gate it like a round (one mechanism, `loop:<id>`).
        let _budget_scope = omniget_core::core::llm::budget::policy::Binding::new(
            &conv,
            vec![omniget_core::core::llm::budget::policy::Scope::Loop(
                loop_id.to_string(),
            )],
        );
        let started = self
            .llm
            .side_turn_stream(
                &conv,
                agent_id,
                goal_audit::audit_system(),
                input,
                requested,
            )
            .await;
        let (request, cancel, mut stream, model) = match started {
            Ok(s) => s,
            Err(e) => {
                return SideAnswer {
                    text: String::new(),
                    usage: None,
                    error: Some(e),
                    model: router::SideModel {
                        model: requested.unwrap_or_default().to_string(),
                        reason: SideReason::Requested,
                    },
                }
            }
        };
        let mut text = String::new();
        let mut usage: Option<Value> = None;
        let mut error: Option<String> = None;
        let label = model.model.clone();
        let run = async {
            while let Some(event) = stream.next().await {
                match &event {
                    TurnEvent::TextDelta { text: t } => text.push_str(t),
                    TurnEvent::Error { error: e } => error = Some(e.to_string()),
                    TurnEvent::Usage { usage: u } => fold_usage(&mut usage, &label, u),
                    _ => {}
                }
            }
        };
        if tokio::time::timeout(AUDIT_TIMEOUT, run).await.is_err() {
            cancel.cancel();
            error = Some(format!(
                "{ERR_JOBS}: the audit took longer than {} s",
                AUDIT_TIMEOUT.as_secs()
            ));
        }
        self.llm.finish_turn(&request, agent_id);
        self.llm.drop_conversation(&conv);
        SideAnswer {
            text,
            usage,
            error,
            model,
        }
    }

    /// Audits one round. `None` = no verdict (the call failed or the answer
    /// did not parse); the entry says why. When the small model itself fails
    /// (a name the account does not offer), the audit is asked once more of
    /// the session's own model.
    pub(super) async fn audit_round(
        &self,
        l: &LoopDef,
        round: u32,
        reply: &str,
        evidence: &str,
        previous: Option<&Audit>,
    ) -> (Option<Audit>, LoopAudit) {
        let input = goal_audit::audit_input(&l.prompt, reply, evidence, previous);
        let started = now_ms();
        let mut answer = self
            .side_call(&l.id, &l.agent_id, &input, l.audit_model.as_deref())
            .await;
        let mut fallback = false;
        let mut spent: Option<Value> = answer.usage.clone();
        if answer.error.is_some()
            && answer.text.trim().is_empty()
            && answer.model.reason == SideReason::ProviderSmall
        {
            if let Some(agent) = self.llm.agent(&l.agent_id) {
                let session = self.llm.model_of(&agent).model;
                if session != answer.model.model {
                    let again = self
                        .side_call(&l.id, &l.agent_id, &input, Some(&session))
                        .await;
                    fallback = true;
                    // Both calls are paid for.
                    spent = merge_usage(spent, again.usage.clone());
                    answer = again;
                }
            }
        }
        let audit = if answer.text.trim().is_empty() {
            None
        } else {
            goal_audit::parse(&answer.text)
        };
        let error = match (&audit, &answer.error) {
            (Some(_), _) => None,
            (None, Some(e)) => Some(e.clone()),
            (None, None) => Some(format!(
                "{ERR_JOBS}: the audit gave no verdict: {}",
                clip(answer.text.trim(), 200)
            )),
        };
        let entry = LoopAudit {
            round,
            verdict: audit.as_ref().map(|a| a.verdict.as_str().to_string()),
            note: audit.as_ref().map(|a| a.note.clone()).unwrap_or_default(),
            claimed_done: goal_audit::claims_done(reply),
            model: answer.model.model.clone(),
            reason: format!(
                "{}{}",
                answer.model.reason.as_str(),
                if fallback { "+fallback" } else { "" }
            ),
            error,
            cost_usd: spent.as_ref().and_then(|u| u["cost_usd"].as_f64()),
            tokens: spent.as_ref().map(billable_of).unwrap_or(0),
            ms: now_ms().saturating_sub(started),
            ..LoopAudit::default()
        };
        (audit, entry)
    }
}

/// The folded usage of two calls, summed field by field.
fn merge_usage(a: Option<Value>, b: Option<Value>) -> Option<Value> {
    match (a, b) {
        (None, b) => b,
        (a, None) => a,
        (Some(mut a), Some(b)) => {
            for k in [
                "input_tokens",
                "output_tokens",
                "cache_read_tokens",
                "cache_write_tokens",
                "calls",
            ] {
                a[k] = json!(a[k].as_u64().unwrap_or(0) + b[k].as_u64().unwrap_or(0));
            }
            let cost = match (a["cost_usd"].as_f64(), b["cost_usd"].as_f64()) {
                (None, None) => Value::Null,
                (x, y) => json!(x.unwrap_or(0.0) + y.unwrap_or(0.0)),
            };
            a["cost_usd"] = cost;
            Some(a)
        }
    }
}

/// Appends an audit to the Loop's list, keeping the newest [`AUDITS_KEPT`].
pub(super) fn push_audit(l: &mut LoopDef, entry: LoopAudit) {
    l.audits.push(entry);
    if l.audits.len() > AUDITS_KEPT {
        let extra = l.audits.len() - AUDITS_KEPT;
        l.audits.drain(..extra);
    }
}

/// The last verdict of a Loop row, for the guide of the next round after a
/// restart.
pub(super) fn last_audit(l: &LoopDef) -> Option<Audit> {
    let a = l.audits.last()?;
    let verdict = match a.verdict.as_deref()? {
        "continue" => goal_audit::Verdict::Continue,
        "complete" => goal_audit::Verdict::Complete,
        "blocked" => goal_audit::Verdict::Blocked,
        _ => return None,
    };
    Some(Audit {
        verdict,
        note: a.note.clone(),
    })
}
