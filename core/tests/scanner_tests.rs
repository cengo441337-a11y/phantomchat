//! Scanner batch + PoW-filter tests.

use phantomchat_core::{
    envelope::Envelope,
    keys::{SpendKey, ViewKey},
    scan_batch,
    scanner::verify_pow,
};

#[test]
fn batch_scan_returns_only_matching_envelopes() {
    let alice_view  = ViewKey::generate();
    let alice_spend = SpendKey::generate();
    let bob_view    = ViewKey::generate();
    let bob_spend   = SpendKey::generate();

    let mut all = Vec::new();
    for i in 0..3 {
        all.push(Envelope::new(
            &alice_view.public,
            &alice_spend.public,
            i,
            vec![],
            format!("alice #{}", i).into_bytes(),
            60, 0,
        ));
    }
    for i in 0..5 {
        all.push(Envelope::new(
            &bob_view.public,
            &bob_spend.public,
            100 + i,
            vec![],
            format!("bob #{}", i).into_bytes(),
            60, 0,
        ));
    }

    let alice_matches = scan_batch(&all, &alice_view, &alice_spend);
    let bob_matches   = scan_batch(&all, &bob_view, &bob_spend);

    assert_eq!(alice_matches.len(), 3, "Alice must find exactly her 3 envelopes");
    assert_eq!(bob_matches.len(),   5, "Bob must find exactly his 5 envelopes");

    // And the contents match.
    for (i, p) in alice_matches.iter().enumerate() {
        assert_eq!(p.encrypted_body, format!("alice #{}", i).into_bytes());
    }
}

#[test]
fn verify_pow_accepts_envelopes_at_or_below_difficulty() {
    let view  = ViewKey::generate();
    let spend = SpendKey::generate();

    // Build with difficulty=8, verify at difficulty=8 must pass.
    let env = Envelope::new(&view.public, &spend.public, 0, vec![], vec![], 60, 8);
    assert!(verify_pow(&env, 8), "envelope built at difficulty 8 must verify at 8");

    // Lower required difficulty also passes.
    assert!(verify_pow(&env, 4));
    assert!(verify_pow(&env, 0));
}

#[test]
fn verify_pow_rejects_dummy_envelopes_at_nonzero_difficulty() {
    // Dummies carry pow_nonce = 0 — they must fail any real difficulty check.
    let dummy = Envelope::dummy().unwrap();
    assert!(!verify_pow(&dummy, 16), "dummies must fail real PoW checks");
}
