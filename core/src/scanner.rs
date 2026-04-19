//! Stealth envelope scanner — the Monero model for messaging.
//!
//! ## How it works
//! 1. All envelopes on a relay are opaque blobs. Nobody knows who they're for.
//! 2. A client downloads ALL envelopes and runs [`scan_envelope`] on each.
//! 3. The scanner uses the ViewKey to attempt HMAC tag verification.
//! 4. If the tag matches → the envelope is addressed to this identity.
//! 5. Only then is [`Envelope::open`] called with the SpendKey to decrypt.
//!
//! ## Privacy guarantee
//! The relay never learns who receives a message. An attacker observing the
//! relay sees only random blobs. Even traffic analysis yields nothing because
//! every client downloads everything. This is fundamentally different from
//! Signal/Telegram/Threema where the server routes to a specific recipient.
//!
//! ## Cryptographic construction
//! Matches [`Envelope::new`] exactly:
//! ```text
//! view_shared = view_secret × epk
//! tag_key     = HKDF-SHA256(view_shared, info = "PhantomChat-v1-ViewTag", 32 bytes)
//! expected    = HMAC-SHA256(tag_key, epk)
//! ```
//! Then constant-time compare against `env.tag`. Tag match → `Envelope::open`
//! re-derives the encryption key from `spend_shared`.
//!
//! ## Performance
//! ViewKey scanning is fast (one ECDH + one HMAC per envelope). For 10k
//! envelopes: ~50 ms on a mid-range phone.

use crate::envelope::{Envelope, Payload, TAG_HKDF_INFO};
use crate::keys::{ViewKey, SpendKey};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use x25519_dalek::PublicKey;

type HmacSha256 = Hmac<Sha256>;

/// Result of scanning a single envelope.
#[derive(Debug)]
pub enum ScanResult {
    /// Envelope is addressed to this identity — payload decrypted.
    Mine(Payload),
    /// Envelope is not for this identity.
    NotMine,
    /// Tag matched but decryption failed (corrupted, replay, or
    /// ViewKey/SpendKey mismatch on the identity).
    Corrupted,
}

/// Phase-1 ViewKey scan only — returns `true` if the stealth tag matches,
/// i.e. the envelope is addressed to this identity. Does **not** attempt
/// decryption (callers pick classic vs hybrid open themselves).
pub fn scan_envelope_tag_ok(env: &Envelope, view_key: &ViewKey) -> bool {
    let epk = PublicKey::from(env.epk);
    let view_shared = view_key.secret.diffie_hellman(&epk);

    let hk = Hkdf::<Sha256>::new(None, view_shared.as_bytes());
    let mut tag_key = [0u8; 32];
    if hk.expand(TAG_HKDF_INFO, &mut tag_key).is_err() {
        return false;
    }

    let mut hmac = match HmacSha256::new_from_slice(&tag_key) {
        Ok(h) => h,
        Err(_) => return false,
    };
    hmac.update(&env.epk);
    let expected_tag: [u8; 32] = hmac.finalize().into_bytes().into();

    constant_time_eq(&expected_tag, &env.tag)
}

/// Scans a single envelope with the given ViewKey.
///
/// Phase 1 (fast): ECDH(view_secret, epk) → derive tag_key → verify HMAC tag.
/// Phase 2 (only if tag matches): open with SpendKey → decrypt payload.
///
/// Only decrypts **classic** (v1) envelopes — a hybrid (v2) envelope whose
/// tag matches will come back as `Corrupted` here because the classic
/// `Envelope::open` refuses v2. Use [`crate::session::SessionStore::receive`]
/// for the full hybrid-aware path.
pub fn scan_envelope(
    env: &Envelope,
    view_key: &ViewKey,
    spend_key: &SpendKey,
) -> ScanResult {
    if !scan_envelope_tag_ok(env, view_key) {
        return ScanResult::NotMine;
    }
    match env.open(spend_key) {
        Some(payload) => ScanResult::Mine(payload),
        None => ScanResult::Corrupted,
    }
}

/// Batch-scan a slice of envelopes. Returns only matched payloads.
/// Processes envelopes sequentially; caller may parallelise with rayon.
pub fn scan_batch(
    envelopes: &[Envelope],
    view_key: &ViewKey,
    spend_key: &SpendKey,
) -> Vec<Payload> {
    envelopes
        .iter()
        .filter_map(|env| {
            match scan_envelope(env, view_key, spend_key) {
                ScanResult::Mine(payload) => Some(payload),
                _ => None,
            }
        })
        .collect()
}

/// PoW verification before scanning — cheap DoS filter.
/// Call this before [`scan_envelope`] to reject spam without ECDH cost.
pub fn verify_pow(env: &Envelope, min_difficulty: u32) -> bool {
    use crate::pow::Hashcash;

    let mut pow_header = Vec::with_capacity(40);
    pow_header.extend_from_slice(&env.tag);
    pow_header.extend_from_slice(&env.ts.to_le_bytes());
    Hashcash::new(min_difficulty).verify(&pow_header, env.pow_nonce)
}

/// Constant-time byte slice comparison to prevent timing attacks.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() { return false; }
    a.iter().zip(b.iter()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_eq_works() {
        assert!(constant_time_eq(b"hello", b"hello"));
        assert!(!constant_time_eq(b"hello", b"hellx"));
        assert!(!constant_time_eq(b"hi", b"hello"));
    }
}
