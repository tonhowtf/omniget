//! MCP servers for every client format of 06 §0C.6 (F1): JSON `mcpServers` /
//! `servers` / `mcp` / `context_servers` / `amp.mcpServers`, Codex/Grok TOML
//! `[mcp_servers.x]`, Vibe `[[mcp_servers]]`, Goose YAML `extensions`, Continue
//! one-YAML-per-server. Secrets are written as variable references (`${VAR}`,
//! `${env:VAR}`, `{env:VAR}`, VS Code `inputs`, Codex `bearer_token_env_var` /
//! `env_vars`), never as values, unless the tool cannot reference a variable.
//! Remote servers on tools without remote transport go through `mcp-remote`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::{json, Map, Value};

use super::{ConvertCtx, Converter, Dedupe, PatchOp, PlannedFile};
use crate::core::agentkit::edit::{self, DocFormat, Seg};
use crate::core::agentkit::model::*;
use crate::core::agentkit::parse;
use crate::core::agentkit::targets::{McpTarget, TargetAdapter};
use crate::core::agentkit::{AgentkitError, Result, Scope};

pub struct McpConverter;

/// How secrets end up in one surface's file.
#[derive(Default)]
struct SecretUse {
    /// VS Code `inputs` entries to add.
    inputs: Vec<Value>,
    /// Env vars a Codex-like tool must forward (`env_vars`).
    forward: Vec<String>,
    losses: Vec<String>,
    notes: Vec<String>,
}

fn input_id(name: &str) -> String {
    name.to_ascii_lowercase().replace('_', "-")
}

/// Replaces `{{secret:NAME}}` markers for this surface.
fn render_secrets(
    s: &str,
    surface: &McpTarget,
    ctx: &ConvertCtx,
    use_: &mut SecretUse,
    file: &str,
) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(a) = rest.find("{{secret:") {
        out.push_str(&rest[..a]);
        let after = &rest[a + 9..];
        let Some(end) = after.find("}}") else {
            out.push_str(&rest[a..]);
            return out;
        };
        let name = &after[..end];
        let mode = surface.secret_mode.as_deref().unwrap_or("plain");
        let rendered = match mode {
            "vscode_inputs" => {
                let id = input_id(name);
                if !use_.inputs.iter().any(|i| i["id"] == json!(id)) {
                    use_.inputs.push(json!({"type": "promptString", "id": id, "description": name, "password": true}));
                }
                format!("${{input:{id}}}")
            }
            "env_ref" if surface.env_ref.is_some() => {
                surface.env_ref.as_deref().unwrap().replace("VAR", name)
            }
            _ => match ctx.secret_values.get(name) {
                Some(v) => {
                    let msg = format!("secret {name} is stored as plain text in {file}");
                    if !use_.losses.contains(&msg) {
                        use_.losses.push(msg);
                    }
                    v.clone()
                }
                None => {
                    let msg = format!("set {name} by hand in {file} (this tool cannot read it from the environment)");
                    if !use_.notes.contains(&msg) {
                        use_.notes.push(msg);
                    }
                    format!("<{name}>")
                }
            },
        };
        out.push_str(&rendered);
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
}

fn is_whole_secret(s: &str) -> Option<&str> {
    s.strip_prefix("{{secret:")
        .and_then(|x| x.strip_suffix("}}"))
        .filter(|n| !n.contains('}'))
}

/// Bearer header → secret name (`Authorization: Bearer {{secret:X}}`).
fn bearer_secret(headers: &BTreeMap<String, String>) -> Option<(String, String)> {
    headers.iter().find_map(|(k, v)| {
        if !k.eq_ignore_ascii_case("authorization") {
            return None;
        }
        let rest = v
            .strip_prefix("Bearer ")
            .or_else(|| v.strip_prefix("bearer "))?;
        is_whole_secret(rest.trim()).map(|n| (k.clone(), n.to_string()))
    })
}

/// Server record for one surface.
fn render_server(
    s: &McpServer,
    surface: &McpTarget,
    ctx: &ConvertCtx,
    use_: &mut SecretUse,
    file: &str,
) -> Value {
    let transport = match s.transport {
        McpTransport::Stdio => "stdio",
        McpTransport::Http => "http",
        McpTransport::Sse => "sse",
    };
    let remote = s.transport != McpTransport::Stdio;
    let bridged = remote
        && (!surface.supports(transport)
            || surface.remote_shape.is_empty()
            || surface.remote_shape == "none");
    let codex_like = matches!(surface.stdio_shape.as_str(), "codex")
        || surface.remote_shape == "codex"
        || surface.secret_mode.as_deref() == Some("bearer_env_var");

    let mut m = Map::new();
    let name_field = surface.name_field.clone().unwrap_or_else(|| "name".into());
    let array_container = surface.container == "array"
        || matches!(surface.stdio_shape.as_str(), "goose" | "vibe" | "continue");
    if array_container {
        m.insert(name_field.clone(), json!(s.name));
    }

    if remote && !bridged {
        let url = render_secrets(
            s.url.as_deref().unwrap_or_default(),
            surface,
            ctx,
            use_,
            file,
        );
        let mut headers: BTreeMap<String, String> = s.headers.clone();
        let mut bearer_var: Option<String> = None;
        if codex_like {
            if let Some((hk, name)) = bearer_secret(&headers) {
                headers.remove(&hk);
                bearer_var = Some(name);
            }
        }
        let mut env_headers: Map<String, Value> = Map::new();
        let mut hmap = Map::new();
        for (k, v) in &headers {
            if codex_like {
                if let Some(n) = is_whole_secret(v) {
                    env_headers.insert(k.clone(), json!(n));
                    continue;
                }
            }
            hmap.insert(
                k.clone(),
                json!(render_secrets(v, surface, ctx, use_, file)),
            );
        }
        let has_headers = !hmap.is_empty();
        let hv = Value::Object(hmap);
        match surface.remote_shape.as_str() {
            "typed" => {
                m.insert(
                    "type".into(),
                    json!(if s.transport == McpTransport::Sse {
                        "sse"
                    } else {
                        "http"
                    }),
                );
                m.insert("url".into(), json!(url));
                if has_headers {
                    m.insert("headers".into(), hv);
                }
            }
            "gemini" => {
                let k = if s.transport == McpTransport::Sse {
                    "url"
                } else {
                    "httpUrl"
                };
                m.insert(k.into(), json!(url));
                if has_headers {
                    m.insert("headers".into(), hv);
                }
            }
            "opencode" => {
                m.insert("type".into(), json!("remote"));
                m.insert("url".into(), json!(url));
                if has_headers {
                    m.insert("headers".into(), hv);
                }
                m.insert("enabled".into(), json!(true));
            }
            "server_url" => {
                m.insert("serverUrl".into(), json!(url));
                if has_headers {
                    m.insert("headers".into(), hv);
                }
            }
            "goose" => {
                m.insert(
                    "type".into(),
                    json!(if s.transport == McpTransport::Sse {
                        "sse"
                    } else {
                        "streamable_http"
                    }),
                );
                m.insert("uri".into(), json!(url));
                if has_headers {
                    m.insert("headers".into(), hv);
                }
                m.insert("enabled".into(), json!(true));
                m.insert("timeout".into(), json!(300));
            }
            "codex" => {
                m.insert("url".into(), json!(url));
                if has_headers {
                    m.insert("http_headers".into(), hv);
                }
                if let Some(b) = &bearer_var {
                    m.insert("bearer_token_env_var".into(), json!(b));
                }
                if !env_headers.is_empty() {
                    m.insert(
                        "env_http_headers".into(),
                        Value::Object(env_headers.clone()),
                    );
                }
            }
            "cline" => {
                m.insert(
                    "type".into(),
                    json!(if s.transport == McpTransport::Sse {
                        "sse"
                    } else {
                        "streamableHttp"
                    }),
                );
                m.insert("url".into(), json!(url));
                if has_headers {
                    m.insert("headers".into(), hv);
                }
                m.insert("disabled".into(), json!(false));
                m.insert("autoApprove".into(), json!([]));
            }
            "roo" | "continue" => {
                m.insert(
                    "type".into(),
                    json!(if s.transport == McpTransport::Sse {
                        "sse"
                    } else {
                        "streamable-http"
                    }),
                );
                m.insert("url".into(), json!(url));
                if has_headers {
                    m.insert("headers".into(), hv);
                }
            }
            "kimi" => {
                if s.transport == McpTransport::Sse {
                    m.insert("transport".into(), json!("sse"));
                }
                m.insert("url".into(), json!(url));
                if has_headers {
                    m.insert("headers".into(), hv);
                }
                if let Some(b) = &bearer_var {
                    m.insert("bearerTokenEnvVar".into(), json!(b));
                }
            }
            "vibe" => {
                m.insert(
                    "transport".into(),
                    json!(if s.transport == McpTransport::Sse {
                        "sse"
                    } else {
                        "streamable-http"
                    }),
                );
                m.insert("url".into(), json!(url));
                if has_headers {
                    m.insert("headers".into(), hv);
                }
                if let Some(b) = &bearer_var {
                    m.insert("api_key_env".into(), json!(b));
                    use_.notes
                        .push("Vibe bearer key field name is not verified".into());
                }
            }
            other => {
                // `url` and unknown shapes
                if other != "url" {
                    use_.notes.push(format!(
                        "remote shape `{other}` written as {{url, headers}}"
                    ));
                }
                m.insert("url".into(), json!(url));
                if has_headers {
                    m.insert("headers".into(), hv);
                }
            }
        }
        if s.oauth.is_some()
            && !matches!(surface.remote_shape.as_str(), "typed" | "gemini" | "codex")
        {
            use_.losses.push("OAuth settings of the server".into());
        }
        return Value::Object(m);
    }

    // stdio (or remote bridged through mcp-remote)
    let (command, args, mut env): (String, Vec<String>, BTreeMap<String, String>) = if bridged {
        let url = s.url.clone().unwrap_or_default();
        let mut args = vec!["-y".to_string(), "mcp-remote@latest".to_string(), url];
        let mut env = BTreeMap::new();
        for (i, (k, v)) in s.headers.iter().enumerate() {
            let var = if i == 0 {
                "MCP_REMOTE_HEADER".to_string()
            } else {
                format!("MCP_REMOTE_HEADER_{i}")
            };
            args.push("--header".into());
            args.push(format!("{k}:${{{var}}}"));
            env.insert(var, v.clone());
        }
        if s.transport == McpTransport::Sse {
            args.push("--transport".into());
            args.push("sse-only".into());
        }
        use_.notes.push(format!(
            "{} has no {transport} transport: bridged with npx mcp-remote",
            surface.surface
        ));
        ("npx".into(), args, env)
    } else {
        (
            s.command.clone().unwrap_or_default(),
            s.args.clone(),
            s.env.clone(),
        )
    };
    let command = render_secrets(&command, surface, ctx, use_, file);
    let args: Vec<String> = args
        .iter()
        .map(|a| render_secrets(a, surface, ctx, use_, file))
        .collect();
    // Codex forwards secrets from the environment by name
    if codex_like {
        let secret_keys: Vec<String> = env
            .iter()
            .filter(|(_, v)| is_whole_secret(v).is_some())
            .map(|(k, _)| k.clone())
            .collect();
        for k in secret_keys {
            let v = env.remove(&k).unwrap_or_default();
            let name = is_whole_secret(&v).unwrap_or(&k).to_string();
            if name != k {
                use_.notes
                    .push(format!("{k} must hold the value of {name}"));
            }
            if !use_.forward.contains(&k) {
                use_.forward.push(k);
            }
        }
    }
    let env_rendered: Map<String, Value> = env
        .iter()
        .map(|(k, v)| {
            (
                k.clone(),
                json!(render_secrets(v, surface, ctx, use_, file)),
            )
        })
        .collect();
    let has_env = !env_rendered.is_empty();
    let envv = Value::Object(env_rendered);
    let shape = surface.stdio_shape.as_str();
    match shape {
        "opencode" => {
            m.insert("type".into(), json!("local"));
            let mut cmd = vec![json!(command)];
            cmd.extend(args.iter().map(|a| json!(a)));
            m.insert("command".into(), Value::Array(cmd));
            if has_env {
                m.insert("environment".into(), envv);
            }
            m.insert("enabled".into(), json!(true));
        }
        "goose" => {
            m.insert("type".into(), json!("stdio"));
            m.insert("cmd".into(), json!(command));
            m.insert("args".into(), json!(args));
            m.insert("envs".into(), if has_env { envv } else { json!({}) });
            m.insert("enabled".into(), json!(true));
            m.insert("timeout".into(), json!(300));
        }
        "vibe" => {
            m.insert("transport".into(), json!("stdio"));
            m.insert("command".into(), json!(command));
            m.insert("args".into(), json!(args));
            if has_env {
                m.insert("env".into(), envv);
            }
        }
        _ => {
            match shape {
                "standard_typed" => {
                    m.insert("type".into(), json!("stdio"));
                }
                "local_typed" => {
                    m.insert("type".into(), json!("local"));
                }
                _ => {}
            }
            m.insert("command".into(), json!(command));
            if !args.is_empty() || shape != "codex" {
                m.insert("args".into(), json!(args));
            }
            if has_env {
                m.insert("env".into(), envv);
            }
            if shape == "codex" && !use_.forward.is_empty() {
                m.insert("env_vars".into(), json!(use_.forward));
            }
            match shape {
                "cline" | "kiro" => {
                    m.insert("disabled".into(), json!(false));
                    m.insert("autoApprove".into(), json!([]));
                }
                "kimi" => {
                    m.insert("enabled".into(), json!(true));
                }
                "standard" | "standard_typed" | "local_typed" | "codex" | "continue" => {}
                other => use_.notes.push(format!(
                    "stdio shape `{other}` written as the standard record"
                )),
            }
        }
    }
    if let Some(cwd) = &s.cwd {
        if matches!(
            shape,
            "standard" | "standard_typed" | "codex" | "kimi" | "cline" | "kiro"
        ) {
            m.insert("cwd".into(), json!(cwd));
        } else {
            use_.losses
                .push("working directory (cwd) of the server".into());
        }
    }
    Value::Object(m)
}

fn surface_path(surface: &McpTarget, scope: Scope, ctx: &ConvertCtx) -> Option<PathBuf> {
    let t = surface.template(scope, ctx.env.os)?;
    ctx.env.expand(t, ctx.project)
}

fn doc_format(surface: &McpTarget) -> DocFormat {
    match surface.format.as_str() {
        "toml" => DocFormat::Toml,
        "yaml" => DocFormat::Yaml,
        "jsonc" => DocFormat::Jsonc,
        _ => DocFormat::Json,
    }
}

impl Converter for McpConverter {
    fn id(&self) -> &'static str {
        "mcp"
    }

    fn supports(&self, kind: ComponentKind, target: &TargetAdapter) -> bool {
        kind == ComponentKind::Mcp && !target.mcp.is_empty()
    }

    fn native_for(&self, c: &Component, target: &TargetAdapter) -> bool {
        target.id == c.origin_tool
    }

    fn convert(
        &self,
        c: &Component,
        target: &TargetAdapter,
        scope: Scope,
        ctx: &ConvertCtx,
    ) -> Result<Vec<PlannedFile>> {
        let ComponentBody::Mcp(spec) = &c.body else {
            return Ok(vec![]);
        };
        let mut out = Vec::new();
        let mut any_surface = false;
        for surface in target.default_mcp() {
            let Some(path) = surface_path(surface, scope, ctx) else {
                continue;
            };
            any_surface = true;
            let file_label = path.display().to_string();
            if surface.format == "yaml_file_per_server" {
                for s in &spec.servers {
                    let mut use_ = SecretUse::default();
                    let rec = render_server(s, surface, ctx, &mut use_, &file_label);
                    let doc = json!({"name": s.name, "version": "0.0.1", "schema": "v1", "mcpServers": [rec]});
                    let file = path.join(format!("{}.yaml", parse::sanitize_name(&s.name)));
                    let mut pf = PlannedFile::write(
                        &target.id,
                        c,
                        file,
                        edit::yaml::to_text(&doc),
                        "mcp server",
                    );
                    pf.losses = use_.losses;
                    pf.notes = use_.notes;
                    pf.commands = vec![server_command_line(s)];
                    pf.primary = true;
                    out.push(pf);
                }
                continue;
            }
            let fmt = doc_format(surface);
            let key: Vec<Seg> = surface.key.iter().map(|k| Seg::Key(k.clone())).collect();
            let mut use_ = SecretUse::default();
            let mut ops = Vec::new();
            let mut commands = Vec::new();
            for s in &spec.servers {
                let rec = render_server(s, surface, ctx, &mut use_, &file_label);
                commands.push(server_command_line(s));
                if surface.container == "array" {
                    let nf = surface.name_field.clone().unwrap_or_else(|| "name".into());
                    ops.push(PatchOp::Append {
                        path: key.clone(),
                        value: rec,
                        dedupe: Dedupe::Fields { fields: vec![nf] },
                    });
                } else {
                    let mut p = key.clone();
                    p.push(Seg::Key(s.name.clone()));
                    ops.push(PatchOp::Set {
                        path: p,
                        value: rec,
                        rename_at: Some(key.len()),
                    });
                }
            }
            for input in std::mem::take(&mut use_.inputs) {
                ops.push(PatchOp::Append {
                    path: vec![Seg::Key("inputs".into())],
                    value: input,
                    dedupe: Dedupe::Fields {
                        fields: vec!["id".into()],
                    },
                });
            }
            let mut pf = PlannedFile::merge(&target.id, c, path, fmt, ops, "mcp server");
            pf.losses = use_.losses;
            pf.notes = use_.notes;
            if let Some(cli) = &surface.prefer_cli {
                pf.notes.push(format!(
                    "{} also has its own command for this: {cli}",
                    target.name
                ));
            }
            pf.commands = commands;
            out.push(pf);
        }
        if !any_surface {
            return Err(AgentkitError::new(
                "AGENTKIT_NO_PATH",
                format!(
                    "{} has no MCP file for scope {}",
                    target.name,
                    scope.as_str()
                ),
            ));
        }
        Ok(out)
    }

    fn import(
        &self,
        kind: ComponentKind,
        target: &TargetAdapter,
        scope: Scope,
        ctx: &ConvertCtx,
    ) -> Result<Vec<Component>> {
        if kind != ComponentKind::Mcp {
            return Ok(vec![]);
        }
        let mut out = Vec::new();
        for surface in &target.mcp {
            let Some(path) = surface_path(surface, scope, ctx) else {
                continue;
            };
            let mut records: Vec<(String, Value, PathBuf)> = Vec::new();
            if surface.format == "yaml_file_per_server" {
                for e in std::fs::read_dir(&path).into_iter().flatten().flatten() {
                    let p = e.path();
                    let Ok(text) = std::fs::read_to_string(&p) else {
                        continue;
                    };
                    let Ok(v) = edit::yaml::parse(&text) else {
                        continue;
                    };
                    for rec in v
                        .get("mcpServers")
                        .and_then(|x| x.as_array())
                        .cloned()
                        .unwrap_or_default()
                    {
                        let name = rec
                            .get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or("server")
                            .to_string();
                        records.push((name, rec, p.clone()));
                    }
                }
            } else {
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                let Ok(v) = edit::parse_value(doc_format(surface), &text) else {
                    continue;
                };
                let key: Vec<Seg> = surface.key.iter().map(|k| Seg::Key(k.clone())).collect();
                match edit::value_at(&v, &key) {
                    Some(Value::Object(m)) => {
                        for (name, rec) in m {
                            records.push((name.clone(), rec.clone(), path.clone()));
                        }
                    }
                    Some(Value::Array(a)) => {
                        let nf = surface.name_field.clone().unwrap_or_else(|| "name".into());
                        for rec in a {
                            let name = rec
                                .get(&nf)
                                .and_then(|n| n.as_str())
                                .unwrap_or("server")
                                .to_string();
                            records.push((name, rec.clone(), path.clone()));
                        }
                    }
                    _ => {}
                }
            }
            for (name, rec, p) in records {
                let server = parse::mcp_from_record(&name, &rec);
                let json_doc = json!({"mcpServers": {name.clone(): rec}});
                let files: parse::RawFiles = [(
                    format!("{}.json", parse::sanitize_name(&name)),
                    serde_json::to_vec_pretty(&json_doc).unwrap_or_default(),
                )]
                .into_iter()
                .collect();
                let entry = files.keys().next().cloned().unwrap_or_default();
                if let Ok(mut comp) = parse::parse_raw(ComponentKind::Mcp, &entry, &files) {
                    comp.body = ComponentBody::Mcp(McpSpec {
                        servers: vec![server],
                    });
                    comp.origin_tool = target.id.clone();
                    super::claude::tag_installed(&mut comp, target, &p);
                    if !out.iter().any(|x: &Component| x.id == comp.id) {
                        out.push(comp);
                    }
                }
            }
        }
        Ok(out)
    }
}

/// What the server runs or connects to, for the "always show the command" rule.
pub fn server_command_line(s: &McpServer) -> String {
    match s.transport {
        McpTransport::Stdio => {
            let mut parts = vec![s.command.clone().unwrap_or_default()];
            parts.extend(s.args.iter().cloned());
            parts.join(" ")
        }
        McpTransport::Http => format!("http {}", s.url.clone().unwrap_or_default()),
        McpTransport::Sse => format!("sse {}", s.url.clone().unwrap_or_default()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::agentkit::convert::ConvertCtx;
    use crate::core::agentkit::targets::target;
    use crate::core::agentkit::{Env, Os};
    use std::path::Path;

    fn comp(json_text: &str) -> Component {
        let files: parse::RawFiles = [("x.json".to_string(), json_text.as_bytes().to_vec())]
            .into_iter()
            .collect();
        parse::parse_raw(ComponentKind::Mcp, "x.json", &files).unwrap()
    }

    fn convert(c: &Component, id: &str) -> Vec<PlannedFile> {
        let env = Env::sandbox(Path::new("/h"), Os::Linux);
        let empty = BTreeMap::new();
        let ctx = ConvertCtx {
            env: &env,
            project: Some(Path::new("/p")),
            scope: Scope::Project,
            claude: target("claude").unwrap(),
            name_override: None,
            secret_values: &empty,
        };
        McpConverter
            .convert(c, target(id).unwrap(), Scope::Project, &ctx)
            .unwrap()
    }

    fn set_value(pf: &PlannedFile) -> Value {
        match &pf.ops[0] {
            PatchOp::Set { value, .. } | PatchOp::Append { value, .. } => value.clone(),
            _ => panic!(),
        }
    }

    #[test]
    fn shapes_per_tool() {
        let remote = comp(
            r#"{"mcpServers":{"hf":{"url":"https://huggingface.co/mcp","headers":{"Authorization":"Bearer <YOUR_HF_TOKEN>"}}}}"#,
        );
        let v = set_value(&convert(&remote, "claude")[0]);
        assert_eq!(
            v,
            json!({"type": "http", "url": "https://huggingface.co/mcp", "headers": {"Authorization": "Bearer ${HF_TOKEN}"}})
        );
        let v = set_value(&convert(&remote, "codex")[0]);
        assert_eq!(
            v,
            json!({"url": "https://huggingface.co/mcp", "bearer_token_env_var": "HF_TOKEN"})
        );
        let v = set_value(&convert(&remote, "cursor")[0]);
        assert_eq!(v["headers"]["Authorization"], "Bearer ${env:HF_TOKEN}");
        let files = convert(&remote, "copilot");
        let vs = files
            .iter()
            .find(|f| f.path.ends_with(".vscode/mcp.json"))
            .unwrap();
        assert_eq!(
            set_value(vs)["headers"]["Authorization"],
            "Bearer ${input:hf-token}"
        );
        assert!(vs.ops.iter().any(|o| matches!(o, PatchOp::Append { path, .. } if path == &vec![Seg::Key("inputs".into())])));
        let v = set_value(&convert(&remote, "opencode")[0]);
        assert_eq!(v["type"], "remote");
        assert_eq!(v["headers"]["Authorization"], "Bearer {env:HF_TOKEN}");
        let v = set_value(&convert(&remote, "gemini")[0]);
        assert_eq!(v["httpUrl"], "https://huggingface.co/mcp");

        let stdio = comp(
            r#"{"mcpServers":{"gh":{"command":"npx","args":["-y","gh-mcp"],"env":{"GITHUB_TOKEN":"<token>","LOG":"1"}}}}"#,
        );
        let v = set_value(&convert(&stdio, "codex")[0]);
        assert_eq!(
            v,
            json!({"command": "npx", "args": ["-y", "gh-mcp"], "env": {"LOG": "1"}, "env_vars": ["GITHUB_TOKEN"]})
        );
        let v = set_value(&convert(&stdio, "opencode")[0]);
        assert_eq!(v["command"], json!(["npx", "-y", "gh-mcp"]));
        assert_eq!(v["environment"]["GITHUB_TOKEN"], "{env:GITHUB_TOKEN}");
    }

    #[test]
    fn sse_on_codex_goes_through_mcp_remote() {
        let sse = comp(r#"{"mcpServers":{"s":{"type":"sse","url":"https://x/sse"}}}"#);
        let pf = &convert(&sse, "codex")[0];
        let v = set_value(pf);
        assert_eq!(v["command"], "npx");
        assert!(v["args"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a == "mcp-remote@latest"));
        assert!(pf.notes.iter().any(|n| n.contains("mcp-remote")));
    }
}
