use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

pub const MAX_PROMPT: usize = 12_000;
const MAX_VALUE: usize = 200;
const REFERENCE_PARAM: &str = "image_references";
const MEDIA_WORDS: &[&str] = &[
    "image",
    "video",
    "audio",
    "mask",
    "media",
    "file",
    "reference",
    "upload",
    "soul",
    "path",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSummary {
    pub model: String,
    pub name: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParamSpec {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub default: Value,
    pub required: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    pub editable: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSchema {
    pub model: String,
    pub name: String,
    pub kind: String,
    pub params: Vec<ParamSpec>,
    pub references: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenRequest {
    pub model: String,
    pub prompt: String,
    #[serde(default)]
    pub params: Map<String, Value>,
    #[serde(default)]
    pub references: Vec<String>,
}

pub fn identifier(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.as_bytes()[0].is_ascii_alphanumeric()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn media_name(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    MEDIA_WORDS.iter().any(|w| l.contains(w))
}

pub fn parse_model_list(raw: &Value, kind: &str) -> Result<Vec<ModelSummary>, String> {
    let rows = raw.as_array().ok_or("PROVIDER_INVALID_RESPONSE")?;
    Ok(rows
        .iter()
        .take(500)
        .filter_map(|v| {
            let model = v.get("job_type")?.as_str()?;
            let k = v.get("type").and_then(Value::as_str).unwrap_or("");
            if !identifier(model) || k != kind {
                return None;
            }
            let name = v
                .get("display_name")
                .and_then(Value::as_str)
                .map(|s| s.chars().take(80).collect::<String>())
                .unwrap_or_else(|| model.to_string());
            Some(ModelSummary {
                model: model.to_string(),
                name,
                kind: k.to_string(),
            })
        })
        .collect())
}

pub fn parse_model(raw: &Value, expect: &str) -> Result<ModelSchema, String> {
    let model = raw
        .get("job_type")
        .and_then(Value::as_str)
        .ok_or("PROVIDER_INVALID_RESPONSE")?;
    if model != expect {
        return Err("PROVIDER_MODEL_MISMATCH".into());
    }
    let kind = raw
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let name = raw
        .get("display_name")
        .and_then(Value::as_str)
        .unwrap_or(model)
        .chars()
        .take(80)
        .collect();
    let mut params = Vec::new();
    let mut references = false;
    for p in raw
        .get("params")
        .and_then(Value::as_array)
        .ok_or("PROVIDER_INVALID_RESPONSE")?
        .iter()
        .take(64)
    {
        let Some(pname) = p.get("name").and_then(Value::as_str) else {
            continue;
        };
        if !identifier(pname) {
            continue;
        }
        let ty = p
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("string")
            .to_string();
        let options: Vec<String> = p
            .get("enum")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .filter(|s| s.len() <= MAX_VALUE)
                    .map(str::to_string)
                    .take(64)
                    .collect()
            })
            .unwrap_or_default();
        if pname == REFERENCE_PARAM {
            references = true;
        }
        let base = ty.split('|').next().unwrap_or("").to_string();
        let editable = pname != "prompt"
            && matches!(base.as_str(), "string" | "boolean" | "number" | "integer")
            && (!media_name(pname) || !options.is_empty());
        params.push(ParamSpec {
            name: pname.to_string(),
            ty,
            default: p.get("default").cloned().unwrap_or(Value::Null),
            required: p.get("required").and_then(Value::as_bool).unwrap_or(false),
            options,
            editable,
        });
    }
    Ok(ModelSchema {
        model: model.to_string(),
        name,
        kind,
        params,
        references,
    })
}

fn safe_text(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= MAX_VALUE
        && !v.starts_with(['@', '-', '.', '~'])
        && !v.contains(['/', '\\', '\0', '\n', '\r'])
}

pub fn check_prompt(prompt: &str) -> Result<(), String> {
    if prompt.trim().is_empty() {
        return Err("PROVIDER_INVALID_PROMPT: the prompt is empty".into());
    }
    if prompt.len() > MAX_PROMPT {
        return Err(format!(
            "PROVIDER_INVALID_PROMPT: the prompt is longer than {MAX_PROMPT} bytes"
        ));
    }
    if prompt.trim_start().starts_with('@') || prompt.contains('\0') {
        return Err("PROVIDER_INVALID_PROMPT: the prompt cannot start with @".into());
    }
    Ok(())
}

fn render(spec: &ParamSpec, v: &Value) -> Result<String, String> {
    let bad = || format!("PROVIDER_INVALID_PARAMETER: {}", spec.name);
    let types: Vec<&str> = spec.ty.split('|').collect();
    let out = match v {
        Value::String(s) => {
            if !types.contains(&"string") {
                return Err(bad());
            }
            if spec.options.is_empty() {
                if !safe_text(s) {
                    return Err(bad());
                }
            } else if !spec.options.iter().any(|o| o == s) {
                return Err(format!(
                    "PROVIDER_INVALID_PARAMETER: {} must be one of {}",
                    spec.name,
                    spec.options.join(", ")
                ));
            }
            s.clone()
        }
        Value::Bool(b) if types.contains(&"boolean") => b.to_string(),
        Value::Number(n) if types.contains(&"number") || types.contains(&"integer") => {
            if types.contains(&"integer") && !types.contains(&"number") && !n.is_i64() {
                return Err(bad());
            }
            let f = n.as_f64().ok_or_else(bad)?;
            if !f.is_finite() || f.abs() > 1e9 {
                return Err(bad());
            }
            n.to_string()
        }
        _ => return Err(bad()),
    };
    if out.starts_with('@') {
        return Err(bad());
    }
    Ok(out)
}

pub fn clean_params(
    schema: &ModelSchema,
    params: &Map<String, Value>,
) -> Result<Map<String, Value>, String> {
    let mut out = Map::new();
    for (k, v) in params {
        if v.is_null() {
            continue;
        }
        let spec = schema.params.iter().find(|p| &p.name == k).ok_or_else(|| {
            format!(
                "PROVIDER_UNKNOWN_PARAMETER: {k} (accepted: {})",
                schema
                    .params
                    .iter()
                    .filter(|p| p.editable)
                    .map(|p| p.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;
        if !spec.editable {
            return Err(format!(
                "PROVIDER_INVALID_PARAMETER: {k} cannot be set here (prompt and media come from the node and its cords)"
            ));
        }
        render(spec, v)?;
        if spec.default != *v {
            out.insert(k.clone(), v.clone());
        }
    }
    for p in &schema.params {
        if p.required
            && p.name != "prompt"
            && p.default.is_null()
            && !out.contains_key(&p.name)
            && p.name != REFERENCE_PARAM
        {
            return Err(format!("PROVIDER_MISSING_PARAMETER: {}", p.name));
        }
    }
    Ok(out)
}

pub fn args(verb: &str, schema: &ModelSchema, req: &GenRequest) -> Result<Vec<String>, String> {
    if !matches!(verb, "cost" | "create") {
        return Err("PROVIDER_INVALID_VERB".into());
    }
    if !identifier(&req.model) || req.model != schema.model {
        return Err("PROVIDER_MODEL_MISMATCH".into());
    }
    check_prompt(&req.prompt)?;
    let params = clean_params(schema, &req.params)?;
    let mut a = vec![
        "generate".to_string(),
        verb.to_string(),
        req.model.clone(),
        format!("--prompt={}", req.prompt),
    ];
    for (k, v) in &params {
        let spec = schema
            .params
            .iter()
            .find(|p| &p.name == k)
            .ok_or("PROVIDER_INVALID_PARAMETER")?;
        a.push(format!("--{k}={}", render(spec, v)?));
    }
    if !req.references.is_empty() {
        if !schema.references {
            return Err(
                "PROVIDER_REFERENCES_UNSUPPORTED: this model takes no reference images".into(),
            );
        }
        if req.references.len() > 8 {
            return Err("PROVIDER_TOO_MANY_REFERENCES: at most 8".into());
        }
        for r in &req.references {
            if !upload_id(r) {
                return Err("PROVIDER_INVALID_REFERENCE".into());
            }
            a.push(format!("--image-references={r}"));
        }
    }
    a.push("--json".into());
    a.push("--no-color".into());
    Ok(a)
}

pub fn upload_id(s: &str) -> bool {
    s.len() == 36
        && s.bytes().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            _ => b.is_ascii_hexdigit(),
        })
}

pub fn fingerprint(
    model: &str,
    prompt: &str,
    params: &Map<String, Value>,
    refs: &[String],
) -> String {
    let sorted: std::collections::BTreeMap<&String, &Value> = params.iter().collect();
    let body = serde_json::json!({
        "model": model,
        "prompt": prompt,
        "params": sorted,
        "refs": refs,
    });
    hex::encode(Sha256::digest(body.to_string().as_bytes()))
}

pub fn credits(raw: &Value) -> Result<f64, String> {
    raw.get("credits")
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite() && *v >= 0.0 && *v <= 1_000_000.0)
        .ok_or_else(|| "PROVIDER_COST_UNKNOWN".to_string())
}

pub fn micro(credits: f64) -> i64 {
    (credits * 1_000_000.0).ceil().clamp(0.0, 1e15) as i64
}

pub fn from_micro(v: i64) -> f64 {
    (v as f64) / 1_000_000.0
}

pub fn submitted_id(raw: &Value) -> Option<String> {
    let one = |a: &Vec<Value>| -> Option<String> {
        if a.len() != 1 {
            return None;
        }
        a[0].as_str()
            .or_else(|| a[0].get("id").and_then(Value::as_str))
            .or_else(|| a[0].get("job_id").and_then(Value::as_str))
            .map(str::to_string)
    };
    let id = match raw {
        Value::String(s) => Some(s.clone()),
        Value::Array(a) => one(a),
        Value::Object(_) => raw
            .get("id")
            .or_else(|| raw.get("job_id"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| raw.get("job_ids").and_then(Value::as_array).and_then(one))
            .or_else(|| raw.get("jobs").and_then(Value::as_array).and_then(one)),
        _ => None,
    }?;
    identifier(&id).then_some(id)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobStatus {
    pub id: String,
    pub status: String,
    pub raw_status: String,
    pub model: String,
    pub name: String,
    pub prompt: String,
    pub created_at: String,
    pub result_url: Option<String>,
}

pub fn normalize_status(s: &str) -> &'static str {
    match s.to_ascii_lowercase().as_str() {
        "completed" | "succeeded" | "success" | "done" => "completed",
        "failed" | "error" | "cancelled" | "canceled" | "nsfw" | "rejected" | "timeout" => "failed",
        "queued" | "pending" | "waiting" | "created" => "queued",
        _ => "running",
    }
}

pub fn parse_job(raw: &Value) -> Option<JobStatus> {
    let id = raw
        .get("id")
        .or_else(|| raw.get("job_id"))
        .and_then(Value::as_str)?;
    if !identifier(id) {
        return None;
    }
    let raw_status = raw
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .chars()
        .take(40)
        .collect::<String>();
    let result_url = raw
        .get("result_url")
        .and_then(Value::as_str)
        .or_else(|| {
            raw.get("results")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(|v| v.get("url").or_else(|| v.get("raw")))
                .and_then(|v| v.as_str().or_else(|| v.get("url").and_then(Value::as_str)))
        })
        .filter(|u| u.starts_with("https://") && u.len() <= 8192)
        .map(str::to_string);
    Some(JobStatus {
        id: id.to_string(),
        status: normalize_status(&raw_status).to_string(),
        raw_status,
        model: raw
            .get("job_type")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        name: raw
            .get("display_name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        prompt: raw
            .get("params")
            .and_then(|p| p.get("prompt"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        created_at: raw
            .get("created_at")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        result_url,
    })
}

pub fn parse_jobs(raw: &Value) -> Vec<JobStatus> {
    raw.as_array()
        .or_else(|| raw.get("items").and_then(Value::as_array))
        .or_else(|| raw.get("jobs").and_then(Value::as_array))
        .map(|a| a.iter().take(200).filter_map(parse_job).collect())
        .unwrap_or_default()
}

pub fn parse_upload(raw: &Value) -> Result<String, String> {
    let id = raw
        .get("id")
        .or_else(|| raw.get("upload_id"))
        .and_then(Value::as_str)
        .ok_or("PROVIDER_INVALID_RESPONSE")?;
    if !upload_id(id) {
        return Err("PROVIDER_INVALID_RESPONSE".into());
    }
    Ok(id.to_string())
}

pub fn parse_balance(raw: &Value) -> Result<(f64, String), String> {
    let credits = raw
        .get("credits")
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite() && *v >= 0.0)
        .ok_or("PROVIDER_BALANCE_UNKNOWN")?;
    let account = raw
        .get("email")
        .and_then(Value::as_str)
        .map(|e| hex::encode(&Sha256::digest(e.trim().to_ascii_lowercase().as_bytes())[..8]))
        .unwrap_or_else(|| "default".into());
    Ok((credits, account))
}

#[derive(Debug, Clone, PartialEq)]
pub struct Spend {
    pub at: String,
    pub credits: f64,
    pub name: String,
}

pub fn parse_spends(raw: &Value) -> Vec<Spend> {
    raw.get("items")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .take(100)
                .filter(|v| v.get("action").and_then(Value::as_str) == Some("spend"))
                .filter_map(|v| {
                    Some(Spend {
                        at: v.get("created_at")?.as_str()?.to_string(),
                        credits: v.get("credits")?.as_f64()?.abs(),
                        name: v
                            .get("display_name")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn aspect(params: &Map<String, Value>, schema: Option<&ModelSchema>) -> Option<(f64, f64)> {
    let raw = params
        .get("aspect_ratio")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            schema?
                .params
                .iter()
                .find(|p| p.name == "aspect_ratio")?
                .default
                .as_str()
                .map(str::to_string)
        })?;
    let (w, h) = raw.split_once(':')?;
    let (w, h) = (w.trim().parse::<f64>().ok()?, h.trim().parse::<f64>().ok()?);
    (w > 0.0 && h > 0.0 && w.is_finite() && h.is_finite()).then_some((w, h))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn banana() -> ModelSchema {
        parse_model(
            &json!({
                "display_name": "Nano Banana 2",
                "job_type": "nano_banana_flash",
                "params": [
                    {"name": "aspect_ratio", "type": "string", "default": "1:1", "required": false, "enum": ["auto", "1:1", "16:9"]},
                    {"name": "image_references", "type": "array", "default": null, "required": false},
                    {"name": "is_inpaint", "type": "boolean", "default": false, "required": false},
                    {"name": "mask", "type": "object|null", "default": null, "required": false},
                    {"name": "prompt", "type": "string", "default": null, "required": true},
                    {"name": "resolution", "type": "string", "default": "1k", "required": false, "enum": ["1k", "2k", "4k"]},
                    {"name": "start_image", "type": "string", "default": null, "required": false},
                    {"name": "seed", "type": "integer", "default": null, "required": false}
                ],
                "rules": [],
                "type": "image"
            }),
            "nano_banana_flash",
        )
        .unwrap()
    }

    fn req(params: Value, refs: &[&str]) -> GenRequest {
        GenRequest {
            model: "nano_banana_flash".into(),
            prompt: "a red apple".into(),
            params: params.as_object().cloned().unwrap_or_default(),
            references: refs.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn model_list_keeps_only_the_kind() {
        let raw = json!([
            {"display_name": "FLUX.2", "job_type": "flux_2", "type": "image"},
            {"display_name": "Seedance", "job_type": "seedance_2_5", "type": "video"},
            {"display_name": "Bad", "job_type": "../x", "type": "image"}
        ]);
        let list = parse_model_list(&raw, "image").unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].model, "flux_2");
        assert_eq!(list[0].name, "FLUX.2");
    }

    #[test]
    fn schema_marks_media_and_prompt_as_not_editable() {
        let s = banana();
        assert!(s.references);
        let editable: Vec<&str> = s
            .params
            .iter()
            .filter(|p| p.editable)
            .map(|p| p.name.as_str())
            .collect();
        assert_eq!(
            editable,
            vec!["aspect_ratio", "is_inpaint", "resolution", "seed"]
        );
        assert!(parse_model(&json!({"job_type": "x", "params": []}), "y").is_err());
    }

    #[test]
    fn args_are_built_from_the_schema() {
        let a = args(
            "cost",
            &banana(),
            &req(
                json!({"aspect_ratio": "16:9", "resolution": "1k", "seed": 7}),
                &[],
            ),
        )
        .unwrap();
        assert_eq!(
            a,
            vec![
                "generate",
                "cost",
                "nano_banana_flash",
                "--prompt=a red apple",
                "--aspect_ratio=16:9",
                "--seed=7",
                "--json",
                "--no-color"
            ]
        );
    }

    #[test]
    fn media_and_paths_never_reach_the_cli() {
        let s = banana();
        assert!(args(
            "create",
            &s,
            &req(json!({"start_image": "/etc/passwd"}), &[])
        )
        .is_err());
        assert!(args(
            "create",
            &s,
            &req(json!({"image_references": ["./a.png"]}), &[])
        )
        .is_err());
        assert!(args("create", &s, &req(json!({"mask": {"a": 1}}), &[])).is_err());
        assert!(args("create", &s, &req(json!({"resolution": "9k"}), &[])).is_err());
        assert!(args("create", &s, &req(json!({"unknown": "1"}), &[])).is_err());
        assert!(args("create", &s, &req(json!({}), &["./photo.png"])).is_err());
        assert!(args("create", &s, &req(json!({"seed": 1.5}), &[])).is_err());
        let mut r = req(json!({}), &[]);
        r.prompt = "@/Users/me/.ssh/id_rsa".into();
        assert!(args("create", &s, &r).is_err());
        let ok = args(
            "create",
            &s,
            &req(json!({}), &["87cce316-38ae-4b60-b209-edf1a72d2846"]),
        )
        .unwrap();
        assert!(ok.contains(&"--image-references=87cce316-38ae-4b60-b209-edf1a72d2846".to_string()));
    }

    #[test]
    fn references_need_a_model_that_takes_them() {
        let mut s = banana();
        s.references = false;
        let e = args(
            "create",
            &s,
            &req(json!({}), &["87cce316-38ae-4b60-b209-edf1a72d2846"]),
        )
        .unwrap_err();
        assert!(e.starts_with("PROVIDER_REFERENCES_UNSUPPORTED"));
    }

    #[test]
    fn fingerprint_is_canonical_and_covers_refs() {
        let mut a = Map::new();
        a.insert("b".into(), json!(1));
        a.insert("a".into(), json!("x"));
        let mut b = Map::new();
        b.insert("a".into(), json!("x"));
        b.insert("b".into(), json!(1));
        let f1 = fingerprint("m", "p", &a, &["h1".into()]);
        assert_eq!(f1, fingerprint("m", "p", &b, &["h1".into()]));
        assert_ne!(f1, fingerprint("m", "p", &b, &["h2".into()]));
        assert_ne!(f1, fingerprint("m", "p2", &b, &["h1".into()]));
    }

    #[test]
    fn cli_outputs_parse() {
        assert_eq!(credits(&json!({"credits": 1.5})).unwrap(), 1.5);
        assert!(credits(&json!({"credits": -1})).is_err());
        assert_eq!(micro(1.5), 1_500_000);
        assert_eq!(micro(0.0000001), 1);
        assert_eq!(
            submitted_id(&json!(["cc114105-2f5a-452f-9c5d-897e233e3eb1"])).as_deref(),
            Some("cc114105-2f5a-452f-9c5d-897e233e3eb1")
        );
        assert_eq!(
            submitted_id(&json!({"job_ids": ["a1"]})).as_deref(),
            Some("a1")
        );
        assert_eq!(submitted_id(&json!([{"id": "b2"}])).as_deref(), Some("b2"));
        assert_eq!(submitted_id(&json!(["a", "b"])), None);
        assert_eq!(submitted_id(&json!({"id": "../x"})), None);
        let job = parse_job(&json!({
            "created_at": "2026-10-01T12:35:53.544447Z",
            "display_name": "GPT Image 2.5",
            "id": "cc114105-2f5a-452f-9c5d-897e233e3eb1",
            "job_type": "gpt_image_2_5",
            "params": {"prompt": "x"},
            "result_url": "https://d8j0ntlcm91z4.cloudfront.net/a.png",
            "status": "completed"
        }))
        .unwrap();
        assert_eq!(job.status, "completed");
        assert_eq!(job.prompt, "x");
        assert_eq!(
            job.result_url.as_deref(),
            Some("https://d8j0ntlcm91z4.cloudfront.net/a.png")
        );
        let bad =
            parse_job(&json!({"id": "a", "status": "in_progress", "result_url": "http://x/a.png"}))
                .unwrap();
        assert_eq!(bad.status, "running");
        assert_eq!(bad.result_url, None);
        assert_eq!(parse_jobs(&json!([{"id": "a"}, {"nope": 1}])).len(), 1);
        assert_eq!(
            parse_upload(&json!({"id": "87cce316-38ae-4b60-b209-edf1a72d2846", "type": "image"}))
                .unwrap(),
            "87cce316-38ae-4b60-b209-edf1a72d2846"
        );
        assert!(parse_upload(&json!({"id": "x"})).is_err());
        let (c, acct) = parse_balance(&json!({"credits": 3159.35, "email": "A@b.c"})).unwrap();
        assert_eq!(c, 3159.35);
        assert_eq!(
            acct,
            parse_balance(&json!({"credits": 1, "email": "a@b.c"}))
                .unwrap()
                .1
        );
        let spends = parse_spends(&json!({"items": [
            {"action": "spend", "created_at": "2026-10-01T12:35:53Z", "credits": -0.5, "display_name": "GPT Image 2.5 Flare"},
            {"action": "topup", "created_at": "2026-10-01T12:00:00Z", "credits": 100}
        ]}));
        assert_eq!(spends.len(), 1);
        assert_eq!(spends[0].credits, 0.5);
    }

    #[test]
    fn aspect_reads_params_or_default() {
        let s = banana();
        let mut p = Map::new();
        assert_eq!(aspect(&p, Some(&s)), Some((1.0, 1.0)));
        p.insert("aspect_ratio".into(), json!("16:9"));
        assert_eq!(aspect(&p, Some(&s)), Some((16.0, 9.0)));
        p.insert("aspect_ratio".into(), json!("auto"));
        assert_eq!(aspect(&p, Some(&s)), None);
    }
}
