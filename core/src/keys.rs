//! Schlüsselverwaltung für PhantomChat.
//!
//! Jede Instanz besitzt drei Schlüsselpaare:
//!
//! * Eine **Identity‑Key** (id_key), die zur Authentisierung der App
//!   verwendet wird.  In späteren Versionen könnte hier Ed25519 zum
//!   Signieren verwendet werden.
//! * Einen **View‑Key** (view_key), bestehend aus privatem und öffentlichem
//!   X25519‑Schlüssel.  Der Empfänger nutzt den privaten view_key, um
//!   aus eingehenden Envelopes das HMAC‑Tag zu reproduzieren und so seine
//!   Nachrichten zu identifizieren.
//! * Einen **Spend‑Key** (spend_key), ebenfalls bestehend aus einem
//!   X25519‑Keypair.  Nur der spend_key erlaubt das Entschlüsseln der
//!   Nutzlast.
//!
//! Die tatsächliche Implementierung nutzt x25519‑dalek, um die
//! Schlüssel zu generieren und Diffie‑Hellman durchzuführen.  Für eine
//! produktionsreife Version sollte das Handling der Schlüssel (z.&nbsp;B.
//! Serialisierung, Zeroization) sorgfältig implementiert werden.

use rand_core::{OsRng, RngCore};
use x25519_dalek::{PublicKey, StaticSecret};
use serde::{Serialize, Deserialize};

/// Identity‑Keypair.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityKey {
    pub public: [u8; 32],
    pub private: [u8; 32],
}

/// View‑Keypair (X25519)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewKey {
    #[serde(skip, default = "generate_secret")]
    pub secret: StaticSecret,
    pub public: PublicKey,
}

/// Spend‑Keypair (X25519)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpendKey {
    #[serde(skip, default = "generate_secret")]
    pub secret: StaticSecret,
    pub public: PublicKey,
}

fn generate_secret() -> StaticSecret {
    StaticSecret::new(&mut OsRng)
}

impl IdentityKey {
    /// Erzeugt ein neues Identity‑Keypair mit zufälligem privaten Schlüssel.
    pub fn generate() -> Self {
        let mut priv_bytes = [0u8; 32];
        OsRng.fill_bytes(&mut priv_bytes);
        // In einer echten Implementierung würde hier aus priv_bytes ein
        // Ed25519‑ oder andere Identity‑Key abgeleitet werden.  Zur
        // Vereinfachung speichern wir die Bytes direkt.
        let public = priv_bytes; // Platzhalter: Identity‑Key = priv
        Self { public, private: priv_bytes }
    }
}

impl ViewKey {
    /// Erzeugt ein neues View‑Keypair.
    pub fn generate() -> Self {
        let secret = StaticSecret::new(&mut OsRng);
        let public = PublicKey::from(&secret);
        Self { secret, public }
    }
    /// Berechnet ein gemeinsames Geheimnis mit dem Spend‑Key des
    /// Empfängers.  Dieses Geheimnis dient als Input für HKDF.
    pub fn ecdh(&self, remote: &SpendKey) -> [u8; 32] {
        let shared = self.secret.diffie_hellman(&remote.public);
        *shared.as_bytes()
    }
}

impl SpendKey {
    /// Erzeugt ein neues Spend‑Keypair.
    pub fn generate() -> Self {
        let secret = StaticSecret::new(&mut OsRng);
        let public = PublicKey::from(&secret);
        Self { secret, public }
    }
    /// Berechnet ein gemeinsames Geheimnis mit dem Ephemeral‑Key des
    /// Senders.
    pub fn ecdh(&self, remote_epk: &PublicKey) -> [u8; 32] {
        let shared = self.secret.diffie_hellman(remote_epk);
        *shared.as_bytes()
    }
}
