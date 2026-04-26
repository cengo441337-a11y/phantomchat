//! End-to-end tests for the Sealed-Sender + Padding path.

use phantomchat_core::{
    address::PhantomAddress,
    envelope::{Payload, PAYLOAD_PAD_BLOCK},
    keys::{HybridKeyPair, PhantomSigningKey, SpendKey, ViewKey},
    session::SessionStore,
};

struct Id {
    view: ViewKey,
    spend: SpendKey,
    addr: PhantomAddress,
}

fn new_id() -> Id {
    let view = ViewKey::generate();
    let spend = SpendKey::generate();
    let addr = PhantomAddress::new(view.public, spend.public);
    Id { view, spend, addr }
}

#[test]
fn sealed_sender_roundtrip_and_signature_verifies() {
    let bob = new_id();
    let alice_sign = PhantomSigningKey::generate();

    let mut alice_store = SessionStore::new();
    let mut bob_store = SessionStore::new();

    let env = alice_store.send_sealed(&bob.addr, b"hi bob", &alice_sign, 0);

    let got = bob_store
        .receive_full(&env, &bob.view, &bob.spend, None)
        .expect("receive")
        .expect("should be mine");
    assert_eq!(got.plaintext, b"hi bob");

    let (attr, ok) = got.sender.expect("sealed sender present");
    assert!(ok, "signature must verify");
    assert_eq!(
        attr.sender_pub,
        alice_sign.public_bytes(),
        "sender_pub must match alice's signing key"
    );
}

#[test]
fn sealed_sender_block_catches_impersonation() {
    // Unit-level: verify() must fail when the signature was produced by a
    // *different* key than the one claimed in `sender_pub`. This is the
    // exact attack pattern receive_full guards against when it reports
    // `sender: Some((_, false))`.
    use phantomchat_core::SealedSender;

    let real = PhantomSigningKey::generate();
    let impostor = PhantomSigningKey::generate();

    // The impostor signs the same payload bytes but publishes `real`'s
    // public key as `sender_pub`.
    let rh = vec![7u8; 60];
    let body = b"payload".to_vec();
    let mut joined = rh.clone();
    joined.extend_from_slice(&body);

    let fake = SealedSender {
        sender_pub: real.public_bytes(),
        signature: impostor.sign(&joined),
    };
    assert!(!fake.verify(&rh, &body), "impersonated signature must NOT verify");

    // And the genuine one still does.
    let genuine = SealedSender::new(&real, &rh, &body);
    assert!(genuine.verify(&rh, &body));
}

#[test]
fn payload_wire_serialisation_is_padded_to_block() {
    // Different plaintext lengths should produce *the same* wire size
    // after Payload::to_bytes — that's the whole point of padding against
    // length correlation.
    let p_small = Payload::classic(0, vec![0u8; 60], vec![0u8; 10]);
    let p_med = Payload::classic(0, vec![0u8; 60], vec![0u8; 800]);
    let p_large = Payload::classic(0, vec![0u8; 60], vec![0u8; 1000]);

    let b_small = p_small.to_bytes();
    let b_med = p_med.to_bytes();
    let b_large = p_large.to_bytes();

    assert_eq!(b_small.len() % PAYLOAD_PAD_BLOCK, 0);
    assert_eq!(b_med.len()   % PAYLOAD_PAD_BLOCK, 0);
    assert_eq!(b_large.len() % PAYLOAD_PAD_BLOCK, 0);

    // A 10-byte body and an 800-byte body should end up in the same
    // block when both fit in the first 1024 bytes.
    assert_eq!(b_small.len(), b_med.len());
    // A 1000-byte body spills into the next block.
    assert!(b_large.len() > b_med.len());
}

#[test]
fn padded_payload_roundtrips_through_from_bytes() {
    let p = Payload::classic(42, vec![1u8; 60], b"hello-world".to_vec());
    let wire = p.to_bytes();
    let restored = Payload::from_bytes(&wire).expect("parse");
    assert_eq!(restored.msg_id, p.msg_id);
    assert_eq!(restored.ratchet_header, p.ratchet_header);
    assert_eq!(restored.encrypted_body, p.encrypted_body);
    // Padding is present but we don't care what's in it — only that the
    // length brings the wire up to the next block boundary.
    assert!(restored.padding.len() <= PAYLOAD_PAD_BLOCK);
}

#[test]
fn sealed_sender_works_over_hybrid_envelope() {
    // Carol uses a hybrid address (PQXDH). A sealed envelope through the
    // hybrid path must still carry + verify the Ed25519 sender attribution.
    let carol_view = ViewKey::generate();
    let carol_kp = HybridKeyPair::generate();
    let carol_spend = SpendKey {
        public: carol_kp.public.x25519,
        secret: carol_kp.secret.x25519.clone(),
    };
    let carol_addr = PhantomAddress::new_hybrid(
        carol_view.public,
        carol_kp.public.x25519,
        carol_kp.public.to_bytes()[32..].to_vec(),
    );

    let dave = new_id();
    let dave_sign = PhantomSigningKey::generate();

    let mut dave_store = SessionStore::new();
    let mut carol_store = SessionStore::new();

    let env = dave_store.send_sealed(&carol_addr, b"pq+sealed", &dave_sign, 0);
    assert_eq!(env.ver, phantomchat_core::envelope::VERSION_HYBRID);

    let got = carol_store
        .receive_full(&env, &carol_view, &carol_spend, Some(&carol_kp.secret))
        .unwrap()
        .unwrap();
    assert_eq!(got.plaintext, b"pq+sealed");
    let (attr, ok) = got.sender.unwrap();
    assert!(ok);
    assert_eq!(attr.sender_pub, dave_sign.public_bytes());
    let _ = dave;
}
