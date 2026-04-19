//! Envelope-Struktur und Serialisierung für PhantomChat.
//!
//! Das Envelope kapselt alle notwendigen Metadaten und die verschlüsselte
//! Nutzlast. Es nutzt Stealth-Tags zur Empfängeridentifikation und
//! Proof-of-Work zur Spam-Abwehr.
//!
//! ## Monero-Stealth-Address-Modell (korrekt implementiert)
//!
//! Der Empfänger veröffentlicht zwei Public Keys:
//! - `ViewKey.public` — erlaubt das Scannen (Tag-Verifikation) ohne Entschlüsselung.
//! - `SpendKey.public` — erforderlich zum Entschlüsseln des Payloads.
//!
//! Der Sender erzeugt pro Envelope ein ephemeres Keypair (epk) und leitet
//! **zwei** unabhängige geteilte Geheimnisse ab:
//!
//! ```text
//! view_shared  = eph_secret × recipient_view_pub   → HKDF → tag_key   (für HMAC-Tag)
//! spend_shared = eph_secret × recipient_spend_pub  → HKDF → enc_key   (für XChaCha20)
//! ```
//!
//! **Scan-Phase** (schnell, ohne Entschlüsselung): Empfänger berechnet
//! `view_shared = view_secret × epk` und verifiziert den HMAC-Tag.
//! **Open-Phase** (nur wenn Tag matcht): Empfänger berechnet
//! `spend_shared = spend_secret × epk` und entschlüsselt den Payload.
//!
//! Dadurch kann ein Relay-Operator oder Device mit nur dem ViewKey **sehen**,
//! welche Nachrichten für den Empfänger sind, aber sie nicht **lesen**.

use crate::keys::SpendKey;
use crate::pow::Hashcash;
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use rand_core::{OsRng, RngCore};
use sha2::Sha256;
use x25519_dalek::{PublicKey, StaticSecret};
use chacha20poly1305::{
    KeyInit as AeadKeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, Payload as AeadPayload},
};

/// HKDF info label for the scan-tag derivation. Uses `view_shared`.
pub(crate) const TAG_HKDF_INFO: &[u8] = b"PhantomChat-v1-ViewTag";
/// HKDF info label for the payload-encryption-key derivation. Uses `spend_shared`.
pub(crate) const ENC_HKDF_INFO: &[u8] = b"PhantomChat-v1-Envelope";

// ── Payload ───────────────────────────────────────────────────────────────────

/// Innere Nutzlast — wird verschlüsselt im Envelope transportiert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Payload {
    /// Eindeutige Nachrichten-ID (zufällig).
    pub msg_id: u128,
    /// Double-Ratchet-Header (PubKey, Count, Nonce).
    pub ratchet_header: Vec<u8>,
    /// Die eigentliche Nachricht (verschlüsselt durch Ratchet).
    pub encrypted_body: Vec<u8>,
}

impl Payload {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&self.msg_id.to_le_bytes());
        out.extend_from_slice(&(self.ratchet_header.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.ratchet_header);
        out.extend_from_slice(&(self.encrypted_body.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.encrypted_body);
        out
    }

    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        let mut c = 0usize;
        if data.len() < 16 + 4 + 4 { return None; }

        let msg_id = u128::from_le_bytes(data[c..c+16].try_into().ok()?); c += 16;

        let rh_len = u32::from_le_bytes(data[c..c+4].try_into().ok()?) as usize; c += 4;
        if c + rh_len > data.len() { return None; }
        let ratchet_header = data[c..c+rh_len].to_vec(); c += rh_len;

        if c + 4 > data.len() { return None; }
        let eb_len = u32::from_le_bytes(data[c..c+4].try_into().ok()?) as usize; c += 4;
        if c + eb_len > data.len() { return None; }
        let encrypted_body = data[c..c+eb_len].to_vec();

        Some(Self { msg_id, ratchet_header, encrypted_body })
    }
}

// ── Envelope ──────────────────────────────────────────────────────────────────

/// Äußere Schale für den Transport über Relays und P2P-Mesh.
///
/// Wire-Layout (little-endian):
/// ```text
///  1 B  ver
///  8 B  ts          (Unix seconds)
///  4 B  ttl         (seconds)
/// 32 B  epk         (ephemeral X25519 public key)
/// 32 B  tag         (HMAC stealth tag — HKDF(view_shared) keyed, over epk)
///  8 B  pow_nonce
/// 24 B  nonce       (XChaCha20 nonce)
///  4 B  ciphertext_len
///  N B  ciphertext  (XChaCha20-Poly1305, aad = tag, key = HKDF(spend_shared))
/// ```
#[derive(Debug, Clone)]
pub struct Envelope {
    pub ver: u8,
    pub ts: u64,
    pub ttl: u32,
    /// Ephemerer Public Key für den Schlüssel-Handshake.
    pub epk: [u8; 32],
    /// Stealth-Tag zur Empfängeridentifikation (HMAC-SHA256).
    pub tag: [u8; 32],
    /// Proof-of-Work Nonce.
    pub pow_nonce: u64,
    /// XChaCha20-Poly1305 Nonce.
    pub nonce: [u8; 24],
    /// Verschlüsselter Payload (XChaCha20-Poly1305, `tag` als AAD gebunden).
    pub ciphertext: Vec<u8>,
}

impl Envelope {
    /// Erzeugt ein neues, vollständig verschlüsseltes und PoW-gestempeltes Envelope.
    ///
    /// Verlangt **beide** Public Keys des Empfängers (Monero-Modell):
    /// - `recipient_view_pub`  → Stealth-Tag (Scanner kann matchen ohne Spend-Key)
    /// - `recipient_spend_pub` → Payload-Verschlüsselung (nur Spend-Key öffnet)
    pub fn new(
        recipient_view_pub: &PublicKey,
        recipient_spend_pub: &PublicKey,
        msg_id: u128,
        ratchet_header: Vec<u8>,
        encrypted_body: Vec<u8>,
        ttl: u32,
        pow_difficulty: u32,
    ) -> Self {
        // 1. Ephemerer Schlüssel
        let eph_secret = StaticSecret::random_from_rng(&mut OsRng);
        let eph_public = PublicKey::from(&eph_secret);
        let epk_bytes  = *eph_public.as_bytes();

        // 2. Zwei unabhängige ECDH-Shares
        let view_shared  = eph_secret.diffie_hellman(recipient_view_pub);
        let spend_shared = eph_secret.diffie_hellman(recipient_spend_pub);

        // 3. tag_key aus view_shared
        let tag_hkdf = Hkdf::<Sha256>::new(None, view_shared.as_bytes());
        let mut tag_key = [0u8; 32];
        tag_hkdf.expand(TAG_HKDF_INFO, &mut tag_key).expect("HKDF tag");

        // 4. Stealth-Tag = HMAC(tag_key, epk). Sender und Empfänger können
        //    beide `epk` lesen, also ist das über den Wire reproduzierbar.
        let mut tag_mac = <Hmac<Sha256> as Mac>::new_from_slice(&tag_key).expect("HMAC key");
        tag_mac.update(&epk_bytes);
        let tag: [u8; 32] = tag_mac.finalize().into_bytes().into();

        // 5. enc_key aus spend_shared
        let enc_hkdf = Hkdf::<Sha256>::new(None, spend_shared.as_bytes());
        let mut enc_key = [0u8; 32];
        enc_hkdf.expand(ENC_HKDF_INFO, &mut enc_key).expect("HKDF enc");

        // 6. Payload bauen und verschlüsseln (Tag als AAD → bindet ihn an den Chiffretext)
        let payload = Payload { msg_id, ratchet_header, encrypted_body };
        let payload_bytes = payload.to_bytes();

        let mut nonce = [0u8; 24];
        OsRng.fill_bytes(&mut nonce);

        let cipher = XChaCha20Poly1305::new_from_slice(&enc_key).expect("Cipher init");
        let ciphertext = cipher
            .encrypt(XNonce::from_slice(&nonce), AeadPayload {
                msg: &payload_bytes,
                aad: &tag,
            })
            .expect("Encryption failed");

        // 7. Proof-of-Work über (tag || ts)
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let mut pow_header = Vec::with_capacity(40);
        pow_header.extend_from_slice(&tag);
        pow_header.extend_from_slice(&ts.to_le_bytes());
        let pow_nonce = Hashcash::new(pow_difficulty).compute_nonce(&pow_header);

        Self { ver: 1, ts, ttl, epk: epk_bytes, tag, pow_nonce, nonce, ciphertext }
    }

    /// Serialisiert das Envelope für den Versand (Wire-Format).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(1 + 8 + 4 + 32 + 32 + 8 + 24 + 4 + self.ciphertext.len());
        out.push(self.ver);
        out.extend_from_slice(&self.ts.to_le_bytes());
        out.extend_from_slice(&self.ttl.to_le_bytes());
        out.extend_from_slice(&self.epk);
        out.extend_from_slice(&self.tag);
        out.extend_from_slice(&self.pow_nonce.to_le_bytes());
        out.extend_from_slice(&self.nonce);
        out.extend_from_slice(&(self.ciphertext.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.ciphertext);
        out
    }

    /// Deserialisiert ein Envelope aus rohen Bytes.
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        const MIN: usize = 1 + 8 + 4 + 32 + 32 + 8 + 24 + 4;
        if data.len() < MIN { return None; }

        let mut c = 0usize;
        let ver = data[c]; c += 1;
        let ts  = u64::from_le_bytes(data[c..c+8].try_into().ok()?); c += 8;
        let ttl = u32::from_le_bytes(data[c..c+4].try_into().ok()?); c += 4;

        let mut epk = [0u8; 32];
        epk.copy_from_slice(&data[c..c+32]); c += 32;

        let mut tag = [0u8; 32];
        tag.copy_from_slice(&data[c..c+32]); c += 32;

        let pow_nonce = u64::from_le_bytes(data[c..c+8].try_into().ok()?); c += 8;

        let mut nonce = [0u8; 24];
        nonce.copy_from_slice(&data[c..c+24]); c += 24;

        let ct_len = u32::from_le_bytes(data[c..c+4].try_into().ok()?) as usize; c += 4;
        if c + ct_len > data.len() { return None; }
        let ciphertext = data[c..c+ct_len].to_vec();

        Some(Self { ver, ts, ttl, epk, tag, pow_nonce, nonce, ciphertext })
    }

    /// Versucht das Envelope mit dem eigenen SpendKey zu öffnen.
    /// Gibt `Some(Payload)` zurück wenn der SpendKey passt, sonst `None`.
    pub fn open(&self, spend_key: &SpendKey) -> Option<Payload> {
        let spend_shared = spend_key.secret.diffie_hellman(&PublicKey::from(self.epk));
        let hk = Hkdf::<Sha256>::new(None, spend_shared.as_bytes());
        let mut enc_key = [0u8; 32];
        hk.expand(ENC_HKDF_INFO, &mut enc_key).ok()?;

        let cipher = XChaCha20Poly1305::new_from_slice(&enc_key).ok()?;
        let plain  = cipher.decrypt(XNonce::from_slice(&self.nonce), AeadPayload {
            msg: &self.ciphertext,
            aad: &self.tag,
        }).ok()?;

        Payload::from_bytes(&plain)
    }

    /// Erzeugt ein Cover-Traffic-Dummy-Envelope.
    ///
    /// Alle Felder sind CSPRNG-Zufallsdaten — auf dem Wire nicht von echten
    /// Envelopes zu unterscheiden.  Der PoW-Nonce ist 0; Empfänger verwerfen
    /// das Envelope beim HMAC-Scan stillschweigend.
    pub fn dummy() -> Option<Self> {
        let mut epk   = [0u8; 32];
        let mut tag   = [0u8; 32];
        let mut nonce = [0u8; 24];
        OsRng.fill_bytes(&mut epk);
        OsRng.fill_bytes(&mut tag);
        OsRng.fill_bytes(&mut nonce);
        // Variable Länge damit reine Byte-Count-Analyse Dummies nicht identifiziert.
        let ct_len: usize = 64 + rand::random::<u8>() as usize;
        let mut ciphertext = vec![0u8; ct_len];
        OsRng.fill_bytes(&mut ciphertext);
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs();
        Some(Self { ver: 1, ts, ttl: 300, epk, tag, pow_nonce: 0, nonce, ciphertext })
    }
}
