//! Envelope‑Struktur und Serialisierung für PhantomChat (Production).
//!
//! Das Envelope kapselt alle notwendigen Metadaten und die verschlüsselte
//! Nutzlast. Es nutzt Stealth‑Tags zur Empfängeridentifikation und
//! Proof‑of‑Work zur Spam‑Abwehr.

use crate::keys::{SpendKey};
use crate::pow::Hashcash;
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use rand_core::{OsRng, RngCore};
use sha2::Sha256;
use x25519_dalek::{PublicKey, StaticSecret};
use chacha20poly1305::{KeyInit as AeadKeyInit, XChaCha20Poly1305, XNonce, aead::{Aead, Payload as AeadPayload}};

/// Struktur der inneren Nutzlast (verschlüsselt im Envelope).
#[derive(Debug, Clone)]
pub struct Payload {
    /// Eindeutige Nachrichten‑ID (zufällig).
    pub msg_id: u128,
    /// Der Double‑Ratchet‑Header (PubKey, Count, Nonce).
    pub ratchet_header: Vec<u8>,
    /// Die eigentliche Nachricht (verschlüsselt durch Ratchet).
    pub encrypted_body: Vec<u8>,
}

impl Payload {
    /// Serialisiert die Nutzlast.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&self.msg_id.to_le_bytes());
        out.extend_from_slice(&(self.ratchet_header.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.ratchet_header);
        out.extend_from_slice(&(self.encrypted_body.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.encrypted_body);
        out
    }

    /// Deserialisiert die Nutzlast.
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        let mut cursor = 0;
        if data.len() < 16 + 4 + 4 { return None; }
        
        let msg_id = u128::from_le_bytes(data[cursor..cursor+16].try_into().ok()?);
        cursor += 16;
        
        let rh_len = u32::from_le_bytes(data[cursor..cursor+4].try_into().ok()?) as usize;
        cursor += 4;
        if cursor + rh_len > data.len() { return None; }
        let ratchet_header = data[cursor..cursor+rh_len].to_vec();
        cursor += rh_len;
        
        if cursor + 4 > data.len() { return None; }
        let eb_len = u32::from_le_bytes(data[cursor..cursor+4].try_into().ok()?) as usize;
        cursor += 4;
        if cursor + eb_len > data.len() { return None; }
        let encrypted_body = data[cursor..cursor+eb_len].to_vec();
        
        Some(Self { msg_id, ratchet_header, encrypted_body })
    }
}

/// Das Envelope ist die äußere Schale für den Transport über Relays.
#[derive(Debug, Clone)]
pub struct Envelope {
    pub ver: u8,
    pub ts: u64,
    pub ttl: u32,
    /// Ephemerer Public Key für den Schlüssel-Handshake.
    pub epk: [u8; 32],
    /// Stealth‑Tag zur Identifikation durch den Empfänger (HMAC).
    pub tag: [u8; 32],
use hmac::{Hmac, Mac};
use rand_core::{OsRng, RngCore};
use sha2::Sha256;
use x25519_dalek::{PublicKey, StaticSecret};
use chacha20poly1305::{KeyInit as AeadKeyInit, XChaCha20Poly1305, XNonce, aead::{Aead, Payload as AeadPayload}};

// ... (original code with errors) ...

impl Envelope {
    /// Erzeugt ein neues Envelope.
    pub fn new(
        recipient_spend_pub: &PublicKey,
        msg_id: u128,
        ratchet_header: Vec<u8>,
        encrypted_body: Vec<u8>,
        ttl: u32,
        pow_difficulty: u32,
    ) -> Self {
        // 1. Ephemerer Schlüssel für initialen Handshake/Tagging
        let eph_secret = StaticSecret::random_from_rng(&mut OsRng);
        let eph_public = PublicKey::from(&eph_secret);
        let epk_bytes = *eph_public.as_bytes();

        // 2. ECDH mit Empfänger-Spend-Pub
        let shared = eph_secret.diffie_hellman(recipient_spend_pub);
        
        // 3. HKDF zur Ableitung von Envelop-Encryption und Tagging
        let hk = Hkdf::<Sha256>::new(None, shared.as_bytes());
        let mut okm = [0u8; 64];
        hk.expand(b"PhantomChat-v1-Envelope", &mut okm).expect("HKDF expand");
        let enc_key = &okm[0..32];
        let tag_key = &okm[32..64];

        // 4. Stealth-Tag berechnen (HMAC über msg_id)
        let mut hmac = <Hmac<Sha256> as hmac::Mac>::new_from_slice(tag_key).expect("HMAC key");
        hmac.update(&msg_id.to_le_bytes());
        let tag: [u8; 32] = hmac.finalize().into_bytes().into();

        // ... (rest of the function is the same)

        let payload_bytes = payload.to_bytes();
        
        let mut nonce = [0u8; 24];
        OsRng.fill_bytes(&mut nonce);
        
        let cipher = XChaCha20Poly1305::new_from_slice(enc_key).expect("Cipher init");
        let ciphertext = cipher.encrypt(XNonce::from_slice(&nonce), AeadPayload {
            msg: &payload_bytes,
            aad: &tag, // Tag als Associated Data binden
        }).expect("Encryption failed");

        // 6. Proof-of-Work
        let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let mut pow_header = Vec::new();
        pow_header.extend_from_slice(&tag);
        pow_header.extend_from_slice(&ts.to_le_bytes());
        
        let pow = Hashcash::new(pow_difficulty);
        let pow_nonce = pow.compute_nonce(&pow_header);

        Self {
            ver: 1,
            ts,
            ttl,
            epk: epk_bytes,
            tag,
            pow_nonce,
            nonce,
            ciphertext,
        }
    }

    /// Serialisiert das Envelope für den Versand.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
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

    /// Deserialisiert ein Envelope.
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        let mut cursor = 0;
        if data.len() < 1 + 8 + 4 + 32 + 32 + 8 + 24 + 4 { return None; }
        
        let ver = data[cursor]; cursor += 1;
        let ts = u64::from_le_bytes(data[cursor..cursor+8].try_into().ok()?); cursor += 8;
        let ttl = u32::from_le_bytes(data[cursor..cursor+4].try_into().ok()?); cursor += 4;
        
        let mut epk = [0u8; 32];
        epk.copy_from_slice(&data[cursor..cursor+32]); cursor += 32;
        
        let mut tag = [0u8; 32];
        tag.copy_from_slice(&data[cursor..cursor+32]); cursor += 32;
        
        let pow_nonce = u64::from_le_bytes(data[cursor..cursor+8].try_into().ok()?); cursor += 8;
        
        let mut nonce = [0u8; 24];
        nonce.copy_from_slice(&data[cursor..cursor+24]); cursor += 24;
        
        let ct_len = u32::from_le_bytes(data[cursor..cursor+4].try_into().ok()?) as usize;
        cursor += 4;
        if cursor + ct_len > data.len() { return None; }
        let ciphertext = data[cursor..cursor+ct_len].to_vec();
        
        Some(Self { ver, ts, ttl, epk, tag, pow_nonce, nonce, ciphertext })
    }

    /// Versucht das Envelope mit einem Spend-Key zu öffnen (Discovery).
    pub fn open(&self, spend_key: &crate::keys::SpendKey) -> Option<Payload> {
        let shared = spend_key.secret.diffie_hellman(&PublicKey::from(self.epk));
        let hk = Hkdf::<Sha256>::new(None, shared.as_bytes());
        let mut okm = [0u8; 64];
        hk.expand(b"PhantomChat-v1-Envelope", &mut okm).ok()?;
        
        let enc_key = &okm[0..32];
        let cipher = XChaCha20Poly1305::new_from_slice(enc_key).ok()?;
        
        let decrypted = cipher.decrypt(XNonce::from_slice(&self.nonce), AeadPayload {
            msg: &self.ciphertext,
            aad: &self.tag,
        }).ok()?;
        
        Payload::from_bytes(&decrypted)
    }
}

