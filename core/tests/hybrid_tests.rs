//! PQXDH-hybrid envelope tests.
//!
//! Anchors the end-to-end claim that a hybrid address → hybrid envelope
//! → hybrid open round-trips plaintext, and that a receiver without the
//! PQ secret cannot open a v2 envelope even with the right SpendKey.

use phantomchat_core::{
    address::PhantomAddress,
    envelope::{VERSION_CLASSIC, VERSION_HYBRID},
    keys::{HybridKeyPair, SpendKey, ViewKey},
    session::SessionStore,
};

struct HybridIdentity {
    view: ViewKey,
    spend: SpendKey,   // X25519 half doubles as the hybrid.x25519
    hybrid: HybridKeyPair,
    addr: PhantomAddress,
}

fn new_hybrid_identity() -> HybridIdentity {
    let view = ViewKey::generate();
    let hybrid = HybridKeyPair::generate();

    // The session code treats recipient.spend_pub as `hybrid.public.x25519`
    // for hybrid envelopes, so we keep them in sync by constructing a
    // SpendKey from the hybrid's X25519 secret.
    let spend_secret = hybrid.secret.x25519.clone();
    let spend = SpendKey {
        public: hybrid.public.x25519,
        secret: spend_secret,
    };

    let addr = PhantomAddress::new_hybrid(
        view.public,
        hybrid.public.x25519,
        hybrid.public.to_bytes()[32..].to_vec(), // 1568-byte ML-KEM half
    );

    HybridIdentity { view, spend, hybrid, addr }
}

#[test]
fn hybrid_address_wire_roundtrip() {
    let id = new_hybrid_identity();
    let s = id.addr.to_string();
    assert!(s.starts_with("phantomx:"), "hybrid addresses use the phantomx: prefix");

    let parsed = PhantomAddress::parse(&s).expect("parse");
    assert_eq!(parsed, id.addr);
    assert!(parsed.is_hybrid());

    // Raw four-part form (no prefix) also accepted.
    let raw = s.strip_prefix("phantomx:").unwrap();
    let parsed2 = PhantomAddress::parse(raw).expect("parse raw");
    assert_eq!(parsed2, id.addr);
}

#[test]
fn classic_address_is_not_hybrid() {
    let v = ViewKey::generate();
    let s = SpendKey::generate();
    let addr = PhantomAddress::new(v.public, s.public);
    assert!(!addr.is_hybrid());
    assert!(addr.mlkem_pub.is_none());
    assert!(addr.to_string().starts_with("phantom:"));
}

#[test]
fn hybrid_self_send_roundtrip() {
    let alice = new_hybrid_identity();

    let mut store = SessionStore::new();
    let envelope = store.send(&alice.addr, b"quantum-safe hi", 0);

    // The envelope must be hybrid since the recipient carries an ML-KEM pub.
    assert_eq!(envelope.ver, VERSION_HYBRID, "recipient with mlkem_pub → v2");
    assert!(envelope.mlkem_ct.is_some(), "v2 must carry a PQ ciphertext");

    let plain = store
        .receive_hybrid(&envelope, &alice.view, &alice.spend, &alice.hybrid.secret)
        .expect("receive")
        .expect("should be mine");
    assert_eq!(plain, b"quantum-safe hi");
}

#[test]
fn classic_receive_ignores_hybrid_envelope() {
    // Same identity both sides, but the receiver call omits the PQ secret.
    // The scanner tag still matches — but the hybrid AEAD key can't be
    // derived without ML-KEM, so we should silently return `None` rather
    // than attempt a classic-open that would fail loudly.
    let alice = new_hybrid_identity();

    let mut store = SessionStore::new();
    let envelope = store.send(&alice.addr, b"pq only", 0);
    assert_eq!(envelope.ver, VERSION_HYBRID);

    let out = store
        .receive(&envelope, &alice.view, &alice.spend)
        .expect("no hard error");
    assert!(
        out.is_none(),
        "classic receive must not leak / mis-decrypt a hybrid envelope"
    );
}

#[test]
fn hybrid_envelope_ignored_by_foreign_identity() {
    let alice = new_hybrid_identity();
    let bob   = new_hybrid_identity();

    let mut a_store = SessionStore::new();
    let mut b_store = SessionStore::new();

    let env = a_store.send(&alice.addr, b"not for bob", 0);
    let out = b_store
        .receive_hybrid(&env, &bob.view, &bob.spend, &bob.hybrid.secret)
        .expect("no hard error");
    assert!(out.is_none());
}

#[test]
fn wire_bytes_roundtrip_preserves_mlkem_ct() {
    let alice = new_hybrid_identity();
    let mut store = SessionStore::new();
    let env = store.send(&alice.addr, b"wire", 0);

    let wire = env.to_bytes();
    let restored = phantomchat_core::envelope::Envelope::from_bytes(&wire)
        .expect("wire parse");
    assert_eq!(restored.ver, VERSION_HYBRID);
    assert_eq!(restored.mlkem_ct, env.mlkem_ct);
    assert_eq!(restored.ciphertext, env.ciphertext);

    // And it still decrypts after the round trip.
    let plain = store
        .receive_hybrid(&restored, &alice.view, &alice.spend, &alice.hybrid.secret)
        .unwrap()
        .unwrap();
    assert_eq!(plain, b"wire");
}

#[test]
fn classic_envelope_still_classic_after_extension() {
    // Belt-and-braces: adding the hybrid path must not break any v1 flow.
    let view = ViewKey::generate();
    let spend = SpendKey::generate();
    let addr = PhantomAddress::new(view.public, spend.public);

    let mut store = SessionStore::new();
    let env = store.send(&addr, b"still classic", 0);
    assert_eq!(env.ver, VERSION_CLASSIC);
    assert!(env.mlkem_ct.is_none());

    let plain = store
        .receive(&env, &view, &spend)
        .unwrap()
        .unwrap();
    assert_eq!(plain, b"still classic");
}
