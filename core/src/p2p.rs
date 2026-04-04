//! libp2p GossipSub node for fully decentralized envelope distribution.
//!
//! ## Architecture
//! Each PhantomChat client optionally runs a libp2p node that:
//! 1. Joins a GossipSub topic ("phantom/envelopes/1")
//! 2. Discovers peers via mDNS (LAN) and Kademlia DHT (internet)
//! 3. Publishes envelopes to the mesh — all peers receive all messages
//! 4. Incoming envelopes are passed to the scanner (ViewKey scan)
//!
//! ## Why libp2p over Nostr relays?
//! - No relay infrastructure needed at all — pure P2P
//! - Relay-less: even if all Nostr relays go down, P2P still works
//! - GossipSub provides efficient fan-out without central coordination
//! - Kademlia DHT enables peer discovery without bootstrapping server
//!
//! ## Privacy note
//! GossipSub reveals your IP to direct peers. For higher anonymity,
//! combine with Tor transport (feature flag: `p2p-tor`) or use
//! Nostr relay transport instead. The trade-off: P2P = faster delivery,
//! Nostr relay = better network-level privacy.

use libp2p::{
    gossipsub::{self, Behaviour as Gossipsub, ConfigBuilder as GossipsubConfigBuilder,
        Event as GossipsubEvent, IdentTopic, MessageAuthenticity, ValidationMode},
    identify::{self, Behaviour as Identify},
    kad::{self, store::MemoryStore, Behaviour as Kademlia},
    mdns::{self, tokio::Behaviour as Mdns},
    noise, yamux,
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, Multiaddr, PeerId, SwarmBuilder,
};
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{info, warn, debug};
use crate::envelope::Envelope;

/// PhantomChat P2P GossipSub topic identifier.
const PHANTOM_TOPIC: &str = "phantom/envelopes/1";

/// Combined libp2p behaviour for PhantomChat.
#[derive(NetworkBehaviour)]
pub struct PhantomBehaviour {
    pub gossipsub: Gossipsub,
    pub kademlia: Kademlia<MemoryStore>,
    pub mdns: Mdns,
    pub identify: Identify,
}

/// A running P2P node. Spawn with `PhantomNode::start()`.
pub struct PhantomNode {
    /// Channel to send envelopes for publishing.
    pub publish_tx: mpsc::Sender<Envelope>,
    /// Channel receiving envelopes from the network.
    pub receive_rx: mpsc::Receiver<Envelope>,
    /// This node's PeerId.
    pub local_peer_id: PeerId,
}

impl PhantomNode {
    /// Start a P2P node. Binds TCP on a random port.
    /// Returns `PhantomNode` with publish/receive channels.
    pub async fn start() -> anyhow::Result<Self> {
        let (publish_tx, mut publish_rx) = mpsc::channel::<Envelope>(256);
        let (receive_tx, receive_rx) = mpsc::channel::<Envelope>(256);

        let topic = IdentTopic::new(PHANTOM_TOPIC);
        let topic_clone = topic.clone();

        let mut swarm = SwarmBuilder::with_new_identity()
            .with_tokio()
            .with_tcp(
                tcp::Config::default(),
                noise::Config::new,
                yamux::Config::default,
            )?
            .with_behaviour(|key| {
                // GossipSub — message-based pub/sub mesh
                let gossipsub_config = GossipsubConfigBuilder::default()
                    .heartbeat_interval(Duration::from_secs(10))
                    .validation_mode(ValidationMode::Anonymous)
                    .message_id_fn(|msg| {
                        // Use first 8 bytes of SHA-256 as message ID (dedup)
                        use sha2::{Sha256, Digest};
                        let hash = Sha256::digest(&msg.data);
                        gossipsub::MessageId::from(&hash[..8])
                    })
                    .build()
                    .map_err(|e| anyhow::anyhow!("GossipSub config: {}", e))?;

                let gossipsub = Gossipsub::new(
                    MessageAuthenticity::Anonymous,
                    gossipsub_config,
                ).map_err(|e| anyhow::anyhow!("GossipSub init: {}", e))?;

                // Kademlia DHT for peer discovery
                let kademlia = Kademlia::new(
                    key.public().to_peer_id(),
                    MemoryStore::new(key.public().to_peer_id()),
                );

                // mDNS for LAN discovery
                let mdns = Mdns::new(mdns::Config::default(), key.public().to_peer_id())?;

                // Identify protocol
                let identify = Identify::new(
                    identify::Config::new("/phantom/1.0.0".to_string(), key.public())
                );

                Ok(PhantomBehaviour { gossipsub, kademlia, mdns, identify })
            })?
            .build();

        swarm.behaviour_mut().gossipsub.subscribe(&topic_clone)
            .map_err(|e| anyhow::anyhow!("subscribe failed: {:?}", e))?;

        // Listen on all interfaces
        swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse::<Multiaddr>()?)?;

        let local_peer_id = *swarm.local_peer_id();
        info!("PhantomChat P2P node started: {}", local_peer_id);

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    // Outbound: publish envelope to gossipsub
                    Some(env) = publish_rx.recv() => {
                        let bytes = env.to_bytes();
                        match swarm.behaviour_mut().gossipsub.publish(topic.clone(), bytes) {
                            Ok(id) => debug!("Published envelope, msg_id: {:?}", id),
                            Err(e) => warn!("GossipSub publish error: {:?}", e),
                        }
                    }
                    // Inbound: swarm events
                    event = swarm.select_next_some() => {
                        match event {
                            SwarmEvent::Behaviour(PhantomBehaviourEvent::Gossipsub(
                                GossipsubEvent::Message { message, .. }
                            )) => {
                                if let Some(env) = Envelope::from_bytes(&message.data) {
                                    let _ = receive_tx.send(env).await;
                                }
                            }
                            SwarmEvent::Behaviour(PhantomBehaviourEvent::Mdns(
                                mdns::Event::Discovered(peers)
                            )) => {
                                for (peer_id, addr) in peers {
                                    info!("mDNS discovered: {} at {}", peer_id, addr);
                                    swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                                    swarm.behaviour_mut().kademlia.add_address(&peer_id, addr);
                                }
                            }
                            SwarmEvent::NewListenAddr { address, .. } => {
                                info!("P2P listening on: {}", address);
                            }
                            _ => {}
                        }
                    }
                }
            }
        });

        Ok(Self { publish_tx, receive_rx, local_peer_id })
    }

    /// Connect to a known bootstrap peer (e.g. shared in the Phantom ID).
    pub async fn connect(&mut self, addr: Multiaddr) -> anyhow::Result<()> {
        self.publish_tx.send(
            // Hack: we signal the background task via a special message
            // In production this would be a separate command channel
            Envelope::from_bytes(&[]).unwrap_or_else(|| panic!("bootstrap signal"))
        ).await.ok();
        Ok(())
    }

    /// Publish an envelope to the P2P mesh.
    pub async fn publish(&self, env: Envelope) -> anyhow::Result<()> {
        self.publish_tx.send(env).await
            .map_err(|_| anyhow::anyhow!("P2P node channel closed"))
    }

    /// Receive the next envelope from the mesh (blocks until available).
    pub async fn recv(&mut self) -> Option<Envelope> {
        self.receive_rx.recv().await
    }
}
