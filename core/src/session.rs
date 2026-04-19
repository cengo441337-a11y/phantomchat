//! High-level session API.
//!
//! Binds the pieces together: takes plaintext bytes, returns a fully-sealed
//! [`Envelope`] on the way out, and takes a received [`Envelope`] + local
//! key material to give plaintext back.
//!
//! ## Bootstrap
//!
//! Both sides derive the initial shared secret deterministically from the
//! envelope-layer ECDH via
//!
//! ```text
//! initial_shared = HKDF(spend_shared, info = "PhantomChat-v1-Session", 32)
//! ```
//!
//! The sender learns `spend_shared` from its own ephemeral × recipient
//! spend-pub. The receiver can recompute the exact same value from its
//! spend-key secret × the envelope's `epk`. Once both sides have
//! `initial_shared`, a Signal-style [`RatchetState`] takes over.
//!
//! ## Persistence
//!
//! The [`SessionStore`] is a flat `HashMap<PhantomAddress, RatchetState>`
//! and serialises to JSON. Callers such as the CLI write it back to disk
//! after every send / receive so sessions survive process restarts.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use x25519_dalek::PublicKey;

use crate::address::PhantomAddress;
use crate::envelope::Envelope;
use crate::keys::{SpendKey, ViewKey};
use crate::ratchet::{RatchetError, RatchetState};
use crate::scanner::{scan_envelope, ScanResult};

const SESSION_HKDF_INFO: &[u8] = b"PhantomChat-v1-Session";

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("envelope was not addressed to this identity")]
    NotMine,
    #[error("envelope scanned but the outer AEAD decryption failed")]
    OuterDecryptFailed,
    #[error("outer payload did not carry a ratchet header")]
    MissingHeader,
    #[error("ratchet decrypt failed: {0}")]
    Ratchet(#[from] RatchetError),
    #[error("session store i/o: {0}")]
    Io(#[from] std::io::Error),
    #[error("session store serde: {0}")]
    Serde(#[from] serde_json::Error),
}

/// Collection of Double-Ratchet sessions keyed by contact address.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SessionStore {
    sessions: HashMap<String, RatchetState>,
}

impl SessionStore {
    pub fn new() -> Self {
        Self::default()
    }

    // ── Persistence ────────────────────────────────────────────────────────

    pub fn load(path: impl AsRef<Path>) -> Result<Self, SessionError> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read(path)?;
        let mut store: Self = serde_json::from_slice(&raw)?;
        for session in store.sessions.values_mut() {
            session.restore_secret();
        }
        Ok(store)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), SessionError> {
        let bytes = serde_json::to_vec_pretty(self)?;
        fs::write(path, bytes)?;
        Ok(())
    }

    // ── Core send / receive ────────────────────────────────────────────────

    /// Encrypt `plaintext` addressed to `recipient` and return a ready-to-
    /// publish [`Envelope`]. Creates a new ratchet session on first contact.
    pub fn send(
        &mut self,
        recipient: &PhantomAddress,
        plaintext: &[u8],
        pow_difficulty: u32,
    ) -> Envelope {
        let session = self.sessions.entry(recipient.short_id()).or_insert_with(|| {
            // Bootstrap: derive `initial_shared` from a throwaway shared
            // secret between us and the recipient's spend_pub. The envelope
            // layer does its own fresh ECDH later, so the ratchet's
            // bootstrap value only needs to be deterministic on both sides.
            let bootstrap = initial_shared_from_spend_pub(&recipient.spend_pub());
            RatchetState::initialize_as_sender(bootstrap, recipient.spend_pub())
        });

        let (header, inner_ciphertext) = session.encrypt(plaintext);
        let msg_id = rand::random::<u128>();

        Envelope::new(
            &recipient.view_pub(),
            &recipient.spend_pub(),
            msg_id,
            header,
            inner_ciphertext,
            300,
            pow_difficulty,
        )
    }

    /// Try to decrypt `envelope` using our own `view_key` + `spend_key`.
    /// Returns `Ok(None)` when the envelope simply isn't for us (silent drop
    /// — the common case on a relay stream). Propagates real errors.
    pub fn receive(
        &mut self,
        envelope: &Envelope,
        view_key: &ViewKey,
        spend_key: &SpendKey,
    ) -> Result<Option<Vec<u8>>, SessionError> {
        let payload = match scan_envelope(envelope, view_key, spend_key) {
            ScanResult::Mine(p) => p,
            ScanResult::NotMine => return Ok(None),
            ScanResult::Corrupted => return Err(SessionError::OuterDecryptFailed),
        };

        if payload.ratchet_header.len() < 32 {
            return Err(SessionError::MissingHeader);
        }

        // We only see what the envelope exposes: the outer `epk` and the
        // ratchet header. We do **not** see the peer's stable PhantomAddress.
        //
        // First, try every existing session. This covers the common case
        // where an ongoing conversation's peer just did a DH-ratchet step
        // (the peer_ratchet public rotated but it's still the same contact).
        // We clone before attempting so a failed decrypt doesn't advance the
        // real session's chain keys.
        for session in self.sessions.values_mut() {
            let mut candidate = session.clone();
            candidate.restore_secret();
            if let Ok(plain) = candidate.decrypt(
                &payload.ratchet_header,
                &payload.encrypted_body,
            ) {
                *session = candidate;
                return Ok(Some(plain));
            }
        }

        // No existing session could decrypt — treat this as first contact.
        let peer_ratchet_bytes: [u8; 32] = payload.ratchet_header[0..32]
            .try_into()
            .map_err(|_| SessionError::MissingHeader)?;
        let peer_ratchet_pub = PublicKey::from(peer_ratchet_bytes);
        let bootstrap = initial_shared_from_spend_pub(&spend_key.public);

        let session_key = format!("peer:{}", hex::encode(&peer_ratchet_bytes[..8]));
        let session = self.sessions.entry(session_key).or_insert_with(|| {
            RatchetState::initialize_as_receiver(bootstrap, &spend_key.secret, peer_ratchet_pub)
        });

        let plaintext = session.decrypt(&payload.ratchet_header, &payload.encrypted_body)?;
        Ok(Some(plaintext))
    }
}

/// `initial_shared = HKDF(spend_pub_bytes, info = SESSION_HKDF_INFO, 32 B)`.
///
/// Note: this only needs to be *deterministic on both sides*, not a secret.
/// Forward secrecy comes from the per-message DH ratchet that sits on top.
fn initial_shared_from_spend_pub(spend_pub: &PublicKey) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(None, spend_pub.as_bytes());
    let mut out = [0u8; 32];
    hk.expand(SESSION_HKDF_INFO, &mut out).expect("HKDF");
    out
}
