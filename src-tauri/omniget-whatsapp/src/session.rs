use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use anyhow::{anyhow, bail, Context, Result};
use chrono::{FixedOffset, Offset};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::Mutex;
use whatsapp_rust::pair_code::PairCodeOptions;
use whatsapp_rust::prelude::*;
use whatsapp_rust::wacore::download::Downloadable;
use whatsapp_rust::wacore::store::DevicePropsOverride;

use crate::export::{self, ExportFormat, ExportLabels, ExportMessage};
use crate::model::{self, MediaKind, MessageKind};
use crate::naming::{self, NameInput};
use crate::store::{self, ChatRow, MediaRow, MediaStats, MessageRow, NewMessage, Store};

pub trait Host: Send + Sync + 'static {
    fn emit(&self, event: &str, payload: Value);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Idle,
    Starting,
    Pairing,
    Connected,
    Offline,
    LoggedOut,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Account {
    pub jid: String,
    pub push_name: Option<String>,
    pub platform: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Status {
    pub state: State,
    pub linked: bool,
    pub account: Option<Account>,
    pub qr_svg: Option<String>,
    pub qr_timeout_secs: Option<u64>,
    pub pair_code: Option<String>,
    pub error: Option<String>,
    pub sync_progress: Option<u32>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SavedFile {
    pub message_id: String,
    pub path: String,
    pub skipped: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct DownloadJob {
    pub job_id: String,
    pub total: usize,
    pub dir: String,
}

struct Running {
    client: Arc<Client>,
    handle: Option<BotHandle>,
}

struct Inner {
    state: State,
    qr_svg: Option<String>,
    qr_timeout_secs: Option<u64>,
    pair_code: Option<String>,
    error: Option<String>,
    sync_progress: Option<u32>,
}

pub struct WhatsApp {
    dir: PathBuf,
    host: Arc<dyn Host>,
    store: Arc<Store>,
    running: Mutex<Option<Running>>,
    inner: StdMutex<Inner>,
    jobs: StdMutex<HashMap<String, Arc<AtomicBool>>>,
    job_seq: AtomicU64,
    logging_out: AtomicBool,
}

const SESSION_FILE: &str = "session.db";
const ACCOUNT_FILE: &str = "account.json";
const WIPE_MARKER: &str = "wipe-pending";

fn qr_svg(code: &str) -> Option<String> {
    let qr = qrcode::QrCode::new(code.as_bytes()).ok()?;
    Some(
        qr.render::<qrcode::render::svg::Color<'_>>()
            .min_dimensions(240, 240)
            .quiet_zone(true)
            .dark_color(qrcode::render::svg::Color("#111111"))
            .light_color(qrcode::render::svg::Color("#ffffff"))
            .build(),
    )
}

pub fn local_offset() -> FixedOffset {
    chrono::Local::now().offset().fix()
}

pub fn normalize_phone(raw: &str) -> Result<String> {
    let digits: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
    let digits = digits.trim_start_matches('0').to_string();
    if digits.len() < 8 || digits.len() > 15 {
        bail!("invalid_phone");
    }
    Ok(digits)
}

fn media_of(msg: &wa::Message) -> Option<&dyn Downloadable> {
    let base = msg.get_base_message();
    if let Some(m) = base.image_message.as_option() {
        return Some(m);
    }
    if let Some(m) = base.video_message.as_option() {
        return Some(m);
    }
    if let Some(m) = base.ptv_message.as_option() {
        return Some(m);
    }
    if let Some(m) = base.audio_message.as_option() {
        return Some(m);
    }
    if let Some(m) = base.document_message.as_option() {
        return Some(m);
    }
    if let Some(m) = base.sticker_message.as_option() {
        return Some(m);
    }
    None
}

fn base_mut(msg: &mut wa::Message) -> &mut wa::Message {
    let mut current = msg;
    if current
        .device_sent_message
        .as_option()
        .is_some_and(|d| d.message.is_set())
    {
        current = current
            .device_sent_message
            .as_option_mut()
            .and_then(|d| d.message.as_option_mut())
            .expect("checked above");
    }
    macro_rules! peel {
        ($field:ident) => {
            if current
                .$field
                .as_option()
                .is_some_and(|w| w.message.is_set())
            {
                current = current
                    .$field
                    .as_option_mut()
                    .and_then(|w| w.message.as_option_mut())
                    .expect("checked above");
            }
        };
    }
    peel!(ephemeral_message);
    peel!(view_once_message);
    peel!(view_once_message_v2);
    peel!(view_once_message_v2_extension);
    peel!(document_with_caption_message);
    peel!(edited_message);
    current
}

pub fn media_key_of(msg: &wa::Message) -> Option<Vec<u8>> {
    media_of(msg)
        .and_then(|m| m.media_key())
        .filter(|k| !k.is_empty())
        .map(<[u8]>::to_vec)
}

pub fn set_direct_path(msg: &mut wa::Message, path: &str) -> bool {
    let base = base_mut(msg);
    let path = Some(path.to_string());
    if let Some(m) = base.image_message.as_option_mut() {
        m.direct_path = path;
        return true;
    }
    if let Some(m) = base.video_message.as_option_mut() {
        m.direct_path = path;
        return true;
    }
    if let Some(m) = base.ptv_message.as_option_mut() {
        m.direct_path = path;
        return true;
    }
    if let Some(m) = base.audio_message.as_option_mut() {
        m.direct_path = path;
        return true;
    }
    if let Some(m) = base.document_message.as_option_mut() {
        m.direct_path = path;
        return true;
    }
    if let Some(m) = base.sticker_message.as_option_mut() {
        m.direct_path = path;
        return true;
    }
    false
}

fn is_expired(e: &anyhow::Error) -> bool {
    use whatsapp_rust::ErrorChainExt;
    let chain: &(dyn std::error::Error + Send + Sync + 'static) = e.as_ref();
    if matches!(chain.http_status(), Some(404 | 410)) {
        return true;
    }
    friendly_download_error(e) == "media_expired"
}

const MAX_READ_RECEIPTS: u32 = 300;

pub type ReceiptGroup = ((String, Option<String>), Vec<String>);

pub fn group_receipts(targets: &[store::ReadTarget]) -> Vec<ReceiptGroup> {
    let mut out: Vec<ReceiptGroup> = Vec::new();
    for t in targets {
        let sender = if store::is_group(&t.wire_chat) {
            t.wire_sender.clone()
        } else {
            None
        };
        let key = (t.wire_chat.clone(), sender);
        match out.iter_mut().find(|(k, _)| *k == key) {
            Some((_, ids)) => ids.push(t.id.clone()),
            None => out.push((key, vec![t.id.clone()])),
        }
    }
    out
}

const RETRY_WAIT: std::time::Duration = std::time::Duration::from_secs(45);

fn friendly_download_error(e: &anyhow::Error) -> String {
    let s = format!("{e:#}");
    let low = s.to_lowercase();
    if low.contains("404") || low.contains("410") || low.contains("status 4") {
        "media_expired".into()
    } else if low.contains("not connected") || low.contains("not logged") {
        "offline".into()
    } else {
        s
    }
}

async fn fetch_to(client: &Arc<Client>, msg: &wa::Message, part: &Path) -> Result<()> {
    let media = media_of(msg).ok_or_else(|| anyhow!("no_media"))?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .read(true)
        .write(true)
        .open(part)?;
    let f = client.download_to_writer(media, file).await?;
    drop(f);
    Ok(())
}

impl WhatsApp {
    pub fn new(dir: PathBuf, host: Arc<dyn Host>) -> Result<Arc<Self>> {
        std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
        let store = Store::open(&dir.join("cache.db")).context("open WhatsApp cache")?;
        let me = Arc::new(Self {
            dir,
            host,
            store: Arc::new(store),
            running: Mutex::new(None),
            inner: StdMutex::new(Inner {
                state: State::Idle,
                qr_svg: None,
                qr_timeout_secs: None,
                pair_code: None,
                error: None,
                sync_progress: None,
            }),
            jobs: StdMutex::new(HashMap::new()),
            job_seq: AtomicU64::new(1),
            logging_out: AtomicBool::new(false),
        });
        me.finish_pending_wipe();
        Ok(me)
    }

    fn inner(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn account(&self) -> Option<Account> {
        let raw = std::fs::read(self.dir.join(ACCOUNT_FILE)).ok()?;
        serde_json::from_slice(&raw).ok()
    }

    fn save_account(&self, account: &Account) {
        if let Ok(raw) = serde_json::to_vec_pretty(account) {
            let _ = std::fs::write(self.dir.join(ACCOUNT_FILE), raw);
        }
    }

    pub fn status(&self) -> Status {
        let account = self.account();
        let i = self.inner();
        Status {
            state: i.state,
            linked: account.is_some(),
            account,
            qr_svg: i.qr_svg.clone(),
            qr_timeout_secs: i.qr_timeout_secs,
            pair_code: i.pair_code.clone(),
            error: i.error.clone(),
            sync_progress: i.sync_progress,
        }
    }

    fn update(&self, f: impl FnOnce(&mut Inner)) {
        {
            let mut i = self.inner();
            f(&mut i);
        }
        self.host.emit("whatsapp-status", json!(self.status()));
    }

    fn session_path(&self) -> PathBuf {
        self.dir.join(SESSION_FILE)
    }

    fn remove_session_files(&self) -> bool {
        let mut ok = true;
        for suffix in ["", "-wal", "-shm", "-journal"] {
            let p = PathBuf::from(format!("{}{suffix}", self.session_path().display()));
            if p.exists() && std::fs::remove_file(&p).is_err() {
                ok = false;
            }
        }
        let _ = std::fs::remove_file(self.dir.join(ACCOUNT_FILE));
        ok
    }

    fn finish_pending_wipe(&self) {
        let marker = self.dir.join(WIPE_MARKER);
        if marker.exists() && self.remove_session_files() {
            let _ = std::fs::remove_file(marker);
        }
    }

    async fn client(&self) -> Result<Arc<Client>> {
        let guard = self.running.lock().await;
        match guard.as_ref() {
            Some(r) => Ok(r.client.clone()),
            None => bail!("not_running"),
        }
    }

    async fn connected_client(&self) -> Result<Arc<Client>> {
        let client = self.client().await?;
        if !client.is_connected() || !client.is_logged_in() {
            bail!("offline");
        }
        Ok(client)
    }

    pub async fn connect(self: &Arc<Self>) -> Result<Status> {
        let mut guard = self.running.lock().await;
        if guard.is_some() {
            return Ok(self.status());
        }
        self.finish_pending_wipe();
        self.update(|i| {
            i.state = State::Starting;
            i.error = None;
            i.qr_svg = None;
            i.pair_code = None;
        });
        let path = self.session_path();
        let backend = match SqliteStore::new(&path.to_string_lossy()).await {
            Ok(b) => b,
            Err(e) => {
                let msg = format!("{e}");
                self.update(|i| {
                    i.state = State::Idle;
                    i.error = Some(msg.clone());
                });
                bail!(msg);
            }
        };
        let me = Arc::downgrade(self);
        let props = DevicePropsOverride::new()
            .with_os("OmniGet")
            .with_platform_type(wa::device_props::PlatformType::DESKTOP)
            .with_require_full_sync(true);
        let built = Bot::builder()
            .with_backend(backend)
            .with_device_props(props)
            .on_event(move |event, client| {
                let me = me.clone();
                async move {
                    if let Some(me) = me.upgrade() {
                        me.on_event(event, client).await;
                    }
                }
            })
            .build()
            .await;
        let bot = match built {
            Ok(b) => b,
            Err(e) => {
                let msg = format!("{e}");
                self.update(|i| {
                    i.state = State::Idle;
                    i.error = Some(msg.clone());
                });
                bail!(msg);
            }
        };
        let client = bot.client();
        let handle = bot.spawn();
        *guard = Some(Running {
            client,
            handle: Some(handle),
        });
        drop(guard);
        Ok(self.status())
    }

    pub async fn disconnect(&self) -> Result<Status> {
        let running = self.running.lock().await.take();
        if let Some(mut r) = running {
            if let Some(h) = r.handle.take() {
                h.shutdown().await;
            }
        }
        self.update(|i| {
            i.state = State::Idle;
            i.qr_svg = None;
            i.pair_code = None;
            i.sync_progress = None;
        });
        Ok(self.status())
    }

    pub async fn pair_phone(self: &Arc<Self>, phone: &str) -> Result<String> {
        let phone = normalize_phone(phone)?;
        if self.running.lock().await.is_none() {
            self.connect().await?;
        }
        let client = self.client().await?;
        for _ in 0..40 {
            if client.is_connected() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
        if !client.is_connected() {
            bail!("offline");
        }
        client.cancel_pair_code().await;
        let code = client
            .pair_with_code(PairCodeOptions {
                phone_number: phone,
                ..Default::default()
            })
            .await
            .map_err(|e| anyhow!("{e}"))?;
        self.update(|i| {
            i.state = State::Pairing;
            i.pair_code = Some(code.clone());
            i.error = None;
        });
        Ok(code)
    }

    pub async fn logout(&self) -> Result<Status> {
        self.logging_out.store(true, Ordering::SeqCst);
        let running = self.running.lock().await.take();
        if let Some(mut r) = running {
            r.client.logout().await;
            if let Some(h) = r.handle.take() {
                h.shutdown().await;
            }
            drop(r);
        }
        self.wipe_local();
        self.update(|i| {
            i.state = State::Idle;
            i.qr_svg = None;
            i.pair_code = None;
            i.sync_progress = None;
            i.error = None;
        });
        self.host.emit("whatsapp-chats-changed", json!({}));
        self.logging_out.store(false, Ordering::SeqCst);
        Ok(self.status())
    }

    fn wipe_local(&self) {
        let _ = self.store.wipe();
        if !self.remove_session_files() {
            let _ = std::fs::write(self.dir.join(WIPE_MARKER), b"1");
        }
    }

    async fn on_event(self: Arc<Self>, event: Arc<Event>, client: Arc<Client>) {
        match &*event {
            Event::PairingQrCode(qr) => {
                let svg = qr_svg(&qr.code);
                let secs = qr.timeout.as_secs();
                self.update(|i| {
                    i.state = State::Pairing;
                    i.qr_svg = svg;
                    i.qr_timeout_secs = Some(secs);
                });
            }
            Event::PairingCode(pc) => {
                let code = pc.code.clone();
                self.update(|i| {
                    i.state = State::Pairing;
                    i.pair_code = Some(code);
                });
            }
            Event::PairingCodeError(e) => {
                let msg = e.error.clone();
                self.update(|i| {
                    i.pair_code = None;
                    i.error = Some(msg);
                });
            }
            Event::PairingQrCodesExhausted(_) => {
                self.update(|i| {
                    i.qr_svg = None;
                    i.error = Some("qr_expired".into());
                });
                let me = self.clone();
                tokio::spawn(async move {
                    let _ = me.disconnect().await;
                    me.update(|i| i.error = Some("qr_expired".into()));
                });
            }
            Event::PairSuccess(ps) => {
                self.save_account(&Account {
                    jid: ps.id.to_non_ad_string(),
                    push_name: None,
                    platform: Some(ps.platform.clone()).filter(|p| !p.is_empty()),
                });
                self.update(|i| {
                    i.state = State::Starting;
                    i.qr_svg = None;
                    i.pair_code = None;
                    i.error = None;
                });
            }
            Event::PairError(e) => {
                let msg = e.error.clone();
                self.update(|i| i.error = Some(msg));
            }
            Event::Connected(_) => {
                let mut account = self.account().unwrap_or_default();
                if let Some(pn) = client.pn() {
                    account.jid = pn.to_non_ad_string();
                }
                let name = client.push_name();
                if !name.is_empty() {
                    account.push_name = Some(name);
                }
                if !account.jid.is_empty() {
                    self.save_account(&account);
                }
                self.update(|i| {
                    i.state = State::Connected;
                    i.qr_svg = None;
                    i.pair_code = None;
                    i.error = None;
                });
                let me = self.clone();
                tokio::spawn(async move { me.refresh_groups(client).await });
            }
            Event::Disconnected(_) => {
                let linked = self.account().is_some();
                self.update(|i| {
                    if i.state == State::Connected || linked {
                        i.state = State::Offline;
                    }
                });
            }
            Event::LoggedOut(_) => {
                if self.logging_out.load(Ordering::SeqCst) || self.account().is_none() {
                    return;
                }
                let me = self.clone();
                tokio::spawn(async move {
                    let running = me.running.lock().await.take();
                    if let Some(mut r) = running {
                        if let Some(h) = r.handle.take() {
                            h.abort();
                        }
                    }
                    me.wipe_local();
                    me.update(|i| {
                        i.state = State::LoggedOut;
                        i.qr_svg = None;
                        i.pair_code = None;
                        i.sync_progress = None;
                    });
                    me.host.emit("whatsapp-chats-changed", json!({}));
                });
            }
            Event::StreamReplaced(_) => {
                self.update(|i| {
                    i.state = State::Offline;
                    i.error = Some("stream_replaced".into());
                });
            }
            Event::TemporaryBan(_) => {
                self.update(|i| {
                    i.state = State::Offline;
                    i.error = Some("temporary_ban".into());
                });
            }
            Event::ClientOutdated(_) => {
                self.update(|i| {
                    i.state = State::Offline;
                    i.error = Some("client_outdated".into());
                });
            }
            Event::ConnectFailure(f) => {
                let msg = f
                    .message
                    .clone()
                    .unwrap_or_else(|| format!("{:?}", f.reason));
                self.update(|i| i.error = Some(msg));
            }
            Event::HistorySync(lazy) => {
                let lazy = (**lazy).clone();
                let store = self.store.clone();
                let progress = lazy.progress();
                let result = tokio::task::spawn_blocking(move || {
                    let Some(hs) = lazy.get() else {
                        return Ok((0usize, 0usize));
                    };
                    crate::ingest::history(&store, hs)
                })
                .await;
                if let Ok(Ok((chats, messages))) = result {
                    if let Some(p) = progress {
                        self.update(|i| i.sync_progress = Some(p));
                    }
                    self.host.emit(
                        "whatsapp-sync",
                        json!({ "progress": progress, "chats": chats, "messages": messages }),
                    );
                    self.host.emit("whatsapp-chats-changed", json!({}));
                }
            }
            Event::Messages(batch) => {
                let mut touched: Vec<String> = Vec::new();
                let store = self.store.clone();
                let items: Vec<InboundMessage> = batch.iter().cloned().collect();
                let res =
                    tokio::task::spawn_blocking(move || crate::ingest::live(&store, &items)).await;
                if let Ok(Ok(chats)) = res {
                    touched = chats;
                }
                for chat in &touched {
                    self.host.emit("whatsapp-message", json!({ "chat": chat }));
                }
                if !touched.is_empty() {
                    self.host.emit("whatsapp-chats-changed", json!({}));
                }
            }
            Event::ContactUpdate(c) => {
                let jid = c.jid.to_non_ad_string();
                let full = c
                    .action
                    .full_name
                    .clone()
                    .or_else(|| c.action.first_name.clone());
                let lid = c.action.lid_jid.clone();
                let pn = c.action.pn_jid.clone();
                let _ = self.store.with_tx(|tx| {
                    if let (Some(l), Some(p)) = (lid.as_deref(), pn.as_deref()) {
                        tx.alias(l, p)?;
                    }
                    let target = tx.canonical(&jid);
                    tx.contact(&target, full.as_deref(), None)
                });
            }
            Event::MarkChatAsReadUpdate(u) => {
                let jid = self.store.canonical(&u.jid.to_non_ad_string());
                match u.action.read {
                    Some(true) => {
                        let _ = self.store.mark_read(&jid);
                    }
                    Some(false) => {
                        let _ = self.store.set_unread(&jid, 1);
                    }
                    None => {}
                }
                self.host.emit("whatsapp-chats-changed", json!({}));
            }
            Event::ArchiveUpdate(u) => {
                let jid = self.store.canonical(&u.jid.to_non_ad_string());
                if let Some(a) = u.action.archived {
                    let _ = self.store.set_archived(&jid, a);
                    self.host.emit("whatsapp-chats-changed", json!({}));
                }
            }
            _ => {}
        }
    }

    async fn refresh_groups(&self, client: Arc<Client>) {
        let Ok(groups) = client.groups().list_participating().await else {
            return;
        };
        let _ = self.store.with_tx(|tx| {
            for g in &groups {
                tx.upsert_chat(
                    &g.id.to_non_ad_string(),
                    g.subject.as_deref(),
                    None,
                    None,
                    None,
                )?;
            }
            Ok(())
        });
        self.host.emit("whatsapp-chats-changed", json!({}));
    }

    pub fn chats(&self, query: Option<&str>) -> Result<Vec<ChatRow>> {
        Ok(self.store.chats(query)?)
    }

    pub fn messages(
        &self,
        chat: &str,
        before_ts: Option<i64>,
        limit: Option<u32>,
    ) -> Result<Vec<MessageRow>> {
        let limit = limit.unwrap_or(80).clamp(1, 500);
        Ok(self.store.messages(chat, before_ts, limit)?)
    }

    pub async fn mark_read(&self, chat: &str) -> Result<usize> {
        let unread = self.store.unread(chat)?;
        if unread == 0 {
            return Ok(0);
        }
        let targets = self
            .store
            .read_targets(chat, unread.min(MAX_READ_RECEIPTS))?;
        let client = self.connected_client().await?;
        let mut sent = 0usize;
        for ((wire_chat, sender), ids) in group_receipts(&targets) {
            let jid: Jid = wire_chat
                .parse()
                .map_err(|e| anyhow!("invalid chat id: {e}"))?;
            let sender: Option<Jid> = sender.as_deref().and_then(|s| s.parse().ok());
            let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
            client.mark_as_read(&jid, sender.as_ref(), &refs).await?;
            sent += refs.len();
        }
        self.store.mark_read(chat)?;
        self.host.emit("whatsapp-chats-changed", json!({}));
        Ok(sent)
    }

    pub fn media_stats(&self, chat: &str) -> Result<MediaStats> {
        Ok(self.store.media_stats(chat)?)
    }

    pub async fn send_text(&self, chat: &str, text: &str) -> Result<MessageRow> {
        let text = text.trim_end();
        if text.trim().is_empty() {
            bail!("empty_message");
        }
        let jid: Jid = chat.parse().map_err(|e| anyhow!("invalid chat id: {e}"))?;
        let client = self.connected_client().await?;
        let sent = client
            .send_text(jid, text.to_string())
            .await
            .map_err(|e| anyhow!("{e}"))?;
        let ts = chrono::Utc::now().timestamp();
        let summary = model::Summary {
            kind: Some(MessageKind::Text),
            text: Some(text.to_string()),
            ..Default::default()
        };
        self.store.with_tx(|tx| {
            tx.upsert_chat(chat, None, Some(ts), None, None)?;
            tx.insert_message(&NewMessage {
                chat,
                id: &sent.message_id,
                from_me: true,
                sender: None,
                push_name: None,
                ts,
                summary: &summary,
                raw: None,
                wire_chat: Some(chat),
                wire_sender: None,
            })
        })?;
        self.host.emit("whatsapp-chats-changed", json!({}));
        Ok(MessageRow {
            id: sent.message_id,
            chat: chat.to_string(),
            from_me: true,
            sender: None,
            sender_name: None,
            ts,
            kind: MessageKind::Text,
            text: Some(text.to_string()),
            mime: None,
            size: None,
            file_name: None,
            duration: None,
            thumb: None,
            saved_path: None,
            edited: false,
            deleted: false,
        })
    }

    pub async fn request_older(&self, chat: &str) -> Result<()> {
        let Some((id, from_me, ts)) = self.store.oldest(chat)? else {
            bail!("no_anchor");
        };
        let jid: Jid = chat.parse().map_err(|e| anyhow!("invalid chat id: {e}"))?;
        let client = self.connected_client().await?;
        client
            .fetch_message_history(&jid, &id, from_me, ts.saturating_mul(1000), 50)
            .await?;
        Ok(())
    }

    pub fn chat_dir(&self, base: &Path, chat: &str) -> Result<PathBuf> {
        let name = self.store.chat_name(chat)?;
        Ok(base.join("WhatsApp").join(naming::sanitize(&name)))
    }

    async fn request_reupload(
        &self,
        client: &Arc<Client>,
        chat: &str,
        row: &MediaRow,
        key: &[u8],
    ) -> std::result::Result<String, String> {
        use whatsapp_rust::{MediaRetryResult, MediaReuploadError, MediaReuploadRequest};
        let wire = row.wire_chat.as_deref().unwrap_or(chat);
        let chat_jid: Jid = wire.parse().map_err(|_| "media_retry_failed".to_string())?;
        let participant: Option<Jid> = if store::is_group(wire) && !row.from_me {
            row.wire_sender.as_deref().and_then(|s| s.parse().ok())
        } else {
            None
        };
        let req = MediaReuploadRequest {
            msg_id: &row.id,
            chat_jid: &chat_jid,
            media_key: key,
            is_from_me: row.from_me,
            participant: participant.as_ref(),
        };
        let reupload = client.media_reupload();
        let answer = tokio::time::timeout(RETRY_WAIT, reupload.request(&req)).await;
        match answer {
            Err(_) | Ok(Err(MediaReuploadError::Timeout)) => Err("retry_timeout".into()),
            Ok(Err(MediaReuploadError::NotLoggedIn)) => Err("offline".into()),
            Ok(Err(_)) => Err("media_retry_failed".into()),
            Ok(Ok(MediaRetryResult::Success { direct_path })) => Ok(direct_path),
            Ok(Ok(MediaRetryResult::NotFound)) => Err("media_gone".into()),
            Ok(Ok(_)) => Err("media_retry_failed".into()),
        }
    }

    async fn save_one(
        &self,
        client: &Arc<Client>,
        chat: &str,
        row: &MediaRow,
        dir: &Path,
        job: Option<&str>,
    ) -> Result<SavedFile> {
        if let Some(prev) = row.saved_path.as_deref() {
            if Path::new(prev).is_file() {
                return Ok(SavedFile {
                    message_id: row.id.clone(),
                    path: prev.to_string(),
                    skipped: true,
                });
            }
        }
        let mut msg = whatsapp_rust::waproto::codec::message_decode(&row.raw)
            .map_err(|e| anyhow!("decode: {e:?}"))?;
        if media_of(&msg).is_none() {
            bail!("no_media");
        }
        std::fs::create_dir_all(dir)?;
        let name = naming::media_file_name(
            &NameInput {
                ts: row.ts,
                id: &row.id,
                kind: row.kind,
                mime: row.mime.as_deref(),
                file_name: row.file_name.as_deref(),
            },
            local_offset(),
        );
        let target = naming::unique_path(dir, &name);
        let part = target.with_extension(format!(
            "{}.part",
            target
                .extension()
                .map(|e| e.to_string_lossy().into_owned())
                .unwrap_or_default()
        ));
        let first = fetch_to(client, &msg, &part).await;
        let outcome = match first {
            Err(e) if is_expired(&e) => {
                let Some(key) = media_key_of(&msg) else {
                    let _ = std::fs::remove_file(&part);
                    bail!("media_expired");
                };
                self.host.emit(
                    "whatsapp-download-progress",
                    json!({ "job_id": job, "chat": chat, "message_id": row.id, "stage": "retrying" }),
                );
                let path = match self.request_reupload(client, chat, row, &key).await {
                    Ok(p) => p,
                    Err(code) => {
                        let _ = std::fs::remove_file(&part);
                        bail!(code);
                    }
                };
                set_direct_path(&mut msg, &path);
                let raw = whatsapp_rust::waproto::codec::message_to_vec(&msg);
                self.store.set_raw(chat, &row.id, &raw)?;
                fetch_to(client, &msg, &part).await
            }
            other => other,
        };
        match outcome {
            Ok(()) => std::fs::rename(&part, &target)?,
            Err(e) => {
                let _ = std::fs::remove_file(&part);
                bail!(friendly_download_error(&e));
            }
        }
        let path = target.to_string_lossy().into_owned();
        self.store.set_saved(chat, &row.id, &path)?;
        Ok(SavedFile {
            message_id: row.id.clone(),
            path,
            skipped: false,
        })
    }

    pub async fn download_message(&self, chat: &str, id: &str, base: &Path) -> Result<SavedFile> {
        let row = self
            .store
            .media_row(chat, id)?
            .ok_or_else(|| anyhow!("no_media"))?;
        let client = self.connected_client().await?;
        let dir = self.chat_dir(base, chat)?;
        self.host.emit(
            "whatsapp-download-progress",
            json!({ "job_id": null, "chat": chat, "message_id": id, "stage": "start" }),
        );
        let out = self.save_one(&client, chat, &row, &dir, None).await;
        match &out {
            Ok(f) => self.host.emit(
                "whatsapp-download-progress",
                json!({ "job_id": null, "chat": chat, "message_id": id, "stage": "done", "path": f.path }),
            ),
            Err(e) => self.host.emit(
                "whatsapp-download-progress",
                json!({ "job_id": null, "chat": chat, "message_id": id, "stage": "error", "error": e.to_string() }),
            ),
        }
        out
    }

    pub async fn download_chat(
        self: &Arc<Self>,
        chat: &str,
        filter: &[MediaKind],
        base: &Path,
    ) -> Result<DownloadJob> {
        let kinds = model::kinds_for_filter(filter);
        if kinds.is_empty() {
            bail!("empty_filter");
        }
        let rows = self.store.media_rows(chat, &kinds)?;
        let client = self.connected_client().await?;
        let dir = self.chat_dir(base, chat)?;
        let job_id = format!("wa-{}", self.job_seq.fetch_add(1, Ordering::Relaxed));
        let cancel = Arc::new(AtomicBool::new(false));
        self.jobs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(job_id.clone(), cancel.clone());
        let job = DownloadJob {
            job_id: job_id.clone(),
            total: rows.len(),
            dir: dir.to_string_lossy().into_owned(),
        };
        let me = self.clone();
        let chat = chat.to_string();
        tokio::spawn(async move {
            let total = rows.len();
            let (mut done, mut skipped, mut failed) = (0usize, 0usize, 0usize);
            let mut bytes: u64 = 0;
            let mut errors: HashMap<String, usize> = HashMap::new();
            for (index, row) in rows.iter().enumerate() {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                me.host.emit(
                    "whatsapp-download-progress",
                    json!({
                        "job_id": job_id, "chat": chat, "message_id": row.id, "stage": "file",
                        "index": index, "total": total, "done": done, "skipped": skipped, "failed": failed, "bytes": bytes,
                    }),
                );
                match me.save_one(&client, &chat, row, &dir, Some(&job_id)).await {
                    Ok(f) if f.skipped => skipped += 1,
                    Ok(_) => {
                        done += 1;
                        bytes += row.size.unwrap_or(0);
                    }
                    Err(e) => {
                        failed += 1;
                        *errors.entry(e.to_string()).or_default() += 1;
                    }
                }
            }
            let cancelled = cancel.load(Ordering::Relaxed);
            me.jobs
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&job_id);
            me.host.emit(
                "whatsapp-download-done",
                json!({
                    "job_id": job_id, "chat": chat, "total": total, "done": done, "skipped": skipped,
                    "failed": failed, "bytes": bytes, "dir": dir.to_string_lossy(), "cancelled": cancelled,
                    "errors": errors,
                }),
            );
        });
        Ok(job)
    }

    pub fn cancel_download(&self, job_id: &str) -> bool {
        match self
            .jobs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(job_id)
        {
            Some(flag) => {
                flag.store(true, Ordering::Relaxed);
                true
            }
            None => false,
        }
    }

    pub fn export(
        &self,
        chat: &str,
        format: ExportFormat,
        path: &Path,
        labels: &ExportLabels,
    ) -> Result<usize> {
        let name = self.store.chat_name(chat)?;
        let rows = self.store.all_messages(chat)?;
        let items: Vec<ExportMessage> = rows
            .into_iter()
            .map(|r| ExportMessage {
                id: r.id,
                ts: r.ts,
                from_me: r.from_me,
                sender: if r.from_me {
                    labels.you.clone()
                } else {
                    r.sender_name.unwrap_or_else(|| store::phone_label(chat))
                },
                kind: r.kind,
                text: r.text,
                file_name: r.file_name,
                mime: r.mime,
                size: r.size,
                saved_path: r.saved_path.filter(|p| Path::new(p).is_file()),
                edited: r.edited,
                deleted: r.deleted,
            })
            .collect();
        let offset = local_offset();
        let body = match format {
            ExportFormat::Json => export::to_json(chat, &name, &items, offset),
            ExportFormat::Txt => export::to_txt(&items, labels, offset),
            ExportFormat::Html => export::to_html(&name, &items, labels, offset),
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, body)?;
        Ok(items.len())
    }

    pub fn suggested_export_name(&self, chat: &str, format: ExportFormat) -> Result<String> {
        let name = self.store.chat_name(chat)?;
        Ok(format!(
            "WhatsApp - {}.{}",
            naming::sanitize(&name),
            format.extension()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phone_numbers_are_normalized() {
        assert_eq!(
            normalize_phone("+55 (11) 99999-0000").unwrap(),
            "5511999990000"
        );
        assert_eq!(
            normalize_phone("0055 11 99999 0000").unwrap(),
            "5511999990000"
        );
        assert!(normalize_phone("123").is_err());
        assert!(normalize_phone("1234567890123456").is_err());
    }

    #[test]
    fn qr_renders_svg() {
        let svg = qr_svg("2@abc,def,ghi").unwrap();
        assert!(svg.starts_with("<?xml") || svg.starts_with("<svg"));
        assert!(svg.contains("#111111"));
    }

    #[test]
    fn download_errors_are_classified() {
        assert_eq!(
            friendly_download_error(&anyhow!("HTTP status 404")),
            "media_expired"
        );
        assert_eq!(friendly_download_error(&anyhow!("boom")), "boom");
    }

    #[test]
    fn receipts_group_by_chat_and_group_sender() {
        let t = |id: &str, chat: &str, sender: Option<&str>| store::ReadTarget {
            id: id.into(),
            wire_chat: chat.into(),
            wire_sender: sender.map(Into::into),
        };
        let g = group_receipts(&[
            t("1", "g@g.us", Some("a@lid")),
            t("2", "g@g.us", Some("b@lid")),
            t("3", "g@g.us", Some("a@lid")),
            t("4", "55@s.whatsapp.net", Some("ignored@lid")),
            t("5", "55@s.whatsapp.net", None),
        ]);
        assert_eq!(
            g,
            vec![
                (
                    ("g@g.us".into(), Some("a@lid".into())),
                    vec!["1".into(), "3".into()]
                ),
                (("g@g.us".into(), Some("b@lid".into())), vec!["2".into()]),
                (
                    ("55@s.whatsapp.net".into(), None),
                    vec!["4".into(), "5".into()]
                ),
            ]
        );
    }

    #[test]
    fn direct_path_is_rewritten_inside_wrappers() {
        use whatsapp_rust::prelude::MessageField;
        let mut msg = wa::Message {
            ephemeral_message: MessageField::some(wa::message::FutureProofMessage {
                message: MessageField::some(wa::Message {
                    image_message: MessageField::some(wa::message::ImageMessage {
                        direct_path: Some("/old".into()),
                        media_key: Some(vec![1, 2, 3]),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
            }),
            ..Default::default()
        };
        assert_eq!(media_key_of(&msg), Some(vec![1, 2, 3]));
        assert!(set_direct_path(&mut msg, "/new"));
        let base = msg.get_base_message();
        assert_eq!(
            base.image_message
                .as_option()
                .unwrap()
                .direct_path
                .as_deref(),
            Some("/new")
        );
        assert!(!set_direct_path(&mut wa::Message::default(), "/x"));
    }
}
