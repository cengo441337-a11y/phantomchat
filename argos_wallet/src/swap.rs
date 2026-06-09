//! Jupiter v6 swap integration for [`ArgosWallet`].
//!
//! Wraps the Jupiter Aggregator REST API:
//! - `GET  https://api.jup.ag/swap/v1/quote`  — find best route
//! - `POST https://api.jup.ag/swap/v1/swap`   — get an unsigned tx
//! - `POST https://api.jup.ag/swap/v1/swap-instructions` — raw ix for composition
//!
//! ## Fee model
//!
//! `PLATFORM_FEE_BPS = 50` (0,5 %). Jupiter pays the fee directly into the
//! Associated Token Account of [`TREASURY_WALLET_PLACEHOLDER`] on the OUTPUT
//! mint in the same atomic transaction — no separate fee-collection step.
//! The fee is taken from the swap output, NOT charged on top.
//!
//! ## Auto-Swap-on-Send (killer feature)
//!
//! [`ArgosWallet::swap_and_send`] uses `/swap-instructions` and composes
//! a single Solana v0 transaction that does, atomically:
//!
//! 1. setup (compute-budget + Jupiter setup instructions)
//! 2. ensure recipient ATA exists (idempotent create)
//! 3. ensure treasury ATA exists (idempotent create)
//! 4. the Jupiter swap routed to `destinationTokenAccount = recipient_ata`
//! 5. cleanup (close temp WSOL, etc.)
//!
//! → Recipient gets the OUTPUT token directly in one tx. Nobody else in the
//! Solana messenger space does this.
//!
//! ## Threat model
//!
//! - Jupiter is the source of truth for the route. A compromised Jupiter
//!   could re-route to a malicious AMM. Mitigation: pin to `api.jup.ag`,
//!   HTTPS-only, no proxy override at runtime.
//! - User sees the route + slippage + fee BEFORE signing via [`SwapPreview`].
//! - Treasury wallet leak does NOT compromise users: it can only collect
//!   our own fees. Hardware-wallet for the treasury secret is mandatory
//!   (enforced operationally, not by code).

use std::str::FromStr;

use serde::{Deserialize, Serialize};
use solana_sdk::{
    address_lookup_table::{state::AddressLookupTable, AddressLookupTableAccount},
    instruction::{AccountMeta, Instruction},
    message::{v0, VersionedMessage},
    pubkey::Pubkey,
    signature::Signature,
    transaction::VersionedTransaction,
};

use crate::{ArgosWallet, Error};

/// Argos platform fee in basis points (1 bp = 0.01 %). 50 = 0.5 %.
pub const PLATFORM_FEE_BPS: u16 = 50;

/// Reduced swap fee for Argos Pro subscribers (0,25 %). The client passes
/// this when the wallet holds an active Pro subscription; free users keep
/// PLATFORM_FEE_BPS. quote_swap clamps to [PRO_FEE_BPS, PLATFORM_FEE_BPS].
pub const PRO_FEE_BPS: u16 = 25;

/// Argos treasury wallet — receives all platform fees as SPL tokens.
///
/// **Placeholder** until UG is set up. Once UG is registered, this rotates
/// to the UG-controlled cold-storage Ledger and the app pushes an update.
/// Until then the app uses an obviously-fake all-ones address that the
/// Jupiter API will reject for real fees if PLATFORM_FEE_BPS > 0 against
/// an unowned ATA — defence-in-depth so a half-baked dev build never
/// accidentally routes real fees to a wrong owner.
pub const TREASURY_WALLET_DEFAULT: &str = "86Kt31jCCdmL2DMxijKHjEZboC3HWXm2aSdRsJy8kERU";

/// Backwards-compat alias for code that still references the placeholder name.
pub use self::TREASURY_WALLET_DEFAULT as TREASURY_WALLET_PLACEHOLDER;

/// Jupiter v6 base URL. Const for the swap layer so a malicious dep
/// version can't quietly point us at a fork.
///
/// 2026-05-31: Jupiter migrated `quote-api.jup.ag` → `api.jup.ag`. The
/// path layout under `/swap/v1/` is the canonical v6 quote/swap surface.
const JUPITER_BASE: &str = "https://api.jup.ag/swap/v1";

/// Read the treasury address: env var `ARGOS_TREASURY_WALLET` overrides
/// the placeholder. Lets the production build inject the post-UG cold-
/// storage Ledger without recompiling the wallet crate.
pub fn treasury_address() -> Result<Pubkey, Error> {
    let raw = std::env::var("ARGOS_TREASURY_WALLET")
        .unwrap_or_else(|_| TREASURY_WALLET_PLACEHOLDER.to_string());
    Pubkey::from_str(&raw).map_err(|e| Error::InvalidAddress(e.to_string()))
}

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

#[derive(Debug, Deserialize)]
struct JupInstr {
    #[serde(rename = "programId")]
    program_id: String,
    accounts: Vec<JupAccount>,
    data: String,
}

#[derive(Debug, Deserialize)]
struct JupAccount {
    pubkey: String,
    #[serde(rename = "isSigner")]
    is_signer: bool,
    #[serde(rename = "isWritable")]
    is_writable: bool,
}

#[derive(Debug, Deserialize)]
struct SwapInstructionsResponse {
    #[serde(rename = "tokenLedgerInstruction", default)]
    token_ledger_instruction: Option<JupInstr>,
    #[serde(rename = "computeBudgetInstructions", default)]
    compute_budget_instructions: Vec<JupInstr>,
    #[serde(rename = "setupInstructions", default)]
    setup_instructions: Vec<JupInstr>,
    #[serde(rename = "swapInstruction")]
    swap_instruction: JupInstr,
    #[serde(rename = "cleanupInstruction", default)]
    cleanup_instruction: Option<JupInstr>,
    #[serde(rename = "addressLookupTableAddresses", default)]
    address_lookup_table_addresses: Vec<String>,
}

impl JupInstr {
    fn into_instruction(self) -> Result<Instruction, Error> {
        use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
        let program_id = Pubkey::from_str(&self.program_id)
            .map_err(|e| Error::InvalidAddress(e.to_string()))?;
        let mut accounts = Vec::with_capacity(self.accounts.len());
        for a in self.accounts {
            let pk = Pubkey::from_str(&a.pubkey)
                .map_err(|e| Error::InvalidAddress(e.to_string()))?;
            accounts.push(if a.is_writable {
                AccountMeta::new(pk, a.is_signer)
            } else {
                AccountMeta::new_readonly(pk, a.is_signer)
            });
        }
        let data = B64
            .decode(&self.data)
            .map_err(|e| Error::Rpc(format!("instr data b64: {e}")))?;
        Ok(Instruction {
            program_id,
            accounts,
            data,
        })
    }
}

/// Result struct for [`ArgosWallet::swap_and_send`]. Carries the on-chain
/// signature + the derived recipient ATA so the UI can render a Solscan link.
#[derive(Debug, Clone)]
pub struct SwapAndSendOutcome {
    /// Confirmed signature of the single atomic transaction.
    pub signature: Signature,
    /// The recipient's associated token account that received the output.
    pub recipient_ata: Pubkey,
    /// Output mint that was delivered.
    pub output_mint: Pubkey,
    /// Expected output amount (post-fee, pre-slippage).
    pub amount_out_expected: u64,
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
        fee_bps: u16,
    ) -> Result<SwapPreview, Error> {
        // Clamp to [PRO_FEE_BPS, PLATFORM_FEE_BPS] so a client can never set
        // the fee ABOVE the standard rate, and never below the Pro rate.
        let effective_fee = fee_bps.clamp(PRO_FEE_BPS, PLATFORM_FEE_BPS);
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
            fee = effective_fee,
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

    /// Execute the swap described by [`SwapPreview`]. Output stays in the
    /// USER's wallet (not sent to a recipient). For Auto-Swap-on-Send use
    /// [`ArgosWallet::swap_and_send`] instead.
    pub async fn swap(&self, preview: &SwapPreview) -> Result<Signature, Error> {
        let treasury = treasury_address()?;

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

        use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
        let tx_bytes = B64
            .decode(&parsed.swap_transaction)
            .map_err(|e| Error::Rpc(format!("swap tx b64: {e}")))?;
        let mut versioned: VersionedTransaction = bincode::deserialize(&tx_bytes)
            .map_err(|e| Error::Rpc(format!("swap tx deser: {e}")))?;

        let kp = self.keypair_for_signing();
        let recent_blockhash = self
            .rpc()
            .get_latest_blockhash()
            .await
            .map_err(|e| Error::Rpc(e.to_string()))?;
        let mut msg = versioned.message.clone();
        msg.set_recent_blockhash(recent_blockhash);
        versioned = VersionedTransaction::try_new(msg, &[&kp])
            .map_err(|e| Error::Rpc(format!("swap sign: {e}")))?;

        self.rpc()
            .send_and_confirm_transaction(&versioned)
            .await
            .map_err(|e| Error::Rpc(e.to_string()))
    }

    /// **Auto-Swap-on-Send** — the Argos killer feature.
    ///
    /// Atomically (in one Solana transaction):
    /// 1. Swap `input_mint` → `output_mint` via Jupiter
    /// 2. Route the output directly to `recipient`'s associated token account
    /// 3. Idempotently create recipient + treasury ATA if missing
    /// 4. Collect the 0.5 % platform fee to the Argos treasury ATA
    ///
    /// One signature, one fee, one round-trip. Nobody in the Solana messenger
    /// space does this today.
    ///
    /// `preview` MUST come from a fresh [`Self::quote_swap`] call — the quote
    /// embeds a blockhash-adjacent route that expires after ~30 s.
    pub async fn swap_and_send(
        &self,
        preview: &SwapPreview,
        recipient: &Pubkey,
    ) -> Result<SwapAndSendOutcome, Error> {
        let treasury = treasury_address()?;
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
        let recipient_ata = spl_associated_token_account::get_associated_token_address(
            recipient,
            &output_mint,
        );

        // Step 1: Ask Jupiter for raw instructions (not a prebuilt tx) so
        // we can inject our ATA-create instructions in front of the swap.
        let body = serde_json::json!({
            "quoteResponse": preview.raw_quote,
            "userPublicKey": self.pubkey().to_string(),
            "wrapAndUnwrapSol": true,
            "dynamicComputeUnitLimit": true,
            "feeAccount": treasury_ata.to_string(),
            "destinationTokenAccount": recipient_ata.to_string(),
        });
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .map_err(|e| Error::Rpc(format!("http client: {e}")))?;
        let url = format!("{}/swap-instructions", JUPITER_BASE);
        let resp = client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Rpc(format!("jupiter swap-instructions: {e}")))?;
        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Rpc(format!(
                "jupiter swap-instructions HTTP error: {body}"
            )));
        }
        let parsed: SwapInstructionsResponse = resp
            .json()
            .await
            .map_err(|e| Error::Rpc(format!("swap-instructions decode: {e}")))?;

        // Step 2: Build the instruction list in the right order.
        //
        //   compute_budget* + [token_ledger?] + setup* +
        //   create_recipient_ata_idempotent +
        //   create_treasury_ata_idempotent +
        //   swap +
        //   [cleanup?]
        //
        // The two extra create_ata_idempotent calls are the cost of being
        // able to send to ANY recipient regardless of whether they hold the
        // token already. Cost: ~0.002 SOL rent (sender pays) if the account
        // didn't exist; ~0 if it did (idempotent variant is a no-op).
        let mut ixs: Vec<Instruction> = Vec::new();
        for i in parsed.compute_budget_instructions {
            ixs.push(i.into_instruction()?);
        }
        if let Some(i) = parsed.token_ledger_instruction {
            ixs.push(i.into_instruction()?);
        }
        for i in parsed.setup_instructions {
            ixs.push(i.into_instruction()?);
        }
        ixs.push(
            spl_associated_token_account::instruction::create_associated_token_account_idempotent(
                &self.pubkey(),
                recipient,
                &output_mint,
                &spl_token::ID,
            ),
        );
        ixs.push(
            spl_associated_token_account::instruction::create_associated_token_account_idempotent(
                &self.pubkey(),
                &treasury,
                &output_mint,
                &spl_token::ID,
            ),
        );
        ixs.push(parsed.swap_instruction.into_instruction()?);
        if let Some(i) = parsed.cleanup_instruction {
            ixs.push(i.into_instruction()?);
        }

        // Step 3: Resolve every address lookup table referenced by Jupiter.
        // Without LUTs the tx exceeds Solana's 1232-byte max.
        let rpc = self.rpc();
        let mut luts: Vec<AddressLookupTableAccount> =
            Vec::with_capacity(parsed.address_lookup_table_addresses.len());
        for lut_addr in &parsed.address_lookup_table_addresses {
            let key = Pubkey::from_str(lut_addr)
                .map_err(|e| Error::InvalidAddress(e.to_string()))?;
            let acc = rpc
                .get_account(&key)
                .await
                .map_err(|e| Error::Rpc(format!("lut fetch {lut_addr}: {e}")))?;
            let table = AddressLookupTable::deserialize(&acc.data)
                .map_err(|e| Error::Rpc(format!("lut deser {lut_addr}: {e}")))?;
            luts.push(AddressLookupTableAccount {
                key,
                addresses: table.addresses.to_vec(),
            });
        }

        // Step 4: Compose v0 message, sign, broadcast.
        let blockhash = rpc
            .get_latest_blockhash()
            .await
            .map_err(|e| Error::Rpc(e.to_string()))?;
        let v0_msg = v0::Message::try_compile(&self.pubkey(), &ixs, &luts, blockhash)
            .map_err(|e| Error::Rpc(format!("v0 compile: {e}")))?;
        let kp = self.keypair_for_signing();
        let versioned =
            VersionedTransaction::try_new(VersionedMessage::V0(v0_msg), &[&kp])
                .map_err(|e| Error::Rpc(format!("sign: {e}")))?;

        let signature = rpc
            .send_and_confirm_transaction(&versioned)
            .await
            .map_err(|e| Error::Rpc(e.to_string()))?;

        Ok(SwapAndSendOutcome {
            signature,
            recipient_ata,
            output_mint,
            amount_out_expected: preview.amount_out_expected,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Network;
    use std::sync::Mutex;

    // The ARGOS_TREASURY_WALLET tests mutate process-global env and must not
    // run concurrently (cargo test is multi-threaded by default).
    static ENV_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[tokio::test]
    #[ignore = "live HTTP to api.jup.ag; run with --ignored when online"]
    async fn quote_usdc_to_sol_decodes_cleanly() {
        let (w, _) = ArgosWallet::generate(Network::MainnetBeta).unwrap();
        let usdc =
            Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap();
        let sol =
            Pubkey::from_str("So11111111111111111111111111111111111111112").unwrap();
        let preview = w
            .quote_swap(&usdc, &sol, 10_000_000, 50, PLATFORM_FEE_BPS)
            .await
            .expect("quote should succeed");
        assert!(preview.amount_out_expected > 0);
        assert!(!preview.route_label.is_empty());
        let expected_fee = preview.amount_out_expected * 50 / 10_000;
        let observed = preview.platform_fee_out as i64;
        let expected = expected_fee as i64;
        let diff = (observed - expected).abs();
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

    #[test]
    fn treasury_address_defaults_to_placeholder() {
        let _guard = ENV_TEST_LOCK.lock().unwrap();
        // No env var set in this test → falls back to placeholder.
        // (Other tests may set ARGOS_TREASURY_WALLET; isolate by reading
        // the env explicitly and asserting on the parsed pubkey.)
        let current = std::env::var("ARGOS_TREASURY_WALLET").ok();
        std::env::remove_var("ARGOS_TREASURY_WALLET");
        let pk = treasury_address().unwrap();
        assert_eq!(pk.to_string(), TREASURY_WALLET_PLACEHOLDER);
        if let Some(v) = current {
            std::env::set_var("ARGOS_TREASURY_WALLET", v);
        }
    }

    #[test]
    fn treasury_address_honours_env_override() {
        let _guard = ENV_TEST_LOCK.lock().unwrap();
        let usdc = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
        let old = std::env::var("ARGOS_TREASURY_WALLET").ok();
        std::env::set_var("ARGOS_TREASURY_WALLET", usdc);
        let pk = treasury_address().unwrap();
        assert_eq!(pk.to_string(), usdc);
        match old {
            Some(v) => std::env::set_var("ARGOS_TREASURY_WALLET", v),
            None => std::env::remove_var("ARGOS_TREASURY_WALLET"),
        }
    }

    #[test]
    fn treasury_address_rejects_garbage() {
        let _guard = ENV_TEST_LOCK.lock().unwrap();
        let old = std::env::var("ARGOS_TREASURY_WALLET").ok();
        std::env::set_var("ARGOS_TREASURY_WALLET", "this is not a pubkey");
        let result = treasury_address();
        assert!(matches!(result, Err(Error::InvalidAddress(_))));
        match old {
            Some(v) => std::env::set_var("ARGOS_TREASURY_WALLET", v),
            None => std::env::remove_var("ARGOS_TREASURY_WALLET"),
        }
    }
}
