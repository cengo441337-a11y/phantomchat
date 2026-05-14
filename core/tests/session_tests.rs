//! End-to-end session tests exercising the full pipeline:
//! plaintext → ratchet encrypt → envelope seal → scan → envelope open →
//! ratchet decrypt → plaintext.

use phantomchat_core::{
    address::PhantomAddress,
    keys::{SpendKey, ViewKey},
    session::SessionStore,
};

struct Identity {
    view: ViewKey,
    spend: SpendKey,
    addr: PhantomAddress,
}

fn new_identity() -> Identity {
    let view = ViewKey::generate();
    let spend = SpendKey::generate();
    let addr = PhantomAddress::new(view.public, spend.public);
    Identity { view, spend, addr }
}

#[test]
fn address_wire_roundtrip() {
    let id = new_identity();
    let s = id.addr.to_string();
    assert!(s.starts_with("phantom:"));
    let parsed = PhantomAddress::parse(&s).expect("parse");
    assert_eq!(parsed, id.addr);

    // Also accept the raw view:spend form (without the `phantom:` prefix).
    let raw = s.strip_prefix("phantom:").unwrap();
    assert_eq!(PhantomAddress::parse(raw).unwrap(), id.addr);

    // And reject nonsense.
    assert!(PhantomAddress::parse("not an address").is_none());
    assert!(PhantomAddress::parse("phantom:abc:def").is_none());
}

#[test]
fn self_send_round_trip() {
    // Alice sends a message to herself — the simplest daily-driver smoke.
    let alice = new_identity();
    let mut store = SessionStore::new();

    let envelope = store.send(&alice.addr, b"echo", 0);
    let plaintext = store
        .receive(&envelope, &alice.view, &alice.spend)
        .expect("receive")
        .expect("should be mine");
    assert_eq!(plaintext, b"echo");
}

// Audit 2026-04-30 (C-1): receive-side PoW filter — opt-in via
// `set_min_pow_difficulty`. Default is `0` so the existing send/receive
// tests above (which all pass `difficulty=0`) keep round-tripping.
// These tests cover the new opt-in path.

#[test]
fn pow_filter_off_by_default_accepts_zero_difficulty() {
    let alice = new_identity();
    let mut store = SessionStore::new();
    assert_eq!(store.min_pow_difficulty(), 0);

    let envelope = store.send(&alice.addr, b"hi", 0);
    let plaintext = store
        .receive(&envelope, &alice.view, &alice.spend)
        .expect("receive")
        .expect("default filter must accept difficulty=0");
    assert_eq!(plaintext, b"hi");
}

#[test]
fn pow_filter_rejects_difficulty_below_floor() {
    let alice = new_identity();

    // Floor = 8. Send envelope was constructed at difficulty 0 (no PoW
    // grind). Receive must short-circuit to Ok(None).
    let mut sender = SessionStore::new();
    let envelope = sender.send(&alice.addr, b"spam", 0);

    let mut receiver = SessionStore::new();
    receiver.set_min_pow_difficulty(8);
    let outcome = receiver
        .receive(&envelope, &alice.view, &alice.spend)
        .expect("receive must not error");
    assert!(
        outcome.is_none(),
        "envelope built at difficulty 0 must be filtered when floor is 8"
    );
}

#[test]
fn pow_filter_accepts_envelope_at_or_above_floor() {
    let alice = new_identity();

    // Sender grinds difficulty 8; receiver demands at least 8.
    let mut sender = SessionStore::new();
    let envelope = sender.send(&alice.addr, b"legit", 8);

    let mut receiver = SessionStore::new();
    receiver.set_min_pow_difficulty(8);
    let plaintext = receiver
        .receive(&envelope, &alice.view, &alice.spend)
        .expect("receive")
        .expect("envelope at floor must pass");
    assert_eq!(plaintext, b"legit");
}

#[test]
fn alice_to_bob_multi_round_exchange() {
    let alice = new_identity();
    let bob = new_identity();

    let mut alice_store = SessionStore::new();
    let mut bob_store = SessionStore::new();

    // Round 1: A → B
    let env_a0 = alice_store.send(&bob.addr, b"hello bob", 0);
    let got_a0 = bob_store
        .receive(&env_a0, &bob.view, &bob.spend)
        .unwrap()
        .unwrap();
    assert_eq!(got_a0, b"hello bob");

    // Round 2: B → A (bootstraps A's view of the reverse direction)
    let env_b0 = bob_store.send(&alice.addr, b"hi alice", 0);
    let got_b0 = alice_store
        .receive(&env_b0, &alice.view, &alice.spend)
        .unwrap()
        .unwrap();
    assert_eq!(got_b0, b"hi alice");

    // Round 3: A → B (Alice's ratchet just rotated on receiving B0)
    let env_a1 = alice_store.send(&bob.addr, b"still here", 0);
    let got_a1 = bob_store
        .receive(&env_a1, &bob.view, &bob.spend)
        .unwrap()
        .unwrap();
    assert_eq!(got_a1, b"still here");

    // Round 4: B → A
    let env_b1 = bob_store.send(&alice.addr, b"same", 0);
    let got_b1 = alice_store
        .receive(&env_b1, &alice.view, &alice.spend)
        .unwrap()
        .unwrap();
    assert_eq!(got_b1, b"same");
}

#[test]
fn envelope_for_someone_else_is_silently_ignored() {
    let bob = new_identity();
    let eve = new_identity();

    let mut alice_store = SessionStore::new();
    let mut eve_store = SessionStore::new();

    let env = alice_store.send(&bob.addr, b"not for eve", 0);
    // Eve scans the same envelope from the relay — should see nothing.
    let out = eve_store
        .receive(&env, &eve.view, &eve.spend)
        .expect("no error");
    assert!(out.is_none(), "Eve must not be able to decrypt Bob's envelope");
}

#[test]
fn session_store_persists_to_disk() {
    // Alice talks to Bob across a "process restart": we send two messages,
    // Bob decrypts both (advancing his chain), then Alice's store is
    // serialised to disk, re-loaded, and must continue seamlessly — Bob
    // decrypts the third message against the same chain he built up.
    let bob = new_identity();

    let tmp = std::env::temp_dir().join(format!("phantom-sessions-{}.json", uuid_ish()));

    let mut bob_store = SessionStore::new();

    {
        let mut alice = SessionStore::new();
        let e0 = alice.send(&bob.addr, b"first", 0);
        let e1 = alice.send(&bob.addr, b"second", 0);
        assert_eq!(
            bob_store.receive(&e0, &bob.view, &bob.spend).unwrap().unwrap(),
            b"first"
        );
        assert_eq!(
            bob_store.receive(&e1, &bob.view, &bob.spend).unwrap().unwrap(),
            b"second"
        );
        alice.save(&tmp).unwrap();
    } // drop alice

    let mut alice2 = SessionStore::load(&tmp).unwrap();
    let env = alice2.send(&bob.addr, b"after-reload", 0);
    let decoded = bob_store
        .receive(&env, &bob.view, &bob.spend)
        .unwrap()
        .unwrap();
    assert_eq!(decoded, b"after-reload");

    let _ = std::fs::remove_file(tmp);
}

// Audit 2026-04-30 (H-7): the in-memory session map is bounded so a
// remote attacker can't pin our RAM by spraying envelopes with fresh
// `peer_ratchet_pub` values. Cap is `MAX_SESSIONS = 4096`; new peers
// beyond that are dropped silently (same wire surface as non-mine),
// while peers already in the map keep round-tripping.

#[test]
fn session_count_tracks_unique_peers() {
    let bob = new_identity();
    let mut bob_store = SessionStore::new();
    assert_eq!(bob_store.session_count(), 0);

    for _ in 0..3 {
        let mut alice_store = SessionStore::new();
        let env = alice_store.send(&bob.addr, b"hi", 0);
        bob_store
            .receive(&env, &bob.view, &bob.spend)
            .unwrap()
            .unwrap();
    }
    assert_eq!(bob_store.session_count(), 3);
}

#[test]
#[ignore = "slow: fills the map to MAX_SESSIONS (4096 ECDH ops in debug). Run with --ignored."]
fn session_cap_drops_new_peer_when_full() {
    use phantomchat_core::session::MAX_SESSIONS;

    let bob = new_identity();
    let mut bob_store = SessionStore::new();
    let mut first_alice_store: Option<SessionStore> = None;

    for i in 0..MAX_SESSIONS {
        let mut alice_store = SessionStore::new();
        let env = alice_store.send(&bob.addr, b"fill", 0);
        bob_store
            .receive(&env, &bob.view, &bob.spend)
            .unwrap()
            .unwrap();
        if i == 0 {
            first_alice_store = Some(alice_store);
        }
    }
    assert_eq!(bob_store.session_count(), MAX_SESSIONS);

    let mut eve_store = SessionStore::new();
    let env = eve_store.send(&bob.addr, b"dos", 0);
    let out = bob_store.receive(&env, &bob.view, &bob.spend).unwrap();
    assert!(
        out.is_none(),
        "fresh peer beyond MAX_SESSIONS must be silently dropped"
    );
    assert_eq!(bob_store.session_count(), MAX_SESSIONS);

    let mut alice0_store = first_alice_store.unwrap();
    let env = alice0_store.send(&bob.addr, b"still-here", 0);
    let plain = bob_store
        .receive(&env, &bob.view, &bob.spend)
        .unwrap()
        .expect("existing peer must still round-trip");
    assert_eq!(plain, b"still-here");
}

/// Tiny per-run disambiguator so parallel test processes don't clash on the
/// same tmp filename. Avoids pulling in the `uuid` crate just for a test.
fn uuid_ish() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{:x}-{}", nanos, std::process::id())
}
