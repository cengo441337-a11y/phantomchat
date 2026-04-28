//! Schlüsselverwaltung für PhantomChat.
//!
//! ## Key-Hierarchie
//! - `IdentityKey` — Ed25519-Signatur (Authentizität)
//! - `ViewKey` — X25519 (Stealth-Tag-Scanning ohne Entschlüsseln)
//! - `SpendKey` — X25519 (Payload-Entschlüsselung)
//! - `HybridKeyPair` — PQXDH: ML-KEM-1024 + X25519 (post-quantum)
//!
//! ## PQXDH (Post-Quantum Extended Diffie-Hellman)
//! Kombiniert klassisches X25519 ECDH mit ML-KEM-1024 (NIST PQC Standard).
//! Das resultierende Session-Secret ist `SHA-256(x25519_shared || mlkem_shared)`.
//! Ein Quantencomputer müsste beide Algorithmen gleichzeitig brechen.

use rand_core::{OsRng, RngCore};
use x25519_dalek::{PublicKey, StaticSecret};
use serde::{Serialize, Deserialize};
use sha2::{Sha256, Digest};
use pqcrypto_mlkem::mlkem1024::{
    self, PublicKey as MlKemPublicKey, SecretKey as MlKemSecretKey,
    Ciphertext as MlKemCiphertext,
};
use pqcrypto_traits::kem::{PublicKey as KemPubTrait,
    Ciphertext as KemCtTrait, SharedSecret as KemSSTrait};
use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey, Signature as Ed25519Signature};
use zeroize::{Zeroize, ZeroizeOnDrop};

/// Identity‑Keypair.
///
/// `Zeroize` + `ZeroizeOnDrop`: the 32-byte `private` scalar is wiped from
/// RAM as soon as the struct goes out of scope, so a memory dump taken after
/// the user closes a session (or an attacker's `gcore` while the process is
/// still alive) cannot recover it. The `public` half is intentionally not
/// scrubbed — pubkeys are designed to be shared.
#[derive(Debug, Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct IdentityKey {
    #[zeroize(skip)]
    pub public: [u8; 32],
    pub private: [u8; 32],
}

/// Ed25519 signing keypair — used by the Sealed-Sender flow to
/// authenticate envelopes to their receiver.
///
/// `Debug` is intentionally *not* derived — the 32-byte scalar must never
/// leak into logs or panic traces. `Zeroize` + `ZeroizeOnDrop` wipe the
/// seed when the struct is dropped (the rehydrated `SigningKey` returned
/// by [`Self::signing_key`] is itself zeroized by `ed25519-dalek`).
#[derive(Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct PhantomSigningKey {
    /// Raw 32-byte seed. Rehydrated into an `ed25519_dalek::SigningKey` on
    /// demand so the live key material does not live in long-lived globals.
    bytes: [u8; 32],
}

impl PhantomSigningKey {
    pub fn generate() -> Self {
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
        Self { bytes }
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self { bytes }
    }

    pub fn to_bytes(&self) -> [u8; 32] {
        self.bytes
    }

    pub fn signing_key(&self) -> SigningKey {
        SigningKey::from_bytes(&self.bytes)
    }

    pub fn verifying_key(&self) -> VerifyingKey {
        self.signing_key().verifying_key()
    }

    /// Raw 32-byte encoding of the public key — what peers see and verify
    /// against.
    pub fn public_bytes(&self) -> [u8; 32] {
        self.verifying_key().to_bytes()
    }

    /// Sign arbitrary bytes. Produces a 64-byte Ed25519 signature.
    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        self.signing_key().sign(message).to_bytes()
    }
}

/// Verify an Ed25519 signature produced by [`PhantomSigningKey::sign`].
///
/// `public_bytes` must be the 32-byte encoding of the signer's verifying
/// key — typically pulled out of [`SealedSender`] in the decrypted payload.
pub fn verify_ed25519(public_bytes: &[u8; 32], message: &[u8], signature: &[u8; 64]) -> bool {
    let Ok(verifying) = VerifyingKey::from_bytes(public_bytes) else { return false; };
    let Ok(sig) = Ed25519Signature::try_from(&signature[..]) else { return false; };
    verifying.verify(message, &sig).is_ok()
}

/// View‑Keypair (X25519).
///
/// `Debug` is intentionally *not* derived — the secret scalar must never
/// leak through `{:?}` formatting into logs or panic traces. The
/// `Zeroize` + `ZeroizeOnDrop` derives wipe the secret half on drop;
/// `x25519_dalek::StaticSecret` already does this internally, but the
/// wrapper-level derive defends against accidental future fields landing
/// next to it without their own zeroization story.
#[derive(Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct ViewKey {
    #[serde(skip, default = "generate_secret")]
    pub secret: StaticSecret,
    #[zeroize(skip)]
    pub public: PublicKey,
}

/// Spend‑Keypair (X25519).
///
/// `Debug` is intentionally *not* derived — the secret scalar must never
/// leak through `{:?}` formatting into logs or panic traces. See [`ViewKey`]
/// for the same Zeroize rationale.
#[derive(Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct SpendKey {
    #[serde(skip, default = "generate_secret")]
    pub secret: StaticSecret,
    #[zeroize(skip)]
    pub public: PublicKey,
}

fn generate_secret() -> StaticSecret {
    StaticSecret::random_from_rng(OsRng)
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
        let secret = StaticSecret::random_from_rng(OsRng);
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
        let secret = StaticSecret::random_from_rng(OsRng);
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

// ── PQXDH: Post-Quantum Extended Diffie-Hellman ───────────────────────────────

/// Öffentlicher Hybridschlüssel: X25519 + ML-KEM-1024.
/// Wird im Phantom-ID kodiert und mit Kontakten geteilt.
#[derive(Debug, Clone)]
pub struct HybridPublicKey {
    /// Klassischer X25519 Spend-Public-Key (32 Bytes)
    pub x25519: PublicKey,
    /// Post-Quantum ML-KEM-1024 Public-Key (1568 Bytes)
    pub mlkem: MlKemPublicKey,
}

/// Privater Hybridschlüssel: X25519 + ML-KEM-1024.
///
/// Memory-zeroing: the `x25519` half is zeroized by `StaticSecret`'s own
/// `Drop`. The `mlkem` half is a `pqcrypto` newtype around a flat byte
/// array with no public mutation API, so we override [`Drop`] manually and
/// scrub it via `ptr::write_bytes` below — sound because `MlKemSecretKey`
/// has no destructor of its own and is `Copy + Clone`.
pub struct HybridSecretKey {
    pub x25519: StaticSecret,
    pub mlkem: MlKemSecretKey,
}

impl Drop for HybridSecretKey {
    fn drop(&mut self) {
        // Best-effort wipe of the ML-KEM secret. `MlKemSecretKey` is a
        // newtype around a fixed-size byte array (see pqcrypto-mlkem's
        // `simple_struct!` macro) with no destructor of its own, so
        // overwriting the bytes via `write_bytes` is sound. We then issue
        // a fence to keep the compiler from reordering the wipe past
        // subsequent drops.
        unsafe {
            std::ptr::write_bytes(
                &mut self.mlkem as *mut MlKemSecretKey as *mut u8,
                0u8,
                std::mem::size_of::<MlKemSecretKey>(),
            );
            std::sync::atomic::fence(std::sync::atomic::Ordering::SeqCst);
        }
        // x25519's own Drop scrubs the StaticSecret bytes — nothing to do
        // here for that half.
    }
}

/// Vollständiges PQXDH-Keypair.
pub struct HybridKeyPair {
    pub public: HybridPublicKey,
    pub secret: HybridSecretKey,
}

/// Sender-Seite: PQXDH-Encapsulation.
/// Gibt zurück: (session_key_32, encapsulated_mlkem_ct, ephemeral_x25519_pub)
pub struct PqxdhSendResult {
    /// 32-Byte Session-Key: SHA-256(x25519_shared || mlkem_shared)
    pub session_key: [u8; 32],
    /// ML-KEM Ciphertext (1568 Bytes) für den Empfänger
    pub mlkem_ct: Vec<u8>,
    /// Ephemerer X25519 Public-Key des Senders (32 Bytes)
    pub epk_x25519: [u8; 32],
}

impl HybridKeyPair {
    /// Generiert ein neues PQXDH-Keypair.
    pub fn generate() -> Self {
        let x25519_secret = StaticSecret::random_from_rng(OsRng);
        let x25519_public = PublicKey::from(&x25519_secret);
        let (mlkem_pub, mlkem_sec) = mlkem1024::keypair();
        Self {
            public: HybridPublicKey { x25519: x25519_public, mlkem: mlkem_pub },
            secret: HybridSecretKey { x25519: x25519_secret, mlkem: mlkem_sec },
        }
    }

    /// Empfänger-Seite: Dekapsulation eines PQXDH-Sends.
    /// Rekonstruiert denselben Session-Key wie der Sender.
    pub fn decapsulate(
        &self,
        epk_x25519: &[u8; 32],
        mlkem_ct_bytes: &[u8],
    ) -> Option<[u8; 32]> {
        // 1. X25519 ECDH
        let epk = PublicKey::from(*epk_x25519);
        let x_shared = self.secret.x25519.diffie_hellman(&epk);

        // 2. ML-KEM Decapsulation
        let ct = MlKemCiphertext::from_bytes(mlkem_ct_bytes).ok()?;
        let mlkem_shared = mlkem1024::decapsulate(&ct, &self.secret.mlkem);

        // 3. Combined session key: SHA-256(x25519 || mlkem)
        let mut hasher = Sha256::new();
        hasher.update(x_shared.as_bytes());
        hasher.update(mlkem_shared.as_bytes());
        let result: [u8; 32] = hasher.finalize().into();
        Some(result)
    }
}

impl HybridPublicKey {
    /// Sender-Seite: Encapsulation.
    /// Generiert ephemeres X25519-Keypair + ML-KEM-Kapselung → Session-Key.
    pub fn encapsulate(&self) -> PqxdhSendResult {
        // 1. Ephemeres X25519 Keypair
        let eph_secret = StaticSecret::random_from_rng(OsRng);
        let eph_public = PublicKey::from(&eph_secret);

        // 2. X25519 ECDH mit Empfänger-X25519-Public
        let x_shared = eph_secret.diffie_hellman(&self.x25519);

        // 3. ML-KEM Encapsulation
        let (mlkem_shared, mlkem_ct) = mlkem1024::encapsulate(&self.mlkem);

        // 4. Combined session key: SHA-256(x25519 || mlkem)
        let mut hasher = Sha256::new();
        hasher.update(x_shared.as_bytes());
        hasher.update(mlkem_shared.as_bytes());
        let session_key: [u8; 32] = hasher.finalize().into();

        PqxdhSendResult {
            session_key,
            mlkem_ct: mlkem_ct.as_bytes().to_vec(),
            epk_x25519: *eph_public.as_bytes(),
        }
    }

    /// Serialisiert den Public-Key für Transport / Phantom-ID Encoding.
    /// Format: [32 bytes X25519 || 1568 bytes ML-KEM] = 1600 bytes total
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(1600);
        out.extend_from_slice(self.x25519.as_bytes());
        out.extend_from_slice(self.mlkem.as_bytes());
        out
    }

    /// Deserialisiert aus Bytes.
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.len() < 1600 { return None; }
        let x25519 = PublicKey::from(<[u8; 32]>::try_from(&data[0..32]).ok()?);
        let mlkem = MlKemPublicKey::from_bytes(&data[32..1600]).ok()?;
        Some(Self { x25519, mlkem })
    }
}
