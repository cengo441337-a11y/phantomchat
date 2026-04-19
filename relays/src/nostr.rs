//! NIP-01 Nostr event types and signing for anonymous PhantomChat relay transport.
//!
//! PhantomChat uses Nostr relays as dumb message stores. Each envelope is posted
//! as a Nostr event with kind 1984 (application-specific ephemeral data).
//! A fresh secp256k1 keypair is generated per publish so posts are unlinkable.

use serde::{Deserialize, Serialize};
use sha2::{Sha256, Digest};
use secp256k1::{Secp256k1, SecretKey, Message};
use rand::rngs::OsRng;

/// Nostr event kind used for PhantomChat envelopes.
/// 1984 = application-specific ephemeral event (NIP-78 style).
pub const PHANTOM_KIND: u64 = 1984;

/// NIP-01 Nostr event.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Creates and signs a PhantomChat envelope event.
    /// Uses a fresh ephemeral keypair — publish is anonymous and unlinkable.
    pub fn new_phantom(envelope_hex: &str, created_at: u64) -> anyhow::Result<Self> {
        let secp = Secp256k1::new();
        let (secret_key, public_key) = secp.generate_keypair(&mut OsRng);

        // pubkey = x-only (32 bytes hex) per NIP-01
        let pubkey = hex::encode(public_key.x_only_public_key().0.serialize());

        // NIP-01: id = SHA-256 of canonical JSON
        let empty_tags: Vec<Vec<String>> = Vec::new();
        let id_preimage = serde_json::json!([
            0,
            pubkey,
            created_at,
            PHANTOM_KIND,
            empty_tags,
            envelope_hex,
        ])
        .to_string();

        let id = hex::encode(Sha256::digest(id_preimage.as_bytes()));

        // Sign id bytes
        let id_bytes = hex::decode(&id)?;
        let msg = Message::from_slice(&id_bytes)
            .map_err(|_| anyhow::anyhow!("id not 32 bytes"))?;
        let sig = secp.sign_schnorr(&msg, &secret_key.keypair(&secp));
        let sig_hex = hex::encode(sig.as_ref());

        Ok(Self {
            id,
            pubkey,
            created_at,
            kind: PHANTOM_KIND,
            tags: vec![],
            content: envelope_hex.to_string(),
            sig: sig_hex,
        })
    }

    /// NIP-01 ["EVENT", <event>] publish command.
    pub fn to_publish_msg(&self) -> String {
        serde_json::json!(["EVENT", self]).to_string()
    }
}

/// NIP-01 subscription filter for PhantomChat events.
#[derive(Debug, Serialize, Deserialize)]
pub struct NostrFilter {
    pub kinds: Vec<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
}

impl NostrFilter {
    /// Subscribe to all PhantomChat envelopes since a timestamp.
    pub fn phantom_since(since: u64) -> Self {
        Self { kinds: vec![PHANTOM_KIND], since: Some(since), limit: Some(500) }
    }

    /// ["REQ", sub_id, filter] subscription command.
    pub fn to_req_msg(&self, sub_id: &str) -> String {
        serde_json::json!(["REQ", sub_id, self]).to_string()
    }
}

/// Incoming relay message variants (NIP-01).
#[derive(Debug)]
pub enum RelayMsg {
    Event { sub_id: String, event: NostrEvent },
    Eose { sub_id: String },
    Notice(String),
    Ok { event_id: String, accepted: bool, message: String },
}

impl RelayMsg {
    pub fn parse(raw: &str) -> Option<Self> {
        let v: serde_json::Value = serde_json::from_str(raw).ok()?;
        let arr = v.as_array()?;
        match arr.first()?.as_str()? {
            "EVENT" => {
                let sub_id = arr.get(1)?.as_str()?.to_string();
                let event: NostrEvent = serde_json::from_value(arr.get(2)?.clone()).ok()?;
                Some(Self::Event { sub_id, event })
            }
            "EOSE" => Some(Self::Eose { sub_id: arr.get(1)?.as_str()?.to_string() }),
            "NOTICE" => Some(Self::Notice(arr.get(1)?.as_str()?.to_string())),
            "OK" => Some(Self::Ok {
                event_id: arr.get(1)?.as_str()?.to_string(),
                accepted: arr.get(2)?.as_bool().unwrap_or(false),
                message: arr.get(3).and_then(|v| v.as_str()).unwrap_or("").to_string(),
            }),
            _ => None,
        }
    }
}
