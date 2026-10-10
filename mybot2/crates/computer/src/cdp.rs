//! A minimal Chrome DevTools Protocol client over one WebSocket.
//!
//! We attach to the Chromium already running inside the container rather than
//! launching our own: the human takes over the *same* session over the live
//! view, so a separate browser could never be handed over.
//!
//! Chromium advertises its websocket as `ws://127.0.0.1:<inner port>/…` — its
//! own address inside the container. The port is swapped for the host-side
//! one here (rewriting the HTTP body in the relay would break Content-Length).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::{Mutex, broadcast, mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message as WsMessage;

#[derive(Debug, Clone)]
pub struct CdpEvent {
    pub method: String,
    pub params: Value,
    pub session_id: Option<String>,
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;

#[derive(Clone)]
pub struct Cdp {
    tx: mpsc::UnboundedSender<String>,
    pending: Pending,
    next: Arc<AtomicU64>,
    events: broadcast::Sender<CdpEvent>,
    alive: Arc<std::sync::atomic::AtomicBool>,
}

pub async fn wait_for_cdp(port: u16, timeout: Duration) -> Result<String, String> {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut last = "no attempt".to_string();
    while tokio::time::Instant::now() < deadline {
        match version(port).await {
            Ok(v) => return Ok(v["Browser"].as_str().unwrap_or("Chromium").to_string()),
            Err(e) => last = e,
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    Err(format!("Chromium did not expose CDP on :{port} within {}s ({last})", timeout.as_secs()))
}

async fn version(port: u16) -> Result<Value, String> {
    let r = reqwest::Client::new()
        .get(format!("http://127.0.0.1:{port}/json/version"))
        .timeout(Duration::from_secs(3))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !r.status().is_success() {
        return Err(format!("HTTP {}", r.status()));
    }
    r.json().await.map_err(|e| e.to_string())
}

impl Cdp {
    pub async fn connect(port: u16) -> Result<Self, String> {
        let v = version(port).await?;
        let advertised = v["webSocketDebuggerUrl"].as_str().ok_or("CDP advertised no websocket URL")?;
        let mut url = url::Url::parse(advertised).map_err(|e| e.to_string())?;
        url.set_host(Some("127.0.0.1")).map_err(|e| e.to_string())?;
        url.set_port(Some(port)).map_err(|_| "bad port")?;

        let (ws, _) = tokio_tungstenite::connect_async(url.as_str()).await.map_err(|e| format!("CDP connect: {e}"))?;
        let (mut sink, mut stream) = ws.split();
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let (events, _) = broadcast::channel(512);
        let alive = Arc::new(std::sync::atomic::AtomicBool::new(true));

        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if sink.send(WsMessage::Text(msg.into())).await.is_err() {
                    break;
                }
            }
        });

        let p2 = pending.clone();
        let ev2 = events.clone();
        let alive2 = alive.clone();
        tokio::spawn(async move {
            while let Some(Ok(msg)) = stream.next().await {
                let text = match msg {
                    WsMessage::Text(t) => t.to_string(),
                    WsMessage::Binary(b) => String::from_utf8_lossy(&b).to_string(),
                    WsMessage::Close(_) => break,
                    _ => continue,
                };
                let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
                if let Some(id) = v["id"].as_u64() {
                    if let Some(done) = p2.lock().await.remove(&id) {
                        let r = if let Some(err) = v.get("error") {
                            Err(err["message"].as_str().unwrap_or("CDP error").to_string())
                        } else {
                            Ok(v["result"].clone())
                        };
                        let _ = done.send(r);
                    }
                } else if let Some(m) = v["method"].as_str() {
                    let _ = ev2.send(CdpEvent {
                        method: m.to_string(),
                        params: v["params"].clone(),
                        session_id: v["sessionId"].as_str().map(String::from),
                    });
                }
            }
            alive2.store(false, Ordering::SeqCst);
            for (_, done) in p2.lock().await.drain() {
                let _ = done.send(Err("the browser connection closed".into()));
            }
        });

        Ok(Self { tx, pending, next: Arc::new(AtomicU64::new(1)), events, alive })
    }

    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    pub fn events(&self) -> broadcast::Receiver<CdpEvent> {
        self.events.subscribe()
    }

    pub async fn call(&self, method: &str, params: Value, session: Option<&str>) -> Result<Value, String> {
        self.call_timeout(method, params, session, Duration::from_secs(60)).await
    }

    pub async fn call_timeout(&self, method: &str, params: Value, session: Option<&str>, timeout: Duration) -> Result<Value, String> {
        if !self.is_alive() {
            return Err("the browser connection closed".into());
        }
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let mut msg = json!({"id": id, "method": method, "params": params});
        if let Some(s) = session {
            msg["sessionId"] = json!(s);
        }
        let (done, wait) = oneshot::channel();
        self.pending.lock().await.insert(id, done);
        self.tx.send(msg.to_string()).map_err(|_| "the browser connection closed".to_string())?;
        match tokio::time::timeout(timeout, wait).await {
            Ok(Ok(r)) => r,
            Ok(Err(_)) => Err("the browser connection closed".into()),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err(format!("{method} timed out after {}s", timeout.as_secs()))
            }
        }
    }
}
