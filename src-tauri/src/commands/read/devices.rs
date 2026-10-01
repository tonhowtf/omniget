use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{blocking, db};

#[derive(Debug, Clone, Serialize)]
pub struct Device {
    pub kind: String,
    pub name: String,
    pub mount: String,
    pub books_dir: String,
    pub formats: Vec<String>,
    pub free_bytes: Option<u64>,
}

fn candidates() -> Vec<PathBuf> {
    let mut v = Vec::new();
    #[cfg(target_os = "macos")]
    if let Ok(rd) = std::fs::read_dir("/Volumes") {
        v.extend(rd.flatten().map(|e| e.path()));
    }
    #[cfg(target_os = "linux")]
    {
        let user = std::env::var("USER").unwrap_or_default();
        for base in [
            format!("/media/{user}"),
            format!("/run/media/{user}"),
            "/mnt".into(),
        ] {
            if let Ok(rd) = std::fs::read_dir(&base) {
                v.extend(rd.flatten().map(|e| e.path()));
            }
        }
    }
    #[cfg(target_os = "windows")]
    for l in b'D'..=b'Z' {
        let p = PathBuf::from(format!("{}:\\", l as char));
        if p.exists() {
            v.push(p);
        }
    }
    v
}

fn free_space(p: &Path) -> Option<u64> {
    fs4::available_space(p).ok()
}

pub fn detect() -> Vec<Device> {
    let mut out = Vec::new();
    for m in candidates() {
        let mount = m.to_string_lossy().to_string();
        let name = m
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| mount.clone());
        let dev = if m.join("system").join("thumbnails").is_dir()
            || (m.join("documents").is_dir() && name.to_lowercase().contains("kindle"))
        {
            Some(("kindle", "documents", vec!["azw3", "mobi", "pdf", "txt"]))
        } else if m.join(".kobo").is_dir() {
            Some(("kobo", "", vec!["kepub", "epub", "pdf", "cbz"]))
        } else if m.join("system").join("config").join("books.db").exists()
            || name.to_lowercase().contains("pocketbook")
        {
            Some(("pocketbook", "", vec!["epub", "fb2", "pdf", "cbz", "mobi"]))
        } else if m.join(".adds").is_dir() || m.join("koreader").is_dir() {
            Some(("koreader", "", vec!["epub", "cbz", "pdf", "fb2", "mobi"]))
        } else if m.join("Digital Editions").is_dir() {
            Some(("ereader", "", vec!["epub", "pdf"]))
        } else {
            None
        };
        if let Some((kind, sub, formats)) = dev {
            let books_dir = if sub.is_empty() {
                m.clone()
            } else {
                m.join(sub)
            };
            out.push(Device {
                kind: kind.into(),
                name,
                mount: mount.clone(),
                books_dir: books_dir.to_string_lossy().to_string(),
                formats: formats.into_iter().map(str::to_string).collect(),
                free_bytes: free_space(&m),
            });
        }
    }
    out
}

#[tauri::command]
pub async fn read_devices_detect() -> Result<Vec<Device>, String> {
    blocking(|| Ok(detect())).await
}

#[tauri::command]
pub async fn read_send_to_device(
    book_ids: Vec<i64>,
    mount: String,
    subfolder: Option<String>,
) -> Result<Vec<String>, String> {
    blocking(move || {
        let dev = detect()
            .into_iter()
            .find(|d| d.mount == mount)
            .ok_or("device not found")?;
        let mut dest = PathBuf::from(&dev.books_dir);
        if let Some(s) = subfolder.filter(|s| !s.trim().is_empty()) {
            dest = dest.join(sanitize_filename::sanitize(s.trim()));
        }
        std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
        let mut skipped = Vec::new();
        for id in book_ids {
            let b = db::book(id)?;
            let fmt = if b.path.to_lowercase().ends_with(".kepub.epub") {
                "kepub".to_string()
            } else {
                b.format.clone()
            };
            let ok = dev.formats.contains(&fmt)
                || (fmt == "kepub" && dev.formats.iter().any(|f| f == "epub"));
            if !ok {
                skipped.push(b.title.clone());
                continue;
            }
            let src = Path::new(&b.path);
            let name = src.file_name().ok_or("bad path")?;
            std::fs::copy(src, dest.join(name)).map_err(|e| format!("{}: {e}", b.title))?;
        }
        Ok(skipped)
    })
    .await
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct MailConfig {
    pub host: String,
    pub port: u16,
    pub security: String,
    pub username: String,
    pub password: String,
    pub from: String,
    pub to: String,
}

pub fn mail_config() -> MailConfig {
    let mut c: MailConfig = db::kv_get("mail").unwrap_or_default();
    c.password = super::secret("mail");
    c
}

#[tauri::command]
pub async fn read_mail_config(set: Option<MailConfig>) -> Result<MailConfig, String> {
    blocking(move || {
        if let Some(mut c) = set {
            if c.password != "••••••" {
                super::set_secret("mail", &c.password)?;
            }
            c.password = String::new();
            db::kv_set("mail", &c)?;
        }
        let mut c = mail_config();
        if !c.password.is_empty() {
            c.password = "••••••".into();
        }
        Ok(c)
    })
    .await
}

#[tauri::command]
pub async fn read_send_email(book_ids: Vec<i64>, to: Option<String>) -> Result<usize, String> {
    use lettre::message::{header::ContentType, Attachment, MultiPart, SinglePart};
    use lettre::transport::smtp::authentication::Credentials;
    use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

    let cfg = blocking(|| Ok(mail_config())).await?;
    if cfg.host.is_empty() || cfg.from.is_empty() {
        return Err("set up the e-mail account first".into());
    }
    let to = to
        .filter(|t| !t.trim().is_empty())
        .unwrap_or(cfg.to.clone());
    if to.is_empty() {
        return Err("no recipient".into());
    }
    let builder = match cfg.security.as_str() {
        "tls" => AsyncSmtpTransport::<Tokio1Executor>::relay(&cfg.host),
        "none" => Ok(AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(
            &cfg.host,
        )),
        _ => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&cfg.host),
    }
    .map_err(|e| e.to_string())?;
    let mut builder = builder.port(if cfg.port == 0 {
        if cfg.security == "tls" {
            465
        } else {
            587
        }
    } else {
        cfg.port
    });
    if !cfg.username.is_empty() {
        builder = builder.credentials(Credentials::new(cfg.username.clone(), cfg.password.clone()));
    }
    let mailer = builder.build();
    let mut sent = 0;
    for id in book_ids {
        let b = blocking(move || db::book(id)).await?;
        let bytes = tokio::fs::read(&b.path).await.map_err(|e| e.to_string())?;
        if bytes.len() > 50 * 1024 * 1024 {
            return Err(format!("{} is over 50 MB", b.title));
        }
        let name = Path::new(&b.path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "book".into());
        let mime = omniget_core::core::read::format::BookFormat::from_path(Path::new(&b.path))
            .map(|f| f.mime())
            .unwrap_or("application/octet-stream");
        let ct = ContentType::parse(mime).unwrap_or(ContentType::TEXT_PLAIN);
        let msg = Message::builder()
            .from(cfg.from.parse().map_err(|e| format!("from: {e}"))?)
            .to(to.parse().map_err(|e| format!("to: {e}"))?)
            .subject(&b.title)
            .multipart(
                MultiPart::mixed()
                    .singlepart(SinglePart::plain(format!(
                        "{} — sent from OmniGet",
                        b.title
                    )))
                    .singlepart(Attachment::new(name).body(bytes, ct)),
            )
            .map_err(|e| e.to_string())?;
        mailer.send(msg).await.map_err(|e| e.to_string())?;
        sent += 1;
    }
    Ok(sent)
}
