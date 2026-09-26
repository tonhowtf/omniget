//! Agent-facing provider tools. Authority derives from the executing conversation,
//! never from model-supplied principal, mission, executable, or credential fields.
use super::{
    authority,
    ctx::{AssistCtx, Scope},
    db::{AssistDb, Migration},
    media_provider::{Cli, Generation, Model},
    tools::AssistToolset,
};
use crate::core::llm::types::ToolSpec;
use async_trait::async_trait;
use rusqlite::{params, OptionalExtension};
use serde::{de::DeserializeOwned, Deserialize};
use serde_json::{json, Value};
use std::{path::Path, sync::Arc};

pub const MIGRATIONS: &[Migration] = &[Migration { module: "media_tools", version: 1, sql: "CREATE TABLE media_tools_config(singleton INTEGER PRIMARY KEY CHECK(singleton=1), cli_path TEXT NOT NULL);" },Migration{module:"media_tools",version:2,sql:"CREATE TABLE media_collections(id TEXT PRIMARY KEY,principal TEXT NOT NULL,mission TEXT NOT NULL REFERENCES missions_missions(id),intent TEXT NOT NULL REFERENCES media_provider_intents(id),artifact_index INTEGER NOT NULL,path TEXT NOT NULL,digest TEXT NOT NULL,bytes INTEGER NOT NULL,mime TEXT NOT NULL,state TEXT NOT NULL,UNIQUE(principal,mission,intent,artifact_index,path));"},Migration{module:"media_tools",version:3,sql:"ALTER TABLE media_tools_config ADD COLUMN interpreter TEXT;"}];
/// Host configuration only. This function is not exposed by any model tool.
pub fn configure_local_cli(db: &AssistDb, path: &Path) -> Result<(), String> {
    // The folder the person picked the CLI from (before resolving symlinks):
    // package managers put the script's interpreter launcher there.
    let picked_dir = path.parent().map(Path::to_path_buf);
    let path = path
        .canonicalize()
        .map_err(|_| "MEDIA_PROVIDER_NOT_CONFIGURED")?;
    let interpreter = resolve_interpreter(&path, picked_dir.as_deref())?;
    Cli::installed_with(&path, interpreter.clone())?;
    let path = path.to_str().ok_or("MEDIA_INVALID_CONFIGURATION")?;
    let interpreter = interpreter.map(|p| p.to_string_lossy().to_string());
    db.with(|c|c.execute("INSERT INTO media_tools_config(singleton,cli_path,interpreter) VALUES(1,?1,?2) ON CONFLICT(singleton) DO UPDATE SET cli_path=excluded.cli_path, interpreter=excluded.interpreter",rusqlite::params![path, interpreter])).map_err(|_|"MEDIA_CONFIGURATION_UNAVAILABLE")?;
    Ok(())
}
/// A `#!/usr/bin/env <name>` script needs `<name>` resolved at trusted
/// configuration time; the adapter never searches an ambient PATH later.
/// Looks next to the picked path, next to the resolved script, then in the
/// standard system/package locations. An absolute shebang needs nothing.
pub fn resolve_interpreter(
    script: &Path,
    picked_dir: Option<&Path>,
) -> Result<Option<std::path::PathBuf>, String> {
    use std::io::Read;
    let mut head = [0u8; 256];
    let n = std::fs::File::open(script)
        .and_then(|mut f| f.read(&mut head))
        .map_err(|_| "MEDIA_PROVIDER_NOT_CONFIGURED")?;
    let first = String::from_utf8_lossy(&head[..n])
        .lines()
        .next()
        .unwrap_or("")
        .to_string();
    let Some(rest) = first.strip_prefix("#!/usr/bin/env ") else {
        return Ok(None);
    };
    let name = rest.split_whitespace().next().unwrap_or("");
    if name.is_empty() || name.contains('/') {
        return Err("MEDIA_INVALID_CONFIGURATION".into());
    }
    let mut dirs: Vec<std::path::PathBuf> = Vec::new();
    if let Some(d) = picked_dir {
        dirs.push(d.to_path_buf());
    }
    if let Some(d) = script.parent() {
        dirs.push(d.to_path_buf());
    }
    for d in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"] {
        dirs.push(d.into());
    }
    for d in dirs {
        let candidate = d.join(name);
        if candidate.is_file() {
            return candidate
                .canonicalize()
                .map(Some)
                .map_err(|_| "MEDIA_PROVIDER_NOT_CONFIGURED".into());
        }
    }
    Err("MEDIA_PROVIDER_INTERPRETER_NOT_FOUND".into())
}
fn configured_cli(db: &AssistDb) -> Result<Cli, String> {
    let row: Option<(String, Option<String>)> = db
        .with(|c| {
            c.query_row(
                "SELECT cli_path, interpreter FROM media_tools_config WHERE singleton=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
        })
        .map_err(|_| "MEDIA_CONFIGURATION_UNAVAILABLE")?;
    let (path, interpreter) = row.ok_or("MEDIA_PROVIDER_NOT_CONFIGURED")?;
    Cli::installed_with(Path::new(&path), interpreter.map(std::path::PathBuf::from))
}
struct MediaTools;
pub fn toolset() -> Arc<dyn AssistToolset> {
    Arc::new(MediaTools)
}
#[async_trait]
impl AssistToolset for MediaTools {
    fn name(&self) -> &'static str {
        "media"
    }
    fn specs(&self) -> Vec<ToolSpec> {
        specs()
    }
    async fn call(&self, ctx: &AssistCtx, tool: &str, input: Value) -> Result<Value, String> {
        let db = super::db::global()?;
        let cli = configured_cli(&db)?;
        dispatch(&db, &cli, ctx, tool, input).await
    }
}
fn schema(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
pub fn specs() -> Vec<ToolSpec> {
    let model = json!({"type":"string","enum":["gpt_image_2_5","seedance_2_5","recraft_v4_1","nano_banana_2","seed_audio"]});
    let generation = schema(
        json!({"model":model,"prompt":{"type":"string","minLength":1,"maxLength":12000},"aspect_ratio":{"type":"string"},"resolution":{"type":"string"},"duration":{"type":"integer","minimum":2,"maximum":30},"quality":{"type":"string"}}),
        &["model", "prompt"],
    );
    let id = json!({"type":"string","minLength":1,"maxLength":128});
    [
        ("media_models_list","List models supported by the restricted media adapter.",schema(json!({}),&[])),
        ("media_model_info","Read supported fields of one media model.",schema(json!({"model":model}),&["model"])),
        ("media_estimate","Quote generation in microcredits. Does not authorize spending.",schema(json!({"generation":generation}),&["generation"])),
        ("media_submit","Submit using an existing quote and locally authorized mission budget. Reuse intent_key after uncertain outcomes; never invent a new key to retry.",schema(json!({"generation":generation,"quote_id":id,"intent_key":id}),&["generation","quote_id","intent_key"])),
        ("media_collect","Collect one owned provider artifact into a new workspace file; never overwrites. Returns a durable hash manifest, not acceptance.",schema(json!({"intent_id":id,"index":{"type":"integer","minimum":0,"maximum":19},"path":{"type":"string","minLength":1,"maxLength":1024}}),&["intent_id","index","path"])),
        ("media_get","Read only a recorded intent owned by this mission. Artifact URLs and credentials are not returned.",schema(json!({"intent_id":id}),&["intent_id"])),
    ].into_iter().map(|(name,description,input_schema)|ToolSpec{name:name.into(),description:description.into(),input_schema}).collect()
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Info {
    model: Model,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Estimate {
    generation: Generation,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Submit {
    generation: Generation,
    quote_id: String,
    intent_key: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Get {
    intent_id: String,
}
fn parse<T: DeserializeOwned>(v: Value) -> Result<T, String> {
    serde_json::from_value(v).map_err(|_| "MEDIA_INVALID_ARGUMENTS".into())
}
fn id(s: &str) -> Result<(), String> {
    if s.is_empty()
        || s.len() > 128
        || !s
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        Err("MEDIA_INVALID_ID".into())
    } else {
        Ok(())
    }
}
fn binding(
    db: &AssistDb,
    ctx: &AssistCtx,
    tool: &str,
    active: bool,
) -> Result<(String, String), String> {
    let conversation = ctx
        .conversation_id
        .as_deref()
        .ok_or("MEDIA_MISSION_SCOPE_REQUIRED")?;
    let bot = ctx
        .bot_id
        .as_deref()
        .ok_or("MEDIA_MISSION_SCOPE_REQUIRED")?;
    if !authority::external(conversation) {
        return Err("MEDIA_EXTERNAL_MISSION_REQUIRED".into());
    }
    let grant = authority::resolve(db, conversation, bot).map_err(|_| "MEDIA_AUTHORITY_DENIED")?;
    let has_scope = |scopes: &[Scope]| {
        scopes
            .iter()
            .any(|s| matches!(s,Scope::Bot{bot:b} if b==bot))
    };
    if grant.principal != ctx.principal
        || !grant.tools.iter().any(|t| t == tool)
        || !has_scope(&ctx.readable)
        || (matches!(tool, "media_submit" | "media_estimate" | "media_collect")
            && !has_scope(&ctx.writable))
    {
        return Err("MEDIA_AUTHORITY_DENIED".into());
    }
    if active {
        authority::execution_active(db, conversation, bot)
            .map_err(|_| "MEDIA_EXECUTION_INACTIVE")?;
    }
    let mission:Option<String>=db.with(|c|c.query_row("SELECT m.id FROM external_executions e JOIN missions_missions m ON m.id=e.mission WHERE e.conversation=?1 AND e.bot=?2 AND m.conversation_id=e.conversation AND m.principal=?3",params![conversation,bot,grant.principal],|r|r.get(0)).optional()).map_err(|_|"MEDIA_STORAGE_ERROR")?;
    Ok((grant.principal, mission.ok_or("MEDIA_AUTHORITY_DENIED")?))
}
/// Caller may inject a trusted configured adapter for tests; model input cannot.
/// Dropping this future drops the provider subprocess future. A submitted intent
/// remains durably unknown/reserved if cancellation prevents recording its ID.
pub async fn dispatch(
    db: &AssistDb,
    cli: &Cli,
    ctx: &AssistCtx,
    tool: &str,
    input: Value,
) -> Result<Value, String> {
    if !specs().iter().any(|s| s.name == tool) {
        return Err("MEDIA_UNKNOWN_TOOL".into());
    }
    if serde_json::to_vec(&input)
        .map_err(|_| "MEDIA_INVALID_ARGUMENTS")?
        .len()
        > 16 * 1024
    {
        return Err("MEDIA_ARGUMENT_LIMIT".into());
    }
    let active = tool != "media_get";
    let (principal, mission) = binding(db, ctx, tool, active)?;
    let out = match tool {
        "media_models_list" => {
            let _: Empty = parse(input)?;
            cli.discover().await?
        }
        "media_model_info" => {
            let a: Info = parse(input)?;
            cli.model_get(a.model).await?
        }
        "media_estimate" => {
            let a: Estimate = parse(input)?;
            serde_json::to_value(cli.cost(db, &principal, &mission, &a.generation).await?)
                .map_err(|_| "MEDIA_RESPONSE_ERROR")?
        }
        "media_submit" => {
            let a: Submit = parse(input)?;
            id(&a.quote_id)?;
            id(&a.intent_key)?;
            serde_json::to_value(
                cli.submit_authorized(
                    db,
                    &principal,
                    &mission,
                    &a.intent_key,
                    &a.quote_id,
                    &a.generation,
                    ctx.conversation_id
                        .as_deref()
                        .ok_or("MEDIA_MISSION_SCOPE_REQUIRED")?,
                    ctx.bot_id
                        .as_deref()
                        .ok_or("MEDIA_MISSION_SCOPE_REQUIRED")?,
                )
                .await?,
            )
            .map_err(|_| "MEDIA_RESPONSE_ERROR")?
        }
        "media_collect" => collect(db, cli, ctx, &principal, &mission, parse(input)?).await?,
        "media_get" => {
            let a: Get = parse(input)?;
            id(&a.intent_id)?;
            serde_json::to_value(cli.get(db, &principal, &mission, &a.intent_id).await?)
                .map_err(|_| "MEDIA_RESPONSE_ERROR")?
        }
        _ => return Err("MEDIA_UNKNOWN_TOOL".into()),
    };
    // Revocation/cancellation while waiting cannot disclose a newly returned
    // result. Any paid intent already admitted remains available for recovery.
    binding(db, ctx, tool, active)?;
    if serde_json::to_vec(&out)
        .map_err(|_| "MEDIA_RESPONSE_ERROR")?
        .len()
        > 64 * 1024
    {
        return Err("MEDIA_RESPONSE_LIMIT".into());
    }
    Ok(out)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Collect {
    intent_id: String,
    index: usize,
    path: String,
}

fn bump_collection_revision(tx: &rusqlite::Transaction, mission: &str) -> Result<(), String> {
    tx.execute(
        "UPDATE missions_missions SET revision=revision+1 WHERE id=?1",
        [mission],
    )
    .map_err(|_| "MEDIA_STORAGE_ERROR")?;
    Ok(())
}
fn collect_authority(
    tx: &rusqlite::Transaction,
    ctx: &AssistCtx,
    mission: &str,
) -> Result<(), String> {
    let synchronous: i64 = tx
        .query_row("PRAGMA synchronous", [], |r| r.get(0))
        .map_err(|_| "MEDIA_STORAGE_ERROR")?;
    if synchronous < 2 {
        return Err("MEDIA_DURABILITY_REQUIRED".into());
    }
    let body:Option<String>=tx.query_row("SELECT g.body FROM external_executions e JOIN external_grants g ON g.id=e.grant_id JOIN missions_missions m ON m.id=e.mission WHERE e.conversation=?1 AND e.bot=?2 AND e.mission=?3 AND m.conversation_id=e.conversation AND m.principal=?4 AND g.principal=m.principal AND g.revoked=0 AND m.state IN ('running','verifying')",params![ctx.conversation_id,ctx.bot_id,mission,ctx.principal],|r|r.get(0)).optional().map_err(|_|"MEDIA_STORAGE_ERROR")?;
    let grant: authority::Ceiling = serde_json::from_str(&body.ok_or("MEDIA_AUTHORITY_DENIED")?)
        .map_err(|_| "MEDIA_AUTHORITY_DENIED")?;
    if !grant.tools.iter().any(|t| t == "media_collect")
        || !grant.bots.iter().any(|b| Some(b) == ctx.bot_id.as_ref())
    {
        return Err("MEDIA_AUTHORITY_DENIED".into());
    }
    Ok(())
}
fn file_matches(
    root: &crate::core::secure_files::Root,
    path: &str,
    digest: &str,
    bytes: i64,
) -> bool {
    root.snapshot(Path::new(path), 64 * 1024 * 1024)
        .is_ok_and(|snapshot| snapshot.bytes == bytes as u64 && snapshot.digest == digest)
}

fn manifest(
    id: &str,
    intent: &str,
    index: usize,
    path: &str,
    digest: &str,
    bytes: i64,
    mime: &str,
) -> Value {
    json!({"artifact_id":id,"intent_id":intent,"index":index,"path":path,"sha256":digest,"bytes":bytes,"mime":mime,"state":"collected","evidence":"file bytes verified; provider status alone is not acceptance"})
}
async fn collect(
    db: &AssistDb,
    cli: &Cli,
    ctx: &AssistCtx,
    principal: &str,
    mission: &str,
    a: Collect,
) -> Result<Value, String> {
    use crate::core::secure_files::Root;
    use sha2::{Digest, Sha256};
    id(&a.intent_id)?;
    if a.index >= 20 {
        return Err("MEDIA_INVALID_INDEX".into());
    }
    super::external_files::validate_path(&a.path)?;
    let grant = authority::resolve(
        db,
        ctx.conversation_id
            .as_deref()
            .ok_or("MEDIA_MISSION_SCOPE_REQUIRED")?,
        ctx.bot_id
            .as_deref()
            .ok_or("MEDIA_MISSION_SCOPE_REQUIRED")?,
    )?;
    let identity = grant
        .workspace_identity
        .as_ref()
        .ok_or("WORKSPACE_IDENTITY_REQUIRED")?;
    let workspace = Path::new(&grant.workspace);
    let root = Root::open(workspace, Some(identity)).map_err(|_| "WORKSPACE_CHANGED")?;
    let previous:Option<(String,String,i64,String,String)>=db.with(|c|c.query_row("SELECT id,digest,bytes,mime,state FROM media_collections WHERE principal=?1 AND mission=?2 AND intent=?3 AND artifact_index=?4 AND path=?5",params![principal,mission,a.intent_id,a.index as i64,a.path],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()).map_err(|_|"MEDIA_STORAGE_ERROR")?;
    if let Some((id, digest, bytes, mime, state)) = &previous {
        if state == "conflict" {
            return Err("MEDIA_DESTINATION_EXISTS".into());
        }
        if file_matches(&root, &a.path, digest, *bytes) {
            db.tx(|tx| {
                collect_authority(tx, ctx, mission)?;
                bump_collection_revision(tx, mission)?;
                tx.execute(
                    "UPDATE media_collections SET state='collected' WHERE id=?1",
                    [id],
                )
                .map_err(|_| "MEDIA_STORAGE_ERROR")?;
                Ok(())
            })?;
            return Ok(manifest(
                id,
                &a.intent_id,
                a.index,
                &a.path,
                digest,
                *bytes,
                mime,
            ));
        }
        if state == "collected" {
            return Err("MEDIA_COLLECTED_ARTIFACT_CHANGED".into());
        }
    }
    let locator = cli
        .owned_artifact_locator(db, principal, mission, &a.intent_id, a.index)
        .await?;
    let data = super::media_retrieval::collect_bytes(locator.as_url()).await?;
    if data.bytes.is_empty()
        || data.bytes.len() > 64 * 1024 * 1024
        || !matches!(
            data.mime.as_str(),
            "image/png"
                | "image/jpeg"
                | "image/webp"
                | "image/gif"
                | "video/mp4"
                | "video/webm"
                | "audio/mpeg"
                | "audio/wav"
                | "audio/ogg"
                | "audio/mp4"
        )
    {
        return Err("MEDIA_ARTIFACT_TYPE_OR_SIZE_INVALID".into());
    }
    let digest = hex::encode(Sha256::digest(&data.bytes));
    if digest != data.digest {
        return Err("MEDIA_ARTIFACT_DIGEST_MISMATCH".into());
    }
    let bytes = data.bytes.len() as i64;
    if let Some((_, old_digest, old_bytes, old_mime, _)) = &previous {
        if old_digest != &digest || *old_bytes != bytes || old_mime != &data.mime {
            return Err("MEDIA_PROVIDER_ARTIFACT_CHANGED".into());
        }
    }
    let artifact_id = previous
        .as_ref()
        .map(|p| p.0.clone())
        .unwrap_or_else(super::new_id);
    // Persist expected bytes before publication: a crash after create can be
    // recovered by a matching immutable snapshot without overwriting anything.
    db.tx(|tx|{collect_authority(tx,ctx,mission)?;bump_collection_revision(tx,mission)?;let changed=tx.execute("INSERT INTO media_collections(id,principal,mission,intent,artifact_index,path,digest,bytes,mime,state) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,'pending') ON CONFLICT(principal,mission,intent,artifact_index,path) DO NOTHING",params![artifact_id,principal,mission,a.intent_id,a.index as i64,a.path,digest,bytes,data.mime]).map_err(|_|"MEDIA_COLLECTION_CONFLICT")?;if changed==0&&previous.is_none(){return Err("MEDIA_COLLECTION_IN_PROGRESS".into());}Ok(())})?;
    let parent = workspace.parent().ok_or("STAGING_UNAVAILABLE")?;
    let staging = tempfile::Builder::new()
        .prefix(".omniget-private-")
        .tempdir_in(parent)
        .map_err(|_| "STAGING_UNAVAILABLE")?;
    let stage = Root::open(staging.path(), None).map_err(|_| "STAGING_UNAVAILABLE")?;
    let publication = db.tx(|tx| {
        collect_authority(tx, ctx, mission)?;
        bump_collection_revision(tx, mission)?;
        let root = Root::open(workspace, Some(identity)).map_err(|_| "WORKSPACE_CHANGED")?;
        root.create_new(&stage, Path::new(&a.path), &data.bytes)
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    "MEDIA_DESTINATION_EXISTS"
                } else {
                    "MEDIA_PUBLICATION_UNKNOWN"
                }
            })?;
        if !file_matches(&root, &a.path, &digest, bytes) {
            return Err("MEDIA_PUBLICATION_UNKNOWN".into());
        }
        tx.execute(
            "UPDATE media_collections SET state='collected' WHERE id=?1",
            [&artifact_id],
        )
        .map_err(|_| "MEDIA_STORAGE_ERROR")?;
        Ok(())
    });
    if let Err(error) = publication {
        if error == "MEDIA_DESTINATION_EXISTS" && previous.is_none() {
            db.with(|c| {
                c.execute(
                    "UPDATE media_collections SET state='conflict' WHERE id=?1",
                    [&artifact_id],
                )
            })
            .map_err(|_| "MEDIA_STORAGE_ERROR")?;
        }
        return Err(error);
    }
    Ok(manifest(
        &artifact_id,
        &a.intent_id,
        a.index,
        &a.path,
        &digest,
        bytes,
        &data.mime,
    ))
}

#[cfg(all(test, unix))]
mod tests {
    use crate::core::assist::{
        authority,
        ctx::{AssistCtx, Scope},
        db::AssistDb,
        media_provider::{self, Cli},
        media_tools,
        missions::*,
    };
    use serde_json::{json, Value};
    use std::os::unix::fs::PermissionsExt;
    fn fixture() -> (tempfile::TempDir, AssistDb, Cli, AssistCtx, String) {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().canonicalize().unwrap();
        let mut migrations = Vec::new();
        migrations.extend_from_slice(MIGRATIONS);
        migrations.extend_from_slice(authority::MIGRATIONS);
        migrations.extend_from_slice(media_provider::MIGRATIONS);
        migrations.extend_from_slice(media_tools::MIGRATIONS);
        let db = AssistDb::open_with(&dir.path().join("assist.db"), &migrations).unwrap();
        let ceiling:authority::Ceiling=serde_json::from_value(json!({"id":"","principal":"A","workspace_id":"w","workspace":workspace,"bots":["bot-a"],"bot_revisions":{"bot-a":"a".repeat(64)},"tools":media_tools::specs().into_iter().map(|s|s.name).collect::<Vec<_>>(),"max_tokens":100,"max_usd":1.0})).unwrap();
        let grant_id = authority::grant(&db, ceiling).unwrap();
        let grant = authority::get(&db, &grant_id, "A").unwrap();
        let c:Criterion=serde_json::from_value(json!({"id":"c","version":1,"kind":"artifact","spec":{"path":"a.txt","contains":["hello"]},"origin":"user","reviewer":"auto","required":true,"title":"artifact"})).unwrap();
        let d = create_external(
            &db,
            NewMission {
                objective: "media fixture".into(),
                bot_id: Some("bot-a".into()),
                workspace: Some(grant.workspace),
                criteria: vec![c],
                budget: MissionBudget {
                    tokens: Some(100),
                    usd: Some(1.0),
                    ..Default::default()
                },
                start: true,
                ..Default::default()
            },
            "A",
            &grant_id,
            "intent",
        )
        .unwrap();
        transition(&db, &d.mission.id, MissionState::Running, "driver", None).unwrap();
        let path = workspace.join("fixture-cli");
        let script=format!("#!/bin/sh\ncase \"$1 $2\" in\n'model list') echo '[{{\"job_type\":\"gpt_image_2_5\",\"secret\":\"CANARY\"}}]' ;;\n'model get') echo '{{\"job_type\":\"gpt_image_2_5\",\"params\":[{{\"name\":\"prompt\"}},{{\"name\":\"api_key\"}}]}}' ;;\n'generate cost') echo '{{\"credits\":0.25}}' ;;\n'generate create') echo x >> '{}'; echo '{{\"job_ids\":[\"job-1\"]}}' ;;\n'generate get') echo '{{\"id\":\"job-1\",\"status\":\"completed\",\"results\":[{{\"url\":\"https://private/?token=CANARY\"}}]}}' ;;\nesac\n",workspace.join("submissions").display());
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        media_tools::configure_local_cli(&db, &path).unwrap();
        let cli = Cli::installed(&path).unwrap();
        let ctx = AssistCtx {
            principal: "A".into(),
            bot_id: Some("bot-a".into()),
            conversation_id: Some(d.mission.conversation_id),
            run_id: None,
            readable: vec![Scope::Bot {
                bot: "bot-a".into(),
            }],
            writable: vec![Scope::Bot {
                bot: "bot-a".into(),
            }],
        };
        (dir, db, cli, ctx, d.mission.id)
    }
    fn generation() -> Value {
        json!({"model":"gpt_image_2_5","prompt":"controlled fixture"})
    }
    #[tokio::test]
    async fn agent_tools_require_budget_and_never_expose_private_response() {
        let (dir, db, cli, ctx, mission) = fixture();
        for (name, args) in [
            ("media_models_list", json!({})),
            ("media_model_info", json!({"model":"gpt_image_2_5"})),
        ] {
            let v = media_tools::dispatch(&db, &cli, &ctx, name, args)
                .await
                .unwrap();
            assert!(!v.to_string().contains("CANARY"));
        }
        let q = media_tools::dispatch(
            &db,
            &cli,
            &ctx,
            "media_estimate",
            json!({"generation":generation()}),
        )
        .await
        .unwrap();
        let args = json!({"generation":generation(),"quote_id":q["id"],"intent_key":"same-key"});
        assert!(
            media_tools::dispatch(&db, &cli, &ctx, "media_submit", args.clone())
                .await
                .unwrap_err()
                .contains("BUDGET_NOT_AUTHORIZED")
        );
        assert!(!dir.path().join("submissions").exists());
        media_provider::set_local_budget(&db, "A", &mission, 250000).unwrap();
        let a = media_tools::dispatch(&db, &cli, &ctx, "media_submit", args.clone())
            .await
            .unwrap();
        let b = media_tools::dispatch(&db, &cli, &ctx, "media_submit", args)
            .await
            .unwrap();
        assert_eq!(a, b);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("submissions"))
                .unwrap()
                .lines()
                .count(),
            1
        );
        cancel(&db, &mission, "test").unwrap();
        let response =
            media_tools::dispatch(&db, &cli, &ctx, "media_get", json!({"intent_id":a["id"]}))
                .await
                .unwrap();
        assert!(!response.to_string().contains("CANARY"));
        assert!(!response.to_string().contains("https://"));
        assert!(media_tools::dispatch(
            &db,
            &cli,
            &ctx,
            "media_estimate",
            json!({"generation":generation()})
        )
        .await
        .is_err());
        authority::revoke_principal(&db, "A").unwrap();
        assert!(
            media_tools::dispatch(&db, &cli, &ctx, "media_get", json!({"intent_id":a["id"]}))
                .await
                .is_err()
        );
    }
    #[tokio::test]
    async fn cannot_select_other_owner_or_inject_parameters() {
        let (_dir, db, cli, mut ctx, _) = fixture();
        assert!(media_tools::dispatch(
            &db,
            &cli,
            &ctx,
            "media_models_list",
            json!({"principal":"B"})
        )
        .await
        .is_err());
        assert!(media_tools::dispatch(
            &db,
            &cli,
            &ctx,
            "media_estimate",
            json!({"generation":{"model":"gpt_image_2_5","prompt":"x","image":"/private"}})
        )
        .await
        .is_err());
        ctx.principal = "B".into();
        assert!(
            media_tools::dispatch(&db, &cli, &ctx, "media_models_list", json!({}))
                .await
                .is_err()
        );
        ctx.principal = "A".into();
        ctx.bot_id = Some("bot-b".into());
        assert!(
            media_tools::dispatch(&db, &cli, &ctx, "media_models_list", json!({}))
                .await
                .is_err()
        );
        ctx.bot_id = Some("bot-a".into());
        ctx.conversation_id = Some("local-conversation".into());
        assert!(
            media_tools::dispatch(&db, &cli, &ctx, "media_models_list", json!({}))
                .await
                .is_err()
        );
    }
    #[tokio::test]
    async fn stale_resolved_authority_cannot_reserve_after_revocation() {
        let (dir, db, cli, ctx, mission) = fixture();
        let generation: media_provider::Generation = serde_json::from_value(generation()).unwrap();
        let quote = cli.cost(&db, "A", &mission, &generation).await.unwrap();
        media_provider::set_local_budget(&db, "A", &mission, 250000).unwrap();
        // Resolve successfully, then revoke before the provider's reservation transaction.
        authority::resolve(&db, ctx.conversation_id.as_deref().unwrap(), "bot-a").unwrap();
        authority::revoke_principal(&db, "A").unwrap();
        assert!(cli
            .submit_authorized(
                &db,
                "A",
                &mission,
                "stale-key",
                &quote.id,
                &generation,
                ctx.conversation_id.as_deref().unwrap(),
                "bot-a"
            )
            .await
            .unwrap_err()
            .contains("AUTHORITY_REVOKED"));
        assert!(!dir.path().join("submissions").exists());
        let reserved: i64 = db
            .with(|c| {
                c.query_row("SELECT reserved FROM media_provider_budgets", [], |r| {
                    r.get(0)
                })
            })
            .unwrap();
        assert_eq!(reserved, 0);
        let count: i64 = db
            .with(|c| {
                c.query_row("SELECT count(*) FROM media_provider_intents", [], |r| {
                    r.get(0)
                })
            })
            .unwrap();
        assert_eq!(count, 0);
    }
    #[tokio::test]
    async fn configured_toolset_survives_database_reopen() {
        let (dir, db, _cli, ctx, _) = fixture();
        drop(db);
        let db =
            std::sync::Arc::new(AssistDb::open_with(&dir.path().join("assist.db"), &[]).unwrap());
        let cli = super::configured_cli(&db).unwrap();
        let out = media_tools::dispatch(&db, &cli, &ctx, "media_models_list", json!({}))
            .await
            .unwrap();
        assert!(!out.to_string().contains("CANARY"));
    }
    #[tokio::test]
    async fn collection_recovers_published_bytes_after_crash_without_network_or_overwrite() {
        use sha2::{Digest, Sha256};
        let (dir, db, cli, ctx, mission) = fixture();
        media_provider::set_local_budget(&db, "A", &mission, 250000).unwrap();
        let q = media_tools::dispatch(
            &db,
            &cli,
            &ctx,
            "media_estimate",
            json!({"generation":generation()}),
        )
        .await
        .unwrap();
        let intent = media_tools::dispatch(
            &db,
            &cli,
            &ctx,
            "media_submit",
            json!({"generation":generation(),"quote_id":q["id"],"intent_key":"recovery"}),
        )
        .await
        .unwrap();
        let bytes = b"recorded fixture bytes";
        let digest = hex::encode(Sha256::digest(bytes));
        db.with(|c|c.execute("INSERT INTO media_collections VALUES('artifact','A',?1,?2,0,'recovered.png',?3,?4,'image/png','pending')",rusqlite::params![mission,intent["id"].as_str().unwrap(),digest,bytes.len() as i64])).unwrap();
        std::fs::write(dir.path().join("recovered.png"), bytes).unwrap();
        let path = db.path().unwrap().to_path_buf();
        drop(db);
        let db = AssistDb::open_with(&path, &[]).unwrap();
        let args = json!({"intent_id":intent["id"],"index":0,"path":"recovered.png"});
        let result = media_tools::dispatch(&db, &cli, &ctx, "media_collect", args.clone())
            .await
            .unwrap();
        assert_eq!(result["sha256"], digest);
        assert_eq!(result["state"], "collected");
        std::fs::write(dir.path().join("recovered.png"), "local edit").unwrap();
        assert!(
            media_tools::dispatch(&db, &cli, &ctx, "media_collect", args)
                .await
                .unwrap_err()
                .contains("ARTIFACT_CHANGED")
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("recovered.png")).unwrap(),
            "local edit"
        );
    }

    #[test]
    fn env_shebang_interpreter_is_resolved_next_to_the_picked_symlink() {
        let base = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("omniget-interp-{}", uuid::Uuid::new_v4()));
        let bin = base.join("bin");
        let pkg = base.join("lib/pkg/bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(
            pkg.join("tool.js"),
            "#!/usr/bin/env fakenode\nconsole.log(1)\n",
        )
        .unwrap();
        std::fs::write(bin.join("fakenode"), "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(pkg.join("tool.js"), bin.join("tool")).unwrap();
        let script = bin.join("tool").canonicalize().unwrap();
        let found = super::resolve_interpreter(&script, Some(&bin)).unwrap();
        assert_eq!(found.unwrap(), bin.join("fakenode").canonicalize().unwrap());
        // Not found anywhere trusted: refused, not left to PATH.
        std::fs::write(
            pkg.join("other.js"),
            "#!/usr/bin/env definitely-not-installed-xyz\n",
        )
        .unwrap();
        assert!(super::resolve_interpreter(&pkg.join("other.js"), None).is_err());
        // Absolute shebang: nothing to resolve.
        std::fs::write(pkg.join("abs.sh"), "#!/bin/sh\n").unwrap();
        assert!(super::resolve_interpreter(&pkg.join("abs.sh"), None)
            .unwrap()
            .is_none());
    }
}
