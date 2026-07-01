use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::super::api::{check_api_response, ApiClient, BilibiliError, Result};
use super::{fetch_account_info, persist_account};

const QR_GENERATE_URL: &str =
    "https://passport.bilibili.com/x/passport-login/web/qrcode/generate?source=main-fe-header&go_url=https%3A%2F%2Fwww.bilibili.com%2F&web_location=333.1007";
const QR_POLL_URL: &str = "https://passport.bilibili.com/x/passport-login/web/qrcode/poll";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QrSession {
    pub qrcode_key: String,
    pub login_url: String,
    pub qrcode_svg: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum QrPollStatus {
    Pending,
    Scanned,
    Success {
        slug: String,
        uname: String,
        mid: u64,
        is_vip: bool,
    },
    Expired,
    Cancelled,
}

pub async fn generate(client: &ApiClient) -> Result<QrSession> {
    let raw = client.get_json(QR_GENERATE_URL).await?;
    let data = check_api_response(&raw)?;
    let login_url = data
        .get("url")
        .and_then(Value::as_str)
        .ok_or(BilibiliError::ContentUnavailable)?
        .to_string();
    let qrcode_key = data
        .get("qrcode_key")
        .and_then(Value::as_str)
        .ok_or(BilibiliError::ContentUnavailable)?
        .to_string();
    let svg = render_qr_svg(&login_url)?;
    Ok(QrSession {
        qrcode_key,
        login_url,
        qrcode_svg: svg,
    })
}

pub async fn poll(client: &ApiClient, qrcode_key: &str) -> Result<QrPollStatus> {
    let url = format!(
        "{}?qrcode_key={}",
        QR_POLL_URL,
        urlencoding::encode(qrcode_key)
    );
    let raw = client.get_json(&url).await?;
    let data = check_api_response(&raw)?;
    let code = data.get("code").and_then(Value::as_i64).unwrap_or(-1);
    match code {
        86101 => Ok(QrPollStatus::Pending),
        86090 => Ok(QrPollStatus::Scanned),
        86038 => Ok(QrPollStatus::Expired),
        86039 | 86087 => Ok(QrPollStatus::Cancelled),
        0 => {
            let redirect_url = data
                .get("url")
                .and_then(Value::as_str)
                .ok_or(BilibiliError::ContentUnavailable)?;
            let cookies = extract_cookies_from_url(redirect_url);
            if cookies.is_empty() {
                return Err(BilibiliError::ContentUnavailable);
            }
            finalize(client, cookies).await
        }
        _ => Err(BilibiliError::ApiCode {
            code,
            message: data
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        }),
    }
}

async fn finalize(base_client: &ApiClient, cookies: Vec<(String, String)>) -> Result<QrPollStatus> {
    let cookie_header = cookies
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join("; ");

    let probe_client = ApiClient::new()?
        .with_user_agent(base_client.user_agent())
        .with_raw_cookies(cookie_header);

    let info = fetch_account_info(&probe_client).await?;
    let slug = persist_account(&cookies, &info.uname, "QR login")
        .map_err(|_| BilibiliError::ContentUnavailable)?;

    Ok(QrPollStatus::Success {
        slug,
        uname: info.uname,
        mid: info.mid,
        is_vip: info.is_vip,
    })
}

fn extract_cookies_from_url(redirect_url: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    if let Ok(u) = url::Url::parse(redirect_url) {
        for (k, v) in u.query_pairs() {
            let key = k.to_string();
            if matches!(
                key.as_str(),
                "SESSDATA" | "bili_jct" | "DedeUserID" | "DedeUserID__ckMd5" | "sid"
            ) {
                out.push((key, v.into_owned()));
            }
        }
    }
    out
}

fn render_qr_svg(content: &str) -> Result<String> {
    use qrcode::render::svg;
    use qrcode::QrCode;

    let code = QrCode::new(content.as_bytes()).map_err(|_| BilibiliError::ContentUnavailable)?;
    let svg = code
        .render::<svg::Color>()
        .min_dimensions(240, 240)
        .quiet_zone(true)
        .build();
    Ok(svg)
}
