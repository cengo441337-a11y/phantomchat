//! Integration tests for the Envelope encrypt/decrypt flow.
//!
//! These tests pin down the Monero-stealth-address construction:
//! encrypt with `(view_pub, spend_pub)` → scan with `view_key` → open with
//! `spend_key`. Any regression that splits these three paths apart (as an
//! earlier version of the code did) will fail here.

use phantomchat_core::{
    envelope::Envelope,
    keys::{SpendKey, ViewKey},
    scan_envelope, ScanResult,
};

/// End-to-end: A sends to B. B's scanner must find the envelope and decrypt.
#[test]
fn envelope_roundtrip_succeeds_with_matching_keys() {
    let alice_view  = ViewKey::generate();
    let alice_spend = SpendKey::generate();

    let msg_id: u128 = 0xDEAD_BEEF_CAFE_BABE_1234_5678_9ABC_DEF0;
    let body = b"the ghost walks at midnight".to_vec();

    let env = Envelope::new(
        &alice_view.public,
        &alice_spend.public,
        msg_id,
        Vec::new(),
        body.clone(),
        300,
        0, // PoW difficulty 0 keeps the test fast; PoW itself is tested separately
    );

    let result = scan_envelope(&env, &alice_view, &alice_spend);
    match result {
        ScanResult::Mine(payload) => {
            assert_eq!(payload.msg_id, msg_id, "msg_id must round-trip intact");
            assert_eq!(payload.encrypted_body, body, "body must round-trip intact");
        }
        ScanResult::NotMine => panic!("scanner failed to recognise our own envelope"),
        ScanResult::Corrupted => panic!("decryption failed on our own envelope"),
    }
}

/// A third party's ViewKey must not match Alice's envelope.
#[test]
fn envelope_rejected_by_foreign_view_key() {
    let alice_view  = ViewKey::generate();
    let alice_spend = SpendKey::generate();
    let eve_view    = ViewKey::generate();
    let eve_spend   = SpendKey::generate();

    let env = Envelope::new(
        &alice_view.public,
        &alice_spend.public,
        42,
        Vec::new(),
        b"not for eve".to_vec(),
        300,
        0,
    );

    let result = scan_envelope(&env, &eve_view, &eve_spend);
    assert!(
        matches!(result, ScanResult::NotMine),
        "Eve's scan must yield NotMine (got {:?})",
        result
    );
}

/// Even if the attacker happens to know Alice's SpendKey (stolen device) but
/// NOT her ViewKey, the scanner still discards the envelope — it cannot match
/// the stealth tag without view_shared. Positive validation of the two-key
/// split.
#[test]
fn scanner_uses_view_key_for_tag_not_spend_key() {
    let alice_view  = ViewKey::generate();
    let alice_spend = SpendKey::generate();
    let fake_view   = ViewKey::generate();

    let env = Envelope::new(
        &alice_view.public,
        &alice_spend.public,
        1,
        Vec::new(),
        b"payload".to_vec(),
        300,
        0,
    );

    // Wrong ViewKey + correct SpendKey ⇒ tag must NOT match.
    let result = scan_envelope(&env, &fake_view, &alice_spend);
    assert!(
        matches!(result, ScanResult::NotMine),
        "wrong ViewKey must reject even when SpendKey matches"
    );
}

/// Tag matches but the wrong SpendKey fails AEAD decrypt ⇒ Corrupted.
/// This is the "identity split" case — should never happen in practice.
#[test]
fn mismatched_spend_key_yields_corrupted() {
    let alice_view  = ViewKey::generate();
    let alice_spend = SpendKey::generate();
    let other_spend = SpendKey::generate();

    let env = Envelope::new(
        &alice_view.public,
        &alice_spend.public,
        7,
        Vec::new(),
        b"body".to_vec(),
        300,
        0,
    );

    let result = scan_envelope(&env, &alice_view, &other_spend);
    assert!(
        matches!(result, ScanResult::Corrupted),
        "tag match + wrong SpendKey must yield Corrupted (got {:?})",
        result
    );
}

/// Wire format roundtrip: serialize + deserialize must be bit-exact.
#[test]
fn wire_format_roundtrips() {
    let view  = ViewKey::generate();
    let spend = SpendKey::generate();
    let env = Envelope::new(
        &view.public,
        &spend.public,
        999,
        b"ratchet_header".to_vec(),
        b"hello".to_vec(),
        600,
        0,
    );

    let wire = env.to_bytes();
    let restored = Envelope::from_bytes(&wire).expect("deserialisation failed");

    assert_eq!(env.ver, restored.ver);
    assert_eq!(env.ts, restored.ts);
    assert_eq!(env.ttl, restored.ttl);
    assert_eq!(env.epk, restored.epk);
    assert_eq!(env.tag, restored.tag);
    assert_eq!(env.pow_nonce, restored.pow_nonce);
    assert_eq!(env.nonce, restored.nonce);
    assert_eq!(env.ciphertext, restored.ciphertext);

    // And after deserialising, the scanner still accepts it.
    let result = scan_envelope(&restored, &view, &spend);
    assert!(matches!(result, ScanResult::Mine(_)));
}

/// Truncated buffers must fail gracefully, not panic.
#[test]
fn truncated_wire_bytes_fail_cleanly() {
    let view  = ViewKey::generate();
    let spend = SpendKey::generate();
    let env = Envelope::new(&view.public, &spend.public, 0, vec![], vec![], 60, 0);
    let wire = env.to_bytes();

    // Every single shorter prefix must parse as None, never panic.
    for cut in 0..wire.len() {
        let _ = Envelope::from_bytes(&wire[..cut]);
    }
}

/// Mutating the stealth tag must invalidate AEAD decryption (AAD binding).
/// A man-in-the-middle who swaps tags between envelopes should fail to open.
#[test]
fn tag_tampering_breaks_decryption() {
    let view  = ViewKey::generate();
    let spend = SpendKey::generate();

    let mut env = Envelope::new(
        &view.public,
        &spend.public,
        0,
        vec![],
        b"data".to_vec(),
        60,
        0,
    );
    env.tag[0] ^= 0xFF;

    assert!(
        env.open(&spend).is_none(),
        "tampering with tag must cause AEAD decryption to fail"
    );
}

/// Mutating the ciphertext must also break decryption.
#[test]
fn ciphertext_tampering_breaks_decryption() {
    let view  = ViewKey::generate();
    let spend = SpendKey::generate();

    let mut env = Envelope::new(
        &view.public,
        &spend.public,
        0,
        vec![],
        b"data".to_vec(),
        60,
        0,
    );
    if !env.ciphertext.is_empty() {
        env.ciphertext[0] ^= 0xFF;
    }

    assert!(env.open(&spend).is_none());
}

/// A dummy cover-traffic envelope must be wire-valid but scanner-rejected.
/// This is the entire premise of cover traffic: indistinguishable on the wire,
/// silently discarded on receipt.
#[test]
fn dummy_envelope_is_wire_valid_but_not_mine() {
    let view  = ViewKey::generate();
    let spend = SpendKey::generate();

    let dummy = Envelope::dummy().expect("dummy generation failed");
    let wire = dummy.to_bytes();
    let restored = Envelope::from_bytes(&wire).expect("dummy must be wire-valid");

    // Random bytes in the tag slot → astronomically unlikely to match a real
    // HMAC for our ViewKey. NotMine is the expected outcome.
    let result = scan_envelope(&restored, &view, &spend);
    assert!(
        matches!(result, ScanResult::NotMine),
        "dummy envelope must be rejected by scanner (got {:?})",
        result
    );
}

/// Two dummies from the same call must differ in every cryptographic field.
/// If any field is reused, an observer could fingerprint dummies.
#[test]
fn two_dummies_differ() {
    let a = Envelope::dummy().unwrap();
    let b = Envelope::dummy().unwrap();
    assert_ne!(a.epk, b.epk, "epk must be random per dummy");
    assert_ne!(a.tag, b.tag, "tag must be random per dummy");
    assert_ne!(a.nonce, b.nonce, "nonce must be random per dummy");
}
