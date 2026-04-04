//! Privacy mode configuration for PhantomChat.
//!
//! Two operational modes selected by the user:
//!
//! ## DailyUse (default)
//! - libp2p GossipSub with Dandelion++ routing
//! - Nostr relays over TLS as fallback transport
//! - Light cover traffic (30–180 s random intervals)
//! - Low battery/latency impact; suitable for everyday use
//!
//! ## MaximumStealth
//! - libp2p disabled entirely (no direct peer connections)
//! - All Nostr WebSocket connections routed exclusively through SOCKS5
//!   (Tor on 127.0.0.1:9050 by default, configurable for Nym)
//! - Aggressive cover traffic (5–15 s intervals)
//! - Protects against global passive adversaries (traffic-correlation attacks)
//! - User explicitly accepts higher battery usage and latency

use serde::{Deserialize, Serialize};

/// The two privacy operating modes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PrivacyMode {
    /// Fast & Secure — P2P + Dandelion++ + light cover traffic.
    #[default]
    DailyUse,
    /// Maximum Stealth — Nostr-over-proxy only + aggressive cover traffic.
    MaximumStealth,
}

/// Which anonymising proxy backs the Nostr transport in MaximumStealth.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProxyKind {
    /// Tor SOCKS5 daemon (default: 127.0.0.1:9050)
    Tor,
    /// Nym SOCKS5 client (default: 127.0.0.1:1080)
    Nym,
}

/// SOCKS5 proxy coordinates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyConfig {
    /// Socket address of the SOCKS5 proxy, e.g. `"127.0.0.1:9050"`.
    pub addr: String,
    pub kind: ProxyKind,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            addr: "127.0.0.1:9050".to_string(),
            kind: ProxyKind::Tor,
        }
    }
}

/// Full runtime privacy configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrivacyConfig {
    pub mode: PrivacyMode,
    /// Proxy used when `mode == MaximumStealth`.
    pub proxy: ProxyConfig,
    /// Whether the cover-traffic generator is active.
    pub cover_traffic_enabled: bool,
}

impl Default for PrivacyConfig {
    fn default() -> Self {
        Self {
            mode: PrivacyMode::DailyUse,
            proxy: ProxyConfig::default(),
            cover_traffic_enabled: true,
        }
    }
}

impl PrivacyConfig {
    /// `true` when the libp2p P2P layer should be started.
    #[inline]
    pub fn p2p_enabled(&self) -> bool {
        self.mode == PrivacyMode::DailyUse
    }

    /// Returns the SOCKS5 proxy address string when MaximumStealth is active.
    #[inline]
    pub fn proxy_addr(&self) -> Option<&str> {
        match self.mode {
            PrivacyMode::MaximumStealth => Some(&self.proxy.addr),
            PrivacyMode::DailyUse => None,
        }
    }
}
