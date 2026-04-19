//! Phantom address — the public identity handed out to contacts.
//!
//! An address bundles the two public keys needed to reach a recipient:
//!
//! - `view_pub`  — lets the sender build the stealth tag
//! - `spend_pub` — derives the envelope encryption key and serves as the
//!                 bootstrap peer-ratchet public for session initialisation
//!
//! Wire format: `"phantom:<view_pub_hex>:<spend_pub_hex>"`. The
//! `phantom:` prefix is optional when parsing so short form addresses copied
//! out of `phantom pair` output still work.

use serde::{Deserialize, Serialize};
use std::fmt;
use x25519_dalek::PublicKey;

/// Public, shareable identifier of a PhantomChat recipient.
#[derive(Clone, Copy, Debug, Eq, Serialize, Deserialize)]
pub struct PhantomAddress {
    pub view_pub: [u8; 32],
    pub spend_pub: [u8; 32],
}

impl PhantomAddress {
    pub fn new(view_pub: PublicKey, spend_pub: PublicKey) -> Self {
        Self {
            view_pub: *view_pub.as_bytes(),
            spend_pub: *spend_pub.as_bytes(),
        }
    }

    pub fn view_pub(&self) -> PublicKey {
        PublicKey::from(self.view_pub)
    }

    pub fn spend_pub(&self) -> PublicKey {
        PublicKey::from(self.spend_pub)
    }

    /// Parse `"phantom:<hex64>:<hex64>"` or just `"<hex64>:<hex64>"`.
    pub fn parse(s: &str) -> Option<Self> {
        let raw = s.strip_prefix("phantom:").unwrap_or(s);
        let (view_hex, spend_hex) = raw.split_once(':')?;

        let view: [u8; 32]  = hex::decode(view_hex).ok()?.try_into().ok()?;
        let spend: [u8; 32] = hex::decode(spend_hex).ok()?.try_into().ok()?;
        Some(Self { view_pub: view, spend_pub: spend })
    }

    /// Short, stable identifier for indexing session state (first 8 bytes of
    /// the spend-pub is plenty to disambiguate contacts inside a single
    /// user's address book and keeps on-disk session files compact).
    pub fn short_id(&self) -> String {
        hex::encode(&self.spend_pub[..8])
    }
}

impl PartialEq for PhantomAddress {
    fn eq(&self, other: &Self) -> bool {
        self.view_pub == other.view_pub && self.spend_pub == other.spend_pub
    }
}

impl std::hash::Hash for PhantomAddress {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.view_pub.hash(state);
        self.spend_pub.hash(state);
    }
}

impl fmt::Display for PhantomAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "phantom:{}:{}",
            hex::encode(self.view_pub),
            hex::encode(self.spend_pub)
        )
    }
}
