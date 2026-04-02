use crate::storage;
use crate::network::{run_swarm, NetworkCommand, NetworkEvent, PhantomBehaviour};
use aes_gcm::{aead::{Aead, KeyInit}, Aes256Gcm, Nonce};
use flutter_rust_bridge::frb;
use libp2p::{identity, PeerId, SwarmBuilder};
use rand_core::{OsRng, RngCore};
use std::sync::OnceLock;
use tokio::sync::mpsc;
use std::time::Duration;
use std::path::PathBuf;
use serde::{Deserialize, Serialize};

static COMMAND_TX: OnceLock<mpsc::Sender<NetworkCommand>> = OnceLock::new();

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

#[frb(sync)]
pub fn start_network_node(sink: flutter_rust_bridge::StreamSink<NetworkEvent>, avatar_cid: Option<String>) {
    let (tx, rx) = mpsc::channel(32);
    let _ = COMMAND_TX.set(tx.clone());

    tokio::spawn(async move {
        let local_key = identity::Keypair::generate_ed25519();
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

        let _ = sink.add(NetworkEvent::NodeStarted { peer_id: local_peer_id.to_string() });
        let _ = swarm.behaviour_mut().gossipsub.subscribe(&libp2p::gossipsub::IdentTopic::new("phantom-chat"));

        let _ = tx.send(NetworkCommand::PublishIdentity { 
            phantom_id: "UNKNOWN".to_string(), 
            avatar_cid 
        }).await;

        if let Err(e) = run_swarm(swarm, rx, sink.clone()).await {
             let _ = sink.add(NetworkEvent::Error { message: e.to_string() });
        }
    });
}

pub async fn send_secure_message(target_peer_id: String, phantom_id: String, message: String) {
    if let Some(tx) = COMMAND_TX.get() {
        let mut key_bytes = [0u8; 32];
        let id_bytes = phantom_id.as_bytes();
        for (i, b) in id_bytes.iter().enumerate().take(32) { key_bytes[i] = *b; }
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
            avatar_cid: Some(cid) 
        }).await;
     }
}

pub async fn perform_panic_wipe(db_path: String) {
    storage::panic_wipe(PathBuf::from(db_path));
}
