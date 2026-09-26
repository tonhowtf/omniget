//! Durable external execution ceilings. Local UI creates grants; MCP can only
//! select them. Reserved conversation IDs never fall back to local authority.
use super::{
    ctx::{AssistCtx, Scope},
    db::{AssistDb, Migration},
};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

pub const PREFIX: &str = "external-mcp-";
pub const MIGRATIONS:&[Migration]=&[Migration{module:"external_authority",version:1,sql:"
CREATE TABLE external_grants(id TEXT PRIMARY KEY,principal TEXT NOT NULL,body TEXT NOT NULL,revoked INTEGER NOT NULL DEFAULT 0);
CREATE TABLE external_executions(conversation TEXT PRIMARY KEY,grant_id TEXT NOT NULL REFERENCES external_grants(id),mission TEXT NOT NULL,bot TEXT NOT NULL);
"},Migration{module:"external_authority",version:2,sql:"
CREATE TABLE external_mission_intents(principal TEXT NOT NULL,key TEXT NOT NULL,fingerprint TEXT NOT NULL,mission TEXT NOT NULL REFERENCES missions_missions(id),PRIMARY KEY(principal,key));
"},Migration{module:"external_authority",version:3,sql:"
CREATE TABLE external_model_debits(id TEXT PRIMARY KEY,grant_id TEXT NOT NULL,mission TEXT NOT NULL,amount INTEGER NOT NULL);
"}];

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ceiling {
    pub id: String,
    pub principal: String,
    pub workspace_id: String,
    pub workspace: String,
    #[serde(default)]
    pub workspace_identity: Option<crate::core::secure_files::Identity>,
    pub bots: Vec<String>,
    pub bot_revisions: std::collections::BTreeMap<String, String>,
    pub tools: Vec<String>,
    pub max_tokens: u64,
    pub max_usd: Option<f64>,
}
pub fn external(conversation: &str) -> bool {
    conversation.starts_with(PREFIX)
}
pub fn agent_revision(agent: &crate::core::llm::agent::AgentDef) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(agent).map_err(|_| "INVALID_EXECUTOR")?)
    ))
}
pub fn execution_active(db: &AssistDb, conversation: &str, bot: &str) -> Result<(), String> {
    let active:bool=db.with(|c|c.query_row("SELECT EXISTS(SELECT 1 FROM external_executions e JOIN external_grants g ON g.id=e.grant_id JOIN missions_missions m ON m.id=e.mission WHERE e.conversation=?1 AND e.bot=?2 AND g.revoked=0 AND m.principal=g.principal AND m.state IN ('running','verifying'))",params![conversation,bot],|r|r.get(0)))?;
    if active {
        Ok(())
    } else {
        Err("EXTERNAL_EXECUTION_INACTIVE".into())
    }
}
pub fn check_agent(
    conversation: &str,
    agent: &crate::core::llm::agent::AgentDef,
) -> Result<(), String> {
    if !external(conversation) {
        return Ok(());
    }
    let db = super::db::global()?;
    execution_active(&db, conversation, &agent.id)?;
    let ceiling = resolve(&db, conversation, &agent.id)?;
    if ceiling.bot_revisions.get(&agent.id) != Some(&agent_revision(agent)?) {
        return Err("EXECUTOR_REVISION_CHANGED".into());
    }
    external_runtime(agent)?;
    Ok(())
}
/// Flags a Claude Code binary must list for a projection-only external run:
/// no built-in tools (`--tools ""`), user settings/hooks ignored
/// (`--restricted`), only OmniGet's MCP projection (`--strict-mcp-config`),
/// prompts routed to OmniGet (`--permission-prompt-tool`).
pub const CLAUDE_PROJECTION_FLAGS: &[&str] = &[
    "--tools",
    "--restricted",
    "--mcp-config",
    "--strict-mcp-config",
    "--permission-prompt-tool",
];
/// Runtimes an external mission may run on. Native: every tool call goes
/// through the broker. Claude Code: only in projection-only mode, where it has
/// no built-in tools and reaches files through OmniGet's checked projection;
/// allowed only when the installed binary was probed and lists every flag.
pub fn external_runtime(agent: &crate::core::llm::agent::AgentDef) -> Result<&'static str, String> {
    use crate::core::llm::agent::RuntimeKind;
    match &agent.runtime {
        RuntimeKind::Native => Ok("native"),
        RuntimeKind::Cli { cli, .. } if cli == "claude" => {
            let caps = crate::core::llm::caps::cached_cli("claude")
                .ok_or("EXTERNAL_RUNTIME_CAPS_UNKNOWN")?;
            if caps.missing || !CLAUDE_PROJECTION_FLAGS.iter().all(|f| caps.has_flag(f)) {
                return Err("EXTERNAL_RUNTIME_ISOLATION_REQUIRED".into());
            }
            Ok("cli:claude-projection")
        }
        _ => Err("EXTERNAL_RUNTIME_ISOLATION_REQUIRED".into()),
    }
}
/// Trusted local domain entry, not exposed as an MCP tool.
pub fn grant(db: &AssistDb, mut ceiling: Ceiling) -> Result<String, String> {
    if ceiling.principal.is_empty()
        || ceiling.bots.is_empty()
        || ceiling.max_tokens == 0
        || ceiling.bots.len() > 20
        || ceiling.tools.len() > 100
        || ceiling.max_usd.is_some_and(|v| !v.is_finite() || v < 0.0)
    {
        return Err("INVALID_EXECUTION_GRANT".into());
    }
    if ceiling
        .tools
        .iter()
        .any(|t| t.contains('*') || t.is_empty())
    {
        return Err("EXACT_TOOL_GRANTS_REQUIRED".into());
    }
    if ceiling.bots.iter().any(|b| {
        ceiling
            .bot_revisions
            .get(b)
            .is_none_or(|r| r.len() != 64 || !r.bytes().all(|c| c.is_ascii_hexdigit()))
    }) {
        return Err("EXECUTOR_REVISION_REQUIRED".into());
    }
    let path = std::fs::canonicalize(&ceiling.workspace).map_err(|_| "WORKSPACE_UNAVAILABLE")?;
    let root =
        crate::core::secure_files::Root::open(&path, None).map_err(|_| "WORKSPACE_UNSAFE")?;
    ceiling.workspace = path.to_string_lossy().into_owned();
    ceiling.workspace_identity = Some(root.identity().clone());
    ceiling.id = super::new_id();
    let body = serde_json::to_string(&ceiling).map_err(|_| "INVALID_EXECUTION_GRANT")?;
    db.with(|c| {
        c.execute(
            "INSERT INTO external_grants(id,principal,body) VALUES(?1,?2,?3)",
            params![ceiling.id, ceiling.principal, body],
        )
    })?;
    Ok(ceiling.id)
}
/// Revokes every grant of a client, and in the same transaction archives its
/// external rooms, disables its derived bots and fails their pending tasks.
pub fn revoke_principal(db: &AssistDb, principal: &str) -> Result<(), String> {
    db.tx(|tx| {
        tx.execute(
            "UPDATE external_grants SET revoked=1 WHERE principal=?1",
            [principal],
        )
        .map_err(|e| e.to_string())?;
        super::external_config::retire_revoked(tx)
    })
}
/// Trusted local entry (L4): revokes ONE execution grant and retires what
/// was derived from it. Other grants of the same client stay live.
pub fn revoke(db: &AssistDb, grant_id: &str) -> Result<(), String> {
    db.tx(|tx| {
        let n = tx
            .execute(
                "UPDATE external_grants SET revoked=1 WHERE id=?1",
                [grant_id],
            )
            .map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("EXECUTION_GRANT_NOT_FOUND".into());
        }
        super::external_config::retire_revoked(tx)
    })
}
/// Tools of an external run: the grant's supported tools intersected with the
/// executor's own grants (L2). An owner `Deny` removes the tool, `Ask` stays
/// `Ask`, and a tool the executor has no grant for asks (never `Auto`).
pub fn external_tool_grants(
    own: &[crate::core::llm::agent::ToolGrant],
    ceiling_tools: &[String],
) -> Vec<crate::core::llm::agent::ToolGrant> {
    use crate::core::llm::agent::{GrantMode, ToolGrant, ToolSource};
    let mut out: Vec<ToolGrant> = Vec::new();
    for name in ceiling_tools.iter().filter(|n| supported_tool(n)) {
        if out
            .iter()
            .any(|g| matches!(&g.source,ToolSource::Internal{name:n} if n==name))
        {
            continue;
        }
        let mode = own
            .iter()
            .find(|g| matches!(&g.source,ToolSource::Internal{name:n} if n==name))
            .map(|g| g.mode)
            .unwrap_or(GrantMode::Ask);
        if mode == GrantMode::Deny {
            continue;
        }
        out.push(ToolGrant {
            source: ToolSource::Internal { name: name.clone() },
            mode,
        });
    }
    out
}
pub fn get(db: &AssistDb, id: &str, principal: &str) -> Result<Ceiling, String> {
    let body: Option<String> = db.with(|c| {
        c.query_row(
            "SELECT body FROM external_grants WHERE id=?1 AND principal=?2 AND revoked=0",
            params![id, principal],
            |r| r.get(0),
        )
        .optional()
    })?;
    serde_json::from_str(&body.ok_or("EXECUTION_NOT_GRANTED")?)
        .map_err(|_| "INVALID_EXECUTION_GRANT".into())
}
pub fn list(db: &AssistDb, principal: &str) -> Result<Vec<Ceiling>, String> {
    let bodies: Vec<String> = db.with(|c| {
        let mut s =
            c.prepare("SELECT body FROM external_grants WHERE principal=?1 AND revoked=0")?;
        let rows = s.query_map([principal], |r| r.get(0))?;
        rows.collect()
    })?;
    bodies
        .into_iter()
        .map(|b| serde_json::from_str(&b).map_err(|_| "INVALID_EXECUTION_GRANT".into()))
        .collect()
}
pub fn bind(db: &AssistDb, ceiling: &Ceiling, mission: &str, bot: &str) -> Result<String, String> {
    let actual = get(db, &ceiling.id, &ceiling.principal)?;
    // Derived bots (C02) bind under the SAME parent grant id and ledger.
    for_bot(db, &actual, bot)?;
    let conversation = format!("{PREFIX}{}", super::new_id());
    db.with(|c| {
        c.execute(
            "INSERT INTO external_executions VALUES(?1,?2,?3,?4)",
            params![conversation, actual.id, mission, bot],
        )
    })?;
    Ok(conversation)
}
/// Effective authority of `bot` in an external conversation: the raw grant
/// for a granted executor, or the derived subset/pins for a C02 derived bot.
pub fn resolve(db: &AssistDb, conversation: &str, bot: &str) -> Result<Ceiling, String> {
    resolve_with(
        db,
        conversation,
        bot,
        super::external_config::installed_roster().as_deref(),
    )
}
pub fn resolve_with(
    db: &AssistDb,
    conversation: &str,
    bot: &str,
    roster: Option<&dyn super::external_config::BotRoster>,
) -> Result<Ceiling, String> {
    let body:Option<String>=db.with(|c|c.query_row("SELECT g.body FROM external_executions e JOIN external_grants g ON g.id=e.grant_id WHERE e.conversation=?1 AND e.bot=?2 AND g.revoked=0",params![conversation,bot],|r|r.get(0)).optional())?;
    let grant: Ceiling = serde_json::from_str(&body.ok_or("EXECUTION_NOT_GRANTED")?)
        .map_err(|_| "INVALID_EXECUTION_GRANT")?;
    if grant.bots.iter().any(|b| b == bot) {
        return Ok(grant);
    }
    db.with(|c| {
        Ok(super::external_config::derived_ceiling(
            c,
            &grant.id,
            &grant.principal,
            bot,
            roster,
        ))
    })?
}
/// `get`/`list` stay raw (missions compare the raw body transactionally).
/// This resolves a bot under a raw ceiling: itself for a granted executor,
/// or the live derived subset/pins for a derived bot of the same grant.
pub fn for_bot(db: &AssistDb, ceiling: &Ceiling, bot: &str) -> Result<Ceiling, String> {
    for_bot_with(
        db,
        ceiling,
        bot,
        super::external_config::installed_roster().as_deref(),
    )
}
pub fn for_bot_with(
    db: &AssistDb,
    ceiling: &Ceiling,
    bot: &str,
    roster: Option<&dyn super::external_config::BotRoster>,
) -> Result<Ceiling, String> {
    let live = get(db, &ceiling.id, &ceiling.principal)?;
    if live.bots.iter().any(|b| b == bot) {
        return Ok(live);
    }
    db.with(|c| {
        Ok(super::external_config::derived_ceiling(
            c,
            &live.id,
            &live.principal,
            bot,
            roster,
        ))
    })?
}
pub fn context(conversation: &str, bot: &str) -> AssistCtx {
    let ceiling = super::db::global().and_then(|db| resolve(&db, conversation, bot));
    let (principal, scopes) = match ceiling {
        Ok(c) => (c.principal, vec![Scope::Bot { bot: bot.into() }]),
        Err(_) => ("external-unavailable".into(), vec![]),
    };
    AssistCtx {
        principal,
        bot_id: Some(bot.into()),
        conversation_id: Some(conversation.into()),
        run_id: None,
        readable: scopes.clone(),
        writable: scopes,
    }
}
pub fn supported_tool(name: &str) -> bool {
    matches!(
        name,
        "fs_read"
            | "fs_list"
            | "fs_glob"
            | "fs_grep"
            | "fs_write"
            | "fs_edit"
            | "fs_apply_patch"
            | "media_models_list"
            | "media_model_info"
            | "media_estimate"
            | "media_submit"
            | "media_get"
            | "media_collect"
    )
}
pub fn check_tool(conversation: &str, bot: &str, name: &str) -> Result<(), String> {
    if !external(conversation) {
        return Ok(());
    }
    let db = super::db::global()?;
    execution_active(&db, conversation, bot)?;
    let grant = resolve(&db, conversation, bot)?;
    if !grant.tools.iter().any(|t| t == name) {
        return Err("EXTERNAL_TOOL_NOT_GRANTED".into());
    }
    if !supported_tool(name) {
        return Err("EXTERNAL_EXECUTOR_ISOLATION_REQUIRED".into());
    }
    // Executable tools need the worker's file/network sandbox. Until that
    // path is installed they cannot be enabled by a permissive local rule.
    if matches!(name, "shell_exec" | "agent_delegate" | "delegate_task")
        || name.starts_with("reading_")
        || name.starts_with("web_")
        || (name.starts_with("fs_")
            && !matches!(
                name,
                "fs_read"
                    | "fs_list"
                    | "fs_glob"
                    | "fs_grep"
                    | "fs_write"
                    | "fs_edit"
                    | "fs_apply_patch"
            ))
    {
        return Err("EXTERNAL_EXECUTOR_ISOLATION_REQUIRED".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authority_binds_recipient_executor_and_survives_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("assist.db");
        let db = AssistDb::open_with(&path, MIGRATIONS).unwrap();
        let id = grant(
            &db,
            Ceiling {
                id: String::new(),
                principal: "A".into(),
                workspace_id: "root-id".into(),
                workspace: dir.path().to_string_lossy().into_owned(),
                workspace_identity: None,
                bots: vec!["bot-a".into()],
                bot_revisions: std::collections::BTreeMap::from([("bot-a".into(), "a".repeat(64))]),
                tools: vec!["fs_read".into()],
                max_tokens: 10000,
                max_usd: None,
            },
        )
        .unwrap();
        assert!(get(&db, &id, "B").is_err());
        let ceiling = get(&db, &id, "A").unwrap();
        assert!(bind(&db, &ceiling, "mission", "bot-b").is_err());
        let conv = bind(&db, &ceiling, "mission", "bot-a").unwrap();
        drop(db);
        let db = AssistDb::open_with(&path, MIGRATIONS).unwrap();
        assert_eq!(resolve(&db, &conv, "bot-a").unwrap().principal, "A");
        assert!(resolve(&db, &conv, "bot-b").is_err());
        revoke_principal(&db, "A").unwrap();
        assert!(resolve(&db, &conv, "bot-a").is_err());
    }
    #[test]
    fn revoking_one_grant_keeps_the_others_live() {
        let dir = tempfile::tempdir().unwrap();
        let db = AssistDb::open_in_memory().unwrap();
        let mk = |db: &AssistDb| {
            grant(
                db,
                Ceiling {
                    id: String::new(),
                    principal: "A".into(),
                    workspace_id: "w".into(),
                    workspace: dir.path().to_string_lossy().into_owned(),
                    workspace_identity: None,
                    bots: vec!["bot-a".into()],
                    bot_revisions: std::collections::BTreeMap::from([(
                        "bot-a".into(),
                        "a".repeat(64),
                    )]),
                    tools: vec!["fs_read".into()],
                    max_tokens: 10,
                    max_usd: None,
                },
            )
            .unwrap()
        };
        let (g1, g2) = (mk(&db), mk(&db));
        revoke(&db, &g1).unwrap();
        assert!(get(&db, &g1, "A").is_err());
        assert!(get(&db, &g2, "A").is_ok());
        assert_eq!(list(&db, "A").unwrap().len(), 1);
        assert_eq!(
            revoke(&db, "no-such-grant").unwrap_err(),
            "EXECUTION_GRANT_NOT_FOUND"
        );
        // Works on a schema without the C02 tables too.
        let path = dir.path().join("partial.db");
        let partial = AssistDb::open_with(&path, MIGRATIONS).unwrap();
        let g3 = mk(&partial);
        revoke(&partial, &g3).unwrap();
        revoke_principal(&partial, "A").unwrap();
    }
    #[test]
    fn external_tools_never_widen_the_executor_modes() {
        use crate::core::llm::agent::{GrantMode, ToolGrant, ToolSource};
        let g = |n: &str, mode| ToolGrant {
            source: ToolSource::Internal { name: n.into() },
            mode,
        };
        let own = vec![
            g("fs_read", GrantMode::Auto),
            g("fs_write", GrantMode::Deny),
            g("fs_edit", GrantMode::Ask),
            g("shell_exec", GrantMode::Auto),
        ];
        let ceiling: Vec<String> = ["fs_read", "fs_write", "fs_edit", "fs_list", "shell_exec"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let out: Vec<(String, GrantMode)> = external_tool_grants(&own, &ceiling)
            .into_iter()
            .map(|t| (crate::core::llm::broker::grant_key(&t.source), t.mode))
            .collect();
        assert_eq!(
            out,
            vec![
                ("fs_read".into(), GrantMode::Auto),
                ("fs_edit".into(), GrantMode::Ask),
                ("fs_list".into(), GrantMode::Ask)
            ]
        );
    }
    #[test]
    fn missing_external_binding_never_becomes_local_user() {
        let c = context("external-mcp-nonexistent", "bot");
        assert_ne!(c.principal, super::super::ctx::LOCAL_USER);
        assert!(!c.is_user_ui());
        assert!(!c.can_read(&Scope::User));
    }
}

/// Synchronous bounded file transaction. Capabilities are resolved here, never
/// supplied by model arguments; every call rechecks revocation/root identity.
pub fn execute_files(
    conversation: &str,
    bot: &str,
    name: &str,
    args: &serde_json::Value,
) -> Result<String, String> {
    check_tool(conversation, bot, name)?;
    let db = super::db::global()?;
    let ceiling = resolve(&db, conversation, bot)?;
    let identity = ceiling
        .workspace_identity
        .as_ref()
        .ok_or("WORKSPACE_IDENTITY_REQUIRED")?;
    let workspace = std::path::Path::new(&ceiling.workspace);
    crate::core::secure_files::Root::open(workspace, Some(identity))
        .map_err(|_| "WORKSPACE_CHANGED")?;
    // All external file tools deny the reserved staging prefix, including
    // discovery under a broader grant. No external process has access here.
    let parent = workspace.parent().ok_or("WORKSPACE_PARENT_REQUIRED")?;
    let staging = tempfile::Builder::new()
        .prefix(".omniget-private-")
        .tempdir_in(parent)
        .map_err(|_| "STAGING_UNAVAILABLE")?;
    let stage = crate::core::secure_files::Root::open(staging.path(), None)
        .map_err(|_| "STAGING_UNAVAILABLE")?;
    let files = super::external_files::ExternalFiles::open(
        workspace,
        identity,
        staging.path(),
        stage.identity(),
    )?;
    check_tool(conversation, bot, name)?;
    let result = files.execute(name, args)?;
    check_tool(conversation, bot, name)?;
    serde_json::to_string(&result).map_err(|_| "FILE_RESPONSE_ENCODING".into())
}

/// Charge before each provider request, including retries. Conservative debits
/// survive crashes and are never refunded without authoritative usage evidence.
pub fn reserve_model(conversation: &str, bot: &str, amount: u64) -> Result<(), String> {
    reserve_model_id(conversation, bot, amount).map(|_| ())
}
/// Replaces a conservative debit by the usage the provider reported for that
/// request, never raising it (an unreported request keeps its upper bound).
pub fn settle_model(debit: &str, actual_tokens: u64) -> Result<(), String> {
    if debit.is_empty() || actual_tokens == 0 {
        return Ok(());
    }
    let db = super::db::global()?;
    db.with(|c| {
        c.execute(
            "UPDATE external_model_debits SET amount=MIN(amount,?2) WHERE id=?1",
            params![debit, actual_tokens as i64],
        )
    })?;
    Ok(())
}
/// For a CLI run (its own tool loop inside one request): the debit becomes the
/// usage the CLI reported, even above the up-front bound, so the shared
/// ledger reflects what really ran; the next request is refused if over.
pub fn settle_model_actual(debit: &str, actual_tokens: u64) -> Result<(), String> {
    if debit.is_empty() || actual_tokens == 0 {
        return Ok(());
    }
    let db = super::db::global()?;
    db.with(|c| {
        c.execute(
            "UPDATE external_model_debits SET amount=?2 WHERE id=?1",
            params![debit, actual_tokens.min(i64::MAX as u64) as i64],
        )
    })?;
    Ok(())
}
/// [`reserve_model`] returning the debit id to settle after the request.
pub fn reserve_model_id(conversation: &str, bot: &str, amount: u64) -> Result<String, String> {
    if !external(conversation) {
        return Ok(String::new());
    }
    if amount == 0 || amount > i64::MAX as u64 {
        return Err("EXTERNAL_BUDGET_EXCEEDED".into());
    }
    let db = super::db::global()?;
    db.tx(|tx|{
        let (body,mission):(String,String)=tx.query_row("SELECT g.body,e.mission FROM external_executions e JOIN external_grants g ON g.id=e.grant_id WHERE e.conversation=?1 AND e.bot=?2 AND g.revoked=0",params![conversation,bot],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|"EXECUTION_NOT_GRANTED")?;
        let grant:Ceiling=serde_json::from_str(&body).map_err(|_|"INVALID_EXECUTION_GRANT")?;
        if grant.max_usd.is_some(){return Err("USD_ESTIMATE_REQUIRED".into());}
        // Derived bots debit the parent grant's ledger; still check they are live.
        if !grant.bots.iter().any(|b|b==bot){super::external_config::derived_ceiling(tx,&grant.id,&grant.principal,bot,super::external_config::installed_roster().as_deref())?;}
        let (state,budget,principal):(String,String,String)=tx.query_row("SELECT state,budget,principal FROM missions_missions WHERE id=?1",[&mission],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_|"MISSION_UNAVAILABLE")?;
        if principal!=grant.principal||!matches!(state.as_str(),"running"|"verifying"){return Err("MISSION_NOT_ACTIVE".into());}
        let budget:super::missions::MissionBudget=serde_json::from_str(&budget).map_err(|_|"INVALID_MISSION_BUDGET")?;
        let mission_cap=budget.tokens.ok_or("EXTERNAL_BUDGET_REQUIRED")?;
        let used:u64=tx.query_row("SELECT COALESCE(SUM(amount),0) FROM external_model_debits WHERE grant_id=?1",[&grant.id],|r|r.get(0)).map_err(|_|"BUDGET_UNAVAILABLE")?;
        let mission_used:u64=tx.query_row("SELECT COALESCE(SUM(amount),0) FROM external_model_debits WHERE mission=?1",[&mission],|r|r.get(0)).map_err(|_|"BUDGET_UNAVAILABLE")?;
        // Say which limit ran out: the grant's shared ceiling or this mission's cap.
        if used.checked_add(amount).is_none_or(|n|n>grant.max_tokens){return Err(format!("EXTERNAL_BUDGET_EXCEEDED: grant {used} of {} tokens used, this request needs {amount}",grant.max_tokens));}
        if mission_used.checked_add(amount).is_none_or(|n|n>mission_cap){return Err(format!("EXTERNAL_MISSION_BUDGET_EXCEEDED: mission {mission_used} of {mission_cap} tokens used, this request needs {amount}"));}
        let id=super::new_id();
        tx.execute("INSERT INTO external_model_debits(id,grant_id,mission,amount) VALUES(?1,?2,?3,?4)",params![id,grant.id,mission,amount]).map_err(|_|"BUDGET_UNAVAILABLE")?;
        Ok(id)
    })
}
