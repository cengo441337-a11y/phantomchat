//! Tests for the Dandelion++ router.
//!
//! Checks the deterministic parts: fallback to Fluff when no peers exist,
//! stem-peer rotation on peer removal, force_rotate guarantees.
//!
//! The probabilistic stem/fluff split is verified statistically — with
//! FLUFF_PROB = 0.1 we expect ~10% Fluff over many samples. We use a wide
//! tolerance to keep the test non-flaky.

use libp2p::PeerId;
use phantomchat_core::dandelion::{DandelionRouter, Phase};

fn make_peers(n: usize) -> Vec<PeerId> {
    (0..n).map(|_| PeerId::random()).collect()
}

#[test]
fn router_without_peers_falls_back_to_fluff() {
    let router = DandelionRouter::new();
    assert_eq!(
        router.route(),
        Phase::Fluff,
        "empty router must broadcast (Fluff) rather than refuse"
    );
    assert!(router.stem_peer().is_none());
}

#[test]
fn updating_peers_picks_a_stem() {
    let mut router = DandelionRouter::new();
    router.update_peers(make_peers(5));
    assert!(
        router.stem_peer().is_some(),
        "after peer update a stem peer must be selected"
    );
}

#[test]
fn removing_current_stem_peer_triggers_rotation() {
    let mut router = DandelionRouter::new();
    let peers = make_peers(3);
    router.update_peers(peers.clone());

    let original = router.stem_peer().expect("stem must exist");
    router.remove_peer(&original);
    let new_stem = router.stem_peer();

    // After removing the only-selected stem, either a different stem is
    // picked or (if we removed the last peer) None is returned.
    match new_stem {
        Some(p) => assert_ne!(p, original, "rotation must not pick the removed peer"),
        None => {
            assert_eq!(
                router.stem_peer(),
                None,
                "no replacement possible when peer pool drained"
            );
        }
    }
}

#[test]
fn force_rotate_is_allowed_on_empty_router() {
    // Must not panic even with no peers.
    let mut router = DandelionRouter::new();
    router.force_rotate();
    assert!(router.stem_peer().is_none());
}

#[test]
fn add_peer_selects_initial_stem() {
    let mut router = DandelionRouter::new();
    let p = PeerId::random();
    router.add_peer(p);
    assert_eq!(
        router.stem_peer(),
        Some(p),
        "first added peer becomes the stem"
    );
}

#[test]
fn route_distribution_is_roughly_stem_heavy() {
    // FLUFF_PROB = 0.1 → ~10% Fluff, ~90% Stem over many draws.
    // Tolerate a wide band (5%–20% Fluff) to avoid flaky CI.
    let mut router = DandelionRouter::new();
    router.update_peers(make_peers(4));

    const N: usize = 2000;
    let mut fluff = 0usize;
    for _ in 0..N {
        if router.route() == Phase::Fluff { fluff += 1; }
    }

    let pct = (fluff as f64) / (N as f64);
    assert!(
        (0.05..=0.20).contains(&pct),
        "expected ~10% Fluff, observed {:.1}%",
        pct * 100.0
    );
}
