use serde_json::{json, Value};

use super::super::codehub;
use super::super::model::Platform;

pub const VIEWS: [(&str, &str); 3] = [
    ("stargazers", "followers"),
    ("new_stargazers", "new_followers"),
    ("unstarred", "unfollowers"),
];

pub const WITH_REPO: [&str; 4] = ["summary", "trend", "kept", "hidden"];

pub fn base_view(name: &str) -> Option<&'static str> {
    VIEWS.iter().find(|(id, _)| *id == name).map(|(_, v)| *v)
}

pub fn repo_key(raw: &str) -> Option<String> {
    codehub::repo_of(raw.trim()).map(|r| r.to_lowercase())
}

pub fn supports(p: Platform) -> bool {
    p == Platform::CodeHub
}

pub fn resolve(
    p: Platform,
    view: &str,
    repo: Option<&str>,
) -> Result<(String, Option<String>), String> {
    let repo = match repo {
        Some(_) if !supports(p) => {
            return Err(format!(
                "INVALID_ARGUMENTS: repo is a Code Hub argument; {} has no repositories",
                p.label()
            ))
        }
        Some(raw) => Some(repo_key(raw).ok_or_else(|| {
            "INVALID_ARGUMENTS: repo must be owner/name or a github.com link".to_string()
        })?),
        None => None,
    };
    match (base_view(view).filter(|_| supports(p)), &repo) {
        (Some(base), Some(_)) => Ok((base.to_string(), repo)),
        (Some(_), None) => Err(format!(
            "INVALID_ARGUMENTS: view {view} needs repo (owner/name), the repository whose stargazers were synced"
        )),
        (None, Some(_)) if !WITH_REPO.contains(&view) => Err(format!(
            "INVALID_ARGUMENTS: with repo the view must be stargazers, new_stargazers, unstarred, summary, trend, kept or hidden, not {view}"
        )),
        (None, _) => Ok((view.to_string(), repo)),
    }
}

pub fn list_call(repo: Option<&str>, kind: &str, cursor: Option<&str>) -> (&'static str, Value) {
    let mut args = json!({});
    if let Some(c) = cursor {
        args["cursor"] = json!(c);
    }
    match repo {
        Some(r) => {
            args["repo"] = json!(r);
            ("stargazers", args)
        }
        None if kind == "following" => ("following", args),
        None => ("followers", args),
    }
}

fn rename(o: &mut serde_json::Map<String, Value>) {
    for (star, base) in VIEWS {
        if let Some(v) = o.remove(base) {
            o.insert(star.to_string(), v);
        }
    }
    for gone in [
        "following",
        "not_following_back",
        "fans",
        "mutuals",
        "inactive",
        "inactive_unknown",
    ] {
        o.remove(gone);
    }
}

pub fn relabel(out: &mut Value, view: &str, repo: &str) {
    out["view"] = json!(view);
    out["repo"] = json!(repo);
    if let Some(o) = out.as_object_mut() {
        o.remove("account");
        o.remove("inactive_days");
    }
    if let Some(items) = out["items"].as_array_mut() {
        for item in items {
            if let Some(o) = item.as_object_mut() {
                o.remove("follows_me");
                o.remove("followed_by_me");
                if o.get("status").and_then(Value::as_str) == Some("unfollowed") {
                    o.insert("status".into(), json!("unstarred"));
                }
            }
        }
    }
    for key in ["counts", "synced_at"] {
        if let Some(o) = out[key].as_object_mut() {
            rename(o);
        }
    }
    if let Some(lists) = out["lists"].as_array_mut() {
        for l in lists {
            l["kind"] = json!("stargazers");
        }
    }
    if let Some(days) = out["days"].as_array_mut() {
        for d in days {
            if let Some(o) = d.as_object_mut() {
                if let Some(n) = o.remove("followers") {
                    o.insert("stargazers".into(), n);
                }
                o.remove("following");
                o.remove("ratio");
            }
        }
    }
    if let Some(step) = out["next_step"].as_str() {
        if step.contains("audience_sync") {
            out["next_step"] = json!(format!(
                "Call gh_audience_sync with repo={repo} until complete is true, then read this view again; un-starred accounts appear from the second complete sync."
            ));
        }
    }
}

pub fn relabel_sync(out: &mut Value, repo: &str) {
    out["repo"] = json!(repo);
    if let Some(o) = out.as_object_mut() {
        o.remove("account");
    }
    if let Some(lists) = out["lists"].as_array_mut() {
        for l in lists {
            l["kind"] = json!("stargazers");
        }
    }
    if out["complete"] == true {
        out["next_step"] = json!(format!(
            "Read gh_audience with repo={repo}: view stargazers now; new_stargazers and unstarred show what changed between two complete syncs of this repository."
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stargazer_views_need_a_repository_and_only_exist_on_codehub() {
        assert_eq!(
            resolve(
                Platform::CodeHub,
                "unstarred",
                Some("https://github.com/Ana/Tool")
            ),
            Ok(("unfollowers".into(), Some("ana/tool".into())))
        );
        assert_eq!(
            resolve(Platform::CodeHub, "summary", Some("ana/tool")),
            Ok(("summary".into(), Some("ana/tool".into())))
        );
        assert_eq!(
            resolve(Platform::CodeHub, "fans", None),
            Ok(("fans".into(), None))
        );
        for bad in [
            resolve(Platform::CodeHub, "stargazers", None),
            resolve(Platform::CodeHub, "fans", Some("ana/tool")),
            resolve(Platform::CodeHub, "stargazers", Some("not a repo")),
            resolve(Platform::PinBoard, "followers", Some("ana/tool")),
        ] {
            assert!(bad.unwrap_err().starts_with("INVALID_ARGUMENTS"));
        }
        assert_eq!(
            resolve(Platform::PinBoard, "stargazers", None),
            Ok(("stargazers".into(), None))
        );
    }

    #[test]
    fn a_repository_list_is_read_with_the_stargazers_tool() {
        assert_eq!(
            list_call(Some("ana/tool"), "followers", Some("3")),
            ("stargazers", json!({ "repo": "ana/tool", "cursor": "3" }))
        );
        assert_eq!(list_call(None, "followers", None), ("followers", json!({})));
        assert_eq!(
            list_call(None, "following", Some("2")),
            ("following", json!({ "cursor": "2" }))
        );
    }

    #[test]
    fn a_repository_view_speaks_of_stars_not_follows() {
        let mut out = json!({
            "view": "unfollowers", "account": "ana/tool", "total": 1,
            "items": [{ "handle": "bob", "follows_me": false, "status": "unfollowed", "between": [1, 2] }, { "handle": "eve", "status": "gone" }],
            "synced_at": { "followers": 9, "following": null },
        });
        relabel(&mut out, "unstarred", "ana/tool");
        assert_eq!(out["view"], "unstarred");
        assert_eq!(out["repo"], "ana/tool");
        assert!(out.get("account").is_none());
        assert_eq!(out["items"][0]["status"], "unstarred");
        assert_eq!(out["items"][0]["between"], json!([1, 2]));
        assert!(out["items"][0].get("follows_me").is_none());
        assert_eq!(out["items"][1]["status"], "gone");
        assert_eq!(out["synced_at"], json!({ "stargazers": 9 }));
        let mut summary = json!({
            "view": "summary",
            "counts": { "followers": 3, "following": 0, "new_followers": 1, "unfollowers": 1, "fans": 3, "kept": 0, "hidden": 0, "inactive_unknown": 0 },
            "lists": [{ "kind": "followers", "state": "complete" }],
        });
        relabel(&mut summary, "summary", "ana/tool");
        assert_eq!(
            summary["counts"],
            json!({ "stargazers": 3, "new_stargazers": 1, "unstarred": 1, "kept": 0, "hidden": 0 })
        );
        assert_eq!(summary["lists"][0]["kind"], "stargazers");
        let mut trend = json!({ "view": "trend", "days": [{ "day": "2026-11-11", "followers": 3, "following": 0, "ratio": null, "gained": 1, "lost": 0 }] });
        relabel(&mut trend, "trend", "ana/tool");
        assert_eq!(
            trend["days"][0],
            json!({ "day": "2026-11-11", "stargazers": 3, "gained": 1, "lost": 0 })
        );
    }
}
