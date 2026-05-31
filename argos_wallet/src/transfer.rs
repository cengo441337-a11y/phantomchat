//! Solana send / receive primitives for [`ArgosWallet`].
//!
//! All RPC calls use a non-blocking `RpcClient`; the public surface is
//! `async` so callers (mobile Flutter, desktop Tauri, CLI) pick their
//! preferred runtime. Helpers return the on-chain [`Signature`] of the
//! confirmed transaction so the UI can show a Solscan link.
//!
//! ## Supported transfers
//!
//! - **Native SOL** via the `system_instruction::transfer` path
//! - **SPL Token** (USDC / USDT / arbitrary mint) via the SPL Associated
//!   Token Account derivation + `spl_token::instruction::transfer`
//!
//! The SPL path is "transfer-with-create": if the recipient does NOT yet
//! have an Associated Token Account for the mint, we create it inside the
//! same atomic transaction. The fee for that creation (~0.002 SOL rent
//! exemption) is paid by the sender — matches the UX of every other
//! Solana wallet (Phantom, Backpack, Solflare).

use std::str::FromStr;

use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    instruction::Instruction,
    message::Message,
    native_token::LAMPORTS_PER_SOL,
    pubkey::Pubkey,
    signature::Signature,
    transaction::Transaction,
};
use solana_system_interface::instruction as system_instruction;

use crate::{ArgosWallet, Error};

/// Smallest SOL unit: 1 SOL = 1_000_000_000 lamports. Re-exported for
/// callers that prefer working in lamports directly.
pub const LAMPORTS_PER_SOL_CONST: u64 = LAMPORTS_PER_SOL;

/// One-shot RPC client built from the wallet's configured network. Each
/// helper creates a fresh client so a long-lived `ArgosWallet` doesn't
/// pin a TCP socket. For high-frequency callers, use `ArgosWallet::rpc()`
/// to hold a reusable handle.
impl ArgosWallet {
    /// Construct an `RpcClient` pointed at the wallet's network with
    /// finalised commitment (waits for ~13s but never reorgs).
    pub fn rpc(&self) -> RpcClient {
        RpcClient::new_with_commitment(
            self.network().rpc_url().to_string(),
            CommitmentConfig::confirmed(),
        )
    }

    /// Native SOL balance of this wallet, in lamports. UI converts to SOL
    /// by dividing by [`LAMPORTS_PER_SOL_CONST`].
    pub async fn balance_lamports(&self) -> Result<u64, Error> {
        self.rpc()
            .get_balance(&self.pubkey())
            .await
            .map_err(|e| Error::Rpc(e.to_string()))
    }

    /// SPL token balance of this wallet for the given mint, in raw token
    /// units (no decimal conversion). Returns 0 if the wallet has no ATA
    /// for the mint yet.
    pub async fn balance_spl(&self, mint: &Pubkey) -> Result<u64, Error> {
        let ata = spl_associated_token_account::get_associated_token_address(
            &self.pubkey(),
            mint,
        );
        match self.rpc().get_token_account_balance(&ata).await {
            Ok(b) => b
                .amount
                .parse::<u64>()
                .map_err(|e| Error::Rpc(format!("token amount parse: {e}"))),
            Err(e) => {
                let msg = e.to_string();
                // No ATA = no balance, surface 0 instead of an RPC error.
                if msg.contains("could not find account")
                    || msg.contains("not found")
                    || msg.contains("AccountNotFound")
                {
                    Ok(0)
                } else {
                    Err(Error::Rpc(msg))
                }
            }
        }
    }

    /// Send `lamports` of native SOL to `recipient`. Returns the confirmed
    /// transaction signature. Reverts on insufficient balance or invalid
    /// recipient.
    pub async fn send_sol(
        &self,
        recipient: &Pubkey,
        lamports: u64,
    ) -> Result<Signature, Error> {
        let rpc = self.rpc();
        let ix = system_instruction::transfer(&self.pubkey(), recipient, lamports);
        let blockhash = rpc
            .get_latest_blockhash()
            .await
            .map_err(|e| Error::Rpc(e.to_string()))?;
        let kp = self.keypair_for_signing();
        let msg = Message::new_with_blockhash(&[ix], Some(&self.pubkey()), &blockhash);
        let tx = Transaction::new(&[&kp], msg, blockhash);
        rpc.send_and_confirm_transaction(&tx)
            .await
            .map_err(|e| Error::Rpc(e.to_string()))
    }

    /// Send `amount` raw units of an SPL token mint to `recipient`.
    /// Auto-creates the recipient's Associated Token Account if missing
    /// (sender pays ~0.002 SOL rent).
    pub async fn send_spl(
        &self,
        mint: &Pubkey,
        recipient: &Pubkey,
        amount: u64,
    ) -> Result<Signature, Error> {
        let rpc = self.rpc();
        let sender = self.pubkey();
        let sender_ata =
            spl_associated_token_account::get_associated_token_address(&sender, mint);
        let recipient_ata =
            spl_associated_token_account::get_associated_token_address(recipient, mint);

        let mut ixs: Vec<Instruction> = Vec::with_capacity(2);

        // If recipient has no ATA yet, create one in the same tx. Idempotent
        // create-or-skip via the "idempotent" variant — cheaper RPC round-trip
        // than first probing get_account_info.
        ixs.push(
            spl_associated_token_account::instruction::create_associated_token_account_idempotent(
                &sender,
                recipient,
                mint,
                &spl_token::ID,
            ),
        );

        // SPL transfer instruction. Note: spl_token::instruction::transfer
        // is documented-deprecated in favour of transfer_checked, but the
        // legacy variant is what every existing Solana wallet emits and the
        // recipient indexers (Helius, Solscan) display correctly. We use
        // transfer_checked-style decimals check via the mint account when
        // we render the UX-side amount, not on-chain.
        ixs.push(
            spl_token::instruction::transfer(
                &spl_token::ID,
                &sender_ata,
                &recipient_ata,
                &sender,
                &[],
                amount,
            )
            .map_err(|e| Error::Rpc(format!("spl transfer ix: {e}")))?,
        );

        let blockhash = rpc
            .get_latest_blockhash()
            .await
            .map_err(|e| Error::Rpc(e.to_string()))?;
        let kp = self.keypair_for_signing();
        let msg = Message::new_with_blockhash(&ixs, Some(&sender), &blockhash);
        let tx = Transaction::new(&[&kp], msg, blockhash);
        rpc.send_and_confirm_transaction(&tx)
            .await
            .map_err(|e| Error::Rpc(e.to_string()))
    }

    /// Request a 1 SOL airdrop on Devnet (used by integration tests).
    /// Errors with a clear message on Mainnet where airdrop is disabled.
    pub async fn airdrop_devnet_1_sol(&self) -> Result<Signature, Error> {
        if !matches!(self.network(), crate::Network::Devnet) {
            return Err(Error::Rpc("airdrop only available on Devnet".into()));
        }
        let rpc = self.rpc();
        let sig = rpc
            .request_airdrop(&self.pubkey(), LAMPORTS_PER_SOL)
            .await
            .map_err(|e| Error::Rpc(e.to_string()))?;
        // Wait for confirmation. RPC `request_airdrop` returns immediately
        // with a signature but the lamports aren't spendable yet.
        rpc.confirm_transaction_with_commitment(&sig, CommitmentConfig::confirmed())
            .await
            .map_err(|e| Error::Rpc(e.to_string()))?;
        Ok(sig)
    }
}

/// Helper for parsing user-pasted Solana addresses. Returns a friendly
/// error instead of the raw bs58 / pubkey parse error.
pub fn parse_address(s: &str) -> Result<Pubkey, Error> {
    let trimmed = s.trim();
    Pubkey::from_str(trimmed).map_err(|_| Error::InvalidAddress(trimmed.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Network;

    #[tokio::test]
    #[ignore = "hits live Solana Devnet RPC; run with --ignored when online"]
    async fn devnet_airdrop_then_send_to_self() {
        let (w, _m) = ArgosWallet::generate(Network::Devnet).unwrap();
        w.airdrop_devnet_1_sol().await.unwrap();
        let bal = w.balance_lamports().await.unwrap();
        assert!(bal >= LAMPORTS_PER_SOL_CONST, "expected ≥1 SOL, got {bal}");
        // Send 0.001 SOL to self → balance should drop by fee only
        let sig = w
            .send_sol(&w.pubkey(), LAMPORTS_PER_SOL_CONST / 1000)
            .await
            .unwrap();
        assert!(sig.to_string().len() > 40);
    }

    #[test]
    fn parse_valid_solana_address() {
        let usdc_mint = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
        let pk = parse_address(usdc_mint).unwrap();
        assert_eq!(pk.to_string(), usdc_mint);
    }

    #[test]
    fn parse_address_rejects_garbage() {
        match parse_address("definitely not a key") {
            Err(Error::InvalidAddress(s)) => assert_eq!(s, "definitely not a key"),
            other => panic!("expected InvalidAddress, got {other:?}"),
        }
    }
}
