//! Tests for the key hierarchy and PQXDH (post-quantum) handshake.
//!
//! These tests exercise the single most security-critical invariant in the
//! codebase: **sender and receiver must derive identical 32-byte session
//! secrets** after the hybrid X25519 + ML-KEM-1024 handshake.

use phantomchat_core::{HybridKeyPair, HybridPublicKey, IdentityKey, SpendKey, ViewKey};

#[test]
fn hybrid_pqxdh_roundtrip_produces_identical_session_keys() {
    let bob = HybridKeyPair::generate();
    let sent = bob.public.encapsulate();

    let received = bob
        .decapsulate(&sent.epk_x25519, &sent.mlkem_ct)
        .expect("decapsulation failed");

    assert_eq!(
        sent.session_key, received,
        "sender and receiver must agree on the same 32-byte session key"
    );
    assert_eq!(sent.session_key.len(), 32);
}

#[test]
fn two_hybrid_pairs_produce_different_session_keys() {
    // Two independent senders encapsulating to the same recipient must land
    // on different session keys — that's the whole point of a fresh ephemeral
    // per message.
    let bob = HybridKeyPair::generate();
    let a = bob.public.encapsulate();
    let b = bob.public.encapsulate();
    assert_ne!(
        a.session_key, b.session_key,
        "two independent encapsulations must never collide"
    );
}

#[test]
fn hybrid_public_key_wire_roundtrips() {
    let kp = HybridKeyPair::generate();
    let bytes = kp.public.to_bytes();
    assert_eq!(bytes.len(), 1600, "expected [32B X25519 || 1568B ML-KEM] = 1600 B");

    let restored = HybridPublicKey::from_bytes(&bytes)
        .expect("from_bytes must accept its own to_bytes output");

    // Round-trip one more encapsulation to prove the restored key is functional
    // (we can't simply compare the key material because PublicKey/MlKemPublicKey
    // don't derive PartialEq).
    let enc = restored.encapsulate();
    let dec = kp
        .decapsulate(&enc.epk_x25519, &enc.mlkem_ct)
        .expect("restored key must still decapsulate");
    assert_eq!(enc.session_key, dec);
}

#[test]
fn hybrid_public_key_rejects_short_input() {
    assert!(HybridPublicKey::from_bytes(&[]).is_none());
    assert!(HybridPublicKey::from_bytes(&[0u8; 1599]).is_none());
}

#[test]
fn view_spend_keys_are_distinct_by_construction() {
    // ViewKey and SpendKey must be independently random. If they shared
    // state we'd collapse the whole two-key stealth model.
    let v = ViewKey::generate();
    let s = SpendKey::generate();
    assert_ne!(v.public.as_bytes(), s.public.as_bytes());
    assert_ne!(v.secret.to_bytes(), s.secret.to_bytes());
}

#[test]
fn identity_keys_are_32_bytes() {
    let id = IdentityKey::generate();
    assert_eq!(id.public.len(), 32);
    assert_eq!(id.private.len(), 32);
    // And successive calls produce fresh material.
    let id2 = IdentityKey::generate();
    assert_ne!(id.private, id2.private);
}

#[test]
fn view_key_ecdh_is_symmetric_with_sender() {
    // ECDH(view_secret, spend_pub) must equal ECDH(spend_secret, view_pub).
    // This is the underlying mathematical property the Monero stealth model
    // leans on for the tag derivation.
    let alice = ViewKey::generate();
    let bob   = SpendKey::generate();
    let left  = alice.ecdh(&bob);
    let right = bob.ecdh(&alice.public);
    assert_eq!(left, right, "X25519 ECDH must be commutative");
}
