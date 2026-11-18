use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use serde::Serialize;
use serde_json::json;

use super::PhoneChat;
use crate::rules::{self, Action, Decision, Facts, Limits, Rule, Skip};
use crate::store::{self, MessageRow, OptOut, RuleRun, RuleStat};

const BATCH: u32 = 50;
const AI_TIMEOUT: Duration = Duration::from_secs(45);
const TEST_CHAT: &str = "0@s.whatsapp.net";
const FIELD_CHARS: usize = 500;
const ASK_GAP: Duration = Duration::from_millis(1200);

#[derive(Clone, Debug, Serialize)]
pub struct RulesState {
    pub limits: Limits,
    pub rules: Vec<Rule>,
    pub sent_hour: u32,
    pub sent_day: u32,
    pub opt_outs: Vec<OptOut>,
    pub stats: Vec<RuleStat>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Contact {
    pub tags: Vec<String>,
    pub fields: Vec<(String, String)>,
}

#[derive(Clone, Debug, Serialize)]
pub struct RuleTest {
    pub rule: Option<Rule>,
    pub reply: Option<String>,
    pub opt_out: bool,
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn skip_code(skip: Skip) -> &'static str {
    match skip {
        Skip::Disabled => "disabled",
        Skip::Stale => "stale",
        Skip::OptedOut => "opted_out",
        Skip::NoRule => "no_rule",
        Skip::HumanActive => "human_active",
        Skip::Cooldown => "cooldown",
        Skip::HourCap => "hour_cap",
        Skip::DayCap => "day_cap",
    }
}

impl PhoneChat {
    pub(super) fn refresh_rules(&self) {
        let on = self.store.limits().is_ok_and(|l| l.enabled)
            && self
                .store
                .rules()
                .is_ok_and(|r| r.iter().any(|r| r.enabled));
        if on && !self.rules_on.load(Ordering::SeqCst) {
            self.rule_cursor
                .store(self.store.latest_seq().unwrap_or(0), Ordering::SeqCst);
        }
        self.rules_on.store(on, Ordering::SeqCst);
    }

    fn rules_changed(&self) {
        self.refresh_rules();
        self.host.emit("phonechat-rules-changed", json!({}));
    }

    pub(super) fn pause_rules(&self) {
        let Ok(mut limits) = self.store.limits() else {
            return;
        };
        if limits.enabled {
            limits.enabled = false;
            let _ = self.store.set_limits(&limits);
            self.rules_changed();
        }
    }

    pub fn rules_state(&self) -> Result<RulesState> {
        let (sent_hour, sent_day) = self.store.run_counts(now())?;
        Ok(RulesState {
            limits: self.store.limits()?,
            rules: self.store.rules()?,
            sent_hour,
            sent_day,
            opt_outs: self.store.opt_outs()?,
            stats: self.store.rule_stats()?,
        })
    }

    pub fn rule_save(&self, rule: Rule) -> Result<Rule> {
        let mut rule = rules::validate(rule).map_err(|code| anyhow!(code))?;
        rule.chats = rule.chats.iter().map(|c| self.store.canonical(c)).collect();
        let id = self
            .store
            .rule_save(&rule)?
            .ok_or_else(|| anyhow!("rule_not_found"))?;
        rule.id = id;
        self.rules_changed();
        Ok(rule)
    }

    pub fn rule_enable(&self, id: i64, enabled: bool) -> Result<Rule> {
        let mut rule = self
            .store
            .rules()?
            .into_iter()
            .find(|r| r.id == id)
            .ok_or_else(|| anyhow!("rule_not_found"))?;
        rule.enabled = enabled;
        self.store.rule_save(&rule)?;
        self.rules_changed();
        Ok(rule)
    }

    pub fn rule_delete(&self, id: i64) -> Result<bool> {
        let gone = self.store.rule_delete(id)?;
        if gone {
            self.rules_changed();
        }
        Ok(gone)
    }

    pub fn set_limits(&self, limits: Limits) -> Result<Limits> {
        let limits = limits.clamped();
        self.store.set_limits(&limits)?;
        self.rules_changed();
        Ok(limits)
    }

    pub fn rule_runs(&self, limit: u32) -> Result<Vec<RuleRun>> {
        Ok(self.store.runs(limit.clamp(1, 200))?)
    }

    pub fn opt_in(&self, chat: &str) -> Result<bool> {
        let done = self.store.opt_in(chat)?;
        if done {
            self.rules_changed();
        }
        Ok(done)
    }

    pub fn rule_test(&self, chat: Option<&str>, text: &str) -> Result<RuleTest> {
        if text.trim().is_empty() {
            bail!("empty_message");
        }
        let chat = chat.unwrap_or(TEST_CHAT);
        let is_group = store::is_group(chat);
        if !is_group && rules::is_opt_out(text) {
            return Ok(RuleTest {
                rule: None,
                reply: None,
                opt_out: true,
            });
        }
        let all = self.store.rules()?;
        let none = std::collections::HashMap::new();
        let tags = self.store.tags_of(chat)?;
        let fields = self.store.fields_of(chat)?;
        let facts = Facts {
            is_group,
            chat,
            text: Some(text),
            ts: 0,
            now: 0,
            first_message: false,
            opted_out: false,
            sent_last_hour: 0,
            sent_last_day: 0,
            last_manual_ts: None,
            last_runs: &none,
            tags: &tags,
            minute_of_day: minute_of_day(now()),
            own: false,
        };
        let rule = rules::matching(&all, &facts).cloned();
        let reply = match rule.as_ref().map(|r| &r.action) {
            Some(Action::Reply { text }) => {
                let name = if chat == TEST_CHAT {
                    "Ana Souza".to_string()
                } else {
                    self.chat_name(chat)
                };
                Some(rules::render(rules::variant(text, 0), &name, &fields))
            }
            Some(Action::React { emoji }) => Some(emoji.clone()),
            _ => None,
        };
        Ok(RuleTest {
            rule,
            reply,
            opt_out: false,
        })
    }

    pub(super) fn answer_inbound(self: &Arc<Self>) {
        if !self.rules_on.load(Ordering::SeqCst) {
            return;
        }
        let (Ok(limits), Ok(all)) = (self.store.limits(), self.store.rules()) else {
            return;
        };
        let mine = all.iter().any(|r| r.enabled && r.own_messages);
        let after = self.rule_cursor.load(Ordering::SeqCst);
        let Ok(inbound) = self.store.live_after(None, after, mine, BATCH) else {
            return;
        };
        let Some(last) = inbound.last() else {
            return;
        };
        self.rule_cursor.store(last.seq, Ordering::SeqCst);
        for msg in inbound {
            if msg.deleted || (msg.from_me && self.sent_by_a_rule(&msg)) {
                continue;
            }
            if let Err(e) = self.consider(&all, &limits, msg) {
                self.host.emit(
                    "phonechat-rules-error",
                    json!({ "error": format!("{e:#}") }),
                );
            }
        }
    }

    fn sent_by_a_rule(&self, msg: &MessageRow) -> bool {
        self.auto_sent
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&msg.id)
            || self.store.rule_sent(&msg.chat, &msg.id).unwrap_or(true)
    }

    fn consider(self: &Arc<Self>, all: &[Rule], limits: &Limits, msg: MessageRow) -> Result<()> {
        let at = now();
        if limits.enabled && at - msg.ts <= rules::STALE_AFTER_SECS {
            self.capture_answer(&msg, at)?;
        }
        let (sent_last_hour, sent_last_day) = self.store.run_counts(at)?;
        let last_runs = self.store.last_runs(&msg.chat)?;
        let tags = self.store.tags_of(&msg.chat)?;
        let facts = Facts {
            is_group: store::is_group(&msg.chat),
            chat: &msg.chat,
            text: msg.text.as_deref(),
            ts: msg.ts,
            now: at,
            first_message: self.store.inbound_count(&msg.chat)? == 1,
            opted_out: self.store.opted_out(&msg.chat)?,
            sent_last_hour,
            sent_last_day,
            last_manual_ts: self.store.last_manual_ts(&msg.chat)?,
            last_runs: &last_runs,
            tags: &tags,
            minute_of_day: minute_of_day(msg.ts),
            own: msg.from_me,
        };
        match rules::decide(all, limits, &facts) {
            Decision::OptOut => {
                self.store.opt_out(&msg.chat, at)?;
                if self.store.cancel_follow_ups(&msg.chat)? > 0 {
                    self.sched_wake.notify_one();
                    self.host.emit("phonechat-scheduled-changed", json!({}));
                }
                self.host.emit("phonechat-rules-changed", json!({}));
            }
            Decision::Skip(
                skip @ (Skip::HumanActive | Skip::Cooldown | Skip::HourCap | Skip::DayCap),
            ) => {
                if let Some(rule) = rules::matching(all, &facts) {
                    let run = self.store.run_start(rule.id, &msg.chat, at)?;
                    self.store
                        .run_finish(run, "skipped", Some(skip_code(skip)), None)?;
                    self.host.emit("phonechat-rules-changed", json!({}));
                }
            }
            Decision::Skip(_) => {}
            Decision::Run(rule) => {
                let fresh = self
                    .rule_busy
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(msg.chat.clone());
                if !fresh {
                    return Ok(());
                }
                let run = self.store.run_start(rule.id, &msg.chat, at)?;
                let (me, rule) = (self.clone(), rule.clone());
                tokio::spawn(async move {
                    let chat = msg.chat.clone();
                    let (outcome, detail, sent_id) =
                        match me.run_rule(&rule, &msg, limits_delay(&me, run)).await {
                            Ok(Done::Sent(id)) => {
                                me.after_answer(&rule, &msg).await;
                                ("sent", None, Some(id))
                            }
                            Ok(Done::Held(why)) => ("skipped", Some(why.to_string()), None),
                            Err(e) => ("failed", Some(format!("{e:#}")), None),
                        };
                    let _ =
                        me.store
                            .run_finish(run, outcome, detail.as_deref(), sent_id.as_deref());
                    me.rule_busy
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .remove(&chat);
                    me.host.emit("phonechat-rules-changed", json!({}));
                });
            }
        }
        Ok(())
    }

    async fn run_rule(
        self: &Arc<Self>,
        rule: &Rule,
        msg: &MessageRow,
        delay: Duration,
    ) -> Result<Done> {
        let fields = self.store.fields_of(&msg.chat)?;
        let group = store::is_group(&msg.chat);
        let name = match (&msg.sender_name, group) {
            (Some(n), true) => n.clone(),
            _ => self.chat_name(&msg.chat),
        };
        let quote = group.then_some(msg.id.as_str());
        if !matches!(rule.action, Action::React { .. }) {
            let _ = self.send_typing(&msg.chat, true).await;
        }
        let text = match &rule.action {
            Action::React { emoji } => {
                tokio::time::sleep(delay).await;
                if !self.still_wanted(msg)? {
                    return Ok(Done::Held("human_active"));
                }
                self.react(&msg.chat, &msg.id, emoji).await?;
                return Ok(Done::Sent(String::new()));
            }
            Action::Reply { text } => {
                tokio::time::sleep(delay).await;
                rules::render(rules::variant(text, msg.seq), &name, &fields)
            }
            Action::Ai {
                instructions,
                fallback,
                draft,
            } => {
                let history: Vec<(bool, String)> = self
                    .store
                    .messages(&msg.chat, None, rules::AI_CONTEXT_MESSAGES)?
                    .into_iter()
                    .filter(|m| !m.deleted)
                    .filter_map(|m| m.text.map(|t| (m.from_me, t)))
                    .collect();
                let asked = self.host.generate(
                    rules::ai_system(instructions),
                    rules::ai_user(&history, &name),
                );
                let answer = match tokio::time::timeout(AI_TIMEOUT, asked).await {
                    Ok(Ok(raw)) => rules::ai_reply(&raw, instructions),
                    Ok(Err(e)) => match fallback {
                        Some(_) => None,
                        None => bail!("ai_failed: {e}"),
                    },
                    Err(_) => match fallback {
                        Some(_) => None,
                        None => bail!("ai_timeout"),
                    },
                };
                let text = match answer
                    .or_else(|| fallback.as_ref().map(|f| rules::render(f, &name, &fields)))
                {
                    Some(text) => text,
                    None => {
                        let _ = self.send_typing(&msg.chat, false).await;
                        bail!("ai_silent");
                    }
                };
                if *draft {
                    let _ = self.send_typing(&msg.chat, false).await;
                    self.store.draft_set(&msg.chat, &text, now())?;
                    self.host
                        .emit("phonechat-draft", json!({ "chat": msg.chat }));
                    return Ok(Done::Held("draft"));
                }
                text
            }
        };
        if !self.still_wanted(msg)? {
            let _ = self.send_typing(&msg.chat, false).await;
            return Ok(Done::Held("human_active"));
        }
        let sent = self.send_auto(&msg.chat, &text, quote).await?;
        Ok(Done::Sent(sent.id))
    }

    fn capture_answer(&self, msg: &MessageRow, at: i64) -> Result<()> {
        let Some(text) = msg.text.as_deref().map(str::trim).filter(|t| !t.is_empty()) else {
            return Ok(());
        };
        let group = store::is_group(&msg.chat);
        if (group && !msg.from_me) || (!msg.from_me && rules::is_opt_out(text)) {
            return Ok(());
        }
        let Some((rule_id, field)) = self.store.wait_take(&msg.chat, at)? else {
            return Ok(());
        };
        let value: String = text.chars().take(FIELD_CHARS).collect();
        self.store.field_set(&msg.chat, &field, &value)?;
        let run = self.store.run_start(rule_id, &msg.chat, at)?;
        self.store
            .run_finish(run, "skipped", Some("captured"), None)?;
        self.host.emit("phonechat-chats-changed", json!({}));
        self.host.emit("phonechat-rules-changed", json!({}));
        Ok(())
    }

    async fn after_answer(self: &Arc<Self>, rule: &Rule, msg: &MessageRow) {
        self.after_answer_local(rule, msg);
        let Some(ask) = &rule.ask else {
            return;
        };
        if store::is_group(&msg.chat) && !msg.from_me {
            return;
        }
        tokio::time::sleep(ASK_GAP).await;
        let name = self.chat_name(&msg.chat);
        let fields = self.store.fields_of(&msg.chat).unwrap_or_default();
        let question = rules::render(&ask.question, &name, &fields);
        if self.send_auto(&msg.chat, &question, None).await.is_ok() {
            let _ = self.store.wait_set(&msg.chat, rule.id, &ask.field, now());
        }
    }

    fn after_answer_local(&self, rule: &Rule, msg: &MessageRow) {
        if let Some(tag) = &rule.tag {
            let _ = self.store.tag_set(&msg.chat, tag, true);
            self.host.emit("phonechat-chats-changed", json!({}));
        }
        if rule.follow_ups.is_empty() {
            return;
        }
        let at = now();
        let name = self.chat_name(&msg.chat);
        let fields = self.store.fields_of(&msg.chat).unwrap_or_default();
        for step in &rule.follow_ups {
            let _ = self.store.schedule_follow_up(
                rule.id,
                &msg.chat,
                &rules::render(&step.text, &name, &fields),
                at + i64::from(step.after_minutes) * 60,
                at,
            );
        }
        self.sched_wake.notify_one();
        self.host.emit("phonechat-scheduled-changed", json!({}));
    }

    pub(super) fn follow_up_allowed(&self, chat: &str) -> Result<bool> {
        let limits = self.store.limits()?;
        let (hour, day) = self.store.run_counts(now())?;
        Ok(limits.enabled
            && !self.store.opted_out(chat)?
            && hour < limits.per_hour
            && day < limits.per_day)
    }

    pub(super) fn follow_up_sent(&self, rule_id: i64, chat: &str, sent_id: &str) {
        if let Ok(run) = self.store.run_start(rule_id, chat, now()) {
            let _ = self
                .store
                .run_finish(run, "sent", Some("follow_up"), Some(sent_id));
        }
        self.host.emit("phonechat-rules-changed", json!({}));
    }

    pub fn tag_chat(&self, chat: &str, tag: &str, on: bool) -> Result<bool> {
        let tag = rules::clean_tag(tag).map_err(|code| anyhow!(code))?;
        let changed = self.store.tag_set(chat, &tag, on)?;
        if changed {
            self.host.emit("phonechat-chats-changed", json!({}));
        }
        Ok(changed)
    }

    pub fn set_field(&self, chat: &str, key: &str, value: &str) -> Result<()> {
        let key = rules::clean_field(key).map_err(|code| anyhow!(code))?;
        let value: String = value.trim().chars().take(FIELD_CHARS).collect();
        self.store.field_set(chat, &key, &value)?;
        self.host.emit("phonechat-chats-changed", json!({}));
        Ok(())
    }

    pub fn contact(&self, chat: &str) -> Result<Contact> {
        Ok(Contact {
            tags: self.store.tags_of(chat)?,
            fields: self.store.fields_of(chat)?,
        })
    }

    pub fn export_contacts(&self, path: &std::path::Path) -> Result<usize> {
        let chats = self.store.chats(None)?;
        std::fs::write(path, crate::export::contacts_csv(&chats))?;
        Ok(chats.len())
    }

    pub fn draft(&self, chat: &str) -> Result<Option<String>> {
        Ok(self.store.draft(chat)?)
    }

    pub fn draft_clear(&self, chat: &str) -> Result<bool> {
        Ok(self.store.draft_clear(chat)?)
    }

    pub fn tags(&self) -> Result<Vec<(String, u32)>> {
        Ok(self.store.tag_counts()?)
    }

    fn still_wanted(&self, msg: &MessageRow) -> Result<bool> {
        let on = self.store.limits()?.enabled;
        if msg.from_me {
            return Ok(on);
        }
        let human = self
            .store
            .last_manual_ts(&msg.chat)?
            .is_some_and(|t| t >= msg.ts);
        Ok(on && !human)
    }
}

enum Done {
    Sent(String),
    Held(&'static str),
}

fn minute_of_day(ts: i64) -> u16 {
    use chrono::{TimeZone, Timelike};
    chrono::Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|t| (t.hour() * 60 + t.minute()) as u16)
        .unwrap_or(0)
}

fn limits_delay(me: &PhoneChat, seed: i64) -> Duration {
    let limits = me.store.limits().unwrap_or_default();
    let span = u64::from(limits.delay_max_ms.saturating_sub(limits.delay_min_ms)) + 1;
    let jitter = (chrono::Utc::now().timestamp_subsec_nanos() as u64 ^ seed as u64) % span;
    Duration::from_millis(u64::from(limits.delay_min_ms) + jitter)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{MessageKind, Summary};
    use crate::rules::{Match, Trigger};
    use crate::store::NewMessage;

    struct Quiet;
    impl super::super::Host for Quiet {
        fn emit(&self, _event: &str, _payload: serde_json::Value) {}
    }

    const CHAT: &str = "5511@s.whatsapp.net";

    fn inbound(me: &PhoneChat, id: &str, text: &str) {
        let summary = Summary {
            kind: Some(MessageKind::Text),
            text: Some(text.into()),
            ..Default::default()
        };
        me.store
            .with_tx(|tx| {
                tx.upsert_chat(CHAT, None, Some(now()), None, None)?;
                tx.insert_message(&NewMessage {
                    chat: CHAT,
                    id,
                    from_me: false,
                    sender: None,
                    push_name: None,
                    ts: now(),
                    summary: &summary,
                    raw: None,
                    wire_chat: Some(CHAT),
                    wire_sender: None,
                })?;
                tx.mark_live(CHAT, id)
            })
            .unwrap();
    }

    fn rule() -> Rule {
        Rule {
            id: 0,
            name: "preço".into(),
            enabled: true,
            trigger: Trigger::Keyword {
                mode: Match::Contains,
                words: vec!["preço".into()],
            },
            chats: Vec::new(),
            action: Action::Reply {
                text: "Oi {{first_name}}, custa R$ 10".into(),
            },
            cooldown_minutes: 60,
            needs_tag: None,
            ask: None,
            own_messages: false,
            tag: Some("lead".into()),
            follow_ups: vec![rules::FollowUp {
                after_minutes: 10,
                text: "Ficou alguma dúvida, {{first_name}}?".into(),
            }],
        }
    }

    #[test]
    fn engine_stays_off_until_a_rule_and_the_switch_are_on() {
        let dir = tempfile::tempdir().unwrap();
        let me = PhoneChat::new(dir.path().join("pc"), Arc::new(Quiet)).unwrap();
        assert!(!me.rules_on.load(Ordering::SeqCst));
        let saved = me.rule_save(rule()).unwrap();
        assert!(!me.rules_on.load(Ordering::SeqCst));
        inbound(&me, "old", "qual o preço?");
        me.set_limits(Limits {
            enabled: true,
            ..Default::default()
        })
        .unwrap();
        assert!(me.rules_on.load(Ordering::SeqCst));
        me.answer_inbound();
        assert!(me.rule_runs(10).unwrap().is_empty());

        me.store.run_start(saved.id, CHAT, now()).unwrap();
        inbound(&me, "again", "e o preço?");
        me.answer_inbound();
        let log = me.rule_runs(10).unwrap();
        assert_eq!(log[0].outcome, "skipped");
        assert_eq!(log[0].detail.as_deref(), Some("cooldown"));

        inbound(&me, "bye", "SAIR");
        me.answer_inbound();
        let state = me.rules_state().unwrap();
        assert_eq!(state.opt_outs.len(), 1);
        assert_eq!(state.sent_hour, 1);
        assert!(me.opt_in(CHAT).unwrap());

        let live = me.store.live_after(None, 0, false, 1).unwrap().remove(0);
        me.after_answer_local(&saved, &live);
        assert_eq!(me.store.chats(None).unwrap()[0].tags, vec!["lead"]);
        let later = me.store.schedule_next().unwrap().unwrap();
        assert_eq!(later.rule_id, Some(saved.id));
        assert!(later.text.starts_with("Ficou alguma dúvida, "));
        assert!(me.follow_up_allowed(CHAT).unwrap());
        me.store.opt_out(CHAT, 1).unwrap();
        assert!(!me.follow_up_allowed(CHAT).unwrap());
        me.store.opt_in(CHAT).unwrap();
        assert!(me.tag_chat(CHAT, " vip ", true).unwrap());
        assert_eq!(me.tags().unwrap().len(), 2);
        assert!(me.tag_chat(CHAT, "", true).is_err());

        me.store.wait_set(CHAT, saved.id, "cidade", now()).unwrap();
        inbound(&me, "city", "Belo Horizonte");
        me.answer_inbound();
        assert_eq!(
            me.contact(CHAT).unwrap().fields,
            vec![("cidade".to_string(), "Belo Horizonte".to_string())]
        );
        assert_eq!(
            me.rule_runs(1).unwrap()[0].detail.as_deref(),
            Some("captured")
        );
        me.store.wait_set(CHAT, saved.id, "cidade", now()).unwrap();
        inbound(&me, "out", "parar");
        me.answer_inbound();
        assert_eq!(me.contact(CHAT).unwrap().fields[0].1, "Belo Horizonte");
        me.store.opt_in(CHAT).unwrap();
        me.set_field(CHAT, "Cidade", "  Contagem ").unwrap();
        assert_eq!(me.contact(CHAT).unwrap().fields[0].1, "Contagem");
        assert!(me.set_field(CHAT, "first_name", "x").is_err());
        assert_eq!(
            me.rule_test(Some(CHAT), "qual o preço em {{cidade}}?")
                .unwrap()
                .reply
                .as_deref(),
            Some("Oi +5511, custa R$ 10")
        );

        me.pause_rules();
        assert!(!me.follow_up_allowed(CHAT).unwrap());
        assert!(!me.rules_on.load(Ordering::SeqCst));
        assert!(!me.rules_state().unwrap().limits.enabled);
    }

    #[test]
    fn test_box_shows_the_reply_without_sending() {
        let dir = tempfile::tempdir().unwrap();
        let me = PhoneChat::new(dir.path().join("pc"), Arc::new(Quiet)).unwrap();
        me.rule_save(rule()).unwrap();
        let hit = me.rule_test(None, "Qual o PRECO?").unwrap();
        assert_eq!(hit.reply.as_deref(), Some("Oi Ana, custa R$ 10"));
        assert!(hit.rule.is_some() && !hit.opt_out);
        let miss = me.rule_test(None, "bom dia").unwrap();
        assert!(miss.rule.is_none() && miss.reply.is_none());
        assert!(me.rule_test(None, "parar").unwrap().opt_out);
        assert!(me.rule_test(None, "  ").is_err());
        assert!(me.rule_runs(10).unwrap().is_empty());
        let mut bad = rule();
        bad.name = String::new();
        assert_eq!(
            me.rule_save(bad).unwrap_err().to_string(),
            "invalid_rule_name"
        );
    }
    #[test]
    fn in_a_group_only_my_own_answer_is_saved_as_a_field() {
        const GROUP: &str = "1@g.us";
        let dir = tempfile::tempdir().unwrap();
        let me = PhoneChat::new(dir.path().join("pc"), Arc::new(Quiet)).unwrap();
        let mut mine = rule();
        mine.chats = vec![GROUP.into()];
        mine.own_messages = true;
        let saved = me.rule_save(mine).unwrap();
        me.set_limits(Limits {
            enabled: true,
            ..Default::default()
        })
        .unwrap();
        let put = |id: &str, from_me: bool, text: &str| {
            let summary = Summary {
                kind: Some(MessageKind::Text),
                text: Some(text.into()),
                ..Default::default()
            };
            me.store
                .with_tx(|tx| {
                    tx.upsert_chat(GROUP, None, Some(now()), None, None)?;
                    tx.insert_message(&NewMessage {
                        chat: GROUP,
                        id,
                        from_me,
                        sender: (!from_me).then_some("9@s.whatsapp.net"),
                        push_name: None,
                        ts: now(),
                        summary: &summary,
                        raw: None,
                        wire_chat: Some(GROUP),
                        wire_sender: None,
                    })?;
                    tx.mark_live(GROUP, id)
                })
                .unwrap();
        };
        me.store.wait_set(GROUP, saved.id, "cidade", now()).unwrap();
        put("other", false, "Recife");
        me.answer_inbound();
        assert!(me.contact(GROUP).unwrap().fields.is_empty());
        put("mine", true, "Belo Horizonte");
        me.answer_inbound();
        assert_eq!(
            me.contact(GROUP).unwrap().fields,
            vec![("cidade".to_string(), "Belo Horizonte".to_string())]
        );
    }
}
