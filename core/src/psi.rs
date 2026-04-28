//! Private Set Intersection — contact discovery without leaking the non-matching half.
//!
//! Alice wants to know which of her contacts are also PhantomChat users
//! (from some authoritative directory of registered `PhantomAddress`es
//! Bob publishes), **without** telling Bob who else is in her address
//! book and **without** letting Bob learn anything about the people she
//! knows who are *not* in his directory.
//!
//! ## Protocol (3-round DDH-PSI)
//!
//! Both sides use the Ristretto group. Alice holds `α` (secret scalar),
//! Bob holds `β`. `H(·)` is hash-to-Ristretto.
//!
//! ```text
//!   Round 1  Alice → Bob :  X_A = { H(a)^α  : a ∈ A }
//!   Round 2  Bob   → Alice: X_B = { H(b)^β  : b ∈ B }
//!                         + Z_A = { x^β    : x ∈ X_A }          // = H(a)^(αβ)
//!   Round 3  Alice locally: Z_B = { y^α    : y ∈ X_B }          // = H(b)^(αβ)
//!            intersection  = { a_i | Z_A[i] ∈ set(Z_B) }
//! ```
//!
//! Each side learns *only* the intersection — the DDH assumption in
//! Ristretto255 hides everything else.
//!
//! ## Security notes
//!
//! - Re-using `α` or `β` across runs leaks set membership across the
//!   sessions. Generate a fresh scalar per PSI exchange (the API does).
//! - Untrusted parties can pad their set with extra elements to detect
//!   whether a *specific* a ∈ A is in B. Mitigate by capping query size
//!   at the protocol level.

use curve25519_dalek::{
    ristretto::{CompressedRistretto, RistrettoPoint},
    scalar::Scalar,
};
use rand_core::OsRng;
use sha2::{Digest, Sha512};

use crate::address::PhantomAddress;

/// HKDF-style domain separation tag. Bumping invalidates all in-flight
/// PSI sessions.
pub const PSI_DOMAIN: &[u8] = b"PhantomChat-v1-PSI";

#[derive(Debug, thiserror::Error)]
pub enum PsiError {
    #[error("compressed Ristretto point failed to decode")]
    BadPoint,
    #[error("double-blind arity mismatch (got {got} entries, expected {expected})")]
    ArityMismatch { got: usize, expected: usize },
}

/// Hash a PhantomAddress into a Ristretto point. Domain-separated so
/// points used in PSI can never collide with points from any other
/// PhantomChat subprotocol.
fn hash_to_point(addr: &PhantomAddress) -> RistrettoPoint {
    let mut h = Sha512::new();
    h.update(PSI_DOMAIN);
    h.update(addr.view_pub);
    h.update(addr.spend_pub);
    if let Some(mlkem) = &addr.mlkem_pub {
        h.update(mlkem);
    }
    let digest = h.finalize();
    let mut buf = [0u8; 64];
    buf.copy_from_slice(&digest);
    RistrettoPoint::from_uniform_bytes(&buf)
}

fn compress_point(p: RistrettoPoint) -> [u8; 32] {
    p.compress().to_bytes()
}

fn decompress_point(bytes: &[u8; 32]) -> Result<RistrettoPoint, PsiError> {
    CompressedRistretto(*bytes)
        .decompress()
        .ok_or(PsiError::BadPoint)
}

/// Alice-side session. Keeps the secret scalar hidden; exposes the
/// blinded set for transmission.
pub struct PsiClient {
    scalar: Scalar,
    /// The addresses Alice is querying, in order. Needed to reconstruct
    /// the human-readable intersection after the protocol returns.
    originals: Vec<PhantomAddress>,
    /// Parallel vector to `originals`: `H(a)^α` as compressed bytes.
    blinded: Vec<[u8; 32]>,
}

impl PsiClient {
    pub fn new(local_set: &[PhantomAddress]) -> Self {
        let scalar = Scalar::random(&mut OsRng);
        let blinded: Vec<[u8; 32]> = local_set
            .iter()
            .map(|a| compress_point(hash_to_point(a) * scalar))
            .collect();
        Self {
            scalar,
            originals: local_set.to_vec(),
            blinded,
        }
    }

    /// Round 1 — the bytes Alice ships to Bob.
    pub fn blinded_query(&self) -> &[[u8; 32]] {
        &self.blinded
    }

    /// Round 3 — finish the protocol with Bob's responses and return
    /// the subset of `local_set` that is also in Bob's directory.
    ///
    /// `my_doubly_blinded` is `Z_A` (Bob exponentiated Alice's round-1
    /// messages with β). `peer_blinded` is `X_B` (Bob's own blinded set).
    pub fn intersect(
        &self,
        my_doubly_blinded: &[[u8; 32]],
        peer_blinded: &[[u8; 32]],
    ) -> Result<Vec<PhantomAddress>, PsiError> {
        if my_doubly_blinded.len() != self.originals.len() {
            return Err(PsiError::ArityMismatch {
                got: my_doubly_blinded.len(),
                expected: self.originals.len(),
            });
        }

        // Re-blind Bob's set with α → produces H(b)^(αβ).
        let mut peer_double: std::collections::HashSet<[u8; 32]> =
            std::collections::HashSet::with_capacity(peer_blinded.len());
        for pb in peer_blinded {
            let pt = decompress_point(pb)?;
            peer_double.insert(compress_point(pt * self.scalar));
        }

        let mut hits = Vec::new();
        for (addr, mine) in self.originals.iter().zip(my_doubly_blinded.iter()) {
            if peer_double.contains(mine) {
                hits.push(addr.clone());
            }
        }
        Ok(hits)
    }
}

/// Bob-side session. Bob holds the directory; he answers a client query
/// without learning anything beyond the intersection cardinality.
pub struct PsiServer {
    scalar: Scalar,
    /// Parallel: directory entries + pre-computed `H(b)^β`.
    #[allow(dead_code)]
    originals: Vec<PhantomAddress>,
    blinded: Vec<[u8; 32]>,
}

impl PsiServer {
    pub fn new(directory: &[PhantomAddress]) -> Self {
        let scalar = Scalar::random(&mut OsRng);
        let blinded: Vec<[u8; 32]> = directory
            .iter()
            .map(|a| compress_point(hash_to_point(a) * scalar))
            .collect();
        Self {
            scalar,
            originals: directory.to_vec(),
            blinded,
        }
    }

    /// Round 2 part 1 — directory blinded with `β`, shipped to Alice.
    pub fn blinded_directory(&self) -> &[[u8; 32]] {
        &self.blinded
    }

    /// Round 2 part 2 — exponentiate Alice's round-1 messages with `β`.
    pub fn double_blind(&self, client_blinded: &[[u8; 32]]) -> Result<Vec<[u8; 32]>, PsiError> {
        let mut out = Vec::with_capacity(client_blinded.len());
        for cb in client_blinded {
            let pt = decompress_point(cb)?;
            out.push(compress_point(pt * self.scalar));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::{SpendKey, ViewKey};

    fn mock_addr() -> PhantomAddress {
        PhantomAddress::new(ViewKey::generate().public, SpendKey::generate().public)
    }

    #[test]
    fn psi_returns_exact_intersection() {
        let shared = mock_addr();
        let shared2 = mock_addr();

        let alice_only = mock_addr();
        let bob_only = mock_addr();

        let alice_set = vec![alice_only.clone(), shared.clone(), shared2.clone()];
        let bob_set   = vec![bob_only.clone(), shared.clone(), shared2.clone()];

        let alice = PsiClient::new(&alice_set);
        let bob   = PsiServer::new(&bob_set);

        let my_dbl = bob.double_blind(alice.blinded_query()).unwrap();
        let peer_blinded = bob.blinded_directory().to_vec();
        let hits = alice.intersect(&my_dbl, &peer_blinded).unwrap();

        let hit_set: std::collections::HashSet<_> =
            hits.iter().map(|a| (a.view_pub, a.spend_pub)).collect();
        assert!(hit_set.contains(&(shared.view_pub, shared.spend_pub)));
        assert!(hit_set.contains(&(shared2.view_pub, shared2.spend_pub)));
        assert!(!hit_set.contains(&(alice_only.view_pub, alice_only.spend_pub)));
        assert!(!hit_set.contains(&(bob_only.view_pub, bob_only.spend_pub)));
    }

    #[test]
    fn psi_empty_intersection_leaks_nothing() {
        let alice_set: Vec<_> = (0..5).map(|_| mock_addr()).collect();
        let bob_set: Vec<_> = (0..5).map(|_| mock_addr()).collect();

        let alice = PsiClient::new(&alice_set);
        let bob   = PsiServer::new(&bob_set);

        let dbl = bob.double_blind(alice.blinded_query()).unwrap();
        let peer = bob.blinded_directory().to_vec();
        let hits = alice.intersect(&dbl, &peer).unwrap();
        assert_eq!(hits.len(), 0);
    }

    #[test]
    fn psi_detects_every_self_match() {
        // Alice and Bob share the exact same set — all 4 should match.
        let shared: Vec<_> = (0..4).map(|_| mock_addr()).collect();
        let alice = PsiClient::new(&shared);
        let bob   = PsiServer::new(&shared);

        let dbl = bob.double_blind(alice.blinded_query()).unwrap();
        let peer = bob.blinded_directory().to_vec();
        let hits = alice.intersect(&dbl, &peer).unwrap();
        assert_eq!(hits.len(), 4);
    }

    #[test]
    fn psi_arity_check() {
        let alice = PsiClient::new(&[mock_addr(), mock_addr()]);
        // Give Alice the wrong number of double-blinded replies.
        let wrong = vec![[0u8; 32]];
        assert!(alice.intersect(&wrong, &[]).is_err());
    }

    #[test]
    fn psi_fresh_scalars_each_run() {
        // Two independent PsiClient instances for the same local_set
        // must produce different `blinded_query()` outputs — otherwise
        // repeat-runs leak cross-session membership.
        let set = vec![mock_addr()];
        let a = PsiClient::new(&set);
        let b = PsiClient::new(&set);
        assert_ne!(a.blinded_query(), b.blinded_query());
    }
}
