use crate::address::PhantomAddress;
use crate::cover_traffic::{CoverTrafficGenerator, CoverTrafficMode};
use crate::envelope::Envelope;
use crate::keys::{SpendKey, ViewKey};
use crate::privacy::{PrivacyConfig, PrivacyMode, ProxyConfig, ProxyKind};
use crate::session::SessionStore;
use crate::storage;
use crate::frb_generated::StreamSink;
use crate::network::{run_swarm, NetworkCommand, NetworkEvent, PhantomBehaviour};
use flutter_rust_bridge::frb;
use libp2p::{identity, PeerId, SwarmBuilder};
use rand_core::{OsRng, RngCore};
use std::sync::{OnceLock, RwLock, Mutex};
use tokio::sync::mpsc;
use std::time::Duration;
use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use x25519_dalek::{PublicKey, StaticSecret};

// ── global state ──────────────────────────────────────────────────────────────

static COMMAND_TX: OnceLock<mpsc::Sender<NetworkCommand>> = OnceLock::new();
static PRIVACY_CONFIG: OnceLock<RwLock<PrivacyConfig>> = OnceLock::new();
static COVER_TX: OnceLock<mpsc::Sender<Envelope>> = OnceLock::new();

/// The long-term keypair of the local user. Set once during
/// [`start_network_node`]; required by [`send_secure_message`] and
/// [`scan_incoming_envelope`] so they can drive the Envelope + Ratchet
/// pipeline without asking the Flutter layer for secrets on every call.
static LOCAL_VIEW:  OnceLock<RwLock<Option<ViewKey>>>  = OnceLock::new();
static LOCAL_SPEND: OnceLock<RwLock<Option<SpendKey>>> = OnceLock::new();

/// Per-peer Double-Ratchet session state. Persisted through the same FFI
/// entry-points Flutter already calls when a chat screen opens or closes.
static SESSIONS: OnceLock<Mutex<SessionStore>> = OnceLock::new();

fn sessions() -> &'static Mutex<SessionStore> {
    SESSIONS.get_or_init(|| Mutex::new(SessionStore::new()))
}

/// In MaximumStealth mode cover dummies are routed here instead of into libp2p.
/// The relays crate calls `take_stealth_cover_rx()` once at startup to consume them.
static STEALTH_COVER_RX: OnceLock<Mutex<Option<mpsc::Receiver<Vec<u8>>>>> = OnceLock::new();
static STEALTH_COVER_TX: OnceLock<mpsc::Sender<Vec<u8>>> = OnceLock::new();

fn privacy_config() -> &'static RwLock<PrivacyConfig> {
    PRIVACY_CONFIG.get_or_init(|| RwLock::new(PrivacyConfig::default()))
}

/// Called by the relays crate to obtain the stealth-mode cover traffic stream.
/// Returns `None` if already taken or P2P mode is active.
pub fn take_stealth_cover_rx() -> Option<mpsc::Receiver<Vec<u8>>> {
    STEALTH_COVER_RX.get()?.lock().ok()?.take()
}

// ── types ─────────────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PhysicalSecureMessage {
    pub ciphertext: Vec<u8>,
    pub nonce: [u8; 12],
    pub sender_id: String,
}

pub struct IdentityKeys {
    pub kyber_pk: Vec<u8>,
    pub x25519_pk: Vec<u8>,
}

// ── public API ────────────────────────────────────────────────────────────────

pub async fn init_secure_storage(db_path: String, password: String) -> String {
    match storage::init_db(PathBuf::from(db_path), &password) {
        Ok(_) => "SUCCESS: SECURE CORE INITIALIZED".to_string(),
        Err(e) => format!("ERROR: SECURE CORE FAILURE: {}", e),
    }
}

pub fn generate_phantom_id() -> String {
    let mut id_bytes = [0u8; 32];
    OsRng.fill_bytes(&mut id_bytes);
    format!("PH-{}", bs58::encode(id_bytes).into_string())
}

/// Switch the privacy mode at runtime.
///
/// `mode_str`  — `"daily"` | `"stealth"`
/// `proxy_addr` — optional SOCKS5, e.g. `"127.0.0.1:9050"` (Tor) or `"127.0.0.1:1080"` (Nym)
/// `use_nym`   — `true` → Nym, `false` → Tor
#[frb(sync)]
pub fn set_privacy_mode(mode_str: String, proxy_addr: Option<String>, use_nym: bool) -> String {
    let mode = match mode_str.to_lowercase().as_str() {
        "daily" | "dailyuse" => PrivacyMode::DailyUse,
        "stealth" | "maximumstealth" | "paranoia" => PrivacyMode::MaximumStealth,
        other => return format!("ERROR: unknown mode '{}'", other),
    };

    let proxy = ProxyConfig {
        addr: proxy_addr.unwrap_or_else(|| {
            if use_nym { "127.0.0.1:1080".to_string() } else { "127.0.0.1:9050".to_string() }
        }),
        kind: if use_nym { ProxyKind::Nym } else { ProxyKind::Tor },
    };

    if let Ok(mut cfg) = privacy_config().write() {
        cfg.mode = mode.clone();
        cfg.proxy = proxy;
        tracing::info!("Privacy mode changed to {:?}", mode);
    }

    // Restart cover traffic generator at the new intensity.
    if let Some(tx) = COVER_TX.get() {
        let ct_mode = match mode {
            PrivacyMode::DailyUse => CoverTrafficMode::Light,
            PrivacyMode::MaximumStealth => CoverTrafficMode::Aggressive,
        };
        CoverTrafficGenerator::new(ct_mode, tx.clone()).start();
    }

    format!("OK: privacy mode set to {:?}", mode)
}

/// Returns the currently active privacy mode as a string (`"DailyUse"` or `"MaximumStealth"`).
#[frb(sync)]
pub fn get_privacy_mode() -> String {
    privacy_config()
        .read()
        .map(|c| format!("{:?}", c.mode))
        .unwrap_or_else(|_| "DailyUse".to_string())
}

#[frb(sync)]
pub fn start_network_node(sink: StreamSink<NetworkEvent>, avatar_cid: Option<String>) {
    let (cmd_tx, cmd_rx) = mpsc::channel::<NetworkCommand>(32);
    let _ = COMMAND_TX.set(cmd_tx.clone());

    // Cover traffic channel — shared with the CoverTrafficGenerator.
    let (cover_tx, cover_rx) = mpsc::channel::<Envelope>(64);
    let _ = COVER_TX.set(cover_tx.clone());

    let p2p_enabled = privacy_config()
        .read()
        .map(|c| c.p2p_enabled())
        .unwrap_or(true);

    let ct_mode = privacy_config()
        .read()
        .map(|c| match c.mode {
            PrivacyMode::DailyUse => CoverTrafficMode::Light,
            PrivacyMode::MaximumStealth => CoverTrafficMode::Aggressive,
        })
        .unwrap_or(CoverTrafficMode::Light);

    CoverTrafficGenerator::new(ct_mode, cover_tx).start();

    if p2p_enabled {
        // ── DailyUse: bridge cover_rx → PublishRaw into the P2P swarm ──────────
        let bridge_cmd_tx = cmd_tx.clone();
        tokio::spawn(async move {
            let mut rx = cover_rx;
            while let Some(dummy) = rx.recv().await {
                let _ = bridge_cmd_tx.send(NetworkCommand::PublishRaw {
                    topic: "phantom-chat".to_string(),
                    data: dummy.to_bytes(),
                }).await;
            }
        });

        let sink_clone = sink.clone();
        tokio::spawn(async move {
            let local_key     = identity::Keypair::generate_ed25519();
            let local_peer_id = PeerId::from(local_key.public());

            let mut swarm = SwarmBuilder::with_existing_identity(local_key)
                .with_tokio()
                .with_tcp(
                    libp2p::tcp::Config::default(),
                    libp2p::noise::Config::new,
                    libp2p::yamux::Config::default,
                ).unwrap()
                .with_behaviour(|key| {
                    Ok(PhantomBehaviour {
                        gossipsub: libp2p::gossipsub::Behaviour::new(
                            libp2p::gossipsub::MessageAuthenticity::Signed(key.clone()),
                            libp2p::gossipsub::Config::default(),
                        ).unwrap(),
                        kademlia: libp2p::kad::Behaviour::new(
                            local_peer_id,
                            libp2p::kad::store::MemoryStore::new(local_peer_id),
                        ),
                    })
                }).unwrap()
                .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(60)))
                .build();

            let _ = sink_clone.add(NetworkEvent::NodeStarted {
                peer_id: local_peer_id.to_string(),
            });
            let _ = swarm.behaviour_mut().gossipsub.subscribe(
                &libp2p::gossipsub::IdentTopic::new("phantom-chat")
            );
            let _ = cmd_tx.send(NetworkCommand::PublishIdentity {
                phantom_id: "UNKNOWN".to_string(),
                avatar_cid,
            }).await;

            if let Err(e) = run_swarm(swarm, cmd_rx, sink_clone.clone()).await {
                let _ = sink_clone.add(NetworkEvent::Error { message: e.to_string() });
            }
        });
    } else {
        // ── MaximumStealth: P2P disabled — route cover dummies to relays crate ──
        tracing::info!("Maximum Stealth mode: libp2p disabled, relay-only transport active");

        let (stealth_tx, stealth_rx) = mpsc::channel::<Vec<u8>>(64);
        let _ = STEALTH_COVER_TX.set(stealth_tx);
        STEALTH_COVER_RX.get_or_init(|| Mutex::new(Some(stealth_rx)));

        // Bridge: cover_rx → stealth channel (raw bytes for relay publish)
        let stealth_sender = STEALTH_COVER_TX.get().cloned();
        tokio::spawn(async move {
            let mut rx = cover_rx;
            while let Some(dummy) = rx.recv().await {
                if let Some(ref tx) = stealth_sender {
                    let _ = tx.send(dummy.to_bytes()).await;
                }
            }
        });

        let _ = sink.add(NetworkEvent::NodeStarted { peer_id: "relay-only".to_string() });
    }
}

/// Import the caller's long-term identity into the Rust side so the
/// envelope pipeline can sign/scan on their behalf.
///
/// `view_secret_hex` and `spend_secret_hex` are 64-char hex encodings of
/// the 32-byte X25519 secrets written by `PhantomIdentity` on the Dart side.
pub fn load_local_identity(
    view_secret_hex: String,
    spend_secret_hex: String,
) -> String {
    let parse = |s: &str| -> Option<StaticSecret> {
        let bytes: [u8; 32] = hex::decode(s).ok()?.try_into().ok()?;
        Some(StaticSecret::from(bytes))
    };
    let view_secret  = match parse(&view_secret_hex)  {
        Some(s) => s, None => return "ERROR: view_secret parse".to_string(),
    };
    let spend_secret = match parse(&spend_secret_hex) {
        Some(s) => s, None => return "ERROR: spend_secret parse".to_string(),
    };

    let view_key  = ViewKey  { public: PublicKey::from(&view_secret),  secret: view_secret  };
    let spend_key = SpendKey { public: PublicKey::from(&spend_secret), secret: spend_secret };

    let _ = LOCAL_VIEW
        .get_or_init(|| RwLock::new(None))
        .write()
        .map(|mut g| *g = Some(view_key));
    let _ = LOCAL_SPEND
        .get_or_init(|| RwLock::new(None))
        .write()
        .map(|mut g| *g = Some(spend_key));

    "OK: identity loaded".to_string()
}

/// Send a real encrypted message.
///
/// Replaces the legacy AES-GCM demo pipeline. Takes a PhantomChat address
/// (`phantom:view:spend` or `view:spend`), wraps the plaintext in a
/// [`SessionStore::send`] → [`Envelope`] pair, and hands the serialised
/// bytes off to the network layer for publishing.
///
/// The 3-parameter signature is kept for backwards-compat with the
/// auto-generated Flutter bridge (`flutter_rust_bridge`) — the middle
/// `_local_phantom_id` argument is no longer used internally but removing
/// it would require regenerating `frb_generated.rs` from a machine that
/// has the Flutter toolchain installed.
pub async fn send_secure_message(
    recipient_address: String,
    _local_phantom_id: String,
    plaintext: String,
) -> String {
    let recipient = match PhantomAddress::parse(&recipient_address) {
        Some(a) => a,
        None => return "ERROR: invalid recipient address".to_string(),
    };

    let envelope = {
        let mut guard = match sessions().lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        guard.send(&recipient, plaintext.as_bytes(), 16)
    };

    let preview: String = plaintext.chars().take(64).collect();
    let _ = storage::save_message(&recipient.short_id(), &preview);

    let wire = envelope.to_bytes();
    if let Some(tx) = COMMAND_TX.get() {
        let _ = tx.send(NetworkCommand::PublishRaw {
            topic: "phantom-chat".to_string(),
            data: wire,
        }).await;
    }
    "OK: envelope sealed + published".to_string()
}

/// Try to decrypt an incoming envelope using the loaded local identity.
///
/// Called by the network listener for every envelope the node observes.
/// Returns:
/// - `Some(plaintext)` when the envelope is ours and decrypts cleanly
/// - `None` when it belongs to someone else (cover traffic, other peer)
pub fn scan_incoming_envelope(wire_bytes: Vec<u8>) -> Option<Vec<u8>> {
    let envelope = Envelope::from_bytes(&wire_bytes)?;

    let view_guard  = LOCAL_VIEW.get()?.read().ok()?;
    let spend_guard = LOCAL_SPEND.get()?.read().ok()?;
    let view_key  = view_guard.as_ref()?;
    let spend_key = spend_guard.as_ref()?;

    let mut guard = sessions().lock().ok()?;
    guard.receive(&envelope, view_key, spend_key).ok().flatten()
}

pub async fn join_group(group_id: String) {
    if let Some(tx) = COMMAND_TX.get() {
        let _ = tx.send(NetworkCommand::JoinGroup { group_id }).await;
    }
}

pub async fn send_group_message(group_id: String, message: String) {
    if let Some(tx) = COMMAND_TX.get() {
        let _ = tx.send(NetworkCommand::SendGroupMessage { group_id, message }).await;
    }
}

pub async fn update_avatar_cid(cid: String) {
    if let Some(tx) = COMMAND_TX.get() {
        let _ = tx.send(NetworkCommand::PublishIdentity {
            phantom_id: "UNKNOWN".to_string(),
            avatar_cid: Some(cid),
        }).await;
    }
}

pub async fn perform_panic_wipe(db_path: String) {
    storage::panic_wipe(PathBuf::from(db_path));
}
