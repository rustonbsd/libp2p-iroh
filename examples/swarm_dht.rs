use futures::StreamExt;
use libp2p_core::{Multiaddr, Transport as CoreTransport, muxing::StreamMuxerBox, upgrade};
use libp2p_kad::{Behaviour as Kademlia, Config as KademliaConfig, Event as KademliaEvent, store::MemoryStore};
use libp2p_swarm::{NetworkBehaviour, Swarm, SwarmEvent};
use std::io::{self, BufRead, Write};
use std::str::FromStr;
use std::time::Duration;

#[derive(NetworkBehaviour)]
struct MyBehaviour {
    kademlia: Kademlia<MemoryStore>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let keypair = libp2p_identity::Keypair::generate_ed25519();
    let peer_id = keypair.public().to_peer_id();

    let transport = libp2p_iroh::Transport::new(Some(&keypair))
        .await?
        .map(move |conn, _| (peer_id, StreamMuxerBox::new(conn)))
        .boxed();

    println!("Local Peer ID: {peer_id}");

    let mut kad_config = KademliaConfig::default();
    kad_config.set_query_timeout(Duration::from_secs(60));

    let store = MemoryStore::new(peer_id);
    let mut kademlia = Kademlia::with_config(peer_id, store, kad_config);

    kademlia.set_mode(Some(libp2p_kad::Mode::Server));

    let behaviour = MyBehaviour { kademlia };

    let mut swarm = Swarm::new(
        transport,
        behaviour,
        peer_id,
        libp2p_swarm::Config::with_executor(Box::new(|fut| {
            tokio::spawn(fut);
        }))
        .with_idle_connection_timeout(Duration::from_secs(60)),
    );

    swarm.listen_on(Multiaddr::empty())?;

    println!("Swarm started. Enter commands:");
    println!("  <multiaddr> - Dial a peer");
    println!("  bootstrap   - Bootstrap the DHT");
    println!();

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

    let input_handle = tokio::spawn(async move {
        let stdin = io::stdin();
        let mut handle = stdin.lock();
        loop {
            print!("> ");
            io::stdout().flush().ok();
            let mut line = String::new();
            if handle.read_line(&mut line).is_ok()
                && !line.is_empty()
                && tx.send(line.trim().to_string()).is_err()
            {
                break;
            }
        }
    });

    loop {
        tokio::select! {
            event = swarm.next() => {
                if let Some(event) = event {
                    match event {
                        SwarmEvent::NewListenAddr { address, .. } => {
                            println!("Listening on: {address}");
                        }
                        SwarmEvent::Behaviour(MyBehaviourEvent::Kademlia(kad_event)) => {
                            match kad_event {
                                KademliaEvent::RoutingUpdated { peer, .. } => {
                                    println!("Routing updated for peer: {peer}");
                                }
                                KademliaEvent::InboundRequest { request } => {
                                    println!("Inbound request: {request:?}");
                                }
                                KademliaEvent::OutboundQueryProgressed { result, .. } => {
                                    println!("Query progressed: {result:?}");
                                }
                                _ => {}
                            }
                        }
                        SwarmEvent::ConnectionEstablished { peer_id, endpoint, .. } => {
                            println!("Connection established with {peer_id} at {endpoint:?}");
                            swarm.behaviour_mut().kademlia.add_address(&peer_id, endpoint.get_remote_address().clone());
                        }
                        SwarmEvent::ConnectionClosed { peer_id, cause, .. } => {
                            println!("Connection closed with {peer_id}: {cause:?}");
                        }
                        SwarmEvent::IncomingConnection { send_back_addr, .. } => {
                            println!("Incoming connection from {send_back_addr}");
                        }
                        SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                            eprintln!("Outgoing connection error to {peer_id:?}: {error}");
                        }
                        SwarmEvent::IncomingConnectionError { send_back_addr, error, .. } => {
                            eprintln!("Incoming connection error from {send_back_addr}: {error}");
                        }
                        _ => {}
                    }
                }
            }
            Some(cmd) = rx.recv() => {
                if cmd == "bootstrap" {
                    if let Err(e) = swarm.behaviour_mut().kademlia.bootstrap() {
                        eprintln!("Bootstrap error: {e}");
                    } else {
                        println!("Bootstrap initiated");
                    }
                } else if let Ok(addr) = Multiaddr::from_str(&cmd) {
                    println!("Dialing: {addr}");
                    if let Err(e) = swarm.dial(addr) {
                        eprintln!("Dial error: {e}");
                    }
                } else {
                    eprintln!("Unknown command: {cmd}");
                }
            }
        }
    }
}


