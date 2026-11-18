use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Impact {
    pub files: u32,
    pub added: u32,
    pub removed: u32,
    pub outside: bool,
}

pub fn tool_base(tool: &str) -> &str {
    tool.split(" (").next().unwrap_or(tool).trim()
}

fn lines(text: &str) -> u32 {
    text.lines().count().min(u32::MAX as usize) as u32
}

fn text<'a>(input: &'a Value, keys: &[&str]) -> &'a str {
    keys.iter()
        .find_map(|k| input.get(*k).and_then(Value::as_str))
        .unwrap_or("")
}

fn is_outside(path: &str, root: Option<&Path>) -> bool {
    use std::path::Component;
    let Some(root) = root else {
        return false;
    };
    let raw = Path::new(path.trim());
    let joined = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        root.join(raw)
    };
    let mut flat = std::path::PathBuf::new();
    for part in joined.components() {
        match part {
            Component::ParentDir => {
                flat.pop();
            }
            Component::CurDir => {}
            other => flat.push(other),
        }
    }
    !flat.starts_with(root)
}

fn patch_impact(patch: &str, root: Option<&Path>) -> Impact {
    let mut out = Impact::default();
    for line in patch.lines() {
        let named = ["*** Update File: ", "*** Add File: ", "*** Delete File: "]
            .iter()
            .find_map(|m| line.strip_prefix(m))
            .or_else(|| {
                line.strip_prefix("+++ ")
                    .filter(|p| *p != "/dev/null")
                    .map(|p| p.strip_prefix("b/").unwrap_or(p))
            });
        if let Some(path) = named {
            out.files += 1;
            out.outside |= is_outside(path, root);
        } else if line.starts_with("---") || line.starts_with("***") {
        } else if line.starts_with('+') {
            out.added += 1;
        } else if line.starts_with('-') {
            out.removed += 1;
        }
    }
    out
}

pub fn impact_of(tool: &str, input: &Value, root: Option<&Path>) -> Option<Impact> {
    let path = text(input, &["path", "file_path"]);
    let one = |added: u32, removed: u32| Impact {
        files: 1,
        added,
        removed,
        outside: is_outside(path, root),
    };
    match tool_base(tool).to_ascii_lowercase().as_str() {
        "fs_edit" | "edit" => Some(one(
            lines(text(input, &["new_string"])),
            lines(text(input, &["old_string"])),
        )),
        "fs_write" | "write" => Some(one(lines(text(input, &["content"])), 0)),
        "multiedit" => {
            let edits = input.get("edits").and_then(Value::as_array)?;
            Some(one(
                edits.iter().map(|e| lines(text(e, &["new_string"]))).sum(),
                edits.iter().map(|e| lines(text(e, &["old_string"]))).sum(),
            ))
        }
        "fs_apply_patch" | "apply_patch" => {
            let found = patch_impact(text(input, &["patch", "input"]), root);
            (found.files > 0).then_some(found)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_edit_counts_the_lines_that_leave_and_enter() {
        let got = impact_of(
            "fs_edit",
            &json!({"path": "src/a.rs", "old_string": "a\nb", "new_string": "a\nb\nc"}),
            Some(Path::new("/ws")),
        );
        assert_eq!(
            got,
            Some(Impact {
                files: 1,
                added: 3,
                removed: 2,
                outside: false
            })
        );
    }

    #[test]
    fn a_write_outside_the_workspace_is_flagged() {
        let got = impact_of(
            "Write (Builder)",
            &json!({"file_path": "/etc/hosts", "content": "x\ny\n"}),
            Some(Path::new("/ws")),
        )
        .unwrap();
        assert!(got.outside);
        assert_eq!((got.files, got.added, got.removed), (1, 2, 0));
    }

    #[test]
    fn dot_dot_does_not_hide_a_path_that_leaves_the_workspace() {
        let ws = Some(Path::new("/ws"));
        for path in ["../etc/hosts", "/ws/../etc/hosts", "a/../../x"] {
            let got = impact_of("fs_write", &json!({"path": path, "content": "x"}), ws).unwrap();
            assert!(got.outside, "{path}");
        }
        let inside = impact_of(
            "fs_write",
            &json!({"path": "a/../b.txt", "content": "x"}),
            ws,
        )
        .unwrap();
        assert!(!inside.outside);
    }

    #[test]
    fn a_multi_edit_sums_its_edits() {
        let got = impact_of(
            "MultiEdit",
            &json!({"file_path": "/ws/a", "edits": [
                {"old_string": "a", "new_string": "b\nc"},
                {"old_string": "d\ne", "new_string": ""}
            ]}),
            Some(Path::new("/ws")),
        )
        .unwrap();
        assert_eq!((got.added, got.removed, got.outside), (2, 3, false));
    }

    #[test]
    fn a_patch_counts_files_and_lines_in_both_formats() {
        let ours = "*** Begin Patch\n*** Update File: a.rs\n-old\n+new\n+more\n*** Add File: /tmp/b.rs\n+x\n*** End Patch";
        let got = impact_of(
            "fs_apply_patch",
            &json!({"patch": ours}),
            Some(Path::new("/ws")),
        )
        .unwrap();
        assert_eq!(
            (got.files, got.added, got.removed, got.outside),
            (2, 3, 1, true)
        );
        let unified = "--- a/x.rs\n+++ b/x.rs\n@@ -1 +1,2 @@\n-a\n+b\n+c\n";
        let got = impact_of("apply_patch", &json!({"patch": unified}), None).unwrap();
        assert_eq!(
            (got.files, got.added, got.removed, got.outside),
            (1, 2, 1, false)
        );
    }

    #[test]
    fn what_cannot_be_measured_has_no_number() {
        assert_eq!(
            impact_of("shell_exec", &json!({"command": "rm -rf x"}), None),
            None
        );
        assert_eq!(
            impact_of("fs_apply_patch", &json!({"patch": "nothing"}), None),
            None
        );
        assert_eq!(impact_of("fs_read", &json!({"path": "a"}), None), None);
    }

    #[test]
    fn the_agent_name_in_the_label_is_not_part_of_the_tool() {
        assert_eq!(tool_base("Edit (Builder)"), "Edit");
        assert_eq!(tool_base("shell_exec"), "shell_exec");
    }
}
