use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::super::api::{check_api_response, ApiClient, BilibiliError, Result};

const CAPTCHA_URL: &str = "https://passport.bilibili.com/x/passport-login/captcha?source=main_h5";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptchaChallenge {
    pub token: String,
    pub geetest_gt: String,
    pub geetest_challenge: String,
}

pub async fn request_challenge(client: &ApiClient) -> Result<CaptchaChallenge> {
    let raw = client.get_json(CAPTCHA_URL).await?;
    let data = check_api_response(&raw)?;
    let token = data
        .get("token")
        .and_then(Value::as_str)
        .ok_or(BilibiliError::ContentUnavailable)?
        .to_string();
    let geetest = data
        .get("geetest")
        .ok_or(BilibiliError::ContentUnavailable)?;
    let gt = geetest
        .get("gt")
        .and_then(Value::as_str)
        .ok_or(BilibiliError::ContentUnavailable)?
        .to_string();
    let challenge = geetest
        .get("challenge")
        .and_then(Value::as_str)
        .ok_or(BilibiliError::ContentUnavailable)?
        .to_string();
    Ok(CaptchaChallenge {
        token,
        geetest_gt: gt,
        geetest_challenge: challenge,
    })
}
