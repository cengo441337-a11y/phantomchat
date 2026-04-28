//! Phantom address — the public identity handed out to contacts.
//!
//! An address bundles the public keys needed to reach a recipient:
//!
//! - `view_pub`  — lets the sender build the stealth tag
//! - `spend_pub` — derives the envelope encryption key (classic) and serves
//!   as the bootstrap peer-ratchet public for session init
//! - `mlkem_pub` — optional ML-KEM-1024 public key; presence triggers the
//!   PQXDH-hybrid envelope path
//!
//! Wire format:
//! - **Classic**: `"phantom:<view_hex>:<spend_hex>"` (32-byte keys)
//! - **Hybrid** : `"phantomx:<view_hex>:<spend_hex>:<mlkem_b64>"`
//!   where `mlkem_b64` is url-safe base64 of the 1568-byte ML-KEM pub key
//!   (hex would be 3136 chars — base64 keeps it at ~2090 and it's still
//!   copy-pastable as one token).
//!
//! Parsing accepts both `phantom:` and `phantomx:` prefixes as well as a
//! raw `view:spend[:mlkem]` shortcut — so strings copied out of `phantom
//! pair` continue to work.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
use serde::{Deserialize, Serialize};
use std::fmt;
use x25519_dalek::PublicKey;

/// Public, shareable identifier of a PhantomChat recipient.
#[derive(Clone, Debug, Eq, Serialize, Deserialize)]
pub struct PhantomAddress {
    pub view_pub: [u8; 32],
    pub spend_pub: [u8; 32],
    /// 1568-byte ML-KEM-1024 public key. When `Some`, this address triggers
    /// the PQXDH-hybrid envelope path.
    #[serde(default)]
    pub mlkem_pub: Option<Vec<u8>>,
}

impl PhantomAddress {
    /// Classic address (X25519-only).
    pub fn new(view_pub: PublicKey, spend_pub: PublicKey) -> Self {
        Self {
            view_pub: *view_pub.as_bytes(),
            spend_pub: *spend_pub.as_bytes(),
            mlkem_pub: None,
        }
    }

    /// Hybrid address carrying an ML-KEM-1024 public key.
    pub fn new_hybrid(
        view_pub: PublicKey,
        spend_pub: PublicKey,
        mlkem_pub_bytes: Vec<u8>,
    ) -> Self {
        Self {
            view_pub: *view_pub.as_bytes(),
            spend_pub: *spend_pub.as_bytes(),
            mlkem_pub: Some(mlkem_pub_bytes),
        }
    }

    pub fn view_pub(&self) -> PublicKey {
        PublicKey::from(self.view_pub)
    }

    pub fn spend_pub(&self) -> PublicKey {
        PublicKey::from(self.spend_pub)
    }

    pub fn is_hybrid(&self) -> bool {
        self.mlkem_pub.is_some()
    }

    /// Parse classic (`phantom:`) or hybrid (`phantomx:`) wire forms.
    pub fn parse(s: &str) -> Option<Self> {
        // Hybrid always starts with `phantomx:` (or four-part raw).
        let trimmed = s.trim();
        if let Some(rest) = trimmed.strip_prefix("phantomx:") {
            return parse_hybrid_parts(rest);
        }

        let raw = trimmed.strip_prefix("phantom:").unwrap_or(trimmed);
        // Sniff: 3 colon-separated parts → hybrid raw, 2 parts → classic raw.
        let parts: Vec<&str> = raw.splitn(3, ':').collect();
        match parts.len() {
            2 => parse_classic_parts(parts[0], parts[1]),
            3 => parse_hybrid_parts(raw),
            _ => None,
        }
    }

    /// Short, stable identifier for indexing session state.
    pub fn short_id(&self) -> String {
        hex::encode(&self.spend_pub[..8])
    }
}

fn parse_classic_parts(view_hex: &str, spend_hex: &str) -> Option<PhantomAddress> {
    let view: [u8; 32]  = hex::decode(view_hex).ok()?.try_into().ok()?;
    let spend: [u8; 32] = hex::decode(spend_hex).ok()?.try_into().ok()?;
    Some(PhantomAddress { view_pub: view, spend_pub: spend, mlkem_pub: None })
}

fn parse_hybrid_parts(raw: &str) -> Option<PhantomAddress> {
    let (view_hex, rest) = raw.split_once(':')?;
    let (spend_hex, mlkem_b64) = rest.split_once(':')?;

    let view: [u8; 32]  = hex::decode(view_hex).ok()?.try_into().ok()?;
    let spend: [u8; 32] = hex::decode(spend_hex).ok()?.try_into().ok()?;
    let mlkem = B64.decode(mlkem_b64.as_bytes()).ok()?;
    if mlkem.len() != 1568 {
        return None;
    }
    Some(PhantomAddress {
        view_pub: view,
        spend_pub: spend,
        mlkem_pub: Some(mlkem),
    })
}

impl PartialEq for PhantomAddress {
    fn eq(&self, other: &Self) -> bool {
        self.view_pub == other.view_pub
            && self.spend_pub == other.spend_pub
            && self.mlkem_pub == other.mlkem_pub
    }
}

impl std::hash::Hash for PhantomAddress {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.view_pub.hash(state);
        self.spend_pub.hash(state);
        self.mlkem_pub.hash(state);
    }
}

impl fmt::Display for PhantomAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.mlkem_pub {
            None => write!(
                f,
                "phantom:{}:{}",
                hex::encode(self.view_pub),
                hex::encode(self.spend_pub)
            ),
            Some(mlkem) => write!(
                f,
                "phantomx:{}:{}:{}",
                hex::encode(self.view_pub),
                hex::encode(self.spend_pub),
                B64.encode(mlkem),
            ),
        }
    }
}
