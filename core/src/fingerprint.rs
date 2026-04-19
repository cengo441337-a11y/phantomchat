//! Safety numbers — Signal-style session fingerprint for out-of-band MITM
//! detection.
//!
//! Two contacts compute the same 60-digit decimal number from their
//! canonicalised address pair. They compare the number in person, on the
//! phone, or via QR. A mismatch means an active-in-the-middle attacker is
//! sitting on the relay stream.
//!
//! ## Derivation
//!
//! ```text
//! canonical = min(addr_a, addr_b) || max(addr_a, addr_b)
//! hash      = iterate 5 200 times: SHA-512(hash || version || canonical)
//! safety    = encode first 30 bytes of hash as twelve 5-digit groups
//! ```
//!
//! - The ordered concatenation makes the output symmetric.
//! - The iterated SHA-512 is the same KDF hardening Signal uses in
//!   `NumericFingerprintGenerator` (5 200 rounds ≈ 5 ms on a phone).
//! - Twelve 5-digit groups (= 60 decimal digits) is the Signal canonical
//!   format and fits nicely in a spoken-aloud verification.

use sha2::{Digest, Sha512};

use crate::address::PhantomAddress;

/// Version byte mixed into every round. Bump if the derivation changes in
/// an incompatible way.
const VERSION: u8 = 0x01;
/// Iteration count — Signal uses 5 200.
const ITERATIONS: usize = 5_200;
/// Number of 5-digit groups rendered. 12 × 5 = 60 decimal digits.
const GROUPS: usize = 12;
/// Digits per group.
const DIGITS_PER_GROUP: usize = 5;

/// Compute the shared 60-digit safety number for a pair of PhantomAddresses.
/// Argument order does not matter — the output is symmetric.
///
/// 12 groups × 5 digits = 60 decimal digits, consuming 60 bytes of the
/// 64-byte iterated SHA-512 digest (exactly the same arithmetic Signal's
/// `NumericFingerprintGenerator` uses).
pub fn safety_number(a: &PhantomAddress, b: &PhantomAddress) -> String {
    let hash = derive_hash(a, b);
    encode_numeric(&hash[..GROUPS * 5])
}

fn address_bytes(addr: &PhantomAddress) -> Vec<u8> {
    let mut out = Vec::with_capacity(32 + 32 + addr.mlkem_pub.as_ref().map_or(0, |m| m.len()));
    out.extend_from_slice(&addr.view_pub);
    out.extend_from_slice(&addr.spend_pub);
    if let Some(mlkem) = &addr.mlkem_pub {
        out.extend_from_slice(mlkem);
    }
    out
}

fn derive_hash(a: &PhantomAddress, b: &PhantomAddress) -> [u8; 64] {
    let a_bytes = address_bytes(a);
    let b_bytes = address_bytes(b);
    let (first, second) = if a_bytes <= b_bytes {
        (a_bytes, b_bytes)
    } else {
        (b_bytes, a_bytes)
    };

    let mut canonical = Vec::with_capacity(first.len() + second.len());
    canonical.extend_from_slice(&first);
    canonical.extend_from_slice(&second);

    let mut hash = [0u8; 64];
    let mut current: Vec<u8> = Vec::new(); // first round starts empty
    for _ in 0..ITERATIONS {
        let mut h = Sha512::new();
        h.update([VERSION]);
        h.update(&current);
        h.update(&canonical);
        let digest = h.finalize();
        hash.copy_from_slice(&digest);
        current.clear();
        current.extend_from_slice(&digest);
    }
    hash
}

fn encode_numeric(bytes: &[u8]) -> String {
    // Pull 5 bytes at a time → mod 100000 → 5-digit group.
    let mut groups = Vec::with_capacity(GROUPS);
    for chunk in bytes.chunks(5).take(GROUPS) {
        let mut v: u64 = 0;
        for &b in chunk {
            v = (v << 8) | (b as u64);
        }
        let modulus = 10u64.pow(DIGITS_PER_GROUP as u32);
        groups.push(format!("{:0>5}", v % modulus));
    }
    groups.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::{SpendKey, ViewKey};

    fn random_address() -> PhantomAddress {
        PhantomAddress::new(ViewKey::generate().public, SpendKey::generate().public)
    }

    #[test]
    fn fingerprint_is_symmetric() {
        let a = random_address();
        let b = random_address();
        assert_eq!(safety_number(&a, &b), safety_number(&b, &a));
    }

    #[test]
    fn fingerprint_has_correct_shape() {
        let a = random_address();
        let b = random_address();
        let n = safety_number(&a, &b);
        let groups: Vec<&str> = n.split(' ').collect();
        assert_eq!(groups.len(), GROUPS);
        for g in groups {
            assert_eq!(g.len(), DIGITS_PER_GROUP);
            assert!(g.chars().all(|c| c.is_ascii_digit()));
        }
    }

    #[test]
    fn fingerprint_changes_with_different_counterparty() {
        let a = random_address();
        let b = random_address();
        let c = random_address();
        assert_ne!(safety_number(&a, &b), safety_number(&a, &c));
    }
}
