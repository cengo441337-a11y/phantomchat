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

// Helper: the ratchet APIs take &StaticSecret but SpendKey holds it behind
// its `secret` field. Go through `clone` since StaticSecret is Clone-safe
// and doesn't reveal material through `Debug`.
fn spend_secret(sk: &SpendKey) -> x25519_dalek::StaticSecret {
    sk.secret.clone()
}
