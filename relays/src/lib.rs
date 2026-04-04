//! Relay-Adapter für PhantomChat.
//!
//! Drei `BridgeProvider`-Implementierungen:
//!
//! | Provider            | Modus            | Transport                        |
//! |---------------------|------------------|----------------------------------|
//! | `InMemoryRelay`     | Test / Demo      | In-Process Queue                 |
//! | `NostrRelay`        | DailyUse         | WebSocket + TLS (direkt)         |
//! | `StealthNostrRelay` | MaximumStealth   | WebSocket + TLS über SOCKS5      |
//!
//! ## Nostr-Protokoll
//! PhantomChat nutzt Nostr als anonymes Transport-Overlay.  Jede Nachricht
//! wird als Nostr-Event (Kind 1059 — Gift Wrap, NIP-59) publiziert.  Der
//! Payload ist das hex-kodierte `Envelope`-Byte-Array.  Weil das Envelope
//! bereits Ende-zu-Ende-verschlüsselt ist, fügt das Nostr-Layer nur
//! Transport-Routing hinzu, kein weiteres Krypto.
//!
//! Events werden mit einem **ephemeren secp256k1-Keypair** pro Session
//! signiert — der Sender ist gegenüber Relay-Betreibern pseudonym.

use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::STANDARD as B64};
use futures::StreamExt;
use phantomchat_core::envelope::Envelope;
use rand::rngs::OsRng;
use secp256k1::{Keypair, Message, Secp256k1, SecretKey};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};
use std::collections::VecDeque;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message as WsMessage};
use url::Url;

// ── Nostr event primitives ────────────────────────────────────────────────────

/// Nostr Kind 1059 — Gift Wrap (NIP-59): opaque, sealed event.
const NOSTR_KIND_PHANTOM: u64 = 1059;

/// A fully serialized Nostr event ready for `["EVENT", <event>]` wire format.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct NostrEvent {
    pub id: String,
    pub pubkey: String,
    pub created_at: u64,
    pub kind: u64,
    pub tags: Vec<Vec<String>>,
    pub content: String,
    pub sig: String,
}

impl NostrEvent {
    /// Build and sign a new event carrying `payload` bytes.
    ///
    /// Uses an ephemeral `Keypair` so each session appears as a different
    /// pseudonymous author to relay operators.
    pub fn new(payload: &[u8], keypair: &Keypair) -> Self {
        let secp = Secp256k1::new();
        let pubkey_hex = hex::encode(keypair.public_key().serialize()[1..].to_vec()); // x-only
        let created_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let content = hex::encode(payload);
        let tags: Vec<Vec<String>> = vec![];

        // Event ID = SHA256 of the canonical serialization
        let id = Self::compute_id(&pubkey_hex, created_at, NOSTR_KIND_PHANTOM, &tags, &content);

        // Schnorr signature over the event ID
        let msg = Message::from_digest(
            <[u8; 32]>::try_from(hex::decode(&id).unwrap().as_slice()).unwrap()
        );
        let sig = secp.sign_schnorr(&msg, keypair);
        let sig_hex = hex::encode(sig.as_ref());

        Self { id, pubkey: pubkey_hex, created_at, kind: NOSTR_KIND_PHANTOM, tags, content, sig: sig_hex }
    }

    fn compute_id(
        pubkey: &str,
        created_at: u64,
        kind: u64,
        tags: &[Vec<String>],
        content: &str,
    ) -> String {
        // NIP-01: id = SHA256([0, pubkey, created_at, kind, tags, content])
        let canonical = serde_json::json!([0, pubkey, created_at, kind, tags, content]);
        let bytes = serde_json::to_vec(&canonical).unwrap();
        hex::encode(Sha256::digest(&bytes))
    }

    /// Parse an envelope from this event's content field.
    pub fn to_envelope(&self) -> Option<Envelope> {
        let bytes = hex::decode(&self.content).ok()?;
        Envelope::from_bytes(&bytes)
    }

    /// Serialize to `["EVENT", <event>]` wire message.
    pub fn to_wire(&self) -> String {
        serde_json::json!(["EVENT", self]).to_string()
    }

    /// Build a `["REQ", <sub_id>, {"kinds": [1059]}]` subscription filter.
    pub fn subscription_req(sub_id: &str) -> String {
        serde_json::json!(["REQ", sub_id, {"kinds": [NOSTR_KIND_PHANTOM]}]).to_string()
    }
}

// ── Session keypair (ephemeral per process start) ─────────────────────────────

fn session_keypair() -> Keypair {
    let secp = Secp256k1::new();
    let secret = SecretKey::new(&mut OsRng);
    Keypair::from_secret_key(&secp, &secret)
}

// ── BridgeHealth ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct BridgeHealth {
    pub latency_ms: u32,
    pub uptime: f32,
    pub failure_rate: f32,
}

// ── BridgeProvider trait ──────────────────────────────────────────────────────

#[async_trait]
pub trait BridgeProvider: Send + Sync {
    fn id(&self) -> &str;
    async fn publish(&self, env: Envelope) -> anyhow::Result<()>;
    async fn subscribe<F>(&self, handler: F) -> anyhow::Result<()>
    where
        F: Fn(Envelope) + Send + 'static;
    async fn health(&self) -> BridgeHealth;
}

// ── InMemoryRelay ─────────────────────────────────────────────────────────────

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

    async fn subscribe<F>(&self, handler: F) -> anyhow::Result<()>
    where
        F: Fn(Envelope) + Send + 'static,
    {
        let queue = self.queue.clone();
        tokio::spawn(async move {
            loop {
                let env = { queue.lock().unwrap().pop_front() };
                match env {
                    Some(e) => handler(e),
                    None => tokio::time::sleep(Duration::from_millis(100)).await,
                }
            }
        });
        Ok(())
    }

    async fn health(&self) -> BridgeHealth {
        BridgeHealth { latency_ms: 1, uptime: 1.0, failure_rate: 0.0 }
    }
}

// ── NostrRelay (DailyUse — direkte TLS-WebSocket-Verbindung) ─────────────────

/// Nostr-Relay für den DailyUse-Modus.
/// Baut eine direkte TLS-WebSocket-Verbindung zum Relay auf.
pub struct NostrRelay {
    id: String,
    url: String,
    keypair: Keypair,
}

impl NostrRelay {
    pub fn new(url: &str) -> Self {
        Self {
            id: url.to_owned(),
            url: url.to_owned(),
            keypair: session_keypair(),
        }
    }

    async fn connect(&self) -> anyhow::Result<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>
        >
    > {
        let url = Url::parse(&self.url)
            .map_err(|e| anyhow::anyhow!("Invalid relay URL: {}", e))?;
        let (ws, _) = connect_async(url).await
            .map_err(|e| anyhow::anyhow!("WebSocket connect failed: {}", e))?;
        tracing::debug!("NostrRelay: connected to {}", self.url);
        Ok(ws)
    }
}

#[async_trait]
impl BridgeProvider for NostrRelay {
    fn id(&self) -> &str { &self.id }

    async fn publish(&self, env: Envelope) -> anyhow::Result<()> {
        let mut ws = self.connect().await?;
        let event = NostrEvent::new(&env.to_bytes(), &self.keypair);
        ws.send(WsMessage::Text(event.to_wire())).await
            .map_err(|e| anyhow::anyhow!("WS send error: {}", e))?;
        tracing::debug!("NostrRelay: published event {}", event.id);
        Ok(())
    }

    async fn subscribe<F>(&self, handler: F) -> anyhow::Result<()>
    where
        F: Fn(Envelope) + Send + 'static,
    {
        let url = self.url.clone();
        let (handler_tx, mut handler_rx) = mpsc::unbounded_channel::<Envelope>();

        tokio::spawn(async move {
            let parsed = match Url::parse(&url) {
                Ok(u) => u,
                Err(e) => {
                    tracing::error!("NostrRelay subscribe: bad URL: {}", e);
                    return;
                }
            };

            let (mut ws, _) = match connect_async(parsed).await {
                Ok(v) => v,
                Err(e) => {
                    tracing::error!("NostrRelay subscribe connect: {}", e);
                    return;
                }
            };

            // Send subscription filter
            let req = NostrEvent::subscription_req("phantom-sub-1");
            if ws.send(WsMessage::Text(req)).await.is_err() {
                return;
            }

            while let Some(Ok(msg)) = ws.next().await {
                if let WsMessage::Text(text) = msg {
                    // Nostr wire: ["EVENT", <sub_id>, <event>]
                    if let Ok(arr) = serde_json::from_str::<serde_json::Value>(&text) {
                        if arr[0] == "EVENT" {
                            if let Ok(event) = serde_json::from_value::<NostrEvent>(arr[2].clone()) {
                                if let Some(env) = event.to_envelope() {
                                    let _ = handler_tx.send(env);
                                }
                            }
                        }
                    }
                }
            }
        });

        tokio::spawn(async move {
            while let Some(env) = handler_rx.recv().await {
                handler(env);
            }
        });

        Ok(())
    }

    async fn health(&self) -> BridgeHealth {
        let start = std::time::Instant::now();
        let ok = self.connect().await.is_ok();
        BridgeHealth {
            latency_ms: start.elapsed().as_millis() as u32,
            uptime: if ok { 1.0 } else { 0.0 },
            failure_rate: if ok { 0.0 } else { 1.0 },
        }
    }
}

// ── StealthNostrRelay (MaximumStealth — WebSocket über SOCKS5 + TLS) ─────────

/// Nostr-Relay für den MaximumStealth-Modus.
///
/// Verbindungsaufbau:
/// ```text
/// App → SOCKS5 (Tor/Nym) → TCP → TLS → WebSocket → Nostr Relay
/// ```
/// Das Relay sieht ausschließlich die Exit-IP des Anonymisierungsnetzes,
/// nie die echte IP der App.
pub struct StealthNostrRelay {
    id: String,
    relay_url: String,
    proxy_addr: String,
    keypair: Keypair,
}

impl StealthNostrRelay {
    /// * `relay_url`  — z. B. `"wss://relay.example.com"`
    /// * `proxy_addr` — SOCKS5-Proxy, z. B. `"127.0.0.1:9050"` für Tor
    pub fn new(relay_url: &str, proxy_addr: &str) -> Self {
        Self {
            id: format!("stealth:{}", relay_url),
            relay_url: relay_url.to_owned(),
            proxy_addr: proxy_addr.to_owned(),
            keypair: session_keypair(),
        }
    }

    /// Öffnet einen SOCKS5-proxied TLS-WebSocket zum Relay.
    async fn connect_stealth(&self) -> anyhow::Result<
        tokio_tungstenite::WebSocketStream<
            tokio_native_tls::TlsStream<
                tokio_socks::tcp::Socks5Stream<tokio::net::TcpStream>
            >
        >
    > {
        let parsed = Url::parse(&self.relay_url)
            .map_err(|e| anyhow::anyhow!("Invalid relay URL: {}", e))?;

        let host = parsed.host_str()
            .ok_or_else(|| anyhow::anyhow!("No host in relay URL"))?;
        let port = parsed.port_or_known_default()
            .ok_or_else(|| anyhow::anyhow!("No port for relay URL"))?;
        let target = format!("{}:{}", host, port);

        // Step 1: TCP through SOCKS5
        let socks_stream = tokio_socks::tcp::Socks5Stream::connect(
            self.proxy_addr.as_str(), target
        ).await.map_err(|e| anyhow::anyhow!("SOCKS5 connect: {}", e))?;

        tracing::debug!("StealthNostrRelay: SOCKS5 stream established via {}", self.proxy_addr);

        // Step 2: TLS over the proxied stream
        let tls_connector = native_tls::TlsConnector::new()
            .map_err(|e| anyhow::anyhow!("TLS connector: {}", e))?;
        let tokio_tls = tokio_native_tls::TlsConnector::from(tls_connector);
        let tls_stream = tokio_tls.connect(host, socks_stream).await
            .map_err(|e| anyhow::anyhow!("TLS handshake: {}", e))?;

        tracing::debug!("StealthNostrRelay: TLS handshake complete");

        // Step 3: WebSocket upgrade over the TLS-over-SOCKS5 stream
        let (ws, _) = tokio_tungstenite::client_async(
            self.relay_url.as_str(), tls_stream
        ).await.map_err(|e| anyhow::anyhow!("WebSocket upgrade: {}", e))?;

        tracing::debug!("StealthNostrRelay: WebSocket ready (stealth mode)");
        Ok(ws)
    }
}

#[async_trait]
impl BridgeProvider for StealthNostrRelay {
    fn id(&self) -> &str { &self.id }

    async fn publish(&self, env: Envelope) -> anyhow::Result<()> {
        let mut ws = self.connect_stealth().await?;
        let event = NostrEvent::new(&env.to_bytes(), &self.keypair);
        ws.send(WsMessage::Text(event.to_wire())).await
            .map_err(|e| anyhow::anyhow!("WS send: {}", e))?;
        tracing::debug!("StealthNostrRelay: published event {} via proxy", event.id);
        Ok(())
    }

    async fn subscribe<F>(&self, handler: F) -> anyhow::Result<()>
    where
        F: Fn(Envelope) + Send + 'static,
    {
        let relay_url = self.relay_url.clone();
        let proxy_addr = self.proxy_addr.clone();
        let (handler_tx, mut handler_rx) = mpsc::unbounded_channel::<Envelope>();

        tokio::spawn(async move {
            // Build a fresh stealth relay just for the subscription stream
            let stealth = StealthNostrRelay::new(&relay_url, &proxy_addr);
            let mut ws = match stealth.connect_stealth().await {
                Ok(v) => v,
                Err(e) => {
                    tracing::error!("StealthNostrRelay subscribe connect: {}", e);
                    return;
                }
            };

            let req = NostrEvent::subscription_req("phantom-stealth-sub-1");
            if ws.send(WsMessage::Text(req)).await.is_err() {
                return;
            }

            while let Some(Ok(msg)) = ws.next().await {
                if let WsMessage::Text(text) = msg {
                    if let Ok(arr) = serde_json::from_str::<serde_json::Value>(&text) {
                        if arr[0] == "EVENT" {
                            if let Ok(event) = serde_json::from_value::<NostrEvent>(arr[2].clone()) {
                                if let Some(env) = event.to_envelope() {
                                    let _ = handler_tx.send(env);
                                }
                            }
                        }
                    }
                }
            }
        });

        tokio::spawn(async move {
            while let Some(env) = handler_rx.recv().await {
                handler(env);
            }
        });

        Ok(())
    }

    async fn health(&self) -> BridgeHealth {
        let start = std::time::Instant::now();
        let ok = self.connect_stealth().await.is_ok();
        BridgeHealth {
            latency_ms: start.elapsed().as_millis() as u32,
            uptime: if ok { 1.0 } else { 0.0 },
            failure_rate: if ok { 0.0 } else { 1.0 },
        }
    }
}

// ── Factory ───────────────────────────────────────────────────────────────────

/// Wählt den passenden Provider basierend auf dem Privacy-Modus.
///
/// * `stealth = false` → `NostrRelay` (direkte TLS-Verbindung)
/// * `stealth = true`  → `StealthNostrRelay` (über SOCKS5)
pub fn make_relay(
    relay_url: &str,
    stealth: bool,
    proxy_addr: Option<&str>,
) -> Box<dyn BridgeProvider> {
    if stealth {
        let proxy = proxy_addr.unwrap_or("127.0.0.1:9050");
        Box::new(StealthNostrRelay::new(relay_url, proxy))
    } else {
        Box::new(NostrRelay::new(relay_url))
    }
}

// ── Stealth Cover Traffic Consumer ───────────────────────────────────────────

/// Startet einen Background-Task der Cover-Traffic-Dummies aus dem Core-Kanal
/// liest und über den `StealthNostrRelay` publiziert.
///
/// Muss nach `start_network_node()` aufgerufen werden wenn MaximumStealth
/// aktiv ist.  Ist der Channel nicht vorhanden (DailyUse-Modus oder schon
/// abgeholt), passiert nichts.
pub fn start_stealth_cover_consumer(relay_url: &str, proxy_addr: &str) {
    let mut rx = match phantomchat_core::api::take_stealth_cover_rx() {
        Some(r) => r,
        None => return,
    };

    let relay = StealthNostrRelay::new(relay_url, proxy_addr);

    tokio::spawn(async move {
        while let Some(raw_bytes) = rx.recv().await {
            if let Some(env) = Envelope::from_bytes(&raw_bytes) {
                if let Err(e) = relay.publish(env).await {
                    tracing::debug!("Stealth cover traffic publish error: {}", e);
                }
            }
        }
    });
}
