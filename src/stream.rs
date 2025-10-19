use std::{fmt::Display, pin::Pin, sync::Arc, task::Poll};

use actor_helper::{Action, Actor, ActorError, Handle, Receiver, act_ok};
use futures::{AsyncRead, AsyncWrite};
use iroh::endpoint::VarInt;
use tokio::{
    io::{AsyncReadExt, AsyncWrite as _, AsyncWriteExt},
    sync::Mutex,
};
use tracing::{debug, warn};

// IrohStream error:
#[derive(Debug, Clone)]
pub struct StreamError {
    kind: StreamErrorKind,
}

#[derive(Debug, Clone)]
pub enum StreamErrorKind {
    Read(String),
    Write(String),
    Connection(String),
}

impl ActorError for StreamError {
    fn from_actor_message(msg: String) -> Self {
        Self {
            kind: StreamErrorKind::Connection(msg),
        }
    }
}

impl From<std::io::Error> for StreamError {
    fn from(err: std::io::Error) -> Self {
        Self {
            kind: StreamErrorKind::Read(err.to_string()),
        }
    }
}

impl From<iroh::endpoint::ConnectionError> for StreamError {
    fn from(err: iroh::endpoint::ConnectionError) -> Self {
        Self {
            kind: StreamErrorKind::Connection(err.to_string()),
        }
    }
}

impl From<iroh::endpoint::WriteError> for StreamError {
    fn from(err: iroh::endpoint::WriteError) -> Self {
        Self {
            kind: StreamErrorKind::Write(err.to_string()),
        }
    }
}

impl From<iroh::endpoint::ReadError> for StreamError {
    fn from(err: iroh::endpoint::ReadError) -> Self {
        Self {
            kind: StreamErrorKind::Read(err.to_string()),
        }
    }
}

impl From<iroh::endpoint::RemoteNodeIdError> for StreamError {
    fn from(err: iroh::endpoint::RemoteNodeIdError) -> Self {
        Self {
            kind: StreamErrorKind::Connection(err.to_string()),
        }
    }
}

impl Display for StreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.kind {
            StreamErrorKind::Read(msg) => write!(f, "IrohStream Read Error: {msg}"),
            StreamErrorKind::Write(msg) => write!(f, "IrohStream Write Error: {msg}"),
            StreamErrorKind::Connection(msg) => {
                write!(f, "IrohStream Connection Error: {msg}")
            }
        }
    }
}

impl std::error::Error for StreamError {}

#[derive(Debug, Clone)]
pub struct Stream {
    api: Handle<StreamActor, StreamError>,

    sender: Arc<Mutex<iroh::endpoint::SendStream>>,
    receiver: Arc<Mutex<iroh::endpoint::RecvStream>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActorStatus {
    Running,
    Stopped,
}

#[derive(Debug)]
struct StreamActor {
    rx: Receiver<Action<StreamActor>>,
    handle: u64,
    actor_status: ActorStatus,

    node_id: iroh::NodeId,
    _conn: iroh::endpoint::Connection,
}

#[derive(Debug, Clone)]
pub enum ConnectionShould {
    Accept,
    Open,
}

impl Stream {
    pub async fn new(
        handle: u64,
        conn: iroh::endpoint::Connection,
        connection_should: ConnectionShould,
    ) -> Result<Self, StreamError> {
        let (api, rx) = Handle::channel();

        // Open (or accept) the QUIC bidi stream before spawning the actor.
        // IMPORTANT: write to the open stream and read from the accept stream 1 byte to get things going.
        // (quirks of the iroh).
        let (conn_sender, conn_receiver) = match connection_should {
            ConnectionShould::Accept => {
                let mut split = conn.accept_bi().await?;
                split.1.read_u8().await?;
                split
            }
            ConnectionShould::Open => {
                let mut split = conn.open_bi().await?;
                split.0.write_u8(0).await?;
                split.0.flush().await?;
                split
            }
        };
        let remote_node_id = conn.remote_node_id()?;
        debug!(
            "Established IrohStream with remote node id: {}",
            remote_node_id
        );

        // Spawn the stream actor - no handshake / blocking wait.
        tokio::spawn(async move {
            let mut actor = StreamActor {
                actor_status: ActorStatus::Running,
                rx,
                handle,
                node_id: remote_node_id,
                _conn: conn,
            };

            if let Err(e) = actor.run().await {
                warn!("IrohStreamActor exited with error: {:?}", e);
            }
        });

        Ok(Self {
            api,
            sender: Arc::new(Mutex::new(conn_sender)),
            receiver: Arc::new(Mutex::new(conn_receiver)),
        })
    }

    pub async fn get_handle(&self) -> Result<u64, StreamError> {
        self.api
            .call(act_ok!(actor => async move { actor.handle }))
            .await
    }

    pub async fn get_status(&self) -> Result<ActorStatus, StreamError> {
        self.api
            .call(act_ok!(actor => async move {
                actor.actor_status.clone()
            }))
            .await
    }

    pub async fn get_node_id(&self) -> Result<iroh::NodeId, StreamError> {
        self.api
            .call(act_ok!(actor => async move { actor.node_id }))
            .await
    }
}

impl Actor<StreamError> for StreamActor {
    async fn run(&mut self) -> Result<(), StreamError> {
        while self.actor_status == ActorStatus::Running {
            tokio::select! {
                Ok(action) = self.rx.recv_async() => {
                    let res = action(self).await;
                    if self.actor_status == ActorStatus::Stopped {
                        break;
                    }
                    if self.actor_status == ActorStatus::Stopped {
                        break
                    }
                    res
                }
                else => break,
            }
        }

        Err(StreamError {
            kind: StreamErrorKind::Connection("IrohStreamActor stopped".into()),
        })
    }
}

impl AsyncRead for Stream {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        if let Ok(mut recv) = self.receiver.try_lock() {
            return Pin::new(&mut *recv).poll_read(cx, buf).map_err(Into::into);
        }
        std::task::Poll::Pending
    }
}

impl AsyncWrite for Stream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        if let Ok(mut send) = self.sender.try_lock() {
            return Pin::new(&mut *send).poll_write(cx, buf).map_err(Into::into);
        }
        std::task::Poll::Pending
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        if let Ok(mut send) = self.sender.try_lock() {
            return Pin::new(&mut *send).poll_flush(cx).map_err(Into::into);
        }
        std::task::Poll::Pending
    }

    fn poll_close(
        self: Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        if let Ok(conn) = self
            .api
            .call_blocking(act_ok!(actor => async move { actor._conn.clone()}))
        {
            conn.close(VarInt::default(), b"stopped called on stream wrapper");
            let _ = self.api.call_blocking(act_ok!(actor => async move {
                actor.actor_status = ActorStatus::Stopped;
            }));
        }
        Poll::Ready(Ok(()))
    }
}