//! Dandelion++ routing for PhantomChat P2P.
//!
//! ## How Dandelion++ works
//!
//! Message propagation is split into two phases:
//!
//! ```text
//! Sender → [stem] → peer A → [stem] → peer B → [FLUFF] → full broadcast
//!                                       ↑
//!                              transition coin-flip
//! ```
//!
//! ### Stem phase
//! The originating node forwards the envelope to exactly **one** randomly-chosen
//! peer (the "stem peer"). Each relay node flips a biased coin (FLUFF_PROB ≈ 0.1):
//! heads → transition to fluff; tails → stem to the next single peer.
//!
//! ### Fluff phase
//! The envelope is broadcast to all peers via GossipSub as usual.
//!
//! ### Epoch rotation
//! Every `EPOCH_DURATION` seconds each node re-picks a random stem peer, so no
//! long-term correlation between envelopes and a fixed forwarding path builds up.
//!
//! ## Privacy gain
//! An adversary watching the network can only see the fluff-phase broadcaster,
//! which is several hops removed from the true origin, making IP-origin linking
//! much harder than plain GossipSub flooding.

use libp2p::PeerId;
use rand::Rng;
use std::time::{Duration, Instant};

/// Probability of transitioning from stem to fluff at each relay hop.
const FLUFF_PROB: f64 = 0.1;

/// How long before the stem peer is rotated to a new random peer.
const EPOCH_DURATION: Duration = Duration::from_secs(600); // 10 minutes

/// Decision at a single routing step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    /// Forward to a single stem peer.
    Stem,
    /// Broadcast to all peers (standard GossipSub publish).
    Fluff,
}

/// Dandelion++ routing state for a single node.
pub struct DandelionRouter {
    /// The current stem peer for this epoch (None if no peers available).
    stem_peer: Option<PeerId>,
    /// When the current epoch began.
    epoch_start: Instant,
    /// Current known peers.
    peers: Vec<PeerId>,
}

impl DandelionRouter {
    pub fn new() -> Self {
        Self {
            stem_peer: None,
            epoch_start: Instant::now(),
            peers: Vec::new(),
        }
    }

    /// Update the known peer list.
    /// Automatically rotates the stem peer when the epoch expires or when no
    /// stem peer has been chosen yet.
    pub fn update_peers(&mut self, peers: Vec<PeerId>) {
        self.peers = peers;
        let epoch_expired = self.epoch_start.elapsed() > EPOCH_DURATION;
        if self.stem_peer.is_none() || epoch_expired {
            self.rotate_stem();
        }
    }

    /// Add a single newly discovered peer and rotate stem if needed.
    pub fn add_peer(&mut self, peer: PeerId) {
        if !self.peers.contains(&peer) {
            self.peers.push(peer);
        }
        if self.stem_peer.is_none() || self.epoch_start.elapsed() > EPOCH_DURATION {
            self.rotate_stem();
        }
    }

    /// Remove a peer (e.g. on disconnect).
    pub fn remove_peer(&mut self, peer: &PeerId) {
        self.peers.retain(|p| p != peer);
        // If we lost our stem peer, pick a new one immediately.
        if self.stem_peer.as_ref() == Some(peer) {
            self.rotate_stem();
        }
    }

    /// Decide the routing phase for an outgoing envelope.
    ///
    /// Returns `Phase::Stem` with probability `(1 - FLUFF_PROB)` and
    /// `Phase::Fluff` with probability `FLUFF_PROB`.
    /// Falls back to `Fluff` when no stem peer is available.
    pub fn route(&self) -> Phase {
        if self.stem_peer.is_none() {
            return Phase::Fluff;
        }
        if rand::thread_rng().gen_bool(FLUFF_PROB) {
            Phase::Fluff
        } else {
            Phase::Stem
        }
    }

    /// The peer to forward to during the stem phase.
    pub fn stem_peer(&self) -> Option<PeerId> {
        self.stem_peer
    }

    /// Force an immediate epoch rotation (e.g. after a batch of stem messages).
    pub fn force_rotate(&mut self) {
        self.rotate_stem();
    }

    // ── private ────────────────────────────────────────────────────────────

    fn rotate_stem(&mut self) {
        self.epoch_start = Instant::now();
        if self.peers.is_empty() {
            self.stem_peer = None;
            return;
        }
        let idx = rand::thread_rng().gen_range(0..self.peers.len());
        self.stem_peer = Some(self.peers[idx]);
        tracing::debug!(
            "Dandelion++: new stem peer {:?} (epoch rotated)",
            self.stem_peer
        );
    }
}

impl Default for DandelionRouter {
    fn default() -> Self {
        Self::new()
    }
}
