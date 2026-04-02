use libp2p::{
    futures::StreamExt,
    gossipsub,
    kad,
    swarm::{NetworkBehaviour, SwarmEvent},
    PeerId, Swarm,
};
use serde::{Deserialize, Serialize};
use std::time::Duration;
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
}

#[derive(NetworkBehaviour)]
pub struct PhantomBehaviour {
    pub gossipsub: gossipsub::Behaviour,
    pub kademlia: kad::Behaviour<kad::store::MemoryStore>,
}

pub async fn run_swarm(
    mut swarm: Swarm<PhantomBehaviour>,
    mut command_rx: mpsc::Receiver<NetworkCommand>,
    event_sink: flutter_rust_bridge::StreamSink<NetworkEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    loop {
        tokio::select! {
            event = swarm.select_next_some() => {
                match event {
                    SwarmEvent::NewListenAddr { address, .. } => {
                        println!("Local node is listening on {:?}", address);
                    }
                    SwarmEvent::Behaviour(PhantomBehaviourEvent::Kademlia(kad::Event::RoutingUpdated { peer, .. })) => {
                        let _ = event_sink.add(NetworkEvent::PeerDiscovered { 
                            peer_id: peer.to_string(), 
                            avatar_cid: None 
                        });
                    }
                    SwarmEvent::Behaviour(PhantomBehaviourEvent::Gossipsub(gossipsub::Event::Message { propagation_source, message_id: _, message })) => {
                         let msg_str = String::from_utf8_lossy(&message.data).to_string();
                         let topic_str = message.topic.to_string();
                         
                         if topic_str == "phantom-chat" {
                            let _ = event_sink.add(NetworkEvent::MessageReceived { from: propagation_source.to_string(), message: msg_str });
                         } else if topic_str.starts_with("phantom-group-") {
                            let group_id = topic_str.replace("phantom-group-", "");
                            let _ = event_sink.add(NetworkEvent::GroupMessageReceived { group_id, from: propagation_source.to_string(), message: msg_str });
                         }
                    }
                    _ => {}
                }
            }
            Some(command) = command_rx.recv() => {
                match command {
                    NetworkCommand::PublishIdentity { phantom_id, avatar_cid } => {
                        let record_data = IdentityRecord { peer_id: swarm.local_peer_id().to_string(), avatar_cid };
                        let record_json = serde_json::to_vec(&record_data).unwrap();
                        let key = kad::RecordKey::new(&phantom_id.as_bytes());
                        let record = kad::Record { key, value: record_json, publisher: None, expires: None };
                        let _ = swarm.behaviour_mut().kademlia.put_record(record, kad::Quorum::One);
                    }
                    NetworkCommand::SendMessage { target_peer_id: _, message } => {
                         let topic = gossipsub::IdentTopic::new("phantom-chat");
                         let _ = swarm.behaviour_mut().gossipsub.publish(topic, message.as_bytes());
                    }
                    NetworkCommand::JoinGroup { group_id } => {
                         let topic = gossipsub::IdentTopic::new(format!("phantom-group-{}", group_id));
                         let _ = swarm.behaviour_mut().gossipsub.subscribe(&topic);
                    }
                    NetworkCommand::SendGroupMessage { group_id, message } => {
                         let topic = gossipsub::IdentTopic::new(format!("phantom-group-{}", group_id));
                         let _ = swarm.behaviour_mut().gossipsub.publish(topic, message.as_bytes());
                    }
                }
            }
        }
    }
}
