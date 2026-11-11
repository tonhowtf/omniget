use std::time::Duration;

use base64::Engine;
use serde_json::{json, Value};

use super::super::model::Platform;
use super::super::page as web;
use super::super::x;
use super::rules;

const UPLOAD: &str = "https://upload.x.com/i/media/upload.json";
const SEGMENT: usize = 1024 * 1024;
const P: Platform = Platform::X;

pub fn init_url(bytes: usize, mime: &str) -> String {
    format!(
        "{UPLOAD}?command=INIT&total_bytes={bytes}&media_type={}&media_category=tweet_image",
        urlencoding::encode(mime)
    )
}

pub fn append_url(id: &str, segment: usize) -> String {
    format!("{UPLOAD}?command=APPEND&media_id={id}&segment_index={segment}")
}

pub fn finalize_url(id: &str) -> String {
    format!("{UPLOAD}?command=FINALIZE&media_id={id}")
}

pub fn status_url(id: &str) -> String {
    format!("{UPLOAD}?command=STATUS&media_id={id}")
}

pub fn media_id(v: &Value) -> Result<String, String> {
    v["media_id_string"]
        .as_str()
        .map(String::from)
        .or_else(|| v["media_id"].as_u64().map(|n| n.to_string()))
        .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
        .ok_or_else(|| "UPSTREAM_CHANGED: X did not return a media id for the upload".to_string())
}

#[derive(Debug, PartialEq)]
pub enum Processing {
    Ready,
    Wait(u64),
}

pub fn processing(v: &Value) -> Result<Processing, String> {
    let info = &v["processing_info"];
    match info["state"].as_str() {
        None | Some("succeeded") => Ok(Processing::Ready),
        Some("pending") | Some("in_progress") => Ok(Processing::Wait(
            info["check_after_secs"].as_u64().unwrap_or(1).clamp(1, 10),
        )),
        Some("failed") => Err(format!(
            "REJECTED: X could not process the image: {}",
            info["error"]["message"]
                .as_str()
                .unwrap_or("no reason given")
        )),
        Some(other) => Err(format!(
            "UPSTREAM_CHANGED: unknown media processing state {other}"
        )),
    }
}

pub fn reply(v: &Value) -> Result<Value, String> {
    let status = v["status"].as_u64().unwrap_or(0);
    let body = v["body"].as_str().unwrap_or_default();
    match status {
        200..=299 if body.trim().is_empty() => Ok(Value::Null),
        200..=299 => serde_json::from_str(body).map_err(|_| {
            "UPSTREAM_CHANGED: the X upload answered something that is not JSON".to_string()
        }),
        401 | 403 => {
            Err("NOT_LOGGED_IN: X rejected the session for the upload; sign in again".into())
        }
        429 => {
            Err("RATE_LIMITED: X is limiting uploads from this account; wait a few minutes".into())
        }
        0 => Err("NETWORK: could not reach the X upload server".into()),
        _ => Err(format!(
            "UPSTREAM_{status}: the X upload answered {status}: {}",
            body.chars().take(200).collect::<String>().trim()
        )),
    }
}

fn request_js(method: &str, url: &str, headers: &Value, chunk: Option<&str>) -> String {
    format!(
        "(async()=>{{const b={chunk};let body;if(b!==null){{const s=atob(b);const u=new Uint8Array(s.length);for(let i=0;i<s.length;i++)u[i]=s.charCodeAt(i);body=new FormData();body.append('media',new Blob([u]),'blob');}}\
         try{{const r=await fetch({url},{{method:{method},credentials:'include',headers:{headers},body}});return {{status:r.status,body:await r.text()}}}}catch(e){{return {{status:0,body:String(e)}}}}}})()",
        chunk = json!(chunk),
        url = json!(url),
        method = json!(method),
    )
}

async fn request(
    method: &str,
    url: &str,
    headers: &Value,
    chunk: Option<&str>,
) -> Result<Value, String> {
    let b = web::ready(P).await?;
    let v = web::eval_for(
        &b,
        P,
        &request_js(method, url, headers, chunk),
        Duration::from_secs(120),
    )
    .await?;
    reply(&v)
}

pub async fn upload(path: &str) -> Result<String, String> {
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|_| format!("MEDIA_NOT_FOUND: {path} is no longer on this computer"))?;
    let mime = rules::image_type(path, bytes.len() as u64).map_err(|r| r.wire())?;
    let mut headers = x::base_headers().await?;
    headers.remove("content-type");
    let headers = Value::Object(headers);
    let id = media_id(&request("POST", &init_url(bytes.len(), mime), &headers, None).await?)?;
    for (i, chunk) in bytes.chunks(SEGMENT).enumerate() {
        let encoded = base64::engine::general_purpose::STANDARD.encode(chunk);
        request("POST", &append_url(&id, i), &headers, Some(&encoded)).await?;
    }
    let mut state = processing(&request("POST", &finalize_url(&id), &headers, None).await?)?;
    for _ in 0..20 {
        let Processing::Wait(secs) = state else {
            return Ok(id);
        };
        tokio::time::sleep(Duration::from_secs(secs)).await;
        state = processing(&request("GET", &status_url(&id), &headers, None).await?)?;
    }
    Err("UPSTREAM_CHANGED: X did not finish processing the image".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_the_three_upload_steps() {
        assert_eq!(
            init_url(1234, "image/jpeg"),
            "https://upload.x.com/i/media/upload.json?command=INIT&total_bytes=1234&media_type=image%2Fjpeg&media_category=tweet_image"
        );
        assert_eq!(
            append_url("77", 2),
            "https://upload.x.com/i/media/upload.json?command=APPEND&media_id=77&segment_index=2"
        );
        assert_eq!(
            finalize_url("77"),
            "https://upload.x.com/i/media/upload.json?command=FINALIZE&media_id=77"
        );
    }

    #[test]
    fn reads_the_media_id() {
        assert_eq!(media_id(&json!({ "media_id": 710511363345354753u64, "media_id_string": "710511363345354753", "expires_after_secs": 86400 })).unwrap(), "710511363345354753");
        assert_eq!(media_id(&json!({ "media_id": 55 })).unwrap(), "55");
        assert!(media_id(&json!({ "errors": [] }))
            .unwrap_err()
            .starts_with("UPSTREAM_CHANGED"));
    }

    #[test]
    fn reads_processing_states() {
        assert_eq!(
            processing(&json!({ "media_id_string": "1", "size": 10 })).unwrap(),
            Processing::Ready
        );
        assert_eq!(
            processing(&json!({ "processing_info": { "state": "succeeded" } })).unwrap(),
            Processing::Ready
        );
        assert_eq!(
            processing(
                &json!({ "processing_info": { "state": "pending", "check_after_secs": 60 } })
            )
            .unwrap(),
            Processing::Wait(10)
        );
        assert!(processing(&json!({ "processing_info": { "state": "failed", "error": { "message": "InvalidMedia" } } })).unwrap_err().starts_with("REJECTED"));
    }

    #[test]
    fn maps_upload_replies() {
        assert_eq!(
            reply(&json!({ "status": 204, "body": "" })).unwrap(),
            Value::Null
        );
        assert_eq!(
            reply(&json!({ "status": 200, "body": "{\"a\":1}" })).unwrap()["a"],
            1
        );
        assert!(reply(&json!({ "status": 200, "body": "<html>" }))
            .unwrap_err()
            .starts_with("UPSTREAM_CHANGED"));
        assert!(reply(&json!({ "status": 401, "body": "" }))
            .unwrap_err()
            .starts_with("NOT_LOGGED_IN"));
        assert!(reply(&json!({ "status": 429, "body": "" }))
            .unwrap_err()
            .starts_with("RATE_LIMITED"));
        assert!(reply(&json!({ "status": 0, "body": "TypeError" }))
            .unwrap_err()
            .starts_with("NETWORK"));
        assert!(reply(&json!({ "status": 400, "body": "bad" }))
            .unwrap_err()
            .starts_with("UPSTREAM_400"));
    }

    #[test]
    fn the_page_script_carries_the_chunk_as_form_data() {
        let js = request_js(
            "POST",
            "https://upload.x.com/x?a=1",
            &json!({ "authorization": "Bearer t" }),
            Some("QUJD"),
        );
        assert!(js.contains("const b=\"QUJD\";"));
        assert!(js.contains("fetch(\"https://upload.x.com/x?a=1\",{method:\"POST\",credentials:'include',headers:{\"authorization\":\"Bearer t\"},body}"));
        assert!(request_js("POST", "u", &json!({}), None).contains("const b=null;"));
    }
}
