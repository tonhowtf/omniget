use serde_json::json;

use super::frontmatter::{parse_yaml, split};
use super::model::{content_hash, CatalogItem, ItemFile, ItemSource};
use super::normalize::*;
use super::search::{facets, search, Filters, Page, Sort};

#[test]
fn yaml_subset() {
    let y = "name: a-b\ndescription: \"Use it\\nwhen\"\ntools: Read, Write\ntags: [x, 'y z']\nmetadata:\n  author: me\n  list:\n    - one\n    - k: v\n      n: 2\nlong: >-\n  folded\n  text\nblock: |\n  l1\n  l2\nplain: first\n  second\nempty:\nnum: 12\n";
    let v = parse_yaml(y);
    assert_eq!(v["name"], "a-b");
    assert_eq!(v["description"], "Use it\nwhen");
    assert_eq!(v["tools"], "Read, Write");
    assert_eq!(v["tags"], json!(["x", "y z"]));
    assert_eq!(v["metadata"]["author"], "me");
    assert_eq!(v["metadata"]["list"], json!(["one", {"k": "v", "n": 2}]));
    assert_eq!(v["long"], "folded text");
    assert_eq!(v["block"], "l1\nl2\n");
    assert_eq!(v["plain"], "first second");
    assert!(v["empty"].is_null());
    assert_eq!(v["num"], 12);
}

#[test]
fn yaml_split_and_garbage() {
    let (y, body) = split("---\nname: x\n---\n# Body\n").unwrap();
    assert_eq!(y, "name: x\n");
    assert_eq!(body, "# Body\n");
    assert!(split("# no fm\n").is_none());
    // lixo não panica
    let v = parse_yaml(": : :\n- [unclosed\n  \"x");
    assert!(v.is_object() || v.is_array());
}

#[test]
fn description_strips_examples_and_truncates() {
    let raw = "Use when building UIs. Specifically:\\n\\n<example>\\nContext: x\\n</example>\\n<example>y</example>";
    let (s, marks) = short_description(raw);
    assert_eq!(s, "Use when building UIs.");
    assert!(marks.contains(&"stripped_example_blocks"));
    let long = "word ".repeat(200);
    let (s, marks) = short_description(&long);
    assert!(s.chars().count() <= DESC_MAX);
    assert!(s.ends_with('…'));
    assert!(marks.contains(&"truncated_description"));
}

#[test]
fn tools_and_models() {
    assert_eq!(
        tools_origin(&json!("Read, Write, Bash"), None),
        ToolsOrigin::Claude
    );
    assert_eq!(
        tools_origin(&json!("codebase, edit/editFiles, runCommands"), None),
        ToolsOrigin::Copilot
    );
    assert_eq!(
        tools_origin(&json!(["read", "edit", "search", "github/*"]), None),
        ToolsOrigin::Copilot
    );
    assert_eq!(
        tools_origin(&json!("Read, Edit, azure_design_architecture"), None),
        ToolsOrigin::Mixed
    );
    assert_eq!(
        tools_origin(&json!("Read"), Some("Claude Sonnet 4.5 (copilot)")),
        ToolsOrigin::Copilot
    );
    assert!(valid_claude_model("sonnet"));
    assert!(valid_claude_model("claude-sonnet-4-5-20250929"));
    assert!(!valid_claude_model("claude-sonnet-4-5"));
    assert!(!valid_claude_model("Claude Sonnet 4"));
}

#[test]
fn licenses() {
    assert_eq!(normalize_license(&json!("MIT")).as_deref(), Some("MIT"));
    assert_eq!(
        normalize_license(&json!("Apache 2.0")).as_deref(),
        Some("Apache-2.0")
    );
    assert_eq!(
        normalize_license(&json!("Complete terms in LICENSE.txt")),
        None
    );
    assert_eq!(
        normalize_license(&json!("Proprietary. LICENSE.txt has complete terms")).as_deref(),
        Some("Proprietary")
    );
    assert_eq!(
        detect_license_text("Apache License\n                           Version 2.0, January 2004"),
        Some("Apache-2.0")
    );
    assert_eq!(
        detect_license_text("MIT License\n\nPermission is hereby granted, free of charge"),
        Some("MIT")
    );
    assert_eq!(
        skill_exclusion("docx", None),
        Some("anthropic_proprietary_document_skill")
    );
    assert_eq!(
        skill_exclusion("pdf-official", Some("MIT")),
        Some("anthropic_official_duplicate")
    );
    assert_eq!(
        skill_exclusion("pdf-processing", Some("Proprietary")),
        Some("proprietary_license")
    );
    assert_eq!(skill_exclusion("frontend-design", Some("Apache-2.0")), None);
}

fn item(
    id: &str,
    kind: &str,
    name: &str,
    cat: &str,
    desc: &str,
    stars: Option<u64>,
    updated: &str,
) -> CatalogItem {
    CatalogItem {
        id: id.into(),
        kind: kind.into(),
        name: name.into(),
        category: cat.into(),
        description: desc.into(),
        source: ItemSource {
            id: id.split(':').next().unwrap().into(),
            ..Default::default()
        },
        license: Some("MIT".into()),
        author: None,
        tags: vec![],
        origin_tool: "claude".into(),
        files: vec![ItemFile {
            path: "a.md".into(),
            sha256: "0".repeat(64),
            size: 10,
        }],
        entry: "a.md".into(),
        frontmatter: json!({}),
        references: vec![],
        stars,
        updated: Some(updated.into()),
        normalization: vec![],
        security: None,
        collides_with: vec![],
        install_name: None,
    }
}

#[test]
fn search_filters_facets_sort() {
    let items = vec![
        item(
            "cct:agents/dev/frontend-developer",
            "agent",
            "frontend-developer",
            "dev",
            "React UIs",
            None,
            "2026-01-01",
        ),
        item(
            "cct:skills/web/frontend-design",
            "skill",
            "frontend-design",
            "web",
            "Design",
            Some(5),
            "2026-03-01",
        ),
        item(
            "x/y:plugins/general/ui",
            "plugin",
            "ui",
            "general",
            "frontend kit",
            Some(100),
            "2026-02-01",
        ),
        item(
            "cct:mcps/db/postgres",
            "mcp",
            "postgres",
            "db",
            "SQL",
            None,
            "2026-01-01",
        ),
    ];
    let r = search(
        &items,
        "frontend",
        &Filters::default(),
        Sort::Relevance,
        Page::default(),
    );
    assert_eq!(r.total, 3);
    let kinds: std::collections::BTreeSet<_> = r.items.iter().map(|h| h.kind.as_str()).collect();
    assert_eq!(kinds.len(), 3);
    // nome casa mais que descrição
    assert_ne!(r.items[0].kind, "plugin");
    let r = search(
        &items,
        "frontend",
        &Filters::default(),
        Sort::Stars,
        Page::default(),
    );
    assert_eq!(r.items[0].kind, "plugin");
    let r = search(
        &items,
        "",
        &Filters::default(),
        Sort::Updated,
        Page::default(),
    );
    assert_eq!(r.items[0].name, "frontend-design");
    let f = Filters {
        kinds: vec!["skill".into(), "agent".into()],
        ..Default::default()
    };
    let r = search(
        &items,
        "frontend",
        &f,
        Sort::Name,
        Page {
            offset: 0,
            limit: 1,
        },
    );
    assert_eq!((r.total, r.items.len()), (2, 1));
    let fc = facets(&items, "frontend", &f);
    assert_eq!(fc.total, 2);
    // a faceta de tipo ignora o próprio filtro
    assert_eq!(fc.kind.iter().map(|v| v.count).sum::<usize>(), 3);
    assert_eq!(
        fc.source.iter().find(|v| v.value == "cct").unwrap().count,
        2
    );
    assert!(content_hash(&items[0]).is_some());
}

#[test]
fn registry_conversion() {
    let entry = json!({
        "server": {
            "name": "io.github.acme/weather",
            "description": "Weather tools",
            "version": "1.2.0",
            "repository": {"url": "https://github.com/acme/weather", "source": "github"},
            "packages": [{
                "registryType": "npm", "identifier": "@acme/weather", "version": "1.2.0",
                "transport": {"type": "stdio"},
                "packageArguments": [{"type": "named", "name": "--units", "value": "metric"}],
                "environmentVariables": [{"name": "API_KEY", "isSecret": true, "isRequired": true}]
            }],
            "remotes": [{"type": "sse", "url": "https://w.example/sse",
                "headers": [{"name": "Authorization", "value": "Bearer {token}", "isSecret": true}]}]
        },
        "_meta": {"io.modelcontextprotocol.registry/official": {"status": "active", "updatedAt": "2026-05-01T00:00:00Z"}}
    });
    let it = super::registry::convert(&entry).unwrap();
    assert_eq!(it.id, "mcp-registry:mcps/io.github.acme/weather");
    assert_eq!(it.kind, "mcp");
    assert_eq!(it.name, "weather");
    assert_eq!(it.source.repo.as_deref(), Some("acme/weather"));
    assert_eq!(it.updated.as_deref(), Some("2026-05-01"));
    let variants = it.frontmatter["variants"].as_array().unwrap();
    assert_eq!(variants.len(), 2);
    let remote = &it.frontmatter["mcpServers"]["weather"];
    assert_eq!(remote["type"], "sse");
    assert_eq!(remote["headers"]["Authorization"], "Bearer {token}");
    let stdio = &variants[1]["config"];
    assert_eq!(stdio["command"], "npx");
    assert_eq!(
        stdio["args"],
        json!(["-y", "@acme/weather@1.2.0", "--units", "metric"])
    );
    assert_eq!(stdio["env"]["API_KEY"], "${API_KEY}");
    let pypi = json!({"server": {"name": "x/y", "description": "d", "version": "1",
        "packages": [{"registryType": "pypi", "identifier": "pkg", "version": "0.1", "runtimeHint": "uvx",
            "transport": {"type": "stdio"}, "packageArguments": [{"type": "positional", "value": "serve"}]}]}});
    let it = super::registry::convert(&pypi).unwrap();
    assert_eq!(
        it.frontmatter["mcpServers"]["y"]["args"],
        json!(["pkg==0.1", "serve"])
    );
}

#[test]
fn marketplace_sources() {
    use super::marketplace::plugin_source;
    let s = plugin_source(&json!("./plugins/foo"), "o/r", Some("abc"), None);
    assert_eq!(
        (s.repo.as_deref(), s.commit.as_deref(), s.path.as_str()),
        (Some("o/r"), Some("abc"), "plugins/foo")
    );
    let s = plugin_source(&json!("foo"), "o/r", None, Some("./plugins"));
    assert_eq!(s.path, "plugins/foo");
    let s = plugin_source(
        &json!({"source": "github", "repo": "a/b", "sha": "123"}),
        "o/r",
        None,
        None,
    );
    assert_eq!(
        (s.repo.as_deref(), s.commit.as_deref()),
        (Some("a/b"), Some("123"))
    );
    let s = plugin_source(
        &json!({"source": "url", "url": "https://github.com/a/b.git"}),
        "o/r",
        None,
        None,
    );
    assert_eq!(s.repo.as_deref(), Some("a/b"));
    let (name, items) = super::marketplace::plugins_from_manifest(
        "o/r",
        Some("abc"),
        Some(
            &json!({"name": "mk", "plugins": [{"name": "p1", "source": "./p1", "category": "Dev Tools", "description": "x"}]}),
        ),
        None,
        Some(3),
        Some("MIT".into()),
        None,
        None,
    );
    assert_eq!(name, "mk");
    assert_eq!(items[0].id, "o/r:plugins/dev-tools/p1");
    assert_eq!(items[0].source.dir, "p1");
    assert_eq!(items[0].stars, Some(3));
}

#[test]
fn source_specs() {
    use super::sources::{parse_spec, Spec};
    assert_eq!(
        parse_spec("owner/repo/skills/x@v1", None).unwrap(),
        Spec::Git {
            repo: "owner/repo".into(),
            path: Some("skills/x".into()),
            git_ref: Some("v1".into())
        }
    );
    assert_eq!(
        parse_spec("https://github.com/a/b/tree/main/sub/dir", None).unwrap(),
        Spec::Git {
            repo: "a/b".into(),
            path: Some("sub/dir".into()),
            git_ref: Some("main".into())
        }
    );
    assert_eq!(
        parse_spec("marketplace:a/b", None).unwrap(),
        Spec::Marketplace("a/b".into())
    );
    assert!(parse_spec("a/../b", None).is_err());
    assert!(parse_spec("nope", None).is_err());
}

#[test]
fn stack_roundtrip() {
    use super::collections::{build_stack, parse_stack, Store};
    let st = Store::open_in_memory().unwrap();
    let c = st
        .create("My stack", None, &["claude".into()], Some("project"))
        .unwrap();
    st.add(&c.id, "cct:agents/a/x", &[], None).unwrap();
    st.add(&c.id, "cct:skills/b/y", &["codex".into()], Some("user"))
        .unwrap();
    st.add(&c.id, "cct:mcps/c/z", &[], None).unwrap();
    let c = st.move_item(&c.id, "cct:mcps/c/z", 0).unwrap();
    assert_eq!(c.items[0].item_id, "cct:mcps/c/z");
    let c = st.remove(&c.id, "cct:agents/a/x").unwrap();
    assert_eq!(
        c.items.iter().map(|i| i.position).collect::<Vec<_>>(),
        vec![0, 1]
    );
    let stack = build_stack(&c, |id| Some(format!("h-{id}")));
    let bytes = serde_json::to_vec(&stack).unwrap();
    let back = parse_stack(&bytes).unwrap();
    assert_eq!(back.items, stack.items);
    let mut tampered: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    tampered["items"][0]["id"] = json!("evil");
    assert!(parse_stack(&serde_json::to_vec(&tampered).unwrap()).is_err());
    st.delete(&c.id).unwrap();
    assert!(st.list().unwrap().is_empty());
}

#[test]
fn scan_generic_layout() {
    let dir = std::env::temp_dir().join(format!("omniget-catalog-scan-{}", uuid::Uuid::new_v4()));
    let w = |p: &str, s: &str| {
        let f = dir.join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, s).unwrap();
    };
    w("agents/dev/a.md", "---\nname: a\ndescription: Agent A <example>x</example>\ntools: Read\nmodel: gpt-4\n---\nbody");
    w("agents/b.md", "# B agent\n\nDoes b things.\n");
    w(
        "commands/git/commit.md",
        "---\ndescription: commit\n---\n$ARGUMENTS",
    );
    w(
        "skills/web/s1/SKILL.md",
        "---\nname: s1\ndescription: skill one\nlicense: Apache-2.0\n---\n",
    );
    w("skills/web/s1/references/r.md", "ref");
    w(
        "skills/web/s1/sub/SKILL.md",
        "---\nname: sub\ndescription: nested\n---\n",
    );
    w(
        "skills/docs/docx/SKILL.md",
        "---\nname: docx\ndescription: x\n---\n",
    );
    w(
        "hooks/sec/guard.json",
        r#"{"description":"guard","hooks":{"PreToolUse":[]}}"#,
    );
    w("hooks/sec/guard.py", "print(1)");
    w(
        "mcps/db/pg.json",
        r#"{"mcpServers":{"pg":{"descrption":"Postgres","command":"npx"}}}"#,
    );
    w(
        "settings/statusline/bar.json",
        r#"{"description":"bar","statusLine":{"command":"python3 .claude/scripts/bar.py"}}"#,
    );
    w("settings/statusline/bar.py", "print()");
    w(
        "loose/thing/SKILL.md",
        "---\nname: thing\ndescription: loose skill\n---\n",
    );
    let src = super::scan::ScanSource {
        id: "user-t".into(),
        default_license: Some("MIT".into()),
        local_root: Some(dir.clone()),
        ..Default::default()
    };
    let r = super::scan::scan(&dir, &src);
    let ids: Vec<&str> = r.items.iter().map(|i| i.id.as_str()).collect();
    for want in [
        "user-t:agents/dev/a",
        "user-t:agents/general/b",
        "user-t:commands/git/commit",
        "user-t:skills/web/s1",
        "user-t:skills/web/s1/sub",
        "user-t:hooks/sec/guard",
        "user-t:mcps/db/pg",
        "user-t:statuslines/statusline/bar",
        "user-t:skills/loose/thing",
    ] {
        assert!(ids.contains(&want), "missing {want} in {ids:?}");
    }
    assert!(!ids.iter().any(|i| i.contains("docx")));
    assert_eq!(r.excluded.len(), 1);
    let a = r
        .items
        .iter()
        .find(|i| i.id == "user-t:agents/dev/a")
        .unwrap();
    assert_eq!(a.description, "Agent A");
    assert!(a.frontmatter.get("model").is_none());
    assert!(a
        .normalization
        .contains(&"removed_invalid_model".to_string()));
    let s1 = r
        .items
        .iter()
        .find(|i| i.id == "user-t:skills/web/s1")
        .unwrap();
    assert_eq!(
        s1.files.len(),
        2,
        "nested skill files excluded: {:?}",
        s1.files
    );
    assert_eq!(s1.license.as_deref(), Some("Apache-2.0"));
    assert_eq!(s1.references, vec!["user-t:skills/web/s1/sub".to_string()]);
    let hook = r
        .items
        .iter()
        .find(|i| i.id == "user-t:hooks/sec/guard")
        .unwrap();
    assert_eq!(hook.files.len(), 2);
    let bar = r.items.iter().find(|i| i.kind == "statusline").unwrap();
    assert_eq!(bar.files.len(), 2);
    let pg = r.items.iter().find(|i| i.kind == "mcp").unwrap();
    assert!(pg
        .normalization
        .contains(&"fixed_description_typo".to_string()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn embedded_index_loads() {
    let ix = super::index::embedded_index().unwrap();
    assert!(ix.items.len() > 1500);
    assert!(ix.items.iter().any(|i| i.kind == "statusline"));
    assert!(!ix
        .items
        .iter()
        .any(|i| i.kind == "skill" && (i.name == "docx" || i.name.ends_with("-official"))));
    let r = search(
        &ix.items,
        "frontend",
        &Filters::default(),
        Sort::Relevance,
        Page::default(),
    );
    let kinds: std::collections::BTreeSet<_> = r.items.iter().map(|h| h.kind.clone()).collect();
    assert!(kinds.len() >= 3, "{kinds:?}");
}
