//! F6 acceptance on disk: the "read-only" profile applied under a fake home to
//! Claude, Codex, Cursor CLI, OpenCode, Gemini and Qwen writes the right file
//! in each tool's dialect and uninstalls byte for byte; the catalog statusline
//! `statusline/context-monitor` (from the claude-code-templates clone) lands on
//! Claude and on Cursor CLI behind the stdin adapter.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::*;
use crate::core::agentkit::edit::{self, DocFormat};
use crate::core::agentkit::plan::UnitStatus;
use crate::core::agentkit::{parse, writer, Os};

const DEFAULT_CCT: &str = "/private/tmp/claude-501/-Users-tonho-Documents-projetos-omniget/902a5230-621d-4c1b-8fc8-259f1e480186/scratchpad/cct/cli-tool/components";

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(tag: &str) -> TempRoot {
        let p = std::env::temp_dir().join(format!(
            "agentkit-{tag}-{}",
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

fn snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    walkdir::WalkDir::new(root)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .filter(|e| !e.path().to_string_lossy().contains(".omniget-data"))
        .map(|e| {
            (
                e.path().strip_prefix(root).unwrap().display().to_string(),
                std::fs::read(e.path()).unwrap(),
            )
        })
        .collect()
}

fn read_doc(p: &Path, f: DocFormat) -> Value {
    let text = std::fs::read_to_string(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    edit::parse_value(f, &text).unwrap()
}

fn has(v: &Value, item: &str) -> bool {
    v.as_array()
        .map(|a| a.iter().any(|x| x.as_str() == Some(item)))
        .unwrap_or(false)
}

#[test]
fn read_only_profile_on_six_tools() {
    let root = TempRoot::new("profile");
    let home = root.0.join("home");
    std::fs::create_dir_all(&home).unwrap();
    // a user setting already there must survive install + uninstall untouched
    std::fs::create_dir_all(home.join(".codex")).unwrap();
    std::fs::write(
        home.join(".codex/config.toml"),
        "# mine\nmodel = \"gpt-5\"\n\n[mcp_servers.foo]\ncommand = \"x\"\n",
    )
    .unwrap();
    let before = snapshot(&home);
    let env = Env::sandbox(&home, Os::Macos);
    let targets: Vec<String> = ["claude", "codex", "cursor", "opencode", "gemini", "qwen"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let plan = plan_profile(
        &env,
        ProfileRequest {
            id: "read-only".into(),
            params: json!({}),
            targets: Some(targets.clone()),
            scope: Some(Scope::Global),
            project_dir: None,
            policy: ConflictPolicy::Rename,
        },
    )
    .unwrap();
    for u in &plan.units {
        assert_eq!(
            u.status,
            UnitStatus::New,
            "{}: {:?} {:?}",
            u.target,
            u.compat,
            u.error
        );
    }
    let report = writer::apply(&env, &plan).unwrap();
    assert_eq!(report.installed.len(), 6);

    let claude = read_doc(&home.join(".claude/settings.json"), DocFormat::Json);
    assert!(has(&claude["permissions"]["deny"], "Bash"));
    assert!(has(&claude["permissions"]["allow"], "Read"));

    let codex_text = std::fs::read_to_string(home.join(".codex/config.toml")).unwrap();
    assert!(codex_text.starts_with("# mine\n"), "{codex_text}");
    let codex = read_doc(&home.join(".codex/config.toml"), DocFormat::Toml);
    assert_eq!(codex["sandbox_mode"], "read-only");
    assert_eq!(codex["approval_policy"], "on-request");
    assert_eq!(codex["model"], "gpt-5");
    assert_eq!(codex["mcp_servers"]["foo"]["command"], "x");
    assert!(
        codex["mcp_servers"]["foo"].get("sandbox_mode").is_none(),
        "{codex_text}"
    );

    let cursor = read_doc(&home.join(".cursor/cli-config.json"), DocFormat::Json);
    assert_eq!(cursor["version"], 1);
    assert!(has(&cursor["permissions"]["deny"], "Shell(*)"), "{cursor}");
    assert!(has(&cursor["permissions"]["deny"], "Write(**)"));
    assert!(has(&cursor["permissions"]["allow"], "Read(**)"));

    let oc = read_doc(
        &home.join(".config/opencode/opencode.json"),
        DocFormat::Jsonc,
    );
    assert_eq!(oc["permission"]["bash"], "deny", "{oc}");
    assert_eq!(oc["permission"]["edit"], "deny");
    assert_eq!(oc["permission"]["read"], "allow");
    assert_eq!(oc["permission"]["grep"], "allow");

    let pol =
        std::fs::read_to_string(home.join(".gemini/policies/omniget-read-only.toml")).unwrap();
    assert!(
        pol.contains("toolName = \"run_shell_command\"\ndecision = \"deny\""),
        "{pol}"
    );
    assert!(pol.contains("toolName = \"write_file\"\ndecision = \"deny\""));
    assert!(pol.contains("decision = \"allow\""));

    let qwen = read_doc(&home.join(".qwen/settings.json"), DocFormat::Json);
    assert!(has(&qwen["permissions"]["deny"], "Bash"));
    assert!(has(&qwen["permissions"]["deny"], "Edit"));

    // re-planning finds everything installed
    let again = plan_profile(
        &env,
        ProfileRequest {
            id: "read-only".into(),
            params: json!({}),
            targets: Some(targets),
            scope: Some(Scope::Global),
            project_dir: None,
            policy: ConflictPolicy::Rename,
        },
    )
    .unwrap();
    assert!(again
        .units
        .iter()
        .all(|u| u.status == UnitStatus::Installed));

    for rec in &report.installed {
        writer::uninstall(&env, &rec.install_id, None, false).unwrap();
    }
    assert_eq!(
        snapshot(&home),
        before,
        "uninstall leaves the home as it was"
    );
}

#[test]
fn provider_profile_maps_endpoint() {
    let root = TempRoot::new("provider");
    let home = root.0.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let env = Env::sandbox(&home, Os::Linux);
    let params = json!({"protocol": "openai", "base_url": "https://api.example.com/v1", "key_env": "EXAMPLE_KEY", "model": "ex-large"});
    let c = component("provider", &params).unwrap();
    let all = targets::load_targets(&env);
    let cp: BTreeMap<String, Compat> = compat(&c, &all).into_iter().collect();
    assert!(matches!(cp["claude"], Compat::Unsupported { .. }));
    let plan = plan_profile(
        &env,
        ProfileRequest {
            id: "provider".into(),
            params,
            targets: Some(vec!["codex".into(), "opencode".into(), "claude".into()]),
            scope: Some(Scope::Global),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(plan.warnings.iter().any(|w| w.starts_with("claude:")));
    writer::apply(&env, &plan).unwrap();
    let codex = read_doc(&home.join(".codex/config.toml"), DocFormat::Toml);
    assert_eq!(codex["model_provider"], "omniget-example-com");
    assert_eq!(
        codex["model_providers"]["omniget-example-com"]["env_key"],
        "EXAMPLE_KEY"
    );
    assert_eq!(codex["model"], "ex-large");
    let oc = read_doc(
        &home.join(".config/opencode/opencode.json"),
        DocFormat::Jsonc,
    );
    assert_eq!(
        oc["provider"]["omniget-example-com"]["options"]["apiKey"],
        "{env:EXAMPLE_KEY}"
    );
    assert_eq!(oc["model"], "omniget-example-com/ex-large");
    assert!(
        !std::fs::read_to_string(home.join(".config/opencode/opencode.json"))
            .unwrap()
            .contains("secret")
    );
}

#[test]
fn context_monitor_statusline_on_claude_and_cursor() {
    let cct =
        PathBuf::from(std::env::var("AGENTKIT_CCT_DIR").unwrap_or_else(|_| DEFAULT_CCT.into()));
    let src = cct.join("settings/statusline/context-monitor.json");
    if !src.is_file() {
        eprintln!("skipping: {} not found", src.display());
        return;
    }
    let mut c = parse::parse_path(ComponentKind::Statusline, &src).unwrap();
    c.id = "cct:statuslines/statusline/context-monitor".into();
    c.category = Some("statusline".into());
    let root = TempRoot::new("statusline");
    let home = root.0.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let before = snapshot(&home);
    let env = Env::sandbox(&home, Os::Macos);
    let plan = plan::plan(
        &env,
        PlanRequest {
            components: vec![c],
            targets: vec!["claude".into(), "cursor".into()],
            scope: Some(Scope::Global),
            project_dir: None,
            policy: ConflictPolicy::Rename,
            secret_values: Default::default(),
        },
    )
    .unwrap();
    for u in &plan.units {
        assert_eq!(
            u.status,
            UnitStatus::New,
            "{}: {:?} {:?}",
            u.target,
            u.compat,
            u.error
        );
    }
    let report = writer::apply(&env, &plan).unwrap();

    let claude_script = home.join(".claude/scripts/context-monitor.py");
    assert!(claude_script.is_file());
    let claude = read_doc(&home.join(".claude/settings.json"), DocFormat::Json);
    let cmd = claude["statusLine"]["command"].as_str().unwrap();
    assert_eq!(cmd, format!("python3 {}", claude_script.display()));
    assert_eq!(claude["statusLine"]["type"], "command");

    let cursor_script = home.join(".cursor/context-monitor.py");
    assert!(cursor_script.is_file());
    assert_eq!(
        std::fs::read(&cursor_script).unwrap(),
        std::fs::read(cct.join("settings/statusline/context-monitor.py")).unwrap()
    );
    let cursor = read_doc(&home.join(".cursor/cli-config.json"), DocFormat::Json);
    let cmd = cursor["statusLine"]["command"].as_str().unwrap();
    assert!(cmd.contains("omniget-hook-shim"), "{cmd}");
    assert!(cmd.contains("--tool cursor --statusline --"), "{cmd}");
    assert_eq!(
        crate::core::agentkit::convert::statusline_stdin::unwrap_command(cmd).unwrap(),
        format!("python3 {}", cursor_script.display())
    );
    assert_eq!(cursor["statusLine"]["type"], "command");

    for rec in &report.installed {
        writer::uninstall(&env, &rec.install_id, None, false).unwrap();
    }
    assert_eq!(snapshot(&home), before);
}

#[test]
fn statusline_and_profile_compat_per_tool() {
    let files: parse::RawFiles = [(
        "x.json".to_string(),
        br#"{"description":"d","statusLine":{"type":"command","command":"echo hi"}}"#.to_vec(),
    )]
    .into_iter()
    .collect();
    let sl = parse::parse_raw(ComponentKind::Statusline, "x.json", &files).unwrap();
    let all = targets::all_targets().to_vec();
    let cp = crate::core::agentkit::convert::compute_compat(&sl, &all);
    assert_eq!(cp["claude"], Compat::Native);
    for t in ["cursor", "qwen", "qoder", "droid", "copilot"] {
        assert_eq!(cp[t], Compat::Converted, "{t}: {:?}", cp[t]);
    }
    for t in ["codex", "gemini", "opencode", "letta"] {
        assert!(
            matches!(cp[t], Compat::Unsupported { .. }),
            "{t}: {:?}",
            cp[t]
        );
    }
    let ro = component("read-only", &json!({})).unwrap();
    let cp: BTreeMap<String, Compat> = compat(&ro, &all).into_iter().collect();
    for t in [
        "claude", "codex", "cursor", "opencode", "gemini", "qwen", "qoder", "devin", "kiro",
        "droid", "goose", "kilo",
    ] {
        assert!(
            !matches!(cp[t], Compat::Unsupported { .. }),
            "{t}: {:?}",
            cp[t]
        );
    }
    assert!(matches!(cp["aider"], Compat::Unsupported { .. }));
}
