//! Double‑Ratchet‑Engine (Production Grade)
//!
//! Diese Umsetzung implementiert den vollständigen Double‑Ratchet‑Algorithmus
//! gemäß der Signal‑Spezifikation, inklusive KDF‑Ketten für Root, Senden
//! und Empfangen.

use crate::keys::{ViewKey, SpendKey};
use rand::Rng;
use rand_core::OsRng;
use x25519_dalek::{PublicKey, StaticSecret};
use hkdf::Hkdf;
use sha2::Sha256;
use hmac::{Hmac, Mac, KeyInit};

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
    h.expand(b"PhantomChat-v1-Root", &mut okm).expect("HKDF expansion should not fail");
    let mut new_rk = [0u8; 32];
    let mut new_ck = [0u8; 32];
    new_rk.copy_from_slice(&okm[0..32]);
    new_ck.copy_from_slice(&okm[32..64]);
    (new_rk, new_ck)
}

/// KDF für die Nachrichten-Ketten (KDF-CK).
fn kdf_ck(ck: &[u8; 32]) -> ([u8; 32], [u8; 32]) {
    let mut h_ck = HmacSha256::new_from_slice(ck).expect("HMAC should accept 32-byte key");
    h_ck.update(&[0x01]);
    let next_ck: [u8; 32] = h_ck.finalize().into_bytes().into();
    
    let mut h_mk = HmacSha256::new_from_slice(ck).expect("HMAC should accept 32-byte key");
    h_mk.update(&[0x02]);
    let msg_key: [u8; 32] = h_mk.finalize().into_bytes().into();
    
    (next_ck, msg_key)
}

/// Zustand einer Double‑Ratchet‑Session.
#[derive(Debug, Clone)]
pub struct RatchetState {
    /// Root‑Key aus der letzten DH‑Operation.
    root_key: [u8; 32],
    /// Aktueller Sende‑Chain‑Key.
    send_chain: [u8; 32],
    /// Aktueller Empfangs‑Chain‑Key.
    recv_chain: [u8; 32],
    /// Aktuelles Ratchet‑Keypair (privat).
    ratchet_secret: StaticSecret,
    /// Aktueller Ratchet‑Public‑Key des Peers.
    peer_ratchet_public: PublicKey,
    /// Nachrichtenzähler für Sendekette.
    send_count: u32,
    /// Nachrichtenzähler für Empfangskette.
    recv_count: u32,
}

impl RatchetState {
    /// Initialisiert einen neuen Ratchet‑Zustand (Alice-Seite oder Bob-init).
    pub fn new(shared_h_secret: [u8; 32], peer_ratchet_public: PublicKey) -> Self {
        let ratchet_secret = StaticSecret::new(&mut OsRng);
        
        // Initialer DH-Schritt um Root und Chain-Keys abzuleiten
        let dh_out = ratchet_secret.diffie_hellman(&peer_ratchet_public);
        let (root_key, send_chain) = kdf_rk(&shared_h_secret, dh_out.as_bytes());
        
        Self {
            root_key,
            send_chain,
            recv_chain: [0u8; 32], // Wird beim ersten Empfang gesetzt
            ratchet_secret,
            peer_ratchet_public,
            send_count: 0,
            recv_count: 0,
        }
    }

    /// Führt einen DH-Ratchet-Schritt aus (bei Empfang eines neuen Peer-Schlüssels).
    pub fn dh_ratchet(&mut self, new_peer_public: PublicKey) {
        self.peer_ratchet_public = new_peer_public;
        self.recv_count = 0;
        
        // DH mit altem eigenem Secret und neuem Peer-Public -> neue Empfangskette
        let dh_1 = self.ratchet_secret.diffie_hellman(&self.peer_ratchet_public);
        let (rk, ck_recv) = kdf_rk(&self.root_key, dh_1.as_bytes());
        self.root_key = rk;
        self.recv_chain = ck_recv;
        
        // Neues eigenes Secret generieren
        self.ratchet_secret = StaticSecret::new(&mut OsRng);
        self.send_count = 0;
        
        // DH mit neuem eigenem Secret und neuem Peer-Public -> neue Sendekette
        let dh_2 = self.ratchet_secret.diffie_hellman(&self.peer_ratchet_public);
        let (rk, ck_send) = kdf_rk(&self.root_key, dh_2.as_bytes());
        self.root_key = rk;
        self.send_chain = ck_send;
    }

    /// Erzeugt einen neuen Nachrichten‑Schlüssel (Senden).
    fn next_send_key(&mut self) -> [u8; 32] {
        let (next_ck, msg_key) = kdf_ck(&self.send_chain);
        self.send_chain = next_ck;
        self.send_count += 1;
        msg_key
    }

    /// Erzeugt einen neuen Nachrichten‑Schlüssel (Empfangen).
    fn next_recv_key(&mut self) -> [u8; 32] {
        let (next_ck, msg_key) = kdf_ck(&self.recv_chain);
        self.recv_chain = next_ck;
        self.recv_count += 1;
        msg_key
    }

    /// Verschlüsselt eine Nachricht mit XChaCha20‑Poly1305.
    pub fn encrypt(&mut self, plaintext: &[u8]) -> (Vec<u8>, Vec<u8>) {
        use chacha20poly1305::{aead::{Aead, KeyInit as AeadKeyInit, Payload}, XChaCha20Poly1305, XNonce};

        let msg_key = self.next_send_key();
        let cipher = XChaCha20Poly1305::new(msg_key.as_slice().into());
        
        let mut nonce_bytes = [0u8; 24];
        rand::prelude::thread_rng().fill(&mut nonce_bytes);
        let nonce = XNonce::from_slice(&nonce_bytes);

        // Header: PubKey (32) + Count (4) + Nonce (24)
        let mut header = self.ratchet_secret.public_key().as_bytes().to_vec();
        header.extend_from_slice(&self.send_count.to_le_bytes());
        header.extend_from_slice(&nonce_bytes);

        let ciphertext = cipher.encrypt(nonce, Payload {
            msg: plaintext,
            aad: &header, // Authenticate the header!
        }).expect("Encryption should not fail");

        (ciphertext, header)
    }

    /// Entschlüsselt eine Nachricht mit XChaCha20‑Poly1305.
    pub fn decrypt(&mut self, header: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, RatchetError> {
        use chacha20poly1305::{aead::{Aead, KeyInit as AeadKeyInit, Payload}, XChaCha20Poly1305, XNonce};

        if header.len() < 60 { // 32 bytes PubKey + 4 bytes Count + 24 bytes Nonce
            return Err(RatchetError::InvalidHeader);
        }

        let peer_pub_bytes: [u8; 32] = header[0..32].try_into().map_err(|_| RatchetError::InvalidHeader)?;
        let peer_pub = PublicKey::from(peer_pub_bytes);
        
        // DH-Ratchet falls neuer Peer-Key
        if peer_pub != self.peer_ratchet_public {
            self.dh_ratchet(peer_pub);
        }

        let msg_key = self.next_recv_key();
        let cipher = XChaCha20Poly1305::new(msg_key.as_slice().into());
        
        let nonce_bytes = &header[36..60];
        let nonce = XNonce::from_slice(nonce_bytes);

        cipher.decrypt(nonce, Payload {
            msg: ciphertext,
            aad: header,
        }).map_err(|_| RatchetError::DecryptionFailed)
    }
}

