use std::{error::Error, fmt::Display, pin::Pin, task::Poll};

use crate::{
    TransportError,
    stream::{Stream, StreamError},
};
use futures::{FutureExt, future::BoxFuture};
use iroh::endpoint::{RecvStream, SendStream};
use libp2p_core::StreamMuxer;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Debug)]
pub struct ConnectionError {
    kind: ConnectionErrorKind,
}

#[derive(Debug)]
pub enum ConnectionErrorKind {
    Accept(String),
    Open(String),
    Stream(String),
}

impl Display for ConnectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ConnectionError: {:?}", self.kind)
    }
}

impl Error for ConnectionError {}

impl From<iroh::endpoint::ConnectionError> for ConnectionError {
    fn from(err: iroh::endpoint::ConnectionError) -> Self {
        Self {
            kind: ConnectionErrorKind::Accept(err.to_string()),
        }
    }
}

impl From<&str> for ConnectionError {
    fn from(err: &str) -> Self {
        Self {
            kind: ConnectionErrorKind::Accept(err.to_string()),
        }
    }
}

impl From<StreamError> for ConnectionError {
    fn from(err: StreamError) -> Self {
        Self {
            kind: ConnectionErrorKind::Stream(err.to_string()),
        }
    }
}

pub struct Connection {
    connection: iroh::endpoint::Connection,
    incoming: Option<BoxFuture<'static, Result<(SendStream, RecvStream), ConnectionError>>>,
    outgoing: Option<BoxFuture<'static, Result<(SendStream, RecvStream), ConnectionError>>>,
    closing: Option<BoxFuture<'static, ConnectionError>>,
}

pub struct Connecting {
    pub connecting:
        BoxFuture<'static, Result<(libp2p::PeerId, iroh::endpoint::Connection), TransportError>>,
}

impl Connection {
    pub fn new(connection: iroh::endpoint::Connection) -> Self {
        tracing::debug!("Connection::new - Creating new connection wrapper");
        Self {
            connection,
            incoming: None,
            outgoing: None,
            closing: None,
        }
    }
}

impl StreamMuxer for Connection {
    type Substream = Stream;
    type Error = ConnectionError;

    fn poll_inbound(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<Self::Substream, Self::Error>> {
        let this = self.get_mut();

        let incoming = this.incoming.get_or_insert_with(|| {
            tracing::debug!("Connection::poll_inbound - Setting up incoming stream future");
            let connection = this.connection.clone();
            async move {
                tracing::debug!("Connection::poll_inbound - Accepting bidirectional stream");
                match connection.accept_bi().await {
                    Ok((s, mut r)) => {
                        tracing::debug!("Connection::poll_inbound - Bidirectional stream accepted, reading handshake byte");
                        r.read_u8().await.map_err(|e| {
                            tracing::error!("Connection::poll_inbound - Failed to read handshake byte: {}", e);
                            ConnectionError::from("Failed to read from stream")
                        })?;
                        tracing::debug!("Connection::poll_inbound - Handshake byte read successfully");
                        Ok((s, r))
                    },
                    Err(e) => {
                        tracing::error!("Connection::poll_inbound - Failed to accept bidirectional stream: {}", e);
                        Err(ConnectionError::from("Iroh handshake failed during accept"))
                    }
                }
             }.boxed()
        });

        let (send, recv) = futures::ready!(incoming.poll_unpin(cx))?;
        this.incoming.take();
        tracing::debug!("Connection::poll_inbound - Inbound stream ready, creating Stream wrapper");
        Poll::Ready(Stream::new(send, recv).map_err(Into::into))
    }

    fn poll_outbound(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<Self::Substream, Self::Error>> {
        let this = self.get_mut();

        let outgoing = this.outgoing.get_or_insert_with(|| {
            tracing::debug!("Connection::poll_outbound - Setting up outgoing stream future");
            let connection = this.connection.clone();
            async move {
                tracing::debug!("Connection::poll_outbound - Opening bidirectional stream");
                match connection.open_bi().await {
                    Ok((mut s, r)) => {
                        tracing::debug!("Connection::poll_outbound - Bidirectional stream opened, writing handshake byte");
                        // one byte iroh-handshake since accept only connects after open and write, not just open
                        s.write_u8(0).await.map_err(|e| {
                            tracing::error!("Connection::poll_outbound - Failed to write handshake byte: {}", e);
                            ConnectionError::from("Failed to write to stream")
                        })?;
                        tracing::debug!("Connection::poll_outbound - Handshake byte written successfully");
                        Ok((s, r))
                    }
                    Err(e) => {
                        tracing::error!("Connection::poll_outbound - Failed to open bidirectional stream: {}", e);
                        Err(ConnectionError::from("Iroh handshake failed during open"))
                    }
                }
            }.boxed()
        });

        let (send, recv) = futures::ready!(outgoing.poll_unpin(cx))?;
        this.outgoing.take();
        tracing::debug!(
            "Connection::poll_outbound - Outbound stream ready, creating Stream wrapper"
        );
        Poll::Ready(Stream::new(send, recv).map_err(Into::into))
    }

    fn poll_close(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        let this = self.get_mut();

        let closing = this.closing.get_or_insert_with(|| {
            tracing::debug!("Connection::poll_close - Closing connection");
            this.connection.close(From::from(0u32), &[]);
            let connection = this.connection.clone();
            async move {
                tracing::debug!("Connection::poll_close - Waiting for connection to close");
                connection.closed().await.into()
            }
            .boxed()
        });

        if matches!(
            futures::ready!(closing.poll_unpin(cx)),
            crate::ConnectionError { .. }
        ) {
            tracing::error!("Connection::poll_close - Failed to close connection");
            return Poll::Ready(Err("failed to close connection".into()));
        };

        tracing::debug!("Connection::poll_close - Connection closed successfully");
        Poll::Ready(Ok(()))
    }

    fn poll(
        self: Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<libp2p_core::muxing::StreamMuxerEvent, Self::Error>> {
        Poll::Pending
    }
}

impl Future for Connecting {
    type Output = Result<(libp2p::PeerId, libp2p_core::muxing::StreamMuxerBox), TransportError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        tracing::debug!("Connecting::poll - Polling connection future");
        let (peer_id, conn) = match self.connecting.poll_unpin(cx) {
            Poll::Ready(Ok((peer_id, conn))) => {
                tracing::debug!("Connecting::poll - Connection established");
                (peer_id, conn)
            }
            Poll::Ready(Err(e)) => {
                tracing::error!("Connecting::poll - Connection failed: {}", e);
                return Poll::Ready(Err(e));
            }
            Poll::Pending => {
                tracing::trace!("Connecting::poll - Connection still pending");
                return Poll::Pending;
            }
        };

        let muxer = Connection::new(conn);

        tracing::debug!("Connecting::poll - Connection muxer created");
        Poll::Ready(Ok((
            peer_id,
            libp2p_core::muxing::StreamMuxerBox::new(muxer),
        )))
    }
}
