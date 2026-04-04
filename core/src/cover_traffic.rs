//! Cover traffic generator for PhantomChat.
//!
//! Sends periodic **dummy envelopes** into the network so that an observer
//! cannot distinguish real message traffic from noise based on timing alone.
//!
//! ## Modes
//!
//! | Mode        | Interval      | Use case                            |
//! |-------------|---------------|-------------------------------------|
//! | Light       | 30–180 s rnd  | DailyUse — minimal battery impact   |
//! | Aggressive  | 5–15 s rnd    | MaximumStealth — masks real patterns |
//!
//! ## Dummy envelope design
//! A dummy envelope is cryptographically indistinguishable from a real one:
//! - All fields are randomly filled with CSPRNG output
//! - `pow_nonce` is set to 0 (valid receivers reject it via PoW check, but
//!   the bytes look identical on the wire)
//! - Real recipients' scan loop simply fails HMAC check and discards it
//!
//! No sensitive data is ever included. The only cost is bandwidth.

use rand::Rng;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time;
use tracing::debug;
use crate::envelope::Envelope;

/// Cover traffic intensity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoverTrafficMode {
    /// 30–180 s random delay between dummies (DailyUse).
    Light,
    /// 5–15 s random delay between dummies (MaximumStealth).
    Aggressive,
}

/// Drives periodic dummy-envelope emission on a background task.
pub struct CoverTrafficGenerator {
    mode: CoverTrafficMode,
    /// Sink into the envelope publishing pipeline.
    publish_tx: mpsc::Sender<Envelope>,
}

impl CoverTrafficGenerator {
    pub fn new(mode: CoverTrafficMode, publish_tx: mpsc::Sender<Envelope>) -> Self {
        Self { mode, publish_tx }
    }

    /// Spawn the background task. Returns immediately; task runs until the
    /// `publish_tx` channel is closed.
    pub fn start(self) {
        tokio::spawn(async move {
            loop {
                let delay_secs: u64 = match self.mode {
                    CoverTrafficMode::Light => rand::thread_rng().gen_range(30..=180),
                    CoverTrafficMode::Aggressive => rand::thread_rng().gen_range(5..=15),
                };
                time::sleep(Duration::from_secs(delay_secs)).await;

                match Envelope::dummy() {
                    Some(dummy) => {
                        if self.publish_tx.send(dummy).await.is_err() {
                            // Channel closed — node is shutting down.
                            break;
                        }
                        debug!(
                            "Cover traffic: dummy envelope emitted (mode={:?}, interval={}s)",
                            self.mode, delay_secs
                        );
                    }
                    None => {
                        tracing::warn!("Cover traffic: failed to generate dummy envelope");
                    }
                }
            }
        });
    }
}
