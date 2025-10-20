use std::str::FromStr;

use iroh::NodeId;
use libp2p_core::Multiaddr;

pub(crate)fn multiaddr_to_iroh_node_id(
    addr: &Multiaddr,
) -> Option<iroh::NodeId> {

    let addr_str = addr.to_string();
    
    if let Some(iroh_part) = addr_str.strip_prefix("/iroh/") {
        if let Some(node_id_str) = iroh_part.split('/').next() {
            return NodeId::from_str(node_id_str).ok()
        }
    }

    // Fallback: try parse ed25519 peer id if present
    for protocol in addr.iter() {
        if let libp2p_core::multiaddr::Protocol::P2p(peer_id) = protocol {
            if let Some(node_id) = peer_id_to_node_id(&peer_id) {
                return Some(node_id);
            }
        }
    }

    None
}

pub(crate) fn peer_id_to_node_id(
    peer_id: &libp2p_core::PeerId,
) -> Option<iroh::NodeId> {
    let bytes = peer_id.to_bytes();
    if let Ok(byte_array) = <[u8; 32]>::try_from(bytes.as_slice()) {
        if let Ok(node_id) = iroh::NodeId::from_bytes(&byte_array) {
            return Some(node_id);
        }
    }
    None
}

pub(crate) fn libp2p_keypair_to_iroh_secret(keypair: &libp2p_identity::Keypair) -> Option<iroh::SecretKey> {
    if let Ok(ed25519) = keypair.clone().try_into_ed25519() {
        let secret = ed25519.secret();
        let secret_key = iroh::SecretKey::from_bytes(secret.as_ref().try_into().ok()?);
        return Some(secret_key);
    }
    None
}

pub(crate) fn iroh_node_id_to_multiaddr(node_id: &NodeId) -> Multiaddr {
    format!("/iroh/{node_id}").parse::<Multiaddr>()
        .expect("Failed to parse custom multiaddr")
}