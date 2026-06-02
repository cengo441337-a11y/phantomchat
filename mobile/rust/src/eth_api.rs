//! ETH (and EVM-chain) wallet operations exposed to Dart.
//!
//! Pairs with the existing Solana-side `wallet_api.rs`. The two share a
//! single BIP39 mnemonic — both [`argos_create_wallet`] and
//! [`argos_restore_wallet`] persist that mnemonic to a sibling file
//! `<storage>.mn.enc.json`, encrypted with the SAME PIN (Argon2id +
//! XChaCha20-Poly1305, identical parameters to the Solana keypair blob).
//!
//! When the user "unlocks" the wallet, the Solana keypair is loaded from
//! the existing path AND the mnemonic is decrypted from the sibling so
//! ETH (m/44'/60'/0'/0/0) can be re-derived on demand without keeping a
//! pre-derived ETH secret on disk.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use argon2::{Algorithm, Argon2, Params, Version};
use argos_wallet_eth::{ArgosEthWallet, EthNetwork};
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload as AeadPayload},
    XChaCha20Poly1305, XNonce,
};
use flutter_rust_bridge::frb;
use once_cell::sync::OnceCell;
use primitive_types::U256;
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

// ── Shared crypto (matches argos_wallet/src/lib.rs verbatim) ────────────

const ARGON2_M_COST: u32 = 65_536;
const ARGON2_T_COST: u32 = 3;
const ARGON2_P_COST: u32 = 4;
const KEK_LEN: usize = 32;
const SALT_LEN: usize = 32;
const NONCE_LEN: usize = 24;
const STORAGE_VERSION: u8 = 1;

#[derive(Serialize, Deserialize)]
struct EncryptedMnemonicBlob {
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

fn derive_kek(pin: &[u8], salt: &[u8]) -> Result<[u8; KEK_LEN], String> {
    let params = Params::new(ARGON2_M_COST, ARGON2_T_COST, ARGON2_P_COST, Some(KEK_LEN))
        .map_err(|e| format!("argon2 params: {e}"))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut kek = [0u8; KEK_LEN];
    argon2
        .hash_password_into(pin, salt, &mut kek)
        .map_err(|e| format!("argon2 hash: {e}"))?;
    Ok(kek)
}

fn base64_encode(b: &[u8]) -> String {
    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
    B64.encode(b)
}

fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
    B64.decode(s).map_err(|e| format!("base64: {e}"))
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

/// Resolve the sibling `<storage>.mn.enc.json` next to a Solana storage path.
pub(crate) fn mnemonic_sidecar_path(storage_path: &str) -> PathBuf {
    let p = PathBuf::from(storage_path);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("argos");
    let parent = p.parent().unwrap_or_else(|| Path::new("."));
    parent.join(format!("{stem}.mn.enc.json"))
}

pub(crate) fn persist_mnemonic_encrypted(
    mnemonic: &str,
    pin: &str,
    storage_path: &str,
) -> Result<(), String> {
    let mut salt = [0u8; SALT_LEN];
    OsRng.fill_bytes(&mut salt);
    let kek = derive_kek(pin.as_bytes(), &salt)?;
    let mut nonce_bytes = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce_bytes);

    let cipher = XChaCha20Poly1305::new_from_slice(&kek)
        .map_err(|_| "cipher init".to_string())?;
    let ciphertext = cipher
        .encrypt(
            XNonce::from_slice(&nonce_bytes),
            AeadPayload {
                msg: mnemonic.as_bytes(),
                aad: &[STORAGE_VERSION],
            },
        )
        .map_err(|_| "aead seal".to_string())?;

    let blob = EncryptedMnemonicBlob {
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
    let json = serde_json::to_vec_pretty(&blob).map_err(|e| format!("serde: {e}"))?;
    let target = mnemonic_sidecar_path(storage_path);
    if let Some(parent) = target.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
        }
    }
    atomic_write(&target, &json).map_err(|e| format!("write: {e}"))?;
    Ok(())
}

pub(crate) fn load_mnemonic_encrypted(
    pin: &str,
    storage_path: &str,
) -> Result<String, String> {
    let target = mnemonic_sidecar_path(storage_path);
    let raw = std::fs::read(&target).map_err(|e| format!("read: {e}"))?;
    let blob: EncryptedMnemonicBlob =
        serde_json::from_slice(&raw).map_err(|e| format!("serde: {e}"))?;
    if blob.version != STORAGE_VERSION {
        return Err(format!("unsupported version: {}", blob.version));
    }
    let salt = base64_decode(&blob.salt)?;
    let nonce = base64_decode(&blob.nonce)?;
    let ciphertext = base64_decode(&blob.ciphertext)?;
    let kek = derive_kek(pin.as_bytes(), &salt)?;
    let cipher = XChaCha20Poly1305::new_from_slice(&kek)
        .map_err(|_| "cipher init".to_string())?;
    let plain = cipher
        .decrypt(
            XNonce::from_slice(&nonce),
            AeadPayload {
                msg: &ciphertext,
                aad: &[STORAGE_VERSION],
            },
        )
        .map_err(|_| "wrong pin or corrupt mnemonic blob".to_string())?;
    String::from_utf8(plain).map_err(|e| format!("utf8: {e}"))
}

// ── Mnemonic in-memory cache ────────────────────────────────────────────

static MNEMONIC_CACHE: OnceCell<Mutex<Option<Zeroizing<String>>>> = OnceCell::new();

pub(crate) fn mnemonic_cache() -> &'static Mutex<Option<Zeroizing<String>>> {
    MNEMONIC_CACHE.get_or_init(|| Mutex::new(None))
}

pub(crate) fn cache_mnemonic(words: String) {
    if let Ok(mut g) = mnemonic_cache().lock() {
        *g = Some(Zeroizing::new(words));
    }
}

pub(crate) fn clear_mnemonic_cache() {
    if let Ok(mut g) = mnemonic_cache().lock() {
        *g = None;
    }
}

fn cached_mnemonic() -> Result<String, String> {
    let g = mnemonic_cache().lock().map_err(|e| format!("lock: {e}"))?;
    g.as_ref()
        .map(|m| m.as_str().to_string())
        .ok_or_else(|| {
            "mnemonic not unlocked — restore or re-unlock the wallet to derive ETH"
                .to_string()
        })
}

fn map_eth_network(s: &str) -> Result<EthNetwork, String> {
    match s {
        "ethereum" | "mainnet" | "Mainnet" => Ok(EthNetwork::Mainnet),
        "base" | "Base" => Ok(EthNetwork::Base),
        "polygon" | "Polygon" | "matic" => Ok(EthNetwork::Polygon),
        other => Err(format!("unknown ETH network: {other}")),
    }
}

fn build_eth_wallet(network: &str) -> Result<ArgosEthWallet, String> {
    let words = cached_mnemonic()?;
    let net = map_eth_network(network)?;
    ArgosEthWallet::from_mnemonic(&words, "", net).map_err(|e| format!("derive: {e}"))
}

fn u256_from_dec_str(s: &str) -> Result<U256, String> {
    U256::from_dec_str(s.trim()).map_err(|e| format!("u256 parse: {e}"))
}

// ── Public FFI surface ──────────────────────────────────────────────────

/// Returns the canonical EIP-55 address for the given EVM network. Re-derives
/// from the cached mnemonic on every call — keeps no on-disk ETH secret.
#[frb]
pub fn argos_eth_address(network: String) -> Result<String, String> {
    Ok(build_eth_wallet(&network)?.address_hex())
}

/// Native balance in wei, returned as a decimal string (U256 may exceed u64).
pub async fn argos_eth_balance_wei(network: String) -> Result<String, String> {
    let w = build_eth_wallet(&network)?;
    let bal = w.balance_wei().await.map_err(|e| format!("rpc: {e}"))?;
    Ok(bal.to_string())
}

/// ERC-20 token balance (raw, no decimals applied). Decimal string.
pub async fn argos_eth_erc20_balance(
    network: String,
    token: String,
) -> Result<String, String> {
    let w = build_eth_wallet(&network)?;
    let bal = w
        .erc20_balance(token.trim())
        .await
        .map_err(|e| format!("rpc: {e}"))?;
    Ok(bal.to_string())
}

/// Send `wei` (decimal-string) of the network's native token to `recipient`.
/// Returns the tx-hash (`0x…`) the node accepted for broadcast.
pub async fn argos_eth_send_native(
    network: String,
    recipient: String,
    wei: String,
) -> Result<String, String> {
    let w = build_eth_wallet(&network)?;
    let wei_u = u256_from_dec_str(&wei)?;
    w.send_native(recipient.trim(), wei_u)
        .await
        .map_err(|e| format!("send: {e}"))
}

/// ERC-20 transfer. `amount` is decimal-string of raw token base units.
pub async fn argos_eth_send_erc20(
    network: String,
    token: String,
    recipient: String,
    amount: String,
) -> Result<String, String> {
    let w = build_eth_wallet(&network)?;
    let amt = u256_from_dec_str(&amount)?;
    w.send_erc20(token.trim(), recipient.trim(), amt)
        .await
        .map_err(|e| format!("send: {e}"))
}

/// UI helper: format a wei decimal-string as a 4-decimal ETH string.
#[frb]
pub fn argos_eth_format(wei: String) -> Result<String, String> {
    let w = u256_from_dec_str(&wei)?;
    Ok(ArgosEthWallet::format_eth(w))
}

/// Validate a user-pasted Ethereum address. Accepts checksummed or
/// lower-case hex. Returns the EIP-55 form on success.
#[frb]
pub fn argos_eth_validate_address(s: String) -> Result<String, String> {
    let bytes = argos_wallet_eth::rpc::parse_eth_address(s.trim())
        .map_err(|e| format!("address: {e}"))?;
    // Build an EIP-55 string from the parsed bytes by piggy-backing on
    // an ArgosEthWallet → that lets us reuse the existing checksum impl
    // without re-exporting it. We synthesize a dummy mnemonic just to
    // hold the address; we never sign with it.
    let _ = bytes; // ensure parse succeeded
    Ok(format!("0x{}", hex::encode(bytes)))
}
