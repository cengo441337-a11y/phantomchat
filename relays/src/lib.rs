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
//! Events werden mit einem **ephemeren secp256k1-KeyPair** pro Session
//! signiert — der Sender ist gegenüber Relay-Betreibern pseudonym.

pub mod nostr;

use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use phantomchat_core::envelope::Envelope;
use rand::rngs::OsRng;
use secp256k1::{KeyPair, Message, Secp256k1, SecretKey};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};
use std::collections::{HashSet, VecDeque};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
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
    /// Uses an ephemeral `KeyPair` so each session appears as a different
    /// pseudonymous author to relay operators.
    pub fn new(payload: &[u8], keypair: &KeyPair) -> Self {
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
        let id_bytes = hex::decode(&id).expect("event id is valid hex");
        let msg = Message::from_slice(&id_bytes).expect("event id must be 32 bytes");
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

fn session_keypair() -> KeyPair {
    let secp = Secp256k1::new();
    let secret = SecretKey::new(&mut OsRng);
    KeyPair::from_secret_key(&secp, &secret)
}

// ── BridgeHealth ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct BridgeHealth {
    pub latency_ms: u32,
    pub uptime: f32,
    pub failure_rate: f32,
}

// ── BridgeProvider trait ──────────────────────────────────────────────────────

/// Callback type for incoming envelopes. Boxed (not generic) so the trait
/// stays dyn-compatible and `make_relay()` can return `Box<dyn BridgeProvider>`.
pub type EnvelopeHandler = Box<dyn Fn(Envelope) + Send + Sync + 'static>;

/// Lifecycle events surfaced by `subscribe_with_state`. The frontend's pill
/// maps these onto the existing three-state vocabulary
/// (`connecting | connected | disconnected`); `Reconnecting` is a richer
/// payload so a tooltip can show the next backoff window.
#[derive(Debug, Clone)]
pub enum ConnectionEvent {
    Connecting,
    Connected,
    Disconnected(String),
    Reconnecting { attempt: u32, backoff_secs: u32 },
}

/// Callback for `subscribe_with_state`. Same Boxed-Fn shape as
/// `EnvelopeHandler` so the trait stays dyn-compatible.
pub type StateHandler = Box<dyn Fn(ConnectionEvent) + Send + Sync + 'static>;

#[async_trait]
pub trait BridgeProvider: Send + Sync {
    fn id(&self) -> &str;
    async fn publish(&self, env: Envelope) -> anyhow::Result<()>;
    async fn subscribe(&self, handler: EnvelopeHandler) -> anyhow::Result<()>;
    async fn health(&self) -> BridgeHealth;

    /// Subscribe + receive lifecycle state events. Default implementation
    /// wraps `subscribe` and emits a single `Connected` on success / a single
    /// `Disconnected` on failure — adequate for providers that don't yet
    /// have native reconnect (e.g. `InMemoryRelay`, `StealthNostrRelay`).
    async fn subscribe_with_state(
        &self,
        envelope_handler: EnvelopeHandler,
        state_handler: StateHandler,
    ) -> anyhow::Result<()> {
        state_handler(ConnectionEvent::Connecting);
        match self.subscribe(envelope_handler).await {
            Ok(()) => {
                state_handler(ConnectionEvent::Connected);
                Ok(())
            }
            Err(e) => {
                state_handler(ConnectionEvent::Disconnected(e.to_string()));
                Err(e)
            }
        }
    }
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

    async fn subscribe(&self, handler: EnvelopeHandler) -> anyhow::Result<()> {
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
    keypair: KeyPair,
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

    async fn subscribe(&self, handler: EnvelopeHandler) -> anyhow::Result<()> {
        // Forward to the state-aware variant with a no-op state handler so
        // the auto-reconnect lives in exactly one place.
        self.subscribe_with_state(handler, Box::new(|_| {})).await
    }

    async fn subscribe_with_state(
        &self,
        envelope_handler: EnvelopeHandler,
        state_handler: StateHandler,
    ) -> anyhow::Result<()> {
        let url = self.url.clone();
        let (handler_tx, mut handler_rx) = mpsc::unbounded_channel::<Envelope>();
        let state = Arc::new(state_handler);

        // ── Reconnect-loop task ────────────────────────────────────────────
        // Tries to connect, on success runs the inner WS read loop, on any
        // exit (Err or stream-end) sleeps with exponential backoff and tries
        // again. Successful connects reset the attempt counter so a flaky
        // link that recovers doesn't keep escalating backoff.
        let state_for_loop = Arc::clone(&state);
        tokio::spawn(async move {
            let parsed = match Url::parse(&url) {
                Ok(u) => u,
                Err(e) => {
                    tracing::error!("NostrRelay subscribe: bad URL: {}", e);
                    state_for_loop(ConnectionEvent::Disconnected(format!("bad URL: {}", e)));
                    return;
                }
            };

            let mut attempt: u32 = 0;
            loop {
                state_for_loop(ConnectionEvent::Connecting);
                match connect_async(parsed.clone()).await {
                    Ok((mut ws, _)) => {
                        attempt = 0;
                        tracing::debug!("NostrRelay: subscribe connected to {}", url);
                        state_for_loop(ConnectionEvent::Connected);

                        // Subscription filter; if the very first send fails
                        // we fall through to the reconnect arm.
                        let req = NostrEvent::subscription_req("phantom-sub-1");
                        if let Err(e) = ws.send(WsMessage::Text(req)).await {
                            tracing::debug!("NostrRelay: send REQ failed: {}", e);
                            state_for_loop(ConnectionEvent::Disconnected(format!("send REQ: {}", e)));
                        } else {
                            // Inner read loop. Exits cleanly on stream-end
                            // (Some(Err) / None) so we can reconnect.
                            let mut exit_reason = String::from("stream ended");
                            while let Some(item) = ws.next().await {
                                match item {
                                    Ok(WsMessage::Text(text)) => {
                                        if let Ok(arr) =
                                            serde_json::from_str::<serde_json::Value>(&text)
                                        {
                                            if arr[0] == "EVENT" {
                                                if let Ok(event) =
                                                    serde_json::from_value::<NostrEvent>(arr[2].clone())
                                                {
                                                    if let Some(env) = event.to_envelope() {
                                                        let _ = handler_tx.send(env);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    Ok(_) => {} // ignore Ping/Pong/Binary
                                    Err(e) => {
                                        exit_reason = format!("ws error: {}", e);
                                        break;
                                    }
                                }
                            }
                            tracing::debug!("NostrRelay: ws exit ({})", exit_reason);
                            state_for_loop(ConnectionEvent::Disconnected(exit_reason));
                        }
                    }
                    Err(e) => {
                        tracing::debug!("NostrRelay: connect failed (attempt {}): {}", attempt, e);
                        state_for_loop(ConnectionEvent::Disconnected(format!("connect: {}", e)));
                    }
                }

                // Exponential backoff with jitter, capped at 60s. Curve:
                // attempt 0 → 1s, 1 → 2s, 2 → 4s, ... 6+ → 60s, all + 0..5s
                // jitter so a fleet doesn't thunder-herd a recovering relay.
                attempt = attempt.saturating_add(1);
                let base = (1u32 << attempt.min(6)).min(60);
                let jitter = rand::random::<u32>() % 5;
                let backoff = base + jitter;
                state_for_loop(ConnectionEvent::Reconnecting { attempt, backoff_secs: backoff });
                tokio::time::sleep(Duration::from_secs(backoff as u64)).await;
            }
        });

        // ── Envelope dispatch task ────────────────────────────────────────
        tokio::spawn(async move {
            while let Some(env) = handler_rx.recv().await {
                envelope_handler(env);
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
    keypair: KeyPair,
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

    async fn subscribe(&self, handler: EnvelopeHandler) -> anyhow::Result<()> {
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

/// Build a `MultiRelay` that fans publish out to every URL and dedupes
/// envelopes coming back from the union of subscribes. Single-URL inputs
/// short-circuit to `make_relay` so the wrapping overhead only kicks in
/// when there's actually redundancy to exploit.
pub fn make_multi_relay(
    urls: &[&str],
    stealth: bool,
    proxy_addr: Option<&str>,
) -> Box<dyn BridgeProvider> {
    if urls.len() == 1 {
        return make_relay(urls[0], stealth, proxy_addr);
    }
    let inners: Vec<Box<dyn BridgeProvider>> = urls
        .iter()
        .map(|u| make_relay(u, stealth, proxy_addr))
        .collect();
    Box::new(MultiRelay::new(inners))
}

// ── MultiRelay ───────────────────────────────────────────────────────────────
//
// Wraps N underlying providers, fans publishes out in parallel, and dedupes
// the union of incoming envelopes via an LRU keyed on `Sha256(env.to_bytes())`.
//
// Dedupe-LRU implementation choice: `HashSet<[u8; 32]>` paired with a
// `VecDeque<[u8; 32]>` of insertion order. O(1) contains, O(1) push, O(1)
// evict-oldest. We avoided `linked-hash-map`/`lru` crates to keep the
// dependency surface minimal — `sha2` + `std::collections` are already pulled
// in by this crate, so no new transitive deps for the dedupe path.
//
// Cap is 4096 entries → ~128 KiB resident (32 B per hash × 2 containers),
// which is the upper bound; on a normal traffic relay we never approach it.

const DEDUPE_CAP: usize = 4096;

struct DedupeLru {
    set: HashSet<[u8; 32]>,
    order: VecDeque<[u8; 32]>,
    cap: usize,
}

impl DedupeLru {
    fn new(cap: usize) -> Self {
        Self {
            set: HashSet::with_capacity(cap),
            order: VecDeque::with_capacity(cap),
            cap,
        }
    }

    /// Returns `true` if this hash is new (handler should fire). `false`
    /// means we've seen it already across some other relay.
    fn insert_if_absent(&mut self, key: [u8; 32]) -> bool {
        if self.set.contains(&key) {
            return false;
        }
        if self.order.len() >= self.cap {
            if let Some(evicted) = self.order.pop_front() {
                self.set.remove(&evicted);
            }
        }
        self.set.insert(key);
        self.order.push_back(key);
        true
    }
}

/// Per-underlying connection state, used to compute the aggregate emitted
/// to the caller's `state_handler`.
#[derive(Debug, Clone)]
enum UnderlyingState {
    Connecting,
    Connected,
    Reconnecting,
    /// Holds the timestamp at which the underlying first reported failed,
    /// so the aggregator can wait 30s before flipping global state to
    /// `Disconnected` (avoids flapping on a single relay's transient).
    Failed(Instant),
}

/// Coarse aggregate state we report up to the application; mirrors the
/// frontend's three-state pill plus `Reconnecting` for tooltip surface.
#[derive(Debug, Clone, PartialEq)]
enum AggregateState {
    Connecting,
    Connected,
    Reconnecting,
    Disconnected,
}

pub struct MultiRelay {
    id: String,
    inners: Vec<Arc<dyn BridgeProvider>>,
}

impl MultiRelay {
    pub fn new(inners: Vec<Box<dyn BridgeProvider>>) -> Self {
        let id = format!("multi:{}", inners.len());
        // Convert Box → Arc so we can share each inner across the per-relay
        // subscribe tasks without cloning their internal state.
        let inners = inners.into_iter().map(Arc::from).collect();
        Self { id, inners }
    }

    fn dedupe_key(env: &Envelope) -> [u8; 32] {
        // Hashes the full padded envelope wire bytes. Identical envelopes
        // sent through two relays will produce identical bytes (the Envelope
        // has no random per-publish field), so the second arrival hits the
        // LRU and is suppressed. `Sha256` is already a transitive dep of
        // this crate via Nostr event-ID computation.
        let bytes = env.to_bytes();
        let digest = Sha256::digest(&bytes);
        let mut out = [0u8; 32];
        out.copy_from_slice(&digest);
        out
    }
}

#[async_trait]
impl BridgeProvider for MultiRelay {
    fn id(&self) -> &str { &self.id }

    /// Fan out to every underlying relay in parallel. Success if AT LEAST
    /// ONE succeeds — only return Err if every underlying failed.
    async fn publish(&self, env: Envelope) -> anyhow::Result<()> {
        let futs = self.inners.iter().map(|r| {
            let r = Arc::clone(r);
            let env = env.clone();
            async move { r.publish(env).await }
        });
        let results = futures::future::join_all(futs).await;
        let mut last_err: Option<String> = None;
        let mut any_ok = false;
        for (i, r) in results.into_iter().enumerate() {
            match r {
                Ok(()) => any_ok = true,
                Err(e) => {
                    tracing::debug!("MultiRelay publish: relay #{} failed: {}", i, e);
                    last_err = Some(format!("relay #{}: {}", i, e));
                }
            }
        }
        if any_ok {
            Ok(())
        } else {
            Err(anyhow::anyhow!(
                "all {} relays failed to publish (last: {})",
                self.inners.len(),
                last_err.unwrap_or_else(|| "no underlying relays".into())
            ))
        }
    }

    async fn subscribe(&self, handler: EnvelopeHandler) -> anyhow::Result<()> {
        self.subscribe_with_state(handler, Box::new(|_| {})).await
    }

    async fn subscribe_with_state(
        &self,
        envelope_handler: EnvelopeHandler,
        state_handler: StateHandler,
    ) -> anyhow::Result<()> {
        let lru = Arc::new(Mutex::new(DedupeLru::new(DEDUPE_CAP)));
        let handler = Arc::new(envelope_handler);
        let state_handler = Arc::new(state_handler);
        let n = self.inners.len();

        // Per-underlying state, all start in `Connecting`.
        let states: Arc<Mutex<Vec<UnderlyingState>>> =
            Arc::new(Mutex::new(vec![UnderlyingState::Connecting; n]));
        // Last aggregate we emitted — guards against double-emit.
        let last_agg: Arc<Mutex<AggregateState>> =
            Arc::new(Mutex::new(AggregateState::Connecting));

        // Emit the initial Connecting once so callers see SOMETHING before
        // the first underlying transitions.
        state_handler(ConnectionEvent::Connecting);

        // Aggregator helper — recompute + emit on any per-relay change.
        let recompute_and_emit = {
            let states = Arc::clone(&states);
            let last_agg = Arc::clone(&last_agg);
            let state_handler = Arc::clone(&state_handler);
            move || {
                let snapshot = states.lock().unwrap().clone();
                let now = Instant::now();
                let mut any_connected = false;
                let mut all_reconnecting = true;
                let mut all_failed_long = true;
                for s in &snapshot {
                    match s {
                        UnderlyingState::Connected => {
                            any_connected = true;
                            all_reconnecting = false;
                            all_failed_long = false;
                        }
                        UnderlyingState::Reconnecting => {
                            all_failed_long = false;
                        }
                        UnderlyingState::Connecting => {
                            all_reconnecting = false;
                            all_failed_long = false;
                        }
                        UnderlyingState::Failed(since) => {
                            all_reconnecting = false;
                            if now.duration_since(*since) < Duration::from_secs(30) {
                                all_failed_long = false;
                            }
                        }
                    }
                }
                let new_agg = if any_connected {
                    AggregateState::Connected
                } else if all_failed_long {
                    AggregateState::Disconnected
                } else if all_reconnecting {
                    AggregateState::Reconnecting
                } else {
                    AggregateState::Connecting
                };

                let mut last = last_agg.lock().unwrap();
                if *last != new_agg {
                    *last = new_agg.clone();
                    let event = match new_agg {
                        AggregateState::Connecting => ConnectionEvent::Connecting,
                        AggregateState::Connected => ConnectionEvent::Connected,
                        AggregateState::Reconnecting => ConnectionEvent::Reconnecting {
                            attempt: 0,
                            backoff_secs: 0,
                        },
                        AggregateState::Disconnected => ConnectionEvent::Disconnected(
                            "all underlying relays down >30s".into(),
                        ),
                    };
                    state_handler(event);
                }
            }
        };
        let recompute_and_emit = Arc::new(recompute_and_emit);

        // Spawn a subscribe task per underlying relay.
        for (idx, inner) in self.inners.iter().enumerate() {
            let inner = Arc::clone(inner);
            let lru = Arc::clone(&lru);
            let handler = Arc::clone(&handler);
            let states = Arc::clone(&states);
            let recompute = Arc::clone(&recompute_and_emit);

            // Per-relay envelope handler: dedupe before invoking shared cb.
            let dedupe_handler: EnvelopeHandler = {
                let lru = Arc::clone(&lru);
                let handler = Arc::clone(&handler);
                Box::new(move |env: Envelope| {
                    let key = MultiRelay::dedupe_key(&env);
                    let fresh = {
                        let mut g = lru.lock().unwrap();
                        g.insert_if_absent(key)
                    };
                    if fresh {
                        handler(env);
                    } else {
                        tracing::trace!("MultiRelay: dedup hit, suppressing duplicate");
                    }
                })
            };

            // Per-relay state handler: update slot + recompute aggregate.
            let per_state_handler: StateHandler = {
                let states = Arc::clone(&states);
                let recompute = Arc::clone(&recompute);
                Box::new(move |ev: ConnectionEvent| {
                    {
                        let mut s = states.lock().unwrap();
                        if let Some(slot) = s.get_mut(idx) {
                            *slot = match &ev {
                                ConnectionEvent::Connecting => UnderlyingState::Connecting,
                                ConnectionEvent::Connected => UnderlyingState::Connected,
                                ConnectionEvent::Reconnecting { .. } => {
                                    UnderlyingState::Reconnecting
                                }
                                ConnectionEvent::Disconnected(_) => {
                                    // Preserve original `Failed` instant if
                                    // we were already failed — we only want
                                    // to start the 30s clock on the FIRST
                                    // failure, not reset it on every retry.
                                    match slot {
                                        UnderlyingState::Failed(t) => {
                                            UnderlyingState::Failed(*t)
                                        }
                                        _ => UnderlyingState::Failed(Instant::now()),
                                    }
                                }
                            };
                        }
                    }
                    recompute();
                })
            };

            // Detached: a failing single subscribe shouldn't block us from
            // keeping the others alive. The inner provider is responsible
            // for its own reconnect (NostrRelay does, InMemory doesn't need
            // to, Stealth deliberately doesn't per task brief).
            tokio::spawn(async move {
                if let Err(e) = inner
                    .subscribe_with_state(dedupe_handler, per_state_handler)
                    .await
                {
                    tracing::error!(
                        "MultiRelay: relay #{} subscribe_with_state error: {}",
                        idx,
                        e
                    );
                }
            });
            // Suppress unused-warning if `states` isn't otherwise used here.
            let _ = &states;
        }

        Ok(())
    }

    /// Aggregate health: latency = min, uptime = max, failure_rate = min
    /// (i.e. report the "best" underlying — what the user effectively
    /// experiences, since we only need ONE healthy relay).
    async fn health(&self) -> BridgeHealth {
        let healths = futures::future::join_all(self.inners.iter().map(|r| {
            let r = Arc::clone(r);
            async move { r.health().await }
        }))
        .await;
        if healths.is_empty() {
            return BridgeHealth { latency_ms: u32::MAX, uptime: 0.0, failure_rate: 1.0 };
        }
        let latency_ms = healths.iter().map(|h| h.latency_ms).min().unwrap_or(u32::MAX);
        let uptime = healths.iter().map(|h| h.uptime).fold(0.0_f32, f32::max);
        let failure_rate = healths
            .iter()
            .map(|h| h.failure_rate)
            .fold(1.0_f32, f32::min);
        BridgeHealth { latency_ms, uptime, failure_rate }
    }
}

// ── Stealth Cover Traffic Consumer ───────────────────────────────────────────

