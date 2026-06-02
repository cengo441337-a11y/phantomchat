//! Argos Wallet — flutter_rust_bridge API surface.
//!
//! Slim wrapper around `argos_wallet`. The Dart layer calls `unlock` once
//! with PIN + storage path; subsequent calls reference the cached
//! `Arc<ArgosWallet>` without round-tripping secrets across the FFI border.
//!
//! ## Threat model recap
//!
//! - Secrets cross the FFI border only at `unlock` / `create` / `restore`.
//! - Cache is wiped by `lock_wallet`; idle screens MUST call it on background.
//! - `pubkey()` is the only stateful read that returns publicly-leak-safe data.

use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use flutter_rust_bridge::frb;
use once_cell::sync::OnceCell;

use argos_wallet::swap::{SwapAndSendOutcome, SwapPreview};

use crate::api::wallet::eth_api::{cache_mnemonic, clear_mnemonic_cache, load_mnemonic_encrypted, persist_mnemonic_encrypted};
use argos_wallet::{ArgosWallet, Network};
use solana_sdk::pubkey::Pubkey;

static WALLET_CACHE: OnceCell<Mutex<Option<Arc<ArgosWallet>>>> = OnceCell::new();
static PREVIEW_CACHE: OnceCell<Mutex<Option<SwapPreview>>> = OnceCell::new();

fn cache() -> &'static Mutex<Option<Arc<ArgosWallet>>> {
    WALLET_CACHE.get_or_init(|| Mutex::new(None))
}

fn preview_cache() -> &'static Mutex<Option<SwapPreview>> {
    PREVIEW_CACHE.get_or_init(|| Mutex::new(None))
}

fn map_network(s: &str) -> Result<Network, String> {
    match s {
        "mainnet" | "mainnet-beta" | "MainnetBeta" => Ok(Network::MainnetBeta),
        "devnet" | "Devnet" => Ok(Network::Devnet),
        other => Err(format!("unknown network: {other}")),
    }
}

fn unlocked() -> Result<Arc<ArgosWallet>, String> {
    let guard = cache().lock().map_err(|e| format!("lock: {e}"))?;
    guard
        .as_ref()
        .cloned()
        .ok_or_else(|| "wallet locked — call argos_unlock_wallet first".into())
}

// ── Data structs the Dart side sees ──────────────────────────────────────

/// What the UI shows on wallet creation / restore. Mnemonic is ONLY
/// populated on `create` and `restore` — never on `unlock`, so an unlocked
/// session can't re-leak the recovery phrase.
#[frb]
#[derive(Debug, Clone)]
pub struct ArgosWalletInfo {
    /// Solana public key (base58, the Solscan-friendly form).
    pub pubkey_b58: String,
    /// BIP39 mnemonic — 24 English words separated by single spaces.
    /// Empty on `unlock_wallet`; non-empty on `create_wallet` and `restore_wallet`.
    pub mnemonic: String,
    /// Network the wallet was created against.
    pub network: String,
}

/// UI-side projection of [`argos_wallet::swap::SwapPreview`]. The full
/// preview is stored Rust-side via `PREVIEW_CACHE`; the Dart layer just
/// keeps the human-readable fields.
#[frb]
#[derive(Debug, Clone)]
pub struct ArgosSwapPreview {
    /// Raw amount the user pays, in input-mint smallest units.
    pub amount_in: u64,
    /// Min output amount, in output-mint smallest units (post-slippage).
    pub amount_out_min: u64,
    /// Expected (mid-point) output, in output-mint smallest units.
    pub amount_out_expected: u64,
    /// Platform fee, in output-mint smallest units (0.5 % of output).
    pub platform_fee_out: u64,
    /// Best-effort human route summary, e.g. "Raydium → Orca".
    pub route_label: String,
    /// Slippage tolerance in basis points (50 = 0.5 %).
    pub slippage_bps: u16,
    /// Output mint (for Dart-side display of "you receive X of mint Y").
    pub output_mint_b58: String,
}

/// Result of `argos_swap_and_send` — the killer Auto-Swap-on-Send feature.
#[frb]
#[derive(Debug, Clone)]
pub struct ArgosSwapAndSendOutcome {
    /// Confirmed transaction signature (base58, links to Solscan).
    pub signature_b58: String,
    /// Recipient's associated token account that received the swap output.
    pub recipient_ata_b58: String,
    /// Mint the recipient received.
    pub output_mint_b58: String,
    /// Expected output amount delivered (pre-slippage).
    pub amount_out_expected: u64,
}

// ── Public API exposed to Dart ───────────────────────────────────────────

/// Generate a fresh wallet, persist it encrypted with `pin` to `storage_path`,
/// and return the recovery mnemonic. The mnemonic MUST be displayed to the
/// user exactly once for backup; the app should treat it as write-only.
pub fn argos_create_wallet(
    network: String,
    pin: String,
    storage_path: String,
) -> Result<ArgosWalletInfo, String> {
    let net = map_network(&network)?;
    let (wallet, mnemonic) =
        ArgosWallet::generate(net).map_err(|e| format!("generate: {e}"))?;
    let path = PathBuf::from(&storage_path);
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
        }
    }
    wallet
        .persist_encrypted(&pin, &path)
        .map_err(|e| format!("persist: {e}"))?;
    let pk = wallet.pubkey().to_string();
    *cache().lock().map_err(|e| format!("lock: {e}"))? = Some(Arc::new(wallet));
    let words = mnemonic.to_string();
    persist_mnemonic_encrypted(&words, &pin, &storage_path)?;
    cache_mnemonic(words.clone());
    Ok(ArgosWalletInfo {
        pubkey_b58: pk,
        mnemonic: words,
        network,
    })
}

/// Restore from a user-typed 12/24-word BIP39 mnemonic, persist encrypted,
/// and unlock in one step. Use during onboarding after a wipe.
pub fn argos_restore_wallet(
    mnemonic: String,
    network: String,
    pin: String,
    storage_path: String,
) -> Result<ArgosWalletInfo, String> {
    let net = map_network(&network)?;
    let wallet = ArgosWallet::from_mnemonic(mnemonic.trim(), "", net)
        .map_err(|e| format!("restore: {e}"))?;
    let path = PathBuf::from(&storage_path);
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
        }
    }
    wallet
        .persist_encrypted(&pin, &path)
        .map_err(|e| format!("persist: {e}"))?;
    let pk = wallet.pubkey().to_string();
    *cache().lock().map_err(|e| format!("lock: {e}"))? = Some(Arc::new(wallet));
    let words = mnemonic.trim().to_string();
    persist_mnemonic_encrypted(&words, &pin, &storage_path)?;
    cache_mnemonic(words.clone());
    Ok(ArgosWalletInfo {
        pubkey_b58: pk,
        mnemonic: words,
        network,
    })
}

/// Unlock an existing on-disk wallet with `pin`. Returns the pubkey on
/// success. Wrong PIN surfaces as `Err("wrong PIN")`.
pub fn argos_unlock_wallet(pin: String, storage_path: String) -> Result<String, String> {
    let path = PathBuf::from(&storage_path);
    let wallet =
        ArgosWallet::load_encrypted(&pin, &path).map_err(|e| format!("unlock: {e}"))?;
    let pk = wallet.pubkey().to_string();
    *cache().lock().map_err(|e| format!("lock: {e}"))? = Some(Arc::new(wallet));
    // Best-effort: load the mnemonic sidecar so EVM chains can derive on
    // demand. Pre-1.4 wallets won't have it — that's OK; ETH commands
    // will then return a clear "mnemonic not unlocked" error and the UI
    // can prompt the user to restore from phrase to regenerate it.
    if let Ok(words) = load_mnemonic_encrypted(&pin, &storage_path) {
        cache_mnemonic(words);
    }
    Ok(pk)
}

/// Wipe the cached wallet from RAM. Idempotent — safe to call when already
/// locked.
pub fn argos_lock_wallet() -> Result<(), String> {
    *cache().lock().map_err(|e| format!("lock: {e}"))? = None;
    *preview_cache().lock().map_err(|e| format!("lock: {e}"))? = None;
    clear_mnemonic_cache();
    Ok(())
}

/// Returns Some(pubkey_b58) when a wallet is currently unlocked.
#[frb(sync)]
pub fn argos_wallet_pubkey() -> Option<String> {
    cache()
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|w| w.pubkey().to_string()))
}

/// Active network ("mainnet-beta" or "devnet") of the unlocked wallet.
#[frb(sync)]
pub fn argos_wallet_network() -> Option<String> {
    cache().lock().ok().and_then(|g| {
        g.as_ref().map(|w| match w.network() {
            Network::MainnetBeta => "mainnet-beta".to_string(),
            Network::Devnet => "devnet".to_string(),
        })
    })
}

/// Native SOL balance in lamports.
pub async fn argos_balance_sol() -> Result<u64, String> {
    let w = unlocked()?;
    w.balance_lamports()
        .await
        .map_err(|e| format!("rpc: {e}"))
}

/// SPL token balance for `mint_b58`, in raw token units (no decimal conversion).
pub async fn argos_balance_token(mint_b58: String) -> Result<u64, String> {
    let mint = Pubkey::from_str(mint_b58.trim()).map_err(|e| format!("mint: {e}"))?;
    let w = unlocked()?;
    w.balance_spl(&mint).await.map_err(|e| format!("rpc: {e}"))
}

/// Send `lamports` of native SOL to `recipient_b58`. Returns the confirmed
/// signature (base58, links to Solscan).
pub async fn argos_send_sol(
    recipient_b58: String,
    lamports: u64,
) -> Result<String, String> {
    let recipient =
        Pubkey::from_str(recipient_b58.trim()).map_err(|e| format!("recipient: {e}"))?;
    let w = unlocked()?;
    let sig = w
        .send_sol(&recipient, lamports)
        .await
        .map_err(|e| format!("send: {e}"))?;
    Ok(sig.to_string())
}

/// Send `amount` raw units of `mint_b58` SPL token to `recipient_b58`.
/// Auto-creates the recipient's ATA if missing (sender pays ~0.002 SOL rent).
pub async fn argos_send_token(
    mint_b58: String,
    recipient_b58: String,
    amount: u64,
) -> Result<String, String> {
    let mint = Pubkey::from_str(mint_b58.trim()).map_err(|e| format!("mint: {e}"))?;
    let recipient =
        Pubkey::from_str(recipient_b58.trim()).map_err(|e| format!("recipient: {e}"))?;
    let w = unlocked()?;
    let sig = w
        .send_spl(&mint, &recipient, amount)
        .await
        .map_err(|e| format!("send: {e}"))?;
    Ok(sig.to_string())
}

/// Ask Jupiter for the best swap route. Caches the full preview Rust-side
/// so `argos_execute_swap` / `argos_swap_and_send` don't need to round-trip
/// the raw quote JSON across the FFI border.
pub async fn argos_quote_swap(
    input_mint_b58: String,
    output_mint_b58: String,
    amount_in: u64,
    slippage_bps: u16,
) -> Result<ArgosSwapPreview, String> {
    let input_mint =
        Pubkey::from_str(input_mint_b58.trim()).map_err(|e| format!("input: {e}"))?;
    let output_mint =
        Pubkey::from_str(output_mint_b58.trim()).map_err(|e| format!("output: {e}"))?;
    let w = unlocked()?;
    let preview = w
        .quote_swap(&input_mint, &output_mint, amount_in, slippage_bps)
        .await
        .map_err(|e| format!("quote: {e}"))?;
    let info = ArgosSwapPreview {
        amount_in: preview.amount_in,
        amount_out_min: preview.amount_out_min,
        amount_out_expected: preview.amount_out_expected,
        platform_fee_out: preview.platform_fee_out,
        route_label: preview.route_label.clone(),
        slippage_bps: preview.slippage_bps,
        output_mint_b58: output_mint.to_string(),
    };
    *preview_cache().lock().map_err(|e| format!("lock: {e}"))? = Some(preview);
    Ok(info)
}

/// Execute the most recently quoted swap. Output stays in the user's wallet.
pub async fn argos_execute_swap() -> Result<String, String> {
    let preview = {
        let guard = preview_cache().lock().map_err(|e| format!("lock: {e}"))?;
        guard
            .as_ref()
            .cloned()
            .ok_or_else(|| "no preview cached — call argos_quote_swap first".to_string())?
    };
    let w = unlocked()?;
    let sig = w.swap(&preview).await.map_err(|e| format!("swap: {e}"))?;
    Ok(sig.to_string())
}

/// **Auto-Swap-on-Send** — Argos killer feature.
///
/// Atomic single-tx: swap `input_mint` → `output_mint`, deliver to
/// `recipient_b58` directly, collect 0.5 % platform fee. Uses the cached
/// preview from the previous `argos_quote_swap` call.
pub async fn argos_swap_and_send(
    recipient_b58: String,
) -> Result<ArgosSwapAndSendOutcome, String> {
    let recipient =
        Pubkey::from_str(recipient_b58.trim()).map_err(|e| format!("recipient: {e}"))?;
    let preview = {
        let guard = preview_cache().lock().map_err(|e| format!("lock: {e}"))?;
        guard
            .as_ref()
            .cloned()
            .ok_or_else(|| "no preview cached — call argos_quote_swap first".to_string())?
    };
    let w = unlocked()?;
    let outcome: SwapAndSendOutcome = w
        .swap_and_send(&preview, &recipient)
        .await
        .map_err(|e| format!("swap_and_send: {e}"))?;
    Ok(ArgosSwapAndSendOutcome {
        signature_b58: outcome.signature.to_string(),
        recipient_ata_b58: outcome.recipient_ata.to_string(),
        output_mint_b58: outcome.output_mint.to_string(),
        amount_out_expected: outcome.amount_out_expected,
    })
}

/// Validate a user-pasted Solana address. Returns the canonical base58
/// form on success, or a friendly error message on parse failure.
#[frb(sync)]
pub fn argos_validate_address(s: String) -> Result<String, String> {
    Pubkey::from_str(s.trim())
        .map(|pk| pk.to_string())
        .map_err(|e| format!("invalid Solana address: {e}"))
}

/// Request a 1 SOL airdrop on Devnet — for QA only. Fails on Mainnet.
pub async fn argos_devnet_airdrop_one_sol() -> Result<String, String> {
    let w = unlocked()?;
    let sig = w
        .airdrop_devnet_1_sol()
        .await
        .map_err(|e| format!("airdrop: {e}"))?;
    Ok(sig.to_string())
}


/// EVM (Ethereum / Base / Polygon) operations.
pub mod eth_api;
