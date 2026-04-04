use crate::cover_traffic::{CoverTrafficGenerator, CoverTrafficMode};
use crate::envelope::Envelope;
use crate::privacy::{PrivacyConfig, PrivacyMode, ProxyConfig, ProxyKind};
use crate::storage;
use crate::frb_generated::StreamSink;
use crate::network::{run_swarm, NetworkCommand, NetworkEvent, PhantomBehaviour};
use aes_gcm::{aead::{Aead, KeyInit}, Aes256Gcm, Nonce};
use flutter_rust_bridge::frb;
use libp2p::{identity, PeerId, SwarmBuilder};
use rand_core::{OsRng, RngCore};
use std::sync::{OnceLock, RwLock, Mutex};
use tokio::sync::mpsc;
use std::time::Duration;
use std::path::PathBuf;
use serde::{Deserialize, Serialize};

// ── global state ──────────────────────────────────────────────────────────────

static COMMAND_TX: OnceLock<mpsc::Sender<NetworkCommand>> = OnceLock::new();
static PRIVACY_CONFIG: OnceLock<RwLock<PrivacyConfig>> = OnceLock::new();
static COVER_TX: OnceLock<mpsc::Sender<Envelope>> = OnceLock::new();

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

pub async fn send_secure_message(target_peer_id: String, phantom_id: String, message: String) {
    if let Some(tx) = COMMAND_TX.get() {
        let mut key_bytes = [0u8; 32];
        for (i, b) in phantom_id.as_bytes().iter().enumerate().take(32) {
            key_bytes[i] = *b;
        }
        let cipher = Aes256Gcm::new_from_slice(&key_bytes).unwrap();
        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        if let Ok(ciphertext) = cipher.encrypt(nonce, message.as_ref()) {
            let _ = storage::save_message(&target_peer_id, &message);
            let physical_msg = PhysicalSecureMessage { ciphertext, nonce: nonce_bytes, sender_id: phantom_id };
            let json_msg = serde_json::to_string(&physical_msg).unwrap();
            let _ = tx.send(NetworkCommand::SendMessage { target_peer_id, message: json_msg }).await;
        }
    }
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
