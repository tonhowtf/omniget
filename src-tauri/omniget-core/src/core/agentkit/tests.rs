//! Integration test of the F0 acceptance: install an agent, two MCP servers
//! (stdio + remote, both with secrets), a hook with a support script and a
//! skill folder from the claude-code-templates clone into Claude, Codex (MCP in
//! TOML), Cursor, OpenCode (MCP in JSONC with comments) and VS Code/Copilot, in
//! a temporary project under a fake home; reinstall without duplicating; update;
//! uninstall leaving every byte as it was.
//!
//! The clone path comes from `AGENTKIT_CCT_DIR` (default: the session scratchpad
//! clone). Without it the test builds equivalent components inline.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::edit::{self, DocFormat};
use super::model::*;
use super::parse;
use super::plan::{self, ConflictPolicy, PlanRequest, UnitStatus};
use super::writer;
use super::{Env, Os, Scope};

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

/// Every file (bytes) and folder under `root`, except OmniGet's own data dir.
fn snapshot(root: &Path) -> (BTreeMap<String, Vec<u8>>, Vec<String>) {
    let mut files = BTreeMap::new();
    let mut dirs = Vec::new();
    for e in walkdir::WalkDir::new(root).into_iter().flatten() {
        let rel = e.path().strip_prefix(root).unwrap().display().to_string();
        if rel.contains(".omniget-data") {
            continue;
        }
        if e.file_type().is_symlink() {
            let t = std::fs::read_link(e.path()).unwrap();
            files.insert(rel, format!("symlink:{}", t.display()).into_bytes());
        } else if e.file_type().is_dir() {
            dirs.push(rel);
        } else {
            files.insert(rel, std::fs::read(e.path()).unwrap());
        }
    }
    (files, dirs)
}

fn load(cct: &Path, kind: ComponentKind, rel: &str, id: &str, category: &str) -> Component {
    let mut c = parse::parse_path(kind, &cct.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"));
    c.id = id.to_string();
    c.category = Some(category.to_string());
    c
}

fn inline_components() -> Vec<Component> {
    let f = |pairs: &[(&str, &str)]| -> parse::RawFiles {
        pairs
            .iter()
            .map(|(p, t)| (p.to_string(), t.as_bytes().to_vec()))
            .collect()
    };
    let mut out = Vec::new();
    let mut a = parse::parse_raw(ComponentKind::Agent, "frontend-developer.md", &f(&[("frontend-developer.md", "---\nname: frontend-developer\ndescription: Builds UIs\ntools: Read, Write, Edit, Bash\n---\n\nYou are a frontend dev.\n")])).unwrap();
    a.id = "cct:agents/development-team/frontend-developer".into();
    out.push(a);
    let mut m = parse::parse_raw(ComponentKind::Mcp, "github-official.json", &f(&[("github-official.json", r#"{"mcpServers":{"github-official":{"command":"docker","args":["run","-i","--rm","-e","GITHUB_PERSONAL_ACCESS_TOKEN","ghcr.io/github/github-mcp-server"],"env":{"GITHUB_PERSONAL_ACCESS_TOKEN":"<your-github-token>"}}}}"#)])).unwrap();
    m.id = "cct:mcps/devtools/github-official".into();
    out.push(m);
    let mut r = parse::parse_raw(ComponentKind::Mcp, "huggingface.json", &f(&[("huggingface.json", r#"{"mcpServers":{"huggingface":{"url":"https://huggingface.co/mcp","headers":{"Authorization":"Bearer <YOUR_HF_TOKEN>"}}}}"#)])).unwrap();
    r.id = "cct:mcps/devtools/huggingface".into();
    out.push(r);
    let mut h = parse::parse_raw(
        ComponentKind::Hook,
        "change-logger.json",
        &f(&[
            ("change-logger.json", r#"{"description":"log","supportingFiles":[{"source":"change-logger.py","destination":".claude/hooks/change-logger.py","executable":true}],"hooks":{"PostToolUse":[{"matcher":"Edit","hooks":[{"type":"command","command":"python3 .claude/hooks/change-logger.py"}]},{"matcher":"Write","hooks":[{"type":"command","command":"python3 .claude/hooks/change-logger.py"}]}]}}"#),
            ("change-logger.py", "print('x')\n"),
        ]),
    )
    .unwrap();
    h.id = "cct:hooks/automation/change-logger".into();
    out.push(h);
    let mut s = parse::parse_raw(
        ComponentKind::Skill,
        "design-to-code/SKILL.md",
        &f(&[
            (
                "design-to-code/SKILL.md",
                "---\nname: design-to-code\ndescription: Turns designs into code\n---\nBody\n",
            ),
            ("design-to-code/scripts/run.sh", "#!/bin/sh\necho hi\n"),
        ]),
    )
    .unwrap();
    s.id = "cct:skills/design-to-code".into();
    out.push(s);
    out
}

fn components() -> Vec<Component> {
    let cct = std::env::var("AGENTKIT_CCT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_CCT));
    if !cct.join("agents").is_dir() {
        return inline_components();
    }
    vec![
        load(
            &cct,
            ComponentKind::Agent,
            "agents/development-team/frontend-developer.md",
            "cct:agents/development-team/frontend-developer",
            "development-team",
        ),
        load(
            &cct,
            ComponentKind::Mcp,
            "mcps/devtools/github-official.json",
            "cct:mcps/devtools/github-official",
            "devtools",
        ),
        load(
            &cct,
            ComponentKind::Mcp,
            "mcps/devtools/huggingface.json",
            "cct:mcps/devtools/huggingface",
            "devtools",
        ),
        load(
            &cct,
            ComponentKind::Hook,
            "hooks/automation/change-logger.json",
            "cct:hooks/automation/change-logger",
            "automation",
        ),
        load(
            &cct,
            ComponentKind::Skill,
            "skills/design-to-code",
            "cct:skills/design-to-code",
            "design",
        ),
    ]
}

const TARGETS: [&str; 5] = ["claude", "codex", "cursor", "opencode", "copilot"];

fn seed(project: &Path, home: &Path) {
    let w = |p: PathBuf, t: &str| {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, t).unwrap();
    };
    w(project.join("opencode.json"), "{\n  // my opencode config\n  \"$schema\": \"https://opencode.ai/config.json\",\n  \"mcp\": {\n    // keep this server\n    \"existing\": { \"type\": \"local\", \"command\": [\"x\"] },\n  },\n}\n");
    w(project.join(".codex").join("config.toml"), "# my codex project config\nmodel = \"gpt-5\" # pinned\n\n[profiles.fast]\nmodel = \"gpt-5-mini\"\n");
    w(
        project.join(".mcp.json"),
        "{\n  \"mcpServers\": {\n    \"mine\": {\n      \"command\": \"echo\"\n    }\n  }\n}\n",
    );
    w(
        project.join(".claude").join("settings.json"),
        "{\n  \"permissions\": {\n    \"allow\": [\"Bash(ls:*)\"]\n  }\n}\n",
    );
    w(project.join("README.md"), "# demo\n");
    w(
        home.join(".claude").join("CLAUDE.md"),
        "# my global notes\n",
    );
}

#[test]
fn install_reinstall_update_uninstall_leaves_no_byte() {
    let root = TempRoot::new("accept");
    let home = root.0.join("home");
    let project = root.0.join("project");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&project).unwrap();
    seed(&project, &home);
    let env = Env::sandbox(&home, Os::current());
    let before = snapshot(&root.0);

    let comps = components();
    let req = PlanRequest {
        components: comps.clone(),
        targets: TARGETS.iter().map(|s| s.to_string()).collect(),
        scope: Some(Scope::Project),
        project_dir: Some(project.clone()),
        policy: ConflictPolicy::Rename,
        secret_values: BTreeMap::new(),
    };
    let p = plan::plan(&env, req.clone()).unwrap();
    for u in &p.units {
        assert!(
            u.error.is_none(),
            "{} → {}: {:?}",
            u.component.name,
            u.target,
            u.error
        );
    }
    let unit = |cid: &str, t: &str| {
        p.units
            .iter()
            .find(|u| u.component.id.ends_with(cid) && u.target == t)
            .unwrap()
    };
    // agent: Claude native, Cursor/Copilot read Claude's file, Codex/OpenCode converted (round 2)
    assert_eq!(unit("frontend-developer", "claude").status, UnitStatus::New);
    assert!(matches!(
        unit("frontend-developer", "cursor").compat,
        Compat::Native
    ));
    assert_eq!(unit("frontend-developer", "codex").status, UnitStatus::New);
    assert!(matches!(
        unit("frontend-developer", "opencode").compat,
        Compat::Converted | Compat::Degraded { .. }
    ));
    // MCP everywhere
    for t in TARGETS {
        assert_eq!(
            unit("github-official", t).status,
            UnitStatus::New,
            "mcp on {t}"
        );
    }
    assert!(p
        .files
        .iter()
        .all(|f| !f.diff.is_empty() || f.action == "unchanged"));
    assert!(
        p.files
            .iter()
            .any(|f| f.commands.iter().any(|c| c.contains("docker run"))),
        "MCP command shown"
    );

    let rep = writer::apply(&env, &p).unwrap();
    assert!(!rep.installed.is_empty());

    // --- what landed
    let read = |p: PathBuf| {
        std::fs::read_to_string(&p).unwrap_or_else(|_| panic!("missing {}", p.display()))
    };
    let agent = read(project.join(".claude/agents/frontend-developer.md"));
    assert!(agent.starts_with("---\nname: frontend-developer"));
    let codex = read(project.join(".codex/config.toml"));
    assert!(
        codex.starts_with("# my codex project config\nmodel = \"gpt-5\" # pinned\n"),
        "{codex}"
    );
    assert!(codex.contains("[mcp_servers.github-official]"), "{codex}");
    assert!(
        codex.contains("env_vars = [\"GITHUB_PERSONAL_ACCESS_TOKEN\"]"),
        "{codex}"
    );
    assert!(
        codex.contains("bearer_token_env_var = \"HF_TOKEN\""),
        "{codex}"
    );
    let v = edit::parse_value(DocFormat::Toml, &codex).unwrap();
    assert_eq!(v["profiles"]["fast"]["model"], "gpt-5-mini");
    let oc = read(project.join("opencode.json"));
    assert!(
        oc.contains("// my opencode config") && oc.contains("// keep this server"),
        "{oc}"
    );
    let ocv = edit::parse_value(DocFormat::Jsonc, &oc).unwrap();
    assert_eq!(ocv["mcp"]["github-official"]["command"][0], "docker");
    assert_eq!(
        ocv["mcp"]["github-official"]["environment"]["GITHUB_PERSONAL_ACCESS_TOKEN"],
        "{env:GITHUB_PERSONAL_ACCESS_TOKEN}"
    );
    assert_eq!(ocv["mcp"]["existing"]["type"], "local");
    let vs = edit::parse_value(DocFormat::Json, &read(project.join(".vscode/mcp.json"))).unwrap();
    assert_eq!(
        vs["servers"]["huggingface"]["headers"]["Authorization"],
        "Bearer ${input:hf-token}"
    );
    assert!(vs["inputs"]
        .as_array()
        .unwrap()
        .iter()
        .any(|i| i["id"] == "hf-token"));
    let cur = edit::parse_value(DocFormat::Json, &read(project.join(".cursor/mcp.json"))).unwrap();
    assert_eq!(
        cur["mcpServers"]["github-official"]["env"]["GITHUB_PERSONAL_ACCESS_TOKEN"],
        "${env:GITHUB_PERSONAL_ACCESS_TOKEN}"
    );
    let cl = edit::parse_value(DocFormat::Json, &read(project.join(".mcp.json"))).unwrap();
    assert_eq!(cl["mcpServers"]["mine"]["command"], "echo");
    assert_eq!(cl["mcpServers"]["huggingface"]["type"], "http");
    let settings = edit::parse_value(
        DocFormat::Json,
        &read(project.join(".claude/settings.json")),
    )
    .unwrap();
    assert_eq!(settings["permissions"]["allow"][0], "Bash(ls:*)");
    let post = settings["hooks"]["PostToolUse"].as_array().unwrap();
    assert!(post.iter().any(|g| g["matcher"] == "Edit"));
    let codex_hooks =
        edit::parse_value(DocFormat::Json, &read(project.join(".codex/hooks.json"))).unwrap();
    assert!(
        codex_hooks["hooks"]["PostToolUse"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .contains(".codex/hooks/change-logger.py")
    );
    assert!(project.join(".claude/hooks/change-logger.py").is_file());
    assert!(project.join(".codex/hooks/change-logger.py").is_file());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let m = std::fs::metadata(project.join(".claude/hooks/change-logger.py"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(m & 0o111, 0o111);
    }
    // canonical store in .agents/skills (Codex, Cursor, OpenCode, Copilot read it);
    // Claude does not, so it gets a copy
    for d in [".claude/skills", ".agents/skills"] {
        assert!(
            project
                .join(d)
                .join("design-to-code")
                .join("SKILL.md")
                .is_file(),
            "skill in {d}"
        );
    }
    for d in [".cursor/skills", ".opencode/skills", ".github/skills"] {
        assert!(!project.join(d).exists(), "no copy needed in {d}");
    }
    assert!(project.join(".omniget/agentkit.lock.json").is_file());
    let installed_snapshot = snapshot(&root.0);

    // --- reinstall: nothing changes, nothing duplicates
    let p2 = plan::plan(&env, req.clone()).unwrap();
    assert!(
        p2.units
            .iter()
            .filter(|u| u.status != UnitStatus::Unsupported)
            .all(|u| u.status == UnitStatus::Installed),
        "{:?}",
        p2.units
            .iter()
            .map(|u| (&u.component.name, &u.target, u.status))
            .collect::<Vec<_>>()
    );
    let r2 = writer::apply(&env, &p2).unwrap();
    assert!(r2.files_written.is_empty(), "{:?}", r2.files_written);
    assert_eq!(snapshot(&root.0), installed_snapshot);

    // --- update the hook: old pieces out, new in, still one entry per matcher
    let mut comps2 = comps.clone();
    for c in comps2.iter_mut() {
        if c.kind == ComponentKind::Hook {
            c.files[0].bytes.extend_from_slice(b" ");
            c.rehash();
        }
    }
    let p3 = plan::plan(
        &env,
        PlanRequest {
            components: comps2,
            ..req.clone()
        },
    )
    .unwrap();
    assert!(p3.units.iter().any(|u| u.status == UnitStatus::Update));
    writer::apply(&env, &p3).unwrap();
    let settings = edit::parse_value(
        DocFormat::Json,
        &read(project.join(".claude/settings.json")),
    )
    .unwrap();
    let edits = settings["hooks"]["PostToolUse"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|g| g["matcher"] == "Edit")
        .count();
    assert_eq!(edits, 1, "hook duplicated on update");

    // --- drift is quiet on our own files
    assert!(writer::drift(&env, Some(&project)).unwrap().is_empty());

    // --- uninstall everything
    let all = writer::installed(&env, Some(&project));
    assert!(!all.is_empty());
    for r in all {
        let u = writer::uninstall(&env, &r.install_id, Some(&project), false).unwrap();
        assert!(u.drift.is_empty(), "{:?}", u.drift);
    }
    let after = snapshot(&root.0);
    for (path, bytes) in &before.0 {
        let now = after
            .0
            .get(path)
            .unwrap_or_else(|| panic!("{path} disappeared"));
        assert_eq!(
            String::from_utf8_lossy(now),
            String::from_utf8_lossy(bytes),
            "{path} changed"
        );
    }
    let extra: Vec<&String> = after
        .0
        .keys()
        .filter(|k| !before.0.contains_key(*k))
        .collect();
    assert!(extra.is_empty(), "left behind: {extra:?}");
    let extra_dirs: Vec<&String> = after.1.iter().filter(|d| !before.1.contains(d)).collect();
    assert!(extra_dirs.is_empty(), "folders left behind: {extra_dirs:?}");
}

#[test]
fn global_scope_uses_absolute_paths_and_restore_rolls_back() {
    let root = TempRoot::new("global");
    let home = root.0.join("home");
    std::fs::create_dir_all(home.join(".codex")).unwrap();
    std::fs::write(home.join(".codex/config.toml"), "# keep\n").unwrap();
    let env = Env::sandbox(&home, Os::Linux);
    let before = snapshot(&root.0);
    let comps: Vec<Component> = components()
        .into_iter()
        .filter(|c| matches!(c.kind, ComponentKind::Hook | ComponentKind::Mcp))
        .collect();
    let p = plan::plan(
        &env,
        PlanRequest {
            components: comps,
            targets: vec!["claude".into(), "codex".into()],
            scope: Some(Scope::Global),
            project_dir: None,
            policy: ConflictPolicy::Rename,
            secret_values: BTreeMap::new(),
        },
    )
    .unwrap();
    let rep = writer::apply(&env, &p).unwrap();
    let settings = std::fs::read_to_string(home.join(".claude/settings.json")).unwrap();
    let abs = home
        .join(".claude")
        .join("hooks")
        .join("change-logger.py")
        .display()
        .to_string();
    assert!(
        settings.contains(&abs),
        "global hook command must be absolute: {settings}"
    );
    let claude_json = edit::parse_value(
        DocFormat::Json,
        &std::fs::read_to_string(home.join(".claude.json")).unwrap(),
    )
    .unwrap();
    assert!(claude_json["mcpServers"]["github-official"].is_object());
    // user edits our agent-less file → drift
    let hook_script = home.join(".claude/hooks/change-logger.py");
    std::fs::write(&hook_script, "edited\n").unwrap();
    let d = writer::drift(&env, None).unwrap();
    assert!(d.iter().any(|x| x.state == "modified"));
    // roll the install back
    std::fs::write(
        &hook_script,
        std::fs::read(home.join(".claude/hooks/change-logger.py")).unwrap(),
    )
    .unwrap();
    writer::restore(&env, &rep.tx).unwrap();
    let after = snapshot(&root.0);
    assert_eq!(after.0, before.0);
}

// ------------------------------------------------------------------ round 2 (k1-convert)

/// The cct agent `development-team/frontend-developer` and command `git/feature`
/// installed into seven tools' own formats in a fake home, reinstalled without
/// duplicating, and uninstalled to the byte.
#[test]
fn agent_and_command_in_seven_tools_round_trip_to_the_byte() {
    let cct = std::env::var("AGENTKIT_CCT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_CCT));
    let comps: Vec<Component> = if cct.join("commands/git/feature.md").is_file() {
        vec![
            load(
                &cct,
                ComponentKind::Agent,
                "agents/development-team/frontend-developer.md",
                "cct:agents/development-team/frontend-developer",
                "development-team",
            ),
            load(
                &cct,
                ComponentKind::Command,
                "commands/git/feature.md",
                "cct:commands/git/feature",
                "git",
            ),
        ]
    } else {
        let f = |p: &str, t: &str| -> parse::RawFiles {
            [(p.to_string(), t.as_bytes().to_vec())]
                .into_iter()
                .collect()
        };
        let mut a = parse::parse_raw(ComponentKind::Agent, "frontend-developer.md", &f("frontend-developer.md", "---\nname: frontend-developer\ndescription: Builds UIs\ntools: Read, Write, Edit, Bash, Glob, Grep\n---\n\nYou are a senior frontend developer.\n")).unwrap();
        a.id = "cct:agents/development-team/frontend-developer".into();
        let mut c = parse::parse_raw(ComponentKind::Command, "feature.md", &f("feature.md", "---\nallowed-tools: Bash(git:*)\nargument-hint: <feature-name>\ndescription: Create a new Git Flow feature branch\n---\n\nCreate new feature branch: **$ARGUMENTS**\n- Current branch: !`git branch --show-current`\n")).unwrap();
        c.id = "cct:commands/git/feature".into();
        vec![a, c]
    };
    let root = TempRoot::new("k1");
    let home = root.0.join("home");
    let project = root.0.join("project");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("README.md"), "# demo\n").unwrap();
    let env = Env::sandbox(&home, Os::current());
    let before = snapshot(&root.0);
    let targets = [
        "codex", "gemini", "opencode", "cursor", "copilot", "kiro", "goose",
    ];
    let req = PlanRequest {
        components: comps.clone(),
        targets: targets.iter().map(|s| s.to_string()).collect(),
        scope: Some(Scope::Project),
        project_dir: Some(project.clone()),
        policy: ConflictPolicy::Rename,
        secret_values: BTreeMap::new(),
    };
    let p = plan::plan(&env, req.clone()).unwrap();
    for u in &p.units {
        assert!(
            matches!(u.status, UnitStatus::New),
            "{} → {}: {:?} {:?} {:?}",
            u.component.name,
            u.target,
            u.status,
            u.error,
            u.notes
        );
        assert!(
            !matches!(u.compat, Compat::Unsupported { .. }),
            "{} → {}",
            u.component.name,
            u.target
        );
    }
    writer::apply(&env, &p).unwrap();

    let read = |rel: &str| {
        std::fs::read_to_string(project.join(rel)).unwrap_or_else(|_| panic!("missing {rel}"))
    };
    // Codex: TOML agent + command as a user-invoked skill
    let codex = edit::parse_value(
        DocFormat::Toml,
        &read(".codex/agents/frontend-developer.toml"),
    )
    .unwrap();
    assert_eq!(codex["name"], "frontend-developer");
    assert!(codex["developer_instructions"]
        .as_str()
        .unwrap()
        .contains("senior frontend developer"));
    let skill = read(".agents/skills/feature/SKILL.md");
    assert!(skill.contains("disable-model-invocation: true"), "{skill}");
    assert!(skill.contains("$ARGUMENTS"), "{skill}");
    assert!(project
        .join(".agents/skills/feature/agents/openai.yaml")
        .is_file());
    // Gemini: Markdown agent with Gemini tool names + TOML command
    let gem = read(".gemini/agents/frontend-developer.md");
    assert!(
        gem.contains("kind: local")
            && gem.contains("read_file")
            && gem.contains("run_shell_command"),
        "{gem}"
    );
    let gcmd = edit::parse_value(DocFormat::Toml, &read(".gemini/commands/feature.toml")).unwrap();
    let prompt = gcmd["prompt"].as_str().unwrap();
    assert!(
        prompt.contains("**{{args}}**") && prompt.contains("!{git branch --show-current}"),
        "{prompt}"
    );
    // OpenCode: subagent + 1:1 command
    let oc = read(".opencode/agents/frontend-developer.md");
    assert!(oc.contains("mode: subagent"), "{oc}");
    let occ = read(".opencode/commands/feature.md");
    assert!(
        occ.contains("**$ARGUMENTS**") && occ.contains("!`git branch --show-current`"),
        "{occ}"
    );
    // Cursor reads the Claude agent; its command is plain text
    assert!(
        read(".claude/agents/frontend-developer.md").starts_with("---\nname: frontend-developer")
    );
    let cur = read(".cursor/commands/feature.md");
    assert!(
        !cur.starts_with("---") && !cur.contains("$ARGUMENTS"),
        "{cur}"
    );
    // Copilot reads .claude/agents and .claude/commands (CLI)
    assert!(project.join(".claude/commands/feature.md").is_file());
    // Kiro: JSON agent + prompt
    let kiro = edit::parse_value(
        DocFormat::Json,
        &read(".kiro/agents/frontend-developer.json"),
    )
    .unwrap();
    assert_eq!(kiro["tools"], serde_json::json!(["read", "write", "shell"]));
    assert!(project.join(".kiro/prompts/feature.md").is_file());
    // Goose: agent in .agents/agents (Claude format) + recipe
    assert!(project
        .join(".agents/agents/frontend-developer.md")
        .is_file());
    let recipe = edit::parse_value(DocFormat::Yaml, &read(".goose/recipes/feature.yaml")).unwrap();
    assert_eq!(recipe["title"], "feature");
    assert!(recipe["prompt"].as_str().unwrap().contains("{{ args }}"));
    assert_eq!(recipe["parameters"][0]["key"], "args");

    // import reads our files back as canonical components
    for (t, n) in [
        ("codex", 2),
        ("gemini", 2),
        ("opencode", 2),
        ("kiro", 2),
        ("goose", 2),
    ] {
        let got =
            super::convert::import_installed(&env, t, Scope::Project, Some(&project)).unwrap();
        let names: Vec<(ComponentKind, String)> =
            got.iter().map(|c| (c.kind, c.name.clone())).collect();
        assert!(
            names.contains(&(ComponentKind::Agent, "frontend-developer".into()))
                && names.contains(&(ComponentKind::Command, "feature".into())),
            "{t}: {names:?}"
        );
        assert!(got.len() >= n, "{t}: {names:?}");
    }
    let installed_snapshot = snapshot(&root.0);

    // reinstall: everything already installed, nothing written
    let p2 = plan::plan(&env, req.clone()).unwrap();
    assert!(p2.units.iter().all(|u| u.status == UnitStatus::Installed));
    let r2 = writer::apply(&env, &p2).unwrap();
    assert!(r2.files_written.is_empty(), "{:?}", r2.files_written);
    assert_eq!(snapshot(&root.0), installed_snapshot);

    // uninstall: back to the byte, no file or folder left
    for r in writer::installed(&env, Some(&project)) {
        let u = writer::uninstall(&env, &r.install_id, Some(&project), false).unwrap();
        assert!(u.drift.is_empty(), "{:?}", u.drift);
    }
    let after = snapshot(&root.0);
    assert_eq!(after.0, before.0, "files differ after uninstall");
    let extra_dirs: Vec<&String> = after.1.iter().filter(|d| !before.1.contains(d)).collect();
    assert!(extra_dirs.is_empty(), "folders left behind: {extra_dirs:?}");
}

// ------------------------------------------------------------------ round 3 (r3-fix-agentkit)

use super::convert::{Dedupe, FileAction, PatchOp, PlannedFile};
use super::edit::Seg;
use super::plan::{FilePlan, InstallPlan, PlanUnit};

fn raw(pairs: &[(&str, &str)]) -> parse::RawFiles {
    pairs
        .iter()
        .map(|(p, t)| (p.to_string(), t.as_bytes().to_vec()))
        .collect()
}

fn dummy(id: &str) -> Component {
    let mut c = parse::parse_raw(
        ComponentKind::Agent,
        "x.md",
        &raw(&[(
            "x.md",
            &format!("---\nname: {id}\ndescription: d\n---\nBody {id}\n"),
        )]),
    )
    .unwrap();
    c.id = format!("test:agents/{id}");
    c
}

/// A plan made by hand (the writer only replays what it is given).
fn hand_plan(project: &Path, units: &[(&Component, Vec<PlannedFile>)]) -> InstallPlan {
    let mut paths: Vec<PathBuf> = Vec::new();
    for (_, fs) in units {
        for f in fs {
            if !paths.contains(&f.path) {
                paths.push(f.path.clone());
            }
        }
    }
    InstallPlan {
        id: "hand".into(),
        created_at: String::new(),
        scope: Scope::Project,
        project_dir: Some(project.to_path_buf()),
        policy: ConflictPolicy::Rename,
        units: units
            .iter()
            .map(|(c, fs)| PlanUnit {
                unit_id: format!("{}@t", c.id),
                component: (*c).into(),
                target: "claude".into(),
                target_name: "Claude".into(),
                status: UnitStatus::New,
                compat: Compat::Native,
                install_name: c.name.clone(),
                replaces: None,
                files: fs.clone(),
                losses: vec![],
                notes: vec![],
                commands: vec![],
                error: None,
            })
            .collect(),
        files: paths
            .into_iter()
            .map(|p| FilePlan {
                before_sha: std::fs::read(&p).ok().map(|b| super::sha256_hex(&b)),
                path: p,
                action: "merge".into(),
                after_sha: None,
                diff: String::new(),
                targets: vec![],
                components: vec![],
                conflicts: vec![],
                commands: vec![],
                executable: false,
                owned_by_update: false,
                link_to: None,
            })
            .collect(),
        warnings: vec![],
    }
}

fn pieces(project: &Path, c: &Component, event: &str, cmd: &str, block: &str) -> Vec<PlannedFile> {
    let k = |s: &str| Seg::Key(s.to_string());
    vec![
        PlannedFile::merge(
            "claude",
            c,
            project.join(".cursor").join("hooks.json"),
            DocFormat::Json,
            vec![
                PatchOp::Set {
                    path: vec![k("version")],
                    value: serde_json::json!(1),
                    rename_at: None,
                },
                PatchOp::Append {
                    path: vec![k("hooks"), k(event)],
                    value: serde_json::json!({ "command": cmd }),
                    dedupe: Dedupe::FlatHook,
                },
            ],
            "hook",
        ),
        PlannedFile::merge(
            "claude",
            c,
            project.join("AGENTS.md"),
            DocFormat::Markdown,
            vec![PatchOp::TextBlock {
                id: block.to_string(),
                content: format!("## {block}\n\nText of {block}.\n"),
            }],
            "rule",
        ),
    ]
}

fn permutations(n: usize) -> Vec<Vec<usize>> {
    if n == 1 {
        return vec![vec![0]];
    }
    let mut out = Vec::new();
    for p in permutations(n - 1) {
        for i in 0..=p.len() {
            let mut q = p.clone();
            q.insert(i, n - 1);
            out.push(q);
        }
    }
    out
}

/// Three installs share a new hooks file (`version: 1` set by all, two events)
/// and an AGENTS.md block (two of them); uninstalling in every order restores
/// the tree byte for byte, and a shared piece stays until its last holder goes.
#[test]
fn shared_pieces_leave_with_their_last_holder_in_any_order() {
    for together in [false, true] {
        for order in permutations(3) {
            let root = TempRoot::new("r3-order");
            let home = root.0.join("home");
            let project = root.0.join("project");
            std::fs::create_dir_all(&home).unwrap();
            std::fs::create_dir_all(&project).unwrap();
            std::fs::write(project.join("AGENTS.md"), "# mine\n").unwrap();
            let env = Env::sandbox(&home, Os::current());
            let before = snapshot(&root.0);
            let (a, b, c) = (dummy("a"), dummy("b"), dummy("c"));
            let units = vec![
                (&a, pieces(&project, &a, "postToolUse", "a", "shared")),
                (&b, pieces(&project, &b, "preToolUse", "b", "shared")),
                (&c, pieces(&project, &c, "postToolUse", "c", "c-own")),
            ];
            if together {
                writer::apply(&env, &hand_plan(&project, &units)).unwrap();
            } else {
                for u in &units {
                    writer::apply(&env, &hand_plan(&project, std::slice::from_ref(u))).unwrap();
                }
            }
            let recs = writer::installed(&env, Some(&project));
            assert_eq!(recs.len(), 3, "together={together}");
            assert!(writer::drift(&env, Some(&project)).unwrap().is_empty());
            for (step, i) in order.iter().enumerate() {
                let id = &recs
                    .iter()
                    .find(|r| r.component.id == units[*i].0.id)
                    .unwrap()
                    .install_id;
                let u = writer::uninstall(&env, id, Some(&project), false).unwrap();
                assert!(u.drift.is_empty(), "{order:?}: {:?}", u.drift);
                let left: Vec<usize> = order[step + 1..].to_vec();
                let hooks = std::fs::read_to_string(project.join(".cursor/hooks.json")).ok();
                let agents = std::fs::read_to_string(project.join("AGENTS.md")).unwrap();
                if left.is_empty() {
                    assert!(hooks.is_none(), "{order:?} together={together}: {hooks:?}");
                } else {
                    let h = hooks.unwrap_or_else(|| panic!("{order:?}: hooks.json gone early"));
                    assert!(h.contains("\"version\": 1"), "{order:?} step {step}: {h}");
                }
                let shared_left = left.iter().any(|x| *x != 2);
                assert_eq!(
                    agents.contains("Text of shared"),
                    shared_left,
                    "{order:?} step {step}: {agents}"
                );
            }
            let after = snapshot(&root.0);
            assert_eq!(after.0, before.0, "{order:?} together={together}");
            let extra: Vec<&String> = after.1.iter().filter(|d| !before.1.contains(d)).collect();
            assert!(extra.is_empty(), "{order:?}: {extra:?}");
        }
    }
}

fn skill() -> Component {
    let mut s = parse::parse_raw(
        ComponentKind::Skill,
        "tdd/SKILL.md",
        &raw(&[
            (
                "tdd/SKILL.md",
                "---\nname: tdd\ndescription: Test first\n---\nBody\n",
            ),
            ("tdd/scripts/run.sh", "#!/bin/sh\necho hi\n"),
        ]),
    )
    .unwrap();
    s.id = "test:skills/tdd".into();
    s
}

/// Claude does not read `.agents/skills`: its skill folder is a link to the
/// store (a copy where links are not allowed); both reinstall to nothing and
/// uninstall to the byte.
#[test]
fn skill_folder_is_a_link_or_a_copy_and_leaves_clean() {
    for copy in [false, true] {
        writer::set_force_copy_links(copy);
        let root = TempRoot::new("r3-link");
        let home = root.0.join("home");
        let project = root.0.join("project");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&project).unwrap();
        let env = Env::sandbox(&home, Os::current());
        let before = snapshot(&root.0);
        let req = PlanRequest {
            components: vec![skill()],
            targets: vec!["claude".into(), "kiro".into()],
            scope: Some(Scope::Project),
            project_dir: Some(project.clone()),
            policy: ConflictPolicy::Rename,
            secret_values: BTreeMap::new(),
        };
        let p = plan::plan(&env, req.clone()).unwrap();
        let link = p
            .units
            .iter()
            .flat_map(|u| u.files.iter())
            .find(|f| f.action == FileAction::Link && f.target == "claude")
            .expect("claude gets a link");
        assert_eq!(link.path, project.join(".claude/skills/tdd"));
        writer::apply(&env, &p).unwrap();
        let dir = project.join(".claude/skills/tdd");
        assert!(dir.join("SKILL.md").is_file());
        assert!(project.join(".kiro/skills/tdd/scripts/run.sh").is_file());
        let is_link = std::fs::symlink_metadata(&dir)
            .unwrap()
            .file_type()
            .is_symlink();
        assert_eq!(is_link, !copy, "copy={copy}");
        let lock = std::fs::read_to_string(project.join(".omniget/agentkit.lock.json")).unwrap();
        assert!(lock.contains("\"type\": \"linked\""), "{lock}");
        assert!(writer::drift(&env, Some(&project)).unwrap().is_empty());
        let snap = snapshot(&root.0);
        let p2 = plan::plan(&env, req.clone()).unwrap();
        assert!(p2.units.iter().all(|u| u.status == UnitStatus::Installed));
        let p3 = plan::plan(
            &env,
            PlanRequest {
                components: vec![{
                    let mut s = skill();
                    s.id = "test:skills/other-id".into();
                    s
                }],
                targets: vec!["claude".into()],
                ..req.clone()
            },
        )
        .unwrap();
        // same folder, same bytes under another id: not a collision
        assert!(
            p3.units.iter().all(|u| u.status == UnitStatus::New),
            "{:?}",
            p3.units
                .iter()
                .map(|u| (&u.status, &u.notes))
                .collect::<Vec<_>>()
        );
        assert_eq!(snapshot(&root.0), snap);
        for r in writer::installed(&env, Some(&project)) {
            let u = writer::uninstall(&env, &r.install_id, Some(&project), false).unwrap();
            assert!(u.drift.is_empty(), "{:?}", u.drift);
        }
        let after = snapshot(&root.0);
        assert_eq!(after.0, before.0, "copy={copy}");
        let extra: Vec<&String> = after.1.iter().filter(|d| !before.1.contains(d)).collect();
        assert!(extra.is_empty(), "copy={copy}: {extra:?}");
        writer::set_force_copy_links(false);
    }
}

/// Root rules go to each tool's own file (AGENTS.md for OpenCode/Cursor…, never
/// CLAUDE.md); one AGENTS.md block serves Codex, OpenCode and Kiro and leaves
/// with the last of them.
#[test]
fn rules_use_native_files_and_share_one_agents_md_block() {
    let root = TempRoot::new("r3-rules");
    let home = root.0.join("home");
    let project = root.0.join("project");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("AGENTS.md"), "# team rules\n").unwrap();
    let env = Env::sandbox(&home, Os::current());
    let before = snapshot(&root.0);
    let mut r = parse::parse_raw(
        ComponentKind::Rule,
        "no-console.md",
        &raw(&[(
            "no-console.md",
            "---\ndescription: No console.log\n---\nNever leave console.log in code.\n",
        )]),
    )
    .unwrap();
    r.id = "test:rules/no-console".into();
    let targets = ["codex", "opencode", "kiro", "cursor", "copilot", "zed"];
    let p = plan::plan(
        &env,
        PlanRequest {
            components: vec![r.clone()],
            targets: targets.iter().map(|s| s.to_string()).collect(),
            scope: Some(Scope::Project),
            project_dir: Some(project.clone()),
            policy: ConflictPolicy::Rename,
            secret_values: BTreeMap::new(),
        },
    )
    .unwrap();
    for u in &p.units {
        assert!(
            matches!(u.status, UnitStatus::New),
            "{}: {:?} {:?}",
            u.target,
            u.status,
            u.notes
        );
        for f in &u.files {
            let s = f.path.display().to_string();
            assert!(
                !s.ends_with("CLAUDE.md") && !s.contains(".claude"),
                "{}: {s}",
                u.target
            );
        }
    }
    writer::apply(&env, &p).unwrap();
    let agents = std::fs::read_to_string(project.join("AGENTS.md")).unwrap();
    assert_eq!(
        agents.matches("Never leave console.log").count(),
        1,
        "{agents}"
    );
    let recs = writer::installed(&env, Some(&project));
    assert_eq!(recs.len(), targets.len());
    // Codex leaves first: the block stays for the others
    let codex = recs.iter().find(|r| r.target == "codex").unwrap();
    writer::uninstall(&env, &codex.install_id, Some(&project), false).unwrap();
    assert!(std::fs::read_to_string(project.join("AGENTS.md"))
        .unwrap()
        .contains("Never leave console.log"));
    for r in recs.iter().filter(|r| r.target != "codex") {
        writer::uninstall(&env, &r.install_id, Some(&project), false).unwrap();
    }
    let after = snapshot(&root.0);
    assert_eq!(after.0, before.0);
    let extra: Vec<&String> = after.1.iter().filter(|d| !before.1.contains(d)).collect();
    assert!(extra.is_empty(), "{extra:?}");
}

/// A Copilot chatmode goes to Copilot as a native `.github/agents/*.agent.md`,
/// to Claude with Claude tool names; Zed and Codebuff simulate agents.
#[test]
fn copilot_agents_native_and_simulated_agents_on_zed_and_codebuff() {
    let root = TempRoot::new("r3-agents");
    let home = root.0.join("home");
    let project = root.0.join("project");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&project).unwrap();
    let env = Env::sandbox(&home, Os::current());
    let text = "---\ndescription: 'Plan things'\ntools: ['codebase', 'editFiles', 'runCommands']\n---\nPlan.\n";
    let mut c = parse::parse_raw(
        ComponentKind::Agent,
        "planner.chatmode.md",
        &raw(&[("planner.chatmode.md", text)]),
    )
    .unwrap();
    c.id = "test:agents/planner".into();
    assert_eq!(c.origin_tool, "copilot");
    let p = plan::plan(
        &env,
        PlanRequest {
            components: vec![c],
            targets: vec![
                "copilot".into(),
                "claude".into(),
                "zed".into(),
                "codebuff".into(),
            ],
            scope: Some(Scope::Project),
            project_dir: Some(project.clone()),
            policy: ConflictPolicy::Rename,
            secret_values: BTreeMap::new(),
        },
    )
    .unwrap();
    let unit = |t: &str| p.units.iter().find(|u| u.target == t).unwrap();
    let cp = unit("copilot");
    assert!(matches!(cp.compat, Compat::Native), "{:?}", cp.compat);
    assert_eq!(
        cp.files[0].path,
        project.join(".github/agents/planner.agent.md")
    );
    assert_eq!(cp.files[0].content.as_deref(), Some(text.as_bytes()));
    let cl = unit("claude");
    let body = String::from_utf8(cl.files[0].content.clone().unwrap()).unwrap();
    assert!(
        body.contains("Edit") && body.contains("Bash") && !body.contains("editFiles"),
        "{body}"
    );
    for t in ["zed", "codebuff"] {
        let u = unit(t);
        assert_eq!(u.status, UnitStatus::New, "{t}: {:?}", u.notes);
        assert!(
            u.files
                .iter()
                .any(|f| f.path == project.join(".agents/agents/planner.md")),
            "{t}"
        );
        assert!(
            u.files.iter().any(|f| f.path == project.join("AGENTS.md")),
            "{t}"
        );
    }
}
