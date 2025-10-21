use std::fmt::Display;

use actor_helper::{Action, Actor, ActorError, Handle, Receiver, act_ok};
use futures::{FutureExt, future::BoxFuture};
use iroh::protocol::ProtocolHandler;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::{
    connection::{Connecting, Connection},
    helper,
};

#[derive(Debug)]
pub struct Transport {
    _secret_key: iroh::SecretKey,
    protocol: Protocol,

    pub node_id: iroh::NodeId,
    pub peer_id: libp2p_core::PeerId,

    pub timeout: std::time::Duration,
    transport_events_rx:
        UnboundedReceiver<libp2p_core::transport::TransportEvent<Connecting, TransportError>>,
    transport_events_tx:
        UnboundedSender<libp2p_core::transport::TransportEvent<Connecting, TransportError>>,
}

#[derive(Debug, Clone)]
pub struct Protocol {
    api: Handle<ProtocolActor, TransportError>,
}

#[derive(Debug)]
struct ProtocolActor {
    rx: Receiver<Action<ProtocolActor>>,

    listener_id: Option<libp2p_core::transport::ListenerId>,
    endpoint: iroh::Endpoint,
    _router: Option<iroh::protocol::Router>,
    transport_tx:
        UnboundedSender<libp2p_core::transport::TransportEvent<Connecting, TransportError>>,
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

impl std::error::Error for TransportError {}

impl Transport {
    pub async fn new(keypair: Option<&libp2p_identity::Keypair>) -> Result<Self, TransportError> {
        let (transport_events_tx, transport_events_rx) = tokio::sync::mpsc::unbounded_channel();

        let (secret_key, peer_id) = if let Some(kp) = keypair {
            let sk = helper::libp2p_keypair_to_iroh_secret(kp).ok_or_else(|| TransportError {
                kind: TransportErrorKind::Listen(
                    "Failed to convert libp2p keypair to iroh secret key".to_string(),
                ),
            })?;
            let pid = libp2p_core::PeerId::from(kp.public());
            (sk, pid)
        } else {
            let sk = iroh::SecretKey::generate(&mut rand::rng());
            let node_id = sk.public();
            let node_id_bytes = node_id.as_bytes();
            let ed25519_pubkey = libp2p_identity::ed25519::PublicKey::try_from_bytes(node_id_bytes)
                .map_err(|e| TransportError {
                    kind: TransportErrorKind::Listen(format!(
                        "Failed to create libp2p public key from iroh node id: {e}"
                    )),
                })?;
            let libp2p_pubkey = libp2p_identity::PublicKey::from(ed25519_pubkey);
            let pid = libp2p_core::PeerId::from_public_key(&libp2p_pubkey);
            (sk, pid)
        };

        let (waiter_tx, mut waiter_rx) = tokio::sync::mpsc::channel(1);

        tokio::spawn({
            let transport_events_tx = transport_events_tx.clone();
            let secret_key = secret_key.clone();
            async move {
                if let Ok(endpoint) = iroh::Endpoint::builder()
                    .secret_key(secret_key)
                    .discovery_n0()
                    .bind()
                    .await
                    .map_err(|e| TransportError {
                        kind: TransportErrorKind::Listen(e.to_string()),
                    })
                {
                    let protocol = Protocol::new(endpoint.clone(), transport_events_tx);

                    if waiter_tx.send(Ok(protocol)).await.is_ok() {
                        return;
                    }
                }

                waiter_tx
                    .send(Err(TransportError {
                        kind: TransportErrorKind::Listen(
                            "Failed to initialize iroh endpoint".to_string(),
                        ),
                    }))
                    .await
                    .expect("fatal: failed to send error through channel");
            }
        });

        let protocol = waiter_rx.recv().await.ok_or_else(|| TransportError {
            kind: TransportErrorKind::Listen(
                "Failed to receive transport from initialization".to_string(),
            ),
        })??;

        Ok(Transport {
            transport_events_tx,
            transport_events_rx,
            _secret_key: secret_key.clone(),
            node_id: secret_key.public(),
            peer_id,
            timeout: std::time::Duration::from_secs(20),
            protocol,
        })
    }
}

impl Protocol {
    const ALPN: &'static [u8] = b"/iroh/libp2p-transport/0.0.1";
    pub fn new(
        endpoint: iroh::Endpoint,
        transport_tx: UnboundedSender<
            libp2p_core::transport::TransportEvent<Connecting, TransportError>,
        >,
    ) -> Self {
        let (api, rx) = Handle::channel();

        tokio::spawn(async move {
            let mut actor = ProtocolActor {
                rx,
                transport_tx,
                endpoint,
                _router: None,
                listener_id: None,
            };
            if let Err(e) = actor.run().await {
                eprintln!("TransportProtocolActor error: {e}");
            }
        });

        Self { api }
    }
}

impl ActorError for TransportError {
    fn from_actor_message(msg: String) -> Self {
        TransportError {
            kind: TransportErrorKind::Listen(msg),
        }
    }
}

impl Actor<TransportError> for ProtocolActor {
    async fn run(&mut self) -> Result<(), TransportError> {
        loop {
            tokio::select! {
                Ok(action) = self.rx.recv_async() => {
                    action(self).await;
                }
            }
        }
    }
}

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
        let listener_id = self
            .protocol
            .api
            .call_blocking(act_ok!(actor => async move { actor.listener_id }))
            .map_err(libp2p_core::transport::TransportError::Other)?;
        if listener_id.is_some() {
            return Err(libp2p_core::transport::TransportError::Other(
                TransportError {
                    kind: TransportErrorKind::Listen(
                        "Listener already exists for this transport".to_string(),
                    ),
                },
            ));
        }

        let endpoint = self
            .protocol
            .api
            .call_blocking(act_ok!(actor => async move { actor.endpoint.clone() }))
            .map_err(|e| {
                libp2p_core::transport::TransportError::Other(TransportError {
                    kind: TransportErrorKind::Listen(format!(
                        "Failed to get endpoint from transport protocol: {e}"
                    )),
                })
            })?;
        let _router = iroh::protocol::Router::builder(endpoint.clone())
            .accept(Protocol::ALPN, self.protocol.clone())
            .spawn();
        self.protocol
            .api
            .call_blocking(act_ok!(actor => async move {
                actor._router = Some(_router);
                actor.listener_id = Some(id);
            }))
            .map_err(|e| {
                libp2p_core::transport::TransportError::Other(TransportError {
                    kind: TransportErrorKind::Listen(format!("Failed to set router: {e}")),
                })
            })?;

        let iroh_addr = helper::iroh_node_id_to_multiaddr(&self.node_id);
        self.transport_events_tx
            .send(libp2p_core::transport::TransportEvent::NewAddress {
                listener_id: id,
                listen_addr: iroh_addr,
            })
            .map_err(|e| {
                libp2p_core::transport::TransportError::Other(TransportError {
                    kind: TransportErrorKind::Listen(format!(
                        "Failed to send NewAddress event: {e}"
                    )),
                })
            })
    }

    fn remove_listener(&mut self, id: libp2p_core::transport::ListenerId) -> bool {
        let listener_id = self
            .protocol
            .api
            .call_blocking(act_ok!(actor => async move { actor.listener_id }))
            .map_err(|_| false)
            .unwrap_or(None);
        if let Some(current_id) = listener_id {
            if current_id == id {
                self.protocol
                    .api
                    .call_blocking(act_ok!(actor => async move {
                        actor.listener_id = None;
                    }))
                    .ok();
                return true;
            }
        }
        false
    }

    fn dial(
        &mut self,
        addr: libp2p_core::Multiaddr,
        _opts: libp2p_core::transport::DialOpts,
    ) -> Result<Self::Dial, libp2p_core::transport::TransportError<Self::Error>> {
        let node_id = helper::multiaddr_to_iroh_node_id(&addr).ok_or_else(|| {
            libp2p_core::transport::TransportError::Other(TransportError {
                kind: TransportErrorKind::Dial(
                    "Failed to extract iroh NodeId from multiaddr".to_string(),
                ),
            })
        })?;
        let protocol = self.protocol.clone();

        let endpoint = protocol
            .api
            .call_blocking(act_ok!(actor => async move { actor.endpoint.clone() }))
            .map_err(|e| {
                libp2p_core::transport::TransportError::Other(TransportError {
                    kind: TransportErrorKind::Dial(format!(
                        "Failed to get endpoint from transport protocol: {e}"
                    )),
                })
            })?;

        Ok(async move {
            let connecting = endpoint.connect(node_id, Protocol::ALPN);
            let conn = connecting.await.map_err(|e| TransportError {
                kind: TransportErrorKind::Dial(e.to_string()),
            })?;

            Ok(Connection::new(conn))
        }
        .boxed())
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

impl ProtocolHandler for Protocol {
    async fn accept(
        &self,
        connection: iroh::endpoint::Connection,
    ) -> Result<(), iroh::protocol::AcceptError> {
        let remote_multi = helper::iroh_node_id_to_multiaddr(&connection.remote_node_id()?);
        let local_multi = helper::iroh_node_id_to_multiaddr(&self.api
            .call(act_ok!(actor => async move {
                actor.endpoint.node_id()
            }))
            .await
            .map_err(iroh::protocol::AcceptError::from_err)?);

        self.api
            .call(act_ok!(actor => async move {
               actor.transport_tx.send(
                   libp2p_core::transport::TransportEvent::Incoming {
                       listener_id: actor.listener_id.expect("Listener ID should be set"),
                       upgrade: Connecting {
                           connecting: async move {
                               Ok(connection)
                           }.boxed()
                       },
                       local_addr: local_multi.clone(),
                       send_back_addr: remote_multi.clone(),
                   }).map_err(|e| TransportError::from(e.to_string().as_str()))
            }))
            .await
            .map_err(iroh::protocol::AcceptError::from_err)?
            .map_err(iroh::protocol::AcceptError::from_err)
    }
}
