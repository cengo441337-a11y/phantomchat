//! Jupiter v6 swap integration for [`ArgosWallet`].
//!
//! Wraps the Jupiter Aggregator REST API:
//! - `GET  https://api.jup.ag/swap/v1/quote`  — find best route
//! - `POST https://api.jup.ag/swap/v1/swap`   — get an unsigned tx
//!
//! ## Fee model
//!
//! `PLATFORM_FEE_BPS = 50` (0,5 %). Jupiter pays the fee directly into the
//! Associated Token Account of [`TREASURY_WALLET`] on the OUTPUT mint in
//! the same atomic transaction — no separate fee-collection step. The fee
//! is taken from the swap output, NOT charged on top.
//!
//! ## Threat model
//!
//! - Jupiter is the source of truth for the route. A compromised Jupiter
//!   could re-route to a malicious AMM. Mitigation: pin `quote_api_host`
//!   to `quote-api.jup.ag`, HTTPS-only, no proxy override at runtime.
//! - User sees the route + slippage + fee BEFORE signing. The `quote()`
//!   helper returns the human-readable preview struct; the swap is only
//!   built after a separate `swap()` call.
//! - Treasury wallet leak does NOT compromise users: it can only collect
//!   our own fees. Hardware-wallet for the treasury secret is mandatory
//!   (enforced operationally, not by code).

use std::str::FromStr;

use serde::{Deserialize, Serialize};
use solana_sdk::{
    pubkey::Pubkey,
    signature::Signature,
    transaction::VersionedTransaction,
};

use crate::{ArgosWallet, Error};

/// Argos platform fee in basis points (1 bp = 0.01 %). 50 = 0.5 %.
pub const PLATFORM_FEE_BPS: u16 = 50;

/// Argos treasury wallet — receives all platform fees as SPL tokens.
///
/// **Placeholder** until UG is set up. Once UG is registered, this rotates
/// to the UG-controlled cold-storage Ledger and the app pushes an update.
/// Until then the app uses an obviously-fake all-ones address that the
/// Jupiter API will reject — defence-in-depth so a half-baked dev build
/// never accidentally routes real fees to a wrong owner.
pub const TREASURY_WALLET_PLACEHOLDER: &str = "11111111111111111111111111111112";

/// Jupiter v6 base URL. Const for the swap layer so a malicious dep
/// version can't quietly point us at a fork.
///
/// 2026-05-31: Jupiter migrated `quote-api.jup.ag` -> `api.jup.ag`. The
/// path layout under `/swap/v1/` is the canonical v6 quote/swap surface.
const JUPITER_BASE: &str = "https://api.jup.ag/swap/v1";

// ── Public types ─────────────────────────────────────────────────────────

/// User-facing preview of a quoted swap. Surfaced to the UI so the user
/// can confirm BEFORE signing. All amounts are in raw token units (no
/// decimals applied) — UI does the human-readable formatting.
#[derive(Debug, Clone)]
pub struct SwapPreview {
    /// What the user pays, in input-mint smallest units.
    pub amount_in: u64,
    /// What the user receives, in output-mint smallest units (post-slippage).
    pub amount_out_min: u64,
    /// Expected (mid-point) output amount, in output-mint smallest units.
    pub amount_out_expected: u64,
    /// Platform fee, in output-mint smallest units (50 bps of out).
    pub platform_fee_out: u64,
    /// Route summary, e.g. "Raydium → Orca" (best-effort human label).
    pub route_label: String,
    /// Slippage applied, in basis points.
    pub slippage_bps: u16,
    /// Raw Jupiter quote JSON — passed verbatim to [`ArgosWallet::swap`].
    /// Opaque to the caller; do not mutate.
    pub raw_quote: serde_json::Value,
}

// ── Internal Jupiter API types ──────────────────────────────────────────

#[derive(Debug, Serialize)]
struct SwapRequest<'a> {
    #[serde(rename = "quoteResponse")]
    quote_response: &'a serde_json::Value,
    #[serde(rename = "userPublicKey")]
    user_public_key: String,
    #[serde(rename = "wrapAndUnwrapSol")]
    wrap_and_unwrap_sol: bool,
    #[serde(rename = "dynamicComputeUnitLimit")]
    dynamic_compute_unit_limit: bool,
    #[serde(rename = "feeAccount", skip_serializing_if = "Option::is_none")]
    fee_account: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SwapResponse {
    #[serde(rename = "swapTransaction")]
    swap_transaction: String,
}

// ── Public API ───────────────────────────────────────────────────────────

impl ArgosWallet {
    /// Ask Jupiter for the best route for the given trade and return a
    /// preview the UI can render before the user confirms. Pure read —
    /// no on-chain action yet.
    ///
    /// `slippage_bps`: 50 = 0.5 %, 100 = 1 %, etc. Default UI value: 50.
    pub async fn quote_swap(
        &self,
        input_mint: &Pubkey,
        output_mint: &Pubkey,
        amount_in: u64,
        slippage_bps: u16,
    ) -> Result<SwapPreview, Error> {
        let treasury = Pubkey::from_str(TREASURY_WALLET_PLACEHOLDER)
            .map_err(|e| Error::InvalidAddress(e.to_string()))?;

        // The Jupiter API derives the fee-account ATA from the treasury
        // pubkey + the output mint — we just hand it both via the
        // platformFeeBps + the feeAccount-via-prefix mechanism in /swap.
        // For /quote we only signal the BPS so Jupiter routes a path that
        // reserves the fee slot.
        let url = format!(
            "{base}/quote?\
             inputMint={input}&outputMint={output}&\
             amount={amount}&slippageBps={slippage}&\
             platformFeeBps={fee}",
            base = JUPITER_BASE,
            input = input_mint,
            output = output_mint,
            amount = amount_in,
            slippage = slippage_bps,
            fee = PLATFORM_FEE_BPS,
        );
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|e| Error::Rpc(format!("http client: {e}")))?;
        let resp = client
            .get(&url)
            .send()
            .await
            .map_err(|e| Error::Rpc(format!("jupiter quote: {e}")))?;
        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Rpc(format!("jupiter quote HTTP error: {body}")));
        }
        let raw_quote: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Rpc(format!("jupiter quote decode: {e}")))?;

        // Pull the structured fields from the raw quote. Jupiter changes
        // the shape occasionally — we read defensively and surface a
        // friendly error if a field is missing.
        let amount_out_expected = raw_quote
            .get("outAmount")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<u64>().ok())
            .ok_or_else(|| Error::Rpc("quote missing outAmount".into()))?;
        let amount_out_min = raw_quote
            .get("otherAmountThreshold")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<u64>().ok())
            .ok_or_else(|| Error::Rpc("quote missing otherAmountThreshold".into()))?;
        let platform_fee_out = raw_quote
            .get("platformFee")
            .and_then(|f| f.get("amount"))
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);

        // Best-effort human label of the route. Jupiter returns
        // routePlan[].swapInfo.label = AMM name.
        let route_label = raw_quote
            .get("routePlan")
            .and_then(|p| p.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|step| {
                        step.get("swapInfo")
                            .and_then(|si| si.get("label"))
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string())
                    })
                    .collect::<Vec<_>>()
                    .join(" → ")
            })
            .unwrap_or_else(|| "Jupiter".to_string());

        let _ = treasury; // referenced once /swap is the call that needs it
        Ok(SwapPreview {
            amount_in,
            amount_out_min,
            amount_out_expected,
            platform_fee_out,
            route_label,
            slippage_bps,
            raw_quote,
        })
    }

    /// Execute the swap described by [`SwapPreview`]. Signs locally with
    /// the wallet's keypair, broadcasts via the wallet's network RPC, and
    /// returns the confirmed signature.
    ///
    /// Fee is automatically collected to the Argos treasury ATA on the
    /// output mint — no caller-side work needed.
    pub async fn swap(&self, preview: &SwapPreview) -> Result<Signature, Error> {
        let treasury = Pubkey::from_str(TREASURY_WALLET_PLACEHOLDER)
            .map_err(|e| Error::InvalidAddress(e.to_string()))?;

        // Pull the output-mint out of the raw quote so we can derive the
        // treasury's output-mint ATA.
        let output_mint_str = preview
            .raw_quote
            .get("outputMint")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Rpc("quote missing outputMint".into()))?;
        let output_mint = Pubkey::from_str(output_mint_str)
            .map_err(|e| Error::InvalidAddress(e.to_string()))?;
        let treasury_ata = spl_associated_token_account::get_associated_token_address(
            &treasury,
            &output_mint,
        );

        let body = SwapRequest {
            quote_response: &preview.raw_quote,
            user_public_key: self.pubkey().to_string(),
            wrap_and_unwrap_sol: true,
            dynamic_compute_unit_limit: true,
            fee_account: Some(treasury_ata.to_string()),
        };
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .map_err(|e| Error::Rpc(format!("http client: {e}")))?;
        let url = format!("{}/swap", JUPITER_BASE);
        let resp = client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Rpc(format!("jupiter swap: {e}")))?;
        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Rpc(format!("jupiter swap HTTP error: {body}")));
        }
        let parsed: SwapResponse = resp
            .json()
            .await
            .map_err(|e| Error::Rpc(format!("jupiter swap decode: {e}")))?;

        // Base64 → versioned transaction → sign → broadcast.
        use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
        let tx_bytes = B64
            .decode(&parsed.swap_transaction)
            .map_err(|e| Error::Rpc(format!("swap tx b64: {e}")))?;
        let mut versioned: VersionedTransaction = bincode::deserialize(&tx_bytes)
            .map_err(|e| Error::Rpc(format!("swap tx deser: {e}")))?;

        // Re-sign with our keypair. The Jupiter-returned tx has the
        // user_public_key baked in as the fee-payer + signer, but the
        // signature is empty; we fill it in here.
        let kp = self.keypair_for_signing();
        let recent_blockhash = self
            .rpc()
            .get_latest_blockhash()
            .await
            .map_err(|e| Error::Rpc(e.to_string()))?;
        // Replace the placeholder blockhash with a fresh one so we don't
        // race the Jupiter-quote-side blockhash expiry.
        let mut msg = versioned.message.clone();
        msg.set_recent_blockhash(recent_blockhash);
        versioned = VersionedTransaction::try_new(msg, &[&kp])
            .map_err(|e| Error::Rpc(format!("swap sign: {e}")))?;

        self.rpc()
            .send_and_confirm_transaction(&versioned)
            .await
            .map_err(|e| Error::Rpc(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Network;

    /// Sanity-check we hit the right Jupiter host and decode the response
    /// shape. Runs against the live API (mainnet quote-api), so it's
    /// gated behind --ignored. No on-chain action.
    #[tokio::test]
    #[ignore = "live HTTP to quote-api.jup.ag; run with --ignored when online"]
    async fn quote_usdc_to_sol_decodes_cleanly() {
        let (w, _) = ArgosWallet::generate(Network::MainnetBeta).unwrap();
        let usdc =
            Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap();
        let sol =
            Pubkey::from_str("So11111111111111111111111111111111111111112").unwrap();
        // 10 USDC (USDC has 6 decimals → 10_000_000)
        let preview = w
            .quote_swap(&usdc, &sol, 10_000_000, 50)
            .await
            .expect("quote should succeed");
        assert!(preview.amount_out_expected > 0);
        assert!(!preview.route_label.is_empty());
        // Fee should be ~50 bps of out
        let expected_fee = preview.amount_out_expected * 50 / 10_000;
        let observed = preview.platform_fee_out as i64;
        let expected = expected_fee as i64;
        let diff = (observed - expected).abs();
        // Allow 1 % rounding tolerance because Jupiter computes off the
        // routed path, not the round-number expected output.
        assert!(
            diff < (expected_fee / 100 + 100) as i64,
            "fee mismatch: expected ~{expected_fee}, got {}",
            preview.platform_fee_out
        );
    }

    #[test]
    fn placeholder_treasury_parses() {
        let pk = Pubkey::from_str(TREASURY_WALLET_PLACEHOLDER).unwrap();
        assert_eq!(pk.to_string(), TREASURY_WALLET_PLACEHOLDER);
    }
}
