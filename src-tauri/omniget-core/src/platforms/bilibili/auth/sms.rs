use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::super::api::{check_api_response, ApiClient, BilibiliError, Result};
use super::{fetch_account_info, persist_account};

const SMS_SEND_URL: &str = "https://passport.bilibili.com/x/passport-login/web/sms/send";
const SMS_LOGIN_URL: &str = "https://passport.bilibili.com/x/passport-login/web/login/sms";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmsSendInput {
    pub cid: String,
    pub tel: String,
    pub captcha_token: String,
    pub geetest_challenge: String,
    pub geetest_validate: String,
    pub geetest_seccode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmsSendResult {
    pub captcha_key: String,
    pub recaptcha_url: Option<String>,
}

pub async fn send(client: &ApiClient, input: SmsSendInput) -> Result<SmsSendResult> {
    let form = [
        ("cid", input.cid.as_str()),
        ("tel", input.tel.as_str()),
        ("source", "main-fe-header"),
        ("token", input.captcha_token.as_str()),
        ("challenge", input.geetest_challenge.as_str()),
        ("validate", input.geetest_validate.as_str()),
        ("seccode", input.geetest_seccode.as_str()),
    ];
    let raw = client.post_form(SMS_SEND_URL, &form).await?;
    let data = check_api_response(&raw)?;
    let captcha_key = data
        .get("captcha_key")
        .and_then(Value::as_str)
        .ok_or(BilibiliError::ContentUnavailable)?
        .to_string();
    let recaptcha = data
        .get("recaptcha_url")
        .and_then(Value::as_str)
        .map(String::from);
    Ok(SmsSendResult {
        captcha_key,
        recaptcha_url: recaptcha,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmsVerifyInput {
    pub cid: String,
    pub tel: String,
    pub code: String,
    pub captcha_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmsVerifyResult {
    pub slug: String,
    pub uname: String,
    pub mid: u64,
    pub is_vip: bool,
}

pub async fn verify(client: &ApiClient, input: SmsVerifyInput) -> Result<SmsVerifyResult> {
    let form = [
        ("cid", input.cid.as_str()),
        ("tel", input.tel.as_str()),
        ("code", input.code.as_str()),
        ("source", "main-fe-header"),
        ("captcha_key", input.captcha_key.as_str()),
        ("go_url", "https://www.bilibili.com/"),
        ("keep", "true"),
    ];
    let raw = client.post_form(SMS_LOGIN_URL, &form).await?;
    let data = check_api_response(&raw)?;
    let status = data.get("status").and_then(Value::as_i64).unwrap_or(0);
    if status != 0 {
        return Err(BilibiliError::ApiCode {
            code: status,
            message: data
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        });
    }

    let cookies = extract_cookies(data);
    if cookies.is_empty() {
        return Err(BilibiliError::ContentUnavailable);
    }
    let cookie_header = cookies
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join("; ");
    let probe_client = ApiClient::new()?
        .with_user_agent(client.user_agent())
        .with_raw_cookies(cookie_header);
    let info = fetch_account_info(&probe_client).await?;
    let slug = persist_account(&cookies, &info.uname, "SMS login")
        .map_err(|_| BilibiliError::ContentUnavailable)?;
    Ok(SmsVerifyResult {
        slug,
        uname: info.uname,
        mid: info.mid,
        is_vip: info.is_vip,
    })
}

fn extract_cookies(data: &Value) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    if let Some(arr) = data
        .get("cookie_info")
        .and_then(|v| v.get("cookies"))
        .and_then(Value::as_array)
    {
        for c in arr {
            if let (Some(name), Some(value)) = (
                c.get("name").and_then(Value::as_str),
                c.get("value").and_then(Value::as_str),
            ) {
                out.push((name.to_string(), value.to_string()));
            }
        }
    }
    if let Some(url) = data.get("url").and_then(Value::as_str) {
        if let Ok(u) = url::Url::parse(url) {
            for (k, v) in u.query_pairs() {
                if matches!(
                    k.as_ref(),
                    "SESSDATA" | "bili_jct" | "DedeUserID" | "DedeUserID__ckMd5" | "sid"
                ) {
                    out.push((k.to_string(), v.into_owned()));
                }
            }
        }
    }
    out
}
