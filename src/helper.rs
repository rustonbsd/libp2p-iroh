use iroh::NodeId;
use libp2p_core::Multiaddr;

pub(crate) fn multiaddr_to_iroh_node_id(addr: &Multiaddr) -> Option<iroh::NodeId> {
    // Try to extract node_id from /p2p/ protocol component
    for protocol in addr.iter() {
        if let libp2p_core::multiaddr::Protocol::P2p(peer_id) = protocol {
            if let Some(node_id) = peer_id_to_node_id(&peer_id) {
                return Some(node_id);
            } else {
                println!("Failed to convert PeerId to NodeId");
            }
        }
    }

    None
}

pub(crate) fn peer_id_to_node_id(peer_id: &libp2p_core::PeerId) -> Option<iroh::NodeId> {
    let bytes = peer_id.to_bytes();
    if bytes.len() != 38 {
        return None;
    }
    if let Ok(byte_array) = <[u8; 32]>::try_from(&bytes[6..]) {
        if let Ok(node_id) = iroh::NodeId::from_bytes(&byte_array) {
            return Some(node_id);
        }
    }
    None
}

pub(crate) fn libp2p_keypair_to_iroh_secret(
    keypair: &libp2p_identity::Keypair,
) -> Option<iroh::SecretKey> {
    if let Ok(ed25519) = keypair.clone().try_into_ed25519() {
        let secret = ed25519.secret();
        let secret_key = iroh::SecretKey::from_bytes(secret.as_ref().try_into().ok()?);
        return Some(secret_key);
    }
    None
}

pub fn iroh_node_id_to_multiaddr(node_id: &NodeId) -> Multiaddr {
    let mut addr = Multiaddr::empty();

    addr.push(libp2p_core::multiaddr::Protocol::P2p(
        libp2p_identity::ed25519::PublicKey::try_from_bytes(node_id.as_bytes())
            .map(|pk| libp2p_core::PeerId::from_public_key(&libp2p_identity::PublicKey::from(pk)))
            .expect("Failed to convert iroh NodeId to libp2p PeerId"),
    ));

    addr
}
