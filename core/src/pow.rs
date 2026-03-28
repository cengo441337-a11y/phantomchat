//! Proof‑of‑Work (Hashcash) für PhantomChat.
//!
//! Um Spam zu verhindern, muss der Sender einen Nonce finden, so dass der
//! SHA‑256‑Hash über bestimmte Headerfelder mit einer konfigurierbaren
//! Anzahl führender Nullbits beginnt.  Die Schwierigkeit wird in
//! `target_zero_bits` angegeben.  Diese Implementierung ist bewusst
//! einfach gehalten und sollte für den produktiven Einsatz optimiert
//! werden (z. B. durch parallele Berechnung).

use crate::util::{leading_zero_bits, sha256};

/// Struktur zur Berechnung und Verifikation des Proof‑of‑Work.
pub struct Hashcash {
    /// Anzahl der führenden Nullbits, die der Hash aufweisen muss.
    pub target_zero_bits: u32,
}

impl Hashcash {
    /// Erstellt eine neue Hashcash‑Instanz mit gegebener Schwierigkeit.
    pub fn new(target_zero_bits: u32) -> Self {
        Self { target_zero_bits }
    }
    /// Berechnet einen Nonce für die gegebenen Daten.  Die Daten sollten
    /// die seriellen Headerfelder (Version, Timestamp, Ephemeral‑Key,
    /// Tag) enthalten.  Es wird ein 64‑Bit‑Nonce zurückgegeben.
    pub fn compute_nonce(&self, data: &[u8]) -> u64 {
        let mut nonce: u64 = 0;
        loop {
            let mut buf = Vec::with_capacity(data.len() + 8);
            buf.extend_from_slice(data);
            buf.extend_from_slice(&nonce.to_le_bytes());
            let hash = sha256(&buf);
            if leading_zero_bits(&hash) >= self.target_zero_bits {
                return nonce;
            }
            nonce = nonce.wrapping_add(1);
        }
    }
    /// Verifiziert den Nonce für die gegebenen Daten.
    pub fn verify(&self, data: &[u8], nonce: u64) -> bool {
        let mut buf = Vec::with_capacity(data.len() + 8);
        buf.extend_from_slice(data);
        buf.extend_from_slice(&nonce.to_le_bytes());
        let hash = sha256(&buf);
        leading_zero_bits(&hash) >= self.target_zero_bits
    }
}
