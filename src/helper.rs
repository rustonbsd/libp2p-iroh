use iroh::EndpointId;
use libp2p::Multiaddr;

pub(crate) fn multiaddr_to_iroh_node_id(addr: &Multiaddr) -> Option<EndpointId> {
    tracing::debug!(
        "helper::multiaddr_to_iroh_node_id - Converting multiaddr: {}",
        addr
    );
    // Try to extract node_id from /p2p/ protocol component
    for protocol in addr.iter() {
        if let libp2p_core::multiaddr::Protocol::P2p(peer_id) = protocol {
            tracing::debug!(
                "helper::multiaddr_to_iroh_node_id - Found P2p protocol with peer_id: {}",
                peer_id
            );
            if let Some(node_id) = peer_id_to_node_id(&peer_id) {
                tracing::debug!(
                    "helper::multiaddr_to_iroh_node_id - Converted to EndpointId: {:?}",
                    node_id
                );
                return Some(node_id);
            } else {
                tracing::warn!(
                    "helper::multiaddr_to_iroh_node_id - Failed to convert PeerId to EndpointId"
                );
                println!("Failed to convert PeerId to EndpointId");
            }
        }
    }

    tracing::warn!("helper::multiaddr_to_iroh_node_id - No valid P2p protocol found in multiaddr");
    None
}

pub(crate) fn peer_id_to_node_id(peer_id: &libp2p_core::PeerId) -> Option<EndpointId> {
    tracing::debug!(
        "helper::peer_id_to_node_id - Converting PeerId: {}",
        peer_id
    );
    let bytes = peer_id.to_bytes();
    tracing::debug!(
        "helper::peer_id_to_node_id - PeerId bytes length: {}",
        bytes.len()
    );
    if bytes.len() != 38 {
        tracing::warn!(
            "helper::peer_id_to_node_id - Invalid byte length: expected 38, got {}",
            bytes.len()
        );
        return None;
    }
    if let Ok(byte_array) = <[u8; 32]>::try_from(&bytes[6..]) {
        if let Ok(node_id) = EndpointId::from_bytes(&byte_array) {
            tracing::debug!(
                "helper::peer_id_to_node_id - Successfully converted to EndpointId: {:?}",
                node_id
            );
            return Some(node_id);
        } else {
            tracing::warn!("helper::peer_id_to_node_id - Failed to create EndpointId from bytes");
        }
    } else {
        tracing::warn!(
            "helper::peer_id_to_node_id - Failed to extract 32-byte array from PeerId bytes"
        );
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

pub fn iroh_node_id_to_multiaddr(node_id: &EndpointId) -> Multiaddr {
    tracing::debug!(
        "helper::iroh_node_id_to_multiaddr - Converting EndpointId: {:?}",
        node_id
    );
    let mut addr = Multiaddr::empty();
    addr.push(libp2p_core::multiaddr::Protocol::P2p(
        libp2p_identity::ed25519::PublicKey::try_from_bytes(node_id.as_bytes())
            .map(|pk| {
                let peer_id =
                    libp2p_core::PeerId::from_public_key(&libp2p_identity::PublicKey::from(pk));
                tracing::debug!(
                    "helper::iroh_node_id_to_multiaddr - Converted to PeerId: {}",
                    peer_id
                );
                peer_id
            })
            .expect("Failed to convert iroh EndpointId to libp2p PeerId"),
    ));

    tracing::debug!(
        "helper::iroh_node_id_to_multiaddr - Created multiaddr: {}",
        addr
    );
    addr
}

pub fn node_id_to_peerid(node_id: &EndpointId) -> Option<libp2p::PeerId> {
    let pubkey_bytes = node_id.to_vec();
    let libp2p_pubkey = libp2p_identity::ed25519::PublicKey::try_from_bytes(pubkey_bytes.as_slice()).ok()?;

    Some(libp2p_core::PeerId::from_public_key(
        &libp2p_identity::PublicKey::from(libp2p_pubkey),
    ))
}
