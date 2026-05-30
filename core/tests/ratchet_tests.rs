//! Double-Ratchet correctness tests.
//!
//! The Signal-style ratchet depends on ECDH commutativity between the
//! sender's bootstrap DH (`ratchet_secret × recipient_spend_pub`) and the
//! receiver's bootstrap DH (`recipient_spend_secret × sender_ratchet_pub`).
//! These tests pin that symmetry down and exercise multi-round DH ratchet
//! rotations.

use phantomchat_core::{
    keys::SpendKey,
    ratchet::RatchetState,
};
use x25519_dalek::PublicKey;

fn shared() -> [u8; 32] {
    // For these tests we just need *some* agreed-upon 32-byte initial
    // secret. In the real flow `SessionStore` derives it from HKDF over
    // the recipient spend-pub.
    *b"phantomchat-ratchet-test-1234567"
}

#[test]
fn sender_to_receiver_first_message_decrypts() {
    let bob = SpendKey::generate();
    let mut alice = RatchetState::initialize_as_sender(shared(), bob.public);

    let plaintext = b"hello bob - first stealth message";
    let (header, ciphertext) = alice.encrypt(plaintext);

    // Bob reconstructs the matching state from his own spend_secret + the
    // peer_ratchet_pub that came with the header.
    let peer_ratchet: [u8; 32] = header[0..32].try_into().unwrap();
    let peer_ratchet_pub = PublicKey::from(peer_ratchet);

    let mut bob_state =
        RatchetState::initialize_as_receiver(shared(), &bob.secret, peer_ratchet_pub);

    let decoded = bob_state.decrypt(&header, &ciphertext).expect("decrypt");
    assert_eq!(decoded, plaintext);
}

#[test]
fn multiple_messages_in_the_same_chain_decrypt() {
    let bob = SpendKey::generate();
    let mut alice = RatchetState::initialize_as_sender(shared(), bob.public);

    // First message bootstraps Bob.
    let (h0, c0) = alice.encrypt(b"msg 0");
    let peer_ratchet = PublicKey::from(<[u8; 32]>::try_from(&h0[0..32]).unwrap());
    let mut bob = RatchetState::initialize_as_receiver(shared(), &spend_secret(&bob), peer_ratchet);
    assert_eq!(bob.decrypt(&h0, &c0).unwrap(), b"msg 0");

    // Three more messages on Alice's same send-chain (no rotation yet).
    for i in 1..=3 {
        let plain = format!("msg {}", i);
        let (h, c) = alice.encrypt(plain.as_bytes());
        assert_eq!(bob.decrypt(&h, &c).unwrap(), plain.as_bytes());
    }
}

#[test]
fn bidirectional_ratchet_exchange_rotates_keys() {
    let bob = SpendKey::generate();
    let mut alice = RatchetState::initialize_as_sender(shared(), bob.public);

    // A → B — bootstraps Bob's state.
    let (h_a0, c_a0) = alice.encrypt(b"A0");
    let peer_ratchet = PublicKey::from(<[u8; 32]>::try_from(&h_a0[0..32]).unwrap());
    let mut bob_state =
        RatchetState::initialize_as_receiver(shared(), &spend_secret(&bob), peer_ratchet);
    assert_eq!(bob_state.decrypt(&h_a0, &c_a0).unwrap(), b"A0");

    // B → A — Bob replies. Triggers the first DH ratchet step on Alice.
    let (h_b0, c_b0) = bob_state.encrypt(b"B0");
    assert_eq!(alice.decrypt(&h_b0, &c_b0).unwrap(), b"B0");

    // A → B — Alice sends again. She's rotated, so Bob must rotate too.
    let (h_a1, c_a1) = alice.encrypt(b"A1");
    assert_eq!(bob_state.decrypt(&h_a1, &c_a1).unwrap(), b"A1");

    // And back once more, confirming the send-chain is still healthy.
    let (h_b1, c_b1) = bob_state.encrypt(b"B1");
    assert_eq!(alice.decrypt(&h_b1, &c_b1).unwrap(), b"B1");
}

#[test]
fn ratchet_state_round_trips_through_serde() {
    // Alice and Bob exchange two messages, Alice's state is persisted and
    // re-hydrated, and a third message still decrypts — proves the chain
    // keys, counter, and DH secret all survive the JSON round trip.
    let bob = SpendKey::generate();
    let mut alice = RatchetState::initialize_as_sender(shared(), bob.public);

    let (h0, c0) = alice.encrypt(b"msg-0");
    let peer_ratchet = PublicKey::from(<[u8; 32]>::try_from(&h0[0..32]).unwrap());
    let mut bob_state =
        RatchetState::initialize_as_receiver(shared(), &spend_secret(&bob), peer_ratchet);
    assert_eq!(bob_state.decrypt(&h0, &c0).unwrap(), b"msg-0");

    let (h1, c1) = alice.encrypt(b"msg-1");
    assert_eq!(bob_state.decrypt(&h1, &c1).unwrap(), b"msg-1");

    // Serialize Alice, drop her, rebuild her from disk.
    let serialized = serde_json::to_string(&alice).unwrap();
    drop(alice);
    let mut restored: RatchetState = serde_json::from_str(&serialized).unwrap();
    restored.restore_secret();

    // The restored Alice sends msg-2; Bob's in-memory chain must still
    // advance correctly.
    let (h2, c2) = restored.encrypt(b"msg-2");
    assert_eq!(bob_state.decrypt(&h2, &c2).unwrap(), b"msg-2");
}

#[test]
fn tampered_ciphertext_fails_to_decrypt() {
    let bob = SpendKey::generate();
    let mut alice = RatchetState::initialize_as_sender(shared(), bob.public);

    let (h, mut c) = alice.encrypt(b"confidential");
    c[0] ^= 0xFF;

    let peer_ratchet = PublicKey::from(<[u8; 32]>::try_from(&h[0..32]).unwrap());
    let mut bob_state =
        RatchetState::initialize_as_receiver(shared(), &spend_secret(&bob), peer_ratchet);
    assert!(bob_state.decrypt(&h, &c).is_err());
}

// ─── Audit 2026-05-30 (R-1/R-2/R-3) regression tests ─────────────────────────
//
// Pre-rewrite, decrypt mutated `recv_chain` / `recv_count` *before* the AEAD
// validated. A duplicate, an out-of-order envelope, or an attacker-crafted
// envelope with a random peer_ratchet_pub was enough to permanently desync
// the session. The transactional decrypt closed all three.

use phantomchat_core::ratchet::{RatchetError, MAX_SKIP};

fn handshake() -> (RatchetState, RatchetState, SpendKey) {
    let bob = SpendKey::generate();
    let mut alice = RatchetState::initialize_as_sender(shared(), bob.public);
    let (h0, c0) = alice.encrypt(b"bootstrap");
    let peer = PublicKey::from(<[u8; 32]>::try_from(&h0[0..32]).unwrap());
    let mut bob_state =
        RatchetState::initialize_as_receiver(shared(), &spend_secret(&bob), peer);
    assert_eq!(bob_state.decrypt(&h0, &c0).unwrap(), b"bootstrap");
    (alice, bob_state, bob)
}

#[test]
fn replay_of_decrypted_envelope_is_rejected_without_breaking_session() {
    let (mut alice, mut bob, _) = handshake();
    let (h1, c1) = alice.encrypt(b"msg-1");
    assert_eq!(bob.decrypt(&h1, &c1).unwrap(), b"msg-1");

    // Replay the same envelope — must be rejected as a replay.
    let err = bob.decrypt(&h1, &c1).unwrap_err();
    assert_eq!(err, RatchetError::Replay, "expected Replay, got {err:?}");

    // Session must still work — Alice sends msg-2 and Bob decrypts.
    let (h2, c2) = alice.encrypt(b"msg-2");
    assert_eq!(bob.decrypt(&h2, &c2).unwrap(), b"msg-2");
}

#[test]
fn first_message_counter_is_one_indexed_on_the_wire() {
    // Audit 2026-05-30 (R-5): the on-wire counter MUST be 1-indexed (first
    // message = 1, second = 2 …). Every shipped client's receiver relies on
    // this; a 0-indexed counter silently broke cross-version decryption — an
    // updated phone could not read a not-yet-updated desktop's messages
    // while the reverse direction (old receiver ignores the counter) still
    // worked, producing a one-way-chat bug. This pins the wire format.
    let bob = SpendKey::generate();
    let mut alice = RatchetState::initialize_as_sender(shared(), bob.public);
    let (h1, _c1) = alice.encrypt(b"first");
    assert_eq!(
        u32::from_le_bytes(h1[32..36].try_into().unwrap()),
        1,
        "first message must be counter 1 (1-indexed wire format)"
    );
    let (h2, _c2) = alice.encrypt(b"second");
    assert_eq!(
        u32::from_le_bytes(h2[32..36].try_into().unwrap()),
        2,
        "second message must be counter 2"
    );
}

#[test]
fn out_of_order_arrival_is_tolerated_via_skipped_keys_cache() {
    let (mut alice, mut bob, _) = handshake();

    // Alice sends 1, 2, 3 in order.
    let (h1, c1) = alice.encrypt(b"a-1");
    let (h2, c2) = alice.encrypt(b"a-2");
    let (h3, c3) = alice.encrypt(b"a-3");

    // Bob receives 3 first → 1 and 2 are stored as skipped keys.
    assert_eq!(bob.decrypt(&h3, &c3).unwrap(), b"a-3");

    // Then receives 1 (cached key path).
    assert_eq!(bob.decrypt(&h1, &c1).unwrap(), b"a-1");

    // Then 2 — also cached.
    assert_eq!(bob.decrypt(&h2, &c2).unwrap(), b"a-2");

    // Cache is now empty: re-delivering 1 must register as a replay.
    let err = bob.decrypt(&h1, &c1).unwrap_err();
    assert_eq!(err, RatchetError::Replay);

    // Forward progress still works.
    let (h4, c4) = alice.encrypt(b"a-4");
    assert_eq!(bob.decrypt(&h4, &c4).unwrap(), b"a-4");
}

#[test]
fn attacker_forged_peer_ratchet_pub_does_not_desync_session() {
    let (mut alice, mut bob, _) = handshake();

    // Snapshot Bob's view: a valid msg-1 from Alice that we'll feed AFTER
    // the attacker attempt to prove the live state survived.
    let (h1, c1) = alice.encrypt(b"a-1");

    // Forge an envelope: random 32-byte "peer_ratchet_pub", arbitrary
    // counter / nonce, garbage ciphertext. Pre-audit this triggered a
    // dh_ratchet step on the live recv chain.
    let mut forged_header = vec![0u8; 60];
    forged_header[0..32].copy_from_slice(&[0xAB; 32]); // attacker-controlled peer pub
    forged_header[32..36].copy_from_slice(&1u32.to_le_bytes()); // valid 1-indexed counter
    forged_header[36..60].copy_from_slice(&[0xCD; 24]);
    let forged_ct = vec![0xEF; 80];
    let err = bob.decrypt(&forged_header, &forged_ct).unwrap_err();
    assert_eq!(err, RatchetError::DecryptionFailed);

    // The live state must not have ratcheted: legitimate Alice envelope
    // still decrypts.
    assert_eq!(bob.decrypt(&h1, &c1).unwrap(), b"a-1");

    // And so does the next legit one.
    let (h2, c2) = alice.encrypt(b"a-2");
    assert_eq!(bob.decrypt(&h2, &c2).unwrap(), b"a-2");
}

#[test]
fn counter_too_far_ahead_is_capped_at_max_skip() {
    let (mut alice, mut bob, _) = handshake();

    // Snapshot a real msg-1 to prove later that bob's chain didn't advance.
    let (h1, c1) = alice.encrypt(b"a-1");

    // Forge an envelope on the SAME chain (Alice's current peer_ratchet_pub)
    // but with a counter well past MAX_SKIP. Without the cap the receiver
    // would HMAC-loop hundreds of thousands of times. With the cap this is
    // rejected before any keys are derived.
    let mut header_far = h1.clone();
    let huge = 100_000u32;
    header_far[32..36].copy_from_slice(&huge.to_le_bytes());
    let garbage = vec![0u8; 80];
    let err = bob.decrypt(&header_far, &garbage).unwrap_err();
    match err {
        RatchetError::TooMuchSkip(n) => {
            assert!(n > MAX_SKIP, "skip count {n} should exceed MAX_SKIP={MAX_SKIP}");
        }
        other => panic!("expected TooMuchSkip, got {other:?}"),
    }

    // Live state untouched: msg-1 still decrypts.
    assert_eq!(bob.decrypt(&h1, &c1).unwrap(), b"a-1");
}

#[test]
fn legacy_skipped_keys_object_form_is_accepted_on_load() {
    // Audit 2026-05-30 (R-6): reported live by Deniz — desktop installs
    // written between R-1 (added skipped_keys) and R-4 (added the Vec
    // codec) carry "skipped_keys": {} on disk. Loading those files with
    // the strict Vec-only deserializer errored out with "invalid type:
    // map, expected a sequence" and dropped every chat on upgrade. This
    // pins the tolerant-decoder behaviour so the bug can't sneak back in.
    let bob = SpendKey::generate();
    let alice = RatchetState::initialize_as_sender(shared(), bob.public);
    let json = serde_json::to_string(&alice).unwrap();
    // Splice the canonical (empty) `"skipped_keys":[]` into the legacy
    // `"skipped_keys":{}` object form that 1.1.6-era installs wrote.
    let legacy = json.replace("\"skipped_keys\":[]", "\"skipped_keys\":{}");
    assert!(legacy.contains("\"skipped_keys\":{}"));
    let mut restored: RatchetState =
        serde_json::from_str(&legacy).expect("legacy {} form must load");
    restored.restore_secret();
    // Sanity-check the restored state is still usable.
    let (h, c) = restored.encrypt(b"after legacy-format load");
    assert!(!h.is_empty() && !c.is_empty());
}

#[test]
fn state_with_skipped_keys_round_trips_through_json() {
    // Audit 2026-05-30 (R-4): a ratchet state that accumulated skipped
    // message keys must still serialize to JSON. The skipped-keys cache is
    // keyed by a ([u8;32], u32) tuple; serde_json rejects non-string map
    // keys, so the state needs the flat-list codec to persist. Before the
    // fix the CLI logged "session store serde: key must be a string" and
    // dropped the session after any out-of-order receive.
    let (mut alice, mut bob, _) = handshake();

    // Alice sends 1,2,3; Bob receives 3 first → 1 and 2 become skipped keys.
    let (_h1, _c1) = alice.encrypt(b"s-1");
    let (_h2, _c2) = alice.encrypt(b"s-2");
    let (h3, c3) = alice.encrypt(b"s-3");
    assert_eq!(bob.decrypt(&h3, &c3).unwrap(), b"s-3");

    // Bob's state now holds 2 skipped keys. Serialize → deserialize → it
    // must survive AND still decrypt the back-fill envelopes.
    let json = serde_json::to_string(&bob).expect("state with skipped keys must serialize to JSON");
    assert!(json.contains("skipped"), "skipped keys should be present in JSON");
    let mut restored: RatchetState = serde_json::from_str(&json).unwrap();
    restored.restore_secret();

    // The restored state still has the cached keys: re-deliver msg 1.
    assert_eq!(restored.decrypt(&_h1, &_c1).unwrap(), b"s-1");
    assert_eq!(restored.decrypt(&_h2, &_c2).unwrap(), b"s-2");
}

#[test]
fn dh_ratchet_step_commits_only_when_first_new_chain_message_validates() {
    // Alice and Bob have bootstrapped. Bob is about to ratchet to a new
    // chain on Alice's next send (which happens once Alice receives a Bob
    // message and rotates). To exercise the DH-ratchet commit path on
    // Alice's side specifically: Bob sends, Alice receives.
    let (mut alice, mut bob, _) = handshake();
    // Snapshot a real Bob-to-Alice message we'll deliver later as proof.
    let (h_b, c_b) = bob.encrypt(b"b-1");

    // Forge a Bob-shaped envelope with a brand-new (random) ratchet_pub
    // — pre-audit this triggered alice.dh_ratchet on the LIVE state, so
    // when the real h_b/c_b followed, the chain was already past it and
    // decryption silently failed.
    let mut forged = vec![0u8; 60];
    forged[0..32].copy_from_slice(&[0x42; 32]);
    forged[32..36].copy_from_slice(&1u32.to_le_bytes()); // valid 1-indexed counter
    forged[36..60].copy_from_slice(&[0x99; 24]);
    let forged_ct = vec![0u8; 80];
    let _ = alice.decrypt(&forged, &forged_ct); // expect Err; doesn't matter which

    // The real Bob message must still decrypt cleanly.
    assert_eq!(alice.decrypt(&h_b, &c_b).unwrap(), b"b-1");
}

// Helper: the ratchet APIs take &StaticSecret but SpendKey holds it behind
// its `secret` field. Go through `clone` since StaticSecret is Clone-safe
// and doesn't reveal material through `Debug`.
fn spend_secret(sk: &SpendKey) -> x25519_dalek::StaticSecret {
    sk.secret.clone()
}
