//! Envelope‑Struktur und Serialisierung für PhantomChat.
//!
//! Das Envelope kapselt alle notwendigen Metadaten und die
//! verschlüsselte Nutzlast.  Die Serialisierung erfolgt in einem
//! einfachen length‑prefixed‑Format.  Bei der Erstellung werden
//! Verschlüsselungs‑ und Tag‑Schlüssel aus einem ECDH‑Geheimnis mittels
//! HKDF abgeleitet.  Anschließend wird die Payload mit
//! XChaCha20‑Poly1305 verschlüsselt, und es wird ein Proof‑of‑Work
//! berechnet.

use crate::keys::{SpendKey, ViewKey};
use crate::pow::Hashcash;
use crate::util::sha256;
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use rand_core::{OsRng, RngCore};
use sha2::Sha256;
use x25519_dalek::{PublicKey, StaticSecret};
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};

/// Struktur der Klartextnutzlast.  Für die Demonstration ist die
/// Serialisierung sehr einfach gehalten: Alle Felder werden in der
/// gegebenen Reihenfolge hintereinander als Big‑Endian‑Bytes
/// geschrieben; Strings werden mit ihrer Länge und anschließendem
/// UTF‑8‑Inhalt kodiert.
#[derive(Debug, Clone)]
pub struct Payload {
    pub msg_id: u128,
    pub sender_fp: u32,
    pub ratchet_header: Vec<u8>,
    pub body: Vec<u8>,
}

impl Payload {
    /// Serialisiert die Nutzlast in einen Byte‑Vektor.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&self.msg_id.to_le_bytes());
        out.extend_from_slice(&self.sender_fp.to_le_bytes());
        out.extend_from_slice(&(self.ratchet_header.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.ratchet_header);
        out.extend_from_slice(&(self.body.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.body);
        out
    }
    /// Deserialisiert eine Nutzlast aus einem Byte‑Slice.  Diese
    /// Funktion ist lediglich ein Beispiel und führt keine robuste
    /// Fehlerbehandlung durch.
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.len() < 16 + 4 { return None; }
        let msg_id = u128::from_le_bytes(data[0..16].try_into().ok()?);
        let sender_fp = u32::from_le_bytes(data[16..20].try_into().ok()?);
        let mut cursor = 20;
        let rh_len = u32::from_le_bytes(data[cursor..cursor+4].try_into().ok()?) as usize;
        cursor += 4;
        if cursor + rh_len > data.len() { return None; }
        let ratchet_header = data[cursor..cursor+rh_len].to_vec();
        cursor += rh_len;
        if cursor + 4 > data.len() { return None; }
        let body_len = u32::from_le_bytes(data[cursor..cursor+4].try_into().ok()?) as usize;
        cursor += 4;
        if cursor + body_len > data.len() { return None; }
        let body = data[cursor..cursor+body_len].to_vec();
        Some(Self { msg_id, sender_fp, ratchet_header, body })
    }
}

/// Envelope beinhaltet alle Metadaten und den verschlüsselten Payload.
#[derive(Debug, Clone)]
pub struct Envelope {
    pub ver: u8,
    pub ts: u64,
    pub ttl: u32,
    pub epk: [u8; 32],
    pub tag: Vec<u8>,
    pub pow_nonce: u64,
    pub nonce: [u8; 24],
    pub ciphertext: Vec<u8>,
    pub mac: [u8; 16],
}

impl Envelope {
    /// Erzeugt ein neues Envelope aus View/Spend‑Schlüsseln, Payload und
    /// Konfigurationsparametern.  Diese Funktion führt folgende
    /// Schritte aus:
    /// 1. Generiert einen zufälligen ephemeren Secret und Public Key.
    /// 2. Berechnet das ECDH‑Geheimnis K = ECDH(ephemeral, spend_pub).
    /// 3. Leitet mittels HKDF `enc_key` und `tag_key` ab.
    /// 4. Berechnet `tag` = HMAC(tag_key, msg_id).
    /// 5. Serialisiert den Payload und verschlüsselt ihn mit
    ///    XChaCha20‑Poly1305 unter Verwendung von `enc_key` und einem
    ///    zufälligen Nonce.
    /// 6. Berechnet ein Proof‑of‑Work über die Header und den Nonce.
    pub fn new(
        spend_pub: &PublicKey,
        msg_id: u128,
        sender_fp: u32,
        ratchet_header: Vec<u8>,
        body: Vec<u8>,
        ttl: u32,
        pow_difficulty: u32,
    ) -> Self {
        // 1. Ephemerer Schlüssel
        let eph_secret = StaticSecret::new(&mut OsRng);
        let eph_public = PublicKey::from(&eph_secret);
        let epk_bytes = *eph_public.as_bytes();
        // 2. ECDH mit spend_pub
        let shared = eph_secret.diffie_hellman(spend_pub);
        let shared_bytes = shared.as_bytes();
        // 3. HKDF zur Ableitung von enc_key und tag_key
        let hk = Hkdf::<Sha256>::new(None, shared_bytes);
        let mut okm = [0u8; 64];
        hk.expand(b"pc.enc|pc.tag", &mut okm).expect("HKDF expand");
        let enc_key = &okm[..32];
        let tag_key = &okm[32..64];
        // 4. HMAC‑Tag über msg_id
        let mut mac = <Hmac<Sha256>>::new_from_slice(tag_key).expect("HMAC key");
        mac.update(&msg_id.to_le_bytes());
        let tag_bytes = mac.finalize().into_bytes().to_vec();
        // 5. Payload serialisieren und verschlüsseln
        let payload = Payload { msg_id, sender_fp, ratchet_header, body };
        let payload_bytes = payload.to_bytes();
        // Zufälliger Nonce für XChaCha20
        let mut nonce = [0u8; 24];
        OsRng.fill_bytes(&mut nonce);
        let cipher = XChaCha20Poly1305::new_from_slice(enc_key).expect("cipher");
        let ciphertext = cipher.encrypt(XNonce::from_slice(&nonce), payload_bytes.as_ref()).expect("encrypt");
        // extrahiere Poly1305‑Tag (letzte 16 Bytes)
        let (ciphertext_body, auth_tag) = ciphertext.split_at(ciphertext.len() - 16);
        let mut mac_arr = [0u8; 16];
        mac_arr.copy_from_slice(auth_tag);
        // 6. Proof‑of‑Work
        let mut header = Vec::new();
        header.extend_from_slice(&1u8.to_le_bytes());
        header.extend_from_slice(&0u64.to_le_bytes()); // ts placeholder
        header.extend_from_slice(&0u32.to_le_bytes()); // ttl placeholder
        header.extend_from_slice(&epk_bytes);
        header.extend_from_slice(&tag_bytes);
        let pow = Hashcash::new(pow_difficulty);
        let nonce_pow = pow.compute_nonce(&header);
        let ts = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64);
        Self {
            ver: 1,
            ts,
            ttl,
            epk: epk_bytes,
            tag: tag_bytes,
            pow_nonce: nonce_pow,
            nonce,
            ciphertext: ciphertext_body.to_vec(),
            mac: mac_arr,
        }
    }
    /// Serialisiert das Envelope in eine Bytefolge.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.push(self.ver);
        out.extend_from_slice(&self.ts.to_le_bytes());
        out.extend_from_slice(&self.ttl.to_le_bytes());
        out.extend_from_slice(&self.epk);
        out.extend_from_slice(&(self.tag.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.tag);
        out.extend_from_slice(&self.pow_nonce.to_le_bytes());
        out.extend_from_slice(&self.nonce);
        out.extend_from_slice(&(self.ciphertext.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.ciphertext);
        out.extend_from_slice(&self.mac);
        out
    }
    /// Deserialisiert ein Envelope aus einem Byte‑Slice.  Diese
    /// Implementierung ist beispielhaft und prüft keine Integrität.
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        let mut cursor = 0;
        if data.len() < 1 + 8 + 4 + 32 + 4 + 8 + 24 + 4 + 16 {
            return None;
        }
        let ver = data[cursor];
        cursor += 1;
        let ts = u64::from_le_bytes(data[cursor..cursor+8].try_into().ok()?);
        cursor += 8;
        let ttl = u32::from_le_bytes(data[cursor..cursor+4].try_into().ok()?);
        cursor += 4;
        let mut epk = [0u8; 32];
        epk.copy_from_slice(&data[cursor..cursor+32]);
        cursor += 32;
        let tag_len = u32::from_le_bytes(data[cursor..cursor+4].try_into().ok()?) as usize;
        cursor += 4;
        if cursor + tag_len > data.len() { return None; }
        let tag = data[cursor..cursor+tag_len].to_vec();
        cursor += tag_len;
        let pow_nonce = u64::from_le_bytes(data[cursor..cursor+8].try_into().ok()?);
        cursor += 8;
        let mut nonce = [0u8; 24];
        nonce.copy_from_slice(&data[cursor..cursor+24]);
        cursor += 24;
        let c_len = u32::from_le_bytes(data[cursor..cursor+4].try_into().ok()?) as usize;
        cursor += 4;
        if cursor + c_len + 16 > data.len() { return None; }
        let ciphertext = data[cursor..cursor+c_len].to_vec();
        cursor += c_len;
        let mut mac = [0u8; 16];
        mac.copy_from_slice(&data[cursor..cursor+16]);
        Some(Self { ver, ts, ttl, epk, tag, pow_nonce, nonce, ciphertext, mac })
    }
    /// Entschlüsselt die Nutzlast, sofern der Empfänger über den passenden
    /// Spend‑Key verfügt.  Es werden das ECDH‑Geheimnis und HKDF
    /// verwendet, um den `enc_key` zu rekonstruieren.  Anschließend
    /// erfolgt die AEAD‑Entschlüsselung.  Bei Erfolg wird die
    /// deserialisierte Payload zurückgegeben.
    pub fn decrypt(&self, spend_key: &SpendKey) -> Option<Payload> {
        // 1. Gemeinsames Geheimnis berechnen
        let remote_epk = PublicKey::from(self.epk);
        let shared_bytes = spend_key.secret.diffie_hellman(&remote_epk).as_bytes().clone();
        // 2. HKDF ableiten
        let hk = Hkdf::<Sha256>::new(None, &shared_bytes);
        let mut okm = [0u8; 64];
        hk.expand(b"pc.enc|pc.tag", &mut okm).ok()?;
        let enc_key = &okm[..32];
        // 3. AEAD entschlüsseln
        let cipher = XChaCha20Poly1305::new_from_slice(enc_key).ok()?;
        let mut ct = self.ciphertext.clone();
        ct.extend_from_slice(&self.mac);
        let decrypted = cipher.decrypt(XNonce::from_slice(&self.nonce), ct.as_ref()).ok()?;
        Payload::from_bytes(&decrypted)
    }
    /// Prüft, ob dieses Envelope für den Empfänger bestimmt ist.  Dazu
    /// wird aus dem Spend‑Key das Tag‑Key rekonstruiert und ein HMAC
    /// über die `msg_id` gebildet.  Stimmt das Ergebnis, ist die
    /// Nachricht für den Empfänger bestimmt.  Da die `msg_id` in der
    /// verschlüsselten Payload steckt, muss diese Methode nach dem
    /// Entschlüsseln aufgerufen werden.
    pub fn verify_recipient(&self, spend_key: &SpendKey, msg_id: u128) -> bool {
        let remote_epk = PublicKey::from(self.epk);
        let shared_bytes = spend_key.secret.diffie_hellman(&remote_epk).as_bytes().clone();
        let hk = Hkdf::<Sha256>::new(None, &shared_bytes);
        let mut okm = [0u8; 64];
        if hk.expand(b"pc.enc|pc.tag", &mut okm).is_err() { return false; }
        let tag_key = &okm[32..64];
        let mut mac = <Hmac<Sha256>>::new_from_slice(tag_key).expect("HMAC key");
        mac.update(&msg_id.to_le_bytes());
        let calculated = mac.finalize().into_bytes();
        calculated.as_slice() == self.tag.as_slice()
    }
}
