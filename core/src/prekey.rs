//! X3DH-style prekey bundle.
//!
//! A recipient pre-publishes a small bundle so senders can run the initial
//! handshake without an interactive back-and-forth:
//!
//! - `identity_pub` — long-term Ed25519 signing key (stable across
//!   devices, used to sign the rotating prekey).
//! - `signed_prekey` — short-lived X25519 key signed by the identity key.
//!   Rotate this every ~week to bound exposure if a prekey leaks.
//! - `one_time_prekey` — optional X25519 key consumed exactly once. If the
//!   recipient has a pool of these, each new sender picks a fresh one;
//!   this gives stronger forward secrecy but runs out after N conversations
//!   unless replenished.
//!
//! ## Signature chain
//!
//! `identity_pub.sign(signed_prekey.public)` is stored in `signature`. A
//! sender verifies that before doing the ECDH. If the bundle signature
//! fails to verify, the sender refuses to send — an attacker could
//! otherwise swap the prekey for one they control.
//!
//! ## Wire format
//!
//! Serialised via `serde_json` so bundles can live in any transport — a
//! Nostr-kind event, an HTTP directory, a QR code, a Nostr NIP-05 entry.
//! A canonical JSON dump is also hex-encodable for QR.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use x25519_dalek::{PublicKey, StaticSecret};

use crate::keys::{verify_ed25519, PhantomSigningKey};

/// A rotating X25519 prekey signed by the identity key.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignedPrekey {
    /// Monotonically-increasing integer so stale bundles can be detected.
    pub id: u64,
    /// 32-byte X25519 public key, hex-encoded for JSON portability.
    pub public: String,
    /// 64-byte Ed25519 signature over the raw `public` bytes, b64.
    pub signature: String,
}

impl SignedPrekey {
    /// Generate + sign a fresh signed prekey. Returns the public bundle
    /// *and* the matching X25519 secret (the recipient holds onto the
    /// secret and exposes only the public half).
    pub fn generate(id: u64, identity: &PhantomSigningKey) -> (Self, StaticSecret) {
        let secret = StaticSecret::random_from_rng(&mut OsRng);
        let public = PublicKey::from(&secret);
        let signature = identity.sign(public.as_bytes());
        (
            Self {
                id,
                public: hex::encode(public.as_bytes()),
                signature: B64.encode(signature),
            },
            secret,
        )
    }

    /// Parse the hex-encoded public field back into the 32-byte X25519 key.
    pub fn public_bytes(&self) -> Option<[u8; 32]> {
        hex::decode(&self.public).ok()?.try_into().ok()
    }

    /// Parse the b64-encoded signature field.
    pub fn signature_bytes(&self) -> Option<[u8; 64]> {
        B64.decode(&self.signature).ok()?.try_into().ok()
    }

    /// Verify the signature chains back to the given identity public.
    pub fn verify(&self, identity_pub: &[u8; 32]) -> bool {
        let Some(pub_bytes) = self.public_bytes() else { return false; };
        let Some(sig) = self.signature_bytes() else { return false; };
        verify_ed25519(identity_pub, &pub_bytes, &sig)
    }
}

/// Single-use prekey. Consumed by the sender during the initial handshake
/// and dropped from the receiver's pool immediately.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OneTimePrekey {
    pub id: u64,
    pub public: String,
}

impl OneTimePrekey {
    pub fn generate(id: u64) -> (Self, StaticSecret) {
        let secret = StaticSecret::random_from_rng(&mut OsRng);
        let public = PublicKey::from(&secret);
        (
            Self { id, public: hex::encode(public.as_bytes()) },
            secret,
        )
    }

    pub fn public_bytes(&self) -> Option<[u8; 32]> {
        hex::decode(&self.public).ok()?.try_into().ok()
    }
}

/// Publicly-publishable bundle that a sender fetches to run X3DH against.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PrekeyBundle {
    /// Ed25519 identity public (hex).
    pub identity_pub: String,
    /// The currently-active signed prekey.
    pub signed_prekey: SignedPrekey,
    /// Optional fresh one-time prekey. Bundles served from a directory can
    /// rotate this on every fetch; peer-to-peer exchanges may omit it.
    pub one_time_prekey: Option<OneTimePrekey>,
}

impl PrekeyBundle {
    /// Build a bundle from raw material.
    pub fn new(
        identity_pub: [u8; 32],
        signed_prekey: SignedPrekey,
        one_time_prekey: Option<OneTimePrekey>,
    ) -> Self {
        Self {
            identity_pub: hex::encode(identity_pub),
            signed_prekey,
            one_time_prekey,
        }
    }

    pub fn identity_bytes(&self) -> Option<[u8; 32]> {
        hex::decode(&self.identity_pub).ok()?.try_into().ok()
    }

    /// Validate the internal signature chain. A sender **must** call this
    /// before doing any ECDH against the bundle — otherwise an attacker
    /// who replaced the signed_prekey could read future messages.
    pub fn verify(&self) -> bool {
        let Some(id) = self.identity_bytes() else { return false; };
        self.signed_prekey.verify(&id)
    }

    /// Random 16-byte bundle fingerprint for the recipient-side UI
    /// (prevents accidentally publishing two identical bundles).
    pub fn fingerprint(&self) -> [u8; 16] {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(self.identity_pub.as_bytes());
        h.update(self.signed_prekey.public.as_bytes());
        if let Some(opk) = &self.one_time_prekey {
            h.update(opk.public.as_bytes());
        }
        let digest = h.finalize();
        let mut out = [0u8; 16];
        out.copy_from_slice(&digest[..16]);
        out
    }
}

/// Receiver-side bookkeeping for the bundle's private material. This never
/// leaves the owner's device — only the [`PrekeyBundle`] goes on the wire.
pub struct PrekeyMaterial {
    pub signed_prekey_secret: StaticSecret,
    pub signed_prekey_id: u64,
    pub one_time_prekey_secret: Option<StaticSecret>,
    pub one_time_prekey_id: Option<u64>,
}

impl PrekeyMaterial {
    /// Generate a fresh set: one signed prekey plus one one-time prekey.
    /// The monotonic ids default to random `u64`s which is fine for toy
    /// deployments; real directories should allocate sequentially.
    pub fn fresh(identity: &PhantomSigningKey) -> (Self, PrekeyBundle) {
        let spk_id = OsRng.next_u64();
        let (spk, spk_secret) = SignedPrekey::generate(spk_id, identity);

        let opk_id = OsRng.next_u64();
        let (opk, opk_secret) = OneTimePrekey::generate(opk_id);

        let bundle = PrekeyBundle::new(identity.public_bytes(), spk, Some(opk));
        let material = Self {
            signed_prekey_secret: spk_secret,
            signed_prekey_id: spk_id,
            one_time_prekey_secret: Some(opk_secret),
            one_time_prekey_id: Some(opk_id),
        };
        (material, bundle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_verifies_against_its_identity() {
        let identity = PhantomSigningKey::generate();
        let (_material, bundle) = PrekeyMaterial::fresh(&identity);
        assert!(bundle.verify(), "fresh bundle must verify");
    }

    #[test]
    fn bundle_rejects_foreign_identity_signature() {
        let identity = PhantomSigningKey::generate();
        let (_material, mut bundle) = PrekeyMaterial::fresh(&identity);

        // Swap in a different identity pub — signature must no longer match.
        let other = PhantomSigningKey::generate();
        bundle.identity_pub = hex::encode(other.public_bytes());
        assert!(!bundle.verify());
    }

    #[test]
    fn tampered_signed_prekey_fails_verify() {
        let identity = PhantomSigningKey::generate();
        let (_material, mut bundle) = PrekeyMaterial::fresh(&identity);
        // Flip a bit in the signed prekey public — signature no longer matches.
        let mut bytes = bundle.signed_prekey.public_bytes().unwrap();
        bytes[0] ^= 0xFF;
        bundle.signed_prekey.public = hex::encode(bytes);
        assert!(!bundle.verify());
    }

    #[test]
    fn bundle_json_wire_roundtrip() {
        let identity = PhantomSigningKey::generate();
        let (_material, bundle) = PrekeyMaterial::fresh(&identity);
        let j = serde_json::to_string(&bundle).unwrap();
        let restored: PrekeyBundle = serde_json::from_str(&j).unwrap();
        assert!(restored.verify());
    }
}
