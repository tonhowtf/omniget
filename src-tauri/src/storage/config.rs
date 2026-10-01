use std::path::Path;

use tauri::AppHandle;

use crate::models::settings::AppSettings;
use crate::storage::settings_repository::{self, RepoError, ENVELOPE_KEY, SETTINGS_FILE};

pub fn load_settings(_app: &AppHandle) -> AppSettings {
    settings_repository::global().settings()
}

pub fn load_settings_standalone() -> AppSettings {
    match crate::core::paths::app_data_dir() {
        Some(dir) => read_settings_file(&dir.join(SETTINGS_FILE)),
        None => AppSettings::default(),
    }
}

pub fn read_settings_file(path: &Path) -> AppSettings {
    let Ok(content) = std::fs::read(path) else {
        return AppSettings::default();
    };
    let Ok(json) = serde_json::from_slice::<serde_json::Value>(&content) else {
        return AppSettings::default();
    };
    match json.get(ENVELOPE_KEY) {
        Some(val) => {
            let mut settings = parse_settings(val);
            settings.migrate();
            settings
        }
        None => AppSettings::default(),
    }
}

pub fn update_settings<F>(app: &AppHandle, f: F) -> Result<AppSettings, RepoError>
where
    F: FnOnce(&mut AppSettings),
{
    let commit = settings_repository::global().mutate(None, |current| {
        let mut next = current.clone();
        f(&mut next);
        Ok(next)
    })?;
    crate::storage::settings_effects::after_commit(app, &commit);
    Ok(commit.after)
}

fn parse_settings(val: &serde_json::Value) -> AppSettings {
    recover_settings(val).0
}

pub(crate) fn recover_settings(val: &serde_json::Value) -> (AppSettings, Vec<String>) {
    if let Ok(settings) = serde_json::from_value::<AppSettings>(val.clone()) {
        return (settings, Vec::new());
    }
    let mut merged = serde_json::to_value(AppSettings::default()).unwrap_or_default();
    let mut leaves = Vec::new();
    collect_leaves(&merged, val, "", &mut leaves);
    let mut fallen = Vec::new();
    for (path, value) in leaves {
        let previous = crate::models::settings::get_path(&merged, &path).cloned();
        crate::models::settings::set_path(&mut merged, &path, value);
        if serde_json::from_value::<AppSettings>(merged.clone()).is_err() {
            match previous {
                Some(p) => crate::models::settings::set_path(&mut merged, &path, p),
                None => remove_path(&mut merged, &path),
            }
            fallen.push(path);
        }
    }
    (serde_json::from_value(merged).unwrap_or_default(), fallen)
}

fn collect_leaves(
    base: &serde_json::Value,
    saved: &serde_json::Value,
    prefix: &str,
    out: &mut Vec<(String, serde_json::Value)>,
) {
    let Some(saved) = saved.as_object() else {
        return;
    };
    for (k, v) in saved {
        let path = if prefix.is_empty() {
            k.clone()
        } else {
            format!("{prefix}.{k}")
        };
        match base.get(k) {
            Some(b) if b.is_object() && v.is_object() => collect_leaves(b, v, &path, out),
            _ => out.push((path, v.clone())),
        }
    }
}

fn remove_path(v: &mut serde_json::Value, path: &str) {
    let (parent, key) = match path.rsplit_once('.') {
        Some((p, k)) => (p.split('.').try_fold(&mut *v, |cur, k| cur.get_mut(k)), k),
        None => (Some(v), path),
    };
    if let Some(serde_json::Value::Object(map)) = parent {
        map.remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored() -> serde_json::Value {
        let mut s = AppSettings::default();
        s.download.video_quality = "1080p".into();
        s.bridge.token = "paired-token".into();
        serde_json::to_value(s).unwrap()
    }

    #[test]
    fn a_missing_field_keeps_every_other_setting() {
        let mut val = stored();
        val["download"]
            .as_object_mut()
            .unwrap()
            .remove("video_quality");
        let s = parse_settings(&val);
        assert_eq!(s.bridge.token, "paired-token");
        assert_eq!(
            s.download.video_quality,
            AppSettings::default().download.video_quality
        );
    }

    #[test]
    fn a_field_of_another_type_falls_back_alone() {
        let mut val = stored();
        val["download"]["video_quality"] = serde_json::json!(1080);
        val["download"]["field_from_a_newer_version"] = serde_json::json!(true);
        let s = parse_settings(&val);
        assert_eq!(s.bridge.token, "paired-token");
        assert_eq!(
            s.download.video_quality,
            AppSettings::default().download.video_quality
        );
    }

    #[test]
    fn a_valid_file_is_read_as_is() {
        let s = parse_settings(&stored());
        assert_eq!(s.download.video_quality, "1080p");
        assert_eq!(s.bridge.token, "paired-token");
    }
}

#[cfg(test)]
mod settings_rebrand_audit {
    use super::*;
    use serde_json::{json, Value};

    fn leaf_count(value: &Value) -> usize {
        match value {
            Value::Object(fields) => fields.values().map(leaf_count).sum(),
            _ => 1,
        }
    }

    fn synthetic_values(value: &mut Value, path: &str) {
        if path == "canvas" {
            return;
        }
        match value {
            Value::Object(fields) => {
                for (key, slot) in fields {
                    let child = if path.is_empty() {
                        key.clone()
                    } else {
                        format!("{path}.{key}")
                    };
                    synthetic_values(slot, &child);
                }
            }
            Value::Bool(flag) => *flag = !*flag,
            Value::String(text) => *text = "audit-fixture-only".into(),
            Value::Number(number) => {
                *value = if number.is_f64() {
                    json!(1.5)
                } else {
                    json!(number.as_u64().expect("unsigned settings default") + 1)
                };
            }
            Value::Array(_) => {
                *value = if matches!(path, "league.pick_champions" | "league.ban_champions") {
                    json!([1, 2])
                } else if path == "tube.blocked_channels" {
                    json!([{ "id": "audit-fixture-only", "name": "audit-fixture-only" }])
                } else {
                    json!(["audit-fixture-only"])
                };
            }
            Value::Null => {
                *value = match path {
                    "llm.ask_timeout_job_s" => json!(45),
                    "llm.jobs_max_per_agent" => json!(2),
                    "last_download_options.mode"
                    | "last_download_options.quality"
                    | "tube.download_folder" => json!("audit-fixture-only"),
                    _ => panic!("audit inventory needs the type of nullable field {path}"),
                };
            }
        }
    }

    #[test]
    fn actual_rust_defaults_match_the_leaf_harness_fixture() {
        let mut actual = serde_json::to_value(AppSettings::default()).unwrap();
        actual["download"]["default_output_dir"] =
            json!("/tmp/omniget-settings-rebrand-fixture/downloads");
        let expected: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/settings-audit-defaults.json"
        ))
        .unwrap();
        assert_eq!(leaf_count(&actual), 238);
        assert_eq!(leaf_count(&expected), 238);
        let typed_fixture: AppSettings = serde_json::from_value(expected).unwrap();
        assert_eq!(actual, serde_json::to_value(typed_fixture).unwrap());
    }

    #[test]
    fn all_settings_keep_nondefault_values_through_the_real_codec() {
        let mut expected = serde_json::to_value(AppSettings::default()).unwrap();
        synthetic_values(&mut expected, "");
        assert_eq!(leaf_count(&expected), 238);
        let decoded: AppSettings = serde_json::from_value(expected.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), expected);
        assert_eq!(
            serde_json::to_value(parse_settings(&expected)).unwrap(),
            expected
        );
    }

    #[test]
    fn a_wrong_json_kind_falls_back_only_for_that_preference() {
        let mut expected = serde_json::to_value(AppSettings::default()).unwrap();
        synthetic_values(&mut expected, "");
        expected["proxy"]["port"] = json!(AppSettings::default().proxy.port);
        let mut saved = expected.clone();
        saved["proxy"]["port"] = json!("invalid-port-fixture");
        assert_eq!(
            serde_json::to_value(parse_settings(&saved)).unwrap(),
            expected
        );
    }

    #[test]
    fn an_out_of_range_proxy_port_preserves_every_other_preference() {
        let mut expected = serde_json::to_value(AppSettings::default()).unwrap();
        synthetic_values(&mut expected, "");
        expected["proxy"]["port"] = json!(AppSettings::default().proxy.port);
        let mut saved = expected.clone();
        saved["proxy"]["port"] = json!(70_000);
        let actual = serde_json::to_value(parse_settings(&saved)).unwrap();
        assert_eq!(actual["appearance"], expected["appearance"]);
        assert_eq!(
            actual["download"]["video_quality"],
            expected["download"]["video_quality"]
        );
        assert_eq!(actual["bridge"]["token"], expected["bridge"]["token"]);
        assert_eq!(actual, expected);
    }
}
