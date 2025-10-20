use std::{error::Error, fmt::Display, sync::Arc};

use futures::{channel::oneshot, future::BoxFuture};
use iroh::{
    protocol::{self, DynProtocolHandler, ProtocolHandler}, NodeId
};
use rand::rand_core::le;
use tokio::sync::{mpsc::{UnboundedReceiver, UnboundedSender}};


use crate::{connection::{Connecting, Connection}, helper};

#[derive(Debug)]
pub struct Transport {
    secret_key: iroh::SecretKey,
    pub(crate) node_id: iroh::NodeId,
    pub(crate) peer_id: libp2p_core::PeerId,

    transport_events_rx: UnboundedReceiver<libp2p_core::transport::TransportEvent<Connecting, TransportError>>,
    transport_events_tx: UnboundedSender<libp2p_core::transport::TransportEvent<Connecting, TransportError>>,
    new_conns_tx: UnboundedSender<iroh::endpoint::Connection>,
    _new_conns_rx: Option<UnboundedReceiver<iroh::endpoint::Connection>>,
    transport_protocol: Option<TransportProtocol>,
}

#[derive(Debug, Clone)]
pub struct TransportProtocol {
    new_conns_rx: Arc<UnboundedReceiver<iroh::endpoint::Connection>>,
    _new_conns_tx: UnboundedSender<iroh::endpoint::Connection>,
    _router: Option<iroh::protocol::Router>,
}

#[derive(Clone, Debug)]
pub struct TransportError {
    kind: TransportErrorKind,
}

#[derive(Clone, Debug)]
pub enum TransportErrorKind {
    Dial(String),
    Listen(String),
}

impl Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TransportError: {:?}", self.kind)
    }
}

impl From<iroh::endpoint::BindError> for TransportError {
    fn from(err: iroh::endpoint::BindError) -> Self {
        Self {
            kind: TransportErrorKind::Listen(err.to_string()),
        }
    }
}

impl From<&str> for TransportError {
    fn from(err: &str) -> Self {
        Self {
            kind: TransportErrorKind::Listen(err.to_string()),
        }
    }
}

impl Transport {
    const ALPN: &'static [u8] = b"/iroh/libp2p-transport/0.0.1";

    pub async fn new(keypair: Option<&libp2p_identity::Keypair>) -> Result<Self, TransportError> {
        let (new_conns_tx, new_conns_rx) = tokio::sync::mpsc::unbounded_channel();
        let (transport_events_tx, transport_events_rx) = tokio::sync::mpsc::unbounded_channel();
        
        let keypair = if let Some(kp) = keypair {
            kp.clone()
        } else {
            let secret_key_temp = iroh::SecretKey::generate(&mut rand::rng());
            let ed25519_keypair = libp2p_identity::ed25519::Keypair::try_from_bytes(&mut secret_key_temp.to_bytes().to_vec())
                .map_err(|e| TransportError {
                    kind: TransportErrorKind::Listen(format!("Failed to create libp2p ed25519 keypair: {e}")),
                })?;
            ed25519_keypair.into()
        };
        let (secret_key, peer_id) = helper::libp2p_keypair_to_iroh_secret(&keypair)
            .map(|sk| (sk, libp2p_core::PeerId::from(keypair.public())))
            .ok_or_else(|| TransportError {
                kind: TransportErrorKind::Listen("Failed to convert libp2p keypair to iroh secret key".to_string()),
            })?;
        Ok(Self {
            new_conns_tx,
            _new_conns_rx: Some(new_conns_rx),
            transport_protocol: None,
            transport_events_tx,
            transport_events_rx,
            secret_key: secret_key.clone(),
            node_id: secret_key.public(),
            peer_id,
        })
    }
}

impl TransportProtocol {
    pub(in crate::transport) fn set_router(&mut self, router: iroh::protocol::Router) {
        self._router = Some(router);
    }
}

impl Error for TransportError {}

impl libp2p_core::Transport for Transport {
    type Output = Connection;

    type Error = TransportError;

    type ListenerUpgrade = Connecting;

    type Dial = BoxFuture<'static, Result<Self::Output, Self::Error>>;

    fn listen_on(
        &mut self,
        id: libp2p_core::transport::ListenerId,
        _addr: libp2p_core::Multiaddr,
    ) -> Result<(), libp2p_core::transport::TransportError<Self::Error>> {

        // /iroh/[node-id]
        let iroh_addr = helper::iroh_node_id_to_multiaddr(&self.node_id);
        let secret_key = self.secret_key.clone();
        
        let (waiter_tx, mut waiter_rx) = tokio::sync::mpsc::channel(1);
        let transport_events_tx = self.transport_events_tx.clone();
        
        tokio::spawn(async move {
            let endpoint = iroh::Endpoint::builder()
                .secret_key(secret_key)
                .discovery_n0()
                .bind()
                .await
                .map_err(|e| TransportError {
                    kind: TransportErrorKind::Listen(e.to_string()),
                })?;

            let (new_conns_tx, new_conns_rx) = tokio::sync::mpsc::unbounded_channel();

            let mut protocol = TransportProtocol {
                new_conns_rx: Arc::new(new_conns_rx),
                _new_conns_tx: new_conns_tx.clone(),
                _router: None,
            };

            protocol.set_router(
                iroh::protocol::Router::builder(endpoint.clone())
                    .accept(Transport::ALPN, protocol.clone())
                    .spawn(),
            );
            
            let _ = waiter_tx.send(protocol).await;

            let _ = transport_events_tx.send(libp2p_core::transport::TransportEvent::NewAddress { listener_id: id, listen_addr: iroh_addr });

            Ok::<(), TransportError>(())
        });
        
        if let Some(protocol) = waiter_rx.blocking_recv() {
            self.transport_protocol = Some(protocol);
        }
        
        Ok(())
    }

    fn remove_listener(&mut self, id: libp2p_core::transport::ListenerId) -> bool {
        todo!()
    }

    fn dial(
        &mut self,
        addr: libp2p_core::Multiaddr,
        opts: libp2p_core::transport::DialOpts,
    ) -> Result<Self::Dial, libp2p_core::transport::TransportError<Self::Error>> {
        todo!()
    }

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<libp2p_core::transport::TransportEvent<Self::ListenerUpgrade, Self::Error>>
    {
        let this = self.get_mut();
        match this.transport_events_rx.poll_recv(cx) {
            std::task::Poll::Ready(Some(event)) => std::task::Poll::Ready(event),
            std::task::Poll::Ready(None) => std::task::Poll::Pending,
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }
}

impl ProtocolHandler for TransportProtocol {
    async fn accept(
        &self,
        connection: iroh::endpoint::Connection,
    ) -> Result<(), iroh::protocol::AcceptError> {
        self._new_conns_tx.send(connection).map_err(|e| {
            iroh::protocol::AcceptError::from_err(std::io::Error::other(
                format!("failed to send new connection: {e}"),
            ))
        })
    }
}
