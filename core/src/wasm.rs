//! WebAssembly bindings.
//!
//! Exposes a compact, **stateless** subset of the PhantomChat API to
//! JavaScript via `wasm-bindgen`. Stateful session management
//! (SessionStore, ratchet persistence) lives in the JS side: call
//! `session_send` / `session_receive` with serialised `SessionStore`
//! bytes, store the returned new-state bytes back into IndexedDB.
//!
//! ## Build
//!
//! ```bash
//! RUSTFLAGS='--cfg getrandom_backend="wasm_js"' \
//!   cargo build --target wasm32-unknown-unknown \
//!                --no-default-features --features wasm --release
//!
//! wasm-bindgen --target web --out-dir ./pkg \
//!   target/wasm32-unknown-unknown/release/phantomchat_core.wasm
//! ```
//!
//! ## JS API surface
//!
//! - [`wasm_generate_address`] — one-shot keygen, returns JSON identity
//! - [`wasm_safety_number`]    — compute 60-digit fingerprint
//! - [`wasm_address_parse_ok`] — validate a `phantom:` / `phantomx:` string
//! - [`wasm_prekey_bundle_verify`] — verify a JSON-serialised PrekeyBundle
//! - [`wasm_pack_onion`] / [`wasm_peel_onion`] — mixnet helpers

use wasm_bindgen::prelude::*;

use crate::{
    address::PhantomAddress,
    fingerprint::safety_number,
    keys::{PhantomSigningKey, SpendKey, ViewKey},
    mixnet::{pack_onion, peel_onion, MixnetHop, MixnetPacket, Peeled},
    prekey::PrekeyBundle,
};
use x25519_dalek::StaticSecret;

#[derive(serde::Serialize, serde::Deserialize)]
pub struct WasmIdentity {
    pub view_private: String,
    pub view_public: String,
    pub spend_private: String,
    pub spend_public: String,
    pub signing_private: String,
    pub signing_public: String,
    pub address: String,
}

/// Generate a fresh PhantomChat identity + signing key. The returned JSON
/// blob is the seed the JS side stores in IndexedDB (or hands to the
/// user for manual backup).
#[wasm_bindgen]
pub fn wasm_generate_address() -> Result<JsValue, JsValue> {
    let view = ViewKey::generate();
    let spend = SpendKey::generate();
    let signing = PhantomSigningKey::generate();
    let addr = PhantomAddress::new(view.public, spend.public);

    let id = WasmIdentity {
        view_private: hex::encode(view.secret.to_bytes()),
        view_public: hex::encode(view.public.as_bytes()),
        spend_private: hex::encode(spend.secret.to_bytes()),
        spend_public: hex::encode(spend.public.as_bytes()),
        signing_private: hex::encode(signing.to_bytes()),
        signing_public: hex::encode(signing.public_bytes()),
        address: addr.to_string(),
    };
    serde_wasm_bindgen::to_value(&id).map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Compute the 60-digit Signal-style safety number for two addresses.
/// Either party can call this; the output is symmetric.
#[wasm_bindgen]
pub fn wasm_safety_number(addr_a: &str, addr_b: &str) -> Result<String, JsValue> {
    let a = PhantomAddress::parse(addr_a).ok_or_else(|| JsValue::from_str("bad addr_a"))?;
    let b = PhantomAddress::parse(addr_b).ok_or_else(|| JsValue::from_str("bad addr_b"))?;
    Ok(safety_number(&a, &b))
}

/// `true` if the input parses as a valid `phantom:` or `phantomx:` address.
#[wasm_bindgen]
pub fn wasm_address_parse_ok(s: &str) -> bool {
    PhantomAddress::parse(s).is_some()
}

/// Verify a PrekeyBundle serialised as JSON. Used by the JS fetch path
/// before doing X3DH against the bundle.
#[wasm_bindgen]
pub fn wasm_prekey_bundle_verify(bundle_json: &str) -> bool {
    match serde_json::from_str::<PrekeyBundle>(bundle_json) {
        Ok(b) => b.verify(),
        Err(_) => false,
    }
}

/// Pack an onion packet for a 1-to-N hop route. `hops_hex` is a
/// newline-separated list of 64-char X25519 public keys. Returns the
/// serialised packet bytes.
#[wasm_bindgen]
pub fn wasm_pack_onion(hops_hex: &str, payload: &[u8]) -> Result<Vec<u8>, JsValue> {
    let mut hops = Vec::new();
    for line in hops_hex.split('\n').filter(|s| !s.trim().is_empty()) {
        let bytes: [u8; 32] = hex::decode(line.trim())
            .map_err(|e| JsValue::from_str(&format!("hex: {}", e)))?
            .try_into()
            .map_err(|_| JsValue::from_str("hop pub not 32 bytes"))?;
        hops.push(MixnetHop::from_bytes(bytes));
    }
    if hops.is_empty() {
        return Err(JsValue::from_str("at least one hop required"));
    }
    let packet = pack_onion(&hops, payload);
    Ok(packet.to_bytes())
}

/// Peel one mixnet layer with `my_secret_hex`. Returns a JSON object:
/// `{ forward: { next_hop, packet } }` or `{ final: payload }`.
#[wasm_bindgen]
pub fn wasm_peel_onion(packet_bytes: &[u8], my_secret_hex: &str) -> Result<JsValue, JsValue> {
    let packet = MixnetPacket::from_bytes(packet_bytes)
        .ok_or_else(|| JsValue::from_str("bad packet bytes"))?;
    let sec_bytes: [u8; 32] = hex::decode(my_secret_hex)
        .map_err(|_| JsValue::from_str("bad secret hex"))?
        .try_into()
        .map_err(|_| JsValue::from_str("secret not 32 bytes"))?;
    let secret = StaticSecret::from(sec_bytes);

    match peel_onion(&packet, &secret) {
        Ok(Peeled::Final { payload }) => {
            let obj = serde_json::json!({ "final": hex::encode(&payload) });
            serde_wasm_bindgen::to_value(&obj).map_err(|e| JsValue::from_str(&e.to_string()))
        }
        Ok(Peeled::Forward { next_hop, packet }) => {
            let obj = serde_json::json!({
                "forward": {
                    "next_hop": hex::encode(next_hop.as_bytes()),
                    "packet": hex::encode(packet.to_bytes()),
                }
            });
            serde_wasm_bindgen::to_value(&obj).map_err(|e| JsValue::from_str(&e.to_string()))
        }
        Err(e) => Err(JsValue::from_str(&e.to_string())),
    }
}
