//! Relay transport layer for PhantomChat.
//!
//! ## Architecture
//! All envelopes are broadcast to a dumb relay (Nostr or in-memory).
//! The relay is metadata-blind: it stores opaque blobs, knows neither
//! sender nor recipient. Every client downloads ALL envelopes and scans
//! with their ViewKey — similar to Monero's stealth address model.
//!
//! ## Providers
//! - `InMemoryRelay` — for tests, same process
//! - `NostrRelay`    — production: WebSocket to any NIP-01 relay

pub mod nostr;

use async_trait::async_trait;
use phantomchat_core::Envelope;
use std::sync::{Arc, Mutex};
use std::collections::VecDeque;
use std::time::Duration;
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use crate::nostr::{NostrEvent, NostrFilter, RelayMsg};

/// Health metrics for a relay connection.
#[derive(Debug, Clone)]
pub struct BridgeHealth {
    pub latency_ms: u32,
    pub uptime: f32,
    pub failure_rate: f32,
    pub connected: bool,
}

#[async_trait]
pub trait BridgeProvider: Send + Sync {
    fn id(&self) -> &str;
    async fn publish(&self, env: Envelope) -> anyhow::Result<()>;
    async fn subscribe<F>(&self, since: u64, handler: F) -> anyhow::Result<()>
    where
        F: Fn(Envelope) + Send + 'static;
    async fn health(&self) -> BridgeHealth;
}

// ── In-Memory Relay (tests) ───────────────────────────────────────────────────

pub struct InMemoryRelay {
    id: String,
    queue: Arc<Mutex<VecDeque<Envelope>>>,
}

impl InMemoryRelay {
    pub fn new(id: &str) -> Self {
        Self { id: id.to_owned(), queue: Arc::new(Mutex::new(VecDeque::new())) }
    }
}

#[async_trait]
impl BridgeProvider for InMemoryRelay {
    fn id(&self) -> &str { &self.id }

    async fn publish(&self, env: Envelope) -> anyhow::Result<()> {
        self.queue.lock().unwrap().push_back(env);
        Ok(())
    }

    async fn subscribe<F>(&self, _since: u64, handler: F) -> anyhow::Result<()>
    where
        F: Fn(Envelope) + Send + 'static,
    {
        let queue = self.queue.clone();
        tokio::spawn(async move {
            loop {
                let env = { queue.lock().unwrap().pop_front() };
                if let Some(e) = env { handler(e); }
                else { tokio::time::sleep(Duration::from_millis(50)).await; }
            }
        });
        Ok(())
    }

    async fn health(&self) -> BridgeHealth {
        BridgeHealth { latency_ms: 0, uptime: 1.0, failure_rate: 0.0, connected: true }
    }
}

// ── Nostr Relay (production WebSocket) ───────────────────────────────────────

/// Real Nostr relay via WebSocket (NIP-01).
///
/// Each `publish()` call:
/// 1. Serialises the Envelope to bytes → hex-encodes it
/// 2. Creates a fresh ephemeral secp256k1 keypair (unlinkable post)
/// 3. Builds and signs a NIP-01 event (kind 1984)
/// 4. Sends ["EVENT", ...] over WebSocket
///
/// Each `subscribe()` call:
/// 1. Connects WebSocket
/// 2. Sends ["REQ", sub_id, {kinds:[1984], since:...}]
/// 3. For each incoming EVENT: hex-decodes content → Envelope::from_bytes
/// 4. Calls handler(envelope) — client does ViewKey scanning
pub struct NostrRelay {
    url: String,
}

impl NostrRelay {
    pub fn new(url: &str) -> Self {
        Self { url: url.to_owned() }
    }
}

#[async_trait]
impl BridgeProvider for NostrRelay {
    fn id(&self) -> &str { &self.url }

    async fn publish(&self, env: Envelope) -> anyhow::Result<()> {
        let (ws_stream, _) = connect_async(&self.url).await
            .map_err(|e| anyhow::anyhow!("WebSocket connect failed: {}", e))?;
        let (mut write, _read) = ws_stream.split();

        let bytes = env.to_bytes();
        let hex_content = hex::encode(&bytes);
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let event = NostrEvent::new_phantom(&hex_content, ts)?;
        let msg = event.to_publish_msg();

        write.send(WsMessage::Text(msg)).await
            .map_err(|e| anyhow::anyhow!("WebSocket send failed: {}", e))?;
        write.send(WsMessage::Close(None)).await.ok();
        Ok(())
    }

    async fn subscribe<F>(&self, since: u64, handler: F) -> anyhow::Result<()>
    where
        F: Fn(Envelope) + Send + 'static,
    {
        let url = self.url.clone();
        tokio::spawn(async move {
            loop {
                match connect_async(&url).await {
                    Err(e) => {
                        tracing::warn!("Relay connect failed: {}, retrying in 5s", e);
                        tokio::time::sleep(Duration::from_secs(5)).await;
                        continue;
                    }
                    Ok((ws, _)) => {
                        let (mut write, mut read) = ws.split();
                        let sub_id = format!("phantom-{}", rand::random::<u32>());
                        let filter = NostrFilter::phantom_since(since);
                        let req = filter.to_req_msg(&sub_id);

                        if write.send(WsMessage::Text(req)).await.is_err() {
                            continue;
                        }

                        while let Some(Ok(msg)) = read.next().await {
                            if let WsMessage::Text(text) = msg {
                                if let Some(RelayMsg::Event { event, .. }) = RelayMsg::parse(&text) {
                                    if let Ok(bytes) = hex::decode(&event.content) {
                                        if let Some(env) = Envelope::from_bytes(&bytes) {
                                            handler(env);
                                        }
                                    }
                                }
                            }
                        }
                        // Disconnected — reconnect after delay
                        tokio::time::sleep(Duration::from_secs(3)).await;
                    }
                }
            }
        });
        Ok(())
    }

    async fn health(&self) -> BridgeHealth {
        let start = std::time::Instant::now();
        match connect_async(&self.url).await {
            Ok((mut ws, _)) => {
                let latency = start.elapsed().as_millis() as u32;
                ws.close(None).await.ok();
                BridgeHealth { latency_ms: latency, uptime: 1.0, failure_rate: 0.0, connected: true }
            }
            Err(_) => BridgeHealth { latency_ms: 9999, uptime: 0.0, failure_rate: 1.0, connected: false },
        }
    }
}

// ── Multi-Relay Manager ───────────────────────────────────────────────────────

/// Publishes to multiple relays in parallel and collects from all.
/// Provides redundancy: if one relay is down, others still work.
pub struct RelayPool {
    relays: Vec<Arc<dyn BridgeProvider>>,
}

impl RelayPool {
    pub fn new(relays: Vec<Arc<dyn BridgeProvider>>) -> Self {
        Self { relays }
    }

    /// Default pool of public Nostr relays known to be stable.
    pub fn default_public() -> Self {
        Self::new(vec![
            Arc::new(NostrRelay::new("wss://relay.damus.io")),
            Arc::new(NostrRelay::new("wss://nos.lol")),
            Arc::new(NostrRelay::new("wss://relay.nostr.band")),
            Arc::new(NostrRelay::new("wss://nostr.bitcoiner.social")),
        ])
    }

    /// Publish to all relays concurrently. Returns Ok if at least one succeeds.
    pub async fn publish(&self, env: Envelope) -> anyhow::Result<()> {
        let (tx, mut rx) = mpsc::channel::<anyhow::Result<()>>(self.relays.len());
        for relay in &self.relays {
            let relay = relay.clone();
            let env = env.clone();
            let tx = tx.clone();
            tokio::spawn(async move {
                let _ = tx.send(relay.publish(env).await).await;
            });
        }
        drop(tx);
        let mut last_err = None;
        let mut success = false;
        while let Some(result) = rx.recv().await {
            match result {
                Ok(_) => success = true,
                Err(e) => last_err = Some(e),
            }
        }
        if success { Ok(()) }
        else { Err(last_err.unwrap_or_else(|| anyhow::anyhow!("all relays failed"))) }
    }

    /// Subscribe to all relays. Handler called for each received envelope.
    pub async fn subscribe<F>(&self, since: u64, handler: F) -> anyhow::Result<()>
    where
        F: Fn(Envelope) + Send + Sync + 'static,
    {
        let handler = Arc::new(handler);
        for relay in &self.relays {
            let h = handler.clone();
            relay.subscribe(since, move |env| h(env)).await?;
        }
        Ok(())
    }
}
