//! Lightweight JSON-RPC client for the Argos Ethereum wallet.
//!
//! We don't pull `ethers-rs` or `alloy` — both are massive and one of
//! them currently fights the FFI-side tokio runtime. A handful of typed
//! wrappers over `reqwest` is plenty for balance reads + raw-tx broadcast.

use std::time::Duration;

use primitive_types::U256;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ArgosEthWallet, Error};

const RPC_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Serialize)]
struct RpcRequest<'a> {
    jsonrpc: &'static str,
    method: &'a str,
    params: Value,
    id: u32,
}

#[derive(Debug, Deserialize)]
struct RpcResponse<T> {
    #[serde(default)]
    result: Option<T>,
    #[serde(default)]
    error: Option<RpcError>,
}

#[derive(Debug, Deserialize)]
struct RpcError {
    code: i64,
    message: String,
}

async fn call<T: for<'de> Deserialize<'de> + Default>(
    url: &str,
    method: &str,
    params: Value,
) -> Result<T, Error> {
    let client = reqwest::Client::builder()
        .timeout(RPC_TIMEOUT)
        .build()
        .map_err(|e| Error::Rpc(format!("client: {e}")))?;
    let req = RpcRequest {
        jsonrpc: "2.0",
        method,
        params,
        id: 1,
    };
    let resp = client
        .post(url)
        .json(&req)
        .send()
        .await
        .map_err(|e| Error::Rpc(format!("send {method}: {e}")))?;
    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(Error::Rpc(format!("HTTP {method}: {body}")));
    }
    let parsed: RpcResponse<T> = resp
        .json()
        .await
        .map_err(|e| Error::Rpc(format!("decode {method}: {e}")))?;
    if let Some(err) = parsed.error {
        return Err(Error::Rpc(format!(
            "{} {}: {}",
            method, err.code, err.message
        )));
    }
    parsed
        .result
        .ok_or_else(|| Error::Rpc(format!("{method}: empty result")))
}

fn parse_hex_u256(s: &str) -> Result<U256, Error> {
    let trimmed = s.strip_prefix("0x").unwrap_or(s);
    U256::from_str_radix(trimmed, 16).map_err(|e| Error::Rpc(format!("u256: {e}")))
}

fn parse_hex_u64(s: &str) -> Result<u64, Error> {
    let trimmed = s.strip_prefix("0x").unwrap_or(s);
    u64::from_str_radix(trimmed, 16).map_err(|e| Error::Rpc(format!("u64: {e}")))
}

impl ArgosEthWallet {
    /// Native balance in wei.
    pub async fn balance_wei(&self) -> Result<U256, Error> {
        let addr_hex = format!("0x{}", hex::encode(self.address_bytes()));
        let result: String = call(
            self.rpc_url(),
            "eth_getBalance",
            serde_json::json!([addr_hex, "latest"]),
        )
        .await?;
        parse_hex_u256(&result)
    }

    /// Current nonce (pending), the value the next signed tx must use.
    pub async fn nonce(&self) -> Result<u64, Error> {
        let addr_hex = format!("0x{}", hex::encode(self.address_bytes()));
        let result: String = call(
            self.rpc_url(),
            "eth_getTransactionCount",
            serde_json::json!([addr_hex, "pending"]),
        )
        .await?;
        parse_hex_u64(&result)
    }

    /// Latest block's `baseFeePerGas`. Foundation for EIP-1559 fee math.
    pub async fn base_fee_wei(&self) -> Result<U256, Error> {
        let result: Value = call(
            self.rpc_url(),
            "eth_getBlockByNumber",
            serde_json::json!(["latest", false]),
        )
        .await?;
        let base = result
            .get("baseFeePerGas")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Rpc("block missing baseFeePerGas".into()))?;
        parse_hex_u256(base)
    }

    /// `eth_call` against an ERC-20 `balanceOf(address)` for this wallet.
    /// Returns the raw token amount.
    pub async fn erc20_balance(&self, token: &str) -> Result<U256, Error> {
        let token = parse_eth_address(token)?;
        // balanceOf(address) selector: 0x70a08231
        let mut data = Vec::with_capacity(36);
        data.extend_from_slice(&hex::decode("70a08231").unwrap());
        // ABI-encode a single address: 32-byte big-endian, left-padded with 0s.
        data.extend_from_slice(&[0u8; 12]);
        data.extend_from_slice(&self.address_bytes());
        let payload = serde_json::json!([
            {
                "to": format!("0x{}", hex::encode(token)),
                "data": format!("0x{}", hex::encode(&data)),
            },
            "latest"
        ]);
        let result: String = call(self.rpc_url(), "eth_call", payload).await?;
        parse_hex_u256(&result)
    }

    /// Broadcast a fully-signed RLP-encoded transaction.
    pub async fn send_raw(&self, raw_signed_tx_hex_with_0x: &str) -> Result<String, Error> {
        let result: String = call(
            self.rpc_url(),
            "eth_sendRawTransaction",
            serde_json::json!([raw_signed_tx_hex_with_0x]),
        )
        .await?;
        Ok(result)
    }
}

/// Parse a `0x…` 40-char hex address into 20 raw bytes. Lower-cases first
/// so we don't enforce EIP-55 (which would reject lower-case-only addresses
/// the user might paste from somewhere informal).
pub fn parse_eth_address(s: &str) -> Result<[u8; 20], Error> {
    let trimmed = s.trim();
    let no0x = trimmed.strip_prefix("0x").unwrap_or(trimmed);
    if no0x.len() != 40 {
        return Err(Error::InvalidAddress(format!(
            "want 40 hex chars (20 bytes), got {}",
            no0x.len()
        )));
    }
    let bytes = hex::decode(no0x)
        .map_err(|_| Error::InvalidAddress(format!("not valid hex: {trimmed}")))?;
    let mut out = [0u8; 20];
    out.copy_from_slice(&bytes);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lower_address() {
        let a = parse_eth_address("0xe65b85fd7ae369c0e107f8facd0ae163040f3763").unwrap();
        assert_eq!(a.len(), 20);
    }

    #[test]
    fn parses_checksum_address() {
        let a = parse_eth_address("0xE65B85fd7ae369c0E107F8fAcD0ae163040f3763").unwrap();
        assert_eq!(a.len(), 20);
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(parse_eth_address("0xdeadbeef").is_err());
    }

    #[tokio::test]
    #[ignore = "live HTTP to llamarpc.com; run with --ignored when online"]
    async fn live_balance_of_deniz_metamask() {
        // Sanity check: live read of the founder MetaMask address. Not
        // useful as a real assertion (balance can vary) but proves the
        // JSON-RPC round-trip works against the public gateway.
        let m =
            "test test test test test test test test test test test junk";
        let w = ArgosEthWallet::from_mnemonic(m, "", crate::EthNetwork::Mainnet).unwrap();
        let bal = w.balance_wei().await.unwrap();
        // U256::is_zero is fine even with no balance.
        let _ = bal;
    }
}
