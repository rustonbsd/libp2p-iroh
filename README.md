# libp2p-iroh

A libp2p transport using [iroh](https://github.com/n0-computer/iroh) QUIC that works behind any NAT without port forwarding, UPnP, or manual configuration.

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
libp2p-iroh = "0.0.1"
```

## Usage

Replace your existing transport with libp2p-iroh. That's it.
Here's a minimal example using libp2p-kad with libp2p-iroh transport:

```rust
use libp2p::Multiaddr;
use libp2p::StreamProtocol;
use libp2p_kad::store::MemoryStore;
use libp2p_swarm::NetworkBehaviour;
use libp2p_swarm::Swarm;

use libp2p_iroh::Transport;
use libp2p_iroh::TransportTrait;
use libp2p_swarm::SwarmEvent;

#[derive(NetworkBehaviour)]
struct MyBehaviour {
    kademlia: libp2p_kad::Behaviour<MemoryStore>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let keypair = libp2p_identity::Keypair::generate_ed25519();
    let peer_id = keypair.public().to_peer_id();

    let transport = Transport::new(Some(&keypair)).await?.boxed();

    println!("Copy and paste this in a second terminal, press enter to connect back to this node from anywhere:");
    println!("  /p2p/{peer_id}");

    let kad_config = libp2p_kad::Config::new(StreamProtocol::new("/example/kad/1.0.0"));
    let store = MemoryStore::new(peer_id);
    let behaviour = MyBehaviour {
        kademlia: libp2p_kad::Behaviour::with_config(peer_id, store, kad_config),
    };

    let mut swarm = Swarm::new(
        transport,
        behaviour,
        peer_id,
        libp2p_swarm::Config::with_executor(Box::new(|fut| {
            tokio::spawn(fut);
        })).with_idle_connection_timeout(std::time::Duration::from_secs(300)),
    );

    // Our listener address looks like this: /p2p/12D3KooWEUowGZ...
    swarm.listen_on(Multiaddr::empty())?;

    // Mini cli to dial other peers
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        print!("> ");
        let mut stdin = std::io::stdin().lock();
        let mut line = String::new();
        if std::io::BufRead::read_line(&mut stdin, &mut line).is_ok() && !line.is_empty() {
            if let Ok(peer_multiaddr) = line.trim().parse::<Multiaddr>() {
                tx.send(peer_multiaddr).unwrap();
            };
        }
    });

    loop {
        tokio::select! {
            event = futures::StreamExt::select_next_some(&mut swarm) => {
                if let SwarmEvent::ConnectionEstablished { peer_id, .. } = event {
                    println!("Connection established with {peer_id}!");
                }
            }
            Some(addr) = rx.recv() => {
                println!("Dialing {addr}...");
                swarm.dial(addr)?;
            }
        }
    }
}
```

## Examples

Run the included swarm_dht example for a full demo:

```bash
# Terminal 1
cargo run --example swarm_dht

# Terminal 2 
cargo run --example swarm_dht
# Copy the multiaddr from terminal 1 and dial it
```

Both peers can be behind different NATs. The connection will establish successfully.

## What This Solves

If you're building with libp2p, you know NAT traversal is complicated. You need:
- Multiple transport implementations
- Relay servers
- AutoNAT protocol
- DCUtR for hole punching
- Careful configuration

With libp2p-iroh, you get reliable peer-to-peer connections behind any firewall out of the box. No relay configuration. No hole punching setup. Just dial a peer by their PeerId and it works.

## How It Works

The transport uses iroh's built-in relay network and NAT traversal. When you dial a peer, iroh handles:
- Direct connections when possible
- Relay routing as fallback if (immediate) direct connections fail
- Automatic hole punching
- Connection upgrades from relayed to direct

You don't configure any of this. It just works with the magic of [iroh](https://github.com/n0-computer/iroh).


### Multiaddr Format

Peers are addressed using iroh's node ID format:

```
/p2p/12D3KooWAbCdEf...  # Standard libp2p PeerId format
```

The transport automatically handles the conversion between libp2p PeerIds and iroh NodeIds.

## Features

- `swarm` (default): Includes libp2p-swarm and libp2p-kad dependencies for the examples.

Disable default features if you only need the transport:

```toml
libp2p-iroh = { version = "0.0.1", default-features = false }
```

## Technical Details

- Uses iroh's QUIC implementation (based on quinn)
- Leverages iroh's relay protocol for guaranteed connectivity
- Supports ed25519 keypairs compatible with libp2p
- Implements libp2p's Transport trait
- Connection multiplexing via QUIC streams

## Status

Working work in progress. Contributions welcome!

## License

MIT