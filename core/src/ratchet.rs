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
//!
//! ## Replay / out-of-order safety (audit 2026-05-30, R-1/R-2/R-3)
//!
//! The decrypt path is **transactional**: it stages every state mutation on
//! a clone and only commits when the AEAD seal validates. That gives three
//! properties at once:
//!
//! 1. **Replay** of a previously-decrypted ciphertext fails with
//!    [`RatchetError::Replay`] *without* mutating the live state. Before
//!    this rewrite, the receive-chain advanced on every call regardless of
//!    AEAD outcome, so a single duplicate envelope permanently desynced the
//!    session.
//! 2. **Out-of-order** messages within a single sending chain are tolerated
//!    via the [`RatchetState::skipped_keys`] cache (capped at [`MAX_SKIP`]
//!    per chain and [`MAX_SKIPPED_KEYS`] in total). Senders can keep
//!    pipelining without losing messages to natural reordering on the
//!    relay tier.
//! 3. **Attacker-controlled `peer_ratchet_pub`** can no longer DoS a
//!    session: the DH ratchet step happens on the staged clone, so a
//!    malformed envelope with a fresh-but-random peer_pub is discarded
//!    along with the clone when its AEAD check fails.

use std::collections::HashMap;

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

/// Maximum number of message keys we'll derive eagerly when a peer counter
/// runs ahead of our `recv_count` within a single sending chain. Cap
/// prevents an attacker from forging a single envelope with a huge counter
/// and forcing the receiver to do millions of HMACs.
pub const MAX_SKIP: u32 = 1_000;

/// Global cap on the size of the [`RatchetState::skipped_keys`] cache.
/// Cross-chain plus within-chain entries share this budget. Oldest entries
/// (insertion order) are evicted FIFO when the cap is exceeded.
pub const MAX_SKIPPED_KEYS: usize = 4_000;

/// Fehler, der bei der Ratchet‑Verarbeitung auftreten kann.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RatchetError {
    #[error("Entschlüsselung fehlgeschlagen")]
    DecryptionFailed,
    #[error("Ungültiger Header")]
    InvalidHeader,
    /// The header counter is less than `recv_count` for the matching chain
    /// — i.e. either a re-delivery of an already-consumed envelope, or a
    /// counter from a prior DH-chain whose key did not survive in the
    /// [`RatchetState::skipped_keys`] cache.
    #[error("Replay erkannt (counter bereits verarbeitet)")]
    Replay,
    /// The header counter is more than [`MAX_SKIP`] ahead of `recv_count`.
    /// Either an attacker, a network with extreme reordering, or a sender
    /// that pipelined too aggressively across a DH ratchet boundary.
    #[error("zu viele Nachrichten in einer Kette übersprungen ({0})")]
    TooMuchSkip(u32),
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

/// Cache key for [`RatchetState::skipped_keys`]: `(peer_ratchet_pub, counter)`.
type SkippedKeyId = ([u8; 32], u32);

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

    /// Cache of message keys for envelopes that arrived out of order, keyed
    /// by `(peer_ratchet_pub, counter)`. Capped at [`MAX_SKIPPED_KEYS`]
    /// total entries via FIFO eviction tracked in
    /// [`RatchetState::skipped_order`]. Audit 2026-05-30 (R-1/R-2): without
    /// this cache, a single dropped envelope between two consecutive sends
    /// permanently broke the chain.
    #[serde(default)]
    skipped_keys: HashMap<SkippedKeyId, [u8; 32]>,

    /// Insertion order of [`skipped_keys`]. Drives FIFO eviction once the
    /// cache exceeds [`MAX_SKIPPED_KEYS`].
    #[serde(default)]
    skipped_order: Vec<SkippedKeyId>,
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
        let ratchet_secret = StaticSecret::random_from_rng(OsRng);
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
            skipped_keys: HashMap::new(),
            skipped_order: Vec::new(),
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
        let ratchet_secret = StaticSecret::random_from_rng(OsRng);
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
            skipped_keys: HashMap::new(),
            skipped_order: Vec::new(),
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
        self.ratchet_secret = StaticSecret::random_from_rng(OsRng);
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

    /// Insert a skipped (chain, counter) → msg-key entry, evicting the
    /// oldest entry FIFO if the cache exceeds [`MAX_SKIPPED_KEYS`]. Re-inserting
    /// an already-cached key is a no-op (preserves insertion order).
    fn insert_skipped(&mut self, peer_pub: [u8; 32], counter: u32, key: [u8; 32]) {
        let id = (peer_pub, counter);
        if self.skipped_keys.insert(id, key).is_none() {
            self.skipped_order.push(id);
            while self.skipped_order.len() > MAX_SKIPPED_KEYS {
                let drop_id = self.skipped_order.remove(0);
                self.skipped_keys.remove(&drop_id);
            }
        }
    }

    /// Encrypts a plaintext and returns `(ratchet_header, ciphertext)`.
    /// The header layout is `[32 B ratchet_pub][4 B counter][24 B nonce]`.
    pub fn encrypt(&mut self, plaintext: &[u8]) -> (Vec<u8>, Vec<u8>) {
        // Header carries the COUNTER OF THIS message (= send_count before
        // we bump it), so we capture it before [`next_send_key`] increments.
        let counter = self.send_count;
        let msg_key = self.next_send_key();
        let cipher = XChaCha20Poly1305::new(msg_key.as_slice().into());

        let mut nonce_bytes = [0u8; 24];
        rand::thread_rng().fill(&mut nonce_bytes);
        let nonce = XNonce::from_slice(&nonce_bytes);

        let mut header = Vec::with_capacity(60);
        let own_pub = PublicKey::from(&self.ratchet_secret);
        header.extend_from_slice(own_pub.as_bytes());
        header.extend_from_slice(&counter.to_le_bytes());
        header.extend_from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, Payload { msg: plaintext, aad: &header })
            .expect("XChaCha20-Poly1305 encrypt should not fail");

        (header, ciphertext)
    }

    /// Decrypts a `(ratchet_header, ciphertext)` pair.
    ///
    /// **Transactional**: every state change (skipped-key derivations,
    /// DH-ratchet step, recv_chain advance, recv_count bump) is staged on a
    /// local clone and committed atomically only after the AEAD seal
    /// validates. A malformed / replayed / attacker-crafted envelope
    /// therefore *cannot* desynchronise the session, even though the call
    /// site sees an `Err`. See module-level docs for the threat model that
    /// rewrite this fixes (audit 2026-05-30, R-1/R-2/R-3).
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
        let counter = u32::from_le_bytes(
            header[32..36]
                .try_into()
                .map_err(|_| RatchetError::InvalidHeader)?,
        );
        let nonce_bytes = &header[36..60];

        // ── Path 1: cached skipped key ────────────────────────────────────
        // Out-of-order arrival of a previously-skipped envelope: the
        // matching msg-key was derived (and stored) when a later-counter
        // envelope in the same chain landed first. Use it and remove from
        // the cache so a replay of the same skipped envelope still fails.
        let cache_id = (peer_pub_bytes, counter);
        if let Some(msg_key) = self.skipped_keys.get(&cache_id).copied() {
            let cipher = XChaCha20Poly1305::new(msg_key.as_slice().into());
            let plain = cipher
                .decrypt(
                    XNonce::from_slice(nonce_bytes),
                    Payload { msg: ciphertext, aad: header },
                )
                .map_err(|_| RatchetError::DecryptionFailed)?;
            // Authenticated — evict the consumed entry so a re-delivery is
            // caught as Replay rather than silently re-accepted.
            self.skipped_keys.remove(&cache_id);
            self.skipped_order.retain(|k| k != &cache_id);
            return Ok(plain);
        }

        // ── Path 2: stage-on-clone, commit-on-AEAD-success ────────────────
        let mut staged = self.clone();
        // The Clone derive copies ratchet_secret_bytes but Clone-on
        // StaticSecret is implemented as well (curve25519-dalek v4), so the
        // staged secret is already usable without restore_secret.
        let peer_pub = PublicKey::from(peer_pub_bytes);
        let same_chain = staged.recv_initialised
            && peer_pub.as_bytes() == &staged.peer_ratchet_public;

        if !same_chain {
            // New chain ⇒ DH ratchet step on the staged clone. If the
            // ensuing AEAD fails (attacker-controlled peer_pub + random
            // ciphertext), the clone is discarded and the live state stays
            // pinned to the previous chain. Pre-audit, this step ran on
            // the live state and made the session DoS-able by a single
            // forged envelope.
            staged.dh_ratchet(peer_pub);
        }

        // Replay / catch-up window inside the (possibly fresh) recv chain.
        if counter < staged.recv_count {
            return Err(RatchetError::Replay);
        }
        let skip_n = counter
            .checked_sub(staged.recv_count)
            .ok_or(RatchetError::Replay)?;
        if skip_n > MAX_SKIP {
            return Err(RatchetError::TooMuchSkip(skip_n));
        }

        // Derive (and stage) keys for any counters we're jumping over so
        // the matching real envelopes can land later.
        for k in staged.recv_count..counter {
            let (next_ck, msg_key) = kdf_ck(&staged.recv_chain);
            staged.recv_chain = next_ck;
            staged.insert_skipped(peer_pub_bytes, k, msg_key);
            staged.recv_count = k + 1;
        }

        // Derive *this* message's key (don't advance recv_chain yet —
        // we only do that once AEAD validates).
        let (next_ck, msg_key) = kdf_ck(&staged.recv_chain);

        let cipher = XChaCha20Poly1305::new(msg_key.as_slice().into());
        let plain = cipher
            .decrypt(
                XNonce::from_slice(nonce_bytes),
                Payload { msg: ciphertext, aad: header },
            )
            .map_err(|_| RatchetError::DecryptionFailed)?;

        // ── Commit ────────────────────────────────────────────────────────
        staged.recv_chain = next_ck;
        staged.recv_count = counter.saturating_add(1);
        *self = staged;
        Ok(plain)
    }
}

impl std::fmt::Debug for RatchetState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Explicitly *do not* include chain keys or the DH secret.
        f.debug_struct("RatchetState")
            .field("send_count", &self.send_count)
            .field("recv_count", &self.recv_count)
            .field("recv_initialised", &self.recv_initialised)
            .field("skipped_keys_count", &self.skipped_keys.len())
            .finish_non_exhaustive()
    }
}
