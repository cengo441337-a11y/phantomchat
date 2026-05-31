//! Native ETH + ERC-20 transfer paths.
//!
//! Type-2 EIP-1559 transactions only — every modern L1/L2 supports them,
//! and the legacy type-0 path adds 200 LOC of edge-cases for no benefit
//! to our use-case.

use primitive_types::U256;
use rlp::RlpStream;
use secp256k1::ecdsa::RecoverableSignature;
use secp256k1::{Message, Secp256k1};
use sha3::{Digest, Keccak256};

use crate::rpc::parse_eth_address;
use crate::{ArgosEthWallet, Error};

const TX_TYPE_EIP1559: u8 = 2;

/// Gas limit for native ETH transfer. Foundation 21k spec.
pub const GAS_LIMIT_ETH: u64 = 21_000;

/// Gas limit ceiling for ERC-20 transfers. Most tokens land at ~65 k; we
/// over-provision so we don't underfund a token with hooks (USDT-style).
pub const GAS_LIMIT_ERC20: u64 = 100_000;

/// Priority-fee floor we add to the base fee to bid the tx into the next
/// few blocks. 1.5 gwei is the de-facto retail default on Mainnet and Base.
pub const PRIORITY_FEE_WEI: u128 = 1_500_000_000;

impl ArgosEthWallet {
    /// Send `wei` of native ETH/MATIC/etc. to `recipient`. Returns the
    /// resulting transaction hash (`0x…`) once the node accepts the
    /// broadcast. Does NOT wait for inclusion — callers can poll
    /// `eth_getTransactionReceipt` to confirm.
    pub async fn send_native(
        &self,
        recipient: &str,
        wei: U256,
    ) -> Result<String, Error> {
        let to = parse_eth_address(recipient)?;
        let nonce = self.nonce().await?;
        let base = self.base_fee_wei().await?;
        let max_priority_fee = U256::from(PRIORITY_FEE_WEI);
        // Bid 2x base + priority for fast inclusion.
        let max_fee_per_gas = base.saturating_mul(U256::from(2u64)) + max_priority_fee;
        let signed = self.sign_eip1559(
            nonce,
            max_priority_fee,
            max_fee_per_gas,
            GAS_LIMIT_ETH,
            Some(to),
            wei,
            Vec::new(),
        )?;
        let hex = format!("0x{}", hex::encode(&signed));
        self.send_raw(&hex).await
    }

    /// ERC-20 `transfer(address,uint256)`. Returns the tx-hash on broadcast.
    pub async fn send_erc20(
        &self,
        token: &str,
        recipient: &str,
        amount: U256,
    ) -> Result<String, Error> {
        let to = parse_eth_address(recipient)?;
        let token = parse_eth_address(token)?;
        let nonce = self.nonce().await?;
        let base = self.base_fee_wei().await?;
        let max_priority_fee = U256::from(PRIORITY_FEE_WEI);
        let max_fee_per_gas = base.saturating_mul(U256::from(2u64)) + max_priority_fee;

        let mut data = Vec::with_capacity(68);
        // transfer(address,uint256) selector
        data.extend_from_slice(&hex::decode("a9059cbb").unwrap());
        // padded address
        data.extend_from_slice(&[0u8; 12]);
        data.extend_from_slice(&to);
        // uint256 amount BE
        let amount_be = amount.to_big_endian();
        data.extend_from_slice(&amount_be);

        let signed = self.sign_eip1559(
            nonce,
            max_priority_fee,
            max_fee_per_gas,
            GAS_LIMIT_ERC20,
            Some(token),
            U256::zero(),
            data,
        )?;
        let hex_str = format!("0x{}", hex::encode(&signed));
        self.send_raw(&hex_str).await
    }

    /// Build + sign an EIP-1559 (type-2) tx. Internal helper.
    fn sign_eip1559(
        &self,
        nonce: u64,
        max_priority_fee_per_gas: U256,
        max_fee_per_gas: U256,
        gas_limit: u64,
        to: Option<[u8; 20]>,
        value: U256,
        data: Vec<u8>,
    ) -> Result<Vec<u8>, Error> {
        let chain_id = self.network().chain_id();
        // Hash the unsigned tx (type prefix + RLP).
        let unsigned = rlp_encode_eip1559(
            chain_id,
            nonce,
            max_priority_fee_per_gas,
            max_fee_per_gas,
            gas_limit,
            to,
            value,
            &data,
            &[],
            None,
        );
        let mut hasher = Keccak256::new();
        hasher.update([TX_TYPE_EIP1559]);
        hasher.update(&unsigned);
        let hash = hasher.finalize();

        let secp = Secp256k1::new();
        let msg = Message::from_digest_slice(&hash[..])
            .map_err(|e| Error::Tx(format!("msg: {e}")))?;
        let sk = self.secret_key()?;
        let recoverable: RecoverableSignature =
            secp.sign_ecdsa_recoverable(&msg, &sk);
        let (rec_id, sig_bytes) = recoverable.serialize_compact();
        let r = U256::from_big_endian(&sig_bytes[..32]);
        let s = U256::from_big_endian(&sig_bytes[32..]);
        let y_parity = rec_id.to_i32() as u8;

        // Signed RLP — re-encodes all fields plus (y_parity, r, s).
        let signed = rlp_encode_eip1559(
            chain_id,
            nonce,
            max_priority_fee_per_gas,
            max_fee_per_gas,
            gas_limit,
            to,
            value,
            &data,
            &[],
            Some((y_parity, r, s)),
        );
        let mut out = Vec::with_capacity(signed.len() + 1);
        out.push(TX_TYPE_EIP1559);
        out.extend_from_slice(&signed);
        Ok(out)
    }
}

fn rlp_encode_eip1559(
    chain_id: u64,
    nonce: u64,
    max_priority_fee_per_gas: U256,
    max_fee_per_gas: U256,
    gas_limit: u64,
    to: Option<[u8; 20]>,
    value: U256,
    data: &[u8],
    access_list: &[u8],
    sig: Option<(u8, U256, U256)>,
) -> Vec<u8> {
    let n_fields = 9 + if sig.is_some() { 3 } else { 0 };
    let mut s = RlpStream::new_list(n_fields);
    s.append(&chain_id);
    s.append(&nonce);
    append_u256(&mut s, max_priority_fee_per_gas);
    append_u256(&mut s, max_fee_per_gas);
    s.append(&gas_limit);
    match to {
        Some(addr) => {
            s.append(&addr.as_slice());
        }
        None => {
            s.append_empty_data();
        }
    }
    append_u256(&mut s, value);
    s.append(&data);
    s.append_list::<u8, _>(access_list); // empty access list — typed but we never populate
    if let Some((y_parity, r, sig_s)) = sig {
        s.append(&y_parity);
        append_u256(&mut s, r);
        append_u256(&mut s, sig_s);
    }
    s.out().to_vec()
}

fn append_u256(s: &mut RlpStream, value: U256) {
    // RLP encodes integers as the shortest big-endian byte string with no
    // leading zeros. `U256::to_big_endian` always gives us 32 bytes; we
    // strip the leading zeros ourselves to match the spec.
    let buf = value.to_big_endian();
    let first_nonzero = buf
        .iter()
        .position(|b| *b != 0)
        .unwrap_or(buf.len());
    s.append(&&buf[first_nonzero..]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EthNetwork;

    /// Sanity: signing produces SOMETHING and doesn't crash. We can\'t
    /// assert byte-equality against a hand-crafted reference without
    /// pulling a full RLP testkit, but this catches regressions where the
    /// secret-key path falls through.
    #[test]
    fn sign_eip1559_eth_transfer_does_not_panic() {
        let m = "test test test test test test test test test test test junk";
        let w =
            ArgosEthWallet::from_mnemonic(m, "", EthNetwork::Mainnet).unwrap();
        let to = parse_eth_address("0xE65B85fd7ae369c0E107F8fAcD0ae163040f3763").unwrap();
        let bytes = w
            .sign_eip1559(
                0,
                U256::from(1_500_000_000u64),
                U256::from(50_000_000_000u64),
                21_000,
                Some(to),
                U256::from(10u64).pow(U256::from(15u64)),
                Vec::new(),
            )
            .unwrap();
        // type prefix + RLP list is at least ~70 bytes for an ETH transfer.
        assert!(bytes.len() > 64, "signed tx unexpectedly short: {}", bytes.len());
        assert_eq!(bytes[0], TX_TYPE_EIP1559);
    }
}
