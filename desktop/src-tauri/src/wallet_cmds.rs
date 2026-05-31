//! Tauri commands that bridge the existing `argos_wallet` Rust crate to
//! the React frontend.
//!
//! Mirrors the mobile FFI surface in `mobile/rust/src/wallet_api.rs` so
//! the Desktop and Mobile flows look identical from the UI side. We pin
//! a single `Arc<ArgosWallet>` in a `OnceCell<Mutex<...>>` cache; UI calls
//! `argos_unlock_wallet` once and every subsequent command (`balance`,
//! `send_sol`, `send_token`, `quote_swap`, `swap`, `swap_and_send`) just
//! grabs a fresh `Arc::clone` and drops the lock before any await.
//!
//! No Tauri-state plumbing — the cache is module-level, identical pattern
//! to mobile, so a future refactor can deduplicate both surfaces.

use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use argos_wallet::swap::{SwapAndSendOutcome, SwapPreview};
use argos_wallet::{ArgosWallet, Network};
use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};
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
        .ok_or_else(|| "wallet locked".to_string())
}

#[derive(Debug, Clone, Serialize)]
pub struct ArgosWalletInfo {
    pub pubkey_b58: String,
    pub mnemonic: String,
    pub network: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArgosSwapPreview {
    pub amount_in: u64,
    pub amount_out_min: u64,
    pub amount_out_expected: u64,
    pub platform_fee_out: u64,
    pub route_label: String,
    pub slippage_bps: u16,
    pub output_mint_b58: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArgosSwapAndSendOutcomeDto {
    pub signature_b58: String,
    pub recipient_ata_b58: String,
    pub output_mint_b58: String,
    pub amount_out_expected: u64,
}

#[derive(Debug, Deserialize)]
pub struct StoragePath {
    pub path: String,
}

// ── Commands ─────────────────────────────────────────────────────────────

#[tauri::command]
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
    Ok(ArgosWalletInfo {
        pubkey_b58: pk,
        mnemonic: mnemonic.to_string(),
        network,
    })
}

#[tauri::command]
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
    Ok(ArgosWalletInfo {
        pubkey_b58: pk,
        mnemonic: mnemonic.trim().to_string(),
        network,
    })
}

#[tauri::command]
pub fn argos_unlock_wallet(
    pin: String,
    storage_path: String,
) -> Result<String, String> {
    let path = PathBuf::from(&storage_path);
    let wallet =
        ArgosWallet::load_encrypted(&pin, &path).map_err(|e| format!("unlock: {e}"))?;
    let pk = wallet.pubkey().to_string();
    *cache().lock().map_err(|e| format!("lock: {e}"))? = Some(Arc::new(wallet));
    Ok(pk)
}

#[tauri::command]
pub fn argos_lock_wallet() -> Result<(), String> {
    *cache().lock().map_err(|e| format!("lock: {e}"))? = None;
    *preview_cache().lock().map_err(|e| format!("lock: {e}"))? = None;
    Ok(())
}

#[tauri::command]
pub fn argos_wallet_pubkey() -> Option<String> {
    cache()
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|w| w.pubkey().to_string()))
}

#[tauri::command]
pub fn argos_wallet_network() -> Option<String> {
    cache().lock().ok().and_then(|g| {
        g.as_ref().map(|w| match w.network() {
            Network::MainnetBeta => "mainnet-beta".to_string(),
            Network::Devnet => "devnet".to_string(),
        })
    })
}

#[tauri::command]
pub async fn argos_balance_sol() -> Result<u64, String> {
    let w = unlocked()?;
    w.balance_lamports()
        .await
        .map_err(|e| format!("rpc: {e}"))
}

#[tauri::command]
pub async fn argos_balance_token(mint_b58: String) -> Result<u64, String> {
    let mint = Pubkey::from_str(mint_b58.trim()).map_err(|e| format!("mint: {e}"))?;
    let w = unlocked()?;
    w.balance_spl(&mint).await.map_err(|e| format!("rpc: {e}"))
}

#[tauri::command]
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

#[tauri::command]
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

#[tauri::command]
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

#[tauri::command]
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

#[tauri::command]
pub async fn argos_swap_and_send(
    recipient_b58: String,
) -> Result<ArgosSwapAndSendOutcomeDto, String> {
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
    Ok(ArgosSwapAndSendOutcomeDto {
        signature_b58: outcome.signature.to_string(),
        recipient_ata_b58: outcome.recipient_ata.to_string(),
        output_mint_b58: outcome.output_mint.to_string(),
        amount_out_expected: outcome.amount_out_expected,
    })
}

#[tauri::command]
pub fn argos_validate_address(s: String) -> Result<String, String> {
    Pubkey::from_str(s.trim())
        .map(|pk| pk.to_string())
        .map_err(|e| format!("invalid Solana address: {e}"))
}

#[tauri::command]
pub async fn argos_devnet_airdrop_one_sol() -> Result<String, String> {
    let w = unlocked()?;
    let sig = w
        .airdrop_devnet_1_sol()
        .await
        .map_err(|e| format!("airdrop: {e}"))?;
    Ok(sig.to_string())
}
