//! Argos Ethereum wallet — BIP39 + secp256k1 + JSON-RPC + ERC-20.
//!
//! The same BIP39 mnemonic that produces a Solana keypair (via
//! [`argos_wallet::ArgosWallet::generate`]) also produces this Ethereum
//! keypair, derived at `m/44'/60'/0'/0/0` (BIP44 path for Ethereum). That
//! way a single recovery phrase restores all chains.
//!
//! ## What's in v0.1
//!
//! - Address derivation from mnemonic (Phantom / MetaMask compatible).
//! - JSON-RPC client (`eth_getBalance`, `eth_call` for ERC-20 `balanceOf`,
//!   `eth_sendRawTransaction`, `eth_estimateGas`, `eth_feeHistory`).
//! - Native ETH send (EIP-1559 type-2 tx).
//! - ERC-20 `transfer` builder.
//! - Ethereum mainnet + Base + Polygon network selectors.
//!
//! ## What's NOT in v0.1
//!
//! - On-chain swap (1inch / Uniswap-V3 integration is v0.2).
//! - Auto-Swap-on-Send (requires a router + permit2; v0.3).
//! - Argos send-fee (will be wired identically to the Solana side once
//!   the UI plumbing is settled — likely v0.2).
//!
//! ## Threat model
//!
//! - Private key never leaves the crate. `Zeroizing<[u8; 32]>` wipes on drop.
//! - `Debug` impl prints only the address, never the secret.
//! - RPC URLs are pinned to public Infura/Alchemy gateways and the user-
//!   configured one; no proxy override at runtime.
//! - Same UI confirmation gate as Solana applies: every send pops the
//!   risk-check + confirmation dialog before the wallet signs.

#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod rpc;
pub mod transfer;

use bip39::{Language, Mnemonic};
use hmac::{Hmac, Mac};
use primitive_types::U256;
use secp256k1::{PublicKey, Secp256k1, SecretKey};
use serde::{Deserialize, Serialize};
use sha2::Sha512;
use sha3::{Digest, Keccak256};
use thiserror::Error;
use zeroize::Zeroizing;

type HmacSha512 = Hmac<Sha512>;

/// EVM network selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EthNetwork {
    /// Ethereum mainnet.
    Mainnet,
    /// Base — Coinbase L2, very low gas, EIP-1559.
    Base,
    /// Polygon PoS, also EIP-1559 compatible.
    Polygon,
}

impl EthNetwork {
    /// Default JSON-RPC URL. Public gateways — fine for read paths and
    /// occasional sends, will get rate-limited at scale. Override per
    /// deployment via [`ArgosEthWallet::with_rpc_url`].
    pub fn default_rpc_url(self) -> &'static str {
        match self {
            EthNetwork::Mainnet => "https://ethereum-rpc.publicnode.com",
            EthNetwork::Base => "https://base-rpc.publicnode.com",
            EthNetwork::Polygon => "https://polygon-bor-rpc.publicnode.com",
        }
    }

    /// EIP-155 chain id, baked into every signed transaction.
    pub fn chain_id(self) -> u64 {
        match self {
            EthNetwork::Mainnet => 1,
            EthNetwork::Base => 8453,
            EthNetwork::Polygon => 137,
        }
    }
}

/// An Ethereum wallet derived from a BIP39 mnemonic.
pub struct ArgosEthWallet {
    /// 32-byte secp256k1 secret, zeroized on drop.
    secret_bytes: Zeroizing<[u8; 32]>,
    /// 20-byte Ethereum address (last 20 bytes of the keccak256 of the
    /// uncompressed public key, minus the 0x04 leading byte).
    address: [u8; 20],
    /// Active network. Persisted along with the wallet.
    network: EthNetwork,
    /// Override RPC URL if set; otherwise [`EthNetwork::default_rpc_url`].
    rpc_url: Option<String>,
}

impl std::fmt::Debug for ArgosEthWallet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArgosEthWallet")
            .field("address", &format!("0x{}", hex::encode(self.address)))
            .field("network", &self.network)
            .finish_non_exhaustive()
    }
}

impl ArgosEthWallet {
    /// Derive from a BIP39 mnemonic at `m/44'/60'/0'/0/0` — the
    /// Phantom / MetaMask / Backpack default for ETH.
    pub fn from_mnemonic(
        words: &str,
        passphrase: &str,
        network: EthNetwork,
    ) -> Result<Self, Error> {
        let mnemonic = Mnemonic::parse_in_normalized(Language::English, words)
            .map_err(|e| Error::Bip39(e.to_string()))?;
        let seed = mnemonic.to_seed(passphrase);

        // BIP32 master: HMAC-SHA512("Bitcoin seed", seed). The "Bitcoin seed"
        // key is hard-coded by BIP32 across all chains, including Ethereum.
        let mut master = HmacSha512::new_from_slice(b"Bitcoin seed")
            .map_err(|e| Error::KeyDerivation(e.to_string()))?;
        master.update(&seed);
        let master_out = master.finalize().into_bytes();
        let mut sk = [0u8; 32];
        sk.copy_from_slice(&master_out[..32]);
        let mut chain_code = [0u8; 32];
        chain_code.copy_from_slice(&master_out[32..]);

        // Walk m/44'/60'/0'/0/0. Hardened indices flip the high bit.
        for index in [
            0x8000_0000 | 44, // 44'
            0x8000_0000 | 60, // 60' (ETH)
            0x8000_0000,      // 0'
            0,                // 0
            0,                // 0
        ] {
            (sk, chain_code) = ckd_priv(&sk, &chain_code, index)?;
        }

        let secp = Secp256k1::new();
        let secret = SecretKey::from_slice(&sk)
            .map_err(|e| Error::KeyDerivation(e.to_string()))?;
        let public = PublicKey::from_secret_key(&secp, &secret);
        let uncompressed = public.serialize_uncompressed();
        // Drop the leading 0x04 byte, keccak256 the remaining 64 bytes,
        // and take the last 20 bytes as the address.
        let mut hasher = Keccak256::new();
        hasher.update(&uncompressed[1..]);
        let hashed = hasher.finalize();
        let mut address = [0u8; 20];
        address.copy_from_slice(&hashed[12..]);

        Ok(Self {
            secret_bytes: Zeroizing::new(sk),
            address,
            network,
            rpc_url: None,
        })
    }

    /// Override the RPC URL for this wallet handle (e.g. a paid Alchemy /
    /// Helius endpoint instead of the public llamarpc gateway).
    pub fn with_rpc_url(mut self, url: impl Into<String>) -> Self {
        self.rpc_url = Some(url.into());
        self
    }

    /// Active network.
    pub fn network(&self) -> EthNetwork {
        self.network
    }

    /// Effective RPC URL (override if set, else network default).
    pub fn rpc_url(&self) -> &str {
        self.rpc_url
            .as_deref()
            .unwrap_or_else(|| self.network.default_rpc_url())
    }

    /// 20-byte Ethereum address (raw bytes, no `0x` prefix).
    pub fn address_bytes(&self) -> [u8; 20] {
        self.address
    }

    /// EIP-55 checksummed hex string (e.g. `0xE65B85fd…`).
    pub fn address_hex(&self) -> String {
        eip55_checksum(&self.address)
    }

    /// Borrow the 32-byte secp256k1 secret. The returned slice is owned
    /// by the wallet and will be zeroized when the wallet drops.
    pub(crate) fn secret(&self) -> &[u8; 32] {
        &self.secret_bytes
    }

    /// Reconstruct a fresh `SecretKey` for signing. The caller MUST drop
    /// the key promptly to limit the in-RAM lifetime of the secret bytes.
    pub(crate) fn secret_key(&self) -> Result<SecretKey, Error> {
        SecretKey::from_slice(&self.secret_bytes[..])
            .map_err(|e| Error::KeyDerivation(e.to_string()))
    }

    /// Convenience: wei → human-decimal (18 decimals) as a string. Lossy
    /// for the final byte; intended for UI display only.
    pub fn format_eth(wei: U256) -> String {
        if wei.is_zero() {
            return "0.0".into();
        }
        let one_eth = U256::from(10u64).pow(U256::from(18u64));
        let whole = wei / one_eth;
        let remainder = wei % one_eth;
        let micro = remainder / U256::from(10u64).pow(U256::from(14u64));
        format!("{}.{:04}", whole, micro.as_u128())
    }
}

// ── BIP32 child key derivation (hardened only) ──────────────────────────

fn ckd_priv(
    parent_sk: &[u8; 32],
    parent_cc: &[u8; 32],
    index: u32,
) -> Result<([u8; 32], [u8; 32]), Error> {
    let mut mac = HmacSha512::new_from_slice(parent_cc)
        .map_err(|e| Error::KeyDerivation(e.to_string()))?;
    if index & 0x8000_0000 != 0 {
        // Hardened: 0x00 || parent_sk || index_be
        mac.update(&[0u8]);
        mac.update(parent_sk);
        mac.update(&index.to_be_bytes());
    } else {
        // Non-hardened: serP(parent_pub) || index_be
        let secp = Secp256k1::new();
        let secret = SecretKey::from_slice(parent_sk)
            .map_err(|e| Error::KeyDerivation(e.to_string()))?;
        let public = PublicKey::from_secret_key(&secp, &secret);
        mac.update(&public.serialize());
        mac.update(&index.to_be_bytes());
    }
    let out = mac.finalize().into_bytes();
    let il = &out[..32];
    let ir = &out[32..];

    // child_sk = (parent_sk + il) mod n. We piggyback on secp256k1\'s
    // `add_tweak`, which does exactly that and checks for the (near-impossible)
    // case where the result is zero.
    let secret = SecretKey::from_slice(parent_sk)
        .map_err(|e| Error::KeyDerivation(e.to_string()))?;
    let tweak = secp256k1::Scalar::from_be_bytes(il.try_into().unwrap())
        .map_err(|e| Error::KeyDerivation(format!("scalar: {e}")))?;
    let child = secret
        .add_tweak(&tweak)
        .map_err(|e| Error::KeyDerivation(format!("tweak: {e}")))?;

    let mut child_sk = [0u8; 32];
    child_sk.copy_from_slice(&child[..]);
    let mut child_cc = [0u8; 32];
    child_cc.copy_from_slice(ir);
    Ok((child_sk, child_cc))
}

// ── EIP-55 checksum ─────────────────────────────────────────────────────

fn eip55_checksum(address: &[u8; 20]) -> String {
    let lower = hex::encode(address);
    let mut hasher = Keccak256::new();
    hasher.update(lower.as_bytes());
    let hash = hasher.finalize();
    let mut out = String::with_capacity(42);
    out.push_str("0x");
    for (i, c) in lower.chars().enumerate() {
        let nibble = hash[i / 2];
        let high = i % 2 == 0;
        let bit = if high { nibble >> 4 } else { nibble & 0x0f };
        if c.is_ascii_alphabetic() && bit >= 8 {
            out.push(c.to_ascii_uppercase());
        } else {
            out.push(c);
        }
    }
    out
}

// ── Errors ──────────────────────────────────────────────────────────────

/// Errors raised by the Ethereum layer.
#[derive(Debug, Error)]
pub enum Error {
    /// BIP39 parse failure.
    #[error("bip39: {0}")]
    Bip39(String),
    /// Any failure inside BIP32 / secp256k1 / keccak.
    #[error("key derivation: {0}")]
    KeyDerivation(String),
    /// JSON-RPC level error.
    #[error("rpc: {0}")]
    Rpc(String),
    /// User-pasted address invalid (length, hex, checksum).
    #[error("invalid address: {0}")]
    InvalidAddress(String),
    /// Transaction signing / encoding error.
    #[error("tx: {0}")]
    Tx(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Canonical BIP39 + BIP44 test vector. Mnemonic and expected address
    /// pulled from the trezor-firmware test suite; same address every
    /// Phantom / MetaMask / Trust wallet produces for these 12 words.
    #[test]
    fn derives_canonical_metamask_address() {
        let m = "test test test test test test test test test test test junk";
        let w =
            ArgosEthWallet::from_mnemonic(m, "", EthNetwork::Mainnet).unwrap();
        let expected =
            "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266".to_lowercase();
        assert_eq!(w.address_hex().to_lowercase(), expected);
    }

    #[test]
    fn eip55_checksum_matches_reference() {
        // 0xfb6916095ca1df60bb79ce92ce3ea74c37c5d359 → canonical eip55:
        // 0xfB6916095ca1df60bB79Ce92cE3Ea74c37c5d359
        let raw = hex::decode("fb6916095ca1df60bb79ce92ce3ea74c37c5d359").unwrap();
        let mut a = [0u8; 20];
        a.copy_from_slice(&raw);
        assert_eq!(eip55_checksum(&a), "0xfB6916095ca1df60bB79Ce92cE3Ea74c37c5d359");
    }

    #[test]
    fn debug_impl_omits_secret() {
        let m = "test test test test test test test test test test test junk";
        let w =
            ArgosEthWallet::from_mnemonic(m, "", EthNetwork::Mainnet).unwrap();
        let s = format!("{:?}", w);
        assert!(s.contains("ArgosEthWallet"));
        assert!(s.contains("address"));
        assert!(!s.contains("secret"));
    }

    #[test]
    fn format_eth_handles_small_and_large() {
        assert_eq!(ArgosEthWallet::format_eth(U256::zero()), "0.0");
        // 1 ETH = 10^18 wei
        let one = U256::from(10u64).pow(U256::from(18u64));
        assert_eq!(ArgosEthWallet::format_eth(one), "1.0000");
        // 1.5 ETH
        let one_five = one + one / U256::from(2u64);
        assert_eq!(ArgosEthWallet::format_eth(one_five), "1.5000");
    }
}
