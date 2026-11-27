use std::io::{Read, Seek, SeekFrom};

use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};

pub fn partial_md5<R: Read + Seek>(r: &mut R) -> std::io::Result<String> {
    let mut h = Md5::new();
    let mut buf = vec![0u8; 1024];
    for i in -1i32..=10 {
        let off: u64 = if i < 0 { 1024 >> 2 } else { 1024u64 << (2 * i) };
        r.seek(SeekFrom::Start(off))?;
        let mut n = 0;
        while n < buf.len() {
            let got = r.read(&mut buf[n..])?;
            if got == 0 {
                break;
            }
            n += got;
        }
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

pub fn partial_md5_path(p: &std::path::Path) -> Option<String> {
    let mut f = std::fs::File::open(p).ok()?;
    partial_md5(&mut f).ok()
}

pub fn same_contents(a: &std::path::Path, b: &std::path::Path) -> bool {
    use std::io::Read;
    let (Ok(ma), Ok(mb)) = (std::fs::metadata(a), std::fs::metadata(b)) else {
        return false;
    };
    if ma.len() != mb.len() {
        return false;
    }
    let (Ok(fa), Ok(fb)) = (std::fs::File::open(a), std::fs::File::open(b)) else {
        return false;
    };
    let mut ra = std::io::BufReader::with_capacity(64 * 1024, fa);
    let mut rb = std::io::BufReader::with_capacity(64 * 1024, fb);
    let mut ba = [0u8; 64 * 1024];
    let mut bb = [0u8; 64 * 1024];
    loop {
        let Ok(n) = ra.read(&mut ba) else {
            return false;
        };
        if n == 0 {
            return true;
        }
        if rb.read_exact(&mut bb[..n]).is_err() || ba[..n] != bb[..n] {
            return false;
        }
    }
}

pub fn user_key(password: &str) -> String {
    hex::encode(Md5::digest(password.as_bytes()))
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct Progress {
    pub document: String,
    pub progress: String,
    pub percentage: f64,
    pub device: String,
    pub device_id: String,
    #[serde(default)]
    pub timestamp: Option<i64>,
}

pub struct Client {
    pub base: String,
    pub user: String,
    pub key: String,
    http: reqwest::Client,
}

impl Client {
    pub fn new(base: &str, user: &str, password_or_key: &str, is_key: bool) -> Self {
        Self {
            base: base.trim_end_matches('/').to_string(),
            user: user.to_string(),
            key: if is_key {
                password_or_key.to_string()
            } else {
                user_key(password_or_key)
            },
            http: super::sources::client(),
        }
    }

    fn req(&self, m: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.http
            .request(m, format!("{}{path}", self.base))
            .header("x-auth-user", &self.user)
            .header("x-auth-key", &self.key)
            .header("accept", "application/vnd.koreader.v1+json")
    }

    pub async fn register(&self) -> Result<(), String> {
        let r = self
            .http
            .post(format!("{}/users/create", self.base))
            .header("accept", "application/vnd.koreader.v1+json")
            .json(&serde_json::json!({ "username": self.user, "password": self.key }))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if r.status().is_success() {
            Ok(())
        } else {
            Err(format!("register failed: {}", r.status()))
        }
    }

    pub async fn auth(&self) -> Result<(), String> {
        let r = self
            .req(reqwest::Method::GET, "/users/auth")
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if r.status().is_success() {
            Ok(())
        } else {
            Err(format!("login failed: {}", r.status()))
        }
    }

    pub async fn push(&self, p: &Progress) -> Result<(), String> {
        let r = self
            .req(reqwest::Method::PUT, "/syncs/progress")
            .json(p)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if r.status().is_success() {
            Ok(())
        } else {
            Err(format!("push failed: {}", r.status()))
        }
    }

    pub async fn pull(&self, document: &str) -> Result<Option<Progress>, String> {
        let r = self
            .req(reqwest::Method::GET, &format!("/syncs/progress/{document}"))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !r.status().is_success() {
            return Err(format!("pull failed: {}", r.status()));
        }
        let v: serde_json::Value = r.json().await.map_err(|e| e.to_string())?;
        if v.get("percentage").is_none() {
            return Ok(None);
        }
        Ok(serde_json::from_value(v).ok())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn same_contents_needs_every_byte_to_match() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = dir.path().join("a.pdf");
        let b = dir.path().join("b.pdf");
        let c = dir.path().join("c.pdf");
        let mut body = vec![7u8; 300_000];
        std::fs::write(&a, &body).expect("write a");
        std::fs::write(&b, &body).expect("write b");
        body[150_000] = 8;
        std::fs::write(&c, &body).expect("write c");
        assert_eq!(partial_md5_path(&a), partial_md5_path(&c));
        assert!(super::same_contents(&a, &b));
        assert!(!super::same_contents(&a, &c));
        assert!(!super::same_contents(&a, &dir.path().join("missing.pdf")));
    }

    use super::*;

    #[test]
    fn partial_md5_small_and_empty() {
        let mut c = std::io::Cursor::new(vec![7u8; 5000]);
        let a = partial_md5(&mut c).unwrap();
        let mut h = Md5::new();
        h.update(vec![7u8; 1024]);
        h.update(vec![7u8; 1024]);
        h.update(vec![7u8; 904]);
        assert_eq!(a, hex::encode(h.finalize()));
        let mut e = std::io::Cursor::new(Vec::<u8>::new());
        assert_eq!(
            partial_md5(&mut e).unwrap(),
            hex::encode(Md5::new().finalize())
        );
    }

    #[test]
    fn key_is_md5() {
        assert_eq!(user_key("abc"), "900150983cd24fb0d6963f7d28e17f72");
    }
}
