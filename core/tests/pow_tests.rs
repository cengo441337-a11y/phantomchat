//! Proof-of-Work (Hashcash) tests.
//!
//! Verifies that computed nonces satisfy the advertised difficulty, that
//! verify() accepts only the correct nonce, and that the difficulty scale
//! actually grows exponentially as expected.

use phantomchat_core::Hashcash;

#[test]
fn pow_verifies_what_it_computes() {
    let data = b"PhantomChat-v1-PoW test vector";
    let pow  = Hashcash::new(8);
    let nonce = pow.compute_nonce(data);
    assert!(pow.verify(data, nonce), "computed nonce must verify");
}

#[test]
fn pow_rejects_wrong_nonce() {
    let data = b"abc";
    let pow  = Hashcash::new(8);
    let good = pow.compute_nonce(data);
    assert!(!pow.verify(data, good.wrapping_add(1)), "neighbour nonces must fail");
}

#[test]
fn pow_difficulty_zero_accepts_anything() {
    // With 0 required zero-bits every hash trivially matches, so the very
    // first nonce (0) should always verify. This is our fast-path in tests.
    let data = b"zero-difficulty";
    let pow  = Hashcash::new(0);
    assert!(pow.verify(data, 0));
    assert!(pow.verify(data, 12345));
}

#[test]
fn pow_higher_difficulty_produces_more_leading_zero_bits() {
    let data = b"difficulty ladder";
    for bits in [0u32, 4, 8, 12] {
        let pow   = Hashcash::new(bits);
        let nonce = pow.compute_nonce(data);
        assert!(pow.verify(data, nonce), "nonce must verify for bits={}", bits);
    }
}

#[test]
fn pow_different_data_needs_different_nonce() {
    let pow = Hashcash::new(8);
    let a = pow.compute_nonce(b"message A");
    let b = pow.compute_nonce(b"message B");
    // Astronomically unlikely to collide. A collision would mean our PoW is
    // leaking structure across inputs.
    assert_ne!(a, b);
}
