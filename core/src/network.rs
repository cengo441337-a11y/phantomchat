use libp2p::{
    futures::StreamExt,
    gossipsub,
    kad,
    swarm::{NetworkBehaviour, SwarmEvent},
    Swarm,
};
use crate::dandelion::{DandelionRouter, Phase};
use crate::frb_generated::StreamSink;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct IdentityRecord {
    pub peer_id: String,
    pub avatar_cid: Option<String>,
}

#[derive(Debug, Clone)]
pub enum NetworkEvent {
    NodeStarted { peer_id: String },
    PeerDiscovered { peer_id: String, avatar_cid: Option<String> },
    MessageReceived { from: String, message: String },
    GroupMessageReceived { group_id: String, from: String, message: String },
    Error { message: String },
}

#[derive(Debug)]
pub enum NetworkCommand {
    PublishIdentity { phantom_id: String, avatar_cid: Option<String> },
    SendMessage { target_peer_id: String, message: String },
    JoinGroup { group_id: String },
    SendGroupMessage { group_id: String, message: String },
    /// Publish raw bytes directly onto a GossipSub topic — used for cover traffic.
    PublishRaw { topic: String, data: Vec<u8> },
}

#[derive(NetworkBehaviour)]
pub struct PhantomBehaviour {
    pub gossipsub: gossipsub::Behaviour,
    pub kademlia: kad::Behaviour<kad::store::MemoryStore>,
}

/// GossipSub topic used for Dandelion++ stem-phase forwarding.
/// Only the designated stem peer subscribes to this topic in their epoch.
const STEM_TOPIC_PREFIX: &str = "phantom-stem-";

pub async fn run_swarm(
    mut swarm: Swarm<PhantomBehaviour>,
    mut command_rx: mpsc::Receiver<NetworkCommand>,
    event_sink: StreamSink<NetworkEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut dandelion = DandelionRouter::new();

    loop {
        tokio::select! {
            event = swarm.select_next_some() => {
                match event {
                    SwarmEvent::NewListenAddr { address, .. } => {
                        println!("Local node is listening on {:?}", address);
                    }

                    SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                        dandelion.add_peer(peer_id);
                        tracing::debug!("Peer connected: {}", peer_id);
                    }

                    SwarmEvent::ConnectionClosed { peer_id, .. } => {
                        dandelion.remove_peer(&peer_id);
                        tracing::debug!("Peer disconnected: {}", peer_id);
                    }

                    SwarmEvent::Behaviour(PhantomBehaviourEvent::Kademlia(
                        kad::Event::RoutingUpdated { peer, .. }
                    )) => {
                        let _ = event_sink.add(NetworkEvent::PeerDiscovered {
                            peer_id: peer.to_string(),
                            avatar_cid: None,
                        });
                    }

                    SwarmEvent::Behaviour(PhantomBehaviourEvent::Gossipsub(
                        gossipsub::Event::Message { propagation_source, message_id: _, message }
                    )) => {
                        let msg_str = String::from_utf8_lossy(&message.data).to_string();
                        let topic_str = message.topic.to_string();

                        if topic_str == "phantom-chat" {
                            let _ = event_sink.add(NetworkEvent::MessageReceived {
                                from: propagation_source.to_string(),
                                message: msg_str,
                            });
                        } else if topic_str.starts_with("phantom-group-") {
                            let group_id = topic_str.replace("phantom-group-", "");
                            let _ = event_sink.add(NetworkEvent::GroupMessageReceived {
                                group_id,
                                from: propagation_source.to_string(),
                                message: msg_str,
                            });
                        } else if topic_str.starts_with(STEM_TOPIC_PREFIX) {
                            // We are the designated stem peer for this epoch.
                            // Re-route: decide whether to stem further or fluff.
                            let phase = dandelion.route();
                            publish_with_phase(
                                &mut swarm,
                                &dandelion,
                                phase,
                                "phantom-chat",
                                message.data,
                            );
                        }
                    }

                    _ => {}
                }
            }

            Some(command) = command_rx.recv() => {
                match command {
                    NetworkCommand::PublishIdentity { phantom_id, avatar_cid } => {
                        let record_data = IdentityRecord {
                            peer_id: swarm.local_peer_id().to_string(),
                            avatar_cid,
                        };
                        let record_json = serde_json::to_vec(&record_data).unwrap();
                        let key = kad::RecordKey::new(&phantom_id.as_bytes());
                        let record = kad::Record {
                            key,
                            value: record_json,
                            publisher: None,
                            expires: None,
                        };
                        let _ = swarm
                            .behaviour_mut()
                            .kademlia
                            .put_record(record, kad::Quorum::One);
                    }

                    NetworkCommand::SendMessage { target_peer_id: _, message } => {
                        let phase = dandelion.route();
                        publish_with_phase(
                            &mut swarm,
                            &dandelion,
                            phase,
                            "phantom-chat",
                            message.into_bytes(),
                        );
                    }

                    NetworkCommand::JoinGroup { group_id } => {
                        let topic = gossipsub::IdentTopic::new(
                            format!("phantom-group-{}", group_id)
                        );
                        let _ = swarm.behaviour_mut().gossipsub.subscribe(&topic);
                    }

                    NetworkCommand::SendGroupMessage { group_id, message } => {
                        // Group messages always fluff — smaller groups have less
                        // cover so stem-phase savings are not worth the complexity.
                        let topic = gossipsub::IdentTopic::new(
                            format!("phantom-group-{}", group_id)
                        );
                        let _ = swarm
                            .behaviour_mut()
                            .gossipsub
                            .publish(topic, message.as_bytes());
                    }

                    NetworkCommand::PublishRaw { topic, data } => {
                        let t = gossipsub::IdentTopic::new(&topic);
                        if let Err(e) = swarm.behaviour_mut().gossipsub.publish(t, data) {
                            tracing::debug!("PublishRaw error (cover traffic): {:?}", e);
                        }
                    }
                }
            }
        }
    }
}

// ── helpers ────────────────────────────────────────────────────────────────────

/// Publish `data` using Dandelion++ phase decision.
///
/// - `Phase::Fluff` → standard GossipSub broadcast on `fluff_topic`
/// - `Phase::Stem`  → publish on the stem peer's private topic so only
///   they receive it and must re-route
fn publish_with_phase(
    swarm: &mut Swarm<PhantomBehaviour>,
    dandelion: &DandelionRouter,
    phase: Phase,
    fluff_topic: &str,
    data: Vec<u8>,
) {
    match phase {
        Phase::Fluff => {
            let topic = gossipsub::IdentTopic::new(fluff_topic);
            if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic, data) {
                tracing::warn!("Gossipsub publish (fluff) error: {:?}", e);
            } else {
                tracing::debug!("Dandelion++: FLUFF — broadcast on {}", fluff_topic);
            }
        }
        Phase::Stem => {
            if let Some(stem_peer) = dandelion.stem_peer() {
                // Publish on a stem-specific topic that only the stem peer
                // subscribes to.  If the peer has disconnected, fall back to fluff.
                let stem_topic_name = format!("{}{}", STEM_TOPIC_PREFIX, stem_peer);
                let stem_topic = gossipsub::IdentTopic::new(&stem_topic_name);
                if swarm.behaviour_mut().gossipsub.publish(stem_topic, data.clone()).is_err() {
                    // Stem failed — fall back to fluff rather than dropping the message.
                    tracing::debug!("Dandelion++: stem fallback → fluff (peer unreachable)");
                    let topic = gossipsub::IdentTopic::new(fluff_topic);
                    let _ = swarm.behaviour_mut().gossipsub.publish(topic, data);
                } else {
                    tracing::debug!("Dandelion++: STEM → peer {}", stem_peer);
                }
            } else {
                // No stem peer available — fluff immediately.
                let topic = gossipsub::IdentTopic::new(fluff_topic);
                let _ = swarm.behaviour_mut().gossipsub.publish(topic, data);
            }
        }
    }
}
