//! Double Ratchet — Signal-style forward-secret message ratchet.
//!
//! ## Roles
//!
//! Sessions are asymmetric at bootstrap time:
//!
//! - The **sender** calls [`RatchetState::initialize_as_sender`] with the
//!   initial shared secret negotiated through PhantomChat's envelope-level
//!   ECDH. It picks its own fresh ratchet X25519 keypair and uses the peer's
//!   long-term spend public key as the *bootstrap* peer-ratchet public.
//! - The **receiver** calls [`RatchetState::initialize_as_receiver`] the
//!   first time a message arrives, lifting the sender's ratchet public out
//!   of the header. Its first outgoing message triggers a DH ratchet step.
//!
//! ## Per-message flow
//!
//! ```text
//! encrypt(plaintext) → (ratchet_header, ciphertext)
//!                         │            └── XChaCha20-Poly1305 with a
//!                         │                freshly derived msg_key
//!                         └── [32 B ratchet_pub][4 B counter][24 B nonce]
//! ```
//!
//! The `ratchet_header` is placed directly into [`Payload::ratchet_header`]
//! and the `ciphertext` into [`Payload::encrypted_body`]. The outer
//! [`Envelope`] layer then adds the stealth tag and PoW.
//!
//! ## Forward secrecy
//!
//! Every call to [`RatchetState::encrypt`] rotates the sending chain key,
//! and every new peer ratchet public observed during [`RatchetState::decrypt`]
//! triggers a DH ratchet step. Compromising the current state does not
//! reveal earlier plaintexts.

use chacha20poly1305::{
    aead::{Aead, KeyInit as AeadKeyInit, Payload},
    XChaCha20Poly1305, XNonce,
};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use rand::Rng;
use rand_core::OsRng;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use x25519_dalek::{PublicKey, StaticSecret};

type HmacSha256 = Hmac<Sha256>;

/// Fehler, der bei der Ratchet‑Verarbeitung auftreten kann.
#[derive(Debug, thiserror::Error)]
pub enum RatchetError {
    #[error("Entschlüsselung fehlgeschlagen")]
    DecryptionFailed,
    #[error("Ungültiger Header")]
    InvalidHeader,
}

/// KDF für die Root-Kette (KDF-RK).
fn kdf_rk(rk: &[u8; 32], dh_out: &[u8; 32]) -> ([u8; 32], [u8; 32]) {
    let h = Hkdf::<Sha256>::new(Some(rk), dh_out);
    let mut okm = [0u8; 64];
    h.expand(b"PhantomChat-v1-Root", &mut okm)
        .expect("HKDF expansion should not fail");
    let mut new_rk = [0u8; 32];
    let mut new_ck = [0u8; 32];
    new_rk.copy_from_slice(&okm[0..32]);
    new_ck.copy_from_slice(&okm[32..64]);
    (new_rk, new_ck)
}

/// KDF für die Nachrichten-Ketten (KDF-CK).
fn kdf_ck(ck: &[u8; 32]) -> ([u8; 32], [u8; 32]) {
    let mut h_ck = <HmacSha256 as Mac>::new_from_slice(ck).expect("HMAC should accept 32-byte key");
    h_ck.update(&[0x01]);
    let next_ck: [u8; 32] = h_ck.finalize().into_bytes().into();

    let mut h_mk = <HmacSha256 as Mac>::new_from_slice(ck).expect("HMAC should accept 32-byte key");
    h_mk.update(&[0x02]);
    let msg_key: [u8; 32] = h_mk.finalize().into_bytes().into();

    (next_ck, msg_key)
}

/// Zustand einer Double‑Ratchet‑Session.
#[derive(Clone, Serialize, Deserialize)]
pub struct RatchetState {
    root_key: [u8; 32],
    send_chain: [u8; 32],
    recv_chain: [u8; 32],
    /// Current ratchet X25519 secret. Skipped on serialization — deserialized
    /// states need [`RatchetState::restore_secret`] before use.
    #[serde(skip, default = "zero_secret")]
    ratchet_secret: StaticSecret,
    /// Serialized copy of `ratchet_secret` so the session can round-trip
    /// through JSON/SQLite without losing its DH key material.
    ratchet_secret_bytes: [u8; 32],
    peer_ratchet_public: [u8; 32],
    send_count: u32,
    recv_count: u32,
    /// True once at least one incoming message has been processed. Until then
    /// `recv_chain` is not yet keyed and [`decrypt`] will trigger the first
    /// DH ratchet step on any incoming peer key.
    recv_initialised: bool,
}

fn zero_secret() -> StaticSecret {
    StaticSecret::from([0u8; 32])
}

impl RatchetState {
    /// Sender-seitiger Bootstrap.
    ///
    /// `initial_shared` ist der aus dem Envelope-ECDH abgeleitete Startwert
    /// (üblicherweise `HKDF(spend_shared, "PhantomChat-v1-Session")`).
    /// `recipient_spend_pub` wird als Bootstrap-"peer ratchet public" benutzt
    /// — die erste empfangene Antwort enthält den echten Ratchet-Pub des
    /// Peers und triggert dann einen regulären DH-Schritt.
    pub fn initialize_as_sender(
        initial_shared: [u8; 32],
        recipient_spend_pub: PublicKey,
    ) -> Self {
        let ratchet_secret = StaticSecret::random_from_rng(&mut OsRng);
        let ratchet_secret_bytes = ratchet_secret.to_bytes();

        let dh_out = ratchet_secret.diffie_hellman(&recipient_spend_pub);
        let (root_key, send_chain) = kdf_rk(&initial_shared, dh_out.as_bytes());

        Self {
            root_key,
            send_chain,
            recv_chain: [0u8; 32],
            ratchet_secret,
            ratchet_secret_bytes,
            peer_ratchet_public: *recipient_spend_pub.as_bytes(),
            send_count: 0,
            recv_count: 0,
            recv_initialised: false,
        }
    }

    /// Receiver-seitiger Bootstrap, ausgelöst durch die erste eintreffende
    /// Nachricht. `peer_ratchet_pub` ist der vom Sender übertragene
    /// Ratchet-Public (erste 32 B des `ratchet_header`), `own_spend_secret`
    /// ist der eigene Spend-X25519-Secret — das Gegenstück zu dem
    /// `recipient_spend_pub`, das der Sender in
    /// [`initialize_as_sender`] als Bootstrap-Peer genommen hat.
    ///
    /// Durch ECDH-Kommutativität stimmen
    /// `own_spend_secret × peer_ratchet_pub`
    /// und
    /// `peer_ratchet_secret × own_spend_pub`
    /// überein — dadurch kommen Sender und Empfänger auf denselben
    /// Root-Key, obwohl sie unterschiedliche Seiten des Handshakes sehen.
    ///
    /// Initialisiert zusätzlich direkt die Sende-Kette (DH mit einem frisch
    /// gezogenen eigenen Ratchet-Secret), damit die erste eigene Antwort
    /// sofort verschlüsselt werden kann.
    pub fn initialize_as_receiver(
        initial_shared: [u8; 32],
        own_spend_secret: &StaticSecret,
        peer_ratchet_pub: PublicKey,
    ) -> Self {
        // Schritt 1 — Empfangs-Kette (symmetrisch zum Sender-DH).
        let dh_recv = own_spend_secret.diffie_hellman(&peer_ratchet_pub);
        let (rk1, recv_chain) = kdf_rk(&initial_shared, dh_recv.as_bytes());

        // Schritt 2 — eigenes Ratchet-Secret + Sende-Kette.
        let ratchet_secret = StaticSecret::random_from_rng(&mut OsRng);
        let dh_send = ratchet_secret.diffie_hellman(&peer_ratchet_pub);
        let (root_key, send_chain) = kdf_rk(&rk1, dh_send.as_bytes());

        Self {
            root_key,
            send_chain,
            recv_chain,
            ratchet_secret_bytes: ratchet_secret.to_bytes(),
            ratchet_secret,
            peer_ratchet_public: *peer_ratchet_pub.as_bytes(),
            send_count: 0,
            recv_count: 0,
            recv_initialised: true,
        }
    }

    /// After JSON/SQLite load, rebuild `ratchet_secret` from its serialized
    /// bytes. Serde default leaves it zeroed — calling encrypt/decrypt in
    /// that state would produce predictable key material.
    pub fn restore_secret(&mut self) {
        self.ratchet_secret = StaticSecret::from(self.ratchet_secret_bytes);
    }

    /// Executes a DH ratchet step triggered by a new peer ratchet public.
    /// Advances both the receive- and send-chain atomically.
    fn dh_ratchet(&mut self, new_peer_public: PublicKey) {
        self.peer_ratchet_public = *new_peer_public.as_bytes();
        self.recv_count = 0;

        // Derive a new receive chain from the CURRENT secret and the new peer public.
        let dh_recv = self.ratchet_secret.diffie_hellman(&new_peer_public);
        let (rk, ck_recv) = kdf_rk(&self.root_key, dh_recv.as_bytes());
        self.root_key = rk;
        self.recv_chain = ck_recv;
        self.recv_initialised = true;

        // Rotate our own ratchet secret, then re-derive the send chain.
        self.ratchet_secret = StaticSecret::random_from_rng(&mut OsRng);
        self.ratchet_secret_bytes = self.ratchet_secret.to_bytes();
        self.send_count = 0;

        let dh_send = self.ratchet_secret.diffie_hellman(&new_peer_public);
        let (rk, ck_send) = kdf_rk(&self.root_key, dh_send.as_bytes());
        self.root_key = rk;
        self.send_chain = ck_send;
    }

    fn next_send_key(&mut self) -> [u8; 32] {
        let (next_ck, msg_key) = kdf_ck(&self.send_chain);
        self.send_chain = next_ck;
        self.send_count += 1;
        msg_key
    }

    fn next_recv_key(&mut self) -> [u8; 32] {
        let (next_ck, msg_key) = kdf_ck(&self.recv_chain);
        self.recv_chain = next_ck;
        self.recv_count += 1;
        msg_key
    }

    /// Encrypts a plaintext and returns `(ratchet_header, ciphertext)`.
    /// The header layout is `[32 B ratchet_pub][4 B counter][24 B nonce]`.
    pub fn encrypt(&mut self, plaintext: &[u8]) -> (Vec<u8>, Vec<u8>) {
        let msg_key = self.next_send_key();
        let cipher = XChaCha20Poly1305::new(msg_key.as_slice().into());

        let mut nonce_bytes = [0u8; 24];
        rand::thread_rng().fill(&mut nonce_bytes);
        let nonce = XNonce::from_slice(&nonce_bytes);

        let mut header = Vec::with_capacity(60);
        let own_pub = PublicKey::from(&self.ratchet_secret);
        header.extend_from_slice(own_pub.as_bytes());
        header.extend_from_slice(&self.send_count.to_le_bytes());
        header.extend_from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, Payload { msg: plaintext, aad: &header })
            .expect("XChaCha20-Poly1305 encrypt should not fail");

        (header, ciphertext)
    }

    /// Decrypts a `(ratchet_header, ciphertext)` pair. Rotates the DH ratchet
    /// if the header's ratchet-public differs from the currently tracked one.
    pub fn decrypt(
        &mut self,
        header: &[u8],
        ciphertext: &[u8],
    ) -> Result<Vec<u8>, RatchetError> {
        if header.len() < 60 {
            return Err(RatchetError::InvalidHeader);
        }

        let peer_pub_bytes: [u8; 32] = header[0..32]
            .try_into()
            .map_err(|_| RatchetError::InvalidHeader)?;
        let peer_pub = PublicKey::from(peer_pub_bytes);

        if !self.recv_initialised || peer_pub.as_bytes() != &self.peer_ratchet_public {
            self.dh_ratchet(peer_pub);
        }

        let msg_key = self.next_recv_key();
        let cipher = XChaCha20Poly1305::new(msg_key.as_slice().into());

        let nonce_bytes = &header[36..60];
        let nonce = XNonce::from_slice(nonce_bytes);

        cipher
            .decrypt(nonce, Payload { msg: ciphertext, aad: header })
            .map_err(|_| RatchetError::DecryptionFailed)
    }
}

impl std::fmt::Debug for RatchetState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Explicitly *do not* include chain keys or the DH secret.
        f.debug_struct("RatchetState")
            .field("send_count", &self.send_count)
            .field("recv_count", &self.recv_count)
            .field("recv_initialised", &self.recv_initialised)
            .finish_non_exhaustive()
    }
}
