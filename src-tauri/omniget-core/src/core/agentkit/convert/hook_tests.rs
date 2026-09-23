//! Hook and plugin conversion (F5): the catalog hooks `security/dangerous-
//! command-blocker` and `automation/change-logger` installed into Claude,
//! Codex, Cursor, Copilot, Gemini, Cline, Kiro and OpenCode (plugin) in a fake
//! home + temp project, then removed byte for byte; the shim fed Cursor and
//! Gemini stdin blocks `rm -rf /`; plugins become Gemini/Qwen extensions,
//! Agent Plugins and OpenCode plugins; the observe component.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::hook_common::set_shim_path;
use super::hook_shim::{self, main_with, ShimArgs};
use super::{hook_convert, hook_observe, ConvertCtx, Registry};
use crate::core::agentkit::edit::{self, DocFormat};
use crate::core::agentkit::model::*;
use crate::core::agentkit::parse;
use crate::core::agentkit::plan::{self, ConflictPolicy, PlanRequest, UnitStatus};
use crate::core::agentkit::targets;
use crate::core::agentkit::writer;
use crate::core::agentkit::{Env, Os, Scope};

const DEFAULT_CCT: &str = "/private/tmp/claude-501/-Users-tonho-Documents-projetos-omniget/902a5230-621d-4c1b-8fc8-259f1e480186/scratchpad/cct/cli-tool/components";
const SHIM: &str = "/opt/omniget/omniget-hook-shim";

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(tag: &str) -> TempRoot {
        let p = std::env::temp_dir().join(format!(
            "agentkit-k2-{tag}-{}",
            &uuid::Uuid::new_v4().simple().to_string()[..10]
        ));
        std::fs::create_dir_all(&p).unwrap();
        TempRoot(p.canonicalize().unwrap())
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn snapshot(root: &Path) -> (BTreeMap<String, Vec<u8>>, Vec<String>) {
    let mut files = BTreeMap::new();
    let mut dirs = Vec::new();
    for e in walkdir::WalkDir::new(root).into_iter().flatten() {
        let rel = e.path().strip_prefix(root).unwrap().display().to_string();
        if rel.contains(".omniget-data") {
            continue;
        }
        if e.file_type().is_dir() {
            dirs.push(rel);
        } else {
            files.insert(rel, std::fs::read(e.path()).unwrap());
        }
    }
    (files, dirs)
}

fn cct() -> Option<PathBuf> {
    let p = std::env::var("AGENTKIT_CCT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_CCT));
    p.join("hooks").is_dir().then_some(p)
}

const BLOCKER_PY: &str = r#"import json, re, sys
d = json.load(sys.stdin)
cmd = d.get('tool_input', {}).get('command', '')
if re.search(r'\brm\s+.*\s+/\s*$', cmd):
    print('BLOCKED: rm on root directory', file=sys.stderr)
    sys.exit(2)
sys.exit(0)
"#;

fn raw(pairs: &[(&str, &str)]) -> parse::RawFiles {
    pairs
        .iter()
        .map(|(p, t)| (p.to_string(), t.as_bytes().to_vec()))
        .collect()
}

fn hooks() -> Vec<Component> {
    if let Some(c) = cct() {
        let mut out = Vec::new();
        for (rel, id, cat) in [
            (
                "hooks/security/dangerous-command-blocker.json",
                "cct:hooks/security/dangerous-command-blocker",
                "security",
            ),
            (
                "hooks/automation/change-logger.json",
                "cct:hooks/automation/change-logger",
                "automation",
            ),
        ] {
            let mut h = parse::parse_path(ComponentKind::Hook, &c.join(rel)).unwrap();
            h.id = id.into();
            h.category = Some(cat.into());
            out.push(h);
        }
        return out;
    }
    let mut b = parse::parse_raw(
        ComponentKind::Hook,
        "dangerous-command-blocker.json",
        &raw(&[
            ("dangerous-command-blocker.json", r#"{"description":"block","supportingFiles":[{"source":"dangerous-command-blocker.py","destination":".claude/hooks/dangerous-command-blocker.py","executable":true}],"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"python3 .claude/hooks/dangerous-command-blocker.py"}]}]}}"#),
            ("dangerous-command-blocker.py", BLOCKER_PY),
        ]),
    )
    .unwrap();
    b.id = "cct:hooks/security/dangerous-command-blocker".into();
    let mut l = parse::parse_raw(
        ComponentKind::Hook,
        "change-logger.json",
        &raw(&[
            ("change-logger.json", r#"{"description":"log","supportingFiles":[{"source":"change-logger.py","destination":".claude/hooks/change-logger.py","executable":true}],"hooks":{"PostToolUse":[{"matcher":"Edit","hooks":[{"type":"command","command":"python3 .claude/hooks/change-logger.py"}]},{"matcher":"Write","hooks":[{"type":"command","command":"python3 .claude/hooks/change-logger.py"}]},{"matcher":"MultiEdit","hooks":[{"type":"command","command":"python3 .claude/hooks/change-logger.py"}]},{"matcher":"Bash","hooks":[{"type":"command","command":"python3 .claude/hooks/change-logger.py"}]}]}}"#),
            ("change-logger.py", "print('x')\n"),
        ]),
    )
    .unwrap();
    l.id = "cct:hooks/automation/change-logger".into();
    vec![b, l]
}

fn blocker_script() -> Vec<u8> {
    hooks()[0]
        .files
        .iter()
        .find(|f| f.path.ends_with(".py"))
        .map(|f| f.bytes.clone())
        .unwrap()
}

const TARGETS: [&str; 8] = [
    "claude", "codex", "cursor", "copilot", "gemini", "cline", "kiro", "opencode",
];

fn read(p: PathBuf) -> String {
    std::fs::read_to_string(&p).unwrap_or_else(|_| panic!("missing {}", p.display()))
}

#[test]
fn hooks_install_in_eight_tools_and_uninstall_to_the_byte() {
    set_shim_path(Some(PathBuf::from(SHIM)));
    let root = TempRoot::new("hooks");
    let home = root.0.join("home");
    let project = root.0.join("project");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(project.join(".gemini")).unwrap();
    std::fs::write(
        project.join(".gemini/settings.json"),
        "{\n  \"model\": { \"name\": \"gemini-3-pro\" }\n}\n",
    )
    .unwrap();
    let env = Env::sandbox(&home, Os::Macos);
    let before = snapshot(&root.0);

    let req = PlanRequest {
        components: hooks(),
        targets: TARGETS.iter().map(|s| s.to_string()).collect(),
        scope: Some(Scope::Project),
        project_dir: Some(project.clone()),
        policy: ConflictPolicy::Rename,
        secret_values: BTreeMap::new(),
    };
    let p = plan::plan(&env, req).unwrap();
    for u in &p.units {
        assert!(
            u.error.is_none(),
            "{} → {}: {:?}",
            u.component.name,
            u.target,
            u.error
        );
        assert_eq!(
            u.status,
            UnitStatus::New,
            "{} → {}: {:?}",
            u.component.name,
            u.target,
            u.notes
        );
    }
    let unit = |cid: &str, t: &str| {
        p.units
            .iter()
            .find(|u| u.component.id.ends_with(cid) && u.target == t)
            .unwrap()
            .clone()
    };
    // Gemini/Cline/Kiro/OpenCode convert; Cursor/Copilot read Claude's file
    for t in ["gemini", "cline", "kiro", "opencode"] {
        assert!(
            matches!(
                unit("dangerous-command-blocker", t).compat,
                Compat::Converted | Compat::Degraded { .. }
            ),
            "{t}: {:?}",
            unit("dangerous-command-blocker", t).compat
        );
    }
    assert!(matches!(
        unit("dangerous-command-blocker", "cursor").compat,
        Compat::Native
    ));
    // MultiEdit has no Kiro/Gemini name but the event maps: still converted
    let rep = writer::apply(&env, &p).unwrap();
    assert!(!rep.installed.is_empty());

    // Claude: the catalog JSON as it is
    let cl: Value = serde_json::from_str(&read(project.join(".claude/settings.json"))).unwrap();
    assert_eq!(cl["hooks"]["PreToolUse"][0]["matcher"], "Bash");
    assert_eq!(
        cl["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
        "python3 .claude/hooks/dangerous-command-blocker.py"
    );
    assert!(project
        .join(".claude/hooks/dangerous-command-blocker.py")
        .is_file());
    // Codex: Claude shape in .codex/hooks.json, script in .codex/hooks, Bash kept
    let cx: Value = serde_json::from_str(&read(project.join(".codex/hooks.json"))).unwrap();
    assert_eq!(cx["hooks"]["PreToolUse"][0]["matcher"], "Bash");
    assert!(cx["hooks"]["PreToolUse"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap()
        .contains(".codex/hooks/dangerous-command-blocker.py"));
    // Gemini: BeforeTool, mapped matcher, shim, settings kept
    let gm: Value = serde_json::from_str(&read(project.join(".gemini/settings.json"))).unwrap();
    assert_eq!(gm["model"]["name"], "gemini-3-pro");
    let bt = &gm["hooks"]["BeforeTool"][0];
    assert_eq!(bt["matcher"], "run_shell_command");
    let cmd = bt["hooks"][0]["command"].as_str().unwrap();
    assert!(
        cmd.starts_with(SHIM)
            && cmd.contains("--tool gemini --event PreToolUse --native-event BeforeTool"),
        "{cmd}"
    );
    assert!(
        cmd.contains("'python3 .gemini/hooks/dangerous-command-blocker.py'"),
        "{cmd}"
    );
    let at = gm["hooks"]["AfterTool"].as_array().unwrap();
    assert!(at.iter().any(|g| g["matcher"] == "replace"));
    assert!(at.iter().any(|g| g["matcher"] == "write_file"));
    // Cline: dispatcher + registry
    let disp = read(project.join(".clinerules/hooks/PreToolUse"));
    assert!(
        disp.starts_with("#!/bin/sh\n")
            && disp.contains("--dispatch \"$(dirname \"$0\")/../../.omniget/cline-hooks.json\""),
        "{disp}"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(project.join(".clinerules/hooks/PreToolUse"))
            .unwrap()
            .permissions()
            .mode();
        assert!(mode & 0o111 != 0);
    }
    let reg: Value =
        serde_json::from_str(&read(project.join(".omniget/cline-hooks.json"))).unwrap();
    assert_eq!(reg["hooks"]["PreToolUse"][0]["matcher"], "Bash");
    assert_eq!(
        reg["hooks"]["PreToolUse"][0]["hooks"][0]["event"],
        "PreToolUse"
    );
    assert!(project.join(".clinerules/hooks/PostToolUse").is_file());
    // Kiro: one v1 file per component
    let kr: Value = serde_json::from_str(&read(
        project.join(".kiro/hooks/dangerous-command-blocker.json"),
    ))
    .unwrap();
    assert_eq!(kr["version"], "v1");
    assert_eq!(kr["hooks"][0]["trigger"], "PreToolUse");
    assert_eq!(kr["hooks"][0]["matcher"], "shell");
    assert_eq!(kr["hooks"][0]["action"]["type"], "command");
    // OpenCode: generated plugin calling the shim
    let oc = read(project.join(".opencode/plugins/omniget-dangerous-command-blocker.js"));
    assert!(
        oc.contains("\"tool.execute.before\": async (input, output)"),
        "{oc}"
    );
    assert!(oc.contains(&format!("const SHIM = \"{SHIM}\";")));
    assert!(oc.contains("\"matcher\": \"Bash\""));
    assert!(oc.contains("throw new Error(r.reason)"));
    let ocl = read(project.join(".opencode/plugins/omniget-change-logger.js"));
    assert!(ocl.contains("\"tool.execute.after\""));
    assert!(!ocl.contains("\"tool.execute.before\""));

    // reinstall: nothing new
    let p2 = plan::plan(
        &env,
        PlanRequest {
            components: hooks(),
            targets: TARGETS.iter().map(|s| s.to_string()).collect(),
            scope: Some(Scope::Project),
            project_dir: Some(project.clone()),
            policy: ConflictPolicy::Rename,
            secret_values: BTreeMap::new(),
        },
    )
    .unwrap();
    assert!(p2.units.iter().all(|u| u.status == UnitStatus::Installed));

    // uninstall everything (last installed first): byte-identical. In another
    // order a hooks file shared by two components keeps an empty `hooks: {}`
    // (writer prune_to is per install), see the handoff.
    for r in rep.installed.iter().rev() {
        writer::uninstall(&env, &r.install_id, Some(&project), false).unwrap();
    }
    let after = snapshot(&root.0);
    for (k, v) in &after.0 {
        if !before.0.contains_key(k) {
            eprintln!("LEFT {k}:\n{}", String::from_utf8_lossy(v));
        }
    }
    assert_eq!(
        before.0.keys().collect::<Vec<_>>(),
        after.0.keys().collect::<Vec<_>>()
    );
    assert_eq!(before.0, after.0);
    assert_eq!(before.1, after.1, "folders left behind");
}

fn ctx_for<'a>(
    env: &'a Env,
    project: &'a Path,
    claude: &'a targets::TargetAdapter,
    empty: &'a BTreeMap<String, String>,
) -> ConvertCtx<'a> {
    ConvertCtx {
        env,
        project: Some(project),
        scope: Scope::Project,
        claude,
        name_override: None,
        secret_values: empty,
    }
}

#[test]
fn cursor_and_copilot_native_dialects() {
    set_shim_path(Some(PathBuf::from(SHIM)));
    let env = Env::sandbox(Path::new("/k2/home"), Os::Macos);
    let project = PathBuf::from("/k2/project");
    let claude = targets::target("claude").unwrap();
    let empty = BTreeMap::new();
    let ctx = ctx_for(&env, &project, claude, &empty);
    let hs = hooks();
    // Cursor: flat camelCase entries with version 1
    let cursor = targets::target("cursor").unwrap();
    let files = hook_convert::convert_native(&hs[0], cursor, Scope::Project, &ctx).unwrap();
    let main = &files[0];
    assert_eq!(main.path, project.join(".cursor/hooks.json"));
    let applied = writer::apply_ops(DocFormat::Json, "", &main.ops).unwrap();
    let v: Value = serde_json::from_str(&applied.text).unwrap();
    assert_eq!(v["version"], 1);
    let e = &v["hooks"]["preToolUse"][0];
    assert_eq!(e["matcher"], "Shell");
    assert!(e["command"].as_str().unwrap().contains("--tool cursor --event PreToolUse --native-event preToolUse -- 'python3 .cursor/hooks/dangerous-command-blocker.py'"));
    assert!(files.iter().any(|f| f.path
        == project.join(".cursor/hooks/dangerous-command-blocker.py")
        && f.executable));
    // Copilot: its own file in .github/hooks, bash + timeoutSec shape
    let cp = targets::target("copilot").unwrap();
    let files = hook_convert::convert_native(&hs[1], cp, Scope::Project, &ctx).unwrap();
    assert_eq!(
        files[0].path,
        project.join(".github/hooks/change-logger.json")
    );
    let v: Value = serde_json::from_slice(files[0].content.as_ref().unwrap()).unwrap();
    assert_eq!(v["version"], 1);
    let post = v["hooks"]["postToolUse"].as_array().unwrap();
    assert!(post.iter().any(|e| e["matcher"] == "edit"));
    assert!(post.iter().any(|e| e["matcher"] == "create"));
    assert!(post.iter().any(|e| e["matcher"] == "bash"));
    assert!(post[0]["bash"].as_str().unwrap().contains("--tool copilot"));
    // Kimi (TOML) and Windsurf Cascade too
    let kimi = targets::target("kimi").unwrap();
    let files = hook_convert::convert_native(&hs[0], kimi, Scope::Global, &ctx).unwrap();
    let t = writer::apply_ops(DocFormat::Toml, "# mine\nmodel = \"k2\"\n", &files[0].ops).unwrap();
    assert!(t.text.starts_with("# mine\nmodel = \"k2\"\n"), "{}", t.text);
    let tv = edit::parse_value(DocFormat::Toml, &t.text).unwrap();
    assert_eq!(tv["hooks"][0]["event"], "PreToolUse");
    assert_eq!(tv["hooks"][0]["matcher"], "Bash");
    assert!(tv["hooks"][0]["command"]
        .as_str()
        .unwrap()
        .contains("/k2/home/.kimi-code/hooks/dangerous-command-blocker.py"));
    let ws = targets::target("windsurf").unwrap();
    let files = hook_convert::convert_native(&hs[0], ws, Scope::Project, &ctx).unwrap();
    let w = writer::apply_ops(DocFormat::Json, "", &files[0].ops).unwrap();
    let wv: Value = serde_json::from_str(&w.text).unwrap();
    // Bash matcher keeps only pre_run_command of the four PreToolUse events
    assert_eq!(
        wv["hooks"].as_object().unwrap().keys().collect::<Vec<_>>(),
        vec!["pre_run_command"]
    );
    // global scope: support paths absolute
    let files = hook_convert::convert_native(&hs[0], cursor, Scope::Global, &ctx).unwrap();
    assert!(files
        .iter()
        .any(|f| f.path == PathBuf::from("/k2/home/.cursor/hooks/dangerous-command-blocker.py")));
    assert!(files[0]
        .commands
        .iter()
        .any(|c| c.contains("/k2/home/.cursor/hooks/dangerous-command-blocker.py")));
}

fn shim_run(
    tool: &str,
    event: &str,
    native: &str,
    stdin: Value,
    script: &Path,
) -> hook_shim::Outcome {
    let args = ShimArgs {
        tool: tool.into(),
        event: Some(event.into()),
        native_event: Some(native.into()),
        command: vec![format!("python3 '{}'", script.display())],
        ..Default::default()
    };
    main_with(&args, stdin.to_string().as_bytes(), &|_| None)
}

#[test]
fn shim_blocks_rm_rf_root_for_cursor_and_gemini() {
    if std::process::Command::new("python3")
        .arg("-V")
        .output()
        .is_err()
    {
        eprintln!("python3 missing: skipped");
        return;
    }
    let root = TempRoot::new("shim");
    let script = root.0.join("blocker.py");
    std::fs::write(&script, blocker_script()).unwrap();
    let cwd = root.0.display().to_string();
    // Cursor preToolUse
    let o = shim_run(
        "cursor",
        "PreToolUse",
        "preToolUse",
        json!({"conversation_id":"c","generation_id":"g","hook_event_name":"preToolUse","cursor_version":"2.0",
               "workspace_roots":[cwd],"tool_name":"Shell","tool_input":{"command":"rm -rf /"}}),
        &script,
    );
    let v: Value = serde_json::from_str(&o.stdout).unwrap_or_else(|_| panic!("{o:?}"));
    assert_eq!(v["permission"], "deny", "{o:?}");
    assert!(v["user_message"].as_str().unwrap().contains("BLOCKED"));
    let ok = shim_run(
        "cursor",
        "PreToolUse",
        "preToolUse",
        json!({"hook_event_name":"preToolUse","workspace_roots":[cwd],"tool_name":"Shell","tool_input":{"command":"ls -la"}}),
        &script,
    );
    assert_eq!(ok.exit, 0);
    assert!(ok.stdout.is_empty(), "{ok:?}");
    // Gemini BeforeTool
    let o = shim_run(
        "gemini",
        "PreToolUse",
        "BeforeTool",
        json!({"session_id":"s","transcript_path":"/t","cwd":cwd,"hook_event_name":"BeforeTool","timestamp":"x",
               "tool_name":"run_shell_command","tool_input":{"command":"rm -rf /"}}),
        &script,
    );
    let v: Value = serde_json::from_str(&o.stdout).unwrap_or_else(|_| panic!("{o:?}"));
    assert_eq!(v["decision"], "deny");
    assert!(v["reason"].as_str().unwrap().contains("rm on root"));
    // Cline dispatch: registry → cancel
    let reg = root.0.join("reg.json");
    std::fs::write(
        &reg,
        json!({"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","event":"PreToolUse","command":format!("python3 '{}'", script.display())}]}]}}).to_string(),
    )
    .unwrap();
    let args = ShimArgs {
        tool: "cline".into(),
        native_event: Some("PreToolUse".into()),
        dispatch: Some(reg),
        ..Default::default()
    };
    let o = main_with(
        &args,
        json!({"hookName":"PreToolUse","workspaceRoots":[cwd],"preToolUse":{"toolName":"execute_command","parameters":{"command":"rm -rf /"}}}).to_string().as_bytes(),
        &|_| None,
    );
    let v: Value = serde_json::from_str(&o.stdout).unwrap();
    assert_eq!(v["cancel"], true, "{o:?}");
    // a matcher that does not fit lets it through
    let args = ShimArgs {
        tool: "windsurf".into(),
        event: Some("PreToolUse".into()),
        native_event: Some("pre_read_code".into()),
        matcher: Some("Bash".into()),
        command: vec!["exit 2".into()],
        ..Default::default()
    };
    let o = main_with(
        &args,
        json!({"agent_action_name":"pre_read_code","tool_info":{"file_path":"/x"}})
            .to_string()
            .as_bytes(),
        &|_| None,
    );
    assert_eq!(o.exit, 0, "{o:?}");
}

fn demo_plugin() -> Component {
    let files = raw(&[
        (
            ".claude-plugin/plugin.json",
            r#"{"name":"guard-kit","version":"1.2.0","description":"Guards and helpers","author":{"name":"A"},"license":"MIT"}"#,
        ),
        (
            "skills/careful/SKILL.md",
            "---\nname: careful\ndescription: Be careful\n---\nThink twice.\n",
        ),
        (
            "commands/hello.md",
            "---\ndescription: Say hello\n---\nHello $ARGUMENTS and !`date`\n",
        ),
        (
            "agents/reviewer.md",
            "---\nname: reviewer\ndescription: Reviews\ntools: Read, Grep\n---\nReview.\n",
        ),
        (
            "hooks/hooks.json",
            r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"${CLAUDE_PLUGIN_ROOT}/scripts/check.sh"}]}]}}"#,
        ),
        ("scripts/check.sh", "#!/bin/sh\nexit 0\n"),
        (
            ".mcp.json",
            r#"{"mcpServers":{"docs":{"command":"node","args":["${CLAUDE_PLUGIN_ROOT}/server.js"]}}}"#,
        ),
        ("output-styles/terse.md", "terse\n"),
    ]);
    let mut c =
        parse::parse_raw(ComponentKind::Plugin, ".claude-plugin/plugin.json", &files).unwrap();
    c.id = "local:plugins/guard-kit".into();
    c
}

#[test]
fn claude_plugins_for_gemini_qwen_kiro_and_opencode() {
    set_shim_path(Some(PathBuf::from(SHIM)));
    let env = Env::sandbox(Path::new("/k2/home"), Os::Macos);
    let project = PathBuf::from("/k2/project");
    let claude = targets::target("claude").unwrap();
    let empty = BTreeMap::new();
    let mut ctx = ctx_for(&env, &project, claude, &empty);
    let c = demo_plugin();
    let reg = Registry::global();
    // Gemini extension (global)
    ctx.scope = Scope::Global;
    let g = reg
        .convert(&c, targets::target("gemini").unwrap(), &ctx)
        .unwrap();
    let root = PathBuf::from("/k2/home/.gemini/extensions/guard-kit");
    let by = |files: &[super::PlannedFile], p: PathBuf| {
        files
            .iter()
            .find(|f| f.path == p)
            .unwrap_or_else(|| {
                panic!(
                    "no {} in {:?}",
                    p.display(),
                    files.iter().map(|f| &f.path).collect::<Vec<_>>()
                )
            })
            .clone()
    };
    let m: Value = serde_json::from_slice(
        by(&g.files, root.join("gemini-extension.json"))
            .content
            .as_ref()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(m["name"], "guard-kit");
    assert_eq!(
        m["mcpServers"]["docs"]["args"][0],
        "${extensionPath}/server.js"
    );
    let cmd = String::from_utf8(
        by(&g.files, root.join("commands/hello.toml"))
            .content
            .unwrap(),
    )
    .unwrap();
    assert!(cmd.contains("description = \"Say hello\""), "{cmd}");
    assert!(cmd.contains("Hello {{args}} and !{date}"), "{cmd}");
    let hk: Value = serde_json::from_slice(
        by(&g.files, root.join("hooks/hooks.json"))
            .content
            .as_ref()
            .unwrap(),
    )
    .unwrap();
    let hc = hk["hooks"]["BeforeTool"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap();
    assert!(
        hc.contains("/k2/home/.gemini/extensions/guard-kit/scripts/check.sh"),
        "{hc}"
    );
    assert!(by(&g.files, root.join("scripts/check.sh"))
        .content
        .is_some());
    assert!(
        matches!(g.compat, Compat::Degraded { ref lost } if lost.iter().any(|l| l.contains("output styles")))
    );
    assert!(!g
        .files
        .iter()
        .any(|f| f.path.to_string_lossy().contains(".claude-plugin")));
    // Qwen extension
    let q = reg
        .convert(&c, targets::target("qwen").unwrap(), &ctx)
        .unwrap();
    let qm: Value = serde_json::from_slice(
        by(
            &q.files,
            PathBuf::from("/k2/home/.qwen/extensions/guard-kit/qwen-extension.json"),
        )
        .content
        .as_ref()
        .unwrap(),
    )
    .unwrap();
    assert_eq!(qm["skills"], "skills");
    assert_eq!(qm["commands"], "commands");
    assert_eq!(qm["mcpServers"]["docs"]["command"], "node");
    // Kiro power (Agent Plugins)
    let k = reg
        .convert(&c, targets::target("kiro").unwrap(), &ctx)
        .unwrap();
    let km: Value = serde_json::from_slice(
        by(
            &k.files,
            PathBuf::from("/k2/home/.kiro/powers/guard-kit/plugin.json"),
        )
        .content
        .as_ref()
        .unwrap(),
    )
    .unwrap();
    assert!(km["$schema"]
        .as_str()
        .unwrap()
        .contains("agent-plugins.org"));
    assert_eq!(km["license"], "MIT");
    let mj: Value = serde_json::from_slice(
        by(
            &k.files,
            PathBuf::from("/k2/home/.kiro/powers/guard-kit/mcp.json"),
        )
        .content
        .as_ref()
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        mj["mcpServers"]["docs"]["args"][0],
        "/k2/home/.kiro/powers/guard-kit/server.js"
    );
    assert!(by(
        &k.files,
        PathBuf::from("/k2/home/.kiro/powers/guard-kit/skills/careful/SKILL.md")
    )
    .content
    .is_some());
    // OpenCode (project): skill, plugin, MCP in opencode.json, support files
    ctx.scope = Scope::Project;
    let o = reg
        .convert(&c, targets::target("opencode").unwrap(), &ctx)
        .unwrap();
    let js = String::from_utf8(
        by(
            &o.files,
            project.join(".opencode/plugins/omniget-guard-kit.js"),
        )
        .content
        .unwrap(),
    )
    .unwrap();
    assert!(
        js.contains("/k2/project/.opencode/omniget-plugins/guard-kit/scripts/check.sh"),
        "{js}"
    );
    assert!(
        by(&o.files, project.join(".opencode/skills/careful/SKILL.md"))
            .content
            .is_some()
    );
    assert!(o
        .files
        .iter()
        .any(|f| f.path == project.join("opencode.json") && !f.ops.is_empty()));
    // Claude itself keeps the plugin as-is; a mod stays Claude-only
    let cc = reg.convert(&c, claude, &ctx).unwrap();
    assert!(matches!(cc.compat, Compat::Native));
    if let Some(cct) = cct() {
        let modp = cct.join("mods/security/block-destructive-commands");
        if modp.join(".claude-plugin/plugin.json").is_file() {
            let m = parse::parse_path(ComponentKind::Mod, &modp).unwrap();
            let mc = reg.convert(&m, claude, &ctx).unwrap();
            assert!(mc.files.iter().any(|f| f
                .path
                .starts_with(project.join(".claude/skills/block-destructive-commands"))));
            assert!(mc.files.iter().any(|f| f
                .notes
                .iter()
                .any(|n| n.contains("CLAUDE_CODE_ENABLE_FUNCTION_HOOKS=1"))));
            let gm = reg
                .convert(&m, targets::target("gemini").unwrap(), &ctx)
                .unwrap();
            assert!(matches!(gm.compat, Compat::Unsupported { .. }));
        }
    }
}

#[test]
fn observe_component_per_tool() {
    set_shim_path(Some(PathBuf::from(SHIM)));
    let root = TempRoot::new("observe");
    let env = Env::sandbox(&root.0, Os::Macos);
    let project = root.0.join("p");
    std::fs::create_dir_all(&project).unwrap();
    let tok = hook_observe::ensure_token(&env, "gemini").unwrap();
    assert_eq!(tok.len(), 64);
    assert_eq!(hook_observe::ensure_token(&env, "gemini").unwrap(), tok);
    assert!(hook_observe::check_token(&env.app_data, "gemini", &tok));
    assert!(!hook_observe::check_token(&env.app_data, "gemini", "nope"));
    assert!(!hook_observe::check_token(&env.app_data, "claude", &tok));
    let claude = targets::target("claude").unwrap();
    let empty = BTreeMap::new();
    let ctx = ctx_for(&env, &project, claude, &empty);
    // Gemini: every event it has, wrapped by the Gemini shim, observe inside
    let gem = targets::target("gemini").unwrap();
    let c = hook_observe::observe_component(&env, gem, &Default::default());
    let conv = Registry::global().convert(&c, gem, &ctx).unwrap();
    assert!(
        matches!(conv.compat, Compat::Converted),
        "{:?}",
        conv.compat
    );
    let t = writer::apply_ops(DocFormat::Json, "", &conv.files[0].ops).unwrap();
    let v: Value = serde_json::from_str(&t.text).unwrap();
    for ev in [
        "SessionStart",
        "BeforeTool",
        "AfterTool",
        "AfterAgent",
        "Notification",
        "BeforeAgent",
    ] {
        let cmd = v["hooks"][ev][0]["hooks"][0]["command"]
            .as_str()
            .unwrap_or_else(|| panic!("{ev}"));
        assert!(
            cmd.contains("--tool gemini") && cmd.contains("--observe --as gemini"),
            "{cmd}"
        );
    }
    // Claude: PermissionRequest held with a long timeout
    let cc = hook_observe::observe_component(&env, claude, &Default::default());
    let conv = Registry::global().convert(&cc, claude, &ctx).unwrap();
    let t = writer::apply_ops(DocFormat::Json, "", &conv.files[0].ops).unwrap();
    let v: Value = serde_json::from_str(&t.text).unwrap();
    let pr = &v["hooks"]["PermissionRequest"][0]["hooks"][0];
    assert!(pr["command"]
        .as_str()
        .unwrap()
        .contains("--observe --as claude"));
    assert_eq!(pr["timeout"], 115);
    // the shim reports nothing and lets the tool ask when the app is not there
    let args = hook_shim::parse_args(&[
        "--tool".into(),
        "claude".into(),
        "--observe".into(),
        "--as".into(),
        "claude".into(),
        "--data-dir".into(),
        env.app_data.display().to_string(),
    ])
    .unwrap();
    let o = main_with(&args, json!({"hook_event_name":"PermissionRequest","tool_name":"Bash","tool_input":{"command":"ls"}}).to_string().as_bytes(), &|_| None);
    assert_eq!(o, hook_shim::Outcome::default());
    // an entry for Cursor written into Claude's settings stays quiet inside Claude
    let args = hook_shim::parse_args(&[
        "--tool".into(),
        "claude".into(),
        "--observe".into(),
        "--as".into(),
        "cursor".into(),
    ])
    .unwrap();
    let o = main_with(
        &args,
        json!({"hook_event_name":"Stop"}).to_string().as_bytes(),
        &|k| (k == "CLAUDECODE").then(|| "1".into()),
    );
    assert_eq!(o, hook_shim::Outcome::default());
}
