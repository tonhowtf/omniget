//! Minimal Chrome DevTools Protocol client over one WebSocket, in flatten
//! mode: one connection to the browser target, page sessions addressed by
//! `sessionId`. Commands get a oneshot for their reply; events go to one
//! unbounded channel the browser loop reads.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

type Reply = oneshot::Sender<Result<Value, String>>;

pub struct Cdp {
    out: mpsc::UnboundedSender<String>,
    next_id: AtomicU64,
    pending: Arc<Mutex<HashMap<u64, Reply>>>,
    closed: Arc<AtomicBool>,
}

/// One CDP event: `method`, `params` and the page session it came from.
#[derive(Debug, Clone)]
pub struct Event {
    pub method: String,
    pub params: Value,
    pub session: Option<String>,
}

impl Cdp {
    pub async fn connect(
        ws_url: &str,
    ) -> Result<(Arc<Cdp>, mpsc::UnboundedReceiver<Event>), String> {
        let (ws, _) = tokio::time::timeout(
            Duration::from_secs(10),
            tokio_tungstenite::connect_async(ws_url),
        )
        .await
        .map_err(|_| "timed out connecting to the browser".to_string())?
        .map_err(|e| format!("could not connect to the browser: {e}"))?;
        let (mut sink, mut stream) = ws.split();
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
        let (ev_tx, ev_rx) = mpsc::unbounded_channel::<Event>();
        let pending: Arc<Mutex<HashMap<u64, Reply>>> = Arc::default();
        let closed = Arc::new(AtomicBool::new(false));

        tokio::spawn(async move {
            while let Some(text) = out_rx.recv().await {
                if sink.send(Message::Text(text.into())).await.is_err() {
                    break;
                }
            }
            let _ = sink.close().await;
        });

        let pending_r = pending.clone();
        let closed_r = closed.clone();
        tokio::spawn(async move {
            while let Some(msg) = stream.next().await {
                let text = match msg {
                    Ok(Message::Text(t)) => t.to_string(),
                    Ok(Message::Binary(b)) => String::from_utf8_lossy(&b).into_owned(),
                    Ok(Message::Close(_)) | Err(_) => break,
                    Ok(_) => continue,
                };
                let Ok(v) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };
                if let Some(id) = v.get("id").and_then(Value::as_u64) {
                    let reply = pending_r.lock().ok().and_then(|mut p| p.remove(&id));
                    if let Some(tx) = reply {
                        let res = match v.get("error") {
                            Some(e) => Err(e
                                .get("message")
                                .and_then(Value::as_str)
                                .unwrap_or("CDP error")
                                .to_string()),
                            None => Ok(v.get("result").cloned().unwrap_or(Value::Null)),
                        };
                        let _ = tx.send(res);
                    }
                } else if let Some(method) = v.get("method").and_then(Value::as_str) {
                    let _ = ev_tx.send(Event {
                        method: method.to_string(),
                        params: v.get("params").cloned().unwrap_or(Value::Null),
                        session: v.get("sessionId").and_then(Value::as_str).map(String::from),
                    });
                }
            }
            closed_r.store(true, Ordering::SeqCst);
            if let Ok(mut p) = pending_r.lock() {
                for (_, tx) in p.drain() {
                    let _ = tx.send(Err("the browser closed".into()));
                }
            }
            // Dropping ev_tx ends the browser loop.
        });

        Ok((
            Arc::new(Cdp {
                out: out_tx,
                next_id: AtomicU64::new(1),
                pending,
                closed,
            }),
            ev_rx,
        ))
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    pub async fn call(
        &self,
        method: &str,
        params: Value,
        session: Option<&str>,
    ) -> Result<Value, String> {
        self.call_timeout(method, params, session, Duration::from_secs(30))
            .await
    }

    pub async fn call_timeout(
        &self,
        method: &str,
        params: Value,
        session: Option<&str>,
        timeout: Duration,
    ) -> Result<Value, String> {
        if self.is_closed() {
            return Err("the browser is not running".into());
        }
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let mut msg = json!({ "id": id, "method": method, "params": params });
        if let Some(s) = session {
            msg["sessionId"] = json!(s);
        }
        let (tx, rx) = oneshot::channel();
        if let Ok(mut p) = self.pending.lock() {
            p.insert(id, tx);
        }
        if self.out.send(msg.to_string()).is_err() {
            return Err("the browser is not running".into());
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(res)) => res.map_err(|e| format!("{method}: {e}")),
            Ok(Err(_)) => Err("the browser closed".into()),
            Err(_) => {
                if let Ok(mut p) = self.pending.lock() {
                    p.remove(&id);
                }
                Err(format!("{method}: timed out"))
            }
        }
    }
}
