//! Tripo (VAST-AI) OpenAPI v2 with the user's key: upload, create task,
//! poll, download the GLB. Estudo 05: H3.1 image-to-3D at `face_limit`
//! 10 000 was the cheapest good result we measured.

use super::async_trait_free::BoxFuture;
use super::{GenerateRequest, Generated, Generator, Quote};
use serde_json::{json, Value};
use std::time::Duration;

pub const BASE: &str = "https://api.tripo3d.ai/v2/openapi";

pub struct Tripo {
    pub api_key: String,
    pub base: String,
    pub client: reqwest::Client,
    /// Give up polling after this long.
    pub timeout: Duration,
    /// Largest GLB accepted from the download URL.
    pub max_download: usize,
    /// Wait between task status polls.
    pub poll_every: Duration,
}

impl Tripo {
    pub fn new(api_key: &str) -> Self {
        Tripo {
            api_key: api_key.trim().to_string(),
            base: BASE.into(),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(120))
                .build()
                .unwrap_or_default(),
            timeout: Duration::from_secs(15 * 60),
            max_download: 64 << 20,
            poll_every: Duration::from_secs(4),
        }
    }

    fn check(v: Value) -> Result<Value, String> {
        if v.get("code").and_then(Value::as_i64) == Some(0) {
            Ok(v.get("data").cloned().unwrap_or(Value::Null))
        } else {
            Err(format!(
                "Tripo: {}",
                v.get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("request failed")
            ))
        }
    }

    pub async fn balance(&self) -> Result<f64, String> {
        let r = self
            .client
            .get(format!("{}/user/balance", self.base))
            .bearer_auth(&self.api_key)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let v: Value = r.json().await.map_err(|e| e.to_string())?;
        let d = Self::check(v)?;
        Ok(d.get("balance").and_then(Value::as_f64).unwrap_or(0.0))
    }

    async fn upload(&self, image: Vec<u8>) -> Result<(String, &'static str), String> {
        let kind = match image.get(0..4) {
            Some([0x89, b'P', b'N', b'G']) => "png",
            Some([0xFF, 0xD8, ..]) => "jpg",
            _ if image.get(8..12) == Some(b"WEBP") => "webp",
            _ => return Err("the image must be PNG, JPEG or WebP".into()),
        };
        let part = reqwest::multipart::Part::bytes(image)
            .file_name(format!("image.{kind}"))
            .mime_str(match kind {
                "png" => "image/png",
                "jpg" => "image/jpeg",
                _ => "image/webp",
            })
            .map_err(|e| e.to_string())?;
        let form = reqwest::multipart::Form::new().part("file", part);
        let r = self
            .client
            .post(format!("{}/upload", self.base))
            .bearer_auth(&self.api_key)
            .multipart(form)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let d = Self::check(r.json().await.map_err(|e| e.to_string())?)?;
        let token = d
            .get("image_token")
            .and_then(Value::as_str)
            .ok_or("Tripo: upload returned no token")?;
        Ok((token.to_string(), kind))
    }

    pub async fn run(&self, req: GenerateRequest) -> Result<Generated, String> {
        if self.api_key.is_empty() {
            return Err("Tripo is not configured (no API key)".into());
        }
        let mut body = match (&req.image, &req.prompt) {
            (Some(img), _) => {
                let (token, kind) = self.upload(img.clone()).await?;
                json!({"type": "image_to_model", "file": {"type": kind, "file_token": token}})
            }
            (None, Some(p)) if !p.trim().is_empty() => {
                json!({"type": "text_to_model", "prompt": p})
            }
            _ => return Err("give a prompt or an image".into()),
        };
        body["face_limit"] = json!(req.face_limit.unwrap_or(10_000).clamp(500, 100_000));
        body["texture"] = json!(true);
        body["pbr"] = json!(false);
        let r = self
            .client
            .post(format!("{}/task", self.base))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let d = Self::check(r.json().await.map_err(|e| e.to_string())?)?;
        let task = d
            .get("task_id")
            .and_then(Value::as_str)
            .ok_or("Tripo: no task id")?
            .to_string();

        let started = std::time::Instant::now();
        let url = loop {
            if started.elapsed() > self.timeout {
                return Err(format!("Tripo: task {task} did not finish in time"));
            }
            tokio::time::sleep(self.poll_every).await;
            let r = self
                .client
                .get(format!("{}/task/{}", self.base, task))
                .bearer_auth(&self.api_key)
                .send()
                .await
                .map_err(|e| e.to_string())?;
            let d = Self::check(r.json().await.map_err(|e| e.to_string())?)?;
            match d.get("status").and_then(Value::as_str).unwrap_or("") {
                "success" => {
                    let out = d.get("output").cloned().unwrap_or(Value::Null);
                    let u = ["pbr_model", "model", "base_model"]
                        .iter()
                        .find_map(|k| out.get(*k).and_then(Value::as_str))
                        .ok_or("Tripo: finished without a model URL")?;
                    break u.to_string();
                }
                "failed" | "cancelled" | "banned" | "expired" | "unknown" => {
                    return Err(format!("Tripo: task {task} ended as {}", d["status"]));
                }
                _ => {}
            }
        };
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if let Some(len) = resp.content_length() {
            if len as usize > self.max_download {
                return Err("Tripo: the model is larger than the import allows".into());
            }
        }
        let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
        if bytes.len() > self.max_download {
            return Err("Tripo: the model is larger than the import allows".into());
        }
        Ok(Generated {
            glb: bytes.to_vec(),
            provider: "tripo".into(),
            model: "tripo-default".into(),
            task_id: task,
        })
    }
}

impl Generator for Tripo {
    fn id(&self) -> &'static str {
        "tripo"
    }

    fn quote(&self) -> BoxFuture<'_, Result<Quote, String>> {
        Box::pin(async move {
            let balance = self.balance().await.ok();
            Ok(Quote {
                provider: "tripo".into(),
                // Tripo API price list read in estudo 05: image-to-model
                // 20–50 credits with texture (US$ 0.01 per credit).
                cost: 30.0,
                balance,
                note: "image or text to a textured model; credits are charged by Tripo".into(),
            })
        })
    }

    fn generate(&self, req: GenerateRequest) -> BoxFuture<'_, Result<Generated, String>> {
        Box::pin(self.run(req))
    }
}
