use std::{error::Error, fmt::Display, pin::Pin, task::Poll};

use crate::{
    TransportError,
    stream::{Stream, StreamError},
};
use futures::{
    FutureExt,
    future::{BoxFuture, Select},
};
use futures_timer::Delay;
use iroh::{
    endpoint::{RecvStream, SendStream},
    protocol::ProtocolHandler,
};
use libp2p_core::StreamMuxer;

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
    connecting: Select<BoxFuture<'static, iroh::endpoint::Connection>, Delay>,
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
            let connection = this.connection.clone();
            async move { connection.accept_bi().await.map_err(Into::into) }.boxed()
        });

        let (send, recv) = futures::ready!(incoming.poll_unpin(cx))?;
        this.incoming.take();
        Poll::Ready(Stream::new(send, recv).map_err(Into::into))
    }

    fn poll_outbound(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<Self::Substream, Self::Error>> {
        let this = self.get_mut();

        let outgoing = this.outgoing.get_or_insert_with(|| {
            let connection = this.connection.clone();
            async move { connection.open_bi().await.map_err(Into::into) }.boxed()
        });

        let (send, recv) = futures::ready!(outgoing.poll_unpin(cx))?;
        this.outgoing.take();
        Poll::Ready(Stream::new(send, recv).map_err(Into::into))
    }

    fn poll_close(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        let this = self.get_mut();

        let closing = this.closing.get_or_insert_with(|| {
            this.connection.close(From::from(0u32), &[]);
            let connection = this.connection.clone();
            async move { connection.closed().await.into() }.boxed()
        });

        if matches!(
            futures::ready!(closing.poll_unpin(cx)),
            crate::ConnectionError { .. }
        ) {
            return Poll::Ready(Err("failed to close connection".into()));
        };

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
    type Output = Result<Connection, TransportError>;

    fn poll(self: Pin<&mut Self>, _cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let connection = match futures::ready!(self.get_mut().connecting.poll_unpin(_cx)) {
            futures::future::Either::Right(_) => {
                return Poll::Ready(Err(TransportError::from("Connection timed out")));
            }
            futures::future::Either::Left((connection, _)) => connection,
        };

        let muxer = Connection {
            connection,
            incoming: None,
            outgoing: None,
            closing: None,
        };

        Poll::Ready(Ok(muxer))
    }
}
