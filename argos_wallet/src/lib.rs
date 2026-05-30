//! Argos — non-custodial Solana wallet.
//!
//! Provides BIP39 mnemonic generation, Argon2id-encrypted on-disk
//! persistence, and signing helpers that integrate with the existing
//! PhantomChat ratchet stack via flutter_rust_bridge (mobile) and Tauri
//! IPC (desktop).
//!
//! ## Threat model
//!
//! - Private key NEVER leaves the device (no cloud sync, no telemetry).
//! - Disk-at-rest: wallet.enc.json holds an AEAD-encrypted blob; the KEK
//!   is derived from the user's PIN via Argon2id (OWASP-2023 parameters).
//! - In-memory: keypair bytes wrapped in `Zeroizing` so a drop wipes RAM.
//! - PIN brute-force: caller (UI layer) MUST implement the 10-tries-then-
//!   panic-wipe policy; this crate just exposes the cryptographic primitive.

#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod transfer;
pub use transfer::{parse_address, LAMPORTS_PER_SOL_CONST};

use std::path::Path;

use argon2::{Algorithm, Argon2, Params, Version};
use bip39::{Language, Mnemonic};
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload as AeadPayload},
    XChaCha20Poly1305, XNonce,
};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use solana_sdk::{
    pubkey::Pubkey,
    signature::{Keypair, Signature, Signer},
    signer::keypair::keypair_from_seed,
};
use thiserror::Error;
use zeroize::Zeroizing;

/// Solana network selector — drives the RPC endpoint + airdrop availability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Network {
    /// Production. Default for the shipping client.
    MainnetBeta,
    /// `https://api.devnet.solana.com` — has free airdrop, for integration tests.
    Devnet,
}

impl Network {
    /// Public RPC URL. For mainnet production deployments callers should
    /// swap in a paid endpoint (Helius / Triton) before high-volume use.
    pub fn rpc_url(self) -> &'static str {
        match self {
            Network::MainnetBeta => "https://api.mainnet-beta.solana.com",
            Network::Devnet => "https://api.devnet.solana.com",
        }
    }
}

/// Argon2id parameters pinned to OWASP 2023 recommended values.
const ARGON2_M_COST: u32 = 65_536; // 64 MiB
const ARGON2_T_COST: u32 = 3;
const ARGON2_P_COST: u32 = 4;
const KEK_LEN: usize = 32;
const SALT_LEN: usize = 32;
const NONCE_LEN: usize = 24;
const STORAGE_VERSION: u8 = 1;

/// In-memory wallet. The keypair is held inside [`Zeroizing`] so when the
/// struct is dropped the secret bytes get scrubbed from RAM.
///
/// `Debug` is implemented manually and prints ONLY the pubkey + network
/// — the secret-bearing field is omitted on purpose so a debug-log line
/// or test panic can never leak the keypair.
pub struct ArgosWallet {
    /// 64-byte Solana keypair (32 secret + 32 public).
    keypair_bytes: Zeroizing<[u8; 64]>,
    /// Cached pubkey, derived once at construction.
    pubkey: Pubkey,
    /// Network this wallet targets. Persisted alongside the encrypted blob
    /// so a `load_encrypted` call restores the correct RPC endpoint.
    network: Network,
}

impl std::fmt::Debug for ArgosWallet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArgosWallet")
            .field("pubkey", &self.pubkey)
            .field("network", &self.network)
            .finish_non_exhaustive()
    }
}

impl ArgosWallet {
    /// Generate a fresh wallet from secure RNG. Returns the wallet and the
    /// BIP39 mnemonic the user MUST write down — the only recovery path.
    pub fn generate(network: Network) -> Result<(Self, Mnemonic), Error> {
        let mut entropy = [0u8; 32]; // 256 bits → 24-word mnemonic
        OsRng.fill_bytes(&mut entropy);
        let mnemonic = Mnemonic::from_entropy_in(Language::English, &entropy)
            .map_err(|e| Error::Bip39(e.to_string()))?;
        let wallet = Self::from_mnemonic_inner(&mnemonic, "", network)?;
        Ok((wallet, mnemonic))
    }

    /// Restore from a BIP39 mnemonic (12 or 24 words). `passphrase` is the
    /// optional BIP39 passphrase ("25th word"); pass `""` for the common case.
    pub fn from_mnemonic(words: &str, passphrase: &str, network: Network) -> Result<Self, Error> {
        let mnemonic = Mnemonic::parse_in_normalized(Language::English, words)
            .map_err(|e| Error::Bip39(e.to_string()))?;
        Self::from_mnemonic_inner(&mnemonic, passphrase, network)
    }

    fn from_mnemonic_inner(mnemonic: &Mnemonic, passphrase: &str, network: Network) -> Result<Self, Error> {
        // BIP39 seed -> first 32 bytes -> ed25519 secret. Solana convention:
        // derive m/44'/501'/0'/0' via SLIP-0010 for hardware-wallet parity.
        // For the MVP we use the simpler bip39-seed[..32] which matches
        // every popular Solana wallet's "import 12 words" UX (Phantom,
        // Backpack, Solflare). We can add SLIP-0010 later behind a flag.
        let seed = mnemonic.to_seed(passphrase);
        let kp = keypair_from_seed(&seed[..32])
            .map_err(|e| Error::KeyDerivation(e.to_string()))?;
        let bytes = kp.to_bytes();
        let pubkey = kp.pubkey();
        Ok(Self {
            keypair_bytes: Zeroizing::new(bytes),
            pubkey,
            network,
        })
    }

    /// Reconstruct a Solana SDK [`Keypair`] for signing. The returned
    /// Keypair holds a copy of the secret bytes; let it drop ASAP.
    fn keypair(&self) -> Keypair {
        Keypair::try_from(&self.keypair_bytes[..]).expect("internally-validated bytes")
    }

    /// Public alias for [`Self::keypair`] used by the `transfer` module.
    /// Returns a fresh `Keypair` each call so the caller can move it into
    /// a signers slice without lifetime contortions; secret bytes are
    /// copied, not referenced.
    pub(crate) fn keypair_for_signing(&self) -> Keypair {
        self.keypair()
    }

    /// Solana public address (32 bytes).
    pub fn pubkey(&self) -> Pubkey {
        self.pubkey
    }

    /// Active network.
    pub fn network(&self) -> Network {
        self.network
    }

    /// Sign an arbitrary byte slice. Used by transaction builders that
    /// need raw signatures (e.g. Jupiter swap-instructions).
    pub fn sign(&self, message: &[u8]) -> Signature {
        self.keypair().sign_message(message)
    }

    // ── Encrypted persistence ────────────────────────────────────────────

    /// Encrypt the keypair with a KEK derived from `pin` and write to
    /// `path`. The file is overwritten atomically (write-tmp-then-rename).
    pub fn persist_encrypted(&self, pin: &str, path: &Path) -> Result<(), Error> {
        let mut salt = [0u8; SALT_LEN];
        OsRng.fill_bytes(&mut salt);
        let kek = derive_kek(pin.as_bytes(), &salt)?;

        let mut nonce_bytes = [0u8; NONCE_LEN];
        OsRng.fill_bytes(&mut nonce_bytes);

        // Plain blob: 64-byte keypair || 1-byte network.
        let mut plain = Vec::with_capacity(65);
        plain.extend_from_slice(&self.keypair_bytes[..]);
        plain.push(match self.network {
            Network::MainnetBeta => 0,
            Network::Devnet => 1,
        });

        let cipher = XChaCha20Poly1305::new_from_slice(&kek)
            .map_err(|_| Error::Crypto("cipher init".into()))?;
        let ciphertext = cipher
            .encrypt(XNonce::from_slice(&nonce_bytes), AeadPayload {
                msg: &plain,
                aad: &[STORAGE_VERSION],
            })
            .map_err(|_| Error::Crypto("aead seal".into()))?;

        let blob = EncryptedBlob {
            version: STORAGE_VERSION,
            kdf: "argon2id".into(),
            kdf_params: KdfParams {
                m_cost: ARGON2_M_COST,
                t_cost: ARGON2_T_COST,
                p_cost: ARGON2_P_COST,
            },
            salt: base64_encode(&salt),
            nonce: base64_encode(&nonce_bytes),
            ciphertext: base64_encode(&ciphertext),
        };

        let json = serde_json::to_vec_pretty(&blob)?;
        atomic_write(path, &json)?;
        Ok(())
    }

    /// Decrypt the file at `path` with `pin` and reconstruct the wallet.
    /// A wrong PIN surfaces as [`Error::WrongPin`].
    pub fn load_encrypted(pin: &str, path: &Path) -> Result<Self, Error> {
        let raw = std::fs::read(path)?;
        let blob: EncryptedBlob = serde_json::from_slice(&raw)?;
        if blob.version != STORAGE_VERSION {
            return Err(Error::UnsupportedVersion(blob.version));
        }
        let salt = base64_decode(&blob.salt)?;
        let nonce = base64_decode(&blob.nonce)?;
        let ciphertext = base64_decode(&blob.ciphertext)?;
        let kek = derive_kek(pin.as_bytes(), &salt)?;
        let cipher = XChaCha20Poly1305::new_from_slice(&kek)
            .map_err(|_| Error::Crypto("cipher init".into()))?;
        let plain = cipher
            .decrypt(XNonce::from_slice(&nonce), AeadPayload {
                msg: &ciphertext,
                aad: &[STORAGE_VERSION],
            })
            .map_err(|_| Error::WrongPin)?;
        if plain.len() != 65 {
            return Err(Error::Corrupt("plaintext length".into()));
        }
        let mut bytes = [0u8; 64];
        bytes.copy_from_slice(&plain[..64]);
        let network = match plain[64] {
            0 => Network::MainnetBeta,
            1 => Network::Devnet,
            other => return Err(Error::Corrupt(format!("unknown network tag {other}"))),
        };
        let kp = Keypair::try_from(&bytes[..]).map_err(|e| Error::KeyDerivation(e.to_string()))?;
        let pubkey = kp.pubkey();
        Ok(Self {
            keypair_bytes: Zeroizing::new(bytes),
            pubkey,
            network,
        })
    }
}

// ── KDF / serialization helpers ──────────────────────────────────────────

fn derive_kek(pin: &[u8], salt: &[u8]) -> Result<[u8; KEK_LEN], Error> {
    let params = Params::new(ARGON2_M_COST, ARGON2_T_COST, ARGON2_P_COST, Some(KEK_LEN))
        .map_err(|e| Error::Crypto(format!("argon2 params: {e}")))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut kek = [0u8; KEK_LEN];
    argon2
        .hash_password_into(pin, salt, &mut kek)
        .map_err(|e| Error::Crypto(format!("argon2 hash: {e}")))?;
    Ok(kek)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

fn base64_encode(b: &[u8]) -> String {
    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
    B64.encode(b)
}

fn base64_decode(s: &str) -> Result<Vec<u8>, Error> {
    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
    B64.decode(s).map_err(|e| Error::Corrupt(format!("base64: {e}")))
}

#[derive(Serialize, Deserialize)]
struct EncryptedBlob {
    version: u8,
    kdf: String,
    kdf_params: KdfParams,
    salt: String,
    nonce: String,
    ciphertext: String,
}

#[derive(Serialize, Deserialize)]
struct KdfParams {
    m_cost: u32,
    t_cost: u32,
    p_cost: u32,
}

/// Errors surfaced by the wallet API. Designed for UI consumption — the
/// `WrongPin` arm is explicit so the lock screen can show a clear message.
#[derive(Debug, Error)]
pub enum Error {
    /// Wrong PIN supplied to [`ArgosWallet::load_encrypted`].
    #[error("wrong PIN")]
    WrongPin,
    /// On-disk blob version we don't know how to parse.
    #[error("unsupported wallet storage version: {0}")]
    UnsupportedVersion(u8),
    /// Stored blob failed structural validation after AEAD decrypt.
    #[error("wallet corrupt: {0}")]
    Corrupt(String),
    /// BIP39 mnemonic invalid.
    #[error("bip39: {0}")]
    Bip39(String),
    /// Solana SDK error while deriving / loading the keypair.
    #[error("key derivation: {0}")]
    KeyDerivation(String),
    /// Argon2id / AEAD failure.
    #[error("crypto: {0}")]
    Crypto(String),
    /// Solana RPC error (network failure, insufficient funds, ...).
    /// Surfaces the upstream string so the UI can display it as-is.
    #[error("rpc: {0}")]
    Rpc(String),
    /// User-pasted Solana address could not be parsed.
    #[error("invalid address: {0}")]
    InvalidAddress(String),
    /// JSON serde error on the persisted blob.
    #[error("serde: {0}")]
    Serde(#[from] serde_json::Error),
    /// Filesystem I/O.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(name: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        p.push(format!("argos_wallet_test_{name}_{nanos}.json"));
        p
    }

    #[test]
    fn generate_then_restore_via_mnemonic() {
        let (w1, m) = ArgosWallet::generate(Network::Devnet).unwrap();
        let w2 = ArgosWallet::from_mnemonic(&m.to_string(), "", Network::Devnet).unwrap();
        assert_eq!(w1.pubkey(), w2.pubkey());
    }

    #[test]
    fn persist_and_load_roundtrip() {
        let (w1, _m) = ArgosWallet::generate(Network::Devnet).unwrap();
        let path = tmp_path("persist");
        w1.persist_encrypted("4711", &path).unwrap();
        let w2 = ArgosWallet::load_encrypted("4711", &path).unwrap();
        assert_eq!(w1.pubkey(), w2.pubkey());
        assert_eq!(w1.network(), w2.network());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn wrong_pin_returns_wrong_pin_error() {
        let (w1, _m) = ArgosWallet::generate(Network::Devnet).unwrap();
        let path = tmp_path("wrongpin");
        w1.persist_encrypted("correct", &path).unwrap();
        match ArgosWallet::load_encrypted("wrong", &path) {
            Err(Error::WrongPin) => {}
            other => panic!("expected WrongPin, got {other:?}"),
        }
        let _ = std::fs::remove_file(&path);
    }
}
